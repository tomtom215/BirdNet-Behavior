// Migration phenology — ridgeline plot of weekly abundance, plus arrival/departure marks.
function Migration() {
  const { MIGRATION, SPECIES } = window.BNB;
  const W = 1240, H = 360;
  const padL = 56, padR = 16, padT = 16, padB = 32;
  const innerW = W - padL - padR;
  const innerH = H - padT - padB;
  const weeks = 52;
  const x = (w) => padL + (w / (weeks - 1)) * innerW;

  return (
    <Screen>
      <TopNav active="Analytics" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Behavioral analytics · phenology</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Arrivals and departures</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 540 }}>
            Weekly abundance index for migratory species this year. Each ridge is normalized to its own peak so small-population species stay readable.
          </div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <span className="bnb-pill">Calendar year 2025</span>
          <span className="bnb-pill">Migratory only</span>
          <span className="bnb-pill">vs. eBird baseline ▾</span>
        </div>
      </div>

      {/* Stats up top */}
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr 1fr", gap: "var(--pad-3)" }}>
        <Stat label="First-of-year arrivals" value="6" sub="2 this week" accent="var(--moss-ink)" />
        <Stat label="Peak diversity" value="May 8–14" sub="32 species in 7 days" />
        <Stat label="Earliest vs. 2024" value="−4 d" sub="Yellow-rumped Warbler" accent="var(--dawn-ink)" />
        <Stat label="Still expected" value="9 species" sub="seasonal forecast" />
      </div>

      <div className="bnb-card" style={{ padding: "var(--pad-3)", flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}>
        <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6 }}>
          <div className="bnb-eyebrow">Ridgeline · per-species weekly index</div>
          <div style={{ display: "flex", gap: 14, alignItems: "center" }}>
            <span className="bnb-meta" style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
              <span style={{ width: 10, height: 6, borderRadius: 2, background: "color-mix(in oklch, var(--moss) 60%, transparent)" }} /> spring window
            </span>
            <span className="bnb-meta" style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
              <span style={{ width: 10, height: 6, borderRadius: 2, background: "color-mix(in oklch, var(--dawn) 60%, transparent)" }} /> fall window
            </span>
            <div className="bnb-meta mono">w 1 — 52</div>
          </div>
        </div>
        <svg viewBox={`0 0 ${W} ${H}`} width="100%" height="auto" preserveAspectRatio="none" style={{ flex: 1, minHeight: 0 }}>
          {/* Month gridlines */}
          {MONTH_WEEKS.map(({ label, week }, i) => (
            <g key={label}>
              <line x1={x(week)} y1={padT} x2={x(week)} y2={padT + innerH} stroke="var(--hairline)" />
              <text x={x(week)} y={padT + innerH + 18} textAnchor="middle" className="mono" style={{ fontSize: 11, fill: "var(--fg-3)" }}>{label}</text>
            </g>
          ))}
          {/* Season bands */}
          <rect x={x(8)} y={padT} width={x(20) - x(8)} height={innerH} fill="var(--moss-soft)" fillOpacity="0.35" />
          <text x={x(14)} y={padT + 12} textAnchor="middle" className="mono" style={{ fontSize: 10, fill: "var(--moss-ink)" }}>spring migration</text>
          <rect x={x(34)} y={padT} width={x(44) - x(34)} height={innerH} fill="var(--dawn-soft)" fillOpacity="0.45" />
          <text x={x(39)} y={padT + 12} textAnchor="middle" className="mono" style={{ fontSize: 10, fill: "var(--dawn-ink)" }}>fall migration</text>

          {/* Each species: ridge */}
          {MIGRATION.map((m, i) => {
            const sp = SPECIES[m.sp];
            const rowH = innerH / MIGRATION.length;
            const yBase = padT + (i + 1) * rowH - 6;
            const max = Math.max(0.001, ...m.curve);
            const pts = m.curve.map((v, w) => [x(w), yBase - (v / max) * (rowH - 8)]);
            const path = pts.map(([px, py], j) => `${j === 0 ? "M" : "L"}${px.toFixed(2)},${py.toFixed(2)}`).join(" ");
            const area = `${path} L${x(weeks - 1)},${yBase} L${x(0)},${yBase} Z`;
            const peakWeek = m.curve.indexOf(max);
            const peakY = yBase - (rowH - 8);
            const gradId = `mg-${i}`;
            return (
              <g key={i}>
                <line x1={padL} y1={yBase} x2={padL + innerW} y2={yBase} stroke="var(--hairline)" />
                <defs>
                  <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor={sp.color} stopOpacity="0.55" />
                    <stop offset="100%" stopColor={sp.color} stopOpacity="0.05" />
                  </linearGradient>
                </defs>
                <g className="ridge-band">
                  <path d={area} fill={`url(#${gradId})`} />
                  <path d={path} stroke={sp.color} fill="none" strokeWidth={1.5} />
                </g>
                {/* Peak marker */}
                <line x1={x(peakWeek)} y1={peakY} x2={x(peakWeek)} y2={yBase} stroke={sp.color} strokeWidth={0.8} strokeDasharray="2 2" strokeOpacity={0.4} />
                <circle cx={x(peakWeek)} cy={peakY} r={3} fill={sp.color} stroke="var(--surface)" strokeWidth={1} />
                {/* Species label */}
                <text x={padL - 8} y={yBase - 6} textAnchor="end" style={{ fontSize: 12, fill: "var(--fg)", fontWeight: 500 }}>{sp.common}</text>
                <text x={padL - 8} y={yBase + 8} textAnchor="end" className="mono" style={{ fontSize: 9.5, fill: "var(--fg-3)" }}>{sp.short} · peak w{peakWeek + 1}</text>
              </g>
            );
          })}

          {/* Today indicator at week 21 */}
          <g>
            <line x1={x(21)} y1={padT} x2={x(21)} y2={padT + innerH} stroke="var(--fg)" strokeWidth={1} strokeDasharray="3 3" />
            <rect x={x(21) - 24} y={padT - 2} width="48" height="14" rx="3" fill="var(--fg)" />
            <text x={x(21)} y={padT + 8} textAnchor="middle" style={{ fontSize: 10, fill: "var(--bg)" }} className="mono">today</text>
          </g>
        </svg>

        {/* Weekly diversity bar — total active species per week as a small bar chart underneath */}
        <div style={{ marginTop: 12, paddingTop: 12, borderTop: "0.5px solid var(--hairline)" }}>
          <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6 }}>
            <div className="bnb-eyebrow">Weekly diversity · all species combined</div>
            <div className="bnb-meta mono">peak May 8–14 · 32 species</div>
          </div>
          <DiversityBars weeks={52} />
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: "var(--pad-3)" }}>
        <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
          <div className="bnb-eyebrow" style={{ color: "var(--moss-ink)" }}>Just arrived</div>
          <div className="display" style={{ fontSize: 18, marginTop: 6 }}>Yellow-rumped Warbler</div>
          <div className="bnb-meta" style={{ marginTop: 4 }}>First heard May 18 · 4 days earlier than 2024</div>
        </div>
        <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
          <div className="bnb-eyebrow" style={{ color: "var(--dawn-ink)" }}>Currently peaking</div>
          <div className="display" style={{ fontSize: 18, marginTop: 6 }}>Magnolia Warbler</div>
          <div className="bnb-meta" style={{ marginTop: 4 }}>Week 20 · 22 detections this week (year-high)</div>
        </div>
        <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
          <div className="bnb-eyebrow" style={{ color: "var(--rare)" }}>Missing</div>
          <div className="display" style={{ fontSize: 18, marginTop: 6 }}>Wood Thrush</div>
          <div className="bnb-meta" style={{ marginTop: 4 }}>Expected by week 19 · 0 detections so far this year</div>
        </div>
      </div>
    </Screen>
  );
}

const MONTH_WEEKS = [
  { label: "Jan", week: 0 }, { label: "Feb", week: 4 }, { label: "Mar", week: 9 },
  { label: "Apr", week: 13 }, { label: "May", week: 17 }, { label: "Jun", week: 22 },
  { label: "Jul", week: 26 }, { label: "Aug", week: 30 }, { label: "Sep", week: 35 },
  { label: "Oct", week: 39 }, { label: "Nov", week: 44 }, { label: "Dec", week: 48 },
];

function DiversityBars({ weeks = 52 }) {
  // Synthetic species count per week — peaks in May and Sep.
  const data = Array.from({ length: weeks }, (_, w) => {
    const spring = 28 * Math.exp(-Math.pow((w - 19) / 5, 2));
    const fall   = 24 * Math.exp(-Math.pow((w - 37) / 6, 2));
    const winter = 8;
    return Math.round(winter + spring + fall + ((w * 7) % 5));
  });
  const max = Math.max(...data);
  const W = 1240, H = 70, padL = 56, padR = 16;
  const innerW = W - padL - padR;
  const bw = innerW / weeks;
  const x = (w) => padL + (w / (weeks - 1)) * innerW;

  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} preserveAspectRatio="none">
      {data.map((v, w) => {
        const isSpring = w >= 8 && w <= 20;
        const isFall = w >= 34 && w <= 44;
        const fill = isSpring ? "var(--moss)" : isFall ? "var(--dawn)" : "var(--fg-3)";
        const op = 0.30 + (v / max) * 0.55;
        const h = (v / max) * (H - 16);
        return (
          <g key={w}>
            <rect x={x(w) - bw / 2 + 0.5} y={H - 14 - h} width={bw - 1} height={h} fill={fill} opacity={op} rx="1" />
            {/* label every fourth bar */}
            {w % 4 === 0 && (
              <text x={x(w)} y={H - 14 - h - 3} textAnchor="middle" className="mono" style={{ fontSize: 8.5, fill: "var(--fg-3)" }}>{v}</text>
            )}
          </g>
        );
      })}
      {/* axis */}
      <line x1={padL} y1={H - 14} x2={padL + innerW} y2={H - 14} stroke="var(--hairline)" />
      <text x={padL - 8} y={H - 12} textAnchor="end" className="mono" style={{ fontSize: 9.5, fill: "var(--fg-3)" }}>species / wk</text>
      {/* today */}
      <line x1={x(21)} y1={2} x2={x(21)} y2={H - 14} stroke="var(--fg)" strokeWidth={1} strokeDasharray="3 3" />
    </svg>
  );
}

Object.assign(window, { Migration });
