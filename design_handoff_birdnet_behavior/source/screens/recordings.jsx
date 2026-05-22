// Recordings — browse and listen to detection clips.
// Side-by-side: a list of clips with spectrogram thumbs, and a player pane.

const { useState: useState_rec } = React;

function Recordings() {
  const { SPECIES } = window.BNB;
  const [selected, setSelected] = useState_rec(0);

  const clips = [
    { sp: 1, time: "06:42:18", dur: 1.4, conf: 0.97, file: "20250522_064218_NOCA.wav", size: "112 KB" },
    { sp: 0, time: "06:38:02", dur: 2.1, conf: 0.94, file: "20250522_063802_BLJA.wav", size: "168 KB" },
    { sp: 3, time: "06:34:48", dur: 1.6, conf: 0.91, file: "20250522_063448_BCCH.wav", size: "128 KB" },
    { sp: 2, time: "06:31:09", dur: 1.3, conf: 0.92, file: "20250522_063109_AMRO.wav", size: "104 KB" },
    { sp: 1, time: "06:27:33", dur: 1.5, conf: 0.96, file: "20250522_062733_NOCA.wav", size: "120 KB" },
    { sp: 5, time: "06:22:11", dur: 1.4, conf: 0.93, file: "20250522_062211_AMGO.wav", size: "112 KB" },
    { sp: 6, time: "06:17:55", dur: 2.0, conf: 0.90, file: "20250522_061755_WBNU.wav", size: "160 KB" },
    { sp: 14, time: "02:14:38", dur: 2.4, conf: 0.93, file: "20250522_021438_BADO.wav", size: "192 KB", rare: true },
  ];

  const item = clips[selected];
  const sp = SPECIES[item.sp];

  return (
    <Screen>
      <TopNav active="Today" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Recordings · today</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Listen to what the Pi heard</h2>
          <div className="bnb-meta" style={{ marginTop: 6 }}>8 clips · 13.5 seconds total · 1.1 MB · auto-purged after 30 days</div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <span className="bnb-pill">Today</span>
          <span className="bnb-pill">All species</span>
          <span className="bnb-pill">≥ 0.80</span>
          <button className="bnb-btn">Download all</button>
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1.4fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        {/* Clips list */}
        <div className="bnb-card" style={{ padding: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ padding: "12px 16px", borderBottom: "0.5px solid var(--hairline)", display: "flex", justifyContent: "space-between" }}>
            <div className="bnb-eyebrow">Clips · newest first</div>
            <span className="bnb-meta mono">{clips.length}</span>
          </div>
          {clips.map((c, i) => {
            const csp = SPECIES[c.sp];
            const isSel = i === selected;
            return (
              <button key={i} onClick={() => setSelected(i)} style={{
                background: isSel ? "var(--surface-2)" : "transparent",
                border: 0,
                borderLeft: isSel ? "2.5px solid var(--moss)" : "2.5px solid transparent",
                borderBottom: "0.5px solid var(--hairline)",
                padding: "12px 16px 12px 14px",
                textAlign: "left", cursor: "pointer",
                display: "grid", gridTemplateColumns: "auto 1fr auto 60px",
                gap: 10, alignItems: "center",
              }}>
                <SpeciesAvatar sp={c.sp} size={32} />
                <div style={{ minWidth: 0 }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <span style={{ fontSize: 13.5, fontWeight: 500 }}>{csp.common}</span>
                    {c.rare && <span className="bnb-pill rare" style={{ fontSize: 9.5 }}>rare</span>}
                  </div>
                  <div className="bnb-meta mono">{c.time} · {c.dur.toFixed(1)}s · {(c.conf).toFixed(2)}</div>
                </div>
                <MiniSpectro seed={i + 1} color={csp.color} />
                <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)", textAlign: "right" }}>{c.size}</span>
              </button>
            );
          })}
        </div>

        {/* Player pane */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 14, overflow: "hidden" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: 16 }}>
            <div>
              <div className="bnb-eyebrow">Now playing</div>
              <h3 className="display" style={{ fontSize: 32, lineHeight: 1.05, marginTop: 4 }}>{sp.common}</h3>
              <div className="bnb-meta mono" style={{ fontStyle: "italic", fontSize: 13.5, marginTop: 2 }}>{sp.sci}</div>
            </div>
            <div style={{ display: "flex", gap: 6 }}>
              <button className="bnb-btn ghost" title="Mark as favorite">☆</button>
              <button className="bnb-btn ghost" title="Lock from auto-delete">🔒</button>
              <button className="bnb-btn ghost" title="Download">↓</button>
              <button className="bnb-btn ghost" title="Re-label">✎</button>
            </div>
          </div>

          {/* Large spectrogram */}
          <div style={{ flex: 1, background: "var(--surface-2)", borderRadius: 10, padding: 12, display: "flex", flexDirection: "column", gap: 10, minHeight: 0 }}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline" }}>
              <span className="bnb-eyebrow">Spectrogram · {item.dur.toFixed(1)} s</span>
              <span className="bnb-meta mono">48 kHz · FFT 1024</span>
            </div>
            <LargeSpectrogram color={sp.color} />
            <div className="bnb-meta mono" style={{ marginTop: 2 }}>Detection band: 2.0 – 6.5 kHz · 0.4 – 1.1 s</div>
          </div>

          {/* Waveform */}
          <div style={{ background: "var(--surface-2)", borderRadius: 10, padding: 12 }}>
            <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>Waveform</div>
            <LargeWaveform color={sp.color} />
          </div>

          {/* Player controls */}
          <div style={{ display: "grid", gridTemplateColumns: "auto 1fr auto auto", gap: 12, alignItems: "center", padding: 12, background: "var(--surface-2)", borderRadius: 10 }}>
            <button className="bnb-btn primary" style={{ padding: "10px 16px", fontSize: 14 }}>▶</button>
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <span style={{ height: 4, background: "var(--bg-2)", borderRadius: 2, position: "relative" }}>
                <span style={{ display: "block", width: "42%", height: "100%", background: "var(--moss)", borderRadius: 2 }} />
                <span style={{ position: "absolute", left: "42%", top: -4, width: 12, height: 12, borderRadius: 999, background: "var(--moss)", transform: "translateX(-50%)" }} />
              </span>
              <div style={{ display: "flex", justifyContent: "space-between" }}>
                <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)" }}>00:00.59</span>
                <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)" }}>00:01.40</span>
              </div>
            </div>
            <div style={{ display: "flex", gap: 4 }}>
              <button className="bnb-btn ghost" style={{ fontSize: 12 }}>0.5×</button>
              <button className="bnb-btn ghost" style={{ fontSize: 12, background: "var(--surface)" }}>1×</button>
              <button className="bnb-btn ghost" style={{ fontSize: 12 }}>2×</button>
            </div>
            <button className="bnb-btn ghost" style={{ fontSize: 12 }}>↻ loop</button>
          </div>

          {/* File detail */}
          <div className="bnb-meta mono" style={{ display: "flex", justifyContent: "space-between", paddingTop: 6, borderTop: "0.5px solid var(--hairline)", color: "var(--fg-3)" }}>
            <span>/data/recordings/2025-05-22/{item.file}</span>
            <span>{item.size} · WAV · 48 kHz · mono</span>
          </div>
        </div>
      </div>
    </Screen>
  );
}

function MiniSpectro({ seed = 1, color }) {
  let s = (seed * 17 + 31) % 100;
  const r = () => { s = (s * 9301 + 49297) % 233280; return s / 233280; };
  return (
    <svg width="80" height="30" viewBox="0 0 80 30">
      {Array.from({ length: 40 }).map((_, i) => {
        const env = Math.sin((i / 40) * Math.PI);
        const v = env * (0.4 + r() * 0.55);
        return <rect key={i} x={i * 2} y={15 - v * 12} width={1.5} height={Math.max(2, v * 24)} fill={color} fillOpacity={0.55 + v * 0.4} rx="0.5" />;
      })}
    </svg>
  );
}

function LargeSpectrogram({ color }) {
  return (
    <svg viewBox="0 0 600 220" width="100%" height="100%" preserveAspectRatio="none" style={{ flex: 1, background: "var(--bg)", borderRadius: 6 }}>
      {Array.from({ length: 600 }).map((_, i) => (
        <rect key={i} x={Math.random() * 600} y={Math.random() * 220} width="1" height="1" fill="var(--fg-4)" opacity={Math.random() * 0.2} />
      ))}
      {/* Three frequency bands of activity */}
      <g>
        {Array.from({ length: 90 }).map((_, i) => {
          const x = 80 + i * 4.5;
          const env = Math.exp(-Math.pow((i - 45) / 28, 2));
          return (
            <g key={i}>
              <rect x={x} y={120 - env * 6} width="3" height={26 + env * 8} fill={color} opacity={0.7 + env * 0.25} rx="0.5" />
              <rect x={x} y={84 - env * 4} width="3" height={14 + env * 6} fill={color} opacity={0.5 + env * 0.3} rx="0.5" />
              <rect x={x} y={50 - env * 4} width="3" height={9 + env * 4} fill={color} opacity={0.3 + env * 0.25} rx="0.5" />
            </g>
          );
        })}
      </g>
      {/* Detection box */}
      <rect x="76" y="40" width="408" height="120" fill="none" stroke={color} strokeWidth="1.5" strokeDasharray="3 3" rx="4" />
      <text x="84" y="34" fontSize="10" fontFamily="var(--font-mono)" fill={color}>detection · 0.97</text>
      {/* freq labels */}
      {[{ y: 10, l: "12k" }, { y: 60, l: "8k" }, { y: 110, l: "5k" }, { y: 160, l: "3k" }, { y: 210, l: "0" }].map((m) => (
        <text key={m.l} x={6} y={m.y} fontSize="9" fontFamily="var(--font-mono)" fill="var(--fg-3)">{m.l}</text>
      ))}
    </svg>
  );
}

function LargeWaveform({ color }) {
  return (
    <svg viewBox="0 0 600 60" width="100%" height={60} preserveAspectRatio="none">
      <line x1="0" y1="30" x2="600" y2="30" stroke="var(--hairline)" />
      {Array.from({ length: 240 }).map((_, i) => {
        const env = Math.exp(-Math.pow((i - 130) / 50, 2)) * 0.9 + 0.1;
        const v = (Math.sin(i * 0.4) + Math.sin(i * 1.1) * 0.5) * env;
        const y = 30 + v * 22;
        return <rect key={i} x={i * 2.5} y={Math.min(30, y) - 1} width="1.8" height={Math.max(1, Math.abs(v * 44))} fill={color} opacity={0.4 + Math.abs(v) * 0.5} />;
      })}
    </svg>
  );
}

Object.assign(window, { Recordings });
