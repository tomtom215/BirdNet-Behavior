# 10. How every model file reaches a station, and what happens when it does not

Read at `cbf25e5` on 2026-11-25 by a delegated read-only pass over the code (no build, no test, no install),
after the maintainer asked whether every classifier and the geomodel install and work for someone who is not
technical. They do not, and the dashboard often does not say so: the list near the end has 18 ways.

**How far to trust this file.** It is one reader's map and has not been re-read line by line. Three of its
claims were checked against the code afterwards. Two held as written: one classifier that fails to load fails
the whole registry (`crates/birdnet-core/src/inference/registry.rs`, every load is `?`), and "installed" is a
size check with no hash (`model_catalog::looks_installed`). One is imprecise: the station's health checks do
include `classifiers` (`src/integrations/station_health.rs`, `check_classifiers`), but it reports a classifier
that is running and failing its windows; a detector that never started reports no model health, so the map's
conclusion (nothing is raised) stands for that case. Treat every other line as a lead to verify, not as a
finding. Nothing here was run.

The next batch turns the list into checklist rows as it verifies each, and closes them against a real install
under systemd with the real files downloaded.

I checked the GitHub releases once with a read-only `gh api repos/tomtom215/BirdNet-Behavior/releases`. Everything else below comes from files.
- Release `models-v2.4` holds `birdnet-v24.onnx` (54068648 bytes, sha256 3b72e8…423d), `birdnet-v24-range-v2.onnx` (29524170 bytes, b18100…b271) and `SHA256SUMS`.
- Release `models-v3.0-preview3` holds the V3.0 model (541391777 bytes, 2a0f9e…d743), the V3.0 labels (8124b0…f8f0), the geomodel (14700313 bytes, b151f6…92ea), the geomodel labels (c15818…c784) and `SHA256SUMS`.
- Every one of those digests and sizes matches the pins in the code.
- There is no release for Perch.

The Rust catalogue has exactly 4 entries (`crates/birdnet-integrations/src/model_catalog.rs:192-338`): birdnet-v3, perch-v2, birdnet-v24 and birdnet-v24-range-v2. **The geomodel is not in the catalogue.** It exists only in the shell scripts: `installer/lib/10-config.sh:249-259` and `docker/entrypoint.sh:337-342`.

## Shared machinery

**A. The binary's `--install-model <id> [--from-file P]`**
- Wiring: `src/main.rs:330-345` calls `src/helpers/models.rs:481-528`, which calls `model_catalog::install` / `install_from_file`.
- Destination directory, in order: env `BIRDNET_MODEL_DIR`, then config `MODEL_DIR`, then `/data/model` if it exists, then `<DB dir>/models` (`src/helpers/station_defaults.rs:48-82`).
- Order of operations:
  - The digest format is checked before any network use (`model_catalog.rs:1118-1121`).
  - If the file already hashes to the pin, nothing is downloaded (`:1127`, `:1139`; `already_installed` re-hashes the whole file, `:895-897`).
  - Disk check: model size plus 512 MiB must be free, and an unknown free-space figure counts as not enough (`:573-580`, `:1128`).
- Origins: the project mirror first, then upstream (`model_origins`, `:389-395`).
  - A bad origin (network error, wrong digest, too many bytes) moves on to the next origin.
  - An I/O error or not-enough-space stops at once (`:854`).
- Each origin gets exactly one attempt. There is no retry inside the binary.
- **No resume.** Bytes stream to `<name>.<pid>.<n>.part`; the part file is deleted on any failure (`:801-803`, `:815-818`) and stale parts are cleaned up on the next run (`:638-667`).
- Verification: streamed sha256, the download is capped at the pinned size (`:775-777`), fsync, then rename (`:741-823`).
- Timeout: one overall reqwest timeout of 6 h; no connect timeout is set (`:1131-1137`).
- Labels: either downloaded from `labels_url` (one origin only, no fallback, `:1160-1163`) or written from embedded bytes and verified (`:933-959`).
- Exit status: 0 on success, 1 on any error, with `Install failed: …` printed to stderr (`src/helpers/models.rs:523-526`).

**B. Dashboard install button (setup wizard "Species model" step, and Settings → Capture)**
- It installs only the defaults, birdnet-v24 then birdnet-v24-range-v2 (`crates/birdnet-web/src/classifier_setup.rs:223-225`, `:367`), through `install_with_progress`, on a thread (`:340-420`).
- The button is offered only when the engine is not started and the configured `MODEL_PATH` is unset or equals `<model_dir>/birdnet-v24.onnx` (`:348`, `:438-443`).
- One attempt; the button then reads "Try again" (`routes/admin/classifier.rs:175-185`).
- "Installed" is decided by a size check only, with no hash (`looks_installed`, `model_catalog.rs:1006-1013`).
- The station is restarted by systemd exit, or by re-exec when nothing supervises it (`crates/birdnet-web/src/restart.rs:1-17`; `routes/admin/system_controls/service.rs:84-91`).
- The only model API is `GET /api/v2/models/catalog`, and it is read-only (`routes/api_write.rs:122`, `:1527-1544`). Nothing in the web crate edits `MODEL_2_*` or `METADATA_MODEL_PATH` (grep found no match).

**C. Bare-metal installer**
- Install, update and reinstall: `do_install` runs `download_classifier`, then `download_occurrence_filter`, then `write_config`, … `validate_install`, `print_summary`, `exit_if_classifier_missing` (`installer/lib/77-manage.sh:59-92`).
- Repair does the same downloads (`:99-171`).
- Which classifier (`52-classifier.sh:96-159`):
  1. an existing config's `MODEL_PATH` (V2.4 only if it is exactly the managed V2.4 path, otherwise V3);
  2. otherwise `--classifier` / `BIRDNET_CLASSIFIER`;
  3. otherwise the V3 file already present selects V3;
  4. otherwise the default, birdnet-v24 (`10-config.sh:105`).
- Retry: `retry_with_backoff`, default waits "30 90" (3 attempts; `55-model.sh:173-192`, `10-config.sh:166`). It is applied to the classifier only, not to the filters.
- If the classifier fails: `classifier_not_installed` prints a loud warning and the install continues (`55-model.sh:197-204`). The summary prints "Installed WITHOUT the classifier" plus a boxed warning (`80-summary.sh:102-116`, `:159-160`) and the run exits 75 (`10-config.sh:165`, `80-summary.sh:120-125`).
- `BIRDNET_SKIP_MODEL=1` skips the classifier with a loud warning and still exits 0 (`55-model.sh:245-252`, `:458-466`; `10-config.sh:162-163`).
- Disk preflight only warns (`45-preflight.sh:74-90`).
- `validate_install` runs `--doctor` but is advisory only (`76-validate.sh:82-90`).

**D. Container entrypoint (`docker/entrypoint.sh`)**
- Runs on every container start.
- Which classifier: `BIRDNET_CLASSIFIER`, then `BIRDNET_MODEL`, then a V3 file on the volume, then the default birdnet-v24 (`:198-258`).
- It always exports `BIRDNET_MODEL` and `BIRDNET_LABELS`, defaulted to the managed paths (`:345-349`).
- Classifier failure: retried (`:276`, `:284-308`), then the container starts anyway with a log warning (`:310-321`, `:910-912`).
- `HEALTHCHECK` calls the non-strict `/api/v2/health` (`Dockerfile:420-423`), which returns 200 while the detection daemon is stopped (`crates/birdnet-web/src/routes/system.rs:249-262`, `:300-309`).

**E. macOS installer**
- Calls `--install-model` for `birdnet-v24` and `birdnet-v24-range-v2`, or for `birdnet-v3` (`installer/lib/85-macos.sh:44-63`).
- Non-fatal, and the run exits 0 regardless (`installer/lib/95-main.sh:22-28`).
- No retry.

**F. Start-up model selection**
- Primary model and labels:
  - `--model` / `BIRDNET_MODEL`, else config `MODEL_PATH`; `--labels` / `BIRDNET_LABELS`, else `LABELS_PATH`; plus a watch directory. Without these the daemon is not started (`src/daemon/config.rs:582-599`, `src/daemon/mod.rs:325-336`).
  - When nothing names a model and the V2.4 files pass the size check in the model directory, they are adopted, together with the range filter if no `METADATA_MODEL_PATH` is set (`src/helpers/station_defaults.rs:114-158`, applied at `src/main.rs:275-290`).
  - The primary's id defaults to "birdnet" (`src/helpers/models.rs:223`).
- Occurrence filter: `--metadata-model` / `BIRDNET_METADATA_MODEL`, else `METADATA_MODEL_PATH`; labels from `METADATA_LABELS_PATH` (`src/daemon/mod.rs:373-381`).
- Extra classifiers: `MODEL_2_*` and `MODEL_3_*` (`src/helpers/models.rs:70-89`), read as env `BIRDNET_<key>` or config `<key>` (`:175-181`), planned against memory at `:215-344`.
- Whole-daemon failure: `ClassifierRegistry::load` fails if ANY admitted classifier's labels or model will not load (`crates/birdnet-core/src/inference/registry.rs:267-293`). `run_daemon` then returns Err (`crates/birdnet-core/src/detection/daemon/run.rs:100-105`), which logs "failed to start detection daemon" and returns None (`src/daemon/mod.rs:685-688`). The web UI keeps running (`src/app.rs:632-657`).
- Labels file whose count does not match the model: NOT refused at load; there is a one-shot warning at the first inference (`crates/birdnet-core/src/inference/model.rs:313`, `:365-371`).
- Occurrence-filter failures:
  - none configured: WARN log, pass-through (`run.rs:170-176`);
  - model unusable: ERROR log, pass-through, daemon keeps running (`run.rs:219-235`);
  - `METADATA_LABELS_PATH` unreadable: the **daemon refuses to start** (`run.rs:178-189`).
- The filter state is published to metrics (`run.rs:251-253`; `process.rs:494-498`) and shown on Station Health as "Species filter: off · …" (`routes/pages/station_health.rs:745-762`).
- Where "detector not running" is surfaced:
  - classifier panel phases (`classifier_setup.rs:229-314`; wording in `routes/admin/classifier.rs:231-333`);
  - Today first-run checklist "Detector not running", shown only before the first detection ever (`routes/pages/today.rs:142-163`, `:405-409`);
  - empty feed `DetectorOff` (`routes/pages/dashboard/partials.rs:85-98`);
  - `/api/v2/health?strict=1` returns 503 (`system.rs:300-309`);
  - deadman after 24 h by default, pushed only when Apprise is configured (`src/integrations/deadman.rs:11-16`, `:35-39`).
- Where it is NOT surfaced:
  - the station-conditions CHECKS list has no daemon, classifier-missing or filter-off condition (`src/integrations/station_health.rs:374-391`);
  - the header badge / issue list has none either (`routes/pages/station_issues.rs:119-266`);
  - `live_capture_state` treats a live microphone with a dead detector as healthy for the badge (`today.rs:787-805`).
- Doctor:
  - model: exists, size > 1 MB, loads, width vs labels (`src/doctor/model.rs:12-103`, `:130-226`); extra classifiers (`:250+`);
  - occurrence filter (`src/doctor/config.rs:172-260`) and location (`:42-65`);
  - the unit's `ExecStartPre --doctor-gate` blocks start only for config, listen, DB-directory or HTTPS faults, so model failures do not stop the start (`installer/lib/65-service.sh:117-131`).

## 1. birdnet-v24: BirdNET V2.4 classifier and embedded labels

- **Who downloads it, and when**
  - Bare metal, by default (no flag needed): through the binary, `download_model_v24` (`55-model.sh:449-499`), via `install_model_with_binary` (`:423-441`).
  - Docker: every start, if the file is missing or empty (`entrypoint.sh:659-718`).
  - macOS installer (`85-macos.sh:48`).
  - By hand: `--install-model`.
  - Dashboard button (B).
  - Daemon start never downloads.
- **URLs, in order**
  1. Mirror: `https://github.com/tomtom215/BirdNet-Behavior/releases/download/models-v2.4/birdnet-v24.onnx` (`model_catalog.rs:267-270`). It exists and matches the pin.
  2. Upstream: `huggingface.co/tphakala/BirdNET-v2.4/resolve/acd49768…/BirdNET_v2.4_fp32_dfttrunc.onnx` (`:265-266`).
  - Labels are embedded, so no URL (`:288-290`).
- **Verification**
  - sha256 3b72e8…423d, 54068648 bytes (`:279-280`); labels sha256 is pinned (`:290`).
  - No resume. The binary retries nothing.
  - Installer and Docker wrapper: 3 attempts (`55-model.sh:484`; `entrypoint.sh:699`). Dashboard: 1 attempt.
  - Disk: 54 MB + 512 MiB must be free. Installer floor is 600 MB, warning only (`52-classifier.sh:54`).
- **If an existing file is damaged**
  - The installer and Docker trust any file that is present and non-empty (`55-model.sh:443-456`; `entrypoint.sh:671-678`).
  - Only the binary itself re-hashes.
- **On failure**
  - Bare metal: install finishes, exits 75, boxed warning. No later retry without `install.sh repair` or the dashboard button.
  - Docker: starts anyway; every restart retries.
  - macOS: exits 0 with a warning.
  - Dashboard: phase Missing with an install button, because the configured path is the default path (`classifier_setup.rs:296-312`). After install, "Installed — restart".
- **Load at start**: see F. If missing or corrupt, the daemon stops entirely. If size is right but content is corrupt, the panel says NeedsRestart (size-only check). If labels are wrong, it runs and names birds wrongly (log warning; doctor FAIL).
- **Tests**
  - Unit tests with loopback servers and fixture bytes (`model_catalog.rs:1442-2210`).
  - Installer shell tests use a stub binary (`installer/test/lib/install-flow.sh:125-201`): `classifier-default.sh`, `-kept`, `-never-mixed`, `classifier-docker.sh`, `macos-config.sh`, `model-pins-match-catalogue.sh`.
  - CI "Inference against the real model" downloads the real model **from Hugging Face upstream by curl, not through `--install-model` and not from the mirror**; a download failure is skipped (`.github/workflows/ci.yml:359-396`).
  - **No test runs the real `--install-model birdnet-v24` against the network.** The upgrade job does not install V2.4 at all (`.github/workflows/upgrade.yml:152-157`).
- **Licence**: CC BY-NC-SA 4.0 (`model_catalog.rs:275-278`).
  - The redistribution question for the embedded labels is left to the maintainer (`crates/birdnet-core/data/labels/README.md:43-50`).
  - NOTICE's "Downloaded" section does not list V2.4 or the models-v2.4 mirror (`NOTICE:60-76`).

## 2. birdnet-v24-range-v2: V2.4 range filter

- **Who downloads it**: the same paths as birdnet-v24, but non-fatal and **not retried** (`55-model.sh:517-551`, single call at `:537`; `entrypoint.sh:837-869`, single call at `:850`); macOS (`85-macos.sh:48`); dashboard step 2.
- **URLs**
  1. Mirror: `…/models-v2.4/birdnet-v24-range-v2.onnx` (`model_catalog.rs:311-314`). It exists and matches the pin.
  2. Upstream: `raw.githubusercontent.com/tphakala/birdnet-go/f4f019a…/BirdNET_MData_V2.onnx` (`:308-310`).
- **Verification**: sha256 b18100…b271, 29524170 bytes (`:323-324`). Disk check needs 29.5 MB + 512 MiB.
- **On failure**
  - Bare metal: warning; the config gets `# METADATA_MODEL_PATH=` commented out (`62-config-file.sh:127-134`). Exit status 0. Retried only by `install.sh repair` or `update`, which also fills in the placeholder (`:65-85`).
  - Docker: warning; `BIRDNET_METADATA_MODEL` is not exported; the next container start retries.
  - Dashboard: the snapshot shows NeedsRestart even when step 2 failed, because the model check runs before the failed-job check (`classifier_setup.rs:305-309`).
- **Load at start**: missing gives a WARN log and the filter is off; unusable gives an ERROR log and the filter is off; detection continues (F).
- **Coordinates are required.** With no coordinates the filter does not run (`species_filter.rs:583-585`), but it is still reported as active with every species as candidates (`process.rs:451-458`, `:494-498`), so Station Health reads "on · admitting N species" (`station_health.rs:761`).
- **Tests**: stubbed installer and Docker tests (above). CI downloads the real file from upstream for `v24_range_filter_matches_birdnet_pi` (`ci.yml:385-395`, `:428-431`). No test of the real install path.
- **Licence**: CC BY-NC-SA 4.0, cited only from the release notes; "no upstream page for this file was read" (`model_catalog.rs:318-322`).

## 3. birdnet-v3: BirdNET+ V3.0 preview and labels

- **Who downloads it**
  - Bare metal: only with `--classifier birdnet-v3`, or when an existing V3 config or V3 file selects it. Done in shell, not by the binary (`55-model.sh:226-302`), with the installer's own file names (`10-config.sh:192-195`).
  - Docker: `BIRDNET_CLASSIFIER=birdnet-v3`, or a V3 volume (`entrypoint.sh:577-637`).
  - macOS and `--install-model birdnet-v3` use the binary, with different file names (`birdnet-v3.onnx`, `birdnet-v3_labels.csv`; `85-macos.sh:85-86`).
  - The dashboard never installs it (a V3 config path falls in the Configured phase: `classifier_setup.rs:296-303`).
- **URLs**
  - Shell paths: the project release `models-v3.0-preview3`, then Zenodo record 18247420 (`55-model.sh:115-120`; `entrypoint.sh:240`, `:174`, `:535-557`). The release exists and matches.
  - The binary: the release URL only. `mirror_url` is None, so there is no Zenodo fallback, and the labels come from one URL (`model_catalog.rs:196-212`, `:1160-1163`).
- **Verification**: sha256 pins (`10-config.sh:194-195`; `entrypoint.sh:171-172`; `model_catalog.rs:207-214`).
  - Bare metal resumes the model download (`40-download.sh:74-91`; curl `--retry 5`, 30 s connect timeout) and is wrapped in 3 attempts (`55-model.sh:272-273`).
  - Labels use plain `download` (`40-download.sh:12-22`, `--retry 3`).
  - Docker resumes (`entrypoint.sh:431-436`, `--retry 5`, `--retry-max-time 900`) with 3 attempts.
  - Existing files are re-hashed (`55-model.sh:157-161`, `:234-238`; `entrypoint.sh:508-533`).
  - Disk: 900 MB floor, warning only.
- **On failure**: bare metal exits 75; Docker starts anyway; macOS exits 0. The dashboard shows "Set in the configuration file / its own model file … not there" with no install button (`routes/admin/classifier.rs:309-330`), although the installer wrote that path.
- **Tests**
  - Stubbed shell tests (`model-resume.sh`, `container-model-cache.sh`, `classifier-*`).
  - CI inference job: real download from the mirror, falling back to Zenodo; a fetch failure is skipped, a checksum mismatch fails (`ci.yml:298-356`).
  - `upgrade-e2e.sh` installs it for real through the previous release's installer, using a cached copy (`upgrade.yml:149-161`).
  - No test runs `--install-model birdnet-v3`.
- **Licence**: CC BY-SA 4.0 (`model_catalog.rs:202`; `NOTICE:63-68`).

## 4. Geomodel V3.0.2 and its labels (V3 occurrence filter)

- **Who downloads it**: bare metal, only when the classifier is not V2.4 (`55-model.sh:325-391`, `:566-572`), non-fatal, **no retry wrapper**. Docker, every start, for V3 (`entrypoint.sh:741-832`).
  - Not in the Rust catalogue, so it cannot be installed with `--install-model`, from the dashboard, or by the macOS installer ("a macOS V3.0 station has no occurrence filter", `85-macos.sh:37-39`, `:88`).
- **URLs**: the project release `models-v3.0-preview3` (it exists and matches), then `github.com/birdnet-team/geomodel/releases/download/v3.0.2` (`10-config.sh:257-259`; `entrypoint.sh:342`, `:760-777`). Docker can override the upstream with `BIRDNET_GEOMODEL_BASE`.
- **Verification**: sha256 pins (`10-config.sh:252-253`).
  - Bare metal uses `download_or_absent`: no resume, curl `--retry 3`, and a 404 is treated as quiet fall-through (`40-download.sh:32-55`).
  - Docker resumes. Existing files are re-hashed. A half pair is removed (`55-model.sh:372-383`; `entrypoint.sh:822-830`).
- **On failure**: warning; `METADATA_*` left commented out (`62-config-file.sh:156-160`); Docker does not export. The filter is off; doctor WARN.
- **Retrofit**: on update or repair, a kept config is retrofitted (`62-config-file.sh:14-55`).
- **Load**: an unreadable `METADATA_LABELS_PATH` stops the whole daemon (`run.rs:178-189`). Coordinates are required (as in section 2).
- **Tests**
  - `geomodel-resume.sh`, `geomodel-fallback-quiet.sh` (stub curl), `rollback-geomodel.sh`.
  - CI fetches the real file from the mirror for `the_geomodel_keeps_out_birds_that_do_not_occur_here` (`ci.yml:337-351`, `:432-433`).
  - `upgrade-e2e.sh` checks it loads and drops a magpie when the station is placed in Kansas (`installer/test/upgrade-e2e.sh:386-401`).
- **Licence**: weights CC BY-SA 4.0 with prohibited uses (`10-config.sh:245-248`; `NOTICE:69-72`; `publish-model.yml:303-307`).

## 5. perch-v2: Google Perch v2 (second classifier)

- **Who downloads it**: only by hand, `--install-model perch-v2` (docs `docs/book/admin/classifiers.md:60-67`). No installer, entrypoint or dashboard path.
- **URL**: Hugging Face `justinchuby/Perch-onnx` at pinned revision da88bf8… only; `mirror_url` is None; no release (`model_catalog.rs:223-225`).
- **Labels**: embedded, `perch-v2_labels.csv` (`:242-244`).
- **Verification**: sha256 bf0c84…fa1f, 409148616 bytes (`:234-235`). Needs 409 MB + 512 MiB free. One attempt, no resume, 6 h timeout.
- **Adding it as a second classifier**: edit `MODEL_2_PATH`, `MODEL_2_LABELS`, `MODEL_2_ID`, `MODEL_2_SAMPLE_RATE=32000` (and optionally `MODEL_ROUTES`) in config or env by hand (`src/helpers/models.rs:615-619`; `classifiers.md:115-120`). There is no dashboard editor.
- **When it is missing or not admitted**
  - Model file unreadable, labels key missing, memory ceiling unknown, or over budget: skipped. The note is logged at **INFO** only (`src/daemon/mod.rs:500-502`; `models.rs:239-295`); doctor WARN.
  - File present but labels missing or model corrupt: **the whole daemon fails to start** (`registry.rs:269-293`).
- **Tests**: catalogue pin test only (`model_catalog.rs:1632-1648`) and the plan logic with tiny fixtures (`models.rs:761-817`, which uses `tiny_v24_test.onnx`). **No real download anywhere** (grep of workflows, `tests` and `installer/test` found nothing).
- **Licence**: Apache-2.0, from the HF model card (`model_catalog.rs:229-233`). Labels: Apache-2.0 claimed from cgeorgiaw/Perch, with sufficiency "the maintainer's question" (`labels/README.md:128-133`). NOTICE says "their terms are their publishers'" (`NOTICE:73-76`).

## Documentation that has gone stale

- `RELEASING.md:383-387` says "Nothing fetches from the release yet". The catalogue does now (`model_catalog.rs:267`, `:311`), and `docs/HANDOFF.md:225-232` agrees with the catalogue.
- `RELEASING.md:363` says a classifier failure "install **aborts**". The installer now exits 75 and the install continues.
- `RELEASING.md:366-368` says the release may lack the geomodel; it has it.

## Ways a station ends up installed but not detecting, or detecting without its configured filter, without the dashboard saying so

1. **Primary classifier missing after an install, with a live microphone.** The daemon does not start (`src/daemon/mod.rs:685-688`).
   - Header badge, issue list and conditions have no such item (`station_issues.rs:119-266`; `station_health.rs:374-391`).
   - `live_capture_state` grades a live microphone as healthy whether or not the detector runs (`today.rs:787-805`).
   - Default `/api/v2/health` and the Docker `HEALTHCHECK` return 200 (`system.rs:300-309`; `Dockerfile:420-423`).
   - Surfaced only on the Capture tab or wizard panel, the Today first-run checklist (before any detection ever: `today.rs:153-163`), the empty-feed `DetectorOff` state, `?strict=1`, and the deadman after 24 h (pushed only with Apprise).
2. **A previously working station whose daemon stops starting** (corrupt model, bad labels, a second classifier that fails to load, unreadable `METADATA_LABELS_PATH`). `total_ever > 0`, so Today shows no checklist (`today.rs:153-166`), and the badge has nothing, as in item 1. Triggers: `registry.rs:267-293`, `run.rs:178-189`.
3. **V2.4 file the right size but corrupt** (bad card, truncated restore that kept the length, hand copy). `looks_installed` checks size only (`model_catalog.rs:1006-1013`), so the panel loops on "Installed — restart to start listening" (`classifier_setup.rs:305-306`). The installer and Docker also trust any present, non-empty file (`55-model.sh:445-456`; `entrypoint.sh:671-678`).
4. **V2.4 file truncated or emptied after install with a non-zero size.** The installer's repair does not re-hash it (`55-model.sh:443-456`). Docker trusts `-s` (`entrypoint.sh:672`). The daemon fails as in items 1/2.
5. **A second classifier whose file exists but whose labels are missing, or whose model will not load.** The whole daemon fails, not just that classifier (`registry.rs:269-293`); unannounced, as in items 1/2.
6. **A second classifier skipped** (memory budget, ceiling unknown, file unreadable, no labels). Logged at INFO only (`src/daemon/mod.rs:500-502`; `models.rs:239-295`). Nothing on the dashboard (Station Health shows cards only for classifiers that reported).
7. **Wrong labels file for the primary** (width mismatch). Not refused; a one-shot log warning (`model.rs:313`, `:365-371`). Detection runs with wrong names. Only doctor FAILs (`doctor/model.rs:199-208`), and the start gate does not block on it (`65-service.sh:117-131`).
8. **Range filter or geomodel download failed at install.** Bare metal leaves `METADATA_MODEL_PATH` commented out (`62-config-file.sh:127-134`, `:156-160`) and nothing retries without `install.sh repair` (`55-model.sh:537-545`, no retry wrapper). Dashboard: Station Health cell "Species filter: off" (`station_health.rs:756`) and panel text "filter is off" (`classifier.rs:288-291`). No badge or condition.
9. **Range filter installed later from the dashboard on a bare-metal or macOS station whose config has the commented placeholder.** The file lands, but nothing writes `METADATA_MODEL_PATH`: `station_defaults` only adopts the filter when no `MODEL_PATH` is set (`station_defaults.rs:130-149`), and the installers always write `MODEL_PATH` (`62-config-file.sh:203`; `85-macos.sh:115`). The filter stays off.
10. **Docker V2.4 range filter installed from the dashboard, then "Restart the station now".** The restart re-execs the binary with the same environment, not the entrypoint (`restart.rs:9-14`). `BIRDNET_METADATA_MODEL` was never exported (`entrypoint.sh:855-858`), and `BIRDNET_MODEL` is always set (`:347-349`), so `station_defaults` does not adopt the filter. The filter stays off until the container is restarted by Docker.
11. **Dashboard install where step 1 succeeded and step 2 (range filter) failed.** The phase is NeedsRestart, not Failed, because the installed-model check comes first (`classifier_setup.rs:305-309`). The failure text is never shown.
12. **Occurrence model configured but no coordinates.** The filter does nothing (`species_filter.rs:583-585`), yet the metric reports it active and Station Health says "on · admitting N species" (`process.rs:451-458`, `:494-498`; `station_health.rs:761`). The panel says "on … (it needs the station's coordinates…)" (`classifier.rs:284-287`). Only doctor warns (`doctor/config.rs:211-223`). The macOS config deliberately leaves the coordinates commented out (`85-macos.sh:108-109`).
13. **Occurrence model present but unusable** (wrong file for the network, corrupt). ERROR log, pass-through (`run.rs:226-234`). The panel still shows "local species filter is on" because it reads whether the filter is configured, not whether it loaded (`classifier_setup.rs:285`).
14. **V3.0 station whose model is missing.** No install button. The panel calls it "its own model file" (`classifier.rs:309-330`) even though the installer wrote the path. The only fix is `install.sh repair`, or restarting the container in Docker.
15. **macOS install where the model fetch failed.** The installer exits 0 (`95-main.sh:27-28`; `85-macos.sh:95-97`).
16. **macOS V3 station never has an occurrence filter**, by design (`85-macos.sh:38-39`, `:88`).
17. **`BIRDNET_SKIP_MODEL=1` or `BIRDNET_SKIP_MODEL_DOWNLOAD=1`** exits 0 / starts with no model (`55-model.sh:245-252`; `entrypoint.sh:578-583`, `:660-665`). Dashboard as in item 1.
18. **`--install-model` hangs on a stalled connection.** The only timeout is 6 h overall, with no connect timeout (`model_catalog.rs:1131-1137`). Wrapped in 3 attempts by the installer and Docker, a worst-case first start can block for that long before the dashboard comes up (the entrypoint runs before `exec`).

## What a real end-to-end check would need

**Closest existing scripts**
- `installer/test/upgrade-e2e.sh`, run by `upgrade.yml` and `release.yml:877-929`, needs root and systemd. It:
  - installs the PREVIOUS release with its own installer;
  - records real magpie detections;
  - updates with the candidate installer and checks the geomodel is configured and loaded;
  - places the station in Kansas and checks the magpie is dropped;
  - runs the printed rollback (`upgrade-e2e.sh:13-24`, `:300-440`).
- Real files it touches: the V3.0 model and labels plus the geomodel and labels, fetched by the previous release's installer on the first run, then cached in an Actions cache keyed `upgrade-models-v3.0-preview3-geo-3.0.2` and staged before each later run (`upgrade.yml:149-161`; `upgrade-e2e.sh:295-299`, `:342-345`). The candidate's update re-hashes those cached files and does not download.
- It does **not** install V2.4, the range filter or Perch, and does not exercise `--install-model` (`upgrade.yml:152-157`).
- `install-smoke.yml` runs a fresh install in a container without systemd, with `BIRDNET_SKIP_MODEL=1` and an offline tarball. No model at all; the doctor's model error is expected (`install-smoke.yml:13-14`, `:131-174`).
- `docker.yml` never runs the entrypoint: both `docker run` calls use `--entrypoint` (`docker.yml:231-237`, `:281-282`).
- CI "Inference against the real model" downloads real V3.0 plus geomodel (mirror, then Zenodo) and real V2.4 model, labels and range filter (Hugging Face and raw.githubusercontent upstream) by curl, then runs inference tests. It does not test the installers, the binary's installer, the mirrors for V2.4, or startup wiring (`ci.yml:298-435`).
- Hermetic shell tests (`installer/test/run-ci.sh:26-58`) stub the binary (`lib/install-flow.sh:125-201`), stub curl (`geomodel-fallback-quiet.sh`), or source entrypoint functions with recorders and a stub binary (`classifier-docker.sh`). Excluded from CI: `pkg-manager.sh` and `upgrade-e2e.sh` (`run-ci.sh:63-67`).

**Paths with no real-download test today**
- `--install-model` for all four entries, against both mirror and upstream, including mirror-first fallback with real hosts.
- The default V2.4 fresh install on bare metal under systemd.
- The container entrypoint in a real container (V2.4 or V3).
- The dashboard install and restart.
- Perch end to end.
- macOS.
- Real-network failure paths: exit 75, the Docker start-without-model loop, filter-off retrofits.


## What a real station did (2026-11-25)

Run by the session itself, not a lane: the binary built from `15a5d15`, the live network, the real files,
nothing stubbed. The scripts were scratch and are not in the tree; turning them into repository tests is row
MDL-18. Each station was started on its own port with its own database and handed
`tests/testdata/Pica_pica_30s.wav` through its watch directory.

| Check | Result |
|---|---|
| `--install-model` for birdnet-v24, birdnet-v24-range-v2, birdnet-v3, perch-v2 into an empty directory | all exit 0; each file's size and sha256 equals its pin |
| The installer's `download_geomodel`, called from its own library files | both files verified against the installer's pins |
| Origins used | this project's releases for all but Perch (Hugging Face, its only origin) |
| A second install of V2.4 | exit 0, nothing downloaded |
| V2.4 with eight bytes overwritten (same size) | replaced; hash restored |
| `--from-file` with the real file / with the range filter's file | installed / refused, nothing installed |
| V2.4 with its range filter, 51.48 N 0.0 E | 3 detections of Pica pica; filter active, 137 candidates |
| V3.0, no filter configured | 4 detections |
| V3.0 with the geomodel, London / Kansas (38.5 N 98.0 W) | 4 detections / 0 |
| V3.0 with the geomodel, no coordinates | 4 detections; Health page "Species filter: on · admitting" |
| V3.0, the geomodel's labels file missing | detector did not start; 0 files analysed; health 200, strict 503 |
| V3.0 in Kansas, the geomodel truncated | 4 detections; Health page "off"; nothing on Today |
| V2.4 with Perch as a second classifier, its file missing | 3 detections from V2.4; Perch skipped at INFO |
| V2.4 with Perch, its file truncated | detector did not start; 0 files analysed; strict health 503 |
| V2.4 with Perch, no route | Perch loaded, 0 windows |
| V2.4 with Perch, `MODEL_ROUTES=local:birdnet+perch`, a recording named as a local microphone names it | Perch loaded, 0 windows |
| V2.4 with Perch, `MODEL_ROUTES=RTSP_1:birdnet+perch`, a recording carrying `RTSP_1` in its name | Perch ran 10 windows; 6 detections recorded under Perch, V2.4 agreeing on 3 |

Not covered by this run: the installer scripts under systemd, the container, the dashboard's install button, a
network that really fails (every download took seconds), macOS.
