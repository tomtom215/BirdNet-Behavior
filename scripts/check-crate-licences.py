#!/usr/bin/env python3
"""Every crate tarball carries the licence files, byte for byte.

    cargo package --workspace --locked --no-verify
    python3 scripts/check-crate-licences.py target/package
    python3 scripts/check-crate-licences.py --self-test

Why this exists: the workspace is licensed CC BY-NC-SA 4.0 and derived from
BirdNET-Pi, whose terms (`LICENSE-UPSTREAM`) and attribution (`NOTICE`) travel
with every copy. `cargo package` ships a crate's own directory, and only the
root crate (`birdnet-behavior`) listed the three files in its `include`: the
other eight tarballs (what `cargo publish` uploads, and what a downstream
`cargo vendor` copies) carried a SPDX string and no licence text, no upstream
terms and no attribution. Nothing noticed, because nothing read a tarball.

The fix is a symlink per crate to the file at the repository root, so there is
one copy to edit. `cargo package` follows a symlink and archives the file it
points at, which is what makes that work -- and it is also what this reads back:
a checkout that does not preserve symlinks (`core.symlinks=false`, a zip
download) turns each link into a 13-byte text file naming its target, and
`cargo package` then archives *that*. So the check is on the tarball, not on the
tree: each crate must contain a regular, non-empty `LICENSE`, `LICENSE-UPSTREAM`
and `NOTICE` at the package root whose bytes equal the repository's.

The crates expected are read from the root `Cargo.toml` (the root package and
every workspace member), so a ninth library crate added without links, or a
`.crate` that was not produced, fails rather than being skipped.

Exit 0 = every expected crate passes; 1 = a finding; 2 = bad usage.
"""

from __future__ import annotations

import io
import os
import sys
import tarfile
import tempfile
import tomllib
from pathlib import Path

LICENCE_FILES = ("LICENSE", "LICENSE-UPSTREAM", "NOTICE")


def expected_crates(root: Path) -> dict[str, str]:
    """`{package name: version}` for the root package and every workspace member."""
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    version = manifest["workspace"]["package"]["version"]
    names = {manifest["package"]["name"]: version}
    for member in manifest["workspace"]["members"]:
        member_manifest = tomllib.loads((root / member / "Cargo.toml").read_text(encoding="utf-8"))
        names[member_manifest["package"]["name"]] = version
    return names


def check_tarball(path: Path, prefix: str, reference: dict[str, bytes]) -> list[str]:
    """What is wrong with one `.crate` file, as a list of findings (empty = fine)."""
    problems: list[str] = []
    try:
        archive = tarfile.open(path, "r:gz")
    except (tarfile.TarError, OSError) as e:
        return [f"{path.name}: cannot be read as a gzipped tarball ({e})"]
    with archive:
        members = {m.name: m for m in archive.getmembers()}
        for name in LICENCE_FILES:
            member = members.get(f"{prefix}/{name}")
            if member is None:
                problems.append(f"{path.name}: has no {name}")
                continue
            if not member.isreg():
                problems.append(f"{path.name}: {name} is not a regular file (type {member.type!r})")
                continue
            data = archive.extractfile(member).read()  # type: ignore[union-attr]
            if not data.strip():
                problems.append(f"{path.name}: {name} is empty")
            elif data != reference[name]:
                problems.append(
                    f"{path.name}: {name} is {len(data)} bytes and differs from the repository's "
                    f"{len(reference[name])}"
                    + (f" (it reads {data[:60]!r}: a symlink checked out as text?)"
                       if len(data) < 200 else "")
                )
    return problems


def check_dir(package_dir: Path, root: Path) -> list[str]:
    """Check every expected crate's tarball in `package_dir` against `root`'s files."""
    reference = {name: (root / name).read_bytes() for name in LICENCE_FILES}
    for name, data in reference.items():
        if not data.strip():
            return [f"the repository's own {name} is empty"]
    problems: list[str] = []
    expected = expected_crates(root)
    for name, version in sorted(expected.items()):
        tarball = package_dir / f"{name}-{version}.crate"
        if not tarball.is_file():
            problems.append(f"{tarball.name}: not found in {package_dir} "
                            f"(was `cargo package --workspace` run?)")
            continue
        found = check_tarball(tarball, f"{name}-{version}", reference)
        status = "FAIL" if found else "ok  "
        print(f"  {status}  {tarball.name}: {', '.join(LICENCE_FILES)}")
        for f in found:
            print(f"        {f}")
        problems += found
    return problems


# ---------------------------------------------------------------------------
# The self-test: the check is run over synthetic tarballs in both directions, so
# a check that fails everything (or nothing) cannot satisfy it.
# ---------------------------------------------------------------------------

def _tarball(path: Path, prefix: str, files: dict[str, bytes], *, symlinks: dict[str, str] | None = None) -> None:
    with tarfile.open(path, "w:gz") as archive:
        for name, data in files.items():
            info = tarfile.TarInfo(f"{prefix}/{name}")
            info.size = len(data)
            archive.addfile(info, io.BytesIO(data))
        for name, target in (symlinks or {}).items():
            info = tarfile.TarInfo(f"{prefix}/{name}")
            info.type = tarfile.SYMTYPE
            info.linkname = target
            archive.addfile(info)


def self_test() -> int:
    failures: list[str] = []

    def expect(ok: bool, what: str) -> None:
        print(f"  {'ok  ' if ok else 'FAIL'}  {what}")
        if not ok:
            failures.append(what)

    good = {"LICENSE": b"licence text\n" * 20, "LICENSE-UPSTREAM": b"upstream terms\n" * 50,
            "NOTICE": b"attribution\n" * 10, "README.md": b"# a crate\n"}
    crates = ("top", "lib-a", "lib-b")

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp) / "repo"
        pkg = Path(tmp) / "package"
        root.mkdir()
        pkg.mkdir()
        for name in LICENCE_FILES:
            (root / name).write_bytes(good[name])
        (root / "Cargo.toml").write_text(
            '[workspace]\nmembers = ["crates/a", "crates/b"]\n'
            '[workspace.package]\nversion = "1.2.3"\n[package]\nname = "top"\n',
            encoding="utf-8")
        for member in ("a", "b"):
            (root / "crates" / member).mkdir(parents=True)
            (root / "crates" / member / "Cargo.toml").write_text(
                f'[package]\nname = "lib-{member}"\n', encoding="utf-8")

        def rebuild(changes: dict[tuple[str, str], bytes | None] | None = None) -> None:
            """Write a complete tarball per crate, then apply `{(crate, file): bytes | None}`.

            `None` leaves the file out of that crate's tarball.
            """
            for crate in crates:
                files = dict(good)
                for (target, file), value in (changes or {}).items():
                    if target == crate:
                        if value is None:
                            files.pop(file, None)
                        else:
                            files[file] = value
                path = pkg / f"{crate}-1.2.3.crate"
                path.unlink(missing_ok=True)
                _tarball(path, f"{crate}-1.2.3", files)

        def findings() -> list[str]:
            real, sys.stdout = sys.stdout, io.StringIO()
            try:
                return check_dir(pkg, root)
            finally:
                sys.stdout = real

        def reports(needle: str) -> bool:
            return any(needle in p for p in findings())

        rebuild()
        expect(findings() == [], "three complete tarballs pass")

        rebuild({("lib-a", "NOTICE"): None})
        expect(reports("lib-a-1.2.3.crate: has no NOTICE"), "a crate without NOTICE fails, and is named")
        rebuild({("lib-b", "LICENSE-UPSTREAM"): None})
        expect(reports("lib-b-1.2.3.crate: has no LICENSE-UPSTREAM"),
               "a crate without LICENSE-UPSTREAM fails, and is named")
        rebuild({("top", "LICENSE"): None})
        expect(reports("top-1.2.3.crate: has no LICENSE"), "a crate without LICENSE fails, and is named")
        rebuild({("top", "LICENSE"): b"../../LICENSE"})
        expect(reports("LICENSE is 13 bytes") and reports("a symlink checked out as text"),
               "a symlink checked out as 13 bytes of text fails, and says why")
        rebuild({("lib-a", "LICENSE"): good["LICENSE"] + b"edited\n"})
        expect(reports("differs from the repository's"), "a copy that has drifted from the repository's fails")
        rebuild({("lib-a", "NOTICE"): b"\n"})
        expect(reports("NOTICE is empty"), "an empty NOTICE fails")

        rebuild()
        _tarball(pkg / "lib-a-1.2.3.crate", "lib-a-1.2.3", {"README.md": good["README.md"]},
                 symlinks={n: f"../../{n}" for n in LICENCE_FILES})
        expect(reports("is not a regular file"), "a symlink member in the tarball fails")

        rebuild()
        (pkg / "lib-b-1.2.3.crate").unlink()
        expect(reports("lib-b-1.2.3.crate: not found"),
               "an expected crate with no tarball fails rather than being skipped")

        (root / "crates" / "c").mkdir(parents=True)
        (root / "crates" / "c" / "Cargo.toml").write_text('[package]\nname = "lib-c"\n', encoding="utf-8")
        manifest = (root / "Cargo.toml").read_text(encoding="utf-8")
        (root / "Cargo.toml").write_text(
            manifest.replace('"crates/b"]', '"crates/b", "crates/c"]'), encoding="utf-8")
        rebuild()
        expect(reports("lib-c-1.2.3.crate: not found"),
               "a workspace member added without being packaged fails")

    print(f"\nself-test: {len(failures)} failure(s)")
    return 1 if failures else 0


def main(argv: list[str]) -> int:
    if argv == ["--self-test"]:
        return self_test()
    args = [a for a in argv if not a.startswith("--")]
    root = Path(".")
    if "--root" in argv:
        i = argv.index("--root")
        if i + 1 >= len(argv):
            print(__doc__, file=sys.stderr)
            return 2
        root = Path(argv[i + 1])
        args = [a for a in args if a != argv[i + 1]]
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    package_dir = Path(args[0])
    if not package_dir.is_dir():
        print(f"{package_dir} is not a directory", file=sys.stderr)
        return 2
    print(f"Every crate tarball in {package_dir} carries {', '.join(LICENCE_FILES)}")
    problems = check_dir(package_dir, root)
    if problems:
        print(f"\n{len(problems)} problem(s). The files are symlinks at each crate's root "
              f"to the repository's; see scripts/check-crate-licences.py.")
        return 1
    print("all crate tarballs carry the licence files")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
