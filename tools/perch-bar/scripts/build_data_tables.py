"""results/audio_files_used.csv : every audio file used (source, member, kept bytes, prefix or full, sha256 of the bytes kept,
decoded duration, windows analysed) and a transfer-volume summary (results/transfer_summary.txt)."""
import sys, os, csv, re, collections, glob
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c

rows = []
win = collections.Counter(); 
for f in glob.glob(c.WORK + "/real/*_windows.csv"):
    for r in csv.DictReader(open(f)): win[(r["dataset"], r["file"])] += 1
zmap = {"7525805": "SNE", "7078499": "HAW", "7079380": "SSW", "7525349": "COR", "7079124": "PER", "10943500": "KEN", "4656848": "POW"}
for r in csv.DictReader(open(c.META + "/zip_members.tsv"), delimiter="\t"):
    rid = re.search(r"records/(\d+)/", r["zip_url"]).group(1)
    base = os.path.basename(r["member"]); tag = zmap[rid]
    rows.append(dict(source=f"Zenodo {rid}", dataset=tag, file=base, role="real annotated soundscape", total_member_bytes=r["uncompressed_member_size"], bytes_kept=r["bytes_kept"],
                     kind=r["kind"], crc32_in_zip=r["crc32_member"], sha256_of_bytes_kept=r["sha256_kept"], windows=win.get((tag, base), 0)))
for f, role, tag, rid in ((c.META + "/ceb_test_members.tsv", "real annotated soundscape", "CEB", "21821133"), (c.META + "/ceb_trainxc_members.tsv", "clean single-species source (Xeno-canto focal)", "CEB-XC", "21821133")):
    used = set(os.listdir(c.DATA + ("/ceb_test_flac" if tag == "CEB" else "/ceb_trainxc_flac")))
    for r in csv.DictReader(open(f), delimiter="\t"):
        if r["status"] != "complete": continue
        base = r["member"].replace("/", "__")
        rows.append(dict(source="Zenodo 21821133 (tar.gz prefix)", dataset=tag, file=r["member"], role=role, total_member_bytes=r["size"], bytes_kept=r["size"], kind="full (complete tar member)",
                         crc32_in_zip="", sha256_of_bytes_kept=r["sha256"], windows=win.get(("CEB", base), "") if tag == "CEB" else ""))
for r in csv.DictReader(open(c.META + "/manifest.tsv"), delimiter="\t"):
    if "/esc50_noise_clips/" in r["path"]:
        rows.append(dict(source="GitHub karolpiczak/ESC-50", dataset="ESC-50", file=os.path.basename(r["path"]), role="real non-bird noise (5 s clip)", total_member_bytes=r["bytes"], bytes_kept=r["bytes"],
                         kind="full", crc32_in_zip="", sha256_of_bytes_kept=r["sha256"], windows=1))
with open(c.RES + "/audio_files_used.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0])); w.writeheader(); w.writerows(rows)
# transfer volume: manifest (direct fetches) + RangeFile "bytes fetched" lines in the zip logs
man = list(csv.DictReader(open(c.META + "/manifest.tsv"), delimiter="\t"))
direct = sum(int(r["bytes"]) for r in man)
rng = 0
for lf in (c.META + "/dl_zip_cmds.log", c.META + "/dl_pow.log"):
    for line in open(lf):
        m_ = re.match(r"requests (\d+) bytes fetched (\d+)", line)
        if m_: rng += int(m_.group(2))
lists = 0
for lf in glob.glob(c.META + "/listings/*.tsv"): pass
with open(c.RES + "/transfer_summary.txt", "w") as f:
    f.write(f"direct downloads (manifest.tsv, {len(man)} fetches): {direct/1e6:.1f} MB\n")
    f.write(f"zip range reads (Range-requested bytes incl. 4 MB read-ahead): {rng/1e6:.1f} MB\n")
    f.write(f"total transferred, excluding the small zip central-directory listings: {(direct+rng)/1e6:.1f} MB\n")
print(open(c.RES + "/transfer_summary.txt").read())
print(len(rows), "audio rows")
