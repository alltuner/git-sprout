// ABOUTME: Proves a rule git applies when it reads an attributes file from disk, and not
// ABOUTME: when a checkout reads the same bytes from the index, is not taken at its word.

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

/// `src/.gitattributes` starts with a UTF-8 byte order mark. Git skips the mark when it
/// reads the file from disk, so in the source `*.txt -text` applies and the LF file is
/// binary and clean. A checkout reads the blob out of the index, where the mark stays in
/// front of the pattern and the rule matches nothing, so under `core.autocrlf=true` it
/// writes CRLF. The working copy is its blob byte for byte and every git command calls the
/// source clean, so only knowing that git reads these bytes two ways keeps the LF file
/// from being cloned.
///
/// This is not a differential fixture because git does not agree with itself about the
/// worktree it has just written: `git status` there reads the file from disk again and
/// reports the CRLF files modified whenever their index entries are racily clean.
#[test]
fn a_rule_only_the_working_tree_reader_applies_is_left_to_git() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sandbox = Sandbox(
        std::env::temp_dir().join(format!("sprout-readers-{}-{unique}", std::process::id())),
    );
    let source = sandbox.0.join("source");
    fs::create_dir_all(source.join("src")).unwrap();
    fs::create_dir_all(source.join("assets")).unwrap();
    git(&source, &["init", "-q"]);
    git(&source, &["config", "core.autocrlf", "false"]);
    git(&source, &["config", "commit.gpgsign", "false"]);
    fs::write(
        source.join("src/.gitattributes"),
        b"\xEF\xBB\xBF*.txt -text\n",
    )
    .unwrap();
    fs::write(source.join("src/marked.txt"), "one\ntwo\n").unwrap();
    fs::write(source.join("assets/.gitattributes"), "* -text\n").unwrap();
    fs::write(source.join("assets/sibling.bin"), "unchanged\n").unwrap();
    git(&source, &["add", "."]);
    git(&source, &["commit", "-qm", "marked rules"]);
    git(&source, &["config", "core.autocrlf", "true"]);
    assert_eq!(
        git(&source, &["check-attr", "text", "--", "src/marked.txt"]),
        "src/marked.txt: text: unset",
        "the scenario needs the source to apply the rule behind the mark"
    );
    assert_eq!(
        git(&source, &["status", "--porcelain"]),
        "",
        "the scenario needs git to report the source clean"
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
        fs::read(control.join("src/marked.txt")).unwrap(),
        b"one\r\ntwo\r\n",
        "the scenario needs a checkout to miss the rule behind the mark"
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
        fs::read(candidate.join("src/marked.txt")).unwrap(),
        fs::read(control.join("src/marked.txt")).unwrap(),
        "the new worktree must hold what git worktree add writes"
    );
    assert_eq!(
        fs::read(candidate.join("assets/sibling.bin")).unwrap(),
        fs::read(control.join("assets/sibling.bin")).unwrap(),
    );

    // Without this the test passes after a change that simply stops cloning: the sibling
    // sits behind rules git reads one way only. A temporary directory on a filesystem
    // without block cloning (ext4 on CI) picks a backend whose first clone then fails, so
    // only a fallback for that reason excuses an empty clone.
    let stats: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&stats_file).unwrap()).unwrap();
    let filesystem_cannot_clone = stats["clone_backend"] == "none"
        || stats["fallback_reason"]
            .as_str()
            .is_some_and(|reason| reason.starts_with("cloning a file failed"));
    if !filesystem_cannot_clone {
        assert!(
            stats["cloned"].as_u64().unwrap() > 0,
            "the sibling should still have been cloned: {stats}"
        );
    }
}
