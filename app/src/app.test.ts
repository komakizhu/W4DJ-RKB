import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  bindApp,
  renderApp,
  type AppHistoryEntry,
  type AppPreview,
  type AppServices,
  type AppSyncSlotViewState,
  type AppViewState,
  type DesktopState,
  type DesktopSyncSlotState,
  type SyncSlotIndex,
} from './app';

beforeEach(() => {
  localStorage.clear();
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
  ...overrides,
});

const makeDesktopState = (overrides: Partial<DesktopState> = {}): DesktopState => ({
  slots: [
    makeDesktopSlot({ source_directory: '/music/in-1', destination_directory: '/music/out-1' }),
    makeDesktopSlot({ source_directory: '/music/in-2', destination_directory: '/music/out-2' }),
  ],
  mode: 'compat',
  lossless_format: null,
  conflict_strategy: 'skip',
  filename_rule: 'title_artist',
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
  logExpanded: false,
  logs: ['Ready'],
  ...overrides,
});

const makeViewState = (overrides: Partial<AppViewState> = {}): AppViewState => ({
  slots: [
    makeViewSlot({ sourceDirectory: '/music/in-1', destinationDirectory: '/music/out-1' }),
    makeViewSlot({ sourceDirectory: '/music/in-2', destinationDirectory: '/music/out-2' }),
  ],
  mode: 'compat',
  losslessFormat: null,
  conflictStrategy: 'skip',
  filenameRule: 'title_artist',
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
  status: 'partial',
  retry_of: null,
  conflict_strategy: 'skip',
  filename_rule: 'title_artist',
  ...overrides,
});

const makeMockServices = (overrides: Partial<AppServices> = {}): AppServices => ({
  loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState()),
  pickDirectory: vi.fn().mockResolvedValue(null),
  selectSourceDirectory: vi.fn().mockResolvedValue(makeDesktopState()),
  selectDestinationDirectory: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseMode: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseLosslessFormat: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseConflictStrategy: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseFilenameRule: vi.fn().mockResolvedValue(makeDesktopState()),
  previewAllSync: vi.fn().mockResolvedValue(makePreviewResponse()),
  startConfirmedSync: vi.fn().mockResolvedValue(makeDesktopState({
    slots: [
      makeDesktopSlot({ status: 'running', progress_total: 2 }),
      makeDesktopSlot({ status: 'running', progress_total: 2 }),
    ],
  })),
  loadHistory: vi.fn().mockResolvedValue([]),
  retryHistoryFailures: vi.fn().mockResolvedValue(makePreview(0)),
  exportHistoryErrorReport: vi.fn().mockResolvedValue(undefined),
  deleteHistoryEntry: vi.fn().mockResolvedValue(undefined),
  clearHistory: vi.fn().mockResolvedValue(undefined),
  loadAppInfo: vi.fn().mockResolvedValue({
    version: '2.2.0',
    developer: 'komakizhu',
    project_url: 'https://github.com/komakizhu/W4DJ-RKB',
  }),
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
    expect(root.querySelectorAll('[data-role="log-drawer"][hidden]')).toHaveLength(2);
    expect(root.querySelector('.rail-copy')).toBeNull();
  });

  it('renders new and skipped track counts in the global status card', () => {
    const root = renderApp(
      makeViewState({
        slots: [
          makeViewSlot({ newTracks: 3, skippedTracks: 1 }),
          makeViewSlot({ newTracks: 2, skippedTracks: 4 }),
        ],
      }),
    );

    const status = root.querySelector('.global-status-card') as HTMLElement;
    expect(status.textContent).toContain('新增歌曲');
    expect(status.textContent).toContain('5');
    expect(status.textContent).toContain('跳过歌曲');
    expect(status.textContent).toContain('5');
  });

  it('keeps planned, completed, skipped, and error counts independent', () => {
    const root = renderApp(
      makeViewState({
        slots: [
          makeViewSlot({
            newTracks: 5,
            progressCompleted: 2,
            skippedTracks: 3,
            errorTracks: 1,
          }),
          makeViewSlot({
            newTracks: 4,
            progressCompleted: 1,
            skippedTracks: 2,
            errorTracks: 2,
          }),
        ],
      }),
    );

    const status = root.querySelector('.global-status-card') as HTMLElement;
    expect(Array.from(status.querySelectorAll('dd')).map((item) => item.textContent)).toEqual([
      '2/2',
      '3',
      '9',
      '5',
      '3',
    ]);
    expect(status.textContent).toContain('错误文件');
  });

  it('renders the selected color theme and a top-right theme toggle', () => {
    const root = renderApp(makeViewState({ theme: 'dark' }));

    expect(root.dataset.theme).toBe('dark');
    expect(root.dataset.lightPalette).toBe('c');
    expect(root.querySelector('[data-action="toggle-theme"]')).not.toBeNull();
    expect(root.querySelector('.topbar-actions')?.lastElementChild?.getAttribute('data-action'))
      .toBe('toggle-lang');
  });

  it('renders the global lossless format selector only in lossless mode', () => {
    const compatRoot = renderApp(makeViewState({ mode: 'compat' }));
    expect(compatRoot.querySelector('.format-row')).toBeNull();
    expect(compatRoot.querySelector('.format-slot')).not.toBeNull();

    const root = renderApp(makeViewState({ mode: 'lossless', losslessFormat: 'wav' }));
    expect(root.querySelector('.format-slot')).not.toBeNull();
    expect(root.querySelector('[data-format="wav"]')?.classList.contains('selected')).toBe(true);
    expect(root.querySelector('[data-format="aiff"]')?.classList.contains('selected')).toBe(false);
  });

  it('renders conflict and filename settings with safe defaults', () => {
    const root = renderApp(makeViewState());

    expect((root.querySelector('[data-action="choose-conflict"]') as HTMLSelectElement).value)
      .toBe('skip');
    expect((root.querySelector('[data-action="choose-filename-rule"]') as HTMLSelectElement).value)
      .toBe('title_artist');
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

  it('renders version, developer, project, and support details in About', () => {
    const root = renderApp(
      makeViewState(),
      null,
      null,
      null,
      [],
      null,
      false,
      {
      version: '2.2.0',
        developer: 'komakizhu',
        project_url: 'https://github.com/komakizhu/W4DJ-RKB',
      },
    );

    expect(root.querySelector('[data-role="about-modal"]')?.textContent).toContain('v2.2.0');
    expect(root.querySelector('[data-role="about-modal"]')?.textContent).toContain('komakizhu');
    expect(root.querySelector('[data-role="about-modal"] a')?.getAttribute('href')).toBe('https://github.com/komakizhu/W4DJ-RKB');
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
    expect(root.querySelector('[data-action="pause-all"]')).not.toBeNull();
    expect((slotTwo.querySelector('.progress-fill') as HTMLElement).style.width).toBe('45%');
    expect(slotTwo.querySelector('.current-track')?.textContent).toBe('track02.wav');
  });

  it('shows a localized destination fallback hint for slot two', () => {
    const root = renderApp(
      makeViewStateWithSlot(1, { destinationDirectory: '' }),
    );

    const hint = root.querySelector('[data-role="fallback-hint"][data-slot="1"]');
    expect(hint?.textContent).toContain('使用输出目录 1');
    expect(hint?.textContent).toContain('/music/out-1');
  });

  it('unhides only the selected slot log drawer', () => {
    const root = renderApp(
      makeViewStateWithSlot(1, { logExpanded: true, logs: ['Slot 2 line'] }),
    );

    expect((root.querySelector('[data-role="log-drawer"][data-slot="0"]') as HTMLElement).hidden)
      .toBe(true);
    const drawer = root.querySelector(
      '[data-role="log-drawer"][data-slot="1"]',
    ) as HTMLElement;
    expect(drawer.hidden).toBe(false);
    expect(drawer.textContent).toContain('Slot 2 line');
  });
});

describe('bindApp', () => {
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

  it('toggles only slot two log drawer', async () => {
    const root = document.createElement('div');
    bindApp(root, makeViewState(), makeMockServices());

    const toggle = root.querySelector(
      '[data-action="toggle-log"][data-slot="1"]',
    ) as HTMLButtonElement;
    toggle.click();

    await vi.waitFor(() => {
      expect(
        (root.querySelector('[data-role="log-drawer"][data-slot="0"]') as HTMLElement).hidden,
      ).toBe(true);
      expect(
        (root.querySelector('[data-role="log-drawer"][data-slot="1"]') as HTMLElement).hidden,
      ).toBe(false);
    });
  });

  it('selects slot two source directory with its slot index', async () => {
    const services = makeMockServices({
      pickDirectory: vi.fn().mockResolvedValue('/new/source-2'),
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
      expect(services.pickDirectory).toHaveBeenCalledWith('source', 1);
      expect(services.selectSourceDirectory).toHaveBeenCalledWith(1, '/new/source-2');
      expect(root.textContent).toContain('/new/source-2');
    });
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

    (root.querySelector('[data-mode="lossless"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('.format-row')).not.toBeNull());

    (root.querySelector('[data-format="aiff"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(services.chooseMode).toHaveBeenCalledWith('lossless');
      expect(services.chooseLosslessFormat).toHaveBeenCalledWith('aiff');
    });
  });

  it('persists conflict and filename selections through backend services', async () => {
    const services = makeMockServices({
      chooseConflictStrategy: vi.fn().mockResolvedValue(
        makeDesktopState({ conflict_strategy: 'rename' }),
      ),
      chooseFilenameRule: vi.fn().mockResolvedValue(
        makeDesktopState({ filename_rule: 'artist_title' }),
      ),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const conflict = root.querySelector('[data-action="choose-conflict"]') as HTMLSelectElement;
    conflict.value = 'rename';
    conflict.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(services.chooseConflictStrategy).toHaveBeenCalledWith('rename'));

    const filename = root.querySelector('[data-action="choose-filename-rule"]') as HTMLSelectElement;
    filename.value = 'artist_title';
    filename.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(services.chooseFilenameRule).toHaveBeenCalledWith('artist_title'));
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

  it('does not invoke the backend or animation for the already selected mode', async () => {
    const services = makeMockServices();
    const root = document.createElement('div');
    bindApp(root, makeViewState({ mode: 'compat' }), services);

    (root.querySelector('[data-mode="compat"]') as HTMLButtonElement).click();
    await Promise.resolve();

    expect(services.chooseMode).not.toHaveBeenCalled();
    expect(root.querySelector('.app-shell')?.dataset.selectionMotion).not.toBe('mode');
  });

  it('serializes rapid WAV and AIFF selection clicks', async () => {
    const deferred = createDeferred<DesktopState>();
    const services = makeMockServices({
      chooseLosslessFormat: vi.fn().mockReturnValue(deferred.promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState({ mode: 'lossless', losslessFormat: 'wav' }), services);

    (root.querySelector('[data-format="aiff"]') as HTMLButtonElement).click();
    const wavButton = root.querySelector('[data-format="wav"]') as HTMLButtonElement;
    expect(wavButton.disabled).toBe(true);
    wavButton.click();
    expect(services.chooseLosslessFormat).toHaveBeenCalledTimes(1);

    deferred.resolve(makeDesktopState({ mode: 'lossless', lossless_format: 'aiff' }));
    await vi.waitFor(() => expect(root.querySelector('[data-format="aiff"]')).not.toBeNull());
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
    });
    (root.querySelector('[data-action="retry-history"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(services.retryHistoryFailures).toHaveBeenCalledWith('history-1');
      expect(root.querySelector('[data-role="preview-modal"]')).not.toBeNull();
    });
  });

  it('opens About and can cancel a running slot', async () => {
    const running = makeDesktopStateWithSlot(0, { status: 'running' });
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(running),
      cancelSync: vi.fn().mockResolvedValue(
        makeDesktopStateWithSlot(0, { status: 'cancelled' }),
      ),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    await vi.waitFor(() => expect(root.querySelector('[data-action="cancel-slot"]')).not.toBeNull());
    (root.querySelector('[data-action="open-about"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('[data-role="about-modal"]')).not.toBeNull());
    (root.querySelector('[data-action="close-about"]') as HTMLButtonElement).click();
    (root.querySelector('[data-action="cancel-slot"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(services.cancelSync).toHaveBeenCalledWith(0));
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
      expect(root.querySelector('[data-action="pause-all"]')).not.toBeNull();
      expect(root.querySelectorAll('[data-status="running"][data-role="sync-slot"]')).toHaveLength(2);
    });

    (root.querySelector('[data-action="pause-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(services.pauseAllSync).toHaveBeenCalledTimes(1));
  });

  it('ignores repeated global start clicks while the first start is pending', async () => {
    const deferred = createDeferred<AppPreview[]>();
    const services = makeMockServices({
      previewAllSync: vi.fn().mockReturnValue(deferred.promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    const pendingButton = root.querySelector('[data-action="start-all"]') as HTMLButtonElement;
    expect(pendingButton.disabled).toBe(true);
    pendingButton.click();

    expect(services.previewAllSync).toHaveBeenCalledTimes(1);

    deferred.resolve(makePreviewResponse());

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
      previewAllSync: vi.fn().mockRejectedValue(new Error('Sync failed dramatically')),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(
        (root.querySelector('[data-role="sync-slot"][data-slot="0"]') as HTMLElement).dataset
          .status,
      ).toBe('error');
      expect(
        (root.querySelector('[data-role="sync-slot"][data-slot="1"]') as HTMLElement).dataset
          .status,
      ).toBe('error');
      expect(
        root.querySelector('[data-role="log-drawer"][data-slot="1"]')?.textContent,
      ).toContain('Sync failed dramatically');
    });
  });
});
