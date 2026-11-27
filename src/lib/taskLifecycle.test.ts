import { describe, expect, it } from 'vitest';
import {
  ACTIVE_STATUSES,
  IN_PROGRESS_STATUSES,
  TASK_STATUSES,
  TASK_STAGE_ORDER,
  canCancelStatus,
  canChangeManualStatus,
  canDispatchStatus,
  canPauseStatus,
  canReassignStatus,
  canResetStatus,
  canResumeStatus,
  canRetryStatus,
  isActiveStatus,
  isInProgressStatus,
  isReviewLockedStatus,
  isTaskStatus,
  needsYou,
  needsYouTask,
  isReviewPending,
  stageColumns,
  stageCount,
  stageDescriptor,
  statusDescriptor,
} from './taskLifecycle';

describe('task lifecycle vocabulary', () => {
  it('describes every persisted status and derives its stage', () => {
    for (const status of TASK_STATUSES) {
      const descriptor = statusDescriptor(status);
      expect(descriptor.known).toBe(true);
      expect(descriptor.status).toBe(status);
      expect(TASK_STAGE_ORDER).toContain(descriptor.stage);
    }
    expect(statusDescriptor('completed').stage).toBe('closed');
    expect(statusDescriptor('cancelled').stage).toBe('closed');
  });

  it('shows unknown statuses as unknown instead of Inbox', () => {
    const descriptor = statusDescriptor('legacy_value');
    expect(descriptor.known).toBe(false);
    expect(descriptor.stage).toBe('unknown');
    expect(descriptor.label).not.toBe(statusDescriptor('inbox').label);
  });

  it('splits active work from in-progress counters', () => {
    expect(ACTIVE_STATUSES).toEqual(['queued', 'running', 'waiting_approval']);
    expect(IN_PROGRESS_STATUSES).toEqual([...ACTIVE_STATUSES, 'paused']);
    for (const status of ACTIVE_STATUSES) {
      expect(isActiveStatus(status)).toBe(true);
      expect(isInProgressStatus(status)).toBe(true);
    }
    expect(isActiveStatus('paused')).toBe(false);
    expect(isInProgressStatus('paused')).toBe(true);
    expect(isActiveStatus('legacy_value')).toBe(false);
  });

  it('adds exactly one Unknown column when an unknown value is present', () => {
    expect(stageColumns(TASK_STATUSES)).toEqual([...TASK_STAGE_ORDER]);
    expect(stageColumns(['inbox', 'legacy_value'])).toEqual([
      ...TASK_STAGE_ORDER,
      'unknown',
    ]);
    expect(stageDescriptor('unknown').stage).toBe('unknown');
  });

  it('counts closed and unknown columns without dropping statuses', () => {
    expect(stageCount('closed', { completed: 2, cancelled: 1 }, 0)).toBe(3);
    expect(stageCount('inbox', { inbox: 4 }, 0)).toBe(4);
    expect(stageCount('inbox', {}, 2)).toBe(2);
    expect(
      stageCount('unknown', { legacy_a: 2, legacy_b: 1 }, 0),
    ).toBe(3);
    expect(stageCount('unknown', {}, 2)).toBe(2);
  });
});

describe('task lifecycle guards', () => {
  it('matches the backend transition guards', () => {
    expect(canDispatchStatus('running')).toBe(false);
    expect(canDispatchStatus('waiting_approval')).toBe(false);
    expect(canDispatchStatus('queued')).toBe(true);
    expect(canDispatchStatus('paused')).toBe(true);
    expect(canDispatchStatus('completed')).toBe(true);
    expect(canDispatchStatus('legacy_value')).toBe(false);

    expect(canPauseStatus('queued')).toBe(true);
    expect(canPauseStatus('paused')).toBe(false);

    expect(canResumeStatus('paused')).toBe(true);
    expect(canResumeStatus('blocked')).toBe(false);

    expect(canCancelStatus('waiting_approval')).toBe(true);
    expect(canCancelStatus('paused')).toBe(true);
    expect(canCancelStatus('completed')).toBe(false);
    expect(canCancelStatus('legacy_value')).toBe(false);

    expect(canReassignStatus('blocked')).toBe(true);
    expect(canReassignStatus('running')).toBe(false);

    expect(canResetStatus('running')).toBe(false);
    expect(canResetStatus('blocked')).toBe(true);
    expect(canResetStatus('legacy_value')).toBe(true);

    expect(canRetryStatus('blocked')).toBe(true);
    expect(canRetryStatus('paused')).toBe(false);

    expect(isReviewLockedStatus('running')).toBe(true);
    expect(isReviewLockedStatus('paused')).toBe(false);
    expect(isReviewLockedStatus('legacy_value')).toBe(false);
  });

  it('allows manual status changes only from inactive states', () => {
    expect(canChangeManualStatus('blocked', 'completed')).toBe(true);
    expect(canChangeManualStatus('paused', 'inbox')).toBe(true);
    expect(canChangeManualStatus('blocked', 'running')).toBe(false);
    expect(canChangeManualStatus('running', 'blocked')).toBe(false);
    expect(canChangeManualStatus('legacy_value', 'inbox')).toBe(false);
    expect(canChangeManualStatus('blocked', 'legacy_value')).toBe(false);
  });
});

describe('task attention', () => {
  it('marks only user-waiting statuses as needing attention', () => {
    expect(needsYou('inbox')).toBe(true);
    expect(needsYou('waiting_approval')).toBe(true);
    expect(needsYou('paused')).toBe(true);
    expect(needsYou('blocked')).toBe(true);
    expect(needsYou('running')).toBe(false);
    expect(needsYou('queued')).toBe(false);
    expect(needsYou('completed')).toBe(false);
    expect(needsYou('legacy_value')).toBe(false);
  });

  it('does not count review while the task is still active', () => {
    expect(isReviewPending({ status: 'completed', reviewPending: true })).toBe(true);
    expect(isReviewPending({ status: 'running', reviewPending: true })).toBe(false);
    expect(isReviewPending({ status: 'queued', reviewPending: true })).toBe(false);
    expect(isReviewPending({ status: 'completed', reviewPending: false })).toBe(false);
  });

  it('combines status and review attention', () => {
    expect(needsYouTask({ status: 'completed', reviewPending: true })).toBe(true);
    expect(needsYouTask({ status: 'completed', reviewPending: false })).toBe(false);
    expect(needsYouTask({ status: 'blocked', reviewPending: false })).toBe(true);
    expect(needsYouTask({ status: 'paused', reviewPending: false })).toBe(true);
  });

  it('recognises the persisted vocabulary', () => {
    expect(isTaskStatus('inbox')).toBe(true);
    expect(isTaskStatus('paused_budget')).toBe(false);
    expect(isTaskStatus('failed')).toBe(false);
  });
});
