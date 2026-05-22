// Dashboard — desktop. The live feed animates in new detections every few seconds.
const { useState: useState_dash, useEffect: useEffect_dash, useMemo: useMemo_dash } = React;

function Dashboard({ demo = "busy" }) {
  const { SPECIES, FEED_SEED, HEATMAP, DAY_LABELS } = window.BNB;

  // The live ticker — prepend a fresh "just detected" item every N seconds.
  const [feed, setFeed] = useState_dash(() => FEED_SEED.map((d, i) => ({ ...d, id: i, age: i })));
  useEffect_dash(() => {
    const interval = demo === "dawn" ? 1800 : demo === "quiet" ? 6500 : 3200;
    const pool = demo === "dawn" ? [1, 3, 0, 2, 14, 5] : demo === "quiet" ? [0, 3, 5] : [0, 1, 2, 3, 5, 6, 9, 12];
    const t = setInterval(() => {
      setFeed((prev) => {
        const sp = pool[Math.floor(Math.random() * pool.length)];
        const conf = 0.78 + Math.random() * 0.20;
        const lat = 1.0 + Math.random() * 1.2;
        const next = [
          { id: Date.now(), sp, conf, lat, ago: "just now", rare: SPECIES[sp].rare && Math.random() > 0.5 },
          ...prev.slice(0, 11).map((d) => ({ ...d, age: (d.age || 0) + 1 })),
        ];
        return next;
      });
    }, interval);
    return () => clearInterval(t);
  }, [demo]);

  const topSpecies = useMemo_dash(() => [...SPECIES].sort((a, b) => b.count - a.count).slice(0, 6), [SPECIES]);
  const todayCount = useMemo_dash(() => SPECIES.reduce((s, x) => s + x.count, 0), [SPECIES]);

  return (
    <Screen>
      <TopNav active="Dashboard" />

      {/* Hero strip — statement + live pulse on right */}
      <div style={{ display: "grid", gridTemplateColumns: "1.25fr 1fr", gap: "var(--pad-4)", alignItems: "center", paddingTop: 6 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>Right now · {new Date().toLocaleString("en-US", { weekday: "long", month: "long", day: "numeric" })}</div>
          <h1 className="display" style={{ fontSize: 64, lineHeight: 1.02, letterSpacing: "-0.025em", textWrap: "balance" }}>
            The yard is <em style={{ fontStyle: "italic", color: "var(--moss-ink)" }}>singing</em>.
          </h1>
          <p style={{ marginTop: 14, color: "var(--fg-2)", fontSize: 15, maxWidth: 540, lineHeight: 1.55 }}>
            <span className="mono">{todayCount.toLocaleString()}</span> calls from <span className="mono">{SPECIES.length}</span> species so far today. Activity is <span style={{ color: "var(--moss-ink)", fontWeight: 500 }}>32% above</span> your seasonal baseline — driven mostly by Cardinals and a late Magnolia Warbler.
          </p>
          <div style={{ display: "flex", gap: 8, marginTop: 16, flexWrap: "wrap" }}>
            <span className="bnb-pill moss"><span className="bnb-dot live" /> recording · 14h 22m</span>
            <span className="bnb-pill">☀ sunrise 5:21</span>
            <span className="bnb-pill">☾ sunset 20:08</span>
            <span className="bnb-pill mono">42.36°N · −71.06°W</span>
          </div>
        </div>
        <HeroPulse />
      </div>

      {/* Stat row */}
      <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 0, border: "0.5px solid var(--border)", borderRadius: "var(--r-lg)", overflow: "hidden", background: "var(--surface)", boxShadow: "var(--shadow-sm)" }}>
        <StatTile label="Detections · 24h" value={todayCount.toLocaleString()} sub="↑ 14% vs. last week" trend={[3,5,4,7,8,9,11,13,14,12,11,10]} />
        <StatTile
          label="Species · 24h"
          value={SPECIES.length}
          sub="+ 2 first-of-year"
          subAccent="var(--moss-ink)"
          trend={[1,2,3,4,5,7,8,10,11,13,14,15]}
          chips={[{ label: "MAWA", color: "oklch(72% 0.16 85)" }, { label: "RBGR", color: "oklch(50% 0.16 20)" }]}
        />
        <StatTile
          label="Rare today"
          value={2}
          sub="1 awaiting review"
          subAccent="var(--rare)"
          accentLine="var(--rare)"
          trend={[0,0,1,0,0,0,1,0,0,0,1,2]}
          chips={[{ label: "BADO 02:14", color: "var(--rare)" }]}
        />
        <StatTile label="Listening" value="14h 22m" sub="0 dropouts · 41°C" live trend={[1,1,1,1,1,1,1,1,1,1,1,1]} constant />
      </div>


      <hr className="bnb-divider" />

      {/* Two-column working area */}
      <div style={{ display: "grid", gridTemplateColumns: "1.35fr 1fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        {/* Live feed */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 12, minHeight: 0 }}>
          <SectionHeader
            eyebrow="Live feed"
            title="Detections as they happen"
            action={<div style={{ display: "flex", gap: 6 }}>
              <span className="bnb-pill moss"><span className="bnb-dot live" /> Listening</span>
              <span className="bnb-pill">All species</span>
              <span className="bnb-pill">≥ 0.80</span>
            </div>}
          />
          <div style={{ display: "flex", flexDirection: "column", gap: 6, overflow: "hidden", flex: 1 }}>
            {feed.slice(0, 10).map((d, i) => (
              <FeedRow key={d.id} d={d} fresh={i === 0 && d.ago === "just now"} />
            ))}
          </div>
        </div>

        {/* Right rail */}
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--pad-3)", minHeight: 0 }}>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <SectionHeader eyebrow="Today" title="Top species" action={<a className="bnb-meta" href="#">See all 15 →</a>} />
            <div style={{ display: "flex", flexDirection: "column", marginTop: 10 }}>
              {topSpecies.map((s, i) => (
                <div key={i} style={{
                  display: "grid", gridTemplateColumns: "auto 1fr auto 56px",
                  alignItems: "center", gap: 10, padding: "8px 0",
                  borderTop: i > 0 ? "0.5px solid var(--hairline)" : "0",
                }}>
                  <SpeciesAvatar sp={SPECIES.indexOf(s)} size={26} />
                  <div style={{ minWidth: 0 }}>
                    <div style={{ fontSize: 13, fontWeight: 500, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{s.common}</div>
                    <div className="bnb-meta mono">{s.sci}</div>
                  </div>
                  <span className="mono tabular" style={{ fontSize: 13, color: "var(--fg-2)" }}>{s.count}</span>
                  <Sparkline data={s.trend} width={56} height={16} accent={s.color} />
                </div>
              ))}
            </div>
          </div>

          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <SectionHeader eyebrow="Activity · 7 days" title="Hour × day" action={<a className="bnb-meta" href="#">Open heatmap →</a>} />
            <div style={{ marginTop: 12 }}>
              <MiniHeat />
            </div>
          </div>
        </div>
      </div>
    </Screen>
  );
}

// Stat tile (in the 4-col grid) — has subtle dividers between
function StatTile({ label, value, sub, subAccent, trend, live, accentLine, chips, constant }) {
  return (
    <div style={{
      padding: "var(--pad-3)",
      borderRight: "0.5px solid var(--hairline)",
      display: "flex", flexDirection: "column", gap: 12, minHeight: 156,
    }}>
      <div className="bnb-eyebrow" style={{ display: "flex", alignItems: "center", gap: 6 }}>
        {label}
        {live && <span className="bnb-dot live" />}
      </div>
      <div>
        <div className="display tabular" style={{ fontSize: 44, lineHeight: 0.95, letterSpacing: "-0.02em" }}>
          {typeof value === "number" ? value.toLocaleString() : value}
        </div>
        <div className="bnb-meta mono" style={{ marginTop: 6, color: subAccent || "var(--fg-3)" }}>{sub}</div>
      </div>
      {chips && chips.length > 0 && (
        <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
          {chips.map((c, i) => (
            <span key={i} className="mono" style={{ fontSize: 10, padding: "2px 6px", borderRadius: 3,
              background: `color-mix(in oklch, ${c.color} 18%, var(--surface))`, color: c.color, fontWeight: 500 }}>{c.label}</span>
          ))}
        </div>
      )}
      <div style={{ marginTop: "auto" }}>
        {constant ? <ListeningStripe /> : <Sparkline data={trend} width={220} height={26} accent={accentLine} />}
      </div>
    </div>
  );
}

// ─── HeroPulse — a live-feeling waveform + dawn-chorus mini gauge ─────────
function HeroPulse() {
  const canvasRef = React.useRef(null);
  React.useEffect(() => {
    const cnv = canvasRef.current;
    if (!cnv) return;
    const ctx = cnv.getContext("2d");
    const w = cnv.width = 600, h = cnv.height = 80;
    let phase = 0, raf;
    function tick() {
      const dark = document.documentElement.classList.contains("theme-dark");
      ctx.clearRect(0, 0, w, h);
      // baseline
      ctx.fillStyle = dark ? "oklch(80% 0.14 150 / .15)" : "oklch(45% 0.10 150 / .15)";
      // Build a soundwave silhouette
      const bars = 90;
      const bw = (w - bars * 2) / bars;
      for (let i = 0; i < bars; i++) {
        const xt = i / bars;
        const env = Math.sin(xt * Math.PI); // bell
        const v = env * (0.5 + 0.45 * Math.sin(i * 0.4 + phase) + 0.18 * Math.sin(i * 1.7 + phase * 1.5));
        const height = Math.max(4, Math.abs(v) * h * 0.85);
        const op = 0.35 + Math.abs(v) * 0.55;
        ctx.fillStyle = dark
          ? `oklch(80% 0.14 150 / ${op.toFixed(2)})`
          : `oklch(48% 0.10 150 / ${op.toFixed(2)})`;
        const xx = i * (bw + 2);
        ctx.fillRect(xx, (h - height) / 2, bw, height);
      }
      phase += 0.06;
      raf = requestAnimationFrame(tick);
    }
    tick();
    return () => cancelAnimationFrame(raf);
  }, []);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
      <div style={{
        padding: "var(--pad-3)",
        background: "linear-gradient(135deg, var(--surface) 0%, var(--surface-2) 100%)",
        border: "0.5px solid var(--border)",
        borderRadius: "var(--r-lg)",
        boxShadow: "var(--shadow-md)",
      }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: 6 }}>
          <span className="bnb-eyebrow">Live signal · last 30 s</span>
          <span className="bnb-pill moss" style={{ fontSize: 10 }}><span className="bnb-dot live" /> hearing 3 species</span>
        </div>
        <canvas ref={canvasRef} style={{ width: "100%", height: 80, display: "block" }} />
        <div style={{ display: "flex", justifyContent: "space-between", marginTop: 4 }}>
          <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>SNR 14.2 dB</span>
          <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>48 kHz · mono</span>
          <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>inference 48 ms</span>
        </div>
      </div>
      <div style={{ display: "flex", gap: 14, alignItems: "center", padding: "0 8px" }}>
        <DayClock now={6.7} sunrise={5.35} sunset={20.13} />
        <div style={{ flex: 1 }}>
          <div className="bnb-eyebrow" style={{ marginBottom: 4 }}>Dawn chorus window</div>
          <div className="display" style={{ fontSize: 22, lineHeight: 1.1 }}>1h 18m left</div>
          <div className="bnb-meta mono" style={{ marginTop: 4 }}>Peak ends ≈ 8:00 · then the regulars take over</div>
        </div>
      </div>
    </div>
  );
}

function DayClock({ now, sunrise, sunset }) {
  const size = 80, r = size / 2 - 6, cx = size / 2, cy = size / 2;
  const a = (h) => (h / 24) * Math.PI * 2 - Math.PI / 2;
  // night wedge
  const a1 = a(sunset), a2 = a(sunrise + 24);
  const x1 = cx + r * Math.cos(a1), y1 = cy + r * Math.sin(a1);
  const x2 = cx + r * Math.cos(a2), y2 = cy + r * Math.sin(a2);
  const sweep = (a2 - a1) % (Math.PI * 2);
  const large = sweep > Math.PI ? 1 : 0;
  // dawn band: sunrise to sunrise+3
  const dawnStart = a(sunrise);
  const dawnEnd = a(sunrise + 2.7);
  const dx0 = cx + r * Math.cos(dawnStart), dy0 = cy + r * Math.sin(dawnStart);
  const dx1 = cx + r * Math.cos(dawnEnd), dy1 = cy + r * Math.sin(dawnEnd);
  // now hand
  const na = a(now);
  const nx = cx + (r - 2) * Math.cos(na), ny = cy + (r - 2) * Math.sin(na);
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
      <circle cx={cx} cy={cy} r={r} fill="var(--surface)" stroke="var(--border)" />
      <path d={`M${cx},${cy} L${x1},${y1} A${r},${r} 0 ${large} 1 ${x2},${y2} Z`} fill="var(--night)" fillOpacity="0.18" />
      <path d={`M${cx},${cy} L${dx0},${dy0} A${r},${r} 0 0 1 ${dx1},${dy1} Z`} fill="var(--dawn)" fillOpacity="0.45" />
      <line x1={cx} y1={cy} x2={nx} y2={ny} stroke="var(--fg)" strokeWidth={1.5} strokeLinecap="round" />
      <circle cx={cx} cy={cy} r={2.5} fill="var(--fg)" />
      <text x={cx} y={6} textAnchor="middle" className="mono" style={{ fontSize: 7, fill: "var(--fg-3)" }}>12a</text>
      <text x={size - 2} y={cy + 2} textAnchor="end" className="mono" style={{ fontSize: 7, fill: "var(--fg-3)" }}>6a</text>
      <text x={cx} y={size - 1} textAnchor="middle" className="mono" style={{ fontSize: 7, fill: "var(--fg-3)" }}>12p</text>
      <text x={2} y={cy + 2} className="mono" style={{ fontSize: 7, fill: "var(--fg-3)" }}>6p</text>
    </svg>
  );
}

function HeroStat({ label, value, sub, subAccent, trend, live, accentLine, chips, constant }) {
  return (
    <div style={{ borderLeft: "0.5px solid var(--hairline)", paddingLeft: "var(--pad-3)", display: "flex", flexDirection: "column", justifyContent: "space-between", minWidth: 0 }}>
      <div className="bnb-eyebrow">{label}{live && <span className="bnb-dot live" style={{ marginLeft: 6, transform: "translateY(-1px)" }} />}</div>
      <div>
        <div className="display tabular" style={{ fontSize: 36, lineHeight: 1 }}>{typeof value === "number" ? value.toLocaleString() : value}</div>
        <div className="bnb-meta mono" style={{ marginTop: 6, color: subAccent || "var(--fg-3)" }}>{sub}</div>
        {chips && chips.length > 0 && (
          <div style={{ display: "flex", gap: 4, marginTop: 8, flexWrap: "wrap" }}>
            {chips.map((c, i) => (
              <span key={i} className="mono" style={{ fontSize: 10, padding: "2px 6px", borderRadius: 3,
                background: `color-mix(in oklch, ${c.color} 16%, var(--surface))`, color: c.color, fontWeight: 500 }}>{c.label}</span>
            ))}
          </div>
        )}
      </div>
      <div style={{ marginTop: 10 }}>
        {constant
          ? <ListeningStripe />
          : <Sparkline data={trend} width={170} height={22} accent={accentLine} />}
      </div>
    </div>
  );
}

function ListeningStripe() {
  // 24-hour listening stripe — green if recording, gray if not
  return (
    <div style={{ display: "flex", gap: 1, height: 22, alignItems: "end" }}>
      {Array.from({ length: 48 }).map((_, i) => {
        const h = i * 0.5;
        const active = h > 6.3 && h < 20.2; // simulated schedule with morning gap
        const recording = active && !(h > 12.1 && h < 12.4);
        return <span key={i} style={{
          flex: 1,
          height: recording ? `${10 + (Math.sin(h * 0.6) + 1) * 5}px` : 4,
          background: recording ? "var(--moss)" : "var(--bg-2)",
          borderRadius: 1,
        }} />;
      })}
    </div>
  );
}

function FeedRow({ d, fresh }) {
  const { SPECIES } = window.BNB;
  const sp = SPECIES[d.sp];
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "62px 36px 1fr 130px 90px 96px",
        alignItems: "center",
        gap: 12,
        padding: "10px 10px",
        background: fresh ? "color-mix(in oklch, var(--moss) 6%, var(--surface))" : "transparent",
        border: fresh ? "0.5px solid color-mix(in oklch, var(--moss) 30%, var(--border))" : "0.5px solid transparent",
        borderRadius: 8,
        animation: fresh ? "bnb-rise 320ms cubic-bezier(.2,.7,.2,1)" : "none",
        transition: "background 600ms, border 600ms",
      }}
    >
      <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)", textAlign: "right", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{d.ago}</span>
      <SpeciesAvatar sp={d.sp} size={32} />
      <div style={{ minWidth: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ fontWeight: 500, fontSize: 14 }}>{sp.common}</span>
          {d.rare && <span className="bnb-pill rare">rare</span>}
        </div>
        <div className="bnb-meta mono" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{sp.sci} · {d.lat?.toFixed?.(1) ?? "1.3"}s clip</div>
      </div>
      <MiniWaveform seed={d.id} />
      <ConfBar value={d.conf} />
      <button className="bnb-btn ghost" style={{ fontSize: 11.5, padding: "4px 8px", justifyContent: "center" }}>▶  Play</button>

      <style>{`@keyframes bnb-rise { from { opacity: 0; transform: translateY(-6px); } to { opacity: 1; transform: translateY(0); } }`}</style>
    </div>
  );
}

function MiniWaveform({ seed = 1, bars = 24 }) {
  // deterministic pseudo-random envelope
  const arr = useMemo_dash(() => {
    let s = (Number(seed) || 1) * 9301 + 49297;
    const r = () => { s = (s * 9301 + 49297) % 233280; return s / 233280; };
    return Array.from({ length: bars }, (_, i) => {
      const env = Math.sin((i / bars) * Math.PI); // call envelope
      return 0.25 + env * (0.55 + r() * 0.4);
    });
  }, [seed]);
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 1.5, height: 22 }}>
      {arr.map((v, i) => (
        <span key={i} style={{
          width: 2, height: `${Math.round(v * 22)}px`, borderRadius: 1,
          background: `color-mix(in oklch, var(--moss) ${30 + v * 60}%, var(--fg-4))`,
        }} />
      ))}
    </span>
  );
}

// Compact heatmap (24h × 7d) — also used in mobile.
function MiniHeat() {
  const { HEATMAP, DAY_LABELS } = window.BNB;
  return (
    <div>
      <div style={{ display: "flex", gap: 6 }}>
        <div style={{ width: 28 }} />
        <div style={{ flex: 1, display: "grid", gridTemplateColumns: "repeat(24, 1fr)", gap: 2 }}>
          {Array.from({ length: 24 }).map((_, h) => (
            <span key={h} className="mono" style={{ fontSize: 8.5, color: "var(--fg-4)", textAlign: "center" }}>{h % 6 === 0 ? h : ""}</span>
          ))}
        </div>
      </div>
      {HEATMAP.map((row, di) => (
        <div key={di} style={{ display: "flex", gap: 6, alignItems: "center", marginTop: 3 }}>
          <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)", width: 28 }}>{DAY_LABELS[di]}</span>
          <div style={{ flex: 1, display: "grid", gridTemplateColumns: "repeat(24, 1fr)", gap: 2 }}>
            {row.map((v, hi) => (
              <div key={hi} className={`bnb-heat-${v}`} style={{ aspectRatio: "1", borderRadius: 2 }} title={`${DAY_LABELS[di]} ${hi}:00 — ${v}/5`} />
            ))}
          </div>
        </div>
      ))}
      <div style={{ display: "flex", justifyContent: "space-between", marginTop: 10, alignItems: "center" }}>
        <span className="bnb-meta mono">Local time · UTC−5</span>
        <div style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
          <span className="bnb-meta">less</span>
          {[0,1,2,3,4,5].map((v) => <span key={v} className={`bnb-heat-${v}`} style={{ width: 10, height: 10, borderRadius: 2 }} />)}
          <span className="bnb-meta">more</span>
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { Dashboard, MiniHeat });
