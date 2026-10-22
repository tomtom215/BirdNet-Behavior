//! A credential is never printed in clear on any page, the administrator's
//! included.
//!
//! # The defect
//!
//! The administrator's settings forms printed every stored value into
//! `value="…"`: the SMTP password, the BirdWeather token, the MQTT password, the
//! eBird and Flickr keys, and each camera and notification address with its
//! password in it. `/admin/audio` printed a stream's address whole, in its list
//! and in its edit form. A viewer's copy of the same pages was masked, and so
//! were the logs, the wizard's summary, the doctor and the support bundle; the
//! administrator's was the one place a password could be read off a screen, out
//! of a screenshot attached to a bug report, or from a proxy's cache.
//!
//! # What is guarded
//!
//! * No page the administrator can open prints any canary secret, with a
//!   counterpart that the page still shows the parts that are not secret (the
//!   camera's host, the account name) and a mask where the password was.
//! * Pressing Save on the form exactly as it was shown keeps every stored
//!   credential, and writes and audits nothing.
//! * A value typed over a mask replaces the stored one; an emptied box clears
//!   it; a typed password replaces one inside a list without touching its
//!   neighbours.
//! * A mask that stands for nothing (the host edited, the hidden password
//!   kept) is refused, and a bad value already stored is refused again when its
//!   own mask is sent back: the submission is validated, not the diff.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_db::audio_sources::{AudioSourceStore as _, NewAudioSource, SourceKind};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const MASK: &str = "***REDACTED***";

/// Every canary is a different string so a failure names the field that leaked.
const SECRETS: &[(&str, &str)] = &[
    ("email_smtp_pass", "smtpCANARYpass"),
    ("birdweather_token", "bwCANARYtoken"),
    ("mqtt_password", "mqttCANARYpass"),
    ("ebird_api_key", "ebirdCANARYkey"),
    ("flickr_api_key", "flickrCANARYkey"),
];

const ADDRESSES: &[(&str, &str, &str)] = &[
    (
        "rtsp_url",
        "rtsp://cam:rtspCANARYone@camera.local/stream",
        "rtspCANARYone",
    ),
    (
        "rtsp_urls",
        "rtsp://a:multiCANARYa@cam1.local/s,rtsp://b:multiCANARYb@cam2.local/s",
        "multiCANARY",
    ),
    (
        "apprise_url",
        "http://apprise.local:8000/notify/appriseCANARYkey",
        "appriseCANARYkey",
    ),
    (
        "notify_urls",
        "tgram://tgCANARYtoken/987\nntfy://topicCANARY",
        "CANARY",
    ),
    (
        "heartbeat_url",
        "https://hc-ping.example/hbCANARYuuid",
        "hbCANARYuuid",
    ),
];

const SOURCE_PASSWORD: &str = "sourceCANARYpw";

/// Built at run time, not written as a literal (see the viewer gate's note on
/// CodeQL's `rust/hard-coded-cryptographic-value`).
fn admin_password() -> String {
    format!("admin-password-{}", std::process::id())
}

/// A station with an administrator, every credential above saved, and — when
/// `with_source` — one camera in the audio-source list.
fn station(with_source: bool) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let hash = birdnet_db::accounts::hash_password(&admin_password()).expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [&hash],
    )
    .expect("admin password");
    for (key, value) in SECRETS {
        birdnet_db::settings::set(
            &conn,
            key,
            value,
            birdnet_db::settings::SettingsCategory::System,
        )
        .expect("secret");
    }
    for (key, value, _) in ADDRESSES {
        birdnet_db::settings::set(
            &conn,
            key,
            value,
            birdnet_db::settings::SettingsCategory::System,
        )
        .expect("address");
    }
    if with_source {
        conn.insert(&NewAudioSource::defaults(
            "cam1",
            SourceKind::Rtsp,
            format!("rtsp://viewer:{SOURCE_PASSWORD}@camera.local/stream"),
        ))
        .expect("rtsp source");
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn sign_in(app: &axum::Router) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/login")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("origin", "http://localhost")
                .header("host", "localhost")
                .body(Body::from(format!(
                    "username=admin&password={}",
                    admin_password()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "the admin signs in");
    res.headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), 8 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

async fn get(app: &axum::Router, uri: &str, cookie: &str) -> (StatusCode, String) {
    send(
        app,
        Request::builder()
            .uri(uri)
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

async fn write(
    app: &axum::Router,
    method: Method,
    uri: &str,
    cookie: &str,
    body: String,
) -> (StatusCode, String) {
    send(
        app,
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("origin", "http://localhost")
            .header("host", "localhost")
            .body(Body::from(body))
            .unwrap(),
    )
    .await
}

async fn post_settings(
    app: &axum::Router,
    cookie: &str,
    fields: &[(String, String)],
) -> (StatusCode, String) {
    let body = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish();
    write(app, Method::POST, "/admin/settings", cookie, body).await
}

fn stored(state: &AppState, key: &str) -> String {
    state.with_db(|conn| birdnet_db::settings::get(conn, key).unwrap_or_default())
}

fn audit_count(state: &AppState) -> i64 {
    state.with_db(|conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'settings.update'",
            [],
            |r| r.get(0),
        )
        .expect("audit_log")
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

/// The pages a station's administrator opens, from the settings form to the
/// live view. Not all answer 200 on a fixture; a body that is not the page is
/// no leak, and the precondition below counts the ones that are.
const PAGES: &[&str] = &[
    "/",
    "/admin/settings",
    "/admin/settings/partial",
    "/station",
    "/station/capture",
    "/station/alerts",
    "/station/data",
    "/station/settings",
    "/station/access",
    "/admin/overview",
    "/admin/system",
    "/admin/audio/sources/cam1",
    "/admin/audio/sources/cam1/edit",
    "/admin/audio/sources/cam1/probe",
    "/admin/notifications",
    "/admin/notifications/test",
    "/admin/rules",
    "/admin/backups",
    "/admin/doctor",
    "/admin/doctor.json",
    "/recordings?view=live",
];

fn every_canary() -> Vec<&'static str> {
    let mut all: Vec<&str> = SECRETS.iter().map(|(_, v)| *v).collect();
    all.extend(ADDRESSES.iter().map(|(_, _, c)| *c));
    all.push(SOURCE_PASSWORD);
    all
}

/// Seen failing before the change: `/admin/settings` carried all five secrets
/// and every address in `value="…"`, and `/admin/audio/sources/cam1/edit` the
/// camera's password.
#[tokio::test]
async fn no_page_prints_a_credential_in_clear_to_the_administrator() {
    for with_source in [false, true] {
        let app = build_router(station(with_source));
        let cookie = sign_in(&app).await;
        let mut served = 0;
        for uri in PAGES {
            let (status, html) = get(&app, uri, &cookie).await;
            if status == StatusCode::OK {
                served += 1;
            }
            for canary in every_canary() {
                assert!(
                    !html.contains(canary),
                    "{uri} (source in the list: {with_source}) printed {canary} in clear"
                );
            }
        }
        assert!(
            served >= 12,
            "only {served} pages answered: the sweep saw too little"
        );

        // Counterpart: the pages are not blanked. The host and the account are
        // what a person needs to recognise their camera, and the mask is where
        // the password was.
        let (_, html) = get(&app, "/admin/settings", &cookie).await;
        assert!(
            html.contains(r#"name="email_smtp_pass" type="password" value="***REDACTED***""#),
            "a saved secret shows as the mask"
        );
        if with_source {
            let (_, html) = get(&app, "/admin/audio/sources/cam1/edit", &cookie).await;
            assert!(
                html.contains("rtsp://viewer:***REDACTED***@camera.local/stream"),
                "{html}"
            );
        } else {
            assert!(
                html.contains("rtsp://cam:***REDACTED***@camera.local/stream"),
                "the camera's account and host stay readable: {html}"
            );
            assert!(html.contains(
                "rtsp://a:***REDACTED***@cam1.local/s,rtsp://b:***REDACTED***@cam2.local/s"
            ));
        }
    }
}

async fn shown_fields(app: &axum::Router, cookie: &str) -> Vec<(String, String)> {
    let (status, page) = get(app, "/admin/settings", cookie).await;
    assert_eq!(status, StatusCode::OK);
    let fields = submission(settings_form(&page));
    assert!(
        fields.len() >= 20,
        "the form scan found {} fields",
        fields.len()
    );
    fields
}

fn set_field(fields: &mut [(String, String)], key: &str, value: &str) {
    fields
        .iter_mut()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("the form has no {key} field"))
        .1 = value.to_owned();
}

fn field<'a>(fields: &'a [(String, String)], key: &str) -> &'a str {
    fields.iter().find(|(k, _)| k == key).map_or_else(
        || panic!("the form has no {key} field"),
        |(_, v)| v.as_str(),
    )
}

/// The form exactly as shown, pressed Save on. Every credential must still be
/// what it was, and nothing may be written or audited.
#[tokio::test]
async fn saving_the_form_as_shown_keeps_every_stored_credential() {
    let state = station(false);
    let app = build_router(state.clone());
    let cookie = sign_in(&app).await;
    let fields = shown_fields(&app, &cookie).await;
    // Precondition: the form really carried masks, so "kept" is not "never
    // sent".
    for (key, _) in SECRETS {
        assert_eq!(field(&fields, key), MASK, "{key} is shown as the mask");
    }
    for (key, _, canary) in ADDRESSES {
        assert!(!field(&fields, key).contains(canary), "{key}");
        assert!(field(&fields, key).contains(MASK), "{key}");
    }
    let before = audit_count(&state);

    let (status, reply) = post_settings(&app, &cookie, &fields).await;

    assert_eq!(status, StatusCode::OK);
    assert!(!reply.contains("Nothing was saved"), "{reply}");
    for (key, value) in SECRETS {
        assert_eq!(stored(&state, key), *value, "{key} was overwritten");
    }
    for (key, value, _) in ADDRESSES {
        assert_eq!(stored(&state, key), *value, "{key} was overwritten");
    }
    assert_eq!(
        audit_count(&state),
        before,
        "an unchanged save was audited as a change"
    );
}

/// The counterparts: a typed value is stored, an emptied box clears, and one
/// camera in a list can be replaced without disturbing the other.
#[tokio::test]
async fn a_typed_credential_replaces_the_stored_one_and_an_emptied_box_clears_it() {
    let state = station(false);
    let app = build_router(state.clone());
    let cookie = sign_in(&app).await;
    let mut fields = shown_fields(&app, &cookie).await;
    set_field(&mut fields, "email_smtp_pass", "brandNEWsmtp");
    set_field(&mut fields, "mqtt_password", "");
    set_field(
        &mut fields,
        "rtsp_urls",
        "rtsp://a:***REDACTED***@cam1.local/s,rtsp://c:newCANARYc@cam3.local/s",
    );

    let (_, reply) = post_settings(&app, &cookie, &fields).await;

    assert!(!reply.contains("Nothing was saved"), "{reply}");
    assert_eq!(stored(&state, "email_smtp_pass"), "brandNEWsmtp");
    assert_eq!(stored(&state, "mqtt_password"), "");
    assert_eq!(
        stored(&state, "rtsp_urls"),
        "rtsp://a:multiCANARYa@cam1.local/s,rtsp://c:newCANARYc@cam3.local/s"
    );
    // Untouched neighbours.
    assert_eq!(stored(&state, "birdweather_token"), "bwCANARYtoken");
    assert_eq!(
        stored(&state, "rtsp_url"),
        "rtsp://cam:rtspCANARYone@camera.local/stream"
    );
}

/// A mask is only ever "the value saved, unchanged". Edit the host and keep the
/// hidden password, and the station cannot know the password still fits.
#[tokio::test]
async fn a_mask_that_stands_for_nothing_is_refused_and_nothing_is_written() {
    let state = station(false);
    let app = build_router(state.clone());
    let cookie = sign_in(&app).await;
    let mut fields = shown_fields(&app, &cookie).await;
    set_field(&mut fields, "audio_format", "flac");
    set_field(
        &mut fields,
        "rtsp_url",
        "rtsp://cam:***REDACTED***@other.local/stream",
    );

    let (_, reply) = post_settings(&app, &cookie, &fields).await;

    assert!(reply.contains("Nothing was saved"), "{reply}");
    assert!(reply.contains("rtsp url"), "the field is named: {reply}");
    assert!(!reply.contains("rtspCANARYone"), "{reply}");
    assert_eq!(
        stored(&state, "rtsp_url"),
        "rtsp://cam:rtspCANARYone@camera.local/stream"
    );
    assert_eq!(
        stored(&state, "audio_format"),
        "",
        "the whole submission is refused"
    );

    // The same placeholder over a secret with nothing saved behind it.
    let state = station(false);
    state.with_db(|conn| {
        conn.execute("DELETE FROM settings WHERE key = 'email_smtp_pass'", [])
            .unwrap()
    });
    let app = build_router(state.clone());
    let cookie = sign_in(&app).await;
    let mut fields = shown_fields(&app, &cookie).await;
    set_field(&mut fields, "email_smtp_pass", MASK);
    let (_, reply) = post_settings(&app, &cookie, &fields).await;
    assert!(reply.contains("Nothing was saved"), "{reply}");
}

/// "Validate the submission, not the diff": a bad value already stored is found
/// again when its own mask is sent back, rather than waved through as unchanged.
#[tokio::test]
async fn a_bad_value_already_stored_is_found_again_when_its_mask_is_sent_back() {
    let state = station(false);
    state.with_db(|conn| {
        birdnet_db::settings::set(
            conn,
            "birdweather_token",
            "abc/123",
            birdnet_db::settings::SettingsCategory::Notifications,
        )
        .unwrap();
    });
    let app = build_router(state.clone());
    let cookie = sign_in(&app).await;
    let fields = shown_fields(&app, &cookie).await;
    assert_eq!(field(&fields, "birdweather_token"), MASK);

    let (_, reply) = post_settings(&app, &cookie, &fields).await;

    assert!(reply.contains("Nothing was saved"), "{reply}");
    assert!(reply.contains("BirdWeather token"), "{reply}");
    assert!(
        !reply.contains("abc/123"),
        "the value is not echoed: {reply}"
    );
    assert_eq!(stored(&state, "birdweather_token"), "abc/123");
}

fn stored_device(state: &AppState) -> String {
    state.with_db(|conn| conn.get("cam1").expect("read").expect("row").device_id)
}

/// The source list's edit form: the address comes back masked and a save of it
/// keeps the stored one; a typed address replaces it.
#[tokio::test]
async fn the_audio_source_edit_form_keeps_a_password_it_did_not_show() {
    let state = station(true);
    let app = build_router(state.clone());
    let cookie = sign_in(&app).await;
    let original = format!("rtsp://viewer:{SOURCE_PASSWORD}@camera.local/stream");

    let (status, form) = get(&app, "/admin/audio/sources/cam1/edit", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    let fields = submission(&form);
    assert_eq!(
        field(&fields, "device_id"),
        "rtsp://viewer:***REDACTED***@camera.local/stream"
    );
    let body = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&fields)
        .finish();
    let (status, reply) = write(
        &app,
        Method::PATCH,
        "/admin/audio/sources/cam1",
        &cookie,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(
        stored_device(&state),
        original,
        "the shown form was saved back"
    );
    assert!(
        !reply.contains(SOURCE_PASSWORD),
        "the row that comes back is masked too"
    );

    // A kept mask over another host is not a value.
    let body = "kind=rtsp&device_id=rtsp%3A%2F%2Fviewer%3A***REDACTED***%40other.local%2Fstream"
        .to_owned();
    let (_, reply) = write(
        &app,
        Method::PATCH,
        "/admin/audio/sources/cam1",
        &cookie,
        body,
    )
    .await;
    assert!(reply.contains("hidden-password placeholder"), "{reply}");
    assert_eq!(stored_device(&state), original);

    // Counterpart: a typed address is stored.
    let body =
        "kind=rtsp&device_id=rtsp%3A%2F%2Fviewer%3AnewCANARYpw%40camera.local%2Fstream".to_owned();
    let (status, _) = write(
        &app,
        Method::PATCH,
        "/admin/audio/sources/cam1",
        &cookie,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        stored_device(&state),
        "rtsp://viewer:newCANARYpw@camera.local/stream"
    );

    // And a new source cannot be added with the placeholder.
    let body = "kind=rtsp&device_id=rtsp%3A%2F%2Fa%3A***REDACTED***%40h%2Fs".to_owned();
    let (_, reply) = write(&app, Method::POST, "/admin/audio/sources", &cookie, body).await;
    assert!(reply.contains("placeholder"), "{reply}");
}
