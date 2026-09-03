//! Redacting configuration before it leaves the station.
//!
//! # Why this lives in `birdnet-core`
//!
//! It was written for `--support-bundle`, whose module doc states the policy
//! these functions implement: deny-by-default in both directions, **by key**
//! for names that look like secrets and **by shape** for values that carry a
//! credential without saying so in the key. Replace the value rather than drop
//! the line, because "this station has an SMTP password set" is diagnostic and
//! a missing line reads identically to a setting that was never configured.
//!
//! `GET /api/v2/settings` (`O-1`) has to answer the same question about the
//! same keys, from a different crate. Two copies of a rule about which values
//! are secret is precisely the arrangement that once shipped an open `/admin`
//! while the station's own diagnostic reported it protected — see
//! `helpers::auth::resolve_admin_password`. So the rule moved here, where both
//! callers can reach it, and `crate::support` calls through rather than
//! keeping a second copy.

/// What the redactor put in place of a secret.
pub const REDACTED: &str = "***REDACTED***";

/// Whether a configuration key names a secret.
///
/// Substring matching on the *key*, deliberately over-broad: a false positive
/// costs one redacted line in a support bundle, a false negative costs an
/// operator their admin password in a public issue. `KEY` catches
/// `FLICKR_API_KEY` and would catch a hypothetical `API_KEY`; it also catches
/// innocuous names containing "key", which is the trade being made.
///
/// `PASS` rather than `PASSWORD` because the real key is `EMAIL_SMTP_PASS`, and
/// a needle that only matched the long form would have missed the one SMTP
/// secret a station actually stores. It would also match an audio setting named
/// `HIGH_PASS`, which is not a configuration key today — and if it ever became
/// one, a masked boolean costs a reader nothing while a leaked mail password
/// costs an operator their account.
#[must_use]
pub fn is_secret_key(key: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "PASS",
        "PWD",
        "SECRET",
        "TOKEN",
        "APIKEY",
        "API_KEY",
        "KEY",
        "CREDENTIAL",
        "AUTH",
        "SALT",
        "HASH",
    ];
    let upper = key.to_ascii_uppercase();
    NEEDLES.iter().any(|n| upper.contains(n))
}

/// Strip `user:pass@` from a URL-shaped value.
///
/// Returns the value unchanged when it carries no credentials, so a plain
/// `rtsp://camera.local/stream` stays readable — the hostname and path are
/// exactly what makes an RTSP problem diagnosable.
///
/// Only the authority between `://` and the first `/` (or end) is examined, so
/// an `@` in a path or query is left alone.
#[must_use]
pub fn redact_url_credentials(value: &str) -> String {
    let Some(scheme_end) = value.find("://") else {
        return value.to_owned();
    };
    let authority_start = scheme_end + 3;
    let rest = &value[authority_start..];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];

    let Some(at) = authority.rfind('@') else {
        return value.to_owned();
    };
    // Keep the username: "which account is this camera using" is diagnostic,
    // and it is not the secret. Only the password half is replaced.
    let userinfo = &authority[..at];
    let host = &authority[at + 1..];
    let user = userinfo.split_once(':').map_or(userinfo, |(u, _)| u);

    format!(
        "{}{user}:{REDACTED}@{host}{}",
        &value[..authority_start],
        &rest[authority_end..]
    )
}

/// Mask the local part of a value that is a bare email address.
///
/// `me@example.com` becomes `***@example.com`. The domain stays because that
/// is the diagnostic half — "is this pointing at the right mail host", "is the
/// setting populated at all" — and the local part is the half that identifies a
/// person in a bundle attached to a public issue.
///
/// Deliberately narrow: exactly one `@`, no whitespace, a dot in the domain,
/// and something on both sides. A value that is not unambiguously one address
/// is returned untouched, because a broad "looks like an email" rule applied to
/// every configuration value would eventually mangle a setting nobody expected
/// it to touch.
#[must_use]
pub fn redact_email_local_part(value: &str) -> String {
    let Some((local, domain)) = value.split_once('@') else {
        return value.to_owned();
    };
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || !domain.contains('.')
        || value.chars().any(char::is_whitespace)
    {
        return value.to_owned();
    }
    format!("***@{domain}")
}

#[cfg(test)]
mod tests {
    use super::{REDACTED, is_secret_key, redact_email_local_part, redact_url_credentials};

    #[test]
    fn secret_key_names_are_recognised() {
        for k in [
            "CADDY_PWD",
            "BIRDWEATHER_TOKEN",
            "BNB_SESSION_SECRET",
            "EMAIL_SMTP_PASS",
            "FLICKR_API_KEY",
            "BNB_SHARE_SECRET",
            // The API write token (`O-1`). Caught by the `TOKEN` needle, and
            // named here so that is asserted rather than inferred.
            "BNB_API_TOKEN",
            "caddy_pwd",
        ] {
            assert!(is_secret_key(k), "{k} must be treated as a secret");
        }
    }

    /// The counterpart. Without it "redact everything" would pass the test
    /// above and produce a bundle with no diagnostic value at all.
    #[test]
    fn ordinary_keys_are_not_redacted() {
        for k in [
            "LATITUDE",
            "RECORDING_LENGTH",
            "MODEL_PATH",
            "BIRDNET_LISTEN",
            "SF_THRESH",
        ] {
            assert!(!is_secret_key(k), "{k} must survive into the bundle");
        }
    }

    #[test]
    fn url_credentials_are_stripped_but_the_host_survives() {
        assert_eq!(
            redact_url_credentials("rtsp://admin:hunter2@camera.local:554/stream"),
            format!("rtsp://admin:{REDACTED}@camera.local:554/stream")
        );
    }

    /// A URL without credentials must come through untouched — the hostname
    /// and path are what make an RTSP fault diagnosable.
    #[test]
    fn urls_without_credentials_are_untouched() {
        for url in [
            "rtsp://camera.local:554/stream",
            "http://localhost:8000",
            "not a url at all",
        ] {
            assert_eq!(redact_url_credentials(url), url);
        }
    }

    /// An `@` after the authority is part of the path, not credentials.
    /// Treating it as a userinfo separator would mangle the URL and hide the
    /// host — the opposite of the intent.
    #[test]
    fn an_at_sign_in_the_path_is_not_credentials() {
        let url = "http://example.com/path/@handle";
        assert_eq!(redact_url_credentials(url), url);
    }

    /// The email masker is narrow on purpose: a broad rule over every value
    /// would eventually mangle a setting nobody expected it to touch.
    #[test]
    fn only_an_unambiguous_email_address_has_its_local_part_masked() {
        assert_eq!(redact_email_local_part("me@example.com"), "***@example.com");
        assert_eq!(
            redact_email_local_part("first.last+tag@mail.example.co.uk"),
            "***@mail.example.co.uk"
        );
        for untouched in [
            "plughw:1,0",             // no @ at all
            "user@host",              // no dot: a LAN hostname, not an address
            "@example.com",           // no local part
            "me@",                    // no domain
            "a@b@example.com",        // not one address
            "see me@example.com now", // prose, not a value
        ] {
            assert_eq!(
                redact_email_local_part(untouched),
                untouched,
                "{untouched:?} must be left alone"
            );
        }
    }
}
