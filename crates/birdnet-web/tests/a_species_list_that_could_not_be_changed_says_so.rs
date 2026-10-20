//! A species list that could not be read or written is not shown as changed,
//! and an unreadable list is not an empty one.
//!
//! `modify_list` read the stored list with `get(conn, key).ok()` — so a SQLite
//! error and "never written" looked the same — and wrote it back with
//! `set(..).ok()`. Two things followed from that:
//!
//! * a list the database could not read was taken to be empty, and the one name
//!   the operator had just added was written over the whole of it. They had
//!   forty excluded species; after one click they had one; and
//! * a write the database refused was dropped, while the audit log recorded the
//!   change and the Species storage page said "X will no longer be recorded".
//!
//! An unreadable value is made here with a BLOB: the `settings.value` column has
//! TEXT affinity, which converts numbers but leaves a BLOB alone, and
//! `settings::get` reads it as a `String` and so fails with
//! `InvalidColumnType` — an error that is not `NotFound`. A refused write is a
//! trigger.

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

const LIST: &str = "Turdus merula, Parus major, Erithacus rubecula";

/// A station whose `species_exclude` is `seed` (`None`: never written), stored
/// as a BLOB when `unreadable`, optionally refusing every write to `settings`.
fn station(seed: Option<&str>, unreadable: bool, refuse_writes: bool) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    if let Some(list) = seed {
        if unreadable {
            conn.execute(
                "INSERT INTO settings (key, value, category) VALUES ('species_exclude', ?1, 'species')",
                [list.as_bytes()],
            )
            .expect("seed blob");
        } else {
            conn.execute(
                "INSERT INTO settings (key, value, category) VALUES ('species_exclude', ?1, 'species')",
                [list],
            )
            .expect("seed");
        }
    }
    if refuse_writes {
        conn.execute_batch(
            "CREATE TRIGGER refuse_insert BEFORE INSERT ON settings
               BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;
             CREATE TRIGGER refuse_update BEFORE UPDATE ON settings
               BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
        )
        .expect("triggers");
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    form: &str,
) -> (u16, Option<String>, String) {
    let res = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let reswap = res
        .headers()
        .get("hx-reswap")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, reswap, String::from_utf8_lossy(&body).into_owned())
}

/// What is stored under `species_exclude`, and in which storage class.
fn stored(state: &AppState) -> Option<(String, String)> {
    state.with_db(|c| {
        c.query_row(
            "SELECT typeof(value), CAST(value AS TEXT) FROM settings WHERE key = 'species_exclude'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
    })
}

fn audited(state: &AppState) -> Vec<String> {
    state.with_db(|c| {
        let mut stmt = c
            .prepare("SELECT action FROM audit_log WHERE action LIKE 'species.%' ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    })
}

/// The three places that add a species to the exclusion list, as a
/// (route, form) pair: the Species lists partial, the Species storage table and
/// the suspect-species report.
const ADD_ROUTES: [(&str, &str); 3] = [
    ("/admin/species/exclude/add", "name=Passer+domesticus"),
    (
        "/admin/species/manage/exclude",
        "sci_name=Passer+domesticus",
    ),
    (
        "/admin/quality/phantoms/exclude",
        "sci_name=Passer+domesticus",
    ),
];

// ── a list that cannot be read ──────────────────────────────────────────

/// **The destructive one.** A list the database could not read must be left
/// exactly as it is, not replaced by the single name that was just added.
#[tokio::test]
async fn an_unreadable_list_is_not_overwritten_by_one_added_name() {
    for (uri, form) in ADD_ROUTES {
        let st = station(Some(LIST), true, false);
        let (status, reswap, body) = send(&st, "POST", uri, form).await;

        assert_eq!(
            stored(&st),
            Some(("blob".to_owned(), LIST.to_owned())),
            "{uri}: the operator's list was replaced"
        );
        assert_eq!(status, 200, "{uri}: htmx discards a 4xx/5xx body: {body}");
        assert_eq!(reswap.as_deref(), Some("none"), "{uri}: {body}");
        assert!(
            audited(&st).is_empty(),
            "{uri}: audited as a change that was not made: {:?}",
            audited(&st)
        );
        assert!(
            !body.contains("will no longer be recorded"),
            "{uri}: claimed the species is excluded: {body}"
        );
    }
}

/// The list partial and the Capture tab say they could not read the lists
/// rather than drawing three empty ones.
#[tokio::test]
async fn an_unreadable_list_is_not_drawn_as_an_empty_one() {
    let st = station(Some(LIST), true, false);
    let (status, _, body) = send(&st, "GET", "/admin/species/partial", "").await;
    assert_eq!(status, 200);
    assert!(
        body.contains("couldn't load"),
        "an unreadable list rendered as a list: {body}"
    );
}

// ── a write that is refused ─────────────────────────────────────────────

#[tokio::test]
async fn a_list_write_the_database_refused_is_not_reported_as_done() {
    for (uri, form) in ADD_ROUTES {
        let st = station(Some(LIST), false, true);
        let (status, reswap, body) = send(&st, "POST", uri, form).await;

        assert_eq!(status, 200, "{uri}: {body}");
        assert_eq!(
            reswap.as_deref(),
            Some("none"),
            "{uri}: shown as saved: {body}"
        );
        assert_eq!(
            stored(&st),
            Some(("text".to_owned(), LIST.to_owned())),
            "{uri}: the stored list changed"
        );
        assert!(
            audited(&st).is_empty(),
            "{uri}: audited as a change that was not made: {:?}",
            audited(&st)
        );
        assert!(
            !body.contains("will no longer be recorded"),
            "{uri}: claimed the species is excluded: {body}"
        );
        assert!(body.contains("not"), "{uri}: no toast: {body}");
    }
}

/// Removal is the same code path in the other direction.
#[tokio::test]
async fn a_removal_the_database_refused_is_not_reported_as_done() {
    let st = station(Some(LIST), false, true);
    let (status, reswap, body) = send(
        &st,
        "POST",
        "/admin/species/exclude/remove",
        "name=Parus+major",
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(reswap.as_deref(), Some("none"), "{body}");
    assert_eq!(stored(&st), Some(("text".to_owned(), LIST.to_owned())));
    assert!(audited(&st).is_empty(), "{:?}", audited(&st));
}

// ── the counterparts ────────────────────────────────────────────────────

/// A key that has never been written is an empty list, not an error: the first
/// exclusion on a new station must still work. This is the discrimination the
/// fix draws (`NotFound` is empty, anything else is a fault), so it has to stay
/// green or the fix would be a blanket refusal.
#[tokio::test]
async fn the_first_name_on_a_list_that_was_never_written_is_added() {
    for (uri, form) in ADD_ROUTES {
        let st = station(None, false, false);
        let (status, reswap, body) = send(&st, "POST", uri, form).await;
        assert_eq!(status, 200, "{uri}");
        assert_eq!(reswap, None, "{uri}: {body}");
        assert_eq!(
            stored(&st),
            Some(("text".to_owned(), "Passer domesticus".to_owned())),
            "{uri}"
        );
        assert_eq!(audited(&st), vec!["species.exclude.add".to_owned()]);
    }
}

/// A name added to a list that exists is appended to it, not substituted for it.
#[tokio::test]
async fn a_name_is_appended_to_the_list_that_is_there() {
    let st = station(Some(LIST), false, false);
    let (_, reswap, body) = send(
        &st,
        "POST",
        "/admin/species/exclude/add",
        "name=Passer+domesticus",
    )
    .await;
    assert_eq!(reswap, None, "{body}");
    assert_eq!(
        stored(&st),
        Some(("text".to_owned(), format!("{LIST}, Passer domesticus"),))
    );
    assert_eq!(audited(&st), vec!["species.exclude.add".to_owned()]);
}

/// And the Species storage action still says what it did when it did it.
#[tokio::test]
async fn the_species_storage_exclude_says_so_when_it_worked() {
    let st = station(Some(LIST), false, false);
    let (_, reswap, body) = send(
        &st,
        "POST",
        "/admin/species/manage/exclude",
        "sci_name=Passer+domesticus",
    )
    .await;
    assert_eq!(reswap, None, "{body}");
    assert!(
        body.contains("Passer domesticus will no longer be recorded."),
        "{body}"
    );
}
