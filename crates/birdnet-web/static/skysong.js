// Sky song: every song heard today, as a note on the day's own sky.
//
// Across is the time of day (00:00 to 24:00); up is how sure the station was, from its accept
// cut-off to 100 %. Each song is one small glowing note in the colour of its species, all one
// size: where it sits already says how sure the station was, so the size says nothing. Behind
// the notes is the day's real sky (night, first light, day, last light, night), laid out from the
// station's own sunrise and sunset, with the sun's arc over the top and a low horizon that the
// cut-off line IS: nothing below it was ever accepted, so nothing is drawn there.
//
// The card (the host `<div class="ss" data-ss data-ss-feed="/pages/today-songs">`) is empty on
// purpose: this module fetches the feed, builds everything inside the host and refreshes it every
// 30 s while the picture is on screen and the tab is visible.
//
// What it promises, and what holds it to it (skysong.test.mjs):
//   * A note's height is exactly how sure the station was (`sureToY`), never moved. Its place
//     along the day is exactly its time, except that songs which would land on the very same
//     spot (same minute, same height) are fanned out sideways by a fixed step, at most
//     `MAX_SHIFT_R` note radii (`layoutSongs`). The reading always says the true time.
//   * Nothing is drawn that the data does not say: with no location there is no sun, no moon and no
//     twilight (a neutral sky), and the sun's disc sits at the current time only while it is up.
//   * The picture says in one plain sentence what it encodes (the lede), and the sentence under it
//     says what it found (`summarize`). Everything a hover shows is in the readout, the key and the
//     accessible label; the pointer, a finger and the keyboard give the same reading.
//   * Reduced motion (the OS, or the operator's Motion setting) means no entrance sweep and no
//     shimmer: the picture simply is. Nothing runs while the picture is off screen or the tab hidden.
//   * No colour is written in this file: every one is read from the stylesheet's tokens.
//
// An ES module so its pure parts can be exported and tested under node. It starts itself only
// where there is a document.

export const DAY = 86400;
export const SLOTS = 8;
export const POLL_MS = 30000;
/** The most a note is moved sideways to leave room for another on the same spot, in note radii. */
export const MAX_SHIFT_R = 3;
/** The spacing between notes fanned out from one spot, in note radii. */
export const FAN_R = 1.9;
const MAX_SONGS = 4000;
const NBSP = " ";

// ── pure: numbers ───────────────────────────────────────────────────────────

export const clamp = (v, lo, hi) => (v < lo ? lo : v > hi ? hi : v);
const p2 = (n) => String(n).padStart(2, "0");

/** `08:14` for seconds since local midnight (the minute it falls in, never rounded up). */
export function clockOfSecond(t) {
  const m = Math.floor(clamp(Number(t) || 0, 0, DAY - 1) / 60);
  return `${p2(Math.floor(m / 60))}:${p2(m % 60)}`;
}

/** `06:48` for local fractional hours (6.8), to the nearest minute. */
export function clockOfHour(h) {
  let m = Math.round(clamp(Number(h) || 0, 0, 24) * 60);
  if (m >= 1440) m = 1439;
  return `${p2(Math.floor(m / 60))}:${p2(m % 60)}`;
}

/** The x (px) of `t` seconds since midnight on an axis from `left` to `right` (00:00 .. 24:00). */
export function timeToX(t, left, right) {
  return left + (clamp(t, 0, DAY) / DAY) * (right - left);
}

/** The time (seconds) at x px: the inverse of `timeToX`, clamped to the day. */
export function xToTime(x, left, right) {
  const w = right - left;
  return w > 0 ? clamp(((x - left) / w) * DAY, 0, DAY) : 0;
}

/** The share of the way from the cut-off (0) to a sure 100 % (1) of how sure `c` is. */
export function sureShare(c, floor) {
  const range = Math.max(1e-6, 1 - floor);
  return clamp((c - floor) / range, 0, 1);
}

/** The y (px) of "how sure" `c`: the cut-off `floor` sits on `bottom`, 100 % on `top`. */
export function sureToY(c, floor, top, bottom) {
  return bottom - sureShare(c, floor) * (bottom - top);
}

/** The "how sure" at y px: the inverse of `sureToY`. */
export function yToSure(y, floor, top, bottom) {
  const h = bottom - top;
  return h > 0 ? floor + clamp((bottom - y) / h, 0, 1) * (1 - floor) : floor;
}

/** A percentage for the axis and the reading: whole, or one decimal when the cut-off is not whole. */
export function pctText(c) {
  const v = c * 100;
  const whole = Math.round(v);
  return Math.abs(v - whole) < 0.05 ? String(whole) : v.toFixed(1);
}

/** How sure, for a reading: a whole percent (never above 100). */
export function surePct(c) {
  return clamp(Math.round(c * 100), 0, 100);
}

/** Hours on the time axis: every three on a wide picture, every six on a phone. */
export function hourTicks(narrow) {
  const step = narrow ? 6 : 3;
  const out = [];
  for (let h = 0; h <= 24; h += step) out.push(h);
  return out;
}

/**
 * Where the "how sure" axis is labelled: the cut-off at the bottom, 100 % at the top, and (when
 * the span is wide enough to need one) a round figure nearest the middle, preferring a multiple
 * of ten when two are equally near. Fractions of 1, ascending.
 */
export function yTicks(floor) {
  const f = clamp(floor, 0, 0.99);
  const out = [f];
  const span = (1 - f) * 100;
  if (span >= 12) {
    const mid = f * 100 + span / 2;
    let best = null;
    for (let v = Math.ceil((f * 100 + span * 0.25) / 5) * 5; v <= f * 100 + span * 0.75; v += 5) {
      const d = Math.abs(v - mid);
      if (!best || d < best.d - 1e-9 || (Math.abs(d - best.d) < 1e-9 && v % 10 === 0 && best.v % 10 !== 0)) best = { v, d };
    }
    if (best) out.push(best.v / 100);
  }
  out.push(1);
  return out;
}

// ── pure: the sky ───────────────────────────────────────────────────────────

/** The sun, if the station knows where it is: finite rise and set with the set after the rise. */
export function cleanSun(sun) {
  if (!sun || typeof sun !== "object") return null;
  const rise = Number(sun.rise);
  const set = Number(sun.set);
  if (!Number.isFinite(rise) || !Number.isFinite(set) || sun.rise === null || sun.set === null) return null;
  if (!(set > rise)) return null;
  const opt = (v) => (v === null || v === undefined || !Number.isFinite(Number(v)) ? null : Number(v));
  return { rise: clamp(rise, 0, 24), set: clamp(set, 0, 24), dawn: opt(sun.dawn), dusk: opt(sun.dusk) };
}

/**
 * The wash behind the notes, left to right, as colour stops: `{ o, k }` with `o` 0..1 along the
 * day and `k` one of `night`, `dawn`, `day`, `dusk`. Night until first light (civil dawn, or half an
 * hour before sunrise when the station gives none), the dawn colour at sunrise, full day a little
 * after, and the same mirrored at sunset. Offsets never go backwards: a short winter day squeezes the
 * day wash out instead of inverting it. `null` when there is no sun: the sky is then neutral.
 */
export function skyStops(sun) {
  const s = cleanSun(sun);
  if (!s) return null;
  const rise = s.rise;
  const set = s.set;
  const mid = (rise + set) / 2;
  const first = clamp(s.dawn ?? rise - 0.55, 0, rise);
  const last = clamp(s.dusk ?? set + 0.55, set, 24);
  // The wash starts a little before first light and ends a little after last light, so the
  // edge of the night is a gradient the eye reads as a horizon glow and not as a seam.
  const hours = [
    ["night", 0],
    ["night", Math.max(0, first - 0.9)],
    ["dawn", rise],
    ["day", Math.min(rise + 2.2, mid)],
    ["day", Math.max(set - 2.2, mid)],
    ["dusk", set],
    ["night", Math.min(24, last + 0.9)],
    ["night", 24],
  ];
  // Already in order for any sun `cleanSun` lets through (rise < set, first light no later than
  // sunrise, last light no earlier than sunset); a short winter day squeezes the day wash because of
  // the `Math.min`/`Math.max` against the midpoint, and the test holds every case to it.
  return hours.map(([k, h]) => ({ o: clamp(h / 24, 0, 1), k }));
}

/** sRGB (0..1 each) mixed `t` of the way from `a` to `b` in OKLab, so a gradient keeps its hue and its lightness steps evenly. */
export function mixOklab(a, b, t) {
  const lin = (c) => (c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4));
  const encodeSrgb = (c) => (c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 1 / 2.4) - 0.055);
  const toLab = (c) => {
    const r = lin(c[0]);
    const g = lin(c[1]);
    const bl = lin(c[2]);
    const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * bl);
    const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * bl);
    const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * bl);
    return [0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s, 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s];
  };
  const A = toLab(a);
  const B = toLab(b);
  const L = A[0] + (B[0] - A[0]) * t;
  const u = A[1] + (B[1] - A[1]) * t;
  const v = A[2] + (B[2] - A[2]) * t;
  const l = Math.pow(L + 0.3963377774 * u + 0.2158037573 * v, 3);
  const m = Math.pow(L - 0.1055613458 * u - 0.0638541728 * v, 3);
  const s = Math.pow(L - 0.0894841775 * u - 1.291485548 * v, 3);
  const out = [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s];
  return out.map((c) => clamp(encodeSrgb(clamp(c, 0, 1)), 0, 1));
}

/** How high the sun is, 0 (not up) to 1 (the top of its arc), at local hour `h`: a half sine. */
export function sunHeight(h, sun) {
  const s = cleanSun(sun);
  if (!s || h <= s.rise || h >= s.set) return 0;
  return Math.sin((Math.PI * (h - s.rise)) / (s.set - s.rise));
}

/**
 * Where the moon sits: the middle of the longer stretch of night (hours), or null when neither
 * is at least `minHours` long (the picture has no room for one) or the station has no sun.
 */
export function moonHour(sun, minHours) {
  const s = cleanSun(sun);
  if (!s) return null;
  const morning = s.dawn ?? s.rise - 0.55;
  const evening = s.dusk ?? s.set + 0.55;
  const a = Math.max(0, morning);
  const b = Math.max(0, 24 - evening);
  if (Math.max(a, b) < minHours) return null;
  return a >= b ? a / 2 : evening + b / 2;
}

// ── pure: the feed ──────────────────────────────────────────────────────────

/**
 * The feed as the picture uses it, or null when it is not a feed. Defensive: a song whose time or
 * sureness is not a number is dropped, a species index out of range falls to "Other species", at
 * most `MAX_SONGS` songs are read. Parallel typed arrays, so 2,000 songs cost four arrays.
 */
export function parseFeed(json) {
  if (!json || typeof json !== "object" || json.error || !Array.isArray(json.songs)) return null;
  const floor = Number.isFinite(Number(json.floor)) && json.floor !== null ? clamp(Number(json.floor), 0, 0.99) : 0;
  const species = [];
  for (const sp of Array.isArray(json.species) ? json.species : []) {
    if (!sp || typeof sp.name !== "string") continue;
    const slot = Number.isInteger(sp.slot) && sp.slot >= 1 && sp.slot <= SLOTS ? sp.slot : 0;
    const n = Number(sp.n);
    species.push({ name: sp.name.slice(0, 160), slot, n: Number.isFinite(n) && n >= 0 ? n : null });
  }
  let other = species.findIndex((s) => s.slot === 0);
  const raw = json.songs.length > MAX_SONGS ? json.songs.slice(0, MAX_SONGS) : json.songs;
  const t = [];
  const c = [];
  const s = [];
  const href = [];
  for (const song of raw) {
    if (!song) continue;
    const tt = Number(song.t);
    const cc = Number(song.c);
    if (!Number.isFinite(tt) || !Number.isFinite(cc) || song.t === null || song.c === null) continue;
    let si = Number.isInteger(song.s) ? song.s : -1;
    if (si < 0 || si >= species.length) {
      if (other < 0) {
        species.push({ name: "Other species", slot: 0, n: null });
        other = species.length - 1;
      }
      si = other;
    }
    t.push(clamp(tt, 0, DAY - 1));
    c.push(clamp(cc, 0, 1));
    s.push(si);
    href.push(typeof song.href === "string" ? song.href : "");
  }
  const counts = new Array(species.length).fill(0);
  for (const si of s) counts[si] += 1;
  const total = Number(json.total);
  const speciesTotal = Number(json.speciesTotal);
  return {
    date: typeof json.date === "string" ? json.date : "",
    nowSec: json.nowSec === null || json.nowSec === undefined || !Number.isFinite(Number(json.nowSec)) ? null : clamp(Number(json.nowSec), 0, DAY - 1),
    sun: cleanSun(json.sun),
    floor,
    species: species.map((sp, i) => ({ name: sp.name, slot: sp.slot, n: sp.n !== null && sp.n >= counts[i] ? sp.n : counts[i] })),
    n: t.length,
    t: Float64Array.from(t),
    c: Float64Array.from(c),
    s: Int16Array.from(s),
    href,
    total: Number.isFinite(total) && total > t.length ? total : t.length,
    speciesTotal: Number.isFinite(speciesTotal) && speciesTotal > 0 ? Math.round(speciesTotal) : null,
  };
}

/** A song's identity across two fetches (the same song never changes these). */
export function songKey(m, i) {
  return `${m.t[i]}|${m.s[i] >= 0 && m.species[m.s[i]] ? m.species[m.s[i]].name : ""}|${m.c[i].toFixed(4)}`;
}

/** The set of keys of every song in `m`. */
export function keySet(m) {
  const out = new Set();
  for (let i = 0; i < m.n; i++) out.add(songKey(m, i));
  return out;
}

/** The indices of the songs of `m` that are not in `before` (a `keySet`): what arrived since. */
export function arrivals(before, m) {
  const out = [];
  if (!before) return out;
  for (let i = 0; i < m.n; i++) if (!before.has(songKey(m, i))) out.push(i);
  return out;
}

/** Song indices in time order (ties: the less sure first, then the order given). */
export function timeOrder(m) {
  const idx = new Int32Array(m.n);
  for (let i = 0; i < m.n; i++) idx[i] = i;
  return idx.sort((a, b) => m.t[a] - m.t[b] || m.c[a] - m.c[b] || a - b);
}

/**
 * Song indices in the order they are painted, the bottom first: "Other species" first, then the
 * species most heard to least, so the rare ones are never buried under the common ones; within a
 * species the less sure first, so the surest song of a crowd is on top.
 */
export function paintOrder(m) {
  const rank = (i) => (m.species[m.s[i]] && m.species[m.s[i]].slot === 0 ? -1 : m.s[i]);
  const idx = new Int32Array(m.n);
  for (let i = 0; i < m.n; i++) idx[i] = i;
  return idx.sort((a, b) => rank(a) - rank(b) || m.c[a] - m.c[b] || m.t[a] - m.t[b] || a - b);
}

// ── pure: the sentence ──────────────────────────────────────────────────────

const group = (n) => Math.round(n).toLocaleString("en-US");

/**
 * What the picture found, in one sentence, for the line under it.
 * `opts.today` says the day is still going ("so far", "the most recent").
 */
export function summarize(m, opts = {}) {
  const today = !!opts.today;
  if (!m || m.n === 0) return today ? "Nothing has been heard yet today." : "Nothing was heard that day.";
  let first = Infinity;
  let last = -Infinity;
  const seen = new Set();
  for (let i = 0; i < m.n; i++) {
    if (m.t[i] < first) first = m.t[i];
    if (m.t[i] > last) last = m.t[i];
    seen.add(m.s[i]);
  }
  const confs = Array.from(m.c).sort((a, b) => a - b);
  const lower = confs[Math.floor((confs.length - 1) / 2)];
  // The lower median: more than half of the songs are at least this sure.
  const t5 = Math.floor((lower * 100 + 1e-9) / 5) * 5;
  const floorPct = pctText(m.floor);
  let sure;
  if (t5 >= 100) sure = "most were 100 % sure";
  else if (t5 / 100 <= m.floor + 1e-9) sure = `most were only just above the station's ${floorPct} % cut-off`;
  else sure = `most were ${t5} % sure or better`;
  const tail = m.total > m.n ? ` The picture shows ${group(m.n)} of them.` : "";
  if (m.n === 1) {
    const name = m.species[m.s[0]] ? m.species[m.s[0]].name : "bird";
    return `One song${today ? " so far" : ""}, ${name} at ${clockOfSecond(m.t[0])}, ${surePct(m.c[0])} % sure.${tail}`;
  }
  let kinds = 0;
  let hasOther = false;
  for (const si of seen) {
    if (m.species[si] && m.species[si].slot === 0) hasOther = true;
    else kinds += 1;
  }
  let speciesText;
  if (m.speciesTotal) speciesText = `${group(m.speciesTotal)} species`;
  else if (hasOther) speciesText = `at least ${group(kinds + 1)} species`;
  else speciesText = `${group(kinds)} ${kinds === 1 ? "species" : "species"}`;
  const songs = m.total > m.n ? m.total : m.n;
  const when =
    clockOfSecond(first) === clockOfSecond(last)
      ? `all at ${clockOfSecond(first)}`
      : `the first at ${clockOfSecond(first)} and ${today ? "the most recent" : "the last"} at ${clockOfSecond(last)}`;
  return `${group(songs)} songs from ${speciesText}${today ? " so far" : ""}, ${when}; ${sure}.${tail}`;
}

/** One song as a reading: `Blue Jay at 08:14, 87 % sure.` */
export function readingText(m, i) {
  const sp = m.species[m.s[i]];
  return `${sp ? sp.name : "A bird"} at ${clockOfSecond(m.t[i])}, ${surePct(m.c[i])} % sure.`;
}

/** The pieces of the card over the picture: `{ name, time, sure }`. */
export function cardParts(m, i) {
  const sp = m.species[m.s[i]];
  return { name: sp ? sp.name : "A bird", time: clockOfSecond(m.t[i]), sure: `${surePct(m.c[i])} % sure` };
}

// ── pure: the layout ────────────────────────────────────────────────────────

/**
 * The picture's boxes for a stage `w` by `h` px. `narrow` is the phone layout: a narrower gutter,
 * a taller sky band and ground, bigger notes. The plot runs from `left` to `right` along the day
 * and from `top` (100 % sure) down to `bottom` (the cut-off: the horizon, where the ground begins).
 */
export function geometry(w, h, narrow) {
  const left = narrow ? 46 : 52;
  const right = Math.max(left + 40, w - (narrow ? 14 : 20));
  const band = narrow ? 50 : 58;
  const ground = narrow ? 50 : 52;
  const top = band + (narrow ? 14 : 14);
  const bottom = Math.max(top + 60, h - ground);
  return { w, h, left, right, top, bottom, band, ground, narrow: !!narrow };
}

/**
 * The radius of one note: one size for every song on a given day, smaller when the day is very
 * busy so that a crowd stays a crowd of notes and does not become a solid sheet.
 */
export function noteRadius(n, narrow) {
  const base = narrow ? 6.4 : 5.2;
  const crowd = clamp(Math.sqrt(Math.max(0, n - 100) / 1900), 0, 1);
  return base * (1 - 0.5 * crowd);
}

/**
 * Where each song is drawn. y is exactly how sure the station was. x is exactly the time, except
 * that songs that would land on the same spot (the same minute and the same height, to within
 * 0.8 of a radius) are fanned out sideways, in `FAN_R` radii steps, never more than `MAX_SHIFT_R`
 * radii from where they belong. The order inside a spot is the less sure first, then the earlier, so the set
 * of places never depends on the order the server happened to list the songs in.
 * Returns `{ x, y }` Float32Arrays of px.
 */
export function layoutSongs(t, c, g, r, floor) {
  const n = t.length;
  const x = new Float32Array(n);
  const y = new Float32Array(n);
  const spots = new Map();
  const row = Math.max(0.5, r * 0.8);
  for (let i = 0; i < n; i++) {
    x[i] = timeToX(t[i], g.left, g.right);
    y[i] = sureToY(c[i], floor, g.top, g.bottom);
    const key = Math.floor(clamp(t[i], 0, DAY - 1) / 60) * 100003 + Math.round(y[i] / row);
    const list = spots.get(key);
    if (list) list.push(i);
    else spots.set(key, [i]);
  }
  for (const list of spots.values()) {
    if (list.length < 2) continue;
    list.sort((a, b) => c[a] - c[b] || t[a] - t[b] || a - b);
    const cap = MAX_SHIFT_R * r;
    for (let k = 0; k < list.length; k++) {
      x[list[k]] += clamp((k - (list.length - 1) / 2) * FAN_R * r, -cap, cap);
    }
  }
  return { x, y };
}

/**
 * The song under a point: the nearest of those painted, within `maxDist` px, and the one on top
 * when two are equally near. `order` is the paint order (bottom first); `active` marks the songs
 * that can be read (or null for all).
 */
export function nearestSong(xs, ys, order, active, px, py, maxDist) {
  const lim = maxDist * maxDist;
  let best = -1;
  let bestD = Infinity;
  for (let k = 0; k < order.length; k++) {
    const i = order[k];
    if (active && !active[i]) continue;
    const dx = xs[i] - px;
    const dy = ys[i] - py;
    const d = dx * dx + dy * dy;
    if (d <= lim && d <= bestD) {
      best = i;
      bestD = d;
    }
  }
  return best;
}

/**
 * One step through the songs in time order. `time` is `timeOrder`; `from` a song index or -1;
 * `dir` +1 or -1; `active` marks the songs that can be read. Stops at the ends (no wrap), and
 * with nothing selected starts at the first (+1) or the last (-1).
 */
export function stepTime(time, active, from, dir) {
  const ok = (i) => !active || active[i];
  let pos = from < 0 ? (dir > 0 ? -1 : time.length) : time.indexOf(from);
  if (pos < 0) pos = dir > 0 ? -1 : time.length;
  for (let k = pos + dir; k >= 0 && k < time.length; k += dir) if (ok(time[k])) return time[k];
  return from;
}

/** The first or last readable song in time order, or -1. */
export function endSong(time, active, last) {
  if (last) {
    for (let k = time.length - 1; k >= 0; k--) if (!active || active[time[k]]) return time[k];
  } else {
    for (let k = 0; k < time.length; k++) if (!active || active[time[k]]) return time[k];
  }
  return -1;
}

/**
 * Move to the next (+1) or previous (-1) species that has a readable song, landing on its song
 * nearest in time to `from`. Wraps round, so Up and Down never dead-end. Returns the song, or
 * `from` when there is no other species to go to.
 */
export function stepSpecies(m, active, from, dir) {
  const k = m.species.length;
  if (from < 0 || k < 2) return from;
  for (let step = 1; step < k; step++) {
    const sp = (((m.s[from] + dir * step) % k) + k) % k;
    let best = -1;
    let bestD = Infinity;
    for (let i = 0; i < m.n; i++) {
      if (m.s[i] !== sp || (active && !active[i])) continue;
      const d = Math.abs(m.t[i] - m.t[from]);
      if (d < bestD) {
        best = i;
        bestD = d;
      }
    }
    if (best >= 0) return best;
  }
  return from;
}

/** Whether `href` is a path on this site (a root-relative path), never another scheme or origin. */
export function safeHref(href) {
  return typeof href === "string" && href.length > 1 && href.charAt(0) === "/" && href.charAt(1) !== "/" && href.charAt(1) !== "\\";
}

// ── the page ────────────────────────────────────────────────────────────────

/** A 1px canvas turns any CSS colour (oklch, color-mix of tokens) into the sRGB the page paints. */
let probe = null;
function cssRgb(el, prop) {
  const raw = getComputedStyle(el).getPropertyValue(prop).trim();
  if (!probe) {
    const cv = document.createElement("canvas");
    cv.width = cv.height = 1;
    probe = cv.getContext("2d", { willReadFrequently: true });
  }
  probe.clearRect(0, 0, 1, 1);
  // A token the page does not define falls back to the system text colour, never to a colour of
  // ours: no colour is written in this file, they are all read from the stylesheet.
  probe.fillStyle = "canvastext";
  probe.fillStyle = raw || "canvastext";
  probe.fillRect(0, 0, 1, 1);
  const d = probe.getImageData(0, 0, 1, 1).data;
  return [d[0] / 255, d[1] / 255, d[2] / 255];
}

const css = (c, a) => `color(srgb ${c[0].toFixed(4)} ${c[1].toFixed(4)} ${c[2].toFixed(4)}${a === undefined || a >= 1 ? "" : ` / ${Math.max(0, a).toFixed(3)}`})`;
const mixRgb = (a, b, t) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
const luma = (c) => 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];

function palette(host) {
  const tok = (name) => cssRgb(host, name);
  const chart = [];
  for (let i = 1; i <= SLOTS; i++) chart.push(tok(`--chart-${i}`));
  chart.push(tok("--chart-other"));
  const dim = parseFloat(getComputedStyle(host).getPropertyValue("--chart-dim"));
  const surface = tok("--surface");
  return {
    chart,
    dim: Number.isFinite(dim) ? dim : 0.35,
    night: tok("--sky-night"),
    dawn: tok("--sky-dawn"),
    day: tok("--sky-day"),
    dusk: tok("--sky-dusk"),
    grid: tok("--chart-grid-strong"),
    axis: tok("--chart-axis"),
    label: tok("--chart-label"),
    annot: tok("--chart-annot"),
    gap: tok("--chart-gap"),
    track: tok("--chart-track"),
    sunken: tok("--surface-sunken"),
    surface,
    sun: tok("--dawn"),
    sunCore: tok("--seq-1"),
    fg: tok("--fg"),
    dark: luma(surface) < 0.4,
  };
}

function fonts(host) {
  const cs = getComputedStyle(host);
  return {
    mono: cs.getPropertyValue("--font-mono").trim() || "monospace",
    ui: cs.getPropertyValue("--font-ui").trim() || "sans-serif",
    display: cs.getPropertyValue("--font-display").trim() || "serif",
  };
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

function url(p) {
  return typeof window.bnbUrl === "function" ? window.bnbUrl(p) : p;
}

const slotClass = (slot) => (slot >= 1 && slot <= SLOTS ? `ck-c${slot}` : "ck-cother");
const easeOut = (t) => 1 - Math.pow(1 - clamp(t, 0, 1), 3);

let counter = 0;

class SkySong {
  constructor(host) {
    this.host = host;
    this.id = ++counter;
    this.feed = host.dataset.ssFeed || "";
    // Optional `data-ss-poll` (ms) lets a test refresh faster than the 30 s a station uses.
    this.poll = Number(host.dataset.ssPoll) >= 500 ? Number(host.dataset.ssPoll) : POLL_MS;
    this.hover = false;
    this.readKey = "";
    this.keySig = "";
    this.headGrad = null;
    this.beamKey = "";
    this.m = null;
    this.keys = null;
    this.pal = null;
    this.fonts = null;
    this.narrow = false;
    this.w = 0;
    this.h = 0;
    this.dpr = 1;
    this.g = null;
    this.r = 4.6;
    this.lay = null;
    this.paint = new Int32Array(0);
    this.time = new Int32Array(0);
    this.active = new Uint8Array(0);
    this.skip = new Uint8Array(0);
    this.solo = -1;
    this.cur = -1; // the song being read
    this.tapped = -1; // the song a finger had selected before the latest touch
    this.chips = [];
    this.status = "";
    this.fetchedAt = 0;
    this.nowAt = 0;
    this.onScreen = false;
    this.disposed = false;
    this.busy = false;
    this.timer = 0;
    this.raf = 0;
    this.dirty = { back: true, notes: true };
    this.sweep = null; // { t0 } while the entrance runs; false once it has
    this.fresh = new Map(); // song -> start ms, for the shimmer
    this.lastFrame = 0;
    this.sprites = null;
    this.layers = {};
    this.build();
    this.bind();
    this.load();
  }

  // ── markup ──
  build() {
    const h = this.host;
    h.textContent = "";
    h.setAttribute("data-ss-ready", "1");
    this.lede = el("p", "ss-lede bnb-meta");
    this.stage = el("div", "ss-stage");
    this.canvas = el("canvas", "ss-canvas");
    this.canvas.setAttribute("role", "img");
    this.canvas.tabIndex = 0;
    this.canvas.setAttribute("aria-label", "Songs heard today (loading)");
    this.helpId = `ss-help-${this.id}`;
    this.canvas.setAttribute("aria-describedby", this.helpId);
    this.card = el("a", "ss-card");
    this.card.hidden = true;
    this.card.tabIndex = -1;
    this.card.setAttribute("aria-hidden", "true");
    this.cardDot = el("i", "ss-card-dot");
    this.cardName = el("b", "ss-card-name");
    this.cardRest = el("span", "ss-card-rest");
    this.card.append(this.cardDot, this.cardName, this.cardRest);
    this.msg = el("p", "ss-msg bnb-meta");
    this.stage.append(this.canvas, this.card, this.msg);
    // The sentence under the picture says what it found and is not live (it changes whenever a song
    // arrives, and a screen reader should not read it out every half minute). The line under it is
    // the live one: the song being read, how to read one, or that the picture could not refresh.
    this.sum = el("p", "ss-sum");
    this.read = el("p", "ss-read");
    this.read.setAttribute("aria-live", "polite");
    this.key = el("div", "ss-key");
    this.key.setAttribute("role", "group");
    this.key.setAttribute("aria-label", "Species heard. Press one to see only that species; press it again to see them all.");
    this.help = el("p", "ss-help bnb-meta");
    this.help.id = this.helpId;
    this.help.textContent =
      "Each note is one song. Point at a note, or tap it, to read it; press Enter or tap it again to open the recording. " +
      "With the keyboard, Left and Right move through the songs in time order, Up and Down jump to the next species, " +
      "Home and End go to the first and last, Escape lets go.";
    h.append(this.lede, this.stage, this.sum, this.read, this.key, this.help);
    this.ctx = this.canvas.getContext("2d");
  }

  bind() {
    const c = this.canvas;
    c.addEventListener("pointermove", (e) => this.onPointer(e, false));
    c.addEventListener("pointerdown", (e) => {
      this.tapped = e.pointerType === "mouse" ? -1 : this.cur;
      this.onPointer(e, true);
    });
    c.addEventListener("pointerleave", (e) => {
      if (e.pointerType === "mouse") this.select(-1);
    });
    c.addEventListener("click", (e) => this.onClick(e));
    c.addEventListener("keydown", (e) => this.onKey(e));
    c.addEventListener("focus", () => {
      // A keyboard arrival reads the most recent song; a click on the picture does not.
      if (this.m && this.cur < 0 && c.matches(":focus-visible")) this.select(endSong(this.time, this.active, true), false);
    });
    c.addEventListener("blur", () => {
      if (!this.hover) this.select(-1);
    });
    this.card.addEventListener("click", (e) => {
      e.preventDefault();
      this.open(this.cur);
    });

    this.ro = new ResizeObserver(() => this.resize());
    this.ro.observe(this.stage);
    if ("IntersectionObserver" in window) {
      this.io = new IntersectionObserver((es) => {
        const was = this.onScreen;
        this.onScreen = es.some((e) => e.isIntersecting);
        if (this.onScreen && !was) this.wake();
      });
      this.io.observe(this.stage);
    } else this.onScreen = true;

    this.onVis = () => {
      if (!document.hidden) this.wake();
    };
    document.addEventListener("visibilitychange", this.onVis);
    this.onHide = () => this.dispose();
    window.addEventListener("pagehide", this.onHide);

    const redo = () => {
      if (this.disposed) return;
      this.pal = palette(this.host);
      this.fonts = fonts(this.host);
      this.sprites = null;
      this.headGrad = null;
      this.beamKey = "";
      this.dirty.back = this.dirty.notes = true;
      this.schedule();
    };
    this.mo = new MutationObserver(redo);
    this.mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-contrast"] });
    if (window.matchMedia) {
      this.mq = window.matchMedia("(prefers-color-scheme: dark)");
      this.mq.addEventListener("change", redo);
      this.redo = redo;
    }
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(() => this.refont());
  }

  /** The label font may arrive after the first paint: draw again once it has. */
  refont() {
    if (this.disposed) return;
    this.dirty.back = true;
    this.schedule();
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    clearTimeout(this.timer);
    cancelAnimationFrame(this.raf);
    if (this.ro) this.ro.disconnect();
    if (this.io) this.io.disconnect();
    if (this.mo) this.mo.disconnect();
    if (this.mq && this.redo) this.mq.removeEventListener("change", this.redo);
    document.removeEventListener("visibilitychange", this.onVis);
    window.removeEventListener("pagehide", this.onHide);
  }

  /** Gone from the page (swapped out)? Then stop for good. */
  alive() {
    if (this.disposed) return false;
    if (!this.host.isConnected) {
      this.dispose();
      return false;
    }
    return true;
  }

  // ── data ──
  async load() {
    if (!this.feed) return;
    if (this.busy) return;
    this.busy = true;
    let data = null;
    let ok = false;
    try {
      const ctl = typeof AbortController === "function" ? new AbortController() : null;
      const to = ctl ? setTimeout(() => ctl.abort(), 15000) : 0;
      const res = await fetch(url(this.feed), { credentials: "same-origin", cache: "no-store", signal: ctl ? ctl.signal : undefined });
      clearTimeout(to);
      if (res.ok) {
        data = parseFeed(await res.json());
        ok = !!data;
      }
    } catch (e) {
      ok = false;
    }
    this.busy = false;
    if (!this.alive()) return;
    if (!ok) {
      this.status = "Could not refresh.";
      if (!this.m) this.say("The songs are not available just now.");
      this.refreshRead();
      this.arm();
      return;
    }
    this.status = "";
    this.fetchedAt = Date.now();
    this.take(data);
    this.arm();
  }

  say(text) {
    this.msg.textContent = text;
  }

  /** Next poll: only for a day that is still going. */
  arm() {
    clearTimeout(this.timer);
    if (this.m && this.m.nowSec === null) return; // a past day does not change
    this.timer = setTimeout(() => {
      if (!this.alive()) return;
      if (this.onScreen && !document.hidden) this.load();
      else this.arm();
    }, this.poll);
  }

  /** The picture or the tab has come back: refresh if it has been a while. */
  wake() {
    if (!this.alive()) return;
    if (this.m && this.m.nowSec === null && this.fetchedAt) {
      this.dirty.back = this.dirty.notes = true;
      this.begin();
      return;
    }
    if (Date.now() - this.fetchedAt >= this.poll && !document.hidden && this.onScreen) this.load();
    this.begin();
  }

  take(m) {
    const before = this.keys;
    const reading = this.cur >= 0 && this.m ? songKey(this.m, this.cur) : "";
    const keepSolo = this.solo >= 0 && this.solo < m.species.length && this.m && this.m.species[this.solo] && this.m.species[this.solo].name === m.species[this.solo].name ? this.solo : -1;
    this.m = m;
    this.keys = keySet(m);
    this.nowAt = performance.now();
    this.solo = keepSolo;
    this.cur = -1;
    this.card.hidden = true;
    this.time = timeOrder(m);
    this.paint = paintOrder(m);
    this.skip = new Uint8Array(m.n);
    this.fresh.clear();
    if (before && !reducedMotion()) {
      const born = arrivals(before, m);
      if (born.length && born.length <= 80) {
        const t0 = performance.now();
        for (const i of born) {
          this.fresh.set(i, t0);
          this.skip[i] = 1;
        }
      }
    }
    this.say("");
    this.syncKey();
    this.applySolo();
    this.fillLede();
    this.describe();
    this.showIdle();
    this.host.dataset.ssSun = m.sun ? "on" : "off";
    this.dirty.back = this.dirty.notes = true;
    this.relayout();
    // A reader who is looking at a song when the picture refreshes keeps it.
    if (reading) {
      for (let i = 0; i < m.n; i++) {
        if (this.active[i] && songKey(m, i) === reading) {
          this.select(i, this.hover);
          break;
        }
      }
    }
    this.begin();
  }

  fillLede() {
    const m = this.m;
    const day = m.nowSec === null ? "that day" : "today";
    let t = `Each glowing note is one song heard ${day}. Across is the time of day; up is how sure the station was.`;
    if (!m.sun) t += " No sun is drawn: this station does not know where it is.";
    this.lede.textContent = t;
  }

  describe() {
    const m = this.m;
    const label = `Songs heard ${m.nowSec === null ? "that day" : "today"}, by the time of day and by how sure the station was. ${summarize(m, { today: m.nowSec !== null })}`;
    this.canvas.setAttribute("aria-label", label);
  }

  // ── the key ──
  /** The key's counts follow the feed; its buttons are only rebuilt when the species themselves change. */
  syncKey() {
    const m = this.m;
    const sig = m.species.map((sp) => `${sp.slot}:${sp.name}`).join("\u0000");
    if (sig === this.keySig && this.chips.length === m.species.length) {
      m.species.forEach((sp, i) => {
        this.chips[i].lastChild.textContent = group(sp.n);
      });
      return;
    }
    this.keySig = sig;
    this.fillKey();
  }

  fillKey() {
    const m = this.m;
    this.key.textContent = "";
    this.chips = [];
    m.species.forEach((sp, i) => {
      const b = el("button", "ss-chip");
      b.type = "button";
      b.setAttribute("aria-pressed", "false");
      b.dataset.i = String(i);
      const sw = el("span", `ss-sw ${slotClass(sp.slot)}`);
      sw.setAttribute("aria-hidden", "true");
      b.append(sw, el("span", "ss-chip-name", sp.name), el("span", "ss-chip-n", group(sp.n)));
      b.title = sp.name;
      b.addEventListener("click", () => this.setSolo(this.solo === i ? -1 : i));
      this.key.appendChild(b);
      this.chips.push(b);
    });
    this.all = el("button", "ss-chip ss-all", "Show all");
    this.all.type = "button";
    this.all.hidden = true;
    this.all.addEventListener("click", () => {
      const was = this.solo;
      this.setSolo(-1);
      // The button has just gone; focus goes back to the species that was on its own.
      if (was >= 0 && this.chips[was]) this.chips[was].focus();
    });
    this.key.appendChild(this.all);
  }

  setSolo(i) {
    this.solo = i;
    this.applySolo();
    if (this.cur >= 0 && !this.active[this.cur]) this.select(-1);
    this.dirty.notes = true;
    this.schedule();
  }

  applySolo() {
    const m = this.m;
    this.active = new Uint8Array(m.n);
    for (let i = 0; i < m.n; i++) this.active[i] = this.solo < 0 || m.s[i] === this.solo ? 1 : 0;
    this.chips.forEach((b, i) => {
      b.setAttribute("aria-pressed", this.solo === i ? "true" : "false");
      b.classList.toggle("is-dim", this.solo >= 0 && this.solo !== i);
    });
    if (this.all) this.all.hidden = this.solo < 0;
  }

  // ── reading ──
  /** The live line: `text` (quiet for a hint, plain for a reading), then any note about the feed. Written only when it changes. */
  setRead(text, quiet) {
    const tail = this.status;
    const key = `${text}\u0000${tail}\u0000${quiet ? 1 : 0}`;
    if (key === this.readKey) return;
    this.readKey = key;
    this.read.textContent = "";
    if (text) this.read.appendChild(el("span", quiet ? "ss-hint" : "ss-sentence", text));
    if (tail) this.read.appendChild(el("span", "ss-hint", text ? ` ${tail}` : tail));
  }

  /** Say again what the live line is saying (a song being read, or the hint), with any note about the feed. */
  refreshRead() {
    if (this.m && this.cur >= 0) this.setRead(readingText(this.m, this.cur));
    else this.showIdle();
  }

  showIdle() {
    const m = this.m;
    if (!m) {
      this.sum.textContent = "";
      this.setRead("");
      return;
    }
    this.sum.textContent = summarize(m, { today: m.nowSec !== null });
    this.setRead(m.n ? (this.coarse() ? "Tap a note to read it." : "Point at a note, or use the arrow keys, to read it.") : "", true);
  }

  select(i, fromPointer) {
    this.hover = !!fromPointer;
    if (i === this.cur) {
      if (i >= 0) this.placeCard();
      return;
    }
    this.cur = i;
    const m = this.m;
    if (i < 0 || !m) {
      this.card.hidden = true;
      this.canvas.classList.remove("is-hit");
      this.showIdle();
    } else {
      const parts = cardParts(m, i);
      this.cardDot.className = `ss-card-dot ${slotClass(m.species[m.s[i]] ? m.species[m.s[i]].slot : 0)}`;
      this.cardName.textContent = parts.name;
      this.cardRest.textContent = ` · ${parts.time} · ${parts.sure}`;
      const h = safeHref(m.href[i]) ? url(m.href[i]) : "";
      if (h) this.card.setAttribute("href", h);
      else this.card.removeAttribute("href");
      this.card.hidden = false;
      this.setRead(readingText(m, i));
      this.placeCard();
    }
    this.schedule();
  }

  placeCard() {
    if (this.cur < 0 || !this.lay) return;
    const x = this.lay.x[this.cur];
    const y = this.lay.y[this.cur];
    const cw = this.card.offsetWidth;
    const ch = this.card.offsetHeight;
    const lift = this.r + (this.coarse() ? 30 : 14);
    let top = y - lift - ch;
    if (top < 4) top = y + this.r + (this.coarse() ? 26 : 14);
    const left = clamp(x - cw / 2, 4, Math.max(4, this.w - cw - 4));
    this.card.style.left = `${Math.round(left)}px`;
    this.card.style.top = `${Math.round(top)}px`;
  }

  coarse() {
    return !!(window.matchMedia && window.matchMedia("(pointer: coarse)").matches);
  }

  open(i) {
    const m = this.m;
    if (!m || i < 0) return;
    const h = m.href[i];
    if (!safeHref(h)) return;
    let to = url(h);
    if (typeof window.bnbSameOrigin === "function") {
      to = window.bnbSameOrigin(to);
      if (!to) return;
    }
    window.location.assign(to);
  }

  hitAt(e) {
    const r = this.canvas.getBoundingClientRect();
    const px = e.clientX - r.left;
    const py = e.clientY - r.top;
    const reach = e.pointerType === "mouse" ? 14 : 22;
    return nearestSong(this.lay.x, this.lay.y, this.paint, this.active, px, py, reach);
  }

  onPointer(e, down) {
    if (!this.m || !this.lay) return;
    const i = this.hitAt(e);
    if (e.pointerType === "mouse") {
      this.canvas.classList.toggle("is-hit", i >= 0);
      this.select(i, true);
    } else if (i >= 0) this.select(i, true);
    else if (down) this.select(-1, true);
  }

  onClick(e) {
    if (!this.m || !this.lay) return;
    const i = this.hitAt(e);
    if (i < 0) return;
    // A mouse has already read it by hovering; a finger reads on the first tap, opens on the second.
    if (e.pointerType === "mouse" || this.tapped === i) this.open(i);
  }

  onKey(e) {
    const m = this.m;
    if (!m || !m.n) return;
    let to = this.cur;
    if (e.key === "ArrowRight") to = stepTime(this.time, this.active, this.cur, 1);
    else if (e.key === "ArrowLeft") to = stepTime(this.time, this.active, this.cur, -1);
    else if (e.key === "ArrowDown") to = this.cur < 0 ? endSong(this.time, this.active, false) : stepSpecies(m, this.active, this.cur, 1);
    else if (e.key === "ArrowUp") to = this.cur < 0 ? endSong(this.time, this.active, true) : stepSpecies(m, this.active, this.cur, -1);
    else if (e.key === "Home") to = endSong(this.time, this.active, false);
    else if (e.key === "End") to = endSong(this.time, this.active, true);
    else if (e.key === "Escape") {
      this.select(-1);
      return;
    } else if (e.key === "Enter" || e.key === " ") {
      if (this.cur >= 0) {
        e.preventDefault();
        this.open(this.cur);
      }
      return;
    } else return;
    e.preventDefault();
    if (to >= 0) this.select(to, false);
  }

  // ── sizing and layout ──
  resize() {
    if (this.disposed || !this.alive()) return;
    const w = Math.floor(this.stage.clientWidth);
    if (w < 80) return;
    const narrow = w < 520;
    if (narrow !== this.narrow || this.host.dataset.ssSize === undefined) {
      this.narrow = narrow;
      this.host.dataset.ssSize = narrow ? "narrow" : "wide";
    }
    const h = Math.floor(this.stage.clientHeight);
    if (h < 120) return;
    this.dpr = Math.min(window.devicePixelRatio || 1, 2);
    this.w = w;
    this.h = h;
    this.dirty.back = this.dirty.notes = true;
    this.relayout();
    this.schedule();
  }

  relayout() {
    const m = this.m;
    if (!this.w || !this.h) return;
    this.g = geometry(this.w, this.h, this.narrow);
    if (!m) return;
    this.r = noteRadius(m.n, this.narrow);
    this.lay = layoutSongs(m.t, m.c, this.g, this.r, m.floor);
    this.sprites = null;
    if (this.cur >= 0) this.placeCard();
  }

  // ── drawing ──
  schedule() {
    if (this.raf || this.disposed) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      this.frame();
    });
  }

  /** Start whatever should run now that the picture is on screen: the entrance sweep. */
  begin() {
    if (!this.m || !this.onScreen || document.hidden) return;
    if (this.sweep === null && !reducedMotion() && this.m.n > 0) {
      this.sweep = { t0: 0 };
      this.dirty.back = true;
    } else if (this.sweep === null) this.sweep = false;
    this.schedule();
  }

  animating() {
    return (this.sweep && this.sweep.t0 !== undefined) || this.fresh.size > 0;
  }

  layer(name) {
    let l = this.layers[name];
    const W = Math.round(this.w * this.dpr);
    const H = Math.round(this.h * this.dpr);
    if (!l) {
      l = document.createElement("canvas");
      this.layers[name] = l;
    }
    if (l.width !== W || l.height !== H) {
      l.width = W;
      l.height = H;
      if (name === "back") this.dirty.back = true;
      else this.dirty.notes = true;
    }
    return l;
  }

  frame() {
    if (this.disposed || !this.w || !this.h || !this.g) return;
    if (!this.pal) {
      this.pal = palette(this.host);
      this.fonts = fonts(this.host);
    }
    const now = performance.now();
    // Cap the loop near 30 fps: a frame that comes too soon is put off, not drawn.
    if (this.animating() && now - this.lastFrame < 30 && this.lastFrame) {
      this.schedule();
      return;
    }
    this.lastFrame = now;
    const W = Math.round(this.w * this.dpr);
    const H = Math.round(this.h * this.dpr);
    if (this.canvas.width !== W || this.canvas.height !== H) {
      this.canvas.width = W;
      this.canvas.height = H;
    }
    const back = this.layer("back");
    const halo = this.layer("halo");
    const core = this.layer("core");
    if (this.dirty.back) {
      this.drawBack(back.getContext("2d"));
      this.dirty.back = false;
    }
    if (this.dirty.notes) {
      this.drawNotes(halo.getContext("2d"), core.getContext("2d"));
      this.dirty.notes = false;
    }
    const ctx = this.ctx;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, W, H);
    ctx.drawImage(back, 0, 0);
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    this.drawBeam(ctx);
    this.drawHair(ctx);

    // The entrance: the day's songs appear left to right, once, like a score being played.
    let clipX = this.w;
    if (this.sweep && this.sweep.t0 !== undefined) {
      if (!this.sweep.t0) this.sweep.t0 = now;
      const p = clamp((now - this.sweep.t0) / 1500, 0, 1);
      const end = this.m.nowSec !== null ? timeToX(this.nowSecond(), this.g.left, this.g.right) + 16 : this.g.right + 16;
      clipX = this.g.left - 12 + (end - this.g.left + 12) * easeOut(p);
      if (p >= 1) {
        this.sweep = false;
        clipX = this.w;
      }
    }
    const hold = this.sweep === null && !reducedMotion() && this.m && this.m.n > 0 && this.onScreen === false;
    if (!hold) {
      ctx.save();
      if (clipX < this.w) {
        ctx.beginPath();
        ctx.rect(0, 0, clipX, this.h);
        ctx.clip();
      }
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.globalCompositeOperation = this.pal.dark ? "lighter" : "source-over";
      ctx.drawImage(halo, 0, 0);
      ctx.globalCompositeOperation = "source-over";
      ctx.drawImage(core, 0, 0);
      ctx.restore();
      ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
      if (clipX < this.w) this.drawPlayhead(ctx, clipX);
    }
    this.drawFresh(ctx, now);
    this.drawFocus(ctx);
    if (this.animating() && this.onScreen && !document.hidden) this.schedule();
  }

  nowSecond() {
    const m = this.m;
    if (!m || m.nowSec === null) return 0;
    return clamp(m.nowSec + (performance.now() - this.nowAt) / 1000, 0, DAY - 1);
  }

  /** The sky, the sun's arc, the moon, the grid, the ground and the axes: everything but the notes. */
  drawBack(ctx) {
    const { w, h } = this;
    const g = this.g;
    const p = this.pal;
    const m = this.m;
    const f = this.fonts;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    // Sky: one wash left to right, laid out from the real sunrise and sunset.
    const sky = ctx.createLinearGradient(g.left, 0, g.right, 0);
    const stops = m ? skyStops(m.sun) : null;
    const colour = { night: p.night, dawn: p.dawn, day: p.day, dusk: p.dusk };
    if (stops) {
      // Between two stops the colour is mixed in OKLab (six steps), so a dawn does not go muddy
      // on its way from night to day the way a straight sRGB blend does.
      for (let k = 0; k < stops.length; k++) {
        const a = stops[k];
        sky.addColorStop(a.o, css(colour[a.k]));
        const b = stops[k + 1];
        if (!b || b.o - a.o < 0.004 || a.k === b.k) continue;
        for (let j = 1; j < 6; j++) sky.addColorStop(a.o + ((b.o - a.o) * j) / 6, css(mixOklab(colour[a.k], colour[b.k], j / 6)));
      }
    } else {
      sky.addColorStop(0, css(p.day));
      sky.addColorStop(1, css(p.day));
    }
    ctx.fillStyle = sky;
    ctx.fillRect(0, 0, w, g.bottom);
    // The zenith: the top of the sky is a little deeper than the horizon, which gives the wash height.
    const zenith = ctx.createLinearGradient(0, 0, 0, g.bottom * 0.7);
    zenith.addColorStop(0, css(p.night, p.dark ? 0.7 : 0.3));
    zenith.addColorStop(1, css(p.night, 0));
    ctx.fillStyle = zenith;
    ctx.fillRect(0, 0, w, g.bottom);
    // The haze: the sky clears toward the horizon, so the notes sing into light, not into a wall.
    const haze = ctx.createLinearGradient(0, g.band * 0.4, 0, g.bottom);
    if (p.dark) {
      haze.addColorStop(0, css(p.fg, 0));
      haze.addColorStop(1, css(p.fg, 0.07));
    } else {
      haze.addColorStop(0, css(p.gap, 0));
      haze.addColorStop(1, css(p.gap, 0.55));
    }
    ctx.fillStyle = haze;
    ctx.fillRect(0, 0, w, g.bottom);

    // Grid: the faintest rules, one per labelled hour and per labelled sureness.
    ctx.lineWidth = 1;
    ctx.strokeStyle = css(p.grid, p.dark ? 0.55 : 0.5);
    const ticks = hourTicks(g.narrow);
    ctx.beginPath();
    for (const hr of ticks) {
      const x = Math.round(timeToX(hr * 3600, g.left, g.right)) + 0.5;
      ctx.moveTo(x, g.top - 8);
      ctx.lineTo(x, g.bottom);
    }
    // Until the station has said what its cut-off is, the picture draws no "how sure" axis at all.
    const yt = m ? yTicks(m.floor) : [];
    for (let k = 1; k < yt.length; k++) {
      const y = Math.round(sureToY(yt[k], m.floor, g.top, g.bottom)) + 0.5;
      ctx.moveTo(g.left, y);
      ctx.lineTo(g.right, y);
    }
    ctx.stroke();

    this.drawSun(ctx);

    // Ground: below the cut-off nothing was accepted, so the picture stops being sky.
    ctx.fillStyle = css(p.sunken);
    ctx.fillRect(0, g.bottom, w, g.ground);
    const shade = ctx.createLinearGradient(0, g.bottom, 0, g.bottom + 14);
    shade.addColorStop(0, css(p.axis, 0.22));
    shade.addColorStop(1, css(p.axis, 0));
    ctx.fillStyle = shade;
    ctx.fillRect(0, g.bottom, w, 14);
    ctx.strokeStyle = css(p.axis);
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, Math.round(g.bottom) + 0.5);
    ctx.lineTo(w, Math.round(g.bottom) + 0.5);
    ctx.stroke();

    // Hour ticks and labels, on the ground.
    const small = g.narrow ? 11 : 10.5;
    ctx.font = `500 ${small}px ${f.mono}`;
    ctx.fillStyle = css(p.label);
    ctx.textAlign = "center";
    ctx.textBaseline = "alphabetic";
    const rowB = g.bottom + (g.narrow ? 38 : 38);
    ctx.strokeStyle = css(p.axis);
    ctx.beginPath();
    for (const hr of ticks) {
      const x = Math.round(timeToX(hr * 3600, g.left, g.right)) + 0.5;
      ctx.moveTo(x, g.bottom);
      ctx.lineTo(x, g.bottom + 5);
    }
    ctx.stroke();
    for (const hr of ticks) ctx.fillText(p2(hr), timeToX(hr * 3600, g.left, g.right), rowB);

    // The "how sure" axis, in the gutter.
    ctx.textAlign = "right";
    const floor = m ? m.floor : 0;
    for (const v of yt) {
      const y = sureToY(v, floor, g.top, g.bottom);
      // The cut-off's own label sits just above the horizon, not on it.
      ctx.fillText(`${pctText(v)} %`, g.left - 8, v === floor && yt.length > 1 ? y - 5 : y + small * 0.36);
    }

    // The cut-off, said in words on the ground.
    this.cutEnd = 0;
    if (m) {
      ctx.textAlign = "left";
      ctx.font = `italic 14px ${f.display}`;
      ctx.fillStyle = css(p.annot);
      const cut = g.narrow ? `cut-off ${pctText(floor)} %` : `station cut-off ${pctText(floor)} %`;
      ctx.fillText(cut, g.left, g.bottom + 17);
      this.cutEnd = g.left + ctx.measureText(cut).width;
    }
  }

  drawSun(ctx) {
    const m = this.m;
    const g = this.g;
    const p = this.pal;
    const f = this.fonts;
    if (!m || !m.sun) return;
    const { rise, set } = m.sun;
    const y0 = g.band - 4;
    const amp = g.band - 22;
    const xOf = (hr) => timeToX(hr * 3600, g.left, g.right);
    const yOf = (hr) => y0 - sunHeight(hr, m.sun) * amp;
    const nowH = m.nowSec !== null ? this.nowSecond() / 3600 : null;

    // The arc: the part the sun has already travelled solid, the rest dashed.
    const steps = 72;
    const arc = (a, b) => {
      ctx.beginPath();
      for (let k = 0; k <= steps; k++) {
        const hr = a + ((b - a) * k) / steps;
        const x = xOf(hr);
        const y = hr <= rise || hr >= set ? y0 : yOf(hr);
        if (k === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();
    };
    ctx.lineWidth = 1.5;
    ctx.lineCap = "round";
    const doneTo = nowH === null ? set : clamp(nowH, rise, set);
    ctx.setLineDash([]);
    ctx.strokeStyle = css(p.sun, p.dark ? 0.75 : 0.9);
    if (doneTo > rise) arc(rise, doneTo);
    if (doneTo < set) {
      ctx.setLineDash([1.5, 5]);
      ctx.strokeStyle = css(p.sun, p.dark ? 0.55 : 0.7);
      arc(doneTo, set);
      ctx.setLineDash([]);
    }

    // The sun: at the current time while it is up, at solar noon on a past day, nowhere at night.
    const sunH = nowH === null ? (rise + set) / 2 : nowH;
    if (sunH > rise && sunH < set) {
      const x = xOf(sunH);
      const y = yOf(sunH);
      const rr = g.narrow ? 8 : 8.5;
      const glow = ctx.createRadialGradient(x, y, rr * 0.6, x, y, rr * 3.4);
      glow.addColorStop(0, css(p.sun, p.dark ? 0.5 : 0.45));
      glow.addColorStop(1, css(p.sun, 0));
      ctx.fillStyle = glow;
      ctx.fillRect(x - rr * 3.4, y - rr * 3.4, rr * 6.8, rr * 6.8);
      ctx.fillStyle = css(p.sun);
      ctx.beginPath();
      ctx.arc(x, y, rr, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = css(p.sunCore, 0.75);
      ctx.beginPath();
      ctx.arc(x - rr * 0.18, y - rr * 0.18, rr * 0.5, 0, Math.PI * 2);
      ctx.fill();
    }

    // Sunrise and sunset, said once, at the foot of the arc.
    ctx.font = `500 10.5px ${f.mono}`;
    // On the sky these are in the stronger ink: the dawn and dusk washes are a touch too busy for
    // the muted one (4.3:1 in the dark theme, against the 4.5:1 that small text needs).
    ctx.fillStyle = css(p.annot);
    ctx.textBaseline = "alphabetic";
    const rl = g.narrow ? clockOfHour(rise) : `sunrise ${clockOfHour(rise)}`;
    const sl = g.narrow ? clockOfHour(set) : `sunset ${clockOfHour(set)}`;
    const rw = ctx.measureText(rl).width;
    const sw = ctx.measureText(sl).width;
    const rx = xOf(rise) - 8 - rw;
    const sx = xOf(set) + 8;
    const roomR = rx >= 4;
    const roomS = sx + sw <= this.w - 4;
    ctx.textAlign = "left";
    if (roomR) ctx.fillText(rl, rx, y0 + 3.5);
    if (roomS) ctx.fillText(sl, sx, y0 + 3.5);

    // The moon, on the longer stretch of night, where it does not sit on a label.
    const mh = moonHour(m.sun, g.narrow ? 2.5 : 2);
    if (mh !== null) {
      const x = xOf(mh);
      const y = g.band * 0.42;
      const rr = g.narrow ? 7 : 7.5;
      const clear = (a, b) => !(a - rr < b.r && a + rr > b.l);
      const boxes = [];
      if (roomR) boxes.push({ l: rx, r: rx + rw });
      if (roomS) boxes.push({ l: sx, r: sx + sw });
      if (boxes.every((b) => clear(x, b)) || y < y0 - 12) this.moon(ctx, x, y, rr);
    }
  }

  /** A crescent: two arcs, the lit limb on the left. */
  moon(ctx, x, y, r) {
    const p = this.pal;
    const dx = r * 0.62;
    const hh = Math.sqrt(r * r - (dx * dx) / 4);
    const a1 = Math.atan2(-hh, dx / 2);
    const a2 = Math.atan2(hh, dx / 2);
    const b1 = Math.atan2(-hh, -dx / 2);
    const b2 = Math.atan2(hh, -dx / 2);
    ctx.save();
    ctx.translate(x, y);
    ctx.beginPath();
    ctx.arc(0, 0, r, a1, a2, true);
    ctx.arc(dx, 0, r, b2, b1, false);
    ctx.closePath();
    const glow = ctx.createRadialGradient(0, 0, r * 0.5, 0, 0, r * 3);
    glow.addColorStop(0, css(p.annot, p.dark ? 0.2 : 0.1));
    glow.addColorStop(1, css(p.annot, 0));
    ctx.fillStyle = css(p.annot, p.dark ? 0.85 : 0.6);
    ctx.fill();
    ctx.fillStyle = glow;
    ctx.beginPath();
    ctx.arc(0, 0, r * 3, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  // ── the notes ──
  makeSprites() {
    const p = this.pal;
    const r = this.r;
    const dpr = this.dpr;
    const crowd = clamp(Math.sqrt(Math.max(0, (this.m ? this.m.n : 0) - 100) / 1900), 0, 1);
    const hr = r * (this.narrow ? 3.6 : 4.2);
    const ring = r >= 4 ? 1.5 : r >= 3 ? 1.2 : 1;
    // On a very busy day each halo is fainter, so the glow where it is crowded is the sum of many
    // and not one solid wash.
    const peak = (p.dark ? 0.62 : 0.62) * (1 - 0.45 * crowd);
    const lift = p.dark ? p.fg : p.surface;
    const mk = (size, draw) => {
      const c = document.createElement("canvas");
      c.width = c.height = Math.ceil(size * dpr);
      const x = c.getContext("2d");
      x.setTransform(dpr, 0, 0, dpr, 0, 0);
      draw(x, size / 2);
      return c;
    };
    const halo = [];
    const core = [];
    p.chart.forEach((col) => {
      halo.push(
        mk(hr * 2, (x, c) => {
          const g = x.createRadialGradient(c, c, 0, c, c, hr);
          g.addColorStop(0, css(col, peak));
          g.addColorStop(0.22, css(col, peak * 0.62));
          g.addColorStop(0.5, css(col, peak * 0.2));
          g.addColorStop(1, css(col, 0));
          x.fillStyle = g;
          x.fillRect(0, 0, hr * 2, hr * 2);
        }),
      );
      core.push(
        mk((r + ring + 1) * 2, (x, c) => {
          // A lit centre: the species colour, lifted a little toward the light at its heart.
          const g = x.createRadialGradient(c - r * 0.25, c - r * 0.3, 0, c, c, r);
          g.addColorStop(0, css(mixRgb(col, lift, p.dark ? 0.38 : 0.34)));
          g.addColorStop(0.7, css(col));
          g.addColorStop(1, css(col));
          x.fillStyle = g;
          x.beginPath();
          x.arc(c, c, r, 0, Math.PI * 2);
          x.fill();
          x.lineWidth = ring;
          x.strokeStyle = css(p.gap, 0.9);
          x.beginPath();
          x.arc(c, c, r + ring / 2 - 0.2, 0, Math.PI * 2);
          x.stroke();
        }),
      );
    });
    this.sprites = { halo, core, hr, cr: r + ring + 1 };
  }

  colourIndex(i) {
    const sp = this.m.species[this.m.s[i]];
    return sp && sp.slot >= 1 && sp.slot <= SLOTS ? sp.slot - 1 : SLOTS;
  }

  drawNotes(hctx, cctx) {
    const m = this.m;
    hctx.setTransform(1, 0, 0, 1, 0, 0);
    cctx.setTransform(1, 0, 0, 1, 0, 0);
    hctx.clearRect(0, 0, hctx.canvas.width, hctx.canvas.height);
    cctx.clearRect(0, 0, cctx.canvas.width, cctx.canvas.height);
    if (!m || !m.n || !this.lay) return;
    if (!this.sprites) this.makeSprites();
    const { halo, core, hr, cr } = this.sprites;
    const dpr = this.dpr;
    const dim = this.pal.dim;
    const { x, y } = this.lay;
    hctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    cctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const soloing = this.solo >= 0;
    for (let k = 0; k < this.paint.length; k++) {
      const i = this.paint[k];
      if (this.skip[i]) continue;
      if (soloing && !this.active[i]) continue; // the others keep no glow: they fall back
      hctx.drawImage(halo[this.colourIndex(i)], x[i] - hr, y[i] - hr, hr * 2, hr * 2);
    }
    for (let k = 0; k < this.paint.length; k++) {
      const i = this.paint[k];
      if (this.skip[i]) continue;
      const on = this.active[i];
      cctx.globalAlpha = on ? 1 : dim;
      cctx.drawImage(core[this.colourIndex(i)], x[i] - cr, y[i] - cr, cr * 2, cr * 2);
    }
    cctx.globalAlpha = 1;
  }

  /** The playhead of the entrance sweep: a soft edge where the day is being written. */
  drawPlayhead(ctx, x) {
    const g = this.g;
    const p = this.pal;
    // One gradient, drawn at the origin and moved: no allocation per frame.
    if (!this.headGrad) {
      this.headGrad = ctx.createLinearGradient(-46, 0, 0, 0);
      this.headGrad.addColorStop(0, css(p.annot, 0));
      this.headGrad.addColorStop(1, css(p.annot, p.dark ? 0.16 : 0.1));
    }
    ctx.save();
    ctx.translate(x, 0);
    ctx.fillStyle = this.headGrad;
    ctx.fillRect(-46, g.top - 8, 46, g.bottom - g.top + 8);
    ctx.restore();
  }

  /** Songs that arrived since the last fetch: a ring and a flare, once, then they are ordinary notes. */
  drawFresh(ctx, now) {
    if (!this.fresh.size || !this.sprites) return;
    const { halo, core, hr, cr } = this.sprites;
    const p = this.pal;
    const { x, y } = this.lay;
    let ended = false;
    this.fresh.forEach((t0, i) => {
      const k = (now - t0) / 1400;
      if (k >= 1) {
        this.fresh.delete(i);
        this.skip[i] = 0;
        ended = true;
        return;
      }
      if (!this.active[i]) return;
      const e = easeOut(k);
      const ci = this.colourIndex(i);
      const flare = 1 + (1 - e) * 1.1;
      ctx.save();
      ctx.globalCompositeOperation = p.dark ? "lighter" : "source-over";
      ctx.globalAlpha = Math.min(1, 0.4 + e);
      ctx.drawImage(halo[ci], x[i] - hr * flare, y[i] - hr * flare, hr * 2 * flare, hr * 2 * flare);
      ctx.restore();
      const s = 0.45 + 0.55 * easeOut(k * 2.2);
      ctx.drawImage(core[ci], x[i] - cr * s, y[i] - cr * s, cr * 2 * s, cr * 2 * s);
      ctx.strokeStyle = css(p.annot, (1 - e) * 0.7);
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(x[i], y[i], this.r + 3 + e * 14, 0, Math.PI * 2);
      ctx.stroke();
    });
    if (ended) this.dirty.notes = true;
  }

  /** The now beam: a thin vertical light from the sun (or the top of the sky) down to the horizon. */
  drawBeam(ctx) {
    const m = this.m;
    if (!m || m.nowSec === null) return;
    const g = this.g;
    const p = this.pal;
    const f = this.fonts;
    const t = this.nowSecond();
    const x = Math.round(timeToX(t, g.left, g.right)) + 0.5;
    const hr = t / 3600;
    const up = m.sun && sunHeight(hr, m.sun) > 0;
    // By day the beam hangs from the sun. By night it starts under the sky band, so it never runs
    // through the sunrise and sunset labels or the moon, which are all there is up there.
    const y0 = up ? g.band - 4 - sunHeight(hr, m.sun) * (g.band - 22) + 10 : m.sun ? g.band + 2 : 6;
    const col = up ? p.sun : p.annot;
    // The beam's two gradients only change when the beam moves (a new minute) or the sun does.
    const key = `${x}|${y0}|${g.bottom}|${up}|${p.dark}`;
    if (this.beamKey !== key) {
      this.beamKey = key;
      this.beamWide = ctx.createLinearGradient(x - 10, 0, x + 10, 0);
      this.beamWide.addColorStop(0, css(col, 0));
      this.beamWide.addColorStop(0.5, css(col, p.dark ? 0.16 : 0.12));
      this.beamWide.addColorStop(1, css(col, 0));
      this.beamLine = ctx.createLinearGradient(0, y0, 0, g.bottom);
      this.beamLine.addColorStop(0, css(col, 0.9));
      this.beamLine.addColorStop(1, css(col, 0.55));
    }
    ctx.fillStyle = this.beamWide;
    ctx.fillRect(x - 10, y0, 20, g.bottom - y0);
    ctx.strokeStyle = this.beamLine;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(x, y0);
    ctx.lineTo(x, g.bottom + 4);
    ctx.stroke();

    // The pill that names it, on the ground, clear of the cut-off's own caption.
    const text = `now ${clockOfSecond(t)}`;
    ctx.font = `500 10.5px ${f.mono}`;
    const tw = ctx.measureText(text).width;
    const pw = tw + 14;
    const ph = 16;
    let px = x - pw / 2;
    if (this.cutEnd && px < this.cutEnd + 10 && px + pw > g.left) px = this.cutEnd + 10;
    px = clamp(px, 2, this.w - pw - 2);
    const py = g.bottom + 6;
    ctx.fillStyle = css(p.annot);
    ctx.beginPath();
    if (ctx.roundRect) ctx.roundRect(px, py, pw, ph, ph / 2);
    else ctx.rect(px, py, pw, ph);
    ctx.fill();
    ctx.fillStyle = css(p.gap);
    ctx.textAlign = "center";
    ctx.textBaseline = "alphabetic";
    ctx.fillText(text, px + pw / 2, py + ph / 2 + 3.6);
  }

  /** From the song being read straight down to the ground: its time, said against the axis. */
  drawHair(ctx) {
    if (this.cur < 0 || !this.lay) return;
    const g = this.g;
    const p = this.pal;
    const x = Math.round(this.lay.x[this.cur]) + 0.5;
    const y = this.lay.y[this.cur];
    ctx.save();
    ctx.strokeStyle = css(p.annot, 0.6);
    ctx.lineWidth = 1;
    ctx.setLineDash([2, 3]);
    ctx.beginPath();
    ctx.moveTo(x, y + this.r + 4);
    ctx.lineTo(x, g.bottom + 6);
    ctx.stroke();
    ctx.restore();
  }

  drawFocus(ctx) {
    if (this.cur < 0 || !this.lay || !this.sprites) return;
    const p = this.pal;
    const x = this.lay.x[this.cur];
    const y = this.lay.y[this.cur];
    const ci = this.colourIndex(this.cur);
    ctx.drawImage(this.sprites.core[ci], x - this.sprites.cr, y - this.sprites.cr, this.sprites.cr * 2, this.sprites.cr * 2);
    ctx.lineWidth = 4;
    ctx.strokeStyle = css(p.gap);
    ctx.beginPath();
    ctx.arc(x, y, this.r + 4.5, 0, Math.PI * 2);
    ctx.stroke();
    ctx.lineWidth = 1.75;
    ctx.strokeStyle = css(p.annot);
    ctx.stroke();
  }
}

// ── boot ────────────────────────────────────────────────────────────────────

/** Start every host that has not been started: safe to call again after a swap. */
export function boot(root) {
  if (typeof document === "undefined") return;
  const scope = root && root.querySelectorAll ? root : document;
  const hosts = [];
  if (scope.matches && scope.matches("[data-ss]")) hosts.push(scope);
  scope.querySelectorAll("[data-ss]").forEach((h) => hosts.push(h));
  for (const host of hosts) {
    if (host.getAttribute("data-ss-ready")) continue;
    new SkySong(host);
  }
}

if (typeof document !== "undefined") {
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", () => boot());
  else boot();
  document.addEventListener("htmx:afterSwap", (e) => boot(e.target && e.target.nodeType === 1 ? e.target : document));
}
