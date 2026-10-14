# Documentation map

Where each kind of question is answered. The first group is current and kept
true by gates; the last is a set of point-in-time audits that stay as records.

## Using and running a station

- **The manual** — [`book/`](book/), an mdBook, published for every release at
  `https://tomtom215.github.io/BirdNet-Behavior/` and at `/vX.Y.Z/` for each
  version, and served by the binary itself at `/help`. Install, configure,
  operate, field guides, the API and integration reference.
- [`../README.md`](../README.md) — what it is, quick start, requirements.
- [`../TROUBLESHOOTING.md`](../TROUBLESHOOTING.md), [`../SUPPORT.md`](../SUPPORT.md).
- [`grafana-dashboard.json`](grafana-dashboard.json) — a dashboard for the
  Prometheus series.

## What the project promises, and how it is run

- [`../CHANGELOG.md`](../CHANGELOG.md) — every change, per release.
- [`../STABILITY.md`](../STABILITY.md) — what a release may and may not change
  (proposed; see [`HANDOFF.md`](HANDOFF.md)).
- [`../RELEASING.md`](../RELEASING.md) — the release runbook, the shared model
  release, and what to update when DuckDB, ONNX Runtime or an extension moves.
- [`../SECURITY.md`](../SECURITY.md), [`../GOVERNANCE.md`](../GOVERNANCE.md),
  [`../CONTRIBUTING.md`](../CONTRIBUTING.md).
- [`../NOTICE`](../NOTICE) and [`../THIRD_PARTY_LICENSES.txt`](../THIRD_PARTY_LICENSES.txt)
  — other people's work in a build, and its licences.
- [`../.github/workflows/README.md`](../.github/workflows/README.md) — every
  workflow, and the checks `main` should require.
- [`HANDOFF.md`](HANDOFF.md) — what only the maintainer can do, and where the
  next working session starts.

## How it is built

- [`architecture/`](architecture/) — sixteen numbered chapters, from motivation
  to end-to-end testing standards. [`RUST_ARCHITECTURE_PLAN.md`](RUST_ARCHITECTURE_PLAN.md)
  is the short version.
- [`DESIGN_BRIEF.md`](DESIGN_BRIEF.md) and [`design/`](design/) — the interface's
  design brief, and the design handover and proposals.
- [`../CLAUDE.md`](../CLAUDE.md) — the conventions and the testing lessons the
  repository has paid for; read before changing a test.
- [`FEATURE_GAP_ANALYSIS.md`](FEATURE_GAP_ANALYSIS.md) — what BirdNet-Behavior
  does not yet do that BirdNET-Pi and birdnet-go do. The one *planning* document
  here that is still open.

## Audits that are records, not status

Each was written against a particular commit, and each says so in its own first
lines. Statuses inside them are not updated after the fact (the ones that were
say where); read them for what was found and why, not for what is open now.

| Document | Written for |
|---|---|
| [`PRODUCTION_AUDIT.md`](PRODUCTION_AUDIT.md) | permanent unattended deployment, 2026-08-17; superseded by the two below |
| [`FIELD_READINESS_AUDIT.md`](FIELD_READINESS_AUDIT.md) | 2026-08-19 |
| [`ENCLOSURE_READINESS_AUDIT.md`](ENCLOSURE_READINESS_AUDIT.md) | 2026-08-26, from the outside in |
| [`POST_0140_AUDIT.md`](POST_0140_AUDIT.md) | after 0.14.0; all twenty-five findings fixed |
| [`UNATTENDED_DEPLOYMENT_AUDIT.md`](UNATTENDED_DEPLOYMENT_AUDIT.md) | "what a year alone in a field would find"; rows updated in place |
| [`RELEASE_PLAN.md`](RELEASE_PLAN.md) | the `v0.10.x` cycle, complete |
| [`RELEASE_PUNCHLIST.md`](RELEASE_PUNCHLIST.md) | superseded, 2026-05-29 |
| [`RELEASE_READINESS.md`](RELEASE_READINESS.md) | superseded, 2026-06-03 |
