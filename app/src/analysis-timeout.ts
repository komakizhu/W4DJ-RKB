export const ANALYSIS_MIN_TIMEOUT_MS = 300_000;
export const ANALYSIS_PROGRESS_STALL_TIMEOUT_MS = 300_000;
export const ANALYSIS_TIMEOUT_BUFFER_MS = 60_000;

/**
 * Give each track an overall analysis budget based on its decoded duration.
 * A separate progress watchdog in the Worker client handles a stalled Worker;
 * this budget must therefore not be capped or reset by progress events.
 */
export function analysisTimeoutMs(durationSeconds: number): number {
  const duration = Number.isFinite(durationSeconds) && durationSeconds > 0
    ? durationSeconds
    : 0;
  const adaptive = Math.ceil(duration * 3_000 + ANALYSIS_TIMEOUT_BUFFER_MS);
  return Math.max(ANALYSIS_MIN_TIMEOUT_MS, adaptive);
}
