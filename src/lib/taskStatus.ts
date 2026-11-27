/**
 * Single frontend source of truth for task status policy.
 *
 * Values mirror `src-tauri/src/task_lifecycle.rs`; keep both in sync when the
 * lifecycle changes. "Active" means the task owns a run slot, while
 * "in progress" is the board grouping that also shows paused work.
 */

export const ACTIVE_STATUSES = [
  'queued',
  'running',
  'waiting_approval',
  'paused_budget',
] as const;

export const IN_PROGRESS_STATUSES = [...ACTIVE_STATUSES, 'paused'] as const;

export function isActiveStatus(status: string): boolean {
  return (ACTIVE_STATUSES as readonly string[]).includes(status);
}

export function isInProgressStatus(status: string): boolean {
  return (IN_PROGRESS_STATUSES as readonly string[]).includes(status);
}

/**
 * Dispatch affordance. Paused tasks expose Resume instead, so the UI hides
 * dispatch there even though the backend would accept a re-dispatch.
 */
export function canDispatchStatus(status: string): boolean {
  return !['running', 'waiting_approval', 'paused'].includes(status);
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

export function canResetStatus(status: string): boolean {
  return !isActiveStatus(status);
}

export function canRetryStatus(status: string): boolean {
  return status === 'blocked';
}

/** `TaskStatus::is_review_locked` — review actions must not race a stage. */
export function isReviewLockedStatus(status: string): boolean {
  return ['queued', 'running', 'waiting_approval'].includes(status);
}
