//! Which `Host` names may use the no-password "open admin" bypass.
//!
//! # The attack this closes
//!
//! A station with no admin password lets every request into `/admin` (the
//! fresh-Pi contract). The only thing standing between that and *the rest of
//! the internet* is that the station is on a private network — and DNS
//! rebinding removes it. A page on `evil.example` re-points its own name at
//! the station's LAN address; the visitor's browser then sends
//! `Host: evil.example:8502` and `Origin: http://evil.example:8502` to the Pi.
//! The CSRF guard compares `Origin` with `Host`, both of which the attacker
//! chose, so it agrees. From there any website anyone in the house visits can
//! read the database, restore an archive, add webhooks and change settings.
//!
//! A cookie does not have this hole: the browser files it under the name the
//! owner used, and never sends it to `evil.example`. So only the open bypass —
//! the one door that asks for no cookie — needs a check on the name.
//!
//! # The rule
//!
//! The bypass is granted to names no outside party can point at this station:
//! an IP literal, `localhost`, a single-label name (`birdnet`, resolved by the
//! router), and the suffixes reserved for or conventionally used on home
//! networks (`.local`, `.lan`, `.home`, `.home.arpa`, `.internal`,
//! `.localdomain`). Any other name is one somebody registered, and it gets the
//! bypass only if the owner lists it in `BIRDNET_ALLOWED_HOSTS`. Setting an
//! admin password makes the whole question moot, which is what the refusal
//! page says first.

use std::sync::OnceLock;

/// Suffixes that no public DNS resolves, or that are only ever served by a home
/// router, so an outside website cannot rebind them.
const LAN_SUFFIXES: &[&str] = &[
    ".local",
    ".lan",
    ".home",
    ".home.arpa",
    ".internal",
    ".localdomain",
    ".localhost",
];

/// The names the owner has vouched for, from `BIRDNET_ALLOWED_HOSTS`
/// (comma-separated, compared case-insensitively, without a port).
fn allowed_from_env() -> &'static [String] {
    static ALLOWED: OnceLock<Vec<String>> = OnceLock::new();
    ALLOWED.get_or_init(|| {
        std::env::var("BIRDNET_ALLOWED_HOSTS")
            .map(|v| parse_allowed(&v))
            .unwrap_or_default()
    })
}

fn parse_allowed(spec: &str) -> Vec<String> {
    spec.split(',')
        .map(|s| s.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The host part of a `Host` header value (or URI authority), without the port
/// or the brackets around an IPv6 literal.
fn host_part(authority: &str) -> &str {
    let authority = authority.trim();
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("");
    }
    authority.rsplit_once(':').map_or(authority, |(h, port)| {
        if port.bytes().all(|b| b.is_ascii_digit()) {
            h
        } else {
            authority
        }
    })
}

/// Whether a request that named this station as `authority` may be given the
/// open-admin bypass.
///
/// `None` — no `Host` and no URI authority — is allowed: every browser sends a
/// `Host`, and rebinding needs a browser.
#[must_use]
pub fn may_open_bypass(authority: Option<&str>) -> bool {
    may_open_bypass_with(authority, allowed_from_env())
}

fn may_open_bypass_with(authority: Option<&str>, allowed: &[String]) -> bool {
    let Some(authority) = authority else {
        return true;
    };
    let host = host_part(authority)
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    if host.parse::<std::net::IpAddr>().is_ok() || host == "localhost" || !host.contains('.') {
        return true;
    }
    LAN_SUFFIXES.iter().any(|s| host.ends_with(s)) || allowed.iter().any(|a| *a == host)
}

/// The `Host` a request names: the header, or for HTTP/2 the URI authority.
#[must_use]
pub fn request_authority<B>(request: &axum::http::Request<B>) -> Option<String> {
    request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .or_else(|| request.uri().authority().map(|a| a.as_str().to_owned()))
}

/// What a refused request is told. `403`, because the condition is the
/// operator's to change and nothing about retrying will.
#[must_use]
pub fn refused(authority: &str) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let host = crate::routes::pages::escape_html(host_part(authority));
    (
        axum::http::StatusCode::FORBIDDEN,
        [
            (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        format!(
            "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
             <title>Set an admin password first · BirdNet-Behavior</title></head><body>\
             <h1>Set an admin password first</h1>\
             <p>This station has no admin password, so its settings are open to anyone who \
             reaches it by its local address. It was reached here as <code>{host}</code>, a \
             name that any website can point at a device on your network, so the settings \
             are refused under that name: otherwise a page you visit somewhere else could \
             change them.</p>\
             <p>Open the station by its local address (for example its IP address, or \
             <code>birdnet.local</code>) and set an admin password under \
             <b>Admin → Accounts</b>. With a password set, this name works normally.</p>\
             <p>If <code>{host}</code> is your own name for this station and you want it to \
             work without a password, add it to <code>BIRDNET_ALLOWED_HOSTS</code> and \
             restart.</p></body></html>"
        ),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(host: &str) -> bool {
        may_open_bypass_with(Some(host), &[])
    }

    #[test]
    fn names_nobody_outside_can_point_here_are_allowed() {
        for h in [
            "192.168.1.20:8502",
            "10.0.0.5",
            "[fe80::1]:8502",
            "[::1]",
            "localhost:8502",
            "birdnet",
            "birdnet:8502",
            "birdnet.local",
            "birdnet.local.",
            "pi.lan:8502",
            "station.home.arpa",
            "BIRDNET.LOCAL",
        ] {
            assert!(ok(h), "{h} should be allowed");
        }
    }

    #[test]
    fn a_registered_name_is_refused_unless_the_owner_lists_it() {
        for h in [
            "evil.example:8502",
            "rebind.attacker.com",
            "local.evil.com",
            "lan.evil.com",
            "birdnet.local.evil.com",
        ] {
            assert!(!ok(h), "{h} must be refused");
        }
        let allowed = parse_allowed(" Birds.Example.org , other.net. ");
        assert!(may_open_bypass_with(
            Some("birds.example.org:443"),
            &allowed
        ));
        assert!(may_open_bypass_with(Some("other.net"), &allowed));
        assert!(!may_open_bypass_with(Some("evil.example"), &allowed));
    }

    #[test]
    fn a_missing_host_is_not_a_browser_and_an_empty_one_is_refused() {
        assert!(may_open_bypass_with(None, &[]));
        assert!(!may_open_bypass_with(Some(""), &[]));
        assert!(!may_open_bypass_with(Some(":8502"), &[]));
    }
}
