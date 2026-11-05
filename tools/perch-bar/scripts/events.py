"""Unified annotation events for the eight annotated soundscape sets, and the window-label rule.

An event is (start_s, end_s, species_name).  species_name is the Latin binomial from the dataset's own species table,
or 'unknown' for the datasets' '????' (unidentifiable call) boxes.
"""
import csv, io, os, glob, zipfile, collections
import common as c

D = c.DATA
ZIP_SETS = {  # tag -> (annotation dir, species dir, audio dir)
    "SNE": ("z7525805_sne", "z7525805_sne_audio"),
    "HAW": ("z7078499_hsn", "z7078499_haw_audio"),
    "SSW": ("z7079380_ssw", "z7079380_ssw_audio"),
    "COR": ("z7525349_cor", "z7525349_cor_audio"),
    "PER": ("z7079124_per", "z7079124_per_audio"),
    "KEN": ("z10943500_ken", "z10943500_ken_audio"),
}


def kahl_events(tag):
    pre, _ = ZIP_SETS[tag]
    names = {r["Species eBird Code"]: r["Scientific Name"] for r in csv.DictReader(open(f"{D}/{pre}_species_csv/species.csv", encoding="utf-8"))}
    ev = collections.defaultdict(list)
    for r in csv.DictReader(open(f"{D}/{pre}_annotations_csv/annotations.csv", encoding="utf-8")):
        ev[r["Filename"]].append((float(r["Start Time (s)"]), float(r["End Time (s)"]), names[r["Species eBird Code"]]))
    return ev


def powdermill_events():
    txt = open(f"{D}/z4656848_pow_readme/README.txt", encoding="utf-8", errors="replace").read().replace("\r", "")
    codes = {}
    for line in txt.split("---Alpha Codes---")[1].splitlines():
        parts = [p for p in line.split("\t") if p.strip()]
        if len(parts) == 3: codes[parts[1].strip()] = parts[2].strip()
    z = zipfile.ZipFile(f"{D}/z4656848_pow_annotations_zip/annotation_Files.zip")
    ev = {}
    for n in z.namelist():
        if not n.endswith(".txt"): continue
        key = os.path.basename(n).replace(".Table.1.selections.txt", "").lower()
        rows = csv.DictReader(io.StringIO(z.read(n).decode("utf-8", "replace").replace("\r", "")), delimiter="\t")
        ev[key] = [(float(r["Begin Time (s)"]), float(r["End Time (s)"]), codes[r["Species"].strip()]) for r in rows]
    return ev


def ceb_events():
    ev = collections.defaultdict(list)
    for r in csv.DictReader(open(f"{D}/z21821133_ceb_test_csv/test_soundscape.csv", encoding="utf-8")):
        ev[r["filepath"]].append((float(r["start_time"]), float(r["end_time"]), r["scientific_name"]))
    return ev


def real_files():
    """-> list of (dataset_tag, audio_path, events list)"""
    out = []
    for tag, (pre, adir) in ZIP_SETS.items():
        ev = kahl_events(tag)
        for p in sorted(glob.glob(f"{D}/{adir}/*.flac")):
            out.append((tag, p, ev.get(os.path.basename(p), [])))
    pev = powdermill_events()
    for p in sorted(glob.glob(f"{D}/z4656848_pow_audio/*")):
        key = os.path.splitext(os.path.basename(p))[0].lower()
        out.append(("POW", p, pev[key]))
    cev = ceb_events()
    for p in sorted(glob.glob(f"{D}/ceb_test_flac/*.flac")):
        out.append(("CEB", p, cev.get(os.path.basename(p).replace("__", "/", 1), [])))
    return out


def overlap(a0, a1, b0, b1):
    return max(0.0, min(a1, b1) - max(a0, b0))


def label_window(events, t0, t1, pad_min=0.3, edge_margin=0.5, core_frac=0.5, core_cap=1.0):
    """Return (present, edge) as sets of species names, plus has_unknown.

    d  = max(event length, pad_min) (point events are padded around their centre)
    present: overlap with the window >= min(core_frac*d, core_cap)
    edge   : not present, but the event widened by edge_margin on both sides overlaps the window
    """
    present, edge = set(), set()
    for a, b, sp in events:
        d = max(b - a, pad_min); ctr = (a + b) / 2; a2, b2 = ctr - d / 2, ctr + d / 2
        if overlap(a2, b2, t0, t1) >= min(core_frac * d, core_cap):
            present.add(sp)
        elif overlap(a2 - edge_margin, b2 + edge_margin, t0, t1) > 0:
            edge.add(sp)
    edge -= present
    unk_present = "unknown" in present
    has_unknown = unk_present or ("unknown" in edge)
    present.discard("unknown"); edge.discard("unknown")
    return present, edge, has_unknown
