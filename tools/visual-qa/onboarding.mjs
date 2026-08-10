// Capture every onboarding step by clicking through, in both themes.
//
// The step count is read from the page rather than hard-coded: this script used
// to say `step <= 5`, and silently stopped capturing the last step the moment a
// sixth (Accuracy) was added — the failure mode being a screenshot set that
// looks complete while missing exactly the new thing worth reviewing.
import { chromium } from 'playwright';
const BASE = 'http://127.0.0.1:8502';
const b = await chromium.launch();
for (const theme of ['light', 'dark']) {
  const ctx = await b.newContext({ viewport: { width: 1100, height: 820 }, colorScheme: theme });
  await ctx.addInitScript((t) => { try { localStorage.setItem('theme', t); } catch (e) {} }, theme);
  const p = await ctx.newPage();
  await p.goto(BASE + '/onboarding', { waitUntil: 'networkidle' });
  const total = await p.$$eval('section.ob-step', (n) => n.length);
  if (!total) throw new Error('no onboarding steps found — did the markup change?');
  for (let step = 1; step <= total; step++) {
    await p.waitForTimeout(500);
    await p.screenshot({ path: `shots/onboarding-step${step}__${theme}__desktop.png` });
    if (step < total) await p.click('#ob-next');
  }
  await ctx.close();
  console.log(`captured onboarding (${total} steps)`, theme);
}
await b.close();
