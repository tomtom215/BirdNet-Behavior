# Bundled species names

`species_names.tsv` is what the **Common-name language** setting (`DATABASE_LANG`)
draws on. It is compiled into the binary (`birdnet_core::i18n`); nothing is
downloaded and nothing reads it from disk at run time.

## Where it comes from

The 35 non-English tables in `model/l18n/labels_<code>.json` of
[BirdNET-Pi](https://github.com/Nachtzuster/BirdNET-Pi) (Nachtzuster's fork), as of
commit `88985a3a6fb1931a33f26ba1c46717a2a444c5b7` (2026-02-28). Those files last
changed in `2a79f1f` (2025-11-26). `SOURCES.tsv` lists the SHA-256 of each file as
it was read, so a checkout of that commit can be compared with
`(cd model/l18n && sha256sum labels_*.json)`.

BirdNET-Pi's own README credits the names to
[@patlevin](https://github.com/patlevin)'s BirdNET label translations and to
Wikipedia. It does not say which name came from where, and neither do the files,
so that split is not recorded here either. In particular some names derive from
Wikipedia, whose text is CC BY-SA 4.0.

## Licence

BirdNET-Pi is licensed CC BY-NC-SA 4.0 (its `LICENSE`: "BirdNET-Lite and BirdNET-Pi
are licensed under a Creative Commons Attribution-NonCommercial-ShareAlike 4.0
International License"), and so is this project (`LICENSE`, `LICENSE-UPSTREAM`).
The table is redistributed under those terms with that credit. Whether the
Wikipedia-derived names can be combined with CC BY-NC-SA 4.0 is a question this
repository has not settled; it is listed for the maintainer in `docs/HANDOFF.md`.

## What it contains

Only **genuine translations**. Upstream pads every language file to the same
7,058 species and fills a gap with the English name, so a Korean file with 622
Korean names lists 7,058 lines. A cell that equals the English pack's name is
"no translation" and is empty here: the station then shows the classifier's own
name for that species, not a second and different English one.

* `sci_name`, then one column per language except English, tab-separated, UTF-8,
  rows in byte order of the scientific name, columns in byte order of the code.
* A row exists only for a species with at least one name; 7,008 of the 7,058 do.
* English has no column. Choosing it means "show what the classifier stored".

## Coverage

Names each language has, out of the 7,058 species upstream lists. The share is of
that figure, which is how upstream's own `docs/translations.md` counts.

| Language | Code | Names | Share |
|---|---|---|---|
| Afrikaans | `af` | 801 | 11.3% |
| Arabic | `ar` | 2,087 | 29.6% |
| Catalan | `ca` | 6,773 | 96.0% |
| Czech | `cs` | 6,100 | 86.4% |
| Danish | `da` | 5,986 | 84.8% |
| German | `de` | 6,869 | 97.3% |
| Spanish | `es` | 6,840 | 96.9% |
| Estonian | `et` | 3,280 | 46.5% |
| Finnish | `fi` | 6,659 | 94.3% |
| French | `fr` | 6,922 | 98.1% |
| Croatian | `hr` | 6,033 | 85.5% |
| Hungarian | `hu` | 4,401 | 62.4% |
| Indonesian | `id` | 1,182 | 16.7% |
| Icelandic | `is` | 911 | 12.9% |
| Italian | `it` | 5,833 | 82.6% |
| Japanese | `ja` | 6,659 | 94.3% |
| Korean | `ko` | 622 | 8.8% |
| Lithuanian | `lt` | 5,845 | 82.8% |
| Latvian | `lv` | 1,667 | 23.6% |
| Dutch | `nl` | 6,918 | 98.0% |
| Norwegian | `no` | 6,757 | 95.7% |
| Polish | `pl` | 6,828 | 96.7% |
| Portuguese | `pt` | 6,510 | 92.2% |
| Romanian | `ro` | 1,065 | 15.1% |
| Russian | `ru` | 6,731 | 95.4% |
| Slovak | `sk` | 6,792 | 96.2% |
| Slovenian | `sl` | 1,215 | 17.2% |
| Serbian | `sr` | 5,411 | 76.7% |
| Swedish | `sv` | 6,943 | 98.4% |
| Thai | `th` | 1,041 | 14.7% |
| Turkish | `tr` | 6,400 | 90.7% |
| Ukrainian | `uk` | 6,942 | 98.4% |
| Vietnamese | `vi` | 1,454 | 20.6% |
| Chinese (Simplified) | `zh_CN` | 6,210 | 88.0% |
| Chinese (Traditional) | `zh_TW` | 833 | 11.8% |

The classifier is not the same list. Measured on 2026-10-15 against the pinned
BirdNET+ V3.0 Preview 3 label file (11,560 species), 6,632 of its species have a
name in at least one language and 4,928 have none, in any language. The Settings
page shows the real figure for the language and classifier a station is using.

## What it does not contain

* Names a language's tables never had. Where there is none, the classifier's own
  name is shown.
* Any review. These are upstream's names; an error in one is upstream's error, and
  this repository has not proofread 160,000 names in 35 languages.

## Rebuilding

```sh
python3 -I scripts/build-species-names.py <BirdNET-Pi checkout>/model/l18n
```

writes `species_names.tsv` and `SOURCES.tsv` here, byte-identically on every run
over the same input. `crates/birdnet-core/tests/species_names_table.rs` fails if the
table, `SOURCES.tsv` and the coverage table above disagree.
