# Feature gap analysis

> What BirdNet-Behavior does **not** yet do, measured against the two projects
> that share its problem domain:
>
> * [`Nachtzuster/BirdNET-Pi`](https://github.com/Nachtzuster/BirdNET-Pi) — the
>   surviving maintained fork of the PHP/Python original this project descends
>   from. Compared at `88985a3` (2026‑02‑28).
> * [`tphakala/birdnet-go`](https://github.com/tphakala/birdnet-go) — an
>   independent Go rewrite of the same idea: about twenty-five times
>   BirdNET-Pi's size and about twice this project's by total lines (table below).
>   Compared at `1e74c82` (2026‑09‑02), then re-checked at `780ce51`
>   (2026‑10‑11), where the 239 commits between are triaged in **Part 5**;
>   `f4f019a` (2026‑10‑15), 37 more commits, **Part 7**; `a1a77af`, 65 more,
>   found by the parity audit of 2026‑10‑20 and listed in **Part 8**; and
>   `30d163ded` (2026‑10‑20), 19 more. **Part 9 (2026‑10‑21)** is the first part
>   whose upstream citations were opened and read; it re-verifies Part 8, classifies
>   all 84 commits since `f4f019a`, and enumerates both upstreams' features.
>   `Nachtzuster/BirdNET-Pi` was re-read in full on 2026‑10‑21 and is still at
>   `88985a3`.

## How this was measured

Both repositories were cloned and read, not summarised from memory. Sizes, for
scale — and each names the tip it was measured at rather than inheriting the
comparison's, because these numbers move: upstream at `88985a3` and
`b184f689` (2026‑09‑03), ours at `ee795ed`; those three are `git ls-files`
line counts with tests included. Our row was counted again on 2026‑10‑20, at
the commit this was edited on, with
`find src crates -name '*.rs' | xargs cat | wc -l`, which counts `.rs` files
only and so is not the same measure as the two upstream rows; the upstream rows
were not counted again.

| Project | Language | Lines | Notes |
|---|---|---|---|
| Nachtzuster/BirdNET-Pi | PHP + Python + shell | 24 961 at `88985a3` (re-counted 2026‑10‑21), of which 18 145 is the fork's own code — the rest is vendored Adminer and file-manager PHP | 188 tracked files |
| tphakala/birdnet-go | Go + Svelte | 631 437 Go lines at `30d163ded` (292 215 excluding `_test.go`); ~542 k (~265 k) at `b184f689`, 2026‑09‑03 | 3 438 tracked files; 55 directories under `internal/` (52 subtrees at `b184f689`) |
| **BirdNet-Behavior** | Rust | 293 541 `.rs` lines in 654 files at `ea2d439` (`git archive HEAD src crates`, 2026‑10‑21); 276 926 on 2026‑10‑20 by `find`; ~200 k at `ee795ed` | 8 crates + binary |

Every row below carries the upstream file that is the evidence for the claim
and the file in this repository that is the evidence for our state. A row that
says a thing is missing was checked by grep against the whole workspace, and
where the first grep hit was a false positive (a comment, a similarly-named
unrelated symbol) that is recorded rather than quietly dropped — several of the
"absent" verdicts in the first pass turned out to be exactly that.

**Verdicts** are one of:

* `GAP` — they have a capability we do not, and it is worth having.
* `PARTIAL` — we have some of it; the remainder is worth having.
* `SHIPPED` — was a `GAP` or a `PARTIAL`, and the work has since landed. The
  row is kept, rewritten to say what is in our source now, so that a
  `G‑NN` cross-reference from elsewhere in this document does not send a
  reader to a gap that was closed.
* `PARITY` — we do the same thing, possibly differently.
* `DECLINED` — deliberate divergence, with the reason stated. These are not
  work items; they are recorded so that the next person to run this comparison
  does not re-open them.

Nothing is listed as done that has not been read in our source. The
reciprocal needed repairing and has been: nine Tier‑1 findings were still
written as open long after they shipped, because only the Tier‑1 **Status**
column was being kept current. A finding's own body is what a `G‑NN`
cross-reference lands on, so that is what has to be updated when work lands.

---

## Part 1 — versus Nachtzuster/BirdNET-Pi

The configuration file is the fork's own feature list, so it is the right place
to start. `scripts/install_config.sh` defines 57 settings. Mapping each onto
this project (`.env.example`, the `settings` table keys rendered by
`crates/birdnet-web/src/routes/admin/settings/render/`) leaves six genuine
gaps and a handful of deliberate divergences. (The fifth, N‑5, was missed by the
first mapping because the setting existed: `DATABASE_LANG` was a field on the
settings page and a bridged key, so it counted as present. Nothing used it. The sixth, N‑6, was found by the audit of 2026‑10‑20 and is the same mistake: `custom_image_dir` was taken to be BirdNET-Pi's `CUSTOM_IMAGE`, and it is not.)

### N‑1 · Flickr image provider — SHIPPED

| | |
|---|---|
| **Upstream** | `scripts/api.php:16` — `if ($config["IMAGE_PROVIDER"] === 'FLICKR') { $image_provider = new Flickr(); }`; settings `IMAGE_PROVIDER`, `FLICKR_API_KEY`, `FLICKR_FILTER_EMAIL` |
| **Ours** | The seam has a second occupant. `species_images/provider.rs:12` still defines the `ImageProvider` trait, and `species_images/flickr/` now implements it alongside `wikipedia.rs`. `SpeciesImages::from_settings` (`species_images/mod.rs:182`) picks between them on an `image_provider` setting. |
| **Why it matters** | Wikipedia/Wikimedia has no photograph at all for a long tail of species, and for many others has a museum skin or a range map. `FLICKR_FILTER_EMAIL` also lets an operator show *their own* photographs of the birds their own station heard, which is the single most-requested cosmetic feature in the upstream issue tracker. |
| **Resolution** | `species_images/flickr/` queries `flickr.photos.search` with `sort=relevance` and `license` restricted to the commercial-use-permitted set (`ALLOWED_LICENSES = "4,5,6,7,8,9,10"`, `flickr/mod.rs:60`), optionally narrowed to one photographer's photostream via `FLICKR_FILTER_EMAIL`. Attribution is carried, not assumed: the photographer's name goes in `SpeciesImage::description` and the photo-page link in `wiki_url`, which the species page already renders as its credit line. Selecting Flickr does **not** replace Wikipedia — `from_settings` wraps the two in `species_images/chain/`'s `FallbackProvider`, which falls through on `ImageError::NotFound` only; a network error or a rejected key stops the chain and is reported, so a broken `FLICKR_API_KEY` cannot hide behind Wikipedia's coverage. `FLICKR_API_KEY` is in the redaction list (`crates/birdnet-core/src/config/redact.rs:129`). |

### N‑2 · Frequency shift on the live stream — SHIPPED

| | |
|---|---|
| **Upstream** | `scripts/livestream.sh:15` — `if [ "$ACTIVATE_FREQSHIFT_IN_LIVESTREAM" == "true" ]; then FREQSHIFT_OPT='-af rubberband=pitch='${FREQSHIFT_LO}'/'${FREQSHIFT_HI}; fi`, applied to the Icecast MP3 source. |
| **Ours** | **This row was wrong when first written, and the correction is the finding.** It said "`routes/livestream.rs` streams the raw tap unshifted". It does not: `livestream.rs:444` builds `freq_shift_filter(freq_shift_hz)` (defined at `:169`; when this row was written the call was at `:275` and the definition at `:126`, and the function took the stream's sample rate too) and passes it to ffmpeg as `-af`, so `/stream?freq_shift_hz=N` has always worked. What was missing is that **nothing in the UI ever sent it** — `recordings.html`'s `srcFor()` built `/stream` or `/stream?source_id=…` and never a shift — so the feature was reachable only by hand-editing a URL. |
| **And a defect the re-check found** | Five doc comments, including the `--freq-shift-hz` CLI help an operator reads before choosing a value, said a **positive** (upward) shift "makes calls accessible to people with high-frequency hearing loss". That is backwards. Presbycusis takes the *top* of the range first, so an 8 kHz warbler is restored by moving it **down**. Upstream agrees and was checked as the primary source: `install_config.sh` ships `FREQSHIFT_HI=6000` / `FREQSHIFT_LO=3000` (a `rubberband` ratio of 0.5) and a sox `FREQSHIFT_PITCH=-1500` — two independent settings, both downward. A listener following our documentation would have shifted the song further out of their hearing. |
| **And a second defect, found 2026‑10‑20** | The filter this row calls working was `asetrate` followed by `aresample`, documented as shifting without stretching the audio. It scaled frequencies by a ratio: measured with ffmpeg 6.1.1 on an 8 kHz tone, `-3000` gave 7 500 Hz, not the 5 000 the text promised, and the audio was slowed by the same ratio, which on a live stream is a delay that grows. `bf05525e` replaced it, here and in the saved-clip path, with `afreqshift` (ffmpeg 4.3 or newer), a shift in hertz that leaves the length alone (`freq_shift_filter` in `routes/livestream.rs`, with tests that pin the filter string). The saved-clip path also lost its sox fallback, because sox cannot shift by an amount of hertz. |
| **Why it matters** | This is an accessibility feature, not a novelty. Age-related high-frequency hearing loss starts around 8 kHz; a great deal of warbler and kinglet song lives above it. A feature that works only if you know to hand-edit a query string is not available to the people it is for, and one documented in the wrong direction is worse than absent. |
| **Resolution** | A pitch control beside the Listen button on `/recordings`, with downward presets (the accessibility direction) and one upward option; the choice is remembered per browser in `localStorage`. Per-listener rather than upstream's station-wide flag, and deliberately: hearing loss is a property of a person, and this station serves one ffmpeg per connection rather than one Icecast broadcast for everyone, so it can do better than upstream here. All five doc comments corrected against the primary source, with `ACCESSIBILITY_SHIFT_HZ` naming the direction and a `const` assertion failing the *build* if its sign is ever flipped back. The `freq_shift_hz` query parameter is now clamped to ±24 kHz — it was an unbounded `i32` from an unauthenticated request, and `freq_shift_hz=2000000000` asked ffmpeg to resample from ~2 GHz, four streams at a time. |

### N‑3 · Choosing which RTSP source feeds the live stream — SHIPPED

| | |
|---|---|
| **Upstream** | `RTSP_STREAM_TO_LIVESTREAM` (an index into the comma-separated `RTSP_STREAM` list), consumed at `scripts/livestream.sh:26-36`. |
| **Ours** | Both halves are now in the source. **Per listener**: `GET /api/v2/stream` accepts `?source_id=<id>`, resolved against the `audio_sources` table — `routes/livestream.rs` declares the parameter and branches on it, an unknown id returns `404`, and the Recordings page's `srcFor()` builds it (`templates/recordings.html`), so a listener can choose a source from the UI. **Per station**: without the parameter, `resolve_default_source` consults the `livestream_source` setting and falls back to the first non-disabled row; a station with no rows at all still gets `503`. (This row previously read "`grep -rn livestream_source` finds nothing", which was true when it was written.) |
| **Why it matters** | A two-microphone station (feeder + nest box) has one of them that a person actually wants to listen to. |
| **Resolution** | `livestream_source` (`routes/livestream.rs`, `LIVESTREAM_SOURCE_SETTING`) names the default `audio_sources.id`. `resolve_default_source` reads it and hands it with the source list to `pick_default`, a pure function: the named row if it is still enabled, else the first enabled row. The fallback is deliberate — this is the path a visitor reaches by pressing Listen with no choice of their own, and a station whose named default was unplugged last week should keep streaming the microphone it still has. The shipped query parameter is **`?source_id=`** — an earlier draft of this row specified `?source=`, and implementing the default against that spelling would leave the station answering to two names for one thing. |
| **Where it is set** | A **Make listen default** button per row on `/admin/audio`, not a field on the settings page: the value is an `audio_sources` row id, which belongs beside the rows rather than typed into a text box. The POST answers with *both* source lists as out-of-band swaps, because the row losing the pill is usually in the other one. Audited as `audio.source.listen_default`. |
| **And a gate the change had to add** | That made `/admin/audio` a third writer of the `settings` table, and the guard that fails when a settings key nothing reads is shipped covered only the admin form and the first-run wizard — the two writers whose past mistakes created it. `AUDIO_ADMIN_SETTING_KEYS` and `audio_admin_keys_are_all_classified` extend it to this one. That made three hand-maintained per-writer lists, and a fourth writer — `routes/admin/migration.rs`, writing `analytics_exclude_imports` — was outside all of them. G‑34's drift gate has since replaced the three lists with a scan of the source and caught exactly that key. |

### N‑4 · Bulk species management — SHIPPED

| | |
|---|---|
| **Upstream** | `scripts/species_tools.php` — per-species on-disk clip counts (`disk_species_count.sh`), bulk delete of a species' detections *and* its files, and confirm/exclude/whitelist list editing from the same table. |
| **Ours** | Per-species retention and purge exist (`crates/birdnet-core/src/audio/capture/disk/purge.rs`, driven by `max_files_per_species` on the disk manager, `disk/manager.rs:47`), the quarantine review flow exists (`routes/pages/quarantine.rs`), and include/exclude editing exists (`routes/admin/species/`). What is missing is the single table that shows *every* species with its clip count and offers "delete this species entirely". |
| **Why it matters** | The recurring real-world need is "my station has logged 4 000 phantom Eurasian Wrens from a squeaky gate; remove them and stop recording them". Today that is three separate screens. |
| **Plan** | Add `/admin/species/manage`: one row per species with detection count, on-disk clip count and bytes, last-heard date, and current list membership; actions are *exclude*, *delete detections*, *delete clips*, each confirmed and audit-logged through the existing `audit_log` table. Counts come from a new `species_disk_usage` query in `birdnet-db` rather than shelling out. |
| **Resolution** | Shipped as planned: `species_disk_usage` in `crates/birdnet-db/src/sqlite/queries/species.rs`, the page at `crates/birdnet-web/src/routes/admin/species/manage.rs`, three actions audited as `species.exclude.add`, `species.detections.delete` and `species.clips.delete`. Admin-gated by inheritance — `admin_routes()` is cookie-gated as a unit — and registered in the nav subpage gate, which live-requests it under its parent tab. |
| **The decision the plan did not anticipate** | **Locked detections are never touched by a bulk action.** `delete_detection` does not check `is_locked`, and that is right for a single row the operator is looking at; a bulk action is issued against a *species* and sweeps up rows nobody is thinking about. So every action here skips locked rows, `SpeciesUsage.locked` is shown **before** the button, and `BulkOutcome.locked_skipped` is reported after — an operator not told would find two detections of a species they believe they deleted and conclude the button is broken. Deleting clips keeps the rows and filenames (migration 22's provenance point) and marks them reclaimed *before* unlinking, because a row marked pruned whose file survives merely wastes disk while a file removed without the mark offers a player for audio that is gone. |
| **Bytes are measured, not tabulated** | Clip size is not in the database, so a byte total is `stat` per file — thousands of syscalls for the species this page exists for. It is a per-row **Measure** action rather than a column: a page that took twenty seconds to open on an SD card would not be worth the number. |
| **A path that unlinks** | `File_Name` comes from the database, which on a migrated station holds whatever BirdNET-Pi wrote there, and this page removes what it resolves. Paths are canonicalised and checked to be under the recordings directory, with a gate that puts a file outside the tree and asserts it survives a `../` filename. |
| **A gate that was green for the wrong reason** | The "worst first" ordering test used a fixture in which detection-count order and alphabetical order coincided, so a mutation replacing `ORDER BY count DESC` with `ORDER BY name` passed. Found by running the mutation rather than by reading the test. The fixture now makes the two orders disagree, and the repaired gate also catches a rejected-detections mutation it could not have caught before. |

### N‑5 · Species names in the station's language — SHIPPED

| | |
|---|---|
| **Upstream** | `DATABASE_LANG`, and `model/l18n/labels_<code>.json`: 36 files of 7,058 `{scientific: common}` pairs each, activated by `install_language_label.sh`. BirdNET-Pi writes the translated name into the detection row. |
| **Ours before** | A dead control. `init_i18n` built a label manager only when `--labels-dir` pointed at `{lang}_labels.txt` files that nothing shipped; no code called `translate` on a name (`git grep` at `378afbe` finds none outside `i18n.rs`); `GET /api/v2/languages` listed all 36 as available whether or not anything could be shown; and `--help` named the files `labels_de.txt` where the loader reads `de_labels.txt`. The settings-page guard passed because the key *is read*; nothing asked whether the result is *used*. |
| **Why it matters** | A station whose owner does not read English bird names cannot read its own Today page. |
| **Resolution** | The tables are compiled into `birdnet-core` (`data/l18n/`, reduced to genuine translations, with per-file sha256, licence and a coverage table that a test holds equal to the data). **Read-time, not write-time** — a deliberate divergence: the stored name stays the classifier's, so changing language splits no history and species lists keep matching; `NameLocalizer` resolves a stored name to the name to show, keyed by scientific name. Every page, feed, notification and the live feed draws through it, and a name typed into a search box is understood in the station's language. |
| **How it is held** | `every_name_a_reader_sees_is_in_the_stations_language` fetches the pages from a German station and fails on any English name a reader sees, fails when a link, form or API field carries a translated one, and fails on any web-crate site that escapes a stored name and is not classified. `the_language_setting_is_a_language_we_have`, `species_names_table.rs`, and a doctor check cover the rest. |
| **Still open** | Coverage is uneven (Korean 8.8% of the table's 7,058, Swedish 98.4%) and the BirdNET+ V3.0 classifier has 11,560 species, of which 6,632 have a name in at least one language. The interface text is not translated. The table's licence fit is the maintainer's to decide (`HANDOFF.md`). |

### N‑6 · A dashboard picture with a title — GAP

| | |
|---|---|
| **Upstream** | BirdNET-Pi's `CUSTOM_IMAGE`: one picture of the operator's choosing, with a title, on the Overview page (`scripts/overview.php:19-27`); the audit also names a webcam snapshot. The citation is the 2026‑10‑20 audit's. It was not re-read for this row, because upstream was not reachable from the session that wrote it. |
| **Ours** | Nothing shows one picture on a dashboard page. `custom_image_dir` (`--custom-image-dir`) is something else: `species_image_file` in `routes/images.rs` looks in that directory for a file named `{lowercase_scientific_name_with_underscores}` with the extension `jpg`, `jpeg`, `png` or `webp`, before the Wikipedia cache, so it replaces one species' picture. Its code comment, the hint on the Settings page (`routes/admin/settings/render/system.rs`) and its `--help` text all call it BirdNET-Pi's `CUSTOM_IMAGE`. By the audit's reading of upstream it is not that. |
| **Why it matters** | A wall-mounted display (G‑22) is where a photograph of the site, or a feeder camera frame, belongs, and there is no place to put one. |
| **Plan** | None yet. Whoever builds it also owes the three labels above a correction. |

### Deliberate divergences from BirdNET-Pi — DECLINED

These are not gaps. They are recorded so the comparison does not keep
re-discovering them.

| Upstream feature | Why we do not have it |
|---|---|
| Adminer (`scripts/adminer.php`) — a full SQL console in the web UI | An unauthenticated-by-default SQL console on a LAN appliance is a remote-code-execution surface. `birdnet-behavior --check-db`, the backup/restore flow, and the read-only query views cover the legitimate uses. |
| File manager (`scripts/filemanager/`) | Same reasoning; arbitrary filesystem write through the web UI. Recording browse/delete is offered narrowly at `/recordings` with path-traversal defences (`routes/recordings.rs`). |
| Web terminal (gotty) | A root-capable shell over HTTP. Declined outright. |
| `phpsysinfo` iframe | Replaced by `/station/…` and `/api/v2/system/*`, which read the same `/proc` and `/sys` data in-process. |
| Streamlit / Plotly stats app (`scripts/plotly_streamlit.py`) | A second Python runtime and a second web server for one page. Our `/pages/*` analytics and the DuckDB behavioural queries cover it without the dependency. |
| Icecast2 (`ICE_PWD`) | We stream MP3 directly over HTTP chunked transfer (`routes/livestream.rs`), which removes a service, a port and a password. |
| `git`-based self-update and the "commits behind" badge | We ship a single binary; `auto_update/` checks GitHub Releases and swaps the binary atomically. A source checkout is not part of the deployment. |
| `SILENCE_UPDATE_INDICATOR` | Exists only to hide the above badge. |

---

## Part 2 — versus tphakala/birdnet-go

birdnet-go was roughly 2.7× this project by line count when both were counted (about 542 k Go against ~200 k Rust, ours at `ee795ed`); against our 276 926 `.rs` lines of 2026‑10‑20 and its older count the ratio is about 2×, a figure that mixes dates. It has taken the design
in directions we have not. Sorting its capabilities against ours produces 34
findings (G‑1 … G‑34, contiguous). They are grouped by the part of the system they touch, and ordered
within each group by how much they change what a station can do.

### 2.1 Audio capture and conditioning

#### G‑1 · Sound level monitoring (ISO 266 ⅓-octave bands) — SHIPPED

| | |
|---|---|
| **Upstream** | `internal/audiocore/soundlevel/processor.go` — a bank of biquad bandpass filters on the 30 standard ⅓-octave centre frequencies from 25 Hz to 20 kHz (`octaveBandCenterFreqs`, ISO 266), each producing a 1-second RMS in dB, aggregated over a configurable interval into min/max/mean per band. Skips bands whose upper edge passes 0.95 × Nyquist because the biquad goes unstable there. Streamed at `GET /api/v2/soundlevels/stream` and exported to Prometheus (`internal/observability/metrics/soundlevel.go`). |
| **Ours** | Both measurements now exist, and they are different instruments. `crates/birdnet-core/src/audio/quality/` still computes the **single** broadband SNR, spectral flatness, adaptive noise floor and rain/wind flag (`types.rs:17`) that decide whether a chunk is worth classifying. Beside it, `crates/birdnet-core/src/audio/soundlevel/` — `bands.rs`, `filter.rs`, `meter.rs` — is the soundscape measurement, described by its own module header as *"what the site sounds like, not whether a chunk is worth classifying"*. |
| **Why it matters** | This is the difference between "was that chunk clean enough to classify" and "what does this site sound like". A banded SPL series is the standard unit of acoustic-ecology fieldwork: it is what shows a road opening, a generator running at night, a dawn chorus rising 12 dB in the 2–4 kHz bands over six weeks of spring. It also diagnoses the station itself — a microphone going deaf, a preamp oscillating, a mount picking up wind — none of which a broadband SNR separates from "quiet night". |
| **Resolution** | `crates/birdnet-core/src/audio/soundlevel/` ships as the three planned units — `filter.rs` (*"the third-octave band filter: three biquads in series"*), `bands.rs` (the ISO 266 centre frequencies and the A-weighting curve) and `meter.rs` (*"filter bank in, interval statistics out"*, carrying the `NYQUIST_MARGIN = 0.95` band exclusion at `:24`) — plus `tests.rs`. The biquad itself was built once and shared, as planned: `crates/birdnet-core/src/audio/biquad.rs` is what both this and G‑2's `eq` use. Served at `GET /api/v2/soundlevel` (`routes/system.rs:18`), which returns the newest third-octave spectrum for one source. Persistence went to **new** tables rather than the existing `audio_levels`: migration 37, *"Record third-octave band levels, so the station measures its soundscape"*, creates `sound_levels` and `sound_level_broadband`, because `audio_levels` keeps one broadband figure per source per hour and cannot hold the shape of a change. One piece of the plan did not land: the `birdnet_sound_level_db{band=…}` gauge family is **not** in `crates/birdnet-web/src/metrics.rs`, which has no `sound_level` mention at all. |

#### G‑2 · Per-source parametric equalizer — SHIPPED

| | |
|---|---|
| **Upstream** | `conf.EqualizerFilter` — a chain of filters each with `type` (LowPass/HighPass/BandPass/Peaking/…), `frequency`, `q`, `gain`, `width`, `passes`; global default plus a per-source and per-stream override (`Settings.ResolveEQOverride`), implemented in `internal/audiocore/equalizer`. |
| **Ours** | `crates/birdnet-core/src/audio/eq/` — a configurable filter chain per capture source, stored as the `audio_sources.eq_chain` column (migration 39) and edited at `/admin/audio` (`routes/admin/audio.rs:186`, parsed at `:411` by `EqChain::parse`). The three `AudioPipeline` booleans it replaces are still reachable: `EqChain::from_pipeline_flags` reproduces them exactly, and an empty `eq_chain` means a station's audio does not change on upgrade. |
| **Why it matters** | Sites differ in the noise they are fighting. A station next to a motorway needs a steeper low-cut than 120 Hz/one pole; a station under a fluorescent transformer needs a notch at 100/120 Hz that no high-pass provides; a hydrophone or a bat detector needs the band moved entirely. A fixed corner is a compromise picked for a garden. |
| **Resolution** | `audio::eq` shipped with the `EqChain` the plan describes and the biquad primitive shared with G‑1. The chain is stored per source in `audio_sources.eq_chain` (`ADD COLUMN eq_chain TEXT NOT NULL DEFAULT ''`, migration 39) and applied in both backends — as `-af` stages for ffmpeg sources, in-process for the tee. `EqChain::from_pipeline_flags` converts the three booleans into an explicit chain, and an empty `eq_chain` falls back to the old fixed 120 Hz high-pass and 5 Hz DC block (`capture/process.rs:311‑318`), so no station's audio moved on upgrade. |

#### G‑3 · Pre-capture across the segment boundary — SHIPPED

| | |
|---|---|
| **Upstream** | `conf.ExportSettings.PreCapture` — a live ring buffer sized `maxDuration + preCapture + margin` (`EffectiveCaptureBufferSeconds`), so a clip starts *before* the analysis window that triggered it regardless of where the trigger fell. |
| **Ours** | The window is no longer clamped to the segment. `extraction/extractor.rs:73` computes `lead_in = spacer + pre_capture_secs.max(0.0)` and hands the unclamped window to `extraction/span/`, whose module header records what the old behaviour cost: measured on a 15 s segment at 48 kHz with a 6 s extraction, a detection at 0.0 s produced a 4.5 s clip and one at 12.0 s produced another — **two of every five** windows at zero overlap. |
| **Why it matters** | With a 15-second segment and a 6-second extraction, one in ten clips starts inside the call. Those are the clips a person plays to decide whether the identification is right, and the ones uploaded to BirdWeather. The failure is invisible — the clip is a valid file of the right length, just missing its beginning. |
| **Resolution** | Both parts landed. `extraction/span/` resolves a window against the segment *and its neighbours*, reading the tail of the predecessor and the head of the successor when the window reaches them; `pre_capture_secs` lengthens the requested lead-in beyond the symmetric spacer, floored at zero rather than silently inverted (`extractor.rs:70‑73`). The guard that matters more than the fix is stated in the module's own header: **a neighbour is used only when it actually abuts**, so a restart, a dropped source or a purge cannot splice audio from two different times into one clip that looks continuous. |

#### G‑4 · Solar-relative quiet hours — SHIPPED

| | |
|---|---|
| **Upstream** | `conf.QuietHoursConfig` — `mode: "fixed"` (HH:MM) or `"solar"` (`startEvent: sunset` ± `startOffset` minutes → `endEvent: sunrise` ± `endOffset`), per source and per stream. |
| **Ours** | `crates/birdnet-db/src/audio_sources.rs:272` still types the window as `schedule_quiet: Option<(String, String)>`, but each endpoint is now either a clock time **or** a solar anchor with a signed offset — `sunset`, `sunset+30`, `sunrise-15`. The admin form validates that shape (`routes/admin/audio.rs:294`: *"Quiet window ends must be a clock time (22:00) or a solar anchor (sunset, sunset+30, sunrise-15)"*), and the solar maths in `crates/birdnet-scheduler/src/solar.rs` is what resolves it. |
| **Why it matters** | A fixed 22:00–06:00 window is wrong for eight months of the year at any latitude that matters. At 55° N sunrise moves by four hours between solstices; an operator who set quiet hours in January is recording two hours of dawn chorus into a disabled source by June, or burning CPU on two hours of daylight in December. |
| **Resolution** | Shipped, and more cheaply than planned: **no tagged form and no migration**. The two shapes share the one stored column and are unambiguous — *"a clock time contains a colon and no letters"* — so `parse_quiet_endpoint` (`src/capture/sources.rs:180`) reads `HH:MM` as `QuietEndpoint::Fixed` and `sunrise`/`sunset` with a required signed offset as `Sunrise`/`Sunset`. A bare `sunset30` is rejected rather than guessed at, and an offset beyond ±12 h is rejected because it has stopped meaning "around sunset". `/admin/audio` validates the same shape (`routes/admin/audio.rs:255`) so a value the form accepts is one the daemon can read back. |

#### G‑5 · Loudness normalisation of exported clips (EBU R128) — SHIPPED

| | |
|---|---|
| **Upstream** | `conf.NormalizationSettings` (`targetLUFS`, `truePeak`), applied as a single linear gain in `internal/audiocore/audionorm`. |
| **Ours** | Clips are written at capture level. `agc` exists as a capture-time toggle but is documented as mostly amplifying the noise floor, and is off by default. |
| **Why it matters** | A gallery of clips at wildly different levels is unusable — the listener rides the volume control between every one, and a quiet clip at the end of a playlist gets missed. Normalising the *export* (not the analysis input, which must stay untouched) is the standard fix. |
| **Resolution** | `crates/birdnet-core/src/audio/extraction/loudness.rs`: the two-stage K-weighting filter, 400 ms blocks at 75 % overlap, the `-0.691` offset, the absolute gate at −70 LUFS and the relative gate at 10 LU below the absolutely-gated mean. Mono, which is what every clip this station writes is. The filter is built from BS.1770's **analogue prototype** rather than transcribed from the standard's 48 kHz table, so a station capturing at 44.1 kHz is measured through a filter designed for 44.1 kHz. Off by default and set from Settings → Audio Capture (`clip_target_lufs`) or `BIRDNET_CLIP_TARGET_LUFS`; −18 LUFS recommended. Applied at write time only — the module lives under `extraction/` rather than beside `audio::soundlevel` so the tree says so. The measured LUFS goes into the clip's RIFF INFO comment (`Normalised to -18.0 LUFS from -31.4`). |
| **One deliberate shortfall, stated rather than glossed** | The ceiling is a **sample** peak, not an ITU true peak. BS.1770's true peak needs 4× oversampling through a specified interpolation filter and this does not do it; the −1 dBFS default leaves about a decibel of headroom for inter-sample peaks, which is the usual allowance. The module header says this in as many words, because a "true peak" that is not one is worse than an honest sample peak. |
| **What the verification found** | Two things worth keeping. The first version of the coefficient test carried a table typed from memory and **failed against a correct implementation** (`1.535123299202456` recalled against `1.53512485958697` derived); the construction was then checked line by line against libebur128's `ebur128_init_filter`. And that pinned test turned out to be the *more* sensitive of the two: replacing the shelf `Q` with `1/√2` — one part in ten thousand from the specified value — passes the 1 kHz gain-relationship test and is caught only by the pin. |

#### G‑6 · Extended capture for long calling sessions — GAP

| | |
|---|---|
| **Upstream** | `conf.ExtendedCaptureSettings` — for a configured species list, merge consecutive detections into one clip up to `maxDuration` (capped at 1200 s) instead of emitting one clip per window. |
| **Ours** | One clip per detection, deduplicated by `duplicate_interval_secs`. |
| **Why it matters** | An owl calling for six minutes, a nightjar churring, a woodpecker drumming session — these produce dozens of near-identical short clips today, which is both worse listening and more disk. |
| **Plan** | A session-merging stage in the extraction path keyed on species + source + gap: while the same species keeps being detected within the gap, extend the open clip rather than opening a new one. Bounded by `max_duration_secs` and by the segment-spanning machinery from G‑3, which this depends on. |

#### G‑7 · Stream protocols beyond RTSP — SHIPPED (RTSP, HTTP/Icecast, HLS, RTMP, UDP, SRT; `rtp://` declined, with the reason)

| | |
|---|---|
| **Upstream** | `conf.StreamType` — `rtsp`, `http` (direct/Icecast), `hls` (`.m3u8`), `rtmp` (OBS push), `udp` (RTP), each with transport selection. |
| **Ours** | One network source kind (stored as `rtsp`, for history) whose URL scheme decides the protocol: `birdnet_core::audio::capture::StreamProtocol`, read by `network_input_options` (`capture/process.rs`), which builds the ffmpeg input options for capture **and** for listening live (`routes/livestream.rs`). No second field, so the kind and the URL cannot disagree, and no table rebuild — `audio_sources.kind` has a `CHECK` that SQLite cannot alter in place. |
| **Why the options had to be per protocol** | ffmpeg 6.1 refused to open an HLS playlist given `-rtsp_transport` ("Option not found"), which every stream got; RTSP refuses `-rw_timeout` the same way; and RTMP reads `-timeout` as the time to wait for an *incoming* connection, turning the reader into a listener ("Address already in use"). RTSP keeps transport, `-timeout` and media mode; the rest get `-rw_timeout`. |
| **Verified** | The station binary against MediaMTX 1.21.2 (HLS, RTMP, SRT), an MPEG-TS UDP sender and a multi-client HTTP MP3 server, all ffmpeg 6.1: every source recorded 15 s, 48 kHz segments carrying its own fixture's tone (1500 / 2500 / 3500 Hz, by FFT of the segment). Listening live worked for HLS, RTMP, SRT and HTTP alongside capture. With MediaMTX frozen by `SIGSTOP`, RTMP and SRT capture exited and were restarted within seconds, HLS exited after 52 s (its playlist reloader retries first), and all three resumed on their own once the server did. The add form accepted all five and refused `rtp://` with the list of schemes. |
| **Not adopted** | `rtp://`: ffmpeg read a bare RTP stream carrying G.711 (a static payload type) and refused one carrying AAC ("Invalid data found"), which needs an SDP file. MPEG-TS over `udp://` describes itself and covers the case. RTMP is read *from* a server; the station does not accept a push. |
| **Limits** | A UDP stream has no live audio: a unicast datagram reaches one socket, and capture holds it (a second reader failed with "Address already in use", answered as an empty `200`). `/stream` now answers `503` with the reason, and the Live view lists the source as *spectrogram only*. |
| **Found on the way** | `--doctor` failed every stream URL that was not `rtsp://`, and its `Fail` stops the unit's `ExecStartPre` — a station configured with an Icecast URL would not have started. It now probes each protocol's default port and skips the UDP-based two. The log redactors passed a URL's query through, where SRT keeps its `passphrase=` and MediaMTX its `streamid=…:user:pass`; both are masked now. The Live view's source picker and the first-run Today checklist named an unlabelled source by its URL, `user:pass@` included, on pages open to anonymous visitors; they name it by its host now (`tests/a_visitor_never_sees_a_streams_password.rs`). And the docs described an "Add-RTSP wizard" with a locked prefix and a reachability pill that the web crate does not contain; the section now describes the form. The plan's "probing must learn to answer for them" did not apply: `capture/probe.rs` probes ALSA rates only, and a stream is resampled to 48 kHz by ffmpeg. |

#### G‑8 · RTSP media mode — SHIPPED (`full` and `audio`; `auto` not adopted, with the reason)

| | |
|---|---|
| **Upstream** | `conf.MediaMode` — `auto` (try audio-only, fall back), `audio-only` (never fall back, fail visibly), `full-stream` (request video and discard it). Default `full-stream`, with a comment naming the cameras that complete an audio-only handshake just long enough to mislead the fallback. |
| **Ours** | A per-source `media_mode` (`audio_sources`, migration 53), `full` by default or `audio`, which adds ffmpeg's `-allowed_media_types audio` — to capture (`capture/process.rs` `rtsp_command`) **and** to listening live (`routes/livestream.rs`), which opens its own session to the camera. Set on the add form and the edit form. |
| **Verified** | Against MediaMTX 1.21.2 with a published H264 + AAC stream, running the station binary: the audio-only row's sessions — capture and live — set up "1 track (MPEG-4 Audio)", the full row's "2 tracks (H264, MPEG-4 Audio)", and both recorded audible segments. An audio-only row pointed at a video-only path failed at PLAY (`501 Not Implemented`), logged at WARN per attempt, and recorded nothing. |
| **Why not `auto`** | Its fallback has to tell "this camera refuses audio-only" from every other reason a session fails, and upstream's own comment names cameras that pass the audio-only handshake and then misbehave — the case a fallback gets wrong. An operator who chooses `audio` and sees the source go down learns the same thing in one step, and can switch back. |
| **Found on the way** | The live stream hard-coded `-rtsp_transport tcp`, so a UDP-only camera could not be listened to; the add form described `auto` as "TCP w/ UDP fallback", which nothing implements (it is TCP); and transport could not be changed after a stream was added. All three fixed. |

#### G‑9 · Audio watchdog tuning — SHIPPED, and the row's rationale was misattributed

| | |
|---|---|
| **Upstream** | `conf.WatchdogSettings` — operator-tunable `checkInterval`, `silenceThreshold`, `maxRetries`, `retryBackoff`, `cooldown`, `escalationTimeout`, with an explicit ESCALATED→FAILED state machine (`internal/audiocore/liveness.go`). |
| **Ours** | We have a watchdog (`src/doctor/watchdog.rs`, `sd_notify.rs`, capture restart logic in `audio/capture/manager.rs`) and a deadman timer (`BIRDNET_DEADMAN_HOURS`), but the thresholds are constants. |
| **Why it matters — corrected** | The rationale this row carried was *"the right silence threshold at a busy feeder is not the right one for an arctic winter station where 30 s of silence is normal and 6 h is not"*. That describes `BIRDNET_DEADMAN_HOURS`, which is **already** an operator setting (`deadman_hours` on the settings form, bridged to `DEADMAN_HOURS`). Our stall threshold is not about silence at all: `stalled()` ages the newest *recording segment*, and a microphone writes segments through hours of quiet. The real case for tuning is narrower and still real — slow storage, long segments, a camera that legitimately pauses, a station whose journal is filling with repeat warnings. |
| **Resolution** | `src/capture/watchdog.rs`: `WatchdogConfig` with seven timings — reconcile cadence, stall segments and floor, backoff base and cap, down-warn delay and repeat — each read from `BIRDNET_WATCHDOG_*` or the unprefixed `birdnet.conf` key, clamped to a documented range, with every adjustment logged so a station never runs on timings its config file does not describe. Defaults are exactly the constants the supervisor used before, and a gate asserts that. |
| **Two knobs deliberately not added** | **A maximum retry count**, which upstream has: a field sensor unreachable for six hours must still be reachable on hour seven, and a supervisor that has given up is a station silently not recording. Offering it would be offering a way to break the property the supervisor exists to provide. And **the flapping threshold and window**, which live in `birdnet-core` because the web layer renders against them too — a per-station value would have to reach both, and half-wiring it is worse than leaving it fixed. |
| **Not exposed on `/station/capture`** | The plan said to. Seven expert numeric fields on a page an operator visits to see whether their microphone is alive is a poor trade; the values are in `.env.example` with their ranges, and the journal reports any that were adjusted. Recorded as a deliberate divergence rather than left as an open item. |

### 2.2 Classification and detection quality

#### G‑10 · Multiple classifier models — IN PROGRESS (Stages 1–5 shipped; Stage 6 blocked, with the reasons)

| | |
|---|---|
| **Upstream** | `internal/classifier/` (≈44 k lines). An orchestrator running any of: BirdNET v2.4, BirdNET v3.0, Google **Perch v2**, a **bat** classifier built on BirdNET v2.4 embeddings, and **BSG regional** models — concurrently, routed per audio source (`AudioSourceConfig.Models`), each with its own labels, locale and threshold. A model catalog with regional variants (`model_catalog.go`, `model_catalog_regional_gen.go`), a download manager pulling from HuggingFace with a configurable endpoint for mirrors, primary-model swap and failover (`model_manager*.go`), and `GET /api/v2/models/catalog` · `POST /api/v2/models/install/:id`. |
| **Ours** | One BirdNET ONNX classifier plus the metadata/geomodel (`crates/birdnet-core/src/inference/model.rs`, `species_filter.rs`). Model and labels are paths given by config; `scripts/setup-onnxruntime.sh` seeds the runtime. The only workspace mention of Perch is a comment in `audio/resample.rs:4` noting it wants 32 kHz. |
| **Why it matters** | Two distinct things. (a) **Coverage**: BirdNET is weakest exactly where a hobbyist most wants help — outside Europe/North America, and on non-birds. Perch v2 is materially better in the tropics; a bat classifier turns one box into two instruments. (b) **Corroboration**: two independent models agreeing is far stronger evidence than one model being confident, and it is the honest way to attack the false-positive problem that our `corroboration.rs` attacks with repetition alone. |
| **Plan** | This is a multi-stage programme, not one change. **Stage 1** — make the classifier a trait. Extract `trait Classifier { fn labels(&self) -> &LabelSet; fn input_spec(&self) -> InputSpec; fn infer(&self, samples: &[f32]) -> Result<Vec<f32>, InferenceError>; }` from the concrete `Model`, with `InputSpec` carrying sample rate, window length and normalisation so the pipeline stops assuming 48 kHz/3 s. **Stage 2** — a registry that loads N classifiers from config and a per-source routing table. **Stage 3** — a merge policy in the detection pipeline: union with per-model thresholds, plus an *agreement* flag recorded on the detection row that the review UI and `corroboration.rs` can both use. **Stage 4** — Perch v2 as the second concrete implementation (32 kHz, 5 s windows, a CSV label file), which is the real test of whether Stages 1–3 are right. **Stage 5** — a model catalog and downloader with checksum verification, mirror support and atomic install, reusing the auto-update machinery in `birdnet-integrations/src/auto_update/`. **Stage 6** — the bat classifier, which additionally needs the ≥192 kHz capture path and the ultrasonic validation filter (G‑14). Stages 1–3 are worth doing even if no second model ever ships, because they remove the hardcoded assumption that there is exactly one. |

| **Resolution (Stage 1)** | `trait Classifier` (`crates/birdnet-core/src/inference/classifier.rs`) with `labels` / `input_spec` / `infer`, implemented for `BirdNetModel`. `InputSpec` carries sample rate, window length and **format** as three declared facts, derived from the ONNX shape's rank and middle dimension: `[1, N]` and `[1, 1, N]` are waveform windows, only `[1, M, F]` with `M > 1` can be a mel. One divergence from the sketch above: `infer` takes `&mut self`, because the ONNX session does, and interior mutability for a signature nobody needs is the worse trade. |
| **What Stage 1 found** | The assumption in the pipeline was not "48 kHz / 3 s" — the model already derived both from its input shape. It was that input **format** came from sample rate: `expects_raw_audio()` was `infer_sample_rate() == 32_000`. BirdNET V2.4 declares `[1, 144_000]`, exactly 48 kHz × 3 s and therefore a sample count, so it is a waveform model — but being 48 kHz it took the mel branch, and the pipeline handed it a 128 × 282 = 36 096-value spectrogram that `build_input_tensor` zero-filled to 144 000. **Three-quarters of every V2.4 inference was silence**, with no error and no warning. The builder now refuses a slice under half the expected width; padding is for a recording that ran out of audio, not for a category error. |
| **Why nothing caught it** | Two reasons, both worth keeping in mind elsewhere. CI fetches a **V3.0** model (`ci.yml`, tag `models-v3.0-preview3`), which is 32 kHz and therefore takes the waveform branch — so the mel branch, the code default and what any 48 kHz model selects, had no real-model coverage at all. And the one unit test of the routing, `loaded_v24_model_does_not_expect_raw_audio`, asserted the implementation restated: *"expects_raw_audio is `infer_sample_rate() == 32_000`. V2.4 is 48 kHz, so this must be false."* It agreed with the rule by construction and would have passed for any rule of that shape. It has been replaced by one that names the property (a declared length equal to `sample_rate × 3` is a sample count) and that fails against the old rule. |
| **What remained unverified — closed 2026-10-15** | When this row was written no real BirdNET V2.4 ONNX had been run through the pipeline, so *that a real V2.4 declares `[1, 144_000]`* was inference. birdnet-go publishes one (`tphakala/BirdNET-v2.4`, `BirdNET_v2.4_fp32_dfttrunc.onnx`, input `[batch, 144000]`, output `[batch, 6522]` logits). It is now in the model catalogue as `birdnet-v24`, was run through the pipeline (48 kHz raw waveform, 3 s window, as the fix assumed) and compared with BirdNET-Pi's own code on 14 clips (`docs/book/guides/parity.md`, `tools/parity`), and CI fetches it, checks its sha256 and runs `tests/v24_matches_birdnet_pi.rs`. This is the *species classifier*; the embedding extractor Stage 6 needs is a different file (`tphakala/BattyBirdNET-onnx`) and has not been run here. |
| **Verified** | Against the real 11 K-species model (sha256 `2a0f9efb…b7d743`, the artifact CI pins): it reports a dynamic `[1, 1]` shape resolving to 32 kHz / 144 000 samples / waveform / 4.5 s, and `expects_raw_audio` is `true` both before and after, so a station on the shipped model is unaffected. The four model-gated e2e suites pass under `BIRDNET_REQUIRE_MODEL=1` (34 tests), which makes a silent skip a hard failure. |

| **Resolution (Stage 2)** | `ClassifierRegistry` (`crates/birdnet-core/src/inference/registry.rs`) loads up to three classifiers and resolves a per-source routing table; `src/helpers/models.rs` decides which the machine can afford. Three invariants, each gated, all chosen for a station nobody is watching: a registry with **no** classifier is refused (a station that detects nothing is not a degraded station); an unknown route target **stops startup** rather than leaving one microphone silently unjudged for months; and a source with no route gets the **primary**, never nothing — silence must not be reachable by omission. Memory policy lives in the application layer beside `G-33`'s: all classifiers together may use at most half the effective ceiling, counting file size plus a 256 MiB working-set allowance, and a machine that will not report its memory gets one classifier because "unknown" must not read as "plenty". The fraction is a judgement, not a Pi measurement, and the module header says so. |
| **Resolution (Stage 3)** | `crates/birdnet-core/src/detection/merge.rs` and migration 50 (`model_id`, `model_agreement` — nullable, `ALTER TABLE`, no rewrite of a multi-year `detections`). Union with per-model thresholds; agreement counted per species across *distinct* classifiers; the reported confidence is the winning classifier's own output. **Agreement is never folded into the confidence** — an averaged or bonused number would have no calibration behind it while occupying the same column as a real one, silently changing the meaning of every threshold and every historical comparison. `NULL` agreement is a third state (the row predates the migration) and is not `1` (one classifier asked, one answered). The human score that drives the privacy gate is the maximum across classifiers, because publishing a conversation is not recoverable. |
| **Plan (what remains)** | Stage 6 — the bat classifier, which additionally needs the ≥192 kHz capture path and `G-14`, and which the rows below show to be a chained head rather than a peer classifier. Stages 4 and 5 are done. No second model is *shipped* with the station, so the merge is exercised by construction, by gates, and against the real Perch v2 and BirdNET+ V3.0 files in a development environment — not by two classifiers shipped together disagreeing on a station's own audio. That is the honest limit of what Stage 3 is verified to do. |

| **Resolution (Stage 4, partial)** | Run against the real Perch v2 ONNX (409 148 616 bytes, sha256 `bf0c8467…cefa1f`, from `justinchuby/Perch-onnx`), Stage 4 did what it was for and found two defects in Stages 1–3. **(a) The class head was selected by position.** Perch declares four outputs — `embedding`, `spatial_embedding`, `spectrogram`, `label` — and `usize::from(outputs.len() > 1)` took index 1, a 98 304-wide internal representation, in place of 14 795 species scores. `class_output_index` now finds it by trailing dimension against the label count, with a name fallback so a mispaired label file still reaches the doctor's width check rather than being pre-empted. Verified by loading the real model: `output_dimension == Some(14795)`. **(b) The sample rate was derived, not declared.** `[-1, 160_000]` is 5 s at 32 kHz and equally 3⅓ s at 48 kHz; the lookup defaulted to 48 kHz. `MODEL_SAMPLE_RATE` / `MODEL_n_SAMPLE_RATE` declare it. |
| **The question Stage 4 left open** | The pipeline decodes, resamples and chunks a recording **once**, from the primary's spec. Perch wants 5 s windows where BirdNET+ V3.0 wants 4.5 s. Feeding Perch the primary's chunks would hand it a window it was never trained on, silently — so the registry refused any classifier whose spec differed from the primary's, at startup, naming both. Lifting that was never an implementation gap: Stage 3's agreement count means "two classifiers reported this species in this chunk", and that sentence has no meaning when the two judged different spans of time. The options recorded were to align on the coarsest window, or to merge by overlapping interval and define agreement over that. |
| **Resolution (Stage 4, the window arithmetic)** | **Neither option as stated.** The coarsest window alone is wrong: cut chunks at Perch's 160 000 samples and step 160 000, and the 144 000-sample classifier hears the first 4.5 s of every chunk and never the last 0.5 s — a blind spot it would not have had running alone, recurring on every chunk for the life of the station, and reported by nothing. The chunk is cut to the **longest** window and stepped by the **shortest**, so no classifier sees less than it would alone: the shortest-window model gets chunk starts identical to running by itself, and longer-window models get overlap rather than gaps. Overlapping-interval merging is not needed, because every classifier reads its own window from the same chunk start — which is what lets agreement be stated exactly: *two classifiers reported this species from audio beginning at the same instant*. `PipelineConfig::chunk_step_secs` carries the step (`None` = `chunk_duration_secs - chunk_overlap_secs`, the one-classifier rule unchanged); the registry now refuses a differing **sample rate** only, because a recording is resampled once and no rate is right for both. **The price**: a detection is stamped with the *chunk's* span, so a shorter-window classifier's detection can carry an end time up to `max(window) - min(window)` later than the audio it read — half a second for BirdNET beside Perch. Per-model spans would give the same merged species two different end times depending on which classifier won the confidence; the chunk span always contains the audio heard, and with one classifier the two are identical. |
| **What it costs, measured rather than estimated** | A classifier whose window is longer than the shortest runs `max(window) / min(window)` more inferences — 160 000 / 144 000 ≈ 1.11, about 11 % for Perch beside BirdNET. `run_daemon` logs it at startup as `extra_inference_ratio` alongside both window lengths, because on a Pi 4 an 11 % increase in inference load is a real number and an operator should not have to derive it. The shortest-window classifier's cost is unchanged. |
| **A comment that was wrong before it was committed** | `run_daemon` took the chunk length from `model.recommended_chunk_secs()` and now takes it from the registry's longest window; the new comment claimed the two were "exactly" the same number. They are on every waveform shape — and they are not on a mel shape, where `recommended_chunk_samples_from_shape` reads a count of spectrogram columns as a sample count and would ask for a six-millisecond chunk. `input_spec` refuses that reading, so where they differ this is a fix, not an equivalence. Established by running both functions over ten shapes rather than by reading them. Both halves are gated, the second as the counterpart that stops the first passing against an `input_spec_from_shape` that had simply delegated every shape. |
| **A mutation that proved less than its name** | The first attempt to verify the new selector disabled only the width test, leaving the name fallback — and Perch's head is literally called `label`, so the gate still passed. The label claimed the old rule had been restored; it had not. Re-run with both lookups removed, leaving exactly `usize::from(outputs.len() > 1)`, the gate fails as it should. Recorded because it is the second time this session a mutation's description outran what it changed. |

| **Resolution (Stage 5)** | `crates/birdnet-integrations/src/model_catalog.rs`, `--install-model <id>`, and `GET /api/v2/models/catalog`. Digests are measured from the files, not copied from model cards. **Compiled in rather than fetched**: a remote catalogue is a document that decides both the URL a station pulls 400 MB from and the digest it checks them against, so pinning both in the binary makes the checksum a promise this repository makes. **Streams rather than buffers** — `auto_update` reads an asset into memory before verifying, which is right for a 20 MB binary and an OOM kill for a 541 MB model on a 1 GB board; the digest is computed as bytes arrive, and unverified bytes carry a `.part` name the station cannot load. **Free space is checked before the network**, with a 512 MiB margin, and an unreadable filesystem counts as no space rather than plenty. **No install endpoint**, deliberately: a multi-hour download in a request handler has nowhere to report progress and a retry starts a second download beside the first. |
| **Found by running it rather than testing it** | The command **panicked on first invocation**: `reqwest::blocking` cannot build a client inside a tokio runtime and `main` is `#[tokio::main]`. Thirteen unit tests were green because they are plain sync functions that never enter a runtime — and `birdnet-integrations`' own module header says callers must wrap in `spawn_blocking`. Verified afterwards by a real install against the live Hugging Face URL: 409 148 616 bytes, digest matching the catalogue, no `.part` left behind. |
| **Two gates that were green for the wrong reason** | The free-space refusal called `install` against `/proc` and accepted an `Io` error as success, so it passed with the space check deleted — `create_dir_all` failed first. The "unknown is not plenty" gate only exercised `free_space_bytes`, never the `unwrap_or(0)` in `install`. Both are now written against a pure `fits(model_bytes, available)`, which is the same separation of decision from measurement that `plan_with` needed in Stage 2. The lesson, third time this session: **a gate that accepts more than one failure mode proves only that something went wrong.** |

| **Stage 6 — what it actually is, which is not what this row said** | The plan above describes the bat classifier as needing "the ≥192 kHz capture path and the ultrasonic validation filter". Checked against the model ([`tphakala/BattyBirdNET-onnx`](https://huggingface.co/tphakala/BattyBirdNET-onnx)), that substantially understates it. **`BattyBirdNET` is not a peer classifier.** Its input is `[batch, 1024]` — BirdNET **v2.4** embeddings — and it maps them to bat species. 256 kHz recordings are fed to BirdNET *without resampling*, deliberately: 144 000 samples is 0.5625 s at 256 kHz, which BirdNET treats as the 3 s at 48 kHz it was trained on, aliasing ultrasound down into the audible band. It is a **chained second stage**, not an independent opinion. |
| **Three consequences the plan did not capture** | **(a) Stage 3's agreement count must not apply to it.** "Two classifiers agreed" is false when one reads the other's intermediate output; recording agreement there would manufacture corroboration from a single model. **(b) The pipeline needs a deliberate no-resample path**, which is the opposite of what it does. **(c) It requires BirdNET v2.4 specifically** — V3.0's embeddings are 1280-wide, Perch's 1536, `BattyBirdNET` wants 1024 — and its embedding extractor is not the species classifier the catalogue now carries as `birdnet-v24`. |
| **Groundwork shipped toward Stage 6** | The piece all of it needs and none of Stages 1–5 provided: `embedding_output_index`, `BirdNetModel::embedding_width` and `BirdNetModel::embed`. Found **by exact name only** — Perch v2 exposes both `embedding` (a 1536 pooled vector) and `spatial_embedding` (a 16 × 4 × 1536 feature map), and a `contains` match would hand a chained head 98 304 numbers where it expected 1 536, which is the Stage 4 output-head defect one layer down. A classifier exposing no embedding reports `None` rather than offering its class head. Verified against the real BirdNET+ V3.0 (`embeddings [-1, 1280]`) and both committed fixtures. |
| **What still blocks Stage 6** | The no-resample path; BirdNET v2.4's embedding extractor (the species classifier is now installable, the extractor is not); `audio_sources`'s `CHECK (sample_rate IN (…, 48000))`, which has no 192 or 256 kHz; `G-14`'s ultrasonic filter; and the registry's remaining refusal. Stage 4's window arithmetic removed the *window* half of that refusal but not the rate half, and a 256 kHz chained head is a differing **rate** — correctly refused, since a chained head is not fed the pipeline's resampled audio at all. What it needs is the no-resample path, not a narrower check. None of these is guesswork away: each is a decision or an artifact this repository does not yet have. |

#### G‑11 · Inference backends beyond ONNX Runtime CPU — GAP

| | |
|---|---|
| **Upstream** | `conf.BirdNETConfig.Backend` (`auto`/`onnx`/`openvino`), `OpenVINODevice` (`auto`/`cpu`/`gpu`), `UseXNNPACK`, plus TFLite. `internal/classifier/model_openvino.go`, `openvino_gating_openvino_test.go`. |
| **Ours** | `ort` with the default CPU execution provider. Grep for `openvino`/`xnnpack`/`ExecutionProvider` across the workspace: no hits. |
| **Why it matters** | On the x86 half of our target list, an Intel iGPU through OpenVINO is several times faster than CPU, which is the difference between 2.0 s of overlap being affordable and not — and overlap is what makes our own `corroboration.rs` filter effective (see its own table: `lenient` and `moderate` are no-ops at zero overlap). On Raspberry Pi, XNNPACK is the same argument in miniature. |
| **Plan** | Expose an `inference_backend` setting resolving to `ort` execution providers, with the CPU provider always present as the fallback and a startup probe that logs which provider actually bound (`ort` will silently fall back, which is precisely the kind of confident-but-wrong state this repo's conventions exist to prevent). A `--channel-report`-style `--inference-report` should measure it rather than assert it. |

#### G‑12 · Dynamic per-species confidence threshold — SHIPPED

| | |
|---|---|
| **Upstream** | `internal/analysis/processor/dynamic_threshold.go` — once a species is confirmed present at a site by a high-confidence detection, its threshold drops in steps (×0.75, ×0.50, ×0.25) for `validHours`, floored at `min`, then decays back. Persisted per species (`internal/datastore/dynamic_threshold.go`) and tunable at `/api/v2/dynamic-thresholds/test`. |
| **Ours** | The global `confidence_threshold` and the operator-typed per-species overrides (`species_thresholds` table) are still there, and something learned now sits beside them: `crates/birdnet-core/src/detection/dynamic_threshold/` — *"Let a species that is **known present** be easier to hear."* Off unless asked for (`src/daemon/config.rs:278` `resolve_dynamic_threshold`), and what it learns survives a restart through the `dynamic_thresholds` table (migration 38, `crates/birdnet-db/src/dynamic_thresholds.rs`). |
| **Why it matters** | A fixed threshold is a bad instrument because it is answering two questions at once — "is this a bird" and "is this bird plausible here". Once a Tawny Owl is *known* to be in the wood, a 0.4 Tawny Owl is very likely another Tawny Owl; a 0.4 for a species never recorded within 500 km is not. Learning the first without loosening the second is what this buys, and it is the single highest-yield detection-quality change on this list after multi-model. |
| **Resolution** | Shipped as a directory rather than a file: `detection/dynamic_threshold/{mod,tests}.rs`, with `DynamicThresholds::effective_threshold(sci_name, base, now_ms)` (`mod.rs:261`) checking expiry on every read, and `LearnedThreshold` rows round-tripped by `birdnet-db`'s `replace_all` / `load_all`. It is applied in the binary's daemon — `src/daemon/disposition.rs:145` takes an `Option<&DynamicThresholds>`, held as `DynamicThresholdState` in `src/daemon/processor.rs:35` — not in `detection/daemon/process.rs`, which is not a path in this tree. **Checked in the 2026-09-07 pass, and it goes against us:** the `/admin/species` threshold panel and the `/admin/species/test` preview (`routes/admin/species/mod.rs:29`) read only the operator-typed `species_thresholds` table (`routes/admin/species/handler.rs:175` `get_species_thresholds`); nothing under `routes/admin/species/` references `DynamicThresholds` or `effective_threshold`, so the preview shows the *configured* threshold, not the effective one. That was the plan's stated trap, it was not avoided, and it is the remainder of this item. |

#### G‑13 · Silero VAD privacy gate — GAP

| | |
|---|---|
| **Upstream** | `conf.VADSettings` — an embedded Silero VAD ONNX model detecting speech *presence* (not content, not speaker), opt-in, augmenting the label-based privacy filter. **Refreshed from the audit of 2026‑10‑20** (upstream not re-read here): birdnet-go raised its VAD threshold from 0.35 to 0.6 (`61009d9`), now needs a second hit at high overlap (`e81763b`), and defers detections until the whole clip has been scanned (`fc34a97`). |
| **Ours** | `crates/birdnet-core/src/detection/privacy.rs` has two rules, and either one suppresses a chunk and the chunks either side of it. Both read the model's whole output, before the detection threshold and the top-N cut. The **rank rule** is BirdNET-Pi's: a human class among the model's `N` highest-scoring classes (`ChunkPrediction::human_rank`; `PRIVACY_RANK`, default 10, `0` = off). It has been on by default since `6888399f`, so a station that sets neither key now masks a human in its top ten. The **score rule** is this project's: a human class whose confidence reaches `PRIVACY_THRESHOLD`, off at its default of `0`. (An earlier version of this row said the filter was rank-based and never excluded anything at the shipped `top_n`: that described a rule which read only the detection list, which `privacy.rs` replaced by reading the model's whole output. The rank rule came back afterwards, read the same way.) Neither rule is a speech detector; both use BirdNET's own human labels. |
| **Why it matters** | A garden microphone records the neighbours. BirdNET's human classes are a by-product of its training set, not a speech detector, and they miss quiet conversation at exactly the distance where it is still intelligible. For anyone deploying where consent matters, a real VAD is the difference between a defensible privacy claim and a hopeful one. |
| **Plan** | Optional second small ONNX session (~2 MB) run only on chunks that pass the cheap gates, gating clip *retention* rather than detection: a chunk with speech is analysed and its detection recorded, but no audio is written. Off by default, and the model shipped alongside rather than embedded so the binary size story does not change for stations that do not want it. |

#### G‑14 · Ultrasonic validation filter — GAP (blocked on G‑10 Stage 6, which is larger than recorded)

| | |
|---|---|
| **Upstream** | `conf.UltrasonicFilterConfig` — measures the coefficient of variation of ultrasonic-band energy across STFT frames; real echolocation is bursty (high CV), audible-range false positives are flat at the noise floor. |
| **Ours** | No ultrasonic path at all. |
| **Plan** | Ships with bat support or not at all. Recorded here so the dependency is explicit. |
| **The dependency is bigger than it looked** | `G-10` Stage 6 turned out not to be "a bat classifier plus a capture path": `BattyBirdNET` is a chained head on BirdNET **v2.4** embeddings, fed 256 kHz audio without resampling so ultrasound aliases into the band BirdNET was trained on. See the Stage 6 rows under `G-10`. This filter still ships with that or not at all, but "that" is a larger piece of work than this row implied. |

#### G‑15 · Taxonomy: family, genus, tree, and synonym aliasing — SHIPPED (no family rank; the label file states none)

| | |
|---|---|
| **Upstream** | `internal/openfauna/aliases.go` — an authoritative legacy→canonical scientific-name map, because "acoustic models are trained on different taxonomies and time-frozen label sets, so they emit different scientific names for the same species" (e.g. `Streptopelia senegalensis` → `Spilopelia senegalensis`). Plus `internal/classifier/taxonomy.go`, `genus.go`, and `GET /api/v2/taxonomy/{family,genus,tree}/…`. |
| **Ours** | (a) `crates/birdnet-core/src/inference/vocabulary.rs` aligns the geomodel's vocabulary to the classifier's once at load — exact scientific name, then common name + specific epithet — and `--doctor` reports both residues. `SPECIES_ALIASES_PATH` takes an operator's own map. (b) `labels.rs` parses the `order` column beside `class` and derives the genus from the binomial; `/species?rank=order&taxon=…` narrows the List and Photos grids, with order chips built from the station's own species and a `class · order · genus` line on each detail page. |
| **Why it matters** | Two separate problems. (a) **Silent exclusion, and then double-counting** — raised from "double-counting" by the 2026-09-04 audit (UNATTENDED_DEPLOYMENT_AUDIT.md §2, S-6), which is correct: our own species filter matches the geomodel's vocabulary to the classifier's by scientific name with no alias step (`species_filter.rs`'s whole doc comment is about this hazard, and `load_with_vocabulary`'s scoring loop `continue`s past any geomodel name that `find_by_scientific_name` cannot find in the classifier's labels). A species whose name differs between the two files is therefore *permanently undetectable* at that station — it never enters the passing set, and nothing reports that it did not — which is worse than the double-counting a reclassified genus also causes, where the same bird becomes two species in the life list, the year list, and every retention query. (b) **Browsing**: "show me every warbler" is a natural question that a flat species list cannot answer. |
| **Measured** | Both label files downloaded and verified against the SHAs `installer/lib/10-config.sh` pins (geomodel `c15818db…`, 12 012 rows; classifier `8124b0ea…`, 11 560 rows). **1 679 geomodel rows have no exact scientific-name counterpart.** 62 are the same taxon under a reclassified genus, 41 of them birds — Hairy Woodpecker, Red-cockaded Woodpecker, White-headed Woodpecker, Evening Grosbeak and 16 more woodpeckers (19 of the 41 are woodpeckers) — every one undetectable while the range filter ran. In the other direction, **1 165 of the classifier's 11 560 species are reached by no geomodel row** (1 227 lack an exact scientific-name counterpart; 62 are recovered) and so cannot pass while it runs; that is correct behaviour but was equally unreported. |
| **Plan superseded** | The original plan — vendor a curated alias table, normalise on write in `detections`, migrate existing rows — was wrong on all three counts, and the measurements are why. **The table**: OpenFauna's `aliases.json` is CC BY-SA 4.0 against this project's CC BY-NC-SA 4.0, which ShareAlike forbids; and all 237 of its entries recover **0** of the 1 679, because its reclassifications are ones both of our files already agree on. **Normalise on write**: the disagreement is between two *label files*, not between a detection and its own model — a detection already carries the classifier's spelling, so there is nothing in `detections` to collapse and rewriting it would break every join back to the label file. **The staleness gate**: it was to fire when "the shipped classifier's label set" contained a mapped-from name, but no label set ships — the installer downloads both at install time. What shipped instead: alignment from the data the station already has, with the residue reported by `--doctor`, and an operator-supplied alias file for the rest. |
| **Not shipped, and why** | **No family rank.** The pinned classifier CSV's header is `idx;id;sci_name;com_name;class;order` — there is no family column, and inventing one from a genus would be a guess beside two stated facts. So "show me every warbler" is answered by order (*Passeriformes* is too broad) or by genus (*Setophaga* is too narrow) or by the existing name search, not by a family chip. **No genus chip row**: 2 907 genera is not a control; the genus is reachable from each species' detail page, where the question is about one bird. **No `/api/v2/taxonomy/{family,genus,tree}` endpoints** to match upstream's: the tree upstream serves has a rank ours does not have, and a two-level tree is the chip row already on the page. |

#### G‑16 · Species tracking: yearly, seasonal, and returning-after-absence — SHIPPED (API only; not yet on the pages)

| | |
|---|---|
| **Upstream** | `conf.SpeciesTrackingSettings` — "new species" window, **yearly** tracking with a configurable reset date, **seasonal** tracking with hemisphere-aware season boundaries (`GetDefaultSeasons` handles northern, southern *and* equatorial wet/dry), and **infrequent** tracking that flags a species returning after `absenceDays`. Notification suppression is tracked separately per category. |
| **Ours** | `rare_species_days` drives `/feeds/rare.rss` and `/feeds/rare.ics`, whose SQL (`routes/feeds.rs:47`) implements two definitions of rare — first-ever, and returning after a gap — and a life list exists (`routes/pages/life_list.rs`). The year list, the season list and the hemisphere awareness now exist too: `crates/birdnet-db/src/species_tracking.rs` computes `new_ever`, `new_this_year`, `new_this_season`, `returning_after_absence` and `days_since_previous` per species, served at `GET /api/v2/species/tracking` (`routes/species.rs:19`). The season boundary is hemisphere- and latitude-dependent and lives in `crates/birdnet-core/src/season.rs`, which carries northern, southern and equatorial wet/dry tables. |
| **Why it matters** | "First of the year" is the unit birders actually keep score in, and a station is uniquely good at catching it — it is listening at 04:40 when nobody is awake. Seasonal firsts are the phenology signal this project's DuckDB analytics already exist to measure, so not surfacing them on the dashboard is leaving the best story untold. Hemisphere matters because half the potential users are south of the equator and a northern-defaults season table is wrong by six months for all of them. |
| **Resolution** | The module and the season table landed as planned, and the API reports the windows alongside the species rather than as a courtesy — a bare "first this season" is unreadable without knowing which season the station thinks it is in, and a station with no latitude honestly returns `season: null`. **What did not land is the wiring.** `crate::tracking` has exactly one consumer, `routes/species.rs:52`; the flags do not reach the today page, the RSS/iCal feeds or the notification trigger vocabulary, so the capability exists but nothing a non-API user looks at shows it. That is the remainder of this item. |

#### G‑17 · Dog-bark suppression window — SHIPPED

| | |
|---|---|
| **Upstream** | `conf.DogBarkFilterSettings` — a species list plus `remember`, suppressing those species for N **minutes** *after* a bark (an earlier draft of this row said seconds; corrected per UNATTENDED_DEPLOYMENT_AUDIT.md §2). |
| **Ours** | `crates/birdnet-core/src/detection/noise.rs` drops the whole chunk on a noise class at or above threshold, and its doc comment argues explicitly against spreading to neighbouring chunks (a bark is a few hundred milliseconds and the chunks overlap). |
| **Verdict** | Our design is better reasoned for the *chunk* case. The upstream `remember` window addresses something different: a dog that barks for a minute produces phantom detections in the gaps *between* barks, where no bark is present to trigger the chunk filter. |
| **Plan** | Keep the chunk filter as-is; add an optional `noise_remember_secs` that suppresses the specific species that co-occur with the noise class, not all species, for a bounded window after it. Off by default. |
| **Resolution** | Shipped as planned. `NOISE_REMEMBER_SECS` (a `birdnet.conf` key; no CLI flag) sets the window; the chunk filter is untouched. When a watched class suppresses a chunk, the species that were in it are remembered and dropped from later chunks inside the window — **those species alone**, which is the whole point: a blanket window is a mute button, and a dog barking through the dawn chorus would erase the chorus. A gate asserts a blackbird singing in the same gap survives. Off by default because the window does remove real detections whenever a real bird is the species a dog resembles; setting it without `NOISE_THRESHOLD` logs a warning rather than doing nothing quietly. |
| **Stateless, and bounded by the recording** | Computed from the chunk start times `ChunkFilters::apply` already carries, so `filter_predictions` stays `&self` and the window reaches only to the end of the recording being analysed. Carrying it across segments would need state on a filter **every audio source shares**, so a bark on the garden microphone could silence a species on the pond one — a worse error than the one being fixed, and invisible. Within-recording covers the gaps inside each segment, which at the default fifteen-second segment and three-second chunks is most of them. The boundary case is stated in the module header rather than papered over. |
| **The four properties the gates pin** | Forward only (a bark must not retroactively erase a bird recorded before the dog was let out); bounded (a dog that barked at teatime must not silence a wren all evening, boundary inclusive); the noise class itself is not remembered as a species; and a non-finite or non-positive setting leaves the window **off** rather than making it unbounded — an operator's typo must not silence a species for the rest of the recording. |

### 2.3 Web, security and deployment

#### G‑18 · Trusted-proxy client-IP resolution — SHIPPED (correctness/security)

| | |
|---|---|
| **Upstream** | `conf.Security.TrustedProxies` — a CIDR/IP list whose forwarded headers (`CF-Connecting-IP`, `X-Forwarded-For`, `X-Real-IP`) may be believed, with loopback/link-local/RFC1918 peers always trusted, a reserved `"cloudflare"` value expanding to the published edge ranges, and — the point — headers **ignored** when the immediate peer is not trusted, so a directly exposed instance cannot be IP-spoofed. |
| **Ours** | The boolean is gone. `crates/birdnet-web/src/rate_limit.rs:233` now reads `pub(crate) fn extract_ip(req: &Request<Body>, trusted: &TrustedProxies) -> IpAddr`, and `RateLimitConfig` carries `trusted_proxies: TrustedProxies` (`rate_limit.rs:65`) rather than a flag. The allow-list itself is `crates/birdnet-web/src/client_ip.rs`, populated at startup by `server.rs:94` `trusted_proxies_from_env`. |
| **Why it matters** | Both settings of that boolean are wrong behind a proxy. `false` — today's only reachable state — means every request through a reverse proxy shares the proxy's IP, so one abusive client exhausts the bucket for the whole household, and `sessions.ip_hash` records the proxy for every login. `true`, had it been wired, would mean any client can set `X-Forwarded-For` and get a fresh bucket, which is worse. The correct behaviour needs the peer address, which neither state consults. |
| **Resolution** | `client_ip.rs` walks `X-Forwarded-For` right-to-left and stops at the first untrusted hop (`:408`), with loopback and the RFC1918 ranges trusted by default and a reserved `cloudflare` name expanding to a snapshot of the published edge ranges (`:234`, `:318`). The discrimination the plan demanded is gated **both** ways rather than only the permissive one: `an_untrusted_peer_cannot_forge_its_own_address` (`:498`), `the_walk_stops_at_the_first_untrusted_hop_from_the_right` (`:537`) and `cf_connecting_ip_from_an_untrusted_peer_is_ignored` (`:605`) are the ignoring half; `the_cloudflare_name_lets_the_walk_pass_the_edge` (`:617`) is the honouring half. **Confirmed on 2026-09-07:** the resolved address reaches session binding as well as the rate limiter — `routes/auth_pages.rs:221` hashes the `ClientIp` extractor's value through `session::hash_client_ip` (`session.rs:402`) into `sessions.ip_hash`. The audit log does not record a client IP at all (`crates/birdnet-db/src/accounts/audit.rs` inserts `user_id, action, target, metadata`), which is a design choice rather than a wiring gap. |

#### G‑19 · Reverse-proxy base path — SHIPPED

| | |
|---|---|
| **Upstream** | `WebServerSettings.BasePath` (e.g. `/birdnet`), with `internal/api/basepath.go`, `basepath_test.go`, `basepath_race_test.go` and an ingress test — enough machinery to show it is not a one-line prefix. |
| **Ours** | `crates/birdnet-web/src/base_path/` — *"Serving the station from under a prefix, e.g. `https://home.example/birdnet`"*, whose own header explains why this is not a one-line `nest`: mounting the router under a prefix fixes *incoming* requests and nothing else. Read from `BIRDNET_BASE_PATH` (`base_path/mod.rs:204`, falling back to the root when it does not parse) and installed by `server.rs:130`. |
| **Why it matters** | The common home deployment is one hostname and a reverse proxy with several services under paths. Without base-path support such a user must give the station its own subdomain or its own port, and mixed absolute/relative links break in ways that look like caching bugs. Home Assistant ingress (which upstream tests for) works this way too. |
| **Resolution** | The outgoing half was solved by rewriting rather than by a `url_for()` helper: `security.rs:165` `inject_base_path` prefixes the absolute links in served HTML, so the templates keep their literals and cannot drift out of step with the setting. Gated end to end by `crates/birdnet-web/tests/base_path_end_to_end.rs` — the station answering under the prefix *and not beside it*, the trailing slash not being a dead end, the bare root redirecting into the prefix, **every** link a rendered page emits being prefixed, the prefix being published for the page's scripts, and static assets served from under it. |

#### G‑20 · OAuth2 / OIDC authentication — GAP

| | |
|---|---|
| **Upstream** | `conf.Security.OAuthProviders` — Google, GitHub, Microsoft and generic OIDC (issuer URL, scopes), plus basic auth, plus an allowed-subnet bypass, plus `PrivateMode` requiring auth before any UI data is shown. |
| **Ours** | A session cookie validated by `auth_middleware.rs` in front of `/admin/*` (HMAC session tokens in `session.rs`; `users` and `sessions` tables, O‑15), bearer tokens for `/api/v2` writes (`api_token.rs`), and an audit log that is now actually written — `crates/birdnet-web/src/audit.rs`, whose own header records that `AuditLog::record` previously had zero production callers; it is now referenced from 12 files. There is no `auth.rs` and no HTTP Basic path (an earlier draft of this row named both). No federated identity. Grep for `oauth`/`oidc` finds only Apprise URL parsing and an unrelated `rules.rs` match. |
| **Why it matters** | Less about the home station and more about the shared one — a reserve, a school, a research group where several people need access, one of them leaves, and there is exactly one shared password written on the wall. |
| **Plan** | Authorization-code + PKCE against a configured OIDC discovery document, mapping the `sub`/`email` claim onto the existing `users` table so sessions, roles and the audit log are unchanged. Named providers are then just pre-filled issuer URLs. Basic auth stays as the fallback for a headless LAN box. |

#### G‑21 · HLS live streaming — SHIPPED (upstream's API; fixed encoder settings)

| | |
|---|---|
| **Upstream** | `POST /api/v2/streams/hls/:sourceID/start`, `…/stop`, `/heartbeat`, `GET /status` and `GET /streams/hls/t/:token/playlist.m3u8`, with per-session tokens and `LiveStreamSettings` (bitrate 16–320 kbit/s, default 128; segment length 1–30 s, default 2). Since its native-HLS change it encodes AAC in process at a pinned 48 kHz and serves from memory (`internal/api/v2/audio/audio_hls.go`, `audio_hls_native.go` at 780ce51). |
| **Ours** | The same five routes and response fields (`routes/hls.rs`), beside the MP3 stream at `/stream`. One `ffmpeg` per source and pitch shift, shared by everyone listening and counted against `/stream`'s four slots; AAC at 128 kbit/s and 48 kHz, 2-second segments, a six-segment playlist, ready at two segments or reported `starting` after 20 s — upstream's defaults. A stream ends when its last session leaves, when nothing has fetched it for a minute, or when `ffmpeg` exits, and its segments are deleted; a directory left by a process that died mid-stream, which nothing writes again, is removed by the next start once it is five minutes old. The Live view plays HLS where the browser declares native support (Safari, iOS, Android Chrome) and falls back to MP3 elsewhere, or when a start is refused. |
| **Differences** | Bitrate and segment length are fixed, not settings. Segments are files under the service's temporary directory (a private `/tmp` under the installer's unit), not memory. Start and stop are open wherever `/stream` is, rather than behind sign-in: a stop ends only the caller's own session, and on a private station the routes follow the `live_audio` carve-out. A pitch shift is a stream of its own (upstream has none). |
| **Verified** | `tests/hls_live_audio.rs` drives the real router and a real `ffmpeg` from a capture tap: the playlist is live (no end marker), its segments are whole MPEG‑TS packets, a second listener joins the same encoder, one leaving keeps it, the last stopping ends it and deletes its directory; tokens, traversal and malformed heartbeats are refused. The watchdog is tested against a real child process for idleness, exit and the counterpart (a fetched stream is kept). `interactions.mjs` checks the player takes the HLS path when the browser claims it, leaves on stop, and falls back to `/stream`. Not verified: playback in Safari or on a phone — no such browser was available — and behaviour across a real network handover. |

#### G‑22 · Dashboard layout customisation — SHIPPED (order and visibility; not widths, banner or colour schemes)

| | |
|---|---|
| **Upstream** | `conf.DashboardLayout` — an ordered list of elements (`banner`, `daily-summary`, `new-species-highlights`, `currently-hearing`, `detections-grid`, `live-spectrogram`, `video-embed`), each enabled/half/full width, with a banner carrying a location map, live weather and a custom image, plus five colour schemes (an earlier draft said six; Part 5 corrected it) and a custom primary/accent pair. |
| **Ours** | A fixed dashboard (`routes/pages/dashboard/`, `homes/`), light/dark via `theme-guard.js`. (This cell used to add "and a custom image (`custom_image_dir`)". That is a per-species picture override, not a dashboard image: see N‑6.) |
| **Why it matters** | Half the stations that exist are on a wall-mounted tablet in a visitor centre or a kitchen, where what needs to be on screen is not what a person debugging a microphone needs. |
| **Plan** | Store an element list in `settings` and render from it. Our HTMX partials are *already* the element vocabulary — `/pages/hero-status`, `/pages/today-list`, `/pages/most-recent`, `/pages/hourly-chart` and the rest are exactly these components — so this is a layout table and a drag-to-reorder editor over machinery that exists, not a rewrite. |

| **Resolution** | `routes/pages/today_layout.rs` and the editor in `routes/admin/today_layout.rs`, stored as one `today_layout` setting (`daystrip,hero,-nudge,…`). The Today template marks each block and the page is reassembled in that order — the DOM, not CSS `order`, so reading and tab order follow the screen. Blocks move within their area (top stack, side rail; the log can only be hidden); the survivor of log and rail takes the width. Move up / Move down / Show-Hide buttons rather than upstream's drag, which a keyboard or switch user cannot operate. A station with no detections renders the default, so the setup checklist cannot be hidden. **Not done**, and recorded rather than implied: half/full widths per element, the banner with map and weather, a video embed, and the colour schemes. |
#### G‑23 · Detection comments — SHIPPED

| | |
|---|---|
| **Upstream** | `POST /api/v2/detections/:id/comments`, `batch/{delete,lock,resolve,review}`, and an ignored-species list. |
| **Shipped — batch operations** | `POST /api/v2/detections/batch` landed in PR #234 (`routes/api_write.rs`: route table `:60`, registration `:96`, handler `async fn batch` at `:434`). It applies one of `review` / `lock` / `unlock` / `delete` to up to `BATCH_MAX = 500` detections per request — a bound the compiler checks, not a test (`api_write.rs:339`, with a `const` assertion at `:347`). One bad key does not sink the batch: each item gets its own result and the `failed` count is named at the top level. Every detection it changes gets an audit row. It is bearer-only, mounted behind `api_token::require_bearer` (`server.rs:152`), so it does not exist on a station with no `BNB_API_TOKEN`. Gated by four tests in `crates/birdnet-web/tests/the_api_can_change_the_station.rs:783,840,890,967` and by the SQLite↔DuckDB desync guard in `tests/analytics_divergence.rs:749,795`. Upstream's batch `resolve` is the only member of their set with no analogue here. |
| **Shipped — free-text comments** | Migration 49's `detection_comments`: many rows per detection, `user_id` (`ON DELETE SET NULL`) beside a denormalised `author` so attribution outlives the account, and a `BEFORE UPDATE OF …` trigger that aborts any rewrite of the id, detection, author, body or timestamp. Deleting stays possible and is audited by id and author, never by body. A thread on the detection-detail page (`pages/detection_comments.rs`, read public, write behind the admin gate) and three API endpoints under `/api/v2/detections/comments`, where comments are attributed to `api`. **Written first with a whole-row trigger, which made users undeletable** — `ON DELETE SET NULL` is an UPDATE of the child row — caught by `removing_the_account_keeps_the_comment_and_its_name`. |
| **Superseded — what was missing** | **Free-text comments.** There was no `detection_comments` table: 42 migrations exist and none creates one. There is no route in `crates/birdnet-web/src/routes/` whose path contains `comment`. And `detection_reviews.notes` is not a thread wearing another name — migration 13 puts it under `UNIQUE(date, time, sci_name)` and writes it with `INSERT … ON CONFLICT`, so a second review **overwrites** the first, and the table carries no user column and no foreign key to `users`. The batch endpoint writes that field (*"`review` only: free text attached to every verdict in the batch"*), which is exactly what makes it look like a comment and why the distinction is worth stating. Review (`detection_reviews`, `/detection-reviews`), lock/unlock and bulk review in the search page (`/pages/search-bulk`) are unchanged. |
| **Why it matters** | The verification loop is where a station's data becomes usable to anyone else. "Why did I mark this wrong" is the note that makes a review defensible six months later. An overwritten, unattributed `notes` field cannot carry that: the second reviewer erases the first one's reasoning without either of them knowing. |
| **Not shipped, and why** | **No comment count in the detection lists.** A per-day count query was written for it and then deleted unused: a public function with no caller is the shape this repository's own audit kept finding, and the lists are a separate change that can add the query back when it has a reader. **No `resolve`**, upstream's one batch operation with no analogue here: a comment thread has no resolved state to set, and inventing one would make the thread a ticket queue. |

#### G‑24 · Profiling endpoints — SHIPPED (CPU; no heap profile)

| | |
|---|---|
| **Upstream** | `conf.DiagnosticsConfig.Profiling` — pprof behind the auth middleware, with a generated token when no auth provider is configured, and block/mutex sampling rates whose costs are documented in unusual detail. |
| **Ours** | `--doctor`, `--channel-report`, `--support-bundle`, Prometheus metrics — and, since `9f42652`, the doctor report and the support bundle over HTTP too (`GET /admin/doctor.json`, `GET /admin/support-bundle`, `routes/admin/doctor.rs:41‑42`; read-only, `--fix` is never implied by a GET). No live profiler. |
| **Why it matters** | "The Pi is at 100 % CPU and I don't know why" is answerable in one command with a profiler and is a week of guessing without one. |
| **Shipped** | `GET /admin/debug/pprof/profile?seconds=&format=` (`routes/admin/profiling.rs`), compiled only with the `profiling` feature (`pprof` 0.15, no default features: its `flamegraph` feature needs `inferno`, CDDL-1.0, outside `deny.toml`). Gzipped `pprof` protobuf or folded stacks; administrators only, behind the admin cookie gate rather than a separate token, since every account is authenticated here; one profile at a time; nothing armed until asked. No heap profile — Rust has none independent of the allocator, and swapping the global allocator for a profiling jemalloc is a change to every build. Block and mutex profiles are Go-runtime concepts with no counterpart. |

#### G‑25 · Error-tracking telemetry — DECLINED, with a substitute

| | |
|---|---|
| **Upstream** | `internal/telemetry` — opt-in Sentry with a `SystemID`, plus `internal/observability`. |
| **Ours** | Prometheus metrics, structured tracing, and the support bundle with secret redaction gated by a test (`src/helpers/offsite.rs:591`). |
| **Verdict** | Shipping a crash reporter that phones a third party from a device with a microphone in someone's garden is a privacy posture this project should not adopt, even opt-in. |
| **Substitute** | Make the local path better: a persistent panic/error ring buffer written to disk, surfaced in the support bundle and at `/station/…`, so a user can *choose* to send it. That is the same diagnostic value without the default-on-the-network question. **Status, read on 2026‑10‑20: most of it exists.** `src/log_capture.rs` appends ERROR and WARN lines to `errors.jsonl` beside the database, which survives a reboot, and starts the file again when it passes `ERROR_LOG_MAX_BYTES` (1 000 000) rather than rotating it. `src/support.rs` stages it in the support bundle (`read_error_log`), after the same secret scrub as the journal. Not done: no `/station` page shows it (nothing outside `src/support.rs`, `src/log_capture.rs` and `src/main.rs` names the file), and whether a panic reaches it was not checked. |

### 2.4 Integrations

#### G‑26 · Weather providers — SHIPPED (Open-Meteo, MET Norway/yr.no, Wunderground, Pirate Weather; OpenWeather declined)

| | |
|---|---|
| **Upstream** | `internal/weather/` — `provider_yrno.go`, `provider_openweather.go`, `provider_wunderground.go`, a common interface, icon mapping, and a poll interval. Weather is joined onto detections and shown on the dashboard banner. |
| **Ours** | `crates/birdnet-integrations/src/weather.rs` — a `Provider` enum over Open-Meteo (default), MET Norway and Wunderground personal stations, chosen with `WEATHER_PROVIDER`. Still off unless `BNB_WEATHER_ENABLED=1`, still self-host-able via `BNB_WEATHER_BASE_URL`, still one `weather` table. `WEATHER_API_KEY` is mountable from a file. |
| **Why it matters** | Open-Meteo is the right default (no key, permissive terms, self-hostable) and we should keep it. But a station owner who already runs a **personal weather station** has ground-truth data ten metres from the microphone, and that is a far better covariate for bird activity than a gridded forecast — which is exactly what the Wunderground provider is for. |
| **Departures from the plan** | **An enum, not a trait.** The set is closed, `birdnet-integrations` carries no `async-trait` and builds no runtime, so a trait would be dyn-incompatible (`async fn` in traits) or a boxed-future dance for no gain; an exhaustive `match` also makes a fourth provider fail to compile until every site handles it. **OpenWeather is declined, not deferred**: it needs a key like Wunderground but returns a gridded forecast like the two keyless ones, so it is the only one of the four that asks an operator for a credential and gives nothing the default does not. |
| **Verification** | The MET Norway decoder is written against a live response captured from `api.met.no` on 2026-09-11 and committed as the fixture; its units are read from that response's own `properties.meta.units`. The **Wunderground decoder is not verified against real bytes** — that endpoint needs a station owner's API key this repository does not have — and its doc comment says so rather than implying otherwise. |

#### G‑27 · eBird API integration — SHIPPED

| | |
|---|---|
| **Upstream** | `internal/ebird/` — `client.go`, `observations.go`; `EBirdSettings` with API key, cache TTL and locale; `/api/v2/integrations/ebird/test`. |
| **Ours** | `crates/birdnet-integrations/src/ebird.rs` — a cached client for `data/obs/geo/recent` and `data/obs/{regionCode}/recent`, polled every 6 h, cached to disk as JSON and published into `AppState`. Feeds a *Reported nearby* card on the detection detail page and a corroboration mark in the suspect-species report. Off unless `EBIRD_API_KEY` is set. The pre-existing `info_site=EBIRD` links and `GET /detections/export/ebird` are unchanged; data now goes both ways. |
| **Why it matters** | eBird recent-observations for the station's region is the best available answer to "is this plausible right now" — better than the geomodel, which is a static climatological prior with no idea that the species arrived last Tuesday. It is the natural input to a "phantom species" check, which we already have a page for (`/admin/quality/phantoms`). |
| **Plan** | A cached client for `data/obs/{regionCode}/recent` keyed on the station's region, feeding (a) a "recently reported nearby" badge on the detection detail page and (b) a corroboration signal in the data-quality view. Cache to disk with the TTL so an offline station degrades to its last snapshot rather than failing. |
| **Departures from the plan** | **Coordinates, not a region, by default.** The plan said `{regionCode}`; the station already has `LATITUDE`/`LONGITUDE` for the range model and the solar schedule, and *"reported within 25 km"* is a far stronger statement than *"reported somewhere in this state"*. `data/obs/geo/recent` is the default and `EBIRD_REGION` overrides it — both endpoints return the same documented shape, so supporting the plan's wording cost one URL builder. **No enable flag and no TTL key.** eBird needs a key for every endpoint, so the key *is* the opt-in; a second switch would only add a state where a key is configured and silently unused. The refresh interval is a constant, and a snapshot's *age* is shown to the operator rather than made configurable — the property the plan wanted from "TTL" is that a failed fetch keeps the last snapshot, which is the refresh policy, not a knob. **No `sppLocale`.** Matching is on scientific names and the common names shown come from the station's own label file, so a localised eBird name would be a second vocabulary to reconcile for no gain. |
| **The decision the row did not anticipate** | **Corroboration only ever exculpates; it never accuses.** A hit is shown. A miss is shown as *nothing at all*, and changes no verdict anywhere. eBird coverage tracks birder density, not bird presence: a well-watched county yields hundreds of checklists a week and a quiet valley yields none, ever, for any species. Had absence been allowed to count towards a phantom verdict, the station with nobody nearby to confirm anything — the one that most needs an automated check — would have had its entire list flagged. `phantoms.rs` is therefore untouched: the join happens in the renderer, and `PhantomSignal` gains no fifth variant. A gate asserts an uncorroborated row renders byte-for-byte as it did before the feature existed. |
| **What is left, and how a wrong key surfaces today** | Upstream's `/api/v2/integrations/ebird/test` is **not** implemented. An operator who mistypes the key gets no button to press; what they get is a `tracing::error!` naming `EBIRD_API_KEY`, raised as its own `EbirdError::Unauthorized` variant precisely because a refused key is the one failure that never fixes itself and is otherwise indistinguishable in a log from a network blip worth retrying. That is diagnosable but it is not discoverable — a station owner who never reads the journal sees a feature that is silently doing nothing, which is the same shape of defect `G-28` catalogued. A test endpoint, or an integration-status line on the settings page, is the honest follow-up. |
| **Verification** | **The decoder is written against eBird's published API documentation, not against live bytes.** Every `/v2/` endpoint answers `403` without an API key this repository does not have — confirmed against four of them (`data/obs/{region}/recent`, `data/obs/geo/recent`, `ref/hotspot/{region}`, `product/spplist/{region}`), each with no key and with a bogus one. The three fixtures are eBird's own documented example bodies from the eBird API 2.0 Postman collection (`documenter.getpostman.com/view/664302/S1ENwy59`, retrieved 2026-09-11), taken verbatim; the region one is trimmed to its first three entries. Field optionality is read off those bodies rather than assumed: `subId` is in the region example and absent from both geo examples, and the notable-observations example carries fifteen fields none of the others has. Test literals probing shapes the documentation does not illustrate (a missing `howMany`, a date-only `obsDt`) are constructed in the test module and named as constructed. |

#### G‑28 · Notification delivery resilience — SHIPPED (`*_file`), and the queue half was a mis-read

| | |
|---|---|
| **Upstream** | `conf.PushSettings` — circuit breaker (max failures, timeout, half-open probes), periodic health check, token-bucket rate limiting, per-provider filters on type/priority/component/metadata, a **script** provider (exec with env/stdin format), and webhook auth with `*_file` secret indirection for Docker/Kubernetes secrets. |
| **Ours** | `crates/birdnet-integrations/src/retry.rs` (exponential backoff), `dispatch/limit.rs` (a per-destination circuit breaker *and* rate limiting — its header: *"Two guards on outbound notifications"*), an `outbound_queue` table for store-and-forward replay, seven native targets plus Apprise, and webhook auth in `alert_rules.rs` (bearer/basic/custom header). |
| **Verdict** | Our store-and-forward queue is arguably stronger than a circuit breaker for the actual failure mode (a station's uplink is down for an hour), and the *fast-fail* half has shipped too — it landed in `80bc37e` (2026‑09‑01), the day before this row was first written, which said it was missing. `dispatch/limit.rs` carries a per-destination `Breaker` (`:64`): closed until `TRIP_AFTER = 3` consecutive failures, then open for `OPEN_BASE = 60 s` doubling per trip up to `OPEN_CAP = 30 min`, with one half-open probe admitted each time the open period elapses. What is missing is narrower than the row used to say: while a circuit is open the dispatcher *drops* the notification — `apprise.rs:491` counts it as `skipped_circuit_open` under `birdnet_notifications_dropped_total` — rather than parking it in `outbound_queue` (no `outbound_queue`/`enqueue` reference under `dispatch/` or in `apprise.rs`), and no credential the dispatcher reads has a `*_file` form (the only `*_file` in the crate is MQTT's `ca_file`, a trust anchor, not a secret). |
| **`*_file` — shipped** | `crates/birdnet-core/src/config/secret_file.rs`. `BIRDNET_<KEY>_FILE` names a file whose contents become the value, for the five credentials that leave the station: `NOTIFY_URLS`, `APPRISE_URL`, `BIRDWEATHER_TOKEN`, `MQTT_PASSWORD`, `HEARTBEAT_URL`. Environment only — this is a container facility, and a station editing `birdnet.conf` can put the secret in the file it is already editing. The direct value wins and the file is reported; an unreadable or empty file is an error-level report, not a silent off; whitespace around the value is trimmed and newlines inside it are kept. The one that is easy to miss: a file-supplied value is **not** seeded into the `settings` table, because that would put the credential in the database and so in every backup of it — the exposure the mount exists to avoid. `APPRISE_CONFIG` is excluded (its `_FILE` name is already taken, by an Apprise config path) and the SMTP password is excluded (settings-table only); a gate asserts no indirect key can collide with an existing config key. |
| **The queue half — this row was wrong about the consequence, and the correction is the finding** | It said an open circuit means the notification is *lost*. For the traffic that matters it is not. `src/integrations/announce.rs` already holds every operational alert in an `Outbox` keyed by episode and retries it **at every poll until it is delivered**: `send` returns `AppriseError::AllDestinationsSkipped` when every destination's circuit is open (`apprise.rs`, the `(0, None) if skipped_open + skipped_limited > 0` arm), `flush` treats that as undelivered and calls `outbox.settle(&key, false)`, and the entry stays. It even does the part that would have been the hardest to add: `Alert::body_at` appends *"(Raised N minutes ago; earlier attempts to send this did not reach a destination.)"* once the wait passes `LATE_AFTER`, so a late alert says it is late instead of reading as current. The first failure is recorded in `notification_log` as `NotifStatus::Queued`. So an alert about the station is already store-and-forward — in memory, at the announcer, rather than in `outbound_queue` at the dispatcher. |
| **What is actually left** | Two narrow things, neither obviously worth a queue table. (1) The outbox is in memory, so a station that **restarts** while its notifier is down loses what was pending — but the health loop re-derives the condition on its next poll, so a still-true alert comes back and only a transient one (a burst of capture restarts an hour ago) is really gone. (2) A station that runs an Apprise server *as well as* native routes takes the Apprise POST's result as the answer, so a native destination whose circuit is open is masked; the alert still left the station by the other path. Routine detection notifications stay dropped, which is `outbound_queue`'s own stated design — its module header says replaying a look-now alert hours later is worse than dropping it, and for a bird alert that is right. |
| **Plan** | The `*_file` half has shipped (above). The queue half is **not planned**: the mechanism it asked for exists one layer up and is better placed there. The script provider is declined: arbitrary command execution configured through the web UI is the same RCE surface as the declined web terminal. |

#### G‑29 · Metric-triggered alert rules — SHIPPED (differently from the plan, with the reason)

| | |
|---|---|
| **Upstream** | `internal/alerting/` — a rules engine over both **events** (detection, stream error) *and* **metrics** (CPU %, memory %, disk %), with a typed schema of operators (`is`, `in`, `contains`, `>`, `≥`, …), escalation steps, per-metric-key cooldowns, and persisted history. |
| **Ours** | `crates/birdnet-db/src/alert_rules.rs` — detection-triggered only: species glob, confidence range, hour window, day-of-week; actions webhook/log/suppress; import/export with credential redaction. Separately, `BIRDNET_STATION_HEALTH_ALERTS` and the deadman timer cover some system alerting with fixed thresholds. |
| **Why it matters** | The station failure that actually loses data is silent: the disk fills, or the microphone dies, and nobody notices for three weeks. Fixed-threshold health alerts cover the obvious cases; a rule engine lets an operator say "tell me if disk goes over 85 % *or* if the hourly detection count drops below 20 % of its 7-day median", which is the one that catches a dying microphone. |
| **Resolution** | `crates/birdnet-db/src/metric_rules.rs` (migration 48) plus a fifteenth check in `src/integrations/station_health.rs`. Seven measurements — `disk_pct`, `mem_pct`, `cpu_temp_c`, `detections_per_hour`, `seconds_since_last_detection`, `capture_restarts_per_hour`, `outbound_queue_depth` — each rule an `above`/`below` comparison against a threshold. Created, listed, toggled and removed on the same **Station → Alerts** page as the detection rules. |
| **Three deliberate departures from the plan above** | **(1) A separate table, not a discriminant on `alert_rules`.** The two are different mechanisms sharing a word: an `alert_rules` row matches one *detection* as it arrives and fires an action of its own (webhook, log, suppress); one of these is a *sampled measurement* compared every five minutes. Bolting a discriminant on would have made every detection-rule read carry columns that are always NULL, and given neither mechanism the other's machinery. **(2) Evaluated by the station-health poll, not the maintenance loop.** The maintenance loop is daily and weekly — the wrong cadence for "tell me when the disk passes 85 %". The health poll is every five minutes and, more to the point, a firing rule becomes an ordinary `Condition`, so it inherits the three-poll debounce, the episode latching, the recovery notice, the notification log, the store-and-forward outbox and a place on `/api/v2/health/conditions`. A parallel engine would have had none of that. **(3) No per-rule cooldown field, and no `EXPORT_VERSION` bump.** The episode *is* the cooldown; and nothing was added to the `alert_rules` export, so its version is untouched. Metric rules are not exportable yet — see below. |
| **Two decisions worth stating** | A metric with no reading produces **no** alert: a board with no temperature sensor is not cold, and treating a missing sample as zero would make every `below` rule fire for ever. (That mutation survived the first version of the gate — an `above` rule cannot tell the two apart — so the gate now uses a `below` one.) And a rule that could never stop firing, such as "disk above −1", is refused at creation. |
| **Resolution (export/import)** | Shipped. `RuleSet` carries `metric_rules` alongside `rules`, `EXPORT_VERSION` is 2, and `/admin/rules/export` and its import handle both. The version bump is only safe because a version-1 file still imports — the field carries a serde default, so its absence reads as "no metric rules" rather than a parse failure that would take the detection rules with it; a gate removes that default to prove it catches the regression. A file from a newer station is still refused with a message naming both versions. Metric rules have no credential, so `?secrets=1` and the `redacted` flag say nothing about them, which the documentation states rather than leaving ambiguous. Two refusals are deliberate: a metric this station does not implement is **named and skipped** (a newer station's file would otherwise import looking complete while missing the rules that mattered), and `from_export` runs the same `validate()` the form does, so "disk above −1" is refused on both paths — an import is not a way around the check. |

#### G‑30 · Backup destinations — SHIPPED

| | |
|---|---|
| **Upstream** | `internal/backup/targets/` — `local`, `ftp`, `sftp`, `rsync`, `gdrive`; **no S3 target** (an earlier draft listed one; corrected per UNATTENDED_DEPLOYMENT_AUDIT.md §1.2 and §2, which also found the package dormant, with one importer and no route); encryption with an auto-managed AES-256-GCM key; retention by age/count/minimum; daily and weekly schedules. |
| **Ours** | Local, SFTP, S3-compatible and **rsync-over-SSH** (`crates/birdnet-integrations/src/offsite/`), an encrypted container (`offsite/envelope.rs`), SigV4 signing, keep-N retention, and a restore path. |
| **Verdict** | Close, and on S3 we have more than they do. `rsync` is the one worth adding — it is what a person with a NAS already uses, it is incremental, and it does not need a bucket or an FTP daemon. FTP is declined (cleartext by default, and FTPS is worse-supported than SFTP everywhere it matters). Google Drive is declined (an OAuth flow and a third-party dependency for a destination `rclone` already serves). |
| **Plan** | Add an `rsync` target driving the system binary over SSH with the existing host-key policy from `offsite/sftp.rs`, and daily-schedule support alongside the current weekly. |
| **The plan's stated reason for rsync is wrong, and measuring it is the finding** | This row justified rsync with "it is incremental". It is not, here — backups are encrypted before they leave under a fresh random argon2 salt and nonce prefix, so every run derives a different key and produces unrelated ciphertext. Measured with a scratch probe against the real `envelope::encrypt` (1 MiB file, rsync's 700-byte block size, matches counted at **every** offset rather than only aligned ones): plaintext with one 400-byte region changed → **1496/1497** blocks reusable; the same change encrypted → **0/1498**; and byte-**identical** plaintext encrypted twice → **0/1498**. The last figure settles it: with nothing changed at all, the salt alone leaves no block in common. rsync transfers 100 % of every backup, exactly as SFTP does. |
| **Why it shipped anyway** | The three reasons that survive the measurement. (1) **It resumes** — `--partial`, into a partial directory of its own so an interrupted transfer never sits under the final name where retention and restore would see it. A station on a rural link that dies at 90 % of a 1.3 GB backup continues; over SFTP it restarts, and on a bad enough link may never finish. (2) **`--bwlimit`**, so a backup does not saturate a shared or metered connection. (3) It is what a NAS owner already has. The module header states all of this, including the measurement, rather than repeating the row's original claim. |
| **rsync moves the bytes; `sftp` does the housekeeping** | Not a shortcut. rsync has no primitive that removes one named remote file, and its nearest idiom — an empty source directory with `--delete` plus an include filter — deletes **everything the filter does not name**, so one wrong filter costs every offsite backup. `sftp` already removes exactly one file under a quoting and allowlist discipline written and tested once. Directory creation is `sftp`'s too: rsync's `--mkpath` needs 3.2.3 (2020) and an older rsync would fail on first run. A station using this target needs `rsync` and `sftp` — one package more than it already had, since the transport is SSH either way. |
| **One list, not two** | `SftpTarget::ssh_policy_options` is now shared by the `sftp` client and by the `ssh` transport handed to rsync. A copied `-o` list is the obvious way for host-key checking to be enforced on one path and quietly absent from the other, and neither path's own tests would see it; a gate asserts the two carry the same policy. Two asymmetries the gates pinned: `ssh` spells the port `-p` where `sftp` spells it `-P`, and the remote-path allowlist **permits a space** (sftp quotes its batch arguments; rsync splits its own), so the rsync target refuses a remote directory its sibling accepts. A colon-refusal branch was written, found unreachable behind the allowlist, and deleted rather than left in as reassurance. |
| **Configuration** | `OFFSITE_BACKUP=rsync` reads the same `OFFSITE_SFTP_*` keys: it genuinely is the same SSH connection to the same account on the same server. A parallel `OFFSITE_RSYNC_HOST` family would be seven more keys, seven more chances to set one and not the other, no new capability, and it would stop an operator switching between the two by editing one word while testing a flaky link. The one setting that is rsync's own is `OFFSITE_RSYNC_BWLIMIT`. |
| **Resolution (the daily schedule)** | `BACKUP_SCHEDULE=weekly` (default) or `daily`, parsed by `birdnet_db::sqlite::BackupSchedule` beside the job keys so the loop that schedules a backup and `GET /api/v2/system/jobs` that reports when it is next due read one definition. Weekly stays the default: switching an existing station to daily multiplies its offsite upload by seven, which is not a change to make underneath somebody. |
| **What `daily` deliberately does not make daily** | The weekly job was four steps — local snapshot, offsite upload, prune, then a **space reclaim** (WAL checkpoint, free pages returned). Shortening its interval would have rewritten parts of the database file every day, and on the SD card this project targets that is write endurance spent for space nobody asked to have back. The reclaim is therefore its own job on its own weekly cadence under the new key `space_reclaim`, and `BACKUP_SCHEDULE` moves only the backup. `G-32`'s catalogue drift gate made this cheap: adding a job to the scheduler without adding it to the catalogue fails a test that scans the source for `pub const JOB_` declarations. A gate asserts the reclaim keeps its weekly interval under **both** schedules, which is what catches the cadence being applied to every job rather than the one that is configurable. `JOB_BACKUP_VACUUM` keeps its `backup_vacuum` spelling — renaming the key would orphan every existing station's recorded history — with its title corrected to "Backup and offsite upload". |
| **A signature that had grown too long** | `spawn_database_maintenance` reached nine positional parameters with this change, two of them `u32` and three of them paths, so a transposed pair would compile and surface as a station pruning the wrong directory. It takes a `MaintenancePlan` struct now. |

#### G‑31 · MySQL / external database — DECLINED

| | |
|---|---|
| **Upstream** | `Output.MySQL` alongside SQLite. |
| **Ours** | SQLite for OLTP, DuckDB for analytics. |
| **Verdict** | The SQLite+DuckDB pairing is this project's architectural thesis — it is why the behavioural analytics are possible at all on a Pi, and a network round trip per row would undo it. A station wanting central aggregation is better served by shipping DuckDB/Parquet exports than by writing detections over the network. |
| **Substitute** | Make the export path good enough that this never comes up: Parquet export of the detections table, already trivially available through DuckDB, exposed as a scheduled job. |

### 2.5 Operations and platform

#### G‑32 · System introspection APIs — PARTIAL (per-source restart, jobs, network interfaces and the process view have shipped)

| | |
|---|---|
| **Upstream** | `/api/v2/system/{info,resources,disks,processes,network-interfaces,jobs,temperature/cpu,external-media,inference}` plus `/api/v2/control/{restart,reload,rebuild-filter,restart-source/:id}`. |
| **Ours** | `/api/v2/health`, `/api/v2/metrics`, `/station/*`, `/system/disk`, `/admin/system/*` and `POST /admin/system/service/restart`. **Per-source restart has landed** — see the resolution below. Still missing: external media detection and filter rebuild. `GET /api/v2/system/network` (interfaces, addresses with prefix, hardware address, byte counters) and `GET /api/v2/system/processes` (the station's process and its descendants — capture `ffmpeg`, `arecord` — with CPU over a 250 ms sample) have shipped, bearer-gated; the process view is deliberately the station's own tree, not the machine's. Our `GET /api/v2/system/jobs` is a maintenance-job catalogue; upstream's `/system/jobs` (`GetJobQueueStats`) is detection-action queue statistics — the same path for a different thing. |
| **Why it matters** | Per-source restart is the one that matters daily — restarting the whole service to recover one wedged RTSP camera drops every other source and loses in-flight audio. |
| **Resolution (per-source restart)** | The supervisor owns its source list privately on its own thread, so this needed a seam in the direction that did not exist: `crates/birdnet-core/src/audio/capture/control.rs` holds the set of pending restart requests the web layer writes and `run_supervisor` drains once per tick (`src/capture/runloop.rs`), the mirror of `status.rs`. A drained request stops the source and clears its fault state, so `reconcile` starts it again on that same tick rather than waiting out a backoff. It goes through the supervisor's *own* start path, so the schedule and quiet window still hold and a request for a paused source is spent rather than overriding them. Three surfaces: a **Restart** button per row on `/admin/audio`, `POST /api/v2/control/restart-source`, and `GET /api/v2/system/capture` to read the labels it takes. Audited as `audio.source.restart`. |
| **Two things this deliberately does not do** | It does **not** reload the source's settings — `CaptureManager` is built once at supervisor start and `start()` respawns from that stored config, so an edit still needs a service restart. And an operator-requested restart is **not** counted in `restarts_last_hour`, so it never raises `flapping`: that verdict exists to spot a source that cannot stay up on its own. Both are gated, the second with its counterpart. |
| **A divergence from upstream's URL, on purpose** | Ours takes the source in the JSON body (`POST /api/v2/control/restart-source`, `{"source_id": …}`) rather than upstream's `restart-source/:id`. `WRITE_ROUTES`/`READ_ROUTES` in `routes/api_write.rs` are literal paths read by the CSRF guard, the OpenAPI gate and four test loops that *send a request to each entry*; a `{id}` placeholder would be a path none of them could exercise, which would cost more than matching a URL shape. |
| **Resolution (the jobs API)** | `GET /api/v2/system/jobs`, bearer-gated beside `/system/capture`. The substantive decision is that it is **not** "over the `maintenance_runs` table" as the plan worded it: that table holds a row per job that has *completed at least once*, so enumerating it omits precisely the jobs worth asking about — a station whose backup has never run would get a short, clean list with the problem absent. `JOBS` in `crates/birdnet-db/src/sqlite/queries/maintenance.rs` is the catalogue, `job_statuses` walks it and looks each row up, and a never-run job reports `last_run_unix: null` with `due_reason: "never_run"`. A unit test scans that file's own source for `pub const JOB_` declarations and fails when one is missing from the catalogue. `ok` is tri-state — never-run and no-verdict both serialise `null` — so a `reports_verdict` flag ships alongside it, because a session prune that succeeded and an integrity check that failed otherwise look identical. |
| **A refactor the gap made necessary** | The scheduler's due-rule was three branches (never run, clock moved backwards, interval elapsed) living only in `src/maintenance.rs`, and a reporting surface would have been a second copy of them. It moved to `birdnet_db::sqlite::due_state` and `due` now calls it, keeping only what a pure function cannot do: read the timestamp off disk and apply the in-process floor that stops a full-disk station re-running a weekly VACUUM every half hour. The clock-went-backwards branch had **no test** before this; it has one now. The API cannot see the in-process floor, which is documented rather than hidden: it only ever delays a job within one process lifetime, so a job reported as due may already have run. |
| **Plan (what remains)** | External media detection and filter rebuild. |

#### G‑33 · Hardware profiling and memory policy — SHIPPED (the capability-gating half)

| | |
|---|---|
| **Upstream** | `internal/hwprofile`, `internal/cpuspec`, `internal/mempolicy` — detect the machine, set `GOMEMLIMIT` and cap the glibc arena, and gate features on available memory (`LowMemoryConfig`). |
| **Ours** | `sysinfo`-based CPU/memory/temperature reporting, `--doctor` checks, and `BIRDNET_DUCKDB_MEMORY_LIMIT`. |
| **Verdict** | Most of `mempolicy` is a Go GC concern that does not transfer to Rust. What does transfer is **capability gating**: refusing to enable DuckDB analytics or a second model on a 1 GB Pi Zero, with an explanation, rather than being OOM-killed at 3 a.m. |
| **Resolution** | `crates/birdnet-behavioral/src/memory.rs`. The buffer pool is sized to a quarter of the *effective ceiling* — the smaller of `/proc/meminfo`'s `MemTotal` and any cgroup limit (`memory.max`, or v1's `memory.limit_in_bytes`) — capped at 256 MiB and floored at 64 MiB. Below the floor, `helpers::state` starts the station **without** analytics and says why, rather than starting something that will be OOM-killed mid-query. `--doctor` reports the decision under *Analytics memory*, because the choice is made once at startup and an operator diagnosing a dead station months later is looking at `--doctor`, not at a boot they no longer have. |
| **Both numbers are anchored, not chosen** | **A quarter** is the proportion the shipped systemd unit already implies (`MemoryHigh=768M`/`MemoryMax=1G` with a flat `256MB` pool), so a station on that unit gets exactly the limit it got before — asserted at compile time, `const _: () = assert!(1024 / POOL_SHARE_DENOMINATOR == MAX_POOL_MIB)`, because the unit test that looks like it covers this is satisfied by the cap alone (measured: halving the fraction still passed it). **64 MiB** was measured: a sessionisation shaped like `queries.rs`'s — `lag` and a running sum over 1.5 M detections partitioned by species, then aggregated — OOM'd at 8, 16 and 32 MiB and succeeded from 48 MiB up on DuckDB 1.5. |
| **What this deliberately does not claim** | The rest of the process's footprint on a Pi is *not* measured — that needs a Pi. So this sizes a **proportion**, not a budget: it makes the analytics engine's share scale with the machine and does not claim the remainder is enough. What would falsify the fraction is a station inside its ceiling still being OOM-killed with the pool at a quarter of it; the answer then is a smaller fraction, not a different mechanism. Stated in the module header rather than left implied. |
| **A defect found while doing it** | `.env.example` shipped `BIRDNET_DUCKDB_MEMORY_LIMIT=256MB` **uncommented**, so any container using that file as its `.env` pinned 256 MiB on every board — including the 512 MB ones this sizing exists for, where it is half the machine. Now commented, with the sizing explained. |
| **Plan (what remains)** | The analytics *sync* is not yet sized from RAM, only the query engine's pool. Upstream's `GOMEMLIMIT`/glibc-arena half does not transfer to Rust. |

#### G‑34 · Machine-readable config schema — PARTIAL (the drift gate has shipped; the schema artifact is declined for now)

| | |
|---|---|
| **Upstream** | `config.schema.json` generated by `cmd/gen-schema`, with tests asserting the shipped schema, the wiki page and the config comments cannot drift apart. |
| **Ours** | `.env.example` (37 kB, hand-maintained), the admin settings pages, and `docs/book/`. |
| **Why it matters** | Three surfaces describe every setting today and nothing gates them against each other, which is exactly the drift this repository's own testing conventions warn about. |
| **Resolution (the drift gate)** | `src/helpers/settings_overlay.rs` gains two source-scanning gates. `every_settings_key_written_anywhere_is_classified` finds every `settings::set` in production code — resolving a key written as a named constant, excluding `#[cfg(test)]` fixtures, both pinned by the scan's own self-test — and fails when one is not in `SETTING_SPECS`. `every_subsystem_owned_setting_is_read_somewhere` is the reverse for `Wiring::OwnedBy` keys. Runtime-keyed `set_many` sites are allowlisted by file with a note naming what bounds their keys, so a new dynamic writer trips the gate. This replaces three hand-maintained per-writer lists with one scan. |
| **What it found** | `analytics_exclude_imports` — written from `/admin/migration`, read by both analytics engines, classified nowhere, so nothing was checking that the control did anything. Now classified. It also reported `timezone` as owned-but-never-read, which was a shape gap rather than a defect: `--doctor` reads it through `setting_from_db(config, …)`. Taught to the scan. |
| **Declined for now — the schema artifact** | `SETTING_SPECS` records a key, its wiring and its category, and nothing else. A JSON Schema derived from it would say `"type": "string"` about every setting and assert nothing; making it useful means adding type, default and description metadata to ~50 specs, which is its own piece of work. The drift such a schema would catch between `.env.example`, the config keys and their readers is already gated from the environment side (`src/helpers/env_keys.rs`, two-way against `.env.example`) and from the config-key side (`tests/every_config_key_is_known.rs`, two-way against the readers). A `Wiring::Bridged` spec's config key cannot be checked against `.env.example` mechanically, because the two namespaces are not parallel — the config key `APPRISE_MIN_CONFIDENCE` is the environment variable `BIRDNET_NOTIFY_CONFIDENCE` — and a hand-written mapping between them would be the very drift the gate is for. Checked, not assumed: 45 bridged keys, all present in `KNOWN_CONFIG_KEYS`, 17 with no `BIRDNET_<same name>` line in `.env.example`. |

---

## Part 3 — what this project has that neither reference does

Recorded for balance, and because the roadmap below deliberately protects these
rather than trading them away to close gaps.

| Capability | Where | Neither upstream has |
|---|---|---|
| **Behavioural analytics on DuckDB** — sessionisation, retention curves, funnels, sequence matching, next-species prediction | `crates/birdnet-behavioral/src/queries.rs` | birdnet-go has time/species analytics; neither has product-analytics primitives applied to bird activity |
| **Phenology** — migration-timing percentiles, inter-annual trend, effort-corrected weekly abundance, peak weeks, weekly richness | `crates/birdnet-behavioral/src/phenology/` | birdnet-go has `analytics/species/phenology`; effort correction and inter-annual trend are ours |
| **Time-series primitives** — tumbling/sliding/hopping/session windows, Shannon diversity, gap characterisation, peak detection | `crates/birdnet-timeseries/` | — |
| **Repetition-based false-positive filter with derived effectiveness check** | `detection/corroboration.rs` — and `minimum_overlap()` *derives* the overlap at which each level stops being a no-op, rather than documenting it | birdnet-go's equivalent has levels but no self-check that a configured level is inert |
| **Quarantine** for implausible detections rather than silent discard | `sqlite/queries/quarantine.rs`, `/quarantine` | — |
| **Encrypted off-site backup with SigV4 signing implemented in-tree** | `integrations/src/offsite/` | upstream leans on SDKs. *Corrected 2026‑10‑21 (9.6): birdnet-go also encrypts backups (`backup.encryption`); only the implementation is ours.* |
| **Share links** — signed, expiring, single-detection public links | `routes/share.rs` | — |
| **RSS and iCal feeds** of rare detections | `routes/feeds.rs` | — |
| **`--doctor`, `--channel-report`, `--migration-report`** — self-diagnosis that measures rather than asserts | `src/doctor/` | birdnet-go has `support collect`; the measured channel report is ours |
| **Audit log** of every operator action | `audit_log` table, written by `crates/birdnet-web/src/audit.rs` | — |
| **First-class BirdNET-Pi migration** with validation, per-species before/after comparison and batch import | `crates/birdnet-migrate/` | birdnet-go has an importer; the species-level reconciliation report is ours |
| **`unsafe` forbidden workspace-wide, `missing_docs` enforced, clippy pedantic+nursery** | `Cargo.toml` | a property of the language choice, but a real operational difference |

---

## Part 4 — the plan, ordered

Ordering is by (value to a station) × (confidence we can do it well) ÷ risk.
Each item links to its finding above. Nothing here is a stub: a tier is done
when the feature works end to end and carries a gate that was observed failing
against the code it was written for, per `CLAUDE.md`.

### Tier 1 — land first

> **Status** is kept current as work lands. "Done" means implemented,
> gated by tests that were each observed failing against the code they guard,
> and documented — not merely written.

| # | Item | Finding | Status | Why first |
|---|---|---|---|---|
| 1 | Trusted-proxy client IP | G‑18 | **Done** | A correctness defect with a security edge, in code that already exists. Small, self-contained, and every later access-control feature builds on a correct client identity. |
| 2 | Sound level monitoring | G‑1 | **Done** | Largest new capability per line of code, no dependencies, pure DSP, and it builds the biquad primitive that G‑2 needs. |
| 3 | Dynamic per-species threshold | G‑12 | **Done** | Highest detection-quality yield available without the multi-model programme. |
| 4 | Species tracking (year/season/return) | G‑16 | **Done** | Highest *user-visible* yield on the list; the data is already in the database. |
| 5 | Per-source parametric EQ | G‑2 | **Done** | Reuses the biquad from #2; replaces three fixed toggles with something a site can actually be tuned with. |
| 6 | Pre-capture across segment boundaries | G‑3 | **Done** | Fixes a silent, invisible data-quality defect in clips we already ship and upload. |
| 7 | Reverse-proxy base path | G‑19 | **Done** | Deployment blocker for a whole class of user; mechanical but must be done exhaustively. |
| 8 | Solar quiet hours | G‑4 | **Done** | The solar maths already existed; shipped as parsing on the existing column, with no tagged form and no migration. |
| 9 | Flickr image provider | N‑1 | **Done** | The provider seam was built for this and has stood empty. |
| 10 | Live-stream frequency shift | N‑2 | **Done** | Accessibility parity; shares the re-encode seam with G‑21. |

### Tier 2 — next

| # | Item | Finding |
|---|---|---|
| 11 | ~~Metric-triggered alert rules~~ — **shipped**, export/import included: both kinds of rule travel in one file at `EXPORT_VERSION` 2. See G‑29. | G‑29 |
| 12 | ~~Notification `*_file` secrets + enqueue-while-open~~ — **closed**: `*_file` shipped, and the queue half turned out to be already covered by `announce.rs`'s outbox. See G‑28. | G‑28 |
| 13 | ~~Taxonomy: synonyms, family, genus~~ — **shipped**: vocabulary alignment (41 birds recovered) and browsing by class/order/genus. No family rank: the label file states none. See G‑15. | G‑15 |
| 14 | ~~Detection comments~~ — **shipped**: `detection_comments`, append-only at the database, attributed, audited. See G‑23. | G‑23 |
| 15 | ~~Weather provider trait + Wunderground + yr.no~~ — **shipped** as an enum over three providers; OpenWeather declined with a reason. See G‑26. | G‑26 |
| 16 | ~~eBird recent-observations client~~ — **shipped**: a cached client, corroboration on two pages, and a stated refusal to let eBird's silence count against a species. See G‑27. | G‑27 |
| 17 | ~~Loudness normalisation of exports~~ — **shipped**, see G‑5 | G‑5 |
| 18 | ~~Jobs API over `maintenance_runs`~~ — **shipped** as `GET /api/v2/system/jobs`, driven by a job catalogue rather than the table so a never-run job is visible. See G‑32. | G‑32 |
| 19 | ~~`rsync` backup target + daily schedules~~ — **shipped**: rsync for resume rather than the incrementality the row assumed (measured at 0 %), and a daily backup that leaves the space reclaim weekly. See G‑30. | G‑30 |
| 20 | ~~Config schema generation + drift gate~~ — the drift gate has shipped; the schema artifact is declined with a reason. See G‑34. | G‑34 |
| 21 | ~~A station-wide default source for the live stream~~ — **shipped**, see N‑3 | N‑3 |
| 22 | ~~Bulk species management page~~ — **shipped** at `/admin/species/manage`, with bulk actions that never touch a locked detection. See N‑4. | N‑4 |
| 23 | ~~Watchdog tuning~~ — **shipped**, see G‑9 | G‑9 |
| 24 | ~~Memory-budget capability gating~~ — **shipped**, see G‑33; sizing the analytics *sync* remains | G‑33 |
| 25 | ~~Noise "remember" window~~ — **shipped** as `NOISE_REMEMBER_SECS`, suppressing only the species a bark produced. See G‑17. | G‑17 |

### Tier 3 — programmes, not tickets

| # | Item | Finding | Shape |
|---|---|---|---|
| 26 | Multi-model classifier stack | G‑10 | **Stages 1–5 shipped.** Perch v2 exposed two defects (class head by position, sample rate derived not declared), both fixed against the real model; the catalogue and verified installer ship. Classifiers with different windows now run together — chunk cut to the longest, stepped by the shortest, so none loses coverage — and agreement means both judged audio from the same chunk start. Only a differing sample rate is still refused. Stage 6 (bats) follows |
| 27 | Inference backends (OpenVINO / XNNPACK) | G‑11 | Not gated: the trait it waited for (#26 Stage 1) has shipped. Nobody has built it |
| 28 | OAuth2 / OIDC | G‑20 | One generic OIDC implementation; named providers are configuration |
| 29 | ~~HLS live streaming~~ — **shipped** (upstream's API; fixed encoder settings). See G‑21. | G‑21 | Shares the encode seam with #10 |
| 30 | ~~Dashboard layout customisation~~ — **shipped** (order and visibility). See G‑22. | G‑22 | The HTMX partials are already the component vocabulary |
| 31 | Extended capture | G‑6 | Not gated: what it depended on, pre-capture across segments (#6, G‑3), is Done. Nobody has built it |
| 32 | ~~Additional stream protocols~~ — **shipped** (HTTP/Icecast, HLS, RTMP, UDP, SRT; `rtp://` declined). See G‑7. | G‑7 | One kind, the URL decides the protocol |
| 33 | Silero VAD privacy gate | G‑13 | Not gated: #26 Stage 1 has shipped. G‑13's plan is a second, small ONNX session that gates clip retention, beside the classifiers |
| 34 | ~~RTSP media mode~~ — **shipped** (`full` / `audio`). See G‑8. | G‑8 | It mattered for RTSP on its own — the camera's video sessions — not only alongside #32 |
| 35 | ~~Profiling endpoints~~ — **shipped** (CPU, behind the `profiling` feature). See G‑24. | G‑24 | No heap profile |
| 36 | ~~Local error ring buffer (Sentry substitute)~~ — **mostly shipped** as the capped `errors.jsonl` and its place in the support bundle; no page shows it. See G‑25. | G‑25 | |
| 37 | Parquet export (MySQL substitute) | G‑31 | |
| 38 | Bat support + ultrasonic filter | G‑14 | Depends on #26 Stage 6 and a ≥192 kHz capture path |

### Not planned

G‑25 (Sentry), G‑31 (MySQL), the script notification provider in G‑28, FTP and
Google Drive in G‑30, and the BirdNET-Pi Adminer / file manager / web terminal.
Each has a stated reason above and a substitute where one is warranted.

---

## Part 5 — birdnet-go `1e74c82` → `780ce51` (2026‑09‑02 → 2026‑10‑11)

Every `feat` commit in the range was read from its diff, and the `fix`/`perf`
commits were triaged for user-visible capability; about forty Svelte-only,
wizard-only and Go-specific fixes were grouped rather than checked one by one.
Our side was read at `4e8fe34`. Statuses are kept current as work lands, as in
Part 4.

### Gaps and partials

| Upstream | What it does | Status | Ours (evidence) |
|---|---|---|---|
| `221262b`, `ef76b6f` reject non-finite scores | NaN / ±inf scores never become detections | **SHIPPED** | `compute_confidence` sent `+inf` to 1.0 on both output paths; non-finite now maps to 0.0, a non-finite human score masks the clip, and the geomodel's `prob < sf_thresh` test — which NaN never satisfies — now reports a non-finite output as a run-time failure instead of passing the species (`inference/model.rs`, `inference/species_filter.rs`). |
| `631cdf1` forwarded client-IP headers must agree | subnet bypass decided on a strictly resolved address | **SHIPPED** (the defect it exposed here) | We have no subnet bypass, but `client_ip.rs` let a client-written `X-Forwarded-For` win behind a proxy that set only `X-Real-IP`. When the two name different clients the request is now the proxy's. |
| `780ce51` FLAC detection tags | Vorbis comments in saved FLAC clips | **SHIPPED** (FLAC, MP3 and Ogg) | The intermediate WAV gets its RIFF INFO chunk before conversion and ffmpeg maps it to each format's tags — checked against this container's ffmpeg (libavformat 60.16) for all three. Upstream also writes the model and app version; ours carries the species, confidence, date and loudness note the WAV already had. |
| `50547d0` ONNX Runtime spin-wait off | idle intra-op threads stop burning CPU | **SHIPPED** | `classifier_session_builder` in `inference/model.rs`. Measured here: CPU per inference 93–98 ms → 52–54 ms, wall time +3–4 ms (4-vCPU x86_64, BirdNET v2.4 INT8). The geomodel runs one intra-op thread and has no pool to spin. Not measured on a Pi. |
| `6dfdc10` MQTT first/last detection time | `speciesFirstDetectedAt` / `speciesLastDetectedAt` in the payload | **SHIPPED** | `species_first_detected_at` / `species_last_detected_at` (`mqtt/types.rs` `SpeciesHistory`, read by `species_tracking::prior_detections`). One deliberate difference: upstream sends `null` both for a first detection and for a failed read; ours omits the keys on a failed read, so a "new species" automation cannot fire on a database error. |
| `63e7c64` Pirate Weather | a fourth weather provider (Dark Sky-compatible) | **SHIPPED** | `Provider::PirateWeather`, reading the `hourly` block (upstream reads `currently`). Built against Pirate Weather's field definitions, not a live response; `windBearing` is taken as the direction the wind comes from, an assumption the definition does not settle. |
| `0b51812` BirdWeather token check | refuses to save sharing-on with an empty or malformed token | **SHIPPED** (characters, not length) | Saved and used trimmed; a token with anything but letters and digits is refused on save; the client encodes it as one path segment. Upstream also requires exactly 24 characters — not adopted, because BirdWeather's own documentation was not available to confirm it, and refusing a valid token turns sharing off. |
| `7def0cf` detections filtered by source | `source` on the detections list API | **SHIPPED** | `/api/v2/detections` now runs on the search page's `DetectionFilter`, which also fixed `?species=…&date=…` silently dropping the date. |
| `a73db53` dawn-chorus window | `period_days` / `min_days` on the API | **SHIPPED** (the whole insight, not only its window) | This row first cited `routes/pages/dawn_chorus.rs`, which draws hourly activity ribbons — a different computation. Upstream's insight is each species' average and earliest *first* detection in 04:00–10:00, which nothing here served as an answer. `GET /api/v2/insights/dawn-chorus` now does, with upstream's parameters, limits and fields (`routes/insights.rs`, query `birdnet_db::sqlite::queries::insights::dawn_first_songs`). See Part 6. |
| `e853776` a model failing every window | persistent notice, per-model health, a health check | **SHIPPED** (no live banner) | Per-classifier health on the registry (`inference/health.rs`, upstream's threshold of 10 and its two error classes); the `classifier:<id>` condition in `station_health.rs`, so the notice is an ordinary station-health episode; a `classifiers` array on `/api/v2/health`, failing ones a `?strict=1` fault; four `birdnet_model_*` series. It also fixed what the gap hid: a failing *second* classifier failed every file, the primary's detections with it. Upstream's live dashboard banner has no counterpart — the alert and the Station Health conditions carry it. |
| `712d4c2` stream metrics | per-source errors and data rates | **SHIPPED** (data rate; wire rate and engine not applicable) | Errors, health and stalls per source were already exported (`birdnet_capture_restarts_total`, `birdnet_audio_source_up`, `birdnet_capture_stalls_total`). The data rate is `birdnet_audio_analysed_seconds_total{source}`, read from each analysed file's header: checked on a running station against a UDP and an HTTP source. Upstream's wire rate is the compressed bytes off the network, which ffmpeg does not report to this process; its engine gauge chooses between two ingest engines, and there is one here. |
| `dee43f5` effective overlap from measured latency | caps overlap to what the device sustains, shows configured vs effective | PARTIAL | The static "needs overlap N s" per level is shown; nothing measures. Its effect cannot be judged off a Pi. |
| `cd488cc` browser location | `navigator.geolocation` button | PARTIAL | IP-based lookup in onboarding only. Needs a secure context, which a LAN station on plain HTTP is not. |
| `54b5545` place search | Photon geocoder in settings | GAP | Plain latitude/longitude inputs. Would add an outbound host to the egress inventory. |
| `6d2335c`, `64aa9e2` SSRF guard on outbound probes | | **SHIPPED** for webhooks (O‑11) | `birdnet_integrations::egress_guard`, upstream's policy. Not applied to the native notification senders, which some stations reach only through a proxy. |
| `0e7290d` location change without restart | | PARTIAL | Latitude/longitude are `Wiring::Bridged`: next restart. |
| `92067fa` spectrograms after audio retention | | GAP | Spectrograms are rendered on demand from the clip. |
| `aa86b38` per-source Home Assistant state topics | | GAP (minor) | Discovery publishes station-wide sensors only. |
| `111fb34`, `121f005` host-network Compose, Portainer/Unraid templates, mDNS in the container | | PARTIAL / GAP | Bridge-network Compose only; no nss-mdns in the image. Not verifiable without a Docker daemon. |
| `0f46189` Africa regional tiles | three more regional model variants | GAP, inside G‑10 | No regional variants in `model_catalog.rs`. |

### Parity, not applicable, or declined

`af617ba` settings deep links — PARITY (`#set-…` anchors; there are no tabs).
`1b7e56c` ffmpeg stderr tail — PARITY in shape (we drain and promote stderr);
whether ffmpeg echoes RTSP credentials into the lines we log is **unverified**.
`812237a`, `e445493`, `ad25bd2`, `c39534a`, `9a1fb24`, `7dcae61`, `db87734` —
PARITY. `2210354`/`c404137`/`ffd6d52` native RTSP ingest and `b4e39ca` native
MP3 encoder — not applicable (ffmpeg is our ingest and encoder). `8f0d3ec`,
`c04d0cd` model-catalog dependencies and optimisation offers — not applicable
to a compiled-in two-entry catalogue. `0ee57b1`, `32e1f50` ultrasonic export and
audible bats — wait on G‑14. `83ed7b9` running with zero acoustic models —
declined: `inference/registry.rs` refuses an empty registry on purpose.
`98b1c62` `lang` follows the UI language — not applicable (the UI is English). *Corrected 2026‑10‑21: "the UI is English" is the gap, not a reason; birdnet-go ships 15 UI translations (A‑5).*

### Corrections to Part 2 this re-check found

* **G‑13**'s "Ours" cell described a rank-based filter that had been replaced.
  Corrected above. Upstream now embeds Silero in every build (`3e03da1`); our
  plan to ship it alongside stands. (Since then the rank rule has returned beside the score rule, on by default; G‑13 has the current description.)
* **G‑20** upstream now always sends PKCE (S256) (`90ebedf`).
* **G‑22** upstream's colour schemes are five plus custom, not six.
* **G‑26** upstream's set is now yr.no, OpenWeather, Wunderground and Pirate
  Weather; ours is Open-Meteo, MET Norway (yr.no), Wunderground and Pirate
  Weather.
* **G‑32**'s "Ours" cell listed "a job list" as missing while the same row said
  the jobs API shipped. Corrected, with the note that upstream's
  `/system/jobs` is `GetJobQueueStats` — detection-action queue statistics,
  not a maintenance-job catalogue — so the two share a path, not a meaning.

## Part 6 — the analytics and insights surface (birdnet-go `780ce51`)

Parts 2 and 5 compared capabilities; neither went endpoint by endpoint
through upstream's analytics API, and the one row that touched it (dawn chorus,
above) cited a chart that computes something else. This part does: each of the
29 routes `internal/api/v2/analytics/handler.go` registers was read, and our
counterpart found by grepping for the concept and opening the code on both
sides. Five are client conveniences (batch variants, a thumbnail URL builder,
a sun-times helper for chart shading) and not features; five are at parity in
substance — diversity, accumulation, phenology, hourly and daily activity,
hour-of-day distribution — though ours take a look-back window where upstream
takes a date range.

| Upstream | What it answers | Status here |
|---|---|---|
| `/insights/dawn-chorus` | Each species' typical first song of the morning | **SHIPPED** — `GET /api/v2/insights/dawn-chorus` |
| `/insights/migration` | Recent first-ever arrivals; species gone quiet | **SHIPPED** — `GET /api/v2/insights/migration`. Gone-quiet had no counterpart; arrivals existed only as the life list's six newest. |
| `/insights/expected-today/regional` | eBird species seen nearby, not heard here | **SHIPPED** — `GET /api/v2/insights/expected-today/regional`, from the snapshot already in memory. One deliberate difference: a failed read of the station's own history is a `500`, not the unfiltered eBird list upstream falls back to. |
| `/insights/expected-today` | Species heard within ±3 days of today in past years | **SHIPPED** — `GET /api/v2/insights/expected-today`: ten earlier years, one window each, so a window across New Year counts once. (The Migration tab's 42-day "still expected" count answers a different question and stays.) |
| `/dashboard/kpis` | Lifetime species, today, best day, detection streak | **SHIPPED** — `GET /api/v2/dashboard/kpis`, upstream's fields and empty values. The streak is counted in SQL over every detection date, where upstream reads the 90 most recent and so stops at 90. |
| `/species/detections/new` | Species first seen in a date range, with counts | **SHIPPED** — `GET /api/v2/analytics/species/detections/new`, upstream's path, defaults, paging and fields. One deliberate difference: a species with an imported detection of no date is never new, as for the daemon's new-species notification; upstream ignores such rows and would list it. |
| `/species/summary` | Per-species count, first, last, mean and max confidence over a range | **SHIPPED** — `GET /api/v2/analytics/species/summary`, upstream's path, parameters and fields (no `species_code`). First and last are by instant, so the repeated autumn hour orders correctly. |
| `/species/daily` | One date's species with hourly counts and first/last heard | **SHIPPED** — `GET /api/v2/analytics/species/daily`, upstream's parameters, sort and fields with its `omitempty`. Its tracking fields come from upstream's live tracker, which answers for today whatever date is asked; here they are read from history as of the date, with upstream's 7-day windows and its seasons by hemisphere. `days_since_last_seen` is the gap back to the previous day heard, where upstream reports the gap at the latest detection of an open novelty episode. `species_code` is not sent. |
| `/time/distribution/species`, `/time/succession` | Top-N species' hour-of-day profile, normalised or stacked | **SHIPPED** — both, upstream's defaults (5 and 6 species, at most 10), repeated `species` selection and fields. Hours are read from the local `Time` column; upstream applies the zone offset at the range's start to the whole range, so across a change of the clocks its hours drift by one and ours do not. The dawn-chorus ribbons stay. |
| `/insights/phantom-species` | Last 30 days, ≥ 3 detections, mean confidence < 0.6 | **SHIPPED** — `GET /api/v2/insights/phantom-species`, upstream's rule and fields. `/admin/quality/phantoms` answers a different question (four shape signals over all history) and stays. |
| `/confidence/distribution` | Per-species confidence histogram | **SHIPPED** — `GET /api/v2/analytics/confidence/distribution`, upstream's parameters, bounds, minimum of 20 detections and camelCase fields. Ties in volume are broken by scientific name, where upstream breaks them by its internal label id. The station-wide six-bucket histogram stays for the quality page. |
| `/time/year-over-year` | Cumulative year-to-date against last year | **SHIPPED** — `GET /api/v2/analytics/time/year-over-year`, upstream's parameter, fields and leap-day alignment. `/api/v2/timeseries/year-over-year` (weekly totals) answers a different question and stays. |
| `/analytics/sources` | Sources with counts over a range | **SHIPPED** — `GET /api/v2/analytics/sources`, upstream's path, parameters and fields. One deliberate difference: names are not anonymised for visitors who are not signed in, because the public Station Health tab already shows them; private mode hides both. |
| `/time/heatmap` | Date × time-of-day density, 15–60 minute slots, CSV | **SHIPPED** — `GET /api/v2/analytics/time/heatmap`, upstream's slot widths, sparse columnar payload and CSV columns. Slots are read from the local `Time` column, so a day of the clocks changing has its detections where the station's clock put them. The hour × weekday heatmap stays. |
| `/time/dawn-onset` | Each day's chorus onset against civil dawn | **SHIPPED** — `GET /api/v2/analytics/time/dawn-onset`, upstream's rank (third detection), minimum (five) and fields; civil dawn from `birdnet-scheduler`, placed on each day's local date and offset. |
| `/system/database/overview` | Table stats, latencies, slow queries | **SHIPPED** — `GET /api/v2/system/database/overview`, bearer-gated as upstream gates it, upstream's fields. Read latency, query rates, slow queries, busy timeouts and checkpoints are `null`: not measured here, and upstream's checkpoint figure comes from running one on a GET. Upstream's `integrity_check` is a constant `ok`; ours is the recorded verdict. |
| `/range/heatmap` | Geomodel occurrence over a lat/lon grid | GAP — the largest: grid inference in the web process and a map without a tile host. |

Found on the way, both the silent-zero shape `CLAUDE.md` describes: the
suspect-species report read a failed query as "Nothing looks suspicious", and
the dawn-sequence card read one as "Not enough dawn activity yet". Both now say
the data could not be read.


## Part 7 — birdnet-go `780ce51` → `f4f019a` (2026‑10‑11 → 2026‑10‑15)

Thirty-seven commits. Twelve of them are generated dependency-licence updates or
the like; the rest were sorted by subject line, and three were looked at beyond that, to
the depth each row says (one diff's key lines, one change's file list, one
change's message and test). A subject-line triage can miss a change whose message undersells
it, so a row saying "not applicable" means the subject did not touch anything this
project has, not that the diff was checked.

| birdnet-go change | Here |
|---|---|
| `5eb998d` third-octave bands measured with a tested DSP library, "correcting high-band levels" | Same class of defect as ours, found independently: the top bands' edges were wrong near Nyquist. Ours is corrected and measured in `tests/soundlevel_accuracy.rs` (CHANGELOG, "Make each third-octave band measure the band it names"). Their change was not compared sample for sample. |
| `a122b6b` *read*: float → 16-bit conversion by 32768, round half to even, saturating; was 32767 and truncation (resampler) | Ours had the truncation (fixed earlier in this release) and the 32767 scale (fixed in `Export a 16-bit recording's clip bit for bit`). Rounding here is half away from zero, not half to even; they differ only on exact ties. The live-gain half of that commit (gain edits applied to running sources without a restart) was not compared. |
| `00941d4` trim a trailing partial sample before WAV/FLAC export | Not applicable: clips here are written from decoded `f32` samples, never from raw byte buffers. |
| `4610` evaluate every new voice-activity hop before trimming | Not applicable to a stage we do not have: there is no voice-activity stage here (the privacy filter reads BirdNET's human labels; see G‑13). Whether there should be one is G‑13, a GAP that predates this range and is in the plan (Tier 3 #33). The hash is four characters as first written and the commit was not identified. |
| `ed89e32` *read (stat only)*: trusted reverse proxies configurable from the Security page, and a read-only endpoint saying how the requester's connection relates to the list | Gap in kind, not in function: the list exists here (`BIRDNET_TRUSTED_PROXIES`, `client_ip.rs`) and is set from the environment or the config file only; no page edits it, and no endpoint reports how a request was classified. |
| `2ddb837` *read (message and test only)*: Perch sound-class labels such as `Bathtub_(filling_or_washing)` kept whole instead of split at the first underscore | Not verified here. The text-format label parser splits at the first underscore; the CSV and TSV parsers do not. Whether a Perch label file in text form reaches the first one was not checked, and no Perch label file is on this machine. |
| `da2938f`, `ba1e9f4` clip clean-up leaving spectrogram renders, stale clip references | Not applicable to the first: spectrograms are rendered on demand here and never stored. The second (stale clip references on aged-out libraries) was not compared against `maintenance.rs`. |
| `335d8d1`, `c275285` reconcile-pass back-off | Not compared. |
| `86a9863` unreadable cgroup memory limits reported as unknown | Not compared against `memory.rs`. |
| the `go-audio-dsp`, `go-flac`, `go-mp3`, `go-m4a`, `go-opus` refactors; native AAC/MP3/Opus clip encoding with no FFmpeg; Node 26 CI | Implementation changes in a Go project. Native clip encoding would be a feature here, but the statement this row first made, that clips are WAV only, was wrong: clips are WAV, converted to MP3, FLAC or Ogg by `ffmpeg` or, failing that, `sox` (`crates/birdnet-core/src/audio/extraction/format.rs`, `convert.rs`). Opus and AAC are not offered; see P‑18. |
| `9642f74`, `c77f11b` interface-language and settings-page fixes | Not applicable: this interface is English, and species names are the localised part (Part 5). *Corrected 2026‑10‑21: see A‑5; the interface language is a gap.* |

## Part 8 — birdnet-go `f4f019a` → `a1a77af`, and the parity audit of 2026‑10‑20

Sixty-five commits, and the 22 findings the audit listed, P‑1 … P‑22.

**What was and was not checked.** The upstream columns are the audit's citations (a file and line in birdnet-go, one in BirdNET-Pi) and were **not re-read** for this part; upstream was not reachable from the session that wrote it. **They were re-read on 2026‑10‑21: see 9.1**, which corrects P‑3, P‑4, P‑6, P‑15, P‑17 and P‑20 (marked below). The `Ours` column was checked at the commit this was edited on: each cell names the file and says what a grep or a read of it showed. It was written first when none of P‑1 to P‑6 was in the tree; rows P‑1 to P‑4 and P‑6 were rewritten on 2026‑10‑20 after the work landed on this branch (commits named in each cell), and what each commit's tests did and did not cover is said in its cell. The verdicts are the document's (GAP, PARTIAL, SHIPPED, PARITY, DECLINED).

Tallied from the last column by script after the 2026‑10‑21 corrections, counting P‑2 once, as PARTIAL (its 0,0 default is fixed; fuzzing and a separate threshold are not): 4 SHIPPED (P‑1, P‑3, P‑5, P‑6), 6 PARTIAL (P‑2, P‑4, P‑7, P‑14, P‑15, P‑22), 12 GAP (P‑20 moved from PARTIAL to GAP).

| # | Upstream (the audit's citation) | Ours (checked) | Verdict |
|---|---|---|---|
| P‑1 | Alert-rule cooldown per rule and species (`internal/alerting/engine.go:337,436,475`) | `alert_rules.cooldown_secs` (migration 55, `51eaaa71`) and the gate in `src/daemon/alert_cooldown.rs`, which runs on the detection's own time, applies to webhook actions only and gives its window back when the request never left. The rule form sets it (`49def1d8`): minutes, 5 by default, refused above 7 days. The export is version 3. Upstream's "per rule *and species*" is matched: the gate is keyed on both. | SHIPPED |
| P‑2 | BirdWeather: the audit lists three differences. Ours posts coordinates 0,0 when no location is set; upstream can fuzz the posted location; upstream has an upload threshold separate from the detection threshold (`internal/conf/config.go:348-349`, `birdweather_client.go:258`) | The 0,0 default is fixed (`0b456c20`): `birdnet_integrations::birdweather::check_location` refuses a missing, non-finite, off-globe or exactly 0,0 position, the client is not built without one, `--doctor` has a "BirdWeather location" family, and an upload an earlier version parked at 0,0 is dropped. No fuzzing: the only "jitter" in `crates/birdnet-integrations/src/birdweather.rs` is retry back-off. No BirdWeather-specific threshold: the posts are the detections that passed the global one. | SHIPPED (the 0,0 default); GAP (fuzzing, separate threshold) |
| P‑3 | Range-filter explorer: a route that lists the species the filter admits (`internal/api/v2/range/range.go:49-60`) | `GET /api/v2/species/occurrence` and the page `/species/occurrence` (`1f92c980`) give, per species, the model's probability, the threshold in force and the station's decision with its reason (range model, or the operator's lists), for the station's place and a week; a species page for a held-back bird links to it. Checked against the real V2.4 range model (a European station keeps the magpie and holds back a North American bird, and the reverse) and, with the operator's lists, against `filter_species`. Not run: the V3 geomodel variant, a Raspberry Pi, and the memory a second copy of the range model takes. **Corrected 2026‑10‑21 (9.1, A‑14):** upstream's explorer is eight routes, including a what-if at any latitude, longitude, threshold and date, a CSV and a manual rebuild; ours answers for the station's own place and a week only. SHIPPED is for the explorer, not for those. | SHIPPED |
| P‑4 | Container storage-persistence check (`a43718b`, `internal/health/checks/storage_persistence.go:54`) | The Station Health banner and header badge (`b1025a19`, shared list `station_issues`) and `--doctor`'s "Data volume" check (`45781dcd`) flag a data directory on the container's own writable layer, from the mount table. The shipped image declares `VOLUME /data`, so a bare `docker run` gets an anonymous volume, which is a mount and is not flagged although `docker rm -v` or `--rm` deletes it. Tested on `mountinfo` fixtures modelled on Docker's output, not in a container. **Added 2026‑10‑21 (9.1):** upstream judges four paths (config, database, models, clips); ours judges the directory holding the database only. | PARTIAL |
| P‑5 | Support-bundle coordinates (`91618b8`, `internal/support/collector.go:245`) | The bundle shows `LATITUDE` and `LONGITUDE` to one decimal, in the configuration and wherever the journal printed them: `145f95e4`, `src/support.rs` (`coarse_coordinate`), gated by `the_stations_coordinates_leave_the_bundle_at_one_decimal`. | SHIPPED |
| P‑6 | Session cookie `Secure` decided from the connection (`6596663`) | `birdnet_web::request_scheme` (`af748031`): a request is HTTPS when it arrived on the station's own TLS listener, when a trusted proxy says so (`X-Forwarded-Proto` or `Forwarded: proto=`, leftmost hop), or when `BNB_PUBLIC_URL` is `https://` and the request's `Host` is that address; `Secure` follows. A real TLS handshake is in the test. **Cite corrected 2026‑10‑21 (9.1):** `6596663` is upstream's opt-in list of exact client IPs whose direct plain-HTTP sessions omit `Secure` (`security.plainhttpsessionclients`), a different mechanism that serves the same reported case. | SHIPPED |
| P‑7 | Settings that take effect without a restart | `SETTING_SPECS` in `src/helpers/settings_overlay.rs` has 67 entries: 47 are `Wiring::Bridged` (overlaid onto the config, so they take effect on the next restart, as the enum's own comment says) and 20 are `Wiring::OwnedBy` (read from the `settings` table by the named subsystem). Whether each of the 20 reads it per use or once at start was not checked. Counted with `grep -c 'Wiring::Bridged'` and `grep -c 'Wiring::OwnedBy'` over the text of `SETTING_SPECS`. | PARTIAL |
| P‑8 | Browser playback gain, high-pass and compressor (`frontend/src/lib/utils/audioNodes.ts:110-150`) | No Web Audio code in `crates/birdnet-web` (a grep of its `.js`, `.html` and `.rs` for `AudioContext`, `createGain` and `BiquadFilter` finds none). The player offers a pitch control only (N‑2). | GAP |
| P‑9 | Opt-in iframe embedding (`internal/api/middleware/security.go:51-132`) | `content_security_policy` sends `frame-ancestors 'self'` (`crates/birdnet-web/src/security.rs:75`) and `X-Frame-Options: SAMEORIGIN`, both fixed; no setting changes them. | GAP |
| P‑10 | MQTT client-certificate TLS | `.with_no_client_auth()` at `crates/birdnet-integrations/src/mqtt/publisher.rs:267` (the audit gave `src/integrations/mqtt/publisher.rs`, which does not exist). MQTT over TLS with a CA file and a user name and password is there; a client certificate is not. | GAP |
| P‑11 | Dynamic-threshold list, stats and reset routes (the test route is in G‑12) | Nothing under `crates/birdnet-web/src` names `DynamicThresholds` or the `dynamic_thresholds` table, so no route or page lists what has been learned, reports it, or clears it. It is the remainder G‑12 already records. | GAP |
| P‑12 | Per-species minimum interval between detections | No setting or table of that shape was found (a grep for `min_interval`, `minimum.interval` and `species_interval` in `crates` and `src` finds none). `species_thresholds` carries thresholds, by its name; its columns were not opened. | GAP |
| P‑13 | A dashboard custom image or webcam snapshot with a title (BirdNET-Pi `scripts/overview.php:19-27`) | Nothing on a dashboard page. `custom_image_dir` is a per-species override. See N‑6. | GAP |
| P‑14 | A hardware inventory in the support bundle | The bundle stages `uname -a` and `df -h` (`uname.txt`, `disk.txt`) beside the doctor report, which has a CPU check; no CPU model, memory or USB audio inventory member. The doctor report's contents were not compared item for item. | PARTIAL |
| P‑15 | "Test connection" buttons for MQTT, eBird, weather and ntfy | `routes/admin/notification_test.rs` tests the push channel and BirdWeather singly, and its "test everything" run (`test_all`) also runs an MQTT and an e‑mail probe (DD‑24; gate `mqtt_and_email_are_tested_and_a_dead_broker_fails_the_run`). **eBird and weather are tested nowhere.** ntfy is a native push URL, so the push test sends to it; that was not run. (Corrected 2026‑10‑21: this cell said no route tests MQTT; 9.1.) | PARTIAL |
| P‑16 | Temperature unit and 12/24-hour display | No setting for either was found (a grep of `crates` and `src` for `fahrenheit`, `temperature_unit`, `time_format` and `12h` finds only `routes/feeds.rs`, which was not opened). | GAP |
| P‑17 | Spectrogram style and dynamic-range options | One colour map, `viridis` (`routes/spectrogram/colormap.rs`), and a fixed 80 dB range (`mel.to_db(1.0, 80.0)`, `routes/spectrogram/render.rs:71`) — upstream's 80 / 100 / 120 dB options and four styles (`config.go:253-279`) have no setting here. `raw_spectrogram` only removes the overlay label. (Corrected 2026‑10‑21: this cell said no range constant existed; 9.1. Options for both were uncommitted in the shared tree that day.) | GAP |
| P‑18 | Opus and AAC clip export | Clips are WAV, converted to MP3, FLAC or Ogg by `ffmpeg` or `sox` (`audio/extraction/format.rs`). Deferred on purpose: `UNATTENDED_DEPLOYMENT_AUDIT.md` S-15 calls it optional and low priority. | GAP |
| P‑19 | AviCommons image provider (the licence is to check) | The providers are Wikipedia and Flickr (`species_images/`); AviCommons is named nowhere in the tree. Its licence was not checked. | GAP |
| P‑20 | Season carry-over: "new this season" for a resident (`3582fc8`) | `species_tracking.rs` computes `new_this_season` (G‑16, served at `routes/species.rs`). **It does not** (9.1, read): `classify` sets `new_this_season = first_season == on_date`, with no carry-over from before the season boundary, so a resident heard on the last day of one season and the first of the next is "first this season" on the second. Upstream's fix (`3582fc843`) treats a species seen within the seasonal window (7 days by default) before the boundary as carried over. API-only today (G‑16). | GAP |
| P‑21 | A weather JSON API | The weather providers shipped (G‑26), but no route serves the data: `openapi.json` has no weather path and a grep of the route tables finds none. | GAP |
| P‑22 | A page to upload, generate and download TLS certificates (ACME declined here: `crates/birdnet-web/src/tls.rs:17-23`) | `--tls-mode` has `off`, `self-signed` and `manual`, set by flag, environment or config; the generated CA is a file on disk. No page uploads, generates or downloads a certificate. | PARTIAL |

## Part 9 — full re-audit of 2026‑10‑21

Parts 1–8 were written without being able to open either upstream (Part 8 says so
of its own citations). Both are now cloned read-only, and this part is the first
in the document whose upstream citations were **opened and read**: tphakala/birdnet-go
at `30d163ded` (2026‑10‑20), Nachtzuster/BirdNET-Pi at `88985a3` (2026‑02‑28, unchanged
since Part 1: `git log -1`), and this project at `ea2d439`.

### 9.0 Method, and what was not checked

Every table cell carries one of four evidence tags, because "I looked" covers four
very different things:

* **ran** — I executed it and saw the output (a command, the real binary, the
  running fixture on `:8517`).
* **read** — I opened the cited source or document at the cited place.
* **grep** — presence or absence by search (`git grep … HEAD` for this project).
  An absence found this way is "not found by that pattern", not "does not exist".
* **not checked** — said so, so nobody has to guess.

Rules this part followed:

* **Ours is `ea2d439`, not the working tree.** Other agents had 46 uncommitted
  files in the shared tree while this was written (spectrogram look options, a
  per-species interval in `birdnet-db`, settings, `processor.rs`, `duplicate.rs`
  among them). Any claim about "ours" was re-run as `git grep … HEAD` or
  `git show HEAD:…` so it describes a commit, not a half-written edit. Where the
  tree already differs, the row says "in flight".
* **A parity claim needs an observed equivalent.** "Observed" means the route or
  setting exists at HEAD and, for pages, answers on the fixture (`curl` status
  and body on `:8517`). It does **not** mean the behaviour matches upstream's
  beyond what a row states.
* **No screenshots were reviewed.** This was a documentation task; no UI was
  changed. The fixture was read with `curl`, not looked at.

Not done, stated plainly:

* No upstream program was run; nothing here compares BirdNET-Pi's or
  birdnet-go's runtime output with ours beyond what Parts 5–8 and
  `docs/book/guides/parity.md` already say.
* birdnet-go's Svelte frontend (about 130 components) was listed and sampled,
  not read page by page. Its `config.schema.json` was reduced to 450 leaf keys
  and mapped by group (9.5), not key by key.
* Of the 84 birdnet-go commits since `f4f019a`, those marked (m) in 9.3 were read
  from their messages (`git show --format=%B -s`); the rest were triaged by subject
  line alone. No diff was read in full.
* Nothing was run on a Raspberry Pi, a real systemd host, or in a container.
* The older rows (Parts 1–7) were checked mechanically, not re-derived (9.2).

### 9.1 Part 8 re-verified against the upstream source

Each upstream citation of P‑1 … P‑22 was opened; each `Ours` cell was re-checked at
`ea2d439`. Rows not listed as changed were found as written.

| # | Upstream citation, re-read | Verdict on the row | Evidence |
|---|---|---|---|
| P‑1 | `engine.go:337` is `cooldownExpiredLocked` in the detection-escalation path; `:436` and `:475` are `tryAcquireCooldown`. The key is `cooldownStateKey{ruleID, scope}` (`:119-139`): species-scoped only for an event rule on a detection event that names a species, per metric instance for metric rules. Upstream's cooldown guards every action a rule has; ours guards the webhook arm only (`processor.rs`, `AlertAction::Webhook` calls `alert_cooldowns.should_fire`; the Log and Suppress arms do not, and have nothing to rate-limit) | **Right.** All nine commits the Part 8 cells name are ancestors of HEAD (`git merge-base --is-ancestor`, nine times, all true); migration 55 is `version: 55` at `migration.rs:2310` | read, ran |
| P‑2 | `config.go:348-349` is right (`Threshold`, `LocationAccuracy`, metres). `birdweather_client.go:258` is right for the fuzz: it is `func (b *BwClient) RandomizeLocation` (called at `:679`); it says nothing of the threshold, whose gate is `analysis/processor/actions_integrations.go:74`, in neither cited file | **Verdict stands** (0,0 fixed; no fuzz; no separate threshold). `git grep -i -e locationaccuracy -e location_accuracy -e birdweather_threshold HEAD` finds nothing. Ours posts the station's exact coordinates to a third party; that is a privacy difference, not only a missing option | read, grep |
| P‑3 | `range.go:49-60` right. Upstream's explorer is eight routes: `status`, `species/scores`, `species/count`, `species/list`, `species/csv`, `POST species/test` (a **what-if** at any latitude, longitude, threshold and date), `POST rebuild`, and `GET /range/heatmap` (Part 6) | **Stays SHIPPED for the explorer, but the cell overstated parity.** Ours is `GET /api/v2/species/occurrence` and `/species/occurrence` (`species.rs:20`), for the station's own place and a week. Not matched: the what-if, the CSV, the manual rebuild. Recorded as A‑14 | read, grep |
| P‑4 | `storage_persistence.go:54` is `Name()`; the check covers **four** key paths — config, database, models, clips (`:39`, the commit title) | Ours (`src/doctor/disk.rs::check_data_volume`, `birdnet_web::data_volume`) judges **one** directory: the one holding the database. A recordings directory or model directory elsewhere on the container layer is not examined. PARTIAL stands; its reason is now two-part (anonymous volume; one path of four) | read |
| P‑5 | `collector.go:245` lists `latitude`, `longitude` among the redacted keys. `91618b8` is "keep date-time prefixes intact when scrubbing coordinates", a fix *to* the scrubber, not its origin | Right as written; `coarse_coordinate` is at `src/support.rs:105`, its gate at `:531` | read |
| P‑6 | **The cited commit is a different mechanism.** `6596663` is `659666336` "keep session cookies Secure unless the client is listed": an **opt-in list of exact client IPs** (`security.plainhttpsessionclients`, hot-reloadable) whose direct plain-HTTP sessions omit `Secure`; with the list empty, behaviour is as before | Our `request_scheme` decides `Secure` from the connection instead, and so also serves upstream's scenario (a container calling `http://birdnet-go:8080` while browsers use an HTTPS proxy: its `Host` is not the public one, so it gets a non-`Secure` cookie). Residual: a plain-HTTP client that sends the *public* `Host` is still `Secure` and cannot be admitted, where upstream's list admits it. SHIPPED stands, as "different mechanism, same outcome for the reported case" | read |
| P‑7 | Not a citation row. Counts re-derived **from the committed file**: `git show HEAD:src/helpers/settings_overlay.rs`, text of `SETTING_SPECS`: 67 entries, 47 `Wiring::Bridged`, 20 `Wiring::OwnedBy`. (A whole-file `grep -c` gives 57 and 27: it counts the enum, tests and comments too. The row's sentence naming the method is the right one.) Upstream marks settings "Hot-reloadable" in its config reference; ours says "next restart" on the Settings page | PARTIAL stands; per-key hot-reload of the 20 `OwnedBy` keys still **not checked** (the species lists are refreshed live: `SpeciesListsProvider`) | ran, read |
| P‑8 | `audioNodes.ts:110-150` right (gain, high-pass biquad, optional compressor) | Right; `git grep -e AudioContext HEAD -- crates/birdnet-web` finds nothing | read, grep |
| P‑9 | `security.go:51-132` right; the setting is `WebServer.AllowEmbedding` (`config.go:1707`), described there as the Home Assistant case | Right. `git grep -i allow_embedding HEAD` finds nothing; `frame-ancestors 'self'` is at `security.rs:75` | read, grep |
| P‑10 | Upstream stores `ClientCert` / `ClientKey` paths (`config.go:764-765`) and a UI uploads them (`api/v2/integrations/mqtt_tls.go`, `GET`/`POST`/`DELETE /integrations/mqtt/tls/certificate`) | Right; `.with_no_client_auth()` at `publisher.rs:267` | read |
| P‑11 | Upstream: `GET`/`DELETE /dynamic-thresholds`, `/:species`, `/:species/events`, `/stats` (`routes_enumeration_test.go:31-32,76-79`) | Right; `git grep -e dynamic_thresholds -e DynamicThresholds HEAD -- crates/birdnet-web/src` finds nothing | read, grep |
| P‑12 | `SpeciesConfig{Threshold, Interval, Actions}` (`config.go:937-941`). **Upstream also has per-species `Actions`** (run a command on a detection of that species); the row names only the interval | At `ea2d439` ours has `species_thresholds(sci_name, confidence_threshold, created_at)` (`migration.rs:124`) and no interval. **In flight**: the shared tree exports a `set_species_interval` from `birdnet_db::sqlite`, uncommitted and not audited. Per-species actions are not in flight and are not covered by an alert rule's webhook, which fires per rule, not per species list entry | read, grep |
| P‑13 | `overview.php:19-27` right (`CUSTOM_IMAGE` and `CUSTOM_IMAGE_TITLE`, read from the file at `install_config.sh:283-284`, edited at `advanced.php:490-493`) | Right. **The three mislabels N‑6 owes a correction are still there**: `settings/render/system.rs:199` ("BirdNET-Pi: CUSTOM_IMAGE"), `cli.rs:974`, `routes/images.rs:164` | read, grep |
| P‑14 | Upstream added the inventory in `e05d97536` (USB devices and sound cards), `97ec7176f` (inference runtime, host environment) and masks USB serials (`91674440d`) | Right. Ours stages `uname -a` and `df -h` (`support.rs:372-373`); the doctor reports CPU cores | read |
| P‑15 | `integrations.go:105,117,121,125`: test routes for MQTT, BirdWeather, weather, eBird | **Wrong cell.** It says "No route tests MQTT". `test_all` in `notification_test.rs` runs an MQTT and an e‑mail probe (DD‑24), and `mqtt_and_email_are_tested_and_a_dead_broker_fails_the_run` (`:536`) holds it. What is true: those two are inside the one "test everything" run, not separate buttons; **eBird and weather are not tested anywhere**. Corrected in place | read |
| P‑16 | `config.go:186` (`Dashboard.TemperatureUnit`), `settings.ts:701` (`timeFormat`) | Right; weather is Celsius from the provider request (`weather.rs:389,408`) to the page (`WeatherSample.temp_c`, `overlays.rs:125`; `{temp:.0}°C`, `pages/health.rs:84`), and so is the CPU temperature (`routes/admin/system.rs:343`). `git grep -i -e temperature_unit -e fahrenheit HEAD` finds nothing | read, grep |
| P‑17 | Upstream: four styles (`default`, `scientific_dark`, `high_contrast_dark`, `scientific`) and a dynamic range of 80, 100 or 120 dB (`config.go:253-279`) | **Wrong cell.** It says "no dynamic-range constant … was found". It is `mel.to_db(1.0, 80.0)` at `routes/spectrogram/render.rs:71`: a fixed 80 dB, upstream's "high contrast" value. **In flight**: the shared tree adds `spectrogram_colormap` and `spectrogram_range_db` (uncommitted). Corrected in place | read, grep |
| P‑18 | `config.go:49`: `wav`, `flac`, `aac`, `opus`, `mp3`; ultrasonic captures only `wav` or `flac` | Right | read |
| P‑19 | `imageprovider/avicommons.go` serves an **embedded dataset** (`imageprovider/data/latest.json`) with a per-photo licence and author, images from `static.avicommons.org` at 320 px; it reads the licence code (`cc-by-nc` …) per entry | Right. The licence question is real: entries carry non-commercial codes | read |
| P‑20 | `3582fc843`: a species already seen within the window *before* the season boundary is "carried over", not new | **Verdict was too soft: PARTIAL → GAP.** Ours (`species_tracking.rs::classify`) sets `new_this_season = first_season == on_date`, where `first_season` is the first detection since `season_start`. A resident heard on 20 and 21 December is therefore "first this season" on the 21st, which is the defect upstream fixed. The tests cover a species last heard 66 days before the boundary (new, which upstream would also say: its carry-over window is `DefaultSeasonalTrackingWindowDays = 7`) and one already heard inside the season; none covers a species heard just before the boundary. Not run, read only; the effect is API-only today (G‑16). Corrected in place | read |
| P‑21 | Upstream: `GET /weather/latest`, `/daily/:date`, `/hourly/:date`, `/hourly/:date/:hour`, `/detection/:id`, `/sun/:date`, `/moon/:date` | Right; no weather path in `openapi.json` (98 operations) | read, ran |
| P‑22 | Upstream has `TLSModeAutoTLS` (Let's Encrypt, `config.go:1584`) as well as upload, generate, download and delete (`GET`/`POST`/`DELETE /tls/certificate`, `/download`, `/generate`) | Right; `tls.rs:17-23` is the "no ACME client here" paragraph | read |

Tally after the corrections, by script over the Part 8 table's last column: 4 SHIPPED, 6 PARTIAL, 12 GAP, with P‑2 counted once as PARTIAL (see the header of Part 8).

### 9.2 The older rows, checked mechanically

* **Paths.** A script took every backticked file name in this document (186) and
  looked each up in `git ls-files`. 172 resolve. The other 14 are upstream files
  (`install_config.sh`, `plotly_streamlit.py`, `config.schema.json`, …), generated
  names (`uname.txt`), or the one the audit got wrong (`src/integrations/mqtt/publisher.rs`,
  already corrected at P‑10). **No `file:line` points past the end of its file.**
* **Line numbers drift.** Of nine `file:line` citations into our source that I
  opened, six are exact (`flickr/mod.rs:60`, `provider.rs:12`, `livestream.rs:444`,
  `security.rs:75`, `publisher.rs:267`, `sources.rs:180`) and three had moved by
  46 to 300 lines: `SpeciesImages::from_settings` is at `species_images/mod.rs:235`
  (cited `:182`), `FLICKR_API_KEY` in the redaction list at `redact.rs:431` (cited
  `:129`), `max_files_per_species` at `disk/manager.rs:93` (cited `:47`). A line
  number into a living file is a promise nobody keeps; the rows were left as
  written because the symbol is named beside each. A gate that checks a cited
  symbol sits near its cited line would cost less than the next audit.
* **Tallies.** Counted by script on this file: Part 1 has six rows (5 SHIPPED, 1 GAP);
  Part 2 has 34 (24 SHIPPED, 5 GAP, 2 PARTIAL, 2 DECLINED, 1 IN PROGRESS) — the
  "34 findings" sentence is right; Part 8's tally sentence was right before the
  corrections above and is updated. Part 6's route count is right: 29 routes
  registered in `analytics/handler.go` (8 species, 10 time, 1 confidence, `sun`,
  `sources`, `range/heatmap`, 5 insights, `dashboard/kpis`, `database/overview`);
  every one of the 17 route paths named by the 16 rows its table marks SHIPPED
  exists in our route table (**route exists** — response fields were not re-compared).
* **Sizes (header table).** Re-counted: birdnet-go at `30d163ded` is 631 437 lines
  of Go in 3 438 tracked files (292 215 outside `_test.go`), 55 directories under
  `internal/`; this project at `ea2d439` is 293 541 lines in 654 `.rs` files
  (`git archive HEAD src crates`, tests included: 2.2× on total lines; against
  birdnet-go's 292 215 non-test lines it is 1.0×, but ours was not counted without
  tests, so that one is not like for like); BirdNET-Pi is
  24 961 lines of PHP, Python and shell, 18 145 without vendored Adminer and the
  file manager — both as the header says. The header's "about two to three times
  this project's" was the ratio of an older Go count to an older Rust count and
  is corrected there.
* **Rows whose verdict I did not re-derive** (G‑1 … G‑34 apart from those P‑rows
  touch, N‑1 … N‑5, Parts 5–7 rows): not re-checked beyond the three bullets above.
  Part 7 says it triaged by subject line; it was not redone.

### 9.3 birdnet-go `f4f019a` → `30d163ded`: 84 commits, classified

Part 8 listed the 22 findings but never classified the 65 commits between `f4f019a`
and `a1a77af`; the 19 since are new. Subject lines for all 84; messages read for the
ones marked (m). Dependency, licence and CI bumps (`go-audio-dsp`, `go-flac`,
`go-opus`, `go-m4a`, npm groups, `golangci-lint`, `mockery`, "update dependency
licenses" ×8, `scorecard`, docker testing channel) are 18 commits and are not listed.

| birdnet-go | What a user sees | Here | Verdict |
|---|---|---|---|
| `fc34a9706` (m), `83db2906a`, `e81763beb`, `61009d97e`, `8f09e5a9f` (m) | The privacy filter keeps speech out of the *whole saved clip* including pre-capture and post-capture, for every model and overlap; a second hit is needed at high overlap; the VAD default is 0.6; only voiced human sounds engage it for Perch labels (BirdNET's Human classes all still do) | Ours checks the clip's reach: `ClipReach` in `detection/privacy.rs:26`, `privacy_clip_reach` in the daemon config (`PIPE7`). Upstream's *deferral* (hold a detection until a speech detector has scanned the whole clip, 30 s limit) and the two-hit rule at overlap were **not compared**. No VAD here (G‑13) | PARITY (clip span, read); GAP (VAD, G‑13); not compared (the rest) |
| `3582fc843` | "New this season" no longer flags residents | See P‑20 | GAP |
| `2d05aa907`, `177dfd3d6` (m: cooldowns per species, pruned) | Alert cooldown per species; stale entries pruned | P‑1; ours prunes above `PRUNE_ABOVE = 512` (`alert_cooldown.rs`) | SHIPPED |
| `14bc82557` (m) | With the alert engine on, dead legacy filter fields are replaced by a notice | Not applicable: one notification path here | n/a |
| `8a652712e` (m) | A disabled push provider resolves no secrets and raises no error | Not compared (ours has no provider list to disable; destinations come from URLs) | not compared |
| `659666336` (m), `4f13856b3` (m) | Secure cookies; OAuth redirects keep the proxy base path | P‑6; no OAuth here (G‑20) | SHIPPED / GAP |
| `a43718b7e` | Warn on ephemeral container storage | P‑4 | PARTIAL |
| `ad311867c` (m) | The install no longer mounts an unused HLS tmpfs and unmounts stacked ones (one dump showed 219 layers) | Not applicable: the unit sets `PrivateTmp=yes` and mounts nothing per start (`install.sh:2371-2379`, `:2454`, read); the one host tmpfs unit is for manual runs (`:1927-1967`) | n/a |
| `91618b861`, `6d7a4c851`, `91674440d`, `aa841942a`, `a9dc76cab` (m), `ca8e77238`, `2700a9395` | Support dump: coordinates, USB serials, date-time prefixes, trusted-proxy lists, device tokens redacted or kept intact | Coordinates: P‑5 SHIPPED. USB serials: ours collects no USB inventory. `BIRDNET_TRUSTED_PROXIES` in the bundle: not checked | PARTIAL (not checked) |
| `e05d97536`, `97ec7176f` | Support dump gains USB/sound-card inventory, ONNX runtime and host environment | P‑14 | PARTIAL |
| `711c6c0a8` and the 14 spectrogram commits (`bba90b3f9`, `c06a56e88`, `7ac8612f4`, `f01f2d6a7`, `bf67ed470`, `66fcf7907`, `171e14908`, `0627ad0df`, `15cb04288`, `359fa4999`, `5bc07132c` …) | A native Go renderer replaces sox; 4xx for clips and widths it cannot serve; bat spectrograms at the clip's own rate; 503 for a recent clip still missing after a grace wait; stops retrying on a terminal reason | Ours renders spectrograms in Rust on demand (`routes/spectrogram/`); no sox. Whether a missing clip answers 404 or 503 here, and how a long clip's decode is bounded, were **not compared** | not applicable / not compared |
| `f7ea8b702`, `a75dc27c5`, `7783081fb`, `80e97144b`, `54319d72a`, `e895f9d40`, `2b97854cc` | Frontend: the spectrogram loader stops stalling and over-requesting; one probe before retrying a failed audio load; opt-in loader timing warnings; capture-buffer lag diagnostics | Svelte-specific; the player here is `clip-player.js` (not read against these cases) | not applicable |
| `1fa733477`, `bd52d32d0`, `91cdc237b`, `1c89bcc90`, `25db3dc48`, `709cd6bb5`, `30b47c71c` | Equalizer: a pass count of 0 means off; Q for low/high-pass; stored values kept on save; the response curve is computed by the backend and plotted | Ours has a per-source EQ with a response preview (`/admin/audio/sources/{id}/eq-preview`, G‑2) | PARITY (route exists; curves not compared) |
| `df21c8f4e` (m), `71fb75eae`, `ef4b6a77a` | Clip format kept in step with the stored name; ultrasonic capture is never lossy | Not applicable: the stored name decides here (`extractor.rs:397`); no ultrasonic path (G‑14) | n/a |
| `431181c9e` (m) | Disables KleidiAI kernels on ONNX Runtime before 1.27 (a memory leak on Apple M4/M5) | We ship ONNX Runtime 1.31.0 (`third_party/onnxruntime-1.31.0`), above the fixed version; we have a macOS package (`packaging/macos`) | not applicable (version above the fix) |
| `c80a5ba68` | Faster first/last detection per species | Not compared (ours reads a summary table built by migration 30) | not compared |
| `8fe0e3247` | One canonical UI path; bare `/settings` shows General | Server-rendered; `/admin` → 308 to `/station/…` | n/a |
| `a1a77affe` | Error reports from builds older than six months are stopped | We send none (G‑25) | n/a |
| `30d163ded`, `286426740`, `9eb4b96c6`, `cba1103ef`, `6126393ae`, `3a27e4072`, `bb3504d8d`, `a35aca29b`, `74a6bd74d` | Console logging no longer blocks; internal refactors and docs | Go-internal | n/a |
| `9927b4c68` | eBird client sent `/v2/v2/…`; fixed | Ours builds `{base}/v2/data/obs/…` once (`ebird.rs:316`, `DEFAULT_BASE_URL` has no path) | PARITY (built against upstream's docs, not live: G‑27 says so) |

Of the 84, the user-visible ones that need work here are **P‑20** (carry-over) and
the two privacy items not compared (deferral until the whole clip is scanned; the
two-hit rule at overlap). Nothing in the 84 is a new feature we lack that is not
already an `A‑` or `P‑` row.

### 9.4 Nachtzuster/BirdNET-Pi `88985a3`, enumerated

Source of the list: `homepage/views.php` (every `view=` branch), `scripts/install_services.sh`
(every unit and cron), `scripts/install_config.sh` (every key), `scripts/config.php` and
`advanced.php` (every form field), `scripts/utils/notifications.py` (every notification
rule), and the `scripts/*.sh` helpers. Each row names an equivalent that **exists at
`ea2d439`** (route registered, setting read) or says it does not. "Fixture" means the page
answered `200` on `:8517`.

#### 9.4.1 Pages and tools

| BirdNET-Pi | Ours | Verdict | Evidence |
|---|---|---|---|
| **Overview** (`overview.php`): newest detection, today's species with pictures, "new" and "rare" markers (`RARE_SPECIES_THRESHOLD`, days), `Combo-<date>.png` chart, the operator's one picture (`CUSTOM_IMAGE`) | Today (`/`): live signal, feed, top species, best recordings, Rare / First-today / Very-sure chips; the rare window is `rare_species_days` (web setting). Not carried: the one picture with a title (**N‑6**), the daily chart as a PNG file (ours are HTML/SVG; `git grep "Combo-" HEAD` finds nothing) | PARITY (page); GAP (N‑6) | ran, grep |
| **Today's Detections** and `Kiosk`: search, delete, "protect", change identification | Today list: `/pages/today-lock`, `-unlock`, `-delete`, `-relabel`, the search box; `/kiosk` | PARITY | ran |
| **Spectrogram** (live; `spectrogram_viewer.service`) | `/recordings?view=live`, `/ws/spectrogram` (in-process) | PARITY (route; not watched with audio) | ran |
| **Best Recordings** (`stats.php`): best clips per species | Today's "Best recordings", `/pages/best-detections`, species pages | PARITY | ran |
| **Species Stats** (Streamlit, port 8501) | `/species`, `/species/detail`, `/patterns`, `/pages/*` analytics, the DuckDB queries (Part 3). The Streamlit process itself is declined (Part 1) | PARITY (substance) | ran |
| **Daily Charts** (`history.php`) | `/reports?tab=history`, `/pages/history-*` | PARITY | ran |
| **Weekly Report** (`weekly_report.php`; cron Mon 01:00; Apprise copy) | `/reports`, `WEEKLY_REPORT_SCHEDULE`, `src/weekly_report.rs` | PARITY | ran |
| **Recordings** (`play.php`): by date or by species; sort by name, count, confidence or date; rename a file's species; delete; protect; "only excluded" filter; "load 40 more" | `/recordings`, `/recordings/{file}`, lock / unlock / delete (`/pages/recordings-*`); sort options not compared | PARITY (not compared in detail) | ran |
| **View Log** (gotty on `birdnet_log`) | `/admin/system/logs` and `/logs/page` | PARITY | ran |
| **Tools → Settings** (`config.php`): model, "use metadata model v2", `SF_THRESH`, site name, latitude/longitude, time zone, language, BirdWeather ID, Apprise settings, image provider, info site, colour scheme | `/station/settings`, `/station/capture`, `/station/alerts` (Settings tabs). Colour scheme is per browser here, not per station | PARITY (see 9.4.3 for the key-by-key result) | ran |
| **Tools → Advanced** (`advanced.php`): audio format, channels, recording card, RTSP streams (many), recording length, extraction length, overlap, confidence, sensitivity, privacy, disk policy, frequency shift, password, public URL, custom image, auto-update, rare threshold, raw spectrogram, three log levels, "run species count" | The same settings, spread over the Settings tabs and `/admin/audio`. Not carried: `FULL_DISK=keep` (**A‑4**), the custom image (**N‑6**), the update schedule (**A‑17**) | PARTIAL | ran, read |
| **System Info** (phpsysinfo iframe) | `/station`, `/admin/system`, `/api/v2/system/*` | PARITY (Part 1 DECLINED row) | ran |
| **System Controls**: Reboot, Update, Shutdown, Clear ALL data, Backup, Restore | Update: `POST /admin/update/apply` (manual). Clear: `/admin/system/clear-detections`, `clear-extracted`. Backup: `/admin/system/backup/full`, upload at `/station/data`. **Reboot and Shutdown: none** (**A‑16**) | PARTIAL | ran, read |
| **Services**: restart / stop / disable each of livestream, Icecast, web terminal, log viewer, analysis, stats, recording, chart viewer, spectrogram viewer | `POST /admin/system/service/restart` (the whole station), `…/audio/sources/{id}/restart`. There are no separate services to stop: one binary | PARTIAL, by design | ran |
| **File Manager**, **Database Maintenance** (Adminer) | Declined (Part 1) | DECLINED | read |
| **Web Terminal** (gotty `-w`, port 8888, `su --pty -l <user>` after a `Login:` prompt) | Declined outright (Part 1). **Security divergence worth stating:** when `CADDY_PWD` is empty, BirdNET-Pi's Caddyfile has *no* `basicauth` block at all (`install_services.sh:194-211`), so the terminal's own OS-password prompt is the only gate; with a password set it is behind Caddy's `birdnet` user *and* the OS login. birdnet-go ships a browser terminal too (`webserver.enableterminal`, off by default, labelled "security risk" at `config.go:1709`, `GET /terminal/ws`). We have none | DECLINED | read |
| **Custom / Excluded / Whitelist species lists**, **Species Management** | `/admin/species` (Allow, Exclusion, Always allowed), `/admin/species/manage` (N‑4). Entries are not read from BirdNET-Pi's `*_species_list.txt` files (`migration.md`) | PARITY | ran, read |
| `GET /api/v1/image/<sci>` (`api.php`) | `/species/image/{scientific_name}` — a different path and a different JSON shape; a script written against BirdNET-Pi's breaks | PARTIAL (not a goal) | read |
| `send_test_notification.py` | `/admin/notifications/test` | PARITY | ran |
| `print_diagnostic_info.sh`, `dump_logs.sh` | `birdnet-behavior --doctor`, `--support-bundle`, `/admin/doctor` | PARITY | ran |
| `species.py` (species list for a place, at a threshold) | `/species/occurrence` (adds the reason per species) | PARITY | ran |
| `extra_info.sh` (Pi undervoltage via `vcgencmd get_throttled`) | `routes/health.rs` (OP‑3: "the Pi's power mask") | PARITY | read |
| `HEARTBEAT_URL` | `heartbeat.rs`; the `heartbeat_url` setting | PARITY | read |
| `INFO_SITE` (AllAboutBirds or eBird; `ebird.php` code table) | `info_site` setting; eBird links by species code (`species_pages.rs:1022-1038`) | PARITY | read |
| Livestream (Icecast + `livestream.service`) | `/api/v2/stream`, HLS (G‑21). Icecast declined | PARITY | ran |
| `avahi-alias@` + `http.service` (publish `<host>.local`) | A service-discovery record only (`src/helpers/system.rs:358`, `_http._tcp`, "<name> on %h"). Its log line says "station discoverable as birdnet.local", which the file does not do: it creates no host name | PARTIAL (**A‑19**: the message is wrong) | read |
| Cron: `disk_check.sh` /5 min, `disk_species_clean.sh` 02:00, `cleanup.sh` /3 min, weekly report, **auto-update Sun 03:00** | In-process disk manager and per-species floor; `max_files_per_species`; stream retention; `WEEKLY_REPORT_SCHEDULE`. **No scheduled update** (A‑17) | PARTIAL | read |
| zram swap (`install_zram_service.sh`) | `installer/` zram unit (`installer/test/service-unit.sh` §7) | PARITY | read |

#### 9.4.2 The sixty keys of a default `birdnet.conf`, run through our `--doctor`

`install_config.sh:23-297` writes 60 `KEY=` lines (57 upper-case plus three `LogLevel_*`). I
extracted that block, filled in a latitude, longitude and `BIRDNET_USER`, and ran the real
binary: `target/debug/birdnet-behavior --doctor -c <that file>`. **Result (ran): 20 keys are
read, 40 are reported "is not a setting this station reads".** Only one of the 40
(`SITE_NAME`) gets a "did you mean" (`SITENAME`).

*Read (20):* `AUDIOFMT` (our default is `wav`; BirdNET-Pi's is `mp3`), `CADDY_PWD` (the sign-in
name is `admin`, not BirdNET-Pi's `birdnet`; `common.php:73`), `CONFIDENCE`, `DATABASE_LANG`
(read-time, N‑5), `EXTRACTION_LENGTH`, `FLICKR_API_KEY`, `FLICKR_FILTER_EMAIL`, `HEARTBEAT_URL`,
`IMAGE_PROVIDER`, `INFO_SITE`, `LATITUDE`, `LONGITUDE`, `MAX_FILES_SPECIES`, `MODEL` (read; whether
BirdNET-Pi's value, a `.tflite` file stem, selects our V2.4 ONNX classifier was **not checked**),
`OVERLAP`, `PRIVACY_THRESHOLD` (different scale — `parity.md`), `RECORDING_LENGTH` (range-checked,
sets nothing), `RECS_DIR`, `SENSITIVITY`, `SF_THRESH`.

*Not read (40) — what each becomes here:*

| BirdNET-Pi key | Here | What an operator sees if the file is copied across |
|---|---|---|
| `BIRDWEATHER_ID` | `BIRDWEATHER_TOKEN` (and the web setting) | **BirdWeather uploads stop, silently** apart from the doctor line |
| `PURGE_THRESHOLD` | `DISK_PURGE_THRESHOLD` (the `--help` text and the Settings hint call it BirdNET-Pi's `DISK_PURGE_THRESHOLD`; BirdNET-Pi's is `PURGE_THRESHOLD`, `render/system.rs:144`, `cli.rs:923`) | purges at the default 95 % whatever the old value was |
| `FULL_DISK` (`purge` / `keep`) | none (**A‑4**) | a `keep` station now deletes its oldest clips at the threshold; BirdNET-Pi stopped its services instead (`disk_check.sh:31-33`) |
| `REC_CARD` | `ALSA_CARD` | the microphone is not selected |
| `RTSP_STREAM`, `RTSP_STREAM_TO_LIVESTREAM` | `RTSP_URLS`, the audio sources table, `livestream_source` | streams are not carried over |
| `CHANNELS` | the per-source channel layout (`audio_sources`) | — |
| `SITE_NAME` | `SITENAME` / `STATION_NAME` | hinted |
| `DATA_MODEL_VERSION` | `METADATA_MODEL_PATH` (v2 range filter) | `parity.md` |
| `APPRISE_NOTIFY_EACH_DETECTION`, `_NEW_SPECIES`, `_NEW_SPECIES_EACH_DAY` | one `APPRISE_TRIGGER` (**A‑3**) | notifications default to "each detection"; a station that had two flags on gets one |
| `APPRISE_NOTIFICATION_TITLE`, `APPRISE_WEEKLY_REPORT`, `APPRISE_MINIMUM_SECONDS_BETWEEN_NOTIFICATIONS_PER_SPECIES` | `APPRISE_TITLE_TEMPLATE`, `WEEKLY_REPORT_SCHEDULE`, `APPRISE_COOLDOWN` | defaults apply |
| `APPRISE_ONLY_NOTIFY_SPECIES_NAMES` (**an exclusion list**, `notifications.py`) and `_2` (an inclusion list) | `APPRISE_WATCHLIST_EXCLUDE` and `APPRISE_WATCHLIST` | the names are swapped on purpose; a naive rename would invert both |
| `BIRDNETPI_URL` | `BNB_PUBLIC_URL` (environment) | links in notifications fall back to `http://<hostname>.local` |
| `FREQSHIFT_TOOL`, `_HI`, `_LO`, `_PITCH`, `_RECONNECT_DELAY` | none: a per-listener control (N‑2) | — |
| `RAW_SPECTROGRAM`, `RARE_SPECIES_THRESHOLD` | web settings `raw_spectrogram`, `rare_species_days` | web-only; not read from the file |
| `COLOR_SCHEME` | a per-browser theme | — |
| `CUSTOM_IMAGE`, `CUSTOM_IMAGE_TITLE` | none (N‑6) | — |
| `AUTOMATIC_UPDATE`, `SILENCE_UPDATE_INDICATOR` | none (A‑17); the badge is declined | — |
| `ICE_PWD` | none (Icecast declined) | — |
| `LogLevel_*` ×3 | `LOG_LEVEL`, `LOG_MODULES` | — |
| `BIRDNET_USER`, `PROCESSED`, `EXTRACTED`, `IDFILE`, `LAST_RUN`, `THIS_RUN` | none; internal to BirdNET-Pi | — |

**`docs/book/guides/migration.md:67` is wrong about this.** It says "BirdNET-Pi config keys
(`ALSA_CARD`, `LATITUDE`/`LONGITUDE`, `OVERLAP`, `SF_THRESH`, `BIRDWEATHER_TOKEN`, …) are also
understood when reading a `birdnet.conf`; see the mapping table in Configuration." Two of the
five names are ours, not BirdNET-Pi's (BirdNET-Pi writes `REC_CARD` and `BIRDWEATHER_ID`), the
Configuration page has no BirdNET-Pi mapping table, and the guide says nothing about the
sign-in name, the purge key, `FULL_DISK`, or the notification flags. That is **A‑1**.

#### 9.4.3 Notifications (Apprise)

| BirdNET-Pi rule (`utils/notifications.py`) | Ours | Verdict |
|---|---|---|
| Message variables `$sciname $comname $confidencepct $confidence $listenurl $friendlyurl $date $time $week $latitude $longitude $cutoff $sens $flickrimage $image $overlap $reason` | All seventeen are substituted (`notification.rs:210-229`). `$confidence` prints two decimals here, the raw float there | PARITY (read) |
| Three independent triggers: each detection; first of the day; seen ≤ 5 times in 7 days | One trigger mode (`TriggerMode`, `notification.rs:234`). The reasons are worded as BirdNET-Pi's (`detection`, `first time today`, `only seen N times in last 7d`) and N includes the current detection in both | PARTIAL (**A‑3**) |
| Per-species minimum seconds between notifications | `APPRISE_COOLDOWN`, default 300 s, per species; plus a per-destination rate cap | PARITY |
| Include and exclude species lists, matched on common names with spaces removed | Matched on common or scientific name | PARITY |
| Image in the message (`attach=`) | `$image` is a URL in the text; whether each native sender attaches the picture was not checked | PARTIAL (not checked) |
| Apprise config pasted in the UI (`apprise_input`) | `notify_urls` field and `apprise_config` path | PARITY |
| Apprise itself | Native Discord, Slack, Telegram, ntfy, Gotify, Pushover and JSON webhook; the `apprise` CLI only for other schemes; an Apprise API server; SMTP e-mail; MQTT | PARITY (wider) |

#### 9.4.4 What BirdNET-Pi gives a user that we deliberately do not (and where it is written down)

Adminer, the file manager, the web terminal, phpsysinfo, Streamlit, Icecast, `git` self-update
and its badge: Part 1's DECLINED table. Two that table does not state: the `Combo-<date>.png` /
`Charts/` static chart images (nothing here writes a PNG chart), and Caddy's browsable
`/By_Date/…` and `/Charts/` trees (`install_services.sh:161-168`), which made any clip
reachable by a predictable URL with no sign-in. Ours serves clips at `/recordings/{file}` and
signed `/r/{token}` share links. A bookmark or an external link to a `By_Date` URL does not
survive the move.

### 9.5 tphakala/birdnet-go `30d163ded`, enumerated

Sources: `config.schema.json` (450 leaf keys, reduced by a script, grouped below), the
golden route list in `internal/api/v2/routes_enumeration_test.go` (223 operations, plus
the 5 insights routes and `dashboard/kpis` it omits and `internal/api/v2/analytics/handler.go`
registers), `frontend/src/lib/desktop/{views,features}`, `cmd/`, `README.md`, `doc/wiki/`.
Ours: `openapi.json` (98 documented operations) and the route table at `ea2d439`.

#### 9.5.1 The settings tree, by group (450 leaves)

| birdnet-go group (leaves) | Ours | Verdict |
|---|---|---|
| `realtime.audio.sources` / `rtsp.streams` (name, device/url, type, transport, channel mode, media mode, gain, per-stream EQ and quiet hours, models) | `/admin/audio` and the `audio_sources` table: kind, transport, channel layout (mono / left / right / stereo), gain, EQ chain, quiet window, `MODEL_ROUTES` (G‑2, G‑4, G‑7, G‑8, G‑10) | PARITY |
| `realtime.rtsp.ffmpegParameters` (arbitrary ffmpeg arguments) | none (**A‑12**) | GAP |
| `realtime.audio.ffmpegpath`, `soxpath` | `ffmpeg` and `sox` are looked up on `PATH` (`capture/process.rs:611`); no setting | GAP (minor, **A‑12**) |
| `realtime.audio.export.*`: type, bitrate, length, pre-capture, gain, retention (policy, max age, max usage, min clips, keep spectrograms, interval), normalisation (EBU R128) | Type (wav, mp3, flac, ogg: P‑18 for Opus and AAC), length, pre-capture, normalisation (G‑5), retention by age (`CLIP_RETENTION_DAYS`), by disk use (`DISK_PURGE_THRESHOLD`) and a per-species floor. **No bitrate setting** (ffmpeg's default), no clip gain | PARTIAL (**A‑21**) |
| `realtime.audio.soundlevel.*`, `equalizer.*`, `quietHours.*`, `watchdog.*` | G‑1, G‑2, G‑4, G‑9 | PARITY (SHIPPED rows) |
| `birdnet.*` (sensitivity, threshold, overlap, location, **threads**, locale, range filter, model/label paths, XNNPACK, ONNX Runtime path, OpenVINO, backend, region) | Present except **threads**: the classifier session is built with the model's default of 2 intra-op threads (`ModelConfig::default`, `model.rs:78`; `daemon/config.rs:243` says "comes from the model's defaults") and no setting, flag or key changes it (`git grep -i thread HEAD -- src/cli.rs known_keys.rs` finds none). Backends: G‑11 | PARTIAL (**A‑7**) |
| `birdnetv3.*`, `perch.*`, `bsg.*`, `bat.*`, `models.*` | G‑10 (Stages 1–5), G‑13, G‑14. Installing a model from the UI is **declined on purpose**: `api_write.rs:1353-1365` says why (a download needs a foreground progress channel) and points at `--install-model` | PARTIAL / DECLINED |
| `realtime.species` (include, exclude, **config**: per species threshold, **interval**, **actions**) | Lists and per-species thresholds yes (`species_thresholds`); interval in flight; **actions** none | PARTIAL (P‑12) |
| `realtime.dynamicthreshold.*` | G‑12 (learning); P‑11 (no view or reset) | PARTIAL |
| `realtime.falsepositivefilter`, `bat.falsepositivefilter` | `CONFIRMATION_LEVEL` (corroboration) | PARITY |
| `realtime.privacyfilter.*` | Rank and threshold rules, clip reach; no VAD (G‑13) | PARTIAL |
| `realtime.dogbarkfilter.*` | G‑17 | PARITY |
| `realtime.daylightfilter.*` | `--night-filter`, `--night-margin-mins`, `--night-extra-nocturnal` — the same family (species that cannot be calling at this hour); the rule's direction and species list were not compared | PARITY (not compared) |
| `realtime.extendedcapture.*` | G‑6 | GAP |
| `realtime.interval` (minimum seconds between detections of a species; **15 by default**) | `DUPLICATE_INTERVAL_SECS`, **off by default** (`parity.md` states the difference) | PARITY (default differs) |
| `realtime.speciestracking.*` | G‑16; P‑20 | PARTIAL |
| `realtime.birdweather.*` (threshold, location accuracy, retry) | token only (P‑2) | PARTIAL |
| `realtime.ebird.*` | G‑27 | PARITY |
| `realtime.weather.*` | G‑26 (four providers; OpenWeather declined). No weather API (P‑21), no per-detection weather (**A‑6**), no test button (A‑15), no °F (P‑16) | PARTIAL |
| `realtime.mqtt.*` (retain, retry, TLS, client cert, Home Assistant discovery) | All but the client certificate (P‑10); per-source HA topics are Part 5 `aa86b38` | PARTIAL |
| `realtime.monitoring.*` (CPU, memory, disk thresholds) | Station Health conditions and metric alert rules (G‑29) | PARITY |
| `realtime.log` (a text file of detections for OBS) | none (**A‑8**) | GAP |
| `realtime.logdeduplication`, `logging.*` (levels, per-module levels, console, **rotating file output**) | `LOG_LEVEL`, `LOG_MODULES`, journald or stdout, the capped `errors.jsonl` (G‑25). No rotating log file | PARTIAL (minor) |
| `realtime.telemetry` (Prometheus listener), `diagnostics.profiling` | `GET /api/v2/metrics`, `docs/grafana-dashboard.json`; CPU profiling (G‑24) | PARITY |
| `realtime.dashboard.*` | thumbnails and provider fallback: PARITY. Layout order/visibility: G‑22. Banner with picture, title, description, **map**, weather; **video embed**: GAP (N‑6, **A‑11**). Temperature unit, colour schemes, logo style: P‑16, G‑22. Spectrogram style and range: P‑17 (in flight). Default audio gain: P‑8. UI locale: **A‑5**. Summary limit: no setting | PARTIAL |
| `notification.*` (providers with per-provider filters on type / priority / component, circuit breaker, health check, rate limit, templates; script and webhook endpoints with auth) | Native senders, circuit breaker and rate cap (`apprise.rs`), templates, `*_file` secrets (G‑28). Per-destination filters on type or priority and a script provider: none found (`git grep` on `filter`, `priority` in `dispatch/`) | PARTIAL |
| `alerting.history_retention_days`, alert rules | `/admin/rules`, `/admin/metric-rules`, `/admin/notifications/prune`; P‑1 | PARITY |
| `security.*`: basic auth, `oauthProviders` (Google, GitHub, Microsoft, generic OIDC with PKCE), **a built-in OAuth2 server** for API clients (`clientid`, `authcodeexp`, `accesstokenexp`), `allowsubnetbypass`, `privatemode`, `publicaccess.liveaudio`, trusted proxies, `plainhttpsessionclients`, TLS modes (off, manual, self-signed, autotls) | Accounts with session revocation (ours alone, 9.6); `--private-mode`; `--public-access`; trusted proxies (config only); TLS off / self-signed / manual; one static bearer token for the write API. **No OIDC (G‑20), no subnet bypass (A‑18), no ACME (declined), no OAuth2 server** | PARTIAL |
| `webserver.*`: port, base path, `allowembedding`, livestream bitrate / rate / segment, `enableterminal` | `--listen`, base path (G‑19), HLS fixed (G‑21), no embedding switch (P‑9), terminal declined | PARTIAL |
| `backup.*`: targets (local, FTP, SFTP, rsync, Google Drive), schedules, retention, encryption | G‑30: local, SFTP, S3-compatible and rsync, always encrypted off-site; upstream's FTP and Google Drive declined | PARITY |
| `output.sqlite`, `output.mysql` | SQLite and DuckDB; MySQL declined (G‑31) | DECLINED |
| `import.*`, `lowmemory.mode`, `sentry.*`, `taxonomySynonyms`, `main.timeas24h` | BirdNET-Pi import; memory capability gating (G‑33); Sentry declined (G‑25); synonyms (G‑15); 12/24-hour (P‑16) | mixed — see those rows |

Not one of the 450 was mapped leaf by leaf; the groups above are the unit.

#### 9.5.2 The REST API v2, by family

Upstream registers **229** operations (223 in the golden list + 6 registered elsewhere). This
project documents **98** in `openapi.json`, read-only by default with a bearer-gated write
subset (`api_write.rs`). Counted by script by first path segment; upstream's count first.

| Family (upstream count) | Ours | Verdict |
|---|---|---|
| `system` (42): info, resources, disks, processes, temperature, network interfaces, external media, jobs, models, inference, events, diagnostics run / report / errors, database stats / overview / backup jobs / **migration** / legacy cleanup | `/system/disk`, `/capture`, `/jobs`, `/network`, `/processes`, `/database/overview`; backup is a page and `POST`, not a job API. External media listing: none | PARTIAL |
| `analytics` (21), `insights` (5), `dashboard` (1) | Part 6: every route but `range/heatmap` | PARITY (route exists) |
| `streams` (14): sources, status, health + stream, audio level, analyze channels, test, quiet-hours status, HLS | HLS (5 routes, same paths); `/admin/audio/sources/{id}/probe`, `--channel-report` for the rest | PARTIAL |
| `detections` (14): list, recent, one, delete, lock, review, comments, batch delete / lock / review / resolve, ignore / ignored, time-of-day, SSE stream | list, recent, detail, daily, delete / lock / review / comments / batch (bearer), WebSocket stream, export (CSV, Raven, BirdDB, eBird) | PARITY (shape differs) |
| `alerts` (14) | The same functions as HTML routes plus export / import; no REST | PARTIAL (minor) |
| `notifications` (10): list, read, acknowledge, unread count, SSE stream, ntfy server check | `/notifications` page and `/admin/notifications/*`; no unread-count API | PARTIAL (minor) |
| `integrations` (10): BirdWeather, eBird, MQTT, weather tests; HA discovery trigger; MQTT certificate | P‑15, P‑10; discovery is published on connect | PARTIAL |
| `settings` (9) | `GET`/`PUT /api/v2/settings` (bearer), schema drift gate (G‑34) | PARITY |
| `models` (9): catalogue, install with progress, installed, delete, reinstall, regions + map | `GET /models/catalog` only (install declined, above) | PARTIAL / DECLINED |
| `range` (8) | P‑3, A‑14 | PARTIAL |
| `weather` (7) | none (P‑21) | GAP |
| `media` (7), `spectrogram` (4), `audio` (4) | `/species/image/*`, `/spectrogram/{file}`, `/recordings/{file}`, `/stream` | PARITY |
| `import` (7) | `/admin/migrate/*` (upload, validate, run, progress, batches) | PARITY |
| `control` (7): restart, restart-source, restart-container, reload, rebuild-filter | `restart`, `restart-source` (bearer). No hot reload (P‑7), no rebuild | PARTIAL |
| `dynamic-thresholds` (6) | none (P‑11) | GAP |
| `tls` (5) | none (P‑22) | PARTIAL |
| `species` (5), `taxonomy` (3), `search` (1) | `/species/*`, taxonomy browsing (G‑15), `/search`, `/species/search` | PARITY |
| `support` (3), `health` (2), `ping`, `sse`, `soundlevels/stream` | `--support-bundle` and `/admin/support-bundle` (**`503` on the screenshot fixture**: "the station's diagnostics are not wired into this process"; the real binary was not driven), `/api/v2/health`, `/health/conditions`, `GET /api/v2/soundlevel` | PARITY (support route: see note) |
| `auth` (4), `app` (2) | login / logout pages, onboarding | PARITY |
| `filesystem` (1) | none (**A‑13**) | GAP (minor) |
| `terminal` (1) | declined | DECLINED |

#### 9.5.3 UI features

Upstream's pages are: dashboard, live stream, notifications centre, species, analytics (summary,
activity, trends, biodiversity, nocturnal, review; *weather* and *soundscape* are
"coming soon" shells — `WeatherPage.svelte`, `SoundscapePage.svelte` render
`AnalyticsComingSoon`, so they are **not** gaps), search, detections list, detection detail
(review, verify, lock, comment), About, Help, report-a-bug, System (health, inference,
database), Settings (ten pages), setup wizard, import / export, terminal.

| Upstream | Ours | Verdict | Evidence |
|---|---|---|---|
| Dashboard with edit mode, "Currently hearing" card, new-species highlights, daily summary | Today (`/`) with "Right now", live signal, feed, customise (`/admin/today-layout/…`) | PARITY | ran |
| Live stream page | `/recordings?view=live` | PARITY | ran |
| Notifications centre | `/notifications`, `/pages/notif-*` | PARITY | ran |
| Species pages, taxonomy | `/species`, `/species/detail`, life list, photos | PARITY | ran |
| Analytics (the eleven chart families of `registry/charts.ts`: seasonal heat map, ridgeline, succession, dawn-chorus onset, nocturnal clock, time-of-day, trend, year over year, diversity, accumulation, phenology, confidence) | `/pages/*`, `/patterns`, `/reports`, the DuckDB queries | PARITY (more) | ran |
| Review queue, verify / false positive, lock, comment | `/detection-reviews`, `/pages/detection-review*`, `/api/v2/detections/comments` (G‑23) | PARITY | ran |
| Per-detection **weather badge** and moon badge | none on the detection page (`git grep -c -i weather HEAD -- …/detection_detail.rs` is 0); Today shows the current weather and a temperature line | GAP (**A‑6**) | grep |
| **Audible bats** button, bat classifiers | G‑14 | GAP | read |
| System health and inference pages; database overview | `/station` (health), `/admin/doctor`, `/api/v2/system/database/overview` | PARITY | ran |
| One-click **support bundle** | `--support-bundle`; the web route exists but did not answer on the fixture | PARITY (CLI) | ran, read |
| Setup wizard | `/onboarding` | PARITY | ran |
| **UI in 15 languages** (cs, da, de, es, fi, fr, hu, it, lv, nb, nl, pl, pt, sk, sv; `frontend/static/messages/*.json`) | English only; species names in the station's language (N‑5). Part 5's "`98b1c62` … not applicable (the UI is English)" and Part 7's `9642f74` row treated this as out of scope; it is a gap against a project whose README lists it | GAP (**A‑5**) | read |
| PWA install | `manifest.webmanifest`, `sw.js` | PARITY | ran |
| Colour scheme picker (5 + custom), logo style | a light / dark / observatory theme per browser | GAP (cosmetic; G‑22) | read |
| "Browser push notifications" (README) | **Not found in the source**: `git grep -i -e webpush -e vapid` over `internal/` and `frontend/src` finds nothing; the notification centre is fed by SSE. Ours has the same SSE-style centre | n/a (claim without code) | grep |
| "File analysis" (`doc/wiki/file-analysis.md`: `birdnet-go file x.wav --threshold`) | **Not in the source either**: `cmd/` has no `file` or `directory` command and `git grep -i -e AnalyzeFile -e FileAnalysis` finds nothing; the wiki page is stale. Ours: `--watch-dir` with `--process-existing` | n/a (documented, absent) | grep |

#### 9.5.4 CLI

| birdnet-go | Ours | Verdict |
|---|---|---|
| `serve` (default) | the binary | PARITY |
| `support collect` | `--support-bundle` | PARITY |
| `benchmark` (inference timing for a machine) | none; `birdnet_inference_duration_seconds` is exported, nothing recommends an overlap (Part 5 `dee43f5`) | GAP (**A‑9**) |
| `rangefilter print` (species list for a place and week) | `/species/occurrence` | PARITY |
| `notify` (send a test notification) | `/admin/notifications/test` | PARITY |
| `import-stage`, `perch-benchmark`, `gen-schema`, `license`, `authors`, `wiki-export` | build and maintenance tools | n/a |
| (database doctor, `tools/db-doctor/db-doctor.py`) | `--check-db`, `--fix`, `/admin/doctor` | PARITY |

#### 9.5.5 Defaults that differ, for anyone moving *from* birdnet-go

From `internal/conf/defaults.go` against this project's defaults (`parity.md` has the
three-way table and was not repeated): `birdnet.threshold` is 0.8 in code (0.7 on first run) and
ours is 0.70; `birdnet.sensitivity` is 1.0 and is a slope, ours is 1.25 and is BirdNET-Pi's sigmoid
setting; **`realtime.interval` is 15 s** (ours off); **`dynamicthreshold.enabled` is true**
(ours: `BIRDNET_DYNAMIC_THRESHOLD`, off unless set); **`privacyfilter.enabled` is true** (ours:
rank rule on); `export.retention.policy` is `usage` with `minclips` 10 and `keepspectrograms` true;
`speciestracking.enabled` is true with a 7-day window. A birdnet-go station copied across counts
more detections per species per hour than it did, unless `DUPLICATE_INTERVAL_SECS=15` is set.

### 9.6 What this project has that neither reference has

Part 3 lists twelve rows. Re-checked against both clones, with the grep named, only some of
them survive a strict reading of "neither". Searched in `internal/`, `frontend/src/`, `cmd/`
(birdnet-go) and `scripts/`, `homepage/` (BirdNET-Pi), tests, vendored code and translation
files excluded; **0 matches in each** for the patterns given.

| Capability (ours) | Ours | Pattern searched in both | Standing |
|---|---|---|---|
| Product-analytics primitives on detections: sessions, funnels, retention, sequence matching, next / previous species | `/analytics/sessions`, `/funnel`, `/retention`, `/sequence-count`, `/next-species`, `/previous-species`; `crates/birdnet-behavioral` | `retention curve`, `cohort`, `funnel analysis`, `sessioniz`, `markov`, `next.species` | **Neither has it** |
| Quarantine of implausible detections for review | `/quarantine` | `quarantine` | **Neither** |
| Signed, expiring, single-detection share links | `/r/{token}` | `share_token`, `ShareLink`, `share link` | **Neither** |
| RSS and iCal feeds of rare detections | `/feeds/rare.rss`, `/feeds/rare.ics`, `/feeds/today.rss` | `text/calendar`, `VCALENDAR`, `rss+xml`, `<rss` | **Neither** |
| A queryable log of operator actions | `audit_log` (`migration.rs:326`; 69 `audit(` call sites in `birdnet-web`), `/admin/audit` | `audit` | **Neither**: birdnet-go has a "security" log module and no table or page |
| Several accounts, each with revocable sessions | `/admin/accounts` | `CreateUser`, `multiple user accounts` | **Neither**: birdnet-go's own comment says so (`api/auth/adapter.go:154`, "does not support multiple user accounts or authorization levels"); BirdNET-Pi has one `birdnet` user |
| Command palette | `/pages/cmdk` (⌘K, `/`) | `command palette`, `cmdk` | **Neither** |
| Year in review | `routes/pages/year_in_review.rs` | `year.in.review` | **Neither** |
| A measured channel report and a preflight that checks the machine (`--channel-report`, `--doctor`, `--migration-report`) | `src/doctor/`, `src/channel_report.rs` | — | Neither has *this*; both have a support collector (`support collect`, `dump_logs.sh`) and birdnet-go a database doctor and a system diagnostics run |
| Import reconciliation: a per-species before-and-after comparison, and removing one imported batch | `/admin/migrate/*`, `crates/birdnet-migrate` | `rollback` appears in birdnet-go for the *database* migration only | Neither, **by grep only** |

Rows of Part 3 that fail "neither":

* **Encrypted off-site backup.** birdnet-go has `backup.encryption` (schema) and its own
  targets; G‑30 already says so. What is ours is the argon2 envelope and in-tree SigV4, which
  are implementation choices an operator does not see.
* **Phenology.** Part 3 itself says birdnet-go has `analytics/species/phenology`.
* **Kiosk.** BirdNET-Pi has `view=Kiosk` (`views.php`); not claimed in Part 3, noted here so
  it is not added.
* **Self-update.** BirdNET-Pi has it (by `git`); birdnet-go does not (`internal/update` is
  empty of Go files), so "neither" fails on one side.
* **First-class BirdNET-Pi migration.** birdnet-go has an importer for BirdNET-Pi
  (`/import/birdnet-pi`, `/import/validate`, progress and cancel); the reconciliation
  report in the table above is the part that is ours.

### 9.7 Newly found, and the verdicts changed

Severity: **High** = changes what a migrating user sees or loses; **Medium** = a missing
capability with a workaround, or a decision nobody has recorded; **Low** = niche or cosmetic.
"Value" is an ordering by who is affected and how often, a judgement with no usage data behind
it (**CONJECTURED**); the evidence column is what was actually checked.

| # | Finding | Evidence | Severity | Verdict |
|---|---|---|---|---|
| A‑1 | **BirdNET-Pi's `birdnet.conf` does not carry across.** 40 of its 60 default keys are unread; renames that matter (`BIRDWEATHER_ID`→`BIRDWEATHER_TOKEN`, `PURGE_THRESHOLD`→`DISK_PURGE_THRESHOLD`, `REC_CARD`→`ALSA_CARD`, `RTSP_STREAM`, the three `APPRISE_NOTIFY_*`, the swapped include / exclude lists) get no hint (only `SITE_NAME` does). `migration.md:67` names keys BirdNET-Pi does not use and a mapping table that does not exist. The sign-in name changes from `birdnet` to `admin` and is not in the migration guide | ran (the real binary, 9.4.2), read | High | GAP |
| A‑2 | The importer copies rows, not clips: an imported detection has no audio (`migration.md`, "Does not carry over: the audio"). `git grep -e By_Date -e Extracted HEAD -- crates/birdnet-migrate` finds nothing | read, grep | High | GAP |
| A‑3 | BirdNET-Pi's three notification triggers are independent flags; ours is one mode. A station with "each detection" and "new species" both on gets one | read | Medium | PARTIAL |
| A‑4 | `FULL_DISK=keep` (stop recording at the threshold instead of deleting) has no equivalent; `DISK_PURGE_THRESHOLD` always purges | read | Medium | GAP |
| A‑5 | No UI translation. birdnet-go ships 15 non-English locales (`frontend/static/messages/*.json`) and lists them first among its features. Part 5 and Part 7 filed this as "not applicable (the UI is English)" | read | Medium | GAP — those two rows corrected |
| A‑6 | No weather on a detection: upstream shows a weather badge per detection and serves `GET /weather/detection/:id`; ours shows current weather on Today and has a timeline overlay, and `detection_detail.rs` does not mention weather | grep | Low | GAP |
| A‑7 | The classifier runs on 2 intra-op threads and nothing can change that (`model.rs:78`, `:1224`; `birdnet.threads` upstream, `config.go:1323`) | read, grep | Medium | GAP |
| A‑8 | No text file of detections for an OBS "chat log" source (`realtime.log`, `defaults.go:224-225`, `doc/wiki/guide.md:235-238`) | grep | Low | GAP |
| A‑9 | No `benchmark` and no advice on the overlap a machine can sustain (`cmd/benchmark`; Part 5 `dee43f5`) | read | Low | GAP |
| A‑10 | No import from a birdnet-go database; `crates/birdnet-migrate` reads BirdNET-Pi only | grep | Medium | GAP |
| A‑11 | Dashboard video embed and a banner with a location map (`VideoEmbedCard.svelte`, `BannerLocationMap.svelte`); with N‑6 / P‑13 | read | Low | GAP |
| A‑12 | No arbitrary ffmpeg parameters per stream, no configurable `ffmpeg` / `sox` path (`config.go:695`, `ffmpegpath`, `soxpath`) | grep | Low | GAP |
| A‑13 | No folder picker or external-media listing (`GET /filesystem/browse`, `/system/external-media`) | grep | Low | GAP |
| A‑14 | The range-filter explorer has no what-if (latitude, longitude, threshold, date), no CSV, no manual rebuild (P‑3's remainder) | read | Low | GAP |
| A‑15 | Test buttons: eBird and weather have none; MQTT and e‑mail are tested only inside "test everything" (P‑15's remainder) | read | Low | PARTIAL |
| A‑16 | No Reboot / Shutdown. The unit forbids it (`SystemCallFilter=~… @reboot`, `NoNewPrivileges=yes`, `install.sh:2480-2493`), which is a reasonable decision that is written down nowhere a user would look | read | Medium | DECLINED by the unit — **undocumented** |
| A‑17 | No scheduled update (`AUTOMATIC_UPDATE`, cron Sun 03:00 upstream). Update is a button (`/admin/update/apply`). No document records whether this is declined | grep | Medium | GAP (decide) |
| A‑18 | No LAN "skip the sign-in" switch (`security.allowsubnetbypass`). The installer generates a password, which is the opposite choice | grep | Low | GAP (decide; a security trade) |
| A‑19 | `maybe_install_avahi_service` logs "station discoverable as birdnet.local"; the file it writes advertises an `_http._tcp` service named "<site> on %h" and creates no host name. BirdNET-Pi's `avahi-alias` does publish one | read | Low | defect |
| A‑20 | Part 3's "neither upstream has" is overstated for encrypted backup, phenology, and (by omission) is right for eight others (9.6) | grep | Low | doc |
| A‑21 | No clip bitrate and no clip gain setting (`export.bitrate`, `export.gain`); ffmpeg's defaults decide | grep | Low | GAP |

Verdicts changed in place in Part 8 by this part: **P‑20** PARTIAL → GAP; **P‑15** and **P‑17**
cells corrected (wrong statements); **P‑6** cell now names the commit's real mechanism;
**P‑3** and **P‑4** cells say what was not matched.

### 9.8 The prioritised backlog

Order is value to the user first (CONJECTURED, as above), then size. **S** = a day or less
with a gate; **M** = a few days; **L** = a programme. "Upstream" is where to read how it is done.

| Rank | ID | Upstream cite | Ours (state at `ea2d439`) | Size | Who benefits |
|---|---|---|---|---|---|
| 1 | A‑1 | BirdNET-Pi `install_config.sh:23-297`; `common.php:73` | 40/60 keys unread, 1 hinted | S (rename hints and the guide) → M (an importer that writes the settings table) | every BirdNET-Pi migrator |
| 2 | A‑2 | `scripts/play.php` layout `Extracted/By_Date/<date>/<species>/` | rows only | M | BirdNET-Pi migrators who want playback |
| 3 | P‑2 | `birdweather_client.go:255-283,679`; `actions_integrations.go:74` | exact coordinates, global threshold | S | anyone sharing to BirdWeather (privacy) |
| 4 | A‑3 | `utils/notifications.py` | one trigger mode | S | Apprise users, migrators |
| 5 | A‑5 | `frontend/static/messages/*.json` | English UI | L | non-English speakers |
| 6 | P‑12 | `config.go:937-941` | threshold only; interval in flight | S–M | owners of noisy sites |
| 7 | A‑4 | `disk_check.sh:31-33` | always purges | S | small-card stations that prefer data |
| 8 | N‑6 / P‑13 / A‑11 | `overview.php:17-27`; `BannerCard.svelte` | none | S–M | wall displays; BirdNET-Pi parity |
| 9 | P‑9 | `security.go:51-132`; `config.go:1707` | `frame-ancestors 'self'` | S | Home Assistant dashboards |
| 10 | P‑11 | `routes_enumeration_test.go:31-32,76-79` | learned thresholds invisible; no UI switch | M | owners with `BIRDNET_DYNAMIC_THRESHOLD` |
| 11 | A‑7 | `config.go:1323` | 2 threads fixed | S | Pi 4B / 5 owners |
| 12 | P‑20 | `3582fc843` | boundary residents flagged | S | API consumers of G‑16 |
| 13 | A‑6, P‑16 | `WeatherBadge.svelte`; `config.go:186` | none | S–M | everyone who uses weather; US users (°F, 12 h) |
| 14 | P‑8 | `audioNodes.ts:110-150` | none | S–M | quiet recordings |
| 15 | A‑17, A‑16, A‑18 | `automatic_update.cron`; `system_controls.php` | decisions not recorded | S (write the decision) | unattended stations |
| 16 | P‑10 | `mqtt_tls.go` | no client cert | S | brokers that require one |
| 17 | P‑17 | `config.go:253-279` | fixed 80 dB, one palette; options in flight | S | spectrogram readers |
| 18 | A‑15, A‑14 | `integrations.go:105-125`; `range.go:733` | partial | S | operators |
| 19 | A‑10 | `internal/imports/` | none | M | birdnet-go users |
| 20 | A‑9 | `cmd/benchmark` | none | M | Pi owners tuning overlap |
| 21 | G‑20 | `security.go`, `oauthProviders` | none | L | multi-user sites, SSO |
| 22 | G‑13 | `realtime.privacyfilter.vad` | none | M–L | private sites |
| 23 | P‑22 | `tls/` routes | flags and files only | M | LAN HTTPS |
| 24 | A‑8, P‑21, A‑12, A‑13, A‑21, P‑14, P‑4 | as in their rows | none / partial | S each | streamers, integrators |
| 25 | P‑18, P‑19 | `config.go:49`; `imageprovider/avicommons.go` | none | M | storage; photo coverage |
| 26 | A‑19 | `avahi-alias@.service` | wrong log line | S | LAN discovery |
| 27 | G‑6, G‑11, G‑14, P‑7 | their rows | their rows | L | as stated there |

### 9.9 Tallies, counted by script

Counted over the verdict of each finding in this file after Part 9 was added: the `N‑`
headings (Part 1), the `G‑` headings (Part 2), the last column of the Part 8 table (P‑2,
which is mixed, counted once as PARTIAL), and the last column of the table in 9.7. A row is a
*finding*, not a feature: N‑6 and P‑13 are one gap, P‑3 and A‑14 are one remainder, and the A‑
rows partly restate P‑rows, so the total overstates the number of distinct things missing.

| Group | Rows | SHIPPED | PARTIAL | GAP | DECLINED | IN PROGRESS | other |
|---|---|---|---|---|---|---|---|
| N (Part 1) | 6 | 5 | 0 | 1 | 0 | 0 | 0 |
| G (Part 2) | 34 | 24 | 2 | 5 | 2 | 1 | 0 |
| P (Part 8, corrected) | 22 | 4 | 6 | 12 | 0 | 0 | 0 |
| A (9.7, new) | 21 | 0 | 2 | 16 | 1 | 0 | 2 (one defect, one documentation error) |
| **All** | **83** | **33** | **10** | **34** | **3** | **1** | **2** |

So 45 of 83 findings (34 GAP, 10 PARTIAL, 1 IN PROGRESS) are open, before counting the
Part 5 and Part 6 rows that are tracked in their own tables (Part 5's GAP / PARTIAL rows:
`dee43f5`, `cd488cc`, `54b5545`, `0e7290d`, `92067fa`, `aa86b38`, `111fb34`, `0f46189`; Part 6's one GAP, `range/heatmap`).
The script (run it with `python3 -I`) reads this file and prints the counts the table is made from:

```python
import re, sys
from collections import Counter
t = open("docs/FEATURE_GAP_ANALYSIS.md").read()
word = lambda s: (re.match(r"(SHIPPED|GAP|PARTIAL|PARITY|DECLINED|IN PROGRESS|defect|doc)",
                           s.strip().strip("*").strip()) or [None, "OTHER"])[1]
c = Counter()
for id_, _, v in re.findall(r"^#{3,4} ((?:N|G)‑\d+) · (.*?) — (.*)$", t, re.M):
    c[(id_[0], word(v))] += 1
for l in t[t.index("## Part 8"):t.index("## Part 9")].split("\n"):
    if re.match(r"\| P‑\d+ ", l):
        last = l.rstrip().rstrip("|").split("|")[-1]
        c[("P", "PARTIAL" if l.startswith("| P‑2 ") else word(last))] += 1
for l in t[t.index("### 9.7"):t.index("### 9.8")].split("\n"):
    if re.match(r"\| A‑\d+ ", l):
        c[("A", word(l.rstrip().rstrip("|").split("|")[-1]))] += 1
print(sorted(c.items()))
```

### 9.10 For the next person

1. Read A‑1 and A‑2 first: they are the findings where a migrating user's station behaves
   differently on day one, and the documentation says the opposite.
2. Delete the line numbers from the rows whose symbol is named beside them (9.2), or add a gate
   that fails when a cited symbol is not within a few lines of its citation.
3. Verify the unverified: whether BirdNET-Pi's `MODEL` value selects our V2.4 classifier;
   the unit's behaviour on a real systemd host for A‑16; the speech-deferral and two-hit rules
   from `fc34a9706` / `e81763beb`; the per-key hot-reload of the 20 `OwnedBy` settings.
4. The working tree held other agents' uncommitted work (spectrogram options, a per-species
   interval) while this was written. Re-run the `git grep … HEAD` checks in 9.1 against
   whatever has merged before closing P‑12 and P‑17.
