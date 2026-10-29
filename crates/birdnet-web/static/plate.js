/* plate.js — the kiosk's "plate" display.
 *
 * The server writes a complete plate with no script: paper, plate-mark, title,
 * a numbered key, and for each species a hatched oval cartouche carrying its
 * Latin name. This script only improves that: it fetches each species'
 * picture and replaces the cartouche with it, engraved.
 *
 * Three kinds of picture are drawn, each differently on purpose. The plate
 * endpoint is asked first; only when it has nothing (most species) is the
 * photograph asked for.
 *   - a public-domain plate (`X-Bnb-Image-Source: plate`, a nineteenth-century
 *     engraving by Gould, Naumann or von Wright) is already a plate. It is
 *     shown whole, with its paper levelled to the page's: on the day plate
 *     its paper is multiplied into the sheet and its edge feathered, on the
 *     night plate it is a lit print mounted on the dark ground.
 *   - a photograph (the cache's Wikipedia / AviCommons / Flickr picture) is
 *     engraved: line-hatched by darkness, tinted from its own colours, and
 *     feathered into the paper. Nothing about it is hand-made, so it does not
 *     pretend to be, and the plate's footer says "engraved by the station from
 *     photographs".
 *   - a picture the operator put in the custom image directory
 *     (`X-Bnb-Image-Source: custom`) is art somebody made. It is drawn as it
 *     is, with its own alpha, and never processed.
 * When the pictures have settled the footer is rewritten from what was drawn:
 * the books the plates are from, whether any photograph was engraved, and
 * where the credits are.
 *
 * No eval, no inline style, no third-party code. Every colour comes from the
 * plate's own CSS custom properties, so a day plate and a night plate are the
 * same code reading different values. When the ink is lighter than the paper the
 * tone map is inverted, so the lines fall where the picture is bright.
 */
(function () {
  'use strict';

  /* The sheet is drawn on 1600 units and scaled to the window, and its smallest label
     (`.plate-com`, 11.5 units) is 8.5 px at 1180 px of window and 2.8 px on a phone. A
     window narrower than that is given the numbers face unless the address names a mode,
     so a phone opens on something it can read and the plate stays one tap away. */
  var MIN_SHEET_PX = 1180;
  if (!/[?&]mode=/.test(location.search) && window.innerWidth < MIN_SHEET_PX) {
    location.replace(location.pathname + '?mode=numbers');
    return;
  }

  /* The operator's "Reduced" motion setting (Display preferences, `bnb-motion`). This page does
     not carry the preference guard the other pages do, so without this the sheet's slow drift
     ignores it; the OS setting is read by the stylesheet. */
  try {
    if (localStorage.getItem('bnb-motion') === 'reduced') {
      document.documentElement.setAttribute('data-motion', 'reduced');
    }
  } catch (e) { /* storage blocked: the OS setting still applies */ }

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

  // A public-domain plate, after three corrections. The scans are of bound books, so many carry the
  // page's dark edge, the binding and the table beyond: `pageBounds` finds the paper and the plate
  // is cut to it. A page of a folio is mostly paper, with the birds in the middle of it; fitted
  // whole into a figure's box the birds come out small, so the plate is then cut again to its ink
  // (`contentBounds`), with a margin. And the paper is levelled: every channel scaled so that the
  // paper becomes white, which on the day plate is then multiplied into the sheet by the browser
  // (`plate.css`), so the paper of the print is the paper of the sheet, fibres and all. At night
  // `--plate-mount` is a lit tone and the print stays a print. A plate that is a picture to its
  // edge (a chromolithograph with a landscape) has no paper to level, and levelling it would tint
  // the scenery, so it is drawn as it is. The rim is feathered with the engraving's own noise.
  function luma255(d, i) { return 0.299 * d[i] + 0.587 * d[i + 1] + 0.114 * d[i + 2]; }

  // `rect` of `img` (fractions: left, top, right, bottom) drawn into a canvas no larger than 200
  // pixels on a side, as pixel data.
  function analyse(img, rect) {
    var sw = (rect[2] - rect[0]) * img.width, sh = (rect[3] - rect[1]) * img.height;
    var side = 200, aw = sw >= sh ? side : Math.max(8, Math.round(side * sw / sh));
    var ah = sw >= sh ? Math.max(8, Math.round(side * sh / sw)) : side;
    var c = document.createElement('canvas');
    c.width = aw; c.height = ah;
    var g = c.getContext('2d', { willReadFrequently: true });
    g.drawImage(img, rect[0] * img.width, rect[1] * img.height, sw, sh, 0, 0, aw, ah);
    return { d: g.getImageData(0, 0, aw, ah).data, w: aw, h: ah };
  }

  // The paper's rectangle as fractions of the picture, or the whole picture when no clear page is
  // found. A column (row) belongs to the page when most of the middle of it is as bright as paper
  // is, judged against the picture's own brightest tenth.
  function pageBounds(img) {
    var a = analyse(img, [0, 0, 1, 1]), d = a.d, aw = a.w, ah = a.h, all = [], i;
    for (i = 0; i < aw * ah; i += 3) all.push(luma255(d, i * 4));
    all.sort(function (x, y) { return x - y; });
    var floor = 0.62 * all[Math.floor(all.length * 0.9)];
    function span(len, cross, at) {
      var lo = -1, hi = -1, k, j, n, hit;
      for (k = 0; k < len; k++) {
        n = 0; hit = 0;
        for (j = Math.floor(cross * 0.2); j < Math.floor(cross * 0.8); j++) { n++; if (luma255(d, at(k, j)) > floor) hit++; }
        if (hit / n > 0.7) { if (lo < 0) lo = k; hi = k; }
      }
      return lo < 0 ? null : [lo / len, (hi + 1) / len];
    }
    var cols = span(aw, ah, function (k, j) { return (j * aw + k) * 4; });
    var rows = span(ah, aw, function (k, j) { return (k * aw + j) * 4; });
    if (!cols || !rows || (cols[1] - cols[0]) * (rows[1] - rows[0]) < 0.5) return [0, 0, 1, 1];
    // A little further in, off the page's own ragged edge and its shadow.
    var mx = (cols[1] - cols[0]) * 0.02, my = (rows[1] - rows[0]) * 0.02;
    return [cols[0] + mx, rows[0] + my, cols[1] - mx, rows[1] - my];
  }

  // The paper's colour in a picture of a page, from a band 3% to 8% in from its edge (the brightest
  // three quarters of it, per channel), or null when that band is not paper: paper is light and
  // nearly colourless, and a band that is neither is picture.
  function paperOf(a) {
    var d = a.d, w = a.w, h = a.h, band = [[], [], []], x, y, k, i;
    var i0 = Math.max(1, Math.round(Math.min(w, h) * 0.03)), i1 = Math.max(i0 + 1, Math.round(Math.min(w, h) * 0.08));
    for (y = 0; y < h; y++) for (x = 0; x < w; x++) {
      var e = Math.min(x, w - 1 - x, y, h - 1 - y);
      if (e < i0 || e >= i1) continue;
      i = (y * w + x) * 4;
      band[0].push(d[i]); band[1].push(d[i + 1]); band[2].push(d[i + 2]);
    }
    if (band[0].length < 16) return null;
    var paper = [0, 0, 0];
    for (k = 0; k < 3; k++) {
      band[k].sort(function (p, q) { return p - q; });
      paper[k] = Math.max(1, band[k][Math.floor(band[k].length * 0.75)]);
    }
    if (luma255(paper, 0) < 170 || Math.max(paper[0], paper[1], paper[2]) - Math.min(paper[0], paper[1], paper[2]) > 80) return null;
    return paper;
  }

  // Where the ink is, as fractions of the page: the rows and columns with more than a speck of it.
  // The foot of the page (its printed caption) and the outer margin are not looked at. Whole page
  // when the answer is implausible (nearly nothing, or nearly everything).
  function contentBounds(a, paper) {
    var d = a.d, w = a.w, h = a.h, rowN = new Array(h).fill(0), colN = new Array(w).fill(0), x, y, k;
    var yMax = Math.floor(h * 0.89), m = Math.floor(Math.min(w, h) * 0.03);
    for (y = m; y < yMax; y++) for (x = m; x < w - m; x++) {
      var i = (y * w + x) * 4, deficit = 0;
      for (k = 0; k < 3; k++) deficit = Math.max(deficit, 1 - d[i + k] / paper[k]);
      if (deficit > 0.14) { rowN[y]++; colN[x]++; }
    }
    function span(counts, from, to, across) {
      var lo = -1, hi = -1, j;
      for (j = from; j < to; j++) if (counts[j] > across * 0.012) { if (lo < 0) lo = j; hi = j; }
      return lo < 0 ? null : [lo, hi + 1];
    }
    var cs = span(colN, 0, w, yMax - m), rs = span(rowN, 0, yMax, w - 2 * m);
    if (!cs || !rs) return [0, 0, 1, 1];
    var bw = cs[1] - cs[0], bh = rs[1] - rs[0];
    if ((bw * bh) / (w * h) < 0.1 || (bw * bh) / (w * h) > 0.9) return [0, 0, 1, 1];
    var pad = 0.1;
    return [
      Math.max(0, (cs[0] - bw * pad) / w), Math.max(0, (rs[0] - bh * pad) / h),
      Math.min(1, (cs[1] + bw * pad) / w), Math.min(1, (rs[1] + bh * pad) / h)
    ];
  }

  function levelled(img, w, h, mount) {
    var page = pageBounds(img), paper = paperOf(analyse(img, page)), rect = page;
    if (paper) {
      var cb = contentBounds(analyse(img, page), paper), pw = page[2] - page[0], ph = page[3] - page[1];
      rect = [page[0] + cb[0] * pw, page[1] + cb[1] * ph, page[0] + cb[2] * pw, page[1] + cb[3] * ph];
    }
    var sx = rect[0] * img.width, sy = rect[1] * img.height;
    var sw = (rect[2] - rect[0]) * img.width, sh = (rect[3] - rect[1]) * img.height;
    var c = document.createElement('canvas');
    c.width = w; c.height = h;
    var g = c.getContext('2d', { willReadFrequently: true });
    var s = Math.min(w / sw, h / sh);
    var dw = Math.max(1, Math.round(sw * s)), dh = Math.max(1, Math.round(sh * s));
    var x0 = Math.floor((w - dw) / 2), y0 = Math.floor((h - dh) / 2);
    g.drawImage(img, sx, sy, sw, sh, x0, y0, dw, dh);
    var src = g.getImageData(x0, y0, dw, dh), d = src.data, x, y, k;
    var feather = Math.max(4, Math.min(dw, dh) * 0.05);
    for (y = 0; y < dh; y++) for (x = 0; x < dw; x++) {
      var i = (y * dw + x) * 4;
      if (paper) for (k = 0; k < 3; k++) d[i + k] = Math.min(1, d[i + k] / paper[k]) * mount[k];
      var edge = Math.min(x, dw - 1 - x, y, dh - 1 - y) +
                 (vnoise((x + x0) * 0.09, (y + y0) * 0.09) - 0.5) * feather * 1.2;
      d[i + 3] = smooth(0, feather, edge) * 255;
    }
    g.clearRect(0, 0, w, h);
    g.putImageData(src, x0, y0);
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

  // One picture from `url`, with which kind of picture it is (`X-Bnb-Image-Source`), or null when
  // the endpoint has none for this species. A species with no plate is the common case, and the
  // plate endpoint says so with 204, which a browser does not log; the photograph endpoint says
  // 404 for a species with no photograph.
  function fetchPicture(url) {
    return fetch(url, { credentials: 'same-origin' }).then(function (r) {
      if (r.status === 204 || !r.ok) return null;
      var source = r.headers.get('X-Bnb-Image-Source') || 'photo';
      return r.blob().then(function (blob) { return { blob: blob, source: source }; });
    });
  }

  function paint(fig, svg) {
    var slot = fig.querySelector('.plate-pic');
    if (!slot || fig.getAttribute('data-state') === 'ready') return Promise.resolve();
    var w = Math.round(parseFloat(slot.getAttribute('width')) * (window.devicePixelRatio > 1 ? 1.5 : 1));
    var h = Math.round(parseFloat(slot.getAttribute('height')) * (window.devicePixelRatio > 1 ? 1.5 : 1));
    var name = encodeURIComponent(fig.getAttribute('data-sci'));
    fig.setAttribute('data-state', 'loading');
    // A plate (or the operator's own art, which the plate endpoint also answers first) before a
    // photograph: a species with a plate never downloads a photograph it will not draw.
    return fetchPicture(basePath() + '/api/v2/species/plate/' + name + '/file').then(function (res) {
      return res || fetchPicture(basePath() + '/api/v2/species/image/' + name + '/file');
    }).then(function (res) {
      if (!res) throw new Error('no picture');
      return createImageBitmap(res.blob).then(function (bmp) {
        var canvas, kind;
        if (res.source === 'custom') {
          canvas = asIs(bmp, w, h); kind = 'art';
        } else if (res.source === 'plate') {
          canvas = levelled(bmp, w, h, token(svg, '--plate-mount', [237, 227, 205])); kind = 'plate';
        } else {
          canvas = engrave(bmp, w, h,
            token(svg, '--plate-ink', [42, 32, 23]),
            token(svg, '--plate-paper', [237, 227, 205]),
            parseFloat(getComputedStyle(svg).getPropertyValue('--plate-wash')) || 0.52);
          kind = 'engraved';
        }
        slot.setAttributeNS(XLINK_NS, 'href', canvas.toDataURL('image/png'));
        slot.setAttribute('href', canvas.toDataURL('image/png'));
        fig.setAttribute('data-state', 'ready');
        fig.setAttribute('data-kind', kind);
        if (bmp.close) bmp.close();
      });
    }).catch(function () {
      // Keep the cartouche the server drew. A species with no picture is a
      // normal case (offline, not in any provider), not an error to announce.
      fig.setAttribute('data-state', 'none');
    });
  }

  /* ---- the foot ----------------------------------------------------------- */
  // Rewritten from what was drawn, because the server cannot know: which books the plates are from,
  // whether a photograph was engraved, whether art was drawn as it was made. Where nothing was
  // drawn the server's line stands. It is measured against the room the server left it, and
  // shortened a clause at a time, never cut mid-word unless nothing else fits.
  function foot(svg) {
    var el = svg.querySelector('#plate-colophon');
    if (!el) return;
    var count = { plate: 0, engraved: 0, art: 0 }, works = [];
    Array.prototype.forEach.call(svg.querySelectorAll('.plate-fig'), function (fig) {
      var kind = fig.getAttribute('data-kind');
      if (!(kind in count)) return;
      count[kind]++;
      var work = fig.getAttribute('data-work');
      if (kind === 'plate' && work && works.indexOf(work) < 0) works.push(work);
    });
    if (!count.plate && !count.engraved && !count.art) return;
    var max = parseFloat(el.getAttribute('data-max')) || 0;
    var art = el.getAttribute('data-art') || '';
    var credits = 'Picture credits: ' + location.host + basePath() + '/system/credits';
    function fits(text) {
      el.textContent = text;
      if (!max) return true;
      try { return el.getComputedTextLength() <= max; } catch (e) { return true; }
    }
    // `o`: how many works to name (and whether with their years), the size sentence, the credits
    // address. A photograph that was engraved is an adaptation that has to be credited, so the
    // address is the last thing given up, and only when no photograph was drawn.
    function compose(o) {
      var parts = [];
      if (count.art && art) parts.push('Art: ' + art);
      if (count.plate) {
        var names = works.map(function (wk) { return o.years ? wk : wk.replace(/ \(\d{4}[–-]\d{4}\)$/, ''); });
        var shown = names.slice(0, o.works), more = names.length - shown.length;
        parts.push(shown.length
          ? 'Plates after ' + shown.join(', ') + (more > 0 ? ' and ' + more + ' more' : '') + ', public domain'
          : 'Plates from ' + names.length + (names.length === 1 ? ' book' : ' books') + ' in the public domain');
      }
      if (count.engraved) parts.push('engraved by the station from photographs');
      if (o.size) parts.push('size follows the number of detections');
      if (o.credits && (count.plate || count.engraved)) parts.push(credits);
      var text = parts.join(' · ');
      return text.charAt(0).toUpperCase() + text.slice(1);
    }
    var all = works.length;
    var tries = [
      { works: all, years: true, size: true, credits: true },
      { works: all, years: true, size: false, credits: true },
      { works: all, years: false, size: false, credits: true },
      { works: 1, years: false, size: false, credits: true },
      { works: 0, years: false, size: false, credits: true }
    ];
    for (var i = 0; i < tries.length; i++) {
      if (fits(compose(tries[i]))) return;
    }
    var text = compose(tries[tries.length - 1]);
    while (text.length > 8 && !fits(text + '…')) text = text.slice(0, -1);
    el.textContent = text + '…';
  }

  function init(root) {
    var svgs = (root || document).querySelectorAll('svg.plate-svg');
    Array.prototype.forEach.call(svgs, function (svg) {
      var figs = svg.querySelectorAll('.plate-fig[data-sci]');
      // One at a time, biggest first (document order): the Pi decodes and
      // hatches a picture in tens of milliseconds, but a dozen at once is a
      // visible stall on the first draw of the day.
      var queue = Array.prototype.slice.call(figs), settled = [];
      (function next() {
        var fig = queue.shift();
        if (!fig) {
          Promise.all(settled).then(function () { foot(svg); });
          return;
        }
        settled.push(paint(fig, svg));
        setTimeout(next, 60);
      })();
    });
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', function () { init(); });
  else init();
  // The plate is swapped in by htmx (and only when the birds changed).
  document.addEventListener('htmx:afterSwap', function (e) { init(e.target); });
  window.bnbPlate = { init: init, engrave: engrave, levelled: levelled };
})();
