// Kiosk mode — auto-rotating dedicated display.
// Eight stations (six day, one bridge, one night). User-toggleable Night Mode
// for calm after-hours display. Quiet-hours can be scheduled.

const { useState: useState_kk, useEffect: useEffect_kk, useRef: useRef_kk } = React;

const KIOSK_DAY_STATIONS = [
  { id: "nowDetection",   label: "Now heard",      Render: NowDetection },
  { id: "dailyPulse",     label: "Daily pulse",    Render: DailyPulse },
  { id: "circadianSky",   label: "Sky arc",        Render: CircadianSky },
  { id: "constellation",  label: "Constellation",  Render: Constellation },
  { id: "soundscapeBloom",label: "Soundscape",     Render: SoundscapeBloom },
  { id: "livingSpectrum", label: "Living spectrum", Render: LivingSpectrum },
  { id: "feedTicker",     label: "Feed",           Render: FeedTicker },
];

function KioskMode() {
  const [stationIdx, setStationIdx] = useState_kk(0);
  const [nightMode, setNightMode] = useState_kk(false);
  const [showControls, setShowControls] = useState_kk(false);

  const stations = nightMode ? [{ id: "night", label: "Night", Render: NightStation }] : KIOSK_DAY_STATIONS;
  const station = stations[stationIdx % stations.length];

  // Auto-rotate
  useEffect_kk(() => {
    if (nightMode) return;
    const t = setInterval(() => setStationIdx((i) => (i + 1) % stations.length), 9000);
    return () => clearInterval(t);
  }, [nightMode, stations.length]);

  const bg = nightMode
    ? "radial-gradient(ellipse at 50% 100%, oklch(10% 0.02 250) 0%, oklch(5% 0.012 250) 60%, oklch(2% 0.008 250) 100%)"
    : "radial-gradient(ellipse at 20% 100%, oklch(20% 0.04 240) 0%, oklch(8% 0.012 240) 60%, oklch(5% 0.008 240) 100%)";

  return (
    <div className="bnb-root theme-dark" style={{
      width: "100%", height: "100%",
      background: bg,
      color: "oklch(94% 0.005 240)",
      overflow: "hidden",
      position: "relative",
      transition: "background .6s",
    }}>
      {/* Aurora — only in day mode */}
      {!nightMode && <Aurora />}

      {/* Header strip */}
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "28px 48px", position: "absolute", top: 0, left: 0, right: 0, zIndex: 5, opacity: nightMode ? 0.45 : 1, transition: "opacity .4s" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <BrandMark size={20} />
          <span className="mono" style={{ fontSize: 12, opacity: 0.7, letterSpacing: "0.08em", textTransform: "uppercase" }}>Behavior · kiosk</span>
          {nightMode && <span className="bnb-pill" style={{ background: "oklch(20% 0.04 250 / .6)", color: "oklch(75% 0.10 250)", border: 0, fontSize: 11, marginLeft: 8 }}><span className="mono">☾</span> night mode</span>}
        </div>
        <div style={{ display: "flex", gap: 18, alignItems: "center" }}>
          {!nightMode && (
            <span className="bnb-pill" style={{ background: "oklch(20% 0.04 150 / .6)", color: "oklch(85% 0.10 150)", border: 0, fontSize: 11 }}>
              <span className="bnb-dot live" /> live
            </span>
          )}
          <span className="mono" style={{ fontSize: 12, opacity: 0.6 }}>{new Date().toLocaleString("en-US", { weekday: "long" })} · 06:42</span>
          <span className="mono" style={{ fontSize: 12, opacity: 0.4 }}>42.36°N · −71.06°W</span>
          <button
            onClick={() => setShowControls((s) => !s)}
            style={{
              padding: "6px 10px", borderRadius: 6, border: "0.5px solid oklch(40% 0.02 240)",
              background: showControls ? "oklch(25% 0.02 240)" : "transparent",
              color: "oklch(82% 0.005 240)", fontSize: 11, cursor: "pointer",
              fontFamily: "var(--font-mono)",
            }}
          >⚙ controls</button>
        </div>
      </div>

      {/* Station content */}
      <div style={{ position: "absolute", inset: 0, padding: nightMode ? "120px 64px 80px" : "100px 56px 56px", display: "flex", flexDirection: "column", justifyContent: "space-between" }}>
        <StationFrame key={station.id} station={station} nightMode={nightMode} />
      </div>

      {/* Station indicator dots */}
      {!nightMode && (
        <div style={{ position: "absolute", bottom: 24, left: "50%", transform: "translateX(-50%)", display: "flex", gap: 8, zIndex: 5 }}>
          {stations.map((s, i) => (
            <button
              key={s.id}
              onClick={() => setStationIdx(i)}
              style={{
                width: i === stationIdx ? 24 : 6, height: 6, borderRadius: 999,
                background: i === stationIdx ? "oklch(80% 0.16 150)" : "oklch(40% 0.02 240)",
                transition: "width .4s, background .4s",
                border: 0, cursor: "pointer", padding: 0,
              }}
              title={s.label}
            />
          ))}
        </div>
      )}

      {/* Control panel — slide in from right */}
      {showControls && (
        <KioskControls
          stations={stations}
          stationIdx={stationIdx}
          setStationIdx={setStationIdx}
          nightMode={nightMode}
          setNightMode={setNightMode}
          onClose={() => setShowControls(false)}
        />
      )}
    </div>
  );
}

// Wrap each station with a fade transition.
function StationFrame({ station, nightMode }) {
  return (
    <div style={{ width: "100%", height: "100%", position: "relative", animation: "kiosk-fade 600ms ease" }}>
      <style>{`@keyframes kiosk-fade { from { opacity: 0; transform: scale(.99); } to { opacity: 1; transform: scale(1); } }`}</style>
      <station.Render nightMode={nightMode} />
    </div>
  );
}

// ─── Aurora — ambient background waves ────────────────────────────────────
function Aurora() {
  return (
    <svg style={{ position: "absolute", inset: 0, pointerEvents: "none", opacity: 0.55 }} viewBox="0 0 1440 900" preserveAspectRatio="none">
      <defs>
        <linearGradient id="aurora1" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0%" stopColor="oklch(60% 0.18 150)" stopOpacity="0.0" />
          <stop offset="40%" stopColor="oklch(60% 0.18 150)" stopOpacity="0.25" />
          <stop offset="100%" stopColor="oklch(60% 0.18 150)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id="aurora2" x1="1" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="oklch(60% 0.18 60)" stopOpacity="0.0" />
          <stop offset="50%" stopColor="oklch(60% 0.18 60)" stopOpacity="0.18" />
          <stop offset="100%" stopColor="oklch(60% 0.18 60)" stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d="M-100,400 Q400,200 720,400 T1500,360 L1500,900 L-100,900 Z" fill="url(#aurora1)">
        <animate attributeName="d" dur="14s" repeatCount="indefinite"
          values="M-100,400 Q400,200 720,400 T1500,360 L1500,900 L-100,900 Z;
                  M-100,460 Q400,260 720,420 T1500,380 L1500,900 L-100,900 Z;
                  M-100,400 Q400,200 720,400 T1500,360 L1500,900 L-100,900 Z" />
      </path>
      <path d="M-100,600 Q500,520 800,560 T1500,540 L1500,900 L-100,900 Z" fill="url(#aurora2)">
        <animate attributeName="d" dur="18s" repeatCount="indefinite"
          values="M-100,600 Q500,520 800,560 T1500,540 L1500,900 L-100,900 Z;
                  M-100,560 Q500,580 800,520 T1500,580 L1500,900 L-100,900 Z;
                  M-100,600 Q500,520 800,560 T1500,560 L1500,900 L-100,900 Z" />
      </path>
    </svg>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// STATIONS
// ═══════════════════════════════════════════════════════════════════════════

// ─── Station 1: "Now detected" — the marquee moment ───────────────────────
function NowDetection() {
  const { SPECIES } = window.BNB;
  const sp = SPECIES[1];
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1.3fr 1fr", gap: 56, alignItems: "center", height: "100%", position: "relative", zIndex: 2 }}>
      <div>
        <div className="mono" style={{ fontSize: 13, letterSpacing: "0.10em", opacity: 0.6, marginBottom: 14 }}>JUST HEARD · 14 SECONDS AGO</div>
        <h1 className="display" style={{ fontSize: 144, lineHeight: 0.92, letterSpacing: "-0.04em", color: "oklch(96% 0.005 240)" }}>
          {sp.common}
        </h1>
        <div style={{ marginTop: 18, fontSize: 22, fontStyle: "italic", opacity: 0.65, fontFamily: "var(--font-display)" }}>{sp.sci}</div>
        <div style={{ marginTop: 36, display: "flex", gap: 36, alignItems: "flex-end" }}>
          <KioskStat label="Confidence" value="0.97" />
          <KioskStat label="118th today" value="118" />
          <KioskStat label="Heard since" value="Mar 12 ’24" />
        </div>
        <div style={{ marginTop: 32, padding: "10px 16px", background: "oklch(15% 0.02 150 / .6)", color: "oklch(88% 0.14 150)", borderRadius: 999, display: "inline-flex", alignItems: "center", gap: 8, fontSize: 13 }}>
          <span className="bnb-dot live" /> A resident bird · most active around dawn
        </div>
      </div>
      <SpeciesPortrait sp={sp} />
    </div>
  );
}

function KioskStat({ label, value }) {
  return (
    <div>
      <div className="mono" style={{ fontSize: 10, opacity: 0.5, textTransform: "uppercase", letterSpacing: "0.12em" }}>{label}</div>
      <div className="display tabular" style={{ fontSize: 36, lineHeight: 1, marginTop: 4, color: "oklch(96% 0.005 240)" }}>{value}</div>
    </div>
  );
}

function SpeciesPortrait({ sp }) {
  return (
    <div style={{ position: "relative", width: "100%", aspectRatio: "1/1", maxWidth: 480, marginLeft: "auto", borderRadius: 20, overflow: "hidden", border: "0.5px solid oklch(35% 0.02 240)", background: "oklch(15% 0.01 240)" }}>
      <BirdPhoto sp={sp} idx={1} slotId="kiosk-species" attribution={false} />
      <div style={{ position: "absolute", left: 16, right: 16, bottom: 16, padding: 12, background: "oklch(10% 0.02 240 / .7)", backdropFilter: "blur(10px)", borderRadius: 10, zIndex: 3 }}>
        <div className="mono" style={{ fontSize: 9.5, opacity: 0.5, letterSpacing: "0.10em", textTransform: "uppercase", marginBottom: 6 }}>This detection</div>
        <div style={{ display: "flex", gap: 1.5, alignItems: "center", height: 36 }}>
          {Array.from({ length: 80 }).map((_, i) => {
            const env = Math.sin((i / 80) * Math.PI);
            const v = 0.2 + env * (0.6 + Math.sin(i * 0.7) * 0.3);
            return <span key={i} style={{ width: 3, height: `${v * 36}px`, background: `oklch(80% 0.14 30 / ${0.5 + v * 0.4})`, borderRadius: 1 }} />;
          })}
        </div>
      </div>
    </div>
  );
}

// ─── Station 2: Daily pulse ───────────────────────────────────────────────
function DailyPulse() {
  return (
    <div style={{ display: "flex", flexDirection: "column", justifyContent: "space-between", height: "100%" }}>
      <div>
        <div className="mono" style={{ fontSize: 13, letterSpacing: "0.10em", opacity: 0.6, marginBottom: 14 }}>TODAY · BACKYARD</div>
        <h1 className="display" style={{ fontSize: 112, lineHeight: 0.95, letterSpacing: "-0.03em", maxWidth: 1100 }}>
          <em style={{ color: "oklch(85% 0.15 150)" }}>912</em> calls heard from <em style={{ color: "oklch(85% 0.15 60)" }}>15</em> species so far.
        </h1>
        <div style={{ marginTop: 24, fontSize: 18, opacity: 0.6, maxWidth: 800, lineHeight: 1.5 }}>
          That's a third more than this time last week. The morning chorus ran from 5:21 a.m. through 7:48 a.m. — about as long as it gets in May.
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 36, paddingTop: 32, borderTop: "0.5px solid oklch(30% 0.02 240)" }}>
        <KioskBigStat label="Loudest species"     value="Cardinal"    sub="118 calls" />
        <KioskBigStat label="Earliest detection"  value="04:48"       sub="Barred Owl" />
        <KioskBigStat label="First-of-year"       value="2"           sub="Magnolia · Grosbeak" />
        <KioskBigStat label="Listening"           value="14h 22m"     sub="0 dropouts" />
      </div>
    </div>
  );
}

function KioskBigStat({ label, value, sub }) {
  return (
    <div>
      <div className="mono" style={{ fontSize: 10, opacity: 0.5, textTransform: "uppercase", letterSpacing: "0.12em" }}>{label}</div>
      <div className="display" style={{ fontSize: 52, lineHeight: 1, marginTop: 8, color: "oklch(96% 0.005 240)" }}>{value}</div>
      <div className="mono" style={{ fontSize: 12, opacity: 0.5, marginTop: 6 }}>{sub}</div>
    </div>
  );
}

// ─── Station 3: Circadian sky — beautiful diurnal arc ────────────────────
function CircadianSky() {
  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", position: "relative" }}>
      <div>
        <div className="mono" style={{ fontSize: 13, letterSpacing: "0.10em", opacity: 0.6, marginBottom: 14 }}>THE 24 HOURS · STACKED</div>
        <h1 className="display" style={{ fontSize: 88, lineHeight: 0.95, letterSpacing: "-0.03em", maxWidth: 1100 }}>
          The chorus has <em style={{ color: "oklch(85% 0.18 60)" }}>1 hour 18 minutes</em> left.
        </h1>
      </div>
      <div style={{ flex: 1, position: "relative", marginTop: 28 }}>
        <SkyArc />
      </div>
    </div>
  );
}

function SkyArc() {
  return (
    <svg viewBox="0 0 1300 460" width="100%" height="100%" preserveAspectRatio="xMidYMax meet">
      <defs>
        <linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%"  stopColor="oklch(70% 0.18 80)" stopOpacity="0.35" />
          <stop offset="100%" stopColor="oklch(70% 0.18 80)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id="ground" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%"  stopColor="oklch(85% 0.18 150)" stopOpacity="0.6" />
          <stop offset="100%" stopColor="oklch(85% 0.18 150)" stopOpacity="0" />
        </linearGradient>
      </defs>
      <line x1="40" y1="380" x2="1260" y2="380" stroke="oklch(40% 0.02 240)" strokeDasharray="3 4" strokeWidth="0.8" />
      <path d="M 40,380 Q 650,40 1260,380" fill="none" stroke="oklch(90% 0.16 60)" strokeWidth="1" strokeDasharray="2 3" />
      <path d="M 40,380 Q 650,40 1260,380 L 1260,380 L 40,380 Z" fill="url(#sky)" />

      {(() => {
        const dawnStart = 5.35 / 24, dawnEnd = 8 / 24;
        const x0 = 40 + dawnStart * 1220, x1 = 40 + dawnEnd * 1220;
        return (
          <g>
            <rect x={x0} y="40" width={x1 - x0} height="340" fill="oklch(80% 0.16 60)" opacity="0.10" />
            <text x={(x0 + x1) / 2} y="60" textAnchor="middle" className="mono" style={{ fontSize: 13, fill: "oklch(85% 0.16 60)", opacity: 0.95 }}>dawn chorus</text>
          </g>
        );
      })()}

      {Array.from({ length: 24 }).map((_, h) => {
        const env = 0.4 * Math.exp(-Math.pow((h - 6.5) / 1.8, 2)) + 0.3 * Math.exp(-Math.pow((h - 18.5) / 2.5, 2)) + (h > 8 && h < 18 ? 0.12 : 0.04);
        const x = 40 + (h / 24) * 1220;
        const w = 1220 / 24;
        const height = env * 280;
        return (
          <rect key={h} x={x} y={380 - height} width={w - 3} height={height} fill="url(#ground)" rx="2" />
        );
      })}

      <g transform={`translate(${40 + (6.7 / 24) * 1220}, 0)`}>
        <line x1="0" y1="40" x2="0" y2="380" stroke="oklch(96% 0.005 240)" strokeWidth="1" />
        <circle cx="0" cy={380 - 0.4 * 280} r="6" fill="oklch(96% 0.005 240)" />
        <text x="0" y={380 - 0.4 * 280 - 16} textAnchor="middle" className="mono" style={{ fontSize: 12, fill: "oklch(96% 0.005 240)" }}>now</text>
      </g>

      {[
        { h: 0,  label: "12a", sub: "" },
        { h: 5.35, label: "☼", sub: "5:21 rise" },
        { h: 12, label: "noon", sub: "" },
        { h: 20.13, label: "☾", sub: "20:08 set" },
      ].map((m, i) => {
        const x = 40 + (m.h / 24) * 1220;
        return (
          <g key={i}>
            <text x={x} y="408" textAnchor="middle" style={{ fontSize: 14, fill: "oklch(96% 0.005 240)", fontFamily: "var(--font-mono)" }}>{m.label}</text>
            <text x={x} y="426" textAnchor="middle" className="mono" style={{ fontSize: 11, fill: "oklch(60% 0.005 240)", opacity: 0.7 }}>{m.sub}</text>
          </g>
        );
      })}
    </svg>
  );
}

// ─── Station 4: Constellation — species as stars in the night sky ─────────
function Constellation() {
  const { SPECIES } = window.BNB;
  // Place top species as stars in a constellation, sized by detection count.
  const stars = useMemo_st(() => {
    const top = [...SPECIES].sort((a, b) => b.count - a.count).slice(0, 14);
    let s = 7;
    const r = () => { s = (s * 9301 + 49297) % 233280; return s / 233280; };
    // Cluster into ~3 zones (resident, visitor, rare)
    return top.map((sp, i) => {
      const cluster = i < 4 ? 0 : i < 9 ? 1 : 2;
      const cx = [320, 800, 1180][cluster];
      const cy = [380, 280, 460][cluster];
      const spread = [180, 220, 160][cluster];
      return {
        sp,
        x: cx + (r() - 0.5) * spread,
        y: cy + (r() - 0.5) * spread,
        size: 6 + Math.sqrt(sp.count) * 1.2,
        glow: sp.count > 30,
      };
    });
  }, []);

  // Build connections — Delaunay-ish nearest neighbors
  const connections = useMemo_st(() => {
    const cs = [];
    for (let i = 0; i < stars.length; i++) {
      const dists = stars.map((s, j) => ({ j, d: Math.hypot(s.x - stars[i].x, s.y - stars[i].y) }))
                          .filter((x) => x.j > i)
                          .sort((a, b) => a.d - b.d)
                          .slice(0, 2);
      for (const { j, d } of dists) {
        if (d < 280) cs.push([i, j, d]);
      }
    }
    return cs;
  }, [stars]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div>
        <div className="mono" style={{ fontSize: 13, letterSpacing: "0.10em", opacity: 0.6, marginBottom: 14 }}>YOUR YARD · AS A CONSTELLATION</div>
        <h1 className="display" style={{ fontSize: 76, lineHeight: 0.95, letterSpacing: "-0.03em" }}>
          The brightest stars are the <em style={{ color: "oklch(88% 0.14 60)" }}>regulars</em>.
        </h1>
      </div>
      <div style={{ flex: 1, position: "relative" }}>
        <svg viewBox="0 0 1400 600" width="100%" height="100%" preserveAspectRatio="xMidYMid meet">
          {/* faint background stars */}
          {Array.from({ length: 80 }).map((_, i) => {
            let seed = i * 137;
            const r = () => { seed = (seed * 9301 + 49297) % 233280; return seed / 233280; };
            return (
              <circle key={i} cx={r() * 1400} cy={r() * 600} r={0.3 + r() * 0.8} fill="oklch(85% 0.005 240)" opacity={0.15 + r() * 0.3}>
                <animate attributeName="opacity" dur={`${3 + r() * 4}s`} repeatCount="indefinite"
                  values={`${0.15 + r() * 0.2};${0.4 + r() * 0.3};${0.15 + r() * 0.2}`} />
              </circle>
            );
          })}

          {/* connections */}
          {connections.map(([i, j], k) => (
            <line key={k}
              x1={stars[i].x} y1={stars[i].y}
              x2={stars[j].x} y2={stars[j].y}
              stroke="oklch(70% 0.06 240)" strokeWidth="0.6" strokeOpacity="0.22"
              strokeDasharray="2 3" />
          ))}

          {/* stars */}
          {stars.map((s, i) => (
            <g key={i}>
              {s.glow && (
                <circle cx={s.x} cy={s.y} r={s.size * 3} fill={s.sp.color} opacity="0.06">
                  <animate attributeName="opacity" dur={`${3 + i * 0.3}s`} repeatCount="indefinite" values="0.04;0.10;0.04" />
                </circle>
              )}
              <circle cx={s.x} cy={s.y} r={s.size} fill={s.sp.color} opacity="0.85">
                <animate attributeName="r" dur={`${2 + i * 0.4}s`} repeatCount="indefinite"
                  values={`${s.size};${s.size + 1};${s.size}`} />
              </circle>
              {/* cross-glint */}
              <line x1={s.x - s.size * 2} y1={s.y} x2={s.x + s.size * 2} y2={s.y} stroke={s.sp.color} strokeWidth="0.4" opacity="0.5" />
              <line x1={s.x} y1={s.y - s.size * 2} x2={s.x} y2={s.y + s.size * 2} stroke={s.sp.color} strokeWidth="0.4" opacity="0.5" />
              {/* label for the top 6 */}
              {i < 6 && (
                <text x={s.x + s.size + 8} y={s.y + 4} className="mono" style={{ fontSize: 11.5, fill: "oklch(90% 0.005 240)" }}>
                  {s.sp.common}
                  <tspan x={s.x + s.size + 8} dy="13" style={{ fontSize: 9.5, fill: "oklch(60% 0.005 240)" }}>{s.sp.count}</tspan>
                </text>
              )}
            </g>
          ))}

          {/* cluster labels */}
          <g style={{ fill: "oklch(70% 0.008 240)", fontFamily: "var(--font-mono)", fontSize: 10, letterSpacing: "0.12em", textTransform: "uppercase" }}>
            <text x="320" y="180" textAnchor="middle">— Residents</text>
            <text x="800" y="80"  textAnchor="middle">— Visitors</text>
            <text x="1180" y="260" textAnchor="middle">— Rare</text>
          </g>
        </svg>
      </div>
    </div>
  );
}

// ─── Station 5: Soundscape Bloom — radial bloom of today's species ────────
function SoundscapeBloom() {
  const { SPECIES } = window.BNB;
  const top = [...SPECIES].sort((a, b) => b.count - a.count).slice(0, 10);
  const max = Math.max(...top.map((s) => s.count));
  const cx = 700, cy = 360, rIn = 40, rOut = 260;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div>
        <div className="mono" style={{ fontSize: 13, letterSpacing: "0.10em", opacity: 0.6, marginBottom: 14 }}>TODAY · IN BLOOM</div>
        <h1 className="display" style={{ fontSize: 76, lineHeight: 0.95, letterSpacing: "-0.03em" }}>
          The yard's <em style={{ color: "oklch(85% 0.15 150)" }}>soundscape</em>.
        </h1>
      </div>
      <div style={{ flex: 1 }}>
        <svg viewBox="0 0 1400 700" width="100%" height="100%" preserveAspectRatio="xMidYMid meet">
          {/* radial axes */}
          {top.map((_, i) => {
            const a = (i / top.length) * Math.PI * 2 - Math.PI / 2;
            return <line key={i} x1={cx} y1={cy} x2={cx + (rOut + 30) * Math.cos(a)} y2={cy + (rOut + 30) * Math.sin(a)}
                         stroke="oklch(35% 0.02 240)" strokeWidth="0.4" />;
          })}
          {/* radial rings */}
          {[0.25, 0.5, 0.75, 1.0].map((f, i) => (
            <circle key={i} cx={cx} cy={cy} r={rIn + (rOut - rIn) * f} fill="none"
                    stroke="oklch(40% 0.02 240)" strokeWidth="0.4" strokeDasharray="3 4" opacity="0.4" />
          ))}

          {/* petals */}
          {top.map((sp, i) => {
            const a0 = (i / top.length) * Math.PI * 2 - Math.PI / 2 - 0.15;
            const a1 = (i / top.length) * Math.PI * 2 - Math.PI / 2 + 0.15;
            const len = rIn + (sp.count / max) * (rOut - rIn);
            const peak = (a0 + a1) / 2;
            const pX = (r, a) => cx + r * Math.cos(a);
            const pY = (r, a) => cy + r * Math.sin(a);

            return (
              <g key={i}>
                <path
                  d={`M${pX(rIn, a0)},${pY(rIn, a0)} Q${pX(len * 0.7, peak)},${pY(len * 0.7, peak)} ${pX(len, peak)},${pY(len, peak)} Q${pX(len * 0.7, peak)},${pY(len * 0.7, peak)} ${pX(rIn, a1)},${pY(rIn, a1)} Z`}
                  fill={sp.color} fillOpacity="0.55" stroke={sp.color} strokeWidth="1.4" strokeOpacity="0.9">
                  <animate attributeName="fill-opacity" dur={`${4 + i * 0.5}s`} repeatCount="indefinite"
                    values="0.55;0.75;0.55" />
                </path>
                {/* label */}
                <g transform={`translate(${pX(rOut + 50, peak)}, ${pY(rOut + 50, peak)})`}>
                  <text textAnchor="middle" style={{ fontSize: 13, fill: "oklch(96% 0.005 240)", fontWeight: 500 }}>{sp.common}</text>
                  <text textAnchor="middle" y="14" className="mono" style={{ fontSize: 10, fill: "oklch(60% 0.005 240)" }}>{sp.count}</text>
                </g>
              </g>
            );
          })}

          {/* center */}
          <circle cx={cx} cy={cy} r={rIn - 6} fill="oklch(15% 0.02 240)" stroke="oklch(40% 0.02 240)" />
          <text x={cx} y={cy - 4} textAnchor="middle" className="display" style={{ fontSize: 30, fill: "oklch(96% 0.005 240)" }}>912</text>
          <text x={cx} y={cy + 16} textAnchor="middle" className="mono" style={{ fontSize: 10, fill: "oklch(60% 0.005 240)" }}>calls</text>
        </svg>
      </div>
    </div>
  );
}

// ─── Station 6: Living Spectrum — fullscreen flowing spectrogram ─────────
function LivingSpectrum() {
  const canvasRef = useRef_kk(null);

  useEffect_kk(() => {
    const cnv = canvasRef.current;
    if (!cnv) return;
    const ctx = cnv.getContext("2d");
    const W = cnv.width = 1400;
    const H = cnv.height = 380;
    let chirps = [];
    let frame = 0, raf;

    function tick() {
      // shift left
      const prev = ctx.getImageData(2, 0, W - 2, H);
      ctx.putImageData(prev, 0, 0);
      // wipe right column
      ctx.fillStyle = "oklch(8% 0.012 240)";
      ctx.fillRect(W - 2, 0, 2, H);

      // background sparkle noise
      for (let y = 0; y < H; y++) {
        if (Math.random() < 0.03) {
          ctx.fillStyle = `oklch(70% 0.005 240 / ${(Math.random() * 0.20).toFixed(2)})`;
          ctx.fillRect(W - 2, y, 2, 1);
        }
      }

      // schedule chirps
      if (Math.random() < 0.06) {
        chirps.push({
          t: 0, dur: 60 + Math.random() * 80,
          fStart: 60 + Math.random() * 240,
          fEnd: 80 + Math.random() * 220,
          color: ["150", "60", "30", "100", "200"][Math.floor(Math.random() * 5)],
          amp: 0.7 + Math.random() * 0.3,
        });
      }

      chirps = chirps.filter((c) => {
        if (c.t < c.dur) {
          const u = c.t / c.dur;
          const f = c.fStart * (1 - u) + c.fEnd * u;
          for (let yy = -4; yy <= 4; yy++) {
            const y = Math.round(f + yy);
            if (y < 0 || y >= H) continue;
            const fall = Math.exp(-(yy * yy) / 2.4);
            const env = 1 - Math.abs(0.5 - u) * 1.4;
            const a = c.amp * fall * env;
            if (a > 0.04) {
              ctx.fillStyle = `oklch(82% 0.16 ${c.color} / ${a.toFixed(2)})`;
              ctx.fillRect(W - 2, y, 2, 1);
            }
          }
          c.t++;
          return true;
        }
        return false;
      });

      frame++;
      raf = requestAnimationFrame(tick);
    }
    tick();
    return () => cancelAnimationFrame(raf);
  }, []);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div>
        <div className="mono" style={{ fontSize: 13, letterSpacing: "0.10em", opacity: 0.6, marginBottom: 14 }}>LIVE · LAST 90 SECONDS</div>
        <h1 className="display" style={{ fontSize: 76, lineHeight: 0.95, letterSpacing: "-0.03em" }}>
          A river of <em style={{ color: "oklch(85% 0.15 150)" }}>sound</em>.
        </h1>
      </div>
      <div style={{ flex: 1, position: "relative", marginTop: 24, borderRadius: 16, overflow: "hidden", border: "0.5px solid oklch(28% 0.02 240)" }}>
        <canvas ref={canvasRef} style={{ width: "100%", height: "100%", imageRendering: "pixelated" }} />
        {/* freq labels */}
        <div style={{ position: "absolute", top: 12, left: 14, display: "flex", flexDirection: "column", gap: 4 }}>
          {["12 kHz", "9 kHz", "6 kHz", "3 kHz"].map((l) => (
            <span key={l} className="mono" style={{ fontSize: 10, color: "oklch(70% 0.005 240)", opacity: 0.6, textShadow: "0 0 4px oklch(0% 0 0)" }}>{l}</span>
          ))}
        </div>
        {/* now line */}
        <div style={{ position: "absolute", right: 0, top: 0, bottom: 0, width: 2, background: "linear-gradient(180deg, transparent, oklch(80% 0.16 150), transparent)", boxShadow: "0 0 12px oklch(80% 0.16 150)" }} />
        {/* floating species labels — anchored to recent chirps */}
        <div style={{ position: "absolute", inset: 0, pointerEvents: "none" }}>
          <FloatingLabel name="Cardinal"  color="oklch(60% 0.18 25)"  right="22%" top="32%" />
          <FloatingLabel name="Chickadee" color="oklch(70% 0.02 80)"  right="34%" top="56%" />
          <FloatingLabel name="Blue Jay"  color="oklch(64% 0.14 240)" right="48%" top="22%" />
          <FloatingLabel name="Robin"     color="oklch(60% 0.14 50)"  right="62%" top="48%" />
        </div>
      </div>
    </div>
  );
}

function FloatingLabel({ name, color, right, top }) {
  return (
    <span style={{
      position: "absolute", right, top,
      padding: "2px 8px", borderRadius: 4,
      background: "oklch(8% 0.02 240 / .8)",
      backdropFilter: "blur(6px)",
      border: `0.5px solid ${color}`,
      color, fontSize: 11, fontFamily: "var(--font-mono)",
    }}>{name}</span>
  );
}

// ─── Station 7: Live feed ticker ──────────────────────────────────────────
function FeedTicker() {
  const { SPECIES } = window.BNB;
  const items = [
    { sp: 1, time: "07:14", conf: 0.97 },
    { sp: 3, time: "07:13", conf: 0.91 },
    { sp: 0, time: "07:11", conf: 0.94 },
    { sp: 2, time: "07:09", conf: 0.89 },
    { sp: 5, time: "07:06", conf: 0.93 },
    { sp: 1, time: "07:04", conf: 0.96 },
    { sp: 6, time: "07:02", conf: 0.88 },
  ];
  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div>
        <div className="mono" style={{ fontSize: 13, letterSpacing: "0.10em", opacity: 0.6, marginBottom: 14 }}>LAST 10 MINUTES</div>
        <h1 className="display" style={{ fontSize: 88, lineHeight: 0.95, letterSpacing: "-0.03em" }}>
          The yard is busy.
        </h1>
      </div>
      <div style={{ flex: 1, marginTop: 36, display: "flex", flexDirection: "column", justifyContent: "center", gap: 6 }}>
        {items.map((d, i) => {
          const sp = SPECIES[d.sp];
          return (
            <div key={i} style={{
              display: "grid", gridTemplateColumns: "80px auto 1fr 100px",
              alignItems: "center", gap: 24,
              padding: "12px 0",
              borderBottom: i < items.length - 1 ? "0.5px solid oklch(30% 0.02 240)" : "none",
              opacity: 1 - (i * 0.07),
            }}>
              <span className="mono" style={{ fontSize: 16, opacity: 0.7 }}>{d.time}</span>
              <SpeciesAvatar sp={d.sp} size={36} />
              <div>
                <span className="display" style={{ fontSize: 28, color: "oklch(96% 0.005 240)" }}>{sp.common}</span>
              </div>
              <span className="mono tabular" style={{ fontSize: 14, opacity: 0.6, textAlign: "right" }}>{d.conf.toFixed(2)}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ─── NIGHT STATION — the calm after-hours display ─────────────────────────
function NightStation() {
  const { SPECIES } = window.BNB;
  const sp = SPECIES[14]; // Barred Owl
  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", justifyContent: "center", alignItems: "center", textAlign: "center", gap: 24 }}>
      {/* Moon */}
      <svg width="180" height="180" viewBox="0 0 180 180" style={{ opacity: 0.85 }}>
        <defs>
          <radialGradient id="moon-glow" cx="50%" cy="50%" r="50%">
            <stop offset="0%"  stopColor="oklch(90% 0.04 80)" stopOpacity="1" />
            <stop offset="70%" stopColor="oklch(90% 0.04 80)" stopOpacity="0.6" />
            <stop offset="100%" stopColor="oklch(90% 0.04 80)" stopOpacity="0" />
          </radialGradient>
        </defs>
        <circle cx="90" cy="90" r="86" fill="url(#moon-glow)" opacity="0.32">
          <animate attributeName="r" dur="6s" repeatCount="indefinite" values="84;90;84" />
        </circle>
        <circle cx="90" cy="90" r="54" fill="oklch(88% 0.012 80)" />
        {/* craters */}
        <circle cx="74" cy="76" r="6"  fill="oklch(76% 0.012 80)" />
        <circle cx="102" cy="98" r="8" fill="oklch(76% 0.012 80)" />
        <circle cx="86" cy="106" r="4" fill="oklch(78% 0.012 80)" />
        <circle cx="106" cy="74" r="3" fill="oklch(78% 0.012 80)" />
      </svg>

      <div>
        <div className="mono" style={{ fontSize: 12, letterSpacing: "0.18em", opacity: 0.5, textTransform: "uppercase" }}>Listening quietly</div>
        <h1 className="display" style={{ fontSize: 96, lineHeight: 0.95, marginTop: 14, color: "oklch(96% 0.005 240)", letterSpacing: "-0.025em" }}>
          11:38 p.m.
        </h1>
        <div style={{ marginTop: 12, fontSize: 18, opacity: 0.55, fontFamily: "var(--font-display)", fontStyle: "italic" }}>
          Two Barred Owls have been talking since 9:47.
        </div>
      </div>

      <div style={{ display: "flex", gap: 40, marginTop: 16, opacity: 0.75 }}>
        <NightStat label="Tonight" value="18" sub="detections" />
        <NightStat label="Last call" value="11 min ago" sub="Barred Owl" />
        <NightStat label="Wake-up" value="04:48" sub="early Cardinals" />
      </div>

      <div className="mono" style={{ fontSize: 10.5, opacity: 0.4, marginTop: 24, letterSpacing: "0.10em" }}>
        DISPLAY DIMMED · WILL RESUME AT 06:00
      </div>
    </div>
  );
}

function NightStat({ label, value, sub }) {
  return (
    <div style={{ textAlign: "center" }}>
      <div className="mono" style={{ fontSize: 9.5, opacity: 0.5, textTransform: "uppercase", letterSpacing: "0.14em" }}>{label}</div>
      <div className="display" style={{ fontSize: 26, lineHeight: 1, marginTop: 6 }}>{value}</div>
      <div className="mono" style={{ fontSize: 10, opacity: 0.4, marginTop: 4 }}>{sub}</div>
    </div>
  );
}

// ─── Controls panel ────────────────────────────────────────────────────────
function KioskControls({ stations, stationIdx, setStationIdx, nightMode, setNightMode, onClose }) {
  return (
    <div style={{
      position: "absolute", top: 80, right: 24, zIndex: 20,
      width: 320,
      background: "oklch(15% 0.012 240 / .92)",
      backdropFilter: "blur(20px)",
      border: "0.5px solid oklch(35% 0.02 240)",
      borderRadius: 14,
      padding: 18,
      color: "oklch(94% 0.005 240)",
      boxShadow: "0 24px 60px oklch(0% 0 0 / .55)",
      animation: "kiosk-fade 200ms ease",
    }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <span className="mono" style={{ fontSize: 11, opacity: 0.6, letterSpacing: "0.10em", textTransform: "uppercase" }}>Kiosk controls</span>
        <button onClick={onClose} style={{ background: "transparent", border: 0, color: "oklch(70% 0.005 240)", fontSize: 14, cursor: "pointer" }}>×</button>
      </div>

      <ControlSection label="Display">
        <ControlToggle label="Night mode" sub="Dim, plain, calmer" on={nightMode} onChange={() => setNightMode(!nightMode)} />
        <ControlRow label="Quiet hours" value="22:00 – 06:00" />
        <ControlRow label="Auto-advance" value="every 9 s" />
      </ControlSection>

      {!nightMode && (
        <ControlSection label="Jump to station">
          {stations.map((s, i) => (
            <button key={s.id} onClick={() => setStationIdx(i)} style={{
              display: "flex", alignItems: "center", gap: 8, padding: "6px 0", border: 0, background: "transparent", color: "inherit", cursor: "pointer", width: "100%", textAlign: "left",
            }}>
              <span style={{ width: 8, height: 8, borderRadius: 999, background: i === stationIdx ? "oklch(80% 0.16 150)" : "oklch(35% 0.02 240)" }} />
              <span style={{ fontSize: 12.5 }}>{s.label}</span>
            </button>
          ))}
        </ControlSection>
      )}

      <ControlSection label="Output">
        <ControlRow label="Resolution" value="1920 × 1080" />
        <ControlRow label="Brightness" value="78%" />
        <ControlRow label="Burn-in protect" value="pixel shift · on" />
      </ControlSection>

      <div style={{ marginTop: 14, paddingTop: 14, borderTop: "0.5px solid oklch(30% 0.02 240)", display: "flex", justifyContent: "space-between" }}>
        <span className="mono" style={{ fontSize: 10, opacity: 0.5 }}>tap-and-hold any spot to open later</span>
        <a href="#" className="mono" style={{ fontSize: 10, color: "oklch(80% 0.10 150)" }}>↗ docs</a>
      </div>
    </div>
  );
}

function ControlSection({ label, children }) {
  return (
    <div style={{ marginBottom: 16 }}>
      <div className="mono" style={{ fontSize: 9.5, opacity: 0.5, letterSpacing: "0.10em", textTransform: "uppercase", marginBottom: 8 }}>{label}</div>
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>{children}</div>
    </div>
  );
}

function ControlToggle({ label, sub, on, onChange }) {
  return (
    <button onClick={onChange} style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "8px 10px", borderRadius: 8, background: "oklch(22% 0.012 240)", border: "0.5px solid oklch(32% 0.014 240)", color: "inherit", cursor: "pointer", width: "100%" }}>
      <div style={{ textAlign: "left" }}>
        <div style={{ fontSize: 13, fontWeight: 500 }}>{label}</div>
        <div className="mono" style={{ fontSize: 10, opacity: 0.5, marginTop: 2 }}>{sub}</div>
      </div>
      <span style={{
        width: 36, height: 20, borderRadius: 999, padding: 2,
        background: on ? "oklch(70% 0.14 150)" : "oklch(30% 0.02 240)",
        display: "inline-flex", alignItems: "center", transition: "background .2s",
      }}>
        <span style={{ width: 16, height: 16, borderRadius: 999, background: "oklch(96% 0.005 240)", transform: on ? "translateX(16px)" : "translateX(0)", transition: "transform .2s" }} />
      </span>
    </button>
  );
}

function ControlRow({ label, value }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", padding: "6px 10px", fontSize: 12 }}>
      <span style={{ opacity: 0.65 }}>{label}</span>
      <span className="mono" style={{ opacity: 0.85 }}>{value}</span>
    </div>
  );
}

// useMemo polyfill local
const { useMemo: useMemo_st } = React;

Object.assign(window, { KioskMode });
