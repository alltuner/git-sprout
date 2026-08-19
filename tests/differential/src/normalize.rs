// ABOUTME: Rewrites the few things that legitimately differ between two sides of a
// ABOUTME: comparison - absolute paths and reflog timestamps - into stable tokens.

use crate::stats::STDERR_PREFIX;
use std::path::{Path, PathBuf};

/// The substitutions applied before any byte comparison. Everything not listed here
/// is compared exactly, so the list is deliberately short and each entry is a
/// difference the compatibility contract does not cover.
pub struct Normaliser {
    /// Longest first, so a nested path never gets partially rewritten.
    replacements: Vec<(Vec<u8>, &'static str)>,
}

impl Normaliser {
    pub fn new(side_root: &Path, workspace_root: &Path) -> Normaliser {
        let mut replacements: Vec<(Vec<u8>, &'static str)> = Vec::new();
        for (path, token) in [(side_root, "<SIDE>"), (workspace_root, "<WORKSPACE>")] {
            for variant in path_variants(path) {
                replacements.push((
                    variant
                        .as_os_str()
                        .to_string_lossy()
                        .into_owned()
                        .into_bytes(),
                    token,
                ));
                let slashed = variant.to_string_lossy().replace('\\', "/");
                replacements.push((slashed.into_bytes(), token));
            }
        }
        replacements.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        replacements.dedup_by(|a, b| a.0 == b.0);
        Normaliser { replacements }
    }

    pub fn bytes(&self, input: &[u8]) -> Vec<u8> {
        let mut out = input.to_vec();
        for (needle, token) in &self.replacements {
            out = replace(&out, needle, token.as_bytes());
        }
        out
    }

    pub fn text(&self, input: &[u8]) -> String {
        String::from_utf8_lossy(&self.bytes(input)).into_owned()
    }

    /// stderr as a terminal would render it, minus the harness's own instrument and
    /// minus git's checkout progress meter.
    ///
    /// The `sprout-stats:` lines are dropped because the harness asked for them.
    ///
    /// The progress meter is dropped because §3.2's "byte for byte" cannot mean it.
    /// Git starts the meter only once a checkout has run for about two seconds, so
    /// whether it appears at all depends on the wall clock: two consecutive runs of
    /// real `git worktree add` on the kernel produced 124 and 118 frames, and a run
    /// fast enough produces none. A tool that leaves git almost nothing to write
    /// therefore never triggers it — the divergence is what success looks like.
    /// Everything else on stderr is still compared exactly; see
    /// [`Normaliser::malformed_progress`] for what is checked instead.
    pub fn stderr(&self, input: &[u8]) -> String {
        self.text(input)
            .lines()
            .filter(|line| !line.starts_with(STDERR_PREFIX))
            .map(last_frame)
            .filter(|line| !is_progress(line))
            .map(|line| format!("{line}\n"))
            .collect()
    }

    /// Progress frames that do not look like git's, so exempting the meter narrows
    /// the comparison rather than blinding it: anything else written where progress
    /// belongs is still a difference.
    pub fn malformed_progress(&self, input: &[u8]) -> Vec<String> {
        self.text(input)
            .lines()
            .flat_map(|line| line.split('\r').map(str::to_string).collect::<Vec<_>>())
            .filter(|frame| is_progress(frame) && !is_wellformed_progress(frame))
            .collect()
    }
}

/// A frame of git's checkout progress meter.
fn is_progress(line: &str) -> bool {
    line.starts_with("Updating files: ")
}

/// `Updating files:  47% (44676/95056)`, optionally finished with `, done.`
fn is_wellformed_progress(frame: &str) -> bool {
    let rest = frame.trim_start_matches("Updating files: ").trim_start();
    let Some((percent, rest)) = rest.split_once('%') else {
        return false;
    };
    if percent.is_empty() || !percent.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let rest = rest.trim_start().trim_end_matches(", done.");
    let Some(counts) = rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')) else {
        return false;
    };
    match counts.split_once('/') {
        Some((done, total)) => {
            !done.is_empty()
                && !total.is_empty()
                && done.chars().all(|c| c.is_ascii_digit())
                && total.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// The last carriage-return-separated frame of a line: what a terminal would still
/// be showing once the line is finished.
fn last_frame(line: &str) -> &str {
    line.rsplit('\r').next().unwrap_or(line)
}

/// A reflog line is `<old> <new> <ident> <epoch> <tz>\t<message>`. The epoch is
/// pinned by the harness environment but a tool that writes the reflog itself may
/// still land a second later, so the clock is tokenised and the rest is exact.
pub fn reflog_line(line: &str) -> String {
    let Some(tab) = line.find('\t') else {
        return line.to_string();
    };
    let (head, message) = line.split_at(tab);
    let mut fields: Vec<&str> = head.split(' ').collect();
    if fields.len() >= 2 {
        let tz = fields.len() - 1;
        let epoch = fields.len() - 2;
        if fields[tz].len() == 5
            && fields[epoch].chars().all(|c| c.is_ascii_digit())
            && !fields[epoch].is_empty()
        {
            fields[epoch] = "<EPOCH>";
            fields[tz] = "<TZ>";
        }
    }
    format!("{}{message}", fields.join(" "))
}

fn path_variants(path: &Path) -> Vec<PathBuf> {
    let mut out = vec![path.to_path_buf()];
    if let Ok(canonical) = std::fs::canonicalize(path) {
        if canonical != path {
            out.push(canonical);
        }
    }
    out
}

fn replace(haystack: &[u8], needle: &[u8], with: &[u8]) -> Vec<u8> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return haystack.to_vec();
    }
    let mut out = Vec::with_capacity(haystack.len());
    let mut i = 0;
    while i < haystack.len() {
        if haystack[i..].starts_with(needle) {
            out.extend_from_slice(with);
            i += needle.len();
        } else {
            out.push(haystack[i]);
            i += 1;
        }
    }
    out
}
