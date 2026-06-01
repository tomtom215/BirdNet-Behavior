// Visual-regression runner for the P3-3 sweep.
//
//   node vqa.mjs snap <label> <route> [<route> ...]
//       Capture each route (light/dark x desktop/mobile) into shots/<label>/.
//   node vqa.mjs diff <before_label> <after_label>
//       Quantitative RGBA pixel diff of every common shot. Reports per-shot
//       differing-pixel counts, splitting "content" from "chrome" (the shared
//       topnav live-status dot + the footer uptime ticker animate/tick between
//       captures, so they are reported separately and don't fail the run).
//       Exits non-zero iff any CONTENT pixels differ.
//
// Routes are given as name=path pairs, e.g.  life-list=/life-list
// The RGBA compare runs inside the headless browser for speed.
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const ROOT = process.env.VQA_ROOT || '/tmp/vqa';
const VPS = [['desktop', 1440, 900], ['mobile', 390, 844]];
const THEMES = ['light', 'dark'];

// Chrome bands (px from top / bottom) that legitimately animate or tick and so
// are excluded from the content-diff gate: the topnav (live-status pulse dot)
// and the footer (process-uptime ticker).
const TOP_CHROME = 170;     // topnav lives above y=170
const BOTTOM_CHROME = 80;   // footer lives in the last ~80px

const sub = process.argv[2];

async function snap(label, routes) {
  const out = path.join(ROOT, label);
  fs.mkdirSync(out, { recursive: true });
  const browser = await chromium.launch();
  for (const [vn, w, h] of VPS) {
    for (const theme of THEMES) {
      const ctx = await browser.newContext({ viewport: { width: w, height: h }, colorScheme: theme });
      await ctx.addInitScript((t) => {
        try { localStorage.setItem('theme', t); localStorage.setItem('bnb-density', 'regular'); } catch (e) {}
      }, theme);
      const page = await ctx.newPage();
      for (const [name, route] of routes) {
        await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 20000 });
        await page.waitForLoadState('networkidle', { timeout: 8000 }).catch(() => {});
        await new Promise((r) => setTimeout(r, 1100));
        await page.screenshot({ path: path.join(out, `${name}__${theme}__${vn}.png`), fullPage: true });
        process.stdout.write(`. ${name} ${theme}/${vn}\n`);
      }
      await ctx.close();
    }
  }
  await browser.close();
}

async function diff(beforeLabel, afterLabel) {
  const bDir = path.join(ROOT, beforeLabel);
  const aDir = path.join(ROOT, afterLabel);
  const files = fs.readdirSync(bDir).filter((f) => f.endsWith('.png') && fs.existsSync(path.join(aDir, f))).sort();
  if (!files.length) { console.error('no common shots'); process.exit(2); }

  const browser = await chromium.launch();
  const page = await browser.newPage();
  let contentBad = 0;
  const rows = [];

  for (const f of files) {
    const b1 = fs.readFileSync(path.join(bDir, f)).toString('base64');
    const b2 = fs.readFileSync(path.join(aDir, f)).toString('base64');
    const s = await page.evaluate(async ([d1, d2, topC, botC]) => {
      async function dec(u) {
        const img = new Image(); img.src = 'data:image/png;base64,' + u; await img.decode();
        const c = new OffscreenCanvas(img.naturalWidth, img.naturalHeight);
        const x = c.getContext('2d'); x.drawImage(img, 0, 0);
        return { w: c.width, h: c.height, d: x.getImageData(0, 0, c.width, c.height).data };
      }
      const A = await dec(d1), B = await dec(d2);
      if (A.w !== B.w || A.h !== B.h) return { mismatch: [A.w, A.h, B.w, B.h] };
      let content = 0, chrome = 0, maxCh = 0;
      const n = A.d.length, W = A.w, H = A.h;
      for (let i = 0; i < n; i += 4) {
        const m = Math.max(Math.abs(A.d[i]-B.d[i]), Math.abs(A.d[i+1]-B.d[i+1]), Math.abs(A.d[i+2]-B.d[i+2]), Math.abs(A.d[i+3]-B.d[i+3]));
        if (!m) continue;
        const py = Math.floor((i / 4) / W);
        if (py < topC || py >= H - botC) chrome++;
        else { content++; if (m > maxCh) maxCh = m; }
      }
      return { content, chrome, maxCh, total: W * H };
    }, [b1, b2, TOP_CHROME, BOTTOM_CHROME]);

    if (s.mismatch) {
      rows.push(`SIZE  ${f}  ${s.mismatch[0]}x${s.mismatch[1]} -> ${s.mismatch[2]}x${s.mismatch[3]}`);
      contentBad++;
    } else if (s.content === 0) {
      rows.push(`ok    ${f}  content 0 px  (chrome ${s.chrome})`);
    } else {
      rows.push(`FAIL  ${f}  content ${s.content} px (maxΔ ${s.maxCh})  (chrome ${s.chrome})`);
      contentBad++;
    }
  }
  await browser.close();
  console.log(rows.join('\n'));
  console.log(contentBad ? `\nRESULT: ${contentBad} shot(s) with content differences` : '\nRESULT: all shots content-identical');
  process.exit(contentBad ? 1 : 0);
}

const routesFromArgs = (args) => args.map((a) => {
  const i = a.indexOf('=');
  return i === -1 ? [a.replace(/\W+/g, '-'), a] : [a.slice(0, i), a.slice(i + 1)];
});

if (sub === 'snap') {
  await snap(process.argv[3], routesFromArgs(process.argv.slice(4)));
} else if (sub === 'diff') {
  await diff(process.argv[3], process.argv[4]);
} else {
  console.error('usage:\n  node vqa.mjs snap <label> name=/route [name=/route ...]\n  node vqa.mjs diff <before_label> <after_label>');
  process.exit(2);
}
