// ABOUTME: Property test: random small repositories crossed with random flag cases,
// ABOUTME: always asserting that the candidate's output is the same as git's.

use differential::case::{self, Outcome, Runner};

/// Seed of the first case. Fixed so a failing run is reproducible; override with
/// `SPROUT_PROPERTY_SEED` to explore further.
const DEFAULT_SEED: u64 = 20_240_101;
const DEFAULT_CASES: u64 = 24;

#[test]
fn random_repositories_and_flags_match_git() {
    let seed = number("SPROUT_PROPERTY_SEED", DEFAULT_SEED);
    let cases = number("SPROUT_PROPERTY_CASES", DEFAULT_CASES);
    let runner = Runner::new("property").expect("scratch workspace");
    let all = case::all_cases();

    let mut failures: Vec<String> = Vec::new();
    for step in 0..cases {
        let case_seed = seed.wrapping_add(step);
        let template = runner.random_template(case_seed).expect("random fixture");
        if let Some(reason) = &template.skipped {
            println!("skipped seed {case_seed}: {reason}");
            continue;
        }
        // The flag case is drawn from the same seed, so the pairing is reproducible.
        let flags = all[(mix(case_seed) % all.len() as u64) as usize];
        match runner.run(&template, flags, None).expect("case run") {
            Outcome::NotApplicable(_) => {}
            Outcome::Ran(result) => {
                if !result.differences.is_empty() {
                    failures.push(format!(
                        "seed {case_seed}{}",
                        case::report(&template.name, flags, runner.tool(), &result)
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {cases} random cases diverged (rerun with SPROUT_PROPERTY_SEED):{}",
        failures.len(),
        failures.join("")
    );
}

fn number(key: &str, fallback: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(fallback)
}

/// A cheap avalanche so consecutive seeds do not pick consecutive flag cases.
fn mix(seed: u64) -> u64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x
}
