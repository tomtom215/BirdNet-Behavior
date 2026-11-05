"""Extract members (or byte-prefixes of members) of a remote Zenodo zip via HTTP Range, into a NEW empty dir.

usage: zip_extract.py ZIP_URL DEST_DIR LISTING_TSV MEMBER[:MAXBYTES] [MEMBER[:MAXBYTES] ...]
 * a member without :MAXBYTES is read completely and its CRC-32 is verified by zipfile (BadZipFile on mismatch)
 * a member with :MAXBYTES keeps only the first MAXBYTES uncompressed bytes (a prefix of the file; for the
   hour-long FLAC files this is the first few minutes) -- recorded as 'prefix' in the listing
 * member names are used only as dictionary keys into the zip's central directory; the output file name is the
   member's basename (checked against a strict regex), so nothing from the archive controls a path.
Nothing downloaded is executed or imported.
"""
import sys, os, re, hashlib, zipfile
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from remotezip import RangeFile

url, dest, listing = sys.argv[1:4]
specs = sys.argv[4:]
if os.path.exists(dest) and os.listdir(dest): sys.exit("dest not empty")
os.makedirs(dest, exist_ok=True)
safe = re.compile(r"^[A-Za-z0-9_.\-]+$")
rf = RangeFile(url, block=4 << 20)
zf = zipfile.ZipFile(rf)
new = not os.path.exists(listing)
with open(listing, "a") as L:
    if new: L.write("zip_url\tmember\tout_file\tuncompressed_member_size\tbytes_kept\tkind\tcrc32_member\tsha256_kept\n")
    for spec in specs:
        name, _, mx = spec.partition(":")
        info = zf.getinfo(name)
        base = os.path.basename(name)
        if not safe.match(base): sys.exit(f"unsafe name {base!r}")
        out = os.path.join(dest, base)
        h = hashlib.sha256(); kept = 0
        with zf.open(info) as src, open(out, "wb") as o:
            limit = int(mx) if mx else None
            while True:
                want = 1 << 20 if limit is None else min(1 << 20, limit - kept)
                if want <= 0: break
                b = src.read(want)
                if not b: break
                o.write(b); h.update(b); kept += len(b)
        kind = "prefix" if (limit is not None and kept < info.file_size) else "full(crc-verified)" if limit is None else "full(crc-not-checked)"
        L.write(f"{url}\t{name}\t{out}\t{info.file_size}\t{kept}\t{kind}\t{info.CRC:08x}\t{h.hexdigest()}\n")
        print(base, info.file_size, kept, kind)
print("requests", rf.requests, "bytes fetched", rf.bytes_fetched)
