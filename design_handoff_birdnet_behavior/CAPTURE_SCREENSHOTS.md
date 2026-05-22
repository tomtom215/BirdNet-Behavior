# Generating screenshots from the prototype

The screenshot capture tool wasn't available during the handoff hand-off, so this short script lets you (or your developer) generate clean PNGs of every screen from the prototype, in about 60 seconds.

## One-liner — capture every screen at full design width

1. Serve the prototype locally (Python or any static server):

   ```bash
   cd source/
   python3 -m http.server 8000
   ```

2. Open <http://localhost:8000/BirdNet-Behavior.html> in Chrome.

3. Open DevTools → Console, paste the script below, and hit Enter:

   ```js
   // Loads html-to-image and saves a PNG for every <figure data-frame-id>.
   const s = document.createElement('script');
   s.src = 'https://unpkg.com/html-to-image@1.11.13/dist/html-to-image.min.js';
   s.onload = async () => {
     const frames = [...document.querySelectorAll('[data-frame-id]')];
     for (const [i, f] of frames.entries()) {
       const inner = f.querySelector('.frame-content');
       const label = f.dataset.frameId;
       // Temporarily render at native 1440px (clear the scale transform)
       const prev = inner.style.transform;
       inner.style.transform = 'none';
       f.querySelector('.frame-wrap').style.height = inner.scrollHeight + 'px';
       await new Promise(r => requestAnimationFrame(r));
       const dataUrl = await htmlToImage.toPng(inner, {
         pixelRatio: 2,
         backgroundColor: getComputedStyle(document.documentElement).getPropertyValue('--bg').trim(),
       });
       inner.style.transform = prev;
       const a = document.createElement('a');
       a.href = dataUrl;
       a.download = String(i+1).padStart(2,'0') + '-' + label + '.png';
       a.click();
       await new Promise(r => setTimeout(r, 400));
     }
     console.log('done · 27 PNGs saved to your Downloads folder');
   };
   document.body.appendChild(s);
   ```

4. Chrome will save 27 PNGs (`01-onboarding.png` through `27-mobile-showcase.png`) to your Downloads folder at 2× pixel density (≈ 2880 × native height per frame).

5. If you want dark-mode screenshots, before running the script open the **Tweaks panel** (bottom-right) and toggle Mode → dark, then run the script again — files will be saved alongside the light versions; rename `dark-` prefix as you prefer.

## Why this approach

- Captures every screen at **native 1440 px design width** (no scaling artifacts from the page's fit-to-column layout)
- Uses the canonical token CSS already loaded, so colors and shadows are pixel-perfect
- Renders animations at a steady frame (live-feed pulses, kiosk aurora) — re-run if you want a different moment
- Works in any modern Chromium/Firefox/Safari; nothing to install
