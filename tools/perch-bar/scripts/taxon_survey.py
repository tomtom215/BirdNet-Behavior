"""List every annotated species name per dataset and how it maps to Perch / BirdNET+ classes by EXACT name.
Writes results/taxon_exact_match.csv and prints the unmatched ones with same-genus / same-epithet candidates."""
import sys, os, csv, io, zipfile, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c

sci, isb = c.load_perch_labels()
perch = {s: i for i, s in enumerate(sci) if isb[i]}
bsci, _ = c.load_bnp_labels()
bnp = {s: i for i, s in enumerate(bsci)}
D = c.DATA

def species_csv(path):
    return {r["Species eBird Code"]: r["Scientific Name"] for r in csv.DictReader(open(path, encoding="utf-8"))}

sets = {}
for tag, d in [("SNE", "z7525805_sne"), ("SSW", "z7079380_ssw"), ("COR", "z7525349_cor"), ("PER", "z7079124_per"),
               ("HAW", "z7078499_hsn"), ("KEN", "z10943500_ken")]:
    m = species_csv(f"{D}/{d}_species_csv/species.csv")
    ann = f"{D}/{d}_annotations_csv/annotations.csv"
    cnt = collections.Counter(r["Species eBird Code"] for r in csv.DictReader(open(ann, encoding="utf-8")))
    sets[tag] = {m.get(k, k): v for k, v in cnt.items()}
    for k in cnt:
        if k not in m: print("code without species.csv entry:", tag, k, cnt[k])
# Powdermill (names from README table)
txt = open(f"{D}/z4656848_pow_readme/README.txt", encoding="utf-8", errors="replace").read().replace("\r", "")
pow_codes = {}
for line in txt.split("---Alpha Codes---")[1].splitlines():
    parts = [p for p in line.split("\t") if p.strip()]
    if len(parts) == 3: pow_codes[parts[1].strip()] = parts[2].strip()
z = zipfile.ZipFile(f"{D}/z4656848_pow_annotations_zip/annotation_Files.zip")
cnt = collections.Counter()
for n in z.namelist():
    if n.endswith(".txt"):
        for r in csv.DictReader(io.StringIO(z.read(n).decode("utf-8", "replace").replace("\r", "")), delimiter="\t"):
            cnt[r["Species"].strip()] += 1
sets["POW"] = {pow_codes.get(k, "?" + k): v for k, v in cnt.items()}
# CEB test
cnt = collections.Counter()
for r in csv.DictReader(open(f"{D}/z21821133_ceb_test_csv/test_soundscape.csv", encoding="utf-8")):
    cnt[r["scientific_name"]] += 1
sets["CEB"] = dict(cnt)

from taxon import Mapper
mp = Mapper(sci, isb, bsci)
with open(c.RES + "/taxon_whole_collections.csv", "w", newline="") as f:
    w = csv.writer(f); w.writerow(["dataset", "annotated_name", "n_events_in_whole_collection", "perch_class_index", "basis", "bnp_class_index"])
    for tag, d in sets.items():
        for name, n in sorted(d.items()):
            i, basis = mp.perch_idx(name)
            w.writerow([tag, name, n, "" if i is None else i, basis, "" if mp.bnp_idx(name) is None else mp.bnp_idx(name)])
with open(c.RES + "/taxon_exact_match.csv", "w", newline="") as f:
    w = csv.writer(f); w.writerow(["dataset", "annotated_name", "n_events_in_dataset", "perch_exact", "bnp_exact"])
    for tag, d in sets.items():
        un = []
        for name, n in sorted(d.items()):
            w.writerow([tag, name, n, perch.get(name, ""), bnp.get(name, "")])
            if name not in perch: un.append((name, n))
        print(f"== {tag}: {len(d)} annotated names, {len(d)-len(un)} exact Perch matches, {len(un)} unmatched")
        for name, n in un:
            g, _, e = name.partition(" ")
            cands = [s for s in perch if (s.split()[0] == g) or (e and s.split()[1] == e)]
            print(f"   {name!r} ({n} events)  candidates: {cands[:8]}")
