// ABOUTME: Entry point shared by the git-sprout and git-worktree-fast binaries.
// ABOUTME: Decides whether a `worktree add` can be accelerated and runs it either way.

use std::ffi::OsString;
use std::process::ExitCode;

pub mod argv;
pub mod attributes;
pub mod clone;
pub mod delegate;
pub mod git;
pub mod interrupt;
pub mod plan;
pub mod scratch_index;
pub mod source;
pub mod sprout;
pub mod stats;
pub mod tree;
pub mod verify;

use argv::{AddCommand, Invocation};
use stats::Stats;

/// Runs the tool with the given argv tail (everything after the program name).
pub fn run(args: Vec<OsString>) -> ExitCode {
    let mut stats = Stats::default();

    let (git_args, reason) = match argv::parse(&args) {
        Invocation::Version => {
            println!("git-sprout {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Invocation::Delegate { git_args, reason } => (git_args, reason.to_string()),
        Invocation::Add(add) => match decline(&add) {
            Some(reason) => (add.git_args(), reason),
            None => return sprout::add(&add, &mut stats),
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
