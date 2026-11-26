//! A station started from the built binary, for the tests that break one on
//! purpose and read what it did and what it said (MDL-18).
//!
//! The in-process tests build an `AppState` and call the router, or call the
//! detection daemon's functions directly; none of them run `main`, the
//! start-up order in `src/app.rs`, the classifier plan, or the way the pieces
//! hand each other what they loaded. A real station's two worst faults on
//! 2026-11-25 — one damaged optional file stopping all detection, and a
//! second classifier that judged nothing — lived exactly there. So this
//! starts `env!("CARGO_BIN_EXE_birdnet-behavior")` with a configuration file
//! in a temporary directory, on a free port, with nothing from the machine's
//! environment, and talks to it the way an operator would: it drops a
//! recording into the watch folder and reads the metrics, the database and
//! the log.
//!
//! # What makes it independent of the machine
//!
//! * **Ports**: the kernel picks one (`127.0.0.1:0`) for each station.
//! * **Files**: everything (configuration, database, watch folder, models,
//!   `HOME`) is under one `tempfile::TempDir`, removed when the station drops.
//! * **Environment**: the child starts from an empty environment, so a
//!   developer's `BIRDNET_*` variables, a `~/.duckdb` extension cache, or a
//!   `BIRDNET_MODEL_DIR` cannot reach it. `PATH` is passed for the tools the
//!   station runs (`arecord`, `ffmpeg`), which it does not need here.
//! * **Time**: every wait is a deadline checked on this thread with
//!   `std::thread::sleep`, never a tokio timer (CLAUDE.md, "a test cannot time
//!   out on the runtime it is proving stuck"), and the recording's name holds
//!   a fixed date, so no branch depends on the day the test runs.
//! * **Disk**: the database and the recordings are a few megabytes; nothing
//!   reads how full the disk is, and the station's own disk checks only warn.
//!
//! # The fixture models
//!
//! The classifiers are the tiny ONNX models under
//! `crates/birdnet-core/src/testdata`, which load in milliseconds: the V2.4
//! and V3.0 shapes score 11 classes, so their labels files have 11 rows, and
//! `tiny_meta_test.onnx` is a geomodel of five species. They identify
//! nothing; they answer, and with `CONFIDENCE` low enough every window of the
//! magpie recording becomes a detection, which is what "the primary still
//! detects" needs.

// Each test file compiles this module and uses the part it needs.
#![allow(dead_code)]

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The binary under test.
pub const BIN: &str = env!("CARGO_BIN_EXE_birdnet-behavior");

/// How long a station may take to answer its health endpoint. A cold start
/// migrates a new database; on a loaded CI runner that has taken seconds.
const UP_WITHIN: Duration = Duration::from_secs(90);

/// The recording every station is handed: 30 s of a magpie, 48 kHz mono.
pub fn recording() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testdata/Pica_pica_30s.wav")
}

/// How much of [`recording`] a station is handed, in seconds: two of V2.4's
/// 3 s windows.
///
/// Not the whole 30 s. With the tiny models every class clears a low
/// confidence in every window, so the whole file is about 300 detections on
/// a two-classifier station, each one a clip the processor cuts after the
/// daemon has stopped — about 50 s on this machine, measured with the
/// released binary as well, which a stop then waits out. Six seconds keeps a
/// station's whole life inside a test's budget.
const HANDED_SECS: u32 = 6;

/// The first [`HANDED_SECS`] of [`recording`], written to `to`.
fn write_excerpt(to: &Path) {
    let mut reader = hound::WavReader::open(recording()).expect("open the recording");
    let spec = reader.spec();
    let keep =
        usize::try_from(spec.sample_rate * u32::from(spec.channels) * HANDED_SECS).expect("fits");
    let mut writer = hound::WavWriter::create(to, spec).expect("create the excerpt");
    for sample in reader.samples::<i16>().take(keep) {
        writer
            .write_sample(sample.expect("read a sample"))
            .expect("write a sample");
    }
    writer.finalize().expect("finish the excerpt");
}

/// A fixture model's bytes, by file name under `crates/birdnet-core/src/testdata`.
pub fn fixture_model(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/birdnet-core/src/testdata")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {}: {e}", path.display()))
}

/// A labels file of `n` invented species, in the V2.4 text form
/// (`Scientific_Common`), so a classifier scoring `n` classes has a name
/// for each.
pub fn labels_text(prefix: &str, n: usize) -> String {
    (0..n)
        .map(|i| format!("{prefix} sp{i}_{prefix} bird {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The tiny geomodel's own five species, in its order. Its scores do not
/// depend on the place or the week: at a threshold of 0.5 the magpie, the
/// raven and the robin pass.
pub const TINY_META_LABELS: &str = "Pica pica_Eurasian Magpie\nParus major_Great Tit\n\
    Corvus corax_Common Raven\nTurdus merula_Eurasian Blackbird\n\
    Erithacus rubecula_European Robin\n";

/// A directory a station is built in, before it starts.
pub struct StationDir {
    dir: tempfile::TempDir,
    settings: Vec<String>,
    env: Vec<(String, String)>,
}

impl StationDir {
    /// An empty one, with a database path, a watch folder and the settings
    /// every station here shares: a confidence low enough that the tiny
    /// models' answers are recorded, and no overlap.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("watch")).expect("watch dir");
        std::fs::create_dir_all(dir.path().join("models")).expect("models dir");
        std::fs::create_dir_all(dir.path().join("home")).expect("home dir");
        let settings = vec![
            "SITENAME=Harness".to_owned(),
            format!("DB_PATH={}", dir.path().join("data/birds.db").display()),
            format!("RECS_DIR={}", dir.path().join("watch").display()),
            "CONFIDENCE=0.01".to_owned(),
        ];
        Self {
            dir,
            settings,
            env: Vec::new(),
        }
    }

    /// Set an environment variable for the station's process (it starts
    /// from an empty environment otherwise).
    pub fn env(&mut self, key: &str, value: impl std::fmt::Display) -> &mut Self {
        self.env.push((key.to_owned(), value.to_string()));
        self
    }

    /// The directory itself.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Where a file named `name` goes in the station's model folder.
    pub fn model_path(&self, name: &str) -> PathBuf {
        self.dir.path().join("models").join(name)
    }

    /// Write `bytes` into the model folder as `name`, and return its path.
    pub fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.model_path(name);
        std::fs::write(&path, bytes).expect("write fixture");
        path
    }

    /// Add one `KEY=value` line to the configuration file.
    pub fn set(&mut self, key: &str, value: impl std::fmt::Display) -> &mut Self {
        self.settings.push(format!("{key}={value}"));
        self
    }

    /// The primary classifier: the tiny V2.4-shaped model and 11 labels.
    pub fn with_primary(&mut self) -> &mut Self {
        let model = self.put("primary.onnx", &fixture_model("tiny_v24_test.onnx"));
        let labels = self.put("primary_labels.txt", labels_text("Primus", 11).as_bytes());
        self.set("MODEL_PATH", model.display());
        self.set("LABELS_PATH", labels.display())
    }

    /// A second classifier, `MODEL_2_*`, with the bytes and labels given
    /// (`None` for labels leaves `MODEL_2_LABELS` naming a file that does
    /// not exist).
    pub fn with_second(
        &mut self,
        id: &str,
        bytes: Option<&[u8]>,
        labels: Option<&str>,
    ) -> &mut Self {
        let model = bytes.map_or_else(
            || self.model_path("second-not-there.onnx"),
            |b| self.put("second.onnx", b),
        );
        let labels_path = labels.map_or_else(
            || self.model_path("second_labels-not-there.txt"),
            |text| self.put("second_labels.txt", text.as_bytes()),
        );
        self.set("MODEL_2_PATH", model.display());
        self.set("MODEL_2_LABELS", labels_path.display());
        self.set("MODEL_2_ID", id)
    }

    /// The occurrence filter: the tiny geomodel and its labels, either of
    /// which may be replaced by bytes of the test's choosing or left missing.
    pub fn with_filter(&mut self, model: Option<&[u8]>, labels: Option<&str>) -> &mut Self {
        let model_path = model.map_or_else(
            || self.model_path("geo-not-there.onnx"),
            |b| self.put("geo.onnx", b),
        );
        let labels_path = labels.map_or_else(
            || self.model_path("geo_labels-not-there.txt"),
            |text| self.put("geo_labels.txt", text.as_bytes()),
        );
        self.set("METADATA_MODEL_PATH", model_path.display());
        self.set("METADATA_LABELS_PATH", labels_path.display());
        self.set("SF_THRESH", "0.5")
    }

    /// A position (London), so the occurrence filter can run.
    pub fn with_coordinates(&mut self) -> &mut Self {
        self.set("LATITUDE", "51.48");
        self.set("LONGITUDE", "0.0")
    }

    /// Write the configuration and start the binary on a free port; return
    /// once it answers its health endpoint.
    ///
    /// # Panics
    ///
    /// If it exits or does not answer within [`UP_WITHIN`], with its log.
    pub fn start(self) -> Station {
        let config = self.dir.path().join("birdnet.conf");
        std::fs::write(&config, self.settings.join("\n") + "\n").expect("write config");
        let port = free_port();
        let log_path = self.dir.path().join("station.log");
        let log = std::fs::File::create(&log_path).expect("log file");
        let child = Command::new(BIN)
            .args([
                "--config",
                &config.display().to_string(),
                "--listen",
                &format!("127.0.0.1:{port}"),
                "--offline",
            ])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.dir.path().join("home"))
            .env("RUST_LOG", "info")
            .env("NO_COLOR", "1")
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(log.try_clone().expect("log handle"))
            .stderr(log)
            .spawn()
            .unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
        let mut station = Station {
            child: Some(child),
            port,
            dir: self.dir,
            log_path,
            handed: 0,
        };
        station.wait_until_up();
        station
    }
}

/// A running station.
pub struct Station {
    child: Option<Child>,
    port: u16,
    dir: tempfile::TempDir,
    log_path: PathBuf,
    handed: u32,
}

impl Station {
    fn wait_until_up(&mut self) {
        let deadline = Instant::now() + UP_WITHIN;
        loop {
            if let Some(status) = self
                .child
                .as_mut()
                .and_then(|c| c.try_wait().ok().flatten())
            {
                panic!(
                    "the station exited during start-up with {status}:\n{}",
                    self.log()
                );
            }
            if self
                .get("/api/v2/health")
                .is_some_and(|(status, _)| status > 0)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the station did not answer within {UP_WITHIN:?}:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// The station's port.
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Everything the station has logged so far.
    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    /// `GET path`: the status code and the body, or `None` when nothing
    /// answered.
    pub fn get(&self, path: &str) -> Option<(u16, String)> {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).ok()?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .ok()?;
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .ok()?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).ok()?;
        let text = String::from_utf8_lossy(&buf).into_owned();
        let status = text
            .lines()
            .next()?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()?;
        let body = text
            .split_once("\r\n\r\n")
            .map_or_default(|(_, b)| b.to_owned());
        Some((status, dechunk(&body)))
    }

    /// How many times the process has started: a restart in place runs the
    /// program again under the same process id and the same log.
    pub fn starts(&self) -> usize {
        self.log().matches("starting BirdNet-Behavior").count()
    }

    /// How many lines of the log contain `text`.
    pub fn log_lines(&self, text: &str) -> usize {
        self.log().lines().filter(|l| l.contains(text)).count()
    }

    /// The value of one series now, or `None` when the station does not
    /// answer (it may be restarting) or does not export it.
    pub fn metric_now(&self, series: &str) -> Option<f64> {
        match self.get("/api/v2/metrics") {
            Some((200, body)) => series_value(&body, series),
            _ => None,
        }
    }

    /// Wait until `series` reads 1, or `within` has passed; whether it did.
    /// Tolerates a station that is restarting and not answering.
    pub fn wait_for_state(&self, series: &str, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        loop {
            if self.metric_now(series) == Some(1.0) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Whether the process has exited.
    pub fn has_exited(&mut self) -> bool {
        self.child
            .as_mut()
            .is_some_and(|c| c.try_wait().ok().flatten().is_some())
    }

    /// The Prometheus exposition.
    pub fn metrics(&self) -> String {
        match self.get("/api/v2/metrics") {
            Some((200, body)) => body,
            other => panic!("GET /api/v2/metrics answered {other:?}"),
        }
    }

    /// The value of one series, named with its labels exactly as exposed
    /// (`birdnet_detector_state{state="running"}`), or `None` when the
    /// exposition does not have it.
    pub fn metric(&self, series: &str) -> Option<f64> {
        series_value(&self.metrics(), series)
    }

    /// Every series of a metric family whose value is 1, as the label set
    /// between the braces: for a state-set gauge, the state it is in.
    pub fn states_of(&self, family: &str) -> Vec<String> {
        let text = self.metrics();
        let prefix = format!("{family}{{");
        text.lines()
            .filter_map(|l| l.strip_prefix(&prefix))
            .filter_map(|rest| rest.rsplit_once("} "))
            .filter(|(_, v)| v.trim() == "1")
            .map(|(labels, _)| labels.to_owned())
            .collect()
    }

    /// Drop the magpie recording (its first [`HANDED_SECS`]) into the watch
    /// folder under a name of the station's own form, with a fixed date and
    /// `source` as its source id when one is given, written beside it first
    /// and renamed so the watcher never sees half a file.
    pub fn hand_recording(&mut self, source: Option<&str>) {
        self.handed += 1;
        let minute = self.handed;
        let name = source.map_or_else(
            || format!("2026-05-19-birdnet-09:{minute:02}:00.wav"),
            |s| format!("2026-05-19-birdnet-{s}-09:{minute:02}:00.wav"),
        );
        let watch = self.dir.path().join("watch");
        let part = watch.join(format!(".{name}.part"));
        write_excerpt(&part);
        std::fs::rename(&part, watch.join(&name)).expect("hand the recording over");
    }

    /// Files the detection daemon has finished analysing, over every source.
    pub fn files_analysed(&self) -> u64 {
        self.metrics()
            .lines()
            .filter(|l| l.starts_with("birdnet_files_analysed_total"))
            .filter_map(|l| l.rsplit_once(' ').and_then(|(_, v)| v.parse::<u64>().ok()))
            .sum()
    }

    /// Wait until `n` files have been analysed, or `within` has passed;
    /// return how many were.
    pub fn wait_for_analysed(&self, n: u64, within: Duration) -> u64 {
        let deadline = Instant::now() + within;
        loop {
            let done = self.files_analysed();
            if done >= n || Instant::now() >= deadline {
                return done;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Wait until the database holds detections and has stopped gaining them
    /// for two seconds, or `within` has passed.
    ///
    /// The processor writes after the analyser counts the file, one row at a
    /// time, and the rows of a second classifier can come after the first's;
    /// reading at the first row would make "only the primary detected" true
    /// by timing.
    pub fn wait_for_detections(&self, within: Duration) -> Vec<(String, String)> {
        const QUIET: Duration = Duration::from_secs(2);
        let deadline = Instant::now() + within;
        let mut last = (0, Instant::now());
        loop {
            let rows = self.detections();
            if rows.len() != last.0 {
                last = (rows.len(), Instant::now());
            }
            if (!rows.is_empty() && last.1.elapsed() >= QUIET) || Instant::now() >= deadline {
                return rows;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Every detection in the database as `(scientific name, classifier)`.
    pub fn detections(&self) -> Vec<(String, String)> {
        let path = self.dir.path().join("data/birds.db");
        let Ok(conn) = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            return Vec::new();
        };
        let Ok(mut stmt) =
            conn.prepare("SELECT Sci_Name, COALESCE(model_id, '') FROM detections ORDER BY rowid")
        else {
            return Vec::new();
        };
        // Not yet there (the processor has not written) reads as none: every
        // caller polls until what it waits for appears.
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_or_else(|_| Vec::new(), |rows| rows.filter_map(Result::ok).collect())
    }

    /// The classifiers detections were recorded under, without repeats.
    pub fn detecting_classifiers(&self) -> std::collections::BTreeSet<String> {
        self.detections().into_iter().map(|(_, id)| id).collect()
    }

    /// Stop the station the way systemd does (SIGTERM) and wait for it.
    ///
    /// # Panics
    ///
    /// If it is still running 30 s later.
    pub fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status();
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!(
            "the station did not stop within 30 s of SIGTERM:\n{}",
            self.log()
        );
    }
}

impl Drop for Station {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A loopback HTTP origin for model files: `GET /<name>` answers the file of
/// that name in the directory it serves, whole, and 404 for anything else.
/// Reserved first and served later, so a test can have a station retry an
/// origin that is down and then see it come back.
pub struct Mirror {
    port: u16,
}

impl Mirror {
    /// A port for the mirror, not yet listening.
    pub fn reserve() -> Self {
        Self { port: free_port() }
    }

    /// The base URL to set as `BIRDNET_MODEL_MIRROR`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Start serving `dir` on the reserved port, on a thread of its own.
    pub fn serve(&self, dir: &Path) {
        let listener = TcpListener::bind(("127.0.0.1", self.port)).expect("bind the mirror");
        let dir = dir.to_path_buf();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(sock) = conn else { continue };
                let dir = dir.clone();
                std::thread::spawn(move || answer(sock, &dir));
            }
        });
    }
}

/// Answer one request to a [`Mirror`].
fn answer(mut sock: TcpStream, dir: &Path) {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match sock.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    let head = String::from_utf8_lossy(&head);
    let name = head
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .trim_start_matches('/')
        .to_owned();
    let file = (!name.is_empty() && !name.contains('/') && !name.contains(".."))
        .then(|| std::fs::read(dir.join(&name)).ok())
        .flatten();
    let _ = match file {
        Some(bytes) => sock
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .as_bytes(),
            )
            .and_then(|()| sock.write_all(&bytes)),
        None => sock
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
    };
}

/// A port nothing is listening on, chosen by the kernel.
pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("a free port")
}

/// The value of `series` in an exposition, matched as the whole name and
/// label set.
pub fn series_value(text: &str, series: &str) -> Option<f64> {
    text.lines().filter(|l| !l.starts_with('#')).find_map(|l| {
        let (name, value) = l.rsplit_once(' ')?;
        (name == series).then(|| value.parse().ok()).flatten()
    })
}

/// Undo HTTP/1.1 chunked transfer coding, when the body is chunked; a body
/// that is not is returned as it is.
fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    loop {
        let Some((size_line, after)) = rest.split_once("\r\n") else {
            return body.to_owned();
        };
        let Ok(size) = usize::from_str_radix(size_line.trim(), 16) else {
            return body.to_owned();
        };
        if size == 0 {
            return out;
        }
        if after.len() < size {
            return body.to_owned();
        }
        out.push_str(&after[..size]);
        rest = after[size..].trim_start_matches("\r\n");
    }
}
