#!/usr/bin/env bash
# Rebuild crates/birdnet-web/static/vendor/three/three.module.min.js from the pinned three.js,
# or check that the committed file is exactly what the pinned inputs produce.
#
#   tools/vendor-three/build.sh           # rewrite the file (and print its hashes)
#   tools/vendor-three/build.sh --check   # fail if the committed file differs from a fresh build
#
# What is pinned, and where:
#   * three.js 0.186.1 and esbuild 0.28.2, exactly, in package.json; their tarballs' sha512
#     integrity in package-lock.json. `npm ci` refuses a tarball that does not match, which is
#     the check that the file came from the npm registry's three@0.186.1 and not from
#     something that merely has that name.
#   * The tarball's sha256 below, a second, independent record of the same bytes (the lockfile
#     has sha512 only; a person comparing against a download wants the other one).
#   * What goes into the bundle: entry.js, the named exports and nothing else.
# esbuild's output depends on nothing but those inputs: two clean builds in different
# directories gave the same sha256 (and so does --check).
#
# Needs node, npm and network access to registry.npmjs.org. Nothing here runs in a build of the
# station: the built file is committed and embedded with include_bytes!.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
out="$root/crates/birdnet-web/static/vendor/three/three.module.min.js"

THREE_VERSION=0.186.1
THREE_TARBALL_SHA256=8cd068708ea44f2c73c944b1cead2ba2f0d5c15c8fc194e5700f4e4f4a033fe7

mode=write
case "${1:-}" in
  "") ;;
  --check) mode=check ;;
  *) echo "usage: $0 [--check]" >&2; exit 2 ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/build" "$tmp/tarball"
cp "$here/package.json" "$here/package-lock.json" "$here/entry.js" "$tmp/build/"

# The tarball itself, from the registry, against the sha256 recorded here.
( cd "$tmp/tarball" && npm pack "three@$THREE_VERSION" --silent >/dev/null )
got=$(sha256sum "$tmp/tarball/three-$THREE_VERSION.tgz" | cut -d' ' -f1)
if [ "$got" != "$THREE_TARBALL_SHA256" ]; then
  echo "three-$THREE_VERSION.tgz is $got, expected $THREE_TARBALL_SHA256" >&2
  exit 1
fi

# The build, from the lockfile (its integrity hashes are checked by npm ci).
( cd "$tmp/build" \
  && npm ci --ignore-scripts --no-audit --no-fund >/dev/null \
  && ./node_modules/.bin/esbuild entry.js --bundle --minify --format=esm --target=es2020 \
       --legal-comments=none --log-level=warning --outfile=three.module.min.js )

built="$tmp/build/three.module.min.js"
sha=$(sha256sum "$built" | cut -d' ' -f1)
bytes=$(stat -c %s "$built")
gz=$(gzip -9 -c "$built" | wc -c)

if [ "$mode" = check ]; then
  if cmp -s "$built" "$out"; then
    echo "ok: $out is the build of three@$THREE_VERSION ($sha, $bytes bytes, $gz gzipped)"
  else
    echo "$out differs from a fresh build ($(sha256sum "$out" | cut -d' ' -f1) vs $sha)" >&2
    exit 1
  fi
else
  cp "$built" "$out"
  echo "wrote $out"
  echo "  sha256 $sha"
  echo "  $bytes bytes, $gz gzipped (gzip -9)"
  echo "now update the hash in static/vendor/three/README.md and the THREE_HASH in static/terrain.js"
  echo "(the first eight digits), then run the size-budget test."
fi
