// Photo gallery — visual card grid for browsing species by photo.
// Hobbyist favorite. Uses image-slot placeholders.

function Gallery() {
  const { SPECIES } = window.BNB;

  return (
    <Screen>
      <TopNav active="Species" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Browse · photo gallery</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>{SPECIES.length} portraits</h2>
          <div className="bnb-meta" style={{ marginTop: 6 }}>Drop your own photos onto any card · Wikipedia thumbnails fill the gaps</div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <span className="bnb-pill">A→Z</span>
          <span className="bnb-pill">Recent first</span>
          <span className="bnb-pill mono">grid · 3 cols ▾</span>
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "repeat(5, 1fr)", gap: "var(--pad-3)", flex: 1, alignContent: "flex-start" }}>
        {SPECIES.map((s, i) => (
          <GalleryCard key={i} sp={s} idx={i} />
        ))}
      </div>
    </Screen>
  );
}

function GalleryCard({ sp, idx }) {
  return (
    <div className="bnb-card" style={{ overflow: "hidden", display: "flex", flexDirection: "column" }}>
      <div style={{ position: "relative", aspectRatio: "4/3", background: "var(--surface-2)" }}>
        <BirdPhoto sp={sp} idx={idx} slotId={`gallery-${sp.short}`} />
        <div style={{ position: "absolute", top: 8, left: 8, display: "flex", gap: 4, zIndex: 2 }}>
          {sp.rare && <span className="bnb-pill rare" style={{ fontSize: 9.5 }}>rare</span>}
          {idx < 4 && <span className="bnb-pill moss" style={{ fontSize: 9.5 }}><span className="bnb-dot live" /> active</span>}
        </div>
        <div style={{ position: "absolute", top: 8, right: 8, zIndex: 2 }}>
          <span className="mono" style={{
            padding: "3px 7px", borderRadius: 4, fontSize: 10,
            background: "color-mix(in oklch, var(--bg) 80%, transparent)",
            backdropFilter: "blur(6px)",
            color: "var(--fg)",
          }}>{sp.short}</span>
        </div>
      </div>
      <div style={{ padding: 12, display: "flex", flexDirection: "column", gap: 6 }}>
        <div>
          <div style={{ fontSize: 14, fontWeight: 500, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{sp.common}</div>
          <div className="bnb-meta mono" style={{ fontStyle: "italic", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{sp.sci}</div>
        </div>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginTop: 4 }}>
          <span className="mono tabular" style={{ fontSize: 12, color: "var(--fg-2)" }}>{sp.count.toLocaleString()}</span>
          <Sparkline data={sp.trend} width={80} height={18} accent={sp.color} />
        </div>
      </div>
    </div>
  );
}

Object.assign(window, { Gallery });
