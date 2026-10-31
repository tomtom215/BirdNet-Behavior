# Contributing to BirdNet-Behavior

Thanks for your interest in contributing. This document covers how to report bugs, suggest features, and submit code.

## Reporting bugs

Open an issue and include:

- What you did and what you expected to happen
- What actually happened (paste the relevant log lines from `sudo journalctl -u birdnet-behavior -f`)
- Your platform (`uname -m`, OS version, Rust version if building from source)

## Suggesting features

Open an issue with the `enhancement` label. Describe the use case — what problem you're trying to solve — rather than jumping straight to a proposed solution. That makes it easier to discuss alternatives.

## Submitting a pull request

1. **Fork and branch** — work on a branch named `feature/…` or `fix/…`
2. **Run `scripts/preflight.sh`** before every push. It runs the workflows' own
   commands in their own scope (format, clippy on all three feature sets,
   rustdoc with `-D warnings`, spelling, unused dependencies, `cargo deny`,
   `cargo audit`, OSV, the manual's links, the licence notices) and prints
   what it cannot cover (`--msrv` and `--tests` add the slow ones). A tool it
   needs and cannot find fails the run; the script prints the install line.
   The commands below are a subset of it.
3. **Build and test** — the local quality gates mirror what CI enforces:
   ```bash
   cargo build --workspace
   cargo test --workspace --all-features
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cargo fmt --check --all
   ```
   CI splits the test job into five runs (`--lib --bins`, `--tests`,
   `--doc`, `--all-features`, and the bundled-DuckDB extension check), so
   a green `--all-features` run locally is the closest single command.
4. **Supply-chain & licence checks** (when you've touched `Cargo.toml`):
   ```bash
   cargo install --locked cargo-deny    # one-time
   cargo deny check                     # licences, advisories, bans, sources
   cargo install --locked cargo-audit   # one-time
   cargo audit --deny warnings          # advisory database
   ```
5. **Diagnostic smoke test** — if you've touched anything that affects
   startup, config parsing, or the audio/model surface, run the doctor
   against a known-good config and a deliberately broken one:
   ```bash
   birdnet-behavior --doctor --config /etc/birdnet/birdnet.conf
   ```
6. **Keep PRs focused** — one fix or feature per PR makes review faster
7. **Update the README, `.env.example`, and `CHANGELOG.md`** if you're
   adding or changing user-visible behaviour
8. **Add or update an ADR** under `docs/architecture/` for non-trivial
   design decisions (use the format of the existing `14-diagnostics.md`)
9. **Editing the installer?** `install.sh` is **generated** — never edit it by
   hand. Edit the single-responsibility modules under `installer/lib/*.sh` and
   regenerate it:
   ```bash
   installer/build.sh            # regenerate install.sh
   installer/build.sh --check    # verify it's in sync (CI + pre-commit gate)
   shellcheck -S warning -x install.sh
   ```

## Getting a working tree

- **Prerequisites** (a fresh Debian/Ubuntu machine): `git`, [Rust 1.95 or newer](https://rustup.rs)
  (`rust-toolchain.toml` selects `stable` and `rustup` installs it on the first `cargo`
  command), and `cmake` and a C++ compiler (`sudo apt-get install cmake g++`) for the
  bundled DuckDB. `ffmpeg` is optional: CI installs it, and the tests that drive it skip
  when it is absent. The browser gates additionally need Node 20 or newer
  (`tools/visual-qa/README.md`). Then:
  ```bash
  git clone https://github.com/tomtom215/BirdNet-Behavior.git && cd BirdNet-Behavior
  export CARGO_INCREMENTAL=0
  cargo test -p birdnet-core --all-features --lib    # a first, small green run
  ```
- **A cold build takes about 12 minutes and several GB.** The workspace statically
  links ONNX Runtime and a bundled libduckdb (an rlib of about 480 MB), and every
  integration test is its own binary of 200-300 MB. Keep 20 GB free,
  `export CARGO_INCREMENTAL=0`, and select tests (`cargo test -p birdnet-core --lib`,
  `--test <name>`) rather than building `--all-targets` on a small disk. `df` can
  mislead on a quota; "No space left on device" shows up as unrelated build or
  test failures, so rerun at a healthy `df` before suspecting the code.
- **A cold build fails with `ort-sys ... UnknownIssuer`?** You are behind a
  TLS-intercepting proxy: run `scripts/setup-onnxruntime.sh` once.
- **The tests that need a real model** skip loudly unless you point them at one:
  `BIRDNET_TEST_MODEL` / `BIRDNET_TEST_LABELS` for BirdNET+ V3.0 (what
  `--install-model birdnet-v3` fetches), and `BIRDNET_TEST_V24_MODEL`,
  `BIRDNET_TEST_V24_LABELS`, `BIRDNET_TEST_V24_MDATA` for BirdNET V2.4 and its range
  filter (`--install-model birdnet-v24`, `birdnet-v24-range-v2`). Set
  `BIRDNET_REQUIRE_MODEL=1` / `BIRDNET_REQUIRE_V24_MODEL=1` to make a skip a
  failure, as CI does. Detection changes should be run against them; a green
  suite without them has not exercised the classifier.
- **The browser gates** (axe, interactions, legibility, base path, layout) run
  against a seeded fixture: see `tools/visual-qa/README.md`.
- **The BirdNET-Pi comparison** (`tools/parity`) regenerates its goldens with
  `tools/parity/regenerate.sh`; `docs/book/guides/parity.md` says what it covers.

## Testing conventions

The one that matters most: **a new gate must be observed failing against the code
it was written for, and the commit message must say how.** Revert the fix or
mutate the constant, run the test, watch it go red, restore, `touch` the file (an
older mtime does not trigger a rebuild), watch it go green. A test that was only
ever seen passing proves nothing about what it would catch. When a gate covers a
discrimination, write the counterpart too and check it stays green. The long form,
with the ways this has gone wrong, is in [`CLAUDE.md`](CLAUDE.md).

## Code conventions

Documented in [`CLAUDE.md`](CLAUDE.md). The short version:

- No `anyhow`/`thiserror` in library crates — hand-rolled error types
- No async in the compute/storage library crates (`birdnet-core`, `birdnet-db`) —
  blocking only. `birdnet-integrations` is the deliberate exception: an async
  network-client library that constructs no runtime of its own
- `unsafe` is forbidden workspace-wide (`unsafe_code = "forbid"`)
- `missing_docs` is enforced — every public item carries a rustdoc comment
- Clippy pedantic + nursery, warnings denied in CI
- No fixed-sleep synchronization in tests — signal readiness or poll a
  condition with a deadline (a `sleep` *is* the subject only when testing a
  timeout itself)
- `rust-toolchain.toml` tracks `channel = "stable"` (it pins components and
  profile, not a version). The MSRV lives in `Cargo.toml` `rust-version` and
  in the `msrv` job of `.github/workflows/ci.yml`, which fails if the two
  disagree — bump both together

## License

By opening a PR you agree that your contribution will be licensed under the same [CC BY-NC-SA 4.0](LICENSE) terms as the rest of the project.
