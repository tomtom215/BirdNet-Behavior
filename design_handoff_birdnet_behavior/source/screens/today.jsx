// Today's detections — searchable, filterable, paginated list with inline play.
function TodayList() {
  const { SPECIES } = window.BNB;
  // Build a long list of fake detections for today
  const rows = [];
  const baseTime = ["06:14:02", "06:14:38", "06:15:21", "06:17:55", "06:19:02", "06:22:11",
                    "06:24:48", "06:27:33", "06:31:09", "06:38:42", "06:41:17", "06:48:55",
                    "07:02:14", "07:08:39", "07:14:01", "07:22:48", "07:31:09", "07:42:55",
                    "08:01:22", "08:14:08", "08:22:11", "08:38:54", "09:01:17", "09:14:33"];
  const pool = [1, 0, 3, 2, 5, 6, 1, 4, 7, 3, 0, 12, 1, 9, 2, 5, 3, 0, 1, 6, 0, 14, 3, 10];
  for (let i = 0; i < baseTime.length; i++) {
    rows.push({
      id: i,
      time: baseTime[i],
      sp: pool[i],
      conf: 0.78 + ((i * 13) % 22) / 100,
      lat: 1.0 + (i % 4) * 0.3,
      rare: SPECIES[pool[i]].rare && (i % 4 === 0),
    });
  }

  return (
    <Screen>
      <TopNav active="Today" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Detection log · today</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Thursday, May 22</h2>
          <div className="bnb-meta" style={{ marginTop: 6 }}>14h 22m of listening · {rows.length} detections shown · 15 species today</div>
        </div>
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <div style={{ position: "relative" }}>
            <input
              type="text"
              placeholder="Search species, time…"
              defaultValue=""
              style={{
                background: "var(--surface)", border: "0.5px solid var(--border-2)",
                borderRadius: 8, padding: "6px 10px 6px 28px", fontSize: 13, color: "var(--fg)",
                width: 220, fontFamily: "var(--font-ui)",
              }}
            />
            <span style={{ position: "absolute", left: 9, top: "50%", transform: "translateY(-50%)", color: "var(--fg-3)" }}>⌕</span>
          </div>
          <span className="bnb-pill">≥ 0.80</span>
          <span className="bnb-pill">All species</span>
          <span className="bnb-pill">Range: today ▾</span>
          <button className="bnb-btn">Export CSV</button>
        </div>
      </div>

      {/* Day strip — every detection as a dot on the 24-hour timeline */}
      <DayStrip rows={rows} />

      <div className="bnb-card flush" style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", borderTop: "0.5px solid var(--border)" }}>
        {/* Header row */}
        <div style={{
          display: "grid",
          gridTemplateColumns: "78px 40px 1.8fr 1fr 100px 110px 110px",
          alignItems: "center", gap: 12, padding: "10px var(--pad-3)",
          borderBottom: "0.5px solid var(--hairline)",
          background: "var(--surface-2)",
        }}>
          <span className="bnb-eyebrow">Time</span>
          <span />
          <span className="bnb-eyebrow">Species</span>
          <span className="bnb-eyebrow">Spectrogram</span>
          <span className="bnb-eyebrow">Confidence</span>
          <span className="bnb-eyebrow">Clip</span>
          <span className="bnb-eyebrow" style={{ textAlign: "right" }}>Actions</span>
        </div>
        <div style={{ flex: 1, overflow: "hidden" }}>
          {rows.map((d, i) => {
            const sp = SPECIES[d.sp];
            return (
              <div key={d.id} style={{
                display: "grid",
                gridTemplateColumns: "78px 40px 1.8fr 1fr 100px 110px 110px",
                alignItems: "center", gap: 12, padding: "10px var(--pad-3)",
                borderBottom: "0.5px solid var(--hairline)",
                background: i % 2 === 0 ? "var(--surface)" : "var(--bg-2)",
              }}>
                <span className="mono" style={{ fontSize: 12, color: "var(--fg-2)" }}>{d.time}</span>
                <SpeciesAvatar sp={d.sp} size={28} />
                <div style={{ minWidth: 0 }}>
                  <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                    <span style={{ fontWeight: 500, fontSize: 13.5 }}>{sp.common}</span>
                    {d.rare && <span className="bnb-pill rare">rare</span>}
                  </div>
                  <div className="bnb-meta mono" style={{ fontStyle: "italic" }}>{sp.sci}</div>
                </div>
                <MiniSpecRow seed={d.id + 100} />
                <ConfBar value={d.conf} width={56} />
                <button className="bnb-btn ghost" style={{ fontSize: 11.5 }}>▶  {d.lat.toFixed(1)}s</button>
                <div style={{ display: "flex", gap: 4, justifyContent: "flex-end" }}>
                  <button className="bnb-btn ghost" title="Lock">🔒</button>
                  <button className="bnb-btn ghost" title="Re-label">✎</button>
                  <button className="bnb-btn ghost" title="Delete">×</button>
                </div>
              </div>
            );
          })}
        </div>
        <div style={{ display: "flex", justifyContent: "space-between", padding: "10px var(--pad-3)", borderTop: "0.5px solid var(--border)" }}>
          <span className="bnb-meta">Showing 1–{rows.length} of 1,247</span>
          <div style={{ display: "flex", gap: 4 }}>
            <button className="bnb-btn ghost">‹ Prev</button>
            {[1,2,3,4].map((p) => (
              <button key={p} className={`bnb-btn ${p === 1 ? "primary" : "ghost"}`} style={{ minWidth: 28, justifyContent: "center" }}>{p}</button>
            ))}
            <span className="bnb-meta" style={{ alignSelf: "center", padding: "0 4px" }}>…</span>
            <button className="bnb-btn ghost">52</button>
            <button className="bnb-btn ghost">Next ›</button>
          </div>
        </div>
      </div>
    </Screen>
  );
}

function MiniSpecRow({ seed = 1 }) {
  // tiny static spectrogram strip
  const W = 140, H = 30;
  let s = (Number(seed) || 1);
  const r = () => { s = (s * 9301 + 49297) % 233280; return s / 233280; };
  const cells = [];
  for (let x = 0; x < W; x += 2) {
    for (let y = 0; y < H; y += 2) {
      const env = Math.exp(-Math.pow((x - W/2) / (W * 0.25), 2));
      const v = r() * 0.35 + env * (0.45 + r() * 0.4);
      if (v > 0.35) cells.push({ x, y, op: Math.min(0.9, v) });
    }
  }
  return (
    <svg width={W} height={H} viewBox={`0 0 ${W} ${H}`} style={{ background: "var(--surface-2)", borderRadius: 4 }} aria-hidden="true">
      {cells.map((c, i) => (
        <rect key={i} x={c.x} y={c.y} width="2" height="2" fill="var(--moss-ink)" fillOpacity={c.op} />
      ))}
    </svg>
  );
}

Object.assign(window, { TodayList });

// ─── DayStrip — 24h timeline with every detection as a colored dot ────────
function DayStrip({ rows }) {
  const { SPECIES } = window.BNB;
  // Convert HH:MM:SS to hour float
  const dots = rows.map((r) => {
    const [h, m, s] = r.time.split(":").map(Number);
    return { ...r, hour: h + m / 60 + s / 3600 };
  });

  // Histogram per hour
  const histogram = Array.from({ length: 24 }, (_, h) => dots.filter((d) => Math.floor(d.hour) === h).length);
  const maxHist = Math.max(1, ...histogram);

  return (
    <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: 10 }}>
        <div>
          <div className="bnb-eyebrow">Detection density · 24 hours</div>
          <div style={{ display: "flex", gap: 6, marginTop: 4 }}>
            <span className="bnb-meta">Each dot = one detection · colored by species</span>
          </div>
        </div>
        <div style={{ display: "flex", gap: 18, alignItems: "center" }}>
          <span className="bnb-meta"><span className="display" style={{ fontSize: 24, color: "var(--fg)" }}>06:47</span> peak hour</span>
          <span className="bnb-meta"><span className="display" style={{ fontSize: 24, color: "var(--moss-ink)" }}>34</span> in dawn chorus</span>
          <span className="bnb-meta"><span className="display" style={{ fontSize: 24, color: "var(--rare)" }}>1</span> rare</span>
        </div>
      </div>
      <svg viewBox="0 0 1380 120" width="100%" height="120" preserveAspectRatio="none">
        {/* sunrise / sunset bands */}
        <rect x={(0 / 24) * 1380} y="0" width={(5.35 / 24) * 1380} height="120" fill="var(--night)" fillOpacity="0.05" />
        <rect x={(20.13 / 24) * 1380} y="0" width={((24 - 20.13) / 24) * 1380} height="120" fill="var(--night)" fillOpacity="0.05" />

        {/* histogram bars */}
        {histogram.map((v, h) => (
          <rect
            key={h}
            x={(h / 24) * 1380 + 2}
            y={92 - (v / maxHist) * 78}
            width={1380 / 24 - 4}
            height={(v / maxHist) * 78}
            fill="var(--moss-soft)"
            rx="2"
          />
        ))}

        {/* hour grid */}
        {[0, 6, 12, 18].map((h) => (
          <g key={h}>
            <line x1={(h / 24) * 1380} y1="6" x2={(h / 24) * 1380} y2="100" stroke="var(--hairline)" />
            <text x={(h / 24) * 1380 + 6} y="116" className="mono" style={{ fontSize: 11, fill: "var(--fg-3)" }}>
              {h === 0 ? "midnight" : h === 12 ? "noon" : h < 12 ? `${h} a.m.` : `${h - 12} p.m.`}
            </text>
          </g>
        ))}

        {/* sunrise/sunset markers */}
        <g>
          <line x1={(5.35 / 24) * 1380} y1="6" x2={(5.35 / 24) * 1380} y2="100" stroke="var(--dawn-ink)" strokeDasharray="2 3" />
          <text x={(5.35 / 24) * 1380 + 6} y="14" className="mono" style={{ fontSize: 10, fill: "var(--dawn-ink)" }}>☼ 5:21</text>
        </g>
        <g>
          <line x1={(20.13 / 24) * 1380} y1="6" x2={(20.13 / 24) * 1380} y2="100" stroke="var(--dawn-ink)" strokeDasharray="2 3" />
          <text x={(20.13 / 24) * 1380 - 6} y="14" textAnchor="end" className="mono" style={{ fontSize: 10, fill: "var(--dawn-ink)" }}>☾ 20:08</text>
        </g>

        {/* dots */}
        {dots.map((d, i) => {
          const sp = SPECIES[d.sp];
          const x = (d.hour / 24) * 1380;
          const y = 50 + ((i * 13.7) % 38) - 19; // gentle vertical scatter so they don't all stack
          return (
            <g key={d.id}>
              {d.rare && <circle cx={x} cy={y} r="7" fill="none" stroke={sp.color} strokeWidth="0.8" opacity="0.5" />}
              <circle cx={x} cy={y} r={d.rare ? 4 : 3.2} fill={sp.color} fillOpacity={0.55 + d.conf * 0.4} stroke="var(--surface)" strokeWidth="0.8" />
            </g>
          );
        })}

        {/* now line */}
        <line x1={(9.4 / 24) * 1380} y1="0" x2={(9.4 / 24) * 1380} y2="100" stroke="var(--fg)" strokeWidth="1" strokeDasharray="3 3" />
        <rect x={(9.4 / 24) * 1380 - 22} y="0" width="44" height="14" rx="3" fill="var(--fg)" />
        <text x={(9.4 / 24) * 1380} y="10" textAnchor="middle" className="mono" style={{ fontSize: 10, fill: "var(--bg)" }}>now</text>
      </svg>
    </div>
  );
}
