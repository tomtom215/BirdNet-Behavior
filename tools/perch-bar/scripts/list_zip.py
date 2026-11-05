"""List the members of a remote zip (central directory only).  usage: list_zip.py URL OUT_TSV"""
import sys, zipfile
sys.path.insert(0, __file__.rsplit("/", 1)[0])
from remotezip import RangeFile
url, out = sys.argv[1], sys.argv[2]
rf = RangeFile(url, block=1 << 20)
zf = zipfile.ZipFile(rf)
with open(out, "w") as f:
    f.write("name\tcompress_type\tcompress_size\tfile_size\theader_offset\tcrc\n")
    for i in zf.infolist():
        f.write(f"{i.filename}\t{i.compress_type}\t{i.compress_size}\t{i.file_size}\t{i.header_offset}\t{i.CRC:08x}\n")
print(out, len(zf.infolist()), "members; requests", rf.requests, "bytes", rf.bytes_fetched, "zip size", rf.size)
