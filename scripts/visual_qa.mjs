#!/usr/bin/env node
// Visual QA: take screenshots of every page in the BirdNet-Behavior web UI.
// Usage: node scripts/visual_qa.mjs [base_url] [output_dir]

import { chromium } from 'playwright';
import { mkdirSync, existsSync } from 'fs';
import { join } from 'path';

const BASE_URL = process.argv[2] || 'http://127.0.0.1:8502';
const OUTPUT_DIR = process.argv[3] || '/tmp/birdnet-screenshots';

const PAGES = [
  { path: '/', name: 'dashboard' },
  { path: '/today', name: 'today' },
  { path: '/history', name: 'history' },
  { path: '/weekly', name: 'weekly-report' },
  { path: '/species', name: 'species-list' },
  { path: '/gallery', name: 'gallery' },
  { path: '/life-list', name: 'life-list' },
  { path: '/recordings', name: 'recordings' },
  { path: '/heatmap', name: 'heatmap' },
  { path: '/correlation', name: 'correlation' },
  { path: '/analytics', name: 'analytics' },
  { path: '/timeseries', name: 'timeseries' },
  { path: '/quarantine', name: 'quarantine' },
  { path: '/notifications', name: 'notifications' },
  { path: '/system', name: 'system-health' },
  { path: '/kiosk', name: 'kiosk' },
  { path: '/admin', name: 'admin-overview' },
  { path: '/admin/settings', name: 'admin-settings' },
];

const VIEWPORTS = [
  { width: 1440, height: 900, suffix: 'desktop' },
  { width: 375, height: 812, suffix: 'mobile' },
];

async function main() {
  mkdirSync(OUTPUT_DIR, { recursive: true });
  console.log(`Target: ${BASE_URL} | Output: ${OUTPUT_DIR}`);

  const browser = await chromium.launch({ headless: true });

  for (const viewport of VIEWPORTS) {
    for (const theme of ['dark', 'light']) {
      const context = await browser.newContext({
        viewport: { width: viewport.width, height: viewport.height },
        deviceScaleFactor: 2,
        // Block external font loading to prevent timeout
        extraHTTPHeaders: {},
      });

      // Block Google Fonts requests to avoid font-loading timeouts
      await context.route('**fonts.googleapis.com**', route => route.abort());
      await context.route('**fonts.gstatic.com**', route => route.abort());

      const page = await context.newPage();

      // Set theme
      await page.addInitScript((t) => {
        localStorage.setItem('theme', t);
      }, theme);

      for (const pg of PAGES) {
        const url = `${BASE_URL}${pg.path}`;
        const filename = `${pg.name}_${viewport.suffix}_${theme}.png`;
        const filepath = join(OUTPUT_DIR, filename);

        try {
          await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 10000 });
          await page.waitForTimeout(2500);
          await page.screenshot({ path: filepath, fullPage: true, timeout: 10000 });
          console.log(`  OK  ${filename}`);
        } catch (err) {
          console.log(`  FAIL ${filename}: ${err.message.split('\n')[0]}`);
        }
      }

      await context.close();
    }
  }

  await browser.close();
  console.log(`Done. ${PAGES.length * VIEWPORTS.length * 2} screenshots attempted.`);
}

main().catch(console.error);
