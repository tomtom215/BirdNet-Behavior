#!/usr/bin/env python3
"""How BirdNET-Pi reads a recording's file name (`classes.ParseFileName`), for a list of names good and bad.

    python -I file_names_ref.py --pi-root PI --out golden/file_names.json

The name carries the recording's date, time and RTSP id, and BirdNET-Pi derives everything else (stored date and time, the
week passed to the range filter) from it.  `tests/file_names_match_birdnet_pi.rs` reads the same names with
`RecordingFile::parse`.
"""
import argparse
import importlib.util
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True

NAMES = [
    "2026-02-16-birdnet-12:00:00.wav",
    "2026-02-16-birdnet-RTSP_1-12:00:00.wav",
    "2026-02-16-birdnet-RTSP_12-12:00:00.wav",
    "2024-02-29-birdnet-23:59:59.wav",
    "2026-12-31-birdnet-23:59:59.wav",
    "2027-01-01-birdnet-00:00:00.wav",
    "2026-12-28-birdnet-00:00:01.wav",
    "2026-01-01-birdnet-00:00:00.wav",
    "2020-12-31-birdnet-12:00:00.wav",
    "2021-01-03-birdnet-12:00:00.wav",
    "2026-02-16-birdnet-12:00:00.WAV",
    "2026-02-16-birdnet-12:00:00.flac",
    "2026-02-16-birdnet-12:00:00.mp3",
    "2026-2-16-birdnet-12:00:00.wav",
    "2026-02-16-birdnet-1:02:03.wav",
    "2026-02-30-birdnet-12:00:00.wav",
    "2026-13-01-birdnet-12:00:00.wav",
    "2025-02-29-birdnet-12:00:00.wav",
    "2026-02-16-birdnet-25:00:00.wav",
    "2026-02-16-birdnet-12:60:00.wav",
    "2026-02-16-birdnet-12:00:60.wav",
    "2026-02-16-birdnet-12:00:00-extra.wav",
    "2026-02-16-12:00:00.wav",
    "birdnet-12:00:00.wav",
    "2026-02-16-birdnet-12-00-00.wav",
    "2026-02-16-birdnet-12:00.wav",
    "2026-02-16-birdnet-.wav",
    "20260216-birdnet-12:00:00.wav",
    "2026-02-16-bird-12:00:00.wav",
    "2026-02-16-BirdNET-12:00:00.wav",
    "12026-02-16-birdnet-12:00:00.wav",
    "2026-02-16-birdnet-12:00:00",
    "x-2026-02-16-birdnet-12:00:00.wav",
]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pi-root", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    spec = importlib.util.spec_from_file_location("ref_pi", os.path.join(HERE, "ref_pi.py"))
    rp = importlib.util.module_from_spec(spec)
    sys.modules["ref_pi"] = rp
    spec.loader.exec_module(rp)
    classes = rp.PiRef(a.pi_root).classes
    out = {"schema": "birdnet-parity/file-names/1", "names": {}}
    for name in NAMES:
        try:
            f = classes.ParseFileName(name)
            out["names"][name] = {
                "accepted": True,
                "date": f.file_date.strftime("%Y-%m-%d"),
                "time": f.file_date.strftime("%H:%M:%S"),
                "iso_week": f.week,
                "rtsp_id": f.RTSP_id,
            }
        except Exception as e:  # BirdNET-Pi's process_file() logs and skips the file
            out["names"][name] = {"accepted": False, "error": type(e).__name__}
    with open(a.out, "w") as fh:
        json.dump(out, fh, indent=1)
        fh.write("\n")
    for n, v in out["names"].items():
        print(f"{n:48} {v}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
