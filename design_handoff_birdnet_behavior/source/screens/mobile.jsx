// Mobile companion views — phone-frame screens.

const { useState: useState_m, useEffect: useEffect_m } = React;

function PhoneFrame({ children, label }) {
  return (
    <div style={{ width: "100%", height: "100%", background: "var(--bg)", display: "flex", alignItems: "stretch", justifyContent: "center", padding: 0 }}>
      <div style={{
        width: "100%", height: "100%",
        position: "relative",
        background: "var(--bg)",
        overflow: "hidden",
        display: "flex", flexDirection: "column",
      }}>
        {/* Status bar */}
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "16px 24px 8px", flex: "0 0 auto" }}>
          <span className="mono" style={{ fontSize: 13, fontWeight: 600, color: "var(--fg)" }}>9:41</span>
          <span style={{ display: "inline-flex", gap: 4, alignItems: "center", color: "var(--fg)" }}>
            <svg width="16" height="10" viewBox="0 0 16 10" fill="currentColor"><rect x="0" y="6" width="2.5" height="4" rx="0.5"/><rect x="3.5" y="4" width="2.5" height="6" rx="0.5"/><rect x="7" y="2" width="2.5" height="8" rx="0.5"/><rect x="10.5" y="0" width="2.5" height="10" rx="0.5"/></svg>
            <svg width="22" height="11" viewBox="0 0 22 11" fill="none" stroke="currentColor"><rect x="0.5" y="0.5" width="18" height="10" rx="2.5"/><rect x="2" y="2" width="13" height="7" rx="1" fill="currentColor"/><rect x="19.5" y="3.5" width="1.5" height="4" rx="0.5" fill="currentColor"/></svg>
          </span>
        </div>
        {children}
      </div>
    </div>
  );
}

// ─── Phone dashboard ──────────────────────────────────────────────────────
function PhoneDashboard({ demo = "busy" }) {
  const { SPECIES, FEED_SEED } = window.BNB;
  const [feed, setFeed] = useState_m(() => FEED_SEED.slice(0, 6).map((d, i) => ({ ...d, id: `seed-${i}` })));

  useEffect_m(() => {
    const interval = demo === "dawn" ? 1800 : demo === "quiet" ? 6500 : 3200;
    const pool = demo === "dawn" ? [1, 3, 0, 2, 5] : [0, 1, 2, 3, 5];
    const t = setInterval(() => {
      const sp = pool[Math.floor(Math.random() * pool.length)];
      setFeed((prev) => [{ id: Date.now(), sp, conf: 0.8 + Math.random() * 0.18, ago: "just now", lat: 1.2 + Math.random() }, ...prev.slice(0, 5)]);
    }, interval);
    return () => clearInterval(t);
  }, [demo]);

  return (
    <PhoneFrame>
      <div className="bnb-root" style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
        {/* Header */}
        <div style={{ padding: "0 24px 12px", display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 8 }}>
          <div>
            <div className="bnb-eyebrow">Good morning</div>
            <h1 className="display" style={{ fontSize: 32, lineHeight: 1.05 }}>Singing.</h1>
            <div className="bnb-meta mono" style={{ marginTop: 4 }}>Thu · 6:42 AM · 14h listening</div>
          </div>
          <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 6 }}>
            <span className="bnb-pill moss"><span className="bnb-dot live" /> live</span>
            <BrandMark size={20} />
          </div>
        </div>

        {/* Big stats */}
        <div style={{ display: "flex", gap: 12, padding: "0 20px 16px", marginTop: 8 }}>
          <BigStat label="Today" value="912" sub="↑ 14%" />
          <BigStat label="Species" value="15" sub="2 new" accent="var(--moss)" />
        </div>

        {/* Section header */}
        <div style={{ padding: "12px 24px 8px", display: "flex", justifyContent: "space-between", alignItems: "baseline" }}>
          <div className="bnb-eyebrow">Live feed</div>
          <a href="#" className="bnb-meta">All →</a>
        </div>

        {/* Feed */}
        <div style={{ flex: 1, overflow: "hidden", padding: "0 16px 12px" }}>
          {feed.slice(0, 6).map((d, i) => {
            const sp = SPECIES[d.sp];
            return (
              <div key={d.id} style={{
                display: "grid", gridTemplateColumns: "40px 1fr auto", gap: 10,
                alignItems: "center", padding: "12px 10px",
                background: i === 0 && d.ago === "just now" ? "color-mix(in oklch, var(--moss) 7%, var(--surface))" : "var(--surface)",
                border: "0.5px solid var(--border)",
                borderRadius: 12,
                marginBottom: 6,
              }}>
                <SpeciesAvatar sp={d.sp} size={36} />
                <div style={{ minWidth: 0 }}>
                  <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                    <span style={{ fontWeight: 500, fontSize: 14.5 }}>{sp.common}</span>
                  </div>
                  <div className="bnb-meta mono">{d.ago} · {(d.conf || 0.9).toFixed(2)}</div>
                </div>
                <span style={{ display: "inline-flex", gap: 1.5, alignItems: "center", height: 22 }}>
                  {[2,4,6,5,3,4,5,3,2,3,2].map((v, j) => (
                    <span key={j} style={{ width: 2, height: `${v * 3}px`, borderRadius: 1, background: `color-mix(in oklch, ${sp.color} ${30 + v * 8}%, var(--fg-4))` }} />
                  ))}
                </span>
              </div>
            );
          })}
        </div>

        {/* Tab bar */}
        <div style={{ borderTop: "0.5px solid var(--border)", padding: "10px 16px 24px", display: "flex", justifyContent: "space-around", background: "var(--surface)" }}>
          {[
            { label: "Today",   active: true },
            { label: "Species", active: false },
            { label: "Heatmap", active: false },
            { label: "List",    active: false },
          ].map((t) => (
            <div key={t.label} style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 4 }}>
              <span style={{ width: 24, height: 4, borderRadius: 2, background: t.active ? "var(--fg)" : "transparent" }} />
              <span style={{ fontSize: 11, color: t.active ? "var(--fg)" : "var(--fg-3)", fontWeight: t.active ? 500 : 400 }}>{t.label}</span>
            </div>
          ))}
        </div>
      </div>
    </PhoneFrame>
  );
}

function BigStat({ label, value, sub, accent }) {
  return (
    <div style={{ flex: 1, background: "var(--surface)", border: "0.5px solid var(--border)", borderRadius: 14, padding: 14 }}>
      <div className="bnb-eyebrow" style={{ fontSize: 10 }}>{label}</div>
      <div className="display tabular" style={{ fontSize: 30, lineHeight: 1, marginTop: 6, color: accent || "var(--fg)" }}>{value}</div>
      <div className="bnb-meta mono" style={{ marginTop: 4 }}>{sub}</div>
    </div>
  );
}

// ─── Phone species detail ─────────────────────────────────────────────────
function PhoneSpecies() {
  const { SPECIES, CHORUS } = window.BNB;
  const speciesIdx = 1; // Cardinal
  const sp = SPECIES[speciesIdx];
  const hours = CHORUS.find((c) => c.sp === speciesIdx)?.hours || sp.trend.slice(0, 24);
  const maxH = Math.max(0.001, ...hours);

  return (
    <PhoneFrame>
      <div className="bnb-root" style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
        <div style={{ padding: "0 16px 8px", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <button className="bnb-btn ghost" style={{ padding: 6 }}>← Species</button>
          <button className="bnb-btn ghost" style={{ padding: 6 }}>⋯</button>
        </div>

        {/* Photo */}
        <div style={{ padding: "0 16px" }}>
          <image-slot
            id="phone-species-cardinal"
            shape="rect"
            placeholder="Drop Cardinal photo"
            radius="18"
            style={{ width: "100%", height: 180, display: "block", borderRadius: 18, overflow: "hidden" }}
          ></image-slot>
        </div>

        <div style={{ padding: "16px 24px 8px" }}>
          <div className="bnb-pill moss" style={{ marginBottom: 8 }}><span className="bnb-dot live" /> heard 14 min ago</div>
          <h1 className="display" style={{ fontSize: 30, lineHeight: 1.05 }}>{sp.common}</h1>
          <div className="bnb-meta" style={{ fontStyle: "italic", fontSize: 13, marginTop: 4 }}>{sp.sci}</div>
        </div>

        <div style={{ padding: "0 16px", display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 8 }}>
          <PhoneTile label="Today" value={sp.count} />
          <PhoneTile label="All-time" value="4.2k" />
          <PhoneTile label="Mean conf." value={sp.conf.toFixed(2)} accent="var(--moss-ink)" />
        </div>

        <div style={{ padding: "16px 24px 8px" }}>
          <div className="bnb-eyebrow">When you'll hear it</div>
        </div>
        <div style={{ padding: "0 24px", height: 80, display: "flex", alignItems: "flex-end", gap: 3 }}>
          {hours.map((v, h) => (
            <div key={h} style={{ flex: 1, height: `${(v / maxH) * 100}%`, background: `color-mix(in oklch, ${sp.color} ${20 + (v / maxH) * 60}%, var(--surface-2))`, borderRadius: 2, minHeight: 2 }} />
          ))}
        </div>
        <div style={{ padding: "4px 24px 16px", display: "flex", justifyContent: "space-between" }}>
          {[0, 6, 12, 18, 23].map((h) => (
            <span key={h} className="mono" style={{ fontSize: 9.5, color: "var(--fg-3)" }}>{h === 0 ? "12a" : h === 12 ? "12p" : h < 12 ? `${h}a` : `${h - 12}p`}</span>
          ))}
        </div>

        <div style={{ padding: "0 16px", marginTop: "auto", marginBottom: 24 }}>
          <button className="bnb-btn primary" style={{ width: "100%", justifyContent: "center", padding: "12px" }}>▶  Play latest clip · 1.4 s</button>
        </div>
      </div>
    </PhoneFrame>
  );
}

function PhoneTile({ label, value, accent }) {
  return (
    <div style={{ background: "var(--surface)", border: "0.5px solid var(--border)", borderRadius: 12, padding: 10 }}>
      <div className="bnb-eyebrow" style={{ fontSize: 9.5 }}>{label}</div>
      <div className="display tabular" style={{ fontSize: 22, lineHeight: 1.1, marginTop: 4, color: accent || "var(--fg)" }}>{value}</div>
    </div>
  );
}

// ─── Phone rare-bird alert ────────────────────────────────────────────────
function PhoneAlert() {
  const { SPECIES } = window.BNB;
  const sp = SPECIES[14]; // Barred Owl
  return (
    <PhoneFrame>
      <div className="bnb-root" style={{ flex: 1, display: "flex", flexDirection: "column", background: "var(--night)", color: "var(--bg)", paddingTop: 0 }}>
        <style>{`
          .phone-alert .bnb-pill { background: rgba(255,255,255,.12); color: var(--bg); border-color: transparent; }
          .phone-alert .bnb-meta { color: rgba(255,255,255,.55); }
        `}</style>
        <div className="phone-alert" style={{ flex: 1, padding: "32px 24px", display: "flex", flexDirection: "column" }}>
          <div className="bnb-pill"><span className="bnb-dot" style={{ background: "var(--rare)", boxShadow: "0 0 0 2px rgba(255,99,0,.25)" }} /> Rare bird</div>
          <h1 className="display" style={{ fontSize: 40, lineHeight: 1.05, marginTop: 18, color: "var(--bg)" }}>{sp.common}</h1>
          <div style={{ fontSize: 14, fontStyle: "italic", color: "rgba(255,255,255,.6)", marginTop: 4 }}>{sp.sci}</div>

          <div style={{ marginTop: 28 }}>
            <div className="bnb-meta">Detected · just now</div>
            <div className="mono" style={{ fontSize: 16, marginTop: 4 }}>02:14:38 · 24°F · windless</div>
            <div className="bnb-meta" style={{ marginTop: 14 }}>Confidence</div>
            <div className="display tabular" style={{ fontSize: 32, color: "var(--bg)" }}>0.93</div>
          </div>

          {/* Mini spectrogram */}
          <div style={{ marginTop: 28, padding: 12, background: "rgba(255,255,255,.05)", borderRadius: 14 }}>
            <div className="bnb-meta">Acoustic evidence</div>
            <svg viewBox="0 0 240 80" width="100%" height="80" style={{ marginTop: 8 }}>
              {Array.from({ length: 60 }).map((_, i) => {
                const phase = (i / 60) * Math.PI * 4;
                const v = 0.2 + 0.6 * Math.exp(-Math.pow((i - 30) / 14, 2));
                const y = 40 + Math.sin(phase) * 18 * v;
                return <rect key={i} x={i * 4} y={y - 1} width={3} height={2} rx={1} fill="var(--bg)" fillOpacity={v} />;
              })}
            </svg>
          </div>

          <div style={{ marginTop: "auto", display: "flex", gap: 8 }}>
            <button className="bnb-btn" style={{ flex: 1, background: "rgba(255,255,255,.10)", color: "var(--bg)", border: 0, padding: "12px", justifyContent: "center" }}>Reject</button>
            <button className="bnb-btn" style={{ flex: 1, background: "var(--bg)", color: "var(--night)", border: 0, padding: "12px", justifyContent: "center", fontWeight: 600 }}>Approve →</button>
          </div>
        </div>
      </div>
    </PhoneFrame>
  );
}

Object.assign(window, { PhoneFrame, PhoneDashboard, PhoneSpecies, PhoneAlert, MobileShowcase });

// ─── Mobile Showcase — three phones on one cohesive canvas ────────────────
function MobileShowcase({ demo = "busy" }) {
  return (
    <div style={{
      position: "relative",
      width: "100%", height: "100%",
      background: "radial-gradient(ellipse at 50% 30%, oklch(28% 0.03 250) 0%, oklch(14% 0.02 250) 70%, oklch(8% 0.02 250) 100%)",
      display: "flex", alignItems: "center", justifyContent: "center", gap: 64,
      padding: "48px 32px",
      overflow: "hidden",
    }}>
      {/* Decorative sound waves in the bg */}
      <BgWaves />

      <PhoneShell label="Live feed" caption="Your dawn ritual" delay={0}>
        <PhoneDashboard demo={demo} />
      </PhoneShell>
      <PhoneShell label="Species page" caption="Tap any bird" tilt={-3} delay={120}>
        <PhoneSpecies />
      </PhoneShell>
      <PhoneShell label="Rare-bird alert" caption="Push notification" tilt={3} delay={240}>
        <PhoneAlert />
      </PhoneShell>
    </div>
  );
}

function PhoneShell({ children, label, caption, tilt = 0, delay = 0 }) {
  return (
    <div style={{
      display: "flex", flexDirection: "column", alignItems: "center", gap: 16,
      transform: `rotate(${tilt}deg)`,
      animation: `phone-rise 800ms cubic-bezier(.2,.7,.2,1) ${delay}ms both`,
    }}>
      <style>{`@keyframes phone-rise { from { opacity: 0; transform: translateY(20px) rotate(${tilt}deg); } to { opacity: 1; transform: translateY(0) rotate(${tilt}deg); } }`}</style>
      <div style={{
        width: 320, height: 690,
        padding: 8,
        borderRadius: 46,
        background: "linear-gradient(160deg, oklch(35% 0.01 250) 0%, oklch(20% 0.01 250) 50%, oklch(28% 0.01 250) 100%)",
        boxShadow: "0 40px 80px oklch(0% 0 0 / .45), 0 16px 40px oklch(0% 0 0 / .35), inset 0 0 0 1px oklch(50% 0.01 250 / .4)",
      }}>
        <div style={{
          width: "100%", height: "100%",
          borderRadius: 38,
          overflow: "hidden",
          background: "var(--bg)",
          position: "relative",
        }}>
          {/* dynamic-island bump */}
          <div style={{
            position: "absolute", top: 10, left: "50%", transform: "translateX(-50%)",
            width: 100, height: 28, borderRadius: 20,
            background: "oklch(8% 0.01 250)", zIndex: 20,
          }} />
          {/* The actual phone content — scaled to fit 304×674 (the inner display) */}
          <div style={{
            width: 390, height: 844,
            transformOrigin: "top left",
            transform: `scale(${304 / 390})`,
          }}>
            {children}
          </div>
        </div>
      </div>
      <div style={{ textAlign: "center" }}>
        <div className="mono" style={{ fontSize: 11, color: "oklch(75% 0.04 80)", textTransform: "uppercase", letterSpacing: "0.1em" }}>{label}</div>
        <div style={{ fontFamily: "var(--font-display)", fontSize: 18, color: "oklch(92% 0.02 80)", marginTop: 2, letterSpacing: "-0.01em" }}>{caption}</div>
      </div>
    </div>
  );
}

function BgWaves() {
  return (
    <svg width="100%" height="100%" viewBox="0 0 1400 700" style={{ position: "absolute", inset: 0, opacity: 0.08 }}>
      {Array.from({ length: 8 }).map((_, i) => {
        const r = 60 + i * 90;
        return <circle key={i} cx="700" cy="380" r={r} fill="none" stroke="oklch(80% 0.14 150)" strokeWidth="0.5" />;
      })}
    </svg>
  );
}
