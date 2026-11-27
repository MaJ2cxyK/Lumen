// 根据 NSIS 打包产物生成 Tauri updater 清单 latest.json
// 用法：node scripts/gen-latest-json.mjs
//
// 环境变量：
//   UPDATE_BASE_URL     更新包下载前缀，默认取 GitHub 仓库的 latest release 地址
//   GITHUB_REPOSITORY   CI 中自动注入，用于推导默认下载地址
//   VERSION             覆盖版本号（默认读取 src-tauri/tauri.conf.json）
//   RELEASE_NOTES       直接提供发布说明（优先）
//   RELEASE_NOTES_FILE  从文件读取发布说明（RELEASE_NOTES 为空时生效）
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const bundleDir = path.join(root, 'src-tauri', 'target', 'release', 'bundle', 'nsis');
const tauriConfPath = path.join(root, 'src-tauri', 'tauri.conf.json');
const outPath = path.join(bundleDir, 'latest.json');

const DEFAULT_UPDATE_BASE_URL = 'https://github.com/<owner>/<repo>/releases/latest/download';
const DEFAULT_NOTES = 'Bug fixes and stability improvements.';

function readJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (err) {
    throw new Error(`读取或解析 JSON 失败：${file}（${err.message}）`);
  }
}

function resolveBaseUrl() {
  const fromEnv = (process.env.UPDATE_BASE_URL ?? '').trim();
  if (fromEnv) return fromEnv.replace(/\/+$/, '');

  const repo = (process.env.GITHUB_REPOSITORY ?? '').trim();
  if (repo.includes('/')) return `https://github.com/${repo}/releases/latest/download`;

  return DEFAULT_UPDATE_BASE_URL;
}

function resolveNotes() {
  const inline = (process.env.RELEASE_NOTES ?? '').trim();
  if (inline) return inline;

  const notesFile = (process.env.RELEASE_NOTES_FILE ?? '').trim();
  if (notesFile) {
    const abs = path.resolve(process.cwd(), notesFile);
    if (!fs.existsSync(abs)) {
      throw new Error(`RELEASE_NOTES_FILE 指向的文件不存在：${abs}`);
    }
    const content = fs.readFileSync(abs, 'utf8').trim();
    if (content) return content;
  }

  return DEFAULT_NOTES;
}

if (!fs.existsSync(bundleDir)) {
  throw new Error(`找不到 NSIS 产物目录：${bundleDir}`);
}

const version = (process.env.VERSION ?? '').trim() || String(readJson(tauriConfPath).version ?? '').trim();
if (!version) {
  throw new Error('无法确定版本号：请设置 VERSION，或在 src-tauri/tauri.conf.json 中提供 version。');
}

const entries = fs.readdirSync(bundleDir);
const hasSig = (name) => fs.existsSync(path.join(bundleDir, `${name}.sig`));

// 目标目录里可能残留旧版本产物，优先挑选文件名包含当前版本的那一个
function choose(names) {
  if (names.length === 0) return null;
  if (names.length === 1) return names[0];
  const matched = names.filter((name) => name.includes(version));
  if (matched.length === 1) return matched[0];
  throw new Error(`存在多个更新包，无法确定发布目标：${(matched.length ? matched : names).join(', ')}`);
}

// createUpdaterArtifacts: true 时 Tauri 为 NSIS 安装包生成 .exe.sig；
// v1Compatible 模式生成 .nsis.zip + .sig。两种产物 updater 插件都支持。
const artifactName =
  choose(entries.filter((name) => name.endsWith('.nsis.zip') && hasSig(name))) ??
  choose(entries.filter((name) => name.endsWith('-setup.exe') && hasSig(name)));
if (!artifactName) {
  throw new Error(
    `未找到带签名的更新包（*.nsis.zip.sig 或 *-setup.exe.sig）：${bundleDir}。请确认已启用 createUpdaterArtifacts 且构建设置了 TAURI_SIGNING_PRIVATE_KEY。`,
  );
}

if (!artifactName.includes(version)) {
  console.warn(`[gen-latest-json] 警告：产物名 ${artifactName} 不包含版本号 ${version}，请确认版本一致。`);
}

const sigPath = path.join(bundleDir, `${artifactName}.sig`);
const signature = fs.readFileSync(sigPath, 'utf8').trim();
if (!signature) {
  throw new Error(`签名文件为空：${sigPath}`);
}

const url = `${resolveBaseUrl()}/${artifactName}`;
const latest = {
  version,
  notes: resolveNotes(),
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': {
      signature,
      url,
    },
  },
};

fs.writeFileSync(outPath, `${JSON.stringify(latest, null, 2)}\n`, 'utf8');

console.log(`[gen-latest-json] 已生成 ${path.relative(root, outPath)}`);
console.log(`[gen-latest-json] version = ${version}`);
console.log(`[gen-latest-json] url = ${url}`);
