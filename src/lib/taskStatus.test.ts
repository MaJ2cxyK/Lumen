import { describe, expect, it } from 'vitest';
import {
  canCancelStatus,
  canDispatchStatus,
  canPauseStatus,
  canReassignStatus,
  canResetStatus,
  canResumeStatus,
  canRetryStatus,
  isActiveStatus,
  isInProgressStatus,
  isReviewLockedStatus,
} from './taskStatus';

describe('task status policy', () => {
  it('treats active statuses as run owners and paused as in-progress', () => {
    for (const status of ['queued', 'running', 'waiting_approval', 'paused_budget']) {
      expect(isActiveStatus(status)).toBe(true);
      expect(isInProgressStatus(status)).toBe(true);
    }
    expect(isActiveStatus('paused')).toBe(false);
    expect(isInProgressStatus('paused')).toBe(true);
    expect(isActiveStatus('completed')).toBe(false);
  });

  it('matches the backend transition guards', () => {
    expect(canDispatchStatus('running')).toBe(false);
    expect(canDispatchStatus('waiting_approval')).toBe(false);
    expect(canDispatchStatus('paused')).toBe(false);
    expect(canDispatchStatus('completed')).toBe(true);

    expect(canPauseStatus('queued')).toBe(true);
    expect(canPauseStatus('paused')).toBe(false);

    expect(canResumeStatus('paused')).toBe(true);
    expect(canResumeStatus('blocked')).toBe(false);

    expect(canCancelStatus('paused_budget')).toBe(true);
    expect(canCancelStatus('paused')).toBe(true);
    expect(canCancelStatus('completed')).toBe(false);

    expect(canReassignStatus('blocked')).toBe(true);
    expect(canReassignStatus('running')).toBe(false);

    expect(canResetStatus('running')).toBe(false);
    expect(canResetStatus('blocked')).toBe(true);

    expect(canRetryStatus('blocked')).toBe(true);
    expect(canRetryStatus('paused')).toBe(false);

    expect(isReviewLockedStatus('running')).toBe(true);
    expect(isReviewLockedStatus('paused')).toBe(false);
    expect(isReviewLockedStatus('blocked')).toBe(false);
  });
});
