// Audio settings — adding & managing microphones (USB / RTSP / PulseAudio).
// The flagship "set it once, forget it" surface — paired with a "researcher mode"
// for multi-stream rigs.

const { useState: useState_au } = React;

function AudioSettings() {
  const [adding, setAdding] = useState_au(false);

  // Mock state — three current sources
  const sources = [
    {
      id: "alsa1",
      kind: "alsa",
      name: "Yard microphone",
      device: "plughw:1,0",
      detail: "USB Audio Device · channel 0",
      sampleRate: "48 kHz",
      status: "active",
      uptime: "14 d 02 h",
      lastDetection: "14 s ago",
      lastSpecies: "Northern Cardinal",
      level: 0.55,
      snr: 14.2,
      detections24h: 912,
      gain: 12,
      icon: "usb",
    },
    {
      id: "rtsp1",
      kind: "rtsp",
      name: "Maple ridge camera",
      device: "rtsp://192.168.1.42:554/audio",
      detail: "IP camera · 200 m east · 16 kHz mono",
      sampleRate: "16 kHz",
      status: "active",
      uptime: "9 d 18 h",
      lastDetection: "2 m 14 s ago",
      lastSpecies: "Wood Thrush",
      level: 0.31,
      snr: 11.6,
      detections24h: 408,
      gain: 6,
      icon: "rtsp",
    },
    {
      id: "rtsp2",
      kind: "rtsp",
      name: "Pond hydrophone",
      device: "rtsp://192.168.1.51:8554/stream0",
      detail: "Reolink TrackMix · pond edge",
      sampleRate: "48 kHz",
      status: "reconnecting",
      uptime: "—",
      lastDetection: "4 h 28 m ago",
      lastSpecies: "Mallard",
      level: 0,
      snr: 0,
      detections24h: 0,
      gain: 0,
      icon: "rtsp",
    },
  ];

  return (
    <Screen>
      <TopNav active="System" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Settings · audio sources</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>What's the Pi listening to?</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 600 }}>
            BirdNet-Behavior can listen to a USB microphone, a network camera with an RTSP audio feed, or both at once. You can have many — each becomes its own labeled stream in detections.
          </div>
        </div>
        <div style={{ display: "flex", gap: 10, alignItems: "center" }}>
          <label style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 12.5, color: "var(--fg-2)" }}>
            <span className="bnb-pill mono">advanced ✓</span>
          </label>
          <button className="bnb-btn">Discard</button>
          <button className="bnb-btn primary">Save</button>
        </div>
      </div>

      {/* Three-column shell — sidebar / main / right rail */}
      <div style={{ display: "grid", gridTemplateColumns: "240px 1fr 300px", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        <AdminSidebar />

        {/* Main — sources list */}
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--pad-3)", minHeight: 0 }}>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: 14 }}>
              <div>
                <div className="bnb-eyebrow">Active sources</div>
                <h3 className="display" style={{ fontSize: 22, lineHeight: 1.15, marginTop: 2 }}>3 microphones · listening 14 d</h3>
              </div>
              <button className="bnb-btn primary" onClick={() => setAdding(true)}>＋  Add a microphone</button>
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
              {sources.map((s) => <SourceRow key={s.id} source={s} />)}
            </div>
          </div>

          {/* The add-RTSP wizard, always visible — this is the primary task on this screen */}
          <AddRtspCard />

          {/* Researcher advanced */}
          <details className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <summary style={{ display: "flex", justifyContent: "space-between", alignItems: "center", cursor: "default", listStyle: "none", outline: "none" }}>
              <div>
                <div className="bnb-eyebrow">Advanced · multi-stream rigs</div>
                <div style={{ fontSize: 13, color: "var(--fg-2)", marginTop: 4 }}>Round-robin scheduling, per-source confidence overrides, RTSP transport, drift correction</div>
              </div>
              <span style={{ color: "var(--fg-3)" }}>▾</span>
            </summary>
            <div style={{ marginTop: 16, display: "grid", gridTemplateColumns: "1fr 1fr", gap: 16 }}>
              <AdvField label="RTSP transport" helper="UDP is lower latency; TCP survives packet loss." value="tcp" options={["tcp", "udp", "auto"]} />
              <AdvField label="Reconnect backoff" helper="Time between retry attempts when a stream drops." value="2s · 4s · 8s · 16s" />
              <AdvField label="Per-stream sampling" helper="Round-robin across all sources every N seconds, or run them in parallel." value="parallel" options={["parallel", "round-robin"]} />
              <AdvField label="Drift correction" helper="Compensate for clock skew between RTSP streams. Researcher-grade." value="enabled" />
            </div>
          </details>
        </div>

        {/* Right rail — diagnostics */}
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--pad-3)", minHeight: 0 }}>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>Combined input</div>
            <div className="display tabular" style={{ fontSize: 36, lineHeight: 1, color: "var(--moss-ink)" }}>1,320</div>
            <div className="bnb-meta mono" style={{ marginTop: 4 }}>detections · last 24 h</div>
            <hr className="bnb-divider" style={{ margin: "12px 0" }} />
            <CombinedLevels sources={sources.filter((s) => s.status === "active")} />
          </div>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>Common pitfalls</div>
            <Tip glyph="!" text="If your RTSP URL needs a password, embed it: rtsp://user:pass@host/path" />
            <Tip glyph="?" text="Some cameras hide audio on a separate channel. Check the camera's audio URL in the manual." />
            <Tip glyph="✓" text="Test the URL on the host first: ffmpeg -i rtsp://… -t 5 test.wav" />
          </div>
          <div style={{ marginTop: "auto", fontSize: 11, color: "var(--fg-3)", textAlign: "right" }}>
            Connection diagnostic last ran 38 s ago.
          </div>
        </div>
      </div>
    </Screen>
  );
}

// ─── Source row — one configured microphone ───────────────────────────────
function SourceRow({ source }) {
  const [expanded, setExpanded] = useState_au(false);
  const isActive = source.status === "active";
  const isReconnecting = source.status === "reconnecting";
  return (
    <div style={{
      background: "var(--surface-2)",
      borderRadius: 12,
      border: "0.5px solid var(--border)",
      overflow: "hidden",
    }}>
      <div style={{
        display: "grid",
        gridTemplateColumns: "44px 1.5fr 130px 130px 130px 90px 110px",
        gap: 14, alignItems: "center",
        padding: "14px 16px",
      }}>
        {/* Source kind icon */}
        <div style={{
          width: 44, height: 44, borderRadius: 10,
          background: source.kind === "alsa" ? "color-mix(in oklch, var(--moss) 14%, var(--surface))" : "color-mix(in oklch, oklch(58% 0.12 240) 14%, var(--surface))",
          color: source.kind === "alsa" ? "var(--moss-ink)" : "oklch(58% 0.12 240)",
          display: "flex", alignItems: "center", justifyContent: "center",
          flex: "0 0 auto",
        }}>
          {source.kind === "alsa" ? <UsbIcon /> : <RtspIcon />}
        </div>

        {/* Name + device */}
        <div style={{ minWidth: 0 }}>
          <div style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 2 }}>
            <span style={{ fontSize: 14.5, fontWeight: 500 }}>{source.name}</span>
            {isActive && <span className="bnb-pill moss" style={{ fontSize: 10 }}><span className="bnb-dot live" /> streaming</span>}
            {isReconnecting && <span className="bnb-pill" style={{ fontSize: 10, color: "var(--dawn-ink)", background: "var(--dawn-soft)", border: 0 }}>● reconnecting</span>}
          </div>
          <div className="mono" style={{ fontSize: 12, color: "var(--fg-3)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{source.device}</div>
          <div className="bnb-meta" style={{ marginTop: 2 }}>{source.detail}</div>
        </div>

        {/* Live level meter */}
        <div>
          <div className="bnb-eyebrow" style={{ fontSize: 9, marginBottom: 4 }}>Level</div>
          <LevelMeter value={source.level} active={isActive} />
          <div className="bnb-meta mono" style={{ marginTop: 4 }}>
            {isActive ? `SNR ${source.snr.toFixed(1)} dB` : "no signal"}
          </div>
        </div>

        {/* Uptime */}
        <div>
          <div className="bnb-eyebrow" style={{ fontSize: 9, marginBottom: 4 }}>Uptime</div>
          <div className="mono tabular" style={{ fontSize: 14, color: isActive ? "var(--fg)" : "var(--fg-3)" }}>{source.uptime}</div>
          <div className="bnb-meta mono" style={{ marginTop: 2 }}>{isActive ? "stable" : "dropped"}</div>
        </div>

        {/* Last seen */}
        <div>
          <div className="bnb-eyebrow" style={{ fontSize: 9, marginBottom: 4 }}>Last detection</div>
          <div className="mono tabular" style={{ fontSize: 13, color: isActive && source.lastDetection.includes("s ago") ? "var(--moss-ink)" : "var(--fg-2)" }}>{source.lastDetection}</div>
          <div className="bnb-meta" style={{ marginTop: 2, fontStyle: "italic", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{source.lastSpecies}</div>
        </div>

        {/* Stats */}
        <div>
          <div className="bnb-eyebrow" style={{ fontSize: 9, marginBottom: 4 }}>24 h</div>
          <div className="display tabular" style={{ fontSize: 20, lineHeight: 1 }}>{source.detections24h.toLocaleString()}</div>
          <div className="bnb-meta mono" style={{ marginTop: 2 }}>detections</div>
        </div>

        {/* Actions */}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 4 }}>
          <button onClick={() => setExpanded(!expanded)} className="bnb-btn ghost" title="Audio settings" style={{ padding: "4px 8px" }}>
            {expanded ? "▾ tune" : "▸ tune"}
          </button>
          <button className="bnb-btn ghost" title="Test connection">⏵</button>
          <button className="bnb-btn ghost" title="Remove">×</button>
        </div>
      </div>

      {/* Expanded audio controls */}
      {expanded && <AudioControls source={source} />}
    </div>
  );
}

// ─── Audio controls — gain, sample rate, channels, normalization ─────────
function AudioControls({ source }) {
  return (
    <div style={{
      borderTop: "0.5px solid var(--border)",
      background: "var(--surface)",
      padding: "16px",
      display: "grid", gridTemplateColumns: "1.2fr 1fr 1fr 1fr", gap: 24,
    }}>
      {/* Gain */}
      <div>
        <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>Input gain</div>
        <GainSlider value={source.gain} active={source.status === "active"} />
        <div className="bnb-meta" style={{ marginTop: 6 }}>Boost soft microphones (use sparingly — noise floor too).</div>
      </div>

      {/* Sample rate */}
      <AudioField label="Sample rate" helper="Higher captures more frequency range" current={source.sampleRate} options={["8 kHz", "16 kHz", "22.05 kHz", "44.1 kHz", "48 kHz"]} />

      {/* Channels */}
      <AudioField label="Channels" helper="Mono is plenty for BirdNET" current="mono" options={["mono", "left", "right", "stereo"]} />

      {/* Encoding */}
      <AudioField label="Bit depth" helper="16-bit is BirdNET-standard" current="16-bit PCM" options={["16-bit PCM", "24-bit PCM"]} />

      {/* Row 2 — pipeline */}
      <div style={{ gridColumn: "1 / -1", display: "grid", gridTemplateColumns: "1fr 1fr 1fr 1fr", gap: 24, borderTop: "0.5px dashed var(--hairline)", paddingTop: 16 }}>
        <PipelineToggle label="High-pass filter"     sub="Cut wind/handling rumble <80 Hz" on={true} />
        <PipelineToggle label="DC offset removal"    sub="Normalize zero-line drift" on={true} />
        <PipelineToggle label="Auto-gain control"    sub="Compensates over-quiet streams" on={false} />
        <PipelineToggle label="RTSP keepalive"       sub={source.kind === "rtsp" ? "TCP · 2 s heartbeat" : "n/a · USB direct"} on={source.kind === "rtsp"} disabled={source.kind !== "rtsp"} />
      </div>

      {/* Row 3 — labels + position */}
      <div style={{ gridColumn: "1 / -1", display: "grid", gridTemplateColumns: "1fr 1fr 200px 200px", gap: 24, borderTop: "0.5px dashed var(--hairline)", paddingTop: 16, alignItems: "center" }}>
        <TextField label="Friendly name" value={source.name} />
        <TextField label="Position note" value={source.detail.split("·")[1]?.trim() || ""} placeholder="e.g. north side, near oak" />
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 4 }}>Schedule</div>
          <span className="mono" style={{ fontSize: 12, color: "var(--fg-2)" }}>24h · always on</span>
        </div>
        <div style={{ display: "flex", gap: 6, justifyContent: "flex-end" }}>
          <button className="bnb-btn ghost">Discard</button>
          <button className="bnb-btn primary">Apply</button>
        </div>
      </div>
    </div>
  );
}

function GainSlider({ value, active }) {
  const pct = ((value + 12) / 36) * 100; // -12 to +24 dB range
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
      <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>−12 dB</span>
      <div style={{ flex: 1, position: "relative", height: 24, display: "flex", alignItems: "center" }}>
        <div style={{ width: "100%", height: 3, background: "var(--bg-2)", borderRadius: 2, position: "relative" }}>
          {/* zero mark */}
          <span style={{ position: "absolute", left: `${(12 / 36) * 100}%`, top: -4, width: 1, height: 11, background: "var(--fg-3)" }} />
          <span style={{ display: "block", width: `${pct}%`, height: "100%", background: active ? "var(--moss)" : "var(--fg-4)", borderRadius: 2 }} />
        </div>
        <div style={{ position: "absolute", left: `${pct}%`, transform: "translateX(-50%)", width: 18, height: 18, borderRadius: "50%", background: "var(--surface)", border: `1.5px solid ${active ? "var(--moss)" : "var(--fg-4)"}`, boxShadow: "var(--shadow-sm)", display: "flex", alignItems: "center", justifyContent: "center" }}>
          <span className="mono tabular" style={{ fontSize: 9, color: "var(--fg)" }}>{value > 0 ? `+${value}` : value}</span>
        </div>
      </div>
      <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>+24 dB</span>
    </div>
  );
}

function AudioField({ label, helper, current, options }) {
  return (
    <div>
      <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>{label}</div>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 4, marginBottom: 6 }}>
        {options.map((o) => (
          <span key={o} style={{
            padding: "5px 9px", borderRadius: 5, fontSize: 11.5,
            background: o === current ? "var(--fg)" : "var(--surface-2)",
            color: o === current ? "var(--bg)" : "var(--fg-2)",
            border: o === current ? 0 : "0.5px solid var(--border)",
            fontFamily: "var(--font-mono)",
            cursor: "default",
          }}>{o}</span>
        ))}
      </div>
      <div className="bnb-meta" style={{ fontSize: 11.5 }}>{helper}</div>
    </div>
  );
}

function PipelineToggle({ label, sub, on, disabled }) {
  return (
    <div style={{ display: "flex", gap: 10, alignItems: "flex-start", opacity: disabled ? 0.4 : 1 }}>
      <span style={{
        width: 32, height: 18, borderRadius: 999, padding: 2,
        background: on ? "var(--moss)" : "var(--bg-2)",
        display: "inline-flex", alignItems: "center", flex: "0 0 auto", marginTop: 2,
      }}>
        <span style={{ width: 14, height: 14, borderRadius: "50%", background: "var(--surface)", boxShadow: "var(--shadow-sm)", transform: on ? "translateX(14px)" : "translateX(0)", transition: "transform .15s" }} />
      </span>
      <div>
        <div style={{ fontSize: 12.5, fontWeight: 500 }}>{label}</div>
        <div className="bnb-meta" style={{ marginTop: 2, fontSize: 11.5, lineHeight: 1.4 }}>{sub}</div>
      </div>
    </div>
  );
}

function TextField({ label, value, placeholder }) {
  return (
    <div>
      <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>{label}</div>
      <input
        defaultValue={value}
        placeholder={placeholder}
        style={{
          width: "100%", boxSizing: "border-box",
          background: "var(--surface-2)", border: "0.5px solid var(--border-2)",
          borderRadius: 6, padding: "7px 10px",
          fontSize: 13, color: "var(--fg)", fontFamily: "var(--font-ui)",
        }}
      />
    </div>
  );
}

// ─── Add RTSP wizard — primary task ───────────────────────────────────────
function AddRtspCard() {
  return (
    <div className="bnb-card" style={{ padding: "var(--pad-3)", borderColor: "color-mix(in oklch, var(--moss) 30%, var(--border))" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", marginBottom: 16 }}>
        <div>
          <div className="bnb-eyebrow" style={{ color: "var(--moss-ink)" }}>Add a microphone · RTSP stream</div>
          <h3 className="display" style={{ fontSize: 22, lineHeight: 1.15, marginTop: 2 }}>From an IP camera or network audio device</h3>
          <p className="bnb-meta" style={{ marginTop: 6, maxWidth: 580 }}>
            Paste the camera's RTSP URL. We'll test the connection, sniff the audio properties, and add it to your sources.
          </p>
        </div>
        <a href="#" className="bnb-meta" style={{ textDecoration: "underline" }}>↗ Where do I find this?</a>
      </div>

      {/* Stepper */}
      <div style={{ display: "grid", gridTemplateColumns: "1.5fr 1fr 1fr", gap: 0, border: "0.5px solid var(--border)", borderRadius: 12, overflow: "hidden", background: "var(--surface-2)" }}>
        {/* Step 1: URL */}
        <Step n={1} title="Stream URL" sub="Required" complete>
          <UrlField />
          <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
            <span className="bnb-pill" style={{ background: "var(--moss-soft)", color: "var(--moss-ink)", border: 0 }}>
              <span className="bnb-dot" style={{ background: "var(--moss-ink)" }} /> reachable · 38 ms
            </span>
            <span className="bnb-pill mono">48 kHz mono · aac</span>
          </div>
        </Step>

        {/* Step 2: Auth */}
        <Step n={2} title="Authentication" sub="If your camera requires it">
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            <Input label="Username" placeholder="admin" value="bnb-listener" />
            <Input label="Password" placeholder="••••••••" value="••••••••••" type="password" />
          </div>
          <div className="bnb-meta" style={{ marginTop: 8 }}>Or embed credentials in the URL: <span className="mono">rtsp://user:pass@host/…</span></div>
        </Step>

        {/* Step 3: Label */}
        <Step n={3} title="Label" sub="How it'll appear in detections">
          <Input label="Friendly name" value="Maple ridge camera" placeholder="e.g. Pond microphone" />
          <Input label="Position" value="200 m east of station" placeholder="Optional · for your notes" />
        </Step>
      </div>

      {/* Live preview */}
      <div style={{ marginTop: 18, padding: 14, background: "var(--surface-2)", borderRadius: 12, border: "0.5px solid var(--border)" }}>
        <div style={{ display: "grid", gridTemplateColumns: "auto 1fr auto", gap: 14, alignItems: "center" }}>
          <div style={{ width: 40, height: 40, borderRadius: 999, background: "color-mix(in oklch, var(--moss) 20%, var(--surface))", color: "var(--moss-ink)", display: "flex", alignItems: "center", justifyContent: "center", flex: "0 0 auto" }}>
            <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor"><path d="M3 8v4l4 2V6L3 8zm6-1.5v7l5-3.5-5-3.5z" /></svg>
          </div>
          <div>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span className="bnb-eyebrow" style={{ color: "var(--moss-ink)" }}>Live preview</span>
              <span className="bnb-pill moss" style={{ fontSize: 10 }}><span className="bnb-dot live" /> hearing 3 birds</span>
            </div>
            <PreviewWaveform />
            <div className="bnb-meta mono" style={{ marginTop: 6 }}>SNR 11.6 dB · 5.4 s buffered · stream-relative time 00:00:38</div>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 6, alignItems: "flex-end" }}>
            <button className="bnb-btn">⏵  Listen for 10s</button>
            <button className="bnb-btn primary">Add to sources →</button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Atoms ────────────────────────────────────────────────────────────────
function Step({ n, title, sub, complete, children }) {
  return (
    <div style={{
      padding: 16,
      borderRight: "0.5px solid var(--border)",
      display: "flex", flexDirection: "column", gap: 10,
      background: complete ? "var(--surface)" : "var(--surface-2)",
    }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={{
          width: 22, height: 22, borderRadius: 999,
          display: "inline-flex", alignItems: "center", justifyContent: "center",
          background: complete ? "var(--moss)" : "var(--bg-2)",
          color: complete ? "var(--bg)" : "var(--fg-3)",
          fontSize: 11, fontWeight: 600,
        }} className="mono">{complete ? "✓" : n}</span>
        <span style={{ fontSize: 13, fontWeight: 500 }}>{title}</span>
      </div>
      <div className="bnb-meta">{sub}</div>
      {children}
    </div>
  );
}

function UrlField() {
  return (
    <div style={{
      display: "flex", alignItems: "stretch",
      background: "var(--surface)", border: "0.5px solid var(--border-2)",
      borderRadius: 8, overflow: "hidden",
    }}>
      <span className="mono" style={{
        padding: "8px 10px", background: "var(--bg-2)", color: "var(--fg-3)", fontSize: 12,
        borderRight: "0.5px solid var(--border)",
      }}>rtsp://</span>
      <input
        defaultValue="192.168.1.42:554/audio"
        style={{
          flex: 1, border: 0, outline: 0,
          padding: "8px 10px", fontFamily: "var(--font-mono)",
          fontSize: 13, color: "var(--fg)", background: "transparent",
        }}
      />
    </div>
  );
}

function Input({ label, value, placeholder, type = "text" }) {
  return (
    <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <span className="bnb-eyebrow" style={{ fontSize: 9.5 }}>{label}</span>
      <input
        type={type}
        defaultValue={value}
        placeholder={placeholder}
        style={{
          background: "var(--surface)",
          border: "0.5px solid var(--border-2)",
          borderRadius: 6,
          padding: "6px 10px",
          fontSize: 13, color: "var(--fg)",
          fontFamily: "var(--font-ui)",
          width: "100%", boxSizing: "border-box",
        }}
      />
    </label>
  );
}

function LevelMeter({ value, active }) {
  const bars = 18;
  const filled = Math.round(value * bars);
  return (
    <div style={{ display: "flex", gap: 2, alignItems: "flex-end", height: 18 }}>
      {Array.from({ length: bars }).map((_, i) => {
        const isOn = active && i < filled;
        const color = i > bars * 0.85
          ? (isOn ? "var(--rare)" : "var(--bg-2)")
          : i > bars * 0.65
            ? (isOn ? "var(--dawn)" : "var(--bg-2)")
            : (isOn ? "var(--moss)" : "var(--bg-2)");
        const h = 4 + (i / bars) * 12;
        return <span key={i} style={{ width: 3, height: h, background: color, borderRadius: 1 }} />;
      })}
    </div>
  );
}

function CombinedLevels({ sources }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      {sources.map((s) => (
        <div key={s.id} style={{ display: "grid", gridTemplateColumns: "auto 1fr auto", gap: 8, alignItems: "center" }}>
          <span style={{ width: 8, height: 8, borderRadius: 2, background: s.kind === "alsa" ? "var(--moss)" : "oklch(58% 0.12 240)" }} />
          <span style={{ fontSize: 12, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{s.name}</span>
          <LevelMeter value={s.level} active />
        </div>
      ))}
    </div>
  );
}

function AdvField({ label, helper, value, options }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <span style={{ fontSize: 13, fontWeight: 500 }}>{label}</span>
      <span className="bnb-meta" style={{ lineHeight: 1.4 }}>{helper}</span>
      {options ? (
        <div style={{ display: "flex", gap: 2, marginTop: 4, background: "var(--bg-2)", padding: 2, borderRadius: 6, width: "fit-content" }}>
          {options.map((o) => (
            <span key={o} className="mono" style={{
              padding: "4px 10px", borderRadius: 4, fontSize: 11.5,
              background: o === value ? "var(--surface)" : "transparent",
              color: o === value ? "var(--fg)" : "var(--fg-3)",
              boxShadow: o === value ? "var(--shadow-sm)" : "none",
            }}>{o}</span>
          ))}
        </div>
      ) : (
        <span className="mono" style={{ fontSize: 12, color: "var(--fg-2)", marginTop: 2 }}>{value}</span>
      )}
    </div>
  );
}

function Tip({ glyph, text }) {
  const color = glyph === "!" ? "var(--dawn-ink)" : glyph === "✓" ? "var(--moss-ink)" : "var(--fg-3)";
  return (
    <div style={{ display: "grid", gridTemplateColumns: "16px 1fr", gap: 8, padding: "8px 0", borderTop: "0.5px solid var(--hairline)", alignItems: "flex-start" }}>
      <span style={{
        width: 16, height: 16, borderRadius: 4, display: "inline-flex",
        alignItems: "center", justifyContent: "center",
        background: glyph === "!" ? "var(--dawn-soft)" : glyph === "✓" ? "var(--moss-soft)" : "var(--bg-2)",
        color, fontSize: 10, fontWeight: 700,
      }}>{glyph}</span>
      <span style={{ fontSize: 12, color: "var(--fg-2)", lineHeight: 1.45 }}>{text}</span>
    </div>
  );
}

function PreviewWaveform() {
  // static waveform — wider than the row, scrollable feel
  const bars = 80;
  return (
    <div style={{ display: "flex", gap: 1.5, marginTop: 8, height: 28, alignItems: "center" }}>
      {Array.from({ length: bars }).map((_, i) => {
        const env = Math.sin((i / bars) * Math.PI * 2.4);
        const noise = (Math.sin(i * 1.7) + Math.sin(i * 0.9)) * 0.3;
        const v = Math.abs(env + noise) * 0.5 + 0.15;
        return <span key={i} style={{ width: 3, height: `${Math.min(28, v * 28)}px`, background: `color-mix(in oklch, var(--moss) ${50 + v * 40}%, var(--fg-4))`, borderRadius: 1 }} />;
      })}
    </div>
  );
}

// ─── Icons ────────────────────────────────────────────────────────────────
function UsbIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 22 22" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M11 18 V6" />
      <path d="M7 14 L11 18 L15 14" />
      <circle cx="11" cy="4" r="1.5" fill="currentColor" />
      <rect x="8" y="9" width="6" height="3" rx="0.5" />
    </svg>
  );
}

function RtspIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 22 22" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="6" width="14" height="9" rx="2" />
      <path d="M17 9 L21 7 V14 L17 12" fill="currentColor" />
      <path d="M19 18 Q14 14 8 18" strokeDasharray="1 2" />
    </svg>
  );
}

// ─── Admin sidebar — reused from system.jsx for consistency, with this section highlighted ─
function AdminSidebar() {
  return (
    <div className="bnb-card" style={{ padding: "var(--pad-2)", display: "flex", flexDirection: "column", gap: 1, height: "fit-content" }}>
      <div className="bnb-eyebrow" style={{ padding: "8px 10px 4px" }}>Sections</div>
      {[
        { name: "Station",        sel: false, ok: true },
        { name: "Audio",          sel: true,  ok: true },
        { name: "Detection",      sel: false, ok: true },
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
        }}>
          <span>{g.name}</span>
          <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
            {!g.ok && <span style={{ width: 6, height: 6, borderRadius: 999, background: "var(--dawn)", flex: "0 0 auto" }} />}
            {g.sel && <span style={{ color: "var(--fg-3)" }}>›</span>}
          </span>
        </a>
      ))}
    </div>
  );
}

Object.assign(window, { AudioSettings });
