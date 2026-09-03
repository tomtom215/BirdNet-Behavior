//! Deciding which address belongs to the client, when something sits in front.
//!
//! # Why this is not a boolean
//!
//! Every request arrives from a peer — the far end of the TCP connection —
//! and may carry headers claiming a different, earlier origin. Believing those
//! headers is right behind a reverse proxy and catastrophic without one,
//! because a header is whatever the sender typed.
//!
//! This module replaces a `trust_x_forwarded_for: bool` that could not be
//! correct either way. A probe run against it recorded both failures:
//!
//! ```text
//! trust=false -> A=203.0.113.5  B=127.0.0.1   <- B wrong
//! trust=true  -> A=9.9.9.9      B=9.9.9.9     <- A wrong
//! correct is     A=203.0.113.5  B=9.9.9.9
//! ```
//!
//! where **A** is a station reached directly by a hostile public peer that
//! forged `X-Forwarded-For: 9.9.9.9`, and **B** is the same station behind a
//! local reverse proxy that set the header truthfully. `false` — the only
//! state the shipped code could actually reach, because nothing set the flag —
//! meant every visitor through a proxy shared one rate-limit bucket and one
//! audit-log identity. `true` would have let any client on the open internet
//! mint a fresh bucket per request.
//!
//! Neither is a tuning question. The answer needs the **peer address**, which
//! neither setting consulted.
//!
//! # The rule
//!
//! 1. If the peer is **not** trusted, it *is* the client. Forwarded headers
//!    are ignored entirely — not preferred-but-overridable, ignored.
//! 2. If the peer **is** trusted, take the client from the forwarded headers:
//!    `CF-Connecting-IP` first (Cloudflare writes exactly one value and
//!    overwrites any the client sent), then `X-Forwarded-For` walked
//!    **right to left**, stopping at the first hop that is not itself a
//!    trusted proxy, then `X-Real-IP`.
//! 3. If every hop in `X-Forwarded-For` is trusted, the leftmost is the
//!    client — that is the whole chain being proxies we know about.
//!
//! Walking right-to-left is the part that matters. The left of that header is
//! attacker-controlled: a client can send `X-Forwarded-For: 1.2.3.4` and every
//! proxy in the path will *append* to it rather than replace it. Only the
//! entries added by hops we trust mean anything, and those are on the right.
//!
//! # What is trusted by default
//!
//! `loopback` is always trusted and cannot be removed: a proxy on the same
//! host is the single most common deployment and there is no threat model in
//! which 127.0.0.1 is a hostile stranger.
//!
//! The default beyond that is `private` — RFC1918, unique-local, and
//! link-local. The topology that needs it is a reverse proxy on *another box
//! on the LAN* (a Synology, a Home Assistant host, a Caddy VM), which is
//! common enough that refusing it would make the wrong answer the default: the
//! station would record every visitor as the proxy. A station on a LAN it does
//! not trust can set `loopback` alone.
//!
//! Note that a port-forwarded install needs none of this. DNAT rewrites the
//! destination, not the source, so the peer already *is* the public client.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use axum::http::HeaderMap;

// ---------------------------------------------------------------------------
// The resolved identity
// ---------------------------------------------------------------------------

/// The client address for the current request, as resolved by
/// [`TrustedProxies::client_ip`].
///
/// Inserted into request extensions by the rate-limit middleware, which is the
/// first layer that needs it. Handlers read it with
/// `Extension<ClientIp>` rather than reaching for `ConnectInfo` themselves —
/// a handler that consults the peer address directly gets the *proxy* on every
/// proxied station, which is the bug this module exists to remove, and it
/// would come back one handler at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

impl fmt::Display for ClientIp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

// ---------------------------------------------------------------------------
// CIDR
// ---------------------------------------------------------------------------

/// One CIDR block, or a bare address (which is a `/32` or `/128`).
///
/// Hand-rolled rather than pulling in `ipnet`: the whole of what we need is a
/// prefix comparison, and this crate's dependency list is deliberately short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpCidr {
    /// An IPv4 network: base address and prefix length in `0..=32`.
    V4(Ipv4Addr, u8),
    /// An IPv6 network: base address and prefix length in `0..=128`.
    V6(Ipv6Addr, u8),
}

impl IpCidr {
    /// Whether `ip` falls inside this block.
    ///
    /// An IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) is unmapped first, so a
    /// dual-stack listener that reports every IPv4 peer in mapped form still
    /// matches an IPv4 rule. Without this a `10.0.0.0/8` entry would silently
    /// fail to match `::ffff:10.0.0.7`.
    #[must_use]
    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = unmap(ip);
        match (self, ip) {
            (Self::V4(net, bits), IpAddr::V4(addr)) => {
                prefix_eq(&net.octets(), &addr.octets(), *bits)
            }
            (Self::V6(net, bits), IpAddr::V6(addr)) => {
                prefix_eq(&net.octets(), &addr.octets(), *bits)
            }
            _ => false,
        }
    }
}

impl fmt::Display for IpCidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V4(a, b) => write!(f, "{a}/{b}"),
            Self::V6(a, b) => write!(f, "{a}/{b}"),
        }
    }
}

/// Collapse an IPv4-mapped IPv6 address to the IPv4 address it carries.
fn unmap(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 @ IpAddr::V4(_) => v4,
    }
}

/// Whether the first `bits` bits of two big-endian byte strings agree.
fn prefix_eq(a: &[u8], b: &[u8], bits: u8) -> bool {
    let bits = usize::from(bits);
    let whole = bits / 8;
    let rest = bits % 8;
    if a[..whole] != b[..whole] {
        return false;
    }
    if rest == 0 {
        return true;
    }
    // `rest` is 1..=7 here, so the shift is in range and the mask keeps the
    // high `rest` bits.
    let mask = 0xFF_u8 << (8 - rest);
    (a[whole] & mask) == (b[whole] & mask)
}

/// Failure to parse a trusted-proxy entry.
#[derive(Debug, PartialEq, Eq)]
pub struct CidrParseError(String);

impl fmt::Display for CidrParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "not an IP address or CIDR block: {:?} (expected e.g. 10.0.0.0/8, \
             192.168.1.5, fd00::/8, or one of the names loopback, private, cloudflare)",
            self.0
        )
    }
}

impl std::error::Error for CidrParseError {}

/// Parse `10.0.0.0/8`, `192.168.1.5`, `fd00::/8` or `::1`.
///
/// # Errors
///
/// Returns [`CidrParseError`] when the address or the prefix length does not
/// parse, or when the prefix is wider than the address family allows.
pub fn parse_cidr(s: &str) -> Result<IpCidr, CidrParseError> {
    let s = s.trim();
    let err = || CidrParseError(s.to_string());
    let (addr_part, bits_part) = match s.split_once('/') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };
    let addr: IpAddr = addr_part.parse().map_err(|_| err())?;
    match addr {
        IpAddr::V4(v4) => {
            let bits = match bits_part {
                None => 32,
                Some(b) => b.parse::<u8>().map_err(|_| err())?,
            };
            if bits > 32 {
                return Err(err());
            }
            Ok(IpCidr::V4(v4, bits))
        }
        IpAddr::V6(v6) => {
            let bits = match bits_part {
                None => 128,
                Some(b) => b.parse::<u8>().map_err(|_| err())?,
            };
            if bits > 128 {
                return Err(err());
            }
            Ok(IpCidr::V6(v6, bits))
        }
    }
}

// ---------------------------------------------------------------------------
// Reserved names
// ---------------------------------------------------------------------------

/// Loopback, always trusted, never removable.
const LOOPBACK: &[&str] = &["127.0.0.0/8", "::1/128"];

/// RFC1918, unique-local, and link-local — the `private` name.
const PRIVATE: &[&str] = &[
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16",
    "fc00::/7",
    "fe80::/10",
];

/// Cloudflare's published edge ranges — the `cloudflare` name.
///
/// A snapshot, not a live fetch: <https://www.cloudflare.com/ips/> is the
/// authority and this list is checked against it at release time. It changes
/// rarely (the IPv4 set has been stable for years), and a station that needs a
/// range we do not carry can add the CIDR by hand alongside the name.
///
/// This exists so that a Cloudflare-fronted station gets the real visitor
/// address out of `X-Forwarded-For` rather than a Cloudflare edge IP. Without
/// it the right-to-left walk stops at the edge, which is a trusted-looking
/// wrong answer — every visitor recorded as one of a few hundred addresses.
const CLOUDFLARE: &[&str] = &[
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
    "2400:cb00::/32",
    "2606:4700::/32",
    "2803:f800::/32",
    "2405:b500::/32",
    "2405:8100::/32",
    "2a06:98c0::/29",
    "2c0f:f248::/32",
];

/// The default when nothing is configured: loopback plus the private ranges.
pub const DEFAULT_SPEC: &str = "private";

// ---------------------------------------------------------------------------
// TrustedProxies
// ---------------------------------------------------------------------------

/// The set of peers whose forwarded client-IP headers may be believed.
#[derive(Debug, Clone)]
pub struct TrustedProxies {
    nets: Vec<IpCidr>,
}

impl Default for TrustedProxies {
    /// Loopback plus the private ranges — see [`DEFAULT_SPEC`].
    fn default() -> Self {
        Self::parse(DEFAULT_SPEC).unwrap_or_else(|_| Self {
            nets: expand(LOOPBACK),
        })
    }
}

impl TrustedProxies {
    /// Build from a comma- or whitespace-separated list of CIDR blocks, bare
    /// addresses, and the reserved names `loopback`, `private` and
    /// `cloudflare`.
    ///
    /// Loopback is always included whether or not it is named. An empty spec
    /// therefore means "trust only the local host", which is the strictest
    /// setting an operator can ask for and still have a same-host proxy work.
    ///
    /// # Errors
    ///
    /// Returns [`CidrParseError`] naming the first entry that is neither a
    /// reserved name nor a parseable address, rather than skipping it. A
    /// typo'd CIDR that is silently dropped is a trust list that quietly does
    /// less than the operator asked for, which is the failure mode this whole
    /// module exists to avoid.
    pub fn parse(spec: &str) -> Result<Self, CidrParseError> {
        let mut nets = expand(LOOPBACK);
        for tok in spec.split([',', ' ', '\t', '\n']) {
            let tok = tok.trim();
            if tok.is_empty() {
                continue;
            }
            match tok.to_ascii_lowercase().as_str() {
                "loopback" => {}
                "private" => nets.extend(expand(PRIVATE)),
                "cloudflare" => nets.extend(expand(CLOUDFLARE)),
                _ => nets.push(parse_cidr(tok)?),
            }
        }
        Ok(Self { nets })
    }

    /// Trust nothing but the local host.
    #[must_use]
    pub fn loopback_only() -> Self {
        Self {
            nets: expand(LOOPBACK),
        }
    }

    /// Whether `ip` is a proxy whose forwarded headers we believe.
    #[must_use]
    pub fn is_trusted(&self, ip: IpAddr) -> bool {
        self.nets.iter().any(|n| n.contains(ip))
    }

    /// How many blocks are in the set (diagnostics; the loopback pair is
    /// always among them).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.nets.len()
    }

    /// Always false — loopback is unconditional.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.nets.is_empty()
    }

    /// Resolve the client address for a request from `peer` carrying
    /// `headers`.
    ///
    /// See the module documentation for the rule. The short version: an
    /// untrusted peer is the client and its headers are ignored.
    #[must_use]
    pub fn client_ip(&self, headers: &HeaderMap, peer: IpAddr) -> IpAddr {
        let peer = unmap(peer);
        if !self.is_trusted(peer) {
            return peer;
        }

        // Cloudflare writes exactly one value here and replaces anything the
        // client sent, so when the peer is trusted this is the least ambiguous
        // signal available.
        if let Some(ip) = header_ip(headers, "cf-connecting-ip") {
            return ip;
        }

        if let Some(ip) = self.walk_forwarded_for(headers) {
            return ip;
        }

        // nginx's `X-Real-IP` is a single value describing nginx's own client.
        // Last because in a chain it names the previous proxy rather than the
        // origin, which the `X-Forwarded-For` walk gets right.
        if let Some(ip) = header_ip(headers, "x-real-ip") {
            return ip;
        }

        peer
    }

    /// Walk `X-Forwarded-For` right to left, returning the first hop that is
    /// not itself a trusted proxy.
    ///
    /// Every proxy *appends*, so the rightmost entries were written by the
    /// hops closest to us — the ones we have a reason to believe. Anything to
    /// the left of the first untrusted entry was either written by that
    /// untrusted hop or forged by the original client, and is discarded.
    ///
    /// When every entry is trusted the leftmost is returned: the whole chain
    /// is proxies we know, so the first of them recorded the real client.
    fn walk_forwarded_for(&self, headers: &HeaderMap) -> Option<IpAddr> {
        // A request may carry several `X-Forwarded-For` lines; they concatenate
        // in order, so flatten them into one left-to-right list.
        let mut hops: Vec<IpAddr> = Vec::new();
        for value in headers.get_all("x-forwarded-for") {
            let Ok(text) = value.to_str() else { continue };
            for part in text.split(',') {
                if let Some(ip) = parse_hop(part) {
                    hops.push(unmap(ip));
                }
            }
        }
        let first = *hops.first()?;
        for hop in hops.iter().rev() {
            if !self.is_trusted(*hop) {
                return Some(*hop);
            }
        }
        Some(first)
    }
}

/// Parse the reserved-name tables, which are compile-time constants known to
/// be well-formed. A malformed entry here is a bug in this file, not operator
/// input, so it is dropped rather than propagated — and the tests below assert
/// every table parses in full, so it cannot be dropped silently.
fn expand(table: &[&str]) -> Vec<IpCidr> {
    table.iter().filter_map(|s| parse_cidr(s).ok()).collect()
}

/// Read a single-address header.
fn header_ip(headers: &HeaderMap, name: &str) -> Option<IpAddr> {
    let value = headers.get(name)?.to_str().ok()?;
    parse_hop(value)
}

/// Parse one `X-Forwarded-For` element.
///
/// Tolerates the two shapes seen in the wild besides a bare address: a
/// bracketed IPv6 literal (`[2001:db8::1]`), and either form with a port
/// suffix, which some proxies append. An entry we cannot read is skipped
/// rather than treated as untrusted — an unparseable hop is not evidence
/// about anyone's address.
fn parse_hop(raw: &str) -> Option<IpAddr> {
    let s = raw.trim().trim_matches('"');
    if s.is_empty() {
        return None;
    }
    if let Ok(ip) = s.parse::<IpAddr>() {
        return Some(ip);
    }
    // `[v6]` or `[v6]:port`
    if let Some(rest) = s.strip_prefix('[')
        && let Some((inner, _)) = rest.split_once(']')
        && let Ok(ip) = inner.parse::<IpAddr>()
    {
        return Some(ip);
    }
    // `v4:port` — only split on the last colon, and only when there is exactly
    // one, so a bare IPv6 literal is never mangled.
    if s.matches(':').count() == 1
        && let Some((host, _)) = s.rsplit_once(':')
        && let Ok(ip) = host.parse::<IpAddr>()
    {
        return Some(ip);
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        use axum::http::HeaderName;
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    // -- the pair that replaces the boolean ---------------------------------

    /// Topology A: no proxy. A hostile public peer forges the header.
    ///
    /// This is the half the shipped `trust_x_forwarded_for: true` would have
    /// failed. It is written as a pair with the next test on purpose: a rule
    /// that ignores the header *always* passes this one, so it proves nothing
    /// alone.
    #[test]
    fn an_untrusted_peer_cannot_forge_its_own_address() {
        let t = TrustedProxies::default();
        let got = t.client_ip(
            &headers(&[("x-forwarded-for", "9.9.9.9")]),
            ip("203.0.113.5"),
        );
        assert_eq!(
            got,
            ip("203.0.113.5"),
            "a forged X-Forwarded-For from a peer we do not trust must be ignored"
        );
    }

    /// Topology B: a local reverse proxy. The header is the truth.
    ///
    /// This is the half the shipped `trust_x_forwarded_for: false` failed —
    /// and `false` was the only state configuration could reach, so every
    /// proxied station shared one bucket and one audit identity.
    #[test]
    fn a_trusted_peer_is_believed() {
        let t = TrustedProxies::default();
        let got = t.client_ip(&headers(&[("x-forwarded-for", "9.9.9.9")]), ip("127.0.0.1"));
        assert_eq!(
            got,
            ip("9.9.9.9"),
            "behind a local proxy the forwarded address is the client"
        );
    }

    // -- the walk -----------------------------------------------------------

    /// The left of `X-Forwarded-For` is attacker-controlled: proxies append,
    /// they do not replace. A client that sends `X-Forwarded-For: 1.2.3.4` has
    /// its value carried along in front of the real one.
    ///
    /// So the walk must run right to left and stop at the first untrusted hop.
    /// Taking the *leftmost* entry — the naive reading, and what the shipped
    /// `extract_ip` did with `.split(',').next()` — returns the forgery.
    #[test]
    fn the_walk_stops_at_the_first_untrusted_hop_from_the_right() {
        let t = TrustedProxies::default();
        let got = t.client_ip(
            &headers(&[("x-forwarded-for", "1.2.3.4, 203.0.113.5, 10.0.0.2")]),
            ip("127.0.0.1"),
        );
        assert_eq!(
            got,
            ip("203.0.113.5"),
            "10.0.0.2 is a trusted hop, so the walk continues; 203.0.113.5 is not, so it is \
             the client. 1.2.3.4 is whatever that client typed."
        );
    }

    /// The counterpart: when the whole chain is proxies we trust, the leftmost
    /// entry is the client, because the first trusted hop is the one that saw
    /// them. Without this the walk would fall through to the peer and report
    /// the proxy.
    #[test]
    fn an_all_trusted_chain_yields_the_leftmost_hop() {
        let t = TrustedProxies::parse("private").unwrap();
        let got = t.client_ip(
            &headers(&[("x-forwarded-for", "192.168.1.40, 10.0.0.2")]),
            ip("127.0.0.1"),
        );
        assert_eq!(got, ip("192.168.1.40"));
    }

    /// Several `X-Forwarded-For` lines concatenate in order. A proxy chain
    /// that emits one header each rather than appending to one is legal and
    /// happens; reading only the first line would drop the hops that matter.
    #[test]
    fn repeated_headers_are_read_as_one_chain() {
        let t = TrustedProxies::default();
        let got = t.client_ip(
            &headers(&[
                ("x-forwarded-for", "1.2.3.4"),
                ("x-forwarded-for", "203.0.113.5, 10.0.0.2"),
            ]),
            ip("127.0.0.1"),
        );
        assert_eq!(got, ip("203.0.113.5"));
    }

    // -- header precedence --------------------------------------------------

    /// Cloudflare overwrites `CF-Connecting-IP` with the real visitor, so when
    /// the peer is trusted it beats an `X-Forwarded-For` whose rightmost hop
    /// is an unlisted Cloudflare edge.
    #[test]
    fn cf_connecting_ip_wins_over_forwarded_for() {
        let t = TrustedProxies::default();
        let got = t.client_ip(
            &headers(&[
                ("cf-connecting-ip", "9.9.9.9"),
                ("x-forwarded-for", "9.9.9.9, 172.68.1.1"),
            ]),
            ip("127.0.0.1"),
        );
        assert_eq!(got, ip("9.9.9.9"));
    }

    /// And the counterpart, or the precedence test above is satisfied by any
    /// rule that reads `CF-Connecting-IP`: an *untrusted* peer setting it gets
    /// nowhere. This is the header a directly-exposed station is most likely
    /// to be probed with, precisely because it is a single value with no chain
    /// to walk.
    #[test]
    fn cf_connecting_ip_from_an_untrusted_peer_is_ignored() {
        let t = TrustedProxies::default();
        let got = t.client_ip(
            &headers(&[("cf-connecting-ip", "9.9.9.9")]),
            ip("203.0.113.5"),
        );
        assert_eq!(got, ip("203.0.113.5"));
    }

    /// With the `cloudflare` name the edge ranges become trusted hops, so the
    /// walk passes through them to the visitor even with no `CF-Connecting-IP`.
    #[test]
    fn the_cloudflare_name_lets_the_walk_pass_the_edge() {
        let bare = TrustedProxies::default();
        let with_cf = TrustedProxies::parse("private, cloudflare").unwrap();
        let h = headers(&[("x-forwarded-for", "9.9.9.9, 172.68.1.1")]);

        assert_eq!(
            bare.client_ip(&h, ip("127.0.0.1")),
            ip("172.68.1.1"),
            "without the name the edge is an untrusted hop and the walk stops there"
        );
        assert_eq!(
            with_cf.client_ip(&h, ip("127.0.0.1")),
            ip("9.9.9.9"),
            "with the name the edge is a trusted hop and the walk reaches the visitor"
        );
    }

    /// `X-Real-IP` is the last resort, used only when nothing better is present.
    #[test]
    fn x_real_ip_is_the_fallback() {
        let t = TrustedProxies::default();
        assert_eq!(
            t.client_ip(&headers(&[("x-real-ip", "9.9.9.9")]), ip("127.0.0.1")),
            ip("9.9.9.9")
        );
        assert_eq!(
            t.client_ip(
                &headers(&[("x-real-ip", "9.9.9.9"), ("x-forwarded-for", "8.8.8.8")]),
                ip("127.0.0.1")
            ),
            ip("8.8.8.8"),
            "the forwarded-for walk is better evidence than X-Real-IP and must win"
        );
    }

    /// A trusted peer with no forwarded headers at all is the client itself —
    /// the ordinary case of browsing from the machine the station runs on.
    #[test]
    fn a_trusted_peer_with_no_headers_is_itself() {
        let t = TrustedProxies::default();
        assert_eq!(
            t.client_ip(&HeaderMap::new(), ip("127.0.0.1")),
            ip("127.0.0.1")
        );
    }

    // -- spec parsing -------------------------------------------------------

    /// Loopback survives a spec that does not mention it, including an empty
    /// one. A same-host proxy must work at the strictest setting available.
    #[test]
    fn loopback_is_unconditional() {
        for spec in ["", "203.0.113.0/24", "private"] {
            let t = TrustedProxies::parse(spec).unwrap();
            assert!(
                t.is_trusted(ip("127.0.0.1")),
                "loopback must stay trusted with spec {spec:?}"
            );
            assert!(t.is_trusted(ip("::1")));
        }
    }

    /// The counterpart: `loopback_only` really does exclude the private
    /// ranges. Otherwise the strict setting is the default wearing a different
    /// name and an operator who chose it got nothing.
    #[test]
    fn loopback_only_excludes_the_private_ranges() {
        let t = TrustedProxies::loopback_only();
        assert!(t.is_trusted(ip("127.0.0.1")));
        assert!(!t.is_trusted(ip("192.168.1.1")));
        assert!(!t.is_trusted(ip("10.0.0.1")));
        assert!(!t.is_trusted(ip("fd00::1")));
    }

    /// An entry that does not parse is an error, not a silent skip. A trust
    /// list that quietly does less than the operator wrote is the exact shape
    /// of the bug this module replaces.
    #[test]
    fn a_malformed_entry_is_rejected_by_name() {
        let err = TrustedProxies::parse("private, 10.0.0.0/99").unwrap_err();
        assert!(
            err.to_string().contains("10.0.0.0/99"),
            "the message must name the offending entry, got: {err}"
        );
        assert!(TrustedProxies::parse("private, not-an-ip").is_err());
        assert!(TrustedProxies::parse("192.168.1.0/24").is_ok());
    }

    /// Every reserved table is well-formed. `expand` drops what it cannot
    /// parse, so without this a typo in a constant would shrink the trust set
    /// with no symptom at all.
    #[test]
    fn every_reserved_table_parses_completely() {
        for (name, table) in [
            ("LOOPBACK", LOOPBACK),
            ("PRIVATE", PRIVATE),
            ("CLOUDFLARE", CLOUDFLARE),
        ] {
            assert_eq!(
                expand(table).len(),
                table.len(),
                "an entry in {name} did not parse"
            );
        }
    }

    // -- CIDR arithmetic ----------------------------------------------------

    /// Prefix lengths that are not whole bytes are where a hand-rolled
    /// comparison goes wrong. `172.16.0.0/12` covers 172.16–172.31 and must
    /// not reach 172.32 or 172.15.
    #[test]
    fn a_non_byte_aligned_prefix_has_the_right_edges() {
        let net = parse_cidr("172.16.0.0/12").unwrap();
        assert!(net.contains(ip("172.16.0.1")));
        assert!(net.contains(ip("172.31.255.254")));
        assert!(!net.contains(ip("172.32.0.1")));
        assert!(!net.contains(ip("172.15.255.254")));
    }

    /// A `/0` matches everything in its family and nothing outside it.
    #[test]
    fn a_zero_prefix_matches_its_whole_family_only() {
        let v4 = parse_cidr("0.0.0.0/0").unwrap();
        assert!(v4.contains(ip("203.0.113.5")));
        assert!(!v4.contains(ip("2001:db8::1")));
    }

    /// A bare address is a host route, not a network.
    #[test]
    fn a_bare_address_is_a_host_route() {
        let net = parse_cidr("192.168.1.5").unwrap();
        assert!(net.contains(ip("192.168.1.5")));
        assert!(!net.contains(ip("192.168.1.6")));
    }

    /// A dual-stack listener reports IPv4 peers as `::ffff:a.b.c.d`. An IPv4
    /// rule must still match them, or the whole trust list silently stops
    /// working the moment the socket binds `[::]`.
    #[test]
    fn an_ipv4_mapped_peer_matches_an_ipv4_rule() {
        let t = TrustedProxies::parse("private").unwrap();
        assert!(t.is_trusted(ip("::ffff:192.168.1.9")));
        assert!(!t.is_trusted(ip("::ffff:203.0.113.9")));
        // ...and the resolved client is reported unmapped, so buckets and
        // audit rows do not split one address across two spellings.
        assert_eq!(
            t.client_ip(&HeaderMap::new(), ip("::ffff:203.0.113.9")),
            ip("203.0.113.9")
        );
    }

    // -- hop parsing --------------------------------------------------------

    /// Proxies in the wild append ports and bracket IPv6 literals.
    #[test]
    fn hops_with_ports_and_brackets_are_read() {
        assert_eq!(parse_hop("203.0.113.5"), Some(ip("203.0.113.5")));
        assert_eq!(parse_hop(" 203.0.113.5:8443 "), Some(ip("203.0.113.5")));
        assert_eq!(parse_hop("[2001:db8::1]"), Some(ip("2001:db8::1")));
        assert_eq!(parse_hop("[2001:db8::1]:443"), Some(ip("2001:db8::1")));
        assert_eq!(parse_hop("\"203.0.113.5\""), Some(ip("203.0.113.5")));
    }

    /// A bare IPv6 literal has many colons and must not be mangled by the
    /// `host:port` rule.
    #[test]
    fn a_bare_ipv6_literal_is_not_split_on_its_colons() {
        assert_eq!(parse_hop("2001:db8::1"), Some(ip("2001:db8::1")));
    }

    /// `unknown` and `_obfuscated` are legal RFC 7239 values. They carry no
    /// address, so they are skipped rather than counted as an untrusted hop —
    /// otherwise one anonymising proxy in the chain would stop the walk and
    /// report a name as an address.
    #[test]
    fn unreadable_hops_are_skipped_not_trusted() {
        assert_eq!(parse_hop("unknown"), None);
        assert_eq!(parse_hop("_hidden"), None);
        assert_eq!(parse_hop(""), None);

        let t = TrustedProxies::default();
        let got = t.client_ip(
            &headers(&[("x-forwarded-for", "203.0.113.5, unknown, 10.0.0.2")]),
            ip("127.0.0.1"),
        );
        assert_eq!(got, ip("203.0.113.5"));
    }
}
