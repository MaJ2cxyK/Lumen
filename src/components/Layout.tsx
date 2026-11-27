import { useEffect } from 'react';
import { NavLink, Outlet, useNavigate } from 'react-router-dom';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { toast } from 'sonner';
import {
  Bot,
  History,
  LayoutDashboard,
  ListChecks,
  Minus,
  Moon,
  Settings as SettingsIcon,
  ShieldCheck,
  Square,
  SquarePen,
  Sun,
  X,
  type LucideIcon,
} from 'lucide-react';
import { onEvent } from '../lib/events';
import { isTauri } from '../lib/env';
import { tr, useI18n } from '../lib/i18n';
import type { RunDeltaPayload } from '../lib/types';
import { useAppStore } from '../store';
import UpdatePrompt from './UpdatePrompt';
import { cn } from './ui';

function navGroups(): {
  label: string;
  items: {
    to: string;
    label: string;
    icon: LucideIcon;
    end?: boolean;
    badge?: boolean;
  }[];
}[] {
  return [
    {
      label: tr('工作台', 'Workspace'),
      items: [
        { to: '/', label: tr('总览', 'Overview'), icon: LayoutDashboard, end: true },
        { to: '/projects', label: tr('新建任务', 'New task'), icon: SquarePen, end: true },
        { to: '/agents', label: 'Agent', icon: Bot },
        { to: '/tasks', label: tr('任务看板', 'Tasks'), icon: ListChecks, badge: true },
      ],
    },
    {
      label: tr('运行', 'Operations'),
      items: [
        { to: '/approvals', label: tr('审批中心', 'Approvals'), icon: ShieldCheck },
        { to: '/history', label: tr('执行历史', 'History'), icon: History },
      ],
    },
    {
      label: tr('系统', 'System'),
      items: [{ to: '/settings', label: tr('设置', 'Settings'), icon: SettingsIcon }],
    },
  ];
}

const BRAND_FONT =
  '"Bodoni MT", "Bodoni 72", Didot, "Playfair Display", Georgia, "Times New Roman", serif';

export function LumenWordmark({
  height = 40,
  className,
}: {
  height?: number;
  className?: string;
}) {
  return (
    <span
      className={cn('select-none whitespace-nowrap text-ink', className)}
      style={{
        fontFamily: BRAND_FONT,
        fontStyle: 'italic',
        fontWeight: 700,
        fontSize: height * 0.62,
        lineHeight: 1,
        letterSpacing: '-0.045em',
      }}
    >
      Lumen
    </span>
  );
}

function windowAction(action: 'minimize' | 'maximize' | 'close') {
  if (!isTauri) return;
  const win = getCurrentWindow();
  const promise =
    action === 'minimize'
      ? win.minimize()
      : action === 'maximize'
        ? win.toggleMaximize()
        : win.close();
  promise.catch(() => {});
}

function TitleBar() {
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const { t } = useI18n();

  return (
    <div
      data-tauri-drag-region
      onDoubleClick={() => windowAction('maximize')}
      className="flex h-8 shrink-0 select-none items-center justify-between border-b border-line bg-base pl-3"
    >
      <div data-tauri-drag-region className="flex flex-1 items-center" />
      <div className="flex items-center">
        <button
          type="button"
          title={t(
            theme === 'dark' ? '切换到浅色外观' : '切换到深色外观',
            theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme',
          )}
          onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}
          className="mr-1 flex h-7 w-7 items-center justify-center rounded-full text-muted transition-colors hover:bg-hover hover:text-ink"
        >
          {theme === 'dark' ? <Sun size={14} /> : <Moon size={14} />}
        </button>
        <button
          type="button"
          title={t('最小化', 'Minimize')}
          onClick={() => windowAction('minimize')}
          className="flex h-8 w-10 items-center justify-center text-muted transition-colors hover:bg-hover hover:text-ink"
        >
          <Minus size={13} />
        </button>
        <button
          type="button"
          title={t('最大化 / 还原', 'Maximize / restore')}
          onClick={() => windowAction('maximize')}
          className="flex h-8 w-10 items-center justify-center text-muted transition-colors hover:bg-hover hover:text-ink"
        >
          <Square size={10} />
        </button>
        <button
          type="button"
          title={t('关闭', 'Close')}
          onClick={() => windowAction('close')}
          className="flex h-8 w-10 items-center justify-center text-muted transition-colors hover:bg-danger hover:text-white"
        >
          <X size={13} />
        </button>
      </div>
    </div>
  );
}

export default function Layout() {
  const navigate = useNavigate();
  const { t } = useI18n();
  const needsYou = useAppStore((s) => s.needsYou);
  const loaded = useAppStore((s) => s.loaded);
  const coreError = useAppStore((s) => s.coreError);
  const loadCore = useAppStore((s) => s.loadCore);
  const refreshPending = useAppStore((s) => s.refreshPending);
  const refreshNeedsYou = useAppStore((s) => s.refreshNeedsYou);

  useEffect(() => {
    loadCore();
    refreshPending();
    refreshNeedsYou();
  }, [loadCore, refreshPending, refreshNeedsYou]);

  useEffect(() => {
    let disposed = false;
    const unsubs: Array<() => void> = [];
    Promise.all([
      onEvent('approval.requested', () => {
        refreshPending();
        refreshNeedsYou();
      }),
      onEvent('approval.decided', () => {
        refreshPending();
        refreshNeedsYou();
      }),
      onEvent('approval.changed', () => {
        refreshPending();
        refreshNeedsYou();
      }),
      onEvent('task.updated', () => refreshNeedsYou()),
    ]).then((fns) => {
      if (disposed) fns.forEach((fn) => fn());
      else unsubs.push(...fns);
    });
    return () => {
      disposed = true;
      unsubs.forEach((fn) => fn());
    };
  }, [refreshPending, refreshNeedsYou]);

  // 运行完成/阻塞/审批的应用内提示，附带任务跳转。
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    onEvent<{ title?: string; body?: string; taskId?: string; kind?: string }>(
      'notify',
      (payload) => {
        if (!payload.title) return;
        const taskId = typeof payload.taskId === 'string' ? payload.taskId : '';
        const target = taskId
          ? `/tasks/${taskId}`
          : payload.kind === 'approval' || payload.kind === 'budget'
            ? '/approvals'
            : '';
        toast(payload.title, {
          description: payload.body,
          duration: 6000,
          action: target
            ? { label: t('查看', 'View'), onClick: () => navigate(target) }
            : undefined,
        });
      },
    ).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [navigate, t]);

  // 运行级警告（例如沙箱回退到进程隔离）在全局提示，避免只在任务页可见。
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    onEvent<RunDeltaPayload>('run.delta', (payload) => {
      if (payload.kind !== 'warning' || !payload.warning) return;
      const taskId = typeof payload.taskId === 'string' ? payload.taskId : '';
      toast.warning(t('运行警告', 'Run warning'), {
        description: payload.warning,
        duration: 8000,
        action: taskId
          ? {
              label: t('查看', 'View'),
              onClick: () => navigate(`/tasks/${taskId}`),
            }
          : undefined,
      });
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [navigate, t]);

  // 点击系统通知回到对应任务；Windows 只在 toast 仍显示时可上报点击。
  useEffect(() => {
    if (!isTauri) return;
    let disposed = false;
    let listener: { unregister: () => Promise<void> } | undefined;
    import('@tauri-apps/plugin-notification')
      .then(({ onAction }) =>
        onAction((event) => {
          const extra = (event.notification?.extra ?? {}) as Record<string, unknown>;
          const taskId = typeof extra.taskId === 'string' ? extra.taskId : '';
          const kind = typeof extra.kind === 'string' ? extra.kind : '';
          const target = taskId
            ? `/tasks/${taskId}`
            : kind === 'approval' || kind === 'budget'
              ? '/approvals'
              : '';
          if (!target) return;
          const win = getCurrentWindow();
          win.unminimize().catch(() => {});
          win.setFocus().catch(() => {});
          navigate(target);
        }),
      )
      .then((handle) => {
        if (disposed) handle.unregister();
        else listener = handle;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      listener?.unregister();
    };
  }, [navigate]);

  return (
    <div className="flex h-screen flex-col bg-base text-ink">
      {isTauri && <TitleBar />}
      <div className="flex min-h-0 flex-1">
        <aside className="flex w-[232px] shrink-0 flex-col border-r border-line bg-base">
          <div className="flex h-14 items-center px-4">
            <LumenWordmark height={34} />
          </div>

          <nav className="flex-1 overflow-y-auto px-2.5 py-2.5">
            {navGroups().map((group, index) => (
              <div key={group.label} className={index === 0 ? '' : 'mt-5'}>
                <p className="mb-1.5 px-2.5 text-xs font-semibold tracking-[0.08em] text-muted/70">
                  {group.label}
                </p>
                <div className="space-y-0.5">
                  {group.items.map((item) => (
                    <NavLink
                      key={item.to}
                      to={item.to}
                      end={item.end}
                      className={({ isActive }) =>
                        cn(
                          'group flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm transition-colors',
                          isActive
                            ? 'bg-hover font-semibold text-ink'
                            : 'text-muted hover:bg-hover hover:text-ink',
                        )
                      }
                    >
                      {({ isActive }) => (
                        <>
                          <item.icon
                            size={16}
                            strokeWidth={isActive ? 2.2 : 1.9}
                            className={isActive ? 'text-ink' : 'text-muted group-hover:text-ink'}
                          />
                          <span className="flex-1">{item.label}</span>
                          {item.badge && needsYou > 0 && (
                            <span className="min-w-[18px] rounded-full bg-soft2 px-1.5 text-center text-xs font-semibold leading-[17px] text-ink">
                              {needsYou}
                            </span>
                          )}
                        </>
                      )}
                    </NavLink>
                  ))}
                </div>
              </div>
            ))}
          </nav>

          <div className="border-t border-line p-2.5">
            <div className="rounded-xl bg-elevated px-3 py-2">
              <div className="flex items-center gap-1.5 text-xs text-ink">
                <span
                  className={cn(
                    'h-1.5 w-1.5 rounded-full',
                    coreError ? 'bg-danger' : !loaded ? 'bg-warn' : 'bg-success',
                  )}
                />
                {!isTauri
                  ? t('浏览器预览', 'Browser preview')
                  : coreError
                    ? t('本地服务连接失败', 'Local service unavailable')
                    : loaded
                      ? t('本地运行', 'Local runtime')
                      : t('正在连接', 'Connecting')}
              </div>
              <p className="mt-1 text-xs leading-4 text-muted">
                {!isTauri
                  ? t('使用本地 Mock 数据，操作会持久化', 'Uses local mock data; changes are persisted')
                  : coreError
                    ? coreError
                    : loaded
                      ? t(
                          'SQLite 数据与审批网关已连接',
                          'SQLite and the approval gateway are connected',
                        )
                      : t(
                          '正在初始化数据库与审批网关',
                          'Initializing the database and approval gateway',
                        )}
              </p>
            </div>
          </div>
        </aside>

        <main className="h-full min-h-0 min-w-0 flex-1 overflow-hidden">
          <Outlet />
        </main>
      </div>
      <UpdatePrompt />
    </div>
  );
}
