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
//! Loads `docs/book/book.toml`, builds the book, and writes the rendered
//! HTML tree to `docs/book/_generated/html/`. The output path is checked
//! into `.gitignore` so the rendered tree never lands in commits — every
//! build rebuilds from source.
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

use std::path::{Path, PathBuf};

fn main() {
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

    let book = match mdbook_driver::MDBook::load(&book_root) {
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

    if let Err(e) = book.build() {
        // Don't abort the cargo build: an operator who hits a docs issue
        // should still get a working binary. The help drawer's fallback
        // panel covers the missing-pages case.
        println!(
            "cargo:warning=mdBook build failed: {e} — /help/* will return 404 until docs are rendered"
        );
    }
}

/// Locate the book source directory relative to `CARGO_MANIFEST_DIR`.
/// Defaults to `docs/book/` under the workspace root.
fn book_source_root() -> PathBuf {
    let manifest =
        std::env::var("CARGO_MANIFEST_DIR").map_or_else(|_| PathBuf::from("."), PathBuf::from);
    let candidate = manifest.join("docs/book");
    if candidate.exists() {
        return candidate;
    }
    // Fallback: walk up looking for `docs/book/` so a `cargo build` from a
    // child crate still finds the right tree. Stops at filesystem root.
    let mut cur: &Path = &manifest;
    while let Some(parent) = cur.parent() {
        let here = parent.join("docs/book");
        if here.exists() {
            return here;
        }
        cur = parent;
    }
    candidate
}
