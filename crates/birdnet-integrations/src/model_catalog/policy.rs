//! How hard an install tries, and for how long it waits (MDL-13).
//!
//! The defaults are for a Raspberry Pi on home broadband: 30 s to connect, 60 s
//! of silence in the middle of a transfer before it is called stalled, and the
//! origins tried three times in all, 30 and then 90 seconds apart — the schedule
//! `install.sh` and the container used around the download before the binary
//! took it over, so an operator who set `BIRDNET_MODEL_RETRY_WAITS` keeps what
//! they set. A dropped transfer resumes where it stopped on the next try, so
//! three tries is three chances to finish, not three restarts.

use std::time::Duration;

/// Seconds to wait before each further round of the origins, space separated
/// (`"30 90"`: three rounds). Empty is one round. Tests set `"0 0"`.
pub const RETRY_WAITS_ENV: &str = "BIRDNET_MODEL_RETRY_WAITS";

/// Seconds a connection to an origin may take to open.
pub const CONNECT_TIMEOUT_ENV: &str = "BIRDNET_MODEL_CONNECT_TIMEOUT";

/// Seconds an origin may go without sending a byte before the attempt is
/// abandoned (and resumed on the next one).
pub const STALL_TIMEOUT_ENV: &str = "BIRDNET_MODEL_STALL_TIMEOUT";

/// A base URL tried before every catalogue origin.
///
/// Each file is asked for as `<base>/<installed file name>`, so a directory
/// that holds what `--install-model` writes, served over HTTP, is a mirror. For
/// a site with no internet or a metered one.
pub const MIRROR_ENV: &str = "BIRDNET_MODEL_MIRROR";

/// Every environment variable [`FetchPolicy::from_env`] reads, so the binary
/// does not report them as variables nothing reads.
pub const POLICY_ENV_KEYS: &[&str] = &[
    RETRY_WAITS_ENV,
    CONNECT_TIMEOUT_ENV,
    STALL_TIMEOUT_ENV,
    MIRROR_ENV,
];

/// The default connect timeout.
const DEFAULT_CONNECT: Duration = Duration::from_secs(30);
/// The default stall timeout.
const DEFAULT_STALL: Duration = Duration::from_secs(60);
/// The default waits between rounds.
const DEFAULT_WAITS: [Duration; 2] = [Duration::from_secs(30), Duration::from_secs(90)];

/// The network side of an install: timeouts, retries and an operator's mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPolicy {
    /// How long opening a connection may take.
    pub connect_timeout: Duration,
    /// How long an origin may send nothing — before its answer, or between
    /// two reads of its body — before the attempt is abandoned.
    pub stall_timeout: Duration,
    /// The wait before each further round of the origins; one round more than
    /// there are waits.
    pub retry_waits: Vec<Duration>,
    /// An operator's mirror, tried first: `<mirror>/<installed file name>`.
    pub mirror: Option<String>,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        Self {
            connect_timeout: DEFAULT_CONNECT,
            stall_timeout: DEFAULT_STALL,
            retry_waits: DEFAULT_WAITS.to_vec(),
            mirror: None,
        }
    }
}

impl FetchPolicy {
    /// The defaults, with whatever the environment overrides. A value that does
    /// not parse is reported and the default used: a typo in a wait must not
    /// stop a station getting its model.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_lookup(&|key| std::env::var(key).ok())
    }

    /// [`Self::from_env`], reading through `get` so a test needs no process
    /// environment.
    #[must_use]
    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> Self {
        let mut policy = Self::default();
        if let Some(raw) = get(RETRY_WAITS_ENV) {
            if let Some(waits) = parse_waits(&raw) {
                policy.retry_waits = waits;
            } else {
                tracing::warn!(
                    value = %raw,
                    "{RETRY_WAITS_ENV} is not a list of seconds; using 30 90"
                );
            }
        }
        for (key, slot) in [
            (CONNECT_TIMEOUT_ENV, &mut policy.connect_timeout),
            (STALL_TIMEOUT_ENV, &mut policy.stall_timeout),
        ] {
            if let Some(raw) = get(key) {
                if let Some(d) = parse_positive_secs(&raw) {
                    *slot = d;
                } else {
                    tracing::warn!(
                        value = %raw,
                        "{key} is not a whole number of seconds above zero; using the default"
                    );
                }
            }
        }
        policy.mirror = get(MIRROR_ENV)
            .map(|m| m.trim().trim_end_matches('/').to_owned())
            .filter(|m| !m.is_empty());
        policy
    }

    /// The origins for a file installed as `name`: the operator's mirror, if
    /// any, then `catalogue` in its order.
    #[must_use]
    pub fn origins_for(&self, name: &str, catalogue: &[&str]) -> Vec<String> {
        self.mirror
            .iter()
            .map(|m| format!("{m}/{name}"))
            .chain(catalogue.iter().map(|u| (*u).to_owned()))
            .collect()
    }
}

/// `"30 90"` as two durations; `""` as none. `None` for anything else.
fn parse_waits(raw: &str) -> Option<Vec<Duration>> {
    raw.split_whitespace()
        .map(|w| w.parse::<u64>().ok().map(Duration::from_secs))
        .collect()
}

/// A whole number of seconds above zero.
fn parse_positive_secs(raw: &str) -> Option<Duration> {
    raw.trim()
        .parse::<u64>()
        .ok()
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn with(pairs: &[(&str, &str)]) -> FetchPolicy {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        FetchPolicy::from_lookup(&|k| map.get(k).cloned())
    }

    #[test]
    fn the_defaults_are_the_installers_schedule() {
        let p = with(&[]);
        assert_eq!(p.connect_timeout, Duration::from_secs(30));
        assert_eq!(p.stall_timeout, Duration::from_secs(60));
        assert_eq!(
            p.retry_waits,
            vec![Duration::from_secs(30), Duration::from_secs(90)]
        );
        assert_eq!(p.mirror, None);
    }

    #[test]
    fn the_environment_overrides_each_and_a_bad_value_keeps_the_default() {
        let p = with(&[
            (RETRY_WAITS_ENV, "0 0"),
            (CONNECT_TIMEOUT_ENV, "5"),
            (STALL_TIMEOUT_ENV, "7"),
            (MIRROR_ENV, "http://mirror.lan/models/"),
        ]);
        assert_eq!(p.retry_waits, vec![Duration::ZERO, Duration::ZERO]);
        assert_eq!(p.connect_timeout, Duration::from_secs(5));
        assert_eq!(p.stall_timeout, Duration::from_secs(7));
        assert_eq!(
            p.origins_for("birdnet-v24.onnx", &["https://a/x"]),
            vec![
                "http://mirror.lan/models/birdnet-v24.onnx".to_owned(),
                "https://a/x".to_owned()
            ]
        );
        assert!(with(&[(RETRY_WAITS_ENV, "")]).retry_waits.is_empty());
        let bad = with(&[
            (RETRY_WAITS_ENV, "soon"),
            (STALL_TIMEOUT_ENV, "0"),
            (CONNECT_TIMEOUT_ENV, "x"),
        ]);
        assert_eq!(bad, FetchPolicy::default());
    }
}
