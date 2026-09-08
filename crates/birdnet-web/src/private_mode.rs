//! Private mode: the whole station behind the sign-in, with named carve-outs
//! (`O-4`).
//!
//! The station's default contract is *viewing is open, only changing things
//! needs a password*. That is right for a Pi on a home LAN and wrong for the
//! same Pi behind a tunnel or a port forward, where "anyone who can reach the
//! port" is the internet: the detection history, the live microphone feed of
//! somebody's garden and both `WebSockets` are all served to whoever has the
//! URL, and `--listen 127.0.0.1` is no help because the tunnel is exactly what
//! connects to it.
//!
//! Private mode moves the *public* router behind the same cookie gate the
//! admin panel uses. What stays open is the smallest set a browser needs to
//! reach the sign-in form (`/login`, `/logout`, the static assets), the health
//! probe the watchdog and the container healthcheck depend on, and whatever
//! the operator names in [`PublicAccess`]:
//!
//! * `live_audio` — the `/stream` endpoint and the live spectrogram socket,
//!   for a station whose feed is meant to be listened to;
//! * `share` — the signed `/r/<token>` links the operator mints on purpose;
//! * `metrics` — `/api/v2/metrics`, for a Prometheus scraper that has no
//!   cookie.
//!
//! Everything else answers a `303` to `/login` (pages) or a `401` (the API and
//! the `WebSockets`). The decision is a pure function of the path and the
//! carve-out set, [`is_open`], so the whole exempt table is a unit test.

use std::collections::BTreeSet;
use std::fmt;

/// A surface the operator may leave open on a private station.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PublicAccess {
    /// The live audio stream and the live spectrogram WebSocket.
    LiveAudio,
    /// Operator-minted share links (`/r/<token>` and its media).
    Share,
    /// The Prometheus exposition at `/api/v2/metrics`.
    Metrics,
}

impl PublicAccess {
    /// Every carve-out, in the order the docs list them.
    pub const ALL: [Self; 3] = [Self::LiveAudio, Self::Share, Self::Metrics];

    /// The name the configuration uses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LiveAudio => "live_audio",
            Self::Share => "share",
            Self::Metrics => "metrics",
        }
    }

    /// Parse one name. Case-insensitive; a hyphen is accepted for the
    /// underscore because `live-audio` is what someone typing it will try.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase().replace('-', "_");
        Self::ALL.into_iter().find(|a| a.as_str() == name)
    }

    /// Parse the comma-separated list `BIRDNET_PUBLIC_ACCESS` /
    /// `PUBLIC_ACCESS` carries. Empty entries are ignored, so `""` and
    /// `"share,"` both parse; an unknown name is an error naming it and the
    /// names that exist, because a misspelt `live_audo` silently dropped
    /// would be a stream that vanished with no message anywhere.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownPublicAccess`] for the first entry that is not a
    /// carve-out.
    pub fn parse_list(spec: &str) -> Result<BTreeSet<Self>, UnknownPublicAccess> {
        spec.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| Self::parse(s).ok_or_else(|| UnknownPublicAccess(s.to_owned())))
            .collect()
    }

    /// Render a set the way the configuration spells it (`share,metrics`),
    /// or `none` for an empty set.
    #[must_use]
    pub fn describe(set: &BTreeSet<Self>) -> String {
        if set.is_empty() {
            return "none".to_owned();
        }
        set.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(",")
    }
}

/// A `PUBLIC_ACCESS` entry that names no carve-out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPublicAccess(pub String);

impl fmt::Display for UnknownPublicAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = PublicAccess::ALL
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        write!(
            f,
            "{:?} is not a public-access carve-out; the carve-outs are {names}",
            self.0
        )
    }
}

impl std::error::Error for UnknownPublicAccess {}

/// Whether `path` is served without a session on a private station.
///
/// `path` is the request path as the router sees it, base path already
/// stripped by `Router::nest`. The always-open set is exactly what a browser
/// needs to show the sign-in form and what the watchdog needs to see the
/// process alive; every carve-out is opt-in.
#[must_use]
pub fn is_open(path: &str, public: &BTreeSet<PublicAccess>) -> bool {
    if matches!(
        path,
        "/api/v2/health" | "/login" | "/logout" | "/favicon.ico"
    ) || path.starts_with("/static/")
    {
        return true;
    }
    public.iter().any(|access| match access {
        PublicAccess::LiveAudio => matches!(path, "/stream" | "/api/v2/ws/spectrogram"),
        PublicAccess::Share => path.starts_with("/r/"),
        PublicAccess::Metrics => path == "/api/v2/metrics",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[PublicAccess]) -> BTreeSet<PublicAccess> {
        items.iter().copied().collect()
    }

    #[test]
    fn the_sign_in_and_the_probe_are_always_open() {
        let none = BTreeSet::new();
        for path in [
            "/api/v2/health",
            "/login",
            "/logout",
            "/favicon.ico",
            "/static/css/app.css",
            "/static/fonts/x.woff2",
        ] {
            assert!(
                is_open(path, &none),
                "{path} must be open with no carve-outs"
            );
        }
    }

    #[test]
    fn with_no_carve_outs_everything_else_is_closed() {
        let none = BTreeSet::new();
        for path in [
            "/",
            "/today",
            "/api/v2/detections",
            "/api/v2/ws/detections",
            "/api/v2/ws/spectrogram",
            "/api/v2/metrics",
            "/api/v2/recordings/x.wav",
            "/stream",
            "/r/abc",
            "/r/abc/audio.wav",
            "/feeds/rss",
            "/staticx",
            "/loginx",
        ] {
            assert!(
                !is_open(path, &none),
                "{path} must be closed with no carve-outs"
            );
        }
    }

    #[test]
    fn each_carve_out_opens_only_its_own_surface() {
        let live = set(&[PublicAccess::LiveAudio]);
        assert!(is_open("/stream", &live));
        assert!(is_open("/api/v2/ws/spectrogram", &live));
        assert!(!is_open("/api/v2/ws/detections", &live));
        assert!(!is_open("/r/abc", &live));
        assert!(!is_open("/api/v2/metrics", &live));

        let share = set(&[PublicAccess::Share]);
        assert!(is_open("/r/abc", &share));
        assert!(is_open("/r/abc/audio.wav", &share));
        assert!(is_open("/r/abc/spectrogram.png", &share));
        assert!(!is_open("/stream", &share));
        assert!(!is_open("/api/v2/recordings/x.wav", &share));

        let metrics = set(&[PublicAccess::Metrics]);
        assert!(is_open("/api/v2/metrics", &metrics));
        assert!(!is_open("/api/v2/health/x", &metrics));
        assert!(!is_open("/stream", &metrics));
    }

    #[test]
    fn the_list_parses_loosely_and_rejects_the_unknown() {
        assert_eq!(
            PublicAccess::parse_list("live_audio, share").unwrap(),
            set(&[PublicAccess::LiveAudio, PublicAccess::Share])
        );
        assert_eq!(
            PublicAccess::parse_list("Live-Audio,,METRICS,").unwrap(),
            set(&[PublicAccess::LiveAudio, PublicAccess::Metrics])
        );
        assert!(PublicAccess::parse_list("").unwrap().is_empty());
        let err = PublicAccess::parse_list("share,live_audo").unwrap_err();
        assert_eq!(err, UnknownPublicAccess("live_audo".to_owned()));
        assert!(err.to_string().contains("live_audio, share, metrics"));
    }

    #[test]
    fn the_config_validator_knows_every_carve_out() {
        // `birdnet_core::config::validate` checks `PUBLIC_ACCESS` for the
        // doctor and cannot see this enum; the two lists are held equal here
        // so a carve-out added to one is not "unknown" to the other.
        let here: Vec<&str> = PublicAccess::ALL.iter().map(|a| a.as_str()).collect();
        assert_eq!(
            here,
            birdnet_core::config::validate::PUBLIC_ACCESS_NAMES.to_vec()
        );
    }

    #[test]
    fn describe_spells_the_set_as_the_config_does() {
        assert_eq!(PublicAccess::describe(&BTreeSet::new()), "none");
        assert_eq!(
            PublicAccess::describe(&set(&[PublicAccess::Metrics, PublicAccess::LiveAudio])),
            "live_audio,metrics"
        );
    }
}
