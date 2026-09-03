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
