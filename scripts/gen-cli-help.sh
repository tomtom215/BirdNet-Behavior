#!/usr/bin/env bash
# Regenerate the CLI reference snippet embedded in the documentation book.
#
# The book's reference pages include docs/book/_generated/cli-help.txt, which is
# the verbatim `--help` output of the binary. Run this after changing any clap
# argument in src/cli.rs, then commit the result. CI (.github/workflows/ci.yml)
# runs this and fails if the committed file is stale, so the docs cannot drift
# from the actual flags/env vars/defaults.
#
# Output is deterministic: redirecting stdout to a file is non-TTY, so clap
# renders at its fixed 100-column fallback width. Trailing whitespace is
# stripped so the result matches the repo's pre-commit whitespace-hygiene hook
# (clap emits trailing spaces on blank continuation lines; the hook would strip
# them on commit, which would otherwise leave the CI drift-gate permanently red).

set -euo pipefail

cd "$(dirname "$0")/.."

out="docs/book/_generated/cli-help.txt"

# `--quiet` keeps cargo's build chatter on stderr only, so stdout is purely the
# help text. `--help` is intercepted by clap before main runs, so no config,
# model, or audio device is required.
cargo run --quiet --bin birdnet-behavior -- --help \
    | sed 's/[[:space:]]*$//' > "${out}"

echo "Wrote ${out}"
