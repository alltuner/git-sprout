// ABOUTME: Turns two tree listings and the source index into the set of paths to clone.
// ABOUTME: Nothing enters the plan that has not passed every check in `verify`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use gix_hash::ObjectId;

use crate::tree::{directory_of, Listing};
use crate::verify::{self, Verdict};

/// A path the plan intends to materialise by cloning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub path: Vec<u8>,
    /// The mode from the tree. Never the mode the source file happens to carry.
    pub mode: u32,
    pub oid: ObjectId,
    /// The source file's modification time, as the source index records it.
    pub mtime: gix_index::entry::stat::Time,
}

/// What the verification pass concluded about the target tree.
#[derive(Debug, Default)]
pub struct Verified {
    /// Paths whose source file may stand in for a checkout.
    pub paths: Vec<Planned>,
    /// Paths whose stat data fell in the racily-clean window, for git to settle.
    pub racy: Vec<Planned>,
    /// How many blobs the target tree holds in total.
    pub considered: usize,
}

/// The work to do, once the racy paths have been settled and directories chosen.
#[derive(Debug, Default)]
pub struct Plan {
    /// Subtrees to clone in a single call, topmost only, in tree order.
    pub directories: Vec<Vec<u8>>,
    /// Paths to clone one at a time, because no chosen directory covers them.
    pub files: Vec<Planned>,
    /// Every path the plan materialises, including those inside a cloned directory.
    pub materialised: Vec<Planned>,
}

/// Applies every check that does not depend on git re-reading a file's content.
pub fn verify_paths(
    target: &Listing,
    source_index: &gix_index::File,
    source_root: &Path,
    poisoned: &[Vec<u8>],
) -> Verified {
    let mut verified = Verified {
        considered: target.blobs.len(),
        ..Verified::default()
    };
    let timestamp = source_index.timestamp();

    for (path, blob) in &target.blobs {
        if verify::is_poisoned(path, poisoned) {
            continue;
        }
        let Some(entry) = source_index.entry_by_path(path.as_slice().into()) else {
            continue;
        };
        if !verify::entry_can_stand_in(blob, entry) {
            continue;
        }
        let Ok(metadata) =
            gix_index::fs::Metadata::from_path_no_follow(&source_root.join(as_path(path)))
        else {
            continue;
        };
        let planned = Planned {
            path: path.clone(),
            mode: blob.mode,
            oid: blob.oid,
            mtime: entry.stat.mtime,
        };
        match verify::stat_verdict(entry, &metadata, timestamp) {
            Verdict::Clone => verified.paths.push(planned),
            Verdict::AskGit => verified.racy.push(planned),
            Verdict::Reject => {}
        }
    }
    verified
}

/// Chooses the subtrees worth cloning in one call and splits the rest into single files.
///
/// A subtree qualifies only when both sides record the same tree oid, every blob under it
/// is verified, and the source directory holds exactly the tracked entries and nothing
/// else. That last condition is what keeps untracked and ignored files out of the new
/// worktree: a directory clone copies whatever is on disk, so anything on disk that the
/// tree does not name disqualifies the whole subtree.
pub fn assemble(
    target: &Listing,
    source: &Listing,
    source_root: &Path,
    destination: &Path,
    verified: Vec<Planned>,
    clone_directories: bool,
) -> Plan {
    let mut plan = Plan {
        materialised: verified,
        ..Plan::default()
    };
    let verified_paths: HashSet<&[u8]> = plan
        .materialised
        .iter()
        .map(|planned| planned.path.as_slice())
        .collect();

    if clone_directories {
        let children = children_by_directory(target);
        let mut covered: Vec<Vec<u8>> = Vec::new();
        for (path, oid) in &target.trees {
            if covered.iter().any(|prefix| path.starts_with(prefix)) {
                continue;
            }
            if source.trees.get(path) != Some(oid) {
                continue;
            }
            if !subtree_is_fully_verified(target, path, &verified_paths) {
                continue;
            }
            if destination.join(as_path(path)).exists() {
                continue;
            }
            if !source_holds_only(source_root, path, &children) {
                continue;
            }
            let mut prefix = path.clone();
            prefix.push(b'/');
            covered.push(prefix);
            plan.directories.push(path.clone());
        }
        plan.files = plan
            .materialised
            .iter()
            .filter(|planned| {
                !covered
                    .iter()
                    .any(|prefix| planned.path.starts_with(prefix))
            })
            .cloned()
            .collect();
    } else {
        plan.files = plan.materialised.clone();
    }

    plan
}

/// Every blob under `directory` is verified, there is at least one, and no submodule sits
/// inside it.
fn subtree_is_fully_verified(
    target: &Listing,
    directory: &[u8],
    verified: &HashSet<&[u8]>,
) -> bool {
    let mut prefix = directory.to_vec();
    prefix.push(b'/');
    let mut blobs = 0usize;
    for (path, _) in target.blobs.range(prefix.clone()..) {
        if !path.starts_with(&prefix) {
            break;
        }
        if !verified.contains(path.as_slice()) {
            return false;
        }
        blobs += 1;
    }
    if blobs == 0 {
        return false;
    }
    if let Some(path) = target.gitlinks.range(prefix.clone()..).next() {
        if path.starts_with(&prefix) {
            return false;
        }
    }
    true
}

/// The immediate children of every directory in the tree, keyed by the directory path.
fn children_by_directory(listing: &Listing) -> HashMap<Vec<u8>, HashSet<Vec<u8>>> {
    let mut children: HashMap<Vec<u8>, HashSet<Vec<u8>>> = HashMap::new();
    let paths = listing
        .blobs
        .keys()
        .chain(listing.trees.keys())
        .chain(listing.gitlinks.iter());
    for path in paths {
        let directory = directory_of(path);
        let name = path[directory.len()..].to_vec();
        children
            .entry(directory.strip_suffix(b"/").unwrap_or(directory).to_vec())
            .or_default()
            .insert(name);
    }
    children
}

/// Whether the source directory holds exactly the tracked entries, at every depth.
fn source_holds_only(
    source_root: &Path,
    directory: &[u8],
    children: &HashMap<Vec<u8>, HashSet<Vec<u8>>>,
) -> bool {
    let Some(expected) = children.get(directory) else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(source_root.join(as_path(directory))) else {
        return false;
    };
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    for entry in entries {
        let Ok(entry) = entry else { return false };
        seen.insert(file_name_bytes(&entry.file_name()));
    }
    if &seen != expected {
        return false;
    }
    for name in expected {
        let mut child = directory.to_vec();
        child.push(b'/');
        child.extend_from_slice(name);
        if children.contains_key(&child) && !source_holds_only(source_root, &child, children) {
            return false;
        }
    }
    true
}

#[cfg(unix)]
fn file_name_bytes(name: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    name.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn file_name_bytes(name: &std::ffi::OsStr) -> Vec<u8> {
    name.to_string_lossy().into_owned().into_bytes()
}

/// Turns a repository-relative git path into a filesystem path.
#[cfg(unix)]
pub fn as_path(path: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(path))
}

#[cfg(not(unix))]
pub fn as_path(path: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(path).into_owned())
}

/// The `.gitattributes` blobs the source index records, split into the ones its stat cache
/// vouches for and the ones only git can settle by re-reading the file.
pub fn source_attribute_files(
    source_index: &gix_index::File,
    source_root: &Path,
) -> (BTreeMap<Vec<u8>, ObjectId>, Vec<Vec<u8>>) {
    let mut files = BTreeMap::new();
    let mut suspect = Vec::new();
    let timestamp = source_index.timestamp();
    for entry in source_index.entries() {
        let path = entry.path(source_index).to_vec();
        if !crate::tree::is_attributes_file(&path) {
            continue;
        }
        let verdict =
            gix_index::fs::Metadata::from_path_no_follow(&source_root.join(as_path(&path)))
                .map(|metadata| verify::stat_verdict(entry, &metadata, timestamp))
                .unwrap_or(Verdict::Reject);
        files.insert(path.clone(), entry.id);
        if verdict != Verdict::Clone {
            suspect.push(path);
        }
    }
    (files, suspect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::Blob;

    fn oid(byte: u8) -> ObjectId {
        ObjectId::from_hex(format!("{:02x}", byte).repeat(20).as_bytes()).unwrap()
    }

    fn listing(blobs: &[(&str, u8)], trees: &[(&str, u8)], gitlinks: &[&str]) -> Listing {
        Listing {
            blobs: blobs
                .iter()
                .map(|(path, byte)| {
                    (
                        path.as_bytes().to_vec(),
                        Blob {
                            mode: 0o100644,
                            oid: oid(*byte),
                        },
                    )
                })
                .collect(),
            trees: trees
                .iter()
                .map(|(path, byte)| (path.as_bytes().to_vec(), oid(*byte)))
                .collect(),
            gitlinks: gitlinks
                .iter()
                .map(|path| path.as_bytes().to_vec())
                .collect(),
        }
    }

    #[test]
    fn maps_every_directory_to_its_own_children() {
        let listing = listing(
            &[("src/a.txt", 1), ("src/deep/b.txt", 2), ("top.txt", 3)],
            &[("src", 4), ("src/deep", 5)],
            &[],
        );
        let children = children_by_directory(&listing);
        assert_eq!(
            children[b"".as_slice()],
            HashSet::from([b"src".to_vec(), b"top.txt".to_vec()])
        );
        assert_eq!(
            children[b"src".as_slice()],
            HashSet::from([b"a.txt".to_vec(), b"deep".to_vec()])
        );
    }

    #[test]
    fn a_subtree_needs_every_blob_verified() {
        let listing = listing(&[("src/a.txt", 1), ("src/b.txt", 2)], &[("src", 4)], &[]);
        let all = HashSet::from([b"src/a.txt".as_slice(), b"src/b.txt".as_slice()]);
        assert!(subtree_is_fully_verified(&listing, b"src", &all));
        let partial = HashSet::from([b"src/a.txt".as_slice()]);
        assert!(!subtree_is_fully_verified(&listing, b"src", &partial));
    }

    #[test]
    fn a_subtree_with_a_submodule_is_never_cloned_whole() {
        let listing = listing(&[("src/a.txt", 1)], &[("src", 4)], &["src/vendor"]);
        let all = HashSet::from([b"src/a.txt".as_slice()]);
        assert!(!subtree_is_fully_verified(&listing, b"src", &all));
    }

    #[test]
    fn an_empty_subtree_is_not_worth_cloning() {
        let listing = listing(&[], &[("src", 4)], &[]);
        assert!(!subtree_is_fully_verified(
            &listing,
            b"src",
            &HashSet::new()
        ));
    }

    #[test]
    fn an_untracked_file_disqualifies_the_directory_clone() {
        let scratch = std::env::temp_dir().join("git-sprout-plan-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(scratch.join("src")).unwrap();
        std::fs::write(scratch.join("src/a.txt"), "a").unwrap();
        let listing = listing(&[("src/a.txt", 1)], &[("src", 4)], &[]);
        let children = children_by_directory(&listing);
        assert!(source_holds_only(&scratch, b"src", &children));

        std::fs::write(scratch.join("src/untracked.log"), "x").unwrap();
        assert!(!source_holds_only(&scratch, b"src", &children));
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
