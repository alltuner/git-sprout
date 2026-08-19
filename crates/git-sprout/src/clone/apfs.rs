// ABOUTME: Block cloning on macOS. Files go through reflink-copy; directories go
// ABOUTME: through clonefile(2) directly, which copies a whole hierarchy in one call.

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::BlockCloner;
use crate::stats::CloneBackend;

/// Do not resolve a symlink at the source path; clone the link itself.
const CLONE_NOFOLLOW: u32 = 0x0001;

extern "C" {
    fn clonefile(
        source: *const libc::c_char,
        destination: *const libc::c_char,
        flags: u32,
    ) -> libc::c_int;
}

pub struct Clonefile;

impl BlockCloner for Clonefile {
    fn backend(&self) -> CloneBackend {
        CloneBackend::Apfs
    }

    fn clone_file(&self, source: &Path, destination: &Path) -> io::Result<()> {
        reflink_copy::reflink(source, destination)
    }

    fn clone_directory(&self, source: &Path, destination: &Path) -> io::Result<()> {
        let source = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let destination = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: both pointers come from CStrings that outlive the call.
        let result = unsafe { clonefile(source.as_ptr(), destination.as_ptr(), CLONE_NOFOLLOW) };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn clones_directories(&self) -> bool {
        true
    }
}
