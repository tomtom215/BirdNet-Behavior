# third_party/

Licence texts for the parts of a BirdNet-Behavior build that are not Rust
packages. `scripts/gen-third-party-licenses.py` reads them, and the licences of
every Rust package, into `THIRD_PARTY_LICENSES.txt`, which ships in the release
archive and the container image. The directory name carries the version the text
belongs to, and the script fails if the build links a different one.

| Directory | What it covers | Why this version |
|---|---|---|
| `onnxruntime-1.31.0/` | ONNX Runtime, statically linked through `ort-sys` | the version in `ort-sys`'s `build/download/dist.tsv`; the static library carries `rel-1.31.0` in its build info |
| `duckdb-1.5.6/` | the libraries DuckDB bundles, and ICU (the embedded `icu` extension) | `libduckdb-sys` 1.10506 bundles DuckDB 1.5.6 |
| `htmx-2.0.4/` | htmx, served as `/static/htmx.min.js` | the `version` in the file; its bytes equal the npm `htmx.org@2.0.4` package's `dist/htmx.min.js` |

The fonts' licences are beside the fonts, in
`crates/birdnet-web/static/fonts/LICENSES.txt`.

## Updating

When `ort`, `libduckdb-sys` or `static/htmx.min.js` moves, the generator fails
and names the directory it wants. Make it: fetch the files below at the new tag
(`https://raw.githubusercontent.com/<project>/<tag>/...`) into
`third_party/<name>-<version>/`, remove the old directory, update this file's
table, and rerun `python3 scripts/gen-third-party-licenses.py`. Do not edit the
texts; they are the upstream files, byte for byte (line endings aside: this
repository stores LF).

DuckDB's `third_party/thrift/` has no licence file at v1.5.6 (Apache Thrift is
Apache-2.0); that is the one bundled library without a text here.

## Provenance

Downloaded 2026-10-14. SHA-256 of each file as vendored:

| File | Source | SHA-256 |
|---|---|---|
| `onnxruntime-1.31.0/LICENSE` | https://raw.githubusercontent.com/microsoft/onnxruntime/v1.31.0/LICENSE | `2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c` |
| `onnxruntime-1.31.0/ThirdPartyNotices.txt` | https://raw.githubusercontent.com/microsoft/onnxruntime/v1.31.0/ThirdPartyNotices.txt | `143764b952fdb1a7c69ce653bfba74a7744d6a8a573bfb73e235fba356c83de3` |
| `duckdb-1.5.6/brotli.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/brotli/LICENSE | `3d180008e36922a4e8daec11c34c7af264fed5962d07924aea928c38e8663c94` |
| `duckdb-1.5.6/concurrentqueue.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/concurrentqueue/LICENSE | `eaf0df40f77e929cd83cd4485de2bdf806c7908db84a52007a700588f6566331` |
| `duckdb-1.5.6/fast_float.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/fast_float/LICENSE | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `duckdb-1.5.6/fastpforlib.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/fastpforlib/LICENSE | `9ff562631a04b8cb1030c53fdf461bd749c9af0655d86e4f17eb270e935a7fe2` |
| `duckdb-1.5.6/fmt.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/fmt/LICENSE | `825c9324e70f8c839c8ba910543dd4a7daee243b86ef960594c11381a19980b8` |
| `duckdb-1.5.6/fsst.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/fsst/LICENSE | `20e9c14ff437a73a6e2b105f1b5aaf9ef24a84d962741c0ab01d04b4910294ab` |
| `duckdb-1.5.6/httplib.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/httplib/LICENSE | `4b45cbe16d7b71b89ae6127e26e0d90a029198ca5e958ad8e3d0b8bbed364d8b` |
| `duckdb-1.5.6/hyperloglog.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/hyperloglog/LICENSE | `3a3fcdbaecbeb1e7dea97843961100c637766e59de4674fed809760283b716e1` |
| `duckdb-1.5.6/icu.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/extension/icu/third_party/icu/LICENSE | `f0e4446e54a2cee165d69e007f4787183d497c709664e79d6955691bfeb3f837` |
| `duckdb-1.5.6/jaro_winkler.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/jaro_winkler/LICENSE | `077815ffd10f0677d060e9d5c0059592d28c41d1e9985c848435e0ea17ea06c3` |
| `duckdb-1.5.6/libpg_query.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/libpg_query/LICENSE | `2780b6375a5ab1901472b7694958dd0c8c1381a21f0765cbb64e1923d94c6717` |
| `duckdb-1.5.6/lz4.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/lz4/LICENSE | `99b8d2d8011d1ce548c5aaf3270c6bee014507fa821c4f9a7e300a9e11015db7` |
| `duckdb-1.5.6/mbedtls.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/mbedtls/LICENSE | `9b405ef4c89342f5eae1dd828882f931747f71001cfba7d114801039b52ad09b` |
| `duckdb-1.5.6/miniz.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/miniz/LICENSE (CRLF upstream; stored with LF, as `.gitattributes` requires) | `6f20fa7672b00e2e975c291df737cf227addf3ad32e36fef3fe0f416e4664d3d` |
| `duckdb-1.5.6/parquet.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/parquet/LICENSE | `70b6a7d630fae9f49643f9b16bee42a2feda68301f9c14470e7fb5ad0599bb9f` |
| `duckdb-1.5.6/pcg.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/pcg/LICENSE | `3febf788fd814bf1f350db169bdc51b28ff7e7596ff1f09cc0181e4fde42b480` |
| `duckdb-1.5.6/pdqsort.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/pdqsort/LICENSE | `323fc18e35fcf058d69d684022ef844f206cb8fefb2bac1298fa1767e0b9ae38` |
| `duckdb-1.5.6/re2.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/re2/LICENSE | `6040cda75d90b1738292a631d89934c411ef7ffd543c4d6a1b7edfc8edf29449` |
| `duckdb-1.5.6/ska_sort.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/ska_sort/LICENSE | `3c3a838f6ac2201bb8255b374fad34e3e1de07940c3acdc09e661e118fb75078` |
| `duckdb-1.5.6/skiplist.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/skiplist/LICENSE | `3b9cac57dbb2d7ec76298cb0d33afe12f891e1d0dbb38ff17993a14f3480ee7c` |
| `duckdb-1.5.6/snappy.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/snappy/LICENSE | `55172044f7e241207117448a4d9d6ba1d0925c8ad66b5d4c08c70adfa9cc3de6` |
| `duckdb-1.5.6/tdigest.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/tdigest/LICENSE | `43070e2d4e532684de521b885f385d0841030efa2b1a20bafb76133a5e1379c1` |
| `duckdb-1.5.6/utf8proc.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/utf8proc/LICENSE | `fbc720fcc90fb44396985646f67e4d19be03e0c0539f7138ed9dc6efba49dc49` |
| `duckdb-1.5.6/vergesort.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/vergesort/LICENSE | `12dd392860ea2a3633bce5f33d7eebd4308a7e3386b4372260e5e72be7a64033` |
| `duckdb-1.5.6/yyjson.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/yyjson/LICENSE | `734d7fb3eb56f0a04e6b7ab0c5d3634624a8bfaa2acc87a2c03a029b20868cf8` |
| `duckdb-1.5.6/zstd.LICENSE` | https://raw.githubusercontent.com/duckdb/duckdb/v1.5.6/third_party/zstd/LICENSE | `8b7bc50ec3ecee27224e17d4d316a939fa2ddda9c88e0ddc2f059ce432edbfb9` |
| `htmx-2.0.4/LICENSE` | npm: htmx.org@2.0.4, package/LICENSE | `d3d2456f76414f2456104660ebd65aff1c04cd7966b942bdabd63f3cdb316a38` |
