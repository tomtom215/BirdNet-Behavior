#!/usr/bin/env python3
"""Rebuild crates/birdnet-core/data/l18n/ from BirdNET-Pi's translation tables.

    python3 -I scripts/build-species-names.py <birdnet-pi>/model/l18n

Reads the 36 `labels_<code>.json` files (scientific name -> common name) that
Nachtzuster's BirdNET-Pi keeps in `model/l18n/` and writes

  * `species_names.tsv` -- one row per species that has at least one genuine
    translation: `sci_name`, then one column per language except English, empty
    where the language has no name of its own for the species;
  * `SOURCES.tsv`       -- per source file: its SHA-256 and how many names it
    contributed, which `crates/birdnet-core/tests/species_names_table.rs`
    recomputes from the table.

Why only *genuine* translations: upstream pads every language file to the same
7,058 species and fills the gaps with the English name, so a Korean file with
622 Korean names (8.8%) lists 7,058 entries. A cell that equals the English pack
is therefore "no translation", and is left empty so that a language's coverage
is a count of names rather than a count of lines, and so the station falls back
to the classifier's own name instead of to a second, different English one.

The output is deterministic: rows in byte order of the scientific name,
columns in byte order of the language code, `\n` line ends, UTF-8, no BOM.
Running it twice over the same input produces the same bytes; the second
output of a run is checked against the first below rather than assumed.
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "crates" / "birdnet-core" / "data" / "l18n"
ENGLISH = "en"


def fail(msg: str) -> "None":
    sys.exit(f"build-species-names: {msg}")


def load(src: Path) -> tuple[dict[str, dict[str, str]], dict[str, str]]:
    packs: dict[str, dict[str, str]] = {}
    digests: dict[str, str] = {}
    for path in sorted(src.glob("labels_*.json")):
        code = path.stem.removeprefix("labels_")
        raw = path.read_bytes()
        digests[code] = hashlib.sha256(raw).hexdigest()
        names = json.loads(raw.decode("utf-8"))
        if not isinstance(names, dict):
            fail(f"{path.name}: expected an object")
        packs[code] = names
    if ENGLISH not in packs:
        fail("no labels_en.json: nothing to tell a translation from a fallback")
    return packs, digests


def check_cell(code: str, sci: str, name: object) -> str:
    if not isinstance(name, str) or not name:
        fail(f"{code}: {sci!r} is not a non-empty string")
    if name != name.strip() or any(c in name for c in "\t\r\n\0"):
        fail(f"{code}: {sci!r} -> {name!r} has whitespace the TSV cannot carry")
    return name


def build(src: Path) -> tuple[str, str]:
    packs, digests = load(src)
    english = packs[ENGLISH]
    keys = set(english)
    for code, names in packs.items():
        if set(names) != keys:
            fail(f"{code}: species set differs from en ({len(names)} vs {len(keys)})")
    codes = sorted(c for c in packs if c != ENGLISH)

    counts = dict.fromkeys(codes, 0)
    rows: list[str] = []
    for sci in sorted(keys, key=lambda s: s.encode("utf-8")):
        if "\t" in sci or sci != sci.strip() or not sci:
            fail(f"scientific name {sci!r} cannot be a TSV key")
        en = check_cell(ENGLISH, sci, english[sci])
        cells = []
        for code in codes:
            name = check_cell(code, sci, packs[code][sci])
            if name == en:
                cells.append("")
            else:
                cells.append(name)
                counts[code] += 1
        if any(cells):
            rows.append("\t".join([sci, *cells]))

    table = "\n".join(["\t".join(["sci_name", *codes]), *rows]) + "\n"
    sources = "\n".join(
        ["code\tsha256\tnames", *(f"{c}\t{digests[c]}\t{counts[c]}" for c in codes)]
    )
    sources += f"\nen\t{digests[ENGLISH]}\t0\n"
    return table, sources


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: build-species-names.py <birdnet-pi>/model/l18n")
    src = Path(sys.argv[1])
    table, sources = build(src)
    again, _ = build(src)
    if again != table:
        fail("two runs over the same input disagree")
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "species_names.tsv").write_text(table, encoding="utf-8", newline="")
    (OUT / "SOURCES.tsv").write_text(sources, encoding="utf-8", newline="")
    n_rows = table.count("\n") - 1
    print(f"wrote {OUT}: {n_rows} species, {len(table)} bytes")


if __name__ == "__main__":
    main()
