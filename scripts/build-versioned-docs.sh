#!/usr/bin/env bash
# Build the published, versioned operator manual.
#
#   scripts/build-versioned-docs.sh <out-dir>
#
# Layout of <out-dir> (what GitHub Pages serves):
#
#   /             the latest release's manual — so every existing link to
#                 `…/BirdNet-Behavior/guide/today.html` keeps working, and
#                 now means "the release you can install", not "main"
#   /vX.Y.Z/      one manual per release tag, from that tag's own sources
#   /dev/         the manual on `main` (origin/main), whatever this run checked
#                 out: documentation for features that have not been released.
#                 A release-tag run checks out the tag, so building /dev/ from
#                 HEAD there published the release's own manual as "dev" until
#                 the next push to main. DOCS_DEV_REF=HEAD builds the checkout
#                 instead; docs.yml sets it for pull requests, whose build exists
#                 to grade the pull request's own tree and deploys nothing.
#   /versions.json  what the version switcher reads
#
# Versioning starts at FIRST_VERSIONED_RELEASE. Earlier tags were rendered by
# mdBook 0.4 from a different config and are not rebuilt; their releases'
# manuals are the in-app `/help/*` tree in those binaries, which `build.rs`
# renders from the same sources at build time.
#
# Every build — including old tags — gets the *current* switcher assets, so
# the switcher behaves the same on every version. Each tree is link-checked.
#
# Requires: git with tags and origin/main fetched (`fetch-depth: 0`), mdbook on
# PATH (or $MDBOOK), python3. Self-test: scripts/test-build-versioned-docs.sh.
set -euo pipefail

FIRST_VERSIONED_RELEASE="v0.17.1"
SITE_PATH="${DOCS_SITE_PATH:-/BirdNet-Behavior/}"
MDBOOK="${MDBOOK:-mdbook}"
DEV_REF="${DOCS_DEV_REF:-origin/main}"

out="${1:?usage: $0 <out-dir>}"
repo="$(git rev-parse --show-toplevel)"
work="$(mktemp -d)"
trap 'git -C "$repo" worktree prune; rm -rf "$work"' EXIT

mkdir -p "$out"
out="$(cd "$out" && pwd)"
if [ -n "$(ls -A "$out")" ]; then
    echo "build-versioned-docs: $out is not empty" >&2
    exit 1
fi

# Release tags only (vMAJOR.MINOR.PATCH, no suffix), oldest first, from the
# first versioned release on. `sort -V` orders 0.9 before 0.10.
releases=()
while IFS= read -r tag; do
    releases+=("$tag")
done < <(git -C "$repo" tag -l 'v*' \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    | { cat; echo "$FIRST_VERSIONED_RELEASE"; } \
    | sort -uV \
    | sed -n "/^${FIRST_VERSIONED_RELEASE//./\\.}\$/,\$p")
# The sentinel line keeps `sed` anchored even before the first tag exists;
# drop it again unless it is a real tag.
if ! git -C "$repo" rev-parse -q --verify "refs/tags/$FIRST_VERSIONED_RELEASE" >/dev/null; then
    releases=("${releases[@]:1}")
fi
if [ "${#releases[@]}" -eq 0 ]; then
    echo "build-versioned-docs: no release tags at or after $FIRST_VERSIONED_RELEASE (fetch tags?)" >&2
    exit 1
fi
latest="${releases[${#releases[@]}-1]}"

# Resolve /dev/'s source now rather than after every release has been rendered:
# a checkout without origin/main (a shallow clone, a fresh local repository)
# would otherwise fail at the very end, or, worse, quietly publish some other
# branch under the name `dev`.
if ! dev_commit="$(git -C "$repo" rev-parse -q --verify "${DEV_REF}^{commit}")"; then
    echo "build-versioned-docs: cannot resolve '$DEV_REF' to build /dev/ from." >&2
    echo "  Fetch it (git fetch origin main) so /dev/ is main's manual, or set" >&2
    echo "  DOCS_DEV_REF=HEAD to build the current checkout instead." >&2
    exit 1
fi

# render <source-tree> <dest> <version-label> <depth-below-site-root>
render() {
    local src="$1" dest="$2" label="$3" depth="$4"
    local theme="$src/docs/book-theme"
    mkdir -p "$theme"
    cp "$repo/docs/book-theme/version-switcher.js" "$repo/docs/book-theme/version-switcher.css" \
        "$repo/docs/book-theme/a11y.js" "$theme/"
    # The switcher learns which version it is on, and how far the book root is
    # below the site root, from this file rather than from the URL.
    printf 'window.bnbDocs = { version: "%s", depth: %d };\n' "$label" "$depth" \
        > "$theme/version-id.js"
    local url="$SITE_PATH"
    [ "$depth" -eq 1 ] && url="${SITE_PATH}${label}/"
    # "Suggest an edit" only where the page is main's own source. A release
    # cannot be edited, and a page later renamed on main would leave a
    # release's edit link pointing at nothing — `check-book-links.py` fails
    # on that, which would block publishing for an unrelated reason.
    local edit=()
    [ "$label" = "dev" ] || edit=(MDBOOK_OUTPUT__HTML__EDIT_URL_TEMPLATE='""')
    (
        cd "$src"
        env "${edit[@]}" \
        MDBOOK_OUTPUT__HTML__SITE_URL="$url" \
        MDBOOK_OUTPUT__HTML__ADDITIONAL_CSS='["docs/book-theme/custom.css","docs/book-theme/version-switcher.css"]' \
        MDBOOK_OUTPUT__HTML__ADDITIONAL_JS='["docs/book-theme/a11y.js","docs/book-theme/version-id.js","docs/book-theme/version-switcher.js"]' \
            "$MDBOOK" build -d "$dest"
    )
    python3 "$repo/scripts/check-book-links.py" --base "$url" "$dest"
}

# A release is built from a clean checkout of its tag, never from the working
# tree, so what is published under vX.Y.Z is what that tag says.
for tag in "${releases[@]}"; do
    git -C "$repo" worktree add -q --detach "$work/$tag" "$tag"
    render "$work/$tag" "$out/$tag" "$tag" 1
    if [ "$tag" = "$latest" ]; then
        render "$work/$tag" "$work/root" "$tag" 0
    fi
    git -C "$repo" worktree remove --force "$work/$tag"
done

# `dev` is the committed state of $DEV_REF — a worktree of that commit, so an
# uncommitted edit cannot reach the published site. It is main (origin/main), not
# HEAD: on a release-tag run HEAD is the release.
git -C "$repo" worktree add -q --detach "$work/dev" "$dev_commit"
render "$work/dev" "$out/dev" "dev" 1
git -C "$repo" worktree remove --force "$work/dev"

# The root book is copied in last: it must not shadow a version directory, and
# a page of the book named like one would be a collision worth failing on.
for entry in "$work/root"/*; do
    name="$(basename "$entry")"
    if [ -e "$out/$name" ]; then
        echo "build-versioned-docs: root book entry '$name' collides with a version directory" >&2
        exit 1
    fi
    cp -R "$entry" "$out/"
done
cp "$work/root"/.[!.]* "$out/" 2>/dev/null || true

python3 - "$out/versions.json" "$latest" "${releases[@]}" <<'PY'
import json, sys
path, latest, *releases = sys.argv[1:]
doc = {
    "latest": latest,
    # Newest first: the order a reader scans a menu in.
    "versions": [{"name": v, "path": f"{v}/"} for v in reversed(releases)]
              + [{"name": "dev", "path": "dev/"}],
}
with open(path, "w", encoding="utf-8") as fh:
    json.dump(doc, fh, indent=2)
    fh.write("\n")
PY

echo "build-versioned-docs: root = $latest; versions: ${releases[*]} dev"
echo "build-versioned-docs: dev = $DEV_REF at ${dev_commit:0:12}"
