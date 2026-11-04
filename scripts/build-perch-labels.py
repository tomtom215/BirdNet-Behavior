#!/usr/bin/env python3
"""Build the bundled Perch v2 label file from Google's published one.

    python3 -I scripts/build-perch-labels.py <labels.csv>

`<labels.csv>` is `assets/labels.csv` of the Hugging Face repository
`cgeorgiaw/Perch` at revision c3c3c816976a6c1bb75140b8fbaef7397a6f708f (the
SavedModel of Google's `bird-vocalization-classifier` v2, Apache-2.0, from which
the ONNX conversion this project installs was made):

    https://huggingface.co/cgeorgiaw/Perch/resolve/c3c3c816976a6c1bb75140b8fbaef7397a6f708f/assets/labels.csv

It is one class name per line in the model's output order, led by a line naming
the label set (`inat2024_fsd50k`) rather than a class, with Windows line ends.
The station's label loader wants a header it can recognise, so this writes
`crates/birdnet-core/data/labels/perch-v2.csv`:

    sci_name,com_name
    Abavorana luctuosa,Abavorana luctuosa
    ...

one row per class, in the same order. The file carries no common names, so the
common name is the class name and nothing is invented. Two things follow from
what the file is, and the station's pages show them as they are:

  * Most classes are iNaturalist 2024 taxa (birds, but also frogs, insects,
    mammals), named by scientific name.
  * About two hundred are FSD50K sound classes (`Engine`, `Vehicle`,
    `Wild_animals`, `Animal`, `Mechanisms`, ...), which name a kind of sound
    and not a species.

Refuses to write unless the input has the digest this was written against, 14 795
distinct classes after the first line, and nothing in any name that would break
the CSV it is written into.
"""
import hashlib
import sys

EXPECTED_INPUT_SHA256 = "e4d5c0397d8fb08bf90c6b13a34810af53504faad927e472fcc567793c9de057"
EXPECTED_FIRST_LINE = "inat2024_fsd50k"
EXPECTED_CLASSES = 14_795
OUT = "crates/birdnet-core/data/labels/perch-v2.csv"


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
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
    out = "sci_name,com_name\n" + "".join(f"{n},{n}\n" for n in names)
    open(OUT, "w", encoding="utf-8", newline="\n").write(out)
    print(f"wrote {OUT}: {len(names)} classes, sha256 {hashlib.sha256(out.encode()).hexdigest()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
