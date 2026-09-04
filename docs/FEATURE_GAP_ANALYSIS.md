# Feature gap analysis

> What BirdNet-Behavior does **not** yet do, measured against the two projects
> that share its problem domain:
>
> * [`Nachtzuster/BirdNET-Pi`](https://github.com/Nachtzuster/BirdNET-Pi) — the
>   surviving maintained fork of the PHP/Python original this project descends
>   from. Compared at `88985a3` (2026‑02‑28).
> * [`tphakala/birdnet-go`](https://github.com/tphakala/birdnet-go) — an
>   independent Go rewrite of the same idea, an order of magnitude larger than
>   either. Compared at `1e74c82` (2026‑09‑02).

## How this was measured

Both repositories were cloned and read, not summarised from memory. Sizes, for
scale — and each names the tip it was measured at rather than inheriting the
comparison's, because these numbers move: upstream at `88985a3` and
`b184f689` (2026‑09‑03), ours at `ee795ed`. All three are `git ls-files`
line counts with tests included.

| Project | Language | Lines | Notes |
|---|---|---|---|
| Nachtzuster/BirdNET-Pi | PHP + Python + shell | ~25 k, of which ~18 k is the fork's own code — the rest is vendored Adminer and file-manager PHP | 188 tracked files |
| tphakala/birdnet-go | Go + Svelte | ~542 k Go (~265 k excluding `_test.go`) | 52 subtrees under `internal/`; 137 Go packages counting nested ones |
| **BirdNet-Behavior** | Rust | ~200 k `.rs` | 8 crates + binary |

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
`crates/birdnet-web/src/routes/admin/settings/render/`) leaves four genuine
gaps and a handful of deliberate divergences.

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
| **Ours** | **This row was wrong when first written, and the correction is the finding.** It said "`routes/livestream.rs` streams the raw tap unshifted". It does not: `livestream.rs:250` builds `freq_shift_filter(STREAM_SAMPLE_RATE, params.freq_shift_hz)` and passes it to ffmpeg as `-af`, so `/stream?freq_shift_hz=N` has always worked. What was missing is that **nothing in the UI ever sent it** — `recordings.html`'s `srcFor()` built `/stream` or `/stream?source_id=…` and never a shift — so the feature was reachable only by hand-editing a URL. |
| **And a defect the re-check found** | Five doc comments, including the `--freq-shift-hz` CLI help an operator reads before choosing a value, said a **positive** (upward) shift "makes calls accessible to people with high-frequency hearing loss". That is backwards. Presbycusis takes the *top* of the range first, so an 8 kHz warbler is restored by moving it **down**. Upstream agrees and was checked as the primary source: `install_config.sh` ships `FREQSHIFT_HI=6000` / `FREQSHIFT_LO=3000` (a `rubberband` ratio of 0.5) and a sox `FREQSHIFT_PITCH=-1500` — two independent settings, both downward. A listener following our documentation would have shifted the song further out of their hearing. |
| **Why it matters** | This is an accessibility feature, not a novelty. Age-related high-frequency hearing loss starts around 8 kHz; a great deal of warbler and kinglet song lives above it. A feature that works only if you know to hand-edit a query string is not available to the people it is for, and one documented in the wrong direction is worse than absent. |
| **Resolution** | A pitch control beside the Listen button on `/recordings`, with downward presets (the accessibility direction) and one upward option; the choice is remembered per browser in `localStorage`. Per-listener rather than upstream's station-wide flag, and deliberately: hearing loss is a property of a person, and this station serves one ffmpeg per connection rather than one Icecast broadcast for everyone, so it can do better than upstream here. All five doc comments corrected against the primary source, with `ACCESSIBILITY_SHIFT_HZ` naming the direction and a `const` assertion failing the *build* if its sign is ever flipped back. The `freq_shift_hz` query parameter is now clamped to ±24 kHz — it was an unbounded `i32` from an unauthenticated request, and `freq_shift_hz=2000000000` asked ffmpeg to resample from ~2 GHz, four streams at a time. |

### N‑3 · Choosing which RTSP source feeds the live stream — PARTIAL

| | |
|---|---|
| **Upstream** | `RTSP_STREAM_TO_LIVESTREAM` (an index into the comma-separated `RTSP_STREAM` list), consumed at `scripts/livestream.sh:26-36`. |
| **Ours** | Half of this has landed. `GET /api/v2/stream` accepts **`?source_id=<id>`**, resolved against the `audio_sources` table — `routes/livestream.rs:86` declares the parameter, `:216` branches on it, and an unknown id returns `404`. The Recordings page's `srcFor()` already builds it (`templates/recordings.html`), so a listener can choose a source from the UI. Without the parameter the first non-disabled `audio_sources` row wins, and a station with no rows yet gets `503`. What is missing is the station-wide **default**: `grep -rn livestream_source` over `crates/` and `src/` finds nothing, so an operator cannot say once which source is the one to listen to. |
| **Why it matters** | A two-microphone station (feeder + nest box) has one of them that a person actually wants to listen to. |
| **Plan** | Add a `livestream_source` setting naming the default `audio_sources.id`, consulted where `resolve_default_source` currently takes the first enabled row, and falling back to that when unset or when the named source is not capturing. The shipped query parameter is **`?source_id=`** — an earlier draft of this row specified `?source=`, and implementing the default against that spelling would leave the station answering to two names for one thing. |

### N‑4 · Bulk species management — PARTIAL

| | |
|---|---|
| **Upstream** | `scripts/species_tools.php` — per-species on-disk clip counts (`disk_species_count.sh`), bulk delete of a species' detections *and* its files, and confirm/exclude/whitelist list editing from the same table. |
| **Ours** | Per-species retention and purge exist (`crates/birdnet-core/src/audio/capture/disk/purge.rs`, `max_files_per_species`), the quarantine review flow exists (`routes/pages/quarantine.rs`), and include/exclude editing exists (`routes/admin/species/`). What is missing is the single table that shows *every* species with its clip count and offers "delete this species entirely". |
| **Why it matters** | The recurring real-world need is "my station has logged 4 000 phantom Eurasian Wrens from a squeaky gate; remove them and stop recording them". Today that is three separate screens. |
| **Plan** | Add `/admin/species/manage`: one row per species with detection count, on-disk clip count and bytes, last-heard date, and current list membership; actions are *exclude*, *delete detections*, *delete clips*, each confirmed and audit-logged through the existing `audit_log` table. Counts come from a new `species_disk_usage` query in `birdnet-db` rather than shelling out. |

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

birdnet-go is roughly 2.7× this project by line count and has taken the design
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

#### G‑5 · Loudness normalisation of exported clips (EBU R128) — GAP

| | |
|---|---|
| **Upstream** | `conf.NormalizationSettings` (`targetLUFS`, `truePeak`), applied as a single linear gain in `internal/audiocore/audionorm`. |
| **Ours** | Clips are written at capture level. `agc` exists as a capture-time toggle but is documented as mostly amplifying the noise floor, and is off by default. |
| **Why it matters** | A gallery of clips at wildly different levels is unusable — the listener rides the volume control between every one, and a quiet clip at the end of a playlist gets missed. Normalising the *export* (not the analysis input, which must stay untouched) is the standard fix. |
| **Plan** | Add `audio::extraction::loudness` implementing the ITU-R BS.1770 K-weighted integrated loudness measurement, then a single gain to `target_lufs` clamped so no sample passes `true_peak`. Applied at write time only, with the measured LUFS recorded in the clip's RIFF INFO block (`extraction/metadata.rs`) so re-normalising is idempotent and auditable. |

#### G‑6 · Extended capture for long calling sessions — GAP

| | |
|---|---|
| **Upstream** | `conf.ExtendedCaptureSettings` — for a configured species list, merge consecutive detections into one clip up to `maxDuration` (capped at 1200 s) instead of emitting one clip per window. |
| **Ours** | One clip per detection, deduplicated by `duplicate_interval_secs`. |
| **Why it matters** | An owl calling for six minutes, a nightjar churring, a woodpecker drumming session — these produce dozens of near-identical short clips today, which is both worse listening and more disk. |
| **Plan** | A session-merging stage in the extraction path keyed on species + source + gap: while the same species keeps being detected within the gap, extend the open clip rather than opening a new one. Bounded by `max_duration_secs` and by the segment-spanning machinery from G‑3, which this depends on. |

#### G‑7 · Stream protocols beyond RTSP — GAP

| | |
|---|---|
| **Upstream** | `conf.StreamType` — `rtsp`, `http` (direct/Icecast), `hls` (`.m3u8`), `rtmp` (OBS push), `udp` (RTP), each with transport selection. |
| **Ours** | ALSA, PipeWire and RTSP (`crates/birdnet-core/src/audio/capture/types.rs:14` `CaptureSource`). |
| **Why it matters** | The cheapest way to add a second listening post is often an existing stream that is not RTSP — a neighbour's Icecast feed, an HLS wildlife cam, an OBS push from a laptop. |
| **Plan** | `CaptureSource` gains a `Stream { url, kind, transport }` variant; the ffmpeg command builder in `capture/process.rs` already takes a URL, so most of this is URL classification, per-protocol ffmpeg flags, and the reconnect policy each protocol needs. Probing (`capture/probe.rs`) must learn to answer for them. |

#### G‑8 · RTSP media mode — GAP (minor)

| | |
|---|---|
| **Upstream** | `conf.MediaMode` — `auto` (try audio-only, fall back), `audio-only` (never fall back, fail visibly), `full-stream` (request video and discard it). Default `full-stream`, with a comment naming the cameras that complete an audio-only handshake just long enough to mislead the fallback. |
| **Ours** | We always request the full stream. |
| **Why it matters** | Cameras have a bounded number of concurrent video sessions. A station that opens a video slot to listen to audio can lock the owner out of their own camera. |
| **Plan** | A per-source `media_mode` mapping onto ffmpeg's `-allowed_media_types audio`, with the upstream fallback ladder and its failure accounting. |

#### G‑9 · Audio watchdog tuning — PARTIAL

| | |
|---|---|
| **Upstream** | `conf.WatchdogSettings` — operator-tunable `checkInterval`, `silenceThreshold`, `maxRetries`, `retryBackoff`, `cooldown`, `escalationTimeout`, with an explicit ESCALATED→FAILED state machine (`internal/audiocore/liveness.go`). |
| **Ours** | We have a watchdog (`src/doctor/watchdog.rs`, `sd_notify.rs`, capture restart logic in `audio/capture/manager.rs`) and a deadman timer (`BIRDNET_DEADMAN_HOURS`), but the thresholds are constants. |
| **Why it matters** | The right silence threshold at a busy feeder is not the right one for an arctic winter station where 30 s of silence is normal and 6 h is not. |
| **Plan** | Lift the constants into a `WatchdogConfig` with the same six knobs, defaulted to today's values, and expose them under `/station/capture`. |

### 2.2 Classification and detection quality

#### G‑10 · Multiple classifier models — GAP (largest single item)

| | |
|---|---|
| **Upstream** | `internal/classifier/` (≈44 k lines). An orchestrator running any of: BirdNET v2.4, BirdNET v3.0, Google **Perch v2**, a **bat** classifier built on BirdNET v2.4 embeddings, and **BSG regional** models — concurrently, routed per audio source (`AudioSourceConfig.Models`), each with its own labels, locale and threshold. A model catalog with regional variants (`model_catalog.go`, `model_catalog_regional_gen.go`), a download manager pulling from HuggingFace with a configurable endpoint for mirrors, primary-model swap and failover (`model_manager*.go`), and `GET /api/v2/models/catalog` · `POST /api/v2/models/install/:id`. |
| **Ours** | One BirdNET ONNX classifier plus the metadata/geomodel (`crates/birdnet-core/src/inference/model.rs`, `species_filter.rs`). Model and labels are paths given by config; `scripts/setup-onnxruntime.sh` seeds the runtime. The only workspace mention of Perch is a comment in `audio/resample.rs:4` noting it wants 32 kHz. |
| **Why it matters** | Two distinct things. (a) **Coverage**: BirdNET is weakest exactly where a hobbyist most wants help — outside Europe/North America, and on non-birds. Perch v2 is materially better in the tropics; a bat classifier turns one box into two instruments. (b) **Corroboration**: two independent models agreeing is far stronger evidence than one model being confident, and it is the honest way to attack the false-positive problem that our `corroboration.rs` attacks with repetition alone. |
| **Plan** | This is a multi-stage programme, not one change. **Stage 1** — make the classifier a trait. Extract `trait Classifier { fn labels(&self) -> &LabelSet; fn input_spec(&self) -> InputSpec; fn infer(&self, samples: &[f32]) -> Result<Vec<f32>, InferenceError>; }` from the concrete `Model`, with `InputSpec` carrying sample rate, window length and normalisation so the pipeline stops assuming 48 kHz/3 s. **Stage 2** — a registry that loads N classifiers from config and a per-source routing table. **Stage 3** — a merge policy in the detection pipeline: union with per-model thresholds, plus an *agreement* flag recorded on the detection row that the review UI and `corroboration.rs` can both use. **Stage 4** — Perch v2 as the second concrete implementation (32 kHz, 5 s windows, a CSV label file), which is the real test of whether Stages 1–3 are right. **Stage 5** — a model catalog and downloader with checksum verification, mirror support and atomic install, reusing the auto-update machinery in `birdnet-integrations/src/auto_update/`. **Stage 6** — the bat classifier, which additionally needs the ≥192 kHz capture path and the ultrasonic validation filter (G‑14). Stages 1–3 are worth doing even if no second model ever ships, because they remove the hardcoded assumption that there is exactly one. |

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
| **Resolution** | Shipped as a directory rather than a file: `detection/dynamic_threshold/{mod,tests}.rs`, with `DynamicThresholds::effective_threshold(sci_name, base, now_ms)` (`mod.rs:261`) checking expiry on every read, and `LearnedThreshold` rows round-tripped by `birdnet-db`'s `replace_all` / `load_all`. It is applied in the binary's daemon — `src/daemon/disposition.rs:145` takes an `Option<&DynamicThresholds>`, held as `DynamicThresholdState` in `src/daemon/processor.rs:35` — not in `detection/daemon/process.rs`, which is not a path in this tree. **Still to check against the plan:** that the `/admin/species/test` preview shows the *effective* threshold rather than the configured one. That was the plan's stated trap and nothing in this pass confirmed it was avoided. |

#### G‑13 · Silero VAD privacy gate — GAP

| | |
|---|---|
| **Upstream** | `conf.VADSettings` — an embedded Silero VAD ONNX model detecting speech *presence* (not content, not speaker), opt-in, augmenting the label-based privacy filter. |
| **Ours** | `crates/birdnet-core/src/detection/privacy.rs` — rank-based on BirdNET's own `Human` labels, with adjacent-chunk masking. Our own `noise.rs` doc comment records that at the shipped `top_n` of 10 the privacy filter's cutoff `max(10, …)` never actually excludes anything, so it is a blunt instrument. |
| **Why it matters** | A garden microphone records the neighbours. BirdNET's human classes are a by-product of its training set, not a speech detector, and they miss quiet conversation at exactly the distance where it is still intelligible. For anyone deploying where consent matters, a real VAD is the difference between a defensible privacy claim and a hopeful one. |
| **Plan** | Optional second small ONNX session (~2 MB) run only on chunks that pass the cheap gates, gating clip *retention* rather than detection: a chunk with speech is analysed and its detection recorded, but no audio is written. Off by default, and the model shipped alongside rather than embedded so the binary size story does not change for stations that do not want it. |

#### G‑14 · Ultrasonic validation filter — GAP (blocked on G‑10 Stage 6)

| | |
|---|---|
| **Upstream** | `conf.UltrasonicFilterConfig` — measures the coefficient of variation of ultrasonic-band energy across STFT frames; real echolocation is bursty (high CV), audible-range false positives are flat at the noise floor. |
| **Ours** | No ultrasonic path at all. |
| **Plan** | Ships with bat support or not at all. Recorded here so the dependency is explicit. |

#### G‑15 · Taxonomy: family, genus, tree, and synonym aliasing — GAP

| | |
|---|---|
| **Upstream** | `internal/openfauna/aliases.go` — an authoritative legacy→canonical scientific-name map, because "acoustic models are trained on different taxonomies and time-frozen label sets, so they emit different scientific names for the same species" (e.g. `Streptopelia senegalensis` → `Spilopelia senegalensis`). Plus `internal/classifier/taxonomy.go`, `genus.go`, and `GET /api/v2/taxonomy/{family,genus,tree}/…`. |
| **Ours** | `crates/birdnet-core/src/inference/labels.rs` parses a taxonomic class column when the label file has one (`labels.rs:216` notes the geomodel's file does not) and otherwise has no taxonomy. No synonym map. |
| **Why it matters** | Two separate problems. (a) **Silent double-counting**: our own species filter matches by scientific name across two vocabularies (`species_filter.rs`'s whole doc comment is about this hazard), and a reclassified genus makes the same bird two species in the life list, the year list, and every retention query. (b) **Browsing**: "show me every warbler" is a natural question that a flat species list cannot answer. |
| **Plan** | Ship a curated alias table as a data file with provenance, normalise on write in `detections`, and add a one-off migration that collapses existing rows (reported, reversible, and never run silently). Then derive family/genus from the label file where present and expose `/species?family=…`. The alias table needs a staleness gate — a test that fails when the shipped classifier's label set contains a name the table maps *from*, which would mean the map is being applied to a model that already uses the canonical name. |

#### G‑16 · Species tracking: yearly, seasonal, and returning-after-absence — SHIPPED (API only; not yet on the pages)

| | |
|---|---|
| **Upstream** | `conf.SpeciesTrackingSettings` — "new species" window, **yearly** tracking with a configurable reset date, **seasonal** tracking with hemisphere-aware season boundaries (`GetDefaultSeasons` handles northern, southern *and* equatorial wet/dry), and **infrequent** tracking that flags a species returning after `absenceDays`. Notification suppression is tracked separately per category. |
| **Ours** | `rare_species_days` drives `/feeds/rare.rss` and `/feeds/rare.ics`, whose SQL (`routes/feeds.rs:47`) implements two definitions of rare — first-ever, and returning after a gap — and a life list exists (`routes/pages/life_list.rs`). The year list, the season list and the hemisphere awareness now exist too: `crates/birdnet-db/src/species_tracking.rs` computes `new_ever`, `new_this_year`, `new_this_season`, `returning_after_absence` and `days_since_previous` per species, served at `GET /api/v2/species/tracking` (`routes/species.rs:19`). The season boundary is hemisphere- and latitude-dependent and lives in `crates/birdnet-core/src/season.rs`, which carries northern, southern and equatorial wet/dry tables. |
| **Why it matters** | "First of the year" is the unit birders actually keep score in, and a station is uniquely good at catching it — it is listening at 04:40 when nobody is awake. Seasonal firsts are the phenology signal this project's DuckDB analytics already exist to measure, so not surfacing them on the dashboard is leaving the best story untold. Hemisphere matters because half the potential users are south of the equator and a northern-defaults season table is wrong by six months for all of them. |
| **Resolution** | The module and the season table landed as planned, and the API reports the windows alongside the species rather than as a courtesy — a bare "first this season" is unreadable without knowing which season the station thinks it is in, and a station with no latitude honestly returns `season: null`. **What did not land is the wiring.** `crate::tracking` has exactly one consumer, `routes/species.rs:52`; the flags do not reach the today page, the RSS/iCal feeds or the notification trigger vocabulary, so the capability exists but nothing a non-API user looks at shows it. That is the remainder of this item. |

#### G‑17 · Dog-bark suppression window — PARTIAL

| | |
|---|---|
| **Upstream** | `conf.DogBarkFilterSettings` — a species list plus `remember`, suppressing those species for N seconds *after* a bark. |
| **Ours** | `crates/birdnet-core/src/detection/noise.rs` drops the whole chunk on a noise class at or above threshold, and its doc comment argues explicitly against spreading to neighbouring chunks (a bark is a few hundred milliseconds and the chunks overlap). |
| **Verdict** | Our design is better reasoned for the *chunk* case. The upstream `remember` window addresses something different: a dog that barks for a minute produces phantom detections in the gaps *between* barks, where no bark is present to trigger the chunk filter. |
| **Plan** | Keep the chunk filter as-is; add an optional `noise_remember_secs` that suppresses the specific species that co-occur with the noise class, not all species, for a bounded window after it. Off by default. |

### 2.3 Web, security and deployment

#### G‑18 · Trusted-proxy client-IP resolution — SHIPPED (correctness/security)

| | |
|---|---|
| **Upstream** | `conf.Security.TrustedProxies` — a CIDR/IP list whose forwarded headers (`CF-Connecting-IP`, `X-Forwarded-For`, `X-Real-IP`) may be believed, with loopback/link-local/RFC1918 peers always trusted, a reserved `"cloudflare"` value expanding to the published edge ranges, and — the point — headers **ignored** when the immediate peer is not trusted, so a directly exposed instance cannot be IP-spoofed. |
| **Ours** | The boolean is gone. `crates/birdnet-web/src/rate_limit.rs:233` now reads `pub(crate) fn extract_ip(req: &Request<Body>, trusted: &TrustedProxies) -> IpAddr`, and `RateLimitConfig` carries `trusted_proxies: TrustedProxies` (`rate_limit.rs:65`) rather than a flag. The allow-list itself is `crates/birdnet-web/src/client_ip.rs`, populated at startup by `server.rs:94` `trusted_proxies_from_env`. |
| **Why it matters** | Both settings of that boolean are wrong behind a proxy. `false` — today's only reachable state — means every request through a reverse proxy shares the proxy's IP, so one abusive client exhausts the bucket for the whole household, and the audit log records the proxy for every login. `true`, had it been wired, would mean any client can set `X-Forwarded-For` and get a fresh bucket, which is worse. The correct behaviour needs the peer address, which neither state consults. |
| **Resolution** | `client_ip.rs` walks `X-Forwarded-For` right-to-left and stops at the first untrusted hop (`:408`), with loopback and the RFC1918 ranges trusted by default and a reserved `cloudflare` name expanding to a snapshot of the published edge ranges (`:234`, `:318`). The discrimination the plan demanded is gated **both** ways rather than only the permissive one: `an_untrusted_peer_cannot_forge_its_own_address` (`:498`), `the_walk_stops_at_the_first_untrusted_hop_from_the_right` (`:537`) and `cf_connecting_ip_from_an_untrusted_peer_is_ignored` (`:605`) are the ignoring half; `the_cloudflare_name_lets_the_walk_pass_the_edge` (`:617`) is the honouring half. **Not confirmed in this pass:** whether the resolved IP reached the audit log and session binding, or only the rate limiter. |

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
| **Ours** | HTTP Basic (`auth.rs`), a HMAC cookie session (`session.rs`), a `users` table and a `sessions` table (O‑15), and audit logging. No federated identity. Grep for `oauth`/`oidc` finds only Apprise URL parsing and an unrelated `rules.rs` match. |
| **Why it matters** | Less about the home station and more about the shared one — a reserve, a school, a research group where several people need access, one of them leaves, and there is exactly one shared password written on the wall. |
| **Plan** | Authorization-code + PKCE against a configured OIDC discovery document, mapping the `sub`/`email` claim onto the existing `users` table so sessions, roles and the audit log are unchanged. Named providers are then just pre-filled issuer URLs. Basic auth stays as the fallback for a headless LAN box. |

#### G‑21 · HLS live streaming — GAP

| | |
|---|---|
| **Upstream** | `GET /api/v2/streams/hls/t/:token/playlist.m3u8` with per-session tokens, `LiveStreamSettings` (bitrate, sample rate, segment length). |
| **Ours** | `GET /api/v2/stream` — MP3 over HTTP chunked transfer (`routes/livestream.rs`), which is a genuine simplification over upstream BirdNET-Pi's Icecast. |
| **Why it matters** | Chunked MP3 has no recovery. On mobile, a network handover ends the stream and the tab goes silent with no indication; there is no seeking and no buffer target. HLS is what mobile browsers are built around. |
| **Plan** | Keep the MP3 endpoint (it is simple and it works on a LAN) and add an HLS variant behind a token, sharing the encode stage. This depends on the same re-encode seam as N‑2, so the two should land together. |

#### G‑22 · Dashboard layout customisation — GAP

| | |
|---|---|
| **Upstream** | `conf.DashboardLayout` — an ordered list of elements (`banner`, `daily-summary`, `new-species-highlights`, `currently-hearing`, `detections-grid`, `live-spectrogram`, `video-embed`), each enabled/half/full width, with a banner carrying a location map, live weather and a custom image, plus six colour schemes and a custom primary/accent pair. |
| **Ours** | A fixed dashboard (`routes/pages/dashboard/`, `homes/`), light/dark via `theme-guard.js`, and a custom image (`custom_image_dir`). |
| **Why it matters** | Half the stations that exist are on a wall-mounted tablet in a visitor centre or a kitchen, where what needs to be on screen is not what a person debugging a microphone needs. |
| **Plan** | Store an element list in `settings` and render from it. Our HTMX partials are *already* the element vocabulary — `/pages/hero-status`, `/pages/today-list`, `/pages/most-recent`, `/pages/hourly-chart` and the rest are exactly these components — so this is a layout table and a drag-to-reorder editor over machinery that exists, not a rewrite. |

#### G‑23 · Detection comments — PARTIAL (the batch half has shipped)

| | |
|---|---|
| **Upstream** | `POST /api/v2/detections/:id/comments`, `batch/{delete,lock,resolve,review}`, and an ignored-species list. |
| **Shipped — batch operations** | `POST /api/v2/detections/batch` landed in PR #234 (`routes/api_write.rs`: route table `:62`, registration `:98`, handler `:436`). It applies one of `review` / `lock` / `unlock` / `delete` to up to `BATCH_MAX = 500` detections per request — a bound the compiler checks, not a test (`api_write.rs:341`, with a `const` assertion at `:350`). One bad key does not sink the batch: each item gets its own result and the `failed` count is named at the top level. Every detection it changes gets an audit row. It is bearer-only, mounted behind `api_token::require_bearer` (`server.rs:152`), so it does not exist on a station with no `BNB_API_TOKEN`. Gated by four tests in `crates/birdnet-web/tests/the_api_can_change_the_station.rs:783,840,890,967` and by the SQLite↔DuckDB desync guard in `tests/analytics_divergence.rs:749,795`. Upstream's batch `resolve` is the only member of their set with no analogue here. |
| **Ours — what is still missing** | **Free-text comments.** There is no `detection_comments` table: 41 migrations exist and none creates one. There is no route in `crates/birdnet-web/src/routes/` whose path contains `comment`. And `detection_reviews.notes` is not a thread wearing another name — migration 13 puts it under `UNIQUE(date, time, sci_name)` and writes it with `INSERT … ON CONFLICT`, so a second review **overwrites** the first, and the table carries no user column and no foreign key to `users`. The batch endpoint writes that field (*"`review` only: free text attached to every verdict in the batch"*), which is exactly what makes it look like a comment and why the distinction is worth stating. Review (`detection_reviews`, `/detection-reviews`), lock/unlock and bulk review in the search page (`/pages/search-bulk`) are unchanged. |
| **Why it matters** | The verification loop is where a station's data becomes usable to anyone else. "Why did I mark this wrong" is the note that makes a review defensible six months later. An overwritten, unattributed `notes` field cannot carry that: the second reviewer erases the first one's reasoning without either of them knowing. |
| **Plan** | A `detection_comments` table — append-only, user-attributed, audit-logged — with the routes to read and write it. The batch endpoints this row also planned are done; nothing here is blocked on them. |

#### G‑24 · Profiling endpoints — GAP

| | |
|---|---|
| **Upstream** | `conf.DiagnosticsConfig.Profiling` — pprof behind the auth middleware, with a generated token when no auth provider is configured, and block/mutex sampling rates whose costs are documented in unusual detail. |
| **Ours** | `--doctor`, `--channel-report`, `--support-bundle`, Prometheus metrics. No live profiler. |
| **Why it matters** | "The Pi is at 100 % CPU and I don't know why" is answerable in one command with a profiler and is a week of guessing without one. |
| **Plan** | A `/debug/pprof`-shaped endpoint serving `pprof`-format CPU and heap profiles behind the same auth as the admin panel plus a required token, sampling off by default. Rust has `pprof`-compatible collectors; the constraint is that they must not be linked in unless the feature is enabled, so this becomes a Cargo feature and a `BUILD_FEATURES` entry. |

#### G‑25 · Error-tracking telemetry — DECLINED, with a substitute

| | |
|---|---|
| **Upstream** | `internal/telemetry` — opt-in Sentry with a `SystemID`, plus `internal/observability`. |
| **Ours** | Prometheus metrics, structured tracing, and the support bundle with secret redaction gated by a test (`src/helpers/offsite.rs:591`). |
| **Verdict** | Shipping a crash reporter that phones a third party from a device with a microphone in someone's garden is a privacy posture this project should not adopt, even opt-in. |
| **Substitute** | Make the local path better: a persistent panic/error ring buffer written to disk, surfaced in the support bundle and at `/station/…`, so a user can *choose* to send it. That is the same diagnostic value without the default-on-the-network question. |

### 2.4 Integrations

#### G‑26 · Weather providers — PARTIAL

| | |
|---|---|
| **Upstream** | `internal/weather/` — `provider_yrno.go`, `provider_openweather.go`, `provider_wunderground.go`, a common interface, icon mapping, and a poll interval. Weather is joined onto detections and shown on the dashboard banner. |
| **Ours** | `crates/birdnet-integrations/src/weather.rs` — Open-Meteo only, off unless `BNB_WEATHER_ENABLED=1`, self-host-able via `BNB_WEATHER_BASE_URL`. Stored in a `weather` table. |
| **Why it matters** | Open-Meteo is the right default (no key, permissive terms, self-hostable) and we should keep it. But a station owner who already runs a **personal weather station** has ground-truth data ten metres from the microphone, and that is a far better covariate for bird activity than a gridded forecast — which is exactly what the Wunderground provider is for. |
| **Plan** | Extract a `WeatherProvider` trait from the existing client, keep Open-Meteo as the default implementation, and add Wunderground (personal station) and yr.no (no key, Norwegian Met, good for Europe). OpenWeather is the least interesting of the three and comes last. |

#### G‑27 · eBird API integration — GAP

| | |
|---|---|
| **Upstream** | `internal/ebird/` — `client.go`, `observations.go`; `EBirdSettings` with API key, cache TTL and locale; `/api/v2/integrations/ebird/test`. |
| **Ours** | `info_site=EBIRD` produces *links* to eBird (`admin/settings/render/system.rs`). No API client. |
| **Why it matters** | eBird recent-observations for the station's region is the best available answer to "is this plausible right now" — better than the geomodel, which is a static climatological prior with no idea that the species arrived last Tuesday. It is the natural input to a "phantom species" check, which we already have a page for (`/admin/quality/phantoms`). |
| **Plan** | A cached client for `data/obs/{regionCode}/recent` keyed on the station's region, feeding (a) a "recently reported nearby" badge on the detection detail page and (b) a corroboration signal in the data-quality view. Cache to disk with the TTL so an offline station degrades to its last snapshot rather than failing. |

#### G‑28 · Notification delivery resilience — PARTIAL

| | |
|---|---|
| **Upstream** | `conf.PushSettings` — circuit breaker (max failures, timeout, half-open probes), periodic health check, token-bucket rate limiting, per-provider filters on type/priority/component/metadata, a **script** provider (exec with env/stdin format), and webhook auth with `*_file` secret indirection for Docker/Kubernetes secrets. |
| **Ours** | `crates/birdnet-integrations/src/retry.rs` (exponential backoff), `dispatch/limit.rs` (rate limiting), an `outbound_queue` table for store-and-forward replay, seven native targets plus Apprise, and webhook auth in `alert_rules.rs` (bearer/basic/custom header). |
| **Verdict** | Our store-and-forward queue is arguably stronger than a circuit breaker for the actual failure mode (a station's uplink is down for an hour). What we lack is the *fast-fail* half: when a provider is broken, every notification still pays the full retry ladder, which on a busy dawn means the whole dispatch path is spending its time on a dead endpoint. |
| **Plan** | Add a per-target circuit breaker in front of the existing retry ladder — trip after N consecutive failures, half-open probe after a timeout, and enqueue to `outbound_queue` while open rather than dropping. Add `*_file` indirection for every credential the dispatcher reads, since that is a small change with a real deployment payoff. The script provider is declined: arbitrary command execution configured through the web UI is the same RCE surface as the declined web terminal. |

#### G‑29 · Metric-triggered alert rules — PARTIAL

| | |
|---|---|
| **Upstream** | `internal/alerting/` — a rules engine over both **events** (detection, stream error) *and* **metrics** (CPU %, memory %, disk %), with a typed schema of operators (`is`, `in`, `contains`, `>`, `≥`, …), escalation steps, per-metric-key cooldowns, and persisted history. |
| **Ours** | `crates/birdnet-db/src/alert_rules.rs` — detection-triggered only: species glob, confidence range, hour window, day-of-week; actions webhook/log/suppress; import/export with credential redaction. Separately, `BIRDNET_STATION_HEALTH_ALERTS` and the deadman timer cover some system alerting with fixed thresholds. |
| **Why it matters** | The station failure that actually loses data is silent: the disk fills, or the microphone dies, and nobody notices for three weeks. Fixed-threshold health alerts cover the obvious cases; a rule engine lets an operator say "tell me if disk goes over 85 % *or* if the hourly detection count drops below 20 % of its 7-day median", which is the one that catches a dying microphone. |
| **Plan** | Extend the existing rule schema with a `trigger` discriminant (`detection` \| `metric`), a metric vocabulary (`cpu_pct`, `mem_pct`, `disk_pct`, `detections_per_hour`, `seconds_since_last_detection`, `capture_restarts_per_hour`), numeric operators, and a cooldown keyed on rule + metric instance. Evaluated by the existing maintenance loop. The current detection-rule shape stays valid, and the export format version (`EXPORT_VERSION`, currently 1) bumps with a documented upgrade. |

#### G‑30 · Backup destinations — PARTIAL

| | |
|---|---|
| **Upstream** | `internal/backup/targets/` — `local`, `ftp`, `sftp`, `s3`, `rsync`, `gdrive`; encryption with an auto-managed AES-256-GCM key; retention by age/count/minimum; daily and weekly schedules. |
| **Ours** | Local, SFTP and S3-compatible (`crates/birdnet-integrations/src/offsite/`), an encrypted container (`offsite/envelope.rs`), SigV4 signing, keep-N retention, and a restore path. |
| **Verdict** | Close. `rsync` is the one worth adding — it is what a person with a NAS already uses, it is incremental, and it does not need a bucket or an FTP daemon. FTP is declined (cleartext by default, and FTPS is worse-supported than SFTP everywhere it matters). Google Drive is declined (an OAuth flow and a third-party dependency for a destination `rclone` already serves). |
| **Plan** | Add an `rsync` target driving the system binary over SSH with the existing host-key policy from `offsite/sftp.rs`, and daily-schedule support alongside the current weekly. |

#### G‑31 · MySQL / external database — DECLINED

| | |
|---|---|
| **Upstream** | `Output.MySQL` alongside SQLite. |
| **Ours** | SQLite for OLTP, DuckDB for analytics. |
| **Verdict** | The SQLite+DuckDB pairing is this project's architectural thesis — it is why the behavioural analytics are possible at all on a Pi, and a network round trip per row would undo it. A station wanting central aggregation is better served by shipping DuckDB/Parquet exports than by writing detections over the network. |
| **Substitute** | Make the export path good enough that this never comes up: Parquet export of the detections table, already trivially available through DuckDB, exposed as a scheduled job. |

### 2.5 Operations and platform

#### G‑32 · System introspection APIs — PARTIAL

| | |
|---|---|
| **Upstream** | `/api/v2/system/{info,resources,disks,processes,network-interfaces,jobs,temperature/cpu,external-media,inference}` plus `/api/v2/control/{restart,reload,rebuild-filter,restart-source/:id}`. |
| **Ours** | `/api/v2/health`, `/api/v2/metrics`, `/station/*`, `/system/disk`, `/admin/system/*` and `POST /admin/system/service/restart`. Missing: per-process view, network interfaces, external media detection, a job list, per-source restart, and filter rebuild. |
| **Why it matters** | Per-source restart is the one that matters daily — restarting the whole service to recover one wedged RTSP camera drops every other source and loses in-flight audio. |
| **Plan** | Add `POST /api/v2/control/restart-source/:id` against the capture manager's existing supervisor, plus `/api/v2/system/jobs` over the `maintenance_runs` table. Network interfaces and external media are onboarding aids and follow. |

#### G‑33 · Hardware profiling and memory policy — PARTIAL

| | |
|---|---|
| **Upstream** | `internal/hwprofile`, `internal/cpuspec`, `internal/mempolicy` — detect the machine, set `GOMEMLIMIT` and cap the glibc arena, and gate features on available memory (`LowMemoryConfig`). |
| **Ours** | `sysinfo`-based CPU/memory/temperature reporting, `--doctor` checks, and `BIRDNET_DUCKDB_MEMORY_LIMIT`. |
| **Verdict** | Most of `mempolicy` is a Go GC concern that does not transfer to Rust. What does transfer is **capability gating**: refusing to enable DuckDB analytics or a second model on a 1 GB Pi Zero, with an explanation, rather than being OOM-killed at 3 a.m. |
| **Plan** | A startup memory-budget check that sizes the DuckDB limit and the analytics sync from detected RAM, warns when a configured feature will not fit, and records the decision in `--doctor` output. |

#### G‑34 · Machine-readable config schema — PARTIAL

| | |
|---|---|
| **Upstream** | `config.schema.json` generated by `cmd/gen-schema`, with tests asserting the shipped schema, the wiki page and the config comments cannot drift apart. |
| **Ours** | `.env.example` (37 kB, hand-maintained), the admin settings pages, and `docs/book/`. |
| **Why it matters** | Three surfaces describe every setting today and nothing gates them against each other, which is exactly the drift this repository's own testing conventions warn about. |
| **Plan** | Derive a JSON Schema from the settings registry at build time, ship it, and add a gate that fails when a settings key exists in the registry but not in `.env.example` or the admin UI — or vice versa. This is cheap and it retires a whole class of "documented but not implemented" bug that this project has already hit more than once. |

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
| **Encrypted off-site backup with SigV4 signing implemented in-tree** | `integrations/src/offsite/` | upstream leans on SDKs |
| **Share links** — signed, expiring, single-detection public links | `routes/share.rs` | — |
| **RSS and iCal feeds** of rare detections | `routes/feeds.rs` | — |
| **`--doctor`, `--channel-report`, `--migration-report`** — self-diagnosis that measures rather than asserts | `src/doctor/` | birdnet-go has `support collect`; the measured channel report is ours |
| **Audit log** of every operator action | `audit_log` table | — |
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
| 8 | Solar quiet hours | G‑4 | **Done** | The solar maths already exists; this is wiring plus a schedule-format migration. |
| 9 | Flickr image provider | N‑1 | **Done** | The provider seam was built for this and has stood empty. |
| 10 | Live-stream frequency shift | N‑2 | **Done** | Accessibility parity; shares the re-encode seam with G‑21. |

### Tier 2 — next

| # | Item | Finding |
|---|---|---|
| 11 | Metric-triggered alert rules | G‑29 |
| 12 | Notification circuit breaker + `*_file` secrets | G‑28 |
| 13 | Taxonomy: synonyms, family, genus | G‑15 |
| 14 | Detection comments (batch operations shipped, PR #234) | G‑23 |
| 15 | Weather provider trait + Wunderground + yr.no | G‑26 |
| 16 | eBird recent-observations client | G‑27 |
| 17 | Loudness normalisation of exports | G‑5 |
| 18 | Per-source restart + jobs API | G‑32 |
| 19 | `rsync` backup target + daily schedules | G‑30 |
| 20 | Config schema generation + drift gate | G‑34 |
| 21 | A station-wide default source for the live stream (`?source_id=` has shipped) | N‑3 |
| 22 | Bulk species management page | N‑4 |
| 23 | Watchdog tuning | G‑9 |
| 24 | Memory-budget capability gating | G‑33 |
| 25 | Noise "remember" window | G‑17 |

### Tier 3 — programmes, not tickets

| # | Item | Finding | Shape |
|---|---|---|---|
| 26 | Multi-model classifier stack | G‑10 | Six stages; Stages 1–3 (trait, registry, merge policy) are worth doing on their own merits |
| 27 | Inference backends (OpenVINO / XNNPACK) | G‑11 | Gated on Stage 1 of #26 |
| 28 | OAuth2 / OIDC | G‑20 | One generic OIDC implementation; named providers are configuration |
| 29 | HLS live streaming | G‑21 | Shares the encode seam with #10 |
| 30 | Dashboard layout customisation | G‑22 | The HTMX partials are already the component vocabulary |
| 31 | Extended capture | G‑6 | Depends on #6 |
| 32 | Additional stream protocols | G‑7 | HTTP/Icecast first; RTMP and UDP after |
| 33 | Silero VAD privacy gate | G‑13 | Depends on #26 Stage 1 for a second ONNX session |
| 34 | RTSP media mode | G‑8 | Small, but only meaningful alongside #32 |
| 35 | Profiling endpoints | G‑24 | Behind a Cargo feature |
| 36 | Local error ring buffer (Sentry substitute) | G‑25 | |
| 37 | Parquet export (MySQL substitute) | G‑31 | |
| 38 | Bat support + ultrasonic filter | G‑14 | Depends on #26 Stage 6 and a ≥192 kHz capture path |

### Not planned

G‑25 (Sentry), G‑31 (MySQL), the script notification provider in G‑28, FTP and
Google Drive in G‑30, and the BirdNET-Pi Adminer / file manager / web terminal.
Each has a stated reason above and a substitute where one is warranted.
