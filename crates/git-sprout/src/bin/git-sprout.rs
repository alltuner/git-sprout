// ABOUTME: The `git sprout` binary. Git dispatches `git sprout <args>` here by name.
// ABOUTME: Argument handling lives in the library so both binary names behave alike.

use std::process::ExitCode;

fn main() -> ExitCode {
    git_sprout::run(std::env::args_os().skip(1).collect())
}
