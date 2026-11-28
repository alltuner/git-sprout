// ABOUTME: Proves `text=unset` and `crlf=unset`, which check-attr prints as it prints `-text`,
// ABOUTME: are read the way git reads them: as a value it does not know, not as binary.

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

/// The two worktrees of one scenario, and what the tool reported about its own.
struct Worktrees {
    sandbox: Sandbox,
    control: PathBuf,
    candidate: PathBuf,
    stats: serde_json::Value,
}

impl Worktrees {
    fn assert_same(&self, path: &str) {
        assert_eq!(
            fs::read(self.candidate.join(path)).unwrap(),
            fs::read(self.control.join(path)).unwrap(),
            "{path} must hold what git worktree add writes"
        );
    }

    /// A temporary directory on a filesystem without block cloning (ext4 on CI) picks a
    /// backend whose first clone then fails, and nothing can be asserted about clones.
    fn filesystem_cannot_clone(&self) -> bool {
        self.stats["clone_backend"] == "none"
            || self.stats["fallback_reason"]
                .as_str()
                .is_some_and(|reason| reason.starts_with("cloning a file failed"))
    }
}

/// Stands in for a git older than 2.38, which has no `ls-files --format`: a `git` that
/// refuses that one option, leaves a marker behind, and is real git for everything else.
fn git_without_format(directory: &Path) -> PathBuf {
    let real = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(real.stdout).unwrap().trim().to_owned();
    fs::create_dir_all(directory).unwrap();
    let marker = directory.join("refused");
    let script = format!(
        "#!/bin/sh\nfor argument in \"$@\"; do\n    case \"$argument\" in\n        --format=*) : > '{}'; echo 'error: unknown option' >&2; exit 129 ;;\n    esac\ndone\nexec '{real}' \"$@\"\n",
        marker.display()
    );
    let shim = directory.join("git");
    fs::write(&shim, script).unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    marker
}

/// Commits `text.txt` and `data.dat`, both holding LF text, under the given attributes,
/// then turns `core.autocrlf` on if asked and adds a worktree with git and with the tool.
///
/// The working copies keep their LF bytes and git calls them clean either way: cleaning
/// them gives back the blob. Whether a checkout writes those bytes is the question.
fn worktrees(name: &str, attributes: &str, autocrlf: bool, old_git: bool) -> Worktrees {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sandbox = Sandbox(std::env::temp_dir().join(format!(
        "sprout-text-{name}-{}-{unique}",
        std::process::id()
    )));
    let source = sandbox.0.join("source");
    fs::create_dir_all(&source).unwrap();
    git(&source, &["init", "-q"]);
    git(&source, &["config", "core.autocrlf", "false"]);
    git(&source, &["config", "commit.gpgsign", "false"]);
    fs::write(source.join(".gitattributes"), attributes).unwrap();
    fs::write(source.join("text.txt"), "one\ntwo\n").unwrap();
    fs::write(source.join("data.dat"), "one\ntwo\n").unwrap();
    git(&source, &["add", "."]);
    git(&source, &["commit", "-qm", "lf"]);
    if autocrlf {
        git(&source, &["config", "core.autocrlf", "true"]);
    }
    assert_eq!(
        git(&source, &["status", "--porcelain"]),
        "",
        "the scenario needs git to report the LF files clean"
    );

    let control = sandbox.0.join("control");
    let path = control.to_str().unwrap();
    git(
        &source,
        &["worktree", "add", "-q", "--detach", path, "HEAD"],
    );

    let candidate = sandbox.0.join("candidate");
    let stats_file = sandbox.0.join("stats.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_git-sprout"));
    command
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
        .env("GIT_CONFIG_NOSYSTEM", "1");
    if old_git {
        let shims = sandbox.0.join("old-git");
        git_without_format(&shims);
        let mut path = shims.into_os_string();
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap());
        command.env("PATH", path);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stats = serde_json::from_str(&fs::read_to_string(&stats_file).unwrap()).unwrap();
    Worktrees {
        sandbox,
        control,
        candidate,
        stats,
    }
}

/// `git check-attr` prints `unset` for `text=unset` exactly as it does for `-text`, but
/// to git the first is a value it does not know, which leaves the path to
/// `core.autocrlf`. A checkout therefore writes CRLF for the text file, and only the
/// genuine `-text` beside it is binary.
#[test]
fn text_set_to_the_word_unset_is_left_to_autocrlf() {
    let made = worktrees("unset", "*.txt text=unset\n*.dat -text\n", true, false);
    assert_eq!(
        fs::read(made.control.join("text.txt")).unwrap(),
        b"one\r\ntwo\r\n",
        "the scenario needs git to convert the path"
    );
    made.assert_same("text.txt");
    made.assert_same("data.dat");
    if !made.filesystem_cannot_clone() {
        assert!(
            made.stats["cloned"].as_u64().unwrap() > 0,
            "the genuine -text path should still have been cloned: {}",
            made.stats
        );
    }
}

/// The deprecated spelling has the same two readings: `-crlf` is binary, `crlf=unset` is
/// a value git does not know.
#[test]
fn crlf_set_to_the_word_unset_is_left_to_autocrlf() {
    let made = worktrees("crlf", "*.txt crlf=unset\n*.dat -crlf\n", true, false);
    assert_eq!(
        fs::read(made.control.join("text.txt")).unwrap(),
        b"one\r\ntwo\r\n",
        "the scenario needs git to convert the path"
    );
    made.assert_same("text.txt");
    made.assert_same("data.dat");
    if !made.filesystem_cannot_clone() {
        assert!(
            made.stats["cloned"].as_u64().unwrap() > 0,
            "the genuine -crlf path should still have been cloned: {}",
            made.stats
        );
    }
}

/// With no `core.autocrlf`, an `eol` is what converts the same path: git reads
/// `text=unset eol=crlf` as `text eol=crlf`, and `-text eol=crlf` as binary.
#[test]
fn text_set_to_the_word_unset_does_not_cancel_an_eol() {
    let made = worktrees(
        "unset-eol",
        "*.txt text=unset eol=crlf\n*.dat -text eol=crlf\n",
        false,
        false,
    );
    assert_eq!(
        fs::read(made.control.join("text.txt")).unwrap(),
        b"one\r\ntwo\r\n",
        "the scenario needs git to convert the path"
    );
    made.assert_same("text.txt");
    made.assert_same("data.dat");
}

/// A git that cannot be asked keeps the reading there was before it could be: `unset`
/// is `-text`, so a path that really is binary is cloned as it always was.
#[test]
fn a_git_without_ls_files_format_still_clones_binary_paths() {
    let made = worktrees("old-git", "*.dat -text\n", true, true);
    assert!(
        made.sandbox.0.join("old-git/refused").exists(),
        "the scenario needs the tool to have asked for --format and been refused"
    );
    made.assert_same("text.txt");
    made.assert_same("data.dat");
    if !made.filesystem_cannot_clone() {
        assert!(
            made.stats["cloned"].as_u64().unwrap() > 0,
            "the -text path should still have been cloned: {}",
            made.stats
        );
    }
}

/// `* text=false` is a common way to write "leave line endings alone". Git does not know
/// that value, and with no `core.autocrlf` it converts nothing, so every path is as
/// cloneable as it would be with no attributes at all.
#[test]
fn a_text_value_git_does_not_know_does_not_stop_cloning() {
    let made = worktrees("false", "* text=false\n", false, false);
    made.assert_same("text.txt");
    made.assert_same("data.dat");
    if !made.filesystem_cannot_clone() {
        assert_eq!(
            made.stats["checked_out_by_git"], 0,
            "nothing is converted, so nothing should be left to git: {}",
            made.stats
        );
    }
}

/// Git passes over a `text` value it does not know and reads the deprecated `crlf`, as it
/// does for an unspecified `text`: `text=foo -crlf` is binary, so a checkout writes it as
/// stored even under `core.autocrlf=true`, and `text=foo crlf` is text. A git with
/// `ls-files --format` settles the `-crlf` path by its own reading, since check-attr
/// prints `unset` for it, so the run with a git that lacks it is the one that rests on
/// how the attributes are read here.
#[test]
fn a_text_value_git_does_not_know_falls_back_to_crlf() {
    for old_git in [false, true] {
        let name = if old_git {
            "foo-crlf-old-git"
        } else {
            "foo-crlf"
        };
        let made = worktrees(
            name,
            "*.txt text=foo crlf\n*.dat text=foo -crlf\n",
            true,
            old_git,
        );
        assert_eq!(
            fs::read(made.control.join("text.txt")).unwrap(),
            b"one\r\ntwo\r\n",
            "the scenario needs git to convert the path"
        );
        made.assert_same("text.txt");
        made.assert_same("data.dat");
        if !made.filesystem_cannot_clone() {
            assert!(
                made.stats["cloned"].as_u64().unwrap() > 0,
                "the text=foo -crlf path is binary to git and should have been cloned \
                 (old git: {old_git}): {}",
                made.stats
            );
        }
    }
}
