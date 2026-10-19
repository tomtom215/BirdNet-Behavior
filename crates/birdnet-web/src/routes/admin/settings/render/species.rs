//! Species filter settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let excl = get_setting(s, "species_exclude");
    let incl = get_setting(s, "species_include");
    let white = get_setting(s, "species_whitelist");
    write!(
        out,
        r#"
  <section class="card" id="set-species" aria-labelledby="set-species-h">
    <h2 class="section-title" id="set-species-h">Species Filters</h2>
    <p class="hint">
      Or manage species lists interactively on the
      <a href="/admin/species">Species Lists</a> page.
    </p>
    <div>
      <label for="species_exclude">Never record these (common names, separated by commas)</label>
      <textarea id="species_exclude" name="species_exclude" rows="3"
                placeholder="e.g. House Sparrow, Feral Pigeon">{excl}</textarea>
      <p class="hint"><b>These birds are not recorded at all</b> — no clip, no
      entry in your records, no alert. If you only want to stop the alerts and
      keep the sightings, use "Never notify for these species" under
      Notifications instead.</p>
    </div>
    <div>
      <label for="species_include">Record only these (leave empty for all birds)</label>
      <textarea id="species_include" name="species_include" rows="3"
                placeholder="e.g. European Robin, Eurasian Blackbird">{incl}</textarea>
      <p class="hint">While this list has anything in it, every other bird is
      ignored. Leave it empty unless you are deliberately watching for a few
      species.</p>
    </div>
    <div>
      <label for="species_whitelist">Always allow these, even where they are unlikely (common names, separated by commas)</label>
      <textarea id="species_whitelist" name="species_whitelist" rows="3"
                placeholder="e.g. Snowy Owl">{white}</textarea>
      <p class="hint">Birds the species-occurrence filter would hold back
      because they are rare for your location and week are let through anyway
      (BirdNET-Pi's whitelist). It changes nothing else: a bird on the
      &ldquo;never record&rdquo; list stays out, and so does one that is not on a
      &ldquo;record only&rdquo; list. It only matters once the station has a
      location. Try lowering the occurrence threshold first; this is for the
      one or two birds you know visit anyway.</p>
    </div>
  </section>"#
    )
    .unwrap_or_default();
}
