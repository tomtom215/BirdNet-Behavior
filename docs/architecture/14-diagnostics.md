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

Two complementary mechanisms ship together:

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

### 2. Preflight subcommand (`birdnet-behavior --doctor` / `--preflight`)

Runs ~12 independent checks against the live environment and prints a
one-screen report. Each check produces a `Check { status, name, message,
remediation? }`. The CLI exits with the worst-severity-derived code:

| Exit | Meaning                                              |
| ---- | ---------------------------------------------------- |
| 0    | All checks passed (some may be skipped/informational) |
| 1    | At least one warning — system will run, features degraded |
| 2    | At least one error — system will not work until fixed |

Checks included today:

| Group        | Check                                                            |
| ------------ | ---------------------------------------------------------------- |
| Runtime      | CPU cores, temp directory writability                            |
| Configuration | File parses, every value in range (delegates to validator)      |
| Web          | Listen address parses as a socket address                        |
| Database     | Directory writable, file integrity (delegates to `birdnet-db`)   |
| Filesystem   | Recordings dir writable, image cache dir writable, disk free GiB |
| Audio        | Exactly one source configured, ALSA card present (via `arecord -l`), PulseAudio source listed (`pactl list short sources`), RTSP host TCP-reachable on port 554 (3 s timeout) |
| Model        | File exists and ≥ 1 MB; labels file exists when configured       |
| Tooling      | `ffmpeg`/`sox` present when non-WAV output is selected; `apprise` present when Apprise file-config is used |

## Non-goals

- **Auto-repair.** Diagnostics report and suggest; they never mutate state.
  Operators stay in control of their install.
- **Continuous health monitoring.** That is the web `/healthz` endpoint's
  job; preflight is one-shot.
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
- Disk-space probing shells out to `df -Pk`. If `df` is missing the
  check is `[ SKIP ]` and the diagnostic continues.

## Alternatives considered

- **A long-running supervisor that surfaces problems via the web UI.**
  Rejected for now because it duplicates `/healthz` and requires the web
  server to already be up — exactly the situation that breaks in the
  field. Preflight has to work *before* anything else does.
- **A `libc::statvfs` FFI for disk-free.** Rejected because the workspace
  policy denies `unsafe_code`. Shelling out to `df` matches the existing
  pattern (the audio pipeline already shells out to `ffmpeg`/`arecord`)
  and removes a maintenance surface.
- **Per-check JSON output.** A worthwhile follow-up but not in this slice
  — the current text format is grep-friendly and the exit code already
  provides machine-readable summary status.

## Operating envelope

| Property              | Guarantee                                                   |
| --------------------- | ----------------------------------------------------------- |
| Runtime               | Bounded by network timeouts (≤ 3 s × number of audio sources) |
| Filesystem writes     | Only the `.birdnet-doctor-write-probe` zero-byte file, immediately deleted |
| Network egress        | Only to configured RTSP hosts (TCP connect, no data sent)    |
| Subprocess execution  | `arecord -l`, `pactl list short sources`, `df -Pk PATH`     |
| Side-effects on state | None                                                        |
| Panic surface         | Property-tested against arbitrary string inputs              |

## Future extensions

- Optional `--doctor --json` mode for monitoring integrations.
- Hardware-temperature check on Raspberry Pi (read
  `/sys/class/thermal/thermal_zone*/temp`).
- ONNX model SHA-256 verification once a canonical hash is published.
- Recent-detection sanity check: warn if the database has no detection
  rows newer than the audio-source's expected duty cycle.
