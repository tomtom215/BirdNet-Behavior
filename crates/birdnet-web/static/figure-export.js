// Save a figure's table as a CSV file.
//
//   <button type="button" data-csv-table="#as-table" data-csv-name="chorus-assembling" hidden>
//     Download the table as CSV
//   </button>
//
// The button is `hidden` in the markup and shown here, so a page with no script has no control that
// does nothing. A click reads the table it names (a CSS selector, looked up when it is clicked, so
// a table swapped in by htmx is the one that is read), builds the file in the browser and saves it;
// nothing is sent anywhere. The file name is `<name>-<date>.csv`.
//
// The file is what a spreadsheet will open without a surprise:
//   * RFC 4180: a cell with a comma, a quote or a line break is quoted, a quote is doubled, lines
//     end in CRLF.
//   * A UTF-8 byte order mark first, because without one Excel reads a species' accented name as
//     Latin-1.
//   * A cell that a spreadsheet would run as a formula (it starts with = + - @ or a tab or return)
//     is written with a leading apostrophe, unless it is plainly a number: "-3" and "12%" are data,
//     "=cmd|' /c calc'!A0" is not. The table is the station's own, but a species can be named by
//     anyone who can write to the station's settings, and a downloaded file leaves the page.
//
// An ES module so its pure parts (`cell`, `csv`, `rowsOf`, `fileName`) can be exported and tested
// under node; it wires itself only where there is a document.

/** One cell's text: its own `data-csv` if it has one, else what it shows, with runs of space made one. */
export function cell(el) {
  const own = el.getAttribute ? el.getAttribute("data-csv") : null;
  const text = own !== null && own !== undefined ? own : el.textContent || "";
  return text.replace(/\s+/g, " ").trim();
}

const NUMBER = /^[+-]?\d+([.,]\d+)*%?$/;
const RUNS_AS_FORMULA = /^[=+\-@\t\r]/;

/** The text as it is written into the file: neutralised if a spreadsheet would run it, then quoted if it must be. */
export function field(text) {
  let t = String(text);
  if (RUNS_AS_FORMULA.test(t) && !NUMBER.test(t)) t = `'${t}`;
  return /[",\r\n]/.test(t) ? `"${t.replace(/"/g, '""')}"` : t;
}

/** The whole file text for `rows` (arrays of cell text): CRLF lines, a byte order mark first. */
export function csv(rows) {
  return `\uFEFF${rows.map((r) => r.map(field).join(",")).join("\r\n")}\r\n`;
}

/** The rows of a table element: its header row first, then its body, as arrays of cell text. */
export function rowsOf(table) {
  const out = [];
  table.querySelectorAll("tr").forEach((tr) => {
    const cells = Array.from(tr.children).filter((c) => /^(TH|TD)$/i.test(c.tagName));
    if (cells.length) out.push(cells.map(cell));
  });
  return out;
}

/** `chorus-assembling-2026-10-31.csv`: a name that cannot climb out of the downloads folder. */
export function fileName(name, now = new Date()) {
  const stem = String(name || "table").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "table";
  const day = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
  return `${stem}-${day}.csv`;
}

function save(button) {
  const table = document.querySelector(button.getAttribute("data-csv-table") || "");
  if (!table) return;
  const rows = rowsOf(table);
  if (!rows.length) return;
  const blob = new Blob([csv(rows)], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = fileName(button.getAttribute("data-csv-name"));
  a.hidden = true;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function reveal() {
  document.querySelectorAll("button[data-csv-table][hidden]").forEach((b) => {
    b.hidden = false;
  });
}

if (typeof document !== "undefined") {
  document.addEventListener("click", (e) => {
    const b = e.target && e.target.closest ? e.target.closest("button[data-csv-table]") : null;
    if (b) save(b);
  });
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", reveal);
  else reveal();
  document.addEventListener("htmx:afterSwap", reveal);
}
