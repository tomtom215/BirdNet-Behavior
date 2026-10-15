# Fuzz harnesses

Coverage-guided fuzzing (libFuzzer via [`cargo-fuzz`]) for the parsers that
consume fully untrusted input:

| Target         | Surface                                                            |
| -------------- | ------------------------------------------------------------------ |
| `decode_audio` | `birdnet_core::audio::decode::decode_file_capped` — symphonia WAV/FLAC/MP3 demuxing of files dropped into the watch directory |
| `parse_labels` | `birdnet_core::inference::labels::LabelSet::{parse, parse_csv}` — user-supplied (custom/translated) label files |

## Running

Requires a nightly toolchain (libFuzzer instrumentation). Fuzz what ships: copy
the root lockfile in first, so the dependencies are the versions the release
contains, and pass `-O`, because the shipped binary has no overflow checks (see
[`known/README.md`](known/README.md) for the one finding cargo-fuzz's default
debug assertions produce):

```bash
cargo install --locked --version 0.13.2 cargo-fuzz
cp Cargo.lock fuzz/Cargo.lock
cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 > /dev/null
cargo +nightly fuzz run -O decode_audio -- -max_total_time=300 -timeout=30 -rss_limit_mb=2048
cargo +nightly fuzz run -O parse_labels -- -max_total_time=300 -timeout=30 -rss_limit_mb=2048
```

`.github/workflows/fuzz.yml` does exactly this, weekly and on pull requests that
touch a fuzzed surface. `fuzz/Cargo.lock` is deliberately not committed.

Seed the audio corpus from the committed fixture for much better coverage:

```bash
mkdir -p fuzz/corpus/decode_audio
# Prefix byte selects the demuxer (0=wav); see fuzz_targets/decode_audio.rs.
printf '\x00' | cat - tests/testdata/Pica_pica_30s.wav \
  > fuzz/corpus/decode_audio/seed-pica-wav
```

The harness crate is excluded from the workspace, so the regular build,
test, clippy, and mutation gates never compile it. Crash artifacts land in
`fuzz/artifacts/<target>/`; minimize with `cargo +nightly fuzz tmin` and
file the reproducer with the bug report.

[`cargo-fuzz`]: https://github.com/rust-fuzz/cargo-fuzz
