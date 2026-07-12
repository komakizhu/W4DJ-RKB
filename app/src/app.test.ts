import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  bindApp,
  renderApp,
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

const makeMockServices = (overrides: Partial<AppServices> = {}): AppServices => ({
  loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState()),
  pickDirectory: vi.fn().mockResolvedValue(null),
  selectSourceDirectory: vi.fn().mockResolvedValue(makeDesktopState()),
  selectDestinationDirectory: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseMode: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseLosslessFormat: vi.fn().mockResolvedValue(makeDesktopState()),
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

  it('starts and pauses both configured tasks from one global button', async () => {
    const services = makeMockServices({
      startAllSync: vi
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
      expect(services.startAllSync).toHaveBeenCalledTimes(1);
      expect(root.querySelector('[data-action="pause-all"]')).not.toBeNull();
      expect(root.querySelectorAll('[data-status="running"][data-role="sync-slot"]')).toHaveLength(2);
    });

    (root.querySelector('[data-action="pause-all"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(services.pauseAllSync).toHaveBeenCalledTimes(1));
  });

  it('ignores repeated global start clicks while the first start is pending', async () => {
    const deferred = createDeferred<DesktopState>();
    const services = makeMockServices({
      startAllSync: vi.fn().mockReturnValue(deferred.promise),
    });
    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
    const pendingButton = root.querySelector('[data-action="start-all"]') as HTMLButtonElement;
    expect(pendingButton.disabled).toBe(true);
    pendingButton.click();

    expect(services.startAllSync).toHaveBeenCalledTimes(1);

    deferred.resolve(
      makeDesktopState({
        slots: [
          makeDesktopSlot({ status: 'running', progress_total: 10 }),
          makeDesktopSlot({ status: 'running', progress_total: 8 }),
        ],
      }),
    );

    await vi.waitFor(() => {
      expect(root.querySelector('[data-action="pause-all"]')).not.toBeNull();
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
      startAllSync: vi.fn().mockRejectedValue(new Error('Sync failed dramatically')),
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
