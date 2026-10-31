// Sample data for the harness: realistic years and the edge cases. A classic script (no module),
// so the page can mount a host synchronously before the module boots. Also runs under node
// (`import`-free: it sets globalThis.SSN_CASES) so the test can use the same cases.
(function () {
  // A small deterministic generator so the "eight" and "stress" cases are the same every run.
  function rng(seed) {
    let s = seed >>> 0;
    return function () {
      s = (s + 0x6d2b79f5) >>> 0;
      let t = s;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  // Counts for January..October (the running month is October, so far); November and December have not happened.
  const NULLS = [null, null];
  const sofar = (a) => a.concat(NULLS);

  const year = {
    year: 2026,
    thisMonth: 9,
    species: [
      { name: "Northern Cardinal", slot: 1, counts: sofar([120, 131, 142, 150, 118, 96, 88, 84, 101, 98]) }, // a resident that leads the cold half of the year
      { name: "Blue Jay", slot: 2, counts: sofar([64, 58, 71, 90, 102, 86, 74, 112, 124, 132]) }, // climbs through the autumn and takes the lead in August
      { name: "Indigo Bunting", slot: 3, counts: sofar([0, 0, 0, 40, 210, 188, 96, 0, 0, 0]) }, // a spring migrant that surges May to June and is gone by August
      { name: "Carolina Wren", slot: 4, counts: sofar([52, 60, 74, 88, 96, 104, 112, 80, 66, 58]) }, // overtakes the titmouse in March, leads July
      { name: "Dark-eyed Junco", slot: 5, counts: sofar([108, 118, 96, 22, 0, 0, 0, 0, 0, 31]) }, // the winter bird: gone all summer, back in October
      { name: "Tufted Titmouse", slot: 6, counts: sofar([80, 76, 70, 66, 64, 62, 60, 58, 54, 48]) }, // slides down the order
      { name: "White-throated Sparrow", slot: 7, counts: sofar([0, 0, 14, 52, 28, 0, 0, 0, 40, 96]) }, // a passage migrant: April and again from September
      { name: "Baltimore Oriole", slot: 8, counts: sofar([0, 0, 0, 0, 76, 94, 52, 18, 0, 0]) }, // absent for months at both ends
    ],
  };

  // Two birds, a full year (December has happened): the year is complete, thisMonth is null.
  const two = {
    year: 2025,
    thisMonth: null,
    species: [
      { name: "House Sparrow", slot: 1, counts: [210, 190, 220, 260, 300, 280, 240, 230, 250, 270, 240, 220] },
      { name: "European Starling", slot: 2, counts: [180, 200, 210, 240, 310, 260, 250, 190, 260, 250, 260, 190] },
    ],
  };

  // Eight birds, a complete year, plenty of crossings.
  const r = rng(7);
  const names8 = ["Song Sparrow", "American Robin", "Mourning Dove", "Black-capped Chickadee", "American Goldfinch", "Downy Woodpecker", "House Finch", "Red-winged Blackbird"];
  const eight = {
    year: 2025,
    thisMonth: null,
    species: names8.map((name, i) => {
      const base = 220 - i * 18;
      const phase = r() * 12;
      return {
        name,
        slot: i + 1,
        counts: Array.from({ length: 12 }, (_, m) => {
          const wave = Math.sin(((m + phase) / 12) * Math.PI * 2 * (1 + (i % 3) * 0.5));
          const v = Math.round(base + wave * (90 - i * 6) + (r() - 0.5) * 40);
          return v < 30 && i > 4 && r() > 0.5 ? 0 : Math.max(0, v);
        }),
      };
    }),
  };

  // Three months so far: January, February and March (March is the running month).
  const early = {
    year: 2026,
    thisMonth: 2,
    species: [
      { name: "Dark-eyed Junco", slot: 1, counts: [96, 110, 74].concat(new Array(9).fill(null)) },
      { name: "Northern Cardinal", slot: 2, counts: [88, 92, 80].concat(new Array(9).fill(null)) },
      { name: "American Tree Sparrow", slot: 3, counts: [40, 22, 12].concat(new Array(9).fill(null)) },
      { name: "Black-capped Chickadee", slot: 4, counts: [52, 58, 66].concat(new Array(9).fill(null)) },
      { name: "Red-breasted Nuthatch", slot: 5, counts: [18, 0, 30].concat(new Array(9).fill(null)) },
    ],
  };

  // Only January has happened.
  const jan = {
    year: 2026,
    thisMonth: 0,
    species: [
      { name: "Black-capped Chickadee", slot: 1, counts: [64].concat(new Array(11).fill(null)) },
      { name: "Northern Cardinal", slot: 2, counts: [48].concat(new Array(11).fill(null)) },
      { name: "House Finch", slot: 3, counts: [48].concat(new Array(11).fill(null)) },
      { name: "Downy Woodpecker", slot: 4, counts: [9].concat(new Array(11).fill(null)) },
    ],
  };

  // Forty-character names (no spaces to break at in the first two), eight birds, six months.
  const long = {
    year: 2026,
    thisMonth: 5,
    species: [
      { name: "Black-and-white-Warbler-Long-Name-Subspe", slot: 1, counts: [20, 24, 60, 140, 220, 180, null, null, null, null, null, null] },
      { name: "Yellow-bellied Sapsucker (Eastern form) X", slot: 2, counts: [64, 70, 82, 74, 40, 52, null, null, null, null, null, null] },
      { name: "Northern Rough-winged Swallow Subspecies", slot: 3, counts: [0, 0, 8, 90, 160, 210, null, null, null, null, null, null] },
      { name: "Ruby-crowned Kinglet (Western race) 0123", slot: 4, counts: [30, 34, 55, 66, 20, 0, null, null, null, null, null, null] },
      { name: "Dark-eyed Junco (Slate-colored subspecie", slot: 5, counts: [110, 120, 90, 30, 0, 0, null, null, null, null, null, null] },
      { name: "American Goldfinch", slot: 6, counts: [40, 36, 44, 58, 70, 82, null, null, null, null, null, null] },
      { name: "Tufted Titmouse", slot: 7, counts: [58, 60, 58, 56, 62, 60, null, null, null, null, null, null] },
      { name: "Carolina Chickadee and Friends Extra Lon", slot: 8, counts: [12, 14, 18, 22, 26, 30, null, null, null, null, null, null] },
    ],
  };

  // Counts at the scale of a busy city station: 1.2 to 1.6 million a month.
  const r2 = rng(11);
  const huge = {
    year: 2026,
    thisMonth: 9,
    species: ["Northern Cardinal", "House Sparrow", "European Starling", "American Robin", "Mourning Dove", "Song Sparrow"].map((name, i) => ({
      name,
      slot: i + 1,
      counts: Array.from({ length: 12 }, (_, m) => (m > 9 ? null : Math.round(1612345 - i * 190000 + Math.sin(m * (0.5 + i * 0.2)) * 160000 + r2() * 90000))),
    })),
  };

  // Ties and zeros: equal counts, a month with nothing heard at all, a bird that is only ever heard once.
  const ties = {
    year: 2026,
    thisMonth: 5,
    species: [
      { name: "Blue Jay", slot: 1, counts: [50, 50, 0, 30, 40, 22, null, null, null, null, null, null] },
      { name: "Northern Cardinal", slot: 2, counts: [50, 20, 0, 30, 10, 22, null, null, null, null, null, null] },
      { name: "Carolina Wren", slot: 3, counts: [10, 20, 0, 30, 0, 1, null, null, null, null, null, null] },
      { name: "Gray Catbird", slot: 4, counts: [0, 0, 0, 0, 5, 9, null, null, null, null, null, null] },
    ],
  };

  // A single bird.
  const one = {
    year: 2026,
    thisMonth: 3,
    species: [{ name: "Northern Cardinal", slot: 1, counts: [40, 36, 52, 60, null, null, null, null, null, null, null, null] }],
  };

  const cases = { year, two, eight, early, jan, long, huge, ties, one };
  if (typeof globalThis !== "undefined") globalThis.SSN_CASES = cases;

  // In the browser: ?case=year&theme=dark&motion=reduced&w=390 mounts one host (or all, with ?case=all).
  if (typeof document !== "undefined") {
    const q = new URLSearchParams(location.search);
    const root = document.documentElement;
    if (q.get("theme")) root.setAttribute("data-theme", q.get("theme"));
    if (q.get("motion") === "reduced") root.setAttribute("data-motion", "reduced");
    if (q.get("contrast")) root.setAttribute("data-contrast", q.get("contrast"));
    const mount = document.getElementById("mount");
    if (mount) {
      const which = (q.get("case") || "year").split(",");
      const list = which[0] === "all" ? Object.keys(cases) : which;
      for (const key of list) {
        // Only the harness's own cases: `cases[key]` alone also accepts "constructor" and "__proto__".
        if (!Object.hasOwn(cases, key)) continue;
        const d = cases[key];
        const card = document.createElement("div");
        card.className = "bnb-card pad ssn-card";
        card.id = "card-" + key;
        // The heading is built from nodes: `key` comes from the address, and is text, not markup.
        const head = document.createElement("div");
        head.className = "section-header";
        const titles = document.createElement("div");
        const eyebrow = document.createElement("div");
        eyebrow.className = "bnb-eyebrow";
        eyebrow.textContent = "Year in review";
        const title = document.createElement("h2");
        title.className = "sh-h";
        title.textContent = "Who led each month";
        titles.appendChild(eyebrow);
        titles.appendChild(title);
        const label = document.createElement("span");
        label.className = "bnb-meta";
        label.textContent = key;
        head.appendChild(titles);
        head.appendChild(label);
        card.appendChild(head);
        const host = document.createElement("div");
        host.className = "ssn";
        host.setAttribute("data-ssn", "");
        host.setAttribute("data-ssn-json", JSON.stringify(d));
        card.appendChild(host);
        mount.appendChild(card);
      }
    }
  }
})();
