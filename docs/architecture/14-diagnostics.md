# Diagnostics & preflight

> How operators verify a BirdNet-Behavior install before (and after) it goes
> live, and how the binary itself validates its own configuration.

## Goals

The diagnostic surface exists to answer one question quickly:

> "Is this install in a state where it can actually detect birds, and if not,
> what specifically do I need to do to fix it?"

Constraints that shaped the design:

- **Non-technical operators must benefit.** A stack trace, error code, or
  log line that says only *what* failed is not enough. Each finding ships
  with a concrete *fix* in the same screen.
- **Output must be machine-parseable.** Monitoring scripts read the exit
  code; review threads attach the report; future automation may parse
  individual lines.
- **No new long-running services.** Diagnostics are one-shot. They run
  with the same binary the operator already has and exit.
- **No new runtime dependencies.** All checks shell out to tools the
  install already needs (`arecord`, `pactl`, `df`) or use the standard
  library. Optional tools are detected, not required.

## Surface

Three complementary mechanisms ship together:

### 1. Configuration validation (`birdnet_core::config::validate`)

Pure Rust, no I/O. Runs against an already-parsed `Config` and returns a
`Vec<Finding>`. Each finding carries a `Severity` (Warning / Error), the
configuration key, a human-readable message, and a remediation hint.

Validation is **advisory**: missing keys do not produce findings because the
binary runs with built-in defaults. A finding only fires when a key is
*present* and its value violates a documented bound (range, format, mutual
exclusion).

Unit tests pin every individual rule; property-based tests (proptest) cover
the full reachable range of each numeric field plus a panic-freedom
invariant for arbitrary string input.

### 2. Preflight subcommand (`birdnet-behavior --doctor` / `--doctor-json` / `--fix`)

Runs every check family in the `CHECK_FAMILIES` table of `src/doctor.rs`
(21 rows) against the live environment and prints a one-screen report.
`collect` iterates that table and nothing else; two gates
(`every_check_entry_point_is_in_the_table`,
`the_module_doc_names_every_submodule`) fail when a `check_*` function or a
submodule is missing from it. The checks live in thirteen submodules —
`analytics`, `audio`, `clock`, `config`, `database`, `disk`, `environment`,
`model`, `offsite`, `paths`, `tls`, `watchdog`, plus `fix`, whose `repair`
runs *before* the checks when `--fix` is given — and rendering (`text` /
`json` / exit code) lives in `render`. Each check produces a `Check { status, name, message,
remediation? }`. The CLI exits with the worst-severity-derived code:

| Exit | Meaning                                              |
| ---- | ---------------------------------------------------- |
| 0    | All checks passed (some may be skipped/informational) |
| 1    | At least one warning — system will run, features degraded |
| 2    | At least one error — system will not work until fixed |

The 21 rows of `CHECK_FAMILIES`, by submodule:

| Submodule     | Check families (`CHECK_FAMILIES` rows)                                |
| ------------- | --------------------------------------------------------------------- |
| `environment` | `check_runtime_environment` (CPU cores, temp directory writability); `check_egress` (what the station contacts on its own initiative); `check_optional_tools` (`ffmpeg`/`sox` present when non-WAV output is selected; `apprise` when Apprise file-config is used) |
| `config`      | `check_config_file` (file parses); `check_config_values` (every value in range — delegates to the validator); `check_station_location`; `check_occurrence_filter`; `check_confirmation_filter`; `check_listen_address` (parses as a socket address); `check_admin_exposure`; `check_api_surface` |
| `tls`         | `check_tls` (is the dashboard encrypted, and can it start that way)  |
| `clock`       | `check_clock` (system-clock and timezone sanity)                      |
| `database`    | `check_database` (directory writable, file integrity — delegates to `birdnet-db`) |
| `offsite`     | `check_offsite` (offsite backup configuration; deliberately makes no connection) |
| `paths`       | `check_paths` (recordings directory and image-cache directory)        |
| `audio`       | `check_audio_source` (exactly one source configured, ALSA card present via `arecord -l`, PulseAudio source listed via `pactl list short sources`, RTSP host TCP-reachable on port 554 with a 3 s timeout) |
| `model`       | `check_model` (file exists and ≥ 1 MB; labels file exists when configured) |
| `analytics`   | `check_analytics` (will the behavioral-analytics dashboards work — without opening DuckDB) |
| `disk`        | `check_disk_space` (free GiB, via the shared `disk_usage` helper)     |
| `watchdog`    | `check_systemd_watchdog` (is the supervisor honouring the `sd_notify` watchdog the daemon advertises) |

### 3. Browser (`/admin/doctor`, `/admin/doctor.json`, `/admin/support-bundle`)

An operator with only a browser gets the same report: `GET /admin/doctor`
renders the full `--doctor` output, `GET /admin/doctor.json` returns the
`--doctor-json` document, and `GET /admin/support-bundle` downloads the
support bundle. `birdnet-web` reaches the binary's checks through
`birdnet_web::diagnostics::Diagnostics` hooks, installed by `src/app.rs`
from `helpers::diagnostics::hooks`. These routes are read-only: a `GET`
never implies `--fix`. Gates:
`crates/birdnet-web/tests/the_diagnostics_are_reachable_from_the_browser.rs`,
`tests/the_station_can_diagnose_itself_from_a_browser.rs`.

## Non-goals

- **Unrequested repair.** Diagnostics report and suggest. The only mutation
  is the opt-in `--fix` (`src/doctor/fix.rs`), which creates missing
  configured directories (`create_dir_all`) and nothing else; it runs before
  the checks so the report shows the healed state. Operators stay in control
  of their install.
- **Continuous health monitoring.** That is the web `/api/v2/health`
  endpoint's job; preflight is one-shot.
- **Full RTSP handshake.** Replicating ffmpeg's RTSP/TCP/UDP/SETUP/PLAY
  dance would double the dependency surface for marginal extra signal.
  TCP-connect probes catch the overwhelmingly common failure modes
  (typo, wrong port, host unreachable).

## Failure modes the design accepts

- A check that needs an external tool (`arecord`, `pactl`) on a system
  without that tool returns `[ SKIP ]`, not `[ FAIL ]`. The operator can
  still proceed; they have just lost one verification surface.
- A check that needs network (RTSP probe) on an offline host returns
  `[ WARN ]`, not `[ FAIL ]`, because intermittent connectivity is the
  normal state for many home networks. Logging happens at `INFO` level
  so users see what was tried.
- Disk-space probing delegates to `birdnet_core::audio::capture::disk_usage`
  (which runs POSIX `df -Pk -- PATH`), the same helper the capture disk
  manager uses — there used to be two `df` parsers and they had drifted. If
  `df` is missing the check is `[ SKIP ]` and the diagnostic continues.

## Alternatives considered

- **A long-running supervisor that surfaces problems via the web UI.**
  Rejected because it duplicates `/api/v2/health` and requires the web
  server to already be up — exactly the situation that breaks in the
  field. Preflight has to work *before* anything else does. (The on-demand
  `/admin/doctor` page is not that: it runs the same one-shot checks when
  asked, once the server is up.)
- **A `libc::statvfs` FFI for disk-free.** Rejected because the workspace
  policy denies `unsafe_code`. Shelling out to `df` (through the one shared
  `disk_usage` helper) matches the existing pattern (the audio pipeline
  already shells out to `ffmpeg`/`arecord`) and removes a maintenance surface.
- **Per-check JSON output.** Shipped as `--doctor-json` (and
  `/admin/doctor.json`); see *Shipped extensions* below.

## Operating envelope

| Property              | Guarantee                                                   |
| --------------------- | ----------------------------------------------------------- |
| Runtime               | Bounded by network timeouts (≤ 3 s × number of audio sources) |
| Filesystem writes     | Only the `.birdnet-doctor-write-probe` zero-byte file, immediately deleted |
| Network egress        | Only to configured RTSP hosts (TCP connect, no data sent); the offsite check deliberately makes no connection |
| Subprocess execution  | `arecord -l`, `pactl list short sources`, `df -Pk -- PATH` (via `disk_usage`) |
| Side-effects on state | None without `--fix`; with `--fix`, missing configured directories are created |
| Panic surface         | Property-tested against arbitrary string inputs              |

## Future extensions

- Hardware-temperature check on Raspberry Pi (read
  `/sys/class/thermal/thermal_zone*/temp`).
- ONNX model SHA-256 verification once a canonical hash is published.
- Recent-detection sanity check: warn if the database has no detection
  rows newer than the audio-source's expected duty cycle.

### Shipped extensions

- **`--doctor-json`** — emits the same check results as a single-line JSON
  object so monitoring integrations (Nagios, Zabbix, Home Assistant
  command sensors, Prometheus textfile collector) can consume them
  directly. Same exit-code semantics as `--doctor`. String escaping is
  hand-rolled per RFC 8259 §7 to keep the binary's diagnostic surface
  free of macro magic.
- **Snapshot tests** for the text-mode report. The render is split into
  a pure `render_text(&[Check]) -> String` function; four golden files
  under `src/testdata/doctor_snapshots/` pin the exact bytes of the
  output for representative configurations (all-pass, mixed
  warnings/skips, with errors, empty). Updating the snapshots requires
  `UPDATE_DOCTOR_SNAPSHOTS=1 cargo test`, which forces the change to go
  through a PR review.
- **Mutation testing** for the configuration validator via
  `cargo-mutants` in `.github/workflows/mutation.yml`. It is a per-file
  matrix — `config/validate.rs` (two shards), `inference/model.rs`,
  `audio/extraction/extractor.rs`, `audio/extraction/convert.rs`,
  `civil.rs` (three shards), `birdnet-db`'s `migration.rs` (two) and
  `sqlite/queries/detections/*.rs` (four), and the binary's
  `src/capture/schedule.rs` (three), `src/capture/supervisor.rs` (three)
  and `src/daemon/*.rs` (five) — so a survivor in one file does not tank
  the whole pipeline, and each file is its own job with its own runtime
  budget. The threshold is
  `max_missed: 0` on **every** row: a surviving mutant always means an
  assertion is too weak, and the rule is to refactor the source until
  the boundary is observable rather than to lift the threshold.
- **Coverage measurement** via `cargo-llvm-cov` in
  `.github/workflows/coverage.yml`. Posts a sticky summary comment on
  PRs, uploads HTML + lcov artifacts, and optionally pushes to Codecov
  when `CODECOV_TOKEN` is set. Excludes `crates/birdnet-migrate` (legacy
  surface), `crates/birdnet-behavioral` and `tests/` from the per-file table
  (`--ignore-filename-regex`) to keep the PR comment focused.
