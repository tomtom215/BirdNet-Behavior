"""Zenodo record metadata (title, DOI, licence, version/date, file list with sizes and Zenodo's md5) for every record considered -> meta/records.json
and results/zenodo_records.csv."""
import sys, os, csv, json, urllib.request
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
IDS = ["7079380", "7525349", "7079124", "7525805", "7078499", "10943500", "4656848", "21821133", "7828148", "20038954", "19340660", "13994373"]
out, rows = {}, []
for rid in IDS:
    req = urllib.request.Request(f"https://zenodo.org/api/records/{rid}", headers={"User-Agent": "birdnet-behavior-validation/1.0"})
    d = json.load(urllib.request.urlopen(req, timeout=60))
    m = d["metadata"]
    out[rid] = dict(title=m["title"], doi=d.get("doi"), license=(m.get("license") or {}).get("id"), access=m.get("access_right"), published=m.get("publication_date"),
                    version=m.get("version"), files=[dict(key=f["key"], size=f["size"], checksum=f["checksum"]) for f in d["files"]])
    rows.append(dict(record=rid, doi=d.get("doi"), license=(m.get("license") or {}).get("id"), published=m.get("publication_date"), version=m.get("version") or "",
                     total_gb=round(sum(f["size"] for f in d["files"]) / 1e9, 3), title=m["title"]))
json.dump(out, open(c.META + "/records.json", "w"), indent=1)
with open(c.RES + "/zenodo_records.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0])); w.writeheader(); w.writerows(rows)
for r in rows: print(r["record"], r["license"], r["published"], r["total_gb"], "GB", r["doi"], "|", r["title"][:70])
