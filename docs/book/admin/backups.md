# Backups & Recovery

The **Data** tab (`/station/data`) is where you protect your records: snapshots, exports, storage and the controls you hope you never need.

![The backups and recovery page](../images/admin-backups.png)

## Three kinds of backup

**Snapshots** copy the database only. They are small and quick, they run automatically **every 7 days**, and the station keeps the most recent **14**. They live in `backups/` beside `birds.db` — *on the station itself*. That makes them perfect for undoing a bad import or recovering from database corruption, and useless if the SD card dies.

**Full backups** bundle the database, your recordings and your config into one `.tar.gz` you download. This is the one to keep somewhere else. Nothing takes these automatically — download one after any big change, and periodically if your station matters to you.

**Offsite copies** send each weekly snapshot to an object store or an SSH server, encrypted before it leaves. Off by default; see [Offsite backups](#offsite-backups) below. This is what covers the failure the other two do not: the card wearing out, the enclosure flooding, the Pi being stolen.

The automatic schedule runs on **elapsed wall-clock time, not uptime**, so a station that reboots often still gets its backups: an overdue job runs shortly after the next boot rather than restarting its timer.

## Restoring

**Restore from file** takes a full backup archive and puts its database and recordings in place of the current ones. The station first checks that the disk can hold the archive's contents with headroom, refuses an archive whose database is not named as this station's, stops recording detections, unpacks the archive beside the database and integrity-checks the copy there, and only then swaps the files in (the database by rename, so the running process is never left reading a half-written file; recordings merged clip by clip, so clips the archive lacks are kept). Under systemd the station restarts itself to load the result; elsewhere it tells you to.

> **Restoring is destructive and cannot be undone.** It replaces what is on the station now, and the station does **not** snapshot the current state first. Download a full backup before you restore. Detections stay paused until the restart that follows.
>
> The archive's contents are not signed or verified — only restore an archive you produced yourself and trust.

Individual snapshots can be downloaded and deleted from the snapshot list, so you can recover a database by hand: stop the service, put the snapshot in place of `birds.db`, and start it again.

## Export

Your detection data is yours, in formats other tools read:

- **Detections (CSV or JSON)** — every detection the station stands behind, with date, species and confidence. Rows a reviewer rejected, and imported rows on a station that excludes imports from its analytics, are left out; the same rule the charts use. `Date` and `Time` are the station's local wall clock with no offset, exactly as BirdNET-Pi wrote them; beside them every row carries `Event_Date` (that wall clock with the offset that was in force, RFC 3339, so the two passes of a repeated autumn hour are told apart) and `Detected_At_UTC` (the instant), both blank on a row that names no point in time. Every row also says which model made it: the CSV ends in `Run_Id,Model_Name,Model_SHA256` and the JSON carries `run_id`, `model_name` and `model_sha256` per row — the model file's name and the SHA-256 of its bytes, so a season that spans a model upgrade can be split by which model heard what. The runs themselves (when each started, the labels checksum, the settings in force) are at `/api/v2/analysis-runs`. The three columns are empty on a row this station did not analyse: imported history, or rows older than the run table.
- **Species summary (CSV)** — per-species totals and first-seen dates.
- **eBird checklist** — eBird Record Format, one record per species per hour with `Number` written as `X` (present, not counted) and the detection tally in the comment. Only detections at or above a confidence floor (0.75 by default, `?min_confidence=`) that a reviewer has not rejected are included, and the coordinates are the station's configured location — blank, never `0,0`, if none is set. Protocol, observer count, region and completeness are query parameters (`?protocol=Stationary&observers=1&state=&country=&complete=false`), because they are facts about the submitter rather than the station.
- **BirdNET-Pi `BirdDB.txt`** — semicolon-separated, the twelve BirdNET-Pi columns and nothing more, for tools expecting the original format. Same rows as the CSV, without the three model columns: the format has no room for them and its consumers count fields.
- **Raven selection table** — one tab-separated table over every detection that has a clip, in the format BirdNET-Analyzer writes (`Selection`, `View`, `Channel`, `Begin Time (s)`, `End Time (s)`, `Low Freq (Hz)`, `High Freq (Hz)`, `Common Name`, `Species Code`, `Confidence`, `Begin Path`, `File Offset (s)`), so Raven Pro opens it against the recordings folder as a multi-file table. Each selection is placed where the detection sits inside its clip, from the lead-in the extractor recorded; a row written before that was recorded (schema 47) spans its whole clip, and a row with no clip, or no clip length, is left out rather than given a made-up window. The species code is the eBird code from the station's label file, or the scientific name when it has none. Every clip also has its own table at `/api/v2/recordings/<clip>/raven.txt` and an Audacity label track (`begin`, `end`, label; **File → Import → Labels**) at `/api/v2/recordings/<clip>/labels.txt`; the detection page links to neither yet, so paste the clip's filename in.

## Storage & retention

The storage breakdown shows where space is going — the database (including its write-ahead log), your recordings, and the snapshots — measured live, not estimated.

Time-based retention is **off by default**, not absent: set **Keep Clip Audio (days)** (`CLIP_RETENTION_DAYS`, `--clip-retention-days`) for a rolling window — `0`, the default, keeps audio for ever. Two disk-based limits apply alongside it:

- the disk manager purges the **oldest** recordings once the disk crosses `DISK_PURGE_THRESHOLD` (default 95%);
- `MAX_FILES_SPECIES` keeps at most N clips per species, pruned on the daily maintenance tick.

Recordings you have **locked** are never purged by either, and locking takes effect on the next cycle — no restart needed. When a clip is pruned its detection row survives: your counts, species lists and analytics are unaffected, only the audio is removed.

To lock a clip, open [Recordings](../guide/recordings.md) and use the 🔒 **Lock** action on a row — or select several and use the bulk **Lock** button above the grid. Locked rows show an unlock action in the same place.

## Offsite backups

A snapshot on the same SD card as the database it came from protects you from a corrupt page and a bad import. It does not protect you from the card, and SD cards in a box on a fence post are a *when*, not an *if*.

Set `OFFSITE_BACKUP` and each weekly snapshot is encrypted on the station and uploaded. Nothing else changes: the local snapshots and their 14-file rotation stay exactly as they were, so this only ever adds a copy.

### Encryption is not optional

Your database is a log of what is around your house and when you are there. "Server-side encryption" on a bucket means the provider holds the key; an SFTP host means its administrator does. So the station encrypts before it uploads — argon2id over your passphrase, then ChaCha20-Poly1305 — and there is no setting to turn that off.

```text
OFFSITE_PASSPHRASE=a long passphrase you keep somewhere else
```

**Write it down somewhere that is not this station.** There is no recovery: the passphrase is not stored anywhere, not derivable from the backup, and not known to us. A backup you cannot decrypt is not a backup.

At least 12 characters, and the station refuses shorter ones — not as password policy, but because below that the argon2 parameters stop being what protects the file.

Secrets are **config-file or environment keys only**. There is deliberately no `--offsite-passphrase` flag: anything on a command line is visible in `ps` to every user on the machine, is copied into the journal by systemd's `ExecStart=`, and lands in your shell history.

### To an S3-compatible store

Works with AWS S3, Backblaze B2's S3 API, Cloudflare R2, Wasabi, MinIO, Ceph RGW and Garage.

```text
OFFSITE_BACKUP=s3
OFFSITE_S3_ENDPOINT=https://s3.eu-west-2.amazonaws.com
OFFSITE_S3_BUCKET=my-birdnet-backups
OFFSITE_S3_PREFIX=stations/garden          # optional
OFFSITE_S3_REGION=eu-west-2                # default us-east-1
OFFSITE_S3_ACCESS_KEY=AKIA...
OFFSITE_S3_SECRET_KEY=...
OFFSITE_S3_ADDRESSING=auto                 # auto | virtual | path
OFFSITE_PASSPHRASE=...
OFFSITE_KEEP=8                             # 0 keeps everything
```

`OFFSITE_S3_ADDRESSING` decides whether the bucket goes in the hostname (`bucket.endpoint/key`, which AWS requires for buckets made after September 2020) or the path (`endpoint/bucket/key`, which every self-hosted store speaks and some speak only). `auto` picks by endpoint and is right nearly always; set it explicitly if you get a 404 that mentions the bucket.

Give the station its own access key with permission to `PutObject`, `ListBucket` and `DeleteObject` on that prefix, and nothing else. It never reads a backup back — restoring is a thing you do from another machine, deliberately.

### To an SSH server

Any host you can `sftp` into: a NAS, a VPS, a Pi in a different building.

```text
OFFSITE_BACKUP=sftp
OFFSITE_SFTP_HOST=backup.example.net
OFFSITE_SFTP_PORT=22
OFFSITE_SFTP_USER=birdnet
OFFSITE_SFTP_DIR=/srv/backups/garden
OFFSITE_SFTP_IDENTITY=/var/lib/birdnet/ssh/id_ed25519
OFFSITE_SFTP_KNOWN_HOSTS=/var/lib/birdnet/ssh/known_hosts   # defaults beside the key
OFFSITE_SFTP_HOST_KEY_POLICY=yes                            # yes | accept-new
OFFSITE_PASSPHRASE=...
```

Key authentication only — passwords are disabled, because a batch upload cannot answer a prompt and would hang until something killed it. Generate a key for the station and authorise it on the server:

```bash
sudo -u birdnet ssh-keygen -t ed25519 -f /var/lib/birdnet/ssh/id_ed25519 -N ''
ssh-copy-id -i /var/lib/birdnet/ssh/id_ed25519.pub birdnet@backup.example.net
```

Then record the server's host key, **after checking the fingerprint against the server itself**:

```bash
ssh-keyscan -p 22 backup.example.net | sudo -u birdnet tee /var/lib/birdnet/ssh/known_hosts
```

`OFFSITE_SFTP_HOST_KEY_POLICY` has no "off". Host key checking is what makes the upload go to *your* server rather than to whoever answers, and there is no setting that disables it. Use `accept-new` for the first connection on a network you control, then set it back to `yes` so a *changed* key is refused.

### How often

`BACKUP_SCHEDULE` is `weekly` (the default) or `daily`. Daily means losing at most a day rather than a week; it also means seven times the offsite upload, which is why weekly stays the default rather than changing an existing station's bandwidth underneath it.

It schedules the **backup** only. The space reclaim — checkpointing the write-ahead log and returning free pages to the filesystem — is a separate job and stays weekly whatever you set here. Asking for a daily backup is not asking to rewrite the database file every day, and on an SD card that is write endurance spent for space nobody needed back.

Both are listed at `GET /api/v2/system/jobs`, with when each last ran and whether it is due.

### Over rsync, when the link is bad

`OFFSITE_BACKUP=rsync` sends the same backup to the same SSH server, with the same `OFFSITE_SFTP_*` settings above — only the program that moves the bytes changes. Switching is one word:

```ini
OFFSITE_BACKUP=rsync
OFFSITE_RSYNC_BWLIMIT=512   # KiB/s; 0 or unset for no limit
```

**Use it if your uplink drops.** rsync keeps what already arrived, so a transfer that dies at 90 % of a 1.3 GB backup continues from there next time. Over SFTP the same backup starts again from zero, and on a link bad enough to matter it may never finish at all. `OFFSITE_RSYNC_BWLIMIT` is the other reason: it stops a backup saturating a shared or metered connection for an hour.

**It is not faster because it is "incremental".** rsync's famous trick is sending only the blocks that changed, and that is worth nothing here. Your backups are encrypted before they leave, under a fresh random salt each time, so consecutive backups share no blocks at all — not even when the database has not changed. Measured on a 1 MiB file: 1496 of 1497 blocks reusable in plaintext, **0 of 1498** once encrypted. rsync sends the whole file every run, exactly as SFTP does. The resume and the speed limit are the reasons to use it.

It needs both `rsync` and `sftp` installed. rsync moves the bytes; `sftp` creates the directory, lists what is there and deletes what retention drops — rsync has no command that removes one named remote file, and its nearest equivalent deletes everything a filter does not mention, which is not a risk worth taking with your only offsite copies.

### Retention

`OFFSITE_KEEP` is how many backups stay at the destination; the oldest go when a new one arrives. `0` keeps everything.

Retention only ever removes files this station wrote — names of the form `birds.db.backup.<timestamp>.bnb`. Anything else in the same bucket prefix or directory is left alone, so you can share a bucket without losing the other things in it.

### Checking it works

```bash
birdnet-behavior --doctor
```

reports the destination, how many backups it will keep, and — for SSH — whether the key exists, whether its permissions are ones OpenSSH will accept, and whether the host is known. It makes no connection: `--doctor` runs on every start, and a diagnostic that dials a remote host fails whenever the uplink is down.

Uploads are logged at `info` on success and `warn` on failure, with the destination named. A failed upload never affects the local backup or the space reclaim that follows it.

### Restoring one

Download the `.bnb` file from your bucket or server, then:

```bash
birdnet-behavior --decrypt-backup downloaded.bnb --out birds.db
```

It will ask for the passphrase. The result is an ordinary SQLite database — put it in place of `birds.db` with the service stopped, as with any snapshot.

## Danger zone

Two destructive actions live in a clearly marked danger zone, each gated behind an explicit confirmation:

- **Clear all detections** — empties the detections and notification tables; settings are kept.
- **Clear extracted audio** — deletes the saved WAV clips; the detection records stay.

There is no undo. Take a full backup first.
