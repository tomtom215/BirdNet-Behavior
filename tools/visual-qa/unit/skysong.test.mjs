// node skysong.test.mjs — the pure parts of static/skysong.js, and a few lints over its source.
// Plain `assert`, no framework; exits non-zero on the first failure and says which.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import * as S from '../../../crates/birdnet-web/static/skysong.js';
import { CASES } from './skysong.sample.mjs';

let n = 0;
const t = (name, fn) => {
  try {
    fn();
    n += 1;
  } catch (e) {
    console.error(`FAIL  ${name}\n${e.stack}`);
    process.exit(1);
  }
};
const near = (a, b, eps = 1e-9, msg) => assert.ok(Math.abs(a - b) <= eps, msg || `${a} is not within ${eps} of ${b}`);

// ── time and sureness map exactly ──
t('timeToX: midnight is the left edge, 24:00 the right, noon the middle', () => {
  near(S.timeToX(0, 50, 1050), 50);
  near(S.timeToX(S.DAY, 50, 1050), 1050);
  near(S.timeToX(43200, 50, 1050), 550);
  near(S.timeToX(-5, 50, 1050), 50);
  near(S.timeToX(S.DAY + 99, 50, 1050), 1050);
});
t('xToTime inverts timeToX', () => {
  for (const sec of [0, 1, 59, 60, 3600, 29640, 43200, 86399]) near(S.xToTime(S.timeToX(sec, 52, 1100), 52, 1100), sec, 1e-6);
  assert.equal(S.xToTime(10, 52, 1100), 0);
  assert.equal(S.xToTime(5000, 52, 1100), S.DAY);
});
t('sureToY: the cut-off sits on the horizon, 100 % on the top, the middle in the middle', () => {
  near(S.sureToY(0.7, 0.7, 70, 330), 330);
  near(S.sureToY(1, 0.7, 70, 330), 70);
  near(S.sureToY(0.85, 0.7, 70, 330), 200);
  near(S.sureToY(0.5, 0.7, 70, 330), 330, 1e-9, 'below the cut-off is clamped to it, never off the picture');
  near(S.sureToY(1.2, 0.7, 70, 330), 70);
});
t('yToSure inverts sureToY', () => {
  for (const c of [0.7, 0.71, 0.8, 0.8734, 0.99, 1]) near(S.yToSure(S.sureToY(c, 0.7, 70, 330), 0.7, 70, 330), c, 1e-9);
});
t('more sure is always higher on the screen (smaller y)', () => {
  let prev = Infinity;
  for (let c = 0.7; c <= 1.0001; c += 0.01) {
    const y = S.sureToY(c, 0.7, 70, 330);
    assert.ok(y <= prev + 1e-9);
    prev = y;
  }
});
t('a floor of 1 does not divide by zero', () => {
  assert.ok(Number.isFinite(S.sureToY(1, 1, 70, 330)));
  assert.ok(Number.isFinite(S.sureShare(0.99, 0.99)));
});

// ── words for times and percentages ──
t('clockOfSecond: the minute a second falls in, never rounded up', () => {
  assert.equal(S.clockOfSecond(0), '00:00');
  assert.equal(S.clockOfSecond(59), '00:00');
  assert.equal(S.clockOfSecond(60), '00:01');
  assert.equal(S.clockOfSecond(29640), '08:14');
  assert.equal(S.clockOfSecond(86399), '23:59');
  assert.equal(S.clockOfSecond(90000), '23:59');
  assert.equal(S.clockOfSecond(NaN), '00:00');
});
t('clockOfHour: fractional hours to the nearest minute', () => {
  assert.equal(S.clockOfHour(6.8), '06:48');
  assert.equal(S.clockOfHour(16.6), '16:36');
  assert.equal(S.clockOfHour(0), '00:00');
  assert.equal(S.clockOfHour(23.999), '23:59');
  assert.equal(S.clockOfHour(24), '23:59');
});
t('surePct and pctText', () => {
  assert.equal(S.surePct(0.874), 87);
  assert.equal(S.surePct(0.875), 88);
  assert.equal(S.surePct(1), 100);
  assert.equal(S.surePct(1.4), 100);
  assert.equal(S.pctText(0.7), '70');
  assert.equal(S.pctText(0.725), '72.5');
  assert.equal(S.pctText(0.6999999999), '70');
});

// ── the axes ──
t('hourTicks: every 3 h wide, every 6 h on a phone, 00 to 24 both ways', () => {
  assert.deepEqual(S.hourTicks(false), [0, 3, 6, 9, 12, 15, 18, 21, 24]);
  assert.deepEqual(S.hourTicks(true), [0, 6, 12, 18, 24]);
});
t('yTicks: the cut-off, one round middle figure, 100 %', () => {
  assert.deepEqual(S.yTicks(0.7), [0.7, 0.85, 1]);
  assert.deepEqual(S.yTicks(0.5), [0.5, 0.75, 1]);
  assert.deepEqual(S.yTicks(0.8), [0.8, 0.9, 1]);
  assert.deepEqual(S.yTicks(0.95), [0.95, 1], 'too narrow a span for a middle figure');
  assert.deepEqual(S.yTicks(0), [0, 0.5, 1]);
  for (const f of [0, 0.1, 0.25, 0.33, 0.6, 0.7, 0.725, 0.9, 0.99]) {
    const k = S.yTicks(f);
    assert.equal(k[0], f);
    assert.equal(k[k.length - 1], 1);
    for (let i = 1; i < k.length; i++) assert.ok(k[i] > k[i - 1], `ascending for floor ${f}`);
  }
});

// ── the sky ──
const SUN = { rise: 6.8, set: 16.6, dawn: 6.2, dusk: 17.2 };
t('skyStops: no sun, no sky (null), never an invented one', () => {
  assert.equal(S.skyStops(null), null);
  assert.equal(S.skyStops(undefined), null);
  assert.equal(S.skyStops({ rise: null, set: null }), null);
  assert.equal(S.skyStops({ rise: 'x', set: 16 }), null);
  assert.equal(S.skyStops({ rise: 12, set: 12 }), null, 'a sun that never sets in the data is not drawn');
  assert.equal(S.skyStops({ rise: 18, set: 6 }), null);
});
t('skyStops: night, dawn at sunrise, day, dusk at sunset, night, in that order', () => {
  const st = S.skyStops(SUN);
  assert.deepEqual(st.map((s) => s.k), ['night', 'night', 'dawn', 'day', 'day', 'dusk', 'night', 'night']);
  near(st[0].o, 0);
  near(st[st.length - 1].o, 1);
  near(st[2].o, 6.8 / 24, 1e-9, 'the dawn colour is at sunrise');
  near(st[5].o, 16.6 / 24, 1e-9, 'the dusk colour is at sunset');
  assert.ok(st[1].o < 6.2 / 24, 'the night ends before first light');
  assert.ok(st[6].o > 17.2 / 24, 'and comes back after last light');
});
t('skyStops: offsets never go backwards, for any plausible day', () => {
  const suns = [
    SUN, { rise: 6.8, set: 16.6, dawn: null, dusk: null }, { rise: 8.9, set: 15.1, dawn: 8.1, dusk: 15.9 },
    { rise: 0.2, set: 23.9, dawn: null, dusk: null }, { rise: 3.9, set: 20.6, dawn: 2.5, dusk: 22.0 },
    { rise: 11.9, set: 12.2, dawn: null, dusk: null }, { rise: 5, set: 19, dawn: 7, dusk: 4 },
  ];
  for (const sun of suns) {
    const st = S.skyStops(sun);
    assert.ok(st, JSON.stringify(sun));
    for (let i = 1; i < st.length; i++) assert.ok(st[i].o >= st[i - 1].o, `${JSON.stringify(sun)} stop ${i}`);
    assert.ok(st[0].o >= 0 && st[st.length - 1].o <= 1);
  }
});
t('skyStops: civil dawn and dusk, when the station gives them, move the edges of the night', () => {
  const a = S.skyStops({ rise: 6.8, set: 16.6, dawn: 5.0, dusk: 18.5 });
  const b = S.skyStops({ rise: 6.8, set: 16.6, dawn: null, dusk: null });
  assert.ok(a[1].o < b[1].o, 'an earlier first light starts the wash earlier');
  assert.ok(a[6].o > b[6].o, 'a later last light ends it later');
});
t('sunHeight: a half sine between sunrise and sunset, zero outside', () => {
  near(S.sunHeight(6.8, SUN), 0);
  near(S.sunHeight(16.6, SUN), 0);
  near(S.sunHeight(11.7, SUN), 1);
  assert.equal(S.sunHeight(3, SUN), 0);
  assert.equal(S.sunHeight(20, SUN), 0);
  assert.equal(S.sunHeight(12, null), 0);
  assert.ok(S.sunHeight(9, SUN) > 0 && S.sunHeight(9, SUN) < 1);
});
t('moonHour: the middle of the longer night; none without a sun or without room', () => {
  assert.equal(S.moonHour(null, 2), null);
  const m = S.moonHour({ rise: 6.8, set: 16.6, dawn: 6.2, dusk: 17.2 }, 2);
  assert.ok(m > 17.2 && m < 24, 'the evening is the longer night here');
  const m2 = S.moonHour({ rise: 9, set: 17, dawn: 8.4, dusk: 17.6 }, 2);
  assert.ok(m2 > 0 && m2 < 8.4, 'the morning is the longer night here');
  assert.equal(S.moonHour({ rise: 3.9, set: 20.6, dawn: null, dusk: null }, 5), null, 'no stretch of night is long enough');
});
t('mixOklab: endpoints are the colours, the middle is between them in lightness', () => {
  const a = [0.1, 0.1, 0.3];
  const b = [0.9, 0.8, 0.5];
  const lum = (c) => 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
  S.mixOklab(a, b, 0).forEach((v, i) => near(v, a[i], 1e-6));
  S.mixOklab(a, b, 1).forEach((v, i) => near(v, b[i], 1e-6));
  const mid = S.mixOklab(a, b, 0.5);
  assert.ok(lum(mid) > lum(a) && lum(mid) < lum(b));
  mid.forEach((v) => assert.ok(v >= 0 && v <= 1));
});

// ── the feed ──
const feed = (over = {}) => ({
  date: '2026-10-31', nowSec: 50820, sun: SUN, floor: 0.7,
  species: [{ name: 'Blue Jay', slot: 1 }, { name: 'Northern Cardinal', slot: 2 }, { name: 'Other species', slot: 0 }],
  songs: [
    { t: 23456, c: 0.87, s: 0, href: '/detections/detail?date=2026-10-31&time=06:30:56&name=Blue%20Jay' },
    { t: 30000, c: 0.95, s: 1, href: '/x' },
  ],
  ...over,
});
t('parseFeed: a good feed', () => {
  const m = S.parseFeed(feed());
  assert.equal(m.n, 2);
  assert.equal(m.floor, 0.7);
  assert.equal(m.nowSec, 50820);
  assert.equal(m.species.length, 3);
  assert.equal(m.species[0].n, 1);
  assert.equal(m.species[2].n, 0);
  assert.equal(m.total, 2);
  assert.ok(m.t instanceof Float64Array);
});
t('parseFeed: not a feed, an error, no songs array', () => {
  assert.equal(S.parseFeed(null), null);
  assert.equal(S.parseFeed('x'), null);
  assert.equal(S.parseFeed({ error: 'nope', songs: [] }), null);
  assert.equal(S.parseFeed({ species: [] }), null);
});
t('parseFeed: bad songs are dropped, a bad species index falls to "Other species"', () => {
  const m = S.parseFeed(feed({ songs: [{ t: 'x', c: 0.9, s: 0 }, { t: 10, c: null, s: 0 }, null, { t: 10, c: 0.9, s: 99 }, { t: 5, c: 0.9, s: -1 }, { t: 1, c: 0.9, s: 1.5 }] }));
  assert.equal(m.n, 3);
  assert.deepEqual(Array.from(m.s), [2, 2, 2]);
  const noOther = S.parseFeed({ floor: 0.7, species: [{ name: 'A', slot: 1 }], songs: [{ t: 1, c: 0.8, s: 7 }] });
  assert.equal(noOther.species[noOther.species.length - 1].name, 'Other species');
  assert.equal(noOther.n, 1);
});
t('parseFeed: clamps times and sureness, floors, slots; keeps a null sun and a null nowSec', () => {
  const m = S.parseFeed(feed({ nowSec: null, sun: null, floor: 7, songs: [{ t: -5, c: 2, s: 0 }, { t: 999999, c: -1, s: 0 }] }));
  assert.equal(m.nowSec, null);
  assert.equal(m.sun, null);
  assert.equal(m.floor, 0.99);
  assert.deepEqual(Array.from(m.t), [0, S.DAY - 1]);
  assert.deepEqual(Array.from(m.c), [1, 0]);
  const s = S.parseFeed(feed({ species: [{ name: 'A', slot: 12 }, { name: 'B', slot: 'x' }], songs: [] }));
  assert.deepEqual(s.species.map((x) => x.slot), [0, 0]);
});
t('parseFeed: reads at most 4,000 songs; a species count the server supplies is kept when larger', () => {
  const songs = Array.from({ length: 5000 }, (_, i) => ({ t: i, c: 0.8, s: 0 }));
  const m = S.parseFeed(feed({ songs, species: [{ name: 'A', slot: 1, n: 1600000 }], total: 1600005 }));
  assert.equal(m.n, 4000);
  assert.equal(m.species[0].n, 1600000);
  assert.equal(m.total, 1600005);
  const small = S.parseFeed(feed({ species: [{ name: 'A', slot: 1, n: 1 }], songs: [{ t: 1, c: 0.8, s: 0 }, { t: 2, c: 0.8, s: 0 }] }));
  assert.equal(small.species[0].n, 2, 'a count below what is drawn is not believed');
});
t('arrivals: only songs not seen before', () => {
  const a = S.parseFeed(feed());
  const keys = S.keySet(a);
  assert.deepEqual(S.arrivals(keys, a), []);
  const b = S.parseFeed(feed({ songs: [...feed().songs, { t: 31000, c: 0.9, s: 0 }, { t: 23456, c: 0.87, s: 0 }] }));
  assert.deepEqual(S.arrivals(keys, b), [2], 'the repeat of a song already seen is not new');
  assert.deepEqual(S.arrivals(null, b), [], 'the first load has no arrivals');
});
t('timeOrder and paintOrder', () => {
  const m = S.parseFeed(feed({ songs: [{ t: 300, c: 0.9, s: 0 }, { t: 100, c: 0.8, s: 2 }, { t: 100, c: 0.7, s: 1 }, { t: 200, c: 0.95, s: 1 }] }));
  assert.deepEqual(Array.from(S.timeOrder(m)), [2, 1, 3, 0]);
  const paint = Array.from(S.paintOrder(m));
  assert.equal(paint[0], 1, '"Other species" is painted first (at the bottom)');
  assert.ok(paint.indexOf(0) < paint.indexOf(2), 'the most heard species is under the less heard');
  assert.ok(paint.indexOf(2) < paint.indexOf(3), 'within a species the less sure is under the surer');
});

// ── the sentence ──
t('summarize: the sentence the brief asks for, word for word', () => {
  const songs = [];
  for (let i = 0; i < 49; i++) songs.push({ t: 5 * 3600 + 12 * 60 + i * 1100, c: i < 24 ? 0.72 + (i % 10) * 0.012 : 0.87 + (i % 10) * 0.012, s: i % 9 });
  songs[48].t = 22 * 3600 + 40 * 60 + 12;
  songs[0].t = 5 * 3600 + 12 * 60 + 5;
  for (let i = 1; i < 48; i++) songs[i].t = 5 * 3600 + 20 * 60 + i * 100;
  const species = [1, 2, 3, 4, 5, 6, 7, 8].map((k) => ({ name: `Bird ${k}`, slot: k })).concat([{ name: 'Other species', slot: 0 }]);
  const m = S.parseFeed({ floor: 0.7, nowSec: null, sun: null, species, songs, speciesTotal: 9 });
  assert.equal(S.summarize(m), '49 songs from 9 species, the first at 05:12 and the last at 22:40; most were 85 % sure or better.');
});
t('summarize: nothing, one, a day still going, all in one minute, a busy day', () => {
  const none = S.parseFeed(feed({ songs: [] }));
  assert.equal(S.summarize(none, { today: true }), 'Nothing has been heard yet today.');
  assert.equal(S.summarize(none), 'Nothing was heard that day.');
  const one = S.parseFeed(feed({ songs: [{ t: 29640, c: 0.874, s: 0 }] }));
  assert.equal(S.summarize(one, { today: true }), 'One song so far, Blue Jay at 08:14, 87 % sure.');
  assert.equal(S.summarize(one), 'One song, Blue Jay at 08:14, 87 % sure.');
  const two = S.parseFeed(feed({ songs: [{ t: 29640, c: 0.9, s: 0 }, { t: 29650, c: 0.9, s: 1 }] }));
  assert.match(S.summarize(two), /^2 songs from 2 species, all at 08:14; most were 90 % sure or better\.$/);
  const today = S.parseFeed(feed());
  assert.match(S.summarize(today, { today: true }), /^2 songs from 2 species so far, the first at 06:30 and the most recent at 08:20; /);
  const big = S.parseFeed({ floor: 0.7, species: [{ name: 'A', slot: 1 }], songs: Array.from({ length: 1800 }, (_, i) => ({ t: i * 40, c: 0.8, s: 0 })), total: 1601800, speciesTotal: 27 });
  assert.match(S.summarize(big), /^1,601,800 songs from 27 species, the first at 00:00 and the last at 19:59; most were 80 % sure or better\. The picture shows 1,800 of them\.$/);
});
t('summarize: "most" is more than half, whatever the cut-off', () => {
  for (const n of [3, 4, 49, 50, 51, 100]) {
    const songs = Array.from({ length: n }, (_, i) => ({ t: i * 60, c: 0.7 + (0.3 * i) / n, s: 0 }));
    const m = S.parseFeed({ floor: 0.7, species: [{ name: 'A', slot: 1 }], songs });
    const say = S.summarize(m);
    const pct = Number((say.match(/most were (\d+) % sure/) || [])[1]);
    if (!pct) continue;
    const atLeast = Array.from(m.c).filter((c) => c * 100 >= pct - 1e-9).length;
    assert.ok(atLeast > n / 2, `${n}: ${say}`);
  }
});
t('summarize: songs sitting on the cut-off say so, and never claim a percentage below it', () => {
  const m = S.parseFeed({ floor: 0.7, species: [{ name: 'A', slot: 1 }], songs: [{ t: 1, c: 0.701, s: 0 }, { t: 2, c: 0.71, s: 0 }, { t: 3, c: 0.72, s: 0 }] });
  assert.match(S.summarize(m), /most were only just above the station's 70 % cut-off\.$/);
});
t('readingText and cardParts', () => {
  const m = S.parseFeed(feed());
  assert.equal(S.readingText(m, 0), 'Blue Jay at 06:30, 87 % sure.');
  assert.deepEqual(S.cardParts(m, 1), { name: 'Northern Cardinal', time: '08:20', sure: '95 % sure' });
});

// ── layout ──
const G = S.geometry(1100, 380, false);
t('geometry: a sane box at every size the page can give', () => {
  for (const [w, h] of [[1100, 380], [322, 460], [390, 460], [200, 300], [100, 130], [2000, 380]]) {
    const g = S.geometry(w, h, w < 520);
    assert.ok(g.right > g.left && g.bottom > g.top && g.top > g.band - 1, `${w}x${h}`);
    assert.ok(g.bottom + g.ground <= h + 1e-9 || h < 200, `${w}x${h}: the ground fits (or the stage is absurdly short)`);
  }
});
t('noteRadius: one size per day, smaller only as the day gets busier, never tiny', () => {
  let prev = Infinity;
  for (const n of [0, 1, 49, 100, 150, 400, 1000, 1800, 2000, 4000]) {
    const r = S.noteRadius(n, false);
    assert.ok(r <= prev + 1e-9);
    assert.ok(r >= 2.5, `${n}: ${r}`);
    prev = r;
  }
  assert.ok(S.noteRadius(49, true) > S.noteRadius(49, false), 'a phone gets bigger notes');
});
t('layoutSongs: y is exactly how sure; an isolated song is exactly at its time', () => {
  const T = [100, 20000, 40000, 80000];
  const C = [0.7, 0.8, 0.93, 1];
  const r = S.noteRadius(4, false);
  const { x, y } = S.layoutSongs(T, C, G, r, 0.7);
  for (let i = 0; i < 4; i++) {
    near(y[i], S.sureToY(C[i], 0.7, G.top, G.bottom), 1e-3);
    near(x[i], S.timeToX(T[i], G.left, G.right), 1e-3);
  }
});
t('layoutSongs: songs on the same spot are fanned out sideways, symmetrically, a fixed step apart', () => {
  const r = S.noteRadius(3, false);
  const T = [29000, 29010, 29020];
  const C = [0.9, 0.9, 0.9];
  const { x, y } = S.layoutSongs(T, C, G, r, 0.7);
  const x0 = S.timeToX(29010, G.left, G.right);
  const xs = Array.from(x).sort((a, b) => a - b);
  near(xs[1], S.timeToX(29010, G.left, G.right), 0.05, 'the middle one stays on its time (to the pixel the times differ by)');
  assert.ok(xs[1] - xs[0] >= S.FAN_R * r - 0.6 && xs[2] - xs[1] >= S.FAN_R * r - 0.6, 'a note apart');
  assert.ok(Math.abs(xs[0] + xs[2] - 2 * xs[1]) < 0.7, 'balanced about the middle');
  assert.equal(new Set(Array.from(y)).size, 1, 'sureness is never moved');
  void x0;
});
t('layoutSongs: nobody is moved more than MAX_SHIFT_R radii, however many pile up, and y never moves', () => {
  const r = S.noteRadius(50, false);
  const N = 40;
  const T = Array.from({ length: N }, (_, i) => 30000 + (i % 30));
  const C = Array.from({ length: N }, () => 0.85);
  const { x, y } = S.layoutSongs(T, C, G, r, 0.7);
  for (let i = 0; i < N; i++) {
    const home = S.timeToX(T[i], G.left, G.right);
    assert.ok(Math.abs(x[i] - home) <= S.MAX_SHIFT_R * r + 1e-3, `song ${i} moved ${x[i] - home}`);
    near(y[i], S.sureToY(0.85, 0.7, G.top, G.bottom), 1e-3);
  }
});
t('layoutSongs: songs in different minutes, or at clearly different heights, are not touched', () => {
  const r = S.noteRadius(3, false);
  const T = [30000, 30060, 30000];
  const C = [0.9, 0.9, 0.8];
  const { x } = S.layoutSongs(T, C, G, r, 0.7);
  for (let i = 0; i < 3; i++) near(x[i], S.timeToX(T[i], G.left, G.right), 1e-3);
});
t('layoutSongs: deterministic, and the same set of positions whatever order the server lists them in', () => {
  const T = [29000, 29010, 29020, 29030, 45000, 45020];
  const C = [0.91, 0.91, 0.91, 0.911, 0.7, 0.7];
  const r = S.noteRadius(6, false);
  const a = S.layoutSongs(T, C, G, r, 0.7);
  const b = S.layoutSongs(T, C, G, r, 0.7);
  assert.deepEqual(Array.from(a.x), Array.from(b.x));
  const perm = [5, 3, 1, 4, 0, 2];
  const c = S.layoutSongs(perm.map((i) => T[i]), perm.map((i) => C[i]), G, r, 0.7);
  const A = Array.from(a.x).map((v) => v.toFixed(2)).sort();
  const B = Array.from(c.x).map((v) => v.toFixed(2)).sort();
  assert.deepEqual(A, B);
});
t('layoutSongs on the stress day: 1,800 songs, every position finite and on the picture', () => {
  const d = S.parseFeed(CASES.day1800());
  const g = S.geometry(1100, 380, false);
  const { x, y } = S.layoutSongs(d.t, d.c, g, S.noteRadius(d.n, false), d.floor);
  for (let i = 0; i < d.n; i++) {
    assert.ok(Number.isFinite(x[i]) && Number.isFinite(y[i]));
    assert.ok(x[i] > g.left - 20 && x[i] < g.right + 20 && y[i] >= g.top - 1e-3 && y[i] <= g.bottom + 1e-3);
  }
});

// ── reading the picture ──
t('nearestSong: the nearest within the reach, none beyond it, the top one on a tie', () => {
  const xs = [100, 110, 300];
  const ys = [100, 100, 100];
  const order = Int32Array.from([0, 1, 2]);
  assert.equal(S.nearestSong(xs, ys, order, null, 103, 100, 14), 0);
  assert.equal(S.nearestSong(xs, ys, order, null, 108, 100, 14), 1);
  assert.equal(S.nearestSong(xs, ys, order, null, 200, 100, 14), -1);
  assert.equal(S.nearestSong(xs, ys, order, null, 105, 100, 14), 1, 'equally near: the one painted last, on top');
  assert.equal(S.nearestSong(xs, ys, Int32Array.from([1, 0, 2]), null, 105, 100, 14), 0);
  assert.equal(S.nearestSong(xs, ys, order, null, 114.9, 100, 14), 1);
  assert.equal(S.nearestSong(xs, ys, order, null, 300, 123, 22), -1, '23 px away is out of a finger\'s reach');
  assert.equal(S.nearestSong(xs, ys, order, null, 300, 121, 22), 2, '21 px away is within it');
  assert.equal(S.nearestSong(xs, ys, order, Uint8Array.from([0, 1, 1]), 103, 100, 14), 1, 'songs that have fallen back cannot be read');
  assert.equal(S.nearestSong(xs, ys, new Int32Array(0), null, 1, 1, 14), -1);
});
t('stepTime, endSong: time order, no wrap, skipping songs that have fallen back', () => {
  const time = Int32Array.from([2, 0, 3, 1]);
  const all = null;
  assert.equal(S.stepTime(time, all, -1, 1), 2);
  assert.equal(S.stepTime(time, all, -1, -1), 1);
  assert.equal(S.stepTime(time, all, 2, 1), 0);
  assert.equal(S.stepTime(time, all, 0, -1), 2);
  assert.equal(S.stepTime(time, all, 1, 1), 1, 'at the end it stays');
  assert.equal(S.stepTime(time, all, 2, -1), 2, 'at the start it stays');
  const act = Uint8Array.from([1, 1, 0, 1]);
  assert.equal(S.stepTime(time, act, -1, 1), 0, 'the first readable one');
  assert.equal(S.stepTime(time, act, 0, 1), 3);
  assert.equal(S.endSong(time, act, false), 0);
  assert.equal(S.endSong(time, act, true), 1);
  assert.equal(S.endSong(time, Uint8Array.from([0, 0, 0, 0]), true), -1);
});
t('stepSpecies: the next species\' song nearest in time; wraps; stays put with one species', () => {
  const m = S.parseFeed({ floor: 0.7, species: [{ name: 'A', slot: 1 }, { name: 'B', slot: 2 }, { name: 'C', slot: 3 }], songs: [
    { t: 100, c: 0.8, s: 0 }, { t: 90, c: 0.8, s: 1 }, { t: 400, c: 0.8, s: 1 }, { t: 5000, c: 0.8, s: 2 },
  ] });
  assert.equal(S.stepSpecies(m, null, 0, 1), 1, 'B at 90 is nearer to 100 than B at 400');
  assert.equal(S.stepSpecies(m, null, 1, 1), 3);
  assert.equal(S.stepSpecies(m, null, 3, 1), 0, 'wraps round');
  assert.equal(S.stepSpecies(m, null, 0, -1), 3);
  assert.equal(S.stepSpecies(m, Uint8Array.from([1, 0, 0, 1]), 0, 1), 3, 'skips a species that has fallen back');
  assert.equal(S.stepSpecies(m, Uint8Array.from([1, 0, 0, 0]), 0, 1), 0, 'nowhere to go');
  const one = S.parseFeed({ floor: 0.7, species: [{ name: 'A', slot: 1 }], songs: [{ t: 1, c: 0.8, s: 0 }] });
  assert.equal(S.stepSpecies(one, null, 0, 1), 0);
});
t('safeHref: only a path on this site', () => {
  assert.equal(S.safeHref('/detections/detail?date=2026-10-31'), true);
  for (const bad of ['', '/', '//evil.example/x', '/\\evil', 'https://evil.example', 'javascript:alert(1)', 'detections/x', null, undefined, 5]) assert.equal(S.safeHref(bad), false, String(bad));
});

// ── the sample cases are what the harness says they are ──
t('sample cases parse, and have the sizes the brief asks for', () => {
  const sizes = { day49: 49, day400: 400 };
  for (const [k, f] of Object.entries(CASES)) {
    const m = S.parseFeed(f());
    assert.ok(m, k);
    if (sizes[k]) assert.ok(Math.abs(m.n - sizes[k]) <= 25, `${k}: ${m.n}`);
  }
  assert.equal(S.parseFeed(CASES.day49()).n, 49);
  assert.ok(S.parseFeed(CASES.day1800()).n >= 1700);
  assert.equal(S.parseFeed(CASES.one()).n, 1);
  assert.equal(S.parseFeed(CASES.nosun()).sun, null);
  assert.equal(S.parseFeed(CASES.past()).nowSec, null);
  assert.equal(S.parseFeed(CASES.empty()).n, 0);
});

// ── lints over the module's source: the constraints the repo's gates enforce ──
const src = fs.readFileSync(new URL('../../../crates/birdnet-web/static/skysong.js', import.meta.url), 'utf8');
const code = src.split('\n').filter((l) => !/^\s*\/\//.test(l)).join('\n');
t('lint: no colour literals', () => {
  assert.doesNotMatch(code, /#[0-9a-fA-F]{3,8}\b/);
  assert.doesNotMatch(code, /\b(rgb|rgba|hsl|hsla|oklch|oklab|lab|lch|hwb)\s*\(/);
  assert.doesNotMatch(code, /["'`](white|black|red|green|blue|gray|grey|transparent)["'`]/);
});
t('lint: nothing the CSP refuses', () => {
  assert.doesNotMatch(code, /\beval\s*\(|new Function|innerHTML|outerHTML|insertAdjacentHTML|cssText|setAttribute\(\s*["']style|document\.write/);
});
t('lint: no imports, ES2020 only, no top-level DOM access', () => {
  assert.doesNotMatch(code, /^\s*import\s/m);
  assert.doesNotMatch(code, /\?\?=|\|\|=|&&=|\.at\(|replaceAll|structuredClone/);
  assert.match(src, /if \(typeof document !== "undefined"\)/);
});
t('lint: every token it reads exists in the stylesheet', () => {
  const css = fs.readFileSync(new URL('../../../crates/birdnet-web/static/css/app.css', import.meta.url), 'utf8');
  const wanted = new Set([...src.matchAll(/["'`](--[a-z0-9-]+)["'`]/g)].map((m) => m[1]));
  for (const m of src.matchAll(/`(--chart-)\$\{/g)) void m;
  wanted.add('--chart-1');
  wanted.add('--chart-8');
  for (const tok of wanted) assert.ok(new RegExp(`${tok}\\s*:`).test(css), `${tok} is not defined in app.css`);
});

console.log(`${n} tests passed`);
