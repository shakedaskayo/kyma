#!/usr/bin/env bash
# perf-check.sh — compare current perf metrics vs baseline; emit comparison JSON.
#
# Usage: perf-check.sh <current.json> <baseline.json>
#
# Tolerance bands (hard-coded for v1.0; tunable later via env or config):
#   ingest_rps:   current >= 0.9  * baseline
#   query_p50_ms: current <= 1.1  * baseline
#   query_p99_ms: current <= 1.1  * baseline
#
# Modes:
#   default (warn-only): always exits 0; comparison JSON shows per-metric pass/fail.
#   GAUNTLET_PERF_ENFORCE=1: exits 1 if any metric breaches tolerance.
#
# Missing baseline (file doesn't exist): treated as "no baseline yet; skip"; exits 0.

set -euo pipefail

CURRENT="${1:?current metrics JSON path required}"
BASELINE="${2:?baseline metrics JSON path required}"
ENFORCE="${GAUNTLET_PERF_ENFORCE:-0}"

python3 - <<PY
import json, os, sys

current_path = "$CURRENT"
baseline_path = "$BASELINE"
enforce = "$ENFORCE" == "1"

if not os.path.exists(current_path):
    print(json.dumps({
        "overall_pass": False,
        "enforce_mode": enforce,
        "observations": [f"current metrics file does not exist: {current_path}"],
    }))
    sys.exit(2)

current = json.load(open(current_path))

if not os.path.exists(baseline_path):
    print(json.dumps({
        "current": current,
        "baseline": None,
        "comparisons": [],
        "enforce_mode": enforce,
        "overall_pass": True,
        "observations": ["no baseline.json yet; skipping comparison. Capture one via the perf-baseline-capture workflow."],
    }))
    sys.exit(0)

baseline = json.load(open(baseline_path))

# Each (metric, direction, tolerance) — direction is 'min_ratio' (current/baseline >= tol)
# or 'max_ratio' (current/baseline <= tol).
checks = [
    ("ingest_rps",   "min_ratio", 0.9),
    ("query_p50_ms", "max_ratio", 1.1),
    ("query_p99_ms", "max_ratio", 1.1),
]

comparisons = []
overall_pass = True

for metric, direction, tol in checks:
    cur = current.get(metric)
    base = baseline.get(metric)
    if cur is None or base is None or base == 0:
        comparisons.append({
            "metric": metric,
            "current": cur,
            "baseline": base,
            "ratio": None,
            "tolerance": tol,
            "direction": direction,
            "pass": False,
            "reason": "metric missing or baseline is zero",
        })
        overall_pass = False
        continue
    ratio = cur / base
    if direction == "min_ratio":
        passed = ratio >= tol
    else:
        passed = ratio <= tol
    comparisons.append({
        "metric": metric,
        "current": cur,
        "baseline": base,
        "ratio": round(ratio, 4),
        "tolerance": tol,
        "direction": direction,
        "pass": passed,
    })
    if not passed:
        overall_pass = False

print(json.dumps({
    "current": current,
    "baseline": baseline,
    "comparisons": comparisons,
    "enforce_mode": enforce,
    "overall_pass": overall_pass,
}))

if enforce and not overall_pass:
    sys.exit(1)
sys.exit(0)
PY
