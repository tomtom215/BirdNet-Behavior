"""results/data_sources.csv (one row per source considered: URL, licence, size, checksum as listed by the provider, what was used / why not)
and results/annotation_completeness_quotes.csv (what each provider says about what is annotated).  Values are copied from
meta/records.json (Zenodo API), results/audio_files_used.csv and results/real_inventory.csv; the text cells are written here."""
import sys, os, csv, json, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c

rec = json.load(open(c.META + "/records.json"))
aud = list(csv.DictReader(open(c.RES + "/audio_files_used.csv")))
inv = {r["dataset"]: r for r in csv.DictReader(open(c.RES + "/real_inventory.csv"))}
rows = []
def zrow(tag, rid, key, role, site, rule, extra=""):
    f = next(x for x in rec[rid]["files"] if x["key"] == key)
    used = [a for a in aud if a["dataset"] == tag]
    kinds = collections.Counter(a["kind"] for a in used)
    rows.append(dict(set=tag, role=role, url=f"https://zenodo.org/records/{rid}", doi=rec[rid]["doi"], licence=rec[rid]["license"], licence_basis="Zenodo API metadata.license.id",
                     archive=key, archive_bytes=f["size"], archive_checksum_listed_by_zenodo=f["checksum"],
                     used="yes", files_used=len(used), kinds="; ".join(f"{v} x {k}" for k, v in kinds.items()), bytes_kept=sum(int(a["bytes_kept"]) for a in used),
                     windows=inv.get(tag, {}).get("windows", ""), hours=inv.get(tag, {}).get("hours", ""), site=site, selection_rule=rule, reason_not_used="", notes=extra))
ZR = "files ordered by sha256('bnb-multispecies-v1|<set>|<name>'); "
zrow("SNE", "7525805", "soundscape_data.zip", "real annotated soundscape", "Sierra Nevada, California, USA; 2015; 10-min morning segments", ZR + "first 8, whole files")
zrow("HAW", "7078499", "soundscape_data.zip", "real annotated soundscape", "Island of Hawai'i; 2016-2021", ZR + "first 5, whole files")
zrow("SSW", "7079380", "soundscape_data.zip", "real annotated soundscape", "Sapsucker Woods, Ithaca NY, USA; 2017; 1-h files", ZR + "first 3 with >=20 events in first 540 s; first 15,000,000 bytes of each")
zrow("COR", "7525349", "soundscape_data.zip", "real annotated soundscape", "Colombia and Costa Rica coffee farms; 2019; 1-h files", ZR + "first 3 with >=20 events in first 540 s; first 15,000,000 bytes of each")
zrow("PER", "7079124", "soundscape_data.zip", "real annotated soundscape", "Madre de Dios, Peru; 2019; dawn hour; 1-h files", ZR + "first 3 with >=20 events in first 540 s; first 15,000,000 bytes of each")
zrow("KEN", "10943500", "soundscape_data.zip", "real annotated soundscape (POINT labels)", "Lake Baringo area, Kenya; 2021-22", ZR + "first 3 with >=20 events in first 540 s; first 15,000,000 bytes of each", "libsndfile cannot open these FLACs (error 161); decoded with ffmpeg")
zrow("POW", "4656848", "wav_Files.zip", "real annotated soundscape", "Powdermill Nature Reserve, Pennsylvania, USA; 2018; 5-min segments", "stratified: 2 segments from each of Recording_1, _2, _4 (hash order, >=20 events), whole segments", "README says 32 kHz; the WAVs are 44.1 kHz")
tf = next(x for x in rec["21821133"]["files"] if x["key"] == "test_soundscape.tar.gz")
u = [a for a in aud if a["dataset"] == "CEB"]
rows.append(dict(set="CEB", role="real annotated soundscape (strong boxes)", url="https://zenodo.org/records/21821133", doi=rec["21821133"]["doi"], licence="cc-by-4.0 (record); 'CC-BY 4.0' in the license column of all 15,064 test rows",
                 licence_basis="Zenodo API + test_soundscape.csv", archive="test_soundscape.tar.gz", archive_bytes=tf["size"], archive_checksum_listed_by_zenodo=tf["checksum"], used="yes",
                 files_used=len(u), kinds="complete tar members from a 300,000,000-byte prefix", bytes_kept=sum(int(a["bytes_kept"]) for a in u), windows=inv["CEB"]["windows"], hours=inv["CEB"]["hours"],
                 site="Central Europe (record title); coordinates anonymised; 28 kHz FLAC 3-min clips", selection_rule="first complete members of the archive in its own order (not chosen by content)", reason_not_used="", notes=""))
tx = next(x for x in rec["21821133"]["files"] if x["key"] == "train_xenocanto.tar.gz")
cat = list(csv.DictReader(open(c.WORK + "/syn/catalogue.csv")))
lic = collections.Counter(r["license"] for r in cat)
rows.append(dict(set="CEB-XC", role="clean single-species sources for the mixtures (Xeno-canto focal recordings with boxes)", url="https://zenodo.org/records/21821133", doi=rec["21821133"]["doi"],
                 licence=f"per-recording licence column; for the recordings used: {dict(lic)}", licence_basis="train_xenocanto.csv 'license' column", archive="train_xenocanto.tar.gz", archive_bytes=tx["size"],
                 archive_checksum_listed_by_zenodo=tx["checksum"], used="yes", files_used=len({r['file'] for r in cat}), kinds="complete tar members from a 160,000,000-byte prefix; windows cut from single-species recordings",
                 bytes_kept=sum(int(a["bytes_kept"]) for a in aud if a["dataset"] == "CEB-XC"), windows=f"{len(cat)} source segments", hours="", site="Europe", selection_rule="recordings with exactly one annotated species; >=1.0 s annotated in the 5 s window; <=4 per recording",
                 reason_not_used="", notes="NC-SA licences: used for an internal measurement only, not redistributed; recordist names not reproduced"))
nz = [a for a in aud if a["dataset"] == "ESC-50"]
rows.append(dict(set="ESC-50", role="real non-bird noise clips (5 s)", url="https://github.com/karolpiczak/ESC-50", doi="", licence="CC BY-NC (README badge); per-clip Freesound licences UNKNOWN (not checked)", licence_basis="README badge text 'license-CC BY-NC'",
                 archive="individual files audio/*.wav + meta/esc50.csv", archive_bytes="", archive_checksum_listed_by_zenodo="", used="yes", files_used=len(nz), kinds="full files", bytes_kept=sum(int(a["bytes_kept"]) for a in nz), windows=len(nz), hours="",
                 site="Freesound extracts", selection_rule="8 categories (rain, wind, thunderstorm, sea_waves, engine, airplane, helicopter, train); <=1 clip per source file; first 20 per category in sha256 order", reason_not_used="",
                 notes="Perch was trained on FSD50K, also from Freesound: overlap with these clips UNKNOWN"))
def no(source, url, lic, size, why):
    rows.append(dict(set=source, role="considered", url=url, doi="", licence=lic, licence_basis="Zenodo API / HF card", archive="", archive_bytes=size, archive_checksum_listed_by_zenodo="", used="no", files_used=0, kinds="", bytes_kept=0, windows="", hours="", site="", selection_rule="", reason_not_used=why, notes=""))
no("Zenodo 20038954 stratified forest soundscapes", "https://zenodo.org/records/20038954", rec["20038954"]["license"], sum(f["size"] for f in rec["20038954"]["files"]), "27 GB archives; enough sets already; only the record page was read")
no("Zenodo 7828148 XC 22 species", "https://zenodo.org/records/7828148", rec["7828148"]["license"], sum(f["size"] for f in rec["7828148"]["files"]), "metadata (1.3 MB) and annotation zip (0.8 MB) were fetched, then deleted; no licence/background-species column to judge 'clean'; audio not fetched")
no("Zenodo 19340660 BioDCASE BirdSet", "https://zenodo.org/records/19340660", rec["19340660"]["license"], sum(f["size"] for f in rec["19340660"]["files"]), "Perch embeddings only, no audio; not downloaded")
no("Zenodo 13994373 few-shot soundscapes", "https://zenodo.org/records/13994373", rec["13994373"]["license"], sum(f["size"] for f in rec["13994373"]["files"]), "NC licence and 45 GB; not downloaded")
no("Hugging Face DBD-research-group/BirdSet", "https://huggingface.co/datasets/DBD-research-group/BirdSet", "cc-by-nc-4.0", "", "data = tar.gz shards of 0.2-2 GB on the 'data' branch; the test sets are the Zenodo/Dryad recordings used above; only the loader script BirdSet.py (26 KB text) was fetched and read, never run")
no("Figshare NIPS4BPlus / BirdVox sets", "https://figshare.com/articles/dataset/6798548", "", "", "NIPS4BPlus is a transcription layer on other recordings (not fetched); BirdVox = nocturnal flight calls; not downloaded")
no("Xeno-canto", "https://xeno-canto.org", "", "", "API needs a key (HTTP 401 on one probe request, made before robots.txt was read, plus two HTML page probes returning HTTP 200 with content unused); robots.txt disallows anthropic-ai, Claude-Web, ClaudeBot and /api/, /explore; not used")
no("Wikimedia Commons", "https://commons.wikimedia.org/w/api.php", "", "", "first requests answered HTTP 429 (rate limit); not retried; not needed")
no("BirdNET-Analyzer example soundscape", "", "", "", "not looked for; per-species time labels not expected")
with open(c.RES + "/data_sources.csv", "w", newline="") as f:
    keys = []
    for r in rows:
        for k in r:
            if k not in keys: keys.append(k)
    w = csv.DictWriter(f, fieldnames=keys); w.writeheader(); w.writerows(rows)
Q = [("SSW", "Annotators were asked to box every bird call they could recognize, ignoring those that are too faint.", "Zenodo 7079380 description.pdf"),
     ("SNE", "Annotators were asked to box every bird call they could recognize, ignoring those that are too faint or unidentifiable.", "Zenodo 7525805 description.pdf"),
     ("COR", "annotators were asked to create a selection box around every bird call they could recognize, ignoring those that were too faint or unidentifiable.", "Zenodo 7525349 description.pdf"),
     ("PER", "A single annotator boxed every bird call he could identify and ignored those that were too faint.", "Zenodo 7079124 description.pdf"),
     ("HAW", "annotators were asked to create a selection box around every bird call they could recognize, ignoring those that were too faint or unidentifiable at a spectrogram window size of 700 points. [and] Recordings that did not contain vocalizations of the species of interest were excluded from full annotation and thus this collection.", "Zenodo 7078499 description.pdf"),
     ("KEN", "Annotators were tasked with identifying and labeling each bird call they could discern, excluding any calls that were too weak or indiscernible. ... Provided labels mark the center of each bird call", "Zenodo 10943500 description.pdf"),
     ("POW", "All vocalizations that were not short \"chips\" and could be identified were annotated.", "Zenodo 4656848 README.txt"),
     ("CEB", "strongly-labeled PAM soundscapes for evaluation with vocalization type annotations", "Zenodo 21821133 record description"),
     ("Perch 2.0", "we train the linear classifier using a softmax activation layer and a cross entropy loss using a target vector where each of the k target classes (species) has the value 1/k", "Perch 2.0 paper section 2.3 (local copy scratchpad/dl_perch/perch2.txt)"),
     ("Perch 2.0", "labeled audio datasets: Xeno-Canto ..., iNaturalist ..., the Tierstimmenarchiv ... and FSD50K", "Perch 2.0 paper section 2.1 (same copy)")]
with open(c.RES + "/provider_statements_quoted.csv", "w", newline="") as f:
    w = csv.writer(f); w.writerow(["set", "quoted_text", "source"]); w.writerows(Q)
print(len(rows), "source rows")
