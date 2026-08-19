// ABOUTME: File-level block cloning through reflink-copy: FICLONE on Linux and
// ABOUTME: FSCTL_DUPLICATE_EXTENTS_TO_FILE on Windows ReFS. Neither clones directories.

use std::io;
use std::path::Path;

use super::BlockCloner;
use crate::stats::CloneBackend;

pub struct Reflink {
    backend: CloneBackend,
}

impl Reflink {
    pub fn new(backend: CloneBackend) -> Self {
        Reflink { backend }
    }
}

impl BlockCloner for Reflink {
    fn backend(&self) -> CloneBackend {
        self.backend
    }

    fn clone_file(&self, source: &Path, destination: &Path) -> io::Result<()> {
        reflink_copy::reflink(source, destination)
    }

    fn clone_directory(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn clones_directories(&self) -> bool {
        false
    }
}
