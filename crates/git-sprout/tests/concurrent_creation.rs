#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
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

#[test]
fn another_registration_cannot_steal_the_requested_destination() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sandbox = Sandbox(std::env::temp_dir().join(format!(
        "sprout-destination-{}-{unique}",
        std::process::id()
    )));
    fs::create_dir_all(&sandbox.0).unwrap();
    let source = sandbox.0.join("source");
    fs::create_dir(&source).unwrap();
    git(&source, &["init", "-q"]);
    fs::write(source.join("marker"), "requested contents\n").unwrap();
    git(&source, &["add", "marker"]);
    git(
        &source,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "requested"],
    );
    let requested = git(&source, &["rev-parse", "HEAD"]);
    fs::write(source.join("marker"), "companion contents\n").unwrap();
    git(&source, &["add", "marker"]);
    git(
        &source,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "companion"],
    );
    let companion_commit = git(&source, &["rev-parse", "HEAD"]);
    git(&source, &["reset", "--hard", &requested]);

    let real_git = PathBuf::from(git(&source, &["--exec-path"])).join("git");
    assert!(real_git.is_file());
    let wrappers = sandbox.0.join("wrappers");
    fs::create_dir(&wrappers).unwrap();
    let wrapper = wrappers.join("git");
    fs::write(&wrapper, r#"#!/bin/sh
if [ "$1" = worktree ] && [ "$2" = add ]; then
  "$SPROUT_TEST_REAL_GIT" "$@" || exit $?
  "$SPROUT_TEST_REAL_GIT" -C "$SPROUT_TEST_SOURCE" worktree add --detach "$SPROUT_TEST_COMPANION" "$SPROUT_TEST_OTHER_COMMIT" >/dev/null 2>&1
  exit $?
fi
exec "$SPROUT_TEST_REAL_GIT" "$@"
"#).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let target = sandbox.0.join("z-requested");
    let companion = sandbox.0.join("a-companion");
    let path = std::env::join_paths(
        std::iter::once(wrappers).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    // Register the companion after Git adds our target but before Sprout locates it.
    // This forces the concurrent interleaving without scheduler-dependent timing.
    let output = Command::new(env!("CARGO_BIN_EXE_git-sprout"))
        .current_dir(&source)
        .args(["add", "--detach"])
        .arg(&target)
        .arg(&requested)
        .env("PATH", path)
        .env_remove("SPROUT_DISABLE")
        .env_remove("SPROUT_STATS_FILE")
        .env("SPROUT_STATS", "0")
        .env("SPROUT_TEST_REAL_GIT", real_git)
        .env("SPROUT_TEST_SOURCE", &source)
        .env("SPROUT_TEST_COMPANION", &companion)
        .env("SPROUT_TEST_OTHER_COMMIT", &companion_commit)
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
        fs::read_to_string(target.join("marker")).unwrap(),
        "requested contents\n"
    );
    assert_eq!(
        fs::read_to_string(companion.join("marker")).unwrap(),
        "companion contents\n"
    );
}
