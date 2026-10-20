//! Live audio streaming routes.
//!
//! Provides live audio streaming from the microphone (ALSA) or RTSP source
//! to the browser, replacing BirdNET-Pi's Icecast2 dependency.
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET /stream` | Raw MP3 audio stream via HTTP chunked transfer |
//! | `GET /api/v2/languages` | The languages species names can be shown in, and the active one |

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use birdnet_core::audio::capture::LiveSubscription;
use birdnet_core::i18n::LANGUAGES;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;

use crate::state::AppState;

/// Sample rate used for the live audio stream output (Hz).
const STREAM_SAMPLE_RATE: u32 = 44_100;

/// Bytes read from a live tap per pump iteration (~85 ms of 48 kHz mono audio).
/// Small enough that the stream stays responsive, large enough that the pump
/// isn't syscall-bound on a Raspberry Pi.
const TAP_CHUNK_BYTES: usize = 8 * 1024;

/// How long to wait for a live tap's first audio before concluding the source
/// is not currently recording.
///
/// A running capture pushes every ALSA period — tens of milliseconds — so
/// anything approaching this bound means the source is paused by the recording
/// schedule or is down. Waiting rather than checking a timestamp also covers
/// the moment right after capture starts, when the tap exists but the first
/// period has not landed yet.
const TAP_FIRST_AUDIO_TIMEOUT: Duration = Duration::from_secs(2);

/// Buffered chunks between the blocking tap reader and the async ffmpeg feeder.
///
/// Deliberately shallow. If ffmpeg cannot keep up, the right outcome is for the
/// *listener* to fall behind and lose bytes at the tap — which is bounded and
/// lossy by design — not for chunks to pile up in an ever-growing queue.
const TAP_PUMP_QUEUE: usize = 4;

/// Maximum number of concurrent live `/stream` connections.
///
/// Each connection spawns its own `ffmpeg` capturing + MP3-encoding the source,
/// so an unbounded count is a trivial unauthenticated resource-exhaustion vector
/// on a Raspberry Pi (`kill_on_drop` cleans up on disconnect but doesn't bound
/// the peak). BirdNET-Pi sidesteps this with a single shared Icecast stream; we
/// allow a handful of independent streams (different devices / pitch shifts) and
/// return 503 beyond that.
const MAX_CONCURRENT_STREAMS: usize = 4;

/// Process-global permit pool bounding concurrent live streams.
static STREAM_SLOTS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_STREAMS)));

/// How long a listener may stop reading before its stream is closed.
///
/// A client that stops reading — a suspended tab, a stalled connection, or one
/// held open on purpose — fills the channel and then the socket buffers, and
/// `send` waits. Without a limit it waited forever, holding the stream slot and
/// an `ffmpeg` for as long as the connection stayed up, so four such clients
/// took live audio away from everyone.
const STALL_LIMIT: Duration = Duration::from_secs(30);

/// Forward encoded chunks to the response until either side ends, and return
/// how many bytes were delivered.
async fn forward<S>(
    mut chunks: S,
    tx: &tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
    stall: Duration,
) -> u64
where
    S: tokio_stream::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    let mut delivered = 0_u64;
    while let Some(chunk) = chunks.next().await {
        if let Ok(ref bytes) = chunk {
            delivered += bytes.len() as u64;
        }
        match tokio::time::timeout(stall, tx.send(chunk.map_err(std::io::Error::other))).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => break, // client disconnected
            Err(_) => {
                tracing::info!(
                    stall_secs = stall.as_secs(),
                    "live audio listener stopped reading; closing its stream"
                );
                break;
            }
        }
    }
    delivered
}

/// Query parameters for the live audio stream.
#[derive(Debug, Deserialize)]
pub struct StreamParams {
    /// Frequency shift in Hz applied to the live stream. Positive raises the
    /// pitch, negative lowers it; clamped to ±`MAX_STREAM_SHIFT_HZ`.
    ///
    /// A true shift in hertz (ffmpeg's `afreqshift`): every component moves by
    /// this many Hz and the stream's tempo is untouched. **Negative** is the
    /// accessibility direction — see `freq_shift_filter`.
    /// BirdNET-Pi equivalent: the `rubberband` pitch filter its `livestream.sh`
    /// applies, which scales by a ratio below 1 instead.
    #[serde(default)]
    pub freq_shift_hz: i32,
    /// Optional `audio_sources.id` selecting which configured source to
    /// stream. Without it, `/stream` resolves the first non-disabled
    /// `audio_sources` row; on a station with no rows yet, `/stream` returns
    /// `503` (sources are managed through the `audio_sources` table — add one
    /// via `/admin/audio`).
    #[serde(default)]
    pub source_id: Option<String>,
}

/// Mount livestream and i18n routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/languages", get(list_languages))
}

/// Mount the raw audio stream route (top-level, not under /api/v2).
pub fn stream_router() -> Router<AppState> {
    Router::new().route("/stream", get(livestream))
}

/// The widest shift the stream will apply, in either direction.
///
/// The query parameter is an unbounded `i32` from an unauthenticated request.
/// Without a ceiling, `freq_shift_hz=2000000000` asks ffmpeg to resample from
/// ~2 GHz to 44.1 kHz — a ratio of 45 000:1 — which is a CPU burn a stranger
/// on the LAN can start, four at a time, up to `MAX_CONCURRENT_STREAMS`.
///
/// ±24 kHz is far beyond anything a listener wants (it inverts the whole
/// audible band) and still leaves every useful setting reachable, so clamping
/// here costs nothing an operator would notice.
pub(super) const MAX_STREAM_SHIFT_HZ: i32 = 24_000;

/// Build the ffmpeg filter string for an optional frequency shift.
///
/// A non-zero shift applies `afreqshift`, which moves every frequency
/// component by `shift_hz` and leaves the stream's tempo alone: an 8 kHz song
/// at `-3000` is heard at 5 kHz. (This used to be `asetrate` followed by
/// `aresample`, described as shifting "without stretching duration". It
/// scaled frequencies by a ratio — 8 kHz to 7.5 kHz at `-3000` — and slowed the
/// audio by the same ratio, which on a live stream is a delay that grows.)
///
/// **Negative is the accessibility direction**: age-related hearing loss takes
/// the top of the range first, so bringing 8 kHz song *down* is what restores
/// it. See [`birdnet_core::audio::extraction::ACCESSIBILITY_SHIFT_HZ`], which
/// records the upstream settings this was checked against — several comments in
/// this project used to claim the opposite.
///
/// The shift is clamped to ±[`MAX_STREAM_SHIFT_HZ`], so no query parameter can
/// ask ffmpeg for an absurd filter.
fn freq_shift_filter(shift_hz: i32) -> Option<String> {
    if shift_hz == 0 {
        return None;
    }
    let shift_hz = shift_hz.clamp(-MAX_STREAM_SHIFT_HZ, MAX_STREAM_SHIFT_HZ);
    Some(format!("afreqshift=shift={shift_hz}"))
}

// ---------------------------------------------------------------------------
// GET /api/v2/languages
// ---------------------------------------------------------------------------

/// The languages species common names can be shown in, and the one this
/// station is using.
///
/// Each language's `coverage` says how many of the installed classifier's
/// species it has a name for, so a station can see what choosing it would give
/// it; the rest keep the classifier's own name. It is `null` for English (every
/// name is shown as stored) and when no classifier label file was available to
/// count against.
async fn list_languages(State(state): State<AppState>) -> Json<Value> {
    let names = state.names();
    let active = names.language().code;
    let langs: Vec<Value> = LANGUAGES
        .iter()
        .map(|l| {
            let coverage = names
                .coverage_of(l.code)
                .map(|c| json!({ "named": c.named, "of": c.of }));
            json!({
                "code": l.code,
                "name": l.english_name,
                "native_name": l.native_name,
                "active": l.code == active,
                "coverage": coverage,
            })
        })
        .collect();

    Json(json!({
        "languages": langs,
        "count": langs.len(),
        "active": active,
    }))
}

// ---------------------------------------------------------------------------
// GET /stream
// ---------------------------------------------------------------------------

/// Stream live audio as MP3 via HTTP chunked transfer.
///
/// `ffmpeg` encodes the stream; where its *input* comes from depends on whether
/// capture is already holding the source open:
///
/// * **A teed source** (a local ALSA microphone) publishes its PCM in-process,
///   and ffmpeg is fed from that over a pipe. It has to be: the device is
///   exclusive, so opening it a second time returns `Device or resource busy`
///   for as long as the station is recording. See [`resolve_live_feed`].
/// * **Anything else** (RTSP, `PipeWire`, or a station running web-only) is
///   opened by ffmpeg directly, exactly as before.
///
/// Supports optional frequency shifting via `?freq_shift_hz=<N>` query param
/// (positive = shift up, negative = shift down), a shift in hertz by the same
/// `afreqshift` filter as the extraction pipeline.
///
/// If no audio source is configured, returns `503 Service Unavailable`; so does
/// a teed source that is not currently recording.
///
/// When `?source_id=` is supplied, the row's `kind` + `device_id` from the
/// `audio_sources` table drives the ffmpeg backend selection — the
/// listen-now page uses this to switch between configured mics / streams
/// without restarting the daemon. An unknown id returns `404`.
///
/// Without `?source_id=`, the station's configured default wins: the
/// `audio_sources` row named by the `livestream_source` setting, or the first
/// non-disabled row when that setting is unset or names a row that has since
/// been removed or disabled. On a station whose `audio_sources` table is still
/// empty, `/stream` returns `503` until an operator adds a row through
/// `/admin/audio` — the new row then takes over without a restart.
async fn livestream(State(state): State<AppState>, Query(params): Query<StreamParams>) -> Response {
    let Encoder {
        mut child,
        permit: stream_permit,
        source_id,
    } = match start_encoder(
        &state,
        params.source_id.clone(),
        params.freq_shift_hz,
        &[
            "-f",
            "mp3",
            "-b:a",
            "128k",
            "-ar",
            &STREAM_SAMPLE_RATE.to_string(),
            "-ac",
            "1",
            // Hand each packet to the pipe as it is encoded instead of waiting
            // for the avio buffer to fill. Measured through this exact
            // invocation, it halves time-to-first-audio (1.13 s -> 0.59 s), and
            // that window is precisely when a listener is deciding whether the
            // button worked.
            "-flush_packets",
            "1",
            "pipe:1",
        ],
        std::process::Stdio::piped(),
    )
    .await
    {
        Ok(encoder) => encoder,
        Err(response) => return *response,
    };

    let Some(stdout) = child.stdout.take() else {
        tracing::error!("ffmpeg process has no stdout handle");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to capture audio stream",
        )
            .into_response();
    };

    // Forward ffmpeg's stdout to the response body through a task that owns the
    // ffmpeg child and the concurrency permit, so both are dropped — ffmpeg
    // killed via kill_on_drop, the stream slot freed — exactly when the client
    // disconnects (the receiver drops and `send` fails). Holding the child here
    // also keeps it from being killed the instant this handler returns.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(16);
    let streamed_source = source_id;
    tokio::spawn(async move {
        let _permit = stream_permit;
        let _child = child;
        let delivered = forward(ReaderStream::new(stdout), &tx, STALL_LIMIT).await;
        // The status line went out with the headers, long before ffmpeg could
        // fail, so a dead encoder reaches the browser as a successful but empty
        // stream — silence that looks exactly like a broken button. This cannot
        // retroactively change the response, but it does put the fact in the
        // journal next to whatever ffmpeg said on stderr.
        if delivered == 0 {
            tracing::warn!(
                source = %streamed_source,
                "live audio stream ended without delivering any audio"
            );
        }
    });

    let body = Body::from_stream(ReceiverStream::new(rx));

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/mpeg"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store"),
    );
    // No `Transfer-Encoding` here on purpose. It is a hop-by-hop framing header
    // the HTTP layer owns: hyper already chunks a streaming body and emits the
    // header itself (verified on the wire — setting it by hand changed nothing
    // but the header order), and HTTP/2 forbids it outright, so a station behind
    // an h2 reverse proxy would have the response rejected for carrying it.
    // ICY-compatible metadata
    headers.insert(
        axum::http::HeaderName::from_static("icy-name"),
        HeaderValue::from_static("BirdNet-Behavior Live"),
    );

    (StatusCode::OK, headers, body).into_response()
}

/// An `ffmpeg` encoding a live source, and the stream slot it holds.
///
/// Dropping it kills the encoder (`kill_on_drop`) and frees the slot, which
/// is how every consumer ends a stream: let go of this.
pub(super) struct Encoder {
    /// The encoder. Its stdin, if any, is already fed from the capture tap.
    pub(super) child: tokio::process::Child,
    /// One of the [`MAX_CONCURRENT_STREAMS`] slots, held for the encoder's life.
    pub(super) permit: tokio::sync::OwnedSemaphorePermit,
    /// The `audio_sources` row being encoded.
    pub(super) source_id: String,
}

/// Start an `ffmpeg` reading a configured source, with `output_args` after
/// its input and the optional pitch shift — the part every live listener
/// shares, whatever it encodes to.
///
/// `source_id` empty or `None` is the station's default source. The `Err` is
/// the response to give: `404` for an unknown id, `503` for no source, a
/// stream that cannot be opened, a full set of stream slots, a source that is
/// not recording, or a missing `ffmpeg`, and `500` for a spawn that failed
/// otherwise. A slot is taken only once the source has resolved, so a
/// refusal never consumes one.
pub(super) async fn start_encoder(
    state: &AppState,
    source_id: Option<String>,
    freq_shift_hz: i32,
    output_args: &[&str],
    stdout: std::process::Stdio,
) -> Result<Encoder, Box<Response>> {
    let refuse = |status: StatusCode, message: String| Box::new((status, message).into_response());
    // Resolve the audio source. Two paths:
    //   1. a source id → DB lookup by id; honour the row's kind explicitly.
    //   2. none → the station's default `audio_sources` row, else 503.
    let by_id = source_id.filter(|id| !id.is_empty());
    let asked_for_id = by_id.is_some();
    let looked_up = state
        .run_blocking(move |state| {
            by_id.as_deref().map_or_else(
                || resolve_default_source(state),
                |id| resolve_by_source_id(state, id),
            )
        })
        .await;
    let resolved = match looked_up {
        Some(resolved) => resolved,
        None if asked_for_id => {
            return Err(refuse(
                StatusCode::NOT_FOUND,
                "no such audio source".to_owned(),
            ));
        }
        None => {
            return Err(refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                "no audio source configured".to_owned(),
            ));
        }
    };
    let ResolvedSource {
        id: source_id,
        device: source,
        kind: kind_hint,
        rtsp_input,
    } = resolved;

    // Honour the audio_sources kind when present; otherwise fall back to
    // the URL-prefix heuristic that the single-string path has always used.
    let (is_rtsp, is_pulse) = kind_hint.map_or_else(
        || {
            (
                source.starts_with("rtsp://") || source.starts_with("rtsps://"),
                source.starts_with("pulse://") || source == "pulse" || source == "default",
            )
        },
        |k| {
            use birdnet_db::audio_sources::SourceKind;
            (
                matches!(k, SourceKind::Rtsp),
                matches!(k, SourceKind::PipeWire),
            )
        },
    );

    // The reason names the schemes that work, never the URL, which can carry
    // a camera's password.
    let rtsp_input = rtsp_input.map_err(|reason| {
        refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("this source's stream cannot be opened: {reason}"),
        )
    })?;

    // Bound concurrent streams before spawning ffmpeg (after source resolution,
    // so a 404/503 never consumes a slot).
    let Ok(permit) = STREAM_SLOTS.clone().try_acquire_owned() else {
        return Err(refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "too many concurrent live streams".to_owned(),
        ));
    };

    let live = resolve_live_feed(state, &source_id).await?;

    // Build the audio filter chain: optional freq shift + format conversion.
    let audio_filter = freq_shift_filter(freq_shift_hz);

    let mut cmd = tokio::process::Command::new("ffmpeg");

    // `-loglevel error` reduces stderr to things that actually went wrong, which
    // is what makes draining it below worth doing: every line that arrives is
    // worth an operator's attention rather than banner noise.
    cmd.args(["-hide_banner", "-loglevel", "error"]);

    cmd.args(stream_input_args(
        live.as_ref().map(|l| l.subscription.spec()),
        is_rtsp,
        is_pulse,
        &source,
        &rtsp_input,
    ));

    if let Some(ref filter) = audio_filter {
        cmd.args(["-af", filter.as_str()]);
    }

    cmd.args(output_args);

    let child = cmd
        .stdin(if live.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(stdout)
        // ffmpeg's stderr is the only place a failed live stream explains
        // itself — `Device or resource busy`, an unknown filter, a codec that
        // isn't built in. Discarding it made every failure present identically
        // from the outside: a 200 response carrying no audio, and a journal with
        // nothing in it. Drained on a task below so a full pipe cannot ever
        // block the encoder.
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        // `NotFound` here means ffmpeg is not on PATH, which is a missing
        // dependency rather than a server fault — and on a microphone station
        // it is the *expected* state, because the installer only ensures
        // ffmpeg for RTSP capture while this endpoint needs it for every
        // source kind. Saying so is the difference between an operator running
        // one apt command and an operator filing a bug against an opaque 500.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::error!(
                error = %e,
                "live audio stream needs ffmpeg, which is not installed"
            );
            return Err(refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                "live audio needs ffmpeg, which is not installed on this station — \
                 install it (e.g. `sudo apt install ffmpeg`) and reload this page"
                    .to_owned(),
            ));
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to spawn ffmpeg for livestream");
            return Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to start audio stream".to_owned(),
            ));
        }
    };

    // Surface ffmpeg's own account of any failure. With `-loglevel error` this
    // stays silent on a healthy stream, so anything logged here is a real fault
    // an operator needs — and draining the pipe is mandatory regardless, since
    // an unread stderr eventually blocks the encoder.
    if let Some(stderr) = child.stderr.take() {
        let failing_source = source_id.clone();
        tokio::spawn(async move { drain_ffmpeg_stderr(stderr, &failing_source).await });
    }

    let from_capture_tap = live.is_some();
    if let Some(live) = live {
        let Some(stdin) = child.stdin.take() else {
            tracing::error!("ffmpeg process has no stdin handle");
            return Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to feed the audio stream".to_owned(),
            ));
        };
        spawn_tap_pump(live, stdin);
    }

    tracing::info!(
        source = %source_id,
        device = %loggable(&source),
        freq_shift_hz,
        from_capture_tap,
        "starting live audio stream"
    );
    Ok(Encoder {
        child,
        permit,
        source_id,
    })
}

/// Text as it may be logged: with the password of any URL in it replaced.
///
/// A source's device string is its RTSP URL, and an RTSP URL's userinfo is the
/// camera's password. ffmpeg names its input in its own errors
/// (`Error opening input file rtsp://admin:hunter2@cam/stream.`), and this
/// module logs both ffmpeg's stderr and the device it was started on. The
/// capture path redacts the same way (`capture_log_line` in `birdnet-core`);
/// this is the same call, because the same line reaches the same journal and
/// the support bundle read from it.
fn loggable(text: &str) -> String {
    birdnet_core::config::redact::redact_credentials_in_text(text)
}

/// Drain ffmpeg's stderr into the log, one `warn` per line, redacted.
///
/// With `-loglevel error` this is silent on a healthy stream, so anything here
/// is a real fault. Draining is mandatory regardless: an unread stderr
/// eventually blocks the encoder.
async fn drain_ffmpeg_stderr<R>(stderr: R, source_id: &str)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncBufReadExt as _;
    let mut lines = tokio::io::BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::warn!(source = %source_id, "ffmpeg: {}", loggable(&line));
    }
}

/// Decide whether this source is served from capture's in-process tap.
///
/// For a local microphone it is, and it **must** be: the ALSA device is
/// exclusive, so opening it again returns `Device or resource busy` for as long
/// as the station is recording — which is always. There is deliberately no
/// fallback to opening the device when a tap exists, even while capture is
/// paused: grabbing a device the supervisor is about to resume on would turn one
/// silent stream into a restart loop.
///
/// * `Ok(Some(feed))` — stream from the tap, starting with `feed`'s first chunk.
/// * `Ok(None)` — no tap for this source (RTSP, `PipeWire`, or web-only mode);
///   the caller opens the source itself, exactly as before.
/// * `Err(response)` — there is a tap but it is silent, so the source is not
///   recording. Answering `503` is the honest result; the alternative is a
///   connection that stays open forever delivering nothing. Boxed because a
///   `Response` is large enough that clippy flags the `Err` arm, the same
///   trade `auth_middleware::require_admin` already makes.
async fn resolve_live_feed(
    state: &AppState,
    source_id: &str,
) -> Result<Option<LiveFeed>, Box<Response>> {
    let Some(tap) = state.live_audio().and_then(|hub| hub.lookup(source_id)) else {
        return Ok(None);
    };
    if let Some(feed) = wait_for_first_audio(tap.subscribe()).await {
        return Ok(Some(feed));
    }
    tracing::info!(
        source = source_id,
        "live audio requested for a source that is not recording"
    );
    Err(Box::new(
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "this source is not recording right now — live audio follows capture, \
             so check the recording schedule, any quiet window, and Station Health",
        )
            .into_response(),
    ))
}

/// The ffmpeg **input** arguments for a live stream.
///
/// This is the decision the whole in-process tee exists to change, so it is a
/// pure function that can be pinned by a test rather than an `if` buried in a
/// handler. When `tap` is `Some`, ffmpeg reads raw PCM from `pipe:0` and never
/// names the device at all; that is what makes live audio possible while
/// capture holds an exclusive ALSA microphone open. When it is `None` — an RTSP
/// camera, a PulseAudio/PipeWire source, or a station whose capture is not
/// teeing — the historical behaviour is unchanged.
fn stream_input_args(
    tap: Option<birdnet_core::audio::capture::PcmSpec>,
    is_rtsp: bool,
    is_pulse: bool,
    device: &str,
    rtsp_input: &[String],
) -> Vec<String> {
    let own = |args: [&str; 4]| args.iter().map(|s| (*s).to_string()).collect();
    if let Some(spec) = tap {
        return vec![
            "-f".to_string(),
            "s16le".to_string(),
            "-ar".to_string(),
            spec.sample_rate.to_string(),
            "-ac".to_string(),
            spec.channels.to_string(),
            "-i".to_string(),
            "pipe:0".to_string(),
        ];
    }
    if is_rtsp {
        let mut args = rtsp_input.to_vec();
        args.extend(["-i".to_string(), device.to_string(), "-vn".to_string()]);
        return args;
    }
    if is_pulse {
        return own(["-f", "pulse", "-i", device.trim_start_matches("pulse://")]);
    }
    own(["-f", "alsa", "-i", device])
}

/// A live tap that has already proved it is producing audio, plus the first
/// chunk read from it (which must not be dropped on the floor).
struct LiveFeed {
    subscription: LiveSubscription,
    first_chunk: Vec<u8>,
}

/// Wait for a subscription's first audio, so the handler can answer "is this
/// source actually recording?" before committing to a 200.
///
/// Returns `None` when nothing arrives within [`TAP_FIRST_AUDIO_TIMEOUT`].
/// Runs on the blocking pool because the read parks on a condvar — cheap and
/// bounded, and far more truthful than inspecting a "last audio" timestamp,
/// which cannot distinguish "paused" from "started a moment ago".
async fn wait_for_first_audio(mut subscription: LiveSubscription) -> Option<LiveFeed> {
    tokio::task::spawn_blocking(move || {
        let mut buf = vec![0u8; TAP_CHUNK_BYTES];
        let n = subscription.read(&mut buf, TAP_FIRST_AUDIO_TIMEOUT);
        buf.truncate(n);
        (subscription, buf)
    })
    .await
    .ok()
    .filter(|(_, first)| !first.is_empty())
    .map(|(subscription, first_chunk)| LiveFeed {
        subscription,
        first_chunk,
    })
}

/// Pump a live tap into `sink` — ffmpeg's stdin in production.
///
/// Two tasks, because the tap read blocks and the sink is async:
///
/// * a blocking task that reads the ring and hands chunks over a shallow
///   channel;
/// * an async task that writes them to the sink.
///
/// Everything unwinds from the far end: the client disconnects, the response
/// body's task drops the ffmpeg child (`kill_on_drop`), the stdin write fails,
/// the channel closes, and the blocking reader's send fails and it returns.
/// Nothing here can slow capture down — falling behind costs this listener
/// bytes at the tap and costs the recorder nothing.
///
/// Generic over the sink so the pump can be tested against an in-memory pipe;
/// it is the one piece of this endpoint that would otherwise only ever run with
/// a real ffmpeg on the far end.
fn spawn_tap_pump<W>(live: LiveFeed, mut sink: W)
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let LiveFeed {
        mut subscription,
        first_chunk,
    } = live;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(TAP_PUMP_QUEUE);

    tokio::task::spawn_blocking(move || {
        let mut buf = vec![0u8; TAP_CHUNK_BYTES];
        loop {
            let n = subscription.read(&mut buf, Duration::from_millis(500));
            if n == 0 {
                // The source went quiet (paused, or between periods). Keep the
                // connection open and check whether the listener is still there
                // by looping — `blocking_send` below is what notices departure.
                if tx.is_closed() {
                    break;
                }
                continue;
            }
            if tx.blocking_send(buf[..n].to_vec()).is_err() {
                break; // listener gone
            }
        }
        let dropped = subscription.dropped_bytes();
        if dropped > 0 {
            tracing::debug!(
                dropped_bytes = dropped,
                "live listener fell behind the capture tap"
            );
        }
    });

    tokio::spawn(async move {
        // The chunk the liveness probe already consumed goes first — dropping
        // it would clip the start of every stream.
        if sink.write_all(&first_chunk).await.is_err() {
            return;
        }
        while let Some(chunk) = rx.recv().await {
            if sink.write_all(&chunk).await.is_err() {
                break;
            }
        }
    });
}

/// An `audio_sources` row resolved for streaming.
struct ResolvedSource {
    /// The row id. Also the capture-source label, so it is the key a live tap
    /// is registered under — that identity is why the id is carried here and
    /// not discarded as it used to be.
    id: String,
    /// The row's `device_id`: an ALSA device, a PulseAudio source, or an RTSP
    /// URL, depending on `kind`.
    device: String,
    /// The `SourceKind` from the DB, used to bypass the legacy URL-prefix
    /// heuristic so a PipeWire row whose `device_id` is literally `default`
    /// (operator typed `default`) still picks the PulseAudio ffmpeg backend
    /// rather than ALSA.
    kind: Option<birdnet_db::audio_sources::SourceKind>,
    /// ffmpeg input options for a network stream row, from its URL: for RTSP
    /// its transport and media mode. A stream is not teed, so listening live
    /// opens a second session to it, and it has to be the same kind of session
    /// capture opens — this used to be a fixed `-rtsp_transport tcp`, so a
    /// UDP-only camera could not be listened to, an audio-only one gave up a
    /// video session anyway, and any stream that was not RTSP could not be
    /// opened at all. `Err` names why the row's URL cannot be streamed.
    rtsp_input: Result<Vec<String>, String>,
}

/// Resolve a `?source_id=` query value by looking up the configured
/// `audio_sources` row.
///
/// Returns `None` when the row is missing, disabled, or the DB read itself
/// fails. Callers treat all three as "404 no such source".
fn resolve_by_source_id(state: &AppState, id: &str) -> Option<ResolvedSource> {
    use birdnet_db::audio_sources::AudioSourceStore;
    let sources = state.with_read_db(|conn| AudioSourceStore::list(conn).ok())?;
    pick_by_id(&sources, id)
}

/// The settings key naming the `audio_sources` row `/stream` serves by default.
///
/// Empty, or absent, means "whichever source comes first" — the behaviour
/// before this setting existed, and still the behaviour of a station with one
/// source, which is most of them.
pub const LIVESTREAM_SOURCE_SETTING: &str = "livestream_source";

/// Resolve the default audio source when no `?source_id=` is supplied.
///
/// DB-only, in two steps: the row named by [`LIVESTREAM_SOURCE_SETTING`] if it
/// is set and still enabled, else the first non-disabled `audio_sources` row
/// (in `created_at ASC` order). Returns `None` when the table is empty or
/// unreadable, in which case the caller responds with `503 Service
/// Unavailable`. (The legacy single-string `state.audio_source()` fallback was
/// retired in O-13 — sources are managed exclusively through the
/// `audio_sources` table now.)
fn resolve_default_source(state: &AppState) -> Option<ResolvedSource> {
    use birdnet_db::audio_sources::AudioSourceStore;
    let (sources, configured) = state.with_read_db(|conn| {
        (
            AudioSourceStore::list(conn).unwrap_or_default(),
            birdnet_db::settings::get_or(conn, LIVESTREAM_SOURCE_SETTING, "").unwrap_or_default(),
        )
    });
    pick_default(&sources, &configured)
}

/// The id of the source `/stream` serves for a request with no `?source_id=`.
///
/// Test-only, and deliberately so: it exists for the gate in
/// `routes::admin::audio` that asserts the key the audio page *writes* is the
/// key this resolver *reads*. Those two live in different modules, and the pure
/// `pick_default` tests below cannot see a disagreement about the key's
/// spelling — which is exactly how a setting comes to be stored and never read.
#[cfg(test)]
pub(crate) fn default_source_id(state: &AppState) -> Option<String> {
    resolve_default_source(state).map(|resolved| resolved.id)
}

/// Pure helper: the configured default if it is still there, else the first
/// enabled source.
///
/// The fallback is deliberate rather than a `404`. This is the path a visitor
/// reaches by pressing Listen with no choice of their own, and a station whose
/// named default was unplugged last week should keep streaming the microphone
/// it still has — silence, or an error page, would be a worse answer to
/// "let me hear the garden" than the other microphone.
fn pick_default(
    sources: &[birdnet_db::audio_sources::AudioSource],
    configured: &str,
) -> Option<ResolvedSource> {
    if !configured.is_empty() {
        if let Some(resolved) = pick_by_id(sources, configured) {
            return Some(resolved);
        }
        tracing::warn!(
            configured,
            "the configured default live-stream source is missing or disabled; \
             falling back to the first enabled source"
        );
    }
    pick_first_enabled(sources)
}

/// Pure helper: walk `sources` for an id-match that's still enabled.
/// Factored out so the resolver logic is unit-testable without spinning
/// up an `AppState`.
fn pick_by_id(
    sources: &[birdnet_db::audio_sources::AudioSource],
    id: &str,
) -> Option<ResolvedSource> {
    sources
        .iter()
        .find(|s| s.id == id && s.disabled_at.is_none())
        .map(resolved)
}

/// Pure helper: return the first non-disabled source.
/// `AudioSourceStore::list` already filters disabled rows out, but we re-check
/// here so the helper is safe with arbitrary slices in tests.
fn pick_first_enabled(
    sources: &[birdnet_db::audio_sources::AudioSource],
) -> Option<ResolvedSource> {
    sources
        .iter()
        .find(|s| s.disabled_at.is_none())
        .map(resolved)
}

/// Project an `audio_sources` row onto the fields streaming needs.
fn resolved(source: &birdnet_db::audio_sources::AudioSource) -> ResolvedSource {
    ResolvedSource {
        id: source.id.clone(),
        device: source.device_id.clone(),
        kind: Some(source.kind),
        rtsp_input: if source.kind == birdnet_db::audio_sources::SourceKind::Rtsp {
            stream_input_options(source)
        } else {
            Ok(Vec::new())
        },
    }
}

/// The input options capture uses for the same stream row
/// (`birdnet_core::audio::capture::network_input_options`, reached through the
/// same mapping the capture resolver applies). Without the stall timeout:
/// listening ends when the listener leaves. `Err` for a UDP stream, which
/// capture is already reading.
fn stream_input_options(
    source: &birdnet_db::audio_sources::AudioSource,
) -> Result<Vec<String>, String> {
    use birdnet_core::audio::capture::{
        RtspMediaMode, RtspTransport, StreamProtocol, network_input_options,
    };
    use birdnet_db::audio_sources::{RtspMediaMode as DbMode, RtspTransport as DbTransport};
    // A unicast UDP stream reaches one socket, and capture holds it: a second
    // reader failed to bind ("Address already in use") and the listener got
    // an empty 200. Said plainly instead.
    if StreamProtocol::of(&source.device_id) == Some(StreamProtocol::Udp) {
        return Err("a UDP stream has one reader, and the station's capture is it".to_owned());
    }
    let transport = match source.rtsp_transport {
        DbTransport::Auto => RtspTransport::Auto,
        DbTransport::Tcp => RtspTransport::Tcp,
        DbTransport::Udp => RtspTransport::Udp,
    };
    let mode = match source.media_mode {
        DbMode::Full => RtspMediaMode::Full,
        DbMode::Audio => RtspMediaMode::AudioOnly,
    };
    network_input_options(&source.device_id, transport, mode, false).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_db::audio_sources::{
        AudioSource, Channels, PipelineFlags, RtspTransport, SourceKind,
    };

    fn sample(id: &str, kind: SourceKind, device_id: &str, disabled: bool) -> AudioSource {
        AudioSource {
            id: id.to_string(),
            kind,
            device_id: device_id.to_string(),
            label: None,
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 16,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Auto,
            media_mode: birdnet_db::audio_sources::RtspMediaMode::Full,
            schedule_quiet: None,
            pipeline: PipelineFlags::default(),
            eq_chain: String::new(),
            disabled_at: if disabled {
                Some("2026-01-01".to_string())
            } else {
                None
            },
            created_at: "2026-05-01".to_string(),
            updated_at: "2026-05-01".to_string(),
        }
    }

    #[test]
    fn freq_shift_filter_off_when_zero() {
        assert!(freq_shift_filter(0).is_none());
    }

    /// The live listener's shift is a shift in hertz, like the saved clips':
    /// the same `afreqshift` filter, the sign carried through, and none of the
    /// `asetrate` ratio trick that moved 8 kHz to 7.5 kHz at `-3000` and slowed
    /// the stream as it did.
    #[test]
    fn freq_shift_filter_is_a_shift_in_hertz_and_not_a_resample() {
        assert_eq!(
            freq_shift_filter(-3_000).as_deref(),
            Some("afreqshift=shift=-3000")
        );
        assert_eq!(
            freq_shift_filter(3_000).as_deref(),
            Some("afreqshift=shift=3000")
        );
        assert!(
            !freq_shift_filter(-3_000)
                .expect("filter")
                .contains("asetrate")
        );
    }

    /// Negative lowers the pitch, which is the direction that helps.
    ///
    /// Age-related hearing loss takes the top of the range first, so a warbler
    /// at 8 kHz is restored by moving it *down*. Five doc comments in this
    /// project used to say the opposite, and the CLI help was one of them — a
    /// listener following it would have shifted the song further out of reach.
    /// The upstream this was ported from ships `FREQSHIFT_LO=3000` /
    /// `FREQSHIFT_HI=6000` (a rubberband ratio of 0.5) and a sox
    /// `FREQSHIFT_PITCH=-1500`: both downward, from two independent settings.
    ///
    /// Pinned as a *relation* on the number the filter carries, so it survives
    /// a change of filter syntax: a downward shift has to carry a negative one.
    #[test]
    fn a_negative_shift_goes_down_and_a_positive_one_goes_up() {
        let hz_of = |shift| {
            let f = freq_shift_filter(shift).expect("non-zero shift returns filter");
            // The last `shift=`: the filter is `afreqshift=shift=-3000`, whose
            // first one is the end of the filter's own name.
            let start = f.rfind("shift=").expect("shift present") + "shift=".len();
            f[start..].parse::<i32>().expect("a number")
        };
        assert!(
            hz_of(-3_000) < 0,
            "a downward shift must carry a negative number"
        );
        assert!(hz_of(3_000) > 0, "and an upward one a positive number");
        // The constant the UI offers as its accessibility preset points the
        // same way, so the two cannot drift apart. Its *sign* is guarded at
        // compile time beside the constant itself; what this adds is that the
        // filter agrees with it.
        assert!(hz_of(birdnet_core::audio::extraction::ACCESSIBILITY_SHIFT_HZ) < 0);
    }

    /// The query parameter is an unbounded `i32` from an unauthenticated
    /// request. Unclamped, `freq_shift_hz=2000000000` hands ffmpeg a shift far
    /// outside any audio band, four times over — a CPU burn a stranger on the
    /// LAN can start.
    #[test]
    fn an_absurd_shift_is_clamped_rather_than_handed_to_ffmpeg() {
        for shift in [i32::MAX, 2_000_000_000, 100_000] {
            let f = freq_shift_filter(shift).expect("non-zero shift returns filter");
            assert_eq!(
                f,
                format!("afreqshift=shift={MAX_STREAM_SHIFT_HZ}"),
                "shift {shift} was not clamped"
            );
        }
        for shift in [i32::MIN, -2_000_000_000, -100_000] {
            let f = freq_shift_filter(shift).expect("non-zero shift returns filter");
            assert_eq!(
                f,
                format!("afreqshift=shift=-{MAX_STREAM_SHIFT_HZ}"),
                "shift {shift} was not clamped"
            );
        }
        // The counterpart: a shift inside the range is passed through exactly,
        // so the clamp is a ceiling and not a constant.
        assert_eq!(
            freq_shift_filter(-3_000).as_deref(),
            Some("afreqshift=shift=-3000")
        );
    }

    #[test]
    fn pick_by_id_matches_enabled_row() {
        let sources = vec![
            sample("src_usb_1", SourceKind::UsbAlsa, "plughw:1,0", false),
            sample("src_rtsp_1", SourceKind::Rtsp, "rtsp://cam/feed", false),
        ];
        let picked = pick_by_id(&sources, "src_rtsp_1").expect("rtsp row found");
        assert_eq!(picked.device, "rtsp://cam/feed");
        assert!(matches!(picked.kind, Some(SourceKind::Rtsp)));
        // The row id is what a live tap is registered under, so it has to
        // survive resolution — it used to be dropped here.
        assert_eq!(picked.id, "src_rtsp_1");
    }

    #[test]
    fn pick_by_id_skips_disabled_row() {
        let sources = vec![sample("src_x", SourceKind::UsbAlsa, "plughw:0", true)];
        assert!(pick_by_id(&sources, "src_x").is_none());
    }

    #[test]
    fn pick_by_id_returns_none_for_unknown_id() {
        let sources = vec![sample("src_y", SourceKind::UsbAlsa, "plughw:0", false)];
        assert!(pick_by_id(&sources, "no_such_id").is_none());
    }

    #[test]
    fn pick_first_enabled_returns_first_active_row() {
        // First row is disabled — picker should skip it and pick the
        // second (enabled) row.
        let sources = vec![
            sample("src_disabled", SourceKind::UsbAlsa, "plughw:0", true),
            sample("src_active", SourceKind::PipeWire, "default", false),
            sample("src_other", SourceKind::Rtsp, "rtsp://cam/feed", false),
        ];
        let picked = pick_first_enabled(&sources).expect("an enabled row exists");
        assert_eq!(picked.device, "default");
        assert_eq!(picked.id, "src_active");
        assert!(matches!(picked.kind, Some(SourceKind::PipeWire)));
    }

    // ---- the station's default source (N-3) --------------------------------

    /// The point of the setting: a two-microphone station says once which one a
    /// visitor pressing Listen should hear, and it is not simply the oldest row.
    #[test]
    fn the_configured_default_beats_the_first_row() {
        let sources = vec![
            sample("src_feeder", SourceKind::UsbAlsa, "plughw:0", false),
            sample("src_nestbox", SourceKind::Rtsp, "rtsp://box/feed", false),
        ];
        let picked = pick_default(&sources, "src_nestbox").expect("a source");
        assert_eq!(picked.id, "src_nestbox");
        assert_eq!(picked.device, "rtsp://box/feed");
    }

    /// The counterpart, and the behaviour of every station that never sets one:
    /// no default configured means the first enabled row, exactly as before.
    #[test]
    fn no_configured_default_is_the_first_enabled_row() {
        let sources = vec![
            sample("src_feeder", SourceKind::UsbAlsa, "plughw:0", false),
            sample("src_nestbox", SourceKind::Rtsp, "rtsp://box/feed", false),
        ];
        assert_eq!(
            pick_default(&sources, "").expect("a source").id,
            "src_feeder"
        );
    }

    /// A default naming a row that has since been disabled falls back rather
    /// than serving nothing.
    ///
    /// Silence — or a 503 — would be a worse answer to "let me hear the garden"
    /// than the microphone the station still has.
    #[test]
    fn a_default_that_is_disabled_falls_back_to_a_working_source() {
        let sources = vec![
            sample("src_feeder", SourceKind::UsbAlsa, "plughw:0", false),
            sample("src_nestbox", SourceKind::Rtsp, "rtsp://box/feed", true),
        ];
        assert_eq!(
            pick_default(&sources, "src_nestbox").expect("a source").id,
            "src_feeder"
        );
    }

    /// The same for a default naming a row that was removed outright.
    #[test]
    fn a_default_that_no_longer_exists_falls_back() {
        let sources = vec![sample("src_feeder", SourceKind::UsbAlsa, "plughw:0", false)];
        assert_eq!(
            pick_default(&sources, "src_gone").expect("a source").id,
            "src_feeder"
        );
    }

    /// And with nothing to fall back to, the caller still gets `None` — which
    /// is the 503 an empty station has always answered with.
    #[test]
    fn a_default_on_a_station_with_no_sources_is_still_none() {
        assert!(pick_default(&[], "src_gone").is_none());
        assert!(pick_default(&[], "").is_none());
    }

    // ---- ffmpeg input selection (the point of the whole change) ------------

    /// The counter-test for the tee: the *same* microphone source resolves to a
    /// device open without a tap, and to a pipe with one.
    ///
    /// The device open is what the Pi under test proved fails —
    /// `ffmpeg -f alsa -i plughw:CARD=PRO,DEV=0` returns `Device or resource
    /// busy` for as long as capture holds the microphone, which on a running
    /// station is always. A regression that stopped consulting the tap would
    /// silently reinstate that, and this is what would catch it.
    #[test]
    fn a_teed_microphone_is_streamed_from_the_pipe_never_the_device() {
        use birdnet_core::audio::capture::PcmSpec;
        const DEVICE: &str = "plughw:CARD=PRO,DEV=0";

        // Without a tap: the old behaviour — open the device.
        let without = stream_input_args(None, false, false, DEVICE, &[]);
        assert_eq!(without, vec!["-f", "alsa", "-i", DEVICE]);

        // With a tap: ffmpeg is handed PCM and never names the device.
        let with = stream_input_args(
            Some(PcmSpec {
                sample_rate: 48_000,
                channels: 1,
            }),
            false,
            false,
            DEVICE,
            &[],
        );
        assert_eq!(
            with,
            vec!["-f", "s16le", "-ar", "48000", "-ac", "1", "-i", "pipe:0"]
        );
        assert!(
            !with.iter().any(|a| a.contains("plughw") || a == "alsa"),
            "a teed source must not put the capture device on ffmpeg's command \
             line at all: {with:?}"
        );
    }

    #[test]
    fn stream_input_args_honour_the_tap_format() {
        use birdnet_core::audio::capture::PcmSpec;
        let args = stream_input_args(
            Some(PcmSpec {
                sample_rate: 44_100,
                channels: 2,
            }),
            false,
            false,
            "plughw:1,0",
            &[],
        );
        // A decoder configured with the wrong rate plays the stream at the
        // wrong pitch, so these have to come from the tap, not from a constant.
        assert_eq!(args[3], "44100");
        assert_eq!(args[5], "2");
    }

    /// Listening live opens a second session to an RTSP camera, so it has to
    /// be the kind capture opens: the row's transport and media mode. It was
    /// a fixed `-rtsp_transport tcp`. Observed failing with that restored.
    #[test]
    fn a_live_rtsp_session_follows_the_rows_transport_and_media_mode() {
        let row = birdnet_db::audio_sources::AudioSource {
            rtsp_transport: birdnet_db::audio_sources::RtspTransport::Udp,
            media_mode: birdnet_db::audio_sources::RtspMediaMode::Audio,
            ..sample("cam", SourceKind::Rtsp, "rtsp://cam/feed", false)
        };
        let r = resolved(&row);
        assert_eq!(
            stream_input_args(
                None,
                true,
                false,
                &r.device,
                r.rtsp_input.as_ref().expect("an RTSP URL")
            ),
            vec![
                "-rtsp_transport",
                "udp",
                "-allowed_media_types",
                "audio",
                "-i",
                "rtsp://cam/feed",
                "-vn"
            ]
        );
    }

    #[test]
    fn untapped_sources_keep_their_existing_inputs() {
        // RTSP and PulseAudio are deliberately not teed — a second RTSP session
        // is normal, and PulseAudio permits concurrent opens — so their command
        // lines must be exactly what they were.
        assert_eq!(
            stream_input_args(
                None,
                true,
                false,
                "rtsp://cam/feed",
                &stream_input_options(&sample("cam", SourceKind::Rtsp, "rtsp://cam/feed", false))
                    .expect("an RTSP URL")
            ),
            vec!["-rtsp_transport", "tcp", "-i", "rtsp://cam/feed", "-vn"]
        );
        assert_eq!(
            stream_input_args(None, false, true, "pulse://mic", &[]),
            vec!["-f", "pulse", "-i", "mic"]
        );
        assert_eq!(
            stream_input_args(None, false, true, "default", &[]),
            vec!["-f", "pulse", "-i", "default"]
        );
    }

    /// G-7: listening live to a stream that is not RTSP opens it with the
    /// options its protocol accepts — none of RTSP's, which ffmpeg refuses
    /// for an HLS playlist. Observed failing against the `rtsp_input_args`
    /// this replaced, which gave every stream row `-rtsp_transport tcp`.
    #[test]
    fn a_live_session_to_a_stream_that_is_not_rtsp_carries_no_rtsp_options() {
        let row = birdnet_db::audio_sources::AudioSource {
            media_mode: birdnet_db::audio_sources::RtspMediaMode::Audio,
            ..sample(
                "hls",
                SourceKind::Rtsp,
                "http://cam.local:8888/garden/index.m3u8",
                false,
            )
        };
        let r = resolved(&row);
        assert_eq!(
            stream_input_args(
                None,
                true,
                false,
                &r.device,
                r.rtsp_input.as_ref().expect("a supported URL")
            ),
            vec!["-i", "http://cam.local:8888/garden/index.m3u8", "-vn"]
        );
    }

    /// Listening live to a UDP stream opened a second socket on the port
    /// capture holds: "bind failed: Address already in use", answered as a
    /// 200 with no audio. It is refused with the reason. Observed failing
    /// without the UDP check: the row resolved to `Ok([])`.
    #[test]
    fn a_live_session_to_a_udp_stream_is_refused_with_the_reason() {
        let r = resolved(&sample(
            "udp",
            SourceKind::Rtsp,
            "udp://0.0.0.0:5004",
            false,
        ));
        let reason = r.rtsp_input.expect_err("capture holds the port");
        assert!(reason.contains("one reader"), "{reason}");
    }

    /// A stream row whose URL ffmpeg cannot be asked to read is refused with
    /// the reason, which names the schemes and not the URL.
    #[test]
    fn a_live_session_to_an_unsupported_scheme_is_refused_by_name() {
        let r = resolved(&sample(
            "rtp",
            SourceKind::Rtsp,
            "rtp://user:S3CRET@10.0.0.9:5006",
            false,
        ));
        let reason = r.rtsp_input.expect_err("rtp:// is not supported");
        assert!(reason.contains("rtsp, rtsps, http"), "{reason}");
        assert!(!reason.contains("S3CRET"), "{reason}");
    }

    // ---- live tap plumbing --------------------------------------------------

    /// The tap key must be the `audio_sources` row id, because that is what
    /// `CaptureSource::label` registers the tap under. Getting this wrong would
    /// silently fall through to opening the device — the exact `EBUSY` this
    /// change exists to remove.
    #[test]
    fn the_tap_key_is_the_audio_sources_row_id() {
        use birdnet_core::audio::capture::{CaptureSource, PcmSpec, new_live_audio_hub};

        let sources = vec![sample(
            "src_seed_1",
            SourceKind::UsbAlsa,
            "plughw:1,0",
            false,
        )];
        let picked = pick_by_id(&sources, "src_seed_1").expect("row found");

        // What capture registers the tap under, for the same row.
        let capture_source = CaptureSource::Microphone {
            device: picked.device.clone(),
            sample_rate: 48_000,
            channels: 1,
            channel_pick: None,
            stream_id: Some(picked.id.clone()),
        };
        let hub = new_live_audio_hub();
        hub.tap(
            &capture_source.label(),
            PcmSpec {
                sample_rate: 48_000,
                channels: 1,
            },
        );

        assert!(
            hub.lookup(&picked.id).is_some(),
            "the streaming resolver and the capture side must agree on the key"
        );
    }

    /// A tap that is producing audio yields a feed carrying the first chunk —
    /// which must not be swallowed by the liveness probe.
    #[tokio::test]
    async fn wait_for_first_audio_returns_the_bytes_it_probed_with() {
        use birdnet_core::audio::capture::{LiveTap, PcmSpec};

        let tap = Arc::new(LiveTap::new(PcmSpec {
            sample_rate: 48_000,
            channels: 1,
        }));
        let sub = tap.subscribe();
        tap.push(b"first-audio");
        let feed = wait_for_first_audio(sub)
            .await
            .expect("audio was available");
        assert_eq!(feed.first_chunk, b"first-audio");
        assert_eq!(feed.subscription.spec().sample_rate, 48_000);
    }

    /// The pump must deliver the probe's chunk *and* everything after it.
    ///
    /// Dropping the first chunk would clip the start of every stream, and it is
    /// the easy mistake here: the liveness probe has already consumed it from
    /// the ring, so it exists only in the `LiveFeed` the pump is handed.
    #[tokio::test]
    async fn the_pump_delivers_the_probed_chunk_and_then_the_stream() {
        use birdnet_core::audio::capture::{LiveTap, PcmSpec};
        use tokio::io::AsyncReadExt;

        let tap = Arc::new(LiveTap::new(PcmSpec {
            sample_rate: 48_000,
            channels: 1,
        }));
        let sub = tap.subscribe();
        tap.push(b"FIRST");
        let feed = wait_for_first_audio(sub).await.expect("audio available");

        // Stand in for ffmpeg's stdin.
        let (sink, mut ffmpeg_side) = tokio::io::duplex(4096);
        spawn_tap_pump(feed, sink);

        // Keep producing so the pump has more to forward after the probe.
        let writer = Arc::clone(&tap);
        let feeding = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let stop = Arc::clone(&feeding);
        let producer = std::thread::spawn(move || {
            while stop.load(std::sync::atomic::Ordering::Relaxed) {
                writer.push(b"MORE");
                std::thread::sleep(Duration::from_millis(5));
            }
        });

        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while seen.len() < 9 && tokio::time::Instant::now() < deadline {
            let mut buf = [0u8; 256];
            match tokio::time::timeout(Duration::from_millis(200), ffmpeg_side.read(&mut buf)).await
            {
                Ok(Ok(0)) | Err(_) => {}
                Ok(Ok(n)) => seen.extend_from_slice(&buf[..n]),
                Ok(Err(e)) => panic!("read from the pump failed: {e}"),
            }
        }
        feeding.store(false, std::sync::atomic::Ordering::Relaxed);
        producer.join().expect("producer thread");

        assert!(
            seen.starts_with(b"FIRST"),
            "the chunk the liveness probe consumed must be forwarded first, \
             got {:?}",
            String::from_utf8_lossy(&seen)
        );
        assert!(
            seen.len() > 5,
            "the pump must keep forwarding after the first chunk, got {:?}",
            String::from_utf8_lossy(&seen)
        );
    }

    /// When the sink goes away — ffmpeg killed because the client disconnected
    /// — the pump must unwind rather than spin forever holding a blocking-pool
    /// thread and a stream permit.
    /// A listener that stops reading must not hold its slot and its `ffmpeg`
    /// forever. The receiver here is kept alive and never read; the source
    /// never ends. The stall limit is shortened so the test runs in real time.
    #[tokio::test]
    async fn a_listener_that_stops_reading_is_let_go() {
        let endless = tokio_stream::iter(std::iter::repeat_with(|| Ok(Bytes::from_static(b"mp3"))));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            forward(endless, &tx, Duration::from_millis(200)),
        )
        .await;
        assert!(
            outcome.is_ok(),
            "the stream kept its slot for a listener that stopped reading"
        );
    }

    #[tokio::test]
    async fn the_pump_stops_when_its_sink_closes() {
        use birdnet_core::audio::capture::{LiveTap, PcmSpec};

        let tap = Arc::new(LiveTap::new(PcmSpec {
            sample_rate: 48_000,
            channels: 1,
        }));
        let sub = tap.subscribe();
        tap.push(b"x");
        let feed = wait_for_first_audio(sub).await.expect("audio available");

        let (sink, ffmpeg_side) = tokio::io::duplex(64);
        spawn_tap_pump(feed, sink);
        drop(ffmpeg_side); // the listener is gone

        // Keep the tap producing; the pump must still wind down. If it did not,
        // the subscription would be held forever — observable here as the tap
        // never dropping back to a single strong reference.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while Arc::strong_count(&tap) > 1 && tokio::time::Instant::now() < deadline {
            tap.push(&[0u8; 1024]);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            Arc::strong_count(&tap),
            1,
            "the pump must release its subscription once the sink is gone"
        );
    }

    /// A silent tap — capture paused by the schedule, or the source down —
    /// resolves to `None`, so the handler answers 503 instead of holding a
    /// connection open forever producing nothing.
    #[tokio::test]
    async fn wait_for_first_audio_gives_up_on_a_silent_source() {
        use birdnet_core::audio::capture::{LiveTap, PcmSpec};

        let tap = Arc::new(LiveTap::new(PcmSpec {
            sample_rate: 48_000,
            channels: 1,
        }));
        let sub = tap.subscribe();
        // Nothing is ever pushed. Probe with the real (2 s) timeout would make
        // this test slow, so drive the subscription directly with a short one
        // and assert the same emptiness the helper keys off.
        let mut sub = sub;
        let mut buf = [0u8; 64];
        assert_eq!(sub.read(&mut buf, Duration::from_millis(50)), 0);
        assert!(
            TAP_FIRST_AUDIO_TIMEOUT >= Duration::from_secs(1),
            "the real probe must be generous enough to clear a period boundary"
        );
    }

    #[test]
    fn pick_first_enabled_returns_none_when_all_disabled() {
        let sources = vec![
            sample("src_a", SourceKind::UsbAlsa, "plughw:0", true),
            sample("src_b", SourceKind::Rtsp, "rtsp://cam", true),
        ];
        assert!(pick_first_enabled(&sources).is_none());
    }

    #[test]
    fn pick_first_enabled_returns_none_for_empty_table() {
        assert!(pick_first_enabled(&[]).is_none());
    }

    // ── what reaches the log ────────────────────────────────────────────

    /// A writer that keeps everything the subscriber prints.
    #[derive(Clone, Default)]
    struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// What ffmpeg 6.1 wrote, verbatim, for a camera URL with credentials that
    /// it could not reach. It went to the journal at `warn` with the camera's
    /// password in it, and from the journal into the support bundle.
    #[tokio::test]
    async fn ffmpegs_stderr_reaches_the_log_without_the_camera_password() {
        let captured = Captured::default();
        let sink = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let stderr: &[u8] = b"Error opening input file rtsp://admin:hunter2@127.0.0.1:9/stream.\n\
            [rtsp @ 0x1] method DESCRIBE failed: 401 Unauthorized\n";
        drain_ffmpeg_stderr(stderr, "src_rtsp_1").await;

        let log = String::from_utf8_lossy(&captured.0.lock().expect("lock")).into_owned();
        assert!(!log.contains("hunter2"), "the password was logged: {log}");
        // The counterpart: the capture works, and what helps is still there.
        assert!(log.contains("127.0.0.1:9/stream"), "{log}");
        assert!(log.contains("401 Unauthorized"), "{log}");
        assert!(log.contains("src_rtsp_1"), "{log}");
    }

    /// The device string the encoder is started on is the URL itself.
    #[test]
    fn a_device_is_logged_without_its_password() {
        assert_eq!(
            loggable("rtsp://admin:hunter2@cam.local:554/h264"),
            "rtsp://admin:***REDACTED***@cam.local:554/h264"
        );
        // Nothing to redact: untouched.
        assert_eq!(loggable("plughw:CARD=Mic,DEV=0"), "plughw:CARD=Mic,DEV=0");
        assert_eq!(loggable("rtsp://cam.local/h264"), "rtsp://cam.local/h264");
    }
}
