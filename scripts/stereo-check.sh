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
# ...or let it choose everything: bare --scan finds the recordings directory
# itself (from RECS_DIR in birdnet.conf, falling back to the usual locations),
# keeps the stereo segments, and analyses whichever has the loudest three
# seconds in it — the closest thing to "a segment with a bird in it" without
# asking a human to listen to a day of audio:
#
#   ./stereo-check.sh --scan
#
# Pass a directory to override the search:
#
#   ./stereo-check.sh --scan /some/where
#
# And if the source is set to Mono, --alsa-test answers the question that
# remains: arecord then asks plughw for ONE channel from a TWO-capsule mic, and
# ALSA's plug layer decides what that means. If it picks a channel, nothing is
# lost. If it mixes them, the cancellation happens inside ALSA, before anything
# is written, where no part of this project can see it:
#
#   sudo systemctl stop birdnet-behavior
#   ./stereo-check.sh --alsa-test plughw:1,0 10
#   sudo systemctl start birdnet-behavior
#
# It records both ways back to back, so it compares levels across two
# consecutive captures rather than sample-for-sample: play something steady
# while it runs, and it will say "too quiet to tell" rather than guess.
#
# Both file modes only work if the source is configured Stereo. A Mono source
# writes single-channel segments, and the extracted detection clips are always
# mono whatever the source is — the second channel is gone by then.
#
# The comparison runs on the loudest 3-second window, not on the whole
# recording. BirdNET analyses 3-second chunks, so that window is what actually
# reaches the model — and a whole-file average buries a two-second call in eight
# seconds of silence, which is precisely the case this is meant to judge.

set -euo pipefail

command -v python3 >/dev/null || { echo "python3 not found." >&2; exit 2; }

# Refuse before recording rather than after. An ALSA capture device is
# exclusive, so a running station guarantees `Device or resource busy` — and
# discovering that from arecord's error, ten seconds in, reads like a hardware
# fault instead of the two commands that fix it.
SERVICE="${BIRDNET_SERVICE:-birdnet-behavior}"
require_device_free() {
  command -v systemctl >/dev/null || return 0
  systemctl is-active --quiet "$SERVICE" 2>/dev/null || return 0
  echo "The station is running, and it holds the microphone exclusively." >&2
  echo >&2
  echo "Run the whole thing in one go — the service comes back even if the" >&2
  echo "check fails, because these are separated by ';' and not '&&':" >&2
  echo >&2
  echo "  sudo systemctl stop $SERVICE; $0 $*; sudo systemctl start $SERVICE" >&2
  echo >&2
  exit 2
}

WAV=""
TMP_WAV=""
MONO_WAV=""
ALSA_TEST=0
ERR="$(mktemp -t stereo-check-XXXXXX.err)"
LIST="$(mktemp -t stereo-check-XXXXXX.lst)"
# `return 0` is load-bearing: a trap's exit status becomes the script's, so
# ending on a failed `[ -n "$TMP_WAV" ]` turned every deliberate `exit 2` into
# a 1 and made "device busy" indistinguishable from "wrong usage".
cleanup() {
  rm -f "$ERR" "$LIST"
  [ -n "$TMP_WAV" ] && rm -f "$TMP_WAV"
  [ -n "$MONO_WAV" ] && rm -f "$MONO_WAV"
  return 0
}
trap cleanup EXIT

SCAN_DIR=""
SCAN_COUNT=40

if [ "${1:-}" = "--file" ]; then
  WAV="${2:-}"
  [ -n "$WAV" ] || { echo "--file needs a path." >&2; exit 2; }
  [ -r "$WAV" ] || { echo "cannot read $WAV" >&2; exit 2; }
  echo "Analysing ${WAV} ..."
elif [ "${1:-}" = "--alsa-test" ]; then
  # The question this answers: a source set to Mono makes arecord ask plughw
  # for ONE channel from a TWO-capsule microphone, and ALSA's plug layer decides
  # what that means. If it picks channel 0, nothing is lost. If it mixes L+R,
  # the comb filtering happens inside ALSA, before anything is written, where no
  # part of this project can see it.
  #
  # Recording both ways back to back and comparing levels tells them apart:
  # a mono capture that matches the left channel means ALSA picks, one that
  # matches the average of the two means ALSA mixes.
  ALSA_DEVICE="${2:-plughw:1,0}"
  ALSA_SECS="${3:-10}"
  command -v arecord >/dev/null || {
    echo "arecord not found — install alsa-utils." >&2
    exit 2
  }
  require_device_free "$@"
  MONO_WAV="$(mktemp -t stereo-check-mono-XXXXXX.wav)"
  TMP_WAV="$(mktemp -t stereo-check-XXXXXX.wav)"
  WAV="$TMP_WAV"
  echo "Recording ${ALSA_SECS}s at 1 channel (what a Mono source captures) ..."
  arecord -D "$ALSA_DEVICE" -f S16_LE -c 1 -r 48000 -d "$ALSA_SECS" \
    -t wav "$MONO_WAV" 2>"$ERR" || {
      echo "arecord failed — is the station still running? (systemctl stop birdnet-behavior)" >&2
      sed 's/^/  arecord: /' "$ERR" >&2 || true
      exit 2
    }
  echo "Recording ${ALSA_SECS}s at 2 channels (the capsules themselves) ..."
  arecord -D "$ALSA_DEVICE" -f S16_LE -c 2 -r 48000 -d "$ALSA_SECS" \
    -t wav "$WAV" 2>"$ERR" || {
      echo "arecord could not open 2 channels on ${ALSA_DEVICE}." >&2
      echo "That means it is not presenting a stereo capture, so a Mono source" >&2
      echo "is not mixing anything and there is nothing to worry about." >&2
      sed 's/^/  arecord: /' "$ERR" >&2 || true
      exit 2
    }
  PY_ARGS=(--alsa-test "$MONO_WAV" "$WAV")
  ALSA_TEST=1
elif [ "${1:-}" = "--scan" ]; then
  SCAN_DIR="${2:-}"
  if [ -z "$SCAN_DIR" ]; then
    # Where the segments live is a config value, not a constant, and it differs
    # from BirdNET-Pi's layout. Ask the config first, then try the defaults this
    # project and its predecessor install with.
    CONF="${BIRDNET_CONF:-/etc/birdnet/birdnet.conf}"
    if [ -r "$CONF" ]; then
      FROM_CONF="$(sed -n 's/^[[:space:]]*RECS_DIR[[:space:]]*=[[:space:]]*//p' "$CONF" \
                   | tail -n1 | tr -d '"'"'"'' | sed "s#^~#$HOME#")"
      [ -n "$FROM_CONF" ] && [ -d "$FROM_CONF" ] && SCAN_DIR="$FROM_CONF"
    fi
  fi
  if [ -z "$SCAN_DIR" ]; then
    for candidate in \
      "$HOME/BirdNet-Behavior/recordings" \
      /var/lib/birdnet/recs \
      /var/lib/birdnet/recordings \
      "$HOME/BirdSongs/StreamData" \
      /tmp/StreamData
    do
      [ -d "$candidate" ] && { SCAN_DIR="$candidate"; break; }
    done
  fi
  if [ -z "$SCAN_DIR" ]; then
    echo "Could not find a recordings directory." >&2
    echo >&2
    echo "Looked at RECS_DIR in ${BIRDNET_CONF:-/etc/birdnet/birdnet.conf}, then:" >&2
    echo "  $HOME/BirdNet-Behavior/recordings" >&2
    echo "  /var/lib/birdnet/recs" >&2
    echo "  /var/lib/birdnet/recordings" >&2
    echo "  $HOME/BirdSongs/StreamData" >&2
    echo "  /tmp/StreamData" >&2
    echo >&2
    echo "Find yours with:  grep RECS_DIR ${BIRDNET_CONF:-/etc/birdnet/birdnet.conf}" >&2
    echo "then:             ./stereo-check.sh --scan /that/path" >&2
    exit 2
  fi
  [ -d "$SCAN_DIR" ] || { echo "not a directory: $SCAN_DIR" >&2; exit 2; }
  [ -n "${3:-}" ] && SCAN_COUNT="$3"
  echo "Scanning the ${SCAN_COUNT} most recent segments in ${SCAN_DIR} ..."
else
  # An unrecognised flag must not fall through to "that is the device name".
  # It did, and the result was `arecord -D --alsa-test -d plughw:1,0` reported
  # as "invalid duration argument" — which reads like a broken microphone
  # rather than what it was: an older copy of this script that predated the
  # option being passed to it.
  case "${1:-}" in
    --*)
      echo "unknown option: $1" >&2
      echo >&2
      echo "This copy understands: --scan [DIR], --file PATH, --alsa-test [DEVICE] [SECS]," >&2
      echo "or a bare ALSA device name to record from." >&2
      echo >&2
      echo "If you expected that option to exist, this copy is out of date. Re-fetch:" >&2
      echo "  curl -fsSL -o stereo-check.sh https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/claude/birdnet-analytics-dashboards-1iuyqt/scripts/stereo-check.sh" >&2
      exit 2
      ;;
  esac

  DEVICE="${1:-plughw:1,0}"
  SECONDS_TO_RECORD="${2:-10}"
  RATE="${3:-48000}"

  command -v arecord >/dev/null || {
    echo "arecord not found — install alsa-utils." >&2
    echo "This says nothing about the microphone; nothing was opened." >&2
    exit 2
  }

  require_device_free "$@"
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

# One invocation either way: the analyser takes a single path, or `--scan` and a
# NUL-separated list to choose from.
if [ "$ALSA_TEST" -eq 0 ]; then
  PY_ARGS=("$WAV")
fi
if [ -n "$SCAN_DIR" ]; then
  # NUL-separated so names with spaces or colons survive — BirdNET-Pi-style
  # segment filenames contain colons.
  find "$SCAN_DIR" -maxdepth 3 -type f -name '*.wav' -printf '%T@ %p\0' 2>/dev/null \
    | sort -zrn | head -zn "$SCAN_COUNT" | sed -z 's/^[^ ]* //' > "$LIST" || true
  if [ ! -s "$LIST" ]; then
    echo "No .wav files found in $SCAN_DIR" >&2
    exit 2
  fi
  PY_ARGS=(--scan "$LIST")
fi

python3 - "${PY_ARGS[@]}" <<'PY'
import array, math, sys, wave

CHUNK_SECS = 3.0   # what BirdNET analyses at a time
HOP_SECS = 0.5

def peak_and_channels(path):
    """Peak absolute sample and channel count, or None if unreadable.

    `max`/`min` on an `array` run at C speed, so a whole directory can be
    triaged without the per-sample Python loop the full analysis needs.
    """
    try:
        with wave.open(path, "rb") as w:
            if w.getsampwidth() != 2:
                return None
            ch = w.getnchannels()
            data = array.array("h")
            data.frombytes(w.readframes(w.getnframes()))
    except (wave.Error, OSError, EOFError):
        return None
    if not data:
        return None
    return max(max(data), -min(data)), ch


def mono_samples(path):
    with wave.open(path, "rb") as w:
        data = array.array("h")
        data.frombytes(w.readframes(w.getnframes()))
        return data, w.getnchannels()


def plain_rms(xs):
    if not xs:
        return 0.0
    return math.sqrt(sum(float(x) * x for x in xs) / len(xs))


if sys.argv[1] == "--alsa-test":
    mono, mono_ch = mono_samples(sys.argv[2])
    stereo, st_ch = mono_samples(sys.argv[3])
    if st_ch != 2:
        print("\nThe 2-channel capture came back with "
              f"{st_ch} channel(s); nothing to compare.")
        sys.exit(0)
    left = stereo[0::2]
    right = stereo[1::2]
    m = plain_rms(mono)
    l = plain_rms(left)
    r = plain_rms(right)
    mixed = plain_rms(array.array("h", [(a + b) // 2 for a, b in zip(left, right)]))
    full = 32768.0

    def d(x):
        return 20.0 * math.log10(x / full) if x > 0 else -99.0

    print(f"\n1-channel capture      {m:8.1f}   {d(m):6.1f} dBFS")
    print(f"2-channel, left        {l:8.1f}   {d(l):6.1f} dBFS")
    print(f"2-channel, right       {r:8.1f}   {d(r):6.1f} dBFS")
    print(f"2-channel, mixed L+R   {mixed:8.1f}   {d(mixed):6.1f} dBFS")
    print()

    # Which candidate the 1-channel capture resembles. The two captures are
    # consecutive, not simultaneous, so this compares levels in a steady
    # ambient rather than sample-for-sample.
    to_left = abs(d(m) - d(l))
    to_mixed = abs(d(m) - d(mixed))
    spread = abs(d(l) - d(mixed))

    if max(m, l, r) < full * 0.003:
        print("VERDICT: too quiet to tell.")
        print("  Every capture is near the noise floor. Re-run with a steady")
        print("  sound playing — a tone or continuous noise from a phone a metre")
        print("  away is ideal, because this compares levels across two")
        print("  consecutive recordings and needs the room to hold still.")
    elif spread < 1.0:
        print("VERDICT: cannot distinguish — and it does not matter.")
        print(f"  Taking one channel and mixing both differ by only {spread:.1f} dB here,")
        print("  so whichever ALSA does, the result is the same. Your capsules are")
        print("  close enough together that mixing costs nothing.")
    elif to_left <= to_mixed:
        print("VERDICT: ALSA picks a channel — nothing is being mixed.")
        print(f"  The 1-channel capture sits {to_left:.1f} dB from a single channel and")
        print(f"  {to_mixed:.1f} dB from the mix, so asking for mono gives you one capsule.")
        print("  A Mono source on this station is not losing anything to cancellation.")
    else:
        print("VERDICT: ALSA is mixing both capsules.")
        print(f"  The 1-channel capture sits {to_mixed:.1f} dB from the mix and {to_left:.1f} dB")
        print("  from a single channel — so the averaging is happening inside ALSA,")
        print("  before anything is written, where nothing in this project can see it.")
        print()
        print(f"  That costs {spread:.1f} dB against one capsule on this sound. To avoid it,")
        print("  set the source to Left or Right (needs the build where those work)")
        print("  so the device is opened with two channels and one is selected.")
    sys.exit(0)


if sys.argv[1] == "--scan":
    with open(sys.argv[2], "rb") as fh:
        paths = [p.decode("utf-8", "replace") for p in fh.read().split(b"\0") if p]
    stereo, mono = [], 0
    for path in paths:
        got = peak_and_channels(path)
        if got is None:
            continue
        peak, ch = got
        if ch == 2:
            stereo.append((peak, path))
        else:
            mono += 1
    if not stereo:
        print(f"\nLooked at {len(paths)} segment(s): none are stereo"
              f"{f' ({mono} are mono)' if mono else ''}.")
        print()
        print("So this station is already recording a single channel, and the")
        print("second one is gone before anything is written to disk. Averaging")
        print("cannot be costing you signal in these files.")
        print()
        print("To find out what the capsules themselves are doing, record fresh:")
        print("    sudo systemctl stop birdnet-behavior")
        print("    ./stereo-check.sh plughw:1,0 10")
        print("    sudo systemctl start birdnet-behavior")
        sys.exit(0)
    stereo.sort(reverse=True)
    chosen = stereo[0][1]
    print(f"\n{len(stereo)} stereo segment(s) found; loudest is:\n  {chosen}")
    target = chosen
else:
    target = sys.argv[1]

with wave.open(target, "rb") as w:
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
