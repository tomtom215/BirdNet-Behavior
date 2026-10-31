// node figure-export.test.mjs  -- the pure parts of static/figure-export.js (no browser needed).
import assert from "node:assert/strict";
const SRC = process.env.FIGURE_EXPORT_JS || new URL("../../../crates/birdnet-web/static/figure-export.js", import.meta.url).pathname;
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
const el = (text, attrs = {}, tag = "TD") => ({
  tagName: tag,
  textContent: text,
  getAttribute: (k) => (k in attrs ? attrs[k] : null),
});

t("a cell is what it shows with its spaces made one, or its own data-csv", () => {
  assert.equal(X.cell(el("  Eurasian \n  Blackbird ")), "Eurasian Blackbird");
  assert.equal(X.cell(el("12 min (five)", { "data-csv": "12" })), "12");
  assert.equal(X.cell(el("x", { "data-csv": "" })), "", "an empty data-csv is empty, not the text");
});

t("a field is quoted only when it must be, and a quote is doubled", () => {
  assert.equal(X.field("plain"), "plain");
  assert.equal(X.field("a,b"), '"a,b"');
  assert.equal(X.field('say "hi"'), '"say ""hi"""');
  assert.equal(X.field("two\nlines"), '"two\nlines"');
});

t("a cell a spreadsheet would run as a formula is written with an apostrophe", () => {
  assert.equal(X.field("=1+1"), "'=1+1");
  assert.equal(X.field("@SUM(A1)"), "'@SUM(A1)");
  assert.equal(X.field("+cmd"), "'+cmd");
  assert.equal(X.field("-cmd"), "'-cmd");
  assert.equal(X.field("\tx"), "'\tx");
  assert.equal(X.field("=HYPERLINK(\"http://x\",\"y\")"), '"\'=HYPERLINK(""http://x"",""y"")"', "neutralised, then quoted");
});

t("a number is data, however it starts", () => {
  for (const v of ["-3", "+3", "12%", "-0.5", "1,250", "3.14", "-12%"]) {
    assert.equal(X.field(v), v.includes(",") ? `"${v}"` : v, v);
  }
  // A dash that is not a number is not data.
  assert.equal(X.field("-"), "'-");
  assert.equal(X.field("—"), "—", "an em dash is not a formula starter");
});

t("the file starts with a byte order mark and ends every line, the last included, with CRLF", () => {
  const s = X.csv([["Within", "Median"], ["5 min", "3"]]);
  assert.equal(s.charCodeAt(0), 0xfeff);
  assert.equal(s.slice(1), "Within,Median\r\n5 min,3\r\n");
  assert.equal(X.csv([]).slice(1), "\r\n", "no rows is an empty line, not a crash");
});

t("a table's rows are its header row then its body, cells only", () => {
  const tr = (cells) => ({ children: cells });
  const table = {
    querySelectorAll: (sel) => {
      assert.equal(sel, "tr");
      return [
        tr([el("Within", {}, "TH"), el("Median", {}, "TH")]),
        tr([el("1 min"), el("1")]),
        tr([]),
        tr([el("2 min"), el("2")]),
      ];
    },
  };
  assert.deepEqual(X.rowsOf(table), [["Within", "Median"], ["1 min", "1"], ["2 min", "2"]]);
});

t("a file name is a safe stem and today's date", () => {
  const day = new Date(2026, 9, 31);
  assert.equal(X.fileName("Chorus assembling", day), "chorus-assembling-2026-10-31.csv");
  assert.equal(X.fileName("../../etc/passwd", day), "etc-passwd-2026-10-31.csv");
  assert.equal(X.fileName("", day), "table-2026-10-31.csv");
  assert.equal(X.fileName(undefined, day), "table-2026-10-31.csv");
});

console.log(`${process.exitCode ? "FAILED" : "all pass"}: ${n} tests passed`);
