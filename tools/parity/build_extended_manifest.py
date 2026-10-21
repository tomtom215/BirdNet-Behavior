#!/usr/bin/env python3
"""Write `fixtures_extended.json`: the 14 clips' companion set of ~70 further real recordings.

    python -I build_extended_manifest.py PICKED.json[,PICKED2.json] ESC50_CSV OUT.json [--append]

`--append` keeps an existing OUT.json (and the sha256 pins in it) and adds only the clips and sources it does not have.

Nothing here is a judgement about which clips are "good": PICKED.json is the output of
`search_extended.py`, which takes archive.org `radio-aporee-maps` items (geotagged, dated field
recordings with a Creative Commons licence stated by the uploader) whose titles match a category
keyword, spread evenly over the identifier-sorted result list, the first five with an MP3 of at
least 60 s and at most 25 MB.  Categories: dawn chorus, rain, wind, night, frogs/insects, speech and
crowds, urban noise, water, waterbirds, quiet.  Each becomes a 30 s excerpt, 48 kHz (every fourth
keeps its native rate so that resampling is exercised), channels as recorded.  To them are added:
an ESC-50 clip for each of 20 sound classes (birds, other animals, weather, machines, human
sounds), LibriVox speech at four offsets (CC0), and generated clips (pink noise, a 1 kHz tone, a
clipped square wave, digital silence).
"""
import csv
import json
import sys

import os

args = [a for a in sys.argv[1:] if a != "--append"]
append = "--append" in sys.argv
picked_path, esc_csv, out = args[:3]
picked, seen_ids = [], set()
for path in picked_path.split(","):
    for p in json.load(open(path)):
        if p["identifier"] not in seen_ids:     # one recording appears under two keywords now and then
            seen_ids.add(p["identifier"])
            picked.append(p)

sources, clips = {}, []


def iso_week(date: str) -> int:
    import datetime as dt
    return dt.datetime.strptime(date, "%Y-%m-%d %H:%M:%S").isocalendar()[1]


for i, p in enumerate(picked):
    sid = p["identifier"]
    sources[sid] = {
        "url": f"https://archive.org/download/{sid}/{p['file']}",
        "page": f"https://archive.org/details/{sid}",
        "licence": f"{p['licenseurl']} (as asserted by the uploader on archive.org; not verified)",
        "creator": p["creator"],
        "title": p["title"],
        "ia_sha1": p["sha1"],
        "sha256": None,
    }
    native = i % 4 == 0
    start = int(min(0.3 * p["length"], p["length"] - 35))
    ff = {"ss": float(start), "t": 30.0, "codec": "pcm_s16le"}
    if not native:
        ff["ar"] = 48000
    clips.append({
        "id": f"{p['category']}_{sid.replace('aporee_', '')}",
        "source": sid,
        "recipe": {"ffmpeg": ff},
        "tags": ["real", "aporee", p["category"]] + (["native-rate"] if native else []),
        "note": p["title"],
        "sha256": None,
        "station": {
            "lat": round(p["lat"], 4),
            "lon": round(p["lon"], 4),
            "date": p["date"],
            "timezone": p["timezone"],
            "week": iso_week(p["date"]),
            "basis": "recording metadata (archive.org latitude/longitude/date of the upload; the excerpt is "
                     "taken from %d s into it, so its own clock time is later than 'date' by that much)" % start,
        },
    })

# ESC-50: one clip per class, the first by file name in fold 1.
ESC = ["chirping_birds", "crow", "rooster", "hen", "insects", "crickets", "frog", "dog", "cat", "rain", "wind",
       "thunderstorm", "sea_waves", "engine", "vacuum_cleaner", "laughing", "coughing", "crying_baby",
       "church_bells", "siren"]
rows = list(csv.DictReader(open(esc_csv)))
for cls in ESC:
    row = sorted((r for r in rows if r["category"] == cls and r["fold"] == "1"), key=lambda r: r["filename"])[0]
    fn = row["filename"]
    sid = "esc50_" + fn[:-4].replace("-", "_")
    sources[sid] = {
        "url": f"https://raw.githubusercontent.com/karolpiczak/ESC-50/master/audio/{fn}",
        "page": "https://github.com/karolpiczak/ESC-50",
        "licence": "ESC-50 is CC BY-NC 4.0; each clip carries its own Freesound licence (CC0 / CC BY / CC BY-NC), "
                   "not verified per clip. Fetch-only, never committed.",
        "sha256": None,
    }
    clips.append({
        "id": f"esc50_{cls}",
        "source": sid,
        "recipe": {"copy": True},
        "tags": ["real", "esc50", cls, "5s", "44k1", "may-contain-digital-silence-padding"],
        "note": f"ESC-50 {fn} ({cls}), 5 s, 44.1 kHz mono; ESC-50 pads short clips with digital silence",
        "sha256": None,
        "station": {"lat": 51.48, "lon": -0.13, "date": "2026-06-22 12:00:00", "week": 26, "basis": "ASSUMED (London)"},
    })

# LibriVox speech at four offsets (same CC0 source as `speech_librivox_44k1_12s`).
sources["librivox"] = {
    "url": "https://archive.org/download/newadventuresofalice_1302_librivox/newadventuresofalice_00_rae.mp3",
    "page": "https://archive.org/details/newadventuresofalice_1302_librivox",
    "licence": "CC0 1.0 (archive.org licenseurl https://creativecommons.org/publicdomain/zero/1.0/); LibriVox recording of a public-domain text",
    "creator": "John Rae (LibriVox reader)",
    "sha256": "3e29214a926ca359fbf494b0295842d33a8f19e07d954b76aa993ad4e75eb5ed",
}
for off in (36, 66, 96, 113):
    clips.append({
        "id": f"speech_librivox_44k1_30s_at{off}",
        "source": "librivox",
        "recipe": {"ffmpeg": {"ss": float(off), "t": 30.0, "codec": "pcm_s16le"}},
        "tags": ["real", "speech", "44k1", "librivox"],
        "note": f"LibriVox reader, 30 s from {off} s in",
        "sha256": None,
        "station": {"lat": 51.48, "lon": -0.13, "date": "2026-06-22 12:00:00", "week": 26, "basis": "ASSUMED (London)"},
    })

sources["synthetic"] = {"url": None, "page": None, "licence": "CC0 1.0: generated by tools/parity/fetch_fixtures.py (numpy RandomState seed 20240224)", "sha256": None}
for name in ("pink_noise_30s", "tone_1k_30s", "clipped_square_30s", "digital_silence_30s"):
    clips.append({
        "id": f"synthetic_{name}",
        "source": "synthetic",
        "recipe": {"synthetic": name},
        "tags": ["synthetic", name],
        "note": name,
        "sha256": None,
        "station": {"lat": 51.48, "lon": -0.13, "date": "2026-06-22 12:00:00", "week": 26, "basis": "ASSUMED (London)"},
    })

if append and os.path.exists(out):
    old = json.load(open(out))
    have = {c["id"] for c in old["clips"]}
    old["sources"].update({k: v for k, v in sources.items() if k not in old["sources"]})
    old["clips"] += [c for c in clips if c["id"] not in have]
    with open(out, "w", encoding="utf-8") as f:
        json.dump(old, f, indent=2, ensure_ascii=False)
        f.write("\n")
    print(len(old["clips"]), "clips,", len(old["sources"]), "sources (appended)")
    sys.exit(0)

doc = {
    "_comment": "Extended parity fixtures (see build_extended_manifest.py). Same schema as fixtures.json; 'sha256' fields are filled by `fetch_fixtures.py --manifest fixtures_extended.json --pin`. The audio is never committed.",
    "ffmpeg": "ffmpeg version 6.1.1-3ubuntu5 (decodes MP3 and the two kinds of resample)",
    "sources": sources,
    "clips": clips,
}
with open(out, "w", encoding="utf-8") as f:
    json.dump(doc, f, indent=2, ensure_ascii=False)
    f.write("\n")
print(len(clips), "clips,", len(sources), "sources")
