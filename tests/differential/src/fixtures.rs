// ABOUTME: Locates and runs the repository fixture builders, each of which is a
// ABOUTME: shell script that constructs one repository shape the matrix exercises.

use crate::env::Workspace;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A builder that cannot run here exits with this status and explains why on stderr.
const SKIP_STATUS: i32 = 77;

/// Every fixture the suite knows about, in the order they are listed in the spec's
/// repository matrix. `fixture_scripts_match_the_list` keeps this honest against the
/// contents of `tests/fixtures/build/`.
pub const ALL: &[&str] = &[
    "plain",
    "attrs-eol-crlf",
    "attrs-ident",
    "attrs-utf16",
    "attrs-filter",
    "attrs-lfs",
    "attrs-diverging",
    "autocrlf",
    "symlinks-exec",
    "odd-paths",
    "case-collision",
    "submodule",
    "nested-worktree",
    "sparse-cone",
    "sparse-nocone",
    "split-index",
    "index-v4",
    "many-files",
    "untracked-cache",
    "fsmonitor",
    "dirty-source",
    "mid-rebase",
    "detached-head",
    "sha256",
    "no-commits",
    "text-heavy",
];

#[derive(Debug)]
pub enum Built {
    Ok,
    Skipped(String),
}

/// The `tests/fixtures` directory, resolved from this crate's location so the suite
/// runs the same from any working directory.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the differential crate lives under tests/")
        .join("fixtures")
}

pub fn script_for(name: &str) -> PathBuf {
    fixtures_dir().join("build").join(format!("{name}.sh"))
}

/// The fixture names present on disk, for the drift check.
pub fn discover() -> std::io::Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures_dir().join("build"))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".sh").map(str::to_string)
        })
        .collect();
    names.sort();
    Ok(names)
}

/// Builds one fixture repository at `dest`.
pub fn build(workspace: &Workspace, name: &str, dest: &Path) -> std::io::Result<Built> {
    let script = script_for(name);
    if !script.is_file() {
        return Ok(Built::Skipped(format!(
            "no builder at {}",
            script.display()
        )));
    }
    std::fs::create_dir_all(dest)?;
    let mut cmd = Command::new("sh");
    cmd.arg(&script).arg(dest).stdin(Stdio::null());
    workspace.apply(&mut cmd);
    let out = cmd.output()?;
    let code = crate::run::exit_code(&out.status);
    if code == SKIP_STATUS {
        let reason = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Ok(Built::Skipped(if reason.is_empty() {
            "builder reported the fixture is unavailable here".to_string()
        } else {
            reason
        }));
    }
    if code != 0 {
        return Err(std::io::Error::other(format!(
            "fixture {name} failed with status {code}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(Built::Ok)
}

/// Builds the seeded random repository the property test draws from. The same seed
/// always produces the same repository, so a failure is reproducible from its seed.
pub fn build_random(workspace: &Workspace, dest: &Path, seed: u64) -> std::io::Result<Built> {
    let script = fixtures_dir().join("random.sh");
    std::fs::create_dir_all(dest)?;
    let mut cmd = Command::new("sh");
    cmd.arg(&script)
        .arg(dest)
        .arg(seed.to_string())
        .stdin(Stdio::null());
    workspace.apply(&mut cmd);
    let out = cmd.output()?;
    let code = crate::run::exit_code(&out.status);
    if code == SKIP_STATUS {
        return Ok(Built::Skipped(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    if code != 0 {
        return Err(std::io::Error::other(format!(
            "random fixture with seed {seed} failed with status {code}\n{}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(Built::Ok)
}

/// Refreshes a repository's index stat cache after it was copied. A copy has new
/// inodes, so without this every entry looks stat-dirty and an implementation that
/// verifies through the source index would decline to accelerate anything.
pub fn refresh_index(workspace: &Workspace, repo: &Path) {
    let _ = crate::run::git_full(workspace, repo, &["update-index", "--refresh", "-q"]);
}
