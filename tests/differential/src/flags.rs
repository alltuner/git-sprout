// ABOUTME: The flag matrix: every argv shape the compatibility contract covers,
// ABOUTME: including the ones whose correct answer is an identical failure.

/// What the harness does to the destination before the add runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setup {
    None,
    /// Create the destination directory first, so the add has to refuse it.
    OccupyDestination,
}

#[derive(Debug, Clone)]
pub struct FlagCase {
    pub name: &'static str,
    /// Destination basename, relative to the repository's parent. Identical on both
    /// sides so that the admin directory name and the argv bytes also match.
    pub dest: &'static str,
    pub args: &'static [&'static str],
    pub setup: Setup,
}

impl FlagCase {
    /// The argv after `add`, with the destination expressed relative to the
    /// repository so that not one byte of the command line differs between sides.
    pub fn argv(&self) -> Vec<String> {
        self.args.iter().map(|a| a.to_string()).collect()
    }
}

macro_rules! case {
    ($name:literal, $dest:literal, [$($arg:literal),* $(,)?]) => {
        FlagCase { name: $name, dest: $dest, args: &[$($arg),*], setup: Setup::None }
    };
    ($name:literal, $dest:literal, [$($arg:literal),* $(,)?], $setup:expr) => {
        FlagCase { name: $name, dest: $dest, args: &[$($arg),*], setup: $setup }
    };
}

/// Cases run against every fixture. Where a case cannot apply to a fixture - an
/// `--orphan` against a repo that has commits, a tag that does not exist - both
/// sides are expected to fail identically, which is itself part of the contract.
pub const ALL: &[FlagCase] = &[
    case!("default", "wt", ["../wt"]),
    case!("new-branch", "wt", ["../wt", "-b", "feature"]),
    case!("new-branch-first", "wt", ["-b", "ordered", "../wt"]),
    case!("reset-branch", "wt", ["../wt", "-B", "existing"]),
    case!("detach", "wt", ["--detach", "../wt"]),
    case!("detach-parent", "wt", ["--detach", "../wt", "HEAD~1"]),
    case!("no-checkout", "wt", ["../wt", "--no-checkout", "-b", "nocheckout"]),
    case!("checkout", "wt", ["../wt", "--checkout", "-b", "withcheckout"]),
    case!("lock", "wt", ["../wt", "--lock", "-b", "locked"]),
    case!("lock-reason", "wt", ["../wt", "--lock", "--reason", "held by the suite", "-b", "lockedwhy"]),
    case!("quiet", "wt", ["-q", "../wt", "-b", "quiet"]),
    case!("force", "wt", ["-f", "../wt", "-b", "forced"]),
    case!("orphan", "wt", ["--orphan", "-b", "orphaned", "../wt"]),
    case!("orphan-no-branch", "wt", ["--orphan", "../wt"]),
    case!("commit-ish-head", "wt", ["../wt", "-b", "athead", "HEAD"]),
    case!("commit-ish-parent", "wt", ["../wt", "-b", "atparent", "HEAD~1"]),
    case!("commit-ish-tag", "wt", ["../wt", "-b", "attag", "v1"]),
    case!("detach-tag", "wt", ["--detach", "../wt", "v1"]),
    case!("track-remote", "wt", ["--track", "-b", "tracked", "../wt", "origin/main"]),
    case!("no-track-remote", "wt", ["--no-track", "-b", "untracked", "../wt", "origin/main"]),
    case!("guess-remote", "topic", ["--guess-remote", "../topic"]),
    case!("no-guess-remote", "topic", ["--no-guess-remote", "../topic"]),
    case!("relative-paths", "wt", ["../wt", "-b", "relative", "--relative-paths"]),
    case!("branch-collision", "wt", ["../wt", "-b", "existing"]),
    case!("unknown-commit-ish", "wt", ["../wt", "no-such-ref-anywhere"]),
    case!("destination-exists", "wt", ["../wt"], Setup::OccupyDestination),
];

/// A short matrix for the property test and for fixtures where the full matrix
/// would only repeat what the plain repository already proved.
pub const CORE: &[&str] = &["default", "new-branch", "detach", "no-checkout", "commit-ish-tag", "quiet"];

pub fn by_name(name: &str) -> Option<&'static FlagCase> {
    ALL.iter().find(|c| c.name == name)
}
