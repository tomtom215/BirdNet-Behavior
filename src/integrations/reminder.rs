//! Re-announcing an episode that is still open, on a widening schedule.
//!
//! # Why (`OB-16`)
//!
//! The three alert loops — [`super::deadman`], [`super::station_health`],
//! [`super::acoustic_health`] — announce an episode **once**. After 2.2 that
//! became *once delivered*, which fixed the alert that never left the box, but
//! the posture was still one push per fault for the life of the process: the
//! only thing that re-armed an episode was a restart. A microphone that failed
//! in April and a station that stopped detecting in May are the same one push,
//! and by August nobody remembers it.
//!
//! Alert storms are the opposite failure and are genuinely well prevented here
//! — three-poll debounce, one episode per key, recovery notices — so the
//! schedule has to widen rather than repeat:
//!
//! | Reminder | Falls due |
//! |---|---|
//! | 1st | 24 h after the episode opened |
//! | 2nd | 72 h after it opened |
//! | 3rd and after | one a week thereafter |
//!
//! Four pushes in the first fortnight, then one a week: a fault lasting four
//! months costs about twenty notifications, which is few enough to still be
//! read and often enough that the operator cannot forget.
//!
//! # What this module deliberately does not do
//!
//! It does not deliver anything and it does not decide whether a fault exists.
//! The loops own the first (through [`super::announce`], whose outbox retries
//! until something lands) and their own state machines own the second. This is
//! the clock alone, so the widening schedule is one testable thing rather than
//! three copies.

use std::time::{Duration, Instant};

/// One day.
const DAY: Duration = Duration::from_secs(24 * 3600);

/// When the first two reminders fall due, measured from the episode's onset.
const FIRST_TWO: [Duration; 2] = [DAY, Duration::from_secs(3 * 24 * 3600)];

/// The spacing of every reminder after those two.
const THEN_EVERY: Duration = Duration::from_secs(7 * 24 * 3600);

/// The re-notification clock for one open episode.
///
/// Created when the episode opens; consulted once per poll. Reminders are
/// counted as **queued**, not as delivered: delivery is the outbox's business,
/// and an outbox entry queued under the same key replaces the one before it,
/// so a station whose uplink is down for a fortnight sends one reminder
/// carrying the current age rather than a fortnight's backlog.
#[derive(Debug, Clone)]
pub(super) struct Reminders {
    /// When the episode was first observed.
    opened: Instant,
    /// How many reminders have been queued for it.
    queued: u32,
}

impl Reminders {
    /// An episode that opened at `now`.
    pub(super) const fn opened_at(now: Instant) -> Self {
        Self {
            opened: now,
            queued: 0,
        }
    }

    /// How long after onset reminder `n` (0-based) falls due.
    ///
    /// `saturating_*` throughout: `n` is bounded in practice by how long a
    /// station runs, but a schedule that panics on an overflow it can never
    /// reach is worse than one that flattens to "a week from the last".
    fn due_after(n: u32) -> Duration {
        FIRST_TWO
            .get(usize::try_from(n).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or_else(|| {
                let last = FIRST_TWO.len() - 1;
                let weeks = n.saturating_sub(u32::try_from(last).unwrap_or(u32::MAX));
                FIRST_TWO[last].saturating_add(THEN_EVERY.saturating_mul(weeks))
            })
    }

    /// If a reminder is due at `now`, consume it and return how long the
    /// episode has been open.
    ///
    /// At most one per call **and** at most one per gap. Advancing `queued` by
    /// a single step would be wrong in a way that only shows up on a station
    /// that was off: a process suspended for a month comes back with four
    /// steps of the schedule already behind it, and a counter that moved one
    /// step per call would fire again at the next five-minute poll, and the one
    /// after that, until it caught up. So every step the gap swallowed is
    /// skipped here, and the next reminder is genuinely in the future.
    pub(super) fn due(&mut self, now: Instant) -> Option<Duration> {
        let open_for = now.saturating_duration_since(self.opened);
        if open_for < Self::due_after(self.queued) {
            return None;
        }
        let mut next = self.queued.saturating_add(1);
        // Terminates: `due_after` grows without bound until it saturates at
        // `Duration::MAX`, which no `open_for` can reach.
        while Self::due_after(next) <= open_for {
            next = next.saturating_add(1);
        }
        self.queued = next;
        Some(open_for)
    }
}

/// "26 hours" / "3 days" — how an operator reads an age.
///
/// Hours below two days, because "1 day" for a 30-hour fault reads as a
/// rounding error rather than as a duration; days above, because nobody counts
/// a four-month outage in hours.
pub(super) fn humanise(d: Duration) -> String {
    let hours = d.as_secs() / 3600;
    if hours < 48 {
        let h = hours.max(1);
        return format!("{h} hour{}", if h == 1 { "" } else { "s" });
    }
    let days = hours / 24;
    format!("{days} day{}", if days == 1 { "" } else { "s" })
}

/// The body of a reminder: what is still wrong, and for how long.
///
/// The age goes first because it is the new information — the rest of the body
/// is what the operator already read when the episode opened.
pub(super) fn still_broken(body: &str, open_for: Duration) -> String {
    format!("Still unresolved after {}.\n\n{body}", humanise(open_for))
}

#[cfg(test)]
mod tests {
    use super::{DAY, Reminders, humanise, still_broken};
    use std::time::{Duration, Instant};

    /// A fixed origin, so every test is arithmetic rather than wall-clock.
    fn t0() -> Instant {
        Instant::now()
    }

    /// The alert loops that must re-announce an open episode.
    ///
    /// A named list rather than three call sites, for the reason 2.5 records:
    /// a wiring that exists only as scattered call sites cannot be checked, and
    /// a loop that quietly stops re-announcing produces no failure, no warning
    /// and no alert — which is what a healthy station produces. Test-only data,
    /// so it lives here rather than pretending to be part of the interface.
    const REMINDING_LOOPS: [&str; 3] = ["deadman", "station_health", "acoustic_health"];

    #[test]
    fn nothing_is_due_in_the_first_day() {
        let now = t0();
        let mut r = Reminders::opened_at(now);
        assert!(r.due(now).is_none());
        assert!(r.due(now + Duration::from_secs(23 * 3600 + 3599)).is_none());
    }

    #[test]
    fn the_schedule_is_24h_then_72h_then_weekly() {
        let now = t0();
        let mut r = Reminders::opened_at(now);
        // 24 h.
        assert!(r.due(now + DAY).is_some());
        // Not again until 72 h.
        assert!(r.due(now + DAY + Duration::from_secs(3600)).is_none());
        assert!(r.due(now + 3 * DAY).is_some());
        // Then one a week: 10 days, 17 days, 24 days.
        assert!(r.due(now + 9 * DAY).is_none());
        assert!(r.due(now + 10 * DAY).is_some());
        assert!(r.due(now + 16 * DAY).is_none());
        assert!(r.due(now + 17 * DAY).is_some());
        assert!(r.due(now + 24 * DAY).is_some());
    }

    #[test]
    fn a_long_gap_sends_one_reminder_not_a_backlog() {
        // A station suspended for a month, or a poll loop that missed ticks,
        // must not empty the whole schedule into one burst. The first version
        // of this test asserted only that ONE reminder came back from ONE call,
        // which is true of any implementation; the real question is what the
        // *next* poll does, five minutes later. It fired again — a counter that
        // advances one step per call replays every step the gap swallowed.
        let now = t0();
        let mut r = Reminders::opened_at(now);
        let month = now + 30 * DAY;
        assert!(r.due(month).is_some());
        assert!(
            r.due(month).is_none(),
            "the schedule fired again on the same instant"
        );
        assert!(
            r.due(month + Duration::from_secs(300)).is_none(),
            "and again at the next poll"
        );
        // The steps the gap swallowed (24 h, 72 h, 10 d, 17 d, 24 d) are all
        // spent, so the next falls a week after the last of them — 31 days
        // after onset, not a week after the moment the gap was noticed.
        assert!(
            r.due(now + 30 * DAY + Duration::from_secs(12 * 3600))
                .is_none()
        );
        assert!(r.due(now + 31 * DAY).is_some());
    }

    #[test]
    fn the_age_carried_is_the_episodes_age_not_the_schedules() {
        // The operator is told how long it has really been broken, which is
        // what makes a late reminder useful rather than misleading.
        let now = t0();
        let mut r = Reminders::opened_at(now);
        let open_for = r.due(now + 5 * DAY).expect("due");
        assert_eq!(open_for, 5 * DAY);
    }

    #[test]
    fn ages_read_as_hours_then_days() {
        assert_eq!(humanise(Duration::from_secs(3600)), "1 hour");
        assert_eq!(humanise(Duration::from_secs(26 * 3600)), "26 hours");
        assert_eq!(humanise(Duration::from_secs(47 * 3600)), "47 hours");
        assert_eq!(humanise(Duration::from_secs(48 * 3600)), "2 days");
        assert_eq!(humanise(120 * DAY), "120 days");
        // Below an hour still says "1 hour" rather than "0 hours"; no reminder
        // can be due that early, so this is only about never printing zero.
        assert_eq!(humanise(Duration::from_secs(60)), "1 hour");
    }

    #[test]
    fn the_reminder_body_keeps_the_original() {
        let body = still_broken("Check the microphone.", 5 * DAY);
        assert!(body.starts_with("Still unresolved after 5 days."));
        assert!(body.contains("Check the microphone."));
    }

    #[test]
    fn every_alert_loop_uses_this_schedule() {
        // The 2.5 lesson, applied here. Every test above exercises the policy;
        // none of them checks that any loop *calls* it, and the first mutation
        // applied to the clock work in 2.5 — deleting a check from the table
        // that drives it — killed nothing for exactly that reason.
        //
        // Two needles, because either alone is weak: the import alone could be
        // a leftover (though CI's `-D warnings` would reject an unused one),
        // and `still_broken` alone could be called with a constant. Together
        // they mean the loop takes an age from a `Reminders` and puts it in
        // front of an operator.
        for loop_name in REMINDING_LOOPS {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/integrations")
                .join(format!("{loop_name}.rs"));
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()));
            assert!(
                source.contains("use super::reminder::"),
                "{loop_name} does not import the re-announcement schedule; an \
                 episode it opens would be announced once and then never again"
            );
            assert!(
                source.contains("still_broken("),
                "{loop_name} imports the schedule but never renders a reminder"
            );
        }
    }
}
