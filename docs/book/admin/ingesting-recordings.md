# Analysing Long Recordings

The station analyses the segments it records itself, each a few minutes long. A
recording longer than the longest segment the station can be set to make, with a
minute's margin (eleven minutes in all), is not analysed where it is dropped. An SD
card from a field recorder, an AudioMoth or a Song Meter holds recordings of hours,
and those go through `--ingest-recordings`, which hands them to the station in
segments it can analyse.

## What happens to a long recording in the watch directory

The station reads only a recording's header before it decodes anything. If the
recording is longer than a segment may be, it is not decoded. It is moved to
`ingest/refused/` beside the station's database, and the journal says where it went
and what command takes it. It is moved because the stream directory keeps nothing
past its age limit: left where it was, it would be deleted, unanalysed. A name
already in `ingest/refused/` is kept, and the new file is numbered (`name-1.wav`).

A file refused for any other reason (one that cannot be read at all, for instance)
is left where it is, as before.

## Importing recordings

The station must be running: the command does not analyse anything itself. It writes
each recording into the watch directory a few segments at a time, and the station
analyses them as it analyses its own.

```bash
birdnet-behavior --ingest-recordings /media/card/DATA --ingest-source card_1 --ingest-name-rule yyyymmdd_hhmmss --ingest-utc-offset Z
```

- `--ingest-recordings PATH` is one recording, or a folder of them (`.wav`, `.flac`,
  `.mp3`). A folder's subfolders are not read.
- `--ingest-source NAME` is the source the detections are recorded under: letters,
  digits and `_`, at most 40 characters. Use a name no live source has, so imported
  history and the station's own listening stay apart.
- The start time is never guessed from the clock or from the file's date. Give it
  with `--ingest-start` (one recording, with its UTC offset:
  `2026-05-19T04:30:00Z` or `2026-05-19T06:30:00+02:00`), or read it from each name
  with `--ingest-name-rule yyyymmdd_hhmmss` and `--ingest-utc-offset` (`Z`, which is
  AudioMoth's default, or `+HH:MM` / `-HH:MM`). Without one of them the command
  refuses.
- `--ingest-segment-secs` sets the segment length (3 to 600 seconds; the station's
  own by default). `--ingest-max-pending` (default 4) is how many segments may wait
  in the watch directory at once. `--ingest-stall-secs` (default 600) stops the
  command when the station has analysed none of them for that long.

Each recording is read a packet at a time, so a recording of any length is never
held in memory whole. A damaged packet, or a gap in the recording's timestamps,
starts a new segment where the audio resumes, so the times after it stay true.

A rerun skips what was already imported, matched by the recording's content, its
source and its start, so it is safe to run the command again over the same card.
The record of what was imported is `ingest/journal.jsonl` beside the database.

## What imported history is

Detections in imported audio are stored like any other, under the source you named,
and announced to no one: no notification, BirdWeather upload, MQTT message, webhook,
new-species alert or live-feed entry. An alert about a recording from months ago
would read as what is singing now.

## Limits

- Only explicit UTC offsets are understood, not named time zones (`Europe/Berlin`).
  A recorder that wrote its names in local time across a daylight-saving change
  needs each side imported with its own offset.
- Segment names carry whole seconds, so a detection's time can be up to one second
  early.
- The station's own time zone must be the one the recordings' local dates and times
  are to be read in.
- Recordings that start before 2024 or after 2063 are refused, because the station
  quarantines detections dated outside those years as a clock fault, and that rule
  has no exception for imported history yet.
- There is no re-analysis of audio the station already holds.
