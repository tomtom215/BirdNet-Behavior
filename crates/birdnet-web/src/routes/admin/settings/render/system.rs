//! System, display, and authentication settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

use birdnet_core::i18n::{ENGLISH, LANGUAGES, NameLocalizer, find_language};

use super::get_setting;

/// The common-name language control: one option per language the station has
/// names for, each with the share of the installed classifier's species it
/// covers, so a Korean station finds out *before* choosing it that most birds
/// will keep their English names.
///
/// A stored value that is not a language (a typo from the old free-text box)
/// selects English, which is what the station shows for it, and the hint says
/// so. Saving then replaces it with a real choice rather than refusing the
/// whole form over a value the page no longer lets anyone type.
fn language_picker(stored: &str, names: &NameLocalizer) -> String {
    let stored = stored.trim();
    let chosen = find_language(stored).map_or(ENGLISH, |l| l.code);
    let mut options = String::new();
    for language in LANGUAGES {
        let label = if language.code == ENGLISH {
            "English — the classifier's own names".to_owned()
        } else {
            let share = names.coverage_of(language.code).map_or(String::new(), |c| {
                format!(" — {}%", (c.named * 100 + c.of / 2) / c.of)
            });
            if language.native_name == language.english_name {
                format!("{}{share}", language.english_name)
            } else {
                format!(
                    "{} ({}){share}",
                    language.english_name, language.native_name
                )
            }
        };
        let selected = if language.code == chosen {
            " selected"
        } else {
            ""
        };
        let _ = write!(
            options,
            r#"<option value="{}"{selected}>{}</option>"#,
            language.code,
            crate::routes::pages::escape_html(&label)
        );
    }
    let unrecognised = if stored.is_empty() || find_language(stored).is_some() {
        String::new()
    } else {
        format!(
            r" <b>The stored value <code>{}</code> is not a language with names available, so names are shown as the classifier gives them; choose one to replace it.</b>",
            crate::routes::pages::escape_html(stored)
        )
    };
    let share_note = names.coverage_of("de").map_or(String::new(), |c| {
        format!(
            " The percentages are the share of the {} species your classifier can report that have a name in that language; the rest keep the classifier's name.",
            crate::routes::pages::group_thousands(i64::try_from(c.of).unwrap_or(i64::MAX))
        )
    });
    format!(
        r#"<label for="database_lang">Common-name language</label>
        <select id="database_lang" name="database_lang">{options}</select>
        <p class="hint">The language species common names are shown in. Only what is <em>shown</em> changes: detections, lists, filters and exports keep the classifier's names, so you can switch back at any time and nothing is lost or split.{share_note}{unrecognised} Applies when the station next starts (BirdNET-Pi: DATABASE_LANG)</p>"#
    )
}

/// The `<option>`s of the spectrogram colour-map picker, the stored one chosen.
///
/// A stored value that is no colour map selects the default, which is what the
/// renderer draws for it.
fn spectrogram_colormap_options(stored: &str) -> String {
    let choices = crate::routes::spectrogram::colormap_choices();
    let chosen = choices
        .iter()
        .find(|(value, _)| *value == stored.trim())
        .map_or("viridis", |(value, _)| *value);
    choices
        .iter()
        .map(|(value, label)| {
            let selected = if *value == chosen { " selected" } else { "" };
            format!(r#"<option value="{value}"{selected}>{label}</option>"#)
        })
        .collect()
}

/// The `<option>`s of the spectrogram dynamic-range picker (dB).
fn spectrogram_range_options(stored: &str) -> String {
    use crate::routes::spectrogram::{DEFAULT_RANGE_DB, RANGES_DB};
    let chosen = stored
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|n| RANGES_DB.contains(n))
        .unwrap_or(DEFAULT_RANGE_DB);
    RANGES_DB
        .iter()
        .map(|db| {
            let selected = if *db == chosen { " selected" } else { "" };
            let note = if *db == DEFAULT_RANGE_DB {
                " (default)"
            } else {
                ""
            };
            format!(r#"<option value="{db}"{selected}>{db} dB{note}</option>"#)
        })
        .collect()
}

pub(super) fn render(out: &mut String, s: &HashMap<String, String>, names: &NameLocalizer) {
    // Default 0 = keep audio forever, which is what every station does
    // today; age-based retention is strictly opt-in.
    let days = get_setting(s, "clip_retention_days");
    let imgcache = get_setting(s, "image_cache_dir");
    let customimg = get_setting(s, "custom_image_dir");
    let dash_image = get_setting(s, "dashboard_image");
    let dash_title = get_setting(s, "dashboard_image_title");
    let maxfiles = get_setting(s, "max_files_per_species");
    let purge = get_setting(s, "purge_threshold");
    let raw_spec = get_setting(s, "raw_spectrogram");
    let raw_spec_checked = if raw_spec == "true" { " checked" } else { "" };
    let colormap_options = spectrogram_colormap_options(&get_setting(s, "spectrogram_colormap"));
    let range_options = spectrogram_range_options(&get_setting(s, "spectrogram_range_db"));
    let rare_days = get_setting(s, "rare_species_days");
    let extract_len = get_setting(s, "extraction_length");
    // Defaults mirror `helpers::system::DEFAULT_STREAM_*` so the form shows what
    // the station is actually doing when nothing has been set.
    let streamret = get_setting(s, "stream_retention_secs");
    let streammax = get_setting(s, "stream_max_mb");
    let site = get_setting(s, "site_name");
    let isite = get_setting(s, "info_site");
    // Command-line only until 0.12.0, so a non-English station could not pick
    // its own language from the UI at all.
    let lang_picker = language_picker(s.get("database_lang").map_or("", String::as_str), names);
    let is_ebird = if isite == "ebird" { " selected" } else { "" };
    let is_aab = if isite == "allaboutbirds" {
        " selected"
    } else {
        ""
    };
    let is_none = if isite == "none" { " selected" } else { "" };
    write!(out,
        r#"
  <section class="card" id="set-system" aria-labelledby="set-system-h">
    <h2 class="section-title" id="set-system-h">System &amp; Display</h2>
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
        {lang_picker}
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
        <label for="rare_species_days">Rare after (days)</label>
        <input id="rare_species_days" name="rare_species_days" type="number"
               value="{rare_days}" min="0" max="3650" class="bnb-w-num">
        <p class="hint">A species not heard for this many days is flagged rare when it
          returns, and appears in the rare feeds (BirdNET-Pi: RARE_SPECIES_THRESHOLD).
          0 keeps the old behaviour: only the very first sighting of a species counts.</p>
      </div>
      <div>
        <label for="extraction_length">Clip Length (seconds)</label>
        <input id="extraction_length" name="extraction_length" type="number"
               value="{extract_len}" min="1" max="60" step="1" class="bnb-w-num">
        <p class="hint">Audio saved around each detection (BirdNET-Pi:
          EXTRACTION_LENGTH). Longer clips are easier to verify by ear and cost
          proportionally more disk.</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="raw_spectrogram">Bare spectrograms</label>
        <label class="bnb-check"><input id="raw_spectrogram" name="raw_spectrogram"
               type="checkbox" value="true"{raw_spec_checked}> No text overlay</label>
        <p class="hint">Leaves out the species and confidence caption, for embedding in
          a stream overlay or a print (BirdNET-Pi: RAW_SPECTROGRAM). This renderer draws
          no axes in either case.</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="spectrogram_colormap">Spectrogram colours</label>
        <select id="spectrogram_colormap" name="spectrogram_colormap">{colormap_options}</select>
        <p class="hint">Louder is always lighter, so the order survives a grey print.
          Viridis is what earlier versions drew. The inverted grey puts a white floor
          under black calls, the look of a printed figure. Applies to the next
          image shown.</p>
      </div>
      <div>
        <label for="spectrogram_range_db">Spectrogram dynamic range</label>
        <select id="spectrogram_range_db" name="spectrogram_range_db">{range_options}</select>
        <p class="hint">Cells more than this far below the loudest are drawn as the
          floor. A smaller range gives more contrast and drops faint calls into the
          background; a larger one keeps them. 80 dB is what earlier versions used
          (birdnet-go offers 80, 100 and 120).</p>
      </div>
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
        <p class="hint">Replace one species' picture with your own photograph. Files are named
          after the species: <code>sci_name.jpg</code> (lower case, underscores). This is not
          BirdNET-Pi's <code>CUSTOM_IMAGE</code>: for a picture of the site, use the next two fields.</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="dashboard_image">Today page picture (file)</label>
        <input id="dashboard_image" name="dashboard_image" value="{dash_image}"
               placeholder="/home/pi/camera/latest.jpg" autocomplete="off" spellcheck="false">
        <p class="hint">Full path of a PNG, JPEG, GIF or WebP file, shown on the Today page
          (BirdNET-Pi: CUSTOM_IMAGE). Something else keeps it current, for example a
          camera snapshot saved every minute; the page checks it every ten seconds and
          shows a new frame as soon as the file changes. Up to 8 MB; anything that is
          not a picture is refused. Leave blank for no picture.</p>
      </div>
      <div>
        <label for="dashboard_image_title">Picture title</label>
        <input id="dashboard_image_title" name="dashboard_image_title" value="{dash_title}"
               placeholder="The feeder" maxlength="120">
        <p class="hint">The heading over the picture (BirdNET-Pi: CUSTOM_IMAGE_TITLE).
          Also its description for screen readers. The block can be moved or hidden under
          Today page layout.</p>
      </div>
    </div>
  </section>
  <section class="card" id="set-auth" aria-labelledby="set-auth-h">
    <h2 class="section-title" id="set-auth-h">Web Authentication</h2>
    <p class="hint">
      The admin password is not stored in this table &mdash; it lives as an Argon2id
      hash in the accounts database, seeded from <code>CADDY_PWD</code>. Editing
      <code>CADDY_PWD</code> to a new value and restarting replaces it; a password changed
      on the Accounts page is kept across restarts until then. Until a password is set
      (<code>CADDY_PWD</code>, the setup wizard or the Accounts page), <code>/admin</code> is
      open to anyone who can reach this station.
    </p>
    <p class="hint">
      See <a href="/help/admin/remote-access">Remote Access &amp; Security</a> for the
      full setup, including putting the station behind a reverse proxy.
    </p>
  </section>"#
    ).unwrap_or_default();
}
