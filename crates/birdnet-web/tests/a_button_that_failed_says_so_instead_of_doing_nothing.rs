//! A state-changing button whose work failed must leave a mark on the page.
//!
//! # Why a 500 is invisible here
//!
//! Two facts, both read from what this crate actually ships:
//!
//! 1. `static/htmx.min.js` (htmx 2.0.4, unmodified) declares
//!    `responseHandling:[{code:"204",swap:false},{code:"[23]..",swap:true},
//!    {code:"[45]..",swap:false,error:true}]` — a 4xx or 5xx body is **never**
//!    swapped into the DOM, whatever it contains, including an out-of-band
//!    toast riding along in it.
//! 2. `templates/layout.html` installs the only fallback, on
//!    `htmx:responseError`, and it opens with `if (!isGet(evt)) return;` where
//!    `isGet` tests `evt.detail.requestConfig.verb === 'get'`.
//!
//! So a form post that fails produces nothing at all: no error, no success, no
//! change. The operator clicks "Delete", the row stays, and they cannot tell
//! whether the click registered, the delete failed, or the page is stale. That
//! is worse than an error, because there is nothing to react to.
//!
//! Each of these handlers already spoke through a toast on success and through
//! a discarded 5xx on failure — the channel that works was there all along,
//! used for the half that did not need it.
//!
//! # What is guarded
//!
//! Every enumerated state-changing endpoint, driven against a station whose
//! database cannot serve the write, must answer a status htmx will swap **and**
//! say something. The counterpart holds the same endpoints to a successful
//! 2xx on a working station, so "always answer 200 and say nothing useful"
//! does not pass.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// The state-changing endpoints reached by `hx-post`, with a body that would
/// succeed on a healthy station.
///
/// Listed by hand: the mapping from an `hx-post` attribute to the handler
/// behind it is not something a static scan can follow reliably, and a check
/// that quietly covered nothing would be worse than this list going stale.
const POSTS: &[(&str, &str)] = &[
    (
        "/admin/images/blacklist",
        "sci_name=Pica+pica&url=http%3A%2F%2Fexample.invalid%2Fa.jpg&reason=test",
    ),
    ("/admin/rules/1/delete", ""),
    ("/admin/rules/1/toggle", ""),
    // Creating a rule answered a refused insert with a bare 500.
    ("/admin/rules", "name=owls+at+night&action_type=log"),
];

/// A station whose rule and blacklist tables have been dropped: the connection
/// opens and every write to them fails, which is what a corrupt or
/// partially-restored database looks like from a handler.
fn station(break_writes: bool) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    if break_writes {
        for table in ["image_blacklist", "alert_rules"] {
            // Some of these may not exist under every schema revision; a table
            // that is already absent is just as broken for our purposes.
            let _ = conn.execute(&format!("DROP TABLE {table}"), []);
        }
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn post(state: &AppState, uri: &str, body: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body.to_owned()))
        .expect("request");
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn a_failed_write_answers_a_status_htmx_will_swap() {
    for &(uri, body) in POSTS {
        let (status, page) = post(&station(true), uri, body).await;
        assert!(
            status.is_success(),
            "POST {uri} answered {status} on failure. htmx discards a 4xx/5xx \
             body and the layout's fallback is GET-only, so the operator sees \
             nothing happen at all."
        );
        assert!(
            page.contains("bnb-toast") || page.contains("toast"),
            "POST {uri} failed silently: it answered {status} with nothing that \
             tells the reader. Body: {}",
            page.chars().take(200).collect::<String>()
        );
    }
}

/// The counterpart. Answering 200 with a toast on *every* request would pass
/// the test above while telling the operator a delete failed when it did not.
#[tokio::test]
async fn the_same_endpoints_still_succeed_on_a_working_station() {
    let state = station(false);
    // Give the rule endpoints something real to act on.
    let rule_exists = state
        .with_db(|conn| {
            conn.execute(
                "INSERT INTO alert_rules (id, name, enabled, action_type)
                 VALUES (1, 'test rule', 1, 'log')",
                [],
            )
        })
        .is_ok();

    assert!(
        rule_exists,
        "precondition: the fixture must be able to create a rule, or the rule \
         endpoints below are skipped and this counterpart covers one endpoint"
    );
    for &(uri, body) in POSTS {
        let (status, page) = post(&state, uri, body).await;
        assert!(
            status.is_success(),
            "POST {uri} answered {status} against a working station"
        );
        assert!(
            !page.contains("could not be"),
            "POST {uri} reported a failure against a working station. Body: {}",
            page.chars().take(200).collect::<String>()
        );
    }
}

/// `hx-delete` on a backup row answered a failed delete with a bare 500: the
/// row stayed and nothing said why. A backup that is not there is the
/// reachable failure; the answer must be one htmx swaps, with a toast.
#[tokio::test]
async fn a_backup_that_could_not_be_deleted_says_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    let conn = rusqlite::Connection::open(&db).expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    let state = AppState::from_connection(conn, db);
    let req = Request::builder()
        .method("DELETE")
        .uri("/admin/system/backups/birds.db.backup.1700000000")
        .body(Body::empty())
        .expect("request");
    let res = birdnet_web::server::build_router(state)
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    let page = String::from_utf8_lossy(&bytes);
    assert!(status.is_success(), "answered {status}");
    assert!(page.contains("toast"), "said nothing: {page}");
}
