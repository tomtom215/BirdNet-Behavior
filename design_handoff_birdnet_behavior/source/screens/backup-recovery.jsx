// Backup & Recovery — system admin: backups, restore, storage, updates, logs.

function BackupRecovery() {
  return (
    <Screen>
      <TopNav active="System" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Operations · backups & recovery</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Backups, updates, the disk floor</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 600 }}>
            Daily automatic snapshots. One-click restore. SD-card friendly storage with retention controls. Everything you need to recover from a failed Pi without losing detections.
          </div>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <span className="bnb-pill moss"><span className="bnb-dot live" /> backups healthy</span>
          <button className="bnb-btn">Run backup now</button>
        </div>
      </div>

      {/* Headline stats */}
      <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 0, border: "0.5px solid var(--border)", borderRadius: 12, overflow: "hidden", background: "var(--surface)" }}>
        <StripStat label="Last backup"     value="2h 14m ago" sub="2025-05-22 04:30 · auto" accent="var(--moss-ink)" />
        <StripStat label="Retained"        value="14"         sub="snapshots · 28 days" />
        <StripStat label="Backup size"     value="142 MB"     sub="compressed · gzip" />
        <StripStat label="Restore tested"  value="6 d ago"    sub="passed · 38 ms"           last />
      </div>

      {/* Manual backup actions — upload + export */}
      <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", marginBottom: 16 }}>
          <SectionHeader eyebrow="Manual" title="Upload a backup · export current state" />
          <span className="bnb-meta">Move a station to new hardware, hand-off to a collaborator, or archive a copy off-Pi.</span>
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--pad-3)" }}>
          {/* Upload */}
          <div style={{
            padding: "var(--pad-3)",
            border: "1.5px dashed var(--border-2)", borderRadius: 12,
            background: "var(--surface-2)",
            display: "flex", flexDirection: "column", gap: 12,
            minHeight: 200,
          }}>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <span style={{
                width: 36, height: 36, borderRadius: 8,
                background: "color-mix(in oklch, var(--moss) 16%, var(--surface))",
                color: "var(--moss-ink)",
                display: "inline-flex", alignItems: "center", justifyContent: "center",
                fontSize: 18,
              }}>↑</span>
              <div>
                <div style={{ fontSize: 14, fontWeight: 600 }}>Upload a backup</div>
                <div className="bnb-meta" style={{ marginTop: 2 }}>Drop a .bnb-backup file here, or browse</div>
              </div>
            </div>
            <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center", padding: "16px 0", borderRadius: 8, background: "var(--surface)", border: "0.5px solid var(--border)" }}>
              <div style={{ textAlign: "center" }}>
                <div className="display" style={{ fontSize: 22, color: "var(--fg-3)" }}>Drop file here</div>
                <div className="bnb-meta mono" style={{ marginTop: 6 }}>.bnb-backup · .tar.gz · .sqlite</div>
                <button className="bnb-btn primary" style={{ marginTop: 14 }}>Browse files…</button>
              </div>
            </div>
            <div className="bnb-meta" style={{ display: "flex", gap: 14, paddingTop: 4 }}>
              <span>✓ Signature verified before write</span>
              <span>✓ Auto-snapshot current state first</span>
            </div>
          </div>

          {/* Export */}
          <div style={{
            padding: "var(--pad-3)",
            border: "0.5px solid var(--border)", borderRadius: 12,
            background: "var(--surface)",
            display: "flex", flexDirection: "column", gap: 12,
          }}>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <span style={{
                width: 36, height: 36, borderRadius: 8,
                background: "color-mix(in oklch, var(--dawn) 16%, var(--surface))",
                color: "var(--dawn-ink)",
                display: "inline-flex", alignItems: "center", justifyContent: "center",
                fontSize: 18,
              }}>↓</span>
              <div>
                <div style={{ fontSize: 14, fontWeight: 600 }}>Export current state</div>
                <div className="bnb-meta" style={{ marginTop: 2 }}>Bundle the live database, settings, and logs into one file</div>
              </div>
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <ExportOption label="Full backup bundle"   detail="SQLite + DuckDB + settings + last 30 d of logs" size="142 MB" primary />
              <ExportOption label="Database only"        detail="SQLite detections — birdnet.db" size="92 MB" />
              <ExportOption label="Detections as CSV"    detail="Spreadsheet-friendly · all rows" size="38 MB" />
              <ExportOption label="Recordings (WAV)"     detail="All audio clips · today only by default" size="1.4 GB" />
              <ExportOption label="Settings as JSON"     detail="Portable config — useful for cloning a station" size="12 KB" />
              <ExportOption label="Operations logs"      detail="Last 30 days · gzipped" size="4.8 MB" />
            </div>
            <div className="bnb-meta" style={{ paddingTop: 4 }}>Exports stream in chunks · safe to interrupt and resume.</div>
          </div>
        </div>
      </div>

      {/* Three-column working area */}
      <div style={{ display: "grid", gridTemplateColumns: "1.6fr 1fr", gap: "var(--pad-3)" }}>
        {/* Backups list */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: 14 }}>
            <SectionHeader eyebrow="Snapshots" title="Available backups" />
            <div style={{ display: "flex", gap: 6 }}>
              <span className="bnb-pill">Daily · 04:30</span>
              <span className="bnb-pill">Keep 28 days</span>
              <button className="bnb-btn ghost" style={{ fontSize: 11.5 }}>⚙</button>
            </div>
          </div>

          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            {[
              { date: "2025-05-22 04:30", size: "142 MB", rows: 142180, age: "today", auto: true },
              { date: "2025-05-21 04:30", size: "141 MB", rows: 141072, age: "yesterday", auto: true },
              { date: "2025-05-20 19:14", size: "140 MB", rows: 140258, age: "before update", auto: false, label: "pre-upgrade · 0.4.1 → 0.4.2" },
              { date: "2025-05-20 04:30", size: "140 MB", rows: 139904, age: "2 days ago", auto: true },
              { date: "2025-05-19 04:30", size: "139 MB", rows: 138741, age: "3 days ago", auto: true },
              { date: "2025-05-15 04:30", size: "136 MB", rows: 134022, age: "1 week ago", auto: true },
              { date: "2025-05-08 04:30", size: "131 MB", rows: 128410, age: "2 weeks ago", auto: true },
              { date: "2025-04-22 04:30", size: "118 MB", rows: 110804, age: "1 month ago", auto: true },
            ].map((b, i) => (
              <div key={i} style={{
                display: "grid",
                gridTemplateColumns: "20px 170px 1fr 90px 90px 160px",
                gap: 12, alignItems: "center",
                padding: "12px 8px",
                borderRadius: 8,
                background: i === 0 ? "color-mix(in oklch, var(--moss) 5%, var(--surface-2))" : "transparent",
                border: i === 0 ? "0.5px solid color-mix(in oklch, var(--moss) 28%, var(--border))" : "0.5px solid transparent",
              }}>
                <span style={{
                  width: 8, height: 8, borderRadius: 999,
                  background: b.auto ? "var(--moss)" : "var(--dawn)",
                }} title={b.auto ? "auto" : "manual"} />
                <span className="mono tabular" style={{ fontSize: 12.5, color: "var(--fg)" }}>{b.date}</span>
                <div>
                  <span className="bnb-meta">{b.age}</span>
                  {b.label && <span className="bnb-pill" style={{ marginLeft: 8, fontSize: 9.5, background: "var(--dawn-soft)", color: "var(--dawn-ink)", border: 0 }}>{b.label}</span>}
                </div>
                <span className="mono" style={{ fontSize: 11.5, color: "var(--fg-2)", textAlign: "right" }}>{b.size}</span>
                <span className="mono" style={{ fontSize: 11.5, color: "var(--fg-3)", textAlign: "right" }}>{b.rows.toLocaleString()} rows</span>
                <div style={{ display: "flex", gap: 4, justifyContent: "flex-end" }}>
                  <button className="bnb-btn ghost" style={{ fontSize: 11 }}>↓ Download</button>
                  <button className="bnb-btn ghost" style={{ fontSize: 11 }}>↻ Restore</button>
                  <button className="bnb-btn ghost" title="Lock from auto-purge">🔒</button>
                </div>
              </div>
            ))}
          </div>
          <div className="bnb-meta" style={{ marginTop: 14, paddingTop: 12, borderTop: "0.5px solid var(--hairline)" }}>
            Pre-upgrade snapshots are kept indefinitely. Daily backups are pruned to the most recent 28 days.
          </div>
        </div>

        {/* Right rail — destinations, restore, update */}
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--pad-3)" }}>
          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <SectionHeader eyebrow="Backup destinations" title="Where they live" />
            <div style={{ marginTop: 12, display: "flex", flexDirection: "column", gap: 8 }}>
              <DestRow icon="💾" name="Local · /data/backups" detail="/data · 142 GB free" status="on" />
              <DestRow icon="☁︎" name="Off-site · S3 / Backblaze B2" detail="bnb-station-001 · weekly" status="on" />
              <DestRow icon="⌧" name="Network · SMB share" detail="//homelab/birdnet" status="off" />
              <DestRow icon="✉" name="Email weekly summary" detail="includes .db + manifest" status="off" />
            </div>
            <button className="bnb-btn" style={{ marginTop: 12, width: "100%", justifyContent: "center" }}>＋ Add destination</button>
          </div>

          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <SectionHeader eyebrow="Restore" title="Recover from snapshot" />
            <div className="bnb-meta" style={{ marginTop: 8, lineHeight: 1.55 }}>
              Restoring stops detection briefly, swaps the SQLite + DuckDB files, and resumes. The current database is moved to <span className="mono">birdnet.db.before-restore</span> first.
            </div>
            <div style={{ display: "flex", gap: 6, marginTop: 12 }}>
              <button className="bnb-btn primary" style={{ flex: 1, justifyContent: "center" }}>↻ Restore selected</button>
              <button className="bnb-btn">Dry-run</button>
            </div>
          </div>

          <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
            <SectionHeader eyebrow="System update" title="Binary upgrade" />
            <div style={{ marginTop: 10, display: "flex", flexDirection: "column", gap: 8 }}>
              <div style={{ display: "flex", justifyContent: "space-between", padding: "4px 0" }}>
                <span style={{ fontSize: 12.5, color: "var(--fg-2)" }}>Current</span>
                <span className="mono" style={{ fontSize: 12.5 }}>v 0.4.2</span>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between", padding: "4px 0" }}>
                <span style={{ fontSize: 12.5, color: "var(--fg-2)" }}>Latest stable</span>
                <span className="mono" style={{ fontSize: 12.5, color: "var(--moss-ink)" }}>v 0.4.3 ✓ available</span>
              </div>
              <div className="bnb-meta" style={{ paddingTop: 4 }}>Adds: parallel RTSP, eBird hotline integration, two bug fixes.</div>
              <button className="bnb-btn primary" style={{ marginTop: 6, width: "100%", justifyContent: "center" }}>Upgrade · creates backup first</button>
            </div>
          </div>
        </div>
      </div>

      {/* Storage & retention */}
      <div className="bnb-card" style={{ padding: "var(--pad-3)" }}>
        <SectionHeader eyebrow="Storage & retention" title="What lives where, and for how long" />
        <div style={{ marginTop: 16, display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 0, border: "0.5px solid var(--border)", borderRadius: 10, overflow: "hidden" }}>
          <StorageTile label="SQLite (detections)" size="92 MB"  cap="6.2 GB free" pct={0.014} note="438,219 rows · retained forever" />
          <StorageTile label="DuckDB (analytics)"  size="48 MB"  cap="6.2 GB free" pct={0.007} note="rebuilt nightly" />
          <StorageTile label="Audio recordings"    size="1.4 GB" cap="6.2 GB free" pct={0.222} note="auto-purge at 95% disk" tone="warn" />
          <StorageTile label="Wikipedia cache"     size="34 MB"  cap="6.2 GB free" pct={0.005} note="76 species cached"     last />
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 16, marginTop: 18 }}>
          <RetField label="Recording retention" value="30 days" hint="Auto-purge oldest at disk threshold" />
          <RetField label="Per-species cap" value="200 clips" hint="Keeps clip library bounded" />
          <RetField label="Disk threshold" value="95%" hint="Purge triggers above this fill level" />
        </div>
      </div>

      {/* Log viewer + danger zone */}
      <div style={{ display: "grid", gridTemplateColumns: "1.4fr 1fr", gap: "var(--pad-3)" }}>
        <div className="bnb-card" style={{ padding: 0, overflow: "hidden" }}>
          <div style={{ padding: "12px 16px", borderBottom: "0.5px solid var(--hairline)", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <SectionHeader eyebrow="Operations log" title="Last 24 hours" />
            <div style={{ display: "flex", gap: 6 }}>
              <span className="bnb-pill">all</span>
              <span className="bnb-pill mono">info</span>
              <span className="bnb-pill mono">warn</span>
              <span className="bnb-pill mono">error</span>
              <button className="bnb-btn ghost" style={{ fontSize: 11 }}>↓ Download</button>
            </div>
          </div>
          <div style={{ background: "var(--bg-2)", padding: "10px 16px", fontFamily: "var(--font-mono)", fontSize: 11.5, lineHeight: 1.7, maxHeight: 240, overflow: "auto" }}>
            <LogLine ts="04:30:01" level="info"  msg="backup started · target=/data/backups · destinations=[local,s3]" />
            <LogLine ts="04:30:38" level="info"  msg="backup complete · 142 MB · 38.2s · 142,180 rows" />
            <LogLine ts="04:30:39" level="info"  msg="pruned old backup: 2025-04-22 (28 days)" />
            <LogLine ts="05:00:00" level="info"  msg="duckdb analytics rebuild · 6.4s · 6,408 sessions" />
            <LogLine ts="05:21:00" level="info"  msg="sunrise · scheduler entering active window" />
            <LogLine ts="11:42:18" level="warn"  msg="rtsp · rtsp://192.168.1.51:8554/stream0 dropped · backing off 2s" />
            <LogLine ts="11:42:20" level="info"  msg="rtsp · reconnect attempt 1/∞ · success" />
            <LogLine ts="14:08:02" level="error" msg="apprise · channel=ifttt-bonus · 502 from maker.ifttt.com · queued for retry" />
            <LogLine ts="14:09:14" level="info"  msg="apprise · channel=ifttt-bonus · delivered on retry" />
            <LogLine ts="16:22:48" level="info"  msg="quarantine · BADO 2025-05-22 02:14 awaiting review" />
          </div>
        </div>

        <div className="bnb-card" style={{ padding: "var(--pad-3)", borderColor: "color-mix(in oklch, var(--rare) 26%, var(--border))" }}>
          <div className="bnb-eyebrow" style={{ color: "var(--rare)" }}>Danger zone</div>
          <h3 className="display" style={{ fontSize: 22, lineHeight: 1.15, marginTop: 4 }}>Reset, factory, uninstall</h3>
          <div className="bnb-meta" style={{ marginTop: 8, lineHeight: 1.55 }}>
            Destructive actions. Each is confirmation-gated and creates a safety snapshot first — recordings and database are preserved unless you explicitly check the box.
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 8, marginTop: 16 }}>
            <DangerRow title="Reset all settings" sub="Keeps detections · resets thresholds, notifications, audio config" />
            <DangerRow title="Wipe recordings only" sub="Frees disk · detections stay" />
            <DangerRow title="Factory reset" sub="Wipes everything · station back to first-run state" />
            <DangerRow title="Uninstall service" sub="Stops birdnet-behavior · leaves your data behind" />
          </div>
        </div>
      </div>
    </Screen>
  );
}

function DestRow({ icon, name, detail, status }) {
  const on = status === "on";
  return (
    <div style={{ display: "grid", gridTemplateColumns: "28px 1fr auto", gap: 10, alignItems: "center", padding: "8px 0", borderTop: "0.5px solid var(--hairline)" }}>
      <span style={{ width: 28, height: 28, borderRadius: 8, background: on ? "var(--moss-soft)" : "var(--bg-2)", color: on ? "var(--moss-ink)" : "var(--fg-3)", display: "inline-flex", alignItems: "center", justifyContent: "center", fontSize: 14 }}>{icon}</span>
      <div style={{ minWidth: 0 }}>
        <div style={{ fontSize: 13, fontWeight: 500 }}>{name}</div>
        <div className="bnb-meta mono" style={{ marginTop: 2, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{detail}</div>
      </div>
      <span style={{
        width: 32, height: 18, borderRadius: 999, padding: 2,
        background: on ? "var(--moss)" : "var(--bg-2)",
        display: "inline-flex", alignItems: "center", flex: "0 0 auto",
      }}>
        <span style={{ width: 14, height: 14, borderRadius: "50%", background: "var(--surface)", boxShadow: "var(--shadow-sm)", transform: on ? "translateX(14px)" : "translateX(0)", transition: "transform .15s" }} />
      </span>
    </div>
  );
}

function StorageTile({ label, size, cap, pct, note, tone, last }) {
  const color = tone === "warn" ? "var(--dawn)" : "var(--moss)";
  return (
    <div style={{ padding: "var(--pad-3)", borderRight: last ? "none" : "0.5px solid var(--hairline)" }}>
      <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>{label}</div>
      <div className="display tabular" style={{ fontSize: 24, lineHeight: 1 }}>{size}</div>
      <div className="bnb-meta mono" style={{ marginTop: 4 }}>{cap}</div>
      <div style={{ marginTop: 10, height: 4, background: "var(--bg-2)", borderRadius: 2, overflow: "hidden" }}>
        <div style={{ width: `${Math.max(2, pct * 100)}%`, height: "100%", background: color }} />
      </div>
      <div className="bnb-meta" style={{ marginTop: 8, lineHeight: 1.45 }}>{note}</div>
    </div>
  );
}

function RetField({ label, value, hint }) {
  return (
    <div>
      <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>{label}</div>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <span className="mono" style={{ fontSize: 14, fontWeight: 600, padding: "6px 12px", background: "var(--surface-2)", borderRadius: 6, border: "0.5px solid var(--border)" }}>{value}</span>
        <button className="bnb-btn ghost" style={{ fontSize: 11 }}>change</button>
      </div>
      <div className="bnb-meta" style={{ marginTop: 6, lineHeight: 1.45 }}>{hint}</div>
    </div>
  );
}

function LogLine({ ts, level, msg }) {
  const colors = {
    info:  "var(--fg-3)",
    warn:  "var(--dawn-ink)",
    error: "var(--rare)",
  };
  return (
    <div style={{ display: "grid", gridTemplateColumns: "70px 50px 1fr", gap: 8, padding: "2px 0", color: "var(--fg-2)" }}>
      <span style={{ color: "var(--fg-4)" }}>{ts}</span>
      <span style={{ color: colors[level], fontWeight: 500, textTransform: "uppercase", fontSize: 10, alignSelf: "center" }}>{level}</span>
      <span>{msg}</span>
    </div>
  );
}

function DangerRow({ title, sub }) {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr auto", gap: 10, alignItems: "center", padding: "10px 12px", border: "0.5px solid var(--border)", borderRadius: 8, background: "var(--surface-2)" }}>
      <div>
        <div style={{ fontSize: 13, fontWeight: 500 }}>{title}</div>
        <div className="bnb-meta" style={{ marginTop: 2 }}>{sub}</div>
      </div>
      <button className="bnb-btn" style={{ fontSize: 11, color: "var(--rare)" }}>continue…</button>
    </div>
  );
}

function StripStat({ label, value, sub, accent, last }) {
  return (
    <div style={{ padding: "var(--pad-3)", borderRight: last ? "none" : "0.5px solid var(--hairline)" }}>
      <div className="bnb-eyebrow">{label}</div>
      <div className="display tabular" style={{ fontSize: 30, lineHeight: 1, marginTop: 6, color: accent || "var(--fg)" }}>{value}</div>
      <div className="bnb-meta mono" style={{ marginTop: 4 }}>{sub}</div>
    </div>
  );
}

Object.assign(window, { BackupRecovery });

function ExportOption({ label, detail, size, primary }) {
  return (
    <div style={{
      display: "grid", gridTemplateColumns: "1fr auto auto", gap: 12,
      alignItems: "center",
      padding: "10px 12px",
      background: primary ? "color-mix(in oklch, var(--dawn) 6%, var(--surface-2))" : "var(--surface-2)",
      border: primary ? "0.5px solid color-mix(in oklch, var(--dawn) 30%, var(--border))" : "0.5px solid var(--border)",
      borderRadius: 8,
    }}>
      <div style={{ minWidth: 0 }}>
        <div style={{ fontSize: 13, fontWeight: 500 }}>{label}</div>
        <div className="bnb-meta" style={{ marginTop: 2, fontSize: 11.5, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{detail}</div>
      </div>
      <span className="mono tabular" style={{ fontSize: 11, color: "var(--fg-3)", textAlign: "right" }}>{size}</span>
      <button className={`bnb-btn${primary ? " primary" : ""}`} style={{ fontSize: 11.5, padding: "5px 12px" }}>↓ Export</button>
    </div>
  );
}
