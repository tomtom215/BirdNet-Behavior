//! System, display, and authentication settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    // Default 0 = keep audio forever, which is what every station does
    // today; age-based retention is strictly opt-in.
    let days = get_setting(s, "clip_retention_days", "0");
    let imgcache = get_setting(s, "image_cache_dir", "");
    let customimg = get_setting(s, "custom_image_dir", "");
    let maxfiles = get_setting(s, "max_files_per_species", "0");
    let purge = get_setting(s, "purge_threshold", "95");
    // Defaults mirror `helpers::system::DEFAULT_STREAM_*` so the form shows what
    // the station is actually doing when nothing has been set.
    let streamret = get_setting(s, "stream_retention_secs", "600");
    let streammax = get_setting(s, "stream_max_mb", "512");
    let site = get_setting(s, "site_name", "");
    let isite = get_setting(s, "info_site", "ebird");
    // Command-line only until 0.12.0, so a non-English station could not pick
    // its own language from the UI at all.
    let lang = get_setting(s, "database_lang", "en");
    let is_ebird = if isite == "ebird" { " selected" } else { "" };
    let is_aab = if isite == "allaboutbirds" {
        " selected"
    } else {
        ""
    };
    let is_none = if isite == "none" { " selected" } else { "" };
    write!(out,
        r#"
  <div class="card">
    <div class="section-title">System &amp; Display</div>
    <div class="grid-2">
      <div>
        <label for="site_name">Site Name</label>
        <input id="site_name" name="site_name" value="{site}" placeholder="My Bird Station">
        <p class="hint">Shown in page titles and headers (BirdNET-Pi: SITENAME)</p>
      </div>
      <div>
        <label for="info_site">Species Info Links</label>
        <select id="info_site" name="info_site">
          <option value="ebird"{is_ebird}>eBird</option>
          <option value="allaboutbirds"{is_aab}>AllAboutBirds</option>
          <option value="none"{is_none}>None</option>
        </select>
        <p class="hint">Species detail page links to external info (BirdNET-Pi: INFO_SITE)</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="database_lang">Common-name language</label>
        <input id="database_lang" name="database_lang" type="text" value="{lang}"
               placeholder="en" maxlength="8" class="bnb-w-num">
        <p class="hint">Two-letter code for the language species common names are shown in, e.g. <code>en</code>, <code>de</code>, <code>fr</code>. Scientific names are unaffected (BirdNET-Pi: DATABASE_LANG)</p>
      </div>
      <div></div>
    </div>
    <div class="grid-2">
      <div>
        <label for="clip_retention_days">Keep Clip Audio (days, 0 = forever)</label>
        <input id="clip_retention_days" name="clip_retention_days" type="number"
               value="{days}" min="0" max="3650">
        <p class="hint">Reclaim the audio of detections older than this. <b>0 keeps everything</b>, which is the default. The detections themselves are always kept — your counts, species lists, trends and exports are unaffected; only the sound file goes. Locked clips are never reclaimed.</p>
      </div>
      <div>
        <label for="max_files_per_species">Max Files Per Species (0 = unlimited)</label>
        <input id="max_files_per_species" name="max_files_per_species" type="number"
               value="{maxfiles}" min="0" step="10">
        <p class="hint">Oldest files beyond this limit are auto-deleted (BirdNET-Pi: MAX_FILES_SPECIES)</p>
      </div>
    </div>
    <div>
      <label for="purge_threshold">Disk Purge Threshold (%)</label>
      <input id="purge_threshold" name="purge_threshold" type="number"
             value="{purge}" min="50" max="99" class="bnb-w-num">
      <p class="hint">Start purging old recordings when disk usage exceeds this % (BirdNET-Pi: DISK_PURGE_THRESHOLD). Locked clips are never purged.</p>
    </div>
    <div class="grid-2">
      <div>
        <label for="stream_retention_secs">Raw Segment Retention (seconds)</label>
        <input id="stream_retention_secs" name="stream_retention_secs" type="number"
               value="{streamret}" min="0" step="60">
        <p class="hint">How long raw capture segments stay in the temporary streaming folder before being cleared. They are only needed until the detector has read them; the default (600) is far longer than that. 0 disables the timed clean-up.</p>
      </div>
      <div>
        <label for="stream_max_mb">Streaming Folder Limit (MB)</label>
        <input id="stream_max_mb" name="stream_max_mb" type="number"
               value="{streammax}" min="0" step="64">
        <p class="hint">Hard ceiling on that same temporary folder — oldest segments go first if it is exceeded. It usually lives in RAM, so this stops a busy or backed-up station filling it. 0 removes the ceiling.</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="image_cache_dir">Species Image Cache Directory</label>
        <input id="image_cache_dir" name="image_cache_dir" value="{imgcache}"
               placeholder="/var/lib/birdnet/images">
        <p class="hint">Leave blank to disable Wikipedia image caching</p>
      </div>
      <div>
        <label for="custom_image_dir">Custom Species Image Directory</label>
        <input id="custom_image_dir" name="custom_image_dir" value="{customimg}"
               placeholder="/home/pi/BirdNet-Behavior/custom_images">
        <p class="hint">Override Wikipedia images with custom photos (BirdNET-Pi: CUSTOM_IMAGE). Files: sci_name.jpg</p>
      </div>
    </div>
  </div>
  <div class="card">
    <div class="section-title">Web Authentication</div>
    <p class="hint">
      The admin password is not stored in this table &mdash; it lives as an Argon2id
      hash in the accounts database, seeded from the <code>CADDY_PWD</code> environment
      variable. Set or rotate it there and restart; the station picks the new value up
      on the next start. Until <code>CADDY_PWD</code> is set, <code>/admin</code> is
      open to anyone who can reach this station.
    </p>
    <p class="hint">
      See <a href="/help/admin/remote-access">Remote Access &amp; Security</a> for the
      full setup, including putting the station behind a reverse proxy.
    </p>
  </div>"#
    ).unwrap_or_default();
}
