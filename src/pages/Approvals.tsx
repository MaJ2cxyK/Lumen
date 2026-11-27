import { useCallback, useEffect, useState } from 'react';
import {
  Check,
  ChevronDown,
  Pencil,
  Plus,
  ShieldCheck,
  Trash2,
  X,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import { onEvent } from '../lib/events';
import type { Approval, ApprovalRule } from '../lib/types';
import {
  APPROVAL_STATUS,
  approvalTypeMeta,
  RISK_META,
} from '../lib/labels';
import { tr, useI18n } from '../lib/i18n';
import { formatDateTime } from '../lib/format';
import { useAppStore } from '../store';
import {
  Badge,
  Button,
  Card,
  Empty,
  Field,
  Input,
  Loading,
  Modal,
  PageHeader,
  PageScroll,
  SectionTitle,
  Select,
  Tabs,
  Toggle,
  cn,
} from '../components/ui';

function KV({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex gap-2 text-xs leading-5">
      <span className="w-20 shrink-0 text-muted">{label}</span>
      <span className="min-w-0 break-all text-ink/85">{value}</span>
    </div>
  );
}

function ApprovalDetail({ approval }: { approval: Approval }) {
  const { t } = useI18n();
  const detail = (approval.detail ?? {}) as Record<string, any>;

  if (approval.type === 'stage_gate') {
    return (
      <div className="space-y-1 rounded-lg border border-line/60 bg-base/40 px-3 py-2">
        <KV label={t('席位', 'Seat')} value={String(detail.roleName ?? '—')} />
        <KV
          label={t('阶段', 'Stage')}
          value={t(
            `第 ${Number(detail.stageIndex ?? 0) + 1} 阶段${
              detail.nextStageIndex !== undefined
                ? ` → 第 ${Number(detail.nextStageIndex) + 1} 阶段`
                : ''
            }`,
            `Stage ${Number(detail.stageIndex ?? 0) + 1}${
              detail.nextStageIndex !== undefined
                ? ` → Stage ${Number(detail.nextStageIndex) + 1}`
                : ''
            }`,
          )}
        />
        {detail.summary && (
          <div className="pt-1">
            <p className="text-xs text-muted">{t('阶段摘要', 'Stage summary')}</p>
            <p className="mt-1 whitespace-pre-wrap text-xs leading-5 text-ink/80">
              {String(detail.summary)}
            </p>
          </div>
        )}
      </div>
    );
  }

  if (approval.type === 'tool_permission') {
    return (
      <div className="space-y-1 rounded-lg border border-line/60 bg-base/40 px-3 py-2">
        <KV label={t('工具', 'Tool')} value={String(detail.toolName ?? '—')} />
        <KV label={t('工作目录', 'Working directory')} value={String(detail.cwd ?? '—')} />
        {detail.reason && <KV label={t('原因', 'Reason')} value={String(detail.reason)} />}
        {detail.seatName && (
          <KV label={t('发起席位', 'Requesting seat')} value={String(detail.seatName)} />
        )}
        {detail.toolInput !== undefined && (
          <details className="pt-1">
            <summary className="flex items-center gap-1 text-xs text-primary">
              <ChevronDown size={11} />
              {t('查看工具参数', 'View tool parameters')}
            </summary>
            <pre className="mt-1.5 max-h-40 overflow-auto rounded-xl border border-line bg-elevated p-2 font-mono text-xs leading-4 text-ink/75">
              {JSON.stringify(detail.toolInput, null, 2)}
            </pre>
          </details>
        )}
      </div>
    );
  }

  if (approval.type === 'run_permission') {
    return (
      <div className="space-y-1 rounded-lg border border-warn/25 bg-warn/[0.04] px-3 py-2">
        <KV label="Agent" value={String(detail.agentName ?? '—')} />
        <KV label={t('适配器', 'Adapter')} value={String(detail.adapter ?? '—')} />
        <KV label={t('发起席位', 'Requesting seat')} value={String(detail.seatName ?? '—')} />
        <KV
          label={t('阶段', 'Stage')}
          value={t(
            `第 ${Number(detail.stageIndex ?? 0) + 1} 阶段`,
            `Stage ${Number(detail.stageIndex ?? 0) + 1}`,
          )}
        />
        {detail.reason && <KV label={t('原因', 'Reason')} value={String(detail.reason)} />}
      </div>
    );
  }

  if (approval.type === 'budget_exceeded') {
    return (
      <div className="space-y-1 rounded-lg border border-danger/25 bg-danger/[0.04] px-3 py-2">
        {Object.entries(detail).map(([key, value]) => (
          <KV
            key={key}
            label={key}
            value={typeof value === 'string' ? value : JSON.stringify(value)}
          />
        ))}
        {Object.keys(detail).length === 0 && (
          <p className="text-xs text-muted">
            {t('预算已越限，等待确认是否继续执行。', 'Budget exceeded. Awaiting confirmation to continue.')}
          </p>
        )}
      </div>
    );
  }

  return (
    <pre className="max-h-40 overflow-auto rounded-lg border border-line/60 bg-base/40 p-2 font-mono text-xs leading-4 text-ink/70">
      {JSON.stringify(approval.detail, null, 2)}
    </pre>
  );
}

interface RuleFormState {
  tool: string;
  commandContains: string;
  commandExact: string;
  pathGlob: string;
  action: string;
  priority: string;
}

const EMPTY_RULE: RuleFormState = {
  tool: '',
  commandContains: '',
  commandExact: '',
  pathGlob: '',
  action: 'ask',
  priority: '100',
};

function matcherText(rule: ApprovalRule): string {
  const match = (rule.match ?? {}) as Record<string, unknown>;
  const parts: string[] = [];
  if (match.tool) {
    parts.push(tr(`工具 ${String(match.tool)}`, `Tool ${String(match.tool)}`));
  }
  if (match.commandExact) {
    parts.push(
      tr(
        `命令精确 ${String(match.commandExact)}`,
        `Exact command ${String(match.commandExact)}`,
      ),
    );
  }
  if (match.commandContains) {
    parts.push(
      tr(
        `命令包含 ${String(match.commandContains)}`,
        `Command contains ${String(match.commandContains)}`,
      ),
    );
  }
  if (match.pathGlob) {
    parts.push(tr(`路径 ${String(match.pathGlob)}`, `Path ${String(match.pathGlob)}`));
  }
  return parts.length > 0 ? parts.join(tr('，', ', ')) : JSON.stringify(rule.match);
}

function actionTone(action: string): 'success' | 'danger' | 'warn' {
  if (action === 'allow') return 'success';
  if (action === 'deny') return 'danger';
  return 'warn';
}

export default function Approvals() {
  const { t } = useI18n();
  const [tab, setTab] = useState<'pending' | 'all'>('pending');
  const [approvals, setApprovals] = useState<Approval[]>([]);
  const [approvalsLoaded, setApprovalsLoaded] = useState(false);
  const [rules, setRules] = useState<ApprovalRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [remember, setRemember] = useState<Record<string, boolean>>({});
  const [deciding, setDeciding] = useState('');
  const [ruleModalOpen, setRuleModalOpen] = useState(false);
  const [editingRule, setEditingRule] = useState<ApprovalRule | null>(null);
  const [ruleForm, setRuleForm] = useState<RuleFormState>(EMPTY_RULE);
  const [savingRule, setSavingRule] = useState(false);
  const [deletingRule, setDeletingRule] = useState<ApprovalRule | null>(null);
  const refreshPending = useAppStore((s) => s.refreshPending);

  const load = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    try {
      // 一次拉全量，页签与计数在前端完成，避免“全部”页签显示的是待处理数量。
      const [approvalList, ruleList] = await Promise.all([
        api.approvalsList(),
        api.approvalRulesList(),
      ]);
      setApprovals(approvalList);
      setApprovalsLoaded(true);
      setRules(ruleList);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      if (!silent) setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  // 任务取消/重置或被阻塞会批量过期审批，静默同步列表避免停留在旧状态。
  useEffect(() => {
    let disposed = false;
    const unsubs: Array<() => void> = [];
    Promise.all([
      onEvent('approval.changed', () => load(true)),
      onEvent('approval.requested', () => load(true)),
      onEvent('approval.decided', () => load(true)),
    ]).then((fns) => {
      if (disposed) fns.forEach((fn) => fn());
      else unsubs.push(...fns);
    });
    return () => {
      disposed = true;
      unsubs.forEach((fn) => fn());
    };
  }, [load]);

  async function decide(approval: Approval, approve: boolean) {
    setDeciding(`${approval.id}:${approve ? 'yes' : 'no'}`);
    try {
      await api.approvalDecide({
        approvalId: approval.id,
        approve,
        note: notes[approval.id]?.trim() || undefined,
        remember: Boolean(remember[approval.id]),
      });
      toast.success(approve ? t('已批准', 'Approved') : t('已拒绝', 'Denied'));
      await load();
      await refreshPending();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setDeciding('');
    }
  }

  function openRuleCreate() {
    setEditingRule(null);
    setRuleForm(EMPTY_RULE);
    setRuleModalOpen(true);
  }

  function openRuleEdit(rule: ApprovalRule) {
    const match = (rule.match ?? {}) as Record<string, unknown>;
    setEditingRule(rule);
    setRuleForm({
      tool: match.tool ? String(match.tool) : '',
      commandContains: match.commandContains ? String(match.commandContains) : '',
      commandExact: match.commandExact ? String(match.commandExact) : '',
      pathGlob: match.pathGlob ? String(match.pathGlob) : '',
      action: rule.action,
      priority: String(rule.priority),
    });
    setRuleModalOpen(true);
  }

  async function saveRule() {
    if (!ruleForm.tool.trim() && !ruleForm.commandContains.trim() && !ruleForm.commandExact.trim() && !ruleForm.pathGlob.trim()) {
      toast.error(
        t('请至少填写工具、命令或路径条件', 'Fill in at least a tool, command or path condition'),
      );
      return;
    }
    const matcher: Record<string, string> = {};
    if (ruleForm.tool.trim()) matcher.tool = ruleForm.tool.trim();
    if (ruleForm.commandContains.trim()) matcher.commandContains = ruleForm.commandContains.trim();
    if (ruleForm.commandExact.trim()) matcher.commandExact = ruleForm.commandExact.trim();
    if (ruleForm.pathGlob.trim()) matcher.pathGlob = ruleForm.pathGlob.trim();
    setSavingRule(true);
    try {
      await api.approvalRuleUpsert({
        id: editingRule?.id,
        matcher,
        action: ruleForm.action,
        priority: Number(ruleForm.priority) || 100,
      });
      toast.success(editingRule ? t('规则已更新', 'Rule updated') : t('规则已创建', 'Rule created'));
      setRuleModalOpen(false);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSavingRule(false);
    }
  }

  async function toggleRule(rule: ApprovalRule, enabled: boolean) {
    try {
      await api.approvalRuleToggle(rule.id, enabled);
      setRules((prev) =>
        prev.map((item) => (item.id === rule.id ? { ...item, enabled } : item)),
      );
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function confirmDeleteRule() {
    if (!deletingRule) return;
    try {
      await api.approvalRuleDelete(deletingRule.id);
      toast.success(t('规则已删除', 'Rule deleted'));
      setDeletingRule(null);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  const pendingCount = approvals.filter((item) => item.status === 'pending').length;
  const visible = tab === 'pending'
    ? approvals.filter((item) => item.status === 'pending')
    : approvals;

  return (
    <PageScroll>
      <PageHeader
        title={t('审批中心', 'Approval center')}
        actions={
          <Tabs
            value={tab}
            onChange={setTab}
            items={[
              { value: 'pending', label: t('待处理', 'Pending'), count: pendingCount },
              { value: 'all', label: t('全部', 'All'), count: approvals.length },
            ]}
          />
        }
      />

      {loading ? (
        <Loading />
      ) : !approvalsLoaded ? (
        <Empty
          icon={ShieldCheck}
          title={t('无法加载审批记录', 'Unable to load approvals')}
          description={t('请稍后重试', 'Try again later')}
          action={
            <Button size="sm" variant="outline" onClick={() => load()}>
              {t('重试', 'Retry')}
            </Button>
          }
          className="py-14"
        />
      ) : visible.length === 0 ? (
        <Empty
          icon={ShieldCheck}
          title={
            tab === 'pending'
              ? t('没有待处理审批', 'No pending approvals')
              : t('暂无审批记录', 'No approvals yet')
          }
          description={t(
            '任务执行到门禁或需要工具授权时会出现在这里',
            'Approvals appear here when a task hits a gate or needs tool access',
          )}
          className="py-14"
        />
      ) : (
        <div className="space-y-3.5">
          {visible.map((approval) => {
            const typeMeta = approvalTypeMeta(approval.type);
            const risk = RISK_META[approval.riskLevel ?? ''] ?? null;
            const status = APPROVAL_STATUS[approval.status] ?? APPROVAL_STATUS.pending;
            const pending = approval.status === 'pending';
            return (
              <Card key={approval.id}>
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <p className="text-sm font-semibold leading-5 text-ink">
                        {approval.title}
                      </p>
                      {!pending && (
                        <Badge>
                          <span className={cn('h-1.5 w-1.5 rounded-full', status.dot)} />
                          {status.label}
                        </Badge>
                      )}
                    </div>
                    <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                      <Badge>
                        <span className={cn('h-1.5 w-1.5 rounded-full', typeMeta.dot)} />
                        {typeMeta.label}
                      </Badge>
                      {risk && (
                        <Badge>
                          <span className={cn('h-1.5 w-1.5 rounded-full', risk.dot)} />
                          {risk.label}
                        </Badge>
                      )}
                      <span className="text-xs text-muted">
                        {approval.taskTitle
                          ? t(`任务：${approval.taskTitle}，`, `Task: ${approval.taskTitle}, `)
                          : ''}
                        {t('发起人', 'Requested by')} {approval.requestedBy ?? 'system'}
                        {t('，', ', ')}
                        {formatDateTime(approval.createdAt)}
                        {approval.decidedAt
                          ? t(
                              `，处理于 ${formatDateTime(approval.decidedAt)}`,
                              `, decided ${formatDateTime(approval.decidedAt)}`,
                            )
                          : ''}
                      </span>
                    </div>
                  </div>
                </div>

                <div className="mt-3">
                  <ApprovalDetail approval={approval} />
                </div>

                {pending ? (
                  <div className="mt-3 border-t border-line/60 pt-3">
                    <div className="flex flex-wrap items-center gap-2">
                      <Input
                        value={notes[approval.id] ?? ''}
                        onChange={(e) =>
                          setNotes((prev) => ({ ...prev, [approval.id]: e.target.value }))
                        }
                        placeholder={t('审批备注（可选）', 'Note (optional)')}
                        className="h-8 min-w-[220px] flex-1 py-1 text-xs"
                      />
                      <Button
                        size="sm"
                        loading={deciding === `${approval.id}:yes`}
                        onClick={() => decide(approval, true)}
                      >
                        <Check size={13} />
                        {t('批准', 'Approve')}
                      </Button>
                      <Button
                        variant="danger"
                        size="sm"
                        loading={deciding === `${approval.id}:no`}
                        onClick={() => decide(approval, false)}
                      >
                        <X size={13} />
                        {t('拒绝', 'Deny')}
                      </Button>
                    </div>
                    <label className="mt-2 flex w-fit cursor-pointer items-center gap-1.5 text-xs text-muted">
                      <input
                        type="checkbox"
                        checked={Boolean(remember[approval.id])}
                        onChange={(e) =>
                          setRemember((prev) => ({
                            ...prev,
                            [approval.id]: e.target.checked,
                          }))
                        }
                        className="h-3 w-3 accent-primary"
                      />
                      {t(
                        '记住此类操作的选择（自动生成 allow / deny 规则）',
                        'Remember this choice (auto-creates an allow / deny rule)',
                      )}
                    </label>
                  </div>
                ) : (
                  approval.decisionNote && (
                    <p className="mt-2 text-xs text-muted">
                      {t('处理备注：', 'Note: ')}
                      {approval.decisionNote}
                    </p>
                  )
                )}
              </Card>
            );
          })}
        </div>
      )}

      <Card className="mt-5">
        <SectionTitle
          icon={ShieldCheck}
          title={t('审批规则', 'Approval rules')}
          description={t(
            '按优先级匹配工具、命令与路径，决定 allow / deny / ask',
            'Match tools, commands and paths by priority to decide allow / deny / ask',
          )}
          extra={
            <Button variant="outline" size="xs" onClick={openRuleCreate}>
              <Plus size={12} />
              {t('新建规则', 'New rule')}
            </Button>
          }
        />
        {rules.length === 0 ? (
          <Empty
            title={t('还没有审批规则', 'No approval rules yet')}
            description={t(
              '规则按优先级匹配工具、命令与路径，决定 allow / deny / ask',
              'Rules match tools, commands and paths by priority to decide allow / deny / ask',
            )}
            className="py-8"
          />
        ) : (
          <div className="divide-y divide-line/60 overflow-hidden rounded-xl border border-line/60">
            {rules.map((rule) => (
              <div
                key={rule.id}
                className="group flex flex-wrap items-center gap-3 px-3.5 py-3"
              >
                <Badge tone={actionTone(rule.action)} className="w-12 justify-center">
                  {rule.action}
                </Badge>
                <div className="min-w-0 flex-1">
                  <p className="truncate text-xs font-medium text-ink">{matcherText(rule)}</p>
                  <p className="mt-0.5 text-xs text-muted">
                    {t('优先级', 'Priority')} {rule.priority}
                    {t('，', ', ')}
                    {rule.scopeType === 'global' ? t('全局', 'Global') : rule.scopeType}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <Toggle
                    checked={rule.enabled}
                    onChange={(value) => toggleRule(rule, value)}
                    label={t('启用规则', 'Enable rule')}
                  />
                  <Button
                    variant="ghost"
                    size="xs"
                    onClick={() => openRuleEdit(rule)}
                    title={t('编辑', 'Edit')}
                  >
                    <Pencil size={12} />
                  </Button>
                  <Button
                    variant="ghost"
                    size="xs"
                    className="hover:text-danger"
                    onClick={() => setDeletingRule(rule)}
                    title={t('删除', 'Delete')}
                  >
                    <Trash2 size={12} />
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>

      <Modal
        open={ruleModalOpen}
        onClose={() => setRuleModalOpen(false)}
        title={
          editingRule ? t('编辑审批规则', 'Edit approval rule') : t('新建审批规则', 'New approval rule')
        }
        description={t(
          '至少填写一项匹配条件，优先级数字越小越先匹配',
          'Fill in at least one condition; lower priority numbers match first',
        )}
        footer={
          <>
            <Button variant="outline" size="sm" onClick={() => setRuleModalOpen(false)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button size="sm" loading={savingRule} onClick={saveRule}>
              {editingRule ? t('保存', 'Save') : t('创建', 'Create')}
            </Button>
          </>
        }
      >
        <div className="space-y-3">
          <Field
            label={t('工具名称', 'Tool name')}
            hint={t('例如 Bash、Write、Edit、Read', 'e.g. Bash, Write, Edit, Read')}
          >
            <Input
              value={ruleForm.tool}
              onChange={(e) => setRuleForm({ ...ruleForm, tool: e.target.value })}
              placeholder="Bash"
              className="font-mono text-xs"
            />
          </Field>
          <Field
            label={t('命令包含', 'Command contains')}
            hint={t(
              '模糊匹配；含 &&、| 等组合符的命令不会命中 allow 规则',
              'Fuzzy match; commands with && or | never hit allow rules',
            )}
          >
            <Input
              value={ruleForm.commandContains}
              onChange={(e) => setRuleForm({ ...ruleForm, commandContains: e.target.value })}
              placeholder="npm run"
              className="font-mono text-xs"
            />
          </Field>
          <Field
            label={t('命令精确匹配', 'Exact command')}
            hint={t(
              '完整命令完全一致时才命中，推荐用于 remember 场景',
              'Matches only when the full command is identical; best for remember scenarios',
            )}
          >
            <Input
              value={ruleForm.commandExact}
              onChange={(e) => setRuleForm({ ...ruleForm, commandExact: e.target.value })}
              placeholder="npm run build"
              className="font-mono text-xs"
            />
          </Field>
          <Field
            label={t('路径 Glob', 'Path glob')}
            hint={t('例如 src/** 或 **/*.rs', 'e.g. src/** or **/*.rs')}
          >
            <Input
              value={ruleForm.pathGlob}
              onChange={(e) => setRuleForm({ ...ruleForm, pathGlob: e.target.value })}
              placeholder="src/**"
              className="font-mono text-xs"
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label={t('动作', 'Action')}>
              <Select
                value={ruleForm.action}
                onChange={(e) => setRuleForm({ ...ruleForm, action: e.target.value })}
              >
                <option value="allow">{t('allow（允许）', 'allow')}</option>
                <option value="deny">{t('deny（拒绝）', 'deny')}</option>
                <option value="ask">{t('ask（询问）', 'ask')}</option>
              </Select>
            </Field>
            <Field label={t('优先级', 'Priority')}>
              <Input
                value={ruleForm.priority}
                onChange={(e) => setRuleForm({ ...ruleForm, priority: e.target.value })}
                inputMode="numeric"
                className="font-mono text-xs"
              />
            </Field>
          </div>
        </div>
      </Modal>

      <Modal
        open={Boolean(deletingRule)}
        onClose={() => setDeletingRule(null)}
        title={t('删除审批规则', 'Delete approval rule')}
        width="max-w-sm"
        footer={
          <>
            <Button variant="outline" size="sm" onClick={() => setDeletingRule(null)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button variant="danger" size="sm" onClick={confirmDeleteRule}>
              {t('删除', 'Delete')}
            </Button>
          </>
        }
      >
        <p className="text-xs leading-5 text-muted">
          {t('将删除规则「', 'Delete rule "')}
          {deletingRule ? matcherText(deletingRule) : ''}
          {t(
            '」，之后同类工具调用会重新进入审批。',
            '". Similar tool calls will require approval again.',
          )}
        </p>
      </Modal>
    </PageScroll>
  );
}
