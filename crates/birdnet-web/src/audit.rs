//! Writing the audit log.
//!
//! # What was wrong
//!
//! The table, the store, the admin page and the pruner all existed.
//! [`AuditLog::record`] had **zero production callers** — every call site was
//! inside its own `#[cfg(test)]` block. `/admin/audit` was permanently empty,
//! which on a shared station does not read as "the log is broken"; it reads as
//! "nothing happened".
//!
//! The repo had already caught half of this once: the *pruner* was wired after
//! being found to have no caller, and a retention constant was written for it.
//! Six months of retention on rows nobody wrote.
//!
//! # What is recorded
//!
//! Mutations and authentication, not reads. A `GET` that renders a page is not
//! an event; a `POST` that changes what the station does, or that lets someone
//! new in, is. Actions are dotted and hierarchical (`auth.login.ok`,
//! `account.user.create`, `settings.update`) so `/admin/audit`'s `LIKE` filter
//! can select a family with a prefix.
//!
//! # Values never appear
//!
//! Metadata carries *which* settings changed, never what they changed to. A
//! settings save whose diff included `CADDY_PWD=hunter2` would otherwise put
//! the admin password in a table the audit page renders and the support bundle
//! has no reason to redact. Key names are enough to answer "who changed the
//! recording schedule on the 3rd?", which is the question the log exists for.

use birdnet_db::accounts::AuditLog;

use crate::auth_middleware::RequestUser;
use crate::state::AppState;

/// Record one auditable event.
///
/// Best-effort by design: a station must not refuse an operator's password
/// change because the audit insert failed. The failure is logged at `warn`,
/// which is itself now persisted (see the binary's `log_capture`), so a
/// station whose audit log has silently stopped is still discoverable.
///
/// `user` is `None` for events with no authenticated actor — a failed login is
/// the whole reason that column is nullable.
pub fn audit(
    state: &AppState,
    user: Option<&RequestUser>,
    action: &str,
    target: Option<&str>,
    metadata: Option<&str>,
) {
    let user_id = user.map(|u| u.user.id);
    let result = state.with_db(|conn| conn.record(user_id, action, target, metadata));
    if let Err(e) = result {
        tracing::warn!(error = %e, action, "audit log write failed");
    }
}

/// Record an event for a user identified by id rather than by request.
///
/// Login is the case: the session does not exist yet when the row is written,
/// so there is no [`RequestUser`] to hand over.
pub fn audit_user_id(
    state: &AppState,
    user_id: Option<i64>,
    action: &str,
    target: Option<&str>,
    metadata: Option<&str>,
) {
    let result = state.with_db(|conn| conn.record(user_id, action, target, metadata));
    if let Err(e) = result {
        tracing::warn!(error = %e, action, "audit log write failed");
    }
}

/// Render the names of the settings keys that changed, as audit metadata.
///
/// Names only, sorted, comma-separated — never values. See the module doc.
/// Returns `None` when nothing changed, so a save that altered nothing writes
/// no row at all rather than an empty one.
#[must_use]
pub fn changed_keys(before: &[(String, String)], after: &[(String, String)]) -> Option<String> {
    let mut names: Vec<&str> = Vec::new();
    for (key, new_value) in after {
        let old = before
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str());
        if old != Some(new_value.as_str()) {
            names.push(key);
        }
    }
    for (key, _) in before {
        if !after.iter().any(|(k, _)| k == key) {
            names.push(key);
        }
    }
    if names.is_empty() {
        return None;
    }
    names.sort_unstable();
    names.dedup();
    Some(names.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(kv: &[(&str, &str)]) -> Vec<(String, String)> {
        kv.iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn only_the_keys_that_changed_are_named() {
        let before = pairs(&[("LATITUDE", "52.5"), ("MODEL", "BirdNET_6K")]);
        let after = pairs(&[("LATITUDE", "48.1"), ("MODEL", "BirdNET_6K")]);
        assert_eq!(changed_keys(&before, &after).as_deref(), Some("LATITUDE"));
    }

    #[test]
    fn a_save_that_changed_nothing_writes_no_row() {
        // The discrimination. A settings page posts every field on every save,
        // so "the form was submitted" is not "something changed" — recording
        // it as one turns the audit log into a click counter.
        let same = pairs(&[("LATITUDE", "52.5"), ("MODEL", "BirdNET_6K")]);
        assert_eq!(changed_keys(&same, &same), None);
    }

    #[test]
    fn values_never_appear_in_the_metadata() {
        // A settings diff including CADDY_PWD would otherwise put the admin
        // password into a table the audit page renders.
        let before = pairs(&[("CADDY_PWD", "old-secret")]);
        let after = pairs(&[("CADDY_PWD", "hunter2")]);
        let meta = changed_keys(&before, &after).expect("a change");
        assert_eq!(meta, "CADDY_PWD");
        assert!(!meta.contains("hunter2"), "{meta}");
        assert!(!meta.contains("old-secret"), "{meta}");
    }

    #[test]
    fn an_added_or_removed_key_counts_as_a_change() {
        let before = pairs(&[("A", "1")]);
        let after = pairs(&[("A", "1"), ("B", "2")]);
        assert_eq!(changed_keys(&before, &after).as_deref(), Some("B"));
        assert_eq!(changed_keys(&after, &before).as_deref(), Some("B"));
    }

    #[test]
    fn the_key_list_is_sorted_and_deduplicated() {
        // `/admin/audit` renders this string verbatim; a stable order is what
        // makes two saves comparable at a glance.
        let before = pairs(&[("Z", "1"), ("A", "1")]);
        let after = pairs(&[("Z", "2"), ("A", "2")]);
        assert_eq!(changed_keys(&before, &after).as_deref(), Some("A,Z"));
    }
}
