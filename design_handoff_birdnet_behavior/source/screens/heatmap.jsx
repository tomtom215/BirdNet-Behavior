// Activity surface — answers "WHEN is the yard alive" and "WHO is making it loud".
// Streamgraph (species composition over today) + multi-hue mosaic (week × hour).

const { useMemo: useMemo_hm, useState: useState_hm } = React;

function HeatmapScreen() {
  const { SPECIES, HEATMAP, DAY_LABELS, CHORUS } = window.BNB;
  const [hover, setHover] = useState_hm(null); // {d, h}

  // ── Species composition over today, half-hour resolution ──
  const HALF_HOURS = 48;
  const composition = useMemo_hm(() => {
    // Pick top 7 species from SPECIES; derive per-half-hour share from CHORUS or trend.
    const top = SPECIES
      .map((s, i) => ({ idx: i, s }))
      .filter((x) => CHORUS.find((c) => c.sp === x.idx) || x.s.count > 20)
      .slice(0, 7);

    // For each species, an array of 48 half-hour intensities.
    const arrays = top.map(({ idx, s }) => {
      const chorus = CHORUS.find((c) => c.sp === idx);
      const hours = chorus ? chorus.hours : Array.from({ length: 24 }, (_, h) => Math.sin((h - 6) * Math.PI / 18) ** 2);
      const half = new Array(HALF_HOURS).fill(0);
      for (let i = 0; i < HALF_HOURS; i++) {
        const h = i / 2;
        const fl = Math.floor(h);
        const t = h - fl;
        const v = (hours[fl % 24] * (1 - t) + hours[(fl + 1) % 24] * t);
        half[i] = v * (s.count / 50);
      }
      return { idx, s, data: half };
    });

    return arrays;
  }, []);

  // ── Per-cell species composition in the mosaic ──
  // For each (day, hour) cell: pick the top 3 contributing species and shares.
  const mosaicCells = useMemo_hm(() => {
    const cells = [];
    for (let d = 0; d < 7; d++) {
      const row = [];
      for (let h = 0; h < 24; h++) {
        const intensity = HEATMAP[d][h]; // 0..5
        // weight species by their chorus at this hour + day-jitter
        let seed = (d * 31 + h * 17 + 9) % 233280;
        const rand = () => { seed = (seed * 9301 + 49297) % 233280; return seed / 233280; };
        const contribs = composition.map((c) => ({
          idx: c.idx, s: c.s,
          w: c.data[h * 2] * (0.6 + rand() * 0.8),
        })).sort((a, b) => b.w - a.w);
        const top = contribs.slice(0, 3);
        const total = top.reduce((sum, x) => sum + x.w, 0) || 1;
        row.push({
          intensity,
          species: top.map((x) => ({ idx: x.idx, s: x.s, share: x.w / total })),
        });
      }
      cells.push(row);
    }
    return cells;
  }, [composition]);

  const hourTotals = useMemo_hm(() => {
    const t = new Array(24).fill(0);
    for (let d = 0; d < 7; d++) for (let h = 0; h < 24; h++) t[h] += HEATMAP[d][h];
    return t;
  }, []);
  const dayTotals = useMemo_hm(() => HEATMAP.map((row) => row.reduce((s, v) => s + v, 0)), []);
  const maxHour = Math.max(...hourTotals);
  const maxDay = Math.max(...dayTotals);

  return (
    <Screen>
      <TopNav active="Heatmap" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Behavioral analytics · activity surface</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>When the yard is alive — and <em style={{ fontStyle: "italic", color: "var(--moss-ink)" }}>who</em> is doing the singing</h2>
          <div className="bnb-meta" style={{ marginTop: 6 }}>Streamgraph: today by half-hour. Mosaic: last 7 days. 14,820 detections, 8 contributors shown.</div>
        </div>
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <span className="bnb-pill">Top 8 species</span>
          <span className="bnb-pill">Local time</span>
          <span className="bnb-pill">Composition ▾</span>
          <button className="bnb-btn">Export CSV</button>
        </div>
      </div>

      {/* Streamgraph row */}
      <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column" }}>
        <SectionHeader
          eyebrow="Today · half-hour composition"
          title="Species ribbon"
          action={<div style={{ display: "flex", gap: 14, flexWrap: "wrap", justifyContent: "flex-end" }}>
            {composition.map((c, i) => (
              <span key={i} style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 11.5, color: "var(--fg-2)" }}>
                <span style={{ width: 10, height: 10, borderRadius: 2, background: c.s.color }} />
                {c.s.common}
              </span>
            ))}
          </div>}
        />
        <Streamgraph composition={composition} half={HALF_HOURS} />
      </div>

      {/* Main mosaic + side rail */}
      <div style={{ display: "grid", gridTemplateColumns: "1fr 280px", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 8, minHeight: 0 }}>
          <SectionHeader
            eyebrow="Last 7 days"
            title="Hour × day-of-week"
            action={<span className="bnb-meta">Hue = dominant species · darkness = activity · right ticks = other species heard</span>}
          />

          {/* hour labels */}
          <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
            <div style={{ width: 36 }} />
            <div style={{ flex: 1, display: "grid", gridTemplateColumns: "repeat(24, 1fr)", gap: 3 }}>
              {Array.from({ length: 24 }).map((_, h) => (
                <span key={h} className="mono" style={{ fontSize: 10, color: "var(--fg-3)", textAlign: "center" }}>
                  {h % 6 === 0 ? (h === 0 ? "12a" : h === 12 ? "12p" : h < 12 ? `${h}a` : `${h - 12}p`) : ""}
                </span>
              ))}
            </div>
          </div>

          {/* the rows */}
          <div style={{ display: "flex", flexDirection: "column", gap: 4, flex: 1 }}>
            {mosaicCells.map((row, di) => (
              <div key={di} style={{ display: "flex", gap: 6, alignItems: "stretch", flex: 1 }}>
                <span className="mono" style={{ fontSize: 11.5, color: "var(--fg-2)", width: 36, display: "flex", alignItems: "center", fontWeight: 500 }}>{DAY_LABELS[di]}</span>
                <div style={{ flex: 1, display: "grid", gridTemplateColumns: "repeat(24, 1fr)", gap: 4 }}>
                  {row.map((cell, hi) => (
                    <MosaicCell
                      key={hi}
                      cell={cell}
                      day={DAY_LABELS[di]}
                      hour={hi}
                      isHovered={hover && hover.d === di && hover.h === hi}
                      onEnter={() => setHover({ d: di, h: hi })}
                      onLeave={() => setHover(null)}
                    />
                  ))}
                </div>
              </div>
            ))}
          </div>

          {/* hour profile under mosaic */}
          <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
            <div style={{ width: 36 }} />
            <div style={{ flex: 1, display: "grid", gridTemplateColumns: "repeat(24, 1fr)", gap: 3, alignItems: "end", height: 48 }}>
              {hourTotals.map((v, h) => (
                <div key={h} style={{ height: `${(v / maxHour) * 100}%`, background: "color-mix(in oklch, var(--dawn) 65%, var(--surface-2))", borderRadius: 2, minHeight: 2 }} title={`${h}:00 — ${v}`} />
              ))}
            </div>
          </div>
        </div>

        {/* Right rail — hover detail */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 14 }}>
          <CellDetail hover={hover} mosaicCells={mosaicCells} />
          <div>
            <SectionHeader eyebrow="By day" title="Weekly totals" />
            <div style={{ display: "flex", flexDirection: "column", gap: 6, marginTop: 10 }}>
              {dayTotals.map((v, i) => (
                <div key={i} style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)", width: 28 }}>{DAY_LABELS[i]}</span>
                  <span style={{ flex: 1, height: 8, background: "var(--bg-2)", borderRadius: 2, overflow: "hidden" }}>
                    <span style={{ display: "block", width: `${(v / maxDay) * 100}%`, height: "100%", background: "color-mix(in oklch, var(--moss) 70%, var(--surface-2))" }} />
                  </span>
                  <span className="mono tabular" style={{ fontSize: 10.5, color: "var(--fg-2)", minWidth: 24, textAlign: "right" }}>{v}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>

      {/* Insight cards */}
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr 1fr", gap: "var(--pad-3)" }}>
        <Insight2 eyebrow="Peak hour" value="6 – 7 a.m." sub="4.8× midday baseline" trend={[1,2,3,5,8,12,10,7,5,4,3,2]} />
        <Insight2 eyebrow="Quietest day" value="Saturday" sub="−18% vs. weekday mean" trend={[5,4,5,4,3,2,4]} />
        <Insight2 eyebrow="Loudest species" value="Cardinal" sub="118 detections · ρ 0.71 with chickadee" accent="oklch(58% 0.18 25)" />
        <Insight2 eyebrow="Anomaly" value="Tue · 02:00" sub="Barred Owl — rare for this site" tone="rare" />
      </div>
    </Screen>
  );
}

// ─── Mosaic cell ──────────────────────────────────────────────────────────
function MosaicCell({ cell, day, hour, isHovered, onEnter, onLeave }) {
  const intensity = cell.intensity;
  const dominant = cell.species[0];
  const isQuiet = intensity === 0 || !dominant;
  const colorIntensity = Math.min(1, intensity / 5);
  const diversity = cell.species.filter((s) => s.share > 0.10).length;

  return (
    <div
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{
        position: "relative",
        background: isQuiet
          ? "var(--surface-2)"
          : `color-mix(in oklch, ${dominant.s.color} ${22 + colorIntensity * 68}%, var(--surface))`,
        borderRadius: 5,
        overflow: "hidden",
        cursor: "default",
        boxShadow: isHovered ? "0 0 0 1.75px var(--fg) inset" : "none",
        minHeight: 30,
        transition: "background .12s",
      }}
      title={`${day} ${hour}:00 — ${intensity}/5${dominant ? ` · mostly ${dominant.s.common}` : ""}`}
    >
      {!isQuiet && diversity > 1 && (
        <div style={{ position: "absolute", right: 4, top: 4, bottom: 4, width: 2, display: "flex", flexDirection: "column", gap: 2, justifyContent: "flex-end" }}>
          {Array.from({ length: diversity - 1 }).map((_, i) => (
            <span key={i} style={{ width: 2, height: 3, borderRadius: 1, background: "var(--bg)", opacity: 0.6 }} />
          ))}
        </div>
      )}
      {intensity >= 5 && (
        <span style={{ position: "absolute", left: 5, top: 4, width: 4, height: 4, borderRadius: 999, background: "var(--bg)", opacity: 0.85 }} />
      )}
    </div>
  );
}

// ─── Cell detail panel ────────────────────────────────────────────────────
function CellDetail({ hover, mosaicCells }) {
  const { DAY_LABELS } = window.BNB;
  if (!hover) {
    return (
      <div>
        <div className="bnb-eyebrow">Hover any cell</div>
        <div className="display" style={{ fontSize: 18, lineHeight: 1.2, marginTop: 6, color: "var(--fg-3)" }}>
          Pick an hour to see who was singing.
        </div>
        <div className="bnb-meta" style={{ marginTop: 8, lineHeight: 1.55 }}>
          The cell's hue tells you the species you'd hear most in that hour. Darker cells = more total activity. Right-edge ticks count co-occurring species.
        </div>
      </div>
    );
  }
  const cell = mosaicCells[hover.d][hover.h];
  const fmt = (h) => h === 0 ? "12 a" : h === 12 ? "12 p" : h < 12 ? `${h} a` : `${h - 12} p`;
  return (
    <div>
      <div className="bnb-eyebrow">{DAY_LABELS[hover.d]} · {fmt(hover.h)}–{fmt((hover.h + 1) % 24)}</div>
      <div className="display tabular" style={{ fontSize: 30, lineHeight: 1, marginTop: 6 }}>{cell.intensity}/5</div>
      <div className="bnb-meta" style={{ marginTop: 4 }}>activity intensity</div>

      <div style={{ marginTop: 14 }}>
        <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Top species this hour</div>
        {cell.species.map((sp, i) => (
          <div key={i} style={{ display: "grid", gridTemplateColumns: "auto 1fr auto", gap: 8, alignItems: "center", padding: "6px 0", borderTop: i > 0 ? "0.5px solid var(--hairline)" : "0" }}>
            <span style={{ width: 8, height: 8, borderRadius: 2, background: sp.s.color }} />
            <span style={{ fontSize: 12.5 }}>{sp.s.common}</span>
            <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)" }}>{Math.round(sp.share * 100)}%</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── Streamgraph ───────────────────────────────────────────────────────────
function Streamgraph({ composition, half = 48 }) {
  const W = 1380, H = 180;
  const padL = 40, padR = 16, padT = 6, padB = 22;
  const innerW = W - padL - padR;
  const innerH = H - padT - padB;

  // Build wiggly-baseline stack: classic streamgraph
  const stacks = composition.map((c) => c.data);
  const stackN = stacks.length;

  // For each x, compute species values + total, then center
  const cols = [];
  for (let x = 0; x < half; x++) {
    const vals = stacks.map((s) => s[x]);
    const sum = vals.reduce((a, b) => a + b, 0);
    cols.push({ vals, sum });
  }
  const globalMax = Math.max(...cols.map((c) => c.sum), 0.001);

  // For each species, build top and bottom curves
  const paths = composition.map((_, idx) => {
    const tops = [];
    const bots = [];
    for (let x = 0; x < half; x++) {
      const col = cols[x];
      let below = 0;
      for (let k = 0; k < idx; k++) below += col.vals[k];
      const here = col.vals[idx];
      const centerOffset = (col.sum) / 2; // center the stack
      const top = below - centerOffset + here;
      const bot = below - centerOffset;
      const px = padL + (x / (half - 1)) * innerW;
      const scale = (innerH / 2) / (globalMax / 2);
      const py_top = padT + innerH / 2 - top * scale;
      const py_bot = padT + innerH / 2 - bot * scale;
      tops.push([px, py_top]);
      bots.push([px, py_bot]);
    }
    return { tops, bots };
  });

  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} preserveAspectRatio="none" style={{ marginTop: 6 }}>
      {/* hour grid */}
      {[0, 6, 12, 18, 24].map((h) => {
        const x = padL + (h / 24) * innerW;
        return <line key={h} x1={x} y1={padT} x2={x} y2={padT + innerH} stroke="var(--hairline)" />;
      })}
      {/* hour labels */}
      {[0, 6, 12, 18, 24].map((h) => {
        const x = padL + (h / 24) * innerW;
        const label = h === 0 ? "12a" : h === 24 ? "12a" : h === 12 ? "noon" : h < 12 ? `${h}a` : `${h-12}p`;
        return <text key={h} x={x} y={H - 6} textAnchor="middle" className="mono" style={{ fontSize: 10.5, fill: "var(--fg-3)" }}>{label}</text>;
      })}
      {/* species bands */}
      {paths.map((p, idx) => {
        const c = composition[idx];
        const top = p.tops.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`).join(" ");
        const bot = p.bots.slice().reverse().map(([x, y]) => `L${x.toFixed(2)},${y.toFixed(2)}`).join(" ");
        return (
          <g key={idx} className="streamgraph-band">
            <path d={`${top} ${bot} Z`} fill={c.s.color} fillOpacity={0.85} />
            <path d={top} stroke={c.s.color} fill="none" strokeWidth={1.0} strokeOpacity={0.85} />
          </g>
        );
      })}
      {/* dawn line */}
      <g>
        <line x1={padL + (5.35 / 24) * innerW} y1={padT - 2} x2={padL + (5.35 / 24) * innerW} y2={padT + innerH + 4} stroke="var(--dawn-ink)" strokeWidth={1} strokeDasharray="2 2" />
        <text x={padL + (5.35 / 24) * innerW + 4} y={padT + 10} className="mono" style={{ fontSize: 10, fill: "var(--dawn-ink)" }}>sunrise</text>
      </g>
      {/* sunset */}
      <g>
        <line x1={padL + (20.13 / 24) * innerW} y1={padT - 2} x2={padL + (20.13 / 24) * innerW} y2={padT + innerH + 4} stroke="var(--dawn-ink)" strokeWidth={1} strokeDasharray="2 2" />
        <text x={padL + (20.13 / 24) * innerW - 4} y={padT + 10} textAnchor="end" className="mono" style={{ fontSize: 10, fill: "var(--dawn-ink)" }}>sunset</text>
      </g>
      {/* now indicator */}
      <g>
        <line x1={padL + (9.4 / 24) * innerW} y1={padT - 6} x2={padL + (9.4 / 24) * innerW} y2={padT + innerH + 4} stroke="var(--fg)" strokeWidth={1.5} />
        <rect x={padL + (9.4 / 24) * innerW - 24} y={padT - 18} width="48" height="14" rx="3" fill="var(--fg)" />
        <text x={padL + (9.4 / 24) * innerW} y={padT - 8} textAnchor="middle" style={{ fontSize: 10, fill: "var(--bg)" }} className="mono">now</text>
      </g>
    </svg>
  );
}

// ─── Insight cards (richer with mini chart) ──────────────────────────────
function Insight2({ eyebrow, value, sub, accent, trend, tone }) {
  const c = tone === "rare" ? "var(--rare)" : accent || "var(--moss-ink)";
  return (
    <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 6 }}>
      <div className="bnb-eyebrow" style={{ color: c }}>{eyebrow}</div>
      <div className="display" style={{ fontSize: 22, lineHeight: 1.1 }}>{value}</div>
      <div className="bnb-meta" style={{ fontSize: 12.5, lineHeight: 1.45, color: "var(--fg-2)" }}>{sub}</div>
      {trend && (
        <div style={{ marginTop: 6 }}>
          <MiniBars data={trend} accent={c} width={140} height={22} />
        </div>
      )}
    </div>
  );
}

Object.assign(window, { HeatmapScreen });
