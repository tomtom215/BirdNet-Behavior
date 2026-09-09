//! Resolve private mode and its carve-outs from the flags and the config
//! (O-4), one way, for the runtime and the doctor alike.

use std::collections::BTreeSet;

use birdnet_core::config::Config;
use birdnet_web::private_mode::PublicAccess;

use crate::cli::Cli;
use crate::helpers::resolve::{setting_bool, setting_str};

/// What the operator asked for, with the entries that could not be honoured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateModeSetting {
    /// `--private-mode` / `PRIVATE_MODE`.
    pub enabled: bool,
    /// The carve-outs that parsed.
    pub public: BTreeSet<PublicAccess>,
    /// `PUBLIC_ACCESS` entries that named no carve-out, in the order given.
    /// Reported and skipped: a misspelt carve-out closes one surface, which
    /// is the safe direction, and refusing to start over it would be the
    /// outage the hardening guide's section 5 rules out.
    pub rejected: Vec<String>,
}

/// Resolve the setting. Flags win over the config; `PUBLIC_ACCESS` is read
/// whether or not private mode is on, so a typo is reported before the day
/// the operator turns it on.
#[must_use]
pub fn resolve_private_mode(cli: &Cli, config: Option<&Config>) -> PrivateModeSetting {
    let enabled = setting_bool(
        cli,
        "private_mode",
        cli.private_mode,
        config,
        "PRIVATE_MODE",
    );
    let spec = setting_str(
        cli,
        "public_access",
        cli.public_access.as_deref().unwrap_or(""),
        config,
        "PUBLIC_ACCESS",
    );
    let mut public = BTreeSet::new();
    let mut rejected = Vec::new();
    for name in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match PublicAccess::parse(name) {
            Some(access) => {
                public.insert(access);
            }
            None => rejected.push(name.to_owned()),
        }
    }
    PrivateModeSetting {
        enabled,
        public,
        rejected,
    }
}

/// Apply the setting to the state, logging what was decided. Returns the
/// state untouched on an open station.
pub fn init_private_mode(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&Config>,
) -> birdnet_web::state::AppState {
    let setting = resolve_private_mode(cli, config);
    for name in &setting.rejected {
        tracing::error!(
            entry = %name,
            "PUBLIC_ACCESS names no carve-out; skipped. The carve-outs are live_audio, share and metrics."
        );
    }
    if !setting.enabled {
        return state;
    }
    tracing::info!(
        public_access = %PublicAccess::describe(&setting.public),
        "private mode: the dashboard, the API and the WebSockets need a sign-in"
    );
    state.with_private_mode(setting.public)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::test_support::{cli_with_explicit, config_with, default_cli};

    #[test]
    fn off_by_default_with_nothing_carved_out() {
        let s = resolve_private_mode(&default_cli(), None);
        assert_eq!(
            s,
            PrivateModeSetting {
                enabled: false,
                public: BTreeSet::new(),
                rejected: vec![],
            }
        );
    }

    #[test]
    fn the_config_turns_it_on_and_names_the_carve_outs() {
        let cfg = config_with(&[
            ("PRIVATE_MODE", "true"),
            ("PUBLIC_ACCESS", "share, metrics"),
        ]);
        let s = resolve_private_mode(&default_cli(), Some(&cfg));
        assert!(s.enabled);
        assert_eq!(
            s.public,
            [PublicAccess::Share, PublicAccess::Metrics]
                .into_iter()
                .collect()
        );
        assert!(s.rejected.is_empty());
    }

    #[test]
    fn an_explicit_flag_beats_the_config() {
        let cfg = config_with(&[("PRIVATE_MODE", "false"), ("PUBLIC_ACCESS", "share")]);
        let mut cli = cli_with_explicit(&["private_mode", "public_access"]);
        cli.private_mode = true;
        cli.public_access = Some("live_audio".to_owned());
        let s = resolve_private_mode(&cli, Some(&cfg));
        assert!(s.enabled);
        assert_eq!(s.public, std::iter::once(PublicAccess::LiveAudio).collect());
    }

    #[test]
    fn an_unknown_carve_out_is_reported_and_the_rest_kept() {
        let cfg = config_with(&[
            ("PRIVATE_MODE", "1"),
            ("PUBLIC_ACCESS", "share,live_audo,metrics"),
        ]);
        let s = resolve_private_mode(&default_cli(), Some(&cfg));
        assert!(s.enabled);
        assert_eq!(
            s.public,
            [PublicAccess::Share, PublicAccess::Metrics]
                .into_iter()
                .collect()
        );
        assert_eq!(s.rejected, vec!["live_audo".to_owned()]);
    }

    #[test]
    fn a_non_boolean_reads_as_off() {
        let cfg = config_with(&[("PRIVATE_MODE", "maybe")]);
        assert!(!resolve_private_mode(&default_cli(), Some(&cfg)).enabled);
    }

    #[test]
    fn init_applies_the_setting_to_the_state() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        let state = birdnet_web::state::AppState::from_connection(
            conn,
            std::path::PathBuf::from(":memory:"),
        );
        let cfg = config_with(&[("PRIVATE_MODE", "yes"), ("PUBLIC_ACCESS", "share")]);
        let state = init_private_mode(state, &default_cli(), Some(&cfg));
        assert!(state.private_mode());
        assert_eq!(
            state.public_access(),
            &std::iter::once(PublicAccess::Share).collect::<BTreeSet<_>>()
        );
    }
}
