/* plate.js — the kiosk's "plate" display.
 *
 * The server writes a complete plate with no script: paper, plate-mark, title,
 * a numbered key, and for each species a hatched oval cartouche carrying its
 * Latin name. This script only improves that: it fetches each species'
 * picture and replaces the cartouche with it, engraved.
 *
 * Two kinds of picture arrive from the same endpoint, and they are treated
 * differently on purpose:
 *   - a photograph (the cache's Wikipedia / AviCommons / Flickr picture) is
 *     engraved: line-hatched by darkness, tinted from its own colours, and
 *     feathered into the paper. Nothing about it is hand-made, so it does not
 *     pretend to be, and the plate's footer says "engraved by the station from
 *     photographs".
 *   - a picture the operator put in the custom image directory
 *     (`X-Bnb-Image-Source: custom`) is art somebody made. It is drawn as it
 *     is, with its own alpha, and never processed.
 *
 * No eval, no inline style, no third-party code. Every colour comes from the
 * plate's own CSS custom properties, so a day plate and a night plate are the
 * same code reading different values. When the ink is lighter than the paper the
 * tone map is inverted, so the lines fall where the picture is bright.
 */
(function () {
  'use strict';

  var SVG_NS = 'http://www.w3.org/2000/svg';
  var XLINK_NS = 'http://www.w3.org/1999/xlink';

  /* ---- small deterministic helpers (no Math.random: a redraw must not shimmer) ---- */
  function hash(x, y) {
    var h = (x * 374761393 + y * 668265263) | 0;
    h = Math.imul(h ^ (h >>> 13), 1274126177);
    return ((h ^ (h >>> 16)) >>> 0) / 4294967295;
  }
  function vnoise(x, y) {
    var xi = Math.floor(x), yi = Math.floor(y), xf = x - xi, yf = y - yi;
    var u = xf * xf * (3 - 2 * xf), v = yf * yf * (3 - 2 * yf);
    return hash(xi, yi) * (1 - u) * (1 - v) + hash(xi + 1, yi) * u * (1 - v) +
           hash(xi, yi + 1) * (1 - u) * v + hash(xi + 1, yi + 1) * u * v;
  }
  function smooth(a, b, x) {
    var t = Math.min(1, Math.max(0, (x - a) / (b - a)));
    return t * t * (3 - 2 * t);
  }
  function parseColour(s, fallback) {
    // The plate's tokens are written as #rrggbb; anything else falls back.
    var m = /^#([0-9a-f]{6})$/i.exec((s || '').trim());
    if (!m) return fallback;
    var n = parseInt(m[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  function token(el, name, fallback) {
    return parseColour(getComputedStyle(el).getPropertyValue(name), fallback);
  }

  /* ---- the engraving ---------------------------------------------------- */
  // Three sets of parallel lines at different angles; a set only draws where the
  // picture is darker than its threshold, and its line thickens as it gets
  // darker. The lines wander a little (low-frequency noise) so they read as
  // cut by hand, and the edge of the print is deckled by the same noise so no
  // two plates share an outline.
  //
  // The whole photograph is kept. An earlier version cropped it to an oval to
  // fill the figure's box and faded everything near the oval's rim, which cut
  // the tail or the head off any bird that was not dead centre. The picture is
  // fitted inside the box instead, and only a thin rim of its own rectangle is
  // feathered.
  var ANGLES = [-35, 55, -82].map(function (a) { return a * Math.PI / 180; });
  var THRESHOLDS = [0.88, 0.58, 0.32];

  function luma(c) { return 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]; }

  function engrave(img, w, h, ink, paper, wash) {
    // Dark ink on light paper puts its lines where the picture is dark. Light ink on dark paper
    // (the night plate) has to do the opposite, or the engraving is a photographic negative: a
    // black bird drawn in bright strokes, white on a dark ground.
    var invert = luma(ink) > luma(paper);
    var c = document.createElement('canvas');
    c.width = w; c.height = h;
    var g = c.getContext('2d', { willReadFrequently: true });
    var s = Math.min(w / img.width, h / img.height);
    var dw = Math.max(1, Math.round(img.width * s)), dh = Math.max(1, Math.round(img.height * s));
    var x0 = Math.floor((w - dw) / 2), y0 = Math.floor((h - dh) / 2);
    g.drawImage(img, x0, y0, dw, dh);
    var src = g.getImageData(0, 0, w, h), d = src.data;
    var out = g.createImageData(w, h), o = out.data;
    var n = w * h, L = new Float32Array(n), B = new Float32Array(n), i, x, y;
    for (i = 0; i < n; i++) L[i] = (0.299 * d[i * 4] + 0.587 * d[i * 4 + 1] + 0.114 * d[i * 4 + 2]) / 255;
    for (y = 1; y < h - 1; y++) for (x = 1; x < w - 1; x++) {
      i = y * w + x;
      B[i] = (L[i] * 4 + L[i - 1] + L[i + 1] + L[i - w] + L[i + w]) / 8;
    }
    // The tone range comes from the picture itself, not the empty margin around it.
    var inner = new Float32Array(dw * dh), m = 0;
    for (y = y0; y < y0 + dh; y++) for (x = x0; x < x0 + dw; x++) inner[m++] = B[y * w + x];
    var sorted = inner.sort();
    var lo = sorted[Math.floor(m * 0.03)], hi = sorted[Math.floor(m * 0.97)];
    if (hi - lo < 0.05) hi = lo + 0.05; // a flat picture: do not stretch noise to full range
    var spacing = Math.max(2.4, w / 135);
    var feather = Math.max(4, Math.min(dw, dh) * 0.05);
    for (y = y0; y < y0 + dh; y++) for (x = x0; x < x0 + dw; x++) {
      i = y * w + x;
      var l = Math.min(1, Math.max(0, (B[i] - lo) / (hi - lo)));
      l = Math.pow(l, 1.15);
      if (invert) l = 1 - l;
      var wob = (vnoise(x * 0.045, y * 0.045) - 0.5) * spacing * 1.6 + (vnoise(x * 0.2, y * 0.2) - 0.5) * 0.35;
      var cov = 0;
      for (var k = 0; k < 3; k++) {
        if (l >= THRESHOLDS[k]) continue;
        var u = x * Math.cos(ANGLES[k]) + y * Math.sin(ANGLES[k]) + wob + k * 1.3;
        var ph = Math.abs(((u / spacing) % 1 + 1) % 1 - 0.5) * 2; // 0 on a line, 1 between two
        var f = Math.pow((THRESHOLDS[k] - l) / THRESHOLDS[k], 0.75);
        cov = Math.max(cov, 1 - smooth(f - 0.14, f + 0.14, 1 - ph));
      }
      var r = paper[0] * (1 - wash) + d[i * 4] * wash;
      var gg = paper[1] * (1 - wash) + d[i * 4 + 1] * wash;
      var b = paper[2] * (1 - wash) + d[i * 4 + 2] * wash;
      var a = cov * 0.94;
      o[i * 4] = r * (1 - a) + ink[0] * a;
      o[i * 4 + 1] = gg * (1 - a) + ink[1] * a;
      o[i * 4 + 2] = b * (1 - a) + ink[2] * a;
      // Distance in from the nearest edge of the picture, wobbled so the edge is deckled.
      var edge = Math.min(x - x0, x0 + dw - 1 - x, y - y0, y0 + dh - 1 - y) +
                 (vnoise(x * 0.09, y * 0.09) - 0.5) * feather * 1.2;
      o[i * 4 + 3] = smooth(0, feather, edge) * 255;
    }
    g.putImageData(out, 0, 0);
    return c;
  }

  // Art the operator supplied: kept as it is, drawn to fit inside the figure's box.
  function asIs(img, w, h) {
    var c = document.createElement('canvas');
    c.width = w; c.height = h;
    var g = c.getContext('2d');
    var s = Math.min(w / img.width, h / img.height);
    g.drawImage(img, (w - img.width * s) / 2, (h - img.height * s) / 2, img.width * s, img.height * s);
    return c;
  }

  /* ---- placing a picture into the SVG ---------------------------------- */
  function basePath() {
    return (document.body && document.body.getAttribute('data-base-path')) || '';
  }

  function paint(fig, svg) {
    var slot = fig.querySelector('.plate-pic');
    if (!slot || fig.getAttribute('data-state') === 'ready') return;
    var w = Math.round(parseFloat(slot.getAttribute('width')) * (window.devicePixelRatio > 1 ? 1.5 : 1));
    var h = Math.round(parseFloat(slot.getAttribute('height')) * (window.devicePixelRatio > 1 ? 1.5 : 1));
    var sci = fig.getAttribute('data-sci');
    var url = basePath() + '/api/v2/species/image/' + encodeURIComponent(sci) + '/file';
    fig.setAttribute('data-state', 'loading');
    fetch(url, { credentials: 'same-origin' }).then(function (r) {
      if (!r.ok) throw new Error('no picture');
      var custom = r.headers.get('X-Bnb-Image-Source') === 'custom';
      return r.blob().then(function (blob) { return { blob: blob, custom: custom }; });
    }).then(function (res) {
      return createImageBitmap(res.blob).then(function (bmp) {
        var root = svg;
        var canvas = res.custom
          ? asIs(bmp, w, h)
          : engrave(bmp, w, h,
              token(root, '--plate-ink', [42, 32, 23]),
              token(root, '--plate-paper', [237, 227, 205]),
              parseFloat(getComputedStyle(root).getPropertyValue('--plate-wash')) || 0.52);
        slot.setAttributeNS(XLINK_NS, 'href', canvas.toDataURL('image/png'));
        slot.setAttribute('href', canvas.toDataURL('image/png'));
        fig.setAttribute('data-state', 'ready');
        fig.setAttribute('data-kind', res.custom ? 'art' : 'engraved');
        if (bmp.close) bmp.close();
      });
    }).catch(function () {
      // Keep the cartouche the server drew. A species with no picture is a
      // normal case (offline, not in any provider), not an error to announce.
      fig.setAttribute('data-state', 'none');
    });
  }

  function init(root) {
    var svgs = (root || document).querySelectorAll('svg.plate-svg');
    Array.prototype.forEach.call(svgs, function (svg) {
      var figs = svg.querySelectorAll('.plate-fig[data-sci]');
      // One at a time, biggest first (document order): the Pi decodes and
      // hatches a picture in tens of milliseconds, but a dozen at once is a
      // visible stall on the first draw of the day.
      var queue = Array.prototype.slice.call(figs);
      (function next() {
        var fig = queue.shift();
        if (!fig) return;
        paint(fig, svg);
        setTimeout(next, 60);
      })();
    });
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', function () { init(); });
  else init();
  // The plate is swapped in by htmx (and only when the birds changed).
  document.addEventListener('htmx:afterSwap', function (e) { init(e.target); });
  window.bnbPlate = { init: init, engrave: engrave };
})();
