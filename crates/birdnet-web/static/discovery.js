// The discovery timeline: when each bird joined the life list, and which is the newest.
//
// Wide: a timeline of firsts. Every species is a portrait that stands on the day it was first
// heard, on a thin stem that lands on the "list so far" curve at the moment the list grew. Species
// first heard on the same day share one tray under one stem. Narrow (a phone): a vertical timeline,
// the newest at the top, grouped by month, each bird a row with its number on the spine.
//
// What it promises, and what holds it to it (tools: see discovery.test.mjs):
//   * Position is the date and nothing else. A portrait stands at its first-heard day; lanes lift
//     neighbours up, they never slide them sideways (dayToX, packLanes). The one exception is the
//     two ends of the whole timeline, where a tray is kept inside the picture.
//   * The curve is the list so far: a step that rises by one per species on the day it was first
//     heard, from zero. It is never smoothed, so it cannot invent a rise (stepCurve).
//   * Species first heard the same day are never split across lanes; when there are more than ten
//     the tray shows nine and a "+N" chip, and every one of them is still reachable (the chip opens
//     a list; the arrow keys walk every species whether it is shown or not).
//   * A name is printed beside a portrait only where the whole text fits clear of every other
//     portrait, label and stem (placeLabels); the card carries the rest.
//   * A photo is requested only for a portrait that is on screen, a few at a time; until it loads,
//     and for good if it never does, the disc shows the species code or initials. No broken image.
//   * Everything the picture says is also in the aria-label, the live reading and, on a phone, in
//     the list itself. Nothing is hover-only: touch taps, the keyboard walks.
//   * Reduced motion (the OS, or the operator's Motion setting): no entrance, no glide, no fade.
//   * Nothing is drawn while the tab is hidden or the figure is off screen.
//
// An ES module so its pure parts can be exported and tested under node. It starts itself only
// where there is a document. No colour is written in this file: all of them are read from the
// stylesheet's tokens (cssRgb), and re-read when the theme changes.

// ── pure: dates ─────────────────────────────────────────────────────────────

const DAY_MS = 86400000;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_LONG = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** A day number (days since 1970-01-01, UTC) from `YYYY-MM-DD`, or NaN. */
export function parseDay(iso) {
  if (typeof iso !== "string") return NaN;
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso);
  if (!m) return NaN;
  const y = +m[1];
  const mo = +m[2];
  const d = +m[3];
  const t = Date.UTC(y, mo - 1, d);
  const c = new Date(t);
  if (c.getUTCFullYear() !== y || c.getUTCMonth() !== mo - 1 || c.getUTCDate() !== d) return NaN;
  return Math.round(t / DAY_MS);
}

/** `YYYY-MM-DD` for a day number. */
export function isoOf(day) {
  return new Date(day * DAY_MS).toISOString().slice(0, 10);
}

function parts(day) {
  const d = new Date(day * DAY_MS);
  return { y: d.getUTCFullYear(), m: d.getUTCMonth(), d: d.getUTCDate(), wd: d.getUTCDay() };
}

/** `Wed 6 Aug 2026`. */
export function formatDay(day) {
  const p = parts(day);
  return `${WEEKDAYS[p.wd]} ${p.d} ${MONTHS[p.m]} ${p.y}`;
}

/** `6 Aug`. */
export function formatShort(day) {
  const p = parts(day);
  return `${p.d} ${MONTHS[p.m]}`;
}

/** `2026-08`, the month a day is in. */
export function monthKey(day) {
  const p = parts(day);
  return `${p.y}-${String(p.m + 1).padStart(2, "0")}`;
}

/** `August 2026` for a `YYYY-MM` key. */
export function monthLabel(key) {
  const [y, m] = key.split("-").map(Number);
  return `${MONTHS_LONG[m - 1]} ${y}`;
}

function monthStart(y, m) {
  return Math.round(Date.UTC(y, m, 1) / DAY_MS);
}

const fmt = (n) => (typeof n === "number" && Number.isFinite(n) ? n.toLocaleString("en") : String(n));
const plural = (n, one, many) => `${fmt(n)} ${n === 1 ? one : many}`;

/**
 * How long ago `day` was, as a phrase that goes after "first heard ... (": today, yesterday,
 * 5 days ago, 12 weeks ago, 7 months ago, 1 year, 3 months ago. Weeks run to sixteen of them, so
 * "12 weeks ago" is what a reader would say; months after that, years from a year on.
 */
export function agoPhrase(day, today) {
  const d = Math.max(0, today - day);
  if (d === 0) return "today";
  if (d === 1) return "yesterday";
  if (d < 14) return `${d} days ago`;
  if (d < 112) return `${Math.floor(d / 7)} weeks ago`;
  if (d < 365) return `${plural(Math.floor(d / 30.4375), "month", "months")} ago`;
  const months = Math.round(d / 30.4375);
  const y = Math.floor(months / 12);
  const rem = months % 12;
  return `${plural(y, "year", "years")}${rem ? `, ${plural(rem, "month", "months")}` : ""} ago`;
}

// ── pure: the data ──────────────────────────────────────────────────────────

/** The shortest span the timeline is drawn over: a station a day old still gets four weeks of axis. */
export const MIN_SPAN = 28;

const byName = (a, b) => {
  const x = a.name.toLowerCase();
  const y = b.name.toLowerCase();
  return x < y ? -1 : x > y ? 1 : a.name < b.name ? -1 : a.name > b.name ? 1 : 0;
};

/**
 * Turn the server's JSON into the model everything else reads: species in the order they joined
 * the list (day, then name), each with its `rank` (1 is the first), the days of the first and the
 * last, the `full` span the axis covers (`t1` is the end of today), one `bunch` per day something
 * was first heard (`i0..i1` are species indices) and the `steps` of the list so far.
 */
export function prepare(raw) {
  const rows = raw && Array.isArray(raw.species) ? raw.species : [];
  const list = [];
  for (const r of rows) {
    if (!r || typeof r.name !== "string" || r.name === "") continue;
    const day = parseDay(r.first);
    if (!Number.isFinite(day)) continue;
    const n = Number(r.count);
    list.push({
      name: r.name,
      key: typeof r.key === "string" && r.key ? r.key : r.name,
      sci: typeof r.sci === "string" ? r.sci : "",
      day,
      first: isoOf(day),
      count: Number.isFinite(n) && n > 0 ? Math.floor(n) : 0,
      code: typeof r.code === "string" ? r.code.trim().slice(0, 4).toUpperCase() : "",
      img: typeof r.img === "string" && r.img ? r.img : null,
    });
  }
  list.sort((a, b) => a.day - b.day || byName(a, b));
  list.forEach((s, i) => {
    s.i = i;
    s.rank = i + 1;
  });
  const firstDay = list.length ? list[0].day : NaN;
  const lastDay = list.length ? list[list.length - 1].day : NaN;
  let to = parseDay(raw && raw.to);
  let from = parseDay(raw && raw.from);
  if (!Number.isFinite(to) || (Number.isFinite(lastDay) && lastDay > to)) to = lastDay;
  if (!Number.isFinite(from) || (Number.isFinite(firstDay) && firstDay < from)) from = firstDay;
  const bunches = [];
  for (const s of list) {
    const b = bunches[bunches.length - 1];
    if (b && b.day === s.day) b.i1 = s.i;
    else bunches.push({ day: s.day, i0: s.i, i1: s.i });
  }
  bunches.forEach((b, k) => {
    b.k = k;
    b.n = b.i1 - b.i0 + 1;
  });
  const steps = bunches.map((b) => ({ day: b.day, n: b.i1 + 1 }));
  const full = Number.isFinite(to) ? { t0: Math.min(from, to + 1 - MIN_SPAN), t1: to + 1 } : { t0: 0, t1: MIN_SPAN };
  return { species: list, total: list.length, from, to, full, bunches, steps };
}

/** The sentences the figure speaks in: when, how many times, which number, all of it for one bird. */
export function whenPhrase(s, today) {
  return `first heard ${formatDay(s.day)} (${agoPhrase(s.day, today)})`;
}

export function countPhrase(s) {
  if (s.count <= 0) return "";
  if (s.count === 1) return "heard just once so far";
  return `heard ${fmt(s.count)} times since`;
}

/** What one species says, as one sentence (the live reading, the aria-label of a row). */
export function readingText(model, s) {
  const c = countPhrase(s);
  return `${s.name}: ${whenPhrase(s, model.to)}${c ? `, ${c}` : ""}. Number ${s.rank} of ${fmt(model.total)} on the list.`;
}

/** The two notes drawn beside the first and the newest bird, in words. */
export function notesFor(model) {
  const sp = model.species;
  if (!sp.length) return { first: "", newest: "" };
  const bs = model.bunches;
  const last = bs[bs.length - 1];
  const first = bs[0];
  const ago = agoPhrase(last.day, model.to);
  const names = (b) => sp.slice(b.i0, b.i1 + 1).map((s) => s.name);
  let newest;
  if (last.n === 1) newest = `newest: ${sp[last.i1].name}, ${ago}`;
  else if (last.n === 2) newest = `newest: ${names(last).join(" and ")}, ${ago}`;
  else newest = `newest: ${last.n} species, ${ago}`;
  let firstNote;
  if (bs.length === 1) firstNote = "";
  else if (first.n === 1) firstNote = `first: ${sp[first.i0].name}`;
  else firstNote = `the first day: ${first.n} species`;
  return { first: firstNote, newest };
}

/** The live reading while nothing is pointed at. */
export function idleText(model) {
  const n = notesFor(model);
  const first = model.bunches[0];
  const head = `${fmt(model.total)} species since ${formatDay(model.from)}. ${n.newest.charAt(0).toUpperCase()}${n.newest.slice(1)}.`;
  const start = first.n === 1 ? `The first was ${model.species[first.i0].name}.` : `${first.n} were first heard on the first day.`;
  return model.bunches.length === 1 ? head : `${head} ${start}`;
}

/** Initials for a disc with no picture: the 4-letter code when there is room, else two letters. */
export function initials(s, long) {
  if (long && s.code) return s.code;
  const words = s.name.split(/\s+/).filter(Boolean);
  const letters = words.map((w) => w.charAt(0)).join("").toUpperCase();
  if (letters.length >= 2) return letters.slice(0, 2);
  if (s.code) return s.code.slice(0, 2);
  return (s.name.slice(0, 2) || "?").toUpperCase();
}

// ── pure: the range and the view ────────────────────────────────────────────

/** The ranges worth offering: only a range at most 70% of the whole is a different picture. */
export function rangeOptions(model) {
  const span = model.full.t1 - model.full.t0;
  const out = [{ id: "all", label: "All time", days: span }];
  if (365 <= span * 0.7) out.push({ id: "year", label: "Last year", days: 365 });
  if (90 <= span * 0.7) out.push({ id: "90", label: "Last 90 days", days: 90 });
  return out;
}

/** The view (`{ t0, t1 }`, in day numbers, `t1` exclusive) a range option shows. */
export function viewForRange(model, opt) {
  const { t0, t1 } = model.full;
  return { t0: Math.max(t0, t1 - opt.days), t1 };
}

/** A view inside `full`, its span at least `minSpan` days and at most all of it, panned back in. */
export function clampView(view, full, minSpan = 14) {
  const whole = full.t1 - full.t0;
  const span = Math.min(whole, Math.max(Math.min(minSpan, whole), view.t1 - view.t0));
  let t0 = view.t0;
  if (t0 + span > full.t1) t0 = full.t1 - span;
  if (t0 < full.t0) t0 = full.t0;
  return { t0, t1: t0 + span };
}

/** Zoom by `factor` (below 1 zooms in) keeping the day `anchor` where it is on screen. */
export function zoomView(view, factor, anchor, full, minSpan = 14) {
  const span = view.t1 - view.t0;
  const f = span > 0 ? (anchor - view.t0) / span : 0.5;
  const ns = Math.min(full.t1 - full.t0, Math.max(Math.min(minSpan, full.t1 - full.t0), span * factor));
  const t0 = anchor - f * ns;
  return clampView({ t0, t1: t0 + ns }, full, minSpan);
}

/** Slide a view by `dt` days, keeping its span. */
export function panView(view, dt, full) {
  const span = view.t1 - view.t0;
  return clampView({ t0: view.t0 + dt, t1: view.t0 + dt + span }, full, span);
}

export function sameView(a, b, eps = 0.5) {
  return Math.abs(a.t0 - b.t0) < eps && Math.abs(a.t1 - b.t1) < eps;
}

/** Which range option (if any) a view is: so the right button stays pressed after the view is set. */
export function rangeOf(model, view) {
  for (const o of rangeOptions(model)) if (sameView(viewForRange(model, o), view)) return o.id;
  return null;
}

/** The x (0 at the left of the plot) a day's centre falls at, for a view across `plotW` px. */
export function dayToX(day, view, plotW) {
  return ((day + 0.5 - view.t0) / (view.t1 - view.t0)) * plotW;
}

/** The (fractional) day at x px into the plot. */
export function xToDay(x, view, plotW) {
  return view.t0 + (x / plotW) * (view.t1 - view.t0);
}

// ── pure: the axis ──────────────────────────────────────────────────────────

/**
 * The ticks of the time axis for `view` over `plotW` px. A tick is `{ day, label, year, kind }`:
 * `kind` is `"month"` (a labelled first of the month; `year` is set on a January) or `"week"` (a
 * Monday, labelled with its day of the month, only when a day is wide enough to read). The months
 * thin out (every 2, 3, 6, 12) until their labels are 56 px apart.
 */
export function axisTicks(view, plotW) {
  const span = view.t1 - view.t0;
  const ppd = plotW / span;
  const lo = view.t0 - 1;
  const hi = view.t1 + 1;
  const ticks = [];
  const monthPx = 30.4375 * ppd;
  const step = [1, 2, 3, 6, 12].find((s) => s * monthPx >= 56) || 12;
  const p0 = parts(Math.floor(view.t0));
  for (let y = p0.y, m = 0, guard = 0; guard < 4000; guard++) {
    const day = monthStart(y, m);
    if (day > hi) break;
    if (day >= lo && m % step === 0) ticks.push({ day, label: MONTHS[m], year: m === 0 ? String(y) : null, kind: "month" });
    m += 1;
    if (m === 12) {
      m = 0;
      y += 1;
    }
  }
  if (ppd >= 6) {
    // Mondays: day 4 of the epoch (1970-01-01) was a Thursday, so a Monday is day % 7 === 4.
    const first = Math.ceil(lo) + ((((4 - Math.ceil(lo)) % 7) + 7) % 7);
    for (let day = first; day <= hi; day += 7) {
      const near = ticks.some((t) => t.kind === "month" && Math.abs(t.day - day) * ppd < 22);
      if (near) continue;
      ticks.push({ day, label: String(parts(day).d), year: null, kind: "week" });
    }
  }
  ticks.sort((a, b) => a.day - b.day);
  return ticks;
}

/** The year printed at the left edge of the axis, or null when a January tick is close enough to carry it. */
export function leadYear(view, plotW, ticks) {
  const ppd = plotW / (view.t1 - view.t0);
  const y = String(parts(Math.floor(view.t0)).y);
  const jan = ticks.find((t) => t.year && (t.day - view.t0) * ppd < 70);
  return jan ? null : y;
}

/** Round gridline values for a count axis: at most `max`, none close to `total` itself, none at 0. */
export function countTicks(total, max = 5) {
  if (!(total > 1)) return [];
  const raw = total / max;
  const pow = Math.pow(10, Math.floor(Math.log10(raw)));
  const step = Math.max(1, [1, 2, 5, 10].map((m) => m * pow).find((s) => s >= raw));
  const out = [];
  for (let v = step; v <= total * 0.88; v += step) out.push(Math.round(v));
  return out;
}

// ── pure: trays, lanes ──────────────────────────────────────────────────────

/** The shape of one day's tray: `cells` discs in `cols` columns and `rows` rows, `chip` hidden species. */
export function bunchShape(k, maxShown = 10) {
  const cells = Math.min(k, maxShown);
  const cols = cells <= 4 ? cells : cells <= 6 ? 3 : cells <= 8 ? 4 : 5;
  return { k, cells, cols, rows: Math.ceil(cells / cols), shown: k > maxShown ? maxShown - 1 : k, chip: k > maxShown ? k - (maxShown - 1) : 0 };
}

/** Sizes for one disc diameter `d`: the pitch of a tray, the height of a lane. */
export function tierGeom(d) {
  const big = d >= 28;
  return { d, cellGap: big ? 6 : 4, pitch: d + (big ? 6 : 4), laneH: d + (big ? 10 : 6), tray: big ? 5 : 3, gap: big ? 6 : 4 };
}

/** The width a tray takes (a lone disc is just its diameter). */
export function bunchWidth(sh, g) {
  return sh.cells === 1 ? g.d : sh.cols * g.pitch - g.cellGap + 2 * g.tray;
}

/**
 * Greedy lane packing, in the order the days come. `items` is `[{ cx, w, rows }]` in time order:
 * the middle of the stem, the width, the number of lanes the tray is tall. Each goes in the lowest
 * lane (rows of lanes, for a tall tray) whose last occupant ends at least `gap` before it starts.
 * `minX`/`maxX` keep the ends inside the picture. Returns `{ placed: [{ lane, left, right }], lanes }`.
 * Nothing slides sideways: a crowd goes up, so a portrait's x is always its date.
 */
export function packLanes(items, { gap = 6, minX = -Infinity, maxX = Infinity } = {}) {
  const right = [];
  const placed = [];
  let lanes = 0;
  for (const it of items) {
    let left = it.cx - it.w / 2;
    left = Math.max(minX, Math.min(left, maxX - it.w));
    const end = left + it.w;
    let lane = 0;
    for (;; lane++) {
      let ok = true;
      for (let r = 0; r < it.rows; r++) {
        const edge = right[lane + r];
        if (edge !== undefined && edge + gap > left) {
          ok = false;
          break;
        }
      }
      if (ok) break;
    }
    for (let r = 0; r < it.rows; r++) right[lane + r] = end;
    placed.push({ lane, left, right: end });
    if (lane + it.rows > lanes) lanes = lane + it.rows;
  }
  return { placed, lanes };
}

/**
 * Lay the whole timeline out in WORLD coordinates (x 0 is the left of the all-time picture; a pan
 * only translates it, so portraits never change lane while you drag). `o`:
 *   scale   px per day (plot width over the view's span)
 *   width   px of the picture, for the minimum height
 *   measure (text, kind) -> px, kind is "note" or "label"
 *   notes   { first, newest } texts (notesFor)
 *   tiers   disc diameters to try, the largest that fits `budget` px of lanes wins
 * Returns marks (discs and "+N" chips), trays, stems, labels and the vertical geometry.
 */
export function layoutStage(model, o) {
  const gutterL = o.gutterL ?? 38;
  const gutterR = o.gutterR ?? 30;
  const scale = o.scale;
  const full = model.full;
  const worldW = gutterL + (full.t1 - full.t0) * scale + gutterR;
  const wx = (day) => gutterL + (day + 0.5 - full.t0) * scale;
  const tiers = o.tiers || [40, 34, 28, 22, 16, 10];
  const budget = o.budget ?? 330;
  const bs = model.bunches;
  const shapes = bs.map((b) => bunchShape(b.n));
  let g = null;
  let packed = null;
  for (let t = 0; t < tiers.length; t++) {
    g = tierGeom(tiers[t]);
    packed = packLanes(
      bs.map((b, k) => ({ cx: wx(b.day), w: bunchWidth(shapes[k], g), rows: shapes[k].rows })),
      { gap: g.gap, minX: 4, maxX: worldW - 4 },
    );
    if (packed.lanes * g.laneH <= budget) break;
  }
  const lanes = Math.max(1, packed.lanes);
  if (lanes * g.laneH > budget) {
    // Even the smallest dots do not fit: squeeze the lanes, and the dots with them.
    const laneH = Math.max(2.6, budget / lanes);
    g = { ...g, laneH, d: Math.max(2.4, laneH - 1.6), squeezed: true };
  }
  const d = g.d;
  const r = d / 2;
  // ── labels and notes, in lane space ──
  const occ = Array.from({ length: lanes + 1 }, () => []);
  const stems = [];
  bs.forEach((b, k) => {
    const p = packed.placed[k];
    const sh = shapes[k];
    for (let q = 0; q < sh.rows; q++) occ[p.lane + q].push(p.left, p.right);
    const inset = sh.cells === 1 ? 0 : g.tray + r;
    stems.push({ x: Math.max(p.left + inset, Math.min(wx(b.day), p.right - inset)), lane: p.lane, k });
  });
  const sortedStems = stems.slice().sort((a, b) => a.x - b.x);
  const free = (lane, a, b) => {
    if (a < 4 || b > worldW - 4) return false;
    const row = occ[lane];
    for (let q = 0; q < row.length; q += 2) if (a < row[q + 1] + 4 && b > row[q] - 4) return false;
    let lo = 0;
    let hi = sortedStems.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (sortedStems[mid].x < a - 3) lo = mid + 1;
      else hi = mid;
    }
    for (let q = lo; q < sortedStems.length && sortedStems[q].x <= b + 3; q++) if (sortedStems[q].lane > lane) return false;
    return true;
  };
  const take = (lane, a, b) => occ[lane].push(a, b);
  const labels = [];
  const leaders = [];
  // The two notes ride in a band over the lanes, at the two ends of the timeline, where the eye
  // looks for "where it began" and "what is newest". A hairline joins a note to its portrait when
  // the way down is clear of other portraits.
  const note = (kind, text, k, flushRight) => {
    if (!text) return;
    const p = packed.placed[k];
    const w = o.measure(text, "note") + 2;
    const lx = flushRight ? Math.min(worldW - 6, Math.max(p.right, 6 + w)) : Math.max(6, Math.min(p.left, worldW - 6 - w));
    labels.push({ kind, text, x: lx, lane: lanes, top: true, align: flushRight ? "right" : "left", w });
    const stemX = stems[k].x;
    let clear = true;
    for (let l = p.lane + shapes[k].rows; l < lanes && clear; l++) clear = free(l, stemX - 3, stemX + 3);
    if (clear) leaders.push({ kind, x: stemX, k });
  };
  const nb = bs.length - 1;
  const hasNotes = !!(o.notes && (o.notes.newest || (o.notes.first && bs.length > 1)));
  const needTop = hasNotes;
  if (o.notes && o.notes.newest) note("newest", o.notes.newest, nb, true);
  if (o.notes && o.notes.first && bs.length > 1) note("first", o.notes.first, 0, false);
  // Names: newest first, a single disc at a time, only where the whole text fits clear of everything.
  if (d >= 22 && model.total <= 300) {
    for (let k = nb; k >= 0; k--) {
      if (shapes[k].cells !== 1 || (k === nb && o.notes && o.notes.newest) || (k === 0 && o.notes && o.notes.first)) continue;
      const s = model.species[bs[k].i0];
      let text = s.name;
      let w = o.measure(text, "label");
      const maxW = 150;
      if (w > maxW) {
        while (text.length > 4 && o.measure(text + "…", "label") > maxW) text = text.slice(0, -1);
        text = text.trimEnd() + "…";
        w = o.measure(text, "label");
      }
      const p = packed.placed[k];
      for (const side of ["right", "left"]) {
        const a = side === "right" ? p.right + 5 : p.left - 5 - w;
        if (free(p.lane, a, a + w)) {
          take(p.lane, a, a + w);
          labels.push({ kind: "name", text, x: side === "right" ? a : a + w, lane: p.lane, top: false, align: side === "right" ? "left" : "right", k });
          break;
        }
      }
    }
  }
  // ── the vertical geometry ──
  const noteBand = needTop ? 26 : 0;
  let topPad = 12 + noteBand;
  let curveH = o.curveH ?? 104;
  const midGap = 14;
  const axisH = 40;
  let H = topPad + lanes * g.laneH + midGap + curveH + axisH;
  const minH = o.minH ?? 300;
  if (H < minH) {
    const extra = minH - H;
    const toCurve = Math.min(extra * 0.5, 36);
    curveH += toCurve;
    topPad += extra - toCurve;
    H = minH;
  }
  const laneZoneTop = topPad;
  const curveTop = laneZoneTop + lanes * g.laneH + midGap;
  const curveBase = curveTop + curveH;
  const laneY = (l) => laneZoneTop + (lanes - 1 - l) * g.laneH + g.laneH / 2;
  const curveY = (n) => curveBase - (curveH * n) / Math.max(1, model.total);
  // ── marks ──
  const marks = [];
  const trays = [];
  const stemOut = [];
  const markOf = new Int32Array(model.total).fill(-1);
  bs.forEach((b, k) => {
    const p = packed.placed[k];
    const sh = shapes[k];
    if (sh.cells === 1) {
      const y = laneY(p.lane);
      const cx = (p.left + p.right) / 2;
      markOf[b.i0] = marks.length;
      marks.push({ x: cx, y, r, idx: b.i0, bunch: k, chip: 0, tray: false });
      stemOut.push({ x: stems[k].x, y0: y + r, y1: curveY(b.i1 + 1), k });
      return;
    }
    const top = laneY(p.lane + sh.rows - 1) - r - g.tray;
    const bottom = laneY(p.lane) + r + g.tray;
    trays.push({ left: p.left, top, w: p.right - p.left, h: bottom - top, bunch: k });
    for (let row = 0; row < sh.rows; row++) {
      const inRow = Math.min(sh.cols, sh.cells - row * sh.cols);
      const x0 = p.left + g.tray + ((sh.cols - inRow) * g.pitch) / 2;
      const y = laneY(p.lane + sh.rows - 1 - row);
      for (let c = 0; c < inRow; c++) {
        const cell = row * sh.cols + c;
        const isChip = sh.chip > 0 && cell === sh.cells - 1;
        const x = x0 + c * g.pitch + r;
        if (isChip) {
          for (let i = b.i0 + sh.shown; i <= b.i1; i++) markOf[i] = marks.length;
          marks.push({ x, y, r, idx: -1, bunch: k, chip: sh.chip, tray: true });
        } else {
          markOf[b.i0 + cell] = marks.length;
          marks.push({ x, y, r, idx: b.i0 + cell, bunch: k, chip: 0, tray: true });
        }
      }
    }
    stemOut.push({ x: stems[k].x, y0: bottom, y1: curveY(b.i1 + 1), k });
  });
  const outLabels = labels.map((l) => ({ ...l, y: l.top ? 12 + 13 : laneY(l.lane) }));
  const outLeaders = leaders.map((ld) => {
    const p = packed.placed[ld.k];
    const sh = shapes[ld.k];
    const blockTop = sh.cells === 1 ? laneY(p.lane) - r - 7 : laneY(p.lane + sh.rows - 1) - r - g.tray - 5;
    return { kind: ld.kind, x: ld.x, y0: 12 + 24, y1: blockTop };
  });
  return { d, r, dots: d < 16, laneH: g.laneH, tray: g.tray, lanes, tier: tiers.indexOf(Math.round(g.d)), squeezed: !!g.squeezed, scale, worldW, gutterL, gutterR, H, topPad, laneZoneTop, curveTop, curveBase, curveH, axisH, marks, trays, stems: stemOut, labels: outLabels, leaders: outLeaders, markOf, shapes };
}

/** The mark under (x, y) in the same space as the marks, nearest edge first; `slop` px of forgiveness. */
export function hitTest(marks, x, y, slop = 4) {
  let best = null;
  let bestGap = Infinity;
  for (let k = marks.length - 1; k >= 0; k--) {
    const m = marks[k];
    const dx = x - m.x;
    const dy = y - m.y;
    const dist = Math.sqrt(dx * dx + dy * dy);
    const gap = dist - m.r;
    if (gap <= slop && gap < bestGap) {
      best = m;
      bestGap = gap;
    }
  }
  return best;
}

/**
 * Where a card of `card` size goes beside a marker (`anchor`: x, y, r) inside a stage of `stage`
 * size: above if it fits, else below, else right, else left, always clamped inside by `pad`.
 */
export function placeCard(anchor, card, stage, gap = 12, pad = 6) {
  const clampX = (x) => Math.max(pad, Math.min(x, stage.w - card.w - pad));
  const clampY = (y) => Math.max(pad, Math.min(y, stage.h - card.h - pad));
  const above = anchor.y - anchor.r - gap - card.h;
  if (above >= pad) return { x: clampX(anchor.x - card.w / 2), y: above, side: "above" };
  const below = anchor.y + anchor.r + gap;
  if (below + card.h <= stage.h - pad) return { x: clampX(anchor.x - card.w / 2), y: below, side: "below" };
  const right = anchor.x + anchor.r + gap;
  if (right + card.w <= stage.w - pad) return { x: right, y: clampY(anchor.y - card.h / 2), side: "right" };
  const left = anchor.x - anchor.r - gap - card.w;
  if (left >= pad) return { x: left, y: clampY(anchor.y - card.h / 2), side: "left" };
  return { x: clampX(anchor.x - card.w / 2), y: clampY(anchor.y - card.h - anchor.r - gap), side: "over" };
}

/** The curve of the list so far as `[{ day, n }]` steps, from zero: what the area is drawn from. */
export function stepCurve(model) {
  return model.steps;
}

/** The size of the list on a day (a fractional day is fine): how many species were first heard up to then. */
export function countAt(model, day) {
  let n = 0;
  for (const s of model.steps) {
    if (s.day + 0.5 <= day) n = s.n;
    else break;
  }
  return n;
}

// ── pure: the phone's list ──────────────────────────────────────────────────

/**
 * The species grouped by the month they were first heard, the newest month first and the newest
 * bird first in it. `range` (`{ t0, t1 }`) leaves out what is outside it. `soFar` is the size of
 * the list at the end of that month.
 */
export function monthGroups(species, range) {
  const groups = [];
  let cur = null;
  for (let i = species.length - 1; i >= 0; i--) {
    const s = species[i];
    if (range && (s.day < range.t0 || s.day >= range.t1)) continue;
    const key = monthKey(s.day);
    if (!cur || cur.key !== key) {
      cur = { key, label: monthLabel(key), items: [], soFar: s.rank };
      groups.push(cur);
    }
    cur.items.push(s);
  }
  return groups;
}

/**
 * Where the keyboard cursor goes. `pos` is a position in the chronological order of `n` birds (or
 * null, no cursor yet), `kind` is "stage" or "list". Left and Right walk back and forward in time
 * in both; Up and Down do on the list, where the newest is at the top; Home is the first bird and
 * End the newest. Returns the new position, or null for a key that is not ours.
 */
export function nextIndex(pos, key, n, kind) {
  if (n <= 0) return null;
  const at = pos === null || pos === undefined ? n - 1 : pos;
  const clamp = (v) => Math.max(0, Math.min(n - 1, v));
  const back = key === "ArrowLeft" || (kind === "list" && key === "ArrowDown");
  const fwd = key === "ArrowRight" || (kind === "list" && key === "ArrowUp");
  if (back) return pos === null || pos === undefined ? n - 1 : clamp(at - 1);
  if (fwd) return pos === null || pos === undefined ? n - 1 : clamp(at + 1);
  if (key === "Home") return 0;
  if (key === "End") return n - 1;
  return null;
}

// ── the page ────────────────────────────────────────────────────────────────

const IS_DOM = typeof document !== "undefined" && typeof window !== "undefined";

function url(p) {
  return IS_DOM && typeof window.bnbUrl === "function" ? window.bnbUrl(p) : p;
}

/** A 1px canvas turns any CSS colour (oklch, color-mix of tokens) into the sRGB the page paints. */
let probe = null;
function cssRgb(el, prop) {
  const raw = getComputedStyle(el).getPropertyValue(prop).trim();
  if (!probe) {
    const c = document.createElement("canvas");
    c.width = c.height = 1;
    probe = c.getContext("2d", { willReadFrequently: true });
  }
  probe.clearRect(0, 0, 1, 1);
  // A token the page does not define falls back to the system text colour, never to a colour of ours.
  probe.fillStyle = "canvastext";
  probe.fillStyle = raw || "canvastext";
  probe.fillRect(0, 0, 1, 1);
  const d = probe.getImageData(0, 0, 1, 1).data;
  return [d[0] / 255, d[1] / 255, d[2] / 255];
}

const col = (c, a = 1) => `color(srgb ${c[0].toFixed(4)} ${c[1].toFixed(4)} ${c[2].toFixed(4)} / ${a})`;

function palette(host) {
  const g = (t) => cssRgb(host, t);
  const surface = g("--surface");
  const fg = g("--fg");
  const moss = g("--moss");
  const line = g("--seq-moss-5");
  const fill = g("--seq-moss-3");
  return {
    surface: col(surface),
    sunken: col(g("--surface-sunken")),
    disc: col(g("--border")),
    border2: col(g("--border-2")),
    fg: col(fg),
    fg2: col(g("--fg-2")),
    moss: col(moss),
    mossSoft: col(moss, 0.28),
    line: col(line),
    fillTop: col(fill, 0.5),
    fillBottom: col(fill, 0.06),
    grid: col(g("--chart-grid")),
    gridStrong: col(g("--chart-grid-strong")),
    axis: col(g("--chart-axis")),
    label: col(g("--chart-label")),
    annot: col(g("--chart-annot")),
    stem: col(g("--chart-axis"), 0.9),
    shadow: col(g("--fg"), 0.16),
  };
}

/** A rounded rectangle path (Context2D.roundRect is not in every browser a Pi runs). */
function roundRect(ctx, x, y, w, h, r) {
  const k = Math.min(r, w / 2, h / 2);
  ctx.moveTo(x + k, y);
  ctx.arcTo(x + w, y, x + w, y + h, k);
  ctx.arcTo(x + w, y + h, x, y + h, k);
  ctx.arcTo(x, y + h, x, y, k);
  ctx.arcTo(x, y, x + w, y, k);
  ctx.closePath();
}

function reducedMotion() {
  return (
    (window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches) ||
    document.documentElement.getAttribute("data-motion") === "reduced"
  );
}

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
}

// Pictures: one cache for the page, at most six requests in flight, and only for what is asked for
// right now (the portraits on screen). A failure is remembered, so a 404 is asked once.
const IMGS = new Map();
const IMG_SUBS = new Set();
let imgActive = 0;
let imgQueue = [];

function wantImages(urls) {
  imgQueue = [];
  for (const u of urls) {
    let e = IMGS.get(u);
    if (!e) {
      e = { state: "idle", img: null, t: 0 };
      IMGS.set(u, e);
    }
    if (e.state === "idle") imgQueue.push(u);
  }
  pumpImages();
}

function pumpImages() {
  while (imgActive < 6 && imgQueue.length) {
    const u = imgQueue.shift();
    const e = IMGS.get(u);
    if (!e || e.state !== "idle") continue;
    e.state = "loading";
    imgActive += 1;
    const im = new Image();
    im.decoding = "async";
    const done = (ok) => {
      imgActive -= 1;
      e.state = ok ? "ok" : "fail";
      e.img = ok ? im : null;
      e.t = performance.now();
      IMG_SUBS.forEach((f) => f());
      pumpImages();
    };
    im.onload = () => done(im.naturalWidth > 0);
    im.onerror = () => done(false);
    im.src = u;
  }
}

const DASH = [2, 3];
const NODASH = [];
const easeOut = (t) => 1 - Math.pow(1 - Math.min(1, Math.max(0, t)), 3);
const INTRO_MS = 1100;
const FADE_MS = 200;
const STAGE_MIN_W = 560; // narrower than this is the phone's list
let SEQ = 0;

class Discovery {
  constructor(host) {
    this.host = host;
    this.id = ++SEQ;
    let raw = null;
    try {
      raw = JSON.parse(host.dataset.dscJson || "null");
    } catch (e) {
      raw = null;
    }
    this.model = prepare(raw);
    this.dead = false;
    if (!this.model.total) {
      host.textContent = "";
      host.appendChild(el("p", "bnb-meta dsc-empty", raw === null ? "The timeline is not available just now." : "No birds on the list yet."));
      this.dead = true;
      return;
    }
    const m = this.model;
    m.species.forEach((s) => {
      s.src = s.img ? url(s.img) : null;
      s.href = url(`/species/detail?name=${encodeURIComponent(s.key)}`);
    });
    this.today = m.to;
    this.notes = notesFor(m);
    this.options = rangeOptions(m);
    this.view = { ...m.full };
    this.rangeId = "all";
    this.mode = null;
    this.sel = null; // the keyboard cursor: a species index
    this.hover = null; // the species under the pointer
    this.pinned = false; // a card kept open by a tap, a click or the keyboard
    this.layout = null;
    this.pal = null;
    this.W = 0;
    this.dpr = 1;
    this.off = 0;
    this.onScreen = false;
    this.raf = 0;
    this.lastFrame = 0;
    this.intro = 0;
    this.introT0 = null;
    this.introDone = reducedMotion();
    this.fonts = null;
    this.measureMemo = new Map();
    this.pointers = new Map();
    this.drag = null;
    this.closeTimer = 0;
    this.panelBunch = null;
    this.cleanup = [];
    this.listState = null;
    this.build();
    this.watch();
    this.applyWidth(host.getBoundingClientRect().width || host.clientWidth);
  }

  // ── building the shell ──
  build() {
    const host = this.host;
    host.textContent = "";
    host.classList.add("dsc");
    this.helpId = `dsc-help-${this.id}`;
    const m = this.model;

    const bar = el("div", "dsc-bar");
    const lede = el("p", "dsc-lede");
    lede.appendChild(el("span", "dsc-lede-h", `${fmt(m.total)} species since ${formatShort(m.from)} ${parts(m.from).y}`));
    lede.appendChild(el("span", "dsc-lede-s bnb-meta", "Each bird stands on the day it was first heard. The shaded curve under them is the list so far."));
    bar.appendChild(lede);
    const ctl = el("div", "dsc-ctl");
    if (this.options.length > 1) {
      const seg = el("div", "seg dsc-range");
      seg.setAttribute("role", "group");
      seg.setAttribute("aria-label", "How far back to look");
      for (const o of this.options) {
        const b = el("button", "btn", o.label);
        b.type = "button";
        b.dataset.range = o.id;
        b.setAttribute("aria-pressed", o.id === "all" ? "true" : "false");
        if (o.id === "all") b.classList.add("active");
        seg.appendChild(b);
      }
      ctl.appendChild(seg);
      this.seg = seg;
    }
    const pan = el("span", "dsc-pan");
    pan.hidden = true;
    const mk = (label, text, dir) => {
      const b = el("button", "btn dsc-pan-b", text);
      b.type = "button";
      b.setAttribute("aria-label", label);
      b.dataset.dir = String(dir);
      return b;
    };
    pan.appendChild(mk("Earlier", "←", -1));
    pan.appendChild(mk("Later", "→", 1));
    ctl.appendChild(pan);
    this.pan = pan;
    bar.appendChild(ctl);
    host.appendChild(bar);

    // The stage (wide).
    const stage = el("div", "dsc-stage");
    const canvas = el("canvas", "dsc-canvas");
    canvas.tabIndex = 0;
    canvas.setAttribute("role", "img");
    canvas.setAttribute("aria-describedby", this.helpId);
    stage.appendChild(canvas);
    const card = el("a", "dsc-card");
    card.hidden = true;
    card.tabIndex = -1;
    card.setAttribute("aria-hidden", "true");
    this.cardPic = el("span", "dsc-pic dsc-pic-lg");
    this.cardIni = el("span", "dsc-ini");
    this.cardImg = el("img", "dsc-img");
    this.cardImg.alt = "";
    this.cardImg.decoding = "async";
    this.cardPic.appendChild(this.cardIni);
    this.cardPic.appendChild(this.cardImg);
    const body = el("span", "dsc-card-body");
    this.cardName = el("span", "dsc-card-name");
    this.cardSci = el("span", "dsc-card-sci");
    this.cardWhen = el("span", "dsc-card-line");
    this.cardAgo = el("span", "dsc-ago");
    this.cardCount = el("span", "dsc-card-line");
    this.cardNo = el("span", "dsc-card-line dsc-card-no");
    this.cardGo = el("span", "dsc-card-go", "Open the species page →");
    [this.cardName, this.cardSci, this.cardWhen, this.cardCount, this.cardNo, this.cardGo].forEach((n) => body.appendChild(n));
    card.appendChild(this.cardPic);
    card.appendChild(body);
    stage.appendChild(card);
    const panel = el("div", "dsc-panel");
    panel.hidden = true;
    panel.setAttribute("role", "group");
    panel.setAttribute("aria-label", "Birds first heard that day");
    stage.appendChild(panel);
    host.appendChild(stage);

    // The list (narrow).
    const list = el("div", "dsc-list");
    list.hidden = true;
    host.appendChild(list);

    this.readout = el("p", "dsc-readout bnb-meta");
    this.readout.setAttribute("aria-live", "polite");
    host.appendChild(this.readout);
    this.help = el("p", "dsc-help bnb-meta");
    this.help.id = this.helpId;
    host.appendChild(this.help);

    this.stage = stage;
    this.canvas = canvas;
    this.card = card;
    this.panel = panel;
    this.list = list;
    this.ctx = canvas.getContext("2d");
    this.mctx = document.createElement("canvas").getContext("2d");
    this.bindControls();
    this.bindCanvas();
  }

  watch() {
    const host = this.host;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0].contentRect.width;
      if (Math.abs(w - this.W) >= 1) this.applyWidth(w);
    });
    ro.observe(host);
    this.cleanup.push(() => ro.disconnect());
    const io = new IntersectionObserver((es) => {
      const on = es.some((e) => e.isIntersecting);
      if (on === this.onScreen) return;
      this.onScreen = on;
      if (on) {
        this.updateWanted();
        this.requestDraw();
      }
    });
    io.observe(host);
    this.cleanup.push(() => io.disconnect());
    const redo = () => {
      if (this.dead) return;
      this.pal = palette(host);
      this.requestDraw();
    };
    const mo = new MutationObserver(redo);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-contrast"] });
    this.cleanup.push(() => mo.disconnect());
    let mq = null;
    if (window.matchMedia) {
      mq = window.matchMedia("(prefers-color-scheme: dark)");
      mq.addEventListener("change", redo);
      this.cleanup.push(() => mq.removeEventListener("change", redo));
    }
    const vis = () => {
      if (!document.hidden) this.requestDraw();
    };
    document.addEventListener("visibilitychange", vis);
    this.cleanup.push(() => document.removeEventListener("visibilitychange", vis));
    const imgs = () => this.requestDraw();
    IMG_SUBS.add(imgs);
    this.cleanup.push(() => IMG_SUBS.delete(imgs));
    const hide = () => this.dispose();
    window.addEventListener("pagehide", hide);
    this.cleanup.push(() => window.removeEventListener("pagehide", hide));
    if (document.fonts && document.fonts.ready) {
      const faces = ['italic 15px "Instrument Serif"', '600 11px "Inter Tight"', '400 12px "Inter Tight"'];
      Promise.all(faces.map((f) => document.fonts.load(f).catch(() => null))).then(() => {
        if (this.dead) return;
        this.measureMemo.clear();
        this.fonts = null;
        if (this.mode === "stage") this.relayout();
      });
    }
  }

  dispose() {
    if (this.dead) return;
    this.dead = true;
    cancelAnimationFrame(this.raf);
    clearTimeout(this.closeTimer);
    for (const f of this.cleanup) f();
    this.cleanup = [];
  }

  // ── width decides the form ──
  applyWidth(w) {
    if (this.dead || !(w > 0)) return;
    this.W = Math.floor(w);
    const was = this.mode;
    let mode = was;
    if (!was) mode = w < STAGE_MIN_W ? "list" : "stage";
    else if (was === "list" && w >= STAGE_MIN_W + 40) mode = "stage";
    else if (was === "stage" && w < STAGE_MIN_W) mode = "list";
    if (mode !== was) this.setMode(mode);
    else if (mode === "stage") this.relayout();
  }

  setMode(mode) {
    this.mode = mode;
    this.host.dataset.mode = mode;
    this.stage.hidden = mode !== "stage";
    this.list.hidden = mode !== "list";
    this.readout.hidden = mode !== "stage";
    this.hideCard(true);
    this.closePanel();
    if (mode === "stage") {
      this.help.textContent =
        "Each portrait stands on the day that bird was first heard here, and its stem lands where the list grew. Left and Right arrows move from bird to bird in the order they joined the list; Home is the first, End the newest; Enter opens the species page; Escape closes the card; plus and minus zoom. After you click the picture, the scroll wheel zooms too, and dragging slides it when zoomed.";
      this.pal = this.pal || palette(this.host);
      this.describe();
      this.setReadout(null);
      this.relayout();
    } else {
      this.help.textContent =
        "The newest bird is at the top. Up and Down arrows move from bird to bird, Home goes to the first and End to the newest, and Enter opens the species page. Birds first heard on the same day are numbered alphabetically.";
      this.renderList();
    }
    this.syncControls();
  }

  /** The canvas's text alternative: what the picture is, in one sentence. */
  describe() {
    const m = this.model;
    this.canvas.setAttribute(
      "aria-label",
      `Timeline of firsts: ${fmt(m.total)} species, first heard from ${formatDay(m.from)} to ${formatDay(m.to)}. ${this.notes.newest.charAt(0).toUpperCase()}${this.notes.newest.slice(1)}.`,
    );
  }

  // ── fonts and text ──
  fontSet() {
    if (this.fonts) return this.fonts;
    const cs = getComputedStyle(this.host);
    const ui = cs.getPropertyValue("--font-ui").trim() || "sans-serif";
    const display = cs.getPropertyValue("--font-display").trim() || "serif";
    const mono = cs.getPropertyValue("--font-mono").trim() || "monospace";
    this.fonts = {
      note: `italic 15px ${display}`,
      label: `400 12px ${ui}`,
      axis: `400 11px ${ui}`,
      axisBold: `600 11px ${ui}`,
      num: `400 10px ${mono}`,
      total: `500 12px ${ui}`,
      ini: (px) => `600 ${px}px ${ui}`,
    };
    return this.fonts;
  }

  measureText(text, kind) {
    const key = `${kind}|${text}`;
    let w = this.measureMemo.get(key);
    if (w === undefined) {
      const f = this.fontSet();
      this.mctx.font = kind === "note" ? f.note : f.label;
      w = this.mctx.measureText(text).width;
      this.measureMemo.set(key, w);
    }
    return w;
  }

  // ── controls ──
  bindControls() {
    if (this.seg) {
      this.seg.addEventListener("click", (e) => {
        const b = e.target.closest("button");
        if (!b) return;
        this.setRange(b.dataset.range);
      });
    }
    this.pan.addEventListener("click", (e) => {
      const b = e.target.closest("button");
      if (!b) return;
      const dir = Number(b.dataset.dir);
      this.glide(panView(this.view, dir * (this.view.t1 - this.view.t0) * 0.5, this.model.full));
    });
  }

  syncControls() {
    const whole = this.model.full;
    const zoomed = this.view.t1 - this.view.t0 < whole.t1 - whole.t0 - 0.5;
    this.pan.hidden = !(this.mode === "stage" && zoomed);
    if (this.seg) {
      for (const b of this.seg.querySelectorAll("button")) {
        const on = b.dataset.range === this.rangeId;
        b.classList.toggle("active", on);
        b.setAttribute("aria-pressed", on ? "true" : "false");
      }
    }
    if (this.mode === "stage") {
      this.canvas.classList.toggle("is-pannable", zoomed);
      const [a, b] = this.pan.querySelectorAll("button");
      a.disabled = this.view.t0 <= whole.t0 + 0.01;
      b.disabled = this.view.t1 >= whole.t1 - 0.01;
    }
  }

  setRange(id) {
    const opt = this.options.find((o) => o.id === id);
    if (!opt) return;
    this.rangeId = id;
    this.view = viewForRange(this.model, opt);
    this.hideCard(true);
    this.closePanel();
    if (this.mode === "stage") this.relayout();
    else this.renderList();
    this.syncControls();
  }

  /** Move to `view` (same span: a slide, eased unless motion is off; a different span jumps). */
  glide(next) {
    const from = this.view;
    const sameSpan = Math.abs(from.t1 - from.t0 - (next.t1 - next.t0)) < 0.01;
    this.rangeId = rangeOf(this.model, next);
    this.closePanel();
    if (!sameSpan || reducedMotion() || !this.onScreen) {
      this.view = next;
      this.afterView(!sameSpan);
      return;
    }
    cancelAnimationFrame(this.glideRaf);
    const t0 = performance.now();
    const step = (now) => {
      if (this.dead) return;
      const k = easeOut((now - t0) / 260);
      this.view = { t0: from.t0 + (next.t0 - from.t0) * k, t1: from.t1 + (next.t1 - from.t1) * k };
      this.afterView(false);
      if (k < 1) this.glideRaf = requestAnimationFrame(step);
      else {
        this.view = next;
        this.afterView(false);
      }
    };
    this.glideRaf = requestAnimationFrame(step);
  }

  afterView(relayout) {
    if (relayout) this.relayout();
    else this.moveView();
    this.syncControls();
  }

  // ── stage: layout and drawing ──
  relayout() {
    if (this.mode !== "stage" || this.dead) return;
    const W = this.W;
    const f = this.fontSet();
    void f;
    const plotW = Math.max(120, W - 38 - 30);
    const span = this.view.t1 - this.view.t0;
    const scale = plotW / span;
    this.plotW = plotW;
    this.layout = layoutStage(this.model, { width: W, scale, measure: (t, k) => this.measureText(t, k), notes: this.notes, gutterL: 38, gutterR: 30, minH: 300 });
    const L = this.layout;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    this.dpr = dpr;
    const c = this.canvas;
    const cw = Math.round(W * dpr);
    const ch = Math.round(L.H * dpr);
    if (c.width !== cw || c.height !== ch) {
      c.width = cw;
      c.height = ch;
    }
    this.stage.style.height = `${L.H}px`;
    this.moveView();
  }

  /** The view moved (same scale): recompute what depends on where the edge of the picture is. */
  moveView() {
    const L = this.layout;
    if (!L) return;
    const full = this.model.full;
    this.off = (this.view.t0 - full.t0) * L.scale;
    this.ticks = axisTicks(this.view, this.plotW);
    this.lead = leadYear(this.view, this.plotW, this.ticks);
    this.cticks = countTicks(this.model.total);
    this.buildCurve();
    this.updateWanted();
    this.refreshCard();
    this.requestDraw();
  }

  /** Paths of the list-so-far curve in picture coordinates. */
  buildCurve() {
    const L = this.layout;
    const steps = this.model.steps;
    const full = this.model.full;
    const x = (day) => L.gutterL + (day + 0.5 - full.t0) * L.scale - this.off;
    const xEnd = L.gutterL + (full.t1 - full.t0) * L.scale - this.off;
    const total = this.model.total;
    const y = (n) => L.curveBase - (L.curveH * n) / total;
    const line = new Path2D();
    const area = new Path2D();
    const x0 = x(steps[0].day);
    line.moveTo(x0, L.curveBase);
    area.moveTo(x0, L.curveBase);
    let prev = 0;
    for (const s of steps) {
      const sx = x(s.day);
      line.lineTo(sx, y(prev));
      line.lineTo(sx, y(s.n));
      area.lineTo(sx, y(prev));
      area.lineTo(sx, y(s.n));
      prev = s.n;
    }
    line.lineTo(xEnd, y(prev));
    area.lineTo(xEnd, y(prev));
    area.lineTo(xEnd, L.curveBase);
    area.closePath();
    this.curve = { line, area, xEnd };
  }

  /** Ask for the pictures of the portraits that are on screen right now, newest first. */
  updateWanted() {
    const L = this.layout;
    if (!L || this.mode !== "stage" || !this.onScreen || L.d < 16) return;
    const urls = [];
    const lo = this.off - 30;
    const hi = this.off + this.W + 30;
    const sp = this.model.species;
    for (let k = L.marks.length - 1; k >= 0; k--) {
      const m = L.marks[k];
      if (m.idx < 0 || m.x < lo || m.x > hi) continue;
      const s = sp[m.idx];
      if (s.src) urls.push(s.src);
    }
    if (this.shownIdx !== null && this.shownIdx !== undefined && sp[this.shownIdx] && sp[this.shownIdx].src) urls.unshift(sp[this.shownIdx].src);
    wantImages(urls);
  }

  canDraw() {
    return !this.dead && this.mode === "stage" && this.layout && this.pal && this.onScreen && !document.hidden;
  }

  requestDraw() {
    if (this.raf || this.dead) return;
    this.raf = requestAnimationFrame((t) => {
      this.raf = 0;
      this.frame(t);
    });
  }

  frame(t) {
    if (!this.canDraw()) return;
    let more = false;
    if (!this.introDone) {
      if (this.introT0 === null) this.introT0 = t;
      this.intro = Math.min(1, (t - this.introT0) / INTRO_MS);
      if (this.intro >= 1) this.introDone = true;
      else more = true;
    }
    if (this.draw(performance.now())) more = true;
    if (more) {
      // Animations run at about 30 frames a second: a frame closer than that to the last is skipped.
      this.raf = requestAnimationFrame((n) => {
        this.raf = 0;
        if (n - this.lastFrame < 30) {
          this.requestDraw();
          return;
        }
        this.lastFrame = n;
        this.frame(n);
      });
    }
  }

  tx(day) {
    return this.layout.gutterL + (day - this.model.full.t0) * this.layout.scale - this.off;
  }

  /** The state of the cursor on `idx`: "hover", "focus" or null. */
  activeIdx() {
    return this.hover !== null ? this.hover : this.pinned ? this.sel : null;
  }

  draw(now) {
    const L = this.layout;
    const P = this.pal;
    const ctx = this.ctx;
    const F = this.fontSet();
    const W = this.W;
    const dpr = this.dpr;
    const m = this.model;
    let animating = false;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, L.H);
    const reveal = this.introDone ? 1 : easeOut(this.intro);
    const front = this.introDone ? Infinity : reveal * (W + 90) - 20;
    const off = this.off;
    // Month lines.
    ctx.lineWidth = 1;
    for (let pass = 0; pass < 2; pass++) {
      ctx.strokeStyle = pass ? P.gridStrong : P.grid;
      ctx.beginPath();
      for (const t of this.ticks) {
        if (t.kind !== "month" || !!t.year !== !!pass) continue;
        const x = Math.round(this.tx(t.day)) + 0.5;
        if (x < 0 || x > W) continue;
        ctx.moveTo(x, L.laneZoneTop - 4);
        ctx.lineTo(x, L.curveBase);
      }
      ctx.stroke();
    }
    // Count lines.
    ctx.strokeStyle = P.grid;
    ctx.fillStyle = P.label;
    ctx.font = F.num;
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    ctx.beginPath();
    for (const v of this.cticks) {
      const y = Math.round(L.curveBase - (L.curveH * v) / m.total) + 0.5;
      ctx.moveTo(0, y);
      ctx.lineTo(W, y);
    }
    ctx.stroke();
    for (const v of this.cticks) {
      const y = L.curveBase - (L.curveH * v) / m.total;
      ctx.fillText(String(v), 6, y - 7);
    }
    // The list so far.
    ctx.save();
    if (front !== Infinity) {
      ctx.beginPath();
      ctx.rect(0, 0, Math.max(0, front), L.H);
      ctx.clip();
    }
    const grad = ctx.createLinearGradient(0, L.curveTop, 0, L.curveBase);
    grad.addColorStop(0, P.fillTop);
    grad.addColorStop(1, P.fillBottom);
    ctx.fillStyle = grad;
    ctx.fill(this.curve.area);
    ctx.strokeStyle = P.line;
    ctx.lineWidth = 2;
    ctx.lineJoin = "round";
    ctx.stroke(this.curve.line);
    ctx.restore();
    // Baseline and the time axis.
    ctx.strokeStyle = P.axis;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, Math.round(L.curveBase) + 0.5);
    ctx.lineTo(W, Math.round(L.curveBase) + 0.5);
    for (const t of this.ticks) {
      const x = Math.round(this.tx(t.day)) + 0.5;
      if (x < 0 || x > W) continue;
      ctx.moveTo(x, L.curveBase);
      ctx.lineTo(x, L.curveBase + (t.kind === "month" ? 5 : 3));
    }
    ctx.stroke();
    ctx.fillStyle = P.label;
    ctx.textBaseline = "alphabetic";
    ctx.textAlign = "left";
    const yMonth = L.curveBase + 19;
    const yYear = L.curveBase + 34;
    let lastEnd = -1e9;
    for (const t of this.ticks) {
      const x = this.tx(t.day) + 4;
      if (x < -40 || x > W) continue;
      ctx.font = t.year ? F.axisBold : F.axis;
      const w = ctx.measureText(t.label).width;
      if (t.kind === "week" && x < lastEnd + 6) continue;
      ctx.fillText(t.label, x, yMonth);
      lastEnd = x + w;
      if (t.year) {
        ctx.font = F.axisBold;
        ctx.fillText(t.year, x, yYear);
      }
    }
    if (this.lead) {
      ctx.font = F.axisBold;
      ctx.fillText(this.lead, 6, yYear);
    }
    // Stems and the places they land.
    const lo = -60;
    const hi = W + 60;
    ctx.strokeStyle = P.stem;
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (const s of L.dots ? [] : L.stems) {
      const x = Math.round(s.x - off) + 0.5;
      if (x < lo || x > hi) continue;
      if (s.x - off > front) continue;
      ctx.moveTo(x, s.y0);
      ctx.lineTo(x, s.y1);
    }
    ctx.stroke();
    const act = this.activeIdx();
    if (act !== null && !L.dots) {
      const am = L.marks[L.markOf[act]];
      const st = am && L.stems[am.bunch];
      if (st && st.x - off > lo && st.x - off < hi) {
        ctx.strokeStyle = P.fg2;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(Math.round(st.x - off) + 0.5, st.y0);
        ctx.lineTo(Math.round(st.x - off) + 0.5, st.y1);
        ctx.stroke();
        ctx.lineWidth = 1;
      }
    }
    if (L.leaders.length) {
      ctx.setLineDash(DASH);
      ctx.strokeStyle = P.axis;
      ctx.beginPath();
      for (const ld of L.leaders) {
        const x = Math.round(ld.x - off) + 0.5;
        if (x < lo || x > hi || x > front || ld.y1 <= ld.y0) continue;
        ctx.moveTo(x, ld.y0);
        ctx.lineTo(x, ld.y1);
      }
      ctx.stroke();
      ctx.setLineDash(NODASH);
    }
    ctx.fillStyle = P.line;
    for (const s of L.dots ? [] : L.stems) {
      const x = s.x - off;
      if (x < lo || x > hi || x > front) continue;
      ctx.beginPath();
      ctx.arc(x, s.y1, 3, 0, 6.2832);
      ctx.fill();
    }
    // Trays.
    for (const t of L.dots ? [] : L.trays) {
      const x = t.left - off;
      if (x + t.w < lo || x > hi || x > front) continue;
      ctx.fillStyle = P.surface;
      ctx.strokeStyle = P.border2;
      ctx.lineWidth = 1;
      ctx.beginPath();
      roundRect(ctx, Math.round(x) + 0.5, Math.round(t.top) + 0.5, t.w, t.h, Math.min(16, L.r + L.tray));
      ctx.fill();
      ctx.stroke();
    }
    // Discs.
    const sp = m.species;
    const newestDay = m.bunches[m.bunches.length - 1];
    const active = this.activeIdx();
    const activeMark = active === null ? -1 : L.markOf[active];
    for (let k = 0; k < L.marks.length; k++) {
      const mk = L.marks[k];
      const x = mk.x - off;
      if (x < lo || x > hi || k === activeMark) continue;
      let sc = 1;
      let al = 1;
      if (front !== Infinity) {
        const p = Math.min(1, Math.max(0, (front - x) / 70));
        if (p <= 0) continue;
        sc = 0.55 + 0.45 * easeOut(p);
        al = p;
      }
      if (this.drawDisc(ctx, mk, x, sp, newestDay, sc, al, now, false)) animating = true;
    }
    if (activeMark >= 0) {
      const mk = L.marks[activeMark];
      if (this.drawDisc(ctx, mk, mk.x - off, sp, newestDay, 1.14, 1, now, true)) animating = true;
    }
    // Names and notes.
    ctx.textBaseline = "middle";
    const noteAlpha = this.introDone ? 1 : Math.min(1, Math.max(0, (this.intro - 0.8) / 0.2));
    for (const l of L.labels) {
      const x = l.x - off;
      if (x < -200 || x > W + 200) continue;
      ctx.textAlign = l.align;
      if (l.kind === "name") {
        if (front !== Infinity && x > front) continue;
        ctx.font = F.label;
        ctx.fillStyle = P.label;
        ctx.globalAlpha = 1;
      } else {
        ctx.font = F.note;
        ctx.fillStyle = P.annot;
        ctx.globalAlpha = noteAlpha;
      }
      ctx.fillText(l.text, x, l.y + (l.kind === "name" ? 0 : -1));
    }
    ctx.globalAlpha = 1;
    // The size of the list at the right-hand edge.
    const nAtEdge = countAt(m, this.view.t1 - 0.01);
    const yEdge = L.curveBase - (L.curveH * nAtEdge) / m.total;
    ctx.font = F.total;
    ctx.fillStyle = P.annot;
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    const ctxLabel = `${fmt(nAtEdge)} species`;
    const lw = ctx.measureText(ctxLabel).width;
    ctx.globalAlpha = noteAlpha;
    let lx = Math.min(W - 10, this.curve.xEnd - 8);
    const x0 = this.tx(m.steps[0].day + 0.5);
    // A list that began a few days ago has a riser close to the right edge: say the size beside it.
    if (lx - lw - 6 < x0 + 2 && lx > x0) lx = x0 - 8;
    // Inside the area, near the baseline, clear of the steps above it.
    const ly = L.curveBase - yEdge > 30 ? L.curveBase - 15 : yEdge - 11;
    ctx.fillText(ctxLabel, lx, ly);
    ctx.globalAlpha = 1;
    void lw;
    return animating;
  }

  /** One portrait: its picture or its initials, the ring that says newest, the ring that says "this one". */
  drawDisc(ctx, mk, x, sp, newest, scale, alpha, now, active) {
    const P = this.pal;
    const F = this.fontSet();
    const r = mk.r * scale;
    const y = mk.y;
    let animating = false;
    ctx.globalAlpha = alpha;
    if (this.layout.dots) {
      // Too many for portraits: a dot each, in the curve's own colour, the newest ringed.
      ctx.fillStyle = mk.idx < 0 ? P.fg2 : P.line;
      ctx.beginPath();
      ctx.arc(x, y, Math.max(1.3, r * scale), 0, 6.2832);
      ctx.fill();
      if (mk.idx >= 0 && sp[mk.idx].day === newest.day) {
        ctx.strokeStyle = P.moss;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(x, y, Math.max(1.3, r) + 3, 0, 6.2832);
        ctx.stroke();
      }
      if (active) {
        ctx.strokeStyle = P.fg;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(x, y, Math.max(1.3, r) + 4, 0, 6.2832);
        ctx.stroke();
      }
      ctx.globalAlpha = 1;
      return false;
    }
    if (!mk.tray) {
      ctx.fillStyle = P.surface;
      ctx.beginPath();
      ctx.arc(x, y, r + 2.5, 0, 6.2832);
      ctx.fill();
    }
    if (mk.idx < 0) {
      // The "+N" chip.
      ctx.fillStyle = P.surface;
      ctx.strokeStyle = P.border2;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(x, y, r, 0, 6.2832);
      ctx.fill();
      ctx.stroke();
      if (r >= 8) {
        ctx.fillStyle = P.fg2;
        ctx.font = F.ini(r >= 17 ? 12 : 10);
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(`+${mk.chip}`, x, y + 0.5);
      }
    } else {
      const s = sp[mk.idx];
      ctx.fillStyle = P.disc;
      ctx.beginPath();
      ctx.arc(x, y, r, 0, 6.2832);
      ctx.fill();
      const e = s.src ? IMGS.get(s.src) : null;
      let photo = 0;
      if (e && e.state === "ok") {
        photo = 1;
        if (!reducedMotion()) {
          photo = Math.min(1, (now - e.t) / FADE_MS);
          if (photo < 1) animating = true;
        }
      }
      if (photo < 1 && r >= 7) {
        ctx.fillStyle = P.fg2;
        ctx.font = F.ini(r >= 17 ? 10.5 : r >= 13 ? 10 : 9);
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(initials(s, r >= 16.5), x, y + 0.5);
      }
      if (photo > 0) {
        const im = e.img;
        const side = Math.min(im.naturalWidth, im.naturalHeight);
        ctx.save();
        ctx.beginPath();
        ctx.arc(x, y, r, 0, 6.2832);
        ctx.clip();
        ctx.globalAlpha = alpha * photo;
        ctx.drawImage(im, (im.naturalWidth - side) / 2, (im.naturalHeight - side) / 2, side, side, x - r, y - r, r * 2, r * 2);
        ctx.restore();
        ctx.globalAlpha = alpha;
      }
      ctx.strokeStyle = P.border2;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(x, y, r, 0, 6.2832);
      ctx.stroke();
      if (s.day === newest.day) {
        ctx.strokeStyle = P.moss;
        ctx.lineWidth = 3;
        ctx.beginPath();
        ctx.arc(x, y, r + 3.5, 0, 6.2832);
        ctx.stroke();
      }
    }
    if (active) {
      ctx.strokeStyle = P.surface;
      ctx.lineWidth = 3;
      ctx.beginPath();
      ctx.arc(x, y, r + 2.5, 0, 6.2832);
      ctx.stroke();
      ctx.strokeStyle = P.fg;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(x, y, r + 4.5, 0, 6.2832);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
    return animating;
  }

  // ── stage: the pointer, the keyboard ──
  point(e) {
    const r = this.canvas.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  markAt(e, touch) {
    if (!this.layout) return null;
    const p = this.point(e);
    return hitTest(this.layout.marks, p.x + this.off, p.y, touch ? 14 : 4);
  }

  bindCanvas() {
    const c = this.canvas;
    const on = (t, f, o) => {
      c.addEventListener(t, f, o);
      this.cleanup.push(() => c.removeEventListener(t, f, o));
    };
    on("pointerdown", (e) => {
      if (this.mode !== "stage") return;
      this.pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
      try {
        c.setPointerCapture(e.pointerId);
      } catch (err) {
        /* a synthetic pointer */
      }
      if (this.pointers.size === 2) {
        const [a, b] = [...this.pointers.values()];
        this.pinch = { d: Math.hypot(a.x - b.x, a.y - b.y), view: { ...this.view } };
        this.drag = null;
        return;
      }
      this.drag = { id: e.pointerId, x: e.clientX, view: { ...this.view }, moved: false, touch: e.pointerType !== "mouse", mark: this.markAt(e, e.pointerType !== "mouse") };
    });
    on("pointermove", (e) => {
      if (this.mode !== "stage") return;
      if (this.pointers.has(e.pointerId)) this.pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
      if (this.pinch && this.pointers.size >= 2) {
        const [a, b] = [...this.pointers.values()];
        const d = Math.hypot(a.x - b.x, a.y - b.y);
        const rect = c.getBoundingClientRect();
        const mid = (a.x + b.x) / 2 - rect.left;
        const anchor = xToDay(mid - this.layout.gutterL, this.pinch.view, this.plotW);
        this.view = zoomView(this.pinch.view, this.pinch.d / Math.max(20, d), anchor, this.model.full);
        this.rangeId = rangeOf(this.model, this.view);
        this.drag = null;
        this.afterView(true);
        return;
      }
      if (this.drag && this.drag.id === e.pointerId) {
        const dx = e.clientX - this.drag.x;
        if (!this.drag.moved && Math.abs(dx) > 5) this.drag.moved = true;
        const full = this.model.full;
        const zoomed = this.drag.view.t1 - this.drag.view.t0 < full.t1 - full.t0 - 0.5;
        if (this.drag.moved && zoomed) {
          this.setHover(null);
          this.view = panView(this.drag.view, -dx / this.layout.scale, full);
          this.rangeId = rangeOf(this.model, this.view);
          c.classList.add("is-drag");
          this.afterView(false);
        }
        return;
      }
      if (e.pointerType === "mouse" && !this.pointers.size) {
        const mk = this.markAt(e, false);
        this.setHover(mk);
      }
    });
    const up = (e) => {
      this.pointers.delete(e.pointerId);
      if (this.pinch && this.pointers.size < 2) this.pinch = null;
      const d = this.drag;
      c.classList.remove("is-drag");
      if (!d || d.id !== e.pointerId) return;
      this.drag = null;
      if (d.moved || e.type === "pointercancel") return;
      const mk = this.markAt(e, d.touch);
      this.tap(mk, d.touch);
    };
    on("pointerup", up);
    on("pointercancel", up);
    on("pointerleave", (e) => {
      if (e.pointerType === "mouse" && !this.pointers.size) this.setHover(null, true);
    });
    on("keydown", (e) => this.key(e));
    on("focus", () => {
      this.focused = true;
      // Tabbing in opens the card on the newest bird (or where the cursor was); a click that
      // happens to focus the picture does not: the click itself says what it wants.
      let byKeys = false;
      try {
        byKeys = c.matches(":focus-visible");
      } catch (err) {
        byKeys = true;
      }
      if (byKeys) this.moveTo(this.sel === null ? this.model.total - 1 : this.sel, true);
      this.requestDraw();
    });
    on("blur", (e) => {
      this.focused = false;
      // Focus moving onto the card or the panel (a click on it) must not take it away first.
      if (e.relatedTarget && this.stage.contains(e.relatedTarget)) return;
      this.pinned = false;
      this.hideCard(false);
      this.requestDraw();
    });
    on(
      "wheel",
      (e) => {
        // A bare wheel scrolls the page. Once the picture has the focus, it zooms.
        if (this.mode !== "stage" || document.activeElement !== c) return;
        e.preventDefault();
        const p = this.point(e);
        const anchor = xToDay(p.x - this.layout.gutterL, this.view, this.plotW);
        this.view = zoomView(this.view, Math.exp(e.deltaY * 0.0016), anchor, this.model.full);
        this.rangeId = rangeOf(this.model, this.view);
        this.closePanel();
        this.afterView(true);
      },
      { passive: false },
    );
    // The card keeps itself open while the pointer is on it.
    const cardIn = () => clearTimeout(this.closeTimer);
    const cardOut = (e) => {
      if (e.pointerType === "mouse") this.setHover(null, true);
    };
    this.card.addEventListener("pointerenter", cardIn);
    this.card.addEventListener("pointerleave", cardOut);
    this.panel.addEventListener("pointerenter", cardIn);
    this.cleanup.push(() => {
      this.card.removeEventListener("pointerenter", cardIn);
      this.card.removeEventListener("pointerleave", cardOut);
      this.panel.removeEventListener("pointerenter", cardIn);
    });
    const outside = (e) => {
      if (this.mode !== "stage") return;
      if (this.stage.contains(e.target)) return;
      this.pinned = false;
      this.hideCard(false);
      this.closePanel();
      this.requestDraw();
    };
    document.addEventListener("pointerdown", outside);
    this.cleanup.push(() => document.removeEventListener("pointerdown", outside));
  }

  speciesOf(mk) {
    return mk && mk.idx >= 0 ? mk.idx : null;
  }

  setHover(mk, leaving = false) {
    if (mk && mk.idx < 0) {
      // A "+N" chip: the card for its first hidden species is not shown; the panel is the way in.
      this.canvas.classList.add("is-hot");
      this.hoverChip = mk;
      if (this.hover !== null) {
        this.hover = null;
        this.refreshCard();
        this.requestDraw();
      }
      return;
    }
    this.hoverChip = null;
    const idx = this.speciesOf(mk);
    this.canvas.classList.toggle("is-hot", idx !== null);
    if (idx === null) {
      if (this.hover === null) return;
      clearTimeout(this.closeTimer);
      const close = () => {
        this.hover = null;
        this.refreshCard();
        this.requestDraw();
      };
      if (leaving || this.cardShown()) this.closeTimer = setTimeout(close, 260);
      else close();
      return;
    }
    clearTimeout(this.closeTimer);
    if (idx === this.hover) return;
    this.hover = idx;
    this.refreshCard();
    this.requestDraw();
  }

  cardShown() {
    return !this.card.hidden;
  }

  tap(mk, touch) {
    if (!mk) {
      this.pinned = false;
      this.hover = null;
      this.hideCard(false);
      this.closePanel();
      this.requestDraw();
      return;
    }
    if (mk.idx < 0) {
      this.openPanel(mk);
      return;
    }
    this.closePanel();
    if (!touch) {
      this.go(mk.idx);
      return;
    }
    if (this.pinned && this.sel === mk.idx) {
      this.go(mk.idx);
      return;
    }
    this.sel = mk.idx;
    this.pinned = true;
    this.hover = null;
    this.refreshCard();
    this.requestDraw();
  }

  go(idx) {
    const s = this.model.species[idx];
    if (!s) return;
    this.card.href = s.href;
    this.card.click();
  }

  key(e) {
    if (this.mode !== "stage" || e.altKey || e.ctrlKey || e.metaKey) return;
    const n = this.model.total;
    if (e.key === "Escape") {
      if (!this.panel.hidden) this.closePanel();
      this.pinned = false;
      this.hover = null;
      this.hideCard(false);
      this.setReadout(null);
      this.requestDraw();
      return;
    }
    if (e.key === "Enter") {
      if (this.sel !== null) {
        e.preventDefault();
        this.go(this.sel);
      }
      return;
    }
    if (e.key === "+" || e.key === "=" || e.key === "-" || e.key === "_" || e.key === "0") {
      e.preventDefault();
      const full = this.model.full;
      const mid = this.sel !== null ? this.model.species[this.sel].day + 0.5 : (this.view.t0 + this.view.t1) / 2;
      const next = e.key === "0" ? { ...full } : zoomView(this.view, e.key === "+" || e.key === "=" ? 0.6 : 1 / 0.6, mid, full);
      this.glide(next);
      return;
    }
    if (e.key === "PageUp" || e.key === "PageDown") {
      const full = this.model.full;
      if (this.view.t1 - this.view.t0 < full.t1 - full.t0 - 0.5) {
        e.preventDefault();
        this.glide(panView(this.view, (e.key === "PageUp" ? -0.5 : 0.5) * (this.view.t1 - this.view.t0), full));
      }
      return;
    }
    const to = nextIndex(this.sel, e.key, n, "stage");
    if (to === null) return;
    e.preventDefault();
    this.moveTo(to, true);
  }

  /** Put the cursor on a species: card, reading, and slide it into view if it is off screen. */
  moveTo(idx, pan) {
    this.sel = idx;
    this.pinned = true;
    this.hover = null;
    this.closePanel();
    const s = this.model.species[idx];
    if (pan) {
      const mk = this.layout.marks[this.layout.markOf[idx]];
      const sx = mk.x - this.off;
      if (sx < 40 || sx > this.W - 40) {
        const span = this.view.t1 - this.view.t0;
        const t0 = s.day + 0.5 - span / 2;
        const next = clampView({ t0, t1: t0 + span }, this.model.full, span);
        if (!sameView(next, this.view, 0.01)) this.glide(next);
      }
    }
    void s;
    this.refreshCard();
    this.requestDraw();
  }

  // ── the card, the reading ──
  /** The species the card is about right now: what is pointed at, else the cursor if it is held. */
  currentIdx() {
    if (this.hover !== null) return this.hover;
    return this.pinned ? this.sel : null;
  }

  refreshCard() {
    if (this.mode !== "stage") return;
    const idx = this.currentIdx();
    this.shownIdx = idx;
    if (idx === null) {
      this.hideCard(false);
      this.setReadout(null);
      return;
    }
    const s = this.model.species[idx];
    this.fillCard(s);
    this.setReadout(s);
    this.positionCard(idx);
  }

  fillCard(s) {
    const c = this.card;
    if (c.dataset.i === String(s.i)) return;
    c.dataset.i = String(s.i);
    c.href = s.href;
    this.cardName.textContent = s.name;
    this.cardSci.textContent = s.sci;
    this.cardSci.hidden = !s.sci;
    this.cardWhen.textContent = `First heard ${formatDay(s.day)} `;
    this.cardAgo.textContent = `(${agoPhrase(s.day, this.today)})`;
    this.cardWhen.appendChild(this.cardAgo);
    const cp = countPhrase(s);
    this.cardCount.textContent = cp ? cp.charAt(0).toUpperCase() + cp.slice(1) : "";
    this.cardCount.hidden = !cp;
    this.cardNo.textContent = `Number ${fmt(s.rank)} on the list`;
    this.cardIni.textContent = initials(s, true);
    this.cardPic.classList.remove("has-img");
    this.cardImg.removeAttribute("src");
    const e = s.src ? IMGS.get(s.src) : null;
    if (s.src && !(e && e.state === "fail")) {
      this.cardImg.onload = () => this.cardPic.classList.add("has-img");
      this.cardImg.onerror = () => {
        this.cardPic.classList.remove("has-img");
        this.cardImg.removeAttribute("src");
      };
      this.cardImg.src = s.src;
    }
  }

  positionCard(idx) {
    const L = this.layout;
    const c = this.card;
    const mk = L.marks[L.markOf[idx]];
    if (!mk) return;
    const a = { x: mk.x - this.off, y: mk.y, r: mk.r + 4 };
    // Scrolled out of sight: the card waits (the cursor is still there) rather than float over nothing.
    if (a.x < -mk.r || a.x > this.W + mk.r) {
      c.hidden = true;
      return;
    }
    c.hidden = false;
    const size = { w: c.offsetWidth, h: c.offsetHeight };
    const pos = placeCard(a, size, { w: this.W, h: L.H });
    c.style.left = `${Math.round(Math.max(6, Math.min(pos.x, this.W - size.w - 6)))}px`;
    c.style.top = `${Math.round(Math.max(6, Math.min(pos.y, L.H - size.h - 6)))}px`;
    c.dataset.side = pos.side;
  }

  hideCard(force) {
    if (force) {
      this.pinned = false;
      this.hover = null;
    }
    this.card.hidden = true;
    delete this.card.dataset.i;
    this.shownIdx = null;
    if (force) this.setReadout(null);
  }

  setReadout(s) {
    if (this.mode !== "stage") return;
    const text = s ? readingText(this.model, s) : `${idleText(this.model)} Point at a portrait, or use the arrow keys, to read it.`;
    if (this.readout.textContent !== text) this.readout.textContent = text;
  }

  // ── the "+N" panel ──
  openPanel(mk) {
    const L = this.layout;
    const b = this.model.bunches[mk.bunch];
    const sh = L.shapes[mk.bunch];
    const p = this.panel;
    p.textContent = "";
    p.appendChild(el("p", "dsc-panel-h", `${fmt(b.n)} birds first heard ${formatDay(b.day)}`));
    const ul = el("ul", "dsc-panel-l");
    for (let i = b.i0; i <= b.i1; i++) {
      const s = this.model.species[i];
      const li = el("li");
      const a = el("a", "dsc-panel-a");
      a.href = s.href;
      a.appendChild(el("span", "dsc-panel-n", s.name));
      if (s.count > 0) a.appendChild(el("span", "dsc-panel-c", fmt(s.count)));
      li.appendChild(a);
      ul.appendChild(li);
    }
    p.appendChild(ul);
    void sh;
    this.hideCard(true);
    p.hidden = false;
    const size = { w: p.offsetWidth, h: p.offsetHeight };
    const pos = placeCard({ x: mk.x - this.off, y: mk.y, r: mk.r + 4 }, size, { w: this.W, h: L.H });
    p.style.left = `${Math.round(pos.x)}px`;
    p.style.top = `${Math.round(pos.y)}px`;
    this.panelBunch = mk.bunch;
    this.setReadoutRaw(`${fmt(b.n)} birds were first heard on ${formatDay(b.day)}; the panel lists them.`);
  }

  closePanel() {
    if (this.panel.hidden) return;
    this.panel.hidden = true;
    this.panelBunch = null;
  }

  setReadoutRaw(text) {
    this.readout.textContent = text;
  }

  // ── the phone's list ──
  rangeFilter() {
    const full = this.model.full;
    return this.view.t0 <= full.t0 + 0.01 && this.view.t1 >= full.t1 - 0.01 ? null : this.view;
  }

  renderList() {
    const list = this.list;
    list.textContent = "";
    const groups = monthGroups(this.model.species, this.rangeFilter());
    const months = el("div", "dsc-months");
    list.appendChild(months);
    this.listState = { groups, next: 0, months, rows: new Map(), more: null, order: [], stop: null };
    const st = this.listState;
    for (const g of groups) for (let i = g.items.length - 1; i >= 0; i--) st.order.push(g.items[i].i);
    st.order.sort((a, b) => a - b);
    months.addEventListener("keydown", (e) => this.listKey(e));
    this.renderMore(24);
    const newest = st.order[st.order.length - 1];
    if (newest !== undefined) this.setStop(newest);
    if (!groups.length) list.appendChild(el("p", "bnb-meta dsc-none", "No bird was first heard in this range."));
  }

  /** Add whole months until at least `min` more rows are there, and the "show earlier" button. */
  renderMore(min) {
    const st = this.listState;
    if (!st) return;
    let added = 0;
    while (st.next < st.groups.length && added < min) {
      const g = st.groups[st.next++];
      st.months.appendChild(this.monthEl(g));
      added += g.items.length;
    }
    if (st.more) st.more.remove();
    st.more = null;
    if (st.next < st.groups.length) {
      let left = 0;
      for (let i = st.next; i < st.groups.length; i++) left += st.groups[i].items.length;
      const b = el("button", "btn dsc-more", `Show ${plural(left, "earlier bird", "earlier birds")}`);
      b.type = "button";
      b.addEventListener("click", () => {
        const before = st.next;
        this.renderMore(30);
        const row = st.months.querySelector(`.dsc-mo:nth-child(${before + 1}) .dsc-row`);
        if (row) row.focus();
      });
      this.list.appendChild(b);
      st.more = b;
    }
  }

  monthEl(g) {
    const m = this.model;
    const sec = el("section", "dsc-mo");
    const h = el("h3", "dsc-mh");
    h.appendChild(el("span", "dsc-mh-t", g.label));
    h.appendChild(el("span", "dsc-mh-m", `${g.items.length === 1 ? "1 new" : `${g.items.length} new`} · ${g.soFar} on the list`));
    if (m.bunches.length > 1 && m.bunches[0].n > 1 && g.items.some((s) => s.day === m.bunches[0].day)) {
      h.appendChild(el("span", "dsc-mh-note", `the first day: ${m.bunches[0].n} species`));
    }
    sec.appendChild(h);
    const ol = el("ol", "dsc-rows");
    const newestDay = m.bunches[m.bunches.length - 1].day;
    for (const s of g.items) {
      const li = el("li");
      const a = el("a", "dsc-row");
      a.href = s.href;
      a.dataset.i = String(s.i);
      a.tabIndex = -1;
      a.setAttribute("aria-label", readingText(m, s));
      a.appendChild(el("span", "dsc-no", fmt(s.rank)));
      const pic = el("span", "dsc-pic");
      pic.appendChild(el("span", "dsc-ini", initials(s, true)));
      if (s.day === newestDay) pic.classList.add("is-newest");
      pic.dataset.src = s.src || "";
      a.appendChild(pic);
      const t = el("span", "dsc-txt");
      t.appendChild(el("span", "dsc-nm", s.name));
      t.appendChild(el("span", "dsc-dt", `${formatDay(s.day)} · ${agoPhrase(s.day, m.to)}`));
      const cp = countPhrase(s);
      if (cp) t.appendChild(el("span", "dsc-ct", cp.charAt(0).toUpperCase() + cp.slice(1)));
      if (s.day === newestDay) t.appendChild(el("span", "dsc-note", s.i === m.total - 1 || m.bunches[m.bunches.length - 1].n === 1 ? "the newest on your list" : "among the newest on your list"));
      else if (s.i === 0 && m.bunches[0].n === 1) t.appendChild(el("span", "dsc-note", "the first on your list"));
      a.appendChild(t);
      a.addEventListener("focus", () => this.setStop(s.i));
      li.appendChild(a);
      ol.appendChild(li);
      this.listState.rows.set(s.i, a);
      this.lazyPic(pic, s);
    }
    sec.appendChild(ol);
    return sec;
  }

  /** A row's photo is requested when the row comes near the screen, never before. */
  lazyPic(pic, s) {
    if (!s.src) return;
    if (!this.picIO) {
      this.picIO = new IntersectionObserver(
        (es) => {
          for (const e of es) {
            if (!e.isIntersecting) continue;
            this.picIO.unobserve(e.target);
            const u = e.target.dataset.src;
            if (!u) continue;
            const ent = IMGS.get(u);
            if (ent && ent.state === "fail") continue;
            const img = el("img", "dsc-img");
            img.alt = "";
            img.decoding = "async";
            img.onload = () => e.target.classList.add("has-img");
            img.onerror = () => {
              img.remove();
              IMGS.set(u, { state: "fail", img: null, t: 0 });
            };
            img.src = u;
            e.target.appendChild(img);
          }
        },
        { rootMargin: "240px 0px" },
      );
      this.cleanup.push(() => this.picIO && this.picIO.disconnect());
    }
    this.picIO.observe(pic);
  }

  setStop(idx) {
    const st = this.listState;
    if (!st) return;
    if (st.stop !== null) {
      const old = st.rows.get(st.stop);
      if (old) old.tabIndex = -1;
    }
    const row = st.rows.get(idx);
    if (row) row.tabIndex = 0;
    st.stop = idx;
    this.sel = idx;
  }

  listKey(e) {
    const st = this.listState;
    if (!st || e.altKey || e.ctrlKey || e.metaKey) return;
    const row = e.target.closest ? e.target.closest(".dsc-row") : null;
    if (!row) return;
    const pos = st.order.indexOf(Number(row.dataset.i));
    const to = nextIndex(pos < 0 ? null : pos, e.key, st.order.length, "list");
    if (to === null) return;
    e.preventDefault();
    const idx = st.order[to];
    while (!st.rows.has(idx) && st.next < st.groups.length) this.renderMore(30);
    const target = st.rows.get(idx);
    if (target) target.focus();
  }
}

// ── boot ────────────────────────────────────────────────────────────────────

const registry = new Set();

/** Start every `[data-dsc]` under `root` that has not been started, and let go of the ones that left the page. */
export function boot(root) {
  if (!IS_DOM) return;
  const scope = root && root.querySelectorAll ? root : document;
  scope.querySelectorAll("[data-dsc]").forEach((host) => {
    if (host.dataset.dscBound) return;
    host.dataset.dscBound = "1";
    try {
      registry.add(new Discovery(host));
    } catch (err) {
      host.textContent = "";
      host.appendChild(el("p", "bnb-meta dsc-empty", "The timeline is not available just now."));
    }
  });
  for (const d of registry) {
    if (!d.host.isConnected) {
      d.dispose();
      registry.delete(d);
    }
  }
}

if (IS_DOM) {
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", () => boot());
  else boot();
  document.addEventListener("htmx:afterSwap", () => boot());
}
