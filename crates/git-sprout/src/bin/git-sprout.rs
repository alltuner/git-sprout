// ABOUTME: The `git sprout` binary. Git dispatches `git sprout <args>` here by name.
// ABOUTME: Argument handling lives in the library so both binary names behave alike.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    git_sprout::installer::dispatch(&args).unwrap_or_else(|| git_sprout::run(args))
}
