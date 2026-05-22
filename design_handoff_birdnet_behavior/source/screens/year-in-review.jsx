// Year-in-Review — editorial, celebratory annual recap.
// Big numbers, milestones, the year's story.

function YearInReview() {
  const { SPECIES } = window.BNB;
  const top = [...SPECIES].sort((a, b) => b.count - a.count).slice(0, 8);
  const firsts = [SPECIES[9], SPECIES[10], SPECIES[12], SPECIES[14], SPECIES[13], SPECIES[11]];

  return (
    <Screen padded={false}>
      <div style={{ padding: "var(--pad-4) var(--pad-4) var(--pad-3)" }}>
        <TopNav active="History" />
      </div>

      {/* Cinematic opener */}
      <div style={{
        padding: "60px var(--pad-4) 48px",
        textAlign: "center",
        background: "linear-gradient(180deg, color-mix(in oklch, var(--moss) 5%, var(--bg)) 0%, var(--bg) 100%)",
        borderBottom: "0.5px solid var(--hairline)",
      }}>
        <div className="bnb-eyebrow" style={{ letterSpacing: "0.20em", marginBottom: 14 }}>YEAR IN REVIEW · 2025 · STATION #001</div>
        <h1 className="display" style={{ fontSize: 96, lineHeight: 0.95, letterSpacing: "-0.035em", maxWidth: 1100, margin: "0 auto" }}>
          A year of <em style={{ color: "var(--moss-ink)" }}>listening</em>.
        </h1>
        <div style={{ marginTop: 18, fontSize: 16, color: "var(--fg-2)", maxWidth: 580, margin: "18px auto 0", lineHeight: 1.55 }}>
          From Mar 12 — your install date — through today. <span className="mono">328 days</span>, <span className="mono">142,180</span> detections, <span className="mono">76</span> species. Here's how it went.
        </div>
      </div>

      {/* Big-number reel */}
      <div style={{ padding: "var(--pad-4)" }}>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 0, border: "0.5px solid var(--border)", borderRadius: 16, overflow: "hidden", background: "var(--surface)" }}>
          <YIRBig n="142,180" label="Total detections" sub="↑ 22% over the same window in 2024" />
          <YIRBig n="76"      label="Species heard"     sub="6 new this year"             accent="var(--moss-ink)" />
          <YIRBig n="3,178"   label="Hours listening"   sub="0.6% downtime · 7 brief drops" />
          <YIRBig n="3"       label="Rare confirmed"    sub="2 still in quarantine"        accent="var(--rare)" last />
        </div>

        {/* The arc — year-long calendar heat */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", marginTop: 28 }}>
          <SectionHeader eyebrow="Every day · the year as a tape" title="The whole year, by detection count" action={<span className="bnb-meta">Darker tiles = busier days</span>} />
          <YearTape />
        </div>

        {/* Two-column: top species + lifers */}
        <div style={{ display: "grid", gridTemplateColumns: "1.6fr 1fr", gap: "var(--pad-3)", marginTop: 28 }}>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <SectionHeader eyebrow="The leaderboard" title="Loudest of the year" />
            <div style={{ marginTop: 14 }}>
              {top.map((s, i) => {
                const idx = SPECIES.indexOf(s);
                return (
                  <div key={i} style={{
                    display: "grid", gridTemplateColumns: "30px 36px 1fr 80px 200px 60px",
                    gap: 14, alignItems: "center", padding: "12px 0",
                    borderTop: i > 0 ? "0.5px solid var(--hairline)" : "0",
                  }}>
                    <span className="display tabular" style={{ fontSize: 22, color: "var(--fg-3)", textAlign: "right" }}>{i + 1}</span>
                    <SpeciesAvatar sp={idx} size={28} />
                    <div>
                      <div style={{ fontSize: 14, fontWeight: 500 }}>{s.common}</div>
                      <div className="bnb-meta mono" style={{ fontStyle: "italic" }}>{s.sci}</div>
                    </div>
                    <span className="mono tabular" style={{ fontSize: 14, textAlign: "right" }}>{s.count.toLocaleString()}</span>
                    <Sparkline data={s.trend} width={200} height={26} accent={s.color} />
                    <span className="bnb-pill moss" style={{ fontSize: 10, justifyContent: "center" }}>+{10 + i * 3}%</span>
                  </div>
                );
              })}
            </div>
          </div>

          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <SectionHeader eyebrow="First-ever detections" title="The new lifers" />
            <div style={{ marginTop: 14, display: "flex", flexDirection: "column" }}>
              {firsts.map((sp, i) => {
                const idx = SPECIES.indexOf(sp);
                return (
                  <div key={i} style={{
                    display: "grid", gridTemplateColumns: "auto 1fr auto", gap: 12,
                    padding: "12px 0", borderTop: i > 0 ? "0.5px dashed var(--hairline)" : "0",
                    alignItems: "center",
                  }}>
                    <SpeciesAvatar sp={idx} size={32} />
                    <div>
                      <div style={{ fontSize: 13.5, fontWeight: 500 }}>{sp.common}</div>
                      <div className="bnb-meta mono" style={{ fontStyle: "italic" }}>{sp.sci}</div>
                    </div>
                    <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)" }}>{["Apr 22", "May 4", "May 18", "Oct 19", "Nov 22", "May 15"][i]}</span>
                  </div>
                );
              })}
            </div>
          </div>
        </div>

        {/* Milestones strip */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", marginTop: 28 }}>
          <SectionHeader eyebrow="Milestones" title="Days that mattered" />
          <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 0, marginTop: 14, border: "0.5px solid var(--border)", borderRadius: 12, overflow: "hidden" }}>
            <Milestone date="Mar 12" title="Day one" sub="First detection: Cardinal · 6 species heard before lunch" />
            <Milestone date="May 18" title="Earliest warbler" sub="Yellow-rumped, 4 days before 2024" accent="var(--moss-ink)" />
            <Milestone date="Aug 04" title="Best dawn chorus" sub="11 species in 90 minutes · all-time record" accent="var(--dawn-ink)" />
            <Milestone date="Oct 19" title="First Barred Owl" sub="Confirmed at 02:14 a.m. · 0.93 confidence" accent="var(--rare)" last />
          </div>
        </div>

        {/* Two-column: seasonal + dawn chorus length */}
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--pad-3)", marginTop: 28 }}>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <SectionHeader eyebrow="Seasonal split" title="When the year was loudest" />
            <SeasonalDonut />
          </div>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <SectionHeader eyebrow="Chorus length" title="Dawn-chorus duration · weekly" />
            <ChorusLengthChart />
          </div>
        </div>

        {/* Closing */}
        <div style={{ marginTop: 48, padding: "var(--pad-4)", textAlign: "center", background: "color-mix(in oklch, var(--moss) 4%, var(--surface))", borderRadius: 16, border: "0.5px solid color-mix(in oklch, var(--moss) 18%, var(--border))" }}>
          <div className="bnb-eyebrow" style={{ color: "var(--moss-ink)" }}>The year ahead</div>
          <h3 className="display" style={{ fontSize: 36, lineHeight: 1.1, marginTop: 6 }}>Likely still to come</h3>
          <p style={{ marginTop: 10, color: "var(--fg-2)", fontSize: 14, maxWidth: 580, margin: "10px auto 0", lineHeight: 1.55 }}>
            eBird's regional model says these nine species should appear in your radius before year-end. Two are migrants you missed last year — keep an ear out.
          </p>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 8, justifyContent: "center", marginTop: 18 }}>
            {["Ruby-throated Hummingbird", "Indigo Bunting", "Common Yellowthroat", "Eastern Wood-Pewee", "Scarlet Tanager", "Yellow Warbler", "Pine Siskin", "Hermit Thrush", "Winter Wren"].map((n, i) => (
              <span key={i} className="bnb-pill" style={{ fontSize: 12, padding: "5px 10px" }}>{n}</span>
            ))}
          </div>
          <div style={{ display: "flex", gap: 12, marginTop: 24, justifyContent: "center" }}>
            <button className="bnb-btn primary">Export year as PDF</button>
            <button className="bnb-btn">Share to BirdWeather</button>
          </div>
        </div>
      </div>
    </Screen>
  );
}

function YIRBig({ n, label, sub, accent, last }) {
  return (
    <div style={{ padding: "var(--pad-4)", borderRight: last ? "none" : "0.5px solid var(--hairline)", display: "flex", flexDirection: "column", gap: 10 }}>
      <div className="bnb-eyebrow">{label}</div>
      <div className="display tabular" style={{ fontSize: 56, lineHeight: 0.95, letterSpacing: "-0.025em", color: accent || "var(--fg)" }}>{n}</div>
      <div className="bnb-meta mono">{sub}</div>
    </div>
  );
}

function YearTape() {
  // 52 weeks × 7 days = 364 cells
  let s = 11;
  const r = () => { s = (s * 9301 + 49297) % 233280; return s / 233280; };
  const cells = [];
  for (let w = 0; w < 52; w++) {
    const col = [];
    for (let d = 0; d < 7; d++) {
      // Seasonal envelope: peak in May and Sept
      const spring = 0.6 * Math.exp(-Math.pow((w - 19) / 5, 2));
      const fall = 0.5 * Math.exp(-Math.pow((w - 37) / 6, 2));
      const base = 0.2 + spring + fall;
      const noise = (r() - 0.5) * 0.4;
      col.push(Math.max(0, Math.min(1, base + noise)));
    }
    cells.push(col);
  }

  return (
    <div style={{ marginTop: 14 }}>
      <div style={{ display: "flex", gap: 4, alignItems: "flex-start" }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 3, paddingTop: 18, marginRight: 4 }}>
          {["M", "W", "F"].map((d, i) => (
            <span key={d} className="mono" style={{ fontSize: 9.5, color: "var(--fg-3)", height: 8, lineHeight: 1 }}>{d}</span>
          ))}
        </div>
        <div style={{ flex: 1 }}>
          {/* month labels */}
          <div style={{ display: "grid", gridTemplateColumns: "repeat(52, 1fr)", gap: 3, marginBottom: 4 }}>
            {Array.from({ length: 52 }).map((_, w) => {
              const showMonth = [0, 4, 9, 13, 17, 22, 26, 30, 35, 39, 44, 48].includes(w);
              const m = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"][[0,4,9,13,17,22,26,30,35,39,44,48].indexOf(w)];
              return <span key={w} className="mono" style={{ fontSize: 9, color: "var(--fg-3)", textAlign: "left", gridColumn: "span 1" }}>{showMonth ? m : ""}</span>;
            })}
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(52, 1fr)", gridTemplateRows: "repeat(7, 8px)", gap: 3, gridAutoFlow: "column" }}>
            {cells.flatMap((col, w) => col.map((v, d) => (
              <span key={`${w}-${d}`} style={{
                background: v < 0.05 ? "var(--surface-2)" : `color-mix(in oklch, var(--moss) ${Math.round(v * 78)}%, var(--surface-2))`,
                borderRadius: 2,
              }} title={`week ${w + 1}, day ${d}`} />
            )))}
          </div>
        </div>
      </div>
    </div>
  );
}

function Milestone({ date, title, sub, accent, last }) {
  return (
    <div style={{ padding: "var(--pad-3)", borderRight: last ? "none" : "0.5px solid var(--hairline)", display: "flex", flexDirection: "column", gap: 6 }}>
      <div className="mono" style={{ fontSize: 11, letterSpacing: "0.08em", color: accent || "var(--fg-3)", textTransform: "uppercase" }}>{date}</div>
      <div className="display" style={{ fontSize: 20, lineHeight: 1.15 }}>{title}</div>
      <div className="bnb-meta" style={{ lineHeight: 1.5 }}>{sub}</div>
    </div>
  );
}

function SeasonalDonut() {
  const seasons = [
    { label: "Spring", pct: 0.34, color: "oklch(70% 0.12 150)", count: 48340 },
    { label: "Summer", pct: 0.18, color: "oklch(78% 0.16 95)",  count: 25590 },
    { label: "Fall",   pct: 0.30, color: "oklch(68% 0.14 60)",  count: 42650 },
    { label: "Winter", pct: 0.18, color: "oklch(58% 0.06 240)", count: 25600 },
  ];
  const cx = 110, cy = 110, r = 78, ir = 50;
  let acc = -Math.PI / 2;
  return (
    <div style={{ display: "flex", gap: 24, alignItems: "center", marginTop: 14 }}>
      <svg width="220" height="220" viewBox="0 0 220 220">
        {seasons.map((s, i) => {
          const a0 = acc;
          const a1 = acc + s.pct * Math.PI * 2;
          acc = a1;
          const large = (a1 - a0) > Math.PI ? 1 : 0;
          const x0 = cx + r * Math.cos(a0), y0 = cy + r * Math.sin(a0);
          const x1 = cx + r * Math.cos(a1), y1 = cy + r * Math.sin(a1);
          const xi0 = cx + ir * Math.cos(a0), yi0 = cy + ir * Math.sin(a0);
          const xi1 = cx + ir * Math.cos(a1), yi1 = cy + ir * Math.sin(a1);
          return (
            <path key={i}
              d={`M${x0},${y0} A${r},${r} 0 ${large} 1 ${x1},${y1} L${xi1},${yi1} A${ir},${ir} 0 ${large} 0 ${xi0},${yi0} Z`}
              fill={s.color} opacity="0.85" />
          );
        })}
        <text x={cx} y={cy - 6} textAnchor="middle" className="display" style={{ fontSize: 18, fill: "var(--fg-3)" }}>year</text>
        <text x={cx} y={cy + 12} textAnchor="middle" className="display tabular" style={{ fontSize: 16, fill: "var(--fg)" }}>142k</text>
      </svg>
      <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 6 }}>
        {seasons.map((s) => (
          <div key={s.label} style={{ display: "grid", gridTemplateColumns: "auto 1fr auto auto", gap: 10, alignItems: "center" }}>
            <span style={{ width: 10, height: 10, borderRadius: 2, background: s.color }} />
            <span style={{ fontSize: 13 }}>{s.label}</span>
            <span className="mono tabular" style={{ fontSize: 12, color: "var(--fg-2)" }}>{s.count.toLocaleString()}</span>
            <span className="mono tabular" style={{ fontSize: 11, color: "var(--fg-3)", minWidth: 32, textAlign: "right" }}>{(s.pct * 100).toFixed(0)}%</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function ChorusLengthChart() {
  // Synthetic: chorus duration in minutes, by week of year
  const data = Array.from({ length: 52 }, (_, w) => {
    const spring = 130 * Math.exp(-Math.pow((w - 19) / 8, 2));
    const fall = 70 * Math.exp(-Math.pow((w - 37) / 9, 2));
    return Math.round(20 + spring + fall);
  });
  const max = Math.max(...data);
  const W = 480, H = 140;
  const path = data.map((v, w) => `${w === 0 ? "M" : "L"}${(w / (data.length - 1)) * W},${H - 12 - (v / max) * (H - 24)}`).join(" ");
  return (
    <svg viewBox={`0 0 ${W} ${H + 14}`} width="100%" height={H + 14} preserveAspectRatio="none" style={{ marginTop: 14 }}>
      <defs>
        <linearGradient id="chorus-grad" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--dawn)" stopOpacity="0.55" />
          <stop offset="100%" stopColor="var(--dawn)" stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={`${path} L${W},${H - 12} L0,${H - 12} Z`} fill="url(#chorus-grad)" />
      <path d={path} stroke="var(--dawn)" fill="none" strokeWidth="2" />
      {/* peak */}
      <circle cx={(data.indexOf(max) / (data.length - 1)) * W} cy={H - 12 - (max / max) * (H - 24)} r="4" fill="var(--dawn)" stroke="var(--surface)" strokeWidth="1.5" />
      <text x={(data.indexOf(max) / (data.length - 1)) * W} y={H - 12 - (max / max) * (H - 24) - 10} textAnchor="middle" className="mono" style={{ fontSize: 11, fill: "var(--dawn-ink)", fontWeight: 600 }}>148 min</text>
      {/* X axis */}
      {[0, 13, 26, 39].map((w, i) => (
        <text key={i} x={(w / (data.length - 1)) * W} y={H + 6} className="mono" style={{ fontSize: 10, fill: "var(--fg-3)" }}>{["Jan","Apr","Jul","Oct"][i]}</text>
      ))}
    </svg>
  );
}

Object.assign(window, { YearInReview });
