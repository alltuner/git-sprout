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
            run::worktree_add(&self.workspace, &candidate, &self.tool, &argv)?;

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

/// Formats one diverged case for a failure report.
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
    failures.sort();
    assert!(
        failures.is_empty(),
        "{} of {} flag cases diverged for fixture {fixture}:{}",
        failures.len(),
        cases.len(),
        failures.join("")
    );
}

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
