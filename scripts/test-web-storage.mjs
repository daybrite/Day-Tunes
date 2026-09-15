// Exercise the real browser store: reload with catalog requests blocked and check saved data.
// Run after `day build -p web-dom`. Playwright is resolved like Day's browser driver.
import { createRequire } from 'node:module';
import fs from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';

const dist = path.resolve(process.argv[2] || 'build/day/cargo/web-dom/debug/dist');
await fs.access(path.join(dist, 'index.html'));
const dependencyRoot = process.env.DAY_WEB_DRIVER_PLAYWRIGHT || process.cwd();
const { chromium } = createRequire(path.join(dependencyRoot, 'resolve-anchor.js'))('playwright');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript',
  '.wasm': 'application/wasm', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml' };
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost');
  const file = path.resolve(dist, `.${decodeURIComponent(url.pathname === '/' ? '/index.html' : url.pathname)}`);
  if (!file.startsWith(`${dist}${path.sep}`)) { res.writeHead(403).end(); return; }
  try {
    const bytes = await fs.readFile(file);
    res.writeHead(200, { 'Content-Type': types[path.extname(file)] || 'application/octet-stream',
      'Cross-Origin-Opener-Policy': 'same-origin', 'Cross-Origin-Embedder-Policy': 'require-corp' });
    res.end(bytes);
  } catch { res.writeHead(404).end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const profile = await fs.mkdtemp(path.join(os.tmpdir(), 'tunes-storage-'));
let context;
try {
  context = await chromium.launchPersistentContext(profile, {
    headless: true, viewport: { width: 1000, height: 720 },
  });
  const page = context.pages()[0];
  await page.goto(`http://127.0.0.1:${server.address().port}/?TUNES_CATALOG_URL=asset%3Acatalog-fixture`);
  await page.getByText('France Culture', { exact: true }).first().click({ timeout: 60000 });
  await page.getByRole('button', { name: 'Add to Favorites', exact: true }).click();
  await page.locator('textarea').first().fill('Persistence check');
  await page.getByRole('button', { name: 'Remove from Favorites', exact: true }).waitFor();
  await page.waitForTimeout(1000);

  // The second launch must use OPFS, not fetch another copy of the fixture.
  await page.route('**/assets/data/catalog-fixture/*', route => route.abort());
  await page.reload();
  await page.getByText('France Culture', { exact: true }).first().click({ timeout: 60000 });
  await page.getByRole('button', { name: 'Remove from Favorites', exact: true }).waitFor();
  if (await page.locator('textarea').first().inputValue() !== 'Persistence check') {
    throw new Error('The station note did not survive reload');
  }
  console.log('PASS: cached catalog, favorite, and note survive reload without catalog access');
} finally {
  if (context) await context.close();
  await new Promise(resolve => server.close(resolve));
  await fs.rm(profile, { recursive: true, force: true });
}
