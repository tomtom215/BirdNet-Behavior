// node discovery.test.mjs  -- the pure parts of static/discovery.js (no DOM needed).
import assert from "node:assert/strict";
const SRC = process.env.DSC_JS || new URL("../../../crates/birdnet-web/static/discovery.js", import.meta.url).pathname;
const D = await import(SRC);
import { CASES, withKeys } from "./discovery.data.mjs";

let n = 0;
const t = (name, fn) => {
  try {
    fn();
    n += 1;
  } catch (e) {
    console.error(`FAIL: ${name}\n`, e.message);
    process.exitCode = 1;
  }
};
const measure = (text, kind) => text.length * (kind === "note" ? 7 : 6.2);
const model = (name) => D.prepare(withKeys(CASES[name]()));

// ── dates ──
t("parseDay / isoOf round-trip and reject impossible dates", () => {
  assert.equal(D.isoOf(D.parseDay("2026-08-06")), "2026-08-06");
  assert.equal(D.parseDay("2026-02-30"), NaN);
  assert.equal(D.parseDay("nope"), NaN);
  assert.equal(D.parseDay(20260806), NaN);
  assert.equal(D.parseDay("2026-08-07") - D.parseDay("2026-08-06"), 1);
});

t("formatDay names the weekday", () => {
  assert.equal(D.formatDay(D.parseDay("2026-08-06")), "Thu 6 Aug 2026");
  assert.equal(D.formatDay(D.parseDay("2025-11-02")), "Sun 2 Nov 2025");
  assert.equal(D.formatShort(D.parseDay("2026-10-31")), "31 Oct");
});

t("agoPhrase: the phrase a reader would say", () => {
  const today = D.parseDay("2026-10-31");
  const ago = (iso) => D.agoPhrase(D.parseDay(iso), today);
  assert.equal(ago("2026-10-31"), "today");
  assert.equal(ago("2026-10-30"), "yesterday");
  assert.equal(ago("2026-10-26"), "5 days ago");
  assert.equal(ago("2026-10-17"), "2 weeks ago");
  assert.equal(ago("2026-10-10"), "3 weeks ago");
  assert.equal(ago("2026-08-06"), "12 weeks ago"); // the brief's own example
  assert.equal(ago("2026-05-17"), "5 months ago");
  assert.equal(ago("2025-11-02"), "11 months ago");
  assert.equal(ago("2025-10-31"), "1 year ago");
  assert.equal(ago("2025-07-31"), "1 year, 3 months ago");
  assert.equal(ago("2024-03-31"), "2 years, 7 months ago");
  assert.equal(D.agoPhrase(today + 5, today), "today"); // a clock a day ahead never says "-5 days"
});

// ── the model ──
t("prepare sorts by day then name, ranks from 1, drops bad rows, keeps the stored key", () => {
  const m = D.prepare({
    from: "2026-01-01",
    to: "2026-03-01",
    species: [
      { name: "Zed", first: "2026-02-01", count: 3 },
      { name: "alpha", first: "2026-02-01", count: 1, key: "Alpha (stored)", code: "alpx", img: "/x.png" },
      { name: "Mid", first: "2026-01-10", count: 0 },
      { name: "", first: "2026-01-10" },
      { name: "BadDate", first: "2026-13-40" },
      null,
    ],
  });
  assert.deepEqual(m.species.map((s) => s.name), ["Mid", "alpha", "Zed"]);
  assert.deepEqual(m.species.map((s) => s.rank), [1, 2, 3]);
  assert.equal(m.species[1].key, "Alpha (stored)");
  assert.equal(m.species[0].key, "Mid");
  assert.equal(m.species[1].code, "ALPX");
  assert.equal(m.total, 3);
  assert.equal(m.bunches.length, 2);
  assert.deepEqual([m.bunches[1].i0, m.bunches[1].i1, m.bunches[1].n], [1, 2, 2]);
});

t("prepare widens from/to to hold every species, and the axis is at least four weeks", () => {
  const m = D.prepare({ from: "2026-06-01", to: "2026-06-02", species: [{ name: "A", first: "2026-05-01", count: 1 }, { name: "B", first: "2026-07-01", count: 1 }] });
  assert.equal(D.isoOf(m.from), "2026-05-01");
  assert.equal(D.isoOf(m.to), "2026-07-01");
  const one = model("one");
  assert.ok(one.full.t1 - one.full.t0 >= D.MIN_SPAN);
  assert.equal(one.full.t1, one.to + 1);
});

t("prepare of nothing is empty, not a throw", () => {
  assert.equal(D.prepare(null).total, 0);
  assert.equal(D.prepare({ species: "x" }).total, 0);
});

t("the steps are the list so far: one rise per day, ending at the total", () => {
  const m = model("year");
  assert.equal(m.steps[0].n, 6); // six on day one
  assert.equal(m.steps[m.steps.length - 1].n, m.total);
  for (let i = 1; i < m.steps.length; i++) assert.ok(m.steps[i].n > m.steps[i - 1].n && m.steps[i].day > m.steps[i - 1].day);
  assert.equal(D.countAt(m, m.full.t0 - 5), 0);
  assert.equal(D.countAt(m, m.steps[0].day + 0.5), 6);
  assert.equal(D.countAt(m, m.steps[0].day + 0.49), 0); // not before the day it happened
  assert.equal(D.countAt(m, m.full.t1), m.total);
});

t("the notes name the newest and the first", () => {
  const m = model("year");
  const n = D.notesFor(m);
  assert.equal(n.newest, "newest: Painted Bunting, today");
  assert.equal(n.first, "the first day: 6 species");
  const two = D.notesFor(model("two"));
  assert.equal(two.first, "first: Northern Cardinal");
  assert.match(two.newest, /^newest: Painted Bunting, 4 weeks ago$/);
  assert.equal(D.notesFor(model("one")).first, "");
});

t("readingText is a whole sentence with the count and the rank", () => {
  const m = model("year");
  assert.equal(D.readingText(m, m.species[19]), "Painted Bunting: first heard Sat 31 Oct 2026 (today), heard just once so far. Number 20 of 20 on the list.");
  assert.match(D.readingText(m, m.species[0]), /heard 2,204 times since/);
  const lg = model("long");
  assert.match(D.readingText(lg, lg.species.find((x) => x.count === 1600000)), /heard 1,600,000 times since/);
});

t("initials: the code when there is room, else two letters, never empty", () => {
  assert.equal(D.initials({ name: "Painted Bunting", code: "PABU" }, true), "PABU");
  assert.equal(D.initials({ name: "Painted Bunting", code: "PABU" }, false), "PB");
  assert.equal(D.initials({ name: "Painted Bunting", code: "" }, true), "PB");
  assert.equal(D.initials({ name: "Kiwi", code: "" }, true).length, 2);
  assert.ok(D.initials({ name: "?", code: "" }, false).length >= 1);
});

// ── ranges, views ──
t("rangeOptions offers only ranges that differ from the whole", () => {
  const ids = (name) => D.rangeOptions(model(name)).map((o) => o.id);
  assert.deepEqual(ids("one"), ["all"]); // a day old
  assert.deepEqual(ids("two"), ["all"]); // 73 days
  assert.deepEqual(ids("year"), ["all", "90"]); // a year: "last year" is the whole, so not offered
  assert.deepEqual(ids("big80"), ["all", "year", "90"]);
  const m = model("big80");
  for (const o of D.rangeOptions(m).slice(1)) assert.ok(o.days <= (m.full.t1 - m.full.t0) * 0.7);
});

t("viewForRange ends today and never starts before the first day", () => {
  const m = model("big80");
  const o90 = D.rangeOptions(m).find((o) => o.id === "90");
  const v = D.viewForRange(m, o90);
  assert.equal(v.t1, m.full.t1);
  assert.equal(v.t1 - v.t0, 90);
  const huge = D.viewForRange(m, { id: "x", days: 99999 });
  assert.equal(huge.t0, m.full.t0);
  assert.equal(D.rangeOf(m, v), "90");
  assert.equal(D.rangeOf(m, { t0: v.t0 + 20, t1: v.t1 }), null);
});

t("clampView keeps a view inside the whole, at least minSpan wide", () => {
  const full = { t0: 100, t1: 500 };
  assert.deepEqual(D.clampView({ t0: 0, t1: 50 }, full, 14), { t0: 100, t1: 150 });
  assert.deepEqual(D.clampView({ t0: 480, t1: 560 }, full, 14), { t0: 420, t1: 500 });
  assert.deepEqual(D.clampView({ t0: 200, t1: 201 }, full, 14), { t0: 200, t1: 214 });
  assert.deepEqual(D.clampView({ t0: -999, t1: 9999 }, full, 14), full);
  assert.deepEqual(D.clampView({ t0: 100, t1: 110 }, { t0: 100, t1: 108 }, 14), { t0: 100, t1: 108 }); // a whole smaller than minSpan
});

t("zoomView keeps the anchor day where it was on screen", () => {
  const full = { t0: 0, t1: 1000 };
  const v = { t0: 200, t1: 600 };
  const anchor = 300;
  const f = (anchor - v.t0) / (v.t1 - v.t0);
  const z = D.zoomView(v, 0.5, anchor, full, 14);
  assert.ok(Math.abs((anchor - z.t0) / (z.t1 - z.t0) - f) < 1e-9);
  assert.equal(z.t1 - z.t0, 200);
  const out = D.zoomView(v, 100, anchor, full, 14);
  assert.deepEqual(out, full);
  const tiny = D.zoomView(v, 0.0001, anchor, full, 14);
  assert.equal(tiny.t1 - tiny.t0, 14);
});

t("panView keeps the span and stops at the ends", () => {
  const full = { t0: 0, t1: 1000 };
  assert.deepEqual(D.panView({ t0: 100, t1: 200 }, 50, full), { t0: 150, t1: 250 });
  assert.deepEqual(D.panView({ t0: 100, t1: 200 }, -500, full), { t0: 0, t1: 100 });
  assert.deepEqual(D.panView({ t0: 100, t1: 200 }, 5000, full), { t0: 900, t1: 1000 });
});

t("dayToX maps the middle of a day, and xToDay inverts it", () => {
  const view = { t0: 100, t1: 200 };
  assert.equal(D.dayToX(100, view, 1000), 5); // the middle of the first day of 100
  assert.equal(D.dayToX(199, view, 1000), 995);
  assert.equal(D.dayToX(150, view, 1000), D.dayToX(149, view, 1000) + 10);
  for (const x of [0, 137, 640, 1000]) assert.ok(Math.abs(D.dayToX(D.xToDay(x, view, 1000) - 0.5, view, 1000) - x) < 1e-9);
});

// ── the axis ──
t("axisTicks: first of the month, the year on a January, thinned as it narrows", () => {
  const y = D.axisTicks({ t0: D.parseDay("2025-11-02"), t1: D.parseDay("2026-11-01") }, 1000);
  assert.ok(y.every((k) => D.isoOf(k.day).endsWith("-01") || k.kind === "week"));
  assert.equal(y.filter((k) => k.kind === "month" && k.day >= D.parseDay("2025-11-02") && k.day <= D.parseDay("2026-11-01")).length, 12); // Dec .. Nov, every month at 1000 px
  assert.equal(y.filter((k) => k.kind === "month").length, 13); // and the 1st of Nov 2025, a day before the edge, so its label can start there
  const jan = y.find((k) => D.isoOf(k.day) === "2026-01-01");
  assert.equal(jan.year, "2026");
  assert.equal(y.filter((k) => k.year).length, 1);
  const dec = y.find((k) => D.isoOf(k.day) === "2025-12-01");
  assert.equal(dec.year, null);
  // Three years in 300 px: months cannot all be labelled, so they thin to every 12.
  const wide = D.axisTicks({ t0: D.parseDay("2023-11-01"), t1: D.parseDay("2026-11-01") }, 300);
  assert.ok(wide.length <= 4 && wide.every((k) => k.label === "Jan"));
  // Labels are never closer than ~56px.
  const ppd = 700 / (3 * 365);
  const xs = D.axisTicks({ t0: D.parseDay("2023-11-01"), t1: D.parseDay("2026-11-01") }, 700).map((k) => k.day * ppd);
  for (let i = 1; i < xs.length; i++) assert.ok(xs[i] - xs[i - 1] >= 56 - 1e-6);
});

t("axisTicks: weeks (Mondays) appear only when a day is wide enough", () => {
  const w = D.axisTicks({ t0: D.parseDay("2026-08-03"), t1: D.parseDay("2026-11-01") }, 1000);
  const weeks = w.filter((k) => k.kind === "week");
  assert.ok(weeks.length >= 10);
  for (const k of weeks) assert.equal(new Date(k.day * 86400000).getUTCDay(), 1);
  const none = D.axisTicks({ t0: D.parseDay("2025-11-02"), t1: D.parseDay("2026-11-01") }, 1000);
  assert.equal(none.filter((k) => k.kind === "week").length, 0);
});

t("leadYear: the year at the left edge unless a January is already there", () => {
  const v = { t0: D.parseDay("2025-11-02"), t1: D.parseDay("2026-11-01") };
  assert.equal(D.leadYear(v, 1000, D.axisTicks(v, 1000)), "2025");
  const v2 = { t0: D.parseDay("2025-12-28"), t1: D.parseDay("2026-06-01") };
  assert.equal(D.leadYear(v2, 1000, D.axisTicks(v2, 1000)), null); // 12 days to the January tick: it carries the year
});

t("countTicks: round numbers, none at 0, none crowding the total", () => {
  assert.deepEqual(D.countTicks(20), [5, 10, 15]);
  assert.deepEqual(D.countTicks(80), [20, 40, 60]);
  assert.deepEqual(D.countTicks(2000), [500, 1000, 1500]);
  assert.deepEqual(D.countTicks(1), []);
  for (const total of [2, 3, 7, 13, 99, 101, 640, 1999]) for (const v of D.countTicks(total)) assert.ok(v > 0 && v <= total * 0.88);
});

// ── trays and lanes ──
t("bunchShape: a day's species in a tidy tray, a chip past ten", () => {
  assert.deepEqual(D.bunchShape(1), { k: 1, cells: 1, cols: 1, rows: 1, shown: 1, chip: 0 });
  assert.deepEqual([D.bunchShape(2).cols, D.bunchShape(2).rows], [2, 1]);
  assert.deepEqual([D.bunchShape(4).cols, D.bunchShape(4).rows], [4, 1]);
  assert.deepEqual([D.bunchShape(6).cols, D.bunchShape(6).rows], [3, 2]);
  assert.deepEqual([D.bunchShape(8).cols, D.bunchShape(8).rows], [4, 2]);
  assert.deepEqual([D.bunchShape(10).cols, D.bunchShape(10).rows], [5, 2]);
  const big = D.bunchShape(40);
  assert.equal(big.cells, 10);
  assert.equal(big.shown, 9);
  assert.equal(big.chip, 31); // 9 shown + 31 behind the chip = 40
  assert.equal(big.shown + big.chip, 40);
  for (let k = 1; k <= 60; k++) {
    const s = D.bunchShape(k);
    assert.equal(s.shown + s.chip, k);
    assert.ok(s.cols * s.rows >= s.cells && s.rows * s.cols < s.cells + s.cols);
  }
});

t("packLanes: a crowd goes up, never sideways, and nothing overlaps", () => {
  // Six singles on one day at one x: six lanes, all at the same x.
  const six = D.packLanes(Array.from({ length: 6 }, () => ({ cx: 100, w: 40, rows: 1 })), { gap: 6 });
  assert.equal(six.lanes, 6);
  assert.deepEqual(six.placed.map((p) => p.lane), [0, 1, 2, 3, 4, 5]);
  assert.ok(six.placed.every((p) => p.left === 80 && p.right === 120));
  // Far apart: all in lane 0.
  const apart = D.packLanes([100, 200, 300].map((cx) => ({ cx, w: 40, rows: 1 })), { gap: 6 });
  assert.equal(apart.lanes, 1);
  // Just clear (gap 6): fits; one pixel short: goes up.
  assert.equal(D.packLanes([{ cx: 100, w: 40, rows: 1 }, { cx: 146, w: 40, rows: 1 }], { gap: 6 }).lanes, 1);
  assert.equal(D.packLanes([{ cx: 100, w: 40, rows: 1 }, { cx: 145, w: 40, rows: 1 }], { gap: 6 }).lanes, 2);
  // A two-row tray takes two lanes and lifts the next one above it.
  const tray = D.packLanes([{ cx: 100, w: 120, rows: 2 }, { cx: 110, w: 40, rows: 1 }], { gap: 6 });
  assert.deepEqual(tray.placed.map((p) => p.lane), [0, 2]);
  // Property: random data never overlaps within a lane, and x is the date unless clamped.
  let seed = 7;
  const rnd = () => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff;
  const items = [];
  let x = 40;
  for (let i = 0; i < 400; i++) {
    x += rnd() * 30;
    items.push({ cx: x, w: rnd() < 0.15 ? 90 + rnd() * 80 : 40, rows: rnd() < 0.15 ? 2 : 1 });
  }
  const r = D.packLanes(items, { gap: 6, minX: 4, maxX: x + 60 });
  const byLane = [];
  r.placed.forEach((p, i) => {
    for (let q = 0; q < items[i].rows; q++) (byLane[p.lane + q] = byLane[p.lane + q] || []).push(p);
    const centre = (p.left + p.right) / 2;
    assert.ok(Math.abs(centre - items[i].cx) < 1e-9 || p.left === 4 || p.right === x + 60, "an item moved sideways without being clamped");
  });
  for (const lane of byLane) {
    const row = lane.slice().sort((a, b) => a.left - b.left);
    for (let i = 1; i < row.length; i++) assert.ok(row[i].left >= row[i - 1].right + 6 - 1e-9, "overlap in a lane");
  }
});

t("packLanes keeps the two ends inside the picture", () => {
  const r = D.packLanes([{ cx: 10, w: 120, rows: 1 }, { cx: 990, w: 120, rows: 1 }], { gap: 6, minX: 4, maxX: 1000 });
  assert.equal(r.placed[0].left, 4);
  assert.equal(r.placed[1].right, 1000);
});

// ── the whole layout ──
const stageOf = (name, scaleIn, extra = {}) => {
  const m = model(name);
  const scale = scaleIn ?? 1050 / (m.full.t1 - m.full.t0); // what a 1100px stage gives: about 3px a day for a year
  const o = { width: 1100, scale, measure, notes: D.notesFor(m), ...extra };
  return { m, L: D.layoutStage(m, o) };
};

t("layout: every species has a mark, discs never overlap, all inside the world", () => {
  for (const name of ["year", "two", "one", "oneOld", "big80", "long", "month12", "bulk", "stress400"]) {
    const { m, L } = stageOf(name);
    for (let i = 0; i < m.total; i++) assert.ok(L.markOf[i] >= 0 && L.markOf[i] < L.marks.length, `${name}: species ${i} has no mark`);
    for (let a = 0; a < L.marks.length; a++) {
      const A = L.marks[a];
      assert.ok(A.x - A.r >= 0 && A.x + A.r <= L.worldW, `${name}: a mark outside the world`);
      assert.ok(A.y - A.r >= 0 && A.y + A.r <= L.curveTop, `${name}: a mark outside the lanes`);
    }
    // Overlap check on a sample window (all pairs for small cases, neighbours for big ones).
    const marks = L.marks.slice().sort((p, q) => p.x - q.x);
    for (let a = 0; a < marks.length; a++) {
      for (let b = a + 1; b < marks.length && marks[b].x - marks[a].x < 2 * L.r + 1; b++) {
        const d = Math.hypot(marks[a].x - marks[b].x, marks[a].y - marks[b].y);
        assert.ok(d >= marks[a].r + marks[b].r - 1e-6, `${name}: discs overlap (${d.toFixed(1)} < ${(marks[a].r + marks[b].r).toFixed(1)})`);
      }
    }
  }
});

t("layout: a portrait stands at its date (x is the day) unless a tray or an end moved it", () => {
  const { m, L } = stageOf("year");
  const wx = (day) => L.gutterL + (day + 0.5 - m.full.t0) * L.scale;
  const plotW = L.scale * (m.full.t1 - m.full.t0);
  for (const s of m.species) {
    const mk = L.marks[L.markOf[s.i]];
    const b = m.bunches.find((bb) => bb.day === s.day);
    if (b.n === 1) {
      assert.ok(Math.abs(mk.x - wx(s.day)) < 1e-6, `${s.name} is not on its day`);
      assert.ok(Math.abs(mk.x - (L.gutterL + D.dayToX(s.day, m.full, plotW))) < 1e-6, "layout and dayToX disagree");
    }
  }
  // Every stem stands on the day of its bunch and lands on the curve at that day's count.
  for (const st of L.stems) {
    const b = m.bunches[st.k];
    assert.ok(Math.abs(st.x - wx(b.day)) <= L.tray + L.r + 1, "a stem is not at its day");
    const expect = L.curveBase - (L.curveH * (b.i1 + 1)) / m.total;
    assert.ok(Math.abs(st.y1 - expect) < 1e-6, "a stem does not land on the curve");
    assert.ok(st.y0 <= st.y1, "a stem points the wrong way");
  }
});

t("layout: six on the first day are one tray of 3x2 under one stem", () => {
  const { m, L } = stageOf("year");
  assert.equal(m.bunches[0].n, 6);
  assert.equal(L.trays.length, 1);
  const first = L.marks.filter((k) => k.bunch === 0);
  assert.equal(first.length, 6);
  assert.equal(new Set(first.map((k) => Math.round(k.y))).size, 2);
  assert.equal(L.stems.filter((s) => s.k === 0).length, 1);
});

t("layout: forty on one day show nine and a +31 chip, and all forty are reachable", () => {
  const { m, L } = stageOf("bulk");
  const chip = L.marks.find((k) => k.idx < 0);
  assert.equal(chip.chip, 31);
  const b = m.bunches[0];
  const seen = new Set();
  for (let i = b.i0; i <= b.i1; i++) {
    const mk = L.marks[L.markOf[i]];
    assert.ok(mk, "no mark");
    seen.add(mk.idx < 0 ? "chip" : mk.idx);
  }
  assert.equal([...seen].filter((v) => v !== "chip").length, 9);
  assert.ok(seen.has("chip"));
});

t("layout: the lanes fit the budget by shrinking the discs, and the biggest that fits wins", () => {
  const few = stageOf("year").L;
  assert.equal(few.d, 40);
  const crowd = stageOf("big80").L;
  assert.ok(crowd.d < 40 && crowd.lanes * crowd.laneH <= 330, `${crowd.d}px, ${crowd.lanes} lanes`);
  // The next size up would not have fitted: asked for alone, it has to squeeze.
  const sizes = [40, 34, 28, 22, 16, 10];
  const up = sizes[sizes.indexOf(crowd.d) - 1];
  const bm = model("big80");
  const bigger = D.layoutStage(bm, { width: 1100, scale: 1050 / (bm.full.t1 - bm.full.t0), measure, notes: {}, tiers: [up] });
  assert.ok(bigger.squeezed, `${up}px should not fit big80 but did`);
  const stress = stageOf("stress2000").L;
  assert.ok(stress.lanes * stress.laneH <= 330 + 1e-6, "squeezed lanes still fit");
  assert.ok(stress.dots && stress.marks.length > 1900);
});

t("layout: zooming in separates what was crowded (fewer lanes at a larger scale)", () => {
  const a = stageOf("month12", 0.9).L;
  const b = stageOf("month12", 9).L; // ten times closer: the same month spread over ten times the room
  assert.ok(b.lanes < a.lanes, `${b.lanes} !< ${a.lanes}`);
});

t("layout: a name is printed only where it clears every disc, label and stem", () => {
  for (const name of ["year", "big80", "month12", "long"]) {
    const { m, L } = stageOf(name);
    const rects = L.labels.filter((l) => l.kind === "name").map((l) => {
      const w = measure(l.text, "label");
      const a = l.align === "left" ? l.x : l.x - w;
      return { a, b: a + w, y: l.y, text: l.text };
    });
    for (const r of rects) {
      for (const mk of L.marks) {
        const nearX = mk.x + mk.r > r.a - 1 && mk.x - mk.r < r.b + 1;
        const nearY = Math.abs(mk.y - r.y) < mk.r + 6;
        assert.ok(!(nearX && nearY), `${name}: "${r.text}" runs into a portrait`);
      }
      for (const st of L.stems) assert.ok(!(st.x > r.a - 2 && st.x < r.b + 2 && st.y0 < r.y && st.y1 > r.y), `${name}: a stem strikes "${r.text}"`);
      for (const o of rects) if (o !== r) assert.ok(r.b <= o.a || o.b <= r.a || Math.abs(o.y - r.y) > 12, `${name}: two names overlap`);
      assert.ok(r.a >= 0 && r.b <= L.worldW, `${name}: a name leaves the picture`);
    }
  }
});

t("layout: long names are cut with an ellipsis, never printed past 150px", () => {
  const { L } = stageOf("long");
  for (const l of L.labels.filter((x) => x.kind === "name")) assert.ok(measure(l.text, "label") <= 150 + 1, l.text);
});

t("layout: the notes sit in the band over the lanes, inside the picture, and the newest has its leader", () => {
  const { L } = stageOf("year");
  const notes = L.labels.filter((l) => l.kind !== "name");
  assert.deepEqual(notes.map((l) => l.kind).sort(), ["first", "newest"]);
  for (const n of notes) {
    assert.ok(n.y < L.laneZoneTop);
    const w = measure(n.text, "note");
    const a = n.align === "left" ? n.x : n.x - w;
    assert.ok(a >= 0 && a + w <= L.worldW);
  }
  assert.ok(L.leaders.some((l) => l.kind === "newest"));
});

t("layout: the picture is at least as tall as asked, and the curve zone has a zero baseline", () => {
  const { L } = stageOf("two");
  assert.ok(L.H >= 300);
  assert.ok(L.curveBase > L.curveTop && L.curveBase + L.axisH <= L.H + 1e-6);
});

// ── hit testing, the card ──
t("hitTest: the nearest edge wins, slop forgives, nothing far away hits", () => {
  const marks = [{ x: 100, y: 100, r: 20, idx: 0 }, { x: 130, y: 100, r: 20, idx: 1 }, { x: 400, y: 100, r: 20, idx: 2 }];
  assert.equal(D.hitTest(marks, 100, 100).idx, 0);
  assert.equal(D.hitTest(marks, 135, 100).idx, 1);
  assert.equal(D.hitTest(marks, 400, 124, 4).idx, 2); // 4 px outside the edge
  assert.equal(D.hitTest(marks, 400, 130, 4), null);
  assert.equal(D.hitTest(marks, 400, 130, 14).idx, 2); // a finger gets more
  assert.equal(D.hitTest([], 1, 1), null);
  // Every real mark is hit at its own centre.
  const { L } = stageOf("year");
  for (const mk of L.marks) assert.equal(D.hitTest(L.marks, mk.x, mk.y).idx, mk.idx);
});

t("placeCard: always inside the stage, never over the marker when it can avoid it", () => {
  const stage = { w: 1000, h: 400 };
  const card = { w: 312, h: 174 };
  let seed = 3;
  const rnd = () => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff;
  for (let i = 0; i < 2000; i++) {
    const a = { x: rnd() * stage.w, y: 40 + rnd() * 300, r: 24 };
    const p = D.placeCard(a, card, stage);
    assert.ok(p.x >= 6 - 1e-9 && p.x + card.w <= stage.w - 6 + 1e-9, `x ${p.x} side ${p.side}`);
    assert.ok(p.y >= 6 - 1e-9 && p.y + card.h <= stage.h - 6 + 1e-9, `y ${p.y} side ${p.side}`);
    if (p.side !== "over") {
      const overlap = p.x < a.x + a.r && p.x + card.w > a.x - a.r && p.y < a.y + a.r && p.y + card.h > a.y - a.r;
      assert.ok(!overlap || p.side === "right" || p.side === "left", `card covers its marker (${p.side})`);
    }
  }
  assert.equal(D.placeCard({ x: 500, y: 300, r: 20 }, card, stage).side, "above");
  assert.equal(D.placeCard({ x: 500, y: 60, r: 20 }, card, stage).side, "below");
});

// ── the phone's list and the keyboard ──
t("monthGroups: newest month first, newest bird first, soFar is the list at the end of the month", () => {
  const m = model("year");
  const g = D.monthGroups(m.species);
  assert.equal(g[0].key, "2026-10");
  assert.deepEqual(g[0].items.map((s) => s.name), ["Painted Bunting", "Cedar Waxwing", "Pileated Woodpecker", "Barred Owl"]);
  assert.equal(g[0].soFar, 20);
  assert.equal(g[g.length - 1].key, "2025-11");
  assert.equal(g[g.length - 1].soFar, 9);
  assert.equal(g.reduce((s, x) => s + x.items.length, 0), 20);
  assert.equal(D.monthLabel("2026-08"), "August 2026");
  const last90 = D.monthGroups(m.species, { t0: m.full.t1 - 90, t1: m.full.t1 });
  assert.deepEqual(last90.map((x) => x.key), ["2026-10"]);
});

t("nextIndex: Left/Right walk time, Up/Down do on the list, Home first, End newest", () => {
  const n = 20;
  assert.equal(D.nextIndex(5, "ArrowRight", n, "stage"), 6);
  assert.equal(D.nextIndex(5, "ArrowLeft", n, "stage"), 4);
  assert.equal(D.nextIndex(0, "ArrowLeft", n, "stage"), 0);
  assert.equal(D.nextIndex(n - 1, "ArrowRight", n, "stage"), n - 1);
  assert.equal(D.nextIndex(5, "Home", n, "stage"), 0);
  assert.equal(D.nextIndex(5, "End", n, "stage"), n - 1);
  assert.equal(D.nextIndex(5, "ArrowUp", n, "stage"), null); // not ours on the stage
  assert.equal(D.nextIndex(5, "ArrowUp", n, "list"), 6); // up is newer: the newest is at the top
  assert.equal(D.nextIndex(5, "ArrowDown", n, "list"), 4);
  assert.equal(D.nextIndex(null, "ArrowLeft", n, "stage"), n - 1); // no cursor yet: start at the newest
  assert.equal(D.nextIndex(5, "x", n, "list"), null);
  assert.equal(D.nextIndex(0, "End", 0, "list"), null);
});

// ── the file itself: what the CSP and the token rules forbid ──
import { readFileSync } from "node:fs";
t("the source writes no colour, no innerHTML, no eval, no inline style", () => {
  const src = readFileSync(SRC, "utf8");
  const code = src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
  assert.ok(!/#[0-9a-fA-F]{3,8}\b/.test(code), "a hex colour");
  assert.ok(!/\b(rgb|rgba|hsl|hsla|oklch|oklab|lch|lab)\(/.test(code), "a colour function");
  assert.ok(!/innerHTML|outerHTML|insertAdjacentHTML|document\.write/.test(code), "an HTML sink");
  assert.ok(!/\beval\(|new Function|setTimeout\(\s*["'`]/.test(code), "eval");
  assert.ok(!/setAttribute\(\s*["']style|cssText/.test(code), "an inline style");
  assert.ok(!/\bimport\s*\(|^import /m.test(code), "an import");
});

console.log(`${n} tests passed${process.exitCode ? " (some FAILED)" : ""}`);
