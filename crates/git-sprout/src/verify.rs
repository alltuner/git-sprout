// ABOUTME: Decides whether a source checkout's file may stand in for a fresh checkout.
// ABOUTME: The rule is index-based, so working-tree conversions never have to be replayed.

use std::collections::BTreeMap;

use filetime::FileTime;
use gix_hash::ObjectId;
use gix_index::entry::stat;

use crate::tree::{directory_of, Blob};

/// What may be done with one path from the target tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The source's bytes are, by git's own definition, what a checkout would write.
    Clone,
    /// The stat cache cannot settle it; git must compare the content before we trust it.
    AskGit,
    /// Not clonable. Git checks this path out itself.
    Reject,
}

/// The stat comparison git performs by default: whole-second mtime and ctime, size,
/// inode, uid and gid, and no device or nanosecond comparison.
fn stat_options() -> stat::Options {
    stat::Options::default()
}

/// Flags that mean git has stopped maintaining, or stopped trusting, an entry's stat data.
fn entry_is_untrustworthy(flags: gix_index::entry::Flags) -> bool {
    use gix_index::entry::Flags;
    flags.intersects(Flags::ASSUME_VALID | Flags::SKIP_WORKTREE | Flags::INTENT_TO_ADD)
}

/// Whether a tree mode names something the tool is willing to materialise.
pub fn mode_is_clonable(mode: u32) -> bool {
    matches!(mode, 0o100644 | 0o100755 | 0o120000)
}

/// The checks that need nothing from the filesystem.
///
/// See the module documentation on `stat_verdict` for why this is index-based.
pub fn entry_can_stand_in(target: &Blob, entry: &gix_index::Entry) -> bool {
    mode_is_clonable(target.mode)
        && entry.stage() == gix_index::entry::Stage::Unconflicted
        && !entry_is_untrustworthy(entry.flags)
        && entry.mode.bits() == target.mode
        && entry.id == target.oid
}

/// Judges the source file on disk against the index entry that describes it.
///
/// # Why this is index-based and not a hash of the clone
///
/// A checked-out file holds the blob *after* `.gitattributes` processing — CRLF
/// conversion, `ident`, `working-tree-encoding`, clean/smudge filters. Hashing the
/// source file and comparing it to the target blob oid would therefore reject every
/// converted path, which on a repo with `core.autocrlf=true` is every text file.
///
/// Git's own answer to "is this file an unmodified checkout of this oid?" is the index
/// stat cache, and it is the answer git acts on for every `status` and every `checkout`.
/// So if the source's entry is stat-clean and its oid is the blob we want, then the
/// source's bytes are what a checkout at the destination would write — conversions
/// included — provided the attributes governing the path are the same on both sides.
/// That last condition is checked separately, per subtree, by `poisoned_prefixes`.
///
/// # How far the stat cache is trusted
///
/// Exactly as far as git trusts it, and no further:
///
///   * Git compares whole-second mtime and ctime, size, inode, uid and gid. This does
///     the same, and does it regardless of what `core.checkStat` says, because a repo
///     configured to check less is a repo whose recorded stat data proves less.
///   * Entries flagged assume-unchanged, skip-worktree or intent-to-add are rejected
///     outright by `entry_can_stand_in`: each of those flags is git saying the stat data
///     does not describe the file on disk.
///   * Unmerged entries are rejected; they have no single checked-out content.
///
/// # The racily-clean window
///
/// A file written in the same second the index was written can be modified again inside
/// that same second and still stat identically, because mtime has one-second resolution
/// in the index. Git calls such an entry racily clean and re-reads the content instead of
/// trusting the stat. `Verdict::AskGit` is that case: the caller hands those few paths to
/// `git diff-files`, which re-reads them with the repository's filters applied and reports
/// the ones that really differ. Hashing them here would be wrong for exactly the reason
/// the whole rule is index-based — the working-tree bytes are not the blob bytes.
///
/// The window is not narrowed by us and must not be: git decides it, from the mtime it
/// recorded on the index file itself, and any smaller window would trust files git does
/// not.
pub fn stat_verdict(
    entry: &gix_index::Entry,
    metadata: &gix_index::fs::Metadata,
    index_timestamp: FileTime,
) -> Verdict {
    if metadata.is_dir() {
        return Verdict::Reject;
    }
    let Ok(on_disk) = gix_index::entry::Stat::from_fs(metadata) else {
        return Verdict::Reject;
    };
    if !on_disk.matches(&entry.stat, stat_options()) {
        return Verdict::Reject;
    }
    if entry.stat.is_racy(index_timestamp, stat_options()) {
        return Verdict::AskGit;
    }
    Verdict::Clone
}

/// Directory prefixes whose attributes differ between the two sides.
///
/// A path may only be cloned when every `.gitattributes` on its ancestry is the same blob
/// in both commits, because those files decide the conversion applied on checkout. Any
/// difference — a different blob, present on one side only, or modified in the source
/// working tree — disqualifies the whole directory it governs and everything below it.
/// Prefixes include their trailing slash; the empty prefix is the repository root and
/// therefore disqualifies everything.
pub fn poisoned_prefixes(
    target_attributes: &BTreeMap<Vec<u8>, ObjectId>,
    source_attributes: &BTreeMap<Vec<u8>, ObjectId>,
    dirty_attributes: &[Vec<u8>],
) -> Vec<Vec<u8>> {
    let mut poisoned: Vec<Vec<u8>> = Vec::new();
    let mut note = |path: &[u8]| poisoned.push(directory_of(path).to_vec());

    for (path, oid) in target_attributes {
        if source_attributes.get(path) != Some(oid) {
            note(path);
        }
    }
    for path in source_attributes.keys() {
        if !target_attributes.contains_key(path) {
            note(path);
        }
    }
    for path in dirty_attributes {
        note(path);
    }

    poisoned.sort();
    poisoned.dedup();
    poisoned
}

/// Whether a path lies under any disqualified directory.
pub fn is_poisoned(path: &[u8], poisoned: &[Vec<u8>]) -> bool {
    poisoned
        .iter()
        .any(|prefix| prefix.is_empty() || path.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(byte: u8) -> ObjectId {
        ObjectId::from_hex(format!("{:02x}", byte).repeat(20).as_bytes()).unwrap()
    }

    fn attributes(entries: &[(&str, u8)]) -> BTreeMap<Vec<u8>, ObjectId> {
        entries
            .iter()
            .map(|(path, byte)| (path.as_bytes().to_vec(), oid(*byte)))
            .collect()
    }

    #[test]
    fn only_regular_files_and_symlinks_are_clonable() {
        assert!(mode_is_clonable(0o100644));
        assert!(mode_is_clonable(0o100755));
        assert!(mode_is_clonable(0o120000));
        assert!(!mode_is_clonable(0o160000));
        assert!(!mode_is_clonable(0o040000));
    }

    #[test]
    fn identical_attributes_disqualify_nothing() {
        let both = attributes(&[(".gitattributes", 1), ("src/.gitattributes", 2)]);
        assert!(poisoned_prefixes(&both, &both, &[]).is_empty());
    }

    #[test]
    fn a_changed_attributes_file_disqualifies_its_directory() {
        let target = attributes(&[("src/.gitattributes", 1)]);
        let source = attributes(&[("src/.gitattributes", 2)]);
        let poisoned = poisoned_prefixes(&target, &source, &[]);
        assert_eq!(poisoned, vec![b"src/".to_vec()]);
        assert!(is_poisoned(b"src/a.txt", &poisoned));
        assert!(!is_poisoned(b"doc/a.txt", &poisoned));
    }

    #[test]
    fn a_root_attributes_file_disqualifies_everything() {
        let target = attributes(&[(".gitattributes", 1)]);
        let poisoned = poisoned_prefixes(&target, &BTreeMap::new(), &[]);
        assert!(is_poisoned(b"anything", &poisoned));
    }

    #[test]
    fn an_attributes_file_only_the_source_has_disqualifies_its_directory() {
        let source = attributes(&[("src/.gitattributes", 1)]);
        let poisoned = poisoned_prefixes(&BTreeMap::new(), &source, &[]);
        assert!(is_poisoned(b"src/a.txt", &poisoned));
    }

    #[test]
    fn a_modified_attributes_file_disqualifies_its_directory() {
        let both = attributes(&[("src/.gitattributes", 1)]);
        let poisoned = poisoned_prefixes(&both, &both, &[b"src/.gitattributes".to_vec()]);
        assert!(is_poisoned(b"src/a.txt", &poisoned));
    }
}
