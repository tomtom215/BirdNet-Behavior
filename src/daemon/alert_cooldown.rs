//! The cooldown on alert-rule webhooks: a rule posts for a species at most once
//! per window.
//!
//! # The problem
//!
//! A webhook rule used to fire on every detection that matched it. A calling
//! owl is detected in each 3 s chunk of each segment, and
//! `DUPLICATE_INTERVAL_SECS` is off by default, so a rule on `Strix*` posted to
//! its endpoint once per chunk for as long as the bird called: hundreds an hour
//! into Home Assistant, a Discord channel or a phone.
//! tphakala/birdnet-go's alert engine has a cooldown per rule and species
//! (`CooldownSec`); [`AlertCooldowns`] is this daemon's.
//!
//! # What it decides, and on whose clock
//!
//! [`AlertCooldowns::should_fire`] answers for one `(rule, species)` pair, on
//! the **detection's own timestamp** — the same convention as the duplicate
//! gate ([`super::duplicate`]), for the same reasons: the daemon can be minutes
//! behind on a backlog, and files are not processed in timestamp order, so the
//! comparison is on the absolute difference. A wall clock would also be wrong
//! the other way round: a station that spent the night offline and replays it
//! would see every call of the night as simultaneous, and post once for all of
//! it.
//!
//! The consequence is that the limit is on *recorded* time, not on the rate the
//! endpoint receives posts: a replayed backlog posts once per window of the
//! recording, in a burst. That is the price of not merging a night's distinct
//! visits into one, and it is bounded by the length of the backlog divided by
//! the window — where before it was one post per chunk.
//!
//! A detection whose timestamp cannot be read is **let through** and nothing is
//! remembered for it: `Date`/`Time` are free-form text, and an alert is worth
//! more than the rate limit.
//!
//! # What it remembers
//!
//! One entry per `(rule, species)` that has fired, holding when and for how
//! long. A cooldown of `0` is "no cooldown" — the value every rule written
//! before migration 55 has — and is never recorded, so a station whose rules
//! all carry it holds nothing here.
//!
//! The map is bounded by species × rules, and pruned: once it grows past
//! [`PRUNE_ABOVE`] entries, every entry whose window has passed is dropped.
//! Such an entry could no longer suppress anything, so dropping it changes no
//! decision; it only stops a long-running station accumulating one entry per
//! species it has ever heard.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// Entries held before expired ones are dropped.
///
/// Far above what a station's rules and species produce in a window (a rule
/// per species it can name, over the dozens of species a site hears), so the
/// prune is a backstop for a long-lived process, not something a normal
/// morning reaches.
const PRUNE_ABOVE: usize = 512;

/// One claim: when the rule last fired for the species, and how long it then
/// stayed quiet.
#[derive(Debug, Clone, Copy)]
struct Claim {
    /// The detection timestamp the post was made for, in seconds.
    at_secs: i64,
    /// The rule's cooldown when it fired.
    window_secs: i64,
}

impl Claim {
    /// Whether `at_secs` is inside this claim's window. Absolute, because
    /// files arrive out of order; see the module doc.
    const fn covers(self, at_secs: i64) -> bool {
        at_secs.saturating_sub(self.at_secs).saturating_abs() < self.window_secs
    }
}

/// When each `(rule, species)` last fired a webhook.
///
/// Cheap to clone and shared: the processor claims a slot before it spawns a
/// post, and the post's task gives the slot back if the request never left
/// ([`Self::release`]). The lock is held for a map operation, never across I/O.
#[derive(Debug, Clone, Default)]
pub(super) struct AlertCooldowns {
    claims: Arc<Mutex<HashMap<(i64, String), Claim>>>,
}

impl AlertCooldowns {
    /// Whether rule `rule_id` may post for `species` now, claiming the slot if
    /// it may.
    ///
    /// `cooldown_secs` is the rule's own window; `0` never suppresses.
    /// `at_secs` is the detection's timestamp (see the module doc), `None` when
    /// it could not be read, which is let through.
    ///
    /// The claim is made here, before the post, so the other chunks of the same
    /// song that arrive while the request is in flight do not each decide to
    /// send.
    pub(super) fn should_fire(
        &self,
        rule_id: i64,
        species: &str,
        cooldown_secs: u32,
        at_secs: Option<i64>,
    ) -> bool {
        let Some(at_secs) = at_secs else { return true };
        if cooldown_secs == 0 {
            return true;
        }
        let mut claims = self.claims.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (rule_id, species.to_owned());
        if claims.get(&key).is_some_and(|c| c.covers(at_secs)) {
            return false;
        }
        if claims.len() >= PRUNE_ABOVE {
            claims.retain(|_, c| c.covers(at_secs));
        }
        claims.insert(
            key,
            Claim {
                at_secs,
                window_secs: i64::from(cooldown_secs),
            },
        );
        true
    }

    /// Give back the slot [`Self::should_fire`] claimed, because the post it was
    /// claimed for never left.
    ///
    /// A webhook that failed on the wire (the endpoint restarting, the network
    /// down) should not cost the rule its next window: the next detection of
    /// the bird tries again. The same rule the Apprise cooldown follows for a
    /// notification that did not go.
    ///
    /// Only the claim made for `at_secs` is released. By the time a slow
    /// request fails, a later detection may have claimed a fresh slot, and
    /// deleting that one would let a post through inside its window.
    pub(super) fn release(&self, rule_id: i64, species: &str, at_secs: Option<i64>) {
        let Some(at_secs) = at_secs else { return };
        let mut claims = self.claims.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (rule_id, species.to_owned());
        if claims.get(&key).is_some_and(|c| c.at_secs == at_secs) {
            claims.remove(&key);
        }
    }

    /// How many `(rule, species)` pairs are remembered.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.claims
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::{AlertCooldowns, PRUNE_ABOVE};

    /// Seconds for a time on 2026-03-14.
    fn t(hms: &str) -> Option<i64> {
        crate::daemon::duplicate::detection_secs("2026-03-14", hms)
    }

    const OWL: &str = "Strix aluco";
    const ROBIN: &str = "Erithacus rubecula";

    // ── the window ──────────────────────────────────────────────────────

    #[test]
    fn the_first_detection_fires_and_the_rest_of_the_song_does_not() {
        let g = AlertCooldowns::default();
        assert!(g.should_fire(1, OWL, 300, t("05:00:00")));
        for hms in ["05:00:03", "05:00:06", "05:01:00", "05:04:59"] {
            assert!(
                !g.should_fire(1, OWL, 300, t(hms)),
                "the call at {hms} posted inside the window"
            );
        }
    }

    #[test]
    fn the_rule_posts_again_once_the_window_has_passed() {
        // Counterpart, and the reason this is a window and not a mute: a gate
        // that never reopened would alert once per species for the life of the
        // station.
        let g = AlertCooldowns::default();
        assert!(g.should_fire(1, OWL, 300, t("05:00:00")));
        assert!(!g.should_fire(1, OWL, 300, t("05:04:59")));
        assert!(g.should_fire(1, OWL, 300, t("05:05:00")));
    }

    #[test]
    fn a_suppressed_call_does_not_extend_the_window() {
        // Otherwise an owl calling every three seconds would be shut out for
        // good rather than heard once per window.
        let g = AlertCooldowns::default();
        assert!(g.should_fire(1, OWL, 10, t("05:00:00")));
        assert!(!g.should_fire(1, OWL, 10, t("05:00:05")));
        assert!(!g.should_fire(1, OWL, 10, t("05:00:09")));
        assert!(
            g.should_fire(1, OWL, 10, t("05:00:10")),
            "a suppressed call pushed the window forward"
        );
    }

    #[test]
    fn a_backlog_replayed_out_of_order_is_still_one_post_per_window() {
        // Files are not processed in timestamp order after a restart, so a
        // signed comparison would post for every earlier file that arrives
        // after a later one.
        let g = AlertCooldowns::default();
        assert!(g.should_fire(1, OWL, 300, t("05:02:00")));
        assert!(!g.should_fire(1, OWL, 300, t("05:00:30")));
        assert!(!g.should_fire(1, OWL, 300, t("05:01:00")));
        // And one that arrives late but is hours older is a different visit,
        // not a repeat: with a signed gap every earlier file reads as "inside
        // the window" however long before it was recorded.
        assert!(
            g.should_fire(1, OWL, 300, t("01:00:00")),
            "a call heard hours earlier was swallowed by a later one's window"
        );
    }

    // ── scope ───────────────────────────────────────────────────────────

    #[test]
    fn each_species_has_its_own_window() {
        // A dawn chorus is many species at once. A rule on `*` that treated
        // them as one would post for the first bird and stay silent about the
        // rest of the chorus.
        let g = AlertCooldowns::default();
        assert!(g.should_fire(1, OWL, 300, t("05:00:00")));
        assert!(
            g.should_fire(1, ROBIN, 300, t("05:00:01")),
            "one species' window silenced another"
        );
        assert!(!g.should_fire(1, OWL, 300, t("05:00:02")));
        assert!(!g.should_fire(1, ROBIN, 300, t("05:00:03")));
    }

    #[test]
    fn each_rule_has_its_own_window() {
        // Two rules aimed at different endpoints for the same bird must both
        // be told, once each.
        let g = AlertCooldowns::default();
        assert!(g.should_fire(1, OWL, 300, t("05:00:00")));
        assert!(
            g.should_fire(2, OWL, 300, t("05:00:01")),
            "one rule's window silenced another rule"
        );
        assert!(!g.should_fire(1, OWL, 300, t("05:00:02")));
        assert!(!g.should_fire(2, OWL, 300, t("05:00:03")));
    }

    #[test]
    fn each_rule_keeps_its_own_length_of_window() {
        let g = AlertCooldowns::default();
        assert!(g.should_fire(1, OWL, 10, t("05:00:00")));
        assert!(g.should_fire(2, OWL, 600, t("05:00:00")));
        // Thirty seconds on: the short window has passed, the long one not.
        assert!(g.should_fire(1, OWL, 10, t("05:00:30")));
        assert!(!g.should_fire(2, OWL, 600, t("05:00:30")));
    }

    // ── no cooldown, and what is let through ────────────────────────────

    #[test]
    fn a_cooldown_of_zero_never_suppresses_and_is_never_remembered() {
        // Every rule written before migration 55 carries 0, and must keep
        // posting on every match. Nothing is recorded for it either, so a
        // station of such rules holds no state.
        let g = AlertCooldowns::default();
        for hms in ["05:00:00", "05:00:00", "05:00:03", "05:00:03"] {
            assert!(g.should_fire(1, OWL, 0, t(hms)), "{hms}");
        }
        assert_eq!(g.len(), 0);
    }

    #[test]
    fn a_detection_with_no_readable_time_is_let_through_and_not_remembered() {
        let g = AlertCooldowns::default();
        assert!(g.should_fire(1, OWL, 300, None));
        assert!(g.should_fire(1, OWL, 300, None));
        assert_eq!(g.len(), 0);
        // And it does not disturb a window that is running.
        assert!(g.should_fire(1, OWL, 300, t("05:00:00")));
        assert!(g.should_fire(1, OWL, 300, None));
        assert!(!g.should_fire(1, OWL, 300, t("05:00:03")));
    }

    // ── a post that never left ──────────────────────────────────────────

    #[test]
    fn a_post_that_never_left_gives_its_window_back() {
        let g = AlertCooldowns::default();
        let at = t("05:00:00");
        assert!(g.should_fire(1, OWL, 300, at));
        assert!(!g.should_fire(1, OWL, 300, t("05:00:03")));
        g.release(1, OWL, at);
        assert!(
            g.should_fire(1, OWL, 300, t("05:00:06")),
            "a failed post cost the rule its window"
        );
    }

    #[test]
    fn releasing_a_stale_claim_does_not_open_a_newer_window() {
        // A slow request fails after a later detection has claimed a fresh
        // slot; giving back *that* one would let a post through inside it.
        let g = AlertCooldowns::default();
        let first = t("05:00:00");
        assert!(g.should_fire(1, OWL, 10, first));
        assert!(g.should_fire(1, OWL, 10, t("05:00:10"))); // a new window
        g.release(1, OWL, first);
        assert!(
            !g.should_fire(1, OWL, 10, t("05:00:12")),
            "the first request's failure released the second request's claim"
        );
    }

    // ── bounded ─────────────────────────────────────────────────────────

    #[test]
    fn the_map_does_not_grow_with_every_species_ever_heard() {
        // One new species a minute for a day and a half, a ten-minute window:
        // at most ten are ever live, so a map that kept them all would hold
        // two thousand.
        let g = AlertCooldowns::default();
        let start = t("00:00:00").unwrap();
        for i in 0..2_000_i64 {
            let species = format!("Species {i}");
            assert!(g.should_fire(1, &species, 600, Some(start + i * 60)));
            assert!(
                g.len() <= PRUNE_ABOVE,
                "{} entries held after {i} species",
                g.len()
            );
        }
    }

    #[test]
    fn expired_entries_are_kept_until_the_map_reaches_the_threshold() {
        // The prune is a backstop, not a sweep: below `PRUNE_ABOVE` an expired
        // window is left where it is, and the claim that finds the map AT the
        // threshold drops every one. The bound test above cannot tell this from
        // a prune on every claim, which also keeps the map small.
        let g = AlertCooldowns::default();
        let start = t("00:00:00").unwrap();
        let at_threshold = i64::try_from(PRUNE_ABOVE).unwrap();
        // One species an hour, each with a one-minute window: every earlier
        // entry has expired by the time the next arrives.
        for i in 0..at_threshold {
            assert!(g.should_fire(1, &format!("Species {i}"), 60, Some(start + i * 3_600)));
        }
        assert_eq!(
            g.len(),
            PRUNE_ABOVE,
            "an expired entry was dropped below the threshold"
        );
        assert!(g.should_fire(1, "One more", 60, Some(start + at_threshold * 3_600)));
        assert_eq!(
            g.len(),
            1,
            "the claim that found the map at the threshold did not drop the expired entries"
        );
    }

    #[test]
    fn pruning_keeps_every_window_that_is_still_running() {
        // Counterpart: the prune must drop only what could no longer suppress.
        // Filling past the threshold with live entries and then asking about
        // the first of them must still say "inside its window".
        let g = AlertCooldowns::default();
        let start = t("00:00:00").unwrap();
        let live = i64::try_from(PRUNE_ABOVE).unwrap() + 50;
        for i in 0..live {
            assert!(g.should_fire(1, &format!("Species {i}"), 3_600, Some(start + i)));
        }
        assert!(
            !g.should_fire(1, "Species 0", 3_600, Some(start + live)),
            "a prune dropped a window that was still running"
        );
        assert_eq!(g.len(), usize::try_from(live).unwrap());
    }
}
