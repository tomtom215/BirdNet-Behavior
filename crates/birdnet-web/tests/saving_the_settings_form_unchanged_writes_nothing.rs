//! Pressing Save on a settings form nobody edited must change nothing.
//!
//! # The defect
//!
//! The save compared each submitted field with its row in the settings table,
//! or with `""` when there was no row. But the form shows a field with no row
//! at its *default* — `wav`, `0.80`, `monday`, `false` — and the browser posts
//! that back. So every field still at its default counted as changed. On a
//! fresh install (only the three keys the installer wrote are seeded), the
//! first press of Save with nothing edited wrote 32 rows, answered "Settings
//! saved (32 values updated)", and put a `settings.update` entry naming all 32
//! keys into the audit log. Each default was then pinned in the table, where it
//! outranks a later `birdnet.conf` edit of the same key.
//!
//! Measured on the real binary with an installer-shaped config before this
//! gate was written: 3 rows before the save, 35 after.
//!
//! # What is guarded
//!
//! Each form that posts to `/admin/settings` — the full page and the three
//! Station tabs — is rendered, its fields are collected the way a browser
//! submits them, and the submission is posted back unchanged: no row may be
//! written and nothing may be audited. The counterpart edits one field and
//! requires exactly that row and that audit entry, so "writes nothing" cannot
//! pass by the save being broken.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Every page carrying a form that posts to `/admin/settings`.
const FORM_PAGES: &[&str] = &[
    "/admin/settings",
    "/station/capture",
    "/station/alerts",
    "/station/settings",
];

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

async fn send(state: &AppState, req: Request<Body>) -> (StatusCode, String) {
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn get(state: &AppState, uri: &str) -> String {
    let req = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    let (status, body) = send(state, req).await;
    assert_eq!(status, StatusCode::OK, "GET {uri}");
    body
}

async fn post_settings(state: &AppState, fields: &[(String, String)]) -> String {
    let body = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish();
    let req = Request::builder()
        .method("POST")
        .uri("/admin/settings")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .expect("request");
    let (status, body) = send(state, req).await;
    assert_eq!(status, StatusCode::OK, "POST /admin/settings");
    body
}

fn settings_rows(state: &AppState) -> Vec<(String, String)> {
    state.with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT key, value FROM settings ORDER BY key")
            .expect("settings table");
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query")
            .map(Result::unwrap)
            .collect()
    })
}

fn settings_audits(state: &AppState) -> Vec<String> {
    state.with_db(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT COALESCE(metadata, '') FROM audit_log \
                 WHERE action = 'settings.update' ORDER BY id",
            )
            .expect("audit_log table");
        stmt.query_map([], |r| r.get(0))
            .expect("query")
            .map(Result::unwrap)
            .collect()
    })
}

/// The markup of the first form on `page` that posts to `/admin/settings`.
fn settings_form(page: &str) -> &str {
    let start = page
        .find(r#"<form hx-post="/admin/settings""#)
        .expect("a form posting to /admin/settings");
    let end = page[start..].find("</form>").expect("the form closes") + start;
    &page[start..end]
}

fn decode_entities(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The value of attribute `name` in the tag text `tag`, if present.
///
/// `None` for an absent attribute, `Some("")` for a bare one (`checked`).
fn attr(tag: &str, name: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut i = 0;
    while let Some(off) = tag[i..].find(name) {
        let at = i + off;
        i = at + name.len();
        let before_ok = at > 0 && bytes[at - 1].is_ascii_whitespace();
        let after = bytes.get(i).copied();
        if !before_ok {
            continue;
        }
        match after {
            Some(b'=') => {
                let rest = &tag[i + 1..];
                let value = rest.strip_prefix('"').map_or_else(
                    || rest.split(|c: char| c.is_whitespace() || c == '>').next(),
                    |q| q.split('"').next(),
                );
                return value.map(decode_entities);
            }
            Some(c) if c.is_ascii_whitespace() || c == b'>' || c == b'/' => {
                return Some(String::new());
            }
            None => return Some(String::new()),
            _ => {}
        }
    }
    None
}

/// The fields a browser would submit for this form, in document order.
fn submission(form: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = form;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[..=gt];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<input") {
            let kind = attr(tag, "type").unwrap_or_else(|| "text".into());
            let skip = matches!(kind.as_str(), "submit" | "button" | "reset" | "file")
                || attr(tag, "disabled").is_some()
                || (matches!(kind.as_str(), "checkbox" | "radio")
                    && attr(tag, "checked").is_none());
            if let (false, Some(name)) = (skip, attr(tag, "name")) {
                let default = if kind == "checkbox" || kind == "radio" {
                    "on"
                } else {
                    ""
                };
                out.push((name, attr(tag, "value").unwrap_or_else(|| default.into())));
            }
        } else if lower.starts_with("<select") {
            let close = rest.find("</select>").expect("select closes");
            if let (Some(name), None) = (attr(tag, "name"), attr(tag, "disabled")) {
                let body = &rest[gt + 1..close];
                let options: Vec<(&str, String)> = body
                    .split("<option")
                    .skip(1)
                    .map(|o| {
                        let (open, text) = o.split_once('>').unwrap_or((o, ""));
                        let text = text.split("</option>").next().unwrap_or("").trim();
                        let open_tag = format!(" {open}>");
                        let value =
                            attr(&open_tag, "value").unwrap_or_else(|| decode_entities(text));
                        (
                            if attr(&open_tag, "selected").is_some() {
                                "sel"
                            } else {
                                ""
                            },
                            value,
                        )
                    })
                    .collect();
                let chosen = options
                    .iter()
                    .find(|(s, _)| *s == "sel")
                    .or_else(|| options.first())
                    .map(|(_, v)| v.clone());
                if let Some(v) = chosen {
                    out.push((name, v));
                }
            }
            rest = &rest[close..];
            continue;
        } else if lower.starts_with("<textarea") {
            let close = rest.find("</textarea>").expect("textarea closes");
            if let (Some(name), None) = (attr(tag, "name"), attr(tag, "disabled")) {
                out.push((name, decode_entities(&rest[gt + 1..close])));
            }
            rest = &rest[close..];
            continue;
        }
        rest = &rest[gt + 1..];
    }
    out
}

#[tokio::test]
async fn saving_an_untouched_form_writes_and_audits_nothing() {
    for page_uri in FORM_PAGES {
        let (_dir, state) = station();
        let page = get(&state, page_uri).await;
        let fields = submission(settings_form(&page));
        // Precondition: the scan found the form's fields, or "nothing was
        // written" says nothing.
        assert!(
            fields.len() >= 5,
            "{page_uri}: only {} fields found — the form scan is broken",
            fields.len()
        );
        let before = settings_rows(&state);

        let reply = post_settings(&state, &fields).await;

        assert!(
            !reply.contains("Nothing was saved"),
            "{page_uri}: the untouched form was refused: {reply}"
        );
        let after = settings_rows(&state);
        let written: Vec<_> = after.iter().filter(|r| !before.contains(r)).collect();
        assert!(
            written.is_empty(),
            "{page_uri}: saving the untouched form wrote {} rows: {written:?}",
            written.len()
        );
        assert_eq!(
            settings_audits(&state),
            Vec::<String>::new(),
            "{page_uri}: an untouched save was audited as a change"
        );
    }
}

#[tokio::test]
async fn an_edited_field_is_written_and_audited_alone() {
    let (_dir, state) = station();
    let page = get(&state, "/admin/settings").await;
    let mut fields = submission(settings_form(&page));
    let format = fields
        .iter_mut()
        .find(|(k, _)| k == "audio_format")
        .expect("the full page carries audio_format");
    assert_eq!(format.1, "wav", "precondition: the form shows the default");
    format.1 = "flac".to_owned();

    post_settings(&state, &fields).await;

    assert_eq!(
        settings_rows(&state),
        vec![("audio_format".to_owned(), "flac".to_owned())],
        "exactly the edited field must be written"
    );
    assert_eq!(settings_audits(&state), vec!["audio_format".to_owned()]);
}
