// ABOUTME: Runs the real git and passes its exit status through unchanged.
// ABOUTME: Every path the tool declines to accelerate ends here.

use std::ffi::OsString;
use std::process::{Command, ExitCode};

/// Replaces this process with `git <args>` where the platform allows it, so stdout,
/// stderr, the exit code and signal disposition are git's own.
pub fn exec_git(args: &[OsString]) -> ExitCode {
    let mut command = Command::new("git");
    command.args(args);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        eprintln!("git-sprout: could not run git: {error}");
        ExitCode::from(1)
    }

    #[cfg(not(unix))]
    status_of(&mut command)
}

/// Runs `git <args>` to completion and returns its exit code.
pub fn run_git(args: &[OsString]) -> ExitCode {
    status_of(Command::new("git").args(args))
}

#[allow(dead_code)]
fn status_of(command: &mut Command) -> ExitCode {
    match command.status() {
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(error) => {
            eprintln!("git-sprout: could not run git: {error}");
            ExitCode::from(1)
        }
    }
}
