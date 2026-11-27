import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  AlertTriangle,
  Pencil,
  Plus,
  Power,
  Search,
  ShieldAlert,
  Trash2,
  Wallet,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import type { Agent, Budget, Project, Task, Team } from '../lib/types';
import {
  BUDGET_PERIOD,
  BUDGET_SCOPE,
  BUDGET_UNIT,
} from '../lib/labels';
import { useI18n } from '../lib/i18n';
import { formatTokens, microsToUsd } from '../lib/format';
import {
  Badge,
  Button,
  Card,
  ConfirmDialog,
  Empty,
  Field,
  Input,
  Loading,
  Modal,
  PageHeader,
  PageScroll,
  ProgressBar,
  Select,
  Toggle,
  cn,
} from '../components/ui';

interface FormState {
  scopeType: string;
  scopeId: string;
  unit: string;
  period: string;
  limit: string;
  warnPct: string;
  hardStop: boolean;
}

const EMPTY_FORM: FormState = {
  scopeType: 'global',
  scopeId: '',
  unit: 'tokens',
  period: 'month',
  limit: '',
  warnPct: '80',
  hardStop: false,
};

function amount(unit: string, value: number): string {
  return unit === 'usd' ? microsToUsd(value) : formatTokens(value);
}

export default function Budgets({ embedded = false }: { embedded?: boolean }) {
  const [budgets, setBudgets] = useState<Budget[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [teams, setTeams] = useState<Team[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  // 任务作用域按搜索按需加载，避免一次拉取被 300 条上限截断。
  const [taskOptions, setTaskOptions] = useState<Task[]>([]);
  const [taskSearch, setTaskSearch] = useState('');
  const [taskTotal, setTaskTotal] = useState(0);
  const [taskLoading, setTaskLoading] = useState(false);
  const [selectedTaskName, setSelectedTaskName] = useState('');
  const [loading, setLoading] = useState(true);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<Budget | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState<Budget | null>(null);
  const [deleteLoading, setDeleteLoading] = useState(false);
  const { t } = useI18n();

  const load = useCallback(async () => {
    try {
      const [budgetList, projectList, teamList, agentList] = await Promise.all([
        api.budgetsList(),
        api.projectsList(),
        api.teamsList(),
        api.agentsList(),
      ]);
      setBudgets(budgetList);
      setProjects(projectList);
      setTeams(teamList);
      setAgents(agentList);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const scopeOptions = useMemo(() => {
    switch (form.scopeType) {
      case 'project':
        return projects.map((item) => ({ id: item.id, name: item.name }));
      case 'team':
        return teams.map((item) => ({ id: item.id, name: item.name }));
      case 'agent':
        return agents.map((item) => ({ id: item.id, name: item.name }));
      case 'task': {
        const options = taskOptions.map((item) => ({ id: item.id, name: item.title }));
        // 编辑已有预算时，当前任务可能不在搜索结果里，补一个占位项避免显示为空。
        if (form.scopeId && !options.some((option) => option.id === form.scopeId)) {
          options.unshift({ id: form.scopeId, name: selectedTaskName || t('当前任务', 'Current task') });
        }
        return options;
      }
      default:
        return [];
    }
  }, [form.scopeType, form.scopeId, projects, teams, agents, taskOptions, selectedTaskName, t]);

  useEffect(() => {
    if (!modalOpen || form.scopeType !== 'task') return;
    let cancelled = false;
    const query = taskSearch.trim();
    const timer = window.setTimeout(async () => {
      setTaskLoading(true);
      try {
        const page = await api.tasksPage({ query: query || undefined, limit: 50 });
        if (!cancelled) {
          setTaskOptions(page.items);
          setTaskTotal(page.total);
        }
      } catch (err) {
        if (!cancelled) toast.error(errorMessage(err));
      } finally {
        if (!cancelled) setTaskLoading(false);
      }
    }, query ? 300 : 0);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [modalOpen, form.scopeType, taskSearch]);

  function openCreate() {
    setEditing(null);
    setForm(EMPTY_FORM);
    setTaskSearch('');
    setSelectedTaskName('');
    setModalOpen(true);
  }

  function openEdit(budget: Budget) {
    setEditing(budget);
    setTaskSearch('');
    setSelectedTaskName(budget.scopeType === 'task' ? budget.scopeName : '');
    setForm({
      scopeType: budget.scopeType,
      scopeId: budget.scopeId ?? '',
      unit: budget.unit,
      period: budget.period,
      limit:
        budget.unit === 'usd'
          ? String(budget.limitValue / 1_000_000)
          : String(budget.limitValue),
      warnPct: String(budget.warnPct),
      hardStop: budget.hardStop,
    });
    setModalOpen(true);
  }

  function changeScopeType(scopeType: string) {
    const first =
      scopeType === 'project'
        ? projects[0]?.id
        : scopeType === 'team'
          ? teams[0]?.id
          : scopeType === 'agent'
            ? agents[0]?.id
            : '';
    if (scopeType === 'task') {
      setTaskSearch('');
      setSelectedTaskName('');
    }
    setForm((current) => ({ ...current, scopeType, scopeId: first ?? '' }));
  }

  async function submit() {
    if (form.scopeType !== 'global' && !form.scopeId) {
      toast.error(t('请选择预算作用对象', 'Select a budget target'));
      return;
    }
    const rawLimit = Number(form.limit);
    if (!Number.isFinite(rawLimit) || rawLimit <= 0) {
      toast.error(t('预算上限必须大于 0', 'The budget limit must be greater than 0'));
      return;
    }
    const limitValue =
      form.unit === 'usd' ? Math.round(rawLimit * 1_000_000) : Math.round(rawLimit);
    setSaving(true);
    try {
      await api.budgetUpsert({
        id: editing?.id,
        scopeType: form.scopeType,
        scopeId: form.scopeType === 'global' ? null : form.scopeId,
        unit: form.unit,
        period: form.period,
        limitValue,
        warnPct: Number(form.warnPct) || 80,
        hardStop: form.hardStop,
      });
      toast.success(editing ? t('预算已更新', 'Budget updated') : t('预算已创建', 'Budget created'));
      setModalOpen(false);
      await load();
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
      await api.budgetDelete(deleting.id);
      toast.success(t('预算已删除', 'Budget deleted'));
      setDeleting(null);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setDeleteLoading(false);
    }
  }

  async function toggleEnabled(budget: Budget) {
    try {
      await api.budgetUpsert({
        id: budget.id,
        scopeType: budget.scopeType,
        scopeId: budget.scopeId ?? null,
        unit: budget.unit,
        period: budget.period,
        limitValue: budget.limitValue,
        warnPct: budget.warnPct,
        hardStop: budget.hardStop,
        enabled: !budget.enabled,
      });
      toast.success(
        budget.enabled ? t('预算已停用', 'Budget disabled') : t('预算已启用', 'Budget enabled'),
      );
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  const blockedCount = budgets.filter((budget) => budget.blocked).length;
  const warnCount = budgets.filter(
    (budget) => !budget.blocked && budget.pct >= budget.warnPct,
  ).length;

  return (
    <PageScroll embedded={embedded}>
      <PageHeader
        back={embedded ? undefined : { to: '/settings', label: t('返回设置', 'Back to settings') }}
        title={t('预算', 'Budgets')}
        actions={
          <Button size="sm" onClick={openCreate}>
            <Plus size={14} />
            {t('新建预算', 'New budget')}
          </Button>
        }
      />

      {!loading && budgets.length > 0 && (
        <div className="mb-4 flex flex-wrap items-center gap-2 text-xs text-muted">
          <span className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-card px-2.5 py-1.5">
            <Wallet size={12} className="text-primary" />
            {t(`共 ${budgets.length} 条预算`, `${budgets.length} budgets`)}
          </span>
          <span className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-card px-2.5 py-1.5">
            <AlertTriangle size={12} className="text-warn" />
            {t(`预警 ${warnCount} 条`, `${warnCount} warning`)}
          </span>
          <span className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-card px-2.5 py-1.5">
            <ShieldAlert size={12} className="text-danger" />
            {t(`已硬停 ${blockedCount} 条`, `${blockedCount} stopped`)}
          </span>
        </div>
      )}

      {loading ? (
        <Loading />
      ) : budgets.length === 0 ? (
        <Empty
          icon={Wallet}
          title={t('还没有预算', 'No budgets yet')}
          description={t(
            '为全局、项目、团队、Agent 或任务设置用量与成本上限',
            'Set usage and cost limits for global, project, team, agent, or task scopes',
          )}
          action={
            <Button size="sm" onClick={openCreate}>
              <Plus size={14} />
              {t('新建预算', 'New budget')}
            </Button>
          }
          className="py-16"
        />
      ) : (
        <div className="space-y-3">
          {budgets.map((budget) => {
            const warning = budget.pct >= budget.warnPct;
            return (
              <Card
                key={budget.id}
                className={cn(
                  'group',
                  budget.blocked && 'border-danger/40 bg-danger/[0.04]',
                )}
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <Badge tone="primary">{BUDGET_SCOPE[budget.scopeType] ?? budget.scopeType}</Badge>
                      <p className="truncate text-sm font-semibold text-ink">
                        {budget.scopeName}
                      </p>
                      {budget.blocked && (
                        <Badge>
                          <span className="h-1.5 w-1.5 rounded-full bg-danger" />
                          {t('已阻断', 'Blocked')}
                        </Badge>
                      )}
                      {!budget.blocked && warning && (
                        <Badge>
                          <span className="h-1.5 w-1.5 rounded-full bg-warn" />
                          {t('已预警', 'Warning')}
                        </Badge>
                      )}
                      {budget.hardStop && <Badge>{t('硬停', 'Hard stop')}</Badge>}
                      {!budget.enabled && <Badge>{t('已停用', 'Disabled')}</Badge>}
                    </div>
                    <p className="mt-1 text-xs text-muted">
                      {BUDGET_PERIOD[budget.period] ?? budget.period}
                      {t('，', ', ')}
                      {BUDGET_UNIT[budget.unit] ?? budget.unit}
                      {t(`，预警线 ${budget.warnPct}%`, `, alert at ${budget.warnPct}%`)}
                    </p>
                  </div>
                  <div className="flex items-center gap-0.5">
                    <Button
                      variant="ghost"
                      size="xs"
                      onClick={() => toggleEnabled(budget)}
                      title={budget.enabled ? t('停用预算', 'Disable budget') : t('启用预算', 'Enable budget')}
                    >
                      <Power size={13} />
                    </Button>
                    <Button variant="ghost" size="xs" onClick={() => openEdit(budget)} title={t('编辑', 'Edit')}>
                      <Pencil size={13} />
                    </Button>
                    <Button
                      variant="ghost"
                      size="xs"
                      className="hover:text-danger"
                      onClick={() => setDeleting(budget)}
                      title={t('删除', 'Delete')}
                    >
                      <Trash2 size={13} />
                    </Button>
                  </div>
                </div>

                <div className="mt-3 flex items-end justify-between gap-4">
                  <div className="flex-1">
                    <div className="mb-1.5 flex items-center justify-between text-xs">
                      <span className="font-mono text-ink">
                        {amount(budget.unit, budget.spent)}
                        <span className="text-muted"> / {amount(budget.unit, budget.limitValue)}</span>
                      </span>
                      <span
                        className={cn(
                          'font-mono',
                          budget.blocked
                            ? 'text-danger'
                            : warning
                              ? 'text-warn'
                              : 'text-muted',
                        )}
                      >
                        {budget.pct.toFixed(1)}%
                      </span>
                    </div>
                    <ProgressBar
                      value={budget.pct}
                      warn={warning}
                      blocked={budget.blocked}
                      markerPct={budget.warnPct}
                    />
                  </div>
                  {budget.reserved > 0 && (
                    <span className="shrink-0 text-xs text-muted">
                      {t('预留', 'Reserved')} {amount(budget.unit, budget.reserved)}
                    </span>
                  )}
                </div>
              </Card>
            );
          })}
        </div>
      )}

      <Modal
        open={modalOpen}
        onClose={() => setModalOpen(false)}
        title={editing ? t('编辑预算', 'Edit budget') : t('新建预算', 'New budget')}
        description={t(
          '美元预算按 micro-USD 存储，输入时按美元填写',
          'USD budgets are stored in micro-USD; enter amounts in USD',
        )}
        footer={
          <>
            <Button variant="outline" size="sm" onClick={() => setModalOpen(false)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button size="sm" loading={saving} onClick={submit}>
              {editing ? t('保存', 'Save') : t('创建', 'Create')}
            </Button>
          </>
        }
      >
        <div className="space-y-3">
          <div className="grid grid-cols-2 gap-3">
            <Field label={t('作用范围', 'Scope')}>
              <Select value={form.scopeType} onChange={(e) => changeScopeType(e.target.value)}>
                {Object.entries(BUDGET_SCOPE).map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label={t('计量单位', 'Unit')}>
              <Select value={form.unit} onChange={(e) => setForm({ ...form, unit: e.target.value })}>
                <option value="tokens">Tokens</option>
                <option value="usd">{t('美元（USD）', 'USD')}</option>
              </Select>
            </Field>
          </div>

          {form.scopeType === 'task' && (
            <>
              <Field label={t('搜索任务', 'Search tasks')}>
                <div className="relative">
                  <Search
                    size={13}
                    className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted"
                  />
                  <Input
                    value={taskSearch}
                    onChange={(e) => setTaskSearch(e.target.value)}
                    placeholder={t('按任务标题或描述搜索', 'Search by task title or description')}
                    className="pl-8"
                  />
                </div>
              </Field>
              <p className="-mt-1 text-xs text-muted">
                {taskLoading
                  ? t('搜索中…', 'Searching…')
                  : t(
                      `匹配 ${taskTotal} 个任务${
                        taskTotal > taskOptions.length
                          ? `，仅显示前 ${taskOptions.length} 个，可继续输入缩小范围`
                          : ''
                      }`,
                      `${taskTotal} tasks matched${
                        taskTotal > taskOptions.length
                          ? `, showing the first ${taskOptions.length}. Keep typing to narrow it down`
                          : ''
                      }`,
                    )}
              </p>
            </>
          )}

          {form.scopeType !== 'global' && (
            <Field label={t('作用对象', 'Target')}>
              <Select
                value={form.scopeId}
                onChange={(e) => {
                  const nextId = e.target.value;
                  if (form.scopeType === 'task') {
                    setSelectedTaskName(
                      scopeOptions.find((option) => option.id === nextId)?.name ?? '',
                    );
                  }
                  setForm({ ...form, scopeId: nextId });
                }}
              >
                <option value="">{t('请选择', 'Select')}</option>
                {scopeOptions.map((option) => (
                  <option key={option.id} value={option.id}>
                    {option.name}
                  </option>
                ))}
              </Select>
            </Field>
          )}

          <div className="grid grid-cols-2 gap-3">
            <Field label={t('统计周期', 'Period')}>
              <Select
                value={form.period}
                onChange={(e) => setForm({ ...form, period: e.target.value })}
              >
                <option value="day">{t('每日', 'Daily')}</option>
                <option value="month">{t('每月', 'Monthly')}</option>
                <option value="task">{t('按任务', 'Per task')}</option>
              </Select>
            </Field>
            <Field
              label={form.unit === 'usd' ? t('上限（美元）', 'Limit (USD)') : t('上限（Tokens）', 'Limit (Tokens)')}
              hint={
                form.unit === 'usd'
                  ? t('提交时自动换算为 micro-USD', 'Converted to micro-USD on submit')
                  : undefined
              }
            >
              <Input
                value={form.limit}
                onChange={(e) => setForm({ ...form, limit: e.target.value })}
                placeholder={form.unit === 'usd' ? '2.50' : '200000'}
                inputMode="decimal"
                className="font-mono text-xs"
              />
            </Field>
          </div>

          <div className="grid grid-cols-2 items-end gap-3">
            <Field label={t('预警线（%）', 'Alert threshold (%)')}>
              <Input
                value={form.warnPct}
                onChange={(e) => setForm({ ...form, warnPct: e.target.value })}
                inputMode="numeric"
                className="font-mono text-xs"
              />
            </Field>
            <div className="flex items-center justify-between rounded-lg border border-line bg-base/40 px-3 py-2">
              <div>
                <p className="text-xs text-ink">{t('达到上限硬停', 'Hard stop at limit')}</p>
                <p className="text-xs text-muted">
                  {t('触发后阻断执行并生成审批', 'Blocks runs and creates an approval when triggered')}
                </p>
              </div>
              <Toggle
                checked={form.hardStop}
                onChange={(value) => setForm({ ...form, hardStop: value })}
                label={t('硬停', 'Hard stop')}
              />
            </div>
          </div>
        </div>
      </Modal>

      <ConfirmDialog
        open={Boolean(deleting)}
        onClose={() => setDeleting(null)}
        onConfirm={confirmDelete}
        title={t('删除预算', 'Delete budget')}
        description={t(
          '删除后不再对该范围进行用量限制',
          'Usage will no longer be limited for this scope after deletion',
        )}
        loading={deleteLoading}
      />
    </PageScroll>
  );
}
