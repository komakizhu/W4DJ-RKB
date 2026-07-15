import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow, type DragDropEvent } from '@tauri-apps/api/window';

export type AppMode = 'compat' | 'lossless';
export type AppLosslessFormat = 'wav' | 'aiff';
export type AppConflictStrategy = 'skip' | 'overwrite' | 'rename' | 'update_metadata';
export type AppFilenameRule = 'title_artist' | 'artist_title' | 'original';
export type AppStatus = 'idle' | 'running' | 'paused' | 'completed' | 'error' | 'cancelled';
export type AppLanguage = 'zh' | 'en';
export type AppTheme = 'light' | 'dark';
export type SyncSlotIndex = 0 | 1;
type SelectionMotion = 'mode' | 'format' | 'theme' | 'lang' | null;
type PendingSelection = 'mode' | 'format' | null;

const LIGHT_PALETTE = 'c' as const;

export type AppSyncSlotViewState = {
  sourceDirectory: string;
  destinationDirectory: string;
  status: AppStatus;
  progressTotal: number;
  progressCompleted: number;
  newTracks: number;
  skippedTracks: number;
  errorTracks: number;
  progressText: string;
  currentFile: string;
  logExpanded: boolean;
  logs: string[];
};

export type AppViewState = {
  slots: [AppSyncSlotViewState, AppSyncSlotViewState];
  mode: AppMode;
  losslessFormat: AppLosslessFormat | null;
  conflictStrategy: AppConflictStrategy;
  filenameRule: AppFilenameRule;
  lang: AppLanguage;
  theme: AppTheme;
};

export type DesktopSyncSlotState = {
  source_directory: string;
  destination_directory: string;
  status: AppStatus;
  progress_total: number;
  progress_completed: number;
  new_tracks: number;
  skipped_tracks: number;
  existing_tracks: number;
  error_tracks: number;
  estimated_output_bytes: number | null;
  failed_files: AppFailedFile[];
  current_file: string;
  logs: string[];
};

export type DesktopState = {
  slots: [DesktopSyncSlotState, DesktopSyncSlotState];
  mode: AppMode;
  lossless_format: AppLosslessFormat | null;
  conflict_strategy: AppConflictStrategy;
  filename_rule: AppFilenameRule;
};

export type AppErrorCategory =
  | 'file_damaged'
  | 'unsupported_format'
  | 'ffmpeg'
  | 'output_permission'
  | 'disk_space'
  | 'invalid_filename'
  | 'unknown';

export type AppFailedFile = {
  name: string;
  source_path: string;
  destination_path: string;
  message: string;
  category: AppErrorCategory;
};

export type AppPreviewCandidate = {
  name: string;
  source_path: string;
  destination_path: string;
  source_size_bytes: number;
  estimated_output_bytes: number | null;
  operation: 'convert' | 'update_metadata';
};

export type AppPreviewIssue = {
  path: string;
  message: string;
};

export type AppSyncPreview = {
  source_directory: string;
  destination_directory: string;
  new_count: number;
  existing_count: number;
  skipped_count: number;
  error_count: number;
  estimated_output_bytes: number | null;
  candidates: AppPreviewCandidate[];
  skipped: AppPreviewIssue[];
  errors: AppPreviewIssue[];
  warnings: AppPreviewIssue[];
  available_space_bytes: number | null;
  disk_space_sufficient: boolean | null;
};

export type AppPreview = {
  slot_index: SyncSlotIndex;
  mode: AppMode;
  lossless_format: AppLosslessFormat | null;
  conflict_strategy: AppConflictStrategy;
  filename_rule: AppFilenameRule;
  preview: AppSyncPreview;
  retry_of: string | null;
};

export type AppInfo = {
  version: string;
  developer: string;
  project_url: string;
};

export type AppHistoryStatus = 'completed' | 'partial' | 'cancelled' | 'error';

export type AppHistoryEntry = {
  id: string;
  batch_id: string;
  slot_index: number;
  started_at: string;
  finished_at: string;
  duration_seconds: number;
  source_directory: string;
  destination_directory: string;
  mode: AppMode;
  lossless_format: AppLosslessFormat | null;
  new_count: number;
  existing_count: number;
  skipped_count: number;
  error_count: number;
  completed_count: number;
  failed_count: number;
  failed_files: AppFailedFile[];
  pending_files: AppPreviewCandidate[];
  status: AppHistoryStatus;
  retry_of: string | null;
  conflict_strategy: AppConflictStrategy;
  filename_rule: AppFilenameRule;
};

export type AppPreviewModalState = {
  previews: AppPreview[];
  retryOf: string | null;
};

export type AppServices = {
  loadDesktopState: () => Promise<DesktopState>;
  pickDirectory: (
    kind: 'source' | 'destination',
    slotIndex: SyncSlotIndex,
  ) => Promise<string | null>;
  selectSourceDirectory: (slotIndex: SyncSlotIndex, path: string) => Promise<DesktopState>;
  selectDestinationDirectory: (slotIndex: SyncSlotIndex, path: string) => Promise<DesktopState>;
  chooseMode: (mode: AppMode) => Promise<DesktopState>;
  chooseLosslessFormat: (format: AppLosslessFormat | null) => Promise<DesktopState>;
  chooseConflictStrategy: (strategy: AppConflictStrategy) => Promise<DesktopState>;
  chooseFilenameRule: (rule: AppFilenameRule) => Promise<DesktopState>;
  previewAllSync: () => Promise<AppPreview[]>;
  startConfirmedSync: (previews: AppPreview[], retryOf?: string | null) => Promise<DesktopState>;
  loadHistory: () => Promise<AppHistoryEntry[]>;
  retryHistoryFailures: (id: string) => Promise<AppPreview>;
  exportHistoryErrorReport: (id: string, path: string) => Promise<void>;
  deleteHistoryEntry: (id: string) => Promise<void>;
  clearHistory: () => Promise<void>;
  loadAppInfo: () => Promise<AppInfo>;
  openExternalUrl: (url: string) => Promise<void>;
  startAllSync: () => Promise<DesktopState>;
  pauseAllSync: () => Promise<DesktopState>;
  cancelSync: (slotIndex: SyncSlotIndex) => Promise<DesktopState>;
  cancelAllSync: () => Promise<DesktopState>;
};

const translations = {
  zh: {
    eyebrow: 'W4DJ RKB',
    title: '如果我是DJ',
    railLead: '输出模式',
    sourceKicker: '歌曲下载目录（网易云、SoundCloud 等）',
    destKicker: '任务 1 / 任务 2 独立运行，窗口较小时可滚动',
    sourceLabel: '歌曲下载目录',
    destLabel: '输出目录',
    clearSource: '清空输入目录',
    clearDestination: '清空输出目录',
    pickFolder: '选择文件夹',
    compatMode: '兼容模式',
    losslessMode: '无损模式',
    compatNote: '兼容模式：最高输出 320kbps MP3',
    losslessNote: '无损模式：最高输出 24-bit / 48kHz（兼容 CDJ-350、XDJ-700 及以后机型）',
    startAll: '同时开始',
    pauseAll: '暂停全部',
    idle: '待命',
    running: '运行中',
    paused: '已暂停',
    cancelled: '已取消',
    completed: '已完成',
    error: '错误',
    controlPanel: '控制面板',
    mode: '输出模式',
    logs: '日志',
    losslessFormat: '无损格式',
    syncSlot: '任务',
    fallback: '未单独设置，使用输出目录 1',
    fallbackMissing: '输出目录 1 也未设置',
    noCurrentFile: '等待处理文件',
    globalStatus: '全局状态',
    configuredTasks: '已配置任务',
    completedTracks: '已完成歌曲',
    newTracks: '新增歌曲',
    skippedTracks: '跳过歌曲',
    errorTracks: '错误文件',
    darkTheme: '切换深色模式',
    lightTheme: '切换浅色模式',
    previewTitle: '转换前确认',
    scanning: '正在扫描任务…',
    newFiles: '新增文件',
    existingFiles: '已存在',
    willSkip: '将跳过',
    errorFiles: '错误文件',
    estimatedOutput: '预计输出',
    confirmStart: '确认并开始转换',
    cancel: '取消',
    editBeforeStart: '返回修改',
    noProcessableFiles: '没有可处理的文件',
    history: '转换历史',
    noHistory: '还没有转换记录',
    retryFailures: '重试失败项目',
    exportReport: '导出错误报告',
    completedCount: '完成',
    failedCount: '失败',
    sourcePath: '源目录',
    destinationPath: '输出目录',
    conflictStrategy: '已存在文件',
    conflictSkip: '已存在文件：跳过',
    conflictOverwrite: '已存在文件：覆盖',
    conflictMetadata: '高级选项：仅更新元数据',
    filenameRule: '文件名规则',
    titleArtist: '标题 - 艺术家（默认）',
    artistTitle: '艺术家 - 标题',
    originalName: '保留原文件名',
    availableSpace: '可用空间',
    insufficientSpace: '磁盘空间不足，无法开始转换',
    cancelTask: '取消任务',
    resumeTasks: '继续未完成任务',
    deleteHistory: '删除记录',
    clearHistory: '清空历史',
    about: '关于',
    version: '版本',
    developer: '开发者',
    projectHome: '项目主页',
    close: '关闭',
    pendingCount: '待继续',
    errorCategory: '错误类型',
  },
  en: {
    eyebrow: 'W4DJ RKB',
    title: 'If I Were a DJ',
    railLead: 'Output mode',
    sourceKicker: 'Song folders (NetEase, SoundCloud, etc.)',
    destKicker: 'Task 1 and Task 2 run independently. Scroll when the window is short.',
    sourceLabel: 'Song Folder',
    destLabel: 'Output Folder',
    clearSource: 'Clear input folder',
    clearDestination: 'Clear output folder',
    pickFolder: 'Select Folder',
    compatMode: 'Compat Mode',
    losslessMode: 'Lossless Mode',
    compatNote: 'Compat Mode: Max 320kbps MP3 output',
    losslessNote: 'Lossless Mode: Max 24-bit / 48kHz (CDJ-350, XDJ-700 and later)',
    startAll: 'Start both',
    pauseAll: 'Pause all',
    idle: 'Ready',
    running: 'Running',
    paused: 'Paused',
    cancelled: 'Cancelled',
    completed: 'Completed',
    error: 'Error',
    controlPanel: 'Control panel',
    mode: 'Output mode',
    logs: 'Logs',
    losslessFormat: 'Lossless format',
    syncSlot: 'Task',
    fallback: 'Use output directory 1 when empty',
    fallbackMissing: 'Output directory 1 is also empty',
    noCurrentFile: 'Waiting for a track',
    globalStatus: 'Global status',
    configuredTasks: 'Configured tasks',
    completedTracks: 'Tracks completed',
    newTracks: 'New tracks',
    skippedTracks: 'Skipped tracks',
    errorTracks: 'Error files',
    darkTheme: 'Switch to dark theme',
    lightTheme: 'Switch to light theme',
    previewTitle: 'Confirm conversion',
    scanning: 'Scanning tasks…',
    newFiles: 'New files',
    existingFiles: 'Already exists',
    willSkip: 'Will skip',
    errorFiles: 'Errors',
    estimatedOutput: 'Estimated output',
    confirmStart: 'Confirm and convert',
    cancel: 'Cancel',
    editBeforeStart: 'Edit settings',
    noProcessableFiles: 'No files to process',
    history: 'Conversion history',
    noHistory: 'No conversion history yet',
    retryFailures: 'Retry failed files',
    exportReport: 'Export error report',
    completedCount: 'Completed',
    failedCount: 'Failed',
    sourcePath: 'Source',
    destinationPath: 'Output',
    conflictStrategy: 'Existing files',
    conflictSkip: 'Existing file: skip',
    conflictOverwrite: 'Existing file: overwrite',
    conflictMetadata: 'Advanced: update metadata only',
    filenameRule: 'Filename rule',
    titleArtist: 'Title - Artist (default)',
    artistTitle: 'Artist - Title',
    originalName: 'Keep original filename',
    availableSpace: 'Available space',
    insufficientSpace: 'Not enough disk space to start',
    cancelTask: 'Cancel task',
    resumeTasks: 'Resume unfinished tasks',
    deleteHistory: 'Delete entry',
    clearHistory: 'Clear history',
    about: 'About',
    version: 'Version',
    developer: 'Developer',
    projectHome: 'Project home',
    close: 'Close',
    pendingCount: 'Pending',
    errorCategory: 'Error type',
  },
} as const;

function t(key: keyof typeof translations.zh, lang: AppLanguage): string {
  return translations[lang][key];
}

function defaultSlot(lang: AppLanguage): AppSyncSlotViewState {
  return {
    sourceDirectory: '',
    destinationDirectory: '',
    status: 'idle',
    progressTotal: 0,
    progressCompleted: 0,
    newTracks: 0,
    skippedTracks: 0,
    errorTracks: 0,
    progressText: t('idle', lang),
    currentFile: '',
    logExpanded: false,
    logs: ['Desktop shell ready'],
  };
}

const storedLanguage = localStorage.getItem('w4dj_lang');
const initialLanguage: AppLanguage = storedLanguage === 'en' ? 'en' : 'zh';
const initialTheme: AppTheme = localStorage.getItem('w4dj_theme') === 'dark' ? 'dark' : 'light';

const defaultState: AppViewState = {
  slots: [defaultSlot(initialLanguage), defaultSlot(initialLanguage)],
  mode: 'compat',
  losslessFormat: null,
  conflictStrategy: 'skip',
  filenameRule: 'title_artist',
  lang: initialLanguage,
  theme: initialTheme,
};

const defaultServices: AppServices = {
  loadDesktopState: () => invoke<DesktopState>('load_desktop_state'),
  pickDirectory: async (kind, slotIndex) => {
    const lang = (localStorage.getItem('w4dj_lang') as AppLanguage) || 'zh';
    const slotNumber = slotIndex + 1;
    const title =
      kind === 'source'
        ? lang === 'zh'
          ? `选择歌曲下载目录 ${slotNumber}`
          : `Select song folder ${slotNumber}`
        : lang === 'zh'
          ? `选择输出目录 ${slotNumber}`
          : `Select output folder ${slotNumber}`;
    const selected = await open({
      directory: true,
      multiple: false,
      title,
    });

    return typeof selected === 'string' ? selected : null;
  },
  selectSourceDirectory: (slotIndex, path) =>
    invoke<DesktopState>('select_source_directory', { slotIndex, path }),
  selectDestinationDirectory: (slotIndex, path) =>
    invoke<DesktopState>('select_destination_directory', { slotIndex, path }),
  chooseMode: (mode) => invoke<DesktopState>('choose_mode', { mode }),
  chooseLosslessFormat: (format) =>
    invoke<DesktopState>('choose_lossless_format', { format }),
  chooseConflictStrategy: (strategy) =>
    invoke<DesktopState>('choose_conflict_strategy', { strategy }),
  chooseFilenameRule: (rule) => invoke<DesktopState>('choose_filename_rule', { rule }),
  previewAllSync: () => invoke<AppPreview[]>('preview_all_sync'),
  startConfirmedSync: (previews, retryOf = null) =>
    invoke<DesktopState>('start_confirmed_sync', { previews, retryOf }),
  loadHistory: () => invoke<AppHistoryEntry[]>('load_history'),
  retryHistoryFailures: (id) => invoke<AppPreview>('retry_history_failures', { id }),
  exportHistoryErrorReport: (id, path) =>
    invoke<void>('export_history_error_report', { id, path }),
  deleteHistoryEntry: (id) => invoke<void>('delete_history_entry_command', { id }),
  clearHistory: () => invoke<void>('clear_history_command'),
  loadAppInfo: () => invoke<AppInfo>('app_info'),
  openExternalUrl: (url) => invoke<void>('open_external_url', { url }),
  startAllSync: () => invoke<DesktopState>('start_all_sync'),
  pauseAllSync: () => invoke<DesktopState>('pause_all_sync'),
  cancelSync: (slotIndex) => invoke<DesktopState>('cancel_sync', { slotIndex }),
  cancelAllSync: () => invoke<DesktopState>('cancel_all_sync'),
};

export function renderApp(
  state: AppViewState = defaultState,
  pendingAction: 'start-all' | 'pause-all' | null = null,
  selectionMotion: SelectionMotion = null,
  previewModal: AppPreviewModalState | null = null,
  history: AppHistoryEntry[] = [],
  pendingSelection: PendingSelection = null,
  previewBusy = false,
  aboutInfo: AppInfo | null = null,
): HTMLElement {
  const root = document.createElement('main');
  root.className = 'app-shell';
  root.dataset.status = aggregateStatus(state);
  root.dataset.theme = state.theme;
  root.dataset.lightPalette = LIGHT_PALETTE;
  if (selectionMotion) {
    root.dataset.selectionMotion = selectionMotion;
  }
  const isRunning = state.slots.some((slot) => slot.status === 'running');
  const hasCancelled = state.slots.some((slot) => slot.status === 'cancelled');
  const configuredTasks = state.slots.filter((slot) => slot.sourceDirectory.trim()).length;
  const completedTracks = state.slots.reduce((total, slot) => total + slot.progressCompleted, 0);
  const newTracks = state.slots.reduce((total, slot) => total + slot.newTracks, 0);
  const skippedTracks = state.slots.reduce((total, slot) => total + slot.skippedTracks, 0);
  const errorTracks = state.slots.reduce((total, slot) => total + slot.errorTracks, 0);
  root.innerHTML = `
    <header class="topbar">
      <div class="brand-block">
        <p class="eyebrow">${t('eyebrow', state.lang)}</p>
        <h1>${t('title', state.lang)}</h1>
      </div>
      <div class="topbar-actions">
        <button type="button" class="lang-button" data-action="open-about">${t('about', state.lang)}</button>
        <button type="button" class="theme-button" data-action="toggle-theme" aria-label="${
          state.theme === 'light' ? t('darkTheme', state.lang) : t('lightTheme', state.lang)
        }" title="${state.theme === 'light' ? t('darkTheme', state.lang) : t('lightTheme', state.lang)}">
          ${icon(state.theme === 'light' ? 'moon' : 'sun')}
        </button>
        <button type="button" class="lang-button" data-action="toggle-lang">
          ${state.lang === 'en' ? '中文' : 'EN'}
        </button>
      </div>
    </header>

    <section class="panel control-panel" data-role="control-panel" aria-label="${t('controlPanel', state.lang)}">
      <aside class="workbench-rail" data-role="workbench-rail">
        <div class="global-controls">
          <div class="global-control-head">
            <span>${t('mode', state.lang)}</span>
          </div>
          <div class="mode-row" data-role="mode-switch" data-selected-mode="${state.mode}" aria-label="${t('mode', state.lang)}">
            <button type="button" class="mode-button ${state.mode === 'compat' ? 'selected' : ''}" data-mode="compat" ${pendingSelection === 'mode' ? 'disabled' : ''}>
              ${icon('check')}
              ${t('compatMode', state.lang)}
            </button>
            <button type="button" class="mode-button ${state.mode === 'lossless' ? 'selected' : ''}" data-mode="lossless" ${pendingSelection === 'mode' ? 'disabled' : ''}>
              ${icon('disc')}
              ${t('losslessMode', state.lang)}
            </button>
          </div>
          ${renderLosslessFormats(state, pendingSelection)}
          ${renderOutputSettings(state)}
          <div class="rail-lower">
            <div class="rail-note">
              <p>${t('compatNote', state.lang)}</p>
              <p>${t('losslessNote', state.lang)}</p>
            </div>
            <section class="global-status-card" aria-label="${t('globalStatus', state.lang)}">
              <p class="global-control-head">${t('globalStatus', state.lang)}</p>
              <dl>
                <div><dt>${t('configuredTasks', state.lang)}</dt><dd>${configuredTasks}/2</dd></div>
                <div><dt>${t('completedTracks', state.lang)}</dt><dd>${completedTracks}</dd></div>
                <div><dt>${t('newTracks', state.lang)}</dt><dd class="stat-new">${newTracks}</dd></div>
                <div><dt>${t('skippedTracks', state.lang)}</dt><dd class="stat-skipped">${skippedTracks}</dd></div>
                <div><dt>${t('errorTracks', state.lang)}</dt><dd class="stat-error">${errorTracks}</dd></div>
              </dl>
            </section>
          </div>
          <div class="global-action-group">
            <button type="button" class="global-action" data-action="${isRunning ? 'pause-all' : 'start-all'}" ${
              configuredTasks === 0 || pendingAction !== null ? 'disabled' : ''
            } aria-busy="${pendingAction !== null}">
              ${isRunning ? icon('pause') : icon('play')}
              ${isRunning ? t('pauseAll', state.lang) : hasCancelled ? t('resumeTasks', state.lang) : t('startAll', state.lang)}
            </button>
            ${isRunning ? `<button type="button" class="secondary-action cancel-all" data-action="cancel-all">${t('cancelTask', state.lang)}</button>` : ''}
          </div>
        </div>
      </aside>

      <div class="workbench-main" data-role="workbench-main">
        <div class="workspace-intro">
          <p class="panel-kicker">${t('sourceKicker', state.lang)}</p>
        </div>
        <div class="sync-slots">
          ${renderSyncSlot(state, 0)}
          ${renderSyncSlot(state, 1)}
        </div>
      </div>
    </section>
    ${renderHistory(history, state.lang)}
    ${renderPreviewModal(previewModal, state.lang, previewBusy)}
    ${renderAboutModal(aboutInfo, state.lang)}
  `;

  state.slots.forEach((slot, slotIndex) => {
    const drawer = root.querySelector(
      `[data-role="log-drawer"][data-slot="${slotIndex}"]`,
    ) as HTMLElement;
    drawer.hidden = !slot.logExpanded;
  });

  return root;
}

function renderPreviewModal(
  modal: AppPreviewModalState | null,
  lang: AppLanguage,
  busy = false,
): string {
  if (!modal) {
    return '';
  }

  const processableCount = modal.previews.reduce(
    (total, item) => total + item.preview.candidates.length,
    0,
  );
  const hasEnoughSpace = modal.previews.every(
    (item) => item.preview.disk_space_sufficient !== false,
  );
  const canConfirm = hasEnoughSpace && (processableCount > 0 || previewHasRetryErrors(modal));
  return `
    <div class="preview-modal" data-role="preview-modal" role="dialog" aria-modal="true" aria-label="${t('previewTitle', lang)}">
      <div class="preview-dialog">
        <header class="preview-head">
          <div>
            <p class="panel-kicker">W4DJ RKB</p>
            <h2>${t('previewTitle', lang)}</h2>
          </div>
          <span class="preview-batch-label">${modal.retryOf ? t('retryFailures', lang) : t('startAll', lang)}</span>
        </header>
        <div class="preview-cards">
          ${modal.previews.map((item) => renderPreviewCard(item, lang)).join('')}
        </div>
        ${canConfirm ? '' : `<p class="preview-empty">${t('noProcessableFiles', lang)}</p>`}
        <footer class="preview-actions">
          <button type="button" class="secondary-action" data-action="cancel-preview" ${busy ? 'disabled' : ''}>${t('cancel', lang)}</button>
          <button type="button" class="secondary-action" data-action="edit-preview" ${busy ? 'disabled' : ''}>${t('editBeforeStart', lang)}</button>
          <button type="button" class="global-action preview-confirm" data-action="confirm-start" ${canConfirm && !busy ? '' : 'disabled'}>${busy ? t('scanning', lang) : t('confirmStart', lang)}</button>
        </footer>
      </div>
    </div>
  `;
}

function renderPreviewCard(item: AppPreview, lang: AppLanguage): string {
  const preview = item.preview;
  const issues = [
    ...preview.errors.map(
      (issue) => `<li>${escapeHtml(issue.path)}：${escapeHtml(issue.message)}</li>`,
    ),
    ...preview.warnings.map(
      (issue) => `<li class="preview-warning">${escapeHtml(issue.path)}：${escapeHtml(issue.message)}</li>`,
    ),
  ].join('');
  return `
    <article class="preview-card" data-role="preview-card" data-slot="${item.slot_index}">
      <header class="preview-card-head">
        <div>
          <p class="panel-kicker">${t('syncSlot', lang)} ${item.slot_index + 1}</p>
          <h3>${modeLabel(item.mode, lang)}${item.mode === 'lossless' ? ` · ${(item.lossless_format || 'wav').toUpperCase()}` : ''}</h3>
        </div>
          <div class="preview-estimate"><span>${t('estimatedOutput', lang)}</span><strong>${formatBytes(preview.estimated_output_bytes, lang)}</strong></div>
      </header>
      <dl class="preview-stats">
        <div><dt>${t('newFiles', lang)}</dt><dd>${preview.new_count}</dd></div>
        <div><dt>${t('existingFiles', lang)}</dt><dd>${preview.existing_count}</dd></div>
        <div><dt>${t('willSkip', lang)}</dt><dd>${preview.skipped_count}</dd></div>
        <div><dt>${t('errorFiles', lang)}</dt><dd class="preview-error-count">${preview.error_count}</dd></div>
      </dl>
      <div class="preview-paths">
        <p><span>${t('sourcePath', lang)}</span>${escapeHtml(preview.source_directory)}</p>
        <p><span>${t('destinationPath', lang)}</span>${escapeHtml(preview.destination_directory)}</p>
        ${preview.available_space_bytes == null ? '' : `<p><span>${t('availableSpace', lang)}</span>${formatBytes(preview.available_space_bytes, lang)}</p>`}
      </div>
      ${preview.disk_space_sufficient === false ? `<p class="disk-space-error">${t('insufficientSpace', lang)}</p>` : ''}
      ${issues ? `<ul class="preview-errors">${issues}</ul>` : ''}
    </article>
  `;
}

function previewHasRetryErrors(modal: AppPreviewModalState): boolean {
  return modal.retryOf !== null && modal.previews.some((item) => item.preview.error_count > 0);
}

function renderHistory(entries: AppHistoryEntry[], lang: AppLanguage): string {
  return `
    <section class="history-panel" data-role="history">
      <header class="history-head">
        <div>
          <p class="panel-kicker">W4DJ RKB</p>
          <h2>${t('history', lang)}</h2>
        </div>
        <div class="history-head-actions">
          <span class="history-count">${entries.length}</span>
          ${entries.length > 0 ? `<button type="button" class="secondary-action history-clear" data-action="clear-history">${t('clearHistory', lang)}</button>` : ''}
        </div>
      </header>
      ${entries.length === 0
        ? `<p class="history-empty">${t('noHistory', lang)}</p>`
        : `<div class="history-list">${entries.map((entry) => renderHistoryEntry(entry, lang)).join('')}</div>`}
    </section>
  `;
}

function renderHistoryEntry(entry: AppHistoryEntry, lang: AppLanguage): string {
  const pendingFiles = entry.pending_files || [];
  const failures = entry.failed_files
    .map((failedFile) => `<li><strong>${escapeHtml(failedFile.name)}</strong><span class="failure-category">${t('errorCategory', lang)}：${errorCategoryLabel(failedFile.category, lang)}</span><span>${escapeHtml(failedFile.message)}</span></li>`)
    .join('');
  return `
    <article class="history-entry" data-history-id="${escapeHtml(entry.id)}">
      <header class="history-entry-head">
        <div>
          <strong>${escapeHtml(entry.started_at)}</strong>
          <span class="history-status" data-history-status="${entry.status}">${historyStatusLabel(entry.status, lang)}</span>
        </div>
        <span>${entry.completed_count}/${entry.new_count} · ${entry.failed_count} ${t('failedCount', lang)}${pendingFiles.length > 0 ? ` · ${pendingFiles.length} ${t('pendingCount', lang)}` : ''}</span>
      </header>
      <p class="history-output">${escapeHtml(entry.destination_directory)}</p>
      ${failures ? `<details class="history-failures"><summary>${entry.failed_count} ${t('failedCount', lang)}</summary><ul>${failures}</ul></details>` : ''}
      <footer class="history-entry-actions">
        ${entry.failed_count > 0 || pendingFiles.length > 0 ? `<button type="button" class="secondary-action" data-action="retry-history" data-history-id="${escapeHtml(entry.id)}">${pendingFiles.length > 0 ? t('resumeTasks', lang) : t('retryFailures', lang)}</button>` : ''}
        ${entry.failed_count > 0 ? `<button type="button" class="secondary-action" data-action="export-history" data-history-id="${escapeHtml(entry.id)}">${t('exportReport', lang)}</button>` : ''}
        <button type="button" class="secondary-action history-delete" data-action="delete-history" data-history-id="${escapeHtml(entry.id)}">${t('deleteHistory', lang)}</button>
      </footer>
    </article>
  `;
}

function renderSyncSlot(state: AppViewState, slotIndex: SyncSlotIndex): string {
  const slot = state.slots[slotIndex];
  const fallbackDestination = state.slots[0].destinationDirectory;
  const usesFallback = slotIndex === 1 && slot.destinationDirectory.trim() === '';
  const displayedDestination = usesFallback ? fallbackDestination : slot.destinationDirectory;
  const slotNumber = slotIndex + 1;
  return `
    <article class="sync-slot-card" data-role="sync-slot" data-slot="${slotIndex}" data-status="${slot.status}">
      <header class="sync-slot-head">
        <div>
          <h2>${t('syncSlot', state.lang)} ${slotNumber}</h2>
        </div>
        <div class="slot-head-actions">
          <span class="slot-status" data-status="${slot.status}">${statusLabel(slot.status, state.lang)}</span>
          ${slot.status === 'running' ? `<button type="button" class="secondary-action slot-cancel" data-action="cancel-slot" data-slot="${slotIndex}">${t('cancelTask', state.lang)}</button>` : ''}
        </div>
      </header>

      <div class="path-flow">
          <div class="path-field" data-role="source-picker" data-drop-kind="source" data-slot="${slotIndex}">
          <span>${t('sourceLabel', state.lang)}</span>
          <div class="path-control">
            <button type="button" class="path-button" data-action="pick-source" data-slot="${slotIndex}">
              ${icon('folder')}
              <span class="path-copy">${displayPath(slot.sourceDirectory, state.lang)}</span>
            </button>
            <button type="button" class="path-clear" data-action="clear-source" data-slot="${slotIndex}" aria-label="${t('clearSource', state.lang)}" title="${t('clearSource', state.lang)}" ${slot.sourceDirectory.trim() ? '' : 'disabled'}>
              ${icon('trash')}
            </button>
          </div>
        </div>

        <span class="path-arrow" aria-hidden="true">${icon('arrow')}</span>

          <div class="path-field" data-role="destination-picker" data-drop-kind="destination" data-slot="${slotIndex}">
          <span>${t('destLabel', state.lang)}</span>
          <div class="path-control">
            <button type="button" class="path-button ${usesFallback ? 'is-fallback' : ''}" data-action="pick-destination" data-slot="${slotIndex}">
              ${icon('export')}
              <span class="path-copy">${displayPath(displayedDestination, state.lang)}</span>
            </button>
            <button type="button" class="path-clear" data-action="clear-destination" data-slot="${slotIndex}" aria-label="${t('clearDestination', state.lang)}" title="${t('clearDestination', state.lang)}" ${slot.destinationDirectory.trim() ? '' : 'disabled'}>
              ${icon('trash')}
            </button>
          </div>
          ${
            usesFallback
              ? `<small class="fallback-hint" data-role="fallback-hint" data-slot="1">
                  ${t(fallbackDestination.trim() ? 'fallback' : 'fallbackMissing', state.lang)}${
                    fallbackDestination.trim() ? ` · ${escapeHtml(fallbackDestination)}` : ''
                  }
                </small>`
              : ''
          }
        </div>
      </div>

      <footer class="slot-status-strip">
        <button type="button" class="status-toggle" data-action="toggle-log" data-slot="${slotIndex}">
          ${icon('list')}
          <span class="status-copy progress-copy">${escapeHtml(slot.progressText)}</span>
          <span class="current-track">${escapeHtml(
            slot.currentFile || latestLog(slot.logs, state.lang),
          )}</span>
        </button>
        <div class="progress-track" aria-hidden="true">
          <div class="progress-fill" style="width: ${progressPercent(slot)}%"></div>
        </div>
      </footer>

      <section class="log-drawer" data-role="log-drawer" data-slot="${slotIndex}" aria-label="${t('logs', state.lang)} ${slotNumber}">
        ${slot.logs.map((line) => `<p>${escapeHtml(line)}</p>`).join('')}
      </section>
    </article>
  `;
}

export function bindApp(
  root: HTMLElement,
  initialState: AppViewState = defaultState,
  services: AppServices = defaultServices,
): void {
  let state = initialState;
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;
  let pendingGlobalAction: 'start-all' | 'pause-all' | null = null;
  let selectionMotion: SelectionMotion = null;
  let pendingSelection: PendingSelection = null;
  let previewModal: AppPreviewModalState | null = null;
  let previewBusy = false;
  let history: AppHistoryEntry[] = [];
  let aboutInfo: AppInfo | null = null;

  const render = () => {
    root.replaceChildren(
      renderApp(
        state,
        pendingGlobalAction,
        selectionMotion,
        previewModal,
        history,
        pendingSelection,
        previewBusy,
        aboutInfo,
      ),
    );
  };

  const triggerLocalMotion = (motion: SelectionMotion) => {
    selectionMotion = motion;
    render();
    setTimeout(() => {
      if (selectionMotion === motion) {
        selectionMotion = null;
        render();
      }
    }, 420);
  };

  const queueRefresh = () => {
    if (refreshTimer || !state.slots.some((slot) => slot.status === 'running')) {
      return;
    }

    refreshTimer = setTimeout(() => {
      refreshTimer = null;
      void runAction(() => services.loadDesktopState());
    }, 750);
  };

  const refreshHistory = async (renderAfter = true) => {
    try {
      history = await services.loadHistory();
      if (renderAfter && selectionMotion === null) {
        render();
      }
    } catch (error) {
      console.error('Failed to load conversion history:', error);
    }
  };

  const applyDesktopState = (desktopState: DesktopState) => {
    const nextState = toViewState(desktopState, state.lang, state.theme);
    nextState.slots.forEach((slot, slotIndex) => {
      slot.logExpanded = state.slots[slotIndex].logExpanded;
    });
    state = nextState;
    render();
    void refreshHistory(false);
    queueRefresh();
  };

  const reportError = (error: unknown, errorTarget: SyncSlotIndex | 'all' = 'all') => {
    const message = error instanceof Error ? error.message : String(error);
    const slots: [AppSyncSlotViewState, AppSyncSlotViewState] = [
      { ...state.slots[0], logs: [...state.slots[0].logs] },
      { ...state.slots[1], logs: [...state.slots[1].logs] },
    ];
    const affectedSlots: SyncSlotIndex[] = errorTarget === 'all' ? [0, 1] : [errorTarget];
    affectedSlots.forEach((slotIndex) => {
      slots[slotIndex] = {
        ...slots[slotIndex],
        status: 'error',
        progressText: t('error', state.lang),
        logExpanded: true,
        logs: [...slots[slotIndex].logs, message],
      };
    });
    state = { ...state, slots };
    render();
  };

  const runSelectionAction = async (
    kind: Exclude<PendingSelection, null>,
    changed: boolean,
    action: () => Promise<DesktopState>,
  ) => {
    if (!changed || pendingSelection !== null) {
      return;
    }
    pendingSelection = kind;
    render();
    try {
      const nextState = await action();
      selectionMotion = kind;
      applyDesktopState(nextState);
    } catch (error) {
      reportError(error);
    } finally {
      pendingSelection = null;
      render();
      setTimeout(() => {
        if (selectionMotion === kind) {
          selectionMotion = null;
          render();
        }
      }, 520);
    }
  };

  const openPreview = async (retryOf: string | null = null, previewPromise?: Promise<AppPreview[]>) => {
    pendingGlobalAction = 'start-all';
    previewBusy = true;
    render();
    try {
      const previews = await (previewPromise || services.previewAllSync());
      previewModal = { previews, retryOf };
    } catch (error) {
      reportError(error);
    } finally {
      previewBusy = false;
      pendingGlobalAction = null;
      render();
    }
  };

  const confirmPreview = async () => {
    if (!previewModal) {
      return;
    }
    const hasEnoughSpace = previewModal.previews.every(
      (item) => item.preview.disk_space_sufficient !== false,
    );
    const canConfirm = hasEnoughSpace && (
      previewModal.previews.some((item) => item.preview.candidates.length > 0)
      || previewHasRetryErrors(previewModal)
    );
    if (!canConfirm) {
      return;
    }
    previewBusy = true;
    pendingGlobalAction = 'start-all';
    render();
    try {
      const nextState = await services.startConfirmedSync(
        previewModal.previews,
        previewModal.retryOf,
      );
      previewModal = null;
      applyDesktopState(nextState);
    } catch (error) {
      reportError(error);
    } finally {
      previewBusy = false;
      pendingGlobalAction = null;
      render();
    }
  };

  const retryHistory = async (id: string) => {
    render();
    try {
      const preview = await services.retryHistoryFailures(id);
      previewModal = { previews: [preview], retryOf: id };
    } catch (error) {
      reportError(error);
    } finally {
      render();
    }
  };

  const exportHistory = async (id: string) => {
    try {
      const path = await save({
        defaultPath: 'W4DJ-error-report.txt',
        title: state.lang === 'zh' ? '保存错误报告' : 'Save error report',
      });
      if (typeof path === 'string') {
        await services.exportHistoryErrorReport(id, path);
      }
    } catch (error) {
      reportError(error);
    }
  };

  const deleteHistory = async (id: string) => {
    try {
      await services.deleteHistoryEntry(id);
      await refreshHistory();
    } catch (error) {
      reportError(error);
    }
  };

  const clearAllHistory = async () => {
    const message = state.lang === 'zh' ? '确定清空全部转换历史吗？' : 'Clear all conversion history?';
    if (!window.confirm(message)) {
      return;
    }
    try {
      await services.clearHistory();
      await refreshHistory();
    } catch (error) {
      reportError(error);
    }
  };

  const openAbout = async () => {
    try {
      aboutInfo = await services.loadAppInfo();
      render();
    } catch (error) {
      reportError(error);
    }
  };

  const runAction = async (
    action: () => Promise<DesktopState | void>,
    errorTarget?: SyncSlotIndex | 'all',
    pendingAction: 'start-all' | 'pause-all' | null = null,
    motion: SelectionMotion = null,
  ) => {
    if (motion) {
      selectionMotion = motion;
    }
    pendingGlobalAction = pendingAction;
    if (pendingAction !== null) {
      render();
    }

    try {
      const nextState = await action();
      if (nextState) {
        applyDesktopState(nextState);
      }
    } catch (error) {
      if (errorTarget === undefined) {
        return;
      }
      reportError(error, errorTarget);
    } finally {
      if (pendingAction !== null) {
        pendingGlobalAction = null;
        render();
      }
      if (motion) {
        setTimeout(() => {
          if (selectionMotion === motion) {
            selectionMotion = null;
            render();
          }
        }, 520);
      }
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
    const slotIndex = parseSlotIndex(button.dataset.slot);

    if (action === 'toggle-lang') {
      state = { ...state, lang: state.lang === 'zh' ? 'en' : 'zh' };
      localStorage.setItem('w4dj_lang', state.lang);
      state.slots.forEach((slot) => {
        slot.progressText = formatProgressText(slot, state.lang);
      });
      triggerLocalMotion('lang');
      return;
    }

    if (action === 'toggle-theme') {
      state = { ...state, theme: state.theme === 'light' ? 'dark' : 'light' };
      localStorage.setItem('w4dj_theme', state.theme);
      triggerLocalMotion('theme');
      return;
    }

    if (action === 'open-about') {
      void openAbout();
      return;
    }

    if (action === 'close-about') {
      aboutInfo = null;
      render();
      return;
    }

    if (action === 'open-project-home') {
      const url = button.dataset.url;
      if (url) {
        void services.openExternalUrl(url);
      }
      return;
    }

    if (action === 'toggle-log' && slotIndex !== null) {
      const slots: [AppSyncSlotViewState, AppSyncSlotViewState] = [
        { ...state.slots[0] },
        { ...state.slots[1] },
      ];
      slots[slotIndex].logExpanded = !slots[slotIndex].logExpanded;
      state = { ...state, slots };
      render();
      return;
    }

    if (action === 'cancel-preview' || action === 'edit-preview') {
      if (!previewBusy) {
        previewModal = null;
        render();
      }
      return;
    }

    if (action === 'confirm-start') {
      void confirmPreview();
      return;
    }

    if (action === 'retry-history') {
      const historyId = button.dataset.historyId;
      if (historyId) {
        void retryHistory(historyId);
      }
      return;
    }

    if (action === 'export-history') {
      const historyId = button.dataset.historyId;
      if (historyId) {
        void exportHistory(historyId);
      }
      return;
    }

    if (action === 'delete-history') {
      const historyId = button.dataset.historyId;
      if (historyId) {
        void deleteHistory(historyId);
      }
      return;
    }

    if (action === 'clear-history') {
      void clearAllHistory();
      return;
    }

    if (action === 'cancel-slot' && slotIndex !== null) {
      void runAction(() => services.cancelSync(slotIndex), slotIndex);
      return;
    }

    if (action === 'cancel-all') {
      void runAction(() => services.cancelAllSync(), 'all');
      return;
    }

    if (action === 'pick-source' && slotIndex !== null) {
      void runAction(async () => {
        const path = await services.pickDirectory('source', slotIndex);
        return path ? services.selectSourceDirectory(slotIndex, path) : undefined;
      }, slotIndex);
      return;
    }

    if (action === 'clear-source' && slotIndex !== null) {
      void runAction(() => services.selectSourceDirectory(slotIndex, ''), slotIndex);
      return;
    }

    if (action === 'pick-destination' && slotIndex !== null) {
      void runAction(async () => {
        const path = await services.pickDirectory('destination', slotIndex);
        return path ? services.selectDestinationDirectory(slotIndex, path) : undefined;
      }, slotIndex);
      return;
    }

    if (action === 'clear-destination' && slotIndex !== null) {
      void runAction(() => services.selectDestinationDirectory(slotIndex, ''), slotIndex);
      return;
    }

    if (mode) {
      void runSelectionAction('mode', state.mode !== mode, () => services.chooseMode(mode));
      return;
    }

    if (format) {
      void runSelectionAction(
        'format',
        state.losslessFormat !== format,
        () => services.chooseLosslessFormat(format),
      );
      return;
    }

    if (action === 'start-all') {
      void openPreview();
      return;
    }

    if (action === 'pause-all') {
      void runAction(() => services.pauseAllSync(), 'all', 'pause-all');
    }
  });

  root.addEventListener('change', (event) => {
    const select = (event.target as HTMLElement | null)?.closest<HTMLSelectElement>('select');
    if (!select) {
      return;
    }

    if (select.dataset.action === 'choose-conflict') {
      const strategy = select.value as AppConflictStrategy;
      if (strategy !== state.conflictStrategy) {
        void runAction(() => services.chooseConflictStrategy(strategy), 'all');
      }
      return;
    }

    if (select.dataset.action === 'choose-filename-rule') {
      const rule = select.value as AppFilenameRule;
      if (rule !== state.filenameRule) {
        void runAction(() => services.chooseFilenameRule(rule), 'all');
      }
    }
  });

  const clearDropTargets = () => {
    root.querySelectorAll<HTMLElement>('[data-drop-kind].is-drag-over').forEach((target) => {
      target.classList.remove('is-drag-over');
    });
  };

  const dropTargetAt = (position: { x: number; y: number }) => {
    const scale = window.devicePixelRatio || 1;
    const points = [
      [position.x / scale, position.y / scale],
      [position.x, position.y],
    ];

    for (const [x, y] of points) {
      const target = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-drop-kind]');
      if (target) {
        return target;
      }
    }

    return null;
  };

  const pathFromBrowserDrop = (event: DragEvent): string | null => {
    const file = event.dataTransfer?.files[0] as (File & { path?: string }) | undefined;
    if (file?.path) {
      return file.path;
    }

    const uri = event.dataTransfer?.getData('text/uri-list')
      .split('\n')
      .map((value) => value.trim())
      .find((value) => value && !value.startsWith('#'));
    if (!uri || !uri.startsWith('file://')) {
      return null;
    }

    try {
      return decodeURIComponent(new URL(uri).pathname);
    } catch {
      return null;
    }
  };

  const handleDirectoryDrop = (target: HTMLElement, path: string | null) => {
    target.classList.remove('is-drag-over');
    if (!path) {
      return;
    }

    const slotIndex = parseSlotIndex(target.dataset.slot);
    const kind = target.dataset.dropKind;
    if (slotIndex === null || (kind !== 'source' && kind !== 'destination')) {
      return;
    }

    void runAction(
      () => kind === 'source'
        ? services.selectSourceDirectory(slotIndex, path)
        : services.selectDestinationDirectory(slotIndex, path),
      slotIndex,
    );
  };

  root.addEventListener('dragover', (event) => {
    const target = (event.target as HTMLElement | null)?.closest<HTMLElement>('[data-drop-kind]');
    if (!target) {
      return;
    }

    event.preventDefault();
    clearDropTargets();
    target.classList.add('is-drag-over');
  });

  root.addEventListener('drop', (event) => {
    const target = (event.target as HTMLElement | null)?.closest<HTMLElement>('[data-drop-kind]');
    if (!target) {
      return;
    }

    event.preventDefault();
    handleDirectoryDrop(target, pathFromBrowserDrop(event));
  });

  try {
    const listener = getCurrentWindow().onDragDropEvent(({ payload }: { payload: DragDropEvent }) => {
      if (payload.type === 'leave') {
        clearDropTargets();
        return;
      }

      const target = dropTargetAt(payload.position);
      clearDropTargets();
      target?.classList.add('is-drag-over');

      if (payload.type !== 'drop' || !target || payload.paths.length === 0) {
        return;
      }

      handleDirectoryDrop(target, payload.paths[0]);
    });
    void listener.catch((error) => console.error('Failed to register folder drag-and-drop:', error));
  } catch {
    // Tauri drag-and-drop is unavailable in the browser test environment.
  }

  render();
  void runAction(() => services.loadDesktopState());
  void refreshHistory();
}

function renderLosslessFormats(state: AppViewState, pendingSelection: PendingSelection = null): string {
  const formats: AppLosslessFormat[] = ['wav', 'aiff'];
  return `
    <div class="format-slot">
      ${state.mode === 'lossless' ? `
        <div class="format-row" data-selected-format="${state.losslessFormat || 'wav'}" aria-label="${t('losslessFormat', state.lang)}">
          ${formats
            .map(
              (format) => `
                <button type="button" class="format-button ${state.losslessFormat === format ? 'selected' : ''}" data-format="${format}" ${pendingSelection === 'format' ? 'disabled' : ''}>
                  ${format.toUpperCase()}
                </button>
              `,
            )
            .join('')}
        </div>
      ` : ''}
    </div>
  `;
}

function renderOutputSettings(state: AppViewState): string {
  return `
    <section class="output-settings" aria-label="${t('conflictStrategy', state.lang)}">
      <label>
        <span>${t('conflictStrategy', state.lang)}</span>
        <select data-action="choose-conflict" aria-label="${t('conflictStrategy', state.lang)}">
          <option value="skip" ${state.conflictStrategy === 'skip' ? 'selected' : ''}>${t('conflictSkip', state.lang)}</option>
          <option value="overwrite" ${state.conflictStrategy === 'overwrite' ? 'selected' : ''}>${t('conflictOverwrite', state.lang)}</option>
          <option value="update_metadata" ${state.conflictStrategy === 'update_metadata' ? 'selected' : ''}>${t('conflictMetadata', state.lang)}</option>
        </select>
      </label>
      <label>
        <span>${t('filenameRule', state.lang)}</span>
        <select data-action="choose-filename-rule" aria-label="${t('filenameRule', state.lang)}">
          <option value="title_artist" ${state.filenameRule === 'title_artist' ? 'selected' : ''}>${t('titleArtist', state.lang)}</option>
          <option value="artist_title" ${state.filenameRule === 'artist_title' ? 'selected' : ''}>${t('artistTitle', state.lang)}</option>
          <option value="original" ${state.filenameRule === 'original' ? 'selected' : ''}>${t('originalName', state.lang)}</option>
        </select>
      </label>
    </section>
  `;
}

function renderAboutModal(info: AppInfo | null, lang: AppLanguage): string {
  if (!info) {
    return '';
  }

  return `
    <div class="about-modal" data-role="about-modal" role="dialog" aria-modal="true" aria-label="${t('about', lang)}">
      <section class="about-dialog">
        <p class="panel-kicker">W4DJ RKB</p>
        <h2>${t('about', lang)}</h2>
        <dl>
          <div><dt>${t('version', lang)}</dt><dd>v${escapeHtml(info.version)}</dd></div>
          <div><dt>${t('developer', lang)}</dt><dd>${escapeHtml(info.developer)}</dd></div>
        </dl>
        <div class="about-links">
          <button type="button" class="about-link" data-action="open-project-home" data-url="${escapeHtml(info.project_url)}">${t('projectHome', lang)}</button>
        </div>
        <button type="button" class="global-action" data-action="close-about">${t('close', lang)}</button>
      </section>
    </div>
  `;
}

function toViewState(state: DesktopState, lang: AppLanguage, theme: AppTheme): AppViewState {
  return {
    slots: state.slots.map((slot) => ({
      sourceDirectory: slot.source_directory,
      destinationDirectory: slot.destination_directory,
      status: slot.status,
      progressTotal: slot.progress_total,
      progressCompleted: slot.progress_completed,
      newTracks: slot.new_tracks,
      skippedTracks: slot.skipped_tracks,
      errorTracks: slot.error_tracks,
      progressText: formatDesktopProgress(slot, lang),
      currentFile: slot.current_file,
      logExpanded: false,
      logs: slot.logs,
    })) as [AppSyncSlotViewState, AppSyncSlotViewState],
    mode: state.mode,
    losslessFormat: state.lossless_format,
    conflictStrategy: state.conflict_strategy,
    filenameRule: state.filename_rule,
    lang,
    theme,
  };
}

function formatDesktopProgress(state: DesktopSyncSlotState, lang: AppLanguage): string {
  if (state.progress_total > 0) {
    return `${state.progress_completed}/${state.progress_total}`;
  }

  return statusLabel(state.status, lang);
}

function formatProgressText(state: AppSyncSlotViewState, lang: AppLanguage): string {
  if (state.progressTotal > 0) {
    return `${state.progressCompleted}/${state.progressTotal}`;
  }

  return statusLabel(state.status, lang);
}

function statusLabel(status: AppStatus, lang: AppLanguage): string {
  return t(status, lang);
}

function historyStatusLabel(status: AppHistoryStatus, lang: AppLanguage): string {
  const labels: Record<AppHistoryStatus, { zh: string; en: string }> = {
    completed: { zh: '已完成', en: 'Completed' },
    partial: { zh: '部分完成', en: 'Partial' },
    cancelled: { zh: '已取消', en: 'Cancelled' },
    error: { zh: '错误', en: 'Error' },
  };
  return labels[status][lang];
}

function errorCategoryLabel(category: AppErrorCategory, lang: AppLanguage): string {
  const labels: Record<AppErrorCategory, { zh: string; en: string }> = {
    file_damaged: { zh: '文件损坏或无法读取', en: 'Damaged or unreadable file' },
    unsupported_format: { zh: '格式不支持', en: 'Unsupported format' },
    ffmpeg: { zh: 'FFmpeg 转换失败', en: 'FFmpeg failure' },
    output_permission: { zh: '输出目录无权限', en: 'Output permission denied' },
    disk_space: { zh: '磁盘空间不足', en: 'Insufficient disk space' },
    invalid_filename: { zh: '文件名非法', en: 'Invalid filename' },
    unknown: { zh: '其他错误', en: 'Other error' },
  };
  return labels[category]?.[lang] || labels.unknown[lang];
}

function modeLabel(mode: AppMode, lang: AppLanguage): string {
  return mode === 'compat' ? t('compatMode', lang) : t('losslessMode', lang);
}

function formatBytes(bytes: number | null, lang: AppLanguage): string {
  if (bytes === null) {
    return lang === 'zh' ? '无法估算' : 'Unavailable';
  }
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unitIndex = -1;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unitIndex]}`;
}

function aggregateStatus(state: AppViewState): AppStatus {
  const priority: AppStatus[] = ['error', 'running', 'paused', 'cancelled', 'completed', 'idle'];
  return priority.find((status) => state.slots.some((slot) => slot.status === status)) || 'idle';
}

function latestLog(logs: string[], lang: AppLanguage): string {
  return logs.length > 0 ? logs[logs.length - 1] : t('noCurrentFile', lang);
}

function displayPath(path: string, lang: AppLanguage): string {
  return escapeHtml(path || t('pickFolder', lang));
}

function progressPercent(state: AppSyncSlotViewState): number {
  if (state.progressTotal <= 0) {
    return 0;
  }

  return Math.min(
    100,
    Math.max(0, Math.round((state.progressCompleted / state.progressTotal) * 100)),
  );
}

function parseSlotIndex(value: string | undefined): SyncSlotIndex | null {
  if (value === '0') {
    return 0;
  }
  if (value === '1') {
    return 1;
  }
  return null;
}

function icon(name: 'folder' | 'export' | 'trash' | 'check' | 'disc' | 'play' | 'pause' | 'list' | 'sun' | 'moon' | 'arrow'): string {
  const icons = {
    folder: '<path d="M2.5 5.1h3.4l1.1 1.2h6.5v5.2H2.5z"/><path d="M2.5 4.5h3.2l1.3 1.2"/>',
    export: '<path d="M3 12.2h10"/><path d="M8 4v6.1"/><path d="M5.6 6.4 8 4l2.4 2.4"/>',
    trash: '<path d="M3.8 5.2h8.4"/><path d="M6.2 5.2V3.8h3.6v1.4"/><path d="m5 5.2.5 7.2h5l.5-7.2"/><path d="M6.8 7.1v3.7M9.2 7.1v3.7"/>',
    check: '<path d="M3.3 8.5 6.4 11.4 12.8 4.7"/>',
    disc: '<circle cx="8" cy="8" r="5.1"/><circle cx="8" cy="8" r="1"/>',
    play: '<path d="M5.2 4v8l6.6-4z"/>',
    pause: '<path d="M5.1 4.2v7.6"/><path d="M10.9 4.2v7.6"/>',
    list: '<path d="M5 4.7h8"/><path d="M5 8h8"/><path d="M5 11.3h8"/><path d="M2.7 4.7h.5"/><path d="M2.7 8h.5"/><path d="M2.7 11.3h.5"/>',
    sun: '<circle cx="8" cy="8" r="2.8"/><path d="M8 1.8v1.3M8 12.9v1.3M1.8 8h1.3M12.9 8h1.3M3.6 3.6l.9.9M11.5 11.5l.9.9M12.4 3.6l-.9.9M4.5 11.5l-.9.9"/>',
    moon: '<path d="M12.7 10.4A5.3 5.3 0 0 1 5.6 3.3a5.3 5.3 0 1 0 7.1 7.1z"/>',
    arrow: '<path d="M2.5 8h10.2"/><path d="m9.4 4.8 3.3 3.2-3.3 3.2"/>',
  } as const;

  return `<span class="ui-icon ui-icon-${name}"><svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">${icons[name]}</svg></span>`;
}

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;');
}
