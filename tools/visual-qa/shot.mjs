// Capture one route in light/dark x desktop/mobile.
// Usage: node shot.mjs <route> <out_dir> <name>
import { chromium } from 'playwright';
const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const route = process.argv[2];
const out = process.argv[3];
const name = process.argv[4] || 'shot';
const VPS = [['desktop', 1440, 900], ['mobile', 390, 844]];
const b = await chromium.launch();
for (const [vn, w, h] of VPS) {
  for (const theme of ['light', 'dark']) {
    const ctx = await b.newContext({ viewport: { width: w, height: h }, colorScheme: theme });
    await ctx.addInitScript((t) => {
      try { localStorage.setItem('theme', t); localStorage.setItem('bnb-density', 'regular'); } catch (e) {}
    }, theme);
    const p = await ctx.newPage();
    await p.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 20000 });
    await p.waitForLoadState('networkidle', { timeout: 8000 }).catch(() => {});
    await new Promise((r) => setTimeout(r, 1100));
    await p.screenshot({ path: `${out}/${name}__${theme}__${vn}.png`, fullPage: true });
    console.log(`. ${name} ${theme}/${vn}`);
    await ctx.close();
  }
}
await b.close();
