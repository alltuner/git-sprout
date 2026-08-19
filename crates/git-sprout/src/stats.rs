// ABOUTME: Machine-readable statistics describing what a run cloned, skipped and
// ABOUTME: left to git, emitted when SPROUT_STATS=1 so tests can assert on them.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

/// Which block-cloning primitive a run used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneBackend {
    Apfs,
    Ficlone,
    Refs,
    None,
}

impl CloneBackend {
    fn as_str(self) -> &'static str {
        match self {
            CloneBackend::Apfs => "apfs",
            CloneBackend::Ficlone => "ficlone",
            CloneBackend::Refs => "refs",
            CloneBackend::None => "none",
        }
    }
}

/// What a single `git sprout add` did.
#[derive(Debug, Clone)]
pub struct Stats {
    /// Paths materialised by a block clone.
    pub cloned: usize,
    /// Subtrees cloned in a single call rather than file by file.
    pub cloned_directories: usize,
    /// Paths the plan considered and rejected, so git had to write them.
    pub skipped: usize,
    /// Paths in the target tree that git checked out itself. Zero when no plan ran.
    pub checked_out_by_git: usize,
    /// The checkout the clones came from.
    pub source: Option<PathBuf>,
    pub clone_backend: CloneBackend,
    /// True when the run produced its result through plain `git worktree add`.
    pub fell_back: bool,
    pub fallback_reason: Option<String>,
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            cloned: 0,
            cloned_directories: 0,
            skipped: 0,
            checked_out_by_git: 0,
            source: None,
            clone_backend: CloneBackend::None,
            fell_back: false,
            fallback_reason: None,
        }
    }
}

impl Stats {
    /// Records that the run produced its result through plain `git worktree add`.
    pub fn fall_back(&mut self, reason: impl Into<String>) {
        self.fell_back = true;
        self.fallback_reason = Some(reason.into());
    }

    fn to_json(&self) -> String {
        let value = serde_json::json!({
            "cloned": self.cloned,
            "cloned_directories": self.cloned_directories,
            "skipped": self.skipped,
            "checked_out_by_git": self.checked_out_by_git,
            "source": self.source.as_ref().map(|path| path.to_string_lossy()),
            "clone_backend": self.clone_backend.as_str(),
            "fell_back": self.fell_back,
            "fallback_reason": self.fallback_reason,
        });
        value.to_string()
    }

    /// Writes the statistics where SPROUT_STATS_FILE points, or to stderr.
    ///
    /// Failing to report statistics must never fail the operation, so every error
    /// here is dropped.
    pub fn emit(&self) {
        if std::env::var_os("SPROUT_STATS").is_none_or(|value| value != "1") {
            return;
        }
        let json = self.to_json();
        match std::env::var_os("SPROUT_STATS_FILE") {
            Some(path) => {
                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(file, "{json}");
                }
            }
            None => {
                let _ = writeln!(std::io::stderr(), "sprout-stats: {json}");
            }
        }
    }
}
