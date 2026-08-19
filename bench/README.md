# bench — where every number on the site comes from

The site and the README must never carry a hand-typed figure. The scripts here
measure, write `results.json`, and rewrite the pages from it. If a number appears on
a page that no script produced, `render-site-numbers.py` is supposed to fail.

```
bench/
  run.sh                  the driver; `just bench`
  lib.sh                  timing, real-disk accounting, machine facts
  loopfs.sh               loopback btrfs / XFS / ext4 images for the Linux scenarios
  build-report.py         raw NDJSON -> bench.json
  render-site-numbers.py  bench.json -> the figures in docs/index.html and README.md
  results/bench.json      output of a local run (gitignored)
  results.json            the committed, release-time copy the pages render from
```

## Running it

```bash
just bench                        # every scenario
just bench kernel same-commit     # a subset
just bench --baseline-only        # git worktree add on both sides; no tool needed
just bench --fetch kernel         # download the kernel fixture (about 2 GB)
just bench --runs 5
just site-numbers                 # rewrite the pages from results.json
./bench/render-site-numbers.py --check       # validate markers, write nothing
./bench/render-site-numbers.py --print-keys  # every key and its current value
```

The tool under measurement is taken from `$SPROUT_BIN`, else
`./target/release/git-sprout`, else `git-sprout` on `PATH`. With none of those the
run degrades to `--baseline-only` rather than failing: the harness is fully
exercisable before the tool exists, and the report says so.

Fixtures and scratch live under `$BENCH_CACHE` (default
`~/.cache/git-sprout-bench`), never inside the repo. The kernel clone is kept
between runs; everything else is rebuilt and deleted.

## Scenarios

| id | what it measures |
| --- | --- |
| `kernel` | shallow clone of the Linux kernel — the headline row, and the only fixture that exercises case-colliding paths |
| `same-commit` | 250 MB / 2000 files, new worktree at the same commit |
| `cross-commit` | 188 MB / 3000 files, source checkout six paths behind the target |
| `no-match` | 250 MB / 2000 files where the target commit shares no blob with the source checkout — the worst case, budgeted at ≤30% overhead |
| `btrfs` | 188 MB / 1500 files on a loopback btrfs image; also captures `btrfs filesystem du` |
| `ext4` | the same tree on ext4, which has no block cloning, so the tool must fall back |

`btrfs` and `ext4` need Linux, root, and `mkfs.btrfs` / `mkfs.ext4`. Anywhere else
they are recorded with `"status": "skipped"` and a `skip_reason` naming what was
missing — they never fail the run and never carry a guessed number. The release-time
report is assembled from a macOS run and a Linux CI run:

```bash
./bench/run.sh --merge linux-ci-bench.json --promote
```

`--merge` folds in scenarios this machine skipped; each scenario keeps its own
`machine` block, so a reader can always tell which machine produced which row.

## Methodology

- **Wall clock** is measured around the `worktree add` process only, by a single
  wrapper process, so no shell or interpreter startup lands inside the measurement.
- **Real disk consumed** is a free-space delta on the volume holding the worktree,
  sampled until two readings agree within 2 MB, because APFS keeps accounting for a
  while after a large write. Logical size (`du`, `ls`) is deliberately not used: the
  whole point is that logical size does not change and allocation does.
- Each side runs one **discarded warm-up** before its timed runs, so whichever side
  goes first does not pay the cold page cache.
- Each side runs `--runs` times (default 3); the report carries median, min, max and
  every sample.
- After each run the new worktree's **first `git status --porcelain`** is timed, and
  its **tree oid** and dirty-path count recorded. The report's `comparison` block
  states whether the two sides produced the same tree oid and the same dirty count.
- Fixture content is random bytes, because a compressible fixture would understate
  what a real checkout allocates and flatter every disk figure on the page.

## Provenance and the `provisional` flag

Every report carries `"provisional": true` — at the top level and on every scenario —
until two things are true: the run measured a real `git-sprout` binary (not
`--baseline-only`), and it was invoked with `--differential-verified`, which the
release process passes only when the differential suite is green. Nothing here may be
presented as verified before then, and `render-site-numbers.py` stamps the page's
`meta.status` marker with the reason.

In a `--baseline-only` report the `sprout` side is a real measurement of
`git worktree add`, recorded with `"tool": "git worktree add"` so it cannot be
mistaken for the tool. The renderer refuses to put those figures in a tool column: it
writes `—` for every `*.sprout.*` key and says why in `meta.status`.

`--promote` copies the report to `bench/results.json` and refuses to do it for a
baseline-only run.

## JSON schema (`schema_version: 1`)

Sizes are binary: `MB` is 2²⁰ bytes, `GB` is 2³⁰.

```jsonc
{
  "schema_version": 1,
  "generated_at": "2026-08-19T11:30:28+00:00",  // ISO 8601, UTC
  "provisional": true,             // no figure here is verified while this is true
  "baseline_only": true,           // both sides were `git worktree add`
  "differential_verified": false,  // the differential suite was green for this build
  "runs_per_side": 3,
  "units": { "time_s": "...", "disk_mb": "...", ... },   // prose, per metric suffix
  "tool":    { "name": "git-sprout", "path": "...", "version": "..." },
  "machine": {
    "cpu": "Apple M2", "cores": 8, "os": "macOS 26.6.1",
    "kernel": "Darwin 25.6.0", "arch": "arm64",
    "git_version": "git version 2.55.0", "filesystem": "apfs",
    "scratch_path": "/Users/.../.cache/git-sprout-bench"
  },
  "scenarios": [
    {
      "id": "kernel",
      "provisional": true,
      "title": "Linux kernel shallow clone",   // never carries a number
      "status": "ok",                          // ok | skipped | failed
      "skip_reason": null,                     // why, when status is not ok
      "machine": { ... },                      // the machine that measured THIS row
      "fixture": {
        "tracked_files": 95056,
        "logical_bytes": 1518073744,           // working tree, excluding .git
        "source_commit": "<full oid>",
        "target_commitish": "HEAD",
        "filesystem": "apfs"
      },
      "sides": {
        "git": {
          "tool": "git worktree add",          // what actually ran
          "command": ["git", "worktree", "add", "-b", "...", "..."],
          "cwd": "/path/to/source/repo",
          "runs": 3,
          "time_s":         { "median": 11.58, "min": ..., "max": ..., "samples": [...] },
          "disk_mb":        { "median": 1805.0, "min": ..., "max": ..., "samples": [...] },
          "first_status_s": { "median": 0.35,  "min": ..., "max": ..., "samples": [...] },
          "tree_oid": "92b9cabb...",           // a list, if the runs disagreed
          "dirty_paths": 13
        },
        "sprout": { ... same shape ... }
      },
      "comparison": {
        "time_ratio": 2.12,        // git / sprout; null when the divisor is ~0
        "disk_ratio": 42.0,
        "tree_oid_match": true,
        "dirty_paths_match": true
      },
      "proof": {                   // btrfs only
        "label": "btrfs filesystem du -s",
        "text": "       Total   Exclusive  Set shared  Filename\n..."
      }
    }
  ]
}
```

Every metric's units are in its field name: `_s` is seconds, `_mb` is mebibytes,
`_bytes` is bytes. `time_ratio` and `disk_ratio` are dimensionless and are `null`
rather than a large number when the divisor is under 0.005.

## Marker convention — read this before editing the site or the README

`render-site-numbers.py` rewrites figures **in place, between explicit markers**. The
same convention works in HTML and in Markdown, because HTML comments are inert in
both.

### 1. Span markers — for a figure in the text

```html
<!--bench:kernel.git.disk_mb-->1805 MB<!--/bench-->
```

The opener names the key; the closer is always the literal `<!--/bench-->`. Whatever
sits between them is replaced. Nesting is not supported. Identical syntax in
Markdown:

```markdown
| **<!--bench:kernel.label-->…<!--/bench-->** | <!--bench:kernel.git.summary-->…<!--/bench--> |
```

### 2. Attribute markers — for SVG geometry

An element carrying `data-bench-<attribute>="<key>"` gets that attribute rewritten:

```html
<rect data-bench-width="kernel.git.disk_pct" width="100%" y="0" height="24"/>
```

The attribute is created if it is not already there. Use this for the bar chart, so
the bar widths come from the same JSON as the labels.

### 3. Region markers — the guard against a figure with no marker

```html
<!--bench:region-->
  ... the numbers table, the chart labels, the hero figure ...
<!--/bench:region-->
```

Inside a region, **every digit must come from a marker**. The script strips out span
markers, tags carrying `data-bench-*`, and comments, and then fails on any digit that
is left. Put the numbers table, the hero figure and the chart inside a region; keep
static geometry, CSS and prose that legitimately contains digits outside one.

### Key grammar

```
meta.<field>
proof.<scenario>
<scenario>.<field>
<scenario>.<side>.<metric>
<scenario>.ratio.<disk|time>
```

`<scenario>` is one of `kernel`, `same-commit`, `cross-commit`, `no-match`, `btrfs`,
`ext4`. `<side>` is `git` or `sprout`.

| key | renders as |
| --- | --- |
| `meta.status` | `Provisional, 2026-08-19: baseline-only run — …` |
| `meta.generated` | `2026-08-19` |
| `meta.runs` | `3` |
| `meta.tool_version` | `git-sprout 0.1.0` |
| `meta.machine` | `Apple M2, 8 cores, macOS 26.6.1, git 2.55.0, APFS` |
| `<sc>.title` | `Linux kernel shallow clone` |
| `<sc>.label` | `Linux kernel shallow clone — 95 056 files, 1.4 GB` |
| `<sc>.files` | `95 056` |
| `<sc>.size` | `1.4 GB` |
| `<sc>.machine` | the machine that measured this row |
| `<sc>.status` / `<sc>.skip_reason` | `skipped` / why |
| `<sc>.<side>.summary` | `11.58s · 1805 MB` |
| `<sc>.<side>.time_s` | `11.58s` |
| `<sc>.<side>.disk_mb` | `1805 MB` |
| `<sc>.<side>.first_status_s` | `0.35s` |
| `<sc>.<side>.tree_oid` | `92b9cabb…` |
| `<sc>.<side>.dirty_paths` | `13` |
| `<sc>.<side>.disk_pct` | `100.0%` — share of the larger side, for bar widths |
| `<sc>.<side>.time_pct` | `100.0%` |
| `<sc>.ratio.disk` | `42x` |
| `<sc>.ratio.time` | `2.1x` |
| `<sc>.oid_match` | `identical` |
| `proof.btrfs` | the verbatim `btrfs filesystem du` block |

A scenario that was skipped renders `not measured`; every `*.sprout.*` key in a
baseline-only report renders `—`. Run `--print-keys` for the live list.

### Required markers

The script fails if any of these is absent, so a figure cannot quietly stop being
regenerated:

**`docs/index.html`** — `meta.status`, `meta.generated`, `meta.runs`,
`kernel.label`, `kernel.files`, `kernel.machine`, `kernel.git.summary`,
`kernel.sprout.summary`, `kernel.git.disk_mb`, `kernel.sprout.disk_mb`,
`kernel.ratio.disk`, `same-commit.label`, `same-commit.git.summary`,
`same-commit.sprout.summary`, `cross-commit.label`, `cross-commit.git.summary`,
`cross-commit.sprout.summary`, `btrfs.label`, `btrfs.machine`, `btrfs.git.summary`,
`btrfs.sprout.summary`, `ext4.label`, `ext4.git.summary`, `ext4.sprout.summary`,
`proof.btrfs`.

**`README.md`** — `meta.status`, `kernel.label`, `kernel.machine`,
`kernel.git.summary`, `kernel.sprout.summary`, `kernel.ratio.disk`.

Any other key may be used freely; only the list above is mandatory. The lists live at
the top of `render-site-numbers.py`.
