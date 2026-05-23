#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# apply.sh — bulk-apply the BirdNet-Behavior design follow-on PR set
#
# Run from the repo root of `tomtom215/BirdNet-Behavior` (i.e. the directory
# containing `crates/`, `Cargo.toml`, etc).
#
# This script handles the SAFE operations:
#   * file copies (templates, Rust modules, fonts, static assets)
#   * CSS appends (O-03 display-preferences theming)
#   * Cargo.toml dependency additions (O-07 hmac/sha2/base64)
#
# It does NOT touch existing source files. The 7 in-place edits required —
# route registrations, layout.html link adds, FOUC-guard extension,
# feed-row anchor swaps, and one optional schema migration — are spelled
# out in BUILD_PR.md as per-file edit recipes. Run those after this script.
# ──────────────────────────────────────────────────────────────────────────
set -euo pipefail

# Resolve the bundle root (the dir this script lives in).
BUNDLE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WEB="crates/birdnet-web"

# ── Preflight ───────────────────────────────────────────────────────────
if [[ ! -d "$WEB" ]]; then
  echo "❌ Run this from the BirdNet-Behavior repo root (didn't find $WEB)" >&2
  exit 1
fi
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "❌ Working tree has uncommitted changes. Commit or stash first." >&2
  exit 1
fi

echo "▸ Applying BirdNet-Behavior design follow-on PR set v1.1"
echo "  bundle: $BUNDLE_ROOT"
echo "  target: $WEB"
echo

# ── Step 1: branch ──────────────────────────────────────────────────────
BRANCH="${BNB_BRANCH:-feat/design-followons}"
if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
  echo "▸ Switching to existing branch $BRANCH"
  git checkout "$BRANCH"
else
  echo "▸ Creating new branch $BRANCH"
  git checkout -b "$BRANCH"
fi
echo

# ── Step 2: file copies ─────────────────────────────────────────────────
echo "▸ Copying templates, Rust modules, and static assets"
for pr in "$BUNDLE_ROOT"/O-*/; do
  pr_id="$(basename "$pr")"
  if [[ -d "$pr/templates" ]]; then
    echo "  ↳ templates from $pr_id"
    mkdir -p "$WEB/templates"
    cp -r "$pr"/templates/. "$WEB/templates/"
  fi
  if [[ -d "$pr/src" ]]; then
    echo "  ↳ src from $pr_id"
    cp -r "$pr"/src/. "$WEB/src/"
  fi
  if [[ -d "$pr/static" ]]; then
    echo "  ↳ static from $pr_id"
    mkdir -p "$WEB/static"
    cp -r "$pr"/static/. "$WEB/static/"
  fi
done
echo

# ── Step 3: CSS appends ─────────────────────────────────────────────────
echo "▸ Appending CSS additions"
shopt -s nullglob
APPENDS=("$BUNDLE_ROOT"/O-*/css/*.append)
if (( ${#APPENDS[@]} > 0 )); then
  for ap in "${APPENDS[@]}"; do
    pr_id="$(basename "$(dirname "$(dirname "$ap")")")"
    target="$WEB/static/css/app.css"
    if grep -q "$(basename "$ap" .append)" "$target" 2>/dev/null \
       && grep -q "BNB:CSS-APPEND:$pr_id" "$target" 2>/dev/null; then
      echo "  ↳ $pr_id append already present — skipping"
      continue
    fi
    echo "  ↳ appending $pr_id → static/css/app.css"
    {
      printf "\n/* ── BNB:CSS-APPEND:%s ── */\n" "$pr_id"
      cat "$ap"
      printf "\n/* ── /BNB:CSS-APPEND:%s ── */\n" "$pr_id"
    } >> "$target"
  done
else
  echo "  (none)"
fi
echo

# ── Step 4: Cargo dependencies (O-07 needs hmac, sha2, base64) ──────────
echo "▸ Adding Cargo dependencies for O-07 (share permalinks)"
pushd "$WEB" > /dev/null
needs_hmac=true; grep -q '^hmac\b' Cargo.toml && needs_hmac=false
needs_sha2=true; grep -q '^sha2\b' Cargo.toml && needs_sha2=false
needs_b64=true;  grep -q '^base64\b' Cargo.toml && needs_b64=false

$needs_hmac && cargo add hmac@0.12 || echo "  ↳ hmac already present"
$needs_sha2 && cargo add sha2@0.10 || echo "  ↳ sha2 already present"
$needs_b64  && cargo add base64@0.22 --no-default-features --features std || echo "  ↳ base64 already present"
popd > /dev/null
echo

# ── Step 5: summary ─────────────────────────────────────────────────────
echo "▸ Bulk copy complete. Next steps (see BUILD_PR.md):"
echo
echo "  1. Edit  $WEB/src/routes/pages/mod.rs"
echo "       \u2022 add: pub mod migration; pub mod dawn_chorus; pub mod species_photo;"
echo "                  pub mod today_phrase; pub mod empty_states;"
echo "       \u2022 add to .merge() chain: migration, dawn_chorus, species_photo"
echo "       \u2022 add include_str! consts: MIGRATION_PAGE_HTML, DAWN_CHORUS_PAGE_HTML, DETECTION_DETAIL_HTML"
echo "  2. Edit  $WEB/src/routes/mod.rs"
echo "       \u2022 add: pub mod share; pub mod feeds;"
echo "       \u2022 add to api_routes() chain: share::router(), feeds::router()"
echo "  3. Edit  $WEB/src/routes/pages/today.rs"
echo "       \u2022 register: .route(\"/pages/today-phrase\", get(today_phrase_partial))"
echo "       \u2022 add 'mod today_phrase;' near top"
echo "  4. Edit  $WEB/src/routes/pages/dashboard/partials.rs"
echo "       \u2022 feed-row time \u2192 <a href=\"/detection/{id}\"> wrapper"
echo "  5. Edit  $WEB/src/routes/pages/today.rs (detection card)"
echo "       \u2022 same time \u2192 link wrap as the dashboard rows"
echo "  6. Edit  $WEB/templates/layout.html"
echo "       \u2022 add <link rel=\"stylesheet\" href=\"/static/css/print.css\" media=\"print\">"
echo "       \u2022 add <link rel=\"alternate\" type=\"application/rss+xml\" href=\"/feeds/rare.rss\">"
echo "       \u2022 add Migration topnav link: <a href=\"/migration\" class=\"topnav-link {{nav_migration}}\">Migration</a>"
echo "       \u2022 extend FOUC guard for bnb-motion and bnb-contrast keys (see O-03/DIFF.md)"
echo "  7. (optional) Apply the detection_reviews schema migration"
echo "       (see ROLLBACK.md \u00a7 O-05 for the CREATE TABLE block)"
echo
echo "▸ Then: cargo check && cargo test && cargo clippy --all-targets"
echo
echo "Done."
