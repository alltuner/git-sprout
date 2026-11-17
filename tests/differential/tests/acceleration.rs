// ABOUTME: Asserts the acceleration actually happened on the fixtures built for it.
// ABOUTME: A correctness-only test passes trivially when nothing was cloned at all.

use differential::case::{Outcome, Runner};
use differential::flags;
use differential::run::Tool;

/// Fixtures whose whole point is that cloning fires.
///
/// `attrs-reserved-values` runs under `core.autocrlf=true`, and its paths marked `-crlf` or
/// `-text` are binary to git, so a checkout writes them as stored and they are cloned,
/// while its paths given the string values `unset` and `unspecified`, which
/// `git check-attr` prints as it prints those states, are converted and left to git.
/// `attrs-untracked` leaves to git the directory an ignored `.gitattributes` marks in the
/// source alone, and still clones the one a tracked `-text` marks on both sides, and
/// `attrs-assumed` and `attrs-filtered` do the same for a tracked one changed under
/// `--assume-unchanged` or behind a clean filter.
const ACCELERATED: &[&str] = &[
    "text-heavy",
    "attrs-reserved-values",
    "attrs-untracked",
    "attrs-assumed",
    "attrs-filtered",
];

/// Fixtures where cloning is expected to fire on nothing at all.
///
/// This inverts what spec §8 asks for, deliberately. §8 says to assert `cloned > 0`
/// on `core.autocrlf=true`, because §4 step 5's index-based rule was meant to make a
/// repository whose every text file is converted on checkout accelerate normally.
/// That rule turned out to be unsound: a stat-clean index entry means the working
/// tree file *cleans back* to the blob, not that it equals what a checkout would
/// write, and under `eol=crlf` a hand-written all-LF file is both clean and not what
/// a checkout writes. The tool therefore clones only where no conversion applies,
/// and a converting repository accelerates nothing. That cost was weighed and
/// accepted rather than overlooked, and the documentation states it.
///
/// The `autocrlf-*` fixtures spell `core.autocrlf=true` the other ways git accepts it
/// (`yes`, `on`, `1`, `TRUE`, a key with no value), so they must clone nothing either.
///
/// So this asserts the decision holds. If a future change starts cloning converted
/// paths, that is a correctness regression and this test is what catches it — do not
/// "fix" it by moving the fixture back into ACCELERATED.
const NOT_ACCELERATED: &[&str] = &[
    "autocrlf",
    "autocrlf-yes",
    "autocrlf-on",
    "autocrlf-one",
    "autocrlf-upper",
    "autocrlf-valueless",
];

#[test]
fn the_fast_path_is_taken_where_the_fixture_exists_to_prove_it() {
    let runner = Runner::new("acceleration").expect("scratch workspace");
    if runner.tool().is_self_test() {
        println!(
            "self-test mode: both sides are real git, so there are no statistics to assert. \
             Set SPROUT_BIN to exercise this."
        );
        return;
    }

    let flags = flags::by_name("new-branch").expect("flag case");
    let mut measured = 0usize;
    let mut silent: Vec<&str> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for fixture in ACCELERATED {
        let template = runner.template(fixture).expect("fixture builder");
        if let Some(reason) = &template.skipped {
            println!("skipped fixture {fixture}: {reason}");
            continue;
        }
        let Outcome::Ran(result) = runner.run(&template, flags, None).expect("case run") else {
            continue;
        };
        assert!(
            result.differences.is_empty(),
            "{fixture} diverged while measuring acceleration: {:?}",
            result.differences
        );
        match &result.candidate.stats {
            None => silent.push(fixture),
            Some(stats) => {
                measured += 1;
                println!("{fixture}: {stats:?}");
                if !stats.supports_cloning() {
                    println!(
                        "{fixture}: this filesystem has no block cloning ({:?}); \
                         the fast path cannot be asserted here",
                        stats.fallback_reason
                    );
                } else if !stats.accelerated() {
                    failures.push(format!(
                        "{fixture}: nothing was cloned (cloned={}, skipped={}, reason={:?}); \
                         the output may be right but the fast path never ran",
                        stats.cloned, stats.skipped, stats.fallback_reason,
                    ));
                }
            }
        }
    }

    for fixture in NOT_ACCELERATED {
        let template = runner.template(fixture).expect("fixture builder");
        if let Some(reason) = &template.skipped {
            println!("skipped fixture {fixture}: {reason}");
            continue;
        }
        let Outcome::Ran(result) = runner.run(&template, flags, None).expect("case run") else {
            continue;
        };
        assert!(
            result.differences.is_empty(),
            "{fixture} diverged: {:?}",
            result.differences
        );
        match &result.candidate.stats {
            None => silent.push(fixture),
            Some(stats) => {
                measured += 1;
                println!("{fixture}: {stats:?}");
                if stats.accelerated() {
                    failures.push(format!(
                        "{fixture}: cloned {} paths, but a converting repository must clone \
                         none. Cloning a path a checkout would rewrite produces a worktree \
                         that differs from git's while `git status` stays clean in both. \
                         See NOTES-spec-disagreement.md, the §3.4 item.",
                        stats.cloned
                    ));
                }
            }
        }
    }

    // A binary that reports nothing anywhere has not been instrumented yet, which is
    // a different situation from one that reports zero clones. Say so loudly and let
    // the run continue; the assertion below arms itself as soon as the first
    // statistic appears, and `SPROUT_REQUIRE_STATS` forces it before then.
    let require = std::env::var_os("SPROUT_REQUIRE_STATS").is_some();
    if measured == 0 && !silent.is_empty() {
        println!(
            "NOT MEASURED: {} reported no statistics under SPROUT_STATS=1, so this run \
             proved correctness only and not that anything was cloned. Set \
             SPROUT_REQUIRE_STATS=1 to make that a failure.",
            runner.tool().describe()
        );
        assert!(
            !require,
            "the candidate reported no statistics and SPROUT_REQUIRE_STATS is set"
        );
        return;
    }
    for fixture in silent {
        failures.push(format!(
            "{fixture}: the candidate reported no statistics, but did for other fixtures"
        ));
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A worktree created on a filesystem without block cloning must still be correct.
/// Point `SPROUT_TEST_TMPDIR` at such a volume and the whole suite exercises the
/// fallback; this test states which mode the run is in so the log says so.
#[test]
fn the_run_reports_which_path_it_took() {
    let runner = Runner::new("acceleration-mode").expect("scratch workspace");
    println!("candidate: {}", runner.tool().describe());
    println!("scratch root: {}", runner.workspace().root().display());
    if let Tool::Sprout(bin) = runner.tool() {
        assert!(
            bin.is_file(),
            "SPROUT_BIN does not point at a file: {}",
            bin.display()
        );
    }
}
