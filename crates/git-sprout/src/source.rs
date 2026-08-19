// ABOUTME: Picks which existing checkout a new worktree is grown from.
// ABOUTME: Candidates come from `git worktree list`; the winner is the closest commit.

use std::path::{Path, PathBuf};

use crate::git::Git;

/// How many candidates are worth scoring. Beyond this the scoring costs more than it saves.
const CANDIDATE_LIMIT: usize = 5;

/// One entry of `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub bare: bool,
    pub prunable: bool,
}

/// Parses `git worktree list --porcelain`.
pub fn parse_list(output: &[u8]) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    let mut current: Option<Worktree> = None;
    for line in output.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            if let Some(worktree) = current.take() {
                worktrees.push(worktree);
            }
            continue;
        }
        if let Some(path) = line.strip_prefix(b"worktree ") {
            if let Some(worktree) = current.take() {
                worktrees.push(worktree);
            }
            current = Some(Worktree {
                path: bytes_to_path(path),
                head: None,
                bare: false,
                prunable: false,
            });
            continue;
        }
        let Some(worktree) = current.as_mut() else {
            continue;
        };
        if let Some(head) = line.strip_prefix(b"HEAD ") {
            worktree.head = String::from_utf8(head.to_vec()).ok();
        } else if line == b"bare" {
            worktree.bare = true;
        } else if line == b"prunable" || line.starts_with(b"prunable ") {
            worktree.prunable = true;
        }
    }
    if let Some(worktree) = current.take() {
        worktrees.push(worktree);
    }
    worktrees
}

#[cfg(unix)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

/// The device a path lives on, where the platform reports one.
#[cfg(unix)]
fn device_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|meta| meta.dev())
}

#[cfg(not(unix))]
fn device_of(_path: &Path) -> Option<u64> {
    None
}

/// Orders the candidates the way ties should be broken: the checkout the command was run
/// from, then the main worktree, then the rest, most recently modified first.
fn in_preference_order(worktrees: &[Worktree], current: Option<&Path>) -> Vec<Worktree> {
    let mut ordered: Vec<Worktree> = Vec::new();
    let take = |worktree: &Worktree, ordered: &mut Vec<Worktree>| {
        if !ordered.iter().any(|taken| taken.path == worktree.path) {
            ordered.push(worktree.clone());
        }
    };
    if let Some(current) = current {
        if let Some(worktree) = worktrees.iter().find(|worktree| worktree.path == current) {
            take(worktree, &mut ordered);
        }
    }
    if let Some(main) = worktrees.first() {
        take(main, &mut ordered);
    }
    let mut rest: Vec<Worktree> = worktrees
        .iter()
        .filter(|worktree| !ordered.iter().any(|taken| taken.path == worktree.path))
        .cloned()
        .collect();
    rest.sort_by_key(|worktree| {
        std::fs::metadata(&worktree.path)
            .and_then(|meta| meta.modified())
            .ok()
    });
    rest.reverse();
    ordered.extend(rest);
    ordered
}

/// Chooses the checkout to clone from, or `None` when none can serve.
///
/// Only worktrees of the same repository on the same device qualify, because a block
/// clone cannot cross volumes. Among those, the one whose HEAD differs from the target
/// commit in the fewest paths wins, since every differing path is one git has to write.
pub fn choose(
    git: &Git,
    worktrees: &[Worktree],
    destination: &Path,
    target_commit: &str,
) -> Option<PathBuf> {
    let destination_device = destination.parent().and_then(device_of);
    let current = std::env::current_dir().ok().and_then(|cwd| {
        git.capture_line(
            Some(&cwd),
            ["rev-parse", "--path-format=absolute", "--show-toplevel"],
        )
        .ok()
        .map(PathBuf::from)
    });

    let candidates: Vec<Worktree> = in_preference_order(worktrees, current.as_deref())
        .into_iter()
        .filter(|worktree| !worktree.bare && !worktree.prunable)
        .filter(|worktree| worktree.path != destination)
        .filter(|worktree| worktree.path.is_dir())
        .filter(
            |worktree| match (destination_device, device_of(&worktree.path)) {
                (Some(destination), Some(candidate)) => destination == candidate,
                _ => true,
            },
        )
        .take(CANDIDATE_LIMIT)
        .collect();

    candidates
        .iter()
        .enumerate()
        .min_by_key(|(position, worktree)| {
            (differing_paths(git, worktree, target_commit), *position)
        })
        .map(|(_, worktree)| worktree.path.clone())
}

/// How many paths the candidate's HEAD differs from the target commit in.
fn differing_paths(git: &Git, worktree: &Worktree, target_commit: &str) -> usize {
    let Some(head) = worktree.head.as_deref() else {
        return usize::MAX;
    };
    match git.capture(
        Some(&worktree.path),
        ["diff-tree", "-r", "-z", "--name-only", head, target_commit],
    ) {
        Ok(output) => output.iter().filter(|byte| **byte == 0).count(),
        Err(_) => usize::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_porcelain_listing() {
        let output = b"worktree /repo\nHEAD abc\nbranch refs/heads/main\n\n\
                       worktree /repo/wt\nHEAD def\ndetached\n\n\
                       worktree /repo/gone\nHEAD 123\nprunable gitdir file points to non-existent location\n\n\
                       worktree /repo/bare\nbare\n\n"
            .as_slice();
        let worktrees = parse_list(output);
        assert_eq!(worktrees.len(), 4);
        assert_eq!(worktrees[0].path, PathBuf::from("/repo"));
        assert_eq!(worktrees[0].head.as_deref(), Some("abc"));
        assert!(!worktrees[0].bare);
        assert!(worktrees[2].prunable);
        assert!(worktrees[3].bare);
    }

    #[test]
    fn prefers_the_current_checkout_then_the_main_one() {
        let worktrees = vec![
            Worktree {
                path: PathBuf::from("/repo"),
                head: None,
                bare: false,
                prunable: false,
            },
            Worktree {
                path: PathBuf::from("/repo/a"),
                head: None,
                bare: false,
                prunable: false,
            },
            Worktree {
                path: PathBuf::from("/repo/b"),
                head: None,
                bare: false,
                prunable: false,
            },
        ];
        let ordered = in_preference_order(&worktrees, Some(Path::new("/repo/b")));
        assert_eq!(ordered[0].path, PathBuf::from("/repo/b"));
        assert_eq!(ordered[1].path, PathBuf::from("/repo"));
        assert_eq!(ordered[2].path, PathBuf::from("/repo/a"));
    }
}
