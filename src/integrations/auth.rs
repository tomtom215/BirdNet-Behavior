//! HTTP Basic Auth config construction from the config file.

/// Create an HTTP Basic Auth config from the config file or the environment.
///
/// Looks for `CADDY_PWD` (password) and `CADDY_USER` (username, default
/// "birdnet") to match BirdNET-Pi's Caddy setup. The `birdnet.conf` file is
/// checked first, then the environment — so auth can be enabled in Docker
/// (which configures via env vars, not a config file) as well as on bare metal.
pub fn create_auth_config(
    config: Option<&birdnet_core::config::Config>,
) -> Option<birdnet_web::auth::AuthConfig> {
    let password = config
        .and_then(|c| c.get("CADDY_PWD").map(str::to_owned))
        .or_else(|| std::env::var("CADDY_PWD").ok())
        .filter(|p| !p.is_empty())?;
    let username = config
        .and_then(|c| c.get("CADDY_USER").map(str::to_owned))
        .or_else(|| std::env::var("CADDY_USER").ok())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "birdnet".to_owned());

    let auth = birdnet_web::auth::AuthConfig::new(&username, &password)?;
    tracing::info!(username = %username, "basic auth enabled");
    Some(auth)
}

#[cfg(test)]
mod tests {
    use super::create_auth_config;
    use crate::integrations::test_support::config_with;

    #[test]
    fn auth_none_when_no_password_configured() {
        // Config absent.
        assert!(create_auth_config(None).is_none());
        // Config present but no CADDY_PWD.
        let cfg = config_with(&[("SOMETHING", "irrelevant")]);
        assert!(create_auth_config(Some(&cfg)).is_none());
    }

    #[test]
    fn auth_built_with_default_username_when_only_password_set() {
        let cfg = config_with(&[("CADDY_PWD", "hunter2")]);
        let auth = create_auth_config(Some(&cfg));
        assert!(auth.is_some(), "auth should be built when password set");
    }

    #[test]
    fn auth_built_with_custom_username() {
        let cfg = config_with(&[("CADDY_PWD", "hunter2"), ("CADDY_USER", "operator")]);
        let auth = create_auth_config(Some(&cfg));
        assert!(auth.is_some());
    }
}
