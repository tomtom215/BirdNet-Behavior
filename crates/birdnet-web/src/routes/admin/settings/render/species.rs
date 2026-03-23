//! Species filter settings section.

use std::collections::HashMap;

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let excl = get_setting(s, "species_exclude", "");
    let incl = get_setting(s, "species_include", "");
    out.push_str(&format!(
        r#"
  <div class="card">
    <div class="section-title">Species Filters</div>
    <p class="hint" style="margin-bottom:1rem;">
      Or manage species lists interactively on the
      <a href="/admin/species" style="color:#38bdf8;">Species Lists</a> page.
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
  </div>"#
    ));
}
