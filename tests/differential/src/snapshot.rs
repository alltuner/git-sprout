// ABOUTME: Captures everything observable about one side of a comparison into a
// ABOUTME: value, so two sides can be compared without both trees existing at once.

use crate::env::Workspace;
use crate::files;
use crate::index::{self, IndexFacts};
use crate::normalize::{self, Normaliser};
use crate::run::{self, RunOutput, Side};
use crate::stats::Stats;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeEntry {
    Dir {
        mode: u32,
    },
    Symlink {
        target: String,
    },
    File {
        mode: u32,
        size: u64,
        digest: String,
    },
    /// A `.git` pointer file: compared as normalised text rather than as bytes,
    /// because it names the absolute path of the side it belongs to.
    GitPointer {
        content: String,
    },
}

impl TreeEntry {
    pub fn describe(&self) -> String {
        match self {
            TreeEntry::Dir { mode } => format!("directory mode {mode:06o}"),
            TreeEntry::Symlink { target } => format!("symlink -> {target}"),
            TreeEntry::File { mode, size, digest } => {
                format!("file mode {mode:06o} size {size} sha256 {}", &digest[..16])
            }
            TreeEntry::GitPointer { content } => {
                format!(".git pointer {:?}", content.trim_end())
            }
        }
    }
}

/// A file inside `.git/worktrees/<name>/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminEntry {
    Dir,
    Text(String),
    Binary { size: u64, digest: String },
}

impl AdminEntry {
    pub fn describe(&self) -> String {
        match self {
            AdminEntry::Dir => "directory".to_string(),
            AdminEntry::Text(t) => format!("{:?}", t),
            AdminEntry::Binary { size, digest } => {
                format!("{size} bytes, sha256 {}", &digest[..16])
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Progress frames that do not match git's shape; always empty in a correct run.
    pub malformed_progress: Vec<String>,
    pub label: &'static str,
    pub stdout: String,
    pub stderr: String,
    pub status: i32,
    pub timed_out: bool,
    pub hooks: Vec<String>,
    pub stats: Option<Stats>,
    pub worktree_exists: bool,
    pub tree: BTreeMap<String, TreeEntry>,
    pub porcelain: Vec<String>,
    pub head: String,
    pub head_reflog_entry: String,
    pub reflog: Vec<String>,
    pub refs: Vec<String>,
    pub worktree_list: Vec<String>,
    pub admin: BTreeMap<String, AdminEntry>,
    pub index: IndexFacts,
    /// Present only for split-index repositories. Compared by parsing, because the
    /// file is named after a hash of its own content and that content includes stat
    /// data, so two correct runs produce two different filenames.
    pub shared_index: Option<IndexFacts>,
    pub shared_index_count: usize,
    pub ls_files_stage: Vec<String>,
    pub ls_files_flags: Vec<String>,
}

/// Reads every observable of one side after its add has run.
pub fn capture(
    workspace: &Workspace,
    side: &Side,
    output: RunOutput,
    object_format: &str,
) -> std::io::Result<Snapshot> {
    let norm = Normaliser::new(&side.root, workspace.root());
    let worktree_exists = side.worktree.is_dir();

    let mut snapshot = Snapshot {
        label: side.label,
        stdout: norm.text(&output.stdout),
        stderr: norm.stderr(&output.stderr),
        malformed_progress: norm.malformed_progress(&output.stderr),
        status: output.status,
        timed_out: output.timed_out,
        hooks: output
            .hooks
            .iter()
            .map(|h| norm.text(h.as_bytes()))
            .collect(),
        stats: output.stats,
        worktree_exists,
        tree: BTreeMap::new(),
        porcelain: Vec::new(),
        head: String::new(),
        head_reflog_entry: String::new(),
        reflog: Vec::new(),
        refs: Vec::new(),
        worktree_list: Vec::new(),
        admin: BTreeMap::new(),
        index: IndexFacts::missing("no worktree was created"),
        shared_index: None,
        shared_index_count: 0,
        ls_files_stage: Vec::new(),
        ls_files_flags: Vec::new(),
    };

    if side.repo.is_dir() {
        snapshot.refs = lines(&run::git(
            workspace,
            &side.repo,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname) %(objecttype)",
            ],
        )?);
        snapshot.worktree_list = lines(&run::git(
            workspace,
            &side.repo,
            &["worktree", "list", "--porcelain"],
        )?)
        .into_iter()
        .map(|l| norm.text(l.as_bytes()))
        .collect();
    }

    if !worktree_exists {
        return Ok(snapshot);
    }

    // Read what the operation left behind before asking git anything: `git status`
    // and `git ls-files` refresh the index and write it back, which would make the
    // harness observe its own side effect instead of the result under test.
    let name = side
        .worktree
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let admin_dir = side.repo.join(".git").join("worktrees").join(&name);
    if admin_dir.is_dir() {
        snapshot.admin = capture_admin(&admin_dir, &norm)?;
        snapshot.index = index::read(&admin_dir.join("index"), object_format);
        let mut shared: Vec<std::path::PathBuf> = std::fs::read_dir(&admin_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("sharedindex."))
            })
            .collect();
        shared.sort();
        snapshot.shared_index_count = shared.len();
        snapshot.shared_index = shared.first().map(|p| index::read(p, object_format));
    }

    snapshot.tree = capture_tree(&side.worktree, &norm)?;
    snapshot.porcelain = {
        let raw = run::git(
            workspace,
            &side.worktree,
            &["status", "--porcelain", "-z", "--untracked-files=all"],
        )?;
        let mut entries: Vec<String> = raw
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        entries.sort();
        entries
    };
    // One call for both: git resolves each argument in turn, so the two lines come
    // back in order and the process spawn is paid once instead of twice.
    let resolved = lines(&run::git(
        workspace,
        &side.worktree,
        &["rev-parse", "HEAD", "HEAD@{0}"],
    )?);
    snapshot.head = resolved.first().cloned().unwrap_or_default();
    snapshot.head_reflog_entry = resolved.get(1).cloned().unwrap_or_default();
    snapshot.reflog = lines(&run::git(
        workspace,
        &side.worktree,
        &["reflog", "show", "--format=%H %gd %gs"],
    )?);
    snapshot.ls_files_stage = nul_lines(&run::git(
        workspace,
        &side.worktree,
        &["ls-files", "--stage", "-z"],
    )?);
    snapshot.ls_files_flags = nul_lines(&run::git(
        workspace,
        &side.worktree,
        &["ls-files", "-v", "-z"],
    )?);

    // A parse failure names the file it failed on, which is a per-side path.
    snapshot.index.parse_error = snapshot
        .index
        .parse_error
        .as_ref()
        .map(|e| norm.text(e.as_bytes()));
    if let Some(shared) = snapshot.shared_index.as_mut() {
        shared.parse_error = shared.parse_error.as_ref().map(|e| norm.text(e.as_bytes()));
    }
    Ok(snapshot)
}

fn capture_tree(root: &Path, norm: &Normaliser) -> std::io::Result<BTreeMap<String, TreeEntry>> {
    let mut out = BTreeMap::new();
    for relative in files::walk(root)? {
        let path = root.join(&relative);
        let meta = std::fs::symlink_metadata(&path)?;
        let key = relative.to_string_lossy().replace('\\', "/");
        let entry = if meta.file_type().is_symlink() {
            TreeEntry::Symlink {
                target: std::fs::read_link(&path)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            }
        } else if meta.is_dir() {
            TreeEntry::Dir {
                mode: mode_of(&meta),
            }
        } else if relative.file_name().is_some_and(|n| n == ".git") {
            TreeEntry::GitPointer {
                content: norm.text(&std::fs::read(&path)?),
            }
        } else {
            TreeEntry::File {
                mode: mode_of(&meta),
                size: meta.len(),
                digest: digest_file(&path)?,
            }
        };
        out.insert(key, entry);
    }
    Ok(out)
}

fn capture_admin(dir: &Path, norm: &Normaliser) -> std::io::Result<BTreeMap<String, AdminEntry>> {
    let mut out = BTreeMap::new();
    for relative in files::walk(dir)? {
        let key = relative.to_string_lossy().replace('\\', "/");
        // The index is compared by parsing it, not by its bytes: it embeds stat data
        // that legitimately differs between two checkouts of the same tree.
        if key == "index" || key.starts_with("sharedindex.") {
            continue;
        }
        let path = dir.join(&relative);
        let meta = std::fs::symlink_metadata(&path)?;
        let entry = if meta.is_dir() {
            AdminEntry::Dir
        } else {
            let bytes = std::fs::read(&path)?;
            match String::from_utf8(norm.bytes(&bytes)) {
                Ok(text) => {
                    let text = if key.starts_with("logs/") {
                        text.lines()
                            .map(|l| format!("{}\n", normalize::reflog_line(l)))
                            .collect()
                    } else {
                        text
                    };
                    AdminEntry::Text(text)
                }
                Err(_) => AdminEntry::Binary {
                    size: bytes.len() as u64,
                    digest: index::hex(&Sha256::digest(&bytes)),
                },
            }
        };
        out.insert(key, entry);
    }
    Ok(out)
}

fn digest_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(index::hex(&hasher.finalize()))
}

fn mode_of(meta: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        if meta.permissions().readonly() {
            0o444
        } else {
            0o644
        }
    }
}

fn lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::to_string)
        .collect()
}

fn nul_lines(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// The object format of a repository, needed to parse its index.
pub fn object_format(workspace: &Workspace, repo: &Path) -> String {
    let value = run::git_line(workspace, repo, &["rev-parse", "--show-object-format"]);
    if value == "sha256" {
        value
    } else {
        "sha1".to_string()
    }
}
