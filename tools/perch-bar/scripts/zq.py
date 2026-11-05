"""Zenodo record inspector (metadata only). usage: zq.py RECORD_ID..."""
import sys, json, re, urllib.request
for rid in sys.argv[1:]:
    req = urllib.request.Request(f"https://zenodo.org/api/records/{rid}",
                                 headers={"User-Agent": "birdnet-behavior-validation/1.0"})
    try:
        d = json.load(urllib.request.urlopen(req, timeout=40))
    except Exception as e:
        print(rid, "ERROR", e); continue
    m = d.get("metadata", {})
    print("ID", rid, "| TITLE:", m.get("title"))
    print("DOI:", d.get("doi"), "| version:", m.get("version"), "| pub:", m.get("publication_date"))
    print("LICENSE:", m.get("license"), "| access:", m.get("access_right"))
    desc = re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", m.get("description", "")))
    print("DESC:", desc[:900])
    tot = 0
    for f in d.get("files", []):
        tot += f.get("size", 0)
        print("  FILE", f.get("key"), f.get("size"), f.get("checksum"))
    print("TOTAL GB", round(tot / 1e9, 3), "\n-----")
