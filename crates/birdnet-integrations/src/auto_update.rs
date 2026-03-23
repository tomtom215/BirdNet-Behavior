//! Binary auto-update module.
//!
//! Checks GitHub Releases for newer versions and performs atomic binary
//! replacement using a temp-file + rename pattern. All operations are
//! synchronous — callers should wrap in `tokio::task::spawn_blocking`.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;

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

/// Download the latest release binary and atomically replace the current binary.
///
/// Steps:
/// 1. Download asset to a temp file in the same directory as `current_binary`.
/// 2. Set executable permissions on the temp file.
/// 3. Rename the current binary to `{name}.bak`.
/// 4. Rename the temp file to the original binary path.
///
/// # Errors
///
/// Returns `UpdateError::Network` on download failures, `UpdateError::Io` on
/// filesystem errors.
pub fn apply_update(asset_url: &str, current_binary: &Path) -> Result<(), UpdateError> {
    let parent = current_binary.parent().unwrap_or_else(|| Path::new("."));

    let file_name = current_binary
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("birdnet-behavior");

    let tmp_path = parent.join(format!(".{file_name}.update.tmp"));
    let bak_path = parent.join(format!("{file_name}.bak"));

    // 1. Download
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
        let mut tmp_file = fs::File::create(&tmp_path)?;
        tmp_file.write_all(&bytes)?;
        tmp_file.sync_all()?;
    }

    // 2. Set executable permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&tmp_path, perms)?;
    }

    // 3. Backup current binary
    if current_binary.exists() {
        tracing::info!("backing up current binary to {}", bak_path.display());
        fs::rename(current_binary, &bak_path)?;
    }

    // 4. Move new binary into place
    tracing::info!("installing new binary to {}", current_binary.display());
    fs::rename(&tmp_path, current_binary)?;

    tracing::info!("update applied successfully");
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try to find a platform-appropriate binary asset URL from the release.
fn find_asset_url(release: &serde_json::Value) -> Option<String> {
    let assets = release["assets"].as_array()?;

    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        ""
    };

    // Prefer an asset whose name contains our architecture and "linux".
    for asset in assets {
        let name = asset["name"].as_str().unwrap_or("");
        let lower = name.to_lowercase();
        if lower.contains("linux") && (arch.is_empty() || lower.contains(arch)) {
            return asset["browser_download_url"].as_str().map(String::from);
        }
    }

    // Fall back to the first asset.
    assets
        .first()
        .and_then(|a| a["browser_download_url"].as_str().map(String::from))
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
}
