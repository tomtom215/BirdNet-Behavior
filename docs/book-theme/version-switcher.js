// Version switcher for the published manual.
//
// Loaded only by `scripts/build-versioned-docs.sh`, which injects it (and the
// `version-id.js` that sets `window.bnbDocs`) through mdBook's environment
// config. `book.toml` does not name it, so the in-app `/help/*` tree — which
// is always the manual for the binary serving it — never shows a switcher.
//
// It reads `versions.json` from the site root. If that fails (a local
// `mdbook serve`, a file:// copy), it does nothing: the page is still the
// whole manual for its version, which is all a reader there needs.
(function () {
    "use strict";
    var id = window.bnbDocs;
    if (!id || typeof path_to_root === "undefined") return;

    var bookRoot = new URL(path_to_root, window.location.href);
    var siteRoot = id.depth === 1 ? new URL("../", bookRoot) : bookRoot;
    var page = window.location.pathname.slice(bookRoot.pathname.length) || "index.html";

    function el(tag, attrs, text) {
        var node = document.createElement(tag);
        Object.keys(attrs || {}).forEach(function (k) { node.setAttribute(k, attrs[k]); });
        if (text) node.textContent = text;
        return node;
    }

    // Go to the same page in another version if it exists there, else to
    // that version's front page: a page added or renamed between releases
    // should not land the reader on a 404.
    function go(path) {
        var base = new URL(path, siteRoot);
        var target = new URL(page, base);
        fetch(target, { method: "HEAD" })
            .then(function (r) { window.location.href = (r.ok ? target : base).href; })
            .catch(function () { window.location.href = base.href; });
    }

    function banner(versions) {
        var latest = versions.latest;
        if (id.version === latest && id.depth === 0) return;
        var note = el("div", { "class": "bnb-version-banner", role: "note" });
        var link = el("a", { href: new URL(page, siteRoot).href });
        if (id.version === "dev") {
            note.appendChild(document.createTextNode(
                "This is the development manual: it describes unreleased changes on main. "));
            link.textContent = "Read the manual for " + latest + ", the latest release";
        } else if (id.version === latest) {
            note.appendChild(document.createTextNode(
                "This address is pinned to " + latest + ", currently the latest release. "));
            link.textContent = "The unversioned address follows each new release";
        } else {
            note.appendChild(document.createTextNode(
                "This manual is for " + id.version + ". The latest release is " + latest + ". "));
            link.textContent = "Read the manual for " + latest;
        }
        link.textContent += ".";
        note.appendChild(link);
        var main = document.querySelector("main");
        if (main) main.insertBefore(note, main.firstChild);
    }

    function selector(versions) {
        var host = document.querySelector(".right-buttons");
        if (!host) return;
        var select = el("select", {
            "class": "bnb-version-select",
            "aria-label": "Manual version",
            title: "Manual version",
        });
        var current = id.depth === 0 ? "" : id.version;
        var latestOpt = el("option", { value: "" }, "latest (" + versions.latest + ")");
        select.appendChild(latestOpt);
        versions.versions.forEach(function (v) {
            var opt = el("option", { value: v.path }, v.name === "dev" ? "dev (unreleased)" : v.name);
            if (v.name === current) opt.selected = true;
            select.appendChild(opt);
        });
        if (current === "") latestOpt.selected = true;
        select.addEventListener("change", function () { go(select.value || "./"); });
        host.insertBefore(select, host.firstChild);
    }

    fetch(new URL("versions.json", siteRoot))
        .then(function (r) { return r.ok ? r.json() : null; })
        .then(function (versions) {
            if (!versions || !Array.isArray(versions.versions)) return;
            selector(versions);
            banner(versions);
        })
        .catch(function () { /* no manifest: a lone build, nothing to switch to */ });
})();
