"""Load every real-soundscape window (labels + Perch logits + BirdNET+ probabilities) into memory."""
import sys, os, csv, glob
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
import events as ev
import metrics as m
from taxon import Mapper

TAG_ORDER = ["CEB", "POW", "SNE", "SSW", "COR", "PER", "KEN", "HAW"]
REGION = {"SNE": "z7525805_sne", "SSW": "z7079380_ssw", "COR": "z7525349_cor", "PER": "z7079124_per", "HAW": "z7078499_hsn", "KEN": "z10943500_ken"}


def region_names():
    """Latin names of every species annotated anywhere in each full collection (the 'oracle local list')."""
    out = {}
    for tag, pre in REGION.items():
        out[tag] = {r["Scientific Name"] for r in csv.DictReader(open(f"{c.DATA}/{pre}_species_csv/species.csv", encoding="utf-8"))}
    txt = open(f"{c.DATA}/z4656848_pow_readme/README.txt", encoding="utf-8", errors="replace").read().replace("\r", "")
    names = set()
    for line in txt.split("---Alpha Codes---")[1].splitlines():
        parts = [p for p in line.split("\t") if p.strip()]
        if len(parts) == 3: names.add(parts[2].strip())
    out["POW"] = names
    out["CEB"] = {r["scientific_name"] for r in csv.DictReader(open(f"{c.DATA}/z21821133_ceb_test_csv/test_soundscape.csv", encoding="utf-8"))}
    return out


def load_all(tags=None):
    sci, isb = c.load_perch_labels()
    bsci, bcls = c.load_bnp_labels()
    mp = Mapper(sci, isb, bsci)
    rn = region_names()
    W, Zs, Bs = [], [], []
    for tag in (tags or TAG_ORDER):
        f = f"{c.WORK}/real/{tag}_windows.csv"
        if not os.path.exists(f): continue
        rows = list(csv.DictReader(open(f)))
        for r in rows: r["cluster"] = f"{tag}/{r['file']}"
        W += rows
        Zs.append(np.load(f"{c.WORK}/real/{tag}_perch.npy")); Bs.append(np.load(f"{c.WORK}/real/{tag}_bnp.npy"))
    Z = np.concatenate(Zs); B = np.concatenate(Bs)
    assert len(W) == len(Z) == len(B)
    present = [m.parse_ids(r["present_perch"]) for r in W]
    edge = [m.parse_ids(r["edge_perch"]) for r in W]
    present_b = [m.parse_ids(r["present_bnp"]) for r in W]
    edge_b = [[j for j in (mp.bnp_idx(sci[i]) for i in e) if j is not None] for e in edge]
    region_masks = {}
    for tag, names in rn.items():
        mk = np.zeros(len(sci), bool)
        for nme in names:
            i, _ = mp.perch_idx(nme)
            if i is not None: mk[i] = True
        region_masks[tag] = mk
    return dict(W=W, Z=Z, B=B, present=present, edge=edge, present_b=present_b, edge_b=edge_b, sci=sci, isb=isb,
                bsci=bsci, bcls=bcls, mp=mp, region_masks=region_masks, region_names=rn)
