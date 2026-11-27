import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';

function compareVersion(a, b) {
  const pa = a.split('.').map((n) => Number(n) || 0);
  const pb = b.split('.').map((n) => Number(n) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i += 1) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}

export function findBrowser() {
  const fromEnv = process.env.CLIP_AUDIT_BROWSER || process.env.E2E_BROWSER;
  if (fromEnv && fs.existsSync(fromEnv)) return fromEnv;

  const direct = [
    'C:/Program Files/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Google/Chrome/Application/chrome.exe',
    'C:/Program Files/Microsoft/Edge/Application/msedge.exe',
    'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
  ];
  for (const candidate of direct) {
    if (fs.existsSync(candidate)) return candidate;
  }

  const roots = [
    'C:/Program Files (x86)/Microsoft/EdgeCore',
    'C:/Program Files/Microsoft/EdgeCore',
  ];
  const found = [];
  for (const dir of roots) {
    if (!fs.existsSync(dir)) continue;
    for (const entry of fs.readdirSync(dir)) {
      const exe = path.join(dir, entry, 'msedge.exe');
      if (fs.existsSync(exe)) found.push([entry, exe]);
    }
  }
  found.sort((a, b) => compareVersion(b[0], a[0]));
  return found[0]?.[1] ?? null;
}

export function waitForServer(base, timeoutMs = 60_000) {
  const startedAt = Date.now();
  return new Promise((resolve, reject) => {
    const tick = () => {
      const req = http.get(base, (res) => {
        res.resume();
        resolve();
      });
      req.on('error', () => {
        if (Date.now() - startedAt > timeoutMs) reject(new Error('preview server timeout'));
        else setTimeout(tick, 400);
      });
    };
    tick();
  });
}

export function stopProcess(child) {
  if (!child) return;
  if (process.platform === 'win32') {
    spawn('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
  } else {
    child.kill('SIGTERM');
  }
}

export function startPreview({ root, port, label = 'preview' }) {
  const child = spawn(
    process.platform === 'win32' ? 'npm.cmd' : 'npm',
    ['run', 'preview', '--', '--host', '127.0.0.1', '--port', String(port), '--strictPort'],
    { cwd: root, stdio: ['ignore', 'pipe', 'pipe'], shell: process.platform === 'win32' },
  );
  child.stdout.on('data', () => {});
  child.stderr.on('data', (chunk) => process.stderr.write(`[${label}] ${chunk}`));
  return child;
}
