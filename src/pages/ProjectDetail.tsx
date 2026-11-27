import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import {
  Activity,
  ArrowRight,
  CircleDollarSign,
  Coins,
  FolderOpen,
  ListChecks,
  Plus,
  RefreshCw,
  ShieldAlert,
  Users,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import type { EventItem, ScopedUsage, Task, Team } from '../lib/types';
import {
  formatDateTime,
  formatFullDateTime,
  formatNumber,
  formatTokens,
  fromNow,
  microsToUsd,
} from '../lib/format';
import { eventColor, eventLabel, projectKindLabel } from '../lib/labels';
import { useI18n } from '../lib/i18n';
import { isReviewPending, taskStage } from '../lib/taskStage';
import { IN_PROGRESS_STATUSES } from '../lib/taskStatus';
import { useAppStore } from '../store';
import {
  Badge,
  Button,
  Card,
  Empty,
  Loading,
  PageHeader,
  PageScroll,
  ProgressBar,
  SectionTitle,
  cn,
} from '../components/ui';

function Metric({
  label,
  value,
  sub,
  icon: Icon,
}: {
  label: string;
  value: string;
  sub: string;
  icon: typeof Coins;
}) {
  return (
    <div className="flex flex-col bg-card p-5">
      <div className="flex items-center gap-2 text-xs text-muted">
        <Icon size={13} className="shrink-0" />
        <p>{label}</p>
      </div>
      <p className="mt-2 text-2xl font-semibold leading-7 tracking-tight tabular-nums text-ink">
        {value}
      </p>
      <p className="mt-0.5 text-xs leading-4 text-muted">{sub}</p>
    </div>
  );
}

export default function ProjectDetail() {
  const { id = '' } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const { t } = useI18n();
  const projects = useAppStore((s) => s.projects);
  const coreLoaded = useAppStore((s) => s.loaded);
  const loadCore = useAppStore((s) => s.loadCore);

  const project = projects.find((item) => item.id === id) ?? null;

  const [tasks, setTasks] = useState<Task[]>([]);
  const [taskTotal, setTaskTotal] = useState(0);
  const [statusCounts, setStatusCounts] = useState<Record<string, number>>({});
  const [reviewTotal, setReviewTotal] = useState(0);
  const [attentionTotal, setAttentionTotal] = useState(0);
  const [teams, setTeams] = useState<Team[]>([]);
  const [usage, setUsage] = useState<ScopedUsage | null>(null);
  const [events, setEvents] = useState<EventItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const load = useCallback(
    async (silent = false) => {
      if (!id) return;
      if (!silent) setLoading(true);
      try {
        const [taskPage, attentionPage, teamList, usageList, eventList] = await Promise.all([
          api.tasksPage({ projectId: id, limit: 50 }),
          api.tasksPage({ projectId: id, attentionOnly: true, limit: 1 }),
          api.teamsList(id),
          api.usageByProject(),
          api.eventsList({ projectId: id, limit: 10 }),
        ]);
        setTasks(taskPage.items);
        setTaskTotal(taskPage.total);
        setStatusCounts(taskPage.statusCounts);
        setReviewTotal(taskPage.reviewCount);
        setAttentionTotal(attentionPage.total);
        setTeams(teamList);
        setUsage(usageList.find((item) => item.key === id) ?? null);
        setEvents(eventList);
        setError(null);
      } catch (err) {
        setError(errorMessage(err));
        if (!silent) toast.error(errorMessage(err));
      } finally {
        setLoading(false);
        setRefreshing(false);
      }
    },
    [id],
  );

  useEffect(() => {
    load();
  }, [load]);

  const stats = useMemo(() => {
    const active = IN_PROGRESS_STATUSES.reduce(
      (sum, status) => sum + (statusCounts[status] ?? 0),
      0,
    );
    const completed = statusCounts.completed ?? 0;
    const review = reviewTotal;
    const attention = attentionTotal;
    const tokens = usage?.tokens ?? tasks.reduce((sum, task) => sum + task.actualTokens, 0);
    const costMicros =
      usage?.costMicros ?? tasks.reduce((sum, task) => sum + task.actualCostMicros, 0);
    const seats = teams.reduce((sum, team) => sum + team.seatsCount, 0);
    const assigned = teams.reduce((sum, team) => sum + team.assignedCount, 0);
    return { active, completed, review, attention, tokens, costMicros, seats, assigned };
  }, [tasks, teams, usage, statusCounts, reviewTotal, attentionTotal]);

  async function refresh() {
    setRefreshing(true);
    await Promise.all([loadCore(), load(true)]);
  }

  if (!coreLoaded) {
    return (
      <PageScroll>
        <Loading />
      </PageScroll>
    );
  }

  if (!project) {
    return (
      <PageScroll>
        <PageHeader
          back={{ to: '/projects', label: t('返回新建任务', 'Back to new task') }}
          title={t('项目不存在', 'Project not found')}
        />
        <Empty
          icon={FolderOpen}
          title={t('未找到该项目', 'Project not found')}
          description={t(
            '项目可能已被删除，或链接中的项目编号不正确',
            'It may have been deleted, or the project ID in the link is incorrect',
          )}
          action={
            <Button size="sm" variant="outline" onClick={() => navigate('/projects')}>
              {t('返回项目列表', 'Back to projects')}
            </Button>
          }
        />
      </PageScroll>
    );
  }

  return (
    <PageScroll>
      <PageHeader
        back={{ to: '/projects', label: t('返回新建任务', 'Back to new task') }}
        title={
          <span className="flex min-w-0 items-center gap-2">
            <span className="truncate">{project.name}</span>
            <Badge tone={project.isGit ? 'success' : 'warn'}>
              {project.isGit ? t('Git 仓库', 'Git repo') : t('非 Git', 'Not Git')}
            </Badge>
          </span>
        }
        subtitle={
          <span className="flex flex-wrap items-center gap-2">
            <span className="break-all font-mono text-xs">{project.path}</span>
            <span className="text-muted/40">/</span>
            <span>{projectKindLabel(project.kind)}</span>
            <span className="text-muted/40">/</span>
            <span>
              {t(
                `创建于 ${formatDateTime(project.createdAt)}`,
                `Created ${formatDateTime(project.createdAt)}`,
              )}
            </span>
          </span>
        }
        actions={
          <>
            <Button variant="outline" size="sm" onClick={refresh}>
              <RefreshCw size={13} className={refreshing ? 'animate-spin' : ''} />
              {t('刷新', 'Refresh')}
            </Button>
            <Button size="sm" onClick={() => navigate('/projects')}>
              <Plus size={14} />
              {t('新建任务', 'New task')}
            </Button>
          </>
        }
      />

      {loading ? (
        <Loading />
      ) : error && tasks.length === 0 ? (
        <Empty
          icon={ShieldAlert}
          title={t('无法加载项目数据', 'Could not load project data')}
          description={error}
          action={
            <Button size="sm" variant="outline" onClick={() => load()}>
              {t('重试', 'Retry')}
            </Button>
          }
        />
      ) : (
        <div className="space-y-5">
          <Card className="overflow-hidden p-0">
            <div className="grid grid-cols-1 gap-px bg-line/60 sm:grid-cols-2 xl:grid-cols-4">
              <Metric
                label={t('任务总数', 'Total tasks')}
                value={String(taskTotal)}
                sub={t(
                  `运行中 ${stats.active}，已完成 ${stats.completed}`,
                  `Active ${stats.active}, completed ${stats.completed}`,
                )}
                icon={ListChecks}
              />
              <Metric
                label={t('需要你', 'Needs you')}
                value={String(stats.attention)}
                sub={t(
                  `待审查 ${stats.review}，其余为待审批或阻塞`,
                  `Needs review ${stats.review}; others awaiting approval or blocked`,
                )}
                icon={ShieldAlert}
              />
              <Metric
                label={t('累计 Tokens', 'Total tokens')}
                value={formatTokens(stats.tokens)}
                sub={t(
                  `${formatNumber(usage?.rounds ?? 0)} 轮对话`,
                  `${formatNumber(usage?.rounds ?? 0)} rounds`,
                )}
                icon={Coins}
              />
              <Metric
                label={t('累计成本', 'Total cost')}
                value={microsToUsd(stats.costMicros)}
                sub={t(
                  `${teams.length} 个团队，${stats.assigned}/${stats.seats} 席位已指派`,
                  `${teams.length} teams, ${stats.assigned}/${stats.seats} seats assigned`,
                )}
                icon={CircleDollarSign}
              />
            </div>
          </Card>

          <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
            <Card className="lg:col-span-2">
              <SectionTitle
                icon={ListChecks}
                title={t('项目任务', 'Project tasks')}
                description={t(
                  `共 ${taskTotal} 个任务，按优先级与创建时间排序`,
                  `${taskTotal} tasks, sorted by priority and creation time`,
                )}
                extra={
                  <Link
                    to="/tasks"
                    className="inline-flex items-center gap-1 text-xs text-muted hover:text-primary"
                  >
                    {t('任务看板', 'Task board')} <ArrowRight size={11} />
                  </Link>
                }
              />
              {taskTotal === 0 ? (
                <Empty
                  title={t('还没有任务', 'No tasks yet')}
                  description={t(
                    '在「新建任务」描述目标后创建，再派发给团队',
                    'Create one from “New task”, then assign it to a team',
                  )}
                  action={
                    <Button size="sm" variant="outline" onClick={() => navigate('/projects')}>
                      {t('新建任务', 'New task')}
                    </Button>
                  }
                />
              ) : (
                <div className="divide-y divide-line/60 overflow-hidden rounded-xl border border-line/70">
                  {tasks.slice(0, 10).map((task) => {
                    const stage = taskStage(task.status);
                    const review = isReviewPending(task);
                    return (
                      <button
                        key={task.id}
                        type="button"
                        onClick={() => navigate(`/tasks/${task.id}`)}
                        className="flex w-full items-center gap-3 px-3.5 py-2.5 text-left transition-colors hover:bg-hover focus-visible:bg-hover focus-visible:outline-none"
                      >
                        <span
                          className={cn(
                            'h-1.5 w-1.5 shrink-0 rounded-full',
                            review ? 'bg-warn' : stage.dot,
                          )}
                        />
                        <div className="min-w-0 flex-1">
                          <p className="truncate text-sm text-ink">{task.title}</p>
                          <p className="mt-0.5 truncate text-xs text-muted">
                            {t(
                              `${task.teamName ?? '未选择团队'}，${formatDateTime(task.createdAt)}`,
                              `${task.teamName ?? 'No team'}, ${formatDateTime(task.createdAt)}`,
                            )}
                          </p>
                        </div>
                        {review && <Badge tone="warn">{t('待审查', 'Needs review')}</Badge>}
                        <span
                          className={cn(
                            'shrink-0 text-xs font-medium',
                            review ? 'text-warn' : stage.text,
                          )}
                        >
                          {review ? t('待审查', 'Needs review') : stage.label}
                        </span>
                        <span className="w-16 shrink-0 text-right font-mono text-xs text-muted">
                          {microsToUsd(task.actualCostMicros)}
                        </span>
                      </button>
                    );
                  })}
                </div>
              )}
              {taskTotal > 10 && (
                <p className="pt-2 text-xs text-muted">
                  {t(`另有 ${taskTotal - 10} 个任务`, `${taskTotal - 10} more tasks`)}
                </p>
              )}
            </Card>

            <Card>
              <SectionTitle icon={FolderOpen} title={t('项目信息', 'Project info')} />
              <dl className="space-y-3 text-xs leading-5">
                <div>
                  <dt className="text-muted">{t('项目目录', 'Project directory')}</dt>
                  <dd className="mt-0.5 break-all font-mono text-ink/90">{project.path}</dd>
                </div>
                <div className="flex items-start justify-between gap-3">
                  <dt className="text-muted">{t('项目类型', 'Project type')}</dt>
                  <dd className="text-ink/90">{projectKindLabel(project.kind)}</dd>
                </div>
                <div className="flex items-start justify-between gap-3">
                  <dt className="text-muted">{t('Git 状态', 'Git status')}</dt>
                  <dd className={cn('text-right', project.isGit ? 'text-success' : 'text-warn')}>
                    {project.isGit
                      ? t('可用于隔离工作区', 'Isolation available')
                      : project.gitReason ?? t('非 Git 仓库', 'Not a Git repo')}
                  </dd>
                </div>
                <div className="flex items-start justify-between gap-3">
                  <dt className="text-muted">{t('创建时间', 'Created')}</dt>
                  <dd className="text-ink/90">{formatFullDateTime(project.createdAt)}</dd>
                </div>
                <div className="flex items-start justify-between gap-3">
                  <dt className="text-muted">{t('最近更新', 'Last updated')}</dt>
                  <dd className="text-ink/90">{fromNow(project.updatedAt)}</dd>
                </div>
                {project.description && (
                  <div>
                    <dt className="text-muted">{t('项目说明', 'Description')}</dt>
                    <dd className="mt-0.5 text-ink/80">{project.description}</dd>
                  </div>
                )}
              </dl>
            </Card>
          </div>

          <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
            <Card>
              <SectionTitle
                icon={Users}
                title={t('团队与席位', 'Teams & seats')}
                description={t(
                  `${teams.length} 个团队，${stats.assigned}/${stats.seats} 席位已指派`,
                  `${teams.length} teams, ${stats.assigned}/${stats.seats} seats assigned`,
                )}
                extra={
                  <Link to="/settings/teams" className="text-xs text-muted hover:text-primary">
                    {t('管理团队', 'Manage teams')}
                  </Link>
                }
              />
              {teams.length === 0 ? (
                <Empty
                  title={t('还没有团队', 'No teams yet')}
                  description={t(
                    '在「团队与席位」中基于模板创建，再为席位指派 Agent',
                    'Create one from a template in “Teams & seats”, then assign agents to seats',
                  )}
                  className="py-8"
                />
              ) : (
                <div className="divide-y divide-line/60 overflow-hidden rounded-xl border border-line/70">
                  {teams.map((team) => {
                    const pct =
                      team.seatsCount > 0
                        ? Math.round((team.assignedCount / team.seatsCount) * 100)
                        : 0;
                    return (
                      <div key={team.id} className="px-3.5 py-3">
                        <div className="flex items-center justify-between gap-2">
                          <div className="min-w-0">
                            <p className="truncate text-xs font-medium text-ink">{team.name}</p>
                            <p className="mt-0.5 text-xs text-muted">
                              {team.templateKey ?? t('自定义模板', 'Custom template')}
                            </p>
                          </div>
                          <span className="shrink-0 text-xs text-muted">
                            {t(
                              `${team.assignedCount}/${team.seatsCount} 席位`,
                              `${team.assignedCount}/${team.seatsCount} seats`,
                            )}
                          </span>
                        </div>
                        <ProgressBar
                          value={pct}
                          className="mt-2"
                          warn={pct > 0 && pct < 100}
                        />
                      </div>
                    );
                  })}
                </div>
              )}
            </Card>

            <Card>
              <SectionTitle
                icon={Activity}
                title={t('最近事件', 'Recent events')}
                description={t(
                  '项目相关的运行、审批与预算事件',
                  'Runs, approvals, and budget events for this project',
                )}
                extra={
                  <Link to="/history" className="text-xs text-muted hover:text-primary">
                    {t('全部事件', 'All events')}
                  </Link>
                }
              />
              {events.length === 0 ? (
                <Empty title={t('暂无执行事件', 'No events yet')} className="py-8" />
              ) : (
                <div className="divide-y divide-line/60 overflow-hidden rounded-xl border border-line/70">
                  {events.map((event) => (
                    <div key={event.id} className="flex items-start gap-2.5 px-3.5 py-2.5">
                      <span
                        className={cn(
                          'mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full',
                          eventColor(event.type),
                        )}
                      />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center justify-between gap-2">
                          <p className="truncate text-xs text-ink">{eventLabel(event.type)}</p>
                          <span className="shrink-0 text-xs text-muted">
                            {fromNow(event.createdAt)}
                          </span>
                        </div>
                        <p className="mt-0.5 truncate text-xs text-muted/80">
                          {event.actor ?? 'system'}
                          {event.taskId
                            ? t(`，任务 ${event.taskId.slice(-6)}`, `, task ${event.taskId.slice(-6)}`)
                            : ''}
                        </p>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </Card>
          </div>
        </div>
      )}
    </PageScroll>
  );
}
