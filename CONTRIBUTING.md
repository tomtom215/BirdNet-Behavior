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
2. **Build and test** before opening the PR — the local quality gates
   mirror what CI enforces:
   ```bash
   cargo build --workspace
   cargo test --workspace
   cargo clippy --workspace --all-targets -- -D warnings
   cargo fmt --check --all
   ```
3. **Supply-chain & licence checks** (when you've touched `Cargo.toml`):
   ```bash
   cargo install --locked cargo-deny    # one-time
   cargo deny check                     # licences, advisories, bans, sources
   cargo install --locked cargo-audit   # one-time
   cargo audit --deny warnings          # advisory database
   ```
4. **Diagnostic smoke test** — if you've touched anything that affects
   startup, config parsing, or the audio/model surface, run the doctor
   against a known-good config and a deliberately broken one:
   ```bash
   birdnet-behavior --doctor --config /etc/birdnet/birdnet.conf
   ```
5. **Keep PRs focused** — one fix or feature per PR makes review faster
6. **Update the README, `.env.example`, and `CHANGELOG.md`** if you're
   adding or changing user-visible behaviour
7. **Add or update an ADR** under `docs/architecture/` for non-trivial
   design decisions (use the format of the existing `14-diagnostics.md`)
8. **Editing the installer?** `install.sh` is **generated** — never edit it by
   hand. Edit the single-responsibility modules under `installer/lib/*.sh` and
   regenerate it:
   ```bash
   installer/build.sh            # regenerate install.sh
   installer/build.sh --check    # verify it's in sync (CI + pre-commit gate)
   shellcheck -S warning -x install.sh
   ```

## Code conventions

Documented in [`CLAUDE.md`](CLAUDE.md). The short version:

- No `anyhow`/`thiserror` in library crates — hand-rolled error types
- No async in library crates (`birdnet-core`, `birdnet-db`) — blocking only
- `unsafe` is denied workspace-wide
- Clippy pedantic + nursery, warnings denied in CI
- `rust-toolchain.toml` pins the toolchain; do not bump it without also
  bumping the MSRV in `Cargo.toml` and the MSRV CI job

## License

By opening a PR you agree that your contribution will be licensed under the same [CC BY-NC-SA 4.0](LICENSE) terms as the rest of the project.
