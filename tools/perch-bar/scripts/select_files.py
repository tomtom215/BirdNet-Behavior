"""Deterministic file selection for the annotated soundscape sets (seeded; see results/data_sources.csv).

Order = ascending sha256("bnb-multispecies-v1|"+dataset+"|"+member).  Take the first K members that satisfy the
density rule (>= MIN_EVENTS annotated events starting inside the span that will be downloaded).
"""
import csv, hashlib, sys, os, collections, json
MS = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
D = MS + "/data/"

def order(ds, names):
    return sorted(names, key=lambda n: hashlib.sha256(f"bnb-multispecies-v1|{ds}|{n}".encode()).hexdigest())

def load_zip_ann(path):
    ev = collections.defaultdict(list)
    for r in csv.DictReader(open(path, encoding="utf-8")):
        ev[r["Filename"]].append((float(r["Start Time (s)"]), float(r["End Time (s)"]), r["Species eBird Code"]))
    return ev

def listing(tag):
    rows = list(csv.DictReader(open(f"{MS}/meta/listings/{tag}.tsv"), delimiter="\t"))
    return {r["name"]: r for r in rows}

CFG = {  # tag, annotation csv, span seconds (None = whole file), k, min events, member size cap in bytes (None = full)
    "sne": (D + "z7525805_sne_annotations_csv/annotations.csv", None, 8, 0, None),
    "hsn": (D + "z7078499_hsn_annotations_csv/annotations.csv", None, 5, 0, None),
    "ssw": (D + "z7079380_ssw_annotations_csv/annotations.csv", 540, 3, 20, 15_000_000),
    "cor": (D + "z7525349_cor_annotations_csv/annotations.csv", 540, 3, 20, 15_000_000),
    "per": (D + "z7079124_per_annotations_csv/annotations.csv", 540, 3, 20, 15_000_000),
    "ken": (D + "z10943500_ken_annotations_csv/annotations.csv", 540, 3, 20, 15_000_000),
}
if __name__ == "__main__":
    out = {}
    for tag, (annp, span, k, mn, cap) in CFG.items():
        ev = load_zip_ann(annp); lst = listing(tag)
        chosen = []
        for n in order(tag, lst):
            e = ev.get(n, [])
            cnt = sum(1 for s, _, _ in e if span is None or s < span)
            if cnt >= mn: chosen.append((n, cnt, int(lst[n]["file_size"])))
            if len(chosen) == k: break
        out[tag] = {"span_s": span, "cap_bytes": cap, "files": chosen}
        print(tag, span, cap, [(n, c, round(s / 1e6, 1)) for n, c, s in chosen])
    json.dump(out, open(MS + "/meta/selection_zip.json", "w"), indent=1)


def select_powdermill(per_recording=2, min_events=20, recordings=("Recording_1", "Recording_2", "Recording_4")):
    """Powdermill: 77 five-minute WAV segments.  Stratified by the three long continuous recordings (27 Apr, 8 May,
    16 Jul 2018) so one date cannot supply every window: per_recording segments each, hash order within a stratum,
    density rule, whole segments.  (A first draft matched only the ".WAV" spelling of the member names, which silently
    restricted the choice to Recording_1; the stratified rule was adopted before any audio was analysed.)"""
    import zipfile, io
    z = zipfile.ZipFile(D + "z4656848_pow_annotations_zip/annotation_Files.zip")
    cnt = {}
    for n in z.namelist():
        if n.endswith(".txt"):
            t = z.read(n).decode("utf-8", "replace").replace("\r", "")
            rows = list(csv.DictReader(io.StringIO(t), delimiter="\t"))
            cnt[n.replace(".Table.1.selections.txt", ".WAV")] = len(rows)
    lst = listing("pow")
    # the zip spells the extension ".WAV" for Recording_1 and ".wav" for the others: match case-insensitively
    byl = {k.lower(): k for k in lst}
    cnt = {byl[k.lower()]: v for k, v in cnt.items() if k.lower() in byl}
    chosen = []
    for rec in recordings:
        got = 0
        for n in order("pow", [w for w in cnt if w in lst and w.startswith(rec + "/")]):
            if cnt[n] >= min_events:
                chosen.append((n, cnt[n], int(lst[n]["file_size"]))); got += 1
            if got == per_recording: break
    return chosen

if __name__ == "__main__":
    ch = select_powdermill()
    print("pow", ch)
    out = json.load(open(MS + "/meta/selection_zip.json"))
    out["pow"] = {"span_s": 300, "cap_bytes": None, "files": ch}
    json.dump(out, open(MS + "/meta/selection_zip.json", "w"), indent=1)
