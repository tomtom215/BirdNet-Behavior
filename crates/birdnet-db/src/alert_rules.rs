//! Alert rules: conditional detection-triggered actions.
//!
//! An alert rule specifies:
//!
//! - **Conditions** — which detections trigger it (species pattern, confidence
//!   range, hour-of-day window, days of week).
//! - **Action** — what to do when triggered: fire a webhook, emit an extra
//!   structured log entry, or suppress all other notifications for this event.
//!
//! Rules are stored in the `alert_rules` `SQLite` table (migration v9) and are
//! evaluated in the detection event processor after each successful DB insert.
//!
//! # Example
//!
//! ```rust
//! use birdnet_db::alert_rules::{AlertRule, AlertAction, matches_rule};
//!
//! let rule = AlertRule {
//!     id: 1,
//!     name: "Rare owl webhook".into(),
//!     enabled: true,
//!     species_pattern: Some("Strix*".into()),
//!     confidence_min: 0.75,
//!     confidence_max: 1.0,
//!     hour_start: None,
//!     hour_end: None,
//!     days_of_week: None,
//!     action: AlertAction::Webhook {
//!         url: "https://example.com/hook".into(),
//!         method: "POST".into(),
//!         body_template: None,
//!         auth: None,
//!     },
//! };
//!
//! assert!(matches_rule(&rule, "Strix aluco", 0.90, 14, 3));
//! assert!(!matches_rule(&rule, "Parus major", 0.90, 14, 3));
//! ```

use rusqlite::{Connection, params};
use std::fmt;

// ---------------------------------------------------------------------------
// Portable form (export / import)
// ---------------------------------------------------------------------------

/// Placeholder written in place of a credential in a redacted export.
///
/// Importing a rule carrying this keeps every other field and drops the
/// credential, so a shared or backed-up rule set restores intact and the
/// operator is told exactly which rules need their secret re-entered.
pub const REDACTED: &str = "***REDACTED***";

/// Format version of an exported rule set.
///
/// Written so a future change of shape is detectable rather than being read as
/// a set of rules with every field missing.
pub const EXPORT_VERSION: u32 = 1;

/// One rule in its portable, flat form.
///
/// Deliberately flat rather than mirroring [`AlertAction`]'s enum: an exported
/// file is something an operator reads, hand-edits and pastes into a chat with
/// someone else, and a tagged union renders badly for all three.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RuleExport {
    /// Human-readable rule name.
    pub name: String,
    /// Whether the rule is active.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Glob-style species pattern; absent matches all species.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub species_pattern: Option<String>,
    /// Minimum confidence (inclusive).
    #[serde(default)]
    pub confidence_min: f64,
    /// Maximum confidence (inclusive).
    #[serde(default = "default_one")]
    pub confidence_max: f64,
    /// Hour-of-day window start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour_start: Option<u8>,
    /// Hour-of-day window end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour_end: Option<u8>,
    /// Comma-separated ISO weekday numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days_of_week: Option<String>,
    /// `"webhook"`, `"log"` or `"suppress"`.
    pub action: String,
    /// Webhook target URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
    /// Webhook HTTP method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_method: Option<String>,
    /// Webhook body template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_body: Option<String>,
    /// `"bearer"`, `"basic"` or `"header"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_auth_kind: Option<String>,
    /// The credential, or [`REDACTED`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_auth_value: Option<String>,
    /// Header name, for the `header` scheme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_header_name: Option<String>,
}

/// serde default for [`RuleExport::enabled`].
const fn default_true() -> bool {
    true
}

/// serde default for [`RuleExport::confidence_max`].
const fn default_one() -> f64 {
    1.0
}

/// An exported rule set, with its format version.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RuleSet {
    /// [`EXPORT_VERSION`] at the time of writing.
    pub version: u32,
    /// Whether credentials were replaced with [`REDACTED`].
    pub redacted: bool,
    /// The rules.
    pub rules: Vec<RuleExport>,
}

/// Convert a stored rule to its portable form.
///
/// `include_secrets` false — the default everywhere it is offered — replaces
/// each credential with [`REDACTED`] while keeping its scheme and header name,
/// so the reader can see *that* a rule authenticates and how, without the
/// export being a file that must be handled as a secret.
#[must_use]
pub fn to_export(rule: &AlertRule, include_secrets: bool) -> RuleExport {
    let mut out = RuleExport {
        name: rule.name.clone(),
        enabled: rule.enabled,
        species_pattern: rule.species_pattern.clone(),
        confidence_min: rule.confidence_min,
        confidence_max: rule.confidence_max,
        hour_start: rule.hour_start,
        hour_end: rule.hour_end,
        days_of_week: rule.days_of_week.clone(),
        action: rule.action.type_str().to_string(),
        webhook_url: None,
        webhook_method: None,
        webhook_body: None,
        webhook_auth_kind: None,
        webhook_auth_value: None,
        webhook_header_name: None,
    };
    if let AlertAction::Webhook {
        url,
        method,
        body_template,
        auth,
    } = &rule.action
    {
        out.webhook_url = Some(url.clone());
        out.webhook_method = Some(method.clone());
        out.webhook_body.clone_from(body_template);
        if let Some(auth) = auth {
            let (value, header_name) = auth.to_columns();
            out.webhook_auth_kind = Some(auth.kind_str().to_string());
            out.webhook_header_name = header_name;
            out.webhook_auth_value = Some(if include_secrets {
                value
            } else {
                REDACTED.to_string()
            });
        }
    }
    out
}

/// Why one entry of an imported rule set could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportRejection {
    /// The rule has no name.
    NoName,
    /// `action` was `"webhook"` with no URL.
    WebhookWithoutUrl(String),
    /// `action` named something other than webhook, log or suppress.
    UnknownAction {
        /// The rule's name.
        name: String,
        /// The action string that was not recognised.
        action: String,
    },
}

impl fmt::Display for ImportRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoName => write!(f, "a rule has no name"),
            Self::WebhookWithoutUrl(name) => {
                write!(f, "rule {name:?} is a webhook with no URL")
            }
            Self::UnknownAction { name, action } => {
                write!(f, "rule {name:?} has unknown action {action:?}")
            }
        }
    }
}

/// One imported rule, ready to insert.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The rule.
    pub rule: NewAlertRule,
    /// Whether its credential arrived redacted and was therefore dropped.
    pub credential_was_redacted: bool,
}

/// Convert a portable rule back into an insertable one.
///
/// # Errors
///
/// Returns [`ImportRejection`] if the entry cannot describe a usable rule.
pub fn from_export(e: &RuleExport) -> Result<Imported, ImportRejection> {
    let name = e.name.trim();
    if name.is_empty() {
        return Err(ImportRejection::NoName);
    }

    let mut credential_was_redacted = false;
    let action = match e.action.as_str() {
        "log" => AlertAction::Log,
        "suppress" => AlertAction::Suppress,
        "webhook" => {
            let url = e
                .webhook_url
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .ok_or_else(|| ImportRejection::WebhookWithoutUrl(name.to_string()))?;
            let auth = match e.webhook_auth_value.as_deref() {
                Some(REDACTED) => {
                    credential_was_redacted = true;
                    None
                }
                value => WebhookAuth::from_columns(
                    e.webhook_auth_kind.as_deref().unwrap_or_default(),
                    value,
                    e.webhook_header_name.as_deref(),
                ),
            };
            AlertAction::Webhook {
                url: url.to_string(),
                method: e
                    .webhook_method
                    .clone()
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| "POST".to_string()),
                body_template: e.webhook_body.clone().filter(|b| !b.trim().is_empty()),
                auth,
            }
        }
        other => {
            return Err(ImportRejection::UnknownAction {
                name: name.to_string(),
                action: other.to_string(),
            });
        }
    };

    Ok(Imported {
        rule: NewAlertRule {
            name: name.to_string(),
            enabled: e.enabled,
            species_pattern: e.species_pattern.clone().filter(|p| !p.trim().is_empty()),
            confidence_min: e.confidence_min.clamp(0.0, 1.0),
            confidence_max: e.confidence_max.clamp(0.0, 1.0),
            hour_start: e.hour_start.filter(|h| *h <= 23),
            hour_end: e.hour_end.filter(|h| *h <= 23),
            days_of_week: e.days_of_week.clone().filter(|d| !d.trim().is_empty()),
            action,
        },
        credential_was_redacted,
    })
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from alert-rule operations.
#[derive(Debug)]
pub enum AlertRuleError {
    /// `SQLite` error.
    Sqlite(rusqlite::Error),
    /// Data serialization/validation error.
    Data(String),
}

impl fmt::Display for AlertRuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "alert_rules db error: {e}"),
            Self::Data(msg) => write!(f, "alert_rules data error: {msg}"),
        }
    }
}

impl std::error::Error for AlertRuleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Data(_) => None,
        }
    }
}

impl From<rusqlite::Error> for AlertRuleError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// How an alert-rule webhook authenticates.
///
/// `Debug` is written by hand: this type is embedded in [`AlertAction`], which
/// is `Debug`-formatted into logs and error messages, and a derived
/// implementation would print the credential in every one of them.
#[derive(Clone, PartialEq, Eq)]
pub enum WebhookAuth {
    /// `Authorization: Bearer <token>`.
    Bearer(String),
    /// `Authorization: Basic base64(user:password)`.
    Basic {
        /// Username.
        user: String,
        /// Password.
        password: String,
    },
    /// A named header, for services with their own scheme
    /// (`X-API-Key`, `X-Webhook-Token`, …).
    Header {
        /// Header name. Not a secret.
        name: String,
        /// Header value.
        value: String,
    },
}

impl fmt::Debug for WebhookAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bearer(_) => write!(f, "Bearer(<redacted>)"),
            Self::Basic { user, .. } => {
                write!(f, "Basic {{ user: {user:?}, password: <redacted> }}")
            }
            Self::Header { name, .. } => {
                write!(f, "Header {{ name: {name:?}, value: <redacted> }}")
            }
        }
    }
}

impl WebhookAuth {
    /// The `action_webhook_auth_kind` column value.
    #[must_use]
    pub const fn kind_str(&self) -> &'static str {
        match self {
            Self::Bearer(_) => "bearer",
            Self::Basic { .. } => "basic",
            Self::Header { .. } => "header",
        }
    }

    /// Rebuild from the three stored columns.
    ///
    /// Returns `None` for an unknown kind or a missing value, which is what a
    /// row written by a newer binary and read by an older one looks like: the
    /// rule then fires unauthenticated rather than not at all.
    #[must_use]
    pub fn from_columns(
        kind: &str,
        value: Option<&str>,
        header_name: Option<&str>,
    ) -> Option<Self> {
        let value = value?;
        match kind {
            "bearer" => Some(Self::Bearer(value.to_string())),
            "basic" => {
                let (user, password) = value.split_once(':')?;
                Some(Self::Basic {
                    user: user.to_string(),
                    password: password.to_string(),
                })
            }
            "header" => Some(Self::Header {
                name: header_name.filter(|n| !n.is_empty())?.to_string(),
                value: value.to_string(),
            }),
            _ => None,
        }
    }

    /// The `action_webhook_auth_value` and `action_webhook_header_name`
    /// column values.
    #[must_use]
    pub fn to_columns(&self) -> (String, Option<String>) {
        match self {
            Self::Bearer(t) => (t.clone(), None),
            Self::Basic { user, password } => (format!("{user}:{password}"), None),
            Self::Header { name, value } => (value.clone(), Some(name.clone())),
        }
    }
}

/// The action executed when a rule's conditions are met.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertAction {
    /// Send an HTTP request to a webhook URL.
    Webhook {
        /// Target URL.
        url: String,
        /// HTTP method (`"POST"` or `"GET"`).
        method: String,
        /// Optional body template. Placeholders: `{{species}}`, `{{sci_name}}`,
        /// `{{confidence}}`, `{{date}}`, `{{time}}`.
        body_template: Option<String>,
        /// Optional credential. `None` sends the request unauthenticated,
        /// which is what every rule written before this existed does.
        auth: Option<WebhookAuth>,
    },
    /// Emit a structured log entry at `INFO` level.
    Log,
    /// Suppress all other notifications (Apprise, email, MQTT) for this event.
    Suppress,
}

impl AlertAction {
    /// Serialise to the `action_type` column value.
    #[must_use]
    pub const fn type_str(&self) -> &'static str {
        match self {
            Self::Webhook { .. } => "webhook",
            Self::Log => "log",
            Self::Suppress => "suppress",
        }
    }
}

/// A single alert rule loaded from the database.
#[derive(Debug, Clone)]
pub struct AlertRule {
    /// Row ID.
    pub id: i64,
    /// Human-readable rule name.
    pub name: String,
    /// Whether the rule is active.
    pub enabled: bool,
    /// Optional glob-style species pattern (`*` matches any substring).
    /// `None` matches all species.
    pub species_pattern: Option<String>,
    /// Minimum confidence (inclusive, 0.0–1.0).
    pub confidence_min: f64,
    /// Maximum confidence (inclusive, 0.0–1.0).
    pub confidence_max: f64,
    /// Hour-of-day window start (0–23). `None` = any hour.
    pub hour_start: Option<u8>,
    /// Hour-of-day window end (0–23). `None` = any hour.
    pub hour_end: Option<u8>,
    /// Comma-separated ISO weekday numbers (1=Mon … 7=Sun). `None` = any day.
    pub days_of_week: Option<String>,
    /// Action to execute.
    pub action: AlertAction,
}

/// Lightweight struct for inserting a new rule.
// `PartialEq` so an import round trip can be asserted whole rather than field
// by field; not `Eq`, because the confidence bounds are `f64`.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, PartialEq)]
pub struct NewAlertRule {
    /// Human-readable rule name.
    pub name: String,
    /// Whether the rule starts enabled.
    pub enabled: bool,
    /// Optional glob pattern for species common name (e.g. `"Barn*"`).
    pub species_pattern: Option<String>,
    /// Minimum confidence threshold (0.0–1.0).
    pub confidence_min: f64,
    /// Maximum confidence threshold (0.0–1.0).
    pub confidence_max: f64,
    /// Hour-of-day start (0–23), inclusive.
    pub hour_start: Option<u8>,
    /// Hour-of-day end (0–23), inclusive.
    pub hour_end: Option<u8>,
    /// Comma-separated weekdays (1–7). `None` = every day.
    pub days_of_week: Option<String>,
    /// Action to execute.
    pub action: AlertAction,
}

// ---------------------------------------------------------------------------
// Glob matching
// ---------------------------------------------------------------------------

/// Simple glob match: `*` matches any number of characters.
///
/// Case-insensitive comparison.
#[must_use]
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pat = pattern.to_lowercase();
    let text_lc = text.to_lowercase();
    glob_match_inner(pat.as_bytes(), text_lc.as_bytes())
}

fn glob_match_inner(pattern: &[u8], text: &[u8]) -> bool {
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            // Try consuming zero characters, or one character from text
            glob_match_inner(&pattern[1..], text)
                || (!text.is_empty() && glob_match_inner(pattern, &text[1..]))
        }
        (Some(&pc), Some(&tc)) if pc == tc => glob_match_inner(&pattern[1..], &text[1..]),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Condition evaluation
// ---------------------------------------------------------------------------

/// Returns `true` if `rule` matches the given detection attributes.
///
/// # Parameters
///
/// - `rule` — rule to test
/// - `common_name` — detection common name
/// - `confidence` — detection confidence (0.0–1.0)
/// - `hour` — hour-of-day (0–23, UTC or local depending on caller)
/// - `weekday` — ISO weekday (1=Mon … 7=Sun)
#[must_use]
pub fn matches_rule(
    rule: &AlertRule,
    common_name: &str,
    confidence: f64,
    hour: u8,
    weekday: u8,
) -> bool {
    if !rule.enabled {
        return false;
    }

    // Species pattern
    if let Some(ref pattern) = rule.species_pattern
        && !glob_match(pattern, common_name)
    {
        return false;
    }

    // Confidence range
    if confidence < rule.confidence_min || confidence > rule.confidence_max {
        return false;
    }

    // Hour window
    if let (Some(start), Some(end)) = (rule.hour_start, rule.hour_end) {
        let in_window = if start <= end {
            hour >= start && hour <= end
        } else {
            // Wraps midnight (e.g. 22–05)
            hour >= start || hour <= end
        };
        if !in_window {
            return false;
        }
    }

    // Day of week
    if let Some(ref days) = rule.days_of_week {
        let wd_str = weekday.to_string();
        let matched = days.split(',').any(|d| d.trim() == wd_str);
        if !matched {
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Webhook body rendering
// ---------------------------------------------------------------------------

/// Render a webhook body template with detection values substituted.
///
/// Recognised placeholders: `{{species}}`, `{{sci_name}}`, `{{confidence}}`,
/// `{{date}}`, `{{time}}`.
#[must_use]
pub fn render_webhook_body(
    template: &str,
    common_name: &str,
    sci_name: &str,
    confidence: f64,
    date: &str,
    time: &str,
) -> String {
    template
        .replace("{{species}}", common_name)
        .replace("{{sci_name}}", sci_name)
        .replace("{{confidence}}", &format!("{confidence:.4}"))
        .replace("{{date}}", date)
        .replace("{{time}}", time)
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

/// List all alert rules ordered by id.
///
/// # Errors
///
/// Returns `AlertRuleError` on query failure.
pub fn list_rules(conn: &Connection) -> Result<Vec<AlertRule>, AlertRuleError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, enabled, species_pattern, confidence_min, confidence_max,
                hour_start, hour_end, days_of_week,
                action_type, action_webhook_url, action_webhook_method, action_webhook_body,
                action_webhook_auth_kind, action_webhook_auth_value,
                action_webhook_header_name
         FROM alert_rules ORDER BY id",
    )?;

    let rules = stmt
        .query_map([], |row| {
            let action_type: String = row.get(9)?;
            let webhook_url: Option<String> = row.get(10)?;
            let webhook_method: String = row.get(11)?;
            let webhook_body: Option<String> = row.get(12)?;
            let auth_kind: String = row.get(13)?;
            let auth_value: Option<String> = row.get(14)?;
            let header_name: Option<String> = row.get(15)?;

            let action = match action_type.as_str() {
                "webhook" => AlertAction::Webhook {
                    url: webhook_url.unwrap_or_default(),
                    method: webhook_method,
                    body_template: webhook_body,
                    auth: WebhookAuth::from_columns(
                        &auth_kind,
                        auth_value.as_deref(),
                        header_name.as_deref(),
                    ),
                },
                "suppress" => AlertAction::Suppress,
                _ => AlertAction::Log,
            };

            Ok(AlertRule {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                species_pattern: row.get(3)?,
                confidence_min: row.get(4)?,
                confidence_max: row.get(5)?,
                hour_start: row
                    .get::<_, Option<i64>>(6)?
                    .map(|v| u8::try_from(v.clamp(0, 23)).unwrap_or(0)),
                hour_end: row
                    .get::<_, Option<i64>>(7)?
                    .map(|v| u8::try_from(v.clamp(0, 23)).unwrap_or(0)),
                days_of_week: row.get(8)?,
                action,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rules)
}

/// Fetch a single rule by id.
///
/// Returns `None` if no rule with that id exists.
///
/// # Errors
///
/// Returns `AlertRuleError` on query failure.
pub fn get_rule(conn: &Connection, id: i64) -> Result<Option<AlertRule>, AlertRuleError> {
    let rules = list_rules(conn)?;
    Ok(rules.into_iter().find(|r| r.id == id))
}

/// Insert a new alert rule and return its assigned `id`.
///
/// # Errors
///
/// Returns `AlertRuleError` on constraint or DB failure.
pub fn insert_rule(conn: &Connection, rule: &NewAlertRule) -> Result<i64, AlertRuleError> {
    let (url, method, body, auth) = match &rule.action {
        AlertAction::Webhook {
            url,
            method,
            body_template,
            auth,
        } => (
            Some(url.as_str()),
            method.as_str(),
            body_template.as_deref(),
            auth.as_ref(),
        ),
        _ => (None, "POST", None, None),
    };
    let auth_kind = auth.map_or("", WebhookAuth::kind_str);
    let (auth_value, header_name) = auth.map_or((None, None), |a| {
        let (v, n) = a.to_columns();
        (Some(v), n)
    });

    conn.execute(
        "INSERT INTO alert_rules
             (name, enabled, species_pattern, confidence_min, confidence_max,
              hour_start, hour_end, days_of_week,
              action_type, action_webhook_url, action_webhook_method, action_webhook_body,
              action_webhook_auth_kind, action_webhook_auth_value,
              action_webhook_header_name)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            rule.name,
            i64::from(rule.enabled),
            rule.species_pattern,
            rule.confidence_min,
            rule.confidence_max,
            rule.hour_start.map(i64::from),
            rule.hour_end.map(i64::from),
            rule.days_of_week,
            rule.action.type_str(),
            url,
            method,
            body,
            auth_kind,
            auth_value,
            header_name,
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Delete a rule by id.
///
/// # Errors
///
/// Returns `AlertRuleError` on DB failure.
pub fn delete_rule(conn: &Connection, id: i64) -> Result<bool, AlertRuleError> {
    let deleted = conn.execute("DELETE FROM alert_rules WHERE id = ?1", params![id])?;
    Ok(deleted > 0)
}

/// Toggle the `enabled` flag of a rule.
///
/// Returns the new state, or `None` if the rule was not found.
///
/// # Errors
///
/// Returns `AlertRuleError` on DB failure.
pub fn toggle_rule(conn: &Connection, id: i64) -> Result<Option<bool>, AlertRuleError> {
    let updated = conn.execute(
        "UPDATE alert_rules
         SET enabled = CASE WHEN enabled = 1 THEN 0 ELSE 1 END,
             updated_at = datetime('now')
         WHERE id = ?1",
        params![id],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    let enabled: i64 = conn.query_row(
        "SELECT enabled FROM alert_rules WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    Ok(Some(enabled != 0))
}

/// Evaluate all enabled rules against a detection and return the matching ones.
///
/// The caller should load rules once at startup (or re-load on change) and
/// pass them here to avoid repeated DB queries.
///
/// # Parameters
///
/// - `rules` — slice of all rules (loaded via [`list_rules`])
/// - `common_name` — detection common name
/// - `confidence` — detection confidence (0.0–1.0)
/// - `detection_time` — `"HH:MM:SS"` string from the detection
pub fn evaluate_rules<'a>(
    rules: &'a [AlertRule],
    common_name: &str,
    confidence: f64,
    detection_time: &str,
) -> Vec<&'a AlertRule> {
    let hour = parse_hour(detection_time);
    let weekday = current_weekday();
    rules
        .iter()
        .filter(|r| matches_rule(r, common_name, confidence, hour, weekday))
        .collect()
}

/// Parse the hour component from `"HH:MM:SS"`. Returns 0 on any parse error.
fn parse_hour(time_str: &str) -> u8 {
    time_str
        .split(':')
        .next()
        .and_then(|h| h.parse::<u8>().ok())
        .unwrap_or(0)
}

/// Return the current ISO weekday (1=Mon … 7=Sun) using UTC time.
fn current_weekday() -> u8 {
    // Use a simple calculation based on the Unix timestamp.
    // 1970-01-01 was a Thursday = day 4.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = secs / 86400;
    // (days + 3) % 7 gives 0=Mon … 6=Sun → add 1 → 1=Mon … 7=Sun
    u8::try_from((days + 3) % 7 + 1).unwrap_or(1)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        conn
    }

    fn webhook_rule(pattern: Option<&str>, conf_min: f64) -> NewAlertRule {
        NewAlertRule {
            name: "test-rule".into(),
            enabled: true,
            species_pattern: pattern.map(String::from),
            confidence_min: conf_min,
            confidence_max: 1.0,
            hour_start: None,
            hour_end: None,
            days_of_week: None,
            action: AlertAction::Webhook {
                url: "https://example.com/hook".into(),
                method: "POST".into(),
                body_template: None,
                auth: None,
            },
        }
    }

    // --- glob_match ---

    #[test]
    fn glob_exact_match() {
        assert!(glob_match("Barn Owl", "Barn Owl"));
    }

    #[test]
    fn glob_wildcard_prefix() {
        assert!(glob_match("*Owl", "Barn Owl"));
        assert!(glob_match("*Owl", "Snowy Owl"));
        assert!(!glob_match("*Owl", "Barn Swallow"));
    }

    #[test]
    fn glob_wildcard_suffix() {
        assert!(glob_match("Barn*", "Barn Owl"));
        assert!(glob_match("Barn*", "Barn Swallow"));
        assert!(!glob_match("Barn*", "European Robin"));
    }

    #[test]
    fn glob_wildcard_middle() {
        assert!(glob_match("E*Robin", "European Robin"));
        assert!(!glob_match("E*Robin", "American Robin"));
    }

    #[test]
    fn glob_case_insensitive() {
        assert!(glob_match("barn*", "Barn Owl"));
        assert!(glob_match("BARN*", "Barn Owl"));
    }

    #[test]
    fn glob_star_matches_all() {
        assert!(glob_match("*", "Any Species"));
    }

    // --- matches_rule ---

    #[test]
    fn rule_matches_any_species_when_no_pattern() {
        let rule = AlertRule {
            id: 1,
            name: "all".into(),
            enabled: true,
            species_pattern: None,
            confidence_min: 0.5,
            confidence_max: 1.0,
            hour_start: None,
            hour_end: None,
            days_of_week: None,
            action: AlertAction::Log,
        };
        assert!(matches_rule(&rule, "Any Bird", 0.8, 12, 3));
    }

    #[test]
    fn rule_rejects_below_confidence() {
        let rule = AlertRule {
            id: 1,
            name: "high-conf".into(),
            enabled: true,
            species_pattern: None,
            confidence_min: 0.8,
            confidence_max: 1.0,
            hour_start: None,
            hour_end: None,
            days_of_week: None,
            action: AlertAction::Suppress,
        };
        assert!(!matches_rule(&rule, "Any Bird", 0.7, 12, 3));
        assert!(matches_rule(&rule, "Any Bird", 0.85, 12, 3));
    }

    #[test]
    fn rule_hour_window_normal() {
        let rule = AlertRule {
            id: 1,
            name: "dawn".into(),
            enabled: true,
            species_pattern: None,
            confidence_min: 0.0,
            confidence_max: 1.0,
            hour_start: Some(5),
            hour_end: Some(9),
            days_of_week: None,
            action: AlertAction::Log,
        };
        assert!(matches_rule(&rule, "X", 0.5, 6, 1));
        assert!(!matches_rule(&rule, "X", 0.5, 10, 1));
        assert!(!matches_rule(&rule, "X", 0.5, 4, 1));
    }

    #[test]
    fn rule_hour_window_wraps_midnight() {
        let rule = AlertRule {
            id: 1,
            name: "night".into(),
            enabled: true,
            species_pattern: None,
            confidence_min: 0.0,
            confidence_max: 1.0,
            hour_start: Some(22),
            hour_end: Some(4),
            days_of_week: None,
            action: AlertAction::Log,
        };
        assert!(matches_rule(&rule, "X", 0.5, 23, 1));
        assert!(matches_rule(&rule, "X", 0.5, 2, 1));
        assert!(!matches_rule(&rule, "X", 0.5, 12, 1));
    }

    #[test]
    fn rule_days_of_week_filter() {
        let rule = AlertRule {
            id: 1,
            name: "weekdays".into(),
            enabled: true,
            species_pattern: None,
            confidence_min: 0.0,
            confidence_max: 1.0,
            hour_start: None,
            hour_end: None,
            days_of_week: Some("1,2,3,4,5".into()),
            action: AlertAction::Log,
        };
        assert!(matches_rule(&rule, "X", 0.5, 12, 1)); // Monday
        assert!(!matches_rule(&rule, "X", 0.5, 12, 6)); // Saturday
    }

    #[test]
    fn disabled_rule_never_matches() {
        let rule = AlertRule {
            id: 1,
            name: "disabled".into(),
            enabled: false,
            species_pattern: None,
            confidence_min: 0.0,
            confidence_max: 1.0,
            hour_start: None,
            hour_end: None,
            days_of_week: None,
            action: AlertAction::Log,
        };
        assert!(!matches_rule(&rule, "Any Bird", 0.9, 12, 3));
    }

    // --- CRUD ---

    #[test]
    fn insert_and_list_rules() {
        let conn = memory_db();
        let id = insert_rule(&conn, &webhook_rule(Some("Barn Owl"), 0.8)).unwrap();
        assert!(id > 0);
        let rules = list_rules(&conn).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "test-rule");
    }

    #[test]
    fn delete_rule_removes_it() {
        let conn = memory_db();
        let id = insert_rule(&conn, &webhook_rule(None, 0.0)).unwrap();
        assert!(delete_rule(&conn, id).unwrap());
        assert!(list_rules(&conn).unwrap().is_empty());
    }

    #[test]
    fn toggle_rule_flips_enabled() {
        let conn = memory_db();
        let id = insert_rule(&conn, &webhook_rule(None, 0.0)).unwrap();
        let new_state = toggle_rule(&conn, id).unwrap();
        assert_eq!(new_state, Some(false));
        let new_state2 = toggle_rule(&conn, id).unwrap();
        assert_eq!(new_state2, Some(true));
    }

    #[test]
    fn get_rule_returns_correct_row() {
        let conn = memory_db();
        let _id1 = insert_rule(&conn, &webhook_rule(Some("Owl*"), 0.5)).unwrap();
        let id2 = insert_rule(&conn, &webhook_rule(Some("Robin*"), 0.3)).unwrap();
        let rule = get_rule(&conn, id2).unwrap().expect("should exist");
        assert_eq!(rule.species_pattern.as_deref(), Some("Robin*"));
    }

    #[test]
    fn evaluate_rules_returns_matching_only() {
        let conn = memory_db();
        insert_rule(&conn, &webhook_rule(Some("Barn Owl"), 0.7)).unwrap();
        insert_rule(&conn, &webhook_rule(Some("Robin*"), 0.3)).unwrap();
        let rules = list_rules(&conn).unwrap();
        let matched = evaluate_rules(&rules, "Barn Owl", 0.9, "14:30:00");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].species_pattern.as_deref(), Some("Barn Owl"));
    }

    // --- render_webhook_body ---

    #[test]
    fn webhook_body_substitution() {
        let tmpl = r#"{"bird":"{{species}}","conf":{{confidence}}}"#;
        let out = render_webhook_body(
            tmpl,
            "Barn Owl",
            "Tyto alba",
            0.923_4,
            "2026-03-23",
            "06:15:00",
        );
        assert!(out.contains("Barn Owl"));
        assert!(out.contains("0.9234"));
    }

    // ── webhook authentication (migration 35) ───────────────────────────

    /// A webhook rule carrying `auth`.
    fn authed_rule(name: &str, auth: Option<WebhookAuth>) -> NewAlertRule {
        NewAlertRule {
            name: name.to_owned(),
            enabled: true,
            species_pattern: None,
            confidence_min: 0.0,
            confidence_max: 1.0,
            hour_start: None,
            hour_end: None,
            days_of_week: None,
            action: AlertAction::Webhook {
                url: "https://ha.lan/api/webhook/abc".into(),
                method: "POST".into(),
                body_template: None,
                auth,
            },
        }
    }

    /// The action stored for `name`.
    fn stored_action(conn: &Connection, name: &str) -> AlertAction {
        list_rules(conn)
            .unwrap()
            .into_iter()
            .find(|r| r.name == name)
            .expect("rule was inserted")
            .action
    }

    #[test]
    fn every_auth_scheme_survives_a_round_trip() {
        let conn = memory_db();
        let cases = [
            ("bearer", WebhookAuth::Bearer("tok_ABC123".into())),
            (
                "basic",
                WebhookAuth::Basic {
                    user: "ada".into(),
                    password: "hunter2".into(),
                },
            ),
            (
                "header",
                WebhookAuth::Header {
                    name: "X-API-Key".into(),
                    value: "k_XYZ".into(),
                },
            ),
        ];
        for (name, auth) in cases {
            insert_rule(&conn, &authed_rule(name, Some(auth.clone()))).unwrap();
            let AlertAction::Webhook { auth: stored, .. } = stored_action(&conn, name) else {
                panic!("{name} did not come back as a webhook");
            };
            assert_eq!(stored, Some(auth), "{name} did not survive the round trip");
        }
    }

    #[test]
    fn a_basic_password_containing_a_colon_survives() {
        // `user:password` is packed into one column, so the split must be on
        // the *first* colon. A generated password with a colon in it would
        // otherwise come back truncated and fail to authenticate — which the
        // operator would see only as a 401 in the log.
        let conn = memory_db();
        insert_rule(
            &conn,
            &authed_rule(
                "colon",
                Some(WebhookAuth::Basic {
                    user: "ada".into(),
                    password: "a:b:c".into(),
                }),
            ),
        )
        .unwrap();
        let AlertAction::Webhook { auth, .. } = stored_action(&conn, "colon") else {
            panic!("not a webhook");
        };
        assert_eq!(
            auth,
            Some(WebhookAuth::Basic {
                user: "ada".into(),
                password: "a:b:c".into(),
            })
        );
    }

    #[test]
    fn a_rule_without_auth_still_stores_and_loads_as_unauthenticated() {
        // The no-op-upgrade guarantee: every rule written before migration 35
        // must keep sending exactly the request it sent before.
        let conn = memory_db();
        insert_rule(&conn, &authed_rule("plain", None)).unwrap();
        let AlertAction::Webhook { auth, .. } = stored_action(&conn, "plain") else {
            panic!("not a webhook");
        };
        assert_eq!(auth, None);
    }

    #[test]
    fn a_row_with_a_kind_but_no_value_loads_as_unauthenticated() {
        // What a row written by a newer binary looks like to an older one, and
        // what a hand-edited database can hold. Firing the rule without the
        // credential is better than the alternatives: refusing to load the
        // rule would silently disable an operator's alerting.
        let conn = memory_db();
        insert_rule(&conn, &authed_rule("partial", None)).unwrap();
        conn.execute(
            "UPDATE alert_rules SET action_webhook_auth_kind = 'bearer' WHERE name = 'partial'",
            [],
        )
        .unwrap();
        let AlertAction::Webhook { auth, .. } = stored_action(&conn, "partial") else {
            panic!("not a webhook");
        };
        assert_eq!(auth, None);

        // ...and an unknown scheme name does the same rather than panicking.
        conn.execute(
            "UPDATE alert_rules
                SET action_webhook_auth_kind = 'oauth3', action_webhook_auth_value = 'x'
              WHERE name = 'partial'",
            [],
        )
        .unwrap();
        let AlertAction::Webhook { auth, .. } = stored_action(&conn, "partial") else {
            panic!("not a webhook");
        };
        assert_eq!(auth, None);
    }

    #[test]
    fn debugging_an_alert_rule_never_prints_its_credential() {
        // `AlertRule` is `Debug`-formatted into log lines and error messages,
        // and the rules list is dumped wholesale in more than one diagnostic.
        // A derived `Debug` on `WebhookAuth` would put a live API key in every
        // one of them, and into any support bundle taken afterwards.
        let secrets = [
            WebhookAuth::Bearer("SUPERSECRETTOKEN".into()),
            WebhookAuth::Basic {
                user: "ada".into(),
                password: "SUPERSECRETPASSWORD".into(),
            },
            WebhookAuth::Header {
                name: "X-API-Key".into(),
                value: "SUPERSECRETKEY".into(),
            },
        ];
        for auth in secrets {
            let rule = authed_rule("dbg", Some(auth.clone()));
            let rendered = format!("{:?} {:#?}", rule.action, rule.action);
            for needle in ["SUPERSECRETTOKEN", "SUPERSECRETPASSWORD", "SUPERSECRETKEY"] {
                assert!(
                    !rendered.contains(needle),
                    "{auth:?} leaked its credential: {rendered}"
                );
            }
            // ...while still saying enough to debug with.
            assert!(rendered.contains("redacted"), "{rendered}");
        }
    }

    #[test]
    fn the_non_secret_half_of_a_credential_is_still_visible() {
        // Counterpart: a `Debug` that printed nothing at all would satisfy the
        // gate above and leave an operator unable to tell which scheme or
        // which header a failing rule was using.
        let basic = WebhookAuth::Basic {
            user: "ada".into(),
            password: "hunter2".into(),
        };
        assert!(format!("{basic:?}").contains("ada"));
        let header = WebhookAuth::Header {
            name: "X-API-Key".into(),
            value: "k".into(),
        };
        let rendered = format!("{header:?}");
        assert!(rendered.contains("X-API-Key"), "{rendered}");
    }

    // ── export / import ─────────────────────────────────────────────────

    /// A rule with every field set to something distinguishable.
    fn full_rule(auth: Option<WebhookAuth>) -> AlertRule {
        AlertRule {
            id: 7,
            name: "Owls at night".into(),
            enabled: false,
            species_pattern: Some("Strix*".into()),
            confidence_min: 0.65,
            confidence_max: 0.95,
            hour_start: Some(22),
            hour_end: Some(4),
            days_of_week: Some("1,2,3".into()),
            action: AlertAction::Webhook {
                url: "https://ha.lan/api/webhook/xyz".into(),
                method: "GET".into(),
                body_template: Some("{\"b\":\"{{species}}\"}".into()),
                auth,
            },
        }
    }

    #[test]
    fn an_export_with_secrets_round_trips_every_field() {
        // The backup-and-restore case. A field silently lost here is a rule
        // that looks right in the list and matches something different.
        let original = full_rule(Some(WebhookAuth::Bearer("tok_ABC".into())));
        let imported = from_export(&to_export(&original, true)).expect("imports");
        assert!(!imported.credential_was_redacted);

        let restored = imported.rule;
        assert_eq!(restored.name, original.name);
        assert_eq!(restored.enabled, original.enabled);
        assert_eq!(restored.species_pattern, original.species_pattern);
        assert!((restored.confidence_min - original.confidence_min).abs() < f64::EPSILON);
        assert!((restored.confidence_max - original.confidence_max).abs() < f64::EPSILON);
        assert_eq!(restored.hour_start, original.hour_start);
        assert_eq!(restored.hour_end, original.hour_end);
        assert_eq!(restored.days_of_week, original.days_of_week);
        assert_eq!(restored.action, original.action);
    }

    #[test]
    fn a_redacted_export_carries_no_credential_but_still_says_there_was_one() {
        // The share-when-asking-for-help case: the reader can see that the
        // rule authenticates, and how, without the file being a secret.
        let e = to_export(
            &full_rule(Some(WebhookAuth::Bearer("tok_ABC".into()))),
            false,
        );
        assert_eq!(e.webhook_auth_kind.as_deref(), Some("bearer"));
        assert_eq!(e.webhook_auth_value.as_deref(), Some(REDACTED));
        let rendered = format!("{e:?}");
        assert!(!rendered.contains("tok_ABC"), "{rendered}");
    }

    #[test]
    fn a_redacted_header_export_keeps_the_header_name_and_drops_the_value() {
        // Counterpart on the other half of the pair: the header *name* is not
        // a secret and is the part that makes the export intelligible.
        let e = to_export(
            &full_rule(Some(WebhookAuth::Header {
                name: "X-API-Key".into(),
                value: "k_SECRET".into(),
            })),
            false,
        );
        assert_eq!(e.webhook_header_name.as_deref(), Some("X-API-Key"));
        assert_eq!(e.webhook_auth_value.as_deref(), Some(REDACTED));
        assert!(!format!("{e:?}").contains("k_SECRET"));
    }

    #[test]
    fn importing_a_redacted_rule_keeps_it_and_reports_the_missing_credential() {
        // Dropping the whole rule would lose the conditions and the URL, which
        // are the laborious part; importing it silently authenticated with the
        // literal "***REDACTED***" would be worse still.
        let imported = from_export(&to_export(
            &full_rule(Some(WebhookAuth::Bearer("t".into()))),
            false,
        ))
        .expect("imports");
        assert!(imported.credential_was_redacted, "the caller is not told");
        let AlertAction::Webhook { auth, url, .. } = &imported.rule.action else {
            panic!("not a webhook");
        };
        assert_eq!(*auth, None, "the placeholder was imported as a credential");
        assert_eq!(url, "https://ha.lan/api/webhook/xyz", "the URL was lost");
    }

    #[test]
    fn a_rule_that_never_had_a_credential_is_not_reported_as_needing_one() {
        // Counterpart: a flag that was always true would make the import
        // warning meaningless, and every log-action rule would claim to need
        // a credential.
        for auth in [None, Some(WebhookAuth::Bearer("t".into()))] {
            let redacted = auth.is_some();
            let imported = from_export(&to_export(&full_rule(auth), true)).expect("imports");
            assert!(
                !imported.credential_was_redacted,
                "an unredacted export should never report a missing credential \
                 (had auth: {redacted})"
            );
        }
    }

    #[test]
    fn the_non_webhook_actions_survive_the_round_trip() {
        for action in [AlertAction::Log, AlertAction::Suppress] {
            let mut rule = full_rule(None);
            rule.action = action.clone();
            let imported = from_export(&to_export(&rule, true)).expect("imports");
            assert_eq!(imported.rule.action, action);
        }
    }

    #[test]
    fn an_unusable_entry_is_rejected_rather_than_imported_broken() {
        let base = to_export(&full_rule(None), true);

        let mut nameless = base.clone();
        nameless.name = "   ".into();
        assert_eq!(from_export(&nameless), Err(ImportRejection::NoName));

        let mut no_url = base.clone();
        no_url.webhook_url = None;
        assert_eq!(
            from_export(&no_url),
            Err(ImportRejection::WebhookWithoutUrl("Owls at night".into()))
        );

        let mut nonsense = base;
        nonsense.action = "launch_missiles".into();
        assert_eq!(
            from_export(&nonsense),
            Err(ImportRejection::UnknownAction {
                name: "Owls at night".into(),
                action: "launch_missiles".into(),
            })
        );
    }

    #[test]
    fn a_rejection_message_never_quotes_the_rules_credential() {
        // Import errors are rendered into the admin page and written to the
        // log. `ImportRejection` carries the rule name, which is operator-
        // chosen, and nothing else from the entry.
        let mut e = to_export(
            &full_rule(Some(WebhookAuth::Bearer("SUPERSECRETTOKEN".into()))),
            true,
        );
        e.action = "nonsense".into();
        let err = from_export(&e).expect_err("rejected");
        let rendered = format!("{err} {err:?}");
        assert!(!rendered.contains("SUPERSECRETTOKEN"), "{rendered}");
    }

    #[test]
    fn out_of_range_values_in_a_hand_edited_file_are_clamped_or_dropped() {
        // These files are meant to be hand-edited, so the importer is the
        // boundary. A confidence of 5.0 would make a rule that never matches;
        // an hour of 30 would make a window that never opens.
        let mut e = to_export(&full_rule(None), true);
        e.confidence_min = -3.0;
        e.confidence_max = 5.0;
        e.hour_start = Some(30);
        let r = from_export(&e).expect("imports").rule;
        assert!((r.confidence_min - 0.0).abs() < f64::EPSILON);
        assert!((r.confidence_max - 1.0).abs() < f64::EPSILON);
        assert_eq!(r.hour_start, None);
    }
}
