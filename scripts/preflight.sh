#!/usr/bin/env bash
# Run, before a push, every check CI will run on a pull request that this machine can run.
#
#   scripts/preflight.sh            the fast and the build gates (no tests)
#   scripts/preflight.sh --msrv     also `cargo check` on the declared MSRV, in its own target dir
#   scripts/preflight.sh --tests    also the workspace tests (slow; each integration test links the
#                                   bundled DuckDB, about 280 MB, so mind the disk)
#
# Why it exists: a red CI run costs the better part of an hour, and the failures that cost
# the most were each a check the workflow runs and the author had not (the spell check,
# rustdoc's private intra-doc links, the book's link check). This is the workflows' own
# commands, in the workflows' own scope (the whole workspace, each feature set CI builds),
# with the tools at the versions CI pins. A tool that is not installed is a FAILURE here,
# never a skip: a check that did not run is not a pass.
#
# Read the list at the end. It also says what this cannot cover, so "preflight passed" is
# never read as "CI will pass":
#   * CodeQL, the container image, the install and upgrade smoke tests (they need Docker or
#     systemd) and the aarch64 cross-check (needs a cross toolchain) run only in CI.
#   * The browser gates (axe, layout, legibility, ...) need the screenshot fixture; the
#     commands are in `.github/workflows/a11y.yml`, steps "Accessibility gate" onward.
#   * OSV and RustSec answer for the advisory database as of now. A finding in a package this
#     change did not touch is the state of `main`, and is not yours to fix on this branch.
#
# Tools (versions as pinned in .github/workflows/supply-chain.yml):
#   cargo install --locked typos-cli --version 1.51.1
#   cargo install --locked cargo-machete cargo-deny
#   cargo install --locked cargo-audit --version 0.22.2
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2
export CARGO_INCREMENTAL=0

run_msrv=0
run_tests=0
for arg in "$@"; do
    case "$arg" in
        --msrv) run_msrv=1 ;;
        --tests) run_tests=1 ;;
        *) echo "usage: $0 [--msrv] [--tests]" >&2; exit 2 ;;
    esac
done

results=()
failed=0

gate() {
    local name=$1
    shift
    echo "── $name"
    if "$@"; then
        results+=("ok       $name")
    else
        results+=("FAILED   $name")
        failed=1
    fi
}

# A tool that is missing, or not at the pinned version, fails the gate with the install line.
tool() {
    local name=$1 binary=$2 want=$3 hint=$4 got
    if ! command -v "$binary" >/dev/null 2>&1; then
        echo "preflight: $binary is not installed. $hint" >&2
        return 1
    fi
    if [ -n "$want" ]; then
        got=$("$binary" --version 2>&1)
        case "$got" in
            *"$want"*) ;;
            *) echo "preflight: $name wants $binary $want, found '$got'. $hint" >&2; return 1 ;;
        esac
    fi
    return 0
}

typos_gate() {
    tool typos typos 1.51.1 "cargo install --locked typos-cli --version 1.51.1" && typos
}
machete_gate() {
    tool cargo-machete cargo-machete "" "cargo install --locked cargo-machete" && cargo machete
}
deny_gate() {
    tool cargo-deny cargo-deny "" "cargo install --locked cargo-deny" && cargo deny --all-features check
}
audit_gate() {
    tool cargo-audit cargo-audit 0.22.2 "cargo install --locked cargo-audit --version 0.22.2" \
        && cargo audit --deny warnings
}
rustdoc_gate() {
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --all-features
}
cli_help_gate() {
    ./scripts/gen-cli-help.sh && git diff --exit-code docs/book/_generated/cli-help.txt
}
notices_gate() {
    cargo fetch --locked && python3 scripts/gen-third-party-licenses.py --check
}
msrv_gate() {
    local declared
    declared=$(awk '/^\[workspace\.package\]/{f=1; next} /^\[/{f=0} f && /^rust-version = /{gsub(/[" ]/, "", $3); print $3; exit}' Cargo.toml)
    [ -n "$declared" ] || { echo "preflight: no rust-version in Cargo.toml" >&2; return 1; }
    rustup toolchain list | grep -q "^$declared" \
        || { echo "preflight: install it first: rustup toolchain install $declared --profile minimal" >&2; return 1; }
    RUSTUP_TOOLCHAIN=$declared CARGO_TARGET_DIR="${PREFLIGHT_MSRV_TARGET_DIR:-target/msrv}" \
        cargo check --workspace --all-features --all-targets --locked
}

# Cheapest first, so the common failures show up in seconds.
gate "fmt (ci.yml: Format)" cargo fmt --check --all
gate "typos (supply-chain.yml: Spelling)" typos_gate
gate "cargo-machete (supply-chain.yml)" machete_gate
gate "installer is generated, no drift (supply-chain.yml: shellcheck job)" bash installer/build.sh --check
gate "cargo-deny (supply-chain.yml)" deny_gate
gate "cargo-audit (supply-chain.yml)" audit_gate
gate "OSV, against the database as of now (osv.yml)" python3 scripts/osv-check.py
gate "clippy, default features (ci.yml: Clippy)" cargo clippy --workspace --all-targets -- -D warnings
gate "clippy, all features (ci.yml: Clippy)" cargo clippy --workspace --all-targets --all-features -- -D warnings
gate "clippy, no default features (ci.yml: Clippy)" cargo clippy --workspace --all-targets --no-default-features -- -D warnings
gate "rustdoc, -D warnings (ci.yml: Rustdoc)" rustdoc_gate
gate "build, default features (ci.yml: Build)" cargo build --workspace
gate "CLI help is current (ci.yml: Build)" cli_help_gate
gate "build, all features (ci.yml: Build)" cargo build --workspace --all-features
gate "every manual link resolves (ci.yml: Build)" ./scripts/check-book-links.py docs/book/_generated/html
gate "third-party licence notices (supply-chain.yml)" notices_gate
gate "doc tests (ci.yml: Tests)" cargo test --workspace --doc

if [ "$run_msrv" = 1 ]; then
    gate "MSRV check (ci.yml: MSRV)" msrv_gate
fi
if [ "$run_tests" = 1 ]; then
    gate "unit tests (ci.yml: Tests)" cargo test --workspace --lib --bins
    gate "integration tests (ci.yml: Tests)" cargo test --workspace --tests
    gate "full suite, all features (ci.yml: Tests)" cargo test --workspace --all-features
fi

echo
echo "preflight on $(git rev-parse --short HEAD) ($(git status --porcelain | wc -l | tr -d ' ') uncommitted files)"
printf '%s\n' "${results[@]}"
[ "$run_msrv" = 1 ] || echo "not run   MSRV check (pass --msrv)"
[ "$run_tests" = 1 ] || echo "not run   workspace tests (pass --tests)"
echo "not run   CodeQL, container image, install/upgrade smoke, aarch64 cross-check, browser gates (see the header)"
if [ "$failed" = 0 ]; then
    echo "every gate that ran passed; that is not a promise about the ones that did not"
fi
exit "$failed"
