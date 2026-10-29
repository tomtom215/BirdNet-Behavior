// The soundscape terrain: the station's last 30, 90 or 365 days as a landscape.
//
// Hour of day runs one way, the day the other (the newest at the front), and the height and
// the colour of the ground at each hour of each day are how much was heard then. The card
// (templates/terrain.html) opens on a flat grid of the same numbers, drawn on a 2D canvas, which
// is also what a browser with no WebGL gets for good. The 3D view is the one thing here that
// needs a library, so three.js (static/vendor/three/, built by tools/vendor-three/) is imported
// the first time somebody asks for it and never before.
//
// An ES module so it can import() that library and so its pure parts can be exported and tested
// under node (tools/visual-qa/terrain.mjs): the mesh is a function of the counts and nothing else.
// It starts itself only where there is a document.
//
// What it promises, and what holds it to it:
//   * The surface reaches the height of every cell at that cell's middle and never goes above
//     it between: the surface between cell middles is a smoothstep, which cannot overshoot, so a
//     peak is never made up and never cut off (buildHeightField, tested).
//   * A day the station was not listening and an hour that has not happened yet are not drawn
//     as quiet: they are flat, in a colour of their own, and the reading says why.
//   * Height is the square root of the count, so an hour with a handful of detections is not
//     flattened by one with a thousand; the legend says so. The colour is the same number.
//   * Everything the picture shows is also in the reading under it and in the table behind
//     "See the numbers"; nothing is hover-only.
//   * Reduced motion (the OS, or the operator's Motion setting) means no entrance and no glide.
//   * Nothing is drawn while the tab is hidden or the card is off screen, the GPU memory is
//     freed when the card goes, and a lost WebGL context drops back to the grid.

export const COLS = 24;
export const RAMP_STEPS = 7;
const SPECIES_SLOTS = 8;

// ── pure: numbers ───────────────────────────────────────────────────────────

/** Position on the ramp, 0..1, of `count` against the busiest cell: square-root spaced. */
export function rampT(count, max) {
  if (!(count > 0) || !(max > 0)) return 0;
  return Math.min(1, Math.sqrt(count / max));
}

/** The sequential step, 1..RAMP_STEPS, for a count (0 for an empty cell). */
export function rampStep(count, max) {
  const t = rampT(count, max);
  return t === 0 ? 0 : Math.min(RAMP_STEPS, Math.max(1, Math.ceil(t * RAMP_STEPS)));
}

/** sRGB 0..1 to linear 0..1 (three.js lights in linear space and encodes on output). */
export function srgbToLinear(c) {
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

/** A colour from the ramp `stops` (arrays of r, g, b in 0..1) at `t`, 0..1, blending neighbours. */
export function mixRamp(stops, t) {
  const x = Math.min(1, Math.max(0, t)) * (stops.length - 1);
  const i = Math.min(stops.length - 2, Math.floor(x));
  const f = x - i;
  const a = stops[i];
  const b = stops[i + 1];
  return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f];
}

// ── pure: the mesh ──────────────────────────────────────────────────────────

/**
 * The positions along one axis of `n` cells where the surface has a vertex: the outer edge of
 * the first cell, the middle of every cell with `sub - 1` points between each pair of middles,
 * and the outer edge of the last. In cell units, so cell `k` has its middle at `k` and
 * covers `k - 0.5 .. k + 0.5`.
 */
export function axis(n, sub) {
  const out = [-0.5];
  for (let k = 0; k < n; k++) {
    out.push(k);
    if (k < n - 1) for (let s = 1; s < sub; s++) out.push(k + s / sub);
  }
  out.push(n - 0.5);
  return out;
}

function smooth(t) {
  return t * t * (3 - 2 * t);
}

/**
 * Build the terrain.
 *
 * `heights` is `rows × cols` row-major, each 0..1 (the height of that cell's middle, as a share
 * of the tallest); `void_` is the same shape, true where the cell is not data (a day the station
 * was off, an hour not yet come). Returns typed arrays ready for a BufferGeometry, and the
 * lattice, so a point in the world can be turned back into a cell.
 *
 * `width` and `depth` are the footprint in world units, `height` the tallest peak.
 */
export function buildHeightField({ heights, void_, rows, cols, subX = 4, subZ = 2, width = 24, depth = 20, height = 7 }) {
  const us = axis(cols, subX);
  const vs = axis(rows, subZ);
  const GX = us.length;
  const GZ = vs.length;
  const clampI = (i, n) => (i < 0 ? 0 : i > n - 1 ? n - 1 : i);
  const at = (r, c) => heights[clampI(r, rows) * cols + clampI(c, cols)];
  const sample = (u, v) => {
    const c0 = Math.floor(u);
    const r0 = Math.floor(v);
    const fu = smooth(u - c0);
    const fv = smooth(v - r0);
    const top = at(r0, c0) * (1 - fu) + at(r0, c0 + 1) * fu;
    const bot = at(r0 + 1, c0) * (1 - fu) + at(r0 + 1, c0 + 1) * fu;
    return top * (1 - fv) + bot * fv;
  };
  const dx = width / cols;
  const dz = depth / rows;
  const n = GX * GZ;
  const positions = new Float32Array(n * 3);
  const values = new Float32Array(n);
  const cell = new Int32Array(n); // the cell a vertex belongs to, row * cols + col
  const isVoid = new Uint8Array(n);
  for (let gz = 0; gz < GZ; gz++) {
    for (let gx = 0; gx < GX; gx++) {
      const i = gz * GX + gx;
      const u = us[gx];
      const v = vs[gz];
      const nearC = clampI(Math.round(u), cols);
      const nearR = clampI(Math.round(v), rows);
      const voidHere = void_ ? void_[nearR * cols + nearC] : false;
      const h = voidHere ? 0 : sample(u, v);
      values[i] = h;
      cell[i] = nearR * cols + nearC;
      isVoid[i] = voidHere ? 1 : 0;
      positions[i * 3] = (u + 0.5 - cols / 2) * dx;
      positions[i * 3 + 1] = h * height;
      positions[i * 3 + 2] = (v + 0.5 - rows / 2) * dz;
    }
  }
  // Normals by central differences over the lattice, in world units.
  const normals = new Float32Array(n * 3);
  for (let gz = 0; gz < GZ; gz++) {
    for (let gx = 0; gx < GX; gx++) {
      const i = gz * GX + gx;
      const l = gz * GX + Math.max(0, gx - 1);
      const r = gz * GX + Math.min(GX - 1, gx + 1);
      const u_ = Math.max(0, gz - 1) * GX + gx;
      const d = Math.min(GZ - 1, gz + 1) * GX + gx;
      const sx = positions[r * 3] - positions[l * 3];
      const sz = positions[d * 3 + 2] - positions[u_ * 3 + 2];
      const hx = (positions[r * 3 + 1] - positions[l * 3 + 1]) / (sx || 1);
      const hz = (positions[d * 3 + 1] - positions[u_ * 3 + 1]) / (sz || 1);
      const len = Math.hypot(hx, 1, hz);
      normals[i * 3] = -hx / len;
      normals[i * 3 + 1] = 1 / len;
      normals[i * 3 + 2] = -hz / len;
    }
  }
  const index = new Uint32Array((GX - 1) * (GZ - 1) * 6);
  let k = 0;
  for (let gz = 0; gz < GZ - 1; gz++) {
    for (let gx = 0; gx < GX - 1; gx++) {
      const a = gz * GX + gx;
      const b = a + 1;
      const c = a + GX;
      const d = c + 1;
      // Wound so the top face points up (+y).
      index[k++] = a;
      index[k++] = c;
      index[k++] = b;
      index[k++] = b;
      index[k++] = c;
      index[k++] = d;
    }
  }
  return { positions, normals, index, values, cell, isVoid, GX, GZ, rows, cols, width, depth, height };
}

/** The cell (`{ day, hour }`) under the world point (x, z), or null outside the footprint. */
export function cellAt(field, x, z) {
  const { rows, cols, width, depth } = field;
  const c = Math.floor(x / (width / cols) + cols / 2);
  const r = Math.floor(z / (depth / rows) + rows / 2);
  if (c < 0 || c >= cols || r < 0 || r >= rows) return null;
  return { day: r, hour: c };
}

/** The heights and void flags the mesh is built from, out of the server's flat arrays. */
export function cellsFromData(data) {
  const rows = data.days;
  const cols = COLS;
  let max = 0;
  for (let i = 0; i < data.n.length; i++) if (data.n[i] > max) max = data.n[i];
  const heights = new Float32Array(rows * cols);
  const void_ = new Uint8Array(rows * cols);
  for (let r = 0; r < rows; r++) {
    const off = data.off && data.off[r] === 1;
    for (let c = 0; c < cols; c++) {
      const i = r * cols + c;
      // The last day's hours after now have not happened.
      const notYet = r === rows - 1 && c > data.hour_now;
      if (off || notYet) {
        void_[i] = 1;
        heights[i] = 0;
      } else {
        heights[i] = max > 0 ? rampT(data.n[i], max) : 0;
      }
    }
  }
  return { rows, cols, max, heights, void_ };
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** Whether the data carries a sunrise and a sunset for at least one day. */
export function hasSun(data) {
  return Array.isArray(data.sun) && data.sun.length === data.days && data.sun.some((s) => Array.isArray(s));
}

/** `Tue 14 Oct` for an ISO date. */
export function dayName(iso) {
  const [y, m, d] = iso.split("-").map(Number);
  const wd = new Date(Date.UTC(y, m - 1, d)).getUTCDay();
  return `${WEEKDAYS[wd]} ${d} ${MONTHS[m - 1]}`;
}

/** The hour `h` as the span it covers, `06:00 to 06:59`. */
export function hourSpan(h) {
  const p = (n) => String(n).padStart(2, "0");
  return `${p(h)}:00 to ${p(h)}:59`;
}

/**
 * What a cell says, in words (the reading line, the screen reader, the tooltip).
 * `{ heading, rows: [[label, value]], note }`.
 */
export function reading(data, day, hour) {
  const i = day * COLS + hour;
  const date = data.dates[day];
  const heading = `${dayName(date)}, ${hourSpan(hour)}`;
  if (data.off && data.off[day] === 1) {
    return { heading, rows: [], note: "The station was not listening this day." };
  }
  if (day === data.days - 1 && hour > data.hour_now) {
    return { heading, rows: [], note: "This hour has not happened yet." };
  }
  const n = data.n[i];
  if (n === 0) return { heading, rows: [["Detections", "0"]], note: "The station listened and heard nothing." };
  const top = data.top[i];
  const rows = [["Detections", String(n)]];
  if (top >= 0 && data.species[top]) rows.push(["Most heard", `${data.species[top]} (${data.top_n[i]})`]);
  else if (data.top_n[i] > 0) rows.push(["Most heard", `another species (${data.top_n[i]})`]);
  return { heading, rows, note: "" };
}

/** The reading as one sentence. */
export function readingText(data, day, hour) {
  const r = reading(data, day, hour);
  let out = r.heading;
  if (r.rows.length) out += `: ${r.rows.map(([l, v]) => `${l} ${v}`).join(", ")}`;
  if (r.note) out += `${r.rows.length ? ". " : ": "}${r.note}`;
  return out;
}

// ── the page ────────────────────────────────────────────────────────────────

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
  // A token the page does not define falls back to the system text colour, never to a colour of
  // ours: no colour is written in this file, they are all read from the stylesheet.
  probe.fillStyle = "canvastext";
  probe.fillStyle = raw || "canvastext";
  probe.fillRect(0, 0, 1, 1);
  const d = probe.getImageData(0, 0, 1, 1).data;
  return [d[0] / 255, d[1] / 255, d[2] / 255];
}

const css = (c) => `color(srgb ${c[0].toFixed(4)} ${c[1].toFixed(4)} ${c[2].toFixed(4)})`;

function palette(card) {
  const seq = [];
  for (let i = 1; i <= RAMP_STEPS; i++) seq.push(cssRgb(card, `--seq-${i}`));
  const species = [];
  for (let i = 1; i <= SPECIES_SLOTS; i++) species.push(cssRgb(card, `--chart-${i}`));
  species.push(cssRgb(card, "--chart-other"));
  return {
    seq,
    species,
    floor: cssRgb(card, "--chart-track"),
    voidFill: cssRgb(card, "--chart-grid-strong"),
    axis: cssRgb(card, "--chart-axis"),
    label: cssRgb(card, "--chart-label"),
    annot: cssRgb(card, "--chart-annot"),
    surface: cssRgb(card, "--surface"),
  };
}

function reducedMotion() {
  return (
    (window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches) ||
    document.documentElement.getAttribute("data-motion") === "reduced"
  );
}

function hasWebGL() {
  try {
    const c = document.createElement("canvas");
    return !!(c.getContext("webgl2") || c.getContext("webgl"));
  } catch (e) {
    return false;
  }
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

/** The ranges the server can draw, shortest first (`terrain::RANGES`). */
const RANGES = [30, 90, 365];

class Terrain {
  constructor(card) {
    this.card = card;
    this.days = Number(card.dataset.trDays) || 30;
    this.span = card.querySelector('[data-tr="span"]');
    this.take = card.querySelector('[data-tr="take"]');
    this.sunKey = card.querySelector('[data-tr="sun-key"]');
    this.view = "grid";
    this.colour = "activity";
    this.data = null;
    this.cells = null;
    this.cursor = null; // { day, hour }
    this.pal = null;
    this.gl = null;
    this.glState = "idle"; // idle | loading | ready | failed
    this.tipOn = false;
    this.stage = card.querySelector(".tr-stage");
    this.gridWrap = card.querySelector(".tr-grid");
    this.canvas = card.querySelector(".tr-grid-canvas");
    this.glWrap = card.querySelector(".tr-3d");
    this.glCanvas = card.querySelector(".tr-gl-canvas");
    this.labels = card.querySelector(".tr-labels");
    this.msg = card.querySelector(".tr-msg");
    this.readout = card.querySelector(".tr-readout");
    this.legend = card.querySelector(".tr-legend");
    this.idleText = this.readout ? this.readout.textContent : "";
    this.say();
    this.bind();
    this.load();
  }

  /** Which range the card is showing, in words. */
  say() {
    if (!this.span) return;
    this.span.textContent = this.days === 365 ? "Last year" : `Last ${this.days} days`;
  }

  // ── controls ──
  bind() {
    const seg = (name, fn) => {
      const g = this.card.querySelector(`[data-tr="${name}"]`);
      if (!g) return;
      g.addEventListener("click", (e) => {
        const b = e.target.closest("button");
        if (!b || b.disabled || b.getAttribute("aria-disabled") === "true") return;
        g.querySelectorAll("button").forEach((o) => {
          const on = o === b;
          o.classList.toggle("active", on);
          o.setAttribute("aria-pressed", on ? "true" : "false");
        });
        fn(b);
      });
    };
    // The page owns the range (one control for every figure on the tab). The card follows it,
    // showing the shortest range it can draw that is not shorter than the choice, and says which.
    this.card.addEventListener("bnb:range", (e) => {
      const want = Number((e.detail && e.detail.days) || this.days);
      const days = RANGES.find((r) => r >= want) || RANGES[RANGES.length - 1];
      if (days === this.days) return;
      this.days = days;
      this.say();
      this.load();
    });
    seg("view", (b) => this.setView(b.dataset.view));
    seg("colour", (b) => {
      this.colour = b.dataset.colour;
      this.recolour();
    });
    this.bindGrid();
    this.bindTable();
    this.themeWatch();
    window.addEventListener("pagehide", () => this.dispose());
  }

  themeWatch() {
    const redo = () => {
      if (!this.data) return;
      this.pal = palette(this.card);
      this.drawGrid();
      this.recolour();
    };
    new MutationObserver(redo).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-contrast"] });
    if (window.matchMedia) window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", redo);
  }

  bindTable() {
    const d = this.card.querySelector(".tr-numbers");
    if (!d) return;
    d.addEventListener("toggle", () => {
      if (d.open) this.loadTable();
    });
  }

  loadTable() {
    const slot = this.card.querySelector(".tr-table-slot");
    if (!slot || !window.htmx) return;
    const base = this.card.dataset.trTable;
    // The slot is empty in the markup: "Loading…" written there is text in a closed
    // disclosure, which nobody sees and a gate then (rightly) cannot tell from a
    // stuck one. It is written here, when the reader has asked for the numbers.
    const note = (text) => {
      slot.textContent = "";
      const span = document.createElement("span");
      span.className = "bnb-meta tr-loading";
      span.textContent = text;
      slot.appendChild(span);
    };
    note("Loading…");
    // htmx does not swap a 4xx/5xx body, so a failed request would leave the
    // placeholder up for good; say so when the request is over and nothing came.
    const failed = () => {
      if (slot.querySelector(".tr-loading")) note("The numbers are not available just now.");
    };
    window.htmx.ajax("GET", `${base}?days=${this.days}`, slot).then(failed, failed);
  }

  // ── data ──
  async load() {
    this.say("Loading…");
    const days = this.days;
    let data;
    try {
      const res = await fetch(url(`${this.card.dataset.trFeed}?days=${days}`), { credentials: "same-origin" });
      data = await res.json();
    } catch (e) {
      data = null;
    }
    if (days !== this.days) return; // another range was asked for meanwhile
    if (!data || data.error || !data.n || data.n.length !== data.days * COLS) {
      this.say("The terrain is not available just now.");
      this.data = null;
      this.showFinding(null, false);
      this.drawGrid();
      return;
    }
    this.data = data;
    this.showFinding(data.take, hasSun(data));
    this.cells = cellsFromData(data);
    this.pal = palette(this.card);
    this.cursor = null;
    this.say("");
    this.resetReading();
    this.fillLegend();
    this.drawGrid();
    if (this.gl) this.rebuildMesh();
    const d = this.card.querySelector(".tr-numbers");
    if (d && d.open) this.loadTable();
    this.describe();
  }

  say(text) {
    if (this.msg) this.msg.textContent = text;
  }

  /**
   * What the picture found, as a sentence under the title (plain text: it came from the server's
   * counts and carries no markup), and the legend line for the sun's lines when there are any.
   */
  showFinding(text, sun) {
    if (this.take) {
      this.take.textContent = text || "";
      this.take.hidden = !text;
    }
    if (this.sunKey) this.sunKey.hidden = !sun;
  }

  describe() {
    const d = this.data;
    const label = d
      ? `Soundscape terrain: the last ${d.days} days by the 24 hours of the day. ${d.take ? d.take + " " : ""}${this.cells.max.toLocaleString()} detections in the busiest hour. Use the arrow keys to move through the hours and days; the reading under the picture says what is there.`
      : "Soundscape terrain (not available)";
    this.canvas.setAttribute("aria-label", label);
    if (this.glCanvas) this.glCanvas.setAttribute("aria-label", label.replace("Use the arrow keys to move through the hours and days;", "Arrow keys turn the view, plus and minus zoom, Home resets, Shift and the arrows move the cursor;"));
  }

  resetReading() {
    if (this.readout) this.readout.textContent = this.idleText;
  }

  show(day, hour) {
    this.cursor = { day, hour };
    if (this.readout && this.data) this.readout.textContent = readingText(this.data, day, hour);
    this.drawGrid();
    if (this.gl) this.moveCursorMarker();
  }

  // ── legend ──
  fillLegend() {
    if (!this.legend || !this.data) return;
    const d = this.data;
    const byActivity = this.legend.querySelector(".tr-key-activity");
    const bySpecies = this.legend.querySelector(".tr-key-species");
    if (bySpecies) {
      bySpecies.textContent = "";
      const n = Math.min(SPECIES_SLOTS, d.species.length);
      for (let i = 0; i < n; i++) {
        const item = el("span", "bnb-meta viz-legend-item");
        const sw = el("span", `viz-swatch ck-sw ck-c${i + 1}`);
        item.appendChild(sw);
        item.appendChild(document.createTextNode(d.species[i]));
        bySpecies.appendChild(item);
      }
      if (d.species.length > SPECIES_SLOTS) {
        const item = el("span", "bnb-meta viz-legend-item");
        item.appendChild(el("span", "viz-swatch ck-sw ck-cother"));
        item.appendChild(document.createTextNode("Other species"));
        bySpecies.appendChild(item);
      }
    }
    this.legend.classList.toggle("is-species", this.colour === "species");
    if (byActivity) {
      const max = this.cells.max;
      const top = byActivity.querySelector(".tr-key-max");
      if (top) top.textContent = max.toLocaleString();
    }
  }

  // ── colours of one cell ──
  cellColour(day, hour) {
    const p = this.pal;
    const i = day * COLS + hour;
    if (this.cells.void_[i]) return p.voidFill;
    const n = this.data.n[i];
    if (this.colour === "species") {
      if (n === 0) return p.floor;
      const t = this.data.top[i];
      return p.species[t >= 0 && t < SPECIES_SLOTS ? t : SPECIES_SLOTS];
    }
    if (n === 0) return p.floor;
    return mixRamp(p.seq, rampT(n, this.cells.max));
  }

  // ── the flat grid ──
  bindGrid() {
    const c = this.canvas;
    if (!c) return;
    const cellFrom = (e) => {
      const r = c.getBoundingClientRect();
      const g = this.gridGeom;
      if (!g || !this.data) return null;
      const x = e.clientX - r.left - g.left;
      const y = e.clientY - r.top - g.top;
      if (x < 0 || y < 0 || x >= g.cw * COLS || y >= g.rh * this.data.days) return null;
      return { day: Math.floor(y / g.rh), hour: Math.floor(x / g.cw) };
    };
    c.addEventListener("pointermove", (e) => {
      const cell = cellFrom(e);
      if (cell) this.show(cell.day, cell.hour);
    });
    c.addEventListener("pointerdown", (e) => {
      const cell = cellFrom(e);
      if (cell) this.show(cell.day, cell.hour);
    });
    c.addEventListener("pointerleave", (e) => {
      if (e.pointerType === "mouse") {
        this.cursor = null;
        this.resetReading();
        this.drawGrid();
      }
    });
    c.addEventListener("keydown", (e) => this.gridKey(e));
    c.addEventListener("focus", () => {
      if (!this.data) return;
      if (!this.cursor) this.show(this.data.days - 1, Math.min(23, this.data.hour_now));
    });
    new ResizeObserver(() => this.drawGrid()).observe(this.gridWrap);
  }

  gridKey(e) {
    if (!this.data) return;
    const cur = this.cursor || { day: this.data.days - 1, hour: Math.min(23, this.data.hour_now) };
    let { day, hour } = cur;
    if (e.key === "ArrowLeft") hour -= 1;
    else if (e.key === "ArrowRight") hour += 1;
    else if (e.key === "ArrowUp") day -= 1;
    else if (e.key === "ArrowDown") day += 1;
    else if (e.key === "Home") hour = 0;
    else if (e.key === "End") hour = COLS - 1;
    else if (e.key === "PageUp") day = Math.max(0, day - 7);
    else if (e.key === "PageDown") day = Math.min(this.data.days - 1, day + 7);
    else if (e.key === "Escape") {
      this.cursor = null;
      this.resetReading();
      this.drawGrid();
      return;
    } else return;
    e.preventDefault();
    this.show(Math.max(0, Math.min(this.data.days - 1, day)), Math.max(0, Math.min(COLS - 1, hour)));
  }

  drawGrid() {
    const c = this.canvas;
    if (!c || this.view !== "grid") return;
    const w = Math.max(240, Math.floor(this.gridWrap.clientWidth));
    const h = this.gridWrap.clientHeight > 100 ? Math.floor(this.gridWrap.clientHeight) : 380;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    if (c.width !== Math.round(w * dpr) || c.height !== Math.round(h * dpr)) {
      c.width = Math.round(w * dpr);
      c.height = Math.round(h * dpr);
    }
    const ctx = c.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    if (!this.data || !this.pal) return;
    const p = this.pal;
    const days = this.data.days;
    const font = getComputedStyle(this.card).fontFamily;
    const left = w < 420 ? 44 : 64;
    const top = 20;
    const bottom = 4;
    const cw = (w - left - 4) / COLS;
    const rh = (h - top - bottom) / days;
    this.gridGeom = { left, top, cw, rh };
    // Cells.
    for (let r = 0; r < days; r++) {
      const y0 = Math.round(top + r * rh);
      const y1 = Math.round(top + (r + 1) * rh);
      for (let k = 0; k < COLS; k++) {
        const x0 = Math.round(left + k * cw);
        const x1 = Math.round(left + (k + 1) * cw);
        ctx.fillStyle = css(this.cellColour(r, k));
        ctx.fillRect(x0, y0, Math.max(1, x1 - x0 - (cw > 6 ? 1 : 0)), Math.max(1, y1 - y0 - (rh > 6 ? 1 : 0)));
      }
    }
    // Hatch the days the station was off and the hours not yet come.
    ctx.save();
    ctx.strokeStyle = css(p.axis);
    ctx.globalAlpha = 0.55;
    ctx.lineWidth = 1;
    for (let r = 0; r < days; r++) {
      const off = this.data.off && this.data.off[r] === 1;
      const y0 = top + r * rh;
      for (let k = 0; k < COLS; k++) {
        if (!(off || (r === days - 1 && k > this.data.hour_now))) continue;
        const x0 = left + k * cw;
        if (rh < 3) continue;
        ctx.beginPath();
        ctx.moveTo(x0, y0 + rh);
        ctx.lineTo(x0 + Math.min(cw, rh), y0 + rh - Math.min(cw, rh));
        ctx.stroke();
      }
    }
    ctx.restore();
    // Where the sun rose and set each day, so "when" can be read against the light.
    if (hasSun(this.data)) this.drawSun(ctx, left, top, cw, rh, days);
    // Axes: hours along the top, days down the side.
    ctx.fillStyle = css(p.label);
    ctx.font = `10px ${font}`;
    ctx.textBaseline = "alphabetic";
    ctx.textAlign = "left";
    for (const [h, t] of [[0, "12a"], [6, "6a"], [12, "12p"], [18, "6p"]]) ctx.fillText(t, left + h * cw, 12);
    ctx.textAlign = "right";
    const stepD = days <= 30 ? 7 : days <= 90 ? 30 : 90;
    for (let back = 0; back < days; back += stepD) {
      const r = days - 1 - back;
      const label = back === 0 ? "today" : `${back} d ago`;
      ctx.fillText(label, left - 6, top + (r + 0.5) * rh + 3);
    }
    // The cursor.
    if (this.cursor) {
      const { day, hour } = this.cursor;
      ctx.strokeStyle = css(p.annot);
      ctx.lineWidth = 2;
      ctx.strokeRect(Math.round(left + hour * cw) - 0.5, Math.round(top + day * rh) - 0.5, Math.max(cw, 4), Math.max(rh, 4));
    }
  }

  /**
   * Sunrise and sunset as two dashed lines down the grid, one point per day, each with a halo of
   * the card's own surface so it stays legible over the darkest and the lightest cells. A day
   * with no sunrise (a polar one) breaks the line rather than being joined across.
   */
  drawSun(ctx, left, top, cw, rh, days) {
    const sun = this.data.sun;
    ctx.save();
    ctx.lineJoin = "round";
    for (const [stroke, width, dash] of [[css(this.pal.surface), 3.5, []], [css(this.pal.annot), 1.5, [4, 3]]]) {
      ctx.strokeStyle = stroke;
      ctx.lineWidth = width;
      ctx.setLineDash(dash);
      for (const which of [0, 1]) {
        ctx.beginPath();
        let pen = false;
        for (let r = 0; r < days; r++) {
          const s = sun[r];
          if (!s) {
            pen = false;
            continue;
          }
          const x = left + s[which] * cw;
          const y = top + (r + 0.5) * rh;
          if (pen) ctx.lineTo(x, y);
          else ctx.moveTo(x, y);
          pen = true;
        }
        ctx.stroke();
      }
    }
    ctx.restore();
  }

  // ── view ──
  async setView(view) {
    if (view === "3d") {
      this.view = "3d";
      this.stage.dataset.view = "3d";
      this.gridWrap.hidden = true;
      this.glWrap.hidden = false;
      await this.ensureGL();
      if (this.glState !== "ready") this.setView("grid");
      return;
    }
    this.view = "grid";
    this.stage.dataset.view = "grid";
    this.glWrap.hidden = true;
    this.gridWrap.hidden = false;
    this.pauseGL();
    // The segmented control follows the view when it was changed from here (a fallback).
    const g = this.card.querySelector('[data-tr="view"]');
    if (g)
      g.querySelectorAll("button").forEach((b) => {
        const on = b.dataset.view === "grid";
        b.classList.toggle("active", on);
        b.setAttribute("aria-pressed", on ? "true" : "false");
      });
    this.drawGrid();
  }

  recolour() {
    if (!this.data) return;
    this.legend && this.legend.classList.toggle("is-species", this.colour === "species");
    this.drawGrid();
    if (this.gl) {
      this.paintMesh();
      this.requestRender();
    }
  }

  // ── WebGL ──
  async ensureGL() {
    if (this.glState === "ready") {
      this.resumeGL();
      return;
    }
    if (this.glState === "loading") return;
    this.glState = "loading";
    if (!hasWebGL()) {
      this.say("This browser cannot draw the 3D terrain, so the grid is shown; it has the same numbers.");
      this.glState = "failed";
      this.disable3D("No WebGL here");
      return;
    }
    this.say("Loading the 3D view…");
    let T;
    try {
      // The only place the library is named. A relative URL, so a reverse-proxy prefix comes
      // with this module's own; the ?v= is the file's hash (a test holds the two together).
      T = await import(`./vendor/three/three.module.min.js?v=${THREE_HASH}`);
    } catch (e) {
      this.say("The 3D view could not be loaded, so the grid is shown.");
      this.glState = "failed";
      return;
    }
    try {
      this.startGL(T);
      this.glState = "ready";
      this.say("");
    } catch (e) {
      this.say("The 3D view could not start in this browser, so the grid is shown.");
      this.glState = "failed";
      this.disposeGL();
    }
  }

  disable3D(why) {
    const b = this.card.querySelector('[data-tr="view"] [data-view="3d"]');
    if (b) {
      b.setAttribute("aria-disabled", "true");
      b.title = why;
    }
  }

  startGL(T) {
    this.T = T;
    const canvas = this.glCanvas;
    const renderer = new T.WebGLRenderer({ canvas, antialias: true, alpha: true, powerPreference: "low-power" });
    renderer.setClearColor(0x000000, 0);
    const scene = new T.Scene();
    const camera = new T.PerspectiveCamera(38, 1, 0.5, 400);
    scene.add(new T.AmbientLight(0xffffff, 1.15));
    const sun = new T.DirectionalLight(0xffffff, 1.9);
    sun.position.set(-14, 26, 18);
    scene.add(sun);
    this.gl = { renderer, scene, camera, sun, mesh: null, floor: null, cursor: null, raycaster: new T.Raycaster(), dirty: true, raf: 0, visible: true, intro: 0 };
    this.orbit = { theta: 0.5, phi: 0.95, r: 32, tx: 0, ty: 1.6, tz: 0 };
    this.orbitHome = { ...this.orbit };
    this.metrics = [];
    this.buildFloor();
    this.rebuildMesh();
    this.bindGL();
    this.sizeGL();
    this.resumeGL();
    if (!reducedMotion()) {
      this.gl.intro = performance.now();
      this.gl.mesh.scale.y = 0.02;
      this.requestRender();
    }
  }

  buildFloor() {
    const T = this.T;
    const g = this.gl;
    const w = 24;
    const d = 20;
    const pts = [];
    const seg = (a, b) => pts.push(a[0], 0, a[1], b[0], 0, b[1]);
    seg([-w / 2, -d / 2], [w / 2, -d / 2]);
    seg([w / 2, -d / 2], [w / 2, d / 2]);
    seg([w / 2, d / 2], [-w / 2, d / 2]);
    seg([-w / 2, d / 2], [-w / 2, -d / 2]);
    for (const h of [6, 12, 18]) seg([-w / 2 + h, -d / 2], [-w / 2 + h, d / 2]);
    const geo = new T.BufferGeometry();
    geo.setAttribute("position", new T.BufferAttribute(new Float32Array(pts), 3));
    const mat = new T.LineBasicMaterial({ color: 0x888888, transparent: true, opacity: 0.55 });
    g.floor = new T.LineSegments(geo, mat);
    g.scene.add(g.floor);
    const cg = new T.BufferGeometry();
    cg.setAttribute("position", new T.BufferAttribute(new Float32Array(8 * 3), 3));
    g.cursor = new T.LineSegments(cg, new T.LineBasicMaterial({ color: 0xffffff }));
    g.cursor.visible = false;
    g.scene.add(g.cursor);
    this.paintFloor();
  }

  paintFloor() {
    const g = this.gl;
    if (!g || !this.pal) return;
    g.floor.material.color.setRGB(srgbToLinear(this.pal.axis[0]), srgbToLinear(this.pal.axis[1]), srgbToLinear(this.pal.axis[2]));
    g.cursor.material.color.setRGB(srgbToLinear(this.pal.annot[0]), srgbToLinear(this.pal.annot[1]), srgbToLinear(this.pal.annot[2]));
  }

  rebuildMesh() {
    const T = this.T;
    const g = this.gl;
    if (!g || !this.data) return;
    if (g.mesh) {
      g.scene.remove(g.mesh);
      g.mesh.geometry.dispose();
      g.mesh.material.dispose();
    }
    const c = this.cells;
    const field = buildHeightField({
      heights: c.heights,
      void_: c.void_,
      rows: c.rows,
      cols: c.cols,
      subX: 4,
      subZ: c.rows <= 120 ? 2 : 1,
    });
    this.field = field;
    const geo = new T.BufferGeometry();
    geo.setAttribute("position", new T.BufferAttribute(field.positions, 3));
    geo.setAttribute("normal", new T.BufferAttribute(field.normals, 3));
    geo.setAttribute("color", new T.BufferAttribute(new Float32Array(field.positions.length), 3));
    geo.setIndex(new T.BufferAttribute(field.index, 1));
    geo.computeBoundingSphere();
    const mat = new T.MeshLambertMaterial({ vertexColors: true });
    g.mesh = new T.Mesh(geo, mat);
    g.scene.add(g.mesh);
    this.paintMesh();
    this.buildAxisLabels();
    if (!g.intro) g.mesh.scale.y = 1;
    this.requestRender();
  }

  paintMesh() {
    const g = this.gl;
    if (!g || !g.mesh || !this.field || !this.pal) return;
    const f = this.field;
    const col = g.mesh.geometry.attributes.color.array;
    const p = this.pal;
    const lin = (c) => [srgbToLinear(c[0]), srgbToLinear(c[1]), srgbToLinear(c[2])];
    const seqLin = p.seq.map(lin);
    const spLin = p.species.map(lin);
    const floor = lin(p.floor);
    const voidC = lin(p.voidFill);
    const n = this.data.n;
    for (let i = 0; i < f.values.length; i++) {
      let c;
      if (f.isVoid[i]) c = voidC;
      else if (this.colour === "species") {
        const cellIdx = f.cell[i];
        if (n[cellIdx] === 0) c = floor;
        else {
          const t = this.data.top[cellIdx];
          c = spLin[t >= 0 && t < SPECIES_SLOTS ? t : SPECIES_SLOTS];
        }
      } else if (f.values[i] <= 0) c = floor;
      else c = mixRamp(seqLin, f.values[i]);
      col[i * 3] = c[0];
      col[i * 3 + 1] = c[1];
      col[i * 3 + 2] = c[2];
    }
    g.mesh.geometry.attributes.color.needsUpdate = true;
    this.paintFloor();
  }

  buildAxisLabels() {
    const lab = this.labels;
    if (!lab || !this.data) return;
    lab.textContent = "";
    this.axisLabels = [];
    const days = this.data.days;
    const add = (text, x, z, cls) => {
      const s = el("span", `tr-lbl ${cls || ""}`, text);
      lab.appendChild(s);
      this.axisLabels.push({ el: s, x, z });
    };
    for (const [h, t] of [[0, "12a"], [6, "6a"], [12, "12p"], [18, "6p"], [24, "12a"]]) add(t, -12 + h, 11.8, "tr-lbl-hour");
    const stepD = days <= 30 ? 7 : days <= 90 ? 30 : 90;
    for (let back = 0; back < days; back += stepD) {
      const r = days - 1 - back;
      add(back === 0 ? "today" : `${back} d ago`, -14.2, (r + 0.5 - days / 2) * (20 / days), "tr-lbl-day");
    }
  }

  moveCursorMarker() {
    const g = this.gl;
    if (!g || !this.field || !this.cursor) {
      if (g && g.cursor) g.cursor.visible = false;
      return;
    }
    const f = this.field;
    const { day, hour } = this.cursor;
    const dx = f.width / f.cols;
    const dz = f.depth / f.rows;
    const x0 = (hour - f.cols / 2) * dx;
    const x1 = x0 + dx;
    const z0 = (day - f.rows / 2) * dz;
    const z1 = z0 + Math.max(dz, 0.12);
    const hgt = this.cells.heights[day * f.cols + hour] * f.height + 0.05;
    const a = g.cursor.geometry.attributes.position.array;
    const pts = [
      x0, hgt, z0, x1, hgt, z0,
      x1, hgt, z0, x1, hgt, z1,
      x1, hgt, z1, x0, hgt, z1,
      x0, hgt, z1, x0, hgt, z0,
    ];
    a.set(pts);
    g.cursor.geometry.attributes.position.needsUpdate = true;
    g.cursor.visible = true;
    this.requestRender();
  }

  // ── WebGL: input ──
  bindGL() {
    const c = this.glCanvas;
    const o = () => this.orbit;
    const pointers = new Map();
    let pinch = 0;
    let moved = false;
    let down = null;
    c.addEventListener("pointerdown", (e) => {
      c.setPointerCapture(e.pointerId);
      pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
      moved = false;
      down = { x: e.clientX, y: e.clientY };
      if (pointers.size === 2) {
        const [a, b] = [...pointers.values()];
        pinch = Math.hypot(a.x - b.x, a.y - b.y);
      }
    });
    c.addEventListener("pointermove", (e) => {
      const p = pointers.get(e.pointerId);
      if (p) {
        const dx = e.clientX - p.x;
        const dy = e.clientY - p.y;
        p.x = e.clientX;
        p.y = e.clientY;
        if (pointers.size === 1) {
          if (down && Math.hypot(e.clientX - down.x, e.clientY - down.y) > 3) moved = true;
          o().theta -= dx * 0.008;
          o().phi = Math.min(1.5, Math.max(0.15, o().phi - dy * 0.008));
          this.requestRender();
        } else if (pointers.size === 2) {
          const [a, b] = [...pointers.values()];
          const d = Math.hypot(a.x - b.x, a.y - b.y);
          if (pinch > 0) this.zoom(pinch / d);
          pinch = d;
          moved = true;
        }
      } else if (e.pointerType === "mouse") {
        this.pick(e, false);
      }
    });
    const up = (e) => {
      pointers.delete(e.pointerId);
      pinch = 0;
      if (!moved && e.type === "pointerup") this.pick(e, true);
    };
    c.addEventListener("pointerup", up);
    c.addEventListener("pointercancel", up);
    c.addEventListener("pointerleave", (e) => {
      if (e.pointerType === "mouse") this.hideTip();
    });
    c.addEventListener(
      "wheel",
      (e) => {
        // The page keeps its scroll unless the reader has taken hold of the terrain: a click
        // on it, or a pinch on a trackpad (which arrives as a wheel with ctrl held).
        if (document.activeElement !== c && !e.ctrlKey) return;
        e.preventDefault();
        this.zoom(Math.exp(e.deltaY * 0.0012));
      },
      { passive: false }
    );
    c.addEventListener("keydown", (e) => this.glKey(e));
    c.addEventListener("focus", () => {
      if (!this.cursor && this.data) this.show(this.data.days - 1, Math.min(23, this.data.hour_now));
    });
    new ResizeObserver(() => this.sizeGL()).observe(this.glWrap);
    // Nothing is drawn while the tab is hidden or the card is off screen.
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) this.requestRender();
    });
    if (typeof IntersectionObserver === "function") {
      new IntersectionObserver((entries) => {
        const on = entries.some((en) => en.isIntersecting);
        if (this.gl) {
          this.gl.visible = on;
          if (on) this.requestRender();
        }
      }).observe(this.glWrap);
    }
    // A lost context is the browser reclaiming the GPU: say so and fall back.
    c.addEventListener("webglcontextlost", (e) => {
      e.preventDefault();
      this.lostTimer = setTimeout(() => {
        this.say("The 3D view was interrupted by the browser, so the grid is shown.");
        this.glState = "failed";
        this.disposeGL();
        this.setView("grid");
      }, 2500);
    });
    c.addEventListener("webglcontextrestored", () => {
      clearTimeout(this.lostTimer);
      this.say("");
      this.disposeGL();
      this.glState = "idle";
      this.ensureGL();
    });
  }

  glKey(e) {
    const o = this.orbit;
    const step = 0.12;
    if (e.shiftKey && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key) && this.data) {
      const cur = this.cursor || { day: this.data.days - 1, hour: Math.min(23, this.data.hour_now) };
      let { day, hour } = cur;
      if (e.key === "ArrowLeft") hour -= 1;
      if (e.key === "ArrowRight") hour += 1;
      if (e.key === "ArrowUp") day -= 1;
      if (e.key === "ArrowDown") day += 1;
      e.preventDefault();
      this.show(Math.max(0, Math.min(this.data.days - 1, day)), Math.max(0, Math.min(COLS - 1, hour)));
      return;
    }
    if (e.key === "ArrowLeft") o.theta += step;
    else if (e.key === "ArrowRight") o.theta -= step;
    else if (e.key === "ArrowUp") o.phi = Math.max(0.15, o.phi - step);
    else if (e.key === "ArrowDown") o.phi = Math.min(1.5, o.phi + step);
    else if (e.key === "+" || e.key === "=") this.zoom(0.85);
    else if (e.key === "-" || e.key === "_") this.zoom(1 / 0.85);
    else if (e.key === "Home") Object.assign(o, this.orbitHome);
    else if (e.key === "Escape") {
      this.cursor = null;
      this.resetReading();
      this.hideTip();
      if (this.gl.cursor) this.gl.cursor.visible = false;
    } else return;
    e.preventDefault();
    this.requestRender();
  }

  zoom(factor) {
    const o = this.orbit;
    o.r = Math.min(80, Math.max(14, o.r * factor));
    this.requestRender();
  }

  // Which cell is under the pointer.
  pick(e, keep) {
    const g = this.gl;
    if (!g || !g.mesh || !this.field) return;
    const r = this.glCanvas.getBoundingClientRect();
    const T = this.T;
    const v = new T.Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1);
    g.raycaster.setFromCamera(v, g.camera);
    const hit = g.raycaster.intersectObject(g.mesh, false)[0];
    if (!hit) {
      if (!keep) this.hideTip();
      return;
    }
    // The mesh is scaled in y while it rises; undo that to find the ground point's x and z.
    const cell = cellAt(this.field, hit.point.x, hit.point.z);
    if (!cell) return;
    this.cursor = cell;
    this.moveCursorMarker();
    if (this.readout) this.readout.textContent = readingText(this.data, cell.day, cell.hour);
    this.showTip(reading(this.data, cell.day, cell.hour), e.clientX - r.left, e.clientY - r.top);
  }

  showTip(rd, px, py) {
    let tip = this.glWrap.querySelector(".tr-tip");
    if (!tip) {
      tip = el("div", "ck-tip tr-tip");
      tip.setAttribute("role", "presentation");
      this.glWrap.appendChild(tip);
    }
    tip.textContent = "";
    tip.appendChild(el("div", "ck-tip-h", rd.heading));
    for (const [l, v] of rd.rows) {
      const row = el("div", "ck-tip-r");
      row.appendChild(el("span", "ck-tip-l", l));
      row.appendChild(el("b", "ck-tip-v", v));
      tip.appendChild(row);
    }
    if (rd.note) tip.appendChild(el("div", "ck-tip-n", rd.note));
    tip.classList.add("on");
    const w = this.glWrap.clientWidth;
    const th = tip.offsetHeight;
    const tw = tip.offsetWidth;
    let left = px + 14;
    if (left + tw > w - 2) left = px - tw - 14;
    let top = py - th - 12;
    if (top < 2) top = py + 16;
    tip.style.setProperty("left", `${Math.max(2, Math.round(left))}px`);
    tip.style.setProperty("top", `${Math.round(top)}px`);
  }

  hideTip() {
    const tip = this.glWrap && this.glWrap.querySelector(".tr-tip");
    if (tip) tip.classList.remove("on");
  }

  // ── WebGL: drawing ──
  sizeGL() {
    const g = this.gl;
    if (!g) return;
    const w = Math.max(200, this.glWrap.clientWidth);
    const h = Math.max(200, this.glWrap.clientHeight);
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    g.renderer.setPixelRatio(dpr);
    g.renderer.setSize(w, h, false);
    g.camera.aspect = w / h;
    g.camera.updateProjectionMatrix();
    // Where the view was put, and whether the reader has moved it since: a stage that is resized
    // (a phone turned on its side) is re-fitted only if nobody has turned or zoomed it.
    const keys = ["theta", "phi", "r", "tx", "ty", "tz"];
    const unmoved = !this.homeApplied || keys.every((k) => Math.abs(this.orbit[k] - this.orbitHome[k]) < 1e-9);
    this.fitHome();
    if (unmoved) {
      for (const k of keys) this.orbit[k] = this.orbitHome[k];
      this.homeApplied = true;
    }
    this.requestRender();
  }

  /**
   * The distance and the aim at which the whole slab, the labels round it and the tallest peak
   * stand inside this canvas, at the home angles. A fixed distance fitted one shape of stage: on
   * a wide desktop one the near edge ran off the bottom and the right, and on a phone the mesh
   * was cropped on the left and the bottom. This projects the corners of the box the scene
   * fills through the real camera, takes the nearest distance at which they all land inside the
   * canvas, and moves the aim sideways and up or down until the box has as much room on one
   * side as on the other. The aim moves in the picture's plane, so the turning that follows is
   * about a point that is still on the slab.
   */
  fitHome() {
    const g = this.gl;
    const T = this.T;
    const cam = g.camera;
    const home = this.orbitHome;
    // A narrow stage is better served by a view nearer the front, where the 24 hours run across
    // the picture rather than away along a diagonal.
    home.theta = cam.aspect < 1.2 ? 0.3 : 0.5;
    const peak = (this.field && this.field.height) || 7;
    // The slab is 24 by 20; the day labels are centred on a point 2.2 beyond its left edge and run
    // about 2 further, and the hour labels stand 1.8 beyond its front.
    const corners = [];
    for (const x of [-16.5, 12.8]) for (const z of [-11, 12.4]) for (const y of [0, peak]) corners.push(new T.Vector3(x, y, z));
    const v = new T.Vector3();
    const extent = (r, tx, ty, tz) => {
      const sp = Math.sin(home.phi);
      cam.position.set(tx + r * sp * Math.sin(home.theta), ty + r * Math.cos(home.phi), tz + r * sp * Math.cos(home.theta));
      cam.lookAt(tx, ty, tz);
      cam.updateMatrixWorld();
      let minX = Infinity;
      let maxX = -Infinity;
      let minY = Infinity;
      let maxY = -Infinity;
      for (const c of corners) {
        v.copy(c).project(cam);
        minX = Math.min(minX, v.x);
        maxX = Math.max(maxX, v.x);
        minY = Math.min(minY, v.y);
        maxY = Math.max(maxY, v.y);
      }
      return { minX, maxX, minY, maxY };
    };
    const LIMIT = 0.96;
    const tanV = Math.tan((cam.fov * Math.PI) / 360);
    const tanH = tanV * cam.aspect;
    let [tx, ty, tz] = [0, 1.6, 0];
    let r = 31;
    for (let pass = 0; pass < 5; pass++) {
      let lo = 6;
      let hi = 240;
      for (let i = 0; i < 30; i++) {
        const mid = (lo + hi) / 2;
        const e = extent(mid, tx, ty, tz);
        if (Math.max(-e.minX, e.maxX, -e.minY, e.maxY) <= LIMIT) hi = mid;
        else lo = mid;
      }
      r = hi;
      const e = extent(r, tx, ty, tz);
      // The camera's right is (cos t, 0, -sin t) on the floor; its up has a share sin(phi) of the world's.
      const side = ((e.minX + e.maxX) / 2) * r * tanH;
      tx += side * Math.cos(home.theta);
      tz -= side * Math.sin(home.theta);
      ty += (((e.minY + e.maxY) / 2) * r * tanV) / Math.sin(home.phi);
    }
    home.r = r;
    home.tx = tx;
    home.ty = ty;
    home.tz = tz;
  }

  requestRender() {
    const g = this.gl;
    if (!g || g.raf) return;
    g.raf = requestAnimationFrame((t) => {
      g.raf = 0;
      this.frame(t);
    });
  }

  pauseGL() {
    const g = this.gl;
    if (g && g.raf) {
      cancelAnimationFrame(g.raf);
      g.raf = 0;
    }
  }

  resumeGL() {
    this.sizeGL();
    this.requestRender();
  }

  frame(now) {
    const g = this.gl;
    if (!g) return;
    if (!this.card.isConnected) {
      this.dispose();
      return;
    }
    if (document.hidden || !g.visible || this.view !== "3d") return;
    // The entrance: the ground rises, unless the reader asked for no motion.
    let again = false;
    if (g.intro) {
      const k = Math.min(1, (now - g.intro) / 900);
      const e = 1 - Math.pow(1 - k, 3);
      g.mesh.scale.y = Math.max(0.02, e);
      if (k >= 1) g.intro = 0;
      else again = true;
    }
    const t0 = performance.now();
    this.draw();
    if (this.metrics) {
      this.metrics.push(performance.now() - t0);
      if (this.metrics.length > 600) this.metrics.shift();
    }
    this.frames = (this.frames || 0) + 1;
    if (again) this.requestRender();
    if (this.benchRun) this.benchRun(now);
  }

  // One draw of the scene from where the camera is.
  draw() {
    const g = this.gl;
    const o = this.orbit;
    const cam = g.camera;
    const sp = Math.sin(o.phi);
    cam.position.set(o.tx + o.r * sp * Math.sin(o.theta), o.ty + o.r * Math.cos(o.phi), o.tz + o.r * sp * Math.cos(o.theta));
    cam.lookAt(o.tx, o.ty, o.tz);
    cam.updateMatrixWorld();
    g.renderer.render(g.scene, cam);
    this.placeLabels();
  }

  /** For the gate: draw now, then read back what is on the canvas (a sample of it, hashed). */
  probe() {
    const g = this.gl;
    if (!g || !g.mesh) return null;
    this.draw();
    const gl = g.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const px = new Uint8Array(4 * w * h);
    gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
    let lit = 0;
    let hash = 7;
    for (let i = 0; i < px.length; i += 4 * 5) {
      if (px[i + 3] > 0) lit++;
      hash = (Math.imul(hash, 31) + px[i] + px[i + 1] * 3 + px[i + 2] * 5 + px[i + 3] * 7) >>> 0;
    }
    // Lit pixels in the outermost two rows and columns of the canvas: a mesh the camera has cut off.
    let edgeLit = 0;
    const litAt = (x, y) => px[(y * w + x) * 4 + 3] > 0;
    for (let y = 0; y < h; y++) {
      if (y < 2 || y >= h - 2) {
        for (let x = 0; x < w; x++) if (litAt(x, y)) edgeLit++;
      } else {
        for (const x of [0, 1, w - 2, w - 1]) if (litAt(x, y)) edgeLit++;
      }
    }
    return {
      width: w,
      height: h,
      edgeLit,
      cssWidth: this.glCanvas.clientWidth,
      litShare: lit / (px.length / 20),
      hash,
      frames: this.frames || 0,
      orbit: { ...this.orbit },
      scaleY: g.mesh.scale.y,
      geometries: g.renderer.info.memory.geometries,
      cursor: g.cursor.visible,
    };
  }

  placeLabels() {
    const g = this.gl;
    if (!this.axisLabels || !g) return;
    const T = this.T;
    const w = this.glWrap.clientWidth;
    const h = this.glWrap.clientHeight;
    const v = new T.Vector3();
    for (const l of this.axisLabels) {
      v.set(l.x, 0, l.z).project(g.camera);
      const visible = v.z < 1 && Math.abs(v.x) < 1.05 && Math.abs(v.y) < 1.05;
      l.el.hidden = !visible;
      if (visible) l.el.style.setProperty("transform", `translate(${Math.round((v.x * 0.5 + 0.5) * w)}px, ${Math.round((-v.y * 0.5 + 0.5) * h)}px) translate(-50%, -50%)`);
    }
  }

  /** Orbit for `frames` frames and report how long they took (the gate measures with this). */
  bench(frames) {
    return new Promise((resolve) => {
      if (!this.gl || this.view !== "3d") return resolve(null);
      const stamps = [];
      let n = 0;
      this.benchRun = (now) => {
        stamps.push(now);
        if (n++ >= frames) {
          this.benchRun = null;
          const dts = stamps.slice(1).map((t, i) => t - stamps[i]).sort((a, b) => a - b);
          const cpu = this.metrics.slice(-frames).sort((a, b) => a - b);
          const q = (a, p) => a[Math.min(a.length - 1, Math.floor(a.length * p))];
          resolve({ frames: dts.length, intervalMedian: q(dts, 0.5), intervalP95: q(dts, 0.95), cpuMedian: q(cpu, 0.5), cpuP95: q(cpu, 0.95) });
          return;
        }
        this.orbit.theta += 0.02;
        this.requestRender();
      };
      this.orbit.theta += 0.02;
      this.requestRender();
    });
  }

  disposeGL() {
    const g = this.gl;
    if (!g) return;
    if (g.raf) cancelAnimationFrame(g.raf);
    if (g.mesh) {
      g.mesh.geometry.dispose();
      g.mesh.material.dispose();
    }
    for (const o of [g.floor, g.cursor]) if (o) {
      o.geometry.dispose();
      o.material.dispose();
    }
    g.renderer.dispose();
    try {
      g.renderer.forceContextLoss();
    } catch (e) {
      /* a context already gone */
    }
    this.gl = null;
    this.field = null;
  }

  dispose() {
    this.disposeGL();
    this.card.__terrain = null;
  }
}

// Keep in step with static/vendor/three/three.module.min.js: the first eight hex digits of its
// SHA-256 (crates/birdnet-web/tests/vendored_three_is_pinned_and_small.rs fails when they differ).
const THREE_HASH = "e9e6fbbd";

function boot() {
  document.querySelectorAll(".tr-card").forEach((card) => {
    if (card.__terrain) return;
    card.__terrain = new Terrain(card);
  });
}

if (typeof document !== "undefined") {
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", boot);
  else boot();
  document.addEventListener("htmx:afterSwap", boot);
}
