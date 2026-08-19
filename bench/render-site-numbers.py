#!/usr/bin/env -S uv run --script
# ABOUTME: Rewrites every figure in docs/index.html and README.md from the benchmark
# ABOUTME: report, so no number on either surface can be one that no script produced.
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_REPORT = REPO_ROOT / "bench" / "results.json"

SCENARIOS: list[str] = [
    "kernel",
    "same-commit",
    "cross-commit",
    "no-match",
    "btrfs",
    "ext4",
]
SIDES: list[str] = ["git", "sprout"]

NOT_MEASURED = "not measured"
NOT_EXERCISED = "—"

# Markers the site and the README must carry. A page that quietly loses one is the
# failure this script exists to prevent, so a missing key is an error, not a warning.
REQUIRED: dict[str, list[str]] = {
    "docs/index.html": [
        "meta.status",
        "meta.generated",
        "meta.runs",
        "kernel.label",
        "kernel.files",
        "kernel.machine",
        "kernel.git.summary",
        "kernel.sprout.summary",
        "kernel.git.disk_mb",
        "kernel.sprout.disk_mb",
        "kernel.ratio.disk",
        "same-commit.label",
        "same-commit.git.summary",
        "same-commit.sprout.summary",
        "cross-commit.label",
        "cross-commit.git.summary",
        "cross-commit.sprout.summary",
        "btrfs.label",
        "btrfs.machine",
        "btrfs.git.summary",
        "btrfs.sprout.summary",
        "ext4.label",
        "ext4.git.summary",
        "ext4.sprout.summary",
        "proof.btrfs",
    ],
    "README.md": [
        "meta.status",
        "kernel.label",
        "kernel.machine",
        "kernel.git.summary",
        "kernel.sprout.summary",
        "kernel.ratio.disk",
    ],
}

SPAN_RE = re.compile(
    r"<!--bench:(?!region\b)([A-Za-z0-9_.\-]+)-->(.*?)<!--/bench-->", re.DOTALL
)
REGION_RE = re.compile(r"<!--bench:region-->(.*?)<!--/bench:region-->", re.DOTALL)
TAG_WITH_ATTR_RE = re.compile(r"<[^<>]*\bdata-bench-[^<>]*>", re.DOTALL)
ATTR_RE = re.compile(r'data-bench-([A-Za-z][A-Za-z0-9\-]*)="([A-Za-z0-9_.\-]+)"')
COMMENT_RE = re.compile(r"<!--.*?-->", re.DOTALL)
TAG_RE = re.compile(r"<[^<>]*>", re.DOTALL)
SHAPE_RE = re.compile(
    r"<(?:rect|circle|line|polygon|polyline|path)\b[^<>]*>", re.IGNORECASE
)
DIGITS_RE = re.compile(r"\d")


class MarkerError(Exception):
    pass


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


def fmt_ratio(value: float | None) -> str:
    if value is None:
        return NOT_MEASURED
    return f"{value:.0f}x" if value >= 10 else f"{value:.1f}x"


def fmt_count(value: int | None) -> str:
    return NOT_MEASURED if value is None else f"{value:,}".replace(",", " ")


def fmt_size(size_bytes: int | None) -> str:
    if size_bytes is None:
        return NOT_MEASURED
    gib = size_bytes / 1024**3
    return f"{gib:.1f} GB" if gib >= 1 else f"{size_bytes / 1024**2:.0f} MB"


def fmt_oid(oid: str | None) -> str:
    if not oid:
        return NOT_MEASURED
    return f"{oid[:8]}…" if isinstance(oid, str) else NOT_MEASURED


def fmt_pct(value: float | None, largest: float | None) -> str:
    if value is None or not largest or largest <= 0:
        return "0%"
    return f"{max(value, 0) / largest * 100:.1f}%"


# --- key catalogue -------------------------------------------------------------


def machine_line(machine: dict[str, Any]) -> str:
    return (
        f"{machine['cpu']}, {machine['cores']} cores, {machine['os']}, "
        f"git {machine['git_version'].removeprefix('git version ')}, "
        f"{machine['filesystem'].upper()}"
    )


def status_line(report: dict[str, Any]) -> str:
    generated = report["generated_at"][:10]
    if report["baseline_only"]:
        return (
            f"Provisional, {generated}: baseline-only run — both columns measured "
            "`git worktree add`, the tool itself was not exercised."
        )
    version = report["tool"]["version"] or "an unreleased build"
    if report["provisional"]:
        return (
            f"Provisional, {generated}: measured with {version}, "
            "not yet confirmed by the differential suite."
        )
    return f"Measured {generated} with {version}, differential suite green."


def build_keys(report: dict[str, Any]) -> dict[str, str]:
    baseline_only: bool = report["baseline_only"]
    by_id = {s["id"]: s for s in report["scenarios"]}

    keys: dict[str, str] = {
        "meta.status": status_line(report),
        "meta.generated": report["generated_at"][:10],
        "meta.runs": str(report["runs_per_side"]),
        "meta.tool_version": report["tool"]["version"] or NOT_MEASURED,
        "meta.machine": machine_line(report["machine"]),
        "proof.btrfs": NOT_MEASURED,
    }

    for name in SCENARIOS:
        scenario = by_id.get(name)
        ok = scenario is not None and scenario["status"] == "ok"
        sides = scenario.get("sides", {}) if scenario else {}
        fixture = scenario.get("fixture", {}) if scenario else {}
        machine = (scenario or {}).get("machine") or report["machine"]

        title = (scenario or {}).get("title", NOT_MEASURED)
        keys[f"{name}.title"] = title
        keys[f"{name}.label"] = (
            f"{title} — {fmt_count(fixture['tracked_files'])} files, "
            f"{fmt_size(fixture['logical_bytes'])}"
            if fixture
            else title
        )
        keys[f"{name}.machine"] = machine_line(machine) if ok else NOT_MEASURED
        keys[f"{name}.files"] = fmt_count(fixture.get("tracked_files"))
        keys[f"{name}.size"] = fmt_size(fixture.get("logical_bytes"))
        keys[f"{name}.status"] = (scenario or {}).get("status", "missing")
        keys[f"{name}.skip_reason"] = (scenario or {}).get("skip_reason") or ""

        largest_disk = (
            max(
                (sides.get(s, {}).get("disk_mb", {}).get("median", 0) or 0)
                for s in SIDES
            )
            if sides
            else 0
        )
        largest_time = (
            max(
                (sides.get(s, {}).get("time_s", {}).get("median", 0) or 0)
                for s in SIDES
            )
            if sides
            else 0
        )

        for side in SIDES:
            data = sides.get(side, {})
            blank = baseline_only and side == "sprout" and ok
            time_s = data.get("time_s", {}).get("median") if ok else None
            disk_mb = data.get("disk_mb", {}).get("median") if ok else None
            status_s = data.get("first_status_s", {}).get("median") if ok else None

            keys[f"{name}.{side}.time_s"] = NOT_EXERCISED if blank else fmt_time(time_s)
            keys[f"{name}.{side}.disk_mb"] = (
                NOT_EXERCISED if blank else fmt_disk(disk_mb)
            )
            keys[f"{name}.{side}.first_status_s"] = (
                NOT_EXERCISED if blank else fmt_time(status_s)
            )
            keys[f"{name}.{side}.summary"] = (
                NOT_EXERCISED
                if blank
                else (
                    f"{fmt_time(time_s)} · {fmt_disk(disk_mb)}"
                    if ok
                    else (scenario or {}).get("skip_reason") or NOT_MEASURED
                )
            )
            keys[f"{name}.{side}.tree_oid"] = (
                NOT_EXERCISED
                if blank
                else fmt_oid(data.get("tree_oid") if ok else None)
            )
            keys[f"{name}.{side}.dirty_paths"] = (
                NOT_EXERCISED
                if blank
                else fmt_count(data.get("dirty_paths") if ok else None)
            )
            keys[f"{name}.{side}.disk_pct"] = (
                "0%" if blank else fmt_pct(disk_mb, largest_disk)
            )
            keys[f"{name}.{side}.time_pct"] = (
                "0%" if blank else fmt_pct(time_s, largest_time)
            )

        comparison = (scenario or {}).get("comparison", {})
        keys[f"{name}.ratio.disk"] = (
            NOT_EXERCISED if baseline_only else fmt_ratio(comparison.get("disk_ratio"))
        )
        keys[f"{name}.ratio.time"] = (
            NOT_EXERCISED if baseline_only else fmt_ratio(comparison.get("time_ratio"))
        )
        keys[f"{name}.oid_match"] = (
            NOT_EXERCISED
            if baseline_only
            else ("identical" if comparison.get("tree_oid_match") else "DIFFERENT")
        )

        proof = (scenario or {}).get("proof")
        if proof:
            keys[f"proof.{name}"] = proof["text"]

    return keys


# --- rewriting -----------------------------------------------------------------


def set_attribute(tag: str, name: str, value: str) -> str:
    pattern = re.compile(rf'(\s{re.escape(name)}=")[^"]*(")')
    if pattern.search(tag):
        return pattern.sub(lambda m: m.group(1) + value + m.group(2), tag, count=1)
    close = "/>" if tag.rstrip().endswith("/>") else ">"
    return tag.rstrip()[: -len(close)].rstrip() + f' {name}="{value}"' + close


def rewrite(text: str, keys: dict[str, str]) -> tuple[str, set[str], list[str]]:
    seen: set[str] = set()
    unknown: list[str] = []

    def span(match: re.Match[str]) -> str:
        key = match.group(1)
        seen.add(key)
        if key not in keys:
            unknown.append(key)
            return match.group(0)
        return f"<!--bench:{key}-->{keys[key]}<!--/bench-->"

    text = SPAN_RE.sub(span, text)

    def tag(match: re.Match[str]) -> str:
        rewritten = match.group(0)
        for attribute, key in ATTR_RE.findall(rewritten):
            seen.add(key)
            if key not in keys:
                unknown.append(key)
                continue
            rewritten = set_attribute(rewritten, attribute, keys[key])
        return rewritten

    return TAG_WITH_ATTR_RE.sub(tag, text), seen, unknown


def unmarked_numbers(text: str) -> list[str]:
    """Digits inside a bench region that no marker produced — the silent-drift failure.

    Visible text and chart geometry are both figures a reader can read, so both are
    checked. A shape whose digits really are static opts out with `data-bench-ignore`.
    """
    offenders: list[str] = []
    for region in REGION_RE.findall(text):
        for shape in SHAPE_RE.findall(region):
            if "data-bench-" not in shape and DIGITS_RE.search(shape):
                offenders.append(shape.strip())
        stripped = COMMENT_RE.sub(" ", SPAN_RE.sub(" ", region))
        for line in TAG_RE.sub(" ", stripped).splitlines():
            if DIGITS_RE.search(line):
                offenders.append(line.strip())
    return offenders


def process(path: Path, relative: str, keys: dict[str, str], write: bool) -> list[str]:
    problems: list[str] = []
    if not path.exists():
        return [
            f"{relative}: missing; it must carry these markers: {', '.join(REQUIRED[relative])}"
        ]

    original = path.read_text()
    if original.count("<!--bench:") - original.count(
        "<!--bench:region-->"
    ) != original.count("<!--/bench-->"):
        problems.append(f"{relative}: unbalanced bench markers")

    updated, seen, unknown = rewrite(original, keys)
    problems += [
        f"{relative}: unknown marker key '{key}'" for key in sorted(set(unknown))
    ]
    problems += [
        f"{relative}: required marker '{key}' is missing"
        for key in REQUIRED[relative]
        if key not in seen
    ]
    problems += [
        f"{relative}: unmarked number inside a bench region: {line}"
        for line in unmarked_numbers(updated)
    ]

    if write and not problems and updated != original:
        path.write_text(updated)
        print(f"bench: rewrote {relative}")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--from", dest="source", type=Path, default=DEFAULT_REPORT)
    parser.add_argument(
        "--check", action="store_true", help="validate markers, write nothing"
    )
    parser.add_argument(
        "--print-keys", action="store_true", help="dump every key and its value"
    )
    parser.add_argument("--target", action="append", help="check only these targets")
    args = parser.parse_args()

    if not args.source.exists():
        print(
            f"bench: no report at {args.source}; run `just bench` first",
            file=sys.stderr,
        )
        return 1

    report = json.loads(args.source.read_text())
    keys = build_keys(report)

    if args.print_keys:
        for key in sorted(keys):
            print(f"{key}\t{keys[key]}")
        return 0

    targets = args.target or list(REQUIRED)
    problems: list[str] = []
    for relative in targets:
        if relative not in REQUIRED:
            print(f"bench: {relative} is not a known target", file=sys.stderr)
            return 1
        problems += process(REPO_ROOT / relative, relative, keys, write=not args.check)

    for problem in problems:
        print(f"bench: {problem}", file=sys.stderr)
    if problems:
        return 1

    if report["provisional"]:
        print(f"bench: {status_line(report)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
