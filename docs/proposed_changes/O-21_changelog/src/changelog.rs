//! Changelog viewer + post-upgrade banner. See O-21 DIFF.md.
//!
//! `CHANGELOG.md` is embedded at build time so the page works offline. The
//! parser only knows the "Keep a Changelog" shape we already use — if the
//! file ever drifts, the page renders the raw markdown in a `<pre>` block.

use std::fmt::Write as _;

use axum::Router;
use axum::response::Html;
use axum::routing::get;

use super::{escape_html, render_page};
use crate::state::AppState;

// Embedded at build time relative to crates/birdnet-web/src/routes/pages/.
const CHANGELOG_MD: &str = include_str!("../../../../../CHANGELOG.md");

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/system/changelog", get(page))
        .route("/system/changelog/latest", get(latest_partial))
}

// ---------------------------------------------------------------------------
// Public handlers
// ---------------------------------------------------------------------------

async fn page() -> Html<String> {
    let releases = parse_changelog(CHANGELOG_MD);
    let mut body = String::from(
        r#"<div class="page-head" style="align-items:flex-start;" data-screen-label="Changelog" data-om-validate>
  <div>
    <div class="bnb-eyebrow">System · release history</div>
    <h1 class="display" style="font-size:48px;line-height:1.05;letter-spacing:-0.02em;text-wrap:balance;">
      What's <em style="color:var(--moss-ink);">changed</em>.
    </h1>
    <p class="bnb-meta" style="margin-top:6px;max-width:620px;">
      Every release of BirdNet-Behavior. Click a version to permalink it.
    </p>
  </div>
</div>
<div class="bnb-card pad" style="margin-top:var(--pad-3);">
  <div class="bnb-help-drawer__body" style="height:auto;padding:0;">"#,
    );

    if releases.is_empty() {
        body.push_str(r#"<pre style="white-space:pre-wrap;">"#);
        body.push_str(&escape_html(CHANGELOG_MD));
        body.push_str("</pre>");
    } else {
        for r in &releases {
            render_release(&mut body, r);
        }
    }
    body.push_str("</div></div>");
    render_page("Changelog", &body, "system")
}

async fn latest_partial() -> Html<String> {
    let releases = parse_changelog(CHANGELOG_MD);
    let Some(latest) = releases.first() else {
        return Html(String::new());
    };
    // Returns the banner subject + body — short summary plus link.
    let summary = first_bullet_summary(latest);
    Html(format!(
        r#"<div class="bnb-banner" data-version="{ver}">
  <span class="bnb-pill moss" aria-hidden="true">v{ver}</span>
  <div class="bnb-banner__copy">
    <strong>New in v{ver}.</strong>
    <span>{summary}</span>
  </div>
  <a class="bnb-btn ghost" href="/system/changelog#v{anchor}">See all changes →</a>
  <button type="button" class="bnb-banner__close" data-banner-dismiss aria-label="Dismiss">&times;</button>
</div>"#,
        ver = escape_html(&latest.version),
        anchor = escape_html(&anchor(&latest.version)),
        summary = escape_html(&summary),
    ))
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Release<'a> {
    pub version: String,
    pub date: Option<String>,
    pub sections: Vec<(String, Vec<&'a str>)>,
}

#[must_use]
pub fn parse_changelog(input: &str) -> Vec<Release<'_>> {
    let mut out: Vec<Release<'_>> = Vec::new();
    let mut current: Option<Release<'_>> = None;
    let mut current_section: Option<(String, Vec<&str>)> = None;

    for raw in input.lines() {
        let line = raw.trim_end();

        if let Some((ver, date)) = parse_release_heading(line) {
            // commit previous section + release
            if let Some(sec) = current_section.take() {
                if let Some(r) = current.as_mut() { r.sections.push(sec) }
            }
            if let Some(prev) = current.take() {
                out.push(prev);
            }
            current = Some(Release { version: ver, date, sections: Vec::new() });
            continue;
        }

        if let Some(section_name) = parse_section_heading(line) {
            if let Some(sec) = current_section.take() {
                if let Some(r) = current.as_mut() { r.sections.push(sec) }
            }
            current_section = Some((section_name, Vec::new()));
            continue;
        }

        if let Some(bullet) = parse_bullet(raw) {
            if let Some((_, bullets)) = current_section.as_mut() {
                bullets.push(bullet);
            }
        }
        // Other lines (descriptions, blank) are ignored — bullet-only.
    }

    if let Some(sec) = current_section.take() {
        if let Some(r) = current.as_mut() { r.sections.push(sec) }
    }
    if let Some(prev) = current.take() { out.push(prev) }
    out
}

/// "## [1.5.0] - 2026-04-22" or "## [Unreleased]" → ("1.5.0", Some("2026-04-22"))
fn parse_release_heading(line: &str) -> Option<(String, Option<String>)> {
    let s = line.trim();
    if !s.starts_with("## ") { return None; }
    let s = s.strip_prefix("## ")?.trim();
    let s = s.strip_prefix('[')?;
    let close = s.find(']')?;
    let ver = s[..close].trim().to_string();
    if ver.eq_ignore_ascii_case("Unreleased") { return Some((ver, None)); }
    let rest = s[close + 1..].trim();
    let date = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("– ")).map(|s| s.trim().to_string());
    Some((ver, date))
}

/// "### Added" → "Added"
fn parse_section_heading(line: &str) -> Option<String> {
    let s = line.trim();
    s.strip_prefix("### ").map(|s| s.trim().to_string())
}

/// "- bullet" → "bullet" (preserves backticks/links unchanged for later escape)
fn parse_bullet(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if let Some(b) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
        Some(b.trim_end())
    } else { None }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

fn render_release(out: &mut String, r: &Release<'_>) {
    let anchor = anchor(&r.version);
    let date = r.date.as_deref().unwrap_or("(unreleased)");
    let _ = write!(
        out,
        r#"<article id="v{anchor}" class="changelog-release">
  <header style="display:flex;align-items:baseline;justify-content:space-between;gap:12px;margin:24px 0 6px;">
    <h2 style="font-family:var(--font-display);font-size:24px;letter-spacing:-0.01em;margin:0;">
      <a href="#v{anchor}" style="color:var(--fg);text-decoration:none;">v{ver}</a>
    </h2>
    <span class="bnb-meta mono">{date}</span>
  </header>"#,
        anchor = escape_html(&anchor),
        ver = escape_html(&r.version),
        date = escape_html(date),
    );
    for (heading, bullets) in &r.sections {
        let _ = write!(
            out,
            r#"<h3 style="margin:14px 0 6px;font-size:13px;text-transform:uppercase;letter-spacing:0.08em;color:var(--fg-3);font-weight:500;">{}</h3><ul>"#,
            escape_html(heading)
        );
        for b in bullets {
            let _ = write!(out, "<li>{}</li>", render_inline_md(b));
        }
        out.push_str("</ul>");
    }
    out.push_str("</article>");
}

/// Tiny inline-md renderer: backticks → <code>, [label](href) → <a>, escape
/// the rest. Used for changelog bullets — does **not** support all of CommonMark.
fn render_inline_md(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'`' {
            // find closing backtick
            if let Some(end) = bytes[i + 1..].iter().position(|&b| b == b'`') {
                let code = &input[i + 1..i + 1 + end];
                let _ = write!(out, "<code class=\"mono\">{}</code>", escape_html(code));
                i += end + 2;
                continue;
            }
        } else if c == b'[' {
            if let Some(rb) = bytes[i + 1..].iter().position(|&b| b == b']') {
                let after = i + 1 + rb + 1;
                if bytes.get(after) == Some(&b'(') {
                    if let Some(close) = bytes[after + 1..].iter().position(|&b| b == b')') {
                        let label = &input[i + 1..i + 1 + rb];
                        let href = &input[after + 1..after + 1 + close];
                        let _ = write!(
                            out,
                            "<a href=\"{}\">{}</a>",
                            escape_html(href),
                            escape_html(label)
                        );
                        i = after + close + 2;
                        continue;
                    }
                }
            }
        }
        // Default: escape character.
        match c {
            b'<' => out.push_str("&lt;"),
            b'>' => out.push_str("&gt;"),
            b'&' => out.push_str("&amp;"),
            b'"' => out.push_str("&quot;"),
            _ => out.push(c as char),
        }
        i += 1;
    }
    out
}

/// "1.5.0" → "1-5-0"
fn anchor(version: &str) -> String {
    version
        .chars()
        .map(|c| if c == '.' || c.is_whitespace() { '-' } else { c })
        .collect()
}

fn first_bullet_summary(r: &Release<'_>) -> String {
    // Combine up to three Added bullets, separated by " · " — caps at ~120 chars.
    let pool: Vec<&&str> = r
        .sections
        .iter()
        .find(|(h, _)| h.eq_ignore_ascii_case("Added"))
        .map(|(_, bullets)| bullets.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    if pool.is_empty() {
        return r.sections.iter()
            .flat_map(|(_, b)| b.iter())
            .next()
            .map(ToString::to_string)
            .unwrap_or_else(|| "see the changelog for details".into());
    }
    let mut out = String::new();
    for b in pool.iter().take(3) {
        if !out.is_empty() { out.push_str(" · "); }
        // Strip leading "[O-NN] " tags if present so the banner stays human.
        let s = strip_lead_tag(b);
        out.push_str(s);
        if out.chars().count() > 120 { break; }
    }
    out
}

fn strip_lead_tag(s: &str) -> &str {
    let trimmed = s.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return rest[end + 1..].trim_start();
        }
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"# Changelog

## [Unreleased]

## [1.5.0] - 2026-04-22

### Added
- Themed confirmation modal
- /analytics/dawn-chorus polar plot
- [O-21] Changelog viewer + post-upgrade banner

### Fixed
- Day-strip now respects station time zone

## [1.4.2] - 2026-04-12

### Added
- Live status pill in the topnav
"#;

    #[test]
    fn parses_releases() {
        let r = parse_changelog(FIXTURE);
        // The "[Unreleased]" entry parses with no date and no sections; it
        // appears in the list even though it has nothing under it yet.
        let real: Vec<&Release> = r.iter().filter(|x| x.version != "Unreleased").collect();
        assert_eq!(real.len(), 2);
        assert_eq!(real[0].version, "1.5.0");
        assert_eq!(real[0].date.as_deref(), Some("2026-04-22"));
        assert_eq!(real[0].sections.len(), 2);
        assert_eq!(real[0].sections[0].0, "Added");
        assert_eq!(real[0].sections[0].1.len(), 3);
        assert_eq!(real[1].version, "1.4.2");
    }

    #[test]
    fn anchor_strips_dots() {
        assert_eq!(anchor("1.5.0"), "1-5-0");
        assert_eq!(anchor("1.4.2"), "1-4-2");
        assert_eq!(anchor("Unreleased"), "Unreleased");
    }

    #[test]
    fn inline_md_handles_code_and_link() {
        let s = render_inline_md("Use `cargo run` then visit [home](/).");
        assert!(s.contains(r#"<code class="mono">cargo run</code>"#));
        assert!(s.contains(r#"<a href="/">home</a>"#));
    }

    #[test]
    fn strip_lead_tag_removes_brackets() {
        assert_eq!(strip_lead_tag("[O-21] Changelog"), "Changelog");
        assert_eq!(strip_lead_tag("Plain bullet"), "Plain bullet");
    }

    #[test]
    fn latest_summary_combines_added_bullets() {
        let r = parse_changelog(FIXTURE);
        let latest = r.iter().find(|x| x.version == "1.5.0").unwrap();
        let s = first_bullet_summary(latest);
        assert!(s.contains("Themed confirmation modal"));
        assert!(s.contains("polar plot"));
        assert!(!s.starts_with("[O-")); // tag stripped
    }
}
