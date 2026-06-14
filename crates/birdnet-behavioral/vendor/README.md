# Vendored `behavioral` DuckDB extension binaries

These are the prebuilt [`duckdb-behavioral`](https://github.com/tomtom215/duckdb-behavioral)
community extension binaries, committed here so `LOAD behavioral` works **fully
offline** — in the sandbox, in CI without a registry round-trip, and on a fresh
Raspberry Pi that has never reached the internet.

`build.rs` embeds the copy matching the build *target* (`include_bytes!`), and
the runtime loader (`connection::mod::load_extension`) stages it to a temp file
and `LOAD '<path>'`s it as the final fallback after the cached-`LOAD` and
`INSTALL ... FROM community` stages.

## Files

| File | Platform | DuckDB target | Extension |
|------|----------|---------------|-----------|
| `behavioral-linux_amd64.duckdb_extension` | `linux_amd64` (x86_64) | v1.5.3 | v0.8.0 |
| `behavioral-linux_arm64.duckdb_extension` | `linux_arm64` (aarch64) | v1.5.3 | v0.8.0 |

The DuckDB target (v1.5.3) must match the engine the workspace bundles via the
`duckdb = "~1.10503"` pin in the root `Cargo.toml` — DuckDB refuses to `LOAD` an
extension built for any other engine version.

## Provenance

Downloaded from the **DuckDB community-extensions registry**, which is the same
source the runtime `INSTALL behavioral FROM community` stage uses:

```
http://community-extensions.duckdb.org/v1.5.3/linux_amd64/behavioral.duckdb_extension.gz
http://community-extensions.duckdb.org/v1.5.3/linux_arm64/behavioral.duckdb_extension.gz
```

> **Why the registry and not the GitHub release tarballs?** The v0.8.0
> `behavioral-v0.8.0-linux_arm64.tar.gz` GitHub-release asset ships an **x86_64**
> ELF mislabeled `linux_arm64` in its footer — it would fail to `dlopen` on a
> real aarch64 Pi. The registry's `linux_arm64` build is a genuine AArch64
> shared object. Always vendor from the registry.

## Refreshing (when the extension or DuckDB version changes)

```bash
ddb=v1.5.3   # must equal the bundled DuckDB engine version
for plat in linux_amd64 linux_arm64; do
  curl -sSL "http://community-extensions.duckdb.org/$ddb/$plat/behavioral.duckdb_extension.gz" \
    | gunzip > "crates/birdnet-behavioral/vendor/behavioral-$plat.duckdb_extension"
done
```

Confirm the footer of each file reports the expected extension + DuckDB
versions and the matching platform:

```bash
tail -c 512 behavioral-linux_arm64.duckdb_extension | strings | grep -E 'v0|v1|linux_'
# => v0.8.0 / v1.5.3 / linux_arm64
readelf -h behavioral-linux_arm64.duckdb_extension | grep Machine  # => AArch64
```
