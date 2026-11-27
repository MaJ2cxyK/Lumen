import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  hiddenCliNames,
  setCliHidden,
  subscribeCliVisibility,
} from './visibility';

describe('cli visibility preferences', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('persists hidden names and emits changes', () => {
    const listener = vi.fn();
    const unsubscribe = subscribeCliVisibility(listener);

    setCliHidden('Codex', true);
    expect([...hiddenCliNames()]).toEqual(['Codex']);
    expect(listener).toHaveBeenCalledTimes(1);

    setCliHidden('Codex', false);
    expect(hiddenCliNames().size).toBe(0);
    expect(listener).toHaveBeenCalledTimes(2);

    unsubscribe();
  });

  it('ignores malformed stored values', () => {
    localStorage.setItem('lumen.cli.hidden', '{"bad":true}');
    expect(hiddenCliNames().size).toBe(0);
  });
});
