export const ANALYSIS_MIN_TIMEOUT_MS = 300_000;
export const ANALYSIS_MAX_TIMEOUT_MS = 900_000;
export const ANALYSIS_TIMEOUT_BUFFER_MS = 60_000;

/**
 * Allow long tracks more time while keeping a broken worker from running
 * forever. The duration is measured after Web Audio decoding/resampling.
 */
export function analysisTimeoutMs(durationSeconds: number): number {
  const duration = Number.isFinite(durationSeconds) && durationSeconds > 0
    ? durationSeconds
    : 0;
  const adaptive = Math.ceil(duration * 3_000 + ANALYSIS_TIMEOUT_BUFFER_MS);
  return Math.min(
    ANALYSIS_MAX_TIMEOUT_MS,
    Math.max(ANALYSIS_MIN_TIMEOUT_MS, adaptive),
  );
}
