// ABOUTME: Asserts the acceleration actually happened on the fixtures built for it.
// ABOUTME: A correctness-only test passes trivially when nothing was cloned at all.

use differential::case::{Outcome, Runner};
use differential::flags;
use differential::run::Tool;

/// Fixtures whose whole point is that cloning fires. `autocrlf` is the one that
/// proves index-based verification works: every text file's working-tree bytes
/// differ from its blob, so a hash-against-blob rule would accelerate nothing.
const ACCELERATED: &[&str] = &["text-heavy", "autocrlf"];

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
                if !stats.accelerated() {
                    failures.push(format!(
                        "{fixture}: nothing was cloned (cloned={}, fell_back={}, reason={:?}); \
                         the output may be right but the fast path never ran",
                        stats.cloned, stats.fell_back, stats.fallback_reason
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
