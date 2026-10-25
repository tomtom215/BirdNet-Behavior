# three.js, the part the soundscape terrain uses

`three.module.min.js` is **not** the whole library. It is one ES module that holds the WebGL
renderer and the sixteen classes `static/terrain.js` uses, tree-shaken out of three.js 0.186.1 by
esbuild. The WebGPU renderer, the node material system, the loaders and everything else are not in
it. It is loaded by `static/terrain.js` with a dynamic `import()` the first time somebody opens
the 3D view of the Patterns "When active" terrain, and by nothing else: the page does not
request it on load, and a browser with no WebGL never does.

| | |
|---|---|
| Upstream | https://github.com/mrdoob/three.js (npm: https://www.npmjs.com/package/three) |
| Version | 0.186.1, published 2026-09-24 (the newest release at least two weeks old when it was chosen) |
| Licence | MIT, copyright 2010-2026 three.js authors: `LICENSE` here, and in `THIRD_PARTY_LICENSES.txt` |
| Downloaded | 2026-10-21 from https://registry.npmjs.org/three/-/three-0.186.1.tgz |
| Tarball SHA-256 | `8cd068708ea44f2c73c944b1cead2ba2f0d5c15c8fc194e5700f4e4f4a033fe7` |
| Tarball SHA-1 (npm `shasum`) | `6d50f70c2c437f844179bbb56d6f5b774e1ca38a` |
| Tarball integrity (npm, SHA-512) | `sha512-blFeqb49wRCSGUGj7gtpfnSGHy2lwDk94RhUmS1c/hTby70kvChbWpkJ4Pm1390LqzzvTmzgXKHPEafJwCb8jA==` |
| `three.module.min.js` SHA-256 | `e9e6fbbda1cb1b11bde7e82ca6c8538441e00c3284da4a9c05d7e3727021bda0` |
| `three.module.min.js` size | 538 460 bytes, 134 086 gzipped (`gzip -9`) |

## What was checked against the registry

* The tarball was downloaded with `curl` and its SHA-512 compared with the `dist.integrity` the
  registry reports for `three@0.186.1` (`npm view three@0.186.1 dist`): equal. Its SHA-1 equals
  the registry's `shasum`, and its SHA-256 is written above.
* The same integrity is in `tools/vendor-three/package-lock.json`, so `npm ci` (which the build
  uses) refuses any other tarball under that name.
* The committed file is the output of that tarball and `tools/vendor-three/entry.js`, byte for byte:
  `tools/vendor-three/build.sh --check` downloads the tarball, verifies it, rebuilds, and compares.
  Two clean builds in different directories gave the same SHA-256, so the build is reproducible.

## How to rebuild or bump it

```bash
tools/vendor-three/build.sh           # rewrite the file from the pinned inputs
tools/vendor-three/build.sh --check   # fail if the committed file is not what they produce
```

To change the library version: edit `three` in `tools/vendor-three/package.json`, run
`npm install --package-lock-only --ignore-scripts` there, put the new tarball's SHA-256 in
`build.sh`, run `build.sh`, then update this table, the first eight hex digits of the file's
SHA-256 in `THREE_HASH` in `static/terrain.js`, `third_party/three-<version>/` (and its row in
`third_party/README.md`) and `NOTICE`, and run `python3 scripts/gen-third-party-licenses.py`.
To use another class in the terrain, add its name to `entry.js` and do the same.

`crates/birdnet-web/tests/vendored_three_is_pinned_and_small.rs` holds all of it without the
network: the file's SHA-256 is the one above, its gzipped size is under the budget, the hash in
`terrain.js` is the file's, the licence is the vendored one, and the version agrees everywhere it
is written.

## How it is served

`/static/vendor/three/three.module.min.js`, with `Cache-Control: public, max-age=31536000,
immutable` and a strong `ETag` (the first sixteen hex digits of the SHA-256). The terrain asks for
it as `three.module.min.js?v=<first eight digits of the SHA-256>`, so a new build is a new URL and
the year-long cache is safe. The server's gzip layer compresses it on the way out.
