// ABOUTME: Writes the index that tells git which paths are already on disk and current.
// ABOUTME: Git replaces it with the real index at the end, so it carries no extensions.

use std::path::Path;

use gix_hash::ObjectId;
use gix_index::entry::{Flags, Mode, Stat};
use gix_index::{write, File, State};

/// One verified clone, described the way the index describes it.
#[derive(Debug, Clone)]
pub struct Record {
    pub path: Vec<u8>,
    /// The mode from the tree, which is what git would have written.
    pub mode: u32,
    pub oid: ObjectId,
    /// The stat data of the clone itself, so git sees the path as up to date.
    pub stat: Stat,
}

/// The error a scratch index write can fail with. Every one of them is survivable:
/// without the index git simply checks the whole tree out.
#[derive(Debug)]
pub enum Error {
    UnknownMode(u32),
    Write(gix_index::file::write::Error),
}

/// The only index version this writes. Git keeps whatever version it reads, so a
/// repository asking for any other version is left to check itself out.
pub const SUPPORTED_VERSION: u32 = 2;

/// The lowest and highest index versions git accepts.
const VERSION_RANGE: std::ops::RangeInclusive<u32> = 2..=4;

/// The version git would give a fresh index, following the same order git does.
///
/// This matters because git keeps the version of the index it reads: a scratch index in
/// the wrong version would decide the version of the final index too, which is observable.
/// `git worktree add --no-checkout` writes no index at all, so the answer has to come from
/// the environment and the configuration rather than from a file.
pub fn default_version(
    environment: Option<&str>,
    configured: Option<&str>,
    many_files: bool,
) -> u32 {
    let requested = environment
        .or(configured)
        .and_then(|value| value.trim().parse::<u32>().ok())
        .or_else(|| many_files.then_some(4));
    match requested {
        Some(version) if VERSION_RANGE.contains(&version) => version,
        _ => SUPPORTED_VERSION,
    }
}

/// Writes `records` as the index at `index_path`, replacing whatever is there.
pub fn write(
    index_path: &Path,
    object_hash: gix_hash::Kind,
    records: &[Record],
) -> Result<(), Error> {
    let mut state = State::new(object_hash);
    for record in records {
        let mode = Mode::from_bits(record.mode).ok_or(Error::UnknownMode(record.mode))?;
        state.dangerously_push_entry(
            record.stat,
            record.oid,
            Flags::empty(),
            mode,
            record.path.as_slice().into(),
        );
    }
    state.sort_entries();
    let mut file = File::from_state(state, index_path);
    file.write(write::Options {
        extensions: write::Extensions::None,
        skip_hash: false,
    })
    .map_err(Error::Write)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_repository_gets_version_two() {
        assert_eq!(default_version(None, None, false), 2);
    }

    #[test]
    fn configuration_and_environment_choose_the_version() {
        assert_eq!(default_version(None, Some("4"), false), 4);
        assert_eq!(default_version(Some("3"), Some("4"), false), 3);
    }

    #[test]
    fn many_files_asks_for_version_four() {
        assert_eq!(default_version(None, None, true), 4);
        assert_eq!(default_version(None, Some("2"), true), 2);
    }

    #[test]
    fn an_impossible_version_falls_back_to_the_default() {
        assert_eq!(default_version(None, Some("9"), false), 2);
        assert_eq!(default_version(None, Some("nonsense"), false), 2);
    }
}
