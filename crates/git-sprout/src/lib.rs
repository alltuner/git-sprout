// ABOUTME: Entry point shared by the git-sprout and git-worktree-fast binaries.
// ABOUTME: Decides whether a `worktree add` can be accelerated and runs it either way.

use std::ffi::OsString;
use std::process::ExitCode;

pub mod argv;
pub mod delegate;
pub mod stats;

use argv::{AddCommand, Invocation};
use stats::Stats;

/// Runs the tool with the given argv tail (everything after the program name).
pub fn run(args: Vec<OsString>) -> ExitCode {
    let mut stats = Stats::default();

    let invocation = argv::parse(&args);
    let (git_args, reason) = match invocation {
        Invocation::Version => {
            println!("git-sprout {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Invocation::Delegate { git_args, reason } => (git_args, reason.to_string()),
        Invocation::Add(add) => match decline(&add) {
            Some(reason) => (add.git_args(), reason),
            None => (add.git_args(), "acceleration is not built yet".to_string()),
        },
    };

    stats.fall_back(reason);
    stats.emit();
    delegate::exec_git(&git_args)
}

/// Why this request cannot be accelerated, if it cannot be.
fn decline(add: &AddCommand) -> Option<String> {
    if add.no_cow {
        return Some("--no-cow".to_string());
    }
    if std::env::var_os("SPROUT_DISABLE").is_some_and(|value| value == "1") {
        return Some("SPROUT_DISABLE=1".to_string());
    }
    if !add.checkout {
        return Some("--no-checkout".to_string());
    }
    if add.orphan {
        return Some("--orphan".to_string());
    }
    None
}
