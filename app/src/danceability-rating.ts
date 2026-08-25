export const DANCEABILITY_CURVE_SLOPE = 4.48056;
export const DANCEABILITY_CURVE_MIDPOINT = 1.10370;

export function danceabilityLevel(value: number | null): number | null {
  if (value == null || !Number.isFinite(value)) return null;
  const continuous = 1 + 9 / (
    1 + Math.exp(-DANCEABILITY_CURVE_SLOPE * (value - DANCEABILITY_CURVE_MIDPOINT))
  );
  return Math.min(10, Math.max(1, Math.round(continuous)));
}

export function formatDanceabilityRating(value: number | null): string {
  const level = danceabilityLevel(value);
  if (level == null) return '—';
  const solidStars = Math.floor(level / 2);
  const outlineStar = level % 2 === 1 ? '☆' : '';
  return `${'★'.repeat(solidStars)}${outlineStar} ${level}/10`;
}
