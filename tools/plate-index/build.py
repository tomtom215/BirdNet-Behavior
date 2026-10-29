#!/usr/bin/env python3
"""Build the public-domain plate index for the kiosk's field-guide plate.

Maps BirdNET species to 19th-century plates on Wikimedia Commons (Gould,
Naumann, von Wright) and writes
crates/birdnet-integrations/data/plates/index.json plus a report.

Python 3 standard library only.  Run it isolated:

    python3 -I tools/plate-index/build.py --help

Every value in the index comes from a Commons API response that is cached on
disk, so a rerun repeats no request and writes identical bytes.  The rules
(licence, taxon matching, one species per plate, quality) are documented in
tools/plate-index/README.md and enforced below; none is configurable except
the numeric thresholds.
"""

import argparse
import collections
import datetime
import email.utils
import hashlib
import html.parser
import json
import os
import pathlib
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

API = "https://commons.wikimedia.org/w/api.php"
FILE_PATH = "https://commons.wikimedia.org/wiki/Special:FilePath/"
USER_AGENT = (
    "BirdNet-Behavior-plate-index/0.1 "
    "(https://github.com/tomtom215/BirdNet-Behavior; index build)"
)

MIN_INTERVAL_FLOOR = 1.5  # seconds between requests; below this we refuse
MAX_RETRIES = 8  # throttle retries per request before stopping
BACKOFF_START = 10.0
BACKOFF_CAP = 300.0
RETRY_AFTER_CAP = 900.0  # never sleep longer than this on one header
BATCH = 50  # files per generator request

REPO = pathlib.Path(__file__).resolve().parents[2]
DEFAULT_LABELS = REPO / "crates/birdnet-core/data/labels/birdnet-v24-en.txt"
DEFAULT_OUT = REPO / "crates/birdnet-integrations/data/plates/index.json"
DEFAULT_REPORT = REPO / "tools/plate-index/report.md"

# --------------------------------------------------------------------------
# The works.  Title, author, years and the candidate category titles are typed
# constants, not API data.  The years were checked against a bibliography; the
# span of the Century is disputed (1830-1832 here, 1830-1833 in one paper), and
# Naumann's and von Wright's are left empty on purpose because none could be
# verified.  The category names were typed from Commons' own category tree.  A
# candidate is used only if the API says the category exists; the others are
# ignored and reported.
# --------------------------------------------------------------------------
WORKS = {
    # Crawled in this order, so the required works come first if a run has to stop.
    "gould-europe": {
        "title": "The Birds of Europe",
        "author": "John Gould",
        "years": "1832-1837",
        "candidates": ["Category:The Birds of Europe (Gould)"],
    },
    "naumann": {
        "title": "Naturgeschichte der Vögel Mitteleuropas",
        "author": "Johann Friedrich Naumann; Carl R. Hennicke",
        "years": "",
        "candidates": ["Category:Naturgeschichte der Vögel Mitteleuropas"],
    },
    "von-wright": {
        "title": "Svenska fåglar",
        "author": "Magnus von Wright; Wilhelm von Wright; Ferdinand von Wright",
        "years": "",
        "candidates": ["Category:Svenska fåglar (von Wright)"],
    },
    "gould-asia": {
        "title": "The Birds of Asia",
        "author": "John Gould; Richard Bowdler Sharpe",
        "years": "1850-1883",
        "candidates": ["Category:The Birds of Asia (John Gould)"],
    },
    "gould-britain": {
        "title": "The Birds of Great Britain",
        "author": "John Gould",
        "years": "1862-1873",
        # Not Category:The birds of Great Britain: that is William Lewin's 1795 work.
        "candidates": ["Category:The Birds of Great Britain (illustrations by John Gould)"],
    },
    "gould-century": {
        "title": "A Century of Birds from the Himalaya Mountains",
        "author": "John Gould; Elizabeth Gould",
        "years": "1830-1832",
        "candidates": [
            "Category:A Century of Birds from the Himalaya Mountains (illustrations by John Gould)"
        ],
    },
    "gould-australia": {
        "title": "The Birds of Australia",
        "author": "John Gould",
        "years": "1840-1848",
        "candidates": ["Category:The Birds of Australia (John Gould)"],
    },
    "gould-new-guinea": {
        "title": "The Birds of New Guinea and the adjacent Papuan Islands",
        "author": "John Gould; Richard Bowdler Sharpe",
        "years": "1875-1888",
        "candidates": ["Category:The Birds of New Guinea (John Gould)"],
    },
}
# Tie-break rank when a species has equally wide plates in several works:
# gould-* over naumann over von-wright.
WORK_RANK = {k: (0 if k.startswith("gould-") else 1 if k == "naumann" else 2) for k in WORKS}

# Prefixes for `--discover` (list=allcategories), to find categories by name.
DISCOVER_PREFIXES = [
    "The Birds of ",
    "The birds of ",
    "Birds of Australia",
    "Birds of Great Britain",
    "Birds of New Guinea",
    "Birds of Europe",
    "Birds of Asia",
    "A Century of Birds",
    "A century of birds",
    "Svenska fåglar",
    "Naturgeschichte der V",
    "Gould",
    "John Gould",
]

# ---------------------------------------------------------------- licences
FREE_RE = re.compile(
    r"^(?:public[ -]domain\b.*|pd(?:[-_ /].*)?|cc0\b.*|cc[- ]zero\b.*)$", re.IGNORECASE
)
# Markers of a restricted licence.  A file showing any of these in the short
# name, usage terms or licence slug is rejected even if another field says PD.
RESTRICTED_RE = re.compile(
    r"(cc[- ]?by|gfdl|attribution|share[- ]?alike|fair[- ]use|all rights reserved|"
    r"cc[- ]?nc|non-?commercial|copyrighted)",
    re.IGNORECASE,
)
ACCEPT_MIMES = {"image/jpeg", "image/png"}
# Words in a file name that mark a non-plate (title page, cover, index, ...).
NON_PLATE_RE = re.compile(
    r"\b(title[ _-]?page|cover|index|text|map|frontispiece|contents|dedication|"
    r"preface|introduction|errata|subscribers?|binding|spine|advertisements?|"
    r"table of contents|list of)\b",
    re.IGNORECASE,
)
SPECIES_CAT_RE = re.compile(r"^Category:([A-Z][a-z]+ [a-z]+(?:-[a-z]+)*) \(illustrations\)$")
ILLUSTRATIONS_CAT_RE = re.compile(r"\(illustrations\)$")
BINOMIAL_RE = re.compile(r"[A-Z][a-z]+ [a-z]+(?:-[a-z]+)*")
LABEL_BINOMIAL_RE = re.compile(r"^[A-Z][a-z]+ [a-z]+(?:-[a-z]+)*$")
EXT_KEYS = (
    "Artist|Credit|Copyrighted|LicenseShortName|UsageTerms|License|LicenseUrl|"
    "AttributionRequired|NonFree"
)

# 40 common European and North American species used for the coverage figure
# in the report (a fixed list so the figure is comparable between runs).
SAMPLE_SPECIES = [
    # Europe
    "Erithacus rubecula", "Parus major", "Cyanistes caeruleus", "Turdus merula",
    "Turdus philomelos", "Turdus pilaris", "Passer domesticus", "Fringilla coelebs",
    "Sturnus vulgaris", "Pica pica", "Corvus corone", "Columba palumbus",
    "Troglodytes troglodytes", "Prunella modularis", "Carduelis carduelis",
    "Chloris chloris", "Sylvia atricapilla", "Phylloscopus collybita",
    "Hirundo rustica", "Apus apus",
    # North America
    "Cardinalis cardinalis", "Cyanocitta cristata", "Poecile atricapillus",
    "Turdus migratorius", "Zenaida macroura", "Haemorhous mexicanus",
    "Spinus tristis", "Sitta carolinensis", "Melospiza melodia", "Corvus brachyrhynchos",
    "Agelaius phoeniceus", "Mimus polyglottos", "Dryobates pubescens",
    "Setophaga coronata", "Junco hyemalis", "Bombycilla cedrorum",
    "Thryothorus ludovicianus", "Sialia sialis", "Quiscalus quiscula",
    "Archilochus colubris",
]


class Stop(Exception):
    """The run must stop now and report (throttled, budget spent, offline miss)."""


class ApiError(Exception):
    """The API answered with an error that retrying will not fix."""


# ------------------------------------------------------------------- HTTP
class Http:
    """Polite HTTP: one request at a time, a minimum gap, backoff on 429."""

    def __init__(self, min_interval, max_requests, offline):
        self.min_interval = min_interval
        self.max_requests = max_requests
        self.offline = offline
        self.network_requests = 0
        self.throttled = 0
        self._last = 0.0

    def _pace(self):
        wait = self._last + self.min_interval - time.monotonic()
        if wait > 0:
            time.sleep(wait)

    @staticmethod
    def _retry_after(headers):
        raw = headers.get("Retry-After") if headers else None
        if not raw:
            return None
        try:
            return max(0.0, float(raw))
        except ValueError:
            pass
        try:
            when = email.utils.parsedate_to_datetime(raw)
            return max(0.0, when.timestamp() - time.time())
        except (TypeError, ValueError):
            return None

    def fetch(self, url, want_json):
        """Return (body bytes, parsed JSON or None).  Stops, never hammers."""
        if self.offline:
            raise Stop("--offline and the response is not cached: " + url)
        delay = BACKOFF_START
        for attempt in range(MAX_RETRIES + 1):
            if self.network_requests >= self.max_requests:
                raise Stop(
                    "request budget of %d reached (--max-requests); rerun to resume"
                    % self.max_requests
                )
            self._pace()
            self.network_requests += 1
            retry_after = None
            reason = None
            try:
                req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
                with urllib.request.urlopen(req, timeout=90) as resp:
                    body = resp.read()
                    status = resp.status
                    headers = resp.headers
            except urllib.error.HTTPError as err:
                status = err.code
                headers = err.headers
                body = err.read()
                if status in (429, 500, 502, 503, 504):
                    reason = "HTTP %d" % status
                    retry_after = self._retry_after(headers)
                else:
                    raise ApiError("HTTP %d for %s" % (status, url)) from err
            except (urllib.error.URLError, TimeoutError, ConnectionError) as err:
                reason = "network error: %s" % err
                headers = None
                body = b""
            finally:
                self._last = time.monotonic()
            parsed = None
            if reason is None and want_json:
                try:
                    parsed = json.loads(body.decode("utf-8"))
                except (UnicodeDecodeError, ValueError):
                    reason = "non-JSON body (%d bytes)" % len(body)
                    retry_after = self._retry_after(headers)
                else:
                    err_obj = parsed.get("error") if isinstance(parsed, dict) else None
                    if err_obj and err_obj.get("code") == "maxlag":
                        reason = "maxlag (%s)" % err_obj.get("lag", "?")
                        retry_after = self._retry_after(headers)
                    elif err_obj:
                        raise ApiError("API error %s: %s" % (err_obj.get("code"), err_obj.get("info")))
            if reason is None:
                return body, parsed
            self.throttled += 1
            if attempt == MAX_RETRIES:
                raise Stop("still refused after %d retries (%s); stopping" % (MAX_RETRIES, reason))
            # Never wait less than the server asked, and never less than the
            # exponential backoff either: a `Retry-After: 1` repeated while the
            # shared address stays throttled would otherwise spend all the
            # retries in a minute.
            pause = max(min(retry_after, RETRY_AFTER_CAP) if retry_after is not None else 0.0, delay)
            delay = min(delay * 2, BACKOFF_CAP)
            log("  %s; waiting %.0f s (retry %d of %d)" % (reason, pause, attempt + 1, MAX_RETRIES))
            time.sleep(pause)
            self._last = time.monotonic()
        raise Stop("unreachable")  # pragma: no cover


class Api:
    """The Commons action API with an on-disk cache keyed by the request."""

    def __init__(self, http, cache_dir, refresh):
        self.http = http
        self.cache_dir = pathlib.Path(cache_dir)
        self.refresh = refresh
        self.cache_hits = 0
        self.cache_dir.mkdir(parents=True, exist_ok=True)

    @staticmethod
    def _params(params):
        full = {"action": "query", "format": "json", "formatversion": "2", "maxlag": "5"}
        full.update(params)
        return full

    def get(self, params):
        full = self._params(params)
        key = hashlib.sha256(json.dumps(full, sort_keys=True).encode("utf-8")).hexdigest()[:32]
        path = self.cache_dir / (key + ".json")
        if not self.refresh and path.exists():
            self.cache_hits += 1
            return json.loads(path.read_text(encoding="utf-8"))["response"]
        url = API + "?" + urllib.parse.urlencode(sorted(full.items()))
        body, parsed = self.http.fetch(url, want_json=True)
        record = {
            "params": full,
            "fetched_utc": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "response": parsed,
        }
        tmp = path.with_suffix(".tmp")
        tmp.write_text(json.dumps(record, sort_keys=True, ensure_ascii=False), encoding="utf-8")
        tmp.replace(path)
        return parsed

    def get_all(self, params):
        """Follow the `continue` object and merge the pages of every response."""
        merged = collections.OrderedDict()
        extra = {}
        current = dict(params)
        while True:
            resp = self.get(current)
            query = resp.get("query") or {}
            for page in query.get("pages") or []:
                key = page.get("pageid", page.get("title"))
                into = merged.setdefault(key, {})
                for field, value in page.items():
                    if field == "categories":
                        have = into.setdefault("categories", [])
                        titles = {c["title"] for c in have}
                        have.extend(c for c in value if c["title"] not in titles)
                    elif field == "imageinfo":
                        into.setdefault("imageinfo", value)
                    else:
                        into.setdefault(field, value)
            for field, value in query.items():
                if field != "pages":
                    extra.setdefault(field, []).extend(value if isinstance(value, list) else [value])
            cont = resp.get("continue")
            if not cont:
                return list(merged.values()), extra
            current = dict(params)
            current.update(cont)


def log(msg):
    print(msg, file=sys.stderr, flush=True)


# ------------------------------------------------------------------ labels
def load_labels(path):
    """Return {binomial: common name} for the species lines of the label list."""
    species = {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if not line:
                continue
            sci, _, common = line.partition("_")
            if LABEL_BINOMIAL_RE.match(sci):
                species.setdefault(sci, common)
    return species


# ------------------------------------------------------------- text helpers
class _Text(html.parser.HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.parts = []
        self._skip = 0

    def handle_starttag(self, tag, attrs):
        if tag in ("script", "style"):
            self._skip += 1
        elif tag in ("br", "p", "div", "li", "tr"):
            self.parts.append(" ")

    def handle_endtag(self, tag):
        if tag in ("script", "style") and self._skip:
            self._skip -= 1

    def handle_data(self, data):
        if not self._skip:
            self.parts.append(data)


def strip_html(value):
    if not value:
        return ""
    parser = _Text()
    parser.feed(str(value))
    parser.close()
    return " ".join("".join(parser.parts).split())


def meta(extmetadata, key):
    item = (extmetadata or {}).get(key)
    if not isinstance(item, dict):
        return ""
    return str(item.get("value", "")).strip()


# --------------------------------------------------------------- licensing
def licence_verdict(ext):
    """Return (accepted, licence label).  The rule, exactly as in the README."""
    short = meta(ext, "LicenseShortName")
    terms = meta(ext, "UsageTerms")
    slug = meta(ext, "License")
    copyrighted = meta(ext, "Copyrighted").lower()
    label = short or terms or "(none)"
    if copyrighted in ("true", "yes", "1"):
        return False, label + " [Copyrighted=True]"
    if any(RESTRICTED_RE.search(v) for v in (short, terms, slug) if v):
        return False, label
    free = any(FREE_RE.match(strip_html(v)) for v in (short, terms) if v)
    return free, label


# ----------------------------------------------------------- taxon matching
def species_from_categories(cats):
    return sorted({m.group(1) for c in cats if (m := SPECIES_CAT_RE.match(c))})


def base_name(title):
    return title.split(":", 1)[1] if ":" in title else title


def taxon_of(title, cats, labels):
    """Return (state, species or None, matched_by or None).

    state is one of: ok, multi, none, unlabelled.
    """
    found = species_from_categories(cats)
    if len(found) > 1:
        return "multi", None, None
    if len(found) == 1:
        if found[0] in labels:
            return "ok", found[0], "category"
        return "unlabelled", found[0], None
    # No species-illustrations category: the file name, exact match only.
    name = base_name(title)
    start = BINOMIAL_RE.match(name)
    if not start or start.group(0) not in labels:
        return "none", None, None
    named = {m.group(0) for m in BINOMIAL_RE.finditer(name) if m.group(0) in labels}
    if len(named) > 1:
        return "multi", None, None
    return "ok", start.group(0), "filename"


def quality_problem(title, info, min_width):
    """Return a reason a file is not a usable plate, or None."""
    if info.get("mime") not in ACCEPT_MIMES:
        return "format"
    if NON_PLATE_RE.search(base_name(title).rsplit(".", 1)[0]):
        return "non-plate name"
    if int(info.get("width") or 0) < min_width:
        return "small"
    return None


# ----------------------------------------------------------------- crawling
def member_params(category):
    return {
        "generator": "categorymembers",
        "gcmtitle": category,
        "gcmtype": "file|subcat",
        "gcmlimit": str(BATCH),
        "prop": "imageinfo|categories",
        "iiprop": "url|size|mime|extmetadata",
        "iiextmetadatafilter": EXT_KEYS,
        "cllimit": "max",
    }


def resolve_roots(api, keys):
    """One batched request: which candidate categories exist, and how big."""
    titles = sorted({t for k in keys for t in WORKS[k]["candidates"]})
    info = {}
    for i in range(0, len(titles), BATCH):
        pages, _ = api.get_all({"prop": "categoryinfo", "titles": "|".join(titles[i : i + BATCH])})
        for page in pages:
            info[page["title"]] = page
    roots = {}
    for k in keys:
        roots[k] = []
        for cand in WORKS[k]["candidates"]:
            page = info.get(cand) or info.get(cand.replace("_", " "))
            if not page or page.get("missing"):
                continue
            ci = page.get("categoryinfo") or {}
            roots[k].append(
                {
                    "title": page["title"],
                    "files": ci.get("files", 0),
                    "subcats": ci.get("subcats", 0),
                    "size": ci.get("size", 0),
                }
            )
    return roots, info


def crawl(api, roots, max_depth, max_categories):
    """Walk a work's category and its sub-categories; return (files, tree)."""
    files = {}
    tree = collections.OrderedDict()
    queue = collections.deque((r["title"], 0, None) for r in roots)
    while queue:
        cat, depth, parent = queue.popleft()
        if cat in tree:
            continue
        pages, _ = api.get_all(member_params(cat))
        subs = sorted(p["title"] for p in pages if p.get("ns") == 14)
        n_files = 0
        for page in sorted((p for p in pages if p.get("ns") == 6), key=lambda p: p["title"]):
            n_files += 1
            files.setdefault(page["title"], page)
        entry = {"depth": depth, "parent": parent, "files": n_files, "subcats": subs, "skipped": []}
        tree[cat] = entry
        for sub in subs:
            if sub in tree:
                continue
            if ILLUSTRATIONS_CAT_RE.search(sub):
                entry["skipped"].append((sub, "taxon (illustrations) category"))
            elif depth + 1 > max_depth:
                entry["skipped"].append((sub, "deeper than --max-depth"))
            elif len(tree) + len(queue) >= max_categories:
                entry["skipped"].append((sub, "over --max-categories"))
            else:
                queue.append((sub, depth + 1, cat))
        log("  %s: %d files, %d sub-categories" % (cat, n_files, len(subs)))
    return files, tree


# ----------------------------------------------------------- classification
def classify(files, labels, min_width):
    """Sort every file into exactly one outcome; return (stats, candidates, detail)."""
    stats = collections.Counter()
    lic_reject = collections.Counter()  # licence label -> files
    detail = collections.defaultdict(list)  # outcome -> [(title, note)]
    candidates = []
    licence_only = collections.defaultdict(set)  # species -> licence labels (otherwise fine)
    unlabelled = collections.Counter()
    for title in sorted(files):
        page = files[title]
        stats["seen"] += 1
        infos = page.get("imageinfo") or []
        if not infos:
            stats["no imageinfo"] += 1
            detail["no imageinfo"].append((title, ""))
            continue
        info = infos[0]
        ext = info.get("extmetadata") or {}
        cats = [c["title"] for c in page.get("categories") or []]
        free, label = licence_verdict(ext)
        state, species, matched = taxon_of(title, cats, labels)
        problem = quality_problem(title, info, min_width)
        if not free:
            stats["rejected: licence"] += 1
            lic_reject[label] += 1
            if state == "ok" and problem is None:
                licence_only[species].add(label)
            continue
        if state == "multi":
            stats["rejected: multi-species"] += 1
            continue
        if problem:
            stats["rejected: small or non-plate"] += 1
            stats["  of which " + problem] += 1
            detail[problem].append((title, "%sx%s %s" % (info.get("width"), info.get("height"), info.get("mime"))))
            continue
        if state == "none":
            stats["rejected: no species"] += 1
            continue
        if state == "unlabelled":
            stats["rejected: species not in BirdNET labels"] += 1
            unlabelled[species] += 1
            continue
        stats["accepted"] += 1
        if matched == "filename":
            stats["  of which matched by filename"] += 1
        if meta(ext, "AttributionRequired").lower() == "true":
            stats["  of which AttributionRequired=true (kept)"] += 1
        candidates.append(
            {
                "species": species,
                "file": base_name(title),
                "author": strip_html(meta(ext, "Artist")),
                "credit": strip_html(meta(ext, "Credit")),
                "licence": label,
                "licence_url": meta(ext, "LicenseUrl"),
                "width": int(info.get("width") or 0),
                "height": int(info.get("height") or 0),
                "page_url": info.get("descriptionurl") or "",
                "matched_by": matched,
            }
        )
    return stats, lic_reject, candidates, licence_only, unlabelled, detail


def choose(per_work_candidates):
    """Pick one plate per species: wider, then work rank, then file title."""
    best = {}
    all_by_species = collections.defaultdict(list)
    for work, cands in per_work_candidates.items():
        for c in cands:
            all_by_species[c["species"]].append((work, c))
    for species, options in all_by_species.items():
        options.sort(key=lambda o: (-o[1]["width"], WORK_RANK[o[0]], o[1]["file"], o[0]))
        best[species] = options[0]
    return best, all_by_species


# -------------------------------------------------------------------- output
def dump_json(obj):
    return json.dumps(obj, sort_keys=True, indent=2, ensure_ascii=False) + "\n"


def write_atomic(path, text):
    path = pathlib.Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(text, encoding="utf-8")
    tmp.replace(path)


def build_index(best, found_works, notes, generated):
    works = {}
    for key in found_works:
        spec = WORKS[key]
        works[key] = {
            "title": spec["title"],
            "author": spec["author"],
            "years": spec["years"],
            "commons_category": notes[key]["root"],
            "note": notes[key]["note"],
        }
    plates = {}
    for species, (work, c) in best.items():
        plates[species] = {
            "work": work,
            "file": c["file"],
            "licence": c["licence"],
            "licence_url": c["licence_url"],
            "width": c["width"],
            "height": c["height"],
            "page_url": c["page_url"],
            "matched_by": c["matched_by"],
        }
    return {"schema": 1, "generated": generated, "works": works, "plates": plates}


def md_table(header, rows):
    out = ["| " + " | ".join(header) + " |", "|" + "|".join(" --- " for _ in header) + "|"]
    for row in rows:
        out.append("| " + " | ".join(str(c) for c in row) + " |")
    return out


def build_report(ctx):
    labels = ctx["labels"]
    best = ctx["best"]
    lines = ["# Plate index report", ""]
    lines.append("Generated %s from Wikimedia Commons API responses (cached)." % ctx["generated"])
    lines.append("")
    lines.append("## Totals")
    lines.append("")
    lines.append("- Species with a plate in the index: **%d**" % len(best))
    lines.append(
        "- BirdNET V2.4 binomials: %d (the label list has %d lines; the other %d are non-species classes). "
        "Coverage: **%.2f%%**" % (len(labels), ctx["label_lines"], ctx["label_lines"] - len(labels), 100.0 * len(best) / len(labels))
    )
    by_work = collections.Counter(w for w, _ in best.values())
    lines.append("")
    lines.extend(md_table(["Work", "Species chosen from it"], [(k, by_work.get(k, 0)) for k in ctx["found"]]))
    if ctx["incomplete"]:
        lines.append("")
        lines.append("**Incomplete works (run stopped):**")
        for k, why in ctx["incomplete"]:
            lines.append("- `%s`: %s" % (k, why))
    lines.append("")
    lines.append("## Per work")
    outcomes = [
        "seen",
        "rejected: licence",
        "rejected: multi-species",
        "rejected: small or non-plate",
        "rejected: no species",
        "rejected: species not in BirdNET labels",
        "accepted",
    ]
    for k in ctx["found"]:
        res = ctx["results"].get(k)
        lines.append("")
        years = " (%s)" % WORKS[k]["years"] if WORKS[k]["years"] else ""
        lines.append("### %s - %s%s" % (k, WORKS[k]["title"], years))
        lines.append("")
        if res is None:
            lines.append("Not crawled (run stopped before this work).")
            continue
        lines.append("Roots: " + "; ".join("`%s` (%d files, %d sub-categories)" % (r["title"], r["files"], r["subcats"]) for r in ctx["roots"][k]))
        lines.append("")
        lines.extend(md_table(["Outcome", "Files"], [(o, res["stats"].get(o, 0)) for o in outcomes]))
        extras = [(o.strip(), n) for o, n in sorted(res["stats"].items()) if o.startswith("  ")]
        if extras:
            lines.append("")
            lines.append("Detail: " + "; ".join("%s: %d" % e for e in extras))
        lines.append("")
        lines.append("Rejected by licence (files per licence):")
        lines.append("")
        if res["lic_reject"]:
            lines.extend(md_table(["Licence", "Files"], sorted(res["lic_reject"].items(), key=lambda kv: (-kv[1], kv[0]))))
        else:
            lines.append("none")
        distinct = sorted({c["species"] for c in res["candidates"]})
        only_lic = sorted(s for s in res["licence_only"] if s not in distinct)
        lines.append("")
        lines.append(
            "Species with an accepted plate in this work: %d.  Species whose only files in this "
            "work were rejected on licence: %d." % (len(distinct), len(only_lic))
        )
        lines.append("")
        lines.append("Category tree crawled:")
        lines.append("")
        for cat, ent in res["tree"].items():
            lines.append("%s- `%s` (%d files)" % ("  " * ent["depth"], cat, ent["files"]))
            for sub, why in ent["skipped"]:
                lines.append("%s  - not entered: `%s` (%s)" % ("  " * ent["depth"], sub, why))
    lines.append("")
    lines.append("## Species with no accepted plate because of licence")
    lines.append("")
    lines.append(
        "A species is listed when at least one file passed every rule except the licence, "
        "and no file in any crawled work passed all of them."
    )
    lic_missing = {}
    for k, res in ctx["results"].items():
        for sp, labs in res["licence_only"].items():
            lic_missing.setdefault(sp, set()).update(labs)
    lic_missing = {sp: labs for sp, labs in lic_missing.items() if sp not in best}
    lines.append("")
    lines.append("Count: **%d**." % len(lic_missing))
    lines.append("")
    for sp in sorted(lic_missing):
        lines.append("- %s (%s): %s" % (sp, labels[sp], ", ".join(sorted(lic_missing[sp]))))
    lines.append("")
    lines.append("## Files filtered as non-plates, wrong format or small")
    lines.append("")
    for k, res in ctx["results"].items():
        for problem in ("non-plate name", "format", "small"):
            items = res["detail"].get(problem, [])
            if not items:
                continue
            lines.append("- `%s`, %s: %d files" % (k, problem, len(items)))
            if problem != "small":
                for title, note in items[:60]:
                    lines.append("  - %s (%s)" % (base_name(title), note))
                if len(items) > 60:
                    lines.append("  - ... and %d more" % (len(items) - 60))
    lines.append("")
    lines.append("## Commons species categories not in the BirdNET label list")
    lines.append("")
    lines.append(
        "Files whose single `(illustrations)` binomial is not an exact label (new taxonomy, "
        "or not in BirdNET).  Never mapped by synonym."
    )
    lines.append("")
    total_unl = collections.Counter()
    for res in ctx["results"].values():
        total_unl.update(res["unlabelled"])
    lines.append("Distinct binomials: %d; files: %d." % (len(total_unl), sum(total_unl.values())))
    lines.append("")
    for sp, n in sorted(total_unl.items(), key=lambda kv: (-kv[1], kv[0]))[:40]:
        lines.append("- %s: %d" % (sp, n))
    lines.append("")
    lines.append("## Coverage of a fixed sample of 40 common European and North American species")
    lines.append("")
    have = [s for s in SAMPLE_SPECIES if s in best]
    lines.append("%d of %d have a plate." % (len(have), len(SAMPLE_SPECIES)))
    lines.append("")
    rows = []
    for sp in SAMPLE_SPECIES:
        if sp not in labels:
            rows.append((sp, "(not a BirdNET label)", "-"))
        elif sp in best:
            rows.append((sp, labels[sp], best[sp][0]))
        else:
            rows.append((sp, labels[sp], "none"))
    lines.extend(md_table(["Binomial", "Common name", "Plate from"], rows))
    lines.append("")
    return "\n".join(lines)


# ------------------------------------------------------------------ verify
def verify(index_path, labels):
    """Re-load the index and assert its invariants; return a list of problems."""
    problems = []
    text = pathlib.Path(index_path).read_text(encoding="utf-8")
    duplicated = []

    def hook(pairs):
        keys = [k for k, _ in pairs]
        duplicated.extend(sorted({k for k in keys if keys.count(k) > 1}))
        return collections.OrderedDict(pairs)

    data = json.loads(text, object_pairs_hook=hook)
    problems.extend("duplicate key " + k for k in duplicated)

    def check_sorted(node, where):
        if isinstance(node, collections.OrderedDict):
            keys = list(node)
            if keys != sorted(keys):
                problems.append("keys not sorted in " + where)
            for k, v in node.items():
                check_sorted(v, where + "/" + k)

    check_sorted(data, "")
    if text != dump_json(json.loads(text)):
        problems.append("file is not in canonical form (sorted keys, 2-space indent, trailing newline)")
    if data.get("schema") != 1:
        problems.append("schema is not 1")
    for species, plate in data.get("plates", {}).items():
        if species not in labels:
            problems.append("species not in label list: " + species)
        if plate.get("work") not in data.get("works", {}):
            problems.append("%s: work %r not in works" % (species, plate.get("work")))
        ext = {
            "LicenseShortName": {"value": plate.get("licence", "")},
            "UsageTerms": {"value": ""},
        }
        if not licence_verdict(ext)[0]:
            problems.append("%s: licence %r is not accepted" % (species, plate.get("licence")))
        if plate.get("width", 0) < 800:
            problems.append("%s: width %r < 800" % (species, plate.get("width")))
        if plate.get("matched_by") not in ("category", "filename"):
            problems.append("%s: bad matched_by" % species)
        if not str(plate.get("page_url", "")).startswith("https://commons.wikimedia.org/wiki/File:"):
            problems.append("%s: page_url is not a Commons file page" % species)
        if set(plate) != {
            "work", "file", "licence", "licence_url",
            "width", "height", "page_url", "matched_by",
        }:
            problems.append("%s: unexpected plate fields %s" % (species, sorted(plate)))
    return problems


# ------------------------------------------------------------------ thumbs
def fetch_thumbs(http, data, directory, per_work, only=None):
    """Download `per_work` thumbnails (600 px) of index entries for each work."""
    directory = pathlib.Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    by_work = collections.defaultdict(list)
    for species in sorted(data["plates"]):
        by_work[data["plates"][species]["work"]].append(species)
    done = []
    for work in sorted(by_work):
        names = by_work[work]
        if not names or (only and work not in only):
            continue
        step = max(1, len(names) // per_work)
        for species in names[::step][:per_work]:
            plate = data["plates"][species]
            safe = re.sub(r"[^A-Za-z0-9]+", "_", species)
            target = directory / ("%s__%s.jpg" % (work, safe))
            if target.exists() and target.stat().st_size > 0:
                done.append((work, species, plate["file"], target))
                continue
            url = FILE_PATH + urllib.parse.quote(plate["file"].replace(" ", "_")) + "?width=600"
            body, _ = http.fetch(url, want_json=False)
            target.write_bytes(body)
            done.append((work, species, plate["file"], target))
            log("  %s <- %s" % (target.name, plate["file"]))
    return done


# --------------------------------------------------------------------- main
def parse_args(argv):
    default_cache = pathlib.Path(os.environ.get("XDG_CACHE_HOME") or pathlib.Path.home() / ".cache") / "birdnet-plate-index"
    p = argparse.ArgumentParser(
        description="Build the public-domain plate index (Gould, Naumann, von Wright) from the "
        "Wikimedia Commons API.  Resumable: every response is cached, so a rerun repeats nothing.",
        epilog="Run with `python3 -I`.  See tools/plate-index/README.md for the rules.",
    )
    p.add_argument("--cache-dir", default=str(default_cache), help="where raw API responses are cached (default: %(default)s)")
    p.add_argument("--refresh", action="store_true", help="ignore the cache and fetch everything again")
    p.add_argument("--offline", action="store_true", help="never use the network; fail if a response is not cached")
    p.add_argument("--works", help="comma-separated work keys to include (default: every work found); choose which to ship")
    p.add_argument("--out", default=str(DEFAULT_OUT), help="index.json path (default: %(default)s)")
    p.add_argument("--report", default=str(DEFAULT_REPORT), help="report path (default: %(default)s)")
    p.add_argument("--labels", default=str(DEFAULT_LABELS), help="BirdNET label list (default: %(default)s)")
    p.add_argument("--generated", help="value of `generated` (YYYY-MM-DD, default: today UTC)")
    p.add_argument("--min-interval", type=float, default=2.0, help="seconds between requests, at least %.1f (default: %%(default)s)" % MIN_INTERVAL_FLOOR)
    p.add_argument("--max-requests", type=int, default=400, help="network requests allowed in this run (default: %(default)s)")
    p.add_argument("--max-depth", type=int, default=3, help="sub-category depth below a work's root (default: %(default)s)")
    p.add_argument("--max-categories", type=int, default=200, help="categories crawled per work (default: %(default)s)")
    p.add_argument("--min-width", type=int, default=800, help="smallest accepted plate width in px (default: %(default)s)")
    p.add_argument("--partial", action="store_true", help="write the index even if some works could not be finished")
    p.add_argument("--verify", action="store_true", help="only re-load --out and check its invariants, then exit")
    p.add_argument("--discover", action="store_true", help="list Commons categories matching the known name prefixes, then exit")
    p.add_argument("--fetch-thumbs", metavar="DIR", help="after building, download 600 px thumbnails of index entries into DIR")
    p.add_argument("--thumbs-per-work", type=int, default=4, help="thumbnails per work for --fetch-thumbs (default: %(default)s)")
    p.add_argument("--thumbs-works", help="comma-separated work keys to take thumbnails from (default: all)")
    args = p.parse_args(argv)
    if args.min_interval < MIN_INTERVAL_FLOOR:
        p.error("--min-interval below %.1f s is refused (Wikimedia etiquette)" % MIN_INTERVAL_FLOOR)
    return args


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    labels = load_labels(args.labels)
    label_lines = sum(1 for line in open(args.labels, encoding="utf-8") if line.strip())
    if args.verify:
        problems = verify(args.out, labels)
        for prob in problems:
            print("FAIL:", prob)
        print("verify: %d problem(s) in %s" % (len(problems), args.out))
        return 1 if problems else 0

    http = Http(args.min_interval, args.max_requests, args.offline)
    api = Api(http, args.cache_dir, args.refresh)

    try:
        if args.discover:
            for prefix in DISCOVER_PREFIXES:
                # allcategories is a list module, not a generator: read the raw response
                resp = api.get({"list": "allcategories", "acprefix": prefix, "aclimit": "500", "acprop": "size"})
                rows = resp.get("query", {}).get("allcategories", [])
                print("prefix %r: %d categories%s" % (prefix, len(rows), " (more exist)" if resp.get("continue") else ""))
                for row in rows:
                    print("  Category:%s  files=%s subcats=%s" % (row.get("category"), row.get("files"), row.get("subcats")))
            return 0

        keys = [k.strip() for k in args.works.split(",")] if args.works else list(WORKS)
        for k in keys:
            if k not in WORKS:
                log("unknown work key: " + k)
                return 2
        log("resolving categories")
        roots, _ = resolve_roots(api, keys)
        found = [k for k in keys if roots[k]]
        for k in keys:
            if not roots[k]:
                log("no category found for %s (tried %s)" % (k, ", ".join(WORKS[k]["candidates"])))

        results = {}
        incomplete = []
        for k in found:
            log("crawling %s" % k)
            try:
                files, tree = crawl(api, roots[k], args.max_depth, args.max_categories)
            except Stop as why:
                incomplete.append((k, str(why)))
                log("STOP at %s: %s" % (k, why))
                break
            stats, lic_reject, cands, lic_only, unlabelled, detail = classify(files, labels, args.min_width)
            results[k] = {
                "stats": stats, "lic_reject": lic_reject, "candidates": cands,
                "licence_only": lic_only, "unlabelled": unlabelled, "detail": detail, "tree": tree,
            }
        done = [k for k in found if k in results]
        for k in found:
            if k not in results and not any(k == w for w, _ in incomplete):
                incomplete.append((k, "not reached (an earlier work stopped the run)"))

        if incomplete and not args.partial:
            log("incomplete: %s" % ", ".join("%s (%s)" % kv for kv in incomplete))
            log("not writing the index; rerun to resume from the cache, or pass --partial")
            return 75

        best, _ = choose({k: results[k]["candidates"] for k in done})
        notes = {}
        for k in done:
            res = results[k]
            others = [r["title"] for r in roots[k][1:]]
            n_cats = len(res["tree"])
            note = "%d files seen in %d %s; %d files accepted as single-species public-domain plates of at least %d px" % (
                res["stats"]["seen"], n_cats, "category" if n_cats == 1 else "categories",
                res["stats"]["accepted"], args.min_width)
            if others:
                note += "; also crawled " + ", ".join(others)
            notes[k] = {"root": roots[k][0]["title"], "note": note}
        generated = args.generated or datetime.datetime.now(datetime.timezone.utc).date().isoformat()
        index = build_index(best, done, notes, generated)
        write_atomic(args.out, dump_json(index))
        ctx = {
            "labels": labels, "label_lines": label_lines, "best": best, "found": found,
            "results": results, "roots": roots, "incomplete": incomplete, "generated": generated,
        }
        write_atomic(args.report, build_report(ctx))
        sha = hashlib.sha256(pathlib.Path(args.out).read_bytes()).hexdigest()
        print("wrote %s: %d species from %d works; sha256 %s" % (args.out, len(best), len(done), sha))
        print("network requests: %d (throttled %d), cache hits: %d" % (http.network_requests, http.throttled, api.cache_hits))
        problems = verify(args.out, labels)
        for prob in problems:
            print("FAIL:", prob)
        if args.fetch_thumbs:
            for work, species, name, path in fetch_thumbs(
                http, index, args.fetch_thumbs, args.thumbs_per_work,
                set(args.thumbs_works.split(",")) if args.thumbs_works else None,
            ):
                print("thumb %s | %s | %s | %s" % (work, species, name, path))
        return 1 if problems else (75 if incomplete else 0)
    except Stop as why:
        log("STOP: %s" % why)
        return 75
    except ApiError as why:
        log("API ERROR: %s" % why)
        return 1


if __name__ == "__main__":
    sys.exit(main())
