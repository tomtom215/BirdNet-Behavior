#!/usr/bin/env bash
#
# stereo-check.sh — is this station's stereo microphone costing it signal?
#
# The BirdNET model takes one audio channel, so a stereo capture has to be
# reduced to mono before inference. BirdNet-Behavior (like BirdNET-Pi, which
# uses `librosa.load(mono=True)`) does that by averaging the two channels.
# For two capsules in the same place that is harmless. For two capsules a few
# centimetres apart it is a comb filter: the same wavefront reaches them at
# different times, and averaging cancels every frequency where they disagree in
# phase. Measured through the decoder, a half-period delay costs about 66 dB.
#
# Whether a given microphone lands in that case depends on its capsule spacing
# and on where the bird is, so it can only be measured on the station. This
# records both channels and compares what averaging delivers against what the
# better single channel delivers — the number that decides whether to set the
# source to Left/Right instead of Stereo.
#
# Needs only arecord and python3 (no numpy).
#
# Two ways to use it. Either record fresh — stop the station first, an ALSA
# capture device is exclusive:
#
#   sudo systemctl stop birdnet-behavior
#   ./stereo-check.sh plughw:1,0 10
#   sudo systemctl start birdnet-behavior
#
# ...or point it at a recording the station already made, which needs no
# downtime and lets you choose a segment you know has a bird in it:
#
#   ./stereo-check.sh --file ~/BirdSongs/StreamData/2026-08-14-birdnet-06:12:00.wav
#
# (That only works if the source is configured Stereo; a Mono source writes
# single-channel files and the second channel is already gone.)
#
# The comparison runs on the loudest 3-second window, not on the whole
# recording. BirdNET analyses 3-second chunks, so that window is what actually
# reaches the model — and a whole-file average buries a two-second call in eight
# seconds of silence, which is precisely the case this is meant to judge.

set -euo pipefail

command -v python3 >/dev/null || { echo "python3 not found." >&2; exit 2; }

WAV=""
TMP_WAV=""
ERR="$(mktemp -t stereo-check-XXXXXX.err)"
# `return 0` is load-bearing: a trap's exit status becomes the script's, so
# ending on a failed `[ -n "$TMP_WAV" ]` turned every deliberate `exit 2` into
# a 1 and made "device busy" indistinguishable from "wrong usage".
cleanup() {
  rm -f "$ERR"
  [ -n "$TMP_WAV" ] && rm -f "$TMP_WAV"
  return 0
}
trap cleanup EXIT

if [ "${1:-}" = "--file" ]; then
  WAV="${2:-}"
  [ -n "$WAV" ] || { echo "--file needs a path." >&2; exit 2; }
  [ -r "$WAV" ] || { echo "cannot read $WAV" >&2; exit 2; }
  echo "Analysing ${WAV} ..."
else
  DEVICE="${1:-plughw:1,0}"
  SECONDS_TO_RECORD="${2:-10}"
  RATE="${3:-48000}"

  command -v arecord >/dev/null || {
    echo "arecord not found — install alsa-utils." >&2
    echo "This says nothing about the microphone; nothing was opened." >&2
    exit 2
  }

  TMP_WAV="$(mktemp -t stereo-check-XXXXXX.wav)"
  WAV="$TMP_WAV"
  echo "Recording ${SECONDS_TO_RECORD}s of stereo from ${DEVICE} ..."
  if ! arecord -D "$DEVICE" -f S16_LE -c 2 -r "$RATE" -d "$SECONDS_TO_RECORD" \
       -t wav "$WAV" 2>"$ERR"; then
    echo >&2
    echo "arecord failed. The likeliest cause is that the station is running —" >&2
    echo "an ALSA capture device is exclusive:" >&2
    echo "    sudo systemctl stop birdnet-behavior" >&2
    echo >&2
    echo "Otherwise check 'arecord -l' for the device name. A device that cannot" >&2
    echo "open two channels is not a stereo source and none of this applies." >&2
    echo >&2
    sed 's/^/  arecord: /' "$ERR" >&2 || true
    exit 2
  fi
fi

python3 - "$WAV" <<'PY'
import array, math, sys, wave

CHUNK_SECS = 3.0   # what BirdNET analyses at a time
HOP_SECS = 0.5

with wave.open(sys.argv[1], "rb") as w:
    channels = w.getnchannels()
    if channels != 2:
        print(f"\nThis file has {channels} channel(s), not 2.")
        print("There is no second channel to average away, so averaging cannot be")
        print("costing you anything. (A source set to Mono writes single-channel")
        print("files - record fresh with this script to see what the capsules do.)")
        sys.exit(0)
    if w.getsampwidth() != 2:
        print(f"unexpected sample width {w.getsampwidth()}", file=sys.stderr)
        sys.exit(2)
    rate = w.getframerate()
    frames = w.getnframes()
    raw = w.readframes(frames)

samples = array.array("h")
samples.frombytes(raw)
left = samples[0::2]
right = samples[1::2]
n = min(len(left), len(right))


def db(a, b):
    if a <= 0 or b <= 0:
        return 0.0
    return 20.0 * math.log10(a / b)


# Prefix sums of squares, so any window's energy is two lookups. Ten seconds of
# 48 kHz audio is half a million samples and this script has no numpy; the
# alternative is re-summing every window and taking a minute over it.
def prefix(xs):
    acc = [0.0]
    total = 0.0
    for x in xs:
        total += float(x) * x
        acc.append(total)
    return acc


pl, pr = prefix(left[:n]), prefix(right[:n])


def window_rms(pfx, start, length):
    return math.sqrt(max(pfx[start + length] - pfx[start], 0.0) / length)


chunk = min(int(CHUNK_SECS * rate), n)
hop = max(int(HOP_SECS * rate), 1)
best_start, best_energy = 0, -1.0
for start in range(0, max(n - chunk, 0) + 1, hop):
    e = (pl[start + chunk] - pl[start]) + (pr[start + chunk] - pr[start])
    if e > best_energy:
        best_energy, best_start = e, start

# The loudest chunk is the one that matters: it is the audio a detection would
# have come from. A whole-file average is dominated by whatever silence
# surrounds the call.
l_rms = window_rms(pl, best_start, chunk)
r_rms = window_rms(pr, best_start, chunk)
avg_window = array.array(
    "h",
    [(left[i] + right[i]) // 2 for i in range(best_start, best_start + chunk)],
)
pa = prefix(avg_window)
a_rms = math.sqrt(max(pa[chunk] - pa[0], 0.0) / chunk)

overall_l = window_rms(pl, 0, n)
overall_r = window_rms(pr, 0, n)

best = max(l_rms, r_rms)
better = "Left" if l_rms >= r_rms else "Right"
loss = db(a_rms, best)
full = 32768.0

print(f"\n{n / rate:.1f}s at {rate} Hz")
print(f"Loudest {chunk / rate:.0f}s window starts at {best_start / rate:.1f}s\n")
print("Levels in that window (RMS, dBFS)")
print(f"  left                     {l_rms:8.1f}   {db(l_rms, full):6.1f} dBFS")
print(f"  right                    {r_rms:8.1f}   {db(r_rms, full):6.1f} dBFS")
print(f"  averaged (what you feed) {a_rms:8.1f}   {db(a_rms, full):6.1f} dBFS")
print(f"\nWhole recording, for context: left {db(overall_l, full):.1f} dBFS, "
      f"right {db(overall_r, full):.1f} dBFS")
print(f"\nAveraging vs the better single channel ({better}): {loss:+.1f} dB\n")

# A window this quiet is room tone. Comparing two noise floors says nothing
# about what happens to a bird call, and printing a dB figure for it would
# invite exactly the wrong conclusion.
QUIET_DBFS = -50.0
if db(best, full) < QUIET_DBFS:
    print("VERDICT: still too quiet to judge.")
    print(f"  Even the loudest 3s window peaks at {db(best, full):.0f} dBFS, which is")
    print("  room tone, not a bird. Comparing two noise floors says nothing about")
    print("  what averaging does to a call.")
    print()
    print("  Two ways forward:")
    print("    - Point this at a recording you know has a bird in it:")
    print("        ./stereo-check.sh --file ~/BirdSongs/.../<segment>.wav")
    print("    - Or make a sound yourself: play a bird call from a phone about a")
    print("      metre from the mic, off to one side, and re-run. Off-axis is the")
    print("      point - cancellation depends on direction.")
    print()
    print(f"  Separately: if {db(overall_l, full):.0f} dBFS is what this station sees all")
    print("  the time, that is worth a look on its own. It is a very low level to")
    print("  be classifying from, whatever the channels are doing.")
elif min(l_rms, r_rms) < best * 0.05:
    print("VERDICT: one channel is effectively dead.")
    quiet = "Right" if l_rms > r_rms else "Left"
    print(f"  {quiet} is {db(min(l_rms, r_rms), best):.0f} dB below the other.")
    print("  Averaging a live channel with a dead one throws away ~6 dB for nothing.")
    print(f"  Set this source to {better} on the Audio page - and check the mic.")
elif loss < -1.0:
    print("VERDICT: averaging is costing you signal.")
    print("  The capsules are far enough apart to cancel on this sound. You are")
    print(f"  losing {-loss:.1f} dB against just using {better}.")
    print(f"  Set this source to {better} on the Audio page (needs the build where")
    print("  Left/Right actually work - before that they behaved as Mono).")
else:
    print("VERDICT: averaging is fine on this sound.")
    print("  The capsules are coincident enough that averaging loses nothing")
    print("  measurable here. Leave the source on Stereo.")
    print()
    print("  Cancellation depends on direction, so this is one angle, not a")
    print("  guarantee. If you want to be thorough, re-run with a sound source")
    print("  off to one side.")
PY
