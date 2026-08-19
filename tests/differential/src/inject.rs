// ABOUTME: Deliberate corruptions of a finished worktree, each one a difference the
// ABOUTME: comparison must report. A harness never shown to fail is worth nothing.

use crate::env::Workspace;
use crate::run::{self, RunOutput, Side};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Whether an injection could be applied to this particular worktree.
#[derive(Debug)]
pub enum Applied {
    Yes,
    /// The fixture does not contain what this injection needs; the caller reports a
    /// skip rather than a pass, so an injection can never silently stop being tested.
    NotApplicable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Injection {
    FlipFileByte,
    SetExecutableBit,
    SymlinkBecomesRegularFile,
    RemoveCheckedOutFile,
    AddUntrackedFile,
    RewriteGitPointer,
    ChangeIndexVersion,
    ChangeIndexEntryOid,
    ChangeIndexEntryMode,
    ChangeIndexEntryCount,
    RemoveIndexExtensions,
    SuppressHeadLine,
    SuppressPreparingLine,
    ChangeExitStatus,
    OmitPostCheckoutHook,
    ReorderHooks,
    ChangeReflogMessage,
    DetachHead,
    AddStrayRef,
}

/// Every injection, in the order the report lists them.
pub const ALL: &[Injection] = &[
    Injection::FlipFileByte,
    Injection::SetExecutableBit,
    Injection::SymlinkBecomesRegularFile,
    Injection::RemoveCheckedOutFile,
    Injection::AddUntrackedFile,
    Injection::RewriteGitPointer,
    Injection::ChangeIndexVersion,
    Injection::ChangeIndexEntryOid,
    Injection::ChangeIndexEntryMode,
    Injection::ChangeIndexEntryCount,
    Injection::RemoveIndexExtensions,
    Injection::SuppressHeadLine,
    Injection::SuppressPreparingLine,
    Injection::ChangeExitStatus,
    Injection::OmitPostCheckoutHook,
    Injection::ReorderHooks,
    Injection::ChangeReflogMessage,
    Injection::DetachHead,
    Injection::AddStrayRef,
];

impl Injection {
    pub fn name(self) -> &'static str {
        match self {
            Injection::FlipFileByte => "flip-file-byte",
            Injection::SetExecutableBit => "set-executable-bit",
            Injection::SymlinkBecomesRegularFile => "symlink-becomes-regular-file",
            Injection::RemoveCheckedOutFile => "remove-checked-out-file",
            Injection::AddUntrackedFile => "add-untracked-file",
            Injection::RewriteGitPointer => "rewrite-git-pointer",
            Injection::ChangeIndexVersion => "change-index-version",
            Injection::ChangeIndexEntryOid => "change-index-entry-oid",
            Injection::ChangeIndexEntryMode => "change-index-entry-mode",
            Injection::ChangeIndexEntryCount => "change-index-entry-count",
            Injection::RemoveIndexExtensions => "remove-index-extensions",
            Injection::SuppressHeadLine => "suppress-head-line",
            Injection::SuppressPreparingLine => "suppress-preparing-line",
            Injection::ChangeExitStatus => "change-exit-status",
            Injection::OmitPostCheckoutHook => "omit-post-checkout-hook",
            Injection::ReorderHooks => "reorder-hooks",
            Injection::ChangeReflogMessage => "change-reflog-message",
            Injection::DetachHead => "detach-head",
            Injection::AddStrayRef => "add-stray-ref",
        }
    }

    /// What the injected worktree differs by, in the words the report uses.
    pub fn describe(self) -> &'static str {
        match self {
            Injection::FlipFileByte => "one byte of a checked-out file is flipped",
            Injection::SetExecutableBit => "a regular file gains the executable bit",
            Injection::SymlinkBecomesRegularFile => {
                "a symlink is replaced by a regular file holding its target"
            }
            Injection::RemoveCheckedOutFile => "one tracked file is left un-checked-out",
            Injection::AddUntrackedFile => "an extra untracked file is left behind",
            Injection::RewriteGitPointer => "the worktree's .git pointer names another admin dir",
            Injection::ChangeIndexVersion => "the index is written as version 3 instead of 2",
            Injection::ChangeIndexEntryOid => "an index entry records the wrong blob oid",
            Injection::ChangeIndexEntryMode => {
                "an index entry records mode 100755 instead of 100644"
            }
            Injection::ChangeIndexEntryCount => "the index header declares one entry fewer",
            Injection::RemoveIndexExtensions => "the index is written without its extensions",
            Injection::SuppressHeadLine => "the `HEAD is now at` stdout line is missing",
            Injection::SuppressPreparingLine => "the `Preparing worktree` stderr line is missing",
            Injection::ChangeExitStatus => "the exit status is 1 instead of 0",
            Injection::OmitPostCheckoutHook => "the post-checkout hook never fired",
            Injection::ReorderHooks => "two hooks fired in the wrong order",
            Injection::ChangeReflogMessage => "the reflog records a different message",
            Injection::DetachHead => "HEAD is detached instead of on the new branch",
            Injection::AddStrayRef => "an extra ref is left in the repository",
        }
    }

    /// The comparator that has to notice. Asserted, so a comparator cannot quietly
    /// stop covering the difference it exists for.
    pub fn expected_area(self) -> &'static str {
        match self {
            Injection::FlipFileByte
            | Injection::SetExecutableBit
            | Injection::SymlinkBecomesRegularFile
            | Injection::RemoveCheckedOutFile
            | Injection::AddUntrackedFile
            | Injection::RewriteGitPointer => "working tree",
            Injection::ChangeIndexVersion
            | Injection::ChangeIndexEntryOid
            | Injection::ChangeIndexEntryMode
            | Injection::ChangeIndexEntryCount
            | Injection::RemoveIndexExtensions => "index",
            Injection::SuppressHeadLine => "stdout",
            Injection::SuppressPreparingLine => "stderr",
            Injection::ChangeExitStatus => "exit status",
            Injection::OmitPostCheckoutHook | Injection::ReorderHooks => "hooks fired",
            Injection::ChangeReflogMessage => "worktree admin dir",
            Injection::DetachHead => "worktree list",
            Injection::AddStrayRef => "refs",
        }
    }

    pub fn apply(
        self,
        workspace: &Workspace,
        side: &Side,
        output: &mut RunOutput,
    ) -> std::io::Result<Applied> {
        match self {
            Injection::FlipFileByte => flip_file_byte(workspace, side),
            Injection::SetExecutableBit => set_executable_bit(workspace, side),
            Injection::SymlinkBecomesRegularFile => symlink_to_regular(side),
            Injection::RemoveCheckedOutFile => remove_checked_out_file(workspace, side),
            Injection::AddUntrackedFile => {
                std::fs::write(side.worktree.join("stray-untracked.txt"), b"left behind\n")?;
                Ok(Applied::Yes)
            }
            Injection::RewriteGitPointer => {
                let pointer = side.worktree.join(".git");
                if !pointer.is_file() {
                    return Ok(Applied::NotApplicable("no .git pointer file".into()));
                }
                let text = std::fs::read_to_string(&pointer)?;
                std::fs::write(
                    &pointer,
                    text.replace("worktrees/", "worktrees/../worktrees/"),
                )?;
                Ok(Applied::Yes)
            }
            Injection::ChangeIndexVersion => patch_index(workspace, side, |data| {
                data[4..8].copy_from_slice(&3u32.to_be_bytes());
                true
            }),
            Injection::ChangeIndexEntryOid => patch_index(workspace, side, |data| {
                if data.len() < 12 + 60 {
                    return false;
                }
                data[52] ^= 0xff;
                true
            }),
            Injection::ChangeIndexEntryMode => patch_index(workspace, side, |data| {
                if data.len() < 12 + 40 {
                    return false;
                }
                let mode = u32::from_be_bytes([data[36], data[37], data[38], data[39]]);
                if mode != 0o100644 {
                    return false;
                }
                data[36..40].copy_from_slice(&0o100755u32.to_be_bytes());
                true
            }),
            Injection::ChangeIndexEntryCount => patch_index(workspace, side, |data| {
                let count = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
                if count == 0 {
                    return false;
                }
                data[8..12].copy_from_slice(&(count - 1).to_be_bytes());
                true
            }),
            Injection::RemoveIndexExtensions => remove_index_extensions(workspace, side),
            Injection::SuppressHeadLine => suppress_line(
                &mut output.stdout,
                "HEAD is now at ",
                "no `HEAD is now at` line",
            ),
            Injection::SuppressPreparingLine => suppress_line(
                &mut output.stderr,
                "Preparing worktree ",
                "no `Preparing worktree` line",
            ),
            Injection::ChangeExitStatus => {
                output.status = 1;
                Ok(Applied::Yes)
            }
            Injection::OmitPostCheckoutHook => {
                let before = output.hooks.len();
                output.hooks.retain(|h| !h.starts_with("post-checkout"));
                if output.hooks.len() == before {
                    return Ok(Applied::NotApplicable("post-checkout never fired".into()));
                }
                Ok(Applied::Yes)
            }
            Injection::ReorderHooks => {
                if output.hooks.len() < 2 {
                    return Ok(Applied::NotApplicable("fewer than two hooks fired".into()));
                }
                let last = output.hooks.len() - 1;
                output.hooks.swap(0, last);
                Ok(Applied::Yes)
            }
            Injection::ChangeReflogMessage => change_reflog_message(side),
            Injection::DetachHead => detach_head(workspace, side),
            Injection::AddStrayRef => {
                let head = run::git_line(workspace, &side.repo, &["rev-parse", "HEAD"]);
                if head.len() < 7 || head.starts_with('<') {
                    return Ok(Applied::NotApplicable("repository has no commits".into()));
                }
                let out = run::git_full(
                    workspace,
                    &side.repo,
                    &["update-ref", "refs/heads/stray-injected", &head],
                )?;
                if !out.status.success() {
                    return Ok(Applied::NotApplicable("could not create a ref".into()));
                }
                Ok(Applied::Yes)
            }
        }
    }
}

fn tracked_regular_file(workspace: &Workspace, side: &Side) -> std::io::Result<Option<PathBuf>> {
    let listing = run::git(workspace, &side.worktree, &["ls-files", "-z"])?;
    for name in listing.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let path = side.worktree.join(String::from_utf8_lossy(name).as_ref());
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_file() && !meta.file_type().is_symlink() && meta.len() > 0 {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn flip_file_byte(workspace: &Workspace, side: &Side) -> std::io::Result<Applied> {
    let Some(path) = tracked_regular_file(workspace, side)? else {
        return Ok(Applied::NotApplicable(
            "no non-empty tracked regular file".into(),
        ));
    };
    let mut bytes = std::fs::read(&path)?;
    bytes[0] ^= 0x20;
    std::fs::write(&path, bytes)?;
    Ok(Applied::Yes)
}

fn set_executable_bit(workspace: &Workspace, side: &Side) -> std::io::Result<Applied> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let listing = run::git(workspace, &side.worktree, &["ls-files", "-z"])?;
        for name in listing.split(|b| *b == 0).filter(|s| !s.is_empty()) {
            let path = side.worktree.join(String::from_utf8_lossy(name).as_ref());
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.permissions().mode() & 0o111 == 0
            {
                let mode = meta.permissions().mode() | 0o111;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
                return Ok(Applied::Yes);
            }
        }
        Ok(Applied::NotApplicable(
            "no non-executable tracked file".into(),
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = (workspace, side);
        Ok(Applied::NotApplicable(
            "this platform has no executable bit".into(),
        ))
    }
}

fn symlink_to_regular(side: &Side) -> std::io::Result<Applied> {
    for relative in crate::files::walk(&side.worktree)? {
        let path = side.worktree.join(&relative);
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&path)?;
            std::fs::remove_file(&path)?;
            std::fs::write(&path, format!("{}", target.display()))?;
            return Ok(Applied::Yes);
        }
    }
    Ok(Applied::NotApplicable(
        "the worktree contains no symlink".into(),
    ))
}

fn remove_checked_out_file(workspace: &Workspace, side: &Side) -> std::io::Result<Applied> {
    let Some(path) = tracked_regular_file(workspace, side)? else {
        return Ok(Applied::NotApplicable("no tracked regular file".into()));
    };
    std::fs::remove_file(path)?;
    Ok(Applied::Yes)
}

fn admin_index(side: &Side) -> Option<PathBuf> {
    let name = side.worktree.file_name()?;
    let path = side
        .repo
        .join(".git")
        .join("worktrees")
        .join(name)
        .join("index");
    path.is_file().then_some(path)
}

/// Rewrites the index in place and restores its trailing checksum, so git still
/// accepts the file and the corruption shows up as a difference rather than as a
/// read error that would hide which comparator noticed.
fn patch_index(
    workspace: &Workspace,
    side: &Side,
    edit: impl FnOnce(&mut Vec<u8>) -> bool,
) -> std::io::Result<Applied> {
    let Some(path) = admin_index(side) else {
        return Ok(Applied::NotApplicable("the worktree has no index".into()));
    };
    let mut data = std::fs::read(&path)?;
    let object_format = crate::snapshot::object_format(workspace, &side.repo);
    let hash_len = crate::index::oid_len(&object_format);
    if data.len() < 12 + hash_len {
        return Ok(Applied::NotApplicable("the index is empty".into()));
    }
    let body_len = data.len() - hash_len;
    let mut body = data[..body_len].to_vec();
    if !edit(&mut body) {
        return Ok(Applied::NotApplicable(
            "the index does not hold what this needs".into(),
        ));
    }
    data.clear();
    data.extend_from_slice(&body);
    data.extend_from_slice(&checksum(&body, &object_format));
    std::fs::write(&path, &data)?;
    Ok(Applied::Yes)
}

fn remove_index_extensions(workspace: &Workspace, side: &Side) -> std::io::Result<Applied> {
    let Some(path) = admin_index(side) else {
        return Ok(Applied::NotApplicable("the worktree has no index".into()));
    };
    let object_format = crate::snapshot::object_format(workspace, &side.repo);
    let data = std::fs::read(&path)?;
    let facts = crate::index::read(&path, &object_format);
    if facts.extensions.is_empty() {
        return Ok(Applied::NotApplicable(
            "the index carries no extensions".into(),
        ));
    }
    let hash_len = crate::index::oid_len(&object_format);
    let extension_bytes: usize = facts.extensions.iter().map(|e| e.size as usize + 8).sum();
    let end = data.len() - hash_len - extension_bytes;
    let body = data[..end].to_vec();
    let mut out = body.clone();
    out.extend_from_slice(&checksum(&body, &object_format));
    std::fs::write(&path, &out)?;
    Ok(Applied::Yes)
}

fn checksum(body: &[u8], object_format: &str) -> Vec<u8> {
    if object_format == "sha256" {
        Sha256::digest(body).to_vec()
    } else {
        Sha1::digest(body).to_vec()
    }
}

fn suppress_line(stream: &mut Vec<u8>, prefix: &str, missing: &str) -> std::io::Result<Applied> {
    let text = String::from_utf8_lossy(stream).into_owned();
    if !text.lines().any(|l| l.starts_with(prefix)) {
        return Ok(Applied::NotApplicable(missing.to_string()));
    }
    let kept: String = text
        .lines()
        .filter(|l| !l.starts_with(prefix))
        .map(|l| format!("{l}\n"))
        .collect();
    *stream = kept.into_bytes();
    Ok(Applied::Yes)
}

fn admin_dir(side: &Side) -> Option<PathBuf> {
    let name = side.worktree.file_name()?;
    let dir = side.repo.join(".git").join("worktrees").join(name);
    dir.is_dir().then_some(dir)
}

fn change_reflog_message(side: &Side) -> std::io::Result<Applied> {
    let Some(dir) = admin_dir(side) else {
        return Ok(Applied::NotApplicable("no worktree admin directory".into()));
    };
    let log: PathBuf = dir.join("logs").join("HEAD");
    if !log.is_file() {
        return Ok(Applied::NotApplicable("no HEAD reflog".into()));
    }
    let text = std::fs::read_to_string(&log)?;
    let rewritten: String = text
        .lines()
        .map(|line| match line.split_once('\t') {
            Some((head, _)) => format!("{head}\tinjected reflog message\n"),
            None => format!("{line}\n"),
        })
        .collect();
    std::fs::write(&log, rewritten)?;
    Ok(Applied::Yes)
}

fn detach_head(workspace: &Workspace, side: &Side) -> std::io::Result<Applied> {
    let Some(dir) = admin_dir(side) else {
        return Ok(Applied::NotApplicable("no worktree admin directory".into()));
    };
    let head_file: PathBuf = dir.join("HEAD");
    let current = std::fs::read_to_string(&head_file)?;
    if !current.starts_with("ref: ") {
        return Ok(Applied::NotApplicable("HEAD is already detached".into()));
    }
    let oid = run::git_line(workspace, &side.worktree, &["rev-parse", "HEAD"]);
    if oid.len() < 7 || oid.starts_with('<') {
        return Ok(Applied::NotApplicable("HEAD does not resolve".into()));
    }
    std::fs::write(&head_file, format!("{oid}\n"))?;
    Ok(Applied::Yes)
}

/// A worktree path the injections can report against in messages.
pub fn worktree_of(side: &Side) -> &Path {
    &side.worktree
}
