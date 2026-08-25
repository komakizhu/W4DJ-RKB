import { describe, expect, it } from 'vitest';
import { aggregateDiscogsHead, discogsEffnetHeadContracts } from './discogs-effnet';

describe('Discogs-EffNet head aggregation', () => {
  it('averages multilabel frames and keeps thresholded labels plus raw scores', () => {
    const contract = discogsEffnetHeadContracts.find((candidate) => candidate.id === 'moodTheme')!;
    const result = aggregateDiscogsHead(
      contract,
      ['dark', 'bright', 'warm'],
      [[0.8, 0.2, 0.4], [0.6, 0.4, 0.2]],
      2,
      'discogs-test',
    );
    expect(result.status).toBe('completed');
    expect(result.labels).toEqual([
      { label: 'dark', confidence: 0.7 },
      { label: 'warm', confidence: 0.30000000000000004 },
    ].filter((entry) => entry.confidence >= 0.35));
    expect(result.scores).toMatchObject({ dark: 0.7, bright: 0.30000000000000004, warm: 0.30000000000000004 });
    expect(result.threshold).toBe(0.35);
  });

  it('selects the strongest multiclass result and records its confidence', () => {
    const contract = discogsEffnetHeadContracts.find((candidate) => candidate.id === 'danceability')!;
    const result = aggregateDiscogsHead(
      contract,
      ['danceable', 'not_danceable'],
      [[0.3, 0.7], [0.8, 0.2]],
      2,
      'discogs-test',
    );
    expect(result.selectedClass).toBe('danceable');
    expect(result.selectedConfidence).toBeCloseTo(0.55);
    expect(result.labels).toEqual([{ label: 'danceable', confidence: 0.55 }]);
  });

  it('omits unavailable raw scores instead of emitting JSON null', () => {
    const contract = discogsEffnetHeadContracts.find((candidate) => candidate.id === 'danceability')!;
    const result = aggregateDiscogsHead(contract, ['danceable', 'not_danceable'], [], 0, 'discogs-test');
    expect(result.scores).toEqual({});
    expect(JSON.stringify(result)).not.toContain('null');
  });
});
