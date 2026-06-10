//! Binary auto-update module.
//!
//! Checks GitHub Releases for newer versions and performs atomic binary
//! replacement using a temp-file + rename pattern. All operations are
//! synchronous — callers should wrap in `tokio::task::spawn_blocking`.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

mod version;

use version::{is_newer, validate_release_url};

/// GitHub API endpoint for the latest release.
const RELEASES_URL: &str =
    "https://api.github.com/repos/tomtom215/BirdNet-Behavior/releases/latest";

/// User-Agent header required by GitHub API.
const USER_AGENT: &str = "BirdNet-Behavior-Updater";

/// Request timeout.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Upper bound on the release-metadata JSON read from the GitHub API.
/// Real responses are a few hundred KB at most; the cap only exists so a
/// compromised or misbehaving endpoint cannot stream an arbitrarily large
/// body into memory on a small-RAM Pi.
const MAX_METADATA_BYTES: u64 = 8 * 1024 * 1024;

/// Upper bound on a `SHA256SUMS` fetch. The real file is a few hundred
/// bytes (one line per release asset).
const MAX_SUMS_BYTES: u64 = 64 * 1024;

/// Upper bound on the downloaded release asset. The binary is ~75 MB and
/// the tarball under 100 MB; the asset is held fully in memory so its hash
/// can be verified *before* anything touches disk, which is exactly why a
/// runaway body must be cut off.
const MAX_ASSET_BYTES: u64 = 512 * 1024 * 1024;

/// Read an HTTP response body into memory, refusing to buffer more than
/// `cap` bytes. `declared_len` (the `Content-Length`, when present) fails
/// honest oversized transfers before a single byte is read; the `take`
/// guard stops chunked/lying bodies at the cap.
fn read_body_capped(
    body: impl std::io::Read,
    declared_len: Option<u64>,
    cap: u64,
) -> Result<Vec<u8>, String> {
    use std::io::Read;

    if let Some(len) = declared_len
        && len > cap
    {
        return Err(format!(
            "response body of {len} bytes exceeds the {cap}-byte limit"
        ));
    }

    let mut buf = Vec::new();
    // Read one byte past the cap so "exactly cap" and "over cap" are
    // distinguishable without trusting Content-Length.
    body.take(cap.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| format!("failed to read response body: {e}"))?;

    if buf.len() as u64 > cap {
        return Err(format!("response body exceeds the {cap}-byte limit"));
    }
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during update checking or application.
#[derive(Debug)]
pub enum UpdateError {
    /// HTTP / network failure.
    Network(String),
    /// Failed to parse version string or API response.
    Parse(String),
    /// File-system I/O error.
    Io(std::io::Error),
    /// The downloaded archive's SHA-256 did not match the release's
    /// published `SHA256SUMS` — the download is corrupt or tampered, so the
    /// swap is refused and the running binary is left untouched.
    Integrity(String),
    /// The freshly downloaded binary failed its pre-swap smoke test
    /// (`<binary> --version`), so it is discarded rather than installed over a
    /// working binary (wrong architecture, truncated download, missing runtime).
    SmokeTest(String),
    /// No update is available (current version is up-to-date).
    NotAvailable,
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(msg) => write!(f, "update network error: {msg}"),
            Self::Parse(msg) => write!(f, "update parse error: {msg}"),
            Self::Io(e) => write!(f, "update I/O error: {e}"),
            Self::Integrity(msg) => write!(f, "update integrity error: {msg}"),
            Self::SmokeTest(msg) => write!(f, "update smoke-test failed: {msg}"),
            Self::NotAvailable => write!(f, "no update available"),
        }
    }
}

impl std::error::Error for UpdateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for UpdateError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Information about an available (or not) update.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UpdateInfo {
    /// Currently running version string.
    pub current_version: String,
    /// Latest version string from GitHub.
    pub latest_version: String,
    /// Direct download URL for the release asset.
    pub download_url: String,
    /// Expected lowercase hex SHA-256 of the download asset, parsed from the
    /// release's `SHA256SUMS`. `None` when the release ships no checksum file
    /// or no line matches the chosen asset (older releases); `apply_update`
    /// then falls back to the smoke test alone.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Release notes / body from the GitHub release.
    pub release_notes: String,
    /// Whether the latest version is newer than the current version.
    pub update_available: bool,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Check GitHub Releases for a newer version.
///
/// Performs a synchronous HTTP GET to the GitHub Releases API and compares
/// the latest tag against `current_version`.
///
/// # Errors
///
/// Returns `UpdateError::Network` on HTTP or connection failures, `UpdateError::Parse`
/// if the API response cannot be decoded, or `UpdateError::NotAvailable` if the
/// current version is already up-to-date.
pub fn check_for_update(current_version: &str) -> Result<UpdateInfo, UpdateError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| UpdateError::Network(format!("failed to build HTTP client: {e}")))?;

    let resp = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| UpdateError::Network(format!("request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(UpdateError::Network(format!(
            "GitHub API returned status {}",
            resp.status()
        )));
    }

    let declared_len = resp.content_length();
    let raw =
        read_body_capped(resp, declared_len, MAX_METADATA_BYTES).map_err(UpdateError::Network)?;
    let body: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| UpdateError::Parse(format!("invalid JSON response: {e}")))?;

    let tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| UpdateError::Parse("missing tag_name in response".into()))?;

    let release_notes = body["body"].as_str().unwrap_or("").to_string();

    // Find a suitable asset download URL. Fall back to the tarball URL.
    let download_url = find_asset_url(&body)
        .unwrap_or_else(|| body["tarball_url"].as_str().unwrap_or("").to_string());

    // Best-effort: look up the asset's published SHA-256 from the release's
    // `SHA256SUMS`. A miss (older release, source tarball fallback, network
    // hiccup) leaves `sha256` as `None` — `apply_update` still smoke-tests the
    // staged binary before swapping, so this never blocks a legitimate update.
    let sha256 = asset_filename_from_url(&download_url).and_then(|fname| {
        find_sha256sums_url(&body)
            .and_then(|sums_url| fetch_expected_sha256(&client, &sums_url, &fname))
    });
    if sha256.is_none() {
        tracing::debug!("no SHA256SUMS entry found for the update asset; will rely on smoke test");
    }

    let update_available = match is_newer(current_version, tag) {
        Ok(newer) => newer,
        Err(e) => {
            // Surface a malformed version rather than silently reporting
            // "up to date" — a tag typo would otherwise hide every future
            // update (including security fixes) with no diagnostic.
            tracing::warn!(
                error = %e,
                current = %current_version,
                latest = %tag,
                "could not compare versions for update check; treating as up-to-date"
            );
            false
        }
    };

    Ok(UpdateInfo {
        current_version: current_version.to_string(),
        latest_version: tag.to_string(),
        download_url,
        sha256,
        release_notes,
        update_available,
    })
}

/// Download the latest release and atomically replace the current binary.
///
/// Release archives are gzipped tarballs of the form
/// `birdnet-behavior-<version>-<target>.tar.gz` containing a single top-level
/// directory with the binary inside. Older releases that exposed a raw
/// ELF binary are still supported transparently.
///
/// Steps:
/// 1. Download the asset bytes.
/// 2. Verify the download against `expected_sha256` (when supplied) **before**
///    anything is written — a mismatch aborts with the running binary untouched.
/// 3. If the asset is a tar.gz, extract it and locate the binary inside.
/// 4. Set executable permissions on the new binary.
/// 5. Smoke-test the staged binary (`<binary> --version`) — if it cannot run,
///    abort and leave the running binary in place (no rollback needed).
/// 6. Rename the current binary to `{name}.bak`.
/// 7. Rename the new binary into place.
///
/// `expected_sha256` is the lowercase hex digest from the release's
/// `SHA256SUMS` (see [`UpdateInfo::sha256`]). Pass `None` only when no checksum
/// is available; the smoke test still guards against a broken binary. This is
/// an integrity check, not a signature check — release archives additionally
/// carry SLSA build provenance for out-of-band authenticity verification.
///
/// # Errors
///
/// Returns `UpdateError::Network` on download failures, `UpdateError::Integrity`
/// on a checksum mismatch, `UpdateError::SmokeTest` if the new binary will not
/// run, `UpdateError::Io` on filesystem errors, and `UpdateError::Parse` if the
/// archive layout is unexpected or the embedded binary cannot be located.
pub fn apply_update(
    asset_url: &str,
    current_binary: &Path,
    expected_sha256: Option<&str>,
) -> Result<(), UpdateError> {
    // Defense-in-depth: the downloaded bytes are written next to the running
    // binary and (after checksum + smoke test) installed as the executable, so
    // refuse any asset URL that isn't HTTPS on a GitHub release host before
    // fetching a single byte. The release flow only ever passes
    // github.com / *.githubusercontent.com URLs; this rejects a tampered API
    // response or a future caller that supplies an arbitrary URL.
    validate_release_url(asset_url)?;

    let parent = current_binary.parent().unwrap_or_else(|| Path::new("."));

    let file_name = current_binary
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("birdnet-behavior");

    let download_path = parent.join(format!(".{file_name}.update.download"));
    let staged_path = parent.join(format!(".{file_name}.update.staged"));
    let bak_path = parent.join(format!("{file_name}.bak"));

    // 1. Download the asset bytes.
    tracing::info!("downloading update from {asset_url}");
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| UpdateError::Network(format!("failed to build HTTP client: {e}")))?;

    let resp = client
        .get(asset_url)
        .send()
        .map_err(|e| UpdateError::Network(format!("download failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(UpdateError::Network(format!(
            "download returned status {}",
            resp.status()
        )));
    }

    let declared_len = resp.content_length();
    let bytes =
        read_body_capped(resp, declared_len, MAX_ASSET_BYTES).map_err(UpdateError::Network)?;

    // 2. Verify integrity *before* writing anything to disk, so a corrupt or
    //    tampered download never lands next to the running binary.
    if let Some(expected) = expected_sha256 {
        verify_integrity(&bytes, expected)?;
        tracing::info!("update asset sha256 verified");
    } else {
        tracing::warn!(
            "no sha256 checksum available for the update asset; \
             integrity not verified (relying on the staged-binary smoke test)"
        );
    }

    {
        let mut f = fs::File::create(&download_path)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }

    // 2. If the asset is a tar.gz, extract it and pull the binary out.
    if is_tarball_url(asset_url) {
        tracing::info!("extracting release archive");
        let extract_dir = parent.join(format!(".{file_name}.update.extract"));
        let _ = fs::remove_dir_all(&extract_dir);
        fs::create_dir_all(&extract_dir)?;

        let status = Command::new("tar")
            .arg("-xzf")
            .arg(&download_path)
            .arg("-C")
            .arg(&extract_dir)
            .status()
            .map_err(|e| {
                UpdateError::Network(format!("failed to invoke `tar` for extraction: {e}"))
            })?;

        if !status.success() {
            let _ = fs::remove_dir_all(&extract_dir);
            let _ = fs::remove_file(&download_path);
            return Err(UpdateError::Network(format!(
                "`tar -xzf` failed with exit status {status}"
            )));
        }

        let extracted = find_extracted_binary(&extract_dir, file_name).inspect_err(|_| {
            let _ = fs::remove_dir_all(&extract_dir);
            let _ = fs::remove_file(&download_path);
        })?;

        // Move the extracted binary to the staged path, then clean up.
        fs::rename(&extracted, &staged_path)?;
        let _ = fs::remove_dir_all(&extract_dir);
        let _ = fs::remove_file(&download_path);
    } else {
        // Legacy raw-binary asset — promote the download directly.
        fs::rename(&download_path, &staged_path)?;
    }

    // 4. Set executable permissions on the staged binary.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&staged_path, perms)?;
    }

    // 5. Smoke-test the staged binary before it can replace a working one. A
    //    wrong-arch, truncated, or runtime-incompatible binary fails `--version`
    //    here and is discarded, leaving the running binary untouched.
    if let Err(e) = smoke_test_binary(&staged_path) {
        let _ = fs::remove_file(&staged_path);
        return Err(e);
    }

    // 6. Backup current binary (best-effort).
    if current_binary.exists() {
        tracing::info!("backing up current binary to {}", bak_path.display());
        fs::rename(current_binary, &bak_path)?;
    }

    // 7. Move new binary into place.
    tracing::info!("installing new binary to {}", current_binary.display());
    fs::rename(&staged_path, current_binary)?;

    tracing::info!("update applied successfully");
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try to find a platform-appropriate binary asset URL from the release.
///
/// Prefers release archives (`.tar.gz`) matching the current architecture and
/// Linux target, then falls back to any asset matching the architecture.
// The lint fires on `ends_with(".ext")` but every string tested here is
// pre-lowercased, so the suffix match is effectively case-insensitive.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
fn find_asset_url(release: &serde_json::Value) -> Option<String> {
    let assets = release["assets"].as_array()?;

    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "arm") {
        "armv7"
    } else {
        ""
    };

    // Skip metadata assets such as SHA256SUMS and install.sh that happen to
    // live alongside the binary archives in each release.
    let is_metadata = |lower: &str| -> bool {
        lower.contains("sha256sums")
            || lower.ends_with("install.sh")
            || lower.ends_with(".sig")
            || lower.ends_with(".asc")
    };

    // First pass: prefer a `.tar.gz` archive that targets both Linux and our arch.
    for asset in assets {
        let name = asset["name"].as_str().unwrap_or("");
        let lower = name.to_lowercase();
        if is_metadata(&lower) {
            continue;
        }
        if lower.ends_with(".tar.gz")
            && lower.contains("linux")
            && (arch.is_empty() || lower.contains(arch))
        {
            return asset["browser_download_url"].as_str().map(String::from);
        }
    }

    // Second pass: accept any asset matching Linux and the arch (raw binary from
    // older releases that did not ship tarballs).
    for asset in assets {
        let name = asset["name"].as_str().unwrap_or("");
        let lower = name.to_lowercase();
        if is_metadata(&lower) {
            continue;
        }
        if lower.contains("linux") && (arch.is_empty() || lower.contains(arch)) {
            return asset["browser_download_url"].as_str().map(String::from);
        }
    }

    None
}

/// Returns `true` if the asset URL refers to a gzipped tar archive.
// The input is lowercased before the suffix check, so the comparison is
// effectively case-insensitive — `.tar.gz` is a double extension that
// `std::path::Path::extension` cannot describe in a single call.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
fn is_tarball_url(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let lower = path.to_lowercase();
    lower.ends_with(".tar.gz") || lower.ends_with(".tgz")
}

/// Locate the binary inside an extracted release archive.
///
/// The release archive layout is
/// `birdnet-behavior-<version>-<target>/birdnet-behavior`, so the binary sits
/// exactly one level below the extraction root. This walks the immediate
/// children, then falls back to a limited two-level search for robustness.
fn find_extracted_binary(dir: &Path, binary_name: &str) -> Result<PathBuf, UpdateError> {
    // First: check the top level directly (in case the archive is flat).
    let direct = dir.join(binary_name);
    if direct.is_file() {
        return Ok(direct);
    }

    // Second: one level down (the normal release layout).
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let candidate = path.join(binary_name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err(UpdateError::Parse(format!(
        "binary '{binary_name}' not found in extracted archive at {}",
        dir.display()
    )))
}

/// Extract the bare filename (last path segment, query/fragment stripped) from a
/// download URL so it can be matched against a `SHA256SUMS` line.
fn asset_filename_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Locate the `SHA256SUMS` asset's download URL in a release payload.
fn find_sha256sums_url(release: &serde_json::Value) -> Option<String> {
    let assets = release["assets"].as_array()?;
    for asset in assets {
        if asset["name"]
            .as_str()
            .is_some_and(|n| n.eq_ignore_ascii_case("SHA256SUMS"))
        {
            return asset["browser_download_url"].as_str().map(String::from);
        }
    }
    None
}

/// Fetch the `SHA256SUMS` file and return the expected digest for `filename`.
/// Best-effort: any network/parse failure yields `None`.
fn fetch_expected_sha256(
    client: &reqwest::blocking::Client,
    sums_url: &str,
    filename: &str,
) -> Option<String> {
    // Pin the checksum source to a GitHub host too: a checksum fetched from an
    // attacker-controlled URL could be made to match a malicious binary. (The
    // binary download is independently pinned in `apply_update`.)
    validate_release_url(sums_url).ok()?;
    let resp = client.get(sums_url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let declared_len = resp.content_length();
    let raw = read_body_capped(resp, declared_len, MAX_SUMS_BYTES).ok()?;
    let text = String::from_utf8(raw).ok()?;
    parse_sha256sums(&text, filename)
}

/// Parse a `sha256sum`-format file and return the lowercase hex digest whose
/// line names `filename`. Lines are `"<64-hex>␠␠<name>"` (a leading `*` for
/// binary mode and any directory prefix on the name are tolerated).
fn parse_sha256sums(contents: &str, filename: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(hex), Some(name)) = (fields.next(), fields.next()) else {
            continue;
        };
        let name = name.strip_prefix('*').unwrap_or(name);
        let name = name.rsplit('/').next().unwrap_or(name);
        if name == filename && hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(hex.to_ascii_lowercase());
        }
    }
    None
}

/// Compute the lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in &digest {
        // Infallible: writing to a String never errors.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Verify `bytes` hashes to `expected_hex` (case-insensitive).
///
/// # Errors
///
/// Returns `UpdateError::Integrity` when the computed digest differs.
fn verify_integrity(bytes: &[u8], expected_hex: &str) -> Result<(), UpdateError> {
    let actual = sha256_hex(bytes);
    if actual.eq_ignore_ascii_case(expected_hex.trim()) {
        Ok(())
    } else {
        Err(UpdateError::Integrity(format!(
            "sha256 mismatch: expected {}, got {actual}",
            expected_hex.trim()
        )))
    }
}

/// Run `<binary> --version` and require a clean exit, proving the freshly
/// downloaded binary can actually execute on this host before it replaces the
/// running one. `--version` is intercepted by clap and never starts the daemon.
///
/// # Errors
///
/// Returns `UpdateError::SmokeTest` if the binary cannot be executed or exits
/// non-zero.
fn smoke_test_binary(path: &Path) -> Result<(), UpdateError> {
    // Executing a binary we have just written can transiently fail to *spawn*:
    // ETXTBSY ("text file busy") when a concurrent fork in another thread still
    // holds a writable descriptor to the freshly-staged file, or EAGAIN under
    // fork pressure. Both clear within milliseconds, so retry the spawn a few
    // times rather than failing a perfectly good update (or flaking a parallel
    // test run). A genuinely unrunnable binary (missing, not executable) yields
    // a permanent error kind, is not retried, and still surfaces immediately.
    const MAX_ATTEMPTS: u32 = 5;
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        match Command::new(path).arg("--version").output() {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                return Err(UpdateError::SmokeTest(format!(
                    "staged binary `--version` exited with {}",
                    output.status
                )));
            }
            Err(e)
                if attempt < MAX_ATTEMPTS
                    && matches!(
                        e.kind(),
                        std::io::ErrorKind::ExecutableFileBusy
                            | std::io::ErrorKind::ResourceBusy
                            | std::io::ErrorKind::WouldBlock
                    ) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(20 * u64::from(attempt)));
            }
            Err(e) => {
                return Err(UpdateError::SmokeTest(format!(
                    "cannot execute staged binary: {e}"
                )));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_body_capped_accepts_body_at_cap() {
        let data = vec![7_u8; 16];
        let out = read_body_capped(std::io::Cursor::new(data.clone()), Some(16), 16).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn read_body_capped_rejects_declared_oversize_before_reading() {
        let err = read_body_capped(std::io::Cursor::new(vec![0_u8; 4]), Some(17), 16).unwrap_err();
        assert!(err.contains("exceeds"), "unexpected error: {err}");
    }

    #[test]
    fn read_body_capped_rejects_undeclared_oversize_stream() {
        // No Content-Length header: the take() guard must stop a chunked or
        // lying body at the cap instead of buffering it all.
        let err = read_body_capped(std::io::Cursor::new(vec![0_u8; 17]), None, 16).unwrap_err();
        assert!(err.contains("exceeds"), "unexpected error: {err}");
    }

    #[test]
    fn is_tarball_url_recognises_tar_gz() {
        assert!(is_tarball_url(
            "https://example.com/birdnet-behavior-0.1.0-aarch64-unknown-linux-gnu.tar.gz"
        ));
        assert!(is_tarball_url("file.TAR.GZ"));
        assert!(is_tarball_url("file.tgz"));
    }

    #[test]
    fn is_tarball_url_ignores_raw_binaries() {
        assert!(!is_tarball_url(
            "https://example.com/birdnet-behavior-aarch64-unknown-linux-gnu"
        ));
        assert!(!is_tarball_url("SHA256SUMS"));
    }

    #[test]
    fn is_tarball_url_ignores_query_string() {
        assert!(is_tarball_url(
            "https://example.com/archive.tar.gz?token=abc"
        ));
    }

    #[test]
    fn find_asset_url_prefers_tarball_matching_arch() {
        let release = serde_json::json!({
            "assets": [
                {
                    "name": "SHA256SUMS",
                    "browser_download_url": "https://example.com/SHA256SUMS"
                },
                {
                    "name": "install.sh",
                    "browser_download_url": "https://example.com/install.sh"
                },
                {
                    "name": "birdnet-behavior-0.1.0-x86_64-unknown-linux-gnu.tar.gz",
                    "browser_download_url": "https://example.com/x86_64.tar.gz"
                },
                {
                    "name": "birdnet-behavior-0.1.0-aarch64-unknown-linux-gnu.tar.gz",
                    "browser_download_url": "https://example.com/aarch64.tar.gz"
                }
            ]
        });

        let url = find_asset_url(&release).expect("should find an asset");

        // The exact match depends on the test runner architecture, but the
        // returned URL must always be one of the real tarballs — never
        // SHA256SUMS or install.sh.
        assert!(url.ends_with(".tar.gz"));
        assert!(!url.contains("SHA256SUMS"));
        assert!(!url.contains("install.sh"));
    }

    #[test]
    fn find_extracted_binary_walks_one_level() {
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp
            .path()
            .join("birdnet-behavior-0.1.0-x86_64-unknown-linux-gnu");
        std::fs::create_dir_all(&inner).unwrap();
        let bin = inner.join("birdnet-behavior");
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();

        let found = find_extracted_binary(tmp.path(), "birdnet-behavior").unwrap();
        assert_eq!(found, bin);
    }

    #[test]
    fn find_extracted_binary_finds_flat_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("birdnet-behavior");
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();

        let found = find_extracted_binary(tmp.path(), "birdnet-behavior").unwrap();
        assert_eq!(found, bin);
    }

    #[test]
    fn find_extracted_binary_errors_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let err = find_extracted_binary(tmp.path(), "birdnet-behavior").unwrap_err();
        assert!(matches!(err, UpdateError::Parse(_)));
    }

    // -- integrity verification --------------------------------------------

    #[test]
    fn sha256_hex_matches_known_vector() {
        // FIPS 180-2 test vector: sha256("abc").
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verify_integrity_accepts_match_case_insensitively() {
        let upper = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        assert!(verify_integrity(b"abc", upper).is_ok());
    }

    #[test]
    fn verify_integrity_rejects_mismatch() {
        let err = verify_integrity(b"abc", &"0".repeat(64)).unwrap_err();
        assert!(matches!(err, UpdateError::Integrity(_)));
    }

    #[test]
    fn parse_sha256sums_extracts_matching_digest() {
        let want = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let contents = format!(
            "{want}  birdnet-behavior-0.5.3-aarch64-unknown-linux-gnu.tar.gz\n\
             {}  birdnet-behavior-0.5.3-x86_64-unknown-linux-gnu.tar.gz\n",
            "1".repeat(64)
        );
        assert_eq!(
            parse_sha256sums(
                &contents,
                "birdnet-behavior-0.5.3-aarch64-unknown-linux-gnu.tar.gz"
            )
            .as_deref(),
            Some(want)
        );
    }

    #[test]
    fn parse_sha256sums_tolerates_binary_marker_and_path() {
        let want = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        // Binary-mode `*` marker and a leading directory component.
        let contents = format!("{want} *./dist/archive.tar.gz\n");
        assert_eq!(
            parse_sha256sums(&contents, "archive.tar.gz").as_deref(),
            Some(want)
        );
    }

    #[test]
    fn parse_sha256sums_returns_none_when_absent_or_malformed() {
        let contents = "deadbeef  short-hex.tar.gz\n# a comment\n";
        assert!(parse_sha256sums(contents, "short-hex.tar.gz").is_none());
        assert!(parse_sha256sums(contents, "missing.tar.gz").is_none());
    }

    #[test]
    fn asset_filename_from_url_strips_path_and_query() {
        assert_eq!(
            asset_filename_from_url(
                "https://example.com/releases/download/v0.5.3/archive.tar.gz?token=abc"
            )
            .as_deref(),
            Some("archive.tar.gz")
        );
        assert_eq!(
            asset_filename_from_url("https://example.com/").as_deref(),
            None
        );
    }

    #[test]
    fn find_sha256sums_url_locates_asset() {
        let release = serde_json::json!({
            "assets": [
                {"name": "archive.tar.gz", "browser_download_url": "https://example.com/a.tgz"},
                {"name": "SHA256SUMS", "browser_download_url": "https://example.com/SHA256SUMS"}
            ]
        });
        assert_eq!(
            find_sha256sums_url(&release).as_deref(),
            Some("https://example.com/SHA256SUMS")
        );
        let no_sums = serde_json::json!({"assets": [{"name": "x", "browser_download_url": "u"}]});
        assert!(find_sha256sums_url(&no_sums).is_none());
    }

    // -- pre-swap smoke test -----------------------------------------------

    #[cfg(unix)]
    fn write_exec_script(dir: &Path, name: &str, body: &[u8]) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn smoke_test_passes_when_binary_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = write_exec_script(
            tmp.path(),
            "ok-bin",
            b"#!/bin/sh\necho 'birdnet-behavior 9.9.9'\nexit 0\n",
        );
        assert!(smoke_test_binary(&bin).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn smoke_test_fails_on_nonzero_exit() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = write_exec_script(tmp.path(), "broken-bin", b"#!/bin/sh\nexit 3\n");
        assert!(matches!(
            smoke_test_binary(&bin).unwrap_err(),
            UpdateError::SmokeTest(_)
        ));
    }

    #[test]
    fn smoke_test_errors_when_binary_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        assert!(matches!(
            smoke_test_binary(&missing).unwrap_err(),
            UpdateError::SmokeTest(_)
        ));
    }
}
