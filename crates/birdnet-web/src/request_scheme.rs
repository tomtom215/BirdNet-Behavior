//! Whether a browser reached the station over HTTPS — decided per request.
//!
//! # Why this exists
//!
//! The session cookie's `Secure` attribute used to come from one place: the
//! scheme of the `BNB_PUBLIC_URL` environment variable. That is a statement
//! about how the operator *intends* the station to be reached, and `Secure` is
//! a statement about how *this request* arrived. They disagree in both
//! directions:
//!
//! * `--tls-mode` with no such variable, or a TLS reverse proxy that was never
//!   told its public name, issued a cookie without `Secure` on an encrypted
//!   connection. Cookies are not scoped to a port, so the same cookie is then
//!   sent in clear to the station's plain-HTTP listener.
//! * An `https://` public URL made every sign-in `Secure`, including one from
//!   `http://192.168.1.5:8502` on the LAN — where the browser refuses to store
//!   a `Secure` cookie from an insecure origin and the sign-in loops without
//!   saying why.
//!
//! # The rule
//!
//! A request is HTTPS when any of these holds:
//!
//! 1. **It arrived on the station's own TLS listener** ([`crate::tls::ViaTls`]).
//!    A fact about the connection; nobody's say-so.
//! 2. **A trusted proxy says so**: `X-Forwarded-Proto` or `Forwarded: proto=`,
//!    from a peer that [`TrustedProxies`] trusts — the same question, answered
//!    by the same code, as whose `X-Forwarded-For` is believed. From any other
//!    peer the headers are ignored. (Believing a stranger's would only hurt the
//!    stranger, whose browser would drop the cookie, but a rule that asks
//!    "who said so?" about one forwarded header and not another is the kind
//!    that gets copied wrongly.) The *leftmost* entry is read: it is what the
//!    browser-facing hop saw, and a later plain-HTTP hop does not change what
//!    the browser used.
//! 3. **`BNB_PUBLIC_URL` is `https://` and the request was addressed to that
//!    host** (`Host` equals its authority). The documented nginx configuration
//!    sets `Host` and `X-Forwarded-For` but not `X-Forwarded-Proto`, so a proxy
//!    that sends neither header would otherwise lose `Secure` it used to have.
//!    The `Host` test is what keeps the reverse trap shut: a LAN browser at
//!    `http://192.168.1.5:8502` sends a different `Host`, so it is not
//!    `Secure` however the public URL is set.
//!
//! Anything else is plain HTTP, and the cookie is not `Secure`.

use std::net::IpAddr;

use axum::http::{HeaderMap, header};

use crate::client_ip::TrustedProxies;

/// How a request reached the station, published as a request extension by the
/// rate-limit layer (the first that knows the peer and the trusted proxies).
///
/// Absent only where that layer did not run — a handler called directly in a
/// test — and then [`cookie_secure`] falls back to the public-URL rule alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestScheme {
    /// The browser used HTTPS for this request: rule 1 or 2 above.
    pub https: bool,
}

/// Rules 1 and 2: the connection's own scheme, or a trusted proxy's word for it.
#[must_use]
pub fn resolve(
    trusted: &TrustedProxies,
    headers: &HeaderMap,
    peer: IpAddr,
    via_tls: bool,
) -> RequestScheme {
    let https = via_tls || (trusted.is_trusted(peer) && forwarded_https(headers));
    RequestScheme { https }
}

/// Whether the forwarding headers say the browser-facing hop was HTTPS.
fn forwarded_https(headers: &HeaderMap) -> bool {
    if let Some(first) = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
    {
        return first.trim().eq_ignore_ascii_case("https");
    }
    // RFC 7239: `Forwarded: for=…;proto=https, for=…` — the first element is the
    // browser-facing hop, and `proto` is one of its `;`-separated pairs.
    headers
        .get(header::FORWARDED)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .is_some_and(|element| {
            element.split(';').any(|pair| {
                pair.split_once('=').is_some_and(|(k, v)| {
                    k.trim().eq_ignore_ascii_case("proto")
                        && v.trim().trim_matches('"').eq_ignore_ascii_case("https")
                })
            })
        })
}

/// Whether the session cookie issued for this request carries `Secure`.
///
/// `scheme` is the extension [`RequestScheme`] when the rate-limit layer ran.
/// Reads `BNB_PUBLIC_URL` for rule 3; [`cookie_secure_with`] takes it as a
/// parameter so the rule can be tested without touching the environment.
#[must_use]
pub fn cookie_secure(scheme: Option<&RequestScheme>, headers: &HeaderMap) -> bool {
    cookie_secure_with(
        scheme,
        headers,
        std::env::var("BNB_PUBLIC_URL").ok().as_deref(),
    )
}

/// [`cookie_secure`] with the configured public URL given explicitly.
#[must_use]
pub fn cookie_secure_with(
    scheme: Option<&RequestScheme>,
    headers: &HeaderMap,
    public_url: Option<&str>,
) -> bool {
    scheme.is_some_and(|s| s.https) || addressed_to_public_https_host(headers, public_url)
}

/// Rule 3: `public_url` is `https://` and the request's `Host` is its authority.
fn addressed_to_public_https_host(headers: &HeaderMap, public_url: Option<&str>) -> bool {
    let Some(rest) = public_url.and_then(|u| u.trim().strip_prefix("https://")) else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    // `:443` is the default for https; a Host that spells it out is the same host.
    let canon = |s: &str| {
        let s = s.trim().to_ascii_lowercase();
        s.strip_suffix(":443").unwrap_or(&s).to_owned()
    };
    !authority.is_empty() && canon(authority) == canon(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    fn peer(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn https(trusted: &TrustedProxies, h: &HeaderMap, p: &str, via_tls: bool) -> bool {
        resolve(trusted, h, peer(p), via_tls).https
    }

    /// Rule 1: the station's own listener needs no header and no proxy.
    #[test]
    fn the_tls_listener_is_https_without_any_header() {
        let t = TrustedProxies::default();
        assert!(https(&t, &HeaderMap::new(), "203.0.113.9", true));
        // Counterpart: the same stranger over plain HTTP is not.
        assert!(!https(&t, &HeaderMap::new(), "203.0.113.9", false));
    }

    /// Rule 2, with the discriminator that matters: the same header is believed
    /// from a trusted proxy and ignored from anyone else.
    #[test]
    fn a_forwarded_scheme_is_believed_only_from_a_trusted_proxy() {
        let t = TrustedProxies::default();
        let h = headers(&[("x-forwarded-proto", "https")]);
        assert!(https(&t, &h, "127.0.0.1", false), "loopback is a proxy");
        assert!(
            https(&t, &h, "192.168.1.2", false),
            "the default trusts private"
        );
        assert!(!https(&t, &h, "203.0.113.9", false), "a stranger's word");
        // With trust narrowed to loopback, the LAN peer is a stranger too.
        let strict = TrustedProxies::loopback_only();
        assert!(!https(&strict, &h, "192.168.1.2", false));
        assert!(https(&strict, &h, "127.0.0.1", false));
    }

    #[test]
    fn the_leftmost_hop_is_the_browsers() {
        let t = TrustedProxies::default();
        let ok = |v: &str| {
            https(
                &t,
                &headers(&[("x-forwarded-proto", v)]),
                "127.0.0.1",
                false,
            )
        };
        assert!(ok("https"));
        assert!(ok("HTTPS"));
        assert!(ok(" https , http"));
        assert!(!ok("http"));
        assert!(!ok("http, https"));
        assert!(!ok(""));
        assert!(!ok("wss"));
    }

    #[test]
    fn the_standard_forwarded_header_is_read_when_the_x_one_is_absent() {
        let t = TrustedProxies::default();
        let ok = |v: &str| https(&t, &headers(&[("forwarded", v)]), "127.0.0.1", false);
        assert!(ok("for=203.0.113.5;proto=https"));
        assert!(ok(r#"for="[2001:db8::1]";PROTO="https";by=10.0.0.1"#));
        assert!(ok("for=1.2.3.4;proto=https, for=10.0.0.9;proto=http"));
        assert!(!ok("for=203.0.113.5;proto=http"));
        assert!(!ok("for=203.0.113.5"));
        // Not a substring match: a host or a `for` that merely contains it.
        assert!(!ok("for=https;host=proto=https"));
    }

    // ── rule 3, and the reverse trap ────────────────────────────────────

    #[test]
    fn an_https_public_url_marks_only_requests_addressed_to_it() {
        let url = Some("https://birds.example.com");
        let none = None::<&RequestScheme>;
        // Through a proxy that sent no scheme header: Host is the public name.
        let via_proxy = headers(&[("host", "birds.example.com")]);
        assert!(cookie_secure_with(none, &via_proxy, url));
        // **The reverse trap**: the same station, same configuration, reached
        // directly over plain HTTP on the LAN. `Secure` here is a login loop.
        let lan = headers(&[("host", "192.168.1.5:8502")]);
        assert!(!cookie_secure_with(none, &lan, url));
        let lan_name = headers(&[("host", "birdnet.local")]);
        assert!(!cookie_secure_with(none, &lan_name, url));
    }

    #[test]
    fn the_public_url_comparison_is_about_the_host_and_nothing_else() {
        let none = None::<&RequestScheme>;
        let h = |host| headers(&[("host", host)]);
        // Case, a trailing path on the URL, and an explicit default port.
        assert!(cookie_secure_with(
            none,
            &h("Birds.Example.COM"),
            Some("https://birds.example.com/")
        ));
        assert!(cookie_secure_with(
            none,
            &h("birds.example.com:443"),
            Some("https://birds.example.com/app")
        ));
        // A non-default port is a different authority.
        assert!(!cookie_secure_with(
            none,
            &h("birds.example.com:8443"),
            Some("https://birds.example.com")
        ));
        // A lookalike is not the host.
        assert!(!cookie_secure_with(
            none,
            &h("birds.example.com.evil.test"),
            Some("https://birds.example.com")
        ));
        // `http://` public URLs, an unset one, and a missing Host never match.
        assert!(!cookie_secure_with(
            none,
            &h("birds.example.com"),
            Some("http://birds.example.com")
        ));
        assert!(!cookie_secure_with(none, &h("birds.example.com"), None));
        assert!(!cookie_secure_with(
            none,
            &HeaderMap::new(),
            Some("https://birds.example.com")
        ));
        assert!(!cookie_secure_with(none, &h("anything"), Some("https://")));
    }

    #[test]
    fn a_connection_that_was_https_needs_no_public_url() {
        let secure = RequestScheme { https: true };
        let plain = RequestScheme { https: false };
        assert!(cookie_secure_with(Some(&secure), &HeaderMap::new(), None));
        assert!(!cookie_secure_with(Some(&plain), &HeaderMap::new(), None));
    }
}
