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
    pub skipped: u64,
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
    pub fn accelerated(&self) -> bool {
        self.cloned > 0 && !self.fell_back
    }
}
