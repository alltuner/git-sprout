#!/usr/bin/env -S uv run --script
# ABOUTME: Turns the benchmark driver's raw NDJSON records into bench.json, the single
# ABOUTME: machine-readable source for every figure the README and the site display.
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///

import argparse
import json
import statistics
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

SCENARIO_ORDER: list[str] = [
    "kernel",
    "same-commit",
    "cross-commit",
    "no-match",
    "btrfs",
    "ext4",
]

SIDES: list[str] = ["git", "sprout"]

UNITS: dict[str, str] = {
    "time_s": "seconds of wall clock",
    "disk_mb": "mebibytes of real disk consumed, measured as a free-space delta on the "
    "volume holding the worktree; logical size is deliberately not used",
    "first_status_s": "seconds for the first `git status --porcelain` in the new worktree",
    "logical_bytes": "bytes of working-tree content, excluding .git",
}


def spread(values: list[float]) -> dict[str, Any]:
    return {
        "median": round(statistics.median(values), 4),
        "min": round(min(values), 4),
        "max": round(max(values), 4),
        "samples": [round(v, 4) for v in values],
    }


def ratio(numerator: float, denominator: float) -> float | None:
    """Ratio of the two sides, or None when the divisor is too small to be meaningful."""
    if abs(denominator) < 0.005:
        return None
    return round(numerator / denominator, 3)


def read_records(raw: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in raw.read_text().splitlines() if line.strip()]


def side_report(samples: list[dict[str, Any]], command: dict[str, Any] | None) -> dict[str, Any]:
    oids = sorted({s["tree_oid"] for s in samples})
    return {
        "tool": (command or {}).get("tool"),
        "command": (command or {}).get("argv"),
        "cwd": (command or {}).get("cwd"),
        "runs": len(samples),
        "time_s": spread([float(s["time_s"]) for s in samples]),
        "disk_mb": spread([float(s["disk_mb"]) for s in samples]),
        "first_status_s": spread([float(s["first_status_s"]) for s in samples]),
        "tree_oid": oids[0] if len(oids) == 1 else oids,
        "dirty_paths": int(samples[0]["dirty_paths"]),
    }


def scenario_report(
    scenario: str,
    records: list[dict[str, Any]],
    provisional: bool,
    machine: dict[str, Any],
) -> dict[str, Any] | None:
    mine = [r for r in records if r.get("scenario") == scenario]
    if not mine:
        return None

    skips = [r for r in mine if r["kind"] == "skip"]
    fixture = next((r for r in mine if r["kind"] == "fixture"), None)
    failures = [r for r in mine if r["kind"] == "failure"]

    report: dict[str, Any] = {
        "id": scenario,
        "provisional": provisional,
        "title": (fixture or {}).get("title", scenario),
        "status": "ok",
        "skip_reason": None,
        "machine": machine,
    }

    if skips:
        report["status"] = "skipped"
        report["skip_reason"] = skips[0]["reason"]
        return report

    if fixture is not None:
        report["fixture"] = {
            "tracked_files": fixture["tracked_files"],
            "logical_bytes": fixture["logical_bytes"],
            "source_commit": fixture["source_commit"],
            "target_commitish": fixture["target_commitish"],
            "filesystem": fixture["filesystem"],
        }

    sides: dict[str, Any] = {}
    for side in SIDES:
        samples = [r for r in mine if r["kind"] == "sample" and r["side"] == side]
        command = next((r for r in mine if r["kind"] == "command" and r["side"] == side), None)
        if samples:
            sides[side] = side_report(samples, command)
    report["sides"] = sides

    if failures or len(sides) != len(SIDES):
        report["status"] = "failed"
        report["skip_reason"] = (
            failures[0]["reason"] if failures else "one side produced no samples"
        )
        return report

    git, sprout = sides["git"], sides["sprout"]
    report["comparison"] = {
        "time_ratio": ratio(git["time_s"]["median"], sprout["time_s"]["median"]),
        "disk_ratio": ratio(git["disk_mb"]["median"], sprout["disk_mb"]["median"]),
        "tree_oid_match": git["tree_oid"] == sprout["tree_oid"],
        "dirty_paths_match": git["dirty_paths"] == sprout["dirty_paths"],
    }

    proof = next((r for r in mine if r["kind"] == "proof"), None)
    if proof is not None:
        report["proof"] = {"label": proof["label"], "text": proof["text"]}

    return report


def merge_scenarios(
    local: list[dict[str, Any]], others: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    """Fold in scenarios another machine measured, keeping each one's own provenance.

    The btrfs and ext4 rows can only be measured on Linux, so the release-time report
    is assembled from a macOS run and a Linux CI run.
    """
    by_id = {report["id"]: report for report in local}
    for other in others:
        for report in other["scenarios"]:
            mine = by_id.get(report["id"])
            if report["status"] == "ok" and (mine is None or mine["status"] != "ok"):
                by_id[report["id"]] = report
    return [by_id[name] for name in SCENARIO_ORDER if name in by_id]


def build(raw: Path, merges: list[Path]) -> dict[str, Any]:
    records = read_records(raw)
    meta = next(r for r in records if r["kind"] == "meta")
    others = [json.loads(path.read_text()) for path in merges]
    provisional = bool(meta["provisional"]) or any(o["provisional"] for o in others)
    baseline_only = bool(meta["baseline_only"]) or any(o["baseline_only"] for o in others)

    machine = {
        "cpu": meta["cpu"],
        "cores": meta["cores"],
        "os": meta["os"],
        "kernel": meta["kernel"],
        "arch": meta["arch"],
        "git_version": meta["git_version"],
        "filesystem": meta["filesystem"],
        "scratch_path": meta["scratch_path"],
    }
    local = [
        report
        for name in SCENARIO_ORDER
        if (report := scenario_report(name, records, provisional, machine)) is not None
    ]
    scenarios = merge_scenarios(local, others)

    return {
        "schema_version": meta["schema_version"],
        "generated_at": datetime.now(UTC).isoformat(timespec="seconds"),
        "provisional": provisional,
        "baseline_only": baseline_only,
        "differential_verified": bool(meta["differential_verified"]),
        "runs_per_side": meta["runs_per_side"],
        "units": UNITS,
        "tool": {
            "name": "git-sprout",
            "path": meta["tool_path"] or None,
            "version": meta["tool_version"] or None,
        },
        "machine": machine,
        "scenarios": scenarios,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw", required=True, type=Path, help="NDJSON written by run.sh")
    parser.add_argument("--out", required=True, type=Path, help="where to write bench.json")
    parser.add_argument(
        "--merge",
        action="append",
        default=[],
        type=Path,
        help="fold scenarios this run skipped in from another machine's report",
    )
    args = parser.parse_args()

    report = build(args.raw, args.merge)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + "\n")

    if report["baseline_only"]:
        print(
            "bench: baseline-only run — both sides were `git worktree add`, "
            "so the sprout column measures git, not the tool",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
