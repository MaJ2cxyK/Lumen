import { describe, expect, it } from 'vitest';
import { isReviewPending, needsYou, needsYouTask, taskStage } from './taskStage';

describe('taskStage', () => {
  it('falls back to inbox for unknown statuses', () => {
    expect(taskStage('unknown').key).toBe('inbox');
  });

  it('marks only user-waiting stages as needing attention', () => {
    expect(needsYou('inbox')).toBe(true);
    expect(needsYou('waiting_approval')).toBe(true);
    expect(needsYou('blocked')).toBe(true);
    expect(needsYou('failed')).toBe(true);
    expect(needsYou('running')).toBe(false);
    expect(needsYou('queued')).toBe(false);
    expect(needsYou('completed')).toBe(false);
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
  });
});
