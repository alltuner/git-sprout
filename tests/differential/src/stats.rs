// ABOUTME: Reads the machine-readable statistics git-sprout emits under SPROUT_STATS,
// ABOUTME: which is how a test proves acceleration happened rather than only that
// ABOUTME: the output was right.

use serde::Deserialize;
use std::path::Path;

/// The `sprout-stats:` line prefix used when no statistics file is requested.
pub const STDERR_PREFIX: &str = "sprout-stats: ";

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Stats {
    pub cloned: u64,
    /// Subtrees cloned in one call, where the platform offers a directory clone.
    /// It can stop firing without any other counter moving, so it is asserted on
    /// its own wherever the platform is expected to use it.
    pub cloned_directories: u64,
    pub skipped: u64,
    /// Not an independent measurement: it is `skipped` by construction, and both
    /// are zero when no plan ran at all. Never treat it as a third signal.
    pub checked_out_by_git: u64,
    pub source: Option<String>,
    pub clone_backend: Option<String>,
    pub fell_back: bool,
    pub fallback_reason: Option<String>,
}

impl Stats {
    /// Prefers the statistics file, falling back to the stderr line. Absence is not
    /// an error: in self-test mode real git emits nothing.
    pub fn collect(file: &Path, stderr: &[u8]) -> Option<Stats> {
        if let Ok(text) = std::fs::read_to_string(file) {
            if let Ok(stats) = serde_json::from_str::<Stats>(&text) {
                return Some(stats);
            }
        }
        let text = String::from_utf8_lossy(stderr);
        for line in text.lines() {
            if let Some(json) = line.strip_prefix(STDERR_PREFIX) {
                if let Ok(stats) = serde_json::from_str::<Stats>(json) {
                    return Some(stats);
                }
            }
        }
        None
    }

    /// True when the run actually used the copy-on-write path for some files.
    ///
    /// The test is `cloned`, not `fell_back`: a partial demotion sets
    /// `fallback_reason` while still having cloned most of the tree, and the
    /// question this answers is whether acceleration happened at all.
    /// False when the run fell back because the filesystem cannot clone blocks at
    /// all — ext4, NTFS, tmpfs. Acceleration is not assertable there, and the
    /// fallback being correct is what the rest of the suite checks.
    pub fn supports_cloning(&self) -> bool {
        match self.fallback_reason.as_deref() {
            Some(reason) => {
                !reason.contains("not supported") && !reason.contains("Operation not permitted")
            }
            None => true,
        }
    }

    pub fn accelerated(&self) -> bool {
        self.cloned > 0
    }
}
