#!/usr/bin/env python3
"""Write THIRD_PARTY_LICENSES.txt: the licence texts that must travel with a build.

A release binary statically links several hundred third-party Rust packages,
DuckDB (with the libraries it bundles), ONNX Runtime, and serves a set of fonts
and a JavaScript library from its own memory. MIT, BSD, Apache-2.0 and the SIL Open
Font License each require their notice to accompany redistributed copies, and
MPL-2.0 (the twelve `symphonia` crates) additionally requires saying where the
source is. Nothing in a release archive or the container image did.

This script produces that file from what is actually in the build:

  * the packages: the normal-dependency closure of `birdnet-behavior` as the
    release builds it (`--features analytics`, every target), read from
    `cargo tree`; each package's licence files are read from the crate source
    cargo already unpacked for the locked version, so the text is the text of
    the version that ships, not of whatever the repository says today;
  * the things that are not Rust packages, from files kept in `third_party/`
    and `crates/birdnet-web/static/`: ONNX Runtime, the libraries DuckDB
    bundles, ICU, htmx, three.js and the fonts.

A package that ships no licence file in its published crate is written out
under the standard MIT text with the holders its manifest names, and says so,
provided it declares MIT; any other licence without a file is an error, because
inventing the text is not an option.

Packages are listed by name, not version: an ordinary version bump changes
neither the set of packages nor their texts, so it does not make the file stale.
The versions are in Cargo.lock.

    python3 scripts/gen-third-party-licenses.py            # rewrite the file
    python3 scripts/gen-third-party-licenses.py --check    # fail if it would change,
                                                           # or if coverage broke

`--check` also holds the vendored texts to the versions they claim: the ONNX
Runtime directory must be the version `ort-sys` downloads, the DuckDB one the
version `libduckdb-sys` bundles, and the htmx licence the version of
`static/htmx.min.js`, and the three.js licence the version `tools/vendor-three/`
pins (whose build is `static/vendor/three/three.module.min.js`). A bump that
leaves a stale notice fails here, not in a user's audit.

Run from anywhere; needs cargo and Python 3.9+. It reads the crate sources cargo
has unpacked, so run `cargo fetch --locked` first on a machine that has not built
the workspace. It does not touch the network itself.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "THIRD_PARTY_LICENSES.txt")
BINARY_PACKAGE = "birdnet-behavior"
# What the release workflow and the Dockerfile build.
FEATURES = "analytics"

LICENCE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|UNLICENSE|NOTICE|COPYRIGHT)", re.I)
NOT_TEXT = (".rs", ".toml", ".sh", ".py", ".json", ".yml", ".yaml", ".md.in")
BAR = "=" * 78
THIN = "-" * 78

MIT_TEMPLATE = """\
MIT License

Copyright (c) {holders}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"""


def run(args: list[str]) -> str:
    env = dict(os.environ, CARGO_TERM_COLOR="never")
    done = subprocess.run(args, cwd=ROOT, env=env, text=True, capture_output=True)
    if done.returncode != 0:
        sys.exit(f"`{' '.join(args)}` failed:\n{done.stderr}")
    return done.stdout


def closure() -> list[tuple[str, str]]:
    """(name, version) of every package linked into the shipped binary."""
    out = run([
        "cargo", "tree", "--locked", "-p", BINARY_PACKAGE, "--edges", "normal",
        "--target", "all", "--features", FEATURES, "--prefix", "none",
        "--no-dedupe", "-f", "{p}",
    ])
    found = set()
    for line in out.splitlines():
        m = re.match(r"^(\S+) v(\S+)", line)
        if m:
            found.add((m.group(1), m.group(2)))
    return sorted(found)


def metadata() -> dict[tuple[str, str], dict]:
    packages = json.loads(run(["cargo", "metadata", "--format-version", "1", "--locked"]))["packages"]
    return {(p["name"], p["version"]): p for p in packages}


def read_text(path: str) -> str:
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read().replace("\r\n", "\n")


def licence_files(pkg: dict) -> list[str]:
    """Licence files in a package's unpacked source, shallowest first."""
    base = os.path.dirname(pkg["manifest_path"])
    if not os.path.isdir(base):
        sys.exit(f"{base} is not unpacked; run `cargo fetch --locked` first")
    found = []
    for root, dirs, files in os.walk(base):
        if root[len(base):].count(os.sep) >= 2:
            dirs[:] = []
        dirs.sort()
        for name in sorted(files):
            if LICENCE_FILE.match(name) and not name.endswith(NOT_TEXT):
                found.append(os.path.join(root, name))
    declared = pkg.get("license_file")
    if declared:
        path = os.path.join(base, declared)
        if os.path.exists(path) and path not in found:
            found.append(path)
    return sorted(found, key=lambda p: (p.count(os.sep), p))


def squash(text: str) -> str:
    return re.sub(r"\s+", " ", text).strip()


def holders_of(pkg: dict) -> str:
    authors = [re.sub(r"\s*<[^>]*>", "", a).strip() for a in pkg.get("authors") or []]
    authors = [a for a in authors if a]
    if authors:
        return ", ".join(authors)
    return f"the authors of {pkg.get('repository') or pkg['name']}"


def package_sections(
    third_party: list[tuple[str, str]], meta: dict[tuple[str, str], dict]
) -> tuple[str, list[str]]:
    """The crates part of the file, and the problems found while writing it."""
    problems: list[str] = []
    groups: dict[str, dict] = {}
    templated: list[tuple[dict, str]] = []
    for key in third_party:
        pkg = meta[key]
        files = licence_files(pkg)
        if not files:
            expression = pkg.get("license") or ""
            if "MIT" in re.split(r"[\s()/]+", expression):
                templated.append((pkg, MIT_TEMPLATE.format(holders=holders_of(pkg))))
            else:
                problems.append(
                    f"{key[0]} {key[1]} ({expression or 'no licence declared'}) ships no "
                    "licence file and is not MIT, so no text can be written for it"
                )
            continue
        for path in files:
            text = read_text(path)
            digest = hashlib.sha256(squash(text).encode()).hexdigest()
            group = groups.setdefault(digest, {"text": text, "who": []})
            label = key[0]
            if label not in group["who"]:
                group["who"].append(label)

    out: list[str] = []
    ordered = sorted(groups.values(), key=lambda g: (g["who"][0].lower(), len(g["text"])))
    for group in ordered:
        out.append(BAR)
        out.append("Used by: " + ", ".join(sorted(group["who"], key=str.lower)))
        out.append(THIN)
        out.append(group["text"].strip("\n"))
        out.append("")
    if templated:
        out.append(BAR)
        out.append(
            "Packages that declare MIT but publish no licence file. The standard MIT\n"
            "text is written out for each with the holders named in its manifest."
        )
        out.append(BAR)
        for pkg, text in sorted(templated, key=lambda t: t[0]["name"].lower()):
            out.append(f"{pkg['name']}  (declared: {pkg.get('license')})")
            out.append(THIN)
            out.append(text.strip("\n"))
            out.append("")
    return "\n".join(out) + "\n", problems


def vendored(rel: str) -> str:
    path = os.path.join(ROOT, rel)
    if not os.path.exists(path):
        sys.exit(f"missing {rel}; see third_party/README.md")
    return read_text(path).strip("\n")


def engine_versions(meta: dict[tuple[str, str], dict]) -> dict[str, str]:
    """The ONNX Runtime and DuckDB versions this build links, and htmx's."""
    versions = {}
    ort = next((p for (n, _), p in meta.items() if n == "ort-sys"), None)
    tsv = os.path.join(os.path.dirname(ort["manifest_path"]), "build", "download", "dist.tsv")
    found = set(re.findall(r"/ms@(\d+\.\d+\.\d+)/", read_text(tsv)))
    if len(found) != 1:
        sys.exit(f"ort-sys downloads ONNX Runtime versions {sorted(found)}; expected exactly one")
    versions["onnxruntime"] = found.pop()
    duck = next(p for (n, _), p in meta.items() if n == "libduckdb-sys")
    # The crate's middle component is the engine version as MNNPP: 10506 is DuckDB 1.5.6,
    # 20000 is 2.0.0. (The crate's own major stays 1 across both, so it says nothing.)
    number = int(duck["version"].split(".")[1])
    versions["duckdb"] = f"{number // 10000}.{number // 100 % 100}.{number % 100}"
    js = read_text(os.path.join(ROOT, "crates/birdnet-web/static/htmx.min.js"))
    versions["htmx"] = re.search(r'version:"([^"]+)"', js).group(1)
    # three.js: the version the build recipe pins, which must also be the one the committed
    # bundle says it is (its first statement is `var X="186"`, the library's REVISION).
    pin = json.loads(read_text(os.path.join(ROOT, "tools/vendor-three/package.json")))
    versions["three"] = pin["dependencies"]["three"]
    bundle = read_text(os.path.join(ROOT, "crates/birdnet-web/static/vendor/three/three.module.min.js"))
    revision = re.match(r'var \w+="(\d+)";', bundle)
    if not revision or revision.group(1) != versions["three"].split(".")[1]:
        sys.exit(
            f"static/vendor/three/three.module.min.js is revision {revision and revision.group(1)}, "
            f"but tools/vendor-three pins three {versions['three']}: rebuild it with build.sh"
        )
    return versions


def bundled_sections(versions: dict[str, str]) -> tuple[str, list[str]]:
    problems: list[str] = []
    out: list[str] = []

    def section(title: str, body: str) -> None:
        out.append(BAR)
        out.append(title)
        out.append(BAR)
        out.append(body)
        out.append("")

    ort_dir = f"third_party/onnxruntime-{versions['onnxruntime']}"
    if not os.path.isdir(os.path.join(ROOT, ort_dir)):
        problems.append(
            f"ort-sys downloads ONNX Runtime {versions['onnxruntime']}, but {ort_dir}/ does not "
            "exist: vendor that release's LICENSE and ThirdPartyNotices.txt (third_party/README.md)"
        )
    else:
        section(
            f"ONNX Runtime {versions['onnxruntime']} (statically linked through ort-sys)",
            vendored(f"{ort_dir}/LICENSE")
            + "\n\n" + THIN + "\nONNX Runtime's own third-party notices, unmodified:\n"
            + THIN + "\n" + vendored(f"{ort_dir}/ThirdPartyNotices.txt"),
        )

    duck_dir = f"third_party/duckdb-{versions['duckdb']}"
    if not os.path.isdir(os.path.join(ROOT, duck_dir)):
        problems.append(
            f"libduckdb-sys bundles DuckDB {versions['duckdb']}, but {duck_dir}/ does not exist: "
            "vendor that release's third_party licences (third_party/README.md)"
        )
    else:
        names = sorted(f for f in os.listdir(os.path.join(ROOT, duck_dir)) if f.endswith(".LICENSE"))
        body = [
            f"DuckDB {versions['duckdb']} is statically linked (libduckdb-sys). Its own licence is\n"
            "listed above under libduckdb-sys. These are the licences of the libraries it bundles,\n"
            "and of ICU, which the embedded `icu` extension is built from. Apache Thrift's copy in\n"
            "DuckDB carries no licence file; Thrift is Apache-2.0, whose text appears above."
        ]
        for name in names:
            body.append(THIN)
            body.append(name[: -len(".LICENSE")])
            body.append(THIN)
            body.append(vendored(f"{duck_dir}/{name}"))
        section(f"Libraries bundled with DuckDB {versions['duckdb']}, and ICU", "\n".join(body))

    htmx_dir = f"third_party/htmx-{versions['htmx']}"
    if not os.path.isdir(os.path.join(ROOT, htmx_dir)):
        problems.append(
            f"static/htmx.min.js is htmx {versions['htmx']}, but {htmx_dir}/ does not exist: "
            "vendor that release's LICENSE (third_party/README.md)"
        )
    else:
        section(
            f"htmx {versions['htmx']} (served as /static/htmx.min.js)",
            vendored(f"{htmx_dir}/LICENSE"),
        )
    three_dir = f"third_party/three-{versions['three']}"
    if not os.path.isdir(os.path.join(ROOT, three_dir)):
        problems.append(
            f"tools/vendor-three pins three {versions['three']}, but {three_dir}/ does not exist: "
            "vendor that release's LICENSE (third_party/README.md)"
        )
    else:
        section(
            f"three.js {versions['three']} (the part the soundscape terrain uses, served as "
            "/static/vendor/three/three.module.min.js)",
            vendored(f"{three_dir}/LICENSE"),
        )
    section(
        "Fonts (served from the binary; no external font host)",
        vendored("crates/birdnet-web/static/fonts/LICENSES.txt"),
    )
    return "\n".join(out) + "\n", problems


def render() -> tuple[str, list[str], dict]:
    meta = metadata()
    names = closure()
    missing = [k for k in names if k not in meta]
    if missing:
        sys.exit(f"cargo tree names packages cargo metadata does not: {missing}")
    third_party = [k for k in names if meta[k]["source"] is not None]
    mpl = sorted({n for n, v in third_party if "MPL-2.0" in (meta[(n, v)].get("license") or "")})

    crates_text, problems = package_sections(third_party, meta)
    versions = engine_versions(meta)
    bundled_text, more = bundled_sections(versions)
    problems += more

    header = f"""\
Third-party licences
====================

BirdNet-Behavior's own licence is in LICENSE and LICENSE-UPSTREAM. The binary
also contains, or serves, the work of the projects below; their licence texts
are reproduced here as those licences require. NOTICE describes the same
material in prose.

Generated by scripts/gen-third-party-licenses.py from Cargo.lock
({len(third_party)} third-party packages linked into a build with
`--features {FEATURES}`). Do not edit by hand.

Mozilla Public License 2.0
--------------------------
{len(mpl)} packages are licensed under MPL-2.0: {', '.join(mpl)}.
They are used unmodified, at the versions in Cargo.lock. Their source is
published at https://crates.io/crates/<name> and at
https://github.com/pdeljanov/Symphonia, and can be obtained from either.

Dual-licensed packages
----------------------
Where a package offers a choice (for example "MIT OR Apache-2.0"), the choice is
the recipient's; every text offered is reproduced.

"""
    stats = {"third_party": len(third_party), "mpl": len(mpl)}
    body = header + bundled_text + "\n" + BAR + "\nRust packages\n" + BAR + "\n\n" + crates_text
    return body, problems, stats


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true", help="fail if the file would change or coverage broke")
    ap.add_argument("-o", "--output", default=OUT)
    args = ap.parse_args()

    text, problems, stats = render()
    if problems:
        print("licence coverage problems:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        return 1
    if args.check:
        have = read_text(args.output) if os.path.exists(args.output) else ""
        if have != text:
            print(
                f"{os.path.relpath(args.output, ROOT)} is out of date with Cargo.lock and third_party/; "
                "run python3 scripts/gen-third-party-licenses.py and commit it",
                file=sys.stderr,
            )
            return 1
        print(f"ok: {stats['third_party']} packages, {stats['mpl']} MPL-2.0, {len(text):,} bytes, up to date")
        return 0
    with open(args.output, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"wrote {os.path.relpath(args.output, ROOT)}: {stats['third_party']} packages, {len(text):,} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
