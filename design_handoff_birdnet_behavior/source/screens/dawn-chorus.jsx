// Dawn-chorus circadian plot — a polar (sundial-style) visualization.
// Each species is a radial ribbon over the 24-hour clock; the layered shape
// answers "who sings, and when?" at a single glance.

const { useMemo: useMemo_dc } = React;

function DawnChorus() {
  const { CHORUS, SPECIES } = window.BNB;

  return (
    <Screen>
      <TopNav active="Analytics" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Behavioral analytics · circadian</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>The dawn chorus, by species</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 540 }}>
            Each ribbon is one species's activity over the 24-hour clock, averaged across the last 60 days. Ribbon thickness = call rate; position = time of day. Sunrise <span className="mono">05:21</span>, sunset <span className="mono">20:08</span>.
          </div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <span className="bnb-pill">Last 60 days</span>
          <span className="bnb-pill">Per-species normalized</span>
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1.05fr 0.95fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", alignItems: "center", justifyContent: "center", minHeight: 0 }}>
          <PolarChorus />
        </div>
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 10, minHeight: 0 }}>
          <SectionHeader eyebrow="Selected species" title="Activity ribbons" />
          <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
            {CHORUS.map((c, i) => {
              const sp = SPECIES[c.sp];
              const peakH = c.hours.indexOf(Math.max(...c.hours));
              return (
                <div key={i} style={{
                  display: "grid", gridTemplateColumns: "auto 1fr auto",
                  alignItems: "center", gap: 12, padding: "10px 0",
                  borderTop: i > 0 ? "0.5px solid var(--hairline)" : "0",
                }}>
                  <SpeciesAvatar sp={c.sp} size={28} />
                  <div style={{ minWidth: 0 }}>
                    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                      <span style={{ fontWeight: 500 }}>{sp.common}</span>
                      <span className="bnb-meta mono">peak {fmtHour(peakH)}</span>
                    </div>
                    <div style={{ marginTop: 4 }}>
                      <HourRibbon data={c.hours} accent={sp.color} />
                    </div>
                  </div>
                  <CircadianRing data={c.hours} size={32} accent={sp.color} />
                </div>
              );
            })}
          </div>
          <div className="bnb-meta" style={{ paddingTop: 4, borderTop: "0.5px solid var(--hairline)" }}>
            Owl call moved out of the dawn band — appears in the <span className="mono">02:00</span> ring on the polar plot.
          </div>
        </div>
      </div>
    </Screen>
  );
}

function fmtHour(h) {
  const hh = ((h % 24) + 24) % 24;
  if (hh === 0) return "12 a";
  if (hh === 12) return "12 p";
  return hh < 12 ? `${hh} a` : `${hh - 12} p`;
}

// ─── Stacked polar ribbons ────────────────────────────────────────────────
function PolarChorus() {
  const { CHORUS, SPECIES } = window.BNB;
  const size = 520;
  const cx = size / 2, cy = size / 2;
  const ringMin = 70;  // inner clear area
  const ringMax = 220; // outer cap
  const N = CHORUS.length;
  const ringStep = (ringMax - ringMin) / (N + 1);

  return (
    <svg viewBox={`0 0 ${size} ${size}`} width="100%" height="100%" style={{ maxWidth: 540, maxHeight: 540 }}>
      <defs>
        {/* Night/day gradient — dim night band, bright day */}
        <radialGradient id="night-day" cx="50%" cy="50%" r="50%">
          <stop offset="0%" stopColor="var(--surface)" />
          <stop offset="100%" stopColor="var(--bg)" />
        </radialGradient>
      </defs>

      {/* night band: 20:00 → 05:00 wedge */}
      <DayNightWedge cx={cx} cy={cy} sunrise={5.35} sunset={20.13} rMax={ringMax + 14} />

      {/* hour ticks */}
      {Array.from({ length: 24 }).map((_, h) => {
        const a = hourToAngle(h);
        const r1 = ringMax + 6, r2 = ringMax + 14;
        const big = h % 6 === 0;
        const x1 = cx + r1 * Math.cos(a), y1 = cy + r1 * Math.sin(a);
        const x2 = cx + r2 * Math.cos(a), y2 = cy + r2 * Math.sin(a);
        return <line key={h} x1={x1} y1={y1} x2={x2} y2={y2} stroke={big ? "var(--fg-3)" : "var(--border)"} strokeWidth={big ? 1 : 0.5} />;
      })}
      {/* Hour labels */}
      {[0, 3, 6, 9, 12, 15, 18, 21].map((h) => {
        const a = hourToAngle(h);
        const r = ringMax + 26;
        const x = cx + r * Math.cos(a), y = cy + r * Math.sin(a);
        const label = h === 0 ? "12a" : h === 12 ? "12p" : h < 12 ? `${h}a` : `${h - 12}p`;
        return <text key={h} x={x} y={y} textAnchor="middle" dominantBaseline="central" className="mono" style={{ fontSize: 11, fill: "var(--fg-3)" }}>{label}</text>;
      })}

      {/* Sunrise / sunset markers */}
      <SunMarker cx={cx} cy={cy} hour={5.35} r={ringMax + 14} kind="rise" />
      <SunMarker cx={cx} cy={cy} hour={20.13} r={ringMax + 14} kind="set" />

      {/* Stacked ribbons — outer = first species in CHORUS */}
      {CHORUS.map((c, i) => {
        const baseR = ringMax - (i + 1) * ringStep;
        const sp = SPECIES[c.sp];
        const path = ribbonPath(cx, cy, baseR, c.hours, ringStep * 1.1);
        return <path key={i} d={path} fill={sp.color} fillOpacity={0.55} stroke={sp.color} strokeOpacity={0.85} strokeWidth={0.8} />;
      })}

      {/* Center label */}
      <circle cx={cx} cy={cy} r={ringMin - 8} fill="var(--surface)" stroke="var(--hairline)" />
      <text x={cx} y={cy - 10} textAnchor="middle" className="display" style={{ fontSize: 14, fill: "var(--fg-3)" }}>chorus</text>
      <text x={cx} y={cy + 10} textAnchor="middle" className="mono" style={{ fontSize: 11, fill: "var(--fg-2)" }}>24 h</text>

      {/* Current time hand */}
      <CurrentTimeHand cx={cx} cy={cy} rInner={ringMin - 4} rOuter={ringMax + 14} />
    </svg>
  );
}

function hourToAngle(h) {
  // 0h at top, clockwise
  return (h / 24) * Math.PI * 2 - Math.PI / 2;
}

function ribbonPath(cx, cy, baseR, hours, amp) {
  const max = Math.max(0.001, ...hours);
  const N = hours.length;
  const subdiv = 4; // smoothing
  const outer = [];
  const inner = [];
  for (let i = 0; i < N * subdiv + 1; i++) {
    const h = (i / subdiv) % 24;
    const fl = Math.floor(h);
    const t = h - fl;
    const v = (hours[fl % N] * (1 - t) + hours[(fl + 1) % N] * t) / max;
    const a = hourToAngle(h);
    const rO = baseR + v * amp * 0.95;
    const rI = baseR - v * amp * 0.15; // small carve toward center for shape
    outer.push([cx + rO * Math.cos(a), cy + rO * Math.sin(a)]);
    inner.push([cx + rI * Math.cos(a), cy + rI * Math.sin(a)]);
  }
  const outD = outer.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`).join(" ");
  const inD = inner.reverse().map(([x, y]) => `L${x.toFixed(2)},${y.toFixed(2)}`).join(" ");
  return `${outD} ${inD} Z`;
}

function DayNightWedge({ cx, cy, sunrise, sunset, rMax }) {
  const a1 = hourToAngle(sunset);
  const a2 = hourToAngle(sunrise + 24);
  const x1 = cx + rMax * Math.cos(a1), y1 = cy + rMax * Math.sin(a1);
  const x2 = cx + rMax * Math.cos(a2), y2 = cy + rMax * Math.sin(a2);
  const sweep = ((a2 - a1) % (Math.PI * 2));
  const large = sweep > Math.PI ? 1 : 0;
  return (
    <path
      d={`M${cx},${cy} L${x1},${y1} A${rMax},${rMax} 0 ${large} 1 ${x2},${y2} Z`}
      fill="var(--night)"
      fillOpacity="0.06"
    />
  );
}

function SunMarker({ cx, cy, hour, r, kind }) {
  const a = hourToAngle(hour);
  const x = cx + r * Math.cos(a), y = cy + r * Math.sin(a);
  const fill = kind === "rise" ? "var(--dawn)" : "var(--dawn-ink)";
  return (
    <g>
      <circle cx={x} cy={y} r="3.5" fill={fill} />
      <text x={x} y={y + (Math.sin(a) > 0 ? 16 : -10)} textAnchor="middle" className="mono" style={{ fontSize: 9.5, fill: "var(--fg-3)" }}>
        {kind === "rise" ? "☼ rise 5:21" : "☾ set 20:08"}
      </text>
    </g>
  );
}

function CurrentTimeHand({ cx, cy, rInner, rOuter }) {
  // 6:42 a.m. for a hand-set dramatic moment
  const a = hourToAngle(6 + 42 / 60);
  const x1 = cx + rInner * Math.cos(a), y1 = cy + rInner * Math.sin(a);
  const x2 = cx + rOuter * Math.cos(a), y2 = cy + rOuter * Math.sin(a);
  return (
    <g>
      <line x1={x1} y1={y1} x2={x2} y2={y2} stroke="var(--fg)" strokeWidth={1.5} strokeDasharray="2 3" />
      <circle cx={x2} cy={y2} r="3" fill="var(--fg)" />
    </g>
  );
}

function HourRibbon({ data, accent }) {
  // small linear strip showing hourly activity
  const max = Math.max(0.001, ...data);
  return (
    <svg width="100%" height="14" viewBox={`0 0 240 14`} preserveAspectRatio="none">
      {data.map((v, i) => {
        const op = 0.08 + (v / max) * 0.82;
        return <rect key={i} x={(i / 24) * 240} y={0} width={240 / 24 - 0.6} height="14" rx="1.5" fill={accent} fillOpacity={op} />;
      })}
    </svg>
  );
}

Object.assign(window, { DawnChorus });
