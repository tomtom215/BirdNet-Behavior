# Where every setting is changed

BirdNet-Behavior is meant to be set up and looked after from a browser: install it, open its address, and the setup wizard takes the station from first boot to listening. Everything a person would reasonably want to change later has a place under **Settings**. This page is the complete account of that claim, and a test (`tests/every_setting_has_a_home_in_the_browser.rs`) fails when it stops being true.

A setting is changed in the browser, or it is on this page with the reason it is not. The reasons are of three kinds:

* **It could lock the owner out.** The address the page is served on, HTTPS, and the files behind it. A wrong value leaves no page to correct it from.
* **It needs something a browser cannot give the station.** A certificate file, a model file of your own, a directory on the station's disk.
* **It must not live in the database.** The offsite-backup passphrase and keys: every backup contains the settings table, so a secret saved there would sit inside the archive it protects.

## How the settings take effect

Each setting on the Settings pages carries a small badge. **Applies after restart** means the station reads the value once, when it starts: save it, then press *Restart the station now*, which appears as soon as there is a saved change waiting (and after the setup wizard). **Applies immediately** and **Applies within about 30 seconds** mean the running station picks the value up by itself. The badge is not a guess: for each setting it comes from reading the code that consumes the value (the table below, and `crates/birdnet-web/src/routes/admin/settings/applies.rs`, name the evidence).

*Restart the station* works in three situations: under systemd (the service is restarted), and everywhere else the program shuts down cleanly and starts itself again, so a binary run by hand and a container without a restart policy are covered too.


## Saved settings

Every key the settings pages and `PUT /api/v2/settings` write, where it is changed, and when a saved value reaches the thing it controls.

| Setting | Changed at | When it applies |
|---|---|---|
| `confidence_threshold` | Settings → Capture → Detection Settings → Minimum Confidence (0–1) (`confidence_threshold`) | Applies after restart |
| `sensitivity` | Settings → Capture → Detection Settings → Sensitivity (0.5–1.5) (`sensitivity`) | Applies after restart |
| `overlap` | Settings → Capture → Detection Settings → Analysis Overlap (0–2.9 seconds) (`overlap`) | Applies after restart |
| `sf_thresh` | Settings → Capture → Detection Settings → Species Frequency Threshold (0–1) (`sf_thresh`) | Applies after restart |
| `privacy_threshold` | Settings → Capture → Detection Settings → Privacy Threshold (0 = disabled) (`privacy_threshold`) | Applies after restart |
| `privacy_rank` | Settings → Capture → Detection Settings → Privacy Rank ({rank_min} = disabled, up to {rank_max}) (`privacy_rank`) | Applies after restart |
| `confirmation_level` | Settings → Capture → Detection Settings → Repeat Confirmation (`confirmation_level`) | Applies after restart |
| `alsa_device` | Settings → Capture → Audio Capture → Microphone address (`alsa_device`) | Applies after restart |
| `alsa_devices` | Settings → Capture → Audio sources (several microphones are rows there; this key only seeds them at the first start) | Applies after restart |
| `rtsp_url` | Settings → Capture → Audio Capture → Network camera address (`rtsp_url`) | Applies after restart |
| `rtsp_urls` | Settings → Capture → Audio Capture → Several network cameras (separated by commas) (`rtsp_urls`) | Applies after restart |
| `audio_format` | Settings → Capture → Audio Capture → Extracted Clip Format (`audio_format`) | Applies after restart |
| `segment_duration` | Settings → Capture → Audio Capture → Segment Duration (seconds) (`segment_duration`) | Applies after restart |
| `freq_shift_hz` | Settings → Capture → Audio Capture → Frequency Shift (Hz, 0 = disabled) (`freq_shift_hz`) | Applies after restart |
| `clip_target_lufs` | Settings → Capture → Audio Capture → Even out clip volume (`clip_target_lufs`) | Applies after restart |
| `livestream_source` | Recordings → Live (the source picker) | Applies immediately |
| `analytics_exclude_imports` | Settings → Data → Bring your history with you | Applies immediately |
| `today_layout` | Settings → General → Today Page Layout | Applies immediately |
| `latitude` | Settings → Capture → Location & Recording Schedule → Latitude (`latitude`) | Applies after restart |
| `longitude` | Settings → Capture → Location & Recording Schedule → Longitude (`longitude`) | Applies after restart |
| `station_name` | Settings → Capture → Location & Recording Schedule → Station Name (`station_name`) | Applies after restart |
| `recording_schedule` | Settings → Capture → Location & Recording Schedule → Recording window (`recording_schedule`) | Applies after restart |
| `timezone` | Set by the setup wizard's location lookup; the station's clock zone is the operating system's (see below) | Recorded only |
| `onboarding_complete` | Set when the setup wizard finishes | Applies immediately |
| `night_inhibit` | Settings → Capture → Location & Recording Schedule → Night Inhibit (suppress recording in darkness) (`night_inhibit`) | Applies after restart |
| `pre_sunrise_offset` | Settings → Capture → Location & Recording Schedule → Extra minutes before sunrise (`pre_sunrise_offset`) | Applies after restart |
| `post_sunset_offset` | Settings → Capture → Location & Recording Schedule → Extra minutes after sunset (`post_sunset_offset`) | Applies after restart |
| `site_name` | Settings → General → System & Display → Site Name (`site_name`) | Applies after restart |
| `info_site` | Settings → General → System & Display → Species Info Links (`info_site`) | Applies after restart |
| `apprise_url` | Settings → Alerts → Notifications → Address of your Apprise server (`apprise_url`) | Applies after restart |
| `apprise_config` | Settings → Alerts → Notifications → Path to an Apprise configuration file (`apprise_config`) | Applies after restart |
| `notify_urls` | Settings → Alerts → Notifications → Where to send alerts (`notify_urls`) | Applies after restart |
| `birdweather_token` | Settings → Alerts → Notifications → BirdWeather Station Token (`birdweather_token`) | Applies after restart |
| `notify_confidence` | Settings → Alerts → Notifications → Notification Min Confidence (`notify_confidence`) | Applies after restart |
| `notify_cooldown` | Settings → Alerts → Notifications → Notification Cooldown (seconds) (`notify_cooldown`) | Applies after restart |
| `notify_trigger` | Settings → Alerts → Notifications → Notification Trigger (`notify_trigger`) | Applies after restart |
| `notify_species_only` | Settings → Alerts → Notifications → Notify only for these species (common names, separated by commas) (`notify_species_only`) | Applies after restart |
| `notify_species_exclude` | Settings → Alerts → Notifications → Never notify for these species (common names, separated by commas) (`notify_species_exclude`) | Applies after restart |
| `notify_title_template` | Settings → Alerts → Notifications → Notification Title Template (`notify_title_template`) | Applies after restart |
| `notify_body_template` | Settings → Alerts → Notifications → Notification Body Template (`notify_body_template`) | Applies after restart |
| `weekly_report_schedule` | Settings → Alerts → Notifications → Weekly Report Day (`weekly_report_schedule`) | Applies after restart |
| `heartbeat_url` | Settings → Alerts → Notifications → Heartbeat URL (`heartbeat_url`) | Applies after restart |
| `deadman_hours` | Settings → Alerts → Notifications → Dead-man alert after (hours of no detections) (`deadman_hours`) | Applies after restart |
| `database_lang` | Settings → General → System & Display → Common-name language (`database_lang`) | Applies after restart |
| `species_include` | Settings → Capture → Which birds count → Record only these (`species_include`) | Applies within about 30 seconds |
| `species_exclude` | Settings → Capture → Which birds count → Never record these (`species_exclude`) | Applies within about 30 seconds |
| `species_whitelist` | Settings → Capture → Which birds count → Always allowed (`species_whitelist`) | Applies within about 30 seconds |
| `image_cache_dir` | Settings → General → System & Display → Species Image Cache Directory (`image_cache_dir`) | Applies after restart |
| `custom_image_dir` | Settings → General → System & Display → Custom Species Image Directory (`custom_image_dir`) | Applies after restart |
| `dashboard_image` | Settings → General → System & Display → Today page picture (file) (`dashboard_image`) | Applies immediately |
| `dashboard_image_title` | Settings → General → System & Display → Picture title (`dashboard_image_title`) | Applies immediately |
| `max_files_per_species` | Settings → General → System & Display → Max Files Per Species (0 = unlimited) (`max_files_per_species`) | Applies after restart |
| `purge_threshold` | Settings → General → System & Display → Disk Purge Threshold (%) (`purge_threshold`) | Applies after restart |
| `full_disk_action` | Settings → General → System & Display → When the disk is full (`full_disk_action`) | Applies after restart |
| `extraction_length` | Settings → General → System & Display → Clip Length (seconds) (`extraction_length`) | Applies after restart |
| `raw_spectrogram` | Settings → General → System & Display → Bare spectrograms (`raw_spectrogram`) | Applies immediately |
| `spectrogram_colormap` | Settings → General → System & Display → Spectrogram colours (`spectrogram_colormap`) | Applies immediately |
| `spectrogram_range_db` | Settings → General → System & Display → Spectrogram dynamic range (`spectrogram_range_db`) | Applies immediately |
| `time_format` | Settings → General → System & Display → Clock (`time_format`) | Applies immediately |
| `temperature_unit` | Settings → General → System & Display → Temperature unit (`temperature_unit`) | Applies immediately |
| `rare_species_days` | Settings → General → System & Display → Rare after (days) (`rare_species_days`) | Applies immediately |
| `stream_retention_secs` | Settings → General → System & Display → Raw Segment Retention (seconds) (`stream_retention_secs`) | Applies after restart |
| `stream_max_mb` | Settings → General → System & Display → Streaming Folder Limit (MB) (`stream_max_mb`) | Applies after restart |
| `clip_retention_days` | Settings → General → System & Display → Keep Clip Audio (days, 0 = forever) (`clip_retention_days`) | Applies after restart |
| `email_smtp_host` | Settings → Alerts → Email Alerts → Outgoing mail server (SMTP host) (`email_smtp_host`) | Applies after restart |
| `email_smtp_port` | Settings → Alerts → Email Alerts → Port (`email_smtp_port`) | Applies after restart |
| `email_smtp_user` | Settings → Alerts → Email Alerts → Your email address (`email_smtp_user`) | Applies after restart |
| `email_smtp_pass` | Settings → Alerts → Email Alerts → App password (`email_smtp_pass`) | Applies after restart |
| `email_from` | Settings → Alerts → Email Alerts → Send from (`email_from`) | Applies after restart |
| `email_to` | Settings → Alerts → Email Alerts → Send to (`email_to`) | Applies after restart |
| `email_from_name` | Settings → Alerts → Email Alerts → Sender name (`email_from_name`) | Applies after restart |
| `email_starttls` | Settings → Alerts → Email Alerts → Encryption (`email_starttls`) | Applies after restart |
| `email_min_confidence` | Settings → Alerts → Email Alerts → Only email when at least this sure (0–1) (`email_min_confidence`) | Applies after restart |
| `email_cooldown_secs` | Settings → Alerts → Email Alerts → Wait this long before emailing about the same bird again (seconds) (`email_cooldown_secs`) | Applies after restart |
| `mqtt_host` | Settings → Alerts → Home Assistant & MQTT → Broker address (`mqtt_host`) | Applies after restart |
| `mqtt_port` | Settings → Alerts → Home Assistant & MQTT → Broker port (`mqtt_port`) | Applies after restart |
| `mqtt_username` | Settings → Alerts → Home Assistant & MQTT → Broker user name (`mqtt_username`) | Applies after restart |
| `mqtt_password` | Settings → Alerts → Home Assistant & MQTT → Broker password (`mqtt_password`) | Applies after restart |
| `mqtt_tls` | Settings → Alerts → Home Assistant & MQTT → Secure connection (TLS) (`mqtt_tls`) | Applies after restart |
| `mqtt_ca_file` | Settings → Alerts → Home Assistant & MQTT → Broker trust file (PEM) (`mqtt_ca_file`) | Applies after restart |
| `mqtt_client_cert` | Settings → Alerts → Home Assistant & MQTT → Client certificate (PEM) (`mqtt_client_cert`) | Applies after restart |
| `mqtt_client_key` | Settings → Alerts → Home Assistant & MQTT → Client private key (PEM) (`mqtt_client_key`) | Applies after restart |
| `mqtt_topic_prefix` | Settings → Alerts → Home Assistant & MQTT → Topic prefix (`mqtt_topic_prefix`) | Applies after restart |
| `mqtt_retain` | Settings → Alerts → Home Assistant & MQTT → Keep the last message at the broker (`mqtt_retain`) | Applies after restart |
| `mqtt_ha_discovery` | Settings → Alerts → Home Assistant & MQTT → Tell Home Assistant about this station (`mqtt_ha_discovery`) | Applies after restart |
| `notify_rate_per_minute` | Settings → Alerts → Alert limits → At most this many alerts per minute (`notify_rate_per_minute`) | Applies after restart |
| `station_health_alerts` | Settings → Alerts → Alert limits → Tell me when the station has a problem (`station_health_alerts`) | Applies after restart |
| `duplicate_interval_secs` | Settings → Capture → Detection filters → Ignore repeats of the same bird for (seconds) (`duplicate_interval_secs`) | Applies after restart |
| `noise_threshold` | Settings → Capture → Detection filters → Drop recordings with a barking dog above (0 to 1) (`noise_threshold`) | Applies after restart |
| `noise_classes` | Settings → Capture → Detection filters → Noises to watch for (`noise_classes`) | Applies after restart |
| `noise_remember_secs` | Settings → Capture → Detection filters → Keep dropping recordings for this long after a noise (seconds) (`noise_remember_secs`) | Applies after restart |
| `night_filter` | Settings → Capture → Detection filters → Hold back day birds heard at night (`night_filter`) | Applies after restart |
| `night_margin_mins` | Settings → Capture → Detection filters → Minutes of dusk and dawn that still count as day (`night_margin_mins`) | Applies after restart |
| `night_extra_nocturnal` | Settings → Capture → Detection filters → Extra birds allowed at night (`night_extra_nocturnal`) | Applies after restart |
| `pre_capture_secs` | Settings → Capture → Detection filters → Extra seconds of sound before each call in saved clips (`pre_capture_secs`) | Applies after restart |
| `backup_schedule` | Settings → Data → Backups → Back up the database (`backup_schedule`) | Applies after restart |
| `purge_species_floor` | Settings → General → Storage → Clips of every species that are never purged (`purge_species_floor`) | Applies after restart |
| `raw_audio_keep_every` | Settings → General → Storage → Keep one raw recording in every (`raw_audio_keep_every`) | Applies after restart |
| `ebird_api_key` | Settings → General → Online services → eBird key (`ebird_api_key`) | Applies after restart |
| `ebird_region` | Settings → General → Online services → eBird region (`ebird_region`) | Applies after restart |
| `ebird_dist_km` | Settings → General → Online services → Radius around the station (km) (`ebird_dist_km`) | Applies after restart |
| `ebird_back_days` | Settings → General → Online services → Look back this many days (`ebird_back_days`) | Applies after restart |
| `image_provider` | Settings → General → Online services → Where species photos come from (`image_provider`) | Applies after restart |
| `flickr_api_key` | Settings → General → Online services → Flickr key (`flickr_api_key`) | Applies after restart |
| `flickr_filter_email` | Settings → General → Online services → Only photos by this Flickr account (email) (`flickr_filter_email`) | Applies after restart |
| `birdweather_threshold` | Settings → General → Online services → BirdWeather upload threshold (0 to 1) (`birdweather_threshold`) | Applies after restart |
| `birdweather_location_accuracy` | Settings → General → Online services → Blur the BirdWeather position by up to (metres) (`birdweather_location_accuracy`) | Applies after restart |
| `embed_origins` | Settings → General → Online services → Sites that may show this station in a frame (`embed_origins`) | Applies after restart |

## Configuration-file keys

Every key `birdnet.conf` can carry (`KNOWN_CONFIG_KEYS`). A settings key bridged onto a configuration key shows where it is edited; the rest say why not.

| Key | Changed in the browser at | If not, why |
|---|---|---|
| `ALSA_CARD` | Settings → Capture → Audio Capture → Microphone address (`alsa_device`) | |
| `ALSA_CARDS` | Settings → Capture → Audio sources (several microphones are rows there; this key only seeds them at the first start) | |
| `ANALYTICS_DB_PATH` | — | Where the station keeps its data. Moving it means moving the files first, which a page cannot do; pointing the station at a new path from a form would start it on an empty database and look like lost history. |
| `APPRISE_BODY_TEMPLATE` | Settings → Alerts → Notifications → Notification Body Template (`notify_body_template`) | |
| `APPRISE_CONFIG_FILE` | Settings → Alerts → Notifications → Path to an Apprise configuration file (`apprise_config`) | |
| `APPRISE_COOLDOWN` | Settings → Alerts → Notifications → Notification Cooldown (seconds) (`notify_cooldown`) | |
| `APPRISE_MIN_CONFIDENCE` | Settings → Alerts → Notifications → Notification Min Confidence (`notify_confidence`) | |
| `APPRISE_TITLE_TEMPLATE` | Settings → Alerts → Notifications → Notification Title Template (`notify_title_template`) | |
| `APPRISE_TRIGGER` | Settings → Alerts → Notifications → Notification Trigger (`notify_trigger`) | |
| `APPRISE_URL` | Settings → Alerts → Notifications → Address of your Apprise server (`apprise_url`) | |
| `APPRISE_WATCHLIST` | Settings → Alerts → Notifications → Notify only for these species (common names, separated by commas) (`notify_species_only`) | |
| `APPRISE_WATCHLIST_EXCLUDE` | Settings → Alerts → Notifications → Never notify for these species (common names, separated by commas) (`notify_species_exclude`) | |
| `AUDIOFMT` | Settings → Capture → Audio Capture → Extracted Clip Format (`audio_format`) | |
| `AUDIO_FORMAT` | Settings → Capture → Audio Capture → Extracted Clip Format (`audio_format`) (the same setting under its other spelling) | |
| `BACKUP_SCHEDULE` | Settings → Data → Backups → Back up the database (`backup_schedule`) | |
| `BIRDNET_DYNAMIC_THRESHOLD` | — | Dynamic thresholds are being given their own page and list/reset controls (gap P-11); this setting will live there, not on the settings form. |
| `BIRDNET_DYNAMIC_THRESHOLD_HOURS` | — | Part of the dynamic-threshold feature; see BIRDNET_DYNAMIC_THRESHOLD. |
| `BIRDNET_DYNAMIC_THRESHOLD_MIN` | — | Part of the dynamic-threshold feature; see BIRDNET_DYNAMIC_THRESHOLD. |
| `BIRDNET_DYNAMIC_THRESHOLD_TRIGGER` | — | Part of the dynamic-threshold feature; see BIRDNET_DYNAMIC_THRESHOLD. |
| `BIRDWEATHER_LOCATION_ACCURACY` | Settings → General → Online services → Blur the BirdWeather position by up to (metres) (`birdweather_location_accuracy`) | |
| `BIRDWEATHER_THRESHOLD` | Settings → General → Online services → BirdWeather upload threshold (0 to 1) (`birdweather_threshold`) | |
| `BIRDWEATHER_TOKEN` | Settings → Alerts → Notifications → BirdWeather Station Token (`birdweather_token`) | |
| `BIRDWEATHER_URL` | — | Points uploads at a different BirdWeather server; used only to test against a stand-in. A real station never needs it. |
| `BNB_API_TOKEN` | — | A credential for programs (scripts, Home Assistant) to use the write API. Showing or generating one in a page would put a long-lived secret in browser history and screenshots; set it with a secret file or the environment. |
| `CADDY_PWD` | The setup wizard's first step, and Settings → Access → Accounts | |
| `CLIP_PEAK_CEILING_DBFS` | — | The ceiling the clip-loudness evening-out will not push a clip past (default -1 dBFS). The "Even out clip volume" choice picks a sensible one; the ceiling itself is for audio engineers. |
| `CLIP_RETENTION_DAYS` | Settings → General → System & Display → Keep Clip Audio (days, 0 = forever) (`clip_retention_days`) | |
| `CLIP_TARGET_LUFS` | Settings → Capture → Audio Capture → Even out clip volume (`clip_target_lufs`) | |
| `CONFIDENCE` | Settings → Capture → Detection Settings → Minimum Confidence (0–1) (`confidence_threshold`) | |
| `CONFIRMATION_LEVEL` | Settings → Capture → Detection Settings → Repeat Confirmation (`confirmation_level`) | |
| `CUSTOM_IMAGE_DIR` | Settings → General → System & Display → Custom Species Image Directory (`custom_image_dir`) | |
| `DATABASE_LANG` | Settings → General → System & Display → Common-name language (`database_lang`) | |
| `DB_PATH` | — | Where the station keeps its data. Moving it means moving the files first, which a page cannot do; pointing the station at a new path from a form would start it on an empty database and look like lost history. |
| `DEADMAN_HOURS` | Settings → Alerts → Notifications → Dead-man alert after (hours of no detections) (`deadman_hours`) | |
| `DISK_PURGE_THRESHOLD` | Settings → General → System & Display → Disk Purge Threshold (%) (`purge_threshold`) | |
| `DUPLICATE_INTERVAL_SECS` | Settings → Capture → Detection filters → Ignore repeats of the same bird for (seconds) (`duplicate_interval_secs`) | |
| `EBIRD_API_KEY` | Settings → General → Online services → eBird key (`ebird_api_key`) | |
| `EBIRD_BACK_DAYS` | Settings → General → Online services → Look back this many days (`ebird_back_days`) | |
| `EBIRD_DIST_KM` | Settings → General → Online services → Radius around the station (km) (`ebird_dist_km`) | |
| `EBIRD_REGION` | Settings → General → Online services → eBird region (`ebird_region`) | |
| `EMBED_ORIGINS` | Settings → General → Online services → Sites that may show this station in a frame (`embed_origins`) | |
| `EXTRACTION_LENGTH` | Settings → General → System & Display → Clip Length (seconds) (`extraction_length`) | |
| `FLICKR_API_KEY` | Settings → General → Online services → Flickr key (`flickr_api_key`) | |
| `FLICKR_FILTER_EMAIL` | Settings → General → Online services → Only photos by this Flickr account (email) (`flickr_filter_email`) | |
| `FREQ_SHIFT` | Settings → Capture → Audio Capture → Frequency Shift (Hz, 0 = disabled) (`freq_shift_hz`) | |
| `FULL_DISK` | Settings → General → System & Display → When the disk is full (`full_disk_action`) | |
| `HEARTBEAT_URL` | Settings → Alerts → Notifications → Heartbeat URL (`heartbeat_url`) | |
| `IMAGE_CACHE_DIR` | Settings → General → System & Display → Species Image Cache Directory (`image_cache_dir`) | |
| `IMAGE_PROVIDER` | Settings → General → Online services → Where species photos come from (`image_provider`) | |
| `INFO_SITE` | Settings → General → System & Display → Species Info Links (`info_site`) | |
| `LABELS_DIR` | — | A directory of species-name files that replace the built-in names. Needs files placed on the station's disk, which a browser cannot do; 35 languages are built in and chosen under Common-name language. |
| `LABELS_PATH` | — | Set for you by the installer or by the Bird-identifying model panel. A different labels file than the model's own reports species under other species' names, so it is only ever set together with its model, in the file. |
| `LANG` | Settings → General → System & Display → Common-name language (`database_lang`); the same setting under its other spelling | |
| `LATITUDE` | Settings → Capture → Location & Recording Schedule → Latitude (`latitude`) | |
| `LOG_LEVEL` | — | Read at the very first moment of start-up, before the database is open, so a value saved in a page could not be seen in time. The live log is the System Logs page (Settings → Health → Overview). |
| `LOG_MODULES` | — | Read at the very first moment of start-up, before the database is open (see LOG_LEVEL). |
| `LONGITUDE` | Settings → Capture → Location & Recording Schedule → Longitude (`longitude`) | |
| `MAX_FILES_SPECIES` | Settings → General → System & Display → Max Files Per Species (0 = unlimited) (`max_files_per_species`) | |
| `METADATA_LABELS_PATH` | — | Only needed for the BirdNET+ V3.0 geomodel, whose species list differs from the classifier's; set by the installer. Not used with the default classifier. |
| `METADATA_MODEL_PATH` | — | Installed and switched on by the Bird-identifying model panel together with the classifier it belongs to (the panel shows whether the local species filter is on). A filter for a different classifier mislabels species, so choosing another is done in the file. |
| `MODEL` | — | Names the primary classifier's id for the routing and statistics pages; part of the multi-classifier setup, which is file-only (see MODEL_2_PATH). |
| `MODEL_2_ID` | — | A second or third classifier running beside the first. Each needs its own model and labels file on disk, a sample rate the model's shape cannot state, and enough memory (the station refuses a model that would not fit); a wrong value silently mislabels species. `--install-model list` prints the exact lines. |
| `MODEL_2_LABELS` | — | A second or third classifier running beside the first. Each needs its own model and labels file on disk, a sample rate the model's shape cannot state, and enough memory (the station refuses a model that would not fit); a wrong value silently mislabels species. `--install-model list` prints the exact lines. |
| `MODEL_2_PATH` | — | A second or third classifier running beside the first. Each needs its own model and labels file on disk, a sample rate the model's shape cannot state, and enough memory (the station refuses a model that would not fit); a wrong value silently mislabels species. `--install-model list` prints the exact lines. |
| `MODEL_2_SAMPLE_RATE` | — | A second or third classifier running beside the first. Each needs its own model and labels file on disk, a sample rate the model's shape cannot state, and enough memory (the station refuses a model that would not fit); a wrong value silently mislabels species. `--install-model list` prints the exact lines. |
| `MODEL_2_THRESHOLD` | — | A second or third classifier running beside the first. Each needs its own model and labels file on disk, a sample rate the model's shape cannot state, and enough memory (the station refuses a model that would not fit); a wrong value silently mislabels species. `--install-model list` prints the exact lines. |
| `MODEL_3_ID` | — | A second or third classifier running beside the first. Each needs its own model and labels file on disk, a sample rate the model's shape cannot state, and enough memory (the station refuses a model that would not fit); a wrong value silently mislabels species. `--install-model list` prints the exact lines. |
| `MODEL_3_LABELS` | — | A second or third classifier running beside the first. Each needs its own model and labels file on disk, a sample rate the model's shape cannot state, and enough memory (the station refuses a model that would not fit); a wrong value silently mislabels species. `--install-model list` prints the exact lines. |
| `MODEL_3_PATH` | — | A second or third classifier running beside the first. Each needs its own model and labels file on disk, a sample rate the model's shape cannot state, and enough memory (the station refuses a model that would not fit); a wrong value silently mislabels species. `--install-model list` prints the exact lines. |
| `MODEL_3_SAMPLE_RATE` | — | A second or third classifier running beside the first. Each needs its own model and labels file on disk, a sample rate the model's shape cannot state, and enough memory (the station refuses a model that would not fit); a wrong value silently mislabels species. `--install-model list` prints the exact lines. |
| `MODEL_3_THRESHOLD` | — | A second or third classifier running beside the first. Each needs its own model and labels file on disk, a sample rate the model's shape cannot state, and enough memory (the station refuses a model that would not fit); a wrong value silently mislabels species. `--install-model list` prints the exact lines. |
| `MODEL_DIR` | — | Where the model panel installs and the station looks. The default (beside the database) is what almost everyone wants. |
| `MODEL_ID` | — | Part of the multi-classifier set-up; see MODEL_2_PATH. |
| `MODEL_PATH` | — | Set for you by the installer or by the Bird-identifying model panel (Settings → Capture). Pointing at a model file of your own is the advanced case and is done in the file, together with its labels. |
| `MODEL_ROUTES` | — | Part of the multi-classifier set-up; see MODEL_2_PATH. |
| `MODEL_SAMPLE_RATE` | — | Part of the multi-classifier set-up; see MODEL_2_PATH. |
| `MODEL_THRESHOLD` | — | Part of the multi-classifier set-up; see MODEL_2_PATH. |
| `MQTT_CA_FILE` | Settings → Alerts → Home Assistant & MQTT → Broker trust file (PEM) (`mqtt_ca_file`) | |
| `MQTT_CLIENT_CERT` | Settings → Alerts → Home Assistant & MQTT → Client certificate (PEM) (`mqtt_client_cert`) | |
| `MQTT_CLIENT_KEY` | Settings → Alerts → Home Assistant & MQTT → Client private key (PEM) (`mqtt_client_key`) | |
| `MQTT_HA_DISCOVERY` | Settings → Alerts → Home Assistant & MQTT → Tell Home Assistant about this station (`mqtt_ha_discovery`) | |
| `MQTT_HOST` | Settings → Alerts → Home Assistant & MQTT → Broker address (`mqtt_host`) | |
| `MQTT_PASSWORD` | Settings → Alerts → Home Assistant & MQTT → Broker password (`mqtt_password`) | |
| `MQTT_PORT` | Settings → Alerts → Home Assistant & MQTT → Broker port (`mqtt_port`) | |
| `MQTT_RETAIN` | Settings → Alerts → Home Assistant & MQTT → Keep the last message at the broker (`mqtt_retain`) | |
| `MQTT_TLS` | Settings → Alerts → Home Assistant & MQTT → Secure connection (TLS) (`mqtt_tls`) | |
| `MQTT_TLS_SERVER_NAME` | — | Only needed when the broker's certificate names a different host than the one connected to; a mismatch the browser could only reproduce as a failed connection; set in the file beside the broker trust file (Settings → Alerts → Home Assistant & MQTT). |
| `MQTT_TOPIC_PREFIX` | Settings → Alerts → Home Assistant & MQTT → Topic prefix (`mqtt_topic_prefix`) | |
| `MQTT_USERNAME` | Settings → Alerts → Home Assistant & MQTT → Broker user name (`mqtt_username`) | |
| `NIGHT_EXTRA_NOCTURNAL` | Settings → Capture → Detection filters → Extra birds allowed at night (`night_extra_nocturnal`) | |
| `NIGHT_FILTER` | Settings → Capture → Detection filters → Hold back day birds heard at night (`night_filter`) | |
| `NIGHT_INHIBIT` | Settings → Capture → Location & Recording Schedule → Night Inhibit (suppress recording in darkness) (`night_inhibit`) | |
| `NIGHT_MARGIN_MINS` | Settings → Capture → Detection filters → Minutes of dusk and dawn that still count as day (`night_margin_mins`) | |
| `NOISE_CLASSES` | Settings → Capture → Detection filters → Noises to watch for (`noise_classes`) | |
| `NOISE_REMEMBER_SECS` | Settings → Capture → Detection filters → Keep dropping recordings for this long after a noise (seconds) (`noise_remember_secs`) | |
| `NOISE_THRESHOLD` | Settings → Capture → Detection filters → Drop recordings with a barking dog above (0 to 1) (`noise_threshold`) | |
| `NOTIFY_RATE_PER_MINUTE` | Settings → Alerts → Alert limits → At most this many alerts per minute (`notify_rate_per_minute`) | |
| `NOTIFY_URLS` | Settings → Alerts → Notifications → Where to send alerts (`notify_urls`) | |
| `OFFSITE_BACKUP` | — | Offsite backups are encrypted with a passphrase and need credentials. The settings table is inside every backup, so a passphrase or secret key saved there would sit inside the archive it protects; they are read from the environment or the config file only, on purpose (src/helpers/offsite.rs). |
| `OFFSITE_KEEP` | — | Part of the offsite-backup set-up; see OFFSITE_BACKUP for why it is file-only. |
| `OFFSITE_PASSPHRASE` | — | A secret that must never be stored in the database being backed up; see OFFSITE_BACKUP. |
| `OFFSITE_RSYNC_BWLIMIT` | — | Part of the offsite-backup set-up; see OFFSITE_BACKUP. |
| `OFFSITE_S3_ACCESS_KEY` | — | Credential for the offsite bucket; see OFFSITE_BACKUP. |
| `OFFSITE_S3_ADDRESSING` | — | Part of the offsite-backup set-up; see OFFSITE_BACKUP. |
| `OFFSITE_S3_BUCKET` | — | Part of the offsite-backup set-up; see OFFSITE_BACKUP. |
| `OFFSITE_S3_ENDPOINT` | — | Part of the offsite-backup set-up; see OFFSITE_BACKUP. |
| `OFFSITE_S3_PREFIX` | — | Part of the offsite-backup set-up; see OFFSITE_BACKUP. |
| `OFFSITE_S3_REGION` | — | Part of the offsite-backup set-up; see OFFSITE_BACKUP. |
| `OFFSITE_S3_SECRET_KEY` | — | A secret that must never be stored in the database being backed up; see OFFSITE_BACKUP. |
| `OFFSITE_SFTP_DIR` | — | Part of the offsite-backup set-up; see OFFSITE_BACKUP. |
| `OFFSITE_SFTP_HOST` | — | Part of the offsite-backup set-up; see OFFSITE_BACKUP. |
| `OFFSITE_SFTP_HOST_KEY_POLICY` | — | A security choice about trusting the remote machine; set once in the file, see OFFSITE_BACKUP. |
| `OFFSITE_SFTP_IDENTITY` | — | Path of an SSH private key on the station's disk; see OFFSITE_BACKUP. |
| `OFFSITE_SFTP_KNOWN_HOSTS` | — | Path of a file on the station's disk; see OFFSITE_BACKUP. |
| `OFFSITE_SFTP_PORT` | — | Part of the offsite-backup set-up; see OFFSITE_BACKUP. |
| `OFFSITE_SFTP_USER` | — | Part of the offsite-backup set-up; see OFFSITE_BACKUP. |
| `OVERLAP` | Settings → Capture → Detection Settings → Analysis Overlap (0–2.9 seconds) (`overlap`) | |
| `PIPEWIRE_DEVICE` | Settings → Capture → Audio sources → Add microphone → PipeWire microphone | |
| `POST_SUNSET_OFFSET` | Settings → Capture → Location & Recording Schedule → Extra minutes after sunset (`post_sunset_offset`) | |
| `PRE_CAPTURE_SECS` | Settings → Capture → Detection filters → Extra seconds of sound before each call in saved clips (`pre_capture_secs`) | |
| `PRE_SUNRISE_OFFSET` | Settings → Capture → Location & Recording Schedule → Extra minutes before sunrise (`pre_sunrise_offset`) | |
| `PRIVACY_RANK` | Settings → Capture → Detection Settings → Privacy Rank ({rank_min} = disabled, up to {rank_max}) (`privacy_rank`) | |
| `PRIVACY_THRESHOLD` | Settings → Capture → Detection Settings → Privacy Threshold (0 = disabled) (`privacy_threshold`) | |
| `PRIVATE_MODE` | — | Putting the whole station behind the sign-in changes who can reach the page. Switched on with no admin password set, every visitor, including the owner, is answered 503 and nobody can sign in; switched on from a page the owner is looking at through a proxy that is not signed in, the page is gone mid-click. It is chosen in the file with the service stopped (hardening guide, section 5). |
| `PUBLIC_ACCESS` | — | The list of surfaces that stay open on a private station; it only means anything with PRIVATE_MODE, and has the same lock-out risk. |
| `PURGE_SPECIES_FLOOR` | Settings → General → Storage → Clips of every species that are never purged (`purge_species_floor`) | |
| `RAW_AUDIO_KEEP_EVERY` | Settings → General → Storage → Keep one raw recording in every (`raw_audio_keep_every`) | |
| `RECORDING_LENGTH` | Settings → Capture → Audio Capture → Segment Duration (`segment_duration`); read as `SEGMENT_DURATION` when that is not set | |
| `RECORDING_SCHEDULE` | Settings → Capture → Location & Recording Schedule → Recording window (`recording_schedule`) | |
| `RECS_DIR` | — | Where clips are kept (and, when no watch directory is given, where capture segments land). Moving it means moving the files first; a path typed in a form would leave the existing clips behind. |
| `RTSP_URL` | Settings → Capture → Audio Capture → Network camera address (`rtsp_url`) | |
| `RTSP_URLS` | Settings → Capture → Audio Capture → Several network cameras (separated by commas) (`rtsp_urls`) | |
| `SEGMENT_DURATION` | Settings → Capture → Audio Capture → Segment Duration (seconds) (`segment_duration`) | |
| `SENSITIVITY` | Settings → Capture → Detection Settings → Sensitivity (0.5–1.5) (`sensitivity`) | |
| `SF_THRESH` | Settings → Capture → Detection Settings → Species Frequency Threshold (0–1) (`sf_thresh`) | |
| `SITENAME` | Settings → General → System & Display → Site Name (`site_name`) | |
| `SPECIES_ALIASES_PATH` | — | A tab-separated file of legacy names kept by hand over months; it needs a file on the station's disk. |
| `STATION_HEALTH_ALERTS` | Settings → Alerts → Alert limits → Tell me when the station has a problem (`station_health_alerts`) | |
| `STATION_NAME` | Settings → Capture → Location & Recording Schedule → Station Name (`station_name`) | |
| `STREAM_MAX_MB` | Settings → General → System & Display → Streaming Folder Limit (MB) (`stream_max_mb`) | |
| `STREAM_RETENTION_SECS` | Settings → General → System & Display → Raw Segment Retention (seconds) (`stream_retention_secs`) | |
| `TLS_CERT` | — | Changing how the page is served (HTTPS, certificates, the address it listens on) from the page itself can lock the owner out: a wrong certificate or address makes the page unreachable, with no page left to undo it. Set in the file with the service stopped. |
| `TLS_DIR` | — | Same reason as TLS_CERT: part of the HTTPS set-up, which can lock the owner out if changed from the page. |
| `TLS_KEY` | — | Same reason as TLS_CERT: a wrong key makes the station unreachable and there is no page left to fix it from. |
| `WATCHDOG_BACKOFF_BASE_SECS` | — | Timings of the capture watchdog (how fast a dead microphone is restarted). The defaults suit every station; changing them is a support exercise, and the log says when one is adjusted. |
| `WATCHDOG_BACKOFF_CAP_SECS` | — | Capture watchdog timing; see WATCHDOG_BACKOFF_BASE_SECS. |
| `WATCHDOG_CHECK_SECS` | — | Capture watchdog timing; see WATCHDOG_BACKOFF_BASE_SECS. |
| `WATCHDOG_DOWN_WARN_AFTER_SECS` | — | Capture watchdog timing; see WATCHDOG_BACKOFF_BASE_SECS. |
| `WATCHDOG_DOWN_WARN_EVERY_SECS` | — | Capture watchdog timing; see WATCHDOG_BACKOFF_BASE_SECS. |
| `WATCHDOG_STALL_FLOOR_SECS` | — | Capture watchdog timing; see WATCHDOG_BACKOFF_BASE_SECS. |
| `WATCHDOG_STALL_SEGMENTS` | — | Capture watchdog timing; see WATCHDOG_BACKOFF_BASE_SECS. |
| `WEATHER_API_KEY` | — | Part of the weather overlay, which is opt-in by environment variable only; see WEATHER_PROVIDER. |
| `WEATHER_PROVIDER` | — | The weather overlay is switched on by the environment variable BNB_WEATHER_ENABLED=1, deliberately not a setting, so a station never contacts a weather service unless the person who runs it set that up where the program starts. |
| `WEATHER_STATION_ID` | — | Part of the weather overlay, which is opt-in by environment variable only; see WEATHER_PROVIDER. |
| `WEEKLY_REPORT_SCHEDULE` | Settings → Alerts → Notifications → Weekly Report Day (`weekly_report_schedule`) | |
## Command-line flags

Every flag `birdnet-behavior --help` lists, the environment variable that sets it, and where the same thing is changed in the browser. Flags and environment variables set a value at launch and win over the Settings pages; a setting that is made that way is not shown on its page.

| Flag | Environment variable | Changed in the browser at | If not, why |
|---|---|---|---|
| `--alsa-device` | `BIRDNET_ALSA_DEVICE` | Settings → Capture → Audio Capture → Microphone address (`alsa_device`) (the same as `ALSA_CARD`) | |
| `--alsa-devices` | `BIRDNET_ALSA_DEVICES` | Settings → Capture → Audio sources (several microphones are rows there; this key only seeds them at the first start) (the same as `ALSA_CARDS`) | |
| `--analytics-db` | `BIRDNET_ANALYTICS_DB` | — | Same as `ANALYTICS_DB_PATH`: Where the station keeps its data. Moving it means moving the files first, which a page cannot do; pointing the station at a new path from a form would start it on an empty database and look like lost history. |
| `--apply-config` | — | — | Installs a replacement configuration file and restarts the service; a file-editing workflow. The Settings pages are its browser equivalent. |
| `--apply-import` | — | Settings → Data → Bring your history with you → Apply |  |
| `--apprise-config` | `BIRDNET_APPRISE_CONFIG` | Settings → Alerts → Notifications → Path to an Apprise configuration file (`apprise_config`) (the same as `APPRISE_CONFIG_FILE`) | |
| `--apprise-url` | `BIRDNET_APPRISE_URL` | Settings → Alerts → Notifications → Address of your Apprise server (`apprise_url`) (the same as `APPRISE_URL`) | |
| `--audio-format` | `BIRDNET_AUDIO_FORMAT` | Settings → Capture → Audio Capture → Extracted Clip Format (`audio_format`) (the same as `AUDIOFMT`) | |
| `--backup-db` | — | Settings → Data → Backups → Back up now |  |
| `--birdweather-token` | `BIRDNET_BIRDWEATHER_TOKEN` | Settings → Alerts → Notifications → BirdWeather Station Token (`birdweather_token`) (the same as `BIRDWEATHER_TOKEN`) | |
| `--channel-report` | — | — | Records from the microphone exclusively for a few seconds to report what a stereo microphone delivers; the running station owns the microphone, so this is run with it stopped. |
| `--channel-report-secs` | — | — | Only meaningful with --channel-report; see that flag. |
| `--check-db` | — | — | A run-and-exit maintenance command. The station runs the same check on a schedule, and the station-health alerts say when it has not completed in weeks. |
| `--clip-retention-days` | `BIRDNET_CLIP_RETENTION_DAYS` | Settings → General → System & Display → Keep Clip Audio (days, 0 = forever) (`clip_retention_days`) (the same as `CLIP_RETENTION_DAYS`) | |
| `--clips-mode` | — | Settings → Data → Bring your history with you → Recordings → Copy, link, move or leave |  |
| `--config` | `BIRDNET_CONFIG` | — | Names the configuration file the program reads at the very start, before there is a database or a page to show settings from. |
| `--confirmation-level` | `BIRDNET_CONFIRMATION_LEVEL` | Settings → Capture → Detection Settings → Repeat Confirmation (`confirmation_level`) (the same as `CONFIRMATION_LEVEL`) | |
| `--custom-image-dir` | `BIRDNET_CUSTOM_IMAGE_DIR` | Settings → General → System & Display → Custom Species Image Directory (`custom_image_dir`) (the same as `CUSTOM_IMAGE_DIR`) | |
| `--deadman-hours` | `BIRDNET_DEADMAN_HOURS` | Settings → Alerts → Notifications → Dead-man alert after (hours of no detections) (`deadman_hours`) (the same as `DEADMAN_HOURS`) | |
| `--decrypt-backup` | — | — | An offline recovery tool for when the station itself may be gone; it has to run without a working station. |
| `--disk-exclude` | `BIRDNET_DISK_EXCLUDE` | — | Paths on the station's disk to leave out of disk monitoring; a list of directories only someone with a shell knows. |
| `--disk-purge-threshold` | `BIRDNET_DISK_PURGE_THRESHOLD` | Settings → General → System & Display → Disk Purge Threshold (%) (`purge_threshold`) (the same as `DISK_PURGE_THRESHOLD`) | |
| `--doctor` | — | Settings → Health → Diagnostics |  |
| `--doctor-gate` | — | — | The service's start gate, run by systemd before the program starts, so it cannot be a page. |
| `--doctor-json` | — | Settings → Health → Diagnostics → Download as JSON |  |
| `--duplicate-interval-secs` | `BIRDNET_DUPLICATE_INTERVAL_SECS` | Settings → Capture → Detection filters → Ignore repeats of the same bird for (seconds) (`duplicate_interval_secs`) (the same as `DUPLICATE_INTERVAL_SECS`) | |
| `--extraction-length` | `BIRDNET_EXTRACTION_LENGTH` | Settings → General → System & Display → Clip Length (seconds) (`extraction_length`) (the same as `EXTRACTION_LENGTH`) | |
| `--fix` | — | — | Safe repairs (missing directories) run with the diagnostic at the command line; the station creates its directories at start and Diagnostics reports any it cannot. |
| `--freq-shift-hz` | `BIRDNET_FREQ_SHIFT_HZ` | Settings → Capture → Audio Capture → Frequency Shift (Hz, 0 = disabled) (`freq_shift_hz`) (the same as `FREQ_SHIFT`) | |
| `--heartbeat-url` | `BIRDNET_HEARTBEAT_URL` | Settings → Alerts → Notifications → Heartbeat URL (`heartbeat_url`) (the same as `HEARTBEAT_URL`) | |
| `--image-cache-dir` | `BIRDNET_IMAGE_CACHE_DIR` | Settings → General → System & Display → Species Image Cache Directory (`image_cache_dir`) (the same as `IMAGE_CACHE_DIR`) | |
| `--import-birdnet-pi-clips` | — | Settings → Data → Bring your history with you → Recordings |  |
| `--import-birdnet-pi-conf` | — | Settings → Data → Bring your history with you (upload `birdnet.conf`; the page shows every line before anything changes) |  |
| `--info-site` | `BIRDNET_INFO_SITE` | Settings → General → System & Display → Species Info Links (`info_site`) (the same as `INFO_SITE`) | |
| `--install-model` | — | Settings → Capture → Bird-identifying model → Install |  |
| `--labels` | `BIRDNET_LABELS` | — | Same as `LABELS_PATH`: Set for you by the installer or by the Bird-identifying model panel. A different labels file than the model's own reports species under other species' names, so it is only ever set together with its model, in the file. |
| `--labels-dir` | `BIRDNET_LABELS_DIR` | — | Same as `LABELS_DIR`: A directory of species-name files that replace the built-in names. Needs files placed on the station's disk, which a browser cannot do; 35 languages are built in and chosen under Common-name language. |
| `--lang` | `BIRDNET_LANG` | Settings → General → System & Display → Common-name language (`database_lang`) (the same as `DATABASE_LANG`) | |
| `--latitude` | `BIRDNET_LATITUDE` | Settings → Capture → Location & Recording Schedule → Latitude (`latitude`) (the same as `LATITUDE`) | |
| `--listen` | `BIRDNET_LISTEN` | — | Changes how the page itself is served (the address it listens on, HTTPS, the certificate). A wrong value leaves no page to correct it from, so it is chosen where the program is started. |
| `--log-level` | `BIRDNET_LOG_LEVEL` | — | Same as `LOG_LEVEL`: Read at the very first moment of start-up, before the database is open, so a value saved in a page could not be seen in time. The live log is the System Logs page (Settings → Health → Overview). |
| `--log-modules` | `BIRDNET_LOG_MODULES` | — | Same as `LOG_MODULES`: Read at the very first moment of start-up, before the database is open (see LOG_LEVEL). |
| `--longitude` | `BIRDNET_LONGITUDE` | Settings → Capture → Location & Recording Schedule → Longitude (`longitude`) (the same as `LONGITUDE`) | |
| `--max-files-per-species` | `BIRDNET_MAX_FILES_PER_SPECIES` | Settings → General → System & Display → Max Files Per Species (0 = unlimited) (`max_files_per_species`) (the same as `MAX_FILES_SPECIES`) | |
| `--metadata-labels` | `BIRDNET_METADATA_LABELS` | — | Same as `METADATA_LABELS_PATH`: Only needed for the BirdNET+ V3.0 geomodel, whose species list differs from the classifier's; set by the installer. Not used with the default classifier. |
| `--metadata-model` | `BIRDNET_METADATA_MODEL` | — | Same as `METADATA_MODEL_PATH`: Installed and switched on by the Bird-identifying model panel together with the classifier it belongs to (the panel shows whether the local species filter is on). A filter for a different classifier mislabels species, so choosing another is done in the file. |
| `--migration-report` | — | — | A dry run of the database upgrade that happens at the next start, for deciding whether to upgrade; it runs before the new version is running. |
| `--model` | `BIRDNET_MODEL` | — | Same as `MODEL_PATH`: Set for you by the installer or by the Bird-identifying model panel (Settings → Capture). Pointing at a model file of your own is the advanced case and is done in the file, together with its labels. |
| `--mqtt-ca-file` | `BIRDNET_MQTT_CA_FILE` | Settings → Alerts → Home Assistant & MQTT → Broker trust file (PEM) (`mqtt_ca_file`) (the same as `MQTT_CA_FILE`) | |
| `--mqtt-client-cert` | `BIRDNET_MQTT_CLIENT_CERT` | Settings → Alerts → Home Assistant & MQTT → Client certificate (PEM) (`mqtt_client_cert`) (the same as `MQTT_CLIENT_CERT`) | |
| `--mqtt-client-id` | `BIRDNET_MQTT_CLIENT_ID` | — | The identifier the station presents to the broker. Almost nobody changes it; two stations on one broker are told apart by topic prefix, which is a setting. |
| `--mqtt-client-key` | `BIRDNET_MQTT_CLIENT_KEY` | Settings → Alerts → Home Assistant & MQTT → Client private key (PEM) (`mqtt_client_key`) (the same as `MQTT_CLIENT_KEY`) | |
| `--mqtt-ha-discovery` | `BIRDNET_MQTT_HA_DISCOVERY` | Settings → Alerts → Home Assistant & MQTT → Tell Home Assistant about this station (`mqtt_ha_discovery`) (the same as `MQTT_HA_DISCOVERY`) | |
| `--mqtt-host` | `BIRDNET_MQTT_HOST` | Settings → Alerts → Home Assistant & MQTT → Broker address (`mqtt_host`) (the same as `MQTT_HOST`) | |
| `--mqtt-password` | `BIRDNET_MQTT_PASSWORD` | Settings → Alerts → Home Assistant & MQTT → Broker password (`mqtt_password`) (the same as `MQTT_PASSWORD`) | |
| `--mqtt-port` | `BIRDNET_MQTT_PORT` | Settings → Alerts → Home Assistant & MQTT → Broker port (`mqtt_port`) (the same as `MQTT_PORT`) | |
| `--mqtt-retain` | `BIRDNET_MQTT_RETAIN` | Settings → Alerts → Home Assistant & MQTT → Keep the last message at the broker (`mqtt_retain`) (the same as `MQTT_RETAIN`) | |
| `--mqtt-tls` | `BIRDNET_MQTT_TLS` | Settings → Alerts → Home Assistant & MQTT → Secure connection (TLS) (`mqtt_tls`) (the same as `MQTT_TLS`) | |
| `--mqtt-tls-server-name` | `BIRDNET_MQTT_TLS_SERVER_NAME` | — | Same as `MQTT_TLS_SERVER_NAME`: Only needed when the broker's certificate names a different host than the one connected to; a mismatch the browser could only reproduce as a failed connection; set in the file beside the broker trust file (Settings → Alerts → Home Assistant & MQTT). |
| `--mqtt-topic-prefix` | `BIRDNET_MQTT_TOPIC_PREFIX` | Settings → Alerts → Home Assistant & MQTT → Topic prefix (`mqtt_topic_prefix`) (the same as `MQTT_TOPIC_PREFIX`) | |
| `--mqtt-username` | `BIRDNET_MQTT_USERNAME` | Settings → Alerts → Home Assistant & MQTT → Broker user name (`mqtt_username`) (the same as `MQTT_USERNAME`) | |
| `--night-extra-nocturnal` | `BIRDNET_NIGHT_EXTRA_NOCTURNAL` | Settings → Capture → Detection filters → Extra birds allowed at night (`night_extra_nocturnal`) (the same as `NIGHT_EXTRA_NOCTURNAL`) | |
| `--night-filter` | `BIRDNET_NIGHT_FILTER` | Settings → Capture → Detection filters → Hold back day birds heard at night (`night_filter`) (the same as `NIGHT_FILTER`) | |
| `--night-inhibit` | `BIRDNET_NIGHT_INHIBIT` | Settings → Capture → Location & Recording Schedule → Night Inhibit (suppress recording in darkness) (`night_inhibit`) (the same as `NIGHT_INHIBIT`) | |
| `--night-margin-mins` | `BIRDNET_NIGHT_MARGIN_MINS` | Settings → Capture → Detection filters → Minutes of dusk and dawn that still count as day (`night_margin_mins`) (the same as `NIGHT_MARGIN_MINS`) | |
| `--no-update-check` | `BIRDNET_NO_UPDATE_CHECK` | — | A launch-time privacy choice grouped with --offline; updating itself is a button on the System page. |
| `--noise-classes` | `BIRDNET_NOISE_CLASSES` | Settings → Capture → Detection filters → Noises to watch for (`noise_classes`) (the same as `NOISE_CLASSES`) | |
| `--noise-threshold` | `BIRDNET_NOISE_THRESHOLD` | Settings → Capture → Detection filters → Drop recordings with a barking dog above (0 to 1) (`noise_threshold`) (the same as `NOISE_THRESHOLD`) | |
| `--notify-body-template` | `BIRDNET_NOTIFY_BODY_TEMPLATE` | Settings → Alerts → Notifications → Notification Body Template (`notify_body_template`) (the same as `APPRISE_BODY_TEMPLATE`) | |
| `--notify-confidence` | `BIRDNET_NOTIFY_CONFIDENCE` | Settings → Alerts → Notifications → Notification Min Confidence (`notify_confidence`) (the same as `APPRISE_MIN_CONFIDENCE`) | |
| `--notify-rate-per-minute` | `BIRDNET_NOTIFY_RATE_PER_MINUTE` | Settings → Alerts → Alert limits → At most this many alerts per minute (`notify_rate_per_minute`) (the same as `NOTIFY_RATE_PER_MINUTE`) | |
| `--notify-species-exclude` | `BIRDNET_NOTIFY_SPECIES_EXCLUDE` | Settings → Alerts → Notifications → Never notify for these species (common names, separated by commas) (`notify_species_exclude`) (the same as `APPRISE_WATCHLIST_EXCLUDE`) | |
| `--notify-species-only` | `BIRDNET_NOTIFY_SPECIES_ONLY` | Settings → Alerts → Notifications → Notify only for these species (common names, separated by commas) (`notify_species_only`) (the same as `APPRISE_WATCHLIST`) | |
| `--notify-title-template` | `BIRDNET_NOTIFY_TITLE_TEMPLATE` | Settings → Alerts → Notifications → Notification Title Template (`notify_title_template`) (the same as `APPRISE_TITLE_TEMPLATE`) | |
| `--notify-trigger` | `BIRDNET_NOTIFY_TRIGGER` | Settings → Alerts → Notifications → Notification Trigger (`notify_trigger`) (the same as `APPRISE_TRIGGER`) | |
| `--notify-urls` | `BIRDNET_NOTIFY_URLS` | Settings → Alerts → Notifications → Where to send alerts (`notify_urls`) (the same as `NOTIFY_URLS`) | |
| `--offline` | `BIRDNET_OFFLINE` | — | A launch-time egress policy. The setup page's model download and location lookup are exactly what it forbids, so it is chosen where the program starts, not in the page it would disable. |
| `--offsite-backup` | `BIRDNET_OFFSITE_BACKUP` | — | Same as `OFFSITE_BACKUP`: Offsite backups are encrypted with a passphrase and need credentials. The settings table is inside every backup, so a passphrase or secret key saved there would sit inside the archive it protects; they are read from the environment or the config file only, on purpose (src/helpers/offsite.rs). |
| `--out` | — | — | Only meaningful with --decrypt-backup; see that flag. |
| `--overlap` | `BIRDNET_OVERLAP` | Settings → Capture → Detection Settings → Analysis Overlap (0–2.9 seconds) (`overlap`) (the same as `OVERLAP`) | |
| `--pipewire-device` | `BIRDNET_PIPEWIRE_DEVICE` | Settings → Capture → Audio sources → Add microphone → PipeWire microphone | |
| `--post-sunset-offset` | `BIRDNET_POST_SUNSET_OFFSET` | Settings → Capture → Location & Recording Schedule → Extra minutes after sunset (`post_sunset_offset`) (the same as `POST_SUNSET_OFFSET`) | |
| `--pre-sunrise-offset` | `BIRDNET_PRE_SUNRISE_OFFSET` | Settings → Capture → Location & Recording Schedule → Extra minutes before sunrise (`pre_sunrise_offset`) (the same as `PRE_SUNRISE_OFFSET`) | |
| `--privacy-rank` | `BIRDNET_PRIVACY_RANK` | Settings → Capture → Detection Settings → Privacy Rank ({rank_min} = disabled, up to {rank_max}) (`privacy_rank`) (the same as `PRIVACY_RANK`) | |
| `--privacy-threshold` | `BIRDNET_PRIVACY_THRESHOLD` | Settings → Capture → Detection Settings → Privacy Threshold (0 = disabled) (`privacy_threshold`) (the same as `PRIVACY_THRESHOLD`) | |
| `--private-mode` | `BIRDNET_PRIVATE_MODE` | — | Same as `PRIVATE_MODE`: Putting the whole station behind the sign-in changes who can reach the page. Switched on with no admin password set, every visitor, including the owner, is answered 503 and nobody can sign in; switched on from a page the owner is looking at through a proxy that is not signed in, the page is gone mid-click. It is chosen in the file with the service stopped (hardening guide, section 5). |
| `--process-existing` | — | — | Analyses audio already in the watch directory when the program starts, for batch use; a station that records live never needs it. |
| `--public-access` | `BIRDNET_PUBLIC_ACCESS` | — | Same as `PUBLIC_ACCESS`: The list of surfaces that stay open on a private station; it only means anything with PRIVATE_MODE, and has the same lock-out risk. |
| `--purge-species-floor` | `BIRDNET_PURGE_SPECIES_FLOOR` | Settings → General → Storage → Clips of every species that are never purged (`purge_species_floor`) (the same as `PURGE_SPECIES_FLOOR`) | |
| `--raw-audio-keep-every` | `BIRDNET_RAW_AUDIO_KEEP_EVERY` | Settings → General → Storage → Keep one raw recording in every (`raw_audio_keep_every`) (the same as `RAW_AUDIO_KEEP_EVERY`) | |
| `--rebuild-species-summary` | — | — | A repair run once, with the service stopped, after a data fix; the species totals are kept up to date on every write. |
| `--recording-schedule` | `BIRDNET_RECORDING_SCHEDULE` | Settings → Capture → Location & Recording Schedule → Recording window (`recording_schedule`) (the same as `RECORDING_SCHEDULE`) | |
| `--refresh-extension` | — | — | Reinstalls the analytics extension, a repair run with the service stopped. |
| `--rtsp-url` | `BIRDNET_RTSP_URL` | Settings → Capture → Audio Capture → Network camera address (`rtsp_url`) (the same as `RTSP_URL`) | |
| `--rtsp-urls` | `BIRDNET_RTSP_URLS` | Settings → Capture → Audio Capture → Several network cameras (separated by commas) (`rtsp_urls`) (the same as `RTSP_URLS`) | |
| `--segment-duration` | `BIRDNET_SEGMENT_DURATION` | Settings → Capture → Audio Capture → Segment Duration (seconds) (`segment_duration`) (the same as `SEGMENT_DURATION`) | |
| `--sf-thresh` | `BIRDNET_SF_THRESH` | Settings → Capture → Detection Settings → Species Frequency Threshold (0–1) (`sf_thresh`) (the same as `SF_THRESH`) | |
| `--site-name` | `BIRDNET_SITENAME` | Settings → General → System & Display → Site Name (`site_name`) (the same as `SITENAME`) | |
| `--species-aliases` | `BIRDNET_SPECIES_ALIASES` | — | Same as `SPECIES_ALIASES_PATH`: A tab-separated file of legacy names kept by hand over months; it needs a file on the station's disk. |
| `--station-health-alerts` | `BIRDNET_STATION_HEALTH_ALERTS` | Settings → Alerts → Alert limits → Tell me when the station has a problem (`station_health_alerts`) (the same as `STATION_HEALTH_ALERTS`) | |
| `--stream-max-mb` | `BIRDNET_STREAM_MAX_MB` | Settings → General → System & Display → Streaming Folder Limit (MB) (`stream_max_mb`) (the same as `STREAM_MAX_MB`) | |
| `--stream-retention-secs` | `BIRDNET_STREAM_RETENTION_SECS` | Settings → General → System & Display → Raw Segment Retention (seconds) (`stream_retention_secs`) (the same as `STREAM_RETENTION_SECS`) | |
| `--support-bundle` | — | Settings → Health → Diagnostics → Download a support bundle |  |
| `--tls-cert` | `BIRDNET_TLS_CERT` | — | Changes how the page itself is served (the address it listens on, HTTPS, the certificate). A wrong value leaves no page to correct it from, so it is chosen where the program is started. |
| `--tls-days` | `BIRDNET_TLS_DAYS` | — | Changes how the page itself is served (the address it listens on, HTTPS, the certificate). A wrong value leaves no page to correct it from, so it is chosen where the program is started. |
| `--tls-dir` | `BIRDNET_TLS_DIR` | — | Changes how the page itself is served (the address it listens on, HTTPS, the certificate). A wrong value leaves no page to correct it from, so it is chosen where the program is started. |
| `--tls-hostnames` | `BIRDNET_TLS_HOSTNAMES` | — | Changes how the page itself is served (the address it listens on, HTTPS, the certificate). A wrong value leaves no page to correct it from, so it is chosen where the program is started. |
| `--tls-key` | `BIRDNET_TLS_KEY` | — | Changes how the page itself is served (the address it listens on, HTTPS, the certificate). A wrong value leaves no page to correct it from, so it is chosen where the program is started. |
| `--tls-listen` | `BIRDNET_TLS_LISTEN` | — | Changes how the page itself is served (the address it listens on, HTTPS, the certificate). A wrong value leaves no page to correct it from, so it is chosen where the program is started. |
| `--tls-mode` | `BIRDNET_TLS_MODE` | — | Changes how the page itself is served (the address it listens on, HTTPS, the certificate). A wrong value leaves no page to correct it from, so it is chosen where the program is started. |
| `--tls-redirect` | `BIRDNET_TLS_REDIRECT` | — | Changes how the page itself is served (the address it listens on, HTTPS, the certificate). A wrong value leaves no page to correct it from, so it is chosen where the program is started. |
| `--twilight-offset` | `BIRDNET_TWILIGHT_OFFSET` | Settings → Capture → Location & Recording Schedule → Extra minutes before sunrise / after sunset (`pre_sunrise_offset`, `post_sunset_offset`) |  |
| `--verify-extension` | — | — | Checks that the analytics extension loads, a repair step run with the service stopped. |
| `--watch-dir` | `BIRDNET_WATCH_DIR` | — | Same as `RECS_DIR`: Where clips are kept (and, when no watch directory is given, where capture segments land). Moving it means moving the files first; a path typed in a form would leave the existing clips behind. |
| `--web-only` | — | — | How the program is launched: it skips the detection engine, so there is no station left for a page to configure. The Capture tab's model panel says when a station is running this way. |
| `--weekly-report-schedule` | `BIRDNET_WEEKLY_REPORT_SCHEDULE` | Settings → Alerts → Notifications → Weekly Report Day (`weekly_report_schedule`) (the same as `WEEKLY_REPORT_SCHEDULE`) | |
## Pages with their own forms

Not every change is a setting. These pages change things that are not one value each: lists, accounts, rules, data. Each is under **Settings** in the browser.

| Page | What it changes |
|---|---|
| Settings → Capture → Audio sources | Microphones and camera streams: add, edit gain and filters, quiet hours, restart one, choose which one *Listen live* plays |
| Settings → Capture → Which birds count | The exclude, include and always-allow lists, and a confidence threshold for one species |
| Settings → Alerts → Alert rules | Rules that send a message or call a web address when a bird, a time of day or a confidence matches; rules on the station's own measurements |
| Settings → Alerts → Channels | Send a test through each channel |
| Settings → Data → Backups | Back up now, download, restore, delete a backup |
| Settings → Data → Bring your history with you | Validate and import a BirdNET-Pi database |
| Settings → General → Today Page Layout | Which blocks the Today page shows, and in what order |
| Settings → General → Display | Theme, density and motion, saved on this device |
| Settings → Access → Accounts | The admin password, other people's accounts, signed-in sessions |
| Settings → Health → System | Restart the station; check for and apply an update |
| The setup wizard (`/onboarding`) | First-run: the admin password, the bird-identifying model, location, name and language, microphone, accuracy and alert presets |

## Things that are not settings

* **The station's clock and time zone** belong to the operating system. The setup wizard records the zone it finds so Diagnostics can tell you when the system's disagrees, and says the one command that fixes it; the program cannot change them (it does not run as root).
* **Units and the clock face** are settings: the temperature unit (Celsius or Fahrenheit) and the 12- or 24-hour clock are under Settings → General → System & Display (`temperature_unit`, `time_format`), and each browser can also choose its own in Display preferences.
* **Forgetting the admin password** is recovered from the device itself: the reset link is printed on the station's own console, so that someone who can only reach the page cannot take the station.
