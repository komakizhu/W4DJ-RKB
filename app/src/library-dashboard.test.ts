import { describe, expect, it } from 'vitest';
import { renderLibraryDashboard, saveLibraryColumnOrder, toggleLibraryColumn, type LibraryDashboardState } from './library-dashboard';

const state: LibraryDashboardState = {
  visible: true,
  busy: false,
  status: {
    catalogPath: '/tmp/library-dashboard.sqlite3',
    trackCount: 1,
    netease: {
      databasePath: '/tmp/sqlite_storage.sqlite3',
      musicFolder: '/music/网易云音乐',
      recordCount: 1,
      localFileCount: 1,
    },
  },
  query: { text: '', filters: [], filterLogic: 'and', sorts: [], limit: 100, offset: 0 },
  page: {
    total: 1,
    limit: 100,
    offset: 0,
    items: [{
      trackKey: 'netease:28712318',
      neteaseTrackId: '28712318',
      title: 'FRAGILE',
      artists: '山下達郎',
      album: 'COZY',
      neteaseGenre: 'J-Pop',
      essentiaGenre: 'City Pop',
      coverPath: null,
      coverAvailable: false,
      localStatus: 'available',
      effectiveDurationSeconds: 180,
      durationSource: 'measured',
      effectiveFormat: 'mp3',
      effectiveBitrateBps: 320000,
      effectiveSizeBytes: 1024,
      bpm: 110,
      musicalKey: 'F#',
      scale: 'minor',
      integratedLoudnessLufs: -9,
      energy: 0.8,
      danceability: 0.7,
      moodJson: '[]',
      instrumentJson: '[]',
      dropLoudnessLufs: null,
      lyricPlainText: '',
      lyricTranslatedText: '',
      lyricRomanizedText: '',
      lyricLrcText: '',
      lyricLanguage: '',
      lyricSyncType: 'none',
      lyricSource: '',
      updatedAtMs: 0,
    }],
  },
  detail: null,
  error: null,
};

describe('library dashboard', () => {
  it('renders a searchable paged fact table without null values', () => {
    const html = renderLibraryDashboard(state, 'zh');
    expect(html).toContain('歌曲库');
    expect(html).toContain('FRAGILE');
    expect(html).toContain('山下達郎');
    expect(html).toContain('J-Pop');
    expect(html).toContain('响度');
    expect(html).toContain('能量');
    expect(html).toContain('歌词');
    expect(html).toContain('最后更新');
    expect(html).not.toContain('null');
    expect(html).not.toContain('undefined');
    expect(html).toContain('library-search');
  });

  it('renders the detail drawer and separates NetEase and Essentia genre', () => {
    const html = renderLibraryDashboard({
      ...state,
      detail: {
        ...state.page!.items[0],
        lyricPlainText: '[00:01.00]hello world',
        lyricTranslatedText: '你好世界',
        lyricRomanizedText: 'ni hao shi jie',
        lyricLrcText: '[00:01.00]hello world',
      },
      lyricsTab: 'translated',
      sourceRecords: [{
        trackKey: 'netease:28712318',
        sourceTable: 'web_track',
        sourcePrimaryKey: '28712318',
        sourceVersion: null,
        rawJson: '{"title":"FRAGILE"}',
        importedAtMs: 1,
      }],
    }, 'zh');
    expect(html).toContain('library-detail');
    expect(html).toContain('网易云 Genre');
    expect(html).toContain('Essentia Genre');
    expect(html).toContain('你好世界');
    expect(html).toContain('library-lyrics-tab');
    expect(html).toContain('library-copy-lyrics');
    expect(html).toContain('library-download-lyrics');
    expect(html).toContain('关闭');
    expect(html).toContain('网易云原始信息');
    expect(html).toContain('web_track');
    expect(html).toContain('查看完整 JSON');
  });

  it('renders structured filters, sortable headers and configurable columns', () => {
    localStorage.clear();
    saveLibraryColumnOrder(['cover', 'title', 'artists']);
    toggleLibraryColumn('album');
    const html = renderLibraryDashboard(state, 'zh');
    expect(html).toContain('library-filter-field');
    expect(html).toContain('library-filter-operator');
    expect(html).toContain('data-action="library-sort"');
    expect(html).toContain('data-library-column="album"');
  });

  it('renders an actionable empty state when discovery is unavailable', () => {
    const html = renderLibraryDashboard({
      ...state,
      status: { ...state.status!, trackCount: 0, netease: { ...state.status!.netease, databasePath: null } },
      page: null,
    }, 'zh');
    expect(html).toContain('找不到网易云本地数据库');
    expect(html).toContain('更新歌曲库');
  });
});
