// ABOUTME: Reads `git ls-tree -r -t -z` into the path maps the clone plan works from.
// ABOUTME: Paths stay raw bytes because git path names are not required to be UTF-8.

use std::collections::{BTreeMap, BTreeSet};

use gix_hash::ObjectId;

/// A tracked file or symlink in a commit's tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    /// The mode recorded in the tree. Never taken from the filesystem.
    pub mode: u32,
    pub oid: ObjectId,
}

/// Everything one commit's tree contains, indexed by path.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Listing {
    pub blobs: BTreeMap<Vec<u8>, Blob>,
    pub trees: BTreeMap<Vec<u8>, ObjectId>,
    /// Submodule paths. These are never cloned; git materialises them.
    pub gitlinks: BTreeSet<Vec<u8>>,
}

/// The name git gives the per-directory attributes file.
pub const ATTRIBUTES_FILE: &[u8] = b".gitattributes";

impl Listing {
    /// The `.gitattributes` blobs in this tree, keyed by path.
    pub fn attribute_files(&self) -> BTreeMap<Vec<u8>, ObjectId> {
        self.blobs
            .iter()
            .filter(|(path, _)| is_attributes_file(path))
            .map(|(path, blob)| (path.clone(), blob.oid))
            .collect()
    }
}

/// Whether a repository-relative path names a per-directory attributes file.
pub fn is_attributes_file(path: &[u8]) -> bool {
    path == ATTRIBUTES_FILE
        || (path.len() > ATTRIBUTES_FILE.len()
            && path.ends_with(ATTRIBUTES_FILE)
            && path[path.len() - ATTRIBUTES_FILE.len() - 1] == b'/')
}

/// The directory part of a path, including the trailing slash. Empty at the root.
pub fn directory_of(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|byte| *byte == b'/') {
        Some(position) => &path[..position + 1],
        None => &[],
    }
}

/// Parses the NUL-separated records `git ls-tree -r -t -z` writes.
pub fn parse(output: &[u8]) -> Option<Listing> {
    let mut listing = Listing::default();
    for record in output.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        let tab = record.iter().position(|byte| *byte == b'\t')?;
        let (meta, path) = record.split_at(tab);
        let path = &path[1..];
        let mut fields = meta.split(|byte| *byte == b' ');
        let mode = std::str::from_utf8(fields.next()?).ok()?;
        let mode = u32::from_str_radix(mode, 8).ok()?;
        let kind = fields.next()?;
        let oid = std::str::from_utf8(fields.next()?).ok()?;
        let oid = ObjectId::from_hex(oid.as_bytes()).ok()?;
        match kind {
            b"blob" => {
                listing.blobs.insert(path.to_vec(), Blob { mode, oid });
            }
            b"tree" => {
                listing.trees.insert(path.to_vec(), oid);
            }
            b"commit" => {
                listing.gitlinks.insert(path.to_vec());
            }
            _ => return None,
        }
    }
    Some(listing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(mode: &str, kind: &str, oid: &str, path: &str) -> Vec<u8> {
        format!("{mode} {kind} {oid}\t{path}\0").into_bytes()
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn reads_blobs_trees_and_gitlinks() {
        let mut output = record("040000", "tree", A, "src");
        output.extend(record("100755", "blob", B, "src/run.sh"));
        output.extend(record("160000", "commit", A, "vendor/lib"));
        let listing = parse(&output).expect("parses");
        assert_eq!(listing.trees.len(), 1);
        assert_eq!(
            listing.blobs[b"src/run.sh".as_slice()],
            Blob {
                mode: 0o100755,
                oid: ObjectId::from_hex(B.as_bytes()).unwrap()
            }
        );
        assert!(listing.gitlinks.contains(b"vendor/lib".as_slice()));
    }

    #[test]
    fn keeps_paths_that_are_not_utf8() {
        let mut output = b"100644 blob ".to_vec();
        output.extend(A.as_bytes());
        output.extend(b"\ta/\xff\0");
        let listing = parse(&output).expect("parses");
        assert!(listing.blobs.contains_key(b"a/\xff".as_slice()));
    }

    #[test]
    fn recognises_attributes_files_at_any_depth() {
        assert!(is_attributes_file(b".gitattributes"));
        assert!(is_attributes_file(b"src/.gitattributes"));
        assert!(!is_attributes_file(b"src/not.gitattributes"));
        assert!(!is_attributes_file(b"gitattributes"));
    }

    #[test]
    fn splits_the_directory_from_the_path() {
        assert_eq!(directory_of(b"a/b/c.txt"), b"a/b/");
        assert_eq!(directory_of(b"c.txt"), b"");
    }
}
