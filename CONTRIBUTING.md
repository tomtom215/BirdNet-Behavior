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
2. **Build and test** before opening the PR:
   ```bash
   cargo build --workspace
   cargo test --workspace
   cargo clippy --workspace --all-targets  # no warnings allowed
   cargo fmt --check --all
   ```
3. **Keep PRs focused** — one fix or feature per PR makes review faster
4. **Update the README** if you're adding or changing user-visible behaviour

## Code conventions

Documented in [`CLAUDE.md`](CLAUDE.md). The short version:

- No `anyhow`/`thiserror` in library crates — hand-rolled error types
- No async in library crates (`birdnet-core`, `birdnet-db`) — blocking only
- `unsafe` is denied workspace-wide
- Clippy pedantic + nursery, warnings denied in CI

## License

By opening a PR you agree that your contribution will be licensed under the same [CC BY-NC-SA 4.0](LICENSE) terms as the rest of the project.
