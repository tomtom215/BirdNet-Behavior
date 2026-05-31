// Quantitative pixel-diff between two PNG dirs of identically-named shots.
// Usage: node diff_pair.mjs <before_dir> <after_dir>
// Exit 0 if all pairs are pixel-identical, else 1.
// The RGBA compare runs *inside* the browser (fast typed-array path); only a
// small summary crosses back to node.
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';

const beforeDir = process.argv[2];
const afterDir = process.argv[3];
if (!beforeDir || !afterDir) {
  console.error('usage: node diff_pair.mjs <before_dir> <after_dir>');
  process.exit(2);
}

const files = fs
  .readdirSync(beforeDir)
  .filter((f) => f.endsWith('.png') && fs.existsSync(path.join(afterDir, f)))
  .sort();
if (!files.length) {
  console.error('no common PNG pairs');
  process.exit(2);
}

const browser = await chromium.launch();
const page = await browser.newPage();

async function summarise(beforeAbs, afterAbs) {
  const b1 = fs.readFileSync(beforeAbs).toString('base64');
  const b2 = fs.readFileSync(afterAbs).toString('base64');
  return page.evaluate(async ([d1, d2]) => {
    async function decode(dataUrl) {
      const img = new Image();
      img.src = dataUrl;
      await img.decode();
      const c = new OffscreenCanvas(img.naturalWidth, img.naturalHeight);
      const ctx = c.getContext('2d');
      ctx.drawImage(img, 0, 0);
      return { w: c.width, h: c.height, data: ctx.getImageData(0, 0, c.width, c.height).data };
    }
    const a = await decode('data:image/png;base64,' + d1);
    const b = await decode('data:image/png;base64,' + d2);
    if (a.w !== b.w || a.h !== b.h) {
      return { sizeMismatch: true, aw: a.w, ah: a.h, bw: b.w, bh: b.h };
    }
    let diff = 0, maxCh = 0;
    const da = a.data, db = b.data, n = da.length;
    for (let i = 0; i < n; i += 4) {
      const r = Math.abs(da[i] - db[i]);
      const g = Math.abs(da[i + 1] - db[i + 1]);
      const bl = Math.abs(da[i + 2] - db[i + 2]);
      const al = Math.abs(da[i + 3] - db[i + 3]);
      const m = r > g ? r : g; const m2 = bl > al ? bl : al; const mm = m > m2 ? m : m2;
      if (mm) { diff++; if (mm > maxCh) maxCh = mm; }
    }
    return { sizeMismatch: false, w: a.w, h: a.h, diff, maxCh, total: a.w * a.h };
  }, [b1, b2]);
}

let anyDiff = false;
const rows = [];
for (const f of files) {
  const s = await summarise(path.join(beforeDir, f), path.join(afterDir, f));
  if (s.sizeMismatch) {
    rows.push(`DIFF  ${f}  size ${s.aw}x${s.ah} -> ${s.bw}x${s.bh}`);
    anyDiff = true;
  } else if (s.diff === 0) {
    rows.push(`ok    ${f}  0 px  (${s.w}x${s.h})`);
  } else {
    const pct = ((s.diff / s.total) * 100).toFixed(4);
    rows.push(`DIFF  ${f}  ${s.diff}/${s.total} px (${pct}%) maxChannelΔ=${s.maxCh}`);
    anyDiff = true;
  }
}

await browser.close();
console.log(rows.join('\n'));
console.log(anyDiff ? '\nRESULT: differences found' : '\nRESULT: all pairs pixel-identical');
process.exit(anyDiff ? 1 : 0);
