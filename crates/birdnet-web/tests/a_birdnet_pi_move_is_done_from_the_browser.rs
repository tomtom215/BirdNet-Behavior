//! The move from BirdNET-Pi, driven over HTTP as the Station, Data page drives it.
//!
//! Three steps, each previewed before it is applied: the settings
//! (`birdnet.conf` and the files beside it), the clips of the imported
//! detections, and the guard rails around both. The unit tests in
//! `routes/admin/migration/` hold the pieces; this holds that they are
//! *connected*: a preview that shows a plan the Apply button does not follow, a
//! secret that leaks into the page, a copy that runs without being asked, would
//! each pass those and fail here.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const BOUNDARY: &str = "----bnbtestboundary";

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

thread_local! {
    /// One router for the whole test, as the running station has: the preview
    /// waits in a slot the router owns, so a router built per request would
    /// never find it again. A test runs on one thread, and builds one station.
    static APP: std::cell::RefCell<Option<axum::Router>> = const { std::cell::RefCell::new(None) };
}

async fn send(state: &AppState, req: Request<Body>) -> (StatusCode, String) {
    let app = APP.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| build_router(state.clone()))
            .clone()
    });
    let res = app
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 8 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A multipart body: `(field, optional file name, content)`.
fn multipart(parts: &[(&str, Option<&str>, &str)]) -> Body {
    let mut out = String::new();
    for (name, file, content) in parts {
        out.push_str(&format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\""));
        if let Some(file) = file {
            out.push_str(&format!("; filename=\"{file}\"\r\nContent-Type: text/plain"));
        }
        out.push_str(&format!("\r\n\r\n{content}\r\n"));
    }
    out.push_str(&format!("--{BOUNDARY}--\r\n"));
    Body::from(out)
}

async fn preview(state: &AppState, parts: &[(&str, Option<&str>, &str)]) -> String {
    let req = Request::builder()
        .method("POST")
        .uri("/admin/migrate/conf/preview")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(multipart(parts))
        .expect("request");
    let (status, body) = send(state, req).await;
    assert_eq!(status, StatusCode::OK);
    body
}

async fn apply(state: &AppState, token: &str) -> String {
    let req = Request::builder()
        .method("POST")
        .uri("/admin/migrate/conf/apply")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("token={token}")))
        .expect("request");
    let (status, body) = send(state, req).await;
    assert_eq!(status, StatusCode::OK);
    body
}

fn token_of(html: &str) -> String {
    let at = html.find("&quot;token&quot;: ").or_else(|| html.find("\"token\": ")).expect("an Apply button with a token");
    html[at..]
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect()
}

fn setting(state: &AppState, key: &str) -> Option<String> {
    state.with_db(|conn| {
        birdnet_db::settings::get_or(conn, key, "\u{0}")
            .ok()
            .filter(|v| v != "\u{0}")
    })
}

fn rows(state: &AppState) -> usize {
    state.with_db(|conn| {
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
            .expect("count");
        usize::try_from(n).expect("a count is not negative")
    })
}

const CONF: &str = "LATITUDE=52.09\nLONGITUDE=5.0\nCONFIDENCE=0.65\nFULL_DISK=keep\n\
                    PURGE_THRESHOLD=90\nBIRDWEATHER_ID=SECRETbwid42\nAUDIOFMT=mp3\n\
                    APPRISE_NOTIFY_EACH_DETECTION=1\nAPPRISE_NOTIFY_NEW_SPECIES=1\n\
                    APPRISE_NOTIFY_NEW_SPECIES_EACH_DAY=0\n\
                    APPRISE_ONLY_NOTIFY_SPECIES_NAMES=\"House Sparrow\"\nCADDY_PWD=hunter2pass\n\
                    FLICKR_API_KEY=flickrSECRET\nIMAGE_PROVIDER=FLICKR\n";

/// The preview changes nothing and shows no credential; Apply then writes
/// exactly the settings the preview listed, through the settings validation.
#[tokio::test]
async fn the_preview_changes_nothing_and_apply_writes_what_it_showed() {
    let (_d, state) = station();
    let before = rows(&state);
    let html = preview(
        &state,
        &[
            ("conf_files", Some("birdnet.conf"), CONF),
            ("conf_files", Some("apprise.txt"), "discord://1/DISCORDTOKEN\n"),
            ("conf_files", Some("exclude_species_list.txt"), "Passer domesticus_House Sparrow\n"),
            ("conf_text", None, ""),
            ("source_dir", None, ""),
        ],
    )
    .await;
    assert_eq!(rows(&state), before, "a preview wrote a setting");
    assert!(html.contains("Nothing has been changed yet"), "{html}");
    for secret in ["SECRETbwid42", "hunter2pass", "flickrSECRET", "DISCORDTOKEN"] {
        assert!(!html.contains(secret), "the preview shows {secret}");
    }
    // The rows the owner has to read.
    // Names carry a break opportunity after each underscore; the words are the same.
    let plain = html.replace("<wbr>", "");
    assert!(plain.contains("birdweather_token"), "{html}");
    assert!(html.contains("(hidden, 12 characters)"), "{html}");
    assert!(plain.contains("notify_species_exclude"), "the exclusion list went to the exclusion setting");
    assert!(html.contains("each,new-species"), "the switches became a list of rules");

    let done = apply(&state, &token_of(&html)).await;
    for secret in ["SECRETbwid42", "hunter2pass", "flickrSECRET", "DISCORDTOKEN"] {
        assert!(!done.contains(secret), "the result shows {secret}");
    }
    assert!(done.contains("Done."), "{done}");
    assert_eq!(setting(&state, "confidence_threshold").as_deref(), Some("0.65"));
    assert_eq!(setting(&state, "full_disk_action").as_deref(), Some("keep"));
    assert_eq!(setting(&state, "purge_threshold").as_deref(), Some("90"));
    assert_eq!(setting(&state, "birdweather_token").as_deref(), Some("SECRETbwid42"));
    assert_eq!(setting(&state, "audio_format").as_deref(), Some("mp3"));
    assert_eq!(setting(&state, "notify_trigger").as_deref(), Some("each,new-species"));
    assert_eq!(setting(&state, "notify_confidence").as_deref(), Some("0"));
    assert_eq!(setting(&state, "notify_species_exclude").as_deref(), Some("House Sparrow"));
    assert_eq!(setting(&state, "notify_species_only"), None);
    assert_eq!(setting(&state, "species_exclude").as_deref(), Some("Passer domesticus"));
    let apprise = setting(&state, "apprise_config").expect("the apprise file is pointed at");
    assert!(std::fs::read_to_string(&apprise).expect("the file").contains("DISCORDTOKEN"));
    // The password is not a setting and is not stored anywhere an import wrote.
    assert!(state.with_db(|conn| {
        let mut stmt = conn.prepare("SELECT value FROM settings").expect("prepare");
        stmt.query_map([], |r| r.get::<_, String>(0))
            .expect("query")
            .flatten()
            .all(|v| !v.contains("hunter2pass"))
    }));
    // The disk policy waits for a restart and the page says so.
    assert!(done.contains("need a restart"), "{done}");
}

/// A token belongs to the preview it came with; a second preview replaces the
/// first and the first's button stops working.
#[tokio::test]
async fn a_stale_or_made_up_token_applies_nothing() {
    let (_d, state) = station();
    let first = preview(&state, &[("conf_files", Some("birdnet.conf"), CONF)]).await;
    let first_token = token_of(&first);
    let second = preview(&state, &[("conf_files", Some("birdnet.conf"), "CONFIDENCE=0.5\n")]).await;
    let second_token = token_of(&second);
    assert_ne!(first_token, second_token);

    let before = rows(&state);
    assert!(apply(&state, &first_token).await.contains("no longer waiting"));
    assert!(apply(&state, "999999").await.contains("no longer waiting"));
    assert_eq!(rows(&state), before);

    // The second still works, once.
    assert!(apply(&state, &second_token).await.contains("Done."));
    assert_eq!(setting(&state, "confidence_threshold").as_deref(), Some("0.5"));
    assert!(apply(&state, &second_token).await.contains("no longer waiting"), "applied twice");
}

/// Nothing handed over is said, not previewed as an empty plan.
#[tokio::test]
async fn nothing_to_read_is_said() {
    let (_d, state) = station();
    let html = preview(&state, &[("conf_files", Some(""), ""), ("conf_text", None, "   ")]).await;
    assert!(html.contains("Choose your birdnet.conf"), "{html}");
    let html = preview(&state, &[("source_dir", None, "relative/path")]).await;
    assert!(html.contains("not a full path"), "{html}");
}

/// A value the Settings page would refuse is refused in the preview, and
/// nothing is applied for it.
#[tokio::test]
async fn the_percentage_slip_is_refused_in_the_preview() {
    let (_d, state) = station();
    let html = preview(&state, &[("conf_files", Some("birdnet.conf"), "CONFIDENCE=75\nSENSITIVITY=1.2\n")]).await;
    assert!(html.contains("Not accepted here"), "{html}");
    assert!(html.contains("if you meant 75%, enter 0.75"), "{html}");
    let done = apply(&state, &token_of(&html)).await;
    assert!(done.contains("Done."), "{done}");
    assert_eq!(setting(&state, "confidence_threshold"), None, "the refused value was stored");
    assert_eq!(setting(&state, "sensitivity").as_deref(), Some("1.2"), "the good one was not held back");
}

/// Species lists are added to.
#[tokio::test]
async fn a_species_list_is_added_to_not_replaced() {
    let (_d, state) = station();
    state.with_db(|conn| {
        birdnet_db::settings::set(
            conn,
            "species_exclude",
            "Columba livia",
            birdnet_db::settings::SettingsCategory::Species,
        )
        .expect("set");
    });
    let html = preview(
        &state,
        &[("conf_files", Some("exclude_species_list.txt"), "Passer domesticus_House Sparrow\nColumba livia_Rock Dove\n")],
    )
    .await;
    assert!(html.contains("1 of them is new"), "{html}");
    apply(&state, &token_of(&html)).await;
    assert_eq!(
        setting(&state, "species_exclude").as_deref(),
        Some("Columba livia, Passer domesticus")
    );
}

// ---- the recordings --------------------------------------------------------

fn pi_tree(root: &std::path::Path) -> Vec<(String, String, String)> {
    // (date, common name, file name)
    let species = [("Eurasian Blackbird", "Eurasian_Blackbird"), ("Cetti's Warbler", "Cettis_Warbler")];
    let mut rows = Vec::new();
    for (i, (com, dir)) in species.iter().enumerate() {
        let date = format!("2026-03-0{}", i + 1);
        let name = format!("{dir}-80-{date}-birdnet-06:00:0{i}.wav");
        let folder = root.join("Extracted/By_Date").join(&date).join(dir);
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join(&name), format!("audio-{i}")).expect("clip");
        std::fs::write(folder.join(format!("{name}.png")), b"png").expect("picture");
        rows.push((date, (*com).to_owned(), name));
    }
    rows
}

fn import_rows(state: &AppState, rows: &[(String, String, String)]) {
    state.with_db(|conn| {
        conn.execute("INSERT INTO import_batches (source_kind) VALUES ('birdnet-pi')", [])
            .expect("batch");
        for (i, (date, com, name)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, \
                 Sens, Overlap, File_Name, chunk_offset_secs, import_batch_id) \
                 VALUES (?1, ?2, ?3, ?3, 0.9, 0.7, 9, 1.25, 0.0, ?4, 0, 1)",
                rusqlite::params![date, format!("06:00:0{i}"), com, name],
            )
            .map(|_| ())
            .expect("insert");
        }
    });
}

async fn post_form(state: &AppState, uri: &str, body: &str) -> String {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body.to_owned()))
        .expect("request");
    let (status, html) = send(state, req).await;
    assert_eq!(status, StatusCode::OK, "{uri}");
    html
}

fn encode(s: &str) -> String {
    form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[tokio::test]
async fn the_recordings_are_checked_first_then_brought_across_and_play() {
    let (_d, state) = station();
    let pi = tempfile::tempdir().expect("pi");
    let rows = pi_tree(pi.path());
    import_rows(&state, &rows);
    let source = encode(&pi.path().display().to_string());
    let recordings = state.recording_dir();

    let checked = post_form(
        &state,
        "/admin/migrate/clips/preview",
        &format!("clips_source={source}&clips_mode=copy"),
    )
    .await;
    assert!(checked.contains("Nothing has been changed yet"), "{checked}");
    assert!(checked.contains("<strong>2</strong> recordings would be copied"), "{checked}");
    assert!(
        std::fs::read_dir(&recordings).map_or(true, |d| d.count() == 0),
        "the check wrote a file"
    );

    let started = post_form(
        &state,
        "/admin/migrate/clips/run",
        &format!("clips_source={source}&clips_mode=copy"),
    )
    .await;
    assert!(started.contains("clips-progress"), "{started}");
    // Wait for the job, by polling the way the page does.
    let mut finished = String::new();
    for _ in 0..100 {
        let req = Request::builder()
            .uri("/admin/migrate/progress?for=clips")
            .body(Body::empty())
            .expect("request");
        let (_, html) = send(&state, req).await;
        if html.contains("bar-msg ok") || html.contains("bar-msg err") {
            finished = html;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(finished.contains("2 recordings copied"), "{finished}");
    for (_, _, name) in &rows {
        assert!(recordings.join(name).is_file(), "{name} did not arrive");
        // And it plays: served by the route the player uses (`/api/v2/recordings/..`).
        let req = Request::builder()
            .uri(format!("/api/v2/recordings/{}", encode(name)))
            .body(Body::empty())
            .expect("request");
        let (status, body) = send(&state, req).await;
        assert_eq!(status, StatusCode::OK, "{name} does not play: {body}");
    }
    // Resumable: run again, nothing left to do.
    let again = post_form(
        &state,
        "/admin/migrate/clips/preview",
        &format!("clips_source={source}&clips_mode=copy"),
    )
    .await;
    assert!(again.contains("<strong>0</strong> recordings would be copied"), "{again}");
    assert!(again.contains("2 are already here"), "{again}");
}

#[tokio::test]
async fn a_folder_that_is_not_a_clip_tree_is_said_and_a_relative_path_refused() {
    let (_d, state) = station();
    let nothing = tempfile::tempdir().expect("dir");
    let html = post_form(
        &state,
        "/admin/migrate/clips/preview",
        &format!("clips_source={}&clips_mode=copy", encode(&nothing.path().display().to_string())),
    )
    .await;
    assert!(html.contains("By_Date"), "{html}");
    let html = post_form(&state, "/admin/migrate/clips/preview", "clips_source=sub%2Fdir&clips_mode=copy").await;
    assert!(html.contains("not a full path"), "{html}");
    let html = post_form(&state, "/admin/migrate/clips/preview", "clips_source=%2Ftmp&clips_mode=bogus").await;
    assert!(html.contains("Choose what to do"), "{html}");
}

/// The Data page offers all three steps.
#[tokio::test]
async fn the_data_page_offers_the_three_steps() {
    let (_d, state) = station();
    let req = Request::builder().uri("/station/data").body(Body::empty()).expect("request");
    let (status, html) = send(&state, req).await;
    assert_eq!(status, StatusCode::OK);
    for needle in ["Bring your BirdNET-Pi settings", "conf-files", "Your recordings", "clips-source"] {
        assert!(html.contains(needle), "the Data page lacks {needle}");
    }
}
