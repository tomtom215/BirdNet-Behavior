// Synthetic stations for the discovery timeline. Pure (no DOM), so the node test and the browser page share it.
// Today is fixed at 2026-10-31 so every screenshot and test is reproducible.

export const TODAY = "2026-10-31";

function rng(seed) {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const DAY = 86400000;
const dayOf = (iso) => Math.round(Date.parse(iso + "T00:00:00Z") / DAY);
const isoOf = (d) => new Date(d * DAY).toISOString().slice(0, 10);

// Real-ish birds: [name, scientific, 4-letter code]
const BIRDS = [
  ["Northern Cardinal", "Cardinalis cardinalis", "NOCA"],
  ["Carolina Chickadee", "Poecile carolinensis", "CCHI"],
  ["Tufted Titmouse", "Baeolophus bicolor", "TUTI"],
  ["Blue Jay", "Cyanocitta cristata", "BLJA"],
  ["Mourning Dove", "Zenaida macroura", "MODO"],
  ["Carolina Wren", "Thryothorus ludovicianus", "CARW"],
  ["Downy Woodpecker", "Dryobates pubescens", "DOWO"],
  ["American Robin", "Turdus migratorius", "AMRO"],
  ["Dark-eyed Junco", "Junco hyemalis", "DEJU"],
  ["White-throated Sparrow", "Zonotrichia albicollis", "WTSP"],
  ["Yellow-rumped Warbler", "Setophaga coronata", "YRWA"],
  ["Red-winged Blackbird", "Agelaius phoeniceus", "RWBL"],
  ["Ruby-throated Hummingbird", "Archilochus colubris", "RTHU"],
  ["Great Horned Owl", "Bubo virginianus", "GHOW"],
  ["Painted Bunting", "Passerina ciris", "PABU"],
  ["Eastern Bluebird", "Sialia sialis", "EABL"],
  ["Barred Owl", "Strix varia", "BAOW"],
  ["Pileated Woodpecker", "Dryocopus pileatus", "PIWO"],
  ["Cedar Waxwing", "Bombycilla cedrorum", "CEDW"],
  ["Indigo Bunting", "Passerina cyanea", "INBU"],
  ["Scarlet Tanager", "Piranga olivacea", "SCTA"],
  ["Eastern Towhee", "Pipilo erythrophthalmus", "EATO"],
  ["Gray Catbird", "Dumetella carolinensis", "GRCA"],
  ["Brown Thrasher", "Toxostoma rufum", "BRTH"],
];

/** Image URLs for a species: some real pictures, some that 404, some none (a deterministic mix). */
function imgFor(i, mode = "mix") {
  if (mode === "none") return null;
  if (mode === "missing") return `/img/missing/sp${i}.png`;
  const m = i % 7;
  if (m === 3) return `/img/missing/sp${i}.png`; // 404
  if (m === 5) return null; // no picture at all
  if (m === 6) return "/static/icon-192.png";
  return `/img/bird${i % 8}.png`;
}

function sp(i, first, count, extra = {}) {
  const [name, sci, code] = BIRDS[i % BIRDS.length];
  return { name, sci, code, first, count, img: imgFor(i), ...extra };
}

/** The station that began a year ago: 6 on day one, a thinning trickle, a plateau, a late burst, one today. */
export function year() {
  const rows = [
    [0, "2025-11-02", 8421],
    [1, "2025-11-02", 6310],
    [2, "2025-11-02", 3120],
    [3, "2025-11-02", 2204],
    [4, "2025-11-02", 5127],
    [5, "2025-11-02", 4760],
    [6, "2025-11-09", 612],
    [7, "2025-11-16", 1304],
    [8, "2025-11-28", 2290],
    [9, "2025-12-07", 1481],
    [10, "2025-12-21", 733],
    [11, "2026-01-14", 905],
    [12, "2026-02-11", 66],
    [13, "2026-03-13", 412],
    [14, "2026-04-06", 47],
    [15, "2026-05-17", 12],
    [16, "2026-10-12", 9],
    [17, "2026-10-14", 31],
    [18, "2026-10-17", 5],
    [19, "2026-10-31", 1],
  ];
  // Swap the last two names so the newest is the Painted Bunting of the brief.
  const s = rows.map(([i, d, c]) => sp(i, d, c));
  s[19] = { ...sp(14, "2026-10-31", 1) };
  s[14] = { ...sp(20, "2026-04-06", 47) };
  return { from: "2025-11-02", to: TODAY, species: s.map((x, k) => ({ ...x, key: x.name })) };
}

export function two() {
  return {
    from: "2026-08-20",
    to: TODAY,
    species: [sp(0, "2026-08-20", 310), { ...sp(14, "2026-10-02", 18) }],
  };
}

export function one() {
  return { from: "2026-10-31", to: TODAY, species: [sp(14, "2026-10-31", 1)] };
}

export function oneOld() {
  return { from: "2026-03-02", to: TODAY, species: [sp(0, "2026-03-02", 5421)] };
}

/** 80 species over three years: a fast start that thins out, a few autumn bursts. */
export function big80() {
  const r = rng(11);
  const from = dayOf("2023-11-01");
  const to = dayOf(TODAY);
  const out = [];
  const names = new Set();
  for (let k = 0; k < 80; k++) {
    // First-heard day: strongly front-loaded, with extra weight on the autumn migration months.
    let u = Math.pow(r(), 2.3);
    let d = from + Math.floor(u * (to - from));
    const m = new Date(d * DAY).getUTCMonth();
    if (r() < 0.25 && m > 1 && m < 8) d += Math.floor((9 - m) * 30 * r() * 0.3) * (r() < 0.5 ? 1 : 0);
    d = Math.min(to, Math.max(from, d));
    const base = BIRDS[k % BIRDS.length];
    let name = base[0];
    if (k >= BIRDS.length) name = `${["Eastern", "Western", "Northern", "Lesser", "Great"][k % 5]} ${base[0].split(" ").slice(-1)[0]} ${Math.floor(k / BIRDS.length)}`;
    if (names.has(name)) name += ` ${k}`;
    names.add(name);
    out.push({
      name,
      sci: base[1],
      code: base[2].replace(/.$/, String(k % 10)).slice(0, 4),
      first: isoOf(d),
      count: Math.max(1, Math.round(Math.pow(r(), 3) * 40000)),
      img: imgFor(k),
    });
  }
  // The very first day is busy.
  for (let k = 0; k < 7; k++) out[k].first = isoOf(from);
  out[79].first = TODAY;
  return { from: isoOf(from), to: TODAY, species: out };
}

/** 40-character names and counts in the millions. */
export function long() {
  const base = year();
  const names = [
    "Black-crowned Night-Heron (hybrid, juvenile)", // 43
    "Ruby-throated Hummingbird (immature male)",
    "Yellow-bellied Sapsucker (southern race 1)",
    "Eastern Whip-poor-will (nocturnal visitor)",
  ];
  const s = base.species.slice();
  s[0] = { ...s[0], name: names[0], count: 1600000 };
  s[1] = { ...s[1], name: names[1], count: 1000000 };
  s[2] = { ...s[2], name: "Ab".repeat(20) /* 40 chars, no spaces */ };
  s[19] = { ...s[19], name: names[3], count: 1 };
  s[17] = { ...s[17], name: names[2] };
  return { from: base.from, to: base.to, species: s };
}

/** A month with twelve firsts (twelve different days), on top of a quiet year. */
export function month12() {
  const s = [];
  const early = [
    [0, "2025-11-02", 5000],
    [1, "2025-11-02", 4400],
    [2, "2025-11-30", 900],
  ];
  early.forEach(([i, d, c]) => s.push(sp(i, d, c)));
  const days = [1, 2, 4, 6, 7, 9, 12, 15, 18, 21, 24, 28];
  days.forEach((d, k) => s.push(sp(3 + k, `2026-05-${String(d).padStart(2, "0")}`, 20 + k * 7)));
  s.push(sp(16, "2026-09-11", 4));
  return { from: "2025-11-02", to: TODAY, species: s };
}

/** A migrated station: dozens of species all "first heard" on the day of the import, then a few. */
export function bulk(n = 40) {
  const r = rng(5);
  const s = [];
  for (let k = 0; k < n; k++) {
    const base = BIRDS[k % BIRDS.length];
    s.push({ name: k < BIRDS.length ? base[0] : `${base[0]} ${k}`, sci: base[1], code: base[2], first: "2024-02-14", count: Math.round(r() * 9000) + 1, img: imgFor(k) });
  }
  s.push(sp(14, "2026-04-11", 30), sp(15, "2026-09-02", 5), sp(16, TODAY, 1));
  return { from: "2024-02-14", to: TODAY, species: s };
}

/** A stress case: n species across several years. */
export function stress(n, years = 5, seed = 3) {
  const r = rng(seed);
  const from = dayOf(TODAY) - years * 365;
  const s = [];
  for (let k = 0; k < n; k++) {
    const base = BIRDS[k % BIRDS.length];
    const d = from + Math.floor(Math.pow(r(), 1.8) * (years * 365));
    s.push({ name: `${base[0]} ${k}`, sci: base[1], code: base[2], first: isoOf(Math.min(dayOf(TODAY), d)), count: Math.round(r() * 100000) + 1, img: imgFor(k) });
  }
  s[0].first = isoOf(from);
  return { from: isoOf(from), to: TODAY, species: s };
}

export const CASES = {
  year,
  two,
  one,
  oneOld,
  big80,
  long,
  month12,
  bulk,
  stress400: () => stress(400),
  stress2000: () => stress(2000, 8),
};

/** Server-shaped additions: `key` is the stored name, `img` carries the species image route. */
export function withKeys(data) {
  return { ...data, species: data.species.map((x) => ({ key: x.key ?? x.name, ...x })) };
}
