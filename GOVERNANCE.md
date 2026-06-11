# Governance

This document describes how decisions are made in BirdNet-Behavior.

## Roles

- **Contributor** — anyone who opens an issue, improves the docs, or submits
  a pull request. No sign-up beyond the
  [Code of Conduct](CODE_OF_CONDUCT.md) and the licensing terms in
  [CONTRIBUTING.md](CONTRIBUTING.md).
- **Maintainer** — sets technical direction, reviews and merges pull
  requests, cuts releases, and handles security reports per
  [SECURITY.md](SECURITY.md).

Current maintainer: Tom F. ([@tomtom215](https://github.com/tomtom215)).

## Decision process

- **Reversible changes** (bug fixes, docs, refactors that keep behavior)
  are decided in pull-request review.
- **Architectural changes** (new crates, storage layout, protocol surface,
  licensing-relevant code) need an issue first, describing the trade-offs,
  so the discussion outlives the PR that implements it. The architecture
  history lives in `docs/RUST_ARCHITECTURE_PLAN.md` and
  `docs/architecture/`.
- **Releases** follow [RELEASING.md](RELEASING.md): version, changelog,
  and tag move together, and CI gates (fmt, clippy pedantic+nursery, tests,
  MSRV, aarch64 cross-check, cargo-deny) must be green.

## Relationship to upstream

BirdNet-Behavior is a clean rewrite derived from BirdNET-Pi and uses the
BirdNET model published by the K. Lisa Yang Center for Conservation
Bioacoustics (Cornell) and Chemnitz University of Technology. We track the
upstream model licensing (CC BY-NC-SA 4.0) and do not relicense it; issues
about model behavior itself belong upstream with
[BirdNET](https://github.com/birdnet-team), not here.

## Code of Conduct

The [Code of Conduct](CODE_OF_CONDUCT.md) applies in every project space.
The maintainer is the enforcement contact.
