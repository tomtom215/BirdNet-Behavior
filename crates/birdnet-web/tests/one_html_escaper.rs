//! There is one HTML escaper, and this is what keeps it one.
//!
//! # Why a source scan rather than a behavioural test
//!
//! `routes::pages::escape_html` carries a comment saying three copies were
//! consolidated into it, because they had drifted: it escapes `& < > " '`, the
//! others escaped `& < > "` and omitted the apostrophe. Neither of those two
//! happened to interpolate into a single-quoted attribute, so the difference was
//! latent rather than exploitable.
//!
//! Two of the three were still there. The consolidation removed one copy, the
//! comment described the job as finished, and
//! `escape_html_covers_every_character_that_can_break_out` — a good test — went
//! on passing, because a test of the canonical function cannot see a second
//! function. That is the specific hole this closes.
//!
//! # What "one" means here
//!
//! A crate-wide scan for a function whose body looks like HTML escaping:
//! something that replaces `<` with `&lt;`. Exactly one such function may exist
//! in production code, and it must be the one in `routes/pages/mod.rs`. Test
//! modules are stripped first, so a fixture that hand-builds an expected string
//! does not trip it.
//!
//! If a second escaper is ever genuinely warranted — a different context needs
//! different rules, say — this test is the right place to say so, by name, with
//! the reason. An allowlist entry is a decision; a silent second copy is not.

use std::path::{Path, PathBuf};

/// The one HTML escaper, as `file:function`.
const CANONICAL: (&str, &str) = ("src/routes/pages/mod.rs", "escape_html");

/// Escapers that are deliberately separate, each with the reason.
///
/// An entry here is a decision. A silent second copy is not — which is the
/// distinction this file exists to enforce, so the list has to stay short and
/// each line has to say why the canonical escaper will not do.
const ALLOWED: &[(&str, &str, &str)] = &[(
    "src/routes/feeds.rs",
    "escape_xml",
    "RSS and iCal are XML, not HTML. XML wants `&apos;` for the apostrophe; \
     HTML does not have that entity before HTML5, so `escape_html` emits \
     `&#x27;` instead. Feeding HTML escaping to an XML parser is not a \
     correctness problem, but the reverse is, and keeping them apart is what \
     stops someone \"consolidating\" the wrong direction later.",
)];

/// Every `.rs` file in this crate's `src/`.
fn sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Everything in a file that is not inside a `#[cfg(test)]` module.
fn non_test_source(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out = String::new();
    let mut in_test = false;
    let mut depth: i32 = 0;
    let mut opened = false;
    for line in text.lines() {
        if !in_test && line.trim_start().starts_with("#[cfg(test)]") {
            in_test = true;
            depth = 0;
            opened = false;
            continue;
        }
        if in_test {
            depth += i32::try_from(line.matches('{').count()).unwrap_or(0);
            depth -= i32::try_from(line.matches('}').count()).unwrap_or(0);
            if line.contains('{') {
                opened = true;
            }
            if opened && depth <= 0 {
                in_test = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Every `fn` in `text` whose body escapes `<` into `&lt;`, as
/// `(function name, the line it starts on)`.
fn escaper_fns(text: &str) -> Vec<(String, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(rest) = line.split_once("fn ").map(|(_, r)| r) else {
            continue;
        };
        let Some(name) = rest.split(['(', '<', ' ']).next() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        // An escaper maps a literal `<` onto `&lt;`. Both halves have to be
        // present, and `<` has to appear as a *literal* — `'<'` or `"<"` —
        // rather than as a generic bracket or a comparison.
        //
        // The looser first version of this check (`window.contains('<')`)
        // flagged `render_low_confidence_species`, whose markup contains the
        // text "avg confidence &lt;60%". A gate that cries wolf gets an
        // allowlist entry per false positive until it means nothing.
        // The window stops at the next `fn`, not after a fixed 12 lines. It used
        // to run on regardless, so a short function sitting immediately above
        // `escape_html` was reported as an escaper because the window reached
        // into `escape_html`'s body. That is a false positive produced by
        // *adjacency*, which is the worst kind: it accuses whichever function
        // happens to be written above the real one.
        let end = lines[i + 1..]
            .iter()
            .position(|l| l.contains("fn "))
            .map_or(lines.len(), |off| i + 1 + off)
            .min(lines.len().min(i + 12));
        let window = lines[i..end.max(i + 1)].join("\n");
        let names_a_literal_lt = window.contains("'<'") || window.contains("\"<\"");
        if window.contains("&lt;") && names_a_literal_lt {
            found.push(((*name).to_string(), i + 1));
        }
    }
    found
}

#[test]
fn exactly_one_function_in_this_crate_escapes_html() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut all: Vec<(String, String, usize)> = Vec::new();
    for path in sources() {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        for (name, line) in escaper_fns(&non_test_source(&path)) {
            all.push((rel.clone(), name, line));
        }
    }

    // Every allowlisted escaper must still exist; one that has been removed or
    // renamed should lose its entry rather than sit here excusing nothing.
    for (file, name, _) in ALLOWED {
        assert!(
            all.iter().any(|(f, n, _)| f == file && n == name),
            "`{name}` in {file} is allowlisted but no longer exists — delete the \
             entry rather than leaving a stale exemption"
        );
    }

    let unexplained: Vec<_> = all
        .iter()
        .filter(|(f, n, _)| !ALLOWED.iter().any(|(af, an, _)| af == f && an == n))
        .collect();

    assert_eq!(
        unexplained.len(),
        1,
        "this crate must have exactly one HTML escaper, found {}: {unexplained:#?}\n\
         Escaping is not a place to have two answers — the copies drift, and the \
         one that omits the apostrophe is fine right up until someone writes \
         attr='{{}}'. Route the new call site through \
         `routes::pages::escape_html`, or add it to ALLOWED with the reason it \
         cannot be.",
        unexplained.len()
    );

    let (file, name, _) = unexplained[0];
    assert_eq!(
        (file.as_str(), name.as_str()),
        CANONICAL,
        "the surviving escaper must be the canonical one"
    );
}

/// The counterpart, so the gate above is a discrimination and not a spelling
/// check: the detector must actually recognise the shape it is looking for, on
/// both the form the canonical escaper uses and the form the two removed copies
/// used. Without this, a rename in `escape_html` could make the scan find zero
/// escapers and the assertion above would fail loudly — but a *weakened*
/// detector that finds nothing anywhere would fail too, for the wrong reason,
/// and this is what tells the two apart.
#[test]
fn the_detector_recognises_an_escaper_when_it_sees_one() {
    let chained = r#"
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
"#;
    assert_eq!(
        escaper_fns(chained).len(),
        1,
        "the chained-replace form — what both removed copies looked like"
    );

    let matched = r#"
fn esc(s: &str) -> String {
    s.chars().map(|c| match c {
        '<' => "&lt;".to_string(),
        _ => c.to_string(),
    }).collect()
}
"#;
    assert_eq!(
        escaper_fns(matched).len(),
        1,
        "a match-arm escaper is still an escaper"
    );

    let innocent = r#"
fn render_row(name: &str) -> String {
    format!("<td>{name}</td>")
}
"#;
    assert!(
        escaper_fns(innocent).is_empty(),
        "ordinary markup building must not be mistaken for an escaper"
    );

    // The real false positive this detector was tightened for: markup whose
    // *text* contains an entity, with no literal `<` being replaced.
    let entity_in_prose = r#"
fn render_low_confidence_species(low: &[String]) -> String {
    if low.is_empty() {
        return r"<p>No species with avg confidence &lt;60%. Looks good!</p>".to_string();
    }
    String::new()
}
"#;
    assert!(
        escaper_fns(entity_in_prose).is_empty(),
        "an entity inside prose is not an escaper — this exact function was \
         flagged by the first version of this check"
    );
}

/// And the test modules really are stripped, or a fixture asserting
/// `escape_html("<x>") == "&lt;x&gt;"` would count as a second escaper and this
/// whole file would be noise.
#[test]
fn test_modules_are_not_scanned() {
    let with_tests = r#"
fn render(s: &str) -> String { s.to_string() }

#[cfg(test)]
mod tests {
    fn expected_escape(s: &str) -> String {
        s.replace('<', "&lt;")
    }
}
"#;
    assert!(
        escaper_fns(&{
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join("x.rs");
            std::fs::write(&p, with_tests).unwrap();
            non_test_source(&p)
        })
        .is_empty(),
        "a helper inside `#[cfg(test)]` is not a production escaper"
    );
}
