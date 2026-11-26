//! A bad network, injected: origins that stall, drop the connection, serve the
//! wrong bytes, answer 404, refuse to resume, or are not there at all (MDL-13,
//! MDL-12). Each test drives the real download path against [`Origin`] and
//! waits for it on an OS thread with a deadline, so a regression that hangs is
//! a red test, not a stuck run.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::origin::{Origin, Step, closed_port_url};
use super::{assert_no_part_files, digest_of};
use crate::model_catalog::download::{Network, staging_path};
use crate::model_catalog::{FetchPolicy, FileSpec, InstallError, Progress};

/// A body big enough to arrive in several reads.
fn body() -> Vec<u8> {
    (0..300_000_u32).map(|i| (i % 251) as u8).collect()
}

fn spec_for(body: &[u8], origins: &[&Origin]) -> FileSpec {
    spec_with_urls(body, origins.iter().map(|o| o.url.clone()).collect())
}

fn spec_with_urls(body: &[u8], origins: Vec<String>) -> FileSpec {
    FileSpec {
        name: "model.onnx".to_owned(),
        sha256: digest_of(body),
        bytes: body.len() as u64,
        origins,
        legacy: None,
        embedded: None,
        also_accepted: &[],
    }
}

/// Quick timeouts and `waits` between rounds.
fn policy(waits: &[u64]) -> FetchPolicy {
    FetchPolicy {
        connect_timeout: Duration::from_secs(2),
        stall_timeout: Duration::from_secs(1),
        retry_waits: waits.iter().map(|s| Duration::from_secs(*s)).collect(),
        mirror: None,
        ..FetchPolicy::default()
    }
}

/// Run a fetch on its own thread, and fail rather than hang if it takes
/// longer than `deadline`. Returns the result and how long it took.
fn fetch_within(
    deadline: Duration,
    policy: FetchPolicy,
    spec: FileSpec,
    dest: PathBuf,
) -> (Result<(), InstallError>, Vec<Progress>, Duration) {
    let (tx, rx) = mpsc::channel();
    let started = Instant::now();
    std::thread::spawn(move || {
        let mut seen = Vec::new();
        let r = Network::new(&policy).fetch(&spec, &dest, &mut |p| seen.push(p.clone()));
        let _ = tx.send((r, seen));
    });
    let (r, seen) = rx
        .recv_timeout(deadline)
        .unwrap_or_else(|_| panic!("the install was still waiting after {deadline:?}"));
    (r, seen, started.elapsed())
}

fn dest_in(dir: &Path) -> PathBuf {
    dir.join("model.onnx")
}

/// **A stalled origin is a failure after the stall timeout, not a six-hour
/// wait**, and the next round resumes from the byte it stopped at.
///
/// Observed failing with `.timeout(self.policy.stall_timeout)` removed from
/// the client: "the install was still waiting after 20s".
#[test]
fn an_origin_that_stalls_is_abandoned_and_the_transfer_resumed() {
    let body = body();
    let origin = Origin::start(vec![
        Step::StallAfter(body.clone(), 100_000, Duration::from_secs(60)),
        Step::Serve(body.clone()),
    ]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    let (r, _, took) = fetch_within(
        Duration::from_secs(20),
        policy(&[0]),
        spec_for(&body, &[&origin]),
        dest.clone(),
    );
    r.expect("the second round finishes the file");
    assert!(took < Duration::from_secs(15), "took {took:?}");
    assert_eq!(std::fs::read(&dest).expect("read"), body);
    let seen = origin.seen();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0].range_from, None);
    assert_eq!(
        seen[1].range_from,
        Some(100_000),
        "the second request resumes where the stall left off"
    );
    assert!(!staging_path(&dest).exists());
    assert_no_part_files(dir.path(), "a resumed stall");
}

/// An origin that answers nothing at all (not even headers) is abandoned
/// after the stall timeout too, and the next origin serves the file.
#[test]
fn an_origin_that_never_answers_is_abandoned_for_the_next() {
    let body = body();
    let silent = Origin::start(vec![Step::StallAfter(
        Vec::new(),
        0,
        Duration::from_secs(60),
    )]);
    let good = Origin::start(vec![Step::Serve(body.clone())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    let (r, _, took) = fetch_within(
        Duration::from_secs(20),
        policy(&[]),
        spec_for(&body, &[&silent, &good]),
        dest.clone(),
    );
    r.expect("the second origin serves it");
    assert!(took < Duration::from_secs(15), "took {took:?}");
    assert_eq!(std::fs::read(&dest).expect("read"), body);
}

/// **A connection dropped mid-file is resumed with a Range request**, and the
/// file that results is the whole, verified file.
///
/// Observed failing with the `Range` header not sent (every attempt from the
/// start): "assertion `left == right` failed" on the second request's range.
#[test]
fn a_dropped_connection_is_resumed_from_where_it_stopped() {
    let body = body();
    let origin = Origin::start(vec![
        Step::DropAfter(body.clone(), 120_000),
        Step::Serve(body.clone()),
    ]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    let (r, reports, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[0]),
        spec_for(&body, &[&origin]),
        dest.clone(),
    );
    r.expect("resumed and verified");
    assert_eq!(std::fs::read(&dest).expect("read"), body);
    let seen = origin.seen();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[1].range_from, Some(120_000));
    // Progress, which the dashboard draws, starts the second attempt where
    // the first left off and ends at the pinned size.
    assert!(reports.iter().any(|p| p.done == 120_000));
    assert_eq!(reports.last().map(|p| p.done), Some(body.len() as u64));
    assert!(reports.iter().all(|p| p.total == body.len() as u64));
}

/// A partial left by an earlier run (the process was killed) is resumed by
/// the next run, not thrown away.
#[test]
fn a_partial_from_an_earlier_run_is_resumed() {
    let body = body();
    let origin = Origin::start(vec![Step::Serve(body.clone())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    std::fs::write(staging_path(&dest), &body[..200_000]).expect("seed partial");
    let (r, _, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[]),
        spec_for(&body, &[&origin]),
        dest.clone(),
    );
    r.expect("resumed");
    assert_eq!(std::fs::read(&dest).expect("read"), body);
    assert_eq!(origin.seen()[0].range_from, Some(200_000));
}

/// **A resumed file that fails the hash is discarded and fetched again from
/// the start**: a partial whose bytes came from somewhere else is not
/// trusted because the rest of the file arrived.
///
/// Observed failing with the second attempt in `attempt_origin` disabled:
/// the install returned an error where a verified file was expected.
#[test]
fn a_resumed_file_that_fails_its_hash_is_fetched_again_from_the_start() {
    let body = body();
    let origin = Origin::start(vec![Step::Serve(body.clone())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    // The right length of prefix, the wrong bytes in it.
    std::fs::write(staging_path(&dest), vec![0xAB_u8; 150_000]).expect("seed bad partial");
    let (r, _, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[]),
        spec_for(&body, &[&origin]),
        dest.clone(),
    );
    r.expect("refetched from the start and verified");
    assert_eq!(std::fs::read(&dest).expect("read"), body);
    let seen = origin.seen();
    assert_eq!(
        seen.iter().map(|s| s.range_from).collect::<Vec<_>>(),
        vec![Some(150_000), None],
        "a resume, then the whole file"
    );
}

/// An origin that ignores the Range and sends the whole file with a 200 is
/// written from the start, not appended to the partial.
#[test]
fn an_origin_that_will_not_resume_starts_the_file_again() {
    let body = body();
    let origin = Origin::start(vec![Step::ServeIgnoringRange(body.clone())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    std::fs::write(staging_path(&dest), &body[..50_000]).expect("seed partial");
    let (r, _, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[]),
        spec_for(&body, &[&origin]),
        dest.clone(),
    );
    r.expect("the whole file, from the start");
    assert_eq!(std::fs::read(&dest).expect("read"), body);
}

/// **Wrong bytes from one origin, the right ones from the next**: the second
/// is installed, and the first — which served the wrong file from the start
/// — is not asked again in a later round.
///
/// Observed failing with `refused[i] = true` removed: the bad origin was
/// asked in every round (`seen().len()` was 2, not 1).
#[test]
fn an_origin_that_serves_the_wrong_bytes_falls_back_and_is_not_asked_again() {
    let body = body();
    let mut wrong = body.clone();
    wrong[10] ^= 0xFF;
    let bad = Origin::start(vec![Step::Serve(wrong)]);
    let good = Origin::start(vec![Step::Status(503), Step::Serve(body.clone())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    let (r, _, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[0, 0]),
        spec_for(&body, &[&bad, &good]),
        dest.clone(),
    );
    r.expect("the good origin's bytes are installed");
    assert_eq!(std::fs::read(&dest).expect("read"), body);
    assert_eq!(
        bad.seen().len(),
        1,
        "the wrong-bytes origin was asked again"
    );
    assert_eq!(good.seen().len(), 2, "503 once, then served");
    assert_no_part_files(dir.path(), "a fallback");
}

/// **A 404 moves on to the next origin** and is not asked again; a 503 is
/// asked again in the next round.
#[test]
fn an_origin_that_answers_404_is_passed_over_for_good() {
    let body = body();
    let missing = Origin::start(vec![Step::Status(404)]);
    let flaky = Origin::start(vec![Step::Status(503), Step::Serve(body.clone())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let (r, _, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[0, 0]),
        spec_for(&body, &[&missing, &flaky]),
        dest_in(dir.path()),
    );
    r.expect("served on the second round");
    assert_eq!(missing.seen().len(), 1);
    assert_eq!(flaky.seen().len(), 2);
}

/// **The first origin down, the second up**: installed from the second, in
/// the first round, without waiting.
///
/// Observed failing with `fetch` cut to its first origin
/// (`spec.origins.iter().take(1)`): "the install was still waiting after 20s"
/// (it was waiting out the round's 60 s before trying the dead origin again).
#[test]
fn a_first_origin_that_is_down_falls_back_to_the_second() {
    let body = body();
    let good = Origin::start(vec![Step::Serve(body.clone())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    let (r, _, took) = fetch_within(
        Duration::from_secs(20),
        policy(&[60]),
        spec_with_urls(&body, vec![closed_port_url(), good.url]),
        dest.clone(),
    );
    r.expect("the second origin serves it");
    assert!(
        took < Duration::from_secs(10),
        "it waited a round: {took:?}"
    );
    assert_eq!(std::fs::read(&dest).expect("read"), body);
}

/// The counterpart of the fallbacks: the first origin that verifies is the
/// only one contacted.
#[test]
fn the_first_origin_that_verifies_is_the_only_one_contacted() {
    let body = body();
    let first = Origin::start(vec![Step::Serve(body.clone())]);
    let second = Origin::start(vec![Step::Serve(body.clone())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let (r, _, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[]),
        spec_for(&body, &[&first, &second]),
        dest_in(dir.path()),
    );
    r.expect("installed");
    assert_eq!(second.seen().len(), 0, "the second origin was contacted");
}

/// **Retries are bounded**: with every origin failing, each is tried once per
/// round, the error names every origin, and what arrived is kept for the next
/// run to resume.
#[test]
fn retries_are_bounded_and_the_error_names_every_origin() {
    let body = body();
    let a = Origin::start(vec![Step::DropAfter(body.clone(), 1000)]);
    let b = Origin::start(vec![Step::Status(500)]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    let (r, _, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[0, 0]),
        spec_for(&body, &[&a, &b]),
        dest.clone(),
    );
    let err = r.expect_err("nothing serves the whole file");
    assert_eq!(a.seen().len(), 3, "three rounds");
    assert_eq!(b.seen().len(), 3, "three rounds");
    let text = err.to_string();
    let (pa, pb) = (
        text.find(&a.url).expect("the first origin is named"),
        text.find(&b.url).expect("the second origin is named"),
    );
    assert!(pa < pb, "{text}");
    assert!(!dest.exists());
    assert!(
        std::fs::metadata(staging_path(&dest)).is_ok_and(|m| m.len() > 0),
        "the partial is kept for the next run"
    );
}

/// A server that streams past the pinned size is stopped at it, and nothing
/// is kept: an origin that never stops cannot fill the disk.
#[test]
fn a_server_that_streams_past_the_pinned_size_is_stopped() {
    let origin = Origin::start(vec![Step::Endless(3200)]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    let mut spec = spec_for(b"x", &[&origin]);
    spec.bytes = 2 * 1024 * 1024;
    let (r, _, _) = fetch_within(Duration::from_secs(20), policy(&[0]), spec, dest.clone());
    let err = r.expect_err("more than the pin");
    assert!(
        matches!(err, InstallError::TooLarge { limit } if limit == 2 * 1024 * 1024),
        "{err:?}"
    );
    assert_eq!(
        origin.seen().len(),
        1,
        "an oversize origin is not asked again"
    );
    assert!(!dest.exists());
    assert!(!staging_path(&dest).exists(), "nothing of it is kept");
}

/// The counterpart: a file exactly the pinned size installs.
#[test]
fn a_file_of_exactly_the_pinned_size_installs() {
    let body = b"exactly this many bytes, pinned".to_vec();
    let origin = Origin::start(vec![Step::Serve(body.clone())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    let (r, _, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[]),
        spec_for(&body, &[&origin]),
        dest.clone(),
    );
    r.expect("the pinned size is allowed");
    assert_eq!(std::fs::read(&dest).expect("read"), body);
}

/// Two installs of one file at once: the second is refused while the first
/// holds the staging file, rather than writing into it.
#[test]
fn a_second_install_of_the_same_file_is_refused_while_one_runs() {
    let body = body();
    let origin = Origin::start(vec![Step::Serve(body.clone())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_in(dir.path());
    let held = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(staging_path(&dest))
        .expect("open");
    held.try_lock().expect("lock");
    let (r, _, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[]),
        spec_for(&body, &[&origin]),
        dest.clone(),
    );
    assert!(matches!(r, Err(InstallError::Busy { .. })), "{r:?}");
    assert_eq!(origin.seen().len(), 0, "nothing was requested");
    drop(held);
    let (r, _, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[]),
        spec_for(&body, &[&origin]),
        dest,
    );
    r.expect("once the first is done, it installs");
}

/// A stall is reported as one, and a refused connection with its cause: the
/// operator reading the error is told what the network did, not "error
/// sending request".
#[test]
fn a_failure_says_what_the_network_did() {
    let body = body();
    let stalls = Origin::start(vec![Step::StallAfter(
        body.clone(),
        1000,
        Duration::from_secs(60),
    )]);
    let dir = tempfile::tempdir().expect("tempdir");
    let (r, _, _) = fetch_within(
        Duration::from_secs(20),
        policy(&[]),
        spec_with_urls(&body, vec![stalls.url, closed_port_url()]),
        dest_in(dir.path()),
    );
    let text = r.expect_err("nothing serves it").to_string();
    assert!(text.contains("nothing arrived for 1 s"), "{text}");
    assert!(text.to_ascii_lowercase().contains("refused"), "{text}");
}

/// **An operator's mirror credentials never reach a log line or an error**
/// (MDL-22, review F20). A mirror URL with a password and a token in its
/// query, answering 404, is logged and reported; neither secret may appear in
/// anything the download wrote to the journal or returned, and the host and
/// path still do, so the failure stays diagnosable.
#[test]
fn a_mirrors_credentials_are_never_logged_or_reported() {
    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let body = body();
    let origin = Origin::start(vec![Step::Status(404)]);
    let secret_url = origin
        .url
        .replace("http://", "http://mirror-user:hunter2pw@")
        + "?token=s3cr3ttok&file=model";
    let spec = spec_with_urls(&body, vec![secret_url]);
    let dir = tempfile::tempdir().unwrap();
    let dest = dest_in(dir.path());
    let sink = Sink::default();
    let writer = sink.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let r = tracing::subscriber::with_default(subscriber, || {
            Network::new(&policy(&[])).fetch(&spec, &dest, &mut |_| {})
        });
        let _ = tx.send(r);
    });
    let r = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("the fetch ended");
    let err = r.expect_err("a 404 is not an install");
    let mut seen = String::from_utf8_lossy(&sink.0.lock().unwrap()).into_owned();
    seen.push_str(&format!("\n{err}\n{err:?}"));
    assert!(
        seen.contains("127.0.0.1"),
        "the host is still named: {seen}"
    );
    assert!(
        seen.contains("mirror-user"),
        "the account is still named: {seen}"
    );
    for secret in ["hunter2pw", "s3cr3ttok"] {
        assert!(
            !seen.contains(secret),
            "{secret} reached a log line or the error: {seen}"
        );
    }
}
