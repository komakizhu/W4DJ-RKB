import { describe, expect, it } from 'vitest';
import {
  MODEL_IDS,
  cardOrderForTrack,
  createEvaluationSession,
  exportEvaluationCsv,
  matchRelativeAudioFiles,
  scoreSelection,
  shuffleWithSeed,
  summarizeSession,
} from './evaluator.js';

describe('emotion evaluation pure logic', () => {
  it('reproduces seeded song and card order', () => {
    const values = ['a', 'b', 'c', 'd'];
    expect(shuffleWithSeed(values, 42)).toEqual(shuffleWithSeed(values, 42));
    expect(shuffleWithSeed(values, 42)).not.toEqual(shuffleWithSeed(values, 43));
    expect(cardOrderForTrack(42, 'track-1')).toHaveLength(4);
    expect(new Set(cardOrderForTrack(42, 'track-1'))).toEqual(new Set(MODEL_IDS));
  });

  it('matches directory-relative paths without confusing duplicate names', () => {
    const first = { name: 'song.wav', relativePath: 'test/Album/song.wav' };
    const second = { name: 'song.wav', relativePath: 'test/Other/song.wav' };
    const matched = matchRelativeAudioFiles([first, second], [
      'Album/song.wav',
      'Other/song.wav',
    ]);
    expect(matched.get('Album/song.wav')).toBe(first);
    expect(matched.get('Other/song.wav')).toBe(second);
  });

  it('scores unique and tied winners while excluding none', () => {
    const availableModelIds = [...MODEL_IDS];
    expect(scoreSelection({ availableModelIds, winnerIds: ['emomusic'] }).points.emomusic).toBe(1);
    expect(scoreSelection({ availableModelIds, winnerIds: ['emomusic', 'muse'] }).points.emomusic).toBe(0.5);
    expect(scoreSelection({ availableModelIds, winnerIds: ['emomusic', 'muse', 'mirex'] }).points.muse)
      .toBeCloseTo(1 / 3);
    expect(scoreSelection({ availableModelIds, winnerIds: 'none' }).validSample).toBe(false);
  });

  it('excludes unavailable models from their denominator', () => {
    const result = scoreSelection({
      availableModelIds: ['legacyMood', 'emomusic'],
      winnerIds: ['emomusic'],
    });
    expect(result.validSample).toBe(true);
    expect(result.denominator.muse).toBe(0);
    expect(result.points.emomusic).toBe(1);
  });

  it('excludes missing or unplayable audio from the win-rate denominator', () => {
    const result = scoreSelection({
      audioMatched: false,
      availableModelIds: [...MODEL_IDS],
      winnerIds: ['emomusic'],
    });
    expect(result.validSample).toBe(false);
    expect(result.reason).toBe('audio_unavailable');
    expect(result.denominator.emomusic).toBe(0);
  });

  it('summarizes answers and exports a flat CSV', () => {
    const manifest = {
      schemaVersion: 1,
      sessionId: 'session-1',
      seed: 42,
      sampleSize: 1,
      clipPolicy: 'peak-energy-10s-with-drop-preference',
      tracks: [{
        trackId: 'track-1',
        title: 'Song, One',
        artist: 'Artist',
        album: '',
        relativePath: 'Album/song.wav',
        durationSeconds: 20,
        clipStartSeconds: 2,
        clipDurationSeconds: 10,
        clipSelection: 'peakEnergy',
        legacyMood: { status: 'completed', labels: [] },
        emomusic: { status: 'completed', valence: 7, arousal: 6 },
        muse: { status: 'completed', valence: 6, arousal: 5 },
        mirex: { status: 'completed', labels: [] },
      }],
    };
    const session = createEvaluationSession(manifest);
    expect(session.trackOrder).toEqual(['track-1']);
    session.answers['track-1'] = {
      humanLabel: 'bright',
      availableModelIds: [...MODEL_IDS],
      winnerIds: ['emomusic'],
    };
    expect(summarizeSession(session).winRates.emomusic).toBe(1);
    expect(exportEvaluationCsv(session)).toContain('Song, One');
  });
});
