"""Download one file into a NEW empty directory and append (url, bytes, sha256) to a manifest.

usage: fetch.py URL OUTDIR FILENAME MANIFEST [--max-bytes N]
OUTDIR must not exist or be empty (one download per directory, per the task rules).
Nothing downloaded is executed or imported.  Generic User-Agent.  --max-bytes keeps only a prefix
(the manifest records that the file is a prefix).
"""
import sys, os, hashlib, urllib.request, time, datetime

UA = "birdnet-behavior-validation/1.0"
url, outdir, fname, manifest = sys.argv[1:5]
maxb = int(sys.argv[sys.argv.index("--max-bytes") + 1]) if "--max-bytes" in sys.argv else None
if os.path.exists(outdir) and os.listdir(outdir):
    sys.exit(f"refusing: {outdir} is not empty")
os.makedirs(outdir, exist_ok=True)
path = os.path.join(outdir, fname)
h = hashlib.sha256(); n = 0
hdrs = {"User-Agent": UA}
if maxb: hdrs["Range"] = f"bytes=0-{maxb-1}"
for attempt in range(4):
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers=hdrs), timeout=120) as r, open(path, "wb") as f:
            status = r.status
            while True:
                b = r.read(1 << 20)
                if not b: break
                f.write(b); h.update(b); n += len(b)
        break
    except Exception as e:
        if attempt == 3: raise
        h = hashlib.sha256(); n = 0; time.sleep(3 * (attempt + 1))
with open(manifest, "a") as m:
    m.write(f"{datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")}\t{url}\t{path}\t{n}\t{h.hexdigest()}\t{'prefix' if maxb else 'full'}\n")
print(path, n, h.hexdigest())
