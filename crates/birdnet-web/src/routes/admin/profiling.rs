//! CPU profiles over HTTP (`G-24`): `GET /admin/debug/pprof/profile`.
//!
//! "The Pi is at 100 % CPU and I don't know why" is one request with a
//! profiler and a week of guessing without one. This samples every thread's
//! stack 99 times a second for the asked duration and answers with what it
//! saw, in one of two formats:
//!
//! * `pprof` (the default): gzipped protobuf, the format `go tool pprof`,
//!   `pprof` itself and Speedscope open;
//! * `folded`: one line per distinct stack, `thread;outer;…;inner count`, which
//!   Speedscope and every flame-graph tool read as text.
//!
//! An SVG flame graph would need `pprof`'s `flamegraph` feature, which brings
//! in `inferno`; that crate is CDDL-1.0, outside the licences `deny.toml`
//! allows, so the folded text is offered instead.
//!
//! # What it costs, and when
//!
//! Nothing is linked in unless the binary is built with the `profiling`
//! feature, and nothing samples until an administrator asks: the profiler
//! arms a `SIGPROF` interval timer for the request's duration and disarms it
//! when the profile is taken. One profile at a time; a second request while
//! one runs is a `409`. Sampling interrupts system calls with `EINTR`, which
//! the standard library retries; the timer is not inherited by the `ffmpeg`
//! and `arecord` children.
//!
//! There is no heap profile. Rust has no allocator-agnostic one: it would
//! mean replacing the global allocator with jemalloc built for profiling,
//! which is a change to every build, not a feature flag.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::extract::Query;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};
use serde::Deserialize;

use crate::auth_middleware::{RequestUser, require_admin};
use crate::state::AppState;

/// Seconds sampled when the request does not say: Go's default.
const DEFAULT_SECONDS: u64 = 30;
/// The longest profile a request may ask for.
const MAX_SECONDS: u64 = 120;
/// Samples a second: off the 100 Hz of most periodic work, so the samples
/// do not fall in step with it.
const FREQUENCY_HZ: i32 = 99;

/// Whether a profile is being taken. The profiler is process-wide.
static PROFILING: AtomicBool = AtomicBool::new(false);

/// Holds [`PROFILING`] for as long as it lives, released however the
/// profile ends — a panic in the sampling thread included.
struct Claim;

impl Claim {
    fn take() -> Option<Self> {
        (!PROFILING.swap(true, Ordering::AcqRel)).then_some(Self)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        PROFILING.store(false, Ordering::Release);
    }
}

/// Mount the profiling route.
pub fn router() -> Router<AppState> {
    Router::new().route("/admin/debug/pprof/profile", get(cpu_profile))
}

/// Query parameters for a profile.
#[derive(Debug, Deserialize)]
struct ProfileQuery {
    seconds: Option<String>,
    format: Option<String>,
}

/// The two answers a profile can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Pprof,
    Folded,
}

/// Read a request: `seconds` from 1 to [`MAX_SECONDS`] (default
/// [`DEFAULT_SECONDS`]) and `format` `pprof` (default) or `folded`. `Err` is
/// the message for the 400.
fn parse(q: &ProfileQuery) -> Result<(Duration, Format), String> {
    let seconds = match q.seconds.as_deref() {
        None | Some("") => DEFAULT_SECONDS,
        Some(s) => s
            .parse::<u64>()
            .ok()
            .filter(|n| (1..=MAX_SECONDS).contains(n))
            .ok_or_else(|| format!("seconds must be a whole number from 1 to {MAX_SECONDS}"))?,
    };
    let format = match q.format.as_deref() {
        None | Some("" | "pprof") => Format::Pprof,
        Some("folded") => Format::Folded,
        Some(_) => return Err("format must be pprof or folded".to_owned()),
    };
    Ok((Duration::from_secs(seconds), format))
}

/// One line per distinct stack, `thread;outermost;…;innermost count`, sorted
/// so the same profile reads the same. A `;` inside a name would end its
/// frame early, so it is written as `,`. An idle process is not sampled at
/// all (the timer counts CPU time), and its profile is empty, not one blank
/// stack.
fn fold(stacks: impl IntoIterator<Item = (String, Vec<String>, isize)>) -> String {
    let mut lines: Vec<String> = stacks
        .into_iter()
        .map(|(thread, frames, count)| {
            let mut line = thread.replace(';', ",");
            for frame in frames {
                line.push(';');
                line.push_str(&frame.replace(';', ","));
            }
            format!("{line} {count}\n")
        })
        .collect();
    lines.sort_unstable();
    lines.concat()
}

/// A report's stacks, outermost frame first, for [`fold`].
fn folded(report: &pprof::Report) -> String {
    fold(report.data.iter().map(|(frames, count)| {
        let names = frames
            .frames
            .iter()
            .rev()
            .flat_map(|frame| frame.iter().rev().map(ToString::to_string))
            .collect();
        (frames.thread_name_or_id(), names, *count)
    }))
}

/// Sample for `duration`, then encode what was seen.
fn take_profile(duration: Duration, format: Format) -> Result<Vec<u8>, String> {
    use pprof::protos::Message as _;
    use std::io::Write as _;
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(FREQUENCY_HZ)
        // Unwinding through these from a signal handler can deadlock; a
        // sample landing in them is dropped instead.
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
        .map_err(|e| format!("the profiler could not start: {e}"))?;
    std::thread::sleep(duration);
    let report = guard
        .report()
        .build()
        .map_err(|e| format!("the profile could not be built: {e}"))?;
    drop(guard);
    match format {
        Format::Folded => Ok(folded(&report).into_bytes()),
        Format::Pprof => {
            let profile = report
                .pprof()
                .map_err(|e| format!("the profile could not be encoded: {e}"))?;
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gz.write_all(&profile.encode_to_vec())
                .and_then(|()| gz.finish())
                .map_err(|e| format!("the profile could not be compressed: {e}"))
        }
    }
}

/// `GET /admin/debug/pprof/profile?seconds=&format=` — a CPU profile of the
/// whole station, taken over `seconds` (default 30, at most 120).
///
/// Administrators only. `409` while another profile is being taken.
async fn cpu_profile(user: RequestUser, Query(q): Query<ProfileQuery>) -> Response {
    if let Err(denied) = require_admin(&user) {
        return *denied;
    }
    let (duration, format) = match parse(&q) {
        Ok(request) => request,
        Err(message) => return (StatusCode::BAD_REQUEST, message).into_response(),
    };
    let Some(claim) = Claim::take() else {
        return (
            StatusCode::CONFLICT,
            "a profile is already being taken; ask again when it has finished",
        )
            .into_response();
    };
    tracing::info!(
        seconds = duration.as_secs(),
        user = %user.user.username,
        "taking a CPU profile"
    );
    let result = tokio::task::spawn_blocking(move || {
        let _claim = claim;
        take_profile(duration, format)
    })
    .await;
    let stamp = crate::routes::pages::today_date_string();
    match result {
        Ok(Ok(bytes)) => {
            let (content_type, name) = match format {
                Format::Pprof => ("application/octet-stream", format!("cpu-{stamp}.pb.gz")),
                Format::Folded => ("text/plain; charset=utf-8", format!("cpu-{stamp}.folded")),
            };
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, content_type.to_owned()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{name}\""),
                    ),
                    (header::CACHE_CONTROL, "no-store".to_owned()),
                ],
                bytes,
            )
                .into_response()
        }
        Ok(Err(message)) => {
            tracing::warn!(error = %message, "CPU profile failed");
            (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            crate::routes::log_internal("profile task failed", &e),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The claim is process-wide, and tests run in parallel: each test that
    /// takes it holds this first.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn query(seconds: Option<&str>, format: Option<&str>) -> ProfileQuery {
        ProfileQuery {
            seconds: seconds.map(str::to_owned),
            format: format.map(str::to_owned),
        }
    }

    #[test]
    fn a_request_is_bounded_and_its_format_named() {
        assert_eq!(
            parse(&query(None, None)),
            Ok((Duration::from_secs(30), Format::Pprof))
        );
        assert_eq!(
            parse(&query(Some("1"), Some("folded"))),
            Ok((Duration::from_secs(1), Format::Folded))
        );
        assert_eq!(
            parse(&query(Some("120"), Some("pprof"))),
            Ok((Duration::from_secs(120), Format::Pprof))
        );
        for bad in ["0", "121", "-1", "ten"] {
            assert!(parse(&query(Some(bad), None)).is_err(), "{bad}");
        }
        assert!(parse(&query(None, Some("svg"))).is_err());
    }

    /// One profile at a time, and the claim is released when it ends.
    #[test]
    fn only_one_profile_runs_at_once() {
        let _serial = serial();
        let first = Claim::take().expect("free");
        assert!(
            Claim::take().is_none(),
            "a second is refused while one runs"
        );
        drop(first);
        assert!(Claim::take().is_some(), "and allowed once it has finished");
    }

    /// A real profile of a busy thread: both formats decode, and the folded
    /// text names the function that was busy. The thread spins past both
    /// profiles: the timer counts CPU time, so an idle process yields no
    /// samples at all.
    #[test]
    fn a_profile_sees_the_busy_function() {
        fn spin_for_the_profiler(until: std::time::Instant) -> u64 {
            let mut x = 0_u64;
            while std::time::Instant::now() < until {
                x = std::hint::black_box(x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1));
            }
            x
        }
        let _serial = serial();
        let _claim = Claim::take().expect("free");
        let spinner = std::thread::spawn(|| {
            spin_for_the_profiler(std::time::Instant::now() + Duration::from_millis(2_500))
        });
        let text = take_profile(Duration::from_secs(1), Format::Folded).expect("folded");
        let gz = take_profile(Duration::from_millis(500), Format::Pprof).expect("pprof");
        spinner.join().expect("spinner");

        let text = String::from_utf8(text).expect("utf-8");
        assert!(
            text.lines().any(|l| l.contains("spin_for_the_profiler")),
            "the busy function is in the profile:\n{}",
            text.lines().take(20).collect::<Vec<_>>().join("\n")
        );
        assert!(
            text.lines().all(|l| l
                .rsplit_once(' ')
                .is_some_and(|(_, n)| n.parse::<u64>().is_ok())),
            "every line ends in its count"
        );
        // Outermost first, as flame-graph tools read it: the thread's entry
        // comes before the function it was running.
        let line = text
            .lines()
            .find(|l| l.contains("spin_for_the_profiler"))
            .expect("a spinner line");
        let entry = ["start_thread", "thread_start"]
            .iter()
            .filter_map(|f| line.find(f))
            .min()
            .expect("the thread's entry frame is in its stack");
        assert!(
            entry < line.find("spin_for_the_profiler").unwrap_or(0),
            "{line}"
        );
        assert_eq!(&gz[..2], &[0x1f, 0x8b], "gzip");
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&gz[..]), &mut raw)
            .expect("gunzip");
        let profile =
            <pprof::protos::Profile as pprof::protos::Message>::decode(&raw[..]).expect("protobuf");
        assert!(!profile.sample.is_empty(), "it sampled something");
    }

    /// Stacks fold outermost first, sorted, each with its count; a `;` in a
    /// name cannot split a frame; and no samples — an idle station — is
    /// nothing, not one blank stack.
    #[test]
    fn stacks_fold_to_lines_and_nothing_folds_to_nothing() {
        assert_eq!(fold(Vec::new()), "");
        let folded = fold(vec![
            (
                "worker".to_owned(),
                vec!["main".to_owned(), "infer".to_owned()],
                7,
            ),
            ("a;b".to_owned(), vec!["<T as Trait>::f;g".to_owned()], 2),
        ]);
        assert_eq!(folded, "a,b;<T as Trait>::f,g 2\nworker;main;infer 7\n");
    }
}
