// 自动更新偏好：保存在本地，桌面端与浏览器预览共用。
export type UpdateChannel = 'stable' | 'beta';

export interface UpdatePrefs {
  autoCheck: boolean;
  channel: UpdateChannel;
  skippedVersion: string | null;
  lastCheckAt: number | null;
}

const STORAGE_KEY = 'lumen.update';

const DEFAULTS: UpdatePrefs = {
  autoCheck: true,
  channel: 'stable',
  skippedVersion: null,
  lastCheckAt: null,
};

export const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

export function readUpdatePrefs(): UpdatePrefs {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULTS };
    const parsed = JSON.parse(raw) as Partial<UpdatePrefs>;
    return {
      autoCheck: parsed.autoCheck !== false,
      channel: parsed.channel === 'beta' ? 'beta' : 'stable',
      skippedVersion:
        typeof parsed.skippedVersion === 'string' ? parsed.skippedVersion : null,
      lastCheckAt: typeof parsed.lastCheckAt === 'number' ? parsed.lastCheckAt : null,
    };
  } catch {
    return { ...DEFAULTS };
  }
}

export function writeUpdatePrefs(patch: Partial<UpdatePrefs>): UpdatePrefs {
  const next = { ...readUpdatePrefs(), ...patch };
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    /* 写入失败时仅本次会话生效 */
  }
  return next;
}
