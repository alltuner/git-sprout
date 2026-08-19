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
