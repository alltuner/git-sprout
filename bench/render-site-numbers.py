#!/usr/bin/env -S uv run --script
# ABOUTME: Rewrites every figure in the site pages and README.md from the benchmark
# ABOUTME: report, so no number on either surface can be one that no script produced.
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///

import argparse
import html
import json
import re
import sys
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_REPORT = REPO_ROOT / "bench" / "results.json"
TARGETS: list[str] = ["docs/index.html", "README.md"]

# Figures the report produces and no surface publishes, on purpose.
#
# The kernel wall clock is measured but never printed: on the two machines that have
# run it, the *control* moved 39.64s to 24.18s and 10.61s to 27.40s between consecutive
# runs, so neither could support a claim about a difference smaller than its own noise.
# The specification asks for disk to lead and time not to be claimed. Listing them here
# keeps them measured and unpublished rather than silently unregenerated.
DELIBERATELY_UNPUBLISHED: frozenset[str] = frozenset(
    {"kernel.time.git", "kernel.time.sprout"}
)

NOT_MEASURED = "not measured"
NOT_EXERCISED = "—"

# The page names a workload; the harness names a scenario. `no-match` is measured for
# the worst-case budget in spec §9 and is deliberately not a figure on the page.
WORKLOADS: dict[str, str] = {
    "kernel": "kernel",
    "medium": "same-commit",
    "cross": "cross-commit",
    "btrfs": "btrfs",
    "ext4": "ext4",
}
# The ext4 row states that the tool falls back, so it has no tool column.
SIDES_ON_PAGE: dict[str, list[str]] = {"ext4": ["git"]}
DEFAULT_SIDES: list[str] = ["git", "sprout"]

# The sentence about ten engineers with five worktrees each.
FLEET_WORKTREES = 50

# The bar chart's viewBox is 1000 units wide.
CHART_WIDTH = 1000.0

SPAN_RE = re.compile(
    r"<!--bench:(?!region\b)([A-Za-z0-9_.\-]+)-->(.*?)<!--/bench-->", re.DOTALL
)
REGION_RE = re.compile(r"<!--bench:region-->(.*?)<!--/bench:region-->", re.DOTALL)
COMMENT_RE = re.compile(r"<!--.*?-->", re.DOTALL)
TAG_RE = re.compile(r"<[^<>]*>", re.DOTALL)
SHAPE_RE = re.compile(
    r"<(?:rect|circle|line|polygon|polyline|path)\b[^<>]*>", re.IGNORECASE
)
WIDTH_RE = re.compile(r'width="[^"]*"')
DIGITS_RE = re.compile(r"\d")


# --- formatting ----------------------------------------------------------------


def fmt_time(seconds: float | None) -> str:
    return NOT_MEASURED if seconds is None else f"{seconds:.2f}s"


def fmt_disk(mebibytes: float | None) -> str:
    if mebibytes is None:
        return NOT_MEASURED
    if mebibytes < 0.5:
        return "~0 MB"
    if mebibytes < 10:
        return f"{mebibytes:.1f} MB"
    return f"{mebibytes:.0f} MB"


def fmt_disk_large(mebibytes: float | None) -> str:
    """Bigger figures read better in GB, and the prose around them is written that way."""
    if mebibytes is None:
        return NOT_MEASURED
    return f"{mebibytes / 1024:.1f} GB" if mebibytes >= 1024 else fmt_disk(mebibytes)


def fmt_ratio(value: float | None) -> str:
    if value is None:
        return NOT_MEASURED
    return f"{value:.0f}x" if value >= 10 else f"{value:.1f}x"


def fmt_count(value: int | None) -> str:
    return NOT_MEASURED if value is None else f"{value:,}".replace(",", " ")


def fmt_bytes(size_bytes: int | None) -> str:
    if size_bytes is None:
        return NOT_MEASURED
    gib = size_bytes / 1024**3
    return f"{gib:.1f} GB" if gib >= 1 else f"{size_bytes / 1024**2:.0f} MB"


# --- the key catalogue ---------------------------------------------------------


def median(scenario: dict[str, Any] | None, side: str, metric: str) -> float | None:
    if scenario is None or scenario["status"] != "ok":
        return None
    data = scenario.get("sides", {}).get(side)
    return None if data is None else data[metric]["median"]


def macos_line(machine: dict[str, Any]) -> str:
    return (
        f"{machine['cpu']}, {machine['cores']} cores, {machine['os']}, "
        f"git {machine['git_version'].removeprefix('git version ')}"
    )


def linux_line(machine: dict[str, Any]) -> str:
    return (
        f"kernel {machine['kernel'].removeprefix('Linux ')}, "
        f"git {machine['git_version'].removeprefix('git version ')}, loopback btrfs and ext4"
    )


def build_keys(report: dict[str, Any]) -> tuple[dict[str, str], dict[str, float]]:
    """Every name the pages may carry, and the bar widths that are markup, not text."""
    baseline_only: bool = report["baseline_only"]
    by_id = {s["id"]: s for s in report["scenarios"]}

    def blanked(workload: str, side: str, rendered: str) -> str:
        scenario = by_id.get(WORKLOADS[workload])
        ok = scenario is not None and scenario["status"] == "ok"
        return NOT_EXERCISED if baseline_only and side == "sprout" and ok else rendered

    keys: dict[str, str] = {}
    for workload, scenario_id in WORKLOADS.items():
        scenario = by_id.get(scenario_id)
        for side in SIDES_ON_PAGE.get(workload, DEFAULT_SIDES):
            keys[f"{workload}.disk.{side}"] = blanked(
                workload, side, fmt_disk(median(scenario, side, "disk_mb"))
            )
            keys[f"{workload}.time.{side}"] = blanked(
                workload, side, fmt_time(median(scenario, side, "time_s"))
            )

    kernel = by_id.get("kernel")
    fixture = (kernel or {}).get("fixture", {})
    keys["kernel.files"] = fmt_count(fixture.get("tracked_files"))
    keys["kernel.bytes"] = fmt_bytes(fixture.get("logical_bytes"))

    git_disk = median(kernel, "git", "disk_mb")
    sprout_disk = median(kernel, "sprout", "disk_mb")
    keys["kernel.disk.git.round"] = fmt_disk_large(git_disk)

    derived_unavailable = baseline_only or git_disk is None or sprout_disk is None
    keys["kernel.disk.ratio"] = (
        NOT_EXERCISED
        if baseline_only
        else fmt_ratio((kernel or {}).get("comparison", {}).get("disk_ratio"))
    )
    keys["kernel.disk.saved"] = (
        NOT_EXERCISED if derived_unavailable else fmt_disk_large(git_disk - sprout_disk)
    )
    keys["fleet.disk.git"] = (
        NOT_MEASURED if git_disk is None else fmt_disk_large(git_disk * FLEET_WORKTREES)
    )
    keys["fleet.disk.sprout"] = (
        NOT_EXERCISED
        if derived_unavailable
        else fmt_disk_large(sprout_disk * FLEET_WORKTREES)
    )

    proof = (by_id.get("btrfs") or {}).get("proof")
    keys["btrfs.du"] = proof["text"] if proof else NOT_MEASURED

    macos = next(
        (
            s["machine"]
            for s in report["scenarios"]
            if s["machine"]["os"].startswith("macOS")
        ),
        report["machine"],
    )
    keys["env.macos"] = macos_line(macos)
    linux = next(
        (
            s["machine"]
            for s in report["scenarios"]
            if s["status"] == "ok" and s["id"] in ("btrfs", "ext4")
        ),
        None,
    )
    keys["env.linux"] = linux_line(linux) if linux else NOT_MEASURED

    # The bars are derived from the disk figures so the chart can never disagree with
    # the table it sits above.
    largest = max(git_disk or 0.0, sprout_disk or 0.0)
    bars: dict[str, float] = {
        "chart.bar.git": 0.0
        if not largest
        else (git_disk or 0.0) / largest * CHART_WIDTH,
        "chart.bar.sprout": 0.0
        if baseline_only or not largest
        else max(sprout_disk or 0.0, 0.0) / largest * CHART_WIDTH,
    }
    return keys, bars


# --- rewriting -----------------------------------------------------------------


def render(
    key: str, previous: str, keys: dict[str, str], bars: dict[str, float], as_html: bool
) -> str:
    """A bar keeps the site's own markup and only has its width rewritten."""
    if key in bars:
        width = f"{bars[key]:.1f}"
        if WIDTH_RE.search(previous):
            return WIDTH_RE.sub(f'width="{width}"', previous, count=1)
        return f'<rect class="bar" x="0" y="0" width="{width}" height="24"/>'
    value = keys[key]
    return html.escape(value, quote=False) if as_html else value


def rewrite(
    text: str, keys: dict[str, str], bars: dict[str, float], as_html: bool
) -> tuple[str, set[str], list[str]]:
    seen: set[str] = set()
    unknown: list[str] = []

    def span(match: re.Match[str]) -> str:
        key, previous = match.group(1), match.group(2)
        seen.add(key)
        if key not in keys and key not in bars:
            unknown.append(key)
            return match.group(0)
        return f"<!--bench:{key}-->{render(key, previous, keys, bars, as_html)}<!--/bench-->"

    return SPAN_RE.sub(span, text), seen, unknown


def unmarked_numbers(text: str) -> list[str]:
    """Digits inside a bench region that no marker produced — the silent-drift failure.

    Visible text and chart geometry are both figures a reader can read, so both are
    checked. A shape whose digits really are static opts out with `data-bench-ignore`.
    """
    offenders: list[str] = []
    for region in REGION_RE.findall(text):
        outside = COMMENT_RE.sub(" ", SPAN_RE.sub(" ", region))
        offenders += [
            shape.strip()
            for shape in SHAPE_RE.findall(outside)
            if "data-bench-ignore" not in shape and DIGITS_RE.search(shape)
        ]
        offenders += [
            line.strip()
            for line in TAG_RE.sub(" ", outside).splitlines()
            if DIGITS_RE.search(line)
        ]
    return offenders


def process(
    path: Path,
    relative: str,
    keys: dict[str, str],
    bars: dict[str, float],
    write: bool,
) -> tuple[list[str], set[str]]:
    if not path.exists():
        return [f"{relative}: missing"], set()

    original = path.read_text()
    problems: list[str] = []
    openers = original.count("<!--bench:") - original.count("<!--bench:region-->")
    if openers != original.count("<!--/bench-->"):
        problems.append(f"{relative}: unbalanced bench markers")

    updated, seen, unknown = rewrite(
        original, keys, bars, as_html=relative.endswith(".html")
    )
    problems += [
        f"{relative}: marker '{key}' has no value in the report"
        for key in sorted(set(unknown))
    ]
    problems += [
        f"{relative}: unmarked number inside a bench region: {line}"
        for line in unmarked_numbers(updated)
    ]

    if write and not problems and updated != original:
        path.write_text(updated)
        print(f"bench: rewrote {relative}")
    return problems, seen


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--from", dest="source", type=Path, default=DEFAULT_REPORT)
    parser.add_argument(
        "--check", action="store_true", help="validate markers, write nothing"
    )
    parser.add_argument(
        "--print-keys", action="store_true", help="dump every key and its value"
    )
    args = parser.parse_args()

    if not args.source.exists():
        print(
            f"bench: no report at {args.source}; run `just bench` first",
            file=sys.stderr,
        )
        return 1

    report = json.loads(args.source.read_text())
    keys, bars = build_keys(report)

    if args.print_keys:
        for key in sorted(keys):
            print(f"{key}\t{keys[key]}")
        for key in sorted(bars):
            print(f"{key}\twidth={bars[key]:.1f} of {CHART_WIDTH:.0f}")
        return 0

    problems: list[str] = []
    placed: set[str] = set()
    for relative in TARGETS:
        found, seen = process(
            REPO_ROOT / relative, relative, keys, bars, write=not args.check
        )
        problems += found
        placed |= seen

    # The check that makes this stream worth having, in both directions: a marker the
    # report cannot fill is caught above; a figure the report has and no page carries
    # is a number that quietly stopped being regenerated.
    orphaned = (set(keys) | set(bars)) - placed - DELIBERATELY_UNPUBLISHED
    problems += [
        f"no page carries marker '{key}', so its value is not being regenerated"
        for key in sorted(orphaned)
    ]

    for problem in problems:
        print(f"bench: {problem}", file=sys.stderr)
    if problems:
        return 1

    if report["provisional"]:
        generated = report["generated_at"][:10]
        reason = (
            "both columns measured `git worktree add`"
            if report["baseline_only"]
            else "the differential suite has not confirmed this build"
        )
        print(f"bench: provisional figures ({generated}): {reason}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
