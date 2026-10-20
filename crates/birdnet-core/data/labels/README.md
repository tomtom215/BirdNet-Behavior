# Bundled V2.4 labels

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
