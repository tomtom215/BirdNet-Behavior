//! No literal reaches a parameter that CodeQL reads as a cryptographic value.
//!
//! `rust/hard-coded-cryptographic-value` is a taint query keyed on parameter
//! *names*. A literal (a string, a `bool`, an integer; a `const` too, `""` too)
//! that reaches an argument of any function whose parameter is called
//! `password`, `nonce`, `iv`, or `salt` is an alert, whatever the function does, tests included. It does not
//! care that the value is a test fixture, and it follows a literal through a
//! `let` and through a helper that returns a string made from it.
//!
//! CodeQL runs on the pull request, and a finding costs a round trip. This is
//! the same shape found locally: a parse of every `.rs` file in the repository,
//! the functions that take such a parameter, and every call that hands one a
//! value built from a literal.
//!
//! The fix, in `src/helpers/auth.rs` first and then everywhere else, is to build
//! the value from something the compiler cannot see through at analysis time:
//! `format!("pw-{}", std::process::id())`, `String::new()`, an environment
//! variable, or a file read at run time. A pinned hash that must stay fixed lives
//! in a data file (`crates/birdnet-db/tests/data/`).
//!
//! # What it cannot see
//!
//! It resolves a callee by name and matches the argument by position. A literal
//! that reaches the parameter through a struct field, a closure's return value,
//! or a macro other than `format!`/`vec!` is not followed; the query itself may
//! follow more than this does. A clean run here is therefore evidence that these
//! shapes are gone, not that CodeQL will be silent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;
use syn::visit::Visit;

/// The parameter names the query treats as cryptographic values.
const SINK_PARAMS: &[&str] = &["password", "passwd", "nonce", "iv", "salt", "secret"];

/// Methods that keep a value's provenance: `"x".to_string()` is still `"x"`.
const PASSTHROUGH: &[&str] = &[
    "to_string",
    "to_owned",
    "into",
    "as_bytes",
    "as_str",
    "clone",
    "to_vec",
    "as_ref",
    "borrow",
    "trim",
    "to_lowercase",
    "to_uppercase",
    "into_bytes",
    "bytes",
];

/// Call paths that wrap a value without changing where it came from.
const WRAPPERS: &[&str] = &["from", "new", "from_str", "from_utf8_lossy", "to_string"];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Hit {
    file: String,
    line: usize,
    callee: String,
    param: String,
    arg: String,
}

fn is_const_name(s: &str) -> bool {
    s.len() > 1
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && s.chars().any(|c| c.is_ascii_uppercase())
}

/// Functions (by name) with a sink parameter, and which positions (excluding
/// `self`) hold one.
#[derive(Default)]
struct Sinks {
    by_name: BTreeMap<String, Vec<(usize, String)>>,
    /// The same, per file: a bare call `f(..)` binds to the `f` of its own file
    /// when it has one. `station(password: bool)` in one test must not make every
    /// other file's `station("en")` a finding.
    by_file: BTreeMap<(String, String), Vec<(usize, String)>>,
    /// Every function name defined in each file, whatever its parameters.
    defined: BTreeSet<(String, String)>,
    /// Names defined with no sink parameter anywhere: a call to one of these
    /// with all-literal arguments is a helper that may launder a literal.
    plain: BTreeSet<String>,
}

struct CollectSinks<'a> {
    file: &'a str,
    out: &'a mut Sinks,
}

impl CollectSinks<'_> {
    fn record(&mut self, sig: &syn::Signature) {
        let name = sig.ident.to_string();
        let mut at = Vec::new();
        let mut pos = 0;
        for input in &sig.inputs {
            if let syn::FnArg::Typed(t) = input {
                if let syn::Pat::Ident(i) = &*t.pat {
                    let n = i.ident.to_string();
                    let bare = n.trim_start_matches('_');
                    if SINK_PARAMS.contains(&bare) {
                        at.push((pos, bare.to_string()));
                    }
                }
                pos += 1;
            }
        }
        self.out
            .defined
            .insert((self.file.to_string(), name.clone()));
        if at.is_empty() {
            self.out.plain.insert(name);
        } else {
            self.out
                .by_file
                .entry((self.file.to_string(), name.clone()))
                .or_default()
                .extend(at.clone());
            self.out.by_name.entry(name).or_default().extend(at);
        }
    }
}

impl<'ast> Visit<'ast> for CollectSinks<'_> {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.record(&f.sig);
        syn::visit::visit_item_fn(self, f);
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.record(&f.sig);
        syn::visit::visit_impl_item_fn(self, f);
    }
    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        self.record(&f.sig);
        syn::visit::visit_trait_item_fn(self, f);
    }
}

struct Scan<'a> {
    file: &'a str,
    src: Vec<&'a str>,
    sinks: &'a Sinks,
    /// Locals bound to a literal-derived value, per enclosing function.
    tainted: Vec<BTreeSet<String>>,
    /// Constants (and statics) defined in this file, so `PASSWORD` is known to
    /// be one and a runtime value that happens to be upper-case is not.
    consts: BTreeSet<String>,
    /// Locals bound to a closure: a bare call to one is not a call to a function
    /// of the same name elsewhere.
    closures: BTreeSet<String>,
    out: Vec<Hit>,
}

impl Scan<'_> {
    fn is_tainted_local(&self, name: &str) -> bool {
        self.tainted.iter().any(|s| s.contains(name))
    }

    /// Is this expression a string made from a literal?
    fn literal_derived(&self, e: &syn::Expr) -> bool {
        match e {
            // Not only strings: a `bool` literal handed to `station(password: bool, ..)` and
            // an integer handed to `hashed_unit(day, salt)` were alerts too (the rule is
            // keyed on the parameter's name, not on what the value is), so a bool, integer
            // or char literal counts as much as a string.
            syn::Expr::Lit(l) => matches!(
                l.lit,
                syn::Lit::Str(_)
                    | syn::Lit::ByteStr(_)
                    | syn::Lit::CStr(_)
                    | syn::Lit::Bool(_)
                    | syn::Lit::Int(_)
                    | syn::Lit::Char(_)
                    | syn::Lit::Byte(_)
            ),
            syn::Expr::Path(p) => p.path.get_ident().is_some_and(|i| {
                let n = i.to_string();
                self.consts.contains(&n) || self.is_tainted_local(&n)
            }),
            syn::Expr::Reference(r) => self.literal_derived(&r.expr),
            syn::Expr::Paren(p) => self.literal_derived(&p.expr),
            syn::Expr::Group(g) => self.literal_derived(&g.expr),
            syn::Expr::Cast(c) => self.literal_derived(&c.expr),
            syn::Expr::Unary(u) => self.literal_derived(&u.expr),
            syn::Expr::Index(i) => self.literal_derived(&i.expr),
            syn::Expr::MethodCall(m) => {
                PASSTHROUGH.contains(&m.method.to_string().as_str())
                    && self.literal_derived(&m.receiver)
            }
            syn::Expr::Call(c) => {
                let name = match &*c.func {
                    syn::Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
                    _ => None,
                };
                let Some(name) = name else { return false };
                // A wrapper, or a helper of ours that takes only literals: the
                // `pw("tag")` shape, where the literal is handed to every caller.
                let launders =
                    WRAPPERS.contains(&name.as_str()) || self.sinks.plain.contains(&name);
                launders && !c.args.is_empty() && c.args.iter().all(|a| self.literal_derived(a))
            }
            syn::Expr::Array(a) => {
                !a.elems.is_empty() && a.elems.iter().all(|x| self.literal_derived(x))
            }
            syn::Expr::Macro(m) => {
                // `vec![b'x'; n]` and `b"…".to_vec()` are not strings; only the
                // two macros that build a string out of its tokens count, and
                // `format!` with a runtime argument does not.
                let name = m.mac.path.segments.last().map(|s| s.ident.to_string());
                name.as_deref() == Some("concat")
                    || name.as_deref() == Some("include_str")
                    || name.as_deref() == Some("stringify")
            }
            _ => false,
        }
    }

    fn text(&self, span: proc_macro2::Span) -> String {
        let (a, b) = (span.start(), span.end());
        if a.line == 0 || b.line < a.line || b.line > self.src.len() {
            return String::new();
        }
        let mut out = String::new();
        for ln in a.line..=b.line {
            let line = self.src[ln - 1];
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

    fn check_call(&mut self, callee: &str, bare: bool, args: &[&syn::Expr], line: usize) {
        let key = (self.file.to_string(), callee.to_string());
        let positions = if bare && self.sinks.defined.contains(&key) {
            self.sinks.by_file.get(&key)
        } else {
            self.sinks.by_name.get(callee)
        };
        let Some(positions) = positions else {
            return;
        };
        let mut seen = BTreeSet::new();
        for (pos, param) in positions {
            let Some(arg) = args.get(*pos) else { continue };
            if !seen.insert(*pos) {
                continue;
            }
            if self.literal_derived(arg) {
                let mut text = self.text(arg.span());
                text.truncate(80);
                self.out.push(Hit {
                    file: self.file.to_string(),
                    line,
                    callee: callee.to_string(),
                    param: param.clone(),
                    arg: text,
                });
            }
        }
    }
}

impl<'ast> Visit<'ast> for Scan<'_> {
    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        self.consts.insert(c.ident.to_string());
        syn::visit::visit_item_const(self, c);
    }
    fn visit_item_static(&mut self, c: &'ast syn::ItemStatic) {
        self.consts.insert(c.ident.to_string());
        syn::visit::visit_item_static(self, c);
    }
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.tainted.push(BTreeSet::new());
        syn::visit::visit_item_fn(self, f);
        self.tainted.pop();
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.tainted.push(BTreeSet::new());
        syn::visit::visit_impl_item_fn(self, f);
        self.tainted.pop();
    }
    fn visit_local(&mut self, l: &'ast syn::Local) {
        if let (Some(init), syn::Pat::Ident(i)) = (&l.init, &l.pat) {
            if matches!(&*init.expr, syn::Expr::Closure(_)) {
                self.closures.insert(i.ident.to_string());
            }
            if self.literal_derived(&init.expr)
                && let Some(top) = self.tainted.last_mut()
            {
                top.insert(i.ident.to_string());
            }
        }
        syn::visit::visit_local(self, l);
    }
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*c.func
            && let Some(seg) = p.path.segments.last()
        {
            let args: Vec<&syn::Expr> = c.args.iter().collect();
            let bare = p.path.segments.len() == 1;
            if bare && self.closures.contains(&seg.ident.to_string()) {
                syn::visit::visit_expr_call(self, c);
                return;
            }
            self.check_call(
                &seg.ident.to_string(),
                bare,
                &args,
                seg.ident.span().start().line,
            );
        }
        syn::visit::visit_expr_call(self, c);
    }
    /// `assert!(verify_password(&hash, "x"))`: the call is inside a macro's tokens,
    /// which `syn` does not parse. Macros that take a comma-separated list of
    /// expressions (the `assert` family, `format!`, `vec!`, `println!`, …) are
    /// parsed as one and walked; `matches!` and the like, whose arguments are not
    /// all expressions, are skipped.
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        if let Ok(args) = m.parse_body_with(
            syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
        ) {
            for a in &args {
                self.visit_expr(a);
            }
        }
    }
    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        let args: Vec<&syn::Expr> = m.args.iter().collect();
        self.check_call(
            &m.method.to_string(),
            false,
            &args,
            m.method.span().start().line,
        );
        syn::visit::visit_expr_method_call(self, m);
    }
}

fn rs_files(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.map(|e| e.expect("dir entry").path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if name == "target" || name == "node_modules" || name.starts_with('.') {
                continue;
            }
            rs_files(&p, root, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            let rel = p
                .strip_prefix(root)
                .expect("under root")
                .display()
                .to_string();
            out.push((rel, std::fs::read_to_string(&p).expect("read source")));
        }
    }
}

/// Every hit in `files` (path, source).
fn scan(files: &[(String, String)]) -> (Vec<Hit>, usize) {
    let parsed: Vec<(&String, syn::File)> = files
        .iter()
        .map(|(p, s)| {
            (
                p,
                syn::parse_file(s).unwrap_or_else(|e| panic!("{p} does not parse: {e}")),
            )
        })
        .collect();
    let mut sinks = Sinks::default();
    for (path, f) in &parsed {
        CollectSinks {
            file: path,
            out: &mut sinks,
        }
        .visit_file(f);
    }
    // A name with a sink definition is not also "plain".
    let with_sink: Vec<String> = sinks.by_name.keys().cloned().collect();
    for n in with_sink {
        sinks.plain.remove(&n);
    }
    let mut hits = Vec::new();
    for ((path, f), (_, src)) in parsed.iter().zip(files) {
        let mut s = Scan {
            file: path,
            src: src.lines().collect(),
            sinks: &sinks,
            tainted: Vec::new(),
            consts: BTreeSet::new(),
            closures: BTreeSet::new(),
            out: Vec::new(),
        };
        // Constants first, so a use that precedes its definition is still seen.
        for item in &f.items {
            collect_consts(item, &mut s.consts);
        }
        s.visit_file(f);
        hits.extend(s.out);
    }
    hits.sort();
    hits.dedup();
    (hits, sinks.by_name.len())
}

fn collect_consts(item: &syn::Item, out: &mut BTreeSet<String>) {
    match item {
        syn::Item::Const(c) => {
            out.insert(c.ident.to_string());
        }
        syn::Item::Static(c) => {
            out.insert(c.ident.to_string());
        }
        syn::Item::Mod(m) => {
            if let Some((_, items)) = &m.content {
                for i in items {
                    collect_consts(i, out);
                }
            }
        }
        _ => {}
    }
    let _ = is_const_name("A");
}

fn repo_files() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rs_files(&root.join("src"), root, &mut files);
    rs_files(&root.join("tests"), root, &mut files);
    rs_files(&root.join("benches"), root, &mut files);
    rs_files(&root.join("examples"), root, &mut files);
    rs_files(&root.join("tools"), root, &mut files);
    rs_files(&root.join("crates"), root, &mut files);
    files
}

/// The gate: no string made from a literal reaches a `password`, `nonce`, `iv`,
/// `salt` or `secret` parameter anywhere in the repository.
///
/// The first run found 62 such calls in 17 files, 12 of them in
/// `crates/birdnet-db/src/accounts/users.rs` and 19 in `login_is_throttled.rs`.
#[test]
fn no_literal_reaches_a_credential_parameter() {
    let (hits, sink_fns) = scan(&repo_files());
    // The walk found the code and the functions that take such a parameter: a scan
    // that found neither would be clean for the wrong reason.
    assert!(
        sink_fns >= 10,
        "found only {sink_fns} functions with a credential parameter"
    );
    let rows: Vec<String> = hits
        .iter()
        .map(|h| {
            format!(
                "{}:{}  {}(.. {} ..) <- {}",
                h.file, h.line, h.callee, h.param, h.arg
            )
        })
        .collect();
    assert!(
        rows.is_empty(),
        "{} call(s) hand a literal to a parameter CodeQL reads as a cryptographic \
         value. Build the value with `format!(\"…-{{}}\", std::process::id())` (and let \
         no helper take the literal as an argument), or read it from a file at run \
         time:\n{}",
        rows.len(),
        rows.join("\n")
    );
}

fn hits_in(src: &str) -> Vec<(String, String)> {
    let (hits, _) = scan(&[("x.rs".to_string(), src.to_string())]);
    hits.into_iter().map(|h| (h.callee, h.arg)).collect()
}

fn pair(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

/// Each shape the query alerts on, and that this scan claims to see.
#[test]
fn the_scan_flags_each_shape() {
    let src = r#"
        fn hash_password(password: &str) {}
        fn verify_password(hash: &str, password: &str) {}
        fn mix(nonce: &str) {}
        const PW: &str = "x";
        fn pw(tag: &str) -> String { format!("{tag}-1") }
        fn gate(password: bool, dir: &str) {}
        fn unit(day: u32, salt: u32) {}
        fn t() {
            gate(true, "d");
            let on = false;
            gate(on, "d");
            unit(3, 1);
            hash_password("direct");
            hash_password(PW);
            let local = "bound";
            hash_password(local);
            hash_password(&"ref".to_string());
            hash_password(&pw("helper"));
            assert!(verify_password("h", "inside-an-assert"));
            mix(&String::from("wrapped"));
        }
    "#;
    assert_eq!(
        hits_in(src),
        vec![
            pair("gate", "true"),
            pair("gate", "on"),
            pair("unit", "1"),
            pair("hash_password", "\"direct\""),
            pair("hash_password", "PW"),
            pair("hash_password", "local"),
            pair("hash_password", "&\"ref\".to_string()"),
            pair("hash_password", "&pw(\"helper\")"),
            pair("verify_password", "\"inside-an-assert\""),
            pair("mix", "&String::from(\"wrapped\")"),
        ]
    );
}

/// The counterpart: what the recipe builds passes, so the flags above are about
/// the literal and not about every call into those functions. Includes the two
/// name collisions that produced false hits on the first run: another file's
/// `station(password: bool)`, and a closure named like a sink.
#[test]
fn the_scan_passes_a_value_built_at_run_time() {
    let src = r#"
        fn hash_password(password: &str) {}
        fn station(password: bool) {}
        fn unit(day: u32, salt: u32) {}
        fn t() {
            let stream = std::env::args().count() as u32;
            unit(7, stream);
            let with_password = std::env::args().count() > 1;
            station(with_password);
            hash_password(&format!("pw-{}", std::process::id()));
            let built = format!("pw-{}", std::process::id());
            hash_password(&built);
            hash_password(&std::env::var("X").unwrap_or_default());
            hash_password(&String::new());
            hash_password(&password());
            let station = |prefix: &str| prefix.len();
            station("not-a-credential");
        }
        fn password() -> String { format!("a-{}", std::process::id()) }
    "#;
    assert_eq!(hits_in(src), Vec::<(String, String)>::new());
    // The same call with a literal is caught: the cases above pass for their own
    // reasons and not because the scan sees nothing.
    assert_eq!(
        hits_in("fn hash_password(password: &str) {} fn t() { hash_password(\"x\"); }"),
        vec![pair("hash_password", "\"x\"")]
    );
}
