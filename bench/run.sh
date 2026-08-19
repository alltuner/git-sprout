#!/usr/bin/env bash
# ABOUTME: Benchmark driver for git-sprout: measures wall clock and real disk consumed
# ABOUTME: for `git worktree add` against `git sprout add` and writes results/bench.json.
set -euo pipefail

BENCH_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$BENCH_DIR/.." && pwd)"
. "$BENCH_DIR/lib.sh"
. "$BENCH_DIR/loopfs.sh"

ALL_SCENARIOS="kernel same-commit cross-commit no-match btrfs ext4"
RUNS="${BENCH_RUNS:-3}"
BASELINE_ONLY=0
FETCH="${BENCH_FETCH:-0}"
DIFFERENTIAL_VERIFIED=0
OUT="$BENCH_DIR/results/bench.json"
CACHE="${BENCH_CACHE:-$HOME/.cache/git-sprout-bench}"
SCRATCH="$CACHE/scratch"
KERNEL_URL="${BENCH_KERNEL_URL:-https://github.com/torvalds/linux.git}"
SELECTED=""
MERGE_ARGS=()

usage() {
  cat <<'USAGE'
usage: bench/run.sh [options] [scenario ...]

  --baseline-only        run `git worktree add` on both sides; no tool required
  --runs N               timed runs per side (default 3)
  --fetch                download the kernel fixture if it is missing
  --differential-verified
                         the differential suite is green for this build, so the
                         emitted numbers are not marked provisional
  --out PATH             where to write the report (default bench/results/bench.json)
  --promote              also copy the report to bench/results.json, the committed
                         release-time copy the site renders from
  --merge PATH           fold scenarios this run skipped in from another machine's
                         report (the Linux CI run supplies btrfs and ext4)
  --list                 list scenario ids and exit

scenarios: kernel same-commit cross-commit no-match btrfs ext4
USAGE
}

PROMOTE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --baseline-only) BASELINE_ONLY=1 ;;
    --runs) RUNS="$2"; shift ;;
    --fetch) FETCH=1 ;;
    --differential-verified) DIFFERENTIAL_VERIFIED=1 ;;
    --out) OUT="$2"; shift ;;
    --promote) PROMOTE=1 ;;
    --merge) MERGE_ARGS+=(--merge "$2"); shift ;;
    --list) echo "$ALL_SCENARIOS" | tr ' ' '\n'; exit 0 ;;
    -h|--help) usage; exit 0 ;;
    -*) die "unknown option $1" ;;
    *) SELECTED="$SELECTED $1" ;;
  esac
  shift
done
[ -n "$SELECTED" ] || SELECTED="$ALL_SCENARIOS"

for want in $SELECTED; do
  case " $ALL_SCENARIOS " in *" $want "*) ;; *) die "unknown scenario $want" ;; esac
done

# --- the tool under measurement ------------------------------------------------

SPROUT_BIN="${SPROUT_BIN:-}"
if [ -z "$SPROUT_BIN" ]; then
  if [ -x "$REPO_ROOT/target/release/git-sprout" ]; then
    SPROUT_BIN="$REPO_ROOT/target/release/git-sprout"
  elif command -v git-sprout >/dev/null; then
    SPROUT_BIN="$(command -v git-sprout)"
  fi
fi
SPROUT_VERSION=""
if [ -n "$SPROUT_BIN" ] && [ -x "$SPROUT_BIN" ]; then
  SPROUT_VERSION="$("$SPROUT_BIN" --version 2>/dev/null || echo unknown)"
else
  if [ "$BASELINE_ONLY" = 0 ]; then
    note "no git-sprout binary found; falling back to --baseline-only"
    BASELINE_ONLY=1
  fi
  SPROUT_BIN=""
fi

PROVISIONAL=1
[ "$DIFFERENTIAL_VERIFIED" = 1 ] && [ "$BASELINE_ONLY" = 0 ] && PROVISIONAL=0

mkdir -p "$CACHE" "$SCRATCH" "$(dirname "$OUT")"
BENCH_RAW="$(mktemp "${TMPDIR:-/tmp}/bench-raw.XXXXXX")"
export BENCH_RAW
trap 'rm -f "$BENCH_RAW"' EXIT

git_c() { local repo="$1"; shift; git -C "$repo" -c user.email=bench@example.invalid -c user.name=bench "$@"; }
argv_json() { python3 -c 'import json, sys; print(json.dumps(sys.argv[1:]))' "$@"; }

record kind=meta \
  schema_version:=1 \
  provisional:="$([ "$PROVISIONAL" = 1 ] && echo true || echo false)" \
  baseline_only:="$([ "$BASELINE_ONLY" = 1 ] && echo true || echo false)" \
  differential_verified:="$([ "$DIFFERENTIAL_VERIFIED" = 1 ] && echo true || echo false)" \
  runs_per_side:="$RUNS" \
  tool_path="$SPROUT_BIN" \
  tool_version="$SPROUT_VERSION" \
  cpu="$(cpu_model)" \
  cores:="$(cpu_cores)" \
  os="$(os_name)" \
  kernel="$(uname -sr)" \
  arch="$(uname -m)" \
  git_version="$(git --version)" \
  filesystem="$(fs_type "$CACHE")" \
  scratch_path="$CACHE"

# --- measurement ---------------------------------------------------------------

# Set by each scenario before calling measure_sides.
SC_REPO=""; SC_VOL=""; SC_DESTDIR=""; SC_COMMITISH=""

build_argv() { # side dest branch
  local side="$1" dest="$2" branch="$3"
  if [ "$side" = git ] || [ "$BASELINE_ONLY" = 1 ]; then
    SIDE_ARGV=(git worktree add -b "$branch" "$dest")
    SIDE_TOOL="git worktree add"
  else
    SIDE_ARGV=("$SPROUT_BIN" add -b "$branch" "$dest")
    SIDE_TOOL="git sprout add"
  fi
  [ -n "$SC_COMMITISH" ] && SIDE_ARGV=("${SIDE_ARGV[@]}" "$SC_COMMITISH")
  return 0
}

drop_worktree() { # repo dest branch
  git -C "$1" worktree remove --force "$2" >/dev/null 2>&1 || rm -rf "$2"
  git -C "$1" branch -D "$3" >/dev/null 2>&1 || true
  git -C "$1" worktree prune >/dev/null 2>&1 || true
}

measure_sides() { # scenario-id
  local sc="$1" side run dest branch status_s tree_oid dirty
  for side in git sprout; do
    build_argv "$side" "$SC_DESTDIR/wt-$sc-$side-1" "bench-$sc-$side-1"
    record kind=command scenario="$sc" side="$side" tool="$SIDE_TOOL" \
      cwd="$SC_REPO" argv:="$(argv_json "${SIDE_ARGV[@]}")"
    # One discarded run first: whichever side went first would otherwise pay the
    # cold page cache and look slower than it is.
    build_argv "$side" "$SC_DESTDIR/wt-$sc-$side-warmup" "bench-$sc-$side-warmup"
    rm -rf "$SC_DESTDIR/wt-$sc-$side-warmup"
    ( cd "$SC_REPO" && "${SIDE_ARGV[@]}" >/dev/null 2>&1 ) || true
    drop_worktree "$SC_REPO" "$SC_DESTDIR/wt-$sc-$side-warmup" "bench-$sc-$side-warmup"
    for run in $(seq 1 "$RUNS"); do
      dest="$SC_DESTDIR/wt-$sc-$side-$run"
      branch="bench-$sc-$side-$run"
      rm -rf "$dest"
      build_argv "$side" "$dest" "$branch"
      note "$sc/$side run $run: ${SIDE_ARGV[*]}"
      if ! measure "$SC_VOL" "$SC_REPO" "${SIDE_ARGV[@]}"; then
        record kind=failure scenario="$sc" side="$side" \
          reason="the measured command exited non-zero"
        drop_worktree "$SC_REPO" "$dest" "$branch"
        return 1
      fi
      status_s="$(time_cmd "$dest" git status --porcelain)"
      tree_oid="$(git -C "$dest" rev-parse 'HEAD^{tree}')"
      dirty="$(git -C "$dest" status --porcelain | wc -l | tr -d ' ')"
      record kind=sample scenario="$sc" side="$side" run:="$run" \
        time_s:="$MEASURE_TIME_S" disk_mb:="$MEASURE_DISK_MB" \
        first_status_s:="$status_s" tree_oid="$tree_oid" dirty_paths:="$dirty"
      drop_worktree "$SC_REPO" "$dest" "$branch"
    done
  done
}

describe_fixture() { # scenario-id title
  local sc="$1" title="$2"
  record kind=fixture scenario="$sc" title="$title" \
    tracked_files:="$(git -C "$SC_REPO" ls-files | wc -l | tr -d ' ')" \
    logical_bytes:="$(python3 -c '
import os, sys
total = 0
for root, dirs, names in os.walk(sys.argv[1]):
    dirs[:] = [d for d in dirs if d != ".git"]
    for n in names:
        try:
            total += os.lstat(os.path.join(root, n)).st_size
        except OSError:
            pass
print(total)
' "$SC_REPO")" \
    source_commit="$(git -C "$SC_REPO" rev-parse HEAD)" \
    target_commitish="${SC_COMMITISH:-HEAD}" \
    filesystem="$(fs_type "$SC_VOL")"
}

skip() { record kind=skip scenario="$1" reason="$2"; note "skip $1: $2"; }

gen_tree() { # root nfiles bytes-per-file dirs
  python3 -c '
import os, sys
root, nfiles, size, ndirs = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
for d in range(ndirs):
    os.makedirs(os.path.join(root, f"d{d:03d}"), exist_ok=True)
for i in range(nfiles):
    with open(os.path.join(root, f"d{i % ndirs:03d}", f"f{i:05d}.bin"), "wb") as fh:
        fh.write(os.urandom(size))
' "$@"
}

# Random content only: compressible fixtures would understate what a real checkout
# allocates and flatter every disk figure on the page.
new_repo() { # path
  rm -rf "$1"; mkdir -p "$1/src"
  git -C "$1" init -q -b main
}

# --- scenarios -----------------------------------------------------------------

scenario_same_commit() {
  local dir="$SCRATCH/same-commit"
  SC_REPO="$dir/repo"; SC_VOL="$dir"; SC_DESTDIR="$dir"; SC_COMMITISH=""
  mkdir -p "$dir"
  new_repo "$SC_REPO"
  gen_tree "$SC_REPO/src" 2000 131072 50
  git_c "$SC_REPO" add -A && git_c "$SC_REPO" commit -qm base
  describe_fixture same-commit "Worktree at the same commit"
  measure_sides same-commit
  rm -rf "$dir"
}

scenario_cross_commit() {
  local dir="$SCRATCH/cross-commit"
  SC_REPO="$dir/repo"; SC_VOL="$dir"; SC_DESTDIR="$dir"; SC_COMMITISH="feature"
  mkdir -p "$dir"
  new_repo "$SC_REPO"
  gen_tree "$SC_REPO/src" 3000 65536 50
  git_c "$SC_REPO" add -A && git_c "$SC_REPO" commit -qm base
  git_c "$SC_REPO" checkout -q -b feature
  python3 -c '
import os, sys
root = sys.argv[1]
for i in range(5):
    with open(os.path.join(root, f"d{i:03d}", f"f{i:05d}.bin"), "wb") as fh:
        fh.write(os.urandom(65536))
with open(os.path.join(root, "d000", "added.bin"), "wb") as fh:
    fh.write(os.urandom(65536))
' "$SC_REPO/src"
  git_c "$SC_REPO" add -A && git_c "$SC_REPO" commit -qm feature
  git_c "$SC_REPO" checkout -q main
  describe_fixture cross-commit "Source checkout six paths behind the target"
  measure_sides cross-commit
  rm -rf "$dir"
}

scenario_no_match() {
  local dir="$SCRATCH/no-match"
  SC_REPO="$dir/repo"; SC_VOL="$dir"; SC_DESTDIR="$dir"; SC_COMMITISH="rewritten"
  mkdir -p "$dir"
  new_repo "$SC_REPO"
  gen_tree "$SC_REPO/src" 2000 131072 50
  git_c "$SC_REPO" add -A && git_c "$SC_REPO" commit -qm base
  git_c "$SC_REPO" checkout -q -b rewritten
  gen_tree "$SC_REPO/src" 2000 131072 50
  git_c "$SC_REPO" add -A && git_c "$SC_REPO" commit -qm rewritten
  git_c "$SC_REPO" checkout -q main
  describe_fixture no-match "No path shared with the source checkout"
  measure_sides no-match
  rm -rf "$dir"
}

scenario_kernel() {
  local repo="$CACHE/linux"
  if [ ! -d "$repo/.git" ]; then
    if [ "$FETCH" != 1 ]; then
      skip kernel "kernel fixture missing at $repo; rerun with --fetch (about 2 GB)"
      return 0
    fi
    note "cloning the kernel fixture into $repo (about 2 GB)"
    rm -rf "$repo"
    git clone --depth 1 -q "$KERNEL_URL" "$repo"
  fi
  git -C "$repo" worktree prune >/dev/null 2>&1 || true
  SC_REPO="$repo"; SC_VOL="$CACHE"; SC_DESTDIR="$CACHE"; SC_COMMITISH=""
  describe_fixture kernel "Linux kernel shallow clone"
  measure_sides kernel
}

scenario_loopfs() { # fs-name title files
  local fs="$1" title="$2" files="$3"
  local mnt="$CACHE/mnt-$fs"
  if ! loopfs_available "$fs"; then
    skip "$fs" "$LOOPFS_REASON"
    return 0
  fi
  loopfs_mount "$fs" "$CACHE/images" "$mnt"
  SC_REPO="$mnt/repo"; SC_VOL="$mnt"; SC_DESTDIR="$mnt"; SC_COMMITISH=""
  new_repo "$SC_REPO"
  gen_tree "$SC_REPO/src" "$files" 131072 50
  git_c "$SC_REPO" add -A && git_c "$SC_REPO" commit -qm base
  describe_fixture "$fs" "$title"
  measure_sides "$fs" || true
  [ "$fs" = btrfs ] && capture_btrfs_proof "$mnt"
  loopfs_umount "$mnt"
}

# The filesystem's own shared-versus-exclusive accounting is the strongest artefact
# available, so it is captured verbatim with one worktree per side alive at once.
capture_btrfs_proof() {
  local mnt="$1" side dest text
  for side in git sprout; do
    build_argv "$side" "$mnt/proof-$side" "bench-proof-$side"
    ( cd "$SC_REPO" && "${SIDE_ARGV[@]}" >/dev/null 2>&1 ) || return 0
  done
  text="$(loopfs_btrfs_du "$SC_REPO/src" "$mnt/proof-sprout/src" "$mnt/proof-git/src")" || return 0
  record kind=proof scenario=btrfs label="btrfs filesystem du -s" text="$text"
  for side in git sprout; do
    drop_worktree "$SC_REPO" "$mnt/proof-$side" "bench-proof-$side"
  done
}

# --- drive ---------------------------------------------------------------------

run_scenario() {
  case "$1" in
    kernel) scenario_kernel ;;
    same-commit) scenario_same_commit ;;
    cross-commit) scenario_cross_commit ;;
    no-match) scenario_no_match ;;
    btrfs) scenario_loopfs btrfs "btrfs" 1500 ;;
    ext4) scenario_loopfs ext4 "ext4, no block cloning" 1500 ;;
  esac
}

# A scenario that dies must not take the report with it: the remaining rows are
# still worth having, and the dead one is recorded as failed rather than guessed.
for sc in $SELECTED; do
  note "== $sc =="
  run_scenario "$sc" || record kind=failure scenario="$sc" reason="the scenario aborted"
done

if [ ${#MERGE_ARGS[@]} -gt 0 ]; then
  "$BENCH_DIR/build-report.py" --raw "$BENCH_RAW" --out "$OUT" "${MERGE_ARGS[@]}"
else
  "$BENCH_DIR/build-report.py" --raw "$BENCH_RAW" --out "$OUT"
fi
note "wrote $OUT"

if [ "$PROMOTE" = 1 ]; then
  [ "$BASELINE_ONLY" = 1 ] && die "--promote refuses a baseline-only run: both sides were git worktree add"
  cp "$OUT" "$BENCH_DIR/results.json"
  note "promoted to $BENCH_DIR/results.json"
fi
