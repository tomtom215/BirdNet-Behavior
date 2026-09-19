# BirdNet-Behavior Repository Reference

**A Rust rewrite of BirdNET-Pi with DuckDB behavioral analytics.**

BirdNet-Behavior is a real-time acoustic bird classification system targeting
Raspberry Pi (5, 4B, 400) and x86_64 Linux, built as a single Rust binary.
It integrates [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral)
for bird activity analytics (sessionization, retention, funnel analysis, sequence matching).

## Lineage & Attribution

This project is derived from BirdNET-Pi (CC BY-NC-SA 4.0):
- **BirdNET**: K. Lisa Yang Center for Conservation Bioacoustics, Cornell University
- **BirdNET-Pi**: Patrick McGuire (mcguirepr89)
- **BirdNET-Pi fork**: Nachtzuster
- **BirdNET-Pi fork**: tomtom215

See `LICENSE` and `LICENSE-UPSTREAM` for full attribution and license terms.

## Architecture

Single Rust binary with 8 workspace crates:

| Crate | Purpose |
|-------|---------|
| `birdnet-core` | Audio capture, decode, resample, mel spectrogram, ML inference, detection pipeline, tmpfs, live spectrogram |
| `birdnet-db` | SQLite (OLTP) + DuckDB (OLAP), resilience, migrations |
| `birdnet-web` | axum web server, REST API, WebSocket, HTMX templates, audio player, admin |
| `birdnet-integrations` | BirdWeather, Apprise, MQTT (Home Assistant discovery), Wikipedia images, email, heartbeat, weekly reports, auto-update |
| `birdnet-behavioral` | DuckDB behavioral analytics for bird activity patterns |
| `birdnet-timeseries` | Time-series analytics (activity, diversity, trend, peak, gap, sessions) |
| `birdnet-migrate` | BirdNET-Pi migration: schema detection, validation, import |
| `birdnet-scheduler` | Solar calculations, recording window scheduling |

See `docs/RUST_ARCHITECTURE_PLAN.md` for the full phased implementation plan.

## Quick Reference

### Build & Test

```bash
# Build (debug)
cargo build

# Build (release, optimized for Pi deployment)
cargo build --release

# Run tests
cargo test

# Run with clippy lints
cargo clippy --workspace --all-targets

# Format check
cargo fmt --check --all
```

> **Cold build fails with `ort-sys ... invalid peer certificate: UnknownIssuer`?**
> You're behind a TLS-intercepting proxy that `ort`'s bundled rustls roots don't
> trust (sandboxed CI / Claude Code on the web). Run `scripts/setup-onnxruntime.sh`
> once to seed the ONNX Runtime download cache via `curl`; builds then work
> offline. Web sessions run this automatically via the SessionStart hook.

### Cross-compilation (for Raspberry Pi)

```bash
# Install target
rustup target add aarch64-unknown-linux-gnu

# Build with cross
cross build --release --target aarch64-unknown-linux-gnu
```

### Coding Conventions

- **No `anyhow`/`thiserror` in library crates** - hand-rolled error types
- **No async in the compute/storage library crates** (e.g. `birdnet-core`, `birdnet-db`) - they are synchronous. `birdnet-integrations` is the deliberate exception: an async *client* library for network I/O that constructs no runtime of its own
- **The tokio runtime is owned by application code** (`birdnet-web`, main binary) - library crates never start their own runtime
- **Blocking ops via `tokio::task::spawn_blocking`** for DB, file I/O, inference
- **`unsafe` is forbidden** workspace-wide (`unsafe_code = "forbid"`)
- **`missing_docs` enforced** workspace-wide
- **Clippy pedantic + nursery** enabled

### Testing Conventions

**A new gate must be observed failing against the code it was written for, and
the commit message must say how.**

Not "it should fail" — apply the old code, remove the fix, or mutate the
constant, and watch it go red. A test written after the fix and only ever seen
green proves nothing about what it would catch. This is cheap (one revert, one
`cargo test`) and it has repeatedly caught tests that were green for reasons
that had nothing to do with what they claimed to assert:

- a gate satisfied by a cache an earlier probe had populated
- a gate satisfied by test-execution ordering
- a gate asserting only the rejecting side of a boundary
- a preview query that agreed with its migration only by coincidence

When a gate covers a discrimination rather than a single behaviour, write the
counterpart too and check it stays green — otherwise a blanket alarm passes for
a discriminator.

Corollaries, each learned the same way:

- **Build the smallest thing that settles the question, and run it.** A scratch
  probe (`crates/*/tests/zz_*.rs`, or a `zz_probe_*` unit test, deleted before
  commit) beats any amount of reasoning about what the code does. Two of the
  facts in `--channel-report`'s own doc comments were wrong until one was run.
- **A test that passes tells you nothing until you know why it passes.**
- **Distrust confident prose in this repo's history, including your own.** The
  `load_icu()` comment asserted ICU was statically linked; it was not.
  `aligned_sum` was documented as summing; it averaged. Both misled the next
  reader.
- **`cmd 2>&1 | tail -N` masks the exit code.** A full workspace run has
  reported "exit 0" with two failures inside it. Grep for `^test result:` and
  `FAILED`, or check `PIPESTATUS`.
- **`~/.duckdb` contaminates ICU results.** One probe populates it and every
  ICU-related test then passes for free. `mv ~/.duckdb /tmp/duckdb-cache-backup`
  before trusting any of them.
- **A file restored with an older mtime does not trigger a rebuild.** A script
  that mutates a source file, runs `cargo test`, then moves a backup back over
  it leaves the source *older* than the artifact built from the mutant, and the
  next `cargo test` silently re-runs that build. This reported a brand-new gate
  as failing against the code it guards and then as failing again without it —
  the second result was the first one's binary. `touch` the file after
  restoring it, and check a new gate passes *before* trusting that it fails.
- **A killed in-place `cargo mutants` run can leave its mutant in the tree.**
  `--in-place` (what `mutation.yml` uses) rewrites the source file for each
  mutant and restores it afterwards; a run killed mid-mutant, or a child of it
  that outlives the kill, leaves the file mutated — and `git add -A` commits it.
  A commit on PR #239 shipped `t <` for `t >=` with the tool's own
  `~ changed by cargo-mutants ~` marker in the line. Before staging anything
  while such a run may be alive: `pgrep -f 'cargo.mutants'` must be empty and
  `grep -rn "changed by cargo-mutants" src crates` must find nothing.
- **`pull_request`-triggered gates never see un-PR'd branches.** Open a draft PR
  early, or run the gates locally; work has sat broken on a pushed branch for
  hours because nothing was watching it.
- **`pkill -x screenshot_server` silently matches nothing.** `pgrep`/`pkill`
  will not match `-x` against a process name longer than 15 characters, and it
  exits 1 rather than complaining. A restart script built on it left the old
  binary holding port 8502 while `cargo run` rebuilt, failed to bind and
  exited — so three rounds of "verified in the browser" were answered by the
  *previous* build's HTML. Kill by full command line (`pgrep -f '[s]creenshot_server'`),
  and have the restart refuse to report success when the running process
  predates the binary it just built:
  `ps -o lstart= -p "$PID"` against `stat -c %Y "$BIN"`.
- **A green a11y gate can mean the rule never ran.** `axe.mjs` gates on a tag
  list, and with only the four WCAG A/AA tags, 36 of axe's 105 rules do not
  execute at all — `heading-order`, `landmark-one-main`, `region`,
  `page-has-heading-one`, `target-size` and the rest. Adding them reported 40
  findings on the first run. The same applies to *reach*: the gate ran at
  1280x900 only, so the phone layout had never been graded, and adding it found
  six serious violations immediately. When a gate is clean, ask what it covers
  before believing it.
- **A media query does not raise specificity** (CSS Cascade L4 §6.4.4). Any
  equal-specificity rule *later* in the sheet defeats it, and an ID-scoped rule
  defeats it wherever it sits. Three responsive overrides in `app.css` were
  dead for one of those two reasons, including the one that left the Today
  feed's species-name column at exactly `0px` on a phone. Confirm a responsive
  rule with `getComputedStyle` at the target width; reading the CSS will not
  tell you.
- **`page.evaluate` is not subject to the page's CSP.** Playwright runs it
  through CDP, which is exempt, so it cannot answer "is `eval` blocked here?"
  — it will cheerfully report that `new Function` works on a page where it does
  not. Test CSP behaviour by driving the real code path and listening for
  `securitypolicyviolation`.
- **An accessibility sweep grades the state the page happened to be in.**
  `axe.mjs` loads a route and reads it once. Any state a script reaches at
  runtime — a socket retrying, a button mid-flight, a form after a failed
  submit — is sampled by luck. The live-stream pill's reconnect state carried
  `opacity: .6`, which composites text toward the page: 3.07:1 in light,
  4.24:1 in dark, at 11px, against the 4.5:1 that WCAG 1.4.3 asks. The sweep
  found it on exactly one route in one theme in one run, because the socket
  happened to be retrying when that page was sampled; every other run was
  clean, and the finding looked like noise from a server restart. When a
  state is only reachable through JS, either drive it in the gate or check it
  mechanically without a browser — do not let a green sweep stand for it.
- **A value validated at startup is not a value validated.** `validate()` ran
  on the config file at boot and `overlay_db_settings` laid the settings table
  over the result afterwards, so nothing ever checked a number typed into
  `/admin/settings`. `confidence_threshold=75` — the percentage slip, in a
  field labelled "(0–1)" — stored cleanly and stopped the station detecting
  for good, while `--doctor` reported the file as fine. Validate at the
  boundary the value actually crosses. Where two places need the same bounds,
  give them one table to read (`validate::NUMERIC_RANGES`) — but check what a
  finding *does* before widening the validator: an `Error` makes `is_usable`
  false, and `startup_config::choose` then reverts the station to its last-good
  configuration file. Adding two alert thresholds to that list would have
  turned a mistyped notification threshold into a silent rollback of every
  other setting, so the form bounds those two itself and a test holds the two
  copies of the shared five equal.
- **A filter that skips unchanged fields skips the broken station.**
  `build_settings_items` drops any field whose submitted value equals the
  stored one, so a check driven by its output waves through a bad value that
  is *already* in the database — precisely the station whose owner is on the
  settings page because the birds stopped. Validate the submission, not the
  diff.
- **`page.contains("…")` can be satisfied by the template's own comment.**
  `login.html` opens with a comment documenting its placeholders, quoting
  "Incorrect username or password." verbatim, and that comment ships to the
  browser. A test asserting the page did *not* contain that string failed
  against a correct fix. Assert on the rendered region, not the document.
- **`cargo check` does not build test cfg.** A struct field added for a page
  render compiled clean and broke three `#[cfg(test)]` constructors in the
  same file. Use `cargo check --all-targets`.
- **`No space left on device` surfaces as unrelated test failures.** A full
  disk inside a `cc-rs` build script reported as four failing `birdnet-behavioral`
  ICU/extension tests, in a crate that had not been touched. `df -h /` reads
  "Avail 1.9M" with "Used 38G" — the allowance is spent, not the machine. The
  cheapest ~3–6 GB back is `rm -rf target/debug/incremental`.

### Key Dependencies

| Purpose | Crate |
|---------|-------|
| Async runtime | `tokio` |
| Web framework | `axum` |
| SQLite | `rusqlite` (bundled) |
| Audio decode | `symphonia` |
| Resampling | `rubato` |
| ML inference | `ort` (ONNX Runtime) |
| File watching | `notify` |
| Logging | `tracing` |

### MSRV

Rust 1.95 (edition 2024)
