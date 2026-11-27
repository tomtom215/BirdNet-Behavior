//! One `BirdWeather` post per event, where the owner has said which sources
//! share a site (MSC-07).
//!
//! Several sources at one site hear one call and each stores a row. Without a
//! site map every source is its own site (decision 2 of 2026-11-25), and every
//! row is posted, as before. With one (`SOURCE_SITES`), a detection is posted
//! only when it begins an event at its site: the read-time rule of
//! `birdnet_db::sqlite::queries::events`, applied live. A detection joins the
//! event its site has open for that species when it is within the tolerance of
//! the event's first member and its source is not yet in it; otherwise it
//! begins a new one.
//!
//! Live, this follows the order detections arrive in, where the read-time
//! grouping is order-free (it pairs nearest first). The two agree for calls
//! that reach the processor in time order, which live capture gives; a replayed
//! detection from an earlier start is judged against what is open now. The
//! read-time layer is the count; this decides only what is sent.

use std::collections::{BTreeSet, HashMap};

use birdnet_db::sqlite::queries::events::SiteMap;

/// Members of one event are at most this far from its first, in seconds: the
/// read-time default (`events::DEFAULT_TOLERANCE_SECS`, a V2.4 window and a
/// second), which the tests hold this equal to.
pub(super) const TOLERANCE_SECS: i64 = 4;

/// The open event at each site, per species.
#[derive(Debug, Default)]
pub(super) struct OnePerEvent {
    sites: SiteMap,
    tolerance_secs: i64,
    open: HashMap<(String, String), (i64, BTreeSet<String>)>,
}

impl OnePerEvent {
    /// Events at `sites`, members within `tolerance_secs` of the first.
    pub(super) fn new(sites: SiteMap, tolerance_secs: i64) -> Self {
        Self {
            sites,
            tolerance_secs,
            open: HashMap::new(),
        }
    }

    /// Whether this detection begins an event at its site, and so is posted.
    /// A detection with no source or no instant is an event of its own, as at
    /// read time.
    pub(super) fn first_of_event(
        &mut self,
        source: Option<&str>,
        sci_name: &str,
        at_utc: Option<i64>,
    ) -> bool {
        let (Some(source), Some(at)) = (source, at_utc) else {
            return true;
        };
        let key = (self.sites.site_of(source).to_owned(), sci_name.to_owned());
        if let Some((first, members)) = self.open.get_mut(&key)
            && (at - *first).abs() <= self.tolerance_secs
            && !members.contains(source)
        {
            members.insert(source.to_owned());
            return false;
        }
        self.open
            .insert(key, (at, std::iter::once(source.to_owned()).collect()));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yard() -> OnePerEvent {
        OnePerEvent::new(
            SiteMap::identity()
                .with("north", "yard")
                .with("south", "yard"),
            TOLERANCE_SECS,
        )
    }

    #[test]
    fn the_tolerance_is_the_read_time_default() {
        assert!(
            (f64::from(u32::try_from(TOLERANCE_SECS).unwrap())
                - birdnet_db::sqlite::queries::events::DEFAULT_TOLERANCE_SECS)
                .abs()
                < f64::EPSILON
        );
    }

    /// One call at one site is one event; the second source joins it.
    #[test]
    fn a_second_source_at_the_site_joins_the_event() {
        let mut e = yard();
        assert!(e.first_of_event(Some("north"), "Pica pica", Some(100)));
        assert!(!e.first_of_event(Some("south"), "Pica pica", Some(103)));
    }

    /// The counterparts: past the tolerance, another species, the same source
    /// again, another site, and a row with no source or instant each begin an
    /// event of their own.
    #[test]
    fn what_does_not_join_begins_its_own_event() {
        let mut e = yard();
        assert!(e.first_of_event(Some("north"), "Pica pica", Some(100)));
        assert!(
            e.first_of_event(Some("south"), "Pica pica", Some(105)),
            "past 4 s"
        );
        assert!(
            e.first_of_event(Some("north"), "Turdus merula", Some(105)),
            "species"
        );
        assert!(
            e.first_of_event(Some("south"), "Pica pica", Some(106)),
            "same source"
        );
        assert!(
            e.first_of_event(Some("pond"), "Pica pica", Some(105)),
            "another site"
        );
        assert!(e.first_of_event(None, "Pica pica", Some(105)), "no source");
        assert!(
            e.first_of_event(Some("north"), "Pica pica", None),
            "no instant"
        );
        let mut alone = OnePerEvent::new(SiteMap::identity(), TOLERANCE_SECS);
        assert!(alone.first_of_event(Some("north"), "Pica pica", Some(100)));
        assert!(
            alone.first_of_event(Some("south"), "Pica pica", Some(100)),
            "with no site map every source is its own site"
        );
    }
}
