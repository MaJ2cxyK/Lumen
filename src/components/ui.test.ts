import { describe, expect, it } from 'vitest';
import { isolationReasonText } from './ui';

describe('isolationReasonText', () => {
  it('explains the unified Git fallback', () => {
    const text = isolationReasonText('git_nul');
    expect(text).toContain('NUL');
    expect(text).toContain('Git');
  });

  it('keeps historical OpenCode Git runs readable', () => {
    expect(isolationReasonText('opencode_git_nul')).toBe(
      isolationReasonText('git_nul'),
    );
  });

  it('passes through unknown reasons', () => {
    expect(isolationReasonText('future_reason')).toBe('future_reason');
  });
});
