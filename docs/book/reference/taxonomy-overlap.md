# Taxonomy Overlap Between Classifiers

The station merges what two classifiers heard **by scientific name**, as a string. Two classifiers that name one bird differently are, to the station, two species: a detection each, with `model_agreement` 1, and no row saying that they agreed. This page says how often the three classifiers' names meet, name by name, and what it costs. The numbers were recomputed from the label files; the table of 644 names is `docs/data/taxonomy/v3-names-not-in-perch.tsv`, and a test (`crates/birdnet-core/tests/the_v3_names_perch_lacks.rs`) holds it to the Perch labels.

## How many names meet

| | classes | in BirdNET V2.4 | in BirdNET+ V3.0 preview 3 | in Perch v2 |
|---|---|---|---|---|
| BirdNET V2.4 | 6 522 | all | 6 325 | 6 266 |
| BirdNET+ V3.0 preview 3 | 11 560 | 6 325 | all | **10 916** |
| Perch v2 | 14 795 (14 595 species, 200 sound classes) | 6 266 | 10 916 | all |

All three share 6 253 names. Of V3.0's 11 560, 10 916 (94.4%) are also a Perch class, written the same way; **644 are not**. V2.4 has 197 classes V3.0 lacks under the same name.

## The 644, one by one

Each of the 644 was looked up in the eBird/Clements taxonomy (every annual version from 2021 to 2026, by eBird code), the IOC World Bird List 15.2 and its cross-reference to other checklists, iNaturalist's taxon records and the GBIF backbone (2023). The record ids are in the table's last column, so every row can be checked again. Mechanically, none of the 644 is a Perch class and every Perch name the table gives as a counterpart is one; the rest is a reading of those sources, with a confidence for each.

| Relation | Names | What it means | Confidence (high / medium / low) |
|---|---|---|---|
| `absent_from_perch` | 369 | no Perch class for this species or any of its synonyms | 338 / 18 / 13 |
| `perch_lumps_it` | 129 | Perch has the species this one was merged into (*Acanthis cabaret* and *A. hornemanni* are in Perch as *A. flammea*) | 120 / 5 / 4 |
| `same_species_other_name` | 121 | one species under two names, 109 of them a change of genus (*Hypsiboas albomarginatus* is *Boana albomarginata*; V3.0's *Charadrius alticola* is Perch's *Anarhynchus alticola*) | 95 / 24 / 2 |
| `uncertain` | 11 | the sources disagree | 0 / 0 / 11 |
| `perch_splits_it` | 8 | Perch lists some of the species this one was split into | 1 / 7 / 0 |
| `subspecies_or_form_only` | 6 | V3.0 names a subspecies or form; Perch has the species | 0 / 6 / 0 |

Of the 121 renames, 101 rest on two or more independent sources and 20 on one. **A table of verified aliases would recover 121 of the 644 (19%; 1% of V3.0's classes)**, taking exact-name coverage from 10 916 to 11 037 (95.5%). The other 523 are not a naming problem: 369 are species Perch has no class for, and 129 are lumps, which are a merge decision and not a synonym. The 121 are mostly an old-genus to new-genus table: 18 of them are *Hypsiboas* to *Boana* and 15 *Charadrius* to *Anarhynchus*.

Of the 644, 351 are birds, 107 mammals, 95 amphibians, 75 insects and the rest fish, reptiles, plants and one unclassified. Perch's own class list is the other half of the overlap: 3 879 of its 14 795 classes (3 679 species and the 200 sound classes) are not in V3.0.

## What it costs the station

- **A bird the two classifiers name differently is two detections.** The merge is a union by scientific name, and agreement counts classifiers that gave the *same* name. For the 121 renamed species, V3.0 and Perch hearing one bird give two rows, neither with `model_agreement` 2, and the Who heard it card on each shows the other classifier as having no class for that species.
- **A lumped species is reported by Perch and not by V3.0, or the other way round**, because the two classes are different species to the station.
- **369 V3.0 species cannot be reported by Perch at all**, and V3.0 can say nothing about the 3 679 species Perch has and it does not.

## Why there is no alias table yet

The aliases are known and the 101 well-supported ones could be applied. They are not, because a name is the key of more than the merge: the species list, the history, the exports, the watchlists and the daily summaries all group by it. Mapping two names to one in the merge but not in those would put a species under a name the rest of the station never shows. It needs a decision about which name a stored detection carries and what happens to the detections already stored under the other, and that is a change of its own. The table is the input for it.

## Common names

Perch's list carries scientific names only. The station gives its birds an English name from BirdNET V2.4's names where V2.4 has the species (6 266 classes), and from the IOC World Bird List (CC BY 3.0, credited in `crates/birdnet-core/data/labels/README.md`) for 3 917 more; a name two species would share is held back, which is why a recently renamed bird can show its scientific name. The README says which and why.

## What is not established

The relation column is a reading of public taxonomies on one day (2026-11-05). Their licences were read for what the table keeps: scientific names, the relation, and record ids; V3.0's common names are not copied (its labels are CC BY-SA 4.0, which this project's CC BY-NC-SA 4.0 cannot adapt). The reverse direction, Perch's bird classes with no same-name V3.0 class (821 of 10 307), was checked in the same way and is not tabulated. The lumps are lumps by the eBird/Clements taxonomy of 2024 and later, which Perch's bird names appear to follow (inferred: of the eBird names first used in 2024, 254 of 368 are Perch classes, and none of those first used in 2025 or 2026); Perch's own documentation says only that its classes follow the iNaturalist taxonomy.
