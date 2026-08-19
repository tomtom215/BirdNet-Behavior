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

  5. A `# observed` annotation that stopped being true. Each `timeout-minutes:`
     carries the runtime the budget was sized against, and they were written by
     hand, so they rotted: `Clippy` said `# observed 54s` while really taking
     8m52s, `Tests` claimed 10m42s against a real 22m37s, and four more were out
     by up to a factor of ten. Nothing in the file was wrong in a way a reader
     could see — the numbers simply described a repository from several
     thousand commits ago. They are now generated from run history and gated on
     drift, the same way `scripts/gen-cli-help.sh` keeps the CLI docs from
     drifting from the binary:

         python3 scripts/check-ci-config.py --update-observed

Run from the repository root. Requires PyYAML, and cargo-mutants for the
mutant-enumeration checks (`--list` builds nothing; it costs ~0.2 s per row).
Checks 4 and 5 additionally need `GITHUB_TOKEN` with `actions: read`; outside
CI they say loudly that they are skipping rather than passing silently.
"""

from __future__ import annotations

import datetime as dt
import glob
import json
import os
import re
import statistics
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


def configured_jobs() -> tuple[list[tuple[str, str, str, int]], list[str]]:
    """Every job as `(workflow, job key, rendered display name, timeout)`.

    A matrix job's rendered name is what the API reports, so the matrix is
    expanded here the same way GitHub expands it. Rows whose name or timeout
    cannot be rendered without a matrix value are returned separately rather
    than silently dropped.
    """
    jobs: list[tuple[str, str, str, int]] = []
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
                jobs.append((path, key, name, int(budget)))
    return jobs, sorted(set(unresolved))


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


def superseded(run: dict) -> bool:
    """Was this whole run killed by a newer push, rather than finishing?

    A job in such a run has a `started_at` and a `completed_at` and looks
    exactly like a job that ran that long — but the elapsed time says when
    somebody pushed again, not what the job costs. Feeding it to either check
    below is measuring the contributor's habits instead of the workflow.

    That is not hypothetical: three pushes to one pull request in an hour put
    two superseded runs into a five-run window and turned twelve rows red at
    once, every one of them reading "used 45m of its 45m budget" for jobs that
    really take 15. Any contributor who pushes twice reproduces it.

    The run's own conclusion separates the two cases exactly, which was checked
    against real runs rather than assumed:

      * a job that hits its own `timeout-minutes` is recorded as a cancelled
        *job*, but its run concludes `failure` — this is the signal check 4
        exists to catch, and it survives;
      * a run superseded by a newer push concludes `cancelled` — noise, and it
        is dropped here.
    """
    return run.get("conclusion") == "cancelled"


def observed_runtimes(workflow_file: str, repo: str) -> dict[str, list[float]]:
    """Every recent run's wall-clock, in minutes, per rendered job name.

    Returned as the raw sample rather than one number, because the two
    questions asked of it want different statistics: whether a job is about to
    be cancelled is a question about its *worst* run, and whether an `observed`
    annotation still describes it is a question about its *typical* one.

    Runs superseded by a newer push are skipped — see [`superseded`].
    """
    basename = os.path.basename(workflow_file)
    runs = _api(
        f"/repos/{repo}/actions/workflows/{basename}/runs"
        f"?status=completed&per_page={RUNS_SAMPLED}"
    )
    seen: dict[str, list[float]] = {}
    for run in runs.get("workflow_runs", []):
        if superseded(run):
            continue
        for job in _api(f"/repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100").get("jobs", []):
            mins = _minutes(job)
            if mins is not None:
                seen.setdefault(job["name"], []).append(mins)
    return seen


def lookup_observed(name: str, observed: dict[str, list[float]]) -> list[float]:
    """Match a configured name to observed samples.

    A matrix job whose `name:` carries no matrix reference is rendered by
    GitHub as `Name (values)`, so an exact miss falls back to that prefix and
    pools the rows — they share one `timeout-minutes:` line, so they share one
    answer.
    """
    if name in observed:
        return observed[name]
    pooled: list[float] = []
    for key, samples in observed.items():
        if key.startswith(f"{name} ("):
            pooled += samples
    return pooled


# --- the `# observed` annotations ------------------------------------------
#
# These are generated and drift-gated, not hand-maintained. Hand-maintained
# they rotted badly: `Clippy` was annotated `# observed 54s` while really
# taking 8m52s, `Tests` claimed 10m42s against a real 22m37s, and four more
# were out by up to a factor of ten. A comment that silently stops being true
# is worse than no comment, and this repo already has the answer to that in
# scripts/gen-cli-help.sh — generate it, then fail CI when the committed copy
# drifts. Same pattern, same reason.
#
# Drift is measured against the median rather than the worst run: a single
# cold-cache outlier should not make an accurate annotation red.

# A job-level timeout, at the fixed four-space indent these workflows use.
# Step-level timeouts sit deeper and are deliberately not matched.
TIMEOUT_LINE = re.compile(
    r"^ {4}timeout-minutes: (?P<budget>\d+)"
    r"(?:\s+#\s*observed\s+(?P<observed>[0-9hms]+))?\s*$"
)
JOB_LINE = re.compile(r"^ {2}(?P<key>[A-Za-z_][\w-]*):\s*$")
DURATION = re.compile(r"^(?:(\d+)m)?(\d+)s$")

# How far an annotation may be from the measured median before it is a lie.
# Real spread between the annotations that are still true and the ones that
# rotted is wide: the worst accurate one is out by 1.18x, the best stale one by
# 2.11x. A line at 1.5x sits in that gap with room on both sides.
DRIFT_TOLERANCE = 1.5


def format_duration(seconds: float) -> str:
    """Render seconds in the annotation format already used in these files."""
    total = int(round(seconds))
    return f"{total}s" if total < 60 else f"{total // 60}m{total % 60:02d}s"


def parse_duration(text: str) -> int | None:
    """Read an annotation back. None if it is not in the expected form."""
    m = DURATION.match(text.strip())
    return None if m is None else int(m.group(1) or 0) * 60 + int(m.group(2))


def has_drifted(claimed_seconds: float, actual_seconds: float,
                tolerance: float = DRIFT_TOLERANCE) -> bool:
    """Is the annotation far enough from reality to mislead a reader?

    Symmetric: an annotation claiming far *more* than the job takes is as
    untrue as one claiming far less, even though only the second direction
    hides a job growing toward its timeout.
    """
    if min(claimed_seconds, actual_seconds) <= 0:
        return claimed_seconds != actual_seconds
    return max(claimed_seconds, actual_seconds) / min(claimed_seconds, actual_seconds) > tolerance


def annotation_lines(path: str) -> list[tuple[int, str, int, str | None]]:
    """`(line index, job key, budget, annotation)` for each job-level timeout."""
    found = []
    job = ""
    for i, line in enumerate(open(path).read().split("\n")):
        job_match = JOB_LINE.match(line)
        if job_match:
            job = job_match.group("key")
            continue
        timeout = TIMEOUT_LINE.match(line)
        if timeout and job:
            found.append((i, job, int(timeout.group("budget")), timeout.group("observed")))
    return found


def median_seconds(path: str, key: str, jobs: list[tuple[str, str, str, int]],
                   runtimes: dict[str, dict[str, list[float]]]) -> float | None:
    """Typical runtime of one `timeout-minutes:` line, in seconds.

    A matrix job's rows share the line, so the slowest row's median is what the
    line has to describe.
    """
    medians = [
        statistics.median(samples) * 60
        for wf, job_key, name, _ in jobs
        if wf == path and job_key == key
        for samples in [lookup_observed(name, runtimes.get(path, {}))]
        if samples
    ]
    return max(medians) if medians else None


def check_headroom_arithmetic() -> None:
    """The pure parts of check 4 and 5, against the numbers that produced them.

    Kept in the always-run path rather than behind a flag: this is the only
    part of either check that works without API access, and an unexercised
    helper is how the rest of this file's incidents started.
    """
    print("\nHeadroom and annotation arithmetic (incidents 4, 5)")
    check(resolve_matrix_expr("cargo-mutants on ${{ matrix.label }}", {"label": "validate.rs [1/2]"})
          == "cargo-mutants on validate.rs [1/2]", "a matrix name renders")
    check(resolve_matrix_expr("${{ matrix.timeout_minutes || 45 }}", {}) == "45",
          "an absent matrix value falls back to the expression's default")
    check(resolve_matrix_expr("${{ matrix.timeout_minutes || 45 }}", {"timeout_minutes": 120})
          == "120", "a present matrix value wins over the default")
    check(resolve_matrix_expr("${{ matrix.package }}", {}) is None,
          "a reference with no value and no default stays unresolved")
    # The two rows that produced check 4, and the ones that did not.
    check(exceeds_headroom(39.0, 45), "validate.rs at 39m00s of 45m is a finding")
    check(exceeds_headroom(45.0, 45), "a job cancelled at its budget is a finding")
    check(not exceeds_headroom(23.216, 45), "db/migration.rs at 23m13s of 45m is not yet")
    check(not exceeds_headroom(19.216, 45), "a11y at 19m13s of 45m is not")
    check(not exceeds_headroom(16.0, 45), "a healthy 16m shard is not")
    # Name matching, which decides whether a budget is compared at all.
    observed = {"Tests (x86_64)": [12.0], "Build (amd64)": [9.0], "Build (arm64)": [31.0]}
    check(lookup_observed("Tests (x86_64)", observed) == [12.0], "an exact job name matches")
    check(sorted(lookup_observed("Build", observed)) == [9.0, 31.0],
          "a matrix-suffixed name pools its rows")
    check(lookup_observed("Never Ran", observed) == [], "an unseen job matches nothing")
    # Annotation round-tripping, in the format these files already use.
    for text, seconds in [("11s", 11), ("54s", 54), ("1m50s", 110), ("10m42s", 642)]:
        check(parse_duration(text) == seconds, f"{text} reads back as {seconds}s")
        check(format_duration(seconds) == text, f"{seconds}s renders as {text}")
    check(parse_duration("about a minute") is None, "an unparseable annotation is refused")
    # The drift verdicts, on the annotations that rotted and the ones that held.
    check(has_drifted(54, 532), "Clippy's `# observed 54s` against a real 8m52s has drifted")
    check(has_drifted(642, 1357), "Tests' `# observed 10m42s` against a real 22m37s has drifted")
    check(not has_drifted(11, 13), "`# observed 11s` against a real 13s has not")
    check(not has_drifted(623, 620), "`# observed 10m23s` against a real 10m20s has not")
    # Superseded runs, and the one kind of cancellation that must survive.
    check(superseded({"conclusion": "cancelled"}),
          "a run killed by a newer push is dropped")
    check(not superseded({"conclusion": "failure"}),
          "a run where a job hit its own timeout is kept — that is check 4's signal")
    check(not superseded({"conclusion": "success"}), "a clean run is kept")
    check(not superseded({}), "a run with no conclusion recorded is kept")


def check_duration_headroom(jobs: list[tuple[str, str, str, int]],
                            runtimes: dict[str, dict[str, list[float]]]) -> None:
    """Compare what each job actually took against the budget it declares."""
    print("\nNo job is growing into its own timeout (incident 4)")
    print(f"  ..    {len(jobs)} job(s) with a resolved budget, "
          f"worst of the last {RUNS_SAMPLED} run(s) per workflow")
    unseen = 0
    for path, _key, name, budget in sorted(jobs):
        samples = lookup_observed(name, runtimes.get(path, {}))
        if not samples:
            unseen += 1
            continue
        worst = max(samples)
        check(
            not exceeds_headroom(worst, budget),
            f"{path}::{name} used {worst:.0f}m of its {budget}m budget "
            f"({worst / budget:.0%}, limit {HEADROOM_THRESHOLD:.0%})",
        )
    if unseen:
        skip(f"{unseen} job(s) had no completed run in the last {RUNS_SAMPLED} per workflow")


def check_observed_annotations(jobs: list[tuple[str, str, str, int]],
                               runtimes: dict[str, dict[str, list[float]]]) -> None:
    """Every `# observed` annotation still describes the job it annotates."""
    print("\nEvery `# observed` annotation is still true (incident 5)")
    stale = 0
    for path in sorted({p for p, _, _, _ in jobs}):
        for _i, key, _budget, claimed in annotation_lines(path):
            actual = median_seconds(path, key, jobs, runtimes)
            if actual is None:
                # Nothing to compare against, and nothing to claim either: an
                # annotation is only owed where there is run history.
                if claimed:
                    skip(f"{path}::{key} claims {claimed} but has no recent run to check it")
                continue
            if claimed is None:
                check(False, f"{path}::{key} has no `# observed` annotation "
                             f"(it runs {format_duration(actual)})")
                stale += 1
                continue
            seconds = parse_duration(claimed)
            if seconds is None:
                check(False, f"{path}::{key} annotation {claimed!r} is not a duration")
                stale += 1
                continue
            drifted = has_drifted(seconds, actual)
            check(not drifted,
                  f"{path}::{key} `# observed {claimed}` vs measured "
                  f"{format_duration(actual)}")
            stale += drifted
    if stale:
        print(f"\n  {stale} annotation(s) are stale. They are generated, not "
              f"hand-written:\n      python3 scripts/check-ci-config.py "
              f"--update-observed\n  then commit the result.")


def update_observed(jobs: list[tuple[str, str, str, int]],
                    runtimes: dict[str, dict[str, list[float]]]) -> int:
    """Rewrite every `# observed` annotation from measured run history."""
    written = 0
    for path in sorted({p for p, _, _, _ in jobs}):
        lines = open(path).read().split("\n")
        changed = False
        for i, key, budget, claimed in annotation_lines(path):
            actual = median_seconds(path, key, jobs, runtimes)
            if actual is None:
                if claimed:
                    print(f"  keep  {path}::{key} # observed {claimed} (no recent run)")
                continue
            new_line = f"    timeout-minutes: {budget}  # observed {format_duration(actual)}"
            if lines[i] != new_line:
                print(f"  write {path}::{key} # observed {format_duration(actual)}"
                      f"{'' if claimed is None else f' (was {claimed})'}")
                lines[i] = new_line
                changed = True
        if changed:
            open(path, "w").write("\n".join(lines))
            written += 1
    return written


def read_runtimes(jobs: list[tuple[str, str, str, int]]) -> dict[str, dict[str, list[float]]] | None:
    """Recent run history for every workflow with a resolvable job, or None.

    None means the history could not be read at all, which the callers report
    as a broken check in CI and a skip outside it.
    """
    repo = os.environ.get("GITHUB_REPOSITORY")
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if not (repo and token):
        return None
    runtimes: dict[str, dict[str, list[float]]] = {}
    for path in sorted({p for p, _, _, _ in jobs}):
        try:
            runtimes[path] = observed_runtimes(path, repo)
        except (urllib.error.URLError, urllib.error.HTTPError, KeyError, ValueError) as e:
            skip(f"{path}: could not read run history ({e})")
    return runtimes or None


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    updating = "--update-observed" in argv

    jobs, unresolved = configured_jobs()

    if updating:
        # Regeneration reads run history and nothing else; the enumeration
        # checks below cost a cargo-mutants pass and have no bearing on it.
        runtimes = read_runtimes(jobs)
        if runtimes is None:
            print("--update-observed needs GITHUB_REPOSITORY and a token with actions: read")
            return 1
        changed = update_observed(jobs, runtimes)
        print(f"\n{changed} workflow file(s) updated" if changed
              else "\nevery annotation was already current")
        return 0

    check_timeouts()
    check_mutation_matrix()
    check_headroom_arithmetic()

    for job in unresolved:
        skip(f"{job}: name or timeout does not render without a matrix value")
    runtimes = read_runtimes(jobs)
    if runtimes is None:
        msg = "no GITHUB_REPOSITORY/GITHUB_TOKEN — run durations not read"
        # In CI the credentials are always present, so their absence is a
        # broken gate rather than an environment that simply lacks them.
        if os.environ.get("GITHUB_ACTIONS") == "true":
            check(False, f"checks 4 and 5 have the credentials they need ({msg})")
        else:
            skip(msg)
    else:
        check_duration_headroom(jobs, runtimes)
        check_observed_annotations(jobs, runtimes)

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
