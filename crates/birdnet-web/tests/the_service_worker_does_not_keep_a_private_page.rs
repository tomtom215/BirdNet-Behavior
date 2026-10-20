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
//! * `/admin` and `/login` never reach the worker's strategies.
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
