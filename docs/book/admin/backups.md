# Backups & Recovery

The **Data** tab (`/station/data`) is where you protect your records: snapshots, exports, storage and the controls you hope you never need.

![The backups and recovery page](../images/admin-backups.png)

## Two kinds of backup

**Snapshots** copy the database only. They are small and quick, they run automatically **every 7 days**, and the station keeps the most recent **14**. They live in `backups/` beside `birds.db` — *on the station itself*. That makes them perfect for undoing a bad import or recovering from database corruption, and useless if the SD card dies.

**Full backups** bundle the database, your recordings and your config into one `.tar.gz` you download. This is the one to keep somewhere else. Nothing takes these automatically — download one after any big change, and periodically if your station matters to you.

> **Snapshots are not off-site.** The station has no built-in upload to S3, a NAS, or email. If you need off-site copies, pull a full backup on a schedule from another machine.

The automatic schedule runs on **elapsed wall-clock time, not uptime**, so a station that reboots often still gets its backups: an overdue job runs shortly after the next boot rather than restarting its timer.

## Restoring

**Restore from file** takes a full backup archive and unpacks it over the current database and recordings.

> **Restoring is destructive and cannot be undone.** It overwrites what is on the station now, and the station does **not** snapshot the current state first. Download a full backup before you restore, then restart the service when it finishes.
>
> The archive's contents are not signed or verified — only restore an archive you produced yourself and trust.

Individual snapshots can be downloaded and deleted from the snapshot list, so you can recover a database by hand: stop the service, put the snapshot in place of `birds.db`, and start it again.

## Export

Your detection data is yours, in formats other tools read:

- **Detections (CSV)** — every detection with date, species and confidence.
- **Species summary (CSV)** — per-species totals and first-seen dates.
- **eBird checklist** — record format for submission to eBird.
- **BirdNET-Pi `BirdDB.txt`** — tab-separated, for tools expecting the original format.

## Storage & retention

The storage breakdown shows where space is going — the database (including its write-ahead log), your recordings, and the snapshots — measured live, not estimated.

Retention is **not time-based**. There is no "keep 30 days" setting. Two limits apply instead:

- the disk manager purges the **oldest** recordings once the disk crosses `DISK_PURGE_THRESHOLD` (default 95%);
- `MAX_FILES_SPECIES` keeps at most N clips per species, pruned on the daily maintenance tick.

Recordings you have **locked** (`/admin/recordings` → "lock") are never purged by either, and locking takes effect on the next cycle — no restart needed. When a clip is pruned its detection row survives: your counts, species lists and analytics are unaffected, only the audio is removed.

## Danger zone

Two destructive actions live in a clearly marked danger zone, each gated behind an explicit confirmation:

- **Clear all detections** — empties the detections and notification tables; settings are kept.
- **Clear extracted audio** — deletes the saved WAV clips; the detection records stay.

There is no undo. Take a full backup first.
