//! GitHub Releases update check.

use axum::response::{Html, IntoResponse};

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
                            r#"<div style="color:#4ade80;font-weight:600;">
                              ⬆ Update available: v{latest} (published {published})<br>
                              <a href="{url}" target="_blank" rel="noopener"
                                 style="color:#38bdf8;">View release notes →</a><br>
                              <span style="color:#94a3b8;font-size:.8rem;">
                                Run: <code>curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash</code>
                              </span>
                            </div>"#,
                            url = release.html_url
                        )
                    } else {
                        format!(
                            r#"<div style="color:#94a3b8;">
                              ✓ Up to date (v{current}). Latest: v{latest} ({published}).
                            </div>"#
                        )
                    };
                    Html(html).into_response()
                }
                Err(e) => Html(format!(r#"<p style="color:#f87171;">Parse error: {e}</p>"#))
                    .into_response(),
            }
        }
        Ok(r) => Html(format!(
            r#"<p style="color:#f87171;">GitHub API returned {}</p>"#,
            r.status()
        ))
        .into_response(),
        Err(e) => Html(format!(
            r#"<p style="color:#f87171;">Network error: {e}</p>"#
        ))
        .into_response(),
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

    #[test]
    fn version_comparison() {
        assert!(is_newer_version("0.4.0", "0.3.2"));
        assert!(!is_newer_version("0.3.2", "0.4.0"));
        assert!(!is_newer_version("0.3.2", "0.3.2"));
        assert!(is_newer_version("1.0.0", "0.99.99"));
    }
}
