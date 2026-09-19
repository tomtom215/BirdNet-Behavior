//! A container that re-fetches on a timer must not be an `aria-live` region.
//!
//! `#detections-table` polls every 15s and `#rc-trickle-feed` every 10s, both
//! with `hx-swap="innerHTML"`. A swap is a mutation, and a mutation inside a
//! live region is an announcement — so with `aria-live` on the container a
//! screen reader read the whole feed out again on every tick: eight rows of
//! species, scientific name, confidence and time, four times a minute,
//! whether or not a bird had been heard.
//!
//! Observed, before the fix, by watching the DOM: three forced refetches of an
//! unchanged feed produced three container mutations. After it, the same three
//! refetches produce three container mutations and **zero** status-line
//! mutations, while renaming the newest row produces exactly one announcement
//! ("New detection: Painted Bunting at 16:56").
//!
//! The announcement now rides a small `role="status"` line that a script in
//! `layout.html` writes only when the newest row's label actually differs.
//! This gate stops the attribute being put back on the container, and stops
//! the status line it was moved to from going missing.

use std::path::Path;

/// The `<div id="...">` opening tag for `id`, wherever it is written.
fn opening_tag<'a>(src: &'a str, id: &str) -> Option<&'a str> {
    let at = src.find(&format!("id=\"{id}\""))?;
    let start = src[..at].rfind('<')?;
    let end = src[start..].find('>')? + start;
    Some(&src[start..=end])
}

#[test]
fn polling_feeds_announce_through_a_status_line_not_themselves() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases: [(&str, &str, &str); 2] = [
        ("templates/today.html", "detections-table", "td-feed-status"),
        (
            "src/routes/pages/recordings.rs",
            "rc-trickle-feed",
            "rc-feed-status",
        ),
    ];

    for (file, feed_id, status_id) in cases {
        let src = std::fs::read_to_string(root.join(file))
            .unwrap_or_else(|e| panic!("{file} is readable: {e}"));
        let tag = opening_tag(&src, feed_id)
            .unwrap_or_else(|| panic!("{file} still declares #{feed_id}"));

        assert!(
            tag.contains("hx-trigger") && tag.contains("every"),
            "{file}: #{feed_id} is expected to be a polling feed; if it stopped \
             polling this gate is guarding nothing and should be revisited. Tag: {tag}"
        );
        assert!(
            !tag.contains("aria-live"),
            "{file}: #{feed_id} polls on a timer and carries aria-live, so every \
             swap re-announces the entire feed. Put the announcement on \
             #{status_id} instead, which is written only when the newest row \
             changes. Tag: {tag}"
        );
        assert!(
            tag.contains("aria-label"),
            "{file}: #{feed_id} needs a name now that it is not a live region. Tag: {tag}"
        );
        assert!(
            src.contains(&format!("id=\"{status_id}\"")),
            "{file}: the status line #{status_id} that #{feed_id}'s announcement \
             was moved to is gone, so nothing announces new detections at all"
        );
    }

    // And the announcer that drives them is still wired up.
    let layout = std::fs::read_to_string(root.join("templates/layout.html")).expect("layout.html");
    for id in ["td-feed-status", "rc-feed-status"] {
        assert!(
            layout.contains(id),
            "layout.html no longer references {id}, so the status line is never written"
        );
    }
}

/// The page heading must not sit in a region that re-announces on a timer.
///
/// `#today-phrase` carries the document's only `<h1>` and polls every five
/// minutes; with `aria-live` it read the page heading and subtitle out again
/// on every poll.
#[test]
fn the_page_heading_is_not_re_announced_every_five_minutes() {
    let layout =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/today.html"))
            .expect("today.html");
    let tag = opening_tag(&layout, "today-phrase").expect("today.html declares #today-phrase");
    assert!(
        !tag.contains("aria-live"),
        "#today-phrase holds the page <h1> and polls every 5m; with aria-live \
         the heading is announced again on every poll. Tag: {tag}"
    );
}
