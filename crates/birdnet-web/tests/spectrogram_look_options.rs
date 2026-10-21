//! The spectrogram's colour map and dynamic range are settings, and a change
//! reaches the next image the reader sees.
//!
//! # What is guarded
//!
//! 1. A station that has never chosen gets what every earlier version drew
//!    (viridis, 80 dB): the default is the explicit default, byte for byte.
//! 2. Each colour map paints its own endpoints. The darkest and the brightest
//!    cell of the surface are the table's first and last stop, so the image is
//!    made with the palette that was asked for and not merely a different one.
//! 3. The dynamic range changes the image (a narrower range clips the quiet end
//!    of the surface, so more of it is the floor colour).
//! 4. **A change must not serve a stale image.** The render is cached on the
//!    server and the browser: the server's key carries the look, and the
//!    response is revalidated (an `ETag` that moves with the look, no
//!    `max-age`) so a browser that fetched the old image an hour ago does not
//!    show it after the owner changed the setting.
//! 5. A value in the table that is no colour map or range (a hand-edited row,
//!    an older build) draws the default rather than failing.

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Two seconds of a falling tone over a little noise, at 48 kHz: a surface with
/// a loud ridge, a quiet floor and everything between, so the colour map and
/// the range both have something to act on.
fn chirp_wav() -> Vec<u8> {
    let rate: u32 = 48_000;
    let n = (rate * 2) as usize;
    let mut seed: u32 = 0x1234_5678;
    let mut data = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / rate as f32;
        let f = 9_000.0 - 3_000.0 * t;
        let env = (-(t - 1.0).abs() * 2.0).exp();
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (seed >> 16) as f32 / 65_535.0 - 0.5;
        let s = 0.6 * env * (std::f32::consts::TAU * f * t).sin() + 0.002 * noise;
        let v = (s * 32_767.0) as i16;
        data.extend_from_slice(&v.to_le_bytes());
    }
    let data_len = data.len() as u32;
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    w.extend_from_slice(&data);
    w
}

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("chirp.wav"), chirp_wav()).expect("write clip");
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
        .with_recording_dir(dir.path().to_path_buf());
    (dir, state)
}

fn store(state: &AppState, key: &str, value: &str) {
    state.with_db(|conn| {
        birdnet_db::settings::ensure_settings_table(conn).expect("table");
        birdnet_db::settings::set(
            conn,
            key,
            value,
            birdnet_db::settings::SettingsCategory::System,
        )
        .expect("set");
    });
}

async fn fetch(
    state: &AppState,
    uri: &str,
    if_none_match: Option<&str>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut req = Request::builder().uri(uri);
    if let Some(tag) = if_none_match {
        req = req.header(header::IF_NONE_MATCH, tag);
    }
    let res = build_router(state.clone())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .expect("response");
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 24)
        .await
        .expect("body")
        .to_vec();
    (status, headers, bytes)
}

const URI: &str = "/api/v2/spectrogram/chirp.wav";

async fn png(state: &AppState) -> Vec<u8> {
    let (status, _, body) = fetch(state, URI, None).await;
    assert_eq!(status, StatusCode::OK);
    body
}

/// The RGBA pixels of a PNG this crate's writer made (8-bit RGBA, filter 0).
fn pixels(png: &[u8]) -> Vec<[u8; 3]> {
    use std::io::Read as _;
    assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10], "not a PNG");
    let mut at = 8;
    let mut idat = Vec::new();
    let (mut w, mut h) = (0usize, 0usize);
    while at + 8 <= png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let kind = &png[at + 4..at + 8];
        let body = &png[at + 8..at + 8 + len];
        match kind {
            b"IHDR" => {
                w = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
                h = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
                assert_eq!((body[8], body[9]), (8, 6), "expected 8-bit RGBA");
            }
            b"IDAT" => idat.extend_from_slice(body),
            _ => {}
        }
        at += 12 + len;
    }
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&idat[..])
        .read_to_end(&mut raw)
        .expect("inflate");
    let stride = w * 4 + 1;
    assert_eq!(raw.len(), stride * h);
    let mut out = Vec::with_capacity(w * h);
    for row in 0..h {
        assert_eq!(raw[row * stride], 0, "writer uses filter 0");
        for x in 0..w {
            let p = row * stride + 1 + x * 4;
            out.push([raw[p], raw[p + 1], raw[p + 2]]);
        }
    }
    out
}

fn has(px: &[[u8; 3]], rgb: [u8; 3]) -> bool {
    px.contains(&rgb)
}

fn floor_share(px: &[[u8; 3]], floor: [u8; 3]) -> f64 {
    px.iter().filter(|p| **p == floor).count() as f64 / px.len() as f64
}

#[tokio::test]
async fn a_station_that_never_chose_gets_the_old_look() {
    let (_d, state) = station();
    let implicit = png(&state).await;
    store(&state, "spectrogram_colormap", "viridis");
    store(&state, "spectrogram_range_db", "80");
    let explicit = png(&state).await;
    assert_eq!(implicit, explicit, "the default is viridis at 80 dB");
    let px = pixels(&implicit);
    // The old viridis table's own ends.
    assert!(has(&px, [68, 1, 84]) && has(&px, [253, 231, 37]));
}

#[tokio::test]
async fn each_colour_map_paints_its_own_endpoints() {
    let (_d, state) = station();
    // (name, quietest cell, loudest cell): the first and last stop of each map.
    let maps: [(&str, [u8; 3], [u8; 3]); 5] = [
        ("viridis", [68, 1, 84], [253, 231, 37]),
        ("magma", [0, 0, 4], [252, 253, 191]),
        ("inferno", [0, 0, 4], [252, 255, 164]),
        ("grey", [0, 0, 0], [255, 255, 255]),
        ("grey_inverted", [255, 255, 255], [0, 0, 0]),
    ];
    let mut seen = Vec::new();
    for (name, quiet, loud) in maps {
        store(&state, "spectrogram_colormap", name);
        let image = png(&state).await;
        let px = pixels(&image);
        assert!(
            has(&px, quiet),
            "{name}: no cell is the quiet colour {quiet:?}"
        );
        assert!(
            has(&px, loud),
            "{name}: no cell is the loud colour {loud:?}"
        );
        assert!(
            !seen.contains(&image),
            "{name} drew the same image as an earlier map"
        );
        seen.push(image);
    }
}

#[tokio::test]
async fn a_narrower_range_clips_more_of_the_quiet_end() {
    let (_d, state) = station();
    store(&state, "spectrogram_colormap", "grey");
    let mut shares = Vec::new();
    for range in ["60", "80", "100", "120"] {
        store(&state, "spectrogram_range_db", range);
        let px = pixels(&png(&state).await);
        shares.push((range, floor_share(&px, [0, 0, 0])));
    }
    for pair in shares.windows(2) {
        assert!(
            pair[0].1 > pair[1].1,
            "{} dB should leave more floor than {} dB: {shares:?}",
            pair[0].0,
            pair[1].0
        );
    }
}

#[tokio::test]
async fn a_change_is_not_answered_from_either_cache() {
    let (_d, state) = station();
    let (status, headers, first) = fetch(&state, URI, None).await;
    assert_eq!(status, StatusCode::OK);
    let cc = headers[header::CACHE_CONTROL].to_str().unwrap().to_owned();
    assert!(cc.contains("private"), "{cc}");
    assert!(
        !cc.contains("max-age=3600") && !cc.contains("immutable"),
        "a browser told it may keep the image an hour shows the old look after the owner changes it: {cc}"
    );
    let old_tag = headers[header::ETAG].to_str().unwrap().to_owned();

    // Unchanged: revalidation is answered without a body.
    let (status, _, body) = fetch(&state, URI, Some(&old_tag)).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());

    // Changed: the old tag no longer matches, and the server's own cache does not
    // hand back the image drawn under the old look.
    store(&state, "spectrogram_colormap", "magma");
    let (status, headers, second) = fetch(&state, URI, Some(&old_tag)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the old ETag must not validate the new look"
    );
    assert_ne!(
        first, second,
        "the server answered from a cache keyed without the look"
    );
    let new_tag = headers[header::ETAG].to_str().unwrap().to_owned();
    assert_ne!(old_tag, new_tag);

    store(&state, "spectrogram_range_db", "100");
    let (_, headers, third) = fetch(&state, URI, None).await;
    assert_ne!(second, third, "the range is part of the cache key too");
    assert_ne!(new_tag, headers[header::ETAG].to_str().unwrap());

    // And back: the first look is drawn again, identically.
    store(&state, "spectrogram_colormap", "viridis");
    store(&state, "spectrogram_range_db", "80");
    assert_eq!(png(&state).await, first);
}

#[tokio::test]
async fn a_thumbnail_follows_the_look_too() {
    let (_d, state) = station();
    let uri = "/api/v2/spectrogram/chirp.wav?thumb=1";
    let (_, _, before) = fetch(&state, uri, None).await;
    store(&state, "spectrogram_colormap", "inferno");
    let (_, _, after) = fetch(&state, uri, None).await;
    assert_ne!(before, after);
}

#[tokio::test]
async fn a_stored_value_that_is_no_choice_draws_the_default() {
    let (_d, state) = station();
    let default = png(&state).await;
    store(&state, "spectrogram_colormap", "rainbow");
    store(&state, "spectrogram_range_db", "7");
    assert_eq!(png(&state).await, default);
}
