// ABOUTME: Entry point shared by the git-sprout and git-worktree-fast binaries.
// ABOUTME: Dispatches the `add` subcommand and delegates everything else to git.

use std::process::ExitCode;

pub mod delegate;

/// Runs the tool with the given argv tail (everything after the program name).
pub fn run(args: Vec<std::ffi::OsString>) -> ExitCode {
    delegate::worktree_add(&args)
}
