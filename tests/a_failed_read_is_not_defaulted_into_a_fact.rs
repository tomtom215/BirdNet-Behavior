//! A failed read is not defaulted into a fact.
//!
//! The defect: a database (or analytics) read fails, the code swallows the
//! error *inside* the closure that made the read — `.unwrap_or_default()`,
//! `.unwrap_or(0)`, `.ok()`, `if let Ok(..)`, `Err(_) => <empty>` — and the
//! default is then printed or decided on as if the station had measured it.
//! The species page said "No recording captured for this species yet" for a
//! bird recorded a hundred times; the Today page served the first-run setup to a
//! station with years of records (CLAUDE.md, "Testing Conventions").
//!
//! The earlier sweeps were scripts kept in a session's scratch directory, and the
//! last of them found 188 hit-lines and was lost with the session. This is the
//! same sweep, kept: a `syn` parse of `src/` and `crates/birdnet-web/src/` that
//! finds every *defaulting adaptor applied to something derived from a read*,
//! and a table (`tests/data/read_default_sweep.tsv`) in which a human has put
//! every one of them in a class. A new hit with no row fails; a row that
//! matches nothing fails; and the class `real` is not allowed to be in the
//! table at all, because a real one is fixed, not catalogued.
//!
//! # What counts as "derived from a read"
//!
//! The receiver of the adaptor contains a call that is handed a connection
//! (a closure parameter of `with_db` / `with_read_db` / `with_analytics` /
//! `with_timeseries` / `run_blocking*` / `spawn_blocking` / `cached_fragment`,
//! or a function parameter typed `Connection` / `Transaction`), or contains a
//! call to one of those primitives itself (so `.await.unwrap_or(..)` on a
//! `run_blocking` result is seen: the task-panic class).
//!
//! # What it cannot see
//!
//! It works by name, not by type. A read made through a struct field that holds
//! a connection, through a macro, or in a helper that is handed a value read
//! earlier and not the connection, is not seen. The counterpart tests below show
//! each shape it claims to flag and each it claims to pass.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use proc_macro2::LineColumn;
use syn::spanned::Spanned;
use syn::visit::Visit;

/// Calls that hand a closure the connection (or run it on another thread).
const READ_PRIMITIVES: &[&str] = &[
    "with_db",
    "with_read_db",
    "with_analytics",
    "with_timeseries",
    "run_blocking",
    "run_blocking_heavy",
    "spawn_blocking",
    "cached_fragment",
];

/// Method names that turn an error into a value.
const DEFAULTERS: &[&str] = &[
    "unwrap_or_default",
    "unwrap_or",
    "unwrap_or_else",
    "ok",
    "map_or",
    "map_or_else",
    "is_ok_and",
    "is_ok",
    "is_err",
];

/// The classes a human may give a hit. `real` is deliberately absent: a defect
/// the sweep finds is fixed, and the row that would have excused it never exists.
///
/// `tracked` is the one class that is not "this is fine": it is a defect that is
/// known, has evidence, and is not fixed in the change that wrote the row. Each
/// one names a row of `docs/TECH_DEBT.md`, and a tracked row is a debt, not a
/// verdict.
const CLASSES: &[&str] = &[
    // The error is not lost: it is logged and shown, or the caller refuses.
    "handled",
    // The default decorates (a sparkline, a hint, a sort key, a missing picture)
    // and states nothing about the station.
    "decoration",
    // The fallible step is a conversion, a clock, a file or a task, not a read of
    // the station's data, and nothing claims otherwise.
    "not-a-read",
    // The default is the safe side: denied, off, refused, "already set up".
    "fail-closed",
    // A setting that falls back to its documented default. A failure runs the
    // station as if the operator had not set it, and the settings page, which
    // refuses loudly, is where it is corrected.
    "setting-default",
    // An exposition or a strip that omits the series or the section rather than
    // printing a zero or an all-clear.
    "omitted",
    // A task that panicked, not a query that failed. A panic is already loud.
    "task-panic",
    // The zero is true: a legitimately empty answer, or a lookup in a map.
    "legit-zero",
    // A write whose outcome is carried on another path (a count of successes, an
    // audit row, a log line).
    "write-outcome",
    // A cache or probe: the read is an optimisation and its failure is inert.
    "cache",
    // A defect that is known, evidenced and not yet fixed; see the class note.
    "tracked",
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Hit {
    file: String,
    func: String,
    kind: String,
    snippet: String,
    line: usize,
    /// For a human reading the failure; not part of the row's identity.
    detail: String,
}

impl Hit {
    fn key(&self) -> (String, String, String, String) {
        (
            self.file.clone(),
            self.func.clone(),
            self.kind.clone(),
            self.snippet.clone(),
        )
    }
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

/// The source text a span covers (columns count characters, as proc-macro2's do).
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
    out
}

fn type_is_connection(src: &[&str], ty: &syn::Type) -> bool {
    let t = text_of(src, ty.span());
    t.contains("Connection") || t.contains("Transaction")
}

fn pat_idents(p: &syn::Pat, out: &mut Vec<String>) {
    match p {
        syn::Pat::Ident(i) => out.push(i.ident.to_string()),
        syn::Pat::Reference(r) => pat_idents(&r.pat, out),
        syn::Pat::Type(t) => pat_idents(&t.pat, out),
        syn::Pat::Tuple(t) => t.elems.iter().for_each(|e| pat_idents(e, out)),
        _ => {}
    }
}

/// Does this expression contain a call that is handed one of `conns`, or a
/// read primitive?
struct MentionsRead<'a> {
    conns: &'a BTreeSet<String>,
    found: bool,
}

fn is_conn_arg(e: &syn::Expr, conns: &BTreeSet<String>) -> bool {
    match e {
        syn::Expr::Path(p) => p
            .path
            .get_ident()
            .is_some_and(|i| conns.contains(&i.to_string())),
        syn::Expr::Reference(r) => is_conn_arg(&r.expr, conns),
        syn::Expr::Paren(p) => is_conn_arg(&p.expr, conns),
        _ => false,
    }
}

impl<'ast> Visit<'ast> for MentionsRead<'_> {
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        if c.args.iter().any(|a| is_conn_arg(a, self.conns)) {
            self.found = true;
        }
        if let syn::Expr::Path(p) = &*c.func {
            if let Some(last) = p.path.segments.last() {
                if READ_PRIMITIVES.contains(&last.ident.to_string().as_str()) {
                    self.found = true;
                }
                // `AudioSourceStore::list` passed as a path to a primitive is a
                // read, but that is the primitive's own call; nothing to add.
            }
        }
        syn::visit::visit_expr_call(self, c);
    }
    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        if is_conn_arg(&m.receiver, self.conns)
            || m.args.iter().any(|a| is_conn_arg(a, self.conns))
            || READ_PRIMITIVES.contains(&m.method.to_string().as_str())
        {
            self.found = true;
        }
        syn::visit::visit_expr_method_call(self, m);
    }
}

fn mentions_read(e: &syn::Expr, conns: &BTreeSet<String>) -> bool {
    let mut m = MentionsRead {
        conns,
        found: false,
    };
    m.visit_expr(e);
    m.found
}

struct Sweep<'a> {
    file: &'a str,
    funcs: Vec<String>,
    in_test: usize,
    conns: BTreeSet<String>,
    src_lines: Vec<&'a str>,
    seen: usize,
    out: Vec<Hit>,
}

/// The text of a hit as its identity in the table: comments dropped (an edit to a
/// comment inside the expression is not a new site) and whitespace collapsed.
fn squash(s: &str) -> String {
    let code: Vec<&str> = s
        .lines()
        .map(|l| l.split("//").next().unwrap_or(l))
        .collect();
    let mut t = code
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if t.len() > 110 {
        let mut end = 110;
        while !t.is_char_boundary(end) {
            end -= 1;
        }
        t.truncate(end);
    }
    t
}

impl Sweep<'_> {
    fn func(&self) -> String {
        self.funcs.last().cloned().unwrap_or_default()
    }
    fn push(&mut self, kind: &str, tokens: String, at: LineColumn, detail: String) {
        self.out.push(Hit {
            detail,
            file: self.file.to_string(),
            func: self.func(),
            kind: kind.to_string(),
            snippet: squash(&tokens),
            line: at.line,
        });
    }
    fn enter_fn(&mut self, name: String, sig: &syn::Signature) -> BTreeSet<String> {
        self.funcs.push(name);
        self.seen += 1;
        let saved = self.conns.clone();
        for input in &sig.inputs {
            if let syn::FnArg::Typed(t) = input {
                if type_is_connection(&self.src_lines, &t.ty) {
                    let mut v = Vec::new();
                    pat_idents(&t.pat, &mut v);
                    self.conns.extend(v);
                }
            }
        }
        saved
    }
}

impl<'ast> Visit<'ast> for Sweep<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        let test = m.attrs.iter().any(is_test_attr);
        self.in_test += usize::from(test);
        syn::visit::visit_item_mod(self, m);
        self.in_test -= usize::from(test);
    }
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if self.in_test > 0 || f.attrs.iter().any(is_test_attr) {
            return;
        }
        let saved = self.enter_fn(f.sig.ident.to_string(), &f.sig);
        syn::visit::visit_item_fn(self, f);
        self.conns = saved;
        self.funcs.pop();
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if self.in_test > 0 || f.attrs.iter().any(is_test_attr) {
            return;
        }
        let saved = self.enter_fn(f.sig.ident.to_string(), &f.sig);
        syn::visit::visit_impl_item_fn(self, f);
        self.conns = saved;
        self.funcs.pop();
    }
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        let prim = matches!(&*c.func, syn::Expr::Path(p)
            if p.path.segments.last().is_some_and(|s| READ_PRIMITIVES.contains(&s.ident.to_string().as_str())));
        self.visit_expr(&c.func);
        for a in &c.args {
            self.visit_read_closure(a, prim);
        }
    }
    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        let name = m.method.to_string();
        if DEFAULTERS.contains(&name.as_str()) && mentions_read(&m.receiver, &self.conns) {
            let t = text_of(&self.src_lines, m.span());
            self.push(&name, t, m.method.span().start(), String::new());
        }
        let prim = READ_PRIMITIVES.contains(&name.as_str());
        self.visit_expr(&m.receiver);
        for a in &m.args {
            self.visit_read_closure(a, prim);
        }
    }
    fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
        if let syn::Expr::Let(l) = &*i.cond {
            self.check_let(&l.pat, &l.expr, l.span().start());
        }
        syn::visit::visit_expr_if(self, i);
    }
    fn visit_local(&mut self, l: &'ast syn::Local) {
        if let Some(init) = &l.init {
            if init.diverge.is_some() {
                self.check_let(&l.pat, &init.expr, l.span().start());
            }
        }
        syn::visit::visit_local(self, l);
    }
    fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
        if mentions_read(&m.expr, &self.conns) {
            for arm in &m.arms {
                if pat_is_err(&arm.pat) {
                    self.push(
                        "match-err",
                        format!(
                            "match {} {{ Err .. }}",
                            text_of(&self.src_lines, m.expr.span())
                        ),
                        arm.span().start(),
                        text_of(&self.src_lines, arm.body.span()),
                    );
                }
            }
        }
        syn::visit::visit_expr_match(self, m);
    }
}

fn pat_is_err(p: &syn::Pat) -> bool {
    match p {
        syn::Pat::TupleStruct(t) => t.path.is_ident("Err"),
        syn::Pat::Or(o) => o.cases.iter().any(pat_is_err),
        _ => false,
    }
}

fn pat_is_ok(p: &syn::Pat) -> bool {
    matches!(p, syn::Pat::TupleStruct(t) if t.path.is_ident("Ok"))
}

impl Sweep<'_> {
    fn check_let(&mut self, pat: &syn::Pat, expr: &syn::Expr, at: LineColumn) {
        if pat_is_ok(pat) && mentions_read(expr, &self.conns) {
            self.push(
                "let-ok",
                format!("let Ok(..) = {}", text_of(&self.src_lines, expr.span())),
                at,
                String::new(),
            );
        }
    }
    /// An argument of a read primitive that is a sync closure binds its
    /// parameters as connections for the closure's body.
    fn visit_read_closure(&mut self, a: &syn::Expr, prim: bool) {
        if let (true, syn::Expr::Closure(c)) = (prim, a) {
            let saved = self.conns.clone();
            for p in &c.inputs {
                let mut v = Vec::new();
                pat_idents(p, &mut v);
                self.conns.extend(v);
            }
            self.visit_expr(&c.body);
            self.conns = saved;
        } else {
            self.visit_expr(a);
        }
    }
}

fn sweep(files: &[(String, String)]) -> (Vec<Hit>, usize) {
    let mut hits = Vec::new();
    let mut fns = 0usize;
    for (path, src) in files {
        let parsed = syn::parse_file(src).unwrap_or_else(|e| panic!("{path} does not parse: {e}"));
        let mut s = Sweep {
            file: path,
            funcs: Vec::new(),
            in_test: 0,
            conns: BTreeSet::new(),
            src_lines: src.lines().collect(),
            seen: 0,
            out: Vec::new(),
        };
        s.visit_file(&parsed);
        hits.extend(s.out);
        fns += s.seen;
    }
    hits.sort();
    hits.dedup_by(|a, b| a.key() == b.key());
    (hits, fns)
}

fn rs_files(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
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

fn tree() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rs_files(&root.join("src"), root, &mut files);
    rs_files(&root.join("crates/birdnet-web/src"), root, &mut files);
    files
}

/// `class<TAB>file<TAB>fn<TAB>kind<TAB>snippet`, one per line; `#` comments.
fn table() -> BTreeMap<(String, String, String, String), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text =
        std::fs::read_to_string(root.join("tests/data/read_default_sweep.tsv")).unwrap_or_default();
    let mut out = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.splitn(5, '\t').collect();
        assert!(
            f.len() == 5,
            "read_default_sweep.tsv:{}: want 5 fields",
            n + 1
        );
        let class = f[0].split(':').next().unwrap_or("");
        assert!(
            CLASSES.contains(&class),
            "read_default_sweep.tsv:{}: class {:?} is not one of {CLASSES:?}; \
             a real defect is fixed, not catalogued",
            n + 1,
            f[0]
        );
        // A tracked row names the row of docs/TECH_DEBT.md that carries its evidence.
        if class == "tracked" {
            let id = f[0].strip_prefix("tracked:").unwrap_or("");
            let debt = std::fs::read_to_string(root.join("docs/TECH_DEBT.md")).unwrap_or_default();
            assert!(
                !id.is_empty() && debt.contains(&format!("\n## {id} ")),
                "read_default_sweep.tsv:{}: `tracked` must be `tracked:TD-n` for a \
                 `## TD-n` heading in docs/TECH_DEBT.md; got {:?}",
                n + 1,
                f[0]
            );
        }
        let key = (f[1].into(), f[2].into(), f[3].into(), f[4].into());
        assert!(
            out.insert(key, f[0].to_string()).is_none(),
            "read_default_sweep.tsv:{}: duplicate row",
            n + 1
        );
    }
    out
}

/// The gate: every defaulting adaptor on a read is in the table, and every row of
/// the table still matches one.
#[test]
fn every_read_that_is_defaulted_has_been_looked_at() {
    let (hits, fns) = sweep(&tree());
    // The walk found the code: a sweep of nothing is clean.
    assert!(fns > 2000, "swept only {fns} functions");
    let table = table();
    assert!(
        table.len() > 100,
        "the table has {} rows; it has probably been truncated",
        table.len()
    );

    let mut used = BTreeSet::new();
    let mut new = Vec::new();
    for h in &hits {
        if table.contains_key(&h.key()) {
            used.insert(h.key());
        } else {
            new.push(format!(
                "?\t{}\t{}\t{}\t{}    (line {})",
                h.file, h.func, h.kind, h.snippet, h.line
            ));
        }
    }
    assert!(
        new.is_empty(),
        "{} read(s) are turned into a value inside the closure that made them, \
         and no one has said that is all right. Fix each one so a failure is not \
         published as a fact (see `error_states` and \
         `a_read_the_sweep_found_is_not_published_as_fact.rs`), or classify it in \
         tests/data/read_default_sweep.tsv (class one of {CLASSES:?}):\n{}",
        new.len(),
        new.join("\n")
    );
    let stale: Vec<_> = table
        .keys()
        .filter(|k| !used.contains(*k))
        .map(|k| format!("{}  {}  {}  {}", k.0, k.1, k.2, k.3))
        .collect();
    assert!(
        stale.is_empty(),
        "rows of tests/data/read_default_sweep.tsv that match nothing now (a fix \
         removed the code they described, or an edit changed its text); delete \
         them:\n{}",
        stale.join("\n")
    );
}

fn hits_in(src: &str) -> Vec<(String, String)> {
    let (hits, _) = sweep(&[("x.rs".to_string(), src.to_string())]);
    hits.into_iter().map(|h| (h.func, h.kind)).collect()
}

fn pair(f: &str, k: &str) -> (String, String) {
    (f.to_string(), k.to_string())
}

/// The shapes the sweep exists for. Each is a read whose failure became a value.
#[test]
fn the_sweep_flags_each_shape() {
    let src = r#"
        fn closure_param(s: &S) -> Vec<u8> { s.with_read_db(|conn| list(conn).unwrap_or_default()) }
        fn function_param(conn: &Connection) -> Option<u8> { count(conn).ok() }
        fn if_let(conn: &Connection) { if let Ok(n) = count(conn) { use_it(n); } }
        fn let_else(conn: &Connection) { let Ok(n) = count(conn) else { return; }; }
        fn match_err(s: &S) { match s.with_db(|c| write(c)) { Ok(_) => {}, Err(_) => {} } }
        async fn task_default(s: S) -> u8 {
            tokio::task::spawn_blocking(move || s.with_read_db(|c| n(c))).await.unwrap_or(0)
        }
        fn map_or(conn: &Connection) -> bool { count(conn).map_or(true, |n| n > 0) }
    "#;
    // Sorted as the sweep sorts: by function name.
    assert_eq!(
        hits_in(src),
        vec![
            pair("closure_param", "unwrap_or_default"),
            pair("function_param", "ok"),
            pair("if_let", "let-ok"),
            pair("let_else", "let-ok"),
            pair("map_or", "map_or"),
            pair("match_err", "match-err"),
            pair("task_default", "unwrap_or"),
        ]
    );
}

/// The counterpart: what is not this defect passes, so the flags above are
/// about the read and not about every `unwrap_or`.
#[test]
fn the_sweep_passes_what_is_not_a_defaulted_read() {
    let src = r#"
        // Propagated, not defaulted.
        fn propagated(conn: &Connection) -> Result<u8, E> { let n = count(conn)?; Ok(n) }
        // A request parameter: nothing was read.
        fn parameter(q: Q) -> u32 { q.limit.unwrap_or(50) }
        // A file: no connection reaches it.
        fn file(p: &Path) -> String { std::fs::read_to_string(p).unwrap_or_default() }
        // Mapped, with the error kept.
        fn kept(conn: &Connection) -> Result<u8, String> { count(conn).map_err(|e| e.to_string()) }
        // Test code is not shipped code.
        #[cfg(test)] mod tests { fn t(conn: &Connection) { count(conn).ok(); } }
        #[test] fn also_a_test(conn: &Connection) { count(conn).ok(); }
    "#;
    assert_eq!(hits_in(src), Vec::<(String, String)>::new());
    // The same read, defaulted, is caught: the cases above pass for their own
    // reasons and not because the sweep sees nothing.
    assert_eq!(
        hits_in("fn f(conn: &Connection) -> u8 { count(conn).unwrap_or(0) }"),
        vec![pair("f", "unwrap_or")]
    );
}
