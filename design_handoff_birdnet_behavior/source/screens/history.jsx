// History — date browser with calendar view + per-day breakdown.

const { useState: useState_h } = React;

function History() {
  const [date, setDate] = useState_h("2025-05-22");

  // Generate 8 weeks of calendar data, each day with a count.
  const calendar = [];
  let s = 17;
  const rand = () => { s = (s * 9301 + 49297) % 233280; return s / 233280; };
  const today = new Date("2025-05-22");
  for (let w = 0; w < 8; w++) {
    const week = [];
    for (let d = 0; d < 7; d++) {
      const offset = (7 - w) * 7 + d - today.getDay();
      const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() - offset);
      const count = 200 + Math.round(rand() * 800);
      week.push({ date, count, species: 8 + Math.round(rand() * 14) });
    }
    calendar.push(week);
  }
  const flat = calendar.flat();
  const maxCount = Math.max(...flat.map((d) => d.count));

  return (
    <Screen>
      <TopNav active="History" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Detection history · date browser</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Pick a day</h2>
          <div className="bnb-meta" style={{ marginTop: 6 }}>56 days of listening · 38,420 detections · darkest tiles = busiest days</div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <button className="bnb-btn">‹</button>
          <span className="bnb-pill mono">Mar 28 — May 22 · 2025</span>
          <button className="bnb-btn">›</button>
          <button className="bnb-btn">Jump to today</button>
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1.4fr 1fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        {/* Calendar */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column" }}>
          <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 12 }}>
            <SectionHeader eyebrow="Last 8 weeks" title="Calendar" />
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span className="bnb-meta">quiet</span>
              {[0, 0.2, 0.4, 0.6, 0.8, 1.0].map((v) => (
                <span key={v} style={{ width: 18, height: 12, borderRadius: 3, background: v === 0 ? "var(--surface-2)" : `color-mix(in oklch, var(--moss) ${Math.round(v * 78)}%, var(--surface-2))` }} />
              ))}
              <span className="bnb-meta">busy</span>
            </div>
          </div>

          {/* Weekday header */}
          <div style={{ display: "grid", gridTemplateColumns: "50px repeat(7, 1fr)", gap: 6, marginBottom: 8 }}>
            <span />
            {["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].map((d) => (
              <span key={d} className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)", textAlign: "center" }}>{d}</span>
            ))}
          </div>

          <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 6 }}>
            {calendar.map((week, wi) => {
              const monthLabel = week[0].date.toLocaleString("en-US", { month: "short", day: "numeric" });
              return (
                <div key={wi} style={{ display: "grid", gridTemplateColumns: "50px repeat(7, 1fr)", gap: 6, flex: 1 }}>
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)", display: "flex", alignItems: "center" }}>{monthLabel}</span>
                  {week.map((d, di) => {
                    const intensity = d.count / maxCount;
                    const dateStr = d.date.toISOString().slice(0, 10);
                    const isSel = dateStr === date;
                    const isToday = dateStr === "2025-05-22";
                    return (
                      <button key={di} onClick={() => setDate(dateStr)} style={{
                        background: `color-mix(in oklch, var(--moss) ${Math.round(intensity * 76)}%, var(--surface-2))`,
                        border: 0,
                        borderRadius: 7,
                        padding: "8px 8px",
                        position: "relative",
                        cursor: "pointer",
                        outline: isSel ? "2px solid var(--fg)" : "none",
                        outlineOffset: -1.5,
                        textAlign: "left",
                        overflow: "hidden",
                      }}>
                        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline" }}>
                          <span className="mono" style={{ fontSize: 11, color: intensity > 0.5 ? "var(--bg)" : "var(--fg)", fontWeight: 500 }}>
                            {d.date.getDate()}
                          </span>
                          {isToday && <span className="mono" style={{ fontSize: 8.5, color: intensity > 0.5 ? "var(--bg)" : "var(--fg)", opacity: 0.7 }}>today</span>}
                        </div>
                        <div className="mono tabular" style={{ fontSize: 13, color: intensity > 0.5 ? "var(--bg)" : "var(--fg)", marginTop: 4, fontWeight: 600 }}>{d.count}</div>
                        <div style={{ display: "flex", gap: 1, marginTop: 4 }}>
                          {Array.from({ length: Math.min(8, d.species) }).map((_, i) => (
                            <span key={i} style={{ width: 2, height: 4, background: intensity > 0.5 ? "var(--bg)" : "var(--moss-ink)", opacity: 0.7, borderRadius: 1 }} />
                          ))}
                        </div>
                      </button>
                    );
                  })}
                </div>
              );
            })}
          </div>
        </div>

        {/* Day detail */}
        <DayDetail date={date} />
      </div>
    </Screen>
  );
}

function DayDetail({ date }) {
  const { SPECIES } = window.BNB;
  // Hourly bars
  const hours = Array.from({ length: 24 }, (_, h) => {
    const env = 4.5 * Math.exp(-Math.pow((h - 6.5) / 1.8, 2))
             + 3.0 * Math.exp(-Math.pow((h - 18.5) / 2.4, 2))
             + (h > 8 && h < 18 ? 1.4 : 0.3);
    return Math.round(env * 12);
  });
  const maxH = Math.max(...hours);
  const totalDay = hours.reduce((a, b) => a + b, 0);

  const top = [...SPECIES].sort((a, b) => b.count - a.count).slice(0, 5);

  return (
    <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 14, overflow: "hidden" }}>
      <div>
        <div className="bnb-eyebrow">{new Date(date).toLocaleString("en-US", { weekday: "long", month: "long", day: "numeric" })}</div>
        <h2 className="display" style={{ fontSize: 36, lineHeight: 1.05, marginTop: 4 }}>
          <span className="tabular">{totalDay.toLocaleString()}</span> detections
        </h2>
        <div className="bnb-meta" style={{ marginTop: 4 }}>15 species · listened 14h 22m · 1 rare in quarantine</div>
      </div>

      {/* hourly bars */}
      <div>
        <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>By hour</div>
        <div style={{ display: "flex", alignItems: "flex-end", gap: 3, height: 120 }}>
          {hours.map((v, h) => {
            const isPeak = h >= 5 && h <= 8;
            return (
              <div key={h} style={{ flex: 1, display: "flex", flexDirection: "column", alignItems: "center", gap: 4 }}>
                <span style={{ width: "100%", height: `${(v / maxH) * 96}px`, background: isPeak ? "var(--moss)" : "color-mix(in oklch, var(--moss) 35%, var(--surface-2))", borderRadius: "3px 3px 0 0", minHeight: 2 }} />
                {h % 6 === 0 && <span className="mono" style={{ fontSize: 9, color: "var(--fg-3)" }}>{h === 0 ? "12a" : h === 12 ? "12p" : h < 12 ? `${h}a` : `${h - 12}p`}</span>}
              </div>
            );
          })}
        </div>
      </div>

      {/* Top species this day */}
      <div>
        <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>Loudest five</div>
        {top.map((s, i) => (
          <div key={i} style={{ display: "grid", gridTemplateColumns: "24px 1fr auto", gap: 10, padding: "8px 0", borderTop: i > 0 ? "0.5px solid var(--hairline)" : "0", alignItems: "center" }}>
            <SpeciesAvatar sp={SPECIES.indexOf(s)} size={24} />
            <span style={{ fontSize: 13 }}>{s.common}</span>
            <span className="mono tabular" style={{ fontSize: 12 }}>{Math.round(s.count * 0.25)}</span>
          </div>
        ))}
      </div>

      <div style={{ marginTop: "auto", display: "flex", gap: 8, paddingTop: 12, borderTop: "0.5px solid var(--hairline)" }}>
        <button className="bnb-btn primary" style={{ flex: 1, justifyContent: "center" }}>Open day's log →</button>
        <button className="bnb-btn">Recordings (47)</button>
      </div>
    </div>
  );
}

Object.assign(window, { History });
