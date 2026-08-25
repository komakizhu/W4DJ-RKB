import { describe, expect, it } from 'vitest';
import { summarizeLibraryAnalysisResult } from './library-analysis-runner';

describe('library analysis runner result contract', () => {
  it('only reports completed when every candidate is terminally successful', () => {
    expect(summarizeLibraryAnalysisResult({
      total: 16,
      completed: 16,
      failed: 0,
      timedOut: 0,
      cancelled: 0,
      pending: 0,
    })).toBe('completed');
  });

  it('keeps failure and cancellation runs partial', () => {
    expect(summarizeLibraryAnalysisResult({
      total: 16,
      completed: 15,
      failed: 1,
      timedOut: 0,
      cancelled: 0,
      pending: 0,
    })).toBe('partial');
    expect(summarizeLibraryAnalysisResult({
      total: 16,
      completed: 1,
      failed: 0,
      timedOut: 0,
      cancelled: 1,
      pending: 15,
    })).toBe('partial');
  });
});
