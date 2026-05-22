// Species co-occurrence — bigger, tighter matrix with rich drill-in.
const { useMemo: useMemo_co, useState: useState_co } = React;

function CoOccurrence() {
  const { COOC, COOC_SPECIES, SPECIES } = window.BNB;
  const N = COOC_SPECIES.length;
  const [hovered, setHovered] = useState_co({ i: 3, j: 6 });

  const order = useMemo_co(() => {
    const sums = COOC.map((row) => row.reduce((s, v) => s + v, 0));
    return [...Array(N).keys()].sort((a, b) => sums[b] - sums[a]);
  }, []);

  const cell = 54;
  const labelW = 150;

  return (
    <Screen>
      <TopNav active="Analytics" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Behavioral analytics · co-occurrence</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Who sings with whom</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 560 }}>
            Within a rolling 5-minute window. Cell shade is the Spearman ρ between two species' per-hour detection counts over 60 days. Hover for the pair, click to drill.
          </div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <span className="bnb-pill">5-min window</span>
          <span className="bnb-pill">60 days</span>
          <span className="bnb-pill">Top 8 species</span>
          <span className="bnb-pill">Spearman ρ</span>
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1.15fr 1fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        {/* Matrix */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", minHeight: 0 }}>
          <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 12 }}>
            <SectionHeader eyebrow="Pairwise correlation" title="Matrix" />
            <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
              <span className="bnb-meta">0.0</span>
              {[0,0.2,0.4,0.6,0.8,1.0].map((v) => (
                <span key={v} style={{ width: 22, height: 10, borderRadius: 2, background: `color-mix(in oklch, var(--moss) ${Math.round(v * 78)}%, var(--surface-2))` }} />
              ))}
              <span className="bnb-meta">1.0</span>
            </div>
          </div>

          {/* Header: column labels (4-letter codes only, no rotation needed) */}
          <div style={{ display: "flex", alignItems: "flex-end", gap: 4 }}>
            <div style={{ width: labelW, flex: "0 0 auto" }} />
            <div style={{ display: "grid", gridTemplateColumns: `repeat(${N}, ${cell}px)`, gap: 4 }}>
              {order.map((idx) => {
                const sp = SPECIES[COOC_SPECIES[idx]];
                const active = (hovered.i === idx) || (hovered.j === idx);
                return (
                  <div key={idx} style={{
                    display: "flex", flexDirection: "column", alignItems: "center", gap: 4,
                    paddingBottom: 6,
                  }}>
                    <span style={{
                      width: 8, height: 8, borderRadius: 2,
                      background: sp.color,
                    }} />
                    <span className="mono" style={{
                      fontSize: 11, fontWeight: active ? 700 : 500,
                      color: active ? "var(--fg)" : "var(--fg-2)",
                      letterSpacing: "0.04em",
                    }}>{sp.short}</span>
                  </div>
                );
              })}
            </div>
          </div>

          {/* Body: row labels + matrix */}
          <div style={{ display: "flex", marginTop: 4 }}>
            <div style={{ width: labelW, flex: "0 0 auto", display: "grid", gridTemplateRows: `repeat(${N}, ${cell}px)`, gap: 4 }}>
              {order.map((idx) => {
                const sp = SPECIES[COOC_SPECIES[idx]];
                const active = (hovered.i === idx) || (hovered.j === idx);
                return (
                  <div key={idx} style={{
                    display: "flex", alignItems: "center", justifyContent: "flex-end",
                    gap: 8, paddingRight: 8,
                    fontSize: 13, fontWeight: active ? 600 : 500,
                    color: active ? "var(--fg)" : "var(--fg-2)",
                  }}>
                    <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{sp.common}</span>
                    <SpeciesAvatar sp={COOC_SPECIES[idx]} size={26} />
                  </div>
                );
              })}
            </div>
            <div style={{ display: "grid", gridTemplateColumns: `repeat(${N}, ${cell}px)`, gridTemplateRows: `repeat(${N}, ${cell}px)`, gap: 4 }}>
              {order.flatMap((rIdx, ri) => order.map((cIdx, ci) => {
                const v = COOC[rIdx][cIdx];
                const isDiag = rIdx === cIdx;
                const active = (hovered.i === rIdx && hovered.j === cIdx) || (hovered.j === rIdx && hovered.i === cIdx);
                const rowOrCol = hovered.i === rIdx || hovered.j === rIdx || hovered.i === cIdx || hovered.j === cIdx;
                return (
                  <div
                    key={`${ri}-${ci}`}
                    onMouseEnter={() => setHovered({ i: rIdx, j: cIdx })}
                    style={{
                      borderRadius: 5,
                      background: isDiag
                        ? "repeating-linear-gradient(45deg, var(--surface-2) 0 3px, var(--bg-2) 3px 6px)"
                        : `color-mix(in oklch, var(--moss) ${Math.round(v * 82)}%, var(--surface-2))`,
                      outline: active ? "1.75px solid var(--fg)" : "none",
                      outlineOffset: -1,
                      opacity: rowOrCol || active ? 1 : 0.55,
                      transition: "opacity .12s",
                      display: "flex", alignItems: "center", justifyContent: "center",
                      cursor: "default",
                      position: "relative",
                    }}
                  >
                    {!isDiag && v >= 0.45 && (
                      <span className="mono" style={{ fontSize: 11, color: v > 0.65 ? "var(--bg)" : "var(--moss-ink)", fontWeight: 500 }}>
                        {v.toFixed(2)}
                      </span>
                    )}
                  </div>
                );
              }))}
            </div>
          </div>

          <div className="bnb-meta" style={{ marginTop: "auto", paddingTop: 14, borderTop: "0.5px solid var(--hairline)" }}>
            Click any cell to lock the pair, or skip to the <a href="#" style={{ color: "var(--fg-2)" }}>Acoustic Network →</a> for the full chord view.
          </div>
        </div>

        <CompanionPanel hovered={hovered} />
      </div>
    </Screen>
  );
}

function CompanionPanel({ hovered }) {
  const { COOC, COOC_SPECIES, SPECIES } = window.BNB;
  const ia = COOC_SPECIES[hovered.i];
  const ib = COOC_SPECIES[hovered.j];
  const a = SPECIES[ia], b = SPECIES[ib];
  const rho = COOC[hovered.i][hovered.j];
  const isDiag = hovered.i === hovered.j;

  const aHours = window.BNB.CHORUS.find((c) => c.sp === ia)?.hours || a.trend.slice(0, 24);
  const bHours = window.BNB.CHORUS.find((c) => c.sp === ib)?.hours || b.trend.slice(0, 24);

  if (isDiag) {
    return (
      <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 12 }}>
        <div className="bnb-eyebrow">Self-pair</div>
        <div className="display" style={{ fontSize: 22 }}>{a.common} × itself</div>
        <div className="bnb-meta" style={{ lineHeight: 1.5 }}>The diagonal is always 1.0 by definition. Hover an off-diagonal cell to see how two species correlate.</div>
      </div>
    );
  }

  return (
    <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 14, minHeight: 0 }}>
      <div className="bnb-eyebrow">Pair · drill-in</div>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <SpeciesAvatar sp={ia} size={34} />
        <span className="display" style={{ fontSize: 18 }}>{a.common}</span>
        <span style={{ color: "var(--fg-3)" }}>×</span>
        <SpeciesAvatar sp={ib} size={34} />
        <span className="display" style={{ fontSize: 18 }}>{b.common}</span>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10, borderTop: "0.5px solid var(--hairline)", borderBottom: "0.5px solid var(--hairline)", padding: "12px 0" }}>
        <Stat label="Spearman ρ" value={rho.toFixed(2)} sub="5-min window" size="sm" accent={rho > 0.6 ? "var(--moss-ink)" : "var(--fg)"} />
        <Stat label="Co-detections" value={Math.round(rho * 320).toLocaleString()} sub="last 60 d" size="sm" />
        <Stat label="Median Δt" value={`${(2.6 - rho * 1.5).toFixed(1)} min`} sub="median offset" size="sm" />
      </div>

      <div>
        <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>Hourly activity overlay</div>
        <DualHourChart a={aHours} b={bHours} colorA={a.color} colorB={b.color} />
        <div style={{ display: "flex", gap: 14, marginTop: 6 }}>
          <span className="bnb-meta" style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
            <span style={{ width: 12, height: 2, background: a.color }} /> {a.common}
          </span>
          <span className="bnb-meta" style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
            <span style={{ display: "inline-block", width: 12, height: 2, background: `repeating-linear-gradient(90deg, ${b.color} 0 3px, transparent 3px 5px)` }} /> {b.common}
          </span>
        </div>
      </div>

      <div style={{ background: "var(--surface-2)", borderRadius: 8, padding: 12 }}>
        <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Plain-English read</div>
        <div style={{ fontSize: 13.5, color: "var(--fg)", lineHeight: 1.55 }}>
          When you hear <strong>{a.common}</strong>, there is a <strong style={{ color: "var(--moss-ink)" }}>{Math.round(rho * 100)}%</strong> chance you'll hear <strong>{b.common}</strong> within five minutes. {rho > 0.6 ? "They share habitat and feeding times." : rho > 0.4 ? "They overlap in the soundscape but not closely." : "They mostly avoid each other's airtime."}
        </div>
      </div>

      <div style={{ marginTop: "auto", display: "flex", gap: 6 }}>
        <button className="bnb-btn primary" style={{ flex: 1, justifyContent: "center" }}>Open both species →</button>
        <button className="bnb-btn">Export pair</button>
      </div>
    </div>
  );
}

function DualHourChart({ a, b, colorA, colorB }) {
  const max = Math.max(0.001, ...a, ...b);
  const W = 480, H = 110;
  const pts = (data) => data.map((v, i) => [(i / 23) * W, H - 14 - (v / max) * (H - 22)]);
  const toPath = (ps) => ps.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`).join(" ");
  const toArea = (ps) => `${toPath(ps)} L${W},${H - 14} L0,${H - 14} Z`;
  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} preserveAspectRatio="none">
      {[0, 6, 12, 18].map((h) => (
        <g key={h}>
          <line x1={(h/23)*W} y1={4} x2={(h/23)*W} y2={H-14} stroke="var(--hairline)" />
          <text x={(h/23)*W} y={H-2} className="mono" textAnchor="middle" style={{ fontSize: 9.5, fill: "var(--fg-3)" }}>
            {h === 0 ? "12a" : h === 12 ? "12p" : h < 12 ? `${h}a` : `${h-12}p`}
          </text>
        </g>
      ))}
      <path d={toArea(pts(a))} fill={colorA} fillOpacity={0.20} />
      <path d={toPath(pts(a))} stroke={colorA} fill="none" strokeWidth={1.8} />
      <path d={toArea(pts(b))} fill={colorB} fillOpacity={0.18} />
      <path d={toPath(pts(b))} stroke={colorB} fill="none" strokeWidth={1.8} strokeDasharray="4 3" />
    </svg>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// Acoustic Network — chord diagram (sibling view of the matrix)
// ═══════════════════════════════════════════════════════════════════════════
function AcousticNetwork() {
  const { COOC, COOC_SPECIES, SPECIES } = window.BNB;
  const [hovered, setHovered] = useState_co(null); // pair index [i, j]
  const N = COOC_SPECIES.length;

  // Compute chord arcs.
  const size = 720;
  const cx = size / 2, cy = size / 2;
  const r = size / 2 - 110;
  const rOuter = r + 14;

  // Each species gets an arc proportional to its total connection strength.
  const sums = COOC.map((row, i) => row.reduce((s, v, j) => s + (i === j ? 0 : v), 0));
  const totalSum = sums.reduce((s, v) => s + v, 0);
  let acc = 0;
  const arcs = sums.map((s) => {
    const a0 = (acc / totalSum) * Math.PI * 2 - Math.PI / 2;
    acc += s;
    const a1 = (acc / totalSum) * Math.PI * 2 - Math.PI / 2;
    return { a0, a1, mid: (a0 + a1) / 2, span: a1 - a0 };
  });

  // Build chord ribbons for upper-triangular pairs.
  const ribbons = [];
  for (let i = 0; i < N; i++) {
    for (let j = i + 1; j < N; j++) {
      const v = COOC[i][j];
      if (v < 0.2) continue;
      // Each end of the ribbon is centered on its species' arc, weighted by v
      const arcI = arcs[i], arcJ = arcs[j];
      const widthI = (v / sums[i]) * arcI.span;
      const widthJ = (v / sums[j]) * arcJ.span;
      ribbons.push({ i, j, v, widthI, widthJ, arcI, arcJ });
    }
  }
  // Sort weakest first so strong ribbons render on top.
  ribbons.sort((a, b) => a.v - b.v);

  return (
    <Screen>
      <TopNav active="Analytics" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Behavioral analytics · co-occurrence</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>The acoustic network</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 560 }}>
            Same data as the matrix, drawn as ribbons. Thicker = more co-occurrence. Each species' arc length is its total connectedness in the soundscape.
          </div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <span className="bnb-pill">5-min window</span>
          <span className="bnb-pill">60 days</span>
          <span className="bnb-pill">ρ ≥ 0.20</span>
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1.15fr 1fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", alignItems: "center", justifyContent: "center", minHeight: 0 }}>
          <svg viewBox={`0 0 ${size} ${size}`} width="100%" height="100%" style={{ maxWidth: 720, maxHeight: 720 }}>
            {/* Ribbons */}
            {ribbons.map((rb, idx) => {
              const spI = SPECIES[COOC_SPECIES[rb.i]];
              const spJ = SPECIES[COOC_SPECIES[rb.j]];
              const path = chordPath(cx, cy, r, rb.arcI.mid, rb.widthI, rb.arcJ.mid, rb.widthJ);
              const isHovered = hovered && (hovered[0] === rb.i && hovered[1] === rb.j);
              const dim = hovered && !isHovered;
              return (
                <g key={idx} className="chord-ribbon">
                  <path
                    d={path}
                    fill={`url(#grad-${idx})`}
                    fillOpacity={dim ? 0.10 : 0.45 + rb.v * 0.40}
                    stroke={spI.color}
                    strokeOpacity={dim ? 0.08 : 0.55}
                    strokeWidth={0.7}
                    onMouseEnter={() => setHovered([rb.i, rb.j])}
                    onMouseLeave={() => setHovered(null)}
                    style={{ cursor: "pointer", transition: "fill-opacity .15s" }}
                  />
                  <defs>
                    <linearGradient id={`grad-${idx}`}
                      x1={cx + r * Math.cos(rb.arcI.mid)} y1={cy + r * Math.sin(rb.arcI.mid)}
                      x2={cx + r * Math.cos(rb.arcJ.mid)} y2={cy + r * Math.sin(rb.arcJ.mid)}
                      gradientUnits="userSpaceOnUse"
                    >
                      <stop offset="0%" stopColor={spI.color} stopOpacity="1" />
                      <stop offset="100%" stopColor={spJ.color} stopOpacity="1" />
                    </linearGradient>
                  </defs>
                </g>
              );
            })}

            {/* Species arcs (outer ring) */}
            {arcs.map((arc, i) => {
              const sp = SPECIES[COOC_SPECIES[i]];
              const path = arcPath(cx, cy, r + 3, rOuter, arc.a0 + 0.005, arc.a1 - 0.005);
              const isHovered = hovered && (hovered[0] === i || hovered[1] === i);
              const dim = hovered && !isHovered;
              return (
                <path key={`arc-${i}`}
                  d={path}
                  fill={sp.color}
                  opacity={dim ? 0.18 : isHovered ? 1 : 0.92}
                  style={{ transition: "opacity .15s" }}
                />
              );
            })}

            {/* Labels — ride the arc, so they never collide */}
            {arcs.map((arc, i) => {
              const sp = SPECIES[COOC_SPECIES[i]];
              const labelR = rOuter + 22;
              // Place each label at the arc midpoint, but rotate to be tangent-aligned (readable)
              const deg = (arc.mid * 180 / Math.PI) + 90; // tangent
              // Flip text on the left half so it doesn't read upside-down
              const flip = (arc.mid > Math.PI / 2 && arc.mid < Math.PI * 3 / 2) || (arc.mid < -Math.PI / 2);
              const finalDeg = flip ? deg + 180 : deg;
              const radius = flip ? labelR + 8 : labelR;
              const isHovered = hovered && (hovered[0] === i || hovered[1] === i);
              const dim = hovered && !isHovered;
              return (
                <g key={`label-${i}`} transform={`translate(${cx + radius * Math.cos(arc.mid)}, ${cy + radius * Math.sin(arc.mid)}) rotate(${finalDeg})`}
                   style={{ transition: "opacity .15s" }} opacity={dim ? 0.4 : 1}>
                  <text textAnchor="middle" style={{ fontSize: 12, fontWeight: isHovered ? 600 : 500, fill: "var(--fg)" }}>{sp.common}</text>
                  <text y={12} textAnchor="middle" className="mono" style={{ fontSize: 9.5, fill: "var(--fg-3)" }}>ρ̄ {(sums[i] / (N - 1)).toFixed(2)}</text>
                </g>
              );
            })}
            {/* center label */}
            <text x={cx} y={cy - 6} textAnchor="middle" className="display" style={{ fontSize: 14, fill: "var(--fg-3)" }}>5-minute</text>
            <text x={cx} y={cy + 10} textAnchor="middle" className="display" style={{ fontSize: 14, fill: "var(--fg-3)" }}>co-occurrence</text>
          </svg>
        </div>

        {/* Right rail — top pairs list */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 14 }}>
          <SectionHeader eyebrow="Strongest pairs" title="Top connections" />
          <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
            {ribbons.slice().reverse().slice(0, 8).map((rb, idx) => {
              const spI = SPECIES[COOC_SPECIES[rb.i]];
              const spJ = SPECIES[COOC_SPECIES[rb.j]];
              return (
                <div
                  key={idx}
                  onMouseEnter={() => setHovered([rb.i, rb.j])}
                  onMouseLeave={() => setHovered(null)}
                  style={{
                    display: "grid", gridTemplateColumns: "auto 1fr auto",
                    alignItems: "center", gap: 10, padding: "10px 0",
                    borderTop: idx > 0 ? "0.5px solid var(--hairline)" : "0",
                    cursor: "default",
                  }}
                >
                  <span style={{ display: "inline-flex", gap: 2 }}>
                    <SpeciesAvatar sp={COOC_SPECIES[rb.i]} size={24} />
                    <SpeciesAvatar sp={COOC_SPECIES[rb.j]} size={24} />
                  </span>
                  <div style={{ minWidth: 0 }}>
                    <div style={{ fontSize: 13, fontWeight: 500, lineHeight: 1.3 }}>
                      {spI.common} <span style={{ color: "var(--fg-3)" }}>×</span> {spJ.common}
                    </div>
                    <div className="bnb-meta mono" style={{ marginTop: 2 }}>{spI.short} · {spJ.short}</div>
                  </div>
                  <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 4 }}>
                    <span className="mono tabular" style={{ fontSize: 13, color: "var(--moss-ink)", fontWeight: 500 }}>ρ {rb.v.toFixed(2)}</span>
                    <span style={{ width: 64, height: 4, background: "var(--bg-2)", borderRadius: 2, overflow: "hidden" }}>
                      <span style={{ display: "block", width: `${rb.v * 100}%`, height: "100%", background: "var(--moss)" }} />
                    </span>
                  </div>
                </div>
              );
            })}
          </div>
          <div className="bnb-meta" style={{ paddingTop: 10, borderTop: "0.5px solid var(--hairline)" }}>
            Communities form along habitat lines: feeder regulars cluster, woodpeckers cluster, owls stand apart.
          </div>
        </div>
      </div>
    </Screen>
  );
}

function chordPath(cx, cy, r, midI, widthI, midJ, widthJ) {
  // Each end of the ribbon is an arc on the inner circle, of widthI/J radians.
  const i0 = midI - widthI / 2, i1 = midI + widthI / 2;
  const j0 = midJ - widthJ / 2, j1 = midJ + widthJ / 2;
  const p = (a) => [cx + r * Math.cos(a), cy + r * Math.sin(a)];
  const [x_i0, y_i0] = p(i0);
  const [x_i1, y_i1] = p(i1);
  const [x_j0, y_j0] = p(j0);
  const [x_j1, y_j1] = p(j1);
  // Quadratic curves through center for the cross-over
  return [
    `M${x_i0},${y_i0}`,
    `A${r},${r} 0 0 1 ${x_i1},${y_i1}`,
    `Q${cx},${cy} ${x_j0},${y_j0}`,
    `A${r},${r} 0 0 1 ${x_j1},${y_j1}`,
    `Q${cx},${cy} ${x_i0},${y_i0}`,
    `Z`,
  ].join(" ");
}

function arcPath(cx, cy, rIn, rOut, a0, a1) {
  const large = a1 - a0 > Math.PI ? 1 : 0;
  const xi0 = cx + rIn * Math.cos(a0),  yi0 = cy + rIn * Math.sin(a0);
  const xi1 = cx + rIn * Math.cos(a1),  yi1 = cy + rIn * Math.sin(a1);
  const xo0 = cx + rOut * Math.cos(a0), yo0 = cy + rOut * Math.sin(a0);
  const xo1 = cx + rOut * Math.cos(a1), yo1 = cy + rOut * Math.sin(a1);
  return [
    `M${xi0},${yi0}`,
    `L${xo0},${yo0}`,
    `A${rOut},${rOut} 0 ${large} 1 ${xo1},${yo1}`,
    `L${xi1},${yi1}`,
    `A${rIn},${rIn} 0 ${large} 0 ${xi0},${yi0}`,
    `Z`,
  ].join(" ");
}

Object.assign(window, { CoOccurrence, AcousticNetwork });
