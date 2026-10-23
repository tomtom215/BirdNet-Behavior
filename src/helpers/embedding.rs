//! Which sites may put the station in an `<iframe>` (`P-9`), resolved from
//! `EMBED_ORIGINS` for the runtime and the doctor alike.
//!
//! Off unless the key is set. An unusable value switches embedding **off**, not
//! to a best guess: see [`birdnet_web::embed`] for why one bad entry refuses the
//! whole list.

use birdnet_core::config::Config;
use birdnet_web::embed::{CONFIG_KEY, EmbedError, EmbedOrigins};

/// The origins the configuration allows, or what is wrong with them.
///
/// `Ok(none)` when the key is absent or empty, which is the default.
///
/// # Errors
///
/// [`EmbedError`] for the first entry that is not an acceptable origin.
pub fn resolve_embed_origins(config: Option<&Config>) -> Result<EmbedOrigins, EmbedError> {
    // Read as a literal `config.get("KEY")`: `tests/every_config_key_is_known.rs`
    // finds reads by their shape.
    config
        .and_then(|c| c.get("EMBED_ORIGINS"))
        .map_or_else(|| Ok(EmbedOrigins::none()), EmbedOrigins::parse)
}

/// Install the resolved origins on the station, saying so in the journal.
///
/// An invalid value is an error in the log and embedding stays off.
pub fn apply_embed_origins(state: &birdnet_web::state::AppState, config: Option<&Config>) {
    match resolve_embed_origins(config) {
        Ok(origins) if origins.is_empty() => {}
        Ok(origins) => {
            tracing::info!(
                origins = %origins.origins().join(" "),
                "other sites may embed this station in a frame ({CONFIG_KEY})"
            );
            state.set_embed_origins(origins);
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "{CONFIG_KEY} is not usable, so no site may embed this station"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(value: &str) -> Config {
        Config::parse(&format!("EMBED_ORIGINS={value}\n")).expect("parses")
    }

    #[test]
    fn unset_or_empty_is_off() {
        assert!(resolve_embed_origins(None).unwrap().is_empty());
        assert!(
            resolve_embed_origins(Some(&Config::parse("LATITUDE=1\n").unwrap()))
                .unwrap()
                .is_empty()
        );
        assert!(resolve_embed_origins(Some(&cfg(""))).unwrap().is_empty());
    }

    #[test]
    fn a_listed_origin_is_resolved() {
        let c = cfg("http://homeassistant.local:8123, https://x.ui.nabu.casa");
        let o = resolve_embed_origins(Some(&c)).unwrap();
        assert_eq!(
            o.origins(),
            ["http://homeassistant.local:8123", "https://x.ui.nabu.casa"]
        );
    }

    #[test]
    fn a_bad_value_is_an_error_and_applies_nothing() {
        let c = cfg("https://ok.example, *");
        assert!(resolve_embed_origins(Some(&c)).is_err());
        let state = birdnet_web::state::AppState::from_connection(
            rusqlite::Connection::open_in_memory().unwrap(),
            std::path::PathBuf::from(":memory:"),
        );
        apply_embed_origins(&state, Some(&c));
        assert!(
            state.embed_origins().is_empty(),
            "the valid first entry was kept although the list was refused"
        );
        // And the counterpart: a good list does apply.
        apply_embed_origins(&state, Some(&cfg("https://ok.example")));
        assert_eq!(state.embed_origins().origins(), ["https://ok.example"]);
    }
}
