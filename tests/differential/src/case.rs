// ABOUTME: Runs one fixture through one flag case on both sides and reports whether
// ABOUTME: the two results are indistinguishable, plus the whole-fixture driver.

use crate::compare::{self, Difference};
use crate::env::Workspace;
use crate::files;
use crate::fixtures::{self, Built};
use crate::flags::{FlagCase, Setup};
use crate::inject::{Applied, Injection};
use crate::run::{self, Side, Tool};
use crate::snapshot::{self, Snapshot};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// A fixture repository built once and copied per case.
pub struct Template {
    pub name: String,
    pub repo: PathBuf,
    pub skipped: Option<String>,
}

/// Both sides of one comparison, kept so a test can assert on the content as well
/// as on equality.
pub struct CaseResult {
    pub differences: Vec<Difference>,
    pub control: Snapshot,
    pub candidate: Snapshot,
    pub case_dir: PathBuf,
}

pub enum Outcome {
    Ran(Box<CaseResult>),
    /// The injection this case asked for does not apply to this fixture.
    NotApplicable(String),
}

pub struct Runner {
    workspace: Workspace,
    tool: Tool,
}

impl Runner {
    pub fn new(label: &str) -> std::io::Result<Runner> {
        Ok(Runner {
            workspace: Workspace::create(label)?,
            tool: Tool::candidate(),
        })
    }

    pub fn tool(&self) -> &Tool {
        &self.tool
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Discards a finished case's scratch directory. Callers keep the directories of
    /// cases they still have something to say about.
    pub fn release(&self, case_dir: &std::path::Path) {
        self.workspace.release_case(case_dir);
    }

    /// A template built from the seeded random generator rather than a named builder.
    pub fn random_template(&self, seed: u64) -> std::io::Result<Template> {
        let name = format!("random-{seed}");
        let repo = self
            .workspace
            .root()
            .join("templates")
            .join(&name)
            .join("repo");
        let _ = crate::env::remove_tree(&repo);
        let skipped = match fixtures::build_random(&self.workspace, &repo, seed)? {
            Built::Ok => None,
            Built::Skipped(reason) => Some(reason),
        };
        Ok(Template {
            name,
            repo,
            skipped,
        })
    }

    pub fn template(&self, fixture: &str) -> std::io::Result<Template> {
        let repo = self
            .workspace
            .root()
            .join("templates")
            .join(fixture)
            .join("repo");
        let _ = crate::env::remove_tree(&repo);
        let skipped = match fixtures::build(&self.workspace, fixture, &repo)? {
            Built::Ok => None,
            Built::Skipped(reason) => Some(reason),
        };
        Ok(Template {
            name: fixture.to_string(),
            repo,
            skipped,
        })
    }

    /// Runs one comparison. `injection` corrupts the candidate side on purpose and
    /// is `None` for every real compatibility case. The case directory is left on
    /// disk for the caller to inspect and release with [`Runner::release`].
    pub fn run(
        &self,
        template: &Template,
        flags: &FlagCase,
        injection: Option<Injection>,
    ) -> std::io::Result<Outcome> {
        self.run_against(template, flags, injection, &self.tool)
    }

    /// Runs one case with an explicitly chosen candidate side.
    ///
    /// Passing `Tool::Git` puts real git on both sides, which is how the suite asks
    /// whether git agrees with *itself* for a given case rather than whether the tool
    /// agrees with git.
    pub fn run_against(
        &self,
        template: &Template,
        flags: &FlagCase,
        injection: Option<Injection>,
        candidate_tool: &Tool,
    ) -> std::io::Result<Outcome> {
        let case_name = match injection {
            Some(i) => format!("{}-{}-{}", template.name, flags.name, i.name()),
            None => format!("{}-{}", template.name, flags.name),
        };
        let case_dir = self.workspace.case_dir(&case_name)?;

        let control = Side::new(&case_dir, "control", "a");
        let candidate = Side::new(&case_dir, "candidate", "b");
        for side in [&control, &candidate] {
            std::fs::create_dir_all(&side.root)?;
            files::copy_tree(&template.repo, &side.repo)?;
            fixtures::refresh_index(&self.workspace, &side.repo);
            run::install_hooks(&side.repo, &side.hook_log)?;
            if flags.setup == Setup::OccupyDestination {
                std::fs::create_dir_all(side.root.join(flags.dest))?;
            }
        }

        let argv = flags.argv();
        let control_output = run::worktree_add(&self.workspace, &control, &Tool::Git, &argv)?;
        let mut candidate_output =
            run::worktree_add(&self.workspace, &candidate, candidate_tool, &argv)?;

        if let Some(injection) = injection {
            match injection.apply(&self.workspace, &candidate, &mut candidate_output)? {
                Applied::Yes => {}
                Applied::NotApplicable(reason) => {
                    self.workspace.release_case(&case_dir);
                    return Ok(Outcome::NotApplicable(reason));
                }
            }
        }

        let object_format = snapshot::object_format(&self.workspace, &control.repo);
        let control_snapshot =
            snapshot::capture(&self.workspace, &control, control_output, &object_format)?;
        let candidate_snapshot = snapshot::capture(
            &self.workspace,
            &candidate,
            candidate_output,
            &object_format,
        )?;

        let differences = compare::compare(&control_snapshot, &candidate_snapshot);
        Ok(Outcome::Ran(Box::new(CaseResult {
            differences,
            control: control_snapshot,
            candidate: candidate_snapshot,
            case_dir,
        })))
    }
}

/// Formats one diverged case for a failure report. The caller must have kept the
/// case directory, since the report names it.
pub fn report(fixture: &str, flags: &FlagCase, tool: &Tool, result: &CaseResult) -> String {
    let mut block = format!(
        "\n{fixture} / {} ({})\n  argv: add {}\n  left behind: {}\n",
        flags.name,
        tool.describe(),
        flags.argv().join(" "),
        result.case_dir.display()
    );
    for difference in &result.differences {
        block.push_str(&format!("  {difference}\n"));
    }
    block
}

/// Runs every flag case in `cases` against one fixture and panics with a full
/// report if any of them diverged. Collecting all failures first is deliberate: the
/// first divergence is rarely the most informative one.
///
/// Cases run on a small pool of their own threads. Almost all of the wall clock is
/// spent waiting on git subprocesses, so the pool is deliberately wider than the
/// core count; `SPROUT_CASE_THREADS` overrides it.
pub fn check_fixture(fixture: &str, cases: &[&FlagCase]) {
    let runner = Runner::new(fixture).expect("scratch workspace");
    let template = runner.template(fixture).expect("fixture builder");
    if let Some(reason) = &template.skipped {
        println!("skipped fixture {fixture}: {reason}");
        return;
    }

    let next = AtomicUsize::new(0);
    let failures = Mutex::new(Vec::<String>::new());
    let failed_cases = Mutex::new(Vec::<&FlagCase>::new());
    let unstable = Mutex::new(Vec::<String>::new());

    // One verdict per fixture per run, established before any case is judged.
    //
    // Asking "is git deterministic here?" once, with enough repetitions to mean
    // something, beats asking it per divergence with few. A per-divergence probe has to
    // be cheap, so it is short, so it is weak — and it is weakest exactly where it
    // matters, on a race biased towards agreement: if git agrees nine times in ten,
    // eight consecutive agreements happen about two in five times, and every one of
    // those reports a platform race as a parity bug. Paying the cost once buys a
    // sample large enough to trust, and turns the answer into a single reported fact
    // about the filesystem rather than a judgement that can differ between cases that
    // are physically identical.
    if collision_sensitive(fixture) {
        if let Some(probe) = cases.first() {
            if let Some(evidence) = control_is_unstable(&runner, &template, probe) {
                unstable.lock().expect("unstable list").push(evidence);
            }
        }
    }
    let workers = case_threads().min(cases.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::SeqCst);
                let Some(flags) = cases.get(index) else {
                    return;
                };
                match runner.run(&template, flags, None).expect("case run") {
                    Outcome::NotApplicable(_) => {}
                    Outcome::Ran(result) => {
                        if result.differences.is_empty() {
                            runner.release(&result.case_dir);
                        } else {
                            runner.workspace().retain();
                            failed_cases.lock().expect("failed cases").push(flags);
                            failures.lock().expect("failure list").push(report(
                                fixture,
                                flags,
                                runner.tool(),
                                &result,
                            ));
                        }
                    }
                }
            });
        }
    });

    let mut failures = failures.into_inner().expect("failure list");
    let mut unstable = unstable.into_inner().expect("unstable list");

    // Evidence pooled across the fixture rather than judged case by case. Once git has
    // been caught disagreeing with itself anywhere in this fixture, its collision
    // resolution is nondeterministic on this filesystem *for this fixture*, and that
    // fact does not vary from one flag case to the next. Per-case probing cannot
    // establish it reliably: a pair that flips rarely will agree with itself eight
    // times in a row often enough to matter, and each such case is then reported as a
    // parity failure. One confirmed observation is worth more than eight unconfirmed
    // agreements, and this is where the asymmetry finally gets applied properly.
    // A case that actually diverged is the one whose determinism is in question, and
    // it is a far better subject than the arbitrary case picked up front: the opening
    // probe can report "deterministic" simply because the case it happened to choose
    // flips less often than the ones that failed. So if anything failed, ask again
    // using a case that failed.
    let failed_cases = failed_cases.into_inner().expect("failed cases");
    if unstable.is_empty() && collision_sensitive(fixture) {
        if let Some(culprit) = failed_cases.first() {
            if let Some(evidence) = control_is_unstable(&runner, &template, culprit) {
                unstable.push(evidence);
            }
        }
    }

    if !unstable.is_empty() && !failures.is_empty() {
        unstable.push(format!(
            "{} further case(s) after git was caught disagreeing with itself",
            failures.len()
        ));
        failures.clear();
    }
    unstable.sort();
    if !unstable.is_empty() {
        // Printed on every run that uses it, so a green run never quietly reports a
        // full-strength pass it did not earn.
        println!(
            "fixture {fixture}: {} of {} flag cases compared shape-only because git \
             disagreed with itself on them: {}\n  {UNSTABLE_NOTE}",
            unstable.len(),
            cases.len(),
            unstable.join(", ")
        );
    }

    failures.sort();
    assert!(
        failures.is_empty(),
        "{} of {} flag cases diverged for fixture {fixture}:{}",
        failures.len(),
        cases.len(),
        failures.join("")
    );
}

/// How many times the control is reproduced before its agreement is believed.
///
/// The evidence here is asymmetric, and the whole rule turns on it. **One disagreement
/// proves nondeterminism outright**; nothing more is needed. **One agreement proves
/// nothing at all**, because a subject that picks a winner at random will agree with
/// itself about half the time by chance. A rule that concluded "deterministic" from a
/// single agreeing run would report a legitimate platform race as a parity failure
/// roughly half the time it fired.
///
/// So the probe stops the moment git disagrees, and only concludes the divergence is
/// real after this many consecutive agreements. For a coin-flip subject that is a
/// false accusation under half a percent of the time. The cost is paid only on a case
/// that has already diverged, and on a platform where git really is deterministic that
/// case is a genuine bug worth eight runs of confidence.
const CONTROL_SAMPLES: usize = 25;

/// Runs one case with real git on both sides until git contradicts itself, and reports
/// the contradiction if it comes.
///
/// Returns `None` when git reproduced itself every time, which is the only outcome that
/// licenses holding the tool to a specific collision winner.
fn control_is_unstable(runner: &Runner, template: &Template, probe: &FlagCase) -> Option<String> {
    for attempt in 1..=CONTROL_SAMPLES {
        match runner.run_against(template, probe, None, &Tool::Git) {
            Ok(Outcome::Ran(result)) => {
                let differences = result.differences.len();
                runner.release(&result.case_dir);
                if differences > 0 {
                    return Some(format!(
                        "git contradicted itself on `{}` at attempt {attempt} of \
                         {CONTROL_SAMPLES}, in {differences} place(s)",
                        probe.name
                    ));
                }
            }
            // An unusable probe must not excuse a divergence.
            Ok(Outcome::NotApplicable(_)) | Err(_) => return None,
        }
    }
    None
}

/// Fixtures whose subject is case folding, and the only ones allowed to fall back to a
/// shape comparison when git turns out to be unstable.
///
/// Deliberately a short explicit list rather than a general comparison mode: "equal up
/// to which member of a collision group won" is a powerful escape hatch and it must be
/// impossible to reach it from a fixture that is not about case folding.
fn collision_sensitive(fixture: &str) -> bool {
    matches!(fixture, "case-collision")
}

const UNSTABLE_NOTE: &str = "`git worktree add` resolved the collision differently \
     between two runs of itself on this filesystem, so there is no single winner to \
     hold the tool to. Everything else about those cases was still compared.";

fn case_threads() -> usize {
    std::env::var("SPROUT_CASE_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(4)
}

/// The flag cases the per-fixture tests run. The whole matrix by default;
/// `SPROUT_FLAG_MATRIX=core` narrows it to the short list, for platforms where
/// process spawning is slow enough that the full cross product is not worth its
/// wall clock on every commit.
pub fn all_cases() -> Vec<&'static FlagCase> {
    match std::env::var("SPROUT_FLAG_MATRIX").as_deref() {
        Ok("core") => core_cases(),
        _ => crate::flags::ALL.iter().collect(),
    }
}

/// The short flag matrix, for fixtures where the full one only repeats what the
/// plain repository already proved.
pub fn core_cases() -> Vec<&'static FlagCase> {
    crate::flags::CORE
        .iter()
        .filter_map(|n| crate::flags::by_name(n))
        .collect()
}
