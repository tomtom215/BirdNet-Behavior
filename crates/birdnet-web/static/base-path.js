// Keeps URLs built by scripts inside a reverse-proxy prefix (M8).
//
// The server prefixes the URLs in the HTML it sends and in its redirect
// headers. A URL a script builds in the browser — `htmx.ajax('GET',
// '/pages/…')`, `location.href = '/recordings…'`, `fetch('/…')` — never passes
// through that rewrite, so under `BIRDNET_BASE_PATH` each one left the
// application. The prefix is published on `<body data-base-path>`.
//
// * Every htmx request goes through `htmx:configRequest`, which prefixes an
//   application-absolute path: one listener covers every `htmx.ajax` call.
// * Anything else calls `bnbUrl('/path')`.
//
// Idempotent: a path already under the prefix (the server rewrote it) and a
// protocol-relative or relative one are returned unchanged.
(function () {
  "use strict";
  function base() {
    return (document.body && document.body.dataset.basePath) || "";
  }
  function url(p) {
    var b = base();
    if (!b || typeof p !== "string" || p.charAt(0) !== "/" || p.charAt(1) === "/") return p;
    if (p === b || p.indexOf(b + "/") === 0 || p.indexOf(b + "?") === 0) return p;
    return b + p;
  }
  window.bnbUrl = url;
  // `raw` as an absolute URL on this origin, or null. A value a script reads
  // back from the DOM (`data-play-src`, `data-href`, `data-help-drawer`) is a
  // URL to follow, not one to trust: a `javascript:` URL and another origin
  // are refused here, and what comes back is the parsed `href`, not the string
  // that was read. Every legitimate value is an application path, which
  // resolves against the page and so keeps a reverse-proxy prefix the server
  // put on it.
  window.bnbSameOrigin = function (raw) {
    if (typeof raw !== "string" || raw === "") return null;
    try {
      var u = new URL(raw, window.location.href);
      return u.origin === window.location.origin ? u.href : null;
    } catch (e) {
      return null;
    }
  };
  document.addEventListener("htmx:configRequest", function (e) {
    if (e.detail && typeof e.detail.path === "string") e.detail.path = url(e.detail.path);
  });
})();
