// Quarantine — rare-bird review queue. Two-pane review interface designed for
// reviewers to triage quickly.

const { useState: useState_q } = React;

function Quarantine() {
  const { SPECIES } = window.BNB;
  const [selected, setSelected] = useState_q(0);

  const queue = [
    { sp: 14, time: "May 22 · 02:14", duration: "2.4s", conf: 0.93, snr: 8.2, freq: "210–800 Hz", note: "First-ever detection at this station", priority: "high" },
    { sp: 12, time: "May 18 · 06:25", duration: "1.6s", conf: 0.81, snr: 14.6, freq: "1.8–4.2 kHz", note: "Spring migrant · plausible", priority: "review" },
    { sp: 10, time: "May 17 · 11:42", duration: "1.2s", conf: 0.83, snr: 12.1, freq: "2.0–6.0 kHz", note: "Edge of range", priority: "review" },
  ];

  const sp = SPECIES[queue[selected].sp];
  const item = queue[selected];

  return (
    <Screen>
      <TopNav active="System" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Rare-bird quarantine · review queue</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Three detections need your eyes</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 580 }}>
            Species never heard before at this station are held here until a human approves or rejects. Approved detections roll into the life list. Rejected ones are kept as evidence but excluded from analytics.
          </div>
        </div>
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <span className="bnb-pill mono">3 pending</span>
          <span className="bnb-pill">12 approved this month</span>
          <span className="bnb-pill">1 rejected this month</span>
        </div>
      </div>

      {/* Two-column: queue list + review pane */}
      <div style={{ display: "grid", gridTemplateColumns: "320px 1fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        {/* Queue */}
        <div className="bnb-card" style={{ padding: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ padding: "12px 16px", display: "flex", justifyContent: "space-between", alignItems: "center", borderBottom: "0.5px solid var(--hairline)" }}>
            <div className="bnb-eyebrow">Queue · oldest first</div>
            <span className="bnb-meta mono">3</span>
          </div>
          {queue.map((q, i) => {
            const qsp = SPECIES[q.sp];
            const isSel = i === selected;
            return (
              <button key={i} onClick={() => setSelected(i)} style={{
                background: isSel ? "var(--surface-2)" : "transparent",
                border: 0, borderLeft: isSel ? "2.5px solid var(--moss)" : "2.5px solid transparent",
                borderBottom: "0.5px solid var(--hairline)",
                padding: "16px 16px 16px 14px",
                textAlign: "left", cursor: "pointer",
                display: "flex", flexDirection: "column", gap: 8,
              }}>
                <div style={{ display: "flex", gap: 10, alignItems: "center" }}>
                  <SpeciesAvatar sp={q.sp} size={36} />
                  <div style={{ flex: 1 }}>
                    <div style={{ fontSize: 14, fontWeight: 500 }}>{qsp.common}</div>
                    <div className="bnb-meta mono" style={{ marginTop: 2 }}>{q.time}</div>
                  </div>
                  {q.priority === "high" && (
                    <span className="bnb-pill rare" style={{ fontSize: 9 }}>priority</span>
                  )}
                </div>
                <div className="bnb-meta" style={{ paddingLeft: 46, fontSize: 11.5, lineHeight: 1.45 }}>{q.note}</div>
                <div style={{ paddingLeft: 46, display: "flex", gap: 8, alignItems: "center" }}>
                  <ConfBar value={q.conf} width={64} />
                </div>
              </button>
            );
          })}
        </div>

        {/* Review pane */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 18, overflow: "hidden" }}>
          {/* Header — species identity + decision strip */}
          <div style={{ display: "flex", justifyContent: "space-between", gap: 20, alignItems: "flex-start" }}>
            <div>
              <div className="bnb-eyebrow">Under review</div>
              <h2 className="display" style={{ fontSize: 36, lineHeight: 1.05, marginTop: 4 }}>{sp.common}</h2>
              <div className="bnb-meta" style={{ fontStyle: "italic", fontSize: 13.5 }}>{sp.sci} · {item.time}</div>
            </div>
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap", justifyContent: "flex-end" }}>
              <span className="bnb-pill rare"><span className="bnb-dot" style={{ background: "var(--rare)" }} /> first ever at this station</span>
              <span className="bnb-pill">eBird: ★ Code 4 (rare)</span>
              <span className="bnb-pill">range: edge</span>
            </div>
          </div>

          {/* Two-column evidence area */}
          <div style={{ display: "grid", gridTemplateColumns: "1.4fr 1fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
            <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
              <ReviewCard title="Spectrogram" subtitle={`${item.duration} · ${item.freq}`}>
                <ReviewSpectrogram />
              </ReviewCard>
              <ReviewCard title="Waveform" subtitle={`SNR ${item.snr} dB`}>
                <ReviewWaveform />
              </ReviewCard>
              {/* Audio scrubber */}
              <div style={{ display: "flex", gap: 12, alignItems: "center", background: "var(--surface-2)", padding: 10, borderRadius: 8 }}>
                <button className="bnb-btn primary" style={{ padding: "8px 14px" }}>▶</button>
                <span className="mono" style={{ fontSize: 12, color: "var(--fg-2)" }}>00:00</span>
                <span style={{ flex: 1, height: 3, background: "var(--bg-2)", borderRadius: 2, position: "relative" }}>
                  <span style={{ display: "block", width: "32%", height: "100%", background: "var(--fg)", borderRadius: 2 }} />
                  <span style={{ position: "absolute", left: "32%", top: -3.5, width: 10, height: 10, borderRadius: 999, background: "var(--fg)", transform: "translateX(-50%)" }} />
                </span>
                <span className="mono" style={{ fontSize: 12, color: "var(--fg-2)" }}>{item.duration}</span>
                <button className="bnb-btn ghost" style={{ fontSize: 12 }}>0.5×</button>
                <button className="bnb-btn ghost" style={{ fontSize: 12 }}>loop</button>
              </div>
            </div>

            {/* Right rail — comparison */}
            <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
              <ReviewCard title="Reference recording" subtitle="Macaulay Library · ML 423091" thin>
                <ReferenceSpectrogram color="var(--moss)" />
                <div className="bnb-meta" style={{ marginTop: 6, paddingLeft: 4 }}>Same species · clean studio reference</div>
              </ReviewCard>

              <ReviewCard title="Top alternative ID" subtitle="What else could this be?" thin>
                <div style={{ display: "flex", flexDirection: "column", gap: 6, marginTop: 4 }}>
                  <AltID name="Great Horned Owl" conf={0.34} color="oklch(40% 0.05 60)" />
                  <AltID name="Eastern Screech-Owl" conf={0.22} color="oklch(48% 0.06 60)" />
                  <AltID name="Mourning Dove" conf={0.12} color="oklch(60% 0.04 30)" />
                </div>
              </ReviewCard>

              <ReviewCard title="Context" subtitle="What was happening" thin>
                <div className="bnb-meta" style={{ lineHeight: 1.5 }}>
                  <div>· Tuesday, 02:14 local (2h 38m before sunrise)</div>
                  <div>· Calm, 8°C, no rain</div>
                  <div>· Last detection of any species: 23:48 (Cardinal)</div>
                </div>
              </ReviewCard>
            </div>
          </div>

          {/* Decision strip */}
          <div style={{ display: "flex", gap: 12, paddingTop: 14, borderTop: "0.5px solid var(--hairline)" }}>
            <button className="bnb-btn" style={{ padding: "10px 16px" }}>✕  Reject</button>
            <button className="bnb-btn" style={{ padding: "10px 16px" }}>↻  Re-label as…</button>
            <div style={{ flex: 1 }} />
            <button className="bnb-btn ghost">Save notes…</button>
            <button className="bnb-btn primary" style={{ padding: "10px 18px" }}>✓  Approve · add to life list →</button>
          </div>
        </div>
      </div>
    </Screen>
  );
}

function ReviewCard({ title, subtitle, thin, children }) {
  return (
    <div style={{ background: "var(--surface-2)", borderRadius: 10, padding: thin ? 12 : 14, flex: thin ? "0 0 auto" : 1, display: "flex", flexDirection: "column" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: 8 }}>
        <span className="bnb-eyebrow">{title}</span>
        <span className="bnb-meta mono">{subtitle}</span>
      </div>
      <div style={{ flex: thin ? "0 0 auto" : 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        {children}
      </div>
    </div>
  );
}

function ReviewSpectrogram() {
  // Static stylized spectrogram for an owl call — low-frequency curving tone
  return (
    <svg viewBox="0 0 600 200" width="100%" height="100%" preserveAspectRatio="none" style={{ flex: 1, background: "var(--bg)", borderRadius: 6 }}>
      {/* Background noise */}
      {Array.from({ length: 400 }).map((_, i) => (
        <rect key={i} x={Math.random() * 600} y={Math.random() * 200} width="1" height="1" fill="var(--fg-4)" opacity={Math.random() * 0.25} />
      ))}
      {/* Frequency grid */}
      {[0, 50, 100, 150, 200].map((y) => (
        <line key={y} x1="0" y1={y} x2="600" y2={y} stroke="var(--hairline)" strokeWidth="0.5" />
      ))}
      {/* The owl call — two-syllable hoot, fundamental near 300Hz with overtones */}
      <g fill="var(--moss-ink)">
        {/* First syllable */}
        {Array.from({ length: 24 }).map((_, i) => {
          const x = 100 + i * 4;
          const y0 = 152 - Math.sin(i * 0.2) * 4;
          return (
            <g key={i}>
              <rect x={x} y={y0} width="3" height="12" rx="1" opacity={0.85} />
              <rect x={x} y={y0 - 30} width="3" height="8" rx="1" opacity={0.55} />
              <rect x={x} y={y0 - 60} width="3" height="5" rx="1" opacity={0.35} />
            </g>
          );
        })}
        {/* Pause */}
        {/* Second syllable */}
        {Array.from({ length: 36 }).map((_, i) => {
          const x = 260 + i * 4;
          const env = Math.sin((i / 36) * Math.PI);
          const yShift = env * 8;
          const y0 = 148 - yShift;
          return (
            <g key={i}>
              <rect x={x} y={y0} width="3" height="14" rx="1" opacity={0.85} />
              <rect x={x} y={y0 - 34} width="3" height="9" rx="1" opacity={0.55} />
              <rect x={x} y={y0 - 64} width="3" height="6" rx="1" opacity={0.35} />
            </g>
          );
        })}
      </g>
      {/* Detection bounding box */}
      <rect x="92" y="80" width="320" height="92" fill="none" stroke="var(--moss)" strokeWidth="1.5" strokeDasharray="3 3" rx="4" />
      <text x="98" y="76" fontSize="10" fontFamily="var(--font-mono)" fill="var(--moss-ink)">BADO · 0.93</text>

      {/* Frequency labels */}
      {[{ y: 10, l: "1.2k" }, { y: 60, l: "0.9k" }, { y: 110, l: "0.6k" }, { y: 160, l: "0.3k" }, { y: 195, l: "0" }].map((m) => (
        <text key={m.l} x={4} y={m.y} fontSize="8.5" fontFamily="var(--font-mono)" fill="var(--fg-3)">{m.l}</text>
      ))}
    </svg>
  );
}

function ReviewWaveform() {
  return (
    <svg viewBox="0 0 600 80" width="100%" height="100%" preserveAspectRatio="none" style={{ background: "var(--bg)", borderRadius: 6 }}>
      <line x1="0" y1="40" x2="600" y2="40" stroke="var(--hairline)" />
      {/* Compute deterministic waveform shape */}
      {Array.from({ length: 240 }).map((_, i) => {
        const xt = (i / 240);
        // Two pulses
        const env1 = Math.exp(-Math.pow((i - 50) / 18, 2));
        const env2 = Math.exp(-Math.pow((i - 130) / 24, 2));
        const env = env1 + env2;
        const v = (Math.sin(i * 0.6) + Math.sin(i * 1.3) * 0.4) * env;
        const y = 40 + v * 28;
        const x = xt * 600;
        return <rect key={i} x={x} y={40 - Math.abs(v * 28)} width={1.5} height={Math.abs(v * 56)} fill="var(--moss-ink)" opacity={0.5 + Math.abs(v) * 0.5} />;
      })}
    </svg>
  );
}

function ReferenceSpectrogram({ color }) {
  return (
    <svg viewBox="0 0 280 100" width="100%" height={100} preserveAspectRatio="none" style={{ background: "var(--bg)", borderRadius: 6 }}>
      {Array.from({ length: 200 }).map((_, i) => (
        <rect key={i} x={Math.random() * 280} y={Math.random() * 100} width="1" height="1" fill="var(--fg-4)" opacity={Math.random() * 0.18} />
      ))}
      <g fill={color}>
        {Array.from({ length: 30 }).map((_, i) => {
          const x = 30 + i * 4;
          const env = Math.exp(-Math.pow((i - 15) / 10, 2));
          return <rect key={i} x={x} y={56 - env * 4} width="3" height={20 + env * 6} rx="1" opacity={0.8} />;
        })}
        {Array.from({ length: 40 }).map((_, i) => {
          const x = 160 + i * 3;
          const env = Math.exp(-Math.pow((i - 20) / 14, 2));
          return <rect key={i} x={x} y={54 - env * 4} width="2.5" height={22 + env * 6} rx="1" opacity={0.8} />;
        })}
      </g>
    </svg>
  );
}

function AltID({ name, conf, color }) {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 80px 40px", gap: 8, alignItems: "center" }}>
      <span style={{ fontSize: 12.5 }}>{name}</span>
      <span style={{ height: 4, background: "var(--bg-2)", borderRadius: 2, overflow: "hidden" }}>
        <span style={{ display: "block", width: `${conf * 100}%`, height: "100%", background: color }} />
      </span>
      <span className="mono tabular" style={{ fontSize: 11, color: "var(--fg-3)", textAlign: "right" }}>{conf.toFixed(2)}</span>
    </div>
  );
}

Object.assign(window, { Quarantine });
