// Onboarding — first-run wizard. Designed for the user who has never touched a Pi.
// Five steps with progressive disclosure of advanced options.

const { useState: useState_ob } = React;

function Onboarding() {
  const [step, setStep] = useState_ob(2);

  return (
    <div className="bnb-root" style={{
      width: "100%", height: "100%",
      background: "radial-gradient(ellipse at 50% 0%, color-mix(in oklch, var(--moss) 8%, var(--bg)) 0%, var(--bg) 70%)",
      display: "flex", flexDirection: "column",
    }}>
      {/* Header */}
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "20px 32px", borderBottom: "0.5px solid var(--hairline)" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <BrandMark size={22} />
          <span style={{ fontWeight: 600, fontSize: 14 }}>BirdNet</span>
          <span style={{ fontSize: 14, color: "var(--fg-3)" }}>Behavior</span>
          <span className="bnb-pill mono" style={{ marginLeft: 8 }}>first-run setup</span>
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
          <span className="bnb-meta mono">~ 90 seconds left</span>
          <button className="bnb-btn ghost">Skip ›</button>
        </div>
      </div>

      {/* Stepper */}
      <Stepper step={step} setStep={setStep} />

      {/* Step content */}
      <div style={{ flex: 1, display: "flex", padding: "32px 64px", overflow: "hidden" }}>
        <div style={{ maxWidth: 1100, margin: "0 auto", width: "100%", display: "flex", flexDirection: "column" }}>
          {step === 0 && <WelcomeStep onNext={() => setStep(1)} />}
          {step === 1 && <LocationStep onNext={() => setStep(2)} onBack={() => setStep(0)} />}
          {step === 2 && <AudioStep onNext={() => setStep(3)} onBack={() => setStep(1)} />}
          {step === 3 && <NotificationStep onNext={() => setStep(4)} onBack={() => setStep(2)} />}
          {step === 4 && <DoneStep onBack={() => setStep(3)} />}
        </div>
      </div>
    </div>
  );
}

// ─── Stepper ──────────────────────────────────────────────────────────────
function Stepper({ step, setStep }) {
  const steps = [
    { id: 0, label: "Welcome" },
    { id: 1, label: "Where" },
    { id: 2, label: "How it hears" },
    { id: 3, label: "Who gets notified" },
    { id: 4, label: "Done" },
  ];
  return (
    <div style={{ padding: "24px 64px 12px", display: "flex", justifyContent: "center" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 6, maxWidth: 800, width: "100%" }}>
        {steps.map((s, i) => {
          const done = step > s.id;
          const current = step === s.id;
          return (
            <React.Fragment key={s.id}>
              <button
                onClick={() => setStep(s.id)}
                style={{
                  display: "flex", alignItems: "center", gap: 8,
                  padding: "6px 12px", borderRadius: 999,
                  background: current ? "var(--surface)" : "transparent",
                  border: current ? "0.5px solid var(--border-2)" : "0.5px solid transparent",
                  boxShadow: current ? "var(--shadow-sm)" : "none",
                  cursor: "pointer",
                  color: done || current ? "var(--fg)" : "var(--fg-3)",
                  fontSize: 12.5,
                  fontWeight: current ? 600 : 400,
                }}
              >
                <span style={{
                  width: 20, height: 20, borderRadius: 999,
                  display: "inline-flex", alignItems: "center", justifyContent: "center",
                  background: done ? "var(--moss)" : current ? "var(--fg)" : "var(--bg-2)",
                  color: done || current ? "var(--bg)" : "var(--fg-3)",
                  fontSize: 11, fontWeight: 600,
                }} className="mono">{done ? "✓" : s.id + 1}</span>
                {s.label}
              </button>
              {i < steps.length - 1 && <span style={{ flex: 1, height: 1, background: done ? "var(--moss)" : "var(--hairline)" }} />}
            </React.Fragment>
          );
        })}
      </div>
    </div>
  );
}

// ─── Step 1: Welcome ──────────────────────────────────────────────────────
function WelcomeStep({ onNext }) {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 60, alignItems: "center", flex: 1 }}>
      <div>
        <div className="bnb-eyebrow" style={{ marginBottom: 10 }}>Welcome</div>
        <h1 className="display" style={{ fontSize: 84, lineHeight: 0.95, letterSpacing: "-0.03em" }}>
          Let's teach your Pi <em style={{ color: "var(--moss-ink)" }}>to listen</em>.
        </h1>
        <p style={{ marginTop: 22, fontSize: 16, color: "var(--fg-2)", lineHeight: 1.55, maxWidth: 460 }}>
          In the next 90 seconds we'll find your microphone, set your location, and decide who gets pinged when a rare bird visits. After that, you can ignore us forever — the dashboard does the rest.
        </p>
        <div style={{ display: "flex", gap: 12, marginTop: 32, alignItems: "center" }}>
          <button onClick={onNext} className="bnb-btn primary" style={{ padding: "10px 18px", fontSize: 14 }}>Start →</button>
          <a href="#" className="bnb-meta" style={{ textDecoration: "underline" }}>I'm migrating from BirdNET-Pi</a>
        </div>
        <div style={{ marginTop: 56, display: "flex", gap: 32 }}>
          <Bullet glyph="●" title="No accounts" sub="Nothing leaves your network unless you opt in." />
          <Bullet glyph="●" title="Set once" sub="Defaults work for 95% of yards." />
          <Bullet glyph="●" title="Always tweakable" sub="Everything here lives in /admin." />
        </div>
      </div>
      <SonarIllustration />
    </div>
  );
}

function Bullet({ glyph, title, sub }) {
  return (
    <div style={{ maxWidth: 140 }}>
      <span style={{ color: "var(--moss)", fontSize: 11 }}>{glyph}</span>
      <div style={{ fontSize: 13, fontWeight: 600, marginTop: 6 }}>{title}</div>
      <div className="bnb-meta" style={{ marginTop: 4, lineHeight: 1.45 }}>{sub}</div>
    </div>
  );
}

function SonarIllustration() {
  return (
    <div style={{ position: "relative", width: "100%", aspectRatio: "1", display: "flex", alignItems: "center", justifyContent: "center" }}>
      <svg viewBox="0 0 400 400" width="100%" height="100%" style={{ maxWidth: 460 }}>
        <defs>
          <radialGradient id="ob-glow" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor="var(--moss)" stopOpacity="0.18" />
            <stop offset="60%" stopColor="var(--moss)" stopOpacity="0.04" />
            <stop offset="100%" stopColor="var(--moss)" stopOpacity="0" />
          </radialGradient>
        </defs>
        <circle cx="200" cy="200" r="200" fill="url(#ob-glow)" />
        {/* concentric rings */}
        {[60, 100, 140, 180].map((r, i) => (
          <circle key={i} cx="200" cy="200" r={r} fill="none" stroke="var(--moss)" strokeWidth="1" strokeOpacity={0.4 - i * 0.06}>
            <animate attributeName="r" from={r} to={r + 14} dur={`${4 + i * 0.4}s`} repeatCount="indefinite" />
            <animate attributeName="stroke-opacity" from={0.4 - i * 0.06} to="0" dur={`${4 + i * 0.4}s`} repeatCount="indefinite" />
          </circle>
        ))}
        {/* Pi at the center — minimal */}
        <rect x="170" y="180" width="60" height="40" rx="4" fill="var(--surface)" stroke="var(--border-2)" strokeWidth="1.5" />
        <circle cx="183" cy="195" r="2.5" fill="var(--moss)" />
        <circle cx="183" cy="205" r="2.5" fill="var(--fg-3)" />
        {/* bird silhouettes */}
        <text x="80" y="100" fontSize="18" fill="var(--moss-ink)" opacity="0.7">𓅂</text>
        <text x="300" y="130" fontSize="22" fill="var(--moss-ink)" opacity="0.55">𓅂</text>
        <text x="320" y="280" fontSize="14" fill="var(--moss-ink)" opacity="0.40">𓅂</text>
        <text x="70" y="290" fontSize="18" fill="var(--moss-ink)" opacity="0.55">𓅂</text>
      </svg>
    </div>
  );
}

// ─── Step 2: Location ─────────────────────────────────────────────────────
function LocationStep({ onNext, onBack }) {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 56, alignItems: "center", flex: 1 }}>
      <div>
        <div className="bnb-eyebrow" style={{ marginBottom: 10 }}>Where you are</div>
        <h2 className="display" style={{ fontSize: 56, lineHeight: 1.0, letterSpacing: "-0.025em" }}>
          Set your <em style={{ color: "var(--moss-ink)" }}>backyard</em>.
        </h2>
        <p style={{ marginTop: 18, fontSize: 15, color: "var(--fg-2)", lineHeight: 1.55, maxWidth: 480 }}>
          We use your latitude/longitude to figure out sunrise & sunset, and which species are actually plausible at your location. eBird does the regional homework so we don't tell you a Painted Bunting visited your Vermont snowbank.
        </p>

        <div className="bnb-card" style={{ padding: "var(--pad-3)", marginTop: 28 }}>
          <div style={{ display: "flex", gap: 14, marginBottom: 14 }}>
            <button className="bnb-btn primary" style={{ flex: 1, justifyContent: "center" }}>📍 Auto-detect (uses ipapi.co)</button>
            <button className="bnb-btn" style={{ flex: 1, justifyContent: "center" }}>Enter manually</button>
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
            <FormField label="Latitude" value="42.3601" unit="°N" />
            <FormField label="Longitude" value="−71.0589" unit="°W" />
          </div>
          <div className="bnb-meta" style={{ marginTop: 14, padding: 10, background: "var(--moss-soft)", color: "var(--moss-ink)", borderRadius: 8, display: "flex", gap: 10, alignItems: "center" }}>
            <span style={{ fontSize: 18 }}>✓</span>
            <span>Boston, MA · 247 species expected this time of year · sunrise 5:21 AM</span>
          </div>
        </div>

        <div style={{ display: "flex", justifyContent: "space-between", marginTop: 28 }}>
          <button onClick={onBack} className="bnb-btn ghost">← Back</button>
          <button onClick={onNext} className="bnb-btn primary" style={{ padding: "10px 18px", fontSize: 14 }}>Continue →</button>
        </div>
      </div>

      <MapPreview />
    </div>
  );
}

function FormField({ label, value, unit }) {
  return (
    <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <span className="bnb-eyebrow" style={{ fontSize: 10 }}>{label}</span>
      <div style={{ display: "flex", alignItems: "center", background: "var(--surface)", border: "0.5px solid var(--border-2)", borderRadius: 8 }}>
        <input
          defaultValue={value}
          style={{ flex: 1, border: 0, outline: 0, padding: "10px 12px", fontSize: 16, color: "var(--fg)", fontFamily: "var(--font-mono)", background: "transparent", fontVariantNumeric: "tabular-nums" }}
        />
        <span className="mono" style={{ padding: "0 12px", color: "var(--fg-3)", fontSize: 13 }}>{unit}</span>
      </div>
    </label>
  );
}

function MapPreview() {
  return (
    <div style={{ position: "relative", width: "100%", aspectRatio: "4/3", borderRadius: 14, overflow: "hidden", border: "0.5px solid var(--border)", background: "var(--surface-2)", boxShadow: "var(--shadow-md)" }}>
      <svg viewBox="0 0 400 300" width="100%" height="100%" style={{ display: "block" }}>
        {/* Topographic contours */}
        <defs>
          <pattern id="topo" x="0" y="0" width="40" height="40" patternUnits="userSpaceOnUse">
            <path d="M0,20 Q10,12 20,20 T40,20" fill="none" stroke="var(--moss-soft)" strokeWidth="0.5" opacity="0.5" />
            <path d="M0,30 Q10,22 20,30 T40,30" fill="none" stroke="var(--moss-soft)" strokeWidth="0.5" opacity="0.4" />
          </pattern>
        </defs>
        <rect width="400" height="300" fill="url(#topo)" />
        {/* water bodies */}
        <path d="M0,200 Q100,180 200,200 T400,210 L400,300 L0,300 Z" fill="color-mix(in oklch, oklch(60% 0.10 230) 30%, var(--surface-2))" opacity="0.7" />
        <ellipse cx="120" cy="100" rx="40" ry="25" fill="color-mix(in oklch, oklch(60% 0.10 230) 25%, var(--surface-2))" opacity="0.6" />
        {/* station marker — bullseye */}
        <g>
          {[8, 18, 30].map((r, i) => (
            <circle key={i} cx="200" cy="150" r={r} fill="none" stroke="var(--moss)" strokeWidth={1.2 - i * 0.2} opacity={0.9 - i * 0.25} />
          ))}
          <circle cx="200" cy="150" r="4" fill="var(--moss)" />
        </g>
        <text x="216" y="146" fontSize="11" fill="var(--fg)" fontWeight="500">42.36°N, −71.06°W</text>
        <text x="216" y="158" fontSize="9" fill="var(--fg-3)" fontFamily="var(--font-mono)">station</text>
        {/* 100 km circle */}
        <circle cx="200" cy="150" r="92" fill="none" stroke="var(--dawn)" strokeWidth="1" strokeDasharray="2 3" />
        <text x="290" y="148" fontSize="10" fill="var(--dawn-ink)" fontFamily="var(--font-mono)">100 km</text>
      </svg>
      <div style={{ position: "absolute", bottom: 12, left: 12, padding: "6px 10px", background: "color-mix(in oklch, var(--bg) 90%, transparent)", backdropFilter: "blur(8px)", borderRadius: 6, fontSize: 11, color: "var(--fg-2)" }}>
        Map preview · 100 km baseline radius
      </div>
    </div>
  );
}

// ─── Step 3: Audio source ─────────────────────────────────────────────────
function AudioStep({ onNext, onBack }) {
  const [selected, setSelected] = useState_ob("usb-1");

  const options = [
    {
      id: "usb-1", kind: "usb", name: "USB Audio Device", device: "plughw:1,0",
      detail: "auto-detected · 48 kHz · 1 channel", recommended: true, level: 0.62,
    },
    {
      id: "usb-2", kind: "usb", name: "HD Webcam C920", device: "plughw:2,0",
      detail: "built-in mic · 16 kHz · stereo", level: 0.18,
    },
    {
      id: "rtsp", kind: "rtsp", name: "Add an RTSP camera", device: "rtsp://…",
      detail: "any IP camera with audio · 8 / 16 / 48 kHz",
    },
    {
      id: "files", kind: "files", name: "Watch a folder of files",
      device: "/recordings/incoming/",
      detail: "advanced · for offline batch processing",
    },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1 }}>
      <div className="bnb-eyebrow" style={{ marginBottom: 10 }}>How it hears</div>
      <h2 className="display" style={{ fontSize: 48, lineHeight: 1.05, letterSpacing: "-0.025em", maxWidth: 760 }}>
        Pick a microphone.
      </h2>
      <p style={{ marginTop: 14, fontSize: 15, color: "var(--fg-2)", lineHeight: 1.55, maxWidth: 720 }}>
        We auto-scanned the Pi and found two USB devices. Choose one — or set up a network camera if your station is at the back of the property.
      </p>

      <div style={{ marginTop: 28, display: "grid", gridTemplateColumns: "1fr 1fr", gap: 14 }}>
        {options.map((opt) => (
          <AudioOption key={opt.id} option={opt} selected={selected === opt.id} onSelect={() => setSelected(opt.id)} />
        ))}
      </div>

      <div className="bnb-meta" style={{ marginTop: 16, padding: 12, background: "var(--surface-2)", borderRadius: 8, display: "flex", alignItems: "center", gap: 10 }}>
        <span style={{ width: 18, height: 18, borderRadius: 999, background: "var(--moss-soft)", color: "var(--moss-ink)", display: "inline-flex", alignItems: "center", justifyContent: "center", fontSize: 10, fontWeight: 700 }}>i</span>
        <span>You can add more later. Multiple microphones run in parallel and tag detections with their source.</span>
      </div>

      <div style={{ display: "flex", justifyContent: "space-between", marginTop: "auto", paddingTop: 28 }}>
        <button onClick={onBack} className="bnb-btn ghost">← Back</button>
        <button onClick={onNext} className="bnb-btn primary" style={{ padding: "10px 18px", fontSize: 14 }}>Continue →</button>
      </div>
    </div>
  );
}

function AudioOption({ option, selected, onSelect }) {
  return (
    <button
      onClick={onSelect}
      style={{
        background: selected ? "color-mix(in oklch, var(--moss) 6%, var(--surface))" : "var(--surface)",
        border: selected ? "1px solid var(--moss)" : "0.5px solid var(--border-2)",
        borderRadius: 12,
        padding: "var(--pad-3)",
        textAlign: "left",
        cursor: "pointer",
        display: "flex", flexDirection: "column", gap: 12,
        boxShadow: selected ? "0 0 0 4px color-mix(in oklch, var(--moss) 14%, transparent)" : "none",
        position: "relative",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
        <span style={{
          width: 38, height: 38, borderRadius: 10,
          background: option.kind === "usb" ? "color-mix(in oklch, var(--moss) 14%, var(--surface))" : option.kind === "rtsp" ? "color-mix(in oklch, oklch(58% 0.14 240) 14%, var(--surface))" : "var(--bg-2)",
          color: option.kind === "usb" ? "var(--moss-ink)" : option.kind === "rtsp" ? "oklch(58% 0.14 240)" : "var(--fg-3)",
          display: "flex", alignItems: "center", justifyContent: "center",
        }}>
          {option.kind === "usb" ? "🎙" : option.kind === "rtsp" ? "📡" : "📁"}
        </span>
        <div style={{ flex: 1 }}>
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <span style={{ fontSize: 15, fontWeight: 500 }}>{option.name}</span>
            {option.recommended && <span className="bnb-pill moss" style={{ fontSize: 10 }}>recommended</span>}
          </div>
          <div className="mono" style={{ fontSize: 11.5, color: "var(--fg-3)", marginTop: 2 }}>{option.device}</div>
        </div>
        <span style={{
          width: 20, height: 20, borderRadius: 999,
          background: selected ? "var(--moss)" : "transparent",
          border: selected ? 0 : "1.5px solid var(--border-2)",
          display: "flex", alignItems: "center", justifyContent: "center",
          color: "var(--bg)", fontSize: 11, fontWeight: 700,
        }}>{selected && "✓"}</span>
      </div>
      <div className="bnb-meta" style={{ fontSize: 12 }}>{option.detail}</div>
      {option.level !== undefined && (
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <span className="bnb-meta" style={{ fontSize: 10 }}>live level</span>
          <span style={{ flex: 1, display: "flex", gap: 1.5, alignItems: "flex-end", height: 14 }}>
            {Array.from({ length: 24 }).map((_, i) => {
              const isOn = i < option.level * 24;
              return <span key={i} style={{ width: 3, height: 4 + (i / 24) * 10, background: isOn ? "var(--moss)" : "var(--bg-2)", borderRadius: 1 }} />;
            })}
          </span>
        </div>
      )}
    </button>
  );
}

// ─── Step 4: Notifications ───────────────────────────────────────────────
function NotificationStep({ onNext, onBack }) {
  const [pick, setPick] = useState_ob("rare");
  const choices = [
    { id: "off",   title: "None",                sub: "Just look at the dashboard when you want.", glyph: "○" },
    { id: "rare",  title: "Rare birds only",     sub: "Get pinged when a species first lands on your life list.", glyph: "✦", reco: true },
    { id: "daily", title: "Daily digest",        sub: "One summary message at sunset.", glyph: "✉" },
    { id: "all",   title: "Everything",          sub: "Every detection above confidence 0.9. Noisy.", glyph: "≡" },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1 }}>
      <div className="bnb-eyebrow" style={{ marginBottom: 10 }}>Notifications</div>
      <h2 className="display" style={{ fontSize: 48, lineHeight: 1.05, letterSpacing: "-0.025em" }}>
        Who hears about visitors?
      </h2>
      <p style={{ marginTop: 14, fontSize: 15, color: "var(--fg-2)", lineHeight: 1.55, maxWidth: 720 }}>
        You can configure 80+ channels later (Telegram, Slack, Discord, email, MQTT, Home Assistant, push, webhook…). For now pick what you'd like.
      </p>

      <div style={{ marginTop: 28, display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 12 }}>
        {choices.map((c) => (
          <button key={c.id} onClick={() => setPick(c.id)}
            style={{
              background: pick === c.id ? "color-mix(in oklch, var(--moss) 6%, var(--surface))" : "var(--surface)",
              border: pick === c.id ? "1px solid var(--moss)" : "0.5px solid var(--border-2)",
              borderRadius: 12, padding: "var(--pad-3)",
              textAlign: "left", cursor: "pointer",
              display: "flex", flexDirection: "column", gap: 12,
              boxShadow: pick === c.id ? "0 0 0 4px color-mix(in oklch, var(--moss) 14%, transparent)" : "none",
              minHeight: 160,
            }}>
            <span style={{ fontSize: 30, color: "var(--moss-ink)" }}>{c.glyph}</span>
            <div>
              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <span style={{ fontSize: 15, fontWeight: 500 }}>{c.title}</span>
                {c.reco && <span className="bnb-pill moss" style={{ fontSize: 9.5 }}>start here</span>}
              </div>
              <div className="bnb-meta" style={{ marginTop: 4, fontSize: 12, lineHeight: 1.45 }}>{c.sub}</div>
            </div>
          </button>
        ))}
      </div>

      <details style={{ marginTop: 24, padding: "var(--pad-3)", background: "var(--surface-2)", borderRadius: 10 }}>
        <summary style={{ cursor: "default", listStyle: "none", display: "flex", justifyContent: "space-between", alignItems: "center", outline: "none" }}>
          <div>
            <div style={{ fontSize: 13, fontWeight: 500 }}>Pick channels now (optional)</div>
            <div className="bnb-meta" style={{ marginTop: 4 }}>Hobbyists usually skip this — you can do it any time at /admin/notifications.</div>
          </div>
          <span style={{ color: "var(--fg-3)" }}>▾</span>
        </summary>
        <div style={{ marginTop: 14, display: "grid", gridTemplateColumns: "repeat(6, 1fr)", gap: 10 }}>
          {["Email", "Telegram", "Discord", "Slack", "MQTT", "Push", "Webhook", "Pushover", "Apprise", "ntfy.sh", "Matrix", "Gotify"].map((channel) => (
            <div key={channel} style={{ padding: "10px 12px", background: "var(--surface)", border: "0.5px solid var(--border)", borderRadius: 8, fontSize: 12.5, textAlign: "center" }}>{channel}</div>
          ))}
        </div>
      </details>

      <div style={{ display: "flex", justifyContent: "space-between", marginTop: "auto", paddingTop: 28 }}>
        <button onClick={onBack} className="bnb-btn ghost">← Back</button>
        <button onClick={onNext} className="bnb-btn primary" style={{ padding: "10px 18px", fontSize: 14 }}>Continue →</button>
      </div>
    </div>
  );
}

// ─── Step 5: Done ─────────────────────────────────────────────────────────
function DoneStep({ onBack }) {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 56, alignItems: "center", flex: 1 }}>
      <div>
        <div className="bnb-eyebrow" style={{ marginBottom: 10, color: "var(--moss-ink)" }}>Done</div>
        <h1 className="display" style={{ fontSize: 72, lineHeight: 0.95, letterSpacing: "-0.03em" }}>
          You're <em style={{ color: "var(--moss-ink)" }}>listening</em>.
        </h1>
        <p style={{ marginTop: 22, fontSize: 16, color: "var(--fg-2)", lineHeight: 1.55, maxWidth: 480 }}>
          The Pi is calibrating the model now. Your first detection should arrive in two to three minutes — sooner if you have an active feeder nearby.
        </p>

        <div className="bnb-card" style={{ padding: "var(--pad-3)", marginTop: 28 }}>
          <div className="bnb-eyebrow" style={{ marginBottom: 12 }}>Your station</div>
          <SummaryRow k="Location" v="Boston, MA · 42.36°N −71.06°W" />
          <SummaryRow k="Microphone" v="USB Audio Device · plughw:1,0" />
          <SummaryRow k="Notifications" v="Rare birds only" />
          <SummaryRow k="Dashboard" v="http://birdnet.local:8502" mono />
        </div>

        <div style={{ display: "flex", gap: 12, marginTop: 28 }}>
          <button className="bnb-btn primary" style={{ padding: "10px 18px", fontSize: 14 }}>Open dashboard →</button>
          <button onClick={onBack} className="bnb-btn ghost">← Back</button>
        </div>
      </div>

      <FirstDetectionPreview />
    </div>
  );
}

function SummaryRow({ k, v, mono }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", padding: "8px 0", borderTop: "0.5px solid var(--hairline)", fontSize: 13 }}>
      <span style={{ color: "var(--fg-3)" }}>{k}</span>
      <span style={{ color: "var(--fg)", fontFamily: mono ? "var(--font-mono)" : undefined }}>{v}</span>
    </div>
  );
}

function FirstDetectionPreview() {
  return (
    <div style={{ position: "relative", width: "100%", aspectRatio: "1/1.1", borderRadius: 16, overflow: "hidden", background: "var(--surface)", border: "0.5px solid var(--border)", boxShadow: "var(--shadow-lg)", padding: "var(--pad-4)", display: "flex", flexDirection: "column", justifyContent: "space-between" }}>
      {/* mock dashboard frame */}
      <div>
        <div className="bnb-eyebrow">Right now</div>
        <div className="display" style={{ fontSize: 38, lineHeight: 1.05, marginTop: 4 }}>
          The yard is <em style={{ color: "var(--moss-ink)" }}>warming up</em>.
        </div>
        <div className="bnb-meta" style={{ marginTop: 6 }}>0 detections so far · ready to listen</div>
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
        <div style={{ background: "var(--surface-2)", padding: 14, borderRadius: 10 }}>
          <div className="bnb-eyebrow" style={{ fontSize: 9 }}>Calibrating</div>
          <div style={{ display: "flex", gap: 1.5, marginTop: 10, height: 30, alignItems: "center" }}>
            {Array.from({ length: 30 }).map((_, i) => {
              const v = Math.abs(Math.sin(i * 0.5 + Date.now() / 1000)) * 0.7 + 0.3;
              return <span key={i} style={{ width: 3, height: `${v * 28}px`, background: "var(--moss)", opacity: 0.4 + v * 0.4, borderRadius: 1 }} />;
            })}
          </div>
          <div className="bnb-meta mono" style={{ marginTop: 6 }}>SNR 13.8 dB</div>
        </div>
        <div style={{ background: "var(--surface-2)", padding: 14, borderRadius: 10 }}>
          <div className="bnb-eyebrow" style={{ fontSize: 9 }}>Model</div>
          <div className="display tabular" style={{ fontSize: 24, marginTop: 6, color: "var(--moss-ink)" }}>BirdNET+</div>
          <div className="bnb-meta mono" style={{ marginTop: 4 }}>V3.0 · 11,000 species</div>
        </div>
      </div>
      {/* watermark */}
      <div style={{ position: "absolute", top: 14, right: 14, padding: "4px 8px", background: "var(--bg)", border: "0.5px solid var(--border)", borderRadius: 4, fontSize: 9, color: "var(--fg-3)", fontFamily: "var(--font-mono)" }}>preview</div>
    </div>
  );
}

Object.assign(window, { Onboarding });
