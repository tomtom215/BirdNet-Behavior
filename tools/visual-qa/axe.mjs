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
//   AXE_DISABLE  csv rules to skip              (default link-in-text-block)
//   AXE_ADVISORY_BLOCKS  "0" demotes the WCAG 2.2 / best-practice tier
//   VPS          csv of desktop,mobile          (default desktop,mobile)
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
// Explicit browser binary, for sandboxes that ship their own Chromium rather
// than the build this playwright pinned (same knob as interactions.mjs).
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const THEMES = (process.env.THEMES || 'light,dark').split(',').filter(Boolean);
const FAIL_ON = new Set(
  (process.env.AXE_FAIL_ON || 'serious,critical').split(',').filter(Boolean),
);
const ONLY = process.env.ONLY || '';

// One WCAG rule is deferred to a design pass and excluded from this gate:
//   - link-in-text-block: distinguishing in-text links without relying on
//     colour is an app-wide link-underline policy.
// color-contrast is enforced (DD-29): the species avatar mixes its identity
// hue towards an ink token, the muted text tokens sit at AA on every tinted
// surface, and the bright fills carry --on-fill. It is all-or-nothing — any
// low-contrast node keeps the gate red — which is the point.
// Everything else at serious/critical is enforced. Re-check the full picture
// with AXE_DISABLE="".
const DISABLED_RULES = (process.env.AXE_DISABLE ?? 'link-in-text-block')
  .split(',')
  .map((s) => s.trim())
  .filter(Boolean);

// The tag filter, not the disable list, was the real hole in this gate.
//
// Gating on the four WCAG A/AA tags alone means axe runs 69 of its rules and
// simply does not execute the other 36 — so a whole class of defect was
// invisible here by construction rather than by decision. Not running, among
// others: heading-order, page-has-heading-one, empty-heading, landmark-one-main,
// landmark-no-duplicate-banner, region, focus-order-semantics, tabindex,
// label-title-only, aria-dialog-name, skip-link, and target-size (which is
// wcag22aa, a tag that was not in the list at all).
//
// They were added as an advisory tier first, which reported 40 findings on the
// first run — landmark-one-main and region on all eight shell-less admin
// routes, heading-order on fifteen, page-has-heading-one on three, plus
// landmark-unique and empty-table-header. Those are fixed, all four legs
// (light/dark x desktop/mobile) report zero in both tiers, and the tier is
// blocking by default so the gain cannot quietly erode. Set
// AXE_ADVISORY_BLOCKS=0 to demote it while working through a new batch.
const BLOCKING_TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'];
const ADVISORY_TAGS = ['wcag22a', 'wcag22aa', 'best-practice'];
const ADVISORY_BLOCKS = process.env.AXE_ADVISORY_BLOCKS !== '0';

// Viewports. The gate ran at 1280x900 only, so the entire phone layout — the
// bottom tab bar, the collapsed topnav, the stacked cards and every <=520px
// override — had never been through an accessibility rule, though qa.mjs has
// rendered it all along.
const VP_TABLE = {
  desktop: { viewport: { width: 1280, height: 900 }, hasTouch: false },
  mobile: { viewport: { width: 390, height: 844 }, hasTouch: true },
};
const VPS = (process.env.VPS || 'desktop,mobile')
  .split(',')
  .map((v) => v.trim())
  .filter((v) => VP_TABLE[v]);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
  let blocking = 0;
  let total = 0;
  let advisory = 0;
  const seen = new Set(); // unique "[impact] rule" pairs, for the summary
  const seenAdvisory = new Set();

  for (const theme of THEMES) {
  for (const vp of VPS) {
    const context = await browser.newContext({
      ...VP_TABLE[vp],
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
      const key = `${name}__${theme}__${vp}`;
      try {
        await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
        await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
        await sleep(700);
        // Gate on the WCAG 2.0/2.1 A + AA success criteria — the legal/standard
        // bar — and report the WCAG 2.2 and best-practice rules alongside.
        const builder = new AxeBuilder({ page }).withTags([
          ...BLOCKING_TAGS,
          ...ADVISORY_TAGS,
        ]);
        if (DISABLED_RULES.length) builder.disableRules(DISABLED_RULES);
        const results = await builder.analyze();
        const isBlockingTier = (it) =>
          ADVISORY_BLOCKS || it.tags.some((t) => BLOCKING_TAGS.includes(t));
        const v = results.violations.filter(isBlockingTier);
        const adv = results.violations.filter((it) => !isBlockingTier(it));
        total += v.length;
        advisory += adv.length;
        const blk = v.filter((it) => FAIL_ON.has(it.impact));
        blocking += blk.length;
        for (const it of adv) seenAdvisory.add(`[${it.impact}] ${it.id}`);
        if (v.length || adv.length) {
          for (const it of v) {
            const mark = FAIL_ON.has(it.impact) ? '!' : '.';
            seen.add(`[${it.impact}] ${it.id}`);
            console.log(`${mark} ${key} [${it.impact}] ${it.id}: ${it.help} (${it.nodes.length} node(s))`);
            for (const node of it.nodes.slice(0, 4)) {
              console.log(`      ${node.target.join(' ')}`);
            }
          }
          for (const it of adv) {
            console.log(`~ ${key} [advisory ${it.impact}] ${it.id}: ${it.help} (${it.nodes.length} node(s))`);
            for (const node of it.nodes.slice(0, 2)) {
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
  console.log(
    `--- advisory (WCAG 2.2 + best-practice): ${advisory} finding(s)` +
      `${ADVISORY_BLOCKS ? ', PROMOTED TO BLOCKING' : ', not gated'} ---`,
  );
  if (seenAdvisory.size) {
    for (const r of [...seenAdvisory].sort()) console.log(`  ~ ${r}`);
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
