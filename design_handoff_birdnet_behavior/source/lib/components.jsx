// Shared small components for BirdNet-Behavior mockups.
const { useState, useEffect, useRef, useMemo } = React;

// ─── Sparkline ─────────────────────────────────────────────────────────────
function Sparkline({ data, width = 64, height = 18, showArea = true, accent }) {
  const max = Math.max(1, ...data);
  const stepX = data.length > 1 ? width / (data.length - 1) : width;
  const pts = data.map((v, i) => [i * stepX, height - (v / max) * (height - 2) - 1]);
  const path = pts.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
  const area = `${path} L${(pts.at(-1)?.[0] ?? 0).toFixed(1)},${height} L0,${height} Z`;
  const stroke = accent || "var(--moss)";
  return (
    <svg className="bnb-spark" width={width} height={height} viewBox={`0 0 ${width} ${height}`} aria-hidden="true">
      {showArea && <path className="area" d={area} fill={stroke} fillOpacity="0.10" />}
      <path className="line" d={path} stroke={stroke} fill="none" strokeWidth="1.4" />
    </svg>
  );
}

// ─── Bar mini-chart ────────────────────────────────────────────────────────
function MiniBars({ data, width = 120, height = 28, accent }) {
  const max = Math.max(1, ...data);
  const gap = 1;
  const bw = (width - gap * (data.length - 1)) / data.length;
  const fill = accent || "var(--moss)";
  return (
    <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} aria-hidden="true">
      {data.map((v, i) => {
        const h = (v / max) * (height - 2);
        return <rect key={i} x={i * (bw + gap)} y={height - h} width={bw} height={h} rx="0.5" fill={fill} fillOpacity={0.18 + (v / max) * 0.62} />;
      })}
    </svg>
  );
}

// ─── Species avatar (initials chip in species color) ───────────────────────
function SpeciesAvatar({ sp, size = 28 }) {
  const s = window.BNB.SPECIES[sp];
  return (
    <span
      className="mono"
      style={{
        width: size, height: size, borderRadius: "50%",
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        background: `color-mix(in oklch, ${s.color} 22%, var(--surface))`,
        color: s.color,
        fontSize: size < 24 ? 9 : 10, fontWeight: 600, letterSpacing: 0.5,
        flex: "0 0 auto",
      }}
      title={s.common}
    >{s.short}</span>
  );
}

// ─── Confidence bar (0–1) ──────────────────────────────────────────────────
function ConfBar({ value, width = 56 }) {
  const pct = Math.round(value * 100);
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
      <span style={{ width, height: 4, borderRadius: 2, background: "var(--bg-2)", overflow: "hidden", display: "inline-block" }}>
        <span style={{ display: "block", width: `${pct}%`, height: "100%", background: value > 0.9 ? "var(--moss)" : value > 0.75 ? "var(--dawn)" : "var(--fg-3)" }} />
      </span>
      <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)" }}>{(value).toFixed(2)}</span>
    </span>
  );
}

// ─── Mini circadian ring (24h clock) ───────────────────────────────────────
function CircadianRing({ data, size = 48, accent = "var(--moss)" }) {
  // data = 24 values 0..1
  const r = size / 2;
  const ir = r - 6;
  const max = Math.max(0.001, ...data);
  const segs = data.map((v, i) => {
    const a0 = (i / 24) * Math.PI * 2 - Math.PI / 2;
    const a1 = ((i + 1) / 24) * Math.PI * 2 - Math.PI / 2;
    const op = 0.08 + (v / max) * 0.82;
    const x0 = r + ir * Math.cos(a0), y0 = r + ir * Math.sin(a0);
    const x1 = r + ir * Math.cos(a1), y1 = r + ir * Math.sin(a1);
    const xo0 = r + (r - 0.5) * Math.cos(a0), yo0 = r + (r - 0.5) * Math.sin(a0);
    const xo1 = r + (r - 0.5) * Math.cos(a1), yo1 = r + (r - 0.5) * Math.sin(a1);
    return <path key={i} d={`M${x0},${y0} L${xo0},${yo0} A${r-0.5},${r-0.5} 0 0 1 ${xo1},${yo1} L${x1},${y1} A${ir},${ir} 0 0 0 ${x0},${y0} Z`} fill={accent} fillOpacity={op} />;
  });
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
      <circle cx={r} cy={r} r={ir - 1} fill="var(--surface-2)" />
      {segs}
      <circle cx={r} cy={r} r={ir - 1} fill="none" stroke="var(--hairline)" strokeWidth="0.5" />
    </svg>
  );
}

// ─── Screen scaffold: each artboard wraps content in this ─────────────────
function Screen({ children, padded = true, style }) {
  return (
    <div className="bnb-root" style={{ overflow: "hidden", ...style }}>
      <div style={{ padding: padded ? "var(--pad-4)" : 0, height: "100%", display: "flex", flexDirection: "column", gap: "var(--pad-3)" }}>
        {children}
      </div>
    </div>
  );
}

// ─── Top nav (desktop screens share this) ─────────────────────────────────
function TopNav({ active = "Dashboard", scrolled = false }) {
  const items = ["Dashboard", "Today", "Species", "Heatmap", "Analytics", "Life list", "System"];
  return (
    <div style={{
      display: "flex", alignItems: "center", justifyContent: "space-between",
      paddingBottom: "var(--pad-2)", borderBottom: "0.5px solid var(--hairline)",
    }}>
      <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <BrandMark />
          <span style={{ fontWeight: 600, letterSpacing: -0.01, fontSize: 14 }}>BirdNet</span>
          <span style={{ color: "var(--fg-3)", fontSize: 14 }}>Behavior</span>
        </div>
        <span className="bnb-vrule" style={{ height: 14, alignSelf: "center" }} />
        <nav style={{ display: "flex", gap: 2 }}>
          {items.map((label) => (
            <a key={label} href="#" style={{
              padding: "4px 10px", borderRadius: 6, textDecoration: "none",
              fontSize: 13, color: label === active ? "var(--fg)" : "var(--fg-3)",
              background: label === active ? "var(--surface-2)" : "transparent",
              fontWeight: label === active ? 500 : 400,
            }}>{label}</a>
          ))}
        </nav>
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
        <span className="bnb-pill moss"><span className="bnb-dot live" /> Listening</span>
        <span className="bnb-meta mono">42.36°N · −71.06°W</span>
        <span className="bnb-vrule" style={{ height: 14, alignSelf: "center" }} />
        <span className="bnb-meta mono">Pi 5 · 41°C</span>
      </div>
    </div>
  );
}

// ─── Brand mark — a simple sound-wave circle, not a bird illustration. ────
function BrandMark({ size = 22 }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="11" fill="none" stroke="var(--fg)" strokeWidth="0.8" />
      <g stroke="var(--fg)" strokeWidth="1.4" strokeLinecap="round">
        <line x1="6" y1="12" x2="6" y2="12" />
        <line x1="9" y1="9.5" x2="9" y2="14.5" />
        <line x1="12" y1="6" x2="12" y2="18" />
        <line x1="15" y1="8" x2="15" y2="16" />
        <line x1="18" y1="10.5" x2="18" y2="13.5" />
      </g>
    </svg>
  );
}

// ─── Stat ─────────────────────────────────────────────────────────────────
function Stat({ label, value, sub, accent, size = "md" }) {
  const big = size === "lg" ? 36 : size === "sm" ? 20 : 28;
  return (
    <div>
      <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>{label}</div>
      <div className="display tabular" style={{ fontSize: big, lineHeight: 1, color: accent || "var(--fg)" }}>{value}</div>
      {sub && <div className="bnb-meta mono" style={{ marginTop: 6 }}>{sub}</div>}
    </div>
  );
}

// ─── Section header inside an artboard ────────────────────────────────────
function SectionHeader({ eyebrow, title, action }) {
  return (
    <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 12 }}>
      <div>
        {eyebrow && <div className="bnb-eyebrow" style={{ marginBottom: 4 }}>{eyebrow}</div>}
        {title && <h3 className="display" style={{ fontSize: 22 }}>{title}</h3>}
      </div>
      {action}
    </div>
  );
}

// ─── Tooltip dot — the progressive-disclosure helper ──────────────────────
function HelpDot({ children }) {
  const [open, setOpen] = useState(false);
  return (
    <span
      style={{ position: "relative", display: "inline-flex", marginLeft: 4, cursor: "default" }}
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
    >
      <span style={{
        width: 13, height: 13, borderRadius: 999, background: "var(--surface-2)",
        border: "0.5px solid var(--border)", color: "var(--fg-3)",
        fontSize: 9, fontWeight: 600, display: "inline-flex", alignItems: "center", justifyContent: "center",
      }}>?</span>
      {open && (
        <span style={{
          position: "absolute", top: 18, left: -100, width: 220, zIndex: 5,
          background: "var(--fg)", color: "var(--bg)", padding: "8px 10px",
          borderRadius: 6, fontSize: 11.5, lineHeight: 1.45,
          boxShadow: "var(--shadow-md)",
        }}>{children}</span>
      )}
    </span>
  );
}

// ─── BirdPhoto — Wikipedia/eBird-style species photo with user-upload overlay
// Renders the photo URL by default (from data.SPECIES[i].photo). The image-slot
// sits on top — only shows up on hover/empty so users can drop their own.
function BirdPhoto({ sp, idx, slotId, height, attribution = true, style }) {
  const url = sp.photo;
  return (
    <div style={{
      position: "relative",
      width: "100%",
      height: height || "100%",
      background: `linear-gradient(135deg, color-mix(in oklch, ${sp.color} 28%, var(--surface)) 0%, color-mix(in oklch, ${sp.color} 12%, var(--bg-2)) 100%)`,
      overflow: "hidden",
      ...style,
    }}>
      {/* Real photo from Wikipedia */}
      {url && (
        <img
          src={url}
          alt={sp.common}
          loading="lazy"
          referrerPolicy="no-referrer"
          style={{
            position: "absolute", inset: 0, width: "100%", height: "100%",
            objectFit: "cover", display: "block",
          }}
          onError={(e) => { e.currentTarget.style.display = "none"; }}
        />
      )}

      {/* Decorative silhouette fallback (sits below img, only seen if img missing) */}
      <BirdSilhouette sp={sp} />

      {/* Drop-your-own overlay — image-slot mounts only if user drops */}
      <image-slot
        id={slotId || `photo-${sp.short}`}
        shape="rect"
        placeholder="Drop your own photo to override"
        style={{
          position: "absolute", inset: 0, width: "100%", height: "100%",
          display: "block", background: "transparent", opacity: 0,
          transition: "opacity .15s",
        }}
        onMouseEnter={(e) => (e.currentTarget.style.opacity = "0.85")}
        onMouseLeave={(e) => (e.currentTarget.style.opacity = "0")}
      ></image-slot>

      {/* Attribution */}
      {attribution && (
        <div style={{
          position: "absolute", right: 8, bottom: 8,
          padding: "2px 7px", borderRadius: 4,
          background: "color-mix(in oklch, oklch(0% 0 0) 60%, transparent)",
          color: "rgba(255,255,255,.85)",
          fontFamily: "var(--font-mono)", fontSize: 9.5, letterSpacing: 0.02,
          backdropFilter: "blur(4px)",
        }}>↗ Wikipedia · CC BY-SA</div>
      )}
    </div>
  );
}

// Decorative SVG silhouette behind every photo — looks like a stylized
// illustration if the photo fails to load.
function BirdSilhouette({ sp }) {
  return (
    <svg viewBox="0 0 200 140" preserveAspectRatio="xMidYMid slice" style={{
      position: "absolute", inset: 0, width: "100%", height: "100%",
    }}>
      <g fill={sp.color} fillOpacity="0.35">
        {/* simple perched-bird silhouette — generic, evocative */}
        <path d="M 60,90 Q 70,40 110,42 Q 138,42 142,60 L 152,52 L 148,66 L 158,70 L 142,76 Q 138,92 120,98 L 122,118 L 116,118 L 110,100 L 80,100 Q 64,100 60,90 Z" />
        <circle cx="132" cy="56" r="2.5" fill="oklch(0% 0 0)" fillOpacity="0.7" />
      </g>
      {/* species label */}
      <text x="14" y="22" style={{ fontFamily: "var(--font-display)", fontSize: 12, fill: sp.color, fillOpacity: 0.65, fontStyle: "italic" }}>{sp.sci}</text>
    </svg>
  );
}

Object.assign(window, {
  Sparkline, MiniBars, SpeciesAvatar, ConfBar, CircadianRing,
  Screen, TopNav, BrandMark, Stat, SectionHeader, HelpDot,
  BirdPhoto, BirdSilhouette,
});
