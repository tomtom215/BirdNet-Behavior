//! Database and filesystem work stays off the async runtime.
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
//! # The filesystem
//!
//! `std::fs::read`, `canonicalize`, `read_dir` and their relatives are the same
//! hazard: a syscall that parks a worker for as long as the disk takes, which on
//! an SD card under write load is long enough to see. A first scan found 11 call
//! sites in 6 async functions and 5 more through synchronous helpers, on the
//! page path: `species_image_file` (up to four reads per avatar), `serve_recording`
//! and `serve_spectrogram` (two `canonicalize` calls each) and `fetch_clips`, which
//! read the whole recordings directory. `std::fs::*`, `fs::*` (when it is not
//! `tokio::fs`), `File::open`/`create` and the `Path` methods that stat
//! (`exists`, `is_file`, `is_dir`, `metadata`, `canonicalize`, `read_dir`, …) are
//! primitives alongside the database ones. An awaited call is never one: that is
//! `tokio::fs`, whose futures run the syscall on the blocking pool.
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

/// `std::fs` functions, as `fs::NAME` or `std::fs::NAME`. A path with a `tokio`
/// segment is `tokio::fs`, which is async.
const FS_FNS: &[&str] = &[
    "read",
    "read_to_string",
    "read_dir",
    "read_link",
    "write",
    "create_dir",
    "create_dir_all",
    "remove_file",
    "remove_dir",
    "remove_dir_all",
    "rename",
    "copy",
    "hard_link",
    "metadata",
    "symlink_metadata",
    "canonicalize",
    "set_permissions",
    "exists",
];

/// `Path`/`File`/`DirEntry` methods that make a syscall. Resolved by name, so a
/// method of another type with one of these names is a false positive; the
/// awaited form (`tokio`'s) is never counted.
const FS_METHODS: &[&str] = &[
    "canonicalize",
    "exists",
    "try_exists",
    "is_file",
    "is_dir",
    "is_symlink",
    "metadata",
    "symlink_metadata",
    "read_dir",
    "read_link",
];

/// Method and function names that are also std's (`AtomicBool::store`,
/// `Vec::append`, `Read::read`, …). The scan resolves a call by name only, so a
/// repository function that shares one of these names and does file I/O would
/// otherwise make every `.store(..)` in the tree a blocking call: the first
/// filesystem run reported 21 such calls on atomics. A name here is never
/// promoted to "blocking" by the fixed point; a call that reaches the filesystem
/// through one is still caught where the filesystem call itself is made.
const STD_NAMES: &[&str] = &["store", "load", "append", "open", "read", "write", "flush"];

/// `(file, fn)` pairs that reach `std::fs` and are nonetheless not blocking, each
/// with the reason. Files under `/proc` and `/sys` are generated from kernel
/// memory when read: no storage device is involved, so the call neither waits on
/// a disk nor can be made to wait by one.
const MEMORY_BACKED: &[(&str, &str, &str)] = &[
    (
        "crates/birdnet-web/src/system_info.rs",
        "process_uptime_secs",
        "reads /proc/self/stat",
    ),
    (
        "crates/birdnet-web/src/routes/health.rs",
        "process_metrics",
        "reads /proc/self/status and /proc/cpuinfo",
    ),
];

/// Calls whose closure arguments run on another thread.
///
/// `cached_fragment` is here because it runs its `compute` closure under
/// `spawn_blocking` itself (`birdnet-web/src/analytics_cache.rs`);
/// `run_blocking_heavy` because it is `run_blocking` behind a heavy-read slot
/// (`birdnet-web/src/state.rs`).
const BOUNDARIES: &[&str] = &[
    "spawn_blocking",
    "block_in_place",
    "run_blocking",
    "run_blocking_heavy",
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
    // The same, for the filesystem and for the rest of the start. `serve` reads
    // the configuration, opens, checks and recovers the database, writes the
    // last-good copy and loads the certificate before the listener binds, and
    // `main` runs the one-shot command-line actions (`--doctor`,
    // `--apply-config`, …) the same way: once, in order, with nothing yet waiting
    // on a worker. A call added to either that is not on this list still fails.
    ("src/app.rs", "serve", "choose"),
    ("src/app.rs", "serve", "ensure_db_dir"),
    ("src/app.rs", "serve", "exists"),
    ("src/app.rs", "serve", "check_and_recover"),
    ("src/app.rs", "serve", "quarantine_corrupt_database"),
    ("src/app.rs", "serve", "ensure_incremental_vacuum"),
    ("src/app.rs", "serve", "install_persisted_secret"),
    ("src/app.rs", "serve", "build_state_with_analytics"),
    ("src/app.rs", "serve", "hooks"),
    ("src/app.rs", "serve", "create_apprise_client"),
    ("src/app.rs", "serve", "station_url"),
    ("src/app.rs", "serve", "spawn_watch"),
    ("src/app.rs", "serve", "record_last_good"),
    ("src/app.rs", "serve", "with_db"),
    ("src/app.rs", "serve", "maybe_install_avahi_service"),
    ("src/app.rs", "serve", "server_config"),
    ("src/main.rs", "main", "load_from"),
    ("src/main.rs", "main", "reload_filter_spec"),
    ("src/main.rs", "main", "run_apply_config"),
    ("src/main.rs", "main", "run_migration_report"),
    ("src/main.rs", "main", "run_rebuild_species_summary"),
    ("src/main.rs", "main", "run_with_format"),
    ("src/main.rs", "main", "run_decrypt"),
    ("src/main.rs", "main", "run_import_conf"),
    ("src/main.rs", "main", "run_import_clips"),
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

/// `std::fs::read(..)`, `fs::metadata(..)`, `File::open(..)`: the call is made
/// without `tokio` anywhere in its path. Returns a label for the report.
fn fs_call(func: &syn::Expr) -> Option<String> {
    let syn::Expr::Path(p) = func else {
        return None;
    };
    let segs: Vec<String> = p
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    if segs.iter().any(|s| s == "tokio") || segs.len() < 2 {
        return None;
    }
    let (q, last) = (&segs[segs.len() - 2], &segs[segs.len() - 1]);
    if q == "fs" && FS_FNS.contains(&last.as_str()) {
        return Some(format!("fs::{last}"));
    }
    if q == "File" && matches!(last.as_str(), "open" | "create" | "create_new") {
        return Some(format!("File::{last}"));
    }
    None
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
        let fs = if awaited { None } else { fs_call(&c.func) };
        if let Some(label) = &fs {
            self.record(label.clone(), c.span().start().line);
        }
        if let (false, None, Some((name, key, single))) = (awaited, &fs, &key) {
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
            && (PRIMITIVES.contains(&n.as_str())
                || FS_METHODS.contains(&n.as_str())
                || self.blocking.contains(&format!("m:{n}")))
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

/// A file that is test code by where it lives: `tests.rs` beside the module it
/// tests, or anything under a `tests/` directory.
fn is_test_file(path: &str) -> bool {
    path.ends_with("/tests.rs") || path.contains("/tests/")
}

/// Every call from async code that can block, in `files` (path, source).
fn scan(files: &[(String, String)]) -> (Vec<Hit>, usize) {
    let mut defs = Vec::new();
    for (path, src) in files {
        let parsed = syn::parse_file(src).unwrap_or_else(|e| panic!("{path} does not parse: {e}"));
        let mut c = Collect {
            file: path.clone(),
            // `#[cfg(test)] mod tests;` puts the attribute on the declaration, in
            // another file; the file itself carries nothing to say it is test code.
            in_test: usize::from(is_test_file(path)),
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
            // A std-shaped name is never promoted (see `STD_NAMES`).
            let bare = key.split_once(':').map_or(*key, |(_, n)| n);
            let bare = bare.split('@').next().unwrap_or(bare);
            if STD_NAMES.contains(&bare) {
                continue;
            }
            // Declared cheap (see `MEMORY_BACKED`).
            if ds.iter().all(|d| {
                MEMORY_BACKED
                    .iter()
                    .any(|(f, n, _)| *f == d.file && *n == d.name)
            }) {
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

/// Every source file the scan reads: this binary's, and every workspace crate's.
///
/// The crates are all in, not only `birdnet-web`, because the definitions are what
/// the fixed point follows: a handler that calls `birdnet_core::…::recording_stats`
/// reaches `std::fs` through a function that lives in another crate, and a scan
/// that cannot see the definition cannot see that.
fn source_files() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rs_files(&root.join("src"), root, &mut files);
    let crates = root.join("crates");
    let mut dirs: Vec<_> = std::fs::read_dir(&crates)
        .expect("crates")
        .map(|e| e.expect("entry").path())
        .collect();
    dirs.sort();
    for d in dirs {
        if d.join("src").is_dir() {
            rs_files(&d.join("src"), root, &mut files);
        }
    }
    files
}

#[test]
fn no_async_function_blocks_on_the_database_or_the_filesystem() {
    let files = source_files();
    let (hits, fns) = scan(&files);
    // The walk found the code: a scan of nothing is clean.
    assert!(fns > 3000, "scanned only {fns} functions");

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
        "{} call(s) block a runtime worker on the database or the filesystem; move \
         each into `state.run_blocking(..)` or `tokio::task::spawn_blocking`, or use \
         `tokio::fs`:\n{}",
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
        async fn b2(s: S) { s.run_blocking_heavy(|s| audit(s)).await; }
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

/// An out-of-line `tests.rs` is test code, and the same text in `mod.rs` is not.
#[test]
fn a_tests_file_is_test_code_and_a_module_file_is_not() {
    let src = "async fn t(s: S) { std::fs::write(p, b); }".to_owned();
    let (in_tests, _) = scan(&[("crates/x/src/m/tests.rs".to_owned(), src.clone())]);
    assert!(in_tests.is_empty(), "{:?}", in_tests.len());
    let (in_module, _) = scan(&[("crates/x/src/m/mod.rs".to_owned(), src)]);
    assert_eq!(in_module.len(), 1);
}

/// The filesystem shapes: a `std::fs` call written in full, through `use
/// std::fs`, `File::open`, the `Path` methods that stat, and a synchronous helper
/// two calls away.
#[test]
fn the_scan_flags_each_filesystem_shape() {
    let src = r"
        fn helper(p: &Path) -> u64 { std::fs::metadata(p).map_or(0, |m| m.len()) }
        async fn direct(p: PathBuf) { std::fs::read(&p); }
        async fn short(p: PathBuf) { fs::read_to_string(&p); }
        async fn file(p: PathBuf) { File::open(&p); }
        async fn stat(p: PathBuf) { p.canonicalize(); p.exists(); p.is_dir(); }
        async fn via_helper(p: PathBuf) { helper(&p); }
        async fn dirs(p: PathBuf) { p.read_dir(); }
        fn spawner(p: PathBuf) { tokio::spawn(async move { std::fs::remove_file(&p); }); }
    ";
    assert_eq!(
        hits_in(src),
        vec![
            pair("direct", "fs::read"),
            pair("short", "fs::read_to_string"),
            pair("file", "File::open"),
            pair("stat", "canonicalize"),
            pair("stat", "exists"),
            pair("stat", "is_dir"),
            pair("via_helper", "helper"),
            pair("dirs", "read_dir"),
            pair("spawner", "fs::remove_file"),
        ]
    );
}

/// The counterpart: `tokio::fs` (async, so awaited), the blocking boundaries, and
/// the names std shares with this repository's own functions all pass. `helper`
/// is unambiguously blocking — the first test shows the same helper flagged — so
/// each line passes because of its boundary, not because the helper went
/// unrecognised.
#[test]
fn the_scan_passes_each_filesystem_boundary() {
    let src = r"
        fn helper(p: &Path) { std::fs::metadata(p); }
        struct Slot;
        impl Slot { fn store(&self) { std::fs::write(&self.0, b'x'); } }
        async fn a(p: PathBuf) { tokio::fs::read(&p).await; }
        async fn b(p: PathBuf) { tokio::fs::metadata(&p).await; }
        async fn c(p: PathBuf) { tokio::task::spawn_blocking(move || std::fs::read(&p)).await; }
        async fn d(s: S, p: PathBuf) { s.run_blocking(move |_| helper(&p)).await; }
        async fn e(p: PathBuf) { tokio::fs::File::open(&p).await; }
        async fn f(p: PathBuf) { tokio::fs::try_exists(&p).await; }
        async fn g(flag: AtomicBool) { flag.store(true); }
        async fn h(v: Vec<u8>, w: Vec<u8>) { v.append(&mut w); }
        fn sync_only(p: PathBuf) { std::fs::read(&p); }
    ";
    assert_eq!(hits_in(src), Vec::<(String, String)>::new());
    // The same helper, unbounded, is caught.
    assert_eq!(
        hits_in(
            "fn helper(p: &Path) { std::fs::metadata(p); } async fn a(p: PathBuf) { helper(&p); }"
        ),
        vec![pair("a", "helper")]
    );
}
