import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  Clock,
  ListChecks,
  Play,
  Plus,
  RotateCcw,
  Search,
  Square,
  Trash2,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import type { Task, Team } from '../lib/types';
import { priorityMeta } from '../lib/labels';
import { tr, useI18n } from '../lib/i18n';
import { TASK_STAGE_ORDER, isReviewPending, taskStage } from '../lib/taskStage';
import { formatTokens, fromNow, microsToUsd } from '../lib/format';
import { useAppStore } from '../store';
import {
  Badge,
  Button,
  ConfirmDialog,
  Empty,
  Field,
  Input,
  Loading,
  Modal,
  PageHeader,
  PageScroll,
  Select,
  TaskStatusBadge,
  Textarea,
  cn,
} from '../components/ui';

interface FormState {
  projectId: string;
  teamId: string;
  title: string;
  description: string;
  acceptance: string;
  priority: number;
  mandate: string;
  isolated: boolean;
}

const PRIORITY_OPTIONS = [
  {
    value: 1,
    get label() {
      return tr('紧急', 'Urgent');
    },
  },
  {
    value: 2,
    get label() {
      return tr('高', 'High');
    },
  },
  {
    value: 3,
    get label() {
      return tr('普通', 'Normal');
    },
  },
  {
    value: 4,
    get label() {
      return tr('低', 'Low');
    },
  },
];

export default function Tasks() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const projects = useAppStore((s) => s.projects);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [teams, setTeams] = useState<Team[]>([]);
  const [projectFilter, setProjectFilter] = useState('');
  const [loading, setLoading] = useState(true);
  const [total, setTotal] = useState(0);
  const [statusCounts, setStatusCounts] = useState<Record<string, number>>({});
  const [reviewTotal, setReviewTotal] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [search, setSearch] = useState('');
  const [query, setQuery] = useState('');
  const [modalOpen, setModalOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState<Task | null>(null);
  const [deleteLoading, setDeleteLoading] = useState(false);
  const [busy, setBusy] = useState('');
  const [dispatchTarget, setDispatchTarget] = useState<Task | null>(null);
  const [dispatchLoading, setDispatchLoading] = useState(false);
  const [form, setForm] = useState<FormState>({
    projectId: '',
    teamId: '',
    title: '',
    description: '',
    acceptance: '',
    priority: 3,
    mandate: 'ask',
    isolated: false,
  });

  useEffect(() => {
    api
      .teamsList()
      .then(setTeams)
      .catch((err) => toast.error(errorMessage(err)));
  }, []);

  // 搜索去抖，避免每次输入都重新查询。
  useEffect(() => {
    const timer = window.setTimeout(() => setQuery(search.trim()), 300);
    return () => window.clearTimeout(timer);
  }, [search]);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const page = await api.tasksPage({
        projectId: projectFilter || undefined,
        query: query || undefined,
        limit: 60,
      });
      setTasks(page.items);
      setTotal(page.total);
      setStatusCounts(page.statusCounts);
      setReviewTotal(page.reviewCount);
      setHasMore(page.hasMore);
      setCursor(page.nextCursor ?? null);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [projectFilter, query]);

  useEffect(() => {
    load();
  }, [load]);

  async function loadMore() {
    if (!hasMore || !cursor || loadingMore) return;
    setLoadingMore(true);
    try {
      const page = await api.tasksPage({
        projectId: projectFilter || undefined,
        query: query || undefined,
        cursor,
        limit: 60,
      });
      setTasks((prev) => [...prev, ...page.items]);
      setTotal(page.total);
      setStatusCounts(page.statusCounts);
      setReviewTotal(page.reviewCount);
      setHasMore(page.hasMore);
      setCursor(page.nextCursor ?? null);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setLoadingMore(false);
    }
  }

  function openCreate() {
    setForm({
      projectId: projectFilter || projects[0]?.id || '',
      teamId: '',
      title: '',
      description: '',
      acceptance: '',
      priority: 3,
      mandate: 'ask',
      isolated: false,
    });
    setModalOpen(true);
  }

  async function createTask() {
    if (!form.projectId) {
      toast.error(t('请选择项目', 'Select a project'));
      return;
    }
    if (!form.title.trim()) {
      toast.error(t('请填写任务标题', 'Enter a task title'));
      return;
    }
    setSaving(true);
    try {
      await api.taskCreate({
        projectId: form.projectId,
        teamId: form.teamId || null,
        title: form.title.trim(),
        description: form.description.trim() || undefined,
        acceptance: form.acceptance.trim() || undefined,
        mandate: form.mandate,
        isolated: form.isolated,
        priority: form.priority,
      });
      toast.success(t('任务已创建', 'Task created'));
      setModalOpen(false);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  const formProject = projects.find((project) => project.id === form.projectId) ?? null;
  const formIsolateUnavailable = formProject?.isGit === false;

  async function runAction(task: Task, action: 'dispatch' | 'cancel' | 'reset') {
    // 非隔离运行会直接改项目目录，先让用户确认并说明快照/回滚边界。
    if (action === 'dispatch' && !task.workDir) {
      setDispatchTarget(task);
      return;
    }
    setBusy(`${task.id}:${action}`);
    try {
      if (action === 'dispatch') {
        const snapshot = await api.taskDispatch(task.id);
        toast.success(
          snapshot
            ? t('任务已派发，已生成运行前快照', 'Task dispatched with a pre-run snapshot')
            : t('任务已派发', 'Task dispatched'),
        );
      } else if (action === 'cancel') {
        await api.taskCancel(task.id);
        toast.success(t('任务已停止，状态变为已阻塞', 'Task stopped and marked blocked'));
      } else {
        await api.taskReset(task.id);
        toast.success(t('任务已重置', 'Task reset'));
      }
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy('');
    }
  }

  async function confirmDispatch() {
    const task = dispatchTarget;
    if (!task) return;
    setDispatchLoading(true);
    try {
      const snapshot = await api.taskDispatch(task.id);
      toast.success(
        snapshot
          ? t('任务已派发，已生成运行前快照', 'Task dispatched with a pre-run snapshot')
          : t('任务已派发', 'Task dispatched'),
      );
      setDispatchTarget(null);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setDispatchLoading(false);
    }
  }

  async function confirmDelete() {
    if (!deleting) return;
    setDeleteLoading(true);
    try {
      await api.taskDelete(deleting.id);
      toast.success(t('任务已删除', 'Task deleted'));
      setDeleting(null);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setDeleteLoading(false);
    }
  }

  const formTeams = teams.filter((team) => team.projectId === form.projectId);
  const runningCount = statusCounts.running ?? 0;
  const reviewCount = reviewTotal;

  return (
    <PageScroll>
      <PageHeader
        title={t('任务看板', 'Task board')}
        subtitle={t(
          `共 ${total} 个任务${tasks.length < total ? `，已加载 ${tasks.length}` : ''}，${runningCount} 个运行中${reviewCount > 0 ? `，${reviewCount} 个待审查` : ''}`,
          `${total} tasks${tasks.length < total ? `, ${tasks.length} loaded` : ''}, ${runningCount} running${reviewCount > 0 ? `, ${reviewCount} awaiting review` : ''}`,
        )}
        actions={
          <>
            <div className="relative">
              <Search
                size={13}
                className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted"
              />
              <Input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t('搜索标题或描述', 'Search title or description')}
                className="w-52 pl-8"
              />
            </div>
            <Select
              value={projectFilter}
              onChange={(e) => setProjectFilter(e.target.value)}
              className="w-48"
            >
              <option value="">{t('全部项目', 'All projects')}</option>
              {projects.map((project) => (
                <option key={project.id} value={project.id}>
                  {project.name}
                </option>
              ))}
            </Select>
            <Button size="sm" onClick={openCreate} disabled={projects.length === 0}>
              <Plus size={14} />
              {t('新建任务', 'New task')}
            </Button>
          </>
        }
      />

      {loading ? (
        <Loading />
      ) : total === 0 ? (
        <Empty
          icon={ListChecks}
          title={query ? t('没有匹配任务', 'No matching tasks') : t('暂无任务', 'No tasks yet')}
          description={
            query
              ? t(`没有找到包含「${query}」的任务`, `No tasks match "${query}"`)
              : projects.length === 0
                ? t('请先创建项目与团队', 'Create a project and team first')
                : t(
                    '新建任务并选择团队，即可开始协同执行',
                    'Create a task and pick a team to start collaborating',
                  )
          }
          action={
            query ? (
              <Button size="sm" variant="outline" onClick={() => setSearch('')}>
                {t('清除搜索', 'Clear search')}
              </Button>
            ) : projects.length > 0 ? (
              <Button size="sm" onClick={openCreate}>
                <Plus size={14} />
                {t('新建任务', 'New task')}
              </Button>
            ) : undefined
          }
          className="py-16"
        />
      ) : (
        <div className="space-y-4">
          <div className="-mx-1 flex gap-4 overflow-x-auto px-1 pb-2">
          {TASK_STAGE_ORDER.map((status) => {
            const meta = taskStage(status);
            const items = tasks.filter((task) => task.status === status);
            return (
              <div key={status} className="flex w-[280px] shrink-0 flex-col">
                <div className="mb-2 flex items-center justify-between px-1">
                  <div className="flex items-center gap-2">
                    <span className={cn('h-2 w-2 rounded-full', meta.dot)} />
                    <span className="text-sm font-semibold text-ink">{meta.label}</span>
                    <span className="min-w-[20px] rounded-full bg-soft px-1.5 py-0.5 text-center text-xs font-medium text-muted">
                      {statusCounts[status] ?? items.length}
                    </span>
                  </div>
                </div>
                <div className="flex min-h-[420px] flex-1 flex-col gap-2.5 rounded-xl border border-line/70 bg-elevated/60 p-2.5">
                  {items.length === 0 ? (
                    <p className="rounded-lg border border-dashed border-line/70 py-8 text-center text-xs text-muted/50">
                      {t('暂无任务', 'No tasks')}
                    </p>
                  ) : (
                    items.map((task) => {
                      const priority = priorityMeta(task.priority);
                      return (
                        <div
                          key={task.id}
                          onClick={() => navigate(`/tasks/${task.id}`)}
                          className="group cursor-pointer rounded-lg border border-line bg-card p-3 transition-colors hover:border-primary/40"
                        >
                          <div className="flex items-start justify-between gap-2">
                            <p className="line-clamp-2 text-sm font-medium leading-5 text-ink">
                              {task.title}
                            </p>
                            <Badge className="shrink-0">
                              <span className={cn('h-1.5 w-1.5 rounded-full', priority.dot)} />
                              {priority.label}
                            </Badge>
                          </div>
                          <div className="mt-2 flex items-center justify-between gap-2 text-xs text-muted">
                            <span className="flex min-w-0 items-center gap-1.5">
                              <span className="truncate">
                                {task.teamName ?? t('未选择团队', 'No team')}
                              </span>
                              {taskStage(task.status).waitingOn === 'you' && (
                                <Badge className="shrink-0">
                                  <span className="h-1.5 w-1.5 rounded-full bg-warn" />
                                  {t('等你', 'Needs you')}
                                </Badge>
                              )}
                              {isReviewPending(task) && (
                                <Badge className="shrink-0">
                                  <span className="h-1.5 w-1.5 rounded-full bg-warn" />
                                  {t('待审查', 'Review pending')}
                                </Badge>
                              )}
                            </span>
                            <span className="shrink-0 font-mono">
                              {formatTokens(task.actualTokens)}
                            </span>
                          </div>
                          <div className="mt-1 flex items-center justify-between gap-2 text-xs text-muted/70">
                            <span className="inline-flex items-center gap-1">
                              <Clock size={11} />
                              {fromNow(task.createdAt)}
                            </span>
                            <span className="shrink-0 font-mono">
                              {microsToUsd(task.actualCostMicros)}
                            </span>
                          </div>
                          <div className="mt-2.5 flex items-center gap-0.5 border-t border-line/60 pt-1.5">
                            <Button
                              variant="ghost"
                              size="xs"
                              title={t('派发', 'Dispatch')}
                              className="opacity-60 group-hover:opacity-100"
                              disabled={busy === `${task.id}:dispatch`}
                              onClick={(event) => {
                                event.stopPropagation();
                                runAction(task, 'dispatch');
                              }}
                            >
                              <Play size={12} />
                            </Button>
                            <Button
                              variant="ghost"
                              size="xs"
                              title={t('取消', 'Cancel')}
                              className="opacity-60 group-hover:opacity-100"
                              disabled={busy === `${task.id}:cancel`}
                              onClick={(event) => {
                                event.stopPropagation();
                                runAction(task, 'cancel');
                              }}
                            >
                              <Square size={12} />
                            </Button>
                            <Button
                              variant="ghost"
                              size="xs"
                              title={t('重置', 'Reset')}
                              className="opacity-60 group-hover:opacity-100"
                              onClick={(event) => {
                                event.stopPropagation();
                                runAction(task, 'reset');
                              }}
                            >
                              <RotateCcw size={12} />
                            </Button>
                            <Button
                              variant="ghost"
                              size="xs"
                              className="ml-auto opacity-60 hover:text-danger group-hover:opacity-100"
                              title={t('删除', 'Delete')}
                              onClick={(event) => {
                                event.stopPropagation();
                                setDeleting(task);
                              }}
                            >
                              <Trash2 size={12} />
                            </Button>
                          </div>
                        </div>
                      );
                    })
                  )}
                </div>
              </div>
            );
          })}
          </div>
          {hasMore && (
            <div className="flex justify-center">
              <Button variant="outline" size="sm" loading={loadingMore} onClick={loadMore}>
                {t(
                  `加载更多（剩余 ${Math.max(total - tasks.length, 0)} 个）`,
                  `Load more (${Math.max(total - tasks.length, 0)} remaining)`,
                )}
              </Button>
            </div>
          )}
        </div>
      )}

      <Modal
        open={modalOpen}
        onClose={() => setModalOpen(false)}
        title={t('新建任务', 'New task')}
        description={t(
          '任务创建后进入收件箱，可在看板中派发',
          'New tasks land in the inbox and can be dispatched from the board',
        )}
        width="max-w-xl"
        footer={
          <>
            <Button variant="outline" size="sm" onClick={() => setModalOpen(false)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button size="sm" loading={saving} onClick={createTask}>
              {t('创建任务', 'Create task')}
            </Button>
          </>
        }
      >
        <div className="space-y-3">
          <div className="grid grid-cols-2 gap-3">
            <Field label={t('项目', 'Project')}>
              <Select
                value={form.projectId}
                onChange={(e) => {
                  const next = projects.find((project) => project.id === e.target.value);
                  setForm({
                    ...form,
                    projectId: e.target.value,
                    teamId: '',
                    isolated: next?.isGit === false ? false : form.isolated,
                  });
                }}
              >
                {projects.map((project) => (
                  <option key={project.id} value={project.id}>
                    {project.name}
                  </option>
                ))}
              </Select>
            </Field>
            <Field
              label={t('团队', 'Team')}
              hint={t('派发前必须选择团队', 'Required before dispatch')}
            >
              <Select
                value={form.teamId}
                onChange={(e) => setForm({ ...form, teamId: e.target.value })}
              >
                <option value="">{t('暂不选择', 'None yet')}</option>
                {formTeams.map((team) => (
                  <option key={team.id} value={team.id}>
                    {team.name}
                  </option>
                ))}
              </Select>
            </Field>
          </div>
          <Field label={t('任务标题', 'Title')}>
            <Input
              value={form.title}
              onChange={(e) => setForm({ ...form, title: e.target.value })}
              placeholder={t('要完成什么', 'What needs to be done')}
            />
          </Field>
          <Field label={t('描述', 'Description')}>
            <Textarea
              rows={3}
              value={form.description}
              onChange={(e) => setForm({ ...form, description: e.target.value })}
              placeholder={t('背景、范围与约束', 'Context, scope and constraints')}
            />
          </Field>
          <Field label={t('验收标准', 'Acceptance criteria')}>
            <Textarea
              rows={3}
              value={form.acceptance}
              onChange={(e) => setForm({ ...form, acceptance: e.target.value })}
              placeholder={t(
                '逐条列出可验证的验收条件',
                'List verifiable acceptance conditions',
              )}
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field
              label={t('审批授权', 'Approval mandate')}
              hint={t(
                '自动授权仍保留硬风险与目录外写入确认',
                'Auto still prompts for hard risks and writes outside the workspace',
              )}
            >
              <Select
                value={form.mandate}
                onChange={(e) => setForm({ ...form, mandate: e.target.value })}
              >
                <option value="ask">{t('逐次确认', 'Ask each time')}</option>
                <option value="auto">{t('任务内自动', 'Auto within task')}</option>
              </Select>
            </Field>
            <Field label={t('优先级', 'Priority')}>
              <Select
                value={form.priority}
                onChange={(e) => setForm({ ...form, priority: Number(e.target.value) })}
              >
                {PRIORITY_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </Select>
            </Field>
          </div>
          <label
            className={cn(
              'flex items-center gap-2 text-xs',
              formIsolateUnavailable ? 'text-warn' : 'text-muted',
            )}
          >
            <input
              type="checkbox"
              checked={form.isolated}
              disabled={formIsolateUnavailable}
              onChange={(e) => setForm({ ...form, isolated: e.target.checked })}
              className="h-3.5 w-3.5 accent-primary"
            />
            {formIsolateUnavailable
              ? t(
                  '隔离工作区不可用：当前项目不是 Git 仓库',
                  'Isolated workspace unavailable: this project is not a Git repository',
                )
              : t(
                  '隔离工作区（仅 git 仓库有效，创建独立 worktree）',
                  'Isolated workspace (Git repos only; creates a separate worktree)',
                )}
          </label>
        </div>
      </Modal>

      <ConfirmDialog
        open={Boolean(deleting)}
        onClose={() => setDeleting(null)}
        onConfirm={confirmDelete}
        title={t(`删除任务「${deleting?.title ?? ''}」`, `Delete task "${deleting?.title ?? ''}"`)}
        description={
          deleting?.workDir
            ? t(
                '任务消息、运行记录与审批将一并删除，隔离工作区中的未提交与未应用改动也会丢弃',
                'Messages, runs and approvals will be deleted, along with uncommitted and unapplied changes in the isolated workspace',
              )
            : t('任务消息、运行记录与审批将一并删除', 'Messages, runs and approvals will be deleted')
        }
        loading={deleteLoading}
      />

      <ConfirmDialog
        open={Boolean(dispatchTarget)}
        onClose={() => setDispatchTarget(null)}
        onConfirm={confirmDispatch}
        title={t(
          `派发任务「${dispatchTarget?.title ?? ''}」`,
          `Dispatch task "${dispatchTarget?.title ?? ''}"`,
        )}
        description={t(
          '任务未启用隔离工作区，运行会直接修改项目目录。',
          'This task has no isolated workspace; the run will modify the project directory directly.',
        )}
        hint={
          dispatchTarget &&
          projects.find((project) => project.id === dispatchTarget.projectId)?.isGit === false
            ? t(
                '该项目不是 Git 仓库，改动无法自动回滚；建议先手动备份或初始化 Git。',
                'This project is not a Git repository, so changes cannot be rolled back automatically. Back up manually or initialize Git first.',
              )
            : t(
                '派发前会自动生成一份只读补丁快照，可在数据目录的 snapshots 中查看。',
                'A read-only patch snapshot is created before dispatch and can be found in the snapshots folder of the data directory.',
              )
        }
        confirmText={t('派发', 'Dispatch')}
        loading={dispatchLoading}
      />
    </PageScroll>
  );
}
