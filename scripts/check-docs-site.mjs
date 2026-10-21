// Behaviour gate for the published, versioned manual.
//
//   node scripts/check-docs-site.mjs <site-dir> [--base /BirdNet-Behavior/]
//
// `scripts/build-versioned-docs.sh` proves the *layout* (which tree came from
// which tag) and `check-book-links.py` that every internal link resolves. Neither
// loads a page in a browser, so neither can say whether the version switcher is
// there, says the right thing, or survives the one page a reader of a bad
// address lands on. This does, against the built site served the way GitHub
// Pages serves it (a missing path answers the root book's 404.html with 404).
//
// What it checks, on the root, on every version in versions.json, and on a
// version that does not exist:
//   - the switcher is present and lists exactly the versions in versions.json,
//     with its own version selected (the root: "latest");
//   - the banner says what it should: none on the root, "pinned" on the latest
//     release's own directory, "latest release is X" on an older one, "development
//     manual" on dev;
//   - choosing another version goes to the same page there;
//   - the 404 page offers the switcher too, and choosing a version leaves it;
//   - at 390 px the menu bar does not clip the book title to an ellipsis and
//     the page does not scroll sideways;
//   - search finds a word that is on the page, and each result opens;
//   - axe-core finds nothing serious or critical against WCAG 2.0 to 2.2 A and
//     AA on the front page of the root, the oldest version and dev, the 404
//     page, and two pages with the markup that has failed before (a table of
//     code samples in the dark theme, a wide table), in the light and the navy
//     theme, at 1440 and 390 px. Best-practice rules and the `minor` and
//     `moderate` findings are not gated: mdBook's own toolbar is outside any
//     landmark (`region`), and the switch is a label that takes role=button
//     (`aria-allowed-role`, minor).
//
// Needs Playwright: `npm ci` in tools/visual-qa. CHROMIUM_PATH picks a browser.
import { createRequire } from 'node:module';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';

const require = createRequire(new URL('../tools/visual-qa/package.json', import.meta.url));
const { chromium } = require('playwright');

const args = process.argv.slice(2);
const baseFlag = args.indexOf('--base');
const BASE_PATH = baseFlag >= 0 ? args.splice(baseFlag, 2)[1] : '/BirdNet-Behavior/';
const siteDir = args[0];
if (!siteDir || !fs.existsSync(path.join(siteDir, 'versions.json'))) {
  console.error('usage: check-docs-site.mjs <site-dir> [--base /path/]   (no versions.json there)');
  process.exit(2);
}
const root = path.resolve(siteDir);

const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json',
  '.png': 'image/png', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.woff': 'font/woff', '.txt': 'text/plain' };
const server = http.createServer((req, res) => {
  let p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  let status = 200, file = null;
  if (p.startsWith(BASE_PATH)) {
    const rel = p.slice(BASE_PATH.length);
    let cand = path.normalize(path.join(root, rel));
    if (cand.startsWith(root)) {
      if (fs.existsSync(cand) && fs.statSync(cand).isDirectory()) cand = path.join(cand, 'index.html');
      if (fs.existsSync(cand)) file = cand;
    }
  }
  if (!file) { status = 404; file = path.join(root, '404.html'); }
  res.writeHead(status, { 'Content-Type': TYPES[path.extname(file)] || 'application/octet-stream' });
  fs.createReadStream(file).pipe(res);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const ORIGIN = `http://127.0.0.1:${server.address().port}`;
const SITE = ORIGIN + BASE_PATH;

const manifest = JSON.parse(fs.readFileSync(path.join(root, 'versions.json'), 'utf8'));
const names = manifest.versions.map((v) => v.name);
const latest = manifest.latest;

let failures = 0;
const check = (ok, what, detail = '') => {
  console.log(`  ${ok ? 'ok  ' : 'FAIL'}  ${what}${ok || !detail ? '' : ' — ' + detail}`);
  if (!ok) failures++;
};

check(names[names.length - 1] === 'dev', 'versions.json lists dev last', names.join(','));
check(names[0] === latest, 'versions.json lists the latest release first', `${names[0]} vs latest ${latest}`);
for (const v of manifest.versions) {
  check(fs.existsSync(path.join(root, v.path, 'index.html')), `${v.path} exists in the site`);
}

const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {});
const newPage = async (width) => {
  const ctx = await browser.newContext({ viewport: { width, height: 844 } });
  return ctx.newPage();
};

const snapshot = (page) => page.evaluate(() => {
  const select = document.querySelector('.bnb-version-select');
  const banner = document.querySelector('.bnb-version-banner');
  const title = document.querySelector('.menu-title');
  const ts = title ? getComputedStyle(title) : null;
  return {
    id: window.bnbDocs || null,
    options: select ? [...select.options].map((o) => ({ label: o.textContent, value: o.value, selected: o.selected })) : null,
    banner: banner ? banner.textContent : null,
    titleHidden: !title || ts.display === 'none',
    titleClipped: !!title && ts.display !== 'none' && title.scrollWidth > title.clientWidth,
    sideways: document.documentElement.scrollWidth > document.documentElement.clientWidth,
  };
});

// A page that exists in every tree: the manual's own front page.
const PAGE = 'introduction.html';
const places = [{ label: 'root', url: SITE, expect: { version: latest, depth: 0, selected: '' } }]
  .concat(manifest.versions.map((v) => ({ label: v.name, url: SITE + v.path, expect: { version: v.name, depth: 1, selected: v.path } })));

const page = await newPage(1440);
for (const p of places) {
  console.log(`${p.label}`);
  const resp = await page.goto(p.url + PAGE, { waitUntil: 'networkidle' });
  check(resp.status() === 200, `${p.label}/${PAGE} answers 200`, String(resp.status()));
  await page.waitForSelector('.bnb-version-select', { timeout: 5000 }).catch(() => {});
  const s = await snapshot(page);
  check(s.id && s.id.version === p.expect.version && s.id.depth === p.expect.depth,
    `the page knows it is ${p.expect.version} at depth ${p.expect.depth}`, JSON.stringify(s.id));
  if (!s.options) { check(false, 'the version switcher is present'); continue; }
  const want = ['', ...manifest.versions.map((v) => v.path)];
  check(JSON.stringify(s.options.map((o) => o.value)) === JSON.stringify(want),
    'the switcher lists exactly versions.json (and "latest")', s.options.map((o) => o.value).join('|'));
  const sel = s.options.filter((o) => o.selected).map((o) => o.value);
  check(sel.length === 1 && sel[0] === p.expect.selected, `the switcher shows ${p.expect.selected || 'latest'} selected`, sel.join('|'));
  const b = s.banner || '';
  if (p.label === 'root') check(s.banner === null, 'the root has no banner');
  else if (p.label === 'dev') check(/development manual/.test(b) && b.includes(latest), 'dev says it is the development manual and names the latest release', b);
  else if (p.label === latest) check(/pinned to/.test(b) && b.includes(latest), `${latest} says its address is pinned to it`, b);
  else check(b.includes(`manual is for ${p.label}`) && b.includes(`latest release is ${latest}`), `${p.label} says it is old and names ${latest}`, b);
}

// Choosing a version goes to the same page there.
console.log('switching');
{
  const target = manifest.versions.find((v) => v.name !== 'dev' && v.name !== latest) || manifest.versions[0];
  const from = SITE + target.path + 'getting-started/installation.html';
  await page.goto(from, { waitUntil: 'networkidle' });
  await page.waitForSelector('.bnb-version-select');
  await page.selectOption('.bnb-version-select', 'dev/');
  await page.waitForURL(SITE + 'dev/getting-started/installation.html', { timeout: 5000 })
    .then(() => check(true, `${target.name} -> dev keeps the page`))
    .catch(() => check(false, `${target.name} -> dev keeps the page`, page.url()));
  await page.selectOption('.bnb-version-select', '');
  await page.waitForURL(SITE + 'getting-started/installation.html', { timeout: 5000 })
    .then(() => check(true, 'dev -> latest keeps the page'))
    .catch(() => check(false, 'dev -> latest keeps the page', page.url()));
}

// An address that is not a version.
console.log('unknown version');
{
  const resp = await page.goto(SITE + 'v9.9.9/getting-started/installation.html', { waitUntil: 'networkidle' });
  check(resp.status() === 404, 'an unknown version answers 404', String(resp.status()));
  const has = await page.waitForSelector('.bnb-version-select', { timeout: 3000 }).then(() => true).catch(() => false);
  check(has, 'the 404 page offers the version switcher');
  if (has) {
    await page.selectOption('.bnb-version-select', manifest.versions[0].path);
    await page.waitForURL((u) => u.pathname.startsWith(BASE_PATH + manifest.versions[0].path), { timeout: 5000 })
      .then(() => check(true, 'choosing a version leaves the 404 page'))
      .catch(() => check(false, 'choosing a version leaves the 404 page', page.url()));
  }
}

// Search finds a word on the page, and each result opens.
console.log('search');
for (const p of [places[0], places[places.length - 1]]) {
  await page.goto(p.url + PAGE, { waitUntil: 'networkidle' });
  await page.click('#mdbook-search-toggle');
  await page.fill('#mdbook-searchbar', 'spectrogram');
  await page.waitForSelector('#mdbook-searchresults li a', { timeout: 5000 }).catch(() => {});
  const hrefs = await page.$$eval('#mdbook-searchresults li a', (a) => a.slice(0, 5).map((x) => x.href));
  check(hrefs.length > 0, `${p.label}: search for "spectrogram" has results`);
  let opened = 0;
  for (const h of hrefs) {
    const r = await page.request.get(h.split('#')[0].split('?')[0]);
    if (r.status() === 200 && h.startsWith(p.url)) opened++;
  }
  check(opened === hrefs.length, `${p.label}: every result opens, inside this version`, `${opened} of ${hrefs.length}`);
}

// A phone.
console.log('phone, 390 px');
{
  const phone = await newPage(390);
  for (const p of places) {
    await phone.goto(p.url + PAGE, { waitUntil: 'networkidle' });
    await phone.waitForSelector('.bnb-version-select', { timeout: 5000 }).catch(() => {});
    const s = await snapshot(phone);
    check(!s.titleClipped, `${p.label}: the menu bar does not clip the book title to an ellipsis`);
    check(!s.sideways, `${p.label}: the page does not scroll sideways`);
  }
}

// Accessibility.
console.log('accessibility (axe), light and navy, 1440 and 390 px');
{
  const AxeModule = require('@axe-core/playwright');
  const AxeBuilder = AxeModule.default || AxeModule.AxeBuilder || AxeModule;
  const oldest = manifest.versions[manifest.versions.length - 2];
  const targets = [
    ['root front page', SITE + PAGE],
    [`${oldest.name} front page`, SITE + oldest.path + PAGE],
    ['dev front page', SITE + 'dev/' + PAGE],
    ['the 404 page', SITE + 'v9.9.9/'],
    ['design tokens', SITE + 'reference/design-tokens.html'],
    ['migration guide', SITE + 'guides/migration.html'],
  ];
  for (const width of [1440, 390]) {
    for (const theme of ['light', 'navy']) {
      const ctx = await browser.newContext({ viewport: { width, height: 844 }, colorScheme: theme === 'light' ? 'light' : 'dark' });
      await ctx.addInitScript((t) => { try { localStorage.setItem('mdbook-theme', t); } catch (e) { /* none */ } }, theme);
      const pg = await ctx.newPage();
      for (const [label, url] of targets) {
        await pg.goto(url, { waitUntil: 'networkidle' });
        await pg.waitForSelector('.bnb-version-select', { timeout: 3000 }).catch(() => {});
        const r = await new AxeBuilder({ page: pg })
          .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa']).analyze();
        const bad = r.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
        check(bad.length === 0, `${label}, ${theme}, ${width}px`,
          bad.map((v) => `${v.id} (${v.nodes.length}): ${v.nodes[0].target.join(' ')}`).join('; '));
      }
      await ctx.close();
    }
  }
}

await browser.close();
server.close();
console.log(failures ? `\n${failures} check(s) failed` : '\nOK: the versioned manual behaves as documented in a browser');
process.exit(failures ? 1 : 0);
