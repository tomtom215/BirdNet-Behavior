//! `--dawn` is a fill, not a text colour — and the gate that would catch it
//! cannot see most of where it is used.
//!
//! In light mode `--dawn` is `oklch(68% 0.12 60)` = `#ce8545`, which measures
//! **2.85:1** against `--bg` and **2.97:1** against `--surface`: below the
//! 4.5:1 WCAG AA asks of body text, and below even the 3:1 large-text floor.
//! `--warning` exists for exactly this and resolves per theme —
//! `--dawn-ink` (8.4:1) in light, `--dawn` (10.5:1) in dark.
//!
//! Observed failing against the shipped tree: `admin/system.rs` styled
//! `.badge-warn` and `.meter-val.warn` with `color:var(--dawn)`, and
//! `migration/render.rs` and `logs.rs` did the same at four more sites.
//! `tools/visual-qa/axe.mjs` did not catch any of them for two compounding
//! reasons — `.badge-warn` only renders when the station actually has
//! something to warn about, and `/admin/logs` and the migration pages are not
//! in the sweep's route table at all. The one that did surface, surfaced only
//! after the fixture's disk crossed a threshold mid-session.
//!
//! This gate is mechanical and needs no browser, so it covers every stylesheet
//! in the crate including the ones embedded in Rust string literals.

use std::path::{Path, PathBuf};

/// Every file that can carry CSS: the stylesheets plus the Rust sources that
/// embed `<style>` blocks.
fn css_bearing_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = vec![
        root.join("static/css/app.css"),
        root.join("static/css/print.css"),
    ];
    let mut stack = vec![root.join("src"), root.join("templates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("rs" | "html" | "css")
            ) {
                out.push(path);
            }
        }
    }
    out
}

#[test]
fn amber_is_never_used_as_a_text_colour() {
    let mut offenders = Vec::new();
    for file in css_bearing_files() {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            // Normalise `color : var( --dawn )` spacing before matching, and
            // skip `--dawn-ink` / `--dawn-soft`, which are the correct tokens.
            let flat: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            for key in ["color:var(--dawn)", "fill:var(--dawn)"] {
                let Some(at) = flat.find(key) else { continue };
                // `border-color:` and `background-color:` are fills, not text.
                let before = &flat[..at];
                if before.ends_with("border-") || before.ends_with("background-") {
                    continue;
                }
                offenders.push(format!(
                    "{}:{}  {}",
                    file.file_name().unwrap_or_default().to_string_lossy(),
                    n + 1,
                    line.trim()
                ));
            }
        }
    }
    offenders.sort();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "`--dawn` measures 2.85:1 on --bg in light mode, so it fails WCAG AA as \
         a text or glyph colour. Use `--warning`, which is --dawn-ink in light \
         and --dawn in dark:\n  {}",
        offenders.join("\n  ")
    );
}

/// The counterpart: `--warning` must keep meaning two different colours, or
/// the fix above is a rename with no effect.
#[test]
fn the_warning_token_is_theme_dependent() {
    let css =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("static/css/app.css"))
            .expect("app.css is readable");
    let defs: Vec<&str> = css
        .lines()
        .filter(|l| l.trim_start().starts_with("--warning:"))
        .map(str::trim)
        .collect();
    assert_eq!(
        defs.len(),
        2,
        "expected a light and a dark definition of --warning, found {defs:?}"
    );
    assert_ne!(
        defs[0], defs[1],
        "--warning resolves to the same colour in both themes, so swapping \
         --dawn for it changes nothing: {defs:?}"
    );
}
