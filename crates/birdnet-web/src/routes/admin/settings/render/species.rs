//! Species filter settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let excl = get_setting(s, "species_exclude", "");
    let incl = get_setting(s, "species_include", "");
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
  </section>"#
    )
    .unwrap_or_default();
}
