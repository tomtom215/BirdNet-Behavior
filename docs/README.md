# Documentation map

Where each kind of question is answered, and how much to trust each answer.

| If you are… | Read | What it is |
|---|---|---|
| running or using a station | [`book/`](book/) — **the manual** | User documentation. Kept true by gates (links, sample output, retired claims). The only part of `docs/` that ships in a release |
| working on the code | [`architecture/`](architecture/), [`../CLAUDE.md`](../CLAUDE.md) | Architecture: how it is built and why. Design notes, partly older than the code; `architecture/13-implementation-status.md` says how to check what is current |
| deciding what to build | [`design/`](design/), [`FEATURE_GAP_ANALYSIS.md`](FEATURE_GAP_ANALYSIS.md), [`TECH_DEBT.md`](TECH_DEBT.md) | Proposals, comparisons and known debt. **Nothing in a proposal is implemented** unless the file says it has landed; `design/charts.md` and `design/charts-relational.md` are the exceptions, the rules of charts that are built |
| asking what happened before | [`archive/`](archive/) | Dated working records of earlier sessions. Not current documentation |

## Using and running a station

- **The manual** — [`book/`](book/), an mdBook. Install, configure, operate, the
  field guides, the API and integration reference. It is published at
  `https://tomtom215.github.io/BirdNet-Behavior/` and served by the binary itself
  at `/help`. How its versions work is in the next section.
- [`../README.md`](../README.md) — what it is, quick start, requirements.
- [The manual's troubleshooting guide](book/guides/troubleshooting.md) is the one
  troubleshooting page; [`../TROUBLESHOOTING.md`](../TROUBLESHOOTING.md) only points
  to it. [`../SUPPORT.md`](../SUPPORT.md) says how to ask for help.
- [`grafana-dashboard.json`](grafana-dashboard.json) — a dashboard for the
  Prometheus series.

### How the manual is versioned

`scripts/build-versioned-docs.sh` builds the whole site on every run, and
[`../RELEASING.md`](../RELEASING.md) describes what that does and does not
promise. Today:

| Address | What it is | When it changes |
|---|---|---|
| `…/BirdNet-Behavior/` (the root) | the manual of the **latest stable release** | when a release tag is pushed |
| `…/BirdNet-Behavior/vX.Y.Z/` | the manual of release `X.Y.Z`, built from that tag | rebuilt from its tag on every run; its words do not change |
| `…/BirdNet-Behavior/dev/` | the manual on `main`: **documents features that may not be released** | on a push to `main` that touches `docs/book/**`, the book theme, `book.toml` or the docs scripts (`docs.yml` lists the paths) |
| `/help` in the binary | the manual that was in the tree when that binary was built | never; it is the manual for the binary serving it |

Only releases from `v0.17.1` on have a `/vX.Y.Z/` manual; for anything older the
binary's own `/help` is the manual. **Nothing marks an individual book page or
paragraph "since 0.x".** A feature the manual describes may be newer than the
release you run; the way to know is to read the manual that matches your version
(the version switcher, or `/help` on the station), and the
[changelog](../CHANGELOG.md), which says in which release each change landed.

The Markdown files at the repository's top level (`README.md`, `TROUBLESHOOTING.md`,
…) describe the tree they sit in, so they match the commit you are reading, not
the release you may have installed. Links from them to the book are
repository-relative for that reason.

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
- [`TECH_DEBT.md`](TECH_DEBT.md) — defects that are known, have evidence and were not
  fixed by the change that found them; a test reads it, and a row is deleted in the
  commit that fixes it.

## How it is built

- [`architecture/`](architecture/) — sixteen numbered chapters, from motivation
  to end-to-end testing standards. [`RUST_ARCHITECTURE_PLAN.md`](RUST_ARCHITECTURE_PLAN.md)
  is the short version.
- [`../CLAUDE.md`](../CLAUDE.md) — the conventions and the testing lessons the
  repository has paid for; read before changing a test.
- [`DESIGN_BRIEF.md`](DESIGN_BRIEF.md) — the interface's design brief, an
  internal working document for a design session, not user documentation.

## Proposals and comparisons

None of these describes software that exists.

- [`design/multi-station.md`](design/multi-station.md) — how several stations could
  write to one store. **Design only: nothing is built**, and the file ends with the
  decisions it needs first.
- [`design/tier-c-proposals.md`](design/tier-c-proposals.md) — five proposals that
  need a decision before any code; its own first lines say which precondition has
  since landed.
- [`design/handover/`](design/handover/) — the design handover the interface was
  built from, kept as provenance.
- [`FEATURE_GAP_ANALYSIS.md`](FEATURE_GAP_ANALYSIS.md) — what BirdNet-Behavior
  does not yet do that BirdNET-Pi and birdnet-go do, with the evidence per row;
  Part 10 says what is and is not at parity with BirdNET-Pi. The one *planning*
  document here that is still open.

## Records, not status

Each was written against a particular commit, and each says so in its own first
lines. Statuses inside them are not updated after the fact (the ones that were
say where); read them for what was found and why, not for what is open now.

- [`archive/`](archive/) holds five of them: the production, field-readiness,
  enclosure-readiness and post-0.14.0 audits, and the first release-readiness
  assessment. [`archive/README.md`](archive/README.md) says what each describes.
- Three more are still in `docs/` because source files outside this tree name their
  paths, and that README says which:
  [`RELEASE_PLAN.md`](RELEASE_PLAN.md) (the `v0.10.x` cycle, complete),
  [`RELEASE_PUNCHLIST.md`](RELEASE_PUNCHLIST.md) (superseded, 2026-05-29) and
  [`UNATTENDED_DEPLOYMENT_AUDIT.md`](UNATTENDED_DEPLOYMENT_AUDIT.md) ("what a year
  alone in a field would find"; its rows are updated in place).
