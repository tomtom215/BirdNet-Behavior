//! The service worker does not keep what a private station should not have it
//! keep — the worker half of `a_private_station_does_not_let_a_browser_keep_its_pages.rs`.
//!
//! `static/sw.js` is evaluated as shipped (see `sw_harness.mjs`) against a
//! scripted network, because a grep for a string in the worker proves the string
//! is there and not that the worker behaves. Four things are asked of it:
//!
//! * a page marked `no-store` (what a private station now sends) is **not**
//!   stored, so sign-out leaves nothing to hand back;
//! * a response reached through a **redirect** is not stored either — the
//!   sign-in form a signed-out request for `/today` is redirected to used to be
//!   cached *as* `/today`, and shown to the same browser after it signed in;
//! * an ordinary public page **is** stored and **is** still served with the
//!   network gone, which is the offline behaviour of the PWA that must survive;
//! * `/admin` and `/login` never reach the worker's strategies;
//! * the one message it acts on, `SKIP_WAITING`, is honoured from a page of its
//!   own origin and from nowhere else (CodeQL `js/missing-origin-check`).
//!
//! Needs `node`. Where it is missing the test says so and passes, unless `CI` is
//! set, where a skip would be a gate that quietly stopped gating.

use std::process::Command;

fn run_harness() -> Option<serde_json::Value> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = match Command::new("node")
        .arg(dir.join("tests/sw_harness.mjs"))
        .arg(dir.join("static/sw.js"))
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "node is required in CI to run the service-worker harness: {e}"
            );
            eprintln!("skipped: node is not available ({e})");
            return None;
        }
    };
    assert!(
        output.status.success(),
        "the harness failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(serde_json::from_slice(&output.stdout).expect("the harness prints one JSON object"))
}

#[test]
fn the_worker_keeps_what_is_public_and_nothing_else() {
    let Some(out) = run_harness() else { return };

    // The counterpart first: without it, "kept nothing" below could be a worker
    // (or a harness) that keeps nothing at all.
    assert_eq!(
        out["public_page"]["kept"], 1,
        "an ordinary page should be cached: {out}"
    );
    assert_eq!(
        out["public_page"]["offline"], "<p>public dashboard</p>",
        "the PWA's offline copy of a public page is gone: {out}"
    );
    assert_eq!(out["static_asset"]["kept"], 1, "{out}");

    assert_eq!(
        out["private_page"]["kept"], 0,
        "a `no-store` page was cached, and would be served after sign-out: {out}"
    );
    assert!(
        out["private_page"]["offline"].is_null(),
        "a `no-store` page was served from cache: {out}"
    );

    assert_eq!(
        out["redirected_page"]["kept"], 0,
        "the page a redirect led to was cached under the URL that was asked for: {out}"
    );
    assert!(out["redirected_page"]["offline"].is_null(), "{out}");

    assert_eq!(out["admin_handled"], false, "{out}");
    assert_eq!(out["login_handled"], false, "{out}");
}

/// `SKIP_WAITING` promotes a waiting worker. It is honoured when the message's
/// origin is the worker's own, and not when it is another origin's or has none;
/// and the worker still ignores any other message, so the origin check is not
/// satisfied by a handler that ignores everything.
///
/// Observed failing against the worker before the check existed:
/// `from_another_origin` and `from_no_origin` were both 1.
#[test]
fn the_worker_skips_waiting_for_its_own_origin_only() {
    let Some(out) = run_harness() else { return };
    let skip = &out["skip_waiting"];
    assert_eq!(
        skip["from_own_origin"], 1,
        "the page that registered the worker can no longer ask it to update: {out}"
    );
    assert_eq!(
        skip["from_another_origin"], 0,
        "a message from another origin promoted a waiting worker: {out}"
    );
    assert_eq!(
        skip["from_no_origin"], 0,
        "a message with no origin promoted a waiting worker: {out}"
    );
    assert_eq!(skip["another_message"], 0, "{out}");
    assert_eq!(skip["no_data"], 0, "{out}");
}

/// The install-time precache holds static files and no page.
///
/// `Cache.addAll` stores whatever the server answers, `Cache-Control: no-store`
/// included, and the install runs in a browser that may be signed in. A `/` in
/// the list therefore put a private station's dashboard into Cache Storage
/// where it stayed after sign-out until the next release replaced the cache. It
/// was never read back (see the top of `static/sw.js`: the worker's scope is
/// `/static/`), so nothing served it, but it was on the disk. The real-browser
/// witness is `tools/visual-qa/sw-state.mjs` ("no HTML document is kept in Cache
/// Storage"); this is the check that needs no browser.
///
/// Observed failing against the worker that listed `'/'`: the first assertion.
#[test]
fn the_install_precache_holds_static_files_and_no_page() {
    let sw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static/sw.js"),
    )
    .expect("sw.js is readable");
    let precache = sw
        .split_once("const PRECACHE = [")
        .expect("sw.js has a PRECACHE list")
        .1
        .split_once("];")
        .expect("PRECACHE is closed")
        .0;
    let entries: Vec<&str> = precache
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && !l.is_empty())
        .map(|l| l.trim_end_matches(','))
        .collect();
    // The counterpart: a parse that found nothing would pass the loop below.
    assert!(entries.len() >= 5, "PRECACHE parsed to {entries:?}");
    for entry in entries {
        let url = entry.trim_matches(|c| c == '\'' || c == '`');
        assert!(
            url.starts_with("/static/"),
            "PRECACHE lists {url}, which is not a static file; Cache.addAll would keep the \
             page whatever its Cache-Control says"
        );
    }
}

/// The install precache asks for `<base>/static/…`, where `<base>` is the
/// reverse-proxy prefix the worker was served under.
///
/// With root-absolute URLs a station under `BIRDNET_BASE_PATH=/bn` made nine
/// requests to the *host's* `/static/…` (another application's, on a shared
/// host), `Cache.addAll` rejected on the first 404, and the install failed,
/// which discards the registration: nothing was registered, noisily. The root
/// deployment is the counterpart and must not gain a prefix.
///
/// Observed failing against a worker that precached `PRECACHE` unprefixed:
/// `under_bn` held `/static/…` URLs.
#[test]
fn the_install_precache_follows_the_base_path() {
    let Some(out) = run_harness() else { return };
    let urls = |key: &str| -> Vec<String> {
        out["precache"][key]
            .as_array()
            .unwrap_or_else(|| panic!("no precache list for {key}: {out}"))
            .iter()
            .map(|u| u.as_str().expect("a url").to_owned())
            .collect()
    };
    let root = urls("root");
    let bn = urls("under_bn");
    assert!(root.len() >= 5, "the install precached {root:?}");
    assert_eq!(root.len(), bn.len(), "{out}");
    assert!(
        root.iter().all(|u| u.starts_with("/static/")),
        "served from the root, the precache must stay at /static/: {root:?}"
    );
    assert!(
        bn.iter().all(|u| u.starts_with("/bn/static/")),
        "served from /bn/static/sw.js, every precache URL must be under /bn: {bn:?}"
    );
}
