// Root App — vertical scroll layout with sticky TOC sidebar.

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "theme": "light",
  "density": "regular",
  "demo": "busy",
  "accent": "moss",
  "displayFont": "Instrument Serif"
}/*EDITMODE-END*/;

const SECTIONS = [
  {
    id: "onboarding", label: "Onboarding", eyebrow: "00", title: "First-run setup",
    subtitle: "The five-step wizard a non-technical user sees the first time they open the dashboard.",
    frames: [
      { id: "onboarding", label: "First-run wizard", w: 1440, h: 900, render: () => <Onboarding /> },
    ],
  },
  {
    id: "overview", label: "Overview", eyebrow: "01–02", title: "Overview",
    subtitle: "Live feed, today's pulse, the right-now view.",
    frames: [
      { id: "dashboard", label: "Dashboard — live feed + activity",  w: 1440, h: 1080, render: (t) => <Dashboard demo={t.demo} /> },
      { id: "today",     label: "Today — searchable detection log",  w: 1440, h: 1080, render: () => <TodayList /> },
    ],
  },
  {
    id: "analytics", label: "Analytics", eyebrow: "03–08", title: "Behavioral analytics — the deep dive",
    subtitle: "New visualizations for the behavioral story: when birds sing, who they sing with, when they arrive.",
    frames: [
      { id: "heatmap",      label: "When the yard is alive — streamgraph + hour×day mosaic", w: 1440, h: 1080, render: () => <HeatmapScreen /> },
      { id: "dawn-chorus",  label: "The dawn chorus — circadian polar",                    w: 1440, h: 920,  render: () => <DawnChorus /> },
      { id: "cooccurrence", label: "Who sings with whom — co-occurrence matrix",           w: 1440, h: 920,  render: () => <CoOccurrence /> },
      { id: "network",      label: "The acoustic network — chord diagram",                 w: 1440, h: 1020, render: () => <AcousticNetwork /> },
      { id: "migration",    label: "Arrivals and departures — migration phenology",        w: 1440, h: 1020, render: () => <Migration /> },
      { id: "spectrogram",  label: "The 30-second window — live spectrogram",              w: 1440, h: 920,  render: (t) => <Spectrogram demo={t.demo} /> },
    ],
  },
  {
    id: "browse", label: "Browse", eyebrow: "09–13", title: "Browse & journal",
    subtitle: "Surface a species, listen to a clip, walk the year, scan the gallery.",
    frames: [
      { id: "species-list", label: "Species list — every voice",            w: 1440, h: 1020, render: () => <SpeciesList /> },
      { id: "species",      label: "Species detail — Northern Cardinal",     w: 1440, h: 1140, render: () => <SpeciesDetail /> },
      { id: "gallery",      label: "Photo gallery — drop your own photos",   w: 1440, h: 1100, render: () => <Gallery /> },
      { id: "recordings",   label: "Recordings — listen to detection clips", w: 1440, h: 920,  render: () => <Recordings /> },
      { id: "lifelist",     label: "Life list — year tape + journal",        w: 1440, h: 1020, render: () => <LifeList /> },
    ],
  },
  {
    id: "history", label: "History", eyebrow: "14–17", title: "History · trends · comparisons",
    subtitle: "Browse the past as a calendar. Compare week-over-week, month-over-month, year-over-year. Read the Sunday newspaper. Live the year in review.",
    frames: [
      { id: "history",        label: "History — calendar date browser",        w: 1440, h: 920,  render: () => <History /> },
      { id: "trends",         label: "Trends — week / month / year comparisons", w: 1440, h: 1700, render: () => <Trends /> },
      { id: "year-in-review", label: "Year in Review — annual recap",          w: 1440, h: 2400, render: () => <YearInReview /> },
      { id: "weekly",         label: "Weekly report — the Backyard Bulletin",  w: 1440, h: 1020, render: () => <WeeklyReport /> },
    ],
  },
  {
    id: "operations", label: "Operations", eyebrow: "18–24", title: "System, settings & operations",
    subtitle: "Fool-proof for the hobbyist, real for the researcher. Progressive disclosure throughout.",
    frames: [
      { id: "health",     label: "System health",                              w: 1440, h: 1020, render: () => <SystemHealth /> },
      { id: "audio",      label: "Audio settings — RTSP microphone setup",     w: 1440, h: 1450, render: () => <AudioSettings /> },
      { id: "admin",      label: "Admin settings — detection thresholds",      w: 1440, h: 920,  render: () => <AdminSettings /> },
      { id: "quarantine", label: "Rare-bird quarantine — review queue",        w: 1440, h: 1000, render: () => <Quarantine /> },
      { id: "notifications", label: "Notifications center",                    w: 1440, h: 920,  render: () => <Notifications /> },
      { id: "migrate",    label: "Migrate from BirdNET-Pi · read-only import", w: 1440, h: 1280, render: () => <Migrate /> },
      { id: "backup",     label: "Backups, restore & system admin",            w: 1440, h: 1720, render: () => <BackupRecovery /> },
    ],
  },
  {
    id: "ambient", label: "Ambient", eyebrow: "25", title: "Kiosk mode",
    subtitle: "Wall-mounted display for a nature center, library, or your kitchen counter. Auto-rotates between four ambient stations.",
    frames: [
      { id: "kiosk", label: "Kiosk — auto-rotating wall display", w: 1920, h: 1080, render: () => <KioskMode /> },
    ],
  },
  {
    id: "mobile", label: "Mobile", eyebrow: "26", title: "Mobile",
    subtitle: "The same data, with the same vocabulary, on a phone.",
    frames: [
      { id: "mobile-showcase", label: "Mobile companion — three states", w: 1440, h: 820, render: (t) => <MobileShowcase demo={t.demo} /> },
    ],
  },
];

function App() {
  const [t, setTweak] = useTweaks(TWEAK_DEFAULTS);

  React.useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("theme-dark", t.theme === "dark");
    root.classList.toggle("theme-light", t.theme !== "dark");
    root.style.setProperty("--density", t.density === "compact" ? 0.78 : t.density === "comfy" ? 1.15 : 1);

    const accents = {
      moss:  { moss: 150, dawn: 60 },
      ocean: { moss: 220, dawn: 195 },
      heath: { moss: 305, dawn: 35  },
      ember: { moss: 25,  dawn: 60  },
    };
    const a = accents[t.accent] || accents.moss;
    const dark = t.theme === "dark";
    root.style.setProperty("--moss",      `oklch(${dark ? 78 : 55}% ${dark ? 0.18 : 0.09} ${a.moss})`);
    root.style.setProperty("--moss-soft", `oklch(${dark ? 26 : 92}% ${dark ? 0.10 : 0.04} ${a.moss})`);
    root.style.setProperty("--moss-ink",  `oklch(${dark ? 90 : 35}% ${dark ? 0.16 : 0.09} ${a.moss})`);
    root.style.setProperty("--dawn",      `oklch(${dark ? 82 : 68}% ${dark ? 0.18 : 0.12} ${a.dawn})`);
    root.style.setProperty("--dawn-soft", `oklch(${dark ? 28 : 94}% ${dark ? 0.10 : 0.05} ${a.dawn})`);
    root.style.setProperty("--dawn-ink",  `oklch(${dark ? 92 : 42}% ${dark ? 0.16 : 0.12} ${a.dawn})`);

    root.style.setProperty("--font-display", `"${t.displayFont}", Georgia, serif`);
  }, [t.theme, t.density, t.accent, t.displayFont]);

  return (
    <div className="page-root">
      <PageNav />
      <div className="page-shell">
        <TableOfContents />
        <main className="page-main">
          {SECTIONS.map((section) => (
            <PageSection key={section.id} {...section}>
              {section.frames.map((f) => (
                <Frame key={f.id} id={f.id} label={f.label} width={f.w} height={f.h}>
                  {f.render(t)}
                </Frame>
              ))}
            </PageSection>
          ))}
        </main>
      </div>

      <footer className="page-footer">
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", maxWidth: 1400, margin: "0 auto" }}>
          <span className="mono">BirdNet-Behavior · design exploration · {SECTIONS.reduce((n, s) => n + s.frames.length, 0)} screens</span>
          <span className="mono" style={{ opacity: 0.6 }}>v3 · light + dark · fully tweakable</span>
        </div>
      </footer>

      {/* Tweaks panel */}
      <TweaksPanel>
        <TweakSection label="Theme" />
        <TweakRadio
          label="Mode"
          value={t.theme}
          options={["light", "dark"]}
          onChange={(v) => setTweak("theme", v)}
        />
        <TweakSelect
          label="Accent"
          value={t.accent}
          options={["moss", "ocean", "heath", "ember"]}
          onChange={(v) => setTweak("accent", v)}
        />
        <TweakSelect
          label="Display font"
          value={t.displayFont}
          options={["Instrument Serif", "Source Serif 4", "Newsreader", "Inter Tight"]}
          onChange={(v) => setTweak("displayFont", v)}
        />

        <TweakSection label="Density" />
        <TweakRadio
          label="Spacing"
          value={t.density}
          options={["compact", "regular", "comfy"]}
          onChange={(v) => setTweak("density", v)}
        />

        <TweakSection label="Demo state" />
        <TweakRadio
          label="Activity"
          value={t.demo}
          options={["quiet", "busy", "dawn"]}
          onChange={(v) => setTweak("demo", v)}
        />
        <div style={{ fontSize: 11, color: "rgba(41,38,27,.55)", lineHeight: 1.45, marginTop: 2 }}>
          Controls live feed cadence and spectrogram density. <strong>Dawn</strong> = chorus peak.
        </div>
      </TweaksPanel>
    </div>
  );
}

// ─── Top nav (sticky, with section anchors) ─────────────────────────────
function PageNav() {
  return (
    <header className="page-nav">
      <div className="page-nav-inner">
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <BrandMark size={20} />
          <span style={{ fontWeight: 600, fontSize: 14, color: "var(--fg)" }}>BirdNet</span>
          <span style={{ fontSize: 14, color: "var(--fg-3)" }}>Behavior</span>
          <span className="bnb-pill mono" style={{ marginLeft: 10 }}>design exploration · {SECTIONS.reduce((n, s) => n + s.frames.length, 0)} screens · v3</span>
        </div>
        <nav style={{ display: "flex", gap: 2 }}>
          {SECTIONS.map((s) => (
            <a key={s.id} href={`#${s.id}`} className="page-nav-link">{s.label}</a>
          ))}
        </nav>
      </div>
    </header>
  );
}

// ─── Table of contents — sticky left rail ───────────────────────────────
const { useState: useState_app, useEffect: useEffect_app } = React;
function TableOfContents() {
  const [activeFrame, setActiveFrame] = useState_app(null);

  useEffect_app(() => {
    // Highlight which frame is currently in view.
    const frames = document.querySelectorAll("[data-frame-id]");
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries.filter((e) => e.isIntersecting);
        if (visible.length === 0) return;
        // pick the one closest to the top of viewport
        visible.sort((a, b) => Math.abs(a.boundingClientRect.top) - Math.abs(b.boundingClientRect.top));
        setActiveFrame(visible[0].target.getAttribute("data-frame-id"));
      },
      { rootMargin: "-30% 0% -60% 0%", threshold: [0, 0.25, 0.5, 0.75, 1] }
    );
    frames.forEach((f) => observer.observe(f));
    return () => observer.disconnect();
  }, []);

  return (
    <aside className="page-toc">
      <div className="page-toc-inner">
        <div className="bnb-eyebrow" style={{ marginBottom: 10 }}>Contents</div>
        {SECTIONS.map((s) => (
          <div key={s.id} className="page-toc-section">
            <a href={`#${s.id}`} className="page-toc-section-title">
              <span className="mono" style={{ color: "var(--fg-3)", fontSize: 9.5, letterSpacing: "0.08em", marginRight: 6 }}>{s.eyebrow}</span>
              {s.label}
            </a>
            <ul className="page-toc-frames">
              {s.frames.map((f) => (
                <li key={f.id}>
                  <a href={`#frame-${f.id}`} className={`page-toc-frame ${activeFrame === f.id ? "active" : ""}`}>
                    {f.label.split("—")[0].trim()}
                  </a>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
    </aside>
  );
}

// ─── Section ────────────────────────────────────────────────────────────
function PageSection({ id, eyebrow, title, subtitle, children }) {
  return (
    <section id={id} className="page-section">
      <div className="page-section-header">
        <div className="bnb-eyebrow mono" style={{ color: "var(--fg-3)" }}>{eyebrow}</div>
        <h2 className="display" style={{ fontSize: 40, lineHeight: 1.06, marginTop: 6, color: "var(--fg)", letterSpacing: "-0.02em" }}>{title}</h2>
        {subtitle && <p style={{ marginTop: 8, color: "var(--fg-2)", maxWidth: 760, fontSize: 15, lineHeight: 1.55 }}>{subtitle}</p>}
      </div>
      <div className="page-section-body">
        {children}
      </div>
    </section>
  );
}

// ─── Frame — labeled container scaled to fit available width ────────────
function Frame({ id, label, width, height, children }) {
  const wrapRef = React.useRef(null);
  const [scale, setScale] = React.useState(1);

  React.useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const ro = new ResizeObserver(() => {
      const w = wrap.clientWidth;
      setScale(Math.min(1, w / width));
    });
    ro.observe(wrap);
    return () => ro.disconnect();
  }, [width]);

  return (
    <figure id={`frame-${id}`} data-frame-id={id} className="frame">
      <figcaption className="frame-caption">
        <span className="frame-label">{label}</span>
        <span className="frame-dims mono">{width} × {height}</span>
      </figcaption>
      <div ref={wrapRef} className="frame-wrap" style={{ height: height * scale }}>
        <div className="frame-content" style={{
          width: width, height: height,
          transform: `scale(${scale})`, transformOrigin: "top left",
        }}>
          {children}
        </div>
      </div>
    </figure>
  );
}

ReactDOM.createRoot(document.getElementById("root")).render(<App />);
