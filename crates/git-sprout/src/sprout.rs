// ABOUTME: Runs an accelerated `git worktree add`: git creates and finishes the worktree,
// ABOUTME: and in between the tool clones whatever the source checkout can supply.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use filetime::FileTime;
use gix_hash::ObjectId;
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

/// The configuration that decides how blob bytes become working-tree bytes, with
/// `core.ignoreCase` among it because it decides how attribute patterns match. Cloning is
/// only sound when both worktrees resolve all of it the same way.
///
/// Each key comes with the `git config --type` git reads it as, so that two spellings of
/// one value compare equal and two values one spelling hides do not: `git config --get`
/// prints both a key with no value (true) and an empty one (false) as nothing.
const CONVERSION_KEYS: &[(&str, &str)] = &[
    ("core.autocrlf", "bool-or-str"),
    ("core.eol", "bool-or-str"),
    ("core.symlinks", "bool"),
    ("core.ignoreCase", "bool"),
];

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

    let destination = locate(command);

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

/// Resolves this invocation's destination without inspecting other concurrent creations.
fn locate(command: &AddCommand) -> Option<PathBuf> {
    let requested = working_directory(&command.globals).join(as_path_os(&command.path));
    if !requested.join(".git").exists() {
        return None;
    }
    std::fs::canonicalize(requested).ok()
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
        has_many_files(git, destination),
    );
    if version != scratch_index::SUPPORTED_VERSION {
        stats.fall_back(format!("the repository writes index version {version}"));
        return;
    }
    // Git keeps the shape of the index it reads, the same way it keeps the version.
    // In a `core.splitIndex` repository a plain `git worktree add` writes a small
    // index carrying a `link` extension, with the entries in a `sharedindex` file;
    // a scratch index written whole would make git write the final one whole too,
    // and the difference is visible in the worktree's admin directory.
    if splits_the_index(git, destination) {
        stats.fall_back("the repository splits the index");
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
    let source_paths: Vec<Vec<u8>> = source_index
        .entries()
        .iter()
        .map(|entry| entry.path(&source_index).to_vec())
        .collect();
    let (colliding, colliding_prefixes) = plan::colliding_paths(&target, &source_paths);
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
    drop_resized_paths(git, &source, &mut verified.paths);

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
    for (key, kind) in CONVERSION_KEYS {
        match (
            git.config_typed(source, kind, key),
            git.config_typed(destination, kind, key),
        ) {
            (Ok(source), Ok(destination)) if source == destination => {}
            (Ok(_), Ok(_)) => return Some(format!("{key} differs between the worktrees")),
            _ => return Some(format!("{key} cannot be read")),
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
    let Ok(endings) = line_endings(git, source) else {
        paths.clear();
        return;
    };
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

/// The line-ending options as git reads them. `core.autocrlf` is a boolean or `input`,
/// and git accepts every boolean spelling for it, so git canonicalises it here rather than
/// this matching one spelling. `core.eol` goes through the same reading, which leaves
/// `lf`, `crlf` and `native` as written. An option git cannot be asked about is an
/// error, and the caller then clones nothing it would have to judge.
fn line_endings(git: &Git, source: &Path) -> io::Result<LineEndings> {
    let autocrlf = git.config_typed(source, "bool-or-str", "core.autocrlf")?;
    let eol = git.config_typed(source, "bool-or-str", "core.eol")?;
    Ok(LineEndings::from_config(
        autocrlf.as_deref(),
        eol.as_deref(),
    ))
}

/// Removes the paths whose source file is not the size of the blob a checkout would write.
///
/// Once the converted paths are gone, a checkout writes every remaining blob's bytes as
/// they are, so the source file must be exactly that long. A clean stat cache does not
/// guarantee it: the source may still hold bytes an earlier attributes rule wrote, such
/// as CRLF endings from an `eol=crlf` that has since been removed. Git leaves an
/// unchanged blob's file alone when it switches commits, so those bytes survive the
/// switch, and the index entry refreshed under the old rule still matches them.
///
/// Comparing sizes catches every conversion that changes length; one that kept the length
/// would get through, and finding it would mean hashing every file. A symlink's size on
/// disk is the length of its target, which is its blob, and the same holds for the
/// placeholder file a checkout writes when `core.symlinks` is off.
fn drop_resized_paths(git: &Git, source: &Path, paths: &mut Vec<Planned>) {
    if paths.is_empty() {
        return;
    }
    let oids: HashSet<ObjectId> = paths.iter().map(|planned| planned.oid).collect();
    let mut request = Vec::new();
    for oid in &oids {
        request.extend_from_slice(oid.to_string().as_bytes());
        request.push(b'\n');
    }
    let Ok(output) = git.capture_with_input(
        Some(source),
        [
            "cat-file",
            "--batch-check=%(objectname) %(objectsize)",
            "--buffer",
        ],
        &request,
    ) else {
        paths.clear();
        return;
    };
    let sizes = parse_object_sizes(&output);
    paths.retain(|planned| sizes.get(&planned.oid) == Some(&planned.size));
}

/// Reads `git cat-file --batch-check='%(objectname) %(objectsize)'` output. A line that
/// is not an object id and a size, such as git's `<oid> missing`, names no size, so the
/// paths that need it are dropped.
fn parse_object_sizes(output: &[u8]) -> HashMap<ObjectId, u64> {
    output
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            let text = std::str::from_utf8(line).ok()?;
            let (oid, size) = text.split_once(' ')?;
            Some((ObjectId::from_hex(oid.as_bytes()).ok()?, size.parse().ok()?))
        })
        .collect()
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
        // Its std replacement, `std::fs::set_times_nofollow`, is newer than our rust-version.
        #[allow(deprecated)]
        filetime::set_symlink_file_times(path, wanted, wanted).ok()?;
        metadata = Metadata::from_path_no_follow(path).ok()?;
    }
    Stat::from_fs(&metadata).ok()
}

/// Whether this repository writes a split index, from the same sources git consults.
///
/// `GIT_TEST_SPLIT_INDEX` is git's own test hook and is honoured for the same reason
/// `GIT_INDEX_VERSION` is: a repository under test still has to come out identical.
fn splits_the_index(git: &Git, destination: &Path) -> bool {
    if std::env::var("GIT_TEST_SPLIT_INDEX").as_deref() == Ok("1") {
        return true;
    }
    is_on(git, destination, "core.splitIndex")
}

/// Whether `feature.manyFiles` is on, which makes a fresh index version 4.
fn has_many_files(git: &Git, destination: &Path) -> bool {
    is_on(git, destination, "feature.manyFiles")
}

/// Whether a boolean option is on, with git reading it so that every spelling it accepts
/// (`yes`, `on`, `1`, `TRUE`, a key with no value) counts. A value git cannot read counts
/// as on: both options that use this make the caller leave the worktree to git.
fn is_on(git: &Git, directory: &Path, key: &str) -> bool {
    git.config_typed(directory, "bool", key)
        .map_or(true, |value| value.as_deref() == Some("true"))
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
    // `mode_t` is 16 bits on macOS and 32 on Linux, so the widening is a real
    // conversion on one target and a no-op on the other. Neither target should
    // have to spell the type out.
    #[allow(clippy::useless_conversion)]
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
    use std::ffi::OsStr;

    #[test]
    fn permissions_come_from_the_tree_and_the_umask() {
        assert_eq!(checkout_mode(0o100644, 0o022), 0o644);
        assert_eq!(checkout_mode(0o100755, 0o022), 0o755);
        assert_eq!(checkout_mode(0o100644, 0o077), 0o600);
        assert_eq!(checkout_mode(0o100755, 0o077), 0o700);
        assert_eq!(checkout_mode(0o100644, 0o000), 0o666);
        assert_eq!(directory_mode(0o022), 0o755);
    }

    /// `feature.manyFiles` and `core.splitIndex` are booleans to git, so every spelling it
    /// accepts has to count, or the scratch index takes a version or shape git would not
    /// write. A value git rejects counts as set, which falls back to git.
    #[test]
    fn index_options_read_every_boolean_spelling_git_accepts() {
        let repo =
            std::env::temp_dir().join(format!("git-sprout-index-options-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        let git = Git::new(Vec::new());
        git.capture(
            None,
            [OsStr::new("init"), OsStr::new("-q"), repo.as_os_str()],
        )
        .expect("git init");
        let read = |key: &str, value: &str| {
            let _ = git.capture(Some(&repo), ["config", "--unset-all", key]);
            git.capture(Some(&repo), ["config", key, value])
                .expect("git config");
            match key {
                "feature.manyFiles" => has_many_files(&git, &repo),
                _ => splits_the_index(&git, &repo),
            }
        };

        for key in ["feature.manyFiles", "core.splitIndex"] {
            for spelling in ["true", "yes", "on", "1", "TRUE", "bogus"] {
                assert!(read(key, spelling), "{key}={spelling}");
            }
            for spelling in ["false", "no", "off", "0", "FALSE", ""] {
                assert!(!read(key, spelling), "{key}={spelling}");
            }
            let _ = git.capture(Some(&repo), ["config", "--unset-all", key]);
        }
        assert!(!has_many_files(&git, &repo), "unset feature.manyFiles");
        assert!(!splits_the_index(&git, &repo), "unset core.splitIndex");

        let _ = std::fs::remove_dir_all(&repo);
    }

    /// Git reads `core.autocrlf` as a boolean and `core.eol` without regard to case, so
    /// every spelling it accepts has to reach the plan as what it means, or a path is
    /// cloned with LF where a checkout writes CRLF. Real git does the reading here, because
    /// the reading is git's to make.
    #[test]
    fn line_endings_read_every_spelling_git_accepts() {
        let repo =
            std::env::temp_dir().join(format!("git-sprout-line-endings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        let git = Git::new(Vec::new());
        git.capture(
            None,
            [OsStr::new("init"), OsStr::new("-q"), repo.as_os_str()],
        )
        .expect("git init");
        let read = |autocrlf: Option<&str>, eol: Option<&str>| {
            for (key, value) in [("core.autocrlf", autocrlf), ("core.eol", eol)] {
                let _ = git.capture(Some(&repo), ["config", "--unset-all", key]);
                if let Some(value) = value {
                    git.capture(Some(&repo), ["config", key, value])
                        .expect("git config");
                }
            }
            line_endings(&git, &repo).expect("git answers")
        };

        for spelling in ["true", "yes", "on", "1", "TRUE", "input", "INPUT"] {
            assert!(
                read(Some(spelling), None).autocrlf,
                "core.autocrlf={spelling}"
            );
        }
        for spelling in ["false", "no", "off", "0", "FALSE", ""] {
            assert!(
                !read(Some(spelling), None).autocrlf,
                "core.autocrlf={spelling}"
            );
        }
        for spelling in ["crlf", "CRLF", "bogus"] {
            assert!(read(None, Some(spelling)).crlf, "core.eol={spelling}");
        }
        for spelling in ["lf", "LF", "native", "Native"] {
            assert!(!read(None, Some(spelling)).crlf, "core.eol={spelling}");
        }

        // A key with no value at all is `true` to git.
        let _ = git.capture(Some(&repo), ["config", "--unset-all", "core.autocrlf"]);
        let config = repo.join(".git").join("config");
        let mut text = std::fs::read_to_string(&config).expect("read config");
        text.push_str("[core]\n\tautocrlf\n");
        std::fs::write(&config, text).expect("write config");
        assert!(line_endings(&git, &repo).expect("git answers").autocrlf);

        let _ = std::fs::remove_dir_all(&repo);
    }

    /// A repository with one commit, its main worktree detached, and a second worktree on
    /// branch `topic` (not checked out) whose configuration includes the file returned
    /// last, through `includeIf.onbranch`, which the main worktree never reads.
    fn topic_worktree(name: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let scratch =
            std::env::temp_dir().join(format!("git-sprout-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let (repo, destination) = (scratch.join("repo"), scratch.join("destination"));
        let git = Git::new(Vec::new());
        git.capture(
            None,
            [OsStr::new("init"), OsStr::new("-q"), repo.as_os_str()],
        )
        .expect("git init");
        std::fs::write(repo.join("a.txt"), "one\n").expect("write file");
        for args in [
            &["add", "--", "."][..],
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.invalid",
                "commit",
                "-q",
                "--no-gpg-sign",
                "-m",
                "base",
            ],
            &["checkout", "-q", "--detach"],
            &["config", "includeIf.onbranch:topic.path", "topic-config"],
        ] {
            git.capture(Some(&repo), args).expect("git");
        }
        git.capture(
            Some(&repo),
            [
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("-q"),
                OsStr::new("--no-checkout"),
                OsStr::new("-b"),
                OsStr::new("topic"),
                destination.as_os_str(),
            ],
        )
        .expect("git worktree add");
        let include = repo.join(".git").join("topic-config");
        (scratch, repo, destination, include)
    }

    /// An include can give the destination settings the source never reads, such as one
    /// `includeIf.onbranch` adds for the destination's branch alone. Git reads them when it
    /// checks the destination out, so the source's view of the paths is then no guide to
    /// what the checkout writes.
    #[test]
    fn settings_only_the_destination_reads_are_a_mismatch() {
        let (scratch, repo, destination, include) = topic_worktree("destination-settings");
        let git = Git::new(Vec::new());

        assert_eq!(conversion_mismatch(&git, &repo, &destination), None);
        // `core.ignoreCase` decides whether a rule such as `*.TXT -text` covers `a.txt`.
        std::fs::write(&include, "[core]\n\tignoreCase = false\n").expect("write include");
        let _ = git.capture(Some(&repo), ["config", "core.ignoreCase", "true"]);
        assert_eq!(
            conversion_mismatch(&git, &repo, &destination),
            Some("core.ignoreCase differs between the worktrees".to_string()),
        );
        std::fs::write(&include, "").expect("clear include");

        // Git reads a boolean however it is spelled: a key with no value is true and an
        // empty value false, though `git config --get` prints both as nothing. The include
        // overrides the key only if it comes after it, so the key goes in first.
        let _ = git.capture(Some(&repo), ["config", "--unset-all", "core.ignoreCase"]);
        let config = repo.join(".git").join("config");
        let text = std::fs::read_to_string(&config).expect("read config");
        let text = text.replacen("[includeIf", "[core]\n\tignoreCase\n[includeIf", 1);
        std::fs::write(&config, text).expect("write config");
        std::fs::write(&include, "[core]\n\tignoreCase =\n").expect("write include");
        assert_eq!(
            conversion_mismatch(&git, &repo, &destination),
            Some("core.ignoreCase differs between the worktrees".to_string()),
        );
        // A value git cannot read as a boolean is no answer at all.
        std::fs::write(&include, "[core]\n\tignoreCase = maybe\n").expect("write include");
        assert_eq!(
            conversion_mismatch(&git, &repo, &destination),
            Some("core.ignoreCase cannot be read".to_string()),
        );
        std::fs::write(&include, "").expect("clear include");
        let _ = git.capture(Some(&repo), ["config", "--unset-all", "core.ignoreCase"]);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn applies_every_dash_c_in_order() {
        // The first -C is spelled as this platform spells an absolute path, so the
        // test asserts the ordering rather than the separator.
        let root = if cfg!(windows) { "C:\\repo" } else { "/repo" };
        let globals = ["-C", root, "-c", "x=y", "-C", "sub"]
            .iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        assert_eq!(working_directory(&globals), PathBuf::from(root).join("sub"));
    }

    #[test]
    fn reads_object_sizes_at_either_hash_length() {
        let sha1 = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
        let sha256 = "473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813";
        let output = format!("{sha1} 0\n{sha256} 1234\n");
        let sizes = parse_object_sizes(output.as_bytes());
        assert_eq!(sizes[&ObjectId::from_hex(sha1.as_bytes()).unwrap()], 0);
        assert_eq!(sizes[&ObjectId::from_hex(sha256.as_bytes()).unwrap()], 1234);
    }

    #[test]
    fn a_missing_object_has_no_size() {
        let oid = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
        let sizes = parse_object_sizes(format!("{oid} missing\n").as_bytes());
        assert!(sizes.is_empty());
    }
}
