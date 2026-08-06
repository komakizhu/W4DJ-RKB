import { describe, expect, it } from 'vitest';
import {
  TRACK_ANALYSIS_VERSION,
  filenameIdentity,
  resolveTrackMetadata,
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
    expect(TRACK_ANALYSIS_VERSION).toBe('0.1.5');
  });
});
