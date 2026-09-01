//! How the log filter is composed from `RUST_LOG`, the config file and the CLI.
//!
//! # Why this is not just `RUST_LOG`
//!
//! `RUST_LOG` is a developer's variable. It is not written down anywhere an
//! operator looks, it does not survive a `systemctl edit`, and pointing someone
//! at it during a support conversation means explaining Rust's module paths
//! before they can turn on the one subsystem that is misbehaving. BirdNET-Pi
//! exposed a level *per service* precisely because that conversation is common.
//!
//! We have one process rather than eight services, so the equivalent knob is a
//! level per **subsystem**, and [`SUBSYSTEMS`] is the operator-facing vocabulary
//! for them: `audio`, `detection`, `web`, `db`, `integrations`. An operator sets
//! `LOG_MODULES=audio=debug` in `birdnet.conf` and gets the capture path's debug
//! output without learning that it lives in `birdnet_core::audio`.
//!
//! # Precedence
//!
//! `RUST_LOG` still wins outright when set, because a developer who exported it
//! means it and should not have to reconcile it with a config file. Otherwise
//! the filter is the global level plus any per-subsystem overrides.

/// Default log filter when nothing else is configured.
pub const DEFAULT_LOG_FILTER: &str = "info,birdnet_behavior=debug";

/// Operator-facing subsystem names, mapped to the crate/module paths they
/// stand for.
///
/// Names an operator would use, not the ones the code uses: someone whose RTSP
/// camera is dropping out is looking for "audio", and should not have to know
/// that the supervisor lives in the binary crate while the capture process
/// lives in `birdnet_core`. Each entry expands to every path that subsystem
/// spans, which is why one name can produce several directives.
pub const SUBSYSTEMS: &[(&str, &[&str])] = &[
    (
        "audio",
        &["birdnet_core::audio", "birdnet_behavior::capture"],
    ),
    (
        "detection",
        &["birdnet_core::detection", "birdnet_behavior::daemon"],
    ),
    ("web", &["birdnet_web"]),
    ("db", &["birdnet_db"]),
    (
        "integrations",
        &["birdnet_integrations", "birdnet_behavior::integrations"],
    ),
    ("analytics", &["birdnet_behavioral", "birdnet_timeseries"]),
];

/// Whether `level` is one of the five `tracing` levels.
///
/// Checked rather than passed through so a typo is reported at startup, where
/// an operator sees it, instead of being swallowed by `EnvFilter` and leaving
/// the subsystem silently at its old level.
fn is_level(level: &str) -> bool {
    matches!(
        level.to_ascii_lowercase().as_str(),
        "trace" | "debug" | "info" | "warn" | "error" | "off"
    )
}

/// Expand `audio=debug,web=warn` into `EnvFilter` directives.
///
/// Unknown names and malformed entries are reported in the returned warnings
/// rather than dropped in silence: a filter that ignores what it cannot parse
/// looks identical to one that parsed it and found nothing to say.
pub fn expand_modules(spec: &str) -> (Vec<String>, Vec<String>) {
    let mut directives = Vec::new();
    let mut warnings = Vec::new();

    for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let Some((name, level)) = entry.split_once('=') else {
            warnings.push(format!(
                "log module entry `{entry}` is not `name=level`; ignoring it"
            ));
            continue;
        };
        let (name, level) = (name.trim(), level.trim());

        if !is_level(level) {
            warnings.push(format!(
                "`{level}` is not a log level (trace, debug, info, warn, error, off); \
                 ignoring `{entry}`"
            ));
            continue;
        }

        match SUBSYSTEMS.iter().find(|(n, _)| *n == name) {
            Some((_, paths)) => {
                for path in *paths {
                    directives.push(format!("{path}={}", level.to_ascii_lowercase()));
                }
            }
            None if name.contains("::") || name.starts_with("birdnet") => {
                // An explicit Rust path: pass it through. Someone who typed one
                // knows what they are doing, and refusing it would make this
                // strictly less capable than the RUST_LOG it replaces.
                directives.push(format!("{name}={}", level.to_ascii_lowercase()));
            }
            None => {
                let known: Vec<&str> = SUBSYSTEMS.iter().map(|(n, _)| *n).collect();
                warnings.push(format!(
                    "unknown log subsystem `{name}`; known names are {}",
                    known.join(", ")
                ));
            }
        }
    }

    (directives, warnings)
}

/// Build the complete filter string from a global level and module overrides.
///
/// `None` for either means "not configured". The global level comes first so
/// the per-module directives that follow override it, which is how `EnvFilter`
/// resolves a path matched by more than one directive.
pub fn compose(level: Option<&str>, modules: Option<&str>) -> (String, Vec<String>) {
    let mut warnings = Vec::new();

    let global = match level {
        Some(l) if is_level(l) => l.to_ascii_lowercase(),
        Some(l) => {
            warnings.push(format!(
                "`{l}` is not a log level; falling back to the default filter"
            ));
            return (DEFAULT_LOG_FILTER.to_owned(), warnings);
        }
        None => return finish(DEFAULT_LOG_FILTER.to_owned(), modules, warnings),
    };

    // A global level replaces the default entirely, but the binary's own
    // `debug` default is deliberate — it is what makes the journal useful — so
    // an operator who asks for `info` gets `info` everywhere rather than a
    // filter that quietly keeps one crate louder.
    finish(global, modules, warnings)
}

fn finish(base: String, modules: Option<&str>, mut warnings: Vec<String>) -> (String, Vec<String>) {
    let mut parts = vec![base];
    if let Some(spec) = modules {
        let (directives, mut warns) = expand_modules(spec);
        warnings.append(&mut warns);
        parts.extend(directives);
    }
    (parts.join(","), warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_configured_is_the_default_filter() {
        let (f, w) = compose(None, None);
        assert_eq!(f, DEFAULT_LOG_FILTER);
        assert!(w.is_empty());
    }

    #[test]
    fn a_global_level_replaces_the_default() {
        let (f, w) = compose(Some("warn"), None);
        assert_eq!(f, "warn");
        assert!(w.is_empty());
    }

    /// The operator-facing name expands to every path the subsystem spans —
    /// the whole point of the mapping. A one-path expansion would leave the
    /// supervisor silent while the capture process talked.
    #[test]
    fn a_subsystem_name_expands_to_every_path_it_covers() {
        let (f, w) = compose(None, Some("audio=debug"));
        assert!(w.is_empty(), "{w:?}");
        assert!(f.contains("birdnet_core::audio=debug"), "{f}");
        assert!(f.contains("birdnet_behavior::capture=debug"), "{f}");
    }

    /// Module directives come after the global level, which is how `EnvFilter`
    /// lets the specific one win.
    #[test]
    fn module_overrides_follow_the_global_level() {
        let (f, _) = compose(Some("error"), Some("web=debug"));
        assert_eq!(f, "error,birdnet_web=debug");
    }

    /// A typo must be reported, not swallowed. Silently ignoring it looks
    /// exactly like a subsystem that had nothing to say.
    #[test]
    fn an_unknown_subsystem_is_reported_and_the_rest_still_applies() {
        let (f, w) = compose(None, Some("aduio=debug,web=warn"));
        assert!(
            w.iter().any(|m| m.contains("aduio") && m.contains("audio")),
            "the warning must name the typo and list the real names: {w:?}"
        );
        assert!(
            f.contains("birdnet_web=warn"),
            "one bad entry must not discard the good ones: {f}"
        );
    }

    #[test]
    fn an_invalid_level_is_reported() {
        let (_, w) = compose(None, Some("web=verbose"));
        assert!(
            w.iter().any(|m| m.contains("verbose")),
            "an invalid level must be named: {w:?}"
        );
    }

    /// An explicit Rust path still works, so this is never less capable than
    /// the `RUST_LOG` it is meant to replace for operators.
    #[test]
    fn an_explicit_rust_path_passes_through() {
        let (f, w) = compose(None, Some("birdnet_core::inference=trace"));
        assert!(w.is_empty(), "{w:?}");
        assert!(f.contains("birdnet_core::inference=trace"), "{f}");
    }

    /// Every filter this module can produce must be one `EnvFilter` accepts.
    /// Composing a string that `EnvFilter` rejects would fall back to the
    /// default at startup and look like the setting had no effect.
    #[test]
    fn every_composed_filter_parses_as_an_env_filter() {
        for (level, modules) in [
            (None, None),
            (Some("debug"), None),
            (None, Some("audio=trace,detection=warn,web=off")),
            (Some("error"), Some("db=debug,integrations=trace")),
            (None, Some("analytics=debug,birdnet_core::inference=trace")),
        ] {
            let (f, _) = compose(level, modules);
            assert!(
                tracing_subscriber::EnvFilter::try_new(&f).is_ok(),
                "EnvFilter rejected the composed filter `{f}`"
            );
        }
    }
}
