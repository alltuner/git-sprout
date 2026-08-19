// ABOUTME: Turns two snapshots into a list of named differences, one line per real
// ABOUTME: divergence, so a failure says what differs rather than that something did.

use crate::index::IndexFacts;
use crate::snapshot::{AdminEntry, Snapshot, TreeEntry};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

/// How many differences of one kind are printed before the rest are counted. A
/// case-collision fixture can legitimately produce hundreds; the report has to stay
/// readable without hiding that they exist.
const MAX_PER_AREA: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    pub area: &'static str,
    pub detail: String,
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.area, self.detail)
    }
}

pub fn compare(control: &Snapshot, candidate: &Snapshot) -> Vec<Difference> {
    let mut out = Vec::new();
    let mut d = Collector { out: &mut out };

    d.text("stdout", &control.stdout, &candidate.stdout);
    d.text("stderr", &control.stderr, &candidate.stderr);
    if control.status != candidate.status {
        d.push("exit status", format!("{} vs {}", control.status, candidate.status));
    }
    d.sequence("hooks fired", &control.hooks, &candidate.hooks);

    if control.worktree_exists != candidate.worktree_exists {
        d.push(
            "worktree",
            format!(
                "created by control: {}, by candidate: {}",
                control.worktree_exists, candidate.worktree_exists
            ),
        );
    }

    d.map("working tree", &control.tree, &candidate.tree, TreeEntry::describe);
    d.set("git status --porcelain", &control.porcelain, &candidate.porcelain);
    if control.head != candidate.head {
        d.push("HEAD", format!("{} vs {}", control.head, candidate.head));
    }
    if control.head_reflog_entry != candidate.head_reflog_entry {
        d.push(
            "HEAD@{0}",
            format!("{} vs {}", control.head_reflog_entry, candidate.head_reflog_entry),
        );
    }
    d.sequence("reflog", &control.reflog, &candidate.reflog);
    d.set("refs", &control.refs, &candidate.refs);
    d.sequence("worktree list", &control.worktree_list, &candidate.worktree_list);
    d.map("worktree admin dir", &control.admin, &candidate.admin, AdminEntry::describe);
    d.sequence("ls-files --stage", &control.ls_files_stage, &candidate.ls_files_stage);
    d.sequence("ls-files -v", &control.ls_files_flags, &candidate.ls_files_flags);
    compare_index(&mut d, "index", &control.index, &candidate.index);
    if control.shared_index_count != candidate.shared_index_count {
        d.push(
            "shared index",
            format!(
                "{} shared index files vs {}",
                control.shared_index_count, candidate.shared_index_count
            ),
        );
    }
    match (&control.shared_index, &candidate.shared_index) {
        (Some(a), Some(b)) => compare_index(&mut d, "shared index", a, b),
        (Some(_), None) => d.push("shared index", "only control has one".to_string()),
        (None, Some(_)) => d.push("shared index", "only the candidate has one".to_string()),
        (None, None) => {}
    }

    out
}

fn compare_index(
    d: &mut Collector<'_>,
    area: &'static str,
    control: &IndexFacts,
    candidate: &IndexFacts,
) {
    if control.parse_error != candidate.parse_error {
        d.push(
            area,
            format!("parse result {:?} vs {:?}", control.parse_error, candidate.parse_error),
        );
    }
    if control.version != candidate.version {
        d.push(area, format!("version {} vs {}", control.version, candidate.version));
    }
    if control.declared_entries != candidate.declared_entries {
        d.push(
            area,
            format!(
                "entry count {} vs {}",
                control.declared_entries, candidate.declared_entries
            ),
        );
    }

    let control_entries: BTreeMap<_, _> =
        control.entries.iter().map(|e| ((e.path.clone(), e.stage), e)).collect();
    let candidate_entries: BTreeMap<_, _> =
        candidate.entries.iter().map(|e| ((e.path.clone(), e.stage), e)).collect();
    let mut shown = 0usize;
    let mut extra = 0usize;
    for key in keys(&control_entries, &candidate_entries) {
        let detail = match (control_entries.get(&key), candidate_entries.get(&key)) {
            (Some(a), Some(b)) if a == b => continue,
            (Some(a), Some(b)) => {
                let mut fields = Vec::new();
                if a.oid != b.oid {
                    fields.push(format!("oid {} vs {}", a.oid, b.oid));
                }
                if a.mode != b.mode {
                    fields.push(format!("mode {:06o} vs {:06o}", a.mode, b.mode));
                }
                if (a.assume_valid, a.skip_worktree, a.intent_to_add, a.extended)
                    != (b.assume_valid, b.skip_worktree, b.intent_to_add, b.extended)
                {
                    fields.push(format!(
                        "flags assume_valid={}/{} skip_worktree={}/{} intent_to_add={}/{} extended={}/{}",
                        a.assume_valid, b.assume_valid,
                        a.skip_worktree, b.skip_worktree,
                        a.intent_to_add, b.intent_to_add,
                        a.extended, b.extended,
                    ));
                }
                format!("entry {} stage {}: {}", key.0, key.1, fields.join(", "))
            }
            (Some(_), None) => format!("entry {} stage {} only in control", key.0, key.1),
            (None, Some(_)) => format!("entry {} stage {} only in candidate", key.0, key.1),
            (None, None) => continue,
        };
        if shown < MAX_PER_AREA {
            d.push(area, detail);
            shown += 1;
        } else {
            extra += 1;
        }
    }
    if extra > 0 {
        d.push(area, format!("and {extra} further entry differences"));
    }

    let control_ext: BTreeMap<_, _> =
        control.extensions.iter().map(|e| (e.signature.clone(), e)).collect();
    let candidate_ext: BTreeMap<_, _> =
        candidate.extensions.iter().map(|e| (e.signature.clone(), e)).collect();
    for key in keys(&control_ext, &candidate_ext) {
        match (control_ext.get(&key), candidate_ext.get(&key)) {
            (Some(a), Some(b)) => {
                if a.digest.is_some() && a.digest != b.digest {
                    d.push(
                        area,
                        format!(
                            "extension {key} payload differs ({} vs {} bytes)",
                            a.size, b.size
                        ),
                    );
                }
            }
            (Some(_), None) => d.push(area, format!("extension {key} only in control")),
            (None, Some(_)) => d.push(area, format!("extension {key} only in candidate")),
            (None, None) => {}
        }
    }
}

struct Collector<'a> {
    out: &'a mut Vec<Difference>,
}

impl Collector<'_> {
    fn push(&mut self, area: &'static str, detail: String) {
        self.out.push(Difference { area, detail });
    }

    fn text(&mut self, area: &'static str, control: &str, candidate: &str) {
        if control == candidate {
            return;
        }
        self.push(area, format!("control {:?}, candidate {:?}", control, candidate));
    }

    /// Ordered comparison: position matters, as it does for the hook sequence.
    fn sequence(&mut self, area: &'static str, control: &[String], candidate: &[String]) {
        if control == candidate {
            return;
        }
        let mut shown = 0usize;
        let mut extra = 0usize;
        for i in 0..control.len().max(candidate.len()) {
            let a = control.get(i);
            let b = candidate.get(i);
            if a == b {
                continue;
            }
            let detail = match (a, b) {
                (Some(a), Some(b)) => format!("line {i}: control {a:?}, candidate {b:?}"),
                (Some(a), None) => format!("line {i}: only in control: {a:?}"),
                (None, Some(b)) => format!("line {i}: only in candidate: {b:?}"),
                (None, None) => continue,
            };
            if shown < MAX_PER_AREA {
                self.push(area, detail);
                shown += 1;
            } else {
                extra += 1;
            }
        }
        if extra > 0 {
            self.push(area, format!("and {extra} further differing lines"));
        }
    }

    /// Unordered comparison for sets that are already sorted by the producer.
    fn set(&mut self, area: &'static str, control: &[String], candidate: &[String]) {
        let a: BTreeSet<&String> = control.iter().collect();
        let b: BTreeSet<&String> = candidate.iter().collect();
        let mut shown = 0usize;
        let mut extra = 0usize;
        for value in a.symmetric_difference(&b) {
            let side = if a.contains(value) { "control" } else { "candidate" };
            if shown < MAX_PER_AREA {
                self.push(area, format!("only in {side}: {value:?}"));
                shown += 1;
            } else {
                extra += 1;
            }
        }
        if extra > 0 {
            self.push(area, format!("and {extra} further entries on one side only"));
        }
    }

    fn map<V, F>(
        &mut self,
        area: &'static str,
        control: &BTreeMap<String, V>,
        candidate: &BTreeMap<String, V>,
        describe: F,
    ) where
        V: PartialEq,
        F: Fn(&V) -> String,
    {
        let mut shown = 0usize;
        let mut extra = 0usize;
        for key in keys(control, candidate) {
            let detail = match (control.get(&key), candidate.get(&key)) {
                (Some(a), Some(b)) if a == b => continue,
                (Some(a), Some(b)) => {
                    format!("{key}: control {}, candidate {}", describe(a), describe(b))
                }
                (Some(a), None) => format!("{key}: only in control ({})", describe(a)),
                (None, Some(b)) => format!("{key}: only in candidate ({})", describe(b)),
                (None, None) => continue,
            };
            if shown < MAX_PER_AREA {
                self.push(area, detail);
                shown += 1;
            } else {
                extra += 1;
            }
        }
        if extra > 0 {
            self.push(area, format!("and {extra} further paths differ"));
        }
    }
}

fn keys<K: Ord + Clone, V>(a: &BTreeMap<K, V>, b: &BTreeMap<K, V>) -> Vec<K> {
    let mut set: BTreeSet<K> = a.keys().cloned().collect();
    set.extend(b.keys().cloned());
    set.into_iter().collect()
}
