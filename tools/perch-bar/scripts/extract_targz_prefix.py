"""Extract the COMPLETE members of a truncated .tar.gz prefix into a NEW empty directory.

usage: extract_targz_prefix.py PREFIX_FILE DEST_DIR LISTING_TSV [MAX_MEMBERS]
Safe by construction: only regular files whose name matches a strict whitelist regex are written, to a
flattened name inside DEST_DIR (no path components from the archive are used); nothing is executed.
The listing records member name, size, sha256 and whether the member was complete.
"""
import sys, os, re, tarfile, hashlib, zlib

src, dest, listing = sys.argv[1:4]
max_members = int(sys.argv[4]) if len(sys.argv) > 4 else 10**9
if os.path.exists(dest) and os.listdir(dest): sys.exit("dest not empty")
os.makedirs(dest, exist_ok=True)
ok = re.compile(r"^[A-Za-z0-9_]+/[A-Za-z0-9_]+\.flac$")
n = 0; skipped = 0
with open(listing, "w") as L:
    L.write("member\tsize\tsha256\tstatus\n")
    f = open(src, "rb")
    try:
        tf = tarfile.open(fileobj=f, mode="r|gz")
        for m in tf:
            if not m.isfile(): continue
            if not ok.match(m.name):
                skipped += 1; continue
            flat = m.name.replace("/", "__")
            try:
                data = tf.extractfile(m).read()
            except (EOFError, tarfile.ReadError, zlib.error, OSError) as e:
                L.write(f"{m.name}\t{m.size}\t\ttruncated\n"); break
            if len(data) != m.size:
                L.write(f"{m.name}\t{m.size}\t\ttruncated\n"); break
            with open(os.path.join(dest, flat), "wb") as o: o.write(data)
            L.write(f"{m.name}\t{m.size}\t{hashlib.sha256(data).hexdigest()}\tcomplete\n")
            n += 1
            if n >= max_members: break
    except (EOFError, tarfile.ReadError, zlib.error) as e:
        L.write(f"#stream ended: {type(e).__name__}\n")
print("complete members written:", n, "skipped(non-whitelisted):", skipped)
