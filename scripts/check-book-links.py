#!/usr/bin/env python3
"""Fail on a broken internal link in the rendered operator manual.

Why this exists
---------------
`mdbook-linkcheck` was the backend that did this, and it cannot come with us:
it is an mdBook 0.4-era plugin, last released in 2022, and the book is rendered
by mdBook 0.5 (`mdbook-driver 0.5.4`, which `build.rs` runs on every `cargo
build` to produce the in-app `/help/*` tree).

Until this, the published site and the in-app copy were rendered by two
different mdBook majors and only the published one was link-checked. This runs
against the *rendered HTML*, so it is renderer-agnostic and covers whichever
tree it is pointed at — which means the same check now applies to both.

External links are deliberately not followed. They rot and rate-limit, and a
docs build that fails because someone else's server is down blocks releases for
no reason.

Usage:  scripts/check-book-links.py <rendered-html-dir> [more dirs...]
"""

from __future__ import annotations

import html
import os
import re
import sys
from urllib.parse import unquote, urlparse

# `href="…"` and `src="…"`, single or double quoted.
ATTR = re.compile(r"""\b(?:href|src)\s*=\s*(?:"([^"]*)"|'([^']*)')""", re.I)
# Code samples. mdBook escapes `<` and `>` inside them but leaves quotes alone,
# so `href="/feeds/rare.rss"` in a documented `<link>` snippet reads as a real
# link unless these are stripped first. It did, on the first run of this script:
# a documentation example is not a navigable link and must not be checked as one.
CODE = re.compile(r"<pre\b.*?</pre>|<code\b.*?</code>", re.I | re.S)
# `id="…"` / `name="…"`, for fragment targets.
ANCHOR = re.compile(r"""\b(?:id|name)\s*=\s*(?:"([^"]*)"|'([^']*)')""", re.I)

SKIP_SCHEMES = {"http", "https", "mailto", "tel", "data", "javascript", "ftp"}


def anchors_in(path: str) -> set[str]:
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            body = fh.read()
    except OSError:
        return set()
    return {html.unescape(a or b) for a, b in ANCHOR.findall(body)}


def check(root: str) -> list[str]:
    problems: list[str] = []
    anchor_cache: dict[str, set[str]] = {}
    pages = [
        os.path.join(dp, fn)
        for dp, _, fns in os.walk(root)
        for fn in fns
        if fn.endswith(".html")
    ]
    if not pages:
        return [f"{root}: no rendered HTML found — did the book build?"]

    for page in pages:
        with open(page, encoding="utf-8", errors="replace") as fh:
            body = CODE.sub("", fh.read())
        here = os.path.dirname(page)
        for a, b in ATTR.findall(body):
            raw = html.unescape(a or b).strip()
            if not raw:
                continue
            parsed = urlparse(raw)
            if parsed.scheme.lower() in SKIP_SCHEMES or raw.startswith("//"):
                continue
            target, frag = parsed.path, parsed.fragment
            rel = os.path.relpath(page, root)

            if not target:
                # Same-page fragment.
                if frag:
                    anchors = anchor_cache.setdefault(page, anchors_in(page))
                    if frag not in anchors:
                        problems.append(f"{rel}: #{frag} — no such anchor on this page")
                continue

            dest = (
                os.path.join(root, unquote(target).lstrip("/"))
                if target.startswith("/")
                else os.path.join(here, unquote(target))
            )
            dest = os.path.normpath(dest)
            if os.path.isdir(dest):
                dest = os.path.join(dest, "index.html")
            if not os.path.exists(dest):
                problems.append(f"{rel}: {raw} — target does not exist")
                continue
            if frag and dest.endswith(".html"):
                anchors = anchor_cache.setdefault(dest, anchors_in(dest))
                if frag not in anchors:
                    problems.append(
                        f"{rel}: {raw} — page exists, but it has no #{frag}"
                    )
    return problems


def main(argv: list[str]) -> int:
    roots = argv[1:]
    if not roots:
        print(__doc__)
        return 2
    failed = False
    for root in roots:
        # Deduped before counting, so the number in the header is the number of
        # lines printed under it. Counting occurrences instead read as a silent
        # truncation ("10 broken links" over a list of 8) when the only
        # difference was the same dead target linked twice from one page.
        problems = sorted(set(check(root)))
        if problems:
            failed = True
            print(f"check-book-links: {len(problems)} broken link(s) under {root}:")
            for p in problems:
                print(f"    {p}")
        else:
            print(f"check-book-links: OK — every internal link under {root} resolves")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
