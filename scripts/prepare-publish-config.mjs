// 生成 CI/发布使用的合并配置：release 配置 + signing 配置 -> src-tauri/tauri.publish.conf.json
// 用法：node scripts/prepare-publish-config.mjs
//
// 环境变量：
//   UPDATE_BASE_URL            更新包下载前缀；默认取 GitHub 仓库的 latest release 地址
//   GITHUB_REPOSITORY          形如 owner/repo，CI 中自动注入，用于推导默认下载地址
//   TAURI_SIGNING_PUBLIC_KEY   覆盖 plugins.updater.pubkey（优先级更高）
//   TAURI_UPDATER_PUBKEY       同上，兼容旧命名
//   SKIP_SIGN=1                演练模式：不合并签名配置，并关闭 updater 产物
//   SKIP_CODE_SIGN=1           跳过 Authenticode 代码签名，但保留 updater 签名产物
// 正式模式要求 UPDATE_BASE_URL 必须是 HTTPS；本地 HTTP 演练请使用独立测试配置。
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const srcTauri = path.join(root, 'src-tauri');
const releasePath = path.join(srcTauri, 'tauri.release.conf.json');
const signingPath = path.join(srcTauri, 'tauri.signing.conf.json');
const outPath = path.join(srcTauri, 'tauri.publish.conf.json');

const DEFAULT_UPDATE_BASE_URL = 'https://github.com/<owner>/<repo>/releases/latest/download';

function readJson(file) {
  let raw;
  try {
    raw = fs.readFileSync(file, 'utf8');
  } catch (err) {
    throw new Error(`读取配置失败：${file}（${err.message}）`);
  }
  try {
    return JSON.parse(raw);
  } catch (err) {
    throw new Error(`配置不是合法 JSON：${file}（${err.message}）`);
  }
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

// 深度合并：对象递归合并，数组与基本类型直接由后者覆盖
function deepMerge(base, override) {
  if (isPlainObject(base) && isPlainObject(override)) {
    const result = { ...base };
    for (const [key, value] of Object.entries(override)) {
      result[key] = key in result ? deepMerge(result[key], value) : value;
    }
    return result;
  }
  return override;
}

// 得到更新包下载前缀：显式环境变量 > GitHub 仓库地址 > 占位符
function resolveBaseUrl() {
  const fromEnv = (process.env.UPDATE_BASE_URL ?? '').trim();
  if (fromEnv) return fromEnv.replace(/\/+$/, '');

  const repo = (process.env.GITHUB_REPOSITORY ?? '').trim();
  if (repo.includes('/')) return `https://github.com/${repo}/releases/latest/download`;

  return DEFAULT_UPDATE_BASE_URL;
}

const releaseConfig = readJson(releasePath);
const skipSign = /^(1|true|yes)$/i.test((process.env.SKIP_SIGN ?? '').trim());
const skipCodeSign = /^(1|true|yes)$/i.test((process.env.SKIP_CODE_SIGN ?? '').trim());

let merged = releaseConfig;
if (skipSign) {
  // 演练模式没有 updater 私钥，无法签名更新包，必须关闭 updater 产物。
  merged = deepMerge(releaseConfig, { bundle: { createUpdaterArtifacts: false } });
  console.log('[prepare-publish-config] SKIP_SIGN 已设置：跳过签名配置合并，并关闭更新包产物（演练模式）。');
} else if (skipCodeSign) {
  // 无 Authenticode 证书时仍保留 updater 签名，安装包会被明确标记为未签名。
  console.log('[prepare-publish-config] SKIP_CODE_SIGN 已设置：跳过 Authenticode 签名配置，保留 updater 签名产物。');
} else {
  merged = deepMerge(releaseConfig, readJson(signingPath));
}

// 自动更新配置：endpoints 用拼接出来的 latest.json，pubkey 支持环境变量覆盖
if (!isPlainObject(merged.plugins)) merged.plugins = {};
if (!isPlainObject(merged.plugins.updater)) merged.plugins.updater = {};

const baseUrl = resolveBaseUrl();
if (!skipSign && !/^https:\/\//i.test(baseUrl)) {
  throw new Error(`正式发布的更新地址必须使用 HTTPS：${baseUrl}。本地 HTTP 演练请使用独立的测试配置。`);
}
merged.plugins.updater.endpoints = [`${baseUrl}/latest.json`];

const pubkey = (process.env.TAURI_SIGNING_PUBLIC_KEY ?? process.env.TAURI_UPDATER_PUBKEY ?? '').trim();
if (pubkey) {
  merged.plugins.updater.pubkey = pubkey;
}
if (!skipSign && (!pubkey || pubkey.startsWith('REPLACE_'))) {
  throw new Error(
    '缺少可用的 TAURI_SIGNING_PUBLIC_KEY，更新器无法校验签名。请设置该环境变量（内容为 .key.pub 文件全文）后重试。',
  );
}

// release 配置必须启用 updater 产物，否则不会产出 *.nsis.zip 和 .sig
if (!isPlainObject(merged.bundle)) merged.bundle = {};
if (merged.bundle.createUpdaterArtifacts === undefined) {
  merged.bundle.createUpdaterArtifacts = true;
  console.log('[prepare-publish-config] 已补全 bundle.createUpdaterArtifacts = true。');
}

// 写盘前先序列化并重新解析，确保产物是可解析的 JSON
const serialized = `${JSON.stringify(merged, null, 2)}\n`;
let reparsed;
try {
  reparsed = JSON.parse(serialized);
} catch (err) {
  throw new Error(`合并结果无法解析为 JSON：${err.message}`);
}

fs.writeFileSync(outPath, serialized, 'utf8');

console.log(`[prepare-publish-config] 已生成 ${path.relative(root, outPath)}`);
console.log(`[prepare-publish-config] updater.endpoints = ${reparsed.plugins.updater.endpoints.join(', ')}`);
console.log(`[prepare-publish-config] updater.pubkey = ${reparsed.plugins.updater.pubkey || '(保留原值/为空)'}`);
if (skipSign) {
  console.log('[prepare-publish-config] 未包含 bundle.windows.signCommand。');
} else if (skipCodeSign) {
  console.log('[prepare-publish-config] 未包含 Authenticode signCommand（无证书模式）。');
}
