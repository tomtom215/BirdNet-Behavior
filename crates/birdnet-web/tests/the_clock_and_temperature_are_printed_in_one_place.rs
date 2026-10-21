//! Every clock time and temperature the dashboard prints goes through
//! `birdnet_web::display`, so the station's default and each browser's choice
//! (12- or 24-hour, Celsius or Fahrenheit) reach all of them.
//!
//! # What is guarded
//!
//! A mechanical sweep of `src/`, `templates/` and `static/` for the shapes a
//! hand-rolled time or temperature takes: a degree sign after a placeholder or
//! a unit letter, an `HH:MM` built from `{:02}:{:02}`, a clock truncated with
//! `.get(0..5)`, a stored time escaped straight into a page, and in script
//! `toLocaleTimeString`, `getHours` and `°`. A site that prints one must call
//! the formatter instead, or be named below with the reason it is exempt.
//!
//! # The exemptions are a list, not a heuristic
//!
//! Each names a file, a fragment of the line and why a hand-written form is
//! right there. A listed exemption that no longer matches any line fails the
//! test, so the list cannot outlive the code it excuses. The reasons fall in
//! four groups: machine formats (feeds, CSV, JSON, SQL comparisons), text that
//! cannot carry markup (a `title`, a confirmation dialog, an SVG `<title>`),
//! absolute timestamps labelled UTC, and coordinates (a degree sign that is
//! not a temperature).
//!
//! # What the sweep cannot see
//!
//! It finds the shapes above. A time printed by a helper written some other
//! way is not found; the counterpart below plants each shape in a scratch
//! source and checks the sweep flags it, so the patterns are known to match.

use std::path::{Path, PathBuf};

/// `(file suffix, fragment of the line, why)`.
const EXEMPT: &[(&str, &str, &str)] = &[
    // Coordinates: a degree sign that is not a temperature.
    (
        "routes/pages/today.rs",
        "{:.2}°{ns}",
        "coordinates, not a temperature",
    ),
    (
        "routes/pages/occurrence_page.rs",
        "{:.1}°{}, {:.1}°{}",
        "coordinates, not a temperature",
    ),
    (
        "routes/pages/detection_detail.rs",
        "{:.4}°{ns}",
        "coordinates, not a temperature",
    ),
    // Machine formats.
    (
        "routes/charts.rs",
        "{date},{slot},{:02}:{:02},{count}",
        "a CSV export, machine format",
    ),
    (
        "routes/insights.rs",
        "{:02}:{:02}",
        "a JSON field, machine format",
    ),
    (
        "routes/pages/today_phrase.rs",
        "{:02}:{:02}:{:02}",
        "compared with stored times in a query, never shown",
    ),
    (
        "routes/pages/today.rs",
        "last.get(0..5)",
        "the outage query's result, printed through display::clock where it is shown",
    ),
    // Absolute timestamps labelled UTC (admin tables).
    (
        "routes/admin/backup.rs",
        "{hh:02}:{mm:02} UTC",
        "an absolute timestamp, labelled UTC",
    ),
    (
        "routes/auth_pages.rs",
        "{hh:02}:{mm:02}:{ss:02}",
        "a session timestamp in the audit trail, ISO",
    ),
    // Absolute hours labelled UTC, in a JSON API's own error text.
    (
        "routes/weather.rs",
        "{hour:02}:00 UTC",
        "a JSON error message naming a UTC hour of the weather cache; labelled UTC, machine-facing, not the display clock",
    ),
    // The charts that form a clock from seconds after midnight in one helper.
    (
        "routes/pages/viz/bursts.rs",
        "secs / 3600 % 24",
        "the one place a burst's clock is formed; its only caller is a tooltip (data-tip), which is plain text",
    ),
    (
        "routes/pages/viz/funnel.rs",
        "secs / 3600 % 24",
        "the one place a morning's clock is formed; the SVG labels carry display::clock_attr so they follow the reader's format, and the tooltips and the aria description are plain text",
    ),
    // Labels naming the unit as a choice, and text that cannot carry markup.
    (
        "routes/admin/settings/render/system.rs",
        "Fahrenheit (°F)",
        "the settings page names the two units as its choices",
    ),
    (
        "routes/admin/settings/render/system.rs",
        "Celsius (°C)",
        "the settings page names the two units as its choices",
    ),
    (
        "routes/admin/quality.rs",
        "{hour:02}:00 —",
        "a tooltip (title attribute): plain text",
    ),
    (
        "routes/admin/rules.rs",
        "{s:02}:00–{e:02}:59",
        "a configured rule's hour range, echoed as the 24-hour values the rule stores",
    ),
    (
        "routes/pages/dashboard/heatmap_widget.rs",
        "{h:02}:00",
        "a tooltip (title attribute): plain text",
    ),
    (
        "routes/share.rs",
        "ago_phrase(&det.date, &det.time)",
        "the share page: its own document; a relative phrase",
    ),
    (
        "routes/pages/dawn_chorus.rs",
        "{hh:02}:{mm:02}",
        "the one place the chorus hours are formed; callers print it through display::clock, or in an SVG <title> and data attribute, which are plain text",
    ),
    (
        "routes/pages/viz/bars.rs",
        "{h:02}:00",
        "chart hour labels: axis ticks, tooltips and readouts are plain text; the table rows print through display::clock",
    ),
    (
        "routes/pages/viz/heat.rs",
        "{:02}:00",
        "heat-map cell tooltips: plain text",
    ),
    (
        "routes/pages/behavioral.rs",
        "t.get(..5)",
        "trims a sequence step to HH:MM; the table prints it through display::clock",
    ),
    (
        "routes/pages/station_health.rs",
        "t.get(0..5)",
        "prints through display::clock on the same line",
    ),
    (
        "routes/pages/dashboard/partials.rs",
        "get(0..5)",
        "prints through display::clock on the same line",
    ),
    (
        "routes/pages/species_pages.rs",
        "get(0..5)",
        "prints through display::clock on the same line",
    ),
    // A stored time escaped into text on purpose.
    (
        "routes/share.rs",
        "escape_html(&det.time)",
        "the public share page is its own document and loads none of the layout's scripts",
    ),
    (
        "routes/pages/detection_comments.rs",
        "escape_html(time)",
        "an identity carried in a form field, not shown",
    ),
    (
        "routes/pages/detection_reviews.rs",
        "escape_html(time)",
        "an identity carried in a form field, not shown",
    ),
    (
        "routes/pages/detection_reviews.rs",
        "escape_html(&d.time)",
        "the form field; the visible copy is time_clk",
    ),
    (
        "routes/pages/detection_reviews.rs",
        "escape_html(&r.time)",
        "the form field; the visible copy is time_clk",
    ),
    (
        "routes/pages/today.rs",
        "escape_html(&d.time)",
        "plain text of the confirmation dialogs; the visible link is time_clk",
    ),
    (
        "routes/pages/detection_detail.rs",
        "time = escape_html(time)",
        "echoes the requested time back in the not-found page, as the user typed it",
    ),
    (
        "routes/pages/audio_player.rs",
        "escape_html(time)",
        "also the spectrogram's query string; the visible copy is time_clk",
    ),
    (
        "routes/pages/behavioral.rs",
        "escape_html(&s.start_time)",
        "a session start in the behavioural table: a UTC instant, not a station clock time",
    ),
];

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk(&p, ext, out);
        } else if p.extension().is_some_and(|e| e == ext) {
            out.push(p);
        }
    }
}

/// A finding: `(file, 1-based line, why it is suspect, the line)`.
type Finding = (String, usize, &'static str, String);

fn rust_findings(rel: &str, text: &str) -> Vec<Finding> {
    // Test modules sit at the end of a file and assert on rendered text.
    // (`#[cfg(test)]` or `#[cfg(all(test, …))]`.)
    let cut = text
        .lines()
        .scan(0usize, |at, l| {
            let start = *at;
            *at += l.len() + 1;
            Some((start, l))
        })
        .find(|(_, l)| l.trim_start().starts_with("#[cfg(") && l.contains("test"))
        .map(|(start, _)| start);
    let body = cut.map_or(text, |c| &text[..c]);
    let mut out = Vec::new();
    for (i, line) in body.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") {
            continue;
        }
        let mut why = None;
        if line.contains("°C") || line.contains("°F") {
            why = Some("a temperature unit written by hand");
        } else if line.contains("}°") {
            why = Some("a degree sign after a placeholder");
        } else if line.contains(":02}:{") || line.contains(":02}:00") {
            why = Some("an HH:MM built by hand");
        } else if line.contains(".get(0..5)") || line.contains(".get(..5)") {
            why = Some("a clock truncated to HH:MM");
        } else if line.contains("escape_html(") && escaped_time(line) {
            why = Some("a stored time escaped straight into a page");
        }
        if let Some(why) = why {
            out.push((rel.to_owned(), i + 1, why, line.trim().to_owned()));
        }
    }
    out
}

/// `escape_html(` whose argument names a time: `&d.time`, `time`, `&s.start_time`.
fn escaped_time(line: &str) -> bool {
    let mut rest = line;
    while let Some(at) = rest.find("escape_html(") {
        let after = &rest[at + "escape_html(".len()..];
        let arg = after.split(')').next().unwrap_or("");
        let name = arg.trim_start_matches('&').rsplit('.').next().unwrap_or("");
        if name == "time" || name.ends_with("_time") || name == "time_safe" {
            return true;
        }
        rest = after;
    }
    false
}

fn script_findings(rel: &str, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with('*') || t.starts_with("<!--") {
            continue;
        }
        let why = if line.contains("toLocaleTimeString") || line.contains("getHours(") {
            Some("a clock formatted in script")
        } else if line.contains("°C") || line.contains("°F") || line.contains("\\u00b0") {
            Some("a temperature unit written by hand")
        } else {
            None
        };
        if let Some(why) = why {
            out.push((rel.to_owned(), i + 1, why, line.trim().to_owned()));
        }
    }
    out
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn sweep() -> Vec<Finding> {
    let mut found = Vec::new();
    let mut rs = Vec::new();
    walk(&root().join("src"), "rs", &mut rs);
    for p in rs {
        let rel = p
            .strip_prefix(root().join("src"))
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel == "display.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap();
        found.extend(rust_findings(&rel, &text));
    }
    let mut scripts = Vec::new();
    walk(&root().join("static"), "js", &mut scripts);
    walk(&root().join("templates"), "html", &mut scripts);
    for p in scripts {
        let rel = p
            .strip_prefix(root())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        // The formatter's own browser half; and the display card, which names °C
        // and °F as the labels of its two choices.
        if rel == "static/display-format.js" || rel == "templates/_partial_display_prefs.html" {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap();
        found.extend(script_findings(&rel, &text));
    }
    found
}

fn exempt(f: &Finding) -> bool {
    EXEMPT
        .iter()
        .any(|(file, frag, _)| f.0.ends_with(file) && f.3.contains(frag))
}

#[test]
fn no_page_prints_a_time_or_a_temperature_by_hand() {
    let unexplained: Vec<String> = sweep()
        .iter()
        .filter(|f| !exempt(f))
        .map(|(file, line, why, text)| format!("{file}:{line}: {why}: {text}"))
        .collect();
    assert!(
        unexplained.is_empty(),
        "print these through birdnet_web::display (clock, datetime, temperature), or name \
         the exemption with its reason in EXEMPT:\n{}",
        unexplained.join("\n")
    );
}

/// An exemption that matches nothing is an excuse for code that is gone.
#[test]
fn every_exemption_still_excuses_a_line() {
    let found = sweep();
    let dead: Vec<String> = EXEMPT
        .iter()
        .filter(|(file, frag, _)| {
            !found
                .iter()
                .any(|f| f.0.ends_with(file) && f.3.contains(frag))
        })
        .map(|(file, frag, why)| format!("{file}: {frag:?} ({why})"))
        .collect();
    assert!(
        dead.is_empty(),
        "exemptions with nothing to excuse:\n{}",
        dead.join("\n")
    );
}

/// The counterpart: each shape the sweep looks for is flagged when it is
/// planted, so a green run above is not a sweep that matches nothing.
#[test]
#[allow(
    clippy::literal_string_with_formatting_args,
    reason = "the strings are Rust source the sweep reads, planted to be flagged or not; their braces are the point and nothing formats them"
)]
fn the_sweep_flags_each_shape_it_claims_to_find() {
    let planted = [
        r#"let a = format!("{t:.0}°C");"#,
        r#"let a = format!("{now:.0}°");"#,
        r#"let a = format!("{h:02}:{m:02}");"#,
        r#"let a = format!("{:02}:00", h);"#,
        r"let a = t.get(0..5);",
        r"let a = escape_html(&d.time);",
        r"let a = escape_html(&row.start_time);",
        r"let a = escape_html(time);",
    ];
    for line in planted {
        let hits = rust_findings("planted.rs", line);
        assert_eq!(hits.len(), 1, "not flagged: {line}");
    }
    for line in [
        "el.textContent = d.time.toLocaleTimeString();",
        "var h = new Date().getHours();",
        r#"label.textContent = t + "°C";"#,
    ] {
        assert_eq!(
            script_findings("planted.js", line).len(),
            1,
            "not flagged: {line}"
        );
    }
    // And what is not a time is left alone.
    for line in [
        r"let a = escape_html(&d.com_name);",
        r"let a = escape_html(&d.date);",
        r#"let a = format!("{pct:.0}%");"#,
        r"let a = escape_html(timeout);",
        "// a comment about °C and {h:02}:{m:02}",
    ] {
        assert!(
            rust_findings("planted.rs", line).is_empty(),
            "wrongly flagged: {line}"
        );
    }
}
