import { useCallback, useEffect, useState } from 'react';
import { ArrowUpCircle, Check, Download, Minus, Pencil, Plus, RefreshCw, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import type { Agent, DetectedCli, Provider } from '../lib/types';
import { ADAPTERS } from '../lib/labels';
import { tr, useI18n } from '../lib/i18n';
import { hiddenCliNames, setCliHidden } from '../lib/visibility';
import { useAppStore } from '../store';
import { CliLogo, cliBrand } from '../components/CliLogo';
import {
  Badge,
  Button,
  Card,
  ConfirmDialog,
  Field,
  Input,
  Loading,
  Modal,
  PageHeader,
  PageScroll,
  Select,
  Toggle,
} from '../components/ui';

interface FormState {
  name: string;
  adapter: string;
  command: string;
  model: string;
  args: string;
  providerId: string;
}

const EMPTY_FORM: FormState = {
  name: '',
  adapter: 'claude-code',
  command: '',
  model: '',
  args: '',
  providerId: '',
};

const LAST_CHECK_KEY = 'lumen.cli.lastChecked';

function shortVersion(version?: string | null): string {
  if (!version) return '';
  const match = version.match(/\d+\.\d+(?:\.\d+)?(?:[-+][\w.]+)?/);
  return match ? match[0] : version.slice(0, 20);
}

function parseVersion(version?: string | null): number[] | null {
  if (!version) return null;
  const match = version.match(/\d+(?:\.\d+)*/);
  return match ? match[0].split('.').map(Number) : null;
}

function isNewer(latest?: string | null, current?: string | null): boolean {
  const left = parseVersion(latest);
  const right = parseVersion(current);
  if (!left || !right) return false;
  for (let i = 0; i < Math.max(left.length, right.length); i += 1) {
    const a = left[i] ?? 0;
    const b = right[i] ?? 0;
    if (a !== b) return a > b;
  }
  return false;
}

function capCell(on: boolean, label?: string) {
  return (
    <span
      className={
        on
          ? 'inline-flex items-center gap-1 text-success'
          : 'inline-flex items-center gap-1 text-muted/50'
      }
    >
      {on ? <Check size={12} /> : <Minus size={12} />}
      {on ? (label ?? tr('支持', 'Yes')) : tr('不支持', 'No')}
    </span>
  );
}

export default function Agents() {
  const [clis, setClis] = useState<DetectedCli[] | null>(null);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [providers, setProviders] = useState<Provider[]>([]);
  const [updates, setUpdates] = useState<Record<string, string>>({});
  const [hidden, setHidden] = useState<Set<string>>(() => hiddenCliNames());
  const [lastChecked, setLastChecked] = useState<number | null>(() => {
    const raw = Number(localStorage.getItem(LAST_CHECK_KEY));
    return Number.isFinite(raw) && raw > 0 ? raw : null;
  });
  const [checking, setChecking] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<Agent | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState<Agent | null>(null);
  const [deleteLoading, setDeleteLoading] = useState(false);
  const loadCore = useAppStore((s) => s.loadCore);
  const { t } = useI18n();

  const load = useCallback(async () => {
    try {
      const [agentList, providerList] = await Promise.all([
        api.agentsList(),
        api.providersList(),
      ]);
      setAgents(agentList);
      setProviders(providerList);
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }, []);

  async function loadUpdates(list: DetectedCli[]): Promise<Record<string, string>> {
    const names = list
      .filter((cli) => cli.installed && cli.installable)
      .map((cli) => cli.name);
    const versions = names.length > 0 ? await api.cliLatestVersions(names) : [];
    const map: Record<string, string> = {};
    for (const item of versions) {
      if (item.latestVersion) map[item.name] = item.latestVersion;
    }
    setUpdates(map);
    const now = Date.now();
    setLastChecked(now);
    localStorage.setItem(LAST_CHECK_KEY, String(now));
    return map;
  }

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const quick = await api.cliDetect(false);
        if (cancelled) return;
        setClis(quick);
        const full = await api.cliDetect(true);
        if (cancelled) return;
        setClis(full);
        await loadUpdates(full);
      } catch {
        // 初次检测失败时保留空列表，用户可点「检查更新」重试
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function openCreate(prefill?: DetectedCli) {
    setEditing(null);
    setForm(
      prefill
        ? {
            name: prefill.name,
            adapter: prefill.adapter,
            command: prefill.command,
            model: '',
            args: '',
            providerId: '',
          }
        : EMPTY_FORM,
    );
    setModalOpen(true);
  }

  function addAgent(cli: DetectedCli) {
    if (hidden.has(cli.name)) {
      setCliHidden(cli.name, false);
      setHidden(hiddenCliNames());
    }
    openCreate(cli);
  }

  function openEdit(agent: Agent) {
    setEditing(agent);
    setForm({
      name: agent.name,
      adapter: agent.adapter,
      command: agent.command ?? '',
      model: agent.model ?? '',
      args: '',
      providerId: agent.providerId ?? '',
    });
    setModalOpen(true);
  }

  function toggleVisible(cli: DetectedCli, visible: boolean) {
    setCliHidden(cli.name, !visible);
    setHidden(hiddenCliNames());
    toast.success(
      visible
        ? t(
            `${cli.name} 已显示在 Agent 选择列表`,
            `${cli.name} is now shown in the agent list`,
          )
        : t(
            `${cli.name} 已从 Agent 选择列表隐藏`,
            `${cli.name} is hidden from the agent list`,
          ),
    );
  }

  async function checkUpdates() {
    if (checking || busy) return;
    setChecking(true);
    try {
      const full = await api.cliDetect(true);
      setClis(full);
      const map = await loadUpdates(full);
      const count = full.filter(
        (cli) => cli.installed && isNewer(map[cli.name], cli.version),
      ).length;
      toast.success(
        count > 0
          ? t(`${count} 个 Agent 可升级`, `${count} agents can be upgraded`)
          : t('所有 Agent 均为最新版本', 'All agents are up to date'),
      );
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setChecking(false);
    }
  }

  async function install(cli: DetectedCli) {
    if (busy) return;
    setBusy(cli.name);
    const toastId = toast.loading(
      t(`正在安装 ${cli.name}，请稍候…`, `Installing ${cli.name}, please wait…`),
    );
    try {
      await api.cliInstall(cli.name);
      const full = await api.cliDetect(true);
      setClis(full);
      await loadUpdates(full);
      const installed = full.find((item) => item.name === cli.name)?.installed;
      if (installed) {
        toast.success(t(`${cli.name} 安装完成`, `${cli.name} installed`), { id: toastId });
      } else {
        toast.warning(
          t(
            `${cli.name} 已安装，但未在 PATH 中找到，请重启应用后重新检测`,
            `${cli.name} was installed but was not found in PATH. Restart the app and detect again.`,
          ),
          {
            id: toastId,
          },
        );
      }
    } catch (err) {
      toast.error(errorMessage(err), { id: toastId });
    } finally {
      setBusy(null);
    }
  }

  async function upgradeOne(cli: DetectedCli) {
    if (busy) return;
    setBusy(cli.name);
    const toastId = toast.loading(
      t(`正在升级 ${cli.name}，请稍候…`, `Upgrading ${cli.name}, please wait…`),
    );
    try {
      await api.cliInstall(cli.name);
      const full = await api.cliDetect(true);
      setClis(full);
      const map = await loadUpdates(full);
      const current = full.find((item) => item.name === cli.name)?.version;
      if (isNewer(map[cli.name], current)) {
        toast.warning(
          t(
            `${cli.name} 已执行升级，但检测到的版本仍为 ${shortVersion(current) || '未知'}`,
            `${cli.name} was upgraded, but the detected version is still ${shortVersion(current) || 'unknown'}`,
          ),
          { id: toastId },
        );
      } else {
        toast.success(
          t(
            `${cli.name} 已升级到 ${shortVersion(map[cli.name]) || '最新版本'}`,
            `${cli.name} upgraded to ${shortVersion(map[cli.name]) || 'the latest version'}`,
          ),
          {
            id: toastId,
          },
        );
      }
    } catch (err) {
      toast.error(errorMessage(err), { id: toastId });
    } finally {
      setBusy(null);
    }
  }

  async function upgradeAll() {
    if (busy || checking) return;
    const targets = upgrades;
    if (targets.length === 0) return;
    setBusy('all');
    const toastId = toast.loading(
      t(
        `正在升级 1/${targets.length}：${targets[0].name}`,
        `Upgrading 1/${targets.length}: ${targets[0].name}`,
      ),
    );
    try {
      for (let i = 0; i < targets.length; i += 1) {
        toast.loading(
          t(
            `正在升级 ${i + 1}/${targets.length}：${targets[i].name}`,
            `Upgrading ${i + 1}/${targets.length}: ${targets[i].name}`,
          ),
          {
            id: toastId,
          },
        );
        await api.cliInstall(targets[i].name);
      }
      const full = await api.cliDetect(true);
      setClis(full);
      await loadUpdates(full);
      toast.success(
        t(`已完成 ${targets.length} 项升级`, `${targets.length} upgrades completed`),
        { id: toastId },
      );
    } catch (err) {
      toast.error(errorMessage(err), { id: toastId });
    } finally {
      setBusy(null);
    }
  }

  async function submit() {
    if (!form.name.trim()) {
      toast.error(t('请填写 Agent 名称', 'Enter an agent name'));
      return;
    }
    if (!form.command.trim() && form.adapter === 'pty') {
      toast.error(t('PTY 适配器需要填写启动命令', 'The PTY adapter requires a launch command'));
      return;
    }
    setSaving(true);
    try {
      const args = form.args
        .split(',')
        .map((item) => item.trim())
        .filter(Boolean);
      if (editing) {
        await api.agentUpdate({
          id: editing.id,
          name: form.name.trim(),
          command: form.command.trim(),
          model: form.model.trim() || undefined,
          args: args.length > 0 ? args : undefined,
          providerId: form.providerId || undefined,
        });
        toast.success(t('Agent 已更新', 'Agent updated'));
      } else {
        await api.agentCreate({
          name: form.name.trim(),
          adapter: form.adapter,
          command: form.command.trim(),
          model: form.model.trim() || undefined,
          args: args.length > 0 ? args : undefined,
          providerId: form.providerId || undefined,
        });
        toast.success(t('Agent 已添加', 'Agent added'));
      }
      setModalOpen(false);
      await load();
      await loadCore();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  async function confirmDelete() {
    if (!deleting) return;
    setDeleteLoading(true);
    try {
      await api.agentDelete(deleting.id);
      toast.success(t('Agent 已删除', 'Agent deleted'));
      setDeleting(null);
      await load();
      await loadCore();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setDeleteLoading(false);
    }
  }

  const installed = (clis ?? []).filter((cli) => cli.installed);
  const uninstalled = (clis ?? []).filter((cli) => !cli.installed);
  const upgrades = installed.filter((cli) => isNewer(updates[cli.name], cli.version));
  const addedAgents = new Map(agents.map((agent) => [agent.name, agent]));
  const checkedLabel = lastChecked
    ? new Date(lastChecked).toLocaleTimeString('zh-CN', {
        hour: '2-digit',
        minute: '2-digit',
      })
    : null;

  function renderRow(cli: DetectedCli) {
    const agent = addedAgents.get(cli.name);
    const latest = updates[cli.name];
    const canUpgrade = cli.installed && isNewer(latest, cli.version);
    const rowBusy = busy === cli.name || busy === 'all';
    return (
      <div
        key={cli.name}
        className="group flex items-center gap-3 border-t border-line/60 px-4 py-3 first:border-t-0 hover:bg-hover/40"
      >
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-soft ring-1 ring-line/50">
          <CliLogo brand={cliBrand(cli)} size={18} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <p className="truncate text-sm font-semibold leading-5 text-ink">
              {cli.name}
            </p>
            {cli.recommended && (
              <Badge tone="primary" className="px-1 text-xs">
                {t('推荐', 'Recommended')}
              </Badge>
            )}
            {agent && (
              <Badge className="px-1 text-xs">
                <span className="h-1.5 w-1.5 rounded-full bg-success" />
                {t('已接入', 'Connected')}
              </Badge>
            )}
          </div>
          <p className="mt-0.5 truncate font-mono text-xs text-muted">
            {cli.installed
              ? cli.command
              : (cli.installHint ??
                t(`未找到 ${cli.name} 命令`, `${cli.name} command not found`))}
          </p>
        </div>
        <div className="w-[128px] shrink-0 text-right">
          {cli.installed ? (
            <>
              <p className="font-mono text-xs font-semibold text-ink">
                {shortVersion(cli.version) || '—'}
              </p>
              {canUpgrade ? (
                <p className="mt-0.5 text-xs text-primary">
                  {t(`有新版本 ${shortVersion(latest)}`, `New version ${shortVersion(latest)}`)}
                </p>
              ) : latest ? (
                <p className="mt-0.5 text-xs text-muted">{t('已是最新', 'Up to date')}</p>
              ) : (
                <p className="mt-0.5 text-xs text-muted/70">
                  {cli.installable
                    ? t('未检查更新', 'Update not checked')
                    : t('不支持自动升级', 'No auto-upgrade')}
                </p>
              )}
            </>
          ) : (
            <p className="text-xs text-muted">{t('未安装', 'Not installed')}</p>
          )}
        </div>
        <div className="flex w-[88px] shrink-0 justify-end">
          {cli.installed ? (
            canUpgrade ? (
              <Button
                variant="outline"
                size="xs"
                loading={rowBusy}
                disabled={Boolean(busy) && !rowBusy}
                onClick={() => upgradeOne(cli)}
              >
                {!rowBusy && <ArrowUpCircle size={12} />}
                {t('升级', 'Upgrade')}
              </Button>
            ) : agent ? null : (
              <Button variant="subtle" size="xs" disabled={Boolean(busy)} onClick={() => addAgent(cli)}>
                <Plus size={12} />
                {t('添加', 'Add')}
              </Button>
            )
          ) : cli.installable ? (
            <Button
              variant="outline"
              size="xs"
              loading={busy === cli.name}
              disabled={Boolean(busy) && busy !== cli.name}
              onClick={() => install(cli)}
            >
              {busy !== cli.name && <Download size={12} />}
              {t('安装', 'Install')}
            </Button>
          ) : (
            <span title={t('官方未提供 Windows 一键安装', 'No official one-click Windows installer')}>
              <Badge>{t('未安装', 'Not installed')}</Badge>
            </span>
          )}
        </div>
        <div className="flex w-9 shrink-0 justify-center">
          <Toggle
            checked={!hidden.has(cli.name)}
            onChange={(value) => toggleVisible(cli, value)}
            label={t(`显示 ${cli.name}`, `Show ${cli.name}`)}
          />
        </div>
        <div className="flex w-[54px] shrink-0 justify-end gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
          {agent && (
            <>
              <Button variant="ghost" size="xs" onClick={() => openEdit(agent)} title={t('编辑', 'Edit')}>
                <Pencil size={13} />
              </Button>
              <Button
                variant="ghost"
                size="xs"
                className="hover:text-danger"
                onClick={() => setDeleting(agent)}
                title={t('删除', 'Delete')}
              >
                <Trash2 size={13} />
              </Button>
            </>
          )}
        </div>
      </div>
    );
  }

  function section(title: string, list: DetectedCli[]) {
    return (
      <section className="mb-4">
        <div className="mb-2 flex items-center gap-3 px-1">
          <p className="flex-1 text-xs font-medium text-muted">
            {t(`${title}（${list.length}）`, `${title} (${list.length})`)}
          </p>
          <span className="w-[128px] text-right text-xs text-muted/70">{t('版本', 'Version')}</span>
          <span className="w-[88px]" />
          <span className="w-9 text-center text-xs text-muted/70">{t('显示', 'Show')}</span>
          <span className="w-[54px]" />
        </div>
        <Card className="overflow-hidden p-0">
          {list.length > 0 ? (
            list.map(renderRow)
          ) : (
            <p className="px-4 py-6 text-center text-xs text-muted">{t('暂无', 'None')}</p>
          )}
        </Card>
      </section>
    );
  }

  return (
    <PageScroll>
      <PageHeader
        title="Agent"
        subtitle={t(
          `本机 CLI 与 Agent 接入，已安装 ${installed.length} / ${clis?.length ?? 0}`,
          `Local CLIs and agents · ${installed.length} / ${clis?.length ?? 0} installed`,
        )}
        actions={
          <>
            {checkedLabel && (
              <span className="text-xs text-muted">
                {t(`上次检查 ${checkedLabel}`, `Checked ${checkedLabel}`)}
              </span>
            )}
            <Button
              variant="outline"
              size="sm"
              loading={busy === 'all'}
              disabled={upgrades.length === 0 || Boolean(busy) || checking}
              onClick={upgradeAll}
            >
              {busy !== 'all' && <ArrowUpCircle size={14} />}
              {t('全部升级', 'Upgrade all')}
              {upgrades.length > 0 ? ` (${upgrades.length})` : ''}
            </Button>
            <Button
              variant="outline"
              size="sm"
              loading={checking}
              disabled={Boolean(busy)}
              onClick={checkUpdates}
            >
              {!checking && <RefreshCw size={14} />}
              {t('检查更新', 'Check for updates')}
            </Button>
            <Button size="sm" onClick={() => openCreate()}>
              <Plus size={14} />
              {t('新建 Agent', 'New agent')}
            </Button>
          </>
        }
      />

      {clis === null ? (
        <Loading />
      ) : (
        <>
          {section(t('已安装', 'Installed'), installed)}
          {section(t('未安装', 'Not installed'), uninstalled)}
          <p className="mt-1 px-1 text-xs text-muted/70">
            {t(
              '关闭「显示」后该 Agent 仍保留在此列表，但不会出现在团队席位的 Agent 选择中。',
              'When "Show" is off, the agent stays in this list but is not offered when assigning team seats.',
            )}
          </p>

          <section className="mt-5">
            <div className="mb-2 flex items-center gap-2 px-1">
              <p className="flex-1 text-xs font-medium text-muted">{t('能力矩阵', 'Capabilities')}</p>
              <span className="text-xs text-muted/70">
                {t('按适配器真实实现标注', 'Based on actual adapter support')}
              </span>
            </div>
            <Card className="overflow-x-auto p-0">
              <table className="w-full min-w-[580px] text-left text-xs">
                <thead>
                  <tr className="border-b border-line/60 text-muted">
                    <th className="px-4 py-2 font-medium">CLI</th>
                    <th className="px-2 py-2 font-medium">{t('实时输出', 'Live output')}</th>
                    <th className="px-2 py-2 font-medium">{t('用量统计', 'Usage')}</th>
                    <th className="px-2 py-2 font-medium">{t('工具审批', 'Tool approval')}</th>
                    <th className="px-2 py-2 font-medium">{t('中断续跑', 'Resume')}</th>
                    <th className="px-2 py-2 font-medium">{t('预算硬停', 'Budget stop')}</th>
                  </tr>
                </thead>
                <tbody>
                  {installed.map((cli) => {
                    const claude = cli.adapter === 'claude-code';
                    const caps = cli.capabilities ?? {
                      streaming: true,
                      streamUsage: claude,
                      toolApproval: claude,
                      resume: claude,
                      budgetEnforcement: claude,
                      usageSource: claude ? 'provider' : 'estimate',
                    };
                    return (
                      <tr key={cli.name} className="border-b border-line/40 last:border-0">
                        <td className="px-4 py-2 font-medium text-ink">{cli.name}</td>
                        <td className="px-2 py-2">{capCell(Boolean(caps.streaming))}</td>
                        <td className="px-2 py-2">
                          {capCell(
                            true,
                            caps.usageSource === 'provider'
                              ? t('实测', 'Measured')
                              : t('估算', 'Estimated'),
                          )}
                        </td>
                        <td className="px-2 py-2">{capCell(Boolean(caps.toolApproval))}</td>
                        <td className="px-2 py-2">{capCell(Boolean(caps.resume))}</td>
                        <td className="px-2 py-2">{capCell(Boolean(caps.budgetEnforcement))}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </Card>
            <p className="mt-1.5 px-1 text-xs leading-5 text-muted/70">
              {t(
                '未标注的能力不会在运行中生效；非 Claude Code 的用量按输出文本估算，中断续跑与预算硬停仅对 Claude Code 生效。',
                'Unlisted capabilities are not active at runtime. Usage for non-Claude Code CLIs is estimated from output text; resume and budget hard-stop apply only to Claude Code.',
              )}
            </p>
          </section>
        </>
      )}

      <Modal
        open={modalOpen}
        onClose={() => setModalOpen(false)}
        title={editing ? t('编辑 Agent', 'Edit agent') : t('新建 Agent', 'New agent')}
        description={t('Agent 是 CLI 进程的封装，任务执行时按席位调用', 'An agent wraps a CLI process and is invoked per seat when tasks run')}
        footer={
          <>
            <Button variant="outline" size="sm" onClick={() => setModalOpen(false)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button size="sm" loading={saving} onClick={submit}>
              {editing ? t('保存', 'Save') : t('添加', 'Add')}
            </Button>
          </>
        }
      >
        <div className="space-y-3">
          <Field label={t('名称', 'Name')}>
            <Input
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder={t('例如：Claude Code', 'e.g. Claude Code')}
            />
          </Field>
          <Field
            label={t('适配器', 'Adapter')}
            hint={
              editing
                ? t('适配器创建后不可修改', 'The adapter cannot be changed after creation')
                : t('决定如何解析 CLI 输出', 'Determines how CLI output is parsed')
            }
          >
            <Select
              value={form.adapter}
              disabled={Boolean(editing)}
              onChange={(e) => setForm({ ...form, adapter: e.target.value })}
            >
              {ADAPTERS.map((adapter) => (
                <option key={adapter.value} value={adapter.value}>
                  {adapter.label}
                </option>
              ))}
            </Select>
          </Field>
          <Field
            label={t('启动命令', 'Launch command')}
            hint={t('留空时按适配器自动解析默认命令', 'Leave empty to use the adapter’s default command')}
          >
            <Input
              value={form.command}
              onChange={(e) => setForm({ ...form, command: e.target.value })}
              placeholder={t(
                'claude / opencode / 可执行文件路径',
                'claude / opencode / path to executable',
              )}
              className="font-mono text-xs"
            />
          </Field>
          <Field
            label={t('模型', 'Model')}
            hint={t(
              '可从所选供应商已保存的模型中选择，也可手动填写',
              'Pick a saved model from the selected provider or enter one manually',
            )}
          >
            <Input
              value={form.model}
              onChange={(e) => setForm({ ...form, model: e.target.value })}
              placeholder={t('可选，例如 claude-sonnet-4-5', 'Optional, e.g. claude-sonnet-4-5')}
              className="font-mono text-xs"
              list="agent-model-options"
            />
            <datalist id="agent-model-options">
              {(
                providers.find((provider) => provider.id === form.providerId)?.models ?? []
              ).map((model) => (
                <option key={model} value={model} />
              ))}
            </datalist>
          </Field>
          <Field
            label={t('API 供应商', 'API provider')}
            hint={
              providers.length === 0
                ? t(
                    '还没有供应商，可在「设置 → 自定义 API 供应商」添加',
                    'No providers yet. Add one under Settings → Custom API providers',
                  )
                : t(
                    '运行时仅以环境变量注入 Base URL / Key / 模型，不改写 CLI 配置',
                    'Base URL, key and model are injected as environment variables only; CLI config is left untouched',
                  )
            }
          >
            <Select
              value={form.providerId}
              onChange={(e) => setForm({ ...form, providerId: e.target.value })}
            >
              <option value="">{t('CLI 原有配置', 'CLI default config')}</option>
              {providers.map((provider) => (
                <option key={provider.id} value={provider.id}>
                  {provider.name}
                  {t('，', ', ')}
                  {provider.protocol === 'anthropic' ? 'Anthropic' : 'OpenAI'}
                  {provider.enabled ? '' : t('（已停用）', ' (disabled)')}
                </option>
              ))}
            </Select>
          </Field>
          <Field
            label={t('启动参数', 'Launch args')}
            hint={t(
              '多个参数用英文逗号分隔，例如 run,--model,gpt-5',
              'Separate multiple args with commas, e.g. run,--model,gpt-5',
            )}
          >
            <Input
              value={form.args}
              onChange={(e) => setForm({ ...form, args: e.target.value })}
              placeholder="run"
              className="font-mono text-xs"
            />
          </Field>
        </div>
      </Modal>

      <ConfirmDialog
        open={Boolean(deleting)}
        onClose={() => setDeleting(null)}
        onConfirm={confirmDelete}
        title={t(`删除 Agent「${deleting?.name ?? ''}」`, `Delete agent "${deleting?.name ?? ''}"`)}
        description={t('已指派该 Agent 的席位会被自动清空', 'Seats assigned to this agent will be cleared')}
        loading={deleteLoading}
      />
    </PageScroll>
  );
}
