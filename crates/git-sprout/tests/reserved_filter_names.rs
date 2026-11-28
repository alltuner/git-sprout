// ABOUTME: Proves a filter driver named like a state `git check-attr` prints, `unset` or
// ABOUTME: `unspecified`, never lets its paths be cloned as if they were unfiltered.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct Sandbox(PathBuf);

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn isolated(mut command: Command) -> Command {
    command
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid");
    command
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let mut command = isolated(Command::new("git"));
    let output = command.current_dir(cwd).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// `* text=auto eol=lf` would let these paths be cloned, and `git check-attr` prints
/// `filter=unset` as `unset`, the same as `-filter`. Git reads the string as the name of
/// the configured driver, whose clean and smudge change case and keep the length, so the
/// lowercase working copy is stat-clean and the blob's size while a checkout writes
/// uppercase. Neither the stat cache nor the size check can see that.
#[test]
fn a_filter_driver_named_like_a_state_is_left_to_git() {
    for driver in ["unset", "unspecified"] {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sandbox = Sandbox(std::env::temp_dir().join(format!(
            "sprout-reserved-{driver}-{}-{unique}",
            std::process::id()
        )));
        let source = sandbox.0.join("source");
        fs::create_dir_all(&source).unwrap();
        git(&source, &["init", "-q"]);
        git(&source, &["config", "commit.gpgsign", "false"]);
        git(&source, &["config", "core.autocrlf", "false"]);
        git(&source, &["config", "core.eol", "lf"]);
        git(
            &source,
            &["config", &format!("filter.{driver}.clean"), "tr A-Z a-z"],
        );
        git(
            &source,
            &["config", &format!("filter.{driver}.smudge"), "tr a-z A-Z"],
        );
        fs::write(
            source.join(".gitattributes"),
            format!("*.txt text=auto eol=lf filter={driver}\n"),
        )
        .unwrap();
        fs::write(source.join("hello.txt"), "hello\n").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-qm", "filtered"]);
        assert_eq!(
            git(&source, &["status", "--porcelain"]),
            "",
            "the scenario needs git to report the lowercase working copy clean"
        );

        let control = sandbox.0.join("control");
        git(
            &source,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                control.to_str().unwrap(),
                "HEAD",
            ],
        );
        assert_eq!(
            fs::read(control.join("hello.txt")).unwrap(),
            b"HELLO\n",
            "the scenario needs git to run the {driver} driver on checkout"
        );

        let candidate = sandbox.0.join("candidate");
        let mut command = isolated(Command::new(env!("CARGO_BIN_EXE_git-sprout")));
        let output = command
            .current_dir(&source)
            .args(["add", "-q", "--detach"])
            .arg(&candidate)
            .arg("HEAD")
            .env_remove("SPROUT_DISABLE")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read(candidate.join("hello.txt")).unwrap(),
            fs::read(control.join("hello.txt")).unwrap(),
            "filter={driver}: the new worktree must hold what git worktree add writes"
        );
    }
}
