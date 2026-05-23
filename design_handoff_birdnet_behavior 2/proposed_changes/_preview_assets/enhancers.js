/* ============================================================================
   BirdNet-Behavior · Preview enhancers
   Procedurally generates the rich SVGs the previews need — sparklines,
   mini-waveforms, polar ribbons, day clocks, weekly heat grids, ridgelines,
   live-signal canvases. Token-driven; never invents colors.

   Markup contract (data-* attributes):
     [data-spark]      seeds= '0.2,0.3,…' (CSV)  · accent= var-name  · area?
     [data-wave]       seed=  integer            · bars= int          · color= var-name
     [data-clock]      now= decimal hours        · rise= h            · set= h
     [data-week-heat]  accent= var-name          · weeks= int         · seed= int
     [data-mini-heat]  accent= var-name          · seed= int
     [data-polar]      seed= int                 · rise= h            · set= h
     [data-ridge]      seed= int
     [data-live]       (canvas)                  · color= var-name
     [data-pulse-dot]  color= var-name
   ===========================================================================*/
(function () {
  'use strict';

  // ── Deterministic PRNG ──────────────────────────────────────────────
  function rng(seed) {
    let s = (Number(seed) || 1) >>> 0;
    if (s === 0) s = 1;
    return function () {
      s = (s * 1664525 + 1013904223) >>> 0;
      return (s & 0x7fffffff) / 0x7fffffff;
    };
  }

  function clamp(v, a, b) { return Math.max(a, Math.min(b, v)); }

  // ── Sparkline ───────────────────────────────────────────────────────
  function renderSpark(el) {
    const seedAttr = el.getAttribute('data-seeds');
    let data;
    if (seedAttr) {
      data = seedAttr.split(',').map(Number);
    } else {
      const n = +el.getAttribute('data-points') || 24;
      const r = rng(+el.getAttribute('data-seed') || 7);
      const trend = +el.getAttribute('data-trend') || 0.4; // 0..1 rise
      data = [];
      let v = 0.35 + r() * 0.2;
      for (let i = 0; i < n; i++) {
        v += (r() - 0.5) * 0.18 + trend * 0.025;
        data.push(clamp(v, 0.08, 1));
      }
    }
    const W = el.clientWidth || 220;
    const H = el.clientHeight || 26;
    const accent = el.getAttribute('data-accent') || 'var(--moss)';
    const max = Math.max(...data);
    const stepX = W / (data.length - 1);
    const pts = data.map((v, i) => [i * stepX, H - 2 - (v / max) * (H - 6)]);
    const path = pts.map(([x, y], i) => `${i === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`).join(' ');
    const area = el.hasAttribute('data-area') ? `${path} L${W},${H} L0,${H} Z` : null;
    const lastX = pts[pts.length - 1][0], lastY = pts[pts.length - 1][1];

    el.innerHTML =
      `<svg viewBox="0 0 ${W} ${H}" width="100%" height="100%" preserveAspectRatio="none">
        ${area ? `<path d="${area}" fill="${accent}" fill-opacity="0.14"/>` : ''}
        <path d="${path}" stroke="${accent}" fill="none" stroke-width="1.4" stroke-linejoin="round"/>
        <circle cx="${lastX.toFixed(1)}" cy="${lastY.toFixed(1)}" r="1.8" fill="${accent}"/>
      </svg>`;
  }

  // ── Mini-waveform (deterministic call envelope) ─────────────────────
  function renderWave(el) {
    const seed = +el.getAttribute('data-seed') || 1;
    const bars = +el.getAttribute('data-bars') || 24;
    const color = el.getAttribute('data-color') || 'var(--moss)';
    const r = rng(seed);
    let out = '<span style="display:inline-flex;align-items:center;gap:1.5px;height:22px;">';
    for (let i = 0; i < bars; i++) {
      const env = Math.sin((i / bars) * Math.PI);
      const v = 0.25 + env * (0.55 + r() * 0.4);
      const mix = 30 + v * 60;
      out += `<span style="width:2px;height:${Math.round(v * 22)}px;border-radius:1px;background:color-mix(in oklch, ${color} ${mix}%, var(--fg-4));"></span>`;
    }
    out += '</span>';
    el.innerHTML = out;
  }

  // ── Day clock (24h dial w/ night wedge + dawn band + now hand) ──────
  function renderClock(el) {
    const now = +el.getAttribute('data-now') || 6.7;
    const rise = +el.getAttribute('data-rise') || 5.35;
    const set = +el.getAttribute('data-set') || 20.13;
    const size = +el.getAttribute('data-size') || 80;
    const r = size / 2 - 6, cx = size / 2, cy = size / 2;
    const a = h => (h / 24) * Math.PI * 2 - Math.PI / 2;
    const a1 = a(set), a2 = a(rise + 24);
    const x1 = cx + r * Math.cos(a1), y1 = cy + r * Math.sin(a1);
    const x2 = cx + r * Math.cos(a2), y2 = cy + r * Math.sin(a2);
    const sweep = (a2 - a1) % (Math.PI * 2);
    const large = sweep > Math.PI ? 1 : 0;
    const dx0 = cx + r * Math.cos(a(rise)), dy0 = cy + r * Math.sin(a(rise));
    const dx1 = cx + r * Math.cos(a(rise + 2.7)), dy1 = cy + r * Math.sin(a(rise + 2.7));
    const na = a(now);
    const nx = cx + (r - 2) * Math.cos(na), ny = cy + (r - 2) * Math.sin(na);
    el.innerHTML =
      `<svg width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">
        <circle cx="${cx}" cy="${cy}" r="${r}" fill="var(--surface)" stroke="var(--border)"/>
        <path d="M${cx},${cy} L${x1.toFixed(2)},${y1.toFixed(2)} A${r},${r} 0 ${large} 1 ${x2.toFixed(2)},${y2.toFixed(2)} Z" fill="var(--night)" fill-opacity="0.18"/>
        <path d="M${cx},${cy} L${dx0.toFixed(2)},${dy0.toFixed(2)} A${r},${r} 0 0 1 ${dx1.toFixed(2)},${dy1.toFixed(2)} Z" fill="var(--dawn)" fill-opacity="0.45"/>
        <line x1="${cx}" y1="${cy}" x2="${nx.toFixed(2)}" y2="${ny.toFixed(2)}" stroke="var(--fg)" stroke-width="1.5" stroke-linecap="round"/>
        <circle cx="${cx}" cy="${cy}" r="2.5" fill="var(--fg)"/>
        <text x="${cx}" y="6" text-anchor="middle" font-family="JetBrains Mono" font-size="7" fill="var(--fg-3)">12a</text>
        <text x="${size - 2}" y="${cy + 2}" text-anchor="end" font-family="JetBrains Mono" font-size="7" fill="var(--fg-3)">6a</text>
        <text x="${cx}" y="${size - 1}" text-anchor="middle" font-family="JetBrains Mono" font-size="7" fill="var(--fg-3)">12p</text>
        <text x="2" y="${cy + 2}" font-family="JetBrains Mono" font-size="7" fill="var(--fg-3)">6p</text>
      </svg>`;
  }

  // ── 12-week × 7-day heat grid ───────────────────────────────────────
  function renderWeekHeat(el) {
    const accent = el.getAttribute('data-accent') || 'var(--moss)';
    const weeks = +el.getAttribute('data-weeks') || 12;
    const r = rng(+el.getAttribute('data-seed') || 7);
    let max = 0;
    const grid = [];
    for (let w = 0; w < weeks; w++) {
      const col = [];
      for (let d = 0; d < 7; d++) {
        const base = (w / weeks) * 0.7 + 0.25;
        const dayBoost = (d === 0 || d === 6) ? 0.55 : 1.0;
        const v = Math.max(0, base * dayBoost * (0.4 + r() * 1.4));
        col.push(v);
        if (v > max) max = v;
      }
      grid.push(col);
    }
    let html = '<div style="display:flex;gap:8px;align-items:center;width:100%;height:100%;">';
    html += '<div style="display:flex;flex-direction:column;justify-content:space-around;height:100px;">';
    ['M', 'W', 'F'].forEach(d => {
      html += `<span class="mono" style="font-size:9.5px;color:var(--fg-3);">${d}</span>`;
    });
    html += '</div>';
    html += `<div style="flex:1;display:grid;grid-template-columns:repeat(${weeks}, 1fr);gap:3px;">`;
    grid.forEach((col, wi) => {
      html += `<div style="display:grid;grid-template-rows:repeat(7, 1fr);gap:3px;height:100px;">`;
      col.forEach((v, di) => {
        const op = v / max;
        const bg = op < 0.05 ? 'var(--surface-2)' : `color-mix(in oklch, ${accent} ${Math.min(85, op * 90).toFixed(0)}%, var(--surface-2))`;
        const ring = (wi === weeks - 1 && di === 4) ? '1px solid var(--fg)' : 'none';
        html += `<div style="background:${bg};border-radius:2px;outline:${ring};outline-offset:0;"></div>`;
      });
      html += `</div>`;
    });
    html += '</div></div>';
    el.innerHTML = html;
  }

  // ── 7-day × 24-hour mini heat ───────────────────────────────────────
  function renderMiniHeat(el) {
    const accent = el.getAttribute('data-accent') || 'var(--moss)';
    const r = rng(+el.getAttribute('data-seed') || 33);
    const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
    let html = '';
    html += '<div style="display:flex;gap:6px;"><div style="width:28px;"></div>';
    html += '<div style="flex:1;display:grid;grid-template-columns:repeat(24, 1fr);gap:2px;">';
    for (let h = 0; h < 24; h++) {
      html += `<span class="mono" style="font-size:8.5px;color:var(--fg-4);text-align:center;">${h % 6 === 0 ? h : ''}</span>`;
    }
    html += '</div></div>';
    DAYS.forEach((dn, di) => {
      html += `<div style="display:flex;gap:6px;align-items:center;margin-top:3px;">`;
      html += `<span class="mono" style="font-size:10px;color:var(--fg-3);width:28px;">${dn}</span>`;
      html += `<div style="flex:1;display:grid;grid-template-columns:repeat(24, 1fr);gap:2px;">`;
      for (let h = 0; h < 24; h++) {
        // dawn/dusk peaks
        const env =
          Math.exp(-((h - 6.5) ** 2) / 8) * 0.9 +
          Math.exp(-((h - 18.5) ** 2) / 10) * 0.55 +
          0.06;
        const dayBoost = (di === 5 || di === 6) ? 0.7 : 1;
        const v = clamp(env * dayBoost * (0.7 + r() * 0.6), 0, 1);
        const op = v;
        const bg = op < 0.05 ? 'var(--surface-2)' : `color-mix(in oklch, ${accent} ${Math.min(85, op * 90).toFixed(0)}%, var(--surface-2))`;
        html += `<div style="aspect-ratio:1;border-radius:2px;background:${bg};"></div>`;
      }
      html += '</div></div>';
    });
    el.innerHTML = html;
  }

  // ── Polar dawn-chorus ribbons (TRUE polar arcs) ─────────────────────
  function renderPolar(el) {
    const SIZE = 520;
    const cx = SIZE / 2, cy = SIZE / 2;
    const ringMin = 76, ringMax = 222;
    const rise = +el.getAttribute('data-rise') || 5.35;
    const set = +el.getAttribute('data-set') || 20.13;
    const now = +el.getAttribute('data-now') || 6.7;
    const r0 = rng(+el.getAttribute('data-seed') || 11);

    // Species list — peak hour, color, name, short
    // Spreads are tight (1.8–3.6) so peaks read sharply rather than blurring.
    const SPECIES = [
      { name: 'American Robin',         short: 'AMRO', color: 'oklch(60% 0.12 230)', peak: 6.0, spread: 2.2, mult: 1.0,  dusk: 0.18 },
      { name: 'Blue Jay',               short: 'BLJA', color: 'var(--moss)',         peak: 6.5, spread: 2.6, mult: 0.92, dusk: 0.22 },
      { name: 'Black-capped Chickadee', short: 'BCCH', color: 'oklch(58% 0.16 280)', peak: 7.0, spread: 2.4, mult: 0.85, dusk: 0.15 },
      { name: 'Northern Cardinal',      short: 'NOCA', color: 'oklch(55% 0.14 50)',  peak: 5.8, spread: 1.9, mult: 0.78, dusk: 0.30 },
      { name: 'Tufted Titmouse',        short: 'TUTI', color: 'oklch(62% 0.10 130)', peak: 6.7, spread: 3.0, mult: 0.71, dusk: 0.18 },
      { name: 'Mourning Dove',          short: 'MODO', color: 'oklch(60% 0.06 90)',  peak: 8.2, spread: 3.6, mult: 0.64, dusk: 0.40 },
      { name: 'House Wren',             short: 'HOWR', color: 'oklch(58% 0.13 320)', peak: 5.4, spread: 1.8, mult: 0.55, dusk: 0.12 },
      { name: 'Eastern Phoebe',         short: 'EAPH', color: 'oklch(58% 0.10 175)', peak: 2.2, spread: 3.0, mult: 0.40, dusk: 0.05 },
    ];

    const ringStep = (ringMax - ringMin) / (SPECIES.length + 1);

    // Helpers
    const hourToAngle = h => (h / 24) * Math.PI * 2 - Math.PI / 2;
    const polar = (a, r) => [cx + r * Math.cos(a), cy + r * Math.sin(a)];

    // Compute 24-hr profile: primary Gaussian + small secondary at dusk + jitter.
    function profile(sp) {
      const out = new Array(24);
      for (let h = 0; h < 24; h++) {
        let d1 = h - sp.peak;
        if (d1 > 12) d1 -= 24;
        if (d1 < -12) d1 += 24;
        const main = Math.exp(-(d1 * d1) / (2 * sp.spread * sp.spread)) * sp.mult;
        // Secondary dusk peak at peak + 12 (rotated half-day) with same spread
        let d2 = h - ((sp.peak + 12) % 24);
        if (d2 > 12) d2 -= 24;
        if (d2 < -12) d2 += 24;
        const dusk = Math.exp(-(d2 * d2) / (2 * (sp.spread + 0.5) ** 2)) * sp.mult * (sp.dusk || 0);
        const jitter = (r0() - 0.5) * 0.18;
        out[h] = clamp(main + dusk + jitter, 0, 1);
      }
      return out;
    }

    let svg = '';
    svg += `<svg viewBox="0 0 ${SIZE} ${SIZE}" width="100%" height="100%" style="max-width:540px;max-height:540px;display:block;" preserveAspectRatio="xMidYMid meet">`;

    // Defs — radial gradients per species, more luminous towards outer
    svg += '<defs>';
    SPECIES.forEach((sp, i) => {
      svg += `<linearGradient id="ribGrad${i}" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="${sp.color}" stop-opacity="0.7"/><stop offset="100%" stop-color="${sp.color}" stop-opacity="0.25"/></linearGradient>`;
    });
    svg += '</defs>';

    // Night wedge
    const nightStart = hourToAngle(set);
    const nightEnd = hourToAngle(rise + 24);
    const [nx1, ny1] = polar(nightStart, ringMax + 14);
    const [nx2, ny2] = polar(nightEnd, ringMax + 14);
    const ns = (nightEnd - nightStart) % (Math.PI * 2);
    const nLarge = ns > Math.PI ? 1 : 0;
    svg += `<path d="M${cx},${cy} L${nx1.toFixed(2)},${ny1.toFixed(2)} A${ringMax + 14},${ringMax + 14} 0 ${nLarge} 1 ${nx2.toFixed(2)},${ny2.toFixed(2)} Z" fill="var(--night)" fill-opacity="0.05"/>`;

    // Dawn band (rise to rise + 2.7)
    const dStart = hourToAngle(rise);
    const dEnd = hourToAngle(rise + 2.7);
    const [dx0, dy0] = polar(dStart, ringMax + 14);
    const [dx1, dy1] = polar(dEnd, ringMax + 14);
    svg += `<path d="M${cx},${cy} L${dx0.toFixed(2)},${dy0.toFixed(2)} A${ringMax + 14},${ringMax + 14} 0 0 1 ${dx1.toFixed(2)},${dy1.toFixed(2)} Z" fill="var(--dawn)" fill-opacity="0.10"/>`;

    // Hour ring tick marks
    for (let h = 0; h < 24; h++) {
      const a = hourToAngle(h);
      const big = h % 6 === 0;
      const [tx1, ty1] = polar(a, ringMax + 6);
      const [tx2, ty2] = polar(a, ringMax + (big ? 14 : 10));
      svg += `<line x1="${tx1.toFixed(2)}" y1="${ty1.toFixed(2)}" x2="${tx2.toFixed(2)}" y2="${ty2.toFixed(2)}" stroke="${big ? 'var(--fg-3)' : 'var(--border)'}" stroke-width="${big ? 1.0 : 0.5}"/>`;
    }
    [0, 3, 6, 9, 12, 15, 18, 21].forEach(h => {
      const a = hourToAngle(h);
      const [lx, ly] = polar(a, ringMax + 26);
      const label = h === 0 ? '12a' : h === 12 ? '12p' : h < 12 ? `${h}a` : `${h - 12}p`;
      svg += `<text x="${lx.toFixed(2)}" y="${ly.toFixed(2)}" text-anchor="middle" dominant-baseline="central" font-family="JetBrains Mono" font-size="11" fill="var(--fg-3)">${label}</text>`;
    });

    // Ribbons — outer = highest mult
    SPECIES.forEach((sp, i) => {
      const baseR = ringMax - (i + 1) * ringStep;
      // Tight amp keeps each ribbon inside its row — hourly variation reads cleanly.
      const amp = ringStep * 0.62;
      const prof = profile(sp);
      const SUB = 8; // finer subdiv so curves stay smooth at tight amp
      const nPts = 24 * SUB;
      const outer = [], inner = [];
      for (let k = 0; k <= nPts; k++) {
        const h = (k / SUB) % 24;
        const f = Math.floor(h) % 24;
        const t = h - Math.floor(h);
        const v = prof[f] * (1 - t) + prof[(f + 1) % 24] * t;
        const a = hourToAngle(h);
        const rOuter = baseR + v * amp * 1.0;
        const rInner = baseR - v * amp * 0.18;
        outer.push(polar(a, rOuter));
        inner.push(polar(a, rInner));
      }
      // Faint baseline ring so even species with no signal at an hour read as 'visited'.
      svg += `<circle cx="${cx}" cy="${cy}" r="${baseR.toFixed(2)}" fill="none" stroke="${sp.color}" stroke-opacity="0.10" stroke-width="0.5" stroke-dasharray="1 2"/>`;
      let d = `M${outer[0][0].toFixed(2)},${outer[0][1].toFixed(2)}`;
      for (let k = 1; k < outer.length; k++) d += ` L${outer[k][0].toFixed(2)},${outer[k][1].toFixed(2)}`;
      for (let k = inner.length - 1; k >= 0; k--) d += ` L${inner[k][0].toFixed(2)},${inner[k][1].toFixed(2)}`;
      d += ' Z';
      svg += `<path d="${d}" fill="url(#ribGrad${i})" stroke="${sp.color}" stroke-opacity="0.90" stroke-width="0.9" stroke-linejoin="round"/>`;

      // Peak marker dot
      const peakA = hourToAngle(sp.peak);
      const [px, py] = polar(peakA, baseR + sp.mult * amp * 1.0);
      svg += `<circle cx="${px.toFixed(2)}" cy="${py.toFixed(2)}" r="2.2" fill="${sp.color}" stroke="var(--surface)" stroke-width="0.8"/>`;
    });

    // Sun markers
    const [sxR, syR] = polar(hourToAngle(rise), ringMax + 14);
    svg += `<circle cx="${sxR.toFixed(2)}" cy="${syR.toFixed(2)}" r="4" fill="var(--dawn)"/>`;
    svg += `<text x="${(sxR + 10).toFixed(2)}" y="${(syR + 4).toFixed(2)}" font-family="JetBrains Mono" font-size="10" fill="var(--fg-3)">☼ ${fmtH(rise)}</text>`;
    const [sxS, sySV] = polar(hourToAngle(set), ringMax + 14);
    svg += `<circle cx="${sxS.toFixed(2)}" cy="${sySV.toFixed(2)}" r="4" fill="var(--dawn-ink)"/>`;
    svg += `<text x="${(sxS - 8).toFixed(2)}" y="${(sySV + 4).toFixed(2)}" text-anchor="end" font-family="JetBrains Mono" font-size="10" fill="var(--fg-3)">☾ ${fmtH(set)}</text>`;

    // Center disc
    svg += `<circle cx="${cx}" cy="${cy}" r="${ringMin - 10}" fill="var(--surface)" stroke="var(--hairline)"/>`;
    svg += `<text x="${cx}" y="${cy - 8}" text-anchor="middle" font-family="Instrument Serif" font-size="16" fill="var(--fg-3)">chorus</text>`;
    svg += `<text x="${cx}" y="${cy + 12}" text-anchor="middle" font-family="JetBrains Mono" font-size="11" fill="var(--fg-2)">24 h · 60 days</text>`;

    // Now hand
    const nowA = hourToAngle(now);
    const [hx1, hy1] = polar(nowA, ringMin - 4);
    const [hx2, hy2] = polar(nowA, ringMax + 14);
    svg += `<line x1="${hx1.toFixed(2)}" y1="${hy1.toFixed(2)}" x2="${hx2.toFixed(2)}" y2="${hy2.toFixed(2)}" stroke="var(--fg)" stroke-width="1.4" stroke-dasharray="2 3"/>`;
    svg += `<circle cx="${hx2.toFixed(2)}" cy="${hy2.toFixed(2)}" r="3.5" fill="var(--fg)"/>`;

    svg += '</svg>';
    el.innerHTML = svg;

    function fmtH(h) {
      const m = Math.round((h % 1) * 60);
      const hh = Math.floor(h);
      return `${String(hh).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
    }
  }

  // ── Migration ridgeline ─────────────────────────────────────────────
  function renderRidge(el) {
    const W = 1240, H = 360;
    const PAD_L = 152, PAD_R = 24, PAD_T = 28, PAD_B = 36;
    const innerW = W - PAD_L - PAD_R, innerH = H - PAD_T - PAD_B;
    const weeks = 52;
    const xOf = w => PAD_L + (w / (weeks - 1)) * innerW;

    const SPECIES = [
      { name: 'Tree Swallow',           short: 'TRES', color: 'oklch(60% 0.12 230)', peak: 12, spread: 3.0, mult: 0.94, mode: 'spring' },
      { name: 'Yellow-rumped Warbler',  short: 'YRWA', color: 'oklch(72% 0.16 85)',  peak: 18, spread: 2.4, mult: 1.00, mode: 'spring' },
      { name: 'Magnolia Warbler',       short: 'MAWA', color: 'oklch(58% 0.13 110)', peak: 20, spread: 1.8, mult: 0.88, mode: 'spring' },
      { name: 'Wood Thrush',            short: 'WOTH', color: 'oklch(56% 0.14 160)', peak: 22, spread: 2.6, mult: 0.78, mode: 'summer' },
      { name: 'Scarlet Tanager',        short: 'SCTA', color: 'oklch(55% 0.18 28)',  peak: 24, spread: 2.0, mult: 0.66, mode: 'summer' },
      { name: 'Rose-breasted Grosbeak', short: 'RBGR', color: 'oklch(50% 0.16 20)',  peak: 19, spread: 1.6, mult: 0.60, mode: 'spring' },
      { name: 'Common Nighthawk',       short: 'CONI', color: 'oklch(58% 0.14 280)', peak: 36, spread: 2.8, mult: 0.72, mode: 'fall' },
      { name: 'White-throated Sparrow', short: 'WTSP', color: 'oklch(58% 0.14 320)', peak: 42, spread: 3.0, mult: 0.66, mode: 'fall' },
    ];

    const todayWeek = 21;
    const rowH = innerH / SPECIES.length;

    let svg = `<svg viewBox="0 0 ${W} ${H}" width="100%" height="auto" preserveAspectRatio="none" style="display:block;">`;

    // Background season bands
    svg += `<rect x="${xOf(8)}" y="${PAD_T}" width="${xOf(20) - xOf(8)}" height="${innerH}" fill="var(--moss-soft)" fill-opacity="0.35"/>`;
    svg += `<text x="${xOf(14)}" y="${PAD_T + 13}" text-anchor="middle" font-family="JetBrains Mono" font-size="10" fill="var(--moss-ink)">spring migration</text>`;
    svg += `<rect x="${xOf(34)}" y="${PAD_T}" width="${xOf(44) - xOf(34)}" height="${innerH}" fill="var(--dawn-soft)" fill-opacity="0.45"/>`;
    svg += `<text x="${xOf(39)}" y="${PAD_T + 13}" text-anchor="middle" font-family="JetBrains Mono" font-size="10" fill="var(--dawn-ink)">fall migration</text>`;

    // Month gridlines
    const MONTHS = [['Jan',0],['Feb',4],['Mar',9],['Apr',13],['May',17],['Jun',22],['Jul',26],['Aug',30],['Sep',35],['Oct',39],['Nov',44],['Dec',48]];
    MONTHS.forEach(([m, w]) => {
      svg += `<line x1="${xOf(w)}" y1="${PAD_T}" x2="${xOf(w)}" y2="${PAD_T + innerH}" stroke="var(--hairline)" stroke-width="0.5"/>`;
      svg += `<text x="${xOf(w)}" y="${PAD_T + innerH + 22}" text-anchor="middle" font-family="JetBrains Mono" font-size="11" fill="var(--fg-3)">${m}</text>`;
    });

    SPECIES.forEach((sp, i) => {
      const yBase = PAD_T + (i + 1) * rowH - 8;
      const amp = rowH * 0.92;

      // Compute weekly profile
      const profile = new Array(weeks);
      for (let w = 0; w < weeks; w++) {
        let d = w - sp.peak;
        if (d > weeks / 2) d -= weeks;
        if (d < -weeks / 2) d += weeks;
        profile[w] = Math.exp(-(d * d) / (2 * sp.spread * sp.spread)) * sp.mult;
      }

      // Build smooth curve (Catmull-Rom-ish via 4x subdivision)
      const SUB = 4;
      const path = [];
      for (let k = 0; k <= (weeks - 1) * SUB; k++) {
        const w = k / SUB;
        const f = Math.floor(w);
        const t = w - f;
        const a = profile[Math.max(0, f - 1)] ?? profile[0];
        const b = profile[f] ?? 0;
        const c = profile[Math.min(weeks - 1, f + 1)] ?? 0;
        const dd = profile[Math.min(weeks - 1, f + 2)] ?? 0;
        // Catmull-Rom
        const v = 0.5 * ((2 * b) + (-a + c) * t + (2 * a - 5 * b + 4 * c - dd) * t * t + (-a + 3 * b - 3 * c + dd) * t * t * t);
        path.push([xOf(w), yBase - clamp(v, 0, 1) * amp]);
      }
      let dStr = `M${path[0][0].toFixed(2)},${path[0][1].toFixed(2)}`;
      for (let k = 1; k < path.length; k++) dStr += ` L${path[k][0].toFixed(2)},${path[k][1].toFixed(2)}`;
      const dArea = `${dStr} L${xOf(weeks - 1).toFixed(2)},${yBase} L${xOf(0).toFixed(2)},${yBase} Z`;

      // Baseline
      svg += `<line x1="${xOf(0)}" y1="${yBase}" x2="${xOf(weeks - 1)}" y2="${yBase}" stroke="var(--hairline)"/>`;

      // Gradient
      const gid = `rg-${i}`;
      svg += `<defs><linearGradient id="${gid}" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="${sp.color}" stop-opacity="0.55"/><stop offset="100%" stop-color="${sp.color}" stop-opacity="0.05"/></linearGradient></defs>`;
      svg += `<path d="${dArea}" fill="url(#${gid})"/>`;
      svg += `<path d="${dStr}" fill="none" stroke="${sp.color}" stroke-width="1.5" stroke-linejoin="round"/>`;

      // Peak marker
      const px = xOf(sp.peak), py = yBase - sp.mult * amp;
      svg += `<line x1="${px}" y1="${py}" x2="${px}" y2="${yBase}" stroke="${sp.color}" stroke-width="0.8" stroke-dasharray="2 2" stroke-opacity="0.42"/>`;
      svg += `<circle cx="${px}" cy="${py}" r="3" fill="${sp.color}" stroke="var(--surface)" stroke-width="1"/>`;

      // Labels
      svg += `<text x="${PAD_L - 10}" y="${yBase - 6}" text-anchor="end" font-family="Inter Tight" font-weight="500" font-size="12" fill="var(--fg)">${sp.name}</text>`;
      svg += `<text x="${PAD_L - 10}" y="${yBase + 8}" text-anchor="end" font-family="JetBrains Mono" font-size="9.5" fill="var(--fg-3)">${sp.short} · peak w${sp.peak + 1}</text>`;
    });

    // Today marker
    svg += `<g>`;
    svg += `<line x1="${xOf(todayWeek)}" y1="${PAD_T}" x2="${xOf(todayWeek)}" y2="${PAD_T + innerH}" stroke="var(--fg)" stroke-width="1" stroke-dasharray="3 3"/>`;
    svg += `<rect x="${xOf(todayWeek) - 24}" y="${PAD_T - 2}" width="48" height="14" rx="3" fill="var(--fg)"/>`;
    svg += `<text x="${xOf(todayWeek)}" y="${PAD_T + 8}" text-anchor="middle" font-family="JetBrains Mono" font-size="10" fill="var(--bg)">today</text>`;
    svg += `</g>`;

    svg += '</svg>';
    el.innerHTML = svg;
  }

  // ── Hourly activity bars (24 hours, species-colored) ────────────────
  function renderHourlyBars(el) {
    const accent = el.getAttribute('data-accent') || 'var(--moss)';
    const r = rng(+el.getAttribute('data-seed') || 5);
    const peak = +el.getAttribute('data-peak') || 6;
    const spread = +el.getAttribute('data-spread') || 3.5;
    const hours = [];
    for (let h = 0; h < 24; h++) {
      let d = h - peak;
      if (d > 12) d -= 24;
      if (d < -12) d += 24;
      hours.push(Math.exp(-(d * d) / (2 * spread * spread)) * (0.7 + r() * 0.4));
    }
    const max = Math.max(...hours);
    let html = '<div style="height:100%;display:flex;flex-direction:column;">';
    html += '<div style="flex:1;display:flex;align-items:flex-end;gap:3px;">';
    hours.forEach(v => {
      const mix = 20 + (v / max) * 60;
      html += `<div style="flex:1;height:${((v / max) * 100).toFixed(0)}%;min-height:2px;background:color-mix(in oklch, ${accent} ${mix.toFixed(0)}%, var(--surface-2));border-radius:2px;"></div>`;
    });
    html += '</div>';
    html += '<div style="display:flex;justify-content:space-between;margin-top:6px;">';
    [0, 6, 12, 18, 23].forEach(h => {
      const label = h === 0 ? '12 a' : h === 12 ? '12 p' : h < 12 ? `${h} a` : `${h - 12} p`;
      html += `<span class="mono" style="font-size:10px;color:var(--fg-3);">${label}</span>`;
    });
    html += '</div></div>';
    el.innerHTML = html;
  }

  // ── Live signal canvas (animated soundwave) ─────────────────────────
  function renderLiveSignal(el) {
    const w = el.clientWidth || 480;
    const h = +el.getAttribute('data-h') || 80;
    const canvas = document.createElement('canvas');
    canvas.width = w * (window.devicePixelRatio || 1);
    canvas.height = h * (window.devicePixelRatio || 1);
    canvas.style.width = '100%';
    canvas.style.height = h + 'px';
    canvas.style.display = 'block';
    el.appendChild(canvas);
    const ctx = canvas.getContext('2d');
    ctx.scale(window.devicePixelRatio || 1, window.devicePixelRatio || 1);
    const accent = el.getAttribute('data-color') || 'oklch(48% 0.10 150)';
    const accentDark = el.getAttribute('data-color-dark') || 'oklch(80% 0.14 150)';
    let phase = 0;
    function tick() {
      const dark = document.documentElement.getAttribute('data-theme') === 'dark';
      const W = w, H = h;
      ctx.clearRect(0, 0, W, H);
      const bars = 96;
      const bw = (W - bars * 2) / bars;
      for (let i = 0; i < bars; i++) {
        const xt = i / bars;
        const env = Math.sin(xt * Math.PI);
        const v = env * (0.5 + 0.45 * Math.sin(i * 0.4 + phase) + 0.18 * Math.sin(i * 1.7 + phase * 1.5));
        const height = Math.max(4, Math.abs(v) * H * 0.85);
        const op = 0.35 + Math.abs(v) * 0.55;
        ctx.fillStyle = dark
          ? `oklch(80% 0.14 150 / ${op.toFixed(2)})`
          : `oklch(48% 0.10 150 / ${op.toFixed(2)})`;
        const xx = i * (bw + 2);
        ctx.fillRect(xx, (H - height) / 2, bw, height);
      }
      phase += 0.06;
      raf = requestAnimationFrame(tick);
    }
    let raf;
    tick();
  }

  // ── Day strip (24 hourly bars, multi-period coloring) ───────────────
  function renderDayStrip(el) {
    const r = rng(+el.getAttribute('data-seed') || 41);
    const hours = [];
    for (let h = 0; h < 24; h++) {
      const env =
        Math.exp(-((h - 6.5) ** 2) / 7) * 1.0 +
        Math.exp(-((h - 18.5) ** 2) / 9) * 0.55 +
        0.06;
      hours.push(env * (0.75 + r() * 0.5));
    }
    const max = Math.max(...hours);
    let html =
      '<svg viewBox="0 0 1200 96" width="100%" height="96" preserveAspectRatio="none">';
    html += `<line x1="0" y1="80" x2="1200" y2="80" stroke="var(--hairline)"/>`;
    hours.forEach((v, h) => {
      let fill = 'var(--fg-4)';
      let op = 0.6;
      if (h >= 5 && h <= 9) { fill = 'var(--moss)'; op = 0.7 + (v / max) * 0.25; }
      else if (h >= 16 && h <= 20) { fill = 'var(--dawn)'; op = 0.55 + (v / max) * 0.30; }
      else { fill = 'var(--moss)'; op = 0.30 + (v / max) * 0.30; }
      const hPx = (v / max) * 72;
      html += `<rect x="${h * 50 + 6}" y="${80 - hPx}" width="40" height="${hPx}" rx="1.5" fill="${fill}" opacity="${op.toFixed(2)}"/>`;
    });
    // Hour labels
    [0, 3, 6, 9, 12, 15, 18, 21].forEach(h => {
      const label = h === 0 ? '12 a' : h === 12 ? '12 p' : h < 12 ? `${h} a` : `${h - 12} p`;
      html += `<text x="${h * 50 + 26}" y="92" text-anchor="middle" font-family="JetBrains Mono" font-size="10" fill="var(--fg-3)">${label}</text>`;
    });
    // Period labels above (Dawn / Morning / Midday / Evening / Night)
    html += `<text x="370" y="14" text-anchor="middle" font-family="JetBrains Mono" font-size="10" letter-spacing="0.10em" fill="var(--moss-ink)" font-weight="500">DAWN</text>`;
    html += `<text x="930" y="14" text-anchor="middle" font-family="JetBrains Mono" font-size="10" letter-spacing="0.10em" fill="var(--dawn-ink)" font-weight="500">EVENING</text>`;
    html += '</svg>';
    el.innerHTML = html;
  }

  // ── Image placeholder — uses the production .bnb-photo hatched pattern ──
  //
  // PREVIEW ONLY. In production, the species-photo partial swaps in a real
  // <img> from the project's ImageCache (Wikimedia / Macaulay Library) when
  // available; this placeholder only renders when the cache hasn't fetched
  // the species yet. No anatomical pretense — confident "image pending"
  // state, not a failed bird drawing.
  function renderPhoto(el) {
    const accent = el.getAttribute('data-accent') || 'var(--moss)';
    const caption = el.getAttribute('data-caption') || '';
    const label = el.getAttribute('data-label') || '';

    el.innerHTML =
      `<!-- Diagonal-stripe field (the production .bnb-photo pattern) -->
      <div class="bnb-photo" style="position:absolute;inset:0;border-radius:inherit;"></div>

      <!-- Subtle wash so the centerpiece reads -->
      <div style="position:absolute;inset:0;background:
        radial-gradient(ellipse 60% 50% at 50% 50%, color-mix(in oklch, var(--bg) 86%, transparent) 0%, transparent 70%);"></div>

      <!-- Centered 'image pending' glyph + species hint -->
      <div style="position:absolute;inset:0;display:flex;flex-direction:column;
                  align-items:center;justify-content:center;gap:10px;pointer-events:none;">
        <svg width="36" height="36" viewBox="0 0 24 24" aria-hidden="true"
             style="opacity:0.55;color:var(--fg-3);">
          <rect x="3" y="5" width="18" height="14" rx="2"
                fill="none" stroke="currentColor" stroke-width="1.2"/>
          <circle cx="12" cy="12.5" r="3.2"
                  fill="none" stroke="currentColor" stroke-width="1.2"/>
          <path d="M8 8 L9.5 6.5 L14.5 6.5 L16 8"
                fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/>
        </svg>
        <div class="mono" style="font-size:10px;letter-spacing:0.16em;text-transform:uppercase;color:var(--fg-3);opacity:0.7;">${label || 'photograph pending'}</div>
        <div style="font-size:11px;color:var(--fg-4);opacity:0.7;max-width:240px;text-align:center;line-height:1.4;">
          Production wires this to the workspace's <span class="mono">ImageCache</span> — Wikimedia / Macaulay Library.
        </div>
      </div>

      ${caption ? `<div style="position:absolute;left:16px;bottom:16px;background:color-mix(in oklch, var(--bg) 92%, transparent);backdrop-filter:blur(10px);-webkit-backdrop-filter:blur(10px);border:0.5px solid var(--border-2);border-radius:10px;padding:10px 14px;display:flex;flex-direction:column;gap:2px;z-index:2;">${caption}</div>` : ''}`;

    // Suppress lint for unused param when no accent customization is needed.
    void accent;
  }

  // ── Spectrogram (stylized) ──────────────────────────────────────────
  function renderSpectrogram(el) {
    const accent = el.getAttribute('data-accent') || 'var(--moss-ink)';
    const label = el.getAttribute('data-label') || '';
    const r = rng(+el.getAttribute('data-seed') || 91);
    const W = 800, H = 200;
    let svg = `<svg viewBox="0 0 ${W} ${H}" width="100%" height="auto" preserveAspectRatio="none" style="background:var(--surface-2);border-radius:var(--r-md);display:block;">`;
    // Noise floor
    for (let i = 0; i < 220; i++) {
      const x = r() * W, y = 30 + r() * (H - 60);
      svg += `<rect x="${x.toFixed(0)}" y="${y.toFixed(0)}" width="2" height="2" fill="var(--fg-3)" opacity="${(0.06 + r() * 0.10).toFixed(2)}"/>`;
    }
    // Bird call chirps — 4–5 clusters
    const clusters = 5;
    for (let c = 0; c < clusters; c++) {
      const cx = 110 + c * 130 + r() * 24;
      const baseY = 60 + r() * 40;
      const dur = 18 + r() * 32;
      for (let i = 0; i < 16; i++) {
        const dy = (i / 16) * dur * (r() > 0.5 ? 1 : -1);
        svg += `<rect x="${(cx + i * 1.8).toFixed(1)}" y="${(baseY + dy).toFixed(1)}" width="2.4" height="${(dur * 0.7).toFixed(1)}" rx="1" fill="var(--fg-2)" opacity="${(0.4 + r() * 0.5).toFixed(2)}"/>`;
      }
    }
    // Bounding box
    svg += `<rect x="92" y="36" width="380" height="120" rx="3" fill="none" stroke="${accent}" stroke-width="1.4"/>`;
    if (label) {
      svg += `<rect x="92" y="20" width="${Math.min(220, label.length * 7)}" height="14" rx="3" fill="${accent}"/>`;
      svg += `<text x="${92 + Math.min(220, label.length * 7) / 2}" y="31" text-anchor="middle" font-family="JetBrains Mono" font-size="10" fill="var(--bg)">${label}</text>`;
    }
    // Axes labels
    svg += `<text x="10" y="20" font-family="JetBrains Mono" font-size="10" fill="var(--fg-3)">8k</text>`;
    svg += `<text x="10" y="100" font-family="JetBrains Mono" font-size="10" fill="var(--fg-3)">4k</text>`;
    svg += `<text x="10" y="188" font-family="JetBrains Mono" font-size="10" fill="var(--fg-3)">0</text>`;
    svg += `<text x="100" y="196" font-family="JetBrains Mono" font-size="10" fill="var(--fg-3)">0.0s</text>`;
    svg += `<text x="400" y="196" font-family="JetBrains Mono" font-size="10" fill="var(--fg-3)">1.5s</text>`;
    svg += `<text x="780" y="196" text-anchor="end" font-family="JetBrains Mono" font-size="10" fill="var(--fg-3)">3.0s</text>`;
    svg += '</svg>';
    el.innerHTML = svg;
  }

  // ── Audio scrubber (no real audio — pure visual) ────────────────────
  function renderScrubber(el) {
    const accent = el.getAttribute('data-accent') || 'var(--moss)';
    const dur = el.getAttribute('data-duration') || '3.0';
    const cur = +el.getAttribute('data-current') || 1.2;
    const total = +dur;
    const pct = (cur / total) * 100;
    el.innerHTML =
      `<div style="display:flex;align-items:center;gap:12px;width:100%;">
        <button style="width:36px;height:36px;border-radius:50%;border:0.5px solid var(--border);background:var(--surface);display:inline-flex;align-items:center;justify-content:center;cursor:pointer;color:var(--fg);">▶</button>
        <span class="mono" style="font-size:11px;color:var(--fg-3);min-width:38px;">${cur.toFixed(1)}s</span>
        <div style="flex:1;height:24px;display:flex;align-items:center;position:relative;">
          <span data-wave="" data-seed="${+el.getAttribute('data-seed') || 8}" data-bars="80" data-color="${accent}"></span>
          <div style="position:absolute;left:${pct.toFixed(1)}%;top:0;bottom:0;width:1.5px;background:var(--fg);"></div>
        </div>
        <span class="mono" style="font-size:11px;color:var(--fg-3);min-width:38px;text-align:right;">${dur}s</span>
      </div>`;
    // Recurse waveform inside
    el.querySelectorAll('[data-wave]').forEach(renderWave);
  }

  // ── Confidence bar ──────────────────────────────────────────────────
  function renderConf(el) {
    const v = +el.getAttribute('data-value') || 0.85;
    const w = +el.getAttribute('data-width') || 56;
    const color = v >= 0.85 ? 'var(--moss)' : v >= 0.6 ? 'var(--dawn)' : 'var(--rare)';
    el.innerHTML =
      `<span style="display:inline-flex;align-items:center;gap:8px;">
        <span style="display:inline-block;width:${w}px;height:4px;background:var(--bg-2);border-radius:2px;overflow:hidden;">
          <span style="display:block;width:${(v * 100).toFixed(0)}%;height:100%;background:${color};"></span>
        </span>
        <span class="mono tabular" style="font-size:11.5px;color:var(--fg-2);min-width:30px;">${v.toFixed(2)}</span>
      </span>`;
  }

  // ── Run all enhancers ───────────────────────────────────────────────
  function runAll(scope) {
    const root = scope || document;
    root.querySelectorAll('[data-spark]').forEach(renderSpark);
    root.querySelectorAll('[data-wave]').forEach(renderWave);
    root.querySelectorAll('[data-clock]').forEach(renderClock);
    root.querySelectorAll('[data-week-heat]').forEach(renderWeekHeat);
    root.querySelectorAll('[data-mini-heat]').forEach(renderMiniHeat);
    root.querySelectorAll('[data-polar]').forEach(renderPolar);
    root.querySelectorAll('[data-ridge]').forEach(renderRidge);
    root.querySelectorAll('[data-hourly-bars]').forEach(renderHourlyBars);
    root.querySelectorAll('[data-live]').forEach(renderLiveSignal);
    root.querySelectorAll('[data-day-strip]').forEach(renderDayStrip);
    root.querySelectorAll('[data-photo]').forEach(renderPhoto);
    root.querySelectorAll('[data-spectrogram]').forEach(renderSpectrogram);
    root.querySelectorAll('[data-scrubber]').forEach(renderScrubber);
    root.querySelectorAll('[data-conf]').forEach(renderConf);
  }

  // Public API
  window.BNBPreview = { runAll };
  document.addEventListener('DOMContentLoaded', () => runAll());
})();
