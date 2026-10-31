// node unit/seasons.test.mjs  (from tools/visual-qa/, or anywhere: it resolves the module by path)
// Plain `assert` of the pure functions exported by static/seasons.js. Each check asserts a property,
// not a shape, and the ones that discriminate have a counterpart that asserts the other side.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const M = await import(process.env.SSN_MODULE || path.join(here, "../../../crates/birdnet-web/static/seasons.js"));
const require = createRequire(import.meta.url);
require(path.join(here, "seasons.data.js")); // sets globalThis.SSN_CASES (no document under node)
const C = globalThis.SSN_CASES;

let n = 0;
const t = (name, fn) => {
  fn();
  n += 1;
  console.log(`ok  ${name}`);
};
const model = (key) => M.parseData(C[key]);

// ── the module is importable with no DOM ────────────────────────────────────
t("imports under node with no document", () => {
  assert.equal(typeof document, "undefined");
  assert.equal(typeof M.computeRanks, "function");
});

// ── words and numbers ───────────────────────────────────────────────────────
t("ordinal handles the teens and the 1/2/3 endings", () => {
  const got = [1, 2, 3, 4, 8, 11, 12, 13, 21, 22, 23, 101, 111].map(M.ordinal);
  assert.deepEqual(got, ["1st", "2nd", "3rd", "4th", "8th", "11th", "12th", "13th", "21st", "22nd", "23rd", "101st", "111th"]);
  assert.equal(M.ordinalWord(1), "first");
  assert.equal(M.ordinalWord(8), "eighth");
  assert.equal(M.ordinalWord(12), "12th");
});

t("formatCount prints the true number with separators; formatCompact is for the axis only", () => {
  assert.equal(M.formatCount(0), "0");
  assert.equal(M.formatCount(1234567), "1,234,567");
  assert.equal(M.formatCount(1612345), "1,612,345");
  assert.equal(M.formatCompact(500), "500");
  assert.equal(M.formatCompact(1500), "1,500");
  assert.equal(M.formatCompact(20000), "20k");
  assert.equal(M.formatCompact(1500000), "1.5M");
  assert.equal(M.formatCompact(2000000), "2M");
});

t("niceCeil rounds UP to a whole number the axis divides, never below the value", () => {
  for (const [v, want] of [[0, 1], [1, 1], [2, 2], [7, 8], [132, 150], [210, 250], [1812, 2000], [1612345, 2000000], [1834402, 2000000], [999999, 1000000]]) {
    assert.equal(M.niceCeil(v), want, `niceCeil(${v})`);
  }
  for (let v = 1; v < 5000; v += 7) {
    const c = M.niceCeil(v);
    assert.ok(c >= v && Number.isInteger(c), `${v} -> ${c}`);
    assert.ok(c < v * 1.6 + 1, `${v} -> ${c} is not a wasteful ceiling`); // never more than ~60% slack
  }
});

t("niceTicks: starts at zero, whole numbers, <= 5 intervals, ends on the ceiling", () => {
  for (const top of [1, 2, 3, 4, 5, 6, 8, 10, 15, 20, 25, 30, 40, 50, 60, 80, 100, 150, 250, 400, 2000, 2000000]) {
    const ticks = M.niceTicks(top);
    assert.equal(ticks[0], 0);
    assert.equal(ticks[ticks.length - 1], top);
    assert.ok(ticks.length - 1 <= 5 && ticks.length >= 2, `${top}: ${ticks}`);
    assert.ok(ticks.every(Number.isInteger));
    const step = ticks[1] - ticks[0];
    assert.ok(ticks.every((v, i) => v === i * step), "evenly spaced");
  }
});

t("truncateName and shortName never exceed the limit and never lose the kind of bird in a two-word name", () => {
  const forty = "Pseudo-Long-Hyphenated-Name-Warbler-0123";
  assert.equal(forty.length, 40);
  for (const max of [6, 11, 16, 22]) {
    assert.ok(M.truncateName(forty, max).length <= max);
    assert.ok(M.shortName(forty, max).length <= max);
    assert.ok(M.shortName("Northern Rough-winged Swallow Subspecies", max).length <= max);
  }
  assert.equal(M.shortName("Northern Cardinal", 11), "N. Cardinal");
  assert.equal(M.shortName("Northern Cardinal", 22), "Northern Cardinal");
  assert.equal(M.shortName("White-throated Sparrow", 11), "W. Sparrow");
  assert.ok(M.truncateName("Blue Jay Extra", 9).endsWith("…"));
  assert.equal(M.truncateName("Blue Jay", 8), "Blue Jay");
});

t("joinNames", () => {
  assert.equal(M.joinNames([]), "");
  assert.equal(M.joinNames(["A"]), "A");
  assert.equal(M.joinNames(["A", "B"]), "A and B");
  assert.equal(M.joinNames(["A", "B", "C"]), "A, B and C");
});

// ── parsing the server's JSON ───────────────────────────────────────────────
t("parseData: a real year, null for months that have not happened, a zero stays a zero", () => {
  const m = model("year");
  assert.equal(m.last, 9);
  assert.equal(m.thisMonth, 9);
  assert.equal(m.species.length, 8);
  const junco = m.species.find((s) => s.name === "Dark-eyed Junco");
  assert.equal(junco.counts[4], 0, "heard none is a real zero");
  assert.equal(junco.counts[10], null, "November has not happened");
  assert.equal(junco.counts[11], null);
  assert.equal(junco.total, 108 + 118 + 96 + 22 + 31);
});

t("parseData: a complete year has last 11 and thisMonth null", () => {
  const m = model("two");
  assert.equal(m.thisMonth, null);
  assert.equal(m.last, 11);
  assert.ok(m.species.every((s) => s.counts.every((c) => c !== null)));
});

t("parseData never throws and rejects what it cannot draw", () => {
  for (const bad of [null, undefined, "", "{", "[]", 5, {}, { species: [] }, { species: "x" }, { species: [{}] }, { species: [{ name: "A" }] }]) {
    assert.equal(M.parseData(bad), null, JSON.stringify(bad));
  }
  const m = M.parseData({ thisMonth: 99, species: [{ name: "  ", slot: 99, counts: [1, "2", -4, NaN, null, true, 7.6] }] });
  assert.equal(m.thisMonth, null, "an impossible month means the year is complete");
  assert.equal(m.species[0].name, "Unknown bird");
  assert.equal(m.species[0].slot, 1, "a slot outside 1..8 falls back to the position");
  assert.deepEqual(m.species[0].counts.slice(0, 7), [1, 2, 0, null, null, null, 8]);
  assert.equal(M.parseData(JSON.stringify(C.year)).species.length, 8, "a JSON string works");
});

t("parseData keeps at most 8 birds and forces months after thisMonth to null", () => {
  const many = { thisMonth: 2, species: Array.from({ length: 12 }, (_, i) => ({ name: `B${i}`, counts: new Array(12).fill(5) })) };
  const m = M.parseData(many);
  assert.equal(m.species.length, 8);
  assert.ok(m.species.every((s) => s.counts.slice(3).every((c) => c === null) && s.counts.slice(0, 3).every((c) => c === 5)));
});

// ── ranking: ties, zeros ────────────────────────────────────────────────────
t("computeRanks: rank 1 is the highest count; ranks are 1..k with no gaps among the birds heard", () => {
  const m = model("year");
  const r = M.computeRanks(m);
  for (let mo = 0; mo <= m.last; mo++) {
    const heard = m.species.map((s, i) => [s.counts[mo], r[i][mo]]).filter(([c]) => c > 0);
    const ranks = heard.map(([, k]) => k).sort((a, b) => a - b);
    assert.deepEqual(ranks, heard.map((_, i) => i + 1), `month ${mo}`);
    heard.sort((a, b) => a[1] - b[1]);
    for (let i = 1; i < heard.length; i++) assert.ok(heard[i - 1][0] >= heard[i][0], "counts fall as rank rises");
  }
  const jay = m.species.findIndex((s) => s.name === "Blue Jay");
  assert.equal(r[jay][9], 1, "Blue Jay leads October");
});

t("computeRanks: a zero is unranked (0), never last place", () => {
  const m = model("year");
  const r = M.computeRanks(m);
  const bunting = m.species.findIndex((s) => s.name === "Indigo Bunting");
  assert.deepEqual(r[bunting].slice(0, 3), [0, 0, 0]);
  assert.ok(r[bunting][4] > 0);
  assert.equal(r[bunting][8], 0);
  // counterpart: the birds that WERE heard in a month fill ranks 1..k, so a zero did not take rank k+1
  const heardJan = m.species.filter((s) => s.counts[0] > 0).length;
  const maxRankJan = Math.max(...r.map((row) => row[0]));
  assert.equal(maxRankJan, heardJan);
  assert.ok(maxRankJan < m.species.length, "someone was unranked in January");
});

t("computeRanks: months that have not happened are 0, not ranked", () => {
  const m = model("early");
  const r = M.computeRanks(m);
  assert.ok(r.every((row) => row.slice(3).every((k) => k === 0)));
  assert.ok(r.every((row) => row.slice(0, 3).some((k) => k > 0)));
});

t("computeRanks: ties break by the year's total, then by name, and do not depend on the listing order", () => {
  const m = model("ties");
  const r = M.computeRanks(m);
  const idx = (name) => m.species.findIndex((s) => s.name === name);
  // January: Blue Jay and Northern Cardinal both 50; Blue Jay's year total is higher
  assert.equal(r[idx("Blue Jay")][0], 1);
  assert.equal(r[idx("Northern Cardinal")][0], 2);
  // counterpart: with the same counts but the other bird ahead on the year total, the order flips
  const flipped = M.parseData({ thisMonth: 0, species: [
    { name: "Blue Jay", counts: [50] }, { name: "Northern Cardinal", counts: [50] } ] });
  flipped.species[1].total = 999; // the cardinal has the bigger year
  const rf = M.computeRanks(flipped);
  assert.equal(rf[1][0], 1);
  // same total, same count: by name
  const same = M.parseData({ thisMonth: 0, species: [{ name: "Zebra Finch", counts: [5] }, { name: "Acorn Woodpecker", counts: [5] }] });
  const rs = M.computeRanks(same);
  assert.equal(rs[1][0], 1);
  // listing order does not matter
  const rev = M.parseData({ ...C.ties, species: [...C.ties.species].reverse() });
  const rr = M.computeRanks(rev);
  for (const s of m.species) {
    const a = r[idx(s.name)];
    const b = rr[rev.species.findIndex((x) => x.name === s.name)];
    assert.deepEqual(a, b, s.name);
  }
});

t("computeRanks: a month nobody was heard has no ranks at all", () => {
  const m = model("ties");
  const r = M.computeRanks(m);
  assert.ok(r.every((row) => row[2] === 0));
  assert.equal(M.monthTotal(m, 2), 0);
});

// ── the leaderboard order ───────────────────────────────────────────────────
t("boardPositions: heard birds in rank order, then the unheard; always one row per bird", () => {
  const m = model("year");
  const r = M.computeRanks(m);
  for (let mo = 0; mo <= m.last; mo++) {
    const { pos, heard, order } = M.boardPositions(m, r, mo);
    assert.deepEqual([...pos].sort((a, b) => a - b), m.species.map((_, i) => i), "a permutation of 0..n-1");
    assert.equal(order.length, m.species.length);
    order.forEach((i, k) => assert.equal(pos[i], k));
    order.slice(0, heard).forEach((i, k) => assert.equal(r[i][mo], k + 1));
    order.slice(heard).forEach((i) => assert.equal(r[i][mo], 0));
  }
});

// ── the lead ────────────────────────────────────────────────────────────────
t("leadRuns: who held 1st and when (the sample year turns three times)", () => {
  const m = model("year");
  const runs = M.leadRuns(m, M.computeRanks(m));
  const names = runs.map((x) => [m.species[x.idx].name, x.from, x.to]);
  assert.deepEqual(names, [["Northern Cardinal", 0, 3], ["Indigo Bunting", 4, 5], ["Carolina Wren", 6, 6], ["Blue Jay", 7, 9]]);
  // the runs tile the months that happened, with no overlap
  let next = 0;
  for (const x of runs) {
    assert.equal(x.from, next);
    next = x.to + 1;
  }
  assert.equal(next, m.last + 1);
});

t("leadRuns: a month with nobody heard does not count as a change of lead", () => {
  const m = model("ties");
  const runs = M.leadRuns(m, M.computeRanks(m));
  assert.equal(runs.length, 1);
  assert.equal(m.species[runs[0].idx].name, "Blue Jay");
  assert.equal(runs[0].to, 5);
});

t("leaderOf reports every bird tied for first, in rank order", () => {
  const m = model("ties");
  const r = M.computeRanks(m);
  const jun = M.leaderOf(m, r, 5);
  assert.equal(jun.top, 22);
  assert.deepEqual(jun.idx.map((i) => m.species[i].name), ["Blue Jay", "Northern Cardinal"]);
  assert.deepEqual(M.leaderOf(m, r, 2), { top: 0, idx: [] });
  assert.equal(M.leaderOf(m, r, 4).idx.length, 1, "counterpart: no tie in May");
});

// ── sentences ───────────────────────────────────────────────────────────────
t("leaderSentence: the sentence the page prints", () => {
  const m = model("year");
  const r = M.computeRanks(m);
  assert.equal(M.leaderSentence(m, r, 9), "Blue Jay led October so far with 132 of the 463 heard.");
  assert.equal(M.leaderSentence(m, r, 3), "Northern Cardinal led April with 150 of the 508 heard.");
});

t("leaderSentence: ties, nobody, only one bird, a single bird chart", () => {
  const m = model("ties");
  const r = M.computeRanks(m);
  assert.equal(M.leaderSentence(m, r, 5), "Blue Jay and Northern Cardinal tied for the lead in June so far, with 22 each.");
  assert.equal(M.leaderSentence(m, r, 2), "None of these birds were heard in March.");
  const jan = model("jan");
  assert.match(M.leaderSentence(jan, M.computeRanks(jan), 0), /^Black-capped Chickadee led January so far with 64 of the 169 heard\.$/);
  const one = model("one");
  assert.equal(M.leaderSentence(one, M.computeRanks(one), 3), "Northern Cardinal was heard 60 times in April so far.");
  const lone = M.parseData({ thisMonth: null, species: [{ name: "A", counts: [3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] }, { name: "B", counts: new Array(12).fill(0) }] });
  assert.equal(M.leaderSentence(lone, M.computeRanks(lone), 0), "A was the only one of these birds heard in January: 3.");
});

t("yearSentence: once, twice, N times; and not 'every month' when a month had nobody", () => {
  const y = model("year");
  assert.equal(M.yearSentence(y, M.computeRanks(y)), "The lead changed 3 times so far.");
  const e = model("early");
  assert.equal(M.yearSentence(e, M.computeRanks(e)), "The lead changed once so far.");
  const l = model("long");
  assert.equal(M.yearSentence(l, M.computeRanks(l)), "The lead changed twice so far.");
  const h = model("huge");
  assert.equal(M.yearSentence(h, M.computeRanks(h)), "Northern Cardinal led every month so far.");
  const t = model("ties");
  assert.equal(M.yearSentence(t, M.computeRanks(t)), "Blue Jay led every month in which any of these birds was heard.");
  const two = model("two");
  assert.equal(M.yearSentence(two, M.computeRanks(two)), "The lead changed 10 times.");
});

t("monthReading / speciesReading: the live-region text", () => {
  const m = model("year");
  const r = M.computeRanks(m);
  const s = M.monthReading(m, r, 9);
  assert.ok(s.startsWith("October, so far: Blue Jay first with 132, Northern Cardinal second with 98, White-throated Sparrow third with 96,"), s);
  assert.ok(s.endsWith("Not heard: Indigo Bunting and Baltimore Oriole."), s);
  assert.equal(M.monthReading(m, r, 3).includes("so far"), false);
  const wren = m.species.findIndex((x) => x.name === "Carolina Wren");
  assert.equal(M.speciesReading(m, r, wren, 9), "Carolina Wren: fourth in October so far, heard 58 times. Best: first, in July. Heard in 10 of 10 months so far.");
  const bunting = m.species.findIndex((x) => x.name === "Indigo Bunting");
  assert.ok(M.speciesReading(m, r, bunting, 0).startsWith("Indigo Bunting: not heard in January."));
  const ties = model("ties");
  assert.equal(M.monthReading(ties, M.computeRanks(ties), 2), "March: none of these birds were heard.");
});

// ── curves ──────────────────────────────────────────────────────────────────
t("bumpSegment stays between its two ranks and is monotone: it never overshoots a rank", () => {
  let seed = 3;
  const rnd = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296);
  for (let k = 0; k < 400; k++) {
    const x0 = rnd() * 500;
    const x1 = x0 + 5 + rnd() * 200;
    const y0 = rnd() * 400;
    const y1 = rnd() * 400;
    const pts = M.sampleSegment(M.bumpSegment(x0, y0, x1, y1), 40);
    const lo = Math.min(y0, y1) - 1e-9;
    const hi = Math.max(y0, y1) + 1e-9;
    for (let i = 0; i < pts.length; i++) {
      assert.ok(pts[i].y >= lo && pts[i].y <= hi, "inside the rank range");
      assert.ok(pts[i].x >= x0 - 1e-9 && pts[i].x <= x1 + 1e-9, "inside the months");
      if (i) {
        assert.ok(pts[i].x >= pts[i - 1].x - 1e-9, "x never goes back");
        assert.ok((pts[i].y - pts[i - 1].y) * (y1 - y0) >= -1e-9, "y moves only towards the next rank");
      }
    }
    assert.ok(Math.abs(pts[0].y - y0) < 1e-9 && Math.abs(pts[40].y - y1) < 1e-9, "it meets both dots");
  }
  // counterpart: a flat step is flat (a plateau stays a plateau)
  assert.ok(M.sampleSegment(M.bumpSegment(0, 50, 100, 50), 10).every((p) => Math.abs(p.y - 50) < 1e-9));
  // counterpart: a plain Catmull-Rom style cubic WOULD overshoot, so the property is not vacuous
  const cr = (p0, p1, p2, p3, tt) => 0.5 * (2 * p1 + (p2 - p0) * tt + (2 * p0 - 5 * p1 + 4 * p2 - p3) * tt * tt + (-p0 + 3 * p1 - 3 * p2 + p3) * tt * tt * tt);
  let over = false;
  for (let i = 0; i <= 40; i++) {
    const y = cr(300, 100, 120, 120, i / 40); // ranks 300 -> 100 -> 120 -> 120: a Catmull-Rom curve dips below 100 before climbing
    if (y < 100 - 1e-6 || y > 120 + 1e-6) over = true;
  }
  assert.ok(over, "the property would catch an overshooting spline");
});

t("bumpPath: one move, then one cubic per step, through every point", () => {
  const d = M.bumpPath([{ x: 0, y: 10 }, { x: 40, y: 50 }, { x: 80, y: 10 }]);
  assert.ok(d.startsWith("M0 10C"));
  assert.equal((d.match(/C/g) || []).length, 2);
  assert.ok(d.endsWith("80 10"));
  assert.equal(M.bumpPath([]), "");
});

t("rankRuns: a zero month ends a run, so a bird that went quiet is two lines, not one", () => {
  assert.deepEqual(M.rankRuns([1, 2, 0, 0, 3, 3, 0], 6), [[0, 1], [4, 5]]);
  assert.deepEqual(M.rankRuns([0, 0, 0], 2), []);
  assert.deepEqual(M.rankRuns([1, 1, 1, 1], 1), [[0, 1]], "months after `last` are not looked at");
  const m = model("year");
  const r = M.computeRanks(m);
  const junco = m.species.findIndex((s) => s.name === "Dark-eyed Junco");
  assert.deepEqual(M.rankRuns(r[junco], m.last).map((run) => [run[0], run[run.length - 1]]), [[0, 3], [9, 9]]);
});

// ── layout, the scrubber, hit-testing ───────────────────────────────────────
t("layout: exactly the container wide, rows evenly spaced, columns evenly spaced, in every mode and width", () => {
  for (const [mode, widths] of [["wide", [560, 640, 900]], ["narrow", [300, 350, 420, 600]]]) {
    for (const width of widths) {
      for (const n of [1, 2, 5, 8]) {
        for (const last of [0, 2, 9, 11]) {
          for (const labelW of [0, 70, 150]) {
            const g = M.layout({ width, mode, n, labelW, last });
            assert.ok(Math.abs(g.W - width) <= 1, `${mode} ${width} n${n} last${last} lw${labelW}: W ${g.W}`);
            assert.ok(g.colW > 8, "columns are never squeezed to nothing");
            for (let m = 1; m < 12; m++) assert.ok(Math.abs(g.x(m) - g.x(m - 1) - g.colW) < 1e-9);
            for (let k = 2; k <= n; k++) assert.ok(Math.abs(g.y(k) - g.y(k - 1) - g.pitch) < 1e-9);
            assert.ok(g.y(1) > g.ribbonTop + g.ribbonH && g.y(n) < g.plotBottom, "rank rows are inside the plot");
            // the names fit between the newest dot and the right edge
            if (g.labelW > 0) assert.ok(g.x(last) + 10 + g.labelW <= g.W + 1, `names fit: ${mode} ${width} n${n} last${last} lw${labelW}`);
            assert.ok(g.plotRight <= g.W + 1);
          }
        }
      }
    }
  }
  // the phone really is a different layout: taller rows, a stronger month strip
  const w = M.layout({ width: 560, mode: "wide", n: 8, labelW: 100, last: 11 });
  const nrw = M.layout({ width: 350, mode: "narrow", n: 8, labelW: 70, last: 11 });
  assert.ok(nrw.pitch > w.pitch);
  assert.ok(nrw.handleH >= 32);
});

t("layout: names too wide for the phone are dropped (labelW 0), never allowed to push past the container", () => {
  const g = M.layout({ width: 260, mode: "narrow", n: 8, labelW: 200, last: 11 });
  assert.ok(g.W <= 261);
  assert.ok(g.labelW === 0 || g.labelW >= 40);
});

t("monthAt: the column under x, clamped to the months that happened (drag past either end stays in range)", () => {
  const g = M.layout({ width: 640, mode: "wide", n: 8, labelW: 100, last: 9 });
  for (let m = 0; m < 12; m++) {
    const want = Math.min(9, m);
    assert.equal(M.monthAt(g.x(m), g, 9), want, `middle of column ${m}`);
    assert.equal(M.monthAt(g.bandX(m) + 0.01, g, 9), want, "left edge");
    assert.equal(M.monthAt(g.bandX(m + 1) - 0.01, g, 9), want, "right edge");
  }
  assert.equal(M.monthAt(-500, g, 9), 0);
  assert.equal(M.monthAt(5000, g, 9), 9);
  assert.equal(M.monthAt(NaN, g, 9), 0);
  // counterpart: with the whole year to choose from, December is reachable
  assert.equal(M.monthAt(g.x(11), g, 11), 11);
});

t("nearestSpecies: the nearest line within the threshold, none beyond it; labels count", () => {
  const polys = [
    [[{ x: 0, y: 0 }, { x: 100, y: 0 }]],
    [[{ x: 0, y: 20 }, { x: 100, y: 20 }]],
    [[{ x: 50, y: 80 }]],
  ];
  assert.deepEqual(M.nearestSpecies(50, 4, polys, [], 9), { i: 0, d: 4 });
  assert.deepEqual(M.nearestSpecies(50, 15, polys, [], 9), { i: 1, d: 5 });
  assert.equal(M.nearestSpecies(50, 10, polys, [], 4), null, "between two lines, outside both thresholds");
  assert.equal(M.nearestSpecies(150, 0, polys, [], 9), null, "beyond the end of a line");
  assert.equal(M.nearestSpecies(53, 84, polys, [], 9).i, 2, "a lone dot is a target");
  assert.equal(M.nearestSpecies(200, 200, polys, [{ i: 1, x0: 190, x1: 260, y0: 192, y1: 208 }], 9).i, 1, "inside a name");
  assert.equal(M.nearestSpecies(300, 200, polys, [{ i: 1, x0: 190, x1: 260, y0: 192, y1: 208 }], 9), null);
  assert.deepEqual(M.nearestSpecies(0, 0, [], [], 9), null);
});

t("nearestSpecies on a real chart: touching a dot finds that bird", () => {
  const m = model("year");
  const r = M.computeRanks(m);
  const g = M.layout({ width: 560, mode: "wide", n: 8, labelW: 120, last: 9 });
  const polys = m.species.map((_, i) =>
    M.rankRuns(r[i], m.last).map((run) => {
      const pts = run.map((mo) => ({ x: g.x(mo), y: g.y(r[i][mo]) }));
      if (pts.length === 1) return pts;
      const out = [];
      for (let k = 1; k < pts.length; k++) out.push(...M.sampleSegment(M.bumpSegment(pts[k - 1].x, pts[k - 1].y, pts[k].x, pts[k].y), 10));
      return out;
    }),
  );
  m.species.forEach((_, i) => {
    for (let mo = 0; mo <= m.last; mo++) {
      if (!r[i][mo]) continue;
      const hit = M.nearestSpecies(g.x(mo), g.y(r[i][mo]), polys, [], 9);
      assert.ok(hit && hit.d < 0.01, `bird ${i} month ${mo}`);
    }
  });
});

// ── label placement ─────────────────────────────────────────────────────────
const overlap = (a, b) => a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1;

t("placeLabels: no two placed labels overlap, all stay in bounds, the first keeps its place", () => {
  const items = [];
  for (let k = 0; k < 14; k++) items.push({ id: k, x: 100 + (k % 3) * 8, y: 100 + (k % 4) * 6, w: 90, h: 14, anchor: k % 2 ? "start" : "end" });
  const bounds = { top: 0, bottom: 400, left: 0, right: 400 };
  const out = M.placeLabels(items, bounds);
  const placed = out.filter((o) => !o.hidden);
  assert.ok(placed.length >= 7);
  assert.equal(out[0].y, items[0].y, "the first label is never moved");
  for (let i = 0; i < placed.length; i++) {
    const r = placed[i].rect;
    assert.ok(r.y0 >= bounds.top && r.y1 <= bounds.bottom && r.x0 >= bounds.left && r.x1 <= bounds.right);
    for (let j = i + 1; j < placed.length; j++) assert.ok(!overlap(r, placed[j].rect), `${i} vs ${j}`);
  }
  // counterpart: labels that do not touch are all left exactly where they are
  const apart = M.placeLabels([0, 1, 2, 3].map((k) => ({ id: k, x: 10, y: 40 + k * 50, w: 80, h: 14, anchor: "start" })), bounds);
  assert.ok(apart.every((o, k) => !o.hidden && o.y === 40 + k * 50));
});

t("placeLabels: one that cannot be placed is hidden, not drawn on top; shifts [0] means exactly there or nowhere", () => {
  const bounds = { top: 0, bottom: 60, left: 0, right: 200 };
  const out = M.placeLabels(
    [
      { id: "a", x: 10, y: 20, w: 100, h: 14, anchor: "start" },
      { id: "b", x: 10, y: 20, w: 100, h: 14, anchor: "start", shifts: [0] },
      { id: "c", x: 10, y: 20, w: 100, h: 14, anchor: "start" },
    ],
    bounds,
  );
  assert.equal(out[0].hidden, false);
  assert.equal(out[1].hidden, true, "pinned to its anchor and the anchor is taken");
  assert.equal(out[2].hidden, false);
  assert.notEqual(out[2].y, 20, "the one that may move, moved");
  assert.ok(!overlap(out[0].rect, out[2].rect));
  // outside the bounds: hidden
  assert.equal(M.placeLabels([{ id: "x", x: 150, y: 20, w: 100, h: 14, anchor: "start" }], bounds)[0].hidden, true);
});

t("placeLabels: obstacles (the dots) are kept clear, and anchor 'middle' centres on x", () => {
  const bounds = { top: 0, bottom: 200, left: 0, right: 300 };
  const dot = { x0: 90, x1: 102, y0: 94, y1: 106 };
  const out = M.placeLabels([{ id: "n", x: 100, y: 100, w: 60, h: 14, anchor: "middle" }], bounds, [dot]);
  assert.equal(out[0].hidden, false);
  assert.ok(!overlap(out[0].rect, dot));
  const free = M.placeLabels([{ id: "n", x: 100, y: 100, w: 60, h: 14, anchor: "middle" }], bounds);
  assert.equal(free[0].rect.x0, 70);
  assert.equal(free[0].rect.x1, 130);
});

t("placeLabels does not change its input", () => {
  const items = [{ id: 1, x: 5, y: 20, w: 40, h: 14, anchor: "start" }, { id: 2, x: 5, y: 20, w: 40, h: 14, anchor: "start" }];
  const copy = JSON.stringify(items);
  M.placeLabels(items, { top: 0, bottom: 100, left: 0, right: 100 });
  assert.equal(JSON.stringify(items), copy);
});

// ── every sample, end to end through the pure functions ─────────────────────
t("every sample case parses, ranks and reads without throwing, in every month", () => {
  for (const key of Object.keys(C)) {
    const m = model(key);
    assert.ok(m, key);
    const r = M.computeRanks(m);
    assert.ok(M.describeChart(m, r).length > 20);
    for (let mo = 0; mo <= m.last; mo++) {
      assert.ok(M.leaderSentence(m, r, mo).length > 5, `${key} ${mo}`);
      assert.ok(M.monthReading(m, r, mo).length > 5);
      m.species.forEach((_, i) => assert.ok(M.speciesReading(m, r, i, mo).length > 5));
    }
    assert.ok(M.niceCeil(M.busiest(m)) >= M.busiest(m), key);
  }
});

t("scale: the 1.6-million case's axis is a whole-number ceiling and its ticks are readable", () => {
  const m = model("huge");
  const top = M.niceCeil(M.busiest(m));
  assert.equal(top, 2000000);
  assert.deepEqual(M.niceTicks(top).map(M.formatCompact), ["0", "500k", "1M", "1.5M", "2M"]);
});

console.log(`\n${n} checks passed`);
