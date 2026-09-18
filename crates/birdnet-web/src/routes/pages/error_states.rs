//! Hand-rolled "this could not be loaded" states — the counterpart to
//! [`super::empty_states`].
//!
//! The crate had six designed empty states and, before this module, exactly
//! two error renderers. Everywhere else a failed query was `unwrap_or_default`
//! -ed into an empty `Vec`, and the empty state rendered on top of it. The
//! result was a family of screens that answered a broken database with a
//! confident statement about the operator's birds:
//!
//! * History: *"No detection history yet. Once your station logs its first
//!   day, it'll appear here."* — to someone with three years of data.
//! * Recordings: *"No saved clips yet"* — which reads as *the purge ate them*.
//! * Species: *"No species match this filter yet."* — when the filter was fine.
//!
//! Emptiness and failure are different facts and the operator acts on them
//! differently: one means wait, the other means look at the station. Nothing
//! here retries on its own — a partial that failed is not made truer by being
//! fetched again without the operator asking.
//!
//! ```rust,ignore
//! use crate::routes::pages::error_states;
//!
//! let Ok(rows) = query(conn) else {
//!     return ok_html(error_states::could_not_load("your detection history"));
//! };
//! ```

use super::escape_html;

/// Full-surface failure card: an illustration, what failed, and a way back.
///
/// `what` completes the sentence "We couldn't load {what}." — so pass a noun
/// phrase in the operator's vocabulary ("your detection history", "this
/// month's calendar"), not a table name.
#[must_use]
pub fn could_not_load(what: &str) -> String {
    format!(
        r#"<div class="empty-state error-state" role="alert"><svg width="120" height="80" viewBox="0 0 120 80" aria-hidden="true"><line x1="6" y1="60" x2="114" y2="60" stroke="var(--hairline)" stroke-width="0.7"/><g fill="var(--rare)" fill-opacity="0.22"><rect x="14" y="50" width="3" height="10" rx="1"/><rect x="34" y="46" width="3" height="14" rx="1"/></g><g stroke="var(--rare)" stroke-width="1.1" stroke-linecap="round" stroke-dasharray="3 5" fill="none"><line x1="46" y1="54" x2="104" y2="54"/></g><circle cx="60" cy="26" r="13" fill="var(--rare-soft)" stroke="var(--rare)" stroke-width="0.8"/><g stroke="var(--rare)" stroke-width="2" stroke-linecap="round"><line x1="60" y1="20" x2="60" y2="28"/><line x1="60" y1="32" x2="60" y2="32"/></g></svg><h3 class="display es-h">We couldn't load {what}.</h3><p class="bnb-meta es-sub">The station is running, but this reading failed — so this is not a statement about your birds. Check <a href="/station" class="es-link">Station health</a>, then reload.</p></div>"#,
        what = escape_html(what),
    )
}

/// One-line failure note for a card or a chart slot, where a full-height
/// illustration would push the rest of the page around.
#[must_use]
pub fn inline(what: &str) -> String {
    format!(
        r#"<p class="bnb-load-error" role="alert">We couldn't load {what}. This is a fault, not an empty result — <a href="/station">check the station</a>.</p>"#,
        what = escape_html(what),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the module: neither renderer may read as emptiness.
    ///
    /// A future edit that reworded these into "Nothing here yet" would undo
    /// the distinction this module exists to draw, and every call site would
    /// go on compiling.
    #[test]
    fn error_copy_never_claims_the_yard_is_empty() {
        for html in [
            could_not_load("your detection history"),
            inline("this month"),
        ] {
            let lower = html.to_lowercase();
            for forbidden in [
                "no detections",
                "nothing yet",
                "no clips",
                "none yet",
                "not enough data",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "error state used empty-state wording {forbidden:?}: {html}"
                );
            }
            assert!(lower.contains("couldn't load"), "{html}");
            assert!(html.contains(r#"role="alert""#), "{html}");
        }
    }

    #[test]
    fn what_is_escaped() {
        let html = could_not_load("<img src=x onerror=alert(1)>");
        assert!(!html.contains("<img src=x"), "{html}");
        assert!(html.contains("&lt;img"), "{html}");
    }
}
