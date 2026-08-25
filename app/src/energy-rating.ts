/**
 * Fixed Energy display calibration. The stored value remains Essentia's
 * RMS-squared Energy; these thresholds only change its visible Dashboard
 * representation.
 */
export const ENERGY_LEVEL_THRESHOLDS = [
  0.041982342,
  0.078189338,
  0.111694549,
  0.144505646,
  0.184125843,
  0.214877708,
  0.251289228,
  0.292838207,
  0.355045821,
] as const;

export function energyLevel(value: number | null): number | null {
  if (value == null || !Number.isFinite(value)) return null;

  // Calibrated boundaries are non-negative. Keep finite negative values
  // visible as the first level instead of allowing them to produce an
  // invalid rating.
  const safeValue = Math.max(0, value);
  const boundaryIndex = ENERGY_LEVEL_THRESHOLDS.findIndex(
    (threshold) => safeValue < threshold,
  );
  return boundaryIndex === -1 ? 10 : boundaryIndex + 1;
}

export function formatEnergyRating(value: number | null): string {
  const level = energyLevel(value);
  if (level == null) return '—';

  const solidStars = Math.floor(level / 2);
  const outlineStar = level % 2 === 1 ? '☆' : '';
  return `${'★'.repeat(solidStars)}${outlineStar} ${level}/10`;
}
