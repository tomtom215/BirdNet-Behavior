// Chart labels stay legible however far the chart is scaled.
//
// The charts are hand-built SVGs drawn on a fixed canvas (720 wide, 340 wide, …)
// and scaled to their card with `width: 100%`. Text inside is scaled with them,
// so a label authored at 8 px is 8 px only on a canvas-sized card. Measured on
// a 390 px phone: the dashboard's sunrise/sunset labels rendered at 2.8 px, the
// Patterns axis at 3.6 px, the migration months at 2.7 px and the species-page
// hours at 3.9 px — drawn, in the sense that pixels were lit, and unreadable.
//
// So after an SVG is laid out, any <text> whose on-screen size is under FLOOR
// is enlarged to FLOOR (never by more than MAX_GROW, which is the scale at
// which a chart has stopped being a chart); and where that makes labels collide,
// the later and smaller of two overlapping labels is hidden — a legible axis
// with fewer ticks over an illegible one with all of them. An SVG that needed no
// enlarging is left exactly as drawn, so wide screens are untouched.
//
// Re-run when the page loads, when htmx swaps a fragment in, when a <details>
// opens (it measured 0 while closed), and when the SVG's own size changes.
// Opt out with `data-no-text-fit` on the <svg>.
(function () {
  "use strict";

  var FLOOR = 9.5; // CSS px, on screen
  var MAX_GROW = 4;

  function scaleOf(svg) {
    var m = svg.getScreenCTM && svg.getScreenCTM();
    if (!m) return 0;
    // Uniform in every chart here; the smaller axis is the honest one if not.
    return Math.min(Math.hypot(m.a, m.b), Math.hypot(m.c, m.d));
  }

  // "Common Whitethroat" → "C. Whitethroat". A single word is left alone.
  function abbreviate(text) {
    var words = text.trim().split(/\s+/);
    if (words.length < 2) return text;
    return (
      words
        .slice(0, -1)
        .map(function (w) {
          return w.charAt(0) + ".";
        })
        .join(" ") +
      " " +
      words[words.length - 1]
    );
  }

  function overlaps(a, b) {
    var pad = 1;
    return !(
      a.right + pad <= b.left ||
      b.right + pad <= a.left ||
      a.bottom + pad <= b.top ||
      b.bottom + pad <= a.top
    );
  }

  function fit(svg) {
    if (!svg.isConnected || svg.hasAttribute("data-no-text-fit")) return;
    var scale = scaleOf(svg);
    if (!(scale > 0)) return; // not laid out (display: none, closed <details>)
    var texts = svg.querySelectorAll("text");
    if (!texts.length) return;

    var grew = false;
    var items = [];
    for (var i = 0; i < texts.length; i++) {
      var t = texts[i];
      if (!t.__bnbFs) {
        // Authored size and wording, read once with nothing of ours applied, so
        // a resize recomputes from the drawing rather than compounding.
        t.style.fontSize = "";
        t.__bnbFs = parseFloat(getComputedStyle(t).fontSize) || 10;
        t.__bnbText = t.children.length === 0 ? t.textContent : null;
      }
      if (t.__bnbText !== null && t.textContent !== t.__bnbText) t.textContent = t.__bnbText;
      var authored = t.__bnbFs;
      var want = authored;
      if (authored * scale < FLOOR) {
        want = Math.min(FLOOR / scale, authored * MAX_GROW);
        grew = true;
      }
      t.style.fontSize = want === authored ? "" : want + "px";
      t.style.visibility = "";
      items.push({ el: t, authored: authored, order: i });
    }
    // Enlarged text can reach past the drawing's own edge; let it, rather than
    // clip it — the cards around a chart have padding to spare.
    svg.style.overflow = grew ? "visible" : "";
    if (!grew) return;

    // A label that now runs out of the SVG sideways is shortened to fit: first
    // the way a birder would ("Common Swift" → "C. Swift", which keeps the
    // names that share a first word apart), then cut with an ellipsis.
    var frame = svg.getBoundingClientRect();
    var outside = function (b) {
      return b.left < frame.left - 1 || b.right > frame.right + 1;
    };
    for (var c = 0; c < items.length; c++) {
      var label = items[c].el;
      var base = label.__bnbText;
      if (base === null || label.__bnbFs * scale >= FLOOR) continue; // not grown
      if (!outside(label.getBoundingClientRect())) continue;
      var short = abbreviate(base);
      if (short !== base) {
        label.textContent = short;
        if (!outside(label.getBoundingClientRect())) continue;
      }
      var cut = short.length;
      var guard = 0;
      while (outside(label.getBoundingClientRect()) && cut > 3 && guard++ < 60) {
        cut--;
        label.textContent = short.slice(0, cut).trimEnd() + "\u2026";
      }
    }

    // Larger authored size first (titles, end values), then document order.
    items.sort(function (a, b) {
      return b.authored - a.authored || a.order - b.order;
    });
    var kept = [];
    for (var j = 0; j < items.length; j++) {
      var el = items[j].el;
      var rect = el.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) continue;
      var clash = false;
      for (var k = 0; k < kept.length; k++) {
        if (overlaps(rect, kept[k])) {
          clash = true;
          break;
        }
      }
      if (clash) el.style.visibility = "hidden";
      else kept.push(rect);
    }
  }

  var observer = typeof ResizeObserver === "function" ? new ResizeObserver(onResize) : null;
  var queued = false;
  var pending = new Set();

  function onResize(entries) {
    entries.forEach(function (e) {
      pending.add(e.target);
    });
    schedule();
  }

  function schedule() {
    if (queued) return;
    queued = true;
    requestAnimationFrame(function () {
      queued = false;
      pending.forEach(fit);
      pending.clear();
    });
  }

  function scan(root) {
    var svgs = (root || document).querySelectorAll("svg");
    for (var i = 0; i < svgs.length; i++) {
      var svg = svgs[i];
      if (!svg.querySelector("text") || svg.hasAttribute("data-no-text-fit")) continue;
      if (observer && !svg.__bnbObserved) {
        svg.__bnbObserved = true;
        observer.observe(svg); // fires once now, and on every later size change
      } else {
        pending.add(svg);
      }
    }
    schedule();
  }

  document.addEventListener("htmx:afterSettle", function (e) {
    scan(e.target && e.target.querySelectorAll ? e.target : document);
  });
  // `toggle` does not bubble; capture it so a <details> that was closed when
  // the chart first measured gets fitted when it opens.
  document.addEventListener(
    "toggle",
    function (e) {
      if (e.target && e.target.open) scan(e.target);
    },
    true
  );
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      scan();
    });
  } else {
    scan();
  }
})();
