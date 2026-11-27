import { useEffect, useRef, useState } from 'react';
import { X } from 'lucide-react';
import { toast } from 'sonner';
import { errorMessage } from '../lib/api';
import { formatBytes } from '../lib/format';
import { useI18n } from '../lib/i18n';
import {
  CHECK_INTERVAL_MS,
  readUpdatePrefs,
  writeUpdatePrefs,
} from '../lib/updatePrefs';
import {
  checkForUpdate,
  installUpdate,
  updaterSupported,
  type UpdateInfo,
  type UpdateProgress,
} from '../lib/updater';
import { Button, cn } from './ui';

const FIRST_CHECK_DELAY = 8_000;
const TICK_INTERVAL = 15 * 60 * 1000;

/**
 * 全局更新提示：启动后延迟检查、每 6 小时复查一次。
 * 自动检查失败保持静默，避免未配置更新源时反复打扰。
 */
export default function UpdatePrompt() {
  const { t } = useI18n();
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const dismissedVersion = useRef<string | null>(null);

  useEffect(() => {
    if (!updaterSupported()) return;
    let disposed = false;
    let running = false;

    const maybeCheck = async () => {
      if (running) return;
      const prefs = readUpdatePrefs();
      if (!prefs.autoCheck) return;
      if (prefs.lastCheckAt && Date.now() - prefs.lastCheckAt < CHECK_INTERVAL_MS) {
        return;
      }
      running = true;
      try {
        const found = await checkForUpdate({ channel: prefs.channel });
        writeUpdatePrefs({ lastCheckAt: Date.now() });
        if (disposed || !found) return;
        if (found.version === readUpdatePrefs().skippedVersion) return;
        if (found.version === dismissedVersion.current) return;
        setUpdate(found);
      } catch {
        // 未配置更新源或网络不可用时静默失败
        writeUpdatePrefs({ lastCheckAt: Date.now() });
      } finally {
        running = false;
      }
    };

    const first = window.setTimeout(() => {
      void maybeCheck();
    }, FIRST_CHECK_DELAY);
    const timer = window.setInterval(() => {
      void maybeCheck();
    }, TICK_INTERVAL);
    return () => {
      disposed = true;
      window.clearTimeout(first);
      window.clearInterval(timer);
    };
  }, []);

  if (!update) return null;

  const percent =
    progress && progress.total
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : null;

  function later() {
    if (!update) return;
    dismissedVersion.current = update.version;
    setUpdate(null);
  }

  function skip() {
    if (!update) return;
    writeUpdatePrefs({ skippedVersion: update.version });
    toast(t(`已跳过版本 ${update.version}`, `Skipped version ${update.version}`));
    setUpdate(null);
  }

  async function install() {
    if (!update || installing) return;
    setInstalling(true);
    setProgress({ downloaded: 0, total: null });
    try {
      await installUpdate(setProgress);
      toast.success(
        t('更新已安装，应用将自动重启', 'Update installed; the app will restart automatically'),
      );
      setUpdate(null);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setInstalling(false);
    }
  }

  return (
    <div className="fixed bottom-5 right-5 z-50 w-[360px] rounded-xl border border-line bg-card p-4 shadow-xl">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-sm font-semibold text-ink">
            {t(`发现新版本 v${update.version}`, `Version ${update.version} available`)}
          </p>
          <p className="mt-0.5 text-xs text-muted">
            {t(
              `当前版本 v${update.currentVersion}`,
              `Current version ${update.currentVersion}`,
            )}
          </p>
        </div>
        <button
          type="button"
          title={t('稍后提醒', 'Remind me later')}
          aria-label={t('稍后提醒', 'Remind me later')}
          onClick={later}
          disabled={installing}
          className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-muted transition-colors hover:bg-hover hover:text-ink disabled:opacity-50"
        >
          <X size={14} />
        </button>
      </div>

      {update.notes && (
        <p className="mt-2 max-h-24 overflow-y-auto whitespace-pre-wrap text-xs leading-5 text-muted">
          {update.notes}
        </p>
      )}

      {installing && (
        <div className="mt-3">
          <div className="h-1.5 overflow-hidden rounded-full bg-soft2">
            <div
              className={cn('h-full bg-primary transition-all', percent === null && 'w-1/3')}
              style={percent === null ? undefined : { width: `${percent}%` }}
            />
          </div>
          <p className="mt-1.5 text-xs text-muted">
            {percent === null
              ? t(
                  `正在下载更新… ${formatBytes(progress?.downloaded ?? 0)}`,
                  `Downloading update… ${formatBytes(progress?.downloaded ?? 0)}`,
                )
              : t(`已下载 ${percent}%`, `Downloaded ${percent}%`)}
          </p>
        </div>
      )}

      <div className="mt-3 flex items-center justify-end gap-2">
        <Button variant="outline" size="xs" onClick={skip} disabled={installing}>
          {t('跳过此版本', 'Skip this version')}
        </Button>
        <Button variant="outline" size="xs" onClick={later} disabled={installing}>
          {t('稍后', 'Later')}
        </Button>
        <Button size="xs" loading={installing} onClick={install}>
          {t('立即更新', 'Update now')}
        </Button>
      </div>
    </div>
  );
}
