import { describe, expect, it } from 'vitest';
import { headlessExitCode } from './headless-acceptance';

describe('headless acceptance protocol', () => {
  it('returns zero only for a fully completed run', () => {
    expect(headlessExitCode({
      total: 16,
      completed: 16,
      failed: 0,
      timedOut: 0,
      cancelled: 0,
      pending: 0,
    })).toBe(0);
  });

  it('returns partial for failures, cancellation, or pending tracks', () => {
    expect(headlessExitCode({
      total: 16,
      completed: 15,
      failed: 1,
      timedOut: 0,
      cancelled: 0,
      pending: 0,
    })).toBe(2);
    expect(headlessExitCode({
      total: 16,
      completed: 1,
      failed: 0,
      timedOut: 0,
      cancelled: 1,
      pending: 15,
    })).toBe(2);
  });
});
