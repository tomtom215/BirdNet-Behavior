# What a Perch bar costs: the measurement behind the manual

`docs/book/admin/classifiers.md` ("What a Perch bar costs") quotes the numbers in this directory. This file says
where the audio came from, what was computed, how to run it again, and what the result does not show.

**Status of the result: VALIDATED as a measurement of Perch v2 alone on the recordings below; not validated for
any particular station.** Nothing here shows that a given bar is right for your microphone, your site or your
species list.

## What was measured

Perch v2 (`perch-v2.onnx`, the file the station runs) scores a 5 s window at 32 kHz. The station reports each class's
share of the window's softmax. For every 5 s window of every recording, this study took those shares and asked, for a
bar *b*:

* **found**: the annotators marked species *s* in this window and Perch's share for *s* is at least *b*;
* **unmarked report**: Perch's share for a bird-species class is at least *b* and nobody marked that species in
  this window;
* **empty window**: a window the annotators marked no bird in (1 444 of the 6 069).

"Marked in the window" is `events.label_window`: with *d* the call's length (at least 0.3 s), a call counts as present
when it overlaps the window by at least min(*d*/2, 1.0 s). A call that overlaps by less but comes within 0.5 s of the
window is neither present nor absent and is left out of both counts. A looser and a stricter rule moved the recall by 1 to 2 points
(`results/real_label_sensitivity.csv`). Intervals are 95%, from resampling whole recordings (not windows, which are
correlated within a recording).

Species names in the annotations were matched to Perch's classes by scientific name, with 15 hand synonyms
(`results/taxon_synonyms_used.csv`): 567 of the 569 annotated names have a class; the two without one are
*Celeus grammicus* and *Camaroptera brevicaudata*.

## Data

All audio was fetched by HTTP range request from the public records below into a directory of its own; **no audio is
stored in this repository**. `results/data_sources.csv` has one row per source with the licence and why a source was
or was not used, `results/audio_files_used.csv` the SHA-256 of every file or file prefix that was read, and
`results/data_md5_check.csv` the comparison of 29 Zenodo files against the checksums Zenodo lists (all match).

| Set | Record | Licence | Recordings | Windows | Where |
|---|---|---|---|---|---|
| CEB | [zenodo.org/records/21821133](https://zenodo.org/records/21821133) | CC BY 4.0 | 89 | 3 204 | Central Europe |
| POW | [10.5281/zenodo.4656848](https://zenodo.org/records/4656848) | CC0 | 6 | 360 | Powdermill, Pennsylvania |
| SNE | [10.5281/zenodo.7525805](https://zenodo.org/records/7525805) | CC BY 4.0 | 8 | 960 | Sierra Nevada, California |
| SSW | [10.5281/zenodo.7079380](https://zenodo.org/records/7079380) | CC BY 4.0 | 3 | 341 | Sapsucker Woods, New York |
| COR | [10.5281/zenodo.7525349](https://zenodo.org/records/7525349) | CC BY 4.0 | 3 | 283 | Colombia and Costa Rica |
| PER | [10.5281/zenodo.7079124](https://zenodo.org/records/7079124) | CC BY 4.0 | 3 | 296 | Madre de Dios, Peru |
| KEN | [10.5281/zenodo.10943500](https://zenodo.org/records/10943500) | CC BY 4.0 | 3 | 331 | Lake Baringo, Kenya |
| HAW | [10.5281/zenodo.7078499](https://zenodo.org/records/7078499) | CC BY 4.0 | 5 | 294 | Island of Hawai'i |

Also used, for the controlled part: 23 Xeno-canto focal recordings from the CEB record's training archive (licences
CC BY-NC-SA 3.0 and 4.0, per recording), cut into 32 clean five-second single-species segments of 16 species and mixed
(`scripts/run_syn.py`) into 1 100 two- to four-species windows at equal levels, 10 and 20 dB apart, and as a ladder; and
155 clips of the ESC-50 set (rain, wind, thunderstorm, sea waves, engine, airplane, helicopter and train; CC BY-NC
for the set, the per-clip Freesound licences were not checked) as bird-free sound. Sources considered and not used,
with the reason, are in `results/data_sources.csv`; Xeno-canto itself was not used (its API needs a key).

Every record names its authors and asks to be cited; cite the records above if you use these results.

## Running it again

```bash
export PERCH_BAR_ROOT=/path/to/an/empty/directory       # data/, work/, results/, meta/ are made under it
export PERCH_BAR_MODELS=/path/to/models                  # perch-v2.onnx and the BirdNET+ V3.0 preview 3 files
export PERCH_BAR_PYTHON=/path/to/python-with-numpy-and-onnxruntime
bash tools/perch-bar/scripts/run_all.sh
```

About 1.1 GB is downloaded. The Perch pass over the real recordings is the slow step; neither it nor the
`analyse_*.py` and `make_tables.py` steps were timed for this file. `results/` is what a full
run produced; **rerunning those steps over the stored outputs reproduced all 53 `.csv` and `.txt` files byte for
byte** (checked by hashing before and after; the scripts write CRLF line ends, which git stores as LF, and the
committed files equal the reproduced ones once the CRs are dropped). That check does not repeat the downloads or the model pass; the model
pass was checked by the study's author, who found a rerun bit-identical and the pipeline's logits equal to an
independent probe of the same model (largest difference 0.0). `scripts/selftest.py` holds known-answer checks of the
window labelling and the metrics and runs first.

`parity_check.py`, which compared the pipeline's logits with a throwaway probe script, is not included.

## What it does not show

* Three quarters of the annotated species-window pairs (7 943 of 10 709) are one data set from Central Europe. Four of
  the eight sets have three recordings each, so their rows are indicative only. At a bar of 0.35 the share found runs
  from 2% (Hawai'i) to 41% (Sapsucker Woods); the tropical and island sets are 2% to 8%.
* Annotators leave out calls that are too faint or brief to mark (`results/provider_statements_quoted.csv` has the
  providers' own words), so "unmarked" over-counts false reports.
* Whether any of these recordings was in Perch v2's training data is **unknown**. Perch's sources include Xeno-canto
  and FSD50K; ESC-50 and FSD50K both draw on Freesound.
* The study ran the model on its own: no species filter, no geomodel, no ten-detections cap, none of the station's
  other filters. The columns restricted to each data set's own species list are a best case for a regional filter, not
  a measurement of the station's.
* The mixtures are synthetic: 32 segments, 16 species, summed in the waveform. A real chorus has overlap, reverberation
  and distance that these do not.
* BirdNET+ V3.0 preview 3 was run on the same windows (`results/bnp_vs_perch*.csv`). Its probabilities and Perch's
  shares are not on one scale and its training data may overlap these sets, so the comparison is not a ranking of the
  models and is not used in the manual.
