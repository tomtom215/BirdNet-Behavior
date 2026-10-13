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

One external link *is* checked, offline: each page's "Suggest an edit" link
(`rel="edit"`) must name a file that exists in this repository. mdBook 0.5
expands `{path}` in `edit-url-template` relative to the book root (the
repository root here), not to `src`, so a template written for 0.4 sent every
edit link to `docs/book/docs/book/…` — a 404 on every page of the published
site, invisible to a check that skips `https:` links.

Usage:  scripts/check-book-links.py [--base /served/prefix/] <rendered-html-dir> [more dirs...]

`--base` is the URL path the tree is served under (mdBook's `site-url`). A
root-absolute link under that prefix resolves inside the tree; one outside it
cannot be checked offline and is reported. Without `--base`, `/` is the tree.
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

# The repository-relative path in a GitHub "edit" URL: `…/edit/<ref>/<path>`.
EDIT_URL = re.compile(r"^https://github\.com/[^/]+/[^/]+/edit/[^/]+/(.+)$")
EDIT_LINK = re.compile(r"""<a\b[^>]*\brel\s*=\s*["']edit["'][^>]*>""", re.I)
EDIT_HREF = re.compile(r"""\bhref\s*=\s*(?:"([^"]*)"|'([^']*)')""", re.I)
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

SKIP_SCHEMES = {"http", "https", "mailto", "tel", "data", "javascript", "ftp"}


def anchors_in(path: str) -> set[str]:
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            body = fh.read()
    except OSError:
        return set()
    return {html.unescape(a or b) for a, b in ANCHOR.findall(body)}


def check(root: str, base: str = "/") -> list[str]:
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
        for tag in EDIT_LINK.findall(body):
            href = EDIT_HREF.search(tag)
            raw = html.unescape((href.group(1) or href.group(2)) if href else "")
            m = EDIT_URL.match(raw)
            if not m or not os.path.isfile(os.path.join(REPO_ROOT, unquote(m.group(1)))):
                problems.append(
                    f"{os.path.relpath(page, root)}: edit link {raw!r} — "
                    "names no file in this repository"
                )
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

            if target.startswith("/"):
                if not target.startswith(base) and target + "/" != base:
                    problems.append(f"{rel}: {raw} — outside the served prefix {base}")
                    continue
                dest = os.path.join(root, unquote(target[len(base):]).lstrip("/"))
            else:
                dest = os.path.join(here, unquote(target))
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
    base = "/"
    if roots[:1] == ["--base"] and len(roots) >= 2:
        base = roots[1] if roots[1].endswith("/") else roots[1] + "/"
        roots = roots[2:]
    if not roots:
        print(__doc__)
        return 2
    failed = False
    for root in roots:
        # Deduped before counting, so the number in the header is the number of
        # lines printed under it. Counting occurrences instead read as a silent
        # truncation ("10 broken links" over a list of 8) when the only
        # difference was the same dead target linked twice from one page.
        problems = sorted(set(check(root, base)))
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
