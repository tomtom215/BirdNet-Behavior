"""Fetch a fixed, seeded subset of ESC-50 non-bird clips (5 s each) from the public GitHub repo.

 * esc50.csv  -> data/esc50_meta/
 * clips      -> data/esc50_noise_clips/   (one new directory for this batch of small downloads)
Selection: categories below, at most one clip per Freesound source file (src_file), ordered by
sha256("bnb-multispecies-v1|esc50|"+filename), first N per category.  0.3 s pause between requests.
"""
import csv, hashlib, os, sys, time, urllib.request, datetime
UA = "birdnet-behavior-validation/1.0"
MS = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASE = "https://raw.githubusercontent.com/karolpiczak/ESC-50/master/"
CATS = ["rain", "wind", "thunderstorm", "sea_waves", "engine", "airplane", "helicopter", "train"]
N = 20
manifest = MS + "/meta/manifest.tsv"

def get(url):
    return urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": UA}), timeout=60).read()

def log(url, path, data, kind="full"):
    with open(manifest, "a") as m:
        m.write(f"{datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')}\t{url}\t{path}\t{len(data)}\t{hashlib.sha256(data).hexdigest()}\t{kind}\n")

mdir = MS + "/data/esc50_meta"
if os.path.exists(mdir) and os.listdir(mdir): sys.exit("meta dir not empty")
os.makedirs(mdir, exist_ok=True)
meta = get(BASE + "meta/esc50.csv")
open(mdir + "/esc50.csv", "wb").write(meta); log(BASE + "meta/esc50.csv", mdir + "/esc50.csv", meta)
rows = list(csv.DictReader(open(mdir + "/esc50.csv", encoding="utf-8")))
cdir = MS + "/data/esc50_noise_clips"
if os.path.exists(cdir) and os.listdir(cdir): sys.exit("clip dir not empty")
os.makedirs(cdir, exist_ok=True)
chosen = []
for cat in CATS:
    seen = set(); got = 0
    cand = sorted([r for r in rows if r["category"] == cat],
                  key=lambda r: hashlib.sha256(f"bnb-multispecies-v1|esc50|{r['filename']}".encode()).hexdigest())
    for r in cand:
        if r["src_file"] in seen: continue
        seen.add(r["src_file"]); chosen.append(r); got += 1
        if got == N: break
for r in chosen:
    fn = r["filename"]
    assert fn.replace("-", "").replace(".wav", "").isalnum()
    data = get(BASE + "audio/" + fn)
    open(f"{cdir}/{fn}", "wb").write(data)
    log(BASE + "audio/" + fn, f"{cdir}/{fn}", data)
    time.sleep(0.3)
print(len(chosen), "clips")
