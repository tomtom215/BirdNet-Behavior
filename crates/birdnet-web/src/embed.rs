//! Letting another site frame this one (`P-9`): the origins the station may be
//! embedded by, validated, and the `frame-ancestors` source list they make.
//!
//! The station says `frame-ancestors 'self'` and `X-Frame-Options: SAMEORIGIN`
//! on every response, so no other page can put it in an `<iframe>`. That is the
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
//! An entry that does not parse refuses the **whole** value, and the station
//! then embeds nowhere. Dropping the bad entry and keeping the rest would
//! silently narrow what the operator typed; dropping all of it would silently
//! widen nothing. The setting is off (an empty list) unless set.
//!
//! # What embedding does and does not give you
//!
//! The frame shows the station as an anonymous visitor sees it. The session
//! cookie is `SameSite=Lax`, which a browser withholds from a cross-site frame,
//! so a sign-in does not carry into it — the same rule that stops the framing
//! site from using the visitor's session, and a reason not to loosen it.

use std::fmt::Write as _;

/// Most origins accepted: a person lists a handful of places; a list this long
/// is a paste of the wrong thing.
pub const MAX_ORIGINS: usize = 16;

/// Longest single origin accepted, in bytes (a DNS name is at most 253).
const MAX_ORIGIN_LEN: usize = 300;

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
    let star = if wildcard { "*." } else { "" };
    Ok(port.map_or_else(
        || format!("{scheme}://{star}{host}"),
        |p| format!("{scheme}://{star}{host}:{p}"),
    ))
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
        assert_eq!(ok("https://*.ui.nabu.casa"), ["https://*.ui.nabu.casa"]);
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
