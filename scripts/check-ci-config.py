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

Run from the repository root. Requires PyYAML, and cargo-mutants for the
mutant-enumeration checks (`--list` builds nothing; it costs ~0.2 s per row).
"""

from __future__ import annotations

import glob
import subprocess
import sys
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

failures: list[str] = []


def check(ok: bool, msg: str) -> None:
    print(f"  {'ok  ' if ok else 'FAIL'}  {msg}")
    if not ok:
        failures.append(msg)


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


def main() -> int:
    check_timeouts()
    check_mutation_matrix()
    print()
    if failures:
        print(f"{len(failures)} check(s) failed:")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("all CI-config checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
