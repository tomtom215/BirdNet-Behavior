//! Build the operator manual into `docs/book/_generated/html/` so the
//! runtime `ServeDir` mounted at `/help/*` (see
//! `birdnet-web::routes::pages::help::router`) has content to serve.
//!
//! Hard-required via `[build-dependencies] mdbook` per the maintainer's
//! locked-in RFC answer for O-20. The alternative — detect mdbook on PATH
//! with a silent fallback — was rejected because the drift risk (docs
//! out of date with code) outweighs the ~2 min added to a fresh build.
//!
//! ## What this script does
//!
//! Loads the **workspace root** `book.toml` — the same file GitHub Pages
//! builds from — overrides its build directory, and writes the rendered HTML
//! tree to `docs/book/_generated/html/`. The output path is checked into
//! `.gitignore` so the rendered tree never lands in commits — every build
//! rebuilds from source.
//!
//! ## One book.toml, not two
//!
//! There used to be a second config at `docs/book/book.toml` for this build.
//! The two drifted: the published site used the `light`/`navy` themes with
//! `docs/book-theme/custom.css` and folded sections, and the in-app copy used
//! the `rust` theme with no custom CSS and no folding — same Markdown, two
//! different-looking sites, and only the published one was link-checked.
//!
//! Overriding `build.build_dir` after load is the whole difference now, so the
//! two renders cannot diverge in anything else. `MDBook::build_dir_for` reads
//! that field at build time and joins it to the book root, which is what makes
//! the override work.
//!
//! ## Cargo rerun hints
//!
//! The script tells cargo to re-run only when book source files change,
//! so an iterative `cargo build` after a code-only edit does not pay the
//! mdbook tax. The `docs/book/_generated/html` output directory is
//! deliberately NOT listed as a rerun target — that would create a build
//! loop.
//!
//! ## Air-gapped releases
//!
//! An operator who builds in a context where mdbook fails (no source tree,
//! pre-rendered docs supplied separately) can set `BNB_SKIP_DOCS=1` to
//! make this script a no-op and ship a different `BNB_HELP_DIR` at
//! runtime instead.
//!
//! ## Built from a package, not a checkout
//!
//! A crate that `cargo publish` uploads (and `cargo install` unpacks into the
//! registry cache) carries `.cargo_vcs_info.json`; a checkout never does. For a
//! package this script does nothing. Cargo forbids a build script from writing
//! outside `OUT_DIR` and `cargo package` fails the verification build if it
//! does ("Source directory was modified by build.rs"); the registry cache is
//! often read-only anyway; and the rendered tree is served from a path relative
//! to the working directory, so one rendered into a registry directory would
//! never be found. The published crate therefore ships no manual sources, and
//! a `cargo install` binary answers `/help` with its "docs unavailable" panel
//! until `BNB_HELP_DIR` points at a rendered copy, as the release archives'
//! `help/` directory is.
//!
//! ## Clang's runtime on macOS
//!
//! Independent of the manual, and run first because a package build returns
//! before reaching the manual: on `aarch64-apple-darwin` the ONNX Runtime
//! library `ort-sys` downloads contains CoreML objects that call Clang's
//! `___isPlatformVersionAtLeast`. Rust's `std` defines that symbol, but weakly,
//! and the link of 0.18.0's release build did not take it from there
//! ("Undefined symbols for architecture arm64"). Adding Clang's own
//! `libclang_rt.osx.a` to the binary's link line resolves it; see
//! [`link_clang_runtime`].

use std::path::{Path, PathBuf};

fn main() {
    link_clang_runtime();
    if is_published_package() {
        return;
    }
    let book_root = book_source_root();

    // Always re-build when the source tree changes. Listing the directory
    // (rather than every .md file individually) lets new pages added by
    // SUMMARY.md get picked up without touching this script.
    println!("cargo:rerun-if-changed={}", book_root.display());
    println!(
        "cargo:rerun-if-changed={}",
        book_root.join("book.toml").display()
    );
    println!("cargo:rerun-if-env-changed=BNB_SKIP_DOCS");

    if std::env::var("BNB_SKIP_DOCS").is_ok_and(|v| !v.is_empty()) {
        println!(
            "cargo:warning=BNB_SKIP_DOCS set; skipping mdbook build (help drawer will show 'docs unavailable' until BNB_HELP_DIR points at a pre-rendered tree)"
        );
        return;
    }

    let mut book = match mdbook_driver::MDBook::load(&book_root) {
        Ok(book) => book,
        Err(e) => {
            // Print the underlying error: a silent skip here once masked a
            // book.toml option that an mdbook major release had removed.
            println!(
                "cargo:warning=could not load mdBook at {}: {e} — skipping docs build",
                book_root.display()
            );
            return;
        }
    };

    // The published site's build dir is `docs/.book-build`; the in-app tree has
    // to land where `pages::help`'s ServeDir looks. Everything else — theme,
    // custom CSS, folding, `create-missing` — comes from the one shared config.
    book.config.build.build_dir = PathBuf::from("docs/book/_generated/html");

    if let Err(e) = book.build() {
        // Don't abort the cargo build: an operator who hits a docs issue
        // should still get a working binary. The help drawer's fallback
        // panel covers the missing-pages case.
        // The whole chain, not just `{e}`. mdBook's top-level message here is
        // "Rendering failed", which says nothing: the actual cause — a
        // `git-repository-icon` prefix mdBook 0.5 rejects — was three links
        // down, and the one-line form hid it completely.
        println!(
            "cargo:warning=mdBook build failed: {} — /help/* will return 404 until docs are rendered",
            e.chain()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join(" <- ")
        );
    }
}

/// On macOS, add Clang's runtime library (`libclang_rt.osx.a`) to the link line
/// of this package's binaries.
///
/// The path is asked of `cc`, the linker driver rustc uses by default, so it is
/// the runtime of the Xcode or Command Line Tools install that does the link.
/// `cc --print-file-name` prints the bare file name when it finds nothing, so
/// only an absolute path to a file that exists is used; otherwise this says so
/// and leaves the link as it was, where the original "Undefined symbols" error
/// will still name the symbol.
fn link_clang_runtime() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let runtime = std::process::Command::new("cc")
        .arg("--print-file-name=libclang_rt.osx.a")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|printed| PathBuf::from(printed.trim()))
        .filter(|path| path.is_absolute() && path.is_file());
    match runtime {
        Some(path) => println!("cargo:rustc-link-arg-bins={}", path.display()),
        None => println!(
            "cargo:warning=`cc --print-file-name=libclang_rt.osx.a` found no Clang runtime; if the link fails with ___isPlatformVersionAtLeast undefined, install Xcode or the Command Line Tools"
        ),
    }
}

/// Whether this source tree is a package cargo unpacked rather than a checkout.
///
/// `cargo package` writes `.cargo_vcs_info.json` into every package it builds,
/// and a checkout has none, so its presence is how a build script can tell.
fn is_published_package() -> bool {
    std::env::var("CARGO_MANIFEST_DIR")
        .is_ok_and(|dir| Path::new(&dir).join(".cargo_vcs_info.json").exists())
}

/// Locate the book root — the directory holding `book.toml`, whose `src` key
/// points at `docs/book/`.
///
/// That is the workspace root, not `docs/book/`: there is one config now and
/// GitHub Pages builds from the same one.
fn book_source_root() -> PathBuf {
    let manifest =
        std::env::var("CARGO_MANIFEST_DIR").map_or_else(|_| PathBuf::from("."), PathBuf::from);
    if manifest.join("book.toml").exists() && manifest.join("docs/book").exists() {
        return manifest;
    }
    // Fallback: walk up so a `cargo build` from a child crate still finds it.
    let mut cur: &Path = &manifest;
    while let Some(parent) = cur.parent() {
        if parent.join("book.toml").exists() && parent.join("docs/book").exists() {
            return parent.to_path_buf();
        }
        cur = parent;
    }
    manifest
}
