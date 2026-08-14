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
# Needs only arecord and python3 (no numpy). Stop the station first: an ALSA
# capture device is exclusive.
#
#   sudo systemctl stop birdnet-behavior
#   ./stereo-check.sh plughw:1,0 10
#   sudo systemctl start birdnet-behavior
#
# Run it while birds are actually singing. The cancellation is
# direction-dependent, so ambient noise can look benign on a microphone that
# loses badly on real song.

set -euo pipefail

DEVICE="${1:-plughw:1,0}"
SECONDS_TO_RECORD="${2:-10}"
RATE="${3:-48000}"
WAV="$(mktemp -t stereo-check-XXXXXX.wav)"
ERR="$(mktemp -t stereo-check-XXXXXX.err)"
trap 'rm -f "$WAV" "$ERR"' EXIT

command -v arecord >/dev/null || {
  echo "arecord not found — install alsa-utils." >&2
  echo "This says nothing about the microphone; nothing was opened." >&2
  exit 2
}
command -v python3 >/dev/null || { echo "python3 not found." >&2; exit 2; }

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

python3 - "$WAV" <<'PY'
import array, math, sys, wave

with wave.open(sys.argv[1], "rb") as w:
    if w.getnchannels() != 2:
        print(f"\nThis device gave {w.getnchannels()} channel(s), not 2.")
        print("It is not delivering a stereo signal, so averaging cannot be")
        print("costing you anything. Nothing to change.")
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

def rms(xs):
    if not xs:
        return 0.0
    return math.sqrt(sum(float(x) * x for x in xs) / len(xs))

def db(a, b):
    if a <= 0 or b <= 0:
        return 0.0
    return 20.0 * math.log10(a / b)

l_rms = rms(left)
r_rms = rms(right)
avg = array.array("h", [(a + b) // 2 for a, b in zip(left, right)])
a_rms = rms(avg)
best = max(l_rms, r_rms)
better = "Left" if l_rms >= r_rms else "Right"
loss = db(a_rms, best)

full = 32768.0
print(f"\n{frames/rate:.1f}s at {rate} Hz\n")
print("Levels (RMS, dBFS)")
print(f"  left                     {l_rms:8.1f}   {db(l_rms, full):6.1f} dBFS")
print(f"  right                    {r_rms:8.1f}   {db(r_rms, full):6.1f} dBFS")
print(f"  averaged (what you feed) {a_rms:8.1f}   {db(a_rms, full):6.1f} dBFS")
print()
print(f"Averaging vs the better single channel ({better}): {loss:+.1f} dB")
print()

if max(l_rms, r_rms) < 30:
    print("VERDICT: too quiet to judge.")
    print("  Both channels are near silence, so the comparison means nothing.")
    print("  Check the mic is plugged in and gain is set, then re-run while")
    print("  there is actual sound.")
elif min(l_rms, r_rms) < best * 0.05:
    print("VERDICT: one channel is effectively dead.")
    print(f"  {'Right' if l_rms > r_rms else 'Left'} is {db(min(l_rms,r_rms), best):.0f} dB below the other.")
    print("  Averaging a live channel with a dead one costs you ~6 dB for nothing.")
    print(f"  Set this source to {better} on the Audio page.")
elif loss < -1.0:
    print("VERDICT: averaging is costing you signal.")
    print("  The two capsules are far enough apart to cancel. You are losing")
    print(f"  {-loss:.1f} dB against just using {better}.")
    print(f"  Set this source to {better} on the Audio page (needs the build that")
    print("  makes Left/Right actually work — before that they behaved as Mono).")
else:
    print("VERDICT: averaging is fine here.")
    print("  The capsules are coincident enough that averaging loses nothing")
    print("  measurable. Leave the source on Stereo (or Mono, same result).")
    print()
    print("  Worth one more run while a bird is singing close by: cancellation")
    print("  depends on direction, and ambient noise is the friendliest case.")
PY
