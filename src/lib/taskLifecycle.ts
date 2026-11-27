import { tr } from './i18n';
import type { Task, TaskStatus } from './types';

export type { TaskStatus };

/**
 * The persisted task vocabulary. Mirrors `src-tauri/src/task_lifecycle.rs`;
 * `statusDescriptor` is exhaustive over this union, so adding a status here
 * forces a label and a stage mapping.
 */
export const TASK_STATUSES: readonly TaskStatus[] = [
  'inbox',
  'queued',
  'running',
  'waiting_approval',
  'paused',
  'blocked',
  'completed',
  'cancelled',
];

/** Statuses that own a run slot or an in-flight pipeline. */
export const ACTIVE_STATUSES: readonly TaskStatus[] = [
  'queued',
  'running',
  'waiting_approval',
];

/** Active work plus paused work, used by aggregate counters. */
export const IN_PROGRESS_STATUSES: readonly TaskStatus[] = [...ACTIVE_STATUSES, 'paused'];

/** Statuses a user may set directly; mirrors the Rust manual-target list. */
export const MANUAL_STATUS_TARGETS: readonly TaskStatus[] = [
  'inbox',
  'blocked',
  'completed',
  'cancelled',
];

export type TaskStage =
  | 'inbox'
  | 'queued'
  | 'running'
  | 'waiting_approval'
  | 'paused'
  | 'blocked'
  | 'closed'
  | 'unknown';

export const TASK_STAGE_ORDER: readonly TaskStage[] = [
  'inbox',
  'queued',
  'running',
  'waiting_approval',
  'paused',
  'blocked',
  'closed',
];

export interface ToneMeta {
  label: string;
  text: string;
  bg: string;
  dot: string;
}

export interface TaskStatusDescriptor extends ToneMeta {
  status: TaskStatus | 'unknown';
  stage: TaskStage;
  known: boolean;
  attention: boolean;
  hint: string;
}

export interface TaskStageDescriptor extends ToneMeta {
  stage: TaskStage;
  hint: string;
}

function tone(
  zh: string,
  en: string,
  text: string,
  bg: string,
  dot: string,
): ToneMeta {
  return {
    get label() {
      return tr(zh, en);
    },
    text,
    bg,
    dot,
  };
}

function makeStatusDescriptor(
  status: TaskStatus,
  stage: TaskStage,
  attention: boolean,
  zhLabel: string,
  enLabel: string,
  zhHint: string,
  enHint: string,
  text: string,
  bg: string,
  dot: string,
): TaskStatusDescriptor {
  return {
    ...tone(zhLabel, enLabel, text, bg, dot),
    status,
    stage,
    known: true,
    attention,
    get hint() {
      return tr(zhHint, enHint);
    },
  };
}

const STATUS_DESCRIPTORS: Record<TaskStatus, TaskStatusDescriptor> = {
  inbox: makeStatusDescriptor(
    'inbox',
    'inbox',
    true,
    '收件箱',
    'Inbox',
    '等你派发',
    'Waiting for you to dispatch',
    'text-muted',
    'bg-soft',
    'bg-muted',
  ),
  queued: makeStatusDescriptor(
    'queued',
    'queued',
    false,
    '排队中',
    'Queued',
    '等待运行槽',
    'Waiting for a run slot',
    'text-info',
    'bg-info/10',
    'bg-info',
  ),
  running: makeStatusDescriptor(
    'running',
    'running',
    false,
    '运行中',
    'Running',
    'Agent 正在执行',
    'Agent is working',
    'text-primary',
    'bg-primary/10',
    'bg-primary',
  ),
  waiting_approval: makeStatusDescriptor(
    'waiting_approval',
    'waiting_approval',
    true,
    '待审批',
    'Awaiting approval',
    '审批后继续',
    'Continues after approval',
    'text-warn',
    'bg-warn/10',
    'bg-warn',
  ),
  paused: makeStatusDescriptor(
    'paused',
    'paused',
    true,
    '已暂停',
    'Paused',
    '继续后从当前阶段重跑',
    'Resumes by re-running the current stage',
    'text-info',
    'bg-info/10',
    'bg-info',
  ),
  blocked: makeStatusDescriptor(
    'blocked',
    'blocked',
    true,
    '已阻塞',
    'Blocked',
    '需要你处理阻塞',
    'Needs you to resolve the blocker',
    'text-danger',
    'bg-danger/10',
    'bg-danger',
  ),
  completed: makeStatusDescriptor(
    'completed',
    'closed',
    false,
    '已完成',
    'Completed',
    '无需介入',
    'No action needed',
    'text-success',
    'bg-success/10',
    'bg-success',
  ),
  cancelled: makeStatusDescriptor(
    'cancelled',
    'closed',
    false,
    '已取消',
    'Cancelled',
    '无需介入',
    'No action needed',
    'text-muted',
    'bg-soft',
    'bg-muted',
  ),
};

const UNKNOWN_STATUS: TaskStatusDescriptor = {
  ...tone('未知', 'Unknown', 'text-muted', 'bg-soft', 'bg-muted'),
  status: 'unknown',
  stage: 'unknown',
  known: false,
  attention: false,
  get hint() {
    return tr('状态未知，仅可重置修复', 'Unknown status; reset to repair');
  },
};

const STAGE_DESCRIPTORS: Record<TaskStage, TaskStageDescriptor> = {
  inbox: {
    ...tone('待分派', 'Inbox', 'text-muted', 'bg-soft', 'bg-muted'),
    stage: 'inbox',
    get hint() {
      return tr('等你派发', 'Waiting for you to dispatch');
    },
  },
  queued: {
    ...tone('排队中', 'Queued', 'text-info', 'bg-info/10', 'bg-info'),
    stage: 'queued',
    get hint() {
      return tr('等待运行槽', 'Waiting for a run slot');
    },
  },
  running: {
    ...tone('执行中', 'Running', 'text-primary', 'bg-primary/10', 'bg-primary'),
    stage: 'running',
    get hint() {
      return tr('Agent 正在执行', 'Agent is working');
    },
  },
  waiting_approval: {
    ...tone('等待你审批', 'Awaiting approval', 'text-warn', 'bg-warn/10', 'bg-warn'),
    stage: 'waiting_approval',
    get hint() {
      return tr('审批后继续', 'Continues after approval');
    },
  },
  paused: {
    ...tone('已暂停', 'Paused', 'text-info', 'bg-info/10', 'bg-info'),
    stage: 'paused',
    get hint() {
      return tr('继续后从当前阶段重跑', 'Resumes by re-running the current stage');
    },
  },
  blocked: {
    ...tone('被阻塞', 'Blocked', 'text-danger', 'bg-danger/10', 'bg-danger'),
    stage: 'blocked',
    get hint() {
      return tr('需要你处理阻塞', 'Needs you to resolve the blocker');
    },
  },
  closed: {
    ...tone('已关闭', 'Closed', 'text-success', 'bg-success/10', 'bg-success'),
    stage: 'closed',
    get hint() {
      return tr('已完成或已取消，无需介入', 'Completed or cancelled; no action needed');
    },
  },
  unknown: {
    ...tone('未知状态', 'Unknown', 'text-muted', 'bg-soft', 'bg-muted'),
    stage: 'unknown',
    get hint() {
      return tr('状态未知，仅可重置修复', 'Unknown status; reset to repair');
    },
  },
};

export function isTaskStatus(value: string): value is TaskStatus {
  return (TASK_STATUSES as readonly string[]).includes(value);
}

export function isUnknownStatus(value: string): boolean {
  return !isTaskStatus(value);
}

export function statusDescriptor(status: string): TaskStatusDescriptor {
  return isTaskStatus(status) ? STATUS_DESCRIPTORS[status] : UNKNOWN_STATUS;
}

export function stageDescriptor(stage: TaskStage): TaskStageDescriptor {
  return STAGE_DESCRIPTORS[stage];
}

/**
 * The board always renders the seven known stages, plus an Unknown column
 * when a value outside the vocabulary is present. Unknown statuses are never
 * merged into another column.
 */
export function stageColumns(statuses: Iterable<string>): TaskStage[] {
  let hasUnknown = false;
  for (const status of statuses) {
    if (!isTaskStatus(status)) {
      hasUnknown = true;
      break;
    }
  }
  return hasUnknown ? [...TASK_STAGE_ORDER, 'unknown'] : [...TASK_STAGE_ORDER];
}

export function stageCount(
  stage: TaskStage,
  statusCounts: Record<string, number>,
  visibleItems: number,
): number {
  if (stage === 'closed') {
    return (statusCounts.completed ?? 0) + (statusCounts.cancelled ?? 0);
  }
  if (stage === 'unknown') {
    const unknownKeys = Object.keys(statusCounts).filter((key) => !isTaskStatus(key));
    if (unknownKeys.length === 0) return visibleItems;
    return unknownKeys.reduce((sum, key) => sum + (statusCounts[key] ?? 0), 0);
  }
  return statusCounts[stage] ?? visibleItems;
}

export function isActiveStatus(status: string): boolean {
  return isTaskStatus(status) && (ACTIVE_STATUSES as readonly string[]).includes(status);
}

export function isInProgressStatus(status: string): boolean {
  return (
    isTaskStatus(status) && (IN_PROGRESS_STATUSES as readonly string[]).includes(status)
  );
}

/**
 * Dispatch affordance and backend guard. Queued re-dispatch is allowed;
 * running and awaiting-approval tasks must be stopped first.
 */
export function canDispatchStatus(status: string): boolean {
  return isTaskStatus(status) && status !== 'running' && status !== 'waiting_approval';
}

export function canPauseStatus(status: string): boolean {
  return isActiveStatus(status);
}

export function canResumeStatus(status: string): boolean {
  return status === 'paused';
}

export function canCancelStatus(status: string): boolean {
  return isActiveStatus(status) || status === 'paused';
}

export function canReassignStatus(status: string): boolean {
  return status === 'paused' || status === 'blocked';
}

/** Unknown values may only be repaired by a reset to inbox. */
export function canResetStatus(status: string): boolean {
  return !isActiveStatus(status);
}

export function canRetryStatus(status: string): boolean {
  return status === 'blocked';
}

/** Review actions must not race a stage that is still moving. */
export function isReviewLockedStatus(status: string): boolean {
  return isActiveStatus(status);
}

/** Mirrors `TaskStatus::can_change_manual` in the Rust lifecycle module. */
export function canChangeManualStatus(from: string, to: string): boolean {
  if (!isTaskStatus(from) || !isTaskStatus(to)) return false;
  return (
    !(ACTIVE_STATUSES as readonly string[]).includes(from) &&
    (MANUAL_STATUS_TARGETS as readonly string[]).includes(to)
  );
}

export function needsYou(status: string): boolean {
  const descriptor = statusDescriptor(status);
  return descriptor.known && descriptor.attention;
}

/** An inactive task whose isolated workspace still has unapplied changes. */
export function isReviewPending(
  task: Pick<Task, 'status' | 'reviewPending'>,
): boolean {
  if (!task.reviewPending) return false;
  return !isActiveStatus(task.status);
}

export function needsYouTask(
  task: Pick<Task, 'status' | 'reviewPending'>,
): boolean {
  return needsYou(task.status) || isReviewPending(task);
}
