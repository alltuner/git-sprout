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

### The disk figure needs a quiet volume

A free-space delta measures the whole volume, so anything else writing or deleting
gigabytes during a run lands in the number — a concurrent build or test suite can
produce a worktree that apparently consumed nothing, or negative disk. The harness
does not hide this:

- `settle_kb` gives up after `SETTLE_TIMEOUT_S` (default 20) if free space never
  stops moving, and the sample is recorded with `"settled": false`.
- Every side carries `unsettled_runs`, and `load_avg` (median, min, max of the
  one-minute load average at the end of each run).

**Read those two fields before quoting any disk number.** A row with
`unsettled_runs > 0`, or a load average that is not close to idle, was measured on a
busy machine and the medians should be re-measured before they reach a page.

## The Linux rows in CI

`bench/ci-linux.sh` is the entry point for `btrfs` and `ext4`. It re-execs itself
under `sudo`, installs `btrfs-progs` / `e2fsprogs` if they are missing, and writes its
own report:

```bash
# in a privileged container
docker run --rm --privileged -v "$PWD:/src" -w /src debian:stable-slim \
  bash bench/ci-linux.sh
# on a Linux CI runner
sudo -E bench/ci-linux.sh --out bench/results/linux.json
```

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
baseline-only run, so the first promotion is the first real measurement of the tool.
The `results.json` committed today is a baseline-only report placed by hand, so the
site and the README have something to render against before the tool exists; it says
so in `meta.status`, and every `*.sprout.*` figure on both pages reads `—`.

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
          "load_avg":       { "median": 0.9,   "min": ..., "max": ..., "samples": [...] },
          "unsettled_runs": 0,                 // runs whose disk figure is untrustworthy
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

`render-site-numbers.py` rewrites figures **in place, between explicit markers**. One
syntax, identical in HTML and Markdown, because HTML comments are inert in both:

```html
<!--bench:kernel.disk.git-->1814 MB<!--/bench-->
```

The opener names the key; the closer is always the literal `<!--/bench-->`. Whatever
sits between them is replaced. Nesting is not supported.

**Names are `scenario.metric.side`.** Units are not in the name — they live in the
`units` block of `bench.json`, and the rendered text carries the unit string.

### The complete catalogue — 30 names

| name | renders as |
| --- | --- |
| `kernel.time.git` / `kernel.time.sprout` | `11.13s` |
| `kernel.disk.git` / `kernel.disk.sprout` | `1814 MB` |
| `kernel.disk.git.round` | `1.8 GB` — derived |
| `kernel.disk.ratio` | `42x` — derived |
| `kernel.disk.saved` | `1.7 GB` — derived, git minus sprout |
| `kernel.files` | `95 056` |
| `kernel.bytes` | `1.5 GB` — working tree, excluding `.git` |
| `medium.time.git` / `.sprout`, `medium.disk.git` / `.sprout` | the 250 MB / 2000 files row |
| `cross.time.git` / `.sprout`, `cross.disk.git` / `.sprout` | the 188 MB source-behind row |
| `btrfs.time.git` / `.sprout`, `btrfs.disk.git` / `.sprout` | the btrfs row |
| `btrfs.du` | the verbatim `btrfs filesystem du` block |
| `ext4.time.git`, `ext4.disk.git` | the fallback row; it has no tool column |
| `fleet.disk.git` / `fleet.disk.sprout` | `88.6 GB` — derived, 50 worktrees |
| `chart.bar.git` / `chart.bar.sprout` | SVG bar geometry, derived from the kernel disk figures |
| `env.macos` | `Apple M2, 8 cores, macOS 26.6.1, git 2.55.0` |
| `env.linux` | `kernel 7.0.12-linuxkit, git 2.47.3, loopback btrfs and ext4` |

Four are not plain numbers:

- **`btrfs.du`** is a multi-line block, captured verbatim from the filesystem and
  HTML-escaped when it lands in an `.html` target, left raw in Markdown.
- **`chart.bar.git` / `chart.bar.sprout`** wrap the site's own `<rect>`. The renderer
  rewrites only the `width` attribute and leaves the classes and the rest of the
  markup alone, so the chart's appearance stays the site's business and its length
  stays the report's. Widths are a fraction of a 1000-unit viewBox, derived from the
  kernel disk figures, so the bars can never disagree with the table above them.
- **`env.macos` / `env.linux`** are machine descriptions, taken from the `machine`
  block of the scenarios each one actually measured.

The `no-match` scenario is measured for the worst-case budget in spec §9 and has no
name here on purpose: it is not a row on the page.

### The self-check, in both directions

Every marker on a page must have a value in the report, **and** every name in the
catalogue must appear on a page. A name the report cannot fill is a broken figure; a
name no page carries is a figure that quietly stopped being regenerated. Both fail the
run. This is the same check as:

```bash
grep -oh '<!--bench:[a-z0-9._-]*-->' docs/index.html README.md \
  | sed 's/<!--bench://;s/-->//' | sort -u
```

`./bench/render-site-numbers.py --check` runs it and writes nothing;
`--print-keys` dumps every name with its current value.

### Optional: region markers

A region asserts that everything inside it is a measurement, and catches a figure
typed straight into the prose — which the name check alone cannot see:

```html
<!--bench:region-->
  ... the numbers table, the hero figure, the chart ...
<!--/bench:region-->
```

Inside a region, span markers and comments are stripped and then two checks run: any
digit left in the visible text is an error, and any `rect`, `circle`, `line`,
`polygon`, `polyline` or `path` carrying digits outside a marker is an error. A shape
whose digits really are static opts out with a bare `data-bench-ignore` attribute.
Keep CSS, `<svg viewBox>` and prose that legitimately contains digits outside a
region.
