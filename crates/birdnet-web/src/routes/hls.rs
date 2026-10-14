//! Live audio as HLS (`G-21`): `/api/v2/streams/hls/*`.
//!
//! `/stream` is MP3 over one long HTTP response, which is simple and right on
//! a LAN, and has no recovery: on a phone, a network handover ends the
//! response and the tab goes silent. HLS is a playlist of short segments that
//! the player fetches one at a time and refetches when the network comes
//! back, which is what mobile browsers are built around. Safari, iOS and
//! Android Chrome play it natively.
//!
//! birdnet-go's API, so a client written for either station reads both:
//!
//! | Request | Answer |
//! |---|---|
//! | `POST /streams/hls/{source_id}/start` | the stream's token and playlist URL, once two segments exist (or `starting` after 20 s) |
//! | `POST /streams/hls/heartbeat` | `{"status": "ok"}`; `?disconnect=true` is a stop |
//! | `POST /streams/hls/{source_id}/stop` | the caller's session ends; the stream ends with its last |
//! | `GET /streams/hls/status` | every stream running |
//! | `GET /streams/hls/t/{token}/playlist.m3u8` and `…/{segment}` | the stream itself |
//!
//! One `ffmpeg` per source and pitch shift, shared by everyone listening to
//! it and counted against the same four stream slots as `/stream`; upstream
//! has no pitch shift, and here a listener's shift is theirs (see the Live
//! page), so a shifted stream is a stream of its own. `source_id` is an
//! `audio_sources` id, or `default` for the station's default source.
//!
//! A stream lives while it is asked for: any request for its playlist or a
//! segment, or a heartbeat, keeps it; a minute without one, or its `ffmpeg`
//! exiting, ends it and deletes its segments. Its token is unguessable (128
//! random bits) and the only way in.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get, routing::post};
use serde::Deserialize;
use serde_json::json;

use crate::routes::livestream::{Encoder, start_encoder};
use crate::state::AppState;

/// Seconds of audio in a segment: upstream's default.
const SEGMENT_SECS: u32 = 2;
/// Segments the playlist lists: upstream's `hlsListSize`.
const LIST_SIZE: u32 = 6;
/// Segments a playlist must list before it is reported ready: upstream's
/// `hlsMinSegments`. One segment starts playback with no buffer behind it.
const READY_SEGMENTS: usize = 2;
/// How long a start waits for the playlist to be ready: upstream's
/// `hlsPlaylistWaitTimeout`.
const READY_WAIT: Duration = Duration::from_secs(20);
/// How often the wait looks.
const READY_POLL: Duration = Duration::from_millis(500);
/// A stream nobody has asked for in this long is ended. A playing client asks
/// for the playlist every segment; this is a network handover's worth of
/// silence, not a listener's.
const IDLE_LIMIT: Duration = Duration::from_secs(60);
/// How often a stream's watchdog looks.
const WATCH_EVERY: Duration = Duration::from_secs(5);
/// A session that has not been seen in this long no longer counts as a
/// listener in `active_clients`.
const SESSION_LIMIT: Duration = Duration::from_secs(60);

/// A stream directory not modified in this long has no encoder behind it.
/// A running `ffmpeg` writes a segment and deletes the oldest every
/// [`SEGMENT_SECS`], each of which changes the directory; one that has sat
/// for five minutes belongs to a process that died before it could end the
/// stream — killed, or crashed — and nothing else will ever delete it.
const ABANDONED_AFTER: Duration = Duration::from_secs(300);

/// What a stream is: one source, at one pitch shift.
type StreamKey = (String, i32);

/// One running HLS stream.
struct HlsStream {
    key: StreamKey,
    token: String,
    dir: PathBuf,
    /// The encoder and its slot; `None` once the stream has ended.
    encoder: Mutex<Option<Encoder>>,
    /// When anything last asked for this stream.
    last_activity: Mutex<Instant>,
    /// Session id → when it was last seen.
    sessions: Mutex<HashMap<String, Instant>>,
}

impl HlsStream {
    fn touch(&self) {
        *lock(&self.last_activity) = Instant::now();
    }

    fn see(&self, session: Option<&str>) {
        self.touch();
        if let Some(session) = session {
            lock(&self.sessions).insert(session.to_owned(), Instant::now());
        }
    }

    fn active_clients(&self) -> usize {
        lock(&self.sessions)
            .values()
            .filter(|seen| seen.elapsed() < SESSION_LIMIT)
            .count()
    }

    fn playlist(&self) -> PathBuf {
        self.dir.join("playlist.m3u8")
    }

    fn ready(&self) -> bool {
        playlist_ready(&self.playlist())
    }
}

/// A poisoned lock here guards plain bookkeeping, which stays usable.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Every running stream, by key and by token.
#[derive(Default)]
struct Streams {
    by_key: HashMap<StreamKey, Arc<HlsStream>>,
    by_token: HashMap<String, Arc<HlsStream>>,
}

static STREAMS: LazyLock<Mutex<Streams>> = LazyLock::new(Mutex::default);

/// The running stream for a source and shift.
fn find_by_key(key: &StreamKey) -> Option<Arc<HlsStream>> {
    lock(&STREAMS).by_key.get(key).cloned()
}

/// The running stream a token names.
fn find_by_token(token: &str) -> Option<Arc<HlsStream>> {
    lock(&STREAMS).by_token.get(token).cloned()
}

/// Held while a stream is being created, so two starts for the same source
/// do not start two encoders.
static CREATING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

/// End a stream: forget it, stop its encoder and delete its segments.
fn end(stream: &HlsStream, why: &str) {
    {
        let mut streams = lock(&STREAMS);
        if streams
            .by_token
            .get(&stream.token)
            .is_some_and(|s| std::ptr::eq(s.as_ref(), stream))
        {
            streams.by_token.remove(&stream.token);
            streams.by_key.remove(&stream.key);
        }
    }
    lock(&stream.encoder).take();
    let _ = std::fs::remove_dir_all(&stream.dir);
    tracing::info!(source = %stream.key.0, why, "HLS stream ended");
}

/// Whether a playlist lists at least [`READY_SEGMENTS`] segments.
fn playlist_ready(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| {
        text.lines().filter(|l| l.starts_with("#EXTINF")).count() >= READY_SEGMENTS
    })
}

/// 128 random bits as 32 hex digits.
fn new_token() -> Option<String> {
    use password_hash::rand_core::{OsRng, RngCore};
    let mut bytes = [0u8; 16];
    OsRng.try_fill_bytes(&mut bytes).ok()?;
    Some(bytes.iter().fold(String::with_capacity(32), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    }))
}

/// Delete the stream directories under `root` that no stream of this process
/// owns and that nothing has modified in `older_than`. Only names a token
/// could have are touched.
fn sweep_abandoned(root: &Path, older_than: Duration) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.len() != 32
            || !name.bytes().all(|b| b.is_ascii_hexdigit())
            || find_by_token(name).is_some()
        {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= older_than);
        if stale && std::fs::remove_dir_all(entry.path()).is_ok() {
            tracing::info!(dir = %entry.path().display(), "HLS: removed an abandoned stream's segments");
        }
    }
}

/// Whether `name` is a file the encoder writes: the playlist, or a segment
/// `seg` + digits + `.ts`. Nothing else is served, so no request can name a
/// path outside the stream's directory.
fn servable(name: &str) -> bool {
    name == "playlist.m3u8"
        || name
            .strip_prefix("seg")
            .and_then(|rest| rest.strip_suffix(".ts"))
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// The encoder's output arguments for a stream in `dir`.
fn hls_output_args(dir: &Path) -> Vec<String> {
    let mut args: Vec<String> = [
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        "-ar",
        "48000",
        "-ac",
        "1",
        "-f",
        "hls",
        "-hls_time",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    args.push(SEGMENT_SECS.to_string());
    args.extend(["-hls_list_size".to_owned(), LIST_SIZE.to_string()]);
    // Old segments deleted; no end marker, as the stream is live; the
    // playlist replaced whole, so a read never sees half of one; and every
    // segment starting cleanly, so a player can join at any of them.
    args.extend([
        "-hls_flags".to_owned(),
        "delete_segments+omit_endlist+temp_file+independent_segments".to_owned(),
        "-hls_segment_filename".to_owned(),
        dir.join("seg%05d.ts").display().to_string(),
        dir.join("playlist.m3u8").display().to_string(),
    ]);
    args
}

/// The routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/streams/hls/{source_id}/start", post(start))
        .route("/streams/hls/{source_id}/stop", post(stop))
        .route("/streams/hls/heartbeat", post(heartbeat))
        .route("/streams/hls/status", get(status))
        .route("/streams/hls/t/{token}/{file}", get(content))
}

/// The playlist's URL for a token, as upstream builds it.
fn playlist_url(token: &str) -> String {
    format!("/api/v2/streams/hls/t/{token}/playlist.m3u8")
}

/// A stream's status, in upstream's `HLSStreamStatus` shape.
fn describe(stream: &HlsStream, status: &str, ready: bool, with_token: bool) -> serde_json::Value {
    let mut o = json!({
        "status": status,
        "source": stream.key.0,
        "playlist_url": playlist_url(&stream.token),
        "active_clients": stream.active_clients(),
        "playlist_ready": ready,
        "freq_shift_hz": stream.key.1,
    });
    if with_token {
        o["stream_token"] = json!(stream.token);
    }
    o
}

/// Query parameters for a start.
#[derive(Debug, Deserialize)]
struct StartQuery {
    #[serde(default)]
    freq_shift_hz: i32,
}

/// An optional body naming the browser tab: upstream's `HLSSessionRequest`.
#[derive(Debug, Default, Deserialize)]
struct SessionBody {
    session_id: Option<String>,
}

/// A session id from a body that may be absent or not JSON at all. Bounded,
/// so a client cannot grow the session map with long keys.
fn session_of(body: &axum::body::Bytes) -> Option<String> {
    serde_json::from_slice::<SessionBody>(body)
        .ok()
        .and_then(|b| b.session_id)
        .filter(|s| !s.is_empty() && s.len() <= 64)
}

/// The running stream for a source and shift, started if there is none.
async fn join_or_start(
    state: &AppState,
    source_id: &str,
    shift: i32,
) -> Result<Arc<HlsStream>, Box<Response>> {
    let key: StreamKey = (source_id.to_owned(), shift);
    // Held until the new stream is registered, so two starts for the same
    // source do not start two encoders.
    let _creating = CREATING.lock().await;
    if let Some(stream) = find_by_key(&key) {
        return Ok(stream);
    }
    let Some(token) = new_token() else {
        return Err(Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "no random token available",
            )
                .into_response(),
        ));
    };
    let root = std::env::temp_dir().join("birdnet-hls");
    sweep_abandoned(&root, ABANDONED_AFTER);
    let dir = root.join(&token);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::error!(error = %e, dir = %dir.display(), "HLS: cannot create the segment directory");
        return Err(Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "cannot create the stream's directory",
            )
                .into_response(),
        ));
    }
    let args = hls_output_args(&dir);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let by_id = (source_id != "default").then(|| source_id.to_owned());
    let encoder = match start_encoder(state, by_id, shift, &args, std::process::Stdio::null()).await
    {
        Ok(encoder) => encoder,
        Err(response) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(response);
        }
    };
    let stream = Arc::new(HlsStream {
        key: key.clone(),
        token: token.clone(),
        dir,
        encoder: Mutex::new(Some(encoder)),
        last_activity: Mutex::new(Instant::now()),
        sessions: Mutex::new(HashMap::new()),
    });
    {
        let mut streams = lock(&STREAMS);
        streams.by_key.insert(key, Arc::clone(&stream));
        streams.by_token.insert(token, Arc::clone(&stream));
    }
    tokio::spawn(watch(Arc::downgrade(&stream), WATCH_EVERY, IDLE_LIMIT));
    Ok(stream)
}

/// `POST /api/v2/streams/hls/{source_id}/start?freq_shift_hz=` — join the
/// source's stream, starting it if nobody is listening.
async fn start(
    State(state): State<AppState>,
    UrlPath(source_id): UrlPath<String>,
    Query(q): Query<StartQuery>,
    body: axum::body::Bytes,
) -> Response {
    let session = session_of(&body);
    let shift = q.freq_shift_hz.clamp(
        -crate::routes::livestream::MAX_STREAM_SHIFT_HZ,
        crate::routes::livestream::MAX_STREAM_SHIFT_HZ,
    );
    let stream = match join_or_start(&state, &source_id, shift).await {
        Ok(stream) => stream,
        Err(response) => return *response,
    };
    stream.see(session.as_deref());

    let deadline = Instant::now() + READY_WAIT;
    let ready = loop {
        if stream.ready() {
            break true;
        }
        if Instant::now() >= deadline || lock(&stream.encoder).is_none() {
            break false;
        }
        tokio::time::sleep(READY_POLL).await;
    };
    if lock(&stream.encoder).is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "the stream ended before it started; see the station's log for ffmpeg's reason",
        )
            .into_response();
    }
    let body = describe(
        &stream,
        if ready { "ready" } else { "starting" },
        ready,
        true,
    );
    drop(stream);
    Json(body).into_response()
}

/// End a stream once nobody has asked for it in `idle` ([`IDLE_LIMIT`]), or
/// its encoder has exited, looking every `every` ([`WATCH_EVERY`]). Holds only
/// a weak reference, so a stream ended elsewhere is not kept alive by its
/// watchdog.
///
/// This is the only thing that ends a stream nobody stops, and ending it is
/// what releases everything else: dropping the encoder kills `ffmpeg`
/// (`kill_on_drop`), the tap pump's next write fails, and its reader thread
/// exits.
async fn watch(stream: std::sync::Weak<HlsStream>, every: Duration, idle: Duration) {
    loop {
        tokio::time::sleep(every).await;
        let Some(stream) = stream.upgrade() else {
            return;
        };
        let exited = lock(&stream.encoder)
            .as_mut()
            .is_none_or(|e| !matches!(e.child.try_wait(), Ok(None)));
        if exited {
            end(&stream, "ffmpeg exited");
            return;
        }
        if lock(&stream.last_activity).elapsed() >= idle {
            end(&stream, "nobody is listening");
            return;
        }
    }
}

/// Query parameters for a heartbeat.
#[derive(Debug, Deserialize)]
struct HeartbeatQuery {
    disconnect: Option<String>,
    status: Option<String>,
}

/// A heartbeat's body: upstream's `HLSHeartbeatRequest`. No `Debug`: the
/// token is the only key to the stream.
#[derive(Deserialize)]
struct HeartbeatBody {
    stream_token: String,
    session_id: Option<String>,
}

/// `POST /api/v2/streams/hls/heartbeat` — a listener is still there; with
/// `?disconnect=true` (or `?status=disconnect`), it is leaving.
///
/// An unknown token is `{"status": "ok"}`, as upstream answers: the stream
/// ended, and there is nothing for the client to do about it here.
async fn heartbeat(Query(q): Query<HeartbeatQuery>, body: axum::body::Bytes) -> Response {
    let Ok(beat) = serde_json::from_slice::<HeartbeatBody>(&body) else {
        return (StatusCode::BAD_REQUEST, "Invalid heartbeat format").into_response();
    };
    let session = beat.session_id.filter(|s| !s.is_empty() && s.len() <= 64);
    if let Some(stream) = find_by_token(&beat.stream_token) {
        let leaving =
            q.disconnect.as_deref() == Some("true") || q.status.as_deref() == Some("disconnect");
        if leaving {
            leave(&stream, session.as_deref());
        } else {
            stream.see(session.as_deref());
        }
    }
    Json(json!({ "status": "ok" })).into_response()
}

/// A session leaves; the stream ends if no session is left.
fn leave(stream: &HlsStream, session: Option<&str>) {
    let left = {
        let mut sessions = lock(&stream.sessions);
        if let Some(session) = session {
            sessions.remove(session);
        }
        sessions.retain(|_, seen| seen.elapsed() < SESSION_LIMIT);
        sessions.is_empty()
    };
    if left {
        end(stream, "the last listener left");
    }
}

/// `POST /api/v2/streams/hls/{source_id}/stop?freq_shift_hz=` — the caller's
/// session leaves the source's stream, which ends with its last listener.
///
/// Upstream puts this behind sign-in. Here it only removes the session the
/// request names, and a session id is the caller's own random value, so one
/// listener cannot end another's stream by stopping.
async fn stop(
    UrlPath(source_id): UrlPath<String>,
    Query(q): Query<StartQuery>,
    body: axum::body::Bytes,
) -> Response {
    let session = session_of(&body);
    let shift = q.freq_shift_hz.clamp(
        -crate::routes::livestream::MAX_STREAM_SHIFT_HZ,
        crate::routes::livestream::MAX_STREAM_SHIFT_HZ,
    );
    if let Some(stream) = find_by_key(&(source_id, shift))
        && session.is_some()
    {
        leave(&stream, session.as_deref());
    }
    Json(json!({ "status": "stopped" })).into_response()
}

/// `GET /api/v2/streams/hls/status` — every running stream, without tokens:
/// upstream's `{"streams": [...], "count": N}`.
async fn status() -> Response {
    let streams: Vec<Arc<HlsStream>> = lock(&STREAMS).by_key.values().cloned().collect();
    let mut rows: Vec<serde_json::Value> = streams
        .iter()
        .map(|s| describe(s, "active", s.ready(), false))
        .collect();
    rows.sort_by(|a, b| a["source"].as_str().cmp(&b["source"].as_str()));
    Json(json!({ "count": rows.len(), "streams": rows })).into_response()
}

/// `GET /api/v2/streams/hls/t/{token}/{file}` — the playlist or a segment.
async fn content(UrlPath((token, file)): UrlPath<(String, String)>) -> Response {
    let Some(stream) = find_by_token(&token) else {
        return (StatusCode::NOT_FOUND, "Stream not found").into_response();
    };
    if !servable(&file) {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    stream.touch();
    let path = stream.dir.join(&file);
    let is_playlist = file == "playlist.m3u8";
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                if is_playlist {
                    "application/vnd.apple.mpegurl"
                } else {
                    "video/mp2t"
                },
            ),
            // The playlist changes every segment; a segment never does.
            (
                header::CACHE_CONTROL,
                if is_playlist {
                    "no-cache, no-store"
                } else {
                    "private, max-age=60"
                },
            ),
        ],
        bytes,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_encoders_own_files_are_served() {
        for ok in ["playlist.m3u8", "seg00001.ts", "seg123456.ts"] {
            assert!(servable(ok), "{ok}");
        }
        for bad in [
            "seg.ts",
            "segx.ts",
            "../playlist.m3u8",
            "seg00001.ts/../../etc/passwd",
            "seg00001.mp4",
            "playlist.m3u8.tmp",
            "",
        ] {
            assert!(!servable(bad), "{bad}");
        }
    }

    #[test]
    fn a_playlist_is_ready_at_two_segments() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("playlist.m3u8");
        assert!(!playlist_ready(&p), "no playlist yet");
        std::fs::write(
            &p,
            "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\nseg00000.ts\n",
        )
        .unwrap();
        assert!(!playlist_ready(&p), "one segment");
        std::fs::write(
            &p,
            "#EXTM3U\n#EXTINF:2.0,\nseg00000.ts\n#EXTINF:2.0,\nseg00001.ts\n",
        )
        .unwrap();
        assert!(playlist_ready(&p));
    }

    #[test]
    fn a_token_is_128_random_bits_in_hex() {
        let a = new_token().expect("token");
        let b = new_token().expect("token");
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn the_encoder_writes_into_the_streams_directory() {
        let args = hls_output_args(Path::new("/tmp/birdnet-hls/abc"));
        let joined = args.join(" ");
        assert!(
            joined.contains("-f hls -hls_time 2 -hls_list_size 6"),
            "{joined}"
        );
        assert!(
            joined.contains("temp_file"),
            "the playlist is replaced whole"
        );
        assert!(
            joined.ends_with("/tmp/birdnet-hls/abc/playlist.m3u8"),
            "{joined}"
        );
        assert!(
            joined.contains("/tmp/birdnet-hls/abc/seg%05d.ts"),
            "{joined}"
        );
    }

    /// A stream registered as `start` registers it, its encoder a real child
    /// process (`sleep`) holding a real slot.
    fn registered(source: &str, child_secs: &str) -> (Arc<HlsStream>, Arc<tokio::sync::Semaphore>) {
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let child = tokio::process::Command::new("sleep")
            .arg(child_secs)
            .kill_on_drop(true)
            .spawn()
            .expect("spawn sleep");
        let token = new_token().expect("token");
        let dir = tempfile::tempdir().expect("dir").keep();
        let stream = Arc::new(HlsStream {
            key: (source.to_owned(), 0),
            token: token.clone(),
            dir,
            encoder: Mutex::new(Some(Encoder {
                child,
                permit: Arc::clone(&slots).try_acquire_owned().expect("slot"),
                source_id: source.to_owned(),
            })),
            last_activity: Mutex::new(Instant::now()),
            sessions: Mutex::new(HashMap::new()),
        });
        let mut streams = lock(&STREAMS);
        streams
            .by_key
            .insert(stream.key.clone(), Arc::clone(&stream));
        streams.by_token.insert(token, Arc::clone(&stream));
        drop(streams);
        (stream, slots)
    }

    /// Ended: unregistered, its slot back, its directory gone.
    fn assert_ended(stream: &HlsStream, slots: &tokio::sync::Semaphore) {
        assert!(find_by_token(&stream.token).is_none(), "still registered");
        assert!(
            find_by_key(&stream.key).is_none(),
            "still registered by key"
        );
        assert!(lock(&stream.encoder).is_none(), "encoder still held");
        assert_eq!(
            slots.available_permits(),
            1,
            "its stream slot was not released"
        );
        assert!(!stream.dir.exists(), "its segments were not deleted");
    }

    #[tokio::test]
    async fn a_stream_nobody_asks_for_is_ended() {
        let (stream, slots) = registered("watch-idle", "30");
        let watcher = tokio::spawn(watch(
            Arc::downgrade(&stream),
            Duration::from_millis(20),
            Duration::from_millis(200),
        ));
        tokio::time::timeout(Duration::from_secs(10), watcher)
            .await
            .expect("the watchdog ended the stream")
            .expect("watchdog task");
        assert_ended(&stream, &slots);
    }

    /// The counterpart: a stream that is asked for is not ended for idleness.
    #[tokio::test]
    async fn a_stream_that_is_asked_for_is_kept() {
        let (stream, slots) = registered("watch-busy", "30");
        let watcher = tokio::spawn(watch(
            Arc::downgrade(&stream),
            Duration::from_millis(20),
            Duration::from_millis(200),
        ));
        for _ in 0..30 {
            stream.touch();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!watcher.is_finished(), "ended while it was being fetched");
        assert!(find_by_token(&stream.token).is_some());
        assert_eq!(slots.available_permits(), 0, "the encoder holds its slot");
        end(&stream, "test over");
        assert_ended(&stream, &slots);
    }

    #[tokio::test]
    async fn a_stream_whose_encoder_exits_is_ended() {
        let (stream, slots) = registered("watch-exit", "0");
        let watcher = tokio::spawn(watch(
            Arc::downgrade(&stream),
            Duration::from_millis(20),
            Duration::from_secs(3600),
        ));
        tokio::time::timeout(Duration::from_secs(10), watcher)
            .await
            .expect("the watchdog noticed the encoder exit")
            .expect("watchdog task");
        assert_ended(&stream, &slots);
    }

    /// Set a directory's modification time `ago` in the past.
    fn age(dir: &Path, ago: Duration) {
        std::fs::File::open(dir)
            .expect("open dir")
            .set_modified(std::time::SystemTime::now() - ago)
            .expect("set mtime");
    }

    #[tokio::test]
    async fn only_an_abandoned_streams_directory_is_swept() {
        let root = tempfile::tempdir().expect("root");
        let hour = Duration::from_secs(3600);
        let make = |name: &str, ago: Option<Duration>| {
            let d = root.path().join(name);
            std::fs::create_dir(&d).expect("mkdir");
            std::fs::write(d.join("seg00001.ts"), b"x").expect("segment");
            if let Some(ago) = ago {
                age(&d, ago);
            }
            d
        };
        let abandoned = make(&new_token().expect("token"), Some(hour));
        let live = make(&new_token().expect("token"), None);
        let not_ours = make("lost+found", Some(hour));
        let short = make("abc", Some(hour));
        // A stream of this process whose source has gone quiet: nothing has
        // written its directory for an hour, and it is still running.
        let (quiet, slots) = registered("sweep-quiet", "30");
        let owned = make(&quiet.token, Some(hour));

        sweep_abandoned(root.path(), ABANDONED_AFTER);

        assert!(!abandoned.exists(), "an hour-old stream directory was kept");
        assert!(
            live.exists(),
            "a stream directory written just now was swept"
        );
        assert!(not_ours.exists(), "a name no token has was swept");
        assert!(short.exists(), "a hex name too short for a token was swept");
        assert!(owned.exists(), "this process's own quiet stream was swept");
        end(&quiet, "test over");
        assert_ended(&quiet, &slots);
    }

    #[test]
    fn a_session_id_is_bounded() {
        assert_eq!(
            session_of(&axum::body::Bytes::from_static(
                br#"{"session_id":"tab-1"}"#
            ))
            .as_deref(),
            Some("tab-1")
        );
        assert_eq!(session_of(&axum::body::Bytes::new()), None, "no body");
        assert_eq!(
            session_of(&axum::body::Bytes::from_static(b"nonsense")),
            None
        );
        let long = format!(r#"{{"session_id":"{}"}}"#, "x".repeat(65));
        assert_eq!(session_of(&axum::body::Bytes::from(long)), None, "too long");
    }
}
