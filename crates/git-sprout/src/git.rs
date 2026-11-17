// ABOUTME: Runs git and reads its answers, keeping git the authority on git behaviour.
// ABOUTME: Carries the caller's global options so every child sees the same repository.

use std::ffi::{OsStr, OsString};
use std::io;
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

/// Invokes git with a fixed set of global options in front of every subcommand.
#[derive(Debug, Clone)]
pub struct Git {
    globals: Vec<OsString>,
}

impl Git {
    pub fn new(globals: Vec<OsString>) -> Self {
        Git { globals }
    }

    /// The caller's globals come first so that a `-C` of ours, which is always absolute,
    /// still decides where git runs: repeated `-C` options are applied in order.
    fn command(&self, directory: Option<&Path>) -> Command {
        let mut command = Command::new("git");
        command.args(&self.globals);
        if let Some(directory) = directory {
            command.arg("-C").arg(directory);
        }
        command
    }

    /// Runs git and returns its stdout, or an error if it failed or could not start.
    pub fn capture<I, S>(&self, directory: Option<&Path>, args: I) -> io::Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self
            .command(directory)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other("git exited with a failure status"));
        }
        Ok(output.stdout)
    }

    /// Runs git with the caller's stdio, so its output is indistinguishable from ours.
    pub fn passthrough<I, S>(&self, directory: Option<&Path>, args: I) -> io::Result<ExitStatus>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command(directory).args(args).status()
    }

    /// Runs git with `input` on its stdin and returns its stdout.
    pub fn capture_with_input<I, S>(
        &self,
        directory: Option<&Path>,
        args: I,
        input: &[u8],
    ) -> io::Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut child = self
            .command(directory)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        // The input has to be written while the output is being read: git streams its
        // answers as it consumes paths, so writing everything first deadlocks as soon as
        // either pipe fills.
        let stdin = child.stdin.take();
        let input = input.to_vec();
        let writer = std::thread::spawn(move || {
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(&input);
            }
        });
        let output = child.wait_with_output()?;
        let _ = writer.join();
        if !output.status.success() {
            return Err(io::Error::other("git exited with a failure status"));
        }
        Ok(output.stdout)
    }

    /// Reads a single-line answer such as an object id, with the trailing newline removed.
    pub fn capture_line<I, S>(&self, directory: Option<&Path>, args: I) -> io::Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let bytes = self.capture(directory, args)?;
        let text =
            String::from_utf8(bytes).map_err(|_| io::Error::other("git printed non-UTF-8"))?;
        Ok(text.trim_end_matches(['\n', '\r']).to_string())
    }

    /// Reads a configuration value as git resolves it for the given worktree.
    pub fn config(&self, directory: &Path, key: &str) -> Option<String> {
        self.capture_line(Some(directory), ["config", "--get", key])
            .ok()
    }

    /// Reads a configuration value the way git interprets it, canonicalised by
    /// `git config --type=<kind>`. Unlike [`Git::config`] this tells an unset key
    /// (`Ok(None)`) apart from a git that could not answer.
    pub fn config_typed(
        &self,
        directory: &Path,
        kind: &str,
        key: &str,
    ) -> io::Result<Option<String>> {
        let output = self
            .command(Some(directory))
            .args(["config", &format!("--type={kind}"), "--get", key])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        match output.status.code() {
            // `git config --get` exits 1, and prints nothing, for a key that is not set.
            Some(1) => Ok(None),
            Some(0) => {
                let text = String::from_utf8(output.stdout)
                    .map_err(|_| io::Error::other("git printed non-UTF-8"))?;
                Ok(Some(text.trim_end_matches(['\n', '\r']).to_string()))
            }
            _ => Err(io::Error::other("git config could not read the value")),
        }
    }

    /// The names of the configuration keys matching `regexp`, as
    /// `git config --name-only --get-regexp` prints them. No key matching is `Ok` and
    /// empty, told apart from a git that could not answer.
    pub fn config_names(&self, directory: &Path, regexp: &str) -> io::Result<Vec<String>> {
        let output = self
            .command(Some(directory))
            .args(["config", "--name-only", "--get-regexp", regexp])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        match output.status.code() {
            // `git config --get-regexp` exits 1, and prints nothing, when no key matches.
            Some(1) => Ok(Vec::new()),
            Some(0) => {
                let text = String::from_utf8(output.stdout)
                    .map_err(|_| io::Error::other("git printed non-UTF-8"))?;
                Ok(text.lines().map(str::to_string).collect())
            }
            _ => Err(io::Error::other("git config could not read the keys")),
        }
    }
}

/// Reads the contents `git cat-file --batch` writes for `count` requested objects, in the
/// order they were asked for: `<oid> <type> <size>` on a line, the bytes, and a newline.
/// `None` when the output is not that, or names an object git does not have.
pub fn parse_cat_file_batch(output: &[u8], count: usize) -> Option<Vec<Vec<u8>>> {
    let mut contents = Vec::with_capacity(count);
    let mut rest = output;
    for _ in 0..count {
        let end = rest.iter().position(|byte| *byte == b'\n')?;
        let header = std::str::from_utf8(&rest[..end]).ok()?;
        let size: usize = header.split(' ').nth(2)?.parse().ok()?;
        let body = rest.get(end + 1..end + 1 + size)?;
        if rest.get(end + 1 + size) != Some(&b'\n') {
            return None;
        }
        contents.push(body.to_vec());
        rest = &rest[end + 2 + size..];
    }
    rest.is_empty().then_some(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_objects_cat_file_batch_writes() {
        let output = b"aa blob 3\none\nbb blob 0\n\n".as_slice();
        assert_eq!(
            parse_cat_file_batch(output, 2),
            Some(vec![b"one".to_vec(), Vec::new()])
        );
        assert_eq!(parse_cat_file_batch(b"aa missing\n", 1), None);
        assert_eq!(parse_cat_file_batch(b"aa blob 5\none\n", 1), None);
        assert_eq!(parse_cat_file_batch(output, 1), None);
    }
}
