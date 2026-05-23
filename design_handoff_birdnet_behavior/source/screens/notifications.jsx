// Notifications center — channels list, history, test buttons.
function Notifications() {
  const { SPECIES } = window.BNB;

  const channels = [
    { id: "rare-alerts",    name: "Rare-bird alerts",   kind: "telegram", target: "@username",         sent: 14, last: "2h ago",    active: true,  rule: "first-of-station OR ρ < 0.05" },
    { id: "daily-digest",   name: "Daily digest",       kind: "email",    target: "you@home.lan",      sent: 437, last: "yesterday", active: true,  rule: "Sunday recap · 20:00" },
    { id: "ha-broadcast",   name: "Home Assistant",     kind: "mqtt",     target: "birdnet/events",    sent: 6238, last: "live",      active: true,  rule: "every detection · ≥ 0.85" },
    { id: "ifttt-bonus",    name: "IFTTT trigger",      kind: "webhook",  target: "maker.ifttt.com",   sent: 22,  last: "3d ago",    active: true,  rule: "first species of day" },
    { id: "research-team",  name: "Lab Slack",          kind: "slack",    target: "#bnb-station-12",   sent: 81,  last: "5h ago",    active: true,  rule: "all detections · weekday hours" },
    { id: "field-discord",  name: "Discord field log",  kind: "discord",  target: "field-station-12",  sent: 11,  last: "4d ago",    active: false, rule: "manual" },
  ];

  const recent = [
    { ch: "rare-alerts",  sp: 14, when: "2 hours ago",   subject: "Rare detection: Barred Owl @ 02:14",          status: "delivered" },
    { ch: "ha-broadcast", sp: 1,  when: "14 minutes ago", subject: "birdnet/events · Northern Cardinal",          status: "delivered" },
    { ch: "ha-broadcast", sp: 0,  when: "16 minutes ago", subject: "birdnet/events · Blue Jay",                   status: "delivered" },
    { ch: "research-team",sp: 5,  when: "5 hours ago",    subject: "Hourly summary · 9 species, 48 detections",   status: "delivered" },
    { ch: "daily-digest", sp: 1,  when: "Yesterday 8 PM", subject: "Wed May 21 · 23 species · 1,108 detections",  status: "delivered" },
    { ch: "rare-alerts",  sp: 12, when: "Sat May 18",     subject: "Rare detection: Rose-breasted Grosbeak",       status: "delivered" },
    { ch: "ifttt-bonus",  sp: 9,  when: "Wed May 15",     subject: "Trigger fired: first species of day = YRWA",   status: "delivered" },
  ];

  return (
    <Screen>
      <TopNav active="System" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Settings · notifications</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>Who hears about visitors</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 580 }}>
            BirdNet-Behavior speaks 80+ channels via Apprise plus direct MQTT, email, BirdWeather, and webhooks. Each channel has its own rule for when it fires.
          </div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <span className="bnb-pill moss"><span className="bnb-dot live" /> 5 active</span>
          <span className="bnb-pill">1 paused</span>
          <button className="bnb-btn primary">＋ Add channel</button>
        </div>
      </div>

      {/* Stat strip */}
      <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 0, border: "0.5px solid var(--border)", borderRadius: 12, overflow: "hidden", background: "var(--surface)" }}>
        <StripStat label="Sent · 24h"        value={134}     sub="across 4 channels" />
        <StripStat label="Sent · all-time"   value="6,809"   sub="since first install" />
        <StripStat label="Delivery rate"     value="99.9%"   sub="2 retries, 0 fails" accent="var(--moss-ink)" />
        <StripStat label="Avg latency"       value="1.4 s"   sub="detection → ping"   last />
      </div>

      {/* Two-pane */}
      <div style={{ display: "grid", gridTemplateColumns: "1.1fr 1fr", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        {/* Channels */}
        <div className="bnb-card" style={{ padding: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ padding: "12px 16px", borderBottom: "0.5px solid var(--hairline)", display: "flex", justifyContent: "space-between" }}>
            <div className="bnb-eyebrow">Channels</div>
            <div style={{ display: "flex", gap: 6 }}>
              <span className="bnb-pill mono">all</span>
            </div>
          </div>
          {channels.map((c, i) => <ChannelRow key={c.id} ch={c} />)}
        </div>

        {/* History */}
        <div className="bnb-card" style={{ padding: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ padding: "12px 16px", borderBottom: "0.5px solid var(--hairline)", display: "flex", justifyContent: "space-between" }}>
            <div className="bnb-eyebrow">Recent events</div>
            <span className="bnb-meta mono">7</span>
          </div>
          {recent.map((r, i) => (
            <div key={i} style={{ padding: "14px 18px", borderBottom: "0.5px solid var(--hairline)", display: "grid", gridTemplateColumns: "32px 1fr 110px", gap: 14, alignItems: "center" }}>
              <ChannelGlyph kind={channels.find((c) => c.id === r.ch)?.kind} />
              <div style={{ minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 500, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", lineHeight: 1.35 }}>{r.subject}</div>
                <div className="bnb-meta mono" style={{ marginTop: 4 }}>{r.ch} · {r.when}</div>
              </div>
              <span className="bnb-pill moss" style={{ fontSize: 10, justifyContent: "center" }}>✓ {r.status}</span>
            </div>
          ))}
        </div>
      </div>
    </Screen>
  );
}

function ChannelRow({ ch }) {
  return (
    <div style={{ padding: "16px", borderBottom: "0.5px solid var(--hairline)", display: "grid", gridTemplateColumns: "32px 1fr auto 120px", gap: 14, alignItems: "center" }}>
      <ChannelGlyph kind={ch.kind} />
      <div style={{ minWidth: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ fontSize: 14, fontWeight: 500 }}>{ch.name}</span>
          {!ch.active && <span className="bnb-pill" style={{ fontSize: 9.5, color: "var(--fg-3)" }}>paused</span>}
        </div>
        <div className="bnb-meta mono" style={{ marginTop: 2 }}>{ch.kind} → {ch.target}</div>
        <div className="bnb-meta" style={{ marginTop: 4, fontSize: 11.5 }}>Rule: <span className="mono">{ch.rule}</span></div>
      </div>
      <div style={{ textAlign: "right" }}>
        <div className="mono tabular" style={{ fontSize: 14, color: "var(--fg)" }}>{ch.sent.toLocaleString()}</div>
        <div className="bnb-meta mono" style={{ marginTop: 2 }}>last {ch.last}</div>
      </div>
      <div style={{ display: "flex", gap: 4, justifyContent: "flex-end" }}>
        <button className="bnb-btn ghost" style={{ fontSize: 11 }}>Test</button>
        <button className="bnb-btn ghost" style={{ fontSize: 11 }}>Edit</button>
        <button className="bnb-btn ghost">⋯</button>
      </div>
    </div>
  );
}

function ChannelGlyph({ kind }) {
  const m = {
    telegram: { bg: "color-mix(in oklch, oklch(60% 0.16 230) 22%, var(--surface))", fg: "oklch(60% 0.16 230)", glyph: "✈" },
    email:    { bg: "color-mix(in oklch, oklch(58% 0.12 30) 22%, var(--surface))",  fg: "oklch(58% 0.12 30)",  glyph: "✉" },
    mqtt:     { bg: "color-mix(in oklch, var(--moss) 18%, var(--surface))",         fg: "var(--moss-ink)",     glyph: "≋" },
    webhook:  { bg: "color-mix(in oklch, var(--fg-3) 18%, var(--surface))",         fg: "var(--fg-2)",         glyph: "{}" },
    slack:    { bg: "color-mix(in oklch, oklch(58% 0.16 320) 22%, var(--surface))", fg: "oklch(58% 0.16 320)", glyph: "#" },
    discord:  { bg: "color-mix(in oklch, oklch(58% 0.16 270) 22%, var(--surface))", fg: "oklch(58% 0.16 270)", glyph: "◉" },
  }[kind] || { bg: "var(--surface-2)", fg: "var(--fg-3)", glyph: "·" };
  return (
    <span style={{ width: 32, height: 32, borderRadius: 8, background: m.bg, color: m.fg, display: "inline-flex", alignItems: "center", justifyContent: "center", fontSize: 14, fontWeight: 600 }}>
      {m.glyph}
    </span>
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

Object.assign(window, { Notifications });
