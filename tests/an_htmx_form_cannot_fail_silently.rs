//! A form the page can submit cannot fail where nobody can see it.
//!
//! # The defect class
//!
//! A browser submits every control of a form, a blank one as `name=`. axum's
//! `Form` extractor refuses the request itself, with a `422`, when serde cannot
//! read a field: an `Option<u8>` over `hour_start=` does exactly that. htmx never
//! swaps a 4xx/5xx body (`static/htmx.min.js` ships `{code:"[45]..", swap:false,
//! error:true}`) and `layout.html`'s `htmx:responseError` fallback is gated on
//! `GET`, so the person who left an optional field blank, on a form whose label
//! said "blank = any", saw nothing at all. A real end-to-end run found it on the
//! alert-rule form; the audio-source form had the same shape with
//! `sample_rate`, and the migration confirm and the quarantine pager had it with
//! their numeric fields.
//!
//! # What is held
//!
//! Structural (a `syn` parse of `src/` and `crates/birdnet-web/src/`):
//!
//! 1. **No handler takes a raw `Form<T>`.** Every one takes
//!    `routes::form::HtmxForm<T>`, which answers an htmx request that cannot be
//!    read with `200` + `HX-Reswap: none` + an error toast naming the field, and
//!    anyone else with the 4xx it would always have had. Exemptions are named, with
//!    reasons, in [`RAW_FORM_EXEMPT`].
//! 2. **No form struct has an optional numeric, boolean or enum field that
//!    serde reads strictly.** `Option<u32>`, `Option<bool>`, a defaulted number or an
//!    enum must carry a `deserialize_with` from `routes::form` (`blank_is_none`,
//!    `blank_bool_is_none`, `checkbox`), which is what says "blank means none"
//!    where the field is declared. Exemptions: [`STRICT_FIELD_EXEMPT`].
//! 3. **A `Query` field of that shape is held to the same rule when a control of
//!    that name is rendered anywhere** (`name="…"` in a template or in render code),
//!    because then a browser can submit it blank. A field no page renders a control
//!    for is only reached by a link or a script, which write the value themselves.
//!    Exemptions: [`QUERY_NAME_EXEMPT`].
//!
//! Behavioural (the real router, the exact body a browser sends):
//!
//! 4. Every form handler is in [`FORMS`] (a handler the scan finds that the table
//!    lacks fails, so a new form cannot be added without being driven here).
//! 5. With every field present and blank, an htmx request is not refused with a
//!    4xx/5xx, and neither is one with an empty body.
//! 6. A garbled numeric field from htmx is answered `200`, `HX-Reswap: none`, an
//!    error toast that **names the field and does not repeat the value**; from a
//!    non-htmx client it is the 4xx it always was, with a JSON body naming the field.
//! 7. A *required* numeric field left blank is a toast naming it, not a 422.
//!
//! The scan works by name, not by type: a handler that reads a form through
//! `RawForm`, `Multipart` or a hand-rolled parser is not seen by 1 to 3. They are
//! listed by the report so a person can read them, and each one is, today, a
//! hand-written parser that answers its own errors.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use syn::spanned::Spanned;
use syn::visit::Visit;
use tower::ServiceExt as _;

// ---------------------------------------------------------------------------
// Exemptions. Each has a reason; a row that matches nothing fails.
// ---------------------------------------------------------------------------

/// `file::fn` of a handler allowed to take a raw `Form<T>`.
const RAW_FORM_EXEMPT: &[(&str, &str)] = &[];

/// `Struct.field` of a form struct allowed to read an optional numeric, boolean
/// or enum field strictly.
const STRICT_FIELD_EXEMPT: &[(&str, &str)] = &[];

/// `Struct.field` of a `Query` struct whose field name is also the name of a
/// rendered control but which no browser form can submit blank.
const DAYS_BUTTON: &str = "a script appends `?days=N` from a button's `data-days` (a 7/14/30/90 day pill); no text input is submitted for it. The one `name=\"days\"` control is the correlation page's hidden input, whose query struct carries the adapter.";
const HOURS_COLLISION: &str = "a JSON analytics query that scripts and API clients build; the `name=\"hour_start\"` / `hour_end` controls belong to the alert-rule form, which POSTs to /admin/rules, not to this endpoint.";
const HZ_SCRIPT: &str = "recordings.html appends `freq_shift_hz=` from the stored display setting only when it is non-zero; the `name=\"freq_shift_hz\"` control is the settings form, which POSTs elsewhere.";
const QUERY_NAME_EXEMPT: &[(&str, &str)] = &[
    ("AnomalyQuery.days", DAYS_BUTTON),
    ("DailyQuery.days", DAYS_BUTTON),
    ("DiversityQuery.days", DAYS_BUTTON),
    ("GapsQuery.days", DAYS_BUTTON),
    ("HeatmapQuery.days", DAYS_BUTTON),
    ("HourlyQuery.days", DAYS_BUTTON),
    ("PeakQuery.days", DAYS_BUTTON),
    ("SessionQuery.days", DAYS_BUTTON),
    ("TerrainQuery.days", DAYS_BUTTON),
    ("FunnelQuery.hour_start", HOURS_COLLISION),
    ("FunnelQuery.hour_end", HOURS_COLLISION),
    ("PatternsQuery.hour_start", HOURS_COLLISION),
    ("PatternsQuery.hour_end", HOURS_COLLISION),
    (
        "GapsQuery.threshold",
        "a JSON analytics query; the `name=\"threshold\"` controls are the per-species threshold and metric-rule forms, which POST to other endpoints.",
    ),
    ("StartQuery.freq_shift_hz", HZ_SCRIPT),
    ("StreamParams.freq_shift_hz", HZ_SCRIPT),
];

/// Adapters that say what a blank means.
const LENIENT: &[&str] = &["blank_is_none", "blank_bool_is_none", "checkbox"];

// ---------------------------------------------------------------------------
// The scan.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Field {
    name: String,
    /// The type as written, whitespace removed.
    ty: String,
    /// The text of the field's `#[serde(..)]` attributes.
    serde: String,
}

#[derive(Debug, Clone, Default)]
struct StructDef {
    file: String,
    fields: Vec<Field>,
}

#[derive(Debug, Clone)]
struct Handler {
    file: String,
    func: String,
    /// `Form`, `HtmxForm`, `Query` or `Json`.
    extractor: String,
    /// The last path segment of the inner type.
    inner: String,
}

#[derive(Default)]
struct Scan {
    structs: BTreeMap<String, Vec<StructDef>>,
    handlers: Vec<Handler>,
    /// Handlers that read a form some other way: name only, for the report.
    other_readers: Vec<(String, String, String)>,
}

fn is_test_attr(a: &syn::Attribute) -> bool {
    if a.path().is_ident("test") {
        return true;
    }
    a.path().is_ident("cfg")
        && a.meta.require_list().is_ok_and(|l| {
            let t = l.tokens.to_string();
            t == "test" || t.starts_with("all (test ,")
        })
}

fn text_of(src: &[&str], span: proc_macro2::Span) -> String {
    let (a, b) = (span.start(), span.end());
    if a.line == 0 || b.line < a.line || b.line > src.len() {
        return String::new();
    }
    let mut out = String::new();
    for ln in a.line..=b.line {
        let line = src[ln - 1];
        let from = if ln == a.line { a.column } else { 0 };
        let to = if ln == b.line {
            b.column
        } else {
            line.chars().count()
        };
        out.extend(line.chars().skip(from).take(to.saturating_sub(from)));
        out.push(' ');
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn last_segment(ty: &syn::Type) -> Option<(String, Vec<syn::Type>)> {
    let syn::Type::Path(p) = ty else { return None };
    let seg = p.path.segments.last()?;
    let mut args = Vec::new();
    if let syn::PathArguments::AngleBracketed(a) = &seg.arguments {
        for g in &a.args {
            if let syn::GenericArgument::Type(t) = g {
                args.push(t.clone());
            }
        }
    }
    Some((seg.ident.to_string(), args))
}

struct Visitor<'a> {
    file: &'a str,
    lines: Vec<&'a str>,
    in_test: usize,
    out: &'a mut Scan,
}

impl<'ast> Visit<'ast> for Visitor<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        let t = usize::from(m.attrs.iter().any(is_test_attr));
        self.in_test += t;
        syn::visit::visit_item_mod(self, m);
        self.in_test -= t;
    }
    fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
        if self.in_test > 0 || s.attrs.iter().any(is_test_attr) {
            return;
        }
        let mut def = StructDef {
            file: self.file.to_string(),
            fields: Vec::new(),
        };
        if let syn::Fields::Named(n) = &s.fields {
            for f in &n.named {
                let serde = f
                    .attrs
                    .iter()
                    .filter(|a| a.path().is_ident("serde"))
                    .map(|a| text_of(&self.lines, a.meta.span()))
                    .collect::<Vec<_>>()
                    .join(" ");
                def.fields.push(Field {
                    name: f
                        .ident
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    ty: text_of(&self.lines, f.ty.span()).replace(' ', ""),
                    serde,
                });
            }
        }
        self.out
            .structs
            .entry(s.ident.to_string())
            .or_default()
            .push(def);
    }
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if self.in_test > 0 || f.attrs.iter().any(is_test_attr) {
            return;
        }
        self.sig(&f.sig);
        syn::visit::visit_item_fn(self, f);
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if self.in_test > 0 || f.attrs.iter().any(is_test_attr) {
            return;
        }
        self.sig(&f.sig);
        syn::visit::visit_impl_item_fn(self, f);
    }
}

impl Visitor<'_> {
    fn sig(&mut self, sig: &syn::Signature) {
        for a in &sig.inputs {
            let syn::FnArg::Typed(t) = a else { continue };
            let Some((name, args)) = last_segment(&t.ty) else {
                continue;
            };
            match name.as_str() {
                "Form" | "HtmxForm" | "Query" | "Json" => {
                    let inner = args
                        .first()
                        .and_then(last_segment)
                        .map(|(n, _)| n)
                        .unwrap_or_default();
                    self.out.handlers.push(Handler {
                        file: self.file.to_string(),
                        func: sig.ident.to_string(),
                        extractor: name,
                        inner,
                    });
                }
                "RawForm" | "Multipart" => self.out.other_readers.push((
                    self.file.to_string(),
                    sig.ident.to_string(),
                    name,
                )),
                _ => {}
            }
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn scan_sources(sources: &[(String, String)]) -> Scan {
    let mut scan = Scan::default();
    for (file, src) in sources {
        let ast = syn::parse_file(src).unwrap_or_else(|e| panic!("{file}: {e}"));
        let mut v = Visitor {
            file,
            lines: src.lines().collect(),
            in_test: 0,
            out: &mut scan,
        };
        v.visit_file(&ast);
    }
    scan
}

fn real_sources() -> Vec<(String, String)> {
    let mut files = Vec::new();
    walk(&root().join("src"), &mut files);
    walk(&root().join("crates/birdnet-web/src"), &mut files);
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(root())
                .expect("under root")
                .display()
                .to_string();
            let src = std::fs::read_to_string(&p).expect("readable");
            (rel, src)
        })
        .collect()
}

/// `file::fn`, with the path shortened to what follows `routes/`.
fn key(h: &Handler) -> String {
    let f = h
        .file
        .rsplit_once("routes/")
        .map_or(h.file.as_str(), |x| x.1);
    format!("{f}::{}", h.func)
}

/// Is a field type one serde reads from any string without complaint?
fn is_stringish(ty: &str) -> bool {
    let base = ty
        .strip_prefix("Option<")
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(ty);
    matches!(
        base,
        "String"
            | "Vec<String>"
            | "HashMap<String,String>"
            | "std::collections::HashMap<String,String>"
            | "BTreeMap<String,String>"
    )
}

fn is_lenient(f: &Field) -> bool {
    f.serde.contains("deserialize_with")
        && LENIENT
            .iter()
            .any(|l| f.serde.contains(&format!("\"{l}\"")) || f.serde.contains(&format!("::{l}\"")))
}

/// A field that serde reads strictly and that a blank would break: not a string,
/// and either optional or defaulted (so the page may leave it out or blank).
/// A required, non-defaulted field is a hidden server-filled value; the wrapper
/// answers its absence.
fn strict_blank_hazard(f: &Field) -> bool {
    if is_stringish(&f.ty) || is_lenient(f) || f.serde.contains("flatten") {
        return false;
    }
    f.ty.starts_with("Option<") || f.serde.contains("default")
}

fn structs_for<'a>(scan: &'a Scan, h: &Handler) -> Vec<&'a StructDef> {
    let Some(all) = scan.structs.get(&h.inner) else {
        return Vec::new();
    };
    let same: Vec<&StructDef> = all.iter().filter(|s| s.file == h.file).collect();
    if same.is_empty() {
        all.iter().collect()
    } else {
        same
    }
}

/// Every `name="x"` a page renders, in render code and templates.
fn control_names() -> BTreeSet<String> {
    let mut texts = Vec::new();
    let mut files = Vec::new();
    walk(&root().join("crates/birdnet-web/src"), &mut files);
    for p in files {
        texts.push(std::fs::read_to_string(p).expect("readable"));
    }
    if let Ok(rd) = std::fs::read_dir(root().join("crates/birdnet-web/templates")) {
        for e in rd.flatten() {
            if let Ok(t) = std::fs::read_to_string(e.path()) {
                texts.push(t);
            }
        }
    }
    let mut names = BTreeSet::new();
    for t in texts {
        let mut rest = t.as_str();
        while let Some(i) = rest.find("name=") {
            rest = &rest[i + 5..];
            let r = rest.strip_prefix("\\\"").or_else(|| rest.strip_prefix('"'));
            if let Some(r) = r {
                let n: String = r
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !n.is_empty() {
                    names.insert(n);
                }
            }
        }
    }
    names
}

// ---------------------------------------------------------------------------
// The findings, as functions of the scan so the counterparts can drive them.
// ---------------------------------------------------------------------------

fn raw_forms(scan: &Scan, exempt: &[(&str, &str)]) -> Vec<String> {
    scan.handlers
        .iter()
        .filter(|h| h.extractor == "Form" && !h.file.ends_with("routes/form.rs"))
        .map(key)
        .filter(|k| !exempt.iter().any(|(e, _)| e == k))
        .collect()
}

fn strict_form_fields(scan: &Scan, exempt: &[(&str, &str)]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for h in scan
        .handlers
        .iter()
        .filter(|h| matches!(h.extractor.as_str(), "Form" | "HtmxForm"))
    {
        let defs = structs_for(scan, h);
        if defs.is_empty() {
            out.insert(format!("{}: form struct {} not found", key(h), h.inner));
        }
        for d in defs {
            for f in &d.fields {
                let id = format!("{}.{}", h.inner, f.name);
                if strict_blank_hazard(f) && !exempt.iter().any(|(e, _)| *e == id) {
                    out.insert(format!("{id}: {} ({})", f.ty, f.serde));
                }
            }
        }
    }
    out.into_iter().collect()
}

fn strict_query_fields(
    scan: &Scan,
    controls: &BTreeSet<String>,
    exempt: &[(&str, &str)],
) -> Vec<String> {
    let mut out = BTreeSet::new();
    for h in scan.handlers.iter().filter(|h| h.extractor == "Query") {
        for d in structs_for(scan, h) {
            for f in &d.fields {
                let id = format!("{}.{}", h.inner, f.name);
                if strict_blank_hazard(f)
                    && controls.contains(&f.name)
                    && !exempt.iter().any(|(e, _)| *e == id)
                {
                    out.insert(format!("{id}: {} ({})", f.ty, f.serde));
                }
            }
        }
    }
    out.into_iter().collect()
}

fn unused_exemptions(scan: &Scan, controls: &BTreeSet<String>) -> Vec<String> {
    let mut stale = Vec::new();
    for (k, _) in RAW_FORM_EXEMPT {
        if !scan
            .handlers
            .iter()
            .any(|h| h.extractor == "Form" && key(h) == *k)
        {
            stale.push(format!("RAW_FORM_EXEMPT {k}"));
        }
    }
    for (id, _) in STRICT_FIELD_EXEMPT {
        if strict_form_fields(scan, &[])
            .iter()
            .all(|f| !f.starts_with(id))
        {
            stale.push(format!("STRICT_FIELD_EXEMPT {id}"));
        }
    }
    for (id, _) in QUERY_NAME_EXEMPT {
        if strict_query_fields(scan, controls, &[])
            .iter()
            .all(|f| !f.starts_with(id))
        {
            stale.push(format!("QUERY_NAME_EXEMPT {id}"));
        }
    }
    stale
}

fn src(s: &str) -> Vec<(String, String)> {
    vec![(
        "crates/birdnet-web/src/routes/x.rs".to_string(),
        s.to_string(),
    )]
}

// ---------------------------------------------------------------------------
// Structural gates.
// ---------------------------------------------------------------------------

#[test]
fn no_handler_takes_a_raw_form() {
    let scan = scan_sources(&real_sources());
    let bad = raw_forms(&scan, RAW_FORM_EXEMPT);
    assert!(
        bad.is_empty(),
        "these handlers take axum's `Form`, whose rejection is a 422 that htmx never shows; \
         take `routes::form::HtmxForm` instead:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn no_form_field_reads_a_blank_strictly() {
    let scan = scan_sources(&real_sources());
    let bad = strict_form_fields(&scan, STRICT_FIELD_EXEMPT);
    assert!(
        bad.is_empty(),
        "an optional or defaulted numeric/bool/enum form field is read strictly, so `name=` from a \
         browser is a 422; add `deserialize_with = \"routes::form::blank_is_none\"` (or \
         `blank_bool_is_none` / `checkbox`):\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn no_query_field_with_a_rendered_control_reads_a_blank_strictly() {
    let scan = scan_sources(&real_sources());
    let bad = strict_query_fields(&scan, &control_names(), QUERY_NAME_EXEMPT);
    assert!(
        bad.is_empty(),
        "a `Query` field of this shape shares its name with a control a page renders, so a browser \
         can submit it blank; add the adapter or name an exemption with a reason:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn every_exemption_still_matches_something() {
    let scan = scan_sources(&real_sources());
    let stale = unused_exemptions(&scan, &control_names());
    assert!(
        stale.is_empty(),
        "stale exemptions:\n  {}",
        stale.join("\n  ")
    );
    for (_, reason) in RAW_FORM_EXEMPT
        .iter()
        .chain(STRICT_FIELD_EXEMPT)
        .chain(QUERY_NAME_EXEMPT)
    {
        assert!(reason.len() > 20, "an exemption needs a real reason");
    }
}

// Counterparts: the checks fire on the shapes they claim, and pass the shapes
// they claim to pass.

#[test]
fn counterpart_a_raw_form_is_found_and_the_wrapper_is_not() {
    let raw = scan_sources(&src(
        "#[derive(serde::Deserialize)] struct F { a: String }\n\
         async fn h(axum::extract::Form(f): axum::extract::Form<F>) {}",
    ));
    assert_eq!(raw_forms(&raw, &[]).len(), 1);
    let wrapped = scan_sources(&src(
        "#[derive(serde::Deserialize)] struct F { a: String }\n\
         async fn h(HtmxForm(f): HtmxForm<F>) {}",
    ));
    assert!(raw_forms(&wrapped, &[]).is_empty());
    assert_eq!(
        raw_forms(&raw, &[("y.rs::h", "named")]).len(),
        1,
        "a different key does not exempt"
    );
}

#[test]
fn counterpart_strict_fields_are_found_and_lenient_or_string_ones_are_not() {
    let strict = scan_sources(&src(
        "struct F { #[serde(default)] n: Option<u32>, m: Option<bool>, #[serde(default)] k: u8, mode: Option<Mode> }\n\
         async fn h(HtmxForm(f): HtmxForm<F>) {}",
    ));
    assert_eq!(
        strict_form_fields(&strict, &[]).len(),
        4,
        "{:?}",
        strict_form_fields(&strict, &[])
    );

    let fine = scan_sources(&src("struct F {\n\
           #[serde(default, deserialize_with = \"crate::routes::form::blank_is_none\")] n: Option<u32>,\n\
           #[serde(default, deserialize_with = \"blank_bool_is_none\")] m: Option<bool>,\n\
           #[serde(default, deserialize_with = \"checkbox\")] k: bool,\n\
           s: Option<String>,\n\
           #[serde(default)] t: String,\n\
           id: i64,\n\
           #[serde(flatten)] extra: std::collections::HashMap<String, String>,\n\
         }\n\
         async fn h(HtmxForm(f): HtmxForm<F>) {}"));
    assert!(
        strict_form_fields(&fine, &[]).is_empty(),
        "{:?}",
        strict_form_fields(&fine, &[])
    );
}

#[test]
fn counterpart_a_deserialize_with_that_is_not_ours_does_not_count() {
    let other = scan_sources(&src(
        "struct F { #[serde(default, deserialize_with = \"whatever\")] n: Option<u32> }\n\
         async fn h(HtmxForm(f): HtmxForm<F>) {}",
    ));
    assert_eq!(strict_form_fields(&other, &[]).len(), 1);
}

#[test]
fn counterpart_a_query_field_is_held_only_when_a_control_has_its_name() {
    let s = scan_sources(&src(
        "struct Q { limit: Option<u32>, offset: Option<u32> }\n\
         async fn h(Query(q): Query<Q>) {}",
    ));
    let controls: BTreeSet<String> = ["offset".to_string()].into();
    let bad = strict_query_fields(&s, &controls, &[]);
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].starts_with("Q.offset"));
    assert!(strict_query_fields(&s, &BTreeSet::new(), &[]).is_empty());
}

#[test]
fn counterpart_the_test_code_of_a_file_is_not_scanned() {
    let s = scan_sources(&src(
        "#[cfg(test)] mod tests { struct F { n: Option<u32> } async fn h(Form(f): Form<F>) {} }",
    ));
    assert!(raw_forms(&s, &[]).is_empty());
}

#[test]
fn the_scan_sees_the_handlers_it_is_about() {
    let scan = scan_sources(&real_sources());
    let forms: Vec<_> = scan
        .handlers
        .iter()
        .filter(|h| matches!(h.extractor.as_str(), "Form" | "HtmxForm"))
        .collect();
    assert!(
        forms.len() >= 45,
        "the scan found {} form handlers; it found 51 when this was written, so it has stopped seeing them",
        forms.len()
    );
    assert!(
        control_names().contains("hour_start"),
        "the control scan is blind"
    );
}

// ---------------------------------------------------------------------------
// Behavioural gates: the real router, the body a browser sends.
// ---------------------------------------------------------------------------

/// How a typed field is held. `Optional`: blank means none and must be accepted.
/// `Required`: blank is a real error and must be a toast that names it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Typed {
    Optional(&'static str),
    Required(&'static str),
    None,
}

struct Form {
    /// `routes/` path of the file, `::`, the function.
    handler: &'static str,
    method: &'static str,
    uri: &'static str,
    fields: &'static [&'static str],
    typed: Typed,
}

const fn f(
    handler: &'static str,
    method: &'static str,
    uri: &'static str,
    fields: &'static [&'static str],
    typed: Typed,
) -> Form {
    Form {
        handler,
        method,
        uri,
        fields,
        typed,
    }
}

const AUDIO: &[&str] = &[
    "scope",
    "kind",
    "device_id",
    "label",
    "sample_rate",
    "rtsp_transport",
    "media_mode",
    "quiet_start",
    "quiet_end",
    "high_pass",
    "dc_removal",
    "agc",
    "rtsp_keepalive",
    "pipeline_present",
    "eq_chain",
];
const ROW: &[&str] = &["date", "time", "sci_name", "file_name"];
const NAME: &[&str] = &["name"];
const SCI: &[&str] = &["sci_name"];
const MIGRATE: &[&str] = &["source_path", "source_label", "source_utc_offset_secs"];
const CLIPS: &[&str] = &["clips_source", "clips_mode", "pi_folder"];
const QUAR: &[&str] = &["id", "filter", "offset"];
const REVIEW: &[&str] = &["date", "time", "sci_name", "com_name", "status"];

/// Every handler that takes a form, with the body a browser sends for it.
const FORMS: &[Form] = &[
    f(
        "admin/audio.rs::create",
        "POST",
        "/admin/audio/sources",
        AUDIO,
        Typed::Optional("sample_rate"),
    ),
    f(
        "admin/audio.rs::update",
        "PATCH",
        "/admin/audio/sources/src_x",
        AUDIO,
        Typed::Optional("sample_rate"),
    ),
    f(
        "admin/settings/handler.rs::save_settings",
        "POST",
        "/admin/settings",
        &[
            "latitude",
            "longitude",
            "confidence_threshold",
            "station_name",
            "email_smtp_port",
        ],
        Typed::None,
    ),
    f(
        "admin/images.rs::add_blacklist",
        "POST",
        "/admin/images/blacklist",
        &["sci_name", "url", "reason"],
        Typed::None,
    ),
    f(
        "admin/species/learned.rs::reset_one",
        "POST",
        "/admin/species/learned/reset",
        SCI,
        Typed::None,
    ),
    f(
        "admin/species/manage.rs::exclude",
        "POST",
        "/admin/species/manage/exclude",
        SCI,
        Typed::None,
    ),
    f(
        "admin/species/manage.rs::delete_detections",
        "POST",
        "/admin/species/manage/detections",
        SCI,
        Typed::None,
    ),
    f(
        "admin/species/manage.rs::delete_clips",
        "POST",
        "/admin/species/manage/clips",
        SCI,
        Typed::None,
    ),
    f(
        "admin/species/manage.rs::measure",
        "POST",
        "/admin/species/manage/measure",
        SCI,
        Typed::None,
    ),
    f(
        "admin/species/handler.rs::add_exclude",
        "POST",
        "/admin/species/exclude/add",
        NAME,
        Typed::None,
    ),
    f(
        "admin/species/handler.rs::remove_exclude",
        "POST",
        "/admin/species/exclude/remove",
        NAME,
        Typed::None,
    ),
    f(
        "admin/species/handler.rs::add_include",
        "POST",
        "/admin/species/include/add",
        NAME,
        Typed::None,
    ),
    f(
        "admin/species/handler.rs::remove_include",
        "POST",
        "/admin/species/include/remove",
        NAME,
        Typed::None,
    ),
    f(
        "admin/species/handler.rs::add_whitelist",
        "POST",
        "/admin/species/whitelist/add",
        NAME,
        Typed::None,
    ),
    f(
        "admin/species/handler.rs::remove_whitelist",
        "POST",
        "/admin/species/whitelist/remove",
        NAME,
        Typed::None,
    ),
    f(
        "admin/species/handler.rs::set_threshold",
        "POST",
        "/admin/species/thresholds/set",
        &["sci_name", "threshold"],
        Typed::None,
    ),
    f(
        "admin/species/handler.rs::delete_threshold",
        "POST",
        "/admin/species/thresholds/delete",
        SCI,
        Typed::None,
    ),
    f(
        "admin/species/handler.rs::set_interval",
        "POST",
        "/admin/species/intervals/set",
        &["sci_name", "seconds"],
        Typed::None,
    ),
    f(
        "admin/species/handler.rs::delete_interval",
        "POST",
        "/admin/species/intervals/delete",
        SCI,
        Typed::None,
    ),
    f(
        "admin/quality.rs::exclude_phantom",
        "POST",
        "/admin/quality/phantoms/exclude",
        SCI,
        Typed::None,
    ),
    f(
        "admin/accounts.rs::create_user",
        "POST",
        "/admin/accounts/users",
        &["username", "password", "label"],
        Typed::None,
    ),
    f(
        "admin/accounts.rs::set_password",
        "POST",
        "/admin/accounts/users/1",
        &["password"],
        Typed::None,
    ),
    f(
        "admin/rules.rs::import_rules",
        "POST",
        "/admin/rules/import",
        &["rules_json"],
        Typed::None,
    ),
    f(
        "admin/rules.rs::create_rule",
        "POST",
        "/admin/rules",
        &[
            "name",
            "species_pattern",
            "confidence_min",
            "confidence_max",
            "hour_start",
            "hour_end",
            "days_of_week",
            "action_type",
            "action_webhook_url",
            "action_webhook_method",
            "action_webhook_body",
            "action_webhook_auth_kind",
            "action_webhook_auth_value",
            "action_webhook_header_name",
            "cooldown_minutes",
        ],
        Typed::None,
    ),
    f(
        "admin/rules.rs::create_metric_rule",
        "POST",
        "/admin/metric-rules",
        &["name", "metric", "comparison", "threshold"],
        Typed::None,
    ),
    f(
        "admin/migration/conf_handlers.rs::apply_handler",
        "POST",
        "/admin/migrate/conf/apply",
        &["token"],
        Typed::None,
    ),
    f(
        "admin/migration/clip_import.rs::preview_handler",
        "POST",
        "/admin/migrate/clips/preview",
        CLIPS,
        Typed::None,
    ),
    f(
        "admin/migration/clip_import.rs::run_handler",
        "POST",
        "/admin/migrate/clips/run",
        CLIPS,
        Typed::None,
    ),
    f(
        "admin/migration.rs::delete_batch_handler",
        "POST",
        "/admin/migrate/batches/delete",
        &["batch_id"],
        Typed::Required("batch_id"),
    ),
    f(
        "admin/migration.rs::provenance_handler",
        "POST",
        "/admin/migrate/batches/provenance",
        &["exclude"],
        Typed::None,
    ),
    f(
        "admin/migration.rs::validate_handler",
        "POST",
        "/admin/migrate/validate",
        MIGRATE,
        Typed::Optional("source_utc_offset_secs"),
    ),
    f(
        "admin/migration.rs::run_handler",
        "POST",
        "/admin/migrate/run",
        MIGRATE,
        Typed::Optional("source_utc_offset_secs"),
    ),
    f(
        "admin/migration.rs::upload_confirm_handler",
        "POST",
        "/admin/migrate/upload/confirm",
        &["token"],
        Typed::Optional("token"),
    ),
    f(
        "auth_pages.rs::login_submit",
        "POST",
        "/login",
        &["username", "password", "remember", "next"],
        Typed::None,
    ),
    f(
        "pages/detection_comments.rs::add_comment",
        "POST",
        "/pages/detection-comments/add",
        &["date", "time", "sci_name", "body"],
        Typed::None,
    ),
    f(
        "pages/detection_comments.rs::delete_comment",
        "POST",
        "/pages/detection-comments/delete",
        &["date", "time", "sci_name", "id"],
        Typed::Required("id"),
    ),
    f(
        "pages/today.rs::delete_detection",
        "POST",
        "/pages/today-delete",
        ROW,
        Typed::None,
    ),
    f(
        "pages/today.rs::relabel_detection",
        "POST",
        "/pages/today-relabel",
        &[
            "date",
            "time",
            "old_sci_name",
            "new_sci_name",
            "new_com_name",
            "file_name",
        ],
        Typed::None,
    ),
    f(
        "pages/today.rs::lock_detection",
        "POST",
        "/pages/today-lock",
        ROW,
        Typed::None,
    ),
    f(
        "pages/today.rs::unlock_detection",
        "POST",
        "/pages/today-unlock",
        ROW,
        Typed::None,
    ),
    f(
        "pages/detection_reviews.rs::detection_review_set",
        "POST",
        "/pages/detection-review",
        REVIEW,
        Typed::None,
    ),
    f(
        "pages/detection_reviews.rs::detection_review_clear",
        "POST",
        "/pages/detection-review-clear",
        &["date", "time", "sci_name"],
        Typed::None,
    ),
    f(
        "pages/detection_reviews.rs::detection_review_inline",
        "POST",
        "/pages/detection-review-inline",
        REVIEW,
        Typed::None,
    ),
    f(
        "pages/recordings.rs::lock_clip",
        "POST",
        "/pages/recordings-lock",
        ROW,
        Typed::None,
    ),
    f(
        "pages/recordings.rs::unlock_clip",
        "POST",
        "/pages/recordings-unlock",
        ROW,
        Typed::None,
    ),
    f(
        "pages/recordings.rs::delete_clip",
        "POST",
        "/pages/recordings-delete",
        ROW,
        Typed::None,
    ),
    f(
        "pages/onboarding.rs::onboarding_save",
        "POST",
        "/onboarding/save",
        &[
            "latitude",
            "longitude",
            "timezone",
            "timezone_latitude",
            "timezone_longitude",
            "notification_mode",
            "confidence_threshold",
            "station_name",
            "database_lang",
            "mic_choice",
            "mic_rtsp_url",
            "mic_manual_device",
            "password",
            "password_confirm",
        ],
        Typed::None,
    ),
    f(
        "pages/onboarding.rs::mic_test",
        "POST",
        "/onboarding/mic-test",
        &["device"],
        Typed::None,
    ),
    f(
        "pages/quarantine.rs::quarantine_approve",
        "POST",
        "/pages/quarantine-approve",
        QUAR,
        Typed::Optional("offset"),
    ),
    f(
        "pages/quarantine.rs::quarantine_approve",
        "POST",
        "/pages/quarantine-approve",
        QUAR,
        Typed::Required("id"),
    ),
    f(
        "pages/quarantine.rs::quarantine_reject",
        "POST",
        "/pages/quarantine-reject",
        QUAR,
        Typed::Optional("offset"),
    ),
    f(
        "pages/quarantine.rs::quarantine_reject",
        "POST",
        "/pages/quarantine-reject",
        QUAR,
        Typed::Required("id"),
    ),
    f(
        "pages/quarantine.rs::quarantine_delete",
        "POST",
        "/pages/quarantine-delete",
        QUAR,
        Typed::Optional("offset"),
    ),
    f(
        "pages/quarantine.rs::quarantine_delete",
        "POST",
        "/pages/quarantine-delete",
        QUAR,
        Typed::Required("id"),
    ),
];

/// The marker a garbled value carries, so a toast that repeats it is caught.
const MARK: &str = "zzmarker77";

struct Reply {
    status: u16,
    reswap: String,
    ctype: String,
    body: String,
}

async fn send(state: &AppState, form: &Form, htmx: bool, body: String) -> Reply {
    let mut b = Request::builder()
        .method(form.method)
        .uri(form.uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if htmx {
        b = b.header("HX-Request", "true");
    }
    let res = build_router(state.clone())
        .oneshot(b.body(Body::from(body)).expect("request"))
        .await
        .expect("response");
    let status = res.status().as_u16();
    let header_text = |res: &axum::response::Response, n: &str| {
        res.headers()
            .get(n)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let (reswap, ctype) = (
        header_text(&res, "hx-reswap"),
        header_text(&res, "content-type"),
    );
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    Reply {
        status,
        reswap,
        ctype,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn blank_body(fields: &[&str]) -> String {
    fields
        .iter()
        .map(|f| format!("{f}="))
        .collect::<Vec<_>>()
        .join("&")
}

fn body_with(fields: &[&str], field: &str, value: &str) -> String {
    fields
        .iter()
        .map(|f| {
            if *f == field {
                format!("{f}={value}")
            } else if matches!(*f, "id" | "batch_id" | "token") {
                // A hidden id is never blank in a real page; keep it valid so the
                // field under test is the first one serde meets.
                format!("{f}=1")
            } else {
                format!("{f}=")
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn is_error_toast(r: &Reply) -> bool {
    r.body.contains("bnb-toast") && r.body.contains(r#"data-kind="error""#)
}

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

#[test]
fn every_form_handler_is_driven_by_the_behavioural_table() {
    let scan = scan_sources(&real_sources());
    let have: BTreeSet<String> = scan
        .handlers
        .iter()
        .filter(|h| matches!(h.extractor.as_str(), "Form" | "HtmxForm"))
        .map(key)
        .collect();
    let table: BTreeSet<String> = FORMS.iter().map(|x| x.handler.to_string()).collect();
    let missing: Vec<_> = have.difference(&table).collect();
    let stale: Vec<_> = table.difference(&have).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "form handlers the table does not drive: {missing:?}; table rows with no handler: {stale:?}"
    );
}

#[tokio::test]
async fn a_blank_browser_form_is_never_refused_where_nobody_can_see_it() {
    let (_dir, state) = station();
    let mut failures = Vec::new();
    for form in FORMS {
        for (what, body) in [
            ("every field present and blank", blank_body(form.fields)),
            ("empty body", String::new()),
        ] {
            let r = send(&state, form, true, body).await;
            // A required numeric left blank is allowed to be refused, but only as
            // a toast on a 200; everything else must not be a 4xx/5xx at all.
            if r.status >= 400 {
                failures.push(format!(
                    "{} ({what}): {} {:.90}",
                    form.handler, r.status, r.body
                ));
            }
        }
        if let Typed::Optional(field) = form.typed {
            let r = send(&state, form, true, blank_body(form.fields)).await;
            if r.body
                .contains(&format!("\u{201c}{field}\u{201d} was left blank"))
            {
                failures.push(format!(
                    "{}: `{field}` is optional but a blank was answered as an error",
                    form.handler
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[tokio::test]
async fn a_garbled_number_is_a_toast_that_names_the_field_and_not_the_value() {
    let (_dir, state) = station();
    let mut failures = Vec::new();
    let mut driven = 0;
    for form in FORMS {
        let (Typed::Optional(field) | Typed::Required(field)) = form.typed else {
            continue;
        };
        driven += 1;
        let body = body_with(form.fields, field, MARK);

        let r = send(&state, form, true, body.clone()).await;
        if r.status != 200 || r.reswap != "none" || !is_error_toast(&r) || !r.body.contains(field) {
            failures.push(format!(
                "{} htmx: want 200 + reswap none + error toast naming `{field}`, got {} reswap={:?} {:.100}",
                form.handler, r.status, r.reswap, r.body
            ));
        }
        if r.body.contains(MARK) {
            failures.push(format!(
                "{} htmx: the toast repeats the submitted value",
                form.handler
            ));
        }

        let r = send(&state, form, false, body).await;
        if !(400..500).contains(&r.status) || !r.ctype.contains("json") || !r.body.contains(field) {
            failures.push(format!(
                "{} plain: want a 4xx JSON error naming `{field}`, got {} {} {:.100}",
                form.handler, r.status, r.ctype, r.body
            ));
        }
        if r.body.contains(MARK) {
            failures.push(format!(
                "{} plain: the error repeats the submitted value",
                form.handler
            ));
        }
    }
    assert!(driven >= 8, "only {driven} typed forms are in the table");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[tokio::test]
async fn a_required_number_left_blank_is_a_toast_that_names_it() {
    let (_dir, state) = station();
    let mut seen = 0;
    for form in FORMS {
        let Typed::Required(field) = form.typed else {
            continue;
        };
        seen += 1;
        let r = send(&state, form, true, blank_body(form.fields)).await;
        assert!(
            r.status == 200 && is_error_toast(&r) && r.body.contains(field),
            "{}: blank `{field}` -> {} {:.100}",
            form.handler,
            r.status,
            r.body
        );
    }
    assert!(seen >= 2);
}

#[tokio::test]
async fn counterpart_a_request_that_is_not_htmx_keeps_its_4xx() {
    // The same unreadable body, without the htmx header, must still be refused
    // with a status a script can act on: the wrapper changes what a *page*
    // sees, not what an API client is told.
    let (_dir, state) = station();
    let form = FORMS
        .iter()
        .find(|x| x.uri == "/admin/migrate/batches/delete")
        .expect("row");
    let r = send(&state, form, false, "batch_id=".to_string()).await;
    assert_eq!(r.status, 422, "{}", r.body);
    let r = send(&state, form, true, "batch_id=".to_string()).await;
    assert_eq!(r.status, 200, "{}", r.body);
}

#[tokio::test]
async fn counterpart_a_good_body_still_works() {
    // The success path is unchanged: a valid number is read, not refused.
    let (_dir, state) = station();
    let form = FORMS
        .iter()
        .find(|x| x.uri == "/admin/migrate/batches/delete")
        .expect("row");
    let r = send(&state, form, true, "batch_id=99999".to_string()).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(!r.body.contains("Not submitted"), "{}", r.body);
}

// ---------------------------------------------------------------------------
// Other htmx writes that refused a user-fixable condition with a 4xx/5xx.
// ---------------------------------------------------------------------------

/// A multipart body that is cut off in the middle of the file part: what a
/// dropped connection, or a file past the body limit, looks like to the handler.
const CUT_OFF: &str = "--XBOUNDARY\r\nContent-Disposition: form-data; name=\"source_file\"; \
                       filename=\"a.db\"\r\nContent-Type: application/octet-stream\r\n\r\nSQLite";

async fn multipart(state: &AppState, uri: &str, body: &str) -> Reply {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("HX-Request", "true")
        .header(
            header::CONTENT_TYPE,
            "multipart/form-data; boundary=XBOUNDARY",
        )
        .body(Body::from(body.to_owned()))
        .expect("request");
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    Reply {
        status,
        reswap: String::new(),
        ctype: String::new(),
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

#[tokio::test]
async fn an_upload_that_stops_arriving_is_said_in_the_result_slot() {
    let (_dir, state) = station();
    for (uri, body) in [
        ("/admin/migrate/upload", CUT_OFF),
        (
            "/admin/migrate/conf/preview",
            "--XBOUNDARY\r\nContent-Disposition: form-data; name=\"conf_text\"\r\n\r\nLATITUDE=1",
        ),
    ] {
        let r = multipart(&state, uri, body).await;
        assert_eq!(
            r.status, 200,
            "{uri}: a 4xx is never swapped by htmx: {}",
            r.body
        );
        assert!(
            r.body.contains("result-card") || r.body.contains("err"),
            "{uri}: the answer must say something: {}",
            r.body
        );
    }
}

#[tokio::test]
async fn a_rule_deleted_in_another_tab_says_so_when_tested() {
    // The page is stale, the rule is gone: the button answered 404, which htmx
    // never swaps, so the click did nothing at all.
    let (_dir, state) = station();
    let r = multipart(&state, "/admin/rules/987654/test", "").await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body.contains("no longer exists"), "{}", r.body);
}
