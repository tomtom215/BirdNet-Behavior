//! Version-tag parsing, comparison, and release-URL host validation.
//!
//! Pure logic — no network or filesystem access.

use super::UpdateError;

/// Parse a version tag like `"v0.1.0"` or `"0.1.0"` into `(major, minor, patch)`.
///
/// Strips a leading `v`, then any semver build metadata (`+…`) and pre-release
/// suffix (`-…`): neither affects the numeric `(major, minor, patch)` precedence
/// we compare on. `releases/latest` only ever returns a full (non-prerelease)
/// release, so a pre-release tag is not expected here — but tolerating the
/// suffixes keeps a tag like `v1.2.3+ci` (or a future `v1.2.3-rc1`) from failing
/// to parse and being silently treated as "no update available".
fn parse_version(tag: &str) -> Result<(u64, u64, u64), UpdateError> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    // Take the numeric core before any `-prerelease` / `+build` suffix.
    let core = stripped.split(['-', '+']).next().unwrap_or(stripped);
    let parts: Vec<&str> = core.split('.').collect();
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
pub(super) fn is_newer(current: &str, latest: &str) -> Result<bool, UpdateError> {
    let c = parse_version(current)?;
    let l = parse_version(latest)?;
    Ok(l > c)
}

/// Reject a download URL that is not HTTPS on a trusted GitHub release host.
///
/// GitHub serves release assets and the `SHA256SUMS` manifest from `github.com`
/// (and its redirect targets under `*.githubusercontent.com`); pinning to those
/// hosts keeps an unexpected or tampered `asset_url` from being downloaded and
/// installed as the running binary.
pub(super) fn validate_release_url(url: &str) -> Result<(), UpdateError> {
    // Parse with the parser reqwest itself connects with (`reqwest::Url` is
    // the `url` crate's WHATWG implementation), and trust the host *it*
    // finds. Every hand-rolled authority split this had disagreed with it
    // somewhere: splitting on `:` read `https://github.com:x@evil.com/…` as
    // github.com, and splitting only on `/?#` missed that a special scheme
    // treats `\` as a path separator, so `https://evil.com\@github.com/x` —
    // host evil.com to reqwest — was read as github.com and trusted.
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| UpdateError::Network(format!("refusing unparseable update URL {url}: {e}")))?;
    if parsed.scheme() != "https" {
        return Err(UpdateError::Network(format!(
            "refusing non-HTTPS update URL: {url}"
        )));
    }
    // A domain only (`None` for an IP literal, which is never where GitHub
    // serves assets). The parser has already lowercased it.
    let host = parsed.domain().unwrap_or("");
    let trusted = host == "github.com"
        || host.ends_with(".github.com")
        || host.ends_with(".githubusercontent.com");
    if trusted {
        Ok(())
    } else {
        Err(UpdateError::Network(format!(
            "refusing update URL from untrusted host {host:?}: {url}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_release_url_accepts_github_hosts() {
        assert!(validate_release_url("https://github.com/o/r/releases/download/v1/a.tgz").is_ok());
        assert!(validate_release_url("https://api.github.com/repos/o/r/releases").is_ok());
        assert!(validate_release_url("https://objects.githubusercontent.com/abc/a.tgz").is_ok());
        assert!(validate_release_url("https://codeload.github.com/o/r/tar.gz/v1").is_ok());
    }

    #[test]
    fn validate_release_url_rejects_untrusted_and_non_https() {
        assert!(validate_release_url("https://evil.com/payload").is_err());
        assert!(validate_release_url("http://github.com/o/r/a.tgz").is_err()); // not HTTPS
        // Lookalike hosts must not pass the suffix check.
        assert!(validate_release_url("https://github.com.evil.com/a").is_err());
        assert!(validate_release_url("https://notgithub.com/a").is_err());
        assert!(validate_release_url("ftp://github.com/a").is_err());
    }

    #[test]
    fn validate_release_url_rejects_userinfo_host_spoof() {
        // Regression: the host extractor used to split on `:`, so the userinfo
        // in `https://github.com:x@evil.com/…` was read as host "github.com"
        // and trusted — while reqwest connected to the real host, evil.com.
        // The real host is after the last `@`, so these must all be rejected.
        assert!(validate_release_url("https://github.com:x@evil.com/payload.tgz").is_err());
        assert!(validate_release_url("https://github.com@evil.com/payload.tgz").is_err());
        assert!(validate_release_url("https://objects.githubusercontent.com@evil.com/a").is_err());
        assert!(validate_release_url("https://a@github.com:tok@evil.com/a").is_err());
    }

    /// The WHATWG parser reqwest uses reads `\` as a path separator in a
    /// special scheme, so in `https://evil.com\@github.com/x` the host is
    /// `evil.com` and `\@github.com/x` is path. Splitting the authority only
    /// on `/?#` read the host as `github.com` and trusted it.
    #[test]
    fn validate_release_url_rejects_backslash_host_spoof() {
        for url in [
            r"https://evil.com\@github.com/payload.tgz",
            r"https://evil.com\github.com/payload.tgz",
            r"https://evil.com\x@objects.githubusercontent.com/a",
        ] {
            let real = reqwest::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned));
            assert_eq!(real.as_deref(), Some("evil.com"), "fixture premise: {url}");
            assert!(
                validate_release_url(url).is_err(),
                "{url} connects to evil.com but was trusted"
            );
        }
    }

    /// Counterpart: the parser does not turn real GitHub URLs away, including
    /// an upper-case host and a signed query string.
    #[test]
    fn validate_release_url_accepts_github_hosts_however_spelled() {
        assert!(validate_release_url("https://GitHub.com/o/r/releases/download/v1/a.tgz").is_ok());
        assert!(
            validate_release_url(
                "https://release-assets.githubusercontent.com/github-production-release-asset/1?sig=a%2Fb"
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_release_url_accepts_port_and_benign_userinfo() {
        // An explicit port is part of the authority and must still validate.
        assert!(validate_release_url("https://github.com:443/o/r/a.tgz").is_ok());
        // Benign userinfo whose real host is github.com is where the bytes come
        // from, so it is correctly trusted (GitHub never emits these, but the
        // host extraction must resolve to github.com, not reject spuriously).
        assert!(validate_release_url("https://user@github.com/o/r/a.tgz").is_ok());
    }

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
    fn parse_version_tolerates_semver_suffixes() {
        // Build metadata and pre-release suffixes don't affect the numeric
        // precedence and must not make the parser fail (which would silently
        // suppress the update).
        assert_eq!(parse_version("v1.2.3+ci.5").unwrap(), (1, 2, 3));
        assert_eq!(parse_version("1.2.3-rc1").unwrap(), (1, 2, 3));
        assert_eq!(parse_version("v1.2.3-rc.2+build.7").unwrap(), (1, 2, 3));
    }

    #[test]
    fn is_newer_handles_suffixed_tags() {
        // A `+build`/`-rc` tag compares on its numeric core. (In practice
        // `releases/latest` only returns full releases, but the comparison
        // must not error out.)
        assert!(is_newer("1.2.2", "v1.2.3+ci").unwrap());
        assert!(!is_newer("1.2.3", "v1.2.3+ci").unwrap());
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
