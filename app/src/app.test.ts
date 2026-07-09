import { describe, expect, it, vi } from 'vitest';
import { bindApp, renderApp, type AppServices, type AppViewState, type DesktopState } from './app';

const makeDesktopState = (overrides: Partial<DesktopState> = {}): DesktopState => ({
  source_directory: '/music/in',
  destination_directory: '/music/out',
  mode: 'compat',
  lossless_format: null,
  status: 'idle',
  progress_total: 0,
  progress_completed: 0,
  current_file: '',
  logs: ['Ready'],
  ...overrides,
});

const makeViewState = (overrides: Partial<AppViewState> = {}): AppViewState => ({
  sourceDirectory: '/music/in',
  destinationDirectory: '/music/out',
  mode: 'compat',
  losslessFormat: null,
  status: 'idle',
  progressTotal: 0,
  progressCompleted: 0,
  progressText: 'Ready',
  currentFile: '',
  logExpanded: false,
  logs: ['Ready'],
  ...overrides,
});

const makeMockServices = (overrides: Partial<AppServices> = {}): AppServices => ({
  loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState()),
  pickDirectory: vi.fn().mockResolvedValue(null),
  selectSourceDirectory: vi.fn().mockResolvedValue(makeDesktopState()),
  selectDestinationDirectory: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseMode: vi.fn().mockResolvedValue(makeDesktopState()),
  chooseLosslessFormat: vi.fn().mockResolvedValue(makeDesktopState()),
  startSync: vi.fn().mockResolvedValue(makeDesktopState({ status: 'running', progress_total: 10 })),
  pauseSync: vi.fn().mockResolvedValue(makeDesktopState({ status: 'paused' })),
  ...overrides,
});

describe('renderApp', () => {
  it('renders standard layout elements and initial state', () => {
    const root = renderApp(makeViewState());

    expect(root.querySelector('h1')?.textContent).toBe('如果我是DJ');
    expect(root.querySelector('[data-role="source-picker"]')).not.toBeNull();
    expect(root.querySelector('[data-role="destination-picker"]')).not.toBeNull();
    expect(root.querySelector('[data-role="mode-switch"]')).not.toBeNull();
    expect(root.querySelector('[data-role="status-strip"]')).not.toBeNull();
    expect(root.querySelector('[data-role="log-drawer"]')).not.toBeNull();

    const primaryBtn = root.querySelector('.primary-action') as HTMLButtonElement;
    expect(primaryBtn.dataset.action).toBe('start');
    expect(primaryBtn.textContent).toContain('开始');

    const drawer = root.querySelector('[data-role="log-drawer"]') as HTMLElement;
    expect(drawer.hidden).toBe(true);
  });

  it('renders lossless format selector when mode is lossless', () => {
    const compatRoot = renderApp(makeViewState({ mode: 'compat' }));
    expect(compatRoot.querySelector('.format-row')).toBeNull();

    const losslessRoot = renderApp(makeViewState({ mode: 'lossless', losslessFormat: 'wav' }));
    const formatRow = losslessRoot.querySelector('.format-row');
    expect(formatRow).not.toBeNull();

    const wavBtn = losslessRoot.querySelector('[data-format="wav"]') as HTMLButtonElement;
    const aiffBtn = losslessRoot.querySelector('[data-format="aiff"]') as HTMLButtonElement;
    expect(wavBtn.classList.contains('selected')).toBe(true);
    expect(aiffBtn.classList.contains('selected')).toBe(false);
  });

  it('shows running status, pause action, and progress bar', () => {
    const root = renderApp(
      makeViewState({
        status: 'running',
        progressTotal: 100,
        progressCompleted: 45,
        progressText: '45/100',
        currentFile: 'track01.mp3',
      }),
    );

    const primaryBtn = root.querySelector('.primary-action') as HTMLButtonElement;
    expect(primaryBtn.dataset.action).toBe('pause');
    expect(primaryBtn.textContent).toContain('暂停');

    const progressFill = root.querySelector('.progress-fill') as HTMLElement;
    expect(progressFill.style.width).toBe('45%');

    const currentTrack = root.querySelector('.current-track');
    expect(currentTrack?.textContent).toBe('track01.mp3');
  });

  it('unhides the log drawer when logExpanded is true', () => {
    const root = renderApp(
      makeViewState({
        logExpanded: true,
        logs: ['Line 1', 'Line 2'],
      }),
    );

    const drawer = root.querySelector('[data-role="log-drawer"]') as HTMLElement;
    expect(drawer.hidden).toBe(false);
    expect(drawer.textContent).toContain('Line 1');
    expect(drawer.textContent).toContain('Line 2');
  });
});

describe('bindApp', () => {
  it('calls loadDesktopState on initialization and renders resolved state', async () => {
    const services = makeMockServices({
      loadDesktopState: vi.fn().mockResolvedValue(
        makeDesktopState({
          source_directory: '/loaded/source',
          destination_directory: '/loaded/dest',
        }),
      ),
    });

    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    expect(services.loadDesktopState).toHaveBeenCalled();

    // Wait for async loadDesktopState promise resolution
    await vi.waitFor(() => {
      expect(root.textContent).toContain('/loaded/source');
      expect(root.textContent).toContain('/loaded/dest');
    });
  });

  it('toggles log drawer visibility when status-toggle button is clicked', async () => {
    const root = document.createElement('div');
    bindApp(root, makeViewState({ logExpanded: false }), makeMockServices());

    await vi.waitFor(() => {
      const drawer = root.querySelector('[data-role="log-drawer"]') as HTMLElement;
      expect(drawer.hidden).toBe(true);
    });

    const toggleBtn = root.querySelector('[data-action="toggle-log"]') as HTMLButtonElement;
    toggleBtn.click();

    await vi.waitFor(() => {
      const drawer = root.querySelector('[data-role="log-drawer"]') as HTMLElement;
      expect(drawer.hidden).toBe(false);
    });

    const toggleBtn2 = root.querySelector('[data-action="toggle-log"]') as HTMLButtonElement;
    toggleBtn2.click();

    await vi.waitFor(() => {
      const drawer = root.querySelector('[data-role="log-drawer"]') as HTMLElement;
      expect(drawer.hidden).toBe(true);
    });
  });

  it('triggers pickDirectory and selectSourceDirectory on pick-source action', async () => {
    const services = makeMockServices({
      pickDirectory: vi.fn().mockResolvedValue('/new/source/path'),
      selectSourceDirectory: vi.fn().mockResolvedValue(
        makeDesktopState({ source_directory: '/new/source/path' }),
      ),
    });

    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const pickBtn = root.querySelector('[data-action="pick-source"]') as HTMLButtonElement;
    pickBtn.click();

    await vi.waitFor(() => {
      expect(services.pickDirectory).toHaveBeenCalledWith('source');
      expect(services.selectSourceDirectory).toHaveBeenCalledWith('/new/source/path');
      expect(root.textContent).toContain('/new/source/path');
    });
  });

  it('triggers chooseMode and chooseLosslessFormat when buttons are clicked', async () => {
    const services = makeMockServices({
      chooseMode: vi.fn().mockResolvedValue(
        makeDesktopState({ mode: 'lossless', lossless_format: 'wav' }),
      ),
      chooseLosslessFormat: vi.fn().mockResolvedValue(
        makeDesktopState({ mode: 'lossless', lossless_format: 'aiff' }),
      ),
    });

    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const losslessModeBtn = root.querySelector('[data-mode="lossless"]') as HTMLButtonElement;
    losslessModeBtn.click();

    await vi.waitFor(() => {
      expect(services.chooseMode).toHaveBeenCalledWith('lossless');
      expect(root.querySelector('.format-row')).not.toBeNull();
    });

    const aiffBtn = root.querySelector('[data-format="aiff"]') as HTMLButtonElement;
    aiffBtn?.click();

    await vi.waitFor(() => {
      expect(services.chooseLosslessFormat).toHaveBeenCalledWith('aiff');
    });
  });

  it('triggers startSync and pauseSync actions', async () => {
    const services = makeMockServices({
      startSync: vi.fn().mockResolvedValue(
        makeDesktopState({ status: 'running', progress_total: 5 }),
      ),
      pauseSync: vi.fn().mockResolvedValue(
        makeDesktopState({ status: 'paused' }),
      ),
    });

    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const startBtn = root.querySelector('[data-action="start"]') as HTMLButtonElement;
    startBtn.click();

    await vi.waitFor(() => {
      expect(services.startSync).toHaveBeenCalled();
      const pauseBtn = root.querySelector('[data-action="pause"]') as HTMLButtonElement;
      expect(pauseBtn).not.toBeNull();
    });

    const pauseBtn = root.querySelector('[data-action="pause"]') as HTMLButtonElement;
    pauseBtn.click();

    await vi.waitFor(() => {
      expect(services.pauseSync).toHaveBeenCalled();
    });
  });

  it('catches action errors and transitions to error status', async () => {
    const services = makeMockServices({
      startSync: vi.fn().mockRejectedValue(new Error('Sync failed dramatically')),
    });

    const root = document.createElement('div');
    bindApp(root, makeViewState(), services);

    const startBtn = root.querySelector('[data-action="start"]') as HTMLButtonElement;
    startBtn.click();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-status="error"]')).not.toBeNull();
      expect(root.textContent).toContain('Error');
      const drawer = root.querySelector('[data-role="log-drawer"]');
      expect(drawer?.textContent).toContain('Sync failed dramatically');
    });
  });
});
