import { useCallback, useEffect, useRef, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {
  Check,
  CircleDot,
  Clock,
  Coins,
  FolderOpen,
  GitBranch,
  History,
  Link2,
  ListChecks,
  Paperclip,
  Pause,
  Play,
  Plus,
  RefreshCw,
  RotateCcw,
  Send,
  Square,
  Trash2,
  UserCog,
  Users,
  Wallet,
  Wrench,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage, pickFiles } from '../lib/api';
import { useAutoGrow, useComposer } from '../lib/composer';
import { clearTaskDraft, loadTaskDraft, saveTaskDraft } from '../lib/drafts';
import { onEvent } from '../lib/events';
import { tr, useI18n } from '../lib/i18n';
import type {
  Agent,
  Attachment,
  ChecklistItem,
  Message,
  PrOutcome,
  ReviewBranchInfo,
  ReviewDiff,
  ReviewSnapshot,
  Run,
  RunDeltaPayload,
  Seat,
  SnapshotItem,
  Task,
  TaskDependency,
  ToolCall,
} from '../lib/types';
import { priorityMeta, roleName } from '../lib/labels';
import { taskStage } from '../lib/taskStage';
import {
  canCancelStatus,
  canDispatchStatus,
  canPauseStatus,
  canReassignStatus,
  canResumeStatus,
  isActiveStatus,
  isReviewLockedStatus,
} from '../lib/taskStatus';
import { useAppStore } from '../store';
import {
  formatBytes,
  formatDateTime,
  formatDuration,
  formatNumber,
  formatTime,
  formatTokens,
  microsToUsd,
} from '../lib/format';
import {
  Badge,
  Button,
  Card,
  Empty,
  IsolationBadge,
  Input,
  Loading,
  Modal,
  PageHeader,
  RunStatusBadge,
  SectionTitle,
  Select,
  Textarea,
  cn,
} from '../components/ui';

const SEAT_COLORS = [
  'var(--seat-1)',
  'var(--seat-2)',
  'var(--seat-3)',
  'var(--seat-4)',
  'var(--seat-5)',
  'var(--seat-6)',
];

function seatColor(key?: string | null): string {
  if (!key) return SEAT_COLORS[0];
  let hash = 0;
  for (let i = 0; i < key.length; i += 1) hash = (hash * 31 + key.charCodeAt(i)) % 997;
  return SEAT_COLORS[hash % SEAT_COLORS.length];
}

function initial(name?: string | null): string {
  if (!name) return 'A';
  return name.slice(0, 1).toUpperCase();
}

function previewJson(value: unknown, max = 1200): string {
  if (value === null || value === undefined) return '—';
  try {
    const text = typeof value === 'string' ? value : JSON.stringify(value, null, 2);
    return text.length > max ? `${text.slice(0, max)}…` : text;
  } catch {
    return String(value);
  }
}

const TOOL_STATUS_LABEL: Record<string, string> = {
  get running() {
    return tr('运行中', 'Running');
  },
  get completed() {
    return tr('完成', 'Completed');
  },
  get failed() {
    return tr('失败', 'Failed');
  },
  get pending() {
    return tr('等待', 'Waiting');
  },
};

const REVIEW_STATUS_COLOR: Record<string, string> = {
  added: 'text-success',
  untracked: 'text-success',
  deleted: 'text-danger',
  conflicted: 'text-danger',
  renamed: 'text-info',
  modified: 'text-warn',
};

const REVIEW_CONFIRM = {
  apply: {
    get title() {
      return tr('应用到项目', 'Apply to project');
    },
    get description() {
      return tr(
        '将把隔离工作区中的提交 cherry-pick 到项目仓库；项目存在未提交改动时会中止。',
        'Commits from the isolated workspace will be cherry-picked into the project repo; it aborts if the project has uncommitted changes.',
      );
    },
    get action() {
      return tr('应用', 'Apply');
    },
  },
  revertAll: {
    get title() {
      return tr('还原全部改动', 'Revert all changes');
    },
    get description() {
      return tr(
        '将丢弃隔离工作区中所有未提交改动，包括新增的未跟踪文件。',
        'All uncommitted changes in the isolated workspace will be discarded, including new untracked files.',
      );
    },
    get action() {
      return tr('全部还原', 'Revert all');
    },
  },
  cleanup: {
    get title() {
      return tr('清理隔离工作区', 'Clean up isolated workspace');
    },
    get description() {
      return tr(
        '将删除隔离工作区目录，未提交的改动会一并丢失；该任务后续运行将直接写入项目目录。',
        'The isolated workspace directory will be deleted and any uncommitted changes lost; future runs of this task will write directly to the project directory.',
      );
    },
    get action() {
      return tr('清理', 'Clean up');
    },
  },
};

function ChatMessage({ message }: { message: Message }) {
  const { t } = useI18n();
  const color = seatColor(message.seatId ?? message.seatName ?? message.agentId);
  const duringRun =
    (message.metadata as { duringRun?: boolean } | null)?.duringRun === true;

  if (message.kind === 'system' || message.role === 'system') {
    return (
      <div className="flex justify-center">
        <span className="rounded-full border border-line/60 bg-soft px-3.5 py-1.5 text-xs text-muted">
          {message.content}
        </span>
      </div>
    );
  }

  if (message.role === 'user') {
    return (
      <div className="flex justify-end">
        <div className="max-w-[75%]">
          <p className="mb-1.5 flex items-center justify-end gap-1.5 text-right text-xs text-muted">
            {duringRun && (
              <span className="rounded border border-line/70 px-1.5 py-0.5 text-[10px] leading-none text-muted">
                {t('运行中追加', 'Added during run')}
              </span>
            )}
            {t(`你，${formatTime(message.createdAt)}`, `You, ${formatTime(message.createdAt)}`)}
          </p>
          <div className="whitespace-pre-wrap rounded-xl rounded-tr-sm border border-primary/25 bg-primary/15 px-4 py-2.5 text-sm leading-6 text-ink">
            {message.content}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex gap-3">
      <span
        className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-xs font-semibold text-white"
        style={{ backgroundColor: color, opacity: 0.9 }}
      >
        {initial(message.seatName ?? message.agentName)}
      </span>
      <div className="min-w-0 max-w-[80%]">
        <p className="mb-1.5 flex flex-wrap items-center gap-1.5 text-xs">
          <span className="font-medium" style={{ color }}>
            {message.seatName ?? t('协作席位', 'Seat')}
          </span>
          {message.agentName && (
            <span className="text-muted">
              {t(`（${message.agentName}）`, `(${message.agentName})`)}
            </span>
          )}
          <span className="text-muted/50">{formatTime(message.createdAt)}</span>
        </p>
        <div className="rounded-xl rounded-tl-sm border border-line bg-elevated px-4 py-3">
          <div className="md">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.content}</ReactMarkdown>
          </div>
        </div>
      </div>
    </div>
  );
}

export default function TaskDetail() {
  const { id = '' } = useParams<{ id: string }>();
  const { t } = useI18n();
  const refreshNeedsYou = useAppStore((s) => s.refreshNeedsYou);
  const projects = useAppStore((s) => s.projects);
  const [task, setTask] = useState<Task | null>(null);
  const project = task ? projects.find((item) => item.id === task.projectId) ?? null : null;
  const taskRunning = Boolean(task && isActiveStatus(task.status));
  const taskPaused = task?.status === 'paused';
  const [seats, setSeats] = useState<Seat[]>([]);
  const [messages, setMessages] = useState<Message[]>([]);
  const [runs, setRuns] = useState<Run[]>([]);
  const [loading, setLoading] = useState(true);
  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const [busy, setBusy] = useState('');
  const [hasOlder, setHasOlder] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [promptOpen, setPromptOpen] = useState(false);
  const [promptLoading, setPromptLoading] = useState(false);
  const [promptText, setPromptText] = useState('');
  const [promptLabel, setPromptLabel] = useState('');
  const [toolCalls, setToolCalls] = useState<ToolCall[]>([]);
  const [atBottom, setAtBottom] = useState(true);
  const [stoppingTask, setStoppingTask] = useState(false);
  const [stoppingRun, setStoppingRun] = useState<string | null>(null);
  const [review, setReview] = useState<ReviewSnapshot | null>(null);
  const [reviewBusy, setReviewBusy] = useState('');
  const [diffFile, setDiffFile] = useState<ReviewDiff | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [commitOpen, setCommitOpen] = useState(false);
  const [commitMessage, setCommitMessage] = useState('');
  const [confirmKind, setConfirmKind] = useState<keyof typeof REVIEW_CONFIRM | null>(null);
  const [runConfirm, setRunConfirm] = useState<'dispatch' | 'retry' | 'resume' | null>(null);
  const [reassignOpen, setReassignOpen] = useState(false);
  const [reassignAgentId, setReassignAgentId] = useState('');
  const [agents, setAgents] = useState<Agent[]>([]);
  const [children, setChildren] = useState<Task[]>([]);
  const [deps, setDeps] = useState<TaskDependency[]>([]);
  const [checklist, setChecklist] = useState<ChecklistItem[]>([]);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [projectTasks, setProjectTasks] = useState<Task[]>([]);
  const [depPick, setDepPick] = useState('');
  const [subtaskOpen, setSubtaskOpen] = useState(false);
  const [subtaskTitle, setSubtaskTitle] = useState('');
  const [checklistBusy, setChecklistBusy] = useState(false);
  const [attachmentBusy, setAttachmentBusy] = useState(false);
  const [branchInfo, setBranchInfo] = useState<ReviewBranchInfo | null>(null);
  const [branchOpen, setBranchOpen] = useState(false);
  const [branchMode, setBranchMode] = useState<'create' | 'switch'>('create');
  const [branchName, setBranchName] = useState('');
  const [branchBusy, setBranchBusy] = useState(false);
  const [snapshots, setSnapshots] = useState<SnapshotItem[]>([]);
  const [rollbackTarget, setRollbackTarget] = useState<SnapshotItem | null>(null);
  const [rollbackBusy, setRollbackBusy] = useState(false);
  const [prOpen, setPrOpen] = useState(false);
  const [prTitle, setPrTitle] = useState('');
  const [prBody, setPrBody] = useState('');
  const [prBusy, setPrBusy] = useState(false);
  const [prResult, setPrResult] = useState<PrOutcome | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const atBottomRef = useRef(true);
  const runsRef = useRef<Run[]>([]);
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refreshRuns = useCallback(async () => {
    try {
      setRuns(await api.runsList({ taskId: id, limit: 50 }));
    } catch {
      /* ignore */
    }
  }, [id]);

  const refreshMessages = useCallback(async () => {
    try {
      const list = await api.taskMessages(id);
      setMessages(list);
      setHasOlder(list.length >= 200);
    } catch {
      /* ignore */
    }
  }, [id]);

  const refreshReview = useCallback(async () => {
    try {
      setReview(await api.taskReview(id));
    } catch {
      setReview(null);
    }
  }, [id]);

  const load = useCallback(async () => {
    try {
      const found = await api.taskGet(id);
      setTask(found);
      const [
        messageList,
        runList,
        seatList,
        callList,
        childList,
        depList,
        checklistItems,
        attachmentList,
        projectTaskList,
        branch,
        snapshotList,
      ] = await Promise.all([
        api.taskMessages(id),
        api.runsList({ taskId: id, limit: 50 }),
        found.teamId ? api.seatsList(found.teamId) : Promise.resolve<Seat[]>([]),
        api.toolCallsList(id, 200),
        api.taskChildren(id),
        api.taskDependencies(id),
        api.taskChecklistGet(id),
        api.taskAttachments(id),
        api.tasksList({ projectId: found.projectId }),
        api.taskReviewBranchInfo(id),
        api.taskSnapshots(id),
      ]);
      setMessages(messageList);
      setHasOlder(messageList.length >= 200);
      setRuns(runList);
      setToolCalls(callList);
      setSeats([...seatList].sort((a, b) => a.orderIndex - b.orderIndex));
      setChildren(childList);
      setDeps(depList);
      setChecklist(checklistItems);
      setAttachments(attachmentList);
      setProjectTasks(projectTaskList.filter((item) => item.id !== found.id));
      setBranchInfo(branch);
      setSnapshots(snapshotList);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [id]);

  // 事件可能在短时间内连续触发，合并成一次刷新，避免事件风暴造成重复请求。
  const scheduleRefresh = useCallback(() => {
    if (refreshTimer.current) clearTimeout(refreshTimer.current);
    refreshTimer.current = setTimeout(() => {
      load();
      refreshRuns();
    }, 400);
  }, [load, refreshRuns]);

  useEffect(() => {
    return () => {
      if (refreshTimer.current) clearTimeout(refreshTimer.current);
    };
  }, []);

  useEffect(() => {
    setLoading(true);
    load();
  }, [load]);

  useEffect(() => {
    runsRef.current = runs;
  }, [runs]);

  // 任务与运行进入终态后，才解除“正在停止”的等待状态。
  useEffect(() => {
    if (!stoppingTask) return;
    if (!task || !isReviewLockedStatus(task.status)) {
      setStoppingTask(false);
    }
  }, [task, stoppingTask]);

  useEffect(() => {
    if (!stoppingRun) return;
    const run = runs.find((item) => item.id === stoppingRun);
    if (!run || !['starting', 'running'].includes(run.status)) setStoppingRun(null);
  }, [runs, stoppingRun]);

  useEffect(() => {
    setInput(loadTaskDraft(id));
  }, [id]);

  useEffect(() => {
    saveTaskDraft(id, input);
  }, [id, input]);

  useEffect(() => {
    if (task) void refreshReview();
  }, [id, task?.status, task?.workDir, refreshReview]);

  useAutoGrow(inputRef, input, 180);
  const composer = useComposer(sendNote);

  const scrollToBottom = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    atBottomRef.current = true;
    setAtBottom(true);
  }, []);

  // 只有用户本来就贴在底部时才跟随新消息，避免打断向上翻阅。
  useEffect(() => {
    if (atBottomRef.current) scrollToBottom();
  }, [messages, toolCalls, scrollToBottom]);

  function handleScroll() {
    const el = scrollRef.current;
    if (!el) return;
    const bottom = el.scrollHeight - el.scrollTop - el.clientHeight <= 48;
    atBottomRef.current = bottom;
    setAtBottom(bottom);
  }

  useEffect(() => {
    let disposed = false;
    const unsubs: Array<() => void> = [];
    const registrations = [
      onEvent<RunDeltaPayload>('run.delta', (payload) => {
        const taskId = payload?.taskId ?? payload?.message?.taskId;
        // 工具调用事件只带 runId，用它反查是否属于当前任务。
        const knownRun = Boolean(payload?.runId) && runsRef.current.some((r) => r.id === payload.runId);
        if (taskId !== id && !knownRun) return;
        if (payload.kind === 'message' && payload.message) {
          const message = payload.message;
          setMessages((prev) =>
            prev.some((item) => item.id === message.id) ? prev : [...prev, message],
          );
        }
        if (payload.kind === 'tool_call' && payload.toolCall) {
          const call = payload.toolCall;
          setToolCalls((prev) =>
            prev.some((item) => item.id === call.id)
              ? prev
              : [
                  ...prev,
                  {
                    id: call.id,
                    runId: payload.runId,
                    toolName: call.toolName,
                    input: call.input,
                    output: null,
                    status: call.status ?? 'running',
                    riskLevel: null,
                    createdAt: Date.now(),
                    finishedAt: null,
                  },
                ],
          );
          scheduleRefresh();
        }
        if (payload.kind === 'tool_result' && payload.toolCallId) {
          setToolCalls((prev) =>
            prev.map((item) =>
              item.id === payload.toolCallId
                ? { ...item, status: 'completed', finishedAt: Date.now() }
                : item,
            ),
          );
          scheduleRefresh();
        }
        if (payload.kind === 'usage') scheduleRefresh();
      }),
      onEvent<{ taskId: string; status?: string }>('run.status', (payload) => {
        if (payload?.taskId === id) {
          scheduleRefresh();
          if (payload.status && !['starting', 'running'].includes(payload.status)) {
            void refreshReview();
          }
        }
      }),
      onEvent<{ taskId: string }>('task.updated', (payload) => {
        if (payload?.taskId === id) {
          scheduleRefresh();
          void refreshReview();
        }
      }),
    ];
    Promise.all(registrations).then((fns) => {
      if (disposed) fns.forEach((fn) => fn());
      else unsubs.push(...fns);
    });
    return () => {
      disposed = true;
      unsubs.forEach((fn) => fn());
    };
  }, [id, scheduleRefresh, refreshReview]);

  async function sendNote() {
    const content = input.trim();
    if (!content || sending) return;
    setSending(true);
    try {
      await api.taskAddNote(id, content);
      clearTaskDraft(id);
      setInput('');
      atBottomRef.current = true;
      await refreshMessages();
      toast.success(
        t(
          taskRunning ? '已记录，将在下一阶段送达' : '备注已记录，下次派发时生效',
          taskRunning
            ? 'Recorded; will be delivered in the next stage'
            : 'Note saved; it takes effect on the next dispatch',
        ),
      );
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSending(false);
    }
  }

  async function loadOlder() {
    const oldest = messages[0];
    if (!oldest || loadingOlder) return;
    setLoadingOlder(true);
    try {
      const older = await api.taskMessages(id, oldest.createdAt - 1);
      setMessages((prev) => [...older, ...prev]);
      setHasOlder(older.length >= 200);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setLoadingOlder(false);
    }
  }

  async function performRun(kind: 'dispatch' | 'retry' | 'resume') {
    if (!task) return;
    setBusy(kind);
    try {
      if (kind === 'dispatch') {
        const snapshot = await api.taskDispatch(task.id);
        toast.success(
          snapshot
            ? t('任务已派发，已生成运行前快照', 'Task dispatched; pre-run snapshot created')
            : t('任务已派发', 'Task dispatched'),
        );
      } else if (kind === 'resume') {
        const snapshot = await api.taskResume(task.id);
        toast.success(
          snapshot
            ? t(
                `已从阶段 ${task.stageIndex + 1} 继续，已生成运行前快照`,
                `Resumed from stage ${task.stageIndex + 1}; pre-run snapshot created`,
              )
            : t(
                `已从阶段 ${task.stageIndex + 1} 继续`,
                `Resumed from stage ${task.stageIndex + 1}`,
              ),
        );
      } else {
        const snapshot = await api.taskRetryStage(task.id);
        toast.success(
          snapshot
            ? t(
                `已从阶段 ${task.stageIndex + 1} 重试，已生成运行前快照`,
                `Retried from stage ${task.stageIndex + 1}; pre-run snapshot created`,
              )
            : t(
                `已从阶段 ${task.stageIndex + 1} 重试`,
                `Retried from stage ${task.stageIndex + 1}`,
              ),
        );
      }
      setRunConfirm(null);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy('');
    }
  }

  async function openReassign() {
    if (!task) return;
    setReassignAgentId(seats[task.stageIndex]?.agentId ?? '');
    setReassignOpen(true);
    try {
      setAgents(await api.agentsList());
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function performReassign() {
    if (!task || !reassignAgentId) return;
    setBusy('reassign');
    try {
      const snapshot = await api.taskReassign(task.id, reassignAgentId);
      toast.success(
        snapshot
          ? t(
              '已改派并从当前阶段重跑，已生成运行前快照',
              'Reassigned and rerunning from the current stage; pre-run snapshot created',
            )
          : t('已改派并从当前阶段重跑', 'Reassigned and rerunning from the current stage'),
      );
      setReassignOpen(false);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy('');
    }
  }

  async function toggleChecklist(item: ChecklistItem) {
    if (!task || checklistBusy) return;
    const next = checklist.map((entry) =>
      entry.id === item.id ? { ...entry, done: !entry.done } : entry,
    );
    setChecklist(next);
    setChecklistBusy(true);
    try {
      setChecklist(await api.taskChecklistSave(task.id, next));
    } catch (err) {
      toast.error(errorMessage(err));
      void load();
    } finally {
      setChecklistBusy(false);
    }
  }

  async function generateChecklist() {
    if (!task) return;
    const text = task.acceptance?.trim();
    if (!text) {
      toast.info(t('先在任务里填写验收标准', 'Add acceptance criteria to the task first'));
      return;
    }
    const items: ChecklistItem[] = text
      .split(/\r?\n/)
      .map((line) => line.replace(/^\s*(?:[-*•]|\d+[.、)）])\s*/, '').trim())
      .filter(Boolean)
      .map((line) => ({ id: '', text: line, done: false }));
    if (!items.length) return;
    setChecklistBusy(true);
    try {
      setChecklist(await api.taskChecklistSave(task.id, items));
      toast.success(t('已按验收标准生成清单', 'Checklist generated from acceptance criteria'));
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setChecklistBusy(false);
    }
  }

  async function addDependency() {
    if (!task || !depPick) return;
    try {
      await api.taskDependencyAdd(task.id, depPick);
      setDepPick('');
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function removeDependency(dependsOnId: string) {
    if (!task) return;
    try {
      await api.taskDependencyRemove(task.id, dependsOnId);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function addAttachments() {
    if (!task) return;
    const paths = await pickFiles();
    if (!paths.length) return;
    setAttachmentBusy(true);
    try {
      for (const path of paths) {
        await api.taskAttachmentAdd(task.id, path);
      }
      toast.success(
        paths.length > 1
          ? t(`已添加 ${paths.length} 个附件`, `${paths.length} attachments added`)
          : t('附件已添加', 'Attachment added'),
      );
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setAttachmentBusy(false);
    }
  }

  async function removeAttachment(attachment: Attachment) {
    try {
      await api.taskAttachmentDelete(attachment.id);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  function openBranch(mode: 'create' | 'switch') {
    setBranchMode(mode);
    setBranchName(mode === 'switch' ? (branchInfo?.current ?? '') : '');
    setBranchOpen(true);
  }

  async function performBranch() {
    if (!task || !branchName.trim()) return;
    setBranchBusy(true);
    try {
      if (branchMode === 'create') {
        await api.taskReviewBranchCreate(task.id, branchName.trim());
        toast.success(
          t(
            `已创建并切换到分支 ${branchName.trim()}`,
            `Created and switched to branch ${branchName.trim()}`,
          ),
        );
      } else {
        await api.taskReviewBranchSwitch(task.id, branchName.trim());
        toast.success(
          t(`已切换到分支 ${branchName.trim()}`, `Switched to branch ${branchName.trim()}`),
        );
      }
      setBranchOpen(false);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBranchBusy(false);
    }
  }

  async function performRollback() {
    if (!task || !rollbackTarget) return;
    setRollbackBusy(true);
    try {
      const message = await api.taskSnapshotRollback(task.id, rollbackTarget.path);
      toast.success(message);
      setRollbackTarget(null);
      await Promise.all([load(), refreshReview()]);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setRollbackBusy(false);
    }
  }

  async function performPr() {
    if (!task) return;
    setPrBusy(true);
    try {
      setPrResult(await api.taskReviewPr(task.id, prTitle, prBody));
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setPrBusy(false);
    }
  }

  async function copyText(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(t('已复制到剪贴板', 'Copied to clipboard'));
    } catch {
      toast.error(t('复制失败，请手动选择文本', 'Copy failed; please select the text manually'));
    }
  }

  async function createSubtask() {
    if (!task || !subtaskTitle.trim()) return;
    setBusy('subtask');
    try {
      await api.taskCreate({
        projectId: task.projectId,
        teamId: task.teamId,
        parentId: task.id,
        title: subtaskTitle.trim(),
      });
      toast.success(t('子任务已创建', 'Subtask created'));
      setSubtaskOpen(false);
      setSubtaskTitle('');
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy('');
    }
  }

  async function action(
    kind: 'dispatch' | 'retry' | 'resume' | 'pause' | 'cancel' | 'reset',
  ) {
    if (!task) return;
    // 非隔离运行会直接改项目目录，先确认并说明快照/回滚边界。
    if (
      (kind === 'dispatch' || kind === 'retry' || kind === 'resume') &&
      !task.workDir
    ) {
      setRunConfirm(kind);
      return;
    }
    if (kind === 'dispatch' || kind === 'retry' || kind === 'resume') {
      await performRun(kind);
      return;
    }
    setBusy(kind);
    try {
      if (kind === 'pause') {
        await api.taskPause(task.id);
        toast.success(
          t('任务已暂停，继续时从当前阶段重跑', 'Task paused; resuming reruns from the current stage'),
        );
      } else if (kind === 'cancel') {
        setStoppingTask(true);
        await api.taskCancel(task.id);
        // 后端已发出取消信号，但运行退出是异步的，这里不提前宣称“已停止”。
        toast.info(t('停止信号已发出，等待运行退出', 'Stop signal sent; waiting for the run to exit'));
      } else {
        await api.taskReset(task.id);
        toast.success(t('任务已重置', 'Task reset'));
      }
      await load();
    } catch (err) {
      if (kind === 'cancel') setStoppingTask(false);
      toast.error(errorMessage(err));
    } finally {
      setBusy('');
    }
  }

  async function showPrompt(run: Run) {
    setPromptLabel(
      t(
        `${run.seatName ?? '席位'}，${run.agentName ?? 'Agent'}`,
        `${run.seatName ?? 'Seat'}, ${run.agentName ?? 'Agent'}`,
      ),
    );
    setPromptText('');
    setPromptOpen(true);
    setPromptLoading(true);
    try {
      const text = await api.runPrompt(run.id);
      setPromptText(
        text ?? t('该运行没有保存提示词快照。', 'No prompt snapshot was saved for this run.'),
      );
    } catch (err) {
      toast.error(errorMessage(err));
      setPromptText(t('读取提示词失败。', 'Failed to read the prompt.'));
    } finally {
      setPromptLoading(false);
    }
  }

  async function cancelRun(run: Run) {
    setStoppingRun(run.id);
    try {
      await api.runCancel(run.id);
      toast.info(t('取消信号已发出，等待运行退出', 'Cancel signal sent; waiting for the run to exit'));
      await Promise.all([load(), refreshRuns()]);
    } catch (err) {
      setStoppingRun(null);
      toast.error(errorMessage(err));
    }
  }

  async function openDiff(path: string) {
    setDiffLoading(true);
    setDiffFile({ path, patch: '', truncated: false, binary: false });
    try {
      setDiffFile(await api.taskReviewDiff(id, path));
    } catch (err) {
      toast.error(errorMessage(err));
      setDiffFile(null);
    } finally {
      setDiffLoading(false);
    }
  }

  async function revertFile(path: string) {
    setReviewBusy('revert');
    try {
      await api.taskReviewRevert(id, path);
      toast.success(t('已还原该文件', 'File reverted'));
      setDiffFile(null);
      await refreshReview();
      await refreshNeedsYou();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setReviewBusy('');
    }
  }

  async function commitReview() {
    setReviewBusy('commit');
    try {
      const sha = await api.taskReviewCommit(id, commitMessage);
      toast.success(t(`已提交 ${sha}`, `Committed ${sha}`));
      setCommitOpen(false);
      setCommitMessage('');
      await refreshReview();
      await refreshNeedsYou();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setReviewBusy('');
    }
  }

  async function runReviewAction(kind: keyof typeof REVIEW_CONFIRM) {
    setReviewBusy(kind);
    try {
      if (kind === 'apply') {
        const sha = await api.taskReviewApply(id);
        toast.success(t(`已应用到项目 ${sha}`, `Applied to project ${sha}`));
      } else if (kind === 'revertAll') {
        await api.taskReviewRevertAll(id);
        toast.success(t('已还原全部改动', 'All changes reverted'));
      } else {
        await api.taskReviewCleanup(id);
        toast.success(t('隔离工作区已清理', 'Isolated workspace cleaned up'));
      }
      setConfirmKind(null);
      await refreshReview();
      await refreshNeedsYou();
      if (kind === 'cleanup') await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setReviewBusy('');
    }
  }

  if (loading) {
    return (
      <div className="p-6">
        <Loading />
      </div>
    );
  }

  if (!task) {
    return (
      <div className="p-6">
        <PageHeader
          title={t('任务不存在', 'Task not found')}
          back={{ to: '/tasks', label: t('返回任务看板', 'Back to task board') }}
        />
        <Empty
          icon={ListChecks}
          title={t('未找到该任务', 'Task not found')}
          description={t('任务可能已被删除', 'The task may have been deleted')}
          className="py-16"
        />
      </div>
    );
  }

  const stage = taskStage(task.status);
  const currentIndex = task.stageIndex;
  const canResume = canResumeStatus(task.status);
  const canDispatch = Boolean(task.teamId) && canDispatchStatus(task.status);
  const canPause = canPauseStatus(task.status);
  const canCancel = canCancelStatus(task.status);
  const canReassign = canReassignStatus(task.status) && seats.length > 0;
  const reviewLocked = isReviewLockedStatus(task.status);

  return (
    <div className="h-full overflow-hidden">
      <div className="mx-auto flex h-full max-w-[1280px] flex-col px-8 py-7">
        <PageHeader
          back={{ to: '/tasks', label: t('返回任务看板', 'Back to task board') }}
          title={
            <span className="flex items-center gap-2">
              {task.title}
              <Badge>
                <span className={cn('h-1.5 w-1.5 rounded-full', stage.dot)} />
                {stage.label}
              </Badge>
            </span>
          }
          subtitle={
            <span className="flex flex-wrap items-center gap-2">
              {task.projectId ? (
                <Link
                  to={`/projects/${task.projectId}`}
                  className="transition-colors hover:text-primary"
                >
                  {task.projectName ?? t('未关联项目', 'No project linked')}
                </Link>
              ) : (
                <span>{t('未关联项目', 'No project linked')}</span>
              )}
              <span className="text-muted/40">/</span>
              <span>{task.teamName ?? t('未选择团队', 'No team selected')}</span>
              <Badge>
                <span className={cn('h-1.5 w-1.5 rounded-full', priorityMeta(task.priority).dot)} />
                {priorityMeta(task.priority).label}
              </Badge>
              <span className={cn('text-xs', stage.text)}>{stage.hint}</span>
              <span className="font-mono text-xs text-muted/60">{task.id.slice(-6)}</span>
            </span>
          }
          actions={
            <>
              {canResume ? (
                <Button
                  size="sm"
                  disabled={!task.teamId}
                  loading={busy === 'resume'}
                  onClick={() => action('resume')}
                >
                  <Play size={13} />
                  {t('继续', 'Resume')}
                </Button>
              ) : (
                <Button
                  size="sm"
                  disabled={!canDispatch}
                  loading={busy === 'dispatch' || busy === 'retry'}
                  onClick={() => action(task.status === 'blocked' ? 'retry' : 'dispatch')}
                >
                  <Play size={13} />
                  {task.status === 'blocked'
                    ? t(`重试阶段 ${currentIndex + 1}`, `Retry stage ${currentIndex + 1}`)
                    : t('派发', 'Dispatch')}
                </Button>
              )}
              <Button
                variant="outline"
                size="sm"
                disabled={!canPause}
                loading={busy === 'pause'}
                onClick={() => action('pause')}
              >
                <Pause size={12} />
                {t('暂停', 'Pause')}
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={!canReassign}
                loading={busy === 'reassign'}
                onClick={() => void openReassign()}
              >
                <UserCog size={13} />
                {t('改派', 'Reassign')}
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={!canCancel || stoppingTask}
                loading={busy === 'cancel' || stoppingTask}
                onClick={() => action('cancel')}
              >
                <Square size={12} />
                {stoppingTask ? t('停止中', 'Stopping…') : t('取消', 'Cancel')}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                disabled={task.status === 'inbox'}
                loading={busy === 'reset'}
                onClick={() => action('reset')}
              >
                <RotateCcw size={13} />
                {t('重置', 'Reset')}
              </Button>
            </>
          }
        />

        <div className="mb-4 overflow-x-auto rounded-xl border border-line bg-card px-5 py-4">
          {seats.length === 0 ? (
            <p className="text-xs text-muted">
              {t(
                '该任务未关联团队，无法展示阶段进度',
                'This task has no team, so stage progress is unavailable',
              )}
            </p>
          ) : (
            <div className="flex min-w-max items-start">
              {seats.map((seat, index) => {
                const done = task.status === 'completed' || index < currentIndex;
                const current = index === currentIndex && task.status !== 'completed';
                const blocked = current && task.status === 'blocked';
                return (
                  <div key={seat.id} className="flex items-center">
                    <div className="flex items-center gap-2.5">
                      <span
                        className={cn(
                          'flex h-7 w-7 items-center justify-center rounded-full border text-xs font-mono',
                          done && 'border-primary/50 bg-primary/15 text-primary',
                          current && !blocked && 'border-primary bg-primary text-white',
                          blocked && 'border-danger bg-danger/15 text-danger',
                          !done && !current && 'border-line bg-base/60 text-muted',
                        )}
                      >
                        {done ? <Check size={13} /> : index + 1}
                      </span>
                      <div className="min-w-0">
                        <p
                          className={cn(
                            'text-sm leading-5',
                            current || done ? 'font-medium text-ink' : 'text-muted',
                          )}
                        >
                          {roleName(seat.roleKey, seat.roleName)}
                        </p>
                        <p className="text-xs leading-4 text-muted/70">
                          {seat.agentName ?? t('未指派', 'Unassigned')}
                        </p>
                      </div>
                    </div>
                    {index < seats.length - 1 && (
                      <div
                        className={cn(
                          'mx-4 h-px w-10 shrink-0',
                          index < currentIndex ? 'bg-primary/50' : 'bg-line',
                        )}
                      />
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>

        <div className="flex min-h-0 flex-1 gap-5">
          <Card className="flex min-h-0 flex-1 flex-col p-0">
            <div className="flex items-center justify-between border-b border-line/60 px-5 py-3">
              <div className="flex items-center gap-2.5">
                <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-soft text-muted">
                  <Users size={14} />
                </span>
                <div>
                  <p className="text-sm font-semibold leading-5 text-ink">
                    {t('协作消息', 'Collaboration messages')}
                  </p>
                  <p className="text-xs leading-4 text-muted">
                    {t(
                      `${messages.length} 条消息，席位产出实时汇总`,
                      `${messages.length} messages, live seat output`,
                    )}
                  </p>
                </div>
              </div>
              {task.status === 'running' && (
                <span className="flex items-center gap-1.5 text-xs text-primary">
                  <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-primary" />
                  {t('实时接收中', 'Receiving live')}
                </span>
              )}
            </div>

            <div className="relative min-h-0 flex-1">
              <div
                ref={scrollRef}
                onScroll={handleScroll}
                className="h-full space-y-5 overflow-y-auto px-5 py-5"
              >
                {hasOlder && (
                  <div className="flex justify-center">
                    <Button
                      variant="ghost"
                      size="xs"
                      loading={loadingOlder}
                      onClick={loadOlder}
                    >
                      {t('加载更早的消息', 'Load earlier messages')}
                    </Button>
                  </div>
                )}
                {messages.length === 0 ? (
                  <Empty
                    title={t('还没有协作消息', 'No messages yet')}
                    description={t(
                      '任务派发后，席位产出会实时显示在这里',
                      'Once the task is dispatched, seat output appears here in real time',
                    )}
                    className="mt-10 border-none"
                  />
                ) : (
                  messages.map((message) => <ChatMessage key={message.id} message={message} />)
                )}
              </div>
              {!atBottom && (
                <button
                  type="button"
                  onClick={scrollToBottom}
                  className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full border border-line bg-card/95 px-3 py-1.5 text-xs text-ink shadow-sm backdrop-blur transition-colors hover:bg-hover"
                >
                  {t('回到底部', 'Back to bottom')}
                </button>
              )}
            </div>

            <div className="border-t border-line/60 p-4">
              <div className="flex items-end gap-2.5">
                <textarea
                  ref={inputRef}
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                  {...composer}
                  rows={1}
                  placeholder={
                    taskRunning
                      ? t(
                          '运行中追加备注，不会打断当前执行，下一阶段生效',
                          'Add a note while running; it won’t interrupt execution and takes effect next stage',
                        )
                      : taskPaused
                        ? t(
                            '补充指令，继续时从当前阶段生效',
                            'Add instructions; they take effect from the current stage on resume',
                          )
                        : t(
                            '补充说明或指令，Enter 发送，Shift + Enter 换行',
                            'Add notes or instructions; Enter to send, Shift + Enter for a new line',
                          )
                  }
                  className="composer min-h-[44px] flex-1 resize-none rounded-xl border border-line bg-base/60 px-3.5 py-2.5 text-sm leading-6 text-ink outline-none placeholder:text-muted/50 focus:border-primary/60"
                />
                <Button size="sm" loading={sending} onClick={sendNote} disabled={!input.trim()}>
                  <Send size={13} />
                  {t('发送', 'Send')}
                </Button>
              </div>
              <p className="mt-1.5 text-[11px] text-muted/70">
                {taskRunning
                  ? t(
                      '当前运行不会中断；内容会写入时间线，并在下一阶段或重试时加入上下文',
                      'The current run won’t be interrupted; the content is added to the timeline and included in context on the next stage or retry',
                    )
                  : taskPaused
                    ? t(
                        '任务已暂停；内容会写入时间线，继续时从当前阶段重跑并加入上下文',
                        'Task is paused; the content is added to the timeline and included in context when resuming from the current stage',
                      )
                    : t(
                        '内容会保留在时间线，下次派发时加入上下文',
                        'Content stays in the timeline and is included in context on the next dispatch',
                      )}
              </p>
            </div>
          </Card>

          <aside className="flex w-[340px] shrink-0 flex-col gap-4 overflow-y-auto">
            <Card>
              <SectionTitle
                icon={ListChecks}
                title={t('任务信息', 'Task info')}
                extra={
                  <span className="text-xs text-muted">{formatDateTime(task.createdAt)}</span>
                }
              />
              <div className="space-y-3 text-sm leading-6 text-muted">
                <div>
                  <p className="text-xs text-muted/70">{t('描述', 'Description')}</p>
                  <p className="whitespace-pre-wrap text-ink/85">
                    {task.description || t('暂无描述', 'No description')}
                  </p>
                </div>
                <div>
                  <p className="text-xs text-muted/70">{t('验收标准', 'Acceptance criteria')}</p>
                  <p className="whitespace-pre-wrap text-ink/85">
                    {task.acceptance || t('暂无验收标准', 'No acceptance criteria')}
                  </p>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <p className="text-xs text-muted/70">{t('模型', 'Model')}</p>
                    <p className="truncate text-ink/85" title={task.model ?? ''}>
                      {task.model || t('跟随 Agent', 'Follow Agent')}
                    </p>
                  </div>
                  <div>
                    <p className="text-xs text-muted/70">{t('思考强度', 'Reasoning effort')}</p>
                    <p className="text-ink/85">
                      {task.reasoning === 'low'
                        ? t('低', 'Low')
                        : task.reasoning === 'high'
                          ? t('高', 'High')
                          : task.reasoning === 'medium'
                            ? t('中', 'Medium')
                            : t('默认', 'Default')}
                    </p>
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <p className="text-xs text-muted/70">{t('授权', 'Permission')}</p>
                    <p className="text-ink/85">
                      {task.mandate === 'auto'
                        ? t('任务内自动', 'Auto within task')
                        : t('逐次确认', 'Confirm each time')}
                    </p>
                  </div>
                  <div>
                    <p className="text-xs text-muted/70">{t('工作目录', 'Working directory')}</p>
                    <p className="text-ink/85">
                      {task.workDir
                        ? t('隔离工作区', 'Isolated workspace')
                        : t('项目目录', 'Project directory')}
                    </p>
                  </div>
                </div>
                {task.workDir && (
                  <p className="break-all font-mono text-xs leading-5 text-muted/70">
                    {task.workDir}
                  </p>
                )}
              </div>
              <div className="mt-4 grid grid-cols-2 divide-x divide-line/60 border-t border-line/60 pt-3">
                <div className="pr-3">
                  <p className="flex items-center gap-1 text-xs text-muted">
                    <Coins size={11} /> Tokens
                  </p>
                  <p className="mt-1 font-mono text-sm tabular-nums text-ink">
                    {formatTokens(task.actualTokens)}
                  </p>
                </div>
                <div className="pl-3">
                  <p className="flex items-center gap-1 text-xs text-muted">
                    <Wallet size={11} /> {t('成本', 'Cost')}
                  </p>
                  <p className="mt-1 font-mono text-sm tabular-nums text-ink">
                    {microsToUsd(task.actualCostMicros)}
                  </p>
                </div>
              </div>
            </Card>

            <Card>
              <SectionTitle
                icon={ListChecks}
                title={t('验收清单', 'Acceptance checklist')}
                extra={
                  <span className="text-xs tabular-nums text-muted">
                    {checklist.filter((item) => item.done).length}/{checklist.length}
                  </span>
                }
              />
              {checklist.length === 0 ? (
                <div className="space-y-2">
                  <p className="text-xs leading-5 text-muted">
                    {t(
                      '把验收标准拆成可勾选条目，逐项确认交付是否达标。',
                      'Break acceptance criteria into checkable items and confirm each one.',
                    )}
                  </p>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={checklistBusy || !task.acceptance}
                    onClick={() => void generateChecklist()}
                  >
                    <Plus size={12} />
                    {t('按验收标准生成', 'Generate from criteria')}
                  </Button>
                </div>
              ) : (
                <div className="space-y-1">
                  {checklist.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      disabled={checklistBusy}
                      onClick={() => void toggleChecklist(item)}
                      className="flex w-full items-start gap-2 rounded-md px-1 py-1 text-left transition-colors hover:bg-soft/60 disabled:opacity-60"
                    >
                      <span
                        className={cn(
                          'mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded border',
                          item.done
                            ? 'border-success bg-success/15 text-success'
                            : 'border-line text-transparent',
                        )}
                      >
                        <Check size={11} />
                      </span>
                      <span
                        className={cn(
                          'text-xs leading-5',
                          item.done ? 'text-muted line-through' : 'text-ink/85',
                        )}
                      >
                        {item.text}
                      </span>
                    </button>
                  ))}
                  <div className="flex items-center justify-between pt-1">
                    <span className="text-[11px] text-muted/70">
                      {t(
                        `${checklist.filter((item) => item.done).length}/${checklist.length} 已完成`,
                        `${checklist.filter((item) => item.done).length}/${checklist.length} completed`,
                      )}
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={checklistBusy || !task.acceptance}
                      onClick={() => void generateChecklist()}
                    >
                      {t('重新生成', 'Regenerate')}
                    </Button>
                  </div>
                </div>
              )}
            </Card>

            <Card>
              <SectionTitle
                icon={Link2}
                title={t('协作件', 'Related work')}
                extra={
                  <span className="text-xs text-muted">
                    {t(
                      `${children.length} 子任务 · ${deps.length} 依赖 · ${attachments.length} 附件`,
                      `${children.length} subtasks · ${deps.length} dependencies · ${attachments.length} attachments`,
                    )}
                  </span>
                }
              />
              <div className="space-y-4">
                <div>
                  <div className="flex items-center justify-between">
                    <p className="text-xs font-medium text-muted">{t('子任务', 'Subtasks')}</p>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setSubtaskOpen(true)}
                    >
                      <Plus size={11} />
                      {t('新建', 'New')}
                    </Button>
                  </div>
                  {children.length === 0 ? (
                    <p className="mt-1 text-xs text-muted/70">{t('暂无子任务', 'No subtasks')}</p>
                  ) : (
                    <div className="mt-1.5 space-y-1">
                      {children.map((child) => {
                        const childStage = taskStage(child.status);
                        return (
                          <Link
                            key={child.id}
                            to={`/tasks/${child.id}`}
                            className="flex items-center justify-between gap-2 rounded-md border border-line/60 px-2 py-1.5 text-xs transition-colors hover:border-primary/40"
                          >
                            <span className="truncate">{child.title}</span>
                            <span
                              className={cn(
                                'shrink-0 rounded px-1.5 py-0.5 text-[10px]',
                                childStage.bg,
                                childStage.text,
                              )}
                            >
                              {childStage.label}
                            </span>
                          </Link>
                        );
                      })}
                    </div>
                  )}
                </div>

                <div>
                  <p className="text-xs font-medium text-muted">{t('依赖', 'Dependencies')}</p>
                  {deps.length === 0 ? (
                    <p className="mt-1 text-xs text-muted/70">
                      {t(
                        '暂无依赖；被依赖任务完成后才允许运行',
                        'No dependencies; runs are allowed once prerequisite tasks complete',
                      )}
                    </p>
                  ) : (
                    <div className="mt-1.5 space-y-1">
                      {deps.map((dep) => {
                        const depStage = taskStage(dep.status);
                        return (
                          <div
                            key={dep.dependsOnId}
                            className="flex items-center justify-between gap-2 rounded-md border border-line/60 px-2 py-1.5 text-xs"
                          >
                            <Link
                              to={`/tasks/${dep.dependsOnId}`}
                              className="truncate transition-colors hover:text-primary"
                            >
                              {dep.title}
                            </Link>
                            <span className="flex shrink-0 items-center gap-1.5">
                              <span className={cn('text-[10px]', depStage.text)}>
                                {depStage.label}
                              </span>
                              <button
                                type="button"
                                className="rounded p-0.5 text-muted transition-colors hover:text-danger"
                                title={t('移除依赖', 'Remove dependency')}
                                aria-label={t('移除依赖', 'Remove dependency')}
                                onClick={() => void removeDependency(dep.dependsOnId)}
                              >
                                <Trash2 size={12} />
                              </button>
                            </span>
                          </div>
                        );
                      })}
                    </div>
                  )}
                  {projectTasks.length > 0 && (
                    <div className="mt-2 flex gap-1.5">
                      <Select
                        value={depPick}
                        onChange={(e) => setDepPick(e.target.value)}
                      >
                        <option value="">{t('选择前置任务', 'Select a prerequisite task')}</option>
                        {projectTasks
                          .filter((item) => !deps.some((dep) => dep.dependsOnId === item.id))
                          .map((item) => (
                            <option key={item.id} value={item.id}>
                              {item.title}
                            </option>
                          ))}
                      </Select>
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={!depPick}
                        onClick={() => void addDependency()}
                      >
                        {t('添加', 'Add')}
                      </Button>
                    </div>
                  )}
                </div>

                <div>
                  <div className="flex items-center justify-between">
                    <p className="text-xs font-medium text-muted">{t('附件', 'Attachments')}</p>
                    <Button
                      variant="ghost"
                      size="sm"
                      loading={attachmentBusy}
                      onClick={() => void addAttachments()}
                    >
                      <Paperclip size={11} />
                      {t('上传', 'Upload')}
                    </Button>
                  </div>
                  {attachments.length === 0 ? (
                    <p className="mt-1 text-xs text-muted/70">
                      {t(
                        '暂无附件；文件会复制到应用数据目录',
                        'No attachments; files are copied to the app data directory',
                      )}
                    </p>
                  ) : (
                    <div className="mt-1.5 space-y-1">
                      {attachments.map((attachment) => (
                        <div
                          key={attachment.id}
                          className="flex items-center justify-between gap-2 rounded-md border border-line/60 px-2 py-1.5 text-xs"
                          title={attachment.path}
                        >
                          <span className="flex min-w-0 items-center gap-1.5">
                            <Paperclip size={11} className="shrink-0 text-muted" />
                            <span className="truncate">{attachment.name}</span>
                            <span className="shrink-0 text-[10px] text-muted/70">
                              {formatBytes(attachment.size)}
                            </span>
                          </span>
                          <span className="flex shrink-0 items-center gap-0.5">
                            <button
                              type="button"
                              className="rounded p-0.5 text-muted transition-colors hover:text-ink"
                              title={t('在文件夹中显示', 'Show in folder')}
                              aria-label={t('在文件夹中显示', 'Show in folder')}
                              onClick={() => void api.taskAttachmentReveal(attachment.id)}
                            >
                              <FolderOpen size={12} />
                            </button>
                            <button
                              type="button"
                              className="rounded p-0.5 text-muted transition-colors hover:text-danger"
                              title={t('删除附件', 'Delete attachment')}
                              aria-label={t('删除附件', 'Delete attachment')}
                              onClick={() => void removeAttachment(attachment)}
                            >
                              <Trash2 size={12} />
                            </button>
                          </span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </Card>

            <Card>
              <SectionTitle
                icon={GitBranch}
                title={t('变更审查', 'Changes review')}
                extra={
                  <span className="flex items-center gap-2">
                    {review?.available && (
                      <span className="font-mono text-xs tabular-nums text-muted">
                        {t(
                          `${review.files.length} 文件 +${review.additions} −${review.deletions}`,
                          `${review.files.length} files +${review.additions} −${review.deletions}`,
                        )}
                      </span>
                    )}
                    <button
                      type="button"
                      className="rounded p-0.5 text-muted transition-colors hover:text-ink"
                      title={t('刷新变更', 'Refresh changes')}
                      aria-label={t('刷新变更', 'Refresh changes')}
                      onClick={() => void refreshReview()}
                    >
                      <RefreshCw size={12} />
                    </button>
                  </span>
                }
              />
              {!review ? (
                <p className="text-xs text-muted">{t('读取中…', 'Loading…')}</p>
              ) : !review.available ? (
                <p className="text-xs leading-5 text-muted">
                  {review.reason ?? t('隔离工作区不可用', 'Isolated workspace unavailable')}
                </p>
              ) : (
                <>
                  {reviewLocked && (
                    <p className="mb-2 text-xs text-muted">
                      {t(
                        '任务运行中，停止后才能提交或还原',
                        'Task is running; stop it before committing or reverting',
                      )}
                    </p>
                  )}
                  {review.pendingCommits > 0 && (
                    <p className="mb-2 text-xs text-muted">
                      {t(
                        `${review.pendingCommits} 个提交待应用到项目`,
                        `${review.pendingCommits} commits pending apply to project`,
                      )}
                    </p>
                  )}
                  {review.files.length === 0 ? (
                    <p className="text-xs text-muted">{t('工作区干净', 'Working tree clean')}</p>
                  ) : (
                    <div className="max-h-[240px] divide-y divide-line/60 overflow-y-auto rounded-lg border border-line/60">
                      {review.files.map((file) => (
                        <button
                          key={file.path}
                          type="button"
                          className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left transition-colors hover:bg-hover"
                          title={file.path}
                          onClick={() => void openDiff(file.path)}
                        >
                          <span
                            className={cn(
                              'w-5 shrink-0 text-center font-mono text-xs font-semibold',
                              REVIEW_STATUS_COLOR[file.status] ?? 'text-muted',
                            )}
                          >
                            {file.code}
                          </span>
                          <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink/85">
                            {file.path}
                          </span>
                          {file.binary ? (
                            <span className="shrink-0 text-xs text-muted">
                              {t('二进制', 'Binary')}
                            </span>
                          ) : (
                            <>
                              <span className="shrink-0 font-mono text-xs tabular-nums text-success">
                                +{file.additions}
                              </span>
                              <span className="shrink-0 font-mono text-xs tabular-nums text-danger">
                                −{file.deletions}
                              </span>
                            </>
                          )}
                        </button>
                      ))}
                    </div>
                  )}
                  {review.truncated && (
                    <p className="mt-1 text-xs text-muted">
                      {t(
                        `文件过多，仅显示前 ${review.files.length} 个`,
                        `Too many files; only the first ${review.files.length} are shown`,
                      )}
                    </p>
                  )}
                  <div className="mt-3 flex flex-wrap items-center gap-1.5">
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={
                        reviewLocked || !review.files.length || Boolean(reviewBusy)
                      }
                      onClick={() => setCommitOpen(true)}
                    >
                      {t('提交', 'Commit')}
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={reviewLocked || !review.canApply || Boolean(reviewBusy)}
                      onClick={() => setConfirmKind('apply')}
                    >
                      {t('应用到项目', 'Apply to project')}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={
                        reviewLocked || !review.files.length || Boolean(reviewBusy)
                      }
                      onClick={() => setConfirmKind('revertAll')}
                    >
                      {t('全部还原', 'Revert all')}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={reviewLocked || Boolean(reviewBusy)}
                      onClick={() => setConfirmKind('cleanup')}
                    >
                      {t('清理', 'Clean up')}
                    </Button>
                  </div>
                  {branchInfo?.available && (
                    <div className="mt-3 flex items-center gap-1.5 border-t border-line/60 pt-2.5">
                      <GitBranch size={12} className="shrink-0 text-muted" />
                      <span
                        className="min-w-0 flex-1 truncate font-mono text-xs text-ink/80"
                        title={branchInfo.current ?? t('游离 HEAD', 'Detached HEAD')}
                      >
                        {branchInfo.current ?? t('游离 HEAD', 'Detached HEAD')}
                      </span>
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={reviewLocked}
                        onClick={() => openBranch('switch')}
                      >
                        {t('切换', 'Switch')}
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={reviewLocked}
                        onClick={() => openBranch('create')}
                      >
                        {t('新建分支', 'New branch')}
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={reviewLocked}
                        onClick={() => {
                          setPrTitle(task.title);
                          setPrBody('');
                          setPrResult(null);
                          setPrOpen(true);
                        }}
                      >
                        {t('创建 PR', 'Create PR')}
                      </Button>
                    </div>
                  )}
                </>
              )}
            </Card>

            {snapshots.length > 0 && (
              <Card>
                <SectionTitle
                  icon={History}
                  title={t('运行快照', 'Run snapshots')}
                  extra={
                    <span className="text-xs text-muted">
                      {t(`${snapshots.length} 份`, `${snapshots.length} snapshots`)}
                    </span>
                  }
                />
                <div className="space-y-1">
                  {snapshots.slice(0, 6).map((snapshot) => (
                    <div
                      key={snapshot.path}
                      className="flex items-center gap-2 rounded-md border border-line/60 px-2 py-1.5 text-xs"
                    >
                      <span className="min-w-0 flex-1 truncate font-mono">
                        {formatDateTime(snapshot.createdAt)}
                      </span>
                      <span className="shrink-0 text-[10px] text-muted/70">
                        {formatBytes(snapshot.size)}
                      </span>
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={reviewLocked}
                        onClick={() => setRollbackTarget(snapshot)}
                      >
                        {t('回滚', 'Roll back')}
                      </Button>
                    </div>
                  ))}
                </div>
                <p className="mt-2 text-[11px] leading-5 text-muted/70">
                  {t(
                    '回滚前会先保存当前状态；未跟踪文件不受影响。',
                    'The current state is saved before rollback; untracked files are unaffected.',
                  )}
                </p>
              </Card>
            )}

            <Card className="flex flex-col">
              <SectionTitle
                icon={Clock}
                title={t('运行记录', 'Runs')}
                extra={
                  <span className="text-xs text-muted">
                    {t(`${runs.length} 次`, `${runs.length} runs`)}
                  </span>
                }
              />
              {runs.length === 0 ? (
                <Empty title={t('暂无运行记录', 'No runs yet')} className="py-6" />
              ) : (
                <div className="max-h-[280px] divide-y divide-line/60 overflow-y-auto rounded-xl border border-line/60">
                  {runs.map((run) => {
                    const running = run.status === 'running' || run.status === 'starting';
                    return (
                      <div key={run.id} className="px-3 py-3">
                        <div className="flex items-center justify-between gap-2">
                          <div className="flex min-w-0 items-center gap-1.5">
                            <span
                              className="h-1.5 w-1.5 shrink-0 rounded-full"
                              style={{ backgroundColor: seatColor(run.seatId ?? run.seatName) }}
                            />
                            <span className="truncate text-xs font-medium text-ink">
                              {run.seatName ?? t('席位', 'Seat')}
                            </span>
                            <span className="truncate text-xs text-muted">
                              {run.agentName ?? t('未指派', 'Unassigned')}
                            </span>
                          </div>
                          <div className="flex shrink-0 items-center gap-1">
                            <IsolationBadge
                              isolation={run.isolation}
                              reason={run.isolationReason}
                            />
                            <RunStatusBadge status={run.status} />
                          </div>
                        </div>
                        <div className="mt-2 grid grid-cols-3 gap-1 font-mono text-xs text-muted">
                          <span className="flex items-center gap-1">
                            <CircleDot size={10} />
                            {formatNumber(run.inputTokens + run.outputTokens)}
                          </span>
                          <span>{microsToUsd(run.costMicros)}</span>
                          <span className="flex items-center gap-1">
                            <Clock size={10} />
                            {formatDuration(run.startedAt, run.endedAt)}
                          </span>
                        </div>
                        {run.error && (
                          <p className="mt-1.5 line-clamp-2 text-xs leading-5 text-danger">
                            {run.error}
                          </p>
                        )}
                        <div className="mt-2 flex items-center gap-3">
                          <Button
                            variant="ghost"
                            size="xs"
                            className="h-auto px-1.5 py-0.5 text-xs"
                            onClick={() => showPrompt(run)}
                          >
                            {t('查看提示词', 'View prompt')}
                          </Button>
                          {running && (
                            <Button
                              variant="ghost"
                              size="xs"
                              className="h-auto px-1.5 py-0.5 text-xs hover:text-danger"
                              loading={stoppingRun === run.id}
                              disabled={stoppingRun === run.id}
                              onClick={() => cancelRun(run)}
                            >
                              <Square size={10} />
                              {stoppingRun === run.id
                                ? t('停止中', 'Stopping…')
                                : t('取消运行', 'Cancel run')}
                            </Button>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </Card>

            <Card>
              <SectionTitle
                icon={Wrench}
                title={t('工具调用', 'Tool calls')}
                extra={
                  <span className="text-xs text-muted">
                    {t(`${toolCalls.length} 次`, `${toolCalls.length} calls`)}
                  </span>
                }
              />
              {toolCalls.length === 0 ? (
                <p className="text-xs leading-5 text-muted">
                  {t('暂无工具调用', 'No tool calls')}
                </p>
              ) : (
                <div className="max-h-[320px] space-y-1.5 overflow-y-auto pr-0.5">
                  {[...toolCalls].reverse().map((call) => {
                    const done = call.status === 'completed';
                    const failed = call.status === 'failed';
                    return (
                      <details
                        key={call.id}
                        className="group rounded-lg border border-line/60 bg-base/40"
                      >
                        <summary className="flex items-center gap-2 px-2.5 py-2">
                          <span
                            className={cn(
                              'h-1.5 w-1.5 shrink-0 rounded-full',
                              done && 'bg-success',
                              failed && 'bg-danger',
                              !done && !failed && 'animate-pulse bg-primary',
                            )}
                          />
                          <span className="min-w-0 flex-1 truncate text-xs font-medium text-ink">
                            {call.toolName}
                          </span>
                          <span className="shrink-0 text-xs text-muted">
                            {TOOL_STATUS_LABEL[call.status] ?? call.status}
                          </span>
                          <span className="shrink-0 font-mono text-xs text-muted/70">
                            {formatDuration(call.createdAt, call.finishedAt ?? undefined)}
                          </span>
                        </summary>
                        <div className="space-y-2 border-t border-line/60 px-2.5 py-2">
                          <div>
                            <p className="mb-1 text-xs text-muted/70">{t('参数', 'Arguments')}</p>
                            <pre className="max-h-36 overflow-auto whitespace-pre-wrap break-all rounded-md bg-soft p-2 font-mono text-xs leading-4 text-ink/80">
                              {previewJson(call.input)}
                            </pre>
                          </div>
                          {call.output !== null && call.output !== undefined && (
                            <div>
                              <p className="mb-1 text-xs text-muted/70">{t('结果', 'Result')}</p>
                              <pre className="max-h-36 overflow-auto whitespace-pre-wrap break-all rounded-md bg-soft p-2 font-mono text-xs leading-4 text-ink/80">
                                {previewJson(call.output)}
                              </pre>
                            </div>
                          )}
                        </div>
                      </details>
                    );
                  })}
                </div>
              )}
            </Card>
          </aside>
        </div>
      </div>

      <Modal
        open={promptOpen}
        onClose={() => setPromptOpen(false)}
        title={t('运行提示词', 'Run prompt')}
        description={
          promptLabel
            ? t(`${promptLabel}，启动时快照`, `${promptLabel}, snapshot at start`)
            : undefined
        }
        width="max-w-3xl"
        footer={
          <Button variant="ghost" onClick={() => setPromptOpen(false)}>
            {t('关闭', 'Close')}
          </Button>
        }
      >
        {promptLoading ? (
          <Loading label={t('读取提示词…', 'Loading prompt…')} />
        ) : (
          <pre className="max-h-[60vh] overflow-auto whitespace-pre-wrap break-words rounded-lg border border-line/60 bg-base/60 p-3 font-mono text-xs leading-5 text-ink/85">
            {promptText}
          </pre>
        )}
      </Modal>

      <Modal
        open={Boolean(diffFile)}
        onClose={() => setDiffFile(null)}
        title={diffFile?.path ?? t('文件差异', 'File diff')}
        description={
          diffFile?.binary ? t('二进制文件不展示内容', 'Binary files are not displayed') : undefined
        }
        width="max-w-4xl"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setDiffFile(null)}>
              {t('关闭', 'Close')}
            </Button>
            <Button
              variant="danger"
              size="sm"
              disabled={!diffFile || Boolean(reviewBusy)}
              onClick={() => diffFile && void revertFile(diffFile.path)}
            >
              {t('还原此文件', 'Revert this file')}
            </Button>
          </>
        }
      >
        {diffLoading ? (
          <Loading label={t('读取差异…', 'Loading diff…')} />
        ) : (
          <pre className="max-h-[58vh] overflow-auto whitespace-pre-wrap break-all rounded-lg border border-line/60 bg-base/60 p-3 font-mono text-xs leading-5 text-ink/85">
            {diffFile?.patch || t('（无差异）', '(No changes)')}
          </pre>
        )}
      </Modal>

      <Modal
        open={commitOpen}
        onClose={() => setCommitOpen(false)}
        title={t('提交隔离工作区改动', 'Commit isolated workspace changes')}
        description={t(
          '提交只发生在隔离工作区，随后可应用到项目。',
          'Commits happen only in the isolated workspace and can then be applied to the project.',
        )}
        width="max-w-md"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setCommitOpen(false)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button
              size="sm"
              loading={reviewBusy === 'commit'}
              disabled={Boolean(reviewBusy)}
              onClick={() => void commitReview()}
            >
              {t('提交', 'Commit')}
            </Button>
          </>
        }
      >
        <Input
          autoFocus
          value={commitMessage}
          onChange={(e) => setCommitMessage(e.target.value)}
          placeholder={t('提交说明，留空使用默认说明', 'Commit message; leave empty for the default')}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.nativeEvent.isComposing) void commitReview();
          }}
        />
      </Modal>

      <Modal
        open={branchOpen}
        onClose={() => setBranchOpen(false)}
        title={branchMode === 'create' ? t('新建分支', 'New branch') : t('切换分支', 'Switch branch')}
        description={t(
          '仅在隔离工作区生效；切换前请先提交或还原改动。',
          'Only applies to the isolated workspace; commit or revert changes before switching.',
        )}
        width="max-w-md"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setBranchOpen(false)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button
              size="sm"
              loading={branchBusy}
              disabled={!branchName.trim() || branchBusy}
              onClick={() => void performBranch()}
            >
              {branchMode === 'create' ? t('创建并切换', 'Create and switch') : t('切换', 'Switch')}
            </Button>
          </>
        }
      >
        {branchMode === 'create' ? (
          <Input
            autoFocus
            value={branchName}
            placeholder="feature/lumen-delivery"
            onChange={(e) => setBranchName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void performBranch();
            }}
          />
        ) : (
          <Select value={branchName} onChange={(e) => setBranchName(e.target.value)}>
            <option value="">{t('选择分支', 'Select a branch')}</option>
            {branchInfo?.branches
              .filter((branch) => branch !== branchInfo.current)
              .map((branch) => (
                <option key={branch} value={branch}>
                  {branch}
                </option>
              ))}
          </Select>
        )}
      </Modal>

      <Modal
        open={Boolean(rollbackTarget)}
        onClose={() => setRollbackTarget(null)}
        title={t('回滚到运行前快照', 'Roll back to pre-run snapshot')}
        description={t(
          '会先为当前状态再保存一份快照，然后重置项目已跟踪文件并应用历史补丁；未跟踪文件不会被删除。',
          'Saves another snapshot of the current state first, then resets tracked project files and applies the historical patch; untracked files are not deleted.',
        )}
        width="max-w-md"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setRollbackTarget(null)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button
              size="sm"
              loading={rollbackBusy}
              disabled={rollbackBusy}
              onClick={() => void performRollback()}
            >
              {t('回滚', 'Roll back')}
            </Button>
          </>
        }
      >
        <p className="rounded-lg border border-line/60 bg-soft/40 px-3 py-2 font-mono text-xs text-ink/80">
          {rollbackTarget ? formatDateTime(rollbackTarget.createdAt) : ''}
        </p>
      </Modal>

      <Modal
        open={prOpen}
        onClose={() => setPrOpen(false)}
        title={t('创建 Pull Request', 'Create Pull Request')}
        description={t(
          '优先调用已登录的 gh；不可用时给出可复制的命令与说明。',
          'Uses the signed-in gh when available; otherwise shows a copyable command and instructions.',
        )}
        width="max-w-lg"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setPrOpen(false)}>
              {t('关闭', 'Close')}
            </Button>
            <Button
              size="sm"
              loading={prBusy}
              disabled={prBusy}
              onClick={() => void performPr()}
            >
              {t('生成 PR', 'Generate PR')}
            </Button>
          </>
        }
      >
        <div className="space-y-2">
          <Input
            value={prTitle}
            placeholder={t('PR 标题', 'PR title')}
            onChange={(e) => setPrTitle(e.target.value)}
          />
          <Textarea
            rows={4}
            value={prBody}
            placeholder={t('PR 说明（可选）', 'PR description (optional)')}
            onChange={(e) => setPrBody(e.target.value)}
          />
          {prResult && (
            <div className="rounded-lg border border-line/60 bg-soft/40 p-2.5 text-xs leading-5">
              {prResult.mode === 'created' ? (
                <div className="flex items-center gap-2">
                  <span className="text-success">{t('PR 已创建', 'PR created')}</span>
                  {prResult.url && (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => void copyText(prResult.url ?? '')}
                    >
                      {t('复制链接', 'Copy link')}
                    </Button>
                  )}
                </div>
              ) : (
                <p
                  className={cn(
                    prResult.mode === 'blocked' ? 'text-danger' : 'text-ink/80',
                  )}
                >
                  {prResult.message ?? t('请手动执行命令', 'Run the command manually')}
                </p>
              )}
              {prResult.command && (
                <div className="mt-2">
                  <pre className="max-h-32 overflow-auto whitespace-pre-wrap rounded bg-canvas/70 p-2 font-mono text-[11px] text-ink/80">
                    {prResult.command}
                  </pre>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="mt-1"
                    onClick={() => void copyText(prResult.command)}
                  >
                    {t('复制命令', 'Copy command')}
                  </Button>
                </div>
              )}
            </div>
          )}
        </div>
      </Modal>

      <Modal
        open={subtaskOpen}
        onClose={() => setSubtaskOpen(false)}
        title={t('新建子任务', 'New subtask')}
        description={t(
          '子任务沿用当前项目与团队，可独立派发、运行与计费。',
          'Subtasks inherit the current project and team and can be dispatched, run, and billed independently.',
        )}
        width="max-w-md"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setSubtaskOpen(false)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button
              size="sm"
              loading={busy === 'subtask'}
              disabled={!subtaskTitle.trim() || busy === 'subtask'}
              onClick={() => void createSubtask()}
            >
              {t('创建', 'Create')}
            </Button>
          </>
        }
      >
        <Input
          autoFocus
          value={subtaskTitle}
          placeholder={t('子任务标题', 'Subtask title')}
          onChange={(e) => setSubtaskTitle(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void createSubtask();
          }}
        />
      </Modal>

      <Modal
        open={reassignOpen}
        onClose={() => setReassignOpen(false)}
        title={t('改派当前阶段', 'Reassign current stage')}
        description={t(
          '将替换当前阶段席位的 Agent，并从当前阶段重跑；已完成的阶段不受影响。',
          'Replaces the Agent in the current stage and reruns from it; completed stages are unaffected.',
        )}
        width="max-w-md"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setReassignOpen(false)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button
              size="sm"
              loading={busy === 'reassign'}
              disabled={!reassignAgentId || busy === 'reassign'}
              onClick={() => void performReassign()}
            >
              {t('改派并重跑', 'Reassign and rerun')}
            </Button>
          </>
        }
      >
        <div className="space-y-3">
          <p className="text-xs leading-5 text-muted">
            {t('当前阶段：', 'Current stage: ')}
            {seats[task.stageIndex]?.roleName ?? '—'} ·{' '}
            {seats[task.stageIndex]?.agentName ?? t('未指派', 'Unassigned')}
          </p>
          <Select
            value={reassignAgentId}
            onChange={(e) => setReassignAgentId(e.target.value)}
          >
            <option value="">{t('选择 Agent', 'Select an Agent')}</option>
            {agents.map((agent) => (
              <option key={agent.id} value={agent.id}>
                {agent.name}
              </option>
            ))}
          </Select>
        </div>
      </Modal>

      <Modal
        open={Boolean(runConfirm)}
        onClose={() => setRunConfirm(null)}
        title={
          runConfirm === 'retry'
            ? t('非隔离重试确认', 'Confirm non-isolated retry')
            : runConfirm === 'resume'
              ? t('非隔离继续确认', 'Confirm non-isolated resume')
              : t('非隔离派发确认', 'Confirm non-isolated dispatch')
        }
        width="max-w-md"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setRunConfirm(null)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button
              size="sm"
              loading={Boolean(runConfirm && busy === runConfirm)}
              disabled={Boolean(runConfirm && busy === runConfirm)}
              onClick={() => runConfirm && void performRun(runConfirm)}
            >
              {runConfirm === 'retry'
                ? t('重试', 'Retry')
                : runConfirm === 'resume'
                  ? t('继续', 'Resume')
                  : t('派发', 'Dispatch')}
            </Button>
          </>
        }
      >
        <p className="text-sm leading-6 text-ink/85">
          {t(
            '任务未启用隔离工作区，运行会直接修改项目目录。',
            'This task has no isolated workspace; running it modifies the project directory directly.',
          )}
        </p>
        <p className="mt-2 text-xs leading-5 text-muted">
          {project?.isGit === false
            ? t(
                '该项目不是 Git 仓库，改动无法自动回滚；建议先手动备份或初始化 Git。',
                'This project is not a Git repository, so changes cannot be rolled back automatically; back up manually or initialize Git first.',
              )
            : t(
                '运行前会自动生成一份只读补丁快照，可在数据目录的 snapshots 中查看。',
                'A read-only patch snapshot is created before running and can be found in the snapshots folder of the data directory.',
              )}
        </p>
      </Modal>

      <Modal
        open={Boolean(confirmKind)}
        onClose={() => setConfirmKind(null)}
        title={confirmKind ? REVIEW_CONFIRM[confirmKind].title : ''}
        width="max-w-md"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setConfirmKind(null)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button
              size="sm"
              variant={confirmKind === 'apply' ? 'primary' : 'danger'}
              loading={Boolean(confirmKind && reviewBusy === confirmKind)}
              disabled={Boolean(reviewBusy)}
              onClick={() => confirmKind && void runReviewAction(confirmKind)}
            >
              {confirmKind ? REVIEW_CONFIRM[confirmKind].action : t('确认', 'Confirm')}
            </Button>
          </>
        }
      >
        <p className="text-sm leading-6 text-ink/85">
          {confirmKind ? REVIEW_CONFIRM[confirmKind].description : ''}
        </p>
        {confirmKind === 'apply' && review?.pendingCommits ? (
          <p className="mt-2 font-mono text-xs text-muted">
            {t(
              `${review.pendingCommits} 个提交待应用`,
              `${review.pendingCommits} commits pending apply`,
            )}
          </p>
        ) : null}
      </Modal>
    </div>
  );
}
