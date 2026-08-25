import { describe, expect, it } from 'vitest';
import { danceabilityLevel, formatDanceabilityRating } from './danceability-rating';

describe('danceability ten-level rating', () => {
  it('maps the approved calibration anchors', () => {
    expect(danceabilityLevel(0.8240978122)).toBe(3);
    expect(danceabilityLevel(1.1535)).toBe(6);
    expect(danceabilityLevel(2.8114326)).toBe(10);
  });

  it('handles missing and finite extreme values', () => {
    expect(danceabilityLevel(null)).toBeNull();
    expect(danceabilityLevel(Number.NaN)).toBeNull();
    expect(danceabilityLevel(Number.POSITIVE_INFINITY)).toBeNull();
    expect(danceabilityLevel(-100)).toBe(1);
    expect(danceabilityLevel(100)).toBe(10);
  });

  it('is monotonic across the observed range', () => {
    const inputs = [0, 0.8240978122, 1, 1.1535, 1.5, 2, 2.8114326, 3];
    const levels = inputs.map((value) => danceabilityLevel(value) ?? 0);
    expect(levels).toEqual([...levels].sort((left, right) => left - right));
  });

  it('formats Joe Fight as the approved six-of-ten rating', () => {
    expect(formatDanceabilityRating(1.1535)).toBe('★★★ 6/10');
    expect(formatDanceabilityRating(null)).toBe('—');
  });
});
