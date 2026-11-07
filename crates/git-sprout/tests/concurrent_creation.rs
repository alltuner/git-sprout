#![cfg(unix)]

// Destination regression adapted from jkp's PR #22. The dirty-companion assertion
// additionally checks that the wrong destination cannot lose staged/unstaged work.

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
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", cwd)
        .env("LC_ALL", "C")
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
    let base = std::env::var_os("SPROUT_TEST_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let sandbox = Sandbox(base.join(format!(
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
  "$SPROUT_TEST_REAL_GIT" -C "$SPROUT_TEST_SOURCE" worktree add --detach "$SPROUT_TEST_COMPANION" "$SPROUT_TEST_OTHER_COMMIT" >/dev/null 2>&1 || exit $?
  printf 'staged companion edit\n' > "$SPROUT_TEST_COMPANION/marker"
  "$SPROUT_TEST_REAL_GIT" -C "$SPROUT_TEST_COMPANION" add marker || exit $?
  printf 'unstaged companion edit\n' >> "$SPROUT_TEST_COMPANION/marker"
  exit 0
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
        .env_clear()
        .env("HOME", &sandbox.0)
        .env("LC_ALL", "C")
        .env("PATH", path)
        .env("SPROUT_STATS", "0")
        .env("SPROUT_TEST_REAL_GIT", real_git)
        .env("SPROUT_TEST_SOURCE", &source)
        .env("SPROUT_TEST_COMPANION", &companion)
        .env("SPROUT_TEST_OTHER_COMMIT", &companion_commit)
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
        "staged companion edit\nunstaged companion edit\n"
    );
    assert_eq!(
        git(&companion, &["show", ":marker"]),
        "staged companion edit"
    );
    assert_eq!(git(&companion, &["status", "--short"]), "MM marker");
}

#[test]
fn a_source_edit_after_verification_cannot_poison_a_clean_destination() {
    for marker in ["marker", "nested/marker"] {
        source_edit_during_add(marker, Race::Content);
    }
}

#[test]
fn a_source_attribute_edit_cannot_change_checkout_conversion() {
    source_edit_during_add("marker", Race::Attributes);
}

#[test]
fn unavailable_attribute_source_option_falls_back_to_git_checkout() {
    source_edit_during_add("marker", Race::UnsupportedSource);
}

enum Race {
    Content,
    Attributes,
    UnsupportedSource,
}

fn source_edit_during_add(marker: &str, race: Race) {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::var_os("SPROUT_TEST_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let sandbox = Sandbox(base.join(format!(
        "sprout-source-race-{}-{unique}",
        std::process::id()
    )));
    let source = sandbox.0.join("source");
    fs::create_dir_all(&source).unwrap();
    git(&source, &["init", "-q"]);
    let file = source.join(marker);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "committed contents\n").unwrap();
    let link = source.join("link");
    std::os::unix::fs::symlink("committed-target", &link).unwrap();
    // An ordinary older checkout file is not racily clean. Otherwise Git could
    // happen to rehash the bad scratch entry and hide the bug during this test.
    let old = filetime::FileTime::from_system_time(
        SystemTime::now() - std::time::Duration::from_secs(10),
    );
    filetime::set_file_mtime(&file, old).unwrap();
    filetime::set_symlink_file_times(&link, old, old).unwrap();
    if !matches!(race, Race::Content) {
        let attributes = source.join(".gitattributes");
        fs::write(&attributes, "marker text eol=crlf\n").unwrap();
        filetime::set_file_mtime(&attributes, old).unwrap();
    }
    git(&source, &["add", "."]);
    git(
        &source,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "base"],
    );
    let real_git = PathBuf::from(git(&source, &["--exec-path"])).join("git");
    assert!(real_git.is_file());
    let wrappers = sandbox.0.join("wrappers");
    fs::create_dir(&wrappers).unwrap();
    let wrapper = wrappers.join("git");
    fs::write(
        &wrapper,
        r#"#!/bin/sh
if [ "$1" = -C ] && [ "$3" = check-attr ]; then
  if [ "$SPROUT_TEST_ATTRIBUTES" = 1 ]; then
    printf 'marker -text\n' > "$SPROUT_TEST_SOURCE/.gitattributes"
  fi
  "$SPROUT_TEST_REAL_GIT" "$@" || exit $?
  if [ "$SPROUT_TEST_ATTRIBUTES" != 1 ]; then
    printf 'parallel editor contents\n' > "$SPROUT_TEST_MARKER"
    rm "$SPROUT_TEST_SOURCE/link" || exit $?
    ln -s parallel-editor-target "$SPROUT_TEST_SOURCE/link" || exit $?
  fi
  exit "$SPROUT_TEST_ATTR_STATUS"
fi
exec "$SPROUT_TEST_REAL_GIT" "$@"
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(wrappers).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let target = sandbox.0.join("target");
    let output = Command::new(env!("CARGO_BIN_EXE_git-sprout"))
        .current_dir(&source)
        .args(["add", "--detach"])
        .arg(&target)
        .arg("HEAD")
        .env_clear()
        .env("HOME", &sandbox.0)
        .env("LC_ALL", "C")
        .env("PATH", path)
        .env("SPROUT_TEST_REAL_GIT", &real_git)
        .env("SPROUT_TEST_SOURCE", &source)
        .env("SPROUT_TEST_MARKER", &file)
        .env(
            "SPROUT_TEST_ATTRIBUTES",
            if !matches!(race, Race::Content) {
                "1"
            } else {
                "0"
            },
        )
        .env(
            "SPROUT_TEST_ATTR_STATUS",
            if matches!(race, Race::UnsupportedSource) {
                "129"
            } else {
                "0"
            },
        )
        .env("SPROUT_STATS", "1")
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
        fs::read_to_string(&file).unwrap(),
        if matches!(race, Race::Content) {
            "parallel editor contents\n"
        } else {
            "committed contents\n"
        }
    );
    assert_eq!(
        fs::read_link(&link).unwrap(),
        PathBuf::from(if matches!(race, Race::Content) {
            "parallel-editor-target"
        } else {
            "committed-target"
        })
    );
    assert_eq!(
        fs::read_to_string(target.join(marker)).unwrap(),
        if !matches!(race, Race::Content) {
            "committed contents\r\n"
        } else {
            "committed contents\n"
        }
    );
    assert_eq!(
        fs::read_link(target.join("link")).unwrap(),
        PathBuf::from("committed-target")
    );
    assert_eq!(
        git(&target, &["hash-object", "--path", marker, marker]),
        git(&target, &["rev-parse", &format!(":{marker}")])
    );
    assert_eq!(git(&target, &["status", "--short"]), "");
    if !matches!(race, Race::Content) {
        assert_eq!(
            fs::read_to_string(source.join(".gitattributes")).unwrap(),
            "marker -text\n"
        );
        assert_eq!(
            fs::read_to_string(target.join(".gitattributes")).unwrap(),
            "marker text eol=crlf\n"
        );
    }
    if matches!(race, Race::UnsupportedSource) {
        let stats = stats(&output);
        assert_eq!(stats["cloned"], 0);
        assert_eq!(stats["fell_back"], true);
        assert!(stats["fallback_reason"]
            .as_str()
            .unwrap()
            .contains("git check-attr --source"));
    }
}

fn stats(output: &std::process::Output) -> serde_json::Value {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stderr
        .lines()
        .find_map(|line| line.strip_prefix("sprout-stats: "))
        .unwrap_or_else(|| panic!("missing clone statistics: {stderr}"));
    serde_json::from_str(line).unwrap()
}

#[test]
fn clean_sha1_and_sha256_files_and_symlinks_are_accepted() {
    clean_clone("sha1", false);
    clean_clone("sha256", false);
}

#[test]
fn emulated_symlink_permissions_match_git_under_a_restrictive_umask() {
    clean_clone("sha1", true);
}

fn clean_clone(format: &str, emulate_symlinks: bool) {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::var_os("SPROUT_TEST_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let sandbox = Sandbox(base.join(format!(
        "sprout-clean-clone-{}-{unique}",
        std::process::id()
    )));
    let source = sandbox.0.join("source");
    fs::create_dir_all(&source).unwrap();
    git(
        &source,
        &["init", "-q", &format!("--object-format={format}")],
    );
    fs::write(source.join("marker"), "committed contents\n").unwrap();
    std::os::unix::fs::symlink("committed-target", source.join("link")).unwrap();
    let old = filetime::FileTime::from_system_time(
        SystemTime::now() - std::time::Duration::from_secs(10),
    );
    filetime::set_file_mtime(source.join("marker"), old).unwrap();
    filetime::set_symlink_file_times(source.join("link"), old, old).unwrap();
    git(&source, &["add", "."]);
    git(
        &source,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "base"],
    );
    if emulate_symlinks {
        git(&source, &["config", "core.symlinks", "false"]);
        fs::remove_file(source.join("link")).unwrap();
        git(&source, &["checkout", "--", "link"]);
        fs::set_permissions(source.join("link"), fs::Permissions::from_mode(0o644)).unwrap();
        filetime::set_file_mtime(source.join("link"), old).unwrap();
    }
    git(&source, &["update-index", "--refresh"]);
    let clones_supported = git_sprout::clone::for_this_platform()
        .clone_file(&source.join("marker"), &sandbox.0.join("probe"))
        .is_ok();
    let attribute_source_supported = Command::new("git")
        .current_dir(&source)
        .args(["check-attr", "--source=HEAD", "--all", "--", "marker"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", &sandbox.0)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
        .status
        .success();
    let mut outputs = Vec::new();
    for candidate in [false, true] {
        let target = sandbox.0.join(if candidate { "sprout" } else { "native" });
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "umask 077; exec \"$@\"", "fixture"]);
        if candidate {
            command.arg(env!("CARGO_BIN_EXE_git-sprout")).arg("add");
        } else {
            command.args(["git", "worktree", "add"]);
        }
        let output = command
            .args(["--detach"])
            .arg(&target)
            .arg("HEAD")
            .current_dir(&source)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", &sandbox.0)
            .env("LC_ALL", "C")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("SPROUT_STATS", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(target.join("marker")).unwrap(),
            "committed contents\n"
        );
        assert_eq!(git(&target, &["status", "--short"]), "");
        if emulate_symlinks {
            assert!(!fs::symlink_metadata(target.join("link"))
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(
                fs::read_to_string(target.join("link")).unwrap(),
                "committed-target"
            );
            assert_eq!(
                fs::metadata(target.join("link"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        } else {
            assert_eq!(
                fs::read_link(target.join("link")).unwrap(),
                PathBuf::from("committed-target")
            );
        }
        outputs.push(output);
    }
    if clones_supported && attribute_source_supported {
        // Exactly two planned entries, with both content and link text checked above:
        // a broken SHA-256 or symlink hash cannot hide behind a correct Git fallback.
        assert_eq!(stats(&outputs[1])["cloned"], 2);
        assert_eq!(stats(&outputs[1])["checked_out_by_git"], 0);
    } else {
        eprintln!(
            "clone acceptance not asserted: file cloning or native check-attr --source unavailable"
        );
    }
}
