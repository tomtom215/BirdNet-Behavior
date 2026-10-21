//! HTML for the settings import: the form, the preview, the result.
//!
//! Every verdict is a word as well as a colour, and a credential is never
//! printed (the plan holds it hidden; see `birdnet_migrate::conf`).

use std::fmt::Write as _;

use birdnet_migrate::conf::lists::ListKind;
use birdnet_migrate::conf::{Plan, Row, Verdict};

use super::conf_import::{Applied, ListPlan, Prepared, tally};
use crate::routes::pages::escape_html;

/// The card on the Data tab that starts the settings import.
#[must_use]
pub(super) fn form() -> String {
    String::from(
        r##"<div class="card" id="conf-import">
    <div class="card-head">Bring your BirdNET-Pi settings</div>
    <p class="hint mb-sm">
      Your position, thresholds, species lists, notification rules and disk policy live in
      a few text files in BirdNET-Pi's folder. This reads them and shows you what each
      setting becomes here before anything is changed. Nothing is written until you press
      Apply, and your BirdNET-Pi installation is not touched.
    </p>
    <form id="conf-form" hx-post="/admin/migrate/conf/preview" hx-encoding="multipart/form-data"
          hx-target="#conf-import-result" hx-swap="innerHTML" hx-indicator="#conf-spinner">
      <label for="conf-files">Choose the files</label>
      <input type="file" id="conf-files" name="conf_files" multiple class="mb-sm"
             aria-describedby="conf-files-hint">
      <p class="hint" id="conf-files-hint">
        Select <code>birdnet.conf</code> and, if you have them, <code>apprise.txt</code>,
        <code>body.txt</code>, <code>include_species_list.txt</code>,
        <code>exclude_species_list.txt</code> and <code>whitelist_species_list.txt</code>.
        They are in the BirdNET-Pi folder (<code>~/BirdNET-Pi</code>); choose several at once
        with Ctrl or Shift. The file is recognised by its name.
      </p>
      <label for="conf-text">Or paste the contents of birdnet.conf</label>
      <textarea id="conf-text" name="conf_text" rows="4" spellcheck="false" autocomplete="off"
                placeholder="LATITUDE=52.0&#10;LONGITUDE=5.0&#10;CONFIDENCE=0.7&#10;…"></textarea>
      <label for="conf-dir" class="mt-sm">Or read them from this device</label>
      <input type="text" id="conf-dir" name="source_dir" placeholder="/home/pi/BirdNET-Pi"
             autocomplete="off" spellcheck="false">
      <p class="hint">A folder on the machine this station runs on. Only those file names are read.</p>
      <div class="actions center">
        <button type="submit" class="btn btn-primary">Show me what would change</button>
        <span id="conf-spinner" class="htmx-indicator spinner-note">Reading…</span>
      </div>
    </form>
    <div id="conf-import-result" class="mt" aria-live="polite"></div>
  </div>"##,
    )
}

/// An error in the result area.
#[must_use]
pub(super) fn error(message: &str) -> String {
    format!(
        r#"<div class="card result-card err" role="alert"><div class="result-title err">{}</div></div>"#,
        escape_html(message)
    )
}

fn pill(verdict: Verdict) -> String {
    let class = match verdict {
        Verdict::Imported | Verdict::Converted => "moss",
        Verdict::Attention => "dawn",
        Verdict::ConfigFile => "info",
        Verdict::Refused => "rare",
        Verdict::NotCarried | Verdict::Left => "",
    };
    format!(
        r#"<span class="bnb-pill {class}">{}</span>"#,
        verdict.label()
    )
}

const fn order(v: Verdict) -> u8 {
    match v {
        Verdict::Refused => 0,
        Verdict::Attention => 1,
        Verdict::Converted => 2,
        Verdict::Imported => 3,
        Verdict::ConfigFile => 4,
        Verdict::NotCarried => 5,
        Verdict::Left => 6,
    }
}

/// A key or setting name, escaped, with a place to break after each underscore:
/// the column is narrow, and a name that is cut mid-word reads as two names.
fn breakable(name: &str) -> String {
    escape_html(name).replace('_', "_<wbr>")
}

/// The table's sentences are written once, with `code` and **bold**, for the
/// guide as well as this page. Here they are escaped first and then given the
/// two marks, so no text from the file can become markup.
fn inline_markup(text: &str) -> String {
    let escaped = escape_html(text);
    let mut out = String::with_capacity(escaped.len() + 16);
    let (mut code, mut strong) = (false, false);
    let mut rest = escaped.as_str();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("**") {
            out.push_str(if strong { "</strong>" } else { "<strong>" });
            strong = !strong;
            rest = after;
        } else if let Some(after) = rest.strip_prefix('`') {
            out.push_str(if code { "</code>" } else { "<code>" });
            code = !code;
            rest = after;
        } else {
            let mut chars = rest.chars();
            if let Some(c) = chars.next() {
                out.push(c);
            }
            rest = chars.as_str();
        }
    }
    if code {
        out.push_str("</code>");
    }
    if strong {
        out.push_str("</strong>");
    }
    out
}

fn row_html(r: &Row) -> String {
    let mut becomes = String::new();
    for (setting, value) in &r.becomes {
        let _ = write!(
            becomes,
            "<div><code>{}</code> = {}</div>",
            breakable(setting),
            if value.is_empty() {
                "<em>blank</em>".to_owned()
            } else {
                escape_html(value)
            }
        );
    }
    if let Some(was) = &r.was
        && !was.is_empty()
    {
        let _ = write!(
            becomes,
            r#"<div class="muted">now: {}</div>"#,
            escape_html(was)
        );
    }
    if becomes.is_empty() {
        becomes.push_str(r#"<span class="muted">&mdash;</span>"#);
    }
    format!(
        "<tr><th scope=\"row\"><code>{key}</code></th><td data-label=\"Your value\">{value}</td><td data-label=\"Becomes here\">{becomes}</td><td data-label=\"What happens\">{pill}<div class=\"hint\">{why}</div></td></tr>",
        key = breakable(&r.pi_key),
        value = if r.shown.is_empty() {
            r#"<span class="muted">blank</span>"#.to_owned()
        } else {
            escape_html(&r.shown)
        },
        pill = pill(r.verdict),
        why = inline_markup(&r.why),
    )
}

fn table(rows: &[&Row]) -> String {
    let mut out = String::from(
        r#"<div class="viz-table-scroll"><table class="preview-table conf-table" aria-label="What each BirdNET-Pi setting becomes">
<thead><tr><th scope="col">BirdNET-Pi key</th><th scope="col">Your value</th><th scope="col">Becomes here</th><th scope="col">What happens</th></tr></thead><tbody>"#,
    );
    for r in rows {
        out.push_str(&row_html(r));
    }
    out.push_str("</tbody></table></div>");
    out
}

fn summary(plan: &Plan) -> String {
    let parts: Vec<String> = tally(plan)
        .into_iter()
        .map(|(v, n)| format!("{n} {}", v.label().to_ascii_lowercase()))
        .collect();
    format!(
        "{} keys read: {}.",
        plan.rows
            .iter()
            .filter(|r| r.pi_key != "(no BirdNET-Pi key)")
            .count(),
        parts.join(", ")
    )
}

fn list_html(l: &ListPlan) -> String {
    let name = l.kind.file_name();
    let mut s = format!(
        "<li><strong><code>{}</code></strong> &mdash; {}: {} species",
        escape_html(name),
        l.kind.meaning(),
        l.file.species.len()
    );
    if l.added == 0 {
        if l.file.species.is_empty() {
            s.push_str(". There is nothing in it to add");
        } else {
            s.push_str(". All of them are in this station's list already; nothing is added");
        }
    } else {
        let _ = write!(
            s,
            ". {} of them {} new and will be added to this station's list, which has {} already. \
             Nothing there is removed",
            l.added,
            if l.added == 1 { "is" } else { "are" },
            l.now.len()
        );
    }
    if l.kind == ListKind::Include && l.added > 0 {
        s.push_str(
            ". <strong>Once this list has any species the station records only those</strong>, \
             as BirdNET-Pi did",
        );
    }
    if !l.file.unusable.is_empty() {
        let _ = write!(
            s,
            ". {} {} not {} and matched nothing in BirdNET-Pi either; {} skipped",
            l.file.unusable.len(),
            if l.file.unusable.len() == 1 {
                "line is"
            } else {
                "lines are"
            },
            if l.file.unusable.len() == 1 {
                "a scientific name"
            } else {
                "scientific names"
            },
            if l.file.unusable.len() == 1 {
                "it is"
            } else {
                "they are"
            }
        );
    }
    if l.file.blank_lines > 0 && l.kind == ListKind::Include {
        s.push_str(
            ". It has a blank line, which BirdNET-Pi read as a species that matches nothing",
        );
    }
    s.push_str(".</li>");
    s
}

/// The preview, with its Apply button.
#[must_use]
pub(super) fn preview(p: &Prepared, token: u64) -> String {
    let mut out = String::from(r#"<div class="card result-card ok">"#);
    out.push_str(
        r#"<div class="result-title ok">What importing these files would do</div>
<p class="hint"><strong>Nothing has been changed yet.</strong> Read the table, then press Apply. Credentials are never shown.</p>"#,
    );

    if let Some(plan) = &p.plan {
        let _ = write!(out, "<p>{}</p>", escape_html(&summary(plan)));
        if !plan.repeated_keys.is_empty() {
            let _ = write!(
                out,
                r#"<p class="hint">These keys are in the file more than once; the last line of each is used: {}.</p>"#,
                escape_html(&plan.repeated_keys.join(", "))
            );
        }
        let mut rows: Vec<&Row> = plan.rows.iter().collect();
        rows.sort_by_key(|r| order(r.verdict));
        let (shown, folded): (Vec<&Row>, Vec<&Row>) = rows
            .into_iter()
            .partition(|r| !matches!(r.verdict, Verdict::NotCarried | Verdict::Left));
        if !shown.is_empty() {
            out.push_str(&table(&shown));
        }
        if !folded.is_empty() {
            let _ = write!(
                out,
                r#"<details class="preview-details"><summary class="preview-summary">{} keys that are not carried over, or have nothing to carry</summary>{}</details>"#,
                folded.len(),
                table(&folded)
            );
        }
        if !plan.config_lines.is_empty() {
            out.push_str(
                r#"<p class="hint mt-sm">Some keys are read by this station only from its own <code>birdnet.conf</code>, not from the browser. After you apply, the lines to add are offered as a download; the credentials in them are not shown here.</p>"#,
            );
        }
    }

    if p.apprise.is_some() || !p.lists.is_empty() || p.confirmed_lines > 0 || !p.ignored.is_empty()
    {
        out.push_str(
            r#"<div class="result-title sm mt-sm">The other files</div><ul class="steps">"#,
        );
        if let Some(a) = &p.apprise {
            let _ = write!(
                out,
                "<li><strong><code>apprise.txt</code></strong> &mdash; {} notification address{} ({}). \
                 It is copied to this station's data folder and used as its Apprise configuration; \
                 the addresses themselves are not shown. Services without a built-in sender need \
                 the <code>apprise</code> program installed.</li>",
                a.destinations,
                if a.destinations == 1 { "" } else { "es" },
                if a.schemes.is_empty() {
                    "no recognised service".to_owned()
                } else {
                    escape_html(&a.schemes.join(", "))
                }
            );
        }
        for l in &p.lists {
            out.push_str(&list_html(l));
        }
        if p.confirmed_lines > 0 {
            let _ = write!(
                out,
                "<li><strong><code>confirmed_species_list.txt</code></strong> &mdash; {} species you \
                 marked as confirmed. There is no equivalent here; they are not carried over.</li>",
                p.confirmed_lines
            );
        }
        for name in &p.ignored {
            let _ = write!(
                out,
                "<li><code>{}</code> &mdash; not a file this import uses; skipped.</li>",
                escape_html(name)
            );
        }
        out.push_str("</ul>");
    }

    if p.assignments.is_empty() {
        out.push_str(
            r#"<p class="hint">There is nothing here to set. Nothing would change.</p></div>"#,
        );
        return out;
    }
    let _ = write!(
        out,
        r##"<p class="hint">Applying sets {n} setting{s} through the same checks as the Settings page; if any value would be refused, none are written.</p>
<div class="actions center">
  <button class="btn btn-primary" hx-post="/admin/migrate/conf/apply" hx-vals='{{"token": {token}}}'
          hx-target="#conf-import-result" hx-swap="innerHTML">Apply these settings</button>
</div></div>"##,
        n = p.assignments.len(),
        s = if p.assignments.len() == 1 { "" } else { "s" },
    );
    out
}

/// The result of applying.
#[must_use]
pub(super) fn applied(a: &Applied, token: u64, has_lines: bool, file_written: bool) -> String {
    let mut out = String::from(r#"<div class="card result-card ok"><div class="result-title ok">"#);
    if a.changed.is_empty() {
        out.push_str("Nothing needed changing: this station already had these settings.</div>");
    } else {
        let _ = write!(
            out,
            "Done. {} setting{} saved.</div>",
            a.changed.len(),
            if a.changed.len() == 1 { "" } else { "s" }
        );
    }
    if file_written {
        out.push_str(
            r#"<p class="hint">The Apprise file was copied into this station's data folder.</p>"#,
        );
    }
    if !a.need_restart.is_empty() {
        let names = a
            .need_restart
            .iter()
            .map(|k| k.replace('_', " "))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(
            out,
            r#"<div class="applies-banner" role="status"><div class="applies-banner__text"><strong>{n} of these need a restart</strong> before the station uses them: {names}.</div><button type="button" class="bnb-btn primary" data-restart-now>Restart the station now</button><span class="bnb-meta" data-restart-status role="status"></span></div>"#,
            n = a.need_restart.len(),
            names = escape_html(&names),
        );
    }
    if has_lines {
        let _ = write!(
            out,
            r#"<p class="mt-sm"><a class="bnb-btn" href="/admin/migrate/conf/lines?token={token}" download>Download the lines for birdnet.conf</a></p><p class="hint">These keys (such as the Flickr key) are read from this station's <code>birdnet.conf</code>; add the lines and restart. The download holds the real values, so keep it private.</p>"#
        );
    }
    out.push_str(r#"<p class="hint">Sign in to this station as <code>admin</code>, not <code>birdnet</code>; set its password under Access if you want your old one.</p></div>"#);
    out
}

#[cfg(test)]
mod inline_markup_tests {
    use super::{breakable, inline_markup};

    #[test]
    fn marks_become_tags_and_file_text_cannot() {
        assert_eq!(
            inline_markup("the `admin` user, not **birdnet**"),
            "the <code>admin</code> user, not <strong>birdnet</strong>"
        );
        // Escaped before marked: a tag in the text is text.
        assert!(inline_markup("<b>x</b> `y`").starts_with("&lt;b&gt;x"));
        // An unmatched mark closes at the end rather than swallowing the page.
        assert_eq!(inline_markup("a `b"), "a <code>b</code>");
    }

    #[test]
    fn names_break_after_underscores() {
        assert_eq!(
            breakable("APPRISE_NOTIFY_EACH"),
            "APPRISE_<wbr>NOTIFY_<wbr>EACH"
        );
    }
}
