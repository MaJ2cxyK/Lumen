import type { Task } from './types';
import { tr } from './i18n';
import { isActiveStatus } from './taskStatus';

/**
 * 展示阶段从任务状态派生：同一时刻只属于一个阶段，
 * 并用“在等谁”决定它是否应该占用用户注意力。
 */
export type TaskStageKey =
  | 'inbox'
  | 'queued'
  | 'running'
  | 'waiting_approval'
  | 'paused'
  | 'blocked'
  | 'completed'
  | 'failed';

export interface TaskStageMeta {
  key: TaskStageKey;
  label: string;
  waitingOn: 'you' | 'agent' | 'none';
  hint: string;
  text: string;
  bg: string;
  dot: string;
}

const STAGES: Record<TaskStageKey, TaskStageMeta> = {
  inbox: {
    key: 'inbox',
    get label() {
      return tr('待分派', 'Inbox');
    },
    waitingOn: 'you',
    get hint() {
      return tr('等你派发', 'Waiting for you to dispatch');
    },
    text: 'text-muted',
    bg: 'bg-soft',
    dot: 'bg-muted',
  },
  queued: {
    key: 'queued',
    get label() {
      return tr('排队中', 'Queued');
    },
    waitingOn: 'agent',
    get hint() {
      return tr('等待运行槽', 'Waiting for a run slot');
    },
    text: 'text-info',
    bg: 'bg-info/10',
    dot: 'bg-info',
  },
  running: {
    key: 'running',
    get label() {
      return tr('执行中', 'Running');
    },
    waitingOn: 'agent',
    get hint() {
      return tr('Agent 正在执行', 'Agent is working');
    },
    text: 'text-primary',
    bg: 'bg-primary/10',
    dot: 'bg-primary',
  },
  waiting_approval: {
    key: 'waiting_approval',
    get label() {
      return tr('等待你审批', 'Awaiting your approval');
    },
    waitingOn: 'you',
    get hint() {
      return tr('审批后继续', 'Continues after approval');
    },
    text: 'text-warn',
    bg: 'bg-warn/10',
    dot: 'bg-warn',
  },
  paused: {
    key: 'paused',
    get label() {
      return tr('已暂停', 'Paused');
    },
    waitingOn: 'you',
    get hint() {
      return tr('继续后从当前阶段重跑', 'Resumes by re-running the current stage');
    },
    text: 'text-info',
    bg: 'bg-info/10',
    dot: 'bg-info',
  },
  blocked: {
    key: 'blocked',
    get label() {
      return tr('被阻塞', 'Blocked');
    },
    waitingOn: 'you',
    get hint() {
      return tr('需要你处理阻塞', 'Needs you to resolve the blocker');
    },
    text: 'text-danger',
    bg: 'bg-danger/10',
    dot: 'bg-danger',
  },
  completed: {
    key: 'completed',
    get label() {
      return tr('已完成', 'Completed');
    },
    waitingOn: 'none',
    get hint() {
      return tr('无需介入', 'No action needed');
    },
    text: 'text-success',
    bg: 'bg-success/10',
    dot: 'bg-success',
  },
  failed: {
    key: 'failed',
    get label() {
      return tr('失败', 'Failed');
    },
    waitingOn: 'you',
    get hint() {
      return tr('需要你查看失败原因', 'Review the failure reason');
    },
    text: 'text-danger',
    bg: 'bg-danger/10',
    dot: 'bg-danger',
  },
};

export const TASK_STAGE_ORDER: TaskStageKey[] = [
  'inbox',
  'queued',
  'running',
  'waiting_approval',
  'paused',
  'blocked',
  'completed',
];

export function taskStage(status: string): TaskStageMeta {
  return STAGES[status as TaskStageKey] ?? STAGES.inbox;
}

export function needsYou(status: string): boolean {
  return taskStage(status).waitingOn === 'you';
}

/** 任务不在运行中，但隔离工作区仍有未提交或未应用的改动。 */
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

