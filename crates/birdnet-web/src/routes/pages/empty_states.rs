//! Hand-rolled SVG empty states.
//!
//! Drop-in: import this module from any page partial. Each function returns
//! an HTML string that matches `_empty_states.html` template entries.
//!
//! ```rust,ignore
//! use crate::routes::pages::empty_states;
//!
//! if rows.is_empty() {
//!     return ok_html(empty_states::quiet_yard());
//! }
//! ```

const SHELL_OPEN: &str = r#"<div class="empty-state">"#;
const SHELL_CLOSE: &str = "</div>";

/// Empty state shown when no detections have been recorded yet today.
pub fn quiet_yard() -> String {
    format!(
        r#"{SHELL_OPEN}<svg width="120" height="80" viewBox="0 0 120 80" aria-hidden="true"><defs><linearGradient id="es-qy" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="var(--moss)" stop-opacity="0.35"/><stop offset="100%" stop-color="var(--moss)" stop-opacity="0.05"/></linearGradient></defs><line x1="6" y1="60" x2="114" y2="60" stroke="var(--hairline)" stroke-width="0.7"/><g fill="url(#es-qy)"><rect x="14" y="50" width="3" height="10" rx="1"/><rect x="34" y="42" width="3" height="18" rx="1"/><rect x="54" y="34" width="3" height="26" rx="1"/><rect x="74" y="42" width="3" height="18" rx="1"/><rect x="94" y="50" width="3" height="10" rx="1"/></g><circle cx="60" cy="22" r="3" fill="var(--moss)" opacity="0.4"/><circle cx="60" cy="22" r="9" fill="none" stroke="var(--moss)" stroke-opacity="0.15"/></svg><p class="display es-h">A quiet yard.</p><p class="bnb-meta es-sub">The station is listening — nothing has flown by yet.</p>{SHELL_CLOSE}"#,
    )
}

/// Empty state shown when the species life-list or gallery is empty.
pub fn no_species() -> String {
    format!(
        r#"{SHELL_OPEN}<svg width="120" height="80" viewBox="0 0 120 80" aria-hidden="true"><g><circle cx="22" cy="26" r="8" fill="var(--surface-2)" stroke="var(--hairline)"/><rect x="38" y="22" width="56" height="3" rx="1.5" fill="var(--hairline)"/><rect x="38" y="30" width="36" height="3" rx="1.5" fill="var(--surface-2)" stroke="var(--hairline)" stroke-width="0.4"/></g><g opacity="0.7"><circle cx="22" cy="46" r="8" fill="var(--surface-2)" stroke="var(--hairline)"/><rect x="38" y="42" width="44" height="3" rx="1.5" fill="var(--hairline)"/></g><g opacity="0.4"><circle cx="22" cy="66" r="8" fill="var(--surface-2)" stroke="var(--hairline)"/><rect x="38" y="62" width="58" height="3" rx="1.5" fill="var(--hairline)"/></g></svg><p class="display es-h">No species heard yet.</p><p class="bnb-meta es-sub">First detections usually take a few minutes after the mic comes online. <a href="/system" class="es-link">Check mic status →</a></p>{SHELL_CLOSE}"#,
    )
}

/// Empty state shown on the Dawn Chorus page when there is insufficient
/// historical data to draw ribbons (requires at least one full day).
pub fn no_chorus() -> String {
    format!(
        r#"{SHELL_OPEN}<svg width="120" height="120" viewBox="0 0 120 120" aria-hidden="true"><g stroke="var(--hairline)" stroke-width="0.6" fill="none"><circle cx="60" cy="60" r="44"/><circle cx="60" cy="60" r="24"/></g><g stroke="var(--fg-3)" stroke-width="0.8" stroke-linecap="round"><line x1="60" y1="12" x2="60" y2="18"/><line x1="60" y1="102" x2="60" y2="108"/><line x1="12" y1="60" x2="18" y2="60"/><line x1="102" y1="60" x2="108" y2="60"/></g><circle cx="92" cy="80" r="3" fill="var(--dawn)" opacity="0.55"/></svg><p class="display es-h">The chorus hasn't started.</p><p class="bnb-meta es-sub">We need at least a full day of recordings to draw ribbons. Check back tomorrow.</p>{SHELL_CLOSE}"#,
    )
}

/// Empty state shown when co-occurrence analysis finds no species that
/// were detected together within the same 5-minute window.
pub fn no_co_signal() -> String {
    format!(
        r#"{SHELL_OPEN}<svg width="120" height="80" viewBox="0 0 120 80" aria-hidden="true"><circle cx="30" cy="40" r="14" fill="var(--moss)" fill-opacity="0.35" stroke="var(--moss-ink)" stroke-width="0.8"/><circle cx="90" cy="40" r="14" fill="var(--dawn)" fill-opacity="0.35" stroke="var(--dawn-ink)" stroke-width="0.8"/><line x1="44" y1="40" x2="76" y2="40" stroke="var(--fg-4)" stroke-width="1" stroke-dasharray="3 4"/></svg><p class="display es-h">Not enough overlap yet.</p><p class="bnb-meta es-sub">Co-occurrence needs two species heard within the same 5-minute window. The dawn chorus is the easiest time to catch them together.</p>{SHELL_CLOSE}"#,
    )
}

/// Empty state shown on the quarantine page when there are no rare-bird
/// detections awaiting review.
pub fn no_rare_yet() -> String {
    format!(
        r#"{SHELL_OPEN}<svg width="120" height="80" viewBox="0 0 120 80" aria-hidden="true"><circle cx="60" cy="40" r="26" fill="var(--moss-soft)" stroke="var(--moss)" stroke-width="0.8"/><path d="M48,40 L56,48 L74,30" fill="none" stroke="var(--moss-ink)" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg><p class="display es-h">Nothing waiting for review.</p><p class="bnb-meta es-sub">All rare-bird flags are confirmed or dismissed. Detections above 0.95 confidence skip this queue automatically.</p>{SHELL_CLOSE}"#,
    )
}

/// Empty state shown on the Life List page before any species have been
/// detected for the first time.
pub fn no_life_list() -> String {
    format!(
        r#"{SHELL_OPEN}<svg width="120" height="100" viewBox="0 0 120 100" aria-hidden="true"><rect x="14" y="14" width="44" height="72" rx="2" fill="var(--surface)" stroke="var(--hairline)"/><rect x="62" y="14" width="44" height="72" rx="2" fill="var(--surface)" stroke="var(--hairline)"/><g stroke="var(--hairline)" stroke-width="0.5"><line x1="18" y1="24" x2="54" y2="24"/><line x1="18" y1="32" x2="54" y2="32"/><line x1="18" y1="40" x2="54" y2="40"/><line x1="18" y1="48" x2="54" y2="48"/><line x1="66" y1="24" x2="102" y2="24"/><line x1="66" y1="32" x2="102" y2="32"/></g><circle cx="24" cy="24" r="2.5" fill="var(--moss)"/></svg><p class="display es-h">Your life list starts here.</p><p class="bnb-meta es-sub">Every species your station hears for the first time will be logged on this page — with the date, the recording, and your notes.</p>{SHELL_CLOSE}"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::pages::error_states;

    /// Every empty and failure state, by name.
    fn every_state() -> Vec<(&'static str, String)> {
        vec![
            ("quiet_yard", quiet_yard()),
            ("no_species", no_species()),
            ("no_chorus", no_chorus()),
            ("no_co_signal", no_co_signal()),
            ("no_rare_yet", no_rare_yet()),
            ("no_life_list", no_life_list()),
            (
                "could_not_load",
                error_states::could_not_load("your species"),
            ),
        ]
    }

    /// Whether `html` opens an `<h1>`…`<h6>`.
    fn opens_a_heading(html: &str) -> bool {
        html.match_indices("<h")
            .any(|(i, _)| html[i + 2..].starts_with(|c: char| ('1'..='6').contains(&c)))
    }

    /// These fragments are dropped into pages of different depth: under an
    /// `<h1>` with no `<h2>` (Rare bird quarantine, whose list is empty on a
    /// station with nothing to review), and under a card's `<h2>`. A hard-coded
    /// `<h3>` is right for the second and skips a level in the first, which
    /// axe reports as `heading-order`, and which the CI fixture never showed
    /// because its quarantine list is not empty. A title that is not a heading
    /// is correct in both.
    #[test]
    fn an_empty_state_does_not_declare_a_heading_level() {
        for (name, html) in every_state() {
            assert!(!opens_a_heading(&html), "{name} opens a heading: {html}");
        }
    }

    /// The counterpart: dropping the heading must not have dropped the title.
    #[test]
    fn every_state_still_says_what_it_is() {
        for (name, html) in every_state() {
            let title = html
                .split(r#"class="display es-h">"#)
                .nth(1)
                .and_then(|rest| rest.split('<').next())
                .unwrap_or_default();
            assert!(title.len() > 8, "{name} has no title: {html}");
        }
    }

    #[test]
    fn the_heading_check_tells_a_heading_from_other_h_tags() {
        assert!(opens_a_heading(r#"<h3 class="x">T</h3>"#));
        assert!(opens_a_heading("<div><h1>T</h1></div>"));
        assert!(!opens_a_heading(r#"<p class="x">T</p>"#));
        assert!(!opens_a_heading("<hr><header>T</header><html>"));
    }
}
