#!/usr/bin/env python3
"""Rebuild crates/birdnet-core/data/labels/birdnet-v24-en.txt.

    python3 -I scripts/build-v24-labels.py <birdnet-pi>/model <hf-labels.txt>

The BirdNET V2.4 label file this project installs (birdnet-go's
`labels.txt`, `Scientific_Common` per line) names 460 of its 6,522 classes
differently from the English names BirdNET-Pi stores for the same classes
(`Pica pica` is "Common Magpie" in one and "Eurasian Magpie" in the other;
`Accipiter gentilis` is "Eurasian Goshawk" and "Northern Goshawk"; "Gray" is
"Grey"; "Cinereous Vulture" is "Black Vulture"). A station that imports a
BirdNET-Pi history and then records with V2.4 would hold one species under two
names, and every query that groups by common name would split it.

This writes the V2.4 label file with BirdNET-Pi's English names: the class
order of BirdNET-Pi's own label file (`BirdNET_GLOBAL_6K_V2.4_Model_FP16_Labels.txt`,
scientific names only) and the names of its `model/l18n/labels_en.json`, the
file BirdNET-Pi writes into its `labels.txt` when `DATABASE_LANG=en`. Where
`labels_en.json` has no entry for a class (the model's non-species classes),
the name from the birdnet-go label file is kept.

The class order is not taken on trust. The scientific column of the result must
equal the scientific column of the birdnet-go file the ONNX model is paired
with (`<hf-labels.txt>`), line for line, or this refuses to write anything:
the model outputs scores by position, so a reordered file would name the wrong
bird.

The output is deterministic: BirdNET-Pi's order, `\n` line ends, UTF-8, no BOM.
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

OUT = (
    Path(__file__).resolve().parent.parent
    / "crates"
    / "birdnet-core"
    / "data"
    / "labels"
    / "birdnet-v24-en.txt"
)
PI_LABELS = "BirdNET_GLOBAL_6K_V2.4_Model_FP16_Labels.txt"
PI_EN = "l18n/labels_en.json"


def fail(msg: str) -> "None":
    sys.exit(f"build-v24-labels: {msg}")


def read_lines(path: Path) -> list[str]:
    text = path.read_bytes().decode("utf-8")
    return [line for line in text.split("\n") if line != ""]


def main() -> None:
    if len(sys.argv) != 3:
        fail(__doc__.split("\n\n")[0].strip())
    pi = Path(sys.argv[1])
    hf = Path(sys.argv[2])

    pi_order = read_lines(pi / PI_LABELS)
    en_raw = (pi / PI_EN).read_bytes()
    en = json.loads(en_raw.decode("utf-8"))
    go_lines = read_lines(hf)

    if len(pi_order) != len(go_lines):
        fail(f"BirdNET-Pi lists {len(pi_order)} classes, the V2.4 label file {len(go_lines)}")

    out: list[str] = []
    replaced = 0
    kept = []
    for i, (sci, go_line) in enumerate(zip(pi_order, go_lines)):
        go_sci, sep, go_com = go_line.partition("_")
        if not sep or go_sci != sci:
            fail(f"line {i + 1}: BirdNET-Pi has {sci!r}, the V2.4 label file has {go_line!r}")
        name = en.get(sci)
        if not isinstance(name, str) or name == "":
            kept.append(sci)
            name = go_com
        elif name != go_com:
            replaced += 1
        if "_" in sci or "\n" in name or "\t" in name:
            fail(f"line {i + 1}: {sci!r} / {name!r} would not survive the label format")
        out.append(f"{sci}_{name}")

    text = "\n".join(out) + "\n"
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(text, encoding="utf-8", newline="")
    digest = hashlib.sha256(text.encode("utf-8")).hexdigest()
    print(f"wrote {OUT} ({len(out)} classes)")
    print(f"  names replaced from labels_en.json: {replaced}")
    print(f"  classes with no labels_en.json name, kept from the V2.4 file: {len(kept)} {kept}")
    print(f"  sha256 {digest}")
    print(f"  inputs: {PI_LABELS} {hashlib.sha256((pi / PI_LABELS).read_bytes()).hexdigest()}")
    print(f"          {PI_EN} {hashlib.sha256(en_raw).hexdigest()}")
    print(f"          {hf.name} {hashlib.sha256(hf.read_bytes()).hexdigest()}")


if __name__ == "__main__":
    main()
