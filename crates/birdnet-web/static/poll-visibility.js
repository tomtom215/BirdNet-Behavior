// Polling pauses while the tab is hidden (M13).
//
// Every `hx-trigger="every …"` element kept polling in a background tab: a
// Today page left open all day is a dozen database reads a minute that nobody
// sees, on a Raspberry Pi that is also doing inference. htmx's own trigger
// filter (`every 15s [!document.hidden]`) would say this in the attribute, but
// filters are evaluated with `eval`, which the Content-Security-Policy refuses.
//
// So: a poll that fires while the document is hidden is cancelled here, and
// each element that missed one is refreshed once when the tab is shown again.
// A poll is a request from an element whose trigger has `every`, with no
// triggering event — a click, a submit or a custom event always carries one,
// and those are never held back.
(function () {
  "use strict";
  if (!window.htmx) return;
  var missed = [];

  function polls(elt) {
    var t = elt && elt.getAttribute && elt.getAttribute("hx-trigger");
    return !!t && /(^|,)\s*every\s/.test(t);
  }

  document.addEventListener("htmx:beforeRequest", function (e) {
    if (!document.hidden) return;
    var d = e.detail || {};
    if (!polls(d.elt)) return;
    if (d.requestConfig && d.requestConfig.triggeringEvent) return;
    e.preventDefault();
    if (missed.indexOf(d.elt) < 0) missed.push(d.elt);
  });

  document.addEventListener("visibilitychange", function () {
    if (document.hidden || !missed.length) return;
    var els = missed;
    missed = [];
    els.forEach(function (elt) {
      var url = elt.getAttribute("hx-get");
      if (!url || !document.contains(elt)) return;
      // The element as source keeps its own target, swap and includes.
      window.htmx.ajax("GET", url, { source: elt, event: new Event("bnb:visible") });
    });
  });
})();
