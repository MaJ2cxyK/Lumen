import { useCallback, useEffect, useState } from 'react';
import {
  AlertCircle,
  Boxes,
  CircleDollarSign,
  Coins,
  FolderKanban,
  ListChecks,
  Zap,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import type { ScopedUsage, UsagePoint, UsageSummary } from '../lib/types';
import { formatNumber, formatTokens, microsToUsd } from '../lib/format';
import { useI18n } from '../lib/i18n';
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

function MetricCard({
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
    <Card>
      <div className="flex items-center gap-2 text-xs text-muted">
        <Icon size={13} className="shrink-0" />
        <p>{label}</p>
      </div>
      <p className="mt-2 text-xl font-semibold leading-6 tracking-tight tabular-nums text-ink">
        {value}
      </p>
      <p className="mt-0.5 text-xs leading-4 text-muted">{sub}</p>
    </Card>
  );
}

function UsageTable({
  title,
  icon: Icon,
  rows,
  emptyText,
}: {
  title: string;
  icon: typeof Coins;
  rows: ScopedUsage[];
  emptyText: string;
}) {
  const max = Math.max(1, ...rows.map((row) => row.tokens));
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(false);
  const visible = expanded ? rows : rows.slice(0, 12);
  return (
    <Card className="min-w-0">
      <SectionTitle icon={Icon} title={title} />
      {rows.length === 0 ? (
        <Empty title={emptyText} className="py-8" />
      ) : (
        <table className="w-full table-fixed border-collapse">
          <thead>
            <tr className="text-xs text-muted/70">
              <th className="w-[46%] pb-1.5 text-left font-normal">{t('名称', 'Name')}</th>
              <th className="pb-1.5 text-right font-normal">Tokens</th>
              <th className="pb-1.5 text-right font-normal">{t('成本', 'Cost')}</th>
              <th className="pb-1.5 text-right font-normal">{t('轮次', 'Rounds')}</th>
            </tr>
          </thead>
          <tbody>
            {visible.map((row) => (
              <tr key={row.key} className="border-t border-line/50">
                <td className="py-1.5 pr-2">
                  <p className="truncate text-xs text-ink" title={row.name}>
                    {row.name}
                  </p>
                  <div className="mt-1 h-0.5 w-full overflow-hidden rounded-full bg-soft">
                    <div
                      className="h-full rounded-full bg-primary/70"
                      style={{ width: `${Math.max(3, Math.round((row.tokens / max) * 100))}%` }}
                    />
                  </div>
                </td>
                <td className="py-1.5 text-right font-mono text-xs text-primary">
                  {formatTokens(row.tokens)}
                </td>
                <td className="py-1.5 text-right font-mono text-xs text-muted">
                  {microsToUsd(row.costMicros)}
                </td>
                <td className="py-1.5 text-right font-mono text-xs text-muted">
                  {formatNumber(row.rounds)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {rows.length > 12 && (
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          className="mt-2 text-xs text-primary hover:underline"
        >
          {expanded
            ? t('收起', 'Show less')
            : t(`查看全部 ${rows.length} 条`, `View all ${rows.length}`)}
        </button>
      )}
    </Card>
  );
}

export default function Usage({ embedded = false }: { embedded?: boolean }) {
  const [days, setDays] = useState(7);
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [series, setSeries] = useState<UsagePoint[]>([]);
  const [byAgent, setByAgent] = useState<ScopedUsage[]>([]);
  const [byProject, setByProject] = useState<ScopedUsage[]>([]);
  const [byTask, setByTask] = useState<ScopedUsage[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const { t } = useI18n();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [summaryData, seriesData, agentData, projectData, taskData] = await Promise.all([
        api.usageSummary(),
        api.usageSeries(days),
        api.usageByAgent(),
        api.usageByProject(),
        api.usageByTask(),
      ]);
      setSummary(summaryData);
      setSeries(seriesData);
      setByAgent(agentData);
      setByProject(projectData);
      setByTask(taskData);
      setError(null);
    } catch (err) {
      setError(errorMessage(err));
      toast.error(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [days]);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <PageScroll embedded={embedded}>
      <PageHeader
        back={embedded ? undefined : { to: '/settings', label: t('返回设置', 'Back to settings') }}
        title={t('用量', 'Usage')}
        actions={
          <div className="flex items-center gap-1 rounded-lg border border-line bg-card p-1">
            {[7, 30, 90].map((value) => (
              <button
                key={value}
                onClick={() => setDays(value)}
                className={cn(
                  'rounded-md px-3 py-1 text-xs transition-colors',
                  days === value
                    ? 'bg-primary/15 font-medium text-primary'
                    : 'text-muted hover:text-ink',
                )}
              >
                {t(`${value} 天`, `${value} days`)}
              </button>
            ))}
          </div>
        }
      />

      {loading ? (
        <Loading />
      ) : error && !summary ? (
        <Empty
          icon={AlertCircle}
          title={t('无法加载用量数据', 'Unable to load usage data')}
          description={error}
          action={
            <Button size="sm" variant="outline" onClick={() => load()}>
              {t('重试', 'Retry')}
            </Button>
          }
        />
      ) : !summary ? (
        <Empty
          icon={Coins}
          title={t('暂无用量数据', 'No usage data yet')}
          description={t(
            '派发任务并产生运行后，这里会显示成本统计',
            'Dispatch tasks and run them to see cost statistics here',
          )}
        />
      ) : (
        <div className="space-y-5">
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
            <MetricCard
              label={t('今日 Tokens', 'Tokens today')}
              value={formatTokens(summary.todayTokens)}
              sub={t(`累计 ${formatTokens(summary.totalTokens)}`, `Total ${formatTokens(summary.totalTokens)}`)}
              icon={Coins}
            />
            <MetricCard
              label={t('今日成本', 'Cost today')}
              value={microsToUsd(summary.todayCostMicros)}
              sub={t(`累计 ${microsToUsd(summary.totalCostMicros)}`, `Total ${microsToUsd(summary.totalCostMicros)}`)}
              icon={CircleDollarSign}
            />
            <MetricCard
              label={t('缓存命中率', 'Cache hit rate')}
              value={`${summary.cacheHitPct.toFixed(1)}%`}
              sub={t('缓存读取占比', 'Share of reads from cache')}
              icon={Zap}
            />
            <MetricCard
              label={t('调用轮次', 'Rounds')}
              value={formatNumber(summary.rounds)}
              sub={t(`今日 ${formatNumber(summary.todayRounds)} 轮`, `${formatNumber(summary.todayRounds)} today`)}
              icon={Boxes}
            />
          </div>

          <Card>
            <SectionTitle
              icon={Zap}
              title={t(`近 ${days} 日趋势`, `Last ${days} days`)}
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
            <TrendChart data={series} height={250} />
          </Card>

          <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
            <UsageTable
              title={t('按 Agent', 'By agent')}
              icon={Boxes}
              rows={byAgent}
              emptyText={t('暂无 Agent 用量', 'No agent usage yet')}
            />
            <UsageTable
              title={t('按项目', 'By project')}
              icon={FolderKanban}
              rows={byProject}
              emptyText={t('暂无项目用量', 'No project usage yet')}
            />
          </div>

          <UsageTable
            title={t('按任务', 'By task')}
            icon={ListChecks}
            rows={byTask}
            emptyText={t('暂无任务用量', 'No task usage yet')}
          />
        </div>
      )}
    </PageScroll>
  );
}
