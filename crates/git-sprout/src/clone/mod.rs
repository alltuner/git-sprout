// ABOUTME: The block-cloning layer: one trait, one implementation per platform primitive.
// ABOUTME: Support is found by attempting a clone and reading the error, never by sniffing.

use std::io;
use std::path::Path;

use crate::stats::CloneBackend;

#[cfg(target_os = "macos")]
mod apfs;
#[cfg(any(target_os = "linux", target_os = "android", windows))]
mod reflink;
#[cfg(not(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "android",
    windows
)))]
mod unsupported;

/// Materialises a path by sharing the source's disk blocks rather than copying them.
pub trait BlockCloner {
    /// Which primitive this implementation calls.
    fn backend(&self) -> CloneBackend;

    /// Clones one regular file. `destination` must not exist.
    fn clone_file(&self, source: &Path, destination: &Path) -> io::Result<()>;

    /// Clones a whole directory hierarchy in one call. `destination` must not exist.
    fn clone_directory(&self, source: &Path, destination: &Path) -> io::Result<()>;

    /// Whether `clone_directory` is cheaper than descending file by file.
    fn clones_directories(&self) -> bool;
}

/// The cloner for the platform this binary was built for.
pub fn for_this_platform() -> Box<dyn BlockCloner> {
    #[cfg(target_os = "macos")]
    {
        Box::new(apfs::Clonefile)
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        Box::new(reflink::Reflink::new(CloneBackend::Ficlone))
    }
    #[cfg(windows)]
    {
        Box::new(reflink::Reflink::new(CloneBackend::Refs))
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "android",
        windows
    )))]
    {
        Box::new(unsupported::Unsupported)
    }
}
