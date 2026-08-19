// ABOUTME: Runs one worktree-add on one side of a comparison and captures every
// ABOUTME: observable it produces: streams, exit status, hooks fired and statistics.

use crate::env::Workspace;
use crate::stats::Stats;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Hooks git fires during a worktree add, per the semantics probe against git 2.55.0.
const LOGGED_HOOKS: &[&str] = &[
    "reference-transaction",
    "post-index-change",
    "post-checkout",
];

/// Which implementation materialises the worktree on a given side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tool {
    /// Real `git worktree add`.
    Git,
    /// `git-sprout add`, from the binary at this path.
    Sprout(PathBuf),
}

impl Tool {
    /// The candidate side. With `SPROUT_BIN` unset the harness compares real git
    /// against real git, which is how it stays exercisable and honest with no tool
    /// present. This mode is permanent, not a shim.
    pub fn candidate() -> Tool {
        match std::env::var_os("SPROUT_BIN") {
            Some(path) => Tool::Sprout(PathBuf::from(path)),
            None => Tool::Git,
        }
    }

    pub fn is_self_test(&self) -> bool {
        matches!(self, Tool::Git)
    }

    pub fn describe(&self) -> String {
        match self {
            Tool::Git => "git worktree add (self-test)".to_string(),
            Tool::Sprout(p) => format!("{} add", p.display()),
        }
    }
}

/// One side of a comparison: an independent copy of the fixture repository, the
/// destination the worktree is created at, and the logs the run writes.
pub struct Side {
    pub label: &'static str,
    pub root: PathBuf,
    pub repo: PathBuf,
    pub worktree: PathBuf,
    pub hook_log: PathBuf,
    pub stats_file: PathBuf,
}

impl Side {
    pub fn new(case_dir: &Path, label: &'static str, slot: &str) -> Side {
        let root = case_dir.join(slot);
        Side {
            label,
            repo: root.join("repo"),
            worktree: root.join("wt"),
            hook_log: root.join("hooks.log"),
            stats_file: root.join("stats.json"),
            root,
        }
    }
}

/// Everything one add produced.
#[derive(Debug, Clone, Default)]
pub struct RunOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: i32,
    pub hooks: Vec<String>,
    pub stats: Option<Stats>,
}

/// Installs the logging hooks in a repository. The log path is absolute because the
/// hooks run with the repository as their working directory, and a relative path
/// makes the reference-transaction hook fail, which aborts the whole add.
pub fn install_hooks(repo: &Path, log: &Path) -> std::io::Result<()> {
    let hooks_dir = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let log = log.to_string_lossy().replace('\\', "/");
    for hook in LOGGED_HOOKS {
        let script = format!(
            "#!/bin/sh\n\
             log='{log}'\n\
             printf '%s' '{hook}' >> \"$log\"\n\
             for a in \"$@\"; do printf ' %s' \"$a\" >> \"$log\"; done\n\
             printf '\\n' >> \"$log\"\n\
             while IFS= read -r line; do printf '%s stdin %s\\n' '{hook}' \"$line\" >> \"$log\"; done\n\
             exit 0\n"
        );
        let path = hooks_dir.join(hook);
        std::fs::write(&path, script)?;
        set_executable(&path)?;
    }
    Ok(())
}

fn set_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Runs the add on one side. `args` is the argv after `add`, identical on both
/// sides, with the destination expressed relative to the repository so that not
/// even the argument bytes differ.
pub fn worktree_add(
    workspace: &Workspace,
    side: &Side,
    tool: &Tool,
    args: &[String],
) -> std::io::Result<RunOutput> {
    let _ = std::fs::remove_file(&side.hook_log);
    let _ = std::fs::remove_file(&side.stats_file);

    let mut cmd = match tool {
        Tool::Git => {
            let mut c = Command::new("git");
            c.arg("worktree").arg("add");
            c
        }
        Tool::Sprout(bin) => {
            let mut c = Command::new(bin);
            c.arg("add");
            c
        }
    };
    cmd.args(args);
    cmd.current_dir(&side.repo);
    workspace.apply(&mut cmd);
    cmd.env("SPROUT_STATS", "1");
    cmd.env("SPROUT_STATS_FILE", &side.stats_file);
    cmd.stdin(Stdio::null());

    let out = cmd.output()?;
    let hooks = read_hook_log(&side.hook_log);
    let stats = Stats::collect(&side.stats_file, &out.stderr);
    Ok(RunOutput {
        stdout: out.stdout,
        stderr: out.stderr,
        status: exit_code(&out.status),
        hooks,
        stats,
    })
}

fn read_hook_log(path: &Path) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => text.lines().map(str::to_string).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn exit_code(status: &std::process::ExitStatus) -> i32 {
    status.code().unwrap_or_else(|| {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            status.signal().map(|s| 128 + s).unwrap_or(-1)
        }
        #[cfg(not(unix))]
        -1
    })
}

/// Runs a git command inside a repository under the harness environment and
/// returns its stdout. Used for the read-only observations a snapshot makes.
pub fn git(workspace: &Workspace, cwd: &Path, args: &[&str]) -> std::io::Result<Vec<u8>> {
    let out = git_full(workspace, cwd, args)?;
    Ok(out.stdout)
}

pub fn git_full(
    workspace: &Workspace,
    cwd: &Path,
    args: &[&str],
) -> std::io::Result<std::process::Output> {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(cwd).stdin(Stdio::null());
    workspace.apply(&mut cmd);
    cmd.output()
}

/// git stdout as a trimmed string, for single-value queries.
pub fn git_line(workspace: &Workspace, cwd: &Path, args: &[&str]) -> String {
    match git(workspace, cwd, args) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).trim().to_string(),
        Err(e) => format!("<error: {e}>"),
    }
}
