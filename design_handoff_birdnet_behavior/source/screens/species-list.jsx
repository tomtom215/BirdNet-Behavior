// Species list — searchable, sortable browse view for all detected species.
// Designed to scale from 15 (hobbyist) to 500+ (research station).

const { useState: useState_sl } = React;

function SpeciesList() {
  const { SPECIES } = window.BNB;
  const [sort, setSort] = useState_sl("count");
  const [filter, setFilter] = useState_sl("all");

  let rows = [...SPECIES];
  if (filter === "rare") rows = rows.filter((s) => s.rare);
  if (filter === "today") rows = rows.filter((s) => s.count > 0);
  if (sort === "count") rows.sort((a, b) => b.count - a.count);
  else if (sort === "alpha") rows.sort((a, b) => a.common.localeCompare(b.common));
  else if (sort === "first") rows.reverse();

  return (
    <Screen>
      <TopNav active="Species" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Browse · all detected species</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Every voice your yard has known</h2>
          <div className="bnb-meta" style={{ marginTop: 6 }}>{SPECIES.length} species · {SPECIES.reduce((a, b) => a + b.count, 0).toLocaleString()} detections across 437 listening days</div>
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <div style={{ position: "relative" }}>
            <input
              type="text"
              placeholder="Search common, sci, or 4-letter code…"
              style={{
                background: "var(--surface)", border: "0.5px solid var(--border-2)",
                borderRadius: 8, padding: "7px 12px 7px 30px", fontSize: 13, color: "var(--fg)",
                width: 280, fontFamily: "var(--font-ui)",
              }}
            />
            <span style={{ position: "absolute", left: 10, top: "50%", transform: "translateY(-50%)", color: "var(--fg-3)" }}>⌕</span>
          </div>
          <SortPicker value={sort} onChange={setSort} />
          <FilterPills value={filter} onChange={setFilter} />
          <button className="bnb-btn">Export CSV</button>
        </div>
      </div>

      {/* Stats strip */}
      <div style={{ display: "grid", gridTemplateColumns: "repeat(5, 1fr)", gap: 0, border: "0.5px solid var(--border)", borderRadius: 12, overflow: "hidden", background: "var(--surface)" }}>
        <StripStat label="Total species"   value={SPECIES.length} sub="ever heard" />
        <StripStat label="Active today"    value={9}  sub="of 15 today"  accent="var(--moss-ink)" />
        <StripStat label="First-of-year"   value={6}  sub="this year"    accent="var(--dawn-ink)" />
        <StripStat label="Rare"            value={3}  sub="all reviewed" accent="var(--rare)" />
        <StripStat label="Median conf"     value="0.91" sub="last 60 d" last />
      </div>

      {/* Table */}
      <div className="bnb-card flush" style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", border: "0.5px solid var(--border)", borderRadius: 12, overflow: "hidden" }}>
        <div style={{
          display: "grid",
          gridTemplateColumns: "40px 40px 1.8fr 84px 200px 80px 110px 120px 90px",
          gap: 12, alignItems: "center", padding: "10px var(--pad-3)",
          borderBottom: "0.5px solid var(--hairline)",
          background: "var(--surface-2)", flex: "0 0 auto",
        }}>
          <span className="bnb-eyebrow" />
          <span className="bnb-eyebrow" />
          <span className="bnb-eyebrow">Species</span>
          <span className="bnb-eyebrow" style={{ textAlign: "right" }}>All-time</span>
          <span className="bnb-eyebrow">14-day trend</span>
          <span className="bnb-eyebrow" style={{ textAlign: "center" }}>Conf</span>
          <span className="bnb-eyebrow">First seen</span>
          <span className="bnb-eyebrow">Last heard</span>
          <span className="bnb-eyebrow" style={{ textAlign: "right" }}>Status</span>
        </div>
        <div style={{ flex: 1, overflow: "hidden" }}>
          {rows.map((s, i) => {
            const idx = SPECIES.indexOf(s);
            const lastHeard = i === 0 ? "14 min ago" : i === 1 ? "27 min ago" : i < 6 ? `${i * 23 + 9} min ago` : `${i} d ago`;
            const firstSeen = `Mar ${12 + i}`;
            const isLive = i < 4;
            const status = s.rare ? "rare" : isLive ? "active" : "—";
            return (
              <div key={idx} style={{
                display: "grid",
                gridTemplateColumns: "40px 40px 1.8fr 84px 200px 80px 110px 120px 90px",
                gap: 12, alignItems: "center", padding: "12px var(--pad-3)",
                borderBottom: "0.5px solid var(--hairline)",
                background: i % 2 === 0 ? "var(--surface)" : "var(--bg-2)",
              }}>
                <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)", textAlign: "right" }}>{i + 1}</span>
                <SpeciesAvatar sp={idx} size={32} />
                <div style={{ minWidth: 0 }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span style={{ fontSize: 14, fontWeight: 500 }}>{s.common}</span>
                    {s.rare && <span className="bnb-pill rare" style={{ fontSize: 9.5 }}>rare</span>}
                  </div>
                  <div className="bnb-meta mono" style={{ fontStyle: "italic" }}>{s.sci}</div>
                </div>
                <span className="mono tabular" style={{ fontSize: 14, color: "var(--fg)", textAlign: "right" }}>{s.count.toLocaleString()}</span>
                <Sparkline data={s.trend} width={200} height={24} accent={s.color} />
                <ConfBar value={s.conf} width={48} />
                <span className="bnb-meta mono">{firstSeen}</span>
                <span className="bnb-meta mono" style={{ color: isLive ? "var(--moss-ink)" : "var(--fg-3)" }}>{lastHeard}</span>
                <span style={{ textAlign: "right" }}>
                  {isLive && <span className="bnb-pill moss" style={{ fontSize: 10 }}><span className="bnb-dot live" /> active</span>}
                  {!isLive && s.rare && <span className="bnb-pill rare" style={{ fontSize: 10 }}>rare</span>}
                  {!isLive && !s.rare && <span className="bnb-meta mono">—</span>}
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </Screen>
  );
}

function StripStat({ label, value, sub, accent, last }) {
  return (
    <div style={{ padding: "var(--pad-3)", borderRight: last ? "none" : "0.5px solid var(--hairline)" }}>
      <div className="bnb-eyebrow">{label}</div>
      <div className="display tabular" style={{ fontSize: 30, lineHeight: 1, marginTop: 6, color: accent || "var(--fg)" }}>{value}</div>
      <div className="bnb-meta mono" style={{ marginTop: 4 }}>{sub}</div>
    </div>
  );
}

function SortPicker({ value, onChange }) {
  const options = [
    { id: "count", label: "Most heard" },
    { id: "alpha", label: "A→Z" },
    { id: "first", label: "Newest" },
  ];
  return (
    <div style={{ display: "flex", gap: 2, background: "var(--bg-2)", padding: 2, borderRadius: 7 }}>
      {options.map((o) => (
        <button key={o.id} onClick={() => onChange(o.id)} style={{
          padding: "5px 10px", borderRadius: 5,
          background: value === o.id ? "var(--surface)" : "transparent",
          color: value === o.id ? "var(--fg)" : "var(--fg-3)",
          fontSize: 12, border: 0, cursor: "pointer",
          fontWeight: value === o.id ? 500 : 400,
          boxShadow: value === o.id ? "var(--shadow-sm)" : "none",
        }}>{o.label}</button>
      ))}
    </div>
  );
}

function FilterPills({ value, onChange }) {
  return (
    <div style={{ display: "flex", gap: 4 }}>
      {[
        { id: "all",   label: "All" },
        { id: "today", label: "Today" },
        { id: "rare",  label: "Rare" },
      ].map((o) => (
        <button key={o.id} onClick={() => onChange(o.id)} className="bnb-pill" style={{
          background: value === o.id ? "var(--fg)" : "var(--surface)",
          color: value === o.id ? "var(--bg)" : "var(--fg-2)",
          border: 0, cursor: "pointer",
        }}>{o.label}</button>
      ))}
    </div>
  );
}

Object.assign(window, { SpeciesList });
