# If the SD Card Dies

An SD card in a field station wears out, and when it goes it takes everything on it: the database, the recordings, the configuration file, and the station's own snapshots, which live on the same card. What you get back depends entirely on the copies you made *before* it failed. This page says what each copy holds, what to keep somewhere else, and how to put a station back together on a new card with the commands and screens that exist today.

The commands below are for a bare-metal install. A Docker station keeps its data in the `birdnet-data` volume (the database is `/data/BirdNet-Behavior/birds.db`), so a dead disk under Docker is the same problem with the same answer: only what you copied off the machine survives.

## What each kind of backup holds

| Backup | How it is made | Holds | Where it lives |
|---|---|---|---|
| **Snapshot** | Automatically, every 7 days by default (`BACKUP_SCHEDULE`); 14 kept | The database only | `backups/` beside `birds.db`, **on the same card**: it dies with it |
| **Full backup** | You press **Download full backup** on **Settings → Data** (`/station/data`) | The database (a consistent snapshot), the `recordings/` folder with every clip (or only the kept ones, or none, if you chose), the configuration file the station reads, and a manifest with every clip's checksum | Wherever you saved the download. It is not encrypted |
| **Offsite copy** | Automatically with each snapshot, once `OFFSITE_BACKUP` is set ([Offsite backups](./backups.md#offsite-backups)) | The database only (which records each clip's checksum, not the audio), encrypted with `OFFSITE_PASSPHRASE` | Your bucket or SSH server |

A full backup takes the configuration file the station was started with (`--config`; `/etc/birdnet/birdnet.conf` on a bare-metal install), or, when that names no file, a `birdnet.conf` beside the database. A full backup made before this release took only the second, so on a standard install it holds no configuration. Offsite copies carry neither recordings nor the configuration.

### What comes back with the database

The database is more than the detections. Restoring it brings back:

- every detection, with its review verdict, comments and lock;
- every setting saved in the browser — location, thresholds, species lists, notification settings, the BirdWeather token and the email password among them. Once a station has started, a setting stored here wins over the same key in the configuration file;
- the accounts and their password hashes, the admin account's included;
- the audio sources set up in **Settings → Capture**.

Because it holds those credentials, treat a full backup like a password file: it is a plain `.tar.gz`.

### What no backup holds

- **The configuration file**, and with it everything that is only there: the admin password the installer generated (`CADDY_PWD`), the offsite settings and their secrets (`OFFSITE_PASSPHRASE`, `OFFSITE_S3_SECRET_KEY` and the rest), the API token (`BNB_API_TOKEN`), and any other key you set only in the file.
- **The SSH key** an SFTP or rsync offsite copy logs in with, if you keep it in the data folder as [Offsite backups](./backups.md#to-an-ssh-server) suggests.
- **The model files.** The installer downloads them again.
- **The analytics copy** (`analytics.db`). At start the station compares it with the database and rebuilds the days that differ, so it follows a restored database by itself.
- **Sign-ins.** Every browser signs in again.

## What to keep off the station

Before the card fails, keep these somewhere that is not the station:

1. **A recent full backup.** It is the only copy of the recordings. Download one after any large change and periodically after that.
2. **`OFFSITE_PASSPHRASE`.** Without it an offsite copy cannot be opened by anyone, you included. The station keeps it only in its own configuration file, which is on the card that died.
3. **The offsite credentials** (`OFFSITE_S3_ACCESS_KEY` and `OFFSITE_S3_SECRET_KEY`, or the SSH key), so the new station can write to the same place and you can read the old copies from it.
4. **The admin password.** A restored database brings the old station's admin password back with it (see step 5 below). Losing it costs nothing but a reset: no detection, setting or recording depends on it.
5. **A copy of `/etc/birdnet/birdnet.conf`.** It holds `CADDY_PWD` and the offsite passphrase in plain text, so keep it as carefully as those. `sudo cat /etc/birdnet/birdnet.conf` prints it.

## Putting a station back on a new card

1. **Install the station as if it were new**, following [Installation](../getting-started/installation.md). If the old station ran the BirdNET+ V3.0 preview, install with `--classifier birdnet-v3`, because a new install runs BirdNET V2.4. The location and microphone questions can be skipped: the restored database brings its own settings.
2. **Sign in** at `/login` with the password the installer printed (username `admin`).
3. **If you have a full backup:** open **Settings → Data** (`/station/data`), choose the archive under **Upload a full backup**, press **Restore from archive** and confirm. The station checks that the disk can hold the archive, unpacks it beside the database, checks the database copy, swaps it in and merges the recordings, then restarts itself under systemd to load it. [Restoring](./backups.md#restoring) has the detail.
4. **If you have only an offsite copy:** download the newest `birds.db.backup.<timestamp>.bnb` from your bucket or server to the new station, then, in a shell on the station, in the folder you downloaded it to:

   ```bash
   SVC=$(systemctl show -p User --value birdnet-behavior)
   DATA=$(getent passwd "$SVC" | cut -d: -f6)/BirdNet-Behavior
   read -rsp 'Offsite passphrase: ' BIRDNET_OFFSITE_PASSPHRASE; echo
   export BIRDNET_OFFSITE_PASSPHRASE
   birdnet-behavior --decrypt-backup birds.db.backup.<timestamp>.bnb --out restored.db
   unset BIRDNET_OFFSITE_PASSPHRASE
   sudo systemctl stop birdnet-behavior
   sudo mv "$DATA/birds.db" "$DATA/birds.db.fresh-install"
   sudo rm -f "$DATA/birds.db-wal" "$DATA/birds.db-shm"
   sudo cp restored.db "$DATA/birds.db"
   sudo chown "$SVC": "$DATA/birds.db"
   sudo systemctl start birdnet-behavior
   ```

   `--decrypt-backup` does not ask for the passphrase: it reads `OFFSITE_PASSPHRASE` from the environment (as `BIRDNET_OFFSITE_PASSPHRASE`) or the configuration file, which on a new station does not have it yet. `read -s` keeps it out of your shell history. The command refuses to write over a file that exists. The `-wal` and `-shm` files are removed because a write-ahead log left from the fresh install can be replayed over the restored database when it is opened. This brings back the database only: the recordings are gone unless you also have a full backup.
5. **Sign in again.** If the old station had an admin password, the restored database brings it back, and from the restart that loads it the password the new installer printed no longer works: the station keeps a password it finds changed since the configuration's `CADDY_PWD` was applied, as it would one changed on the Accounts page. Sign in with the old password. If you do not have it, run `sudo bash install.sh reset-password` (with the installer you just used, or `curl --proto '=https' --tlsv1.2 -fsSL https://github.com/tomtom215/BirdNet-Behavior/releases/latest/download/install.sh | sudo bash -s -- reset-password`): it writes a new random `CADDY_PWD`, restarts the station and prints the password. Done by hand, it is the same: set `CADDY_PWD` in `/etc/birdnet/birdnet.conf` to a new value, different from the one there now, and run `sudo systemctl restart birdnet-behavior`; a new value replaces the stored password ([Lost the admin password](./remote-access.md#lost-password)).
6. **Put back what only the configuration file held:** the `OFFSITE_*` lines (and the SSH key and `known_hosts`, for SFTP or rsync), `BNB_API_TOKEN`, and anything else you had set only there. Then restart the service.
7. **Check the microphone** under **Settings → Capture**: the restored database names the old station's audio source, and a different board or microphone may not have it.
8. **Run the diagnostic**, `birdnet-behavior --doctor` (as in [First Steps](../getting-started/first-steps.md#health-check)), and fix what it names.

## What cannot be recovered

- **The station's own snapshots.** They were on the card.
- **The configuration file, without a full backup that holds it.** Restoring a full backup puts its `birdnet.conf` back where the station reads its configuration, keeping the file there as `birdnet.conf.before-restore`; on a fresh card, check that file for anything the new install set that you want to keep (a new `CADDY_PWD`, say). An offsite copy does not hold it, and an older full backup may not.
- **Recordings, without a full backup.** Offsite copies are the database only.
- **An offsite copy, without its passphrase.** It is not stored anywhere but the configuration file, and nothing can derive it.
- **Restoring an offsite copy from the browser.** There is no screen for it; it is the command line in step 4 or nothing.
