//! Database work stays off the async runtime.
//!
//! `AppState::with_db`, `with_read_db`, `with_analytics` and their relatives
//! block — on a mutex the detection processor also takes, then on the query.
//! Called from an `async fn`, that wait holds one of the runtime's worker
//! threads (four on a Raspberry Pi 4), and every request and live socket
//! scheduled behind it waits too. This gate's first run, in October 2026,
//! found 170 such call sites beyond startup: 158 in `birdnet-web` and 12 in
//! this binary's background tasks. Only 45 of them called one of these
//! accessors directly; the rest went through a sync helper — `audit()`,
//! alone, from 43 handlers.
//!
//! So this is a parse, not a grep. It finds every function that reaches one of
//! those primitives without crossing a blocking boundary, follows calls through
//! them to a fixed point, and fails on any `async fn` (or `async` block) that
//! reaches one directly.
//!
//! # What it can and cannot see
//!
//! Calls are resolved by name, not by type. A method call matches only
//! methods, `Type::f` only associated functions, and a bare `f(..)` the same
//! file's `f` before any other. A name is treated as blocking only when
//! *every* definition of it is, so a blocking `record` in one file does not
//! condemn every `record`. That rule can miss a blocking helper that shares a
//! name with an innocent one; it does not report a call that cannot block.
//! The counterpart tests below show it flags each shape it claims to and
//! passes each boundary it honours.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;
use syn::visit::Visit;

/// The accessors that do the blocking.
const PRIMITIVES: &[&str] = &[
    "with_db",
    "with_read_db",
    "with_analytics",
    "with_timeseries",
    "resync_analytics_full",
    "mirror_recent_recording_effort",
];

/// Calls whose closure arguments run on another thread.
///
/// `cached_fragment` is here because it runs its `compute` closure under
/// `spawn_blocking` itself (`birdnet-web/src/analytics_cache.rs`).
const BOUNDARIES: &[&str] = &[
    "spawn_blocking",
    "block_in_place",
    "run_blocking",
    "cached_fragment",
];

/// `(async fn, callee)` pairs allowed to block, each with the reason.
///
/// Startup only: these run once, in order, before the listener binds, and
/// nothing they could delay is serving a reader yet. A call added to `serve`
/// that is not on this list still fails.
const EXEMPT: &[(&str, &str, &str)] = &[
    ("src/app.rs", "serve", "record_boot"),
    ("src/app.rs", "serve", "bootstrap_admin_password"),
    ("src/app.rs", "serve", "purge_legacy_credential_settings"),
    ("src/app.rs", "serve", "seed_db_settings_from_config"),
    ("src/app.rs", "serve", "overlay_db_settings"),
    ("src/app.rs", "serve", "create_email_notifier"),
    ("src/app.rs", "serve", "start_disk_manager"),
    ("src/app.rs", "serve", "start_capture_manager"),
    ("src/app.rs", "serve", "start_detection_daemon"),
    ("src/app.rs", "serve", "admin_password_configured"),
];

struct FnDef {
    file: String,
    name: String,
    is_async: bool,
    keys: Vec<String>,
    body: syn::Block,
}

/// One call that can block, made outside any boundary.
#[derive(Debug, PartialEq, Eq)]
struct Hit {
    file: String,
    line: usize,
    caller: String,
    callee: String,
    in_async: bool,
}

fn is_test_attr(a: &syn::Attribute) -> bool {
    if a.path().is_ident("test") {
        return true;
    }
    // `cfg(test)` and `cfg(all(test, ..))`, the two forms the tree uses; not a
    // doc comment that happens to say "test".
    a.path().is_ident("cfg")
        && a.meta.require_list().is_ok_and(|l| {
            let t = l.tokens.to_string();
            t == "test" || t.starts_with("all (test ,")
        })
}

struct Collect {
    file: String,
    in_test: usize,
    out: Vec<FnDef>,
}

impl<'ast> Visit<'ast> for Collect {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        let test = m.attrs.iter().any(is_test_attr);
        self.in_test += usize::from(test);
        syn::visit::visit_item_mod(self, m);
        self.in_test -= usize::from(test);
    }
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if self.in_test == 0 && !f.attrs.iter().any(is_test_attr) {
            let n = f.sig.ident.to_string();
            self.out.push(FnDef {
                file: self.file.clone(),
                keys: vec![format!("f:{n}"), format!("f:{n}@{}", self.file)],
                name: n,
                is_async: f.sig.asyncness.is_some(),
                body: (*f.block).clone(),
            });
        }
        syn::visit::visit_item_fn(self, f);
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if self.in_test == 0 && !f.attrs.iter().any(is_test_attr) {
            let n = f.sig.ident.to_string();
            let mut keys = vec![format!("a:{n}")];
            if f.sig.receiver().is_some() {
                keys.push(format!("m:{n}"));
            }
            self.out.push(FnDef {
                file: self.file.clone(),
                name: n,
                is_async: f.sig.asyncness.is_some(),
                keys,
                body: f.block.clone(),
            });
        }
        syn::visit::visit_impl_item_fn(self, f);
    }
}

/// Walk one body, recording each call that can block.
struct Calls<'a> {
    file: &'a str,
    local: &'a BTreeSet<String>,
    blocking: &'a BTreeSet<String>,
    awaited: bool,
    boundary_depth: usize,
    async_depth: usize,
    /// `(callee, line, in_async)` for calls outside every boundary.
    out: Vec<(String, usize, bool)>,
}

impl Calls<'_> {
    fn record(&mut self, callee: String, line: usize) {
        if self.boundary_depth == 0 {
            self.out.push((callee, line, self.async_depth > 0));
        }
    }
}

fn path_key(func: &syn::Expr) -> Option<(String, String, bool)> {
    let syn::Expr::Path(p) = func else {
        return None;
    };
    let segs: Vec<String> = p
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    let last = segs.last()?.clone();
    let assoc = segs.len() > 1 && {
        let q = &segs[segs.len() - 2];
        q == "Self" || q.starts_with(char::is_uppercase)
    };
    let kind = if assoc { "a" } else { "f" };
    Some((last.clone(), format!("{kind}:{last}"), segs.len() == 1))
}

fn has_sync_closure(args: &syn::punctuated::Punctuated<syn::Expr, syn::Token![,]>) -> bool {
    args.iter()
        .any(|a| matches!(a, syn::Expr::Closure(c) if c.asyncness.is_none()))
}

impl<'ast> Visit<'ast> for Calls<'_> {
    fn visit_expr_await(&mut self, a: &'ast syn::ExprAwait) {
        // An awaited call is an async fn, whatever else shares its name.
        self.awaited = matches!(&*a.base, syn::Expr::Call(_) | syn::Expr::MethodCall(_));
        self.visit_expr(&a.base);
        self.awaited = false;
    }
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        let awaited = std::mem::take(&mut self.awaited);
        let key = path_key(&c.func);
        if let (false, Some((name, key, single))) = (awaited, &key) {
            let local = format!("{key}@{}", self.file);
            let key = if *single && self.local.contains(&local) {
                &local
            } else {
                key
            };
            if PRIMITIVES.contains(&name.as_str()) || self.blocking.contains(key) {
                self.record(name.clone(), c.span().start().line);
            }
        }
        let name = key.as_ref().map(|k| k.0.as_str());
        let boundary = name.is_some_and(|n| BOUNDARIES.contains(&n))
            || (name == Some("spawn") && has_sync_closure(&c.args));
        self.visit_expr(&c.func);
        self.boundary_depth += usize::from(boundary);
        for a in &c.args {
            self.visit_expr(a);
        }
        self.boundary_depth -= usize::from(boundary);
    }
    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        let awaited = std::mem::take(&mut self.awaited);
        let n = m.method.to_string();
        if !awaited
            && (PRIMITIVES.contains(&n.as_str()) || self.blocking.contains(&format!("m:{n}")))
        {
            self.record(n.clone(), m.method.span().start().line);
        }
        // `thread::Builder::spawn(closure)`
        let boundary =
            BOUNDARIES.contains(&n.as_str()) || (n == "spawn" && has_sync_closure(&m.args));
        self.visit_expr(&m.receiver);
        self.boundary_depth += usize::from(boundary);
        for a in &m.args {
            self.visit_expr(a);
        }
        self.boundary_depth -= usize::from(boundary);
    }
    fn visit_expr_async(&mut self, a: &'ast syn::ExprAsync) {
        let saved = std::mem::take(&mut self.boundary_depth);
        self.async_depth += 1;
        syn::visit::visit_expr_async(self, a);
        self.async_depth -= 1;
        self.boundary_depth = saved;
    }
    fn visit_expr_closure(&mut self, c: &'ast syn::ExprClosure) {
        if c.asyncness.is_some() {
            let saved = std::mem::take(&mut self.boundary_depth);
            self.async_depth += 1;
            syn::visit::visit_expr_closure(self, c);
            self.async_depth -= 1;
            self.boundary_depth = saved;
        } else {
            syn::visit::visit_expr_closure(self, c);
        }
    }
    // Nested items are collected and scanned on their own.
    fn visit_item(&mut self, _: &'ast syn::Item) {}
}

fn calls_in(
    def: &FnDef,
    local: &BTreeSet<String>,
    blocking: &BTreeSet<String>,
) -> Vec<(String, usize, bool)> {
    let mut c = Calls {
        file: &def.file,
        local,
        blocking,
        awaited: false,
        boundary_depth: 0,
        async_depth: 0,
        out: Vec::new(),
    };
    c.visit_block(&def.body);
    c.out
}

/// Every call from async code that can block, in `files` (path, source).
fn scan(files: &[(String, String)]) -> (Vec<Hit>, usize) {
    let mut defs = Vec::new();
    for (path, src) in files {
        let parsed = syn::parse_file(src).unwrap_or_else(|e| panic!("{path} does not parse: {e}"));
        let mut c = Collect {
            file: path.clone(),
            in_test: 0,
            out: Vec::new(),
        };
        c.visit_file(&parsed);
        defs.extend(c.out);
    }
    let local: BTreeSet<String> = defs
        .iter()
        .flat_map(|d| d.keys.iter().filter(|k| k.contains('@')).cloned())
        .collect();
    let mut by_key: BTreeMap<&str, Vec<&FnDef>> = BTreeMap::new();
    for d in &defs {
        for k in &d.keys {
            by_key.entry(k).or_default().push(d);
        }
    }
    // A key is blocking when every definition behind it is a sync fn that
    // reaches a primitive, or another blocking key, outside a boundary.
    let mut blocking = BTreeSet::new();
    loop {
        let mut changed = false;
        for (key, ds) in &by_key {
            if blocking.contains(*key) {
                continue;
            }
            let every = ds
                .iter()
                .all(|d| !d.is_async && calls_in(d, &local, &blocking).iter().any(|(_, _, a)| !a));
            if every {
                blocking.insert((*key).to_string());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut hits = Vec::new();
    for d in &defs {
        for (callee, line, in_async) in calls_in(d, &local, &blocking) {
            if d.is_async || in_async {
                hits.push(Hit {
                    file: d.file.clone(),
                    line,
                    caller: d.name.clone(),
                    callee,
                    in_async: in_async && !d.is_async,
                });
            }
        }
    }
    (hits, defs.len())
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

#[test]
fn no_async_function_blocks_on_the_database() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rs_files(&root.join("src"), root, &mut files);
    rs_files(&root.join("crates/birdnet-web/src"), root, &mut files);
    let (hits, fns) = scan(&files);
    // The walk found the code: a scan of nothing is clean.
    assert!(fns > 2000, "scanned only {fns} functions");

    let mut used = BTreeSet::new();
    let offending: Vec<String> = hits
        .iter()
        .filter(|h| {
            let exempt = EXEMPT
                .iter()
                .position(|(f, c, x)| *f == h.file && *c == h.caller && *x == h.callee);
            exempt.inspect(|i| {
                used.insert(*i);
            });
            exempt.is_none()
        })
        .map(|h| {
            let ctx = if h.in_async { "async block in " } else { "" };
            format!("{}:{}  {ctx}{} -> {}", h.file, h.line, h.caller, h.callee)
        })
        .collect();
    assert!(
        offending.is_empty(),
        "{} call(s) block a runtime worker on the database; move each into \
         `state.run_blocking(..)` or `tokio::task::spawn_blocking`:\n{}",
        offending.len(),
        offending.join("\n")
    );
    let stale: Vec<_> = (0..EXEMPT.len())
        .filter(|i| !used.contains(i))
        .map(|i| EXEMPT[i])
        .collect();
    assert!(
        stale.is_empty(),
        "exemptions that no longer match anything: {stale:?}"
    );
}

fn hits_in(src: &str) -> Vec<(String, String)> {
    let (hits, _) = scan(&[("x.rs".to_string(), src.to_string())]);
    hits.into_iter().map(|h| (h.caller, h.callee)).collect()
}

fn pair(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

/// The shapes the gate exists for: a direct call, a call through a sync
/// helper (two deep), a method helper, and an `async` block in a sync fn.
#[test]
fn the_scan_flags_each_blocking_shape() {
    let src = r"
        fn audit(s: &S) { s.with_db(|c| c.x()); }
        fn wrap(s: &S) { audit(s) }
        impl S { fn lists(&self) -> u8 { self.with_read_db(|c| 1) } }
        async fn direct(s: S) { s.with_db(|c| 1); }
        async fn helper(s: S) { wrap(&s); }
        async fn method(s: S) { s.lists(); }
        fn spawner(s: S) { tokio::spawn(async move { s.with_analytics(|d| 1); }); }
    ";
    assert_eq!(
        hits_in(src),
        vec![
            pair("direct", "with_db"),
            pair("helper", "wrap"),
            pair("method", "lists"),
            pair("spawner", "with_analytics"),
        ]
    );
}

/// The counterpart: every boundary the gate honours, an awaited call that
/// shares a primitive's name, a test module, and a name only one of whose
/// definitions blocks. `audit` here is unambiguously blocking — the first test
/// shows the same helper flagged — so each line passes because of its
/// boundary, not because the helper went unrecognised.
#[test]
fn the_scan_passes_each_boundary() {
    let src = r"
        fn audit(s: &S) { s.with_db(|c| c.x()); }
        fn record(s: &S) { s.with_db(|c| c.x()); }
        mod other { pub fn record(s: &S) {} }
        async fn a(s: S) { tokio::task::spawn_blocking(move || audit(&s)).await; }
        async fn b(s: S) { s.run_blocking(|s| audit(s)).await; }
        async fn c(s: S) { cached_fragment(&s, k, F, |s| audit(s)).await; }
        async fn d(s: S) { std::thread::spawn(move || audit(&s)); }
        async fn e(s: S) { pool.with_db(q).await; }
        async fn f(s: S) { tokio::task::spawn_blocking({ let s = s.clone(); move || audit(&s) }).await; }
        async fn g(s: S) { record(&s); }
        fn h(s: S) { audit(&s); }
        #[cfg(test)] mod tests { async fn t(s: S) { s.with_db(|c| 1); } }
    ";
    assert_eq!(hits_in(src), Vec::<(String, String)>::new());
    // The same helper, unbounded, is caught: the boundaries above are what pass.
    assert_eq!(
        hits_in("fn audit(s: &S) { s.with_db(|c| c.x()); } async fn a(s: S) { audit(&s); }"),
        vec![pair("a", "audit")]
    );
}
