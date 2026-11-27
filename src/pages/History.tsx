import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  Activity,
  Search,
  Table2,
  X,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import type { EventItem, Message, Run } from '../lib/types';
import { eventColor, eventLabel } from '../lib/labels';
import { tr, useI18n } from '../lib/i18n';
import {
  formatDateTime,
  formatDuration,
  formatFullDateTime,
  formatNumber,
  formatTokens,
  microsToUsd,
} from '../lib/format';
import {
  Button,
  Card,
  Empty,
  Input,
  IsolationBadge,
  Loading,
  PageHeader,
  PageScroll,
  RunStatusBadge,
  SectionTitle,
  cn,
} from '../components/ui';

function payloadText(payload: unknown): string {
  if (payload === null || payload === undefined) return '';
  if (typeof payload === 'string') return payload;
  try {
    return JSON.stringify(payload);
  } catch {
    return '';
  }
}

function prettyTranscriptLine(line: string): string {
  try {
    const parsed = JSON.parse(line) as Record<string, any>;
    if (typeof parsed.content === 'string') {
      const who = [parsed.seat, parsed.agent].filter(Boolean).join(tr('，', ', '));
      return `[${parsed.type ?? 'message'}]${who ? ` ${who}` : ''}\n${parsed.content}`;
    }
    return JSON.stringify(parsed, null, 2);
  } catch {
    return line;
  }
}

export default function History() {
  const navigate = useNavigate();
  const [events, setEvents] = useState<EventItem[]>([]);
  const [runs, setRuns] = useState<Run[]>([]);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState('');
  const [searchResults, setSearchResults] = useState<Message[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [selectedRun, setSelectedRun] = useState<Run | null>(null);
  const [transcript, setTranscript] = useState<string[]>([]);
  const [transcriptLoading, setTranscriptLoading] = useState(false);
  const { t } = useI18n();

  const load = useCallback(async () => {
    try {
      const [eventList, runList] = await Promise.all([
        api.eventsList({ limit: 120 }),
        api.runsList({ limit: 100 }),
      ]);
      setEvents(eventList);
      setRuns(runList);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  async function search() {
    const text = query.trim();
    if (!text) {
      setSearchResults(null);
      return;
    }
    setSearching(true);
    try {
      setSearchResults(await api.searchHistory(text, 60));
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSearching(false);
    }
  }

  async function openTranscript(run: Run) {
    setSelectedRun(run);
    setTranscriptLoading(true);
    setTranscript([]);
    try {
      setTranscript(await api.runTranscript(run.id, 300));
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setTranscriptLoading(false);
    }
  }

  return (
    <PageScroll>
      <PageHeader
        title={t('执行历史', 'Run history')}
        actions={
          <Button variant="outline" size="sm" onClick={load}>
            {t('刷新', 'Refresh')}
          </Button>
        }
      />

      {loading ? (
        <Loading />
      ) : (
        <div className="space-y-4">
          <Card>
            <SectionTitle
              icon={Search}
              title={t('消息搜索', 'Message search')}
              description={t(
                '跨任务检索席位产出、指令与历史消息',
                'Search seat output, instructions, and past messages across tasks',
              )}
            />
            <div className="flex gap-2">
              <Input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') search();
                }}
                placeholder={t(
                  '搜索席位产出、指令与历史消息内容',
                  'Search seat output, instructions, and past messages',
                )}
                className="h-8 py-1 text-xs"
              />
              <Button size="sm" loading={searching} onClick={search}>
                {t('搜索', 'Search')}
              </Button>
            </div>
            {searchResults !== null && (
              <div className="mt-3 divide-y divide-line/60 overflow-hidden rounded-xl border border-line/60">
                {searchResults.length === 0 ? (
                  <p className="py-4 text-center text-xs text-muted">
                    {t('没有匹配的消息', 'No matching messages')}
                  </p>
                ) : (
                  searchResults.map((message) => (
                    <button
                      key={message.id}
                      onClick={() => message.taskId && navigate(`/tasks/${message.taskId}`)}
                      className="block w-full px-3.5 py-2.5 text-left transition-colors hover:bg-hover"
                    >
                      <div className="flex items-center justify-between gap-2">
                        <span className="truncate text-xs font-medium text-ink">
                          {message.seatName ??
                            (message.role === 'user' ? t('你', 'You') : t('系统', 'System'))}
                          {message.agentName ? `${t('，', ', ')}${message.agentName}` : ''}
                        </span>
                        <span className="shrink-0 text-xs text-muted">
                          {formatDateTime(message.createdAt)}
                        </span>
                      </div>
                      <p className="mt-1 line-clamp-2 text-xs leading-5 text-muted">
                        {message.content}
                      </p>
                    </button>
                  ))
                )}
              </div>
            )}
          </Card>

          <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
            <Card>
              <SectionTitle
                icon={Activity}
                title={t('事件时间线', 'Event timeline')}
                description={t('系统、审批与运行事件', 'System, approval, and run events')}
                extra={
                  <span className="text-xs text-muted">
                    {t(`${events.length} 条`, `${events.length} events`)}
                  </span>
                }
              />
              {events.length === 0 ? (
                <Empty title={t('暂无事件', 'No events yet')} className="py-8" />
              ) : (
                <div className="max-h-[560px] space-y-3.5 overflow-y-auto pr-1">
                  {events.map((event) => (
                    <div key={event.id} className="timeline-item relative pl-6">
                      <span
                        className={cn(
                          'absolute left-0 top-1.5 h-3 w-3 rounded-full border-2 border-card',
                          eventColor(event.type),
                        )}
                      />
                      <div className="flex items-start justify-between gap-2">
                        <p className="text-xs font-medium text-ink">
                          {eventLabel(event.type)}
                        </p>
                        <span className="shrink-0 text-xs text-muted/70">
                          {formatDateTime(event.createdAt)}
                        </span>
                      </div>
                      <p className="mt-1 truncate text-xs text-muted">
                        {event.actor ?? 'system'}
                        {event.taskId ? `${t('，', ', ')}${event.taskId.slice(-6)}` : ''}
                        {payloadText(event.payload)
                          ? `${t('，', ', ')}${payloadText(event.payload)}`
                          : ''}
                      </p>
                    </div>
                  ))}
                </div>
              )}
            </Card>

            <Card>
              <SectionTitle
                icon={Table2}
                title={t('运行记录', 'Runs')}
                description={t('点击任意一行查看完整转录', 'Click any row to view the full transcript')}
                extra={
                  <span className="text-xs text-muted">
                    {t(`${runs.length} 次`, `${runs.length} runs`)}
                  </span>
                }
              />
              {runs.length === 0 ? (
                <Empty title={t('暂无运行记录', 'No runs yet')} className="py-8" />
              ) : (
                <div className="max-h-[560px] overflow-auto">
                  <table className="w-full text-left text-xs">
                    <thead className="sticky top-0 z-10 bg-card text-xs text-muted">
                      <tr>
                        <th className="pb-2 font-normal">{t('开始时间', 'Started')}</th>
                        <th className="pb-2 font-normal">{t('席位 / Agent', 'Seat / agent')}</th>
                        <th className="pb-2 font-normal">{t('状态', 'Status')}</th>
                        <th className="pb-2 text-right font-normal">Tokens</th>
                        <th className="pb-2 text-right font-normal">{t('成本', 'Cost')}</th>
                        <th className="pb-2 text-right font-normal">{t('耗时', 'Duration')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {runs.map((run) => (
                        <tr
                          key={run.id}
                          onClick={() => openTranscript(run)}
                          className="cursor-pointer border-t border-line/50 transition-colors hover:bg-hover"
                        >
                          <td className="whitespace-nowrap py-2 text-muted">
                            {formatDateTime(run.startedAt)}
                          </td>
                          <td className="max-w-[140px] truncate py-2 text-ink">
                            {run.seatName ?? '—'}
                            <span className="text-muted">
                              {t('，', ', ')}
                              {run.agentName ?? '—'}
                            </span>
                          </td>
                          <td className="py-2">
                            <span className="flex items-center gap-1">
                              <RunStatusBadge status={run.status} />
                              <IsolationBadge
                                isolation={run.isolation}
                                reason={run.isolationReason}
                              />
                            </span>
                          </td>
                          <td className="py-2 text-right font-mono text-primary">
                            {formatTokens(run.inputTokens + run.outputTokens)}
                          </td>
                          <td className="py-2 text-right font-mono text-muted">
                            {microsToUsd(run.costMicros)}
                          </td>
                          <td className="py-2 text-right font-mono text-muted">
                            {formatDuration(run.startedAt, run.endedAt)}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </Card>
          </div>
        </div>
      )}

      {selectedRun && (
        <div className="fixed inset-0 z-50 flex justify-end">
          <div
            className="absolute inset-0 bg-black/50"
            onClick={() => setSelectedRun(null)}
          />
          <div className="relative flex h-full w-[560px] max-w-full flex-col border-l border-line bg-card">
            <div className="flex items-start justify-between border-b border-line px-4 py-3">
              <div>
                <p className="text-sm font-semibold text-ink">{t('运行转录', 'Run transcript')}</p>
                <p className="mt-1 text-xs text-muted">
                  {selectedRun.seatName ?? t('席位', 'Seat')}
                  {t('，', ', ')}
                  {selectedRun.agentName ?? 'Agent'}
                  {t('，', ', ')}
                  {formatFullDateTime(selectedRun.startedAt)}
                </p>
              </div>
              <Button variant="ghost" size="xs" onClick={() => setSelectedRun(null)}>
                <X size={15} />
              </Button>
            </div>
            <div className="flex items-center gap-3 border-b border-line/60 px-4 py-2.5 text-xs text-muted">
              <span>
                {t('输入', 'Input')} {formatNumber(selectedRun.inputTokens)}
              </span>
              <span>
                {t('输出', 'Output')} {formatNumber(selectedRun.outputTokens)}
              </span>
              <span>{microsToUsd(selectedRun.costMicros)}</span>
              <span>{formatDuration(selectedRun.startedAt, selectedRun.endedAt)}</span>
              <IsolationBadge
                isolation={selectedRun.isolation}
                reason={selectedRun.isolationReason}
              />
            </div>
            <div className="flex-1 overflow-y-auto p-4">
              {transcriptLoading ? (
                <Loading label={t('加载转录', 'Loading transcript')} />
              ) : transcript.length === 0 ? (
                <Empty title={t('暂无转录内容', 'No transcript yet')} className="py-10" />
              ) : (
                <div className="space-y-2">
                  {transcript.map((line, index) => (
                    <pre
                      key={`${index}-${line.slice(0, 12)}`}
                      className="whitespace-pre-wrap break-all rounded-xl border border-line/60 bg-elevated p-3 font-mono text-xs leading-5 text-ink/75"
                    >
                      {prettyTranscriptLine(line)}
                    </pre>
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </PageScroll>
  );
}
