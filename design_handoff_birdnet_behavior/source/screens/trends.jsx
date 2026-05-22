// Trends & Comparisons — period-over-period analytics across time.
// Week-over-week · month-over-month · year-over-year · this day last year.

const { useState: useState_tr } = React;

function Trends() {
  const { SPECIES } = window.BNB;
  const [period, setPeriod] = useState_tr("week");

  return (
    <Screen>
      <TopNav active="History" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>History · trends & comparisons</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>How this {period} compares</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 580 }}>
            Side-by-side comparisons across periods. Year-over-year overlays. Long-term trends. Same-day-last-year recall.
          </div>
        </div>
        <PeriodPicker value={period} onChange={setPeriod} />
      </div>

      {/* Headline comparison cards */}
      <div style={{ display: "grid", gridTemplateColumns: "1.4fr 1fr 1fr", gap: "var(--pad-3)" }}>
        <BigComparisonCard period={period} />
        <DiversityCompareCard period={period} />
        <NoveltyCard period={period} />
      </div>

      {/* Year-over-year overlay */}
      <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", marginBottom: 12 }}>
          <SectionHeader eyebrow={`${period.charAt(0).toUpperCase()}${period.slice(1)}-over-${period}`} title="Year-on-year overlay" />
          <div style={{ display: "flex", gap: 14, alignItems: "center" }}>
            <LegendDot color="var(--moss)"   label="2025" weight="500" />
            <LegendDot color="var(--fg-3)"   label="2024" dashed />
            <LegendDot color="var(--dawn)"   label="3-yr average" thin />
          </div>
        </div>
        <YoYChart period={period} />
        <div className="bnb-meta" style={{ marginTop: 8, paddingTop: 10, borderTop: "0.5px solid var(--hairline)" }}>
          Activity is tracking <span style={{ color: "var(--moss-ink)", fontWeight: 500 }}>22% above</span> last year across the same window, driven mostly by an early Magnolia Warbler arrival and a longer dawn chorus.
        </div>
      </div>

      {/* Two-column: species cohort + "on this day" */}
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--pad-3)" }}>
        <SpeciesCohortCard period={period} />
        <OnThisDayCard />
      </div>

      {/* Long-term sparkline grid */}
      <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
        <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 12 }}>
          <SectionHeader eyebrow="14 months · all species" title="Long-term trends" />
          <div className="bnb-meta">Each row: same species, weekly normalized · January 2024 → today</div>
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: "var(--pad-2) var(--pad-3)" }}>
          {SPECIES.slice(0, 10).map((s, i) => (
            <LongTermRow key={i} sp={s} idx={i} />
          ))}
        </div>
      </div>
    </Screen>
  );
}

// ─── Period picker ────────────────────────────────────────────────────────
function PeriodPicker({ value, onChange }) {
  const options = [
    { id: "week",  label: "Week" },
    { id: "month", label: "Month" },
    { id: "year",  label: "Year" },
  ];
  return (
    <div style={{ display: "flex", gap: 0, background: "var(--surface)", padding: 3, borderRadius: 8, border: "0.5px solid var(--border-2)" }}>
      {options.map((o) => (
        <button key={o.id} onClick={() => onChange(o.id)} style={{
          padding: "8px 16px", borderRadius: 6,
          background: value === o.id ? "var(--fg)" : "transparent",
          color: value === o.id ? "var(--bg)" : "var(--fg-2)",
          border: 0, cursor: "pointer",
          fontSize: 13, fontWeight: value === o.id ? 600 : 500,
          fontFamily: "var(--font-ui)",
          transition: "background .15s",
        }}>{o.label}</button>
      ))}
    </div>
  );
}

// ─── Big comparison card (lead headline) ──────────────────────────────────
function BigComparisonCard({ period }) {
  const data = {
    week:  { now: 6238, prev: 5108, label: "This week",     prevLabel: "Last week",     curRange: "May 16 – 22",     prevRange: "May 9 – 15" },
    month: { now: 24840, prev: 19260, label: "This month",  prevLabel: "Last month",    curRange: "May 1 – 22",      prevRange: "April"      },
    year:  { now: 142180, prev: 116410, label: "This year", prevLabel: "Last year",     curRange: "Jan 1 – May 22",  prevRange: "Same window 2024" },
  }[period];
  const delta = ((data.now - data.prev) / data.prev) * 100;
  const up = delta > 0;
  return (
    <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 14 }}>
      <div className="bnb-eyebrow">{data.label} vs. {data.prevLabel}</div>
      <div>
        <div className="display tabular" style={{ fontSize: 56, lineHeight: 1, letterSpacing: "-0.025em" }}>{data.now.toLocaleString()}</div>
        <div className="bnb-meta mono" style={{ marginTop: 6 }}>{data.curRange} · detections</div>
      </div>
      <div style={{ display: "flex", gap: 16, alignItems: "center", padding: "10px 12px", background: up ? "var(--moss-soft)" : "var(--dawn-soft)", borderRadius: 8 }}>
        <span style={{
          width: 28, height: 28, borderRadius: 999,
          background: up ? "var(--moss)" : "var(--dawn)", color: "var(--bg)",
          display: "inline-flex", alignItems: "center", justifyContent: "center", fontWeight: 700,
        }}>{up ? "↑" : "↓"}</span>
        <div>
          <div style={{ fontSize: 16, fontWeight: 600, color: up ? "var(--moss-ink)" : "var(--dawn-ink)" }}>
            {up ? "+" : ""}{delta.toFixed(1)}%
          </div>
          <div className="bnb-meta" style={{ marginTop: 2 }}>vs. {data.prev.toLocaleString()} ({data.prevRange})</div>
        </div>
      </div>
      <BeforeAfterMini period={period} />
    </div>
  );
}

function BeforeAfterMini({ period }) {
  const series = period === "week"
    ? { now: [110, 142, 168, 220, 178, 195, 165], prev: [88, 102, 121, 165, 142, 168, 130], labels: ["S","M","T","W","T","F","S"] }
    : period === "month"
      ? { now: Array.from({ length: 22 }, (_, i) => 800 + Math.sin(i * 0.7) * 280 + i * 6), prev: Array.from({ length: 30 }, (_, i) => 600 + Math.sin(i * 0.5) * 200 + i * 3), labels: [] }
      : { now: Array.from({ length: 22 }, (_, i) => Math.max(0, 1800 + Math.sin(i / 7) * 1200 - Math.pow((i - 18) / 4, 2) * 80)), prev: Array.from({ length: 52 }, (_, i) => 1200 + Math.sin(i / 7) * 800), labels: [] };
  return (
    <DualLineChart now={series.now} prev={series.prev} labels={series.labels} />
  );
}

function DualLineChart({ now, prev, labels = [] }) {
  const W = 380, H = 88;
  const max = Math.max(...now, ...prev);
  const toPath = (d) => d.map((v, i) => `${i === 0 ? "M" : "L"}${(i / (d.length - 1)) * W},${H - 8 - (v / max) * (H - 16)}`).join(" ");
  return (
    <svg viewBox={`0 0 ${W} ${H + 16}`} width="100%" height={H + 16} preserveAspectRatio="none">
      <path d={toPath(prev)} stroke="var(--fg-3)" fill="none" strokeWidth="1.2" strokeDasharray="3 3" opacity="0.7" />
      <path d={`${toPath(now)} L${W},${H - 8} L0,${H - 8} Z`} fill="var(--moss)" fillOpacity="0.14" />
      <path d={toPath(now)} stroke="var(--moss)" fill="none" strokeWidth="1.8" />
      {labels.map((l, i) => (
        <text key={i} x={(i / (labels.length - 1)) * W} y={H + 10} className="mono" style={{ fontSize: 9.5, fill: "var(--fg-3)" }} textAnchor="middle">{l}</text>
      ))}
    </svg>
  );
}

function LegendDot({ color, label, dashed, thin, weight }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
      <span style={{
        width: 16, height: 2.5,
        background: dashed ? `repeating-linear-gradient(90deg, ${color} 0 3px, transparent 3px 5px)` : color,
        opacity: thin ? 0.6 : 1,
      }} />
      <span className="bnb-meta" style={{ fontWeight: weight, color: "var(--fg-2)", fontSize: 11.5 }}>{label}</span>
    </span>
  );
}

// ─── Diversity comparison card ────────────────────────────────────────────
function DiversityCompareCard({ period }) {
  const data = {
    week:  { now: 23, prev: 19, label: "Species heard" },
    month: { now: 38, prev: 32, label: "Species heard" },
    year:  { now: 76, prev: 71, label: "Species heard" },
  }[period];
  const delta = data.now - data.prev;
  return (
    <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 12 }}>
      <div className="bnb-eyebrow">{data.label}</div>
      <div style={{ display: "flex", alignItems: "baseline", gap: 14 }}>
        <span className="display tabular" style={{ fontSize: 48, lineHeight: 1, color: "var(--moss-ink)" }}>{data.now}</span>
        <span className="mono" style={{ fontSize: 14, color: "var(--fg-3)" }}>vs. {data.prev}</span>
      </div>
      <div className="bnb-meta">
        {delta > 0 ? <><span style={{ color: "var(--moss-ink)", fontWeight: 500 }}>+{delta} species</span> compared to the previous period.</> : <>Same diversity as the previous period.</>}
      </div>
      <div style={{ marginTop: 4 }}>
        <DiversityStack now={data.now} prev={data.prev} />
      </div>
    </div>
  );
}

function DiversityStack({ now, prev }) {
  const max = Math.max(now, prev);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div>
        <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 4 }}>
          <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>current</span>
          <span className="mono tabular" style={{ fontSize: 11, color: "var(--fg)" }}>{now}</span>
        </div>
        <div style={{ height: 8, background: "var(--bg-2)", borderRadius: 2, overflow: "hidden" }}>
          <div style={{ width: `${(now / max) * 100}%`, height: "100%", background: "var(--moss)" }} />
        </div>
      </div>
      <div>
        <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 4 }}>
          <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>previous</span>
          <span className="mono tabular" style={{ fontSize: 11, color: "var(--fg-2)" }}>{prev}</span>
        </div>
        <div style={{ height: 8, background: "var(--bg-2)", borderRadius: 2, overflow: "hidden" }}>
          <div style={{ width: `${(prev / max) * 100}%`, height: "100%", background: "var(--fg-3)" }} />
        </div>
      </div>
    </div>
  );
}

// ─── Novelty (gained/lost) ───────────────────────────────────────────────
function NoveltyCard({ period }) {
  const { SPECIES } = window.BNB;
  const gained = period === "week" ? [SPECIES[10], SPECIES[12]] : period === "month" ? [SPECIES[10], SPECIES[12], SPECIES[14]] : [SPECIES[10], SPECIES[12], SPECIES[14], SPECIES[13], SPECIES[11], SPECIES[9]];
  const lost = period === "year" ? [SPECIES[7], SPECIES[8]] : [];
  return (
    <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 12 }}>
      <div className="bnb-eyebrow">Species turnover</div>
      <div>
        <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
          <span className="display tabular" style={{ fontSize: 32, color: "var(--moss-ink)" }}>+{gained.length}</span>
          <span className="bnb-meta">arrived this {period}</span>
        </div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 8 }}>
          {gained.map((sp, i) => (
            <span key={i} className="bnb-pill" style={{ fontSize: 11, background: `color-mix(in oklch, ${sp.color} 16%, var(--surface))`, color: sp.color, fontWeight: 500, border: 0 }}>
              <SpeciesAvatar sp={window.BNB.SPECIES.indexOf(sp)} size={16} /> {sp.common}
            </span>
          ))}
        </div>
      </div>
      {lost.length > 0 && (
        <div style={{ borderTop: "0.5px solid var(--hairline)", paddingTop: 12 }}>
          <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
            <span className="display tabular" style={{ fontSize: 24, color: "var(--fg-3)" }}>−{lost.length}</span>
            <span className="bnb-meta">missing vs. last {period}</span>
          </div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 8 }}>
            {lost.map((sp, i) => (
              <span key={i} className="bnb-pill" style={{ fontSize: 11, background: "var(--surface-2)", color: "var(--fg-3)", fontWeight: 500, border: "0.5px dashed var(--border-2)" }}>
                {sp.common}
              </span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ─── YoY overlay chart ───────────────────────────────────────────────────
function YoYChart({ period }) {
  // Generate three series — current, last year, 3-year avg
  const N = period === "week" ? 7 : period === "month" ? 30 : 52;
  let s = 1;
  const rand = () => { s = (s * 9301 + 49297) % 233280; return s / 233280; };
  const baseShape = (i) => {
    if (period === "year") {
      return 60 * Math.exp(-Math.pow((i - 18) / 6, 2)) + 50 * Math.exp(-Math.pow((i - 38) / 7, 2)) + 22;
    }
    return 0.5 + 0.45 * Math.sin(i * 0.45) + 0.15 * Math.sin(i * 1.3);
  };
  const cur = Array.from({ length: N }, (_, i) => Math.max(0, baseShape(i) * (1.15 + rand() * 0.18)));
  const prev = Array.from({ length: N }, (_, i) => Math.max(0, baseShape(i) * (0.92 + rand() * 0.18)));
  const avg = Array.from({ length: N }, (_, i) => Math.max(0, baseShape(i) * (0.98 + rand() * 0.10)));

  const W = 1340, H = 240;
  const padL = 56, padR = 16, padT = 14, padB = 28;
  const innerW = W - padL - padR, innerH = H - padT - padB;
  const max = Math.max(...cur, ...prev, ...avg);
  const x = (i) => padL + (i / (N - 1)) * innerW;
  const y = (v) => padT + innerH - (v / max) * innerH;
  const path = (arr) => arr.map((v, i) => `${i === 0 ? "M" : "L"}${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(" ");

  // X labels
  const labels = period === "week" ? ["Sun","Mon","Tue","Wed","Thu","Fri","Sat"]
              : period === "month" ? ["1", "5", "10", "15", "20", "25", "30"]
              : ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
  const labelStep = period === "week" ? 1 : period === "month" ? 5 : Math.round(N / 12);

  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} preserveAspectRatio="none">
      {/* gridlines */}
      {[0.25, 0.5, 0.75, 1.0].map((f) => (
        <g key={f}>
          <line x1={padL} y1={padT + innerH * (1 - f)} x2={padL + innerW} y2={padT + innerH * (1 - f)} stroke="var(--hairline)" />
          <text x={padL - 8} y={padT + innerH * (1 - f) + 3} textAnchor="end" className="mono" style={{ fontSize: 9.5, fill: "var(--fg-3)" }}>{Math.round(max * f)}</text>
        </g>
      ))}
      {/* 3-yr avg */}
      <path d={path(avg)} stroke="var(--dawn)" fill="none" strokeWidth="1" opacity="0.5" />
      {/* last year dashed */}
      <path d={path(prev)} stroke="var(--fg-3)" fill="none" strokeWidth="1.4" strokeDasharray="4 3" opacity="0.8" />
      {/* current with area */}
      <path d={`${path(cur)} L${x(N - 1)},${y(0)} L${x(0)},${y(0)} Z`} fill="var(--moss)" fillOpacity="0.16" />
      <path d={path(cur)} stroke="var(--moss)" fill="none" strokeWidth="2.2" />
      {/* X labels */}
      {Array.from({ length: Math.floor(N / labelStep) + 1 }).map((_, i) => {
        const idx = period === "week" ? i : period === "month" ? Math.min(N - 1, i * 5) : Math.min(N - 1, Math.round(i * N / 12));
        const lbl = period === "year" ? labels[Math.min(11, i)] : labels[idx % labels.length] || String(idx + 1);
        return (
          <text key={i} x={x(idx)} y={padT + innerH + 16} textAnchor="middle" className="mono" style={{ fontSize: 10, fill: "var(--fg-3)" }}>{lbl}</text>
        );
      })}
      {/* Today marker for year */}
      {period === "year" && (
        <g>
          <line x1={x(20)} y1={padT} x2={x(20)} y2={padT + innerH} stroke="var(--fg)" strokeWidth="1" strokeDasharray="3 3" />
          <rect x={x(20) - 24} y={padT - 2} width="48" height="14" rx="3" fill="var(--fg)" />
          <text x={x(20)} y={padT + 8} textAnchor="middle" className="mono" style={{ fontSize: 10, fill: "var(--bg)" }}>today</text>
        </g>
      )}
    </svg>
  );
}

// ─── Species cohort card ─────────────────────────────────────────────────
function SpeciesCohortCard({ period }) {
  const { SPECIES } = window.BNB;
  const rows = SPECIES.slice(0, 6).map((sp, i) => ({
    sp, now: sp.count, prev: Math.round(sp.count * (0.6 + (i % 5) * 0.15)),
  }));
  return (
    <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column" }}>
      <SectionHeader eyebrow="By species" title="Who showed up more — and less" />
      <div style={{ marginTop: 14, display: "flex", flexDirection: "column", gap: 0 }}>
        {rows.map(({ sp, now, prev }, i) => {
          const idx = SPECIES.indexOf(sp);
          const delta = ((now - prev) / prev) * 100;
          const up = delta > 0;
          return (
            <div key={i} style={{
              display: "grid", gridTemplateColumns: "auto 1fr 80px 60px 70px",
              gap: 12, alignItems: "center", padding: "10px 0",
              borderTop: i > 0 ? "0.5px solid var(--hairline)" : "0",
            }}>
              <SpeciesAvatar sp={idx} size={26} />
              <span style={{ fontSize: 13, fontWeight: 500 }}>{sp.common}</span>
              <DualBar now={now} prev={prev} />
              <span className="mono tabular" style={{ fontSize: 12, color: "var(--fg-2)", textAlign: "right" }}>{now}</span>
              <span className="mono tabular" style={{ fontSize: 12, fontWeight: 600, color: up ? "var(--moss-ink)" : "var(--dawn-ink)", textAlign: "right" }}>
                {up ? "+" : ""}{delta.toFixed(0)}%
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function DualBar({ now, prev }) {
  const max = Math.max(now, prev);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
      <span style={{ height: 6, background: "var(--bg-2)", borderRadius: 2, overflow: "hidden" }}>
        <span style={{ display: "block", width: `${(now / max) * 100}%`, height: "100%", background: "var(--moss)" }} />
      </span>
      <span style={{ height: 4, background: "var(--bg-2)", borderRadius: 2, overflow: "hidden" }}>
        <span style={{ display: "block", width: `${(prev / max) * 100}%`, height: "100%", background: "var(--fg-3)" }} />
      </span>
    </div>
  );
}

// ─── On This Day ─────────────────────────────────────────────────────────
function OnThisDayCard() {
  const { SPECIES } = window.BNB;
  const memories = [
    { year: "2024", date: "May 22, 2024", count: 745, top: SPECIES[1], note: "First Magnolia Warbler of last year arrived on May 21." },
    { year: "2023", date: "May 22, 2023", count: 612, top: SPECIES[3], note: "Quietest May 22 on record — heavy rain all day, only six species." },
  ];
  return (
    <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column" }}>
      <SectionHeader eyebrow="On this day" title="What happened in years past" />
      <div style={{ marginTop: 14, flex: 1, display: "flex", flexDirection: "column", gap: 14 }}>
        {memories.map((m, i) => (
          <div key={i} style={{ display: "grid", gridTemplateColumns: "60px 1fr", gap: 14, padding: "12px 0", borderTop: i > 0 ? "0.5px dashed var(--hairline)" : "0" }}>
            <div className="display tabular" style={{ fontSize: 26, lineHeight: 1, color: "var(--fg-3)" }}>{m.year}</div>
            <div>
              <div style={{ display: "flex", alignItems: "baseline", gap: 10 }}>
                <span className="display" style={{ fontSize: 22 }}>{m.count.toLocaleString()}</span>
                <span className="bnb-meta">detections</span>
              </div>
              <div style={{ display: "flex", gap: 6, alignItems: "center", marginTop: 6 }}>
                <SpeciesAvatar sp={SPECIES.indexOf(m.top)} size={22} />
                <span style={{ fontSize: 12.5, color: "var(--fg-2)" }}>Loudest: <strong style={{ color: "var(--fg)" }}>{m.top.common}</strong></span>
              </div>
              <div className="bnb-meta" style={{ marginTop: 8, fontStyle: "italic", lineHeight: 1.5 }}>"{m.note}"</div>
            </div>
          </div>
        ))}
      </div>
      <div className="bnb-meta" style={{ marginTop: "auto", paddingTop: 12, borderTop: "0.5px solid var(--hairline)" }}>
        Data goes back to your install date — Mar 12, 2024. Two full prior years available.
      </div>
    </div>
  );
}

// ─── Long-term row — sparkbar of 60+ weeks per species ──────────────────
function LongTermRow({ sp, idx }) {
  // Synthesize 60 weeks of data — yearly cycle + seasonal pattern
  const data = Array.from({ length: 64 }, (_, w) => {
    // Migrants peak in May (week 20) and Sept (week 38); residents are flat-ish
    const isMigrant = sp.rare || idx > 6;
    const spring = 14 * Math.exp(-Math.pow((w % 52 - 19) / 5, 2));
    const fall = 12 * Math.exp(-Math.pow((w % 52 - 37) / 6, 2));
    const baseline = isMigrant ? 1 : 6 + Math.sin(w * 0.3) * 2;
    return Math.max(0, baseline + (isMigrant ? spring + fall : 0) + (w * 7 % 5) * 0.3);
  });
  const max = Math.max(...data);
  return (
    <div style={{ display: "grid", gridTemplateColumns: "auto 1fr 60px", gap: 10, alignItems: "center", padding: "6px 0" }}>
      <SpeciesAvatar sp={window.BNB.SPECIES.indexOf(sp)} size={22} />
      <div>
        <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 3 }}>
          <span style={{ fontSize: 12.5, fontWeight: 500 }}>{sp.common}</span>
          <span className="bnb-meta mono">{sp.count.toLocaleString()} all-time</span>
        </div>
        <svg viewBox="0 0 320 18" width="100%" height="18" preserveAspectRatio="none">
          {data.map((v, w) => {
            const h = (v / max) * 16;
            return <rect key={w} x={(w / data.length) * 320} y={18 - h} width={320 / data.length - 0.4} height={Math.max(1, h)} fill={sp.color} fillOpacity={0.45 + (v / max) * 0.45} rx="0.5" />;
          })}
          {/* this year start indicator */}
          <line x1={(12 / data.length) * 320} y1="0" x2={(12 / data.length) * 320} y2="18" stroke="var(--border-2)" strokeWidth="0.4" />
          {/* today */}
          <line x1={(53 / data.length) * 320} y1="0" x2={(53 / data.length) * 320} y2="18" stroke="var(--fg)" strokeWidth="0.6" strokeDasharray="1 1" />
        </svg>
      </div>
      <Sparkline data={sp.trend} width={60} height={20} accent={sp.color} />
    </div>
  );
}

Object.assign(window, { Trends });
