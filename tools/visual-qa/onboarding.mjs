// Capture all 5 onboarding steps by clicking through, in both themes.
import { chromium } from 'playwright';
const BASE = 'http://127.0.0.1:8502';
const b = await chromium.launch();
for (const theme of ['light', 'dark']) {
  const ctx = await b.newContext({ viewport: { width: 1100, height: 820 }, colorScheme: theme });
  await ctx.addInitScript((t) => { try { localStorage.setItem('theme', t); } catch (e) {} }, theme);
  const p = await ctx.newPage();
  await p.goto(BASE + '/onboarding', { waitUntil: 'networkidle' });
  for (let step = 1; step <= 5; step++) {
    await p.waitForTimeout(500);
    await p.screenshot({ path: `shots/onboarding-step${step}__${theme}__desktop.png` });
    if (step < 5) await p.click('#ob-next');
  }
  await ctx.close();
  console.log('captured onboarding', theme);
}
await b.close();
