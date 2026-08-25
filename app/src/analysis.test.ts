import { describe, expect, it } from 'vitest';
import {
  TRACK_ANALYSIS_VERSION,
  filterHighLevelLabels,
  filenameIdentity,
  resolveTrackMetadata,
  selectDropBeatWindow,
} from './analysis';

describe('analysis filename identity', () => {
  it('uses the existing Artist - Song convention for ordinary files', () => {
    expect(filenameIdentity('/music/Artist - Song.mp3')).toEqual({
      title: 'Song',
      artist: 'Artist',
      album: '',
    });
  });

  it('follows the selected NetEase filename order for NCM files', () => {
    expect(filenameIdentity('/music/网易云/Song - Artist.ncm', 'title_artist')).toMatchObject({
      title: 'Song',
      artist: 'Artist',
    });
    expect(filenameIdentity('/music/网易云/Artist - Song.ncm', 'artist_title')).toMatchObject({
      title: 'Song',
      artist: 'Artist',
    });
    expect(filenameIdentity('/music/网易云/Song.ncm', 'title_only')).toMatchObject({
      title: 'Song',
      artist: '',
    });
  });

  it('repairs metadata that is the exact reverse of the filename identity', () => {
    expect(resolveTrackMetadata('/music/Artist - Song.mp3', {
      title: 'Artist',
      artist: 'Song',
      album: 'Album',
    })).toEqual({
      title: 'Song',
      artist: 'Artist',
      album: 'Album',
    });
  });

  it('keeps valid metadata while filling missing fields from the filename', () => {
    expect(resolveTrackMetadata('/music/Artist - Song.mp3', {
      title: 'Tagged title',
      artist: '',
      album: '',
    })).toEqual({
      title: 'Tagged title',
      artist: 'Artist',
      album: '',
    });
  });

  it('bumps the cache schema when the fingerprinted identity changes', () => {
    expect(TRACK_ANALYSIS_VERSION).toBe('0.2.0');
  });
});

describe('Drop loudness selection', () => {
  it('selects the loudest contiguous 32-beat window after head and tail exclusion', () => {
    const beats = Array.from({ length: 80 }, (_, index) => index);
    const loudness = beats.map((_, index) => (index >= 30 && index < 62 ? -3 : -12));
    expect(selectDropBeatWindow(beats, loudness, 80)).toEqual({
      startIndex: 30,
      endIndex: 61,
      averageLoudness: -3,
    });
  });

  it('skips when the song has fewer than 32 eligible beats', () => {
    expect(selectDropBeatWindow([0, 1, 2], [-1, -2, -3], 3)).toBeNull();
  });

  it('keeps beat positions paired with their matching loudness values', () => {
    expect(selectDropBeatWindow(
      [0, 1, 2, Number.NaN, 3],
      [-10, -10, -10, 100, -2],
      4,
      2,
    )).toEqual({
      startIndex: 2,
      endIndex: 4,
      averageLoudness: -6,
    });
  });
});

describe('Essentia high-level label filtering', () => {
  it('keeps multiple positive labels and filters negative or weak labels', () => {
    expect(filterHighLevelLabels([
      { label: 'aggressive', confidence: 0.9 },
      { label: 'happy', confidence: 0.8 },
      { label: 'non_sad', confidence: 0.99 },
      { label: 'relaxed', confidence: 0.74 },
    ])).toEqual({
      accepted: [
        { label: 'aggressive', confidence: 0.9 },
        { label: 'happy', confidence: 0.8 },
      ],
      filtered: [
        { label: 'non_sad', confidence: 0.99, reason: 'negative_label' },
        { label: 'relaxed', confidence: 0.74, reason: 'below_threshold' },
      ],
    });
  });
});
