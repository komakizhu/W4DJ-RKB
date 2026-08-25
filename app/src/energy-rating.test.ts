import { describe, expect, it } from 'vitest';
import {
  ENERGY_LEVEL_THRESHOLDS,
  energyLevel,
  formatEnergyRating,
} from './energy-rating';

describe('energy ten-level rating', () => {
  it('maps representative RMS-squared values to the approved levels', () => {
    expect(energyLevel(0.041)).toBe(1);
    expect(energyLevel(0.05)).toBe(2);
    expect(energyLevel(0.10)).toBe(3);
    expect(energyLevel(0.13)).toBe(4);
    expect(energyLevel(0.16)).toBe(5);
    expect(energyLevel(0.20)).toBe(6);
    expect(energyLevel(0.23)).toBe(7);
    expect(energyLevel(0.27)).toBe(8);
    expect(energyLevel(0.32)).toBe(9);
    expect(energyLevel(0.40)).toBe(10);
  });

  it('puts an exact threshold into the higher level', () => {
    ENERGY_LEVEL_THRESHOLDS.forEach((threshold, index) => {
      expect(energyLevel(threshold * (1 - 1e-9))).toBe(index + 1);
      expect(energyLevel(threshold)).toBe(index + 2);
    });
  });

  it('handles missing and finite extreme values', () => {
    expect(energyLevel(null)).toBeNull();
    expect(energyLevel(Number.NaN)).toBeNull();
    expect(energyLevel(Number.POSITIVE_INFINITY)).toBeNull();
    expect(energyLevel(-1)).toBe(1);
    expect(energyLevel(100)).toBe(10);
  });

  it('formats the approved six-of-ten example', () => {
    expect(formatEnergyRating(0.208673633)).toBe('★★★ 6/10');
    expect(formatEnergyRating(null)).toBe('—');
  });
});
