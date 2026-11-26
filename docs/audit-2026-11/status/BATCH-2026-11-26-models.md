# Fourth batch (2026-11-26): the station's models

Branch `claude/affectionate-turing-jfz87i`, from `main` at `4df20c3`. The promise the batch was held to: a
station identifies birds with the models its configuration names, out of the box; when one of them cannot be
used it keeps identifying with the rest, says so on every surface that claims to show health, and puts the
file right itself. The findings are the `MDL` rows of `checklist.json`, from `10-model-delivery.md`, which
records a real station broken one model file at a time on 2026-11-25.

The work was done in two phases of two lanes each, every lane a delegated session in its own worktree with
files no other lane touched, integrated here. "Lane run" below means the lane reported it and this session did
not repeat it; "session run" means this session ran it on the integrated tree.

## What changed

| Finding | Commit | Status |
|---|---|---|
| MDL-01, MDL-02, MDL-16 | `f91f96a`, `b64965a` | done: the detector starts on the files that load and publishes what it left out |
| MDL-07 | `f91f96a` | done |
| MDL-08, MDL-09, MDL-11, MDL-13, MDL-17 | `ece8963`, `be35e5f` and the catalogue lane's other commits | done: one install path, pinned size and sha256, resume, retries, timeouts, ordered origins |
| MDL-20 | the catalogue lane | done (documentation) |
| MDL-03, MDL-04, MDL-05, MDL-06 | `0a1432b` | done: `model_conditions` decides; badge, Station Health, Today, the notifier and `/api/v2/health` read it |
| MDL-15 | `0a1432b`, `ef9630d` | done |
| MDL-14 | `be35e5f`, `0a1432b`, `50bec0a` | done: a filter installed beside the classifier is adopted; `METADATA_MODEL_PATH=off` opts out |
| MDL-10 | `d53199b`, `296175c` | done: the station fetches a missing or damaged file again and restarts once to load it |
| MDL-21 | `3100422` | done: a stop writes its backlog to `clip-queue/` and exits; the next start replays it |
| MDL-19 | `f91f96a`, `0b3e753`, `ab0b1d0` | done: `serve` 1,051 lines to 96, `run_daemon` 568 to 46, `start_detection_daemon` split; the allow gate |
| MDL-18 | `f91f96a`, `0f9c5a9` | in_progress: the station harness and the systemd real-install check exist; the container half is not written |
| MDL-12 | `ece8963`, `6e13605` | in_progress: ordered origins everywhere; Perch's project mirror is written and not published |
| MDL-22, MDL-23 | | open |
| MDL-24 to MDL-28 | | new, open |

## Decisions taken in the batch

- **Strict health.** `/api/v2/health?strict=1` answers 503 only when nothing is identifying birds. A second
  classifier left out or unrouted, or a filter that is off, answers 200 with the reason in `model_conditions`.
- **Restart, not load in place, after a repair.** Starting the detector is the one path that wires the
  classifiers, the processor and the published state together. The repair asks for a restart only when a file
  this process saw broken now verifies and the running station is without it; a file that verifies and will
  not load therefore cannot cause a second restart.
- **Adoption of an installed filter.** The service cannot write its configuration file (`root`-owned, read-only
  to the service), so a filter installed from the browser is used because it is beside the classifier and
  verifies, not because a setting was written. `off` or `none` says "no filter" on purpose.
- **birdnet-go's label file is kept on a V2.4 station** (`7435501`). Rewriting it would have split 460 species
  under two names on stations installed before 0.18.0.
- **The allow gate is a ratchet per file**, not "no allow without a reason": 395 allows had none when it was
  written.

## What the integration changed

- `7435501`: the catalogue lane would have rewritten birdnet-go's labels; `labels_also_accepted` keeps them.
  The "Publish model release" V3.0 job read pins from a shell file that no longer held them.
- `6e13605`: a `perch-v2` job for the same workflow. Not run.
- `acdf998`: a redundant clone clippy found with default features off.
- `296175c`: the pages lane did not know the repair lane's `model_files`. `model_conditions::of_station` adds
  a condition per file being fetched again, not fetched, or needing a restart by hand, so every surface shows
  it. `--doctor` passes a filter turned off on purpose. The pages lane's matches on `Unrouted { primary }`.

## Verification

VERIFICATION_PLACEHOLDER

## Not verified

- Docker: the image, its `HEALTHCHECK`, the entrypoint's install and a container restart. None was run.
- macOS, and a real Raspberry Pi.
- The repair against a slow or unreachable internet. The "down" origin in the tests is a closed local port.
- The loss of queued detections on the old code. Only the stop time was reproduced (29 s against the unit's 30).
- `model-e2e.yml` and the `perch-v2` publish job as GitHub runs them, before this pull request's own run.
- The lanes' browser screenshots other than those listed above.

## Open, from this batch

- MDL-12: publish the Perch mirror (needs the maintainer's go), then add its URL to the catalogue.
- MDL-18: the container half of `installer/test/model-e2e.sh`.
- MDL-22: a mirror URL's credentials are printed in errors and logs.
- MDL-23: the async-runtime scan exempts the start-up steps by name.
- MDL-24: 26 `too_many_lines` allows and 395 reasonless clippy allows remain; the gate only stops them rising.
- MDL-25: a station configured with the legacy V3.0 file names is not repaired by the station.
- MDL-26: no coverage floor for the start-up code.
- MDL-27: a `BIRDNET_SKIP_MODEL` station now fetches its models itself (decision for the maintainer).
- MDL-28: ANSI colour codes in the journal, and two "starting" lines per service start.
