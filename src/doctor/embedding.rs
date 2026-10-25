//! Cross-site embedding (`EMBED_ORIGINS`): is the list usable, and who may
//! frame the station?
//!
//! Silent when the key is unset: that is the safe default and there is nothing
//! to report. A value that does not parse is a failure, because the station
//! then embeds nowhere and the operator who typed it expects Home Assistant to
//! show the frame.

use birdnet_core::config::Config;

use super::Check;
use crate::cli::Cli;

const NAME: &str = "Embedding";

pub(super) fn check_embedding(_cli: &Cli, config: Option<&Config>) -> Option<Check> {
    match crate::helpers::embedding::resolve_embed_origins(config) {
        Ok(o) if o.is_empty() => None,
        Ok(o) => Some(Check::pass(
            NAME,
            format!(
                "may be framed by {}; no other site can",
                o.origins().join(", ")
            ),
        )),
        Err(e) => Some(Check::fail(
            NAME,
            format!("EMBED_ORIGINS is not usable, so no site may frame the station: {e}"),
            "fix or remove EMBED_ORIGINS in the configuration file, then restart",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::check_embedding;
    use crate::doctor::Status;
    use clap::Parser as _;

    fn run(value: Option<&str>) -> Option<crate::doctor::Check> {
        let cli = crate::cli::Cli::parse_from(["birdnet-behavior"]);
        let cfg = value
            .map(|v| birdnet_core::config::Config::parse(&format!("EMBED_ORIGINS={v}\n")).unwrap());
        check_embedding(&cli, cfg.as_ref())
    }

    #[test]
    fn off_is_silent_a_list_passes_and_a_bad_one_fails() {
        assert!(run(None).is_none());
        assert!(run(Some("")).is_none());
        let pass = run(Some("https://ha.example:8123")).expect("reported");
        assert_eq!(pass.status, Status::Pass);
        assert!(pass.message.contains("https://ha.example:8123"));
        let fail = run(Some("https://ha.example/path")).expect("reported");
        assert_eq!(fail.status, Status::Fail);
        assert!(fail.message.contains("/path"), "{}", fail.message);
    }
}
