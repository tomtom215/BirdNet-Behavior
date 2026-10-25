#!/usr/bin/env python3
"""How the extended clip set was chosen (kept so the choice can be repeated and questioned).

    python3 -I search_extended.py OUT_DIR

Queries archive.org's `radio-aporee-maps` collection (geotagged, dated field recordings with a stated Creative Commons
licence) for each category keyword below, takes results in identifier order, spreads PER*3 picks evenly over them and keeps the
first PER with an MP3 of 60 s to 25 MB and a latitude/longitude, per category.  Nothing looks at what BirdNET hears in a
recording; the keyword is the only criterion.  Writes OUT_DIR/picked.json (feed it to `build_extended_manifest.py`).
Two rounds were run: the first ten categories (PER 5), then six bird-named ones (PER 5), deduplicated.
"""
import json, os, sys, time, urllib.parse, urllib.request

UA = {"User-Agent": "BirdNet-Behavior-parity-fixtures/0.1 (fixture search)"}
ROUNDS = {
    "round1": {
        "dawn_chorus": 'title:("dawn chorus" OR "morning birdsong" OR "birdsong" OR "morning birds")',
        "rain": "title:(rain OR thunderstorm OR drizzle)",
        "wind": "title:(wind OR windy OR storm)",
        "night": 'title:(night OR owl OR nightingale OR "nocturnal")',
        "frogs_insects": "title:(frogs OR frog OR crickets OR cicadas OR insects OR grasshoppers)",
        "speech_crowd": 'title:(conversation OR market OR crowd OR children OR "talking" OR playground OR cafe)',
        "urban_noise": "title:(traffic OR train OR airport OR construction OR motorway OR lawnmower)",
        "water_sea": "title:(waves OR sea OR beach OR river OR stream OR waterfall)",
        "gulls_waterbirds": "title:(gulls OR seagulls OR geese OR cranes OR ducks OR swans)",
        "quiet": 'title:(silence OR "room tone" OR quiet OR calm)',
    },
    "round2": {
        "named_songbirds": 'title:(blackbird OR nightingale OR robin OR thrush OR wren OR chaffinch OR sparrow OR warbler OR "great tit" OR bluetit)',
        "named_nonpasserine": "title:(cuckoo OR owl OR woodpecker OR pigeon OR dove OR crane OR heron OR curlew OR lapwing)",
        "named_corvids_gulls": "title:(crow OR raven OR magpie OR jay OR rooks OR jackdaw OR gull OR seagull)",
        "bird_choruses": 'title:("bird chorus" OR "birds singing" OR "singing birds" OR "bird song" OR "spring birds" OR "morning chorus")',
        "tropical_birds": "title:(jungle OR rainforest OR tropical OR amazon OR savanna OR bush)",
        "farm_wetland": 'title:(rooster OR "farm" OR marsh OR wetland OR pond OR lake OR reeds)',
    },
}
MP3 = ("VBR MP3", "128Kbps MP3", "64Kbps MP3", "MP3", "160Kbps MP3", "192Kbps MP3", "256Kbps MP3", "320Kbps MP3")


def get(url):
    err = None
    for _ in range(3):
        try:
            return json.load(urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=60))
        except Exception as e:  # noqa: BLE001
            err = e
            time.sleep(2)
    raise err


def seconds(s):
    s = str(s or "0")
    return sum(float(x) * 60**i for i, x in enumerate(reversed(s.split(":")))) if ":" in s else float(s)


def main():
    out_dir = sys.argv[1]
    os.makedirs(out_dir, exist_ok=True)
    per = 5
    picked, seen = [], set()
    for rnd, cats in ROUNDS.items():
        for cat, t in cats.items():
            q = f"collection:radio-aporee-maps AND mediatype:audio AND {t} AND licenseurl:*"
            url = "https://archive.org/advancedsearch.php?" + urllib.parse.urlencode(
                [("q", q), ("fl[]", "identifier"), ("rows", 60), ("sort[]", "identifier asc"), ("output", "json")])
            docs = get(url)["response"]["docs"]
            n, got = len(docs), 0
            for j in [int(i * n / (per * 3)) for i in range(per * 3)]:
                ident = docs[j]["identifier"]
                if ident in seen:
                    continue
                md = get("https://archive.org/metadata/" + ident)
                meta, files = md["metadata"], md["files"]
                mp3 = [f for f in files if f.get("format") in MP3]
                if not mp3:
                    continue
                f = sorted(mp3, key=lambda f: int(f.get("size", 0)))[-1]
                if seconds(f.get("length")) < 60 or int(f.get("size", 0)) > 25_000_000:
                    continue
                try:
                    lat, lon = float(meta["latitude"]), float(meta["longitude"])
                except (KeyError, ValueError):
                    continue
                seen.add(ident)
                picked.append({"category": cat, "identifier": ident, "title": meta.get("title"), "licenseurl": meta.get("licenseurl"),
                               "creator": meta.get("creator"), "date": meta.get("date"), "timezone": meta.get("timezone"),
                               "lat": lat, "lon": lon, "file": f["name"], "size": int(f["size"]), "length": seconds(f.get("length")),
                               "sha1": f.get("sha1"), "md5": f.get("md5")})
                got += 1
                if got >= per:
                    break
    json.dump(picked, open(os.path.join(out_dir, "picked.json"), "w"), indent=1)
    print(len(picked), "recordings")


if __name__ == "__main__":
    main()
