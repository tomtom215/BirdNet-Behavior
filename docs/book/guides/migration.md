# Migrating from BirdNET-Pi

You can move a BirdNET-Pi station here and keep what it had: its **settings**, its **detection history** and the **recordings** that go with it. Each is a step on the **Station → Data** page (`/station/data#import`), each can be done on its own, and each has a command-line twin. Nothing is written until you have seen what it would do and confirmed it, and BirdNET-Pi's files are never changed.

Three things are different from BirdNET-Pi on the first day, whatever you import. Read [What changes whether you like it or not](#what-changes-whether-you-like-it-or-not) once.

## Before you start

1. **The sign-in is `admin`, not `birdnet`.** BirdNET-Pi asks for the user `birdnet` and the password in `CADDY_PWD`. This station's administrator is `admin`. The import does not copy your password (an import never writes one); set the password under **Access**, or, if you put your BirdNET-Pi `birdnet.conf` in place as this station's own file, its `CADDY_PWD` becomes the password of `admin` the next time the station starts.
2. **Stop BirdNET-Pi if it is on the same machine**, so it is not writing while you copy:
   ```bash
   sudo systemctl stop birdnet_*
   ```
3. **Find the files.** In BirdNET-Pi's folder, `~/BirdNET-Pi`:

   | File | Holds |
   |---|---|
   | `birdnet.conf` | the settings (`/etc/birdnet/birdnet.conf` is a link to it) |
   | `apprise.txt` | where notifications go |
   | `body.txt` | the notification message |
   | `include_species_list.txt`, `exclude_species_list.txt`, `whitelist_species_list.txt` | the three species lists |
   | `scripts/birds.db` (or `scripts/BirdDB.txt`) | the detections |

   and the recordings in `~/BirdSongs/Extracted/By_Date`. If BirdNET-Pi is on another machine, copy these to a place this station can reach, or upload them from your browser.

## Step 1 — your settings

On **Station → Data**, under *Bring your BirdNET-Pi settings*, choose `birdnet.conf` and any of the other files (select several at once; they are recognised by their names), or paste the contents of `birdnet.conf`, or give the folder on this device. Press **Show me what would change**.

You get a table with a row for every key in your file: what it is called there, your value, what it becomes here, and what happens to it. Credentials are shown only as their length. The rows are:

| Verdict | Meaning |
|---|---|
| **Will be set** | Carried over as it is. |
| **Will be set, converted** | Carried over, with the value changed on the way (a unit, a name, a rule); the row says how. |
| **Look at this** | Carried over or not, but there is something to read first. |
| **Goes in birdnet.conf** | This station reads it only from its own `birdnet.conf` (the Flickr key, for example); the lines come back as a download after you apply. |
| **Not accepted here** | The Settings page would refuse the value, and the row says why. Nothing is set from it. |
| **Not carried over** / **Nothing to import** | There is no equivalent, or nothing to carry; the row says which and why. |

Press **Apply these settings** and the same checks the Settings page runs are run over the whole set; if any value would be refused, **none** is written. Then restart the station if it says some settings need it.

The species lists are **added to** the lists this station already has, never replacing them, and `apprise.txt` is copied into the data folder (as `apprise-from-birdnet-pi.txt`, readable by the station only) and used as the Apprise configuration. **That copy is not part of the full backup**, which holds the database, `birdnet.conf` and the recordings: the setting that points at it is in the database, so a station restored from a backup onto another machine needs `apprise.txt` copied in again, or its notifications stop. The file is written (to a temporary name, then renamed into place) only after every setting has passed the Settings page's checks, and just before they are saved. A second import applied after you have changed something by hand sets the BirdNET-Pi values again; the preview shows what each row replaces.

From a terminal, the same, looking only:

```bash
birdnet-behavior --import-birdnet-pi-conf ~/BirdNET-Pi
```

and with `--apply-import` it writes. It prints the same table, exits 0 when it showed a plan or finished, 1 when the settings would be refused (nothing written), 2 for a path it cannot use.

If you run `--doctor` on a `birdnet.conf` copied over as it is, every key BirdNET-Pi writes now draws an answer: the key to rename it to, the setting to use instead, or why there is nothing.

## Step 2 — your detection history

BirdNet-Behavior imports an existing BirdNET-Pi history safely and **non-destructively** — the source database is opened read-only and no row of it is ever changed. (If it is in WAL mode, SQLite may create `-shm` and `-wal` bookkeeping files beside it while it is read; that is the other reason to stop BirdNET-Pi first. An uploaded file is held in `migration-staging`, next to this station's own database, and removed together with those files once it is imported or replaced.) It can import *your own* station's history, which is the common case and needs almost no thought, or *another* station's, which needs two answers from you before it will be right.

### The common case: your own station's history

1. Open **Settings → Data → Import**, or go straight to `/station/data#import`.
2. Give it the file — either **Upload** it, or switch to **Server Path** and type the absolute path to `~/BirdNET-Pi/BirdDB.txt` (or `birds.db`).
3. **Review the preview** — schema, total detections, unique species, date range, duplicate count, and a checklist with ✔ / ⚠ / ✘ against each validation.
4. Click **Start Import**.
5. Verify the per-species count comparison when it finishes.

Leave the two **"Where did this recording come from?"** fields blank. Blank means "this is this station, on this clock", which is what makes the import a no-op reconciliation.

Duplicate rows are skipped, so **re-running the import is safe** — if it is interrupted, run it again.

### Importing another station's history

This is a different operation and the form treats it as one. Two facts drive everything below:

- **BirdNET-Pi stores local wall-clock time with no time zone.** A history recorded at UTC−5 and read by a UTC+1 station is six hours out, and nothing in the file says so. Every hour-of-day analytic — the dawn chorus, the heat map, sessionisation, peak windows — would average the two clocks together.
- **A different place is a different set of birds.** Sunrise, habitat and species pool all differ, so "first of year" and species richness stop meaning one thing.

#### Tell it where the file came from

Fill in both fields before importing:

| Field | What to put |
|---|---|
| **Source station name** | Anything that will mean something to you later — "Hollow Oak, north transect". Stored with the batch and shown beside it. |
| **Source station's UTC offset** | The source station's *standard* offset, in seconds, east-positive. UTC−5 is `-18000`; UTC+1 is `3600`. |

Each timestamp is then converted individually — source local → the real UTC instant → *this* station's local time for that instant — so the destination half of the conversion is exactly right on both sides of every daylight-saving change **this** station observes.

> **What a single offset cannot do.** If the source station observed daylight saving, roughly half its history carries a different real offset than one number can describe, and those rows land an hour out. Recovering that needs the source's IANA time zone, which BirdNET-Pi does not record. The number you enter is stored with the batch, and the whole import can be removed, so this is recoverable — but it is not automatic.

#### What the validator will tell you

The preview compares the file's coordinates against this station's and reports:

- a **⚠ warning** when the source is far enough away to be a different site — the batch is then tagged `NNN km away` in the provenance list;
- a **✘ failure**, which refuses the import, when the file *itself* contains detections from several different coordinates. Such a file is already a merge of several sites and cannot be attributed to one place.

The distance check is a warning, not a block: merging two sites is a legitimate thing to want, and only you can say whether two coordinates are one station whose GPS fix moved or two sites a county apart.

#### Decide whether imported rows count

Under **Imported histories** there is one checkbox:

> **Keep imported detections out of the analytics**

- **Off (the default)** — imported detections count as this station's own everywhere: life list, first-of-year, species richness, phenology, the heat map, co-occurrence, the dawn chorus.
- **On** — they stay in the database and in the Recordings browser, but stop contributing to any of those.

Turn it **on** when you have imported another site and want your station's own numbers back. It applies to both the detection database and the DuckDB analytics copy, so the two cannot disagree.

#### Undoing an import

Every imported detection is tagged with the batch that brought it in. Each batch in the **Imported histories** list has a **Remove** action that deletes exactly those rows — nothing this station heard itself is touched. Merging another site's history is a decision you can take back.

### After the history import

The import writes back-dated history straight to SQLite, so the DuckDB analytics copy is **rebuilt** afterwards — otherwise the incremental startup sync would skip every imported row as "older than the latest already synced". The progress card shows this as a second stage; on a large history it takes a while, and the behavioural and time-series pages are thin until it finishes.

## Step 3 — your recordings

The history import copies rows. Each row keeps its `File_Name`, but the recording it names is still in BirdNET-Pi's folder, so an imported detection would have nothing to play and no spectrogram. The third step, **Your recordings**, brings them across.

BirdNET-Pi lays its clips out as `~/BirdSongs/Extracted/By_Date/<date>/<Common_Name>/<File_Name>`, with a picture `<File_Name>.png` beside each and a `By_Date/shifted/` folder of pitch-shifted copies. This station keeps clips in one folder under the same `File_Name`, and draws a spectrogram from the audio when you look at one. So the step takes the **audio** of the detections you imported (never a clip no detection names, never the pictures, never the shifted copies) and puts it where this station serves clips from.

Give it BirdNET-Pi's folder (`~/BirdSongs`, or the `Extracted` or `By_Date` folder inside it) and choose what to do with each file:

| Choice | What happens | Needs |
|---|---|---|
| **Copy** | A copy; BirdNET-Pi keeps its own. | room for the copy |
| **Link** | The same file under a second name; nothing is copied. | BirdNET-Pi's folder and this station's recordings on one disk |
| **Move** | The file is copied, then removed from BirdNET-Pi's folder. BirdNET-Pi loses it. | room for the copy; the button asks you to confirm |
| **Only count them** | Nothing is touched; it reports what is there. | |

**Check first** runs the same code with the writing switched off and tells you how many recordings it found, how much space they take, how many are already here and how many it could not find. **It also refuses a copy that would take the disk past the point at which this station starts deleting its oldest clips** (`DISK_PURGE_THRESHOLD`, 95 % unless you changed it): copying past that would begin a purge of the history you just brought, and of the station's own recordings. It warns when some of the clips are older than the *Keep clip audio* setting, which would delete them at the next cleanup.

It is careful with your files:

- A name already here with the same size is "already there"; with another size it is a **conflict** and is left alone. It **never overwrites**.
- A copy is written under a temporary name and renamed into place, so a run that stops leaves either a whole clip or a leftover the next run removes. **Run it again to carry on.**
- A file name from the database is used only if it is a plain file name; one with a separator, a `..`, a leading dot or no audio extension is skipped. A link in BirdNET-Pi's folder that points out of it is not followed.
- A clip that is not where its row says (a species folder renamed since) is found by name in the tree.

From a terminal:

```bash
birdnet-behavior --import-birdnet-pi-clips ~/BirdSongs                      # look
birdnet-behavior --import-birdnet-pi-clips ~/BirdSongs --apply-import        # copy
birdnet-behavior --import-birdnet-pi-clips ~/BirdSongs --clips-mode hardlink --apply-import
```

**Not carried across:** BirdNET-Pi's *protected* clips (`disk_check_exclude.txt`) are not turned into locked clips here. Lock the ones you want to keep on the Recordings page; a locked clip is never purged.

## What changes whether you like it or not

These are not options the import can hide. The mapping table below says it key by key; this is the short list.

- **The sign-in is `admin`**, as above.
- **Clip links.** BirdNET-Pi serves its clips from a public, predictable address, `/By_Date/<date>/<species>/<file>`, which anyone on the network can browse without signing in. This station serves a clip at `/api/v2/recordings/<file>` and shares one as a signed link, `/r/<token>`, that expires. **A bookmark or a link from another page to an old `/By_Date/...` address no longer works.** Notifications sent from here carry the new links.
- **Clip format.** BirdNET-Pi saves `mp3` by default; a new install here saves `wav`. The import sets the format you had (`AUDIOFMT`). `mp3`, `flac` and `ogg` need `ffmpeg` or `sox` on the station, and a clip falls back to WAV with a log line without one.
- **Notifications.** BirdNET-Pi's three switches — every detection, a species seen five times or fewer in a week, the first of a species each day — are one list of rules here. When more than one matches a detection, **one** message is sent that names every reason (`$reason`); BirdNET-Pi sends one per reason. All three switched off is no notifications, as there. **A `birdnet.conf` that was never edited has all three at 0** (that is how BirdNET-Pi installs it), so importing it turns notifications off here; the preview says so in a row of its own at the top, and a notification rule is switched on under Settings, Notifications. BirdNET-Pi has no minimum confidence for notifications and notifies for every detection it stores; this station has one (0.80 unless set), so the import sets it to 0 and the same detections notify. BirdNET-Pi counts "the last 7 days" as 8 calendar days; this counts 7. `$confidence` prints two decimals here, BirdNET-Pi's raw number there. The weekly report is a weekday here, not Monday at 01:00. `APPRISE_ONLY_NOTIFY_SPECIES_NAMES` is, despite its name, the **exclusion** list and the `_2` key the inclusion list; the importer knows.
- **The occurrence filter** is version 2 here; BirdNET-Pi's default is version 1, which is not available as ONNX. The week it is asked about is also counted differently (this station uses a 48-week year, BirdNET-Pi the ISO week), and not only in December. Measured with BirdNET-Pi's own range models, the lists differ on most days of the year: the calendar alone on 219 to 341 days a year, by a mean of 5 to 33 species; version 1 against version 2 on every day, by a mean of 17 to 141. Over the 120 clips tested, BirdNET-Pi's own stored rows were the same under all three combinations, but those clips held no bird that falls between the lists, so a rare or out-of-season bird may differ. [Same audio, same detections](./parity.md) has what was measured, and Species, *Why isn't a bird heard here?* shows it for any bird.
- **The species cap** (`MAX_FILES_SPECIES`) keeps the N **newest** clips of a species here and never a locked one; BirdNET-Pi keeps the N most **confident** (plus everything from the last 8 days and every protected clip). If you use it, read the row.
- **When the disk is full**, `FULL_DISK=purge` removes the oldest clips of the most-recorded species first, never below five clips of a species and never a locked one (BirdNET-Pi removed about a day's worth of the oldest files at a time). `FULL_DISK=keep` stops recording and deletes nothing, as in BirdNET-Pi, and records again by itself once there is room. While it is stopped the station logs an error, sends an alert if notifications are set up, and lists it at `/api/v2/health/conditions` (`birdnet_recording_held` is 1 in the metrics).
- **Privacy.** BirdNET-Pi's `PRIVACY_THRESHOLD` is a percentage of the species list; here it is a confidence, and the rule BirdNET-Pi applies is `PRIVACY_RANK`. The import converts it (`max(10, 60 × value)`) and sets the confidence rule to 0.
- **Position.** BirdWeather uploads carry this station's position, as before. A position of exactly 0, 0 (what BirdNET-Pi writes when it cannot find one) is not copied.
- **The network name.** The station advertises itself on the local network as a web service named `<name> on <host>` while `avahi-daemon` runs; its address is `http://<this machine's name>.local`. BirdNET-Pi's `avahi-alias` published a second name, `birdnet.local`; this does not.

## The mapping table

Every key BirdNET-Pi writes into `birdnet.conf`, what it does there (from its own code), and what becomes of it. This table is **generated** from the importer's table in the source (`birdnet_migrate::conf::RULES`); a test fails when this page and the importer differ, so what is written here is what the importer does. Keys marked as secrets are never shown or logged by the importer.

<!-- mapping-table:start -->
| BirdNET-Pi key | What it does there | Here | What changes |
|---|---|---|---|
| `SITE_NAME` | title in the web interface's banner (`common.php`) | setting `site_name` | BirdNET-Pi's default is the shell variable `$HOSTNAME`, which is not copied as text. |
| `LATITUDE` | position for the occurrence filter and the weather of a detection | setting `latitude` | A position of 0, 0 (what BirdNET-Pi writes when it cannot find one) is not copied: a station without a position runs without the occurrence filter here, where BirdNET-Pi asked the model about the Gulf of Guinea. |
| `LONGITUDE` | position for the occurrence filter and the weather of a detection | setting `longitude` | See `LATITUDE`. |
| `MODEL` | which classifier runs: BirdNET V1, V2.4 FP16 (the default), Perch v2 or a BirdNET-Go classifier (`utils/models.py`) | not carried over (see the note) | The classifier is chosen on the Classifier page, not copied: a new install here runs BirdNET V2.4, the same network BirdNET-Pi runs by default (32-bit floats where BirdNET-Pi uses 16-bit). |
| `SF_THRESH` | how likely a species must be here, this week, to be accepted (occurrence filter threshold) | setting `sf_thresh` | The same number with the same meaning, but see `DATA_MODEL_VERSION`. |
| `DATA_MODEL_VERSION` | which occurrence model: 1 (the default) or 2 (`utils/models.py: get_meta_model`) | not carried over (see the note) | Version 1 is not offered here (it is not hosted as ONNX): this station uses version 2 with V2.4, and asks it about a week of its own 48-week year, not the ISO week. Measured with BirdNET-Pi's own range models: the species lists differ on most days of the year (the calendar alone, on 219 to 341 days a year, by a mean of 5 to 33 species; version 1 against version 2 on every day, by a mean of 17 to 141). On the 120 clips tested, BirdNET-Pi's own stored rows were the same under all three (version 1 with the ISO week, version 2 with the ISO week, version 2 with the 48-week year), and a bird that falls between the lists was not among them (see "Same audio, same detections"). |
| `BIRDWEATHER_ID` *(a secret; never shown or logged)* | station ID for uploading detections and soundscapes to BirdWeather (`reporting.py`) | setting `birdweather_token` | Uploads resume, with this station's position, as before. Without this the uploads stop and nothing says so. |
| `HEARTBEAT_URL` *(a secret address; only its scheme and host are shown, never logged)* | address pinged after every analysis (`reporting.py`) | setting `heartbeat_url` | Pinged on a schedule here rather than after every analysis. Such addresses carry the monitor's key, so the preview shows only its scheme and host. |
| `FLICKR_API_KEY` *(a secret; never shown or logged)* | key for the Flickr image provider | `FLICKR_API_KEY` in `birdnet.conf` | Read from `birdnet.conf` only: there is no field for it in the browser. |
| `FLICKR_FILTER_EMAIL` | show only this Flickr user's photographs | `FLICKR_FILTER_EMAIL` in `birdnet.conf` | Read from `birdnet.conf` only. |
| `IMAGE_PROVIDER` | where species pictures come from: `WIKIPEDIA`, `FLICKR`, or blank for none | `IMAGE_PROVIDER` in `birdnet.conf` | Read from `birdnet.conf` only, in either case. Blank means no pictures in BirdNET-Pi and Wikipedia here. |
| `INFO_SITE` | which site a species name links to: `ALLABOUTBIRDS` or `EBIRD` (`common.php`) | setting `info_site` | Lower-case here. This station's own default is eBird, so it is set explicitly. |
| `DATABASE_LANG` | language of the common names stored with a detection | setting `database_lang` | Names are translated when shown here, not when stored; the history you import keeps the names it was recorded with. |
| `CADDY_PWD` *(a secret; never shown or logged)* | password of the `birdnet` user for BirdNET-Pi's web interface (Caddy basic auth, `common.php`) | not carried over (see the note) | Not copied, so no import ever writes a password. This station's administrator is `admin`, not `birdnet`; set its password under Access. (If you put this file in place as this station's own `birdnet.conf`, `CADDY_PWD` becomes the password of `admin` the next time the station starts.) |
| `ICE_PWD` *(a secret; never shown or logged)* | password between BirdNET-Pi's streaming programs (Icecast) | no equivalent | This station streams from inside one program; there is nothing to authenticate. |
| `BIRDNETPI_URL` | public address of the site; used by Caddy and as the base of a notification's links (`notifications.py`) | not carried over (see the note) | Read from the environment as `BNB_PUBLIC_URL`, not from a setting. Notification links are built from it; without it they use `http://<hostname>.local`. |
| `REC_CARD` | sound card for the recorder: `arecord -D`, or `default` (`birdnet_recording.sh`) | setting `alsa_device` | `default` is the system's default input, as in BirdNET-Pi. Clear the field to let this station find a USB microphone by itself. |
| `RTSP_STREAM` *(a secret address; only its scheme and host are shown, never logged)* | comma-separated network audio streams that replace the sound card (`birdnet_recording.sh`) | setting `rtsp_urls` | Comma-separated here too. The preview shows only each address's scheme and host; the user name, password and path are not shown. |
| `RTSP_STREAM_TO_LIVESTREAM` | position of the stream the live-audio page plays (0 = the first) | not carried over (see the note) | Streams are not numbered here: choose the default live source again (`livestream_source`). |
| `CHANNELS` | channel count the recorder asks the sound card for (`arecord -c`) | not carried over (see the note) | Per audio source here: the station reads the device's channels and reduces stereo to mono by a rule chosen on the Audio page (`--channel-report` records and compares). |
| `RECORDING_LENGTH` | length in seconds of each recording BirdNET-Pi analyses (3 to 60) | setting `segment_duration` |  |
| `EXTRACTION_LENGTH` | seconds of audio saved around a detection; blank = 6 | setting `extraction_length` |  |
| `AUDIOFMT` | format of the saved clip, any `sox` format (`reporting.py: extract_detection`) | setting `audio_format` | BirdNET-Pi's default is mp3; a new install here saves WAV. This station writes wav, mp3, flac and ogg; mp3, flac and ogg need ffmpeg or sox, and fall back to WAV with a log line without one. |
| `CONFIDENCE` | lowest confidence stored as a detection (`analysis.py`) | setting `confidence_threshold` |  |
| `SENSITIVITY` | sigmoid sensitivity of the classifier, 0.5 to 1.5 | setting `sensitivity` | The same number, with the same meaning, on V2.4. |
| `OVERLAP` | seconds two analysis windows overlap, 0 to 2.9 | setting `overlap` |  |
| `PRIVACY_THRESHOLD` | percent of the species list within which a human sound blanks a window (`analysis.py: filter_humans`) | setting `privacy_rank` and `privacy_threshold` = 0 | A percentage there, a rank here: `privacy_rank` is `max(10, 60 x value)`, the same rule. `privacy_threshold`, a confidence here, is set to 0 so the value is not read twice, in two scales. |
| `FULL_DISK` | what to do at the purge threshold: `purge` the oldest day's clips or `keep` everything and stop the services (`disk_check.sh`) | setting `full_disk_action` | `keep` here stops recording and deletes nothing when the disk is full, and records again by itself once there is room. `purge` removes the oldest clips of the most-recorded species first, never below 5 clips of a species, never a locked clip. |
| `PURGE_THRESHOLD` | disk percentage at which `disk_check.sh` acts; 95 if unset | setting `purge_threshold` | Called `DISK_PURGE_THRESHOLD` in this station's `birdnet.conf`. |
| `MAX_FILES_SPECIES` | clips kept per species; 0 keeps all (`disk_species_clean.sh`) | setting `max_files_per_species` | A different rule: BirdNET-Pi keeps the N most **confident** clips of a species, plus everything from the last 8 days and every protected one. This station keeps the N **newest** clips of a species, and never a locked one. A BirdNET-Pi "protected" clip is a locked one here, but the move does not carry the protection across: lock the clips you want to keep once they are imported. |
| `APPRISE_NOTIFICATION_TITLE` | title of a notification (`notifications.py`) | setting `notify_title_template` | The same `$comname`, `$sciname`, `$reason` and the rest are understood. `$confidence` prints two decimals here, BirdNET-Pi's raw number there. |
| `APPRISE_NOTIFICATION_BODY` | body of a notification, in releases before September 2025 (now `body.txt`) | setting `notify_body_template` | Later releases keep the body in `body.txt`, which the import reads too. |
| `APPRISE_NOTIFY_EACH_DETECTION` | send a notification for every detection (`notifications.py`) | setting `notify_trigger` | The three switches are one setting here, a list of rules. When more than one rule matches a detection, one message is sent that names every reason; BirdNET-Pi sends one message per rule. All three off means no notifications at all, as there. |
| `APPRISE_NOTIFY_NEW_SPECIES` | send a notification for a species seen five times or fewer in the last 7 days, this detection included | setting `notify_trigger` | See `APPRISE_NOTIFY_EACH_DETECTION`. BirdNET-Pi counts the last 8 calendar days (`Date >= today - 7`); this station counts 7. |
| `APPRISE_NOTIFY_NEW_SPECIES_EACH_DAY` | send a notification for the first detection of a species each day | setting `notify_trigger` | See `APPRISE_NOTIFY_EACH_DETECTION`. |
| `APPRISE_WEEKLY_REPORT` | send the weekly report, Mondays at 01:00 (cron, `weekly_report.sh`) | setting `weekly_report_schedule` | A weekday here, not an hour. 1 becomes `monday`, 0 `disabled`. |
| `APPRISE_MINIMUM_SECONDS_BETWEEN_NOTIFICATIONS_PER_SPECIES` | seconds before the same species may notify again; 0 = no limit | setting `notify_cooldown` | BirdNET-Pi's default is no limit; this station's is 300 seconds, so the value is always set explicitly. |
| `APPRISE_ONLY_NOTIFY_SPECIES_NAMES` | species that are **never** notified (an exclusion list, despite the name; `notifications.py: should_notify`) | setting `notify_species_exclude` | Common names, comma-separated, as in BirdNET-Pi. |
| `APPRISE_ONLY_NOTIFY_SPECIES_NAMES_2` | species that are the **only** ones notified (an inclusion list; `notifications.py: should_notify`) | setting `notify_species_only` | Common names, comma-separated, as in BirdNET-Pi. |
| `RAW_SPECTROGRAM` | draw spectrograms without axes and labels (`spectrogram.sh`) | setting `raw_spectrogram` | This renderer draws no axes in either mode; the setting hides the species and confidence text. |
| `RARE_SPECIES_THRESHOLD` | days without a sighting after which a species is called rare (`overview.php`) | setting `rare_species_days` |  |
| `COLOR_SCHEME` | light or dark page colours for everyone (`common.php`) | no equivalent | Chosen per browser here (the Display menu), so each person has their own. |
| `CUSTOM_IMAGE` | the picture shown on the Overview page, a path or a web address (`overview.php`) | setting `dashboard_image` | BirdNET-Pi also accepts a web address (it reads the picture with `file_get_contents`); this station reads a file on this device, so a web address is not carried over. |
| `CUSTOM_IMAGE_TITLE` | the heading over that picture | setting `dashboard_image_title` |  |
| `FREQSHIFT_TOOL` | how a clip is pitch-shifted when you ask for it on the Recordings page: `sox` or `ffmpeg` | no equivalent | BirdNET-Pi makes a shifted copy of a clip on demand. `freq_shift_hz` here rewrites every **new** clip as it is saved, which is not the same, so it is not set. |
| `FREQSHIFT_HI` | upper frequency for the `ffmpeg` shift | no equivalent | See `FREQSHIFT_TOOL`. |
| `FREQSHIFT_LO` | lower frequency for the `ffmpeg` shift | no equivalent | See `FREQSHIFT_TOOL`. |
| `FREQSHIFT_PITCH` | pitch change for the `sox` shift, in hundredths of a semitone | no equivalent | See `FREQSHIFT_TOOL`. |
| `FREQSHIFT_RECONNECT_DELAY` | milliseconds the live page waits when it switches the live stream's shift | no equivalent | See `FREQSHIFT_TOOL`. |
| `ACTIVATE_FREQSHIFT_IN_LIVESTREAM` | pitch-shift the live audio | no equivalent | See `FREQSHIFT_TOOL`. |
| `AUTOMATIC_UPDATE` | update BirdNET-Pi from git every Sunday at 03:00 (`update_birdnet.sh`) | not carried over (see the note) | This station does not update itself from the network. Updates are applied by you, from the update page. |
| `SILENCE_UPDATE_INDICATOR` | hide the number of commits behind | no equivalent | There is no such indicator. |
| `LogLevel_BirdnetRecordingService` | `ffmpeg` log level of BirdNET-Pi's recording service | no equivalent | BirdNET-Pi's three log levels are for the programs it runs; this station is one program with one log level (`LOG_LEVEL`). |
| `LogLevel_LiveAudioStreamService` | `ffmpeg` log level of BirdNET-Pi's live-stream service | no equivalent | See `LogLevel_BirdnetRecordingService`. |
| `LogLevel_SpectrogramViewerService` | `sox` log level of BirdNET-Pi's spectrogram service | no equivalent | See `LogLevel_BirdnetRecordingService`. |
| `BIRDNET_USER` | the Unix user BirdNET-Pi runs as | no equivalent | Internal to BirdNET-Pi. |
| `RECS_DIR` | where BirdNET-Pi records and finds its audio | no equivalent | This station's recordings directory is its own. Give the old folder to the recordings step of the move to bring the clips across. |
| `PROCESSED` | where analysed recordings wait to be deleted | no equivalent | Internal to BirdNET-Pi. |
| `EXTRACTED` | where BirdNET-Pi keeps its clips (`By_Date/...`) | no equivalent | Not read by this station. Give the old install's folder to the recordings step of the move to bring the clips across. |
| `IDFILE` | BirdNET-Pi's list of species identified so far | no equivalent | Internal to BirdNET-Pi. |
| `LAST_RUN` | debugging aid | no equivalent | Internal to BirdNET-Pi. |
| `THIS_RUN` | debugging aid | no equivalent | Internal to BirdNET-Pi. |
| `PUSHED_APP_KEY` *(a secret; never shown or logged)* | key for the Pushed.co service, dropped in 2022 | no equivalent | Pushed.co is not supported; use a notification address. |
| `PUSHED_APP_SECRET` *(a secret; never shown or logged)* | secret for the Pushed.co service, dropped in 2022 | no equivalent | See `PUSHED_APP_KEY`. |
| `DB_PWD` *(a secret; never shown or logged)* | password of a database BirdNET-Pi no longer uses | no equivalent | This station uses SQLite. |
| `DB_ROOT_PWD` *(a secret; never shown or logged)* | root password of a database BirdNET-Pi no longer uses | no equivalent | See `DB_PWD`. |
| `DO_RECORDING` | switch for BirdNET-Pi's recorder | no equivalent | Internal to BirdNET-Pi. |
| `DO_EXTRACTIONS` | switch for BirdNET-Pi's clip extraction | no equivalent | Internal to BirdNET-Pi. |
| `BIRDNETLOG_URL` | address of BirdNET-Pi's log viewer | no equivalent | There is no separate log viewer. |
| `WEBTERMINAL_URL` | address of BirdNET-Pi's web terminal | no equivalent | This station has no web terminal. |
| `EXTRACTIONLOG_URL` | address of BirdNET-Pi's extraction log | no equivalent | There is no separate log viewer. |
| `EXTRACTIONS_URL` | address of BirdNET-Pi's extractions | no equivalent | Clips are served by this station itself. |
| `VENV` | BirdNET-Pi's Python environment | no equivalent | Internal to BirdNET-Pi. |
| `INSTALL_DATE` | when BirdNET-Pi was installed | no equivalent | Internal to BirdNET-Pi. |
| `ANALYZED` | where BirdNET-Pi keeps analysed files | no equivalent | Internal to BirdNET-Pi. |
| `TIMESTAMP_FORMAT` | date format in an old release's file names | no equivalent | Internal to BirdNET-Pi. |
<!-- mapping-table:end -->

## What carries over, and what does not

**Carries over:** every detection row — date, time, scientific and common name, confidence, coordinates, and the BirdNET parameters (cutoff, week, sensitivity, overlap) — with the history import; the settings, species lists and notification files with the settings import; the recordings of the imported detections with the third step.

**Two columns are not the same kind of number afterwards.** An imported row keeps the `Week` BirdNET-Pi wrote, the ISO week (8 for 16 February 2026, 53 for 31 December 2026). A detection this station records stores the week of its own 48-week year (7 and 48 for those two days). The import does not convert them, so that an export of the imported rows matches your old install's. A query that groups or filters by `Week` across imported and new rows therefore mixes two calendars. Imported rows are the ones with an `import_batch_id` (new ones have none), so either leave them out, or work the week out from `Date` instead. Likewise `Confidence`: BirdNET-Pi stores four decimals, and this station stores the score unrounded.

**Does not carry over:** BirdNET-Pi's own spectrogram pictures (this station draws its own), the pitch-shifted copies, the `confirmed_species_list.txt` list, the protected-clip list, the Streamlit, Icecast, web terminal, file manager and Adminer tools it ships, and its sign-in. Its installation is left untouched and safe to restart.
