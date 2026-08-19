// ABOUTME: Filesystem helpers the harness needs: recursive copies that preserve
// ABOUTME: modes and symlinks, and a deterministic recursive walk.

use std::path::{Path, PathBuf};

/// Copies a directory tree, preserving symlinks, unix mode bits and modification
/// times. Used to give both sides of a comparison an independent, identical
/// repository. Times matter: git's index records them, and a copy that resets them
/// leaves every entry looking stat-dirty, which would silently disable exactly the
/// acceleration the suite is meant to observe.
pub fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(from)?;
        symlink(&target, to)?;
        copy_times(&meta, to, true);
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        let mut entries: Vec<PathBuf> = std::fs::read_dir(from)?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<_, _>>()?;
        entries.sort();
        for entry in entries {
            let name = entry.file_name().expect("directory entry has a name");
            copy_tree(&entry, &to.join(name))?;
        }
        copy_permissions(&meta, to)?;
        copy_times(&meta, to, false);
        return Ok(());
    }
    std::fs::copy(from, to)?;
    copy_permissions(&meta, to)?;
    copy_times(&meta, to, false);
    Ok(())
}

fn copy_times(meta: &std::fs::Metadata, to: &Path, is_symlink: bool) {
    let accessed = filetime::FileTime::from_last_access_time(meta);
    let modified = filetime::FileTime::from_last_modification_time(meta);
    let _ = if is_symlink {
        filetime::set_symlink_file_times(to, accessed, modified)
    } else {
        filetime::set_file_times(to, accessed, modified)
    };
}

fn copy_permissions(meta: &std::fs::Metadata, to: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            to,
            std::fs::Permissions::from_mode(meta.permissions().mode()),
        )?;
    }
    #[cfg(not(unix))]
    {
        let _ = (meta, to);
    }
    Ok(())
}

#[cfg(unix)]
pub fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
pub fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

/// Every path under `root`, relative to it, depth first and sorted. Directories are
/// included so that a directory appearing on one side only is itself a difference.
pub fn walk(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk_into(root, Path::new(""), &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_into(root: &Path, relative: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let dir = root.join(relative);
    let mut entries: Vec<_> = std::fs::read_dir(&dir)?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<Result<_, _>>()?;
    entries.sort();
    for name in entries {
        let child = relative.join(&name);
        let meta = std::fs::symlink_metadata(root.join(&child))?;
        let is_dir = meta.is_dir() && !meta.file_type().is_symlink();
        out.push(child.clone());
        if is_dir {
            walk_into(root, &child, out)?;
        }
    }
    Ok(())
}
