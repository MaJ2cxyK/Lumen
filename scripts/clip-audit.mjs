// 交付门禁：构建产物在无头浏览器里逐页跑 clip-audit，发现被裁切的阴影即失败。
// 用法：npm run ui:clip-audit（需要先 npm run build），或 npm run gate:ui 一步到位。
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import clipAudit from './clip-audit.browser.js';
import { findBrowser, startPreview, stopProcess, waitForServer } from './browser-harness.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const port = Number(process.env.CLIP_AUDIT_PORT ?? 4173);
const base = `http://127.0.0.1:${port}`;

const ROUTES = [
  ['/#/dashboard', '总览'],
  ['/#/tasks', '任务看板'],
  [
    '/#/tasks/task_demo_1',
    '任务详情',
    ['Bash', '变更审查', '应用到项目'],
    ['Bash', 'Changes review', 'Apply to project'],
  ],
  ['/#/projects', '新建任务'],
  [
    '/#/projects/p_demo',
    '项目上下文',
    ['Lumen 工作台', '项目信息', '最近事件'],
    ['Lumen Workspace', 'Project info', 'Recent events'],
  ],
  ['/#/approvals', '审批'],
  ['/#/history', '历史'],
  ['/#/settings', '设置'],
  ['/#/agents', 'Agent'],
  ['/#/teams', '团队'],
  ['/#/extensions', '扩展'],
  ['/#/budgets', '预算'],
  ['/#/usage', '用量'],
];

async function auditPage(page, label, failures, expectTexts = []) {
  await page.evaluate(() => (document.fonts ? document.fonts.ready : null));
  const result = await page.evaluate(clipAudit, { viewport: true, tol: 1, max: 60 });
  const text = await page.evaluate(() => document.body.innerText.replace(/\s+/g, ' ').trim());
  if (!result || result.error) {
    failures.push(`${label}：审计执行失败（${result?.error ?? '无返回'}）`);
    return;
  }
  if (text.length < 120) {
    failures.push(`${label}：页面几乎没有内容（${text.length} 字符）`);
  }
  for (const expectText of expectTexts) {
    if (!text.includes(expectText)) {
      failures.push(`${label}：未找到预期内容「${expectText}」`);
    }
  }
  const cut = result.issues.filter((item) => item.level === 'cut');
  console.log(`[clip-audit] ${label}  cut=${result.cut} hidden=${result.hidden} chars=${text.length}`);
  for (const item of cut) {
    console.log(
      `  ✗ ${item.kind} ${item.side} 裁切 ${item.cut}px（gap ${item.gap}px）` +
        `\n    元素 ${item.el}\n    裁切 ${item.by}（${item.why}）`,
    );
    failures.push(`${label}：${item.kind} ${item.side} 被 ${item.by} 裁切 ${item.cut}px`);
  }
}

async function runPass(browser, lang, failures) {
  const zh = lang !== 'en';
  const prefix = zh ? '' : '[en] ';
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.addInitScript((value) => localStorage.setItem('lumen.lang', value), lang);
  page.on('pageerror', (err) => console.warn(`[clip-audit] 页面异常：${err.message}`));

  try {
    for (const [route, zhLabel, expectZh, expectEn] of ROUTES) {
      const label = `${prefix}${zh ? zhLabel : zhLabel === 'Agent' ? 'Agent' : route}`;
      const expectTexts = zh
        ? Array.isArray(expectZh)
          ? expectZh
          : expectZh
            ? [expectZh]
            : []
        : expectEn ?? [];
      await page.goto(`${base}${route}`, { waitUntil: 'domcontentloaded' });
      await page.waitForSelector('#root > *', { timeout: 15_000 });
      for (const text of expectTexts) {
        await page
          .waitForFunction((expected) => document.body.innerText.includes(expected), text, {
            timeout: 6_000,
          })
          .catch(() => {});
      }
      await page.waitForTimeout(700);
      await auditPage(page, label, failures, expectTexts);
    }

    // 弹层是静态扫描覆盖不到的状态，单独打开再查一次。
    await page.goto(`${base}/#/tasks`, { waitUntil: 'domcontentloaded' });
    await page.waitForSelector('#root > *', { timeout: 15_000 });
    const createButton = page
      .getByRole('button', { name: zh ? '新建任务' : 'New task' })
      .first();
    if (await createButton.count()) {
      await createButton.click();
      await page.waitForSelector('[role="dialog"]', { timeout: 5_000 }).catch(() => {});
      await page.waitForTimeout(400);
      await auditPage(page, `${prefix}任务看板 / 新建任务弹窗`, failures);
      await page.keyboard.press('Escape');
    }
  } finally {
    await page.close();
  }
}

async function main() {
  if (!fs.existsSync(path.join(root, 'dist', 'index.html'))) {
    console.error('[clip-audit] 未找到 dist/index.html，请先执行 npm run build');
    process.exit(1);
  }

  const executablePath = findBrowser();
  if (!executablePath) {
    console.error('[clip-audit] 未找到 Chrome/Edge，可通过 CLIP_AUDIT_BROWSER 指定浏览器路径');
    process.exit(1);
  }

  const preview = startPreview({ root, port, label: 'preview' });

  let browser;
  const failures = [];
  try {
    await waitForServer(base);
    browser = await chromium.launch({ executablePath, headless: true });
    await runPass(browser, 'zh', failures);
    await runPass(browser, 'en', failures);
  } finally {
    await browser?.close();
    stopProcess(preview);
  }

  if (failures.length > 0) {
    console.error(`\n[clip-audit] 未通过，共 ${failures.length} 项问题`);
    failures.forEach((item) => console.error(`  - ${item}`));
    process.exitCode = 1;
  } else {
    console.log('\n[clip-audit] 通过：未发现被裁切的阴影');
  }
}

main().catch((err) => {
  console.error(`[clip-audit] 执行失败：${err?.stack ?? err}`);
  process.exit(1);
});
