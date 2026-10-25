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

     A job measured at or under a minute neither needs one nor is held to one
     (`DRIFT_FLOOR_SECONDS`): run to run, the 7-25 s jobs vary by 2-3x, and the
     check went red on `main` for `fmt # observed 18s` against 10s while every
     annotation that mattered was right. `check_observed_rule` runs the rule
     over fixtures in both directions on every run.

  6. A job that is not running the toolchain it is named for. The `msrv` job
     installed Rust 1.95 with `dtolnay/rust-toolchain@1.95`, which only runs
     `rustup default`; `rust-toolchain.toml` says `channel = "stable"`, and
     rustup ranks a toolchain file above the default. Every `cargo` in the job
     ran on the current stable — `rustup show active-toolchain` in the repo
     reads "stable ... overridden by rust-toolchain.toml" — so "MSRV (Rust 1.95)"
     was green for as long as stable compiled, whatever 1.95 would have said. The
     Docker builder has the same exposure: `rust:1.95-slim-trixie` plus a
     `COPY . .` that carries the toolchain file in. Found by reading rustup's
     precedence rules and then reproducing them; the code does build on 1.95
     (`cargo check --workspace --all-features --all-targets --locked`).

  7. An action reference that moves. `dtolnay/rust-toolchain` was used by
     branch name — `@stable` thirteen times, `@1.95` once — and the *ref* was
     also the choice of Rust, so the one thing that must not move silently (the
     MSRV job's compiler) and the one thing that is safe to leave alone (the
     action's own revision) were the same string. Dependabot proposed
     `@1.95` -> `@1.100`, a Rust that did not exist. Every use is now pinned to
     a commit and names its toolchain in a `toolchain:` input, and this reads
     every workflow to keep it so — a pin that nothing reads is a comment.

  8. A required check that stopped existing. Branch protection matches checks
     by name, so renaming a job silently drops the requirement, and a workflow
     skipped by a `paths:` filter leaves its checks pending and blocks every
     merge. `.github/workflows/README.md` lists the names `main` should require;
     this keeps that list true.

  9. A tool installed at whatever version is current. `cargo install
     cargo-audit`, `tool: cargo-mutants`, `tool: cargo-llvm-cov`, `pip install
     pyyaml` and the Dockerfile's `cargo install cargo-chef --version "^0.1"`
     each took the newest release on the day of the run, so the same commit
     could pass on Monday and fail on Tuesday for a reason that is nowhere in the
     diff. Every one is pinned to an exact version now (pip: to a hash, from
     `tools/ci/requirements-ci.txt`), and `npm install` in the
     browser-QA job is `npm ci`, which refuses a lockfile that disagrees with
     `package.json`.

 10. A fuzz target nothing runs. `fuzz/` held two harnesses for the parsers of
     untrusted input for months, run by no workflow; the first run, once there
     was one, crashed `decode_audio` inside a minute (see fuzz.yml and
     fuzz/known/). A target added to the harness without a line in the
     workflow's matrix is the same silence again.

 11. A lockfile the workflows name and the checkout does not contain.
     `tools/visual-qa/package-lock.json` was ignored by two `.gitignore` files
     ("installed on demand; never committed") while `osv.yml` scanned it and the
     browser-QA job ran `npm ci` against it. Everything passed on the machine
     that wrote them, because an ignored file is still on disk there; on GitHub
     the scan stopped at `stat .../package-lock.json: no such file or directory`
     and `npm ci` would have refused to run. A lockfile a workflow names has to
     be a file git tracks, not a file that happens to exist.

Run from the repository root. Requires PyYAML, and cargo-mutants for the
mutant-enumeration checks (`--list` builds nothing; it costs ~0.2 s per row).
Checks 4 and 5 additionally need `GITHUB_TOKEN` with `actions: read`; outside
CI they say loudly that they are skipping rather than passing silently.
"""

from __future__ import annotations

import contextlib
import datetime as dt
import glob
import io
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from math import ceil

import yaml

MUTATION_WORKFLOW = ".github/workflows/mutation.yml"
CI_WORKFLOW = ".github/workflows/ci.yml"
RELEASE_WORKFLOW = ".github/workflows/release.yml"
DOCS_WORKFLOW = ".github/workflows/docs.yml"

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

# Recent completed runs sampled per workflow, on the default branch only (see
# `observed_runtimes`). The *worst* run in the sample is what gets compared: a
# cold cache or a contended pool is exactly the condition that tips a job over
# its budget, so it is signal, not noise — but only when it is main's cold
# cache. A pull request's own runs are a different distribution and were
# drowning this sample.
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
    """Enumerate mutants for one matrix row. `--list` does not build.

    A `cargo` that fails lists nothing, and an empty list reads as "this path
    matches no source". It once failed for every row at once — on a runner
    whose first `cargo` call was also installing the toolchain — and the log
    said only "generates 0 mutants", ten times, with the reason discarded. So
    a failed command is reported as one, with what `cargo` said.
    """
    cmd = ["cargo", "mutants", "--list", "--package", package,
           "--file", file_glob, "--no-shuffle"]
    if shard:
        cmd += ["--shard", shard]
    out = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if out.returncode != 0:
        # The cause leads; a backtrace may follow it, so stop there.
        lines = out.stderr.strip().splitlines()
        cut = next((i for i, ln in enumerate(lines) if ln.startswith("Stack backtrace")), len(lines))
        said = "\n        ".join(lines[:min(cut, 10)]) or "(no stderr)"
        check(False, f"`{' '.join(cmd)}` exited {out.returncode}:\n        {said}")
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


_DEFAULT_BRANCH: dict[str, str] = {}


def default_branch(repo: str) -> str:
    """The repository's default branch, looked up once."""
    if repo not in _DEFAULT_BRANCH:
        _DEFAULT_BRANCH[repo] = _api(f"/repos/{repo}").get("default_branch") or "main"
    return _DEFAULT_BRANCH[repo]


def runs_query(basename: str, repo: str, branch: str, sampled: int = RUNS_SAMPLED) -> str:
    """The API path for a workflow's recent completed runs on one branch.

    Split out from [`observed_runtimes`] so the `branch=` scoping is testable
    without a network call — it is the whole point of the function and it was
    absent for long enough to cost real time (see the note there).
    """
    return (
        f"/repos/{repo}/actions/workflows/{basename}/runs"
        f"?status=completed&branch={urllib.parse.quote(branch, safe='')}"
        f"&per_page={sampled}"
    )


def _minutes(job: dict) -> float | None:
    start, end = job.get("started_at"), job.get("completed_at")
    if not start or not end or job.get("conclusion") == "skipped":
        return None
    fmt = "%Y-%m-%dT%H:%M:%SZ"
    return (dt.datetime.strptime(end, fmt) - dt.datetime.strptime(start, fmt)).total_seconds() / 60


# A cancelled job that got this close to its declared budget ran out its own
# clock rather than being killed from outside.
SELF_TIMEOUT_FRACTION = 0.98


def budget_for(rendered: str, budgets: dict[str, int]) -> int | None:
    """The declared budget for a rendered job name.

    Mirrors [`lookup_observed`]'s matching, for the same reason: GitHub renders
    a matrix job with no matrix reference in its `name:` as `Name (values)`,
    and those rows share one `timeout-minutes:` line.
    """
    if rendered in budgets:
        return budgets[rendered]
    for name, budget in budgets.items():
        if rendered.startswith(f"{name} ("):
            return budget
    return None


def cut_short(job: dict, budget_minutes: int | None) -> bool:
    """Was this job killed from outside, rather than finishing or timing out?

    Such a job has a `started_at` and a `completed_at` and looks exactly like a
    job that ran that long — but the elapsed time records when somebody pushed
    again, not what the job costs. Three pushes to one pull request in an hour
    put two superseded runs into a five-run window and turned twelve rows red
    at once, every one reading "used 45m of its 45m budget" for jobs that
    really take 15. Any contributor who pushes twice reproduces it.

    The hard part is that GitHub records **both** cases as a cancelled job:
    one killed by `cancel-in-progress`, and one that exhausted its own
    `timeout-minutes` — and the second is precisely the incident check 4 exists
    to catch, so it must survive.

    The run's conclusion does not separate them, which was learned the
    expensive way. It looked like it did: run 32185298991 concluded `failure`
    with `civil.rs` cancelled at 45m of 45m, so "run cancelled" seemed to mean
    "superseded". It does not — that run concluded `failure` because a
    *different* job failed in it. Run 32300205290, where `Rustdoc` ran out its
    own 30-minute clock and nothing else went wrong, concluded `cancelled`.
    Filtering on the run would have thrown that away.

    What does separate them is the job's own clock against its own budget. A
    job that timed out sits at essentially 100% of it; a job killed by a newer
    push is wherever it happened to be. So a cancelled job is dropped only when
    it is comfortably short of its budget.

    An unknown budget keeps the sample. Over-reporting is the safe direction:
    a false alarm gets read, a hidden timeout does not.
    """
    if job.get("conclusion") != "cancelled" or budget_minutes is None:
        return False
    mins = _minutes(job)
    return mins is not None and mins < budget_minutes * SELF_TIMEOUT_FRACTION


def observed_runtimes(workflow_file: str, repo: str,
                      budgets: dict[str, int] | None = None) -> dict[str, list[float]]:
    """Every recent run's wall-clock, in minutes, per rendered job name.

    Returned as the raw sample rather than one number, because the two
    questions asked of it want different statistics: whether a job is about to
    be cancelled is a question about its *worst* run, and whether an `observed`
    annotation still describes it is a question about its *typical* one.

    Sampled from the **default branch only**. Without that filter this asked
    for the last N runs of the workflow on *any* ref, which on a repository
    with an active pull request means almost entirely that PR's runs — and a
    branch's runs are not representative of the same job on main. A new branch
    compiles cold on its first runs and hits a warm `rust-cache` afterwards, so
    the same job was sampled at 8m52s and at 58s within one hour, and no
    annotation could be true of both. Every regenerated value was stale again
    within minutes, and `--update-observed` run from a branch wrote main's
    config full of that branch's numbers.

    This narrows check 4's sample too, which is a deliberate trade: cold-cache
    runs are the ones that tip a job over its budget (see `RUNS_SAMPLED`), and
    main still has those — after a cache eviction, and on every merge commit
    that invalidates it. What it loses is unrepresentative branch noise; what
    it keeps is the branch whose timeouts actually matter.

    Jobs killed from outside are skipped — see [`cut_short`].
    """
    budgets = budgets or {}
    basename = os.path.basename(workflow_file)
    runs = _api(runs_query(basename, repo, default_branch(repo)))
    seen: dict[str, list[float]] = {}
    for run in runs.get("workflow_runs", []):
        for job in _api(f"/repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100").get("jobs", []):
            if cut_short(job, budget_for(job["name"], budgets)):
                continue
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

# Below this a job's duration is not something an annotation can usefully be
# wrong about, so it is neither required nor compared.
#
# What the annotation is for: `timeout-minutes` is a whole number of minutes,
# sized against how long the job really takes, and a reader of the file should
# be able to see how far apart the two are. Incident 5 was annotations that
# said 54s for 8m52s and 10m42s for 22m37s — a budget justified by a figure
# that was never true. A job that takes under a minute has no budget that
# figure could justify (the smallest is 60s) and no growth toward one to hide,
# and it is the one kind of job whose run time is mostly noise: on main, in
# the five-run window, `fmt` took 9, 9, 10, 16 and 23s, `cargo-machete` 7 and
# 19s, `shellcheck` 13 and 20s, `third-party licence notices` 13 and 23s — up to
# 2.7x between two runs — while `cargo-deny` (50, 61s) and `cargo-audit` (146,
# 177s) sat within 1.2x of themselves. At 1.5x the short ones could not hold an
# annotation still: four of the nine most recent `CI config` jobs on main went
# red. Two of them (15 and 19 October) failed on nothing but jobs under a
# minute — `fmt # observed 18s` against 10s, `cargo-machete 15s` against 9s,
# and `osv`, `scorecard`, `third-party licence notices` and `publishable`
# (15-42s) having no annotation yet. The other two also carried real findings
# (long jobs drifted by 1.5-6x, a 14-minute job with no annotation, a shard at
# 77% of its budget), which this leaves alone.
#
# So the line is one minute: a job whose measured median is at or under it
# needs no annotation, and an annotation is held to the tolerance only when
# the claim or the measurement is over it. 8m for a 20s job and 5s for a 90s
# job are still findings; 18s for a 10s job is not. The headroom check
# (incident 4) is untouched — it compares worst runs against the budget and has
# no such noise problem.
DRIFT_FLOOR_SECONDS = 60


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

    Two durations that are both at or under [`DRIFT_FLOOR_SECONDS`] have not
    drifted: the claim cannot mislead about a budget of whole minutes, and the
    measurement is noise at that size. The floor is read here, at call time.
    """
    if max(claimed_seconds, actual_seconds) <= DRIFT_FLOOR_SECONDS:
        return False
    if min(claimed_seconds, actual_seconds) <= 0:
        return claimed_seconds != actual_seconds
    return max(claimed_seconds, actual_seconds) / min(claimed_seconds, actual_seconds) > tolerance


def annotation_verdict(claimed: str | None, actual_seconds: float) -> str:
    """What is wrong, if anything, with one job's `# observed` annotation.

    `ok`, or one of `missing` (the job takes longer than the floor and says
    nothing), `unreadable` (the text is not a duration) or `drifted`. A job at
    or under the floor owes no annotation; one it has is judged by
    [`has_drifted`]. Shared by the check, the regenerator and the self-test, so
    the three cannot disagree about what a finding is.
    """
    if claimed is None:
        return "missing" if actual_seconds > DRIFT_FLOOR_SECONDS else "ok"
    seconds = parse_duration(claimed)
    if seconds is None:
        return "unreadable"
    return "drifted" if has_drifted(seconds, actual_seconds) else "ok"


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


def check_embed_precedes_tests() -> None:
    """The extension embed must happen before anything that tests for it.

    `BIRDNET_REQUIRE_LIVE_EXTENSION` is set for the whole `test` job on pushes
    to `main`, turning "the extension is missing, so skip" into a failure. The
    two steps that fetch and embed the extensions used to sit *after*
    `--lib --bins`, `--tests` and `--doc`.

    `connection::tests` compiles under the `analytics` feature, and a workspace
    build turns that on by unification, so those two tests ran in the very first
    step — skipping forbidden, nothing embedded yet. They could only fail, and
    main was red from the merge that introduced the gate onward:

        test result: FAILED. 87 passed; 2 failed

    Reproduced exactly, and both ways, before this check was written:

        BIRDNET_REQUIRE_LIVE_EXTENSION=1 \
          cargo test -p birdnet-behavioral --lib --features analytics
        -> 87 passed; 2 failed          (the CI failure, locally)

        ... plus BIRDNET_BUNDLED_{EXTENSION,ICU}_FILE pointing at the CDN files
        -> 89 passed; 0 failed          (both tests assert, and pass)

    The fix was ordering. This is what keeps it ordered: a comment saying "this
    step must come first" is the shape of every incident this script exists for.
    """
    print("\nThe extension embed runs before the tests that require it")
    spec = yaml.safe_load(open(CI_WORKFLOW))
    steps = spec["jobs"]["test"]["steps"]
    names = [str(st.get("name", "")) for st in steps]

    def first(pred: "callable[[str], bool]", what: str) -> int | None:
        for i, n in enumerate(names):
            if pred(n):
                return i
        check(False, f"the test job still has a step that {what}")
        return None

    last_embed = None
    for i, n in enumerate(names):
        if n.startswith("Embed the "):
            last_embed = i
    if last_embed is None:
        check(False, "the test job still has 'Embed the ...' steps")
        return

    first_test = first(lambda n: n.startswith("Run ") and "tests" in n,
                       "runs tests")
    if first_test is None:
        return

    check(last_embed < first_test,
          f"every embed step (last at #{last_embed + 1} {names[last_embed]!r}) "
          f"precedes the first test step (#{first_test + 1} "
          f"{names[first_test]!r})")

    # The counterpart. Every assertion above is satisfied by a job with no
    # embed steps and no test steps, or by a `names` list this function failed
    # to read — so pin that both kinds are actually present and distinct.
    embeds = [n for n in names if n.startswith("Embed the ")]
    tests = [n for n in names if n.startswith("Run ") and "tests" in n]
    check(len(embeds) == 2,
          f"both extensions are embedded (found {len(embeds)}: {embeds})")
    check(len(tests) >= 3,
          f"the job runs its several test steps (found {len(tests)}: {tests})")


def check_release_gate_runs_the_toolchain_it_names(release_path: str = RELEASE_WORKFLOW) -> None:
    """release.yml's `ci` matrix must put the toolchain it names in charge.

    Incident 6 again, in the copy of the gate that blocks a release. The `ci` job
    installs `matrix.toolchain` with `dtolnay/rust-toolchain`, which only sets
    rustup's *default*; `rust-toolchain.toml` (`channel = "stable"`) outranks a
    default, so the `msrv` entry (toolchain "1.95") ran `cargo check` on stable and
    the release's MSRV gate was green while testing nothing it promised.
    `check_msrv_runs_the_msrv` reads `ci.yml` only and could not see this.

    Structural, as there: a step exports `RUSTUP_TOOLCHAIN` from `matrix.toolchain`
    through `$GITHUB_ENV`, a later step reads `rustc --version` back against it, and
    both come before the step that runs the matrix command.
    """
    print("\nThe release gate runs the toolchain each entry names (incident 6)")
    spec = yaml.safe_load(open(release_path))
    steps = spec["jobs"]["ci"]["steps"]
    run = lambda st: str(st.get("run", ""))  # noqa: E731

    def index_of(pred: "callable[[dict], bool]") -> int | None:
        for i, st in enumerate(steps):
            if pred(st):
                return i
        return None

    export = index_of(
        lambda st: "RUSTUP_TOOLCHAIN=" in run(st)
        and "GITHUB_ENV" in run(st)
        and "matrix.toolchain" in run(st)
    )
    prove = index_of(lambda st: "rustc --version" in run(st) and "RUSTUP_TOOLCHAIN" in run(st))
    matrix_cmd = index_of(lambda st: "matrix.command" in run(st))
    check(export is not None,
          "a step exports RUSTUP_TOOLCHAIN=matrix.toolchain through $GITHUB_ENV")
    check(prove is not None, "a step asserts `rustc --version` against RUSTUP_TOOLCHAIN")
    check(matrix_cmd is not None, "the job still runs the matrix command")
    if None not in (export, prove, matrix_cmd):
        check(export < prove < matrix_cmd,
              f"export (#{export + 1}) precedes the proof (#{prove + 1}), which "
              f"precedes the matrix command (#{matrix_cmd + 1})")
    msrv = [m for m in spec["jobs"]["ci"]["strategy"]["matrix"]["include"] if m["name"] == "msrv"]
    declared = None
    for line in open("Cargo.toml"):
        m = re.match(r'\s*rust-version\s*=\s*"([^"]+)"', line)
        if m:
            declared = m.group(1)
            break
    check(len(msrv) == 1 and str(msrv[0]["toolchain"]) == declared,
          f"the msrv entry names the declared rust-version ({declared}); "
          f"found {[m['toolchain'] for m in msrv]}")


def check_msrv_runs_the_msrv(
    ci_path: str = CI_WORKFLOW,
    dockerignore_path: str = ".dockerignore",
    toolchain_path: str = "rust-toolchain.toml",
) -> None:
    """The `msrv` job must put the declared MSRV in charge, and prove it did.

    See incident 6 in the module docstring. Three assertions, structural rather
    than textual, because a comment explaining the fix is the shape of what this
    script exists to distrust:

      * an `msrv` step writes `RUSTUP_TOOLCHAIN` to `$GITHUB_ENV` — the only
        thing that outranks `rust-toolchain.toml`;
      * a *later* step asserts `rustc --version` against it, so the export is
        read back from a fresh process rather than assumed;
      * both come before the step that runs `cargo check`.

    And the counterpart that makes them matter: while `rust-toolchain.toml`
    names `stable`, the file does override the default, and the Docker build
    context must not carry it into a builder pinned to the MSRV.
    """
    print("\nThe MSRV job runs the MSRV, not `stable` (incident 6)")
    spec = yaml.safe_load(open(ci_path))
    steps = spec["jobs"]["msrv"]["steps"]

    def index_of(pred: "callable[[dict], bool]") -> int | None:
        for i, st in enumerate(steps):
            if pred(st):
                return i
        return None

    run = lambda st: str(st.get("run", ""))  # noqa: E731
    export = index_of(lambda st: "RUSTUP_TOOLCHAIN=" in run(st) and "GITHUB_ENV" in run(st))
    prove = index_of(lambda st: "rustc --version" in run(st) and "RUSTUP_TOOLCHAIN" in run(st))
    cargo = index_of(lambda st: "cargo check" in run(st))

    check(export is not None,
          "a step exports RUSTUP_TOOLCHAIN through $GITHUB_ENV (the only setting "
          "that outranks rust-toolchain.toml)")
    check(prove is not None,
          "a step asserts `rustc --version` against RUSTUP_TOOLCHAIN")
    check(cargo is not None, "the job still has the `cargo check` step")
    if None not in (export, prove, cargo):
        check(export < prove < cargo,
              f"export (#{export + 1}) precedes the proof (#{prove + 1}), which "
              f"precedes the cargo step (#{cargo + 1})")

    # The counterpart: this is only a problem while the toolchain file pins
    # `stable`. If it ever pins the MSRV itself the file is doing the job and
    # the export is redundant, which this should say rather than fail on.
    channel = None
    for line in open(toolchain_path):
        m = re.match(r'\s*channel\s*=\s*"([^"]+)"', line)
        if m:
            channel = m.group(1)
    check(channel is not None, f"{toolchain_path} names a channel (found {channel!r})")

    # The Docker builder is the other place the MSRV has to be what it says: the
    # image compiles with the oldest compiler the project promises to support.
    declared = None
    for line in open("Cargo.toml"):
        m = re.match(r'\s*rust-version\s*=\s*"([^"]+)"', line)
        if m:
            declared = m.group(1)
            break
    builder = None
    if os.path.exists("Dockerfile"):
        for line in open("Dockerfile"):
            m = re.match(r"^FROM\s+rust:(\d+\.\d+)\b", line)
            if m:
                builder = m.group(1)
                break
    check(declared is not None and builder == declared,
          f"the Docker builder's Rust ({builder}) is the declared rust-version ({declared})")

    ignored = [ln.strip() for ln in open(dockerignore_path)
               if ln.strip() and not ln.lstrip().startswith("#")]
    check("rust-toolchain.toml" in ignored,
          ".dockerignore keeps rust-toolchain.toml out of the build context, so "
          "the builder image's pinned Rust is the one that compiles")


def runs_on_every_pull_request(spec: dict) -> bool:
    """A workflow with a `pull_request` trigger and no path filter on it."""
    on = spec.get(True, spec.get("on"))  # PyYAML reads the key `on` as boolean True
    if not isinstance(on, dict) or "pull_request" not in on:
        return False
    trigger = on["pull_request"]
    return not (isinstance(trigger, dict) and ("paths" in trigger or "paths-ignore" in trigger))


def requirements_are_hashed(path: str) -> bool:
    """Whether the requirements file at `path` has at least one entry and every
    entry is `name==version` followed by one or more `--hash=sha256:<64 hex>`.

    Continuation lines are joined and comments dropped, as pip reads them. A
    missing file is not hashed: the install it feeds would fail, and so would
    the job.
    """
    try:
        text = open(path, encoding="utf-8").read()
    except OSError:
        return False
    entries = [e.strip() for e in re.sub(r"\\\n", " ", text).splitlines()]
    entries = [e for e in (re.sub(r"(^|\s)#.*", "", e).strip() for e in entries) if e]
    pattern = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]*==[^\s=;]+(\s+--hash=sha256:[0-9a-f]{64})+")
    return bool(entries) and all(pattern.fullmatch(e) for e in entries)


def tool_installs(workflows_glob: str, dockerfile_path: str) -> list[tuple[str, str, bool]]:
    """Every tool install in the workflows and the Dockerfile: (where, what, pinned).

    Three shapes are recognised: `taiki-e/install-action`'s `tool:` list, where
    each entry needs `@version`; `cargo install`, which needs `--version` (or
    `crate@version`); and `pip install`, which must be `--require-hashes -r FILE`
    with a hash for every entry of FILE. Comment lines are not installs.
    """
    found: list[tuple[str, str, bool]] = []

    def scan_shell(where: str, text: str) -> None:
        for raw in text.splitlines():
            line = raw.strip()
            if line.startswith("#"):
                continue
            if re.search(r"\bcargo install\b", line):
                # An exact version: a range (`^0.1`, `~0.1`) floats within it.
                pinned = bool(re.search(r"--version[ =]\"?=?\d+\.\d+\.\d+\"?(\s|$)|\s[\w-]+@\d+\.\d+\.\d+(\s|$)", line))
                found.append((where, line, pinned))
            m = re.search(r"\bpip3? install\b(.*)", line)
            if m:
                args = m.group(1).split()
                # A version is not the bytes: `pkg==1.2.3` still installs whatever
                # file PyPI serves under that number. The only accepted form is
                # `--require-hashes -r FILE`, FILE holding `name==version` and a
                # sha256 for each (OpenSSF Scorecard's Pinned-Dependencies).
                for i, arg in enumerate(args):
                    if arg in ("-r", "--requirement") and i + 1 < len(args):
                        what = f"pip install -r {args[i + 1]}"
                        found.append((where, what, "--require-hashes" in args
                                      and requirements_are_hashed(args[i + 1])))
                    elif not arg.startswith("-") and (i == 0 or args[i - 1] not in ("-r", "--requirement")):
                        found.append((where, f"pip install {arg}", False))

    for path in sorted(glob.glob(workflows_glob)):
        for node in _walk_dicts(yaml.safe_load(open(path))):
            uses = node.get("uses")
            if isinstance(uses, str) and uses.startswith("taiki-e/install-action@"):
                tools = str((node.get("with") or {}).get("tool", ""))
                # Comma-separated, and an entry may contain spaces
                # (`mdbook@${{ env.MDBOOK_VERSION }}`).
                for tool in (t.strip() for t in tools.split(",")):
                    if tool:
                        found.append((path, f"install-action {tool}", "@" in tool))
            if isinstance(node.get("run"), str):
                scan_shell(path, node["run"])
    if os.path.exists(dockerfile_path):
        scan_shell(dockerfile_path, open(dockerfile_path).read())
    return found


def _walk_dicts(node: object):
    if isinstance(node, dict):
        yield node
        for value in node.values():
            yield from _walk_dicts(value)
    elif isinstance(node, list):
        for value in node:
            yield from _walk_dicts(value)


def check_tools_are_pinned(
    workflows_glob: str = ".github/workflows/*.yml", dockerfile_path: str = "Dockerfile"
) -> None:
    """Every tool the CI installs names an exact version (incident 9)."""
    print("\nEvery installed tool is pinned to a version (incident 9)")
    installs = tool_installs(workflows_glob, dockerfile_path)
    for where, what, pinned in installs:
        check(pinned, f"{where}: {what} is pinned")
    # The counterpart: a reader that found no installs would pass every
    # assertion above.
    check(len(installs) >= 6, f"the reader found the installs it is judging ({len(installs)})")

    # Base images are tools too: a tag moves, a digest does not. A stage that
    # builds `FROM` an earlier stage names no image.
    stages: set[str] = set()
    images = 0
    if os.path.exists(dockerfile_path):
        for line in open(dockerfile_path):
            m = re.match(r"^FROM\s+(?:--\S+\s+)?(\S+)(?:\s+AS\s+(\S+))?", line, re.I)
            if not m:
                continue
            image, stage = m.group(1), m.group(2)
            if image not in stages:
                images += 1
                check(bool(re.search(r"@sha256:[0-9a-f]{64}$", image)),
                      f"{dockerfile_path}: base image {image} is pinned to a digest")
            if stage:
                stages.add(stage)
    if os.path.exists(dockerfile_path):
        check(images >= 2, f"the reader found the base images it is judging ({images})")


def named_lockfiles(workflows_glob: str) -> list[tuple[str, str]]:
    """`(where, path)` for every lockfile a workflow relies on: the `--lockfile=`
    arguments of a scan (less a `parser:` prefix), and the `package-lock.json`
    beside every `npm ci`."""
    found: list[tuple[str, str]] = []
    for workflow in sorted(glob.glob(workflows_glob)):
        text = open(workflow, encoding="utf-8").read()
        for path in re.findall(r"--lockfile=(\S+)", text):
            # `requirements.txt:tools/x/requirements.txt` names the parser, then the file.
            found.append((workflow, re.sub(r"^[A-Za-z][\w.-]*:", "", path)))
        for node in _walk_dicts(yaml.safe_load(text)):
            run = node.get("run")
            if isinstance(run, str) and re.search(r"\bnpm ci\b", run):
                directory = node.get("working-directory", ".")
                found.append((workflow, os.path.normpath(os.path.join(directory, "package-lock.json"))))
    return found


def is_tracked(path: str) -> bool:
    """Whether git tracks `path`: an ignored file that exists on this disk is not
    the file a fresh checkout has."""
    return subprocess.run(["git", "ls-files", "--error-unmatch", "--", path],
                          capture_output=True).returncode == 0


def check_named_lockfiles_are_tracked(workflows_glob: str = ".github/workflows/*.yml") -> None:
    """Every lockfile a workflow names is tracked by git (incident 11)."""
    print("\nEvery lockfile a workflow names is tracked, not just present (incident 11)")
    named = named_lockfiles(workflows_glob)
    for where, path in named:
        check(is_tracked(path), f"{where}: {path} is tracked by git")
    # The counterpart: a reader that found nothing would pass every line above.
    check(len(named) >= 3, f"the reader found the lockfiles it is judging ({len(named)})")


def check_fuzz_targets_are_run(
    cargo_path: str = "fuzz/Cargo.toml", workflow_path: str = ".github/workflows/fuzz.yml"
) -> None:
    """Every fuzz target is run by the fuzz workflow, and it runs nothing else.

    The two targets existed for months and no workflow ran either, so the first
    run found a crash inside a minute (see fuzz.yml). A target added to the
    harness without a line in the matrix would be the same silence again.
    """
    print("\nEvery fuzz target is in the fuzz workflow's matrix")
    declared = set()
    in_bin = False
    for line in open(cargo_path):
        if line.strip() == "[[bin]]":
            in_bin = True
            continue
        m = re.match(r'\s*name\s*=\s*"([^"]+)"', line)
        if in_bin and m:
            declared.add(m.group(1))
            in_bin = False
    spec = yaml.safe_load(open(workflow_path))
    run = set(spec["jobs"]["fuzz"]["strategy"]["matrix"]["target"])
    for name in sorted(declared - run):
        check(False, f"{cargo_path} declares fuzz target {name!r}, which {workflow_path} never runs")
    for name in sorted(run - declared):
        check(False, f"{workflow_path} runs {name!r}, which {cargo_path} does not declare")
    check(len(declared) >= 2 and declared == run,
          f"the {len(declared)} declared fuzz targets are exactly the {len(run)} the workflow runs")


def check_workflows_readme(
    workflows_glob: str = ".github/workflows/*.yml",
    readme_path: str = ".github/workflows/README.md",
) -> None:
    """The workflows README lists every workflow, and its required checks exist.

    A required status check is matched by *name*. Renaming a job drops the
    requirement without a word, and a workflow skipped by a `paths:` filter
    leaves its checks pending forever, so a required name must belong to a job
    of a workflow that runs on every pull request. This reads the README the way
    branch protection would read the repository.
    """
    print("\nThe workflows README lists every workflow, and its required checks are real jobs")
    text = open(readme_path).read()
    files = sorted(glob.glob(workflows_glob))

    listed = set(re.findall(r"^\| `([^`]+\.yml)` \|", text, re.M))
    for path in files:
        base = os.path.basename(path)
        check(base in listed, f"{base} has a row in the workflows README")
    for name in sorted(listed - {os.path.basename(p) for p in files}):
        check(False, f"the workflows README lists {name}, which is not a workflow")

    section = text.split("## Required status checks", 1)
    check(len(section) == 2, "the README has a 'Required status checks' section")
    required = re.findall(r"^- `([^`]+)`$", section[1], re.M) if len(section) == 2 else []

    always = {}
    for path in files:
        spec = yaml.safe_load(open(path))
        if runs_on_every_pull_request(spec):
            for job in spec["jobs"].values():
                always[str(job.get("name"))] = os.path.basename(path)
    for name in required:
        check(name in always,
              f"required check {name!r} is a job of a workflow that runs on every pull request")
    # The counterpart: a reader that found no list, or no always-on jobs, would
    # pass every assertion above.
    check(len(required) >= 10 and len(always) >= 10,
          f"the reader found {len(required)} required checks and {len(always)} always-on jobs")


PINNED = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")


def action_uses(node: object, found: list[tuple[str, dict]] | None = None
                ) -> list[tuple[str, dict]]:
    """Every remote `uses:` in a parsed workflow, with its `with:` inputs.

    Walks the whole document rather than `jobs.*.steps`, so a reusable-workflow
    call (`uses:` on a job) and a composite action's steps are read the same
    way. Local (`./...`) and `docker://` references are not remote refs.
    """
    found = [] if found is None else found
    if isinstance(node, dict):
        ref = node.get("uses")
        if isinstance(ref, str) and not ref.startswith(("./", "docker://")):
            inputs = node.get("with")
            found.append((ref, inputs if isinstance(inputs, dict) else {}))
        for value in node.values():
            action_uses(value, found)
    elif isinstance(node, list):
        for value in node:
            action_uses(value, found)
    return found


def check_actions_are_pinned(workflows_glob: str = ".github/workflows/*.yml") -> None:
    """Every remote action is a full commit SHA, and the toolchain is named.

    See incident 7 in the module docstring. Two assertions over every workflow:

      * each `uses:` ends in `@<40 hex>` — a tag or branch can be moved by
        whoever controls the action, and these jobs build what ships;
      * each `dtolnay/rust-toolchain` has a `toolchain:` input, because with the
        ref no longer selecting the Rust, a use without one asks the action to
        guess.

    And the counterpart: the reader must have found the uses it is judging.
    """
    print("\nEvery action is pinned to a commit, and the toolchain is named (incident 7)")
    files = sorted(glob.glob(workflows_glob))
    seen = 0
    toolchain_uses = 0
    for path in files:
        for ref, inputs in action_uses(yaml.safe_load(open(path))):
            seen += 1
            check(PINNED.match(ref) is not None, f"{path}: {ref} is pinned to a commit")
            if ref.startswith("dtolnay/rust-toolchain@"):
                toolchain_uses += 1
                check(bool(inputs.get("toolchain")),
                      f"{path}: {ref.split('@')[0]} names its toolchain")
    check(seen >= 50 and toolchain_uses >= 15,
          f"the reader found the uses it is judging ({seen} remote uses in "
          f"{len(files)} workflows, {toolchain_uses} of them dtolnay/rust-toolchain)")


def check_docs_publish_main_as_dev(docs_path: str = DOCS_WORKFLOW) -> None:
    """docs.yml builds `/dev/` from main on a tag push, and from the PR on a PR.

    `scripts/build-versioned-docs.sh` built `/dev/` from HEAD. On a release-tag
    push HEAD is the tag, so the site's "development" manual was the release's
    own until the next push to `main`. The script now takes `DOCS_DEV_REF`
    (default `origin/main`) and the workflow decides per event: a pull request
    builds its own tree, because that build exists to grade the change and
    deploys nothing, and everything else builds main.

    Both halves matter and either alone is a silent failure: a workflow that
    sets nothing is correct only because the script's default is, and one that
    sets `HEAD` everywhere is the original bug. So the expression is read back
    for each kind of event rather than checked for being present.
    """
    print("\ndocs.yml publishes main as /dev/ and gates a pull request on its own tree")
    spec = yaml.safe_load(open(docs_path))
    steps = spec["jobs"]["build"]["steps"]
    builds = [st for st in steps
              if re.search(r"(?<!test-)build-versioned-docs\.sh", str(st.get("run", "")))]
    check(len(builds) == 1, f"{docs_path} has one step that runs build-versioned-docs.sh "
                            f"(found {len(builds)})")
    if len(builds) != 1:
        return
    expr = str((builds[0].get("env") or {}).get("DOCS_DEV_REF", ""))
    m = re.fullmatch(
        r"\$\{\{\s*github\.event_name == 'pull_request' && '([^']+)' \|\| '([^']+)'\s*\}\}", expr)
    check(m is not None,
          f"DOCS_DEV_REF is `<pull_request> && 'X' || 'Y'` (found {expr!r})")
    if m is not None:
        check(m.group(1) == "HEAD",
              f"a pull request builds /dev/ from its own checkout (found {m.group(1)!r})")
        check(m.group(2) == "origin/main",
              f"every other event builds /dev/ from origin/main (found {m.group(2)!r})")
    # The counterpart: the self-test must still be wired in front of the build.
    check(any("test-build-versioned-docs.sh" in str(st.get("run", "")) for st in steps),
          f"{docs_path} runs the script's self-test before the real build")


def check_release_notes_link_the_versions_manual(release_path: str = RELEASE_WORKFLOW) -> None:
    """A stable release's body links the manual built from that tag.

    docs.yml publishes `/vX.Y.Z/` for every stable tag, and nothing pointed a
    reader of the Release at it: the notes carried install and verification
    steps and no link to the manual they install. The link is written inside the
    tail block that the step measures before it budgets the changelog, so it
    cannot push the body over GitHub's 125,000-character limit, and it is
    guarded on `is_prerelease` because docs.yml builds nothing for `vX.Y.Z-rc.N`
    and the link would 404.
    """
    print("\nA stable release's notes link that version's own manual")
    spec = yaml.safe_load(open(release_path))
    steps = spec["jobs"]["github-release"]["steps"]
    run = next((str(st.get("run", "")) for st in steps
                if st.get("name") == "Assemble final release notes"), "")
    check(bool(run), "release.yml has the 'Assemble final release notes' step")
    if not run:
        return
    check(re.search(r"DOCS_URL=\"https://[^\"]*github\.io/[^\"]*/v\$\{VERSION\}/\"", run) is not None,
          "the step builds the version's manual URL (…github.io/<repo>/v${VERSION}/)")
    guarded = re.search(
        r'if \[\[ "\$IS_PRE" != "true" \]\]; then\s+echo "## Documentation".*?\$\{DOCS_URL\}.*?\n\s*fi\b',
        run, re.S)
    check(guarded is not None,
          "the link sits in a '## Documentation' section guarded on a stable release")
    # It must be inside the block written to tail_notes.md, the part the step
    # measures, not appended after the budget was decided.
    tail = run.split("} > tail_notes.md", 1)[0]
    check("${DOCS_URL}" in tail,
          "the link is inside tail_notes.md, so the changelog's budget accounts for it")


def check_runners_are_pinned(workflows_glob: str = ".github/workflows/*.yml") -> None:
    """No job runs on a `-latest` runner image.

    `ubuntu-latest` moves to a new Ubuntu on GitHub's schedule, and a toolchain
    that built yesterday can fail tomorrow with nothing in the diff — incident 9,
    one level down. docs.yml was the one workflow still on it (every other job
    names `ubuntu-24.04`, or `macos-14`, or a matrix value).
    """
    print("\nNo job runs on a moving runner image (incident 9)")
    seen = 0
    for path in sorted(glob.glob(workflows_glob)):
        for name, job in (yaml.safe_load(open(path)).get("jobs") or {}).items():
            target = job.get("runs-on")
            if not isinstance(target, str):
                continue
            seen += 1
            check(not target.endswith("-latest"),
                  f"{path}::{name} runs on {target!r}, not a -latest image")
    check(seen >= 20, f"the reader found the runners it is judging ({seen})")


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
    # The branch scoping is the point of `runs_query`, and it was absent long
    # enough to make every regenerated annotation stale within minutes.
    q = runs_query("ci.yml", "o/r", "main", sampled=5)
    check("branch=main" in q, "runs_query scopes the sample to a branch")
    check(q.startswith("/repos/o/r/actions/workflows/ci.yml/runs?"),
          "runs_query addresses the right workflow")
    check("status=completed" in q and "per_page=5" in q,
          "runs_query keeps the completed filter and the sample size")
    check("branch=feature%2Fx" in runs_query("ci.yml", "o/r", "feature/x"),
          "a branch name with a slash is percent-encoded")

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
    # A cancelled job: killed from outside, or out of its own time?
    def at(mins: float, conclusion: str = "cancelled") -> dict:
        return {"conclusion": conclusion,
                "started_at": "2026-01-01T00:00:00Z",
                "completed_at": f"2026-01-01T{int(mins) // 60:02d}:{int(mins) % 60:02d}:00Z"}
    check(cut_short(at(20), 45), "a job killed at 20m of a 45m budget is dropped")
    check(not cut_short(at(45), 45),
          "a job that ran out its own 45m clock is kept — that is check 4's signal")
    check(not cut_short(at(30), 30), "Rustdoc at 30m of 30m is kept")
    check(not cut_short(at(20, "success"), 45), "a job that finished is kept")
    check(not cut_short(at(20, "failure"), 45), "a job that failed is kept")
    check(not cut_short(at(20), None), "an unknown budget keeps the sample")
    check(budget_for("Tests (x86_64)", {"Tests (x86_64)": 45}) == 45, "an exact budget matches")
    check(budget_for("Build (amd64)", {"Build": 20}) == 20, "a matrix-suffixed name finds its row")
    check(budget_for("Nope", {"Build": 20}) is None, "an unknown name has no budget")


def check_observed_rule() -> None:
    """The `# observed` rule, run over fixtures in both directions (incident 5).

    Needs no API access, so it runs everywhere the rest of the file does. The
    figures are real: the first group is what `main` measured on the jobs that
    kept failing the check for being a few seconds off (`fmt` 9-23s against an
    annotation of 18s, `cargo-machete` 15s against 9s) or for having no
    annotation yet (`OSV scan` at 26s); the second is what the check exists to
    catch — the annotations that had rotted by 2x to 10x (`Clippy` `54s` for a
    real 8m52s, `Tests` `10m42s` for 22m37s), a job that takes minutes and says
    nothing, and a claim far above a short job.

    Both groups are asserted. A rule that flagged everything would fail the
    first and one that flagged nothing would fail the second, so neither a blanket
    alarm nor a blanket pass satisfies this. Checked by setting
    `DRIFT_FLOOR_SECONDS = 0`, which is the rule as it was: the first group fails.
    """
    print("\n`# observed` annotations: noise on short jobs passes, rot does not (incident 5)")
    fixture = (
        "name: fixture\n"
        "jobs:\n"
        "  fmt:\n"
        "    name: Format (rustfmt)\n"
        "    timeout-minutes: 10  # observed 18s\n"
        "  machete:\n"
        "    name: cargo-machete\n"
        "    timeout-minutes: 10  # observed 15s\n"
        "  osv:\n"
        "    name: OSV scan\n"
        "    timeout-minutes: 10\n"
        "  tests:\n"
        "    name: Tests (x86_64)\n"
        "    timeout-minutes: 45  # observed 10m42s\n"
        "  clippy:\n"
        "    name: Clippy\n"
        "    timeout-minutes: 30  # observed 54s\n"
        "  audit:\n"
        "    name: cargo-audit\n"
        "    timeout-minutes: 20\n"
        "  tiny:\n"
        "    name: Tiny\n"
        "    timeout-minutes: 10  # observed 8m00s\n"
    )
    # Job name -> seconds per run (the last five completed runs on main).
    samples = {
        "Format (rustfmt)": [23, 9, 9, 10, 16],       # median 10s
        "cargo-machete": [19, 7, 9],                  # median 9s
        "OSV scan": [26],
        "Tests (x86_64)": [1357, 1357, 1357],         # 22m37s
        "Clippy": [532, 532, 532],                    # 8m52s
        "cargo-audit": [177, 146, 160],               # median 2m40s
        "Tiny": [20],
    }
    noise = {"fmt": "ok", "machete": "ok", "osv": "ok"}
    rot = {"tests": "drifted", "clippy": "drifted", "audit": "missing", "tiny": "drifted"}

    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "fixture.yml")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(fixture)
        jobs = [(path, key, name, 10)
                for key, name in [("fmt", "Format (rustfmt)"), ("machete", "cargo-machete"),
                                  ("osv", "OSV scan"), ("tests", "Tests (x86_64)"),
                                  ("clippy", "Clippy"), ("audit", "cargo-audit"),
                                  ("tiny", "Tiny")]]
        runtimes = {path: {name: [s / 60 for s in secs] for name, secs in samples.items()}}

        def verdicts() -> dict[str, str | None]:
            return {key: verdict for _p, key, _c, _a, verdict
                    in annotation_findings(jobs, runtimes)}

        got = verdicts()
        for key, want in noise.items():
            check(got.get(key) == want,
                  f"noise on a short job is not a finding: {key} -> {got.get(key)!r} (want {want!r})")
        for key, want in rot.items():
            check(got.get(key) == want,
                  f"rot is still a finding: {key} -> {got.get(key)!r} (want {want!r})")

        # The regenerator follows the same rule: it repairs what is wrong, and
        # leaves a short job's annotation (and its absence) as written.
        with contextlib.redirect_stdout(io.StringIO()):
            update_observed(jobs, runtimes)
        text = open(path, encoding="utf-8").read()
        check("timeout-minutes: 10  # observed 18s" in text and "OSV scan\n    timeout-minutes: 10\n" in text,
              "--update-observed leaves short jobs as written (no churn, none added)")
        check("# observed 22m37s" in text and "# observed 8m52s" in text
              and "# observed 2m40s" in text and "# observed 20s" in text,
              "--update-observed repairs the drifted and fills in the missing")
        check(all(v == "ok" for v in verdicts().values()),
              "after regeneration every annotation passes the rule it was written by")

    # The boundary, in terms of the constant so that retuning it moves both
    # sides together: at the floor nothing can have drifted, one second over it
    # the tolerance applies again.
    below, above = DRIFT_FLOOR_SECONDS, DRIFT_FLOOR_SECONDS + 1
    check(not has_drifted(5, below), f"5s against {below}s is not a finding (both under a minute)")
    check(has_drifted(5, above), f"5s against {above}s is (the measurement is over a minute)")
    check(has_drifted(below * 8, 20), "8 minutes claimed for a 20s job is (the claim is over a minute)")
    check(annotation_verdict(None, below) == "ok" and annotation_verdict(None, above) == "missing",
          "an annotation is owed above the floor, not at it")
    check(annotation_verdict("soon", 300) == "unreadable", "text that is not a duration is refused")
    check(has_drifted(54, 532) and has_drifted(642, 1357),
          "the two annotations that rotted in incident 5 are still caught")


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


def annotation_findings(
    jobs: list[tuple[str, str, str, int]],
    runtimes: dict[str, dict[str, list[float]]],
) -> list[tuple[str, str, str | None, float | None, str | None]]:
    """`(path, job key, claimed, measured seconds, verdict)` per job-level timeout.

    The verdict is [`annotation_verdict`]'s, or None where there is no run
    history to judge by. Pure apart from reading the workflow files, so the
    self-test can run the whole rule over fixture files.
    """
    found = []
    for path in sorted({p for p, _, _, _ in jobs}):
        for _i, key, _budget, claimed in annotation_lines(path):
            actual = median_seconds(path, key, jobs, runtimes)
            found.append((path, key, claimed, actual,
                          None if actual is None else annotation_verdict(claimed, actual)))
    return found


def check_observed_annotations(jobs: list[tuple[str, str, str, int]],
                               runtimes: dict[str, dict[str, list[float]]]) -> None:
    """Every `# observed` annotation still describes the job it annotates."""
    print("\nEvery `# observed` annotation is still true (incident 5)")
    print(f"  ..    jobs measured at or under {DRIFT_FLOOR_SECONDS}s are neither "
          f"required to carry one nor compared")
    stale = 0
    for path, key, claimed, actual, verdict in annotation_findings(jobs, runtimes):
        if actual is None or verdict is None:
            # Nothing to compare against, and nothing to claim either: an
            # annotation is only owed where there is run history.
            if claimed:
                skip(f"{path}::{key} claims {claimed} but has no recent run to check it")
            continue
        measured = format_duration(actual)
        if verdict == "missing":
            check(False, f"{path}::{key} has no `# observed` annotation (it runs {measured})")
        elif verdict == "unreadable":
            check(False, f"{path}::{key} annotation {claimed!r} is not a duration")
        elif claimed is not None:
            check(verdict == "ok",
                  f"{path}::{key} `# observed {claimed}` vs measured {measured}")
        stale += verdict != "ok"
    if stale:
        print(f"\n  {stale} annotation(s) are stale. They are generated, not "
              f"hand-written:\n      python3 scripts/check-ci-config.py "
              f"--update-observed\n  then commit the result.")


def update_observed(jobs: list[tuple[str, str, str, int]],
                    runtimes: dict[str, dict[str, list[float]]]) -> int:
    """Rewrite each `# observed` annotation that is missing or has drifted.

    An annotation that is still true is left as written, however far it sits
    from today's median: rewriting every line on every run made a diff of noise
    on every job under a minute. A job at or under the floor is not given one.
    """
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
            if annotation_verdict(claimed, actual) == "ok":
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
        budgets = {name: budget for p, _, name, budget in jobs if p == path}
        try:
            runtimes[path] = observed_runtimes(path, repo, budgets)
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
    check_embed_precedes_tests()
    check_msrv_runs_the_msrv()
    check_release_gate_runs_the_toolchain_it_names()
    check_actions_are_pinned()
    check_tools_are_pinned()
    check_named_lockfiles_are_tracked()
    check_fuzz_targets_are_run()
    check_workflows_readme()
    check_docs_publish_main_as_dev()
    check_release_notes_link_the_versions_manual()
    check_runners_are_pinned()
    check_headroom_arithmetic()
    check_observed_rule()

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
