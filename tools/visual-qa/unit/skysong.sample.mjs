// Synthetic feeds for the sky-song harness: the shape /pages/today-songs will serve.
// Deterministic (seeded), so a screenshot is repeatable. Used by the node test and by the page.

function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const gauss = (r) => {
  let u = 0;
  let v = 0;
  while (u === 0) u = r();
  while (v === 0) v = r();
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v);
};

/** A species: where in the day it sings (a mixture of gaussians in hours) and how sure the station is of it. */
export const BIRDS = {
  "Blue Jay": { when: [[7.4, 1.6, 0.55], [12.6, 1.8, 0.45]], sure: [0.93, 0.045], top: true },
  "Northern Cardinal": { when: [[6.5, 0.7, 0.7], [13.0, 2.0, 0.2], [16.4, 0.5, 0.1]], sure: [0.9, 0.06], top: true },
  "Black-capped Chickadee": { when: [[10.5, 2.8, 1]], sure: [0.82, 0.07] },
  "American Goldfinch": { when: [[12.2, 1.4, 1]], sure: [0.86, 0.06] },
  "American Robin": { when: [[6.2, 0.55, 0.65], [16.6, 0.6, 0.35]], sure: [0.84, 0.07] },
  "Song Sparrow": { when: [[6.6, 0.8, 0.8], [11.5, 1.4, 0.2]], sure: [0.8, 0.07] },
  "Carolina Wren": { when: [[6.4, 0.6, 0.55], [16.8, 0.8, 0.45]], sure: [0.88, 0.06], top: true },
  "Great Horned Owl": { when: [[1.6, 1.4, 0.55], [22.4, 1.2, 0.45]], sure: [0.8, 0.07] },
  "Mourning Dove": { when: [[8.2, 1.6, 1]], sure: [0.77, 0.05] },
  "Downy Woodpecker": { when: [[9.5, 1.6, 1]], sure: [0.81, 0.06] },
  "Barred Owl": { when: [[3.2, 1.0, 0.6], [21.0, 1.0, 0.4]], sure: [0.76, 0.05] },
  "Tufted Titmouse": { when: [[8.0, 1.8, 1]], sure: [0.79, 0.06] },
  "White-throated Sparrow": { when: [[7.0, 1.0, 1]], sure: [0.75, 0.04] },
  "Hermit Thrush": { when: [[6.0, 0.5, 0.6], [17.0, 0.5, 0.4]], sure: [0.78, 0.06] },
  "Eastern Phoebe": { when: [[7.8, 1.5, 1]], sure: [0.8, 0.06] },
};

const pad = (n) => String(n).padStart(2, "0");
const hms = (t) => `${pad(Math.floor(t / 3600))}:${pad(Math.floor((t % 3600) / 60))}:${pad(Math.floor(t % 60))}`;

/**
 * One feed. `named` is [[name, count], ...] for slots 1..8 (in that order), `other` is
 * [[name, count], ...] folded into "Other species". `nowSec` null = a past day. Songs after `nowSec` are not made.
 */
export function makeDay({ seed = 1, date = "2026-10-31", nowSec = 50820, sun = { rise: 6.8, set: 16.6, dawn: 6.2, dusk: 17.2 }, floor = 0.7, named, other = [], names = null, extra = null, speciesTotal = null, totalOverride = null, perSpeciesN = null }) {
  const r = rng(seed);
  const species = named.map(([name], i) => ({ name: names && names[name] ? names[name] : name, slot: i + 1 }));
  if (other.length) species.push({ name: "Other species", slot: 0 });
  const songs = [];
  const make = (bird, count, si) => {
    const spec = BIRDS[bird] || { when: [[10, 4, 1]], sure: [0.82, 0.07] };
    let made = 0;
    let guard = 0;
    while (made < count && guard++ < count * 400) {
      const pick = r();
      let acc = 0;
      let comp = spec.when[spec.when.length - 1];
      for (const w of spec.when) {
        acc += w[2];
        if (pick <= acc) {
          comp = w;
          break;
        }
      }
      let h = comp[0] + gauss(r) * comp[1];
      if (h < 0 || h >= 24) continue;
      const t = Math.floor(h * 3600);
      if (nowSec !== null && t > nowSec) continue;
      // BirdNET piles up near the top for a species it knows well: a half-normal tail from 1 downward
      // for those, a plain bell for the rest. Nothing below the cut-off is ever listed.
      let c = spec.top ? 1 - Math.abs(gauss(r)) * (1 - spec.sure[0]) * 1.6 : spec.sure[0] + gauss(r) * spec.sure[1];
      let tries = 0;
      while (c < floor + 0.002 && tries++ < 20) c = spec.top ? 1 - Math.abs(gauss(r)) * (1 - spec.sure[0]) * 1.6 : spec.sure[0] + gauss(r) * spec.sure[1];
      if (c < floor + 0.002) c = floor + r() * 0.03;
      c = Math.min(0.9995, c);
      c = Math.round(c * 10000) / 10000;
      const name = species[si].name;
      songs.push({ t, c, s: si, href: `/detections/detail?date=${date}&time=${hms(t)}&name=${encodeURIComponent(name)}` });
      made += 1;
    }
  };
  named.forEach(([bird, count], i) => make(bird, count, i));
  if (other.length) {
    const oi = species.length - 1;
    for (const [bird, count] of other) make(bird, count, oi);
  }
  if (extra) for (const e of extra) songs.push({ ...e, href: e.href || `/detections/detail?date=${date}&time=${hms(e.t)}&name=x` });
  // The server lists them in any order.
  for (let i = songs.length - 1; i > 0; i--) {
    const j = Math.floor(r() * (i + 1));
    [songs[i], songs[j]] = [songs[j], songs[i]];
  }
  const out = { date, nowSec, sun, floor, species, songs };
  if (speciesTotal) out.speciesTotal = speciesTotal;
  if (totalOverride) out.total = totalOverride;
  if (perSpeciesN) perSpeciesN.forEach((n, i) => { if (species[i]) species[i].n = n; });
  return out;
}

const eight = ["Blue Jay", "Northern Cardinal", "Black-capped Chickadee", "American Goldfinch", "American Robin", "Song Sparrow", "Carolina Wren", "Great Horned Owl"];

export const CASES = {
  // The reference day: today, a little after two in the afternoon. 49 songs, 8 named species + other.
  day49: () =>
    makeDay({
      seed: 7,
      nowSec: 14 * 3600 + 7 * 60,
      named: [["Blue Jay", 13], ["Northern Cardinal", 11], ["Black-capped Chickadee", 5], ["American Goldfinch", 4], ["American Robin", 4], ["Song Sparrow", 3], ["Carolina Wren", 3], ["Great Horned Owl", 2]],
      other: [["Mourning Dove", 2], ["Downy Woodpecker", 1], ["Barred Owl", 1]],
      speciesTotal: 11,
    }),
  // A full past day, so the sentence ends at night and the sun sits at noon.
  past: () =>
    makeDay({
      seed: 11,
      date: "2026-10-30",
      nowSec: null,
      sun: { rise: 6.78, set: 16.62, dawn: null, dusk: null },
      named: [["Blue Jay", 44], ["Northern Cardinal", 52], ["Black-capped Chickadee", 31], ["American Goldfinch", 22], ["American Robin", 28], ["Song Sparrow", 17], ["Carolina Wren", 15], ["Great Horned Owl", 9]],
      other: [["Mourning Dove", 12], ["Downy Woodpecker", 8], ["Barred Owl", 5], ["Tufted Titmouse", 7], ["Hermit Thrush", 4]],
    }),
  day400: () =>
    makeDay({
      seed: 21,
      nowSec: 19 * 3600 + 32 * 60,
      named: [["Blue Jay", 96], ["Northern Cardinal", 88], ["Black-capped Chickadee", 54], ["American Goldfinch", 30], ["American Robin", 44], ["Song Sparrow", 28], ["Carolina Wren", 22], ["Great Horned Owl", 14]],
      other: [["Mourning Dove", 12], ["Downy Woodpecker", 8], ["Barred Owl", 6], ["Tufted Titmouse", 10], ["Hermit Thrush", 4], ["White-throated Sparrow", 6]],
    }),
  // The stress day: 1,800 songs, a 40-character name, a chip count of 1,600,000.
  day1800: () => {
    const d = makeDay({
      seed: 33,
      nowSec: 23 * 3600 + 41 * 60,
      named: [["Blue Jay", 520], ["Northern Cardinal", 430], ["Black-capped Chickadee", 260], ["American Goldfinch", 150], ["American Robin", 190], ["Song Sparrow", 100], ["Carolina Wren", 70], ["Great Horned Owl", 30]],
      other: [["Mourning Dove", 20], ["Downy Woodpecker", 10], ["Barred Owl", 8], ["Tufted Titmouse", 6], ["Hermit Thrush", 6]],
      names: { "American Goldfinch": "Eastern Yellow-bellied Sapsucker-Warbler Hybrid" },
    });
    d.species[0].n = 1600000;
    d.total = 1600000 + 1700;
    d.speciesTotal = 27;
    return d;
  },
  // No location: the station has no sun to draw.
  nosun: () =>
    makeDay({
      seed: 5,
      nowSec: 15 * 3600 + 20 * 60,
      sun: null,
      named: [["Blue Jay", 30], ["Northern Cardinal", 28], ["Black-capped Chickadee", 20], ["American Robin", 18], ["Song Sparrow", 12], ["Carolina Wren", 8], ["Great Horned Owl", 4]],
      other: [["Mourning Dove", 4]],
    }),
  one: () => makeDay({ seed: 2, nowSec: 9 * 3600 + 30 * 60, named: [["Northern Cardinal", 1]] }),
  two: () => makeDay({ seed: 3, nowSec: 12 * 3600, named: [["Blue Jay", 1], ["Great Horned Owl", 1]], extra: [] }),
  empty: () => makeDay({ seed: 4, nowSec: 4 * 3600, named: [] }),
  // Songs that land on the very same spot, to see the fan-out.
  stacked: () =>
    makeDay({
      seed: 8,
      nowSec: 20 * 3600,
      named: [["Blue Jay", 6], ["Northern Cardinal", 6]],
      extra: [
        { t: 29000, c: 0.91, s: 0 }, { t: 29010, c: 0.91, s: 0 }, { t: 29020, c: 0.91, s: 1 }, { t: 29030, c: 0.911, s: 1 }, { t: 29040, c: 0.91, s: 0 },
        { t: 45000, c: 0.7, s: 1 }, { t: 45020, c: 0.7, s: 1 }, { t: 45040, c: 0.7, s: 0 },
      ],
    }),
  // Edge of the day: a song just after midnight and just before it ends, a winter sun at a high latitude.
  winter: () =>
    makeDay({
      seed: 9,
      date: "2026-12-21",
      nowSec: null,
      sun: { rise: 8.9, set: 15.1, dawn: 8.1, dusk: 15.9 },
      floor: 0.5,
      named: [["Blue Jay", 18], ["Black-capped Chickadee", 25], ["Great Horned Owl", 6]],
      extra: [{ t: 12, c: 0.62, s: 2 }, { t: 86395, c: 0.99, s: 2 }],
    }),
};

export function caseJson(name) {
  const f = CASES[name];
  return f ? f() : null;
}
