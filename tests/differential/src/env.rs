// ABOUTME: Builds the deterministic environment every git and git-sprout invocation
// ABOUTME: runs under, and allocates the scratch directories a comparison needs.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Fixed identity and clock for every git invocation. Reflog and commit timestamps
/// are derived from these, so both sides of a comparison produce identical bytes
/// instead of bytes that merely differ by when they ran.
const FIXED_DATE: &str = "1700000000 +0000";
const FIXED_NAME: &str = "Differential Harness";
const FIXED_EMAIL: &str = "harness@example.invalid";

/// Environment variables that would leak the developer's machine into a run.
const STRIPPED: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_NAMESPACE",
    "GIT_CEILING_DIRECTORIES",
    "GIT_ATTR_NOSYSTEM",
    "GIT_EDITOR",
    "GIT_PAGER",
    "GIT_TRACE",
    "GIT_TRACE2",
    "GIT_TRACE2_EVENT",
    "GIT_SEQUENCE_EDITOR",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "GIT_LFS_SKIP_SMUDGE",
];

/// The scratch tree a whole test process works in.
pub struct Workspace {
    root: PathBuf,
    home: PathBuf,
    counter: AtomicU64,
    keep: bool,
    /// Set when a case directory has been kept for inspection. The whole scratch
    /// tree then survives the run, or the path named in a failure message would be
    /// gone by the time anyone read it.
    retained: AtomicBool,
}

/// The scratch root in the spelling git will echo back, resolved once so that every
/// path derived from it already matches.
///
/// The two sides of a comparison live at `<root>/a` and `<root>/b`, and the harness
/// rewrites those prefixes to a token before comparing so the `a` and the `b` never
/// register as a difference. That only works if the prefix it holds is the one git
/// prints. On Windows it is not: `std::env::temp_dir()` returns the 8.3 short form,
/// `C:\Users\RUNNER~1\AppData\Local\Temp`, while git prints the long form,
/// `C:/Users/runneradmin/...`. The replacement then misses and every per-side path
/// leaks into the comparison, which is why a Windows run diverges on every case
/// rather than a few.
///
/// Canonicalising here rather than enumerating spellings in the normaliser keeps the
/// knowledge in one place. The verbatim prefix Windows canonicalisation returns is
/// stripped because git never prints it.
fn as_git_spells_it(base: &Path) -> PathBuf {
    let Ok(resolved) = std::fs::canonicalize(base) else {
        return base.to_path_buf();
    };
    match resolved.to_string_lossy().strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => resolved,
    }
}

impl Workspace {
    /// Creates the process-wide scratch tree. `SPROUT_TEST_TMPDIR` moves it onto a
    /// chosen filesystem, which is how the no-cloning fallback gets exercised.
    pub fn create(label: &str) -> std::io::Result<Workspace> {
        let base = match std::env::var_os("SPROUT_TEST_TMPDIR") {
            Some(dir) => PathBuf::from(dir),
            None => std::env::temp_dir(),
        };
        std::fs::create_dir_all(&base)?;
        let base = as_git_spells_it(&base);
        let root = base.join(format!("git-sprout-diff-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)?;

        let home = root.join("home");
        std::fs::create_dir_all(&home)?;
        std::fs::write(
            home.join("gitconfig"),
            "[init]\n\tdefaultBranch = main\n[protocol \"file\"]\n\tallow = always\n\
             [advice]\n\tdetachedHead = false\n\
             [gc]\n\tauto = 0\n[maintenance]\n\tauto = 0\n\
             [core]\n\tfsync = none\n",
        )?;

        Ok(Workspace {
            root,
            home,
            counter: AtomicU64::new(0),
            keep: std::env::var_os("SPROUT_TEST_KEEP").is_some(),
            retained: AtomicBool::new(false),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Allocates a directory for one comparison. The name is part of failure output,
    /// so it carries the case identity rather than an opaque number.
    pub fn case_dir(&self, name: &str) -> std::io::Result<PathBuf> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let dir = self.root.join(format!("{n:04}-{}", sanitise(name)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Removes a case directory once its comparison passed. Everything survives when
    /// `SPROUT_TEST_KEEP` is set.
    pub fn release_case(&self, dir: &Path) {
        if !self.keep {
            let _ = remove_tree(dir);
        }
    }

    /// Keeps the whole scratch tree past the end of the run, because something in it
    /// is named in a failure message.
    pub fn retain(&self) {
        self.retained.store(true, Ordering::SeqCst);
    }

    /// The environment every git and tool invocation runs under.
    pub fn env(&self) -> BTreeMap<OsString, Option<OsString>> {
        let mut env: BTreeMap<OsString, Option<OsString>> = BTreeMap::new();
        for key in STRIPPED {
            env.insert(OsString::from(*key), None);
        }
        let mut set = |k: &str, v: &str| {
            env.insert(OsString::from(k), Some(OsString::from(v)));
        };
        set("HOME", &self.home.to_string_lossy());
        set("XDG_CONFIG_HOME", &self.home.join("xdg").to_string_lossy());
        set(
            "GIT_CONFIG_GLOBAL",
            &self.home.join("gitconfig").to_string_lossy(),
        );
        set(
            "GIT_CONFIG_SYSTEM",
            &self.home.join("nonexistent-system").to_string_lossy(),
        );
        set("GIT_CONFIG_NOSYSTEM", "1");
        set("GIT_AUTHOR_NAME", FIXED_NAME);
        set("GIT_AUTHOR_EMAIL", FIXED_EMAIL);
        set("GIT_AUTHOR_DATE", FIXED_DATE);
        set("GIT_COMMITTER_NAME", FIXED_NAME);
        set("GIT_COMMITTER_EMAIL", FIXED_EMAIL);
        set("GIT_COMMITTER_DATE", FIXED_DATE);
        set("GIT_TERMINAL_PROMPT", "0");
        set("GIT_ADVICE", "0");
        set("LC_ALL", "C");
        set("LANG", "C");
        set("TZ", "UTC");
        env
    }

    /// Applies [`Workspace::env`] to a command.
    pub fn apply(&self, cmd: &mut Command) {
        for (key, value) in self.env() {
            match value {
                Some(v) => {
                    cmd.env(&key, v);
                }
                None => {
                    cmd.env_remove(&key);
                }
            }
        }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if !self.keep && !self.retained.load(Ordering::SeqCst) {
            let _ = remove_tree(&self.root);
        }
    }
}

/// Removes a tree that may contain read-only files and directories git wrote.
pub fn remove_tree(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => {
            chmod_writable(path);
            std::fs::remove_dir_all(path)
        }
    }
}

fn chmod_writable(path: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    let mut perms = meta.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(perms.mode() | 0o700);
    }
    #[cfg(not(unix))]
    perms.set_readonly(false);
    let _ = std::fs::set_permissions(path, perms);
    if meta.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                chmod_writable(&entry.path());
            }
        }
    }
}

fn sanitise(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod root_spelling_tests {
    use super::as_git_spells_it;
    use std::path::PathBuf;

    /// The root has to come back in the spelling git prints, or the per-side prefix
    /// replacement misses and every comparison diverges. On macOS that means
    /// `/var/folders/...` resolving to `/private/var/folders/...`; on Windows it means
    /// the 8.3 short form resolving to the long one.
    #[test]
    fn resolves_to_a_path_that_exists_and_is_already_canonical() {
        let base = std::env::temp_dir();
        let resolved = as_git_spells_it(&base);
        assert!(resolved.is_dir(), "the resolved root should still exist");
        assert_eq!(
            resolved,
            as_git_spells_it(&resolved),
            "resolving twice should change nothing"
        );
        assert!(
            !resolved.to_string_lossy().starts_with(r"\\?\"),
            "git never prints the verbatim prefix, so the root must not carry it"
        );
    }

    /// A path that cannot be resolved is handed back untouched rather than lost.
    #[test]
    fn an_unresolvable_path_is_returned_unchanged() {
        let missing = PathBuf::from("this-directory-does-not-exist-anywhere");
        assert_eq!(as_git_spells_it(&missing), missing);
    }
}
