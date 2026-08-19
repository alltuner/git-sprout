// ABOUTME: The cloner for platforms with no block-cloning primitive at all.
// ABOUTME: Every call fails, which demotes the run to letting git write the tree.

use std::io;
use std::path::Path;

use super::BlockCloner;
use crate::stats::CloneBackend;

pub struct Unsupported;

impl BlockCloner for Unsupported {
    fn backend(&self) -> CloneBackend {
        CloneBackend::None
    }

    fn clone_file(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn clone_directory(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn clones_directories(&self) -> bool {
        false
    }
}
