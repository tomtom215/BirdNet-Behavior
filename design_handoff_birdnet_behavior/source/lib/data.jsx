// Sample data for BirdNet-Behavior mockups.
// Three demo states: 'quiet' (sparse afternoon), 'busy' (mid-morning), 'dawn' (dawn chorus peak).

const SPECIES = [
  { sci: "Cyanocitta cristata",     common: "Blue Jay",            short: "BLJA", color: "oklch(58% 0.12 240)", count: 142, conf: 0.94, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/2/2e/Blue_Jay-27527.jpg/640px-Blue_Jay-27527.jpg",
    trend: [2,3,5,4,7,8,12,18,14,11,9,7,5,4,6,5,4,3,3,2,2,1,1,0] },
  { sci: "Cardinalis cardinalis",   common: "Northern Cardinal",   short: "NOCA", color: "oklch(58% 0.18 25)",  count: 118, conf: 0.97, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/4/4d/Cardinalis_cardinalis_-Columbus%2C_Ohio%2C_USA-male-8_%281%29.jpg/640px-Cardinalis_cardinalis_-Columbus%2C_Ohio%2C_USA-male-8_%281%29.jpg",
    trend: [1,2,4,7,11,14,16,15,12,10,9,8,7,6,5,5,4,3,3,2,2,1,1,0] },
  { sci: "Turdus migratorius",      common: "American Robin",      short: "AMRO", color: "oklch(55% 0.14 50)",  count: 96,  conf: 0.92, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/b/b8/Turdus-migratorius-002.jpg/640px-Turdus-migratorius-002.jpg",
    trend: [0,1,2,5,9,12,14,12,10,8,7,6,5,4,4,3,3,2,2,1,1,1,0,0] },
  { sci: "Poecile atricapillus",    common: "Black-capped Chickadee", short:"BCCH", color: "oklch(45% 0.02 80)", count: 84,  conf: 0.91, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/0/00/Poecile-atricapillus-001.jpg/640px-Poecile-atricapillus-001.jpg",
    trend: [1,2,3,5,8,10,11,10,9,8,7,6,6,5,5,4,4,3,2,2,1,1,1,0] },
  { sci: "Zenaida macroura",        common: "Mourning Dove",       short: "MODO", color: "oklch(60% 0.04 30)",  count: 71,  conf: 0.89, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/8/80/Mourning_Dove_2006.jpg/640px-Mourning_Dove_2006.jpg",
    trend: [0,1,3,4,6,8,9,9,8,7,6,5,4,4,3,3,2,2,1,1,1,0,0,0] },
  { sci: "Spinus tristis",          common: "American Goldfinch",  short: "AMGO", color: "oklch(78% 0.16 95)",  count: 63,  conf: 0.93, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/8/8d/Carduelis-tristis-002.jpg/640px-Carduelis-tristis-002.jpg",
    trend: [0,1,2,3,5,7,8,8,7,6,5,5,4,4,3,3,2,2,1,1,1,0,0,0] },
  { sci: "Sitta carolinensis",      common: "White-breasted Nuthatch", short:"WBNU", color: "oklch(55% 0.06 250)", count: 51, conf: 0.90, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/9/9c/White-breasted_Nuthatch_RWD2.jpg/640px-White-breasted_Nuthatch_RWD2.jpg",
    trend: [0,1,2,3,4,6,7,7,6,5,5,4,4,3,3,3,2,2,1,1,1,0,0,0] },
  { sci: "Dryobates pubescens",     common: "Downy Woodpecker",    short: "DOWO", color: "oklch(40% 0.02 80)",  count: 44,  conf: 0.88, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/9/91/Picoides_pubescensAAP033CB.jpg/640px-Picoides_pubescensAAP033CB.jpg",
    trend: [0,0,1,2,3,4,5,5,5,4,4,3,3,3,2,2,2,1,1,1,1,0,0,0] },
  { sci: "Picoides villosus",       common: "Hairy Woodpecker",    short: "HAWO", color: "oklch(38% 0.03 80)",  count: 23,  conf: 0.86, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/6/6e/Picoides_villosus_CT4.jpg/640px-Picoides_villosus_CT4.jpg",
    trend: [0,0,1,1,2,3,3,3,3,2,2,2,2,2,1,1,1,1,1,0,0,0,0,0] },
  { sci: "Setophaga coronata",      common: "Yellow-rumped Warbler", short:"YRWA", color: "oklch(70% 0.16 95)",  count: 19, conf: 0.84, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/4/4b/Yellow-rumped_Warbler_%28Setophaga_coronata%29.jpg/640px-Yellow-rumped_Warbler_%28Setophaga_coronata%29.jpg",
    trend: [0,0,0,1,1,2,3,4,4,3,3,2,2,1,1,1,1,0,0,0,0,0,0,0] },
  { sci: "Setophaga magnolia",      common: "Magnolia Warbler",    short: "MAWA", color: "oklch(72% 0.16 85)",  count: 11, conf: 0.83, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/9/9c/Magnolia_Warbler.jpg/640px-Magnolia_Warbler.jpg",
    trend: [0,0,0,0,1,1,2,2,2,2,2,1,1,1,1,0,0,0,0,0,0,0,0,0] },
  { sci: "Hylocichla mustelina",    common: "Wood Thrush",         short: "WOTH", color: "oklch(45% 0.10 40)",  count: 7,  conf: 0.79, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/8/83/Hylocichla-mustelina-001.jpg/640px-Hylocichla-mustelina-001.jpg",
    trend: [0,0,0,0,0,1,1,1,2,1,1,1,1,0,0,0,0,0,0,0,0,0,0,0] },
  { sci: "Pheucticus ludovicianus", common: "Rose-breasted Grosbeak", short:"RBGR", color:"oklch(50% 0.16 20)",  count: 4,  conf: 0.81, rare: true,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/8/88/Rose-breasted_Grosbeak.jpg/640px-Rose-breasted_Grosbeak.jpg",
    trend: [0,0,0,0,0,0,1,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0] },
  { sci: "Bombycilla cedrorum",     common: "Cedar Waxwing",       short: "CEDW", color: "oklch(62% 0.08 70)",  count: 3,  conf: 0.86, rare: false,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/c/cb/Cedar_Waxwing-Yu-1.jpg/640px-Cedar_Waxwing-Yu-1.jpg",
    trend: [0,0,0,0,0,0,0,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0] },
  { sci: "Strix varia",             common: "Barred Owl",          short: "BADO", color: "oklch(40% 0.04 60)",  count: 2,  conf: 0.93, rare: true,
    photo: "https://upload.wikimedia.org/wikipedia/commons/thumb/c/c1/Barred_Owl_%28Strix_varia%29_RWD.jpg/640px-Barred_Owl_%28Strix_varia%29_RWD.jpg",
    trend: [1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1] },
];

// 24×7 activity heatmap (rows = Sun..Sat, cols = 0..23 hour)
// Encoded as 0..5 intensity buckets. Dawn (5–8) + dusk (17–19) peak.
function genHeatmap(seed = 1) {
  const days = 7, hours = 24;
  const m = [];
  let s = seed;
  const rand = () => { s = (s * 9301 + 49297) % 233280; return s / 233280; };
  for (let d = 0; d < days; d++) {
    const row = [];
    for (let h = 0; h < hours; h++) {
      // baseline circadian curve
      let v = 0;
      if (h >= 5 && h <= 8)       v = 4 + rand() * 1.2;   // dawn chorus
      else if (h >= 9 && h <= 11) v = 3 + rand() * 1.0;
      else if (h >= 12 && h <= 15)v = 1.6 + rand() * 1.0;
      else if (h >= 16 && h <= 19)v = 3 + rand() * 1.4;   // evening
      else if (h >= 20 && h <= 22)v = 1 + rand() * 0.8;
      else                        v = rand() * 0.6;       // night
      // weekend variation
      if (d === 0 || d === 6) v *= 0.9 + rand() * 0.2;
      row.push(Math.min(5, Math.round(v)));
    }
    m.push(row);
  }
  return m;
}

const HEATMAP = genHeatmap(7);
const DAY_LABELS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

// Live detection feed — seed messages, the dashboard ticker recycles these.
const FEED_SEED = [
  { sp: 0, conf: 0.96, ago: "just now", lat: 1.2 },
  { sp: 3, conf: 0.91, ago: "12s ago",  lat: 1.4 },
  { sp: 1, conf: 0.98, ago: "27s ago",  lat: 1.1 },
  { sp: 0, conf: 0.88, ago: "41s ago",  lat: 2.0 },
  { sp: 2, conf: 0.94, ago: "58s ago",  lat: 1.3 },
  { sp: 5, conf: 0.93, ago: "1m 24s",   lat: 1.6 },
  { sp: 12, conf: 0.81, ago: "2m 09s",  lat: 1.0, rare: true },
  { sp: 7, conf: 0.86, ago: "2m 41s",   lat: 1.5 },
  { sp: 1, conf: 0.95, ago: "3m 12s",   lat: 1.2 },
  { sp: 6, conf: 0.89, ago: "3m 47s",   lat: 1.4 },
  { sp: 4, conf: 0.84, ago: "4m 03s",   lat: 1.3 },
  { sp: 9, conf: 0.79, ago: "4m 38s",   lat: 1.1 },
];

// Co-occurrence matrix — symmetric pearson-like correlation 0..1
const COOC_SPECIES = [0, 1, 2, 3, 4, 5, 6, 7]; // indexes into SPECIES
const COOC = (() => {
  const n = COOC_SPECIES.length;
  const m = Array.from({ length: n }, () => new Array(n).fill(0));
  // hand-tuned: similar feeder birds correlate; woodpeckers correlate; chickadee/nuthatch high
  const pairs = [
    [0,1,0.62],[0,3,0.71],[0,6,0.55],[1,2,0.48],[1,3,0.66],[1,5,0.59],
    [2,4,0.51],[3,6,0.78],[3,5,0.52],[5,6,0.41],[7,3,0.44],[7,6,0.49],
    [0,5,0.38],[1,6,0.46],[2,5,0.32],[4,5,0.28],[0,2,0.41],[0,4,0.22],
  ];
  for (const [a,b,v] of pairs) { m[a][b] = v; m[b][a] = v; }
  for (let i = 0; i < n; i++) m[i][i] = 1.0;
  return m;
})();

// Migration phenology — weekly abundance for migratory species (weeks 1..52)
const MIGRATION = [
  // sp index, label, weekly counts (52 wk)
  { sp: 9,  curve: weekCurve(18, 6, 0.6) },   // YRWA spring + fall peaks
  { sp: 10, curve: weekCurve(20, 4, 1.0) },   // MAWA spring only
  { sp: 11, curve: weekCurve(22, 6, 0.4) },   // WOTH summer resident
  { sp: 12, curve: weekCurve(19, 3, 1.0) },   // RBGR sharp spring
  { sp: 13, curve: weekCurve(32, 8, 0.7) },   // CEDW fall
];
function weekCurve(peakWeek, sigma, springFallRatio) {
  const arr = [];
  for (let w = 0; w < 52; w++) {
    const a = Math.exp(-Math.pow((w - peakWeek), 2) / (2 * sigma * sigma));
    const b = springFallRatio < 1
      ? Math.exp(-Math.pow((w - (peakWeek + 16)), 2) / (2 * (sigma + 1) * (sigma + 1))) * (1 - springFallRatio)
      : 0;
    arr.push(Math.max(0, a + b));
  }
  return arr;
}

// Dawn-chorus circadian: per-species activity by hour (24)
function chorusByHour(peak, width, amp = 1) {
  return Array.from({ length: 24 }, (_, h) => {
    const d = Math.min(Math.abs(h - peak), 24 - Math.abs(h - peak));
    return Math.max(0, amp * Math.exp(-(d * d) / (2 * width * width)));
  });
}
const CHORUS = [
  { sp: 1, hours: chorusByHour(6.0, 1.6, 1.00) }, // Cardinal — dawn
  { sp: 3, hours: chorusByHour(6.6, 1.8, 0.92) }, // Chickadee
  { sp: 0, hours: chorusByHour(7.2, 2.0, 0.86) }, // Blue Jay
  { sp: 2, hours: chorusByHour(7.6, 2.6, 0.80) }, // Robin (longer)
  { sp: 5, hours: chorusByHour(8.4, 2.6, 0.62) }, // Goldfinch
  { sp: 6, hours: chorusByHour(9.0, 3.0, 0.55) }, // Nuthatch
  { sp: 14, hours: chorusByHour(2.0, 1.4, 0.45) },// Barred Owl — nocturnal
];

const LIFE_LIST = [
  { sp: 1,  first: "2024-03-12", note: "First detection — set up the Pi this morning." },
  { sp: 0,  first: "2024-03-12", note: "Loud and unmistakable from the back yard." },
  { sp: 2,  first: "2024-03-15", note: "Heard chorus 5 minutes after sunrise." },
  { sp: 3,  first: "2024-03-18", note: "Chickadees nesting in the old oak." },
  { sp: 5,  first: "2024-04-02", note: "" },
  { sp: 6,  first: "2024-04-09", note: "" },
  { sp: 4,  first: "2024-04-14", note: "" },
  { sp: 9,  first: "2024-04-22", note: "Spring migration arrived." },
  { sp: 7,  first: "2024-05-01", note: "" },
  { sp: 10, first: "2024-05-04", note: "Rare locally — confirmed by manual review." },
  { sp: 12, first: "2024-05-08", note: "Single visit, never again." },
  { sp: 11, first: "2024-05-15", note: "Singing every evening through June." },
  { sp: 8,  first: "2024-09-03", note: "" },
  { sp: 14, first: "2024-10-19", note: "After dusk, low frequency, very clear." },
  { sp: 13, first: "2024-11-22", note: "Fall foraging flock." },
];

window.BNB = window.BNB || {};
Object.assign(window.BNB, {
  SPECIES, HEATMAP, DAY_LABELS, FEED_SEED, COOC, COOC_SPECIES, MIGRATION, CHORUS, LIFE_LIST,
});
