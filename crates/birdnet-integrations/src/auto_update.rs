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

/// GitHub API endpoint for the latest release.
const RELEASES_URL: &str =
    "https://api.github.com/repos/tomtom215/BirdNet-Behavior/releases/latest";

/// User-Agent header required by GitHub API.
const USER_AGENT: &str = "BirdNet-Behavior-Updater";

/// Request timeout.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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
    /// No update is available (current version is up-to-date).
    NotAvailable,
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(msg) => write!(f, "update network error: {msg}"),
            Self::Parse(msg) => write!(f, "update parse error: {msg}"),
            Self::Io(e) => write!(f, "update I/O error: {e}"),
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
    /// Release notes / body from the GitHub release.
    pub release_notes: String,
    /// Whether the latest version is newer than the current version.
    pub update_available: bool,
}

// ---------------------------------------------------------------------------
// Version comparison
// ---------------------------------------------------------------------------

/// Parse a version tag like `"v0.1.0"` or `"0.1.0"` into `(major, minor, patch)`.
fn parse_version(tag: &str) -> Result<(u64, u64, u64), UpdateError> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<&str> = stripped.split('.').collect();
    if parts.len() != 3 {
        return Err(UpdateError::Parse(format!(
            "expected 3 version components, got {}: {tag}",
            parts.len()
        )));
    }
    let major = parts[0]
        .parse::<u64>()
        .map_err(|e| UpdateError::Parse(format!("bad major version: {e}")))?;
    let minor = parts[1]
        .parse::<u64>()
        .map_err(|e| UpdateError::Parse(format!("bad minor version: {e}")))?;
    let patch = parts[2]
        .parse::<u64>()
        .map_err(|e| UpdateError::Parse(format!("bad patch version: {e}")))?;
    Ok((major, minor, patch))
}

/// Returns `true` if `latest` is strictly newer than `current`.
fn is_newer(current: &str, latest: &str) -> Result<bool, UpdateError> {
    let c = parse_version(current)?;
    let l = parse_version(latest)?;
    Ok(l > c)
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

    let body: serde_json::Value = resp
        .json()
        .map_err(|e| UpdateError::Parse(format!("invalid JSON response: {e}")))?;

    let tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| UpdateError::Parse("missing tag_name in response".into()))?;

    let release_notes = body["body"].as_str().unwrap_or("").to_string();

    // Find a suitable asset download URL. Fall back to the tarball URL.
    let download_url = find_asset_url(&body)
        .unwrap_or_else(|| body["tarball_url"].as_str().unwrap_or("").to_string());

    let update_available = is_newer(current_version, tag).unwrap_or(false);

    Ok(UpdateInfo {
        current_version: current_version.to_string(),
        latest_version: tag.to_string(),
        download_url,
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
/// 1. Download the asset to a temp file next to `current_binary`.
/// 2. If the asset is a tar.gz, extract it and locate the binary inside.
/// 3. Set executable permissions on the new binary.
/// 4. Rename the current binary to `{name}.bak`.
/// 5. Rename the new binary into place.
///
/// # Errors
///
/// Returns `UpdateError::Network` on download failures, `UpdateError::Io` on
/// filesystem errors, and `UpdateError::Parse` if the archive layout is
/// unexpected or the embedded binary cannot be located.
pub fn apply_update(asset_url: &str, current_binary: &Path) -> Result<(), UpdateError> {
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

    let bytes = resp
        .bytes()
        .map_err(|e| UpdateError::Network(format!("failed to read response body: {e}")))?;

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

    // 3. Set executable permissions on the staged binary.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&staged_path, perms)?;
    }

    // 4. Backup current binary (best-effort).
    if current_binary.exists() {
        tracing::info!("backing up current binary to {}", bak_path.display());
        fs::rename(current_binary, &bak_path)?;
    }

    // 5. Move new binary into place.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_with_prefix() {
        assert_eq!(parse_version("v1.2.3").unwrap(), (1, 2, 3));
    }

    #[test]
    fn parse_version_without_prefix() {
        assert_eq!(parse_version("0.10.5").unwrap(), (0, 10, 5));
    }

    #[test]
    fn parse_version_invalid() {
        assert!(parse_version("1.2").is_err());
        assert!(parse_version("abc").is_err());
    }

    #[test]
    fn is_newer_true() {
        assert!(is_newer("0.1.0", "v0.2.0").unwrap());
        assert!(is_newer("v1.0.0", "v1.0.1").unwrap());
        assert!(is_newer("0.9.9", "1.0.0").unwrap());
    }

    #[test]
    fn is_newer_false() {
        assert!(!is_newer("0.2.0", "0.1.0").unwrap());
        assert!(!is_newer("1.0.0", "1.0.0").unwrap());
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
}
