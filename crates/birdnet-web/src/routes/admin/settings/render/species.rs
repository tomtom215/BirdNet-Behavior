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
      <label for="species_exclude">Excluded Species (comma-separated common names)</label>
      <textarea id="species_exclude" name="species_exclude" rows="3"
                placeholder="e.g. House Sparrow, Feral Pigeon">{excl}</textarea>
      <p class="hint">These species will never be saved or notified</p>
    </div>
    <div>
      <label for="species_include">Allow-list (empty = all species)</label>
      <textarea id="species_include" name="species_include" rows="3"
                placeholder="e.g. European Robin, Eurasian Blackbird">{incl}</textarea>
      <p class="hint">When set, only these species are saved or notified</p>
    </div>
  </section>"#
    )
    .unwrap_or_default();
}
