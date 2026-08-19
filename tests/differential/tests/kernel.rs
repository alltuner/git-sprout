// ABOUTME: The Linux kernel parity test: 95 056 tracked files including thirteen
// ABOUTME: pairs of paths that differ only by case, which a case-insensitive volume
// ABOUTME: cannot hold. Whatever git leaves behind is the correct answer.

use differential::compare;
use differential::env::Workspace;
use differential::run::{self, Side, Tool};
use differential::snapshot::{self, Snapshot, TreeEntry};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The thirteen paths real `git worktree add` reports as modified on a
/// case-insensitive volume, because the lower-case member of each colliding pair is
/// checked out last and overwrites its twin. Measured on git 2.55.0.
const EXPECTED_DIRTY: &[&str] = &[
    " M include/uapi/linux/netfilter/xt_CONNMARK.h",
    " M include/uapi/linux/netfilter/xt_DSCP.h",
    " M include/uapi/linux/netfilter/xt_MARK.h",
    " M include/uapi/linux/netfilter/xt_RATEEST.h",
    " M include/uapi/linux/netfilter/xt_TCPMSS.h",
    " M include/uapi/linux/netfilter_ipv4/ipt_ECN.h",
    " M include/uapi/linux/netfilter_ipv4/ipt_TTL.h",
    " M include/uapi/linux/netfilter_ipv6/ip6t_HL.h",
    " M net/netfilter/xt_DSCP.c",
    " M net/netfilter/xt_HL.c",
    " M net/netfilter/xt_RATEEST.c",
    " M net/netfilter/xt_TCPMSS.c",
    " M tools/memory-model/litmus-tests/Z6.0+pooncelock+poonceLock+pombonce.litmus",
];

/// One colliding pair, used to assert which member won on disk. A status-only check
/// would pass for an implementation that produced thirteen modified paths with the
/// wrong content in them.
const UPPER: &str = "include/uapi/linux/netfilter/xt_CONNMARK.h";
const LOWER: &str = "include/uapi/linux/netfilter/xt_connmark.h";

/// Well below any real kernel checkout. The exact count moves with every kernel
/// release, so the fixture is recognised by being large rather than by a literal;
/// what has to match exactly is the two sides against each other.
const TRACKED_FILES_FLOOR: usize = 50_000;
const DESTINATION: &str = "kernel-differential-wt";
const BRANCH: &str = "kernel-differential";

/// The fixture repository is used exclusively for the duration of this test: the
/// two sides run sequentially in it, so anything else creating a ref or a worktree
/// in the same repository in between shows up as a difference that is real but not
/// the implementation's. `just fixtures` puts a clone under `tests/fixtures/cache`
/// for this purpose; point `SPROUT_KERNEL_REPO` at another one only if nothing else
/// is using it.
///
/// Deliberately not asserted anywhere below: the worktree's tree oid. It is written
/// from the index, not from the working tree, so it comes back identical to
/// `HEAD^{tree}` even with thirteen files physically wrong on disk. It is a
/// tautology, not a parity signal. The porcelain set, the on-disk content hashes and
/// the index entry comparison are what actually prove anything here.
#[test]
#[ignore = "needs the 2 GB kernel clone from `just fixtures`"]
fn kernel_worktree_matches_git() {
    let Some(repo) = kernel_repo() else {
        panic!(
            "no kernel clone found. Run `just fixtures`, or point SPROUT_KERNEL_REPO at an \
             existing shallow clone of torvalds/linux."
        );
    };
    let cache = repo
        .parent()
        .expect("the kernel clone has a parent directory")
        .to_path_buf();
    let workspace = Workspace::create("kernel").expect("scratch workspace");
    let tool = Tool::candidate();
    println!("kernel repository: {}", repo.display());
    println!("candidate: {}", tool.describe());

    // A bare or --no-checkout clone has no working tree to clone from, so nothing can
    // be accelerated against it and the failure would read as a tool bug.
    let populated = run::git_line(&workspace, &repo, &["ls-files"]);
    assert!(
        !populated.is_empty(),
        "the kernel fixture at {} has an empty index, so it has no working tree to clone \
         from and no implementation could accelerate against it. Run `git -C {} checkout` \
         first; a --no-checkout or bare clone is not a usable fixture.",
        repo.display(),
        repo.display()
    );

    let case_insensitive = is_case_insensitive(&cache);
    println!(
        "destination volume is case-{}",
        if case_insensitive {
            "insensitive"
        } else {
            "sensitive"
        }
    );

    // The fixture repository is a cache, not a scratch directory: an interrupted run
    // leaves a branch and a worktree behind, so start from a known state.
    cleanup(&workspace, &repo, &cache);

    let control = run_side(&workspace, &cache, &repo, "control", &Tool::Git);
    check_kernel_facts(&workspace, &control, case_insensitive);
    cleanup(&workspace, &repo, &cache);

    let candidate = run_side(&workspace, &cache, &repo, "candidate", &tool);
    check_kernel_facts(&workspace, &candidate, case_insensitive);
    check_collision_winner(
        &workspace,
        &repo,
        &cache.join(DESTINATION),
        case_insensitive,
    );
    cleanup(&workspace, &repo, &cache);

    let differences = compare::compare(&control, &candidate);
    assert!(
        differences.is_empty(),
        "{} differences between git and the candidate on the kernel:\n  {}",
        differences.len(),
        differences
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    check_acceleration(&candidate);
}

/// The kernel is where acceleration is worth having, so a run against a real
/// binary has to show it happened. Absent statistics mean an uninstrumented
/// binary, which is reported rather than silently accepted.
fn check_acceleration(candidate: &Snapshot) {
    let Some(stats) = &candidate.stats else {
        println!(
            "NOT MEASURED: the candidate reported no statistics, so this run proved parity \
             only and not that anything was cloned."
        );
        assert!(
            std::env::var_os("SPROUT_REQUIRE_STATS").is_none(),
            "the candidate reported no statistics and SPROUT_REQUIRE_STATS is set"
        );
        return;
    };
    println!("candidate statistics: {stats:?}");
    assert!(
        stats.accelerated(),
        "nothing was cloned on the fixture that exists to prove cloning works \
         (cloned={}, reason={:?}); the worktree may be correct but the fast path never ran",
        stats.cloned,
        stats.fallback_reason
    );
    assert!(
        stats.cloned + stats.skipped > TRACKED_FILES_FLOOR as u64,
        "every tracked path should have been either cloned or skipped, but the two \
         counters only account for {}",
        stats.cloned + stats.skipped
    );
    // The one-call subtree clone is a separate mechanism from the per-file clone and
    // can stop firing without moving any other counter.
    #[cfg(target_os = "macos")]
    assert!(
        stats.cloned_directories > 0,
        "no subtree was cloned in one call on a platform that offers directory cloning"
    );
}

fn kernel_repo() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SPROUT_KERNEL_REPO") {
        let path = PathBuf::from(path);
        return path.join(".git").exists().then_some(path);
    }
    let default = differential::fixtures::fixtures_dir()
        .join("cache")
        .join("linux");
    default.join(".git").exists().then_some(default)
}

fn run_side(
    workspace: &Workspace,
    cache: &Path,
    repo: &Path,
    label: &'static str,
    tool: &Tool,
) -> Snapshot {
    let side = Side {
        label,
        root: cache.to_path_buf(),
        repo: repo.to_path_buf(),
        worktree: cache.join(DESTINATION),
        hook_log: workspace.root().join(format!("hooks-{label}.log")),
        stats_file: workspace.root().join(format!("stats-{label}.json")),
    };
    run::install_hooks(&side.repo, &side.hook_log).expect("hooks");

    let argv: Vec<String> = vec![
        format!("../{DESTINATION}"),
        "-b".to_string(),
        BRANCH.to_string(),
    ];
    let started = Instant::now();
    let output = run::worktree_add(workspace, &side, tool, &argv).expect("worktree add");
    let elapsed = started.elapsed();
    assert!(
        !output.timed_out,
        "{label} never finished and was killed after {:.0}s. A worktree add that hangs on \
         95 056 paths is a deadlock, not a slow machine; raise SPROUT_ADD_TIMEOUT_SECS only \
         if the machine is genuinely that slow.",
        elapsed.as_secs_f64()
    );
    assert_eq!(
        output.status,
        0,
        "{label} add failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{label}: worktree created in {:.2}s", elapsed.as_secs_f64());
    if let Some(stats) = &output.stats {
        println!("{label}: {stats:?}");
    }

    let object_format = snapshot::object_format(workspace, &side.repo);
    let snapshot = snapshot::capture(workspace, &side, output, &object_format).expect("snapshot");
    println!(
        "{label}: {} tracked entries, {} files on disk, {} dirty paths",
        snapshot.ls_files_stage.len(),
        count_files(&snapshot),
        snapshot.porcelain.len()
    );
    snapshot
}

fn check_kernel_facts(_workspace: &Workspace, snapshot: &Snapshot, case_insensitive: bool) {
    let label = snapshot.label;
    // The kernel gains and loses files, so the count is whatever the clone CI made
    // today. What has to hold is that it is large enough to cross git's progress
    // threshold and to exercise the directory-clone path, and that both sides agree
    // — which the snapshot comparison checks exactly.
    assert!(
        snapshot.ls_files_stage.len() > TRACKED_FILES_FLOOR,
        "{label}: the kernel fixture tracks {} files, too few to be the kernel",
        snapshot.ls_files_stage.len()
    );

    if case_insensitive {
        let expected: Vec<String> = EXPECTED_DIRTY.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            snapshot.porcelain, expected,
            "{label}: the dirty set on a case-insensitive volume is not the thirteen \
             collisions git produces"
        );
        assert_eq!(
            count_files(snapshot),
            snapshot.ls_files_stage.len() - EXPECTED_DIRTY.len(),
            "{label}: {} of the tracked paths should be lost to case collisions",
            EXPECTED_DIRTY.len()
        );
    } else {
        assert!(
            snapshot.porcelain.is_empty(),
            "{label}: a case-sensitive volume holds the whole tree, so the worktree should \
             be clean, but {} paths are dirty: {:?}",
            snapshot.porcelain.len(),
            &snapshot.porcelain[..snapshot.porcelain.len().min(20)]
        );
        assert_eq!(
            count_files(snapshot),
            snapshot.ls_files_stage.len(),
            "{label}: a case-sensitive volume should hold every tracked file"
        );
    }
}

/// In each colliding pair the lower-case member is checked out last and wins, so the
/// bytes on disk under the upper-case name are the lower-case file's blob.
fn check_collision_winner(
    workspace: &Workspace,
    repo: &Path,
    worktree: &Path,
    case_insensitive: bool,
) {
    if !case_insensitive {
        return;
    }
    let upper_blob = run::git_line(workspace, repo, &["rev-parse", &format!("HEAD:{UPPER}")]);
    let lower_blob = run::git_line(workspace, repo, &["rev-parse", &format!("HEAD:{LOWER}")]);
    assert_ne!(
        upper_blob, lower_blob,
        "the colliding pair should hold different blobs"
    );

    let on_disk = run::git_line(
        workspace,
        worktree,
        &[
            "hash-object",
            worktree.join(UPPER).to_string_lossy().as_ref(),
        ],
    );
    assert_eq!(
        on_disk, lower_blob,
        "the bytes on disk at {UPPER} should be {LOWER}'s blob, because the lower-case \
         member is checked out last and overwrites its twin"
    );
    println!("collision winner: {UPPER} on disk holds {LOWER}'s blob {lower_blob}");
}

fn count_files(snapshot: &Snapshot) -> usize {
    snapshot
        .tree
        .values()
        .filter(|e| matches!(e, TreeEntry::File { .. } | TreeEntry::Symlink { .. }))
        .count()
}

fn cleanup(workspace: &Workspace, repo: &Path, cache: &Path) {
    let dest = cache.join(DESTINATION);
    let _ = run::git_full(
        workspace,
        repo,
        &[
            "worktree",
            "remove",
            "--force",
            dest.to_string_lossy().as_ref(),
        ],
    );
    let _ = differential::env::remove_tree(&dest);
    let _ = run::git_full(workspace, repo, &["worktree", "prune"]);
    let _ = run::git_full(workspace, repo, &["branch", "-D", BRANCH]);
    for hook in [
        "reference-transaction",
        "post-index-change",
        "post-checkout",
    ] {
        let _ = std::fs::remove_file(repo.join(".git").join("hooks").join(hook));
    }
}

fn is_case_insensitive(dir: &Path) -> bool {
    let probe = dir.join("SproutCaseProbe");
    let twin = dir.join("sproutcaseprobe");
    let _ = std::fs::remove_file(&probe);
    let _ = std::fs::remove_file(&twin);
    if std::fs::write(&probe, b"probe").is_err() {
        return false;
    }
    let insensitive = twin.exists();
    let _ = std::fs::remove_file(&probe);
    insensitive
}
