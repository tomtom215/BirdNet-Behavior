//! Credentials on the settings forms: shown masked, kept when sent back as they
//! were shown.
//!
//! # The rule
//!
//! A credential is never printed in clear on any page, the administrator's
//! included. The administrator types a camera password or an SMTP password
//! once; after that the form shows where it is stored, not what it is.
//!
//! * A **secret** (the SMTP password, the BirdWeather token, the MQTT password,
//!   the eBird and Flickr keys) is shown as [`REDACTED`] when one is saved, and
//!   as an empty box when none is.
//! * An **address that carries a credential** (a camera's `rtsp://user:pass@…`,
//!   a notification URL, an Apprise server, a heartbeat ping) is shown through
//!   [`redact_value`] — the same rule the viewer's forms, the settings API, the
//!   wizard and the support bundle apply — so `rtsp://cam:hunter2@host/s`
//!   reads `rtsp://cam:***REDACTED***@host/s`.
//!
//! # What the save does with it
//!
//! A form posts back what it was shown, so a save that changed nothing about a
//! credential posts the mask. [`resolve`] puts the stored value back in place of
//! the mask **before anything else looks at the submission**: the checks then
//! run on the value the station will actually use (a bad value already stored is
//! found again when the page is saved, which is what "validate the submission,
//! not the diff" asks), and the diff finds the field unchanged and does not
//! write it. A value the person typed replaces the stored one; an empty box
//! clears it.
//!
//! A mask that has nothing to stand for is left in the submission, and
//! [`leftover_problems`] refuses it. That is the case where the address was
//! edited but its hidden password kept (`rtsp://cam:***REDACTED***@other-host/s`):
//! the station cannot know the password is still right for another host, so the
//! person is asked to type the whole address. The same check protects the
//! settings API, whose `GET` shows the same masks, from storing one.
//!
//! A list of addresses (`rtsp_urls`, `notify_urls`) is read one address at a
//! time, separated by commas or whitespace, so editing one camera leaves the
//! others' passwords alone.

use std::collections::HashMap;

use birdnet_core::config::redact::{REDACTED, redact_value};

use super::form::SettingsForm;

/// Said under a secret's box, so a person knows the dots are not a bug.
pub(crate) const SECRET_HINT: &str = "A saved value is never shown again; the box holds a \
    placeholder. Leave it as it is to keep the saved value, type a new one to replace it, or \
    empty the box to remove it.";

/// Said under an address that may carry a password.
pub(crate) const ADDRESS_HINT: &str = "A password in an address is hidden as ***REDACTED***. \
    Leave that as it is to keep the saved password; to change it, type the whole address again.";

/// How a settings key holds a credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Holds {
    /// The whole value is secret.
    Secret,
    /// The value is one or more addresses, any of which may carry a secret.
    Addresses,
}

/// The hand-written secrets. The table-driven ones are the
/// [`Kind::Secret`](super::extra::Kind::Secret) extras, read from their table so
/// a new one cannot be forgotten here.
pub(crate) const SECRET_KEYS: &[&str] = &["email_smtp_pass", "birdweather_token"];

/// The hand-written address fields that can carry a credential.
pub(crate) const ADDRESS_KEYS: &[&str] = &[
    "rtsp_url",
    "rtsp_urls",
    "apprise_url",
    "notify_urls",
    "heartbeat_url",
];

/// Keys whose names `is_secret_key` matches that are **not** credentials: the
/// path of a file on the station. Listed so the test that makes every
/// secret-looking key say what it is has somewhere to put these.
#[cfg(test)]
pub(crate) const SECRET_LOOKING_BUT_NOT: &[&str] = &["mqtt_client_key"];

/// How `key` holds a credential, or `None` when it does not.
pub(crate) fn holds(key: &str) -> Option<Holds> {
    if SECRET_KEYS.contains(&key)
        || super::extra::find(key).is_some_and(|e| matches!(e.kind, super::extra::Kind::Secret))
    {
        Some(Holds::Secret)
    } else if ADDRESS_KEYS.contains(&key) {
        Some(Holds::Addresses)
    } else {
        None
    }
}

/// A run of an address list: either an address (or other text) or the
/// separator between two.
enum Piece<'a> {
    Text(&'a str),
    Sep(&'a str),
}

/// `value` cut at commas and whitespace, separators kept, so putting the
/// pieces back together gives `value` exactly.
fn pieces(value: &str) -> Vec<Piece<'_>> {
    let is_sep = |c: char| c == ',' || c.is_whitespace();
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_sep = value.chars().next().is_some_and(is_sep);
    for (i, c) in value.char_indices() {
        if is_sep(c) != in_sep {
            let run = &value[start..i];
            out.push(if in_sep {
                Piece::Sep(run)
            } else {
                Piece::Text(run)
            });
            start = i;
            in_sep = !in_sep;
        }
    }
    let run = &value[start..];
    if !run.is_empty() {
        out.push(if in_sep {
            Piece::Sep(run)
        } else {
            Piece::Text(run)
        });
    }
    out
}

/// Every address in a list masked, the separators between them untouched.
pub(crate) fn mask_addresses(value: &str) -> String {
    pieces(value)
        .into_iter()
        .map(|p| match p {
            Piece::Text(t) if t.contains("://") => redact_value(t),
            Piece::Text(t) | Piece::Sep(t) => t.to_owned(),
        })
        .collect()
}

/// `raw` as an administrator's form shows it: the credentials masked, nothing
/// else touched.
///
/// A secret with nothing saved stays empty, so an empty box still means "not
/// set". A viewer's forms are masked more broadly by `mask_credentials`.
pub(crate) fn mask_for_admin(raw: &HashMap<String, String>) -> HashMap<String, String> {
    raw.iter()
        .map(|(k, v)| {
            let shown = match holds(k) {
                Some(Holds::Secret) if !v.is_empty() => REDACTED.to_owned(),
                Some(Holds::Addresses) => mask_addresses(v),
                _ => v.clone(),
            };
            (k.clone(), shown)
        })
        .collect()
}

/// The value `stored` stands for when `submitted` is what the form showed for
/// it, or `None` when `submitted` carries a mask that stands for nothing.
///
/// `Some(submitted)` unchanged when it carries no mask at all.
fn resolve_value(holds: Holds, submitted: &str, stored: &str) -> Option<String> {
    match holds {
        Holds::Secret => {
            if submitted.trim() != REDACTED {
                return Some(submitted.to_owned());
            }
            (!stored.is_empty()).then(|| stored.to_owned())
        }
        Holds::Addresses => resolve_addresses(submitted, stored).map(|resolved| {
            // A browser posts a textarea's line breaks as CRLF. The addresses
            // are the stored ones either way; keeping the stored text when
            // that is all that differs leaves the field unchanged, as it is.
            if resolved.replace("\r\n", "\n") == stored {
                stored.to_owned()
            } else {
                resolved
            }
        }),
    }
}

fn resolve_addresses(submitted: &str, stored: &str) -> Option<String> {
    if !submitted.contains(REDACTED) {
        return Some(submitted.to_owned());
    }
    // The stored addresses, in order, each with the mask the form showed for it.
    let saved: Vec<(&str, String)> = pieces(stored)
        .into_iter()
        .filter_map(|p| match p {
            Piece::Text(t) if t.contains("://") => Some((t, redact_value(t))),
            _ => None,
        })
        .collect();
    let mut used = vec![false; saved.len()];
    let mut ordinal = 0_usize;
    let mut out = String::with_capacity(submitted.len());
    for piece in pieces(submitted) {
        match piece {
            Piece::Sep(s) => out.push_str(s),
            Piece::Text(t) if !t.contains(REDACTED) => {
                if t.contains("://") {
                    ordinal += 1;
                }
                out.push_str(t);
            }
            Piece::Text(t) => {
                // The address at the same place in the list, if it was shown as
                // this; otherwise the first unused one that was. Two cameras
                // with the same user and host differ only in their passwords,
                // and their masks are identical, so position decides first.
                let here = ordinal;
                ordinal += 1;
                let at = if saved.get(here).is_some_and(|(_, m)| m == t) && !used[here] {
                    Some(here)
                } else {
                    saved
                        .iter()
                        .enumerate()
                        .position(|(i, (_, m))| m == t && !used[i])
                };
                let at = at?;
                used[at] = true;
                out.push_str(saved[at].0);
            }
        }
    }
    Some(out)
}

/// Put the stored value back in place of every mask the form was shown, in
/// `form`, before it is checked or diffed. See the module doc.
pub(crate) fn resolve(form: &mut SettingsForm, existing: &HashMap<String, String>) {
    macro_rules! each {
        ($($field:ident),* $(,)?) => {$(
            if let Some(v) = form.$field.as_mut() {
                restore(stringify!($field), v, existing);
            }
        )*};
    }
    each!(
        email_smtp_pass,
        birdweather_token,
        rtsp_url,
        rtsp_urls,
        apprise_url,
        notify_urls,
        heartbeat_url,
    );
    for (key, v) in &mut form.extra {
        restore(key, v, existing);
    }
}

fn restore(key: &str, value: &mut String, existing: &HashMap<String, String>) {
    let Some(kind) = holds(key) else { return };
    let stored = existing.get(key).map_or("", String::as_str);
    if let Some(resolved) = resolve_value(kind, value, stored) {
        *value = resolved;
    }
}

/// Each submitted credential field that still carries a mask after [`resolve`],
/// explained. A mask is never a value to store.
///
/// The messages name the field and never the value, which usually holds a
/// camera password.
pub(crate) fn leftover_problems(form: &SettingsForm) -> Vec<String> {
    let mut named: Vec<(&str, &str)> = Vec::new();
    macro_rules! each {
        ($($field:ident),* $(,)?) => {$(
            if let Some(v) = form.$field.as_deref() {
                named.push((stringify!($field), v));
            }
        )*};
    }
    each!(
        email_smtp_pass,
        birdweather_token,
        rtsp_url,
        rtsp_urls,
        apprise_url,
        notify_urls,
        heartbeat_url,
    );
    named.extend(form.extra.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let mut problems: Vec<String> = named
        .into_iter()
        .filter(|(key, v)| holds(key).is_some() && v.contains(REDACTED))
        .map(|(key, _)| {
            format!(
                "{} holds the placeholder {REDACTED}, which only stands for a value that is \
                 already saved and sent back unchanged. Type the whole value again, password \
                 included, or clear the box.",
                key.replace('_', " ")
            )
        })
        .collect();
    problems.sort();
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn a_saved_secret_is_shown_as_the_mask_and_an_unset_one_as_nothing() {
        let shown = mask_for_admin(&map(&[
            ("email_smtp_pass", "hunter2-pw"),
            ("mqtt_password", ""),
            ("station_name", "Garden"),
        ]));
        assert_eq!(shown["email_smtp_pass"], REDACTED);
        assert_eq!(shown["mqtt_password"], "");
        assert_eq!(shown["station_name"], "Garden");
    }

    #[test]
    fn a_list_of_addresses_is_masked_one_at_a_time_and_keeps_its_separators() {
        let stored = "rtsp://a:pw1@h1/s, rtsp://b:pw2@h2/s\nntfy://topic-secret";
        let shown = mask_addresses(stored);
        assert_eq!(
            shown,
            "rtsp://a:***REDACTED***@h1/s, rtsp://b:***REDACTED***@h2/s\nntfy://***REDACTED***"
        );
        assert!(!shown.contains("pw1") && !shown.contains("topic-secret"));
        // Nothing to hide: nothing changes, so an unchanged field is byte-equal.
        assert_eq!(
            mask_addresses("rtsp://cam.local/stream"),
            "rtsp://cam.local/stream"
        );
    }

    #[test]
    fn what_the_form_was_shown_resolves_back_to_exactly_what_was_stored() {
        for stored in [
            "rtsp://a:pw1@h1/s",
            "rtsp://a:pw1@h1/s,rtsp://b:pw2@h2/s",
            "rtsp://a:pw1@h1/s,  rtsp://plain.local/s\nntfy://tok",
            "http://apprise.lan:8000",
            "srt://cam:8890?streamid=read:garden:ada:S3CRET&passphrase=P4SS",
        ] {
            let shown = mask_addresses(stored);
            assert_eq!(
                resolve_addresses(&shown, stored).as_deref(),
                Some(stored),
                "{stored} -> {shown}"
            );
        }
    }

    #[test]
    fn editing_one_camera_in_a_list_keeps_the_others_passwords() {
        // A kept camera among others, found by its mask.
        let stored = "rtsp://a:pw1@h1/s,rtsp://b:pw2@h2/s";
        let kept_second = "rtsp://b:***REDACTED***@h2/s";
        assert_eq!(
            resolve_addresses(kept_second, stored).as_deref(),
            Some("rtsp://b:pw2@h2/s")
        );
        // A new camera added beside a kept one.
        let edited = "rtsp://a:***REDACTED***@h1/s,rtsp://c:newpw@h3/s";
        assert_eq!(
            resolve_addresses(edited, stored).as_deref(),
            Some("rtsp://a:pw1@h1/s,rtsp://c:newpw@h3/s")
        );
    }

    #[test]
    fn a_mask_with_nothing_behind_it_is_not_resolved() {
        // The host was edited and the password kept: not a value the station
        // can know is right.
        assert_eq!(
            resolve_addresses("rtsp://a:***REDACTED***@other/s", "rtsp://a:pw@h1/s"),
            None
        );
        assert_eq!(resolve_value(Holds::Secret, REDACTED, ""), None);
        // A secret that was typed is a secret, even next to nothing saved.
        assert_eq!(
            resolve_value(Holds::Secret, "newpw", "").as_deref(),
            Some("newpw")
        );
        // An exact mask stands for what is saved.
        assert_eq!(
            resolve_value(Holds::Secret, REDACTED, "oldpw").as_deref(),
            Some("oldpw")
        );
    }

    #[test]
    fn every_secret_looking_settings_key_says_what_it_is() {
        // A key whose name `is_secret_key` matches either holds a credential
        // here or is named as a file path that merely looks like one. A new
        // key must make that decision, not inherit a clear-text box.
        for key in super::super::form::all_form_keys() {
            if birdnet_core::config::redact::is_secret_key(key) {
                assert!(
                    holds(key).is_some() || SECRET_LOOKING_BUT_NOT.contains(&key),
                    "`{key}` looks like a secret to the redactor, but the settings form \
                     would print it in clear: classify it in settings/masking.rs"
                );
            }
        }
        for key in SECRET_LOOKING_BUT_NOT {
            assert!(holds(key).is_none(), "{key}");
        }
    }
}
