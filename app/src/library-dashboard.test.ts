import { describe, expect, it } from 'vitest';
import { DANCEABILITY_MAX, formatRatingStars, libraryColumnIds, libraryOperatorsForField, loadLibraryColumnSettings, renderLibraryDashboard, saveLibraryColumnOrder, saveLibraryColumnWidth, toggleLibraryColumn, type LibraryDashboardState } from './library-dashboard';
import { formatDanceabilityRating } from './danceability-rating';
import { formatEnergyRating } from './energy-rating';

const state: LibraryDashboardState = {
  visible: true,
  busy: false,
  status: {
    catalogPath: '/tmp/library-dashboard.sqlite3',
    trackCount: 1,
    manualDatabasePath: '/tmp/sqlite_storage.sqlite3',
    refresh: {
      refreshId: '',
      status: 'idle',
      stage: 'committing',
      processed: 0,
      total: null,
      currentItem: '',
      message: '',
      summary: null,
      error: null,
    },
    databaseWarning: null,
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
      danceability: 1.1535,
      moodJson: '[]',
      instrumentJson: '[{"label":"instrumental","confidence":0.91}]',
      styleJson: '[{"label":"acoustic","confidence":0.84},{"label":"electronic","confidence":0.8},{"label":"dance","confidence":0.7}]',
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
  it('formats normalized analysis scores and Danceability ratings independently', () => {
    expect(formatRatingStars(0.73)).toBe('★★★☆ 73%');
    expect(formatRatingStars(0.86)).toBe('★★★★☆ 86%');
    expect(formatRatingStars(0.005)).toBe('— 1%');
    expect(formatRatingStars(null)).toBe('—');
    expect(formatRatingStars(DANCEABILITY_MAX, DANCEABILITY_MAX)).toBe('★★★★★ 100%');
    expect(formatDanceabilityRating(1.1535)).toBe('★★★ 6/10');
    expect(formatDanceabilityRating(null)).toBe('—');
    expect(formatEnergyRating(0.8)).toBe('★★★★★ 10/10');
    expect(formatEnergyRating(null)).toBe('—');
  });

  it('renders a searchable paged fact table without null values', () => {
    const html = renderLibraryDashboard(state, 'zh');
    expect(html).toContain('歌曲库');
    expect(html).toContain('FRAGILE');
    expect(html).toContain('山下達郎');
    expect(html).not.toContain('data-library-column-header="neteaseGenre"');
    expect(html).toContain('响度');
    expect(html).toContain('能量');
    expect(html).toContain('可舞性');
    expect(html).toContain('可舞性（10级）');
    expect(html).toContain('data-library-column-header="instrument"');
    expect(html).toContain('data-library-column-header="acousticElectronic"');
    expect(html).toContain('器乐');
    expect(html).toContain('原生乐器');
    expect(html).toContain('电子');
    expect(html).toContain('value="danceability"');
    expect(html).not.toContain('★★★★ 80%');
    expect(html).toContain('★★★ 6/10');
    expect(html).toContain('可舞性原始值');
    expect(html).toContain('★★★★★ 10/10');
    expect(html).toContain('Essentia RMS² raw: 0.8');
    expect(html).toContain('value="energy"');
    expect(html).toContain('歌词');
    expect(html).toContain('最后更新');
    expect(html).not.toContain('null');
    expect(html).not.toContain('undefined');
    expect(html).toContain('library-search');
    expect(html).toContain('>搜索</button>');
    expect(html).not.toContain('>更新歌曲库</button>');
    expect(html).toContain('library-filter-label');
    expect(html).toContain('data-action="search-library"');
    expect(html).toContain('class="library-invalid-confirm"');
    expect(html).not.toContain('library-column-bar');
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
    expect(html).toContain('LUFS / Energy');
    expect(html).toContain('Essentia RMS² raw: 0.8');
    expect(html).toContain('你好世界');
    expect(html).toContain('library-lyrics-tab');
    expect(html).toContain('library-copy-lyrics');
    expect(html).toContain('library-download-lyrics');
    expect(html).toContain('关闭');
    expect(html).toContain('网易云原始信息');
    expect(html).toContain('web_track');
    expect(html).toContain('查看完整 JSON');
    expect(html).toContain('可能包含本机路径');
  });

  it('renders structured filters, sortable headers and configurable columns', () => {
    localStorage.clear();
    saveLibraryColumnOrder(['cover', 'title', 'artists']);
    toggleLibraryColumn('album');
    const html = renderLibraryDashboard(state, 'zh');
    expect(html).toContain('library-filter-field');
    expect(html).toContain('library-filter-operator');
    expect(html).toContain('data-action="library-sort"');
    expect(html).not.toContain('library-column-bar');
  });

  it('omits legacy NetEase and release metadata columns from the table', () => {
    localStorage.clear();
    const html = renderLibraryDashboard({ ...state, detail: null }, 'zh');
    expect(html).not.toContain('data-library-column-header="neteaseGenre"');
    expect(html).not.toContain('data-library-column-header="copyright"');
    expect(html).not.toContain('data-library-column-header="publishDate"');
    expect(libraryColumnIds()).not.toContain('neteaseGenre');
    expect(libraryColumnIds()).not.toContain('copyright');
    expect(libraryColumnIds()).not.toContain('publishDate');
  });

  it('renders an empty state when no analysis has been saved', () => {
    const html = renderLibraryDashboard({
      ...state,
      status: { ...state.status!, trackCount: 0, analyzedTrackCount: 0 },
      page: null,
    }, 'zh');
    expect(html).toContain('还没有完成分析的歌曲');
    expect(html).toContain('W4DJ 歌曲库');
    expect(html).toContain('data-action="search-library"');
  });

  it('keeps database and analysis maintenance controls out of the library view', () => {
    const html = renderLibraryDashboard({
      ...state,
      busy: true,
      status: {
        ...state.status!,
        databaseWarning: '保存的数据库不可用，已尝试自动定位',
        refresh: {
          refreshId: 'library-1',
          status: 'running',
          stage: 'probingLocalFiles',
          processed: 4,
          total: 10,
          currentItem: 'Song.mp3',
          message: '正在探测本地文件',
          summary: null,
          error: null,
        },
      },
    }, 'zh');
    expect(html).not.toContain('library-refresh-progress');
    expect(html).not.toContain('data-action="cancel-library-refresh"');
    expect(html).not.toContain('选择数据库');
    expect(html).not.toContain('分析本地歌曲');
    expect(html).not.toContain('data-action="select-library-database"');
    expect(html).not.toContain('data-action="analyze-library"');
    expect(html).toContain('data-action="search-library"');
    expect(html).not.toContain('恢复自动定位');
    expect(html).not.toContain('清除歌曲库缓存');
    expect(html).not.toContain('保存的数据库不可用');
    expect(html).not.toContain('library-column-bar');
  });

  it('persists column widths and limits operators by field type', () => {
    localStorage.clear();
    saveLibraryColumnWidth('title', 510);
    expect(loadLibraryColumnSettings().widths.title).toBe(420);
    expect(libraryOperatorsForField('bpm')).toContain('between');
    expect(libraryOperatorsForField('cover_available')).toEqual(['is_true', 'is_false']);
    expect(libraryOperatorsForField('title')).toContain('contains');
  });

  it('renders the persisted column order and width into the table structure', () => {
    localStorage.clear();
    saveLibraryColumnOrder(['cover', 'essentiaGenre', 'title']);
    saveLibraryColumnWidth('essentiaGenre', 240);

    expect(libraryColumnIds().slice(0, 3)).toEqual(['cover', 'essentiaGenre', 'title']);
    const html = renderLibraryDashboard(state, 'zh');
    expect(html).toContain('<col data-library-column="essentiaGenre" style="width:240px"');
    expect(html.indexOf('data-library-column-header="essentiaGenre"'))
      .toBeLessThan(html.indexOf('data-library-column-header="title"'));
  });

  it('shows sort direction and priority for compound sorts', () => {
    const html = renderLibraryDashboard({
      ...state,
      query: {
        ...state.query,
        sorts: [{ field: 'title', direction: 'asc' }, { field: 'artists', direction: 'desc' }],
      },
    }, 'zh');
    expect(html).toContain('▲<sup>1</sup>');
    expect(html).toContain('▼<sup>2</sup>');
  });

  it('renders the confirmed invalid-file cleanup and row context actions', () => {
    const unconfirmed = renderLibraryDashboard(state, 'zh');
    expect(unconfirmed).toContain('data-action="clear-invalid-library" disabled');
    const html = renderLibraryDashboard({
      ...state,
      confirmClearInvalid: true,
      contextMenu: { trackKey: 'netease:28712318', x: 120, y: 180 },
    }, 'zh');
    expect(html).toContain('data-action="library-confirm-clear-invalid"');
    expect(html).toContain('data-action="clear-invalid-library"');
    expect(html).toContain('清除所有失效文件');
    expect(html).toContain('data-role="library-context-menu"');
    expect(html).toContain('重新定位文件');
    expect(html).toContain('移除记录');
  });

  it('renders independent output-library statistics and invalid scan controls', () => {
    const html = renderLibraryDashboard({
      ...state,
      status: {
        ...state.status!,
        totalTrackCount: 9,
        availableTrackCount: 8,
        invalidTrackCount: 1,
        notAnalyzedCount: 3,
        analysisFailedCount: 1,
        analysisCompletedCount: 5,
        invalidScan: {
          scanId: 'invalid-1',
          status: 'running',
          processed: 4,
          total: 9,
          currentItem: 'song.mp3',
          message: '正在检查已登记歌曲',
          error: null,
        },
      },
    }, 'zh');
    expect(html).toContain('总歌曲 9');
    expect(html).toContain('可用 8');
    expect(html).toContain('失效 1');
    expect(html).toContain('data-action="cancel-invalid-scan"');
    expect(html).toContain('4/9');
    expect(html).toContain('song.mp3');
  });
});
