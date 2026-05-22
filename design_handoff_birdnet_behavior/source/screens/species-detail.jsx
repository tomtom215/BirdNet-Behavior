// Species detail page — single species deep view, integrates dawn-chorus ring, hourly chart,
// companion species, Wikipedia-image-style photo slot, recent recordings.

function SpeciesDetail() {
  const { SPECIES, CHORUS, COOC, COOC_SPECIES } = window.BNB;
  const speciesIdx = 1; // Northern Cardinal as the showcase
  const sp = SPECIES[speciesIdx];

  // Hourly data — pulled from CHORUS if available, otherwise synthesize.
  const hours = CHORUS.find((c) => c.sp === speciesIdx)?.hours || sp.trend.slice(0, 24);
  const maxH = Math.max(0.001, ...hours);

  // 14-day trend — synthesize from sp.trend
  const days = [10,12,15,18,14,16,22,19,17,21,24,20,18,22];
  const maxD = Math.max(...days);

  // Companion species — top-3 by COOC where this species is in the matrix
  const myIdxInCooc = COOC_SPECIES.indexOf(speciesIdx);
  const companions = myIdxInCooc >= 0
    ? COOC[myIdxInCooc]
        .map((v, i) => ({ v, sp: COOC_SPECIES[i] }))
        .filter((x) => x.sp !== speciesIdx)
        .sort((a, b) => b.v - a.v)
        .slice(0, 4)
    : [];

  return (
    <Screen>
      <TopNav active="Species" />

      <div style={{ display: "flex", gap: 6, color: "var(--fg-3)", fontSize: 12, alignItems: "center" }}>
        <a href="#" style={{ color: "var(--fg-3)", textDecoration: "none" }}>Species</a>
        <span>›</span>
        <span style={{ color: "var(--fg-2)" }}>{sp.common}</span>
      </div>

      {/* Hero — full-bleed photo with overlay info */}
      <div className="bnb-card" style={{ overflow: "hidden", display: "grid", gridTemplateColumns: "440px 1fr", minHeight: 380 }}>
        <div style={{ position: "relative", background: "var(--surface-2)" }}>
          <BirdPhoto sp={sp} idx={speciesIdx} slotId="species-cardinal" />
          {/* Floating moment-of-detection badge */}
          <div style={{
            position: "absolute", left: 16, bottom: 16, zIndex: 3,
            background: "color-mix(in oklch, var(--bg) 90%, transparent)",
            backdropFilter: "blur(10px)", WebkitBackdropFilter: "blur(10px)",
            border: "0.5px solid var(--border-2)",
            borderRadius: 10, padding: "10px 14px",
            display: "flex", flexDirection: "column", gap: 2,
          }}>
            <div className="mono" style={{ fontSize: 10, color: "var(--fg-3)", textTransform: "uppercase", letterSpacing: "0.1em" }}>Last heard</div>
            <div className="display" style={{ fontSize: 20 }}>14 minutes ago</div>
            <div className="bnb-meta mono">06:28 · conf 0.97</div>
          </div>
        </div>
        <div style={{ padding: "var(--pad-4)", display: "flex", flexDirection: "column", justifyContent: "space-between" }}>
          <div>
            <div style={{ display: "flex", gap: 6, alignItems: "center", marginBottom: 12 }}>
              <span className="bnb-pill moss"><span className="bnb-dot live" /> currently active</span>
              <span className="bnb-pill">Year-round resident</span>
              <span className="bnb-pill mono">{sp.short}</span>
            </div>
            <h1 className="display" style={{ fontSize: 56, lineHeight: 0.98, letterSpacing: "-0.025em" }}>{sp.common}</h1>
            <div className="bnb-meta" style={{ fontStyle: "italic", marginTop: 6, fontSize: 15 }}>{sp.sci}</div>
            <p style={{ marginTop: 18, color: "var(--fg-2)", fontSize: 14, lineHeight: 1.6, maxWidth: 620, textWrap: "pretty" }}>
              A medium-sized passerine with a long tail, thick bill, and prominent crest. Males are vivid red; females reddish-olive. Lives in woodland edges, gardens, swamps, and streamside thickets — exactly the kind of yard you have.
            </p>
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: "var(--pad-2)", borderTop: "0.5px solid var(--hairline)", paddingTop: 18, marginTop: 18 }}>
            <Stat label="Today" value={sp.count} sub="↑ 14% vs. avg" size="sm" />
            <Stat label="All-time" value="4,218" sub="since Mar 12" size="sm" />
            <Stat label="First seen" value="Mar 12" sub="2024 · day 1" size="sm" />
            <Stat label="Mean conf." value={sp.conf.toFixed(2)} sub="last 60 days" size="sm" accent="var(--moss-ink)" />
          </div>
        </div>
      </div>

      {/* Three-column analytics row */}
      <div style={{ display: "grid", gridTemplateColumns: "1.3fr 1fr 1fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        {/* Hourly activity */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column" }}>
          <SectionHeader eyebrow="Today · hourly" title="When you'll hear it" action={<span className="bnb-meta mono">Peak {fmtHour2(hours.indexOf(Math.max(...hours)))}</span>} />
          <div style={{ flex: 1, display: "flex", alignItems: "flex-end", gap: 3, marginTop: 16 }}>
            {hours.map((v, h) => (
              <div key={h} style={{
                flex: 1, height: `${(v / maxH) * 100}%`,
                background: `color-mix(in oklch, ${sp.color} ${20 + (v / maxH) * 60}%, var(--surface-2))`,
                borderRadius: 2,
                minHeight: 2,
              }} title={`${h}:00 — ${v.toFixed(2)}`} />
            ))}
          </div>
          <div style={{ display: "flex", justifyContent: "space-between", marginTop: 6 }}>
            {[0, 6, 12, 18, 23].map((h) => (
              <span key={h} className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>{fmtHour2(h)}</span>
            ))}
          </div>
        </div>

        {/* 12-week activity grid */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column" }}>
          <SectionHeader eyebrow="Activity · last 12 weeks" title="Detections per day" action={<span className="bnb-pill moss">+ 22%</span>} />
          <div style={{ flex: 1, marginTop: 14, display: "flex", alignItems: "center" }}>
            <WeeklyHeatGrid accent={sp.color} />
          </div>
          <div style={{ display: "flex", justifyContent: "space-between", marginTop: 8 }}>
            <span className="bnb-meta mono">12 wk ago</span>
            <span className="bnb-meta mono">today</span>
          </div>
          <div className="bnb-meta" style={{ marginTop: 8, paddingTop: 8, borderTop: "0.5px solid var(--hairline)" }}>
            Mean 18 d⁻¹ · peak Sun May 18 (32) · longest streak 21 d
          </div>
        </div>

        {/* Companion species */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
          <SectionHeader eyebrow="Often heard with" title="Companions" />
          <div style={{ marginTop: 8 }}>
            {companions.map((c, i) => {
              const cs = SPECIES[c.sp];
              return (
                <div key={i} style={{ display: "grid", gridTemplateColumns: "auto 1fr auto", alignItems: "center", gap: 10, padding: "10px 0", borderTop: i > 0 ? "0.5px solid var(--hairline)" : "0" }}>
                  <SpeciesAvatar sp={c.sp} size={26} />
                  <div style={{ minWidth: 0 }}>
                    <div style={{ fontSize: 13, fontWeight: 500, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{cs.common}</div>
                    <div className="bnb-meta mono">ρ {c.v.toFixed(2)}</div>
                  </div>
                  <span style={{ width: 60, height: 4, background: "var(--bg-2)", borderRadius: 2, overflow: "hidden" }}>
                    <span style={{ display: "block", width: `${c.v * 100}%`, height: "100%", background: cs.color }} />
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {/* Recordings strip */}
      <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
        <SectionHeader
          eyebrow="Recordings"
          title="Recent clips"
          action={<div style={{ display: "flex", gap: 6 }}>
            <span className="bnb-pill">All confidence</span>
            <span className="bnb-pill">Today</span>
            <button className="bnb-btn">Export</button>
          </div>}
        />
        <div style={{ display: "grid", gridTemplateColumns: "repeat(5, 1fr)", gap: "var(--pad-2)", marginTop: 12 }}>
          {[
            { time: "06:14", conf: 0.97 },
            { time: "06:42", conf: 0.94 },
            { time: "07:02", conf: 0.96 },
            { time: "11:58", conf: 0.89 },
            { time: "16:21", conf: 0.93 },
          ].map((r, i) => (
            <ClipCard key={i} time={r.time} conf={r.conf} accent={sp.color} />
          ))}
        </div>
      </div>
    </Screen>
  );
}

function fmtHour2(h) {
  if (h === 0) return "12 a";
  if (h === 12) return "12 p";
  return h < 12 ? `${h} a` : `${h - 12} p`;
}

function WeeklyHeatGrid({ accent }) {
  // 12 weeks × 7 days, deterministic synthetic data
  let s = 17;
  const r = () => { s = (s * 9301 + 49297) % 233280; return s / 233280; };
  const weeks = [];
  for (let w = 0; w < 12; w++) {
    const col = [];
    for (let d = 0; d < 7; d++) {
      // ramp up over time + dawn-day variation
      const base = (w / 12) * 0.7 + 0.25;
      const dayBoost = (d === 0 || d === 6) ? 0.5 : 1.0;
      col.push(Math.max(0, base * dayBoost * (0.4 + r() * 1.4)));
    }
    weeks.push(col);
  }
  const max = Math.max(...weeks.flat());
  return (
    <div style={{ display: "flex", gap: 8, alignItems: "center", width: "100%" }}>
      <div style={{ display: "flex", flexDirection: "column", justifyContent: "space-around", height: 100 }}>
        {["M", "W", "F"].map((d) => (
          <span key={d} className="mono" style={{ fontSize: 9.5, color: "var(--fg-3)" }}>{d}</span>
        ))}
      </div>
      <div style={{ flex: 1, display: "grid", gridTemplateColumns: "repeat(12, 1fr)", gap: 3 }}>
        {weeks.map((col, wi) => (
          <div key={wi} style={{ display: "grid", gridTemplateRows: "repeat(7, 1fr)", gap: 3, height: 100 }}>
            {col.map((v, di) => {
              const op = v / max;
              return (
                <div key={di} style={{
                  background: op < 0.05 ? "var(--surface-2)" : `color-mix(in oklch, ${accent} ${Math.min(85, op * 90)}%, var(--surface-2))`,
                  borderRadius: 2,
                  border: wi === 11 && di === 4 ? "1px solid var(--fg)" : "none",
                }} title={`week ${wi + 1}, day ${di} — ${(v * 10).toFixed(0)}`} />
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}

function BigSparkline({ data, accent }) {
  const W = 280, H = 96;
  const max = Math.max(1, ...data);
  const stepX = W / (data.length - 1);
  const pts = data.map((v, i) => [i * stepX, H - 4 - (v / max) * (H - 10)]);
  const path = pts.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
  const area = `${path} L${W},${H} L0,${H} Z`;
  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" height="100%" preserveAspectRatio="none">
      <path d={area} fill={accent} fillOpacity={0.14} />
      <path d={path} stroke={accent} fill="none" strokeWidth={1.6} />
      {pts.map(([x, y], i) => (
        <circle key={i} cx={x} cy={y} r={1.8} fill={accent} />
      ))}
    </svg>
  );
}

function ClipCard({ time, conf, accent }) {
  return (
    <div style={{ border: "0.5px solid var(--border)", borderRadius: 8, padding: 10, background: "var(--surface-2)" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <span className="mono" style={{ fontSize: 11 }}>{time}</span>
        <ConfBar value={conf} width={42} />
      </div>
      <div style={{ height: 40, marginTop: 8, display: "flex", alignItems: "center", gap: 1.5 }}>
        {Array.from({ length: 36 }).map((_, i) => {
          const env = Math.sin((i / 36) * Math.PI);
          const v = 0.2 + env * (0.55 + Math.sin(i + time.length) * 0.35);
          return <span key={i} style={{ width: 2, height: `${Math.round(v * 36)}px`, background: `color-mix(in oklch, ${accent} ${30 + v * 60}%, var(--fg-4))`, borderRadius: 1 }} />;
        })}
      </div>
      <div style={{ display: "flex", justifyContent: "space-between", marginTop: 8 }}>
        <button className="bnb-btn ghost" style={{ padding: "2px 0", fontSize: 11 }}>▶  Play</button>
        <span className="bnb-meta mono">1.4s</span>
      </div>
    </div>
  );
}

Object.assign(window, { SpeciesDetail });
