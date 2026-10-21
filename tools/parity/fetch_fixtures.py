#!/usr/bin/env python3
"""Download, derive and verify the parity fixture clips described in fixtures.json.

    python -I fetch_fixtures.py OUT_DIR [--cache DIR] [--only ID,ID] [--pin] [--strict]

* Every download is verified against the sha256 in fixtures.json; a mismatch is
  fatal (the upstream file changed).
* Clips are then built by the recipe in fixtures.json: a straight copy, a
  sample-exact slice of a PCM WAV (stdlib ``wave``), an ``ffmpeg`` decode /
  resample, or the synthetic noise clip (numpy).  Output sha256 is compared
  with the pinned value; a mismatch is a WARNING (exit 2 with --strict)
  because FLAC/MP3 decodes and swr resamples are only guaranteed bit-exact
  for the ffmpeg version used to pin them.  Whatever you build, the golden
  JSON records the sha256 of the clip it actually analysed.
* --pin writes the observed sha256/format of every built clip back into
  fixtures.json (maintainer use).

Requires: Python >= 3.12, numpy, and /usr/bin/ffmpeg (or ffmpeg on PATH).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
import wave

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
MANIFEST = os.path.join(HERE, "fixtures.json")  # --manifest overrides (fixtures_extended.json is the second set)
UA = "BirdNet-Behavior-parity-fixtures/0.1 (fixture fetch)"


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def sha1_file(path: str) -> str:
    h = hashlib.sha1()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def download(url: str, dest: str, expect: str | None, expect_sha1: str | None = None) -> None:
    if os.path.exists(dest) and expect and sha256_file(dest) == expect:
        return
    tmp = dest + ".part"
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    for attempt in range(4):  # archive.org answers 500/503 for a moment now and then
        try:
            with urllib.request.urlopen(req, timeout=600) as r, open(tmp, "wb") as f:
                shutil.copyfileobj(r, f)
            break
        except urllib.error.HTTPError as e:
            if attempt == 3 or e.code < 500:
                raise
            time.sleep(5 * (attempt + 1))
    got = sha256_file(tmp)
    if expect_sha1 and sha1_file(tmp) != expect_sha1:
        os.unlink(tmp)
        raise SystemExit(f"FATAL: {url}\n  sha1 differs from the one archive.org lists ({expect_sha1}): corrupt download")
    if expect and got != expect:
        os.unlink(tmp)
        raise SystemExit(f"FATAL: {url}\n  sha256 {got}\n  expected {expect}\nthe upstream file changed")
    os.replace(tmp, dest)


def wav_slice(src: str, dst: str, start: float, dur: float) -> None:
    with wave.open(src, "rb") as r:
        params = r.getparams()
        rate = r.getframerate()
        a = int(round(start * rate))
        n = int(round(dur * rate))
        r.setpos(a)
        data = r.readframes(n)
    with wave.open(dst, "wb") as w:
        w.setnchannels(params.nchannels)
        w.setsampwidth(params.sampwidth)
        w.setframerate(rate)
        w.writeframes(data)


def ffmpeg(src: str, dst: str, spec: dict) -> None:
    exe = shutil.which("ffmpeg")
    if not exe:
        raise SystemExit("ffmpeg not found on PATH")
    cmd = [exe, "-hide_banner", "-loglevel", "error", "-nostdin", "-y"]
    if "ss" in spec:
        cmd += ["-ss", repr(float(spec["ss"]))]
    if "t" in spec:
        cmd += ["-t", repr(float(spec["t"]))]
    cmd += ["-i", src]
    if "af" in spec:
        cmd += ["-af", spec["af"]]
    if "ar" in spec:
        cmd += ["-ar", str(spec["ar"])]
    # bitexact: no LIST/INFO "Lavf<version>" chunk, so the header does not vary with the ffmpeg build
    cmd += ["-map_metadata", "-1", "-fflags", "+bitexact", "-flags:a", "+bitexact", "-c:a", spec["codec"], dst]
    subprocess.run(cmd, check=True)


def _write_mono16(dst: str, samples: "np.ndarray", rate: int = 48000) -> None:
    with wave.open(dst, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(samples.astype("<i2").tobytes())


def synthetic(dst: str, name: str = "noise_dither_zero_9s") -> None:
    """The generated clips. The default is the 9 s clip of the first set: 3 s white noise at -20 dBFS RMS,
    3 s of +-1 LSB dither, 3 s of zeros; 48 kHz mono s16.  The others (30 s, same format) are for the
    extended set: pink noise at -20 dBFS RMS, a 1 kHz tone at -12 dBFS, a clipped square wave and digital silence."""
    rs = np.random.RandomState(20240224)  # legacy stream: frozen by numpy's compatibility policy
    if name == "noise_dither_zero_9s":
        n = 3 * 48000
        noise = np.clip(np.rint(rs.standard_normal(n) * 0.1 * 32768.0), -32768, 32767).astype("<i2")
        dither = rs.randint(-1, 2, size=n).astype("<i2")
        zeros = np.zeros(n, dtype="<i2")
        _write_mono16(dst, np.concatenate([noise, dither, zeros]))
        return
    n = 30 * 48000
    if name == "pink_noise_30s":
        spec = np.fft.rfft(rs.standard_normal(n))
        spec[1:] /= np.sqrt(np.arange(1, len(spec)))
        x = np.fft.irfft(spec, n)
        x *= 0.1 * 32768.0 / np.sqrt(np.mean(x**2))
    elif name == "tone_1k_30s":
        x = 0.25 * 32768.0 * np.sin(2 * np.pi * 1000.0 * np.arange(n) / 48000.0)
    elif name == "clipped_square_30s":
        x = np.where(np.sin(2 * np.pi * 440.0 * np.arange(n) / 48000.0) >= 0, 32767.0, -32768.0)
    elif name == "digital_silence_30s":
        x = np.zeros(n)
    else:
        raise SystemExit(f"unknown synthetic clip {name!r}")
    _write_mono16(dst, np.clip(np.rint(x), -32768, 32767))


def wav_info(path: str) -> dict:
    with wave.open(path, "rb") as r:
        return {
            "rate": r.getframerate(),
            "channels": r.getnchannels(),
            "bits": 8 * r.getsampwidth(),
            "frames": r.getnframes(),
            "duration_s": r.getnframes() / r.getframerate(),
            "bytes": os.path.getsize(path),
        }


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out_dir")
    ap.add_argument("--cache", help="where downloaded sources go (default OUT_DIR/.sources)")
    ap.add_argument("--only", help="comma-separated clip ids (dependencies are built too)")
    ap.add_argument("--manifest", help="clip manifest (default: fixtures.json next to this script)")
    ap.add_argument("--pin", action="store_true", help="write observed sha256/format back to fixtures.json")
    ap.add_argument("--strict", action="store_true", help="exit 2 if a derived clip's sha256 differs from the pin")
    a = ap.parse_args(argv)

    manifest = os.path.abspath(a.manifest) if a.manifest else MANIFEST
    with open(manifest, encoding="utf-8") as f:
        man = json.load(f)
    clips = {c["id"]: c for c in man["clips"]}
    want = [c["id"] for c in man["clips"]]
    if a.only:
        sel = set(a.only.split(","))
        need = set()

        def add(cid):
            need.add(cid)
            dep = clips[cid]["recipe"].get("from_clip")
            if dep:
                add(dep)

        for cid in sel:
            add(cid)
        want = [c for c in want if c in need]

    out = os.path.abspath(a.out_dir)
    cache = os.path.abspath(a.cache or os.path.join(out, ".sources"))
    os.makedirs(out, exist_ok=True)
    os.makedirs(cache, exist_ok=True)

    src_path: dict[str, str] = {}
    mismatches = 0
    for cid in want:
        c = clips[cid]
        r = c["recipe"]
        dst = os.path.join(out, cid + ".wav")
        if "from_clip" in r:
            src = os.path.join(out, r["from_clip"] + ".wav")
        elif c["source"] != "synthetic":
            sid = c["source"]
            if sid not in src_path:
                s = man["sources"][sid]
                src_path[sid] = os.path.join(cache, sid + os.path.splitext(s["url"].split("?")[0])[1])
                print(f"download {sid}: {s['url']}")
                download(s["url"], src_path[sid], s.get("sha256"), s.get("ia_sha1"))
                if a.pin and not s.get("sha256"):
                    s["sha256"] = sha256_file(src_path[sid])
            src = src_path[sid]
        else:
            src = None

        if r.get("copy"):
            shutil.copyfile(src, dst)
        elif r.get("synthetic"):
            synthetic(dst, r["synthetic"] if isinstance(r["synthetic"], str) else "noise_dither_zero_9s")
        elif "wav_slice" in r:
            wav_slice(src, dst, *r["wav_slice"])
        elif "ffmpeg" in r:
            ffmpeg(src, dst, r["ffmpeg"])
        else:
            raise SystemExit(f"clip {cid}: unknown recipe {r}")

        got = sha256_file(dst)
        info = wav_info(dst)
        pinned = c.get("sha256")
        status = "pinned" if a.pin else ("OK" if pinned == got else ("UNPINNED" if not pinned else "MISMATCH"))
        if status == "MISMATCH":
            mismatches += 1
        print(f"{status:9} {cid}.wav {info['rate']} Hz x{info['channels']} {info['bits']}-bit {info['duration_s']:.3f}s {info['bytes']} B sha256={got}")
        if a.pin:
            c.update(sha256=got, **info)

    if a.pin:
        with open(manifest, "w", encoding="utf-8") as f:
            json.dump(man, f, indent=2, ensure_ascii=False)
            f.write("\n")
        print("fixtures.json updated")
    if mismatches:
        print(f"WARNING: {mismatches} derived clip(s) differ from the pinned sha256 (different ffmpeg/numpy?); "
              "analyses will record the sha256 actually used.", file=sys.stderr)
        return 2 if a.strict else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
