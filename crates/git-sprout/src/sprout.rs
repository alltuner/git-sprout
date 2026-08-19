// ABOUTME: Runs an accelerated `git worktree add`: git creates and finishes the worktree,
// ABOUTME: and in between the tool clones whatever the source checkout can supply.

use std::collections::HashSet;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use filetime::FileTime;
use gix_index::entry::Stat;
use gix_index::fs::Metadata;

use crate::argv::AddCommand;
use crate::attributes::{self, LineEndings};
use crate::clone::{self, BlockCloner};
use crate::delegate;
use crate::git::Git;
use crate::interrupt;
use crate::plan::{self, as_path, Planned};
use crate::scratch_index::{self, Record};
use crate::source;
use crate::stats::Stats;
use crate::tree;
use crate::verify;

/// The tree modes that need naming rather than a magic number.
const EXECUTABLE_MODE: u32 = 0o100755;
const SYMLINK_MODE: u32 = 0o120000;

/// The most paths worth naming on one `git diff-files` command line.
const RACY_PATH_LIMIT: usize = 1000;

/// The configuration that decides how blob bytes become working-tree bytes. Cloning is
/// only sound when both worktrees resolve all of it the same way.
const CONVERSION_KEYS: &[&str] = &["core.autocrlf", "core.eol", "core.symlinks"];

/// Creates the worktree, cloning what it can and letting git finish the job.
pub fn add(command: &AddCommand, stats: &mut Stats) -> ExitCode {
    let git = Git::new(command.globals.clone());

    // Git infers `--orphan` for itself when the repository has no commit to branch from,
    // and then rejects the `--no-checkout` step 2 would add. An unborn HEAD is therefore
    // the same case as an explicit `--orphan` and goes to git untouched.
    if git
        .capture(None, ["rev-parse", "--verify", "--quiet", "HEAD"])
        .is_err()
    {
        stats.fall_back("the repository has no commit on HEAD");
        stats.emit();
        return delegate::exec_git(&command.git_args());
    }

    let before = worktrees(&git);
    let created = match git.passthrough(None, command.worktree_add_args_no_checkout()) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("git-sprout: could not run git: {error}");
            return ExitCode::from(1);
        }
    };
    if !created.success() {
        stats.fall_back("git worktree add failed");
        stats.emit();
        return exit_code(created.code());
    }

    let destination = locate(&git, command, &before);

    // From here on the worktree exists but holds no files, so every remaining step is
    // best effort and step 7 must run whatever happens. A panic in the clone phase would
    // or an interrupt would otherwise leave a half-populated worktree behind.
    interrupt::defer();
    if let Some(destination) = destination.as_deref() {
        let reporting = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            populate(&git, destination, &before, stats)
        }));
        let _ = std::panic::take_hook();
        std::panic::set_hook(reporting);
        if outcome.is_err() {
            stats.fall_back("the clone phase failed");
        }
    } else {
        stats.fall_back("the new worktree could not be located");
    }

    stats.emit();

    let Some(destination) = destination else {
        interrupt::honour();
        return ExitCode::SUCCESS;
    };

    let code = finish(&git, &destination, command.quiet);
    interrupt::honour();
    code
}

/// Steps 7 and 8: git writes whatever is missing and the real index, then the checkout
/// hook fires. `git reset --hard` is what `git worktree add` itself does at this point,
/// down to the reflog entry, the ORIG_HEAD it leaves and the "HEAD is now at" line; it is
/// the one invocation that does not also fire `post-checkout`, which is fired here once.
fn finish(git: &Git, destination: &Path, quiet: bool) -> ExitCode {
    let mut reset: Vec<&str> = vec!["reset"];
    if quiet {
        reset.push("-q");
    }
    reset.push("--hard");
    match git.passthrough(Some(destination), reset) {
        Ok(status) if !status.success() => return exit_code(status.code()),
        Err(error) => {
            eprintln!("git-sprout: could not run git: {error}");
            return ExitCode::from(1);
        }
        Ok(_) => {}
    }

    let Ok(head) = git.capture_line(Some(destination), ["rev-parse", "HEAD"]) else {
        return ExitCode::SUCCESS;
    };
    let null = null_oid(git, destination);
    match git.passthrough(
        Some(destination),
        [
            "hook",
            "run",
            "--ignore-missing",
            "post-checkout",
            "--",
            &null,
            &head,
            "1",
        ],
    ) {
        Ok(status) => exit_code(status.code()),
        Err(_) => ExitCode::SUCCESS,
    }
}

/// The all-zero object id at the repository's hash length.
fn null_oid(git: &Git, destination: &Path) -> String {
    let length = match git
        .capture_line(Some(destination), ["rev-parse", "--show-object-format"])
        .as_deref()
    {
        Ok("sha256") => 64,
        _ => 40,
    };
    "0".repeat(length)
}

fn exit_code(code: Option<i32>) -> ExitCode {
    ExitCode::from(u8::try_from(code.unwrap_or(1)).unwrap_or(1))
}

fn worktrees(git: &Git) -> Vec<source::Worktree> {
    git.capture(None, ["worktree", "list", "--porcelain"])
        .map(|output| source::parse_list(&output))
        .unwrap_or_default()
}

/// Finds the worktree `git worktree add` just created.
///
/// The listing tells us directly which path is new, which needs no guessing about how a
/// relative path resolved. The path the user asked for is the fallback.
fn locate(git: &Git, command: &AddCommand, before: &[source::Worktree]) -> Option<PathBuf> {
    let known: HashSet<&Path> = before
        .iter()
        .map(|worktree| worktree.path.as_path())
        .collect();
    let added = worktrees(git)
        .into_iter()
        .find(|worktree| !known.contains(worktree.path.as_path()))
        .map(|worktree| worktree.path);
    added.or_else(|| {
        let requested = working_directory(&command.globals).join(as_path_os(&command.path));
        requested.join(".git").exists().then_some(requested)
    })
}

/// The directory git resolves relative paths against, after applying every `-C`.
fn working_directory(globals: &[OsString]) -> PathBuf {
    let mut directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut arguments = globals.iter();
    while let Some(argument) = arguments.next() {
        if argument == "-C" {
            if let Some(value) = arguments.next() {
                directory = directory.join(value);
            }
        }
    }
    directory
}

fn as_path_os(path: &OsString) -> PathBuf {
    PathBuf::from(path)
}

/// Steps 3 to 6: choose a source, plan, clone, and write the scratch index.
fn populate(git: &Git, destination: &Path, before: &[source::Worktree], stats: &mut Stats) {
    let cloner = clone::for_this_platform();
    stats.clone_backend = cloner.backend();

    let Ok(head) = git.capture_line(Some(destination), ["rev-parse", "HEAD"]) else {
        stats.fall_back("the new worktree has no HEAD");
        return;
    };
    let object_hash = match git
        .capture_line(Some(destination), ["rev-parse", "--show-object-format"])
        .as_deref()
    {
        Ok("sha1") => gix_hash::Kind::Sha1,
        Ok("sha256") => gix_hash::Kind::Sha256,
        _ => {
            stats.fall_back("unknown object format");
            return;
        }
    };

    let Ok(destination_index) = git.capture_line(
        Some(destination),
        ["rev-parse", "--path-format=absolute", "--git-path", "index"],
    ) else {
        stats.fall_back("could not locate the new worktree's index");
        return;
    };
    let destination_index = PathBuf::from(destination_index);
    let version = scratch_index::default_version(
        std::env::var("GIT_INDEX_VERSION").ok().as_deref(),
        git.config(destination, "index.version").as_deref(),
        git.config(destination, "feature.manyFiles").as_deref() == Some("true"),
    );
    if version != scratch_index::SUPPORTED_VERSION {
        stats.fall_back(format!("the repository writes index version {version}"));
        return;
    }

    let Some(source) = source::choose(git, before, destination, &head) else {
        stats.fall_back("no usable source checkout");
        return;
    };
    stats.source = Some(source.clone());

    if let Some(reason) = conversion_mismatch(git, &source, destination) {
        stats.fall_back(reason);
        return;
    }

    let Some(target) = listing(git, destination, &head) else {
        stats.fall_back("could not read the target tree");
        return;
    };
    let Ok(source_head) = git.capture_line(Some(&source), ["rev-parse", "HEAD"]) else {
        stats.fall_back("the source checkout has no HEAD");
        return;
    };
    let Some(source_tree) = listing(git, &source, &source_head) else {
        stats.fall_back("could not read the source tree");
        return;
    };

    let Ok(index_path) = git.capture_line(
        Some(&source),
        ["rev-parse", "--path-format=absolute", "--git-path", "index"],
    ) else {
        stats.fall_back("could not locate the source index");
        return;
    };
    let Ok(source_index) = gix_index::File::at(
        PathBuf::from(index_path),
        object_hash,
        true,
        gix_index::decode::Options::default(),
    ) else {
        stats.fall_back("could not read the source index");
        return;
    };

    let (source_attributes, suspect_attributes) =
        plan::source_attribute_files(&source_index, &source);
    let dirty_attributes: Vec<Vec<u8>> = changed_paths(git, &source, &suspect_attributes)
        .into_iter()
        .collect();
    let mut poisoned = verify::poisoned_prefixes(
        &target.attribute_files(),
        &source_attributes,
        &dirty_attributes,
    );
    let (colliding, colliding_prefixes) = plan::colliding_paths(&target);
    poisoned.extend(colliding_prefixes);

    let mut verified = plan::verify_paths(&target, &source_index, &source, &poisoned, &colliding);
    let considered = verified.considered;
    let racy_paths: Vec<Vec<u8>> = verified
        .racy
        .iter()
        .map(|planned| planned.path.clone())
        .collect();
    let changed = changed_paths(git, &source, &racy_paths);
    verified.paths.extend(
        verified
            .racy
            .iter()
            .filter(|planned| !changed.contains(&planned.path))
            .cloned(),
    );
    verified.paths.sort_by(|a, b| a.path.cmp(&b.path));
    drop_converted_paths(git, &source, &mut verified.paths);

    let plan = plan::assemble(
        &target,
        &source_tree,
        &source,
        destination,
        verified.paths,
        cloner.clones_directories(),
    );

    let (records, demotion) = materialise(cloner.as_ref(), &source, destination, &plan);
    stats.cloned_directories = if demotion.is_none() {
        plan.directories.len()
    } else {
        0
    };
    if let Some(reason) = demotion {
        stats.fall_back(reason);
    }
    stats.cloned = records.len();
    stats.skipped = considered.saturating_sub(records.len());
    stats.checked_out_by_git = considered.saturating_sub(records.len());

    if records.is_empty() {
        if stats.fallback_reason.is_none() {
            stats.fall_back("nothing in the target tree could be cloned");
        }
        return;
    }

    if scratch_index::write(&destination_index, object_hash, &records).is_err() {
        stats.fall_back("could not write the scratch index");
    }
}

fn listing(git: &Git, worktree: &Path, commit: &str) -> Option<tree::Listing> {
    let output = git
        .capture(Some(worktree), ["ls-tree", "-r", "-t", "-z", commit])
        .ok()?;
    tree::parse(&output)
}

/// Whether the two worktrees would convert blob bytes differently.
fn conversion_mismatch(git: &Git, source: &Path, destination: &Path) -> Option<String> {
    for key in CONVERSION_KEYS {
        if git.config(source, key) != git.config(destination, key) {
            return Some(format!("{key} differs between the worktrees"));
        }
    }
    let source_attributes = worktree_attributes(git, source);
    let destination_attributes = worktree_attributes(git, destination);
    (source_attributes != destination_attributes)
        .then(|| "the worktrees have different attributes files".to_string())
}

/// The contents of a worktree's own `info/attributes`, if it has one.
fn worktree_attributes(git: &Git, worktree: &Path) -> Option<Vec<u8>> {
    let path = git
        .capture_line(
            Some(worktree),
            [
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "info/attributes",
            ],
        )
        .ok()?;
    std::fs::read(path).ok()
}

/// Removes the paths a checkout would rewrite on its way to the working tree.
///
/// Every remaining path is one where the blob's bytes and the working tree's bytes are the
/// same thing, which is what makes the index stat cache a sufficient answer. The
/// attributes are read from the source, which is sound because the plan has already
/// established that both sides carry the same attribute blobs.
fn drop_converted_paths(git: &Git, source: &Path, paths: &mut Vec<Planned>) {
    if paths.is_empty() {
        return;
    }
    let endings = LineEndings::from_config(
        git.config(source, "core.autocrlf").as_deref(),
        git.config(source, "core.eol").as_deref(),
    );
    let mut request = Vec::new();
    for planned in paths.iter() {
        request.extend_from_slice(&planned.path);
        request.push(0);
    }
    let mut arguments: Vec<&str> = vec!["check-attr", "-z", "--stdin"];
    arguments.extend(attributes::CONVERTING_ATTRIBUTES);
    let Ok(output) = git.capture_with_input(Some(source), arguments, &request) else {
        paths.clear();
        return;
    };
    let reported = attributes::parse_check_attr(&output);
    paths.retain(|planned| match reported.get(&planned.path) {
        Some(values) => !attributes::converts(values, endings),
        None => false,
    });
}

/// Asks git which of these paths really differ from what the index records.
///
/// Git re-reads them with the repository's filters applied, which is the only correct way
/// to compare a working-tree file to a blob; hashing them here would compare the wrong
/// bytes. Paths git cannot be asked about are reported as changed, so they get dropped.
fn changed_paths(git: &Git, source: &Path, paths: &[Vec<u8>]) -> HashSet<Vec<u8>> {
    if paths.is_empty() {
        return HashSet::new();
    }
    let mut arguments: Vec<OsString> = ["diff-files", "-z", "--name-only", "--"]
        .iter()
        .map(OsString::from)
        .collect();
    // Past a certain number of paths the command line stops being a way to ask the
    // question, so ask about the whole worktree instead and let the caller pick out the
    // paths it cares about.
    if paths.len() <= RACY_PATH_LIMIT {
        arguments.extend(paths.iter().map(|path| as_path(path).into_os_string()));
    }
    let Ok(output) = git.capture(Some(source), arguments) else {
        return paths.iter().cloned().collect();
    };
    output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

/// Clones the plan and reports what actually landed.
///
/// The first failed clone demotes the run: the rest of the plan is abandoned rather than
/// retried path by path, because a filesystem that cannot clone one file cannot clone the
/// next one either. Whatever was cloned before the failure is still correct and is kept.
fn materialise(
    cloner: &dyn BlockCloner,
    source: &Path,
    destination: &Path,
    plan: &plan::Plan,
) -> (Vec<Record>, Option<String>) {
    let mut demotion = None;
    let umask = umask();

    for directory in &plan.directories {
        if interrupt::requested() {
            demotion = Some("interrupted".to_string());
            break;
        }
        let target = destination.join(as_path(directory));
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = cloner.clone_directory(&source.join(as_path(directory)), &target) {
            let _ = std::fs::remove_dir_all(&target);
            demotion = Some(format!("cloning a directory failed: {error}"));
            break;
        }
    }

    if demotion.is_none() {
        for planned in &plan.files {
            if interrupt::requested() {
                demotion = Some("interrupted".to_string());
                break;
            }
            let target = destination.join(as_path(&planned.path));
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(error) = clone_one(
                cloner,
                &source.join(as_path(&planned.path)),
                &target,
                planned,
            ) {
                let _ = std::fs::remove_file(&target);
                demotion = Some(format!("cloning a file failed: {error}"));
                break;
            }
        }
    }

    if demotion.is_none() {
        for directory in &plan.directories_created {
            set_mode(&destination.join(as_path(directory)), directory_mode(umask));
        }
    }

    let records = plan
        .materialised
        .iter()
        .filter_map(|planned| {
            let target = destination.join(as_path(&planned.path));
            conform_to_checkout(&target, planned, umask).map(|stat| Record {
                path: planned.path.clone(),
                mode: planned.mode,
                oid: planned.oid,
                stat,
            })
        })
        .collect();

    (records, demotion)
}

/// Materialises one path. A symlink in the source is recreated as a symlink; there are no
/// blocks to share, and a platform that stores symlinks as plain files takes the file path.
fn clone_one(
    cloner: &dyn BlockCloner,
    source: &Path,
    destination: &Path,
    planned: &Planned,
) -> io::Result<()> {
    let is_symlink = std::fs::symlink_metadata(source)?.file_type().is_symlink();
    if planned.mode == 0o120000 && is_symlink {
        let target = std::fs::read_link(source)?;
        return symlink(&target, destination);
    }
    cloner.clone_file(source, destination)
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(not(any(unix, windows)))]
fn symlink(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// Gives the clone the permissions and modification time a checkout would have produced,
/// and returns the stat data that describes it afterwards.
///
/// A block clone copies the source's permission bits along with its blocks, but a checkout
/// derives them from the tree: `0777` for an executable and `0666` otherwise, each masked
/// by the process umask. So a source file somebody ran `chmod 600` on, or that a filter
/// created under a different umask, arrives with permissions git would never have written.
/// Spec §6.1 puts it plainly — take modes from the tree, never from `stat`.
///
/// The modification time is set for a different reason. Git treats an index entry whose
/// mtime is not older than the index file's own mtime as racily clean and re-reads the
/// file, which would undo the whole point of cloning. A clone carrying the source's mtime
/// is comfortably older than the index we are about to write. `clonefile` already copies
/// timestamps; the reflink primitives do not.
///
/// Permissions are set first: changing them moves ctime, and the stat data has to describe
/// the file as it is finally left.
fn conform_to_checkout(path: &Path, planned: &Planned, umask: u32) -> Option<Stat> {
    if planned.mode != SYMLINK_MODE {
        set_mode(path, checkout_mode(planned.mode, umask));
    }
    let wanted = FileTime::from_unix_time(i64::from(planned.mtime.secs), planned.mtime.nsecs);
    let mut metadata = Metadata::from_path_no_follow(path).ok()?;
    if Stat::from_fs(&metadata).ok()?.mtime != planned.mtime {
        filetime::set_symlink_file_times(path, wanted, wanted).ok()?;
        metadata = Metadata::from_path_no_follow(path).ok()?;
    }
    Stat::from_fs(&metadata).ok()
}

/// The permissions a checkout gives a path with this tree mode.
fn checkout_mode(mode: u32, umask: u32) -> u32 {
    let base = if mode == EXECUTABLE_MODE {
        0o777
    } else {
        0o666
    };
    base & !umask
}

/// The permissions a checkout gives a directory it has to create.
fn directory_mode(umask: u32) -> u32 {
    0o777 & !umask
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

/// The process umask, read the only way the C library allows: by setting it and putting it
/// back. Nothing else in the process creates files while this runs.
#[cfg(unix)]
fn umask() -> u32 {
    // SAFETY: single-threaded at this point, so no other file creation can see the gap.
    unsafe {
        let previous = libc::umask(0);
        libc::umask(previous);
        u32::from(previous)
    }
}

#[cfg(not(unix))]
fn umask() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_come_from_the_tree_and_the_umask() {
        assert_eq!(checkout_mode(0o100644, 0o022), 0o644);
        assert_eq!(checkout_mode(0o100755, 0o022), 0o755);
        assert_eq!(checkout_mode(0o100644, 0o077), 0o600);
        assert_eq!(checkout_mode(0o100755, 0o077), 0o700);
        assert_eq!(checkout_mode(0o100644, 0o000), 0o666);
        assert_eq!(directory_mode(0o022), 0o755);
    }

    #[test]
    fn applies_every_dash_c_in_order() {
        let globals = ["-C", "/repo", "-c", "x=y", "-C", "sub"]
            .iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        assert_eq!(working_directory(&globals), PathBuf::from("/repo/sub"));
    }
}
