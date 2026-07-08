export type AppMode = 'compat' | 'lossless';
export type AppLosslessFormat = 'wav' | 'flac' | 'aiff';
export type AppStatus = 'idle' | 'running' | 'paused' | 'completed' | 'error';

export type AppViewState = {
  sourceDirectory: string;
  destinationDirectory: string;
  mode: AppMode;
  losslessFormat: AppLosslessFormat | null;
  status: AppStatus;
  progressText: string;
  currentFile: string;
  logExpanded: boolean;
  logs: string[];
};

const defaultState: AppViewState = {
  sourceDirectory: '',
  destinationDirectory: '',
  mode: 'compat',
  losslessFormat: null,
  status: 'idle',
  progressText: 'Ready',
  currentFile: '',
  logExpanded: false,
  logs: ['Desktop shell ready'],
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
  root.innerHTML = `
    <section class="hero-panel" aria-label="W4DJ">
      <div class="title-row">
        <div>
          <p class="eyebrow">W4DJ</p>
          <h1>CloudMusic Sync</h1>
        </div>
        <span class="status-pill" data-status="${state.status}">${statusLabels[state.status]}</span>
      </div>

      <div class="quick-start">
        <label class="path-field" data-role="source-picker">
          <span>原始目录</span>
          <button type="button" class="path-button" data-action="pick-source">
            ${displayPath(state.sourceDirectory)}
          </button>
        </label>

        <label class="path-field" data-role="destination-picker">
          <span>输出目录</span>
          <button type="button" class="path-button" data-action="pick-destination">
            ${displayPath(state.destinationDirectory)}
          </button>
        </label>

        <div class="mode-row" data-role="mode-switch" aria-label="Mode">
          <button type="button" class="mode-button ${state.mode === 'compat' ? 'selected' : ''}" data-mode="compat">
            兼容模式
          </button>
          <button type="button" class="mode-button ${state.mode === 'lossless' ? 'selected' : ''}" data-mode="lossless">
            无损模式
          </button>
        </div>

        ${renderLosslessFormats(state)}

        <button type="button" class="primary-action" data-action="${state.status === 'running' ? 'pause' : 'start'}">
          ${state.status === 'running' ? '暂停' : '开始'}
        </button>
      </div>
    </section>

    <footer class="status-strip" data-role="status-strip">
      <button type="button" class="status-toggle" data-action="toggle-log">
        <span>${state.progressText}</span>
        <span>${state.currentFile || latestLog(state.logs)}</span>
      </button>
    </footer>

    <section class="log-drawer" data-role="log-drawer" aria-label="Logs">
      ${state.logs.map((line) => `<p>${escapeHtml(line)}</p>`).join('')}
    </section>
  `;

  const drawer = root.querySelector('[data-role="log-drawer"]') as HTMLElement;
  drawer.hidden = !state.logExpanded;

  return root;
}

export function bindApp(root: HTMLElement, initialState: AppViewState = defaultState): void {
  root.replaceChildren(renderApp(initialState));
}

function renderLosslessFormats(state: AppViewState): string {
  if (state.mode !== 'lossless') {
    return '';
  }

  const formats: AppLosslessFormat[] = ['wav', 'flac', 'aiff'];
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

function displayPath(path: string): string {
  return escapeHtml(path || '选择文件夹');
}

function latestLog(logs: string[]): string {
  return escapeHtml(logs.at(-1) || 'Ready');
}

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;');
}
