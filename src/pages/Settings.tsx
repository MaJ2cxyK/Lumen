import { lazy, Suspense, useCallback, useEffect, useState } from 'react';
import {
  AlertCircle,
  BarChart3,
  Bell,
  Check,
  Database,
  FolderOpen,
  Gauge,
  Globe,
  Info,
  KeyRound,
  Package,
  Palette,
  Plug,
  RefreshCw,
  Save,
  Shield,
  ShieldAlert,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Users,
  Wallet,
  Webhook,
  X,
  type LucideIcon,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import type { AppInfo, AppSettings, BackupFile, SandboxProbeReport } from '../lib/types';
import { formatDateTime } from '../lib/format';
import { tr, useI18n } from '../lib/i18n';
import {
  checkForUpdate,
  installUpdate,
  updaterSupported,
  type UpdateInfo,
  type UpdateProgress,
} from '../lib/updater';
import {
  readUpdatePrefs,
  writeUpdatePrefs,
  type UpdatePrefs,
} from '../lib/updatePrefs';
import { ProvidersPanel } from '../components/Providers';
import { useAppStore } from '../store';
import type { AccentTheme, ThemeMode } from '../store';
import {
  Button,
  Card,
  ConfirmDialog,
  Empty,
  Field,
  Input,
  Loading,
  PageHeader,
  PageScroll,
  SectionTitle,
  Select,
  Toggle,
  cn,
} from '../components/ui';

const TeamsPanel = lazy(() => import('./Teams'));
const ExtensionsPanel = lazy(() => import('./Extensions'));
const UsagePanel = lazy(() => import('./Usage'));
const BudgetsPanel = lazy(() => import('./Budgets'));

type ProbeCheckKey =
  | 'workRead'
  | 'workWrite'
  | 'existingRead'
  | 'outsideReadBlocked'
  | 'userReadBlocked'
  | 'outsideWriteBlocked'
  | 'homeRedirected'
  | 'relayEcho'
  | 'stdioForwarded';

const PROBE_CHECKS: { key: ProbeCheckKey; label: string }[] = [
  {
    key: 'workRead',
    get label() {
      return tr('工作目录可读', 'Work directory readable');
    },
  },
  {
    key: 'workWrite',
    get label() {
      return tr('工作目录可写', 'Work directory writable');
    },
  },
  {
    key: 'existingRead',
    get label() {
      return tr('既有文件可读', 'Existing files readable');
    },
  },
  {
    key: 'outsideReadBlocked',
    get label() {
      return tr('外部目录读取被拦截', 'Outside directory reads blocked');
    },
  },
  {
    key: 'userReadBlocked',
    get label() {
      return tr('用户目录读取被拦截', 'User directory reads blocked');
    },
  },
  {
    key: 'outsideWriteBlocked',
    get label() {
      return tr('外部路径写入被拦截', 'Outside path writes blocked');
    },
  },
  {
    key: 'homeRedirected',
    get label() {
      return tr('HOME/TEMP 重定向', 'HOME/TEMP redirection');
    },
  },
  {
    key: 'relayEcho',
    get label() {
      return tr('本地代理回路', 'Local proxy loopback');
    },
  },
  {
    key: 'stdioForwarded',
    get label() {
      return tr('标准输出转发', 'Stdout forwarding');
    },
  },
];

const ACCENTS: {
  key: AccentTheme;
  label: string;
  hint: string;
  light: { bg: string; spot: string };
  dark: { bg: string; spot: string };
}[] = [
  {
    key: 'forest',
    get label() {
      return tr('森林', 'Forest');
    },
    get hint() {
      return tr('鼠尾草绿', 'Sage green');
    },
    light: { bg: '#eef2ee', spot: '#4f7a63' },
    dark: { bg: '#212420', spot: '#86b79b' },
  },
  {
    key: 'mint',
    get label() {
      return tr('薄荷', 'Mint');
    },
    get hint() {
      return tr('青绿色', 'Teal');
    },
    light: { bg: '#e9f1f0', spot: '#3f7f79' },
    dark: { bg: '#1e2422', spot: '#7fc0b8' },
  },
  {
    key: 'honey',
    get label() {
      return tr('蜜蜂', 'Honey');
    },
    get hint() {
      return tr('暖金色', 'Warm gold');
    },
    light: { bg: '#f7f0dd', spot: '#8a6a2f' },
    dark: { bg: '#24221b', spot: '#d9b979' },
  },
  {
    key: 'parchment',
    get label() {
      return tr('羊皮纸', 'Parchment');
    },
    get hint() {
      return tr('暖棕色', 'Warm brown');
    },
    light: { bg: '#f4ede2', spot: '#7a6248' },
    dark: { bg: '#24201a', spot: '#c8ae8c' },
  },
];

type SettingsSection =
  | 'general'
  | 'runtime'
  | 'security'
  | 'models'
  | 'appearance'
  | 'data'
  | 'mcp'
  | 'hooks'
  | 'skills'
  | 'plugins'
  | 'teams'
  | 'usage'
  | 'budgets';

function navItem(
  key: string,
  zh: string,
  en: string,
  icon: LucideIcon,
  section: SettingsSection,
) {
  return {
    key,
    get label() {
      return tr(zh, en);
    },
    icon,
    section,
  };
}

const SETTINGS_NAV: {
  group: string;
  items: {
    key: string;
    label: string;
    icon: LucideIcon;
    section: SettingsSection;
  }[];
}[] = [
  {
    get group() {
      return tr('应用', 'Application');
    },
    items: [
      navItem('general', '通用', 'General', Info, 'general'),
      navItem('appearance', '外观', 'Appearance', Palette, 'appearance'),
    ],
  },
  {
    get group() {
      return tr('运行', 'Runtime');
    },
    items: [
      navItem('runtime', '运行参数', 'Run settings', SlidersHorizontal, 'runtime'),
      navItem('security', '安全与沙箱', 'Security & sandbox', ShieldCheck, 'security'),
      navItem('models', '模型与代理', 'Models & proxy', KeyRound, 'models'),
    ],
  },
  {
    get group() {
      return tr('扩展', 'Extensions');
    },
    items: [
      navItem('mcp', 'MCP', 'MCP', Plug, 'mcp'),
      navItem('hooks', 'Hooks', 'Hooks', Webhook, 'hooks'),
      navItem('skills', 'Skills', 'Skills', Sparkles, 'skills'),
      navItem('plugins', '插件', 'Plugins', Package, 'plugins'),
    ],
  },
  {
    get group() {
      return tr('组织与成本', 'Team & cost');
    },
    items: [
      navItem('teams', '团队与席位', 'Teams & seats', Users, 'teams'),
      navItem('usage', '用量', 'Usage', BarChart3, 'usage'),
      navItem('budgets', '预算', 'Budgets', Wallet, 'budgets'),
    ],
  },
  {
    get group() {
      return tr('系统', 'System');
    },
    items: [navItem('data', '数据与工具', 'Data & tools', Database, 'data')],
  },
];

function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

export default function Settings() {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [savedJson, setSavedJson] = useState('');
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [probing, setProbing] = useState(false);
  const [probe, setProbe] = useState<SandboxProbeReport | null>(null);
  const [backups, setBackups] = useState<BackupFile[]>([]);
  const [backupLoading, setBackupLoading] = useState(false);
  const [restoring, setRestoring] = useState<BackupFile | null>(null);
  const [restoreLoading, setRestoreLoading] = useState(false);
  const [section, setSection] = useState<SettingsSection>('general');
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const accent = useAppStore((s) => s.accent);
  const setAccent = useAppStore((s) => s.setAccent);
  const { lang, setLang, t } = useI18n();
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateChecked, setUpdateChecked] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [updateInstalling, setUpdateInstalling] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<UpdateProgress | null>(null);
  const [updatePrefs, setUpdatePrefs] = useState<UpdatePrefs>(() => readUpdatePrefs());

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [settingsData, infoData] = await Promise.all([
        api.settingsGet(),
        api.appInfo(),
      ]);
      setSettings(settingsData);
      setSavedJson(JSON.stringify(settingsData));
      setInfo(infoData);
      setError(null);
    } catch (err) {
      setError(errorMessage(err));
      toast.error(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const loadBackups = useCallback(async () => {
    try {
      setBackups(await api.backupsList());
    } catch {
      /* 备份列表失败不影响设置页其他功能 */
    }
  }, []);

  useEffect(() => {
    loadBackups();
  }, [loadBackups]);

  const dirty = Boolean(settings) && JSON.stringify(settings) !== savedJson;

  async function save() {
    if (!settings) return;
    setSaving(true);
    try {
      await api.settingsSave(settings);
      setSavedJson(JSON.stringify(settings));
      toast.success(t('设置已保存', 'Settings saved'));
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  async function runUpdateCheck() {
    if (updateChecking) return;
    setUpdateChecking(true);
    try {
      const found = await checkForUpdate({ channel: updatePrefs.channel });
      setUpdateInfo(found);
      setUpdateChecked(true);
      if (found) {
        toast.success(t(`发现新版本 ${found.version}`, `Version ${found.version} available`));
      } else {
        toast.success(t('当前已是最新版本', 'You are on the latest version'));
      }
    } catch (err) {
      setUpdateChecked(false);
      toast.error(errorMessage(err));
    } finally {
      setUpdateChecking(false);
    }
  }

  async function runUpdateInstall() {
    if (updateInstalling) return;
    setUpdateInstalling(true);
    setUpdateProgress({ downloaded: 0, total: null });
    try {
      await installUpdate(setUpdateProgress);
      toast.success(
        t('更新已安装，应用将重新启动', 'Update installed; the app will restart'),
      );
      setUpdateInfo(null);
      setUpdateChecked(false);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setUpdateInstalling(false);
    }
  }

  function changeUpdatePrefs(patch: Partial<UpdatePrefs>) {
    setUpdatePrefs(writeUpdatePrefs(patch));
    if (patch.channel) {
      setUpdateChecked(false);
      setUpdateInfo(null);
    }
  }

  async function openDataDir() {
    try {
      await api.openDataDir();
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function backupData() {
    setBackupLoading(true);
    try {
      const path = await api.dataBackup();
      await loadBackups();
      toast.success(t(`数据库已备份到 ${path}`, `Database backed up to ${path}`));
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBackupLoading(false);
    }
  }

  async function openBackupDir() {
    try {
      await api.openBackupDir();
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function restoreBackup() {
    if (!restoring) return;
    setRestoreLoading(true);
    try {
      await api.dataRestore(restoring.name);
      setRestoring(null);
      toast.success(t('备份已恢复，正在重新加载数据', 'Backup restored; reloading data'));
      window.setTimeout(() => window.location.reload(), 600);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setRestoreLoading(false);
    }
  }

  async function exportData() {
    try {
      const path = await api.dataExport();
      toast.success(t(`数据已导出到 ${path}`, `Data exported to ${path}`));
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function runProbe() {
    setProbing(true);
    try {
      const report = await api.sandboxProbe();
      setProbe(report);
      if (report.ok) {
        toast.success(
          t(
            `沙箱自检通过（${report.elapsedMs}ms）`,
            `Sandbox self-test passed (${report.elapsedMs}ms)`,
          ),
        );
      } else {
        toast.error(
          t('沙箱自检未通过，请查看结果详情', 'Sandbox self-test failed; see the details'),
        );
      }
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setProbing(false);
    }
  }

  const standalone =
    section === 'general' ||
    section === 'runtime' ||
    section === 'security' ||
    section === 'models' ||
    section === 'appearance' ||
    section === 'data';

  return (
    <PageScroll>
      <PageHeader
        title={t('设置', 'Settings')}
        actions={
          <Button
            size="sm"
            loading={saving}
            onClick={save}
            disabled={!settings || !dirty}
            title={dirty ? t('保存修改', 'Save changes') : t('没有未保存的修改', 'No unsaved changes')}
          >
            <Save size={13} />
            {t('保存设置', 'Save settings')}
          </Button>
        }
      />

      {loading ? (
        <Loading />
      ) : error && !settings ? (
        <Empty
          icon={AlertCircle}
          title={t('无法加载设置', 'Unable to load settings')}
          description={error}
          action={
            <Button size="sm" variant="outline" onClick={() => load()}>
              {t('重试', 'Retry')}
            </Button>
          }
        />
      ) : !settings ? (
        <Loading label={t('暂无设置数据', 'No settings data')} />
      ) : (
        <div className="flex flex-col gap-8 lg:flex-row lg:items-start">
          <aside className="shrink-0 lg:sticky lg:top-7 lg:w-[190px]">
            <nav className="space-y-6">
              {SETTINGS_NAV.map((group) => (
                <div key={group.group}>
                  <p className="mb-1.5 px-3 text-xs font-semibold tracking-wide text-muted/60">
                    {group.group}
                  </p>
                  <div className="space-y-0.5">
                    {group.items.map((item) => {
                      const active = section === item.section;
                      return (
                        <button
                          key={item.key}
                          type="button"
                          onClick={() => setSection(item.section)}
                          className={cn(
                            'flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm transition-colors',
                            active
                              ? 'bg-hover font-semibold text-ink'
                              : 'text-muted hover:bg-hover hover:text-ink',
                          )}
                        >
                          <item.icon
                            size={16}
                            className={active ? 'text-ink' : 'text-muted'}
                          />
                          {item.label}
                        </button>
                      );
                    })}
                  </div>
                </div>
              ))}
            </nav>
          </aside>

          <div
            className={cn(
              'min-w-0 flex-1 space-y-6',
              standalone && 'lg:max-w-[820px]',
            )}
          >
            <Suspense fallback={<Loading />}>
              {section === 'teams' && <TeamsPanel embedded />}
              {section === 'mcp' && <ExtensionsPanel embedded fixedKind="mcp" />}
              {section === 'hooks' && <ExtensionsPanel embedded fixedKind="hook" />}
              {section === 'skills' && <ExtensionsPanel embedded fixedKind="skill" />}
              {section === 'plugins' && <ExtensionsPanel embedded fixedKind="plugin" />}
              {section === 'usage' && <UsagePanel embedded />}
              {section === 'budgets' && <BudgetsPanel embedded />}
            </Suspense>

            {section === 'runtime' && (
            <Card>
              <SectionTitle
                icon={SlidersHorizontal}
                title={t('运行参数', 'Run settings')}
                description={t(
                  '并发上限、超时与转录保留策略',
                  'Concurrency limits, timeouts and transcript retention',
                )}
              />
              <div className="space-y-3">
                <Field
                  label={t('最大并发运行数', 'Max concurrent runs')}
                  hint={t(
                    '同时执行的 Agent 进程上限，超出后任务排队',
                    'Upper bound on simultaneous agent processes; extra tasks queue up',
                  )}
                >
                  <Input
                    type="number"
                    min={1}
                    value={settings.maxConcurrentRuns}
                    onChange={(e) =>
                      setSettings({
                        ...settings,
                        maxConcurrentRuns: Number(e.target.value),
                      })
                    }
                    className="font-mono text-xs"
                  />
                </Field>
                <Field
                  label={t('审批超时（秒）', 'Approval timeout (seconds)')}
                  hint={t(
                    '超过该时长未处理的审批将自动过期',
                    'Approvals left unanswered past this duration expire automatically',
                  )}
                >
                  <Input
                    type="number"
                    min={30}
                    value={settings.approvalTimeoutSecs}
                    onChange={(e) =>
                      setSettings({
                        ...settings,
                        approvalTimeoutSecs: Number(e.target.value),
                      })
                    }
                    className="font-mono text-xs"
                  />
                </Field>
                <Field
                  label={t('转录保留天数', 'Transcript retention (days)')}
                  hint={t(
                    '超过期限的会话转录会被清理',
                    'Session transcripts older than this are cleaned up',
                  )}
                >
                  <Input
                    type="number"
                    min={1}
                    value={settings.transcriptRetentionDays}
                    onChange={(e) =>
                      setSettings({
                        ...settings,
                        transcriptRetentionDays: Number(e.target.value),
                      })
                    }
                    className="font-mono text-xs"
                  />
                </Field>
                <Field
                  label={t('单次运行时长上限（分钟）', 'Run time limit (minutes)')}
                  hint={t(
                    '0 表示不限制；超时后由看门狗终止整棵进程树',
                    '0 means no limit; the watchdog terminates the whole process tree when exceeded',
                  )}
                >
                  <Input
                    type="number"
                    min={0}
                    value={settings.runMaxMinutes}
                    onChange={(e) =>
                      setSettings({
                        ...settings,
                        runMaxMinutes: Number(e.target.value),
                      })
                    }
                    className="font-mono text-xs"
                  />
                </Field>
              </div>
            </Card>
            )}

            {section === 'security' && (
            <Card>
              <SectionTitle
                icon={ShieldCheck}
                title={t('安全与审批', 'Security & approvals')}
                description={t(
                  '代理、Hook 与文件隔离策略',
                  'Proxy, hook and file isolation policies',
                )}
              />
              <div className="space-y-3">
                <div className="flex items-center justify-between rounded-lg border border-line bg-base/40 px-3 py-2.5">
                  <div className="flex items-start gap-2">
                    <Shield size={14} className="mt-0.5 text-muted" />
                    <div>
                      <p className="text-xs text-ink">
                        {t('本地模型代理', 'Local model proxy')}
                      </p>
                      <p className="text-xs text-muted">
                        {t(
                          '真实 API Key 留在主进程，子 Agent 只拿到回环地址与一次性令牌',
                          'The real API key stays in the main process; child agents only receive a loopback address and a one-time token',
                        )}
                      </p>
                    </div>
                  </div>
                  <Toggle
                    checked={settings.proxyEnabled}
                    onChange={(value) => setSettings({ ...settings, proxyEnabled: value })}
                    label={t('本地模型代理', 'Local model proxy')}
                  />
                </div>
                <div className="flex items-center justify-between rounded-lg border border-line bg-base/40 px-3 py-2.5">
                  <div className="flex items-start gap-2">
                    <ShieldAlert size={14} className="mt-0.5 text-muted" />
                    <div>
                      <p className="text-xs text-ink">
                        {t('严格 Hook 模式', 'Strict hook mode')}
                      </p>
                      <p className="text-xs text-muted">
                        {t(
                          '检测到未经审批网关的工具调用时，直接判定运行失败',
                          'Fail the run immediately when a tool call bypasses the approval gate',
                        )}
                      </p>
                    </div>
                  </div>
                  <Toggle
                    checked={settings.hookStrict}
                    onChange={(value) => setSettings({ ...settings, hookStrict: value })}
                    label={t('严格 Hook 模式', 'Strict hook mode')}
                  />
                </div>
                <div className="flex items-center justify-between rounded-lg border border-line bg-base/40 px-3 py-2.5">
                  <div className="flex items-start gap-2">
                    <ShieldCheck size={14} className="mt-0.5 text-muted" />
                    <div>
                      <p className="text-xs text-ink">
                        {t('非 Claude 运行前授权', 'Approval before non-Claude runs')}
                      </p>
                      <p className="text-xs text-muted">
                        {t(
                          '无工具级 Hook 的适配器在每次运行前需人工放行',
                          'Adapters without tool-level hooks require manual approval before each run',
                        )}
                      </p>
                    </div>
                  </div>
                  <Toggle
                    checked={settings.nonClaudeRunApproval}
                    onChange={(value) =>
                      setSettings({ ...settings, nonClaudeRunApproval: value })
                    }
                    label={t('非 Claude 运行前授权', 'Approval before non-Claude runs')}
                  />
                </div>
                <div className="flex items-center justify-between rounded-lg border border-line bg-base/40 px-3 py-2.5">
                  <div className="flex items-start gap-2">
                    <Shield size={14} className="mt-0.5 text-muted" />
                    <div>
                      <p className="text-xs text-ink">
                        {t('AppContainer 文件隔离', 'AppContainer file isolation')}
                      </p>
                      <p className="text-xs text-muted">
                        {t(
                          '默认开启。Agent 仅能访问项目目录与沙箱 HOME；Git 工作区或未启用模型代理时会回退进程隔离',
                          'On by default. Agents can only access the project directory and sandbox HOME; Git workspaces or a disabled model proxy fall back to process isolation',
                        )}
                      </p>
                    </div>
                  </div>
                  <Toggle
                    checked={settings.sandboxEnabled}
                    onChange={(value) =>
                      setSettings({ ...settings, sandboxEnabled: value })
                    }
                    label={t('AppContainer 文件隔离', 'AppContainer file isolation')}
                  />
                </div>
                <div className="flex items-center justify-between rounded-lg border border-line bg-base/40 px-3 py-2.5">
                  <div className="flex items-start gap-2">
                    <Bell size={14} className="mt-0.5 text-muted" />
                    <div>
                      <p className="text-xs text-ink">
                        {t('系统通知', 'System notifications')}
                      </p>
                      <p className="text-xs text-muted">
                        {t(
                          '审批请求、预算预警与任务完成/阻塞推送系统通知',
                          'Push system notifications for approvals, budget alerts, and tasks completing or getting blocked',
                        )}
                      </p>
                    </div>
                  </div>
                  <Toggle
                    checked={settings.notify}
                    onChange={(value) => setSettings({ ...settings, notify: value })}
                    label={t('系统通知', 'System notifications')}
                  />
                </div>
              </div>
            </Card>
            )}

            {section === 'data' && (
            <>
            <Card>
              <SectionTitle
                icon={Info}
                title={t('应用信息', 'App info')}
                description={t(
                  '版本、数据目录与数据库位置',
                  'Version, data directory and database location',
                )}
              />
              {!info ? (
                <Loading />
              ) : (
                <div className="space-y-2.5">
                  <InfoRow label={t('版本', 'Version')} value={`v${info.version}`} />
                  <InfoRow label={t('平台', 'Platform')} value={info.platform} />
                  <InfoRow
                    label={t('数据目录', 'Data directory')}
                    value={info.dataDir}
                    mono
                  />
                  <InfoRow
                    label={t('数据库', 'Database')}
                    value={info.dbPath}
                    mono
                  />
                  <InfoRow
                    label={t('Gate 端口', 'Gate port')}
                    value={String(info.gatePort)}
                    mono
                  />
                  <div className="flex flex-wrap gap-2 pt-1">
                    <Button variant="outline" size="sm" onClick={openDataDir}>
                      <FolderOpen size={13} />
                      {t('打开数据目录', 'Open data directory')}
                    </Button>
                  </div>
                </div>
              )}
              <div className="mt-4 flex items-start gap-2 rounded-lg border border-line/60 bg-base/40 px-3 py-2">
                <Database size={13} className="mt-0.5 shrink-0 text-muted" />
                <p className="text-xs leading-4 text-muted">
                  {t(
                    '所有项目、任务、用量与审批数据保存在本地 SQLite，不会上传到云端。',
                    'All project, task, usage and approval data is stored in a local SQLite database and never uploaded.',
                  )}
                </p>
              </div>
            </Card>

            <Card>
              <SectionTitle
                icon={Database}
                title={t('数据备份', 'Data backup')}
                description={t(
                  '立即备份、恢复历史版本或导出 JSON',
                  'Back up now, restore an earlier version, or export JSON',
                )}
              />
              <div className="flex flex-wrap gap-2">
                <Button size="sm" loading={backupLoading} onClick={backupData}>
                  <Database size={13} />
                  {t('立即备份', 'Back up now')}
                </Button>
                <Button variant="outline" size="sm" onClick={openBackupDir}>
                  <FolderOpen size={13} />
                  {t('打开备份目录', 'Open backup folder')}
                </Button>
                <Button variant="outline" size="sm" onClick={exportData}>
                  <Save size={13} />
                  {t('导出 JSON', 'Export JSON')}
                </Button>
              </div>

              <div className="mt-4 flex items-center justify-between">
                <p className="text-xs font-medium text-muted">
                  {t('可用备份', 'Available backups')}
                </p>
                <Button variant="ghost" size="xs" onClick={loadBackups}>
                  {t('刷新', 'Refresh')}
                </Button>
              </div>
              {backups.length === 0 ? (
                <p className="mt-2 rounded-lg border border-dashed border-line/70 px-3 py-4 text-center text-xs text-muted">
                  {t(
                    '暂无备份，点击「立即备份」生成第一份。',
                    'No backups yet. Use “Back up now” to create the first one.',
                  )}
                </p>
              ) : (
                <div className="mt-2 divide-y divide-line/60 rounded-lg border border-line/60">
                  {backups.map((item) => (
                    <div key={item.name} className="flex items-center gap-3 px-3 py-2">
                      <Database size={13} className="shrink-0 text-muted" />
                      <div className="min-w-0 flex-1">
                        <p className="truncate font-mono text-xs text-ink">{item.name}</p>
                        <p className="text-[11px] text-muted">
                          {formatDateTime(item.createdAt)} · {formatBytes(item.size)}
                        </p>
                      </div>
                      <Button
                        variant="outline"
                        size="xs"
                        onClick={() => setRestoring(item)}
                      >
                        {t('恢复', 'Restore')}
                      </Button>
                    </div>
                  ))}
                </div>
              )}
              <p className="mt-3 text-xs leading-4 text-muted">
                {t(
                  '手动备份保留最近 7 份；应用启动时自动轮转，若 24 小时内没有备份会补一份。恢复前会先为当前数据库生成安全副本。',
                  'Manual backups keep the latest 7. Rotation runs at startup and adds one if there was no backup in the past 24 hours. A safety copy of the current database is made before restoring.',
                )}
              </p>
            </Card>
            </>
            )}

            {section === 'security' && (
            <Card>
              <SectionTitle
                icon={ShieldCheck}
                title={t('沙箱隔离自检', 'Sandbox isolation self-test')}
                description={t(
                  '以 AppContainer 启动子进程，验证文件隔离、环境重定向与代理回路',
                  'Launch a child process under AppContainer to verify file isolation, environment redirection and the proxy loopback',
                )}
              />
            <div className="flex flex-wrap items-center gap-2">
              <Button variant="outline" size="sm" loading={probing} onClick={runProbe}>
                <Shield size={13} />
                {t('开始自检', 'Start self-test')}
              </Button>
              {probe && (
                <span
                  className={`text-xs ${probe.ok ? 'text-primary' : 'text-danger'}`}
                >
                  {probe.ok
                    ? t(`通过（${probe.elapsedMs}ms）`, `Passed (${probe.elapsedMs}ms)`)
                    : t('未通过', 'Failed')}
                </span>
              )}
            </div>
            {probe && (
              <>
                <div className="mt-3 grid grid-cols-1 gap-1.5 sm:grid-cols-2">
                  {PROBE_CHECKS.map(({ key, label }) => {
                    const passed = probe[key];
                    return (
                      <div key={key} className="flex items-center gap-1.5">
                        {passed ? (
                          <Check size={12} className="text-primary" />
                        ) : (
                          <X size={12} className="text-danger" />
                        )}
                        <span className={`text-xs ${passed ? 'text-ink' : 'text-danger'}`}>
                          {label}
                        </span>
                      </div>
                    );
                  })}
                </div>
                <p className="mt-2 text-xs leading-4 text-muted">{probe.detail}</p>
              </>
            )}
          </Card>
            )}

            {section === 'general' && (
            <>
            <Card>
              <SectionTitle
                icon={Globe}
                title={t('语言', 'Language')}
                description={t(
                  '界面语言切换后立即生效，并记住你的选择',
                  'The UI language applies immediately and is remembered',
                )}
              />
              <div className="flex items-start justify-between gap-3">
                <div>
                  <p className="text-sm font-medium text-ink">
                    {t('界面语言', 'Interface language')}
                  </p>
                  <p className="mt-0.5 text-xs text-muted">
                    {t(
                      '支持简体中文与英文，覆盖导航、页面文案与状态标签',
                      'Simplified Chinese and English across navigation, pages and status labels',
                    )}
                  </p>
                </div>
                <Select
                  value={lang}
                  onChange={(event) => setLang(event.target.value === 'en' ? 'en' : 'zh')}
                  className="h-8 w-[128px] shrink-0 py-0 text-xs"
                  aria-label={t('界面语言', 'Interface language')}
                >
                  <option value="zh">简体中文</option>
                  <option value="en">English</option>
                </Select>
              </div>
            </Card>

            <Card>
              <SectionTitle
                icon={Info}
                title={t('关于与更新', 'About & updates')}
                description={t(
                  '检查新版本并安装；未配置更新源时会明确提示',
                  'Check for new versions; missing update sources are reported clearly',
                )}
              />
              <div className="flex items-start justify-between gap-3">
                <div>
                  <p className="text-sm font-medium text-ink">
                    Lumen {info ? `v${info.version}` : ''}
                  </p>
                  <p className="mt-0.5 text-xs text-muted">
                    {updaterSupported()
                      ? t('当前平台支持自动更新', 'Automatic updates are available')
                      : t('浏览器预览不支持检查更新', 'Update checks are only available in the desktop app')}
                  </p>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  loading={updateChecking}
                  disabled={!updaterSupported() || updateInstalling}
                  onClick={runUpdateCheck}
                >
                  {!updateChecking && <RefreshCw size={13} />}
                  {t('检查更新', 'Check for updates')}
                </Button>
              </div>

              <div className="mt-3 space-y-2.5 border-t border-line/50 pt-3">
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <p className="text-xs text-ink">
                      {t('自动检查更新', 'Automatically check for updates')}
                    </p>
                    <p className="mt-0.5 text-xs leading-4 text-muted">
                      {t(
                        '启动后及每 6 小时检查一次；自动检查失败不会打扰',
                        'Checks after launch and every 6 hours; failures stay silent',
                      )}
                    </p>
                  </div>
                  <Toggle
                    checked={updatePrefs.autoCheck}
                    onChange={(value) => changeUpdatePrefs({ autoCheck: value })}
                    label={t('自动检查更新', 'Automatically check for updates')}
                  />
                </div>
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <p className="text-xs text-ink">{t('更新渠道', 'Update channel')}</p>
                    <p className="mt-0.5 text-xs leading-4 text-muted">
                      {t(
                        '渠道通过请求头传给更新源，由清单决定版本',
                        'The channel is sent to the update source, which decides the version',
                      )}
                    </p>
                  </div>
                  <Select
                    value={updatePrefs.channel}
                    onChange={(event) =>
                      changeUpdatePrefs({
                        channel: event.target.value === 'beta' ? 'beta' : 'stable',
                      })
                    }
                    className="h-8 w-[120px] shrink-0 py-0 text-xs"
                    aria-label={t('更新渠道', 'Update channel')}
                  >
                    <option value="stable">{t('稳定版', 'Stable')}</option>
                    <option value="beta">Beta</option>
                  </Select>
                </div>
                {updatePrefs.skippedVersion && (
                  <div className="flex items-center justify-between gap-3 rounded-lg border border-line/60 bg-base/40 px-3 py-2">
                    <p className="text-xs text-muted">
                      {t(
                        `已跳过版本 v${updatePrefs.skippedVersion}`,
                        `Skipped version v${updatePrefs.skippedVersion}`,
                      )}
                    </p>
                    <Button
                      variant="outline"
                      size="xs"
                      onClick={() => changeUpdatePrefs({ skippedVersion: null })}
                    >
                      {t('恢复提示', 'Show again')}
                    </Button>
                  </div>
                )}
              </div>

              {updateChecked && !updateInfo && (
                <p className="mt-3 text-xs text-success">
                  {t('已是最新版本', 'You are on the latest version')}
                </p>
              )}

              {updateInfo && (
                <div className="mt-3 rounded-lg border border-line bg-soft/60 p-3">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div>
                      <p className="text-xs font-semibold text-ink">
                        {t(
                          `新版本 ${updateInfo.version}`,
                          `Version ${updateInfo.version} available`,
                        )}
                      </p>
                      <p className="mt-0.5 text-xs text-muted">
                        {t(
                          `当前版本 ${updateInfo.currentVersion}`,
                          `Current version ${updateInfo.currentVersion}`,
                        )}
                      </p>
                    </div>
                    <Button
                      size="xs"
                      loading={updateInstalling}
                      onClick={runUpdateInstall}
                    >
                      {t('下载并安装', 'Download & install')}
                    </Button>
                  </div>
                  {updateInfo.notes && (
                    <p className="mt-2 whitespace-pre-wrap text-xs leading-5 text-muted">
                      {updateInfo.notes}
                    </p>
                  )}
                  {updateInstalling && (
                    <p className="mt-2 text-xs text-muted">
                      {updateProgress && updateProgress.total
                        ? t(
                            `已下载 ${Math.round((updateProgress.downloaded / updateProgress.total) * 100)}%`,
                            `Downloaded ${Math.round((updateProgress.downloaded / updateProgress.total) * 100)}%`,
                          )
                        : t('正在下载更新…', 'Downloading update…')}
                    </p>
                  )}
                </div>
              )}
            </Card>
            </>
            )}

            {section === 'appearance' && (
            <Card>
              <SectionTitle
                icon={Palette}
                title={t('外观', 'Appearance')}
                description={t(
                  '主题色与明暗模式，切换后立即生效',
                  'Accent color and light/dark mode; changes apply instantly',
                )}
              />
              <div>
                <div className="flex items-start justify-between gap-3">
                  <div>
                    <p className="text-sm font-medium text-ink">
                      {t('主题色', 'Accent color')}
                    </p>
                    <p className="mt-0.5 text-xs text-muted">
                      {t(
                        '影响按钮、链接与选中状态的强调色',
                        'Accent used for buttons, links and selected states',
                      )}
                    </p>
                  </div>
                  <Select
                    value={theme}
                    onChange={(event) => setTheme(event.target.value as ThemeMode)}
                    className="h-8 w-[104px] shrink-0 py-0 text-xs"
                    aria-label={t('明暗模式', 'Color mode')}
                  >
                    <option value="light">{t('浅色', 'Light')}</option>
                    <option value="dark">{t('深色', 'Dark')}</option>
                  </Select>
                </div>

                <div className="mt-3 grid grid-cols-2 gap-3 sm:grid-cols-4">
                  {ACCENTS.map((item) => {
                    const active = accent === item.key;
                    const palette = theme === 'dark' ? item.dark : item.light;
                    return (
                      <button
                        key={item.key}
                        type="button"
                        onClick={() => setAccent(item.key)}
                        title={item.hint}
                        className={cn(
                          'rounded-lg border p-1.5 transition-colors',
                          active
                            ? 'border-primary/70'
                            : 'border-line hover:border-muted/40',
                        )}
                      >
                        <div
                          className="relative rounded-md border p-2.5"
                          style={{
                            backgroundColor: palette.bg,
                            borderColor:
                              theme === 'dark' ? 'rgba(255,255,255,0.06)' : 'rgba(0,0,0,0.05)',
                          }}
                        >
                          <span
                            className="block h-1.5 w-9 rounded-full"
                            style={{ backgroundColor: palette.spot }}
                          />
                          <span
                            className={cn(
                              'mt-2 block h-1 w-14 rounded-full',
                              theme === 'dark' ? 'bg-white/15' : 'bg-black/10',
                            )}
                          />
                          <span
                            className={cn(
                              'mt-1.5 block h-1 w-11 rounded-full',
                              theme === 'dark' ? 'bg-white/10' : 'bg-black/[0.07]',
                            )}
                          />
                          {active && (
                            <span className="absolute -right-1.5 -top-1.5 flex h-4 w-4 items-center justify-center rounded-full bg-primary text-white ring-2 ring-card">
                              <Check size={10} strokeWidth={3} />
                            </span>
                          )}
                        </div>
                        <p
                          className={cn(
                            'mt-1.5 pb-0.5 text-center text-xs',
                            active ? 'font-medium text-ink' : 'text-muted',
                          )}
                        >
                          {item.label}
                        </p>
                      </button>
                    );
                  })}
                </div>
              </div>
            </Card>
            )}

            {section === 'models' && <ProvidersPanel />}

            {section === 'runtime' && (
            <Card>
              <SectionTitle
                icon={Gauge}
                title={t('当前限制', 'Current limits')}
                description={t(
                  '保存后生效的并发、超时与保留摘要',
                  'Concurrency, timeout and retention summary applied after saving',
                )}
              />
            <div className="grid grid-cols-2 gap-3 text-xs text-muted sm:grid-cols-4">
              <div>
                <p>{t('并发', 'Concurrency')}</p>
                <p className="mt-0.5 font-mono text-sm text-ink">
                  {t(`${settings.maxConcurrentRuns} 个`, `${settings.maxConcurrentRuns} runs`)}
                </p>
              </div>
              <div>
                <p>{t('审批超时', 'Approval timeout')}</p>
                <p className="mt-0.5 font-mono text-sm text-ink">
                  {settings.approvalTimeoutSecs}s
                </p>
              </div>
              <div>
                <p>{t('转录保留', 'Transcript retention')}</p>
                <p className="mt-0.5 font-mono text-sm text-ink">
                  {t(
                    `${settings.transcriptRetentionDays} 天`,
                    `${settings.transcriptRetentionDays} days`,
                  )}
                </p>
              </div>
              <div>
                <p>{t('通知', 'Notifications')}</p>
                <p className="mt-0.5 font-mono text-sm text-ink">
                  {settings.notify ? t('已开启', 'On') : t('已关闭', 'Off')}
                </p>
              </div>
            </div>
          </Card>
            )}
          </div>
        </div>
      )}
      <ConfirmDialog
        open={Boolean(restoring)}
        onClose={() => setRestoring(null)}
        onConfirm={restoreBackup}
        title={t(
          `恢复备份「${restoring?.name ?? ''}」`,
          `Restore backup “${restoring?.name ?? ''}”`,
        )}
        description={t(
          '将用该备份覆盖当前数据库，当前数据会全部替换。恢复前会自动生成一份当前数据库的安全副本。',
          'This backup will overwrite the current database and replace all current data. A safety copy of the current database is created automatically before restoring.',
        )}
        hint={t(
          '恢复完成后应用会自动重新加载；正在运行的 Agent 会被拒绝恢复。',
          'The app reloads automatically afterwards. Restoring is refused while agents are running.',
        )}
        confirmText={t('恢复', 'Restore')}
        loading={restoreLoading}
      />
    </PageScroll>
  );
}

function InfoRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="flex items-start justify-between gap-3 border-b border-line/40 pb-2 last:border-0">
      <span className="shrink-0 text-xs text-muted">{label}</span>
      <span
        className={`max-w-[72%] break-all text-right text-xs text-ink/85 ${
          mono ? 'font-mono' : ''
        }`}
      >
        {value}
      </span>
    </div>
  );
}
