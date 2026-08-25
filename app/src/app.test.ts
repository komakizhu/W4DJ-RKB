import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  bindApp,
  canReuseTrackAnalysis,
  pickSourceWithPlatformDialog,
  humanizeError,
  resolveDropTargetAt,
  renderApp,
  type AppHistoryEntry,
  type AppLosslessFormat,
  type AppPreview,
  type AppScanProgress,
  type AppServices,
  type DjPlaylistUiState,
  type AppSyncSlotViewState,
  type AppViewState,
  type DesktopState,
  type DesktopSyncSlotState,
  type NeteaseDiscoveryProgress,
  type SyncSlotIndex,
} from './app';
import type { LibraryPage, LibraryRefreshProgress, LibraryStatus, LibraryTrack } from './library-dashboard';

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  localStorage.setItem('w4dj_onboarding_seen', '1');
});

const makeDesktopSlot = (
  overrides: Partial<DesktopSyncSlotState> = {},
): DesktopSyncSlotState => ({
  source_directory: '/music/in',
  destination_directory: '/music/out',
  status: 'idle',
  progress_total: 0,
  progress_completed: 0,
  new_tracks: 0,
  skipped_tracks: 0,
  existing_tracks: 0,
  error_tracks: 0,
  estimated_output_bytes: null,
  failed_files: [],
  current_file: '',
  logs: ['Ready'],
  active_concurrency_limit: null,
  ...overrides,
});

const makeDesktopState = (overrides: Partial<DesktopState> = {}): DesktopState => ({
  slots: [
    makeDesktopSlot({ source_directory: '/music/in-1', destination_directory: '/music/out-1' }),
    makeDesktopSlot({ source_directory: '/music/in-2', destination_directory: '/music/out-2' }),
  ],
  mode: 'compat',
  lossless_format: null,
  conversion_mode: 'scan_then_convert',
  enhanced_mode: false,
  conflict_strategy: 'skip',
  filename_rule: 'title_artist',
  netease_filename_format: 'title_artist',
  concurrency_limit: 2,
  ...overrides,
});

const makeDesktopStateWithSlot = (
  slotIndex: SyncSlotIndex,
  slotOverrides: Partial<DesktopSyncSlotState>,
  overrides: Partial<DesktopState> = {},
): DesktopState => {
  const state = makeDesktopState(overrides);
  const slots: [DesktopSyncSlotState, DesktopSyncSlotState] = [
    { ...state.slots[0] },
    { ...state.slots[1] },
  ];
  slots[slotIndex] = { ...slots[slotIndex], ...slotOverrides };
  return { ...state, slots };
};

const makeViewSlot = (overrides: Partial<AppSyncSlotViewState> = {}): AppSyncSlotViewState => ({
  sourceDirectory: '/music/in',
  destinationDirectory: '/music/out',
  status: 'idle',
  progressTotal: 0,
  progressCompleted: 0,
  newTracks: 0,
  skippedTracks: 0,
  errorTracks: 0,
  progressText: '待命',
  currentFile: '',
  logs: ['Ready'],
  activeConcurrencyLimit: null,
  ...overrides,
});

const makeViewState = (overrides: Partial<AppViewState> = {}): AppViewState => ({
  slots: [
    makeViewSlot({ sourceDirectory: '/music/in-1', destinationDirectory: '/music/out-1' }),
    makeViewSlot({ sourceDirectory: '/music/in-2', destinationDirectory: '/music/out-2' }),
  ],
  mode: 'compat',
  losslessFormat: null,
  conversionMode: 'scan_then_convert',
  enhancedMode: false,
  conflictStrategy: 'skip',
  filenameRule: 'title_artist',
  neteaseFilenameFormat: 'title_artist',
  lang: 'zh',
  theme: 'light',
  ...overrides,
});

const makeViewStateWithSlot = (
  slotIndex: SyncSlotIndex,
  slotOverrides: Partial<AppSyncSlotViewState>,
  overrides: Partial<AppViewState> = {},
): AppViewState => {
  const state = makeViewState(overrides);
  const slots: [AppSyncSlotViewState, AppSyncSlotViewState] = [
    { ...state.slots[0] },
    { ...state.slots[1] },
  ];
  slots[slotIndex] = { ...slots[slotIndex], ...slotOverrides };
  return { ...state, slots };
};

const makePreview = (slotIndex: 0 | 1 = 0): AppPreview => ({
  slot_index: slotIndex,
  mode: 'compat',
  lossless_format: null,
  conflict_strategy: 'skip',
  filename_rule: 'title_artist',
  retry_of: null,
  preview: {
    source_directory: `/music/in-${slotIndex + 1}`,
    destination_directory: `/music/out-${slotIndex + 1}`,
    new_count: 2,
    existing_count: 1,
    skipped_count: 1,
    error_count: 0,
    estimated_output_bytes: 2048,
    candidates: [
      {
        name: 'Song',
        source_path: `/music/in-${slotIndex + 1}/Song.mp3`,
        destination_path: `/music/out-${slotIndex + 1}/Song.mp3`,
        source_size_bytes: 1024,
        estimated_output_bytes: 1024,
        operation: 'convert',
      },
    ],
    skipped: [],
    errors: [],
    warnings: [],
    available_space_bytes: 10_000,
    disk_space_sufficient: true,
  },
});

const makePreviewResponse = (): AppPreview[] => [makePreview(0), makePreview(1)];

const makeHistoryEntry = (overrides: Partial<AppHistoryEntry> = {}): AppHistoryEntry => ({
  id: 'history-1',
  batch_id: 'batch-1',
  slot_index: 0,
  started_at: '2026-07-14 12:00',
  finished_at: '2026-07-14 12:01',
  duration_seconds: 60,
  source_directory: '/music/in-1',
  destination_directory: '/music/out-1',
  mode: 'compat',
  lossless_format: null,
  new_count: 2,
  existing_count: 0,
  skipped_count: 0,
  error_count: 1,
  completed_count: 1,
  failed_count: 1,
  failed_files: [
    {
      name: 'Song',
      source_path: '/music/in-1/Song.flac',
      destination_path: '/music/out-1/Song.mp3',
      message: 'FFmpeg failed',
      category: 'ffmpeg',
    },
  ],
  pending_files: [],
  logs: ['Scanning source: /music/in-1'],
  status: 'partial',
  retry_of: null,
  conflict_strategy: 'skip',
  filename_rule: 'title_artist',
  report_path: null,
  ...overrides,
});

const makeMockAnalysisWorker = () => ({
  start: vi.fn().mockResolvedValue(undefined),
  analyze: vi.fn().mockRejectedValue(new Error('test analysis worker fixture')),
  terminate: vi.fn(),
});

const makeCompleteHighLevelAnalysis = () => ({
  status: 'completed' as const,
  modelVersion: 'essentia-v2',
  emotionCandidates: {
    emomusic: { model: 'emomusic' as const, status: 'completed' as const, valence: 0.5, arousal: 0.5 },
    muse: { model: 'muse' as const, status: 'completed' as const, valence: 0.5, arousal: 0.5 },
  },
  moodClusterStatus: 'completed' as const,
  discogsEffnet: {
    embeddingModel: 'discogs-effnet-bs64-1' as const,
    embeddingDimensions: 1280,
    inputShape: [64, 128, 96] as [number, number, number],
    heads: Object.fromEntries([
      'moodTheme', 'approachability', 'instrumentation', 'timbre', 'danceability',
    ].map((model) => [model, {
      model,
      status: 'completed' as const,
      version: 'discogs-v1',
      labels: [],
      scores: {},
      frameCount: 1,
      selectedClass: model === 'danceability' ? 'high' : undefined,
      selectedConfidence: 0.8,
    }])),
  },
});

const makeMockServices = (overrides: Partial<AppServices> = {}): AppServices => ({
  loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState()),
  pickDirectory: vi.fn().mockResolvedValue(null),
  pickSource: vi.fn().mockResolvedValue(null),
  selectSourceDirectory: vi.fn().mockResolvedValue(makeDesktopState()),
  selectDestinationDirectory: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseMode: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseLosslessFormat: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseConversionMode: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseEnhancedMode: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseConflictStrategy: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseFilenameRule: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseConcurrencyLimit: vi.fn().mockResolvedValue(makeDesktopState()),
  previewAllSync: vi.fn().mockResolvedValue(makePreviewResponse()),
  startScan: vi.fn().mockResolvedValue({
    status: 'completed',
    phase: 'completed',
    processed: 2,
    total: 2,
    current_file: '',
    message: '扫描完成',
  }),
  loadScanState: vi.fn().mockResolvedValue({
    status: 'completed',
    phase: 'completed',
    processed: 2,
    total: 2,
    current_file: '',
    message: '扫描完成',
  }),
  loadScanResult: vi.fn().mockResolvedValue(makePreviewResponse()),
  cancelScan: vi.fn().mockResolvedValue({
    status: 'cancelled',
    phase: 'cancelled',
    processed: 0,
    total: 2,
    current_file: '',
    message: '扫描已取消',
  }),
  clearScanCache: vi.fn().mockResolvedValue(undefined),
  startConfirmedSync: vi.fn().mockResolvedValue(makeDesktopState({
    slots: [
      makeDesktopSlot({ status: 'running', progress_total: 2 }),
      makeDesktopSlot({ status: 'running', progress_total: 2 }),
    ],
  })),
  applyTrackAnalysisResults: vi.fn().mockResolvedValue(makeDesktopState()),
  loadHistory: vi.fn().mockResolvedValue([]),
  retryHistoryFailures: vi.fn().mockResolvedValue(makePreview(0)),
  exportHistoryErrorReport: vi.fn().mockResolvedValue(undefined),
  exportRuntimeSession: vi.fn().mockResolvedValue(undefined),
  deleteHistoryEntry: vi.fn().mockResolvedValue(undefined),
  clearHistory: vi.fn().mockResolvedValue(undefined),
  loadAppInfo: vi.fn().mockResolvedValue({
    version: '3.2.0-beta.3',
    developer: 'komakizhu',
    project_url: 'https://github.com/komakizhu/W4DJ-RKB',
  }),
  openExternalUrl: vi.fn().mockResolvedValue(undefined),
  openDestination: vi.fn().mockResolvedValue(undefined),
  openSource: vi.fn().mockResolvedValue(undefined),
  startAllSync: vi
    .fn()
    .mockResolvedValue(makeDesktopState({
      slots: [
        makeDesktopSlot({ status: 'running', progress_total: 10 }),
        makeDesktopSlot({ status: 'running', progress_total: 8 }),
      ],
    })),
  pauseAllSync: vi.fn().mockResolvedValue(makeDesktopState({
    slots: [
      makeDesktopSlot({ status: 'paused' }),
      makeDesktopSlot({ status: 'paused' }),
    ],
  })),
  cancelSync: vi.fn().mockResolvedValue(makeDesktopState()),
  cancelAllSync: vi.fn().mockResolvedValue(makeDesktopState()),
  listAudioFiles: vi.fn().mockResolvedValue([]),
  readAudioFile: vi.fn().mockResolvedValue([]),
  readTrackMetadata: vi.fn().mockResolvedValue({ title: '', artist: '', album: '' }),
  getAudioFileFingerprint: vi.fn().mockResolvedValue({ sizeBytes: 0, modifiedAt: null }),
  loadTrackAnalyses: vi.fn().mockResolvedValue([]),
  saveTrackAnalyses: vi.fn().mockResolvedValue(0),
  clearTrackAnalyses: vi.fn().mockResolvedValue(undefined),
  createAnalysisWorker: makeMockAnalysisWorker,
  exportRekordboxXml: vi.fn().mockResolvedValue(undefined),
  pickLibraryTrackFile: vi.fn().mockResolvedValue(null),
  relocateLibraryTrack: vi.fn().mockResolvedValue(undefined),
  removeLibraryTrack: vi.fn().mockResolvedValue(true),
  clearInvalidLibraryTracks: vi.fn().mockResolvedValue(0),
  ...overrides,
});

const createDeferred = <T>() => {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });

  return { promise, resolve, reject };
};

describe('renderApp', () => {
  it('renders the .w4dj launcher with source attribution and import/export choices', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      undefined,
      null,
      false,
      null,
      null,
      undefined,
      null,
      null,
      null,
      null,
      null,
      {
        visible: true,
        launcher: true,
        busy: false,
        error: null,
        notice: null,
        playlist: null,
        pages: [],
        pageIndex: 0,
        qrDataUrl: null,
        qrRevision: 0,
        matchBusy: false,
        matchReport: null,
        exportBusy: false,
        dropActive: false,
      },
    );
    expect(root.querySelector('[data-action="import-dj-playlist"]')?.textContent).toContain('导入.w4dj');
    expect(root.querySelector('[data-role="dj-playlist-launcher"]')).not.toBeNull();
    expect(root.querySelector('[data-action="dj-playlist-open-import"]')?.textContent).toContain('导入.w4dj');
    expect(root.querySelector('[data-action="dj-playlist-open-export"]')?.textContent).toContain('导出 m3u8');
    expect(root.querySelector('a[data-action="open-dj-crate-digger-link"]')?.getAttribute('href')).toBe('https://github.com/komakizhu/dj-crate-digger-skill');
    expect(root.textContent).toContain('网易云-我的-三竖点-一键导入外部歌单-文字导入');
  });

  it('opens the launcher before opening the .w4dj picker', async () => {
    const root = document.createElement('div');
    const pickW4djPlaylist = vi.fn().mockResolvedValue(null);
    bindApp(root, makeViewState(), makeMockServices({ pickW4djPlaylist }));
    (root.querySelector('[data-action="import-dj-playlist"]') as HTMLButtonElement).click();
    expect(pickW4djPlaylist).not.toHaveBeenCalled();
    expect(root.querySelector('[data-role="dj-playlist-launcher"]')).not.toBeNull();
    (root.querySelector('[data-action="dj-playlist-open-import"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(pickW4djPlaylist).toHaveBeenCalledTimes(1));
  });

  it('renders the DJ playlist import dialog, QR controls, and whole-window drop overlay', () => {
    const playlistState: DjPlaylistUiState = {
      visible: true,
      busy: false,
      error: null,
      notice: null,
      playlist: {
        playlistId: 'playlist-1',
        formatVersion: 2,
        name: 'Afro House',
        sourcePath: null,
        importedAtMs: null,
        tracks: [{
          position: 1,
          title: 'Anchor Point',
          artistDisplay: 'Ahmed Spins',
          neteaseTrackId: '123456789012345678',
          dedupeKey: 'anchor',
          neteaseImportLine: 'Anchor Point - Ahmed Spins',
        }],
        warnings: [],
      },
      pages: [{ index: 0, total: 1, trackCount: 1, byteLength: 28, firstPosition: 1, lastPosition: 1, text: 'Anchor Point - Ahmed Spins' }],
      pageIndex: 0,
      qrDataUrl: 'data:image/png;base64,abc',
      qrRevision: 1,
      matchBusy: false,
      matchReport: null,
      exportBusy: false,
      dropActive: true,
    };
    const root = renderApp(makeViewState(), null, null, null, [], null, false, null, false, false, false, 0, undefined, null, false, null, null, undefined, null, null, null, null, null, playlistState);
    expect(root.querySelector('[data-action="import-dj-playlist"]')).not.toBeNull();
    expect(root.querySelector('[data-role="dj-playlist-dialog"]')).not.toBeNull();
    expect(root.querySelector('[data-role="dj-playlist-drop-overlay"]')).not.toBeNull();
    expect(root.textContent).toContain('Anchor Point - Ahmed Spins');
    expect(root.textContent).toContain('123456789012345678');
  });
  it('shows a persisted recent-playlist action without exposing its source path', () => {
    const root = renderApp(makeViewState(), null, null, null, [], null, false, null, false, false, false, 0, undefined, null, false, null, null, undefined, null, null, null, null, null, null, [{
        playlistId: 'playlist-1',
        name: 'Afro House Club',
        trackCount: 10,
        warningCount: 0,
        importedAtMs: 1,
        sourcePath: '/private/secret/playlist.w4dj',
      }]);
    expect(root.querySelector('[data-action="open-latest-dj-playlist"]')).not.toBeNull();
    expect(root.textContent).not.toContain('/private/secret/playlist.w4dj');
  });
  it('reloads the newest persisted playlist through the list/load services', async () => {
    const root = document.createElement('div');
    const listImportedDjPlaylists = vi.fn().mockResolvedValue([{
      playlistId: 'playlist-1',
      name: 'Afro House Club',
      trackCount: 1,
      warningCount: 0,
      importedAtMs: 2,
      sourcePath: null,
    }]);
    const loadImportedDjPlaylist = vi.fn().mockResolvedValue({
      playlistId: 'playlist-1',
      formatVersion: 2,
      name: 'Afro House Club',
      sourcePath: null,
      importedAtMs: 2,
      tracks: [{
        position: 1,
        title: 'Anchor Point',
        artistDisplay: 'Ahmed Spins',
        neteaseTrackId: null,
        dedupeKey: 'anchor',
        neteaseImportLine: 'Anchor Point - Ahmed Spins',
      }],
      warnings: [],
    });
    bindApp(root, makeViewState(), makeMockServices({ listImportedDjPlaylists, loadImportedDjPlaylist }));
    await vi.waitFor(() => expect(root.querySelector('[data-action="open-latest-dj-playlist"]')).not.toBeNull());
    (root.querySelector('[data-action="open-latest-dj-playlist"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(loadImportedDjPlaylist).toHaveBeenCalledWith('playlist-1'));
    await vi.waitFor(() => expect(root.querySelector('[data-role="dj-playlist-dialog"]')?.textContent).toContain('Anchor Point'));
    await vi.waitFor(() => expect(root.querySelector('.dj-playlist-qr-image img')).not.toBeNull());
  });
  it('gives a single browser-dropped .w4dj file whole-window precedence', async () => {
    const root = document.createElement('div');
    const importW4djPlaylist = vi.fn().mockResolvedValue({
      playlistId: 'playlist-drop',
      formatVersion: 2,
      name: 'Dropped playlist',
      sourcePath: '/music/drop.w4dj',
      importedAtMs: 3,
      tracks: [{
        position: 1,
        title: 'Dropped Song',
        artistDisplay: 'Dropped Artist',
        neteaseTrackId: null,
        dedupeKey: 'drop',
        neteaseImportLine: 'Dropped Song - Dropped Artist',
      }],
      warnings: [],
    });
    const services = makeMockServices({ importW4djPlaylist });
    bindApp(root, makeViewState(), services);
    const file = new File(['{}'], 'playlist.w4dj', { type: 'application/json' });
    Object.defineProperty(file, 'path', { value: '/music/playlist.w4dj' });
    const dragover = new Event('dragover', { bubbles: true, cancelable: true });
    Object.defineProperty(dragover, 'dataTransfer', { value: { files: [file], getData: vi.fn().mockReturnValue('') } });
    root.dispatchEvent(dragover);
    expect(dragover.defaultPrevented).toBe(true);
    expect(root.querySelector('[data-role="dj-playlist-drop-overlay"]')).not.toBeNull();
    const drop = new Event('drop', { bubbles: true, cancelable: true });
    Object.defineProperty(drop, 'dataTransfer', { value: { files: [file], getData: vi.fn().mockReturnValue('') } });
    root.dispatchEvent(drop);
    await vi.waitFor(() => expect(importW4djPlaylist).toHaveBeenCalledWith('/music/playlist.w4dj'));
    await vi.waitFor(() => expect(root.querySelector('[data-role="dj-playlist-dialog"]')?.textContent).toContain('Dropped Song'));
    await vi.waitFor(() => expect(root.querySelector('.dj-playlist-qr-image img')).not.toBeNull());
  });
  it('renders two independent sync slots and global controls', () => {
    const root = renderApp(makeViewState());

    expect(root.querySelector('h1')?.textContent).toBe('如果我是DJ');
    expect(root.querySelector('[data-role="workbench-rail"]')).not.toBeNull();
    expect(root.querySelector('[data-role="workbench-main"]')).not.toBeNull();
    expect(root.querySelectorAll('[data-role="sync-slot"]')).toHaveLength(2);
    expect(root.querySelector('[data-role="source-picker"][data-slot="0"]')?.textContent).toContain(
      '/music/in-1',
    );
    expect(
      root.querySelector('[data-role="destination-picker"][data-slot="1"]')?.textContent,
    ).toContain('/music/out-2');
    expect(root.querySelector('[data-role="mode-switch"]')).not.toBeNull();
    expect(root.querySelectorAll('[data-action="start-all"]')).toHaveLength(1);
    expect(root.querySelectorAll('[data-action="start"]')).toHaveLength(0);
    expect(root.querySelectorAll('[data-role="log-drawer"]')).toHaveLength(0);
    expect(root.querySelectorAll('.slot-status-strip .progress-copy')).toHaveLength(0);
    expect(root.querySelector('.rail-copy')).toBeNull();
  });

  it('puts the explicit NetEase scan action beside Task 1 source and shows its progress in the slot bar', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      { status: 'idle', completed: 0, total: 0, resultCount: 0, failedCount: 0, message: '' },
      null,
      false,
      null,
      null,
      { version: '', embedding: false, genre: false, mood: false, instrument: false, installing: false },
      null,
      { status: 'running', stage: 'checkingMusicFolder', processed: 3, total: 8, currentItem: 'Song.mp3', message: '正在检查本地歌曲', suggestion: null, error: null },
    );

    expect(root.querySelectorAll('[data-action="scan-local-netease"]')).toHaveLength(1);
    expect(root.querySelector('[data-action="scan-local-netease"]')?.textContent).toContain('扫描本地网易云文件夹');
    const progress = root.querySelector('[data-slot="0"] .progress-fill') as HTMLElement;
    expect(progress.style.width).toBe('38%');
    expect(root.querySelector('[data-slot="0"] .progress-copy')?.textContent).toContain('Song.mp3');
  });

  it('renders manual NetEase database controls only beside Task 1', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      { status: 'idle', completed: 0, total: 0, resultCount: 0, failedCount: 0, message: '' },
      null,
      false,
      null,
      null,
      { version: '', embedding: false, genre: false, mood: false, instrument: false, installing: false },
      null,
      null,
      null,
      false,
      {
        status: {
          manualPath: '/music/sqlite_storage.sqlite3',
          effectivePath: '/music/sqlite_storage.sqlite3',
          source: 'manual',
          loaded: true,
          recordCount: 42,
          warning: null,
        },
        busy: false,
        message: null,
        error: null,
      },
    );

    expect(root.querySelectorAll('[data-action="select-netease-database"]')).toHaveLength(1);
    expect(root.querySelector('[data-slot="1"] [data-action="select-netease-database"]')).toBeNull();
    expect(root.querySelector('[data-action="scan-local-netease"]')).not.toBeNull();
    expect(root.querySelector('[data-action="select-netease-database"]')?.textContent)
      .toContain('sqlite_storage.sqlite3');
    expect(root.querySelector('[data-action="select-netease-database"]')?.textContent)
      .not.toContain('/music/');
    expect(root.querySelector('[data-action="clear-netease-database"]')).not.toBeNull();
  });

  it('puts enhanced analysis progress in the Task 1 slot footer', () => {
    const root = renderApp(
      makeViewState({ enhancedMode: true }),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      {
        slotIndex: 0,
        status: 'running',
        completed: 0,
        total: 5,
        resultCount: 0,
        failedCount: 0,
        message: '正在计算 BPM、Key 和响度',
        currentItem: 'Song.flac',
        stage: 'basic',
        resumeAvailable: false,
      },
    );

    const slot = root.querySelector('[data-role="sync-slot"][data-slot="0"]');
    expect(slot?.querySelector('[data-role="analysis-summary"]')?.textContent)
      .toBe('正在计算 BPM、Key 和响度 0/5');
    expect(slot?.querySelector('[data-role="analysis-current"]')?.textContent)
      .toBe('Song.flac');
    expect(slot?.querySelector('[data-role="analysis-message"]')?.classList.contains('analysis-progress-copy'))
      .toBe(true);
    expect((slot?.querySelector('[data-role="analysis-progress"]') as HTMLElement).style.width)
      .toBe('0%');
    expect(root.querySelector('.global-stage-message[data-role="analysis-message"]')).toBeNull();
  });

  it('renders analysis progress only in Task 2 when slotIndex is 1', () => {
    const root = renderApp(
      makeViewState({ enhancedMode: true }),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      {
        slotIndex: 1,
        status: 'running',
        completed: 2,
        total: 9,
        resultCount: 0,
        failedCount: 0,
        message: '正在计算 BPM、Key 和响度',
        currentItem: 'Song.flac',
        stage: 'basic',
        resumeAvailable: false,
      },
    );

    expect(root.querySelector('[data-slot="0"] [data-role="analysis-message"]')).toBeNull();
    expect(root.querySelector('[data-slot="1"] [data-role="analysis-message"]')?.textContent)
      .toContain('2/9');
    expect((root.querySelector('[data-slot="1"] [data-role="analysis-progress"]') as HTMLElement).style.width)
      .toBe('22%');
  });

  it('routes a two-slot analysis batch to one originating slot at a time', () => {
    const taskOne = renderApp(
      makeViewState({ enhancedMode: true }),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      {
        slotIndex: 0,
        status: 'running',
        completed: 1,
        total: 1,
        resultCount: 1,
        failedCount: 0,
        message: '正在分析任务 1',
        currentItem: 'one.flac',
        stage: 'basic',
        resumeAvailable: false,
      },
    );
    expect(taskOne.querySelector('[data-slot="0"] [data-role="analysis-message"]')).not.toBeNull();
    expect(taskOne.querySelector('[data-slot="1"] [data-role="analysis-message"]')).toBeNull();

    const taskTwo = renderApp(
      makeViewState({ enhancedMode: true }),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      {
        slotIndex: 1,
        status: 'running',
        completed: 1,
        total: 1,
        resultCount: 1,
        failedCount: 0,
        message: '正在分析任务 2',
        currentItem: 'two.flac',
        stage: 'basic',
        resumeAvailable: false,
      },
    );
    expect(taskTwo.querySelector('[data-slot="0"] [data-role="analysis-message"]')).toBeNull();
    expect(taskTwo.querySelector('[data-slot="1"] [data-role="analysis-message"]')).not.toBeNull();
  });

  it('clears the analysis slot route after a terminal result', () => {
    const root = renderApp(
      makeViewState({ enhancedMode: true }),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      {
        slotIndex: null,
        status: 'completed',
        completed: 1,
        total: 1,
        resultCount: 1,
        failedCount: 0,
        message: '分析完成',
        currentItem: '',
        stage: 'completed',
        resumeAvailable: false,
      },
    );

    expect(root.querySelector('[data-role="analysis-message"]')).toBeNull();
  });

  it('renders discovery progress without blocking the task source controls', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      { status: 'idle', completed: 0, total: 0, resultCount: 0, failedCount: 0, message: '' },
      null,
      false,
      null,
      null,
      { version: '', embedding: false, genre: false, mood: false, instrument: false, installing: false },
      null,
      {
        status: 'running',
        stage: 'locatingDatabase',
        processed: 1,
        total: 3,
        currentItem: '网易云数据库候选',
        message: '正在查找网易云数据库',
        suggestion: null,
        error: null,
      },
    );
    expect(root.querySelector('[data-role="netease-discovery-progress"]')?.textContent).toContain('1/3');
    expect(root.querySelector('[data-action="pick-source"]')).not.toBeNull();
  });

  it('refreshes conversion history after the desktop reports completion', async () => {
    const history = makeHistoryEntry({ status: 'completed', failed_count: 0, error_count: 0 });
    const services = makeMockServices({
      loadHistory: vi
        .fn()
        .mockResolvedValueOnce([])
        .mockResolvedValueOnce([history]),
      loadDesktopState: vi.fn().mockResolvedValue(
        makeDesktopState({
          slots: [
            makeDesktopSlot({ status: 'completed', progress_total: 1, progress_completed: 1 }),
            makeDesktopSlot({ status: 'running' }),
          ],
        }),
      ),
    });
    const root = document.createElement('div');

    bindApp(root, makeViewState({
      slots: [makeViewSlot({ status: 'running' }), makeViewSlot()],
    }), services);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="history"] article')).not.toBeNull();
    });
  });

  it('keeps progress text for active tasks without showing idle footer text', () => {
    const root = renderApp(
      makeViewStateWithSlot(0, {
        status: 'running',
        progressTotal: 4,
        progressCompleted: 2,
        progressText: '2/4',
      }),
    );

    expect(root.querySelector('[data-role="sync-slot"][data-slot="0"] .progress-copy')?.textContent)
      .toBe('2/4');
    expect(root.querySelector('[data-role="sync-slot"][data-slot="1"] .progress-copy')).toBeNull();
  });

  it('does not render a duplicate global conversion message below the cancel button', () => {
    const root = renderApp(
      makeViewStateWithSlot(0, {
        status: 'running',
        progressTotal: 4,
        progressCompleted: 1,
        progressText: '1/4',
      }),
    );

    expect(root.querySelector('[data-action="cancel-all"]')).not.toBeNull();
    expect(root.querySelector('.global-stage-message')).toBeNull();
    expect(root.querySelector('[data-role="conversion-message"]')).toBeNull();
  });

  it('keeps active conversion progress ahead of a completed library refresh snapshot', () => {
    const completedLibraryRefresh: LibraryRefreshProgress = {
      refreshId: 'refresh-1',
      status: 'completed',
      stage: 'committing',
      processed: 40,
      total: 40,
      currentItem: '',
      message: '歌曲库更新完成',
      summary: null,
      error: null,
    };
    const root = renderApp(
      makeViewStateWithSlot(0, {
        status: 'running',
        progressTotal: 40,
        progressCompleted: 7,
        progressText: '7/40',
      }),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      undefined,
      undefined,
      false,
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      completedLibraryRefresh,
    );

    const slot = root.querySelector('[data-role="sync-slot"][data-slot="0"]');
    expect(slot?.querySelector('.progress-copy')?.textContent).toBe('7/40');
    expect((slot?.querySelector('.progress-fill') as HTMLElement).style.width).toBe('18%');
  });

  it('removes the global status card and moves the start action directly below settings', () => {
    const root = renderApp(makeViewState());

    expect(root.querySelector('.global-status-card')).toBeNull();
    const actionGroup = root.querySelector('.global-action-group');
    expect(actionGroup).not.toBeNull();
    expect(actionGroup?.previousElementSibling?.matches('[data-role="advanced-output-settings"]'))
      .toBe(true);
  });

  it('renders the selected color theme and a top-right theme toggle', () => {
    const root = renderApp(makeViewState({ theme: 'dark' }));

    expect(root.dataset.theme).toBe('dark');
    expect(root.dataset.lightPalette).toBe('c');
    expect(root.querySelector('[data-action="toggle-theme"]')).not.toBeNull();
    expect(root.querySelector('[data-action="open-help"]')?.textContent).toContain('教程');
    expect(root.querySelector('[data-action="open-help"] .ui-icon')).not.toBeNull();
    expect(root.querySelector('.topbar-actions')?.lastElementChild?.getAttribute('data-action'))
      .toBe('toggle-lang');
  });

  it('moves output notes out of the rail and into the tutorial help document', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      undefined,
      null,
      true,
    );

    expect(root.querySelector('.rail-note')).toBeNull();
    expect(root.querySelector('[data-role="help-modal"]')?.textContent)
      .toContain('兼容模式：最高输出 320kbps MP3');
    expect(root.querySelector('[data-role="help-modal"]')?.textContent)
      .toContain('无损模式：最高输出 24-bit / 48kHz');
    const helpText = root.querySelector('[data-role="help-modal"]')?.textContent || '';
    expect(helpText).toContain('兼容模式');
    expect(helpText).toContain('无损模式');
    expect(helpText).toContain('普通转换');
    expect(helpText).toContain('增强转换');
    expect(helpText).not.toContain('Essentia');
    expect(root.querySelector('[data-role="help-modal"] [aria-labelledby="help-output-title"]')?.querySelectorAll('.help-card'))
      .toHaveLength(4);
    expect(root.querySelector('[data-role="help-modal"]')?.textContent).toContain('扫描后转换');
    expect(root.querySelector('[data-role="help-modal"]')?.textContent).toContain('直接转换');

    const helpSections = Array.from(root.querySelectorAll('[data-role="help-modal"] .help-section'));
    expect(helpSections[0]?.getAttribute('aria-labelledby')).toBe('help-conversion-title');
    expect(helpSections[1]?.getAttribute('aria-labelledby')).toBe('help-output-title');
  });

  it('keeps the global lossless format selector mounted and changes its visible state', () => {
    const compatRoot = renderApp(makeViewState({ mode: 'compat' }));
    expect(compatRoot.querySelector('.format-slot')).not.toBeNull();
    expect(compatRoot.querySelector('.format-row')?.getAttribute('data-visible')).toBe('false');
    expect(compatRoot.querySelector('.format-row')?.getAttribute('aria-hidden')).toBe('true');

    const root = renderApp(makeViewState({ mode: 'lossless', losslessFormat: 'wav' }));
    expect(root.querySelector('.format-slot')).not.toBeNull();
    expect(root.querySelector('.format-row')?.getAttribute('data-visible')).toBe('true');
    expect(root.querySelector('.format-row')?.getAttribute('aria-hidden')).toBe('false');
    expect(root.querySelector('[data-format="wav"]')?.classList.contains('selected')).toBe(true);
    expect(root.querySelector('[data-format="aiff"]')?.classList.contains('selected')).toBe(false);
  });

  it('renders persistent aria-hidden selected-label overlays for sliding mode controls', () => {
    const root = renderApp(makeViewState());
    const conversionOverlay = root.querySelector(
      '[data-role="conversion-mode-label-overlay"]',
    );
    const enhancedOverlay = root.querySelector(
      '[data-role="enhanced-mode-label-overlay"]',
    );

    expect(conversionOverlay?.getAttribute('aria-hidden')).toBe('true');
    expect(conversionOverlay?.querySelectorAll('.mode-selected-label')).toHaveLength(2);
    expect(conversionOverlay?.textContent).toContain('扫描后转换');
    expect(conversionOverlay?.textContent).toContain('直接转换');
    expect(enhancedOverlay?.getAttribute('aria-hidden')).toBe('true');
    expect(enhancedOverlay?.querySelectorAll('.mode-selected-label')).toHaveLength(2);
    expect(enhancedOverlay?.textContent).toContain('普通转换');
    expect(enhancedOverlay?.textContent).toContain('增强模式');
  });

  it('hides the enhanced-mode selector without removing its programmatic hooks', () => {
    const root = renderApp(makeViewState({ enhancedMode: true }));
    const row = root.querySelector('[data-role="enhanced-mode-switch"]');

    expect(row?.getAttribute('data-feature-hidden')).toBe('true');
    expect(row?.hasAttribute('hidden')).toBe(true);
    expect(row?.classList.contains('enhanced-mode-row-hidden')).toBe(true);
    expect(row?.querySelector('[data-enhanced-mode="on"]')).not.toBeNull();
  });

  it('keeps secondary output settings collapsed with safe defaults', () => {
    const root = renderApp(makeViewState());
    const settings = root.querySelector(
      '[data-role="advanced-output-settings"]',
    ) as HTMLDetailsElement;

    expect(settings.open).toBe(false);
    expect(settings.querySelector('summary')?.textContent).toContain('高级选项');
    expect(settings.textContent).toContain('已存在歌曲策略');
    expect(Array.from(
      (root.querySelector('[data-action="choose-conflict"]') as HTMLSelectElement).options,
    ).map((option) => option.textContent)).toEqual([
      '跳过',
      '覆盖',
      '仅更新元数据',
    ]);
    expect((root.querySelector('[data-action="choose-conflict"]') as HTMLSelectElement).value)
      .toBe('skip');
    expect((root.querySelector('[data-action="choose-filename-rule"]') as HTMLSelectElement).value)
      .toBe('title_artist');
  });

  it('renders the global concurrency controls and the task snapshot value', () => {
    const root = renderApp(makeViewState({
      concurrencyLimit: 4,
      slots: [
        makeViewSlot({ status: 'running', activeConcurrencyLimit: 2 }),
        makeViewSlot(),
      ],
    }));

    const range = root.querySelector('input[data-action="choose-concurrency-range"]') as HTMLInputElement;
    const number = root.querySelector('input[data-action="choose-concurrency-number"]') as HTMLInputElement;
    expect(range.min).toBe('1');
    expect(range.max).toBe('10');
    expect(range.value).toBe('4');
    expect(number.value).toBe('4');
    expect(root.querySelector('[data-role="concurrency-setting"]')?.textContent)
      .toContain('并行处理数量');
    expect(root.querySelector('[data-role="concurrency-setting"] small')).toBeNull();
    expect(root.querySelector('[data-role="slot-concurrency"]')).toBeNull();
    expect(root.textContent).not.toContain('当前任务并发');
  });

  it('renders independent scan progress inside each task card', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      undefined,
      {
        status: 'running',
        phase: 'scanning_source',
        processed: 5,
        total: 12,
        current_file: '/music/in-1/track.wav',
        message: '正在扫描输入目录',
        tasks: [
          { slot_index: 0, phase: 'scanning_source', processed: 5, total: 10, current_file: '/music/in-1/track.wav' },
          { slot_index: 1, phase: 'scanning_destination', processed: 2, total: 8, current_file: '/music/out-2/track.mp3' },
        ],
      },
    );

    const slots = root.querySelectorAll('[data-role="sync-slot"]');
    expect(slots[0]?.querySelector('.progress-copy')?.textContent).toContain('5/10');
    expect((slots[0]?.querySelector('.progress-fill') as HTMLElement).style.width).toBe('50%');
    expect(slots[1]?.querySelector('.progress-copy')?.textContent).toContain('2/8');
    expect((slots[1]?.querySelector('.progress-fill') as HTMLElement).style.width).toBe('25%');
    expect(slots[0]?.querySelector('.slot-status')?.textContent).toContain('运行中');
    expect(root.querySelector('[data-action="cancel-scan"]')).not.toBeNull();
    expect(root.querySelector('[data-role="scan-modal"]')).toBeNull();
  });

  it('shows an indeterminate per-phase scan count until that phase total is known', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      undefined,
      {
        status: 'running',
        phase: 'scanning_source',
        processed: 1088,
        total: 80,
        current_file: '/music/in-1/track.wav',
        message: '正在扫描输入目录',
        tasks: [
          {
            slot_index: 0,
            phase: 'scanning_source',
            processed: 1088,
            total: 80,
            source_processed: 1088,
            source_total: null,
            destination_processed: 0,
            destination_total: null,
            current_file: '/music/in-1/track.wav',
          },
        ],
      },
    );

    const slot = root.querySelector('[data-role="sync-slot"][data-slot="0"]') as HTMLElement;
    expect(slot.querySelector('.progress-copy')?.textContent).toContain('已扫描 1088 项');
    expect(slot.querySelector('.progress-copy')?.textContent).not.toContain('1088/80');
    expect(slot.querySelector('.progress-fill')?.classList.contains('is-indeterminate')).toBe(true);
    expect(slot.querySelector('.slot-status')?.textContent).toContain('运行中');
  });

  it('renders metadata matching as a real x/total phase with a stable determinate bar', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      undefined,
      {
        status: 'running',
        phase: 'matching_metadata',
        processed: 1089,
        total: 1088,
        current_file: '/music/in-1/track.wav',
        message: '正在匹配网易云元数据',
        tasks: [{
          slot_index: 0,
          phase: 'matching_metadata',
          processed: 1088,
          total: 1088,
          source_processed: 1088,
          source_total: 1088,
          destination_processed: 0,
          destination_total: null,
          metadata_processed: 7,
          metadata_total: 1088,
          current_file: '/music/in-1/track.wav',
        }],
      },
    );

    const slot = root.querySelector('[data-role="sync-slot"][data-slot="0"]') as HTMLElement;
    expect(slot.querySelector('.progress-copy')?.textContent).toContain('7/1088');
    expect(slot.querySelector('.progress-copy')?.textContent).not.toContain('已扫描 7 项');
    expect((slot.querySelector('.progress-fill') as HTMLElement).style.width).toBe('1%');
    expect(slot.querySelector('.progress-fill')?.classList.contains('is-indeterminate')).toBe(false);
  });

  it('hides enhanced analysis controls together while retaining backend hooks', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      true,
    );

    expect(root.querySelector('[data-role="enhanced-mode-switch"]')?.hasAttribute('hidden'))
      .toBe(true);
    expect(root.querySelector('.essentia-model-settings')).toBeNull();
    expect(root.querySelector('[data-action="clear-analysis-cache"]')).toBeNull();
    expect(root.querySelector('[data-action="clear-scan-cache"]')).toBeNull();
    expect(root.querySelector('[data-action="restore-bundled-essentia-models"]')).toBeNull();
    expect(root.querySelector('[data-action="open-essentia-models-page"]')).toBeNull();
    expect(root.querySelector('[data-action="import-essentia-models"]')).toBeNull();
    expect(root.querySelector('[data-role="model-drop-overlay"]')).toBeNull();
  });

  it('uses songs and singers in Chinese filename labels without changing option values', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      true,
    );
    const filenameRule = root.querySelector('[data-action="choose-filename-rule"]') as HTMLSelectElement;
    const neteaseRule = root.querySelector('[data-action="choose-netease-filename-format"]');

    expect(root.textContent).toContain('输出文件名规则');
    expect(neteaseRule).toBeNull();

    expect([...filenameRule.options].map((option) => [option.value, option.textContent])).toEqual([
      ['title_artist', '歌曲名 - 歌手（默认）'],
      ['artist_title', '歌手 - 歌曲名'],
      ['original', '保留原文件名'],
    ]);
  });

  it('blocks confirmation when the destination disk is too full', () => {
    const preview = makePreview(0);
    preview.preview.disk_space_sufficient = false;
    preview.preview.available_space_bytes = 64;
    const root = renderApp(
      makeViewState(),
      null,
      null,
      { previews: [preview], retryOf: null },
    );

    expect((root.querySelector('[data-action="confirm-start"]') as HTMLButtonElement).disabled)
      .toBe(true);
    expect(root.querySelector('[data-role="preview-modal"]')?.textContent)
      .toContain('磁盘空间不足');
  });

  it('renders version, developer, and project details in About', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      {
        version: '3.2.0-beta.3',
        developer: 'komakizhu',
        project_url: 'https://github.com/komakizhu/W4DJ-RKB',
      },
    );

    expect(root.querySelector('[data-role="about-modal"]')?.textContent).toContain('v3.2.0-beta.3');
    expect(root.querySelector('[data-role="about-modal"]')?.textContent).toContain('komakizhu');
    expect(root.querySelector('[data-role="about-modal"] [data-action="open-project-home"]')?.getAttribute('data-url')).toBe('https://github.com/komakizhu/W4DJ-RKB');
    expect(root.querySelector('[data-role="about-modal"] [data-action="reopen-onboarding"]')).toBeNull();
  });

  it('keeps Essentia analysis embedded in conversion instead of rendering a separate panel', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      null,
      false,
      false,
      false,
      0,
      {
        slotIndex: null,
        status: 'idle',
        completed: 0,
        total: 0,
        resultCount: 0,
        failedCount: 0,
        message: '',
      },
    );

    expect(root.querySelector('[data-role="analysis-panel"]')).toBeNull();
    expect(root.querySelector('[data-action="analyze-library"]')).toBeNull();
    expect(root.querySelector('[data-action="export-rekordbox"]')).toBeNull();
  });

  it('hides the song-library entry while keeping the backend UI testable', () => {
    const root = renderApp(makeViewState());
    const libraryButton = root.querySelector<HTMLButtonElement>('[data-action="open-library"]');

    expect(libraryButton).not.toBeNull();
    expect(libraryButton?.hidden).toBe(true);
    expect(libraryButton?.dataset.featureHidden).toBe('true');
  });

  it('shows slot two running state without changing slot one', () => {
    const root = renderApp(
      makeViewStateWithSlot(1, {
        status: 'running',
        progressTotal: 100,
        progressCompleted: 45,
        progressText: '45/100',
        currentFile: 'track02.wav',
      }),
    );

    const slotOne = root.querySelector('[data-role="sync-slot"][data-slot="0"]') as HTMLElement;
    const slotTwo = root.querySelector('[data-role="sync-slot"][data-slot="1"]') as HTMLElement;
    expect(slotOne.dataset.status).toBe('idle');
    expect(slotTwo.dataset.status).toBe('running');
    expect(root.querySelector('[data-action="cancel-all"]')).not.toBeNull();
    expect((slotTwo.querySelector('.progress-fill') as HTMLElement).style.width).toBe('45%');
    expect(slotTwo.querySelector('.progress-copy')?.textContent).toBe('45/100');
    expect(slotTwo.querySelector('.progress-copy--numeric')).not.toBeNull();
    expect(slotTwo.querySelector('.status-toggle')).toBeNull();
    expect(slotTwo.querySelector('[data-role="log-drawer"]')).toBeNull();
    expect(slotTwo.querySelector('.detail-toggle-copy')).toBeNull();
  });

  it('shows a localized destination fallback hint for slot two', () => {
    const root = renderApp(
      makeViewStateWithSlot(1, { destinationDirectory: '' }),
    );

    const hint = root.querySelector('[data-role="fallback-hint"][data-slot="1"]');
    expect(hint?.textContent).toContain('使用输出目录 1');
    expect(hint?.textContent).toContain('/music/out-1');
  });

  it('does not render current-track details or logs in a task card', () => {
    const root = renderApp(
      makeViewStateWithSlot(0, {
        currentFile: '悟空传 - MC赵小六.wav',
        logs: ['Desktop shell ready'],
      }),
    );

    const slot = root.querySelector('[data-role="sync-slot"][data-slot="0"]') as HTMLElement;
    expect(slot.querySelector('[data-role="log-drawer"]')).toBeNull();
    expect(slot.querySelector('.status-toggle')).toBeNull();
    expect(slot.textContent).not.toContain('悟空传 - MC赵小六.wav');
    expect(slot.textContent).not.toContain('Desktop shell ready');
  });

  it('shows a first-use onboarding guide with the five core steps', () => {
    const root = renderApp(makeViewState(), null, null, null, [], null, false, null, false, false, true);

    expect(root.querySelector('[data-role="onboarding-modal"]')?.textContent).toContain('先选输出模式');
    expect(root.querySelector('[data-role="onboarding-modal"]')?.getAttribute('data-step')).toBe('0');
    expect(root.querySelector('[data-action="onboarding-next"]')?.textContent).toBe('下一步');
    expect(root.querySelector('[data-onboarding-target="mode"]')).not.toBeNull();
  });

  it('keeps exactly one relevant control clear in every onboarding step', () => {
    const steps = [
      { step: 0, target: 'mode' },
      { step: 1, target: 'source' },
      { step: 2, target: 'destination' },
      { step: 3, target: 'start' },
      { step: 4, target: 'tutorial' },
    ] as const;

    steps.forEach(({ step, target }) => {
      const root = renderApp(
        makeViewState(),
        null,
        null,
        null,
        [],
        null,
        false,
        null,
        false,
        false,
        true,
        step,
      );

      expect(root.dataset.onboardingActive).toBe('true');
      expect(root.dataset.onboardingStep).toBe(String(step));
      expect(root.querySelectorAll(`[data-onboarding-target="${target}"]`)).toHaveLength(1);
      expect(root.querySelectorAll('[data-onboarding-target]')).toHaveLength(1);
      if (target === 'source' || target === 'destination') {
        expect(root.querySelector(`[data-onboarding-target="${target}"]`)?.closest('[data-slot="0"]')).not.toBeNull();
      }
    });

    const inactiveRoot = renderApp(makeViewState());
    expect(inactiveRoot.querySelectorAll('[data-onboarding-target]')).toHaveLength(0);
  });

  it('uses the fifth onboarding step to explain how to reopen the guide', () => {
    const root = renderApp(makeViewState(), null, null, null, [], null, false, null, false, false, true, 4);

    expect(root.querySelector('[data-role="onboarding-modal"]')?.textContent).toContain('随时重新查看教程');
    expect(root.querySelector('[data-role="onboarding-modal"]')?.textContent).toContain('重新查看使用引导');
    expect(root.querySelector('[data-role="onboarding-modal"]')?.textContent).toContain('5/5');
    expect(root.querySelector('[data-onboarding-target="tutorial"]')?.getAttribute('data-action')).toBe('open-help');
  });

  it('turns technical conversion errors into recovery-focused user messages', () => {
    expect(humanizeError('Permission denied while writing output', 'zh')).toBe(
      '没有权限写入这个文件夹，请换一个输出目录。',
    );
    expect(humanizeError('FFmpeg conversion failed', 'en')).toBe(
      'Conversion failed. Check the file or try again.',
    );
  });
});

describe('bindApp', () => {
  it('shows onboarding only on first use and remembers dismissing it', async () => {
    localStorage.removeItem('w4dj_onboarding_seen');
    const root = document.createElement('div');
    bindApp(root, makeViewState(), makeMockServices());

    await vi.waitFor(() => expect(root.querySelector('[data-role="onboarding-modal"]')).not.toBeNull());
    (root.querySelector('[data-action="dismiss-onboarding"]') as HTMLButtonElement).click();
    expect(root.querySelector('[data-role="onboarding-modal"]')).toBeNull();
    expect(localStorage.getItem('w4dj_onboarding_seen')).toBe('1');

    const secondRoot = document.createElement('div');
    bindApp(secondRoot, makeViewState(), makeMockServices());
    await vi.waitFor(() => expect(secondRoot.querySelector('[data-role="onboarding-modal"]')).toBeNull());
  });

  it('uses the Task 1 NetEase button to auto-locate and fill the source folder', async () => {
    const pickSource = vi.fn().mockResolvedValue('/music/manual');
    const locate = vi.fn().mockResolvedValue({
      databasePath: '/music/netease/db.sqlite3',
      musicFolder: '/music/netease',
      recordCount: 2,
      localFileCount: 2,
    });
    const selectSourceDirectory = vi.fn().mockResolvedValue(makeDesktopState());
    const services = makeMockServices({
      pickSource,
      locateNeteaseLibrary: locate,
      selectSourceDirectory,
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="scan-local-netease"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(locate).toHaveBeenCalledWith(true));
    await vi.waitFor(() => expect(selectSourceDirectory).toHaveBeenCalledWith(0, '/music/netease'));
    expect(pickSource).not.toHaveBeenCalled();
    expect(root.querySelector('[data-action="scan-local-netease"]')?.hasAttribute('disabled')).toBe(false);

    const contextMenu = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    root.dispatchEvent(contextMenu);
    expect(contextMenu.defaultPrevented).toBe(true);
  });

  it('waits for desktop-state hydration before applying an auto-located source folder', async () => {
    const hydration = createDeferred<DesktopState>();
    const locate = vi.fn().mockResolvedValue({
      databasePath: '/music/netease/db.sqlite3',
      musicFolder: '/music/netease',
      recordCount: 1,
      localFileCount: 1,
    });
    const selectSourceDirectory = vi.fn().mockResolvedValue(makeDesktopState());
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockReturnValue(hydration.promise),
      locateNeteaseLibrary: locate,
      selectSourceDirectory,
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="scan-local-netease"]') as HTMLButtonElement).click();
    await Promise.resolve();
    expect(locate).not.toHaveBeenCalled();

    hydration.resolve(makeDesktopState());
    await vi.waitFor(() => expect(locate).toHaveBeenCalledWith(true));
    await vi.waitFor(() => expect(selectSourceDirectory).toHaveBeenCalledWith(0, '/music/netease'));
  });

  it('offers manual folder selection only after automatic NetEase discovery fails', async () => {
    const pickSource = vi.fn().mockResolvedValue('/music/manual');
    const locate = vi.fn().mockResolvedValue({
      databasePath: null,
      musicFolder: null,
      recordCount: 0,
      localFileCount: 0,
    });
    const selectSourceDirectory = vi.fn().mockResolvedValue(makeDesktopState());
    const services = makeMockServices({
      pickSource,
      locateNeteaseLibrary: locate,
      selectSourceDirectory,
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="scan-local-netease"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(locate).toHaveBeenCalledWith(true));
    await vi.waitFor(() => expect(root.querySelector('[data-role="netease-discovery-progress"]')?.textContent).toContain('未能自动找到'));
    expect(pickSource).not.toHaveBeenCalled();
    expect(root.querySelector('[data-action="scan-local-netease"]')?.textContent).toContain('手动选择文件夹');

    (root.querySelector('[data-action="scan-local-netease"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(pickSource).toHaveBeenCalledWith(0));
    await vi.waitFor(() => expect(selectSourceDirectory).toHaveBeenCalledWith(0, '/music/manual'));
  });

  it('updates NetEase discovery progress in place while the automatic scan is running', async () => {
    const locateDeferred = createDeferred<LibraryStatus['netease']>();
    let emitProgress: ((progress: NeteaseDiscoveryProgress) => void) | null = null;
    const services = makeMockServices({
      locateNeteaseLibrary: vi.fn().mockReturnValue(locateDeferred.promise),
      listenNeteaseDiscoveryProgress: vi.fn().mockImplementation(async (handler) => {
        emitProgress = handler;
        return () => undefined;
      }),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="scan-local-netease"]') as HTMLButtonElement).click();
    (root.querySelector('[data-action="scan-local-netease"]') as HTMLButtonElement).click();
    expect(root.querySelector('[data-action="scan-local-netease"]')?.hasAttribute('disabled')).toBe(true);
    await vi.waitFor(() => expect(emitProgress).not.toBeNull());
    const appShell = root.firstElementChild;
    emitProgress?.({
      status: 'running',
      stage: 'checkingMusicFolder',
      processed: 2,
      total: 4,
      currentItem: 'Song.mp3',
      message: '正在检查本地歌曲',
      suggestion: null,
      error: null,
    });

    expect(root.firstElementChild).toBe(appShell);
    expect(root.querySelector('[data-slot="0"] [data-role="slot-progress-message"]')?.textContent)
      .toContain('2/4');

    locateDeferred.resolve({
      databasePath: '/music/netease/db.sqlite3',
      musicFolder: '/music/netease',
      recordCount: 1,
      localFileCount: 4,
    });
    await vi.waitFor(() => expect(services.selectSourceDirectory).toHaveBeenCalledWith(0, '/music/netease'));
  });

  it('offers row-only relocate/remove actions and confirmed invalid cleanup', async () => {
    const track: LibraryTrack = {
      trackKey: 'analysis:song',
      neteaseTrackId: null,
      title: 'Song',
      artists: 'Artist',
      album: 'Album',
      neteaseGenre: '',
      essentiaGenre: 'House',
      coverPath: null,
      coverAvailable: false,
      localStatus: 'missing',
      effectiveDurationSeconds: 180,
      durationSource: 'essentia',
      effectiveFormat: 'mp3',
      effectiveBitrateBps: null,
      effectiveSizeBytes: 10,
      bpm: 124,
      musicalKey: 'F',
      scale: 'minor',
      integratedLoudnessLufs: -9,
      energy: 0.8,
      danceability: 1,
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
    };
    const page: LibraryPage = { items: [track], total: 1, limit: 100, offset: 0 };
    const status: LibraryStatus = {
      catalogPath: '/tmp/w4dj.sqlite3',
      trackCount: 1,
      analyzedTrackCount: 1,
      netease: { databasePath: null, musicFolder: null, recordCount: 0, localFileCount: 0 },
      manualDatabasePath: null,
      refresh: {
        refreshId: 'idle', status: 'idle', stage: 'committing', processed: 0, total: null,
        currentItem: '', message: '', summary: null, error: null,
      },
      databaseWarning: null,
    };
    const loadLibraryStatus = vi.fn().mockResolvedValue(status);
    const queryLibraryCatalog = vi.fn().mockResolvedValue(page);
    const pickLibraryTrackFile = vi.fn().mockResolvedValue('/music/new.flac');
    const relocateLibraryTrack = vi.fn().mockResolvedValue(undefined);
    const removeLibraryTrack = vi.fn().mockResolvedValue(true);
    const clearInvalidLibraryTracks = vi.fn().mockResolvedValue(1);
    const root = document.createElement('div');
    document.body.append(root);
    bindApp(root, makeViewState(), makeMockServices({
      loadLibraryStatus,
      queryLibraryCatalog,
      pickLibraryTrackFile,
      relocateLibraryTrack,
      removeLibraryTrack,
      clearInvalidLibraryTracks,
    }));

    (root.querySelector('[data-action="open-library"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-action="library-track-detail"]')).not.toBeNull());
    const row = root.querySelector('[data-action="library-track-detail"]') as HTMLElement;
    const contextMenu = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 50 });
    row.dispatchEvent(contextMenu);
    expect(contextMenu.defaultPrevented).toBe(true);
    expect(root.querySelector('[data-action="relocate-library-track"]')).not.toBeNull();

    (root.querySelector('[data-action="relocate-library-track"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(relocateLibraryTrack).toHaveBeenCalledWith('analysis:song', '/music/new.flac'));
    expect(pickLibraryTrackFile).toHaveBeenCalledTimes(1);

    const nextRow = root.querySelector('[data-action="library-track-detail"]') as HTMLElement;
    nextRow.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 50 }));
    (root.querySelector('[data-action="remove-library-track"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(removeLibraryTrack).toHaveBeenCalledWith('analysis:song'));
    await vi.waitFor(() => expect(root.textContent).toContain('记录已从 W4DJ SQLite 移除'));

    const confirmation = root.querySelector<HTMLInputElement>('[data-action="library-confirm-clear-invalid"]')!;
    confirmation.click();
    expect((root.querySelector('[data-action="clear-invalid-library"]') as HTMLButtonElement).disabled).toBe(false);
    (root.querySelector('[data-action="clear-invalid-library"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(clearInvalidLibraryTracks).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(root.textContent).toContain('已清除 1 首失效歌曲'));
    root.remove();
  });

  it('keeps the library search caret when an automatic query replaces the table', async () => {
    const status: LibraryStatus = {
      catalogPath: '/tmp/w4dj-library.sqlite',
      trackCount: 0,
      netease: {
        databasePath: '/music/Library.db',
        musicFolder: '/music/NetEase CloudMusic',
        recordCount: 0,
        localFileCount: 0,
      },
      manualDatabasePath: null,
      refresh: {
        refreshId: 'idle',
        status: 'idle',
        stage: 'locatingDatabase',
        processed: 0,
        total: null,
        currentItem: '',
        message: '',
        summary: null,
        error: null,
      },
      databaseWarning: null,
    };
    const queryLibraryCatalog = vi.fn().mockResolvedValue({
      items: [],
      total: 0,
      limit: 100,
      offset: 0,
    });
    const services = makeMockServices({
      loadLibraryStatus: vi.fn().mockResolvedValue(status),
      queryLibraryCatalog,
    });
    const root = document.createElement('div');
    document.body.append(root);
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="open-library"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('.library-table-wrap')).not.toBeNull());

    const input = root.querySelector<HTMLInputElement>('[data-action="library-search"]')!;
    input.focus();
    input.value = 'alias';
    input.setSelectionRange(2, 2);
    input.dispatchEvent(new Event('input', { bubbles: true }));

    await vi.waitFor(() => expect(queryLibraryCatalog).toHaveBeenCalledWith(
      expect.objectContaining({ text: 'alias' }),
    ));
    await vi.waitFor(() => {
      const nextInput = root.querySelector<HTMLInputElement>('[data-action="library-search"]');
      expect(nextInput).not.toBeNull();
      expect(document.activeElement).toBe(nextInput);
      expect(nextInput?.selectionStart).toBe(2);
      expect(nextInput?.selectionEnd).toBe(2);
    });
    root.remove();
  });

  it('runs the library search immediately when pressing Enter', async () => {
    const status: LibraryStatus = {
      catalogPath: '/tmp/w4dj-library.sqlite',
      trackCount: 0,
      netease: {
        databasePath: '/music/Library.db',
        musicFolder: '/music/NetEase CloudMusic',
        recordCount: 0,
        localFileCount: 0,
      },
      manualDatabasePath: null,
      refresh: {
        refreshId: 'idle',
        status: 'idle',
        stage: 'locatingDatabase',
        processed: 0,
        total: null,
        currentItem: '',
        message: '',
        summary: null,
        error: null,
      },
      databaseWarning: null,
    };
    const emptyPage = { items: [], total: 0, limit: 100, offset: 0 };
    const queryLibraryCatalog = vi.fn().mockResolvedValue(emptyPage);
    const services = makeMockServices({
      loadLibraryStatus: vi.fn().mockResolvedValue(status),
      queryLibraryCatalog,
    });
    const root = document.createElement('div');
    document.body.append(root);
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="open-library"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('.library-table-wrap')).not.toBeNull());
    queryLibraryCatalog.mockClear();

    const input = root.querySelector<HTMLInputElement>('[data-action="library-search"]')!;
    input.focus();
    input.value = '弹舌';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const enter = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
    input.dispatchEvent(enter);

    expect(enter.defaultPrevented).toBe(true);
    await vi.waitFor(() => expect(queryLibraryCatalog).toHaveBeenCalledWith(
      expect.objectContaining({ text: '弹舌' }),
    ));
    root.remove();
  });

  it('uses the toolbar search button for the current query', async () => {
    const status: LibraryStatus = {
      catalogPath: '/tmp/w4dj-library.sqlite',
      trackCount: 0,
      netease: { databasePath: null, musicFolder: null, recordCount: 0, localFileCount: 0 },
      manualDatabasePath: null,
      refresh: {
        refreshId: 'idle', status: 'idle', stage: 'locatingDatabase', processed: 0, total: null,
        currentItem: '', message: '', summary: null, error: null,
      },
      databaseWarning: null,
    };
    const queryLibraryCatalog = vi.fn().mockResolvedValue({ items: [], total: 0, limit: 100, offset: 0 });
    const services = makeMockServices({
      loadLibraryStatus: vi.fn().mockResolvedValue(status),
      queryLibraryCatalog,
    });
    const root = document.createElement('div');
    document.body.append(root);
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="open-library"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-action="search-library"]')).not.toBeNull());
    queryLibraryCatalog.mockClear();
    const input = root.querySelector<HTMLInputElement>('[data-action="library-search"]')!;
    input.value = 'city pop';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    (root.querySelector('[data-action="search-library"]') as HTMLButtonElement).click();

    await vi.waitFor(() => expect(queryLibraryCatalog).toHaveBeenCalledWith(
      expect.objectContaining({ text: 'city pop', offset: 0 }),
    ));
    root.remove();
  });

  it('does not replace the search field while a Chinese IME composition is active', async () => {
    const status: LibraryStatus = {
      catalogPath: '/tmp/w4dj-library.sqlite',
      trackCount: 0,
      netease: {
        databasePath: '/music/Library.db',
        musicFolder: '/music/NetEase CloudMusic',
        recordCount: 0,
        localFileCount: 0,
      },
      manualDatabasePath: null,
      refresh: {
        refreshId: 'idle',
        status: 'idle',
        stage: 'locatingDatabase',
        processed: 0,
        total: null,
        currentItem: '',
        message: '',
        summary: null,
        error: null,
      },
      databaseWarning: null,
    };
    const queryLibraryCatalog = vi.fn().mockResolvedValue({
      items: [],
      total: 0,
      limit: 100,
      offset: 0,
    });
    const services = makeMockServices({
      loadLibraryStatus: vi.fn().mockResolvedValue(status),
      queryLibraryCatalog,
    });
    const root = document.createElement('div');
    document.body.append(root);
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="open-library"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-action="library-search"]')).not.toBeNull());
    queryLibraryCatalog.mockClear();

    const input = root.querySelector<HTMLInputElement>('[data-action="library-search"]')!;
    input.focus();
    input.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true, data: '' }));
    input.value = 'ni';
    const composingInput = new Event('input', { bubbles: true });
    Object.defineProperty(composingInput, 'isComposing', { value: true });
    input.dispatchEvent(composingInput);

    await new Promise((resolve) => setTimeout(resolve, 320));
    expect(queryLibraryCatalog).not.toHaveBeenCalled();
    expect(root.querySelector<HTMLInputElement>('[data-action="library-search"]')).toBe(input);

    input.value = '你';
    input.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: '你' }));
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() => expect(queryLibraryCatalog).toHaveBeenCalledWith(
      expect.objectContaining({ text: '你' }),
    ));
    root.remove();
  });

  it('ignores an older search response after the user continues typing', async () => {
    const status: LibraryStatus = {
      catalogPath: '/tmp/w4dj-library.sqlite',
      trackCount: 0,
      netease: {
        databasePath: '/music/Library.db',
        musicFolder: '/music/NetEase CloudMusic',
        recordCount: 0,
        localFileCount: 0,
      },
      manualDatabasePath: null,
      refresh: {
        refreshId: 'idle',
        status: 'idle',
        stage: 'locatingDatabase',
        processed: 0,
        total: null,
        currentItem: '',
        message: '',
        summary: null,
        error: null,
      },
      databaseWarning: null,
    };
    const emptyPage = { items: [], total: 0, limit: 100, offset: 0 };
    const olderResponse = createDeferred<typeof emptyPage>();
    const queryLibraryCatalog = vi.fn().mockResolvedValue(emptyPage);
    const services = makeMockServices({
      loadLibraryStatus: vi.fn().mockResolvedValue(status),
      queryLibraryCatalog,
    });
    const root = document.createElement('div');
    document.body.append(root);
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="open-library"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('.library-table-wrap')).not.toBeNull());
    queryLibraryCatalog.mockReset();
    queryLibraryCatalog.mockReturnValueOnce(olderResponse.promise).mockResolvedValue(emptyPage);

    const input = root.querySelector<HTMLInputElement>('[data-action="library-search"]')!;
    input.focus();
    input.value = '弹';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() => expect(queryLibraryCatalog).toHaveBeenCalledWith(
      expect.objectContaining({ text: '弹' }),
    ));

    input.value = '弹舌';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    olderResponse.resolve(emptyPage);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(root.querySelector<HTMLInputElement>('[data-action="library-search"]')?.value).toBe('弹舌');
    root.remove();
  });

  it('moves through the highlighted onboarding targets without triggering app actions', async () => {
    localStorage.removeItem('w4dj_onboarding_seen');
    const services = makeMockServices();
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => expect(root.querySelector('[data-role="onboarding-modal"]')).not.toBeNull());
    (root.querySelector('[data-action="onboarding-next"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="onboarding-modal"]')?.getAttribute('data-step')).toBe('1');
    });
    expect(root.querySelector('[data-role="onboarding-modal"]')?.textContent).toContain('拖入来源');
    expect(services.chooseMode).not.toHaveBeenCalled();
    expect(services.previewAllSync).not.toHaveBeenCalled();
  });

  it('supports reaching, completing, and reopening the fifth onboarding step', async () => {
    localStorage.removeItem('w4dj_onboarding_seen');
    const root = document.createElement('div');
    bindApp(root, makeViewState(), makeMockServices());

    await vi.waitFor(() => expect(root.querySelector('[data-role="onboarding-modal"]')).not.toBeNull());
    for (let step = 0; step < 4; step += 1) {
      root.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    }
    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="onboarding-modal"]')?.getAttribute('data-step')).toBe('4');
      expect(root.querySelector('[data-onboarding-target="tutorial"]')).not.toBeNull();
    });

    root.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector('[data-role="onboarding-modal"]')).toBeNull());
    expect(localStorage.getItem('w4dj_onboarding_seen')).toBe('1');

    (root.querySelector('[data-action="open-help"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-role="help-modal"]')).not.toBeNull());
    (root.querySelector('[data-action="reopen-onboarding"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="onboarding-modal"]')?.getAttribute('data-step')).toBe('0');
    });
  });

  it('selects a NetEase database without scanning or changing either source slot', async () => {
    const pickNeteaseDatabase = vi.fn().mockResolvedValue('/music/sqlite_storage.sqlite3');
    const selectNeteaseMetadataDatabase = vi.fn().mockResolvedValue({
      manualPath: '/music/sqlite_storage.sqlite3',
      effectivePath: '/music/sqlite_storage.sqlite3',
      source: 'manual',
      loaded: true,
      recordCount: 42,
      warning: null,
    });
    const loadNeteaseMetadataDatabaseStatus = vi.fn().mockResolvedValue({
      manualPath: null,
      effectivePath: null,
      source: 'unavailable',
      loaded: false,
      recordCount: 0,
      warning: null,
    });
    const locateNeteaseLibrary = vi.fn();
    const selectSourceDirectory = vi.fn().mockResolvedValue(makeDesktopState());
    const startScan = vi.fn();
    const previewAllSync = vi.fn();
    const startConfirmedSync = vi.fn();
    const refreshLibraryCatalog = vi.fn();
    const services = makeMockServices({
      pickNeteaseDatabase,
      loadNeteaseMetadataDatabaseStatus,
      selectNeteaseMetadataDatabase,
      locateNeteaseLibrary,
      selectSourceDirectory,
      startScan,
      previewAllSync,
      startConfirmedSync,
      refreshLibraryCatalog,
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => expect(loadNeteaseMetadataDatabaseStatus).toHaveBeenCalledOnce());
    (root.querySelector('[data-action="select-netease-database"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(selectNeteaseMetadataDatabase).toHaveBeenCalledWith('/music/sqlite_storage.sqlite3'));
    expect(locateNeteaseLibrary).not.toHaveBeenCalled();
    expect(selectSourceDirectory).not.toHaveBeenCalled();
    expect(startScan).not.toHaveBeenCalled();
    expect(previewAllSync).not.toHaveBeenCalled();
    expect(startConfirmedSync).not.toHaveBeenCalled();
    expect(refreshLibraryCatalog).not.toHaveBeenCalled();
    expect(root.querySelector('[data-role="netease-database-status"]')?.textContent)
      .toContain('已选择数据库');
  });

  it('does not overwrite the previous database state when the picker is cancelled or validation fails', async () => {
    const pickNeteaseDatabase = vi.fn()
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce('/music/invalid.sqlite3');
    const selectNeteaseMetadataDatabase = vi.fn().mockRejectedValue(new Error('schema 不受支持'));
    const services = makeMockServices({
      pickNeteaseDatabase,
      loadNeteaseMetadataDatabaseStatus: vi.fn().mockResolvedValue({
        manualPath: '/music/old.sqlite3',
        effectivePath: '/music/old.sqlite3',
        source: 'manual',
        loaded: true,
        recordCount: 3,
        warning: null,
      }),
      selectNeteaseMetadataDatabase,
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);
    await vi.waitFor(() => expect(root.querySelector('[data-action="select-netease-database"]')?.textContent)
      .toContain('old.sqlite3'));

    (root.querySelector('[data-action="select-netease-database"]') as HTMLButtonElement).click();
    await Promise.resolve();
    expect(selectNeteaseMetadataDatabase).not.toHaveBeenCalled();

    (root.querySelector('[data-action="select-netease-database"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(selectNeteaseMetadataDatabase).toHaveBeenCalledWith('/music/invalid.sqlite3'));
    expect(root.querySelector('[data-action="select-netease-database"]')?.textContent)
      .toContain('old.sqlite3');
    expect(root.querySelector('[data-role="netease-database-status"]')?.textContent)
      .toContain('schema 不受支持');
  });

  it('clears the manual database without triggering a scan and ignores duplicate clicks while busy', async () => {
    const clearDeferred = createDeferred<{
      manualPath: string | null;
      effectivePath: string | null;
      source: 'manual' | 'automatic' | 'unavailable';
      loaded: boolean;
      recordCount: number;
      warning: string | null;
    }>();
    const clearNeteaseMetadataDatabase = vi.fn().mockReturnValue(clearDeferred.promise);
    const locateNeteaseLibrary = vi.fn();
    const services = makeMockServices({
      loadNeteaseMetadataDatabaseStatus: vi.fn().mockResolvedValue({
        manualPath: '/music/old.sqlite3',
        effectivePath: '/music/old.sqlite3',
        source: 'manual',
        loaded: true,
        recordCount: 3,
        warning: null,
      }),
      clearNeteaseMetadataDatabase,
      locateNeteaseLibrary,
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);
    await vi.waitFor(() => expect(root.querySelector('[data-action="clear-netease-database"]')).not.toBeNull());

    (root.querySelector('[data-action="clear-netease-database"]') as HTMLButtonElement).click();
    (root.querySelector('[data-action="clear-netease-database"]') as HTMLButtonElement).click();
    expect(clearNeteaseMetadataDatabase).toHaveBeenCalledOnce();
    clearDeferred.resolve({
      manualPath: null,
      effectivePath: '/auto/sqlite_storage.sqlite3',
      source: 'automatic',
      loaded: true,
      recordCount: 4,
      warning: null,
    });
    await vi.waitFor(() => expect(root.querySelector('[data-action="clear-netease-database"]')).toBeNull());
    expect(locateNeteaseLibrary).not.toHaveBeenCalled();
  });

  it('loads and renders both resolved backend slots', async () => {
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(
        makeDesktopState({
          slots: [
            makeDesktopSlot({ source_directory: '/loaded/source-1' }),
            makeDesktopSlot({ source_directory: '/loaded/source-2' }),
          ],
        }),
      ),
    });

    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => {
      expect(root.textContent).toContain('/loaded/source-1');
      expect(root.textContent).toContain('/loaded/source-2');
    });
  });

  it('keeps advanced output settings open while a selection refreshes', async () => {
    const services = makeMockServices({
      chooseConflictStrategy: vi.fn().mockResolvedValue(
        makeDesktopState({ conflict_strategy: 'overwrite' }),
      ),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="advanced-output-settings"]')).not.toBeNull();
    });

    const settings = root.querySelector(
      '[data-role="advanced-output-settings"]',
    ) as HTMLDetailsElement;
    settings.open = true;
    settings.dispatchEvent(new Event('toggle', { bubbles: true }));

    const select = root.querySelector('[data-action="choose-conflict"]') as HTMLSelectElement;
    select.value = 'overwrite';
    select.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(
        (root.querySelector('[data-role="advanced-output-settings"]') as HTMLDetailsElement).open,
      ).toBe(true);
      expect((root.querySelector('[data-action="choose-conflict"]') as HTMLSelectElement).value)
        .toBe('overwrite');
    });
  });

  it('selects a slot two source folder through the unified source picker', async () => {
    const services = makeMockServices({
      pickSource: vi.fn().mockResolvedValue('/new/source-2'),
      selectSourceDirectory: vi.fn().mockResolvedValue(
        makeDesktopStateWithSlot(1, { source_directory: '/new/source-2' }),
      ),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const button = root.querySelector(
      '[data-action="pick-source"][data-slot="1"]',
    ) as HTMLButtonElement;
    button.click();

    await vi.waitFor(() => {
      expect(services.pickSource).toHaveBeenCalledWith(1);
      expect(services.selectSourceDirectory).toHaveBeenCalledWith(1, '/new/source-2');
      expect(root.textContent).toContain('/new/source-2');
    });
  });

  it('opens each task output folder, including task two fallback output', async () => {
    const services = makeMockServices();
    const root = document.createElement('div');
    bindApp(root, makeViewState({
      slots: [
        makeViewSlot({ destinationDirectory: '/music/out-1' }),
        makeViewSlot({ destinationDirectory: '' }),
      ],
    }), services);

    (root.querySelector('[data-action="open-destination"][data-slot="0"]') as HTMLButtonElement).click();
    (root.querySelector('[data-action="open-destination"][data-slot="1"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(services.openDestination).toHaveBeenNthCalledWith(1, '/music/out-1');
      expect(services.openDestination).toHaveBeenNthCalledWith(2, '/music/out-1');
    });
  });

  it('selects a file or folder through the unified source picker for slot two', async () => {
    const services = makeMockServices({
      pickSource: vi.fn().mockResolvedValue('/music/single-track.flac'),
      selectSourceDirectory: vi.fn().mockResolvedValue(
        makeDesktopStateWithSlot(1, { source_directory: '/music/single-track.flac' }),
      ),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    expect(root.querySelectorAll('[data-action="pick-source"][data-slot="1"]')).toHaveLength(1);
    expect(root.querySelector('[data-action="pick-source-file"][data-slot="1"]')).toBeNull();
    (root.querySelector('[data-action="pick-source"][data-slot="1"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(services.pickSource).toHaveBeenCalledWith(1);
      expect(services.selectSourceDirectory).toHaveBeenCalledWith(1, '/music/single-track.flac');
      expect(root.textContent).toContain('/music/single-track.flac');
    });
  });

  it('lets the platform fallback picker choose a source folder', async () => {
    const openSource = vi.fn().mockResolvedValue('/music/folder');

    const selected = await pickSourceWithPlatformDialog(
      '选择来源 2',
      'zh',
      async () => 'folder',
      openSource,
    );

    expect(selected).toBe('/music/folder');
    expect(openSource).toHaveBeenCalledWith({
      directory: true,
      title: '选择来源 2',
    });
  });

  it('keeps the platform fallback picker able to choose a single track', async () => {
    const openSource = vi.fn().mockResolvedValue('/music/track.flac');

    const selected = await pickSourceWithPlatformDialog(
      'Choose source 2',
      'en',
      async () => 'track',
      openSource,
    );

    expect(selected).toBe('/music/track.flac');
    expect(openSource).toHaveBeenCalledWith({
      directory: false,
      title: 'Choose source 2',
      filters: [
        {
          name: 'Supported audio files',
          extensions: ['mp3', 'flac', 'ncm', 'wav', 'aiff'],
        },
      ],
    });
  });

  it('does not open a platform picker after cancelling the source type prompt', async () => {
    const openSource = vi.fn();

    const selected = await pickSourceWithPlatformDialog(
      '选择来源 2',
      'zh',
      async () => 'cancel',
      openSource,
    );

    expect(selected).toBeNull();
    expect(openSource).not.toHaveBeenCalled();
  });

  it('shows a source picker failure in the affected task status', async () => {
    const services = makeMockServices({
      pickSource: vi.fn().mockRejectedValue(new Error('无法打开来源选择窗口')),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="pick-source"][data-slot="0"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      const slot = root.querySelector('[data-role="sync-slot"][data-slot="0"]') as HTMLElement;
      expect(slot.dataset.status).toBe('error');
      expect(slot.querySelector('.progress-copy')?.textContent).toContain('无法打开来源选择窗口');
      expect(slot.querySelector('.progress-copy--numeric')).toBeNull();
    });
  });

  it('accepts a single track dropped onto a source slot', async () => {
    const services = makeMockServices({
      selectSourceDirectory: vi.fn().mockResolvedValue(
        makeDesktopStateWithSlot(0, { source_directory: '/music/dropped-track.wav' }),
      ),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);
    const sourcePicker = root.querySelector(
      '[data-role="source-picker"][data-slot="0"]',
    ) as HTMLElement;
    const file = new File(['audio'], 'dropped-track.wav', { type: 'audio/wav' });
    Object.defineProperty(file, 'path', { value: '/music/dropped-track.wav' });
    const event = new Event('drop', { bubbles: true, cancelable: true });
    Object.defineProperty(event, 'dataTransfer', {
      value: { files: [file], getData: vi.fn().mockReturnValue('') },
    });

    sourcePicker.dispatchEvent(event);

    await vi.waitFor(() => {
      expect(services.selectSourceDirectory).toHaveBeenCalledWith(0, '/music/dropped-track.wav');
      expect(root.textContent).toContain('/music/dropped-track.wav');
    });
  });

  it('routes a continuous browser drag by live pointer position after entering task two', async () => {
    const services = makeMockServices();
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);
    const taskOneSource = root.querySelector(
      '[data-role="source-picker"][data-slot="0"]',
    ) as HTMLElement;
    const taskTwoSource = root.querySelector(
      '[data-role="source-picker"][data-slot="1"]',
    ) as HTMLElement;
    vi.spyOn(taskOneSource, 'getBoundingClientRect').mockReturnValue({
      left: 100,
      top: 100,
      right: 300,
      bottom: 200,
      width: 200,
      height: 100,
      x: 100,
      y: 100,
      toJSON: () => ({}),
    });
    vi.spyOn(taskTwoSource, 'getBoundingClientRect').mockReturnValue({
      left: 100,
      top: 300,
      right: 300,
      bottom: 400,
      width: 200,
      height: 100,
      x: 100,
      y: 300,
      toJSON: () => ({}),
    });

    taskTwoSource.dispatchEvent(new MouseEvent('dragover', {
      bubbles: true,
      cancelable: true,
      clientX: 150,
      clientY: 350,
    }));
    expect(taskTwoSource.classList.contains('is-drag-over')).toBe(true);

    // WKWebView can keep the original event target while the pointer moves.
    taskTwoSource.dispatchEvent(new MouseEvent('dragover', {
      bubbles: true,
      cancelable: true,
      clientX: 150,
      clientY: 150,
    }));

    expect(taskOneSource.classList.contains('is-drag-over')).toBe(true);
    expect(taskTwoSource.classList.contains('is-drag-over')).toBe(false);

    taskTwoSource.dispatchEvent(new MouseEvent('dragover', {
      bubbles: true,
      cancelable: true,
      clientX: 350,
      clientY: 250,
    }));
    expect(taskOneSource.classList.contains('is-drag-over')).toBe(false);
    expect(taskTwoSource.classList.contains('is-drag-over')).toBe(false);

    const folder = new File([], 'music');
    Object.defineProperty(folder, 'path', { value: '/music/from-task-two-to-one' });
    const drop = new MouseEvent('drop', {
      bubbles: true,
      cancelable: true,
      clientX: 150,
      clientY: 150,
    });
    Object.defineProperty(drop, 'dataTransfer', {
      value: { files: [folder], getData: vi.fn().mockReturnValue('') },
    });
    taskTwoSource.dispatchEvent(drop);

    await vi.waitFor(() => {
      expect(services.selectSourceDirectory).toHaveBeenCalledWith(
        0,
        '/music/from-task-two-to-one',
      );
    });
  });

  it('rejects model drops without showing an import route', async () => {
    const alert = vi.spyOn(window, 'alert').mockImplementation(() => undefined);
    const selectSourceDirectory = vi.fn();
    const services = makeMockServices({ selectSourceDirectory });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);
    const model = new File(['model'], 'model.json');
    Object.defineProperty(model, 'path', { value: '/Downloads/model.json' });
    const dragover = new Event('dragover', { bubbles: true, cancelable: true });
    Object.defineProperty(dragover, 'dataTransfer', { value: { files: [model], getData: vi.fn().mockReturnValue('') } });
    root.dispatchEvent(dragover);
    expect(dragover.defaultPrevented).toBe(true);
    expect(root.querySelector('[data-role="model-drop-overlay"]')).toBeNull();

    const drop = new Event('drop', { bubbles: true, cancelable: true });
    Object.defineProperty(drop, 'dataTransfer', { value: { files: [model], getData: vi.fn().mockReturnValue('') } });
    root.dispatchEvent(drop);

    await vi.waitFor(() => expect(alert).toHaveBeenCalledWith('模型导入入口已移除，增强模式使用内置模型。'));
    expect(selectSourceDirectory).not.toHaveBeenCalled();
    alert.mockRestore();
  });

  it('routes native drops to all four task fields independent of traversal direction', () => {
    const targets = [
      { value: { id: 'source-1' }, rect: { left: 0, top: 0, right: 200, bottom: 80 } },
      { value: { id: 'destination-1' }, rect: { left: 220, top: 0, right: 420, bottom: 80 } },
      { value: { id: 'source-2' }, rect: { left: 0, top: 100, right: 200, bottom: 180 } },
      { value: { id: 'destination-2' }, rect: { left: 220, top: 100, right: 420, bottom: 180 } },
    ];

    const route = (x: number, y: number) =>
      resolveDropTargetAt(targets, { x: x * 2, y: y * 2 }, 2, 'physical')?.id;
    const downward = [
      route(100, 40),
      route(320, 40),
      route(100, 140),
      route(320, 140),
    ];
    const upward = [
      route(320, 140),
      route(100, 140),
      route(320, 40),
      route(100, 40),
    ];

    expect(downward).toEqual(['source-1', 'destination-1', 'source-2', 'destination-2']);
    expect(upward).toEqual(['destination-2', 'source-2', 'destination-1', 'source-1']);
  });

  it('keeps macOS native drop coordinates in the webview coordinate system', () => {
    const targets = [
      { value: { id: 'source-1' }, rect: { left: 280, top: 180, right: 570, bottom: 285 } },
      { value: { id: 'destination-1' }, rect: { left: 610, top: 180, right: 885, bottom: 285 } },
      { value: { id: 'source-2' }, rect: { left: 280, top: 390, right: 570, bottom: 465 } },
      { value: { id: 'destination-2' }, rect: { left: 610, top: 390, right: 885, bottom: 465 } },
    ];

    // Wry on macOS reports a webview-relative point even though Tauri types it
    // as a PhysicalPosition. Dividing this point by the Retina scale lands on
    // task 1's source field instead of task 2's destination field.
    expect(resolveDropTargetAt(targets, { x: 700, y: 415 }, 2)?.id).toBe('destination-2');
  });

  it('clears slot two source and destination paths without touching files', async () => {
    const services = makeMockServices({
      selectSourceDirectory: vi.fn().mockResolvedValue(
        makeDesktopStateWithSlot(1, { source_directory: '' }),
      ),
      selectDestinationDirectory: vi.fn().mockResolvedValue(
        makeDesktopStateWithSlot(1, { destination_directory: '' }),
      ),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="clear-source"][data-slot="1"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(services.selectSourceDirectory).toHaveBeenCalledWith(1, '');
    });

    (root.querySelector('[data-action="clear-destination"][data-slot="1"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(services.selectDestinationDirectory).toHaveBeenCalledWith(1, '');
    });
  });

  it('updates global mode and lossless format', async () => {
    const services = makeMockServices({
      chooseMode: vi
        .fn()
        .mockResolvedValue(makeDesktopState({ mode: 'lossless', lossless_format: 'wav' })),
      chooseLosslessFormat: vi
        .fn()
        .mockResolvedValue(makeDesktopState({ mode: 'lossless', lossless_format: 'aiff' })),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);
    const formatRow = root.querySelector('.format-row');

    (root.querySelector('[data-mode="lossless"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(root.querySelector('.format-row')).toBe(formatRow);
      expect(root.querySelector('.format-row')?.getAttribute('data-visible')).toBe('true');
      expect(root.querySelector('.format-row')?.getAttribute('aria-hidden')).toBe('false');
    });

    (root.querySelector('[data-format="aiff"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(services.chooseMode).toHaveBeenCalledWith('lossless');
      expect(services.chooseLosslessFormat).toHaveBeenCalledWith('aiff');
      expect(root.querySelector('.format-row')).toBe(formatRow);
      expect(root.querySelector('.format-row')?.getAttribute('data-selected-format')).toBe('aiff');
    });
  });

  it('keeps the lossless selector mounted during a quick mode reversal', async () => {
    const services = makeMockServices({
      chooseMode: vi
        .fn()
        .mockResolvedValueOnce(makeDesktopState({ mode: 'lossless', lossless_format: 'wav' }))
        .mockResolvedValueOnce(makeDesktopState({ mode: 'compat', lossless_format: null })),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);
    const formatRow = root.querySelector('.format-row');

    (root.querySelector('[data-mode="lossless"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('.format-row')?.getAttribute('data-visible')).toBe('true'));
    (root.querySelector('[data-mode="compat"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(root.querySelector('.format-row')).toBe(formatRow);
      expect(root.querySelector('.format-row')?.getAttribute('data-visible')).toBe('false');
      expect(root.querySelector('.format-row')?.getAttribute('aria-hidden')).toBe('true');
    });
  });

  it('switches conversion flow with the same sliding control pattern', async () => {
    const services = makeMockServices({
      chooseConversionMode: vi.fn().mockResolvedValue(
        makeDesktopState({ conversion_mode: 'direct' }),
      ),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const initialSwitch = root.querySelector('[data-role="conversion-mode-switch"]');
    expect(initialSwitch?.getAttribute('data-selected-conversion-mode')).toBe('scan_then_convert');
    (root.querySelector('[data-conversion-mode="direct"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(services.chooseConversionMode).toHaveBeenCalledWith('direct');
      expect(root.querySelector('[data-role="conversion-mode-switch"]')
        ?.getAttribute('data-selected-conversion-mode')).toBe('direct');
    });
  });

  it('keeps conversion and enhanced button nodes stable while sliding', async () => {
    const conversionDeferred = createDeferred<DesktopState>();
    const enhancedDeferred = createDeferred<DesktopState>();
    const services = makeMockServices({
      chooseConversionMode: vi.fn().mockReturnValue(conversionDeferred.promise),
      chooseEnhancedMode: vi.fn().mockReturnValue(enhancedDeferred.promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const conversionSwitch = root.querySelector('[data-role="conversion-mode-switch"]');
    const conversionButton = root.querySelector('[data-conversion-mode="direct"]');
    const conversionOverlay = root.querySelector(
      '[data-role="conversion-mode-label-overlay"]',
    );
    const enhancedSwitchBeforeConversion = root.querySelector('[data-role="enhanced-mode-switch"]');
    const enhancedOverlay = root.querySelector(
      '[data-role="enhanced-mode-label-overlay"]',
    );
    const enhancedButtonsBeforeConversion = Array.from(
      root.querySelectorAll<HTMLButtonElement>('[data-role="enhanced-mode-switch"] .mode-button'),
    );
    (conversionButton as HTMLButtonElement).click();

    expect(root.querySelector('[data-role="conversion-mode-switch"]')).toBe(conversionSwitch);
    expect(root.querySelector('[data-conversion-mode="direct"]')).toBe(conversionButton);
    expect(root.querySelector('[data-role="conversion-mode-label-overlay"]')).toBe(
      conversionOverlay,
    );
    expect(conversionSwitch?.hasAttribute('data-selection-pending')).toBe(true);
    expect((conversionButton as HTMLButtonElement).disabled).toBe(false);
    expect((conversionButton as HTMLButtonElement).getAttribute('aria-disabled')).toBe('true');
    expect(root.querySelector('[data-role="enhanced-mode-switch"]')
      ?.hasAttribute('data-selection-pending')).toBe(false);
    expect(root.querySelector('[data-role="enhanced-mode-switch"]')).toBe(
      enhancedSwitchBeforeConversion,
    );
    enhancedButtonsBeforeConversion.forEach((button) => {
      expect(button.disabled).toBe(false);
    });

    conversionDeferred.resolve(makeDesktopState({ conversion_mode: 'direct' }));
    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="conversion-mode-switch"]')
        ?.getAttribute('data-selected-conversion-mode')).toBe('direct');
      expect((root.querySelector('[data-conversion-mode="direct"]') as HTMLButtonElement).disabled).toBe(false);
      expect(root.querySelector('[data-role="conversion-mode-switch"]')
        ?.getAttribute('data-selection-pending')).toBeNull();
    });

    const enhancedSwitch = root.querySelector('[data-role="enhanced-mode-switch"]');
    const enhancedButton = root.querySelector('[data-enhanced-mode="on"]');
    const conversionButtonsBeforeEnhanced = Array.from(
      root.querySelectorAll<HTMLButtonElement>('[data-role="conversion-mode-switch"] .mode-button'),
    );
    expect((enhancedButton as HTMLButtonElement).disabled).toBe(false);
    (enhancedButton as HTMLButtonElement).click();
    expect(services.chooseEnhancedMode).toHaveBeenCalledWith(true);

    expect(root.querySelector('[data-role="enhanced-mode-switch"]')).toBe(enhancedSwitch);
    expect(root.querySelector('[data-enhanced-mode="on"]')).toBe(enhancedButton);
    expect(root.querySelector('[data-role="enhanced-mode-label-overlay"]')).toBe(
      enhancedOverlay,
    );
    expect((enhancedButton as HTMLButtonElement).disabled).toBe(false);
    expect((enhancedButton as HTMLButtonElement).getAttribute('aria-disabled')).toBe('true');
    expect(root.querySelector('[data-role="conversion-mode-switch"]')
      ?.hasAttribute('data-selection-pending')).toBe(false);
    conversionButtonsBeforeEnhanced.forEach((button) => {
      expect(button.disabled).toBe(false);
    });

    enhancedDeferred.resolve(makeDesktopState({ enhanced_mode: true }));
    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="enhanced-mode-switch"]')
        ?.getAttribute('data-selected-enhanced-mode')).toBe('on');
    });
  });

  it('does not redraw sliding controls when initial state hydration resolves during the first switch', async () => {
    const initialState = createDeferred<DesktopState>();
    const enhancedState = createDeferred<DesktopState>();
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockReturnValue(initialState.promise),
      chooseEnhancedMode: vi.fn().mockReturnValue(enhancedState.promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const enhancedSwitch = root.querySelector('[data-role="enhanced-mode-switch"]');
    (root.querySelector('[data-enhanced-mode="on"]') as HTMLButtonElement).click();

    initialState.resolve(makeDesktopState());
    await Promise.resolve();
    expect(root.querySelector('[data-role="enhanced-mode-switch"]')).toBe(enhancedSwitch);

    enhancedState.resolve(makeDesktopState({ enhanced_mode: true }));
    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="enhanced-mode-switch"]')
        ?.getAttribute('data-selected-enhanced-mode')).toBe('on');
    });
    expect(root.querySelector('[data-role="enhanced-mode-switch"]')).toBe(enhancedSwitch);
  });

  it('keeps the enhanced selector stable when quickly reversing the slide', async () => {
    const enhancedOn = createDeferred<DesktopState>();
    const enhancedOff = createDeferred<DesktopState>();
    const services = makeMockServices({
      chooseEnhancedMode: vi.fn((enabled: boolean) =>
        (enabled ? enhancedOn : enhancedOff).promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const shell = root.querySelector<HTMLElement>('.app-shell');
    let forcedLayoutReads = 0;
    Object.defineProperty(shell as HTMLElement, 'offsetWidth', {
      configurable: true,
      get: () => {
        forcedLayoutReads += 1;
        return 1440;
      },
    });
    const enhancedSwitch = root.querySelector('[data-role="enhanced-mode-switch"]');
    const enhancedOverlay = root.querySelector(
      '[data-role="enhanced-mode-label-overlay"]',
    );
    (root.querySelector('[data-enhanced-mode="on"]') as HTMLButtonElement).click();
    enhancedOn.resolve(makeDesktopState({ enhanced_mode: true }));
    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="enhanced-mode-switch"]')
        ?.getAttribute('data-selected-enhanced-mode')).toBe('on');
    });

    (root.querySelector('[data-enhanced-mode="off"]') as HTMLButtonElement).click();
    enhancedOff.resolve(makeDesktopState({ enhanced_mode: false }));
    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="enhanced-mode-switch"]')
        ?.getAttribute('data-selected-enhanced-mode')).toBe('off');
    });
    expect(root.querySelector('[data-role="enhanced-mode-switch"]')).toBe(enhancedSwitch);
    expect(root.querySelector('[data-role="enhanced-mode-label-overlay"]')).toBe(
      enhancedOverlay,
    );
    expect(forcedLayoutReads).toBe(0);
  });

  it('keeps the conversion selector stable when quickly reversing the slide', async () => {
    const conversionDirect = createDeferred<DesktopState>();
    const conversionScan = createDeferred<DesktopState>();
    const services = makeMockServices({
      chooseConversionMode: vi.fn((mode: 'scan_then_convert' | 'direct') =>
        (mode === 'direct' ? conversionDirect : conversionScan).promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const shell = root.querySelector<HTMLElement>('.app-shell');
    let forcedLayoutReads = 0;
    Object.defineProperty(shell as HTMLElement, 'offsetWidth', {
      configurable: true,
      get: () => {
        forcedLayoutReads += 1;
        return 1440;
      },
    });
    const conversionSwitch = root.querySelector('[data-role="conversion-mode-switch"]');
    const conversionOverlay = root.querySelector(
      '[data-role="conversion-mode-label-overlay"]',
    );
    (root.querySelector('[data-conversion-mode="direct"]') as HTMLButtonElement).click();
    conversionDirect.resolve(makeDesktopState({ conversion_mode: 'direct' }));
    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="conversion-mode-switch"]')
        ?.getAttribute('data-selected-conversion-mode')).toBe('direct');
    });

    (root.querySelector('[data-conversion-mode="scan_then_convert"]') as HTMLButtonElement).click();
    conversionScan.resolve(makeDesktopState({ conversion_mode: 'scan_then_convert' }));
    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="conversion-mode-switch"]')
        ?.getAttribute('data-selected-conversion-mode')).toBe('scan_then_convert');
    });
    expect(root.querySelector('[data-role="conversion-mode-switch"]')).toBe(conversionSwitch);
    expect(root.querySelector('[data-role="conversion-mode-label-overlay"]')).toBe(
      conversionOverlay,
    );
    expect(forcedLayoutReads).toBe(0);
  });

  it('persists the optional Essentia enhanced mode with the sliding control pattern', async () => {
    const services = makeMockServices({
      chooseEnhancedMode: vi.fn().mockResolvedValue(
        makeDesktopState({ enhanced_mode: true }),
      ),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const initialSwitch = root.querySelector('[data-role="enhanced-mode-switch"]');
    expect(initialSwitch?.getAttribute('data-selected-enhanced-mode')).toBe('off');
    (root.querySelector('[data-enhanced-mode="on"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(services.chooseEnhancedMode).toHaveBeenCalledWith(true);
      expect(root.querySelector('[data-role="enhanced-mode-switch"]')
        ?.getAttribute('data-selected-enhanced-mode')).toBe('on');
    });
  });

  it('uses a conversion icon for ordinary conversion instead of a completion checkmark', () => {
    const root = renderApp(makeViewState());

    expect(root.querySelector('[data-enhanced-mode="off"] .ui-icon-convert')).not.toBeNull();
    expect(root.querySelector('[data-enhanced-mode="off"] .ui-icon-check')).toBeNull();
  });

  it('persists conflict and filename selections through backend services', async () => {
    const services = makeMockServices({
      chooseConflictStrategy: vi.fn().mockResolvedValue(
        makeDesktopState({ conflict_strategy: 'overwrite' }),
      ),
      chooseFilenameRule: vi.fn().mockResolvedValue(
        makeDesktopState({ filename_rule: 'artist_title' }),
      ),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const conflict = root.querySelector('[data-action="choose-conflict"]') as HTMLSelectElement;
    conflict.value = 'overwrite';
    conflict.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(services.chooseConflictStrategy).toHaveBeenCalledWith('overwrite'));

    const filename = root.querySelector('[data-action="choose-filename-rule"]') as HTMLSelectElement;
    filename.value = 'artist_title';
    filename.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(services.chooseFilenameRule).toHaveBeenCalledWith('artist_title'));

    expect(root.querySelector('[data-action="choose-netease-filename-format"]')).toBeNull();
  });

  it('shows one combined preview modal before starting both slots', async () => {
    const services = makeMockServices();
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="preview-modal"]')).not.toBeNull();
      expect(root.querySelector('[data-role="preview-modal"]')?.textContent).toContain('新增文件');
      expect(root.querySelector('[data-role="preview-modal"]')?.textContent).toContain('预计输出');
    });
    expect(services.startConfirmedSync).not.toHaveBeenCalled();
  });

  it('shows scan progress immediately and only confirms after the scan completes', async () => {
    const deferred = createDeferred<AppScanProgress>();
    const services = makeMockServices({ startScan: vi.fn().mockReturnValue(deferred.promise) });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    expect(root.querySelector('[data-role="scan-modal"]')).toBeNull();
    await vi.waitFor(() => expect(root.querySelector('[data-action="cancel-scan"]')).not.toBeNull());
    expect(services.startConfirmedSync).not.toHaveBeenCalled();

    deferred.resolve({
      status: 'completed',
      phase: 'completed',
      processed: 4,
      total: 4,
      current_file: '',
      message: '扫描完成',
    });
    await vi.waitFor(() => expect(root.querySelector('[data-role="preview-modal"]')).not.toBeNull());
  });

  it('direct mode starts conversion automatically after a completed scan', async () => {
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({ conversion_mode: 'direct' })),
      loadScanResult: vi.fn().mockResolvedValue(makePreviewResponse()),
      startConfirmedSync: vi.fn().mockResolvedValue(makeDesktopState({
        slots: [
          makeDesktopSlot({ status: 'running', progress_total: 1 }),
          makeDesktopSlot({ status: 'idle' }),
        ],
      })),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({ conversionMode: 'direct' }), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(services.startConfirmedSync).toHaveBeenCalledTimes(1);
      expect(root.querySelector('[data-role="preview-modal"]')).toBeNull();
    });
    expect(services.readAudioFile).not.toHaveBeenCalled();
    expect(services.saveTrackAnalyses).not.toHaveBeenCalled();
  });

  it('waits for desktop-state hydration before deciding whether to run enhanced analysis', async () => {
    const initialState = createDeferred<DesktopState>();
    const services = makeMockServices({
      loadDesktopState: vi.fn()
        .mockReturnValueOnce(initialState.promise)
        .mockResolvedValue(makeDesktopState({
          conversion_mode: 'direct',
          enhanced_mode: true,
        })),
      loadScanResult: vi.fn().mockResolvedValue([makePreview(0)]),
      readAudioFile: vi.fn().mockRejectedValue(new Error('test decode failure')),
      startConfirmedSync: vi.fn().mockResolvedValue(makeDesktopState({
        conversion_mode: 'direct',
        enhanced_mode: true,
        slots: [
          makeDesktopSlot({ status: 'running', progress_total: 1 }),
          makeDesktopSlot({ status: 'idle' }),
        ],
      })),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({ conversionMode: 'direct', enhancedMode: true }), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await Promise.resolve();
    expect(services.startScan).not.toHaveBeenCalled();
    expect(services.startConfirmedSync).not.toHaveBeenCalled();

    initialState.resolve(makeDesktopState({
      conversion_mode: 'direct',
      enhanced_mode: true,
    }));

    await vi.waitFor(() => {
      expect(services.startScan).toHaveBeenCalledTimes(1);
      expect(services.startConfirmedSync).toHaveBeenCalledTimes(1);
      expect(services.applyTrackAnalysisResults).toHaveBeenCalledTimes(1);
    });
  });

  it('runs Essentia only when enhanced mode is enabled', async () => {
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({
        conversion_mode: 'direct',
        enhanced_mode: true,
      })),
      loadScanResult: vi.fn().mockResolvedValue([makePreview(0)]),
      readAudioFile: vi.fn().mockRejectedValue(new Error('test decode failure')),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({
      conversionMode: 'direct',
      enhancedMode: true,
      neteaseFilenameFormat: 'title_artist',
    }), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(services.startConfirmedSync).toHaveBeenCalledTimes(1);
    });
    await vi.waitFor(() => {
      expect(services.readAudioFile).toHaveBeenCalledWith('/music/in-1/Song.mp3');
      expect(services.applyTrackAnalysisResults).toHaveBeenCalledTimes(1);
    });
    expect(services.startConfirmedSync).toHaveBeenCalledWith(
      expect.any(Array),
      null,
      [],
      [],
      expect.any(String),
    );
    expect(services.applyTrackAnalysisResults).toHaveBeenCalledWith(
      expect.any(String),
      expect.any(Array),
      [],
      [{
        path: '/music/in-1/Song.mp3',
        message: 'test decode failure',
      }],
    );
  });

  it('defers model status checks and bundled installation until enhanced analysis starts', async () => {
    const modelStatus = {
      version: 'essentia-v2',
      embedding: true,
      genre: true,
      mood: true,
      instrument: true,
      installing: false,
      emotionContinuous: true,
      emotionCluster: true,
      discogsEffnet: {
        embedding: true,
        moodTheme: true,
        approachability: true,
        instrumentation: true,
        timbre: true,
        danceability: true,
      },
    };
    const getEssentiaModelStatus = vi.fn().mockResolvedValue(modelStatus);
    const ensureEssentiaModels = vi.fn().mockResolvedValue(modelStatus);
    const loadEssentiaModel = vi.fn().mockImplementation((id: string) => Promise.resolve({
      id,
      modelJson: '{}',
      weightData: [],
      classes: [],
      kind: 'embedding' as const,
      version: 'essentia-v2',
    }));
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({
        conversion_mode: 'direct',
        enhanced_mode: true,
      })),
      loadScanResult: vi.fn().mockResolvedValue([makePreview(0)]),
      readAudioFile: vi.fn().mockRejectedValue(new Error('test decode failure')),
      getEssentiaModelStatus,
      ensureEssentiaModels,
      loadEssentiaModel,
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({ conversionMode: 'direct', enhancedMode: true }), services);

    await Promise.resolve();
    expect(getEssentiaModelStatus).not.toHaveBeenCalled();
    expect(ensureEssentiaModels).not.toHaveBeenCalled();
    expect(loadEssentiaModel).not.toHaveBeenCalled();

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(ensureEssentiaModels).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(loadEssentiaModel).toHaveBeenCalled());
    expect(getEssentiaModelStatus).not.toHaveBeenCalled();
    expect(ensureEssentiaModels.mock.invocationCallOrder[0])
      .toBeLessThan(loadEssentiaModel.mock.invocationCallOrder[0]);
  });

  it('keeps enhanced analysis progress in the originating Task 2 slot', async () => {
    const readDeferred = createDeferred<number[]>();
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({
        conversion_mode: 'direct',
        enhanced_mode: true,
      })),
      loadScanResult: vi.fn().mockResolvedValue([makePreview(1)]),
      readAudioFile: vi.fn().mockReturnValue(readDeferred.promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({ conversionMode: 'direct', enhancedMode: true }), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-action="cancel-analysis"]')).not.toBeNull());

    expect(root.querySelector('[data-slot="0"] [data-role="analysis-message"]')).toBeNull();
    expect(root.querySelector('[data-slot="1"] [data-role="analysis-message"]')).not.toBeNull();
    expect(root.querySelector('[data-action="cancel-analysis"]')).not.toBeNull();
    readDeferred.resolve([]);
  });

  it('moves a two-slot analysis batch between its originating task cards', async () => {
    const firstRead = createDeferred<number[]>();
    const secondRead = createDeferred<number[]>();
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({
        conversion_mode: 'direct',
        enhanced_mode: true,
      })),
      loadScanResult: vi.fn().mockResolvedValue([makePreview(0), makePreview(1)]),
      readAudioFile: vi.fn()
        .mockReturnValueOnce(firstRead.promise)
        .mockReturnValueOnce(secondRead.promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({ conversionMode: 'direct', enhancedMode: true }), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(root.querySelector('[data-action="cancel-analysis"]')).not.toBeNull();
      expect(root.querySelector('[data-slot="0"] [data-role="analysis-message"]')).not.toBeNull();
    });
    expect(root.querySelector('[data-slot="1"] [data-role="analysis-message"]')).toBeNull();

    firstRead.resolve([]);
    await vi.waitFor(() => {
      expect(services.readAudioFile).toHaveBeenCalledWith('/music/in-2/Song.mp3');
      expect(root.querySelector('[data-slot="0"] [data-role="analysis-message"]')).toBeNull();
      expect(root.querySelector('[data-slot="1"] [data-role="analysis-message"]')).not.toBeNull();
    });
    secondRead.resolve([]);
  });

  it('persists each completed analysis candidate before moving to the next song', async () => {
    const preview = makePreview(0);
    const secondCandidate = {
      ...preview.preview.candidates[0],
      name: 'Second Song',
      source_path: '/music/in-1/Second.mp3',
      destination_path: '/music/out-1/Second.mp3',
    };
    preview.preview.candidates.push(secondCandidate);
    const cached = {
      path: '/music/in-1/Song.mp3',
      title: 'Song',
      artist: 'Artist',
      album: '',
      durationSeconds: 180,
      bpm: 140,
      key: 'F',
      scale: 'minor',
      keyStrength: 0.8,
      integratedLoudnessLufs: -8,
      loudnessRangeLu: 4,
      energy: 0.9,
      danceability: 0.7,
      beatPositions: [],
      analyzedAt: '2026-08-06T00:00:00Z',
      analyzer: 'Essentia.js',
      analysisVersion: '0.2.0',
      sourceSizeBytes: 0,
      sourceModifiedAt: null,
      dropAnalysis: { status: 'completed' as const },
      highLevel: makeCompleteHighLevelAnalysis(),
    };
    const secondRead = createDeferred<number[]>();
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({
        conversion_mode: 'direct',
        enhanced_mode: true,
      })),
      loadScanResult: vi.fn().mockResolvedValue([preview]),
      loadTrackAnalyses: vi.fn().mockResolvedValue([cached]),
      readAudioFile: vi.fn().mockReturnValue(secondRead.promise),
      getEssentiaModelStatus: vi.fn().mockResolvedValue({
        version: 'essentia-v2',
        embedding: true,
        genre: true,
        mood: true,
        instrument: true,
        installing: false,
        emotionContinuous: true,
        emotionCluster: true,
        discogsEffnet: {
          embedding: true,
          moodTheme: true,
          approachability: true,
          instrumentation: true,
          timbre: true,
          danceability: true,
        },
      }),
      loadEssentiaModel: vi.fn().mockImplementation((id: string) => Promise.resolve({
        id,
        modelJson: '{}',
        weightData: [],
        classes: [],
        kind: 'embedding' as const,
        version: 'essentia-v2',
      })),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({ conversionMode: 'direct', enhancedMode: true }), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(services.applyTrackAnalysisResults).toHaveBeenCalledTimes(1);
      expect(services.applyTrackAnalysisResults).toHaveBeenCalledWith(
        expect.any(String),
        [expect.objectContaining({
          preview: expect.objectContaining({ candidates: [preview.preview.candidates[0]] }),
        })],
        [cached],
        [],
      );
    });
    await vi.waitFor(() => expect(services.readAudioFile).toHaveBeenCalledWith('/music/in-1/Second.mp3'));

    secondRead.resolve([]);
    await vi.waitFor(() => expect(services.applyTrackAnalysisResults).toHaveBeenCalledTimes(2));
  });

  it('cancels enhanced analysis without applying an incomplete result', async () => {
    const readDeferred = createDeferred<number[]>();
    const analysisWorker = makeMockAnalysisWorker();
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({
        conversion_mode: 'direct',
        enhanced_mode: true,
      })),
      loadScanResult: vi.fn().mockResolvedValue([makePreview(0)]),
      readAudioFile: vi.fn().mockReturnValue(readDeferred.promise),
      createAnalysisWorker: () => analysisWorker,
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({
      conversionMode: 'direct',
      enhancedMode: true,
    }), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-action="cancel-analysis"]')).not.toBeNull());
    (root.querySelector('[data-action="cancel-analysis"]') as HTMLButtonElement).click();

    expect(root.querySelector('[data-role="analysis-message"]')?.textContent).toContain('已取消');
    expect(root.querySelector('[data-action="resume-analysis"]')).not.toBeNull();
    expect(analysisWorker.terminate).toHaveBeenCalledTimes(1);
    readDeferred.resolve([]);
    await vi.waitFor(() => expect(services.applyTrackAnalysisResults).not.toHaveBeenCalled());
    expect(services.saveTrackAnalyses).not.toHaveBeenCalled();
  });

  it('keeps the primary action available for a new conversion when analysis can be resumed', async () => {
    localStorage.setItem('w4dj.resumable-analysis.v1', JSON.stringify({
      batchId: 'old-analysis-batch',
      previews: [makePreview(0)],
    }));
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({
        conversion_mode: 'direct',
        enhanced_mode: true,
      })),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({ conversionMode: 'direct' }), services);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-action="start-all"]')).not.toBeNull();
      expect(root.querySelector('[data-action="resume-analysis"]')).not.toBeNull();
    });
    expect(root.querySelector('.global-action')?.textContent).toContain('开始');

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(services.startScan).toHaveBeenCalledTimes(1));
  });

  it('resumes a cancelled analysis batch without starting a new scan', async () => {
    const readDeferred = createDeferred<number[]>();
    const firstWorker = makeMockAnalysisWorker();
    const secondWorker = makeMockAnalysisWorker();
    const createAnalysisWorker = vi.fn()
      .mockReturnValueOnce(firstWorker)
      .mockReturnValueOnce(secondWorker);
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({
        conversion_mode: 'direct',
        enhanced_mode: true,
      })),
      loadScanResult: vi.fn().mockResolvedValue([makePreview(0)]),
      readAudioFile: vi.fn()
        .mockReturnValueOnce(readDeferred.promise)
        .mockResolvedValue([]),
      createAnalysisWorker,
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({
      conversionMode: 'direct',
      enhancedMode: true,
    }), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-action="cancel-analysis"]')).not.toBeNull());
    (root.querySelector('[data-action="cancel-analysis"]') as HTMLButtonElement).click();
    readDeferred.resolve([]);
    await vi.waitFor(() => expect(root.querySelector('[data-action="resume-analysis"]')).not.toBeNull());

    (root.querySelector('[data-action="resume-analysis"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(createAnalysisWorker).toHaveBeenCalledTimes(2));
    expect(services.startScan).toHaveBeenCalledTimes(1);
    expect(firstWorker.terminate).toHaveBeenCalled();
    expect(secondWorker.start).toHaveBeenCalledTimes(1);
  });

  it('restores the resumable analysis entry after a WebView reload', async () => {
    const readDeferred = createDeferred<number[]>();
    const analysisWorker = makeMockAnalysisWorker();
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({
        conversion_mode: 'direct',
        enhanced_mode: true,
      })),
      loadScanResult: vi.fn().mockResolvedValue([makePreview(0)]),
      readAudioFile: vi.fn().mockReturnValue(readDeferred.promise),
      createAnalysisWorker: () => analysisWorker,
    });
    const firstRoot = document.createElement('div');
    bindApp(firstRoot, makeViewState({ conversionMode: 'direct', enhancedMode: true }), services);

    (firstRoot.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(firstRoot.querySelector('[data-action="cancel-analysis"]')).not.toBeNull());
    (firstRoot.querySelector('[data-action="cancel-analysis"]') as HTMLButtonElement).click();
    readDeferred.resolve([]);
    await vi.waitFor(() => expect(firstRoot.querySelector('[data-action="resume-analysis"]')).not.toBeNull());

    const reloadedRoot = document.createElement('div');
    bindApp(reloadedRoot, makeViewState({ conversionMode: 'direct', enhancedMode: true }), services);
    await vi.waitFor(() => expect(reloadedRoot.querySelector('[data-action="resume-analysis"]')).not.toBeNull());
  });

  it('loads the analysis cache at startup and does not reload it for each scan', async () => {
    const cached = {
      path: '/music/in-1/Song.mp3',
      title: 'Song',
      artist: 'Artist',
      album: '',
      durationSeconds: 180,
      bpm: 140,
      key: 'F',
      scale: 'minor',
      keyStrength: 0.8,
      integratedLoudnessLufs: -8,
      loudnessRangeLu: 4,
      energy: 0.9,
      danceability: 0.7,
      beatPositions: [],
      analyzedAt: '2026-08-06T00:00:00Z',
      analyzer: 'Essentia.js',
      analysisVersion: '0.2.0',
      sourceSizeBytes: 0,
      sourceModifiedAt: null,
      dropAnalysis: { status: 'completed' as const },
      highLevel: makeCompleteHighLevelAnalysis(),
    };
    const loadTrackAnalyses = vi.fn().mockResolvedValue([cached]);
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({
        conversion_mode: 'direct',
        enhanced_mode: true,
      })),
      loadScanResult: vi.fn().mockResolvedValue([makePreview(0)]),
      loadTrackAnalyses,
      getEssentiaModelStatus: vi.fn().mockResolvedValue({
        version: 'essentia-v2',
        embedding: true,
        genre: true,
        mood: true,
        instrument: true,
        installing: false,
        emotionContinuous: true,
        emotionCluster: true,
        discogsEffnet: {
          embedding: true,
          moodTheme: true,
          approachability: true,
          instrumentation: true,
          timbre: true,
          danceability: true,
        },
      }),
      loadEssentiaModel: vi.fn().mockImplementation((id: string) => Promise.resolve({
        id,
        modelJson: '{}',
        weightData: [],
        classes: [],
        kind: 'embedding' as const,
        version: 'essentia-v2',
      })),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({
      conversionMode: 'direct',
      enhancedMode: true,
      neteaseFilenameFormat: 'title_artist',
    }), services);

    await vi.waitFor(() => expect(loadTrackAnalyses).toHaveBeenCalledTimes(1));
    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();

    await vi.waitFor(() => expect(services.applyTrackAnalysisResults).toHaveBeenCalledTimes(1));
    expect(services.readAudioFile).not.toHaveBeenCalled();
    expect(services.saveTrackAnalyses).not.toHaveBeenCalled();
    expect(loadTrackAnalyses).toHaveBeenCalledTimes(1);
  });

  it('reuses an unchanged analysis cache entry only for the same filename setting', () => {
    const cached = {
      path: '/music/Artist - Song.mp3',
      title: 'Song',
      artist: 'Artist',
      album: '',
      durationSeconds: 180,
      bpm: 140,
      key: 'F',
      scale: 'minor',
      keyStrength: 0.8,
      integratedLoudnessLufs: -8,
      loudnessRangeLu: 4,
      energy: 0.9,
      danceability: 0.7,
      beatPositions: [],
      analyzedAt: '2026-08-06T00:00:00Z',
      analyzer: 'Essentia.js',
      analysisVersion: '0.2.0',
      sourceSizeBytes: 1234,
      sourceModifiedAt: 1_754_000_000_000,
      sourceFilenameFormat: 'artist_title' as const,
    };

    const fingerprint = { sizeBytes: 1234, modifiedAt: 1_754_000_000_000 };
    expect(canReuseTrackAnalysis(cached, fingerprint, 'artist_title')).toBe(true);
    expect(canReuseTrackAnalysis(cached, fingerprint, 'title_artist')).toBe(false);
    expect(canReuseTrackAnalysis({ ...cached, sourceSizeBytes: 1235 }, fingerprint, 'artist_title'))
      .toBe(false);

    expect(canReuseTrackAnalysis(cached, fingerprint, 'artist_title', 'essentia-v2', true))
      .toBe(false);
    const withHighLevel = {
      ...cached,
      dropAnalysis: { status: 'completed' as const },
      highLevel: makeCompleteHighLevelAnalysis(),
    };
    expect(canReuseTrackAnalysis(withHighLevel, fingerprint, 'artist_title', 'essentia-v2', true))
      .toBe(true);
    expect(canReuseTrackAnalysis(withHighLevel, fingerprint, 'artist_title', 'essentia-v3', true))
      .toBe(false);
  });

  it('cancels a running scan without starting conversion', async () => {
    const deferred = createDeferred<AppScanProgress>();
    const services = makeMockServices({
      startScan: vi.fn().mockResolvedValue({
        status: 'running', phase: 'scanning_source', processed: 1, total: 100,
        current_file: '/music/one.mp3', message: '正在扫描输入目录',
      }),
      loadScanState: vi.fn().mockReturnValue(deferred.promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-action="cancel-scan"]')).not.toBeNull());
    (root.querySelector('[data-action="cancel-scan"]') as HTMLButtonElement).click();
    expect(services.cancelScan).toHaveBeenCalledTimes(1);
    expect(root.querySelector('[data-role="scan-message"]')?.textContent).toContain('正在取消扫描');
    expect((root.querySelector('[data-action="cancel-scan"]') as HTMLButtonElement).disabled).toBe(true);
    deferred.resolve({
      status: 'cancelled', phase: 'cancelled', processed: 1, total: 100,
      current_file: '', message: '扫描已取消',
    });
    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="scan-modal"]')).toBeNull();
      expect(root.querySelector('[data-role="scan-message"]')?.textContent).toContain('扫描已取消');
      expect(services.startConfirmedSync).not.toHaveBeenCalled();
    });
  });

  it('does not invoke the backend or animation for the already selected mode', async () => {
    const services = makeMockServices();
    const root = document.createElement('div');
    bindApp(root, makeViewState({ mode: 'compat' }), services);

    (root.querySelector('[data-mode="compat"]') as HTMLButtonElement).click();
    await Promise.resolve();

    expect(services.chooseMode).not.toHaveBeenCalled();
    expect((root.querySelector('.app-shell') as HTMLElement | null)?.dataset.selectionMotion).not.toBe('mode');
  });

  it('serializes rapid WAV and AIFF selection clicks', async () => {
    const deferred = createDeferred<DesktopState>();
    const services = makeMockServices({
      chooseLosslessFormat: vi.fn().mockReturnValue(deferred.promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({ mode: 'lossless', losslessFormat: 'wav' }), services);
    const formatRow = root.querySelector('.format-row');
    const wavButton = root.querySelector('[data-format="wav"]');
    const aiffButton = root.querySelector('[data-format="aiff"]');

    (root.querySelector('[data-format="aiff"]') as HTMLButtonElement).click();
    expect(root.querySelector('.format-row')).toBe(formatRow);
    expect(root.querySelector('[data-format="wav"]')).toBe(wavButton);
    expect(root.querySelector('[data-format="aiff"]')).toBe(aiffButton);
    expect((wavButton as HTMLButtonElement).disabled).toBe(false);
    expect((wavButton as HTMLButtonElement).getAttribute('aria-disabled')).toBe('true');
    (wavButton as HTMLButtonElement).click();
    expect(services.chooseLosslessFormat).toHaveBeenCalledTimes(1);

    deferred.resolve(makeDesktopState({ mode: 'lossless', lossless_format: 'aiff' }));
    await vi.waitFor(() => {
      expect(root.querySelector('.format-row')).toBe(formatRow);
      expect(root.querySelector('[data-format="wav"]')).toBe(wavButton);
      expect(root.querySelector('[data-format="aiff"]')).toBe(aiffButton);
      expect(root.querySelector('.format-row')?.getAttribute('data-selected-format')).toBe('aiff');
      expect((aiffButton as HTMLButtonElement).getAttribute('aria-disabled')).toBe('false');
    });
  });

  it('keeps the lossless selector stable during a quick WAV/AIFF reversal', async () => {
    const toAiff = createDeferred<DesktopState>();
    const toWav = createDeferred<DesktopState>();
    const services = makeMockServices({
      chooseLosslessFormat: vi.fn((format: AppLosslessFormat) =>
        (format === 'aiff' ? toAiff : toWav).promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({ mode: 'lossless', losslessFormat: 'wav' }), services);
    const formatRow = root.querySelector('.format-row');
    const wavButton = root.querySelector('[data-format="wav"]');
    const aiffButton = root.querySelector('[data-format="aiff"]');

    (aiffButton as HTMLButtonElement).click();
    toAiff.resolve(makeDesktopState({ mode: 'lossless', lossless_format: 'aiff' }));
    await vi.waitFor(() => {
      expect(root.querySelector('.format-row')?.getAttribute('data-selected-format')).toBe('aiff');
    });

    (wavButton as HTMLButtonElement).click();
    toWav.resolve(makeDesktopState({ mode: 'lossless', lossless_format: 'wav' }));
    await vi.waitFor(() => {
      expect(root.querySelector('.format-row')).toBe(formatRow);
      expect(root.querySelector('[data-format="wav"]')).toBe(wavButton);
      expect(root.querySelector('[data-format="aiff"]')).toBe(aiffButton);
      expect(root.querySelector('.format-row')?.getAttribute('data-selected-format')).toBe('wav');
    });
  });

  it('renders history and opens the same preview modal for failed retries', async () => {
    const services = makeMockServices({
      loadHistory: vi.fn().mockResolvedValue([makeHistoryEntry()]),
      retryHistoryFailures: vi.fn().mockResolvedValue({
        ...makePreview(0),
        retry_of: 'history-1',
      }),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="history"]')?.textContent).toContain('重试失败项目');
      expect(root.querySelector('[data-action="export-error-report"]')?.textContent)
        .toContain('导出错误报告');
    });
    (root.querySelector('[data-action="retry-history"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(services.retryHistoryFailures).toHaveBeenCalledWith('history-1');
      expect(root.querySelector('[data-role="preview-modal"]')).not.toBeNull();
    });
  });

  it('places conversion history below the task cards and lets it collapse', async () => {
    const services = makeMockServices({
      loadHistory: vi.fn().mockResolvedValue([makeHistoryEntry()]),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => {
      const history = root.querySelector('[data-role="history"]') as HTMLDetailsElement;
      expect(history).not.toBeNull();
      expect(history.parentElement?.classList.contains('workbench-main')).toBe(true);
      expect(history.open).toBe(false);
      expect(history.querySelector('.history-count')).toBeNull();
      expect(history.querySelector('summary')?.textContent).not.toContain('0');
    });

    const history = root.querySelector('[data-role="history"]') as HTMLDetailsElement;
    (history.querySelector('summary') as HTMLElement).click();
    expect(history.open).toBe(true);

    (root.querySelector('[data-action="toggle-theme"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect((root.querySelector('[data-role="history"]') as HTMLDetailsElement).open).toBe(true);
    });
  });

  it('allows an interrupted task with pending files to export an error report', async () => {
    const pending = makePreview(0).preview.candidates[0];
    const services = makeMockServices({
      loadHistory: vi.fn().mockResolvedValue([makeHistoryEntry({
        failed_count: 0,
        failed_files: [],
        pending_files: [pending],
        status: 'partial',
      })]),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-action="export-error-report"]')?.textContent)
        .toContain('导出错误报告');
    });
  });

  it('manually exports an error report and refreshes the saved history path', async () => {
    const saveFile = vi.fn().mockResolvedValue('/tmp/W4DJ-error-report-history-1.txt');
    const exportHistoryErrorReport = vi.fn().mockResolvedValue(undefined);
    const loadHistory = vi.fn().mockResolvedValue([makeHistoryEntry()]);
    const alert = vi.spyOn(window, 'alert').mockImplementation(() => undefined);
    const services = makeMockServices({ saveFile, exportHistoryErrorReport, loadHistory });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-action="export-error-report"]')).not.toBeNull();
    });
    (root.querySelector('[data-action="export-error-report"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(saveFile).toHaveBeenCalledWith({
        defaultPath: 'W4DJ-error-report-history-1.txt',
        title: '保存错误报告',
      });
      expect(exportHistoryErrorReport).toHaveBeenCalledWith(
        'history-1',
        '/tmp/W4DJ-error-report-history-1.txt',
      );
      expect(loadHistory.mock.calls.length).toBeGreaterThanOrEqual(2);
      expect(alert).toHaveBeenCalledWith(
        '错误报告已导出：/tmp/W4DJ-error-report-history-1.txt',
      );
    });
    alert.mockRestore();
  });

  it('manually exports the runtime session through a separate save dialog', async () => {
    const saveFile = vi.fn().mockResolvedValue('/tmp/W4DJ-runtime-history-1.json');
    const exportRuntimeSession = vi.fn().mockResolvedValue(undefined);
    const alert = vi.spyOn(window, 'alert').mockImplementation(() => undefined);
    const services = makeMockServices({
      saveFile,
      exportRuntimeSession,
      loadHistory: vi.fn().mockResolvedValue([makeHistoryEntry({
        analysis: {
          status: 'partial',
          total: 2,
          completed: 1,
          failed: 0,
          timedOut: 1,
          pending: 0,
        },
      })]),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-action="export-runtime-session"]')).not.toBeNull();
      expect(root.querySelector('.history-analysis-summary')?.textContent).toContain('超时 1');
    });
    (root.querySelector('[data-action="export-runtime-session"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(saveFile).toHaveBeenCalledWith({
        defaultPath: 'W4DJ-runtime-session-history-1.json',
        title: '保存运行会话记录',
      });
      expect(exportRuntimeSession).toHaveBeenCalledWith(
        'history-1',
        '/tmp/W4DJ-runtime-history-1.json',
      );
    });
    alert.mockRestore();
  });

  it('does not call the exporter when the save dialog is cancelled', async () => {
    const saveFile = vi.fn().mockResolvedValue(null);
    const exportHistoryErrorReport = vi.fn().mockResolvedValue(undefined);
    const services = makeMockServices({
      saveFile,
      exportHistoryErrorReport,
      loadHistory: vi.fn().mockResolvedValue([makeHistoryEntry()]),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-action="export-error-report"]')).not.toBeNull();
    });
    (root.querySelector('[data-action="export-error-report"]') as HTMLButtonElement).click();

    await vi.waitFor(() => expect(saveFile).toHaveBeenCalled());
    expect(exportHistoryErrorReport).not.toHaveBeenCalled();
  });

  it('reports a manual export failure without changing conversion slot state', async () => {
    const saveFile = vi.fn().mockResolvedValue('/tmp/report.txt');
    const exportHistoryErrorReport = vi.fn().mockRejectedValue(new Error('disk is full'));
    const alert = vi.spyOn(window, 'alert').mockImplementation(() => undefined);
    const services = makeMockServices({
      saveFile,
      exportHistoryErrorReport,
      loadHistory: vi.fn().mockResolvedValue([makeHistoryEntry()]),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-action="export-error-report"]')).not.toBeNull();
    });
    (root.querySelector('[data-action="export-error-report"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(alert).toHaveBeenCalledWith('错误报告导出失败：disk is full');
    });
    expect(root.querySelector('[data-role="sync-slot"][data-slot="0"]')?.getAttribute('data-status'))
      .not.toBe('error');
    alert.mockRestore();
  });

  it('opens About while keeping cancellation in the global action only', async () => {
    const running = makeDesktopStateWithSlot(0, { status: 'running' });
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(running),
      cancelAllSync: vi.fn().mockResolvedValue(
        makeDesktopStateWithSlot(0, { status: 'cancelled' }),
      ),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => expect(root.querySelector('[data-action="cancel-slot"]')).toBeNull());
    (root.querySelector('[data-action="open-about"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-role="about-modal"]')).not.toBeNull());
    (root.querySelector('[data-action="close-about"]') as HTMLButtonElement).click();
    (root.querySelector('[data-action="cancel-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(services.cancelAllSync).toHaveBeenCalled());
  });

  it('opens the tutorial help and keeps the onboarding guide there', async () => {
    const root = document.createElement('div');
    bindApp(root, makeViewState(), makeMockServices());

    (root.querySelector('[data-action="open-help"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-role="help-modal"]')).not.toBeNull());
    expect(root.querySelector('[data-role="help-modal"] .help-dialog-icon')).toBeNull();
    expect(root.querySelector('[data-role="help-modal"]')?.textContent).toContain('普通转换');
    expect(root.querySelector('[data-role="help-modal"]')?.textContent).toContain('增强转换');
    expect(root.querySelector('[data-role="help-modal"]')?.textContent).not.toContain('Essentia');
    expect(root.querySelector('[data-role="help-modal"] [data-action="reopen-onboarding"]')).not.toBeNull();

    (root.querySelector('[data-action="reopen-onboarding"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-role="onboarding-modal"]')).not.toBeNull());
    expect(root.querySelector('[data-role="help-modal"]')).toBeNull();
  });

  it('closes help and about dialogs when clicking their blurred backdrop', async () => {
    const root = document.createElement('div');
    bindApp(root, makeViewState(), makeMockServices());

    (root.querySelector('[data-action="open-help"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-role="help-modal"]')).not.toBeNull());
    root.querySelector<HTMLElement>('[data-role="help-modal"]')?.dispatchEvent(
      new MouseEvent('click', { bubbles: true }),
    );
    await vi.waitFor(() => expect(root.querySelector('[data-role="help-modal"]')).toBeNull());

    (root.querySelector('[data-action="open-about"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-role="about-modal"]')).not.toBeNull());
    root.querySelector<HTMLElement>('[data-role="about-modal"]')?.dispatchEvent(
      new MouseEvent('click', { bubbles: true }),
    );
    await vi.waitFor(() => expect(root.querySelector('[data-role="about-modal"]')).toBeNull());
  });

  it('shows a history read error instead of replacing a damaged history with an empty state', async () => {
    const services = makeMockServices({
      loadHistory: vi.fn().mockRejectedValue(new Error('invalid history JSON')),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => {
      expect(root.querySelector('.history-error')?.textContent).toContain('转换历史读取失败');
      expect(root.querySelector('.history-empty')).toBeNull();
    });
  });

  it('does not expose enhanced cache controls while the feature bundle is hidden', async () => {
    const services = makeMockServices();
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    expect(root.querySelector('[data-action="clear-analysis-cache"]')).toBeNull();
    expect(services.clearTrackAnalyses).not.toHaveBeenCalled();
  });

  it('does not expose scan cache controls while the feature bundle is hidden', async () => {
    const services = makeMockServices();
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    expect(root.querySelector('[data-action="clear-scan-cache"]')).toBeNull();
    expect(services.clearTrackAnalyses).not.toHaveBeenCalled();
    expect(services.clearScanCache).not.toHaveBeenCalled();
  });

  it('deletes one history entry and clears all history', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    const services = makeMockServices({
      loadHistory: vi
        .fn()
        .mockResolvedValueOnce([makeHistoryEntry()])
        .mockResolvedValue([]),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => expect(root.querySelector('[data-action="delete-history"]')).not.toBeNull());
    (root.querySelector('[data-action="delete-history"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(services.deleteHistoryEntry).toHaveBeenCalledWith('history-1'));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining('不会被删除'));

    // Re-render with an entry so the clear action is visible independently.
    (services.loadHistory as ReturnType<typeof vi.fn>).mockResolvedValue([makeHistoryEntry()]);
    await Promise.resolve();
    const secondRoot = document.createElement('div');
    bindApp(secondRoot, makeViewState(), services);
    await vi.waitFor(() => expect(secondRoot.querySelector('[data-action="clear-history"]')).not.toBeNull());
    (secondRoot.querySelector('[data-action="clear-history"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(services.clearHistory).toHaveBeenCalledTimes(1));
    confirm.mockRestore();
  });

  it('starts and pauses both configured tasks from one global button', async () => {
    const services = makeMockServices({
      previewAllSync: vi.fn().mockResolvedValue(makePreviewResponse()),
      startConfirmedSync: vi
        .fn()
        .mockResolvedValue(makeDesktopState({
          slots: [
            makeDesktopSlot({ status: 'running', progress_total: 5 }),
            makeDesktopSlot({ status: 'running', progress_total: 7 }),
          ],
        })),
      pauseAllSync: vi.fn().mockResolvedValue(makeDesktopState({
        slots: [
          makeDesktopSlot({ status: 'paused' }),
          makeDesktopSlot({ status: 'paused' }),
        ],
      })),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="preview-modal"]')).not.toBeNull();
      expect(services.startConfirmedSync).not.toHaveBeenCalled();
    });
    (root.querySelector('[data-action="confirm-start"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(services.startConfirmedSync).toHaveBeenCalledTimes(1);
      expect(root.querySelector('[data-action="cancel-all"]')).not.toBeNull();
      expect(root.querySelectorAll('[data-status="running"][data-role="sync-slot"]')).toHaveLength(2);
    });

    (root.querySelector('[data-action="cancel-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(services.cancelAllSync).toHaveBeenCalledTimes(1));
  });

  it('ignores repeated global start clicks while the first start is pending', async () => {
    const deferred = createDeferred<AppScanProgress>();
    const services = makeMockServices({
      startScan: vi.fn().mockReturnValue(deferred.promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-action="cancel-scan"]')).not.toBeNull());
    const pendingButton = root.querySelector('[data-action="cancel-scan"]') as HTMLButtonElement;
    expect(pendingButton.disabled).toBe(false);
    pendingButton.click();

    await vi.waitFor(() => expect(services.startScan).toHaveBeenCalledTimes(1));

    deferred.resolve({
      status: 'completed', phase: 'completed', processed: 2, total: 2,
      current_file: '', message: '扫描完成',
    });

    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="preview-modal"]')).not.toBeNull();
    });
  });

  it('toggles and persists the color theme', async () => {
    const root = document.createElement('div');
    bindApp(root, makeViewState(), makeMockServices());

    (root.querySelector('[data-action="toggle-theme"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(localStorage.getItem('w4dj_theme')).toBe('dark');
      expect(root.querySelector('.app-shell')?.getAttribute('data-theme')).toBe('dark');
    });
  });

  it('toggles the whole interface language and persists it', async () => {
    const root = document.createElement('div');
    bindApp(
      root,
      makeViewStateWithSlot(1, { destinationDirectory: '' }, { mode: 'lossless' }),
      makeMockServices(),
    );

    (root.querySelector('[data-action="toggle-lang"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(localStorage.getItem('w4dj_lang')).toBe('en');
      expect(root.textContent).toContain('If I Were a DJ');
      expect(root.textContent).toContain('Use output directory 1');
      expect(root.querySelector('[data-role="control-panel"]')?.getAttribute('aria-label')).toBe(
        'Control panel',
      );
      expect(root.querySelector('.format-row')?.getAttribute('aria-label')).toBe('Lossless format');
    });
  });

  it('reports an action error on only the affected slot', async () => {
    const services = makeMockServices({
      startScan: vi.fn().mockRejectedValue(new Error('Sync failed dramatically')),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-role="scan-message"]')?.textContent)
        .toContain('Sync failed dramatically');
      expect(root.querySelector('[data-role="scan-modal"]')).toBeNull();
    });
  });
});
