//! The "when does this apply" badge beside each setting, and the banner above
//! the form for changes that are saved and waiting for a restart.
//!
//! Done as a pass over the finished markup, not by editing each field, so a new
//! field gets its badge by being in [`super::super::applies::APPLIES`] and a
//! forgotten one is a failing test (`every_field_on_the_page_carries_a_badge`)
//! and not a page that quietly says nothing.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::super::applies::{Applies, applies_for, every};

/// The pseudo-setting the page loader adds: the keys whose saved value is newer
/// than the running station and not yet in force, comma-joined. Never stored.
pub const PENDING_KEY: &str = "__pending_restart";

/// Insert a badge before the closing tag of each field's label.
///
/// A field is found by its `<label for="key"`; a key whose label is not in the
/// markup (the section is not on this page) is skipped, and one that has a
/// label gets exactly one badge.
pub(super) fn add_applies_badges(html: &str) -> String {
    let mut out = html.to_owned();
    for entry in every() {
        let marker = format!(r#"<label for="{}""#, entry.key);
        let mut from = 0;
        while let Some(rel) = out[from..].find(&marker) {
            let start = from + rel;
            let Some(close_rel) = out[start..].find("</label>") else {
                break;
            };
            let close = start + close_rel;
            // The label's own text only: a `<label for>` never encloses another.
            let badge = badge_html(entry.key, entry.applies);
            out.insert_str(close, &badge);
            from = close + badge.len() + "</label>".len();
        }
    }
    out
}

fn badge_html(key: &str, applies: Applies) -> String {
    format!(
        r#" <span class="applies applies--{slug}" data-applies-for="{key}">{text}</span>"#,
        slug = applies.slug(),
        text = applies.badge(),
    )
}

/// The label text of the field `key`, for naming it in the banner.
fn label_of(html: &str, key: &str) -> Option<String> {
    let marker = format!(r#"<label for="{key}""#);
    let start = html.find(&marker)?;
    let open_end = html[start..].find('>')? + start + 1;
    let close = html[open_end..].find("</label>")? + open_end;
    let text = html[open_end..close]
        .split("<span class=\"applies")
        .next()
        .unwrap_or("");
    // Strip any tag a label carries; entities stay as they are (the page is HTML).
    let mut plain = String::new();
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => plain.push(c),
            _ => {}
        }
    }
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    (!plain.is_empty()).then_some(plain)
}

/// The banner for changes waiting on a restart, or nothing.
///
/// `settings` carries [`PENDING_KEY`] when [`crate::pending_restart`] found
/// saved changes newer than the running station.
pub(super) fn pending_banner(settings: &HashMap<String, String>, form_html: &str) -> String {
    let Some(raw) = settings.get(PENDING_KEY).filter(|v| !v.is_empty()) else {
        return String::new();
    };
    let mut names = Vec::new();
    for key in raw.split(',').filter(|k| !k.is_empty()) {
        // Only keys that really wait for a restart are named.
        if applies_for(key).is_some_and(Applies::needs_restart) {
            names.push(label_of(form_html, key).unwrap_or_else(|| key.replace('_', " ")));
        }
    }
    if names.is_empty() {
        return String::new();
    }
    let list = names.join(", ");
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<div class="applies-banner" role="status"><div class="applies-banner__text"><strong>Saved, waiting for a restart.</strong> These changes are stored but the running station has not picked them up yet: {list}.</div><button type="button" class="bnb-btn primary" data-restart-now>Restart the station now</button><span class="bnb-meta" data-restart-status role="status"></span></div>"#
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate that keeps the badges honest as the page grows: every control
    /// the form can save has a badge saying when it applies, exactly one.
    ///
    /// A field with no `<label for>` of its own cannot carry one; those are
    /// listed with the reason, and the list is checked in the other direction
    /// too so it cannot name a field that has since got a label.
    #[test]
    fn every_field_on_the_page_carries_a_badge() {
        use crate::routes::admin::settings::form::all_form_keys;
        let html = super::super::render_settings_form(
            &HashMap::new(),
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        let mut unbadged = Vec::new();
        for key in all_form_keys() {
            let n = html
                .matches(&format!(r#"data-applies-for="{key}""#))
                .count();
            assert!(n <= 1, "{key} has {n} badges");
            if n == 0 {
                unbadged.push(key);
            }
        }
        assert!(
            unbadged.is_empty(),
            "these fields show no \"when does this apply\" badge: {unbadged:?}"
        );
    }

    #[test]
    fn a_labelled_field_gets_one_badge_with_its_key() {
        let html = r#"<label for="latitude">Latitude</label><input id="latitude" name="latitude">"#;
        let out = add_applies_badges(html);
        assert_eq!(
            out.matches("data-applies-for=\"latitude\"").count(),
            1,
            "{out}"
        );
        assert!(out.contains("Applies after restart"), "{out}");
        assert!(out.contains(r#"Latitude <span class="applies"#), "{out}");
    }

    #[test]
    fn a_live_field_says_so() {
        let html = r#"<label for="rare_species_days">Days</label>"#;
        let out = add_applies_badges(html);
        assert!(out.contains("Applies immediately"), "{out}");
        assert!(!out.contains("after restart"), "{out}");
    }

    #[test]
    fn a_field_the_table_does_not_know_gets_nothing() {
        let html = r#"<label for="no_such_field">X</label>"#;
        assert_eq!(add_applies_badges(html), html);
    }

    /// The banner names only what truly waits, by the words on the page.
    #[test]
    fn the_banner_names_the_changes_that_wait_and_not_the_ones_that_do_not() {
        let html =
            r#"<label for="latitude">Latitude</label><label for="rare_species_days">Days</label>"#;
        let s = HashMap::from([(
            PENDING_KEY.to_owned(),
            "latitude,rare_species_days".to_owned(),
        )]);
        let banner = pending_banner(&s, html);
        assert!(banner.contains("Latitude"), "{banner}");
        assert!(
            !banner.contains("Days"),
            "a setting that applies at once is not waiting: {banner}"
        );
        assert!(banner.contains("data-restart-now"), "{banner}");
        assert_eq!(pending_banner(&HashMap::new(), html), "");
        let only_live = HashMap::from([(PENDING_KEY.to_owned(), "rare_species_days".to_owned())]);
        assert_eq!(pending_banner(&only_live, html), "");
    }
}
