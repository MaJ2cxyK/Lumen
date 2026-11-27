// Core workflow smoke test against the production frontend build and mock backend.
// Usage: npm run build && npm run test:e2e
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import { findBrowser, startPreview, stopProcess, waitForServer } from './browser-harness.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const port = Number(process.env.E2E_PORT ?? 4174);
const base = `http://127.0.0.1:${port}`;
const failures = [];

function check(condition, label) {
  if (condition) {
    console.log(`[e2e] PASS ${label}`);
  } else {
    failures.push(label);
    console.error(`[e2e] FAIL ${label}`);
  }
}

async function navigate(page, hash) {
  await page.evaluate((value) => {
    window.location.hash = value;
  }, hash);
  await page.waitForTimeout(250);
}

async function run(page) {
  const title = `E2E smoke ${Date.now()}`;

  await page.goto(`${base}/#/tasks`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('#root > *', { timeout: 15_000 });

  await page.getByRole('button', { name: 'New task' }).first().click();
  const createDialog = page.locator('[role="dialog"]');
  await createDialog.waitFor({ state: 'visible', timeout: 5_000 });

  const selects = createDialog.locator('select');
  await selects.nth(1).selectOption({ label: 'Software Team' });
  await createDialog.getByPlaceholder('What needs to be done').fill(title);
  await createDialog
    .getByPlaceholder('Context, scope and constraints')
    .fill('Smoke-test the create, dispatch, approval, and review workflow.');
  await selects.nth(3).selectOption({ label: 'High' });
  await createDialog.getByRole('button', { name: 'Create task' }).click();
  await createDialog.waitFor({ state: 'hidden', timeout: 5_000 });
  await page.getByText(title, { exact: true }).first().waitFor({ timeout: 5_000 });
  check(true, 'create task');

  const card = page.locator('div.cursor-pointer').filter({ hasText: title }).first();
  await card.waitFor({ state: 'visible', timeout: 5_000 });
  await card.locator('button[title="Dispatch"]').click();

  const dispatchDialog = page.locator('[role="dialog"]');
  await dispatchDialog.waitFor({ state: 'visible', timeout: 5_000 });
  await dispatchDialog.getByRole('button', { name: 'Dispatch' }).click();
  await dispatchDialog.waitFor({ state: 'hidden', timeout: 5_000 });
  check(true, 'dispatch task');

  // The mock backend does not synthesize stage gates or worktrees for new tasks.
  // Approval and review assertions use the seeded demo records.
  // The mock run completes in 2.6 seconds; wait for persisted terminal state.
  await page.waitForTimeout(3_200);
  await page.getByText(title, { exact: true }).first().click();
  await page.waitForURL(/#\/tasks\/task_/, { timeout: 5_000 });
  await page.waitForFunction(
    () => document.body.innerText.includes('Completed'),
    undefined,
    { timeout: 8_000 },
  );
  check(true, 'task reaches completed state');

  const stageTitle = 'Stage gate: Product Manager, Prepare the Q4 operations review';
  await navigate(page, '#/approvals');
  await page.getByText(stageTitle, { exact: true }).first().waitFor({ timeout: 8_000 });
  const approvalCard = page
    .getByText(stageTitle, { exact: true })
    .first()
    .locator('xpath=ancestor::div[contains(@class,"rounded-xl")]')
    .first();
  await approvalCard.getByRole('button', { name: 'Approve' }).click();
  await page.waitForFunction(
    (value) => {
      const cards = [...document.querySelectorAll('.rounded-xl')];
      const card = cards.find((item) => item.innerText.includes(value));
      return !card || ![...card.querySelectorAll('button')].some(
        (button) => button.innerText.trim() === 'Approve',
      );
    },
    stageTitle,
    { timeout: 5_000 },
  );
  check(true, 'approve stage gate');

  await navigate(page, '#/tasks/task_demo_1');
  await page.getByText('Changes review', { exact: true }).first().waitFor({ timeout: 8_000 });
  await page.getByText('src/auth/login.ts', { exact: true }).first().waitFor({ timeout: 5_000 });
  await page.getByRole('button', { name: 'Apply to project' }).waitFor({ timeout: 5_000 });
  check(true, 'load changes review');
}

async function main() {
  if (!fs.existsSync(path.join(root, 'dist', 'index.html'))) {
    console.error('[e2e] dist/index.html is missing; run npm run build first');
    process.exit(1);
  }

  const executablePath = findBrowser();
  if (!executablePath) {
    console.error('[e2e] Chrome or Edge was not found; set E2E_BROWSER to a browser executable');
    process.exit(1);
  }

  const preview = startPreview({ root, port, label: 'e2e-preview' });
  let browser;
  let page;
  const pageErrors = [];

  try {
    await waitForServer(base);
    browser = await chromium.launch({ executablePath, headless: true });
    page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    await page.addInitScript(() => localStorage.setItem('lumen.lang', 'en'));
    page.on('pageerror', (err) => pageErrors.push(err.message));
    await run(page);
    check(pageErrors.length === 0, 'no page errors');
  } catch (err) {
    failures.push(`uncaught: ${err?.message ?? err}`);
    console.error(`[e2e] FAIL uncaught: ${err?.stack ?? err}`);
    if (page) {
      const shot = path.join(os.tmpdir(), 'lumen-e2e-failure.png');
      await page.screenshot({ path: shot, fullPage: true }).catch(() => {});
      console.error(`[e2e] screenshot: ${shot}`);
    }
  } finally {
    await browser?.close();
    stopProcess(preview);
  }

  if (failures.length > 0) {
    console.error(`\n[e2e] ${failures.length} check(s) failed`);
    failures.forEach((item) => console.error(`  - ${item}`));
    process.exitCode = 1;
  } else {
    console.log('\n[e2e] core workflow smoke test passed');
  }
}

main().catch((err) => {
  console.error(`[e2e] fatal: ${err?.stack ?? err}`);
  process.exit(1);
});
