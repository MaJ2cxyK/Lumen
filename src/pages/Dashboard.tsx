import { useCallback, useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import {
  Activity,
  ArrowRight,
  CheckCircle2,
  CircleDollarSign,
  Coins,
  RefreshCw,
  ShieldAlert,
  Users,
  Wallet,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import type { DashboardSummary, EventItem, AgentStat, Task } from '../lib/types';
import { formatNumber, formatTokens, microsToUsd, fromNow } from '../lib/format';
import { eventColor, eventLabel } from '../lib/labels';
import { useI18n } from '../lib/i18n';
import { isReviewPending, statusDescriptor } from '../lib/taskLifecycle';
import { TrendChart } from '../components/TrendChart';
import {
  Button,
  Card,
  Empty,
  Loading,
  PageHeader,
  PageScroll,
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

function AgentRow({ agent, max }: { agent: AgentStat; max: number }) {
  const pct = max > 0 ? Math.max(4, Math.round((agent.tokens / max) * 100)) : 0;
  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between gap-2 text-xs">
        <div className="flex min-w-0 items-center gap-1.5">
          <span
            className={cn(
              'h-1.5 w-1.5 shrink-0 rounded-full',
              agent.status === 'running' ? 'bg-primary' : 'bg-muted/60',
            )}
          />
          <span className="truncate text-ink">{agent.name}</span>
          <span className="shrink-0 text-xs text-muted">{agent.adapter}</span>
        </div>
        <div className="flex shrink-0 items-center gap-3 font-mono text-xs">
          <span className="text-primary">{formatTokens(agent.tokens)}</span>
          <span className="w-14 text-right text-muted">{microsToUsd(agent.costMicros)}</span>
        </div>
      </div>
      <div className="h-1 w-full overflow-hidden rounded-full bg-soft">
        <div className="h-full rounded-full bg-primary/80" style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

export default function Dashboard() {
  const { t } = useI18n();
  const [summary, setSummary] = useState<DashboardSummary | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [attentionTotal, setAttentionTotal] = useState(0);
  const [events, setEvents] = useState<EventItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const load = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    try {
      const [data, eventList, attentionPage] = await Promise.all([
        api.dashboardSummary(),
        api.eventsList({ limit: 8 }),
        api.tasksPage({ attentionOnly: true, limit: 20 }),
      ]);
      setSummary(data);
      setEvents(eventList);
      setTasks(attentionPage.items);
      setAttentionTotal(attentionPage.total);
      setError(null);
    } catch (err) {
      setError(errorMessage(err));
      toast.error(errorMessage(err));
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const usage = summary?.usage;
  const attention = tasks;
  const maxTokens = Math.max(1, ...(summary?.agents.map((a) => a.tokens) ?? [1]));

  return (
    <PageScroll>
      <PageHeader
        title={t('总览', 'Overview')}
        actions={
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setRefreshing(true);
              load(true);
            }}
          >
            <RefreshCw size={13} className={refreshing ? 'animate-spin' : ''} />
            {t('刷新', 'Refresh')}
          </Button>
        }
      />

      {loading ? (
        <Loading />
      ) : error && (!summary || !summary.usage) ? (
        <Empty
          icon={ShieldAlert}
          title={t('无法加载总览', 'Unable to load overview')}
          description={error}
          action={
            <Button size="sm" variant="outline" onClick={() => load()}>
              {t('重试', 'Retry')}
            </Button>
          }
        />
      ) : !summary || !usage ? (
        <Empty
          icon={Activity}
          title={t('暂无总览数据', 'No overview data yet')}
          description={t(
            '创建项目并派发任务后，这里会显示实时概况',
            'Create a project and dispatch tasks to see live metrics here',
          )}
        />
      ) : (
        <div className="space-y-5">
          {attentionTotal > 0 ? (
            <Card className="border-warn/30">
              <SectionTitle
                icon={ShieldAlert}
                title={t('需要你', 'Needs you')}
                description={t('只显示等你介入的事项', 'Only items waiting on you')}
                extra={
                  <Link to="/tasks" className="text-xs text-muted hover:text-primary">
                    {t('全部任务', 'All tasks')}
                  </Link>
                }
              />
              <div className="divide-y divide-line/60">
                {attention.slice(0, 5).map((task) => {
                  const status = statusDescriptor(task.status);
                  const review = isReviewPending(task);
                  return (
                    <Link
                      key={task.id}
                      to={`/tasks/${task.id}`}
                      className="group flex items-center gap-3 py-2.5 transition-colors hover:text-ink"
                    >
                      <span
                        className={cn(
                          'h-1.5 w-1.5 shrink-0 rounded-full',
                          review ? 'bg-warn' : status.dot,
                        )}
                      />
                      <span className="min-w-0 flex-1 truncate text-sm text-ink">
                        {task.title}
                      </span>
                      <span className="shrink-0 text-xs text-muted">
                        {task.projectName ?? t('未关联项目', 'No project')}
                      </span>
                      <span
                        className={cn(
                          'shrink-0 text-xs font-medium',
                          review ? 'text-warn' : status.text,
                        )}
                      >
                        {review ? t('待审查', 'Review pending') : status.label}
                      </span>
                      <ArrowRight size={12} className="shrink-0 text-muted group-hover:text-primary" />
                    </Link>
                  );
                })}
              </div>
              {attentionTotal > 5 && (
                <p className="pt-2 text-xs text-muted">
                  {t(
                    `另有 ${attentionTotal - 5} 项待你处理`,
                    `${attentionTotal - 5} more items need you`,
                  )}
                </p>
              )}
            </Card>
          ) : (
            <div className="flex items-center gap-2.5 rounded-xl border border-line/60 bg-elevated/60 px-4 py-3 text-xs text-muted">
              <CheckCircle2 size={14} className="shrink-0 text-success" />
              {t(
                '全部任务都在自动推进，当前不需要你介入。',
                'All tasks are progressing automatically. Nothing needs you right now.',
              )}
            </div>
          )}

          <Card className="overflow-hidden p-0">
            <div className="grid grid-cols-1 gap-px bg-line/60 sm:grid-cols-2 xl:grid-cols-4">
              <Metric
                label={t('今日 Tokens', "Today's tokens")}
                value={formatTokens(usage.todayTokens)}
                sub={t(
                  `累计 ${formatTokens(usage.totalTokens)}，缓存命中 ${usage.cacheHitPct.toFixed(1)}%`,
                  `Total ${formatTokens(usage.totalTokens)}, cache hit ${usage.cacheHitPct.toFixed(1)}%`,
                )}
                icon={Coins}
              />
              <Metric
                label={t('今日成本', "Today's cost")}
                value={microsToUsd(usage.todayCostMicros)}
                sub={t(
                  `${formatNumber(usage.todayRounds)} 轮对话`,
                  `${formatNumber(usage.todayRounds)} rounds`,
                )}
                icon={CircleDollarSign}
              />
              <Metric
                label={t('累计成本', 'Total cost')}
                value={microsToUsd(usage.totalCostMicros)}
                sub={t(
                  `累计 ${formatNumber(usage.rounds)} 轮`,
                  `${formatNumber(usage.rounds)} rounds total`,
                )}
                icon={Wallet}
              />
              <Metric
                label={t('需要你', 'Needs you')}
                value={String(attentionTotal)}
                sub={
                  attentionTotal > 0
                    ? t(
                        `待审批 ${summary.pendingApprovals} · 待审查 ${summary.pendingReviews}`,
                        `To approve ${summary.pendingApprovals} · To review ${summary.pendingReviews}`,
                      )
                    : t('当前没有等你介入的任务', 'Nothing waiting on you')
                }
                icon={ShieldAlert}
              />
            </div>
          </Card>

          <div className="grid grid-cols-1 gap-4 xl:grid-cols-3">
            <Card className="xl:col-span-2">
              <SectionTitle
                icon={Activity}
                title={t('近 7 日用量趋势', '7-day usage trend')}
                description={t('Token 与成本按日汇总', 'Tokens and cost summarized daily')}
                extra={
                  <div className="flex items-center gap-3 text-xs text-muted">
                    <span className="flex items-center gap-1">
                      <span className="h-1.5 w-3 rounded-full bg-primary" />
                      Tokens
                    </span>
                    <span className="flex items-center gap-1">
                      <span className="h-1.5 w-3 rounded-full bg-success" />
                      {t('成本', 'Cost')}
                    </span>
                  </div>
                }
              />
              <TrendChart data={summary.series} height={260} />
            </Card>

            <Card>
              <SectionTitle
                icon={Users}
                title={t('Agent 用量排行', 'Agent usage ranking')}
                description={t('按累计 Token 排序', 'Ranked by total tokens')}
                extra={
                  <Link to="/settings/usage" className="text-xs text-muted hover:text-primary">
                    {t('用量明细', 'Usage details')}
                  </Link>
                }
              />
              {summary.agents.length === 0 ? (
                <Empty title={t('暂无 Agent 用量', 'No agent usage yet')} className="py-8" />
              ) : (
                <div className="space-y-3.5">
                  {[...summary.agents]
                    .sort((a, b) => b.tokens - a.tokens)
                    .slice(0, 5)
                    .map((agent) => (
                      <AgentRow key={agent.id} agent={agent} max={maxTokens} />
                    ))}
                </div>
              )}
            </Card>
          </div>

          <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
            <Card>
              <SectionTitle
                icon={Users}
                title={t('团队状态', 'Team status')}
                description={t('席位分配与项目关联', 'Seat allocation and project links')}
                extra={
                  <Link to="/settings/teams" className="inline-flex items-center gap-1 text-xs text-muted hover:text-primary">
                    {t('全部团队', 'All teams')} <ArrowRight size={11} />
                  </Link>
                }
              />
              {summary.teams.length === 0 ? (
                <Empty
                  title={t('还没有团队', 'No teams yet')}
                  description={t(
                    '在「团队与席位」中基于模板创建团队',
                    'Create a team from a template in "Teams & seats"',
                  )}
                  className="py-8"
                />
              ) : (
                <div className="divide-y divide-line/60 overflow-hidden rounded-xl border border-line/70">
                  {summary.teams.map((team) => {
                    const pct =
                      team.seatsCount > 0
                        ? Math.round((team.assignedCount / team.seatsCount) * 100)
                        : 0;
                    return (
                      <div
                        key={team.id}
                        className="px-3.5 py-3"
                      >
                        <div className="flex items-center justify-between gap-2">
                          <div className="min-w-0">
                            <p className="truncate text-xs font-medium text-ink">{team.name}</p>
                            <p className="mt-0.5 text-xs text-muted">
                              {team.projectName ?? t('未关联项目', 'No project')}
                              {t('，', ', ')}
                              {team.templateKey ?? t('自定义', 'Custom')}
                            </p>
                          </div>
                          <span className="shrink-0 text-xs text-muted">
                            {t(
                              `${team.assignedCount}/${team.seatsCount} 席位`,
                              `${team.assignedCount}/${team.seatsCount} seats`,
                            )}
                          </span>
                        </div>
                        <div className="mt-2 h-1 w-full overflow-hidden rounded-full bg-soft">
                          <div
                            className={cn(
                              'h-full rounded-full',
                              pct === 100 ? 'bg-success' : 'bg-primary',
                            )}
                            style={{ width: `${Math.max(3, pct)}%` }}
                          />
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </Card>

            <Card>
              <SectionTitle
                icon={Activity}
                title={t('最近执行事件', 'Recent run events')}
                description={t('最新 8 条运行与审批事件', 'Latest 8 run and approval events')}
                extra={
                  <Link to="/history" className="inline-flex items-center gap-1 text-xs text-muted hover:text-primary">
                    {t('全部事件', 'All events')} <ArrowRight size={11} />
                  </Link>
                }
              />
              {events.length === 0 ? (
                <Empty title={t('暂无执行事件', 'No run events yet')} className="py-8" />
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
                            ? t(`，${event.taskId.slice(-6)}`, `, ${event.taskId.slice(-6)}`)
                            : ''}
                          {event.type.startsWith('approval')
                            ? t('，审批流程', ', approval workflow')
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
