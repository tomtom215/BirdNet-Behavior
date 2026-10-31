// Who led each month: a rank-over-time ("bump") chart linked to a leaderboard of true counts.
//
// The question is "which birds led each month of the year, and when did that change?". Two halves
// answer it, and each is honest about what it can and cannot say:
//
//   * The LINES are ranks. One line per bird, month by month, 1st at the top. A rank is an order,
//     not a distance: it says who was ahead and hides by how much. So the chart draws no axis of
//     numbers, the rows are labelled "1st, 2nd, ...", and a curve between two months only joins
//     two ranks (it is a smoothstep that cannot go above the higher rank or below the lower one).
//   * The BARS are counts. They start at zero and share ONE scale across all twelve months (the
//     year's busiest single value, rounded up), so a bar in March can be compared with one in
//     October. They are the number the rank hides.
//
// A month with no sightings of a bird is a real zero: that bird is unranked that month, its line
// BREAKS (a gap, not a plunge to last place) and the leaderboard lists it as "not heard". A month
// that has not happened yet is null and is hatched; it is never drawn as a zero. The running month
// is marked "so far": its dots are hollow and the last segment of each line into it is dashed,
// because its ranks can still change.
//
// Everything here is DOM and SVG (no canvas, no library): colours come from CSS classes that name
// the app's chart tokens, so a theme change needs no repaint code. Strict CSP: no inline style
// attributes, no eval; the only style writes are CSSOM (`el.style.transform`, custom properties).
// Text from the data (bird names) only ever goes through `textContent`.
//
// Reduced motion (the OS, or the operator's Motion setting) means: no wipe-in, no glide, no Play
// button. The scrubber, the buttons and the keys still work and the state simply jumps.
//
// The pure parts (ranking, curves, scales, hit-testing, label placement, sentences) are exported
// and tested under node by `seasons.test.mjs`; the file imports without a DOM.

export const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
export const MONTH_NAMES = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
export const MAX_SPECIES = 8;
const N_MONTHS = 12;

// ── pure: words and numbers ─────────────────────────────────────────────────

/** `1st`, `2nd`, `3rd`, `4th`, `11th`, `21st`. */
export function ordinal(n) {
  const v = n % 100;
  if (v >= 11 && v <= 13) return `${n}th`;
  switch (n % 10) {
    case 1: return `${n}st`;
    case 2: return `${n}nd`;
    case 3: return `${n}rd`;
    default: return `${n}th`;
  }
}

const WORDS = ["zeroth", "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth"];
/** `first` .. `tenth`, then `11th` and on. */
export function ordinalWord(n) {
  return WORDS[n] || ordinal(n);
}

/** 1234567 as `1,234,567`: the true number, as the bars print it. */
export function formatCount(n) {
  const v = Math.round(Math.abs(Number(n) || 0));
  return (n < 0 ? "-" : "") + String(v).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

function trim(v) {
  return String(Math.round(v * 100) / 100);
}

/** A tick label: `0`, `500`, `1,500`, `20k`, `1.5M`. Only for the axis; bars print the whole number. */
export function formatCompact(n) {
  const v = Math.abs(Number(n) || 0);
  if (v < 10000) return formatCount(v);
  if (v < 1e6) return `${trim(v / 1e3)}k`;
  return `${trim(v / 1e6)}M`;
}

/** `A`, `A and B`, `A, B and C`. */
export function joinNames(list) {
  if (list.length <= 1) return list[0] || "";
  return `${list.slice(0, -1).join(", ")} and ${list[list.length - 1]}`;
}

/** The name cut to `max` characters with an ellipsis (the full name stays in the readout and the bars). */
export function truncateName(name, max) {
  const s = String(name);
  if (s.length <= max) return s;
  if (max <= 1) return "…";
  return `${s.slice(0, max - 1).replace(/[\s\-–]+$/, "")}…`;
}

/**
 * A name to fit `max` characters, the way a field notebook does it: the first word of a two-word name
 * shrinks to its initial and the kind of bird stays whole ("Northern Cardinal" becomes "N. Cardinal").
 * Only when that is still too long is it cut with an ellipsis. The whole name is always in the
 * leaderboard and the readout.
 */
export function shortName(name, max) {
  const s = String(name).trim();
  if (s.length <= max) return s;
  const words = s.split(/\s+/);
  const initial = (w) => {
    const m = w.match(/[\p{L}\p{N}]/u);
    return m ? `${m[0]}.` : w;
  };
  // Two words: the first shrinks ("N. Cardinal"). More than that, initials read as a code, so the
  // name is cut where there is room for most of it, and abbreviated first only where there is not.
  if (words.length === 2) {
    const cand = `${initial(words[0])} ${words[1]}`;
    if (cand.length <= max) return cand;
    return truncateName(cand, max);
  }
  if (max >= 16 || words.length < 2) return truncateName(s, max);
  return truncateName(`${initial(words[0])} ${words.slice(1).join(" ")}`, max);
}

const STEPS = [1, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10];

/**
 * The top of the shared bar scale: `v` (the year's busiest single value) rounded UP to a number
 * the axis can divide cleanly (1, 1.5, 2, 2.5, 3, 4, 5, 6, 8 times a power of ten). Always a whole
 * number, never below `v`, never below 1 so an all-zero year still has a scale.
 */
export function niceCeil(v) {
  if (!(v > 1)) return 1;
  const e = Math.floor(Math.log10(v));
  const base = Math.pow(10, e);
  for (const s of STEPS) {
    const c = s * base;
    if (c >= v - 1e-9 && Number.isInteger(c)) return c;
  }
  return 10 * base;
}

/** Whole-number axis ticks from 0 to `max` (a `niceCeil` value): at most five intervals, no fractions. */
export function niceTicks(max) {
  const top = niceCeil(max);
  for (let k = 0; k < 12; k++) {
    for (const m of [1, 2, 5]) {
      const step = m * Math.pow(10, k);
      const n = top / step;
      if (n <= 5 && Number.isInteger(n)) {
        const out = [];
        for (let i = 0; i <= n; i++) out.push(i * step);
        return out;
      }
    }
  }
  return [0, top];
}

// ── pure: the data ──────────────────────────────────────────────────────────

/**
 * The server's JSON (a string or an object) as a model, or null when there is nothing to draw.
 * Never throws. Months after the running month are forced to null (they have not happened); a
 * non-negative integer is a real count (0 included); anything else inside a happened month is
 * null, "no reading", and is neither ranked nor called a zero.
 *
 * `{ year, thisMonth, last, species: [{ name, slot, counts[12], total }] }` where `last` is the
 * index of the newest month that has happened and `thisMonth` is the running month (or null).
 */
export function parseData(raw) {
  let d = raw;
  if (typeof d === "string") {
    try {
      d = JSON.parse(d);
    } catch (e) {
      return null;
    }
  }
  if (!d || typeof d !== "object" || !Array.isArray(d.species)) return null;
  let tm = d.thisMonth === undefined || d.thisMonth === null ? null : Math.trunc(Number(d.thisMonth));
  if (tm !== null && (!Number.isFinite(tm) || tm > N_MONTHS - 1)) tm = null;
  if (tm !== null && tm < 0) tm = 0;
  const last = tm === null ? N_MONTHS - 1 : tm;
  const species = [];
  for (const sp of d.species) {
    if (species.length >= MAX_SPECIES) break;
    if (!sp || typeof sp !== "object" || !Array.isArray(sp.counts)) continue;
    const counts = [];
    for (let m = 0; m < N_MONTHS; m++) {
      const v = sp.counts[m];
      if (m > last || v === null || v === undefined || typeof v === "boolean") counts.push(null);
      else {
        const n = Number(v);
        counts.push(Number.isFinite(n) ? Math.max(0, Math.round(n)) : null);
      }
    }
    let slot = Math.trunc(Number(sp.slot));
    if (!(slot >= 1 && slot <= MAX_SPECIES)) slot = species.length + 1;
    const name = String(sp.name === undefined || sp.name === null ? "" : sp.name).trim() || "Unknown bird";
    species.push({ name, slot, counts, total: counts.reduce((a, c) => a + (c || 0), 0) });
  }
  if (!species.length) return null;
  const year = Number.isFinite(Number(d.year)) ? Math.trunc(Number(d.year)) : null;
  return { year, thisMonth: tm, last, species };
}

function byCount(model, m) {
  const sp = model.species;
  return (a, b) => {
    const d = (sp[b].counts[m] || 0) - (sp[a].counts[m] || 0);
    if (d) return d;
    if (sp[b].total !== sp[a].total) return sp[b].total - sp[a].total;
    if (sp[a].name !== sp[b].name) return sp[a].name < sp[b].name ? -1 : 1;
    return a - b;
  };
}

/**
 * `ranks[i][m]`: 1 for the bird heard most in month `m`, 2 for the next, and so on; 0 when the bird
 * was not heard that month (a real zero: unranked, the line breaks) or the month has not happened.
 * Ties on the count are broken by the year's total, then by name, so the order is the same every
 * time and does not depend on the order the server listed them in.
 */
export function computeRanks(model) {
  const n = model.species.length;
  const ranks = model.species.map(() => new Array(N_MONTHS).fill(0));
  for (let m = 0; m <= model.last; m++) {
    const heard = [];
    for (let i = 0; i < n; i++) if ((model.species[i].counts[m] || 0) > 0) heard.push(i);
    heard.sort(byCount(model, m));
    heard.forEach((i, k) => {
      ranks[i][m] = k + 1;
    });
  }
  return ranks;
}

/** The sum of the counts of the birds shown in month `m`. */
export function monthTotal(model, m) {
  let t = 0;
  for (const sp of model.species) t += sp.counts[m] || 0;
  return t;
}

/** The largest single count of any bird in any month: the top of the one shared bar scale, before rounding. */
export function busiest(model) {
  let b = 0;
  for (const sp of model.species) for (const c of sp.counts) if (c > b) b = c;
  return b;
}

/**
 * Where each bird stands on the leaderboard of month `m`: birds that were heard first, in rank order,
 * then the ones that were not, by year total (the leaderboard always has one row for every bird, so it
 * never changes height). `pos[i]` is the 0-based row of bird `i`; `heard` is how many were heard.
 */
export function boardPositions(model, ranks, m) {
  const n = model.species.length;
  const heard = [];
  const quiet = [];
  for (let i = 0; i < n; i++) (ranks[i][m] > 0 ? heard : quiet).push(i);
  heard.sort((a, b) => ranks[a][m] - ranks[b][m]);
  quiet.sort((a, b) => {
    if (model.species[b].total !== model.species[a].total) return model.species[b].total - model.species[a].total;
    if (model.species[a].name !== model.species[b].name) return model.species[a].name < model.species[b].name ? -1 : 1;
    return a - b;
  });
  const pos = new Array(n).fill(0);
  heard.concat(quiet).forEach((i, k) => {
    pos[i] = k;
  });
  return { pos, heard: heard.length, order: heard.concat(quiet) };
}

/** The birds heard in month `m`, as `{ idx: [...tied for first, in rank order], top }` (`top` 0 when none). */
export function leaderOf(model, ranks, m) {
  let top = 0;
  for (const sp of model.species) if ((sp.counts[m] || 0) > top) top = sp.counts[m];
  if (!top) return { top: 0, idx: [] };
  const idx = [];
  model.species.forEach((sp, i) => {
    if (sp.counts[m] === top) idx.push(i);
  });
  idx.sort((a, b) => ranks[a][m] - ranks[b][m]);
  return { top, idx };
}

/**
 * Who held 1st place, as runs: `[{ idx, from, to }]` over the months that have happened. A month in
 * which nothing was heard has no leader and does not break a run (nobody took over).
 */
export function leadRuns(model, ranks) {
  const runs = [];
  for (let m = 0; m <= model.last; m++) {
    let who = -1;
    for (let i = 0; i < model.species.length; i++) if (ranks[i][m] === 1) who = i;
    if (who < 0) continue;
    const prev = runs[runs.length - 1];
    if (prev && prev.idx === who) prev.to = m;
    else runs.push({ idx: who, from: m, to: m });
  }
  return runs;
}

/** `Apr` or `Apr to Jun`. */
export function rangeText(from, to) {
  return from === to ? MONTHS[from] : `${MONTHS[from]} to ${MONTHS[to]}`;
}

function soFarOf(model, m) {
  return model.thisMonth !== null && m === model.thisMonth;
}

/** `Blue Jay led October so far with 132 of the 410 heard.` (ties, one bird, no birds: each in words). */
export function leaderSentence(model, ranks, m) {
  const name = MONTH_NAMES[m];
  const sf = soFarOf(model, m) ? " so far" : "";
  const { top, idx } = leaderOf(model, ranks, m);
  if (model.species.length === 1) {
    const only = model.species[0];
    return top ? `${only.name} was heard ${times(top)} in ${name}${sf}.` : `${only.name} was not heard in ${name}${sf}.`;
  }
  if (!top) return `None of these birds were heard in ${name}${sf}.`;
  const total = monthTotal(model, m);
  const names = idx.map((i) => model.species[i].name);
  if (idx.length > 1) {
    return `${joinNames(names)} tied for the lead in ${name}${sf}, with ${formatCount(top)} each.`;
  }
  const heard = model.species.filter((sp) => (sp.counts[m] || 0) > 0).length;
  if (heard === 1) return `${names[0]} was the only one of these birds heard in ${name}${sf}: ${formatCount(top)}.`;
  return `${names[0]} led ${name}${sf} with ${formatCount(top)} of the ${formatCount(total)} heard.`;
}

/** The whole story of the lead in a line: how often it changed, and who held it when. */
export function yearSentence(model, ranks) {
  const runs = leadRuns(model, ranks);
  if (!runs.length) return "";
  const nm = (r) => model.species[r.idx].name;
  const months = model.last + 1;
  const open = model.thisMonth !== null ? " so far" : "";
  if (runs.length === 1) {
    if (months === 1) return `${nm(runs[0])} led ${MONTH_NAMES[runs[0].from]}${open}.`;
    let quiet = 0;
    for (let m = 0; m <= model.last; m++) if (!monthTotal(model, m)) quiet += 1;
    if (quiet) return `${nm(runs[0])} led every month in which any of these birds was heard.`;
    return `${nm(runs[0])} led every month${model.thisMonth === null ? " of the year" : " so far"}.`;
  }
  const changes = runs.length - 1;
  return `The lead changed ${changes === 1 ? "once" : changes === 2 ? "twice" : `${changes} times`}${open}.`;
}

/** For the picture's accessible name and the text under it. */
export function describeChart(model, ranks) {
  const n = model.species.length;
  const upTo = model.thisMonth !== null ? `, January to ${MONTH_NAMES[model.last]} so far` : ", January to December";
  const yr = model.year ? ` in ${model.year}` : "";
  return `Rank chart: how ${n === 1 ? "one bird" : `${n} birds`} ranked by how often they were heard, month by month${yr}${upTo}. ${yearSentence(model, ranks)}`.trim();
}

function times(c) {
  return c === 1 ? "1 time" : `${formatCount(c)} times`;
}

/** What one bird did over the year so far: its best rank, in which months, and in how many months it was heard. */
export function speciesSummary(model, ranks, i) {
  let best = 0;
  let bestMonths = [];
  let heard = 0;
  for (let m = 0; m <= model.last; m++) {
    const r = ranks[i][m];
    if (r > 0) heard += 1;
    if (r > 0 && (best === 0 || r < best)) {
      best = r;
      bestMonths = [m];
    } else if (r > 0 && r === best) bestMonths.push(m);
  }
  return { best, bestMonths, monthsHeard: heard, months: model.last + 1 };
}

/** One bird in one month, then its year, in words. */
export function speciesReading(model, ranks, i, m) {
  const sp = model.species[i];
  const sf = soFarOf(model, m) ? " so far" : "";
  const r = ranks[i][m];
  let s;
  if (r > 0) s = `${sp.name}: ${ordinalWord(r)} in ${MONTH_NAMES[m]}${sf}, heard ${times(sp.counts[m])}.`;
  else if (sp.counts[m] === null) s = `${sp.name}: no reading for ${MONTH_NAMES[m]}.`;
  else s = `${sp.name}: not heard in ${MONTH_NAMES[m]}${sf}.`;
  const sum = speciesSummary(model, ranks, i);
  if (sum.best > 0) {
    const where = sum.bestMonths.length <= 3 ? joinNames(sum.bestMonths.map((k) => MONTH_NAMES[k])) : `${sum.bestMonths.length} months`;
    s += ` Best: ${ordinalWord(sum.best)}, in ${where}.`;
    s += ` Heard in ${sum.monthsHeard} of ${sum.months} month${sum.months === 1 ? "" : "s"}${model.thisMonth !== null ? " so far" : ""}.`;
  }
  return s;
}

/** A whole month, in words, for the live region: `October, so far: Blue Jay first with 132, ...`. */
export function monthReading(model, ranks, m) {
  const head = `${MONTH_NAMES[m]}${soFarOf(model, m) ? ", so far" : ""}`;
  const { order, heard } = boardPositions(model, ranks, m);
  if (!heard) return `${head}: none of these birds were heard.`;
  const parts = [];
  for (let k = 0; k < heard; k++) {
    const i = order[k];
    parts.push(`${model.species[i].name} ${ordinalWord(ranks[i][m])} with ${formatCount(model.species[i].counts[m])}`);
  }
  let s = `${head}: ${parts.join(", ")}.`;
  if (heard < order.length) s += ` Not heard: ${joinNames(order.slice(heard).map((i) => model.species[i].name))}.`;
  return s;
}

// ── pure: curves ────────────────────────────────────────────────────────────

/**
 * The curve between two months: a cubic whose control points sit at the middle x with the height of
 * the nearer end. Its tangent is flat at both months, so it is monotone (a Fritsch-Carlson cubic with
 * zero slopes at the nodes): it stays between the two ranks and never overshoots one.
 */
export function bumpSegment(x0, y0, x1, y1) {
  const xm = (x0 + x1) / 2;
  return { x0, y0, c1x: xm, c1y: y0, c2x: xm, c2y: y1, x1, y1 };
}

/** `n + 1` points along a segment, for hit-testing and for tests. */
export function sampleSegment(seg, n = 16) {
  const out = [];
  for (let k = 0; k <= n; k++) {
    const t = k / n;
    const u = 1 - t;
    const a = u * u * u;
    const b = 3 * u * u * t;
    const c = 3 * u * t * t;
    const d = t * t * t;
    out.push({
      x: a * seg.x0 + b * seg.c1x + c * seg.c2x + d * seg.x1,
      y: a * seg.y0 + b * seg.c1y + c * seg.c2y + d * seg.y1,
    });
  }
  return out;
}

const r2 = (v) => Math.round(v * 100) / 100;

/** The SVG path through consecutive points (`[{ x, y }]`), curved between each pair. */
export function bumpPath(pts) {
  if (!pts.length) return "";
  let d = `M${r2(pts[0].x)} ${r2(pts[0].y)}`;
  for (let k = 1; k < pts.length; k++) {
    const s = bumpSegment(pts[k - 1].x, pts[k - 1].y, pts[k].x, pts[k].y);
    d += `C${r2(s.c1x)} ${r2(s.c1y)} ${r2(s.c2x)} ${r2(s.c2y)} ${r2(s.x1)} ${r2(s.y1)}`;
  }
  return d;
}

/** The runs of consecutive ranked months of one bird: `[[m, m+1, ...], ...]`. A zero month ends a run. */
export function rankRuns(rankRow, last) {
  const runs = [];
  let cur = null;
  for (let m = 0; m <= last; m++) {
    if (rankRow[m] > 0) {
      if (!cur) {
        cur = [];
        runs.push(cur);
      }
      cur.push(m);
    } else cur = null;
  }
  return runs;
}

// ── pure: layout, scrubber and hit-testing ──────────────────────────────────

/**
 * The geometry of the chart for a container `width` (px), a mode (`wide` or `narrow`, the phone
 * layout) and `n` birds. Months are 12 equal columns; month `m` has its dots at `x(m)`; rank `r` has
 * its row at `y(r)`. `labelW` is the room the names at the right edge may take (0: no names).
 *
 * Narrow differs on purpose, not by scale: taller rows (the phone has the height to spend and the
 * fingers need it), a thinner gutter, a deeper month strip to grab, and a name column that is
 * allowed to be short because the leaderboard under it carries the whole name.
 */
export function layout({ width, mode, n, labelW, last = 11 }) {
  const narrow = mode === "narrow";
  const left = narrow ? 30 : 36;
  const pitch = narrow ? (n >= 7 ? 46 : n >= 5 ? 52 : 58) : n >= 7 ? 42 : n >= 5 ? 48 : 56;
  const ribbonTop = 4;
  const ribbonH = 8;
  const plotTop = ribbonTop + ribbonH + (narrow ? 16 : 18);
  const plotBottom = plotTop + n * pitch;
  const axisTop = plotBottom + 8;
  const handleH = narrow ? 32 : 28;
  const H = axisTop + handleH + (narrow ? 24 : 22);
  const lastM = Math.max(0, Math.min(N_MONTHS - 1, last));
  let lw = Math.max(0, labelW || 0);
  let gap = lw > 0 ? 10 : 0;
  // The names sit just right of the newest month's dots. In a year that is not over, that is over the
  // hatched months that have not happened, so they need no margin of their own; otherwise the margin is
  // whatever the names need beyond the last column. Either way the figure is exactly `width` wide.
  const freeCols = N_MONTHS - 0.5 - lastM; // from the newest dot to the right edge of December
  const right = 6; // so the handle on December is not cut by the edge of the picture
  const minCol = narrow ? 18 : 24;
  let colW = (width - left - right) / N_MONTHS;
  let extra = 0;
  if (lw > 0 && gap + lw > freeCols * colW + right - 4) {
    colW = (width - left - gap - lw - 4) / (lastM + 0.5);
    if (colW < minCol) {
      // Too tight for the names: give them what is left, or none, and never push past the container.
      colW = Math.min(minCol, (width - left - right) / N_MONTHS);
      lw = Math.max(0, width - left - 4 - (lastM + 0.5) * colW - gap);
      if (lw < 40) {
        lw = 0;
        gap = 0;
        colW = (width - left - right) / N_MONTHS;
      }
    }
    extra = Math.max(0, width - left - right - N_MONTHS * colW);
  }
  const W = Math.round(left + N_MONTHS * colW + extra + right);
  return {
    mode,
    narrow,
    n,
    W,
    H,
    left,
    colW,
    pitch,
    ribbonTop,
    ribbonH,
    plotTop,
    plotBottom,
    plotRight: left + N_MONTHS * colW,
    axisTop,
    handleH,
    labelW: lw,
    x: (m) => left + (m + 0.5) * colW,
    y: (r) => plotTop + (r - 0.5) * pitch,
    bandX: (m) => left + m * colW,
  };
}

/** The month whose column holds x (px in chart units), clamped to the months that have happened. */
export function monthAt(px, geo, last) {
  const m = Math.floor((px - geo.left) / geo.colW);
  return Math.max(0, Math.min(last, Number.isFinite(m) ? m : 0));
}

function distToSegment(px, py, a, b) {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len2 = dx * dx + dy * dy;
  let t = len2 ? ((px - a.x) * dx + (py - a.y) * dy) / len2 : 0;
  t = Math.max(0, Math.min(1, t));
  return Math.hypot(px - (a.x + t * dx), py - (a.y + t * dy));
}

/**
 * The bird whose line (or end label) is nearest the point, if any is within `thr` px.
 * `polys[i]` is bird `i`'s polylines (`[[{x,y},...],...]`, one per run); `rects` are label boxes
 * `{ i, x0, x1, y0, y1 }` (inside one: distance 0). Returns `{ i, d }` or null.
 */
export function nearestSpecies(px, py, polys, rects, thr) {
  let best = null;
  for (const r of rects || []) {
    if (px >= r.x0 - 2 && px <= r.x1 + 2 && py >= r.y0 - 2 && py <= r.y1 + 2) {
      if (!best || best.d > 0) best = { i: r.i, d: 0 };
    }
  }
  for (let i = 0; i < polys.length; i++) {
    for (const poly of polys[i]) {
      if (poly.length === 1) {
        const d = Math.hypot(px - poly[0].x, py - poly[0].y);
        if (d <= thr && (!best || d < best.d)) best = { i, d };
        continue;
      }
      for (let k = 1; k < poly.length; k++) {
        const d = distToSegment(px, py, poly[k - 1], poly[k]);
        if (d <= thr && (!best || d < best.d)) best = { i, d };
      }
    }
  }
  return best;
}

/**
 * Put labels where they do not touch each other. `items`: `{ id, x, y, w, h, anchor: "start" | "middle" | "end" }`
 * in priority order (earlier ones keep their place; later ones move). Each may slide up or down by
 * whole label heights (or by the offsets in the item's own `shifts`, `[0]` for a label that must sit
 * exactly at its anchor or not at all); one that finds no free place inside `bounds` is `hidden` (the readout and the
 * leaderboard still name it). `obstacles` (`{ x0, x1, y0, y1 }`, the dots) are kept clear as well.
 * Returns new objects; the input is not changed.
 */
export function placeLabels(items, bounds, obstacles) {
  const placed = (obstacles || []).slice();
  const out = [];
  const rect = (it, y) => {
    const x0 = it.anchor === "end" ? it.x - it.w : it.anchor === "middle" ? it.x - it.w / 2 : it.x;
    return { x0, x1: x0 + it.w, y0: y - it.h / 2, y1: y + it.h / 2 };
  };
  const hit = (a, b) => a.x0 < b.x1 + 1 && b.x0 < a.x1 + 1 && a.y0 < b.y1 + 1 && b.y0 < a.y1 + 1;
  for (const it of items) {
    const step = it.h + 2;
    let chosen = null;
    for (const k of it.shifts || [0, 1, -1, 2, -2, 3, -3]) {
      const y = it.y + k * step;
      const r = rect(it, y);
      if (r.y0 < bounds.top || r.y1 > bounds.bottom) continue;
      if (r.x0 < bounds.left || r.x1 > bounds.right) continue;
      if (placed.some((p) => hit(p, r))) continue;
      chosen = { y, r };
      break;
    }
    if (chosen) {
      placed.push(chosen.r);
      out.push({ ...it, y: chosen.y, hidden: false, rect: chosen.r });
    } else out.push({ ...it, hidden: true, rect: null });
  }
  return out;
}

// ── the page ────────────────────────────────────────────────────────────────

let uid = 0;
const SVG_NS = "http://www.w3.org/2000/svg";

function reducedMotion() {
  return (
    (typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches) ||
    document.documentElement.getAttribute("data-motion") === "reduced"
  );
}

function h(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
}

function sv(tag, attrs, cls) {
  const n = document.createElementNS(SVG_NS, tag);
  if (cls) n.setAttribute("class", cls);
  if (attrs) for (const k of Object.keys(attrs)) n.setAttribute(k, attrs[k]);
  return n;
}

let measureCtx = null;
function textWidth(text, px, weight, family) {
  if (!measureCtx) {
    try {
      measureCtx = document.createElement("canvas").getContext("2d");
    } catch (e) {
      measureCtx = null;
    }
  }
  if (!measureCtx) return String(text).length * px * 0.56;
  measureCtx.font = `${weight} ${px}px ${family}`;
  return measureCtx.measureText(text).width;
}

/** A small inline icon drawn in `currentColor`. */
function icon(kind) {
  const s = sv("svg", { viewBox: "0 0 16 16", width: "16", height: "16", "aria-hidden": "true", focusable: "false" }, "ssn-ico");
  if (kind === "line") s.appendChild(sv("path", { d: "M1.5 12C5 12 5 4 8 4S11 12 14.5 12" }, "ssn-ico-s"));
  else if (kind === "bars") {
    s.appendChild(sv("rect", { x: "1.5", y: "2.5", width: "9", height: "3", rx: "1" }));
    s.appendChild(sv("rect", { x: "1.5", y: "6.5", width: "13", height: "3", rx: "1" }));
    s.appendChild(sv("rect", { x: "1.5", y: "10.5", width: "6", height: "3", rx: "1" }));
  } else if (kind === "play") s.appendChild(sv("path", { d: "M4.5 2.8v10.4L13 8z" }));
  else if (kind === "pause") {
    s.appendChild(sv("rect", { x: "3.5", y: "3", width: "3.2", height: "10", rx: "1" }));
    s.appendChild(sv("rect", { x: "9.3", y: "3", width: "3.2", height: "10", rx: "1" }));
  } else if (kind === "prev") s.appendChild(sv("path", { d: "M10 3L5 8l5 5" }, "ssn-ico-s"));
  else if (kind === "next") s.appendChild(sv("path", { d: "M6 3l5 5-5 5" }, "ssn-ico-s"));
  return s;
}


class Seasons {
  constructor(host) {
    this.host = host;
    this.id = `ssn-${++uid}`;
    this.model = parseData(host.getAttribute("data-ssn-json"));
    this.disposers = [];
    if (!this.model) {
      this.fail();
      return;
    }
    this.ranks = computeRanks(this.model);
    this.n = this.model.species.length;
    this.month = this.model.last;
    this.hover = -1;
    this.kbd = -1;
    this.pin = -1;
    this.ghost = -1;
    this.playing = false;
    this.finished = false;
    this.timer = 0;
    this.onScreen = true;
    this.revealed = false;
    this.geo = null;
    this.polys = [];
    this.rects = [];
    this.dots = [];
    this.groups = [];
    this.labels = [];
    this.drag = null;
    this.scaleMax = niceCeil(busiest(this.model));
    this.rowPx = 46;
    this.playState = "";
    // Until the figure first scrolls into view the lines are held back, then they draw in (once).
    if (!reducedMotion() && typeof IntersectionObserver === "function") host.classList.add("ssn-pre");
    this.shell();
    this.size(true);
    this.render();
    this.watch();
    host.setAttribute("data-ssn-ready", "");
  }

  /** The data is unusable: say so in words and leave the page whole. */
  fail() {
    this.host.textContent = "";
    this.host.appendChild(h("p", "bnb-meta ssn-msg", "The month-by-month ranking is not available just now."));
    this.host.setAttribute("data-ssn-ready", "");
  }

  // ── structure (built once) ──
  shell() {
    const m = this.model;
    const host = this.host;
    host.textContent = "";
    host.classList.add("ssn");
    const grid = h("div", "ssn-grid");

    // Left: the chart.
    const chart = h("section", "ssn-chart");
    const ph = h("header", "ssn-ph");
    const tag = h("span", "ssn-tag");
    tag.appendChild(icon("line"));
    tag.appendChild(document.createTextNode("Lines: rank"));
    ph.appendChild(tag);
    chart.appendChild(ph);
    chart.appendChild(
      h("p", "ssn-ph-s", (this.n === 1
        ? "Only one bird is shown, so it is always 1st when it was heard. A rank is an order, not a distance; the bars show how many."
        : `1st is the bird heard most that month, among these ${this.n}. A rank is an order, not a distance: it hides how far ahead a bird was, and the bars show that. The strip on top is who held 1st.`)),
    );
    this.plot = h("div", "ssn-plot");
    this.svg = sv("svg", { role: "img", tabindex: "0", focusable: "true" }, "ssn-svg");
    this.helpId = `${this.id}-help`;
    this.svg.setAttribute("aria-describedby", this.helpId);
    this.svg.setAttribute("aria-label", describeChart(m, this.ranks));
    this.plot.appendChild(this.svg);
    chart.appendChild(this.plot);
    this.yearEl = h("p", "ssn-year");
    chart.appendChild(this.yearEl);
    this.leadsEl = h("ul", "ssn-leads");
    chart.appendChild(this.leadsEl);
    this.readout = h("p", "ssn-readout bnb-meta");
    this.readout.setAttribute("aria-hidden", "true");
    this.readoutText = h("span", "ssn-readout-t");
    this.clearBtn = h("button", "btn ssn-clear", "Show all birds");
    this.clearBtn.type = "button";
    this.clearBtn.hidden = true;
    this.readout.appendChild(this.readoutText);
    chart.appendChild(this.readout);
    chart.appendChild(this.clearBtn);
    grid.appendChild(chart);

    // Right: the leaderboard of counts.
    const board = h("section", "ssn-board");
    const bph = h("header", "ssn-ph");
    const btag = h("span", "ssn-tag");
    btag.appendChild(icon("bars"));
    btag.appendChild(document.createTextNode("Bars: how many were heard"));
    bph.appendChild(btag);
    board.appendChild(bph);

    const mrow = h("div", "ssn-mrow");
    this.prevBtn = h("button", "ssn-step");
    this.prevBtn.type = "button";
    this.prevBtn.tabIndex = -1;
    this.prevBtn.setAttribute("aria-label", "Previous month");
    this.prevBtn.appendChild(icon("prev"));
    this.nextBtn = h("button", "ssn-step");
    this.nextBtn.type = "button";
    this.nextBtn.tabIndex = -1;
    this.nextBtn.setAttribute("aria-label", "Next month");
    this.nextBtn.appendChild(icon("next"));
    const mid = h("div", "ssn-mmid");
    this.monthEl = h("h3", "ssn-month");
    this.soFarEl = h("span", "ssn-sofar", "so far");
    mid.appendChild(this.monthEl);
    mid.appendChild(this.soFarEl);
    this.playBtn = h("button", "btn ssn-play");
    this.playBtn.type = "button";
    this.playIcon = h("span", "ssn-play-i");
    this.playLabel = h("span", "ssn-play-t", "Play");
    this.playBtn.appendChild(this.playIcon);
    this.playBtn.appendChild(this.playLabel);
    mrow.appendChild(this.prevBtn);
    mrow.appendChild(mid);
    mrow.appendChild(this.nextBtn);
    mrow.appendChild(this.playBtn);
    board.appendChild(mrow);

    const sum = h("div", "ssn-sum");
    this.noteEl = h("p", "ssn-note");
    sum.appendChild(this.noteEl);
    this.subEl = h("p", "ssn-sub bnb-meta");
    sum.appendChild(this.subEl);
    board.appendChild(sum);

    this.boardBody = h("div", "ssn-body");
    this.rowsEl = h("ol", "ssn-rows");
    this.rowsEl.setAttribute("aria-hidden", "true");
    this.gridlines = h("div", "ssn-gridlines");
    this.gridlines.setAttribute("aria-hidden", "true");
    this.boardBody.appendChild(this.gridlines);
    this.boardBody.appendChild(this.rowsEl);
    board.appendChild(this.boardBody);
    this.axisEl = h("div", "ssn-baxis");
    this.axisEl.setAttribute("aria-hidden", "true");
    board.appendChild(this.axisEl);
    this.capEl = h("p", "ssn-cap bnb-meta");
    board.appendChild(this.capEl);
    grid.appendChild(board);
    host.appendChild(grid);

    this.live = h("p", "ssn-live sr-only");
    this.live.setAttribute("aria-live", "polite");
    this.live.setAttribute("aria-atomic", "true");
    host.appendChild(this.live);
    const help = h(
      "p",
      "ssn-help bnb-meta",
      "Left and right arrows change the month, Home and End jump to the first and last. Up and down pick a bird, Enter keeps it in front, Escape lets go. You can also drag across the months.",
    );
    help.id = this.helpId;
    host.appendChild(help);

    this.rows = this.model.species.map((sp, i) => {
      const li = h("li", `ssn-row ssn-c${sp.slot}`);
      const ord = h("span", "ssn-ord");
      const nm = h("span", "ssn-nm", sp.name);
      nm.title = sp.name;
      const trk = h("span", "ssn-trk");
      const zone = h("span", "ssn-zone");
      const bar = h("span", "ssn-bar");
      const ct = h("span", "ssn-ct");
      zone.appendChild(bar);
      zone.appendChild(ct);
      trk.appendChild(zone);
      li.appendChild(ord);
      li.appendChild(nm);
      li.appendChild(trk);
      li.addEventListener("pointerenter", (e) => {
        if (e.pointerType === "mouse") this.setHover(i);
      });
      li.addEventListener("pointerleave", (e) => {
        if (e.pointerType === "mouse") this.setHover(-1);
      });
      li.addEventListener("click", () => this.togglePin(i));
      this.rowsEl.appendChild(li);
      return { li, ord, nm, bar, ct };
    });

    // The widest count sets how much room each bar leaves for its number.
    let widest = 1;
    for (const sp of this.model.species) for (const c of sp.counts) if (c > widest) widest = c;
    host.style.setProperty("--ssn-ct-ch", String(formatCount(widest).length + 1));
    this.bindControls();
  }

  bindControls() {
    const on = (el, ev, fn, opt) => {
      el.addEventListener(ev, fn, opt);
      this.disposers.push(() => el.removeEventListener(ev, fn, opt));
    };
    this.prevBtn.addEventListener("click", () => {
      this.pause();
      this.setMonth(this.month - 1, true);
    });
    this.nextBtn.addEventListener("click", () => {
      this.pause();
      this.setMonth(this.month + 1, true);
    });
    this.playBtn.addEventListener("click", () => (this.playing ? this.pause() : this.play()));
    this.clearBtn.addEventListener("click", () => {
      this.clearFocus();
      this.svg.focus({ preventScroll: true });
    });
    // Anything the reader does that is not the play button stops the tour.
    on(this.host, "pointerdown", (e) => {
      if (!e.target.closest(".ssn-play")) this.pause();
    }, true);
    on(this.host, "keydown", (e) => {
      // Enter and Space on the play button are the button's own; every other key is the reader moving on.
      if (!(e.target.closest(".ssn-play") && (e.key === "Enter" || e.key === " "))) this.pause();
    }, true);

    const svg = this.svg;
    on(svg, "pointerdown", (e) => this.down(e));
    on(svg, "pointermove", (e) => this.move(e));
    on(svg, "pointerup", (e) => this.up(e));
    on(svg, "pointercancel", () => {
      this.drag = null;
      this.setGhost(-1);
    });
    on(svg, "pointerleave", (e) => {
      if (e.pointerType === "mouse" && !this.drag) {
        this.setHover(-1);
        this.setGhost(-1);
      }
    });
    on(svg, "keydown", (e) => this.key(e));
  }

  // ── watching the world ──
  watch() {
    const on = (el, ev, fn, opt) => {
      el.addEventListener(ev, fn, opt);
      this.disposers.push(() => el.removeEventListener(ev, fn, opt));
    };
    if (typeof ResizeObserver === "function") {
      let raf = 0;
      this.ro = new ResizeObserver(() => {
        if (raf) return;
        raf = requestAnimationFrame(() => {
          raf = 0;
          this.size(false);
        });
      });
      this.ro.observe(this.host);
    }
    // The tokens are custom properties read by CSS, so a theme change repaints on its own. Only the
    // widths of the labels depend on the font, which can arrive late.
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(() => this.size(true));
    this.mo = new MutationObserver(() => {
      this.syncMotion();
    });
    this.mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-motion"] });
    if (typeof window.matchMedia === "function") {
      const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
      on(mq, "change", () => this.syncMotion());
    }
    on(document, "visibilitychange", () => {
      if (document.hidden) this.pause();
    });
    if (typeof IntersectionObserver === "function") {
      this.io = new IntersectionObserver((entries) => {
        for (const en of entries) {
          this.onScreen = en.isIntersecting;
          if (!en.isIntersecting) this.pause();
          else this.reveal();
        }
      });
      this.io.observe(this.host);
      // If the observer never reports (a hidden tab at load), show the lines anyway.
      setTimeout(() => this.reveal(), 2500);
    } else this.reveal();
    on(window, "pagehide", () => this.dispose());
    this.syncMotion();
  }

  syncMotion() {
    const rm = reducedMotion();
    this.playBtn.hidden = rm;
    if (rm) this.pause();
    this.host.classList.toggle("is-still", rm);
    if (rm) this.reveal();
  }

  /** The lines draw in once, the first time the figure is on screen (not at all when motion is reduced). */
  reveal() {
    if (this.revealed) return;
    this.revealed = true;
    this.host.classList.remove("ssn-pre");
    if (reducedMotion()) return;
    this.host.classList.add("ssn-in");
    // The class only starts the one-off entrance; once it has run, a rebuild (a resize) must not replay it.
    setTimeout(() => this.host.classList.remove("ssn-in"), 1600);
  }

  // ── size and chart (rebuilt when the width changes) ──
  size(force) {
    if (!this.model) return;
    const w = Math.round(this.host.clientWidth) || 640;
    if (!force && this.lastWidth && Math.abs(w - this.lastWidth) < 2) return;
    this.lastWidth = w;
    const wide = w >= 780;
    this.host.classList.toggle("is-wide", wide);
    this.host.classList.toggle("is-narrow", !wide);
    const mode = wide ? "wide" : "narrow";
    const chartW = Math.round(this.plot.clientWidth) || (wide ? Math.round(w * 0.58) : w);
    this.buildChart(chartW, mode);
    this.reserveSummary();
    // The leaderboard's rows are as tall as the chart's rank rows, so the two halves keep one rhythm.
    this.rowPx = this.geo.pitch;
    this.host.style.setProperty("--ssn-row", `${this.rowPx}px`);
    this.buildBoardScale();
    this.render();
  }

  /**
   * The sentence under the month's name is a line or two or three, depending on the month. Hold the
   * room for the longest of them so the leaderboard under it does not jump as the months go by.
   */
  reserveSummary() {
    const sum = this.noteEl.parentNode;
    sum.style.minHeight = "";
    let longest = 0;
    let at = this.month;
    for (let m = 0; m <= this.model.last; m++) {
      const len = leaderSentence(this.model, this.ranks, m).length;
      if (len > longest) {
        longest = len;
        at = m;
      }
    }
    const keep = this.month;
    this.noteEl.textContent = leaderSentence(this.model, this.ranks, at);
    this.subEl.textContent = "0 heard across these 8 birds; 8 not heard.";
    const px = sum.offsetHeight;
    this.noteEl.textContent = leaderSentence(this.model, this.ranks, keep);
    if (px > 0) sum.style.minHeight = `${px}px`;
  }

  buildChart(width, mode) {
    const model = this.model;
    const last = model.last;
    const narrow = mode === "narrow";
    const family = getComputedStyle(this.host).fontFamily || "sans-serif";
    const fontPx = narrow ? 11 : 12;
    // How long a name may be at the right edge; the whole name is in the leaderboard and the readout.
    const maxChars = narrow ? (width < 460 ? 11 : 16) : 22;
    // Two birds must never share an abbreviation; if they would, those two are cut instead.
    const shorts = model.species.map((sp) => shortName(sp.name, maxChars));
    this.shorts = shorts.map((t, i) => (shorts.indexOf(t) !== i || shorts.lastIndexOf(t) !== i ? truncateName(model.species[i].name, maxChars) : t));
    let labelW = 0;
    for (const t of this.shorts) labelW = Math.max(labelW, Math.ceil(textWidth(t, fontPx, 500, family)));
    const geo = layout({ width, mode, n: this.n, labelW, last });
    this.geo = geo;
    const svg = this.svg;
    while (svg.firstChild) svg.removeChild(svg.firstChild);
    svg.setAttribute("viewBox", `0 0 ${geo.W} ${geo.H}`);
    svg.setAttribute("width", String(geo.W));
    svg.setAttribute("height", String(geo.H));
    svg.classList.toggle("is-narrow", narrow);

    const hatchId = `${this.id}-hatch`;
    const clipId = `${this.id}-clip`;
    const defs = sv("defs");
    const pat = sv("pattern", { id: hatchId, width: "6", height: "6", patternUnits: "userSpaceOnUse", patternTransform: "rotate(45)" });
    pat.appendChild(sv("line", { x1: "0", y1: "0", x2: "0", y2: "6" }, "ssn-hatch"));
    defs.appendChild(pat);
    const clip = sv("clipPath", { id: clipId });
    clip.appendChild(sv("rect", { x: "0", y: "0", width: String(geo.W), height: String(geo.H) }, "ssn-wipe"));
    defs.appendChild(clip);
    svg.appendChild(defs);

    // The months that have not happened: hatched, never drawn as zero.
    if (last < N_MONTHS - 1) {
      const x0 = geo.bandX(last + 1);
      const fut = sv("g", null, "ssn-future");
      const top = geo.ribbonTop - 2;
      const rect = sv("rect", {
        x: String(x0), y: String(top), width: String(geo.plotRight - x0), height: String(geo.plotBottom - top),
        fill: `url(#${hatchId})`,
      }, "ssn-fut");
      const t = sv("title");
      t.textContent = "These months have not happened yet";
      rect.appendChild(t);
      fut.appendChild(rect);
      fut.appendChild(sv("line", { x1: String(x0), x2: String(x0), y1: String(top), y2: String(geo.plotBottom) }, "ssn-fut-edge"));
      svg.appendChild(fut);
    }

    // The selected month: a column you can drag.
    this.band = sv("rect", { x: "0", y: String(geo.ribbonTop - 2), width: String(geo.colW), height: String(geo.plotBottom - geo.ribbonTop + 2), rx: "6" }, "ssn-band");
    svg.appendChild(this.band);
    this.ghostEl = sv("rect", { x: "0", y: String(geo.ribbonTop - 2), width: String(geo.colW), height: String(geo.plotBottom - geo.ribbonTop + 2), rx: "6" }, "ssn-ghost");
    svg.appendChild(this.ghostEl);

    // Rank rows: a quiet line and the ordinal in the gutter.
    const grid = sv("g", null, "ssn-rank-grid");
    for (let r = 1; r <= this.n; r++) {
      grid.appendChild(sv("line", { x1: String(geo.left), x2: String(geo.plotRight), y1: String(geo.y(r)), y2: String(geo.y(r)) }, "ssn-gl"));
      const t = sv("text", { x: String(geo.left - 8), y: String(geo.y(r)), "text-anchor": "end", "dominant-baseline": "central" }, "ssn-rk");
      t.textContent = ordinal(r);
      grid.appendChild(t);
    }
    svg.appendChild(grid);

    // Who led each month: one chip a month in the leader's colour, above the chart.
    const ribbon = sv("g", null, "ssn-ribbon");
    ribbon.setAttribute("aria-hidden", "true");
    this.chips = [];
    for (let m = 0; m <= last; m++) {
      let who = -1;
      for (let i = 0; i < this.n; i++) if (this.ranks[i][m] === 1) who = i;
      if (who < 0) continue;
      const chip = sv("rect", {
        x: String(geo.bandX(m) + 2.5), y: String(geo.ribbonTop), width: String(geo.colW - 5), height: String(geo.ribbonH), rx: String(geo.ribbonH / 2),
      }, `ssn-chip ssn-c${model.species[who].slot}`);
      ribbon.appendChild(chip);
      this.chips.push({ el: chip, i: who });
    }
    svg.appendChild(ribbon);

    // The lines: a pale casing under each so a crossing reads as one passing over the other.
    const lines = sv("g", { "clip-path": `url(#${clipId})` }, "ssn-lines");
    this.groups = [];
    this.dots = [];
    this.polys = [];
    const live = model.thisMonth;
    for (let i = 0; i < this.n; i++) {
      const sp = model.species[i];
      const g = sv("g", { "data-i": String(i) }, `ssn-s ssn-c${sp.slot}`);
      const polys = [];
      const runs = rankRuns(this.ranks[i], last);
      for (const run of runs) {
        const pts = run.map((m) => ({ x: geo.x(m), y: geo.y(this.ranks[i][m]) }));
        const poly = [];
        for (let k = 1; k < pts.length; k++) {
          const s = sampleSegment(bumpSegment(pts[k - 1].x, pts[k - 1].y, pts[k].x, pts[k].y), 10);
          poly.push(...(k === 1 ? s : s.slice(1)));
        }
        polys.push(poly.length ? poly : [pts[0]]);
        // The segment into the running month is dashed: its ranks can still change.
        const tail = live !== null && run.length > 1 && run[run.length - 1] === live;
        const solid = tail ? pts.slice(0, -1) : pts;
        if (solid.length > 1) {
          const d = bumpPath(solid);
          g.appendChild(sv("path", { d }, "ssn-case"));
          g.appendChild(sv("path", { d }, "ssn-line"));
        }
        if (tail) {
          const d = bumpPath(pts.slice(-2));
          g.appendChild(sv("path", { d }, "ssn-case"));
          g.appendChild(sv("path", { d }, "ssn-line is-live"));
        }
      }
      const row = new Array(N_MONTHS).fill(null);
      for (let m = 0; m <= last; m++) {
        const r = this.ranks[i][m];
        if (!r) continue;
        const dot = sv("circle", { cx: String(geo.x(m)), cy: String(geo.y(r)), r: narrow ? "4" : "4.5" }, `ssn-dot${live !== null && m === live ? " is-live" : ""}`);
        g.appendChild(dot);
        row[m] = dot;
      }
      lines.appendChild(g);
      this.groups.push(g);
      this.dots.push(row);
      this.polys.push(polys);
    }
    svg.appendChild(lines);

    this.placeLabels(fontPx, family, maxChars);
    this.buildAxis();
    this.svgBuilt = true;
  }

  /** End labels at the right edge, at the end of a line that stopped early, and where a line starts late. */
  placeLabels(fontPx, family, maxChars) {
    const geo = this.geo;
    const model = this.model;
    const last = model.last;
    const items = [];
    const lh = fontPx + 2;
    const ends = [];
    const hang = [];
    const starts = [];
    for (let i = 0; i < this.n; i++) {
      const runs = rankRuns(this.ranks[i], last);
      if (!runs.length) continue;
      const first = runs[0][0];
      const lastM = runs[runs.length - 1][runs[runs.length - 1].length - 1];
      const name = model.species[i].name;
      if (geo.labelW > 0) {
        const t = this.shorts[i];
        const item = { id: `e${i}`, i, kind: "end", text: t, x: geo.x(lastM) + 9, y: geo.y(this.ranks[i][lastM]), w: textWidth(t, fontPx, 500, family) + 2, h: lh, anchor: "start", shifts: lastM === last ? undefined : [0, -1, 1] };
        (lastM === last ? ends : hang).push(item);
      }
      // A bird whose line starts after the first months is named where it appears, to the left of its first dot.
      if (first >= 2) {
        const avail = geo.x(first) - 9 - geo.left;
        const fit = Math.floor(avail / (fontPx * 0.56));
        if (fit >= 6) {
          const t = shortName(name, Math.min(maxChars, fit));
          starts.push({ id: `s${i}`, i, kind: "start", text: t, x: geo.x(first) - 9, y: geo.y(this.ranks[i][first]), w: textWidth(t, fontPx, 500, family) + 2, h: lh, anchor: "end", shifts: [0] });
        }
      }
    }
    items.push(...ends, ...hang, ...starts);
    // "Not yet" is named in the hatch if it fits without touching a bird's name; the axis and the
    // caption say it either way.
    if (last < N_MONTHS - 1 && geo.plotRight - geo.bandX(last + 1) >= 64) {
      const t = "Not yet";
      items.push({ id: "nyt", i: -1, kind: "note", text: t, x: (geo.bandX(last + 1) + geo.plotRight) / 2, y: geo.plotBottom - geo.pitch / 2, w: textWidth(t, fontPx, 500, family) + 2, h: lh, anchor: "middle" });
    }
    const dots = [];
    for (let i = 0; i < this.n; i++) {
      for (let m = 0; m <= last; m++) {
        const r = this.ranks[i][m];
        if (r > 0) dots.push({ x0: geo.x(m) - 6, x1: geo.x(m) + 6, y0: geo.y(r) - 6, y1: geo.y(r) + 6 });
      }
    }
    const placed = placeLabels(items, { top: 0, bottom: geo.H, left: geo.left - 4, right: geo.W }, dots);
    const g = sv("g", null, "ssn-labels");
    g.setAttribute("aria-hidden", "true");
    this.labels = [];
    this.rects = [];
    for (const it of placed) {
      if (it.hidden) continue;
      const note = it.kind === "note";
      const t = sv("text", { x: String(it.x), y: String(it.y), "text-anchor": it.anchor, "dominant-baseline": "central" }, note ? "ssn-nyt" : "ssn-lab");
      t.textContent = it.text;
      g.appendChild(t);
      if (note) continue;
      t.setAttribute("data-i", String(it.i));
      this.labels.push({ el: t, i: it.i });
      this.rects.push({ i: it.i, x0: it.rect.x0, x1: it.rect.x1, y0: it.rect.y0, y1: it.rect.y1 });
    }
    this.svg.appendChild(g);
  }

  buildAxis() {
    const geo = this.geo;
    const model = this.model;
    const narrow = geo.narrow;
    const ax = sv("g", null, "ssn-axis");
    ax.setAttribute("aria-hidden", "true");
    const yText = geo.axisTop + geo.handleH / 2;
    this.monthTexts = [];
    for (let m = 0; m < N_MONTHS; m++) {
      const t = sv("text", { x: String(geo.x(m)), y: String(yText), "text-anchor": "middle", "dominant-baseline": "central" }, `ssn-mo${m > model.last ? " is-future" : ""}`);
      t.textContent = narrow ? MONTHS[m][0] : MONTHS[m];
      ax.appendChild(t);
      this.monthTexts.push(t);
    }
    if (model.thisMonth !== null) {
      const t = sv("text", { x: String(geo.x(model.thisMonth)), y: String(geo.axisTop + geo.handleH + (narrow ? 12 : 11)), "text-anchor": "middle", "dominant-baseline": "central" }, "ssn-sf");
      t.textContent = "so far";
      ax.appendChild(t);
    }
    // The handle: the thumb of the scrubber, with the month's name in it.
    const hw = narrow ? Math.max(24, Math.min(32, geo.colW * 1.2)) : Math.max(38, Math.min(44, geo.colW * 1.2));
    this.handle = sv("g", null, "ssn-handle");
    this.handle.appendChild(sv("rect", { x: String(-hw / 2), y: String(-geo.handleH / 2 + 1), width: String(hw), height: String(geo.handleH - 2), rx: String((geo.handleH - 2) / 2) }, "ssn-thumb"));
    this.handleText = sv("text", { x: "0", y: "0", "text-anchor": "middle", "dominant-baseline": "central" }, "ssn-thumb-t");
    this.handle.appendChild(this.handleText);
    this.handle.style.transform = `translate(${geo.x(this.month)}px, ${yText}px)`;
    this.yText = yText;
    ax.appendChild(this.handle);
    this.svg.appendChild(ax);
  }

  /** The bar scale is one for the whole year; its ticks are drawn once per width. */
  buildBoardScale() {
    const ticks = niceTicks(this.scaleMax);
    const max = ticks[ticks.length - 1];
    this.gridlines.textContent = "";
    this.axisEl.textContent = "";
    const zone = h("div", "ssn-gzone");
    const azone = h("div", "ssn-azone");
    ticks.forEach((v, k) => {
      const frac = v / max;
      const line = h("span", k === 0 ? "ssn-gl-line is-zero" : "ssn-gl-line");
      line.style.setProperty("--p", String(frac));
      zone.appendChild(line);
      const lab = h("span", "ssn-tk", formatCompact(v));
      lab.style.setProperty("--p", String(frac));
      if (k === 0) lab.classList.add("is-first");
      azone.appendChild(lab);
    });
    this.gridlines.appendChild(zone);
    this.axisEl.appendChild(azone);
    this.tickMax = max;
    this.capEl.textContent = `Bars start at zero and use one scale for every month, up to ${formatCount(max)} (the year's busiest bird-month was ${formatCount(busiest(this.model))}). A longer bar is a bigger number, always. Birds heard equally often are ordered by the year's total.`;
  }

  // ── state ──
  focusIdx() {
    if (this.hover >= 0) return this.hover;
    if (this.kbd >= 0) return this.kbd;
    return this.pin;
  }

  setMonth(m, announce) {
    const next = Math.max(0, Math.min(this.model.last, m));
    if (next === this.month) return;
    this.month = next;
    this.render();
    if (announce) this.say(monthReading(this.model, this.ranks, this.month));
  }

  setHover(i) {
    if (i === this.hover) return;
    this.hover = i;
    this.render();
  }

  setGhost(m) {
    if (m === this.ghost) return;
    this.ghost = m;
    this.renderGhost();
  }

  togglePin(i) {
    this.pin = this.pin === i ? -1 : i;
    this.kbd = this.pin === i ? i : -1;
    this.render();
    this.say(this.pin >= 0 ? `${speciesReading(this.model, this.ranks, i, this.month)} Kept in front.` : "Showing all birds.");
  }

  clearFocus() {
    this.hover = -1;
    this.kbd = -1;
    this.pin = -1;
    this.render();
    this.say("Showing all birds.");
  }

  say(text) {
    this.live.textContent = text;
  }

  // ── play ──
  play() {
    if (reducedMotion() || this.playing) return;
    this.playing = true;
    this.finished = false;
    if (this.month >= this.model.last) this.setMonth(0, false);
    this.live.setAttribute("aria-live", "off");
    this.syncPlay();
    this.schedule();
  }

  schedule() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.tick(), 1000);
  }

  tick() {
    if (!this.playing) return;
    if (document.hidden || !this.onScreen || reducedMotion()) {
      this.pause();
      return;
    }
    this.setMonth(this.month + 1, false);
    if (this.month >= this.model.last) {
      this.playing = false;
      this.finished = true;
      this.live.setAttribute("aria-live", "polite");
      this.syncPlay();
      this.say(monthReading(this.model, this.ranks, this.month));
    } else this.schedule();
  }

  pause() {
    if (!this.playing) return;
    this.playing = false;
    clearTimeout(this.timer);
    this.live.setAttribute("aria-live", "polite");
    this.syncPlay();
  }

  syncPlay() {
    const label = this.playing ? "Pause" : this.finished ? "Replay" : "Play";
    if (label !== this.playState) {
      this.playState = label;
      this.playIcon.textContent = "";
      this.playIcon.appendChild(icon(this.playing ? "pause" : "play"));
      this.playLabel.textContent = label;
    }
    this.playBtn.setAttribute("aria-label", this.playing ? "Pause the tour of the months" : "Play through the months");
    this.host.classList.toggle("is-playing", this.playing);
  }

  // ── pointer ──
  chartPoint(e) {
    const r = this.svg.getBoundingClientRect();
    const k = r.width ? this.geo.W / r.width : 1;
    return { x: (e.clientX - r.left) * k, y: (e.clientY - r.top) * k };
  }

  pick(p, type) {
    const thr = type === "touch" ? 18 : type === "pen" ? 14 : 9;
    const hit = nearestSpecies(p.x, p.y, this.polys, this.rects, thr);
    return hit ? hit.i : -1;
  }

  down(e) {
    if (e.button > 0 || !this.geo) return;
    const p = this.chartPoint(e);
    const cand = this.pick(p, e.pointerType);
    this.drag = { id: e.pointerId, x: p.x, moved: false, cand };
    try {
      this.svg.setPointerCapture(e.pointerId);
    } catch (err) {
      /* a pointer that is already gone */
    }
    if (cand < 0) this.setMonth(monthAt(p.x, this.geo, this.model.last), true);
  }

  move(e) {
    if (!this.geo) return;
    const p = this.chartPoint(e);
    const d = this.drag;
    if (d && d.id === e.pointerId) {
      if (!d.moved && Math.abs(p.x - d.x) > 6) d.moved = true;
      if (d.moved) {
        d.cand = -1;
        this.setMonth(monthAt(p.x, this.geo, this.model.last), true);
      }
      return;
    }
    if (e.pointerType === "mouse") {
      const i = this.pick(p, "mouse");
      this.setHover(i);
      this.svg.classList.toggle("is-over", i >= 0);
      this.setGhost(i >= 0 ? -1 : monthAt(p.x, this.geo, this.model.last));
    }
  }

  up(e) {
    const d = this.drag;
    this.drag = null;
    try {
      this.svg.releasePointerCapture(e.pointerId);
    } catch (err) {
      /* already released */
    }
    if (!d || d.id !== e.pointerId) return;
    if (!d.moved && d.cand >= 0) {
      const p = this.chartPoint(e);
      this.setMonth(monthAt(p.x, this.geo, this.model.last), false);
      this.togglePin(d.cand);
    }
    if (e.pointerType !== "mouse") this.setGhost(-1);
  }

  // ── keyboard ──
  key(e) {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const last = this.model.last;
    let handled = true;
    switch (e.key) {
      case "ArrowLeft":
        this.setMonth(this.month - 1, true);
        break;
      case "ArrowRight":
        this.setMonth(this.month + 1, true);
        break;
      case "Home":
        this.setMonth(0, true);
        break;
      case "End":
        this.setMonth(last, true);
        break;
      case "ArrowDown":
      case "ArrowUp":
        this.stepSpecies(e.key === "ArrowDown" ? 1 : -1);
        break;
      case "Enter":
      case " ":
        if (this.kbd >= 0 || this.pin >= 0) this.togglePin(this.kbd >= 0 ? this.kbd : this.pin);
        break;
      case "Escape":
        if (this.focusIdx() >= 0) this.clearFocus();
        else handled = false;
        break;
      default:
        handled = false;
    }
    if (handled) e.preventDefault();
  }

  /** Up and down walk the birds in this month's order, 1st first. */
  stepSpecies(dir) {
    const { order, heard } = boardPositions(this.model, this.ranks, this.month);
    if (!heard) return;
    const cur = this.kbd >= 0 ? this.kbd : this.pin;
    const at = cur >= 0 ? order.indexOf(cur) : -1;
    let k;
    if (at < 0 || at >= heard) k = dir > 0 ? 0 : heard - 1;
    else k = Math.max(0, Math.min(heard - 1, at + dir));
    this.kbd = order[k];
    this.render();
    this.say(speciesReading(this.model, this.ranks, this.kbd, this.month));
  }

  // ── painting (cheap, on demand) ──
  renderGhost() {
    if (!this.ghostEl || !this.geo) return;
    const show = this.ghost >= 0 && this.ghost !== this.month;
    this.ghostEl.classList.toggle("is-on", show);
    if (show) this.ghostEl.style.transform = `translateX(${this.geo.bandX(this.ghost)}px)`;
  }

  render() {
    if (!this.model || !this.geo || !this.svgBuilt) return;
    const model = this.model;
    const m = this.month;
    const geo = this.geo;
    const f = this.focusIdx();

    this.band.style.transform = `translateX(${geo.bandX(m)}px)`;
    this.handle.style.transform = `translate(${geo.x(m)}px, ${this.yText}px)`;
    this.handleText.textContent = MONTHS[m];
    this.monthTexts.forEach((t, k) => t.classList.toggle("is-sel", k === m));
    this.renderGhost();
    this.svg.classList.toggle("has-focus", f >= 0);
    for (let i = 0; i < this.n; i++) {
      this.groups[i].classList.toggle("is-focus", i === f);
      const row = this.dots[i];
      for (let k = 0; k < N_MONTHS; k++) if (row[k]) row[k].classList.toggle("is-sel", k === m);
    }
    for (const l of this.labels) l.el.classList.toggle("is-focus", l.i === f);
    for (const c of this.chips) c.el.classList.toggle("is-focus", c.i === f);

    // The leaderboard.
    const { pos, heard } = boardPositions(model, this.ranks, m);
    const rowPx = this.rowPx;
    this.rowsEl.style.height = `${this.n * rowPx}px`;
    for (let i = 0; i < this.n; i++) {
      const r = this.rows[i];
      const sp = model.species[i];
      const c = sp.counts[m];
      const rank = this.ranks[i][m];
      r.li.style.transform = `translateY(${pos[i] * rowPx}px)`;
      r.li.classList.toggle("is-quiet", !rank);
      r.li.classList.toggle("is-focus", i === f);
      r.li.classList.toggle("is-dim", f >= 0 && i !== f);
      r.ord.textContent = rank ? ordinal(rank) : "–";
      r.bar.style.setProperty("--p", String(rank ? c / this.tickMax : 0));
      r.ct.style.setProperty("--p", String(rank ? c / this.tickMax : 0));
      r.ct.textContent = rank ? formatCount(c) : c === null ? "no reading" : "not heard";
    }
    this.svg.classList.toggle("is-playing", this.playing);

    // The words.
    this.monthEl.textContent = MONTH_NAMES[m];
    this.soFarEl.hidden = !soFarOf(model, m);
    this.noteEl.textContent = leaderSentence(model, this.ranks, m);
    const total = monthTotal(model, m);
    this.subEl.textContent = heard
      ? `${formatCount(total)} heard across ${this.n === 1 ? "this bird" : `these ${this.n} birds`}${heard < this.n ? `; ${this.n - heard} not heard` : ""}.`
      : `No sightings of ${this.n === 1 ? "this bird" : `these ${this.n} birds`} in ${MONTH_NAMES[m]}.`;
    this.prevBtn.disabled = m <= 0;
    this.nextBtn.disabled = m >= model.last;
    this.syncPlay();

    this.yearEl.textContent = yearSentence(model, this.ranks);
    this.fillLeads();

    this.readoutText.textContent = f >= 0
      ? speciesReading(model, this.ranks, f, m)
      : "Point at a line or a bar to follow one bird. Tap or click to keep it in front.";
    this.clearBtn.hidden = this.pin < 0;
    // The name changes with the month only (not with every hover), so a screen reader is not told twice.
    if (this.labelMonth !== m) {
      this.labelMonth = m;
      this.svg.setAttribute("aria-label", `${describeChart(model, this.ranks)} ${MONTH_NAMES[m]}${soFarOf(model, m) ? ", so far" : ""} is selected. ${leaderSentence(model, this.ranks, m)}`);
    }
  }

  /** Who held the lead and when, with the colour that carries them. Rebuilt only if the runs change. */
  fillLeads() {
    const runs = leadRuns(this.model, this.ranks);
    const key = runs.map((r) => `${r.idx}:${r.from}:${r.to}`).join("|");
    if (key === this.leadsKey) return;
    this.leadsKey = key;
    this.leadsEl.textContent = "";
    // Past six turns the list is longer than the chart it explains; the strip above already shows each month.
    if (runs.length > 6) {
      this.leadsEl.appendChild(h("li", "ssn-lead ssn-lead-many", "Too many turns to list: follow the strip above, or step through the months."));
      return;
    }
    for (const r of runs) {
      const sp = this.model.species[r.idx];
      const li = h("li", `ssn-lead ssn-c${sp.slot}`);
      li.appendChild(h("i", "ssn-sw"));
      li.appendChild(h("span", "ssn-lead-n", sp.name));
      li.appendChild(h("span", "ssn-lead-r", rangeText(r.from, r.to)));
      this.leadsEl.appendChild(li);
    }
  }

  dispose() {
    this.pause();
    for (const f of this.disposers) f();
    this.disposers = [];
    if (this.ro) this.ro.disconnect();
    if (this.io) this.io.disconnect();
    if (this.mo) this.mo.disconnect();
    this.host.__ssn = null;
    // A page restored from the back/forward cache starts again from the host's own data.
    this.host.removeAttribute("data-ssn-ready");
  }
}

function boot() {
  document.querySelectorAll(".ssn[data-ssn], [data-ssn]").forEach((host) => {
    if (host.__ssn || host.hasAttribute("data-ssn-ready")) return;
    host.__ssn = new Seasons(host);
  });
}

if (typeof document !== "undefined") {
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", boot);
  else boot();
  document.addEventListener("htmx:afterSwap", boot);
  window.addEventListener("pageshow", (e) => {
    if (e.persisted) boot();
  });
}
