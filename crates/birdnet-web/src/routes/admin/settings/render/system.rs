//! System, display, and authentication settings section.

use std::collections::HashMap;

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let days = get_setting(s, "recording_days", "30");
    let imgcache = get_setting(s, "image_cache_dir", "");
    let customimg = get_setting(s, "custom_image_dir", "");
    let maxfiles = get_setting(s, "max_files_per_species", "0");
    let purge = get_setting(s, "purge_threshold", "95");
    let site = get_setting(s, "site_name", "");
    let isite = get_setting(s, "info_site", "ebird");
    let is_ebird = if isite == "ebird" { " selected" } else { "" };
    let is_aab = if isite == "allaboutbirds" {
        " selected"
    } else {
        ""
    };
    let is_none = if isite == "none" { " selected" } else { "" };
    let auth_user = get_setting(s, "auth_username", "");
    let auth_pass = get_setting(s, "auth_password", "");
    out.push_str(&format!(
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
        <label for="recording_days">Keep Recordings (days)</label>
        <input id="recording_days" name="recording_days" type="number"
               value="{days}" min="1" max="365">
        <p class="hint">Audio files older than this are deleted automatically (BirdNET-Pi: PROCESSED_DAYS)</p>
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
             value="{purge}" min="50" max="99" style="max-width:120px">
      <p class="hint">Start purging old recordings when disk usage exceeds this % (BirdNET-Pi: DISK_PURGE_THRESHOLD)</p>
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
    <p class="hint" style="margin-bottom:1rem;">Leave blank to disable HTTP Basic Auth (allow open access).</p>
    <div class="grid-2">
      <div>
        <label for="auth_username">Username</label>
        <input id="auth_username" name="auth_username" value="{auth_user}" placeholder="birdnet"
               autocomplete="username">
        <p class="hint">Web UI login username (BirdNET-Pi: CADDY_USER)</p>
      </div>
      <div>
        <label for="auth_password">Password</label>
        <input id="auth_password" name="auth_password" type="password" value="{auth_pass}"
               placeholder="leave blank to keep current" autocomplete="new-password">
        <p class="hint">Web UI login password (BirdNET-Pi: CADDY_PWD)</p>
      </div>
    </div>
  </div>"#
    ));
}
