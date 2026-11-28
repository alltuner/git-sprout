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
        self.read_config(directory, Some(kind), key)
    }

    /// Whether a configuration key is set at all for the given worktree, whatever its
    /// value. A git that could not answer is an error, not an unset key.
    pub fn config_is_set(&self, directory: &Path, key: &str) -> io::Result<bool> {
        Ok(self.read_config(directory, None, key)?.is_some())
    }

    /// Runs `git config -z --get`, with `--type=<kind>` when there is a kind, and reads the
    /// one value it prints.
    fn read_config(
        &self,
        directory: &Path,
        kind: Option<&str>,
        key: &str,
    ) -> io::Result<Option<String>> {
        let kind = kind.map(|kind| format!("--type={kind}"));
        let output = self
            .command(Some(directory))
            .args(["config", "-z"])
            .args(kind)
            .args(["--get", key])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        match output.status.code() {
            // `git config --get` exits 1, and prints nothing, for a key that is not set.
            Some(1) => Ok(None),
            Some(0) => {
                let text = String::from_utf8(output.stdout)
                    .map_err(|_| io::Error::other("git printed non-UTF-8"))?;
                // `-z` ends the value with a NUL, so a newline of its own survives.
                Ok(Some(text.strip_suffix('\0').unwrap_or(&text).to_string()))
            }
            _ => Err(io::Error::other("git config could not read the value")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value can end in a newline of its own, which a reading that trims the line git
    /// prints would lose.
    #[test]
    fn a_typed_value_keeps_its_trailing_newline() {
        let repo =
            std::env::temp_dir().join(format!("git-sprout-config-value-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        let git = Git::new(Vec::new());
        git.capture(
            None,
            [OsStr::new("init"), OsStr::new("-q"), repo.as_os_str()],
        )
        .expect("git init");
        git.capture(Some(&repo), ["config", "core.attributesFile", "name\n"])
            .expect("git config");

        assert_eq!(
            git.config_typed(&repo, "path", "core.attributesFile")
                .expect("git answers"),
            Some("name\n".to_string())
        );
        assert_eq!(
            git.config_typed(&repo, "path", "core.unsetKey")
                .expect("git answers"),
            None
        );
        let _ = std::fs::remove_dir_all(&repo);
    }
}
