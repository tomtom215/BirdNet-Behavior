import { chromium } from 'playwright';
const BASE = 'http://127.0.0.1:8502';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1440, height: 900 } });
const page = await ctx.newPage();
const failedReqs = [];
page.on('requestfailed', (r) => failedReqs.push(r.url() + ' :: ' + r.failure()?.errorText));
const status = {};
page.on('response', (r) => {
  const u = r.url();
  if (u.includes('/static/fonts/') || u.endsWith('.woff2') || u.includes('app.css')) status[u] = r.status();
});
const consoleErrs = [];
page.on('console', (m) => { if (m.type() === 'error') consoleErrs.push(m.text()); });

await page.goto(BASE + '/', { waitUntil: 'networkidle' });
await page.waitForTimeout(1500);

const fontInfo = await page.evaluate(async () => {
  await document.fonts.ready;
  const families = ['Instrument Serif', 'Inter Tight', 'JetBrains Mono'];
  const checks = {};
  for (const f of families) {
    checks[f] = {
      check12: document.fonts.check(`12px "${f}"`),
      check40: document.fonts.check(`40px "${f}"`),
    };
  }
  // loaded faces
  const loaded = [];
  document.fonts.forEach((ff) => { if (ff.status === 'loaded') loaded.push(`${ff.family} ${ff.style} ${ff.weight}`); });
  // computed family on the hero headline
  const h = document.querySelector('h1, .display, [class*="display"], .hero-title');
  const headlineFamily = h ? getComputedStyle(h).fontFamily : '(no headline found)';
  const headlineText = h ? h.textContent.trim().slice(0, 40) : '';
  // body + mono samples
  const body = getComputedStyle(document.body).fontFamily;
  return { checks, loaded, headlineFamily, headlineText, body };
});

console.log('=== font.check ===');
console.log(JSON.stringify(fontInfo.checks, null, 2));
console.log('=== loaded faces ===');
console.log(fontInfo.loaded.join('\n'));
console.log('=== headline ===', fontInfo.headlineText, '\n  family:', fontInfo.headlineFamily);
console.log('=== body family ===', fontInfo.body);
console.log('=== font/css responses ===');
console.log(JSON.stringify(status, null, 2));
console.log('=== failed requests ===');
console.log(failedReqs.join('\n') || '(none)');
console.log('=== console errors ===');
console.log(consoleErrs.join('\n') || '(none)');
await b.close();
