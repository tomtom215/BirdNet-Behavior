#!/usr/bin/env bash
# Build the ffmpeg the mutation workflow's tests run against, into $1/bin.
#
# Why a build and not `apt-get install ffmpeg`: every mutation shard used to
# install Ubuntu's ffmpeg (6.1.1, 63 MB of packages) from the runner's mirror,
# and on 2026-11-24 the mirror served it at a few kB/s, so every shard of two
# pull requests timed out in that step after 15 minutes without running a
# mutant. This script runs once per change to itself, in the workflow's
# `ffmpeg` job, and the binary is cached; the shards restore it and touch no
# mirror.
#
# What it builds: FFmpeg's latest stable release, from ffmpeg.org's tarball,
# pinned by SHA-256 below. The tarball was checked against FFmpeg's release
# signing key (FCF9 86EA 15E6 E293 A564 4F10 B432 2F04 D676 58D8) when the pin
# was written; the hash is what CI enforces. It has FFmpeg's default built-in
# codecs, formats and filters, plus libmp3lame and libvorbis, which
# `audio/extraction/convert.rs` uses for MP3 and Ogg clips. Auto-detection of
# other system libraries is off, and the binary is linked statically, so it
# needs nothing installed where it runs. Built on Ubuntu 24.04 with 4 cores, it
# takes about three minutes.
#
# To move to a newer release: change FFMPEG_VERSION and FFMPEG_SHA256, check
# the new tarball's `.asc` against the key above, and run the ffmpeg-gated
# tests against the result (`BNB_REQUIRE_FFMPEG=1` with it first on PATH).
set -euo pipefail

FFMPEG_VERSION=9.1
FFMPEG_SHA256=1a4d394ae63d190a526cd052eb4415c156b5c5e2df37efab41a7de0b8fde2433

prefix=${1:?usage: build-ffmpeg.sh <install prefix>}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

apt_opts=(-o Acquire::Retries=3 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20)
sudo apt-get "${apt_opts[@]}" update
sudo apt-get "${apt_opts[@]}" install -y --no-install-recommends \
  nasm pkg-config libmp3lame-dev libvorbis-dev libogg-dev

tarball="$work/ffmpeg-$FFMPEG_VERSION.tar.xz"
curl --proto '=https' --tlsv1.2 --retry 5 --retry-connrefused --location \
  --silent --show-error --fail --output "$tarball" \
  "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz"
echo "$FFMPEG_SHA256  $tarball" | sha256sum --check --strict

mkdir "$work/src"
tar -xJf "$tarball" -C "$work/src" --strip-components=1
cd "$work/src"
./configure --prefix="$prefix" \
  --disable-autodetect --disable-doc --disable-ffplay \
  --enable-libmp3lame --enable-libvorbis \
  --pkg-config-flags=--static --extra-ldflags=-static
make -j"$(nproc)"
make install

"$prefix/bin/ffmpeg" -hide_banner -version | head -1
for encoder in libmp3lame libvorbis flac; do
  "$prefix/bin/ffmpeg" -hide_banner -encoders | grep -qw "$encoder" \
    || { echo "ffmpeg was built without the $encoder encoder" >&2; exit 1; }
done
"$prefix/bin/ffmpeg" -hide_banner -filters | grep -qw afreqshift \
  || { echo "ffmpeg was built without the afreqshift filter" >&2; exit 1; }
