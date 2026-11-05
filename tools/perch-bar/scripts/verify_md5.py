"""For every file fetched in full from Zenodo (manifest.tsv, 'full'), compare its md5 with the checksum Zenodo's API lists
for that file.  Writes results/data_md5_check.csv.  Also writes results/data_manifest.csv (everything that was fetched)."""
import sys, os, csv, json, hashlib, re, urllib.request
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c

man = list(csv.DictReader(open(c.META + "/manifest.tsv"), delimiter="\t"))
recs = {}
def listing(rid):
    if rid not in recs:
        req = urllib.request.Request(f"https://zenodo.org/api/records/{rid}", headers={"User-Agent": "birdnet-behavior-validation/1.0"})
        d = json.load(urllib.request.urlopen(req, timeout=60))
        recs[rid] = {f["key"]: f["checksum"] for f in d["files"]}
    return recs[rid]
out = []
for r in man:
    mt = re.match(r"https://zenodo.org/api/records/(\d+)/files/([^/]+)/content", r["url"])
    row = dict(path=os.path.relpath(r["path"], c.MS), url=r["url"], bytes=r["bytes"], sha256=r["sha256"], kind=r["kind"], zenodo_md5="", local_md5="", md5_match="n/a (not a Zenodo file, or a byte-prefix)")
    if mt and r["kind"] == "full" and os.path.exists(r["path"]):
        rid, key = mt.group(1), urllib.request.unquote(mt.group(2))
        z = listing(rid).get(key, "")
        h = hashlib.md5(open(r["path"], "rb").read()).hexdigest()
        row.update(zenodo_md5=z.replace("md5:", ""), local_md5=h, md5_match="MATCH" if z == "md5:" + h else "MISMATCH")
    elif mt and r["kind"] == "prefix":
        row["md5_match"] = "n/a (byte-prefix of a larger file; whole-file md5 not checkable)"
    out.append(row)
with open(c.RES + "/data_md5_check.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(out[0])); w.writeheader(); w.writerows(out)
print("zenodo full files:", sum(1 for r in out if r["md5_match"] == "MATCH"), "match;", sum(1 for r in out if r["md5_match"] == "MISMATCH"), "mismatch")
