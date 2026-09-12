# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Seven clusters: HTTPS in the listener itself, a searchable detection log with
bulk review, backups that leave the SD card they were written on, removing the
Apprise dependency for the services most stations actually use, giving the
detection pipeline the quality controls that separate a station's real records
from its model's artefacts, closing the ten highest-priority gaps against the
two projects this one is measured against, and — the largest — making a station
nobody can log into able to say what is wrong with it.

That last cluster is the subject of `docs/UNATTENDED_DEPLOYMENT_AUDIT.md`, and
its findings share one shape: **a mechanism built end to end and never
connected to the thing that was supposed to drive it.** The audit log had a
table, a store, a page and a 180-day pruner, and the one function that writes a
row had no callers. The live log viewer had a channel, a page and an SSE
endpoint, and no `tracing` layer. Home Assistant discovery registered a
"Station Status" entity, and nothing ever published to the topic it read.
`MqttConfig::qos` had no reader anywhere. `NotifStatus::Queued` had a
production writer and a schema that refused it. Each was silent, and each
looked from the outside exactly like a healthy station with nothing to report.

Plus bugs that were found the same way each time — by running the thing rather
than by reading it. Thirteen state-changing endpoints with no login, found while
adding a fourteenth. A checkbox group that could not be submitted at all, found
by posting a real form. Five CSS variables that had never been defined, found by
looking at a screenshot. One latent data-loss bug found while adding a
quarantine reason. Every detection clip silently truncated at a segment
boundary, found by reading a waveform rather than a code path. An
accessibility feature documented in the wrong direction for its entire life,
found by checking upstream's own config file instead of trusting a comment. And
a notification status the database had refused to store since the day it was
added, found because a gate written for something else would not go green.

### Added — getting an embedding out of a classifier, and what that revealed about the bat stage

**Embedding extraction** (`G-10`, groundwork for Stage 6):
`embedding_output_index`, `BirdNetModel::embedding_width` and
`BirdNetModel::embed`. A classifier can now be asked for its embedding rather
than its class scores.

This exists because **Stage 6 is not the shape the plan described.** The plan
called the bat classifier "the bat classifier, which additionally needs the
≥192 kHz capture path and the ultrasonic validation filter". Checked against
the model itself, `BattyBirdNET` is not a peer classifier at all: its input is
`[batch, 1024]` — BirdNET **v2.4** embeddings — and 256 kHz recordings are fed
to BirdNET *without resampling*, deliberately, so that 144 000 samples reads as
the 3 s at 48 kHz BirdNET was trained on and ultrasound aliases down into the
audible band. It is a **chained second stage**.

That changes three things the plan did not capture. Stage 3's **agreement count
must not apply to it** — "two classifiers agreed" is false when one is reading
the other's intermediate output, and counting it would manufacture
corroboration from a single model's opinion. The pipeline needs a **deliberate
no-resample path**, the opposite of what it does. And it needs **BirdNET v2.4
specifically**: V3.0 emits 1280-wide embeddings, Perch 1536, `BattyBirdNET`
wants 1024 — and this repository ships V3.0 and has no real v2.4.

The extraction finds the embedding **by exact name only**. Perch v2 exposes
both `embedding` (a 1536-wide pooled vector) and `spatial_embedding` (a
16 × 4 × 1536 feature map); a `contains` match would hand a chained head 98 304
numbers where it expected 1 536, which is the Stage 4 output-head defect one
layer down. A classifier with no embedding output reports `None` rather than
offering its class head, which a chained head would consume as though it were a
feature vector.

Verified against the real BirdNET+ V3.0 (`embeddings [-1, 1280]`) and both
committed fixtures — the V3.0 one exposes a 1280-wide embedding, the V2.4 one
exposes none.

**Stage 6 itself remains blocked**, and on things that are decisions or missing
artifacts rather than unwritten code: the no-resample path, a real BirdNET
v2.4, `audio_sources`'s sample-rate `CHECK` (which permits nothing above
48 kHz), `G-14`'s ultrasonic filter, and Stage 4's unfinished half — a 256 kHz
chained head differs from the primary's spec, and the registry refuses that.

### Added — installing a classifier, without ever installing the wrong one

**A model catalogue and a verified installer** (`G-10` Stage 5).
`birdnet-behavior --install-model perch-v2` fetches a classifier, checks it
against a sha256 compiled into the binary, and installs it atomically;
`--install-model list` prints what is available. `GET /api/v2/models/catalog`
is the same list as JSON.

Every digest in the catalogue was **measured from the file this session**, not
copied from a model card: BirdNET+ V3.0 preview3 at
`2a0f9efb…b7d743` (541 391 777 bytes) and Perch v2 at `bf0c8467…cefa1f`
(409 148 616 bytes).

**The catalogue is compiled in, not fetched.** Upstream fetches one from
Hugging Face with a configurable endpoint. That is a remote document deciding
which URL a station downloads hundreds of megabytes from *and* which digest it
checks them against; pinning both here makes the checksum a promise this
repository makes, verifiable by anyone reading the source. The cost is that a
new model needs a release, which is the right cost.

**It streams rather than buffers.** `auto_update` reads an asset into memory
and verifies before touching disk — correct for a 20 MB binary, and an
out-of-memory kill for a 541 MB model on a 1 GB board, which is the failure
`G-33`'s memory policy exists to prevent. The bytes stream to disk with the
digest computed as they arrive, and the safety property is kept by other means:
what lands unverified has a name the station cannot load, and only a verified
file is ever given the real one. A mismatch deletes both.

**Free space is checked before the network.** 400 MB onto a card with 300 MB
free does not fail cleanly — it fills the card, and a station whose disk is
full stops recording birds. Refused, with a 512 MiB margin, and a filesystem
that will not report its free space counts as none rather than plenty.

**There is no install API endpoint**, though upstream has one. A 400 MB
download takes hours on a field station's uplink: a request handler has nowhere
to report progress, and a retry starts a second download beside the first. It
is a foreground command where an operator can watch it fail.

Two things were found by running the command rather than testing it. It
**panicked on first invocation** — `reqwest::blocking` cannot build a client
inside a tokio runtime, and `main` is `#[tokio::main]`; the thirteen unit tests
passed because they are plain sync functions that never enter one, and
`birdnet-integrations` says in its own header that callers must use
`spawn_blocking`. And `MODEL_DIR` was read from the config file only, ignoring
the `BIRDNET_MODEL_DIR` the shipped compose file sets.

Two gates were also found to be green for the wrong reason, by mutating them:
the free-space refusal accepted an unrelated `Io` error as success, so it
passed with the check deleted, and the "unknown is not plenty" test only ever
exercised the measurement, never the decision. The decision is now a pure
`fits()` that both test directly — the same separation `plan_with` needed in
Stage 2, for the same reason.

### Fixed — a real second model found two defects the first one could not

**Stage 4 of `G-10`** is meant to be the test of whether Stages 1–3 are right.
Run against the real Google Perch v2 ONNX (409 148 616 bytes, sha256
`bf0c8467…cefa1f`), it answered: not quite, in two specific ways. Both were
confirmed against the file rather than its model card.

**The class scores were read from the wrong output.** Perch declares four:

```text
[0] embedding          [-1, 1536]
[1] spatial_embedding  [-1, 16, 4, 1536]
[2] spectrogram        [-1, 500, 128]
[3] label              [-1, 14795]
```

The selector was `usize::from(outputs.len() > 1)` — index 1 when a model has
more than one output, which is right for BirdNET+ V3.0 (`embeddings`, then
`predictions`) and picks `spatial_embedding` here: 98 304 numbers of internal
representation, read as though they were 14 795 species scores. It would have
produced confident detections of whatever the arithmetic landed on. The head is
now found by the thing that identifies it — an output whose trailing dimension
is the label count — with a name match as the fallback so a **mispaired label
file still reaches the doctor's existing width check** instead of being
pre-empted here. Loading the real model through the real loader afterwards
gives `output_dimension = Some(14795)`, exactly the label count.

**The sample rate was a guess wearing the word "declared".** Stage 1 said
`InputSpec` carries the rate; it still derived it from a lookup over two
BirdNET shapes with 48 kHz as the default. Perch declares `[-1, 160_000]`,
which is 5 s at 32 kHz — and is equally 3⅓ s at 48 kHz. Nothing in the tensor
distinguishes them. `MODEL_SAMPLE_RATE` / `MODEL_n_SAMPLE_RATE` now declare it,
with the derivation kept for the shapes it was actually built from.

**And a question, which is recorded rather than guessed at.** The pipeline
decodes, resamples and chunks a recording **once**, from the primary's spec.
Perch wants 5 s windows where BirdNET+ V3.0 wants 4.5 s, so running both means
deciding what a merged detection *means* when two classifiers judged different
spans of time — which Stage 3's agreement count assumes they did not. Until
that has an answer, the registry **refuses** a classifier whose spec differs
from the primary's, at startup, naming both specs. Perch is configurable and
validated; it is not silently fed windows it was never trained on.

### Added — two classifiers agreeing is recorded as what it is

**A merge policy and an agreement count** (`G-10` Stage 3). Every classifier
routed to a chunk now runs on it, and what they say is combined: union, each
model judged against its own threshold, one row per species carrying the
highest confidence any classifier gave it, which classifier gave it, and how
many reported the species at all. Migration 50 adds `model_id` and
`model_agreement` to `detections` — two nullable columns, added with `ALTER
TABLE`, which SQLite does in constant time without rewriting a table that on
the station this project is for holds three years of rows on an SD card.

**Agreement is counted, never folded into the confidence**, and the temptation
to is worth naming. A combined number — an average, a maximum with a bonus, a
noisy-or — would have no calibration behind it while sitting in the same
column, on the same scale, as a model's real output. Every threshold an
operator has set, every historical comparison and every export would silently
change meaning. The reported confidence stays the winning classifier's actual
output; the corroboration is a separate integer a reader can weigh themselves.
A gate fails if the merge turns 0.8 and 0.6 into 0.7.

**Union, not intersection.** A bat classifier and BirdNET share almost no
labels, so requiring both to agree would report nothing at all. A species only
one classifier heard is still a detection — with an agreement of one, which is
a weaker claim than two and is recorded as such.

**One classifier repeating itself is not corroboration.** A duplicated row in a
label file must not manufacture agreement out of one opinion, so the count is
of distinct classifiers.

**`NULL` is a third state** and readers must not collapse it: it means the row
predates this migration, when there was one classifier and nothing recorded
which. `1` means one classifier was asked and one answered.

**The human score is the highest any classifier reported**, not the primary's.
It drives the privacy gate, and if any model heard speech the safe reading is
that there was speech — suppressing a bird is recoverable, publishing somebody's
conversation is not.

A single-classifier station — every station in the field today — gets the same
detections in the same order, plus provenance: `model_id` naming its one
classifier and an agreement of one. That is worth having on its own; a station
whose model was swapped last March can now tell which of its detections came
from which.

Verified: `birdnet-core` and `birdnet-db` 19 suites / 1 346 tests, and the
three model-gated end-to-end suites 15 tests under `BIRDNET_REQUIRE_MODEL=1`
against the real 11 K-species model.

### Added — a station can run more than one classifier, and refuses to run more than it can hold

**A classifier registry and per-source routing** (`G-10` Stage 2). A station
may declare up to three classifiers (`MODEL_2_PATH` / `MODEL_2_LABELS` /
`MODEL_2_ID` / `MODEL_2_THRESHOLD`, and the same at `_3_`) and route audio
sources to them with `MODEL_ROUTES=pond:perch,garden:birdnet+perch`. A station
that declares none loads exactly one classifier, as it always has.

Every decision here is shaped by one question: what happens at three in the
morning, four months in, with nobody on site.

**It refuses more memory than the machine has.** All classifiers together may
use at most **half** the effective ceiling — the smaller of physical RAM and
any cgroup limit, the same number `G-33` uses for the analytics pool —
counting each model's file size plus a 256 MiB working-set allowance. On a 1 GB
board that budget is 512 MiB, which one BirdNET model already fills, so a
second is skipped and the arithmetic goes in the journal. A machine that does
not report its memory gets one classifier: **"unknown" must not read as
"plenty"**, because that reading is what gets a station OOM-killed unattended.
The fraction is a judgement, not a measurement on a Pi — nobody here has one —
and the module header says so, with what would falsify it.

**Every misconfiguration fails at startup.** No classifier at all, two under
one name, a model that will not load, or a route naming a classifier that does
not exist: each stops the daemon before it starts, where the journal and
`--doctor` will show it. The route case is the one that matters most —
`MODEL_ROUTES=front-door:pecrh` accepted and discovered at runtime would leave
that microphone unjudged for months, and the loss would be invisible: the
station stays up, the other sources keep detecting, and nothing says the front
door went quiet.

**Silence is unreachable by omission.** A source nobody routed is judged by the
primary classifier, never by none. Not by *all* of them either — a station that
adds a bat classifier has not asked for every microphone to be run through it,
and quietly doubling an unrouted source's inference cost is how a Pi that was
keeping up stops keeping up.

Two drift gates shaped the implementation rather than being worked around. The
config-key scanner proves each key in `KNOWN_CONFIG_KEYS` is really read by
scanning for literals, which `format!("MODEL_{n}_PATH")` defeats — so the keys
are a literal table with field names distinctive enough for the scan to verify
(`path_key`, not `path`, which would match unrelated code and invent reads).
And `.env.example` is checked against what the binary reads, so the names are
exported as `MODEL_ENV_KEYS` — listed rather than derived by prefix, because
`MODEL` and `MODEL_PATH` are also config keys and are *not* read that way;
deriving would have invented two reads that do not happen.

Verified against the real 11 K-species model: `birdnet-core` 5 suites / 846
tests, and the three model-gated end-to-end suites 15 tests under
`BIRDNET_REQUIRE_MODEL=1` — the evidence that a single-classifier station,
which is every station in the field today, behaves exactly as it did.

### Fixed — a 48 kHz model was being fed three-quarters silence

**BirdNET V2.4 received a mel spectrogram zero-padded into a waveform tensor**
(`G-10` Stage 1). Measured, on the committed V2.4 fixture with the pipeline's
own default configuration:

| | |
|---|---|
| what the model declares | 144 000 values — exactly 48 kHz × 3 s, a sample count |
| what the pipeline computed | a 128 × 282 mel spectrogram: 36 096 values |
| what the tensor builder did | padded it with **107 904 zeros** and inferred on it |

Three things had to line up for this to be invisible. The pipeline decided
input *format* from *sample rate* — `expects_raw_audio()` was literally
`infer_sample_rate() == 32_000` — so "32 kHz" stood in for "waveform", which is
true of V3.0 by coincidence and false of V2.4. The tensor builder padded any
short slice without complaint, so a four-fold mismatch looked like a ragged
final chunk. And CI fetches a V3.0 model, so every real-model end-to-end run
took the waveform branch; the mel branch, which is the code default and what
any 48 kHz model selects, had no real-model coverage at all.

**`InputSpec`** now carries sample rate, window length and format as three
declared facts, derived from the ONNX shape's rank and middle dimension — a
property of the model file rather than a coincidence between the two models
that happened to ship first. `[1, N]` and `[1, 1, N]` are waveform windows;
only `[1, M, F]` with `M > 1` can be a mel. Both BirdNET V2.4 and V3.0 are
waveform models, which is what the old rule got wrong.

**The tensor builder now refuses** a slice under half the expected width,
naming both lengths and the likely cause, instead of filling the difference
with silence. Padding is for a recording that ran out of audio — a few per
cent. At 75 % it is hiding a category error.

**`Classifier`** (`inference/classifier.rs`) is the seam the rest of `G-10`
needs: `labels`, `input_spec`, `infer`. One divergence from the plan's sketch —
`infer` takes `&mut self`, because the ONNX session does, and interior
mutability for a signature nobody needs is a worse trade.

Verified against the **real** 11 K-species model (sha256 2a0f9efb…b7d743, the
same artifact CI pins), not only the fixture: it reports a fully-dynamic
`[1, 1]` shape resolving to 32 kHz, 144 000 samples, waveform, 4.5 s — and
`expects_raw_audio` is `true` both before and after this change, so nothing
moves for a station running the shipped model. The four model-gated end-to-end
suites pass with `BIRDNET_REQUIRE_MODEL=1`, which turns a silent skip into a
hard failure; their non-zero elapsed times are what distinguishes real
inference from a skip that would otherwise report `ok`.

Still unverified, and stated rather than implied: no real BirdNET V2.4 ONNX
exists to test against here — this repository publishes only the V3.0 release —
so *that a real V2.4 declares `[1, 144_000]`* remains inference from the
committed fixture and from 144 000 being exactly 48 000 × 3. The fix is correct
either way: a waveform model now reaches the waveform path, and a model that
genuinely wants mel declares it by its shape instead of being guessed at.

### Added — both kinds of rule travel in one file

**Metric-rule export and import** (`G-29`, completing it). `/admin/rules/export`
now carries the station alerts — disk, memory, temperature, detection rate,
queue depth — alongside the detection rules, and the import reads both. They are
different mechanisms sharing a word, but an operator moving a station or asking
for help wants one file, not two.

`EXPORT_VERSION` goes to 2, which is only safe because the files already in
operators' hands still import: a version-1 file has no `metric_rules` field at
all, and that has to read as "no metric rules" rather than a parse failure that
would take the detection rules down with it. A gate removes the serde default
to prove it catches that. A file from a *newer* station is still refused with a
message naming both versions — which is what the version is for.

Metric rules carry no credential, so the `?secrets=1` question does not apply
to them; the `redacted` flag says nothing about them either way, and the
documentation says so rather than leaving it ambiguous.

Two things an import deliberately does not do. A metric this station has never
heard of is **named and skipped**, not silently dropped — a file from a newer
station would otherwise import looking complete while missing the rules that
mattered. And an import applies the same validation the form does, so a rule
that could never stop firing ("disk above −1", which fires on every poll of
every station for ever) is refused on both paths; an import is not a way around
the check.

### Added — the gaps between the barks

**`NOISE_REMEMBER_SECS`** (`G-17`). The noise filter discards a chunk a dog
barked in, and its doc comment argues — correctly — against spreading that to
neighbouring chunks: a bark is a few hundred milliseconds and the chunks
overlap.

That is right about a *bark* and leaves something uncovered. A dog that barks
for a minute is not one bark; it is a bark, a gap, a bark, a gap. In the gaps
there is no `Dog` above threshold, so the chunk filter has nothing to fire on —
and the classifier, still hearing the tail and the room, produces the same
phantom species it produced during the bark. The filter silences the barks and
lets the gaps through, which is exactly backwards for the record.

After a chunk is suppressed, the species that were **in that chunk with the
noise** now stay suppressed for `NOISE_REMEMBER_SECS`.

**Those species, not every species.** A blanket window would be a mute button:
a dog barking through the dawn chorus would erase the chorus, trading one
phantom wren for every real bird in the minute. What a bark produces is a
*specific* wrong answer — the species its spectrum most resembles, the same one
each time — so that is what the window suppresses. A gate asserts a blackbird
singing in the same gap is still recorded.

Off by default, because it does remove real detections whenever a real bird
happens to be the species a dog resembles, and that is a trade an operator
should make knowingly. Setting it without `NOISE_THRESHOLD` logs a warning
rather than doing nothing quietly.

It reaches to the end of the recording being analysed and no further. Carrying
it across segments would mean state on a filter every audio source shares, so a
bark on the garden microphone could silence a species on the pond one — a worse
error than the one being fixed, and an invisible one. The boundary is stated in
the module header rather than left to be discovered.

### Added — one page for "this species has cost me four thousand detections"

**Species storage** (`N-4`), at `/admin/species/manage`. One row per species
this station has ever recorded: detections, clips still on disk, how many are
locked, when it was last heard. Three actions, each confirmed and each written
to the audit log — exclude it from now on, delete its detections, or reclaim
its audio while keeping the rows.

The recurring situation is a squeaky gate that has produced four thousand
Eurasian Wrens. Doing anything about it previously meant three screens, and
there was no screen at all for the detections themselves.

**Locked detections are never touched.** A single-row delete does not check the
lock, because there the operator is looking at the row they named; a bulk
action is issued against a *species* and sweeps up rows nobody is thinking
about — including the one locked last spring because it was a county first. So
every bulk action here skips locked rows, the table shows the locked count
**before** the button is pressed, and the result says how many were kept. An
operator not told would find two detections of a species they believe they
deleted and reasonably conclude the button is broken.

Deleting clips keeps the rows and their filenames, which is migration 22's
point: the name records that audio existed and what it was called, and that is
provenance an analysis may need long after the space was recovered. The rows
are marked reclaimed *before* the files are unlinked — a row marked pruned
whose file survives wastes disk, while a file removed without the mark offers a
player for audio that is gone, and the first is the cheaper thing to be wrong
about.

`File_Name` comes from the database, which on a migrated station holds whatever
BirdNET-Pi wrote there, and this page **unlinks** what it resolves. Every path
is canonicalised and checked to be inside the recordings directory first; a
gate puts a file outside the tree and asserts it survives a `../` filename.

Byte totals are a per-row **Measure** action rather than a column, because they
are `stat` per clip — thousands of syscalls for the species this page exists
for, and a page that took twenty seconds to open on an SD card would not be
worth the number.

One gate in this change was green for the wrong reason and was found by
mutating it: the "worst first" ordering test used a fixture where detection
order and alphabetical order coincided, so replacing `ORDER BY count DESC` with
`ORDER BY name` changed nothing. The fixture now makes the two disagree, and
the repaired gate catches a second mutation it could not have caught before.

### Added — a daily backup, without a daily rewrite of the database

**`BACKUP_SCHEDULE=daily`** (`G-30`, the schedule half). Weekly stays the
default, because switching an existing station to daily would multiply its
offsite upload by seven and that is not a change to make underneath somebody.

The interesting half is what `daily` deliberately does **not** make daily. The
weekly job was four steps — local snapshot, offsite upload, prune, then a
**space reclaim** that checkpoints the write-ahead log and returns free pages
to the filesystem. Shortening that job's interval would have rewritten parts of
the database file every day, and on the SD card this project targets that is
write endurance spent for space nobody asked to have back. An operator who
wants a daily backup is not asking for that.

So the reclaim is now its own job on its own weekly cadence, under the new key
`space_reclaim`, and `BACKUP_SCHEDULE` moves only the backup. The catalogue
drift gate from `G-32` is what made this cheap: adding the job to the scheduler
without adding it to the catalogue fails a test that scans the source for `pub
const JOB_` declarations.

`GET /api/v2/system/jobs` reports the configured cadence rather than the
default, so a daily station is not told its backup is not due for another six
days. That took one parameter rather than a second copy of the schedule: the
`BackupSchedule` type lives in `birdnet-db` beside the job keys, the scheduler
reads it, and the API reads the config file the way `routes::admin::doctor`
already does — a gate asserts the reclaim keeps its weekly interval under both
schedules, which is what would catch the cadence being applied to every job
instead of the one that is configurable.

`spawn_database_maintenance` took its ninth positional parameter with this
change, two of them `u32` and three of them paths — a transposed pair would
have compiled and shown up as a station pruning the wrong directory. It takes a
`MaintenancePlan` now.

### Added — a backup that survives a bad uplink

**`OFFSITE_BACKUP=rsync`** (`G-30`, the target half). The same backup, to the
same SSH server, with the same `OFFSITE_SFTP_*` settings — only the program
moving the bytes changes, so switching is one word and switching back is one
word.

It is here for one reason: **it resumes.** A station on a rural link that drops
at 90 % of a 1.3 GB backup continues from there next time; over SFTP the same
backup starts again from zero, and on a link bad enough to matter it may never
finish at all. `OFFSITE_RSYNC_BWLIMIT` is the second reason — a backup that
saturates a shared connection for an hour is its own kind of failure.

**It is not here because rsync is incremental, and the row that asked for it
was wrong about that.** The gap analysis justified rsync as "it is
incremental". Backups are encrypted before they leave under a fresh random
argon2 salt and nonce prefix, so every run derives a different key. Measured on
a 1 MiB file at rsync's 700-byte block size, counting matches at every offset:
1496/1497 blocks reusable in plaintext with one region changed, **0/1498**
encrypted, and **0/1498** encrypted with byte-identical plaintext. The last
figure is the one that settles it — with nothing changed at all, the salt alone
leaves no block in common. rsync sends the whole file every time. The module
says so in its own header rather than repeating the claim.

rsync moves the bytes; `sftp` still creates the directory, lists it and deletes
what retention drops. That is not a shortcut: rsync has no command that removes
one named remote file, and its nearest idiom — an empty source directory with
`--delete` and an include filter — removes everything the filter does not name.
Adding a second way to lose every offsite backup, in exchange for nothing, was
not a trade worth making.

The SSH policy is now written once. `SftpTarget::ssh_policy_options` is shared
by the `sftp` client and by the `ssh` transport rsync is handed, because a
copied list is the obvious way for host-key checking to be enforced on one path
and quietly missing from the other — and neither path's own tests would have
noticed. A gate asserts the two carry the same policy.

One asymmetry the gates pinned: the remote-path allowlist permits a space,
because `sftp` quotes its batch arguments and can carry one. rsync splits its
own arguments and cannot, so the rsync target refuses a remote directory its
sibling accepts.

### Added — the station can say which of its own jobs have never run

**`GET /api/v2/system/jobs`** (`G-32`, completing it). Seven background jobs
keep a station healthy — the integrity check, the backup and VACUUM, the
offsite upload, the recording cap, the log retention pass, the summary drift
check, the session prune — and until now there was no way to ask about them
except to read the journal.

The design point is what the list is built from. `maintenance_runs` holds a
row per job that has **completed at least once**, so an endpoint written the
obvious way — enumerate the table — answers a different question from the one
being asked, and answers it reassuringly: a station whose backup has never run
would return a short, clean list with the problem simply absent. Migration 28's
own note already said this ("a third state the badge must not confuse with a
failure") about the same table. So the endpoint walks a job *catalogue* and
looks each row up, and a job that has never run appears with `last_run_unix:
null` and `due_reason: "never_run"`. A unit test scans the source for `pub
const JOB_` declarations and fails if one is missing from the catalogue, so a
job cannot be added to the scheduler and stay invisible here.

`ok` is tri-state and the response says so: `null` means either *never run* or
*this job has no pass/fail to report*, and a `reports_verdict` flag separates
those from a recorded `false`. A session prune that succeeded and an integrity
check that failed both have falsy verdicts and mean opposite things.

**The due rule now has one definition instead of two.** It moved into
`birdnet-db` beside the job keys as `due_state`, and `src/maintenance.rs::due`
calls it for the decision, keeping only what a pure function cannot do: read
the timestamp off disk and apply its in-process floor. Before this the loop
that runs a job and anything reporting it as overdue were separate copies of
three branches — never run, clock moved backwards, interval elapsed — and the
third of those had no test at all. It has one now, in the crate both callers
share.

### Added — eBird says whether anybody else has seen it lately

**eBird recent observations** (`G-27`). The station already had a geographic
opinion about which species are plausible: the BirdNET range model. That
opinion is climatological. It knows a Common Swift belongs here in July, and it
has no idea the first one of the year arrived last Tuesday. eBird's recent
observations are the opposite kind of evidence — a person stood near here
within the last fortnight and wrote down what they saw — and that is the better
answer to *is this bird around right now*.

Two places use it, both as corroboration and neither as a filter. A detection's
detail page gains a **Reported nearby** card when somebody reported that species
in the station's neighbourhood recently. The suspect-species report under
Station → Data marks a flagged species somebody reported nearby, and its
*Exclude* confirmation says so — a human birder standing near the microphone is
about the strongest argument there is against excluding a species.

**What it deliberately does not do is treat eBird's silence as evidence.**
eBird coverage follows birdwatchers, not birds: a well-watched county produces
hundreds of checklists a week, and a quiet valley produces none, ever, for
anything. Had absence from eBird been allowed to count against a species, the
station with nobody nearby to confirm anything — the one that most needs an
automated check — would have had its whole list flagged. So a species eBird
says nothing about renders exactly as it did before, and no verdict anywhere
changes.

Off unless `EBIRD_API_KEY` is set. There is no second enable flag: eBird needs
a key for every endpoint, so the key is the opt-in and a station without one
never contacts eBird. By default it asks about a 25 km circle around the
station's own coordinates rather than an administrative region — *reported
within 25 km* says far more than *reported somewhere in this state* — and
`EBIRD_REGION` overrides that for the genuinely remote station whose radius
contains no observers. Coordinates are sent rounded to two decimal places,
which is all eBird documents that it accepts and about a kilometre of
precision. The key is mountable from a file (`BIRDNET_EBIRD_API_KEY_FILE`) and
travels in eBird's `x-ebirdapitoken` header, never in a URL where a proxy log
would keep it.

The snapshot is cached to disk and read back before the first fetch, so a
station restarting at 03:00 can answer immediately and a station whose uplink
is down keeps the last answer it got — labelled as old rather than passed off
as current — instead of losing the feature.

The decoder is written against eBird's published API documentation rather than
against live bytes: every `/v2/` endpoint answers `403` without a key this
repository does not have, confirmed against four of them. Its fixtures are that
documentation's own example bodies, and its doc comment says so rather than
implying a verification that did not happen.

### Added — the species pages browse by taxonomic rank

**Class, order and genus** (`G-15`, second half). A flat list of every bird a
station has heard cannot answer "show me the woodpeckers". The classifier's
label file states two ranks — its header is
`idx;id;sci_name;com_name;class;order` — and the genus is the first word of the
binomial, so three ranks are available without inventing anything.

The List and Photos views gain a row of **order chips** with each order's
species count, built from the species *this station* has recorded rather than
the classifier's 11 560: a garden with forty birds gets a handful of orders, not
seventy-five, and every chip leads somewhere. Chips, search and the view
switcher compose — each one's links carry the others, which is how a filter
control usually breaks. Each species' detail page gains a `class · order ·
genus` line, every rank a link back to the list narrowed to it, so "the other
*Dryobates* I've heard" is one click from a woodpecker.

**No family rank**, and that is the whole of the design decision. The label file
has no family column; a family inferred from a genus would be a guess sitting
beside two stated facts. Genus gets no chip row either — 2 907 genera is not a
control — only the detail-page link, where the question is about one bird.

A station whose label file carries no taxonomy (the V2.4 text format has no
columns at all) gets no chips and pages identical to before; so does one where
every species falls in a single order, because a control offering the only
choice there is is furniture. `labels.rs` now parses the `order` column
alongside `class`, and `SpeciesLabel::genus()` returns `None` for a one-word
label rather than guessing its rank — the pinned file has 55, of which 14 are
family names ending `-idae` and the rest bare genera.

### Added — the weather can come from the anemometer in your own garden

**Three weather providers** (`G-26`). Open-Meteo stays the default and nothing
about an existing station changes. Two more are selectable with
`WEATHER_PROVIDER` in `birdnet.conf`:

- **`met-no`** — the Norwegian Meteorological Institute's Locationforecast.
  Keyless, and a better model over Europe. Its terms require a User-Agent that
  identifies the application and gives a contact address; a generic one is
  answered with `403`, so the client sends a real one.
- **`wunderground`** — a **personal weather station's** own current
  observations, with `WEATHER_STATION_ID` and `WEATHER_API_KEY`. This is the one
  worth having: a gridded forecast is a model's opinion about a cell several
  kilometres across, and a personal weather station is an instrument ten metres
  from the microphone. For asking why the birds were quiet on Tuesday, the
  instrument wins.

An enum rather than the trait the finding proposed. The set is closed,
`birdnet-integrations` carries no `async-trait` and constructs no runtime, so a
trait here would be either dyn-incompatible or a boxed-future dance for no gain
— and an exhaustive `match` is what makes a fourth provider *fail to compile*
until every site handles it.

A `WEATHER_PROVIDER` nobody implements does not start the poll, rather than
falling back to the default. A station configured to read its own anemometer
and quietly served a county forecast instead looks exactly like a station that
is working. Wunderground without both credentials is refused the same way, at
startup, rather than discovered as a `401` every half hour in a log nobody
reads. The API key joins the mountable credentials from `G-28`
(`BIRDNET_WEATHER_API_KEY_FILE`), and the error path never logs the request URL
— Wunderground's carries the key in its query string.

Two things each provider honestly cannot fill in, left empty rather than
invented:

- **MET Norway reports no weather code.** It describes the sky with a symbol
  string (`partlycloudy_day`), not a WMO number. A mapping table would be
  guesses in a column that reads as fact.
- **A personal weather station reports no cloud cover**, because it measures
  the air rather than the sky. Its precipitation is the hourly *rate* —
  deliberately not `precipTotal`, which accumulates since local midnight and
  would read as a downpour by evening after one morning shower.

Which provider is asked is gated by a test that stands up a listener and reads
the request line off the socket. Every other gate here exercises a decoder or a
struct field, and a `fetch_hourly` whose `match` sent all three providers to the
same endpoint would have passed all of them — which is exactly what a mutation
of that `match` showed before the gate existed.

The MET Norway decoder is written against a **live response**, captured from
`api.met.no` on 2026-09-11 and committed as the test fixture; its units come
from that response's own `properties.meta.units` block rather than from memory.
The Wunderground decoder is **not** verified that way and says so in its own
doc comment: that endpoint needs a station owner's API key, which this
repository does not have, so it is written against the published shape and
pinned to a documented sample.

### Added — a detection can carry more than one person's reasoning

**Detection comments** (`G-23`). A verdict says what the station decided; six
months later the question is why. "Call length says Downy, but the spectrogram
is Hairy" is the sentence that makes a record defensible to somebody who was not
there, and there was nowhere to put it.

`detection_reviews.notes` looked like that field and could not be it. Migration
13 puts that table under `UNIQUE(date, time, sci_name)` and writes it with
`INSERT … ON CONFLICT`, so a second reviewer's note **replaced** the first —
silently, and with no user column, so neither of them was named.

Migration 49 adds `detection_comments`: many rows per detection, each with the
account that wrote it *and* the username as it was at the time, so removing an
account (`ON DELETE SET NULL`, never `CASCADE`) does not delete the reasoning or
make it anonymous.

**Append-only at the database, not by convention.** A trigger aborts any UPDATE
of a comment's id, detection, author, body or timestamp. A no-rewrite rule that
lives only in the absence of an update function is one `conn.execute` away from
being untrue.

That trigger was written to cover the whole row first, and a test caught what
that does: `ON DELETE SET NULL` *is* an UPDATE of the child row, so deleting any
account that had ever commented aborted with the append-only message — **users
became undeletable**. The trigger now names its columns and leaves `user_id` out
of them. It is the join, not the record; `author` holds the name and is locked.

Deleting a comment stays possible, because a note with a typo or a neighbour's
name in it needs a way out. The audit log records the id and the author and
never the body — a comment removed because of what it said must not survive in
the log that recorded its removal.

On the page: a thread under the review widget on every detection-detail page,
oldest first so a reply follows what it answers, lazily loaded like the page's
other panels. Writing needs the same admin sign-in as confirming or rejecting;
reading needs nothing. Over the API: `GET`/`POST` on
`/api/v2/detections/comments` and `POST /api/v2/detections/comments/delete`,
with comments attributed to `api` rather than to a name the caller supplies —
a bearer token is not a person, the same reason every audit row this API writes
has a null user.

The batch half of this finding shipped earlier, in PR #234; nothing here was
blocked on it.

### Fixed — 41 birds the range filter could never admit

**Vocabulary alignment between the two models** (`G-15`, first half). The
classifier says what it heard; the geomodel says which species plausibly occur
at this latitude in this week. They are two models with two label files, frozen
at different points in a moving taxonomy, and they do not always spell a species
the same way. A geomodel name the classifier does not carry verbatim was
dropped, in silence.

Measured on the pair the installer pins — geomodel labels `sha256 c15818db…`,
12 012 rows; classifier labels `sha256 8124b0ea…`, 11 560 rows, both downloaded
and hash-verified rather than described from memory — **1 679 geomodel rows have
no exact scientific-name counterpart**. Most of those the classifier genuinely
cannot emit. **Sixty-two are the same taxon under a reclassified genus**, and
forty-one of those are birds: the geomodel writes *Leuconotopicus villosus*
where the classifier writes *Dryobates villosus*, and both mean Hairy
Woodpecker. Every one of the 41 was **permanently undetectable at every station
running the range filter** — Hairy Woodpecker, Red-cockaded Woodpecker,
White-headed Woodpecker, Evening Grosbeak, Arizona Woodpecker and fifteen more
woodpeckers among them (nineteen of the forty-one are woodpeckers, the
*Veniliornis* and *Leuconotopicus* the classifier files under *Dryobates*) — and
nothing reported it.

`crates/birdnet-core/src/inference/vocabulary.rs` resolves the two vocabularies
once at load: exact scientific name first, then the **common name and the
specific epithet together**, the common name compared on its letters alone
(`Fruit-Dove` = `Fruit Dove`) and the epithet required to agree exactly or
modulo its Latin gender ending (*gymnocerca* / *gymnocercus*). A match returns
the **classifier's** spelling, because that is what a detection carries and what
the passing set is tested against.

The epithet is a veto, not decoration. Three rows match by common name and
disagree on the epithet, and two are plainly wrong: the classifier's own label
file calls *Lama glama* — the llama — "Guanaco", and calls *Scapteriscus
borellii* "Southern Mole Cricket" for a geomodel row that is *Gryllotalpa
australis*. The third, *Physeter macrocephalus* against *Physeter catodon*, is a
real synonym the guard costs us; it is not a bird, and one lost whale against
two wrong admissions is the trade.

Two departures from the finding's original plan, both forced by evidence:

- **No alias table ships.** The plan was to vendor OpenFauna's `aliases.json`,
  which `tphakala/birdnet-go` embeds. It is CC BY-SA 4.0 and this project is CC
  BY-NC-SA 4.0 — ShareAlike does not permit adding the NonCommercial
  restriction. And it does not work: applied to the pinned pair, **all 237 of
  its entries recover 0 of the 1 679**, because its reclassifications
  (*Accipiter* → *Tachyspiza* and the like) are ones both of our files already
  agree on. `SPECIES_ALIASES_PATH` takes an operator's own tab-separated map for
  the pairs no automatic rule can reach; an unreadable file is a warning, never
  a reason to take the filter off a running station.
- **No normalisation on write, and no migration.** The plan called for
  rewriting stored detections to a canonical name. The names disagree *between
  two label files*, not between a detection and its own model — a detection
  already carries the classifier's spelling — so there is nothing in
  `detections` to collapse, and rewriting it would have broken every join back
  to the label file the row came from.

`--doctor` now reports both residues, which is the half of the defect that was
"nothing says so": how many metadata species map onto the classifier and by
which rule, how many do not, and — read differently — how many *classifier*
species no metadata row resolves to, and so cannot pass at all while the filter
runs (1 165 of 11 560 on the pinned pair).

Building the map at load also replaced a linear scan of the classifier's labels
per passing species per inference, which on the pinned pair is up to 12 012 ×
11 560 lowercasing comparisons behind one cache miss. The whole alignment takes
52 ms once (debug build, this machine); no claim is made about the end-to-end
saving, which was not measured.

### Added — an operator can write their own alerts on the station's measurements

**Metric rules** (`G-29`). The station failure that loses a season is silent:
the disk fills, or one microphone of three dies, and nothing says so until
somebody looks at a chart weeks later. The station alerts on a fixed set of
conditions already — 85 % disk, a flapping source, a drifting clock — and those
are the right defaults and not everyone's. The rule that catches a dying
microphone at a particular station is *"tell me when the hourly detection count
drops below what it normally is here"*, and no number chosen in this repository
can be that.

Seven measurements, on the same **Station → Alerts** page as the detection
rules: disk in use, memory in use, CPU temperature, detections in the last hour,
seconds since the last detection, capture restarts in the last hour (the worst
source), and uploads waiting to be sent. A rule is a measurement, a direction
(`above`/`below`) and a threshold.

Three departures from the finding's original plan, each for a reason that only
became clear on reading the code:

- **A separate table, not a discriminant on `alert_rules`.** The two are
  different mechanisms sharing a word: an `alert_rules` row matches one
  *detection* as it arrives and fires an action of its own; one of these is a
  *sampled measurement*. A discriminant would have made every detection-rule
  read carry columns that are always NULL, and given neither mechanism the
  other's machinery.
- **Evaluated by the station-health poll, not the maintenance loop.** The
  maintenance loop is daily and weekly, which is the wrong cadence for "tell me
  when the disk passes 85 %". More to the point, a firing rule becomes an
  ordinary station-health `Condition` — so it inherits the three-poll (fifteen
  minute) debounce, the episode latching, the recovery notice, the notification
  log, the store-and-forward outbox, and a place on
  `/api/v2/health/conditions`. A parallel engine would have had none of that.
- **No per-rule cooldown, and no export-format bump.** The episode *is* the
  cooldown. Nothing was added to the `alert_rules` export, so its version is
  untouched; export/import for metric rules is still to come.

Two decisions the tests exist to hold. A metric the station cannot read right
now produces **no** alert — a board with no temperature sensor is not cold, and
treating a missing reading as zero would make every `below` rule fire for ever.
That mutation *survived* the first version of the gate, because an `above` rule
cannot tell zero from missing; the gate now uses a `below` rule, which is also
the shape the feature exists for. And a rule that could never stop firing —
"disk above −1" — is refused when it is created, rather than becoming a
notification that arrives for ever.

### Added — the analytics engine's memory is sized to the machine, not assumed

**A memory budget for DuckDB** (`G-33`). The buffer-pool cap was a flat 256 MiB
whatever the machine. On the hardware the shipped systemd unit is written for —
`MemoryMax=1G` — that is a quarter of the budget and reasonable. On a 512 MB
board it is **half of physical RAM** for one subsystem, alongside the model, the
web server and the OS. DuckDB treats the limit as permission to use that much,
so the flat default was a standing invitation to be OOM-killed mid-query at
three in the morning.

`crates/birdnet-behavioral/src/memory.rs` sizes the pool to a quarter of the
*effective ceiling* — the smaller of `/proc/meminfo`'s `MemTotal` and any cgroup
limit the process is under — capped at 256 MiB and floored at 64 MiB. Below the
floor the station starts **without** analytics and says why: it still records,
classifies and serves, it just has no behavioural dashboards. Refusing is the
point; the alternative is starting something that will be killed.

Both numbers are anchored rather than chosen, and the source says which is
which:

- **A quarter** is the proportion the shipped unit already implies (`MemoryMax=1G`
  with a 256 MB pool), so a station on that unit gets exactly the limit it got
  before. That derivation is a `const` assertion, because the unit test that
  looks like it covers it is satisfied by the cap alone — measured: halving the
  fraction still passed that test, and only the assertion catches it.
- **64 MiB** was measured. A sessionisation shaped like the behavioural
  queries — `lag` and a running sum over 1.5 million detections partitioned by
  species, then aggregated — ran out of memory at 8, 16 and 32 MiB and succeeded
  from 48 MiB up on DuckDB 1.5.

What is deliberately *not* claimed: the rest of the process's footprint on a
Raspberry Pi is not measured, because that needs a Pi. So this sizes a
proportion, not a budget. The module header says so, and says what would
falsify the fraction.

`--doctor` reports the decision under **Analytics memory** — the choice is made
once at startup, and an operator diagnosing an OOM-killed station months later
is looking at `--doctor`, not at a boot they no longer have.

One defect found while doing this: `.env.example` shipped
`BIRDNET_DUCKDB_MEMORY_LIMIT=256MB` **uncommented**, so any container using that
file as its `.env` pinned 256 MiB on every board — including the small ones the
sizing exists for. Now commented, with the sizing explained in its place.

### Added — the capture watchdog's timings are the operator's

**`WatchdogConfig`** (`G-9`). Every number the capture supervisor decides with
was a `const`: how often it looks, how much missing output makes a live process
stalled, how fast the restart backoff grows, how long down before the loud
warning and how often to repeat it. Defensible defaults, but a station with slow
storage, long segments or a camera that legitimately pauses had no way to say so
short of rebuilding.

Seven knobs, each read from `BIRDNET_WATCHDOG_*` or the unprefixed
`birdnet.conf` key, each clamped to a documented range — and every adjustment
logged, because a clamped watchdog running on timings its config file does not
describe is worse than a refused one. The defaults are exactly the constants the
supervisor used before, and a test asserts that, since a tuning feature that
changes the default behaviour is worthless.

Two knobs deliberately absent. **A maximum retry count**, which upstream
BirdNET-Go has: a field sensor unreachable for six hours must still be reachable
on hour seven, and a supervisor that has given up is a station that is silently
not recording — offering it would be offering a way to break the property the
supervisor exists to provide. And **the flapping threshold and window**, which
live in `birdnet-core` because the web layer renders against them too; a
per-station value would have to reach both, and half-wiring it is worse than
leaving it fixed.

The finding's own rationale turned out to be misattributed, which is worth
recording. It read *"the right silence threshold at a busy feeder is not the
right one for an arctic winter station where 30 s of silence is normal and 6 h
is not"*. That describes `BIRDNET_DEADMAN_HOURS`, which is already a settings
field. The stall threshold is not about silence: it ages the newest *recording
segment*, and a microphone writes segments through hours of quiet. The real case
for tuning is narrower and still real. `docs/FEATURE_GAP_ANALYSIS.md` carries
the correction.

### Added — saved clips can be normalised to an even loudness

**ITU-R BS.1770 integrated loudness, and one gain per clip** (`G-5`). Clips
were written at capture level, so a gallery of them recorded across a season is
not level: the listener rides the volume control between every one, and a quiet
clip at the end of a playlist gets missed.

`crates/birdnet-core/src/audio/extraction/loudness.rs` implements the
measurement: the two-stage K-weighting filter, 400 ms blocks at 75 % overlap,
the `-0.691` offset, the absolute gate at −70 LUFS and the relative gate at
10 LU below the absolutely-gated mean. Mono, which is what every clip this
station writes is. The filter is built from BS.1770's analogue prototype rather
than transcribed from the standard's 48 kHz table, so a station capturing at
44.1 kHz is measured through a filter designed for 44.1 kHz.

Off by default — a clip is an archival record as well as something to listen to
— and set from **Settings → Audio Capture** or `BIRDNET_CLIP_TARGET_LUFS`.
−18 LUFS is the recommended value; −14 and −23 are offered.

It is applied at clip-write time only. The samples the mel spectrogram and the
classifier see are untouched: a per-clip gain there would move every confidence
score and make two stations' thresholds mean different things. The module lives
under `extraction/` rather than beside `audio::soundlevel` so the module tree
says that too.

The peak ceiling (−1 dBFS by default) wins over the target: a clip that is
quiet but peaky — one loud wing-beat over a distant song — is turned up only as
far as the ceiling allows. That ceiling is a **sample** peak, not an ITU true
peak, and the module says so rather than glossing it: BS.1770's true peak needs
4× oversampling through a specified interpolation filter, which this does not
do. The default leaves about a decibel of headroom for inter-sample peaks.

A normalised clip's RIFF INFO comment carries
`Normalised to -18.0 LUFS from -31.4`, appended to the existing comment rather
than given a tag of its own — RIFF INFO has no identifier for loudness, and
inventing a four-letter one would put a field in the file that no player can
read.

Two things the verification turned up, both worth recording because they change
what the tests are worth:

- The first version of the coefficient test carried a "published table" typed
  from memory and **failed against a correct implementation** — the derivation
  produced `1.53512485958697` where the recalled value read `1.535123299202456`.
  The construction was then checked line by line against libebur128's
  `ebur128_init_filter`, the implementation ffmpeg and most loudness tooling
  use: prototype constants, `Vb` exponent, and the RLB stage's un-normalised
  `1, -2, 1` numerator all agree.
- That pinned-coefficient test turned out to be the *more* sensitive of the
  two, which was not the expectation. Replacing the shelf `Q` with `1/√2` — the
  plausible value anyone would write from habit, one part in ten thousand from
  the specified `0.7071752369554196` — moves the 1 kHz gain by less than the
  relationship test's tolerance and passes it. Only the pin catches it. The
  test's own comment now says so instead of describing the pin as the weaker
  check.

### Added — the settings-key guard is now general, and it found a key outside it

**A source-scanning drift gate for the `settings` table** (`G-34`, the part
worth having). The `settings` table is a bag of strings, so a key written under
a name nothing reads is indistinguishable, from the operator's side, from one
that works: the control saves, the page redraws, the value is in the database,
and nothing happens. This project has shipped that twice — twenty admin-form
fields that were editable and inert, and a first-run wizard field
(`notification_mode`, a four-way choice of how often to be alerted) that a
non-technical operator picked on their first day and which governed nothing.

Each fix added a guard for *that writer*, from a list maintained by hand:
`SETTINGS_FORM_KEYS`, then `ONBOARDING_SETTING_KEYS`, then
`AUDIO_ADMIN_SETTING_KEYS`. Three lists, and a fourth writer outside all of
them.

The new gates read the source instead. `every_settings_key_written_anywhere_is_classified`
finds every `settings::set` in production code — resolving a key given as a
named constant, and excluding `#[cfg(test)]` fixtures, both of which the scan's
own self-test pins — and fails when one is not classified in `SETTING_SPECS`.
`every_subsystem_owned_setting_is_read_somewhere` is the other direction: a key
classified as read straight out of the settings table by a named subsystem must
have a read somewhere in the source, so a subsystem that stops reading a key it
owns is caught rather than leaving an inert control behind. Call sites that
build their keys at runtime (`set_many`) are allowlisted by file with a note
saying what bounds their keys, so a *new* dynamic writer trips the gate.

It found one on its first run: **`analytics_exclude_imports`**, written from
`/admin/migration` and read by both analytics engines, was classified nowhere —
so nothing was checking that the "exclude imported detections" control did
anything. It does; it is now classified, and would have been caught the day it
stopped.

It also reported `timezone` as owned-but-never-read, which was the gate's own
message coming true from the other side: `--doctor` reads it through
`setting_from_db(config, "timezone")`, a shape the scan did not know. Taught.

Not done, with a reason rather than an omission: the JSON Schema artifact the
finding also asks for. `SETTING_SPECS` records a key, its wiring and its
category — no types, defaults or descriptions — so a schema derived from it
today would say `"type": "string"` about every setting and assert nothing.
Adding that metadata to ~50 specs is a separate piece of work, and the drift it
would catch between `.env.example`, the config keys and the readers is already
gated from the env-variable side by `helpers::env_keys` and from the config-key
side by `tests/every_config_key_is_known.rs`.

### Added — a credential can be mounted as a file instead of set in the environment

**`BIRDNET_<KEY>_FILE` for every outbound credential** (`G-28`, first half).
A notification URL carries its bot token *inside* the URL, and an environment
variable is readable by `docker inspect`, by anything holding the process's
`/proc/<pid>/environ`, and by anything that logs its own environment. Docker
(`secrets:` → `/run/secrets/<name>`) and Kubernetes (a projected secret volume)
both solve this by mounting the secret as a file, with a `<VAR>_FILE` variable
naming the path. This station had no way to accept that.

Five keys take it: `NOTIFY_URLS`, `APPRISE_URL`, `BIRDWEATHER_TOKEN`,
`MQTT_PASSWORD` and `HEARTBEAT_URL` — every credential that leaves the station,
which is to say every value where knowing it is enough to post as this station.

Four decisions, each gated:

- **The direct value wins**, and the file is reported rather than read. The rest
  of startup resolves the direct value first anyway, so a resolver that
  preferred the file would be describing a station other than the one running.
- **An unreadable or empty file leaves the feature off, at *error* level.** A
  projected volume that has not been populated yet reads as a zero-length file,
  and accepting that as "the password is the empty string" sends the station off
  to authenticate with nothing. More to the point, a station that sends no
  notifications because a mount path had a typo looks exactly like one that was
  told to be quiet.
- **Surrounding whitespace is trimmed, inner newlines are kept.** Secret files
  end with a newline; a bot token with `\n` glued to it fails at the far end,
  which is far harder to diagnose than an empty one. A `NOTIFY_URLS` file may
  still list one URL per line.
- **A file-supplied value is never seeded into the `settings` table.** That
  table is in the database, and the database is in every backup, restore bundle
  and support archive — which is the exposure the mount exists to avoid. The
  first-run seed now takes the list of file-resolved keys and skips them.

The other half of `G-28` — parking a notification in `outbound_queue` while a
destination's circuit is open — was **not** implemented, because reading the
code showed the gap analysis was wrong about the consequence. An alert about the
station is not lost when a circuit is open: `src/integrations/announce.rs` holds
it in an outbox keyed by episode and retries at every poll until it goes out,
and `Alert::body_at` already appends "(Raised N minutes ago; earlier attempts to
send this did not reach a destination.)" so a late alert says so. The finding,
and the two narrow things that really are left, are recorded in
`docs/FEATURE_GAP_ANALYSIS.md`.

Environment only, deliberately: this is a container-deployment facility, and a
station editing `birdnet.conf` by hand can put the secret in the file it is
already editing. `APPRISE_CONFIG` is excluded because `APPRISE_CONFIG_FILE`
already exists and means the path of an Apprise *configuration* file; giving one
variable name two meanings is how an operator's config gets read as a
credential, and a gate now asserts no indirect key can collide that way. The
SMTP password is excluded because it lives only in the settings table.

### Added — a station can say which source Listen plays

**A station-wide default live-stream source** (`N-3`). `?source_id=` has let a
*listener* pick a source since it shipped, and the Recordings picker builds it —
but a station could not say once, for everyone, which of its microphones people
actually want to hear. A two-microphone station (feeder and nest box) has one of
each, and every visitor pressing Listen got whichever row happened to be oldest.

`livestream_source` names the default `audio_sources` row. `/stream` with no
`?source_id=` serves it if it is still enabled, and otherwise falls back to the
first working source — deliberately, because this is the path a visitor reaches
by pressing Listen with no choice of their own, and a station whose named
default was unplugged last week should keep streaming the microphone it still
has rather than answering with silence or a 503. The journal says when the
fallback fires.

It is set with a **Make listen default** button on each row of `/admin/audio`,
not a field on the settings page: the value is an `audio_sources` row id, which
belongs beside the rows rather than typed into a text box. The POST answers with
*both* source lists as out-of-band swaps, because the row losing the pill is
usually in the other one — making an RTSP camera the default takes the pill off
a microphone. A disabled source is neither offered the choice nor accepted if
asked for it, since `/stream` skips disabled rows and the setting would then
name a default that is not the default. Audited as
`audio.source.listen_default`.

That made `/admin/audio` a **third writer of the settings table**, and the guard
that fails when a settings key nothing reads is shipped covered only the admin
form and the first-run wizard — the two writers whose past mistakes created it
(twenty inert form fields, and one wizard field governing nothing).
`AUDIO_ADMIN_SETTING_KEYS` and a third classification gate extend it to this
one. `routes/admin/migration.rs` writes `exclude_imports` and is still outside
that guard; closing that properly is G-34's drift gate, which is still open.

### Added — one capture source can be restarted without restarting the station

**A per-source restart** (`G-32`). The only remedy a station had for one
wedged RTSP camera was `POST /api/v2/control/restart` or the Restart button on
`/admin/system` — both of which stop the whole process, taking every *other*
microphone down with it and losing the audio in flight on each. On a
multi-source station that is a blunt instrument for a fault in one source.

The capture supervisor owns its source list privately on its own thread, so
this is a new seam in the direction that did not exist:
`birdnet-core`'s `audio::capture::control` carries a set of pending restart
requests the web layer writes and the supervisor drains once per reconcile
tick — the mirror of the `status` module that carries per-source health the
other way. A drained request stops the named source and clears its fault
state, so the reconcile in that same tick starts it again immediately rather
than waiting out a backoff the operator can neither see nor shorten. Asking
twice before a tick still restarts it once.

The restart goes through the supervisor's own start path, deliberately: the
recording schedule and the source's quiet window still hold, so a request for
a paused source is spent without overriding the schedule the operator
configured, and the response says so instead of reporting a restart that will
not happen. It recovers a wedged source; it does **not** reload that source's
settings, because the supervisor builds each source's capture config once at
start-up — an edit still needs a service restart, as it always did.

Three surfaces: a **Restart** button on each row of `/admin/audio`,
`POST /api/v2/control/restart-source` for automation, and
`GET /api/v2/system/capture` to read the labels it takes and see whether the
restart took. The capture read is bearer-gated rather than public — a
station's source labels and per-source fault history are operational detail
about someone's home, not a public detection count. Every restart is written
to the audit log as `audio.source.restart`.

One diagnostic subtlety, gated in both directions: a restart an *operator*
asked for is not counted toward `restarts_last_hour` and so never raises the
`flapping` verdict. That number exists to spot a source that cannot stay up on
its own, and three clicks while debugging a camera is not that — but a source
that keeps dying still reads as flapping, which is the counterpart the gate
asserts so the discrimination cannot decay into "stopped counting".

### Added — the target-sensitive crates' tests run natively on aarch64

**A `test-aarch64` CI job** (`ARM-1`). No `cargo test` had ever executed on
the architecture the station ships to; `cross-aarch64` compiles for the Pi
and runs nothing. The new job runs `birdnet-core`, `birdnet-scheduler` and
`birdnet-timeseries` natively on `ubuntu-24.04-arm`, and its comment says
which crates are excluded and why.

### Added — detections can be handed to Raven and Audacity

**A Raven selection table and an Audacity label track** (`FR-1`). No output
of the station was one a verification tool read; the ecologist opened the
clip and found the call by ear. `GET /api/v2/detections/export/raven` is one
table over every detection with a clip, in BirdNET-Analyzer's column layout
with `Begin Path` naming the clip, so Raven Pro opens it against the
recordings folder; each clip has its own table and an Audacity label track
at `/api/v2/recordings/<clip>/raven.txt` and `…/labels.txt`. Each selection
is placed where the detection sits inside its clip: migration 47 records
the lead-in the extractor actually wrote and the window's length, which
`chunk_offset_secs` (the start in the source segment) never said. Rows from
before that span their clip; rows with no clip are left out.

### Fixed — the weekly space reclaim no longer rewrites the database

**`PRAGMA incremental_vacuum` in place of `VACUUM`** (`PS-3`). The weekly
`VACUUM` wrote three times the file size, staged the copy in the unit's
memory-charged `/tmp` inside `MemoryMax=1G`, and held the write lock long
enough for a detection to time out and be logged lost. New databases are
created in `auto_vacuum=INCREMENTAL`; an existing one is converted once at
the next start, with the copy staged beside the file, and the weekly job
then moves only the free pages, a MiB at a time, pausing between steps.
The writer's lock wait is fifteen seconds, up from five.

### Added — a private mode puts the whole station behind the sign-in

**`BIRDNET_PRIVATE_MODE` and `BIRDNET_PUBLIC_ACCESS`** (`O-4`). The default
contract — viewing is open, only changing things needs a password — was the
right one for a Pi on a home LAN and the wrong one for the same Pi behind a
tunnel or a port forward, where the open dashboard is the detection history
and a live microphone feed for anyone with the URL. Private mode moves the
public router behind the cookie gate the admin panel uses; what stays open
is the sign-in form, its assets, the health probe and the carve-outs the
operator names: `live_audio` (the stream and the live spectrogram), `share`
(operator-minted `/r/<token>` links, whose audio and spectrogram are now
served directly rather than redirected to the gated media routes) and
`metrics`. Pages are sent to `/login`; the API and the WebSockets get a
`401`. A private station with no admin password fails closed with a `503`
and a message, at startup, on the page and in `--doctor`, rather than
falling back to the open station it was asked not to be. Config file:
`PRIVATE_MODE`, `PUBLIC_ACCESS`.

### Added — the analysis queue is measured, and a station that falls behind sheds instead of losing audio quietly

**A queue-depth gauge and a stated shed policy** (`PR-2`). "Inference slower
than real time for an hour" resolved to the stream drain deleting the oldest
unanalysed audio and saying nothing. The daemon now publishes
`birdnet_analysis_queue_depth` (and `analysis_queue_depth` on the health
body); while more than 40 segments wait, or while the board is at its
thermal limit or throttling, it analyses one segment in two, counts the rest
in `birdnet_segments_shed_total` by reason, and raises the `backlog`
station-health condition.

### Added — raw audio can be kept at a duty cycle

**`RAW_AUDIO_KEEP_EVERY` keeps one raw capture segment in N** (`R-4`). The
raw audio was gone once analysed, so only what already triggered a
detection survived and a season could never be re-scored under a new
model. One aged segment in N, in capture order, is now copied into
`<recordings>/raw` before the stream directory drains it; the disk-full
purge takes that raw audio before any clip.

### Fixed — a segment being analysed is never the one the purge deletes, and one lost before analysis is counted

**Unanalysed audio is no longer destroyed silently** (`PR-1`, `S-3`). The
stream directory's drain, size cap and disk-full purge ran on age and size
alone, and a probe deleted a segment a live reader held open; a segment
lost before the pipeline read it left a log line identical to the healthy
case and moved no counter. The daemon now claims each segment while it
reads it and the purge skips claimed names; a segment gone before analysis
is a warning and `birdnet_segments_dropped_total`, per source.

### Fixed — every text reads at WCAG AA contrast, and the gate now checks

**Colour contrast is enforced in the accessibility gate** (`DD-29`, `UX-16`,
`UX-17`). Measured across every page in both themes, 285 text nodes fell
below AA: the species chip's identity hue on its own tint, the muted text
tokens on tinted surfaces, white on the bright dark-theme fills, the
failure pill, the amber badges. The species hue is kept as identity and
mixed towards an ink token wherever it is text; `--moss`, `--rare` and the
muted greys sit at AA on every surface; fills carry `--on-fill`. The axe
gate runs `color-contrast` by default and reports 0 violations.

### Fixed — the backup page no longer scrolls sideways on a phone

**A snapshot row fits a 390 px viewport** (`DD-30`). Once a station had one
backup snapshot, its unbreakable file name widened the row's grid track and
the whole page scrolled sideways on a phone. The track is now
`minmax(0, 1fr)`, the children may shrink, and the name wraps.

### Fixed — an approved quarantine row keeps its provenance

**A quarantined detection records the bar it was heard under** (`DD-9`).
The quarantine table never held the confidence threshold, sensitivity or
overlap, so approving a row wrote NULLs into the detection: the records an
operator had looked at hardest had the least provenance. Migration 46 adds
the three columns, the daemon fills them at quarantine time, and approval
copies them across.

### Fixed — the journal survives a reboot and cannot fill the card

**The installer writes a persistent, bounded journald drop-in** (`OP-5`,
`OB-15`, `PS-19`). On a default Raspberry Pi OS the journal was volatile:
about a month on a 2 GB Pi and nothing across a reboot, so every watchdog
bounce, power cut and update erased the evidence of what caused it. The
installer now sets `Storage=persistent` and `SystemMaxUse=200M` beside the
unit, and `uninstall` removes the drop-in. The two per-file INFO lines
("begin processing file", "file processing complete"), measured at 92 % of
the journal's volume and 1.6–2.8 GB a year, are DEBUG; the
`birdnet_files_analysed_total` counter carries what they said.

### Fixed — a silent station is a strict fault

**The detection deadman's verdict is on the health endpoint** (`AD-4`).
`detection_silence_secs` was on the body and in no status code, so a week
without a detection left even `?strict=1` green. The deadman now publishes
its verdict, `/api/v2/health` carries it as `detection_deadman`, and
`tripped` is degraded under `?strict=1`: the pager and the notifier agree
on one threshold and one moment. The plain endpoint stays 200, since a
silent station is the one a container supervisor must not restart. The
doctor's model-integrity gate also covers a download cut off part-way,
closing the last half of `LC-2`.

### Fixed — the container knows what time it is

**The image carries zoneinfo and the compose file passes `TZ` through**
(`ON-7`, `NT-6`). Detections are filed under local hours, and in the
container those come from `TZ`, which needed zoneinfo the slim image did
not carry: every container station filed a season under UTC hours while
its operator read local ones, and a `TZ` the image did not know fell back
to UTC just as silently. The image now installs `tzdata`, compose sets
`TZ` from `.env` (UTC when unset), the entrypoint warns when it is unset or
unknown, and `--doctor` reads `TZ` first, warns when no zone is configured
at all (that used to be silence), warns on a zone it does not know, and
reports the zoneinfo version.

### Fixed — a flapping audio source is reported

**A source that keeps dying and coming back is called flapping** (`AD-3`).
Every signal was built on consecutive failure, so a source restarting every
minute and back in two seconds read as healthy everywhere: live at each
poll, attempt 0, backoff at its base, uptime strip green, and never down
long enough for the "still down" warning. Restarts are now counted over the
last hour; five or more put the count on the Station Health card, an issue
in the banner, a warning in the log, and the `flapping` station-health
condition on the notifier.

### Fixed — a microphone addressed by card index is watched for moving

**A resolving ALSA card index is an advisory, and a moved card is a boot
anomaly** (`AU-1`, `S-13`). The same microphone was `card 1` before a reboot
and `card 3` after it; a station addressing it as `plughw:1,0` passed the
doctor before and after, recording from whatever then sat at 1. `--doctor`
now grades a resolving index as an advisory naming the `CARD=<id>` form for
the very card it resolved to, and every start records the card id the
kernel reports behind each index-form device and compares it with the last
start: a change is the `audio_card_moved` boot anomaly, on the health
endpoint, in the station-health notification, and in the log.

### Fixed — the disk-full purge keeps every species

**A full disk no longer costs the rarest bird its only clip** (`S-2`). The
purge deleted the oldest tenth of the recordings, whatever they were, so
the single clip of the year's rarest bird went before the thousandth of
the commonest. It now takes from the most-recorded species first, the
lowest-confidence and then oldest clip within it, and never takes a species
below `--purge-species-floor` clips (5 unless set; `0` removes the floor).
The raw capture segments, which carry no species, are still drained
oldest-first.

### Fixed — "View on eBird" reaches a page

**The species page's eBird link is built from the eBird species code**
(`NP-1`). eBird keys its species pages on the six-letter code, and the link
put the scientific name in the path, so every one of them 404'd. The code
is on every station that has the geomodel's label file — column 1, which
the parser used to drop — and is now kept, loaded at startup, and used for
the link. A station without that file gets a line saying so, and which flag
supplies it, instead of a dead link.

### Fixed — the setup wizard's preference cards work from the keyboard

**The threshold and alert cards are real radio inputs** (`UX-1`). The seven
cards were `<div>`s with a click handler: nothing a keyboard could reach, so
an operator without a mouse could set neither the detection threshold nor
the alert mode during first-run setup. Each card is now a label around a
radio input in a labelled group: Tab reaches the group, the arrow keys move
within it, and the card draws a focus ring. The interaction gate drives
this in a real browser.

### Fixed — a purge that frees nothing stops, and says what is filling the card

**The disk-full purge stops when a pass achieves nothing** (`PR-7`). When
the card filled for a reason that was not recordings — the database, the
analytics store, the backup ring — the purge deleted a tenth of the
operator's clips every minute until every one was gone and the disk was
still full. A pass that removes recordings and lowers usage by nothing now
marks the purge ineffective; no further pass runs until usage falls, the
`purge` condition names what to look at, and `birdnet_purge_ineffective`
exports the mark.

### Fixed — six smaller gaps a station in a field would find

**The doctor grades the card, not the RAM disk** (`PS-8`): its disk check
read `--watch-dir`, which the unit always sets to the tmpfs; it now grades
the database's directory and, separately, the stream directory when that is
a different filesystem. **A missing `arecord` on an ALSA station fails the
doctor** (`LC-4`) instead of being skipped. **The timezone check no longer
trusts the wizard's row** (`ON-8`): a stored zone that is not a zone is
named as such rather than turned into a `set-timezone` instruction, and with
no row the host's zone is reported. **The Access tab's help trigger is a
button** (`UX-2`). **A Raspberry Pi's under-voltage and throttling are
read** (`NP-5`): a `power` condition while it is happening, a doctor check
that also remembers since boot, and `birdnet_pi_throttled_bits`. **Disk,
scratch, CPU temperature and the maintenance record are exported as
metrics** (`OP-3`).

### Fixed — what is wrong can be asked, the doctor reads the maintenance record, and a lagging analytics copy is a condition

**`GET /api/v2/health/conditions`** (`OP-4`) answers "what is wrong right
now?" with the station-health conditions as the notifier last evaluated them
and when it looked; they were push-only, so an operator who missed a push
could not ask. Alerts disabled no longer means evaluation disabled.

**`--doctor` reads the maintenance verdicts** (`OP-6`): the backup, the
integrity check and the offsite backup, through the notifier's own policy,
so "your backup has failed for a year" is something the diagnostic an
operator runs can now say.

**A detection the DuckDB copy refused is counted and conditioned** (`OP-7`):
`birdnet_analytics_mirror_failures_total`, `analytics_mirror_failures` on
`/api/v2/health`, and the `analytics-mirror` condition while the failures
are recent. It was a `warn!` line while health went on asserting
`"analytics": true`.

### Fixed — a bad configuration edit no longer takes the station down

**A file with an error is run around, not on** (`LC-6`). The daemon
validated its file at start and refused to run on an invalid setting; since
that ran in the new process after systemd had stopped the old one, a typo in
`LATITUDE` made over SSH became a restart loop with no web UI and no way
back. Every successful start now keeps `birdnet.conf.last-good`; a start
whose file has errors runs on that copy and reports `config_reverted`; one
with errors and no copy runs web-only on the file as it is and reports
`config_rejected`, so the diagnostics are reachable. `--apply-config <file>`
is the one-step safe change: it validates the candidate, refuses one with
errors, installs a good one behind a backup and restarts the service. The
doctor reports a configuration error as a warning naming what the start will
do, so the unit's preflight gate lets the start happen.

### Fixed — the doctor loads the model instead of weighing it

**`--doctor` checks the model as a model** (`ON-9`, `OP-13`). It used to
check that the file was larger than a megabyte, so a truncated download or
a stand-in passed, and it checked the metadata model for nothing but
existence. It now loads the classifier with ONNX Runtime and compares the
class width of the output the daemon scores with the labels file's count —
species are assigned by position, so a mispaired station names every bird
wrong — and loads the metadata model through the daemon's own loader, which
checks its width against the vocabulary. A file that is not a model, and a
model that is not the labels file's, both fail with both numbers named.

### Fixed — clips that vanished behind the database's back are reconciled

**A daily reconciliation pass finds clips the disk no longer has** (`S-14`).
The two retention passes stamp the rows whose audio they reclaim; nothing
stamped a row whose clip went any other way — the disk-full purge deletes
the oldest files by name and never opens the database, and so does a person
tidying a card — so those rows kept offering a play button that answered 404,
and nothing counted them. The pass stamps every such row, removes the
`.part` files a killed writer left behind, and reports both, to the log and
as the `birdnet_orphaned_clips` gauge.

**A clip that could not be converted keeps its `.wav` name** (`DD-36`). When
both `ffmpeg` and `sox` failed, the WAV was renamed under the `.mp3`,
`.flac` or `.ogg` name, so the file was a WAV under the wrong extension and
the detection said the wrong format. The WAV now stays a `.wav`, and the
row, the metadata step and the BirdWeather upload record what is there.

### Fixed — a start that lost the database says so

**The station keeps a boot journal outside its database** (`UP-3`). A data
volume that fails to mount leaves the station starting on the empty directory
beneath it, which looked exactly like a first run — a fresh database, an empty
chart, and nothing anywhere saying a season's detections are on a card that
is not mounted. Each start now writes what it saw (version, database path and
rows, whether the data directory is its own mount) to `boot-journal.json` in
the configuration directory, and the next start compares: a database that
held detections and holds none, a changed database path, a mount that is
gone, a downgraded binary. Each is an error in the journal, `boot_anomalies`
on `/api/v2/health` (a strict fault), and the `boot-anomaly` station-health
condition, so the notifier says it the morning it happens.

### Fixed — a restore no longer unpacks over the live database

**Restore from file checks, stops, swaps, restarts** (`UP-2`). It used to
run `tar` straight into the data directory over the open database with no
free-space check and no pause in recording, then ask the operator to restart.
It now lists the archive with sizes and refuses one the disk cannot hold with
headroom, refuses one whose database is not named as this station's, halts
detection writes, unpacks beside the database and integrity-checks the copy
there, and only then swaps the files in — the database by rename, so the
running process is never left reading a half-written file; recordings merged
clip by clip. Under systemd the station then restarts itself.

### Fixed — the analytics copy notices drift that nets to zero

**Net-zero drift between the database and its analytics copy is found and
repaired** (`DD-23`). The startup check compared three counts — rows,
rejected rows, unstamped rows — so a detection deleted and another written
back-dated onto the same day left every count where it was and the copy
wrong for ever: the owl in SQLite and not in DuckDB, the robin the other way
round, the totals equal. Both stores now reduce each day to a fingerprint of
the rows they hold, and the days whose fingerprints differ are rebuilt from
the database — day by day when a few differ, in full when many do.

### Fixed — a misspelt setting is named, not ignored

**An unknown key is reported with the key it was meant to be** (`LC-7`,
`O-7`, `RC-14`). `birdnet.conf` is a bag of strings and clap ignores any
environment variable it was not told about, so `CONFIDENC=0.90` in the file
and `BIRDNET_LATITUD=…` in a unit file were both accepted by everything and
changed nothing, with no journal line, no doctor note and no hint in the UI.
The station now knows which keys it reads — the config-file list is a
constant kept honest by a source scan in both directions, the environment
list is built from the command-line definition itself plus the handful of
direct reads — and names each key set and unread at startup and in
`--doctor`, with the nearest real name when one is close: *"CONFIDENC is not
a setting this station reads; did you mean CONFIDENCE?"* `.env.example` is
now gated against the code in both directions too; that gate documented five
variables the binary read and the file did not name, and removed the two
`BIRDNET_QUALITY_*` keys the file shipped, one uncommented, for a feature
that had been removed.

### Fixed — the species filter's state is on the station page

**`/station` says what the occurrence filter is doing** (`ON-12`). Whether
the metadata model is filtering, and how many species it currently admits,
reached Prometheus and nothing a person reads; a filter admitting zero species
is a station that records nothing, and an operator without a metrics stack
could not see it. The Pipeline row now carries it, and the status banner names
a filter admitting nothing as something to fix.

### Fixed — no child process can hang the station

**Every tool the station shells out to has a deadline** (`PR-8`). `ffmpeg`,
`sox`, `tar`, `df`, `arecord`, `mount`, `timedatectl`, `systemctl`, `apprise`:
twenty-odd production spawns, all reaped, none with a timeout, because
`std::process` has no bounded wait. Several of them sit on the single
event-processor thread or in a periodic probe, so one `df` on a dead network
mount or one `ffmpeg` on a device that stopped answering held that thread for
ever: the detection channel filled, the heartbeat stopped, and the watchdog
restarted the station with no line saying why. `birdnet_core::process::
run_with_timeout` is now the one synchronous wait — both pipes drained on their
own threads, the child polled to a deadline, then killed, reaped and reported
as `TimedOut` with the program and the limit in the message — and every spawn
in the workspace goes through it, with a limit sized to the job (a minute for
a clip conversion, hours for a backup archive). A source scan keeps the next
spawn from waiting on its own. Moving the clip conversion off the event thread
is deliberately not part of this; the register row says why.

### Fixed — the privacy threshold now does something

**`BIRDNET_PRIVACY_THRESHOLD` binds** (`S-5`). The filter inherited
BirdNET-Pi's rule — flag a chunk when a human label sits within the top
`max(10, 6000 × threshold / 100)` of its predictions — and applied it to a
detection list that was at most ten long and had already been cut at the
*detection* threshold. So the setting never mattered: speech was suppressed
exactly when it scored above the detection threshold, and lowering that
threshold to catch quieter birds silently tightened privacy while raising it
loosened it. The model now reports a per-chunk *human score* — the highest
confidence among its human classes, read from its output before either cut —
and the filter suppresses a chunk and its neighbours when that score reaches
the threshold. The value is a confidence on the same scale as a detection's,
and lower suppresses more; the CLI help, the settings form, the recording page
and the hardening guide now say so in one voice, where before one told the
operator to raise it towards `0.5` near a footpath and another said it routed
rows to a log that does not exist. A station whose loaded label set has no
human class is warned at start that the filter cannot fire.

### Fixed — every detection row now says which model made it

**A detection row records the model that produced it** (`R-1`, the register's
only open P0). A row carried where it was heard, when, at what threshold, with
what sensitivity and overlap — and nothing about the classifier. `install.sh`
pins the release checksum of the model and it never reached the database; the
shipped model is a pre-release, and the day an operator swapped it the rows of
two classifiers with different label sets and different calibrations shared one
table indistinguishably. Migration 43 adds `analysis_runs` — one row per
detection-daemon start: the SHA-256 and length of the model file, the SHA-256
and label count of the labels file, the geomodel's SHA-256 when an occurrence
filter is configured, the binary version and the run-wide settings — and a
`run_id` on `detections` and `quarantine` that references it, foreign key
enforced. The daemon hashes the files and registers its run on the processor
thread before it consumes its first event, and refuses to start without one:
a station that cannot say what model it is running does not record detections,
it stops and `?strict=1` says so. Every row the run inserts or quarantines
carries the id; an approved quarantine row carries it into `detections`; the
DuckDB mirror carries the same column. The CSV export ends in
`Run_Id,Model_Name,Model_SHA256`, the JSON export carries the same three per
row, and `/api/v2/detections` carries `run_id`; `BirdDB.txt` is left at
BirdNET-Pi's twelve fields because its consumers count them.
`GET /api/v2/analysis-runs` lists the runs with their row counts. The doctor
hashes the model on disk and warns when it is not the model of the last run.
Imported history and rows older than the migration are NULL, never a guess.
The demo seeder registers a run whose identity is the checksum of the word
`demo`, so it cannot be mistaken for a release model.

**The exports say which clock they are on, and the instant trigger no longer
invents a time** (`R-8`). Every export was local `Date`/`Time` with no
offset, and the instant migration 32 put on every row reached no
`DetectionRow` field, export or route. `detected_at_utc` is now on
`DetectionRow` and `/api/v2/detections`; the CSV export carries `Event_Date`
(the wall clock with the offset that was in force, RFC 3339, so the two
passes of a repeated autumn hour export as `+01:00` and `+00:00`) and
`Detected_At_UTC`, and the JSON export `event_date`. Separately, the trigger
that stamps rows nothing else stamped — imports, the backfill — collapsed a
local time that does not exist (01:30 on London's spring-forward day) onto the
instant an hour before it, because that is what SQLite's `'utc'` modifier does.
Migration 44 converts and converts back, and a time that never happened keeps
a NULL instant. Rows already stamped are left alone: the same check over
history would fire on every row of a station whose zone has since changed.

**The sign-in form is throttled** (`O-6`). The "Too many attempts" page
existed and the flag that rendered it was set in one place: a unit test.
`login_submit` never set it, so the global limiter's tens of posts a second per
address were all Argon2id hashes the Pi computed for whoever asked. Five
failures from one client address inside fifteen minutes now answer `429 Too
Many Requests` with a `Retry-After` and the form disabled, before the password
is checked — a correct password from a throttled address gets the same answer
and no cookie. Each refusal is in the audit log as `auth.login.throttled`; a
successful sign-in clears the address; a restart forgives everything; another
address is never affected, because a blanket lock is a denial of service any
stranger can trigger against the operator.

**Every detection sent to BirdWeather now carries its soundscape** (`DD-32`).
`Client::post_soundscape` was written, tested and never called; the daemon
posted six bare fields, so nothing on BirdWeather from this station could be
listened to and nothing there could be verified. The daemon now uploads the
clip as a soundscape first — the bytes as the body, `?timestamp=&type=` on the
URL, the id from the answer, the shape both reference projects use — and posts
the detection with `soundscapeId`, the detection's start and end inside the
clip, and `algorithm` where BirdWeather has a name for the model (`2p4` for
V2.4; omitted otherwise rather than mislabelled). The row keeps the id
(`birdweather_soundscape_id`, migration 45). A failed upload posts the
detection without it, as every post was before. The post's timestamp was the
local wall clock labelled `Z`, an hour or more wrong on every station outside
UTC; it is now the local time with its offset.

**The setup wizard asks for the admin password first, and the browser that
sets it owns the station** (`DD-14`). With no password set, every `/admin/*`
page was open to anyone who could reach the port for as long as the operator
did not act, the six-step wizard never asked, and the only browser route to a
first password was a form labelled "Reset password" that then signed the
operator out without saying so. The Welcome step now carries a password block
on a passwordless station; finishing setup hashes it onto the admin account
and hands that browser a session, so the next click is not a login prompt. A
mismatched or short password saves nothing and says why; blank means "not
now", and the dashboard's first-run checklist then says in its first row that
the station is open. The accounts form reads "Set password" on such a
station and signs the browser in the same way; a rotation signs the account's
other sessions out and says how many.

**Login sessions survive a restart** (`DD-15`). The signing secret was read
from `BNB_SESSION_SECRET` or `CADDY_PWD` in the environment only; the
installer writes `CADDY_PWD` to the config file, nothing exports it, and the
unit sets neither, so every bare-metal install ran on a per-process random
secret and every session died on restart while the accounts page promised
fourteen days. The station now generates a secret once and keeps it beside
its database (`session.secret`, mode 0600), ahead of the password derivation.
Because the secret no longer rotates with the password, rotating `CADDY_PWD`
signs the old sessions out at the next start, and rotating on the accounts
page signs the other sessions out — the promise the page always made, now by
the mechanism that can keep the operator's own session.

**The health verdict sees a full card, a read-only remount and a vanished
data volume** (`DD-19`, `DD-20`, the writability half of `PS-9`/`AD-4`). On a
100 % full volume `/api/v2/health` answered `200 "healthy"` while
`/api/v2/system/disk` said `critical`, DuckDB had been quarantined on `ENOSPC`
and the admin bootstrap had failed; after the data mount was detached the
station kept writing into the directory underneath it and reported the parent
filesystem as fine; and nothing probed writability at all, so a read-only
remount left every detection classified and discarded behind a green probe.
A watch now probes the data directory once a minute — a create/sync/remove
write, the directory's device id against its parent's, and the `df` verdict —
and the health endpoint reads it: unwritable or vanished is degraded on every
reading, like a halted ingest; a critically full or unanswerable disk and a
failed admin bootstrap are `?strict=1` faults. The body carries `data_volume`
and `admin_bootstrap`; the station-health alerts gained a `data-volume`
condition; the disk endpoint answers a failed `df` with `503 unknown` instead
of `500`; and the journal gets one line on a transition instead of one a
minute.

**Four first-run gaps** (`ON-4`, `ON-5`, `ON-10`, `ON-11`). The wizard's Done
step said detections would roll in within a minute or two; the settings it had
just saved take effect on the next start, which `/admin/settings` said and
the wizard did not — it now says so and links the restart. `CONFIDENCE=0.99`
and `SF_THRESH=0.5` drew no finding: the validator warns above 0.95 and 0.3,
as it always warned below 0.1. The first-run checklist's model row was a
hard-coded tick that read "Model bundled … included" on a process with no
model; it now reads the detector's liveness and says "Detector not running"
with a link to the doctor. The compose file's health check says why it is not
`?strict=1` and where the strict probe is for.

**A clip is on disk whole or not at all** (`S-4`, `PS-7`). Clips were written
straight to their final names, so a power cut mid-write — the field station's
ordinary way of stopping — left a truncated WAV the database row pointed at
for ever, indistinguishable from a short recording; `sync_all` appeared at one
production site in the whole workspace, none of them in the audio path. Clips
and converted clips are now written as `name.part.ext`, synced, renamed into
place, and the directory synced. The gate kills a worker mid-write and looks
at what is left.

**A live sibling's lock is not corruption, and one process owns a data
directory** (`DD-25`). A second instance on the same data directory — a
`systemctl restart` overlapping a slow shutdown, or the binary started by hand
beside the unit — met DuckDB's "Could not set lock on file", took it for a
damaged store, moved the first process's live analytics database aside and
rebuilt an empty one, and only then died on the port. A lock conflict is now
retried for the shutdown grace (30 s) and, if still held, reported as locked
with analytics off for that start; the file is never touched. And before
anything opens a file there, the process takes an advisory lock on
`birdnet.lock` beside the database, waits the same grace for a previous
instance to go, and otherwise refuses to start saying so
(`BNB_INSTANCE_LOCK_GRACE_SECS` lengthens the wait).

**No certificate is minted on an unset clock, and the self-signed one renews
while the station runs** (`NT-2`, `NT-3`). A first boot before NTP minted a
local CA and leaf valid 1969-12-31 to 1971-02-01, nothing regenerated them
while the process ran, `--doctor` reported "valid 397 days" from the
configured number, and recovery was physical. A clock below the plausibility
floor now mints nothing: the station serves plain HTTP on its address, says
so, and restarts itself once the clock is set so the next start mints a real
certificate; material minted on a good clock is still served on a bad one.
Separately, the leaf was renewed only at process start, so a station up past
day 397 served an expired certificate; it is now checked daily and swapped
into the running listener without a restart. The doctor reports the
certificate's real expiry date.

**A dead MQTT broker is an alert, and "Test all channels" tests all of them**
(`DD-22`, `DD-24`). A silent broker was handled soundly and reported at
`debug!` and on one Prometheus gauge; a broker dead from boot never reached
even that. The presence loop now dates the outage and keeps the last error,
the station-health alerts carry an `mqtt` condition after ten minutes, and the
health body says `mqtt: off|connected|disconnected`. The notifications test
page's "Test all" answered "All configured channels passed" with MQTT dead,
because it tested push and BirdWeather only and called an empty run a pass;
it now publishes a real test message over MQTT and sends a test email through
the configured notifier, and a run in which every channel was skipped says
that nothing was tested.

### Fixed — the head of the audit's queue, and what running the station found

The queue at the top of `docs/UNATTENDED_DEPLOYMENT_AUDIT.md` §6 was worked in
the order it was written, and every document in `docs/` was then reconciled
against the code a second time, in place. Each fix below landed with a gate
that was first watched failing against the code it now guards; the failure
text is in each commit message.

**The eBird export now produces a checklist eBird can accept** (`dad46d1`,
`R-18`, `R-19`). It read latitude and longitude as 0, 0 while the real
coordinates sat in settings; it applied no confidence floor and no
one-per-hour deduplication, so one blackbird detected two hundred times went
out as two hundred birds; and it hard-coded `Protocol=S, Observers=1`. It now
takes its coordinates from the station's settings, leaves them blank rather
than inventing an equator crossing when none are set, rejects a half pair or an
off-globe pair with 400, applies a 0.75 confidence floor
(`?min_confidence=`), writes one record per species per hour with `Number=X`
and the detection count in the comment, and takes protocol, observers, state,
country and completeness from the caller. Its rows come from the analytic
view, so a rejected or excluded detection no longer reaches a public database.
The CSV, JSON and BirdDB exports still read the raw table with no verdict
column; that is recorded, not fixed (`R-17`).

**An operator with only a browser can now run the doctor and download a
support bundle** (`9f42652`, `OP-1`). `/admin/doctor` renders the full
`--doctor` report; `/admin/doctor.json` serves the same document
`--doctor-json` prints; `/admin/support-bundle` streams the archive
`--support-bundle` writes, built beside the database and removed afterwards.
The binary hands the web layer two hooks at startup; the hooks are read-only,
so a `--fix` on the command line is never implied by a `GET`. A process built
without the hooks, as tooling and tests build it, answers 503 with a body that
says what is missing, rather than 404 like a typo or 500 like a bug. The end-to-end
gate boots the real binary and untars what it downloads.

**The installer verifies the geomodel pair by checksum, not by presence**
(`c31da32`, `ON-3`). The classifier download had been taught that a partial
download is a file; the geomodel half of the same module still treated
`[ -f ]` as installed, so a fetch that dropped at 60 % was "already present"
on every re-run and every `repair`, the config writer then pointed the daemon at
it, and the daemon refused the pair on every start with the occurrence filter
silently off. Third instance of one shape.

**A detection daemon that dies after boot is now reported as stopped**
(`d0df731`, `OP-2`, the second clause of `PR-5`, the remainder of `OB-4`).
The daemon's `AtomicBool` was written once at startup and no exit path cleared
it, so `/api/v2/health?strict=1` kept answering "running" for a thread that had
returned. The loop thread now holds a guard that clears the flag when it
returns, however it returns, and the binary mirrors that flag into the health
flag every five seconds.

**Configuration values are redacted by shape** (`373ceb2`, item 2.17,
`OB-10`, `OB-11`, `RC-20`). The old rule ran an email-address redactor over
every value and turned `rtsp://user:pass@camera.local/stream` into
`***@camera.local/stream`, destroying the scheme an operator needs to recognise
the source while keeping the host. One function now decides by shape: an
`rtsp://` URL keeps scheme, user, host and path and loses only the password; an
`http(s)://` URL keeps its host and loses its path, because the token in a
heartbeat or webhook URL is the path; any other scheme keeps only the scheme,
because Apprise-style URLs carry their tokens in the authority; lists are
split and each element treated alone. The support bundle and
`GET /api/v2/settings` share it. `1c1be04` re-pointed the one API gate that
still asserted the old host-keeping output.

**A forward clock step no longer deletes the clip library** (`e88a60d`, item
1.11, the remaining half of `NT-4`, `AD-1`). The plausibility check was a
floor only, so a clock that jumped fifty years forward passed it and every
date-relative purge reclaimed everything. The check is now a range, with a
ceiling at 2064-01-01, and a watch compares the wall clock with the monotonic
clock from the first plausible reading and refuses retention when the two
disagree by more than 400 days. Clip and log retention, the acoustic-health
pruner and the weather pruner all consult it. The residual: a jump inside 400
days, or one that happens before the process starts and lands inside the range.

**The CSRF defence is asserted, and the doctor's check set is written down
once** (`95a8272`, `RC-5` to `RC-8`, items 8.1 and 8.2). `security.rs`
argued that no synchroniser token was needed because there was no session to
bind to, long after a session cookie existed; the actual defence is
`SameSite=Lax` plus the same-origin check, and nothing asserted either. Both
cookies are now asserted `HttpOnly` and `SameSite=Lax`, the module doc names
the two mitigations, and `src/doctor.rs` has a table of its twenty-one check
families that a source-scanning gate holds against the check functions in the
tree, so a check cannot be added without being listed.

**The species rollup is keyed by provenance** (`dd10fe7`, the lasting half of
`RC-3`, item 3.22). The reader-side fallback that made the species list correct
on a station that excludes imports did so by putting exactly those stations
back onto the unbounded scan migration 30 existed to remove. Migration 42 adds
`is_import` to `species_summary`'s key, rewrites the three triggers, and the
readers select the rollup whole or `WHERE is_import = 0`. Both answers come
from the rollup; nobody pays the scan.

**The documented analytics opt-out was unreachable** (`12776c7`, `RC-36`).
The code has treated an empty `--analytics-db` as "run without DuckDB" since
analytics became a default feature, and the documentation promised that form,
but clap's stock parser rejected an empty value with exit 2 and the Docker
entrypoint's blank-variable scrubber unset an empty `BIRDNET_ANALYTICS_DB`
before the binary saw it. Removing the flag, as the unit template advised,
falls back to `<database>.duckdb` and turns nothing off; the only working
opt-out was a `--no-default-features` build. Same defect `--image-cache-dir`
had; same fix.

**Every detection row now records where and under what settings it was
made** (`e3f9b80`, `R-2`, `UP-1`). The processor wrote `Lat`, `Lon`, `Cutoff`,
`Sensitivity` and `Overlap` as NULL on every row it inserted, while BirdNET-Pi
fills all five; an exported dataset could not be located, and a row's
confidence could not be read against the threshold that admitted it. The
daemon now builds the run's provenance once from the same resolved values the
model uses and the disposition decision carries the effective threshold, per
species where one is set, after the dynamic adjustment, so `Cutoff` is the bar
each row actually cleared. A station with no coordinates keeps NULL, never
0, 0.

**Then the station was run, and what running it found was fixed the same
day.** Six probes drove the branch with the server up — the interface, the
first run, adversity, research credibility, operability without a shell, and
both upstreams' source at their current tips — and their findings are
`docs/UNATTENDED_DEPLOYMENT_AUDIT.md` §3.14. Thirteen were closed on the spot:

* **Rejected detections left the station through every bulk export but eBird**
  (`441188a`). The CSV, JSON and `BirdDB.txt` exports read the raw table, so a
  reviewer's rejection was undone at the one surface where a dataset is about
  to be cited; all three now read the analytic view. `R-17`, finished.
* **The first-run wizard could not be finished on the station the installer
  actually produces** (`60ead63`). Its page was public and its save was behind
  the admin gate; on a headless install with a generated password the first
  page load was the wizard, Finish answered a bare 401, and following that
  answer after signing in landed on a 405 with every answer gone. The page
  now sits behind the same gate as its save, and an open station still shows
  it to anyone. The same save persisted `latitude=999`, `longitude=abc`,
  `timezone=Mars/Olympus` — the overlay then dropped them silently and the
  doctor recommended `timedatectl set-timezone Mars/Olympus` — and wrote half
  a coordinate pair, so two runs could leave a station at a point nobody
  typed. A location is now a real pair or nothing, and a zone must exist.
* **The health badge went green for a source that had never captured**
  (`bb087fa`). A source added while the daemon was down read as "not known
  yet" and graded "Healthy" while `?strict=1` said the daemon was stopped.
* **A restore deleted the database it replaced** (`2eeadfb`). A backup is
  older than the file it replaces, so everything recorded after the backup
  was in that file and nowhere else; the restore now sets it aside under the
  quarantine name the doctor scans for, and the log says what it holds.
* **The doctor called a quarantined detection database lossless**
  (`2eeadfb`): "Analytics was rebuilt automatically from SQLite, so no
  detections were lost" was printed for a `birds.db.corrupt.*` holding the
  whole history. The check now tells the two stores apart.
* **A page region that failed to load stayed a skeleton for ever**
  (`9b7e81c`), including when the station died under an open dashboard; the
  shell now replaces it with a notice that names the failure. And Enter on
  "Show more" no longer drops keyboard focus to `<body>`.
* **An approved quarantine is a confirmed detection** (`dfdac5e`); it used to
  be indistinguishable from an unreviewed auto-accept.
* **The support bundle downloaded from the browser carries the process's
  recent log** (`8aba101`); on any install without a persistent journal it
  had carried no ordinary log line at all.
* Smaller: the eBird location name falls back to the installer's site name;
  the species admin page's empty-state text uses a text token rather than a
  border token; the installer no longer sends operators to a microphone
  picker the wizard does not have; the installation guide names the
  geolocation provider the code calls.

**The documents were reconciled a second time** (`480d56f`, `200e768`,
`849a2e2`, `1334320`, `bd0994b`, `8e6806f`, `290009b`, `df5fa7a`). Eleven
planning and audit documents, the architecture set, the mdBook, the design
handover and the root documents were re-read against the source by fifteen
independent read-only passes and then corrected in place by eleven editing
passes, each claim re-verified by a command before it was written and every
count re-derived. What that found is in `docs/UNATTENDED_DEPLOYMENT_AUDIT.md`
§0 and §6, including the two counts that were wrong again.

### Fixed — eight ways the station vouched for something it had not checked

This project accumulated eleven planning and audit documents written at
different times. They were reconciled against the source in one pass: every open
item re-verified from the code rather than from another document, and stale
claims corrected in place rather than annotated, because a document that
contradicts the code stops the next person running the real check. That pass
found its own defects, and they share the shape the last one did — a rule
implemented in two places, where the second place learned only half of it.

**A database truncated to zero passed the daily integrity check, every day.**
`check_integrity` was taught to look for SQLite's sixteen-byte magic, because
SQLite opens a zero-length file as a brand-new empty database and answers
`quick_check` with "ok". The guard went on that function alone.
`full_integrity_check` is the one the *running* station uses — the daily
scheduled check whose verdict halts the detection writes, `--check-db`, and
`--doctor` — and it had no guard. So a `birds.db` truncated while the station
was running, which is what a power cut during an SD card's wear-levelling
relocation produces, was reported healthy for the rest of the year. The ingest
halt never tripped. An operator who ran `--check-db`, which is what the failure
message tells them to do, was told the database was fine. Only the next reboot
could notice.

**Opening a chart re-admitted another station's imported history into every
behavioural analytic.** Two crates create a view called `detections_ts` on the
same DuckDB connection, and a gate holds their definitions equal. Migration 34
gave that view a second rule — exclude imported detections when the operator has
asked for it — which has no constant to compare, because the flag lives in
SQLite and the time-series crate cannot see it. Constructing a time-series
executor overwrote the view, so sessionize, retention, funnel, next-species,
co-occurrence and phenology silently counted a foreign site's records until a
later sync happened to reinstall the right definition. The view now has one
owner; a read no longer rewrites the catalog underneath the other crate.

**The species list ranked a bird the station had never heard as its commonest.**
`species_summary` is a trigger-maintained rollup that filters on the review
verdict alone, and it could not learn migration 34's rule because the rule
depends on a setting the operator can flip and the rollup's key has no
provenance dimension. Measured on two of the station's own detections and three
imported, with the exclusion on: the analytic view reported one species and two
rows, while the species list reported two and put the imported one first at three
detections. The per-species detail page reads the view directly and was right
throughout, so the list and the detail page disagreed about the same species.
The five rollup readers now fall back to the view — only on a station that both
has imports and excludes them, so everyone else keeps the rollup's bounded cost.

**The dawn chorus copied half of that same rule** and its comment claimed it had
copied all of it. It was the one surface still counting an excluded site's
records after the operator excluded them, and, being a chart, the most likely to
be believed.

**The watchdog that exists to prove the station is working was blind to a
station that never worked.** The detection deadman measured "seconds since the
last detection", which is unmeasurable when there has never been one, and folded
that into "nothing to say" with a comment about brand-new stations not alarming
on first boot — right about the first hour, with no time bound. A station whose
microphone, gain, confidence threshold or occurrence filter was wrong on the day
it was installed detected nothing on day one and nothing on day three hundred,
in silence. It now measures against `recording_effort`, the station's own record
of how long it has been listening: a new station still says nothing, and one
that has listened past the threshold and heard nothing says so — in different
words, because "no detections for 25 hours" sends the operator to the weather
and "recorded 25 hours and never detected a single bird" sends them to the
configuration, which is where the fault is.

**The container adopted whatever model file it found.** `ensure_model_file`
returned on the file's presence alone, three lines below a comment promising a
sha256 check, so a truncated or half-restored model became final for the life of
the volume — while `--doctor` accepts any model file over a megabyte and the
compose healthcheck cannot see a stopped detection daemon. The installer closed
this on bare metal and the container was never brought along. Its `verify_sha256`
also returned success when `sha256sum` was missing, which is the same shape the
installer's own test suite exists to prevent: "we could not check this" must
never return what "this checked out" returns.

**The backup ring could be eaten by a corruption the guard could not see.**
`PRAGMA quick_check` is not a faster `integrity_check` with the same answer: it
checks page structure and skips verifying that index content matches table
content. A database whose indexes have quietly stopped agreeing with the rows
they point at passes it cleanly, and queries using those indexes then return
fewer rows with nothing to see afterwards. Two decisions that overwrite data
were made on that check — the guard that refuses to snapshot a corrupt source,
whose own comment explained that otherwise "the rolling backup ring would
overwrite the last good backup with a copy of the damaged DB", and the walk that
picks which backup gets restored over the live database. Both now use the deep
check, as does the admin backup page. Demonstrated by a gate: a good backup of
5 000 rows sitting behind a corrupt one of 20 000, and recovery took the corrupt
one. The verdict on the live database at boot deliberately stays on the cheap
check, because the deep one is 24× slower on a path that runs before the server
starts listening, and the daily check covers it.

**A restore that could not finish was the thing that lost the history.**
`restore_from_backup` deleted the database it was replacing before it knew it
could write the replacement, so any failure after that point — a full card,
another I/O error from the card that caused the corruption in the first place,
a power cut — left neither. The station then read that failure as "corrupt and
no good backup exists", quarantined, and started fresh, while a perfectly good
backup sat beside it waiting to be rotated away by the weekly ring. Reproduced
on a 12 MB filesystem with a 2.7 MB backup and 1.3 MB free: the live database
was left at zero bytes with no rows readable. The restore now builds the
replacement alongside, verifies it, and swaps it in by rename; a good backup
that could not be written is reported as exactly that, names the file that is
still intact, and stops the station rather than starting it fresh — which on a
full card would not have let it record anyway.

### Found and not fixed

Recorded so nothing discovered goes untraced. The full register is
`docs/UNATTENDED_DEPLOYMENT_AUDIT.md` §3.12 to §3.14 — 158 rows with severity,
evidence and a remedy each (counted 2026-09-08 by first id per row: 36 + 87 +
35). After this branch it holds one open P0 and 57 open P1s. The ones an
operator or a researcher should know about, worst first:

* **No detection row records which model produced it** (`R-1`, raised to P0).
  There is no model column, no `analysis_runs` table, and nothing in the tree
  that writes a model version or checksum to the database. A station whose
  model is swapped mixes two classifiers' verdicts in one table with nothing to
  tell them apart, which for anyone analysing the data is a silent wrong
  answer. The per-row provenance this branch added (coordinates, threshold,
  sensitivity, overlap) makes the gap sharper, not smaller.
* **Every detection sent to BirdWeather is unverifiable there** (`DD-32`).
  The soundscape upload is written and never called; the daemon posts a
  six-field detection with no soundscape id. Both reference projects post the
  clip first and stamp its id on the detection.
* **Login has no working throttle** (`O-6`). The "Too many attempts" branch
  exists and the flag that reaches it is set only inside a test.
* **With no password set, every admin page is open to anyone who can reach
  the port until the operator acts, and the wizard never asks for one**
  (`DD-14`).
* **On a full card the station says it is healthy** (`DD-19`): `/api/v2/health`
  200 while the disk endpoint is 503, the analytics store has been
  quarantined on ENOSPC and the admin bootstrap has failed.
* **Exports carry local time with no offset**, and the import path's local-to-UTC
  conversion invents an instant for the non-existent spring-forward hour and
  picks the second autumn hour silently (`R-8`).
* **Every login session dies on restart on bare metal** while the access
  page promises fourteen days: the signing secret is read from the
  environment only, and the installer never puts it there (`DD-15`).
* **A second instance on the same data directory quarantined the first's
  live analytics store** during an overlapping restart, treating a lock as
  corruption (`DD-25`).
* **The analytics drift repair counts rows**, so a delete and a back-dated
  insert that net to zero leave the two stores permanently disagreeing
  (`DD-23`).
* **Recording effort never reaches an export**, in this project or either
  reference — so an exported zero cannot be told from a dead recorder
  (`FR-5`), and **Darwin Core cannot be emitted** (`R-DwC`).
* **Seven onboarding preference cards cannot be reached from a keyboard**
  (`UX-1`); the species avatar chips, the failure-state pills and the white
  text on accent fills fail contrast by measurement (`DD-29`); and
  `/station/data` scrolls sideways on a phone once there is one backup
  (`DD-30`).

### Added — a station can now be changed over its API, by something that is not a browser

The `/api/v2` surface was entirely read-only. A grep for `post(`, `put(`,
`delete(` and `patch(` across the fourteen routers nested under it returned
nothing, against the reference implementation's fifty-four mutating routes.
Every state change in this product was an HTMX form post that returned an HTML
fragment, behind a same-origin check any script satisfies by setting a matching
`Origin` header — so it was neither a security boundary nor a contract anyone
could build on. Home Assistant and Node-RED could read a station and never act
on one, and because our own front end was the only client, a change to fragment
markup would silently break whatever automation existed in the wild.

Seven mutating endpoints now exist, and they are the only ones under `/api/v2`
that change anything, plus one read that lives behind the same token:

| Endpoint | Does |
|---|---|
| `POST /api/v2/detections/review` | record `confirmed` / `rejected`, or clear a verdict |
| `POST /api/v2/detections/lock` | protect a clip from the purge and the retention sweep |
| `POST /api/v2/detections/unlock` | return it to the ordinary rules |
| `POST /api/v2/detections/delete` | remove the detection |
| `POST /api/v2/detections/batch` | apply one of those four to up to 500 detections |
| `GET /api/v2/settings` | read every setting, with credentials removed |
| `PUT /api/v2/settings` | change one or more settings |
| `POST /api/v2/control/restart` | restart the station |

**They are off by default, and the default is the safe one.** Set
`BNB_API_TOKEN` — resolved from the config file then the environment, the same
precedence `CADDY_PWD` uses — and each endpoint accepts
`Authorization: Bearer <token>`. Leave it unset and all eight answer `404`: the
write surface does not exist rather than existing unprotected. That is
deliberately the *opposite* default from `CADDY_PWD`, where an unset password
leaves `/admin` open; these endpoints never touch that bypass. A token under 32
bytes is refused, leaving the API off, and `--doctor` now says so — a knob that
was set and silently ignored is the failure an operator cannot see.

The token is not a settings row. This project already deletes plaintext
credential rows a previous build's settings form could write, and the page that
renders settings is unauthenticated on a default station; a token there would be
a credential published on a public page. Only a SHA-256 digest of it is held in
memory, because the state it would live in derives `Debug`.

The endpoints are mounted in their own router rather than the public one, which
is asserted read-only by the gate that exists because thirteen mutating routes
once sat there. They are exempt from the same-origin CSRF check — a cross-site
form cannot set an `Authorization` header, which is the whole premise of the
check — and the exemption is scoped to those paths, with a test that fails
if it widens: a rule keyed on the header instead of the path lets a
cross-origin page write through, and the test watches the detection actually
disappear.

Every change is written to the audit log with no user and `via=api`, because a
token is not a person. A settings change records the key *names* only: `/admin/audit`
renders that table, and an entry reading `birdweather_token=…` would put a
credential on a page.

**One request instead of forty.** `POST /api/v2/detections/batch` applies one
of the four detection operations to a list of them, which is the shape triage
actually takes: a night of false positives is rejected in one call rather than
forty authenticated round trips.

It is deliberately **not** a transaction, and says so rather than letting a
caller infer one from the word "batch". Each detection goes through the same
`AppState` method the single-detection endpoint calls, because those methods
write SQLite *and* the DuckDB analytics copy — `tests/analytics_divergence.rs`
exists because a handler that reached past the pairing for one shared
transaction would compile, pass every contract test, and silently desynchronise
the two stores. One transaction is not worth a second implementation of "delete
a detection", and two new gates in that suite now cover this route: restoring
the raw-SQLite shortcut leaves the analytics copy holding rows SQLite no longer
has (`left: 3, right: 1`) and verdicts it never recorded.

A key that matches nothing does not stop the batch either. A client working from
a list a few seconds stale would otherwise have forty good deletions refused
because three rows had already gone, so every detection gets its own entry in
`results` and `applied`/`failed` sit at the top level. The response is `200`
whenever the *request* was well-formed even if every item failed, which is a
real hazard for a client that reads only the status code — `207` was the
alternative and surprises the shell scripts this exists for, so the trade is
stated in the endpoint's own documentation and in the manual instead.

An unknown `op`, and `status`/`notes` sent with an `op` that is not `review`,
are refused rather than ignored — the same rule as a misspelled settings key.
The list is capped at 500: measured at about 0.44 ms per item on this
workspace's debug build (100 in 39 ms, 500 in 229 ms, 1000 in 434 ms), and 500
is also exactly one page of `/admin/audit`, which caps at 500 rows and then says
so. Every detection actually changed gets its own audit row under the same
action name a single call writes, because a single row reading "deleted 40
detections" cannot answer "what happened to that recording?".

**Settings are readable, with the credentials taken out.** Automation has to read
what it is about to change, so `GET /api/v2/settings` exists — but handing a
scripted client `email_smtp_pass` in the clear is not an acceptable price for it.
The redaction is the support bundle's, not a second implementation: those rules
moved out of `--support-bundle` into `birdnet-core` and both callers now use the
one copy, because two copies of "which values are secret" is precisely the
arrangement that once shipped an open `/admin` while the station's own diagnostic
reported it protected. They deny in two directions — by key name, and by value
shape for the credential inside an `apprise_url` that does not *look* like a
secret. The by-shape rules are blunt — `ntfy://alice:hunter2@ntfy.example/topic`
comes back as `***@ntfy.example/topic`, host and path kept, scheme and username
not — and the gate pins that exact string rather than "the password is gone",
because an earlier version of it asserted only the absence of the password and
the presence of the host, both true for a reason it had not established. A
withheld value is replaced with `***REDACTED***` rather than omitted,
so "you may not read this" stays distinguishable from "this was never
configured", and the response names those keys.

`PUT` refuses two things rather than accepting them quietly. An unknown key: a
misspelled `confidence_treshold` answering `200` would report a change that never
happened. And the literal `***REDACTED***`, which is what the `GET` hands back —
a client that reads the whole object, edits one field and writes it back would
otherwise overwrite the real SMTP password with the placeholder. Values may be
strings, numbers or booleans, and each goes through the settings page's own
normalisation and only-write-what-changed rule rather than a second writer; the
gate for that asserts the *stored* form of `"51,5"` is `51.5`, because a handler
that stored the string it was given would also answer `200`.

`POST /api/v2/control/restart` shares its systemd detection and its delayed
self-`SIGTERM` with the admin page's button. It answers `503` outside systemd,
where nothing would bring the station back — a cheerful `200` there would tell
the caller the opposite of what was about to happen — and the audit entry is
written before the decision, so a refused restart is recorded too.

**Found while doing it, and fixed:** a unit test named
`the_route_table_is_the_router` did not check the router. It compared the route
table against itself and passed with `.put(write_settings)` deleted from
`router()`; `axum::Router` exposes no route list to assert against. It is now
`the_route_table_is_well_formed`, saying what it does, and
`every_documented_route_is_mounted` sends a real request for each table entry —
which is what noticed. `openapi.json` is checked against the same tables in both
directions: every bearer-gated route is documented *and* documented as
bearer-gated, and every other operation is documented as anonymous. Redocly's
`security-defined` rule does not catch a missing per-operation `security`, and
the document's default is anonymous, so a write documented without one would
hand every generated client a `401` it had no way to anticipate.

The audit log's action scanner reads string literals in the lines *after* a
`crate::audit::audit` call, and its window was seven. The batch endpoint picks
its action name with a `match`, which rustfmt expands to one arm per line, so
the fourth literal — the one that deletes a detection — sat at offset seven and
fell outside. The window is now ten. Widening can only ever find *more*
literals, so it strengthens the undocumented-action assertion and cannot weaken
it; checked at 7, 8, 9, 10, 12 and 15 across the crate, the set found is
identical (32 actions) and no unrelated string is mistaken for one.

**And a worse one, found by CI.** The restart endpoint read
`INVOCATION_ID`/`JOURNAL_STREAM` on every request, which made its behaviour a
property of the calling process's environment. A GitHub Actions runner sets
`INVOCATION_ID`, so in CI the endpoint took the *signalling* branch and the test
binary sent itself `SIGTERM` 400 ms later. It surfaced only because one test
asserted the environment before calling; `every_documented_route_is_mounted`, in
the same binary, `POST`s every route in the table with a valid token and had no
such guard.

Whether systemd is supervising the process is a property of how it started and
cannot change while it runs, so it is now read once by `supervised_by_systemd()`,
recorded on `AppState` by `app.rs`, and read from there by both restart
handlers. A state built without it is not supervised — the safe answer, and the
one every test wants. Splitting the decision (`restart_outcome`) and the
rendering (`restart_fragment`) out of the signalling makes both assertable
without a test process killing itself, which they were not before.

`tests/the_restart_endpoint_cannot_signal_a_test.rs` holds the two halves no
behavioural test can see: that `app.rs` still records the answer — delete that
one line and every station refuses every restart for ever, while the suite stays
green because its states are all correctly unsupervised — and that nothing reads
those variables for itself again. The exemption in the second is scoped to
`supervised_by_systemd`'s own body rather than to the file it lives in, because
the original defect was four lines below it; restoring that defect exactly is
what the gate was observed failing against.

**Found while doing it, and not fixed:** two things.

The audit log's vocabulary gate finds action names by scanning for the literal
text of the `audit` call and reading string literals near it. A local helper that
takes the action as a parameter compiles, works, and is invisible to it — which
is how the four detection names first went undocumented, and how any future call
site can. The calls here are written out at each site so the existing mechanism
sees them, and the reason is stated in the source, but the gate itself still has
the hole.

The settings redaction catches a credential in a URL's *authority*
(`ntfy://user:pass@host`) and a key whose *name* looks like a secret. It catches
neither for a URL whose credential is a path segment — a heartbeat ping URL is
exactly that, and `heartbeat_url` is returned in full. This is stated in the
endpoint's own documentation and in `openapi.json` rather than left for a reader
to discover, but it is not fixed: a rule that redacted URL paths generally would
take `rtsp://camera.local/stream` with it, and the path is what makes an RTSP
problem diagnosable.

### Fixed — a database found corrupt was written to for months

The "never write to a corrupt database" policy existed only at startup. There
it is thorough: before the state is built, the daemon verifies the file,
restores from the newest backup that itself verifies, and — failing that —
quarantines it rather than opening it.

The *daily* check had none of that. It ran `PRAGMA integrity_check` and, on
failure, did two things: wrote one `error!` line, and recorded the verdict. The
station then kept inserting detections into the corrupt file until somebody
rebooted it, which on an unattended station is months. Compounding it, and not
noted in the audit finding: `backup_database` refuses to snapshot a corrupt
source, so throughout all of that the backup ring had stopped producing new
restore points. Every hour made the recovery *worse* rather than better,
silently.

A confirmed corrupt verdict now stops the writes that record a detection —
`insert_detection`, `insert_quarantine`, and the BirdWeather upload queue —
through one gate, `AppState::with_ingest_db`.

**Deliberately not a read-only connection.** Login sessions are rows in this
database. `PRAGMA query_only` would honour the policy to the letter and lock the
operator out of the admin UI that exists to tell them what is wrong, while
silencing the notification log that records the alerts about the corruption.
Settings, sessions, the audit log, the notification log and the maintenance-run
record that turns the health endpoint red all keep working. `/api/v2/health`
reports `"detection_writes": "halted"` and answers 503, and the existing
station-health condition for a failing integrity check already alerts — and,
after the change above, keeps saying so weekly.

Only a *confirmed* corrupt verdict halts. A check that could not be completed
is "no verdict", not a failure: a transient I/O error must not stop a working
station from recording a season. The latch is one-way, because a file does not
heal itself and a check that flapped would flap the station with it; recovery
is a restart, where the startup path restores or quarantines. The log line says
exactly that.

Eight gates. The one worth naming is structural rather than behavioural: a
fourth per-detection write added later through the ungated writer would produce
no failure, no warning and no alert — which is what a healthy station produces.
The gated set is therefore written down once, in the gate's own doc comment,
and a source scan reads it back and checks every name, in every production file
in the workspace. Put one call site back on the ungated writer and it names the
file and the function.

### Fixed — a fault was announced once and then never mentioned again

Alert storms are well prevented here — a three-poll debounce, one episode per
condition, a recovery notice, and a compile-time assertion that the debounce
constant stays above two. The opposite failure was not prevented at all:
**nothing re-notified an open episode**. The only thing that re-armed one was a
process restart. A microphone that went deaf in April produced one push, and by
August the operator had forgotten it, because the station had.

An open episode is now re-announced on a widening schedule — 24 h, then 72 h,
then one a week — carrying *"Still unresolved after N days"* and the condition's
**current** description rather than the one it opened with. A disk that was
91 % full in April is 99 % full in August, and the second number is the one
worth waking up for. Four pushes in the first fortnight, then one a week: often
enough that a four-month fault cannot be forgotten, rare enough to still be
read.

All three alert loops widen. The deadman gained a `StillBroken` transition —
its state machine previously answered `None` for a station that was still
silent, which is the same answer it gives for a station that is fine, so the
loop had no way to tell them apart. Station health keeps a clock per condition
key. Acoustic health's set of reported sources became a map of clocks.

One thing the audit finding did not reach, and the tests initially did not
either: a station that is **off** for a month comes back with several steps of
the schedule already behind it. A counter that advanced one step per poll would
replay all of them, one every five minutes, which is the alert storm the rest
of this subsystem exists to prevent. Every step a gap swallowed is skipped, so
the operator gets one reminder and the next falls a week later. The test that
was meant to catch this asserted only that one call returned one reminder —
true of every implementation, including the broken one — and was rewritten to
ask what the *next* poll does.

### Fixed — the "Test notifications" button tested a path the alerts do not use

Two defects in one button, and the second is why the first went unnoticed.

**It tested a path nothing else uses.** The handler built a fresh
`reqwest::Client` and `POST`ed `{apprise_url}/notify` itself. That is not how
an alert about the station is delivered: `announce::flush` locks the shared
`apprise::Client` and calls `send_operational_alert`, which walks the native
`ntfy://` / `discord://` / `slack://` routes delivered in-process, falls back
to the `apprise` CLI for a config file, and puts every destination through a
circuit breaker and a rate limiter first. None of that was under the button, so
a green "test notification sent" said nothing about whether the deadman alert
would leave the box — which is exactly what the alert-latching defect above
turned out to be.

**And it was disabled for the configuration most stations have.** The button
was enabled only when `apprise_url` — an Apprise API *server* — was set, so a
station configured with `NOTIFY_URLS` alone saw "Not configured" and a dead
button while its alerts worked fine.

The web layer now holds the *same* client the three alert loops hold, and the
button makes the identical call `flush` makes. It is live whenever any
destination resolved — native routes, an Apprise server, or a config file the
CLI would be run for — and the page lists what this station resolved rather
than what is typed into the settings form, which is a different question when a
value was saved after the last restart. The labels come from
`dispatch::label_for` and are credential-free by construction.

An operator now gets the notifier's own answer, which is the point: *"every
destination was skipped (1 with an open circuit, 0 rate-limited)"* is a
different problem from a delivery that was tried and failed, and the old test
could report neither.

Eleven gates. The one that matters is the discrimination: with the circuit
already open on the station's destination, the button must **report** that and
not force a send. A fix that read the notifier's routes and then sent them with
a client of its own would pass every other gate and fail that one — verified by
making `Gate::admit_priority` admit unconditionally and watching exactly that
gate go red. The counterpart, a station that resolved nothing at all, passes
against both the old and the new code, which is what stops "enable it whenever
a route resolved" from becoming "enable it always".

Email and MQTT still have no test of any kind; that half is recorded as an open
item rather than quietly folded in.

### Fixed — a notification status the database refused to store, and the alerts nothing logged

Two defects, found together. The second was found by running the first one's
gates.

**The notification log contained every robin and no deadman.** The four
detection channels each recorded an outcome; the three alerting loops recorded
nothing, so an operator who suspected they had missed a station alert had no
record to consult. Because the 2.2 work had already made `announce::flush` the
one delivery path for all three loops, this is a single writer rather than
three. `channel = "alert"`, so `channel = 'alert'` selects the station's own
history and `channel = 'apprise'` the bird traffic.

An undelivered alert is logged as `Queued`, not `Failed` — that variant's own
doc comment describes this exact situation, and the distinction it draws
matters: *an operator looking at a wall of red needs to know which one they are
looking at before they go and climb a hill*. One row per episode, not one per
retry: the retry runs at every five-minute poll, so a notifier down for a day
would write about 288 rows for one alert and bury the log it exists to be.
Species columns stay empty, because an alert about a failing backup is not
about a bird and a placeholder would make the Notification Center's species
filter answer wrongly rather than not at all.

**And then the gate for that would not go green.** `NotifStatus::Queued` could
not be stored at all. Migration 4 created `notification_log.status` with
`CHECK(status IN ('sent','failed','skipped'))`. `Queued` was added to the enum
afterwards, documented at length, and written **in production** by
`daemon/processor.rs`'s store-and-forward path — where every insert was rejected
by the CHECK and the error discarded at `debug!`, which the default filter
drops. A field station on flaky LTE produced exactly the bursts that doc
comment describes, and the Notification Center showed none of them. The careful
distinction between "not there yet" and "lost" was between one status that
existed and one that never had.

Migration 41 rebuilds the table, because SQLite cannot alter a CHECK constraint
in place — the same reason migrations 36 and 40 rebuilt `quarantine`. Unlike
those, the insert here is a plain `INSERT` rather than `INSERT OR IGNORE`, so
the violation *was* returned as an error; it was the caller that threw it away.
Both halves are fixed, and `ALL_NOTIF_STATUSES` now exists so
`every_notification_status_is_accepted_by_the_schema` can enumerate the set
rather than restate it — a sixth status without a migration fails in CI instead
of on a station. Its counterpart checks the CHECK was *widened* and not deleted.

Eleven gates, six mutations killed. Two of them are worth naming: a loop
sending inline again — the pre-2.2 latch-on-attempt shape, whose sends reach no
log — is caught by a source scanner rather than by behaviour, because the whole
point of one writer is that the loops route through it; and the un-widened
CHECK, which is not a hypothetical mutation but the shipped schema, reported as
*"the schema rejects the `queued` status this code writes: CHECK constraint
failed: status IN ('sent','failed','skipped')"*.

### Fixed — the audit log was never written

Table, store, admin page and 180-day pruner all existed. `AuditLog::record`
had **zero production callers** — every call site was inside its own
`#[cfg(test)]` block. `/admin/audit` was permanently empty, which on a shared
station does not read as "the log is broken"; it reads as "nothing happened".

The repo had already caught half of this once. The *pruner* was wired after
being found to have no caller, and a retention constant was written for it: six
months of retention on rows nobody wrote.

Twenty-four actions are now recorded, across every mutating surface — sign-in
and sign-out, account and password changes, session revocations, settings
saves, species include/exclude lists and per-species thresholds, audio sources,
alert rules, clearing detections or recordings, restoring a database, running a
backup, restarting, and applying an update. Species filters and audio sources
were not in the finding's list and belong there: they decide whether a gap in a
season is a real absence or a filter somebody added in April.

**Values are never recorded.** A settings save lists the names of the keys that
changed and nothing else. The finding proposed redacting values "through the
existing secret list"; `rtsp_url` is why that would not have been enough — an
RTSP URL routinely carries `user:pass@` in its authority while its key name says
nothing about a secret, which is precisely the trap `redact_url_credentials`
exists for. Names only, and the question the log exists for — *who changed the
recording schedule on the 3rd?* — is still answered.

A save that changed nothing writes no row. The settings page posts every field
on every submission, so recording each one would turn the audit log into a click
counter and bury the save that moved the schedule.

Destructive actions are recorded *before* the work rather than after: clearing
detections, restoring a database, restarting, applying an update. If the process
does not survive the operation there is no "after" to record from, and a station
whose history vanished with nothing in the log is indistinguishable from one
that was never used.

A failed sign-in records the submitted username and no actor. "Someone tried to
sign in as `admin` sixty times last night" is the thing worth knowing, a
username that does not exist is as interesting as one that does, and there being
no actor is the whole reason `audit_log.user_id` is nullable.

Fifteen gates, six mutations killed. `audit()` writing nothing — the shipped
state — fails six of them and correctly leaves the two "must record nothing"
gates green. One gate is a source scanner: it reads every action literal out of
the web crate and compares it against a documented list, so an action name
with `threshold` misspelled fails the build instead of shipping a row that
renders fine and is invisible to the prefix filter meant to catch it. That is the same
lesson the station-health `CHECKS` table records — a set expressed only as
scattered call sites cannot be checked, so it is written down once.

### Fixed — `/api/v2/system/disk` returned 503 "critical" on a disk it called 76 % full

`used_percent()` carries a doc comment explaining, at length, that fullness is
`used / (used + available)` and *not* `used / total`, because the two diverge
whenever part of the device is invisible to this user. Nine lines below it,
`is_critical` read `available_bytes < total_bytes / 20` and `is_low` read
`available_bytes < total_bytes / 10`.

Reproduced on the filesystem this was written on. `df -Pk /` reported 264 212 084
blocks total, 29 896 308 used, 8 952 216 available — 77 % used, with 85 % of the
device unreachable behind a quota. `used_percent()` agreed at 76.6 %.
`is_critical()` returned **true**, because 8.5 GiB is less than a twentieth of a
252 GiB device. So the endpoint served HTTP 503 with a body saying 76.6 %, and
a monitor pointed at it pages the operator on a healthy station — which is how
a channel gets muted before the real alert arrives. Every ext4 default has a
5 % root reserve, so this is not an exotic shape; it is every Pi image.

Both predicates now read `used_percent()`, and the thresholds are named:
`DiskUsage::CRITICAL_PERCENT` (95) and `LOW_PERCENT` (90). Critical is the
reading at which the purger starts deleting recordings — the same number as
`DiskManagerConfig`'s default, now asserted rather than duplicated — and low is
what the station-health alert and the Station Health badge both use, so the
page and the operator's inbox change at one reading instead of agreeing by
coincidence. The station-health constant's own doc comment claimed it "matches
the capture layer's own default purge threshold"; that threshold is 95 and the
constant was 90. The gap is right and the sentence was wrong: the warning has
to arrive while there is still time to fit a bigger card.

**An existing test asserted the defect.** `disk_usage_percent_with_reserved_space`
built exactly this fixture, checked `used_percent()` was 80.0, and then asserted
`is_critical()` — with the justification `"7/252 available is critical"`. That
is what made `available < total / 20` look like a deliberate choice: a reader
finding it would see a passing test beside it. The fixture is kept and the
assertion inverted, so the history shows which way it flipped.

Six gates, four mutations killed. The instructive one is the fourth: making
`used_percent()` divide by `total` **as well** leaves the swept property gate
green — two surfaces agreeing on the same wrong number is still agreement — and
is caught only by the reproduction, which pins the answer to what `df` says.

### Added — a station now notices its own clock drifting

Runtime clock correctness was never re-checked. `--doctor`'s clock checks run
once, from `ExecStartPre`; at runtime capture tests only a plausibility floor
and trusts anything above it absolutely. A Pi whose NTP has been unreachable
for months keeps recording, keeps detecting, and keeps every gauge green while
filing an entire season under the wrong hours — a loss that shows up only when
someone tries to compare that season against another station's.

Station health gained a sixth condition and `birdnet_clock_synced` a gauge,
from two signals that fail differently. The plausibility floor catches a clock
that was never set — a Pi with no RTC that booted to 1970 — and says so in
those words, because "not synchronised" would send the operator to
`timedatectl status` to be told what they already know when the actual fault is
the uplink. NTP state catches the slow one the floor cannot see.

The probe has three outcomes rather than two, and that is the part worth
knowing: **"cannot tell" is not "broken"**. Every Docker deployment lands
there — `timedatectl` is installed but there is no bus, so it exits non-zero
with *"System has not been booted with systemd as init system"* — and a
container's clock belongs to its host. Those stations produce no condition and
no metric series at all, rather than a `0` that would page an operator about
something they cannot fix from inside the container. The repo's own
`container_can_run_what_the_daemon_spawns` gate caught the new subprocess
immediately and required it to be classified, which is the entry that now
records this reasoning.

`/run/systemd/timesync/synchronized` is a fallback rather than a peer signal.
It is created when `systemd-timesyncd` first synchronises and is *not* removed
if synchronisation is later lost, so it answers "synced at some point since
boot" — precisely the question this check must not ask, given the failure it
exists for. `timedatectl show -p NTPSynchronized --value` reports the state
now, so it is the authority and the file is consulted only when nothing else
can answer.

**A gap this found in its own gates.** The first mutation applied — deleting
`check_clock` from `evaluate`, which is the shipped state — killed *nothing*.
All 31 tests passed. Every gate exercised the policy function and none checked
that anything called it, so a check dropped in a refactor would have been
invisible: it produces no failure, no warning, and no condition, which is
exactly what a healthy station produces. `evaluate` now runs a named `CHECKS`
table, and a gate reads it against the six conditions the module doc promises.
The same mutation now fails that gate alone.

Not covered, and stated rather than implied: timezone drift. `doctor/clock.rs`
still checks that only at `ExecStartPre`.

### Fixed — the live log viewer streamed a channel nothing published to

`routes/admin/logs.rs` opens by saying its lines "are captured by a custom
`tracing` layer that broadcasts to an unbounded channel". No such layer existed
anywhere in the workspace, and the channel is bounded at 512. `AppState` held a
`LogBroadcaster` with no writer, so `GET /admin/system/logs` replayed an empty
backlog and then emitted keep-alives for ever — on every station, since the
feature was written. In Docker, where the operator has no `journalctl`, that
page is the whole story.

The audit that found this also said the three `LogBroadcaster::new()` calls in
`state.rs` were "three distinct channels anyway". They are not: they are three
*alternative* constructors — `AppState::new`, `new_with_analytics`,
`from_connection` — and one run builds one `AppState`. There was a single
channel and nothing wrote to it. The count was never the defect, and no
deduplication was needed.

`LogCapture` implements `tracing_subscriber::Layer` and is installed as a third
`.with(...)` in `main`. It lives in the binary rather than in `birdnet-web`
because which subscriber layers get installed is an application decision — the
same one that owns the tokio runtime — and this keeps `tracing-subscriber` out
of the web crate. The broadcaster is built before the subscriber and handed to
the state through `AppState::with_log_broadcaster`, because the layer must
exist at `init()` time and the state does not exist yet.

Structured fields travel with the message. `tracing::warn!(error = %e, "publish
failed")` carries its whole diagnosis in `error`, and a viewer showing only
"publish failed" would be worse than the journal it stands in for.

**And a log that survives the reboot.** A default Raspberry Pi OS has no
`/var/log/journal`, so the journal is volatile: every watchdog bounce, power
cut and update erases the evidence of what caused it, which is precisely the
event an operator is trying to explain. `errors.jsonl` sits beside the
database, takes ERROR and WARN only, is one JSON object per line, is capped at
1 MB — a station failing in a loop must not fill the card the recordings live
on — and is now a `--support-bundle` member. A missing file is reported in the
bundle rather than staged empty, because "this station has never logged a
warning" and "the bundle could not find the log" are different answers and only
one is good news.

URL credentials are stripped in the layer rather than at each call site.
`errors.jsonl` travels in the support bundle, and `rtsp://user:pass@host/` in a
warning is the shape that ends up in a public forum thread posted by an
operator who was told the bundle was redacted.

Nothing in the layer may log: a `tracing::warn!` raised while handling an event
re-enters `on_event` and deadlocks on the file mutex. Write failures are
counted and swallowed, deliberately.

Twelve gates. Six mutations applied and watched go red, the important one being
`with_log_broadcaster` made a no-op — the shipped arrangement — which fails the
wiring gate alone while every layer gate stays green. A layer writing to one
broadcaster while the state holds another passes any test of either half and
still shows an operator nothing.

### Fixed — the "Station Status" entity Home Assistant showed had nothing behind it

Home Assistant discovery has always registered a `binary_sensor` with
`device_class: connectivity` on `{prefix}/status`. Nothing ever published to
that topic — `publish_status()` and `publish_daily_stats()` had zero call sites
for the life of the project — so two of the four entities were permanently
*unknown*, and the one automation an unattended station exists to support,
*tell me when it stops answering*, could not be built.

It could not be fixed where it looked like it should be. A last will is
discarded by the broker when the client sends DISCONNECT (MQTT 3.1.1 §3.14),
and DISCONNECT is how every one of this station's publishes ends — the
publisher opens a TCP connection per message. Setting the will flags on each
of those CONNECT packets would have produced a will that fires on a
mid-publish network blip
and never on the power cut it exists for: worse than none, because it looks
like it works.

So presence gets its own connection. `PresenceSession` holds one otherwise-idle
socket open with a 30-second keepalive, carrying a will of `offline` (retained,
`QoS` 1) on `{prefix}/status`. The station publishes `online` retained when it
connects and `offline` retained on a clean stop; the broker publishes the will
about 45 seconds after a station stops answering for any other reason. The two
connections use different client identifiers — a broker must disconnect an
existing session when a second claims its identifier (§3.1.3.1), so sharing one
would have had every detection publish kick the presence session off and the
station flap `online`/`offline` for as long as birds were singing.

`birdnet_mqtt_connected` is the gauge for the case the topic structurally
cannot cover: the broker itself being down. A station cannot report on a broker
that is not there, so that signal has to travel by another road.

Three more defects surfaced in the same module, all found by reading it rather
than by the finding that sent us there:

- **Discovery configs were published unretained.** Home Assistant builds its
  entity list from what the broker replays when HA starts, so all four entities
  vanished every time *Home Assistant* restarted, until the station was
  restarted too. They are now always retained, whatever `MQTT_RETAIN` is set to
  — that setting is a real preference for the detection stream and not one for
  discovery.
- **`MqttConfig::qos` had no reader anywhere in the workspace**, while
  `publisher.rs`'s own module doc said `QoS` 1 "is sent at `QoS` 0 after logging
  a warning". There was no warning and no branch. A station configured for
  `QoS` 1 got `QoS` 0, where "the broker never received it" and "the broker has
  it" are the same return value. `QoS` 1 now sends a packet identifier and waits
  for the PUBACK, which is cheap here because the connection carries one message
  and there is no in-flight window to track.
- **`publish_status` is gone rather than wired.** The presence session owns
  `{prefix}/status` now, and a stateless publisher beside it would be a second
  writer to one topic with a different retain flag — the one arrangement in
  which Home Assistant shows a live station as offline.

Nine gates, against a broker stub that decodes CONNECT and PUBLISH rather than
matching bytes, because the failure that matters here is semantic: §3.1.3 lays
the CONNECT payload out positionally, so a will written after the username is a
perfectly well-formed packet that publishes the station's password to whatever
the broker reads next. Eight mutations were applied to the shipped code and
watched go red, including that one.

### Fixed — an alert nobody received counted as one that had been sent

Every alert loop in the station latched its episode on the *attempt*, and the
attempt could not fail. `Client::send_notification_with_image` ended with

```rust
return match (delivered, first_error) {
    (0, Some(e)) => Err(e),
    _ => Ok(()),
};
```

`(0, None)` is the fully-skipped case — every destination refused by the rate
limiter or the circuit breaker, no send attempted, no error to report. It
returned `Ok(())`. Nothing had left the box.

Both guards live on the same `dispatch::Gate` a detection notification uses, and
its token bucket is sized for detections. So the sequence that matters is
ordinary: a dawn chorus drains the minute's budget, the detection deadman
crosses its 24-hour threshold at 06:00, `notify()` gets `Ok(())` from a send
that never happened, the loop sets `alerted = true`, and `transition()` returns
`Transition::None` for the rest of the silence. The station had stopped
detecting, the one mechanism built to say so had said it, and the operator was
never told. The same held for every station-health condition and every stream
fault. The skip itself was logged at `debug`, and the default filter puts
`birdnet_integrations` at `info`, so there was no evidence either.

Four changes, and they are separable:

**The send reports what happened.** `AppriseError::AllDestinationsSkipped
{ circuit_open, rate_limited }` for a notification every destination refused,
and `AppriseError::NoDestinations` for a client with nowhere to send — reachable
today through `--notify-urls` whose every scheme lacks a native sender, where a
startup warning was the only sign and every send since answered `Ok(())`.

**An alert about the station outranks the bird traffic.**
`Gate::admit_priority` bypasses the rate limit for `Priority::Operational`. It
still honours the breaker, deliberately: a destination that has failed three
times running will not accept this one either, the breaker already admits one
probe per open period, and a caller that retries rides that schedule and lands
as soon as the destination comes back. The weekly report moved to the same path
— it is one message a week, and losing it to a blackbird stamped `last_sent_date`
and skipped that week.

**The episode is latched on delivery, without re-logging.** The loud journal
line still happens once, where the state machine decides something changed; the
push is parked in `src/integrations/announce.rs` and retried at every poll until
it lands. Keying that outbox by episode makes supersession free — a recovery
queued while its onset is still stuck replaces it, so nobody is handed a fault
that has already cleared — and an alert delivered more than ten minutes late
carries how long it waited, because everything in these bodies is present tense.

**What still cannot be delivered is counted.**
`birdnet_notifications_dropped_total{reason}` over `circuit_open`,
`rate_limited`, `send_failed` and `no_destination`. A detection notification the
limiter refuses is counted and writes no `notification_log` row: during a dawn
chorus that would be thousands, for the same reason there is deliberately no
`skipped` row beside it. The per-send circuit-open line stays at `debug` for
detections — four thousand a day on a station with a retired webhook — and the
`warn` moved to the transition, where `Breaker::on_failure` now reports the
period it just opened for.

Twenty-five gates, each observed failing first. Six mutations were applied to
the shipped code and watched go red: `admit_priority` delegating to `admit`
(the alert is dropped by the detection limit), `admit_priority` returning `Send`
unconditionally (a dead destination gets hammered), `on_failure` never and
always reporting a transition, deleting the `(0, None)` arm (four tests fail,
one printing *"a notification that reached nobody was reported as sent"*),
`Outbox::settle` dropping the alert whatever happened, `queue` refusing to
replace, and the late-delivery note suppressed.

### Fixed — a backup that failed every week never alerted, and a corrupt database pushed nothing

`src/integrations/station_health.rs` opens by naming the five conditions it
exists for, among them *"a failing integrity check or a backup that has not
completed in weeks — the two things standing between a corrupt database and a
lost season"*. It caught neither, and implemented four of the five.

**The backup.** `mark_ran` was called unconditionally after
`run_backup_and_vacuum`, and the health check read `last_run_unix` while
ignoring the `ok` column. A backup that failed every week for a year therefore
refreshed its timestamp every week and never once looked stale. The only thing
that check could ever detect was the maintenance loop having *stopped*.

**The integrity check.** It records its verdict correctly, and that verdict
correctly reddens a badge and 503s an endpoint — but the staleness-only rule
meant a `Some(false)`, which means the database is corrupting, sent no
notification at all.

**The offsite upload.** Invisible everywhere: no counter, no `maintenance_runs`
row, no health field, no alert. A station whose only off-card copy had failed
for twelve months looked identical to one whose uploads all succeeded.

**The fifth condition.** A quarantined database reached nothing.
`doctor/analytics.rs` matched `.duckdb.corrupt.` only, and its test asserted a
quarantined `birds.db.corrupt.<ts>` was *not* counted, on the stated grounds
that it "belongs to the other check" — which did not exist. So the one
quarantine that means **the entire detection history is gone** was the one
nothing looked for.

Now: `run_backup_and_vacuum` returns a `BackupOutcome` with a verdict per
destination, recorded through `mark_ran_with`; the offsite upload gets its own
`JOB_OFFSITE_BACKUP` key, because "no recoverable snapshot at all" and "the only
copy is on the card the scheme exists to survive" are different news; a recorded
failure is a condition *immediately*, without waiting to go stale, under its own
episode key so a failure and a staleness cannot clear each other; and both
stores' quarantines are found and reported, with the title distinguishing a
rebuilt analytics store from a lost history.

Gates: six, three of them observed failing against the shipped behaviour —
ignoring the verdict, alerting on every recorded run (which would page a healthy
station weekly), and giving both quarantines the same title (which would tell an
operator their history was gone every time a DuckDB version bump rebuilt the
analytics store). The doctor's widened scan was observed failing at
`left: 1, right: 2`.

### Added — the station can now say it has stopped detecting

Two signals, because between them they separate the three states an outside
observer previously could not tell apart.

**`birdnet_files_analysed_total{source}`** counts audio files the pipeline
finished analysing. Nothing counted throughput before.
`birdnet_inference_duration_seconds` is observed once per *stored detection* —
its own `# HELP` says so — so on a station with a wrong label file, a wrong
sample rate, or a model swapped by a bad update, every latency series was flat
and empty, **identical** to a station where inference never started. The four
drop-reason labels did not separate them either: all of them live downstream of
a prediction the model actually made.

A 15-second segment length gives about 5 760 of these a day per source, so a
flat counter alongside `birdnet_audio_source_up == 1` means capture is writing
files nothing analyses, and a rising counter with no detections means the model
is answering nothing.

**`GET /api/v2/health?strict=1`** returns 503 when the detection daemon is not
running. The status code used to be the database verdict and nothing else, so
this endpoint answered `200 "healthy"` on a station whose own response body said
`"detection_daemon": "stopped"`. That is the endpoint the container
`HEALTHCHECK` polls and the one every off-the-shelf monitor gets pointed at.

The default stays 200, deliberately. Docker restarts an unhealthy container, and
a station whose daemon is down is exactly the one that must stay up to be
diagnosed — restarting it in a loop destroys the journal that says why. The
strict form is for the monitor that should wake a human, which is a different
consumer with a different correct answer. Both report `detection_daemon` and
`detection_silence_secs` in the body either way, and the response now echoes
which mode it answered in.

Gates: four. The two metric tests include the discrimination as an explicit
assertion — a station that analysed ten files and detected nothing must not
render identically to one that analysed none. The two health tests were observed
failing against the previous status logic (`left: 200, right: 503`) and against
a version that made every request strict (`left: 503, right: 200`), which is the
change that would have put field stations into a restart loop.

The `run.rs` call sites are not covered by a CI-runnable gate: reaching them
needs the 541 MB model, the same limit `tests/species_filter_e2e.rs` documents
for its own second layer. What keeps them honest is that both sit in the `Ok`
arm of `process_and_infer_filtered`, so "analysed" cannot drift to mean
"attempted". This is stated in the code rather than left to be discovered.

### Fixed — the installer deleted the working binary before writing the new one

`install_binary` ended with `install -m 0755 src dst`. That is not atomic and
does not fsync. Traced with `strace`:

```text
unlinkat(AT_FDCWD, "dst", 0)                            = 0
openat(AT_FDCWD, "src", O_RDONLY)                       = 3
openat(AT_FDCWD, "dst", O_WRONLY|O_CREAT|O_EXCL, 0600)  = 4
```

The working binary is unlinked **first**, then a fresh file is created at the
same path and filled. Writing ~100 MB to an SD card is a multi-second window,
and an upgrade is exactly when a solar or battery-backed box browns out.
Afterwards `ExecStartPre` and `ExecStart` both fail, `Restart=always` with
`StartLimitIntervalSec=0` retries every five minutes for ever, there is no web
UI left to say so, and the previous binary was deleted rather than kept.

That last part also made the *documented* recovery impossible.
`docs/book/field/deployment.md` tells operators to keep the previous binary at
`.prev` "so a one-line `mv` rollback is possible". Nothing in the product ever
created that file.

The swap now copies the outgoing binary to `.prev`, writes the new one to a
sibling temp path, `sync`s it, runs `--version` against it, and `mv`s it into
place — `rename(2)` within one filesystem is atomic, so a reader sees either the
whole old binary or the whole new one and never a hole. The smoke test catches a
wrong architecture, a truncated extraction and a missing shared library while
the working binary is still one `mv` away. The in-tree Rust updater already did
all of this; the installer, which is the path every real upgrade takes, did none
of it.

Gate: `installer/test/binary-swap-atomicity.sh`, driving the shipping function.
Against `install -m 0755`, seven of its ten assertions fail, including *"the
live binary was unlinked; a power cut here leaves no binary at all"* and *"the
working binary was replaced by one that cannot start"*.

### Fixed — a partial model download was never resumed and never verified

Every guard around the model asked `[ -f "${dest}" ]`, and a partial download is
a file. So a 541 MB fetch drops at 60 %; `fetch_verified_model` fails; the
failure path deliberately **keeps** the partial and prints *"Re-run this
installer to resume from where it stopped"*; the operator re-runs; and the
presence guard skips the fetch entirely. The installer then reports *"Model
already downloaded — skipping"* and *"Validation passed"*.

`install.sh repair` — the documented wizard for a broken install — said *"Model
present — skipping download"* and computed no checksum, so the one subcommand
named for fixing this could not fix it. And four downstream checks pass on a
200 MB truncation of a 541 MB file: `--doctor` accepts any model file over
**one megabyte**, `validate_install` takes the doctor's exit code, the daemon
logs a failure and carries on serving the web UI, and `/api/v2/health` answers
`200 "healthy"` because its status is SQLite's and nothing else.

The operator seals the box, drives it forty kilometres out, and gets a green
dashboard that never records a bird.

Every guard now verifies the pinned sha256 rather than asking whether a file
exists, so a partial is resumed — `fetch_verified_model` already passes
`curl -C -` — instead of being mistaken for a finished download. `repair` hands
the decision to `download_model` rather than keeping a second, weaker copy of
it. Presence is not verification; the cost is one checksum of the cached file
per install or repair run.

Gate: `installer/test/model-resume.sh`, driving the shipping `download_model`
with only the network stubbed. Against the presence-only guards it fails with
*"the truncated model was skipped — this is the defect"* for both the model and
the labels, while the counterpart — a verified model must **not** be
re-downloaded, or every re-run costs 541 MB — passes either way and is what
makes the fix a verification rather than an unconditional refetch.

Both new tests are registered in `installer/test/run-ci.sh`, whose accounting
rule fails the suite if a test file is neither run nor excluded with a reason.

### Fixed — detections recorded before the clock was set were filed under 1970, permanently

A Raspberry Pi has no battery-backed RTC. Before NTP lands it reads the epoch;
the capture tee stamps that reading into the segment filename, and a detection's
`Date` and `Time` are parsed straight back out of that filename. Nothing
checked. Every detection made before the clock was set was stored as
`1970-01-01` — where it stayed:

* `species_summary` files it under hour 00 for ever;
* `MIN(Date)` makes every species touched in that window "first seen 1970", so
  the first-of-the-year and first-of-the-season features report nonsense;
* the history calendar acquires a 56-year span;
* `detected_at_utc` of about zero sorts it before everything the station has
  ever heard;
* and clip retention later reclaims its audio for being older than any cutoff —
  so the evidence goes and the poisoned row stays.

On a station whose uplink is down, "before NTP lands" can be weeks.

The write path now refuses such a row before anything else: it is quarantined
with a new reason, `implausible_clock`, and counted as
`birdnet_detections_dropped_total{reason="implausible_clock"}`. Quarantined
rather than dropped, because something was genuinely heard and the operator
should be able to see that their station spent a fortnight recording without
knowing what day it was — and because `tests/clock_steps_backwards.rs` already
pins that a naive "drop implausible dates" filter is the wrong answer.

Migration 40 widens the quarantine `reason` CHECK for the fifth time. That is
not optional bookkeeping: `insert_quarantine` uses `INSERT OR IGNORE`, which
does not distinguish a CHECK violation from the `UNIQUE` collision it exists to
absorb, so without the migration every clock-quarantined detection would have
been swallowed silently and reported as success — exactly the defect migration
36 was written for. The gate that catches it,
`every_quarantine_reason_is_accepted_by_the_schema`, turned red the moment the
enum gained a variant and before a line of the migration existed, which is the
job it was added to do.

Gates, both observed failing:

* with the check disabled — the state this shipped in — a `1970-01-01`
  detection produced `detections = 1, quarantine = 0`;
* with the check replaced by `if true`, the counterpart failed with `a real
  date must still be filed`, because a gate that quarantines everything would
  satisfy the first test and stop the station recording at all.

Also corrects `metrics.rs`'s explanation of the drop-reason labels. It named
`quality` and `occurrence` and taught what a spike in each would mean; neither
is ever emitted in production — both appear only in that file's own tests — so
both readings it taught were unavailable. It now names the five reasons
production actually emits.

### Fixed — two clock floors 1 461 days apart, and retention that ran on an unset clock

`--doctor` and the capture supervisor each had a `CLOCK_SYNCED_FLOOR_SECS`. The
doctor's was `2020-01-01`; the supervisor's was `2024-01-01`; the doctor's
carried a comment saying it *"mirrors the capture supervisor's"*. It did not.
For any reading in those four years the diagnostic printed
`[ PASS ] System clock — set to a plausible current time` while the supervisor
treated the same reading as untrustworthy and disabled the recording schedule
and every quiet window. An operator reading the diagnostic was told the opposite
of what the station was doing.

Both sides had tests. Each tested its own constant, so neither could see the
gap. There is one constant now, `birdnet_core::civil::CLOCK_PLAUSIBLE_FLOOR_SECS`,
in the module that already owns the calendar arithmetic both of them use — and
a gate that sweeps 2018 to 2030 weekly and asserts the two answer identically at
every point, which is what the previous arrangement could not have had.

**And every date-based retention job now refuses to run on an implausible
clock.** Each one computes its cutoff from `date('now')`, which is fine when the
clock is right and catastrophic when it is not. A Raspberry Pi has no
battery-backed RTC: before NTP lands it reads the epoch, and on a station whose
uplink is down that may be for weeks. Clip retention and log retention are
skipped with a warning in that state; the species cap is not, because it is a
count rather than a date and is safe with any clock. Recording continues
throughout — the station waits for the clock rather than stopping.

This covers the clock that is too *early*. A clock far in the **future** — a GPS
week rollover upstream, a carrier NITZ date, a `date -s` typo — is the direction
a probe demonstrated reclaiming an entire clip library in one pass, and it is
**not** covered here, because catching it needs a reference the floor does not
have. That is stated in the code rather than implied, and carried in
`docs/UNATTENDED_DEPLOYMENT_AUDIT.md` as the remaining half of NT-4.

### Fixed — two more tables grew for the life of the station

`sound_levels::prune` and `prune_quarantine` had **no production caller at
all** — the same shape as `AuditLog::prune` before it was wired, and in
`prune_quarantine`'s case under a doc comment reading "This prevents the table
from growing unbounded on long-running stations", which was true of no station.
`sound_levels`' sibling `audio_levels::prune` *is* called, from the
acoustic-health loop, which is what makes this an oversight rather than a
decision: a station kept every ⅓-octave bucket it had ever measured — thirty
bands an hour per source, for the life of the deployment.

Both now run in the daily log-retention pass, at 400 days for the soundscape
buckets (matching `audio_levels`) and 90 days for **reviewed** quarantine rows.
Unreviewed rows are never pruned at any age: they are the operator's queue, and
deleting a decision nobody has made yet is the one thing that pass must not do.

Gates: the existing log-retention pair, extended. With the two new pruners
removed — the state this shipped in — both fail on the new tables; with the
quarantine pruner's `reviewed = 1` condition removed, the counterpart fails on
the surviving row, which is the discrimination rather than the alarm.

### Fixed — one wedged upload was the last thing the maintenance loop ever did

`offsite::s3::client()` set `connect_timeout(30 s)` and nothing else, under a
comment that said so deliberately: *"No overall request timeout: a station on a
rural uplink can legitimately spend an hour on one upload… A wedged connection
is caught by this instead."*

The first half of that reasoning is right and is kept. The last sentence was
false. `connect_timeout` bounds the connect and TLS handshake only; a socket
that *establishes* and then stalls part-way through — the ordinary 4G failure,
and the ordinary behaviour of a middlebox that has dropped the flow without
RST-ing — is not bounded by it at all. A probe against a server that sends
headers, one byte, and then holds confirmed it: still waiting past 45 seconds.

`run_offsite` is awaited **inline** in `src/maintenance.rs`'s single sequential
loop, so one wedged socket stopped the daily `PRAGMA integrity_check`, `VACUUM`,
the local backup, clip retention, the per-species cap and log retention — for
the life of the process, with the `warn!` sitting on an error path that was
never reached. Nothing logged it, because nothing failed. SFTP had the same
shape: `ConnectTimeout=30` and a `child.wait_with_output()` with no timeout of
its own.

Three bounds, at three levels, each of which alone would have been enough and
none of which is the same instrument:

* **S3** gains a 120-second `read_timeout`. A read timeout is the right
  instrument because it bounds *inactivity*: it resets on every successful
  read, so a slow-but-progressing transfer is untouched however long it takes.
  A total `timeout()` would have broken exactly the case the original comment
  set out to protect.
* **SFTP** gains `ServerAliveInterval=30` and `ServerAliveCountMax=6` —
  OpenSSH's own stall detector, three minutes of complete silence. This is the
  transport-level counterpart to the `BatchMode=yes` already there, which
  closes the other way this hung: a prompt nobody would ever answer.
* **The maintenance loop** wraps the whole job in a two-hour budget, because the
  failure it guards against is not "the upload was slow" but "this loop never
  ran again". A transport that finds a new way to hang must cost one weekly
  upload, not every remaining maintenance run.

`run_offsite`'s own doc comment already claimed that a station which cannot
reach its bucket "must still VACUUM, still record birds, and still keep its
local backups". That was an intention, not a behaviour. It is kept, with a note
saying which of the two it was.

Gates: four. The stall test drives the real constructor against a server that
completes the handshake and then holds the socket, with a two-second timeout
injected so it runs in seconds; against the previous `connect_timeout`-only
client and the real 20-second budget it failed with *"the offsite client
returned headers but then waited past 20s for a body that never came"*. Two
counterparts — a server that answers promptly, and one that dribbles a byte
every 400 ms for far longer than any single gap — both pass, which is what makes
this a stall detector rather than a shorter deadline. The fourth pins the
shipped constant, and the existing SFTP option test pins both keepalive options
in the one place an option can go missing.

### Fixed — a zero-length database passed the integrity check

SQLite opens a **zero-length file as a brand-new empty database**. That is by
design — it is how every database in this project gets created — and it means
`PRAGMA quick_check` answers `"ok"` for a `birds.db` that has been truncated to
nothing. `check_integrity` ran that pragma and nothing else.

So `check_and_recover` took its healthy branch, logged *"database healthy"*, and
returned `RecoveryAction::None`. `migrate()` then built a fresh schema into the
empty file, the station started recording into it, and five good backups sat
beside it until the ring rotated them out about 35 days later.

Truncation to zero is not exotic on the hardware this runs on: it is what a
power cut during an SD card's wear-levelling relocation produces, what a
filesystem repair leaves when an inode survives and its extents do not, and what
a partly-restored backup leaves behind.

`check_integrity` now requires the file to begin with SQLite's sixteen-byte
magic before it asks SQLite anything. A file that is empty, too short to hold
the header, or header-shaped-but-wrong is not a database, and `check_and_recover`
walks the backup ring for it as it already does for a database that fails
`quick_check`.

Gate: five tests. Three shapes of "this is not a database" — empty, eight bytes,
and right-length-wrong-magic — plus a recovery that must bring the history back,
plus the discrimination that an ordinary healthy database still passes and is
not restored over. Against the previous code four of the five fail, the first
reporting `quick_check said "ok"` and the recovery one reporting `database
integrity check passed`. The fifth passed before and after, which is what makes
it worth keeping: a `check_integrity` that returned `false` for everything would
satisfy the other four and quarantine every healthy station at its next boot.

### Fixed — the weekly backup never finished on a station that was recording

`backup_database` drove SQLite's online backup API with
`run_to_completion(100, 50 ms)`: a loop of 100-page steps with a 50 ms sleep
after each one. SQLite restarts an online backup **from page 0 whenever the
source is written by a connection other than the backup's own**, and the source
here is opened on its own read-only connection — so every detection the daemon
stores is such a write, and the restart lands on the next step.

A station recording a detection every twenty seconds therefore had a weekly
backup that never returned. Measured on a 209 MB database under that load: still
running after 300 seconds, eight restarts, reaching 77 % and dropping to 0 each
time.

The consequence is larger than a missing backup. `run_backup_and_vacuum` is
awaited **inline** in the single sequential maintenance loop, so the daily
`PRAGMA integrity_check`, `VACUUM`, clip retention, the per-species cap and log
retention all stopped with it, for the life of the process — with no error path
taken, and so nothing logged. The station kept recording birds, which is the
right priority, and quietly stopped taking the snapshots that make a corrupt
card recoverable. That turns "recoverable corruption" into "total data loss",
which is the exact chain `src/maintenance.rs`'s own module documentation was
written to prevent.

The copy is now a single `sqlite3_backup_step(-1)`: every remaining page inside
one step, holding one read transaction, so there is no next call for a write to
restart. In WAL mode that read transaction is a snapshot and does not block the
writer, so the station records straight through it. `Busy` and `Locked` are
retried — they mean the step did not begin, so nothing is lost — under a
ten-minute deadline, because a retry without a bound would reproduce the same
"never returns" failure in a new shape.

Gate: a 4 000-row database, a second connection inserting every 20 ms, and a
30-second budget, with the backup on its own thread so the old code **fails**
rather than hanging the suite. Against `run_to_completion` it timed out with
1 368 rows written meanwhile; the fixed version completes the same work in
0.65 s. The counterpart — the same fixture with no writer — passes either way,
and is kept, because it is the reason the writer is the discrimination rather
than decoration.

### Fixed — the dead-man only fired when a bird sang

`HEARTBEAT_URL` is the station's one *push-based* liveness signal: the only
thing that can tell an operator 40 km away that the box is gone, because when
the box is gone nothing on it can report anything and the alarm has to be the
*absence* of an expected ping. It had exactly one call site in the workspace,
inside the per-detection loop in `src/daemon/processor.rs`, after every early
`continue`. A quiet night sent nothing.

So the absence of a ping meant "the box is dead **or** no bird sang", and those
cannot be told apart — which is fatal for the one signal whose entire job is
that distinction. A grace period wide enough not to false-alarm on a December
night at 55° N (sixteen hours of darkness, longer through a week of storms) is
far too wide to notice a dead box; one tight enough to notice a dead box pages
the operator every winter night until they mute the channel — the same channel
that carries the detection deadman. `docs/book/field/deployment.md` recommended
15 minutes, which is the second of those.

The ping is now a five-minute timer, matching the deadman, station-health and
acoustic-health loops, and fires once immediately at startup so a station coming
back from a power cut clears its monitor within seconds. It is spawned whether
or not `--web-only` is set: "is this box still there" is a question a web-only
station has too. The heartbeat handle is no longer threaded through the
detection daemon at all.

Failures now use the same episode semantics as the other loops — one `warn!`
when pinging starts failing, one when it recovers, `debug!` in between — instead
of a `debug!` line per detection that nobody would ever read.

Three signals, three meanings, and the manual now says so: this one is *"the box
is there"*; `birdnet_detection_silence_seconds` and `DEADMAN_HOURS` are *"it has
stopped detecting"*; the station-health alerts are *"it is degrading"*.

Also: the ping URL is no longer logged in full. `https://hc-ping.com/<uuid>` is
a bearer credential — anyone holding it can ping the monitor, which is exactly
how you make a dead station look alive, and on Healthchecks.io it carries a
`/fail` sibling that can page the operator at will. It was logged at `INFO` on
every start and so reached `journal.log` inside every support bundle. Only
`scheme://host` is logged now.

Gates, each observed failing first: three loopback tests drive the real ping
loop with no detection pipeline present. Against a stub with no loop — the old
code's behaviour on a quiet station — all three fail; against a one-shot startup
ping, the "a ping arrives" test passes and the "it repeats" and "a failing
monitor does not stop the loop" tests fail, which is the discrimination that
matters. Four more cover the URL redactor; against a redactor returning its
input — the previous logging — three of them fail.

### Fixed — `/api/v2/metrics` was not a document a Prometheus parser accepts

`birdnet_detections_total` was emitted **twice in one response body**: as an
unlabelled gauge counting rows in the database, and — from the runtime half of
the exposition, appended a few lines later by a different module — as the
genuine per-species counter. One name, two `# HELP` lines, two `# TYPE` lines,
two meanings, one of them a gauge that falls when a row is deleted.

The Prometheus text format forbids that, and the two common parsers disagree
about how. `expfmt.TextParser` — `promtool check metrics`, Telegraf's
`inputs.prometheus`, the Python client, most collection agents — rejects the
**whole document** on the second `# HELP`, so a station monitored that way
exported nothing at all, `birdnet_detection_silence_seconds` included: the one
series that says the station has stopped detecting. A Prometheus server's own
scrape parser accepts both series and keeps whichever `# TYPE` it saw last, so
the bundled dashboard's `sum by (species)(rate(birdnet_detections_total[1m]))`
folded a decreasing gauge in under `species=""`, where every purge reads as a
counter reset and manufactures a spike — on the panel used to answer "is it
still detecting?".

The three gauges are renamed off the suffix the convention reserves for
counters:

| was | is |
|---|---|
| `birdnet_detections_total` (gauge) | `birdnet_detections_stored` |
| `birdnet_detections_rejected_total` | `birdnet_detections_rejected` |
| `birdnet_species_total` | `birdnet_species_distinct` |

`birdnet_detections_total` now names only the counter it was always meant to.
`docs/grafana-dashboard.json` is updated; an operator's own dashboards and alert
rules need the same edit, and `docs/book/reference/integrations.md` says so.

The gate parses the **composed** body — the bytes actually served, not either
half alone, which is where the defect lived — and holds three structural rules:
one `# TYPE` and one `# HELP` per name, every sample belonging to a declared
family, and `_total` only on counters. It found a third offender the audit had
not: `birdnet_species_total` was also a gauge wearing `_total`.

### Fixed — the species-occurrence filter was asked about week 0, all year

The `BirdNET` geomodel takes `(latitude, longitude, week)` and was trained on a
48-week year, so its input domain is `1..=48`. The daemon passed a literal `0`
at both of its call sites, each carrying the comment *"week will be computed by
caller"* — and `run.rs` **is** the caller. Nothing computed it. `sf_thresh`
defaults to `0.03`, so the filter is on by default: every station with
coordinates has been filtering its species list against a point outside the
model's domain, identically in June and December, for the life of the project.
Every `Week` value ever written to `detections` is `0`.

Nothing caught it because week 0 does not error — the model returns a different,
plausible-looking occurrence vector — and because the one end-to-end test over
that function passed a week of its own (`20`, which is not even the week of the
recording it stages: 19 May is week 19), so it exercised the parameter rather
than the daemon's use of it.

The week is now derived from the *recording's own date*, never from the clock at
analysis time: a backlog drained three days after a power cut is scored against
the season it was recorded in. `process_and_infer_filtered` no longer takes a
`week` argument at all, so there is no longer a position a constant can be
passed in — the compiler enforces what a test could only observe.

`birdnet_core::civil::birdnet_week` is the shared arithmetic, clamping days
29–31 into week 4 of their month. That clamp is not decoration: `birdnet-go`
records an un-clamped copy of the same formula returning week 49 for 29–31
December and feeding it to a live range filter.

Existing rows keep `Week = 0`. The value is a BirdNET-Pi compatibility column
that only one internal query reads, so it is not backfilled here; the
derivation from `Date` is available if that changes.

### Added — the ten gaps against BirdNET-Pi and birdnet-go

`docs/FEATURE_GAP_ANALYSIS.md` is a line-by-line comparison against
[Nachtzuster/BirdNET-Pi](https://github.com/Nachtzuster/BirdNET-Pi) (`88985a3`,
~19k lines) and [tphakala/birdnet-go](https://github.com/tphakala/birdnet-go)
(`1e74c82`, ~540k lines of Go across 51 `internal/` packages): 38 findings, 8 of
them recorded as **declined** with the reason, plus what this project has that
neither of them does. This release closes the ten it ranked first. Every gate
below was watched failing against the code it guards before it was committed.

- **Sound-level monitoring.** A real ISO 266 third-octave spectrum — 1/3-octave
  bands from 20 Hz to 20 kHz, IEC 61672 A-weighting, and both broadband and
  per-band minimum, maximum and *energy* mean over each interval. The energy
  mean matters: averaging decibels instead of power under-reports a two-second
  silence followed by a second of full scale by 43 dB, so the arithmetic is
  pinned by a test that drives the meter with exactly that signal.

  The band filter is a three-section cascade, not one biquad. One biquad gives
  22.5 dB of rejection two bands out where the standard wants far more, and a
  1 kHz tone showed up only 12.6 dB down in the 630 Hz and 1600 Hz bands — a
  spectrum that would have looked plausible and been wrong. The `alpha` term is
  pre-warped with the `sinh` bandwidth-in-octaves form, because without it the
  10 kHz band's lower edge sat at −4.35 dB instead of −3.01.

  A-weighting is evaluated at the *exact* ISO centre (`1000·10^(n/10)`), not the
  rounded label. At the labels it deviates from IEC 61672 table 3 by up to
  0.157 dB; at the exact centres, 0.050 dB. Both halves are asserted.

- **Dynamic per-species confidence thresholds.** A species the station has
  confirmed at high confidence becomes easier to hear for a while: three levels,
  multipliers 0.75 / 0.50 / 0.25, a 15-minute learning cooldown and a floor at
  the model's own threshold. Ported from birdnet-go's `dynamicthreshold`, with
  its expiry semantics.

- **Species tracking — first of the year, first of the season, back after a
  winter away.** Hemisphere-aware seasons (±10° for the equatorial band), and a
  status per species carrying whether it is new to the station, new this year,
  new this season, or returning, with the days since it was last heard.

- **Pre-capture that spans segment boundaries.** Every clip was silently cut at
  the edge of the 15-second capture segment it landed in: a call two seconds
  into a segment lost its lead-in, and one near the end lost its tail. Clips
  are now assembled across neighbouring segments when they abut within 0.25 s
  and match in sample rate. The stream directory keeps ~40 segments, so this is
  live on a real station rather than theoretical.

- **A per-source parametric equaliser.** `pipeline_high_pass` and
  `pipeline_dc_removal` are two fixed filters — 120 Hz and 5 Hz. That is a
  compromise chosen for a garden and wrong in a different direction at most
  sites: a station beside a motorway wants a steeper cut, and a station with
  mains hum wants a *notch*, which no high-pass gives without removing
  everything below it.

  Each source now takes a chain of RBJ-cookbook stages —
  `highpass:120; notch:50:20; peaking:3500:1:4` — rendered from one
  specification for **both** capture backends: biquads in-process for a teed
  microphone, ffmpeg filter fragments for RTSP. The admin editor draws the
  response curve as you type, computed from the same coefficients that will
  filter the audio, and refuses a chain the source's sample rate cannot carry
  rather than accepting it and falling back silently at the next restart.

  Empty is the default and means exactly what the station did before.

- **Serving from under a reverse-proxy path.** `BIRDNET_BASE_PATH=/birdnet`
  puts the whole station under a prefix, for the common home setup of one
  hostname and several services. Home Assistant ingress works this way too.

  Nesting the router fixes incoming requests and nothing else — 234 literal
  `href`/`src`/`hx-*` attributes across 47 Rust files, 88 more in the templates,
  every `Location` header, the session cookie's `Path`, and three WebSocket URLs
  built in the browser all pointed outside the application. Those are handled in
  the pass that already buffers every HTML body to stamp CSP nonces, so the
  rewrite is free and covers markup written after this change as well as before
  it. The cookie keeps a trailing slash because RFC 6265's path-match is a
  prefix rule: `Path=/birdnet` also matches `/birdnetsomethingelse`.

- **A Flickr species-image provider.** The `ImageProvider` trait has had exactly
  one implementor since it was written, and its own documentation said it
  existed so Wikipedia could be joined by Flickr. Wikipedia has no photograph at
  all for a long tail of species and, for many others, a museum skin, an egg or
  a range map.

  Choosing Flickr gives a *chain*, not a replacement: Flickr first, Wikipedia
  behind it, so the setting can only add coverage. Only `NotFound` falls
  through — an API error stops the chain and is reported, because a broken key
  papered over by the other provider stays broken for a year.
  `FLICKR_FILTER_EMAIL` narrows the search to one photographer's photostream,
  which is how an operator shows their own pictures of the birds their own
  station heard.

  Only commercially-licensed photographs are requested, and every one is shown
  with its photographer's name and a link to the licence terms; a photo Flickr
  returns with nobody named is skipped rather than shown uncredited.

- **Resolving the client's address instead of guessing at it.** A trusted-proxy
  list (CIDRs, bare addresses, and the reserved names `loopback`, `private`,
  `cloudflare`) with a right-to-left walk of `X-Forwarded-For` that stops at the
  first untrusted hop. A forged header from an untrusted peer is now ignored;
  from a trusted one it is honoured. Both halves are gated, because a test that
  only asserts the honouring half is a blanket alarm passing for a
  discriminator.

- **A pitch control on the live stream.** See *Fixed* below — the mechanism
  existed; nothing could reach it, and it was documented backwards.

### Fixed — two doc comments that were confidently wrong

Both were found by measuring rather than reading, which is the only reason they
were found at all: each had been true-looking prose for as long as it existed.

- **The two capture backends do not apply the same high-pass.** `AudioPipeline`
  said they did. ffmpeg's `highpass` defaults to two poles (12 dB/octave); the
  in-process tee's is one pole (6 dB/octave). From the identical `high_pass`
  flag a microphone therefore keeps far more low-frequency energy than an RTSP
  camera:

  | Hz | tee | ffmpeg |     | Hz | tee | ffmpeg |
  |---|---|---|---|---|---|---|
  | 20 | −15.68 dB | −31.13 dB | | 60 | −7.00 dB | −12.30 dB |
  | 30 | −12.31 dB | −24.10 dB | | 80 | −5.14 dB | −7.83 dB |
  | 50 | −8.31 dB | −15.34 dB | | 120 | −3.04 dB | −3.01 dB |

  They agree only at the corner. The divergence is **left as it stands** rather
  than quietly corrected — both filters have been in the field, and changing
  either changes what every existing station of that kind records. The table is
  now in the type's documentation *and* asserted to 0.05 dB, and setting an
  explicit equaliser chain is the opt-in fix: inside a chain both backends
  render from one specification and provably agree.

- **`HIGH_PASS_CUTOFF_HZ` claimed the model cannot hear below its corner.**
  "Nothing BirdNET classifies lives down there: the model's mel bank starts well
  above it." `MelConfig::default()` has `fmin: 0.0`, and the V2.4 path hands the
  model raw samples (`[1, 144_000]`) with no filtering of its own. Energy below
  120 Hz reaches the classifier on both model generations. The corner is a
  signal-to-noise judgement, not a free lunch — which is the reason a steeper
  one is now offered rather than imposed.

### Fixed — the frequency shift pointed the wrong way

Five doc comments, including the `--freq-shift-hz` CLI help an operator reads
before choosing a value, said a **positive** (upward) shift "makes calls
accessible to people with high-frequency hearing loss". That is backwards.
Presbycusis takes the top of the range first, so an 8 kHz warbler is restored by
moving it *down*. A listener following our documentation shifted the song
further out of their own hearing.

The upstream this was ported from was fetched and read rather than recalled.
`BirdNET-Pi`'s `install_config.sh` ships `FREQSHIFT_HI=6000` / `FREQSHIFT_LO=3000`
under the comment "useful for earing impaired people", and `livestream.sh` builds
`rubberband=pitch=${FREQSHIFT_LO}/${FREQSHIFT_HI}` — a ratio of 0.5, down one
octave. Its sox path ships `FREQSHIFT_PITCH=-1500`. Two independent settings,
both downward.

All five comments are corrected, `ACCESSIBILITY_SHIFT_HZ = -3000` names the
direction and carries that evidence, and a `const` assertion fails the **build**
if the sign is ever flipped back — a stronger guard than a test, which a
filtered `cargo test` can skip.

Two related things came out of the same re-check:

- **The live-stream shift was unreachable.** `/stream?freq_shift_hz=N` has
  always worked; nothing in the UI ever sent it, so the feature existed only for
  someone willing to hand-edit a URL. (The gap analysis had recorded this as
  "streams the raw tap unshifted", which was wrong about the mechanism and right
  about the outcome; the document now says so.) There is now a pitch control
  beside the Listen button, with downward presets and one upward option for bat
  calls. It is remembered **per browser**, not per station: hearing loss belongs
  to a person, and this station spawns one encoder per connection where upstream
  fed one broadcast to everyone, so two listeners can each have their own.
- **`freq_shift_hz` was an unbounded `i32` from an unauthenticated request.**
  `freq_shift_hz=2000000000` asked ffmpeg to resample from ~2 GHz down to
  44.1 kHz, and `MAX_CONCURRENT_STREAMS` allows four of those at once. Clamped
  to ±24 kHz, which is wider than any useful setting.

### Fixed — the support bundle carried an email address

A config value that is unambiguously an email address now has its local part
masked and its domain kept (`you@example.com` → `***@example.com`) — the domain
is the diagnostic half. `FLICKR_API_KEY` was already caught by the redactor's
`KEY` needle; that claim predated the setting existing, so it is now asserted
rather than trusted.

### Added — HTTPS, without a reverse proxy

Until now the server spoke plain HTTP and the documentation told you to put
Caddy or nginx in front. That is a correct answer and a bad default: a second
daemon and a second config file on a box whose whole point is that it is one
binary. Both projects this one is measured against ship TLS; now so does this
one. **Off by default** — nothing changes for an existing station until
`--tls-mode` is set.

- **`--tls-mode self-signed`.** Generates a small local CA and a server
  certificate it signs, under `--tls-dir` (default: a `tls` directory beside
  the database). HTTPS comes up on 8503; plain HTTP keeps answering on 8502
  unless you say otherwise. Import the CA file once — the startup log and
  `--doctor` both print its path — and the browser warning stops for good: the
  CA lives ten years, the certificate it signs 397 days and rotates a month
  early, so a rotation does not send you back to the trust store.

  Not one self-signed certificate, which is the obvious design and does not
  work. Observed against rustls-webpki before it was written the other way:
  with `CA:FALSE` a client that trusts the file rejects the handshake
  (`BadSignature`), and with `CA:TRUE` it rejects the same file for being a
  `CaUsedAsEndEntity`. Splitting the CA from the leaf satisfies both.

- **`--tls-mode manual`.** Serves `--tls-cert` and `--tls-key`. Both are
  re-read when they change on disk, so `certbot renew` at 03:00 is picked up on
  the next handshake with no restart and no deploy hook — the common failure
  mode for anyone who has wired an ACME client to a long-lived server.

- **`--tls-listen` and `--tls-redirect`.** HTTPS defaults to `--listen`'s host
  on port 8503. Point it at `--listen` to serve only HTTPS on the one port, or
  set `--tls-redirect` to have the plain port answer `308` to the HTTPS origin
  (`308`, not `301`, so a POSTed settings form is not silently downgraded to a
  GET). Setting both is contradictory; the redirect is dropped with a warning
  rather than silently.

- **A `--doctor` check** that does exactly what startup does: parses the
  configured material, verifies the key matches the certificate, and names the
  CA to import. A mistyped `--tls-cert` is a `[ FAIL ]` in the diagnostic
  rather than a service that restart-loops after you have gone back inside.

  It also reports plain HTTP on a routable address as a `[ WARN ]` — and on a
  loopback bind as a `[ PASS ]`, because a station behind a proxy is a good
  deployment and nagging every operator would train them to ignore the report.

- **No ACME client.** Deliberate: it needs a reachable name, an open port 80 or
  a DNS credential, and an account key to look after, and a station on a home
  LAN has none of those. `manual` mode plus the reload above is the supported
  path for anyone who does.

**Dependency cost, counted rather than asserted.** Five new direct edges:
`tokio-rustls`, `hyper` and `hyper-util` were already resolved in the graph
(via reqwest/lettre and axum) and `rustls-pki-types` arrives under `rustls`,
so those four are edges only. `rcgen` is the one new crate anybody chose.
Diffing `Cargo.lock` against `main` and intersecting with `cargo tree -e
normal --all-features` puts the real figure at **nine newly compiled
crates** — `rcgen`, `pem`, `yasna`, `time` (+ `time-core`, `deranged`,
`num-conv`, `powerfmt`) and `futures-macro` — with a further ten appearing in
the lockfile but never built (`x509-parser` and its ASN.1 stack, plus
`time-macros` and `wasm-streams`). `rcgen` uses the `ring` backend already in
the tree rather than `aws-lc-rs`, for the same cross-compilation reason
`rustls` does.

PEM parsing is `rustls_pki_types::pem`, not `rustls-pemfile`. The latter was
archived in August 2025 (RUSTSEC-2025-0134) and its final release is a thin
wrapper around the same parser, so taking it would have bought an advisory
and a compiled crate for nothing. One consequence is visible to operators:
the `pki-types` parser reports an empty key file and a corrupt one
identically, so `--tls-mode manual` tells them apart itself and still says
which it was.

`time` arriving in *every* build configuration falsified a premise
`birdnet-db`'s `clock_premise` test had been guarding since it was written; the
design note in `crates/birdnet-db/src/clock.rs` and the test now record it, in
both directions.

### Added — a searchable detection log

The Today log answers "what happened today", and its four category shortcuts are
the questions a person asks while looking at one day. Everything else is a
*query*: every rejected record from May, this species below 40 %, whatever the
pond microphone heard between 22:00 and 04:00.

**`/search`** — reachable from the command palette and from Today, deliberately
not a seventh nav tab (the v3 spine has six homes and the long tail lives in the
palette by design). Nine criteria, combinable: free text (with the BirdNET-Pi
`NOT ` syntax), an exact species, a date range, an hour-of-day window, a
confidence range, an audio source, review verdict, lock state, and the four
category shortcuts — across six sort orders. The address bar carries the whole
search, so a useful one can be bookmarked or sent to somebody.

**Bulk actions.** Checkboxes and one action bar: confirm, reject, lock, unlock,
delete. Reviewing a season one row at a time is not review, it is attrition. The
endpoint is behind the admin gate, which is exactly the fix above: `action=delete`
over a selection is the most destructive request this application accepts.

Underneath is `DetectionFilter` in `birdnet-db`, a composable clause builder
replacing a three-armed `match` that could not have grown to nine dimensions.
Every placeholder is a positional `?` so the fragment that adds one is the
fragment that binds its value; a generated 6 561-combination matrix asserts
`placeholders == params`. `todays_detections` now runs on it too, and its
category shortcuts became date-relative in the process — the same predicate now
means the same thing over a range as it did on one day.

`review_verdict` joined the projected detection columns so the list can show and
filter review state in one query, and `detection_at` replaced two hand-written
copies of the fifteen-column mapper that were living in `birdnet-web`, outside
the drift gate that exists to prevent exactly that.

### Fixed — a checkbox group could not be submitted

`axum::Form` deserialises through `serde_urlencoded`, which has no
representation for a repeated key. A page of checkboxes posts
`selected=a&selected=b`, which is what the HTML form specification says a
checkbox group is, and the whole body was rejected:

```text
Failed to deserialize form body: selected: invalid type: string "…", expected a sequence
```

Found by posting a real form to a running server. Every unit test around the
handler constructed the struct directly and so never went near the deserialiser
— the bug was in the seam none of them crossed. The body is now parsed with
`form_urlencoded::parse` (the browser's own grammar, already in the tree through
`url`), and three gates go through the wire format.

### Fixed — five CSS custom properties were never defined

`app.css` used `var(--primary)` in four rules and `var(--card-bg)` in two, and
defined neither. An undefined custom property does not warn: the declaration is
invalid at computed-value time and the property silently keeps what it
inherited. The quarantine lede link, the active filter tab's colour *and*
underline, a detail-page link and two admin form backgrounds had all been
shipping in both themes looking approximately right.

Found by looking at a screenshot of a new button that rendered invisible —
white text on the white it had inherited. `tests/css_variables_are_defined.rs`
now fails the build on any `var()` with no definition, with a named allowlist
for the three properties something genuinely sets at runtime, each of which must
also carry a fallback.

### Fixed — the dashboard let anyone on the LAN change things

`public_routes()` carried **thirteen state-changing `POST` endpoints**: delete a
detection, relabel it, set or clear a review verdict, approve or reject or
delete a quarantined record, lock and unlock clips, and save the onboarding
wizard (which writes the station's coordinates, time zone and notification
policy). None of them required a login. The only obstacle was the same-origin
CSRF guard, which stops a hostile *page* and not a hostile *person* — anyone who
could load the dashboard could call all of them with `curl`.

The documented contract — *"viewing is open; only `/admin` needs a login"* — was
a statement about `/admin` that had never been checked against the rest of the
tree. It is now true: those routes moved to `pages::mutating_router()` and are
mounted behind the same middleware as `/admin`.

Nothing changes for a station with no admin password (a fresh Docker run, or an
operator who cleared it): the middleware bypasses entirely in that case, as it
always has. What changes is the station that *has* a password, where these
actions now need the session that `/admin` already needed. Reading stays open in
both cases — a new gate asserts that too, because the obvious over-correction is
to gate the whole of `pages::router()` and turn a viewable station into a login
wall.

`crates/birdnet-web/tests/public_router_is_read_only.rs` now fails the build if
any non-safe method appears in the public router, if a gated route stops being
mounted at all, or if a write is accepted without a session on a station that
has a password set. The three cover different regressions: the first two are
both satisfied by a fixture where the middleware never has to decide anything.

### Added — backups that survive the SD card

Until now every backup a station took lived beside the database it came from,
on the same card. That covers a corrupt page, a bad import, an interrupted
write — and none of the failures that actually end a station's records: the
card wears out, the enclosure floods, the Pi is stolen. The manual said so, in
bold, and told operators to pull a full backup from another machine on a
schedule they would have to build themselves.

`OFFSITE_BACKUP=s3` or `OFFSITE_BACKUP=sftp` now sends each weekly snapshot
somewhere else. **Off by default**, and the local snapshots and their 14-file
rotation are untouched — this only ever adds a copy.

- **Encrypted on the station, and not optionally.** A station's database is a
  log of what is around a house and when somebody is home. "Server-side
  encryption" on a bucket means the provider holds the key; an SFTP host means
  its administrator does. So the file is sealed before it leaves — argon2id
  over the operator's passphrase, then ChaCha20-Poly1305 — and there is no
  setting to turn that off. `OFFSITE_PASSPHRASE` has no command-line flag
  either: an argument is visible in `ps` to every user on the machine and is
  copied into the journal by systemd.

  Two details carry the weight. The 52-byte header is the AAD of every chunk,
  so an attacker with write access to the storage host cannot lower the argon2
  cost to 8 KiB and hand the file back still decrypting. And the nonces are a
  STREAM counter with a final-chunk flag rather than random, so removing the
  tail is *detected* — random per-chunk nonces authenticate every chunk and
  still let a backup restore cleanly, missing last March.

- **`--decrypt-backup <file> --out <path>`.** An encrypted backup with no
  working restore path is worse than no backup, because it looks like insurance
  for a year and then does not pay out. Refuses to overwrite an existing file
  (the likely `--out` on a station is the live `birds.db`) and leaves nothing
  behind when it fails.

- **S3-compatible, without the SDK.** AWS S3, Backblaze B2, Cloudflare R2,
  Wasabi, MinIO, Ceph RGW and Garage. `SigV4` written out rather than pulled
  in: for one PUT, one `GET ?list-type=2` and one DELETE the AWS SDK would
  bring a dependency tree larger than the rest of this binary, onto a board
  whose release build is already dominated by ONNX Runtime and DuckDB. It is
  checked against botocore rather than against a reading of the spec — see
  below.

- **Any SSH host**, through OpenSSH's own `sftp` in batch mode rather than an
  in-process SSH stack: a second place for key handling, host-key policy and
  cipher selection to be subtly wrong is not worth the subprocess it saves.
  Host key checking has no "off" — `yes` or `accept-new`, nothing else —
  because an SFTP backup with it disabled encrypts the upload to whoever
  answers. Uploads land as `<name>.part` and are renamed, so a power cut never
  leaves something a restore would reach for.

- **Retention by the station's own clock.** `OFFSITE_KEEP` (default 8, `0`
  keeps everything) orders by the timestamp in the filename, not the store's
  `LastModified`: a station uploading a backlog writes four backups in one
  minute, and pruning by upload time would keep an arbitrary four. Files this
  station did not write are never removed, so a shared bucket stays shared.

- **A `--doctor` check** that reports the destination, the retention, and — for
  SSH — whether the key exists, whether its mode is one OpenSSH will accept
  (`0644` is a silent refusal on the client's own stderr, once a week), and
  whether the host is known. It opens no connection: `--doctor` runs on every
  start, and a diagnostic that dials a remote host fails whenever the uplink is
  down.

  A half-configured destination is a `[ FAIL ]` listing *every* missing key, not
  a silent fall back to "off". That fallback is the shape of the defect that
  leaves an operator believing they have offsite copies for a year.

#### How this was checked

Every gate was observed failing against the code it was written for; the
interesting ones are the four that were observed *passing* when they should not
have been.

- The envelope's "a plain file is not an envelope" test passed a 25-byte string,
  so `read_exact` hit end-of-file and returned `NotAnEnvelope` before the magic
  was ever compared. It stayed green with the magic check deleted.
- The `SigV4` query-sort test used `b+c` against `b-c`, where the raw and
  encoded orders agree, so sorting before encoding passed. `-` against `:` is
  where they differ.
- The CLI truncation test cut 200 bytes off a single-chunk file, which breaks
  that chunk's own tag — the weaker property. It stayed green with the
  final-chunk flag deleted. The fixture is now two chunks and the cut removes a
  whole one.
- `container_can_run_what_the_daemon_spawns` never saw `sftp` at all: its
  scanner matched `Command::new("literal")` only, and this code names its binary
  in a constant. Removing `sftp` from the classification table left the file
  green. The scanner now resolves same-file `const NAME: &str` too, and has a
  test of its own.

`SigV4` is checked against vectors generated by **botocore**, the signer inside
the AWS CLI, by a script committed beside them. The chain is anchored: one
vector is AWS's own published "Example: GET Object", whose signature
`f0e8bdb8…6036bdb41` appears in the S3 documentation, and botocore reproduces it
byte for byte.

Both transports run end to end. `s3_loopback` drives a store that recomputes
the signature from what arrived on the wire and rejects a mismatch the way
MinIO would — catching the class the vector test cannot, where the request sent
is not the request signed. `sftp_loopback` stands up a real `sshd` on a
loopback port with generated keys. When that harness first ran, `sshd` never
started, and two of its four tests passed on "connection refused"; the harness
now asserts the port is listening before handing back a server.

### Changed — the manual, and a gate for the way it goes wrong

Documentation for everything above, and a sweep for the pages the work made
untrue. Two had said, in plain words, that a feature did not exist — and both
were right when they were written:

- `admin/backups.md`: *"The station has no built-in upload to S3, a NAS, or
  email."*
- `guides/recipes.md`: *"the built-in server is plain HTTP"*

Nothing caught either. There is nothing structural about a paragraph saying a
feature is absent — it reads exactly like one saying it is present — so
`the_manual_does_not_still_say_a_shipped_feature_is_missing` names the
sentences, scans the whole book and the README, and pairs each with the flag
that retired it. `retired_claims_name_something_that_actually_ships` checks the
pairing in the other direction, so an entry cannot outlive the feature and
quietly forbid a sentence that has become true again. Both were observed
failing by restoring the two real sentences, and by hiding one in an unrelated
page.

Also updated:

- **`admin/settings.md`** — the Analysis Overlap and Repeat Confirmation
  controls, which the settings page had grown without the manual noticing.
- **`reference/web-api.md`** — `/search`, with every query parameter it takes.
  The page had shipped with no reference entry at all. The parameter names were
  read off `SearchParams` rather than remembered: it is `conf_min`, not
  `min_conf`, and the sort tokens are `confidence`/`species`, which a first
  draft of the table got wrong.
- **`field/hardening.md`** and **`field/deployment.md`** — the runbooks now say
  how to get a copy off the device rather than only that you should, including
  the least-privilege credential shape for each destination.
- **`guides/faq.md`** — two new entries: what happens when the SD card dies,
  and why turning on repeat confirmation appeared to change nothing.
- **`README.md`** — HTTPS, offsite backups and search in the feature list and
  the BirdNET-Pi comparison; the test count corrected from a badly stale
  "1,690+", and pinned by a gate so it cannot drift again.

### Added — notifications without Apprise

- **Native senders for seven scheme families.** `discord://`, `slack://`,
  `tgram://`, `ntfy://`, `gotify://`, `pover://` and `json://` (with their TLS
  forms) are delivered in-process. The URL syntax is Apprise's, so anything an
  operator already has written down still works, but no Python, no `apprise`
  binary and no subprocess per detection. Set `BIRDNET_NOTIFY_URLS`. Apprise
  still handles every other scheme; an Apprise *config file* is all-or-nothing,
  and the CLI is never invoked when every URL in it is natively supported.
- **A circuit breaker and rate limit per destination.** A retired webhook that
  answers 404 forever is retried three times per detection, all day — and it is
  the retries, not the sends, that get an address rate-limited. The breaker
  opens after three consecutive failures for a period that doubles per trip
  (60 s → 30 min), admitting one probe each time it elapses.
  `BIRDNET_NOTIFY_RATE_PER_MINUTE` (default 12) bounds a *healthy* destination:
  Pushover allows ten thousand messages a month.
- **Authentication for alert-rule webhooks.** Bearer, Basic, or a named header,
  so a rule can target Home Assistant's `/api/webhook` and every hosted
  automation service rather than only endpoints that authenticate by URL alone.
- **Alert rules can be tested, exported and imported.** **Test** fires a rule
  now with an unmistakably synthetic detection and reports the HTTP status.
  Export redacts credentials by default (so it is safe to paste into a forum
  thread) with an opt-in form for backup. Import adds rather than replaces, and
  names any rule whose credential arrived redacted.
- **MQTT over TLS.** `BIRDNET_MQTT_TLS`, with the certificate always verified
  against the platform store plus `BIRDNET_MQTT_CA_FILE`. There is deliberately
  no way to skip verification. rustls was already in the tree, so this adds
  three dependency edges and no new compiled crate.

### Added — detection quality

All six are **off by default**: each changes how many rows a station records,
and doing that silently on upgrade would put a visible step in every chart.

- **A repeat-confirmation filter** (`BIRDNET_CONFIRMATION_LEVEL`, and a
  "Repeat Confirmation" select on the settings page). A real bird sings across
  more than one analysis window; a car door, a squeaking mount or a fragment of
  speech usually fires in exactly one. `lenient`, `moderate`, `balanced` and
  `strict` ask for 20%, 30%, 50% and 70% of the windows within six seconds to
  agree, rounded up.

  **It does nothing without `BIRDNET_OVERLAP`**, and the whole feature is built
  around saying so. With no overlap a six-second neighbourhood is two 3-second
  windows and 20% of two rounds to one, which every detection already meets.
  So: the option text on the settings page carries the overlap each level needs,
  `--doctor` reports which side of that line the station is on, and the daemon
  logs a warning at startup when the level it was given cannot reject anything.
  All three numbers are computed from the filter rather than written down, and a
  test pins the manual's table against it.

  Runs last of the three chunk filters, after privacy and noise. That ordering
  is load-bearing and gated: corroboration counts how many nearby windows
  carried a species, so running it first would credit a species with evidence
  from a chunk the noise filter was about to discard.
- **A noise-class filter** (`BIRDNET_NOISE_THRESHOLD`). A dog barking near the
  microphone is broadband, so the classifier scores whatever species it most
  resembles — and because the barking is regular, the phantom accumulates until
  it looks like a resident. Discards the chunk a watched class was heard in.
- **A duplicate-prediction interval** (`BIRDNET_DUPLICATE_INTERVAL_SECS`). A
  15-second recording is five chunks, so a bird singing throughout is recorded
  five times, and every count in the application is a row count.
- **A taxon-aware night filter** (`BIRDNET_NIGHT_FILTER`). Quarantines day birds
  heard in the small hours while exempting owls, nightjars, rails, bitterns and
  thick-knees by genus. Needs station coordinates; fails open. Stations
  recording nocturnal flight calls should leave it off.
- **Suggested per-species thresholds** (Species page). Works out the threshold
  that best separates the detections you confirmed from the ones you rejected,
  and shows what it would have cost and caught. Only ever suggests.
- **A suspect-species report** (Station → Data). Flags species by the *shape* of
  their detections — every review rejected, never detected confidently,
  confidence that never varies, many detections on very few days — with a
  one-click exclusion. Reports; never filters on its own.

### Added — operations

- **A stream-fault watchdog.** A muted channel or an unplugged input produces a
  valid, punctual stream of zeros: the supervisor reads `Connected`, and on a
  multi-source station the detection deadman never fires because the other
  microphones keep detecting. Digital silence, a stuck level and saturation are
  now detected and alerted on, once per episode with a recovery notice.
- **Sample-rate probing for autodetected microphones.** A 44.1 kHz-only
  interface handed `-r 48000` either failed to start forever or was silently
  plug-converted — the worse case, since capture works and every spectrogram is
  narrower than the station believes. Falls back to the previous behaviour
  whenever the probe learns nothing.

### Fixed

- **`INSERT OR IGNORE` was discarding rows silently.** It absorbs *every*
  constraint violation, not just the duplicate it is written for, and reports
  success either way. Adding a fourth quarantine reason without widening the
  column's `CHECK` meant every detection quarantined for it was dropped on the
  floor with `Ok(())` returned and no row and no error to find. Migration 36
  widens the constraint; every production write now names the conflict it
  actually means with `ON CONFLICT (...) DO NOTHING`, and a workspace guard
  fails if the idiom reappears.
- **A wrong recipe in the tuning guide.** `docs/book/guides/recipes.md` told
  operators to put `tgram://bottoken/chatid` in `BIRDNET_APPRISE_URL`, which is
  the base URL of an Apprise *server* — the station would have POSTed to
  `tgram:///notify`.
- **`BIRDNET_NOTIFY_RATE_PER_MINUTE` would have been inert.** Documented in
  `.env.example` while only the config-file key was read; there is now a flag
  bound to it, and the config key still works.

## [0.15.0] - 2026-08-26

A production-readiness pass against one question: *if this station is sealed
into an outdoor enclosure and left for a year with nobody on site, what does it
get wrong, and would anybody find out?* The full audit, with evidence and the
gates that were observed failing, is `docs/PRODUCTION_AUDIT.md`; a second pass
after `v0.14.0` is `docs/FIELD_READINESS_AUDIT.md`.

Several of these were invisible to a fully green 2 190-test suite.

A **third** pass, `docs/ENCLOSURE_READINESS_AUDIT.md`, deliberately did not
start by reading the code — it built the thing, served it, fetched from it with
a browser and with `curl`, timed it, and went to the source only to explain a
number. Almost nothing it found is visible in the source; it is visible in the
bytes on the wire and in the packages that are and are not in an image.

### Fixed — what running it turned up

- **The Docker image could not record.** The runtime stage installed six
  packages and none of them was `alsa-utils`, `ffmpeg` or `sox`. The daemon does
  not capture in-process: it spawns `arecord` for every ALSA microphone,
  `ffmpeg` for RTSP / PipeWire / Listen→Live, and `ffmpeg`/`sox` for clip
  conversion. So `docker compose -f docker-compose.yml -f docker-compose.alsa.yml
  up -d` — a shipped overlay whose entire purpose is USB microphone capture —
  produced a container that starts, serves the whole dashboard, passes its own
  `HEALTHCHECK`, and records nothing.

  `install.sh` already carried this exact lesson for the bare-metal path
  ("…on a minimal Debian it produces [the failure]"), and `debian:trixie-slim`
  is a minimal Debian. Two gates now hold the line: a Rust test cross-checking
  every `Command::new` against the Dockerfile's package list, and a `docker.yml`
  step that resolves each binary inside the built image on both architectures.

  Classifying the spawns for that gate found a second defect. `is_tool_available`
  forked `which`, which is not POSIX and which Debian's `debianutils` no longer
  ships — so the probe could fail with `ENOENT` on this very image and
  `CaptureManager::start` would refuse with `arecord not found in PATH` while
  `arecord` sat on the `PATH`. It is now a `PATH` walk that checks the execute
  bit, and the second copy in `src/doctor.rs` delegates to it instead of
  answering the same question differently.

- **Nothing was compressed.** No response carried a `Content-Encoding`, with or
  without `Accept-Encoding`. Eight representative paths measured 596 712 bytes
  on the wire; with gzip they are **144 832 — 4.1×**. `app.css` alone is
  212 950 → 43 614.

  The predicate is an allow-list rather than `tower-http`'s default deny-list,
  because the default would have compressed the audio route's `206 Partial
  Content` responses without rewriting their `Content-Range` — a corrupt clip in
  every `<audio>` element that seeks.

  Writing the gate found the defect that mattered. Placed *inside*
  `security_headers_middleware` — which buffers `text/html` and runs
  `String::from_utf8_lossy` to stamp CSP nonces — every gzip stream came back
  with its `0x8b` magic byte replaced by U+FFFD. Correct header, plausible
  length, and not one page decodable in any browser. The layer is now outermost,
  and the gate inflates the body instead of trusting the header.

- **Spectrogram PNGs were stored, not compressed.** The encoder emitted type-0
  (stored) DEFLATE blocks, with the comment "not great compression but no
  dependency and correct output". Measured on real served responses: a full
  spectrogram **499 431 → 67 310** bytes, a Recordings thumbnail
  **164 046 → 26 994**, and the twenty-thumbnail `/recordings` grid
  **3.28 MB → 0.54 MB**. The "no dependency" half had stopped being true —
  `flate2` and `miniz_oxide` were already resolved in `Cargo.lock`.

  The CRC-32 table was also being rebuilt once per PNG chunk; it is a
  `LazyLock` now.

- **`/station` blocked for 200 ms on a `thread::sleep`.** 238 ms serially
  against 4 ms for `/patterns` — 60× the next slowest page, all of it a sleep
  between two CPU refreshes, because a freshly constructed `sysinfo::System` has
  no previous refresh to subtract. The function's own doc comment said to call
  it from a background task; six call sites did the opposite, and two of them
  paid the sleep **only to read a CPU temperature** that comes from sysfs.

  One process-wide `System` handle now makes the delta "since the previous
  caller", which for a page polled once a minute is a better window than a
  200 ms slice, and `cpu_temperature()` is its own function. The Today rail
  polls that path `every 60s`, so a kiosk was holding a blocking thread for
  200 ms a minute, forever.

- **The migration Upload tab computed the "this file is from somewhere else"
  warning and threw it away.** *(Behaviour change: `POST /admin/migrate/upload`
  no longer imports. It stages and returns the report; the new
  `POST /admin/migrate/upload/confirm` imports. Anything scripted against the
  old one-step endpoint needs the second call.)* It ran the same validation the Server Path tab
  runs, refused the file only on a *required* failure, and then never read the
  report again — while `location_check` is deliberately never required, which is
  precisely what stopped it reaching the operator. Upload now stages the
  validated file and shows the full report (species preview, date range,
  duplicate count, distance warning); a separate confirm is what imports.

- **`Cache-Control: immutable` on an unversioned stylesheet URL.** `immutable`
  tells the browser not to revalidate even on an explicit reload, so an updated
  station served new HTML against last year's CSS in every returning browser,
  for up to a year. Every `<link>` now carries `?v=<version>`, including the six
  full documents rendered outside the shared layout, and the service worker
  precaches the same URLs.

- **Seven documents showed a blank browser tab and logged a 404.** A document
  with no `rel="icon"` requests `/favicon.ico` unprompted, and the server did not
  route it. `templates/layout.html` names its icons and says why; the seven full
  documents rendered outside it — login, onboarding, kiosk, the share page and
  its 404, the standalone audio player, the admin shell, the log viewer — never
  got the same treatment. The fallback is routed now, which covers all seven and
  the next one.

  Found the first time `/login` was in the visual-QA route table, which it had
  never been: `login__light__desktop: console=["Failed to load resource: … 404"]`.
  After the fix, 152 screenshots and 0 pages with issues.

### Changed — what the gates now see

- **The visual-QA route table is written in the current URLs.** It listed
  pre-spine paths (`/heatmap`, `/weekly`, `/system`, `/admin/audio`, …), which
  still resolve because they 308-redirect, so the homes *were* being tested —
  under names that did not describe them, and only for as long as the redirect
  table stayed put. `/login`, the only screen an unauthenticated visitor can
  reach, was in neither the table nor any redirect.

- **`is_leap_year` and `days_in_month` are in `birdnet_core::civil`.** Two of
  the four remaining hand-rolled copies now go through them. The other two stay
  on purpose — `birdnet-scheduler` deliberately depends on `serde` and nothing
  else, and `src/capture/schedule.rs`'s copy is the oracle its own conversion is
  checked against — with a test that drives the scheduler's private predicate
  through `SolarDay::for_date` and compares it against `civil`'s over every
  February from 1800 to 2400.

### Fixed

- **A reviewer's rejection now reaches every aggregate.** `detections_analytic`
  landed in migration 26 and the surfaces converted to it were the ones someone
  thought of at the time. The rest kept counting rejected detections: the
  published RSS/JSON/ICS feeds (including `/feeds/rare.*`, whose "new species"
  date comes from `MIN(Date)` — so a rejected row that happened to be the
  earliest announced a first-detection date the life list disagreed with, to an
  audience that never sees the correction), the Today phrase and its 30-day
  baseline, the command palette, the next-species prediction's trigger species,
  the dawn-sequence derivation, the species page's showcase clip, and five
  whole-history aggregates in the query layer (`species_for_date`,
  `detections_per_day`, `detection_dates`, `best_detections_for_date`,
  `detection_count_for_species_date`).

  `/api/v2/metrics` deliberately keeps `birdnet_detections_total` counting every
  row — it is a pipeline-throughput signal, and a detection a human later
  rejected still proves the chain ran — and now exports
  `birdnet_detections_rejected_total` beside it, so a dashboard can show either
  the raw rate or the curated figure the web UI displays. `birdnet_species_total`
  is an analytic and excludes rejections.

  Record-level surfaces still show rejected detections on purpose. The review
  queue keeps only the last 25 verdicts, so hiding them everywhere else would
  make an older rejection unreachable through the UI entirely.

- **Fixed recording windows and per-source quiet windows were evaluated in
  UTC.** `fixed:06:00-20:00` on a UTC-8 station really recorded 22:00-12:00
  local — through the night, stopping at midday, missing the dawn chorus it was
  configured to capture. `--doctor` warned about it; nothing fixed it.

  Both are now evaluated against the station's local clock, which is what an
  operator typing "06:00" means. **Solar schedules are unchanged and must be**:
  `SolarDay` reports sunrise and sunset as absolute instants in UTC, so that
  gate is asked in UTC. `DailySchedule::clock()` names which clock each gate
  wants, so the two can no longer be confused by a caller.

  `--doctor` now reports the window with the station's offset instead of warning
  about the old behaviour — an operator who set UTC hours to compensate needs to
  set them back, and is told so.

- **The dawn-chorus sun markers were for the wrong place, the wrong day and the
  wrong clock.** The Today page's solar helper was fixed some time ago; this
  page kept a private copy that was wrong three ways at once. It read the
  coordinates from `BNB_STATION_LAT`/`BNB_STATION_LON` only and otherwise fell
  back to a hard-coded (40.0 N, 74.0 W), so a station that set its location in
  the setup wizard got a sun computed for the New Jersey coast. Its day-of-year
  was `((unix_secs / 86_400) % 365) + 1`, which drifts about a day a year — 14
  days out by 2026, moving sunrise 18 min at Boston, 27 min at London and 40 min
  at Oslo — and wraps to January in late December. And it returned UTC hours
  while the chorus ribbons it was drawn over are bucketed from the local `Time`
  column. Its own tests asserted UTC while its doc comment claimed "local-civil
  hours".

  Both pages now use one helper, backed by `birdnet_scheduler::SolarDay`. With
  no configured location the markers and the night wedge are omitted rather than
  guessed. The guide page that told operators to run their station on UTC — 
  advice for a defect, and in direct contradiction of `--doctor` — is corrected.

- **`df` was invoked with GNU-only flags on the path that keeps the disk from
  filling.** The capture disk manager passed `--output=size,used,avail -B1`,
  which are coreutils extensions that neither BSD `df` (macOS, a documented
  target) nor BusyBox accepts. `disk_usage` then errored, and the disk manager
  reads an error as "cannot tell" and skips the purge — so a station whose card
  was filling up never reclaimed anything, silently. There were two `df`
  implementations in the workspace and only the doctor's was POSIX; there is now
  one, gated against GNU, BSD and BusyBox output fixtures.

- **`docker compose up` could not start the container.** `docker-compose.yml`
  interpolated fifteen optional settings as `KEY: ${KEY:-}`, which puts the key
  in the container environment as an *empty string* whether or not anyone set
  it. clap reads an empty environment variable as a supplied value, so
  `BIRDNET_LATITUDE=` means "the latitude is the empty string" and exits 2
  during argument parsing. Four such variables blocked startup in sequence —
  latitude, longitude, `--mqtt-ha-discovery`, and a panic on a blank Apprise URL
  — and `restart: unless-stopped` made that a loop rather than a failure with a
  visible cause. `quickstart.sh`, which fills in the first two, still died on
  the third.

  Nothing caught it: the only container check in CI runs `--verify-extension`
  with the entrypoint bypassed and no environment at all, and the Rust suite
  never sees an environment variable. `scripts/check-compose-startup.sh` now
  resolves the real container environment with `docker compose config` and
  starts the real binary under it, in the `build` job.

  Blank values no longer reach the binary from three directions:
  `docker-compose.yml` stops manufacturing them, `.env.example` ships the
  optional keys commented out, and `docker/strip-blank-env.sh` (sourced by the
  entrypoint) strips any that survive. `BIRDNET_IMAGE_CACHE_DIR` is exempt —
  an explicitly empty value is the documented air-gapped opt-out.

- **A blank Apprise URL aborted the daemon during startup.** `APPRISE_URL=`
  with no `APPRISE_CONFIG_FILE` reached an `.expect` and panicked; the settings
  page's own hint says to leave it blank to disable notifications. Release
  builds are `panic = "abort"` and the unit pairs `Restart=always` with
  `StartLimitBurst=5`, so a station in that state burned its five restarts in
  fifty seconds and stayed `failed`. Blank and whitespace-only values are now
  treated as absent.

- **One time-series page silently un-applied every reviewer rejection.**
  `birdnet-behavioral` and `birdnet-timeseries` both created a DuckDB view named
  `detections_ts` with `CREATE OR REPLACE`, on the same connection — and only
  the behavioural one carried the `review_verdict` filter. The last one to run
  therefore decided what *both* crates saw for the rest of the connection's
  life: opening a single time-series page put rejected detections back into
  sessionize, retention, funnel, next-species and co-occurrence until the next
  full sync. Measured on a three-detection fixture with one rejection,
  `COUNT(*) FROM detections_ts` went from 2 to 3 across one `quiet_days` call.

  `tests/analytics_divergence.rs` could not see this — both stores agreed; the
  view changed underneath them. `tests/analytics_view_ownership.rs` now gates
  both the texts and the behaviour, with a counterpart proving unreviewed
  detections still survive.

- **The dashboard's headline tiles disagreed with each other.** "Species",
  "Last hour" and the 12-day sparkline excluded rejected detections; "Detections",
  "Today" and "Species today" counted every row. Adjacent tiles contradicted each
  other by exactly the number of rejections the operator had recorded, so the
  more carefully someone curated, the wronger the screen got. The presentation
  side now reads new `analytic_*` counters; `detection_count` deliberately keeps
  counting every row, because the SQLite-vs-DuckDB reconciliation depends on it.

  The gate that should have caught this was a tautology — it asserted
  `SELECT COUNT(*) FROM detections_analytic`, i.e. the view's own `WHERE` clause
  restated to itself, while claiming to cover "species totals, the heat map, the
  dawn chorus, phenology". It now reads through the query layer.

- **`RECORDING_SCHEDULE=solar` recorded nothing across most of the world.**
  `SolarDay` reports sunrise and sunset wrapped into the *UTC* day. Away from
  Greenwich the two ends of one local day land on different UTC days, so the
  wrapped sunrise minute comes out *larger* than the wrapped sunset minute —
  19:33 to 05:11 in Auckland, 09:24 to 00:30 in New York in June. `NightInhibit`
  compared them as a plain `from <= m < until`, which is empty whenever
  `from > until`, so the schedule allowed **zero minutes of recording per day**.

  Measured against `SolarDay` directly, every day of 2026: Bangkok, Beijing,
  Tokyo, Sydney, Auckland, Seattle, Phoenix, Anchorage and Honolulu wrap on
  **all 365 days**; Denver on 288, Austin on 265, Chicago on 182, Toronto on 136
  and New York on 94 — and all five of those wrap on **every day of June**.
  London wraps on none. Roughly −75° to +75° longitude worked, and not
  year-round.

  Every pre-existing test in `birdnet-scheduler` used London (51.5074, −0.1278),
  a longitude where the wrap never happens — which is exactly why a green suite
  said nothing about it. `tests/solar_window_worldwide.rs` walks sixteen
  stations across the solstices and equinoxes and was watched failing on
  fourteen of them. `NightInhibit` is
  wrap-aware now, its offsets wrap instead of clamping (a 30-minute pre-roll on a
  00:05 sunrise used to lose 25 of them), and offsets wider than the clock
  resolve to "always" rather than "never". Two tests that pinned the clamping are
  replaced, and both replacements plus the "not the complement" counterpart were
  confirmed to catch a mutated fix.

  `--doctor` reports the resolved window in both clocks and now **fails** on a
  schedule that allows no minutes. The failure was silent; the only signal was
  the detection deadman, hours later, reporting the wrong cause.

- **The published feeds told every calendar the wrong time.** A detection's
  `Date`/`Time` is local wall clock with no offset. `rare.ics` appended `Z` and
  both RSS feeds appended `+0000`, asserting Greenwich — so on a UTC−4 station a
  20:46 detection showed as 16:46, and on UTC+8 as 04:46 the next morning, in
  whatever calendar or reader had subscribed.

  `DTSTART` is now a floating local value (RFC 5545 §3.3.5 form 1, the honest
  reading of a row that carries no offset), `DTSTAMP` is a real UTC instant as
  §3.8.7.2 requires rather than the detection time relabelled, `pubDate` carries
  the station's actual offset, and content lines are folded to §3.1's 75 octets
  on UTF-8 boundaries. A test that asserted `...061432Z` — the defect, pinned as
  the contract — is replaced; three mutants, three catches.

- **A browser-uploaded BirdNET-Pi import threw away where it came from.**
  `upload_and_run_handler` read the station's coordinates to validate the file
  and then called the bare `run_migration`, which is
  `run_migration_with_options` with `ImportOptions::default()` and
  `station = (None, None)`. So `station_lat`, `station_lon` and `distance_km`
  were NULL on every browser import, and the Patterns note naming an imported
  foreign site could never fire — it keys on a distance nothing computed.
  Verified against a running station: 3 000 Perth detections uploaded to a
  Boston station produced no warning anywhere.

  The upload tab also carried no origin fields at all, so a browser user could
  not reconcile an eight-hour clock difference even in principle; the
  reconciling flow existed only on the tab that needs the file already on the
  station's disk. Both halves are fixed and gated, and the note fires with the
  right distance.

- **The hour daylight saving repeats could refuse a detection outright.** Local
  wall clock is this schema's identity, so the second pass of that hour can
  produce a clip filename identical to the first — `hound` truncates the
  original — and then collide on `idx_detections_unique` and be refused. Both
  halves reproduced. Clip extraction now claims an unused path (up to
  `MAX_CLIP_NAME_ATTEMPTS`, 50) instead of clobbering, and a refused insert is
  counted on `birdnet_detection_write_failures_total` rather than only logged:
  on an unattended station a `warn!` is the same as not noticing.

  The raw-segment overwrite this pass first suspected turns out **not** to be
  reachable — the stream directory drains by age at
  `DEFAULT_STREAM_RETENTION_SECS` (600 s), well inside the repeated hour — and
  the runbook now says so rather than implying a loss that does not happen.

- **Closed disclosures fetched everything they were hiding.** htmx fires
  `hx-trigger="load"` inside a closed `<details>`, and so does `revealed`,
  because a zero-size element counts as revealed. Both confirmed in a real
  browser, which is also how `toggle from:closest details once, intersect once`
  was chosen: it defers while collapsed and still loads immediately if the
  disclosure is `open`. Eight panels across the Patterns tabs were rendering and
  shipping content nobody had asked to see, including 24 KB of dawn-chorus
  table. `/patterns?tab=trends` goes from ten panel requests to five. A
  structural gate scans every template and Rust-rendered `<details>` for an
  eager trigger inside it, so a ninth panel cannot reintroduce this quietly.

- **Two command-palette entries led nowhere, and nine landed at the top of the
  wrong thing.** `routes::pages::nav` says the six homes are the whole top-level
  menu and that the long tail "stays reachable through the command palette and
  contextual links" — which makes the palette load-bearing, and nothing had ever
  asked the router whether its entries resolved. Walking them against a running
  station:

  - **Migrate** pointed at `/admin/migration`, a route that has never existed
    (it is `/admin/migrate`), so the entry 404'd.
  - **Display · prefs** pointed at `/system#display-prefs`. `/system` is a
    pre-spine path that 308s to `/station`, which drops the fragment, and
    `/station` carries no `display-prefs` anchor anyway.
  - **`/admin/audit` and `/admin/images`** matched no palette query and were
    linked from none of the eleven primary pages. The only way to either was to
    already know the URL — an audit log nobody can find is not an audit log.
  - The other nine settings entries pointed at pre-spine `/admin/*` paths that
    redirect correctly but land at the **top** of a merged tab. `/station/data`
    is 82 KB of backups, import and quality in one document, so "Quality" took
    you to a page whose quality section is somewhere below, with nothing saying
    which part you had asked for. Same for the eight legacy `/admin/*` bookmarks
    a veteran still has.

  The five merged Station tabs carry section anchors now, every palette entry
  and every legacy redirect names one, and Audit log / Species images / System
  status are findable. A redirect that loses the destination is only marginally
  better than the 404 it was written to avoid, so the anchor is part of the
  redirect contract and `folded_pages_redirect_to_their_station_tab` asserts it.
  `every_palette_destination_resolves` walks each static destination through the
  real router, follows redirects the way a browser would, and checks that any
  fragment names an `id` that exists on the page it lands on; it was watched
  failing on all three defects above. `/help` is exempt with a comment — it is a
  `ServeDir` over `BNB_HELP_DIR`, which the installer and the Dockerfile both set
  and a bare `cargo test` does not, so its 404 in a fixture is the documented
  "docs unavailable" path rather than a rotted link.

- **Documentation that contradicted the code it documents.** Two doc comments
  claimed fixed recording windows are evaluated in UTC — they have been local
  since F-10 — and the shipped manual told operators quiet windows are UTC while
  the UI, the code and its tests all say local. Also fixed: no prose anywhere
  described `--channel-report`, the one command that answers whether a stereo
  microphone is costing the station 66 dB to its own downmix.

  And a `--doctor` unit test that read whatever real station database happened to
  sit at `$HOME/BirdNet-Behavior/birds.db` rather than a fixture. It was observed
  failing for exactly that reason, having passed minutes earlier in the same
  tree.

### Added

- **`--rebuild-species-summary`** recomputes the per-species totals from the
  detections. Derived data, so rebuilding cannot lose anything; `--doctor` names
  it if it ever finds the summary and the detections disagreeing. Nothing else
  should need it.

- **A mixed-workload soak.** The existing soak tests each drove one operation
  repeated — 20 000 inserts, or one corrupt file recovered. A station's year is
  detections arriving interleaved with a reviewer confirming, rejecting and
  changing their mind, relabels, deletes, the bulk clip-prune job, and restarts.
  The new test drives 20 000 of those shuffled together, reopening the database
  every 2 500, and checks the maintained summary against a recomputed aggregate
  throughout. Seeded and replayable (`BIRDNET_SOAK_SEED`, `BIRDNET_SOAK_OPS`),
  and it asserts every branch was actually taken, so a schedule that happened
  never to reject anything cannot pass as full coverage.

  It is not a substitute for running a station for a week — still the largest
  untested thing here — but it covers what that week would stress and ten
  seconds can reach: state surviving restarts, resources bounded across them,
  and the summary staying the size of the species list rather than the history.

- **Per-source quiet windows are settable.** `schedule_quiet` has had a column
  since the audio-sources table landed, the capture supervisor has always
  honoured it, and *nothing wrote it* — every construction site in the tree
  passed `None`, so the only way to set one was direct SQL against the database.
  The audio-source edit form now carries both ends, blanking both removes the
  window, half a window is refused rather than half-saved, and a source with one
  says so on its row (a source that goes quiet every night is otherwise
  indistinguishable from one that has failed).

- **A merged history is visible on the charts it changes.** Migration 25 tagged
  every imported detection with its origin — coordinates, distance, the clock
  shift applied — and nothing ever read it. `birdnet-migrate` warns *before* an
  import that the source is 340 km away and rightly does not block, but
  afterwards every location- and hour-dependent analytic read the union as one
  station with nothing saying so, which is not detectable after the fact.

  The Patterns screens now carry a note naming the source, its distance and
  whether the two clocks were reconciled, plus a link to what was imported. It
  renders nothing for a station that imported nothing, and nothing for the
  common case of importing your own BirdNET-Pi history — a false alarm on every
  station that ever imports is how a banner gets ignored by the time it matters.
  `birdnet-db` gained the read API this needed (`list_import_batches`,
  `imported_detection_count`), which did not exist at all.

- **The station now measures itself, not only the birds.** A microphone that
  fails outright is caught three ways: the supervisor restarts its process,
  `birdnet_audio_source_up` drops, and the detection deadman fires. A microphone
  that merely goes **deaf** — water in the capsule, a spider's web across the
  port, a connector loosened by a year of thermal cycling, a preamp drifting —
  is caught by none of them. The process lives, the gauge reads 1, audio keeps
  arriving, and the station goes on detecting the loud close birds while quietly
  losing everything else. Its only symptom is fewer detections. So is the end of
  the breeding season.

  The station's own noise floor separates the two, because ambient background
  does not stop when the birds do. Measured through this project's decode path
  on its own 15-second magpie recording, attenuated to 2 %:

  | gain | noise floor | SNR |
  |---|---|---|
  | 1.00 | −42.5 dBFS | 2.7 dB |
  | 0.02 | −77.3 dBFS | 2.9 dB |

  35 dB on the floor; SNR does not move, because attenuation scales signal and
  background together. A version of this built on SNR would have looked entirely
  reasonable and detected nothing — which is why the gate asserts the
  discrimination rather than the plumbing.

  Migration 31 adds `audio_levels`, one row per (local date, hour, source),
  holding sums and a count so a new observation folds in without revisiting the
  old ones, plus the minimum — which a failing capsule drags down first, and
  which a reflexive `DO UPDATE SET x = excluded.x` would silently turn into "the
  last sample". A sampler takes the newest segment per source every five
  minutes, decodes ten seconds of it and folds one observation in: the same
  sample-don't-instrument trade as `integrations::effort`, so it cannot disturb
  capture, costs milliseconds, and loses at most one interval to a restart.
  Newest by mtime, not by filename — those carry local wall clock, which repeats
  for an hour every autumn.

  Surfaced as a **Microphone health** panel on the Station Health tab and as
  `birdnet_noise_floor_dbfs` / `birdnet_noise_floor_drift_db` per source on
  `/api/v2/metrics`, drift measured against that source's *own* preceding 30-day
  average. A source with no baseline yet reports "building a baseline" and
  exports no drift series at all, because "never measured" and "has not moved"
  are different answers.

  **No threshold and no alert ship with this, deliberately.** A noise floor
  moves for weather, season, a road, leaf-out. A number picked now, without a
  season of real recordings to calibrate against, would fire on all of them and
  teach an operator to ignore the channel — the exact failure
  `integrations::station_health` is written to avoid. The measurement comes
  first.

  `birdnet_core::audio::quality` — 1 310 lines of SNR, spectral flatness,
  rain/wind assessment and noise-floor tracking — had had no production consumer
  since it was written; only the benches referenced it. The deferral recorded in
  `src/cli.rs` is right about the half it covers: *filtering* changes which
  chunks reach the model and wants hardware validation first. It says nothing
  about *observing*, which changes no behaviour at all, and that is the half a
  sealed station needs.

- **Every detection now carries the instant it happened, beside the local wall
  clock it is displayed in.** `Date`/`Time` are local with no offset recorded —
  the shape BirdNET-Pi wrote, kept so a decade of existing databases still
  imports — and that pair is not a point in time. One local hour repeats every
  autumn and one never happens every spring, so *everything measured on it* was
  wrong across a transition, in both directions:

  - the detection deadman subtracted two wall clocks, so on the autumn night the
    station read up to an hour **fresher** than it was (delaying the alarm) and
    on the spring one up to an hour **staler** (firing a false one);
  - session gaps read **zero minutes** between two detections a real hour apart
    on the autumn night, merging two sessions that were separate, and
    **seventy-five** between two fifteen real minutes apart on the spring one,
    splitting one that never broke — either side of the 30-minute default;
  - `sessionize`, `window_funnel`, `sequence_match`, `sequence_next_node` and
    every gap query took the wall clock as their time argument;
  - "latest detection" was `ORDER BY Date DESC, Time DESC`, i.e. lexical — so a
    single imported row with an unparseable date (`not-a-date` sorts above every
    real date) made itself the station's most recent detection, on the dashboard
    and in the freshness signal.

  Migration 32 adds `detected_at_utc` and backfills it through the host's tz
  database **for each row's own date**, so history recorded under a different
  offset converts with the offset that was actually in force rather than
  today's. A trigger covers write paths that forget the column, including ones
  not yet written; the live detection path sets it explicitly because it is the
  only writer that can tell the two passes of the repeated autumn hour apart.

  The analytics view names the two clocks separately — `detection_instant` and
  `detection_timestamp` — and the rule is now written down and gated in both
  directions: **elapsed time and ordering ask the instant; clock position,
  calendar date and anything shown to a human ask the wall clock.** Hour-of-day
  charts and daily buckets deliberately did *not* move.

  Rows whose wall clock names no point in time keep a NULL instant and drop out
  of ordered results rather than being invented at the epoch. Paginated list
  queries stay on `ORDER BY Date, Time`: the covering index supports them, and
  the only error is intra-hour ordering for one hour a year.

  An upgrading station with a populated `analytics.duckdb` was a hazard in its
  own right — the new column adds no rows and changes no verdicts, so both
  existing drift signals agreed while every instant in the copy was NULL, which
  would have left sessionize, funnel, retention, next-species and every gap
  query silently returning nothing. The startup drift check gained a third
  signal for exactly that and rebuilds the copy.

### Changed

- **The health badge and `/api/v2/health` no longer scan the whole database.**
  Both ran `PRAGMA quick_check`, which reads every page of the file. The badge
  is mounted in `layout.html` with `hx-trigger="load, every 30s"`, so that was a
  full read of the database on every page load and twice a minute per open tab,
  forever, competing with the detection write path for the same SD card.

  Measured on a seeded three-year station (2 755 374 detections, 1.29 GB, warm,
  NVMe): `/pages/health-badge` **3.79 s → 0.0037 s**; the pragma alone cost
  1.5–1.9 s. A Raspberry Pi reading that file from an SD card at ~45 MB/s is
  looking at roughly 30 s — longer than the badge's own refresh interval. The
  container `HEALTHCHECK` polls `/api/v2/health` every 30 s with
  `curl --max-time 4`, which a station with real history could not meet.

  Migration 28 stores the daily integrity check's verdict, which that job was
  already computing and throwing away; both surfaces now read one row.
  `/api/v2/health` still probes reachability per request, and reports
  `database` as `"ok"`, `"unchecked"` or `"error"` rather than collapsing "not
  yet verified" into "broken" — `"unchecked"` returns `200`, so a freshly
  started container is not marked unhealthy for the five minutes before the
  first maintenance tick.

- **The species screens' whole-history aggregates are index-only.** The species
  list, the life list and the per-species hour histogram each aggregate the
  entire detection history, uncached, on every page load, so their cost grows
  with how long the station has been useful. Migration 29 adds two covering
  indexes. Measured on the same three-year database: species list 4.96 s →
  1.31 s, life-list firsts 4.12 s → 0.58 s, hour histogram 4.82 s → 1.15 s.

  Cost, measured rather than estimated: +130.6 MB (9.0 % of the file); inserts
  0.20 → 0.27 ms per committed row, three orders of magnitude above what a
  station produces. A third index would take the species list to 0.31 s for
  18.6 % total, which is the wrong trade on an SD card. This made the aggregates
  cheaper, not bounded; migration 30 below makes them bounded.

- **A verdict older than 25 was unreachable.** The review queue showed the last
  25 verdicts and nothing else — and it is the only surface that lists rejected
  detections, so a rejection that fell off the end could not be found, let alone
  undone, except by a saved URL. It is paginated now, with a status filter and a
  total, so every verdict a reviewer has ever recorded stays reachable.

- **A share link outlived the claim it published.** `/share/<id>` read the raw
  detections table, so a detection a reviewer had rejected kept serving a public
  page asserting the station heard that bird — the one surface where being wrong
  reaches an audience that never sees the correction. Share pages read
  `detections_analytic` now, and a withdrawn detection's link returns 404.

- **Ten copies of the same civil-date arithmetic, and three URL escapers.**
  Every implementation of Howard Hinnant's days-from-civil algorithm was checked
  against every other over 1970–2170: all thirteen agreed, zero mismatches — so
  nothing was *broken*, but ten chances for the eleventh to be wrong were. There
  is one in `birdnet-core::civil` now. The URL escapers differed in exactly one
  respect (whether `/` is escaped), which is the difference between encoding a
  path and encoding a segment; both are now named for what they do, with a gate
  asserting the slash is the only thing between them.

- **The species aggregates are now bounded by the species count, not the
  history.** Migration 29 made them cheaper; they still read every detection
  ever recorded, so opening the species list got slower every month the station
  ran. Migration 30 adds `species_summary` — one row per (common name,
  scientific name, hour of day), maintained on write — so the species list, the
  per-species hour histogram and the distinct-species count read a few thousand
  rows instead of millions, permanently.

  Measured on a seeded three-year station (2 755 374 detections, 86 species,
  1.47 GB, warm, x86_64 NVMe), with the histogram measured against the busiest
  species (79 602 detections):

  | query | before | after |
  |---|---|---|
  | species list (top 100) | 1 482 ms | **0.53 ms** |
  | per-species hour histogram | 138 ms | **0.07 ms** |
  | distinct species count | <1 ms | 0.34 ms |

  Cost: **+1.0 MB, 0.07 %** of the file — migration 29's indexes cost +130.6 MB
  (9.0 %) for a fraction of this, because an index scales with the detections
  and a summary scales with the species. Inserts 0.0671 → 0.0724 ms/row
  (+7.9 %); 1.9 s to backfill once. The distinct-species count is a wash and
  moved anyway, so every species-level fact comes from one place.

  Maintained by SQLite **triggers**, not by a function the write paths call:
  `detections` is written from four crates and at least eight call sites, and
  the ninth would drift silently. First-seen dates are deliberately *not*
  summarised — MIN/MAX cannot be reversed on delete, so the life list stays on
  migration 29's covering index rather than buy a rule that could drift.

  A materialised aggregate that can drift is worse than a slow query, so
  `--doctor` now reports whether the summary still agrees with the detections
  (a **warning**, never an error — a stale species count is no reason to stop
  recording birds), and `--rebuild-species-summary` recomputes it. The rebuild
  fails loudly if drift survives it, because that would mean something is
  writing in a way the triggers cannot see.

- **Five broken links in shipped documentation**, live on `main` and passing
  CI: three copies of `guide/today.md#rare-bird-review-queue` (a heading that
  never existed), `remote-access.md#built-in-http-basic-auth`, and
  `backups.md#import--export`.

  They passed because the manual was being rendered *twice* — GitHub Pages by
  the mdBook 0.4.52 CLI with the `mdbook-linkcheck` backend, and `build.rs` by
  `mdbook-driver` 0.5 for the in-app `/help/*` tree, from a second `book.toml`
  with a different theme, no custom CSS and no folding. Same pages, two
  different-looking sites, and only the published one was checked at all — by a
  backend that was not catching these.

  There is one `book.toml` now, at the repository root, and one mdBook version.
  `scripts/check-book-links.py` replaces the 2022-vintage backend by checking
  the *rendered HTML*, which is renderer-agnostic and so covers the published
  site and the in-app manual with one check; it runs in both `docs.yml` and
  `ci.yml`, so a broken link fails any pull request rather than only one that
  touches `docs/**`. The custom theme now reaches the in-app manual for the
  first time.

- **The settings page's structure was visible but not real.** All eight section
  titles on `/admin/settings` were `<div class="section-title">` — styled at
  1.1 rem, semibold and underlined, so they read as headings to anyone looking
  at the screen and as ordinary text to everything else. A screen reader got no
  document outline, "jump to next heading" did nothing, and the page's entire
  organisation was invisible to the accessibility tree. The cards are now
  `<section>` elements labelled by real `<h2>`s, on the standalone page and on
  the Station tabs that share the same renderers.

  Converting them surfaced a cascade collision worth recording: `.card h2` in
  `app.css` is the card *eyebrow* (11 px, uppercase, muted) and its specificity
  (0,1,1) beat a bare `.section-title` class (0,1,0), so the new headings
  rendered smaller than the field labels beneath them until the settings rule
  was raised to match.

- **Links inside settings hints were distinguished by colour alone**, which
  axe-core flags as `link-in-text-block` (WCAG 1.4.1). They are underlined now.
  With this and the section work, `/admin/settings` reports **zero** axe
  violations in both themes with every rule enabled, including the two the CI
  gate defers.

### Added

- **An "On this page" index and a type-to-filter on the settings page.** It
  carries 54 controls over about five screens, and it had no way to move
  between them but scrolling. The index is a sticky jump list beside the
  sections on desktop and a wrapped row above them on a phone; the filter
  narrows to matching sections as you type, matching heading text, field
  labels, hints and the underlying config keys — so `sf_thresh` finds Detection
  Settings by the name you would read in `birdnet.conf`.

  Deliberately not a collapse: the Station tabs already own task-scoped access
  to these same sections, so this page's distinct job is holding everything at
  once and staying findable — including by the browser's own Ctrl+F, which
  stops matching inside a closed `<details>` in most engines. Nothing is hidden
  server-side. The filter ships `hidden` and its script reveals it, so with
  JavaScript off the page behaves exactly as before rather than offering a
  control that does nothing.

- **The default theme shipped text below the WCAG AA contrast floor, and the
  gate that should have caught it was configured not to look.** Measured with
  axe-core across every screen in both themes: **78 serious violations, 1 280
  offending elements**. The accessibility job passes because `AXE_DISABLE`
  defaults to `color-contrast,link-in-text-block` — the two rules that were
  failing. It was not a dark-mode problem; light was worse (42 of the 78).

  The largest single cause was the `--fg-4` ink tier: 2.55:1 in light and
  2.40:1 in dark, against a 4.5:1 requirement, on 9.9–10.5 px text. The project
  already knew the safe values — `data-contrast="high"` sets exactly them — so
  accessibility was available to anyone who went looking for the setting and to
  nobody else. The default now uses them, and high-contrast moves further out.

  Four more root causes, each measured rather than guessed: `--fg-3` passed
  against the base background (4.64:1) but not against the tinted surfaces it
  actually sits on (4.48:1); `.btn-primary` painted hardcoded white on `--moss`,
  which is a dark green in light (4.67:1) and a bright green in dark (1.87:1),
  so the app's primary action failed in dark mode; "enabled"/"sent" badges put
  `--moss` on `--moss-soft` (3.75:1) where `--moss-ink` gives 8.73:1; and the
  history calendar mixed each cell's fill toward green in proportion to the
  day's detection count while the label colour stayed fixed, so the busiest days
  were the least readable — 1.09:1 at the top of the ramp. The ramp now stops at
  80 % (5.19:1) and the in-cell label no longer uses the faintest tier.

  Together these take the audited violations from 78 to 47. What remains is one
  class needing a design decision rather than a fix: species-identity colours
  used as 9.5 px text on pastel tints, at 2.6–3.0:1.

- **The six Station screens had no `<h1>`.** Each composes a sub-tab strip plus
  a content fragment, and neither carries a page heading, so their first heading
  was an `<h2>` and a screen reader had no page title to announce.

### Changed

- `.btn-primary` and friends take their ink from a new `--on-moss` token
  instead of hardcoded white. Six admin pages had re-declared `.btn-primary`
  inside their own inline `<style>` blocks with `color:#fff`, so the shared
  component's token could not reach them.

- **Six of the eight `# observed` runtime notes in CI were false, by up to a
  factor of ten.** Each `timeout-minutes:` carried the runtime its budget was
  sized against, written by hand and never revisited, so they had quietly
  come to describe a repository thousands of commits ago: `Clippy` claimed
  `# observed 54s` while really taking 8m45s, `Tests` claimed 10m42s against a
  real 21m59s, and `MSRV`, `Rustdoc`, `Build` and `Inference` were out by
  5-10x. Nothing was wrong in a way a reader could see — which is what made
  them worse than no note at all, since they were the evidence a reviewer would
  use to judge whether a budget was sane.

  Updating the numbers would only have restarted the same clock, so they are
  now generated from run history and gated on drift, the way
  `scripts/gen-cli-help.sh` already keeps the CLI docs from drifting from the
  binary. Every job-level timeout in every workflow now carries a current note,
  `check-ci-config.py` fails when one is more than 1.5x from the measured
  median, and `--update-observed` rewrites them. Drift is measured against the
  median rather than the worst run so a single cold-cache outlier cannot redden
  an accurate note. The mutation workflow's path filter now covers every
  workflow file, not just its own, because the check no longer only looks at
  its own.

- **A mutation row was 87% of the way into its own timeout and nothing was
  watching.** The config gate added last cycle checks that a matrix row still
  matches source, that no shard is empty by construction, and that every job
  declares a timeout — but never the distance to that timeout. `validate.rs`
  had grown to 67 mutants and 39m00s against a 45-minute budget, and the only
  reason anyone knew was reading run times by hand. It is the same trajectory
  that took `sqlite/queries/detections` down: a job cancelled at its budget
  renders as a grey badge rather than a red one, so the row stops gating
  without ever going red. `validate.rs` is now split across two shards (34 and
  33 mutants, enumerated rather than derived), and the gate reads each job's
  recent wall-clock from the Actions API and fails any job that has used more
  than 75% of the budget it declares. Pointed at the unsharded row it reports
  it at 87%, alone among 56 jobs — the finding that prompted this, now found by
  CI instead of by hand.

- **"Still expected" read zero for the last six weeks of every year.** The
  migration page's six-week look-ahead was a day-of-year `BETWEEN` against
  `strftime('%j','now')` and `strftime('%j','now','+42 days')`. From 20
  November the end of that window falls in the next calendar year, so its day
  number is *smaller* than the start's — 20 November 2026 gives `'324' … '001'`
  — and the range matches nothing at all. The tile reported a confident "0 ·
  no overdue migrants" through the entire late-autumn arrival season, which is
  the one stretch of the year it exists for. The window is now expressed as
  real dates and the prior year's arrivals are re-based onto both this year and
  next, so crossing the boundary is just the second candidate matching. The
  same rewrite drops two smaller errors in the old form: `'now'` was UTC
  against a locally-dated column, and day-of-year is a day out between a leap
  year and a common one.

- **The migration chart's "today" line was drawn in the wrong place.** It was
  positioned by `(days since 1970 % 365) / 7`, which is not a week number: it
  ignores leap days, so it had drifted a fortnight by 2026, and it counts from
  1970 rather than from January, so on 31 December it returned week 1 and drew
  the marker at the far left of a chart whose data ends at the far right. It
  now uses the same `%W` week the chart's own buckets are grouped by, checked
  against SQLite for agreement. The page's current year is read from the
  station's local clock for the same reason.

- **Arrival dates drifted by a day whenever a leap year was involved.** The
  phenology queries derived `first_doy`/`last_doy` from a raw day-of-year, which
  from 1 March runs one higher in a leap year — 1 May is day 122 of 2024 and day
  121 of 2025. The multi-year percentiles behind the migration window averaged
  the two scales together, so every arrival and departure estimate spanning a
  leap year carried a systematic error of up to a day, and the seasonal window
  was smeared by the same amount. It was worse than noise: a species that
  genuinely advanced by one day between 2024 and 2025 had the shift cancelled
  exactly and was reported as unchanged. Day numbers are now projected onto a
  common year (1–365, with 29 February folding onto 28 February) before any
  comparison, so one calendar date is one number in every year. The ISO dates
  returned beside them were always exact and are unchanged.

- **Your edits never reached the analytics.** Deleting a detection, re-labelling
  one, approving one out of quarantine and "clear all detections" all wrote to
  SQLite alone. The DuckDB copy every behavioural and time-series dashboard
  reads is synced *incrementally*, so it could only ever add newer rows — never
  remove one, never re-read a changed one, never pick up a back-dated one. So a
  deleted false positive kept counting in Patterns forever, a corrected
  identification kept its old name, an approved quarantine detection could never
  arrive at all, and "clear all detections" left the analytics rendering your
  whole history beside a dashboard reporting zero. Nothing reported any of it:
  both stores answered every query, just with different histories.

  All four are now paired writes, and after each start the two row counts are
  compared — when they disagree the copy is rebuilt automatically. That last
  part repairs stations that already diverged, with no operator action.

- **"Today" meant UTC's today.** Five queries compared the local-civil `Date`
  column against `date('now')`. West of UTC the day rolls over during your
  evening, so the RSS/iCal "today" feed returned **nothing** for the last hours
  of every evening — 20:00 to midnight in New York. East of UTC "today" was
  still yesterday. The species sparkline was worse than a shifted window: its
  date axis was built from UTC dates and joined against locally-dated counts.

- **The dawn chorus got slower every season.** Its 30-day window was reading the
  station's entire history — SQLite preferred the species index for GROUP BY
  ordering, then built the temp b-tree anyway. Measured on a synthetic four-year
  station: 72 ms at 60 days, 1 711 ms at four years. Now a range seek: 1 613 ms
  → 27 ms, identical results.

- **A reviewer's verdict changed nothing.** `detection_reviews` has stored
  confirmed/rejected verdicts since 0.11, and exactly one panel ever read them.
  Every other analytic counted a rejected detection exactly as it counted a
  confirmed one, so a season of curation left every chart unchanged. Verdicts
  now exclude a detection from the aggregates in both stores, while the
  record-level views still show it so you can listen again and change your mind.
  Verdicts you have already recorded take effect on upgrade.

- **Live and resynced rows carried different columns.** The real-time DuckDB
  insert wrote six of twelve, so `Lat`, `Lon`, `Cutoff`, `Week`, `Sens` and
  `Overlap` were populated or NULL depending on how a detection got there.

- **Interface.** The phone layout was gated on `pointer: coarse` rather than
  width, so an iPad with a keyboard, a touchscreen laptop and a narrow desktop
  window all got the desktop nav — and the QA tooling, which sets a viewport but
  no touch emulation, had never once rendered the real mobile layout. Half the
  Patterns tabs sat off-screen on a phone with nothing signalling they scrolled.
  Chart series colours were a hash mapped to hue at constant lightness, so pairs
  landed 2–3° apart and were indistinguishable (near-certain at any realistic
  series count). The activity streamgraph had no axes at all, and the caption
  above it described a different chart. "Bursts of singing" listed sessions of
  one detection lasting zero seconds. Row controls ran off the right edge at
  360 px and below, traced to an 8 px footer overflow that was widening the
  layout viewport and dragging the fixed tab bar with it.

- The field runbook's stated memory ceiling was half the real one
  (`MemoryMax=512M` documented, `1G` shipped).

### Added

- **Imports from another station stay attributable.** Importing a BirdNET-Pi
  database used to be indistinguishable from having recorded it: no check
  mentioned coordinates or timezones, and no column separated the two
  afterwards. A merged database could silently hold two sites and two clocks,
  and every location- and hour-dependent analytic read it as one — unrecoverably,
  since nothing could tell the rows apart later.

  The import now profiles the source, warns before it runs when the coordinates
  are not this station's, offers the source station's UTC offset so both
  histories share one clock, and tags every imported row with its origin.

- **Station-health alerts.** The detection deadman answers "is the station
  detecting at all?". This answers the faults a station keeps detecting straight
  through: one microphone down while others record, a disk full enough that
  recordings are being purged, a CPU at its throttling point, a backup or
  integrity check that has not completed in weeks. One alert per episode with a
  recovery notice, after three consecutive polls so a self-healing blip stays
  quiet. On by default; `STATION_HEALTH_ALERTS=false` to disable.

- **Recording effort, and abundance corrected by it.** A detection count is a
  numerator over a denominator nobody was recording: a solar window is six hours
  longer in June than December, a week of downtime removes a week of listening,
  a failed microphone halves the channels. Each moves the count without moving a
  single bird, so comparing raw counts across seasons or years measures the
  station as much as the birds.

  The station now records how long it actually listened, per source per day, and
  `/analytics/abundance` returns detections per hour of listening. `/analytics/phenology`
  exposes per-species arrival and departure, flagging the species for which a
  calendar-year window is not a migration window — a resident would otherwise be
  reported as arriving on 1 January.

- The five operational runbooks — field deployment, security hardening, hardware
  validation, multi-stream deduplication, macOS — are now part of the published
  manual under **Running a Permanent Station**. They were repository files
  reachable only as raw GitHub links.

### Migrations

25 through 32 — import provenance; the denormalised reviewer verdict (backfilled
from existing verdicts); the recording-effort table; a maintenance-run result
column; two covering indexes for the species aggregates; `species_summary`, the
per-species totals maintained by triggers; `audio_levels`, the station's own
input level over time; and `detected_at_utc`, the monotonic instant beside each
detection's local wall clock.

All additive. None rewrites existing rows, and `import_batch_id IS NULL`
continues to mean "this station recorded it". 29 and 30 are the ones with a real
size cost: +130.6 MB and +1.0 MB respectively on a 1.47 GB three-year database,
and 30 backfills from existing detections so an upgrading station gets correct
totals immediately rather than only for detections recorded from then on.

32 backfills too, and its conversion is date-aware: SQLite's `'utc'` modifier
consults the tz database *for the timestamp given*, so a station's older history
converts with the offset that was in force then. Two dates it cannot get right,
because the information is not in the row: the local hour that repeats each
autumn is two real instants under one label and the backfill picks the
standard-time reading, and the hour that never happens each spring is collapsed
onto the adjacent one rather than rejected. An instant that is an hour out for
two hours a year is strictly better than no instant at all for every hour of
every year, and both limits are recorded in the migration itself.

A note for whoever adds migration 33: 30's triggers exist from that point on, so
a later migration that rewrites `detections` in bulk will fire them and the
summary will follow along — which is the intent. One that rebuilds the table by
create-copy-drop-rename must drop those triggers first and re-run the backfill
after, or it will double-count the copy. 32's `detections_stamp_utc` trigger has
the same property, and is guarded on `detected_at_utc IS NULL`, so a bulk rewrite
that carries the column forward will not re-stamp it.

`detected_at_utc` is deliberately **not** part of `idx_detections_unique`. The
backfilled value depends on the host's timezone, so a station that changed zones
between imports would find the same detection hashing to two different instants
and its history silently doubling — which is precisely what migration 23's
unique index exists to prevent.

### Fixed — the deployment surface

A third pass, asking what was still not field-ready and looking where the two
above had not: the supply chain, the failure modes nothing had ever provoked,
and the 2 715 lines of `install.sh`. Evidence and the observed-failing gates are
`docs/POST_0140_AUDIT.md` §4 (D14–D25).

- **"Could not verify" no longer takes the same branch as "verified".** Three
  places treated a missing integrity check as an acceptable degradation. The
  binary auto-updater logged *"integrity not verified (relying on the
  staged-binary smoke test)"* and installed anyway; the installer warned
  *"SHA256SUMS could not be downloaded — continuing without checksum
  verification"* and installed anyway; and the model checker returned success
  when `sha256sum` was absent — the same value a verified file returns.

  The `SHA256SUMS` request is the cheapest thing on the wire for an on-path
  attacker to drop, so whoever could substitute a binary also decided whether
  it would be checked. The fallback all three leaned on, `<binary> --version`,
  proves a file executes, not whose it is. All three now refuse, the updater
  before any network I/O and with the reason carried through to the operator.

- **The archive checksum now checks the archive.** Verification ran
  `sha256sum -c SHA256SUMS --ignore-missing`, which answers "did anything both
  listed *and present* mismatch?" — with the archive absent from `SHA256SUMS`
  and another listed file matching, that exits 0, and the installer printed
  "Checksum verified against SHA256SUMS".

- **A missing `openssl` no longer kills the installer.** The fallback admin-
  password generator piped `/dev/urandom` into `head -c 22`; the producer never
  ends, so it always took SIGPIPE, and under `set -euo pipefail` the installer
  exited — silently, with no output on any stream — at the step that secures
  `/admin`. Measured at 200 failures in 200 runs. The same shape appeared in
  eight other places, including two that reported the wrong answer rather than
  aborting; all are fixed and a lint keeps them out.

- **A station that fails to start now keeps trying.** The unit carried
  `StartLimitBurst=5` / `StartLimitIntervalSec=300`, so five restarts inside
  five minutes — under a minute at `RestartSec=10` — marked it failed and
  stopped it permanently, leaving an unattended box down until someone walked
  to it. Every self-clearing cause reached it: a late-mounting external data
  disk, a port the previous process still held. The rate limit is off, with
  10 s → 5 min backoff in its place and `RequiresMountsFor=` so the data
  filesystem is waited for.

- **Stopping ZRAM no longer disables the system's swap.** The generated
  `zram-swap.service` ran `swapoff -a` on stop — every swap on the machine, and
  Raspberry Pi OS enables `dphys-swapfile` by default. It also passed device
  paths to `rmmod`, which takes a module name, so the unload failed on every
  run behind a `|| true`.

- **A failed update leaves the station running.** The installer stopped the
  service before downloading the new binary, so any failure in between — now
  including a refusal to install an unverified download — took a working
  station off the air for a binary that was never installed.

- **macOS no longer writes coordinates of 0.0, 0.0.** Not "unset": Null Island,
  which the metadata model filters the species list for, and which the
  installer's own check reports as a configured location.

### Added — gates for the failure modes a field station meets

- **An unclean shutdown is now tested by causing one.** `tests/unclean_shutdown.rs`
  SIGKILLs a real process mid-insert and requires `species_summary` to
  reconstruct exactly — a torn rollup would drift every count on the dashboard
  a little further on each power cut, with both tables staying well-formed and
  no integrity check ever reporting it.

- **A full disk is now tested with a full disk.** `tests/out_of_space.rs` uses
  a real `ENOSPC` from the kernel, and covers the claim that a part-finished
  backup no longer survives to become the newest one recovery reaches for.

- **A backwards clock is now tested.** `tests/clock_steps_backwards.rs` covers
  the re-recorded window an NTP correction produces: the collision is reported
  rather than silently dropped, never overwrites the original observation, and
  never moves the rollup.

- **The installer's own tests run.** `installer/test/` held five test scripts
  that nothing executed — no workflow, no script, no Makefile. They run in CI
  now, and adding one without wiring it up is a red build.

## [0.14.0] - 2026-08-16

### Added

- **`--migration-report`: what an upgrade would do to your history, before it
  does it.** Most migrations only change the schema around the data. Migration
  24 (below) rewrites rows already on disk and destroys its own input —
  afterwards nothing records what a detection's timestamp used to be. This
  opens the database read-only and prints how many detections would move, how
  many are left alone and why, the largest shift, how many roll onto the next
  day, and the affected date range. It changes nothing, so it is safe to run on
  a live station.

- **Every history-rewriting migration is now preceded by a backup.** Before
  migration 24 runs, the database is copied to
  `<db>.pre-migration-24.backup` with `VACUUM INTO`, so recovery is a file
  move. Existing backups are never overwritten. A backup that cannot be written
  fails the migration rather than proceeding — the rewrite cannot be undone, so
  "could not make it recoverable" has to mean "did not do it". The error names
  the space required and the escape hatch, `BIRDNET_SKIP_MIGRATION_BACKUP=1`,
  for a station whose disk genuinely cannot hold a copy and whose operator
  accepts an unrecoverable rewrite.

- **`--channel-report`: what a stereo microphone is actually delivering.** The
  model has one audio input, so two channels must become one before inference —
  today by averaging, which is harmless for coincident capsules and a comb
  filter for spaced ones. Which case a station is in depends on its microphone
  and its acoustics, so it cannot be answered anywhere but on the station.

  The report records a few seconds from the configured ALSA device and prints
  each channel's level, the inter-channel delay (with the capsule spacing it
  implies), and what each reduction would hand BirdNET: today's average, the
  louder single channel, and a delay-aligned sum. It then recommends a setting.
  Requires the service to be stopped first — an ALSA capture device is
  exclusive — and says so when the device will not open.

### Fixed

- **A stereo microphone delivering one duplicated channel was reported as
  healthy.** `plughw:` satisfies a two-channel request from a one-channel
  device by copying the channel, and the copy scores perfectly on every measure
  `--channel-report` and `stereo-check.sh --alsa-test` take — correlation
  1.000, zero delay, averaging costs nothing. Both tools called that a
  well-matched coincident pair and told the operator there was nothing to fix,
  which is the opposite of the truth: the second capsule is not reaching the
  software at all.

  Both now check whether the channels are bit-identical before anything else.
  Two capsules never agree sample for sample — each carries its own self-noise
  — so exact equality means one channel copied. Both also point at
  `arecord -D hw:N,M --dump-hw-params`, which asks the hardware with the plug
  layer out of the path; `stereo-check.sh` runs it up front and says plainly
  when the device reports one channel.

- **`--channel-report` discarded `arecord`'s diagnosis.** `Channels count non
  available` (the device is not stereo) and `Device or resource busy` (stop the
  station first) are opposite problems, and both rendered as the same generic
  guess. `arecord`'s own words are now shown first. Its stderr was also piped
  and never read, which would deadlock the report if `arecord` ever filled a
  pipe buffer.

- **The "delay-aligned sum" row was never a sum.** It averages — the measured
  ratio for two aligned identical channels is 1.0, not 2.0. The label, the
  field name and the documentation all said otherwise. Averaging is the right
  behaviour, since summing would add a constant 6 dB that reads as recovered
  signal and is not, so the report now names what it does.

- **A mutation-testing gate had been passing without testing anything.** The
  `sqlite/queries/detections.rs` matrix row kept naming a file that 0.7.2 split
  into a directory, so the job matched no source, produced no mutants, and the
  threshold step read the empty result as "0 missed". A run that generates no
  mutants now fails outright on pushes, cron and manual dispatch, so the next
  stale path announces itself instead of going quietly green. Pull requests are
  exempt, where `--in-diff` makes an empty result legitimate.
  `crates/birdnet-db/src/migration.rs` also joins the matrix.

- **A detection's timestamp is now when it was heard, not when its recording
  started.** A 15-second segment is five 3-second chunks, and all five were
  stamped with the file's start second. `chunk_offset_secs` held the difference
  and the detections API does not return it, so one continuous song produced
  five rows identical in every displayed field — which is exactly what "repeated
  detections" looked like. It also put five *simultaneous* detections into
  `detection_timestamp`, which sessionisation, gap analysis and the dawn-chorus
  curve all group on.

  BirdNET-Pi has always added the offset, in the same place (`Detection.__init__`:
  `file_date + timedelta(seconds=self.start)`), so this table has been holding
  two conventions at once: imported BirdNET-Pi rows with chunk-accurate times,
  natively recorded rows without. The pipeline now adds the offset at inference,
  rolling the date when a chunk crosses midnight, and **migration 24 repairs
  history already on disk** from the stored offsets — so the whole table ends on
  one convention. Rows whose `Date`/`Time` name no point in time are left
  untouched rather than turned into an invented timestamp.

  Row *counts* do not change, and were never wrong: BirdNET-Pi has no UNIQUE
  constraint on `detections` at all and stores one row per chunk exactly as this
  does.

- **The Audio page's Left and Right channel options did nothing.** Both
  collapsed to `channels: 1` at the capture source and were never distinguished
  again, so all three of Mono, Left and Right produced byte-identical captures.
  They now select the channel they name: the device is opened with both, and the
  capture tee keeps the requested half, so the segments written to disk are
  single-channel and nothing downstream needs to know a choice was made.

  This matters because of what `Stereo` does. Both channels are kept and the
  decoder averages them to the mono BirdNET requires — which for a **spaced**
  pair is a comb filter, not a noise reduction. Measured through this project's
  own decode path: one wavefront reaching the capsules half a period apart loses
  about 66 dB to cancellation, a quarter period costs 3 dB, and the notches move
  with the bird's direction. A coincident pair is unaffected. Selecting a
  channel is the mitigation, and it was the one thing the UI offered that had
  never been wired up.

  Not a regression: BirdNET-Pi defaults to `CHANNELS=2` and uses
  `librosa.load(mono=True)`, which averages identically. A stereo source now
  says so on the Audio page and warns once in the journal at start-up.

- **The analytics dashboards were blank, and nothing anywhere said why.** Two
  independent defects, both invisible to a green CI matrix, both only reachable
  on a real station.

  The first is the one that emptied them, and it emptied them **permanently**:
  a station reported dashboards blank for days. Every analytics query filters on
  a look-back window, which reaches DuckDB as
  `detection_date >= CURRENT_DATE - INTERVAL n DAYS`, and `CURRENT_DATE` lives
  in DuckDB's ICU extension — as does every other way to name the current local
  date: `today()`, the `TimeZone` setting, and even `CAST(now() AS DATE)`, which
  fails with `Unimplemented type for cast (TIMESTAMP WITH TIME ZONE -> DATE)`.
  There is no ICU-free spelling to fall back to.

  ICU is **not** statically linked into the `libduckdb` that `duckdb-rs`
  bundles. It reports itself `installed` on a connection that has already
  autoinstalled it, which is what an earlier reading of this — and the first
  version of the fix — was built on. Measured properly, with autoload and
  autoinstall off and no local cache, `duckdb_extensions()` reports `icu` as
  `installed=false, NOT_INSTALLED`, and `LOAD icu` fails outright.
  (`core_functions`, by contrast, genuinely does report `STATICALLY_LINKED`,
  which is why `strftime` and `date_diff` kept working throughout.)

  So DuckDB has to fetch it, and it does that by autoinstalling into
  `$HOME/.duckdb`. The shipped systemd unit sets `ProtectHome=read-only`. The
  station's journal:

  ```text
  Failed to create directory "/home/pi/.duckdb": Read-only file system
  ```

  Every analytics query failed from then on, and the store's `birds.duckdb`
  never appeared. Two things attempt that write — ICU autoinstalling, and stage
  2 of the behavioral loader (`INSTALL behavioral FROM community`) — so both are
  fixed at the source: **DuckDB's extension directory now sits beside the
  analytics database**, inside `DATA_DIR` and therefore inside the unit's
  `ReadWritePaths`, instead of under `$HOME`.

  On top of that, **the ICU binary is now embedded in the release binary** the
  same way the `behavioral` extension already was, and loaded from it at open.
  That removes the network *and* the writable `$HOME` from the path entirely, so
  an air-gapped station gets correct local dates on its first query. Release,
  CI and Docker builds all fetch it per target; `build.rs` refuses to embed
  bytes whose footer it cannot parse, and now also refuses bytes built for a
  different platform than the one being compiled for — cargo does not tell a
  build script which DuckDB version will be linked, but it does tell it the
  target triple, and 20 MB of unloadable ICU is worth catching at build time.

  There was a timing bug underneath all of that too, and it is still fixed:
  even where the autoinstall *could* write, DuckDB resolves ICU while binding
  the query that first needs it, too late for that query. Attempt 1 failed,
  attempts 2–4 passed. One failed query per restart would have been survivable,
  except the web layer maps a query error to a rendered "Analytics temporarily
  unavailable" fragment and caches that fragment for ten minutes — so the first
  page visit after every restart poisoned the cache. ICU is loaded when the
  store opens, before any query runs.

  The test that was supposed to cover this went green against the broken
  implementation, because an earlier probe on the same machine had populated
  `~/.duckdb`; moving that cache aside was what exposed it. Its replacement
  turns both escapes off explicitly — autoload and autoinstall disabled,
  extension directory pointed at an empty one — so the embedded bytes are the
  only route `CURRENT_DATE` has, and a separate gate pins the extension
  directory to the data directory. Verified against the previous code, where
  the first fails with `Catalog Error: … "current_date" is not in the catalog`
  and the second sees DuckDB's default (an empty string).

  The time-series execution gate had caught the same disease from the same
  cache. It opened a bare DuckDB connection and issued `LOAD icu` itself, as an
  approximation of what the application does — and a bare `LOAD` never
  autoinstalls (DuckDB only does that while binding a query that needs the
  extension), so it passed only when *some other test binary in the same run*
  had populated `~/.duckdb` first. It now opens a real `AnalyticsDb`, which is
  literally what `birdnet-web` hands these queries, and drops its private copy
  of the `detections_ts` view along with it.

  The second survives dirty history rather than a cold start. `Date` and `Time`
  are free-form `TEXT NOT NULL` — the column type forbids NULL, not nonsense —
  and the BirdNET-Pi importer turns a NULL `Date` into `""` and copies
  malformed values through verbatim. `detections_ts` cast them with a plain
  `CAST`, and DuckDB raises `Conversion Error` for the *whole query*, so one
  unplaceable row anywhere in a multi-year import took down every behavioural
  and time-series dashboard at once. The view now uses `TRY_CAST`: such a row
  falls out of the time-bucketed results instead of aborting them. Coercing to
  an epoch default was rejected — it would invent detections on 1970-01-01.

  Neither could have been caught by the tests that existed. The time-series
  crate's sixteen public queries had no execution coverage at all: every test
  built a SQL string and asserted it *contained* the right substrings, which a
  query DuckDB refuses to bind passes exactly as well as one that works. There
  is now a gate that executes all sixteen against a real DuckDB and requires
  rows back, plus gates for the cold-start bind and the unplaceable row.

- **Ten of the eleven `phenology` query builders emitted SQL DuckDB refuses to
  run.** `birdnet_behavioral::phenology` is a public API documenting a
  SQLite/DuckDB compatibility matrix, but it emitted `strftime('%Y', Date)` —
  SQLite's `strftime(format, value)` argument order — against DuckDB, which
  takes `strftime(value, format)`. Every query using it failed to bind with
  "Could not choose a best candidate function". `phenology_timing_sql` also used
  `julianday`, which DuckDB does not have, and two builders assembled their
  `WHERE` clause by giving each condition its own `WHERE `/`AND ` prefix, so an
  absent species filter left a dangling `AND` straight after `FROM` — a parser
  error.

  The builders now emit DuckDB SQL, read `detections_ts` so `detection_date`
  arrives typed (and unplaceable rows are excluded rather than grouped under a
  NULL year), and assemble the `WHERE` clause from a list of conditions, which
  makes the dangling-`AND` shape unrepresentable. The compatibility matrix has
  been replaced with the truth: these target DuckDB.

  No dashboard was affected — nothing calls these, and the web phenology card is
  SQLite-backed — but the tests asserted only on generated *text*
  (`sql.contains("month")`), which a query no engine will run passes just as
  well as one that works. `tests/phenology_execute.rs` now executes all eleven
  against a real store; it fails on ten of them against the previous code.

- **The embedded-extension check ignored the platform.** A DuckDB extension is
  locked to a platform as well as a version, and the two fail identically at
  `LOAD`, but `embedded_extension_mismatch()` compared only the version — so
  `linux_amd64` bytes embedded in an `aarch64` build agreed on `v1.5.5`, passed
  the check, and then failed to load on the Pi with nothing having warned. Both
  properties are now compared (the engine's own platform comes from
  `pragma_platform()`, which uses the same identifiers the extension registry
  publishes under) and the report names which one disagrees. A platform that
  cannot be read on either side is not treated as a mismatch, so missing
  information cannot manufacture a false alarm. `release.yml` already selected
  the extension per target, so this gap was reachable from local and cross
  builds — which is exactly what a maintainer tests an air-gapped station with.

- **`scripts/hardening-check.sh` could bind-mount over the host as root.** The
  script re-execs itself under `unshare -rm` and carries a guard meant to abort
  if that did not happen, because everything after it bind-mounts over `$HOME`,
  `/usr` and `/tmp` and then deletes its working directory on exit. The guard
  compared the caller's mount namespace against PID 1's, and refused only when
  the two were *equal*. `/proc/1/ns/mnt` is unreadable to a process whose PID 1
  is a sandbox supervisor rather than real init — ordinary in CI containers and
  nested sandboxes — and `readlink` then yields the empty string, which never
  equals a real namespace id. The guard therefore failed **open** on precisely
  the environments it existed to protect: measured in one such container, it
  returned "proceed" in all four cases tested, including the host mount
  namespace as root. It is now a token handed down by the re-exec — the parent
  records its own namespace and the child refuses unless it is demonstrably in
  a different one — so anything that cannot be positively confirmed is a
  refusal. This only ever affected maintainers running the script; it is not
  installed on a station.

### Added

- `GET /api/v2/analytics/status` reports the analytics **store**, not just the
  build flags. `analytics_compiled` and `analytics_configured` are both `true`
  on a station whose dashboards are empty — they describe intent, and stay true
  through every way this actually fails. The new `store` object carries
  `extension_loaded`, the DuckDB row count, `unplaceable_detections` (rows no
  dashboard can place in time), the engine's own DuckDB version and platform,
  and the embedded extension's version, platform and any mismatch — including
  which property disagrees. It is `null` on a slim build, so "no analytics here"
  stays distinguishable from "analytics present but broken".

### Changed

- BirdNET-Pi import validation no longer claims malformed rows "will be
  skipped". Nothing skipped them: they were imported, counted, and then absent
  from every date- or time-based analytic. The check also missed the cases that
  mattered — it sampled only the first 1 000 rows, never looked at `Time`, and
  could not see a NULL `Date` at all, because `NULL NOT GLOB …` is NULL rather
  than true. It now scans the whole table, inspects both columns, and says what
  actually happens to the rows.

- `scripts/setup-onnxruntime.sh` works against current `ort-sys` again. Its dist
  table was renamed `dist.txt` → `dist.tsv` and had its columns reordered with a
  header added, so the script failed with "ort-sys not found" and cold builds
  behind a TLS-intercepting proxy — sandboxed CI, Claude Code on the web — could
  not fetch ONNX Runtime. It now accepts either filename and identifies columns
  by content rather than position.

## [0.13.1] - 2026-08-13

### Fixed

- **A re-imported BirdNET-Pi database silently doubled itself.** Every
  duplicate-suppression path rests on `idx_detections_unique`, and `File_Name`
  is part of that key and nullable — and SQLite considers NULLs distinct in a
  UNIQUE index. A row with no filename conflicted with nothing, so
  `INSERT OR IGNORE` ignored nothing.

  The CSV/TSV path made it easy to hit: an empty `File_Name` field, `\N`, the
  literal `NULL`, or a row with fewer than twelve columns all yield SQL NULL.
  Re-importing the same export doubled those rows and reported "imported N,
  skipped 0" as success. Anyone who re-ran an import after a failure — the only
  recovery available, since batches commit as they go — doubled their history
  and had every dashboard, rate and analytic computed over it.

  Migration 23 makes the key NULL-insensitive via `COALESCE(File_Name, '')`,
  and **repairs databases that already carry duplicates** by collapsing each
  group to its earliest row. `File_Name` itself stays nullable, because NULL is
  meaningful there — it distinguishes "never had a clip" from "reclaimed"
  (migration 22), and `locks.rs` filters on it.

  Regression tests now cover the SQLite path, the CSV path (using the shipped
  fixture's own rows, which are exactly the NULL-`File_Name` kind), the
  migration's repair of pre-existing duplicates, and — new — the operator's
  actual HTTP journey: upload, poll progress to completion, upload again, and
  assert the row count did not move. Verified against the pre-fix index, where
  it fails with 6 rows where 4 were expected.

  Found while auditing the weekly report: its fixture seeded two detections of
  one species at the same second with no clip, which the corrected key rightly
  calls one detection. That is inflation of exactly the kind this bug produced,
  living in a test.

- **Listen → Live appeared to do nothing, because the button cancelled the
  stream you were waiting for.** `audio.play()` sets `paused` to false
  synchronously but resolves only once the browser has buffered enough to start
  — around a second, since ffmpeg must fill a frame before the first MP3 bytes
  leave the station. The button kept reading "Listen (audio)" for that whole
  window, so the natural response to apparent silence — clicking again — landed
  in the stop branch and killed the stream that was about to start. Clicking
  through that cycle is indistinguishable from live audio being broken, and that
  is how it was reported on 0.13.0.

  The button now shows **Connecting…** and ignores clicks until `play()`
  settles, and an `error` on the element reports "Stream unavailable — retry"
  rather than stranding it mid-connect. `-flush_packets 1` on the encoder halves
  time-to-first-audio (measured through the shipped invocation: 1.13 s → 0.59 s),
  shrinking the window in which the trap could spring at all.

  Nothing was wrong on the server: the tap, the source resolution, the segment
  writer and the MP3 encoding were all delivering correctly throughout.

- **A failed live stream now says why.** ffmpeg's stderr was sent to
  `/dev/null`, so every failure — `Device or resource busy`, an unknown filter,
  a missing codec — reached the operator identically: a `200` response carrying
  no audio and an empty journal. It now runs with `-loglevel error` and its
  stderr is drained to the log, and a stream that ends having delivered zero
  bytes says so. Diagnosing the bug above took three refuted hypotheses for want
  of this one log line.

- **The same trap in both clip players.** The detail-page player swapped to its
  pause icon before `play()` resolved and never handled a rejection, so a clip
  that could not start (autoplay policy, decode error, a clip deleted under it)
  showed a pause icon over silence with an unhandled promise rejection. The
  Recordings row player had the cancelling variant: a second click on a clip
  still loading paused the clip being waited for. Both windows are short for a
  local file — but not zero, and a cold cache or a busy Pi widens them.

- **Listen → Live could strand itself on "Connecting…".** A stream that connects
  but never buffers enough to start fires `stalled`, not `error`, so `play()`
  can stay pending indefinitely — and ignoring clicks while that is true would
  have left the button permanently dead. It now gives up after 20 s and hands
  control back.

- **Bulk clip actions could fire twice and could report success after failing.**
  The lock/delete batch had no in-flight guard, so a second click during a slow
  batch re-sent the whole thing; and because `fetch` resolves for 4xx/5xx, a
  batch that failed outright still reloaded the page as if it had worked. The
  batch is now single-flight and checks each response, reporting how many clips
  could not be updated.

- **Two concurrent restores can no longer run over the live database.** A
  restore unpacks an archive over `birds.db` and the recordings directory, takes
  minutes, and shows nothing while it runs — the same conditions that get a
  button clicked twice. htmx does not dedupe in-flight requests unless told to,
  and nothing on the server refused the second one. The endpoint now rejects a
  concurrent restore outright (a UI guard cannot bind a client that simply POSTs
  twice), and the form and the destructive "clear" controls disable themselves
  while their request is in flight.

### Added

- **An interaction gate in CI** (`tools/visual-qa/interactions.mjs`). Every bug
  above is one the existing suite could not have caught: the server was correct,
  the pages rendered, axe was clean and every screenshot looked right — the
  defects lived entirely in what the *second* click did, and nothing anywhere
  drove a control twice. The gate drives controls the way an impatient operator
  does and asserts they neither cancel nor duplicate their own in-flight work.
  Verified against the shipped 0.13.0 build, where it reproduces the reported
  bug (`pause()` during connect) and catches the bulk batch firing twice.

  The visual-QA fixture no longer rate-limits itself. It is deliberately
  hammered — 152 page captures back to back, plus the new gate driving controls
  as fast as Chromium will go — and the station's 30 req/s limiter throttled the
  harness rather than the product, surfacing as an intermittent `429` on a font
  and a red build. **The station's own limiter is unchanged**: measured, a cold
  dashboard load is 24 requests, the heaviest page 34, and two rapid loads 48 —
  all inside the 60-burst default with no `429`, so there was nothing to loosen
  for real clients. A test now pins that the shipped router keeps the strict
  default, since the opt-out is what makes losing it possible.

### Changed

- `/stream` no longer sets `Transfer-Encoding` by hand. It is a hop-by-hop
  framing header the HTTP layer owns: hyper already chunks a streaming body and
  emits the header itself — verified on the wire, where setting it changed
  nothing but header order — and HTTP/2 forbids it, so a station behind an h2
  reverse proxy would have the response rejected for carrying it.

## [0.13.0] - 2026-08-13

### Changed

- **Live audio now comes from capture itself instead of a second microphone
  open, so it works on a single-microphone station at all.** An ALSA `plughw:`
  device is exclusive: on the Raspberry Pi 4 under test,
  `ffmpeg -f alsa -i plughw:CARD=PRO,DEV=0` returns `Device or resource busy`
  for as long as `arecord` is recording — which, on a station doing its job, is
  always. `GET /stream` did exactly that second open, so Listen → Live could
  never play on the commonest build there is.

  `arecord` no longer segments for us. It streams raw PCM into the process and a
  reader thread drives two consumers: the rotating WAV writer that used to be
  `arecord --max-file-time --use-strftime`, and a bounded live tap that
  `/stream` subscribes to. The tap is **lossy on overflow** and never blocks, so
  a stalled listener cannot backpressure recording — losing live-monitoring
  audio is a click in someone's headphones; losing recorded audio is a detection
  that never happens. Filenames are byte-identical to the ones `arecord`
  produced, including their **local** civil time, which the supervisor now
  refreshes every tick so a station keeps naming files correctly across a
  daylight-saving change it never restarts for.

  `/stream` for a source that is not recording — paused by the schedule or by a
  quiet window, or down — now answers `503` with that explanation, instead of
  holding a connection open producing nothing.

  RTSP and PipeWire sources are unchanged: a second RTSP session is normal and
  PulseAudio permits concurrent opens, so neither has the problem this solves.
  macOS microphone capture is also unchanged (ffmpeg/avfoundation), because
  there is no macOS runner in CI and no macOS hardware behind this change.

- **Per-source capture gain no longer needs ffmpeg, and no longer lies about
  it.** `arecord` has no gain control, so a gain-configured microphone used to
  be captured by `ffmpeg -f alsa` and its `volume` filter — but
  `required_tool()` still reported `arecord` for that source, so a station with
  gain set and no ffmpeg installed passed the availability check and then failed
  to spawn. The gain is now applied to the samples in-process (clipping, as the
  ffmpeg filter did) and that second capture backend is gone.

- **"Station" in the navigation is now "Settings"**, and its inner Settings tab
  is "General". The section is what operators go looking for when they want to
  configure the station; `/station` URLs are unchanged and "station" remains a
  command-palette keyword.

- **Live spectrogram frames now carry a `source`.** The broadcast sends every
  source's frames to every client and they previously carried no attribution, so
  the Listen source picker could not filter and a multi-source station drew both
  inputs into one spectrogram.

- **`LabelSet` retains the `class` column** from the BirdNET+ V3.0 CSV, so
  non-bird taxa (the model is a 11K global classifier, not birds-only) can be
  distinguished from birds rather than appearing as a scientific name with no
  common name.

### Fixed

- **The dashboard's day strip drew "now" and sunrise/sunset on a UTC axis while
  its bars were local.** Detections are timestamped by `arecord --use-strftime`,
  which is local, and `hourly_activity` buckets that `Time` column — but the
  marker came from a raw `epoch % 86400` and the solar times from
  `sunrise_utc_min`. On a CEST station the marker sat two hours behind the
  detections beside it and the hero pills read "sunrise 4:10" for an 06:10
  sunrise. `today_date_string()` was UTC for the same reason, so for the first
  hours of each local day the Today page queried the wrong date entirely.
  The offset now comes from SQLite's `localtime` (no date/time crate in the
  workspace, and `unsafe` is forbidden), cached for a minute.

- **The setup wizard could not display any setting the station already had, and
  silently overwrote two of them.** Latitude and longitude had no `value=`
  attribute and the confidence/notification fields were hardcoded in the markup,
  so a station configured at install time rendered a blank wizard. Because the
  hardcoded fields are never empty they slipped past `onboarding_save`'s
  skip-if-blank guard and were written on every completion: an operator who had
  set `CONFIDENCE=0.6` had it reset to 0.75 by clicking through setup.

- **The installer discarded typed coordinates without saying so.** The prompt
  told the operator to read coordinates off OpenStreetMap — which hands over a
  *pair*, `49.4521, 8.6724` — then offered a single-value field whose validator
  rejected exactly that, warned once, and continued. A decimal comma
  (`49,4521`) was rejected too, though the web settings form accepts it. The
  prompt now parses both shapes and re-prompts on bad input, like the
  audio-source prompt above it.

- **"first today" was shown on every detection of a species, not the first
  one.** The badge compared a species' first-ever *date* to today, which is true
  of all of that day's detections — a station that heard 133 blackcaps on their
  arrival day badged all 133. Now keyed on the first-ever instant, so exactly
  one detection can carry it, and renamed "first ever" since only one row can
  hold it.

- **The live spectrogram decoded every clip while it was still recording.** The
  producer decoded on the watcher's create event after a fixed `sleep(100ms)`,
  against segments `arecord` writes for fifteen seconds — so every frame failed
  with "unexpected end of file" and the dashboard showed "idle" on a healthy
  station. The detection daemon already had the right rule and its own docs
  named this exact error; it was private to that module, so it is now shared in
  `crate::file_settle`.

- **Live audio needed ffmpeg that no microphone station ever installed.**
  `GET /stream` shells out to ffmpeg for every source kind including plain ALSA,
  but the installer ensured it only for RTSP capture and `--doctor`'s check was
  gated on the same condition — so a Linux station with a USB microphone
  returned 500 on every request while reporting itself entirely healthy.

- **The browser tab had no icon.** The PWA manifest and `apple-touch-icon` were
  present, but with no `rel="icon"` the browser fell back to `/favicon.ico`,
  which is not routed.

## [0.12.0] - 2026-08-10

### Fixed

- **`RECORDING_SCHEDULE` in `birdnet.conf` was ignored: a station set to
  `solar` recorded around the clock.** `capture::schedule` read
  `cli.recording_schedule` directly, and that flag carries a clap
  `default_value` of `all-day` — so the default always won and the configured
  schedule never applied. A `fixed:HH:MM-HH:MM` window was dropped just as
  silently.

  Nothing contradicted it. `birdnet_core::config::validate` validates the key,
  and `--doctor`'s clock check reads it from the config to warn that a fixed
  window is evaluated in UTC — so the diagnostic reported on a schedule the
  runtime never used, the same shape as the `CADDY_PWD` and `ALSA_CARD` splits
  fixed earlier in this release. Its sibling `resolve_twilight_offsets` had
  always gone through `resolve::setting`; this one line had not, and every
  existing test set the CLI field by hand, exercising only the path that
  worked.

  Measured before the fix: `RECORDING_SCHEDULE=solar` yielded
  `night_inhibit=false, fixed_window=None` — 24/7 recording, on a station whose
  operator had asked for the dawn window and whose disk and CPU paid for it.

### Removed

- **`--quality-filter` and `--quality-min-snr`, which did nothing at all.**
  They promised that "audio chunks are assessed for SNR, spectral flatness, and
  rain/wind interference before being passed to the ML model". No code read
  either field — not from the config, not from the CLI. The feature was
  advertised in `--help`, in the generated CLI reference and in the tuning
  guide, and was inert.

  The implementation is not missing: `birdnet_core::audio::quality` is ~1300
  lines of SNR, spectral flatness, rain/wind assessment and noise-floor
  tracking, with benchmarks — it was simply never called by the detection
  pipeline. Wiring it changes which chunks reach inference, so it belongs in
  its own change with hardware validation behind it rather than a release-prep
  pass. The flags are gone until then, because a switch that silently does
  nothing is worse than no switch: an operator in a noisy garden would set it
  and believe their false positives were being filtered.

### Added

- **Four settings that were command-line-only are now on the settings page.**
  An operator without a terminal — which is most of them — could not reach any
  of these:

  | Setting | Why it matters |
  |---|---|
  | **Recording window** (`RECORDING_SCHEDULE`) | all-day / solar / fixed hours; the page offered the sunrise and sunset *offsets* while the mode they modify was unreachable |
  | **Heartbeat URL** (`HEARTBEAT_URL`) | lets an outside monitor alert you when the station stops reporting |
  | **Dead-man alert** (`DEADMAN_HOURS`) | notifies you after N hours of silence — the symptom of a microphone that died quietly |
  | **Common-name language** (`DATABASE_LANG`) | a non-English station could not pick its own language from the UI |

  Each goes through the existing wiring guard, and a test walks the whole chain
  per key — the settings row the form writes, through the overlay, to the
  config key the consumer actually reads — with a further test proving that
  choosing *Solar* on the settings page really does stop overnight recording.
  Both fail against the pre-fix code.

  MQTT and Home Assistant discovery (8 flags) remain command-line-only and are
  deliberately deferred; `docs/RELEASE_PLAN.md` § 5 records the rest of the
  audit.

### Changed

- **The hardware harness now measures CPU, and checks the dashboard's CPU
  figure against the kernel's.** Reported as looking broken on a Pi. It could
  not be reproduced: measured against `/proc/stat` over the same window the
  reading agrees exactly — 2 % against 2 % idle, 100 % against 100 % with every
  core pinned. But the report pointed at a real gap. `scripts/hardware-test.sh`
  recorded load average and never a utilisation figure, so no run on real
  hardware had ever established that the CPU monitor worked at all; and the
  unit tests only asserted `0.0 ≤ cpu ≤ 100.0`, which a sampler stuck at zero
  satisfies.

  The `perf` phase now samples CPU utilisation into `perf-samples.csv`, reports
  mean and peak, warns when the peak leaves no headroom, and compares the
  figure the Station page displays with `/proc/stat` — failing outright if the
  dashboard shows 0 % on a busy board. A unit test now pins the machine's cores
  and requires the reading to move, which the old range assertions could not.

- **The out-of-the-box minimum confidence is now 0.75** (was 0.70, BirdNET-Pi's
  default). High enough that a new station's log reads as realistic instead of
  padded with marginal IDs, low enough that quiet and distant birds are still
  recorded. It remains a single shared constant, so the daemon, the settings
  form and the wizard cannot disagree about it; existing stations with an
  explicit `CONFIDENCE` are unaffected.

### Added

- **The setup wizard now asks how picky the station should be.** The minimum
  confidence decides whether anything is recorded at all, and nothing in the
  setup path mentioned it: the installer wrote it as a commented-out line and
  the wizard never raised it, so an operator who wanted stricter or looser
  detection had to find Settings → Detection unprompted. A new **Accuracy** step
  offers four presets (0.90 / 0.75 / 0.60 / 0.40) pre-selected on the shared
  default, so clicking straight through yields exactly what the daemon would
  have enforced anyway.

  The submitted value is range-checked before it is stored. An out-of-range
  `CONFIDENCE` is a *fatal* doctor error, and `--doctor` runs from the unit's
  `ExecStartPre` where exit 2 blocks startup — so an unvalidated write here
  would have turned the setup form into a way to leave the station unable to
  start.

### Fixed

- **The setup wizard showed a station that did not exist.** Its Microphone step
  was a mock-up: a hard-coded "UMC202HD · USB audio · card 1 · 48 kHz" card,
  marked *recommended* and pre-selected, described as "detected automatically";
  a "Built-in microphone · card 0 · 44.1 kHz"; and two more cards offering an
  RTSP camera and folder-watching that did nothing when clicked. The final
  summary card was the same — "Boston, MA · 42.36, −71.06", the same UMC202HD,
  and a dashboard address of `http://birdnet.local/` that does not resolve on
  every network.

  None of it was read from the station. A first-run operator was shown hardware
  they do not own, presented as already found — so on a station whose
  microphone was missing or misconfigured, the wizard's answer to *"will this
  hear anything?"* was a confident yes about a device that is not there. That
  is the failure mode the wizard exists to prevent.

  The Microphone step now renders the real rows from `audio_sources`, reusing
  the Capture tab's own `kind_label`/`detail_for` rather than a second copy that
  could drift, and a station with no source is told plainly that nothing will be
  detected and pointed at where to add one. The summary rows that depend on
  operator input are placeholders the page script fills — location from the
  coordinates actually entered, alerts and confidence from the cards actually
  chosen, and the dashboard address from the URL the operator actually reached
  the page on. Verified by driving the wizard end to end in a real browser, and
  a test pins every one of the removed mock strings so none can reappear.

  Two counts went stale when the Accuracy step was added and nothing would have
  caught either: the welcome copy still read "five steps", and
  `tools/visual-qa/onboarding.mjs` looped to a hard-coded `step <= 5`, so its
  screenshot set silently stopped one short — looking complete while missing
  exactly the new step worth reviewing. The prose count is now asserted by a
  test and the capture script reads the count from the page. Re-audited with
  axe-core across all six steps (stricter than the CI gate, which only ever sees
  the visible first step): no WCAG 2.1 A/AA violations outside the two rules the
  gate defers by design, and no horizontal overflow at 390 px in either theme.

- **Green ticks and a green "Healthy" badge on a station that was not working.**
  Walking the first-run journey end to end turned up four places that reported
  success without checking anything:

  * The dashboard's **"Getting ready"** card — the one thing a brand-new
    operator reads — ticked *Microphone detected* as soon as a source existed in
    the database, which says nothing about audio flowing. A source whose device
    vanished on reboot, or whose `arecord` had died, ticked green. It now reads
    the supervisor's own per-source gauge (the signal the Capture tab already
    used) and reports *Microphone not recording* with a link to the page that
    can fix it.
  * The same card's **"Room to record"** row was a hard-coded `✓`. The
    percentage and the wording were real, so it could render "Room to record ✓ —
    nearly full — 97% used": a pass tick on a station about to stop recording.
  * **"Model loaded … ready"** asserted runtime state the page has no signal
    for. It now says only what is true — the model ships with the app.
  * The **"recording"** pill is driven by time since the last detection, which
    is `None` on a station that has never detected anything — so it rendered a
    confident green *recording* forever on exactly the first-run station whose
    microphone never worked. It now consults the capture gauge first.

  The **header health badge** was the same problem at the top of every page:
  "Healthy" meant nothing more than "SQLite is not corrupt", so a station with a
  dead microphone and a 99 %-full disk showed green on every screen. It now
  grades database, capture and disk — the three things that stop detections —
  and names the problem (*Mic down*, *No microphone*, *Disk full*) with the
  reason on hover. The `data-health` token keeps its `ok`/`warn`/`err`
  vocabulary, and the disk threshold is shared with the dashboard so the two
  surfaces cannot disagree about the same disk.

- **The setup wizard's alerts choice governed nothing.** The Alerts step wrote
  `notification_mode` — a key no code anywhere read. An operator picked "Quiet"
  or "Everything" on their first day and it changed nothing, because the
  notification filter reads `notify_trigger` (bridged onto `APPRISE_TRIGGER`).
  Worse, its four options (`quiet`/`rare`/`daily`/`everything`) matched none of
  the three values the runtime understands, and `TriggerMode::parse` maps
  anything unrecognised to *every detection* — the chattiest mode, the opposite
  of a quiet choice.

  The step now offers exactly the three real modes, writes the key the runtime
  reads, and rejects anything else rather than silently selecting "chatty". It
  also says plainly that nothing is sent until a channel is configured, and
  links to where — replacing a "Pick channels now" disclosure that opened
  twelve non-interactive pills.

  The guard that exists to prevent exactly this (`SETTING_SPECS` must classify
  every settings key, enforced by a test) only ever covered the admin *form*, so
  the wizard wrote outside it. It now covers the wizard's keys too, and a test
  pins the declared list against what a full submit actually persists.

- **The timezone the wizard detected was stored and never used.** It cannot be
  applied from the app — the timezone is a system setting and the service does
  not run as root — but it is not cosmetic either: capture names each recording
  from the system's local time, and those filenames become every detection's
  `Date` and `Time`. A Pi left on UTC in a UTC+2 country files its dawn chorus
  two hours early, rolls "today" over at the wrong moment, and deletes by the
  wrong day. Raspberry Pi OS images default to UTC, so this is a common state.
  `--doctor` now compares the host's timezone with the detected one and hands
  over the exact `timedatectl set-timezone` command. Verified on a real
  container: a station configured for `Europe/Berlin` on a `Etc/UTC` host warns
  with that command.

- **`--doctor` was silent about a confidence threshold that guarantees a
  false-positive firehose.** Validation rejected the percentage mistake
  (`CONFIDENCE=70`) and non-numeric junk as errors, but a *decimal* slip — `0.07`
  for `0.7`, or a `0` copied from `SF_THRESH`, where `0` does mean "disabled" —
  parses, sits inside 0–1, and passed clean. The station then records the
  model's best guess for every three-second window: the disk fills, the species
  list fills with noise, and nothing anywhere says why. Verified against a live
  binary before and after; `0`, `0.001` and `0.07` each now warn while `0.1` and
  above stay silent, and the value remains usable rather than blocking startup.

- **`ModelConfig::default()` carried a third confidence threshold.** It
  hard-coded `0.25` — contradicting both the daemon's enforced default and the
  value the admin form advertises, which is precisely the drift the shared
  constant exists to prevent. Nothing shipped broken, because the daemon always
  names the field explicitly, but any future construction that spread
  `..ModelConfig::default()` without it would have silently reopened the exact
  bug. It now references the shared constant.

- **A station with no coordinates silently disabled species filtering.**
  `SpeciesFilter::filter_species` takes `Option<(lat, lon)>`; with `None` the
  metadata model cannot run, so occurrence filtering is skipped and **every one
  of the ~11 000 species stays a candidate**. The station keeps working and
  reports birds that have never occurred within a thousand miles — which reads
  as a bad model rather than as a missing setting.

  Nothing said so. The config validator checked that a latitude was *in range*,
  and warned when one of the pair was set without the other, but was silent
  when both were absent. `--doctor` now reports it, naming the consequence
  rather than the missing key, and pointing at the dashboard's location detect.

  Resolution goes through `daemon::resolve_station_coords` — the same function
  the detection daemon uses — rather than a third copy of the precedence rule,
  and falls back to the `settings` table because `--doctor` runs from
  `ExecStartPre` before the settings overlay has merged `/admin/settings` into
  the config. Reading the config alone would have warned at exactly the
  operators who configured their station the easy way, through the onboarding
  wizard.

  The installer was the other half of the same silence. Its summary warned
  loudly about a missing audio source and said nothing about missing
  coordinates, and its next-steps list called them "(Optional)" — while the
  location prompt itself is skipped entirely on a non-interactive install
  (`BIRDNET_NONINTERACTIVE=1`, or no TTY under `curl | sudo bash`) and on every
  re-install over an existing config, making "no coordinates" the common state
  rather than the rare one. It now says so, in the same place and tone as the
  audio-source notice.

Found by running the new on-device acceptance harness
(`scripts/hardware-test.sh`) against a Raspberry Pi 4 on Pi OS Trixie — the
"real Raspberry Pi hardware" gap `docs/RELEASE_PLAN.md` § 5 had carried open for
three releases — except where a bullet says otherwise. None was reachable from
CI: each needs a real systemd unit, a real USB microphone, or both.

- **A microphone vanished from the admin page about eight seconds after it
  loaded.** The status pill polls `/admin/audio/sources/{id}/probe` every 8 s
  with `hx-swap="outerHTML"`, but carried no `hx-target` of its own. `hx-target`
  is inherited, the enclosing `<li>` declares `hx-target="this"`, and htmx
  resolves an inherited `"this"` to the element that *declares* the attribute —
  the `<li>`. So each poll swapped the probe response, a bare status `<span>`,
  over the entire row. The header still read "1 mic" (a separate out-of-band
  span), and a page refresh restored the row because it is re-rendered
  server-side, which is what made it look cosmetic.

  Reported from a real station whose microphone was down at the time — which is
  exactly when an operator is on that page and least able to afford the list
  emptying itself. The Edit and Remove buttons in the same row already stated
  `hx-target="closest li"` explicitly; the pill was the one that did not. Both
  the template's pill and the `/probe` replacement now set `hx-target="this"`,
  and a test asserts it on both, since fixing only the replacement would leave
  the first poll after every page load still wrong.

- **Microphone capture could never work on a bare-metal install.** The unit
  granted audio with `DeviceAllow=/dev/snd rw`, but `DeviceAllow=` resolves a
  path to a *device node* and `/dev/snd` is a **directory**, so the rule matched
  nothing. With `DevicePolicy=closed` every ALSA node stayed denied and the PCM
  open failed with *"audio open error: No such file or directory"*. `arecord`
  still exec'd successfully — so the daemon logged *"started microphone capture"*
  — and the supervisor then saw a source producing no samples, killed it, and
  restarted it every 60 s forever.

  Fixed by using systemd's documented subsystem form, `DeviceAllow=char-alsa rw`.
  Verified by A/B under `systemd-run` on the affected board: the old form cannot
  open the device, the new one records normally.

  Present since **v0.6.0** (`5dbc8f1`). RTSP stations were unaffected — `ffmpeg`
  over the network never touches `/dev/snd` — which, together with the hidden
  error below, is why it survived six releases.

- **`/admin` was served to the network on every bare-metal install, while
  `--doctor` reported it protected.** The installer generates an admin password
  on a fresh non-loopback install and writes `CADDY_PWD` to
  `/etc/birdnet/birdnet.conf`; the unit it installs sets no `EnvironmentFile`.
  The auth bootstrap read **only** the environment, so it skipped, the seed admin
  kept its legacy hash, `admin_password_configured` returned false, and the
  cookie middleware took its open-bypass path. `check_admin_exposure` read the
  **config**, found the password, and passed — its doc comment asserting the two
  "can never disagree" while they did. Measured on hardware: `CADDY_PWD` present
  in the config, `/admin/settings` 200 unauthenticated, doctor exit 0.

  Both now call one shared resolver (`helpers::resolve_admin_password`,
  config-then-environment, empty treated as unset), so agreement is structural
  rather than asserted. Stations that set `CADDY_PWD` as an environment variable
  — including Docker — were never affected.

- **A corrupt database bricked the station instead of self-healing.** `--doctor`
  reported SQLite corruption as an *error* (exit 2), and the installed unit gates
  startup on `ExecStartPre=... --doctor ... || [ $? -le 1 ]`. So systemd refused
  to start the daemon — and the daemon is what owns the recovery: `app.rs` runs
  `check_and_recover`, restores from the newest backup that verifies, and failing
  that quarantines the corrupt file and starts fresh. The diagnostic blocked its
  own remedy; `Restart=always` then spent `StartLimitBurst=5` in under a minute
  and parked the unit in `failed`, so even repairing the database left the
  station down until someone ran `systemctl reset-failed` on site.

  Corruption is now a **warning**: still reported, and loudly, but exit 1 so the
  daemon starts and recovers. Exit 2 means "errors that will prevent operation",
  and a corrupt database does not prevent operation. Covered by a regression test
  that corrupts a real database and asserts the check warns rather than fails.

- **A nearly full disk bricked the station the same way.** Found by sweeping the
  remaining `--doctor` checks for the class above rather than by a separate test
  run. Less than 1 GiB free was an *error*, so `ExecStartPre` refused to start
  the daemon — and `start_disk_manager`, the purge that reclaims space at
  `DISK_PURGE_THRESHOLD`, runs inside that daemon. The reclaim therefore never
  ran, `StartLimitBurst` was spent in under a minute, and the unit parked in
  `failed`.

  This one is worse than the database case because it is certain rather than
  unlucky: a full disk is the most predictable end state of a 24/7 recorder, and
  the purge exists precisely to absorb it. It was also mistimed — the purge
  triggers on a *percentage*, so on a small card it fires well below 1 GiB free,
  and the check refused startup before the mechanism that fixes it had been
  reached. Now a warning, with the message naming the purge so an operator knows
  the station recovers on its own.

  The grading logic was extracted into a pure `grade_free_space` so every branch
  is testable. The previous test shelled out to `df` against the host and could
  only assert structure, never the verdict — which is exactly how a hard error
  sat on the low-space branch through six releases.

  Both remaining `Check::fail` sites were reviewed and left alone: a
  non-writable recordings directory and a missing `ffmpeg` for a configured RTSP
  source genuinely prevent operation and do not self-heal. A missing audio
  device was already a warning, correctly — the capture supervisor retries it.

- **A reboot could leave a station serving a healthy dashboard and recording
  nothing.** The installer wrote the detected microphone into the config as an
  ALSA card *index* (`plughw:1,0`). An index is assigned in detection order and
  is not stable. Measured on a Raspberry Pi 4 during the acceptance run: the
  same microphone was `card 1: PRO` before a cold reboot and `card 3: PRO`
  after it. The config still said card 1, `arecord` failed the open with *"No
  such file or directory"* on every attempt, and the capture supervisor retried
  a device that no longer existed — indefinitely, while `/api/v2/health`
  returned `healthy` and the dashboard served normally.

  Detection now prefers the card's **id**, which does not move:
  `plughw:CARD=PRO,DEV=0`. `CARD` is a first-class ALSA argument — alsa-lib's
  own `alsa.conf` declares `pcm.plughw { @args [ CARD DEV SUBDEV ] }` with
  `@args.CARD { type string }`, forwarded to a `type hw` slave as `card $CARD`.
  The index remains the fallback for the case where an id cannot identify a
  single card: two identical microphones report the same id, and then only the
  index tells them apart.

  `--doctor` now understands both forms. The id form was previously
  unparseable, so a correctly configured station was told on every startup that
  its device "was not found in `arecord -l`" — the diagnostic calling the
  robust configuration broken. That id form is exactly what
  [`usb-audio-mapper`](https://github.com/tomtom215/usb-audio-mapper) pins via
  a udev rule (`ATTR{id}="<name>"`), which is the supported way to keep several
  identical microphones straight; `docs/book/admin/audio.md` now says so.
  Index matching is also line-anchored: it previously asked whether the listing
  *contained* `"card 1"`, which is true of `card 12:` as well, so an absent
  card could be reported present. And when the configured card really is
  missing, the check now names the card that *is* present and prints the exact
  `ALSA_CARD=` line to set, instead of advising the operator to go and work it
  out.

  Covered by `installer/test/alsa-device-detect.sh`, which drives the detection
  against the two listings captured from the Pi either side of that reboot and
  asserts they produce an identical device string — plus a counter-test
  asserting the previous implementation did **not**, reproducing `plughw:1,0` →
  `plughw:3,0` exactly as the hardware behaved.

- **`--doctor` validated a device the daemon would never open.** Capture
  resolves its sources from the `audio_sources` table, which is seeded from
  `ALSA_CARD` only while that table is *empty* — after that the table is the
  source of truth, as `capture.rs` says outright. The audio check read only the
  config. So an operator on an established station could correct `ALSA_CARD`,
  restart, watch the diagnostic pass, and still record nothing, because the
  daemon was opening the stale device in the table.

  Measured on a Raspberry Pi 4: config set to `plughw:CARD=PRO,DEV=0`, service
  restarted, and the journal kept reporting `started microphone capture
  device=plughw:1,0` from the table — gauge at 0, nothing recorded, while every
  configuration file on the box said the right thing.

  This is the same shape as the `CADDY_PWD` defect above: two readers of one
  setting, disagreeing, with the diagnostic reading the one the runtime
  ignores. Resolved the same way — the check now consults the table through the
  **same `AudioSourceStore::list` query** the capture path uses, probes the
  devices that will really be opened, and when the config and the table
  disagree, says so and names both values. A missing or corrupt database is not
  a finding here: `check_database` owns that, and a doctor that failed on a
  corrupt database would block the startup that repairs it.

- **The installer told operators to sign in with a username that does not
  exist.** `install.sh` printed `username: birdnet`, wrote `CADDY_USER=birdnet`
  into `birdnet.conf`, and four docs pages repeated it — but the only account
  the dashboard seeds is `admin`, and the login form reads `CADDY_USER` from the
  **process environment**, which the bare-metal unit never sets. Until the
  `/admin` fix above, this was harmless: the panel was open, so nobody ever had
  to sign in. Closing that hole converts it into a lockout — the operator
  follows the installer's own output and cannot get in.

  Found on hardware minutes after the auth fix was verified, by trying to sign
  in. The installer, the generated config's comments, and the docs now say
  `admin`, and record that `CADDY_USER` takes effect only where the environment
  reaches the process (Docker). The docs also stop calling the panel HTTP Basic
  Auth: `/admin*` redirects (303) to a `/login` form and issues a session
  cookie, so `curl -u` never applied to it.

- **Every auto-install path was gated on `apt-get`, so on Fedora, Arch and
  openSUSE the installer printed advice that could not be followed.** The
  binary is a plain ELF and runs on those distributions; only the installer
  assumed Debian. A missing `ffmpeg` on Fedora produced "run `sudo apt-get
  install -y ffmpeg`" — worse than saying nothing, because it looks
  authoritative. Package handling now goes through `detect_pkg_mgr` /
  `pkg_name_for` / `pkg_install` / `pkg_install_hint`, covering **apt, dnf,
  pacman and zypper**, and degrading to "install X with your distribution's
  package manager" when it recognises none.

  Package names were established by installing them in real containers rather
  than assumed: `alsa-utils`, `qrencode` and `util-linux` carry the same name
  on all four, and `ffmpeg` is the sole exception — Fedora ships it as
  `ffmpeg-free` in its main repositories, the unencumbered `ffmpeg` being in
  RPM Fusion, which an application installer has no business enabling on
  someone's machine. `pacman` refreshes with `-Sy` and never `-Syu`: upgrading
  an operator's entire system is not an installer's decision.

  The matrix is preserved as `installer/test/pkg-manager.sh` (Debian trixie,
  Fedora 41, Arch, openSUSE Tumbleweed, plus a no-package-manager case). It
  asserts the tool actually lands on `PATH`, not merely that a command was
  issued. Running it caught two defects that reading could not: the
  unknown-distro branch emitted `install ffmpeg with your distribution's
  package manager && sudo systemctl restart …`, chaining prose into something
  that looks runnable, and the `|| true` guards on the `ensure_capture_tool`
  calls turn out to be load-bearing — the installer runs under `set -e`, so
  without them a warning the operator could act on would abort the install
  instead.

- **`alsa-utils` was never installed, so a microphone station could install
  cleanly and record nothing.** The installer ensures `ffmpeg` when the config
  names an RTSP source, but the ALSA path — the default for a USB microphone —
  only ran `command -v arecord … || true`. `arecord` is both the capture backend
  the daemon spawns and what the installer's own card auto-detect reads, so
  without it detection silently found no device, wrote no `ALSA_CARD`, and the
  station recorded nothing while reporting a clean install. Raspberry Pi OS
  ships `alsa-utils`, which is why this stayed invisible; a minimal Debian does
  not. Both backends now go through one `ensure_capture_tool` helper, `arecord`
  is installed before onboarding so auto-detect has something to read, and a
  still-missing `arecord` at detection time says so instead of returning an
  empty string.

  The install smoke test now **asserts** `arecord` is present after
  `install.sh`, rather than inferring it from the job passing. That distinction
  is the whole point: a failed `alsa-utils` install is deliberately only a
  warning, so the installer exits 0 either way and a green job proved nothing
  about this path. Verified both directions in the job's own `ubuntu:24.04`
  image — with the package manager reachable the assertion passes, and with it
  broken `install.sh` still exits 0 while the assertion fails.

### Changed

- **`birdnet_inference_duration_seconds` no longer claims to be per-chunk.** It
  is observed in `daemon/processor.rs` inside the `DispositionDecision::Accept`
  arm, immediately after `insert_detection` — i.e. **once per stored detection**,
  not once per audio chunk fed to the model. Its `HELP` text said "Per-chunk
  inference latency", which invites exactly the wrong inference: dividing the
  count by elapsed time reads a quiet hour as catastrophic audio loss. The
  exposition text and the surrounding docs now say what it measures, and note
  that no per-chunk counter is exported, so analysed-audio coverage cannot be
  derived from the metrics endpoint.

- **Capture-subprocess failures are now logged at `warn` instead of `debug`.**
  `arecord`/`ffmpeg` stderr — the only place the reason a source will not start
  is ever written down — went through `drain_capture_stderr` at `debug!`, and the
  default filter is `info,birdnet_behavior=debug`. That module is in
  `birdnet_core`, so it sat below the threshold: the supervisor's endless
  "capture (re)start issued" was visible while the error explaining it was not.
  Lines reporting a failure are promoted to `warn`; routine chatter (xruns, RTSP
  reconnects) stays at `debug` so a busy station does not spam the journal.

### Added

- **`scripts/hardware-test.sh`** — an on-device acceptance harness, documented in
  [`docs/book/field/hardware-test.md`](docs/book/field/hardware-test.md). It
  installs from the
  published release, measures mean inference latency per 3 s chunk and peak SoC
  temperature under load, and then deliberately breaks the station — watchdog
  SIGSTOP, microphone hot-unplug, network loss, disk-full, SQLite and DuckDB
  corruption, cold reboot — to establish that each documented recovery path is
  real on the hardware rather than only in `cargo test`. Results are written as
  a pasteable `report.md` plus machine-readable JSONL.

  Two defects in the harness itself, both found by running it rather than
  reading it. **Ctrl-C did not stop a run**: `trap cleanup EXIT INT TERM` with
  a handler that returns does not end a bash script — execution resumes where
  the signal landed, so an interrupt during the destructive suite freed the
  ballast and then carried on into the next fault injection. Signals now clean
  up and `exit 130`. And **`--skip` was missing**, so testing a locally
  installed binary meant either letting the install phase overwrite it with the
  published release, or hand-listing fourteen `--phase` flags — and the
  `--resume` the reboot phase prints would have run the install phase anyway,
  swapping the binary halfway through the suite. Skips are now recorded in the
  state file, which is what makes resume honour them.

  The `diskfull` phase sizes its ballast to cross **both** relevant thresholds:
  the purge fires on a percentage (95 % by default) while doctor grades in
  absolute bytes (under 1 GiB free), and on a 32 GB card filling to 96 % leaves
  1.3 GiB — enough to report success without ever reaching the branch under
  test. It also restarts the service while the disk is full, because the defect
  it exists to catch is on the startup path: a daemon that is already running
  never touches the `ExecStartPre` gate.

## [0.11.0] - 2026-08-09

### Fixed

- **Docker images embedded a behavioral extension the engine could never
  load.** `Dockerfile` pinned the DuckDB community extension to `v1.5.3` while
  the workspace bundles DuckDB 1.5.5. When the engine was bumped (`b35d4f5`)
  `ci.yml` and `release.yml` were updated and the `Dockerfile` was not — and
  because the `v1.5.3` URL still returns HTTP 200, the download *succeeded* and
  the fetch pointed at a real but unloadable artifact.

  The first run of the new CI gate then showed the failure was worse than the
  pin: `curl` is installed only in the *runtime* stage, so in the **builder**
  stage the fetch exits 127 and silently takes the fallback branch. **No Docker
  image has ever embedded the extension, on any architecture** — the wrong pin
  never got as far as being downloaded. Both are fixed: the pin is corrected and
  the builder stage installs `curl` + `ca-certificates`. The extension is also
  now fetched over HTTPS rather than plain HTTP, since it is embedded into a
  binary that is subsequently SLSA-attested and cosign-signed (verified
  byte-identical to the HTTP response).

  DuckDB refuses a version-mismatched extension outright: *"The file was built
  specifically for DuckDB version 'v1.5.3' and can only be loaded with that
  version of DuckDB. (this version of DuckDB is 'v1.5.5')"*. The reason nine
  green workflows never noticed is that the loader tries the extension cache,
  then a community-registry install, and only then the embedded copy — so a
  container *with* network installs the correct build and looks perfectly
  healthy. Only air-gapped and metered stations, exactly the deployments the
  embedding exists to serve, ever saw it, and they saw it as empty analytics
  pages.

  Fixed at four levels so the class cannot return quietly: the pin is corrected;
  `build.rs` now parses the extension's metadata footer and refuses to embed
  bytes it cannot identify, recording what they target; a mismatch between the
  embedded copy and the linked engine fails a test *and* is logged as an error
  at startup even when a network install masks it; and `docker.yml` boots the
  built image with networking disabled and asserts the extension loads.

- **A station whose database directory did not exist refused to start.** SQLite
  will not create a missing parent, so the process exited 1 on a bare *"unable
  to open database file"* — after `--doctor` had reported *"will be created on
  first run — no action needed"* and exited 0. Every sibling directory is
  already created on demand, including the DuckDB analytics store; this was the
  only exception, and the only one whose absence is fatal.

  It did not affect a stock install (the installer pre-creates the directory).
  It affected the storage move `docs/FIELD_DEPLOYMENT.md` recommends — consumer
  SD cards fail after ~6 months of WAL churn — where `RECS_DIR` works because it
  is auto-created and `DB_PATH` did not. The directory is now created before the
  database is opened, and a failure that cannot be fixed automatically (a
  read-only mount, wrong ownership) reports the directory, the cause and the
  remedy instead of a bare SQLite error.

### Added

- **`--verify-extension`.** Opens a throwaway DuckDB database, loads the
  behavioral extension the way the station does, and reports the engine version,
  the extension version and what the build-time embedded copy targets. Exits 0
  when it loads and non-zero when it does not, so it is usable from a monitoring
  script. Run with networking disabled it proves the *offline* guarantee
  specifically: with no network neither the cache nor the community registry can
  satisfy the load, so only the embedded copy can.

  `--doctor` cannot answer this question — it deliberately never opens DuckDB —
  and `TROUBLESHOOTING.md` said to use it, which is corrected.

- **`--doctor` now reports whether `/admin` is exposed without a password.**
  `--listen` defaults to `0.0.0.0:8502`, and with no admin password the cookie
  middleware serves `/admin` to anyone on the network. The station logged this
  at startup, but the diagnostic the docs point operators at checked only that
  the listen address *parsed*. It now warns when the bind is non-loopback and no
  password is set, and passes when either is untrue. Resolution mirrors the
  runtime exactly so the two cannot disagree.

### Changed

- **Dependencies converged.** The lockfile had drifted 150 packages behind, none
  of it visible as a Dependabot PR — Dependabot proposes bumps for *declared*
  dependencies, while the lockfile is what ships. The refresh includes the
  transitive security floor of a networked appliance: `rustls`, `aws-lc-rs`
  (with `aws-lc-sys`), `hyper`, `h2`, `webpki-roots`, `zerocopy` and `regex`.

- **`rubato` 3 → 4 and `audioadapter-buffers` 3 → 4, taken together.** They are a
  version-locked pair: rubato 4 requires `audioadapter ^4.0`, so bumping either
  alone puts two versions of the crate that defines `Adapter` in the graph and
  the resampler's buffer type then implements the wrong one. `process()` moved
  its `input_offset` and channel mask into an `Indexing` struct; our call used
  the defaults, so the migration is behaviour-preserving — verified against the
  real 11 000-species model, which returns bit-identical confidences
  (93.0 / 92.7 / 93.5 % on the reference Eurasian Magpie recording).

- `tower-http` 0.6 → 0.7 and `base64` 0.22 → 0.23, both drop-in.

- **GitHub Actions refreshed, and the toolchain action pinned where it signs.**
  Every third-party action SHA was verified to resolve to the tag it claims
  before being taken. `dtolnay/rust-toolchain@master` in the three release jobs
  that build attested, signed artifacts is now SHA-pinned — safe because each
  passes an explicit `toolchain:` input, so the pin cannot change which Rust is
  installed.

  Dependabot's proposed `dtolnay/rust-toolchain@1.95` → `@1.100` was **not**
  taken: for that action the ref *is* the MSRV declaration, and 1.95 → 1.100 is
  a *minor* bump, so the existing `semver-major` ignore never fired. It is now
  ignored at every update type, and a new CI job fails if the MSRV job's ref and
  `Cargo.toml`'s `rust-version` ever disagree.

- **The model-gated tests can no longer pass by doing nothing.** Rust counts a
  test that returns early as passed, so the suites that exercise the scientific
  core reported the same `ok` line whether they ran real inference or skipped —
  only the elapsed time differed (2.94 s versus 0.00 s). CI now sets
  `BIRDNET_REQUIRE_MODEL=1` in the same step that fetches and checksum-verifies
  the model, which turns a skip into a hard failure; a CDN outage leaves it
  unset, so an upstream problem still degrades to a visible skip rather than
  failing an unrelated build.

  This also fixed a suite that had never run in CI at all: `species_filter_e2e`
  — the regression tests for the species include/exclude fix, where an excluded
  species must never become a stored detection — was absent from the only job
  that exports the model path, so its 10 tests skipped in every run while
  reporting `10 passed`.

- **`CITATION.cff` is now enforced at release time.** It had been stuck at 0.8.0
  through two releases because `validate` checked only `Cargo.toml` and
  `CHANGELOG.md`, while the file's own comment asked maintainers to bump it in
  lock-step. It is the version GitHub's "Cite this repository" widget and Zenodo
  hand to anyone citing this software.

- Documentation-only follow-ups that landed after the 0.10.0 entry was written
  and belonged in no section: three surviving mutants killed in the
  species-list log guard with a refreshed CLI help snapshot (`ce54b61`), and a
  typos-config fix for backticked git SHAs plus one genuine misspelling
  (`14a5bb8`).

## [0.10.0] - 2026-08-07

### Added

- **`--offline` / `BIRDNET_OFFLINE`, and `--no-update-check`.** A station made
  two outbound connections nobody asked for — a release check against
  `api.github.com` 60 seconds after start and every 24 hours after, and
  Wikipedia species-image downloads — and the update check had no off switch at
  all. That is awkward on a metered or cellular link and unanswerable during an
  institutional review. `--offline` turns off both at once; `--no-update-check`
  turns off just the release check. Integrations you configured explicitly
  (Apprise, BirdWeather, MQTT, SMTP, heartbeat, weather) are deliberately
  untouched, because configuring one is the consent — silently muting a
  configured alert channel would be the worse surprise.

  `--doctor` now reports the current posture under **Outbound connections**, and
  the complete inventory — including the one first-run-only DuckDB extension
  fetch — is documented in *Configuration → What the station connects to*.

### Fixed

- **`partial_cmp(..).unwrap()` on floats in two page renderers.** The values are
  sums of integer detection counts, so no reachable input is `NaN` and this was
  latent rather than live. It is fixed anyway because the cost of that
  assessment being wrong is unusually high: `[profile.release]` sets
  `panic = "abort"` and the server mounts no catch-panic layer, so a panic in a
  request handler is not a 500 — it takes the whole process down, web server and
  detection daemon together. The comparisons now use `f32::total_cmp`, and both
  modules deny `unwrap`/`expect` so the class cannot return unnoticed.

  A sweep of every panicking construct reachable from a request handler
  (`unwrap`, `expect`, `panic!`, slice indexing) found no other reachable site:
  the remaining `expect`s are on `HmacSha256::new_from_slice`, which accepts any
  key length, and every `[0]` index is guarded by a length check or a
  fixed-size array.

- **A station stopped being able to start at roughly 2.1 million detections.**
  The initial SQLite → DuckDB analytics sync read the *entire* detections table
  into memory before appending a single row, so peak memory grew with the
  station's whole history rather than with the work in flight. Measured: **541
  MiB at 1 M rows and 967 MiB at 2 M**, against the `MemoryMax=1G` the systemd
  unit sets — and with `Restart=always`, crossing that ceiling produced a
  restart loop rather than a clean failure. A multi-year BirdNET-Pi database,
  which is exactly what the migration importer brings in, is that size on
  arrival.

  The sync now streams rows straight into the DuckDB appender in batches, so
  peak memory tracks the batch and not the row count: syncing 400 000 rows grew
  RSS by 53 MiB where it previously grew by 167 MiB, and 1 M rows now costs 62
  MiB. A soak test asserts the bound and fails on the old implementation.

  A failure part-way through is now also recoverable: the next sync recomputes
  its cutoff from what DuckDB actually holds and resumes, where previously an
  all-or-nothing append meant a station that died mid-sync started over.

- **A corrupt analytics database disabled analytics permanently and silently.**
  A DuckDB file that failed to open was logged once as "not available
  (non-fatal)" and then ignored on every subsequent start, leaving every
  analytics page empty until a human noticed and deleted the file by hand —
  which an unattended field station never gets. The DuckDB store is purely
  derived from SQLite, so it is always safe to discard: an unusable file is now
  moved aside with a timestamped `.corrupt.<unix-seconds>` suffix (its `.wal`
  sidecar with it) and rebuilt from SQLite on the same start. Opening is no
  longer taken as proof of health — DuckDB can attach to a damaged file and only
  fail once a query touches the broken block, so a probe read runs first.
  `--doctor` and `/admin/doctor` report any quarantined file, so the recovery is
  visible rather than buried in the journal.

- **The species allow/exclude lists never filtered a single detection.** The
  daemon built its species filter from `SpeciesFilterConfig::default()` and
  nothing in production ever populated the two lists, so a species excluded on
  `/admin/species` kept being recorded, counted, notified on, and uploaded to
  BirdWeather. The page maintained the list, confirmed every addition, and
  offered a preview page describing exactly the effect that never happened.

  Three separate defects had to be fixed for this to work, any one of which
  would have left it broken:

  - The lists were never read. They now come from the settings table through the
    same function `/admin/species` uses, so the two cannot drift, and they are
    re-read on a 30-second TTL inside the daemon loop — excluding a species is
    something an operator does *because it is spamming them right now*, so it
    takes effect on the next processed file rather than the next restart.
  - The page collects **common** names while the filter worked in **scientific**
    names, so even a populated list would have matched nothing. Entries now
    match either name form, case- and whitespace-insensitively, and the
    `/admin/species/test` preview calls the detection path's own predicate
    rather than a parallel implementation that could drift from it.
  - The filter was skipped entirely unless the station had both coordinates set.
    Only the metadata model needs to know where the station is; the operator's
    lists apply either way.

  An include list that matches no known species is now ignored with a warning
  rather than intersected to nothing — otherwise a single misspelt name would
  have silenced the whole station.

- **The species-frequency filter never ran on a normally-installed station.**
  The daemon read `cli.latitude` / `cli.longitude` with no config fallback, so a
  station configured the usual way — the installer writes `LATITUDE` and
  `LONGITUDE` into `birdnet.conf`, and `/admin/settings` writes the settings
  table layered on top of it — handed the daemon no coordinates and never ran
  the metadata model at all, leaving `SF_THRESH` inert. Coordinates now resolve
  CLI-then-config, the same rule the recording scheduler has always used.

- **Twenty settings-page fields were editable, saved, and connected to
  nothing.** The bridge between the `settings` table and the runtime config was
  a hand-maintained allow-list a new form field could simply be missing from,
  and twenty had accumulated on the wrong side of it — while the page told the
  operator "changes apply on next restart" for values no restart would ever
  read. Most reached the runtime through a flag carrying a clap
  `default_value`, so the default won unconditionally and the field could never
  take effect.

  Every key the form can persist now carries an explicit classification —
  bridged onto the runtime config, owned by a subsystem that reads the settings
  table itself, or removed — and a test fails if one is missing, so a field can
  no longer ship inert. The station resolves each setting *explicit CLI flag or
  `BIRDNET_*` variable → admin settings → config file → default*, which needed
  `clap` to be asked which arguments the operator really supplied rather than
  guessed at with per-flag sentinels.

  Newly working from the web UI: segment duration, frequency shift, night
  inhibit, the pre-sunrise and post-sunset offsets, multi-stream RTSP URLs, the
  custom species-image directory, and the weekly report schedule.

- **Apprise and BirdWeather could be configured in the web UI and would never
  send.** Both clients read only the CLI flag and the config file, so a token or
  notification URL entered on the Settings page was stored and ignored — and the
  admin "Send test notification" button read the *saved* value, so the test
  succeeded while live detections notified nobody. Both, along with the
  notification trigger mode, cooldown, minimum confidence, species allow/exclude
  lists and message templates, now reach the runtime from either surface.

- **Dawn and dusk recording windows can now differ.** The scheduler has always
  carried separate pre-sunrise and post-sunset offsets and the settings page has
  always shown two fields, but the runtime wrote a single `--twilight-offset`
  into both, so no surface could make them differ. Each end now resolves on its
  own via `--pre-sunrise-offset` / `--post-sunset-offset` (or the matching
  settings fields), falling back to `--twilight-offset` when unset — so existing
  stations keep their current symmetric behaviour.

### Removed

- **The Settings page's "Web Authentication" card.** Its password field stored
  whatever was typed as a **plaintext** row in the `settings` table, rendered it
  back into the page HTML on every later load, and changed no credential at all
  — the admin password is an Argon2id hash in the accounts database, seeded
  from `CADDY_PWD`. The section also claimed that clearing the field would
  "disable HTTP Basic Auth", which it never did. The card now explains where the
  credential actually lives, and any plaintext row left by an earlier build is
  deleted on the next start.

- **Two settings inputs with no runtime consumer at all.** "Audio Channels"
  duplicated a control that already works per-source on
  `/admin/audio` (which is where the channel count is really read from), and
  "Include Species Image" drove nothing in the notification stack. The audio
  section now points at the page that works; the notification option is gone.

### Added

- **Time-based clip retention that actually works — and is off by default.**
  The settings form has always shown a "Keep Recordings (days)" field promising
  that older audio was deleted automatically. Nothing ever read it: the key had
  no consumer and no bridge into the runtime config, so the setting was inert
  while the configuration docs correctly stated retention was not time-based.
  Age-based retention now runs on the daily maintenance tick — locked clips are
  exempt, a file shared by several detections goes only when every one of them
  is past the cutoff, and the detection rows survive so counts, species lists,
  trends and exports are unaffected. It uses a **new** setting
  (`clip_retention_days`, default `0` = keep forever) rather than the old inert
  one on purpose: the old field defaulted to 30 in the form, so stations carry
  a value nobody meant, and teaching that key to work would have deleted every
  clip older than a month at the first tick after upgrading.

- **Every disk-retention limit is settable from the web UI and the
  environment.** The purge threshold and the transient stream directory's age
  and size limits previously required hand-editing the config file — which the
  Docker entrypoint does not even use, leaving container operators no way to
  change them. All are now settable via `--disk-purge-threshold`,
  `--stream-retention-secs`, `--stream-max-mb` (each with a `BIRDNET_*` env
  var), via **Settings → System**, or via the config file, resolved in that
  order.

- **Station Health shows RAM `/tmp` (scratch) headroom.** The service streams
  live audio segments through `/tmp`, which on a Pi is a small, RAM-backed tmpfs
  separate from the data disk — and the existing "Disk" tile only watches the
  data partition, so a filling `/tmp` (which silently breaks the capture pipeline
  and even `apt`) was invisible on the dashboard. A new "Scratch" vital tile
  shows its usage, and the attention banner flags it when it runs low. Shown only
  when `/tmp` is a distinct filesystem from the data disk, so it never duplicates
  the Disk tile on systems where `/tmp` lives on the data partition.

- **Per-species recording cap (`MAX_FILES_SPECIES`) now actually works.** The
  old filesystem sweep walked a `By_Date/<species>/` subtree that the flat,
  RAM-backed capture directory never has, so the cap silently did nothing on a
  real install. It is now enforced from the database — the authority on which
  clip belongs to which species, since common names can contain hyphens
  (`Black-capped_Chickadee`) and are not reliably parseable from filenames — on
  the daily maintenance tick: the newest N clips per species are kept and older
  ones are deleted from disk. Detection rows are preserved (stats and counts are
  unaffected; only the audio file is removed). `0`, the default, means unlimited.

### Fixed

- **Scheduled maintenance no longer resets on every restart.** The integrity
  check, session prune, per-species cap and weekly backup + VACUUM were driven by
  timers measured from process start, so any station restarting more often than a
  job's period never ran it — and unattended stations restart constantly: a
  settings change ("applies on restart"), an update, a power cut, a systemd
  watchdog bounce. A station rebooting daily never once reached the weekly
  backup. Because `check_and_recover` can only restore from a backup, that turned
  recoverable corruption into total data loss on exactly the deployments the
  schedule protects. Each job's completion is now recorded in the database
  (`maintenance_runs`, migration 21) and the schedule runs on elapsed wall-clock
  time, so an overdue job fires on the next boot. A clock correction that leaves a
  timestamp in the future re-anchors the schedule instead of suppressing the job,
  and a database that cannot be written still throttles to one run per interval.

- **The persistent recordings directory is now disk-managed.** The bare-metal
  installer always passes `--watch-dir`, so the disk manager attached to the
  RAM-backed stream directory and the data disk — where extracted clips now
  accumulate beside `birds.db` — was never watched at all, while
  `DISK_PURGE_THRESHOLD` appeared to guard it. A 24/7 station filled its card
  until SQLite writes began failing. Both directories are supervised now, each
  with the retention it needs: the stream dir keeps its age and size drain, while
  the recordings dir gets the disk-full backstop only — oldest first, never by
  age, and never a locked clip.

- **Per-species confidence thresholds apply without a restart.** Thresholds were
  read once when the daemon started, so setting one in `/admin/species` did
  nothing until the service was restarted — the row appeared, the page confirmed
  the save, and detections kept being judged by the old value, with nothing
  saying why. They are now re-read on a short interval. The page also claimed
  sub-threshold detections "will be discarded"; they are held in **Quarantine**
  for you to confirm or reject, which it now says.

- **Reclaiming a clip no longer erases its filename.** Retention used to clear
  `File_Name` when it deleted audio, losing the capture timestamp and source the
  clip was cut from — the record of what a detection was matched to. The name is
  kept and a new `Clip_Pruned_At` column records when the audio went, so a row
  now distinguishes "never had a clip" from "had one, reclaimed on this date".
  Every counting, grouping and charting query is unaffected.

- **Locking a recording now protects it immediately.** The purge read the locked
  set once at startup and ran on that snapshot for the lifetime of the process,
  so a clip locked from `/admin/recordings` was unprotected until the next
  restart, with nothing saying so. The set is re-read on every purge cycle. The
  per-species cap ignored locks entirely — setting `MAX_FILES_SPECIES` deleted
  the very recordings a researcher had marked to keep — and now excludes them,
  along with any clip another in-cap detection still references.

- **Pruned clips no longer leave a dead play button.** Retention deleted the
  audio but left the row looking playable, so the clips browser kept offering
  playback for a file that no longer existed, and the daily query re-selected
  every already-pruned row forever. The `Clip_Pruned_At` stamp above resolves
  both. The "has playable audio" test was spelled out at eight call sites and is
  now one shared definition, so no surface can disagree with another about what
  can be played.

- **Backups are visible, downloadable and deletable again.** Snapshots are
  written as `{db_name}.backup.{unix_secs}`, whose extension is the timestamp
  rather than `db`, but the admin surface filtered for names ending in `.db`. It
  matched nothing any station has ever produced: `/admin/system/backups` reported
  "No backups found" on every install, and download and delete rejected every
  real file with a 400 — indistinguishable from simply having no backups.

- **The Station → Data tab reports real numbers.** It rendered a mock-up as live
  telemetry: a fixed "Last backup: 2 h ago · auto · nightly 03:00" (there is no
  nightly backup, and on a restart-prone station none had ever run), a
  "Restore tested · verified bootable" line for something nothing tests, eight
  invented snapshot rows with working-looking Restore buttons, hardcoded storage
  figures, and an operations log quoting an S3 upload failure for an integration
  that does not exist. Every figure is now measured from the running station, and
  a station with no snapshots says so. `POST /admin/system/restore` — which
  existed but had no UI anywhere, so a full backup could be downloaded and never
  restored — is now reachable.

- **A full `/tmp` no longer breaks the station (and `apt`).** Raw capture
  segments are written continuously into the RAM-backed stream directory, but
  nothing ever deleted them once the detector had processed them: the disk
  manager's safety net only purged a `By_Date/` subtree, which that flat
  directory never has, so it ran every minute and reclaimed nothing. A station
  could fill a ~2 GiB tmpfs within hours, breaking the capture pipeline and even
  `apt`, while the dashboard's Disk tile — watching the *data* partition — still
  read healthy. The disk manager now drains the stream directory by age and by a
  total-size ceiling (`STREAM_RETENTION_SECS`, `STREAM_MAX_MB`), and its
  disk-full purge now also considers those flat segments. Draining only ever
  applies to the transient capture directory, never a persistent recordings dir.
- **Extracted detection clips now persist, appear in Recordings, and play.**
  Three separate faults stacked into one broken feature on a default systemd
  install. Clips were written to a sibling `Extracted/` directory next to the
  capture directory — i.e. onto `/tmp`, which `PrivateTmp=yes` wipes on **every
  restart** — while the web server reads recordings from the data disk, which
  nothing ever wrote to. They were also nested under `By_Date/<date>/<species>/`,
  though the recordings API serves and lists by bare filename. And the database
  recorded the *source segment's* name rather than the saved clip's, so even a
  correctly-placed clip could not be found. Clips are now written flat into the
  same directory the web server serves from (one source of truth, so the two
  cannot drift apart), and the clip's own filename and duration are what get
  stored. The filename already encodes species, confidence, date and time, so
  nothing is lost by dropping the nested layout. Detections recorded *before*
  this fix keep their old filename and remain unplayable.
- **Adding two different audio sources within the same second no longer fails.**
  The synthetic source id was `src_<kind>_<seconds>`, so two sources added in the
  same second collided and the second add returned a baffling "Retry — a new id
  will be generated" toast. The id now carries a process-local sequence and is
  always unique.
- **The Audio sources admin page no longer strands you or contradicts itself.**
  Several rough edges are fixed together: the RTSP "Network streams" section was
  *hidden* whenever no stream existed yet, so once you had a microphone the "Add
  stream" form was unreachable — both sections are now always shown. The
  per-section counts ("N mics" / "N streams") update the instant a source is
  added or removed (they used to go stale), the separate empty-state card that
  contradicted a freshly-added row is gone, and the edit form's **Cancel** button
  — which fetched the status pill and swapped nothing, leaving the form stuck
  open — now restores the row.
- **The dashboard "what's new" banner no longer reads "New in vUnreleased."**
  The banner showed the topmost changelog entry, which is the in-progress
  `## [Unreleased]` section, so it rendered a meaningless version to everyone.
  It now shows the latest *released* version (skipping `Unreleased`), or no
  banner at all when there is no release yet.
- **The admin "Restart" button now actually restarts the service.** It shelled
  out to `systemctl restart`, which a non-root, sandboxed service can't do
  (polkit-denied) and which races its own `KillMode=mixed` cgroup teardown. It
  now signals itself (SIGTERM) and lets the unit's `Restart=always` bring it
  back — responding to the browser first so the page can show the status. When
  the binary isn't running under systemd it now says so plainly instead of
  killing itself and reporting a false "restart sent."
- **Adding the same microphone or RTSP stream twice is now prevented.** The
  audio-source form only de-duplicated on a synthetic id (always freshly
  generated), so the same physical device could be added over and over. It now
  rejects a source whose kind + device id already exists, with a clear message
  pointing to the existing entry.
- **Station Health "Vitals" now report real CPU and memory.** The hardened
  systemd unit set `ProcSubset=pid`, which hides the system-wide `/proc` files
  (`/proc/stat`, `/proc/cpuinfo`, `/proc/meminfo`) that the `sysinfo` crate reads
  — so the dashboard showed an impossible **0 CPU cores / 0% CPU** and **0 B / 0 B
  memory**, while temperature (read from `/sys/class/thermal`) and disk (via
  `statvfs`) still worked. The unit no longer restricts `/proc` (a comment marks
  why it must stay at the default), while `ProtectProc=invisible` still hides
  other users' processes. Apply to an existing install with
  `sudo bash install.sh repair`, which rewrites and reloads the unit.
- **A fresh bare-metal install now starts the dashboard immediately, even with
  no audio source.** Previously `install.sh` only ran `systemctl start` when an
  ALSA/RTSP source was already in the config, so an operator who clicked through
  the setup wizard with no microphone auto-detected was left with a service that
  "did not come up" — yet the unit is *enabled*, so the next reboot started it
  anyway, which was both confusing and inconsistent. The installer now starts the
  service unconditionally on a fresh install (the systemd doctor preflight treats
  "no audio source" as a warning, not a failure), so the web dashboard — and its
  first-run onboarding wizard, where the microphone and location are chosen — is
  reachable the moment the installer finishes. This matches the Docker quickstart,
  which already brought the dashboard up regardless of audio. The post-install
  summary now clearly notes when no audio source is set yet and points to the
  in-dashboard setup wizard.
- **A mistyped stream URL is no longer silently accepted as a sound card.** The
  installer's audio-source prompt treated anything that wasn't an `rtsp://` URL
  as an ALSA device name, so a typo'd scheme (`http://camera…`) was written into
  the config as a sound-card string that could never open. Input that looks like
  a URL but isn't `rtsp://` / `rtsps://` is now rejected with an explanation and
  re-prompted. Plain ALSA device names (`plughw:1,0`, `default`) are unaffected.
- **Skipping an installer safety check is now impossible to miss.**
  `BIRDNET_SKIP_MODEL` and `BIRDNET_SKIP_GLIBC_CHECK` announced themselves with a
  single `[WARN]` line that blended into the surrounding install output — and
  lost its colour entirely in a piped or CI install — so the eventual failure
  (a daemon that detects nothing; a `GLIBC_… not found` crash at startup) arrived
  with no obvious cause. Each bypass now prints a boxed, unmissable warning that
  survives a non-interactive install and states the consequence.
- **A disabled notification-test button now says why it's disabled.** The Apprise
  and BirdWeather test buttons greyed out with no explanation when the channel
  had no credentials. Each now carries a tooltip *and* visible hint naming the
  exact setting to fill in — the hint because browsers suppress tooltips on
  disabled buttons.
- **The "what's new" banner no longer vanishes silently when it can't load.**
  After an upgrade, if the release-notes request failed — the server still
  restarting, a 5xx, an older build without the endpoint — the banner simply
  never appeared, indistinguishable from having no news. It now falls back to a
  minimal "updated to vX.Y.Z" banner linking to the full changelog. A server
  that intentionally has no release to announce still stays quiet.

### Dependencies

- `duckdb` 1.10503.1 → **1.10505.0** (bundled DuckDB 1.5.3 → **1.5.5**), to pick
  up **`duckdb-behavioral` v0.9.1**. The behavioral extension is version-locked
  to the DuckDB it was built for — DuckDB refuses to load a mismatch, and
  `allow_extensions_metadata_mismatch` does not bypass that check — so the
  bundled engine moves in lockstep with the published community build. Verified
  before landing rather than assumed: the community CDN's `v1.5.5` artifacts for
  both `linux_amd64` and `linux_arm64` report `behavioral_version v0.9.1`, and
  both load paths succeed (online `INSTALL … FROM community` and the offline
  embedded fallback) with every behavioral function executing. Note the `v1.5.4`
  CDN path is *not* usable — it still serves a byte-identical copy of the old
  v0.8.0/1.5.3 build, which is exactly why an HTTP 200 on a version path is not
  sufficient evidence to bump.

## [0.9.0] - 2026-06-22

### Added

- **OpenAPI 3.1 description of the public JSON API.** The full `/api/v2`
  surface (44 read-only endpoints across detections, species, recordings,
  analytics, time-series, export and system) is now described by a committed,
  hand-maintained OpenAPI 3.1 document (`crates/birdnet-web/openapi.json`),
  served live at `GET /api/v2/openapi.json` so any tool — Swagger UI, Redoc,
  Postman, `openapi-generator` — can map the API or generate a client. The spec
  honestly declares the API as unauthenticated (`security: []`); a committed
  `redocly.yaml` documents why two of Redocly's opinionated default rules don't
  apply (intentional openness, read-only endpoints) so `redocly lint` is clean.
  A test parses the embedded document and asserts every documented path is
  actually routed, so the spec can't drift out of sync with the server. The
  HTTP-API reference doc is corrected alongside it (the `detections/daily` and
  `species/activity` query parameters were documented incorrectly).
- **Recordings now shows each saved clip's duration.** A deferred Wave D
  omission (the Clips grid dropped the column rather than fake it) is now
  backed honestly. **Migration 20** adds a nullable `Duration_Secs` to
  detections; the daemon reads the source recording's length from its file
  header — cheaply, via a new `birdnet-core` `decode::probe_duration_secs`, with
  no re-decode — and persists it. Historical, BirdNET-Pi-imported and
  quarantine-approve rows have no length to record and stay `NULL` (the grid
  omits the column for them, never a guess). The Clips grid renders the length
  as `M:SS` under each row's time.
- **Recordings clips show "first today" / "rare" badges.** Another deferred Wave
  D omission: each clip row now carries the same first-seen badge the Today feed
  shows — "first today" when the species' first-ever record is today, "rare"
  when the clip sits on the species' first-ever (historical) date — reusing the
  existing `species_first_seen` query and `bnb-pill` styling (no new query, no
  new tokens). A clip with no first-ever match shows no badge.
- **Recordings clips show a spectrogram thumbnail.** The last deferred Wave D
  Recordings omission is now backed honestly — by reusing the existing
  `/api/v2/spectrogram/{file}` endpoint (the same renderer, viridis colormap and
  byte-budgeted cache the detection-detail view already uses) rather than a
  second system. That endpoint gains a `?thumb=1` mode that max-pools the time
  axis down to a small fixed width (so a multi-second clip ships a few KB instead
  of a multi-thousand-pixel image, and brief calls still survive the shrink),
  cached separately from the full-size render. The Clips grid links a lazy-loaded
  thumbnail only for rows whose audio is present — gated by a single per-page
  directory scan, the same way the locked-clip set is loaded — so there is no
  per-row stat, no schema change, and historical clips get a preview too; rows
  whose audio is gone show an empty aligned spacer rather than a broken image or
  a faked tile. New CSS only (`.rc-spectro`); no new design tokens, no new
  dependency.
- **CI: an accessibility gate and a structural visual-QA sweep.** A new
  `a11y.yml` workflow boots the seeded `screenshot_server` fixture once and runs
  two gates against it — **axe-core** (WCAG 2.1 A/AA, light + dark themes) fails
  the build on any serious or critical violation, and the **`qa.mjs`** sweep
  fails on a structural regression: horizontal overflow, console/page errors,
  responses ≥ 400, broken images or stuck loaders. Path-filtered to web/tooling
  changes; the visual gate is deterministic (no flaky pixel baselines). The axe
  gate enforces every serious/critical rule except two deferred (with a written
  rationale in `axe.mjs`) to a design-reviewed pass: `color-contrast` (the v3
  palette renders each species' identity hue as text and uses a muted meta-text
  hierarchy — an all-or-nothing design-token decision) and `link-in-text-block`
  (an app-wide link-underline policy).
- **Adopt duckdb-behavioral v0.8.0's new ClickHouse-parity functions.** The
  community `behavioral` extension served for the bundled DuckDB (v1.5.3) is now
  v0.8.0 (pin verified — no engine change needed), which adds `sequence_count`,
  `window_funnel_events` and `sequence_match_events`. `birdnet-behavioral` gains
  typed wrappers for all three — `AnalyticsDb::sequence_count` (how *many* times
  an ordered species sequence occurred per day, not just whether it did),
  `AnalyticsDb::funnel_events` (the timestamp each completed dawn-chorus step
  fired) and `AnalyticsDb::sequence_match_events` (the per-step timestamps of an
  ordered NFA-pattern match — the longest in-order prefix reached that day) —
  with SQL builders, unit tests, and live tests verified against the real
  extension. Exposed over the REST API as
  `/analytics/{sequence-count,funnel-events,sequence-match-events}`.
- **The Patterns → Behavior tab surfaces the dawn "running order."** A new
  defined-in-place card reads the station's own dawn-window data to pick the
  morning's leading voices, then uses v0.8.0's `sequence_count` and
  `sequence_match_events` to show how *often* they sing in that exact order and,
  on a recent morning, the *time* each one checked in. Both halves share the
  same NFA-match semantics, so the headline count and the step timing can't
  disagree. The sequence is derived from the data rather than hard-coded (the
  REST defaults are European), so the card reads honestly at a North-American
  station too. The card now also **leads with a funnel picture** (a new
  server-rendered inline-SVG `viz::sequence_funnel`) built from v0.8.0's
  `window_funnel`: how many mornings reach each step of the running order, the
  bars narrowing as the chorus progresses — drop-off you can read at a glance.
  It is omitted, never drawn empty, when no morning reaches even the first step.
- Permanent (`308`) redirects from every pre-spine route to its new home
  (`/today`, `/heatmap`, `/analytics`, `/migration`, `/correlation`,
  `/timeseries`, `/analytics/dawn-chorus`, `/weekly`, `/year-in-review`,
  `/history`, `/system`, plus the live-audio paths `/listen`, `/livestream`
  and `/live`), so existing bookmarks and BirdNET-Pi muscle memory never 404.
- `recent_clips` / `recent_clips_count` (`birdnet-db`): a cross-date,
  filterable, paginated query of clips that saved an audio file, behind a
  `RecordingsFilter` (All · Best · Rare · Locked) that reuses the Today log's
  "best"/"rare" definitions. Powers the Recordings Clips browser.

- **Self-hosted ingest endpoint for uploads** (`BIRDWEATHER_URL` config key /
  `BIRDNET_BIRDWEATHER_URL` env). Research programmes tracking sensitive
  species can route the entire upload pipeline — including the offline queue
  and ordered replay — at their own endpoint implementing the `BirdWeather`
  station API shape, keeping observation locations under their own
  governance. Only the host changes; the `/stations/<token>/...` path shape
  is preserved, and the active endpoint is logged at startup.
- **End-to-end delivery proof for the store-and-forward queue**
  (`tests/store_forward_e2e.rs`): boots the real compiled binary against a
  local stub `BirdWeather` server with a pre-seeded backlog and asserts the
  drainer replays it oldest-first, in the real camelCase wire format, to the
  station-token path, and leaves the queue empty — closing the one branch of
  the replay loop (deliver → 200 → dequeue) that the outage-side live test
  could not reach.

- **Store-and-forward `BirdWeather` uploads** (`outbound_queue`, migration
  19). Posts that fail after their in-flight retries are parked in the local
  database and replayed automatically when the uplink returns — oldest
  first, capped batches with spacing, exponential backoff to a 1 h ceiling,
  bounded to 5 000 entries and 48 attempts so a weeks-long outage can never
  grow the database without limit. The field runbook had promised
  "buffered locally; retried with exponential backoff" all along; the code
  now keeps that promise. MQTT and Apprise/email deliberately stay
  fire-and-forget (live telemetry / look-now alerts — replaying them hours
  later is worse than dropping them). Exposed as the
  `birdnet_outbound_queue_depth{kind}` gauge and a "Queued Uploads" row on
  the `/system` page whenever non-empty.
- **Detection deadman watchdog.** The end-to-end "is the station actually
  detecting?" check: every component gauge can be green while a clogged
  mic foam or a model/labels mismatch silences the station. The daemon now
  measures seconds-since-last-detection (in SQLite's own localtime lens, so
  no TZ skew), exports it as `birdnet_detection_silence_seconds`, surfaces
  it on `/api/v2/health` (`detection_silence_secs`) and as the `/system`
  page's "Last Detection" row, and after a configurable quiet threshold
  (`--deadman-hours` / `BIRDNET_DEADMAN_HOURS` / `DEADMAN_HOURS`, default
  24 h, `0` disables) logs a loud warning and sends one Apprise alert per
  quiet episode with a recovery notice when detections resume.

- **Silent-stall detection for capture sources.** The supervisor now watches
  each source's newest recording segment: a subprocess that stays alive but
  stops delivering audio (a wedged RTSP session, a USB mic hung after a
  re-enumeration) is detected after several missed segments and restarted
  through the same backoff path as a crash — closing the field failure where
  `is_running` reports healthy but a camera has gone quiet. Fails open while
  the clock is unsynced (segment mtimes aren't trustworthy pre-NTP).

- `cargo-fuzz` harnesses (`fuzz/`) for the untrusted-input parsers: symphonia
  audio decode (WAV/FLAC/MP3 demux of watch-directory files) and the
  species-label parsers, with a seeding recipe in `fuzz/README.md`.
- `CITATION.cff` (with the BirdNET reference), `GOVERNANCE.md`,
  `.gitattributes` (LF normalization + binary markers), and live CI /
  coverage / supply-chain badges in the README.

### Changed

- **Web UI reorganized into six homes (the "v3 spine").** The navigation
  collapses from 9 top-level tabs + a 14-entry "More" menu into six
  task-based homes — **Today · Species · Patterns · Recordings · Reports ·
  Station** — generated from a single nav manifest, with one shared
  vocabulary on desktop and the phone bottom bar (the desktop "More"
  dropdown and the mobile "More" sheet are retired; a Help icon and the ⌘K
  command palette cover the long tail). Every navigation surface and the
  command palette are parity-tested against the manifest.
- **Dashboard and Today merged into one home at `/`.** The old separate
  "right now" dashboard and "today log" pages were the same data twice; the
  Today home now leads with a comparative phrase ("a *busy* morning" vs your
  30-day baseline) and an honest live signal (a flat **idle** baseline when
  no audio is arriving — never a fake waveform), surfaces a review nudge or
  outage banner only when one is warranted, plots the day on a rebuilt strip
  (hourly histogram + in-strip temperature + real sunrise/sunset), and folds
  the live feed and the full searchable/filterable day into one log behind a
  disclosure. A brand-new station gets a "getting ready" checklist instead of
  an empty page.
- **Analytics, reports and system pages fold into tabbed homes.** Activity
  heatmap, dawn chorus, migration, co-occurrence, time-series and behavioral
  analytics are now the six tabs of **Patterns**; the weekly report, year in
  review and history are the three tabs of **Reports**; the read-only system
  health page is the public **Health** tab of **Station**. The underlying
  server-rendered SVG renderers are unchanged.
- **Patterns reskinned: one picture per tab, numbers behind a disclosure.**
  All six tabs now open with a one-paragraph, jargon-free `bnb-lede` that says
  what the chart means before the chart appears ("Darker cells mean more birds
  heard that hour…"; "Who sings, and when…"; "Each ridge is one species'
  abundance across the year…"), and each leads with a single picture, tucking
  the supporting tables and numbers behind a "see the numbers" `<details>`
  disclosure: **Who-sings-together** leads with the co-occurrence chord and
  hides the matrix + strongest-pairs tables; **Dawn chorus** leads with the
  circadian polar and hides the per-species ribbons; **Behavior** becomes a
  masonry of cards that define every term in place; **Trends** leads with the
  two headline lines (detections per week, species richness) and folds the rest
  of the dashboard behind a disclosure; **When-active** drops the duplicated
  dawn/phenology panels (each is now its own tab). The underlying server-rendered
  SVG renderers are unchanged.
- **Reports reskinned into editorial recaps.** Weekly and Year-in-review now
  open with an editorial `rp-hero` (a headline that reads the week/year — "A
  *loud* week.", "Your year in *birdsong*.") over a four-up `rp-stats` band
  (detections vs last week, species, new-to-list, busiest day), then a
  leaderboard and the first-ever/milestone columns. **History** becomes a
  month **heat-calendar**: each day is a cell coloured by its detection count
  and annotated with its species tally; selecting one loads that day's hourly
  chart and top species into a detail panel, with ‹/› month navigation, and an
  **Open day →** link to a full-page recap of that day (`/reports/day`) — its
  hourly shape, every species heard, and the complete chronological detection
  log, read-only (managing detections stays on Today / Recordings). Backed by a
  new `detections_per_day` query.
- **Reports gain a "Save as PDF" button.** Each Reports tab now carries a
  CSP-safe print affordance — a real button whose delegated, nonce'd click
  handler opens the browser's print dialog, which the existing `print.css`
  `@media print` rules turn into a clean, light-palette, page-broken keepsake.
- The detection log gains **category filters** (Rare · First today · High
  confidence) alongside text search.
- **Recordings rebuilt into a Clips + Live home (`/recordings`).** The old
  by-species / by-date browser and the separate `/listen` page merge into one
  Recordings home with a `?view=clips|live` switch. **Clips** is a flat,
  newest-first browser of every detection that saved an audio clip, with
  filter chips (All · Best · Rare · Locked), species search, a now-playing
  player that docks to a floating bar on scroll, per-clip lock/download/delete,
  and a Select mode for bulk actions. **Live** folds the live page's honest
  scrolling sonogram (real spectrogram frames; a flat idle baseline when no
  audio is arriving — never a fake waveform), source picker and live-detection
  trickle. `/listen`, `/livestream` and `/live` permanently redirect to
  `/recordings?view=live`.
- **Species rebuilt into a List + Photos + Life list home (`/species`).** The
  three pre-spine destinations — the species list, the `/gallery` photo wall and
  the `/life-list` journal — merge into one home with a `?view=list|photos|
  lifelist` switcher, an "All / This week" filter and species search. **List** is
  the ranked table (rank · avatar · 14-day sparkline · count · avg confidence);
  **Photos** is the Wikipedia-thumbnail gallery with the gradient banding-code
  fallback; **Life list** leads with the big counters (species all-time · active
  days · new this year), the species-accumulation curve, and a "New to the list"
  feed of the most recent firsts. The per-species detail page keeps its `sd-*`
  treatment with cross-links updated to the new homes. `/gallery` and `/life-list`
  permanently redirect to their view.
- **Station Health is now an operator-grade surface.** The public Station
  Health tab (`/station`, the heir to `/system`) gains an overall status
  banner, a **per-source activity** panel (how many detections each audio
  source produced today and how recently — an honest activity signal, since
  the web process has no live handle on the capture supervisor), a vitals row
  (CPU · memory · temperature · df-correct disk meters), a pipeline row (last
  detection · queued uploads · service uptime · total detections) and a short
  diagnostics checklist, in the `st-*` treatment. (The per-source live
  state-chip, 24 h uptime strip and retry/backoff line are now wired through —
  see the next entry.)
- **Station Health's per-source cards go live.** The capture supervisor now
  publishes per-source health — Connected · Stalled · Backing off · Paused,
  plus last-audio age, restart attempts, next retry, and a rolling 48-segment
  24 h uptime strip — into a shared handle the web layer reads, so each
  `st-source` card shows a real status chip, the uptime strip, time since last
  audio, today's detections, and a retry/backoff line (`↻ reconnecting ·
  attempt 3 · next try in 12 s`); the status banner flags a down source. The
  seam is a new `birdnet-core::audio::capture::status` type shared by the
  binary's supervisor (writer) and `birdnet-web` (reader), so neither depends on
  the other. With no supervisor running (web-only mode, tooling) the cards fall
  back to the detection-activity signal — never a faked chip.
- **The Station toolbox gains five gated management tabs.**
  `/station/{capture,alerts,data,settings,access}` fold the twelve flat
  `/admin/*` pages into the Station home's six task groups, rendered through
  the **main** shell with the shared Station sub-tab row but gated behind the
  same admin auth as `/admin/*`. **Capture** = audio sources · which-birds-count
  filter (with a safe Preview) · the single canonical detection-threshold home ·
  recording & location; **Alerts** = rules · channels with Send-test · where
  alerts flow · recent sends; **Data** = backups & export · BirdNET-Pi import ·
  data quality; **Settings** = per-device display prefs · station & system ·
  the kiosk launcher; **Access** = accounts & sessions · a lockout-aware danger
  zone. The real forms are reused verbatim and keep posting to their existing
  `/admin/...` endpoints — only the page GETs move. The eight folded
  `/admin/*` management pages (`audio` · `species` · `rules` · `notifications` ·
  `backups` · `migrate` · `quality` · `accounts`, plus the `/admin` landing) now
  **permanently redirect** to their Station tab, so old bookmarks never 404; the
  Health-detail pages (`overview` · `system` · `doctor`) and the all-in-one
  `/admin/settings` form stay reachable as gated fallbacks.
- **The admin panel's nav is regrouped into the six Station task groups.**
  `admin/nav.rs`'s twelve flat destinations are ordered into labelled
  **Health · Capture · Alerts · Data · Settings · Access** clusters (one
  labelled group each in the shell nav), so the gated admin area's information
  architecture matches the Station home's six tabs. Single source of truth;
  parity- and grouping-tested.
- **Accessibility: the analytics charts now name and describe themselves.**
  Every server-rendered inline-SVG chart (`viz/`) carries a `<title>` accessible
  name and a one-sentence, jargon-free `<desc>` of what it encodes (e.g. "A
  24-hour clock face with midnight at the top; each species' ribbon swells at
  the hours of day it sang most"), replacing the bare `aria-label` so a screen
  reader announces what the picture *means*, not merely that it exists. The
  Recordings → Live detection trickle is now an `aria-live="polite"` region so
  new detections are announced as they arrive (the Today feed already was). The
  segmented controls (the Today log filter, the Species view switcher, the
  display-preference toggles) drop the incorrect `role="tablist"`/`"radiogroup"`
  they carried over plain `<button>`/`<a>` children — they are honest button/link
  groups, now `role="group"` (the filter conveys its active state with
  `aria-pressed`, the view switcher with `aria-current`) — and the kiosk's
  scrolling recent-feed is now keyboard-focusable.

- The time-series dashboard's 13-row API-endpoints table is collapsed into a
  disclosure ("API endpoints · for scripts & integrations") so the page reads
  as a field tool, not an API manual.
- Kiosk mode gained an escape hatch — a dimmed corner "Exit" link and the
  ESC key both return to the dashboard (it was a dead end with no way back).
- The recordings species list uses the shared illustrated empty-state
  component instead of a bare `<p>No species detected yet.</p>`.

- `unsafe_code` lint raised from `deny` to `forbid` workspace-wide (what the
  README badge always claimed); `missing_docs` is now enforced and the ~250
  previously undocumented public items carry real rustdoc.
- Retry constants unified across `apprise` / `birdweather` / `wikipedia` to
  `MAX_ATTEMPTS` (total attempts) with exclusive ranges — the previous mix of
  inclusive/exclusive `MAX_RETRIES` loops made two of the three doc comments
  wrong. No behavioral change.

### Fixed

- **MQTT publishing no longer runs inline on the detection thread.** It was the
  one network integration (of five) dispatched synchronously in the
  single-threaded event processor, so an offline broker blocked every
  detection for the connect timeout and serialized detection handling behind a
  dead network path. It now fires off the detection path like BirdWeather /
  Apprise / email / heartbeat already did — a multi-day broker outage slows
  detection by nothing.
- System-health disk usage now reports `df`'s `used / (used + available)`
  rather than `used / total`, so a host with reserved blocks or a container
  quota no longer shows a contradictory "11% used · critically low".

- **Post-startup `SIGTERM` no longer hangs the process.** The startup-phase
  signal race in `app::run` kept racing the serve loop after startup; its
  biased arm won every later `SIGTERM`, cancelled the graceful-shutdown
  choreography (waking live connections, stopping the detection daemon), and
  left the runtime blocked forever on the detection loop's blocking thread —
  so every `systemctl stop`/`restart` with a loaded model waited out
  `TimeoutStopSec` and was `SIGKILL`-ed. The race now ends at an explicit
  startup handoff; verified live: clean stop in ~2 s with the pipeline hot.
- `--doctor` now validates the model and labels of a config-file install: it
  read the `MODEL` / `LABELS` keys while the daemon and installer use
  `MODEL_PATH` / `LABELS_PATH`, so every standard install reported
  `SKIP: no --model configured` and the model file was never checked.
- The documented image-cache opt-out (`--image-cache-dir ""`, empty
  `BIRDNET_IMAGE_CACHE_DIR`) actually parses now — clap's stock `PathBuf`
  parser rejects empty values, making the air-gapped opt-out unreachable
  from the CLI/env (the config-file key was unaffected).
- BirdNET-Pi migration no longer aborts on dirty source data: TEXT values in
  numeric columns (empty strings, stringified numbers — the upstream
  "empty-string poisoning") degrade to NULL or parse, instead of failing the
  whole import with `InvalidColumnType`.
- Unmatched paths under `/api/` return a machine-readable JSON 404 instead
  of the branded HTML page, so scripts and dashboards see the real failure.

### Security

- Auto-update HTTP reads are bounded (release metadata 8 MiB, `SHA256SUMS`
  64 KiB, release asset 512 MiB) with `Content-Length` pre-checks, so a
  compromised or misbehaving endpoint cannot stream an unbounded body into
  memory on a small-RAM Pi.
- Every GitHub Actions step is now pinned to a full commit SHA (previously a
  mix of tags and three mutable `@main`/`@master` refs), and `ci.yml` gained
  the least-privilege `permissions: contents: read` block the other
  workflows already had.

### CI

- **Mutation testing is now incremental on PRs and ~4× cheaper per mutant.**
  Three layers, each measured: a `mutants` build profile (no debug info —
  per-mutant cost 132 s → 36 s, baseline 90 s + 91 s → 16 s + 21 s on the
  binary-crate shards); unit-test-only target selection per package
  (`--lib` / `--bins`), so the mutant loop no longer rebuilds eight
  DuckDB-linking integration-test executables nor boots real binaries; and
  `--in-diff` scoping on pull requests, so only mutants on changed lines
  run (a test-only one-line diff finishes in 0.2 s, "No mutants to
  filter") while the weekly cron, pushes to main, and manual dispatch
  still run every shard's full set. Config lives in `.cargo/mutants.toml`
  so local `cargo mutants` runs share the same economics.

### Dependencies

- `mdbook` 0.4.52 → **`mdbook-driver` 0.5.3** (folds dependabot #151): mdbook
  0.5 split the project into facade crates and made the `mdbook` crate
  binary-only, so the docs build now consumes the library through
  `mdbook-driver`. The book config dropped the options 0.5 removed
  (`copy-fonts`, `multilingual`), `build.rs` now surfaces the *underlying*
  load error instead of a silent "could not load" (that silence briefly
  masked exactly this migration), and the rendered manual was verified
  page-for-page. New transitive `font-awesome-as-a-crate` carries
  `CC-BY-4.0 AND MIT` for the icon *assets* (attribution-only, not
  copyleft) — allowed via a crate-scoped `deny.toml` exception rather than
  a global allow.
- `rusqlite` 0.40.0 → 0.40.1 (folds dependabot #147).
- `codecov/codecov-action` v6 → v7.0.0, SHA-pinned (folds dependabot #150).
- `password-hash` 0.5 → 0.6 (dependabot #148) is **deliberately not
  taken**: argon2 0.5.x implements password-hash *0.5*'s hasher traits and
  our accounts code passes those types straight into `Argon2` — the bump
  alone does not compile (verified). A manifest comment now documents the
  lock-step requirement; take both together when argon2 0.6 ships.

## [0.7.2] - 2026-06-07

A pre-release hardening pass: process-crash fixes, memory/DoS bounds for small
Raspberry Pis, data-integrity fixes, and several web-security fixes — plus an
internal module-structure cleanup. No user-facing feature changes; everything
here makes an existing install more robust against malformed input, hostile
station metadata, over-long recordings, and abrupt shutdown.

### Security

- **Neutralised CSV formula injection in data exports (CWE-1236).** A species or
  comment beginning with `=`, `+`, `-`, or `@` is no longer written verbatim into
  exported CSVs, where a spreadsheet would evaluate it as a formula. Such fields
  are now prefixed so they import as literal text, and the record-separator /
  control characters that can splice extra rows are stripped.
- **Pinned auto-update downloads to GitHub release hosts over HTTPS.** The
  self-updater now refuses any release-asset URL that is not an `https://` GitHub
  host, so a tampered release feed cannot redirect the download to an arbitrary
  origin.
- **Escaped Home Assistant MQTT discovery payloads.** Discovery messages are now
  emitted as properly encoded JSON, so a station name containing quotes, braces,
  or control characters can no longer break out of the payload or inject fields.
- **Stopped leaking internal error detail to the admin UI.** Recording-save and
  related failures now surface a generic message to the browser and log the
  detail server-side, instead of echoing internal paths and error strings into
  the page.
- **Bounded request-driven work on the web surface.** On-demand spectrogram
  rendering and the live stream are now concurrency-limited, deterministic `4xx`
  client errors are no longer retried, and spectrogram parameters are sanitised —
  closing several avenues for a single client to pin CPU or memory on a small Pi.
- **Closed an auto-update host-pin bypass via URL userinfo.** The release-asset
  host check parsed the authority by splitting on `:`, so a URL like
  `https://github.com:x@evil.com/…` read as the trusted host `github.com` while
  the download would actually go to `evil.com`. The host is now taken from the
  segment after the last `@` (userinfo stripped), closing the spoof for both the
  binary download and the `SHA256SUMS` fetch.
- **Clamped the public analytics query parameters.** The unauthenticated
  `/analytics` endpoints now cap the `limit` and the `?species=` sequence length,
  so a single request can't force an oversized result set or sequence on a Pi.

### Fixed

- **`stop`, `restart`, and upgrades no longer stall ~10 s on every shutdown.**
  The live dashboard holds a WebSocket open (the listen page a second one, and
  the admin Live Logs page an SSE stream). On `SIGTERM`, axum's graceful drain
  waited for those to close on their own, so with any tab open it always hit the
  `SHUTDOWN_GRACE` cap and force-exited with `shutdown grace elapsed with
  connection(s) still open`. The server now signals those handlers to close the
  moment shutdown begins, so the drain finishes in milliseconds and shutdown is
  clean and quiet. The 10 s cap stays only as a backstop for a client that
  ignores the close.
- **Several panics that would abort the whole process are gone.** Because release
  builds compile with `panic = "abort"`, any unhandled panic in a request handler
  or background task takes the entire daemon down. This pass fixes a class of
  them: date parsing that sliced multibyte UTF-8 rows on a byte boundary, webhook
  URLs truncated mid-character in the rules table, and a `date_to_epoch_days`
  underflow on pre-epoch dates (now clamped to the epoch). Malformed or unusual
  data is handled instead of crashing.
- **Poisoned locks no longer wedge analytics and image fetches.** If a thread
  panicked while holding certain mutexes (the full-analytics resync, the
  Wikipedia image cache), every later caller would panic on the poisoned lock in
  turn. Those paths now recover the guard and continue.
- **The DuckDB analytics copy can no longer be wiped by a failed rebuild.** The
  full resync is now atomic: it builds the new OLAP copy and swaps it in only on
  success, so an error partway through leaves the previous analytics intact
  instead of emptying them.
- **Settings writes are atomic.** A configuration save now lands as a single
  transaction, so a crash or concurrent reader can't observe a half-written
  settings row, and the surrounding DB resilience paths were hardened.
- **Long recordings can't exhaust memory.** On-demand spectrogram decoding is now
  capped at ten minutes of audio (≈115 MB), so an unusually long station
  recording — or a misconfigured multi-minute segment — renders its leading
  portion instead of allocating an unbounded buffer and risking an OOM on a Pi.
  The detection pipeline still decodes every sample.
- **Audio seeking works in the recordings player.** The recording endpoint now
  honours HTTP `Range` requests, so scrubbing within a clip seeks in the browser
  instead of re-fetching from the start.
- **Assorted correctness and robustness edge cases** surfaced by the pre-release
  audit — input validation on several admin forms, daemon and purge edge cases,
  scheduler and identifier handling, and live-frame broadcast sizing.
- **Uploaded BirdNET-Pi databases now rebuild the analytics copy too.** The 0.7.1
  fix that refreshes the DuckDB analytics after an import only covered the
  server-path import; the browser upload path imported history into SQLite but
  skipped the rebuild, so uploaded back-dated history silently never reached the
  behavioural / time-series analytics. The upload path now rebuilds it like the
  server path.
- **The i18n lock recovers from poison instead of aborting the daemon.** It was
  the lone lock in the web layer that propagated a poisoned lock via `expect()`;
  under `panic = "abort"` that would take the daemon down. It now recovers the
  guard like every other lock in the crate.

### Changed

- **Internal module-structure cleanup (no behaviour change).** Several oversized
  files were split into focused submodules behind unchanged public paths: the
  1319-line `capture.rs` supervisor, the detection daemon (into process and
  run-loop submodules), `detections.rs` (by query concern), `viz.rs` (chart
  renderers by visual family), `accounts.rs` (by store), and the version logic
  in `auto_update`. The whole tree is now `cargo fmt`-clean.

## [0.7.1] - 2026-06-05

### Fixed

- **Imported history now reaches the behavioural analytics with its original
  timestamps.** A BirdNET-Pi import writes back-dated detections straight to
  SQLite, but the DuckDB analytics copy only ever synced *incrementally* (rows
  newer than the latest already synced) and was never refreshed after an import —
  so a year of imported history was silently invisible to the behavioural and
  time-series dashboards. The import now rebuilds the DuckDB copy in full once the
  rows land, and the migration progress UI shows the "Rebuilding analytics…" step.
- **The confidence threshold is no longer advertised at one value and enforced at
  another.** The detection daemon defaulted to recording everything ≥ 0.25 while
  the settings form displayed 0.70, so a stock station recorded far more than the
  operator believed. Both now read a single shared default (0.7, matching
  BirdNET-Pi), and the installer's documented default matches.
- **The System page disk panel shows real filesystem usage.** It previously
  reported only the database file's size; it now reports actual used/free space
  for the data filesystem (with a "running low" / "critically low" note) — the
  metric that determines whether recording will run out of room.
- **CPU temperature now reads on a Raspberry Pi.** `sysinfo`'s component sensors
  are routinely empty on a Pi; the System page now falls back to the Linux
  thermal-zone sysfs (`/sys/class/thermal`), preferring the CPU/SoC zone.
- **The dashboard "live signal" is honest.** The idle state no longer animates a
  synthetic sine wave that could be mistaken for live audio — it draws a flat
  baseline, and the indicator reads "live" only while genuine spectrogram frames
  are arriving from the capture device, "idle" otherwise.
- **First-run setup no longer offers a lockout footgun.** The interactive
  installer dropped the "Restrict the dashboard to THIS device only?" prompt that
  could strand a non-technical operator on localhost; the restriction remains an
  explicit, advanced `BIRDNET_LISTEN=127.0.0.1:8502` knob.

### Added

- **Multi-stream source attribution.** Every detection is now tagged with a
  first-class `Source` (the RTSP stream id, e.g. `cam1`, or `local` for the
  on-board mic; migration 18, indexed). Non-destructive — historical / imported
  rows stay `NULL` and nothing is rewritten. The detection-detail page uses it
  for **"also heard by"** corroboration: when other audio sources detected the
  same species at nearly the same time, they're listed as confirmation the
  detection is real (a read-only view; it never merges or hides rows). Single-mic
  stations see no change. Groundwork and the corroboration-first design for
  optional cross-stream collapse are in `docs/MULTISTREAM_DEDUP.md`.
- **A pre-warmed query cache for the heavy analytics.** A short-TTL in-memory
  cache now backs the heaviest fragments on the Heatmap, Migration/phenology,
  Co-occurrence, and Time-series (DuckDB) pages, and a background task pre-warms
  the default views shortly after startup and every few minutes after — so jumping
  between analytics pages is snappy on a Raspberry Pi 4 instead of re-running
  multi-second aggregate scans on every visit. Live surfaces (the detection feed
  and stat tiles) stay uncached and real-time.
- **BirdNET-Pi-style "Best recordings" on the dashboard.** A new at-a-glance card
  shows the day's highest-confidence detections that have a playable clip, so the
  best captures are one glance away instead of a hunt through the recordings
  browser.
- **A composite `(Date, Com_Name)` index** so the per-species date-range
  aggregates (sparklines, phenology, co-occurrence) are index-range scans rather
  than full-table scans.
- **A scannable QR of the dashboard URL** in `install.sh` and `quickstart.sh`, so
  a phone can open the station without anyone typing an IP (best-effort via
  `qrencode`).

### Changed

- **The post-install URL is IP-first.** Both installers now lead with the LAN IP
  (which always resolves on the network) and demote the mDNS `.local` name to a
  clearly-captioned secondary — mDNS is not universal, and leading with it could
  leave a phone unable to open the page.
- **`sysinfo` 0.39.2 → 0.39.3** for a Linux fix that hardens process-information
  retrieval when a process exits mid-refresh (supersedes Dependabot #130).
- **The dawn-chorus query is no longer N+1**: the top species' hourly histograms
  are fetched in a single grouped scan instead of one query per species.

## [0.7.0] - 2026-06-04

### Added

- **`--doctor` now checks the analytics preconditions.** The diagnostic gained
  an "Analytics (behavioral)" check that reports, with an actionable fix,
  whether behavioral analytics will actually work on this install: it **warns**
  when an analytics database is configured but the binary was built without
  analytics (a slim build pointed at a release config — the dashboards would
  silently stay empty), notes when analytics is explicitly disabled, and
  otherwise confirms analytics is enabled and that its DuckDB directory is
  writable. It deliberately opens no DuckDB during the preflight, so it adds no
  startup contention when the unit runs `--doctor` as `ExecStartPre`.
- **Offline / air-gapped install.** `install.sh` can now install from a release
  tarball already on disk — `BIRDNET_BINARY_TARBALL=/path/to/…tar.gz sudo -E
  bash install.sh` — skipping the GitHub fetch and checksum round-trip for a
  local file the operator placed themselves. Paired with `BIRDNET_SKIP_MODEL=1`
  (stage the ~541 MB model out-of-band), a station with no internet can be
  installed end to end. The installer also **degrades gracefully without
  systemd** (containers, chroots, staged images): it writes the binary, config,
  and unit file, then prints how to enable the service on a real host instead of
  aborting at the first `systemctl` call.
- **Install smoke test in CI** (`.github/workflows/install-smoke.yml`). On every
  change to the installer or the binary, CI builds the binary, then runs the
  *real* `install.sh` against it in a clean, network-less, no-systemd
  `ubuntu:24.04` container (via the new air-gapped path) and asserts the install
  completes and the dashboard actually serves (`/api/v2/health` reports
  `healthy`, `/` returns 200). This catches the class of regression that ships
  green unit tests but a broken operator install.

### Changed

- **Network retries now use jittered, capped, overflow-safe backoff.** The
  BirdWeather and Apprise clients retried transient failures on a fixed
  `2^attempt` schedule, so concurrent retries — and many stations posting on the
  same cadence — would wake in lockstep and hammer a recovering endpoint (a
  thundering herd). Both now share a backoff helper that adds **equal jitter**
  (each retry lands in a window rather than at one instant), **caps** the delay
  at 32 s so a long outage settles at a steady cadence, and is **overflow-safe**
  regardless of the attempt count.
- **The admin panel now renders entirely through one shared shell.** Six admin
  pages — Overview, Settings, Audio (already), Migration, Rules, System, and
  Notifications — each shipped (or, for the nav tabs, several still shipped)
  their own standalone HTML document with a bespoke top `<nav>` that disagreed
  with the admin shell's nav and with each other. **Every admin nav destination**
  now renders through the shared `admin_shell`, whose navigation is generated
  from a **single admin-nav manifest** (`routes/admin/nav.rs`) — so they show the
  same tabs with consistent active-state, gain a breadcrumb trail, and pick up
  the command palette / help drawer / toast region. The Migration tab, which was
  missing from the shell nav, is now part of the manifest. A parity test
  (`admin_router_serves_every_nav_destination`) guards that every admin nav
  destination resolves to a real route, and a runtime test
  (`folded_pages_render_through_the_shared_shell`) confirms each folded page
  actually composes the shell — mirroring `cmdk_covers_every_nav_destination`
  for the main nav.
- **Species management is now a first-class admin tab, and the admin sub-pages
  follow the standard "sense of place" pattern.** Managing which birds are
  detected/excluded is core to running a station, so **Species** is now its own
  admin nav tab rather than a quick-link a non-technical operator has to hunt
  for. The remaining sub-pages — the species **Filter test**, **Test
  notifications**, and the **Species images** blacklist — now render through the
  shared shell too: each highlights its **parent tab** (Species or Notifications)
  and shows a breadcrumb down to itself (`Home › Admin › <Parent> › <page>`), so
  you always know where you are and have a one-click way back. No admin page
  ships bespoke chrome any more.

### Fixed

- **The installer's completion summary shows the real dashboard port.** When an
  operator set a custom `BIRDNET_LISTEN` (e.g. `…:8599`), the post-install
  summary still printed the URL with the hardcoded `:8502`. It now derives the
  port from the configured listen address.
- **Installation input is now respected in the web UI.** The installer writes
  station settings (latitude/longitude, audio device, station name, …) to
  `/etc/birdnet/birdnet.conf`, and the Docker image passes them as `BIRDNET_*`
  environment variables — but the admin settings form and the first-run
  onboarding check read only the SQLite `settings` table, so a fully-configured
  station showed blank fields and was bounced to the onboarding wizard it had
  already effectively completed. The installed configuration (file **and**
  env/flags) is now seeded into the `settings` table on first start — insert-only,
  so a value the operator later changes in the UI is never overwritten — and a
  station that already has coordinates is no longer redirected to onboarding.
- **The "More" navigation menu no longer renders as overlapping/garbled text.**
  The topnav dropdown and the mobile bottom sheet both ship a `data-open-more`
  opener, and each opener's script selected the *first* one in the DOM — so the
  topnav button opened **both** menus at once (stacked on top of each other) and
  the mobile button opened none. Each opener is now scoped to its own dialog via
  `aria-controls`.
- **The Admin → Settings "saved" confirmation no longer renders a full-screen
  checkmark.** The success icon referenced utility classes that don't exist in
  the hand-written stylesheet, so the SVG rendered unconstrained; it now carries
  an explicit 16×16 size.
- **Live audio is reachable from the navigation.** The `/listen` page (per-source
  playback + live spectrogram + a live detection trickle) is now linked from the
  "More" menu, the mobile sheet, and the Audio settings section — so confirming a
  microphone is working no longer requires typing the URL by hand.
- **The installer falls back to Zenodo immediately when the GitHub model release
  is absent.** The ~541 MB model fetch no longer retries a definitive `404` five
  times with back-off before trying the next source; a missing GitHub asset now
  falls through to Zenodo at once, matching the labels fetch and the Docker
  entrypoint.
- **Importing a real BirdNET-Pi database works again.** The upload endpoint
  inherited axum's default 2 MiB request-body limit, so any real `birds.db`
  (tens to hundreds of MB, sometimes several GB) was rejected before the importer
  ever ran — the import feature was effectively dead. The DB-upload route now
  accepts large files (admin-only) **and streams the upload straight to disk**
  rather than buffering it (twice) in memory: a 163 MB upload now adds ~7 MB to
  peak RSS instead of ~330 MB, so a multi-hundred-MB database imports with flat
  memory instead of OOM-ing a Raspberry Pi. (For a database already on the Pi,
  the "Server Path" tab imports it with no upload at all.)
- **An RTSP source's transport (TCP/UDP/Auto) is now honoured.** The per-source
  transport the admin UI exposes was silently dropped and ffmpeg was always
  forced to TCP, so a camera that only speaks UDP could never be captured. The
  choice now reaches the capture command (`Auto` keeps the TCP default).
- **A per-source capture gain (`gain_db`) is now applied.** The gain the admin
  UI stores and displays for each source had no effect on capture. A non-zero
  gain now routes that source through `ffmpeg`'s `volume` filter
  (`-af volume=<n>dB`) — for a local microphone this switches it from `arecord`
  to `ffmpeg -f alsa`, since `arecord` has no software-gain control; unity-gain
  microphones stay on the lighter `arecord` path unchanged. A negative value
  cuts the level just as a positive one boosts it.
- **A per-source quiet window (`schedule_quiet`) is now enforced.** The quiet
  window stored per source was previously inert. The capture supervisor now
  pauses a source while the wall clock is inside its window and resumes it
  afterwards, on top of the global recording schedule (the source records only
  when the schedule allows it **and** it is outside its quiet window). The
  window uses the same clock basis as the recording schedule (UTC), wraps past
  midnight (e.g. `22:00`–`06:00`), and — like the schedule — is not enforced
  while the clock looks unsynced, so a bogus boot-time date can't silence a
  source. Editing gain or the quiet window takes effect on the next service
  restart, consistent with the other per-source settings. See
  `docs/FIELD_DEPLOYMENT.md` § 7 for the manual hardware-verification steps.
- **Multiple RTSP streams can be configured from the config file.** A new
  comma-separated `RTSP_URLS` config key drives several RTSP captures without
  the `--rtsp-urls` flag, and a multi-stream station no longer mislabels its
  first stream `rtsp` (every stream is numbered `RTSP_1`, `RTSP_2`, … once there
  is more than one).
- **Restoring a backup works for real archives.** `/admin/system/restore` had the
  same flaw as the import — it inherited the 2 MiB body limit and buffered the
  whole `.tar.gz` in memory — so restoring any real backup (database + recordings,
  often several GB) was rejected or OOM-ed the process. It now streams the upload
  to disk and lifts the limit on that admin-only route.
- **The system-status panel no longer blocks the async runtime.**
  `/admin/system/service/status` read `/proc` and spawned `getconf` / `systemctl`
  synchronously inside the request handler; that work now runs on a blocking
  thread so a slow `/proc` or a hung `systemctl` can't stall unrelated requests.
- **Navigation is consolidated and consistent.** The desktop top-nav, the "More"
  dropdown, the mobile tab bar + sheet, the breadcrumb trail, and the ⌘K command
  palette were separately hand-maintained lists that had drifted: `/live` was an
  orphan reachable from no menu, the mobile sheet was missing `/kiosk` and
  `/help`, `/analytics` was absent from mobile entirely, and seven pages
  highlighted the wrong section. They now all derive from — or are parity-tested
  against — a single navigation manifest. Added **breadcrumbs** on secondary
  pages (there were none), grouped the previously-flat mobile sheet, corrected the
  seven active-state mismatches, and redirected the orphaned `/live` to the
  maintained `/listen`.

### CI

- **CI now proves the behavioral extension loads with no network.** Analytics
  ships bundled — the release binary embeds the community `behavioral` extension
  so `LOAD behavioral` works offline on a fresh, air-gapped install — but the
  test that proves it (`embedded_extension_loads_when_bundled`) previously
  *skipped* in CI because no extension was embedded in the test build. The
  `--all-features` test job now fetches and embeds the extension first (the same
  mechanism release.yml uses), so the test runs its real assertion — loading the
  extension from the embedded bytes via a temp file with no network — and a
  dedicated step surfaces the result. Best-effort: if the registry is
  unreachable the test skips as before, adding no flakiness.
- **The mutation-testing job timeout is now matrix-driven.** The three
  binary-crate shards (`src/daemon/`, `src/capture/supervisor.rs`,
  `src/capture/schedule.rs`) rebuild the binary + web tree per mutant and were
  being `cancelled` at the flat 45-minute limit on cold caches. The job now uses
  `timeout-minutes: ${{ matrix.timeout_minutes || 45 }}` and those three rows set
  `timeout_minutes: 90`, so they report `success` instead of `cancelled`.

## [0.6.0] - 2026-06-03

The largest release since the first public one. BirdNet-Behavior gets a
ground-up dashboard redesign, **DuckDB behavioral analytics on by default**, a
real first-run onboarding wizard, account-based authentication, and a
fully self-contained, offline-capable install — the binary, the ~541 MB
BirdNET+ model, and the operator manual all come from a single GitHub origin,
checksum-verified. The release/CI pipeline is hardened end to end (the
integration branch is now gated, the auto-updater verifies what it installs,
and there are full-pipeline, migration, and soak tests). New schema migrations
(audio sources, accounts/sessions) run automatically and idempotently on first
start — no manual steps.

### Added

- **A ground-up dashboard redesign.** 20+ server-rendered HTMX pages on a
  unified design system: OKLCH color tokens, first-class dark/light and
  reduced-motion support, self-hosted fonts, and SVG-rendered visualizations.
  New surfaces include a command palette, a live homepage spectrogram fed by a
  WebSocket producer, a `/listen` page wiring per-source audio + spectrogram, a
  polar dawn-chorus moon-phase ring, an in-app help drawer, and an
  `/admin/audit` log with date-range and action filters.
- **DuckDB behavioral analytics on by default.** The analytics engine
  (sessionize, retention, funnel, sequence, next-species) is compiled into
  every binary *and enabled out of the box*. The community `behavioral` DuckDB
  extension is embedded into the release binary at build time, so analytics
  work fully offline on first run with no network `INSTALL`.
- **Multi-source audio capture.** Audio sources are now first-class,
  CRUD-managed rows (ALSA / PipeWire / RTSP / multiple RTSP), seeded from the
  CLI and config; the capture pipeline, `/listen`, and the metrics gauges all
  read from them, retiring the legacy single-string source.
- **Account-based authentication.** argon2id password hashing with cookie
  sessions and a CSRF guard, role-based access control enforced on every
  `/admin` write, an admin password reset, and session pruning. The legacy
  HTTP Basic Auth path is removed.
- **A real first-run onboarding wizard.** It persists location, timezone, and
  notification settings and redirects a fresh station to `/onboarding`, with an
  IP-geolocation auto-detect that fills latitude/longitude and the IANA
  timezone. A new doctor clock/timezone check surfaces an unset or unsynced
  system clock in plain language.
- **`doctor --fix` self-heal.** Safe, idempotent repairs (recreating missing
  configured directories — the #1 "service runs but records nothing" cause)
  run before the diagnostic, as the unprivileged service user.
- **Offline-capable model + manual bundling.** The ~541 MB BirdNET+ V3.0 model
  and labels are now a single shared, arch-independent GitHub release asset
  (`models-v3.0-preview3`), fetched from the same origin as the binary,
  **verified against a pinned sha256**, resumable, and falling back to Zenodo
  (the upstream source) when unavailable — so a fresh install needs one network
  origin and is offline-capable afterwards. A `publish-model.yml` workflow
  mirrors the model with checksum-pinned provenance (SHA256SUMS + SLSA).
- **An embedded operator manual at `/help`.** The mdBook manual is rendered at
  build time and shipped both in the Docker image and the install tarball
  (screenshots downscaled for the bundle; the committed source and the GitHub
  Pages site stay full-res), served offline at `/help`. The in-app help links
  are wired across 19 screens.
- **A hardened release & test pipeline.** CI now gates the integration branch
  (`claude/**` PRs run fmt, clippy, tests, rustdoc, MSRV, and an aarch64
  cross-check); a full-pipeline E2E test (audio → infer → DB → web), a
  BirdNET-Pi migration integration test, and a compressed soak/longevity test
  assert bounded memory/fd/DB growth. A deterministic demo-data seeder feeds a
  refreshed 48-image screenshot set.
- **Weather polling** — records conditions alongside detections; off by default.

### Changed

- **Content-Security-Policy hardened.** `script-src` is now a per-request nonce
  plus `strict-dynamic`; every inline `on*` handler moved to
  `addEventListener`; and `style-src 'unsafe-inline'` is dropped — the entire
  template surface was swept off inline styles onto utility classes, guarded by
  an inline-style regression test.
- **The auto-updater now verifies what it installs.** The downloaded archive is
  sha256-checked against the release `SHA256SUMS` and the staged binary is
  smoke-tested (`<binary> --version`) *before* the atomic swap; a wrong-arch,
  truncated, or corrupt download is rejected and the running binary is left
  untouched. (SLSA provenance remains the out-of-band authenticity path.)
- Settings accept locale-tolerant decimals and skip unchanged fields on save.

### Fixed

- **`/help` deep links no longer 404.** mdBook emits `<page>.html`, but the
  in-app help links use clean, extensionless URLs; a small middleware now
  rewrites `/help/…` to the rendered `.html` before serving, while `/help/`
  and static assets pass through.
- **The Docker image builds again and ships correct analytics.** `CHANGELOG.md`
  is kept in the build context (it is embedded into the binary at compile
  time), and each architecture embeds its matching DuckDB `behavioral`
  extension instead of defaulting to the amd64 build.
- Wikipedia species images are fetched on cache-miss, and the admin image
  blacklist is enforced on the serve path.

### Security

- CSP per-request nonce + `strict-dynamic`, with no inline script or style.
- Admin actions require an authenticated session with the right role (RBAC);
  passwords are argon2id-hashed; a stateless CSRF guard covers state changes.
- The auto-updater and the bundled model are both integrity-verified
  (sha256) against a provenance-attested origin before anything touches disk.

## [0.5.3] - 2026-05-27

Field-hardening release from real Raspberry Pi + RTSP testing. The service now
starts and shuts down cleanly, RTSP stations actually record detections, the
dashboard is reachable on the LAN with only its admin panel behind a password,
and `install.sh` gains guided repair/update/reinstall/uninstall flows with
pre-flight and post-install validation. No database migration is required.

### Fixed

- **The systemd service no longer fails to start with
  `Failed to set up mount namespacing: /tmp/birdnet-stream: No such file or directory`
  (exit `226/NAMESPACE`).** The unit listed the tmpfs stream directory in
  `ReadWritePaths=` while also setting `PrivateTmp=yes`; systemd mounts a fresh
  empty `/tmp` for the service, so bind-mounting a path *beneath* it fails
  namespace setup and the service never starts. The stream dir is removed from
  `ReadWritePaths=` (the private `/tmp` is already writable) and an
  `ExecStartPre=/bin/mkdir -p` recreates it on every start. Existing broken
  installs are fixed by `sudo bash install.sh repair` (or any update/reinstall).
- **The detection daemon creates its watch directory before attaching the file
  watcher.** With `PrivateTmp=yes` the service's `/tmp` is wiped on every
  restart, so `start_detection_daemon` now `create_dir_all`s the watch dir
  up front — a missing directory previously made `notify` error out and
  silently disabled detection (web UI up, nothing analysed).
- **The service shuts down promptly instead of hanging until SIGKILL.** A live
  WebSocket/event-stream client (the dashboard keeps one open) kept axum's
  graceful shutdown from ever completing, so `stop`/`restart`/uninstall blocked
  until systemd SIGKILLed the process at `TimeoutStopSec` (30 s) and left a
  ghost `Active: failed (timeout)`. Shutdown now caps the connection drain
  (`SHUTDOWN_GRACE`, 10 s) and signals the detection loop to stop so the runtime
  winds down cleanly.
- **`install.sh uninstall` is clean, idempotent, and fool-proof.** It now runs
  `systemctl reset-failed` so the removed unit no longer lingers as
  `Active: failed (timeout)` in `systemctl status`, reports accurately what was
  (or wasn't) present, can also delete data/config (interactive prompt or
  `BIRDNET_PURGE=1`) behind a path-safety guard, and verifies at the end that no
  service or binary remains. Re-running it when nothing is installed is a clean
  no-op.
- **`uninstall.sh --purge` renders its plan correctly and guides recovery.** It
  printed literal `\033[1m…` escape codes (colours are now real ESC bytes); and
  when the config and service are already gone, the guessed-data-dir guard now
  prints the exact `--data-dir` argument to re-run with.
- **RTSP/segmented captures no longer fail with `decode error: ... unexpected
  end of file`.** The watcher decoded each clip on every create/modify event,
  so an ffmpeg segment still being written (RTSP captures a clip in place over
  ~15 s) was decoded while incomplete and reprocessed on every write — meaning
  **zero detections** for RTSP stations. The daemon now debounces: a file is
  decoded once its size has been stable for a short settle window, and exactly
  once.

### Added

- **`install.sh` commands and an existing-install menu.** Running the installer
  on a machine that already has BirdNet-Behavior now offers **update**,
  **repair**, **reinstall**, and **uninstall** (interactively), or you can pass
  one explicitly (`sudo bash install.sh repair`). Non-interactive runs keep the
  historical auto-update behaviour. `repair` re-creates directories, fixes
  ownership/permissions, rewrites the systemd unit, and restarts — without
  re-downloading the binary or model.
- **Pre-flight and post-install validation in `install.sh`.** Before downloading
  it checks for required tools and sufficient free disk; afterwards it validates
  the binary runs, the unit verifies (`systemd-analyze verify`), directories are
  owned by the service user, the config is readable by the daemon, the doctor
  preflight passes, and the web port is listening.
- **`install.sh` ensures the ffmpeg capture backend for RTSP stations.** When the
  config has an `RTSP_URL` (which captures through ffmpeg), install/repair now
  install ffmpeg automatically (`apt-get`), or warn with the exact command if it
  can't — previously an RTSP station with no ffmpeg passed the installer but the
  daemon then failed the doctor preflight and never started.

- **The dashboard bind address persists across installer re-runs.** `repair`
  and `update` no longer silently re-hide a LAN-exposed dashboard on localhost:
  the bind address is read from `BIRDNET_LISTEN` (env or the config file) and,
  failing that, carried forward from the existing service unit. A fresh install
  records it as `BIRDNET_LISTEN=` in the config so it is visible and editable.

### Changed

- **The dashboard is reachable on the LAN out of the box, with the admin panel
  gated by a password.** The default bind is now `0.0.0.0:8502` (was
  `127.0.0.1:8502`, which left non-technical users at "connection refused").
  Only the `/admin` panel — settings, software update, system controls — now
  requires HTTP Basic Auth (route-level, enforced by the binary); viewing the
  dashboard is open. A fresh install **auto-generates a strong admin password**
  (user `birdnet`, shown in the post-install summary and saved as `CADDY_PWD`),
  so the admin surface is protected by default. Restrict the whole dashboard to
  this host again with `BIRDNET_LISTEN=127.0.0.1:8502` (env, config, or the
  interactive prompt).
- **`install.sh` is now assembled from single-responsibility modules under
  `installer/lib/*.sh` by `installer/build.sh`** (developer-facing only — the
  shipped `install.sh` is still one self-contained, checksummed file). A CI gate
  and pre-commit hook verify the generated `install.sh` stays in sync with its
  modules.

## [0.5.2] - 2026-05-27

Installer- and documentation-focused release: it repairs the bare-metal install
flow on Raspberry Pi OS Trixie, adds guided onboarding, and tightens the install
to least privilege. There are no functional changes to the compiled binary —
only its reported version differs from 0.5.1.

### Added

- **Guided onboarding in `install.sh`.** A fresh interactive install now prompts
  for an audio source (auto-detected ALSA device, a typed ALSA device, or an
  RTSP URL), station latitude/longitude, and whether to expose the dashboard to
  the LAN — writing them into the config so a non-technical user gets a working
  station without hand-editing a file, and the post-install summary says exactly
  which URL to open in a web browser (and from which device). Prompts read from
  `/dev/tty`, so they work under `curl … | sudo bash`. `--noninteractive` (or
  `BIRDNET_NONINTERACTIVE=1`) keeps unattended installs silent.
- **`install.sh --version X.Y.Z` / `-v`** to pin a release through the pipe form
  (`curl … | sudo bash -s -- --version X.Y.Z`); the `VERSION` environment
  variable still works.

### Security

- **The web dashboard binds `127.0.0.1` by default** instead of `0.0.0.0`. The
  admin UI can change settings and update software, so it is no longer exposed to
  the whole LAN unauthenticated out of the box. The interactive installer offers
  LAN exposure and captures a password (HTTP basic auth) when you opt in; the
  bind is overridable with `BIRDNET_LISTEN`.
- **`/etc/birdnet/birdnet.conf` is now `0640 root:<service-group>`** (was
  world-readable `0644`), so secrets such as `CADDY_PWD` and `BIRDWEATHER_TOKEN`
  aren't readable by other local users; existing configs are retightened on
  upgrade.
- **Tighter filesystem and service sandboxing.** Data, recordings, model, and
  tmpfs-stream directories are `0750` (were `0755`); the systemd unit adds
  `CapabilityBoundingSet=` (all dropped), `UMask=0027`, and
  `RestrictAddressFamilies=`. Measured `systemd-analyze security` exposure
  dropped from 4.0 to 1.6.

### Fixed

- **Bare-metal install over `sudo` now works end to end on Raspberry Pi OS
  Trixie:**
  - Version pinning no longer needs the broken `sudo bash <(curl …)` form
    (process substitution + `sudo` closes the pipe's file descriptor, so the
    script vanished); docs and generated release notes use the pipe form.
  - The resolved version is no longer corrupted by an `[INFO]` log line bleeding
    into the captured value (which produced `curl: (3) bad range in URL`) — the
    log helpers now write to stderr.
  - The data directory is created under the service user's real home instead of
    `/root` (where `sudo` pointed `$HOME`), so the non-root service can reach its
    database, recordings, and model.
  - ALSA microphone auto-detection no longer fails with `awk: syntax error` on
    Debian / Raspberry Pi OS (replaced a gawk-only `match()` form with a portable
    one).

### Changed

- **CI:** the `Tests (x86_64)` job frees ~25–30 GB of preinstalled SDKs before
  the all-features build, fixing intermittent `No space left on device` failures.

## [0.5.1] - 2026-05-26

### Added

- **CI now compiles and tests the `analytics` feature** (clippy, tests, MSRV
  check, and rustdoc) and adds an **aarch64 (Raspberry Pi) cross-check** on
  every PR — closing the blind spot that let analytics bugs ship undetected.
- **`/api/v2/health` reports `detection_daemon`** (`running`/`stopped`), so
  monitoring can tell a capturing station from one running web-only or with a
  misconfigured model/labels/watch-dir.
- **`BIRDNET_CORS_ALLOWED_ORIGINS`** to allow specific cross-origin origins.
- **`docs/SECURITY_HARDENING.md`** — a deployment hardening guide (network
  exposure, authentication, CORS, privacy, backups, and release verification).

### Changed

- **Configuration is validated at startup**; the daemon now refuses to start on
  an invalid setting (e.g. a latitude outside ±90, a malformed
  `RECORDING_SCHEDULE`) instead of running silently degraded.
- **Database migrations are atomic** — each migration's schema change and its
  version bump commit in one transaction — and a migration failure is now fatal
  at startup rather than serving an under-migrated schema.
- **The detection-event channel is bounded**, so a stalled consumer applies
  backpressure (tripping the systemd watchdog) instead of buffering until the
  process is OOM-killed; the `--process-existing` backlog now runs after the
  server signals readiness.
- **Routine dependency and CI-action updates** — `rusqlite` 0.39 → 0.40
  (pulling `libsqlite3-sys` 0.38), `reqwest` 0.13.3 → 0.13.4, and
  `codecov/codecov-action` v5 → v6.

### Fixed

- **Capture-subprocess stderr is drained to the log**, fixing a slow
  pipe-buffer stall that could silently stop `arecord`/`ffmpeg` audio while the
  process still appeared alive — and surfacing the subprocess's own errors for
  field debugging.
- **`BNB_BASE_URL` defaults to the server's own port** (`:8502`, was `:8080`)
  for RSS/iCal feeds and share links.
- **Documentation drift**: corrected the `/api/v2/health` response example, the
  `.env.example` image-tag note (analytics is built into *every* image, no
  separate tag), stale version pins, the feed-default port, and minor wording.
- **Release attestation no longer aborts the publish pipeline.** The SBOM
  summary step assumed the CycloneDX 1.5 `metadata.tools` object shape while
  cargo-cyclonedx emits the legacy array, so `jq` errored and its non-zero exit
  killed the `package` job before the SLSA build-provenance attestation and
  artifact upload could run. The summary now tolerates both shapes.

### Security

- **CORS is same-origin by default** — the API no longer emits a wildcard
  `Access-Control-Allow-Origin`, so a site you visit can't read the station
  over the LAN. Opt specific origins back in with `BIRDNET_CORS_ALLOWED_ORIGINS`.
- **5xx API responses no longer leak internal error strings** (DB/SQL detail);
  the detail is logged server-side and a generic message is returned.
- **HTTP Basic Auth (`CADDY_PWD`/`CADDY_USER`) is now read from the
  environment** as well as `birdnet.conf`, so it can be enabled under Docker;
  the server logs a prominent warning when bound to a non-loopback address with
  no password set.

## [0.5.0] - 2026-05-26

### Added

- **Dawn-chorus pattern matching — `GET /api/v2/analytics/patterns`.** The
  previously-stubbed endpoint is implemented on the behavioral extension's
  `sequence_match`, reporting per day whether a configured species sequence was
  detected in order (optionally within a maximum gap between consecutive steps).

### Changed

- **Bundled DuckDB upgraded 1.5.1 → 1.5.3** to match the published `behavioral`
  community extension (v0.6.0), which targets DuckDB 1.5.3. The bump is gated on
  the CDN actually serving a 1.5.3-built extension that `LOAD`s on the bundled
  engine — verified, not assumed from an HTTP 200.

### Fixed

- **Behavioral analytics were built against assumed extension signatures and had
  never executed** (the extension could not `LOAD`, and CI does not exercise the
  `analytics` feature), so every query was malformed against the real extension.
  All builders are corrected and now verified end-to-end against the published
  extension on DuckDB 1.5.3:
  - `sessionize` materialises the window-function session id in a subquery
    before aggregating (a window expression cannot appear in `GROUP BY`).
  - `retention` uses the real `retention(BOOLEAN, …) -> BOOLEAN[]` aggregate
    over per-species detection-day cohorts, replacing a non-existent
    `retention(date, int[])` form.
  - `window_funnel` passes step conditions as variadic booleans, not an array.
  - `sequence_next_node` uses the real
    `(direction, mode, timestamp, value, base_cond, …)` signature.

## [0.4.0] - 2026-05-25

### Added

- **`--refresh-extension`** — a maintenance command that force-reinstalls the
  latest `behavioral` DuckDB extension for the bundled DuckDB version, loads it
  to verify, and exits. Useful for recovering a corrupted extension cache.
  Requires `--analytics-db` (or `ANALYTICS_DB_PATH`) and network access.
- The bundled DuckDB version and the loaded `behavioral` extension version are
  logged at startup, so it is clear which analytics engine and extension build
  a station is running.

### Fixed

- **Behavioral analytics (sessionize, retention, funnel, next-species) failed to
  load.** DuckDB version-locks its extensions, but the bundled engine had
  drifted to DuckDB 1.5.3 while the published `behavioral` community extension
  targets 1.5.1, so `LOAD behavioral` was rejected and the extension-backed
  analytics were unavailable. The bundled DuckDB is now pinned to 1.5.1 to match
  the published extension.

### Changed

- Routine dependency and CI-action updates.

## [0.3.0] - 2026-05-24

### Added

- **Migration & phenology page (`/migration`).** A per-species ridgeline
  ("joyplot") of weekly abundance for migratory species, with first-of-year
  arrivals, peak diversity week, earliest-vs-last-year, and "still expected"
  tiles — built entirely from the existing `detections` table.
- **Dawn-chorus page (`/analytics/dawn-chorus`).** A 24-hour polar clock of
  per-species activity with sunrise/sunset markers from the station
  coordinates (`BNB_STATION_LAT`/`BNB_STATION_LON`, falling back to
  `BIRDNET_LATITUDE`/`BIRDNET_LONGITUDE`).
- **Detection detail + public share links.** Every detection links to a detail
  page (spectrogram, audio, daemon correlation id) and can be shared via a
  signed, public `/r/<token>` page — HMAC-SHA256 over `(date, time, com_name,
  expiry)`, constant-time verify, 30-day expiry, filename-based audio/
  spectrogram redirects. Set `BNB_SHARE_SECRET` so links survive restarts
  (fail-secure random per-process secret otherwise).
- **RSS & iCal feeds.** `/feeds/rare.rss`, `/feeds/rare.ics`, and
  `/feeds/today.rss`, linking back to detection detail pages; the rare RSS feed
  is advertised via `<link rel="alternate">` in the dashboard head. Absolute
  links use `BNB_BASE_URL`.
- **Per-device display preferences** on `/system` — theme, density, motion and
  contrast, applied before first paint (no flash on reload).
- **Comparative "today" phrase**, **species-detail hero/status partials**,
  **illustrated empty states** across six surfaces, and a **print stylesheet**
  for the reports.
- **Detection-review triage (`/detection-reviews`).** A non-destructive
  confirm/reject verdict per detection, stored in a new `detection_reviews`
  table (migration 13). The triage page queues recent unreviewed detections
  with Confirm/Reject actions and lists recent verdicts; each detection-detail
  page gains a self-replacing review widget. Distinct from quarantine, which
  gates uncertain rows *out* of the log before they are admitted.
- **Share from the quarantine queue.** Every quarantine row gets a "Share"
  button issuing the same signed `/r/<token>` link as detection detail; the
  share page now falls back to the quarantine table so a pending rare bird (not
  yet in `detections`) still resolves.
- **`uninstall.sh`** — a safe, idempotent, deterministic uninstaller shipped
  beside the binary (and as a standalone release asset). Removes only the
  software by default (systemd service, tmpfs mount unit, binary) and keeps the
  database, recordings, settings, and model unless you opt in via `--purge` or
  granular `--remove-db` / `--remove-recordings` / `--remove-config` /
  `--remove-models` / `--remove-image-cache` flags. Auto-detects the real data
  directory from the installed config/service, refuses to touch protected
  paths, supports `--dry-run` and `--yes`, and handles the macOS launchd
  LaunchAgent. The doctor also now flags missing ffmpeg when a macOS mic
  (avfoundation) or RTSP source is configured, and its config-path hint is
  platform-aware.
- **`install.sh` is now OS-aware.** On macOS it dispatches (before any root
  check or filesystem change) to a per-user launchd path — offering to
  `brew install` ffmpeg/cmake, downloading the `aarch64-apple-darwin` build when
  a release publishes one (else offering to build in place when run from a
  checkout, or printing the source-build steps), and writing
  a starter config + LaunchAgent — instead of failing partway through the
  Linux/systemd flow. Runs without `sudo` on macOS. Also hardened `SERVICE_USER`
  resolution so a missing `$USER` no longer aborts the script under `set -u`.
- **macOS Apple Silicon runbook + Homebrew formula draft** —
  `packaging/macos/verify-macos.sh` (from-source build, doctor, boot, mic
  enumeration, manual TCC/launchd checklist) and a template
  `packaging/macos/birdnet-behavior.rb` pending a hardware-verified release.

### Fixed

- **Startup crash from a duplicate route.** The new `/migration` page and the
  heatmap page both registered `GET /pages/migration-ridgeline`; axum's
  `Router::merge` panicked at construction, so the server never started. The
  heatmap embed moved to `/pages/seasonal-phenology`, and a lib-level test now
  builds the full router so an overlapping route fails CI (the standard test
  job runs `--lib --bins`, which skips the integration tests that would have
  caught it).
- **Print stylesheet 404.** `/static/css/print.css` was linked but never served
  by the static router; `@media print` output was unstyled and every page
  logged a console error.
- **Broken "Species Accumulation" card** on `/timeseries` (pointed at a
  non-existent `/pages/ts-accumulation`) — now uses `/pages/life-accumulation`.
- **Migration page request flood.** A `hx-trigger="… every 1h"` poll was
  parsed by htmx as 1 ms (it understands `s`/`m` but not `h`), hammering
  `/pages/migration-stats`; changed to `every 60m`.
- **Species photos never loaded** — the gallery card and the detection-detail
  link used image URLs that matched no route; pointed both at
  `/api/v2/species/image/{name}/file`.
- **Placeholder copy + missing skip link** on the public share page.
- **Four phone-width (390px) horizontal overflows** — `/history`,
  `/admin/audio`, `/admin/settings`, and `/onboarding` had inline multi-column
  grids the global responsive rules couldn't reach; they now collapse to a
  single column at ≤520px (and the onboarding stepper drops its text labels).
- **Misleading analytics status.** `/analytics` reported "behavioral analytics
  are active" whenever a DuckDB database was connected, even when the
  `duckdb-behavioral` extension failed to load; the badge now states the
  extension is a separate requirement (which the per-feature cards report on).
- **Duplicate species-photo caching.** Gallery and species-detail keyed photos
  by common name while detection-detail used the scientific name, so the same
  bird was fetched and stored twice (and detection-detail's link often 404'd);
  all three now key by scientific name, with a paced gallery background warmer.
- **Unlogged time-series 500s.** Failed `/api/v2/timeseries/*` queries returned
  500 with the error only in the body; the error is now logged server-side.

### CI

- The Tests job now runs the `tests/` integration suite (`cargo test
  --workspace --tests`), including a new `boot_smoke.rs` that spawns the binary
  in `--web-only` mode and curls `GET /` — closing the gap that let a startup
  panic ship despite green CI.

## [0.2.0] - 2026-05-23

### Security

- **Response-hardening headers on every response.** A new
  `birdnet-web::security` middleware layer sets `Content-Security-Policy`
  (own-origin scripts/styles/`connect-src`; no off-origin script, object, or
  framing), `X-Content-Type-Options: nosniff`, `X-Frame-Options: SAMEORIGIN`,
  and `Referrer-Policy: strict-origin-when-cross-origin`. No HSTS — the binary
  serves plain HTTP and expects a reverse proxy to own TLS.
- **Stateless CSRF protection.** State-changing requests (`POST`/`PUT`/`PATCH`/
  `DELETE`) whose `Origin`/`Referer` authority does not match the request
  `Host` are rejected with `403`. The web UI uses HTTP Basic Auth with no
  sessions, so a same-origin check (rather than a per-form synchroniser token)
  is the appropriate CSRF defence; non-browser clients (the CLI, scripts,
  `curl`) that send neither header are unaffected.

### Added

#### Pre-release hardening for 0.2.0 (release pipeline, docs, web)

- **Analytics built in everywhere, on by default.** Release binaries are built
  with `--features analytics` (one binary, no separate archive), and the
  **Docker image is now a single variant** with analytics compiled in — the
  separate `-analytics` tag is gone. `install.sh` runs the service with
  `--analytics-db` and `docker-compose.yml` sets `BIRDNET_ANALYTICS_DB`, so
  behavioral analytics works out of the box with no extra build, flag, or tag.
  Disable on very low-RAM boards by removing the flag / unsetting the env var.
- **Keyless cosign signatures on the Docker images.** The `docker.yml` merge
  job signs each multi-arch manifest with the workflow's GitHub OIDC identity
  (Fulcio + Rekor), matching the SLSA build-provenance attestation already on
  the binaries. Verification recipe in `RELEASING.md` and the job summary.
- **Rehearsable releases.** A `workflow_dispatch` dry run on `release.yml`
  runs validate → ci → build → package → attest without publishing, so a
  release — including the DuckDB analytics cross-build — can be proven green
  before a tag is pushed.
- **mdBook link checking in CI.** `docs.yml` now runs `mdbook-linkcheck`; a
  broken internal documentation link fails the build.
- **Reconnecting live-detection stream client.** A self-contained
  `/static/live-detections.js` consumes the existing `/api/v2/ws/detections`
  WebSocket, surfaces a live/offline indicator, dispatches `birdnet:detection`
  events, and reconnects with exponential backoff + jitter (capped at 30 s),
  dropping the socket while the tab is hidden. All DOM writes use `textContent`
  (never `innerHTML`).
- **Friendly `404` page.** Unmatched URLs now render the branded app layout
  with a route back to the dashboard, replacing the previous empty response.
- **In-UI configuration diagnostics** at `/admin/doctor` (linked from the admin
  nav as *Diagnostics*). Re-reads the active config and renders the same
  range/consistency findings the CLI `--doctor` reports, reusing the canonical
  `birdnet_core::config::validate` so the two can't drift; points to the CLI
  doctor for audio/model/disk/network checks.
- **CLI-help docs drift-gate.** `scripts/gen-cli-help.sh` regenerates
  `docs/book/_generated/cli-help.txt` from the binary's `--help`, and CI fails
  if the committed copy is stale — so the documented flags/env vars/defaults
  stay in lockstep with `src/cli.rs`.
- **Accessibility.** Added an `.sr-only` visually-hidden utility and live-status
  indicator styling (the existing reduced-motion / focus-visible / chart-ARIA
  coverage was already in place).
- **Supported hardware/OS matrix** added prominently to the README and the
  book, making the glibc 2.39 floor, the Bookworm→Docker path, and the
  no-armv7 caveat unmissable.
- **Upgrade-safe installer.** Re-running `install.sh` stops the service before
  swapping the binary (avoiding `ETXTBSY`) and restarts it on the new version;
  data and config are preserved and schema migrations run on startup. The
  installer also refuses to run on glibc < 2.39 with an actionable message.
- **`RELEASING.md` rewritten** to match the real pipeline (two build targets,
  native GCC cross — not `cargo-zigbuild`, SBOM, cosign, dry run) with a
  copy-paste pre-release checklist and a "what is not automated" section.

#### Mutation testing extended to `src/daemon.rs` (item A1, PR #50 carryover)

- **`src/daemon.rs` brought to `missed = 0` cargo-mutants.** PR #50
  explicitly deferred this — the inline struct literals on
  `SpeciesFilterConfig` / `PipelineConfig` / `ModelConfig` /
  `ExtractionConfig` produced ~10 "delete field" mutants, and the
  three orchestrator functions (`start_detection_daemon`,
  `event_processor`, `dispatch_webhook`) had body-replacement
  mutants that no unit test could observe. This release:
  1. **Extracted four per-config builder helpers** —
     `build_pipeline_config`, `build_model_config`,
     `build_species_filter_config`, `build_extraction_config` —
     each pinned by a dedicated unit test covering every field
     individually so a "delete field" mutant on the struct literal
     surfaces as a failing assertion.
  2. **Extracted seven smaller pure helpers** to dissolve the
     remaining inline boundary / arithmetic / boolean mutations:
     `resolve_f32_with_default` (kills the
     `(cli - DEFAULT).abs() < f32::EPSILON` family by using
     bit-exact equality on the documented CLI default — same
     trick PR #50 used for `parse_search_term` /
     `strip_not_prefix`), `confidence_pct_trunc`,
     `confidence_pct_round`, `latency_ms_to_seconds`,
     `is_first_detection_today`, `passes_filter`,
     `should_dispatch_notification`, `species_thresholds_log_count`,
     `resolve_required_paths`, `extraction_output_dir`.
  3. **Refactored `dispatch_webhook`** to return
     `Result<u16, WebhookError>` and introduced `build_webhook_spec`
     + `WebhookSpec` + `WebhookMethod` to encapsulate the inline
     request-builder logic. The typed-error return makes the
     `replace dispatch_webhook with ()` mutant unviable, and the
     `build_webhook_spec` cells (`(GET, body)`, `(POST, body)`,
     `(POST, none)`, unknown-method fallback) are unit-tested.
  4. **Added two in-process integration tests** that catch the
     remaining `replace start_detection_daemon -> Option<...> with None`
     and `replace event_processor with ()` mutants:
     `start_detection_daemon_returns_some_with_valid_inputs` stands
     the daemon up against the tiny `tiny_v24_test.onnx` bundled at
     `crates/birdnet-core/src/testdata/`, in-memory `AppState`, and
     a tempdir watch dir; `event_processor_inserts_row_for_accepted_event`
     drops a fixture `DetectionEvent` through the channel and
     asserts the row lands in the DB (also pinning the migration-12
     correlation-id round trip end-to-end).
  5. **Mutation workflow updated** to include `src/daemon.rs` at
     `max_missed = 0` in the matrix. Path filter updated. The
     workflow's previous "deferred follow-up" note is replaced by
     a record of how the mutants were dissolved.

#### Web UI — `correlation_id` surfaced on detection-detail page (item A5)

- **`/detections/detail?date=...&time=...` now renders the per-row
  correlation id with a "Copy" affordance.** Migration 12 carries
  the daemon's per-file id to durable storage; the operator-facing
  detail page now closes the log → row traceability loop by
  rendering the id alongside a one-click "Copy" button and the
  exact `journalctl -u birdnet | grep <id>` command an admin would
  run to pull the decode/infer/notify slice that produced the row.
  Rows pre-dating migration 12 (BirdNET-Pi imports, quarantine-
  approve writes) render no card at all — no empty-state noise.
  Four new unit tests pin the empty/empty-string/non-empty/
  malicious-content escaping cases.

#### Test-fixture audit — last hand-coded `CREATE TABLE detections` removed (item F15)

- **`crates/birdnet-db/src/sqlite/queries/heatmap.rs` and
  `correlation.rs` test fixtures** were hand-coding a migration-1-
  shape `CREATE TABLE detections` block inside their `setup()`
  helpers. Both follow the exact anti-pattern PR #50 flagged on
  the `tests/web_api*.rs` files — the schema silently drifts the
  moment a new migration adds a column. Replaced both with
  `crate::migration::migrate(&conn)` so the canonical schema is
  always applied. Existing tests still pass; the
  birdnet-migrate crate's own CREATE TABLE blocks (which model
  BirdNET-Pi schemas, *not* our schema) are left alone.

#### Drift gate — `DETECTION_COLS` / `map_detection_row` / `DETECTION_COL_NAMES` (item F16)

- **`DETECTION_COL_NAMES` const list added** as a source-of-truth
  pair to the joined `DETECTION_COLS` string. Four new
  drift-gate tests pin the invariant: `DETECTION_COLS` must
  equal `DETECTION_COL_NAMES.join(", ")`, the projection's
  prepared-statement column count must match the names list, every
  name must resolve against the migrated `detections` schema, and
  `map_detection_row` must round-trip a real
  `DetectionRecord` insert. Migration 12 needed three coordinated
  edits across these three surfaces; the drift-gate tests turn
  the next missed edit into a unit-test failure with a directly-
  actionable message instead of the `"Invalid column type Text at
  index N"` runtime errors that ate half a day in the PR #35
  investigation.

#### Persistence — log-to-row traceability for detections (item C9)

- **Migration 12: `correlation_id TEXT` column on detections.** Closes
  the log→DB→UI traceability loop opened by PR #49. The daemon already
  stamps a short, sortable correlation id on every event for one audio
  file (`new_event_correlation_id` in `birdnet-core::detection::daemon`)
  and threads it through `decode → infer → notify → DB-write` logs;
  this migration carries that id to durable storage so an admin who
  clicks a suspicious row in the web UI can run
  `journalctl -u birdnet | grep <id>` to pull the exact decode/infer/
  notify slice that produced it. The column is NULLABLE so quarantine-
  approve and BirdNET-Pi-importer rows (which have no id to backfill)
  keep working unchanged, and the new `idx_detections_correlation_id`
  index makes "show every row from one file" cheap. `DetectionRecord`
  / `DetectionRow` gain a matching `correlation_id` field; the column
  is serialised on `/api/v2/detections` responses via
  `#[serde(skip_serializing_if = "Option::is_none")]` so historical
  rows don't accumulate a useless `"correlation_id": null` key.

#### Supply-chain — Software Bill of Materials at release (item D14)

- **CycloneDX SBOM attached to every GitHub release.** The release
  pipeline now installs `cargo-cyclonedx@0.5.7` (pinned for repro-
  ducibility), generates both CycloneDX 1.5 JSON and XML BOMs of the
  full workspace, and uploads `birdnet-behavior-<ver>-sbom.cdx.json`
  + `.cdx.xml` alongside the binaries. Both SBOM files are signed by
  the same SLSA build provenance attestation as the binaries and
  hashed in `SHA256SUMS`. Consumers can ingest them into
  Dependency-Track, GitHub Dependency Graph, or any CycloneDX-aware
  vulnerability scanner. The release notes template links to the
  files so operators don't have to dig through the artifact list.

#### Test coverage carryovers from PR #49 (item A1)

- **`src/helpers.rs` lifted from 0 % to ~95 % unit coverage.** Each
  config-and-state helper now has a dedicated test pinning the CLI →
  config → built-in-default precedence — `db_path_from_config`,
  `init_audio_source`, `init_site_name`, `init_i18n`, `init_image_cache`,
  `maybe_install_avahi_service`, `start_disk_manager`. The pattern
  uses `Cli::parse_from(["birdnet-behavior"])` for the "no flags"
  baseline and `Config::parse(...)` for hand-written config snippets,
  so the tests run without filesystem or network I/O. 21 new tests
  total. Closes carryover item A1.
- **`src/integrations.rs` lifted from 0 % to ~90 % unit coverage.**
  Every `create_*_client` and `create_notification_*` helper now has
  precedence tests covering "CLI wins", "config falls through", and
  "neither configured → None". Notable: the MQTT helper's
  `retain` / `port` / `topic_prefix` overrides are pinned per-field
  so a future config-key rename surfaces immediately, and the email
  notifier round-trips through a real settings table seeded via
  `birdnet_db::settings::set`. 32 new tests total.
- **`crates/birdnet-db/src/sqlite/queries/detections.rs` lifted from
  the 11-test smoke surface to a 34-test full-CRUD surface.** The
  remaining helpers — `delete_detection`, `relabel_detection`,
  `lock_detection`/`unlock_detection`/`is_detection_locked`,
  `locked_file_names`, `species_for_date`, `detection_dates`,
  `todays_detections{,_count}` (including the `NOT ` exclusion path
  and whitespace-search behaviour) — are now pinned by dedicated
  tests. The migration-11 chunked-recording contract (5 chunks per
  file each get a row) and the migration-12 correlation-id round
  trip are both regression-tested.
- **Six integration test fixtures fixed (`tests/web_api*.rs`).** Six
  test files had hand-coded `CREATE TABLE detections` declarations
  duplicating migration 1 — the exact anti-pattern ADR-16 flags as
  the source of three of the PR #35 production bugs. Each was rewriting
  the schema to the migration-1 shape on every test run, so the
  fixtures couldn't see any column added by migrations 2–12. Replaced
  with `birdnet_db::migration::migrate(&conn)` so the canonical schema
  is always applied. All 31 web-API integration tests pass on the new
  fixture.

#### Mutation testing matrix expanded (item A2, partial)

- **`crates/birdnet-db/src/sqlite/queries/detections.rs` added to the
  cargo-mutants matrix.** Runs as its own job with the same
  `missed = 0` gate that already pins `validate.rs`,
  `inference/model.rs`, and `extractor.rs`. The 30+ tests added in
  this PR (cover the full CRUD surface plus the migration-12
  correlation-id round trip) make every mutant observable. Path
  filter and PR/cron triggers updated to match. The workflow now
  supports a per-row `package` override so future non-`birdnet-core`
  files plug in cleanly.
- **`src/daemon.rs` deferred to a follow-up PR.** A dry run
  surfaced the right answer to the carryover plan's question: the
  extracted pure helpers (`decide_disposition`,
  `derive_source_label`) are mutation-clean *after* the boundary
  test fix (`<` → `<=` on a float-exact `0.5` rather than a
  non-representable `0.8`) that this PR adds. But the surrounding
  `start_detection_daemon` and `event_processor` orchestrators
  contribute ~10 "delete field from struct" mutants on the
  `SpeciesFilterConfig` / `PipelineConfig` / `ModelConfig` /
  `ExtractionConfig` literals that no unit test can catch without
  either (a) extracting per-config pure builders (the dim_to_usize
  template pattern), or (b) standing up an integration harness that
  actually runs the daemon. Either is a substantial refactor and
  doesn't fit the "dep bump + traceability" theme of this PR.
  Tracking as the highest-priority follow-up; the matrix template
  is already wired so it lands as a one-line addition once the
  helpers exist.

#### Supply chain — last advisory ignore lifted (item A3)

- **RUSTSEC-2026-0097 dropped from `.cargo/audit.toml` and `deny.toml`.**
  The lockfile now pins `rand 0.8.6` (the patched version the
  advisory listed under `>= 0.8.6` ↦ fix). Both ignore lists are now
  empty — the project clears `cargo audit --deny warnings` and
  `cargo deny check advisories` with no exceptions. The comment in
  both files documents the chain that unblocked it for next time.

#### Operability and test coverage on the carryover path from PR #35

- **`birdnet-core::detection::daemon::new_event_correlation_id`** —
  generates a short, sortable ID stamped on every event the daemon emits
  for one audio file. `DetectionEvent` gains a `correlation_id` field
  that propagates through `decode → infer → notify → DB write`, so an
  operator can trace one file end-to-end with a single grep over the log
  stream. Closes the visibility gap noted in the carryover plan ("every
  event currently carries species + confidence but not a recording-id
  or chunk-id").
- **`birdnet-web::metrics`** — process-local Prometheus counters and
  latency histograms surfaced at `/api/v2/metrics`. Replaces the previous
  scrape-time snapshot (DB row count, RSS) with a real time-series
  exposition: `birdnet_detections_total{species,chunk_offset}`,
  `birdnet_inference_duration_seconds`, `birdnet_db_write_duration_seconds`,
  `birdnet_audio_source_up{source}`, `birdnet_watchdog_pings_total`.
  Hand-rolled exposition (no `prometheus` crate dependency); fixed
  histogram buckets bracket the real per-chunk latency on a Pi 5
  (1 ms ... 10 s). 9 new lib tests pin the renderer's escaping,
  bucket-cumulativity, and sort-determinism contracts.
- **`docs/grafana-dashboard.json`** — committed dashboard for the new
  metrics. Five rows: Liveness (audio source up, watchdog ping rate,
  uptime), Detection signal (per-species rate timeseries + lifetime
  table), Pipeline latency (inference + DB-write p50/p95/p99), Resources
  (RSS against the 384 MiB MemoryHigh ceiling, distinct species).
- **`birdnet-behavior --doctor` watchdog check** verifies the daemon's
  systemd-watchdog plumbing is honoured by the supervisor. Walks the
  three-question decision matrix: `NOTIFY_SOCKET` set? `WATCHDOG_USEC`
  set? does a synthetic `WATCHDOG=1` ping reach the socket? Outcomes:
  `Skip` (not under systemd), `Warn` (notify-but-no-watchdog),
  `Pass` (ping delivered, interval echoed), `Fail` (ping rejected —
  supervisor has gone away). Six new unit tests cover the describe and
  probe paths.

### Changed

- **Refactored `src/daemon.rs::event_processor`** to extract its
  threshold gates into a pure-logic helper, `decide_disposition`,
  returning a `DispositionDecision` enum. The 600-line god-function
  shrinks slightly and gains nine unit tests pinning every cell of the
  per-species × global threshold decision matrix — the kind of
  per-file coverage gap the PR #35 carryover identified as the source
  of the production bugs we just shipped fixes for.
- **`crates/birdnet-core/src/inference/model.rs`** refactored to expose
  three new public helpers, `infer_sample_rate_from_shape`,
  `recommended_chunk_samples_from_shape`, and `compute_confidence`,
  each of which used to be inline branching inside a method. The
  helpers are mock-free, branch-pinnable, and now carry 17 additional
  unit tests covering every model-family decision cell — including the
  V3.0 sigmoid-on-probabilities regression that took out the previous
  shipping confidence. The `regression_v30_probability_not_sigmoided`
  test pins the anchor case directly.
- **Mutation testing scope widened** to a 3-file matrix with
  `missed > 0` as the gate on every file:
  `crates/birdnet-core/src/config/validate.rs`,
  `crates/birdnet-core/src/inference/model.rs`,
  `crates/birdnet-core/src/audio/extraction/extractor.rs`. Each file
  is its own job so a surviving mutant in one doesn't tank the
  report on the others. Two embedded ~220-byte ONNX models
  (`crates/birdnet-core/src/testdata/tiny_v24_test.onnx` and
  `tiny_v30_test.onnx`) let the new BirdNetModel tests drive
  `infer_sample_rate`, `recommended_chunk_samples`,
  `is_probability_output`, the setters, and `predict` without the
  real 541 MB BirdNET+ model on disk. The mutation workflow installs
  `ffmpeg` so the freq-shift and format-conversion branch tests in
  extractor.rs actually run instead of skipping. Final mutant counts
  on the touched files: **0 missed / 65 caught on validate.rs**,
  **0 missed / 73 caught on inference/model.rs**, **0 missed / 24
  caught on extractor.rs** (numbers will be re-verified by the
  matrix run after this lands).
- **Eight transitive RUSTSEC advisories lifted** by targeted
  `cargo update --precise`: `rustls-webpki` 0.103.9 → 0.103.13 covers
  RUSTSEC-2026-0049/0098/0099/0104, `aws-lc-rs` 1.16.1 → 1.17.0 brings
  `aws-lc-sys` 0.38.0 → 0.41.0 covering RUSTSEC-2026-0044/0048,
  `tar` 0.4.44 → 0.4.46 covers RUSTSEC-2026-0067/0068. The only
  remaining ignore is RUSTSEC-2026-0097 against `rand` 0.8.5 (no 0.8.x
  patch released upstream as of this writing; rand 0.9.x line is
  current at 0.9.4). `.cargo/audit.toml` and `deny.toml` both reflect
  the new lone-entry state with an explicit justification.
- **`coverage.yml` exclusion comment expanded** to document why
  `crates/birdnet-migrate/` and `crates/birdnet-behavioral/` stay out
  of the per-PR coverage measurement (the analytics crate's DuckDB
  build adds ~10 minutes; the migration crate is fixture-driven and
  per-line numbers would be misleading). Both decisions are revisited
  on each major refactor of those crates.

#### Dependency refresh — folded in PRs #37–#48 from Dependabot

- **GitHub Actions** bumped across every workflow:
  `actions/cache@v4 → v5`, `actions/upload-artifact@v4/v6 → v7`,
  `actions/download-artifact@v7 → v8`,
  `marocchino/sticky-pull-request-comment@v2 → v3`. Pinned SHAs in
  `release.yml` updated to match (`v4.6.2 → v7.0.1` for upload,
  `v4.1.8 → v8.0.1` for download).
- **Cargo patch + minor group**: `clap` 4.6.0 → 4.6.1, `filetime`
  0.2.27 → 0.2.29, `proptest` 1.10 → 1.11, `reqwest` 0.13.2 → 0.13.3,
  `tower-http` 0.6.8 → 0.6.11, `tracing-subscriber` 0.3.22 → 0.3.23.
- **Cargo async runtime group**: `tokio` 1.51 → 1.52 (patch).
- **Cargo web framework group**: `axum` 0.8.8 → 0.8.9,
  `tokio-tungstenite` 0.28 → 0.29 (transitive).
- **`audioadapter-buffers` 2 → 3** — semver-major bump in the audio
  buffer adapter; no API changes needed in this codebase (`rubato`
  consumed it transitively, and our direct uses target only the
  `InterleavedSlice` constructor which is stable across the bump).
- **`criterion` 0.5 → 0.8** — major bench-framework bump; only used
  in `crates/birdnet-core/benches/audio_pipeline.rs`, which compiles
  unchanged against 0.8. Dropped transitive deps `is-terminal` and
  `hermit-abi`.
- **`sysinfo` 0.32 → 0.39** (PR #47) — the 0.39 line requires Rust
  1.95, so it is paired with the **workspace MSRV bump 1.88 → 1.95**
  (see below). The API changes were already adopted on the way through
  0.38 — `RefreshKind::new()` → `RefreshKind::nothing()` (rename, same
  behaviour), `Components::refresh()` takes a `bool` arg, and
  `Component::temperature()` returns `Option<f32>` so we use
  `.and_then` instead of `.map` — so the 0.38 → 0.39 step needed no
  source changes, only the version constraint and the MSRV move.
- **Workspace MSRV raised 1.88 → 1.95**, the current Rust stable as of
  2026-05-22. Driven by `sysinfo` 0.39 (above); 1.95 is both the floor
  that crate demands and the latest released toolchain, so the MSRV
  tracks stable rather than trailing it. Updated in lockstep:
  `Cargo.toml` `rust-version`, `clippy.toml` `msrv`, the Dockerfile
  `RUST_VERSION` arg (`rust:1.95-slim-trixie` builder), the
  `dtolnay/rust-toolchain` pins in `ci.yml` and `release.yml`, and the
  README badge / docs.
- **New clippy nursery lint allowed for the 1.95 toolchain.** Rust
  1.95's clippy enables `duration_suboptimal_units`, which flags ~25
  pre-existing `Duration::from_secs(…)` call sites in favour of
  `from_mins` / `from_hours`. The explicit-seconds form is intentional,
  so the lint is added to the workspace `[lints.clippy]` allowances
  rather than churning those sites (and `from_days` is still unstable
  at this MSRV regardless).
- **Currency sweep (2026-05-22).** In-range `cargo update`: `serde_json`
  1.0.149 → 1.0.150, `duckdb` 1.10502 → 1.10503 (`libduckdb-sys`
  likewise), plus transitive `autocfg` 1.5.0 → 1.5.1 and `either`
  1.15.0 → 1.16.0. The unused `ndarray` workspace entry was aligned
  0.16 → 0.17 to match the version `ort` already resolves transitively
  (0.17.2).
- **`rusqlite` 0.38 → 0.39** and **`rubato` 2.0 → 3.0** — the two
  out-of-range majors surfaced by the currency review, both verified
  drop-in with no source changes. `rusqlite` 0.39 pulls `libsqlite3-sys`
  0.36 → 0.37 and passes the full `birdnet-db` / `birdnet-migrate` /
  `birdnet-web` suites (and the analytics-gated `birdnet-behavioral`
  connection path); `rubato` 3.0 leaves its `audioadapter` pin unchanged
  and passes the `birdnet-core` lib + `audio_pipeline` integration
  tests. With these, every direct dependency is at its latest release as
  of 2026-05-22.
- **`rubato` 1.0.1 → 2.0.0** — major-version bump with no source
  changes needed in our consumer (the resampler API we use is stable
  across the bump). Brought in transitive `audioadapter` 3 to match.
- **`symphonia` 0.5.5 → 0.6.0** — major-version bump that **did**
  break our `decode_file` implementation. Rewrote
  `crates/birdnet-core/src/audio/decode.rs` for the new API:
    * `symphonia::core::probe::Hint` → `symphonia::core::formats::probe::Hint`.
    * `get_probe().format(...)` (taking options by ref, returning a
      `ProbeResult`) → `get_probe().probe(...)` (taking options by
      value, returning a `Box<dyn FormatReader>` directly).
    * `format.default_track()` → `format.default_track(TrackType::Audio)`.
    * `track.codec_params` is now `Option<CodecParameters>` rather
      than a flat struct; access requires `.as_ref().and_then(|p| p.audio())`.
    * `get_codecs().make(...)` → `get_codecs().make_audio_decoder(...)`
      taking the audio-specific `AudioCodecParameters`.
    * `format.next_packet()` now returns `Result<Option<Packet>>`
      (`None` for EOF rather than `UnexpectedEof`).
    * `packet.track_id` is a struct field, not a method.
    * Buffer-copy API switched from
      `SampleBuffer::new(...).copy_interleaved_ref(audio_buf)` to
      `audio_buf.copy_to_slice_interleaved(&mut vec)`, sized via
      `audio_buf.samples_interleaved()`. `num_planes()` now reports
      channel count.
  All 243 birdnet-core lib tests still pass; the live ADR-16 Layer-4
  check (Pica WAV → DB) must run in CI after merge.
- **Skipped: PR #36** (`dtolnay/rust-toolchain` 1.88 → 1.100).
  Rust 1.100 does not exist — current stable is 1.95 and Dependabot
  misordered the `1.x` action tags (it sorts `1.100 > 1.95`
  lexically). The toolchain pins move to **1.95**, the real current
  stable, via the MSRV bump above — not to the bogus 1.100. PR #36
  should be closed.
- **Lockfile**: 8 transitive RUSTSEC advisories now unblocked
  (rustls-webpki 4, aws-lc-sys 2, tar 2 — see A3 above) plus the
  routine churn from the Dependabot bumps. Only RUSTSEC-2026-0097
  (rand 0.8.5) remains, with the same documented justification.

### Fixed

- **Detection confidence on BirdNET+ V3.0 preview models was being
  silently halved** by applying `sigmoid` to the model's `predictions`
  output. The official `birdnet-team/birdnet-V3.0-dev/analyze.py`
  reference uses the model output as already-calibrated probabilities
  in `[0, 1]` (its default threshold is `--min-conf 0.15`, which only
  makes sense against a probability distribution). Our pipeline was
  applying `sigmoid(sensitivity * raw)` to those values, which
  compressed the entire `[0, 1]` range into `[0.5, 0.73]` and turned a
  Magpie that the model rated `0.9247` into a `0.7160` detection. Same
  effect on every species — every detection clustered near 50 % because
  `sigmoid(~0) = 0.5`, which is why every WAV ended up with a long
  list of spurious "owl detections" near the noise floor.
  - Fix: new `is_probability_output` flag set at model-load time from
    the input shape (V3.0 fixed or dynamic ⇒ true). The `predict` path
    branches on it — V3.0 models pass through clamped to `[0, 1]`,
    V2.4 still goes through `sigmoid(sensitivity * logit)`.
  - Live verification on the bundled Pica WAV: confidence climbs from
    71 % to **92.1 %, 91.6 %, 81.9 %, 93.9 %** — matching the V2.4 /
    BirdNET-Pi reference range of 93.9–97.0 % on the same WAV. The
    spurious owl detections at the previous ~50 % noise floor have
    completely disappeared (the real noise floor is below 5 %).
  - `tests/inference_e2e.rs` bumps its assertion from `> 0.50` to
    `> 0.80` so a future regression of this class fails the test
    immediately instead of silently lurking under a tolerant bound.
- **Audio-clip extraction range inversion** (`crates/birdnet-core/src/audio/extraction/extractor.rs`):
  `safe_stop` was clamped to the operator-configured `recording_length`
  rather than the actual decoded audio length. Any detection past that
  window produced `start_sample > stop_sample` and silently dropped the
  clip with the error *"invalid sample range: 1224000..720000"*. The
  fix decodes first, clamps both endpoints to the file's real length,
  rejects empty audio with a clear message, and ships three regression
  tests covering the clamp / EOF / empty-audio paths.
- **Detection rows lost across chunks of one recording**
  (`migration 11`): the previous `UNIQUE(Date, Time, Sci_Name)`
  constraint collapsed every chunk of one recording into a single row
  because every chunk inherits the same `Time` from the file name. A
  Eurasian Magpie that called in chunks 0, 4.5, 9, 13.5, and 18 seconds
  produced **one** database row; the other four were rejected and lost.
  New schema: `chunk_offset_secs REAL NOT NULL DEFAULT 0.0` column plus
  `UNIQUE(Date, Time, Sci_Name, File_Name, chunk_offset_secs)`. Live
  re-run with the bundled Magpie WAV: **5 distinct chunks recorded, top
  confidence 71.9 %**.
- **Test-fixture schema drift**
  (`crates/birdnet-db/src/sqlite/connection.rs::open_or_create`): this
  helper hand-coded its own `CREATE TABLE detections` with only the
  migration-1 columns, so every test using it ran against a stale
  schema. Fixed to apply the full migration chain — surfaced six
  pre-existing test failures masquerading as passes that the new
  migration 11 caught immediately.
- **Three `INSERT INTO detections VALUES (...)` time bombs** with no
  column list in `birdnet-db/sqlite/queries/heatmap.rs`,
  `correlation.rs`, and `birdnet-migrate/birdnet_pi/importer.rs`. Each
  would break the same way as the main daemon insert did when a future
  migration adds a column. Now all use explicit column lists.

- **Detection confidence on BirdNET+ V3.0 preview models** improves
  substantially because the daemon now adopts the model's recommended
  chunk length instead of always using the V2.4-era 3.0-second default.
  Same `Pica_pica_30s.wav` fixture, same model, only chunk length
  changed: Eurasian Magpie confidence went from **52.2 %** (3.0 s × 32 kHz =
  96 000 samples) to **71.5 %** (4.5 s × 32 kHz = 144 000 samples).
  Python ONNX Runtime reference at 4.5 s gives the same 71.8 %, so the
  Rust pipeline now sits at parity with the reference implementation
  rather than 19 percentage points below it. Investigation, evidence
  and the comparison against BirdNET V2.4 (which BirdNET-Pi used and
  which still hits 93–97 % on the same WAV) live in the new ADR
  [`docs/architecture/15-model-chunking.md`](docs/architecture/15-model-chunking.md).
- `BirdNetModel::recommended_chunk_samples()` and
  `recommended_chunk_secs()` expose the per-model chunk size so the
  daemon can pick the right value without hard-coding model knowledge
  in the pipeline.

### Added

#### Field-deployment hardening (24/7/365 unattended operation)

- **systemd watchdog integration** (`src/sd_notify.rs`). The daemon now
  speaks the `sd_notify` protocol natively (no extra dependency): sends
  `READY=1` after the HTTP server binds, `WATCHDOG=1` every
  `WATCHDOG_USEC / 2` from a background tokio task, and `STOPPING=1` on
  graceful shutdown. Verified end-to-end against a real Unix datagram
  socket: `READY=1 → WATCHDOG=1 …  → STOPPING=1`. Fixes the previously
  broken combination of `WatchdogSec=120` (set in the systemd unit) with
  no `sd_notify` call in the binary — under the old config systemd
  would kill the daemon every 2 minutes in production.
- **Periodic database maintenance** (`src/maintenance.rs`) — background
  task that runs a daily `PRAGMA integrity_check`, a weekly WAL
  checkpoint + `VACUUM`, and prunes the backup directory to the most
  recent 14 snapshots. All best-effort with full logging; never crashes
  the loop on transient failure.
- **`vacuum_database` and `checkpoint_wal`** added to
  `birdnet_db::resilience` so the binary can do scheduled maintenance
  without taking a new direct `rusqlite` dependency.
- **Hardened systemd unit** in `install.sh`:
  - `Type=notify` + `NotifyAccess=main` + `WatchdogSec=120` —
    process-supervision contract is now real.
  - `ExecStartPre` runs `birdnet-behavior --doctor`; exit code 2
    (errors) blocks startup so the journal shows *what is broken*
    instead of a restart-loop.
  - `ProtectSystem=strict`, `ProtectHome=read-only`, explicit
    `ReadWritePaths`, `PrivateTmp=yes`, `NoNewPrivileges=yes`,
    `LockPersonality=yes`, `MemoryDenyWriteExecute=yes`,
    `RestrictRealtime=yes`, `RestrictNamespaces=yes`,
    `SystemCallFilter=@system-service` minus the privileged / kernel /
    debug / reboot / mount / cpu-emulation / clock / module groups.
  - Resource ceilings: `MemoryMax=512M`, `MemoryHigh=384M`,
    `TasksMax=512`, `LimitNPROC=256`, `OOMPolicy=stop`.
  - `After=network-online.target sound.target time-sync.target` —
    no startup race with mic enumeration or clock sync on slow-booting
    hardware.
  - `LogRateLimitIntervalSec=30` + `LogRateLimitBurst=1000` — a chatty
    failure mode cannot exhaust the SD card.
- **`docs/FIELD_DEPLOYMENT.md`** — 12-section runbook for unattended
  deployments: hardware checklist, power & thermals, storage planning,
  network resilience, system hardening, time synchronisation, watchdog
  smoke test, backup policy, remote diagnostics, update strategy,
  pre-flight checklist, and a symptom-keyed recovery runbook.

- **`birdnet-behavior --doctor`** (alias `--preflight`) — a one-shot
  preflight diagnostic that runs ~12 environment checks (CPU, temp dir,
  config parse, every config value range, listen address, database
  directory and integrity, recordings dir, audio source reachability with
  ALSA / PulseAudio / RTSP probes, model file sanity, audio encoder
  presence when needed, Apprise CLI when configured, disk free space) and
  prints a one-screen report with a remediation hint per finding. Exit
  code summarises the worst severity (0 = ready, 1 = warnings, 2 = errors)
  so it works in monitoring scripts as well as interactively.
- **`birdnet-behavior --doctor-json`** — same checks, single-line JSON
  output for monitoring integrations (Nagios, Zabbix, Home Assistant
  command sensor, Prometheus textfile collector). String escaping is
  hand-rolled per RFC 8259 §7; control characters become `\uXXXX`.
- Configuration validation at load time
  (`birdnet_core::config::validate`) — surfaces 13 distinct
  misconfigurations (lat/lon pairing and range, CONFIDENCE / SF_THRESH /
  PRIVACY_THRESHOLD / SENSITIVITY / OVERLAP / RECORDING_LENGTH /
  SEGMENT_DURATION bounds, schedule string shape, mutually-exclusive audio
  sources, unsupported AUDIO_FORMAT, unknown INFO_SITE, malformed language
  code) with clear remediation messages.
- Property-based tests (proptest) for the configuration validator cover
  the full reachable numeric range plus a panic-freedom invariant over
  arbitrary string input.
- Supply-chain CI workflow (`.github/workflows/supply-chain.yml`) running
  `cargo-deny`, `cargo-audit`, `cargo-machete`, `typos`, and `shellcheck`
  on every PR and weekly cron.
- Reproducibility files: `rust-toolchain.toml`, `rustfmt.toml`,
  `clippy.toml`, `deny.toml`.
- Repository hygiene: `SECURITY.md`, `.github/CODEOWNERS`,
  `.github/dependabot.yml`, structured GitHub issue forms, and a PR
  template with quality-gate checkboxes.
- Architecture Decision Record `docs/architecture/14-diagnostics.md`
  captures the design and trade-offs of the diagnostic system.
- **Snapshot tests** for the `--doctor` text output. The render is split
  into a pure `render_text(&[Check]) -> String` function; four golden
  files under `src/testdata/doctor_snapshots/` pin the exact bytes of
  the report so accidental wording or formatting drift has to come
  through a PR. Set `UPDATE_DOCTOR_SNAPSHOTS=1 cargo test` to refresh
  after an intentional UX change.
- **Mutation testing** workflow (`.github/workflows/mutation.yml`)
  that runs `cargo-mutants` on the configuration validator. Catches
  "tests pass even after the validator's behaviour changes" — the
  one mutant that survived in the first run revealed a missing minute
  boundary case, which is now covered by a new property test.
  Current score: 0 missed / 61 caught / 4 unviable.
- **Coverage workflow** (`.github/workflows/coverage.yml`) running
  `cargo-llvm-cov` on every PR. Sticky summary comment, HTML + lcov
  artifacts, optional Codecov upload via `CODECOV_TOKEN`.
- **Subprocess smoke tests** for the binary (`tests/doctor_smoke.rs`).
  Builds the actual binary and runs `--version`, `--help`, `--doctor`,
  `--preflight` (alias), `--doctor-json`, and `--check-db` to catch
  "compiles but doesn't run" regressions — exactly the class of bug
  that previously slipped past the unit tests when tracing was writing
  to stdout and silently corrupting the JSON output.
- **`.pre-commit-config.yaml`** mirrors the CI quality gates locally so
  contributors fail fast (rustfmt check, typos, shellcheck, optional
  manual clippy, generic file hygiene, Conventional-Commits message
  format).
- **Top-level `TROUBLESHOOTING.md`** organised by symptom — service
  won't start, web UI not reachable, no detections, database errors,
  memory pressure on small hardware, notifications never arrive,
  cross-cutting "huh, that's weird" checklist. Each section links back
  to the doctor as the first step.

### Changed

- `install.sh` model download now resumes on interrupt (`curl -C -` /
  `wget -c`), shows a progress bar, and keeps the partial file in place
  on failure so a flaky connection no longer forces a 541 MB restart from
  zero. Failure messages list the three common root causes (no internet,
  Zenodo down, disk full) inline.
- `.env.example` gains worked latitude/longitude examples for three
  continents, an OpenStreetMap walk-through for finding coordinates, and
  units + ranges for SF_THRESH, PRIVACY_THRESHOLD, SEGMENT_DURATION, and
  the schedule modes.
- `README.md` troubleshooting section now leads with
  `birdnet-behavior --doctor`.
- `quickstart.sh` post-bootstrap output advertises the diagnostic.

## [0.1.0] - 2026-04-12

First public release. BirdNet-Behavior is a ground-up Rust rewrite of
BirdNET-Pi that ships as a single static binary for Raspberry Pi and
x86_64 Linux.

### Added

#### Core detection pipeline

- Pure-Rust audio pipeline with `symphonia` (decode), `rubato` (resampling),
  and `realfft` (mel spectrogram) — zero C dependencies in the audio path.
- ONNX Runtime inference through the `ort` crate, statically linked into
  release binaries. BirdNET+ V3.0 is the default model; BirdNET V2.4 FP16
  and V1 remain compatible.
- File-watcher detection daemon with configurable chunking, overlap,
  sensitivity, per-species confidence thresholds, and privacy filtering.
- Audio quality pre-filtering: SNR estimation, spectral flatness,
  adaptive noise-floor tracking, and rain / wind detection.
- Species occurrence frequency filter driven by the BirdNET metadata
  model, with whitelist, include, and exclude lists.
- Rare-bird quarantine workflow: detections that fall below per-species
  thresholds are quarantined for manual review rather than dropped.

#### Audio capture

- ALSA, PulseAudio, PipeWire, and RTSP capture sources, each supervised
  as a restart-aware subprocess with gap detection and disk monitoring.
- Multiple simultaneous RTSP streams via `--rtsp-urls`.
- Solar-aware recording scheduler with sunrise / sunset computation,
  twilight offsets, fixed-window schedules, and a night-inhibit mode.
- tmpfs support for transient audio storage to reduce SD card wear on
  Raspberry Pi deployments.
- Automatic disk management: per-species retention caps, auto-purge, and
  configurable disk-usage thresholds.

#### Storage and resilience

- SQLite operational database with WAL mode, ten idempotent schema
  migrations, integrity checks, hot backup, restore, and auto-recovery.
- Per-IP rate limiter on API and admin routes (token-bucket with
  `Retry-After` header).
- HTTP Basic Auth with constant-time comparison.

#### Web server and dashboard

- `axum` HTTP server with REST API, WebSocket, Server-Sent Events, and
  server-rendered HTMX pages. No client-side JavaScript framework.
- HTMX pages: dashboard, today, history, species list, species detail,
  species gallery, life list, activity heatmap, correlation, charts,
  weekly report, recordings browser, audio player, livestream, kiosk,
  notification center, quarantine, system health, and weekly report.
- Admin panel: settings editor, species thresholds, species filter
  tester, BirdNET-Pi migration wizard, system info, backup management,
  live log viewer (SSE), notification history, alert rules, data
  quality dashboard, and binary update check.
- Full dark / light theme support with OS preference detection.

#### Analytics (optional `analytics` feature)

- DuckDB behavioral analytics: sessionize, retention, funnel, sequence,
  and next-species prediction, implemented via the duckdb-behavioral
  extension.
- Phenology analytics: migration timing percentiles, weekly abundance
  index, peak weeks, monthly totals, species richness, and
  effort-corrected abundance.
- Time-series analytics: activity, diversity (Shannon), trend, peak,
  gap, and session windows (tumbling, sliding, hopping, session).

#### Integrations

- BirdWeather detection and soundscape uploads with retry and backoff.
- Apprise notifications across 80+ channels with per-species cooldown,
  watchlist, and template rendering.
- SMTP email alerts via `lettre` with rustls TLS (no OpenSSL).
- Wikipedia species image cache with on-disk and in-memory indexing.
- Pure-Rust MQTT 3.1.1 publisher (no external broker library) with
  Home Assistant auto-discovery.
- GitHub Releases auto-update with atomic binary replacement.
- Heartbeat URL pinging for uptime monitors.

#### Migration

- Non-destructive BirdNET-Pi import wizard. Source database is opened
  read-only. Transactional, idempotent, with pre- and post-migration
  species reports and a data quality report.
- Supports both BirdNET-Pi SQLite databases and `BirdDB.txt` CSV flat
  files.

#### Observability and deployment

- Prometheus metrics endpoint (`/api/v2/metrics`).
- `tracing`-based structured logging with SSE log streaming.
- Multi-architecture Docker images published to GHCR (`linux/amd64`,
  `linux/arm64`), with and without the `analytics` feature.
- Cross-compiled release binaries for `aarch64-unknown-linux-gnu` and
  `x86_64-unknown-linux-gnu`.  The `ort` crate does not ship prebuilt
  ONNX Runtime binaries for `armv7-unknown-linux-gnueabihf`, so 32-bit
  ARM is not supported — Pi 3 / Pi Zero 2W users should install the
  64-bit Raspberry Pi OS, or build from source.
- Release binaries are built on Ubuntu 24.04 (GCC 13, glibc 2.39) to
  match the libstdc++ and glibc baselines that pyke's prebuilt ONNX
  Runtime archives require.  **Runtime requirement: glibc >= 2.39**
  (Raspberry Pi OS Trixie, Debian 13, Ubuntu 24.04, or newer).
- systemd installer script with ALSA microphone auto-detection and
  automatic BirdNET+ model download from Zenodo.

[Unreleased]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.15.0...HEAD
[0.15.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.13.1...v0.14.0
[0.13.1]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.13.0...v0.13.1
[0.13.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.7.2...v0.9.0
[0.7.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.5.3...v0.6.0
[0.5.3]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/tomtom215/BirdNet-Behavior/releases/tag/v0.3.0
[0.2.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tomtom215/BirdNet-Behavior/releases/tag/v0.1.0
