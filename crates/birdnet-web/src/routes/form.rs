//! Reading a form the way a browser sends it.
//!
//! # The defect class this module closes
//!
//! A browser submits **every** control of a form, a blank one as `name=`; an
//! unchecked checkbox is absent. `serde` reads `""` as an error for a number, a
//! `bool` or an enum, so an optional numeric field that the page labels
//! "blank = any" made axum's `Form` extractor refuse the whole request with a
//! `422`. htmx never swaps a 4xx/5xx body (`static/htmx.min.js` ships
//! `{code:"[45]..", swap:false, error:true}`, and `layout.html`'s
//! `htmx:responseError` fallback only speaks for `GET`), so the person saw
//! nothing at all: no result, no message, no sign the click registered. The
//! alert-rule form and the audio-source form both did this.
//!
//! Two pieces close it, and neither is enough alone:
//!
//! * [`blank_is_none`] / [`blank_bool_is_none`] / [`checkbox`] are `serde`
//!   field adapters that say what a blank means (none / off) where the field
//!   is declared. They are what makes a blank *right*, not merely survivable,
//!   and `tests/an_htmx_form_cannot_fail_silently.rs` requires one on every
//!   optional numeric, boolean or enum field of a form struct.
//! * [`HtmxForm`] is the extractor every form handler takes instead of
//!   `axum::Form`. It catches what the adapters cannot foresee (a garbled
//!   number, a field the page no longer sends, a body over the limit): a
//!   request from htmx is answered `200` with `HX-Reswap: none` and an error
//!   toast that names the field; any other client keeps the `4xx` it would
//!   have had, with a small JSON body instead of serde's prose.
//!
//! # What the toast never says
//!
//! `serde`'s messages can quote the value that failed (an "unknown variant"
//! error repeats the variant it was given). The text shown is therefore built from the field's *name* and
//! a fixed phrase chosen by the *kind* of failure; the raw message is logged at
//! `debug` level and never returned. A field name is only printed when it is a
//! plain identifier, so a hostile key cannot inject markup (the toast escapes
//! it as well).

use std::fmt::Display;
use std::str::FromStr;

use axum::extract::rejection::FormRejection;
use axum::extract::{FromRequest, Request};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde::de::{DeserializeOwned, Deserializer, Error as _};

use crate::routes::pages::toast::{self, Toast};

/// Is this request from htmx (it sets `HX-Request: true` on every request)?
#[must_use]
pub fn is_htmx(headers: &HeaderMap) -> bool {
    headers
        .get("hx-request")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
}

/// A form body, read as a browser sends it. Take this instead of
/// [`axum::Form`] in every handler a page can reach.
///
/// See the [module documentation](self) for what it changes and why.
#[derive(Debug, Clone, Copy, Default)]
pub struct HtmxForm<T>(pub T);

impl<S, T> FromRequest<S> for HtmxForm<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let htmx = is_htmx(req.headers());
        match axum::Form::<T>::from_request(req, state).await {
            Ok(axum::Form(value)) => Ok(Self(value)),
            Err(rejection) => Err(refusal(&rejection, htmx)),
        }
    }
}

/// What was wrong with the form, in words that are safe to show.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Problem {
    /// The offending field, when the error named one and it is a plain name.
    pub field: Option<String>,
    /// The sentence that follows the field name (or stands alone).
    pub what: &'static str,
}

impl Problem {
    /// The toast text: `"Not submitted: sample_rate must be a whole number."`
    pub(crate) fn message(&self) -> String {
        self.field.as_ref().map_or_else(
            || format!("Not submitted: {}", self.what),
            |f| format!("Not submitted: \u{201c}{f}\u{201d} {}", self.what),
        )
    }
}

/// Classify a rejection's text without repeating any of it.
///
/// The input is `FormRejection::body_text()`, e.g.
/// `Failed to deserialize form body: hour_start: cannot parse integer from empty string`
/// or `Failed to deserialize form body: missing field` followed by the name of the
/// field in backticks.
pub(crate) fn classify(text: &str) -> Problem {
    let detail = text
        .strip_prefix("Failed to deserialize form body: ")
        .or_else(|| text.strip_prefix("Failed to deserialize form body"))
        .unwrap_or(text)
        .trim();

    if let Some(rest) = detail.strip_prefix("missing field `") {
        let name = rest.split('`').next().unwrap_or_default();
        return Problem {
            field: plain_name(name),
            what: "is missing from the submitted form; reload the page and try again.",
        };
    }

    // serde_path_to_error prefixes the path: "hour_start: cannot parse ...".
    let (field, reason) = match detail.split_once(": ") {
        Some((path, reason)) if plain_name(path).is_some() => (plain_name(path), reason),
        _ => (None, detail),
    };
    let reason = reason.to_ascii_lowercase();
    let what = if reason.contains("empty string") {
        "was left blank, and this field needs a value."
    } else if reason.contains("invalid digit")
        || reason.contains("too large")
        || reason.contains("too small")
        || reason.contains("invalid float")
        || reason.contains("number")
        || reason.contains("integer")
    {
        "must be a number."
    } else if reason.contains("unknown variant") {
        "is not one of the choices the page offers."
    } else if reason.contains("true") && reason.contains("false") {
        "must be on or off."
    } else {
        "could not be read."
    };
    Problem { field, what }
}

/// A field path that is safe to print: an identifier, optionally dotted, short.
fn plain_name(s: &str) -> Option<String> {
    let ok = !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '[' | ']'));
    ok.then(|| s.to_string())
}

fn refusal(rejection: &FormRejection, htmx: bool) -> Response {
    let status = rejection.status();
    let raw = rejection.body_text();
    tracing::debug!(%status, error = %raw, htmx, "form refused");

    let problem = match status {
        StatusCode::PAYLOAD_TOO_LARGE => Problem {
            field: None,
            what: "the form is larger than this station accepts.",
        },
        StatusCode::UNSUPPORTED_MEDIA_TYPE => Problem {
            field: None,
            what: "the form was sent in a format this station cannot read; reload the page and try again.",
        },
        _ => classify(&raw),
    };
    let message = problem.message();

    if htmx {
        // 200 and no swap: htmx discards a 4xx body, toast included, and the
        // page must stay as the person left it. Same shape as `not_applied`.
        return toast::not_applied(&Toast::error(message));
    }
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "error": message, "field": problem.field }).to_string(),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Field adapters. Use as `#[serde(default, deserialize_with = "blank_is_none")]`.
// ---------------------------------------------------------------------------

/// A blank (or all-white-space) value is `None`; anything else must parse.
///
/// For `Option<u32>`, `Option<f64>`, `Option<i64>` and any other `FromStr`
/// type. A value that is present and does not parse is an **error**, not
/// `None`: turning `abc` into "any" would be a silent guess, and the person
/// would never learn the field was ignored.
///
/// # Errors
///
/// When the value is not blank and is not a valid `T`.
pub fn blank_is_none<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: Display,
{
    Option::<String>::deserialize(d)?.map_or(Ok(None), |raw| {
        let t = raw.trim();
        if t.is_empty() {
            Ok(None)
        } else {
            t.parse().map(Some).map_err(D::Error::custom)
        }
    })
}

/// What a checkbox or a toggle sends: `on`, `true`, `1`, `yes` are true;
/// `off`, `false`, `0`, `no` are false.
fn parse_switch(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "yes" => Some(true),
        "off" | "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

/// A blank is `None`; otherwise a switch word (`on`/`true`/`1`/`yes`,
/// `off`/`false`/`0`/`no`).
///
/// # Errors
///
/// When the value is not blank and not a switch word.
pub fn blank_bool_is_none<'de, D>(d: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<String>::deserialize(d)? {
        None => Ok(None),
        Some(raw) if raw.trim().is_empty() => Ok(None),
        Some(raw) => parse_switch(&raw)
            .map(Some)
            .ok_or_else(|| D::Error::custom("provided string was not `true` or `false`")),
    }
}

/// A checkbox: absent is off (pair it with `#[serde(default)]`), blank is off,
/// otherwise a switch word.
///
/// # Errors
///
/// When the value is not blank and not a switch word.
pub fn checkbox<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    blank_bool_is_none(d).map(|v| v.unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Probe {
        #[serde(default, deserialize_with = "blank_is_none")]
        n: Option<u32>,
        #[serde(default, deserialize_with = "blank_is_none")]
        x: Option<f64>,
        #[serde(default, deserialize_with = "blank_bool_is_none")]
        flag: Option<bool>,
        #[serde(default, deserialize_with = "checkbox")]
        box_: bool,
    }

    /// `serde_urlencoded` is not a direct dependency; every field here is read
    /// as a string first, exactly as it is from a form, so JSON strings exercise
    /// the same path. The real-router behaviour is gated in
    /// `tests/an_htmx_form_cannot_fail_silently.rs`.
    fn read(fields: &[(&str, &str)]) -> Result<Probe, serde_json::Error> {
        let map: serde_json::Map<String, serde_json::Value> = fields
            .iter()
            .map(|(k, v)| {
                (
                    (*k).to_string(),
                    serde_json::Value::String((*v).to_string()),
                )
            })
            .collect();
        serde_json::from_value(serde_json::Value::Object(map))
    }

    #[test]
    fn a_blank_is_none_and_a_value_is_read() {
        assert_eq!(
            read(&[("n", ""), ("x", ""), ("flag", ""), ("box_", "")]).unwrap(),
            Probe {
                n: None,
                x: None,
                flag: None,
                box_: false
            }
        );
        assert_eq!(
            read(&[("n", " 7"), ("x", "0.5"), ("flag", "on"), ("box_", "true")]).unwrap(),
            Probe {
                n: Some(7),
                x: Some(0.5),
                flag: Some(true),
                box_: true
            }
        );
        assert_eq!(
            read(&[]).unwrap(),
            Probe {
                n: None,
                x: None,
                flag: None,
                box_: false
            }
        );
    }

    #[test]
    fn garbage_is_an_error_not_a_silent_none() {
        assert!(read(&[("n", "abc")]).is_err());
        assert!(read(&[("n", "-1")]).is_err(), "u32 refuses a negative");
        assert!(read(&[("x", "1,5")]).is_err());
        assert!(read(&[("flag", "maybe")]).is_err());
        assert!(read(&[("box_", "2")]).is_err());
    }

    #[test]
    fn classification_names_the_field_and_never_repeats_the_value() {
        let p = classify(
            "Failed to deserialize form body: sample_rate: cannot parse integer from empty string",
        );
        assert_eq!(p.field.as_deref(), Some("sample_rate"));
        assert!(p.message().contains("sample_rate"));
        assert!(p.message().contains("blank"));

        let p = classify("Failed to deserialize form body: missing field `kind`");
        assert_eq!(p.field.as_deref(), Some("kind"));
        assert!(p.message().contains("missing"));

        // serde quotes the offending value in some messages; it must not survive.
        let p = classify(
            "Failed to deserialize form body: mode: unknown variant `hunter2`, expected `a` or `b`",
        );
        assert_eq!(p.field.as_deref(), Some("mode"));
        assert!(!p.message().contains("hunter2"), "{}", p.message());

        let p = classify("Failed to deserialize form body: id: invalid digit found in string");
        assert!(p.message().contains("number"));
    }

    #[test]
    fn a_field_name_that_is_not_an_identifier_is_not_printed() {
        let p = classify(
            "Failed to deserialize form body: <script>alert(1)</script>: invalid digit found in string",
        );
        assert_eq!(p.field, None);
        assert!(!p.message().contains("script"));
        assert_eq!(plain_name("a b"), None);
        assert_eq!(plain_name(&"x".repeat(65)), None);
        assert_eq!(plain_name("hour_start").as_deref(), Some("hour_start"));
    }

    #[test]
    fn only_a_true_hx_request_header_is_htmx() {
        let mut h = HeaderMap::new();
        assert!(!is_htmx(&h));
        h.insert("hx-request", "true".parse().unwrap());
        assert!(is_htmx(&h));
        h.insert("hx-request", "false".parse().unwrap());
        assert!(!is_htmx(&h));
    }
}
