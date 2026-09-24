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
  document.addEventListener("htmx:configRequest", function (e) {
    if (e.detail && typeof e.detail.path === "string") e.detail.path = url(e.detail.path);
  });
})();
