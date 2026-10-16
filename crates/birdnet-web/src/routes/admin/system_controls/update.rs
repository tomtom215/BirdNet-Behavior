//! GitHub Releases update check.

use axum::response::{Html, IntoResponse};

/// The command that updates a station to `version`.
///
/// It names the `install.sh` attached to that release, which the release
/// workflow builds from the tagged commit and attests, and not the copy on
/// `main`, which is whatever was last merged. `--proto '=https'` and
/// `--tlsv1.2` keep a redirect from moving the script onto plain http. The
/// installer then checks the archive it downloads against the release's
/// `SHA256SUMS` before it installs anything.
pub fn update_command(version: &str) -> String {
    format!(
        "curl --proto '=https' --tlsv1.2 -fsSL \
         https://github.com/tomtom215/BirdNet-Behavior/releases/download/v{version}/install.sh \
         | sudo bash -s -- --version {version}"
    )
}

/// Check GitHub Releases API for a newer version of birdnet-behavior.
///
/// Returns HTML: update badge or "up to date" message.
pub(super) async fn check_update() -> axum::response::Response {
    use axum::http::StatusCode;

    let current = env!("CARGO_PKG_VERSION");

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(format!("birdnet-behavior/{current}"))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let api_url = "https://api.github.com/repos/tomtom215/BirdNet-Behavior/releases/latest";
    let resp = client.get(api_url).send().await;

    match resp {
        Ok(r) if r.status().is_success() => {
            #[derive(serde::Deserialize)]
            struct Release {
                tag_name: String,
                html_url: String,
                published_at: Option<String>,
            }
            match r.json::<Release>().await {
                Ok(release) => {
                    let latest = release.tag_name.trim_start_matches('v').to_string();
                    let update_available = is_newer_version(&latest, current);
                    let published = release.published_at.unwrap_or_default();
                    let html = if update_available {
                        format!(
                            r#"<div class="ctl-ok-strong">
                              ⬆ Update available: v{latest} (published {published})<br>
                              <a href="{url}" target="_blank" rel="noopener"
                                 class="ctl-link">View release notes →</a><br>
                              <span class="ctl-muted-sm">
                                Run: <code>{command}</code>
                              </span>
                            </div>"#,
                            url = release.html_url,
                            command = update_command(&latest),
                        )
                    } else {
                        format!(
                            r#"<div class="ctl-muted">
                              ✓ Up to date (v{current}). Latest: v{latest} ({published}).
                            </div>"#
                        )
                    };
                    Html(html).into_response()
                }
                // A raw `reqwest::Error` or a serde message is the wrong
                // answer to "is there a new version?" — especially on a Pi in
                // a garden, where no internet is the ordinary case rather than
                // a fault. The detail goes to the log.
                Err(e) => {
                    tracing::warn!(error = %e, "update check: could not read the release list");
                    Html(
                        r#"<p class="ctl-err">The update service answered something this station could not read. Nothing was changed; try again later.</p>"#
                            .to_string(),
                    )
                    .into_response()
                }
            }
        }
        Ok(r) => {
            tracing::warn!(status = %r.status(), "update check: unexpected status");
            Html(
                r#"<p class="ctl-err">The update service is not answering right now. Nothing was changed; try again later.</p>"#
                    .to_string(),
            )
            .into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "update check: request failed");
            Html(
                r#"<p class="ctl-err">This station could not reach the internet to check for updates. That is not a problem with your station — it will keep recording either way.</p>"#
                    .to_string(),
            )
            .into_response()
        }
    }
}

/// Compare version strings (simple semver-like: "0.4.0" > "0.3.2").
fn is_newer_version(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> [u32; 3] {
        let mut parts = v.split('.');
        let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        [major, minor, patch]
    };
    parse(latest) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S1: the update command used to pipe `main/install.sh` (whatever was last
    /// merged) through `sudo bash`, over a curl that would follow a redirect to
    /// plain http. It names the release's own asset now.
    #[test]
    fn the_update_command_names_the_release_not_main() {
        let cmd = update_command("0.18.0");
        assert!(
            cmd.contains("/releases/download/v0.18.0/install.sh"),
            "{cmd}"
        );
        assert!(cmd.contains("--version 0.18.0"), "{cmd}");
        assert!(
            cmd.contains("--proto '=https'") && cmd.contains("--tlsv1.2"),
            "{cmd}"
        );
        assert!(!cmd.contains("/main/"), "{cmd}");
        assert!(
            !cmd.contains('\n'),
            "the command is shown in a <code> element and copied as one line: {cmd:?}"
        );
    }

    #[test]
    fn version_comparison() {
        assert!(is_newer_version("0.4.0", "0.3.2"));
        assert!(!is_newer_version("0.3.2", "0.4.0"));
        assert!(!is_newer_version("0.3.2", "0.3.2"));
        assert!(is_newer_version("1.0.0", "0.99.99"));
    }
}
