//! Fetching one file from its origins on a network that drops, stalls and
//! lies (MDL-12, MDL-13).
//!
//! * **Staging.** The bytes go to `<name>.download` beside the destination,
//!   held under an exclusive lock for as long as this install writes it, so a
//!   second install of the same file is refused rather than writing into it.
//!   The file is renamed into place only once all of it hashes to the pin.
//! * **Timeouts.** A connection gets [`FetchPolicy::connect_timeout`] to open,
//!   and an origin gets [`FetchPolicy::stall_timeout`] to answer and then
//!   between any two reads of the body. (`reqwest::blocking` applies its
//!   timeout to the response's arrival and to each read, not to the whole
//!   transfer, which is what makes it a stall timeout: a 541 MB download on a
//!   slow line can take hours, and is allowed to, as long as bytes arrive.)
//! * **Resume.** What a failed attempt received stays in the staging file. The
//!   next attempt — from the same origin or the next — asks for the rest with
//!   `Range: bytes=<have>-` and appends; the whole file, the old part with the
//!   new, is hashed once it is all there. An origin that ignores the range (200) starts the file again. A resumed
//!   file that does not hash to the pin is discarded and fetched once more from
//!   the start, from the same origin, because a resumed prefix from somewhere
//!   else is the likelier culprit than the origin.
//! * **Retries.** Every origin is tried in order, in up to
//!   `1 + retry_waits.len()` rounds with those waits between them. An origin
//!   that answered 404 or 410, or served the wrong bytes from the start, or
//!   more bytes than the pin, is not asked again. A full disk or a filesystem
//!   fault stops the install at once: it is not the origin's.

use std::fs::{self, File};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use super::record::hex;
use super::{FetchPolicy, FileSpec, InstallError, Progress, fits, free_space_bytes};

/// How this identifies itself when downloading.
const USER_AGENT: &str = "BirdNet-Behavior-ModelInstaller";

/// Bytes read from the network at a time.
const CHUNK: usize = 1024 * 1024;

/// What one attempt at one origin came to, when it was not success.
#[derive(Debug)]
enum Failure {
    /// Not the origin's fault; the install stops.
    Fatal(InstallError),
    /// This origin will not serve this file: it is not asked again.
    Refused(InstallError),
    /// It may work on the next round.
    Transient(InstallError),
    /// A resumed file failed its hash and was discarded; try this origin
    /// again from the start.
    ResumedBadly(InstallError),
}

/// The network half of one install: its policy and, once needed, its client.
#[derive(Debug)]
pub(super) struct Network {
    policy: FetchPolicy,
    client: Option<reqwest::blocking::Client>,
}

impl Network {
    /// A network that builds its client the first time a file is fetched,
    /// so an install that downloads nothing never builds one.
    pub(super) fn new(policy: &FetchPolicy) -> Self {
        Self {
            policy: policy.clone(),
            client: None,
        }
    }

    fn client(&mut self) -> Result<reqwest::blocking::Client, InstallError> {
        if let Some(c) = &self.client {
            return Ok(c.clone());
        }
        let built = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(self.policy.connect_timeout)
            .timeout(self.policy.stall_timeout)
            .build()
            .map_err(|e| InstallError::Network(describe(&e)))?;
        self.client = Some(built.clone());
        Ok(built)
    }

    /// Fetch `spec` to `dest` from its origins, retrying as the policy says.
    pub(super) fn fetch(
        &mut self,
        spec: &FileSpec,
        dest: &Path,
        progress: &mut dyn FnMut(&Progress),
    ) -> Result<(), InstallError> {
        if spec.origins.is_empty() || spec.bytes == 0 {
            return Err(InstallError::Network(format!(
                "{} has no origin to download it from",
                spec.name
            )));
        }
        super::stage::clear_stale_parts(dest);
        let mut staging = Staging::open(dest)?;
        let dir = dest.parent().unwrap_or_else(|| Path::new("."));
        fits(
            spec.bytes.saturating_sub(staging.len()?),
            free_space_bytes(dir),
        )?;
        let client = self.client()?;

        let mut last: Vec<Option<InstallError>> = spec.origins.iter().map(|_| None).collect();
        let mut refused = vec![false; spec.origins.len()];
        let rounds = self.policy.retry_waits.len() + 1;
        for round in 0..rounds {
            for (i, url) in spec.origins.iter().enumerate() {
                if refused[i] {
                    continue;
                }
                // Every log line and message names the origin by this, never
                // by `url`: an operator's mirror can carry a password or a
                // token (MDL-22), and the journal and the support bundle are
                // read by people the operator did not give it to.
                let shown = shown(url);
                tracing::info!(url = %shown, round = round + 1, "downloading");
                let stall = self.policy.stall_timeout;
                match attempt_origin(&client, url, spec, stall, &mut staging, progress) {
                    Ok(()) => return staging.finish(dest),
                    Err(Failure::Fatal(e)) => return Err(e),
                    Err(Failure::Refused(e)) => {
                        tracing::warn!(url = %shown, error = %e, "this origin will not serve the pinned file; not asking it again");
                        refused[i] = true;
                        last[i] = Some(e);
                    }
                    Err(Failure::Transient(e) | Failure::ResumedBadly(e)) => {
                        tracing::warn!(url = %shown, error = %e, "this attempt failed; what arrived is kept for the next");
                        last[i] = Some(e);
                    }
                }
            }
            if refused.iter().all(|r| *r) {
                break;
            }
            if let Some(wait) = self.policy.retry_waits.get(round) {
                tracing::warn!(
                    file = %spec.name,
                    attempt = round + 1,
                    of = rounds,
                    wait_secs = wait.as_secs(),
                    "no origin served the file; trying again after a wait"
                );
                std::thread::sleep(*wait);
            }
        }
        staging.discard_if_empty();
        Err(collect(spec, last))
    }
}

/// The error an install that ran out of origins reports: each origin's last
/// reason, in order; one origin's own error when there was only one.
fn collect(spec: &FileSpec, last: Vec<Option<InstallError>>) -> InstallError {
    let mut tried: Vec<(String, InstallError)> = spec
        .origins
        .iter()
        .zip(last)
        .filter_map(|(url, e)| e.map(|e| (shown(url), e)))
        .collect();
    if tried.len() == 1 {
        return tried.remove(0).1;
    }
    InstallError::AllOrigins(tried)
}

/// A network error as text: what went wrong, and why, as far down as the
/// cause goes ("error sending request: … connection refused"), without the
/// URL `reqwest` appends (the URL is logged beside it).
fn describe(e: &(dyn std::error::Error + 'static)) -> String {
    let mut text = e.to_string();
    if let Some(r) = e.downcast_ref::<reqwest::Error>() {
        text = r.to_string();
        if let Some(url) = r.url() {
            text = text.replace(&format!(" for url ({url})"), "");
        }
    }
    let mut cause = e.source();
    while let Some(c) = cause {
        let more = c.to_string();
        if !text.contains(&more) {
            text.push_str(": ");
            text.push_str(&more);
        }
        cause = c.source();
    }
    // A library's own message can name the URL another way (a redirect's
    // target, a cause that prints it): whatever is left is redacted here.
    birdnet_core::config::redact::redact_credentials_in_text(&text)
}

/// `url` as it may be printed: the password of any `user:pass@` and the value
/// of any query parameter that carries a secret replaced (MDL-22). The host,
/// the path and the account name stay, so a failure is still diagnosable.
pub(super) fn shown(url: &str) -> String {
    birdnet_core::config::redact::redact_url_credentials(url)
}

/// One origin, and once more from the start if a resumed file failed its hash.
fn attempt_origin(
    client: &reqwest::blocking::Client,
    url: &str,
    spec: &FileSpec,
    stall: std::time::Duration,
    staging: &mut Staging,
    progress: &mut dyn FnMut(&Progress),
) -> Result<(), Failure> {
    match attempt(client, url, spec, stall, staging, progress) {
        Err(Failure::ResumedBadly(e)) => {
            tracing::warn!(url = %shown(url), error = %e, "the resumed file did not verify; discarded, fetching it again from the start");
            attempt(client, url, spec, stall, staging, progress).map_err(|f| match f {
                // From the start, the wrong bytes are the origin's.
                Failure::ResumedBadly(e) => Failure::Refused(e),
                other => other,
            })
        }
        other => other,
    }
}

/// One request to one origin, resuming whatever the staging file holds.
fn attempt(
    client: &reqwest::blocking::Client,
    url: &str,
    spec: &FileSpec,
    stall: std::time::Duration,
    staging: &mut Staging,
    progress: &mut dyn FnMut(&Progress),
) -> Result<(), Failure> {
    let url_shown = shown(url);
    let mut have = staging.len().map_err(Failure::Fatal)?;
    if have > spec.bytes {
        staging.restart().map_err(Failure::Fatal)?;
        have = 0;
    }
    if have == spec.bytes {
        // A complete file that was never renamed: no request needed.
        return staging.verify(spec, have > 0);
    }
    let mut request = client.get(url);
    if have > 0 {
        tracing::info!(
            url = %url_shown,
            from_byte = have,
            of = spec.bytes,
            "resuming the download"
        );
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut resp = request
        .send()
        .map_err(|e| Failure::Transient(InstallError::Network(describe(&e))))?;
    let status = resp.status();
    let resumed = match status.as_u16() {
        206 if have > 0 => {
            if content_range_start(&resp) != Some(have) {
                staging.restart().map_err(Failure::Fatal)?;
                return Err(Failure::Transient(InstallError::Network(format!(
                    "{url_shown} answered a range other than the one asked for; starting the file again"
                ))));
            }
            true
        }
        200 => {
            if have > 0 {
                tracing::info!(url = %url_shown, "this origin does not resume; starting the file again");
                staging.restart().map_err(Failure::Fatal)?;
            }
            false
        }
        416 => {
            staging.restart().map_err(Failure::Fatal)?;
            return Err(Failure::Transient(InstallError::Network(format!(
                "{url_shown} could not serve the rest of the file; starting it again"
            ))));
        }
        404 | 410 => {
            return Err(Failure::Refused(InstallError::Network(format!(
                "{url_shown} answered {status}"
            ))));
        }
        _ => {
            return Err(Failure::Transient(InstallError::Network(format!(
                "{url_shown} answered {status}"
            ))));
        }
    };
    let start = if resumed { have } else { 0 };
    if resp
        .content_length()
        .is_some_and(|len| start.saturating_add(len) > spec.bytes)
    {
        staging.restart().map_err(Failure::Fatal)?;
        return Err(Failure::Refused(InstallError::TooLarge {
            limit: spec.bytes,
        }));
    }
    staging.append(&mut resp, start, spec, stall, progress)?;
    staging.verify(spec, resumed)
}

/// The first byte a `206`'s `Content-Range: bytes A-B/T` covers.
fn content_range_start(resp: &reqwest::blocking::Response) -> Option<u64> {
    let value = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    let range = value.trim().strip_prefix("bytes ")?;
    range.split('-').next()?.trim().parse().ok()
}

/// `<name>.download`, locked for as long as this install holds it.
#[derive(Debug)]
struct Staging {
    path: PathBuf,
    file: File,
}

impl Staging {
    fn open(dest: &Path) -> Result<Self, InstallError> {
        let mut name = dest.file_name().unwrap_or_default().to_os_string();
        name.push(".download");
        let path = dest.with_file_name(name);
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { path, file }),
            Err(fs::TryLockError::WouldBlock) => Err(InstallError::Busy {
                file: dest
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            }),
            Err(fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }

    fn len(&self) -> Result<u64, InstallError> {
        Ok(self.file.metadata()?.len())
    }

    fn restart(&mut self) -> Result<(), InstallError> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        Ok(())
    }

    /// Write the body after the first `start` bytes, reporting progress.
    /// A read that fails keeps what arrived (flushed to the disk) for the
    /// next attempt; more than the pin is refused and discarded. A read that
    /// failed after `stall` of silence is reported as the stall it was.
    fn append(
        &mut self,
        body: &mut dyn std::io::Read,
        start: u64,
        spec: &FileSpec,
        stall: std::time::Duration,
        progress: &mut dyn FnMut(&Progress),
    ) -> Result<(), Failure> {
        let fatal = |e: std::io::Error| Failure::Fatal(InstallError::Io(e));
        self.file.set_len(start).map_err(fatal)?;
        self.file.seek(SeekFrom::Start(start)).map_err(fatal)?;
        let mut done = start;
        let mut buf = vec![0u8; CHUNK];
        let report = |done: u64, progress: &mut dyn FnMut(&Progress)| {
            progress(&Progress {
                file: spec.name.clone(),
                done,
                total: spec.bytes,
            });
        };
        report(done, progress);
        loop {
            let asked = std::time::Instant::now();
            let n = match body.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    self.file.sync_data().map_err(fatal)?;
                    let why = if asked.elapsed() >= stall {
                        format!("nothing arrived for {} s", stall.as_secs())
                    } else {
                        describe(&e)
                    };
                    return Err(Failure::Transient(InstallError::Network(format!(
                        "the transfer stopped after {done} of {} bytes: {why}",
                        spec.bytes
                    ))));
                }
            };
            if done + n as u64 > spec.bytes {
                self.restart().map_err(Failure::Fatal)?;
                return Err(Failure::Refused(InstallError::TooLarge {
                    limit: spec.bytes,
                }));
            }
            self.file.write_all(&buf[..n]).map_err(fatal)?;
            done += n as u64;
            report(done, progress);
        }
        self.file.sync_data().map_err(fatal)?;
        if done < spec.bytes {
            return Err(Failure::Transient(InstallError::Network(format!(
                "the transfer ended after {done} of {} bytes",
                spec.bytes
            ))));
        }
        Ok(())
    }

    /// Hash the whole staging file against the pin. A mismatch discards it.
    fn verify(&mut self, spec: &FileSpec, resumed: bool) -> Result<(), Failure> {
        let fatal = |e: std::io::Error| Failure::Fatal(InstallError::Io(e));
        self.file.seek(SeekFrom::Start(0)).map_err(fatal)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = self.file.read(&mut buf).map_err(fatal)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let actual = hex(&hasher.finalize());
        if actual.eq_ignore_ascii_case(spec.sha256) {
            return Ok(());
        }
        self.restart().map_err(Failure::Fatal)?;
        let e = InstallError::Integrity {
            expected: spec.sha256,
            actual,
        };
        Err(if resumed {
            Failure::ResumedBadly(e)
        } else {
            Failure::Refused(e)
        })
    }

    /// Remove the staging file when nothing in it is worth resuming.
    fn discard_if_empty(&self) {
        if self.file.metadata().is_ok_and(|m| m.len() == 0) {
            let _ = fs::remove_file(&self.path);
        }
    }

    /// Make the verified staging file the installed one.
    fn finish(self, dest: &Path) -> Result<(), InstallError> {
        self.file.sync_all()?;
        fs::rename(&self.path, dest)?;
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn staging_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".download");
    dest.with_file_name(name)
}
