# Fuzz harnesses

Coverage-guided fuzzing (libFuzzer via [`cargo-fuzz`]) for the parsers that
consume fully untrusted input:

| Target         | Surface                                                            |
| -------------- | ------------------------------------------------------------------ |
| `decode_audio` | `birdnet_core::audio::decode::decode_file_capped` — symphonia WAV/FLAC/MP3 demuxing of files dropped into the watch directory |
| `parse_labels` | `birdnet_core::inference::labels::LabelSet::{parse, parse_csv}` — user-supplied (custom/translated) label files |

## Running

Requires a nightly toolchain (libFuzzer instrumentation):

```bash
cargo install cargo-fuzz
cargo +nightly fuzz run decode_audio -- -max_total_time=300
cargo +nightly fuzz run parse_labels -- -max_total_time=300
```

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
