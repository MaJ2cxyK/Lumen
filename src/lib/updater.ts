import { isTauri } from './env';
import { tr } from './i18n';
import type { UpdateChannel } from './updatePrefs';
import type { Update } from '@tauri-apps/plugin-updater';

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  notes: string;
  date?: string;
}

export interface UpdateProgress {
  downloaded: number;
  total: number | null;
}

let pending: Update | null = null;

export function updaterSupported(): boolean {
  return isTauri;
}

/** 检查更新；未配置更新源时抛出插件返回的错误，由调用方提示。 */
export async function checkForUpdate(options?: {
  channel?: UpdateChannel;
}): Promise<UpdateInfo | null> {
  if (!isTauri) {
    throw new Error(
      tr(
        '当前为浏览器预览，检查更新仅在桌面端可用',
        'Update checks are only available in the desktop app',
      ),
    );
  }
  const { check } = await import('@tauri-apps/plugin-updater');
  // 渠道通过请求头传递，更新源可按渠道返回不同的 latest.json。
  const update = await check({
    headers: { 'X-Lumen-Channel': options?.channel ?? 'stable' },
  });
  if (!update) {
    pending = null;
    return null;
  }
  pending = update;
  return {
    version: update.version,
    currentVersion: update.currentVersion,
    notes: update.body ?? '',
    date: update.date,
  };
}

/** 下载并安装已检查到的更新。 */
export async function installUpdate(
  onProgress?: (progress: UpdateProgress) => void,
): Promise<void> {
  if (!pending) {
    throw new Error(
      tr('没有待安装的更新，请先检查更新', 'No pending update. Check for updates first.'),
    );
  }
  let downloaded = 0;
  let total: number | null = null;
  await pending.downloadAndInstall((event) => {
    if (event.event === 'Started') {
      total = event.data.contentLength ?? null;
    } else if (event.event === 'Progress') {
      downloaded += event.data.chunkLength;
    }
    onProgress?.({ downloaded, total });
  });
  pending = null;
}
