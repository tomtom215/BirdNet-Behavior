# UX audit, October 2026

What was looked at, what was found, what was changed, and what was not checked. Findings
are in the order they were acted on. Every number here comes from a command named in
the "How it was checked" column or section; anything not checked is in the last section.

**Scope.** The 42 routes the `tools/visual-qa` gates covered when this started (and `/system/credits`, added
to them afterwards), rendered in Chromium from the `screenshot_server` fixture (a North American station
with 15 species) in the light and dark themes at a desktop and a phone width (axe: 168 route, theme and
viewport states on the 42). Not
covered: real users, a Raspberry Pi's screen or timing, a screen reader, Safari.

## Findings

| # | Finding | Severity | Status | How it was checked |
|---|---|---|---|---|
| 1 | The Patterns tabs each opened differently: twenty-nine `*-h1` classes, the help link in the eyebrow on four tabs and under the heading on two, chart cards hand-assembled with an `h2` or `h3` as the author chose. | medium | Fixed in `013dad2` (`pages/frame.rs`: `page_head`, `figure`). Recordings and the three Reports tabs moved onto it afterwards. Today keeps its own hero (open item 12). | `every_home_opens_with_the_shared_page_head` (10 pages: one head, one `<h1>`, the help link in the eyebrow; Today is held to the last two). Seen failing with the old Reports and Recordings heads restored. |
| 2 | The When tab carried two range controls that did not agree; Trends redrew the hour-of-day heatmap, the polar clock and accumulation a second time. | medium | Fixed in `013dad2`. | `the_when_tab_has_one_range_control`, seen failing against the old `terrain.html`. |
| 3 | The weekly report held this week so far against the whole of last week, so until Sunday every station read "A quieter week". | high (a wrong statement) | Fixed in `013dad2`: it compares the days that are over with the same days last week and says nothing on a Monday morning. | `the_weekly_report_compares_like_with_like`, seen failing on a Wednesday ("69% quieter than the week before" for a week matching last week's same days). |
| 4 | Species photographs were shown with no credit anywhere on a page. `NOTICE` and the manual said "attribution shown on each image"; for AviCommons and Flickr the credit existed only in the `description` of a JSON answer nobody read, and a Wikipedia picture had none. CC BY and CC BY-SA require it. | high (licence) | Fixed in the library and the web layer: each picture is kept with title, author, source and licence, shown under the picture, in the image JSON (`credit`) and on `/system/credits`, which every footer links to. A picture whose credit cannot be read, or whose licence is ND or NC, is refused; a Flickr CC BY-ND is no longer requested because the kiosk engraves (adapts) photographs. A picture cached before credits were kept is replaced on its next fetch, not given a guessed credit. | `pictures_are_shown_with_their_credit` (8 tests, each with a counterpart), the credit tests in `birdnet-integrations`; two mutants were killed: no credit printed, and no `credit` in the JSON (the second only after a fresh-fetch test was added). The ND/NC refusal is covered by `no_derivatives_and_non_commercial_licences_are_restricted` and the provider tests; no mutant was run against it. |
| 5 | Fourteen hand-written figure cards (Patterns, the analytics dashboards) carried a "Fig. N" number without the `fig` class, so under the Plate chart style they stayed flat cards among sheets. | low | Fixed. | `every_card_that_numbers_itself_is_a_figure_sheet`: red with the 14 cards named, green after. |
| 6 | No chart said what it found. | medium | `pages/insight.rs` writes one sentence from numbers the chart already holds and is silent below 30 detections. The soundscape terrain now says whether song gathers at sunrise (at least 1.25 times an even spread, at most 0.8 times, or about the same) and marks sunrise and sunset on each day. | `the_sunrise_reading_follows_the_comparison_both_ways` and `a_dawn_concentrated_soundscape_is_compared_with_an_even_spread` hold the readings; the sun offset is the one in force on that day (see "Not verified"). |
| 7 | The first version of the Plate chart style hatched everything, sequential scales included, and was harder to read. A reader said so; then it was measured. | medium | Redesigned: a categorical series is its own colour with strokes cut across it; sequential scales are flat. | With the lines averaged into the area, adjacent steps of the heat-grid legend differed by ΔL\* 5.3 to 5.7 hatched against 6.9 to 9.2 flat (light theme); after the change the legend measures the same as standard. Stroke contrast against every series colour: 3.19:1 or better in light, 4.49:1 in dark (`a_plate_stroke_can_be_seen_on_every_colour_it_crosses`; mutants to the two rejected strokes failed at 2.09:1 and 2.93:1). Closest pair of series by ΔE76: 24.7 light, 22.1 dark, identical to standard. axe 0 violations on 168 states and layout 0 findings on 336 with the style on (`ILLUSTRATION=plate`). |
| 8 | The kiosk's photographs were cropped (birds cut off) and the approach did not scale: a licence-clean source of pictures was needed. | high | The photo is shown whole and engraved; a public-domain plate is tried first. | See "The plate tier". |
| 9 | The new Picture credits page, as first built, was crushed on a phone: five columns of names squeezed so words broke mid-word ("Wikimedi/a Common/s", "Chickade/e"), and the table was marked as meant to be read whole on a phone. axe and the layout gate were both clean. | medium | Fixed: below 520 px each row stacks and each cell prints its column's name; the second table has its own label and a gap. | Found by looking at a 390 px screenshot, not by a gate: nothing in `tools/visual-qa` detects a word broken mid-word. `the_credits_page_lists_each_credited_picture_and_counts_the_rest` now asserts the cell labels (red against a mutant that drops one). `/system/credits` is in the gates' route list, and the fixture holds credited pictures, one uncredited one and a plate for it to show. |

## The plate tier

The index (`tools/plate-index/`, `crates/birdnet-integrations/data/plates/index.json`) names one
Commons file per species from Gould's bird books, Naumann's *Naturgeschichte der Vögel Mitteleuropas*
and the von Wrights' *Svenska fåglar*, only where Commons calls it public domain, CC0 or Public
Domain Mark and it shows one species. Measured, from `tools/plate-index/report.md` and the file:

- 458 species of BirdNET V2.4's 6,515 (7.0%); all 458 entries are keys in the V2.4 label list.
- 17 of a fixed sample of 40 common European and North American species; **1 of the 15** species in
  the North American fixture (American Robin). The sources are European and Asian. A North
  American station will mostly see engraved photographs, as before.
- The licence rule costs at most 46 species (those whose only files were tagged CC BY-SA by their
  uploader, or "No restrictions" with `Copyrighted=True`).
- 12 plates were fetched and looked at in a contact sheet: each is one plate of one or two birds of a kind, legible; several carry the
  book's binding and a dark scan border, one is a full-bleed chromolithograph. Whether each is the bird the index names was not checked against the plate's own caption. The browser cuts the
  page, cuts to the ink, and levels the paper only where the band inside the edge is paper.

The Commons file's free-text author is not used in the credit (for Naumann it holds a digitiser's
name and an email address); a plate is credited to its work's author.

## What is still open

| # | Item | Severity |
|---|---|---|
| 10 | Plates are drawn smaller than photographs: a folio plate is portrait and the figure boxes are landscape, so the birds fill the height and not the width. A box shape that follows the picture is the next improvement; it needs the layout to learn the shape before the picture arrives. | low |
| 11 | The empty, loading and failed vocabulary is still page by page (`error_states.rs` is shared by some). One shared set was not built. | low |
| 12 | The Today hero is its own markup, at hero size, with status pills where other pages have a lede, by design (its headline is written from the day's data). It is held to the shared head's two structural rules, not to `page_head`. | low |
| 13 | The soundscape terrain is a canvas and stays flat colour under the Plate style (its key is flat too). | low |
| 14 | Legacy pictures with no credit are only replaced when asked for; a station that never opens a species page keeps them uncredited (the credits page counts them). | low |

## Not verified

- The engraving was judged on four real photographs and synthetic ones, the plate tier on twelve real plates,
  in desktop Chromium only. Nothing was timed on a Raspberry Pi.
- The sunrise and sunset lines use the UTC offset SQLite reports for each day. The container's time zone is UTC,
  and setting another one needs `unsafe`, so a day that crosses a change of clocks has not been run.
- `scripts/check-ci-config.py` could not run here (no `cargo-mutants`).
- arm64, macOS, Firefox and WebKit were not run on this change.
- The Plate tier's console shows a `404` for every species without a plate (the normal case). It is a design choice,
  noted here because it looks like an error in a browser's log.
