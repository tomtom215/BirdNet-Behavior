# Bundled labels

Two files: the BirdNET V2.4 labels (`birdnet-v24-en.txt`) and the Perch v2 labels
(`perch-v2.csv`, below).

## BirdNET V2.4

`birdnet-v24-en.txt` is the label file for the BirdNET V2.4 classifier
(`BirdNET_v2.4_fp32_dfttrunc.onnx`), one `Scientific_Common` line per class in
the model's output order, 6,522 lines. It is compiled into the binary
(`birdnet_core::inference::labels::BIRDNET_V24_ENGLISH_LABELS`) and written to
disk by `birdnet-behavior --install-model birdnet-v24`; nothing downloads it.

## Why it is not the file the model is published with

The label file published beside the ONNX conversion (birdnet-go's `labels.txt`)
names 460 of the 6,522 classes differently from the English names BirdNET-Pi
stores for the same classes. A station that imports a BirdNET-Pi history and
then records with V2.4 would hold one species under two names, and every query
that groups by common name would split it. This file keeps that file's class
order and uses BirdNET-Pi's names.

## Where it comes from

`scripts/build-v24-labels.py` joins two files of
[BirdNET-Pi](https://github.com/Nachtzuster/BirdNET-Pi) (Nachtzuster's fork) at
commit `88985a3a6fb1931a33f26ba1c46717a2a444c5b7`:

| input | SHA-256 |
|---|---|
| `model/BirdNET_GLOBAL_6K_V2.4_Model_FP16_Labels.txt` (class order) | `3559fbf2d79df985897b4b8e1a6266c1fbb25a95a4c3cecd66c1d090d3dc0b12` |
| `model/l18n/labels_en.json` (English names) | `b789df3670a4a2d3ab8cbf18307dd13b6fcb122d5107c59862e01d9c017c9cbd` |

and refuses to write unless the class order equals, line for line, that of the
birdnet-go file the model is paired with
(`labels.txt` at Hugging Face revision `acd49768d227c726ac94550c94f0effb3e1dd04c`,
SHA-256 `487937b6ad132b8506215523209a87b86adc9dce8e5ed3048ce9268189dddd3d`).
The output's SHA-256 is `3773334c2ad218bde0ac083dd0b6e839a5cf34e84a6017830b9f446fbb57a215`.
`crates/birdnet-core/tests/the_v24_labels_name_birds_as_birdnet_pi_does.rs`
holds the file to that digest and, when the birdnet-go file is at hand, to its
class order.

## Licence

The same position as `../l18n/README.md`: BirdNET-Pi is CC BY-NC-SA 4.0, as is
this project, and its README credits the names to @patlevin's BirdNET label
translations and to Wikipedia without saying which name came from where. Whether
that combination can be redistributed is the maintainer's question and is listed
in `docs/HANDOFF.md`; this file adds English names from the same source to the
same question and does not answer it.

---

## Perch v2

`perch-v2.csv` is the label file for Google's Perch v2 classifier
(`perch_v2.onnx`, the ONNX conversion `--install-model perch-v2` installs): a
`sci_name,com_name` header and one row per class in the model's output order,
14,795 rows. It is compiled into the binary
(`birdnet_core::inference::labels::PERCH_V2_LABELS`) and written to disk as
`perch-v2_labels.csv` by `birdnet-behavior --install-model perch-v2`; nothing
downloads it.

### Why it exists

The ONNX conversion ships no labels, and Google publishes the classes as a bare
list: led by a line that names the label set (`inat2024_fsd50k`) rather than a
class, with Windows line ends, and no header the station's loader recognises.
Labels are positional, so a hand-made file that is wrong is not an error but a
station naming birds as their neighbours in the list.

### Where it comes from

`scripts/build-perch-labels.py` converts `assets/labels.csv` of the Hugging Face
repository `cgeorgiaw/Perch` at revision `c3c3c816976a6c1bb75140b8fbaef7397a6f708f`
(SHA-256 `e4d5c0397d8fb08bf90c6b13a34810af53504faad927e472fcc567793c9de057`), the
SavedModel of Google's `bird-vocalization-classifier` v2, and adds English names
from two sources (below). It refuses to write unless both inputs and the V2.4
names have the digests it was written against, 14,795 distinct classes follow the
first line, and no name could break a CSV row. The output's SHA-256 is
`1ddc4a95831dd09f4b407d173f5a0a8bbb5a9425ba74e8a71d3959a9794216b0`.
`crates/birdnet-core/tests/the_perch_labels_are_the_classes_the_model_scores.rs`
holds the file to that digest, to its length and to the positions that are known.

The class order was checked against the real model once, by hand: Perch v2 run
on `tests/testdata/Pica_pica_30s.wav` put its largest output on class 10,397,
which is *Pica pica*, in the four windows where the magpie is clear. That needs
the 409 MB model, so it is not run in CI.

### What is in it

14,595 classes are Latin binomials (iNaturalist 2024 taxa: birds, but also frogs,
insects and mammals). The other 200 are FSD50K sound classes (`Engine`,
`Vehicle`, `Animal`, `Wild_animals`, ...) that name a kind of sound, not a
species. Google's list carries no common names, so they are added from the two
sources below, and where there is none the common name column repeats the class
name; none is invented.

### Where the common names come from

Of the 14,595 species classes (the other 200 are sound classes), in this order:

| Source | Classes | How |
|---|---|---|
| BirdNET V2.4's English names (`birdnet-v24-en.txt`, the file the station already uses; BirdNET-Pi's names) | 6,266 (five of them, `Dog`, `Engine`, `Fireworks`, `Siren` and *Gryllus assimilis*, are named by V2.4 with the class name itself) | the same bird has the same name whichever classifier heard it |
| IOC World Bird List 15.2 English names ([worldbirdnames.org](https://worldbirdnames.org/master_ioc_list_v15.2.xlsx), SHA-256 `cbe406f71656323fafc6e30114385337c8c1cba54223b35fc668b3827bf28324`) | 3,917 | for a bird V2.4 does not list |
| the class name itself | 4,612 | frogs, insects, mammals, the 200 sound classes, and the 146 birds whose IOC name is withheld |

IOC names are Gill, F., D. Donsker & P. Rasmussen (Eds). 2026. *IOC World Bird
List (v 15.2)*, doi:[10.14344/IOC.ML.15.2](https://doi.org/10.14344/IOC.ML.15.2),
licensed CC BY 3.0 (Unported); they are credited here for that reason.

**146 IOC names are withheld** because the English name is already V2.4's name for
a species the station knows under a different scientific name. 120 of the 146
are the same species renamed (Perch's *Acritillas indica* is V2.4's *Iole
indica*, "Yellow-browed Bulbul"); the other 26 include names the two checklists give to
different species after a split (IOC calls *Calonectris borealis* Cory's
Shearwater; V2.4 uses that name for *C. diomedea*). The station groups detections by
common name, so giving one name to both would merge species that a lump or split
makes different, and the generator does not guess which is which. An alias table
that said so, species by species, would let the 120 take the name; none exists
yet (`docs/book/reference/taxonomy-overlap.md`).

The two sources use different naming styles (V2.4's follow BirdNET-Pi's
tables, IOC's its own), so a Perch-only species and a V2.4 species can differ in
hyphens, capitals and "Grey"/"Gray".

### Licence

The Hugging Face model card for `cgeorgiaw/Perch` declares Apache-2.0. The file
adds nothing to the list but a header line and the column repeated; the entries
are taxon names and sound-class names. As with the V2.4 file, whether that is
enough is the maintainer's question, not one this file answers.
