// ABOUTME: Records whether a checked-out file's modification time is the checkout's
// ABOUTME: or the source's, which is observable to make and every other build system.

use differential::case::{Outcome, Runner};
use differential::files;
use differential::flags;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

/// A tree big enough that cloning has something to do and the answer is not one file.
const FIXTURE: &str = "text-heavy";

/// `git worktree add` writes every file at checkout time. A copy-on-write clone
/// preserves the source's timestamps instead, so a cloned worktree can look older
/// than it is. Nothing in the compatibility contract exempts that, and it is exactly
/// what `make` reads, so this test states which of the two the candidate produced
/// rather than leaving it to be discovered by a stale rebuild.
///
/// See NOTES-spec-disagreement.md, "Modification times of cloned files".
#[test]
fn checked_out_files_carry_the_checkout_time_or_the_source_time() {
    let runner = Runner::new("mtimes").expect("scratch workspace");
    let template = runner.template(FIXTURE).expect("fixture builder");
    if let Some(reason) = &template.skipped {
        println!("skipped fixture {FIXTURE}: {reason}");
        return;
    }
    let flags = flags::by_name("new-branch").expect("flag case");
    let Outcome::Ran(result) = runner.run(&template, flags, None).expect("case run") else {
        panic!("the flag case did not run");
    };

    let control = inherited(&result.case_dir.join("a"));
    let candidate = inherited(&result.case_dir.join("b"));

    println!(
        "control:   {} of {} checked-out files kept the source's mtime",
        control.inherited, control.total
    );
    println!(
        "candidate: {} of {} checked-out files kept the source's mtime",
        candidate.inherited, candidate.total
    );
    if let Some(stats) = &result.candidate.stats {
        println!("candidate cloned {} paths", stats.cloned);
    }
    println!(
        "verdict: the candidate's worktree {}",
        if candidate.inherited == 0 {
            "carries checkout timestamps, matching git"
        } else {
            "carries source timestamps for the paths it cloned, which git does not do"
        }
    );

    assert!(
        control.total > 0,
        "the fixture produced no comparable files"
    );
    assert_eq!(
        control.inherited, 0,
        "real `git worktree add` wrote {} files with the source's mtime, which it should \
         never do; the measurement below is meaningless if this is wrong",
        control.inherited
    );

    assert!(
        result.differences.is_empty(),
        "the case diverged before mtimes could be judged: {:?}",
        result.differences
    );
    runner.release(&result.case_dir);
}

struct Counts {
    inherited: usize,
    total: usize,
}

/// Compares each file in the worktree against the same path in the repository it was
/// created from. Equal modification times mean the file was cloned rather than
/// written, since a checkout stamps the moment it ran.
fn inherited(side: &Path) -> Counts {
    let repo = side.join("repo");
    let worktree = side.join("wt");
    let source = mtimes(&repo);
    let mut counts = Counts {
        inherited: 0,
        total: 0,
    };
    for (path, time) in mtimes(&worktree) {
        let Some(origin) = source.get(&path) else {
            continue;
        };
        counts.total += 1;
        if *origin == time {
            counts.inherited += 1;
        }
    }
    counts
}

fn mtimes(root: &Path) -> BTreeMap<String, SystemTime> {
    let mut out = BTreeMap::new();
    let paths = files::walk(root).unwrap_or_else(|e| panic!("cannot walk {}: {e}", root.display()));
    for relative in paths {
        let key = relative.to_string_lossy().replace('\\', "/");
        if key == ".git" || key.starts_with(".git/") {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(root.join(&relative)) else {
            continue;
        };
        if !meta.is_file() || meta.file_type().is_symlink() {
            continue;
        }
        if let Ok(time) = meta.modified() {
            out.insert(key, time);
        }
    }
    out
}
