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
}
