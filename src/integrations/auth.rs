//! HTTP Basic Auth config construction from the config file.

/// Create an HTTP Basic Auth config from the config file.
///
/// Looks for `CADDY_PWD` (password) and defaults username to "birdnet"
/// to match BirdNET-Pi's Caddy setup.
pub fn create_auth_config(
    config: Option<&birdnet_core::config::Config>,
) -> Option<birdnet_web::auth::AuthConfig> {
    let password = config?.get("CADDY_PWD")?;
    let username = config
        .and_then(|c| c.get("CADDY_USER"))
        .unwrap_or("birdnet");

    let auth = birdnet_web::auth::AuthConfig::new(username, password)?;
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
