// ABOUTME: Proves a source file git reports clean, but whose bytes an earlier attributes
// ABOUTME: rule wrote, is never cloned in place of what a fresh checkout writes.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Sandbox(PathBuf);

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Commit A checks `src/stale.txt` out with CRLF; commit B drops that rule without
/// touching the blob. Moving from A to B leaves the CRLF bytes on disk, and because the
/// index was refreshed outside the racy window git still calls them clean. A fresh
/// checkout of B writes the blob's LF bytes, so a clone of the source would differ while
/// both worktrees report a clean status.
///
/// The stale file sits beside an unchanged sibling so that `src/` is a candidate for a
/// whole-directory clone, which would carry the stale file along with its sibling.
#[test]
fn a_file_left_over_from_an_earlier_attributes_rule_is_left_to_git() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sandbox =
        Sandbox(std::env::temp_dir().join(format!("sprout-stale-{}-{unique}", std::process::id())));
    let source = sandbox.0.join("source");
    fs::create_dir_all(source.join("src")).unwrap();
    git(&source, &["init", "-q"]);
    git(&source, &["config", "core.autocrlf", "false"]);
    git(&source, &["config", "commit.gpgsign", "false"]);
    fs::write(source.join(".gitattributes"), "*.txt eol=crlf\n").unwrap();
    fs::write(source.join("src/stale.txt"), "one\ntwo\n").unwrap();
    fs::write(source.join("src/sibling.bin"), "unchanged\n").unwrap();
    git(&source, &["add", "."]);
    git(&source, &["commit", "-qm", "crlf"]);
    let crlf = git(&source, &["rev-parse", "HEAD"]);
    // Committing does not rewrite the LF file just added, so check it out for real.
    fs::remove_file(source.join("src/stale.txt")).unwrap();
    git(&source, &["checkout", "-q", "--", "src/stale.txt"]);

    // Only the attributes file is staged: `commit -a` would re-stage the CRLF file.
    fs::write(source.join(".gitattributes"), "*.md text\n").unwrap();
    git(&source, &["add", ".gitattributes"]);
    git(&source, &["commit", "-qm", "no crlf"]);
    let plain = git(&source, &["rev-parse", "HEAD"]);

    git(&source, &["checkout", "-q", "-f", &crlf]);
    // The refresh has to land in a later second than the checkout, or git treats the
    // entry as racily clean and re-reads it after the switch below, marking it modified.
    std::thread::sleep(Duration::from_millis(1100));
    git(&source, &["update-index", "-q", "--refresh"]);
    git(&source, &["checkout", "-q", "-f", &plain]);
    assert_eq!(
        fs::read(source.join("src/stale.txt")).unwrap(),
        b"one\r\ntwo\r\n",
        "the scenario needs the CRLF bytes to survive the switch"
    );
    assert_eq!(
        git(&source, &["status", "--porcelain"]),
        "",
        "the scenario needs git to report the stale file clean"
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

    let candidate = sandbox.0.join("candidate");
    let stats_file = sandbox.0.join("stats.json");
    let output = Command::new(env!("CARGO_BIN_EXE_git-sprout"))
        .current_dir(&source)
        .args(["add", "-q", "--detach"])
        .arg(&candidate)
        .arg("HEAD")
        .env_remove("SPROUT_DISABLE")
        .env("SPROUT_STATS", "1")
        .env("SPROUT_STATS_FILE", &stats_file)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read(candidate.join("src/stale.txt")).unwrap(),
        fs::read(control.join("src/stale.txt")).unwrap(),
        "the new worktree must hold what git worktree add writes"
    );
    assert_eq!(
        fs::read(candidate.join("src/sibling.bin")).unwrap(),
        fs::read(control.join("src/sibling.bin")).unwrap(),
    );

    // Without this the test passes on a filesystem that cannot clone at all, or after a
    // change that simply stops cloning: the sibling is a clean, unconverted file.
    let stats: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&stats_file).unwrap()).unwrap();
    if stats["clone_backend"] != "none" {
        assert!(
            stats["cloned"].as_u64().unwrap() > 0,
            "the unchanged sibling should still have been cloned: {stats}"
        );
    }
}
