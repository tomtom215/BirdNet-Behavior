#!/usr/bin/env python3
"""Check the CI configuration for the failure modes that have actually bitten us.

Every assertion here corresponds to a real incident, not a style preference.
Each one went unnoticed for weeks or months, because all three failed in a
direction that *looks* fine: a green job, a "cancelled" badge, a run still
spinning. Config bugs do not announce themselves the way test failures do, so
they get their own gate.

  1. A matrix row aimed at a path that no longer matches any source.
     `detections.rs` was split into a directory in 0.7.2; the matrix went on
     naming the old file for two months, generating no mutants, and the
     threshold step read the empty count as "0 missed" and went green.

  2. A shard that is empty by construction. cargo-mutants splits a shard set
     into contiguous ceil(total/n) blocks, so the tail shard gets nothing
     whenever (n-1)*ceil(total/n) >= total. `src/capture/schedule.rs` at 41
     mutants over 8 shards produced 6 6 6 6 6 6 5 0, and the empty shard failed
     the no-vacuous-pass gate on every run — `Mutation testing` was red on main
     from the v0.14.0 merge onward.

  3. A job with no `timeout-minutes`. GitHub then applies a 6-hour default, and
     a step that *hangs* rather than fails burns all six before anything goes
     red. `cross-aarch64` sat 1h27m inside `apt-get install` having built
     nothing, and had to be cancelled by hand.

  4. A job quietly growing into its own timeout. Declaring a budget (3) does
     not help if nothing watches the distance to it. `sqlite/queries/detections`
     grew until it was cancelled at 45m on three separate full runs, and a
     cancelled job renders as a grey badge rather than a red one, so the row had
     never once completed and its gate had never once gated. `validate.rs`
     reached 39m00s of the same 45m budget — 87% — and this gate did not exist
     to say so; it was found by reading run times by hand. The distance to the
     budget is now measured from the API and checked on every run.

Run from the repository root. Requires PyYAML, and cargo-mutants for the
mutant-enumeration checks (`--list` builds nothing; it costs ~0.2 s per row).
Check 4 additionally needs `GITHUB_TOKEN` with `actions: read`; outside CI it
says loudly that it is skipping rather than passing silently.
"""

from __future__ import annotations

import datetime as dt
import glob
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from math import ceil

import yaml

MUTATION_WORKFLOW = ".github/workflows/mutation.yml"

# Matrix rows may omit `package`; the workflow falls back to this.
DEFAULT_PACKAGE = "birdnet-core"

# No job should need longer than this. A larger value is almost always someone
# working around a hang instead of fixing it, which is the thing this checks
# for — so the ceiling is deliberately well above the slowest real job
# (release `build`, cross-compiling every target) and still far below 6 hours.
MAX_TIMEOUT_MINUTES = 120

# How much of its own budget a job may actually use before this is a finding.
#
# Chosen from the two data points that produced incident 4 rather than from
# taste. `db/migration.rs` was split by hand at 23m13s of 45m (52%) and the
# author called it "the next one that would have hit the wall"; `validate.rs`
# reached 39m00s of 45m (87%) and nothing noticed. A line at 75% is above every
# healthy job in the matrix today (the 12-16 min shards sit at 27-36%) and below
# both rows that needed splitting by the time anyone looked.
HEADROOM_THRESHOLD = 0.75

# Recent completed runs sampled per workflow. The *worst* run in the sample is
# what gets compared: a cold cache or a contended pool is exactly the condition
# that tips a job over its budget, so it is signal, not noise.
RUNS_SAMPLED = 5

failures: list[str] = []
skips: list[str] = []


def check(ok: bool, msg: str) -> None:
    print(f"  {'ok  ' if ok else 'FAIL'}  {msg}")
    if not ok:
        failures.append(msg)


def skip(msg: str) -> None:
    """Record a check that could not run.

    Printed, counted and listed in the summary. A check that quietly stops
    running is the shape of every incident above, so "skipped" must never look
    like "passed".
    """
    print(f"  SKIP  {msg}")
    skips.append(msg)


def mutant_count(package: str, file_glob: str, shard: str | None = None) -> int:
    """Enumerate mutants for one matrix row. `--list` does not build."""
    cmd = ["cargo", "mutants", "--list", "--package", package,
           "--file", file_glob, "--no-shuffle"]
    if shard:
        cmd += ["--shard", shard]
    out = subprocess.run(cmd, capture_output=True, text=True, check=False)
    return len([ln for ln in out.stdout.splitlines() if ln.strip()])


def check_timeouts() -> None:
    print("\nEvery workflow job declares a timeout (incident 3)")
    for path in sorted(glob.glob(".github/workflows/*.yml")):
        spec = yaml.safe_load(open(path))
        for name, job in (spec.get("jobs") or {}).items():
            budget = job.get("timeout-minutes")
            check(budget is not None,
                  f"{path}::{name} declares timeout-minutes")
            if budget is None:
                continue
            # A matrix-driven expression (`${{ matrix.timeout_minutes || 45 }}`)
            # is a declaration; its per-row values are the matrix author's call.
            if isinstance(budget, str):
                continue
            check(budget <= MAX_TIMEOUT_MINUTES,
                  f"{path}::{name} timeout {budget}m is within the "
                  f"{MAX_TIMEOUT_MINUTES}m ceiling")


def check_mutation_matrix() -> None:
    spec = yaml.safe_load(open(MUTATION_WORKFLOW))
    rows = spec["jobs"]["mutants"]["strategy"]["matrix"]["include"]

    print(f"\nMutation matrix: {len(rows)} rows")
    slugs = [r["slug"] for r in rows]
    dupes = {s for s in slugs if slugs.count(s) > 1}
    check(not dupes, f"artifact slugs unique{'' if not dupes else f' (dupes: {sorted(dupes)})'}")

    # Group sharded rows so each file is enumerated once per shard, and
    # unsharded rows are enumerated whole.
    by_file: dict[tuple[str, str], list[dict]] = {}
    for row in rows:
        # `package` is optional in the matrix and defaults to birdnet-core, the
        # same fallback the workflow's own run step applies. Reading it bare
        # would enumerate with `--package ""`, which returns 0 mutants for every
        # row regardless of content — a checker that reports a false failure on
        # five healthy rows is worse than no checker.
        by_file.setdefault(
            (row["file"], row.get("package", DEFAULT_PACKAGE)), []).append(row)

    print("\nEvery row still matches source (incident 1)")
    totals: dict[tuple[str, str], int] = {}
    for (path, package), group in sorted(by_file.items()):
        totals[(path, package)] = mutant_count(package, path)
        check(totals[(path, package)] > 0,
              f"{path} generates {totals[(path, package)]} mutants")

    print("\nNo shard is empty by construction (incident 2)")
    for (path, package), group in sorted(by_file.items()):
        shards = [r["shard"] for r in group if r.get("shard")]
        if not shards:
            continue
        n = int(shards[0].split("/")[1])
        indices = sorted(int(s.split("/")[0]) for s in shards)
        check(indices == list(range(n)),
              f"{path}: shards 0..{n - 1} all present exactly once")
        if indices != list(range(n)):
            continue

        total = totals[(path, package)]
        # The arithmetic reason, checked before spending time enumerating: with
        # contiguous ceil-sized blocks the tail shard is empty precisely when
        # the first n-1 shards can already hold every mutant.
        predicted_empty = (n - 1) * ceil(total / n) >= total
        sizes = [mutant_count(package, path, f"{k}/{n}") for k in range(n)]
        check(all(s > 0 for s in sizes),
              f"{path}: {total} mutants over {n} shards -> {sizes}")
        check(not predicted_empty,
              f"{path}: (n-1)*ceil(total/n) = {(n - 1) * ceil(total / n)} "
              f"< total = {total}")


# ---------------------------------------------------------------------------
# Incident 4: distance to the timeout
# ---------------------------------------------------------------------------

# `${{ matrix.key }}` or `${{ matrix.key || default }}`.
MATRIX_EXPR = re.compile(r"\$\{\{\s*matrix\.([A-Za-z_]\w*)\s*(?:\|\|\s*(.*?)\s*)?\}\}")


def resolve_matrix_expr(value: object, row: dict) -> str | None:
    """Render a workflow expression against one matrix row.

    Returns None when a reference cannot be resolved — the row has no such key
    and the expression declares no default. Those jobs are reported as
    unresolved rather than silently dropped.
    """
    unresolved = False

    def one(m: re.Match[str]) -> str:
        nonlocal unresolved
        key, default = m.group(1), m.group(2)
        if key in row:
            return str(row[key])
        if default:
            return default.strip().strip("'\"")
        unresolved = True
        return m.group(0)

    rendered = MATRIX_EXPR.sub(one, str(value))
    return None if unresolved else rendered


def configured_budgets() -> tuple[dict[tuple[str, str], int], list[str]]:
    """Every job's rendered display name and its timeout, plus what didn't render.

    A matrix job's rendered name is what the API reports, so the matrix is
    expanded here the same way GitHub expands it.
    """
    budgets: dict[tuple[str, str], int] = {}
    unresolved: list[str] = []
    for path in sorted(glob.glob(".github/workflows/*.yml")):
        spec = yaml.safe_load(open(path))
        for key, job in (spec.get("jobs") or {}).items():
            raw_budget = job.get("timeout-minutes")
            if raw_budget is None:
                continue  # check_timeouts already fails on this
            raw_name = job.get("name", key)
            rows = ((job.get("strategy") or {}).get("matrix") or {}).get("include") or [{}]
            for row in rows:
                name = resolve_matrix_expr(raw_name, row)
                budget = resolve_matrix_expr(raw_budget, row)
                if name is None or budget is None or not str(budget).isdigit():
                    unresolved.append(f"{path}::{key}")
                    break
                budgets[(path, name)] = int(budget)
    return budgets, sorted(set(unresolved))


def exceeds_headroom(observed_minutes: float, budget_minutes: int,
                     threshold: float = HEADROOM_THRESHOLD) -> bool:
    """Has this job used more of its budget than we are willing to leave to luck?"""
    return observed_minutes > budget_minutes * threshold


def _api(path: str) -> dict:
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or ""
    req = urllib.request.Request(
        f"https://api.github.com{path}",
        headers={
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
            "Authorization": f"Bearer {token}",
            "User-Agent": "check-ci-config",
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.load(resp)


def _minutes(job: dict) -> float | None:
    start, end = job.get("started_at"), job.get("completed_at")
    if not start or not end or job.get("conclusion") == "skipped":
        return None
    fmt = "%Y-%m-%dT%H:%M:%SZ"
    return (dt.datetime.strptime(end, fmt) - dt.datetime.strptime(start, fmt)).total_seconds() / 60


def worst_observed(workflow_file: str, repo: str) -> dict[str, float]:
    """Longest wall-clock seen per rendered job name, over recent completed runs."""
    basename = os.path.basename(workflow_file)
    runs = _api(
        f"/repos/{repo}/actions/workflows/{basename}/runs"
        f"?status=completed&per_page={RUNS_SAMPLED}"
    )
    worst: dict[str, float] = {}
    for run in runs.get("workflow_runs", []):
        for job in _api(f"/repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100").get("jobs", []):
            mins = _minutes(job)
            if mins is not None:
                worst[job["name"]] = max(worst.get(job["name"], 0.0), mins)
    return worst


def lookup_observed(name: str, observed: dict[str, float]) -> float | None:
    """Match a configured name to an observed one.

    A matrix job whose `name:` carries no matrix reference is rendered by GitHub
    as `Name (values)`, so an exact miss falls back to that prefix.
    """
    if name in observed:
        return observed[name]
    matches = [v for k, v in observed.items() if k.startswith(f"{name} (")]
    return max(matches) if matches else None


def check_headroom_arithmetic() -> None:
    """The pure parts of check 4, against the numbers that produced it.

    Kept in the always-run path rather than behind a flag: this is the only
    part of check 4 that works without API access, and an unexercised helper is
    how the rest of this file's incidents started.
    """
    print("\nHeadroom arithmetic (incident 4)")
    check(resolve_matrix_expr("cargo-mutants on ${{ matrix.label }}", {"label": "validate.rs [1/2]"})
          == "cargo-mutants on validate.rs [1/2]", "a matrix name renders")
    check(resolve_matrix_expr("${{ matrix.timeout_minutes || 45 }}", {}) == "45",
          "an absent matrix value falls back to the expression's default")
    check(resolve_matrix_expr("${{ matrix.timeout_minutes || 45 }}", {"timeout_minutes": 120})
          == "120", "a present matrix value wins over the default")
    check(resolve_matrix_expr("${{ matrix.package }}", {}) is None,
          "a reference with no value and no default stays unresolved")
    # The two rows that produced this check, and the two that did not.
    check(exceeds_headroom(39.0, 45), "validate.rs at 39m00s of 45m is a finding")
    check(exceeds_headroom(45.0, 45), "a job cancelled at its budget is a finding")
    check(not exceeds_headroom(23.216, 45), "db/migration.rs at 23m13s of 45m is not yet")
    check(not exceeds_headroom(19.216, 45), "a11y at 19m13s of 45m is not")
    check(not exceeds_headroom(16.0, 45), "a healthy 16m shard is not")
    # Name matching, which is what decides whether a budget is compared at all.
    observed = {"Tests (x86_64)": 12.0, "Build (amd64)": 9.0, "Build (arm64)": 31.0}
    check(lookup_observed("Tests (x86_64)", observed) == 12.0, "an exact job name matches")
    check(lookup_observed("Build", observed) == 31.0,
          "a matrix-suffixed name matches its worst row")
    check(lookup_observed("Never Ran", observed) is None, "an unseen job matches nothing")


def check_duration_headroom() -> None:
    """Compare what each job actually took against the budget it declares."""
    print("\nNo job is growing into its own timeout (incident 4)")
    budgets, unresolved = configured_budgets()
    for job in unresolved:
        skip(f"{job}: name or timeout does not render without a matrix value")

    repo = os.environ.get("GITHUB_REPOSITORY")
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    in_ci = os.environ.get("GITHUB_ACTIONS") == "true"
    if not (repo and token):
        msg = "no GITHUB_REPOSITORY/GITHUB_TOKEN — run durations not read"
        # In CI the credentials are always present, so their absence is a
        # broken gate rather than an environment that simply lacks them.
        if in_ci:
            check(False, f"check 4 has the credentials it needs ({msg})")
        else:
            skip(msg)
        return

    print(f"  ..    {len(budgets)} job(s) with a resolved budget, "
          f"worst of the last {RUNS_SAMPLED} run(s) per workflow")

    by_workflow: dict[str, dict[str, float]] = {}
    for path in sorted({p for p, _ in budgets}):
        try:
            by_workflow[path] = worst_observed(path, repo)
        except (urllib.error.URLError, urllib.error.HTTPError, KeyError, ValueError) as e:
            skip(f"{path}: could not read run history ({e})")

    if in_ci and not by_workflow:
        check(False, "check 4 read run history for at least one workflow")
        return

    unseen = 0
    for (path, name), budget in sorted(budgets.items()):
        observed = lookup_observed(name, by_workflow.get(path, {}))
        if observed is None:
            unseen += 1
            continue
        check(
            not exceeds_headroom(observed, budget),
            f"{path}::{name} used {observed:.0f}m of its {budget}m budget "
            f"({observed / budget:.0%}, limit {HEADROOM_THRESHOLD:.0%})",
        )
    if unseen:
        skip(f"{unseen} job(s) had no completed run in the last {RUNS_SAMPLED} per workflow")


def main() -> int:
    check_timeouts()
    check_mutation_matrix()
    check_headroom_arithmetic()
    check_duration_headroom()
    print()
    if skips:
        print(f"{len(skips)} check(s) skipped:")
        for s in skips:
            print(f"  - {s}")
    if failures:
        print(f"{len(failures)} check(s) failed:")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("all CI-config checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
