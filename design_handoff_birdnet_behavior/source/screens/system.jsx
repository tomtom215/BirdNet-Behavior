// System health + Admin settings. Two screens in one file.

function SystemHealth() {
  return (
    <Screen>
      <TopNav active="System" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>System · health</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Everything is fine.</h2>
          <div className="bnb-meta" style={{ marginTop: 6 }}>Raspberry Pi 5 · 8 GB · 41°C · uptime 14 d 02 h</div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <span className="bnb-pill moss"><span className="bnb-dot live" /> Healthy</span>
          <span className="bnb-pill">birdnet-behavior 0.4.2</span>
          <button className="bnb-btn">Run diagnostic</button>
        </div>
      </div>

      {/* Hero gauges row — one big pulse + 4 specific */}
      <div style={{ display: "grid", gridTemplateColumns: "1.4fr 1fr 1fr 1fr 1fr", gap: 0, border: "0.5px solid var(--border)", borderRadius: "var(--r-lg)", overflow: "hidden", background: "var(--surface)" }}>
        <SystemPulse />
        <BigGauge label="CPU"         value={18} max={100} unit="%"  sub="4-core ARM"     tone="ok" />
        <BigGauge label="Memory"      value={42} max={100} unit="%"  sub="3.4 / 8 GB"     tone="ok" />
        <BigGauge label="Temperature" value={41} max={85}  unit="°C" sub="below throttle" tone="ok" />
        <BigGauge label="Disk"        value={62} max={100} unit="%"  sub="148 / 240 GB"   tone="ok" last />
      </div>

      {/* Two-column working area */}
      <div style={{ display: "grid", gridTemplateColumns: "1.4fr 1fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 10 }}>
          <SectionHeader eyebrow="Process · 24h" title="Resource usage" action={<span className="bnb-meta mono">1m bins · stacked</span>} />
          <ResourceChart />
        </div>
        <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
          <SectionHeader eyebrow="Self-check" title="Everything we tested" />
          <div style={{ marginTop: 10 }}>
            {CHECKS.map((c, i) => (
              <div key={i} style={{ display: "grid", gridTemplateColumns: "20px 1fr auto", gap: 10, padding: "8px 0", borderTop: i > 0 ? "0.5px solid var(--hairline)" : "0", alignItems: "center" }}>
                <CheckGlyph status={c.status} />
                <div>
                  <div style={{ fontSize: 13, fontWeight: 500 }}>{c.name}</div>
                  <div className="bnb-meta">{c.msg}</div>
                </div>
                <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)" }}>{c.detail}</span>
              </div>
            ))}
          </div>
          <div className="bnb-meta" style={{ marginTop: 12, paddingTop: 12, borderTop: "0.5px solid var(--hairline)" }}>
            12 passed · 1 warning · 0 errors · ran 38 ms ago. Same checks available as JSON for monitoring scripts.
          </div>
        </div>
      </div>

      {/* Database row */}
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: "var(--pad-3)" }}>
        <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
          <SectionHeader eyebrow="SQLite" title="Detections database" />
          <div style={{ marginTop: 10 }}>
            <Row k="Rows" v="438,219" />
            <Row k="Size" v="92 MB" />
            <Row k="Integrity" v={<span style={{ color: "var(--moss-ink)" }}>ok</span>} />
            <Row k="Last backup" v="2 h ago" />
          </div>
        </div>
        <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
          <SectionHeader eyebrow="DuckDB" title="Behavioral analytics" />
          <div style={{ marginTop: 10 }}>
            <Row k="Sessions" v="6,408" />
            <Row k="Size" v="48 MB" />
            <Row k="Last refresh" v="13 min ago" />
            <Row k="Mode" v="OLAP · feature-gated" />
          </div>
        </div>
        <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
          <SectionHeader eyebrow="Notifications" title="Last 24h" />
          <div style={{ marginTop: 10 }}>
            <Row k="Apprise" v={<span style={{ color: "var(--moss-ink)" }}>14 sent</span>} />
            <Row k="Email" v="3 sent · 0 deferred" />
            <Row k="MQTT" v="connected · 41 publishes" />
            <Row k="BirdWeather" v="up to date" />
          </div>
        </div>
      </div>
    </Screen>
  );
}

function SystemPulse() {
  // 60-min CPU pulse line + "all systems normal" message
  const pts = Array.from({ length: 60 }, (_, i) => 10 + 8 * Math.sin(i * 0.3) + 4 * Math.sin(i * 0.7) + (i * 7 % 11) * 0.3);
  const W = 380, H = 100;
  const max = 35;
  const path = pts.map((v, i) => `${i === 0 ? "M" : "L"}${(i / 59) * W},${H - 12 - (v / max) * (H - 20)}`).join(" ");
  return (
    <div style={{
      padding: "var(--pad-3)",
      borderRight: "0.5px solid var(--border)",
      display: "flex", flexDirection: "column", gap: 10,
      background: "linear-gradient(135deg, var(--surface), var(--surface-2))",
    }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span className="bnb-pill moss" style={{ fontSize: 11 }}><span className="bnb-dot live" /> all systems normal</span>
        <span className="bnb-meta mono">14 d 02 h uptime</span>
      </div>
      <div className="display" style={{ fontSize: 30, lineHeight: 1.05, marginTop: 2 }}>
        Pi 5 · 8 GB · running cool
      </div>
      <div className="bnb-meta" style={{ maxWidth: 380, lineHeight: 1.5 }}>
        Last self-check passed all 12 probes 38 ms ago. CPU has headroom for two more concurrent inference streams.
      </div>
      <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} preserveAspectRatio="none" style={{ marginTop: "auto" }}>
        <path d={`${path} L${W},${H} L0,${H} Z`} fill="var(--moss)" fillOpacity="0.10" />
        <path d={path} stroke="var(--moss)" fill="none" strokeWidth="1.5" />
        <text x={2} y={12} className="mono" style={{ fontSize: 9.5, fill: "var(--fg-3)" }}>cpu · last 60 min</text>
      </svg>
    </div>
  );
}

function BigGauge({ label, value, max, unit, sub, tone, last }) {
  const pct = value / max;
  const r = 44, c = 2 * Math.PI * r;
  const arc = c * 0.75;
  const off = arc * (1 - pct);
  const color = tone === "warn" ? "var(--dawn)" : tone === "err" ? "var(--rare)" : "var(--moss)";
  return (
    <div style={{
      padding: "var(--pad-3)",
      borderRight: last ? "none" : "0.5px solid var(--hairline)",
      display: "flex", flexDirection: "column", alignItems: "center", gap: 4, justifyContent: "space-between",
    }}>
      <div className="bnb-eyebrow" style={{ alignSelf: "flex-start" }}>{label}</div>
      <svg width="120" height="100" viewBox="0 0 120 100" style={{ flex: "0 0 auto" }}>
        <circle cx="60" cy="60" r={r}
          stroke="var(--bg-2)" strokeWidth="8" fill="none"
          strokeDasharray={`${arc} ${c}`} strokeLinecap="round"
          transform="rotate(135 60 60)" />
        <circle cx="60" cy="60" r={r}
          stroke={color} strokeWidth="8" fill="none"
          strokeDasharray={`${arc - off} ${c}`} strokeLinecap="round"
          transform="rotate(135 60 60)" style={{ transition: "stroke-dasharray .3s" }} />
        <text x="60" y="56" textAnchor="middle" dominantBaseline="central" className="display tabular" style={{ fontSize: 30, fill: "var(--fg)" }}>{value}</text>
        <text x="60" y="78" textAnchor="middle" className="mono" style={{ fontSize: 10, fill: "var(--fg-3)" }}>{unit}</text>
      </svg>
      <div className="bnb-meta mono">{sub}</div>
    </div>
  );
}

function Gauge({ label, value, max, unit, sub, tone }) {
  const pct = value / max;
  const r = 36;
  const c = 2 * Math.PI * r;
  const off = c * (1 - pct);
  const color = tone === "warn" ? "var(--dawn)" : tone === "err" ? "var(--rare)" : "var(--moss)";
  return (
    <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", alignItems: "center", gap: 14 }}>
      <svg width="88" height="88" viewBox="0 0 88 88" style={{ flex: "0 0 auto" }}>
        <circle cx="44" cy="44" r={r} stroke="var(--bg-2)" strokeWidth="6" fill="none" />
        <circle cx="44" cy="44" r={r} stroke={color} strokeWidth="6" fill="none"
                strokeDasharray={c} strokeDashoffset={off} strokeLinecap="round"
                transform="rotate(-90 44 44)" />
        <text x="44" y="44" textAnchor="middle" dominantBaseline="central" className="display tabular" style={{ fontSize: 22 }}>{value}</text>
        <text x="44" y="62" textAnchor="middle" className="mono" style={{ fontSize: 9, fill: "var(--fg-3)" }}>{unit}</text>
      </svg>
      <div>
        <div className="bnb-eyebrow">{label}</div>
        <div className="bnb-meta mono" style={{ marginTop: 4 }}>{sub}</div>
      </div>
    </div>
  );
}

function Row({ k, v }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", padding: "6px 0", borderTop: "0.5px solid var(--hairline)", fontSize: 13 }}>
      <span style={{ color: "var(--fg-3)" }}>{k}</span>
      <span style={{ color: "var(--fg)" }} className="mono">{v}</span>
    </div>
  );
}

function CheckGlyph({ status }) {
  if (status === "pass") return <span style={{ width: 16, height: 16, borderRadius: 999, background: "var(--moss-soft)", color: "var(--moss-ink)", display: "inline-flex", alignItems: "center", justifyContent: "center", fontSize: 10, fontWeight: 700 }}>✓</span>;
  if (status === "warn") return <span style={{ width: 16, height: 16, borderRadius: 4, background: "var(--dawn-soft)", color: "var(--dawn-ink)", display: "inline-flex", alignItems: "center", justifyContent: "center", fontSize: 10, fontWeight: 700 }}>!</span>;
  return <span style={{ width: 16, height: 16, borderRadius: 4, background: "var(--rare-soft)", color: "var(--rare)", display: "inline-flex", alignItems: "center", justifyContent: "center", fontSize: 10, fontWeight: 700 }}>×</span>;
}

const CHECKS = [
  { status: "pass", name: "Audio source reachable",   msg: "plughw:1,0 · USB Audio · 48 kHz",          detail: "2 ms" },
  { status: "pass", name: "BirdNET+ model loaded",     msg: "541 MB ONNX · cached in /data/model",      detail: "ok" },
  { status: "pass", name: "Database integrity",        msg: "PRAGMA integrity_check · all tables",      detail: "0.3 s" },
  { status: "pass", name: "Disk space",                msg: "148 GB free · 62% used",                   detail: "ok" },
  { status: "warn", name: "CPU temperature",           msg: "Within range, climbing slowly this week",  detail: "41 → 47°C" },
  { status: "pass", name: "Web server listening",      msg: "0.0.0.0:8502 · accepting connections",     detail: "ok" },
  { status: "pass", name: "Internet reachable",        msg: "ipapi.co, Zenodo, eBird, Wikipedia",       detail: "98 ms" },
  { status: "pass", name: "Time synchronization",      msg: "ntpd · drift 1.2 ms",                      detail: "ok" },
];

function ResourceChart() {
  const W = 760, H = 200;
  // Simulated 24h-of-1-minute data, smoothed
  const pts = (offset, amp, freq) => Array.from({ length: 96 }, (_, i) => {
    return amp * (0.4 + 0.3 * Math.sin(i * freq + offset) + 0.15 * Math.sin(i * freq * 2.3 + offset));
  });
  const cpu = pts(0.4, 22, 0.15).map((v) => Math.max(2, v));
  const mem = pts(1.1, 8, 0.08).map((v) => 38 + v);

  const xStep = W / (cpu.length - 1);
  const toPath = (d, scale) => d.map((v, i) => `${i === 0 ? "M" : "L"}${(i * xStep).toFixed(1)},${(H - 22 - (v / scale) * (H - 32)).toFixed(1)}`).join(" ");
  const cpuPath = toPath(cpu, 100);
  const memPath = toPath(mem, 100);

  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} preserveAspectRatio="none">
      {[25, 50, 75].map((v) => (
        <g key={v}>
          <line x1="0" y1={H - 22 - (v / 100) * (H - 32)} x2={W} y2={H - 22 - (v / 100) * (H - 32)} stroke="var(--hairline)" />
          <text x="2" y={H - 22 - (v / 100) * (H - 32) - 2} className="mono" style={{ fontSize: 9, fill: "var(--fg-3)" }}>{v}%</text>
        </g>
      ))}
      {/* CPU area */}
      <path d={`${cpuPath} L${W},${H-22} L0,${H-22} Z`} fill="var(--moss)" fillOpacity={0.16} />
      <path d={cpuPath} stroke="var(--moss)" strokeWidth="1.4" fill="none" />
      {/* Mem line */}
      <path d={memPath} stroke="var(--dawn)" strokeWidth="1.4" fill="none" strokeDasharray="3 2" />
      {/* time axis */}
      {[0, 24, 48, 72, 95].map((i, idx) => (
        <text key={i} x={i * xStep} y={H - 4} className="mono" textAnchor={idx === 0 ? "start" : idx === 4 ? "end" : "middle"} style={{ fontSize: 9, fill: "var(--fg-3)" }}>{["−24h","−18h","−12h","−6h","now"][idx]}</text>
      ))}
      {/* legend */}
      <g>
        <circle cx="40" cy="14" r="4" fill="var(--moss)" /><text x="50" y="18" style={{ fontSize: 11, fill: "var(--fg-2)" }}>CPU %</text>
        <circle cx="110" cy="14" r="4" fill="var(--dawn)" /><text x="120" y="18" style={{ fontSize: 11, fill: "var(--fg-2)" }}>Memory %</text>
      </g>
    </svg>
  );
}

// ─── Admin settings — progressive disclosure ──────────────────────────────
function AdminSettings() {
  return (
    <Screen>
      <TopNav active="System" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Settings</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Set it once.</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 540 }}>
            Everything below has a sensible default. Toggle <em>Show advanced</em> to see researcher-only knobs.
          </div>
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <label style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 12.5, color: "var(--fg-2)" }}>
            <span className="bnb-pill mono">advanced ✓</span>
          </label>
          <button className="bnb-btn">Discard</button>
          <button className="bnb-btn primary">Save</button>
        </div>
      </div>

      {/* Two-column form */}
      <div style={{ display: "grid", gridTemplateColumns: "240px 1fr 280px", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        <div className="bnb-card" style={{ padding: "var(--pad-2)", display: "flex", flexDirection: "column", gap: 1, height: "fit-content" }}>
          <div className="bnb-eyebrow" style={{ padding: "8px 10px 4px" }}>Sections</div>
          {[
            { name: "Station",        sel: false, ok: true },
            { name: "Audio",          sel: false, ok: true },
            { name: "Detection",      sel: true,  ok: true },
            { name: "Species rules",  sel: false, ok: true },
            { name: "Notifications",  sel: false, ok: true },
            { name: "BirdWeather",    sel: false, ok: true },
            { name: "MQTT · Home Assistant", sel: false, ok: false },
            { name: "Storage",        sel: false, ok: true },
            { name: "Backup",         sel: false, ok: true },
            { name: "Updates",        sel: false, ok: true },
          ].map((g) => (
            <a key={g.name} href="#" style={{
              padding: "9px 10px", borderRadius: 8, textDecoration: "none",
              background: g.sel ? "var(--surface-2)" : "transparent",
              color: g.sel ? "var(--fg)" : "var(--fg-2)",
              fontSize: 13, display: "flex", justifyContent: "space-between", alignItems: "center",
              fontWeight: g.sel ? 500 : 400,
              transition: "background .15s",
            }}>
              <span>{g.name}</span>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                {!g.ok && <span style={{ width: 6, height: 6, borderRadius: 999, background: "var(--dawn)", flex: "0 0 auto" }} />}
                {g.sel && <span style={{ color: "var(--fg-3)" }}>›</span>}
              </span>
            </a>
          ))}
        </div>

        <div className="bnb-card" style={{ padding: "var(--pad-4)", overflow: "hidden", display: "flex", flexDirection: "column", gap: 20 }}>
          <div>
            <div className="bnb-eyebrow" style={{ marginBottom: 4 }}>Detection</div>
            <h3 className="display" style={{ fontSize: 28, lineHeight: 1.1 }}>When does a sound count as a bird?</h3>
            <p className="bnb-meta" style={{ marginTop: 8, maxWidth: 580, fontSize: 13.5, lineHeight: 1.55 }}>
              The BirdNET+ model returns a confidence score from 0 to 1. Detections below the threshold are dropped before they hit the database. If you've never tuned this, the defaults are excellent.
            </p>
          </div>

          <Field
            label="Confidence threshold"
            helper="0.80 is a strong default. Lower to catch more (and more false positives); raise for researcher-grade certainty."
          >
            <Slider value={0.80} min={0.5} max={1.0} step={0.01} />
            <span className="mono tabular" style={{ fontSize: 13, color: "var(--fg)" }}>0.80</span>
          </Field>

          <Field
            label="Sensitivity"
            helper="Multiplier on the model's logits. 1.0 is BirdNET-Pi-compatible. Above 1.0 = more permissive."
            advanced
          >
            <Slider value={1.0} min={0.5} max={1.5} step={0.05} />
            <span className="mono tabular" style={{ fontSize: 13, color: "var(--fg)" }}>1.00</span>
          </Field>

          <Field
            label="Species frequency filter"
            helper="Suppress detections that are biogeographically implausible at your location. Uses eBird regional data."
          >
            <Toggle on={true} />
            <span className="bnb-meta">SF threshold <span className="mono">0.03</span></span>
          </Field>

          <Field
            label="Quality pre-filter (SNR + flatness)"
            helper="Drop low-SNR or rain-corrupted segments before inference. Saves CPU and false positives."
            advanced
          >
            <Toggle on={true} />
            <select style={{
              padding: "5px 10px", borderRadius: 6, background: "var(--surface)",
              border: "0.5px solid var(--border-2)", fontSize: 12.5, color: "var(--fg)",
              fontFamily: "var(--font-mono)",
            }}>
              <option>SNR 12 dB · flatness 0.4</option>
            </select>
          </Field>

          <Field
            label="Rare-bird quarantine"
            helper="Push detections of never-before-logged species into a review queue. You approve or reject before they hit your life list."
          >
            <Toggle on={true} />
            <span className="bnb-meta"><span className="mono">3</span> in queue · last approved 2 d ago</span>
          </Field>

          <div style={{ marginTop: "auto", display: "flex", justifyContent: "space-between", paddingTop: 16, borderTop: "0.5px solid var(--hairline)", color: "var(--fg-3)", fontSize: 12 }}>
            <span>Changes saved to <span className="mono">/data/settings.sqlite</span></span>
            <span>Last edited 4 h ago by <span className="mono">admin@local</span></span>
          </div>
        </div>

        {/* Right rail — live preview */}
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--pad-3)" }}>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>If you change confidence to…</div>
            <ThresholdPreview />
          </div>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>What this looks like in <span className="mono">birdnet.conf</span></div>
            <pre className="mono" style={{ fontSize: 11, color: "var(--fg-2)", background: "var(--bg-2)", padding: 10, borderRadius: 6, margin: 0, overflow: "hidden" }}>
{`THRESHOLD = 0.80
SENSITIVITY = 1.0
SF_THRESH = 0.03
QUALITY_FILTER = on
QUARANTINE = on`}
            </pre>
          </div>
          <div style={{ marginTop: "auto" }}>
            <a href="#" style={{ fontSize: 12, color: "var(--fg-3)", textDecoration: "none" }}>
              ↗ Reset to defaults
            </a>
          </div>
        </div>
      </div>
    </Screen>
  );
}

function Field({ label, helper, advanced, children }) {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "260px 1fr", gap: 24, paddingBottom: 16, borderBottom: "0.5px solid var(--hairline)" }}>
      <div>
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <span style={{ fontSize: 13, fontWeight: 500 }}>{label}</span>
          {advanced && <span className="bnb-pill mono" style={{ fontSize: 9.5, padding: "0px 6px" }}>adv</span>}
        </div>
        <div className="bnb-meta" style={{ marginTop: 4, lineHeight: 1.45 }}>{helper}</div>
      </div>
      <div style={{ display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap" }}>{children}</div>
    </div>
  );
}

function Slider({ value, min, max }) {
  const pct = ((value - min) / (max - min)) * 100;
  return (
    <div style={{ position: "relative", width: 260, height: 16, display: "flex", alignItems: "center" }}>
      <div style={{ width: "100%", height: 3, background: "var(--bg-2)", borderRadius: 2 }}>
        <div style={{ width: `${pct}%`, height: "100%", background: "var(--moss)", borderRadius: 2 }} />
      </div>
      <div style={{ position: "absolute", left: `${pct}%`, transform: "translateX(-50%)", width: 14, height: 14, borderRadius: "50%", background: "var(--surface)", border: "1px solid var(--moss)", boxShadow: "var(--shadow-sm)" }} />
    </div>
  );
}

function Toggle({ on }) {
  return (
    <span style={{
      display: "inline-flex", alignItems: "center",
      width: 32, height: 18, borderRadius: 999, padding: 2,
      background: on ? "var(--moss)" : "var(--bg-2)",
      transition: "background .15s",
    }}>
      <span style={{
        width: 14, height: 14, borderRadius: "50%", background: "var(--surface)",
        boxShadow: "var(--shadow-sm)",
        transform: on ? "translateX(14px)" : "translateX(0)",
        transition: "transform .15s",
      }} />
    </span>
  );
}

Object.assign(window, { SystemHealth, AdminSettings });

// ─── Threshold preview — shows how many detections you'd get at different thresholds
function ThresholdPreview() {
  const data = [
    { thr: "0.60", count: 1820, label: "noisy" },
    { thr: "0.70", count: 1380, label: "" },
    { thr: "0.80", count: 912,  label: "current", current: true },
    { thr: "0.90", count: 488,  label: "" },
    { thr: "0.95", count: 244,  label: "strict" },
  ];
  const max = Math.max(...data.map((d) => d.count));
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {data.map((d, i) => (
        <div key={i} style={{ display: "grid", gridTemplateColumns: "44px 1fr 60px", gap: 8, alignItems: "center" }}>
          <span className="mono tabular" style={{ fontSize: 12, color: d.current ? "var(--fg)" : "var(--fg-3)", fontWeight: d.current ? 600 : 400 }}>{d.thr}</span>
          <span style={{ height: 14, background: "var(--bg-2)", borderRadius: 3, overflow: "hidden", position: "relative" }}>
            <span style={{
              display: "block", width: `${(d.count / max) * 100}%`, height: "100%",
              background: d.current ? "var(--moss)" : "color-mix(in oklch, var(--moss) 30%, var(--surface-2))",
              borderRadius: 3,
            }} />
            {d.label && (
              <span className="mono" style={{
                position: "absolute", right: 6, top: "50%", transform: "translateY(-50%)",
                fontSize: 9, color: d.current ? "var(--bg)" : "var(--fg-3)",
              }}>{d.label}</span>
            )}
          </span>
          <span className="mono tabular" style={{ fontSize: 12, color: "var(--fg-2)", textAlign: "right" }}>{d.count.toLocaleString()}</span>
        </div>
      ))}
      <div className="bnb-meta" style={{ marginTop: 4, lineHeight: 1.45 }}>Estimated detections per day at each setting, based on the last 30 days.</div>
    </div>
  );
}
