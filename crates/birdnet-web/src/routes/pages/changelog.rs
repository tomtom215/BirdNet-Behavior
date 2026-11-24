//! Changelog viewer + post-upgrade banner. See O-21 DIFF.md.
//!
//! `CHANGELOG.md` is embedded at build time so the page works offline. It is read
//! through `crates/birdnet-web/CHANGELOG.md`, a symlink to the repository's file:
//! an `include_str!` that leaves the crate directory compiles in a checkout and
//! fails in the package `cargo publish` uploads (found by `cargo package`). The
//! parser only knows the "Keep a Changelog" shape we already use — if the
//! file ever drifts, the page renders the raw markdown in a `<pre>` block.

use std::borrow::Cow;
use std::fmt::Write as _;

use axum::Router;
use axum::http::HeaderMap;
use axum::response::Html;
use axum::routing::get;

use super::{escape_html, render_page_for_request};
use crate::state::AppState;

// Embedded at build time relative to crates/birdnet-web/src/routes/pages/.
const CHANGELOG_MD: &str = include_str!("../../../CHANGELOG.md");

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/system/changelog", get(page))
        .route("/system/changelog/latest", get(latest_partial))
}

// ---------------------------------------------------------------------------
// Public handlers
// ---------------------------------------------------------------------------

async fn page(headers: HeaderMap) -> Html<String> {
    let releases = parse_changelog(CHANGELOG_MD);
    let mut body = String::from(
        r#"<div class="page-head cl-head" data-screen-label="Changelog" data-om-validate>
  <div>
    <div class="bnb-eyebrow">System · release history</div>
    <h1 class="display cl-h1">
      What's <em>changed</em>.
    </h1>
    <p class="bnb-meta cl-lede">
      Every release of BirdNet-Behavior. Click a version to permalink it.
    </p>
  </div>
</div>
<div class="bnb-card pad cl-card">
  <div class="bnb-help-drawer__body cl-body">"#,
    );

    if releases.is_empty() {
        body.push_str(r#"<pre class="cl-pre">"#);
        body.push_str(&escape_html(CHANGELOG_MD));
        body.push_str("</pre>");
    } else {
        for r in &releases {
            render_release(&mut body, r);
        }
    }
    body.push_str("</div></div>");
    render_page_for_request("Changelog", &body, "station", &headers)
}

async fn latest_partial() -> Html<String> {
    let releases = parse_changelog(CHANGELOG_MD);
    // Show the latest *released* version — never the in-progress "[Unreleased]"
    // section at the top of the changelog. Using `releases.first()` rendered
    // the banner as "New in vUnreleased." whenever the changelog had pending
    // notes (which it almost always does). Skip any "Unreleased" entry and fall
    // back to no banner when there is no released version yet.
    let Some(latest) = releases
        .iter()
        .find(|r| !r.version.eq_ignore_ascii_case("Unreleased"))
    else {
        return Html(String::new());
    };
    // Returns the banner subject + body — short summary plus link.
    let summary = first_bullet_summary(latest);
    // The "#v{anchor}" URL fragment puts `"` immediately adjacent to `#`,
    // which would close an `r#"…"#` raw string early. Use `r##"…"##`.
    // Clippy's needless_raw_string_hashes hint doesn't account for this.
    #[allow(clippy::needless_raw_string_hashes)]
    let banner = format!(
        r##"<div class="bnb-banner" data-version="{ver}">
  <span class="bnb-pill moss" aria-hidden="true">v{ver}</span>
  <div class="bnb-banner__copy">
    <strong>New in v{ver}.</strong>
    <span>{summary}</span>
  </div>
  <a class="bnb-btn ghost" href="/system/changelog#v{anchor}">See all changes →</a>
  <button type="button" class="bnb-banner__close" data-banner-dismiss aria-label="Dismiss">&times;</button>
</div>"##,
        ver = escape_html(&latest.version),
        anchor = escape_html(&anchor(&latest.version)),
        summary = escape_html(&summary),
    );
    Html(banner)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Release<'a> {
    pub version: String,
    pub date: Option<String>,
    /// Each bullet whole: `CHANGELOG.md` wraps a bullet over several indented
    /// lines, which are joined here with single spaces (UIR-07).
    pub sections: Vec<(String, Vec<Cow<'a, str>>)>,
}

#[must_use]
pub fn parse_changelog(input: &str) -> Vec<Release<'_>> {
    let mut out: Vec<Release<'_>> = Vec::new();
    let mut current: Option<Release<'_>> = None;
    let mut current_section: Option<(String, Vec<Cow<'_, str>>)> = None;
    // Whether the next indented line continues the last bullet: a blank line
    // or a heading ends a bullet.
    let mut in_bullet = false;

    for raw in input.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            in_bullet = false;
            continue;
        }

        if let Some((ver, date)) = parse_release_heading(line) {
            if let Some(sec) = current_section.take()
                && let Some(r) = current.as_mut()
            {
                r.sections.push(sec);
            }
            if let Some(prev) = current.take() {
                out.push(prev);
            }
            current = Some(Release {
                version: ver,
                date,
                sections: Vec::new(),
            });
            in_bullet = false;
            continue;
        }

        if let Some(section_name) = parse_section_heading(line) {
            if let Some(sec) = current_section.take()
                && let Some(r) = current.as_mut()
            {
                r.sections.push(sec);
            }
            current_section = Some((section_name, Vec::new()));
            in_bullet = false;
            continue;
        }

        let Some((_, bullets)) = current_section.as_mut() else {
            continue;
        };
        if let Some(bullet) = parse_bullet(raw) {
            bullets.push(Cow::Borrowed(bullet));
            in_bullet = true;
        } else if in_bullet
            && raw.starts_with(char::is_whitespace)
            && let Some(last) = bullets.last_mut()
        {
            let joined = last.to_mut();
            joined.push(' ');
            joined.push_str(line.trim_start());
        }
    }

    if let Some(sec) = current_section.take()
        && let Some(r) = current.as_mut()
    {
        r.sections.push(sec);
    }
    if let Some(prev) = current.take() {
        out.push(prev);
    }
    out
}

/// `## [1.5.0] - 2026-04-22` or `## [Unreleased]` → ("1.5.0", Some("2026-04-22"))
fn parse_release_heading(line: &str) -> Option<(String, Option<String>)> {
    let s = line.trim();
    if !s.starts_with("## ") {
        return None;
    }
    let s = s.strip_prefix("## ")?.trim();
    let s = s.strip_prefix('[')?;
    let close = s.find(']')?;
    let ver = s[..close].trim().to_string();
    if ver.eq_ignore_ascii_case("Unreleased") {
        return Some((ver, None));
    }
    let rest = s[close + 1..].trim();
    let date = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("– "))
        .map(|s| s.trim().to_string());
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
    trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .map(str::trim_end)
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

// The "#v{anchor}" URL fragment needs the `r##"…"##` raw-string form;
// clippy's needless_raw_string_hashes hint is incorrect for this case.
#[allow(clippy::needless_raw_string_hashes)]
fn render_release(out: &mut String, r: &Release<'_>) {
    let anchor = anchor(&r.version);
    let date = r.date.as_deref().unwrap_or("(unreleased)");
    let _ = write!(
        out,
        r##"<article id="v{anchor}" class="changelog-release">
  <header class="cl-rel-head">
    <h2 class="cl-rel-h2">
      <a href="#v{anchor}" class="cl-rel-link">v{ver}</a>
    </h2>
    <span class="bnb-meta mono">{date}</span>
  </header>"##,
        anchor = escape_html(&anchor),
        ver = escape_html(&r.version),
        date = escape_html(date),
    );
    for (heading, bullets) in &r.sections {
        let _ = write!(
            out,
            r#"<h3 class="cl-sec-h3">{}</h3><ul>"#,
            escape_html(heading)
        );
        for b in bullets {
            let _ = write!(out, "<li>{}</li>", render_inline_md(b));
        }
        out.push_str("</ul>");
    }
    out.push_str("</article>");
}

/// Tiny inline-md renderer: backticks → `<code>`, `[label](href)` → `<a>`,
/// `**strong**` and `*emphasis*`, escape the rest. Used for changelog bullets —
/// does **not** support all of `CommonMark`.
fn render_inline_md(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'*'
            && let Some((tag, inner, next)) = emphasis_at(input, i)
        {
            let _ = write!(out, "<{tag}>{}</{tag}>", render_inline_md(inner));
            i = next;
            continue;
        }
        if c == b'`' {
            // find closing backtick
            if let Some(end) = bytes[i + 1..].iter().position(|&b| b == b'`') {
                let code = &input[i + 1..i + 1 + end];
                let _ = write!(out, "<code class=\"mono\">{}</code>", escape_html(code));
                i += end + 2;
                continue;
            }
        } else if c == b'['
            && let Some(rb) = bytes[i + 1..].iter().position(|&b| b == b']')
        {
            let after = i + 1 + rb + 1;
            if bytes.get(after) == Some(&b'(')
                && let Some(close) = bytes[after + 1..].iter().position(|&b| b == b')')
            {
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
        // Default: escape one character. A character, not a byte: pushing
        // `bytes[i] as char` turned every letter outside ASCII into two or
        // three wrong ones (UIR-07).
        let ch = input[i..].chars().next().unwrap_or('\u{fffd}');
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
        i += ch.len_utf8().max(1);
    }
    out
}

/// A `**strong**` or `*emphasis*` span starting at byte `i` (an asterisk):
/// the tag, the text inside, and the byte after the closing marker. `None`
/// when the asterisk opens nothing — no closing marker, nothing between them,
/// or white space just inside one, as in `a * b`.
fn emphasis_at(input: &str, i: usize) -> Option<(&'static str, &str, usize)> {
    let (tag, marker) = if input[i..].starts_with("**") {
        ("strong", "**")
    } else {
        ("em", "*")
    };
    let open = i + marker.len();
    let close = open + input[open..].find(marker)?;
    let inner = &input[open..close];
    let padded = inner.starts_with(char::is_whitespace) || inner.ends_with(char::is_whitespace);
    if inner.is_empty() || padded {
        return None;
    }
    Some((tag, inner, close + marker.len()))
}

/// "1.5.0" → "1-5-0"
fn anchor(version: &str) -> String {
    version
        .chars()
        .map(|c| {
            if c == '.' || c.is_whitespace() {
                '-'
            } else {
                c
            }
        })
        .collect()
}

fn first_bullet_summary(r: &Release<'_>) -> String {
    // Combine up to three Added bullets, separated by " · " — caps at ~120 chars.
    let pool: Vec<&Cow<'_, str>> = r
        .sections
        .iter()
        .find(|(h, _)| h.eq_ignore_ascii_case("Added"))
        .map_or_default(|(_, bullets)| bullets.iter().collect::<Vec<_>>());
    if pool.is_empty() {
        return r
            .sections
            .iter()
            .flat_map(|(_, b)| b.iter())
            .next()
            .map_or_else(
                || "see the changelog for details".into(),
                ToString::to_string,
            );
    }
    let mut out = String::new();
    for b in pool.iter().take(3) {
        if !out.is_empty() {
            out.push_str(" · ");
        }
        // Strip leading "[O-NN] " tags if present so the banner stays human.
        let s = strip_lead_tag(b);
        out.push_str(s);
        if out.chars().count() > 120 {
            break;
        }
    }
    banner_text(&out)
}

/// The banner's summary as plain text: the emphasis markers dropped (the
/// banner escapes what it is given), and cut at a word near 140 characters,
/// now that a bullet is its whole paragraph rather than its first line.
fn banner_text(summary: &str) -> String {
    const MAX: usize = 140;
    let plain: String = summary.chars().filter(|&c| c != '*').collect();
    if plain.chars().count() <= MAX {
        return plain;
    }
    let cut: String = plain.chars().take(MAX).collect();
    let at_word = cut.rfind(' ').map_or(cut.as_str(), |sp| &cut[..sp]);
    format!("{}…", at_word.trim_end_matches([',', ';', ':', ' ']))
}

fn strip_lead_tag(s: &str) -> &str {
    let trimmed = s.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        return rest[end + 1..].trim_start();
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r"# Changelog

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
";

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

    /// UIR-07: every bullet in `CHANGELOG.md` wraps at about 110 columns, and
    /// the page read one line per bullet, so each entry stopped mid-sentence;
    /// `**bold**` printed as asterisks.
    #[test]
    fn a_wrapped_bold_bullet_renders_whole_and_bold() {
        let md = "## [9.9.9] - 2026-11-24\n\n### Added\n\
                  - **Which classifiers heard it.** A detection's page shows a\n  \
                  *Who heard it* card listing every model.\n\
                  - Second bullet.\n";
        let releases = parse_changelog(md);
        let mut html = String::new();
        render_release(&mut html, &releases[0]);
        assert!(
            html.contains("<strong>Which classifiers heard it.</strong>"),
            "bold must render: {html}"
        );
        assert!(
            html.contains("<em>Who heard it</em> card listing every model."),
            "the continuation line must stay in its bullet: {html}"
        );
        assert!(!html.contains("**"), "{html}");
        assert_eq!(html.matches("<li>").count(), 2, "{html}");
    }

    /// The same renderer pushed each UTF-8 *byte* as a `char`, so any letter
    /// outside ASCII came out as two or three wrong ones ("é" as "Ã©").
    #[test]
    fn non_ascii_text_survives_rendering() {
        assert_eq!(
            render_inline_md("Perch’s 14 795 classes — café"),
            "Perch’s 14 795 classes — café"
        );
    }

    /// Counterparts: a lone asterisk and `a * b` are not emphasis, and code
    /// keeps its asterisks.
    #[test]
    fn an_unpaired_asterisk_is_text() {
        assert_eq!(render_inline_md("a * b"), "a * b");
        assert_eq!(render_inline_md("5*"), "5*");
        assert!(render_inline_md("`a**b**`").contains("<code class=\"mono\">a**b**</code>"));
    }
}
