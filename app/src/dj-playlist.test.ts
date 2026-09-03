import { describe, expect, it } from 'vitest';
import {
  NETEASE_QR_MAX_BYTES,
  NETEASE_QR_MAX_TRACKS,
  NeteaseQrPaginationError,
  buildNeteaseImportText,
  splitNeteaseQrPages,
  type ImportedDjPlaylistTrack,
} from './dj-playlist';

function track(position: number, line = `Song ${position} - Artist ${position}`): ImportedDjPlaylistTrack {
  return {
    position,
    title: `Song ${position}`,
    artistDisplay: `Artist ${position}`,
    dedupeKey: `key-${position}`,
    neteaseImportLine: line,
  };
}

describe('DJ playlist NetEase text', () => {
  it('joins backend-provided lines exactly without a trailing newline', () => {
    expect(buildNeteaseImportText([track(2, '二 - 乙'), track(1, '一 - 甲')])).toBe('二 - 乙\n一 - 甲');
  });

  it('splits at complete lines by track and UTF-8 byte limits', () => {
    const tracks = Array.from({ length: 41 }, (_, index) => track(index + 1));
    const pages = splitNeteaseQrPages(tracks);
    expect(pages).toHaveLength(2);
    expect(pages[0].trackCount).toBe(NETEASE_QR_MAX_TRACKS);
    expect(pages[0].lastPosition).toBe(40);
    expect(pages[1].firstPosition).toBe(41);
    for (const page of pages) {
      expect(page.byteLength).toBeLessThanOrEqual(NETEASE_QR_MAX_BYTES);
      expect(page.trackCount).toBeLessThanOrEqual(NETEASE_QR_MAX_TRACKS);
      expect(new TextEncoder().encode(page.text).byteLength).toBe(page.byteLength);
    }
  });

  it('keeps a 100-track fixture stable across multiple navigable pages', () => {
    const tracks = Array.from({ length: 100 }, (_, index) => track(index + 1));
    const pages = splitNeteaseQrPages(tracks);

    expect(pages.length).toBeGreaterThan(1);
    expect(pages.flatMap((page) => page.text.split('\n'))).toEqual(
      tracks.map((item) => item.neteaseImportLine),
    );
    expect(pages.map((page) => page.index)).toEqual(
      pages.map((_, index) => index),
    );
    expect(pages.every((page) => page.trackCount <= NETEASE_QR_MAX_TRACKS
      && page.byteLength <= NETEASE_QR_MAX_BYTES)).toBe(true);
  });

  it('counts Chinese bytes and returns empty pages for an empty playlist', () => {
    expect(splitNeteaseQrPages([])).toEqual([]);
    const pages = splitNeteaseQrPages([track(1, '中文歌曲 - 艺术家')], { maxBytes: 30 });
    expect(pages[0].byteLength).toBe(new TextEncoder().encode('中文歌曲 - 艺术家').byteLength);
  });

  it('rejects a single line that cannot fit on a page', () => {
    expect(() => splitNeteaseQrPages([track(1, 'x'.repeat(20))], { maxBytes: 10 })).toThrow(NeteaseQrPaginationError);
  });
});
