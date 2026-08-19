// ABOUTME: Runs the real `git worktree add` and passes its exit status through.
// ABOUTME: Every failure path in the tool ends here, so the user gets git's own result.

use std::ffi::OsString;
use std::process::{Command, ExitCode};

/// Runs `git worktree <args>` inheriting stdio, and returns git's exit code.
pub fn worktree_add(args: &[OsString]) -> ExitCode {
    let status = Command::new("git").arg("worktree").args(args).status();
    match status {
        Ok(s) => ExitCode::from(u8::try_from(s.code().unwrap_or(1)).unwrap_or(1)),
        Err(err) => {
            eprintln!("git-sprout: could not run git: {err}");
            ExitCode::from(1)
        }
    }
}
