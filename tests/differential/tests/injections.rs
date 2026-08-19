// ABOUTME: Proves the comparison fails when it should: every deliberate corruption
// ABOUTME: of a finished worktree must be reported, by the comparator it belongs to.

use differential::case::{Outcome, Runner};
use differential::flags;
use differential::inject;

/// The flag case the injections are applied to: a full checkout on a new branch,
/// which fires every hook and writes every observable.
const FLAGS: &str = "new-branch";

/// The same fixture and flags with nothing injected must match, or the injection
/// results below would prove nothing about the injections.
#[test]
fn the_uninjected_case_matches() {
    let runner = Runner::new("inject-control").expect("scratch workspace");
    let template = runner.template("plain").expect("fixture builder");
    assert!(template.skipped.is_none(), "{:?}", template.skipped);
    let flags = flags::by_name(FLAGS).expect("flag case");
    match runner.run(&template, flags, None).expect("case run") {
        Outcome::Ran(result) => assert!(
            result.differences.is_empty(),
            "the control case diverged before any injection: {:?}",
            result.differences
        ),
        Outcome::NotApplicable(reason) => panic!("control case skipped: {reason}"),
    }
}

#[test]
fn every_injection_is_detected() {
    let runner = Runner::new("inject").expect("scratch workspace");
    let template = runner.template("plain").expect("fixture builder");
    assert!(template.skipped.is_none(), "{:?}", template.skipped);
    let flags = flags::by_name(FLAGS).expect("flag case");

    let mut undetected: Vec<String> = Vec::new();
    let mut misattributed: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut table: Vec<String> = Vec::new();

    for injection in inject::ALL {
        match runner
            .run(&template, flags, Some(*injection))
            .expect("case run")
        {
            Outcome::NotApplicable(reason) => {
                skipped.push(format!("{}: {reason}", injection.name()));
            }
            Outcome::Ran(result) => {
                if result.differences.is_empty() {
                    undetected.push(format!(
                        "{} ({}) produced no difference",
                        injection.name(),
                        injection.describe()
                    ));
                    continue;
                }
                let expected = injection.expected_area();
                let areas: Vec<&str> = {
                    let mut areas: Vec<&str> = result.differences.iter().map(|d| d.area).collect();
                    areas.dedup();
                    areas
                };
                if !areas.contains(&expected) {
                    misattributed.push(format!(
                        "{} should have been caught by `{expected}` but was reported as {areas:?}",
                        injection.name()
                    ));
                }
                runner.release(&result.case_dir);
                let reported = result
                    .differences
                    .iter()
                    .find(|d| d.area == expected)
                    .unwrap_or(&result.differences[0]);
                table.push(format!(
                    "  {:<30} {:<24} {}",
                    injection.name(),
                    reported.area,
                    truncate(&reported.detail, 92)
                ));
            }
        }
    }

    println!(
        "injections detected ({} of {}):",
        table.len(),
        inject::ALL.len()
    );
    for line in &table {
        println!("{line}");
    }
    for line in &skipped {
        println!("  not applicable: {line}");
    }

    assert!(
        undetected.is_empty(),
        "the harness failed to notice {} injected differences:\n  {}",
        undetected.len(),
        undetected.join("\n  ")
    );
    assert!(
        misattributed.is_empty(),
        "{} injections were caught by the wrong comparator:\n  {}",
        misattributed.len(),
        misattributed.join("\n  ")
    );
    assert!(
        skipped.is_empty(),
        "{} injections could not be applied to the plain fixture, so they went untested:\n  {}",
        skipped.len(),
        skipped.join("\n  ")
    );
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit).collect();
    format!("{head}…")
}
