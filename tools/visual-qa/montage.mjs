// Build a contact-sheet montage from PNG files so many screens can be eyeballed at once.
// Usage: node montage.mjs <shots_dir> <out.png> <filter_substring> [thumbW] [cols]
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';

const dir = process.argv[2] || 'shots';
const out = process.argv[3] || 'montage.png';
const filter = process.argv[4] || '';
const thumbW = parseInt(process.argv[5] || '460', 10);
const cols = parseInt(process.argv[6] || '4', 10);

const files = fs
  .readdirSync(dir)
  .filter((f) => f.endsWith('.png') && f.includes(filter))
  .sort();

if (!files.length) {
  console.error('no matching pngs');
  process.exit(1);
}

const cells = files
  .map((f) => {
    const abs = 'file://' + path.resolve(dir, f);
    return `<figure><img src="${abs}" /><figcaption>${f.replace('.png', '')}</figcaption></figure>`;
  })
  .join('\n');

const html = `<!doctype html><html><head><meta charset="utf-8"><style>
  body { margin:0; background:#1a1a1a; font-family: monospace; }
  .grid { display:grid; grid-template-columns: repeat(${cols}, ${thumbW}px); gap:14px; padding:14px; }
  figure { margin:0; background:#2a2a2a; border:1px solid #444; border-radius:6px; overflow:hidden; }
  img { width:${thumbW}px; height:auto; display:block; background:#fff; }
  figcaption { color:#ddd; font-size:13px; padding:6px 8px; word-break:break-all; }
</style></head><body><div class="grid">${cells}</div></body></html>`;

const tmp = path.resolve(dir, '.montage.html');
fs.writeFileSync(tmp, html);

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: cols * (thumbW + 14) + 28, height: 1000 }, deviceScaleFactor: 1 });
await page.goto('file://' + tmp, { waitUntil: 'networkidle' });
await page.waitForTimeout(800);
await page.screenshot({ path: out, fullPage: true });
await browser.close();
console.log(`montage of ${files.length} shots -> ${out}`);
