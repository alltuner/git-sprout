// ABOUTME: The `git worktree-fast` binary, a second name for the same tool.
// ABOUTME: It exists so the command reads as self-explanatory without knowing the name.

use std::process::ExitCode;

fn main() -> ExitCode {
    git_sprout::run(std::env::args_os().skip(1).collect())
}
