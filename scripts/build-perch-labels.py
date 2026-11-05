#!/usr/bin/env python3
"""Build the bundled Perch v2 label file: Google's class list plus English names.

    python3 -I scripts/build-perch-labels.py <labels.csv> <master_ioc_list_v15.2.xlsx>

`<labels.csv>` is `assets/labels.csv` of the Hugging Face repository
`cgeorgiaw/Perch` at revision c3c3c816976a6c1bb75140b8fbaef7397a6f708f (the
SavedModel of Google's `bird-vocalization-classifier` v2, Apache-2.0, from which
the ONNX conversion this project installs was made):

    https://huggingface.co/cgeorgiaw/Perch/resolve/c3c3c816976a6c1bb75140b8fbaef7397a6f708f/assets/labels.csv

It is one class name per line in the model's output order, led by a line naming
the label set (`inat2024_fsd50k`) rather than a class, with Windows line ends.
It carries no common names.

`<master_ioc_list_v15.2.xlsx>` is the IOC World Bird List 15.2 master list
(Gill, Donsker & Rasmussen (Eds), doi 10.14344/IOC.ML.15.2, CC BY 3.0):

    https://worldbirdnames.org/master_ioc_list_v15.2.xlsx

The station's label loader wants a header it can recognise, so this writes
`crates/birdnet-core/data/labels/perch-v2.csv`:

    sci_name,com_name
    Pica pica,Eurasian Magpie
    ...

one row per class, in the same order. A class's common name is, in this order:

  1. the English name BirdNET V2.4 gives that species in the station's own
     `birdnet-v24-en.txt` (BirdNET-Pi's names), so the same bird has the same
     name whichever classifier heard it;
  2. the IOC English name of that species;
  3. otherwise the class name itself, and nothing is invented. That is every
     frog, insect and mammal, the FSD50K sound classes, and the birds neither
     list names.

An English name that two classes would share is withheld from the one that took
it from the IOC list: the station groups detections by common name, and two
species under one name would be counted as one. Most classes are iNaturalist
2024 taxa (birds, but also frogs, insects, mammals); about two hundred are
FSD50K sound classes (`Engine`, `Vehicle`, `Wild_animals`, ...) that name a kind
of sound and not a species.

Refuses to write unless both inputs have the digests this was written against,
the label list has 14 795 distinct classes after the first line, and nothing in
any name would break the CSV it is written into.
"""
import hashlib
import re
import sys
import zipfile
import xml.etree.ElementTree as ET
from collections import Counter

EXPECTED_INPUT_SHA256 = "e4d5c0397d8fb08bf90c6b13a34810af53504faad927e472fcc567793c9de057"
EXPECTED_IOC_SHA256 = "cbe406f71656323fafc6e30114385337c8c1cba54223b35fc668b3827bf28324"
V24_NAMES = "crates/birdnet-core/data/labels/birdnet-v24-en.txt"
EXPECTED_V24_SHA256 = "3773334c2ad218bde0ac083dd0b6e839a5cf34e84a6017830b9f446fbb57a215"
EXPECTED_FIRST_LINE = "inat2024_fsd50k"
EXPECTED_CLASSES = 14_795
OUT = "crates/birdnet-core/data/labels/perch-v2.csv"


_NS = {"m": "http://schemas.openxmlformats.org/spreadsheetml/2006/main"}


def ioc_species_names(path: str) -> dict[str, str]:
    """Binomial -> English name for every species row of the IOC master list.

    The genus is named on its own row and not repeated: a species row has an
    epithet and an English name and no subspecies, and takes the genus of the
    nearest row above that names one. The sheet is read with the standard
    library: an .xlsx is a zip of XML, and this runs from one script with
    nothing to install.
    """
    z = zipfile.ZipFile(path)
    tag = "{%s}t" % _NS["m"]
    strings = [
        "".join(t.text or "" for t in si.iter(tag))
        for si in ET.fromstring(z.read("xl/sharedStrings.xml")).findall("m:si", _NS)
    ]
    sheet = ET.fromstring(z.read("xl/worksheets/sheet1.xml"))
    names: dict[str, str] = {}
    genus = ""
    for row in sheet.find("m:sheetData", _NS).findall("m:row", _NS):
        cells: dict[str, str] = {}
        for c in row.findall("m:c", _NS):
            col = re.match(r"[A-Z]+", c.get("r")).group(0)
            v = c.find("m:v", _NS)
            if v is not None:
                cells[col] = strings[int(v.text)] if c.get("t") == "s" else v.text
        row_genus, epithet, sub, english = (cells.get(k, "").strip() for k in "FGHK")
        if row_genus and row_genus != "Genus":
            genus = row_genus
        if genus and epithet and english and not sub and epithet != "Species (Scientific)":
            names[f"{genus} {epithet}"] = english
    return names


def v24_species_names(path: str) -> dict[str, str]:
    """Binomial -> English name from the station's own V2.4 label file (`Scientific_Common` lines)."""
    names: dict[str, str] = {}
    for line in open(path, encoding="utf-8").read().splitlines():
        sci, _, common = line.partition("_")
        if sci and common:
            names[sci] = common
    return names


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    for label, path, want in (
        ("the IOC master list", sys.argv[2], EXPECTED_IOC_SHA256),
        ("the V2.4 names", V24_NAMES, EXPECTED_V24_SHA256),
    ):
        got = hashlib.sha256(open(path, "rb").read()).hexdigest()
        if got != want:
            print(f"{label} is not the file this was written against:\n  want {want}\n  got  {got}", file=sys.stderr)
            return 1
    raw = open(sys.argv[1], "rb").read()
    got = hashlib.sha256(raw).hexdigest()
    if got != EXPECTED_INPUT_SHA256:
        print(f"input is not the file this was written against:\n  want {EXPECTED_INPUT_SHA256}\n  got  {got}", file=sys.stderr)
        return 1
    lines = raw.decode("utf-8").replace("\r\n", "\n").split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    if lines[0] != EXPECTED_FIRST_LINE:
        print(f"first line is {lines[0]!r}, expected {EXPECTED_FIRST_LINE!r}", file=sys.stderr)
        return 1
    names = lines[1:]
    if len(names) != EXPECTED_CLASSES:
        print(f"{len(names)} classes, expected {EXPECTED_CLASSES}", file=sys.stderr)
        return 1
    if len(set(names)) != len(names):
        print("class names are not distinct", file=sys.stderr)
        return 1
    bad = [n for n in names if not n or n != n.strip() or any(c in n for c in ',;\t"\r\n')]
    if bad:
        print(f"{len(bad)} names would break a CSV row, e.g. {bad[:3]!r}", file=sys.stderr)
        return 1
    v24 = v24_species_names(V24_NAMES)
    ioc = ioc_species_names(sys.argv[2])
    chosen: dict[str, tuple[str, str]] = {}
    for n in names:
        if n in v24:
            chosen[n] = (v24[n], "v24")
        elif n in ioc:
            chosen[n] = (ioc[n], "ioc")
        else:
            chosen[n] = (n, "none")
    # A name two species would share is withheld from the IOC-sourced one(s): V2.4's
    # own names are the station's already, and are left as BirdNET-Pi has them.
    taken = Counter(v24.values())
    use = Counter(c for c, src in chosen.values() if src != "none")
    withheld = []
    for n, (common, src) in list(chosen.items()):
        if src == "ioc" and (use[common] > 1 or taken[common] > 0):
            chosen[n] = (n, "none")
            withheld.append(n)
    bad = [c for c, _ in chosen.values() if not c or c != c.strip() or any(ch in c for ch in ',;\t"\r\n')]
    if bad:
        print(f"{len(bad)} common names would break a CSV row, e.g. {bad[:3]!r}", file=sys.stderr)
        return 1
    out = "sci_name,com_name\n" + "".join(f"{n},{chosen[n][0]}\n" for n in names)
    open(OUT, "w", encoding="utf-8", newline="\n").write(out)
    by = Counter(src for _, src in chosen.values())
    print(f"wrote {OUT}: {len(names)} classes, sha256 {hashlib.sha256(out.encode()).hexdigest()}")
    print(f"  common names: {by['v24']} from V2.4, {by['ioc']} from IOC, {by['none']} the class name itself "
          f"({len(withheld)} of those withheld because the name was shared)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
