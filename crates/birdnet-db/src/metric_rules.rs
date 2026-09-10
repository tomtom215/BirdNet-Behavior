//! Operator-defined alerts on the station's own measurements (`G-29`).
//!
//! # What this is for
//!
//! The station failure that loses a season is silent. The disk fills, or one
//! microphone of three dies, and nothing says so until somebody looks at a
//! chart weeks later. `station_health` already alerts on a fixed set of
//! conditions — disk over a threshold, a source flapping, the clock adrift —
//! and those thresholds are compiled in. They are the right defaults and they
//! are not everyone's: the rule that catches a dying microphone at a particular
//! station is *"tell me when the hourly detection count drops below what it
//! normally is here"*, and no number chosen in this repository can be that.
//!
//! # What it is not
//!
//! Not an extension of [`crate::alert_rules`], which is the other thing in this
//! database with "rule" in its name. Those match one **detection** as it
//! arrives — species, confidence, hour — and fire an action of their own
//! (webhook, log, suppress). One of these is a **sampled measurement** compared
//! against a threshold every five minutes, and it produces a *station-health
//! condition* rather than an action. That is the whole reason for the
//! separation: a condition inherits the fifteen-minute debounce, the episode
//! latching, the recovery notice, the notification log and the store-and-forward
//! outbox that path already has, none of which the detection-rule action path
//! has or should grow.
//!
//! The consequence worth stating: a metric rule has **no per-rule cooldown
//! field**. The episode machinery is the cooldown — a rule that fires stays in
//! one episode until the measurement recovers, and is re-announced on the
//! reminder schedule rather than on every poll.

use rusqlite::{Connection, params};
use std::fmt;

/// Errors from metric-rule operations.
#[derive(Debug)]
pub enum MetricRuleError {
    /// `SQLite` error.
    Sqlite(rusqlite::Error),
    /// A field would not be usable as a rule.
    Invalid(String),
}

impl fmt::Display for MetricRuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::Invalid(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for MetricRuleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Invalid(_) => None,
        }
    }
}

impl From<rusqlite::Error> for MetricRuleError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// A measurement a rule can be written against.
///
/// A closed vocabulary rather than free text: an operator cannot invent a
/// metric the station does not sample, and a rule stored against a name that
/// was removed is reported instead of silently never firing. Every variant is
/// something the station already measures for another reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Metric {
    /// Percentage of the recordings volume in use.
    DiskPercent,
    /// Percentage of system memory in use.
    MemoryPercent,
    /// CPU temperature in degrees Celsius.
    CpuTemperatureC,
    /// Detections recorded in the last hour.
    DetectionsPerHour,
    /// Seconds since the most recent detection.
    SecondsSinceLastDetection,
    /// The highest per-source capture restart count over the last hour.
    ///
    /// The *maximum* across sources rather than a sum: three sources restarting
    /// once each is not the same fault as one source restarting three times,
    /// and it is the second that a rule is written to catch.
    MaxCaptureRestartsPerHour,
    /// Payloads parked in the store-and-forward queue.
    OutboundQueueDepth,
}

impl Metric {
    /// Every metric, for the settings UI and the gates.
    ///
    /// Exhaustively matched below so a variant added without a name, a unit or
    /// a description fails to compile.
    pub const ALL: [Self; 7] = [
        Self::DiskPercent,
        Self::MemoryPercent,
        Self::CpuTemperatureC,
        Self::DetectionsPerHour,
        Self::SecondsSinceLastDetection,
        Self::MaxCaptureRestartsPerHour,
        Self::OutboundQueueDepth,
    ];

    /// The stored name. Stable: it is in the database and in exports.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::DiskPercent => "disk_pct",
            Self::MemoryPercent => "mem_pct",
            Self::CpuTemperatureC => "cpu_temp_c",
            Self::DetectionsPerHour => "detections_per_hour",
            Self::SecondsSinceLastDetection => "seconds_since_last_detection",
            Self::MaxCaptureRestartsPerHour => "capture_restarts_per_hour",
            Self::OutboundQueueDepth => "outbound_queue_depth",
        }
    }

    /// The stored name, parsed.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == s)
    }

    /// What an operator sees.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::DiskPercent => "Disk in use",
            Self::MemoryPercent => "Memory in use",
            Self::CpuTemperatureC => "CPU temperature",
            Self::DetectionsPerHour => "Detections in the last hour",
            Self::SecondsSinceLastDetection => "Seconds since the last detection",
            Self::MaxCaptureRestartsPerHour => "Capture restarts in the last hour (worst source)",
            Self::OutboundQueueDepth => "Uploads waiting to be sent",
        }
    }

    /// The unit, for rendering a threshold and an alert body.
    #[must_use]
    pub const fn unit(self) -> &'static str {
        match self {
            Self::DiskPercent | Self::MemoryPercent => "%",
            Self::CpuTemperatureC => "°C",
            Self::SecondsSinceLastDetection => "s",
            Self::DetectionsPerHour
            | Self::MaxCaptureRestartsPerHour
            | Self::OutboundQueueDepth => "",
        }
    }
}

/// How a measurement is compared with the threshold.
///
/// Two operators, not six. `>` and `<` are what an alert is: "too much of
/// something" or "too little". Equality on a sampled float is a rule that never
/// fires, and `>=` versus `>` on a threshold an operator typed is a distinction
/// without a difference — offering them would be offering four ways to write
/// the same rule and one way to write a broken one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    /// Fires while the measurement is strictly above the threshold.
    Above,
    /// Fires while the measurement is strictly below the threshold.
    Below,
}

impl Comparison {
    /// The stored name.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Above => "above",
            Self::Below => "below",
        }
    }

    /// The stored name, parsed.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "above" => Some(Self::Above),
            "below" => Some(Self::Below),
            _ => None,
        }
    }

    /// Whether `value` trips this comparison against `threshold`.
    #[must_use]
    pub fn trips(self, value: f64, threshold: f64) -> bool {
        match self {
            Self::Above => value > threshold,
            Self::Below => value < threshold,
        }
    }

    /// A word for an alert body.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Above => "above",
            Self::Below => "below",
        }
    }
}

/// A stored rule.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricRule {
    /// Row id.
    pub id: i64,
    /// What the operator called it. Appears in the alert.
    pub name: String,
    /// Whether it is evaluated.
    pub enabled: bool,
    /// The measurement.
    pub metric: Metric,
    /// The direction.
    pub comparison: Comparison,
    /// The threshold, in the metric's own unit.
    pub threshold: f64,
}

/// A rule as submitted, before it has an id.
#[derive(Debug, Clone, PartialEq)]
pub struct NewMetricRule {
    /// What the operator called it.
    pub name: String,
    /// Whether it starts enabled.
    pub enabled: bool,
    /// The measurement.
    pub metric: Metric,
    /// The direction.
    pub comparison: Comparison,
    /// The threshold.
    pub threshold: f64,
}

/// Longest rule name accepted.
///
/// The name goes into an alert title, which goes into a push notification; past
/// this it is truncated by something downstream instead of by us.
pub const MAX_NAME_LEN: usize = 80;

impl NewMetricRule {
    /// Reject a rule that could not do anything useful.
    ///
    /// # Errors
    ///
    /// [`MetricRuleError::Invalid`] with a message for the operator.
    pub fn validate(&self) -> Result<(), MetricRuleError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(MetricRuleError::Invalid(
                "give the rule a name — it is what the alert will be titled".into(),
            ));
        }
        if name.chars().count() > MAX_NAME_LEN {
            return Err(MetricRuleError::Invalid(format!(
                "the name is longer than {MAX_NAME_LEN} characters"
            )));
        }
        if !self.threshold.is_finite() {
            return Err(MetricRuleError::Invalid(
                "the threshold is not a number".into(),
            ));
        }
        // A rule that can never stop firing is not an alert, it is a stuck
        // notification. Percentages are the case that bites: "disk above -1"
        // fires on every poll of every station, for ever.
        if matches!(self.metric, Metric::DiskPercent | Metric::MemoryPercent) {
            if self.comparison == Comparison::Above && self.threshold < 0.0 {
                return Err(MetricRuleError::Invalid(
                    "a percentage is never below zero, so this rule would fire for ever".into(),
                ));
            }
            if self.comparison == Comparison::Below && self.threshold > 100.0 {
                return Err(MetricRuleError::Invalid(
                    "a percentage is never above 100, so this rule would fire for ever".into(),
                ));
            }
        }
        Ok(())
    }
}

/// A measurement taken this poll.
///
/// `None` for a metric the station cannot read right now — no capture
/// supervisor, a temperature sensor this board does not have. A rule against a
/// metric with no sample does not fire and does not error: "cannot tell" is not
/// "fine", but it is also not a fault to wake somebody for.
pub type Sample = Option<f64>;

/// One rule that is currently tripped.
#[derive(Debug, Clone, PartialEq)]
pub struct Firing {
    /// The rule.
    pub rule: MetricRule,
    /// What the measurement read.
    pub value: f64,
}

impl Firing {
    /// A stable key for episode tracking, unique per rule.
    #[must_use]
    pub fn key(&self) -> String {
        format!("metric-rule:{}", self.rule.id)
    }

    /// The alert title.
    #[must_use]
    pub fn title(&self) -> String {
        format!("Alert: {}", self.rule.name)
    }

    /// The alert body: the measurement, the threshold, and the rule that joined
    /// them, so a person woken by this does not have to go and look up which
    /// rule fired.
    #[must_use]
    pub fn body(&self) -> String {
        let unit = self.rule.metric.unit();
        format!(
            "{} is {}{unit}, which is {} the {}{unit} this rule watches for.",
            self.rule.metric.label(),
            trim_number(self.value),
            self.rule.comparison.describe(),
            trim_number(self.rule.threshold),
        )
    }
}

/// A number without a pointless `.0`, and never in exponential form.
fn trim_number(v: f64) -> String {
    if (v.fract()).abs() < f64::EPSILON {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// Every enabled rule that is tripped by `sample_of`.
///
/// Pure, so the whole policy is testable without a station: `sample_of` returns
/// the measurement for a metric, or `None` when it cannot be read.
#[must_use]
pub fn evaluate(rules: &[MetricRule], mut sample_of: impl FnMut(Metric) -> Sample) -> Vec<Firing> {
    let mut out = Vec::new();
    for rule in rules {
        if !rule.enabled {
            continue;
        }
        let Some(value) = sample_of(rule.metric) else {
            continue;
        };
        if rule.comparison.trips(value, rule.threshold) {
            out.push(Firing {
                rule: rule.clone(),
                value,
            });
        }
    }
    out
}

/// Every rule, newest first.
///
/// # Errors
///
/// Returns [`MetricRuleError`] on a `SQLite` failure.
pub fn list(conn: &Connection) -> Result<Vec<MetricRule>, MetricRuleError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, enabled, metric, comparison, threshold
         FROM metric_rules ORDER BY id DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, f64>(5)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, name, enabled, metric, comparison, threshold) = row?;
        // A row whose metric or comparison no longer parses is skipped with a
        // warning rather than failing the whole list: one rule written against
        // a metric a later version removed must not take the page down with it.
        let (Some(metric), Some(comparison)) =
            (Metric::parse(&metric), Comparison::parse(&comparison))
        else {
            tracing::warn!(
                id,
                metric,
                comparison,
                "skipping a metric rule this version does not understand"
            );
            continue;
        };
        out.push(MetricRule {
            id,
            name,
            enabled: enabled != 0,
            metric,
            comparison,
            threshold,
        });
    }
    Ok(out)
}

/// Store a new rule, returning its id.
///
/// # Errors
///
/// [`MetricRuleError::Invalid`] when the rule would not do anything useful,
/// or a `SQLite` failure.
pub fn insert(conn: &Connection, rule: &NewMetricRule) -> Result<i64, MetricRuleError> {
    rule.validate()?;
    conn.execute(
        "INSERT INTO metric_rules (name, enabled, metric, comparison, threshold)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            rule.name.trim(),
            i64::from(rule.enabled),
            rule.metric.key(),
            rule.comparison.key(),
            rule.threshold,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Remove a rule. `false` when there was none with that id.
///
/// # Errors
///
/// Returns [`MetricRuleError`] on a `SQLite` failure.
pub fn delete(conn: &Connection, id: i64) -> Result<bool, MetricRuleError> {
    let n = conn.execute("DELETE FROM metric_rules WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

/// Flip a rule's enabled flag, returning the new state.
///
/// `None` when there is no rule with that id.
///
/// # Errors
///
/// Returns [`MetricRuleError`] on a `SQLite` failure.
pub fn toggle(conn: &Connection, id: i64) -> Result<Option<bool>, MetricRuleError> {
    let n = conn.execute(
        "UPDATE metric_rules SET enabled = 1 - enabled WHERE id = ?1",
        params![id],
    )?;
    if n == 0 {
        return Ok(None);
    }
    let enabled: i64 = conn.query_row(
        "SELECT enabled FROM metric_rules WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    Ok(Some(enabled != 0))
}

#[cfg(test)]
mod tests {
    use super::{
        Comparison, MAX_NAME_LEN, Metric, MetricRule, NewMetricRule, delete, evaluate, insert,
        list, toggle,
    };
    use rusqlite::Connection;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        crate::migration::migrate(&conn).expect("migrate");
        conn
    }

    fn rule(id: i64, metric: Metric, comparison: Comparison, threshold: f64) -> MetricRule {
        MetricRule {
            id,
            name: format!("rule {id}"),
            enabled: true,
            metric,
            comparison,
            threshold,
        }
    }

    /// Both directions fire on the right side of the threshold, and neither
    /// fires on the other.
    ///
    /// Written as both halves because a comparison that always fired would pass
    /// a test that only checked the firing side, and one that never fired would
    /// pass a test that only checked the quiet side.
    #[test]
    fn a_rule_fires_on_its_own_side_of_the_threshold_and_not_the_other() {
        let above = rule(1, Metric::DiskPercent, Comparison::Above, 85.0);
        assert_eq!(
            evaluate(std::slice::from_ref(&above), |_| Some(90.0)).len(),
            1
        );
        assert!(evaluate(std::slice::from_ref(&above), |_| Some(80.0)).is_empty());

        let below = rule(2, Metric::DetectionsPerHour, Comparison::Below, 20.0);
        assert_eq!(
            evaluate(std::slice::from_ref(&below), |_| Some(3.0)).len(),
            1
        );
        assert!(evaluate(&[below], |_| Some(50.0)).is_empty());

        // Exactly at the threshold is not "above" or "below" it. A rule for
        // "85% disk" firing at exactly 85.0 would be firing at a value the
        // operator called acceptable.
        assert!(evaluate(&[above], |_| Some(85.0)).is_empty());
    }

    /// A metric with no reading does not fire.
    ///
    /// "Cannot tell" is not "fine", but it is certainly not a fault to wake
    /// somebody for: a board with no temperature sensor is not cold, and a
    /// station with no capture supervisor has not had zero restarts.
    ///
    /// The rule here is a **`Below`** one, deliberately. An `Above` rule is a
    /// weak test of this: the obvious wrong implementation — treating a missing
    /// sample as `0.0` — does not trip `above 80`, so an `Above` rule passes
    /// against it. Measured: that mutation survived the first version of this
    /// test. `below 20` is the case where zero is indistinguishable from a
    /// fault, and it is also the shape of the rule this feature exists for
    /// ("hourly detections below what they normally are here"), so a station
    /// that cannot read the metric would have alerted for ever.
    #[test]
    fn a_metric_with_no_sample_does_not_fire() {
        let r = rule(1, Metric::DetectionsPerHour, Comparison::Below, 20.0);
        assert!(
            evaluate(std::slice::from_ref(&r), |_| None).is_empty(),
            "a missing sample must not be read as zero"
        );

        // The counterpart: with a sample below the threshold it does fire, so
        // the assertion above is about the missing reading and not about the
        // rule being inert.
        assert_eq!(evaluate(&[r], |_| Some(3.0)).len(), 1);

        // And the same for a metric that is often simply absent.
        let hot = rule(2, Metric::CpuTemperatureC, Comparison::Above, 80.0);
        assert!(evaluate(std::slice::from_ref(&hot), |_| None).is_empty());
        assert_eq!(evaluate(&[hot], |_| Some(95.0)).len(), 1);
    }

    /// A disabled rule is not evaluated.
    #[test]
    fn a_disabled_rule_does_not_fire() {
        let mut r = rule(1, Metric::DiskPercent, Comparison::Above, 10.0);
        r.enabled = false;
        assert!(evaluate(std::slice::from_ref(&r), |_| Some(99.0)).is_empty());
        r.enabled = true;
        assert_eq!(evaluate(&[r], |_| Some(99.0)).len(), 1);
    }

    /// Each rule sees the sample for *its own* metric.
    ///
    /// Without this a sampler that ignored its argument — returning one number
    /// for everything — would satisfy every other test here.
    #[test]
    fn each_rule_is_evaluated_against_its_own_metric() {
        let rules = vec![
            rule(1, Metric::DiskPercent, Comparison::Above, 50.0),
            rule(2, Metric::CpuTemperatureC, Comparison::Above, 50.0),
        ];
        let fired = evaluate(&rules, |m| match m {
            Metric::DiskPercent => Some(90.0),
            _ => Some(10.0),
        });
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].rule.id, 1);
        assert!((fired[0].value - 90.0).abs() < f64::EPSILON);
    }

    /// The alert an operator gets names the measurement, the reading and the
    /// threshold — so a person woken at 3 a.m. does not have to go and look up
    /// which rule fired.
    #[test]
    fn the_alert_body_carries_the_reading_and_the_threshold() {
        let fired = evaluate(
            &[rule(7, Metric::DiskPercent, Comparison::Above, 85.0)],
            |_| Some(91.5),
        );
        let f = &fired[0];
        assert_eq!(f.key(), "metric-rule:7");
        assert!(f.title().contains("rule 7"), "{}", f.title());
        let body = f.body();
        assert!(body.contains("91.5%"), "{body}");
        assert!(body.contains("85%"), "{body}");
        assert!(body.contains("above"), "{body}");
        assert!(body.contains("Disk in use"), "{body}");
    }

    /// Each episode key is the rule's, so two rules on the same metric are two
    /// episodes and one rule is never two.
    #[test]
    fn the_episode_key_is_the_rules_identity() {
        let fired = evaluate(
            &[
                rule(1, Metric::DiskPercent, Comparison::Above, 10.0),
                rule(2, Metric::DiskPercent, Comparison::Above, 20.0),
            ],
            |_| Some(90.0),
        );
        assert_eq!(fired.len(), 2);
        assert_ne!(fired[0].key(), fired[1].key());
    }

    /// A rule that could never stop firing is refused at the door.
    ///
    /// "Disk above -1" is not an alert, it is a notification that arrives for
    /// ever on every station, and it is an easy thing to type.
    #[test]
    fn a_rule_that_can_never_recover_is_refused() {
        let base = NewMetricRule {
            name: "always".into(),
            enabled: true,
            metric: Metric::DiskPercent,
            comparison: Comparison::Above,
            threshold: -1.0,
        };
        assert!(base.validate().is_err());

        let below_impossible = NewMetricRule {
            comparison: Comparison::Below,
            threshold: 101.0,
            ..base.clone()
        };
        assert!(below_impossible.validate().is_err());

        // The counterpart: an ordinary percentage rule is accepted, so this is
        // about impossibility and not about refusing percentages.
        let ordinary = NewMetricRule {
            threshold: 85.0,
            ..base.clone()
        };
        assert!(ordinary.validate().is_ok());

        // And the bound is not applied to metrics that are not percentages: a
        // detection count below 20 is exactly the rule that catches a dying
        // microphone.
        let count = NewMetricRule {
            metric: Metric::DetectionsPerHour,
            comparison: Comparison::Below,
            threshold: 20.0,
            ..base
        };
        assert!(count.validate().is_ok());
    }

    /// A rule with no name has no alert title.
    #[test]
    fn a_rule_needs_a_name() {
        let mut r = NewMetricRule {
            name: "   ".into(),
            enabled: true,
            metric: Metric::DiskPercent,
            comparison: Comparison::Above,
            threshold: 85.0,
        };
        assert!(r.validate().is_err());
        r.name = "x".repeat(MAX_NAME_LEN + 1);
        assert!(r.validate().is_err());
        r.name = "Disk filling".into();
        assert!(r.validate().is_ok());
    }

    /// Stored, listed, toggled and removed.
    #[test]
    fn a_rule_round_trips_through_the_database() {
        let conn = db();
        let id = insert(
            &conn,
            &NewMetricRule {
                name: "  Disk filling  ".into(),
                enabled: true,
                metric: Metric::DiskPercent,
                comparison: Comparison::Above,
                threshold: 85.0,
            },
        )
        .expect("insert");

        let rules = list(&conn).expect("list");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "Disk filling", "the name is trimmed");
        assert_eq!(rules[0].metric, Metric::DiskPercent);
        assert_eq!(rules[0].comparison, Comparison::Above);
        assert!((rules[0].threshold - 85.0).abs() < f64::EPSILON);
        assert!(rules[0].enabled);

        assert_eq!(toggle(&conn, id).expect("toggle"), Some(false));
        assert!(!list(&conn).expect("list")[0].enabled);
        assert_eq!(toggle(&conn, id).expect("toggle"), Some(true));

        assert!(delete(&conn, id).expect("delete"));
        assert!(list(&conn).expect("list").is_empty());
        assert!(!delete(&conn, id).expect("delete"), "already gone");
        assert_eq!(toggle(&conn, 9_999).expect("toggle"), None);
    }

    /// A row this version does not understand is skipped, not fatal.
    ///
    /// A rule written against a metric a later version removed must not take
    /// the whole page — and with it every *other* rule — down with it.
    #[test]
    fn an_unknown_metric_is_skipped_rather_than_fatal() {
        let conn = db();
        conn.execute(
            "INSERT INTO metric_rules (name, enabled, metric, comparison, threshold)
             VALUES ('from the future', 1, 'gravity_wells', 'above', 1.0)",
            [],
        )
        .expect("insert raw");
        insert(
            &conn,
            &NewMetricRule {
                name: "ordinary".into(),
                enabled: true,
                metric: Metric::DiskPercent,
                comparison: Comparison::Above,
                threshold: 85.0,
            },
        )
        .expect("insert");

        let rules = list(&conn).expect("list");
        assert_eq!(rules.len(), 1, "the unreadable row is skipped");
        assert_eq!(rules[0].name, "ordinary", "the readable one survives it");
    }

    /// Every metric has a distinct stored key, and every key parses back.
    ///
    /// The keys are in the database and in an operator's rules, so a duplicate
    /// would make two metrics indistinguishable once stored.
    #[test]
    fn the_metric_vocabulary_round_trips_and_is_unique() {
        let mut keys: Vec<&str> = Metric::ALL.iter().map(|m| m.key()).collect();
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "two metrics share a stored key");

        for m in Metric::ALL {
            assert_eq!(Metric::parse(m.key()), Some(m));
            assert!(!m.label().is_empty(), "{} has no label", m.key());
        }
        assert_eq!(Metric::parse("not_a_metric"), None);
    }

    /// Both comparisons round-trip, and nothing else parses.
    #[test]
    fn the_comparison_vocabulary_round_trips() {
        for c in [Comparison::Above, Comparison::Below] {
            assert_eq!(Comparison::parse(c.key()), Some(c));
        }
        assert_eq!(Comparison::parse("equals"), None);
        assert_eq!(Comparison::parse(""), None);
    }
}
