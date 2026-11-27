// node terrain.test.mjs  -- the pure parts of static/terrain.js (no browser needed).
//
// UIR-19: the terrain's hour axis printed "12a", "6a", "12p", "6p", a vocabulary no other chart
// used and that the Display card's clock choice did not reach. Its ticks are now clock times in
// the reader's choice, through the same formatter as every marked time (display-format.js).
// Seen failing before `hourTick` existed: "hourTick is not a function".
import assert from "node:assert/strict";
const SRC = process.env.TERRAIN_JS || new URL("../../../crates/birdnet-web/static/terrain.js", import.meta.url).pathname;
const X = await import(SRC);

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

// The browser's formatter, as display-format.js exposes it (its 12-hour branch, reduced).
const display = (time) => ({
  effective: () => ({ time }),
  formatClock: (s, f) => {
    if (f !== "12h") return s;
    const h = Number(s.slice(0, 2));
    return `${h % 12 === 0 ? 12 : h % 12}:${s.slice(3, 5)} ${h < 12 ? "AM" : "PM"}`;
  },
});

t("a tick is a clock time, 24-hour by default", () => {
  assert.deepEqual([0, 6, 12, 18, 24].map((h) => X.hourTick(h)), ["00:00", "06:00", "12:00", "18:00", "00:00"]);
});
t("the reader's 12-hour choice reaches the ticks", () => {
  assert.deepEqual([0, 6, 12, 18].map((h) => X.hourTick(h, display("12h"))), ["12:00 AM", "6:00 AM", "12:00 PM", "6:00 PM"]);
});
t("and the 24-hour choice leaves them canonical", () => {
  assert.equal(X.hourTick(6, display("24h")), "06:00");
});
t("no tick keeps the old vocabulary", () => {
  for (const h of [0, 6, 12, 18, 24]) {
    for (const d of [undefined, display("12h"), display("24h")]) {
      assert.ok(!/^\d+[ap]$/.test(X.hourTick(h, d)), X.hourTick(h, d));
    }
  }
});

console.log(`${n} passed`);
