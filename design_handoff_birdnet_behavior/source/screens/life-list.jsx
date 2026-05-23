// Life list — birding journal with year tape, photo strip, journal notes.
function LifeList() {
  const { LIFE_LIST, SPECIES } = window.BNB;

  const byMonth = {};
  for (const entry of LIFE_LIST) {
    const d = new Date(entry.first);
    const key = `${d.toLocaleString("en-US", { month: "long" })} ${d.getFullYear()}`;
    (byMonth[key] = byMonth[key] || []).push(entry);
  }

  return (
    <Screen>
      <TopNav active="Life list" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Journal · life list</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Every species, once</h2>
          <div className="bnb-meta" style={{ marginTop: 6 }}>15 lifers since March 12, 2024 · 437 days listening · 14 months on station</div>
        </div>
        <div style={{ display: "flex", gap: 10, alignItems: "center" }}>
          <Stat label="Lifers" value="15" size="sm" accent="var(--moss-ink)" />
          <span className="bnb-vrule" style={{ height: 28, alignSelf: "center" }} />
          <Stat label="Rare" value="3" size="sm" accent="var(--rare)" />
          <span className="bnb-vrule" style={{ height: 28, alignSelf: "center" }} />
          <Stat label="Median gap" value="1.4 d" size="sm" />
          <span className="bnb-vrule" style={{ height: 28, alignSelf: "center" }} />
          <button className="bnb-btn">Print journal</button>
        </div>
      </div>

      {/* Year Tape — every lifer as a dot on a year-long timeline */}
      <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
        <SectionHeader eyebrow="The year tape" title="When each lifer arrived" action={<MonthCounts entries={LIFE_LIST} />} />
        <YearTape entries={LIFE_LIST} />
      </div>

      {/* Two-column journal + sidebar */}
      <div style={{ display: "grid", gridTemplateColumns: "1.6fr 1fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        {/* Tight journal entries */}
        <div className="bnb-card" style={{ padding: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ padding: "var(--pad-3) var(--pad-3) 8px", display: "flex", justifyContent: "space-between", alignItems: "baseline" }}>
            <SectionHeader eyebrow="Chronological" title="The journal" />
            <div style={{ display: "flex", gap: 6 }}>
              <span className="bnb-pill">All</span>
              <span className="bnb-pill">With notes</span>
              <span className="bnb-pill">Rare</span>
            </div>
          </div>
          <div style={{ flex: 1, overflow: "hidden", padding: "0 var(--pad-3) var(--pad-3)" }}>
            {Object.entries(byMonth).map(([month, entries]) => (
              <div key={month}>
                <div style={{ display: "flex", alignItems: "baseline", gap: 10, padding: "10px 0 6px", position: "sticky", top: 0, background: "var(--surface)" }}>
                  <h3 className="display" style={{ fontSize: 14, color: "var(--fg-2)" }}>{month}</h3>
                  <span className="bnb-meta mono">{entries.length}</span>
                  <span style={{ flex: 1, height: "0.5px", background: "var(--hairline)" }} />
                </div>
                {entries.map((e, i) => {
                  const sp = SPECIES[e.sp];
                  return (
                    <div key={i} style={{
                      display: "grid",
                      gridTemplateColumns: "48px 36px 1fr auto",
                      gap: 12,
                      padding: "8px 0",
                      alignItems: e.note ? "flex-start" : "center",
                      borderTop: "0.5px dashed var(--hairline)",
                    }}>
                      <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)", paddingTop: e.note ? 4 : 0 }}>{e.first.slice(5)}</span>
                      <SpeciesAvatar sp={e.sp} size={28} />
                      <div style={{ minWidth: 0 }}>
                        <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
                          <span style={{ fontWeight: 500, fontSize: 13.5 }}>{sp.common}</span>
                          <span className="bnb-meta mono" style={{ fontStyle: "italic" }}>{sp.sci}</span>
                          {sp.rare && <span className="bnb-pill rare" style={{ fontSize: 9.5, padding: "1px 6px" }}>rare</span>}
                        </div>
                        {e.note && (
                          <div className="display" style={{ fontSize: 13.5, marginTop: 3, color: "var(--fg-2)", lineHeight: 1.4, fontStyle: "italic" }}>
                            "{e.note}"
                          </div>
                        )}
                      </div>
                      <Sparkline data={sp.trend} width={56} height={18} accent={sp.color} />
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
        </div>

        {/* Right sidebar */}
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--pad-3)", minHeight: 0 }}>
          {/* Photo strip — let the user drop one hero lifer photo */}
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <SectionHeader eyebrow="Latest lifer" title="Cedar Waxwing · Nov 22" />
            <image-slot
              id="lifelist-latest"
              shape="rect"
              placeholder="Drop a Cedar Waxwing photo"
              style={{ display: "block", width: "100%", height: 140, marginTop: 12, borderRadius: 10, overflow: "hidden" }}
            ></image-slot>
            <div className="bnb-meta" style={{ marginTop: 8, fontStyle: "italic" }}>
              "Fall foraging flock. Eight or nine of them moving through the dogwoods."
            </div>
          </div>

          <div className="bnb-card" style={{ padding: "var(--pad-3)", flex: 1, display: "flex", flexDirection: "column" }}>
            <SectionHeader eyebrow="Still expected" title="Likely next" />
            <div style={{ marginTop: 8, flex: 1 }}>
              {[
                { name: "Ruby-throated Hummingbird", short: "RTHU", prob: 0.86, color: "oklch(55% 0.18 25)" },
                { name: "Indigo Bunting",            short: "INBU", prob: 0.74, color: "oklch(58% 0.14 240)" },
                { name: "Common Yellowthroat",       short: "COYE", prob: 0.62, color: "oklch(72% 0.14 95)" },
                { name: "Eastern Wood-Pewee",        short: "EAWP", prob: 0.55, color: "oklch(50% 0.05 60)" },
                { name: "Scarlet Tanager",           short: "SCTA", prob: 0.41, color: "oklch(55% 0.20 25)" },
              ].map((p, i) => (
                <div key={i} style={{ display: "grid", gridTemplateColumns: "30px 1fr 80px auto", gap: 10, padding: "8px 0", borderTop: i > 0 ? "0.5px solid var(--hairline)" : "0", alignItems: "center" }}>
                  <span className="mono" style={{ width: 30, height: 30, borderRadius: "50%", display: "inline-flex", alignItems: "center", justifyContent: "center", background: `color-mix(in oklch, ${p.color} 22%, var(--surface))`, color: p.color, fontSize: 10, fontWeight: 600 }}>{p.short}</span>
                  <span style={{ fontSize: 13 }}>{p.name}</span>
                  <span style={{ width: 80, height: 4, background: "var(--bg-2)", borderRadius: 2, overflow: "hidden" }}>
                    <span style={{ display: "block", width: `${p.prob * 100}%`, height: "100%", background: p.color }} />
                  </span>
                  <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)" }}>{Math.round(p.prob * 100)}%</span>
                </div>
              ))}
            </div>
            <div className="bnb-meta" style={{ marginTop: 8, paddingTop: 8, borderTop: "0.5px solid var(--hairline)" }}>
              Probabilities from eBird regional baseline · 100 km radius.
            </div>
          </div>
        </div>
      </div>
    </Screen>
  );
}

function MonthCounts({ entries }) {
  const counts = {};
  entries.forEach((e) => {
    const m = new Date(e.first).getMonth();
    counts[m] = (counts[m] || 0) + 1;
  });
  return (
    <div style={{ display: "flex", gap: 4, alignItems: "flex-end", height: 28 }}>
      {Array.from({ length: 12 }).map((_, m) => {
        const v = counts[m] || 0;
        return (
          <div key={m} style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 2, width: 22 }}>
            <span className="mono" style={{ fontSize: 9, color: v > 0 ? "var(--moss-ink)" : "var(--fg-4)", fontWeight: v > 0 ? 600 : 400, lineHeight: 1 }}>{v || ""}</span>
            <span style={{ width: 18, height: 4, background: v > 0 ? "var(--moss)" : "var(--bg-2)", opacity: v > 0 ? 0.30 + Math.min(0.6, v * 0.20) : 1, borderRadius: 2 }} />
            <span className="mono" style={{ fontSize: 8.5, color: "var(--fg-4)" }}>{["J","F","M","A","M","J","J","A","S","O","N","D"][m]}</span>
          </div>
        );
      })}
    </div>
  );
}

function YearTape({ entries }) {
  const { SPECIES } = window.BNB;
  const W = 1380, H = 130;
  const padL = 20, padR = 20, padT = 16, padB = 30;
  const innerW = W - padL - padR;

  // Compute "day of year" for each entry
  const dots = entries.map((e) => {
    const d = new Date(e.first);
    const start = new Date(d.getFullYear(), 0, 1);
    const doy = (d - start) / (1000 * 60 * 60 * 24);
    const sp = SPECIES[e.sp];
    const stem = 30 + (sp.count / 200) * 50;
    return { doy, sp, stem, rare: sp.rare, note: e.note };
  });

  const x = (doy) => padL + (doy / 365) * innerW;

  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} style={{ marginTop: 10 }}>
      {/* baseline */}
      <line x1={padL} y1={H - padB} x2={W - padR} y2={H - padB} stroke="var(--border-2)" strokeWidth={0.8} />
      {/* Month markers */}
      {Array.from({ length: 12 }).map((_, m) => {
        const d = new Date(2025, m, 1);
        const start = new Date(2025, 0, 1);
        const doy = (d - start) / (1000 * 60 * 60 * 24);
        const mx = x(doy);
        return (
          <g key={m}>
            <line x1={mx} y1={H - padB - 4} x2={mx} y2={H - padB + 4} stroke="var(--fg-3)" strokeWidth={0.6} />
            <text x={mx} y={H - 8} className="mono" textAnchor="middle" style={{ fontSize: 10, fill: "var(--fg-3)" }}>{d.toLocaleString("en-US", { month: "short" })}</text>
          </g>
        );
      })}
      {/* Season bands */}
      <rect x={x(60)} y={padT} width={x(135) - x(60)} height={H - padB - padT} fill="var(--moss-soft)" fillOpacity="0.35" />
      <text x={x(98)} y={padT + 12} textAnchor="middle" className="mono" style={{ fontSize: 10, fill: "var(--moss-ink)" }}>spring migration</text>
      <rect x={x(240)} y={padT} width={x(310) - x(240)} height={H - padB - padT} fill="var(--dawn-soft)" fillOpacity="0.45" />
      <text x={x(275)} y={padT + 12} textAnchor="middle" className="mono" style={{ fontSize: 10, fill: "var(--dawn-ink)" }}>fall migration</text>

      {/* Stems + dots */}
      {dots.map((dot, i) => (
        <g key={i}>
          <line x1={x(dot.doy)} y1={H - padB} x2={x(dot.doy)} y2={H - padB - dot.stem} stroke={dot.sp.color} strokeWidth={1} strokeOpacity={0.5} />
          <circle cx={x(dot.doy)} cy={H - padB - dot.stem} r={dot.rare ? 5 : 4} fill={dot.sp.color} stroke="var(--surface)" strokeWidth={1.5} />
          {dot.rare && <circle cx={x(dot.doy)} cy={H - padB - dot.stem} r={9} fill="none" stroke={dot.sp.color} strokeWidth={0.8} strokeOpacity={0.5} />}
        </g>
      ))}

      {/* Today line */}
      <line x1={x(142)} y1={padT} x2={x(142)} y2={H - padB} stroke="var(--fg)" strokeWidth={1} strokeDasharray="3 3" />
      <text x={x(142)} y={padT + 6} textAnchor="middle" className="mono" style={{ fontSize: 10, fill: "var(--fg)" }}>today</text>
    </svg>
  );
}

Object.assign(window, { LifeList });
