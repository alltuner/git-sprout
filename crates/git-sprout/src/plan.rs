// ABOUTME: Turns two tree listings and the source index into the set of paths to clone.
// ABOUTME: Nothing enters the plan that has not passed every check in `verify`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use gix_hash::ObjectId;

use crate::attributes;
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
    /// The source file's size on disk, which for a symlink is the length of its target.
    pub size: u64,
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
    /// Every directory those clones put on disk, roots included. A clone copies the
    /// source's permissions, so each of these needs the ones a checkout would have given
    /// it.
    pub directories_created: Vec<Vec<u8>>,
    /// Paths to clone one at a time, because no chosen directory covers them.
    pub files: Vec<Planned>,
    /// Every path the plan materialises, including those inside a cloned directory.
    pub materialised: Vec<Planned>,
}

/// The paths and subtrees a filesystem that folds case cannot keep apart.
///
/// Where two tracked paths differ only by case, only one file can exist, and real
/// `git worktree add` settles which by checking every entry out in index order: the last
/// one written unlinks and recreates the shared file, so it decides both the name on disk
/// and the content, and the other path is reported modified. Cloning even one member wins
/// a collision git would have lost, which spec §3.5.1 forbids, and inverts the dirty set.
/// So a whole group goes to git, which settles it exactly as it always would.
///
/// The fold is ASCII, which is what git's own case-insensitive comparisons use. A
/// filesystem that also folds beyond ASCII, as APFS does, can still collide on paths this
/// leaves in the plan; the clone of the second one fails and demotes the run, which is
/// slow rather than wrong.
///
/// On a case-sensitive filesystem this costs a handful of paths and changes nothing else.
pub fn colliding_paths(
    target: &Listing,
    source_paths: &[Vec<u8>],
) -> (HashSet<Vec<u8>>, Vec<Vec<u8>>) {
    // Keyed on the folded name, holding the distinct spellings seen under it. A set
    // rather than a list because the same path arrives from both the target tree and
    // the source index, and counting one path twice would make every path in the
    // repository look like a collision with itself.
    let mut by_folded: HashMap<Vec<u8>, HashSet<Vec<u8>>> = HashMap::new();
    // Folded over the source's paths as well as the target's, because the hazard lives
    // in the source's filesystem rather than in either tree. Where two paths differ only
    // by case the operating system keeps one file for both names, and the bytes behind
    // the surviving name may belong to either member. A pair that exists at the source's
    // HEAD but not in the target — a detached checkout, a tag, any commit where one
    // member was added or removed — is invisible to the target alone, and the surviving
    // path then looks like an ordinary clonable file whose content is somebody else's.
    //
    // Widening costs a handful of clones that would have been safe. The alternative
    // costs a worktree that differs from git's.
    let named = target
        .blobs
        .keys()
        .chain(target.trees.keys())
        .chain(target.gitlinks.iter())
        .chain(source_paths.iter());
    for path in named {
        by_folded
            .entry(path.to_ascii_lowercase())
            .or_default()
            .insert(path.clone());
    }

    let mut paths = HashSet::new();
    let mut prefixes = Vec::new();
    for (_, group) in by_folded.into_iter().filter(|(_, group)| group.len() > 1) {
        for path in group {
            // A source-only path is not in the plan to begin with; naming it here would
            // be harmless but misleading. What matters is that its *target* twin is.
            if target.trees.contains_key(&path) {
                let mut prefix = path.clone();
                prefix.push(b'/');
                prefixes.push(prefix);
            }
            paths.insert(path);
        }
    }
    prefixes.sort();
    (paths, prefixes)
}

/// Applies every check that does not depend on git re-reading a file's content.
pub fn verify_paths(
    target: &Listing,
    source_index: &gix_index::File,
    source_root: &Path,
    poisoned: &[Vec<u8>],
    excluded: &HashSet<Vec<u8>>,
) -> Verified {
    let mut verified = Verified {
        considered: target.blobs.len(),
        ..Verified::default()
    };
    let timestamp = source_index.timestamp();

    for (path, blob) in &target.blobs {
        if excluded.contains(path) || verify::is_poisoned(path, poisoned) {
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
            size: metadata.len(),
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
            plan.directories_created.push(path.clone());
            plan.directories_created.extend(
                target
                    .trees
                    .range(prefix.clone()..)
                    .take_while(|(under, _)| under.starts_with(&prefix))
                    .map(|(under, _)| under.clone()),
            );
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

/// The `.gitattributes` blobs the source index records, and the ones among them whose
/// rules the source does not read as the blob states them.
///
/// A checkout of the target reads each directory's rules from the blob, and the plan has
/// the source's own word for them only where the source reads the same thing. Git reads
/// the source's from the working copy when there is one, and nothing the index keeps
/// vouches for that copy's bytes. The stat cache and `git diff-files` compare it with the
/// blob only after cleaning it, so a copy staged through a clean filter can hold rules its
/// blob does not and still be clean, and git checks neither for an entry marked
/// `--assume-unchanged` or `--skip-worktree`. So every copy is compared with its blob
/// directly, by [`attributes::same_rules`], whatever the entry's flags and stat data say.
///
/// A missing copy is alike, since git then reads the rules from the index. One that is not
/// a regular file or cannot be read is not. An unmerged path has a blob per side and no
/// one of them to compare with, so it is kept out of the map and always distrusted.
pub fn source_attribute_files(
    source_index: &gix_index::File,
    source_root: &Path,
) -> (BTreeMap<Vec<u8>, ObjectId>, Vec<Vec<u8>>) {
    let mut files = BTreeMap::new();
    let mut distrusted: Vec<Vec<u8>> = Vec::new();
    for entry in source_index.entries() {
        let path = entry.path(source_index).to_vec();
        if !crate::tree::is_attributes_file(&path) {
            continue;
        }
        if entry.stage() != gix_index::entry::Stage::Unconflicted {
            if distrusted.last() != Some(&path) {
                distrusted.push(path);
            }
            continue;
        }
        let copy = source_root.join(as_path(&path));
        let alike = match std::fs::symlink_metadata(&copy) {
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
            Ok(metadata) if metadata.is_file() => {
                std::fs::read(&copy).is_ok_and(|bytes| attributes::same_rules(&bytes, &entry.id))
            }
            Ok(_) => false,
        };
        if !alike {
            distrusted.push(path.clone());
        }
        files.insert(path, entry.id);
    }
    (files, distrusted)
}

/// The attributes files in the source's working tree that its index does not record, in
/// the directories of the target tree.
///
/// Git reads a directory's `.gitattributes` from the working tree whether or not it is
/// tracked, so an untracked or ignored one shapes what `git check-attr` reports for the
/// source while a checkout of the target never sees it. Its directory is then no more
/// alike on both sides than one whose tracked attributes differ.
pub fn untracked_attribute_files(
    target: &Listing,
    source_attributes: &BTreeMap<Vec<u8>, ObjectId>,
    source_root: &Path,
) -> Vec<Vec<u8>> {
    let root: &[u8] = &[];
    std::iter::once(root)
        .chain(target.trees.keys().map(Vec::as_slice))
        .map(|directory| {
            let mut path = directory.to_vec();
            if !path.is_empty() {
                path.push(b'/');
            }
            path.extend_from_slice(crate::tree::ATTRIBUTES_FILE);
            path
        })
        .filter(|path| {
            !source_attributes.contains_key(path)
                && std::fs::symlink_metadata(source_root.join(as_path(path))).is_ok()
        })
        .collect()
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

    /// A pair that exists in the source but not in the target is still a collision:
    /// the source's filesystem keeps one file for both names, so the survivor's bytes
    /// may belong to either member. Regression for a divergence seen only under
    /// `--detach` and a tag, where the target resolves to a commit the pair straddles.
    #[test]
    fn a_pair_only_the_source_has_still_disqualifies_the_target_twin() {
        let listing = listing(&[("net/xt_mark.h", 0)], &[], &[]);
        let (bare, _) = colliding_paths(&listing, &[]);
        assert!(
            bare.is_empty(),
            "the target alone names no pair, so nothing collides"
        );

        let source = vec![b"net/xt_mark.h".to_vec(), b"net/XT_MARK.h".to_vec()];
        let (widened, _) = colliding_paths(&listing, &source);
        assert!(
            widened.contains(b"net/xt_mark.h".as_slice()),
            "the target's member of a pair the source holds must be dropped from the plan"
        );
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
    fn paths_that_differ_only_by_case_all_go_to_git() {
        let listing = listing(
            &[
                ("net/xt_MARK.c", 1),
                ("net/xt_mark.c", 2),
                ("net/other.c", 3),
            ],
            &[("net", 4)],
            &[],
        );
        let (paths, prefixes) = colliding_paths(&listing, &[]);
        assert!(paths.contains(b"net/xt_MARK.c".as_slice()));
        assert!(paths.contains(b"net/xt_mark.c".as_slice()));
        assert!(!paths.contains(b"net/other.c".as_slice()));
        assert!(prefixes.is_empty());
    }

    #[test]
    fn directories_that_differ_only_by_case_take_their_subtrees_with_them() {
        let listing = listing(
            &[("Net/a.c", 1), ("net/b.c", 2)],
            &[("Net", 3), ("net", 4)],
            &[],
        );
        let (paths, prefixes) = colliding_paths(&listing, &[]);
        assert!(paths.contains(b"Net".as_slice()));
        assert_eq!(prefixes, vec![b"Net/".to_vec(), b"net/".to_vec()]);
    }

    #[test]
    fn a_tree_and_a_blob_that_fold_together_both_go_to_git() {
        let listing = listing(&[("Doc", 1), ("doc/a.c", 2)], &[("doc", 3)], &[]);
        let (paths, prefixes) = colliding_paths(&listing, &[]);
        assert!(paths.contains(b"Doc".as_slice()));
        assert!(paths.contains(b"doc".as_slice()));
        assert_eq!(prefixes, vec![b"doc/".to_vec()]);
    }

    /// Git reads a `.gitattributes` from the source's working tree even where the index
    /// records none, as an untracked or ignored file. A checkout of the target reads only
    /// the target's own, so what such a file says about a path is no guide to the
    /// destination.
    #[test]
    fn reports_attributes_files_only_the_source_working_tree_holds() {
        let scratch = std::env::temp_dir().join(format!(
            "git-sprout-untracked-attributes-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        for directory in ["src", "docs", "tracked"] {
            std::fs::create_dir_all(scratch.join(directory)).unwrap();
        }
        std::fs::write(scratch.join("src/.gitattributes"), "*.txt -text\n").unwrap();
        std::fs::write(scratch.join("tracked/.gitattributes"), "*.txt -text\n").unwrap();
        let target = listing(
            &[
                ("src/a.txt", 1),
                ("docs/b.txt", 2),
                ("tracked/c.txt", 3),
                ("tracked/.gitattributes", 5),
            ],
            &[("src", 6), ("docs", 7), ("tracked", 8)],
            &[],
        );
        let recorded: BTreeMap<Vec<u8>, ObjectId> =
            [(b"tracked/.gitattributes".to_vec(), oid(5))].into();

        assert_eq!(
            untracked_attribute_files(&target, &recorded, &scratch),
            vec![b"src/.gitattributes".to_vec()]
        );

        std::fs::write(scratch.join(".gitattributes"), "* -text\n").unwrap();
        assert_eq!(
            untracked_attribute_files(&target, &recorded, &scratch),
            vec![b".gitattributes".to_vec(), b"src/.gitattributes".to_vec()]
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A scratch repository, and git run in it whatever repository, index and
    /// configuration the test itself runs under: a hook exports `GIT_DIR` and
    /// `GIT_INDEX_FILE`, and a machine's own configuration can pick another hash, add
    /// hooks or convert line endings.
    struct Scratch {
        root: PathBuf,
        repo: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("git-sprout-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let scratch = Scratch {
                repo: root.join("repo"),
                root,
            };
            std::fs::create_dir_all(&scratch.repo).expect("mkdir");
            std::fs::write(scratch.root.join("gitconfig"), "").expect("write");
            scratch.git(&["init", "-q", "--object-format=sha1"], b"");
            scratch.git(&["config", "core.autocrlf", "false"], b"");
            scratch
        }

        fn git(&self, args: &[&str], input: &[u8]) -> String {
            use std::io::Write;
            let mut child = std::process::Command::new("git")
                .arg("-C")
                .arg(&self.repo)
                .args(args)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("git");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(input)
                .expect("write");
            let output = child.wait_with_output().expect("git");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("utf-8")
                .trim()
                .to_string()
        }

        fn rules(&self, directory: &str) -> PathBuf {
            self.repo.join(directory).join(".gitattributes")
        }

        fn write(&self, directory: &str, content: &str) {
            std::fs::create_dir_all(self.repo.join(directory)).expect("mkdir");
            std::fs::write(self.rules(directory), content).expect("write");
        }

        fn source_attribute_files(&self) -> (BTreeMap<Vec<u8>, ObjectId>, Vec<Vec<u8>>) {
            let index = gix_index::File::at(
                self.repo.join(".git").join("index"),
                gix_hash::Kind::Sha1,
                true,
                gix_index::decode::Options::default(),
            )
            .expect("read index");
            source_attribute_files(&index, &self.repo)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// Git reads an attributes file's rules from its working copy, and nothing the index
    /// keeps vouches for that copy's bytes: one staged through a clean filter that has
    /// since been switched off is clean to git, and git does not look at one whose entry
    /// is marked `--assume-unchanged` or `--skip-worktree`. A copy that differs only in
    /// the line endings a checkout writes, or in its timestamp, holds the same rules, and
    /// git reads a missing copy from the index.
    #[test]
    fn attributes_files_unlike_their_blobs_are_distrusted() {
        let scratch = Scratch::new("unlike-attributes");
        for directory in [
            "same", "crlf", "touched", "gone", "sparse", "assumed", "skipped", "linked",
        ] {
            scratch.write(directory, "*.txt text\n");
        }
        scratch.write("filtered", "*.txt text\n.gitattributes filter=rules\n");
        scratch.git(&["config", "filter.rules.clean", "sed s/-text/text/g"], b"");
        scratch.git(&["add", "--", "."], b"");

        scratch.write("crlf", "*.txt text\r\n");
        scratch.write("touched", "*.txt text\n");
        std::fs::remove_file(scratch.rules("gone")).expect("remove");
        scratch.git(
            &[
                "update-index",
                "--skip-worktree",
                "--",
                "sparse/.gitattributes",
            ],
            b"",
        );
        std::fs::remove_file(scratch.rules("sparse")).expect("remove");
        scratch.write("assumed", "*.txt -text\n");
        scratch.git(
            &[
                "update-index",
                "--assume-unchanged",
                "--",
                "assumed/.gitattributes",
            ],
            b"",
        );
        scratch.write("skipped", "*.txt -text\n");
        scratch.git(
            &[
                "update-index",
                "--skip-worktree",
                "--",
                "skipped/.gitattributes",
            ],
            b"",
        );
        // Cleaning this copy gives the blob back, so adding it stages nothing; switching
        // the filter off afterwards leaves a clean entry over bytes the blob does not hold.
        scratch.write("filtered", "*.txt -text\n.gitattributes filter=rules\n");
        scratch.git(&["add", "--", "filtered/.gitattributes"], b"");
        std::fs::create_dir_all(scratch.repo.join(".git/info")).expect("mkdir");
        std::fs::write(
            scratch.repo.join(".git/info/attributes"),
            "filtered/.gitattributes -filter\n",
        )
        .expect("write info/attributes");
        scratch.write("added", "*.txt -text\n");
        scratch.git(
            &["add", "--intent-to-add", "--", "added/.gitattributes"],
            b"",
        );
        // Git does not follow a link in place of an attributes file, and reads the index.
        // That is the blob too, but a copy that is no regular file is not worth telling
        // apart from one git cannot read.
        #[cfg(unix)]
        {
            std::fs::remove_file(scratch.rules("linked")).expect("remove");
            std::os::unix::fs::symlink("../same/.gitattributes", scratch.rules("linked"))
                .expect("symlink");
        }
        let unlike: Vec<Vec<u8>> = [
            &b"added/.gitattributes"[..],
            b"assumed/.gitattributes",
            b"filtered/.gitattributes",
            #[cfg(unix)]
            b"linked/.gitattributes",
            b"skipped/.gitattributes",
        ]
        .iter()
        .map(|path| path.to_vec())
        .collect();

        let (recorded, distrusted) = scratch.source_attribute_files();

        assert_eq!(recorded.len(), 10);
        assert_eq!(distrusted, unlike);
    }

    /// An attributes file left unmerged has a blob per side and no one of them that says
    /// what rules git reads, so it is distrusted whatever its working copy holds.
    #[test]
    fn an_unmerged_attributes_file_is_distrusted() {
        let scratch = Scratch::new("unmerged-attributes");
        let blob =
            |content: &str| scratch.git(&["hash-object", "-w", "--stdin"], content.as_bytes());
        let (base, ours, theirs) = (blob("base\n"), blob("ours\n"), blob("theirs\n"));
        let entries = format!(
            "100644 {base} 0\t.gitattributes\n\
             100644 {base} 1\tsub/.gitattributes\n\
             100644 {ours} 2\tsub/.gitattributes\n\
             100644 {theirs} 3\tsub/.gitattributes\n"
        );
        scratch.git(&["update-index", "--index-info"], entries.as_bytes());
        // Resolved by hand to one side and not yet added, and no working copy at the
        // root, which git then reads from the index.
        scratch.write("sub", "theirs\n");

        let (recorded, distrusted) = scratch.source_attribute_files();

        assert_eq!(
            recorded.keys().collect::<Vec<_>>(),
            vec![&b".gitattributes".to_vec()]
        );
        assert_eq!(distrusted, vec![b"sub/.gitattributes".to_vec()]);
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
