// Live spectrogram — boxes travel with the scrolling content so they line up
// with the chirp pixels. Theme-aware (light/dark) canvas.

const { useEffect: useEffect_sp, useRef: useRef_sp, useState: useState_sp } = React;

const SP_W = 240, SP_H = 80;

function Spectrogram({ demo = "busy" }) {
  const canvasRef = useRef_sp(null);
  const overlayRef = useRef_sp(null);
  const waveRef = useRef_sp(null);
  const histRef = useRef_sp(null);
  const [counter, setCounter] = useState_sp({ total: 0, species: new Set() });

  useEffect_sp(() => {
    const cnv = canvasRef.current;
    const ctx = cnv.getContext("2d");
    cnv.width = SP_W; cnv.height = SP_H;

    const getColors = () => {
      const dark = document.documentElement.classList.contains("theme-dark");
      return dark
        ? { bg: "#161513", noise: "rgba(255,235,200,", signalA: "oklch(80% 0.16 150 / ", signalB: "oklch(88% 0.18 90 / " }
        : { bg: "#f5f3ee", noise: "rgba(60,50,30,",     signalA: "oklch(42% 0.10 150 / ", signalB: "oklch(48% 0.12 70 / " };
    };

    let colors = getColors();
    ctx.fillStyle = colors.bg;
    ctx.fillRect(0, 0, SP_W, SP_H);

    const chirps = [];   // { t, dur, fStart, fEnd, amp, box, signal }
    const boxes  = [];   // { el, lblEl, age, width, right, sp }
    let raf, themeWatchTick = 0;

    function spawnBox(sp, conf, rare, peakF) {
      const overlay = overlayRef.current;
      if (!overlay) return null;
      const stroke = rare ? "var(--rare)" : sp.color;
      const el = document.createElement("div");
      el.style.cssText = `position:absolute; box-sizing:border-box; border:1.25px solid ${stroke}; border-radius:3px;
                          pointer-events:none; height:22%; top:${Math.max(2, Math.min(76, (peakF / SP_H) * 100 - 11))}%;
                          background:linear-gradient(180deg, ${stroke.replace(")", " / .08)").replace("oklch(", "oklch(")}, transparent);`;
      const lblEl = document.createElement("span");
      lblEl.className = "mono";
      lblEl.style.cssText = `position:absolute; left:-1px; top:-18px; padding:1px 6px; font-size:10px; line-height:1.4;
                             border-radius:3px 3px 3px 0; background:${stroke}; color:var(--bg); white-space:nowrap;
                             font-weight:500; letter-spacing:.02em;`;
      lblEl.textContent = `${sp.short} · ${conf.toFixed(2)}${rare ? " · rare" : ""}`;
      el.appendChild(lblEl);
      overlay.appendChild(el);
      return { el, lblEl, age: 0, width: 0, right: 0, sp };
    }

    function sampleSpecies() {
      const { SPECIES } = window.BNB;
      const pool = demo === "dawn"
        ? [1, 3, 0, 2, 14, 5, 6]
        : demo === "quiet"
          ? [3, 6, 5]
          : [0, 1, 2, 3, 5, 6, 9, 12];
      const idx = pool[Math.floor(Math.random() * pool.length)];
      const sp = SPECIES[idx];
      return { idx, sp, conf: 0.78 + Math.random() * 0.20, rare: sp.rare };
    }

    function tick() {
      // Re-read theme every ~30 frames
      themeWatchTick++;
      if (themeWatchTick % 30 === 0) colors = getColors();

      // Shift the canvas left by 1 column
      const prev = ctx.getImageData(1, 0, SP_W - 1, SP_H);
      ctx.putImageData(prev, 0, 0);
      ctx.fillStyle = colors.bg;
      ctx.fillRect(SP_W - 1, 0, 1, SP_H);

      // Faint background noise
      for (let y = 0; y < SP_H; y++) {
        const n = Math.random() * 0.10;
        if (n > 0.02) {
          ctx.fillStyle = colors.noise + n.toFixed(2) + ")";
          ctx.fillRect(SP_W - 1, y, 1, 1);
        }
      }

      // Schedule chirps
      const chirpRate = demo === "dawn" ? 0.040 : demo === "quiet" ? 0.008 : 0.022;
      if (Math.random() < chirpRate) {
        const samp = sampleSpecies();
        const fStart = 14 + Math.random() * 32;
        const fEnd = fStart + (Math.random() * 22 - 8);
        const dur = 18 + Math.floor(Math.random() * 30);
        const peakF = (fStart + fEnd) / 2;
        const box = spawnBox(samp.sp, samp.conf, samp.rare, peakF);
        if (box) {
          boxes.push(box);
          chirps.push({ t: 0, dur, fStart, fEnd, amp: 0.65 + Math.random() * 0.35, box, signal: samp.rare ? colors.signalB : colors.signalA });
          setCounter((c) => ({ total: c.total + 1, species: new Set([...c.species, samp.sp.short]) }));
        }
      }

      // Render in-flight chirps + grow boxes
      for (let i = chirps.length - 1; i >= 0; i--) {
        const c = chirps[i];
        if (c.t < c.dur) {
          const u = c.t / c.dur;
          const f = c.fStart * (1 - u) + c.fEnd * u;
          for (let yy = -2; yy <= 2; yy++) {
            const y = Math.round(f + yy);
            if (y < 0 || y >= SP_H) continue;
            const fall = Math.exp(-(yy * yy) / 1.4);
            const a = c.amp * fall * (1 - Math.abs(0.5 - u) * 0.7);
            if (a > 0.04) {
              ctx.fillStyle = c.signal + a.toFixed(2) + ")";
              ctx.fillRect(SP_W - 1, y, 1, 1);
            }
          }
          c.box.width += 1; // grow by one column
          c.t++;
        } else {
          chirps.splice(i, 1);
        }
      }

      // Boxes that aren't still chirping drift left (right edge advances away from "now")
      for (const c of chirps) c.box.__active = true;
      for (let i = boxes.length - 1; i >= 0; i--) {
        const b = boxes[i];
        if (!b.__active) b.right += 1;
        b.__active = false;
        const leftPct = ((SP_W - 1 - b.right - b.width) / SP_W) * 100;
        const widthPct = (b.width / SP_W) * 100;
        b.el.style.left = leftPct.toFixed(2) + "%";
        b.el.style.width = widthPct.toFixed(2) + "%";
        if ((b.right + b.width) > SP_W + 4) {
          b.el.remove();
          boxes.splice(i, 1);
        }
      }

      raf = requestAnimationFrame(tick);
    }
    tick();
    return () => { cancelAnimationFrame(raf); boxes.forEach((b) => b.el.remove()); };
  }, [demo]);

  // Waveform
  useEffect_sp(() => {
    const cnv = waveRef.current;
    if (!cnv) return;
    const ctx = cnv.getContext("2d");
    cnv.width = SP_W; cnv.height = 44;
    let phase = 0, raf;
    function tick() {
      const dark = document.documentElement.classList.contains("theme-dark");
      ctx.fillStyle = dark ? "#1c1a17" : "#fbf9f4";
      ctx.fillRect(0, 0, SP_W, 44);
      ctx.beginPath();
      for (let x = 0; x < SP_W; x++) {
        const f1 = Math.sin(x * 0.12 + phase) * 0.6;
        const f2 = Math.sin(x * 0.06 + phase * 0.7) * 0.25;
        const env = 0.55 + 0.40 * Math.sin(x * 0.02 + phase * 0.3);
        const y = 22 + (f1 + f2) * 16 * env * (0.5 + Math.random() * 0.5);
        if (x === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
      }
      ctx.strokeStyle = dark ? "oklch(80% 0.14 150 / .9)" : "oklch(45% 0.10 150 / .85)";
      ctx.lineWidth = 1;
      ctx.stroke();
      phase += 0.18;
      raf = requestAnimationFrame(tick);
    }
    tick();
    return () => cancelAnimationFrame(raf);
  }, []);

  // Per-species histogram in side panel
  const { SPECIES } = window.BNB;
  const species = [...counter.species];

  return (
    <Screen>
      <TopNav active="Analytics" />

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", gap: 20 }}>
        <div>
          <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Live · what the Pi hears</div>
          <h2 className="display" style={{ fontSize: 30, lineHeight: 1.1 }}>The 30-second window</h2>
          <div className="bnb-meta" style={{ marginTop: 6, maxWidth: 540 }}>
            Streaming spectrogram from the microphone. Every classification with confidence ≥ 0.78 gets a labeled box that travels with its sound.
          </div>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <span className="bnb-pill moss"><span className="bnb-dot live" /> streaming</span>
          <span className="bnb-pill mono">48 kHz</span>
          <span className="bnb-pill mono">FFT 1024 · hann</span>
          <span className="bnb-pill mono">ONNX · 48 ms</span>
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1fr 280px", gap: "var(--pad-3)", flex: 1, minHeight: 0 }}>
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 10, minHeight: 0 }}>
          {/* Spectrogram + freq axis */}
          <div style={{ display: "flex", gap: 8, alignItems: "stretch", flex: 1, minHeight: 0 }}>
            <div style={{ display: "flex", flexDirection: "column", justifyContent: "space-between", paddingTop: 4, paddingBottom: 4, width: 36, position: "relative" }}>
              {[12, 10, 8, 6, 4, 2, 0].map((khz) => (
                <div key={khz} style={{ display: "flex", alignItems: "center", gap: 4, justifyContent: "flex-end" }}>
                  <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>{khz}</span>
                  <span style={{ width: 4, height: 1, background: "var(--border)" }} />
                </div>
              ))}
              <span className="mono" style={{ position: "absolute", left: -22, top: "50%", transform: "rotate(-90deg) translateX(50%)", transformOrigin: "0 0", fontSize: 9.5, color: "var(--fg-4)", letterSpacing: "0.08em", textTransform: "uppercase" }}>kHz</span>
            </div>
            <div style={{ position: "relative", flex: 1, minHeight: 0, borderRadius: 8, overflow: "hidden", border: "0.5px solid var(--border)" }}>
              <canvas
                ref={canvasRef}
                style={{ width: "100%", height: "100%", imageRendering: "pixelated", display: "block", background: "var(--surface-2)" }}
              />
              <div ref={overlayRef} style={{ position: "absolute", inset: 0, pointerEvents: "none" }} />
              {/* now hairline */}
              <div style={{ position: "absolute", right: 0, top: 0, bottom: 0, width: 2, background: "linear-gradient(180deg, transparent, var(--moss), transparent)", boxShadow: "0 0 8px var(--moss)" }} />
              <span style={{ position: "absolute", right: 8, top: 8, padding: "2px 6px", borderRadius: 4, background: "var(--surface)", border: "0.5px solid var(--border)", fontSize: 10, fontFamily: "var(--font-mono)", color: "var(--fg-3)" }}>now</span>
            </div>
          </div>

          {/* Waveform */}
          <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)", width: 32, textAlign: "right" }}>lvl</span>
            <canvas ref={waveRef} style={{ width: "100%", height: 44, background: "var(--surface-2)", borderRadius: 6, border: "0.5px solid var(--border)" }} />
          </div>

          {/* Time axis */}
          <div style={{ display: "flex", justifyContent: "space-between", paddingLeft: 40 }}>
            {["−30s", "−24s", "−18s", "−12s", "−6s", "now"].map((l) => (
              <span key={l} className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>{l}</span>
            ))}
          </div>
        </div>

        {/* Side panel — live tally */}
        <div className="bnb-card" style={{ padding: "var(--pad-3)", display: "flex", flexDirection: "column", gap: 12 }}>
          <SectionHeader eyebrow="This window" title="Live tally" />
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
            <Stat label="Detections" value={counter.total} sub="last 30 s" size="sm" />
            <Stat label="Species" value={species.length} sub={species.length === 1 ? "1 distinct" : `${species.length} distinct`} size="sm" accent="var(--moss-ink)" />
          </div>

          <div>
            <div className="bnb-eyebrow" style={{ marginBottom: 8 }}>Heard so far</div>
            <div style={{ display: "flex", flexDirection: "column", gap: 6, maxHeight: 200, overflow: "hidden" }}>
              {species.length === 0 && (
                <div className="bnb-meta" style={{ fontStyle: "italic" }}>Listening — no chirps yet…</div>
              )}
              {species.map((shortCode) => {
                const sp = SPECIES.find((x) => x.short === shortCode);
                if (!sp) return null;
                return (
                  <div key={shortCode} style={{ display: "grid", gridTemplateColumns: "auto 1fr auto", gap: 8, alignItems: "center" }}>
                    <span style={{ width: 8, height: 8, borderRadius: 999, background: sp.color }} />
                    <span style={{ fontSize: 12.5 }}>{sp.common}</span>
                    <span className="mono" style={{ fontSize: 11, color: "var(--fg-3)" }}>{shortCode}</span>
                  </div>
                );
              })}
            </div>
          </div>

          <div style={{ marginTop: "auto", borderTop: "0.5px solid var(--hairline)", paddingTop: 10 }}>
            <div className="bnb-eyebrow" style={{ marginBottom: 6 }}>Quality filter</div>
            <div style={{ fontSize: 12, color: "var(--fg-2)", lineHeight: 1.5 }}>
              SNR <span className="mono">14.2 dB</span> · spectral flatness <span className="mono">0.31</span> · rain detector <span className="mono">off</span>.
            </div>
            <div className="bnb-meta" style={{ marginTop: 6 }}>Letting through 94% of inference candidates.</div>
          </div>

          <button className="bnb-btn primary" style={{ justifyContent: "center" }}>Open kiosk mode →</button>
        </div>
      </div>
    </Screen>
  );
}

Object.assign(window, { Spectrogram });
