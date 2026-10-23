// The chart kit: what every chart here shares once it has been drawn.
//
// The charts are server-rendered inline SVG, complete and readable with no script
// at all (every one also has a table behind a disclosure). This file only adds
// what a static picture cannot do: say what is under the pointer, the finger or
// the keyboard; put one series in focus and the rest in the background; and
// arrive instead of appearing. No library, no network, plain ES2020.
//
// ── The contract (the Rust renderers emit it; docs/design/charts.md explains it) ──
//
// A chart is   <div class="ck" …> <svg data-ck="cols|marks" …> … </svg> [legend] [readout] </div>
//
//   .ck                       the positioning box the tooltip lives in. The kit
//                             appends `.ck-tip` and `.ck-live` to it; neither takes
//                             any room, so a chart never moves when it is read.
//   svg[data-ck]              turns the kit on for that svg.
//     data-ck-scroll-end      on a phone a wide chart scrolls sideways inside its card; this
//                             starts it scrolled to the right, where the newest day is
//     data-ck-id="name"       identity for the entrance animation: a chart that is
//                             swapped in again under the same id (a 30-second
//                             refresh) does not play it again. Omit for "always".
//     data-ck-label="…"       what the keys do, written for a person ("Left and
//                             right read each day"); becomes aria-description.
//     data-ck-dim="series|index"   what falls into the background when one thing is
//                             in focus (see below); omit to dim nothing.
//
//   MODE "cols": one reading per column of the plot (a day, an hour), found from the
//   pointer's x and shown with a crosshair. For streams, areas, lines, bars by date.
//     data-ck-cols='[["Tue 14 Oct",[8,5,3],"note"], …]'   heading, one value per
//                             series (null = no data, shown as such), optional note
//     data-ck-series='[["American Robin","var(--chart-1)"], …]'   name, key colour
//     data-ck-x0 / data-ck-x1   plot's left and right edge, in viewBox units
//     data-ck-pos="point|band"  columns sit on the edges (point: first at x0, last at
//                             x1) or in equal bands (band: centred in each)
//     data-ck-total="Total"   label for a sum row (omit for a single series)
//     data-ck-unit="detections"   appended to a one-series value
//     data-ck-ys='[y0, y1, …]'  viewBox y of the first series per column: places `.ck-dot`
//     <line class="ck-cursor"> the crosshair, positioned by the kit (hidden until used)
//     <circle class="ck-dot">  the marker on the line, positioned by the kit
//     [data-ck-s="k"]         a mark belonging to series k: it is what the pointer can
//                             land on to focus the series, and what dims.
//
//   MODE "marks": each mark carries its own reading, no crosshair. For cells and bars.
//     [data-tip="Heading\nLabel\tValue\tcolour\nA note"]   first line is the heading,
//                             lines with a tab are rows (label, value, optional key
//                             colour), lines without are a note
//     [data-ck-i="n"]         the mark's index; every element sharing an index lights
//                             together (a bar and the invisible hit-box over its slot)
//     data-ck-grid="24"       the length of a line of marks, so the other pair of arrows
//                             moves by line (Left/Right step one mark, Up/Down a line)
//     data-ck-major="col"     the lines run down (a calendar's weeks): Left/Right
//                             move by line, Up/Down by one
//     data-ck-start="n"       the index a keyboard lands on first (the peak)
//     data-ck-s="3 5"         (charts2) a mark may belong to several things, as space-
//                             separated tokens: a ribbon between species 3 and 5.
//     data-ck-f="3 5"         (charts2) on a mark with a reading: while it is read, only
//                             marks whose data-ck-s holds all of these tokens stay in the
//                             foreground (data-ck-dim must name "series"). A species' arc
//                             says its own number, and lists its partners' in its data-ck-s,
//                             so its ribbons and partners stay; a ribbon says both of its
//                             species, so only it and those two stay.
//
//   Legend, inside .ck:  <button class="ck-leg" data-ck-s="k" aria-pressed="false">
//                             hover or focus previews that series; a click pins it.
//
//   Readout: a `.viz-readout` inside .ck, if present, repeats the reading as a line
//   of text under the chart (what a screen magnifier and a phone want).
//
// ── What it does ──
//   pointer / touch / keyboard   the same reading, three ways in. One tab stop per
//                                chart; Left/Right/Home/End (and Up/Down by row, or by
//                                series in "cols") move; Escape lets go. Every move is
//                                announced in an aria-live region.
//   focus and dim                one series (or index) at full strength, the rest at
//                                --chart-dim. Hover on a mark, hover/focus/click on its
//                                legend item, or the Up/Down keys.
//   entrance                     CSS only (the .ck-in classes in app.css), so it is
//                                gated by prefers-reduced-motion and the operator's
//                                Motion setting like every other animation. The kit
//                                only decides *not* to repeat it (`.ck-still`).
//   after htmx swaps             scanned on htmx:afterSwap, which runs before paint.
//   resize                       a ResizeObserver drops the tooltip rather than leave
//                                it pointing at where a mark used to be.
//   theme                        nothing to do: every colour is a custom property.
//
// Labels and values are untrusted data (species names): textContent only, never
// innerHTML. A tooltip enhances and never gates: every value in one is also in the
// table under the chart.
(function () {
  "use strict";

  var SVGNS = "http://www.w3.org/2000/svg";
  var seen = Object.create(null); // data-ck-id -> true, so a refresh does not re-animate
  var uid = 0;
  var fmt = typeof Intl !== "undefined" ? new Intl.NumberFormat("en-US") : null;

  function num(v) {
    if (v === null || v === undefined) return "no data";
    if (typeof v !== "number") return String(v);
    if (!isFinite(v)) return "";
    return fmt ? fmt.format(Math.round(v * 100) / 100) : String(v);
  }

  function json(el, name) {
    var raw = el.getAttribute(name);
    if (!raw) return null;
    try {
      return JSON.parse(raw);
    } catch (e) {
      return null;
    }
  }

  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text !== undefined) n.textContent = text;
    return n;
  }

  // ── One chart ──────────────────────────────────────────────────────────────
  function bind(svg) {
    if (svg.__ck) return;
    svg.__ck = true;

    var id = svg.getAttribute("data-ck-id");
    if (id) {
      if (seen[id]) svg.classList.add("ck-still");
      else seen[id] = true;
    }

    // A time series that scrolls sideways starts at its newest end: that is the part a reader came for.
    if (svg.hasAttribute("data-ck-scroll-end")) {
      var scroller = svg.closest(".viz-scroll, .hm-scroll");
      if (scroller) {
        var toEnd = function () {
          scroller.scrollLeft = scroller.scrollWidth;
        };
        requestAnimationFrame(toEnd);
        // Fonts and the chart-text fitter settle a frame or two later and can widen the box.
        setTimeout(toEnd, 250);
      }
    }

    var mode = svg.getAttribute("data-ck") === "cols" ? "cols" : "marks";
    var wrap = svg.closest(".ck");
    if (!wrap) {
      wrap = svg.parentElement;
      wrap.classList.add("ck");
    }
    var dimMode = svg.getAttribute("data-ck-dim") || "";

    var tip = wrap.querySelector(":scope > .ck-tip");
    if (!tip) {
      tip = el("div", "ck-tip");
      tip.setAttribute("role", "presentation");
      wrap.appendChild(tip);
    }
    var live = wrap.querySelector(":scope > .ck-live");
    if (!live) {
      live = el("div", "ck-live sr-only");
      live.setAttribute("aria-live", "polite");
      live.setAttribute("aria-atomic", "true");
      wrap.appendChild(live);
    }
    var readout = wrap.querySelector(".viz-readout");
    var idleReadout = readout ? readout.textContent : "";
    var cursor = svg.querySelector(".ck-cursor");
    var dot = svg.querySelector(".ck-dot");

    // The chart is reachable by keyboard once there is something to say with it.
    if (!svg.hasAttribute("tabindex")) svg.setAttribute("tabindex", "0");
    var label = svg.getAttribute("data-ck-label");
    if (label) svg.setAttribute("aria-description", label);

    // ── data ──
    var cols = mode === "cols" ? json(svg, "data-ck-cols") || [] : [];
    var series = json(svg, "data-ck-series") || [];
    var ys = json(svg, "data-ck-ys");
    var x0 = Number(svg.getAttribute("data-ck-x0")) || 0;
    var x1 = Number(svg.getAttribute("data-ck-x1")) || 1;
    var band = svg.getAttribute("data-ck-pos") === "band";
    var totalLabel = svg.getAttribute("data-ck-total");
    var unit = svg.getAttribute("data-ck-unit") || "";
    var grid = Number(svg.getAttribute("data-ck-grid")) || 0;

    // The mark indices, for "marks" mode, in document order.
    var stops = [];
    if (mode === "marks") {
      var seenIdx = Object.create(null);
      svg.querySelectorAll("[data-tip]").forEach(function (m) {
        var k = m.getAttribute("data-ck-i");
        if (k === null) {
          k = String(stops.length);
          m.setAttribute("data-ck-i", k);
        }
        if (!seenIdx[k]) {
          seenIdx[k] = m;
          stops.push(k);
        }
      });
      // In order of index, so "the next one" means the next in the picture even when
      // the marks were written out in another order (and some indices are absent).
      stops.sort(function (a, b) {
        return Number(a) - Number(b);
      });
    }
    var major = svg.getAttribute("data-ck-major") === "col";
    var startAt = svg.getAttribute("data-ck-start");

    var cur = -1; // current column (cols) or position in `stops` (marks)
    var focusSeries = -1; // series currently in the foreground, or -1
    var pinned = -1; // legend click
    var rects = null; // cached boxes of marks, "marks" mode

    function colX(i) {
      var n = cols.length;
      if (band) return x0 + ((i + 0.5) / n) * (x1 - x0);
      return n > 1 ? x0 + (i / (n - 1)) * (x1 - x0) : (x0 + x1) / 2;
    }

    function colAt(clientX) {
      var m = svg.getScreenCTM();
      if (!m || !cols.length) return -1;
      var p = svg.createSVGPoint();
      p.x = clientX;
      p.y = 0;
      var u = p.matrixTransform(m.inverse()).x;
      var n = cols.length;
      var i = band
        ? Math.floor(((u - x0) / (x1 - x0 || 1)) * n)
        : Math.round(((u - x0) / (x1 - x0 || 1)) * (n - 1));
      return Math.max(0, Math.min(n - 1, i));
    }

    // ── what to say ──
    function readingOfCol(i) {
      var c = cols[i];
      var rows = [];
      var vals = c[1] || [];
      var total = 0;
      var anyNull = false;
      for (var k = 0; k < vals.length; k++) {
        if (vals[k] === null) anyNull = true;
        else total += vals[k];
      }
      if (vals.length === 1) {
        rows.push({ l: "", v: num(vals[0]) + (unit && vals[0] !== null ? " " + unit : ""), key: (series[0] || [])[1] });
      } else {
        var list = [];
        for (var j = 0; j < vals.length; j++) {
          if (vals[j] === null || vals[j] > 0) list.push(j);
        }
        list.sort(function (a, b) {
          return (vals[b] || 0) - (vals[a] || 0);
        });
        // The focused series leads, whatever its rank, so it is the first thing read.
        if (focusSeries >= 0) {
          var at = list.indexOf(focusSeries);
          if (at > 0) {
            list.splice(at, 1);
            list.unshift(focusSeries);
          } else if (at < 0 && focusSeries < vals.length) {
            list.unshift(focusSeries);
          }
        }
        var cap = 7;
        list.slice(0, cap).forEach(function (j) {
          rows.push({
            l: (series[j] || [])[0] || "",
            v: num(vals[j]),
            key: (series[j] || [])[1],
            strong: j === focusSeries,
          });
        });
        if (list.length > cap) rows.push({ l: "and " + (list.length - cap) + " more", v: "", key: null });
        if (totalLabel && !anyNull) rows.push({ l: totalLabel, v: num(total), key: null, total: true });
      }
      return { h: c[0], rows: rows, note: c[2] || "" };
    }

    function readingOfMark(k) {
      var m = svg.querySelector('[data-tip][data-ck-i="' + k + '"]');
      if (!m) return null;
      var lines = (m.getAttribute("data-tip") || "").split("\n");
      var rows = [];
      var notes = [];
      lines.slice(1).forEach(function (ln) {
        var f = ln.split("\t");
        if (f.length >= 2) rows.push({ l: f[0], v: f[1], key: f[2] || null });
        else if (ln) notes.push(ln);
      });
      return { h: lines[0], rows: rows, note: notes.join(" ") };
    }

    function paint(r) {
      while (tip.firstChild) tip.removeChild(tip.firstChild);
      tip.appendChild(el("div", "ck-tip-h", r.h));
      r.rows.forEach(function (row) {
        var line = el("div", "ck-tip-r" + (row.strong ? " is-strong" : "") + (row.total ? " is-total" : ""));
        var key = el("i", "ck-key");
        if (row.key) key.style.setProperty("background", row.key);
        else key.className = "ck-key is-none";
        line.appendChild(key);
        line.appendChild(el("span", "ck-tip-l", row.l));
        line.appendChild(el("b", "ck-tip-v", row.v));
        tip.appendChild(line);
      });
      if (r.note) tip.appendChild(el("div", "ck-tip-n", r.note));
      var spoken =
        r.h +
        (r.rows.length
          ? ": " +
            r.rows
              .map(function (x) {
                return (x.l ? x.l + " " : "") + x.v;
              })
              .join(", ")
          : "") +
        (r.note ? ". " + r.note : "");
      live.textContent = spoken;
      // The line under the chart is the short form (the day, its first three rows), so reading a
      // day never grows it past the room it keeps: the whole reading is in the card and the live region.
      if (readout) {
        var brief = r.rows.slice(0, 3).map(function (x) {
          return (x.l ? x.l + " " : "") + x.v;
        });
        readout.textContent = r.h + (brief.length ? ": " + brief.join(", ") + (r.rows.length > 3 ? ", \u2026" : "") : "");
      }
    }

    // Where the tooltip points, in client pixels.
    function place(ax, ay) {
      var wr = wrap.getBoundingClientRect();
      tip.classList.add("on");
      var tw = tip.offsetWidth;
      var th = tip.offsetHeight;
      var left = ax - wr.left + 14;
      if (left + tw > wr.width - 2) left = ax - wr.left - tw - 14;
      if (left < 2) left = Math.max(2, Math.min(wr.width - tw - 2, ax - wr.left - tw / 2));
      var top = ay - wr.top - th - 12;
      if (top < 2) top = ay - wr.top + 16;
      if (top + th > wr.height - 2) top = Math.max(2, wr.height - th - 2);
      tip.style.setProperty("left", Math.round(left) + "px");
      tip.style.setProperty("top", Math.round(top) + "px");
    }

    function toClient(ux, uy) {
      var m = svg.getScreenCTM();
      if (!m) return null;
      var p = svg.createSVGPoint();
      p.x = ux;
      p.y = uy;
      var q = p.matrixTransform(m);
      return { x: q.x, y: q.y };
    }

    // ── focus and dim ──
    // charts2: a mark belongs to series "3", or to several: tokens separated by spaces
    // ("3 5"), which is what a ribbon between two species needs. A single number is the
    // old contract, unchanged.
    function hasTok(m, k) {
      return (" " + (m.getAttribute("data-ck-s") || "") + " ").indexOf(" " + k + " ") >= 0;
    }
    function applyFocus(k) {
      if (k === focusSeries && svg.classList.contains("ck-focus") === (k >= 0)) return;
      focusSeries = k;
      svg.classList.toggle("ck-focus", k >= 0 && dimMode.indexOf("series") >= 0);
      wrap.classList.toggle("ck-focused", k >= 0);
      svg.querySelectorAll("[data-ck-s]").forEach(function (m) {
        m.classList.toggle("ck-sel", k >= 0 && hasTok(m, k));
      });
      wrap.querySelectorAll(".ck-leg").forEach(function (b) {
        var on = k >= 0 && Number(b.getAttribute("data-ck-s")) === k;
        b.classList.toggle("ck-sel", on);
      });
    }

    function lightIndex(k) {
      svg.querySelectorAll(".ck-on").forEach(function (m) {
        m.classList.remove("ck-on");
      });
      svg.classList.toggle("ck-focus-i", k !== null && dimMode.indexOf("index") >= 0);
      svg.querySelectorAll("[data-ck-i]").forEach(function (m) {
        m.classList.remove("ck-sel");
      });
      if (k === null) return;
      svg.querySelectorAll('[data-ck-i="' + k + '"]').forEach(function (m) {
        m.classList.add("ck-on");
        m.classList.add("ck-sel");
      });
    }

    // charts2: in "marks" mode a reading can bring a set of marks forward. A mark that
    // carries data-ck-f="3 5" lights every mark whose data-ck-s holds all of those tokens
    // (a species: its ribbons and the species they end on; a ribbon: itself and its two
    // species) and the rest fall to --chart-dim. Only charts that write data-ck-f get it.
    var hasF = mode === "marks" && !!svg.querySelector("[data-ck-f]");
    function applyTokens(f) {
      var want = f ? f.split(" ") : [];
      svg.classList.toggle("ck-focus", want.length > 0 && dimMode.indexOf("series") >= 0);
      wrap.classList.toggle("ck-focused", want.length > 0);
      svg.querySelectorAll("[data-ck-s]").forEach(function (m) {
        m.classList.toggle(
          "ck-sel",
          want.length > 0 &&
            want.every(function (t) {
              return hasTok(m, t);
            })
        );
      });
      wrap.querySelectorAll(".ck-leg").forEach(function (b) {
        b.classList.toggle(
          "ck-sel",
          want.length > 0 &&
            want.every(function (t) {
              return hasTok(b, t);
            })
        );
      });
    }

    // ── show and hide ──
    function showCol(i, ay) {
      if (i < 0 || i >= cols.length) return;
      cur = i;
      var r = readingOfCol(i);
      paint(r);
      var x = colX(i);
      if (cursor) {
        cursor.setAttribute("x1", x);
        cursor.setAttribute("x2", x);
        cursor.setAttribute("visibility", "visible");
      }
      if (dot && ys && ys[i] !== null && ys[i] !== undefined) {
        dot.setAttribute("cx", x);
        dot.setAttribute("cy", ys[i]);
        dot.setAttribute("visibility", "visible");
      }
      var vb = svg.viewBox && svg.viewBox.baseVal;
      // The pointer's own height when there is one; the plot's upper third for the keys.
      var anchor = toClient(x, vb ? vb.y + vb.height * 0.35 : 0);
      if (anchor) place(anchor.x, typeof ay === "number" ? ay : anchor.y);
    }

    function showMark(pos, byPointer) {
      if (pos < 0 || pos >= stops.length) return;
      cur = pos;
      var k = stops[pos];
      var r = readingOfMark(k);
      if (!r) return;
      paint(r);
      lightIndex(k);
      if (hasF) applyTokens(svg.querySelector('[data-tip][data-ck-i="' + k + '"]').getAttribute("data-ck-f"));
      // Point at what is painted (the bar), not at the invisible box that catches the
      // pointer over its slot; a slot with nothing painted gets the box's lower part.
      var shown = svg.querySelector('[data-ck-i="' + k + '"]:not(.ck-hit)');
      var box = svg.querySelector('[data-tip][data-ck-i="' + k + '"]').getBoundingClientRect();
      var b = shown ? shown.getBoundingClientRect() : null;
      if (b && (b.width || b.height)) place(b.left + b.width / 2, b.top);
      else place(box.left + box.width / 2, box.top + box.height * 0.6);
      if (byPointer) tip.classList.add("on");
    }

    function hide() {
      tip.classList.remove("on");
      if (cursor) cursor.setAttribute("visibility", "hidden");
      if (dot) dot.setAttribute("visibility", "hidden");
      if (mode === "marks") lightIndex(null);
      if (hasF) applyTokens(null);
      if (readout) readout.textContent = idleReadout;
      cur = -1;
      if (pinned < 0) applyFocus(-1);
      else applyFocus(pinned);
    }

    // ── pointer ──
    function markUnder(e) {
      // The mark itself, or the nearest one whose box (a few pixels fatter than
      // what is painted, so the gap between cells is not a dead zone) holds the pointer.
      var t = e.target && e.target.closest ? e.target.closest("[data-tip]") : null;
      if (t && svg.contains(t)) return stops.indexOf(t.getAttribute("data-ck-i"));
      if (!rects) {
        rects = [];
        svg.querySelectorAll("[data-tip]").forEach(function (m) {
          rects.push({ k: m.getAttribute("data-ck-i"), r: m.getBoundingClientRect() });
        });
      }
      var pad = 3;
      for (var i = 0; i < rects.length; i++) {
        var r = rects[i].r;
        if (e.clientX >= r.left - pad && e.clientX <= r.right + pad && e.clientY >= r.top - pad && e.clientY <= r.bottom + pad)
          return stops.indexOf(rects[i].k);
      }
      return -1;
    }

    function onPointer(e) {
      if (e.pointerType === "mouse" && e.buttons > 1) return;
      if (mode === "cols") {
        var s = e.target && e.target.closest ? e.target.closest("[data-ck-s]") : null;
        if (pinned < 0) applyFocus(s && svg.contains(s) ? Number(s.getAttribute("data-ck-s")) : -1);
        showCol(colAt(e.clientX), e.clientY);
      } else {
        var pos = markUnder(e);
        if (pos < 0) {
          if (e.pointerType === "mouse") hide();
          return;
        }
        showMark(pos, true);
      }
    }

    svg.addEventListener("pointermove", onPointer);
    svg.addEventListener("pointerdown", function (e) {
      rects = null;
      onPointer(e);
    });
    svg.addEventListener("pointerleave", function (e) {
      // A finger lifted from a screen leaves its last reading up; a mouse does not.
      if (e.pointerType === "mouse") hide();
    });
    svg.addEventListener("pointerenter", function () {
      rects = null;
    });

    // ── keyboard ──
    svg.addEventListener("focus", function () {
      // A pointer press focuses too; let the pointer path have it.
      if (svg.matches(":focus-visible")) {
        if (mode === "cols") {
          var at = cur >= 0 ? cur : cols.length - 1;
          showCol(at, null);
        } else {
          var pos = cur >= 0 ? cur : startAt !== null ? Math.max(0, stops.indexOf(startAt)) : 0;
          showMark(pos, false);
        }
      }
    });
    svg.addEventListener("blur", function () {
      hide();
    });
    svg.addEventListener("keydown", function (e) {
      var key = e.key;
      if (key === "Escape") {
        hide();
        return;
      }
      var n = mode === "cols" ? cols.length : stops.length;
      if (!n) return;
      // With no reading yet (focus reached the chart without showing one), the first key
      // starts from where a keyboard would have landed: the newest column, or the start mark.
      var from = mode === "cols" ? cols.length - 1 : startAt !== null ? Math.max(0, stops.indexOf(startAt)) : 0;
      var at = cur < 0 ? from : cur;
      var to = at;
      if (mode === "cols") {
        if (key === "ArrowRight") to = at + 1;
        else if (key === "ArrowLeft") to = at - 1;
        else if (key === "Home") to = 0;
        else if (key === "End") to = n - 1;
        else if ((key === "ArrowDown" || key === "ArrowUp") && series.length > 1) {
          var next = focusSeries + (key === "ArrowDown" ? 1 : -1);
          if (next >= series.length) next = -1;
          if (next < -1) next = series.length - 1;
          pinned = -1;
          applyFocus(next);
          showCol(cur < 0 ? cols.length - 1 : cur, null);
          e.preventDefault();
          return;
        } else return;
        e.preventDefault();
        showCol(Math.max(0, Math.min(n - 1, to)), null);
        return;
      }
      // Marks: arrows move by a mark or by a line of them, to the nearest mark that
      // exists in that direction (a calendar's first and last weeks are short).
      var dh = grid && major ? grid : 1;
      var dv = grid ? (major ? 1 : grid) : 0;
      var delta = 0;
      if (key === "ArrowRight") delta = dh;
      else if (key === "ArrowLeft") delta = -dh;
      else if (key === "ArrowDown" && dv) delta = dv;
      else if (key === "ArrowUp" && dv) delta = -dv;
      else if (key === "Home") to = 0;
      else if (key === "End") to = n - 1;
      else return;
      e.preventDefault();
      if (delta) {
        var want = Number(stops[at]) + delta;
        if (delta > 0) {
          to = n - 1;
          for (var i = at + 1; i < n; i++) {
            if (Number(stops[i]) >= want) {
              to = i;
              break;
            }
          }
        } else {
          to = 0;
          for (var j = at - 1; j >= 0; j--) {
            if (Number(stops[j]) <= want) {
              to = j;
              break;
            }
          }
        }
      }
      showMark(Math.max(0, Math.min(n - 1, to)), false);
    });

    // ── legend ──
    wrap.addEventListener("pointerover", function (e) {
      var b = e.target.closest && e.target.closest(".ck-leg");
      if (b && wrap.contains(b) && pinned < 0) applyFocus(Number(b.getAttribute("data-ck-s")));
    });
    wrap.addEventListener("pointerout", function (e) {
      var b = e.target.closest && e.target.closest(".ck-leg");
      if (b && pinned < 0) applyFocus(-1);
    });
    wrap.addEventListener("focusin", function (e) {
      var b = e.target.closest && e.target.closest(".ck-leg");
      if (b && pinned < 0) applyFocus(Number(b.getAttribute("data-ck-s")));
    });
    wrap.addEventListener("focusout", function (e) {
      var b = e.target.closest && e.target.closest(".ck-leg");
      if (b && pinned < 0) applyFocus(-1);
    });
    wrap.addEventListener("click", function (e) {
      var b = e.target.closest && e.target.closest(".ck-leg");
      if (!b || !wrap.contains(b)) return;
      var k = Number(b.getAttribute("data-ck-s"));
      pinned = pinned === k ? -1 : k;
      wrap.querySelectorAll(".ck-leg").forEach(function (o) {
        o.setAttribute("aria-pressed", pinned >= 0 && Number(o.getAttribute("data-ck-s")) === pinned ? "true" : "false");
      });
      applyFocus(pinned);
    });

    // A touch that lands outside lets the reading go.
    // (Taken off again once the chart has been swapped away, so a page that refreshes
    // a chart every thirty seconds does not collect one listener per refresh.)
    var elsewhere = function (e) {
      if (!wrap.isConnected) {
        document.removeEventListener("pointerdown", elsewhere);
        return;
      }
      if (!wrap.contains(e.target) && tip.classList.contains("on")) hide();
    };
    document.addEventListener("pointerdown", elsewhere);

    if (typeof ResizeObserver === "function") {
      var first = true;
      new ResizeObserver(function () {
        rects = null;
        if (first) {
          first = false;
          return;
        }
        if (tip.classList.contains("on") && document.activeElement !== svg) hide();
      }).observe(svg);
    }
    uid++;
  }

  // A chart wider than its card scrolls sideways inside it; a mark with `.ck-scroll-to`
  // (the "today" line of the migration ridgeline) is brought into view once, so the reader
  // does not arrive at January on a phone in October.
  function scrollToMark(svg) {
    var m = svg.querySelector(".ck-scroll-to");
    var sc = svg.parentNode;
    if (!m || !sc || sc.scrollWidth <= sc.clientWidth + 1 || sc.getAttribute("data-ck-scrolled")) return;
    sc.setAttribute("data-ck-scrolled", "1");
    sc.scrollLeft += m.getBoundingClientRect().left - sc.getBoundingClientRect().left - sc.clientWidth * 0.7;
  }

  // Row names (`.ck-sticky`) stay at the left edge of the card while the chart scrolls
  // under them: an SVG has no `position: sticky`, so they are moved by the scroll offset.
  function stick(svg) {
    var sc = svg.parentNode;
    var els = svg.querySelectorAll(".ck-sticky");
    if (!els.length || !sc || sc.scrollWidth <= sc.clientWidth + 1 || sc.getAttribute("data-ck-stuck")) return;
    sc.setAttribute("data-ck-stuck", "1");
    sc.addEventListener("scroll", function () {
      var k = svg.viewBox.baseVal.width / svg.getBoundingClientRect().width;
      var t = "translate(" + (sc.scrollLeft * k).toFixed(1) + " 0)";
      for (var j = 0; j < els.length; j++) els[j].setAttribute("transform", t);
    }, { passive: true });
  }

  function scan() {
    var list = document.querySelectorAll("svg[data-ck]");
    for (var i = 0; i < list.length; i++) {
      bind(list[i]);
      stick(list[i]);
      scrollToMark(list[i]);
    }
  }

  // `afterSwap` runs in the same task as the swap, before the browser paints, which
  // is what lets `.ck-still` stop a refreshed chart from replaying its entrance.
  document.addEventListener("htmx:afterSwap", scan);
  document.addEventListener("htmx:afterSettle", scan);
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", scan);
  else scan();
})();
