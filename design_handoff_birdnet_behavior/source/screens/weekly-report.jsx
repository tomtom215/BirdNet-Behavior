// Weekly report — newspaper-style "this week in your yard" summary.
// Hobbyist-friendly storytelling; researcher-friendly numbers in the gutters.

function WeeklyReport() {
  const { SPECIES } = window.BNB;
  const topSpecies = [...SPECIES].sort((a, b) => b.count - a.count).slice(0, 8);
  const firstOfWeek = [SPECIES[10], SPECIES[12], SPECIES[14]];

  return (
    <Screen padded={false}>
      <div style={{ display: "grid", gridTemplateColumns: "1fr", padding: "var(--pad-4) var(--pad-4) var(--pad-3)", borderBottom: "2px solid var(--fg)" }}>
        <TopNav active="Weekly" />
      </div>

      {/* Masthead */}
      <div style={{
        padding: "var(--pad-4) var(--pad-4)",
        textAlign: "center",
        borderBottom: "0.5px solid var(--border-2)",
        background: "linear-gradient(180deg, var(--surface) 0%, var(--bg) 100%)",
      }}>
        <div className="bnb-eyebrow" style={{ marginBottom: 8, letterSpacing: "0.18em" }}>The Backyard Bulletin · Issue No. 32</div>
        <h1 className="display" style={{ fontSize: 76, lineHeight: 0.95, letterSpacing: "-0.03em" }}>
          The week the warblers <em style={{ color: "var(--moss-ink)" }}>came back</em>
        </h1>
        <div className="bnb-meta" style={{ marginTop: 12, fontSize: 13 }}>
          May 16 — May 22, 2025 · 6,238 detections · 23 species · sunrise climbed 4 min · weather mild
        </div>
      </div>

      {/* Newspaper columns */}
      <div style={{ padding: "var(--pad-4)", display: "grid", gridTemplateColumns: "2fr 1fr", gap: "var(--pad-4)", flex: 1, minHeight: 0 }}>
        {/* Left column — lead story + stats */}
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--pad-4)" }}>
          <article>
            <div className="bnb-eyebrow" style={{ color: "var(--moss-ink)", marginBottom: 6 }}>Lead story · migration</div>
            <h2 className="display" style={{ fontSize: 36, lineHeight: 1.1, letterSpacing: "-0.02em", marginBottom: 12 }}>
              Two new warblers arrived in one morning, four days early.
            </h2>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 20, columnGap: 28 }}>
              <p style={{ fontSize: 14, lineHeight: 1.65, color: "var(--fg-2)", textWrap: "pretty" }}>
                Tuesday morning at <span className="mono">06:14</span>, the station logged its first Magnolia Warbler of the year — followed eleven minutes later by a Rose-breasted Grosbeak. Both were first-of-year detections, both confidently identified above the 0.80 threshold, and both arrived approximately four days earlier than they did in 2024.
              </p>
              <p style={{ fontSize: 14, lineHeight: 1.65, color: "var(--fg-2)", textWrap: "pretty" }}>
                The early arrival fits a broader regional pattern: eBird's hotline reports unusually warm overnight lows from the Mid-Atlantic up through New England, and migrants appear to be riding the front. The Grosbeak hasn't sung since, but the Magnolia has been a daily fixture.
              </p>
            </div>
          </article>

          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr 1fr", gap: 0, border: "0.5px solid var(--border)", borderRadius: 12, overflow: "hidden" }}>
            <BigNumber n="6,238" label="Detections this week" sub="↑ 22% week-on-week" />
            <BigNumber n="23"    label="Species" sub="3 new this week" accent="var(--moss-ink)" />
            <BigNumber n="0.93" label="Median confidence" sub="quality holding" />
            <BigNumber n="06:14" label="Earliest detection" sub="Magnolia Warbler" last />
          </div>

          {/* Top species ranking */}
          <article>
            <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>The leaderboard · seven-day count</div>
            <h3 className="display" style={{ fontSize: 22, lineHeight: 1.2, marginBottom: 14 }}>Who turned up the volume</h3>
            <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
              {topSpecies.map((s, i) => (
                <div key={i} style={{
                  display: "grid", gridTemplateColumns: "24px 36px 1fr 60px 200px 60px",
                  gap: 14, alignItems: "center", padding: "10px 0",
                  borderTop: i > 0 ? "0.5px solid var(--hairline)" : "0",
                }}>
                  <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)", textAlign: "right" }}>{i + 1}.</span>
                  <SpeciesAvatar sp={SPECIES.indexOf(s)} size={28} />
                  <div>
                    <div style={{ fontSize: 14, fontWeight: 500 }}>{s.common}</div>
                    <div className="bnb-meta mono" style={{ fontStyle: "italic" }}>{s.sci}</div>
                  </div>
                  <span className="mono tabular" style={{ fontSize: 14, color: "var(--fg)", textAlign: "right" }}>{s.count}</span>
                  <Sparkline data={s.trend.slice(0, 14)} width={200} height={26} accent={s.color} />
                  <span className="bnb-pill" style={{ fontSize: 10, justifyContent: "center", color: "var(--moss-ink)", background: "var(--moss-soft)", border: 0 }}>+{(8 + i * 2)}%</span>
                </div>
              ))}
            </div>
          </article>
        </div>

        {/* Right column — sidebar */}
        <aside style={{ display: "flex", flexDirection: "column", gap: "var(--pad-4)" }}>
          <section>
            <div className="bnb-eyebrow" style={{ color: "var(--moss-ink)", marginBottom: 6 }}>First-of-year</div>
            <h3 className="display" style={{ fontSize: 22, lineHeight: 1.2 }}>Lifers this week</h3>
            <div style={{ marginTop: 12, display: "flex", flexDirection: "column", gap: 0 }}>
              {firstOfWeek.map((sp, i) => (
                <div key={i} style={{ display: "grid", gridTemplateColumns: "auto 1fr auto", gap: 10, padding: "12px 0", alignItems: "center", borderTop: i > 0 ? "0.5px dashed var(--hairline)" : "0" }}>
                  <SpeciesAvatar sp={SPECIES.indexOf(sp)} size={36} />
                  <div>
                    <div style={{ fontSize: 14, fontWeight: 500 }}>{sp.common}</div>
                    <div className="bnb-meta mono" style={{ fontStyle: "italic" }}>{sp.sci}</div>
                  </div>
                  <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)" }}>{["Tue 06:14", "Tue 06:25", "Wed 19:48"][i]}</span>
                </div>
              ))}
            </div>
          </section>

          <section>
            <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Day-by-day</div>
            <h3 className="display" style={{ fontSize: 22, lineHeight: 1.2 }}>Activity profile</h3>
            <DailyBars />
            <div className="bnb-meta" style={{ marginTop: 8 }}>
              Tuesday's spike reflects the two early arrivals. Saturday quietness likely human-caused — neighborhood mowers.
            </div>
          </section>

          <section style={{ background: "var(--surface-2)", padding: 16, borderRadius: 12 }}>
            <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Note from your station</div>
            <h3 className="display" style={{ fontSize: 18, lineHeight: 1.3 }}>Owl audio queued for your review</h3>
            <div className="bnb-meta" style={{ marginTop: 6, lineHeight: 1.55 }}>
              A 02:14 detection labeled <strong style={{ color: "var(--fg)" }}>Barred Owl</strong> sits in the quarantine queue — first time this species has been heard at this station. Two clicks to approve.
            </div>
            <a href="#" className="bnb-btn" style={{ marginTop: 12, fontSize: 12 }}>Review queue →</a>
          </section>

          <section>
            <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>For the curious</div>
            <h3 className="display" style={{ fontSize: 22, lineHeight: 1.2 }}>How this issue was made</h3>
            <div className="bnb-meta" style={{ marginTop: 8, lineHeight: 1.55 }}>
              Numbers come from the SQLite detections database, filtered above confidence 0.80. Weekly windows align with the local sunrise calendar. eBird comparison data was fetched on Friday.
            </div>
            <div className="bnb-meta mono" style={{ marginTop: 8, color: "var(--fg-3)" }}>
              Generated 2025-05-23 · view raw data ↗
            </div>
          </section>
        </aside>
      </div>
    </Screen>
  );
}

function BigNumber({ n, label, sub, accent, last }) {
  return (
    <div style={{ padding: "var(--pad-3)", borderRight: last ? "none" : "0.5px solid var(--hairline)" }}>
      <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>{label}</div>
      <div className="display tabular" style={{ fontSize: 40, lineHeight: 1, color: accent || "var(--fg)" }}>{n}</div>
      <div className="bnb-meta mono" style={{ marginTop: 6 }}>{sub}</div>
    </div>
  );
}

function DailyBars() {
  const days = [
    { d: "Sun", v: 712 },
    { d: "Mon", v: 880 },
    { d: "Tue", v: 1240, peak: true },
    { d: "Wed", v: 942 },
    { d: "Thu", v: 1008 },
    { d: "Fri", v: 876 },
    { d: "Sat", v: 580 },
  ];
  const max = Math.max(...days.map((d) => d.v));
  return (
    <div style={{ display: "flex", alignItems: "flex-end", gap: 8, marginTop: 12, height: 110 }}>
      {days.map((d, i) => (
        <div key={i} style={{ flex: 1, display: "flex", flexDirection: "column", alignItems: "center", gap: 6 }}>
          <span className="mono tabular" style={{ fontSize: 10.5, color: d.peak ? "var(--moss-ink)" : "var(--fg-3)" }}>{d.v}</span>
          <span style={{ width: "100%", height: `${(d.v / max) * 76}px`, background: d.peak ? "var(--moss)" : "color-mix(in oklch, var(--moss) 36%, var(--surface-2))", borderRadius: "4px 4px 0 0" }} />
          <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)" }}>{d.d}</span>
        </div>
      ))}
    </div>
  );
}

Object.assign(window, { WeeklyReport });
