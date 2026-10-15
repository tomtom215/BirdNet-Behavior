# Known findings

Inputs the fuzz harnesses found that are understood and not (yet) fixable here.
Each is one file, named for what it hits.

## `symphonia-riff-fmt-block-align-overflow.bin` — `decode_audio`

37 bytes: the selector byte `0` (WAV), then a 36-byte RIFF/WAVE header whose PCM
`fmt ` chunk declares 32 769 channels at 16 bits per sample.

In `symphonia-format-riff` 0.6.1, `WaveFormatChunk::read_pcm_fmt`
(`src/wave/chunks.rs:100`) computes `num_channels * (bits_per_sample / 8)` in
`u16` — 32 769 × 2 = 65 538 — only to log when it differs from the file's block
alignment. With overflow checks that is a panic; without them it wraps and the
log line is wrong.

- `cargo fuzz run decode_audio fuzz/known/symphonia-riff-fmt-block-align-overflow.bin`
  (cargo-fuzz's default, debug assertions on): panics, "attempt to multiply with
  overflow". Found by the first 60-second run.
- `cargo fuzz run -O decode_audio …` (what ships: a release build, no overflow
  checks): returns normally. This was run against the same file.

`.github/workflows/fuzz.yml` therefore fuzzes with `-O`. The shipped binary is
`panic = "abort"`, so what has to be ruled out is an input that aborts the
daemon as built, and a harmless wrap is not that. The defect is upstream's and
this repository cannot patch it; a report to symphonia should quote the bytes
above. Retest after any symphonia bump.
