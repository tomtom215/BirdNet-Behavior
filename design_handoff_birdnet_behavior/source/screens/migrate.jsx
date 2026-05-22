// BirdNET-Pi migration — safe, read-only import of legacy SQLite database.

const { useState: useState_mg } = React;

function Migrate() {
  const [step, setStep] = useState_mg(1); // 0=intro, 1=preview, 2=importing, 3=done

  return (
    <Screen>
      <TopNav active="System" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Operations · migrate from BirdNET-Pi</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Bring your history with you</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 620 }}>
            Imports detections from an existing BirdNET-Pi <span className="mono">BirdDB.txt</span> or <span className="mono">birds.db</span>. The source database is opened <strong>read-only</strong> and is never modified — even if the import fails. Duplicates are skipped silently, so re-running is safe.
          </div>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <span className="bnb-pill"><span className="bnb-dot" style={{ background: "var(--moss)" }} /> read-only</span>
          <span className="bnb-pill">transaction-backed</span>
          <span className="bnb-pill">resumable</span>
        </div>
      </div>

      {/* Stepper */}
      <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "12px 0" }}>
        {["Source", "Preview", "Import", "Done"].map((s, i) => {
          const done = step > i;
          const current = step === i;
          return (
            <React.Fragment key={s}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{
                  width: 22, height: 22, borderRadius: 999,
                  background: done ? "var(--moss)" : current ? "var(--fg)" : "var(--bg-2)",
                  color: done || current ? "var(--bg)" : "var(--fg-3)",
                  display: "inline-flex", alignItems: "center", justifyContent: "center",
                  fontSize: 11, fontWeight: 600, fontFamily: "var(--font-mono)",
                }}>{done ? "✓" : i + 1}</span>
                <span style={{ fontSize: 13, fontWeight: current ? 600 : 500, color: done || current ? "var(--fg)" : "var(--fg-3)" }}>{s}</span>
              </div>
              {i < 3 && <span style={{ flex: 1, height: 1, background: done ? "var(--moss)" : "var(--hairline)", maxWidth: 80 }} />}
            </React.Fragment>
          );
        })}
        <span style={{ flex: 1 }} />
        <button className="bnb-btn ghost" onClick={() => setStep(Math.max(0, step - 1))}>← Back</button>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1.5fr 1fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        {/* Left: main pane */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 14, minHeight: 0 }}>
          <SectionHeader eyebrow="Source database" title="BirdDB.txt or birds.db" />

          {/* File picker */}
          <div style={{ display: "grid", gridTemplateColumns: "1fr auto", gap: 10, alignItems: "stretch" }}>
            <div style={{
              padding: "12px 14px",
              background: "var(--surface-2)", border: "1px dashed var(--border-2)",
              borderRadius: 10, display: "flex", flexDirection: "column", gap: 4,
            }}>
              <div className="bnb-eyebrow">Detected file</div>
              <div className="mono" style={{ fontSize: 13, color: "var(--fg)" }}>~/BirdNET-Pi/BirdDB.txt</div>
              <div className="bnb-meta">14,420 rows · 4.2 MB · last modified 2 h ago</div>
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <button className="bnb-btn primary">↑  Upload file</button>
              <button className="bnb-btn">Browse path…</button>
            </div>
          </div>

          {/* Safety bullet list */}
          <div style={{ padding: 14, background: "var(--moss-soft)", borderRadius: 10, display: "flex", flexDirection: "column", gap: 8 }}>
            <div className="bnb-eyebrow" style={{ color: "var(--moss-ink)" }}>What happens to your old data</div>
            <SafetyBullet glyph="✓" text="Source file is opened with O_RDONLY. We never write to it." />
            <SafetyBullet glyph="✓" text="Import runs inside a single SQLite transaction. If anything fails, nothing changes in birdnet-behavior." />
            <SafetyBullet glyph="✓" text="Duplicate rows (same timestamp + species) are skipped — re-import is safe." />
            <SafetyBullet glyph="!" text="We recommend stopping BirdNET-Pi first to ensure a stable snapshot." tone="warn" />
          </div>

          {/* Schema validation */}
          <div>
            <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>Schema validation</div>
            <SchemaRow name="Date" mapped="detection.timestamp" status="ok" />
            <SchemaRow name="Time" mapped="detection.timestamp" status="ok" />
            <SchemaRow name="Sci_Name" mapped="detection.scientific_name" status="ok" />
            <SchemaRow name="Com_Name" mapped="detection.common_name" status="ok" />
            <SchemaRow name="Confidence" mapped="detection.confidence" status="ok" />
            <SchemaRow name="Lat" mapped="detection.lat" status="ok" />
            <SchemaRow name="Lon" mapped="detection.lon" status="ok" />
            <SchemaRow name="Cutoff" mapped="—" status="skip" note="Not used in birdnet-behavior" />
            <SchemaRow name="Week" mapped="—" status="skip" note="Computed from timestamp" />
            <SchemaRow name="Sens" mapped="—" status="skip" note="Per-detection sensitivity not retained" />
          </div>

          {/* Preview of top species */}
          <div>
            <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>Preview · top 5 species in source</div>
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              {[
                { name: "Northern Cardinal", count: 3214 },
                { name: "Blue Jay", count: 2890 },
                { name: "American Robin", count: 1842 },
                { name: "Black-capped Chickadee", count: 1604 },
                { name: "Mourning Dove", count: 1287 },
              ].map((sp, i) => (
                <div key={i} style={{ display: "grid", gridTemplateColumns: "1fr 80px 80px", gap: 12, alignItems: "center", padding: "6px 0", borderTop: i > 0 ? "0.5px solid var(--hairline)" : "0" }}>
                  <span style={{ fontSize: 13 }}>{sp.name}</span>
                  <span style={{ height: 6, background: "var(--bg-2)", borderRadius: 2, overflow: "hidden" }}>
                    <span style={{ display: "block", width: `${(sp.count / 3214) * 100}%`, height: "100%", background: "var(--moss)" }} />
                  </span>
                  <span className="mono tabular" style={{ fontSize: 12, color: "var(--fg-2)", textAlign: "right" }}>{sp.count.toLocaleString()}</span>
                </div>
              ))}
            </div>
          </div>

          {/* Action footer */}
          <div style={{ marginTop: "auto", display: "flex", justifyContent: "space-between", paddingTop: 16, borderTop: "0.5px solid var(--hairline)" }}>
            <span className="bnb-meta">Estimated import time: <span className="mono">~38 seconds</span></span>
            <button className="bnb-btn primary" onClick={() => setStep(2)} style={{ padding: "10px 18px" }}>Import 14,420 detections →</button>
          </div>
        </div>

        {/* Right rail */}
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--pad-3)", minHeight: 0 }}>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <SectionHeader eyebrow="Data quality" title="Source health" />
            <div style={{ marginTop: 12 }}>
              <QualityRow label="Date range"      value="2022-06-04 → 2024-11-30" />
              <QualityRow label="Total rows"      value="14,420" />
              <QualityRow label="Distinct species" value="87" />
              <QualityRow label="Below confidence threshold (0.80)" value="412" warn />
              <QualityRow label="Malformed timestamps" value="0" good />
              <QualityRow label="Duplicate keys"  value="38 (will skip)" />
              <QualityRow label="Schema version"  value="BirdNET-Pi v0.10" />
            </div>
          </div>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>How it'll merge</div>
            <div className="bnb-meta" style={{ lineHeight: 1.55 }}>
              Imported detections appear alongside live ones in every screen. Your life list extends backward, the year tape will show 2022–24 weeks too, and trends/comparisons will use the full window once available.
            </div>
          </div>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>Need help?</div>
            <a href="#" className="bnb-meta" style={{ display: "block", padding: "4px 0", textDecoration: "underline" }}>↗ Migration guide</a>
            <a href="#" className="bnb-meta" style={{ display: "block", padding: "4px 0", textDecoration: "underline" }}>↗ Schema mapping reference</a>
            <a href="#" className="bnb-meta" style={{ display: "block", padding: "4px 0", textDecoration: "underline" }}>↗ Common errors & how to fix them</a>
          </div>
        </div>
      </div>
    </Screen>
  );
}

function SafetyBullet({ glyph, text, tone }) {
  const c = tone === "warn" ? "var(--dawn-ink)" : "var(--moss-ink)";
  const bg = tone === "warn" ? "var(--dawn-soft)" : "color-mix(in oklch, var(--moss) 14%, var(--surface))";
  return (
    <div style={{ display: "grid", gridTemplateColumns: "16px 1fr", gap: 10, alignItems: "flex-start" }}>
      <span style={{ width: 16, height: 16, borderRadius: 4, background: bg, color: c, display: "inline-flex", alignItems: "center", justifyContent: "center", fontSize: 10, fontWeight: 700, flex: "0 0 auto", marginTop: 1 }}>{glyph}</span>
      <span style={{ fontSize: 12.5, color: "var(--fg)", lineHeight: 1.5 }}>{text}</span>
    </div>
  );
}

function SchemaRow({ name, mapped, status, note }) {
  const colors = {
    ok:   { glyph: "✓", bg: "var(--moss-soft)", fg: "var(--moss-ink)" },
    warn: { glyph: "!", bg: "var(--dawn-soft)", fg: "var(--dawn-ink)" },
    skip: { glyph: "−", bg: "var(--bg-2)",       fg: "var(--fg-3)"     },
  }[status];
  return (
    <div style={{ display: "grid", gridTemplateColumns: "16px 100px 1fr auto", gap: 10, alignItems: "center", padding: "6px 0", borderTop: "0.5px solid var(--hairline)" }}>
      <span style={{ width: 16, height: 16, borderRadius: 4, background: colors.bg, color: colors.fg, display: "inline-flex", alignItems: "center", justifyContent: "center", fontSize: 10, fontWeight: 700 }}>{colors.glyph}</span>
      <span className="mono" style={{ fontSize: 11.5, color: "var(--fg-2)" }}>{name}</span>
      <span className="mono" style={{ fontSize: 11.5, color: "var(--fg-3)" }}>→ {mapped}</span>
      <span className="bnb-meta">{note || (status === "ok" ? "ready" : status === "warn" ? "review" : "skip")}</span>
    </div>
  );
}

function QualityRow({ label, value, good, warn }) {
  const c = good ? "var(--moss-ink)" : warn ? "var(--dawn-ink)" : "var(--fg)";
  return (
    <div style={{ display: "flex", justifyContent: "space-between", padding: "8px 0", borderTop: "0.5px solid var(--hairline)" }}>
      <span style={{ fontSize: 12.5, color: "var(--fg-2)" }}>{label}</span>
      <span className="mono" style={{ fontSize: 12.5, color: c, fontWeight: 500 }}>{value}</span>
    </div>
  );
}

Object.assign(window, { Migrate });
