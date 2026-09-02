# Backups & Recovery

The **Data** tab (`/station/data`) is where you protect your records: snapshots, exports, storage and the controls you hope you never need.

![The backups and recovery page](../images/admin-backups.png)

## Three kinds of backup

**Snapshots** copy the database only. They are small and quick, they run automatically **every 7 days**, and the station keeps the most recent **14**. They live in `backups/` beside `birds.db` — *on the station itself*. That makes them perfect for undoing a bad import or recovering from database corruption, and useless if the SD card dies.

**Full backups** bundle the database, your recordings and your config into one `.tar.gz` you download. This is the one to keep somewhere else. Nothing takes these automatically — download one after any big change, and periodically if your station matters to you.

**Offsite copies** send each weekly snapshot to an object store or an SSH server, encrypted before it leaves. Off by default; see [Offsite backups](#offsite-backups) below. This is what covers the failure the other two do not: the card wearing out, the enclosure flooding, the Pi being stolen.

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

### Retention

`OFFSITE_KEEP` is how many backups stay at the destination; the oldest go when a new one arrives. `0` keeps everything.

Retention only ever removes files this station wrote — names of the form `birds.db.backup.<timestamp>.bnb`. Anything else in the same bucket prefix or directory is left alone, so you can share a bucket without losing the other things in it.

### Checking it works

```bash
birdnet-behavior --doctor
```

reports the destination, how many backups it will keep, and — for SSH — whether the key exists, whether its permissions are ones OpenSSH will accept, and whether the host is known. It makes no connection: `--doctor` runs on every start, and a diagnostic that dials a remote host fails whenever the uplink is down.

Uploads are logged at `info` on success and `warn` on failure, with the destination named. A failed upload never affects the local backup or the VACUUM that follows it.

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
