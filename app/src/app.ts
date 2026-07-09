import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

export type AppMode = 'compat' | 'lossless';
export type AppLosslessFormat = 'wav' | 'aiff';
export type AppStatus = 'idle' | 'running' | 'paused' | 'completed' | 'error';

export type AppViewState = {
  sourceDirectory: string;
  destinationDirectory: string;
  mode: AppMode;
  losslessFormat: AppLosslessFormat | null;
  status: AppStatus;
  progressTotal: number;
  progressCompleted: number;
  progressText: string;
  currentFile: string;
  logExpanded: boolean;
  logs: string[];
};

export type DesktopState = {
  source_directory: string;
  destination_directory: string;
  mode: AppMode;
  lossless_format: AppLosslessFormat | null;
  status: AppStatus;
  progress_total: number;
  progress_completed: number;
  current_file: string;
  logs: string[];
};

export type AppServices = {
  loadDesktopState: () => Promise<DesktopState>;
  pickDirectory: (kind: 'source' | 'destination') => Promise<string | null>;
  selectSourceDirectory: (path: string) => Promise<DesktopState>;
  selectDestinationDirectory: (path: string) => Promise<DesktopState>;
  chooseMode: (mode: AppMode) => Promise<DesktopState>;
  chooseLosslessFormat: (format: AppLosslessFormat | null) => Promise<DesktopState>;
  startSync: () => Promise<DesktopState>;
  pauseSync: () => Promise<DesktopState>;
};

const defaultState: AppViewState = {
  sourceDirectory: '',
  destinationDirectory: '',
  mode: 'compat',
  losslessFormat: null,
  status: 'idle',
  progressTotal: 0,
  progressCompleted: 0,
  progressText: 'Ready',
  currentFile: '',
  logExpanded: false,
  logs: ['Desktop shell ready'],
};

const defaultServices: AppServices = {
  loadDesktopState: () => invoke<DesktopState>('load_desktop_state'),
  pickDirectory: async (kind) => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: kind === 'source' ? '选择网易云下载目录' : '选择输出目录',
    });

    return typeof selected === 'string' ? selected : null;
  },
  selectSourceDirectory: (path) => invoke<DesktopState>('select_source_directory', { path }),
  selectDestinationDirectory: (path) =>
    invoke<DesktopState>('select_destination_directory', { path }),
  chooseMode: (mode) => invoke<DesktopState>('choose_mode', { mode }),
  chooseLosslessFormat: (format) =>
    invoke<DesktopState>('choose_lossless_format', { format }),
  startSync: () => invoke<DesktopState>('start_sync', { totalFiles: 0 }),
  pauseSync: () => invoke<DesktopState>('pause_sync'),
};

const statusLabels: Record<AppStatus, string> = {
  idle: 'Ready',
  running: 'Running',
  paused: 'Paused',
  completed: 'Completed',
  error: 'Error',
};

export function renderApp(state: AppViewState = defaultState): HTMLElement {
  const root = document.createElement('main');
  root.className = 'app-shell';
  root.dataset.status = state.status;
  root.innerHTML = `
    <header class="topbar">
      <div class="brand-block">
        <p class="eyebrow">W4DJ RKB</p>
        <h1>如果我是DJ</h1>
      </div>
    </header>

    <section class="panel sidebar-panel" data-role="control-panel" aria-label="Controls">
      <div class="panel-head">
        <span class="panel-kicker">原始目录：网易云下载目录</span>
        <span class="panel-caption">输出目录：导出歌曲的存放目录</span>
      </div>

      <div class="control-stack">
        <label class="path-field" data-role="source-picker">
          <span>原始目录</span>
          <button type="button" class="path-button" data-action="pick-source">
            ${icon('folder')}
            ${displayPath(state.sourceDirectory)}
          </button>
        </label>

        <label class="path-field" data-role="destination-picker">
          <span>输出目录</span>
          <button type="button" class="path-button" data-action="pick-destination">
            ${icon('export')}
            ${displayPath(state.destinationDirectory)}
          </button>
        </label>

        <div class="mode-row" data-role="mode-switch" aria-label="Mode">
          <button type="button" class="mode-button ${state.mode === 'compat' ? 'selected' : ''}" data-mode="compat">
            ${icon('check')}
            兼容模式
          </button>
          <button type="button" class="mode-button ${state.mode === 'lossless' ? 'selected' : ''}" data-mode="lossless">
            ${icon('disc')}
            无损模式
          </button>
        </div>

        ${renderLosslessFormats(state)}

        <button type="button" class="primary-action" data-action="${state.status === 'running' ? 'pause' : 'start'}">
          ${state.status === 'running' ? icon('pause') : icon('play')}
          ${state.status === 'running' ? '暂停' : '开始'}
        </button>
      </div>

      <div class="sidebar-note">
        <p>兼容模式：最高输出320kbps mp3</p>
        <p>无损模式：最高输出24bit 48000hz（兼容CDJ-350/XDJ-700及以后机型）</p>
      </div>
    </section>

    <footer class="status-strip" data-role="status-strip">
      <button type="button" class="status-toggle" data-action="toggle-log">
        ${icon('list')}
        <span class="status-copy progress-copy">${state.progressText}</span>
        <span class="current-track">${escapeHtml(state.currentFile || latestLog(state.logs))}</span>
      </button>
      <div class="progress-track" aria-hidden="true">
        <div class="progress-fill" style="width: ${progressPercent(state)}%"></div>
      </div>
    </footer>

    <section class="log-drawer" data-role="log-drawer" aria-label="Logs">
      ${state.logs.map((line) => `<p>${escapeHtml(line)}</p>`).join('')}
    </section>
  `;

  const drawer = root.querySelector('[data-role="log-drawer"]') as HTMLElement;
  drawer.hidden = !state.logExpanded;

  return root;
}

export function bindApp(
  root: HTMLElement,
  initialState: AppViewState = defaultState,
  services: AppServices = defaultServices,
): void {
  let state = initialState;
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;

  const render = () => {
    root.replaceChildren(renderApp(state));
  };

  const queueRefresh = () => {
    if (refreshTimer || state.status !== 'running') {
      return;
    }

    refreshTimer = setTimeout(() => {
      refreshTimer = null;
      void runAction(() => services.loadDesktopState());
    }, 750);
  };

  const applyDesktopState = (desktopState: DesktopState) => {
    state = {
      ...toViewState(desktopState),
      logExpanded: state.logExpanded,
    };
    render();
    queueRefresh();
  };

  const runAction = async (action: () => Promise<DesktopState | void>) => {
    try {
      const nextState = await action();
      if (nextState) {
        applyDesktopState(nextState);
      }
    } catch (error) {
      state = {
        ...state,
        status: 'error',
        progressText: 'Error',
        logs: [...state.logs, error instanceof Error ? error.message : String(error)],
      };
      render();
    }
  };

  root.addEventListener('click', (event) => {
    const target = event.target as HTMLElement | null;
    const button = target?.closest<HTMLButtonElement>('button');

    if (!button) {
      return;
    }

    const action = button.dataset.action;
    const mode = button.dataset.mode as AppMode | undefined;
    const format = button.dataset.format as AppLosslessFormat | undefined;

    if (action === 'toggle-log') {
      state = { ...state, logExpanded: !state.logExpanded };
      render();
      return;
    }

    if (action === 'pick-source') {
      void runAction(async () => {
        const path = await services.pickDirectory('source');
        return path ? services.selectSourceDirectory(path) : undefined;
      });
      return;
    }

    if (action === 'pick-destination') {
      void runAction(async () => {
        const path = await services.pickDirectory('destination');
        return path ? services.selectDestinationDirectory(path) : undefined;
      });
      return;
    }

    if (mode) {
      void runAction(() => services.chooseMode(mode));
      return;
    }

    if (format) {
      void runAction(() => services.chooseLosslessFormat(format));
      return;
    }

    if (action === 'start') {
      void runAction(() => services.startSync());
      return;
    }

    if (action === 'pause') {
      void runAction(() => services.pauseSync());
    }
  });

  render();
  void runAction(() => services.loadDesktopState());
}

function renderLosslessFormats(state: AppViewState): string {
  if (state.mode !== 'lossless') {
    return '';
  }

  const formats: AppLosslessFormat[] = ['wav', 'aiff'];
  return `
    <div class="format-row" aria-label="Lossless format">
      ${formats
        .map(
          (format) => `
            <button type="button" class="format-button ${state.losslessFormat === format ? 'selected' : ''}" data-format="${format}">
              ${format.toUpperCase()}
            </button>
          `,
        )
        .join('')}
    </div>
  `;
}

function renderArchiveRows(state: AppViewState): string {
  const rows = [
    {
      field: 'SOURCE',
      value: state.sourceDirectory || '未选择',
      note: '网易云下载目录',
    },
    {
      field: 'DEST',
      value: state.destinationDirectory || '未选择',
      note: '用户自定义输出',
    },
    {
      field: 'MODE',
      value: describeMode(state.mode),
      note: state.mode === 'compat' ? 'MP3 兼容输出' : 'WAV / AIFF 无损输出',
    },
    {
      field: 'FORMAT',
      value: describeFormat(state.losslessFormat),
      note: '仅无损模式有效',
    },
    {
      field: 'STATUS',
      value: statusLabels[state.status],
      note: state.status === 'running' ? '同步进行中' : '待命',
    },
    {
      field: 'PROGRESS',
      value: state.progressText,
      note: '档案处理进度',
    },
    {
      field: 'CURRENT',
      value: state.currentFile || '—',
      note: '当前处理文件',
    },
    {
      field: 'LOGS',
      value: String(state.logs.length).padStart(2, '0'),
      note: latestLog(state.logs),
    },
  ];

  return rows
    .map(
      (row, index) => `
        <div class="archive-row ${row.field === 'CURRENT' && state.currentFile ? 'is-active' : ''}" role="row">
          <span class="archive-index">${String(index + 1).padStart(2, '0')}</span>
          <span class="archive-field">${escapeHtml(row.field)}</span>
          <span class="archive-value">${escapeHtml(row.value)}</span>
          <span class="archive-note">${escapeHtml(row.note)}</span>
        </div>
      `,
    )
    .join('');
}

function displayPath(path: string): string {
  return escapeHtml(path || '选择文件夹');
}

function latestLog(logs: string[]): string {
  return logs.length > 0 ? logs[logs.length - 1] : 'Ready';
}

function detailTrackTitle(state: AppViewState): string {
  return state.currentFile || 'No track selected';
}

function detailTrackArtist(state: AppViewState): string {
  if (state.status === 'running') {
    return 'Sync Engine';
  }

  return 'Unknown Artist';
}

function detailQuality(state: AppViewState): string {
  if (state.mode === 'compat') {
    return 'MP3 320kbps';
  }

  return state.losslessFormat ? state.losslessFormat.toUpperCase() : 'WAV / AIFF';
}

function toViewState(state: DesktopState): AppViewState {
  return {
    sourceDirectory: state.source_directory,
    destinationDirectory: state.destination_directory,
    mode: state.mode,
    losslessFormat: state.lossless_format,
    status: state.status,
    progressTotal: state.progress_total,
    progressCompleted: state.progress_completed,
    progressText: formatProgress(state),
    currentFile: state.current_file,
    logExpanded: false,
    logs: state.logs,
  };
}

function formatProgress(state: DesktopState): string {
  if (state.progress_total > 0) {
    return `${state.progress_completed}/${state.progress_total}`;
  }

  return statusLabels[state.status];
}

function progressPercent(state: AppViewState): number {
  if (state.progressTotal <= 0) {
    return 0;
  }

  return Math.min(
    100,
    Math.max(0, Math.round((state.progressCompleted / state.progressTotal) * 100)),
  );
}

function renderStatusPill(state: AppViewState): string {
  if (state.status === 'completed') {
    return '';
  }

  return `<span class="status-pill" data-status="${state.status}">${statusLabels[state.status]}</span>`;
}

function describeMode(mode: AppMode): string {
  return mode === 'compat' ? '兼容模式' : '无损模式';
}

function describeFormat(format: AppLosslessFormat | null): string {
  if (!format) {
    return 'AUTO';
  }

  return format.toUpperCase();
}

function icon(name: 'folder' | 'export' | 'check' | 'disc' | 'play' | 'pause' | 'list' | 'grid' | 'compact' | 'stack' | 'filter'): string {
  const icons: Record<'folder' | 'export' | 'check' | 'disc' | 'play' | 'pause' | 'list' | 'grid' | 'compact' | 'stack' | 'filter', string> =
    {
      folder: `
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path d="M2.5 5.1h3.4l1.1 1.2h6.5v5.2H2.5z" />
          <path d="M2.5 4.5h3.2l1.3 1.2" />
        </svg>
      `,
      export: `
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path d="M3 12.2h10" />
          <path d="M8 4v6.1" />
          <path d="M5.6 6.4 8 4l2.4 2.4" />
        </svg>
      `,
      check: `
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path d="M3.3 8.5 6.4 11.4 12.8 4.7" />
        </svg>
      `,
      disc: `
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <circle cx="8" cy="8" r="5.1" />
          <circle cx="8" cy="8" r="1" />
        </svg>
      `,
      play: `
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path d="M5.2 4v8l6.6-4z" />
        </svg>
      `,
      pause: `
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path d="M5.1 4.2v7.6" />
          <path d="M10.9 4.2v7.6" />
        </svg>
      `,
      list: `
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path d="M5 4.7h8" />
          <path d="M5 8h8" />
          <path d="M5 11.3h8" />
          <path d="M2.7 4.7h.5" />
          <path d="M2.7 8h.5" />
          <path d="M2.7 11.3h.5" />
        </svg>
      `,
      grid: `
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path d="M3.5 3.5h2.8v2.8H3.5z" />
          <path d="M9.7 3.5h2.8v2.8H9.7z" />
          <path d="M3.5 9.7h2.8v2.8H3.5z" />
          <path d="M9.7 9.7h2.8v2.8H9.7z" />
        </svg>
      `,
      compact: `
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path d="M3.5 4.2h9" />
          <path d="M3.5 8h9" />
          <path d="M3.5 11.8h9" />
        </svg>
      `,
      stack: `
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path d="M4 4.5h8" />
          <path d="M3 7.9h10" />
          <path d="M4.5 11.3h7" />
        </svg>
      `,
      filter: `
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path d="M3.2 4.5h9.6" />
          <path d="M5.6 8h4.8" />
          <path d="M6.8 11.5h2.4" />
        </svg>
      `,
    };

  return `<span class="ui-icon ui-icon-${name}">${icons[name]}</span>`;
}

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;');
}
