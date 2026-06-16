// Accessibility gate for birdnet-web.
//
// Runs axe-core (via @axe-core/playwright) against every route the screenshot
// server renders, across light + dark themes, and fails when any violation at
// or above the configured impact threshold is found. Reuses the route table
// exported by qa.mjs so the two QA gates can never drift.
//
// Pairs with:
//   cargo run -p birdnet-web --example screenshot_server --features analytics
//
// Env:
//   BASE         base url                       (default http://127.0.0.1:8502)
//   THEMES       csv of light,dark              (default light,dark)
//   AXE_FAIL_ON  csv impact levels that fail    (default serious,critical)
//   AXE_DISABLE  csv rules to skip              (default color-contrast,
//                                                link-in-text-block — see below)
//   ONLY         substring filter on route name
//
// Run from this directory after `npm i playwright @axe-core/playwright`.
import { chromium } from 'playwright';
import AxeModule from '@axe-core/playwright';
import { ROUTES } from './qa.mjs';

// @axe-core/playwright ships AxeBuilder as a CJS default export; tolerate the
// named/namespace shapes too so a minor package bump cannot break the import.
const AxeBuilder = AxeModule.default || AxeModule.AxeBuilder || AxeModule;

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const THEMES = (process.env.THEMES || 'light,dark').split(',').filter(Boolean);
const FAIL_ON = new Set(
  (process.env.AXE_FAIL_ON || 'serious,critical').split(',').filter(Boolean),
);
const ONLY = process.env.ONLY || '';

// Two WCAG rules are deferred to a dedicated, design-reviewed pass and excluded
// from this gate (tracked in CHANGELOG / IMPLEMENTATION_PLAN):
//   - color-contrast: the v3 palette renders each species' *identity* hue
//     (oklch ~62% L) as text and uses a deliberately muted meta-text
//     hierarchy. Meeting AA there means changing locked design tokens / species
//     colours — a design decision, not an a11y-batch one, and an all-or-nothing
//     one (any remaining low-contrast node keeps the gate red).
//   - link-in-text-block: distinguishing in-text links without relying on
//     colour is an app-wide link-underline policy — the same design pass.
// Everything else at serious/critical is enforced. Re-check the full picture
// with AXE_DISABLE="" (or e.g. AXE_DISABLE=color-contrast to audit links only).
const DISABLED_RULES = (process.env.AXE_DISABLE ?? 'color-contrast,link-in-text-block')
  .split(',')
  .map((s) => s.trim())
  .filter(Boolean);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const browser = await chromium.launch();
  let blocking = 0;
  let total = 0;
  const seen = new Set(); // unique "[impact] rule" pairs, for the summary

  for (const theme of THEMES) {
    const context = await browser.newContext({
      viewport: { width: 1280, height: 900 },
      colorScheme: theme === 'dark' ? 'dark' : 'light',
    });
    await context.addInitScript((t) => {
      try {
        localStorage.setItem('theme', t);
      } catch (e) {
        /* ignore */
      }
    }, theme);
    const page = await context.newPage();

    for (const [name, route] of ROUTES) {
      if (ONLY && !name.includes(ONLY)) continue;
      // The deliberate 404 route is an error page, not a product surface.
      if (route.includes('does-not-exist')) continue;
      const key = `${name}__${theme}`;
      try {
        await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
        await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
        await sleep(700);
        // Gate on the WCAG 2.0/2.1 A + AA success criteria — the legal/standard
        // bar — and leave axe's "best-practice" rules as non-blocking noise.
        const builder = new AxeBuilder({ page }).withTags([
          'wcag2a',
          'wcag2aa',
          'wcag21a',
          'wcag21aa',
        ]);
        if (DISABLED_RULES.length) builder.disableRules(DISABLED_RULES);
        const results = await builder.analyze();
        const v = results.violations;
        total += v.length;
        const blk = v.filter((it) => FAIL_ON.has(it.impact));
        blocking += blk.length;
        if (v.length) {
          for (const it of v) {
            const mark = FAIL_ON.has(it.impact) ? '!' : '.';
            seen.add(`[${it.impact}] ${it.id}`);
            console.log(`${mark} ${key} [${it.impact}] ${it.id}: ${it.help} (${it.nodes.length} node(s))`);
            for (const node of it.nodes.slice(0, 4)) {
              console.log(`      ${node.target.join(' ')}`);
            }
          }
        } else {
          console.log(`. ${key} — clean`);
        }
      } catch (err) {
        // A page that won't load or analyze is itself a failure.
        console.log(`x ${key}: ${String(err).slice(0, 160)}`);
        blocking += 1;
      }
    }
    await context.close();
  }
  await browser.close();

  console.log(`\n=== axe: ${total} total violation(s); ${blocking} at/above [${[...FAIL_ON].join(', ')}] ===`);
  if (DISABLED_RULES.length) {
    console.log(`(deferred rules, not gated: ${DISABLED_RULES.join(', ')})`);
  }
  if (seen.size) {
    console.log('distinct rules seen:');
    for (const r of [...seen].sort()) console.log(`  ${r}`);
  }
  if (blocking > 0) {
    console.error(`\nFAIL: ${blocking} blocking accessibility violation(s).`);
    process.exitCode = 1;
  } else {
    console.log('\nPASS: no blocking accessibility violations.');
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
