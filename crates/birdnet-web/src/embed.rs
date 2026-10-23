//! Letting another site frame this one (`P-9`): the origins the station may be
//! embedded by, validated, and the `frame-ancestors` source list they make.
//!
//! The station says `frame-ancestors 'self'` and `X-Frame-Options: SAMEORIGIN`
//! on every response, so no other page can put it in an `<iframe>` — unless the
//! operator lists the origin (below), and then only for the few pages meant to
//! be shown in a frame ([`allows_framing_of`]). That is the
//! right default for a page that has an admin login behind it: a site that can
//! frame the station can lay invisible controls over it ("clickjacking").
//! It also means a Home Assistant dashboard cannot show the station's live
//! view in a card. tphakala/birdnet-go's `webserver.allowembedding` is a switch
//! for exactly that, and it is a switch for *everyone*: with it on, any website
//! may frame the station.
//!
//! # A list, not a switch
//!
//! This is a list of origins instead, because the people who ask for it know
//! where they want it embedded: their Home Assistant at
//! `http://homeassistant.local:8123` or `https://xxxx.ui.nabu.casa`. Each entry
//! is a full origin (`scheme://host[:port]`, `http` or `https`), optionally with
//! a leading `*.` on the host to cover its subdomains. There is deliberately no
//! bare `*`, no scheme alone and no `*.com`: a station that "allows embedding by
//! anyone" is the one thing the default exists to prevent, and nothing a
//! Home Assistant, a Grafana or a kiosk page needs requires it.
//!
//! # Wildcards over a suffix other people register under
//!
//! `*.example.com` covers one organisation's names, which is what a wildcard is
//! for. `*.co.uk`, `*.github.io`, `*.pages.dev` and `*.ui.nabu.casa` cover
//! everyone who registers a name under them — every Home Assistant Cloud
//! customer is a subdomain of `ui.nabu.casa` — so each would let strangers
//! frame the station. Such an entry is refused (`shared_suffix`) with the
//! host to list instead. The list is a short hand-kept one of the multi-tenant
//! suffixes people actually meet, plus the generic second levels of the country
//! codes (`co.uk`, `com.au`, …); it is **not** the Public Suffix List, which this
//! crate does not carry, so a suffix missing from it is not protected. An entry
//! naming one host (`https://abcd1234.ui.nabu.casa`) is never affected: that is
//! one tenant, and it is the right thing to list.
//!
//! # Which pages
//!
//! Only the pages a dashboard card or a wall display shows —
//! `/` (Today), `/kiosk` and `/recordings` (the live view) — take the listed
//! origins. `/admin/*`, `/station/*`, `/login`, `/logout`, `/onboarding`,
//! `/restore` and every other page keep `frame-ancestors 'self'` and
//! `X-Frame-Options: SAMEORIGIN` whatever is listed, so a listed site (or a
//! script injected into it) cannot lay invisible controls over the sign-in form
//! or the administration pages.
//!
//! An entry that does not parse refuses the **whole** value, and the station
//! then embeds nowhere. Dropping the bad entry and keeping the rest would
//! silently narrow what the operator typed; dropping all of it would silently
//! widen nothing. The setting is off (an empty list) unless set.
//!
//! # What embedding does and does not give you
//!
//! For an embedder on another *site*, the frame shows the station as an
//! anonymous visitor sees it: the session cookie is `SameSite=Lax`, which a
//! browser withholds from a cross-site frame, so a sign-in does not carry into
//! it. "Site" is the registrable domain, not the origin: an embedder on the same
//! host with another port (`http://192.168.1.10:8123` framing
//! `http://192.168.1.10:8502`, the usual Home Assistant set-up) or on a sibling
//! subdomain is the **same site**, the cookie is sent, and a signed-in
//! administrator sees the listed pages with their controls. That is why only the
//! pages above take the list, and why the list should name only places you trust
//! with your browser.

use std::fmt::Write as _;

/// Most origins accepted: a person lists a handful of places; a list this long
/// is a paste of the wrong thing.
pub const MAX_ORIGINS: usize = 16;

/// Longest single origin accepted, in bytes (a DNS name is at most 253).
const MAX_ORIGIN_LEN: usize = 300;

/// The pages a listed origin may frame: the dashboard, the wall display and the
/// live view. Matched exactly, after the base path (if any) is removed.
pub const FRAMEABLE_PATHS: [&str; 3] = ["/", "/kiosk", "/recordings"];

/// Whether the origins an operator listed apply to `path`.
///
/// `path` is the request path as the router sees it (without the base path). An
/// allow-list rather than a list of the pages to protect: a page added later is
/// unframeable until somebody decides it should be.
#[must_use]
pub fn allows_framing_of(path: &str) -> bool {
    let path = if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    };
    FRAMEABLE_PATHS.contains(&path)
}

/// The setting's key in the `settings` table, and the config key it is bridged
/// onto.
pub const SETTING_KEY: &str = "embed_origins";

/// The config key the binary reads the saved value through.
pub const CONFIG_KEY: &str = "EMBED_ORIGINS";

/// A validated list of origins that may frame the station. Empty means none:
/// the default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbedOrigins {
    origins: Vec<String>,
}

/// Why a list was refused: the entry, and what is wrong with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedError {
    /// The entry as written (truncated if very long).
    pub entry: String,
    /// What is wrong, as one clause that reads after the entry.
    pub reason: &'static str,
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} {}. Write each place as scheme://host or scheme://host:port, \
             for example https://homeassistant.example:8123, separated by commas.",
            self.entry, self.reason
        )
    }
}

impl std::error::Error for EmbedError {}

fn refuse(entry: &str, reason: &'static str) -> EmbedError {
    EmbedError {
        entry: entry.chars().take(80).collect(),
        reason,
    }
}

impl EmbedOrigins {
    /// No origin may frame the station.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            origins: Vec::new(),
        }
    }

    /// Whether anything may frame the station.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }

    /// The origins, normalised (lower case, no trailing slash).
    #[must_use]
    pub fn origins(&self) -> &[String] {
        &self.origins
    }

    /// Parse a comma-, space- or newline-separated list.
    ///
    /// # Errors
    ///
    /// [`EmbedError`] naming the first entry that is not an acceptable origin;
    /// nothing is kept in that case.
    pub fn parse(text: &str) -> Result<Self, EmbedError> {
        let mut origins: Vec<String> = Vec::new();
        for raw in text
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .filter(|e| !e.is_empty())
        {
            let origin = normalise(raw)?;
            if !origins.contains(&origin) {
                origins.push(origin);
            }
            if origins.len() > MAX_ORIGINS {
                return Err(refuse(raw, "is one more than the sixteen origins allowed"));
            }
        }
        Ok(Self { origins })
    }

    /// The `frame-ancestors` directive (without a trailing `;`): `'self'` always,
    /// then each origin.
    #[must_use]
    pub fn frame_ancestors(&self) -> String {
        let mut out = String::from("frame-ancestors 'self'");
        for o in &self.origins {
            let _ = write!(out, " {o}");
        }
        out
    }
}

/// One entry to its canonical origin, or why not.
fn normalise(raw: &str) -> Result<String, EmbedError> {
    if raw.len() > MAX_ORIGIN_LEN {
        return Err(refuse(raw, "is too long to be an origin"));
    }
    if !raw.is_ascii() || raw.bytes().any(|b| b.is_ascii_control()) {
        return Err(refuse(
            raw,
            "has characters an origin cannot (write an internationalised name in its xn-- form)",
        ));
    }
    let lower = raw.to_ascii_lowercase();
    let Some((scheme, rest)) = lower.split_once("://") else {
        return Err(refuse(
            raw,
            "has no scheme (it must start with http:// or https://)",
        ));
    };
    if scheme != "http" && scheme != "https" {
        return Err(refuse(raw, "has a scheme other than http or https"));
    }
    // An origin is scheme, host, port: nothing after, and nothing before.
    if rest.contains(['/', '?', '#', '@', '\\', '\'', '"', ' ', ';', ',']) {
        return Err(refuse(
            raw,
            "is more than an origin (no path, query, user name or quotes)",
        ));
    }
    let (host_part, port) =
        split_port(rest).ok_or_else(|| refuse(raw, "has a port that is not 1 to 65535"))?;
    let (wildcard, host) = host_part
        .strip_prefix("*.")
        .map_or((false, host_part), |h| (true, h));
    if host.contains('*') {
        return Err(refuse(
            raw,
            "has a * somewhere other than the start of the host",
        ));
    }
    if !is_host(host) {
        return Err(refuse(raw, "has a host that is not a name or an address"));
    }
    if wildcard && host.split('.').count() < 2 {
        return Err(refuse(
            raw,
            "is a wildcard over a whole top-level domain; name at least a domain, such as *.example.com",
        ));
    }
    if wildcard && is_ip(host) {
        return Err(refuse(raw, "puts a wildcard on an IP address"));
    }
    if wildcard && shared_suffix(host) {
        return Err(refuse(
            raw,
            "is a wildcard over a suffix that unrelated people register names under, so it \
             would let strangers frame the station; list the one host instead, \
             such as https://yourname.<that suffix>",
        ));
    }
    let star = if wildcard { "*." } else { "" };
    Ok(port.map_or_else(
        || format!("{scheme}://{star}{host}"),
        |p| format!("{scheme}://{star}{host}:{p}"),
    ))
}

/// Multi-tenant suffixes: names under each are registered by unrelated parties,
/// so a wildcard over the suffix itself names all of them.
///
/// Hand-kept, not the Public Suffix List (see the module docs). Lower case, no
/// leading dot. A host *under* one of these (`you.github.io`) is one tenant and
/// is not matched.
const SHARED_SUFFIXES: &[&str] = &[
    "github.io",
    "githubusercontent.com",
    "gitlab.io",
    "pages.dev",
    "workers.dev",
    "netlify.app",
    "vercel.app",
    "herokuapp.com",
    "web.app",
    "firebaseapp.com",
    "appspot.com",
    "azurewebsites.net",
    "cloudfront.net",
    "amazonaws.com",
    "blogspot.com",
    "wordpress.com",
    "myshopify.com",
    "fly.dev",
    "onrender.com",
    "glitch.me",
    "repl.co",
    "surge.sh",
    "ngrok.io",
    "ngrok-free.app",
    "trycloudflare.com",
    "duckdns.org",
    "no-ip.org",
    "ddns.net",
    "dyndns.org",
    "ts.net",
    "nabu.casa",
    "ui.nabu.casa",
    "nip.io",
    "sslip.io",
    "xip.io",
];

/// The generic second-level labels country-code domains hang registrations
/// under (`co.uk`, `com.au`, `ac.nz`, …).
const GENERIC_SECOND_LEVELS: &[&str] = &[
    "co", "com", "org", "net", "gov", "edu", "ac", "or", "ne", "go", "mil", "nom", "sch", "gob",
];

/// Whether `host` (the part after `*.`) is a suffix other people register names
/// under: one of [`SHARED_SUFFIXES`], or a country code with a generic second
/// level (`co.uk`).
fn shared_suffix(host: &str) -> bool {
    if SHARED_SUFFIXES.contains(&host) {
        return true;
    }
    let mut labels = host.split('.');
    matches!(
        (labels.next(), labels.next(), labels.next()),
        (Some(second), Some(cc), None)
            if GENERIC_SECOND_LEVELS.contains(&second) && cc.len() == 2
    )
}

/// `host[:port]` to `(host, Some(port))`. `None` for a bad port. A bracketed
/// IPv6 literal keeps its brackets.
fn split_port(rest: &str) -> Option<(&str, Option<u16>)> {
    if let Some(end) = rest.strip_prefix('[').and_then(|r| r.find(']')) {
        let (host, tail) = rest.split_at(end + 2);
        return match tail {
            "" => Some((host, None)),
            t => t
                .strip_prefix(':')?
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .map(|p| (host, Some(p))),
        };
    }
    match rest.rsplit_once(':') {
        None => Some((rest, None)),
        Some((host, port)) => {
            let p = port.parse::<u16>().ok().filter(|p| *p > 0)?;
            Some((host, Some(p)))
        }
    }
}

fn is_ip(host: &str) -> bool {
    host.parse::<std::net::Ipv4Addr>().is_ok()
        || host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .is_some_and(|h| h.parse::<std::net::Ipv6Addr>().is_ok())
}

fn is_host(host: &str) -> bool {
    if is_ip(host) {
        return true;
    }
    if host.is_empty() || host.len() > 253 || host.starts_with('.') || host.ends_with('.') {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Vec<String> {
        EmbedOrigins::parse(text)
            .unwrap_or_else(|e| panic!("{text:?} was refused: {e}"))
            .origins()
            .to_vec()
    }

    fn refused(text: &str) -> EmbedError {
        EmbedOrigins::parse(text)
            .err()
            .unwrap_or_else(|| panic!("{text:?} was accepted"))
    }

    #[test]
    fn the_places_people_actually_embed_from_are_accepted() {
        assert_eq!(
            ok("http://homeassistant.local:8123"),
            ["http://homeassistant.local:8123"]
        );
        assert_eq!(
            ok("https://abcd1234.ui.nabu.casa"),
            ["https://abcd1234.ui.nabu.casa"]
        );
        assert_eq!(
            ok("https://*.example.com"),
            ["https://*.example.com"],
            "a wildcard over one organisation's own domain is what a wildcard is for"
        );
        assert_eq!(ok("http://192.168.1.10:8123"), ["http://192.168.1.10:8123"]);
        assert_eq!(ok("http://[::1]:3000"), ["http://[::1]:3000"]);
        assert_eq!(ok("https://grafana.lan"), ["https://grafana.lan"]);
    }

    #[test]
    fn separators_case_and_repeats_are_tolerated_and_normalised() {
        assert_eq!(
            ok("HTTPS://HA.Example:8123,\n http://ha.example:8123  https://ha.example:8123 ;"),
            ["https://ha.example:8123", "http://ha.example:8123"],
            "lower-cased, deduplicated, any separator"
        );
        assert!(ok("").is_empty());
        assert!(ok(" \n ,, ").is_empty());
    }

    #[test]
    fn nothing_that_would_let_anyone_embed_is_accepted() {
        for bad in [
            "*",
            "https://*",
            "*.example.com",
            "https://*.com",
            "http://*.localhost",
            "https://*.1.2.3.4",
            "https:",
            "https://",
            "http://*:8123",
            "'self'",
            "data:",
            "https://example.com:*",
        ] {
            let _ = refused(bad);
        }
    }

    /// A wildcard over a suffix strangers register under names all of them. Each
    /// was accepted before: `*.co.uk` and `*.github.io` and `*.pages.dev` by the
    /// label count alone, and the docs' own `*.ui.nabu.casa` example let every
    /// Home Assistant Cloud customer frame the station.
    #[test]
    fn a_wildcard_over_a_shared_suffix_is_refused_and_one_tenant_is_not() {
        for bad in [
            "https://*.co.uk",
            "https://*.com.au",
            "https://*.ac.nz",
            "https://*.github.io",
            "https://*.pages.dev",
            "https://*.ui.nabu.casa",
            "https://*.nabu.casa",
            "https://*.ts.net",
            "http://*.duckdns.org:8123",
            "https://*.nip.io",
        ] {
            let e = refused(bad);
            assert!(e.reason.contains("list the one host"), "{bad}: {e}");
        }
        // The counterpart: one tenant, and a wildcard under a tenant, are fine.
        for good in [
            "https://abcd1234.ui.nabu.casa",
            "https://you.github.io",
            "https://*.you.github.io",
            "https://*.home.example.co.uk",
            "https://*.example.co.uk",
            "https://*.tail1234.ts.net",
            "https://*.example.org",
            "https://*.co.example.com",
        ] {
            let _ = ok(good);
        }
    }

    #[test]
    fn only_the_dashboard_the_wall_display_and_the_live_view_take_the_list() {
        for yes in ["/", "/kiosk", "/kiosk/", "/recordings"] {
            assert!(allows_framing_of(yes), "{yes}");
        }
        for no in [
            "/login",
            "/logout",
            "/onboarding",
            "/onboarding/done",
            "/restore",
            "/admin",
            "/admin/overview",
            "/station",
            "/station/capture",
            "/recordings/x",
            "/kioskx",
            "/api/v2/stats",
            "",
        ] {
            assert!(!allows_framing_of(no), "{no}");
        }
    }

    #[test]
    fn anything_that_is_not_just_an_origin_is_refused() {
        for bad in [
            "homeassistant.local",
            "ftp://ha.example",
            "https://ha.example/dashboard",
            "https://ha.example/",
            "https://ha.example?x=1",
            "https://user@ha.example",
            "https://ha.example:99999",
            "https://ha.example:0",
            "https://ha.example:80a",
            "https://ha_example.lan",
            "https://-bad.example",
            "https://bad..example",
            "https://exa'mple.com",
            "https://ex ample.com",
            "https://exämple.com",
            "https://a.example\u{7}",
            "https://[::1",
            "https://[nope]:80",
        ] {
            let _ = refused(bad);
        }
    }

    #[test]
    fn one_bad_entry_refuses_the_whole_list_and_names_it() {
        let e = refused("https://ha.example:8123, https://ha.example/x");
        assert!(e.entry.contains("/x"), "{e}");
        assert!(e.to_string().contains("scheme://host"), "{e}");
    }

    #[test]
    fn the_list_has_a_length() {
        let many = (0..=MAX_ORIGINS)
            .map(|i| format!("https://h{i}.example"))
            .collect::<Vec<_>>()
            .join(",");
        let _ = refused(&many);
        let exactly = (0..MAX_ORIGINS)
            .map(|i| format!("https://h{i}.example"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(ok(&exactly).len(), MAX_ORIGINS);
    }

    #[test]
    fn the_directive_is_self_first_and_default_is_self_only() {
        assert_eq!(
            EmbedOrigins::none().frame_ancestors(),
            "frame-ancestors 'self'"
        );
        assert_eq!(
            EmbedOrigins::parse("https://a.example, http://b.lan:8123")
                .unwrap()
                .frame_ancestors(),
            "frame-ancestors 'self' https://a.example http://b.lan:8123"
        );
    }

    /// Nothing accepted can break out of the header value or the directive.
    #[test]
    fn an_accepted_origin_cannot_carry_a_directive_separator() {
        for text in [
            "https://a.example",
            "https://*.a.example:1",
            "http://[::1]:2",
        ] {
            let d = EmbedOrigins::parse(text).unwrap().frame_ancestors();
            assert!(!d.contains([';', ',', '\n', '\r', '"']), "{d}");
            assert_eq!(
                d.matches('\'').count(),
                2,
                "only 'self' carries quotes: {d}"
            );
            assert!(d.is_ascii(), "{d}");
        }
    }
}
