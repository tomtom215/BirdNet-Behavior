# Plate index builder

Builds `crates/birdnet-integrations/data/plates/index.json`: a checked-in map from
BirdNET species to public-domain 19th-century bird plates on Wikimedia Commons
(John Gould, Naumann, the von Wright brothers). The kiosk shows today's birds
as a field-guide plate and reads this index to choose the picture and print its
credit.

Everything in the index is copied from Commons API responses. Nothing is typed
in by hand, and nothing is guessed. The only constants in the script are the
work titles, authors and years and the names of the Commons categories it asks
about. The titles, authors and years are typed, not API data: each was checked against
a bibliography, the Century's span (1830-1832) is disputed (one paper has 1830-1833),
and Naumann's and von Wright's years are left empty on purpose because none could
be verified.

## Run it

Python 3 standard library only. Always run isolated:

```bash
python3 -I tools/plate-index/build.py --help
python3 -I tools/plate-index/build.py                 # build or resume
python3 -I tools/plate-index/build.py --verify        # re-check index.json, no network
```

Outputs: `crates/birdnet-integrations/data/plates/index.json` and
`tools/plate-index/report.md` (per-work counts, rejected licences, species lost
to licence, coverage). Both are deterministic: the second run from the cache
writes the same bytes. The only date in them is `generated`; pass
`--generated YYYY-MM-DD` to pin it.

| Flag | Meaning |
| --- | --- |
| `--cache-dir DIR` | Where raw API responses are kept (default `$XDG_CACHE_HOME/birdnet-plate-index`). One JSON file per request, keyed by the request. |
| `--refresh` | Ignore the cache and fetch again. |
| `--offline` | Never touch the network; fail if something is not cached. |
| `--works a,b` | Build from a subset of works, to choose which to ship. A species is then given the best plate among only those works. |
| `--max-requests N` | Network requests allowed in this run (default 400). |
| `--min-interval S` | Seconds between requests (default 2.0; below 1.5 is refused). |
| `--max-depth`, `--max-categories` | How far into a work's sub-categories to crawl (3 and 200). |
| `--min-width PX` | Smallest accepted plate (default 800). |
| `--partial` | Write an index even though some works did not finish. Without it an unfinished run writes nothing. |
| `--discover` | List Commons categories matching the known name prefixes. |
| `--fetch-thumbs DIR` | Download 600 px thumbnails of index entries, for looking at. |

### Etiquette

The sandbox shares one egress address and Wikimedia answers 429 to it often
(about half of this run's attempts). The script identifies itself
(`BirdNet-Behavior-plate-index/0.1 (https://github.com/tomtom215/BirdNet-Behavior; index build)`),
sends `maxlag=5`, batches 50 files per request through
`generator=categorymembers` with `prop=imageinfo|categories`, and waits at least
2 s between requests. On 429, `maxlag` or a non-JSON body it waits the larger of
the server's `Retry-After` and an exponential backoff (10 s doubling to 5 min),
retries up to 8 times, then stops and reports. Rerunning resumes from the cache.
Exit status 75 means "stopped, rerun later".

## The licence rule

A file is accepted only if Commons' own metadata (`prop=imageinfo`,
`iiprop=extmetadata`) says it is free of copyright:

- `Copyrighted` is `False` or absent, **and**
- `LicenseShortName` or `UsageTerms` is public domain (`Public domain`,
  `PD-old*`, `PD-Art*`, `PD-scan*`, `PD-US*`), `Public Domain Mark`, or `CC0`,
  **and**
- none of `LicenseShortName`, `UsageTerms` or the `License` slug names a
  restricted licence (CC BY, CC BY-SA, CC BY-NC, GFDL, "attribution",
  "share alike", ...).

Anything else is rejected, even though the underlying work is old: uploaders
re-license restorations. For example `File:Turdus pilaris John Gould. The birds of
Europe 2. 1837.jpg` is tagged CC BY-SA 4.0 and is rejected (it is not in any
work category, so the crawl never meets it; a direct lookup confirmed the tag).

Commons does not always agree with the real-world status, and the rule follows
Commons. Two cases matter in practice:

- **"No restrictions"** (184 file sightings, mostly Smithsonian/Internet Archive
  scans re-uploaded from Flickr Commons). Commons gives them
  `UsageTerms: No known copyright restrictions` and `Copyrighted: True`, so they
  fail both halves of the rule. They are probably free in fact. Accepting them
  would add **10** species that have no plate today. Not done: that is a decision
  for a person, not a script.
- **CC0 with `Copyrighted: True`** (4 files, all in `gould-britain`). Rejected for
  the same reason; the one species involved (*Turdus philomelos*) has a plate from
  another file.

## The matching rule

1. A file maps to a species through its Commons categories of the form
   `Category:<Genus species> (illustrations)`. The title's binomial is the species.
   Genus-level, family-level and trinomial categories do not count.
2. The binomial must equal a scientific name in the BirdNET V2.4 label list
   (`crates/birdnet-core/data/labels/birdnet-v24-en.txt`; 6,522 lines, 6,515 of them
   binomials; the other seven are `Dog`, `Engine`, `Environmental`, `Fireworks`,
   `Gun`, `Noise`, `Siren`). Exact match only: no synonyms, no fuzzy matching. A
   Commons name that BirdNET spells differently (for example `Astur gentilis` against
   `Accipiter gentilis`) is dropped and listed in the report.
3. **One species per plate.** A file in two or more distinct `(illustrations)`
   categories is rejected as multi-species, even if only one of them is a BirdNET
   label.
4. If a file has no `(illustrations)` category at all, the binomial at the start of
   its file name may be used, only if it exactly matches a label, and the entry is
   marked `matched_by: "filename"`. A file whose name contains a second label
   binomial is rejected as multi-species. A common name (`...Sparrow Hawk.jpg`) is
   never looked up.

## Quality and choice

- Width at least 800 px; MIME type `image/jpeg` or `image/png` (PDF, TIFF and GIF are
  refused); and no title-page, cover, index, text, map, frontispiece, contents,
  dedication, preface, introduction, errata, subscribers, binding or advertisement
  word in the file name.
- If a species has several acceptable files across the works, the one chosen is the
  widest; then `gould-*` over `naumann` over `von-wright`; then file title, then
  work key.
- The index holds one plate per species. To ship fewer works, rebuild from the
  cache with `--works` rather than deleting entries by hand.

## Coverage (run of 2026-10-28)

Counts are file sightings per work; a file is sorted into the first outcome that
applies, in the order of the columns. The sum equals "seen".

| Work | Seen | Licence | Multi | Small / non-plate | No species | Not a label | Accepted | Species (any) | In index |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `gould-europe` | 659 | 177 | 6 | 14 | 276 | 19 | 167 | 138 | 111 |
| `naumann` | 777 | 1 | 85 | 161 | 346 | 19 | 165 | 144 | 77 |
| `von-wright` | 411 | 340 | 0 | 16 | 0 | 2 | 53 | 40 | 21 |
| `gould-asia` | 739 | 5 | 1 | 32 | 336 | 93 | 272 | 176 | 175 |
| `gould-britain` | 75 | 24 | 0 | 32 | 3 | 0 | 16 | 15 | 12 |
| `gould-century` | 94 | 0 | 1 | 2 | 61 | 8 | 22 | 20 | 15 |
| `gould-australia` | 815 | 683 | 4 | 28 | 81 | 11 | 8 | 8 | 7 |
| `gould-new-guinea` | 378 | 2 | 0 | 35 | 260 | 24 | 57 | 40 | 40 |
| **All** | **3,948** | **1,232** | **97** | **320** | **1,363** | **176** | **760** | | **458** |

"Species (any)" is the species with an accepted file in that work; "In index" is
the species whose chosen plate is from it (a species with plates in several works
counts once). 3,948 sightings are 3,947 distinct files: `KaleejPheasant.jpg` is in
both `gould-asia` and `gould-century`.

Licences rejected (files, all works): CC BY-SA 4.0 1,044 (von Wright 340, Australia
683, Great Britain 19, Europe 1, Naumann 1); No restrictions 184 (Europe 176,
Asia 5, New Guinea 2, Great Britain 1); CC0 with `Copyrighted: True` 4. The
per-work split is in `report.md`.

- Species in the index: **458** of the 6,515 BirdNET binomials (**7.03 %**).
- Species that have no plate **only** because of licence: 46 (32 lost only to CC BY-SA,
  10 only to "No restrictions", 4 to both); listed in `report.md`.
- 126 distinct Commons binomials (176 files) are not exact BirdNET labels and were
  dropped; the top of that list is in `report.md`.
- Fixed sample of 40 common species (20 European, 20 North American; the list is
  `SAMPLE_SPECIES` in `build.py`): **17 of 40** have a plate, 16 of 20 European and
  1 of 20 North American (*Turdus migratorius*). The European gaps are *Parus major*,
  *Turdus pilaris*, *Corvus corone* and *Apus apus*. The works are European, Asian
  and Australasian, so North America is nearly empty by construction.
- All 458 chosen files carry the licence `Public domain`. 426 are matched by
  category, 32 by file name. The file's free-text `Artist` and `Credit` are read but not
  kept: nothing uses them (a plate is credited to its work's author), and on Commons they
  are whatever the uploader typed (four Naumann strings carry an institutional email
  address).

## What the crawl does not see

- Only files in a work's category tree are read (the root and its sub-categories,
  skipping any `(illustrations)` category, which belongs to the taxon tree). A
  plate that exists on Commons only under a species category is not found. The
  example in the licence section is one.
- 1,363 files fall in "No species": no `(illustrations)` category, and no file-name
  binomial that is a BirdNET label. By a rough name test (a Flickr or BHL id, a page
  number) 308 of Naumann's 346 are page scans and 59 of the 61 in `gould-century` are
  plates titled by number (`TAB. XIV`); in `gould-europe` the plates are titled by common
  name (`GouldBirdsEuropeIVSabines Snipe Common Snipe.jpg`), and in `gould-new-guinea`
  the title starts with a binomial that BirdNET does not have (`Manucodia comrii - The
  Birds of New Guinea.jpg`). Matching the Century and Europe plates would need a plate-number or
  common-name table, which is guessing, so they stay unmatched.
- `Category:The birds of Great Britain` is William Lewin's 1795 work, not Gould's,
  and is not used. Gould's 1862-1873 plates are in
  `Category:The Birds of Great Britain (illustrations by John Gould)`.
- Naumann's earlier work (*Naturgeschichte der Vögel Deutschlands*,
  `Category:Naturgeschichte der Vögel Deutschlands (Naumann)`, 352 files) is a
  different book and is not in the `naumann` key.
- The `years` of `naumann` and `von-wright` are empty on purpose: none could be
  verified, and the script does not guess.

## Looking at the plates

A plate is a scanned book page. Many show a male and a female (or adult and
young) of the one species, a printed caption and plate number, and the toned edge
of the page; a few are cleaned cut-outs on white. Use `--fetch-thumbs DIR` and look
at the pictures before relying on the index for anything layout-sensitive.
