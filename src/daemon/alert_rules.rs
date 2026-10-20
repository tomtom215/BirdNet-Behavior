//! The alert rules the detection processor judges each detection against.
//!
//! The rules are re-read for every detection, so an edit on the admin page
//! takes effect on the next bird without a restart. What the read did when it
//! *failed* was the defect: the result was defaulted to an empty list, so one
//! `SQLITE_BUSY` made every rule vanish for that detection. A `suppress` rule —
//! "never notify me about the house sparrow" — stopped suppressing, a webhook
//! rule stopped posting, and the only trace was nothing at all, because the
//! failure had been turned into "there are no rules".
//!
//! [`AlertRules::load`] keeps the rules from the last read that succeeded and
//! judges against them when a read fails, and says so in the log, throttled so
//! a database that stays locked does not write a line per detection.

use std::time::{Duration, Instant};

use birdnet_db::alert_rules::{AlertRule, list_rules};
use rusqlite::Connection;

use crate::integrations::RepeatWarning;

/// Least time between two warnings about rules that cannot be read.
const WARN_EVERY: Duration = Duration::from_secs(300);

/// The rules as of the last successful read.
#[derive(Debug)]
pub(super) struct AlertRules {
    last_good: Vec<AlertRule>,
    warning: RepeatWarning,
}

impl AlertRules {
    pub(super) const fn new() -> Self {
        Self {
            last_good: Vec::new(),
            warning: RepeatWarning::new(WARN_EVERY),
        }
    }

    /// The rules to judge the next detection by: freshly read, or — when the
    /// read fails — the last set that was read.
    ///
    /// With no earlier success (the first read of the process failed) the set
    /// is empty, and the warning says that is why.
    pub(super) fn load(&mut self, conn: &Connection) -> &[AlertRule] {
        self.apply(list_rules(conn).map_err(|e| e.to_string()), Instant::now())
    }

    /// [`Self::load`] with the read's outcome and the clock supplied.
    fn apply(&mut self, read: Result<Vec<AlertRule>, String>, now: Instant) -> &[AlertRule] {
        match read {
            Ok(rules) => {
                if self.warning.succeeded() {
                    tracing::info!("alert rules can be read again");
                }
                self.last_good = rules;
            }
            Err(error) => {
                if let Some(passed_over) = self.warning.failed(now) {
                    tracing::warn!(
                        error = %error,
                        rules_in_force = self.last_good.len(),
                        passed_over,
                        "could not read the alert rules; judging detections by the last set \
                         that was read (none if this is the first read since the station \
                         started, in which case no rule applies)"
                    );
                }
            }
        }
        &self.last_good
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_db::alert_rules::{AlertAction, NewAlertRule, insert_rule};

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        conn
    }

    fn rule(name: &str, action: AlertAction) -> NewAlertRule {
        NewAlertRule {
            name: name.to_owned(),
            enabled: true,
            species_pattern: None,
            confidence_min: 0.0,
            confidence_max: 1.0,
            hour_start: None,
            hour_end: None,
            days_of_week: None,
            cooldown_secs: 0,
            action,
        }
    }

    fn names(rules: &[AlertRule]) -> Vec<&str> {
        rules.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn a_failed_read_keeps_the_rules_that_were_in_force() {
        // The defect: the failure became "there are no rules", so a suppress
        // rule stopped suppressing for as long as the database stayed busy.
        let conn = db();
        insert_rule(&conn, &rule("quiet the sparrow", AlertAction::Suppress)).unwrap();
        let mut cache = AlertRules::new();
        assert_eq!(names(cache.load(&conn)), ["quiet the sparrow"]);

        conn.execute_batch("DROP TABLE alert_rules;").unwrap();
        assert_eq!(
            names(cache.load(&conn)),
            ["quiet the sparrow"],
            "a read that failed switched the rules off"
        );
    }

    #[test]
    fn a_successful_read_replaces_the_rules_including_with_none() {
        // Counterpart: "keep the last good set" must not become "never
        // change". Deleting the last rule on the admin page is a real, empty,
        // successful read.
        let conn = db();
        let id = insert_rule(&conn, &rule("r", AlertAction::Log)).unwrap();
        let mut cache = AlertRules::new();
        assert_eq!(cache.load(&conn).len(), 1);
        birdnet_db::alert_rules::delete_rule(&conn, id).unwrap();
        assert!(
            cache.load(&conn).is_empty(),
            "a deleted rule kept applying after a read that succeeded"
        );
    }

    #[test]
    fn the_first_read_failing_leaves_no_rules_and_does_not_panic() {
        let conn = Connection::open_in_memory().unwrap(); // no table at all
        let mut cache = AlertRules::new();
        assert!(cache.load(&conn).is_empty());
    }
}
