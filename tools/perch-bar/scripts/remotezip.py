"""Minimal HTTP-Range file object so zipfile can read single members of a remote zip.

Only the bytes that are needed are transferred (central directory + the requested member
bytes).  Uses the generic User-Agent required by the task.  Every network request is counted
(`RangeFile.requests`, `RangeFile.bytes_fetched`) so the caller can report real transfer volume.
"""
import io, time, urllib.request

UA = "birdnet-behavior-validation/1.0"


class RangeFile(io.RawIOBase):
    def __init__(self, url, block=4 << 20):
        self.url, self.block = url, block
        self.pos = 0
        self.requests = 0
        self.bytes_fetched = 0
        self._cache_off, self._cache = 0, b""
        req = urllib.request.Request(url, headers={"User-Agent": UA, "Range": "bytes=0-0"})
        with urllib.request.urlopen(req, timeout=60) as r:
            cr = r.headers.get("Content-Range")  # bytes 0-0/TOTAL
            self.size = int(cr.split("/")[1])
            r.read()
        self.requests += 1

    def readable(self): return True
    def seekable(self): return True
    def tell(self): return self.pos

    def seek(self, off, whence=0):
        if whence == 0: self.pos = off
        elif whence == 1: self.pos += off
        else: self.pos = self.size + off
        return self.pos

    def _fetch(self, start, length):
        end = min(start + length, self.size) - 1
        for attempt in range(4):
            try:
                req = urllib.request.Request(self.url, headers={"User-Agent": UA, "Range": f"bytes={start}-{end}"})
                with urllib.request.urlopen(req, timeout=120) as r:
                    data = r.read()
                self.requests += 1
                self.bytes_fetched += len(data)
                if len(data) != end - start + 1:
                    raise IOError(f"short read {len(data)} != {end-start+1}")
                return data
            except Exception as e:  # retry with backoff; never hammer
                if attempt == 3: raise
                time.sleep(2 * (attempt + 1))

    def read(self, n=-1):
        if n < 0: n = self.size - self.pos
        n = min(n, self.size - self.pos)
        if n <= 0: return b""
        lo, hi = self._cache_off, self._cache_off + len(self._cache)
        if not (lo <= self.pos and self.pos + n <= hi):
            want = max(n, self.block)
            self._cache = self._fetch(self.pos, want)
            self._cache_off = self.pos
            lo = self.pos
        out = self._cache[self.pos - lo: self.pos - lo + n]
        self.pos += len(out)
        return out

    def readinto(self, b):
        d = self.read(len(b)); b[:len(d)] = d; return len(d)

    def read_range(self, start, length):
        """Exact-range read that bypasses the read-ahead cache (used for stored members)."""
        return self._fetch(start, length)
