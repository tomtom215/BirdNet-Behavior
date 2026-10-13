//! Refuse outbound requests to addresses an operator-typed URL must not reach.
//!
//! An alert-rule webhook URL, and the Test button that fires it, will send a
//! request wherever the URL says. Pointed at `169.254.169.254` on a cloud VM
//! that request reads the instance's credentials; pointed at a link-local
//! service it reaches something that was never meant to be addressable. This
//! is the guard the unattended-deployment audit asked for as **O-11**.
//!
//! # The policy
//!
//! Ported from tphakala/birdnet-go's `internal/httpclient/ssrf.go`
//! (`isBlockedTargetIP`, 6d2335c), which settled the same trade-off:
//!
//! * **Refused:** link-local unicast (`169.254.0.0/16`, `fe80::/10` — the
//!   first holds the AWS/GCP/Azure metadata address), link-local multicast,
//!   the unspecified address, and two metadata addresses outside link-local:
//!   Alibaba's `100.100.100.200` and AWS's IPv6 `fd00:ec2::254`. Each of
//!   those is also refused when it arrives inside an IPv6 address that
//!   carries an IPv4 one — IPv4-mapped, IPv4-compatible, NAT64, 6to4,
//!   Teredo — so it cannot be laundered through a translation gateway.
//! * **Allowed:** loopback, RFC 1918, unique-local (`fc00::/7`, apart from the
//!   one address above), CGNAT (Tailscale lives there), and everything public.
//!   The commonest webhook on a home station is Home Assistant or Node-RED on
//!   the same LAN or the same box, and a guard that broke that would be
//!   switched off.
//!
//! # How it is enforced
//!
//! Every address a name resolves to is checked by [`GuardedResolver`], and the
//! connection is made to the addresses it returns — the address that was
//! checked is the address dialled, so DNS rebinding has no window. A name
//! that resolves to *any* refused address is refused outright: a mixed answer
//! is how a rebinding attack starts.
//!
//! A URL whose host is an IP literal never reaches a resolver (hyper-util
//! connects to a literal directly), so [`check_url`] covers that case, and the
//! redirect policy applies it to every hop. The `url` crate has already
//! normalised the host by then, so `http://2852039166/` and
//! `http://0xa9.0xfe.0xa9.0xfe/` arrive as `169.254.169.254`.
//!
//! Guarded clients do not use `HTTP(S)_PROXY`: through a proxy, the proxy
//! resolves the name and the guard sees nothing. birdnet-go made the same
//! call. A webhook to a LAN service does not need a proxy.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// Most redirects a guarded client follows. Each hop is checked; the bound is
/// so a redirect loop ends rather than so the guard can be skipped.
pub const MAX_REDIRECTS: usize = 5;

/// A request refused because of where it would go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// The refused address, or the host that resolved to one.
    pub target: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "refused by the outbound-address guard: {} is link-local, unspecified or a \
             cloud metadata address",
            self.target
        )
    }
}

impl std::error::Error for Refused {}

/// Whether a request to `ip` must be refused. See the module docs.
#[must_use]
pub fn is_refused(ip: IpAddr) -> bool {
    let ip = unmap(ip);
    refused_as_is(ip) || embedded_ipv4(ip).is_some_and(|v4| refused_as_is(IpAddr::V4(v4)))
}

/// Refuse `url` if its host is an IP literal [`is_refused`] rejects.
///
/// A host *name* is not checked here; [`GuardedResolver`] checks what it
/// resolves to.
///
/// # Errors
///
/// [`Refused`] naming the literal.
pub fn check_url(url: &reqwest::Url) -> Result<(), Refused> {
    // `host_str` is the normalised host: dotted IPv4, bracketed IPv6.
    let Some(host) = url.host_str() else {
        return Ok(());
    };
    let Ok(ip) = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
    else {
        return Ok(());
    };
    if is_refused(ip) {
        // Named as the address it is, not as it was spelled.
        Err(Refused {
            target: unmap(ip).to_string(),
        })
    } else {
        Ok(())
    }
}

/// A `reqwest` client builder with the guard installed: the resolver, the
/// redirect policy, and no proxy.
///
/// The first request's URL must still go through [`check_url`] — a literal
/// host never reaches the resolver.
pub fn guarded_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(GuardedResolver))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error(format!("more than {MAX_REDIRECTS} redirects"));
            }
            match check_url(attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(refused) => attempt.error(refused),
            }
        }))
}

/// The system resolver, with every answer checked by [`is_refused`].
#[derive(Debug, Clone, Copy, Default)]
pub struct GuardedResolver;

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            // Port 0: reqwest substitutes the URL's port (or the scheme's).
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if addrs.iter().any(|a| is_refused(a.ip())) {
                return Err(Box::new(Refused { target: host }) as Box<_>);
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// `::ffff:a.b.c.d` as `a.b.c.d`; anything else unchanged.
fn unmap(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

/// The policy on one address, without looking inside it.
fn refused_as_is(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_link_local()
                || v4.is_unspecified()
                // 224.0.0.0/24, the IPv4 link-local multicast block.
                || v4.octets()[..3] == [224, 0, 0]
                || v4 == Ipv4Addr::new(100, 100, 100, 200)
        }
        IpAddr::V6(v6) => {
            v6.is_unicast_link_local()
                || v6.is_unspecified()
                // ff02::/16, link-local scope multicast.
                || v6.segments()[0] == 0xff02
                || v6 == Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254)
        }
    }
}

/// The IPv4 address carried by an IPv4-compatible, NAT64, 6to4 or Teredo
/// IPv6 address. IPv4-mapped is handled by [`unmap`] first.
fn embedded_ipv4(ip: IpAddr) -> Option<Ipv4Addr> {
    let IpAddr::V6(v6) = ip else { return None };
    let b = v6.octets();
    let last4 = || Ipv4Addr::new(b[12], b[13], b[14], b[15]);
    if b[..12] == [0, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0] {
        // 64:ff9b::/96, RFC 6052 well-known NAT64.
        Some(last4())
    } else if b[..2] == [0x20, 0x02] {
        // 2002::/16, 6to4: the IPv4 address is bytes 2..6.
        Some(Ipv4Addr::new(b[2], b[3], b[4], b[5]))
    } else if b[..4] == [0x20, 0x01, 0, 0] {
        // 2001::/32, Teredo: the client address is the last four bytes, inverted.
        Some(Ipv4Addr::new(!b[12], !b[13], !b[14], !b[15]))
    } else if b[..12] == [0; 12] && !v6.is_unspecified() && !v6.is_loopback() {
        // ::a.b.c.d, IPv4-compatible (deprecated, RFC 4291 §2.5.5.1).
        Some(last4())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn metadata_and_link_local_are_refused_in_every_encoding() {
        for s in [
            "169.254.169.254",
            "169.254.0.1",
            "0.0.0.0",
            "224.0.0.251",
            "100.100.100.200",
            "fe80::1",
            "::",
            "ff02::fb",
            "fd00:ec2::254",
            "::ffff:169.254.169.254",               // IPv4-mapped
            "::169.254.169.254",                    // IPv4-compatible
            "64:ff9b::a9fe:a9fe",                   // NAT64
            "2002:a9fe:a9fe::1",                    // 6to4
            "2001:0:4136:e378:8000:63bf:5601:5601", // Teredo carrying 169.254.169.254 inverted
        ] {
            assert!(is_refused(ip(s)), "{s} must be refused");
        }
    }

    /// The counterpart: the targets a home station's webhooks actually have.
    /// A guard that refused these would be turned off, so a blanket refusal
    /// must fail here.
    #[test]
    fn lan_loopback_and_public_targets_are_allowed() {
        for s in [
            "127.0.0.1",
            "::1",
            "192.168.1.20",
            "10.0.0.5",
            "172.16.4.4",
            "100.101.102.103", // Tailscale (CGNAT), not the Alibaba address
            "fd12:3456::1",    // unique-local, not the AWS address
            "203.0.113.9",
            "2001:db8::1", // inside 2001::/16 but not Teredo's 2001::/32
            "::ffff:192.168.1.20",
        ] {
            assert!(!is_refused(ip(s)), "{s} must be allowed");
        }
    }

    #[test]
    fn a_literal_host_is_checked_after_the_url_crate_normalises_it() {
        for u in [
            "http://169.254.169.254/latest/meta-data/",
            "http://2852039166/",          // decimal
            "http://0xa9.0xfe.0xa9.0xfe/", // hex octets
            "http://[::ffff:a9fe:a9fe]/",
        ] {
            let url = reqwest::Url::parse(u).unwrap();
            assert_eq!(
                check_url(&url),
                Err(Refused {
                    target: "169.254.169.254".into()
                }),
                "{u}"
            );
        }
        assert!(check_url(&reqwest::Url::parse("http://192.168.1.20:8123/api").unwrap()).is_ok());
        assert!(
            check_url(&reqwest::Url::parse("http://homeassistant.local:8123/").unwrap()).is_ok()
        );
    }
}
