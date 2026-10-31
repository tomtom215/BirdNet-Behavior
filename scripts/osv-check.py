#!/usr/bin/env python3
"""Ask OSV about every locked package, as the `OSV-Scanner (lockfiles)` job does.

The job (`.github/workflows/osv.yml`) fails on any advisory not waived in
`osv-scanner.toml`. Whether a given lockfile is clean depends on the advisory
database as of the day, not only on the lockfile: an advisory published, or
corrected, after the last green run turns the job red for every open pull request,
and a record can also change so that a waiver no longer matches anything. So this is
a point-in-time answer: run it immediately before a push, and read a failure that
names a package this change did not touch as the state of `main`.

It sends the name and version of each locked package (nothing else) to
https://api.osv.dev, the database the scanner reads, and applies the waivers in
`osv-scanner.toml` (an entry matches the advisory's id or any alias, and stops
matching after its `ignoreUntil` date). Exit status: 0 when nothing is unwaived.

Lockfiles are the ones `osv.yml` passes to the scanner; keep the two lists equal.
"""

import datetime
import json
import re
import sys
import tomllib
import urllib.request

LOCKFILES = {
    "Cargo.lock": "cargo",
    "tools/visual-qa/package-lock.json": "npm",
    "tools/ci/requirements-ci.txt": "pip",
    "tools/model-convert/requirements.txt": "pip",
    "tools/parity/requirements-crosscheck.txt": "pip",
    "tools/parity/requirements-primary.txt": "pip",
}
API = "https://api.osv.dev/v1"


def packages():
    """(ecosystem, name, version, lockfile) for every pinned package."""
    found = []
    for path, kind in LOCKFILES.items():
        if kind == "cargo":
            with open(path, "rb") as f:
                for p in tomllib.load(f)["package"]:
                    if str(p.get("source", "")).startswith("registry+"):
                        found.append(("crates.io", p["name"], p["version"], path))
        elif kind == "npm":
            with open(path, encoding="utf-8") as f:
                for key, meta in json.load(f).get("packages", {}).items():
                    if "node_modules/" in key and "version" in meta:
                        found.append(
                            ("npm", key.rsplit("node_modules/", 1)[-1], meta["version"], path)
                        )
        else:
            with open(path, encoding="utf-8") as f:
                for line in f:
                    m = re.match(r"^\s*([A-Za-z0-9_.\-]+)\s*==\s*([A-Za-z0-9_.\-+!]+)", line)
                    if m:
                        found.append(("PyPI", m.group(1), m.group(2), path))
    return found


def call(path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        f"{API}/{path}", data, {"Content-Type": "application/json"} if data else {}
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.load(resp)


def main():
    pins = packages()
    if not pins:
        sys.exit("osv-check: no packages read from the lockfiles; this is not a pass")
    hits = {}
    for i in range(0, len(pins), 500):
        chunk = pins[i : i + 500]
        queries = [
            {"package": {"name": n, "ecosystem": e}, "version": v} for e, n, v, _ in chunk
        ]
        for pin, result in zip(chunk, call("querybatch", {"queries": queries})["results"]):
            for vuln in result.get("vulns", []):
                hits.setdefault(vuln["id"], []).append(pin)
    print(f"osv-check: {len(pins)} packages, {len(hits)} advisories match")

    with open("osv-scanner.toml", "rb") as f:
        waivers = {w["id"]: w.get("ignoreUntil") for w in tomllib.load(f).get("IgnoredVulns", [])}
    today = datetime.date.today()
    bad = 0
    for vid in sorted(hits):
        record = call(f"vulns/{vid}")
        waived = [w for w in {vid, *record.get("aliases", [])} if w in waivers]
        until = waivers[waived[0]] if waived else None
        ok = bool(waived) and (until is None or until >= today)
        bad += not ok
        state = "waived" if ok else ("waiver EXPIRED" if waived else "NOT WAIVED")
        names = ", ".join(sorted({f"{n} {v}" for _, n, v, _ in hits[vid]}))
        print(f"  {vid}: {record.get('summary', '')[:70]} [{names}] -> {state}")
    for wid, until in waivers.items():
        if not any(wid == h or wid in call(f"vulns/{h}").get("aliases", []) for h in hits):
            print(f"  note: the waiver for {wid} (until {until}) matches no locked package now")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
