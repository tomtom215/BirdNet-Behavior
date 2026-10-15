# Stability policy

**Status: proposed 2026-10-14.** This describes what the project already does and
what it is willing to commit to; it is the maintainer's to accept, amend or drop
(see [docs/HANDOFF.md](docs/HANDOFF.md)). Until then it is a description, not a promise.
[RELEASING.md](RELEASING.md) is the release runbook and [CHANGELOG.md](CHANGELOG.md)
is where every change is announced.

## 1. Where the project is

BirdNet-Behavior is at `0.17.x`. Between `0.1.0` (2026-04-12) and `0.17.1`
(2026-10-04) there were 24 releases, eleven of them from `0.10.0` (2026-08-07) on. That cadence
suits a station whose owners want fixes quickly; it is hard on anyone who has to
keep an integration compiling. This policy says what a release may and may not
change so that the cadence can stay.

The version follows [Semantic Versioning 2.0.0](https://semver.org/). While the
major version is `0`, a **minor** release (`0.17` → `0.18`) may change anything
listed in §3 as breaking, and a **patch** release (`0.17.1` → `0.17.2`) may not.

## 2. What is versioned

An operator or an integrator can rely on these, in this order of seriousness:

| Surface | Where it is defined | Today |
|---|---|---|
| **Station data** — the SQLite database and its migrations | `crates/birdnet-db/src/migration.rs` | 53 migrations, contiguous from 1; forward-only |
| **Configuration** — `birdnet.conf` keys, `BIRDNET_*` / `BNB_*` environment variables, command-line flags | `crates/birdnet-core/src/config/known_keys.rs`, `src/cli.rs`, `--help` | 147 file keys; 111 option lines in `--help` |
| **HTTP API** — `/api/v2`, bearer-token and public | `crates/birdnet-web/openapi.json`, [`docs/book/reference/api.md`](docs/book/reference/api.md) | 85 paths, 87 operations |
| **Metrics and messages** — Prometheus series, MQTT topics and Home Assistant discovery payloads, webhook bodies | [`docs/book/reference/integrations.md`](docs/book/reference/integrations.md) | `birdnet_*` series |
| **Release artefacts and installer** — tarball names, `SHA256SUMS`, `install.sh` flags, the container image tags | [RELEASING.md](RELEASING.md) | |
| **Supported platforms** | [README](README.md#supported-hardware--os) | see §6 |

The web pages (HTML, CSS, HTMX partials), the log lines and the wording of
messages are **not** a stable surface: they change to be clearer. The
accessibility and visual gates in CI are what hold them to a standard, not a
compatibility promise.

The workspace's library crates (`birdnet-core`, `birdnet-db`, …) carry **no** API
promise. They are packaged so that the binary can be installed from crates.io if
the maintainer publishes it (nothing has been published; see
[RELEASING.md](RELEASING.md)); each says so in its README, and their versions move
in lockstep with the binary's. If a crate is ever offered as a library, this
section gains a row and `cargo-semver-checks` becomes a CI gate.

## 3. What counts as breaking

Each of these is a minor bump, never a patch:

- **Station data.** A migration that cannot be applied to a database any earlier
  release produced, or that discards data. (Migrations never are, see §4.)
- **Configuration.** Removing or renaming a key, a variable or a flag; changing
  what a value means; changing a default so that an unchanged station behaves
  differently without being told.
- **HTTP API.** Removing or renaming a path, a parameter or a response field;
  narrowing a field's type; changing a status code a client could branch on;
  changing which paths need a token. Adding a field, a path or an optional
  parameter is not breaking: clients must ignore fields they do not know.
- **Metrics and messages.** Renaming or removing a series, a label, a topic or a
  payload field.
- **Artefacts.** Renaming a tarball, dropping a platform in §6, or changing what
  `install.sh` does to a station that already works.

A change that fixes an **observed defect** — the page said `0.000000` for a
coordinate it did not know; a limit was read from the wrong file — may ship in a
patch release even when someone's workaround stops working. The changelog says
which, and why.

## 4. How a breaking change is introduced

1. **It is announced where it can be found.** The changelog entry is under
   **Changed** or **Removed** and begins with the word *Breaking*, names the
   surface, and says what to do instead.
2. **Deprecate first where that is possible.** A key, flag, variable or field
   that is going away keeps working for at least one minor release and says so —
   at startup in the journal for configuration, in the response for the API —
   before it is removed. Where keeping the old form is not possible (a bug the old
   form depends on), the changelog says that instead.
3. **Batch.** Breaking changes are collected into one minor release rather than
   spread over several, so an operator has one upgrade to read.
4. **Data is never the thing that breaks.** Migrations are forward-only, are
   numbered once and never edited after release, and `--check-db` and the daily
   integrity check will say if one did not apply. There is **no downgrade**: to go
   back, restore the backup the upgrade took. Every release's upgrade path from the
   previous one is exercised on a real systemd host (`installer/test/upgrade-e2e.sh`,
   the `upgrade` workflow) before it is published.

## 5. Minimum supported Rust

`workspace.package.rust-version` is the MSRV (`1.95`). It is raised only in a
minor release and the changelog says so. CI compiles the whole workspace, tests
included, with exactly that compiler (the `msrv` job asserts the compiler it ran
is the declared one — it once did not).

## 6. Supported platforms

| Platform | Supported as |
|---|---|
| Raspberry Pi 4, 5, 400 — 64-bit Pi OS **Trixie** (glibc ≥ 2.39) | the reference deployment; release binary and installer |
| x86_64 Linux, glibc ≥ 2.39 | release binary and installer |
| Docker (linux/amd64, linux/arm64) | the image, including for Pi OS **Bookworm**, whose glibc is too old for the binary |
| macOS, Apple silicon | **preview**: a release binary is built, but the macOS paths are not yet confirmed on hardware ([the macOS guide](docs/book/field/macos.md) says so); no compatibility promise |

Not built or tested: 32-bit ARM (no ONNX Runtime build exists for it), musl,
Windows. Dropping a supported row (not the preview) is a breaking change.

## 7. Models

The classifier and the occurrence model are pinned by SHA-256 in
`installer/lib/10-config.sh` and refused on mismatch. Changing the default
classifier changes what a station detects and how its confidences read, so it is
a minor release, announced in the changelog with the before and after on the
bundled test recording.

## 8. Security fixes

Security fixes are made on the latest minor release and on `main`
([SECURITY.md](SECURITY.md)). A fix may be a patch release even when it changes
behaviour that someone relied on.

## 9. What would make this `1.0`

Nothing in the list below is promised for a date; each is a measurable condition.

- Two consecutive minor releases with no breaking change, by §3.
- The HTTP API described by `openapi.json` in both directions — every routed path
  documented, every documented path routed — and that checked in CI against the
  previous release's file, so a breaking change cannot ship unlabelled.
- The upgrade path exercised from every release in the supported window.
- Branch protection and a required-checks list on `main` (a repository setting,
  recorded in `docs/HANDOFF.md`).
