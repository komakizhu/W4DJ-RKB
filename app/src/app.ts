import { invoke } from '@tauri-apps/api/core';
import { message, open, save } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow, type DragDropEvent } from '@tauri-apps/api/window';
import {
  analyzeAudioFile,
  ESSENTIA_MODEL_IDS,
  TRACK_ANALYSIS_VERSION,
  type EssentiaModelFile,
  type TrackAnalysis,
  type TrackMetadata,
} from './analysis';
import {
  renderLibraryDashboard,
  libraryColumnIds,
  saveLibraryColumnOrder,
  toggleLibraryColumn,
  type LibraryDashboardState,
  type LibraryField,
  type LibraryFilter,
  type LibraryLyricsTab,
  type LibraryOperator,
  type LibraryPage,
  type LibraryQuery,
  type LibrarySourceRecord,
  type LibraryStatus,
  type LibraryTrack,
} from './library-dashboard';

export type AppMode = 'compat' | 'lossless';
export type AppLosslessFormat = 'wav' | 'aiff';
export type AppConversionMode = 'scan_then_convert' | 'direct';
export type AppConflictStrategy = 'skip' | 'overwrite' | 'rename' | 'update_metadata';
export type AppFilenameRule = 'title_artist' | 'artist_title' | 'original';
export type AppNeteaseFilenameFormat = 'title_only' | 'artist_title' | 'title_artist';
export type AppStatus = 'idle' | 'running' | 'paused' | 'completed' | 'error' | 'cancelled';
export type AppScanStatus = 'idle' | 'running' | 'completed' | 'cancelled' | 'error';
export type AppScanPhase = 'preparing' | 'scanning_source' | 'scanning_destination' | 'checking' | 'analyzing' | 'completed' | 'cancelled' | 'error';
export type AppLanguage = 'zh' | 'en';
export type AppTheme = 'light' | 'dark';
export type SyncSlotIndex = 0 | 1;
export type SourcePickerChoice = 'folder' | 'track' | 'cancel';
type SelectionMotion = 'mode' | 'format' | 'conversion-mode' | 'enhanced-mode' | 'theme' | 'lang' | null;
type PendingSelection = 'mode' | 'format' | 'conversion-mode' | 'enhanced-mode' | null;
type PendingGlobalAction = 'start-all' | 'pause-all' | 'cancel-scan' | 'cancel-all' | 'cancel-analysis' | null;
type OnboardingStep = 0 | 1 | 2 | 3 | 4;
type OnboardingTarget = 'mode' | 'source' | 'destination' | 'start' | 'tutorial';

const ONBOARDING_STEP_COUNT = 5;

function createAnalysisBatchId(): string {
  const timestamp = Date.now().toString(36);
  const random = Math.random().toString(36).slice(2, 8);
  return `batch-${timestamp}-${random}`;
}

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
  logs: string[];
};

export type AppViewState = {
  slots: [AppSyncSlotViewState, AppSyncSlotViewState];
  mode: AppMode;
  losslessFormat: AppLosslessFormat | null;
  conversionMode: AppConversionMode;
  enhancedMode: boolean;
  conflictStrategy: AppConflictStrategy;
  filenameRule: AppFilenameRule;
  neteaseFilenameFormat: AppNeteaseFilenameFormat;
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
  conversion_mode: AppConversionMode;
  enhanced_mode: boolean;
  conflict_strategy: AppConflictStrategy;
  filename_rule: AppFilenameRule;
  netease_filename_format: AppNeteaseFilenameFormat;
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

export type AppUpdateCheck = {
  current_version: string;
  latest_version: string;
  update_available: boolean;
  release_url: string;
  release_name: string;
};

export type AppAnalysisState = {
  status: 'idle' | 'running' | 'completed' | 'cancelled' | 'error';
  completed: number;
  total: number;
  resultCount: number;
  failedCount: number;
  message: string;
};

export type AppAnalysisFailure = {
  path: string;
  message: string;
};

export type AppAudioFileFingerprint = {
  sizeBytes: number;
  modifiedAt: number | null;
};

export function canReuseTrackAnalysis(
  cached: TrackAnalysis | undefined,
  fingerprint: AppAudioFileFingerprint,
  neteaseFilenameFormat: AppNeteaseFilenameFormat,
  highLevelModelVersion: string | null = null,
  highLevelModelsAvailable = false,
): cached is TrackAnalysis {
  const basicMatch = cached?.analysisVersion === TRACK_ANALYSIS_VERSION
    && cached.sourceSizeBytes === fingerprint.sizeBytes
    && (cached.sourceModifiedAt ?? null) === fingerprint.modifiedAt
    && (cached.sourceFilenameFormat ?? 'title_artist') === neteaseFilenameFormat;
  if (!basicMatch) {
    return false;
  }
  if (!highLevelModelsAvailable) {
    return true;
  }
  return cached.highLevel?.status === 'completed'
    && cached.highLevel.modelVersion === highLevelModelVersion;
}

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
  logs: string[];
  status: AppHistoryStatus;
  retry_of: string | null;
  conflict_strategy: AppConflictStrategy;
  filename_rule: AppFilenameRule;
  report_path: string | null;
};

export type AppPreviewModalState = {
  previews: AppPreview[];
  retryOf: string | null;
};

export type AppScanProgress = {
  status: AppScanStatus;
  phase: AppScanPhase;
  processed: number;
  total: number;
  current_file: string;
  message: string;
  tasks?: AppScanTaskProgress[];
};

export type AppScanTaskProgress = {
  slot_index: SyncSlotIndex;
  phase: AppScanPhase;
  processed: number;
  total: number;
  current_file: string;
};

export type AppServices = {
  loadDesktopState: () => Promise<DesktopState>;
  pickDirectory: (
    kind: 'destination',
    slotIndex: SyncSlotIndex,
  ) => Promise<string | null>;
  pickSource: (slotIndex: SyncSlotIndex) => Promise<string | null>;
  selectSourceDirectory: (slotIndex: SyncSlotIndex, path: string) => Promise<DesktopState>;
  selectDestinationDirectory: (slotIndex: SyncSlotIndex, path: string) => Promise<DesktopState>;
  chooseMode: (mode: AppMode) => Promise<DesktopState>;
  chooseLosslessFormat: (format: AppLosslessFormat | null) => Promise<DesktopState>;
  chooseConversionMode: (mode: AppConversionMode) => Promise<DesktopState>;
  chooseEnhancedMode: (enabled: boolean) => Promise<DesktopState>;
  chooseConflictStrategy: (strategy: AppConflictStrategy) => Promise<DesktopState>;
  chooseFilenameRule: (rule: AppFilenameRule) => Promise<DesktopState>;
  chooseNeteaseFilenameFormat: (format: AppNeteaseFilenameFormat) => Promise<DesktopState>;
  previewAllSync: () => Promise<AppPreview[]>;
  startScan: () => Promise<AppScanProgress>;
  loadScanState: () => Promise<AppScanProgress>;
  loadScanResult: () => Promise<AppPreview[]>;
  cancelScan: () => Promise<AppScanProgress>;
  clearScanCache?: () => Promise<void>;
  startConfirmedSync: (
    previews: AppPreview[],
    retryOf?: string | null,
    analyses?: TrackAnalysis[],
    analysisFailures?: AppAnalysisFailure[],
    batchId?: string,
  ) => Promise<DesktopState>;
  applyTrackAnalysisResults: (
    batchId: string,
    previews: AppPreview[],
    analyses: TrackAnalysis[],
    analysisFailures: AppAnalysisFailure[],
  ) => Promise<DesktopState>;
  loadHistory: () => Promise<AppHistoryEntry[]>;
  retryHistoryFailures: (id: string) => Promise<AppPreview>;
  exportHistoryErrorReport: (id: string, path: string) => Promise<void>;
  deleteHistoryEntry: (id: string) => Promise<void>;
  clearHistory: () => Promise<void>;
  loadAppInfo: () => Promise<AppInfo>;
  checkForUpdates: () => Promise<AppUpdateCheck>;
  openExternalUrl: (url: string) => Promise<void>;
  openDestination: (path: string) => Promise<void>;
  openSource: (path: string) => Promise<void>;
  startAllSync: () => Promise<DesktopState>;
  pauseAllSync: () => Promise<DesktopState>;
  cancelSync: (slotIndex: SyncSlotIndex) => Promise<DesktopState>;
  cancelAllSync: () => Promise<DesktopState>;
  listAudioFiles: (path: string) => Promise<string[]>;
  readAudioFile: (path: string) => Promise<number[]>;
  readTrackMetadata: (path: string) => Promise<TrackMetadata>;
  getAudioFileFingerprint: (path: string) => Promise<AppAudioFileFingerprint>;
  loadTrackAnalyses: () => Promise<TrackAnalysis[]>;
  saveTrackAnalyses: (entries: TrackAnalysis[]) => Promise<number>;
  clearTrackAnalyses: () => Promise<void>;
  exportRekordboxXml: (path: string) => Promise<void>;
  getEssentiaModelStatus?: () => Promise<EssentiaModelStatus>;
  downloadEssentiaModels?: () => Promise<EssentiaModelStatus>;
  loadEssentiaModel?: (id: string) => Promise<EssentiaModelFile>;
  loadLibraryStatus?: () => Promise<LibraryStatus>;
  locateNeteaseLibrary?: () => Promise<LibraryStatus['netease']>;
  refreshLibraryCatalog?: () => Promise<unknown>;
  queryLibraryCatalog?: (query: LibraryQuery) => Promise<LibraryPage>;
  getLibraryTrackDetail?: (trackKey: string) => Promise<LibraryTrack | null>;
  getLibraryTrackSourceRecords?: (trackKey: string) => Promise<LibrarySourceRecord[]>;
  getLibraryTrackCover?: (trackKey: string) => Promise<string | null>;
  clearLibraryCatalogCache?: () => Promise<void>;
};

export type EssentiaModelStatus = {
  version: string;
  embedding: boolean;
  genre: boolean;
  mood: boolean;
  instrument: boolean;
  downloading: boolean;
};

const defaultEssentiaModelStatus: EssentiaModelStatus = {
  version: '',
  embedding: false,
  genre: false,
  mood: false,
  instrument: false,
  downloading: false,
};

export type DropTargetRect = {
  left: number;
  top: number;
  right: number;
  bottom: number;
};

export type DropCoordinateSpace = 'logical' | 'physical';

export function resolveDropTargetAt<T>(
  targets: Array<{ value: T; rect: DropTargetRect }>,
  position: { x: number; y: number },
  scaleFactor = 1,
  coordinateSpace: DropCoordinateSpace = 'logical',
): T | null {
  const safeScaleFactor =
    coordinateSpace === 'physical' && Number.isFinite(scaleFactor) && scaleFactor > 0
      ? scaleFactor
      : 1;
  const x = position.x / safeScaleFactor;
  const y = position.y / safeScaleFactor;

  return (
    targets.find(
      ({ rect }) =>
        x >= rect.left &&
        x <= rect.right &&
        y >= rect.top &&
        y <= rect.bottom,
    )?.value ?? null
  );
}

function nativeDropCoordinatesArePhysical(): boolean {
  return /Windows/i.test(navigator.userAgent);
}

const translations = {
  zh: {
    eyebrow: 'W4DJ RKB',
    title: '如果我是DJ',
    railLead: '输出模式',
    sourceKicker: '歌曲文件夹或单曲（网易云、SoundCloud 等）',
    destKicker: '任务 1 / 任务 2 独立运行，窗口较小时可滚动',
    sourceLabel: '歌曲文件夹或单曲',
    destLabel: '输出目录',
    clearSource: '清空输入来源',
    clearDestination: '清空输出目录',
    openSource: '打开输入来源',
    openDestination: '打开输出目录',
    pickFolder: '选择文件夹',
    pickSource: '选择来源',
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
    conversionMode: '转换方式',
    scanThenConvert: '扫描后转换',
    directConvert: '直接转换',
    enhancedAnalysis: '分析增强',
    standardConvert: '普通转换',
    enhancedMode: '增强模式',
    enhancedModeOffNote: '只转换，不执行音乐分析',
    enhancedModeOnNote: '自动分析 BPM、Key、响度和能量并写入元数据',
    advancedOptions: '高级选项',
    losslessFormat: '无损格式',
    syncSlot: '任务',
    fallback: '未单独设置，使用输出目录 1',
    fallbackMissing: '输出目录 1 也未设置',
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
    exportReport: '导出完整错误报告',
    completedCount: '完成',
    failedCount: '失败',
    sourcePath: '输入来源',
    destinationPath: '输出目录',
    conflictStrategy: '已存在文件',
    conflictSkip: '已存在文件：跳过',
    conflictOverwrite: '已存在文件：覆盖',
    conflictMetadata: '高级选项：仅更新元数据',
    filenameRule: '文件名规则',
    neteaseFilenameFormat: '网易云源文件名格式',
    neteaseTitleOnly: '仅歌曲名',
    neteaseArtistTitle: '歌手 - 歌曲名',
    neteaseTitleArtist: '歌曲名 - 歌手',
    titleArtist: '歌曲名 - 歌手（默认）',
    artistTitle: '歌手 - 歌曲名',
    originalName: '保留原文件名',
    availableSpace: '可用空间',
    insufficientSpace: '磁盘空间不足，无法开始转换',
    cancelTask: '取消任务',
    resumeTasks: '继续未完成任务',
    deleteHistory: '删除记录',
    clearHistory: '清空历史',
    historyLoadError: '转换历史读取失败，原记录未被覆盖。请检查历史文件后再重试。',
    about: '关于',
    tutorial: '教程',
    helpTitle: '使用帮助',
    helpIntro: '这里集中说明输出、分析和转换方式，遇到不确定时可以随时回来查看。',
    helpOutputTitle: '输出与分析',
    helpCompatibilityTitle: '普通转换',
    helpCompatibilityBody: '关闭增强模式时，W4DJ 只进行常规格式转换。',
    helpEnhancedTitle: '增强转换',
    helpEnhancedBody: '开启增强模式后，W4DJ 会自动分析 BPM、Key、响度和能量，并写入输出音频元数据。',
    helpConversionTitle: '转换方式',
    helpScanThenConvertBody: '点击开始后先扫描任务，显示重复文件、错误文件和预计输出；确认后再正式转换。',
    helpDirectConvertBody: '完成输入目录、输出目录、磁盘空间和文件可读性检查后直接转换，不显示二次确认页。',
    version: '版本',
    developer: '开发者',
    projectHome: '项目主页',
    checkUpdates: '检查更新',
    updateAvailable: '发现新版本 {version}',
    alreadyLatest: '已是最新版本',
    viewRelease: '查看发布页',
    close: '关闭',
    pendingCount: '待继续',
    errorCategory: '错误类型',
    onboardingTitle: '第一次使用？看这里',
    onboardingIntro: '五步完成一次转换，文件夹和单曲会自动识别。',
    onboardingStepOneTitle: '先选输出模式',
    onboardingStepOneBody: '兼容模式导出 MP3；无损模式可以选择 WAV 或 AIFF。',
    onboardingStepTwoTitle: '拖入来源',
    onboardingStepTwoBody: '把文件夹或单曲拖到任意任务的左侧来源框，软件会自动识别。',
    onboardingStepThreeTitle: '指定输出目录',
    onboardingStepThreeBody: '设置转换后的文件保存在哪里；任务 2 没有单独目录时会沿用任务 1。',
    onboardingStepFourTitle: '开始转换',
    onboardingStepFourBody: '准备好后点击这里，软件会先扫描并让你确认，再正式转换。',
    onboardingStepFiveTitle: '随时重新查看教程',
    onboardingStepFiveBody: '以后可点击右上角「教程」，再选择“重新查看使用引导”，即可再次查看完整教程。',
    onboardingNext: '下一步',
    onboardingPrevious: '上一步',
    onboardingSkip: '跳过教程',
    onboardingFinish: '完成',
    usageGuide: '重新查看使用引导',
    analysisTitle: '音乐分析',
    analysisBody: '扫描输出目录，写入 BPM、Key、响度和能量。',
    analyzeLibrary: '分析音乐库',
    exportRekordbox: '导出 Rekordbox XML',
    analysisIdle: '先设置输出目录，再开始分析。',
    analysisRunning: '正在分析音乐库…',
    analysisCancelled: '增强分析已取消，已完成的结果已保存。',
    analysisComplete: '已保存 {count} 首分析结果，可导入 Rekordbox。',
    analysisPartial: '完成 {done}/{total} 首，{failed} 首失败。',
    analysisNoResults: '没有成功的分析结果。',
    clearAnalysisCache: '清除分析缓存',
    clearAnalysisCacheConfirm: '确定清除已保存的音乐分析缓存吗？不会删除音频文件或转换历史。',
    analysisCacheCleared: '音乐分析缓存已清除。',
    clearEnhancedCache: '清除增强模式缓存',
    clearEnhancedCacheConfirm: '确定清除增强模式缓存吗？不会删除音频文件、扫描缓存或已下载模型。',
    enhancedCacheCleared: '增强模式缓存已清除。',
    clearScanCache: '清除扫描缓存',
    clearScanCacheConfirm: '确定清除扫描缓存吗？下一次开始时会重新扫描全部歌曲。不会删除增强模式缓存或模型。',
    scanCacheCleared: '扫描缓存已清除。',
    essentiaModelsTitle: 'Essentia 预训练模型',
    essentiaModelsReady: '已下载，增强模式会识别流派、情绪和人声/器乐。',
    essentiaModelsMissing: '未下载；增强模式仍可进行基础分析和 Drop LUFS。',
    essentiaModelsDownload: '下载分析模型',
    essentiaModelsDownloading: '正在下载模型…',
    essentiaModelsDownloaded: 'Essentia 预训练模型已下载。',
    essentiaModelsPartial: '模型未全部下载完成，增强模式仍可运行基础分析。',
    scanTitle: '扫描歌曲',
    scanPreparing: '正在准备扫描',
    scanSource: '正在扫描输入目录',
    scanDestination: '正在扫描输出目录',
    scanChecking: '正在检查转换条件',
    scanAnalyzing: '正在分析歌曲并写入元数据',
    scanCompleted: '扫描完成',
    scanCancelled: '扫描已取消',
    scanError: '扫描失败',
    scanCurrentFile: '当前文件',
    scanCancel: '取消扫描',
    conversionCancel: '取消转换',
    conversionRunning: '正在转换',
    analysisCancel: '取消分析',
    scanClose: '关闭',
  },
  en: {
    eyebrow: 'W4DJ RKB',
    title: 'If I Were a DJ',
    railLead: 'Output mode',
    sourceKicker: 'Music folders or tracks (NetEase, SoundCloud, etc.)',
    destKicker: 'Task 1 and Task 2 run independently. Scroll when the window is short.',
    sourceLabel: 'Music Folder or Track',
    destLabel: 'Output Folder',
    clearSource: 'Clear input source',
    clearDestination: 'Clear output folder',
    openSource: 'Open input source',
    openDestination: 'Open output folder',
    pickFolder: 'Select Folder',
    pickSource: 'Choose Source',
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
    conversionMode: 'Conversion flow',
    scanThenConvert: 'Scan then convert',
    directConvert: 'Direct convert',
    enhancedAnalysis: 'Analysis enhancement',
    standardConvert: 'Standard',
    enhancedMode: 'Enhanced',
    enhancedModeOffNote: 'Convert only; music analysis stays off',
    enhancedModeOnNote: 'Analyze BPM, key, loudness, and energy and write metadata',
    advancedOptions: 'Advanced options',
    losslessFormat: 'Lossless format',
    syncSlot: 'Task',
    fallback: 'Use output directory 1 when empty',
    fallbackMissing: 'Output directory 1 is also empty',
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
    exportReport: 'Export full error report',
    completedCount: 'Completed',
    failedCount: 'Failed',
    sourcePath: 'Input source',
    destinationPath: 'Output',
    conflictStrategy: 'Existing files',
    conflictSkip: 'Existing file: skip',
    conflictOverwrite: 'Existing file: overwrite',
    conflictMetadata: 'Advanced: update metadata only',
    filenameRule: 'Filename rule',
    neteaseFilenameFormat: 'NetEase source filename format',
    neteaseTitleOnly: 'Title only',
    neteaseArtistTitle: 'Artist - Title',
    neteaseTitleArtist: 'Title - Artist',
    titleArtist: 'Title - Artist (default)',
    artistTitle: 'Artist - Title',
    originalName: 'Keep original filename',
    availableSpace: 'Available space',
    insufficientSpace: 'Not enough disk space to start',
    cancelTask: 'Cancel task',
    resumeTasks: 'Resume unfinished tasks',
    deleteHistory: 'Delete entry',
    clearHistory: 'Clear history',
    historyLoadError: 'Conversion history could not be read. Existing records were not overwritten. Check the history file and try again.',
    about: 'About',
    tutorial: 'Tutorial',
    helpTitle: 'Help',
    helpIntro: 'A quick guide to output, analysis, and conversion settings whenever you need it.',
    helpOutputTitle: 'Output and analysis',
    helpCompatibilityTitle: 'Standard conversion',
    helpCompatibilityBody: 'With Enhanced mode off, W4DJ performs a regular format conversion.',
    helpEnhancedTitle: 'Enhanced conversion',
    helpEnhancedBody: 'With Enhanced mode on, W4DJ automatically analyzes BPM, key, loudness, and energy, then writes them to the output metadata.',
    helpConversionTitle: 'Conversion flow',
    helpScanThenConvertBody: 'Click Start to scan first. W4DJ shows duplicates, errors, and estimated output before you confirm the conversion.',
    helpDirectConvertBody: 'After checking input, output, disk space, and file readability, W4DJ converts immediately without a confirmation page.',
    version: 'Version',
    developer: 'Developer',
    projectHome: 'Project home',
    checkUpdates: 'Check for updates',
    updateAvailable: 'Version {version} is available',
    alreadyLatest: 'You are up to date',
    viewRelease: 'View release',
    close: 'Close',
    pendingCount: 'Pending',
    errorCategory: 'Error type',
    onboardingTitle: 'New to W4DJ?',
    onboardingIntro: 'Five steps to convert. Folders and single tracks are detected automatically.',
    onboardingStepOneTitle: 'Choose an output mode',
    onboardingStepOneBody: 'Compat mode exports MP3. Lossless mode lets you choose WAV or AIFF.',
    onboardingStepTwoTitle: 'Drop in a source',
    onboardingStepTwoBody: 'Drop a folder or a single track into any task source box. W4DJ detects it automatically.',
    onboardingStepThreeTitle: 'Choose an output folder',
    onboardingStepThreeBody: 'Set where converted files are saved. Task 2 uses Task 1’s folder when left empty.',
    onboardingStepFourTitle: 'Start converting',
    onboardingStepFourBody: 'When ready, click here. W4DJ scans the tasks first and asks you to confirm.',
    onboardingStepFiveTitle: 'Review this guide anytime',
    onboardingStepFiveBody: 'Open Tutorial in the top right, then choose “Review the usage guide” to replay this walkthrough.',
    onboardingNext: 'Next',
    onboardingPrevious: 'Back',
    onboardingSkip: 'Skip tour',
    onboardingFinish: 'Done',
    usageGuide: 'View usage guide again',
    analysisTitle: 'Music analysis',
    analysisBody: 'Scan output folders and write BPM, key, loudness, and energy.',
    analyzeLibrary: 'Analyze library',
    exportRekordbox: 'Export Rekordbox XML',
    analysisIdle: 'Choose an output folder before analyzing.',
    analysisRunning: 'Analyzing music library…',
    analysisCancelled: 'Enhanced analysis cancelled; completed results were saved.',
    analysisComplete: '{count} results saved. Ready for Rekordbox.',
    analysisPartial: 'Completed {done}/{total}; {failed} failed.',
    analysisNoResults: 'No analysis result was completed.',
    clearAnalysisCache: 'Clear analysis cache',
    clearAnalysisCacheConfirm: 'Clear the saved music analysis cache? Audio files and conversion history will not be deleted.',
    analysisCacheCleared: 'Music analysis cache cleared.',
    clearEnhancedCache: 'Clear enhanced-mode cache',
    clearEnhancedCacheConfirm: 'Clear enhanced-mode cache? Audio files, scan cache, and downloaded models will not be deleted.',
    enhancedCacheCleared: 'Enhanced-mode cache cleared.',
    clearScanCache: 'Clear scan cache',
    clearScanCacheConfirm: 'Clear the scan cache? The next run will scan all songs again. Enhanced-mode cache and models will not be deleted.',
    scanCacheCleared: 'Scan cache cleared.',
    essentiaModelsTitle: 'Essentia pretrained models',
    essentiaModelsReady: 'Downloaded; Enhanced mode can identify genre, mood, and voice/instrument.',
    essentiaModelsMissing: 'Not downloaded; basic analysis and Drop LUFS still work.',
    essentiaModelsDownload: 'Download analysis models',
    essentiaModelsDownloading: 'Downloading models…',
    essentiaModelsDownloaded: 'Essentia pretrained models downloaded.',
    essentiaModelsPartial: 'The full model set is not ready; basic analysis still works.',
    scanTitle: 'Scanning songs',
    scanPreparing: 'Preparing scan',
    scanSource: 'Scanning input folders',
    scanDestination: 'Scanning output folders',
    scanChecking: 'Checking conversion conditions',
    scanAnalyzing: 'Analyzing tracks and writing metadata',
    scanCompleted: 'Scan complete',
    scanCancelled: 'Scan cancelled',
    scanError: 'Scan failed',
    scanCurrentFile: 'Current file',
    scanCancel: 'Cancel scan',
    conversionCancel: 'Cancel conversion',
    conversionRunning: 'Converting',
    analysisCancel: 'Cancel analysis',
    scanClose: 'Close',
  },
} as const;

function t(key: keyof typeof translations.zh, lang: AppLanguage): string {
  return translations[lang][key];
}

export function humanizeError(
  message: string,
  lang: AppLanguage,
  category?: AppErrorCategory,
): string {
  const normalized = message.toLowerCase();
  const isZh = lang === 'zh';

  if (category === 'file_damaged' || normalized.includes('no such file') || normalized.includes('无法读取')) {
    return isZh ? '歌曲文件无法读取，可能已损坏。' : 'The song file could not be read and may be damaged.';
  }
  if (category === 'unsupported_format' || normalized.includes('unsupported')) {
    return isZh ? '暂不支持这个音频格式。' : 'This audio format is not supported yet.';
  }
  if (category === 'output_permission' || normalized.includes('permission denied')) {
    return isZh ? '没有权限写入这个文件夹，请换一个输出目录。' : 'You cannot write to this folder. Choose another output folder.';
  }
  if (category === 'disk_space' || normalized.includes('no space')) {
    return isZh ? '磁盘空间不足，请清理空间后重试。' : 'There is not enough disk space. Free up space and try again.';
  }
  if (category === 'invalid_filename' || normalized.includes('invalid filename')) {
    return isZh ? '歌曲文件名无法使用，软件会尝试自动修正。' : 'The song filename is not allowed. W4DJ will try to fix it.';
  }
  if (category === 'ffmpeg' || normalized.includes('ffmpeg') || normalized.includes('conversion failed')) {
    return isZh ? '歌曲转换失败，请检查文件或重试。' : 'Conversion failed. Check the file or try again.';
  }

  return message;
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
  conversionMode: 'scan_then_convert',
  enhancedMode: false,
  conflictStrategy: 'skip',
  filenameRule: 'title_artist',
  neteaseFilenameFormat: 'title_artist',
  lang: initialLanguage,
  theme: initialTheme,
};

const defaultAnalysisState: AppAnalysisState = {
  status: 'idle',
  completed: 0,
  total: 0,
  resultCount: 0,
  failedCount: 0,
  message: '',
};

type SourcePickerOpenOptions = {
  directory: boolean;
  title: string;
  filters?: Array<{ name: string; extensions: string[] }>;
};

export async function pickSourceWithPlatformDialog(
  title: string,
  lang: AppLanguage,
  chooseSourceType: () => Promise<SourcePickerChoice>,
  openSource: (options: SourcePickerOpenOptions) => Promise<string | null>,
): Promise<string | null> {
  const choice = await chooseSourceType();
  if (choice === 'cancel') {
    return null;
  }

  return openSource({
    directory: choice === 'folder',
    title,
    ...(choice === 'folder'
      ? {}
      : {
          filters: [
            {
              name: lang === 'zh' ? '支持的音频文件' : 'Supported audio files',
              extensions: ['mp3', 'flac', 'ncm', 'wav', 'aiff'],
            },
          ],
        }),
  });
}

const defaultServices: AppServices = {
  loadDesktopState: () => invoke<DesktopState>('load_desktop_state'),
  pickDirectory: async (_kind, slotIndex) => {
    const lang = (localStorage.getItem('w4dj_lang') as AppLanguage) || 'zh';
    const slotNumber = slotIndex + 1;
    const title = lang === 'zh' ? `选择输出目录 ${slotNumber}` : `Select output folder ${slotNumber}`;
    const selected = await open({
      directory: true,
      multiple: false,
      title,
    });

    return typeof selected === 'string' ? selected : null;
  },
  pickSource: async (slotIndex) => {
    const lang = (localStorage.getItem('w4dj_lang') as AppLanguage) || 'zh';
    const title = lang === 'zh' ? `选择来源 ${slotIndex + 1}` : `Choose source ${slotIndex + 1}`;
    try {
      return await invoke<string | null>('pick_source_path', { title });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (!message.includes('unified source picker is only available on macOS')) {
        throw error;
      }

      console.warn('Unified source picker unavailable; falling back to the platform picker.', error);
      const choice = await message(
        lang === 'zh'
          ? '请选择来源类型。选择“文件夹”可扫描整组歌曲，选择“单曲”可转换一个音频文件。'
          : 'Choose a source type. Select “Folder” to scan a music folder, or “Track” to convert one audio file.',
        {
          title,
          kind: 'info',
          buttons: lang === 'zh'
            ? { yes: '文件夹', no: '单曲', cancel: '取消' }
            : { yes: 'Folder', no: 'Track', cancel: 'Cancel' },
        },
      );

      const sourceType: SourcePickerChoice =
        choice === '文件夹' || choice === 'Folder'
          ? 'folder'
          : choice === '取消' || choice === 'Cancel'
            ? 'cancel'
            : 'track';

      return pickSourceWithPlatformDialog(
        title,
        lang,
        async () => sourceType,
        async (options) => {
          const selected = await open({ ...options, multiple: false });
          return typeof selected === 'string' ? selected : null;
        },
      );
    }
  },
  selectSourceDirectory: (slotIndex, path) =>
    invoke<DesktopState>('select_source_directory', { slotIndex, path }),
  selectDestinationDirectory: (slotIndex, path) =>
    invoke<DesktopState>('select_destination_directory', { slotIndex, path }),
  chooseMode: (mode) => invoke<DesktopState>('choose_mode', { mode }),
  chooseLosslessFormat: (format) =>
    invoke<DesktopState>('choose_lossless_format', { format }),
  chooseConversionMode: (mode) =>
    invoke<DesktopState>('choose_conversion_mode', { mode }),
  chooseEnhancedMode: (enabled) =>
    invoke<DesktopState>('choose_enhanced_mode', { enabled }),
  chooseConflictStrategy: (strategy) =>
    invoke<DesktopState>('choose_conflict_strategy', { strategy }),
  chooseFilenameRule: (rule) => invoke<DesktopState>('choose_filename_rule', { rule }),
  chooseNeteaseFilenameFormat: (format) =>
    invoke<DesktopState>('choose_netease_filename_format', { format }),
  previewAllSync: () => invoke<AppPreview[]>('preview_all_sync'),
  startScan: () => invoke<AppScanProgress>('start_scan'),
  loadScanState: () => invoke<AppScanProgress>('load_scan_state'),
  loadScanResult: () => invoke<AppPreview[]>('load_scan_result'),
  cancelScan: () => invoke<AppScanProgress>('cancel_scan'),
  clearScanCache: () => invoke<void>('clear_scan_cache'),
  startConfirmedSync: (
    previews,
    retryOf = null,
    analyses = [],
    analysisFailures = [],
    batchId = null,
  ) =>
    invoke<DesktopState>('start_confirmed_sync', {
      previews,
      retryOf,
      analyses,
      analysisFailures,
      batchId,
    }),
  applyTrackAnalysisResults: (batchId, previews, analyses, analysisFailures) =>
    invoke<DesktopState>('apply_track_analysis_results', {
      batchId,
      previews,
      analyses,
      analysisFailures,
    }),
  loadHistory: () => invoke<AppHistoryEntry[]>('load_history'),
  retryHistoryFailures: (id) => invoke<AppPreview>('retry_history_failures', { id }),
  exportHistoryErrorReport: (id, path) =>
    invoke<void>('export_history_error_report', { id, path }),
  deleteHistoryEntry: (id) => invoke<void>('delete_history_entry_command', { id }),
  clearHistory: () => invoke<void>('clear_history_command'),
  loadAppInfo: () => invoke<AppInfo>('app_info'),
  checkForUpdates: () => invoke<AppUpdateCheck>('check_for_updates'),
  openExternalUrl: (url) => invoke<void>('open_external_url', { url }),
  openDestination: (path) => invoke<void>('open_destination', { path }),
  openSource: (path) => invoke<void>('open_source', { path }),
  startAllSync: () => invoke<DesktopState>('start_all_sync'),
  pauseAllSync: () => invoke<DesktopState>('pause_all_sync'),
  cancelSync: (slotIndex) => invoke<DesktopState>('cancel_sync', { slotIndex }),
  cancelAllSync: () => invoke<DesktopState>('cancel_all_sync'),
  listAudioFiles: (path) => invoke<string[]>('list_audio_files', { path }),
  readAudioFile: (path) => invoke<number[]>('read_audio_file', { path }),
  readTrackMetadata: (path) => invoke<TrackMetadata>('read_audio_metadata', { path }),
  getAudioFileFingerprint: (path) => invoke<AppAudioFileFingerprint>('get_audio_file_fingerprint', { path }),
  loadTrackAnalyses: () => invoke<TrackAnalysis[]>('load_track_analyses'),
  saveTrackAnalyses: (entries) => invoke<number>('save_track_analyses', { entries }),
  clearTrackAnalyses: () => invoke<void>('clear_track_analyses'),
  exportRekordboxXml: (path) => invoke<void>('export_rekordbox_xml', { path }),
  getEssentiaModelStatus: () => invoke<EssentiaModelStatus>('get_essentia_model_status'),
  downloadEssentiaModels: () => invoke<EssentiaModelStatus>('download_essentia_models'),
  loadEssentiaModel: (id) => invoke<EssentiaModelFile>('load_essentia_model', { id }),
  loadLibraryStatus: () => invoke<LibraryStatus>('load_library_status'),
  locateNeteaseLibrary: () => invoke<LibraryStatus['netease']>('locate_netease_library'),
  refreshLibraryCatalog: () => invoke<unknown>('refresh_library_catalog'),
  queryLibraryCatalog: (query) => invoke<LibraryPage>('query_library_catalog', { query }),
  getLibraryTrackDetail: (trackKey) => invoke<LibraryTrack | null>('get_library_track_detail', { trackKey }),
  getLibraryTrackSourceRecords: (trackKey) => invoke<LibrarySourceRecord[]>('get_library_track_source_records', { trackKey }),
  getLibraryTrackCover: (trackKey) => invoke<string | null>('get_library_track_cover', { trackKey }),
  clearLibraryCatalogCache: () => invoke<void>('clear_library_catalog_cache'),
};

export function renderApp(
  state: AppViewState = defaultState,
  pendingAction: PendingGlobalAction = null,
  selectionMotion: SelectionMotion = null,
  previewModal: AppPreviewModalState | null = null,
  history: AppHistoryEntry[] = [],
  pendingSelection: PendingSelection = null,
  previewBusy = false,
  aboutInfo: AppInfo | null = null,
  outputSettingsExpanded = false,
  historyExpanded = false,
  onboardingVisible = false,
  onboardingStep: OnboardingStep = 0,
  analysisState: AppAnalysisState = defaultAnalysisState,
  scanProgress: AppScanProgress | null = null,
  helpVisible = false,
  updateInfo: AppUpdateCheck | null = null,
  historyLoadError: string | null = null,
  modelStatus: EssentiaModelStatus = defaultEssentiaModelStatus,
  libraryState: LibraryDashboardState | null = null,
): HTMLElement {
  const root = document.createElement('main');
  root.className = 'app-shell';
  root.dataset.status = aggregateStatus(state);
  root.dataset.theme = state.theme;
  root.dataset.lightPalette = LIGHT_PALETTE;
  root.dataset.onboardingActive = onboardingVisible ? 'true' : 'false';
  root.dataset.onboardingStep = String(onboardingStep);
  if (selectionMotion) {
    root.dataset.selectionMotion = selectionMotion;
  }
  const isRunning = state.slots.some((slot) => slot.status === 'running');
  const scanRunning = scanProgress?.status === 'running';
  const analysisRunning = analysisState.status === 'running';
  const conversionRunning = isRunning && !scanRunning && !analysisRunning;
  const hasCancelled = state.slots.some((slot) => slot.status === 'cancelled');
  const configuredTasks = state.slots.filter((slot) => slot.sourceDirectory.trim()).length;
  const onboardingTarget: OnboardingTarget | null = onboardingVisible
    ? (['mode', 'source', 'destination', 'start', 'tutorial'] as const)[onboardingStep]
    : null;
  root.innerHTML = `
    <header class="topbar">
      <div class="brand-block">
        <p class="eyebrow">${t('eyebrow', state.lang)}</p>
        <h1>${t('title', state.lang)}</h1>
      </div>
      <div class="topbar-actions">
        <button type="button" class="help-button" data-action="open-help"${onboardingTarget === 'tutorial' ? ' data-onboarding-target="tutorial"' : ''} aria-label="${t('tutorial', state.lang)}" title="${t('tutorial', state.lang)}">
          ${icon('help')}
          <span>${t('tutorial', state.lang)}</span>
        </button>
        <button type="button" class="help-button" data-action="open-library" aria-label="${state.lang === 'zh' ? '歌曲库' : 'Song library'}" title="${state.lang === 'zh' ? '歌曲库' : 'Song library'}">
          ${icon('list')}
          <span>${state.lang === 'zh' ? '歌曲库' : 'Library'}</span>
        </button>
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
            <span>${t('conversionMode', state.lang)}</span>
          </div>
          <div class="conversion-mode-row mode-row" data-role="conversion-mode-switch" data-selected-conversion-mode="${state.conversionMode}" aria-label="${t('conversionMode', state.lang)}">
            <button type="button" class="mode-button ${state.conversionMode === 'scan_then_convert' ? 'selected' : ''}" data-conversion-mode="scan_then_convert" ${pendingAction !== null ? 'disabled' : ''}>
              ${icon('list')}
              ${t('scanThenConvert', state.lang)}
            </button>
            <button type="button" class="mode-button ${state.conversionMode === 'direct' ? 'selected' : ''}" data-conversion-mode="direct" ${pendingAction !== null ? 'disabled' : ''}>
              ${icon('play')}
              ${t('directConvert', state.lang)}
            </button>
            <span
              class="mode-selected-labels mode-selected-labels-vertical"
              data-role="conversion-mode-label-overlay"
              aria-hidden="true"
            >
              <span class="mode-selected-label">
                ${icon('list')}
                ${t('scanThenConvert', state.lang)}
              </span>
              <span class="mode-selected-label">
                ${icon('play')}
                ${t('directConvert', state.lang)}
              </span>
            </span>
          </div>
          <div class="global-control-head">
            <span>${t('mode', state.lang)}</span>
          </div>
          <div class="mode-row" data-role="mode-switch"${onboardingTarget === 'mode' ? ' data-onboarding-target="mode"' : ''} data-selected-mode="${state.mode}" aria-label="${t('mode', state.lang)}">
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
          <div
            class="enhanced-mode-row mode-row"
            data-role="enhanced-mode-switch"
            data-selected-enhanced-mode="${state.enhancedMode ? 'on' : 'off'}"
            aria-label="${t('enhancedAnalysis', state.lang)}"
          >
            <button
              type="button"
              class="mode-button ${state.enhancedMode ? '' : 'selected'}"
              data-enhanced-mode="off"
              title="${t('enhancedModeOffNote', state.lang)}"
              ${isRunning || pendingAction !== null ? 'disabled' : ''}
            >
              ${icon('convert')}
              ${t('standardConvert', state.lang)}
            </button>
            <button
              type="button"
              class="mode-button ${state.enhancedMode ? 'selected' : ''}"
              data-enhanced-mode="on"
              title="${t('enhancedModeOnNote', state.lang)}"
              ${isRunning || pendingAction !== null ? 'disabled' : ''}
            >
              ${icon('disc')}
              ${t('enhancedMode', state.lang)}
            </button>
            <span
              class="mode-selected-labels mode-selected-labels-horizontal"
              data-role="enhanced-mode-label-overlay"
              aria-hidden="true"
            >
              <span class="mode-selected-label">
                ${icon('convert')}
                ${t('standardConvert', state.lang)}
              </span>
              <span class="mode-selected-label">
                ${icon('disc')}
                ${t('enhancedMode', state.lang)}
              </span>
            </span>
          </div>
          ${renderOutputSettings(state, outputSettingsExpanded, modelStatus)}
          <div class="global-action-group">
            <button type="button" class="global-action"${onboardingTarget === 'start' ? ' data-onboarding-target="start"' : ''} data-action="${scanRunning ? 'cancel-scan' : analysisRunning ? 'cancel-analysis' : conversionRunning ? 'cancel-all' : 'start-all'}" ${
              !scanRunning && !analysisRunning && !conversionRunning && (configuredTasks === 0 || pendingAction !== null) ? 'disabled' : ''
            } aria-busy="${pendingAction !== null}">
              ${scanRunning || analysisRunning || conversionRunning ? icon('pause') : icon('play')}
              ${scanRunning
                ? t('scanCancel', state.lang)
                : analysisRunning
                  ? t('analysisCancel', state.lang)
                  : conversionRunning
                    ? t('conversionCancel', state.lang)
                    : hasCancelled ? t('resumeTasks', state.lang) : t('startAll', state.lang)}
            </button>
            ${scanProgress && (scanProgress.status === 'error' || scanProgress.status === 'cancelled')
              ? `<small class="global-stage-message" data-role="scan-message">${escapeHtml(scanProgress.message || scanPhaseLabel(scanProgress.phase, state.lang))}</small>`
              : analysisRunning
                ? `<small class="global-stage-message" data-role="analysis-message">${t('analysisRunning', state.lang)} ${analysisState.completed}/${analysisState.total}</small>`
                : analysisState.status === 'cancelled'
                  ? `<small class="global-stage-message" data-role="analysis-message">${escapeHtml(analysisState.message || t('analysisCancelled', state.lang))}</small>`
                : conversionRunning
                  ? `<small class="global-stage-message" data-role="conversion-message">${t('conversionRunning', state.lang)}</small>`
                  : ''}
          </div>
        </div>
      </aside>

      <div class="workbench-main" data-role="workbench-main">
        <div class="workspace-intro">
          <p class="panel-kicker">${t('sourceKicker', state.lang)}</p>
        </div>
        <div class="sync-slots">
          ${renderSyncSlot(
            state,
            0,
            onboardingTarget === 'source' || onboardingTarget === 'destination' ? onboardingTarget : null,
            scanProgress?.tasks?.find((task) => task.slot_index === 0),
            scanRunning,
          )}
          ${renderSyncSlot(state, 1, null, scanProgress?.tasks?.find((task) => task.slot_index === 1), scanRunning)}
        </div>
        ${renderHistory(history, state.lang, historyExpanded, historyLoadError)}
      </div>
    </section>
    ${renderPreviewModal(previewModal, state.lang, previewBusy)}
    ${renderAboutModal(aboutInfo, updateInfo, state.lang)}
    ${renderHelpModal(helpVisible, state.lang)}
    ${renderOnboardingModal(onboardingVisible, state.lang, onboardingStep)}
    ${renderLibraryDashboard(libraryState, state.lang)}
  `;

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
      (issue) => `<li>${escapeHtml(issue.path)}：${escapeHtml(humanizeError(issue.message, lang))}</li>`,
    ),
    ...preview.warnings.map(
      (issue) => `<li class="preview-warning">${escapeHtml(issue.path)}：${escapeHtml(humanizeError(issue.message, lang))}</li>`,
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

function renderHistory(
  entries: AppHistoryEntry[],
  lang: AppLanguage,
  expanded = false,
  historyLoadError: string | null = null,
): string {
  return `
    <details class="history-panel" data-role="history" ${expanded ? 'open' : ''}>
      <summary class="history-head">
        <div>
          <p class="panel-kicker">W4DJ RKB</p>
          <h2>${t('history', lang)}</h2>
        </div>
      </summary>
      <div class="history-body">
        ${(entries.length > 0 || historyLoadError) ? `<div class="history-body-actions"><button type="button" class="secondary-action history-clear" data-action="clear-history">${t('clearHistory', lang)}</button></div>` : ''}
        ${historyLoadError
          ? `<p class="history-error">${escapeHtml(historyLoadError)}</p>`
          : entries.length === 0
          ? `<p class="history-empty">${t('noHistory', lang)}</p>`
          : `<div class="history-list">${entries.map((entry) => renderHistoryEntry(entry, lang)).join('')}</div>`}
      </div>
    </details>
  `;
}

function renderHistoryEntry(entry: AppHistoryEntry, lang: AppLanguage): string {
  const pendingFiles = entry.pending_files || [];
  const failures = entry.failed_files
    .map((failedFile) => `<li><strong>${escapeHtml(failedFile.name)}</strong><span class="failure-category">${t('errorCategory', lang)}：${errorCategoryLabel(failedFile.category, lang)}</span><span>${escapeHtml(humanizeError(failedFile.message, lang, failedFile.category))}</span></li>`)
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
        <button type="button" class="secondary-action" data-action="export-history" data-history-id="${escapeHtml(entry.id)}">${t('exportReport', lang)}</button>
        <button type="button" class="secondary-action history-delete" data-action="delete-history" data-history-id="${escapeHtml(entry.id)}">${t('deleteHistory', lang)}</button>
      </footer>
    </article>
  `;
}

function renderSyncSlot(
  state: AppViewState,
  slotIndex: SyncSlotIndex,
  onboardingTarget: 'source' | 'destination' | null = null,
  scanTask: AppScanTaskProgress | undefined = undefined,
  scanActive = false,
): string {
  const slot = state.slots[slotIndex];
  const fallbackDestination = state.slots[0].destinationDirectory;
  const usesFallback = slotIndex === 1 && slot.destinationDirectory.trim() === '';
  const displayedDestination = usesFallback ? fallbackDestination : slot.destinationDirectory;
  const slotNumber = slotIndex + 1;
  const scanPercent = scanTask && scanTask.total > 0
    ? Math.min(100, Math.round((scanTask.processed / scanTask.total) * 100))
    : 0;
  const displayedProgressText = scanActive && scanTask
    ? `${scanPhaseLabel(scanTask.phase, state.lang)} ${scanTask.processed}/${scanTask.total}`
    : slot.progressText;
  const showProgressText = scanActive && scanTask
    ? true
    : slot.status !== 'idle' && slot.progressText !== t('idle', state.lang);
  const isNumericProgress = /^\d+\/\d+$/.test(displayedProgressText);
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
          <div class="path-field" data-role="source-picker"${onboardingTarget === 'source' ? ' data-onboarding-target="source"' : ''} data-drop-kind="source" data-slot="${slotIndex}">
          <span>${t('sourceLabel', state.lang)}</span>
          <div class="path-control source-path-control">
            <button type="button" class="path-button" data-action="pick-source" data-slot="${slotIndex}">
              ${icon('folder')}
              <span class="path-copy">${displayPath(slot.sourceDirectory, state.lang, t('pickSource', state.lang))}</span>
            </button>
            <button type="button" class="path-action path-open" data-action="open-source" data-slot="${slotIndex}" aria-label="${t('openSource', state.lang)}" title="${t('openSource', state.lang)}" ${slot.sourceDirectory.trim() ? '' : 'disabled'}>
              ${icon('open')}
            </button>
            <button type="button" class="path-action path-clear" data-action="clear-source" data-slot="${slotIndex}" aria-label="${t('clearSource', state.lang)}" title="${t('clearSource', state.lang)}" ${slot.sourceDirectory.trim() ? '' : 'disabled'}>
              ${icon('trash')}
            </button>
          </div>
        </div>

        <span class="path-arrow" aria-hidden="true">${icon('arrow')}</span>

          <div class="path-field" data-role="destination-picker"${onboardingTarget === 'destination' ? ' data-onboarding-target="destination"' : ''} data-drop-kind="destination" data-slot="${slotIndex}">
          <span>${t('destLabel', state.lang)}</span>
          <div class="path-control destination-path-control">
            <button type="button" class="path-button ${usesFallback ? 'is-fallback' : ''}" data-action="pick-destination" data-slot="${slotIndex}">
              ${icon('export')}
              <span class="path-copy">${displayPath(displayedDestination, state.lang)}</span>
            </button>
            <button type="button" class="path-action path-open" data-action="open-destination" data-slot="${slotIndex}" aria-label="${t('openDestination', state.lang)}" title="${t('openDestination', state.lang)}" ${displayedDestination.trim() ? '' : 'disabled'}>
              ${icon('open')}
            </button>
            <button type="button" class="path-action path-clear" data-action="clear-destination" data-slot="${slotIndex}" aria-label="${t('clearDestination', state.lang)}" title="${t('clearDestination', state.lang)}" ${slot.destinationDirectory.trim() ? '' : 'disabled'}>
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
        ${showProgressText ? `<span class="status-copy progress-copy ${isNumericProgress ? 'progress-copy--numeric' : ''}">${escapeHtml(displayedProgressText)}</span>` : ''}
        <div class="progress-track" aria-hidden="true">
          <div class="progress-fill" style="width: ${scanActive && scanTask ? scanPercent : progressPercent(slot)}%"></div>
        </div>
      </footer>
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
  let pendingGlobalAction: PendingGlobalAction = null;
  let selectionMotion: SelectionMotion = null;
  let selectionMotionToken = 0;
  let pendingSelection: PendingSelection = null;
  let deferredDesktopRender = false;
  let previewModal: AppPreviewModalState | null = null;
  let previewBusy = false;
  let history: AppHistoryEntry[] = [];
  let historyLoadError: string | null = null;
  let aboutInfo: AppInfo | null = null;
  let updateInfo: AppUpdateCheck | null = null;
  let modelStatus: EssentiaModelStatus = defaultEssentiaModelStatus;
  let helpVisible = false;
  let outputSettingsExpanded = false;
  let historyExpanded = false;
  let scanProgress: AppScanProgress | null = null;
  let scanTimer: ReturnType<typeof setTimeout> | null = null;
  let analysisCancelRequested = false;
  let onboardingVisible = localStorage.getItem('w4dj_onboarding_seen') !== '1';
  let onboardingStep: OnboardingStep = 0;
  let analysisState: AppAnalysisState = { ...defaultAnalysisState };
  let analysisCache: TrackAnalysis[] = [];
  let analysisCacheLoadPromise: Promise<void> = Promise.resolve();
  let analysisCacheRevision = 0;
  let libraryState: LibraryDashboardState | null = null;
  let librarySearchTimer: ReturnType<typeof setTimeout> | null = null;
  let draggedLibraryColumn: string | null = null;
  let neteaseDiscoveryStarted = false;

  const loadAnalysisCache = async () => {
    const loadRevision = analysisCacheRevision;
    try {
      const entries = await services.loadTrackAnalyses();
      if (loadRevision === analysisCacheRevision) {
        analysisCache = entries;
      }
    } catch (error) {
      if (loadRevision === analysisCacheRevision) {
        analysisCache = [];
      }
      console.warn('Failed to load Essentia analysis cache; rebuilding entries:', error);
    }
  };

  const mergeAnalysisCache = (updates: TrackAnalysis[]) => {
    const entriesByPath = new Map(analysisCache.map((entry) => [entry.path, entry]));
    updates.forEach((entry) => entriesByPath.set(entry.path, entry));
    analysisCache = Array.from(entriesByPath.values()).sort((left, right) =>
      left.path.localeCompare(right.path));
  };

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
        outputSettingsExpanded,
        historyExpanded,
        onboardingVisible,
        onboardingStep,
        analysisState,
        scanProgress,
        helpVisible,
        updateInfo,
        historyLoadError,
        modelStatus,
        libraryState,
      ),
    );

    if (onboardingVisible) {
      root.querySelector<HTMLButtonElement>('[data-action="onboarding-next"]')?.focus();
    } else if (helpVisible) {
      root.querySelector<HTMLButtonElement>('[data-action="close-help"]')?.focus();
    }

    const historyDetails = root.querySelector<HTMLDetailsElement>('[data-role="history"]');
    historyDetails?.querySelector('summary')?.addEventListener('click', () => {
      historyExpanded = !historyDetails.open;
    });
  };

  const syncSlidingSelectionControl = (
    kind: 'mode' | 'format' | 'conversion-mode' | 'enhanced-mode',
    motion: 'none' | 'start' | 'clear' = 'none',
  ) => {
    const isMode = kind === 'mode';
    const isFormat = kind === 'format';
    const isConversionMode = kind === 'conversion-mode';
    const rowSelector = isMode
      ? '[data-role="mode-switch"]'
      : isFormat
        ? '.format-row'
        : isConversionMode
          ? '[data-role="conversion-mode-switch"]'
          : '[data-role="enhanced-mode-switch"]';
    const row = root.querySelector<HTMLElement>(rowSelector);
    if (!row) {
      render();
      return;
    }

    const selectedValue = isMode
      ? state.mode
      : isFormat
        ? state.losslessFormat || 'wav'
        : isConversionMode
          ? state.conversionMode
          : state.enhancedMode ? 'on' : 'off';
    const selectedAttribute = isMode
      ? 'data-selected-mode'
      : isFormat
        ? 'data-selected-format'
        : isConversionMode
          ? 'data-selected-conversion-mode'
          : 'data-selected-enhanced-mode';
    row.setAttribute(
      selectedAttribute,
      selectedValue,
    );

    const selectionPending = pendingSelection === kind;
    row.toggleAttribute('data-selection-pending', selectionPending);
    row.setAttribute('aria-busy', selectionPending ? 'true' : 'false');
    const buttonSelector = isFormat ? '.format-button' : '.mode-button';
    row.querySelectorAll<HTMLButtonElement>(buttonSelector).forEach((button) => {
      const selected = isMode
        ? button.dataset.mode === state.mode
        : isFormat
          ? button.dataset.format === selectedValue
          : isConversionMode
            ? button.dataset.conversionMode === state.conversionMode
            : button.dataset.enhancedMode === selectedValue;
      button.classList.toggle('selected', selected);

      // Do not toggle the native disabled state for an in-flight selector
      // update. WKWebView re-rasterizes disabled button labels, which makes
      // both Chinese labels visibly flash. The row blocks pointer input while
      // pending and runSelectionAction serializes keyboard-triggered clicks.
      const unavailable = pendingGlobalAction !== null
        || (!isMode && !isFormat && !isConversionMode && state.slots.some((slot) => slot.status === 'running'));
      button.disabled = unavailable;
      button.setAttribute(
        'aria-disabled',
        selectionPending || unavailable ? 'true' : 'false',
      );
    });

    if (isMode) {
      const formatRow = root.querySelector<HTMLElement>('.format-row');
      if (formatRow) {
        const losslessVisible = state.mode === 'lossless';
        formatRow.dataset.visible = String(losslessVisible);
        formatRow.setAttribute('aria-hidden', String(!losslessVisible));
      }
    }

    const shell = root.querySelector<HTMLElement>('.app-shell');
    if (motion === 'start') {
      if (shell) {
        // These selectors move through their persistent CSS transition.
        // Keeping the attribute in place lets a quick reversal continue from
        // the current interpolated position instead of flashing at an endpoint.
        shell.dataset.selectionMotion = kind;
      }
    } else if (motion === 'clear' && shell?.dataset.selectionMotion === kind) {
      delete shell.dataset.selectionMotion;
    }
  };

  const flushDeferredDesktopRender = () => {
    if (deferredDesktopRender && selectionMotion === null && pendingSelection === null) {
      deferredDesktopRender = false;
      render();
    }
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
      historyLoadError = null;
      if (renderAfter && selectionMotion === null && pendingSelection === null) {
        render();
      }
    } catch (error) {
      console.error('Failed to load conversion history:', error);
      history = [];
      historyLoadError = t('historyLoadError', state.lang);
      if (renderAfter && selectionMotion === null && pendingSelection === null) {
        render();
      }
    }
  };

  const applyDesktopState = (desktopState: DesktopState) => {
    const slidingControlActive = selectionMotion === 'mode'
      || selectionMotion === 'format'
      || selectionMotion === 'conversion-mode'
      || selectionMotion === 'enhanced-mode'
      || pendingSelection === 'mode'
      || pendingSelection === 'format'
      || pendingSelection === 'conversion-mode'
      || pendingSelection === 'enhanced-mode';
    const hydratedState = toViewState(desktopState, state.lang, state.theme);
    const nextState = slidingControlActive
      ? {
          ...hydratedState,
          // Keep a just-selected preference when a stale background snapshot
          // arrives while that preference's animation is still running.
          mode: state.mode,
          losslessFormat: state.losslessFormat,
          conversionMode: state.conversionMode,
          enhancedMode: state.enhancedMode,
        }
      : hydratedState;
    const finishedRunningTask = state.slots.some(
      (slot, index) => slot.status === 'running' && nextState.slots[index].status !== 'running',
    );
    state = nextState;
    if (slidingControlActive) {
      // Do not replace the sliding controls while an animation is active.
      // The initial desktop-state hydration can otherwise redraw the labels
      // during the first user interaction and make them flash once.
      deferredDesktopRender = true;
    } else {
      render();
    }
    void refreshHistory(finishedRunningTask);
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
        progressText: `${t('error', state.lang)}: ${humanizeError(message, state.lang)}`,
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
    const patchSlidingControlInPlace = kind === 'mode'
      || kind === 'format'
      || kind === 'conversion-mode'
      || kind === 'enhanced-mode';
    if (patchSlidingControlInPlace) {
      syncSlidingSelectionControl(kind, 'none');
    } else {
      render();
    }
    let motionToken: number | null = null;
    try {
      const nextState = await action();
      selectionMotion = kind;
      motionToken = ++selectionMotionToken;
      if (patchSlidingControlInPlace) {
        state = toViewState(nextState, state.lang, state.theme);
        syncSlidingSelectionControl(kind, 'start');
      } else {
        applyDesktopState(nextState);
      }
    } catch (error) {
      reportError(error);
    } finally {
      pendingSelection = null;
      if (patchSlidingControlInPlace) {
        syncSlidingSelectionControl(kind, 'none');
      } else {
        render();
      }
      if (motionToken !== null) {
        setTimeout(() => {
          if (selectionMotion === kind && selectionMotionToken === motionToken) {
            selectionMotion = null;
            if (patchSlidingControlInPlace) {
              syncSlidingSelectionControl(kind, 'clear');
            } else {
              render();
            }
            flushDeferredDesktopRender();
          }
        }, kind === 'mode' ? 820 : 520);
      }
      flushDeferredDesktopRender();
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

  const finishScan = async (progress: AppScanProgress) => {
    if (progress.status === 'running') {
      return;
    }
    if (scanTimer) {
      clearTimeout(scanTimer);
      scanTimer = null;
    }
    if (progress.status !== 'completed') {
      pendingGlobalAction = null;
      render();
      return;
    }

    try {
      const previews = await services.loadScanResult();
      scanProgress = null;
      if (state.conversionMode === 'scan_then_convert') {
        previewModal = { previews, retryOf: null };
        pendingGlobalAction = null;
        render();
        return;
      }

      previewBusy = true;
      const batchId = createAnalysisBatchId();
      const shouldAnalyze = state.enhancedMode;
      render();
      const nextState = await services.startConfirmedSync(
        previews,
        null,
        [],
        [],
        batchId,
      );
      applyDesktopState(nextState);
      previewBusy = false;
      pendingGlobalAction = null;
      render();
      if (shouldAnalyze) {
        void runPostConversionAnalysis(batchId, previews);
      }
    } catch (error) {
      scanProgress = {
        ...progress,
        status: 'error',
        phase: 'error',
        message: error instanceof Error ? error.message : String(error),
      };
    } finally {
      previewBusy = false;
      pendingGlobalAction = null;
      render();
    }
  };

  const pollScan = async () => {
    if (!scanProgress || scanProgress.status !== 'running') {
      return;
    }
    try {
      scanProgress = await services.loadScanState();
      render();
      if (scanProgress.status === 'running') {
        scanTimer = setTimeout(() => void pollScan(), 120);
      } else {
        await finishScan(scanProgress);
      }
    } catch (error) {
      scanProgress = {
        ...(scanProgress || {
          status: 'error', phase: 'error', processed: 0, total: 0, current_file: '', message: '',
        }),
        status: 'error',
        phase: 'error',
        message: error instanceof Error ? error.message : String(error),
      };
      pendingGlobalAction = null;
      render();
    }
  };

  const beginScan = async () => {
    if (scanProgress?.status === 'running' || pendingGlobalAction !== null) {
      return;
    }
    pendingGlobalAction = 'start-all';
    scanProgress = {
      status: 'running',
      phase: 'preparing',
      processed: 0,
      total: 0,
      current_file: '',
      message: t('scanPreparing', state.lang),
    };
    render();
    try {
      scanProgress = await services.startScan();
      render();
      if (scanProgress.status === 'running') {
        scanTimer = setTimeout(() => void pollScan(), 0);
      } else {
        await finishScan(scanProgress);
      }
    } catch (error) {
      scanProgress = {
        status: 'error',
        phase: 'error',
        processed: 0,
        total: 0,
        current_file: '',
        message: error instanceof Error ? error.message : String(error),
      };
      pendingGlobalAction = null;
      render();
    }
  };

  const cancelScanFlow = async () => {
    if (!scanProgress || scanProgress.status !== 'running') {
      return;
    }
    try {
      pendingGlobalAction = 'cancel-scan';
      scanProgress = await services.cancelScan();
      render();
    } catch (error) {
      scanProgress = { ...scanProgress, status: 'error', phase: 'error', message: String(error) };
      pendingGlobalAction = null;
      render();
    }
  };

  const cancelAnalysisFlow = () => {
    if (analysisState.status !== 'running') {
      return;
    }
    analysisCancelRequested = true;
    analysisState = {
      ...analysisState,
      status: 'cancelled',
      message: t('analysisCancelled', state.lang),
    };
    pendingGlobalAction = null;
    render();
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
      const previews = previewModal.previews;
      const retryOf = previewModal.retryOf;
      const batchId = createAnalysisBatchId();
      const shouldAnalyze = state.enhancedMode;
      const nextState = await services.startConfirmedSync(
        previews,
        retryOf,
        [],
        [],
        batchId,
      );
      previewModal = null;
      applyDesktopState(nextState);
      previewBusy = false;
      pendingGlobalAction = null;
      render();
      if (shouldAnalyze) {
        void runPostConversionAnalysis(batchId, previews);
      }
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
        defaultPath: 'W4DJ-complete-error-report.txt',
        title: state.lang === 'zh' ? '保存完整错误报告' : 'Save full error report',
      });
      if (typeof path === 'string') {
        await services.exportHistoryErrorReport(id, path);
      }
    } catch (error) {
      reportError(error);
    }
  };

  const deleteHistory = async (id: string) => {
    const message = state.lang === 'zh'
      ? '确定删除这条转换记录吗？已经生成的音频文件不会被删除。'
      : 'Delete this conversion record? Generated audio files will not be deleted.';
    if (!window.confirm(message)) {
      return;
    }
    try {
      await services.deleteHistoryEntry(id);
      await refreshHistory();
    } catch (error) {
      reportError(error);
    }
  };

  const clearAllHistory = async () => {
    const message = state.lang === 'zh'
      ? '确定清空全部转换历史吗？已经生成的音频文件不会被删除。'
      : 'Clear all conversion history? Generated audio files will not be deleted.';
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

  const clearEnhancedCache = async () => {
    if (!window.confirm(t('clearEnhancedCacheConfirm', state.lang))) {
      return;
    }
    try {
      analysisCacheRevision += 1;
      const clearedRevision = analysisCacheRevision;
      await services.clearTrackAnalyses();
      if (clearedRevision === analysisCacheRevision) {
        analysisCache = [];
      }
      analysisState = { ...defaultAnalysisState, message: t('enhancedCacheCleared', state.lang) };
      render();
      window.alert(t('enhancedCacheCleared', state.lang));
    } catch (error) {
      reportError(error);
    }
  };

  const clearScanCache = async () => {
    if (!services.clearScanCache) {
      return;
    }
    if (!window.confirm(t('clearScanCacheConfirm', state.lang))) {
      return;
    }
    try {
      await services.clearScanCache();
      window.alert(t('scanCacheCleared', state.lang));
    } catch (error) {
      reportError(error);
    }
  };

  const downloadEssentiaModels = async () => {
    if (!services.downloadEssentiaModels) {
      return;
    }
    modelStatus = { ...modelStatus, downloading: true };
    render();
    try {
      modelStatus = await services.downloadEssentiaModels();
      window.alert(modelStatus.embedding && modelStatus.genre && modelStatus.mood && modelStatus.instrument
        ? t('essentiaModelsDownloaded', state.lang)
        : t('essentiaModelsPartial', state.lang));
    } catch (error) {
      reportError(error);
    } finally {
      modelStatus = { ...modelStatus, downloading: false };
      render();
    }
  };

  const defaultLibraryQuery = (): LibraryQuery => ({
    text: '',
    filters: [],
    filterLogic: 'and',
    sorts: [],
    limit: 100,
    offset: 0,
  });

  const loadLibraryPage = async (query: LibraryQuery) => {
    if (!services.queryLibraryCatalog) return null;
    return services.queryLibraryCatalog(query);
  };

  const loadLibraryCovers = async (page: LibraryPage | null) => {
    if (!page || !services.getLibraryTrackCover || !libraryState) return;
    const candidates = page.items
      // The database can contain only a relative cover reference while the
      // real image lives in NetEase's neighbouring `meta` directory. Ask the
      // backend for every visible row so that this recovery path is not
      // hidden behind the database's boolean hint.
      .filter((track) => !libraryState?.coverData?.[track.trackKey])
      .slice(0, 24);
    if (candidates.length === 0) return;
    const resolved = await Promise.all(candidates.map(async (track) => {
      try {
        return [track.trackKey, await services.getLibraryTrackCover!(track.trackKey)] as const;
      } catch {
        return [track.trackKey, null] as const;
      }
    }));
    if (!libraryState?.visible) return;
    const coverData = { ...(libraryState.coverData || {}) };
    resolved.forEach(([key, data]) => {
      if (data) coverData[key] = data;
    });
    if (Object.keys(coverData).length !== Object.keys(libraryState.coverData || {}).length) {
      libraryState = { ...libraryState, coverData };
      render();
    }
  };

  const openLibrary = async () => {
    if (!services.loadLibraryStatus || !services.queryLibraryCatalog) {
      libraryState = {
        visible: true,
        busy: false,
        status: null,
        page: null,
        query: defaultLibraryQuery(),
        detail: null,
        error: state.lang === 'zh' ? '当前版本没有启用歌曲库服务。' : 'Song library service is unavailable.',
        coverData: {},
      };
      render();
      return;
    }
    const query = defaultLibraryQuery();
    libraryState = { visible: true, busy: true, status: null, page: null, query, detail: null, error: null, coverData: {} };
    render();
    try {
      const status = await services.loadLibraryStatus();
      const page = await loadLibraryPage(query);
      libraryState = { visible: true, busy: false, status, page, query, detail: null, error: null, coverData: {} };
      void loadLibraryCovers(page);
    } catch (error) {
      libraryState = {
        ...libraryState,
        busy: false,
        error: error instanceof Error ? error.message : String(error),
      };
    }
    render();
  };

  const refreshLibrary = async () => {
    if (!libraryState || !services.refreshLibraryCatalog || !services.loadLibraryStatus) return;
    libraryState = { ...libraryState, busy: true, error: null };
    render();
    try {
      await services.refreshLibraryCatalog();
      const status = await services.loadLibraryStatus();
      const page = services.queryLibraryCatalog
        ? await services.queryLibraryCatalog(libraryState.query)
        : libraryState.page;
      libraryState = { ...libraryState, busy: false, status, page };
      void loadLibraryCovers(page);
    } catch (error) {
      libraryState = { ...libraryState, busy: false, error: error instanceof Error ? error.message : String(error) };
    }
    render();
  };

  const clearLibraryCache = async () => {
    if (!libraryState || !services.clearLibraryCatalogCache) return;
    if (!window.confirm(state.lang === 'zh'
      ? '确定清除歌曲库索引吗？网易云数据库、音乐文件、扫描缓存和增强分析缓存不会被删除。'
      : 'Clear the song-library index? NetEase data, audio files, scan cache and analysis cache will stay intact.')) {
      return;
    }
    try {
      await services.clearLibraryCatalogCache();
      libraryState = { ...libraryState, page: null, status: null, detail: null, error: null };
      render();
    } catch (error) {
      libraryState = { ...libraryState, error: error instanceof Error ? error.message : String(error) };
      render();
    }
  };

  const queryLibrary = async (query: LibraryQuery) => {
    if (!libraryState || !services.queryLibraryCatalog) return;
    try {
      const page = await services.queryLibraryCatalog(query);
      if (libraryState.visible) {
        libraryState = { ...libraryState, query, page, error: null };
        void loadLibraryCovers(page);
        render();
      }
    } catch (error) {
      libraryState = { ...libraryState, error: error instanceof Error ? error.message : String(error) };
      render();
    }
  };

  const openLibraryDetail = async (trackKey: string) => {
    if (!libraryState || !services.getLibraryTrackDetail) return;
    try {
      const [detail, sourceRecords] = await Promise.all([
        services.getLibraryTrackDetail(trackKey),
        services.getLibraryTrackSourceRecords?.(trackKey) ?? Promise.resolve([]),
      ]);
      if (libraryState.visible) {
        libraryState = { ...libraryState, detail, sourceRecords };
        render();
      }
    } catch (error) {
      libraryState = { ...libraryState, error: error instanceof Error ? error.message : String(error) };
      render();
    }
  };

  const selectedLibraryLyrics = (): string => {
    const track = libraryState?.detail;
    if (!track) return '';
    switch (libraryState?.lyricsTab || 'plain') {
      case 'translated': return track.lyricTranslatedText;
      case 'romanized': return track.lyricRomanizedText;
      case 'lrc': return track.lyricLrcText;
      default: return track.lyricPlainText;
    }
  };

  const copyLibraryLyrics = async () => {
    const text = selectedLibraryLyrics();
    if (!text) return;
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
      } else {
        const textarea = document.createElement('textarea');
        textarea.value = text;
        textarea.style.position = 'fixed';
        textarea.style.opacity = '0';
        document.body.append(textarea);
        textarea.select();
        document.execCommand('copy');
        textarea.remove();
      }
    } catch (error) {
      reportError(error);
    }
  };

  const downloadLibraryLyrics = () => {
    const track = libraryState?.detail;
    const text = selectedLibraryLyrics();
    if (!track || !text) return;
    const extension = libraryState?.lyricsTab === 'lrc' ? 'lrc' : 'txt';
    const filename = `${(track.title || 'lyrics').replace(/[\\/:*?"<>|]/g, '_')}.${extension}`;
    const url = URL.createObjectURL(new Blob([text], { type: 'text/plain;charset=utf-8' }));
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = filename;
    anchor.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  };

  const discoverNeteaseAfterOnboarding = async () => {
    if (neteaseDiscoveryStarted || !services.locateNeteaseLibrary) return;
    neteaseDiscoveryStarted = true;
    try {
      const discovery = await services.locateNeteaseLibrary();
      if (discovery.musicFolder && !state.slots[0].sourceDirectory.trim()) {
        const nextState = await services.selectSourceDirectory(0, discovery.musicFolder);
        applyDesktopState(nextState);
      }
      if (discovery.databasePath && services.refreshLibraryCatalog) {
        void services.refreshLibraryCatalog().catch((error) => {
          console.warn('NetEase library auto-refresh failed:', error);
        });
      }
    } catch (error) {
      console.warn('NetEase library auto-discovery failed:', error);
    }
  };

  const yieldToUi = async () => {
    await new Promise<void>((resolve) => {
      if (typeof requestAnimationFrame === 'function') {
        requestAnimationFrame(() => resolve());
      } else {
        setTimeout(resolve, 0);
      }
    });
  };

  const analyzePreviewCandidates = async (
    previews: AppPreview[],
  ): Promise<{ analyses: TrackAnalysis[]; failures: AppAnalysisFailure[]; cancelled: boolean }> => {
    analysisCancelRequested = false;
    const candidates = Array.from(new Map(
      previews
        .flatMap((item) => item.preview.candidates)
        .map((candidate) => [candidate.source_path, candidate]),
    ).values());
    if (candidates.length === 0) {
      return { analyses: [], failures: [], cancelled: false };
    }

    analysisState = {
      status: 'running',
      completed: 0,
      total: candidates.length,
      resultCount: analysisState.resultCount,
      failedCount: 0,
      message: t('scanAnalyzing', state.lang),
    };
    render();

    const results: TrackAnalysis[] = [];
    const freshResults: TrackAnalysis[] = [];
    const failures: AppAnalysisFailure[] = [];
    let failedCount = 0;
    const analysisCacheRevisionAtStart = analysisCacheRevision;
    const persistFreshResults = async () => {
      if (freshResults.length === 0 || analysisCacheRevisionAtStart !== analysisCacheRevision) {
        return;
      }
      try {
        await services.saveTrackAnalyses(freshResults);
        if (analysisCacheRevisionAtStart === analysisCacheRevision) {
          mergeAnalysisCache(freshResults);
        }
      } catch (error) {
        console.warn('Failed to save Essentia analysis cache:', error);
      }
    };
    const finishCancelledAnalysis = async () => {
      await persistFreshResults();
      analysisState = {
        ...analysisState,
        status: 'cancelled',
        resultCount: results.length,
        failedCount,
        message: t('analysisCancelled', state.lang),
      };
      render();
      return { analyses: results, failures, cancelled: true };
    };
    await analysisCacheLoadPromise;
    const cacheByPath = new Map(analysisCache.map((entry) => [entry.path, entry]));
    let highLevelModels: EssentiaModelFile[] | undefined;
    if (state.enhancedMode && services.loadEssentiaModel
      && modelStatus.embedding && modelStatus.genre && modelStatus.mood && modelStatus.instrument) {
      try {
        highLevelModels = await Promise.all(
          ESSENTIA_MODEL_IDS.map((id) => services.loadEssentiaModel?.(id) as Promise<EssentiaModelFile>),
        );
      } catch (error) {
        console.warn('Failed to load Essentia high-level models; continuing with basic analysis:', error);
      }
    }

    if (analysisCancelRequested) {
      return finishCancelledAnalysis();
    }

    for (const candidate of candidates) {
      if (analysisCancelRequested) {
        return finishCancelledAnalysis();
      }
      let fingerprint: AppAudioFileFingerprint | null = null;
      try {
        fingerprint = await services.getAudioFileFingerprint(candidate.source_path);
      } catch (error) {
        console.warn(`Failed to fingerprint ${candidate.source_path}; reanalyzing it:`, error);
      }

      const cached = cacheByPath.get(candidate.source_path);
      const highLevelModelsAvailable = Boolean(
        state.enhancedMode
        && highLevelModels?.length === ESSENTIA_MODEL_IDS.length
        && modelStatus.version,
      );
      const canReuse = fingerprint !== null
        && canReuseTrackAnalysis(
          cached,
          fingerprint,
          state.neteaseFilenameFormat,
          modelStatus.version || null,
          highLevelModelsAvailable,
        );

      if (canReuse && cached) {
        results.push(cached);
      } else {
        try {
          const bytes = await services.readAudioFile(candidate.source_path);
          let metadata: TrackMetadata | undefined;
          try {
            metadata = await services.readTrackMetadata(candidate.source_path);
          } catch {
            // Analysis can continue using the filename identity.
          }
          const analysis = await analyzeAudioFile(
            candidate.source_path,
            Uint8Array.from(bytes),
            metadata,
            {
              fingerprint: fingerprint || undefined,
              neteaseFilenameFormat: state.neteaseFilenameFormat,
              highLevelModels,
            },
          );
          results.push(analysis);
          freshResults.push(analysis);
        } catch (error) {
          failedCount += 1;
          failures.push({
            path: candidate.source_path,
            message: error instanceof Error ? error.message : String(error),
          });
          console.warn(`Essentia analysis failed for ${candidate.source_path}`, error);
        }
      }
      if (analysisCancelRequested) {
        return finishCancelledAnalysis();
      }
      analysisState = {
        ...analysisState,
        completed: analysisState.completed + 1,
        failedCount,
        message: t('scanAnalyzing', state.lang),
      };
      render();
      await yieldToUi();
    }

    analysisState = {
      ...analysisState,
      status: results.length > 0 ? 'completed' : 'error',
      resultCount: results.length,
      failedCount,
      message: failedCount > 0
        ? t('analysisPartial', state.lang)
          .replace('{done}', String(results.length))
          .replace('{total}', String(candidates.length))
          .replace('{failed}', String(failedCount))
        : t('analysisComplete', state.lang).replace('{count}', String(results.length)),
    };
    await persistFreshResults();
    render();
    return { analyses: results, failures, cancelled: false };
  };

  const waitForConversionBatch = async (previews: AppPreview[]) => {
    const slots = previews.map((preview) => preview.slot_index);
    for (let attempt = 0; attempt < 600; attempt += 1) {
      if (analysisCancelRequested) {
        return false;
      }
      const desktopState = await services.loadDesktopState();
      if (slots.every((slotIndex) => desktopState.slots[slotIndex]?.status !== 'running')) {
        applyDesktopState(desktopState);
        return true;
      }
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    throw new Error('转换仍在进行，增强分析未能及时开始');
  };

  const runPostConversionAnalysis = async (batchId: string, previews: AppPreview[]) => {
    try {
      const conversionReady = await waitForConversionBatch(previews);
      if (!conversionReady) {
        analysisState = {
          ...analysisState,
          status: 'cancelled',
          message: t('analysisCancelled', state.lang),
        };
        render();
        return;
      }
      const analysis = await analyzePreviewCandidates(previews);
      if (analysis.analyses.length === 0 && analysis.failures.length === 0) {
        return;
      }
      const nextState = await services.applyTrackAnalysisResults(
        batchId,
        previews,
        analysis.analyses,
        analysis.failures,
      );
      applyDesktopState(nextState);
    } catch (error) {
      analysisState = {
        ...analysisState,
        status: 'error',
        message: error instanceof Error ? error.message : String(error),
      };
      console.warn('Enhanced analysis did not complete:', error);
      render();
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
    pendingAction: PendingGlobalAction = null,
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

  root.addEventListener('dragstart', (event) => {
    const header = (event.target as HTMLElement | null)?.closest<HTMLElement>('[data-library-column-header]');
    if (!header || !libraryState?.visible) return;
    draggedLibraryColumn = header.dataset.libraryColumnHeader || null;
    if (draggedLibraryColumn && event.dataTransfer) {
      event.dataTransfer.effectAllowed = 'move';
      event.dataTransfer.setData('text/plain', draggedLibraryColumn);
    }
  });

  root.addEventListener('dragover', (event) => {
    const header = (event.target as HTMLElement | null)?.closest<HTMLElement>('[data-library-column-header]');
    if (header && draggedLibraryColumn) event.preventDefault();
  });

  root.addEventListener('drop', (event) => {
    const header = (event.target as HTMLElement | null)?.closest<HTMLElement>('[data-library-column-header]');
    const target = header?.dataset.libraryColumnHeader;
    if (!target || !draggedLibraryColumn || target === draggedLibraryColumn) {
      draggedLibraryColumn = null;
      return;
    }
    event.preventDefault();
    const order = libraryColumnIds();
    const from = order.indexOf(draggedLibraryColumn);
    const to = order.indexOf(target);
    if (from >= 0 && to >= 0) {
      order.splice(from, 1);
      order.splice(order.indexOf(target), 0, draggedLibraryColumn);
      saveLibraryColumnOrder(order);
      render();
    }
    draggedLibraryColumn = null;
  });

  root.addEventListener('click', (event) => {
    const target = event.target as HTMLElement | null;
    const modal = target?.closest('.about-modal');
    const libraryModal = target?.closest('.library-modal');
    const dialog = target?.closest('.about-dialog, .help-dialog, .library-dialog');
    if (libraryModal && !dialog) {
      libraryState = null;
      render();
      return;
    }
    if (modal && !dialog) {
      if (modal.classList.contains('help-modal')) {
        helpVisible = false;
      } else {
        aboutInfo = null;
      }
      render();
      return;
    }

    const libraryRow = target?.closest<HTMLElement>('[data-action="library-track-detail"]');
    if (libraryRow && libraryState?.visible) {
      const trackKey = libraryRow.dataset.trackKey;
      if (trackKey) void openLibraryDetail(trackKey);
      return;
    }

    const button = target?.closest<HTMLButtonElement>('button');
    if (!button) {
      return;
    }

    const action = button.dataset.action;
    const mode = button.dataset.mode as AppMode | undefined;
    const format = button.dataset.format as AppLosslessFormat | undefined;
    const conversionMode = button.dataset.conversionMode as AppConversionMode | undefined;
    const enhancedMode = button.dataset.enhancedMode;
    const slotIndex = parseSlotIndex(button.dataset.slot);
    const isOnboardingAction = action === 'onboarding-next'
      || action === 'onboarding-previous'
      || action === 'dismiss-onboarding';

    if (onboardingVisible && !isOnboardingAction) {
      return;
    }

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

    if (action === 'open-help') {
      helpVisible = true;
      render();
      return;
    }

    if (action === 'open-library') {
      void openLibrary();
      return;
    }

    if (action === 'close-library') {
      libraryState = null;
      render();
      return;
    }

    if (action === 'refresh-library') {
      void refreshLibrary();
      return;
    }

    if (action === 'clear-library-cache') {
      void clearLibraryCache();
      return;
    }

    if (action === 'close-library-detail') {
      if (libraryState) {
        libraryState = { ...libraryState, detail: null };
        render();
      }
      return;
    }

    if (action === 'library-track-detail') {
      const trackKey = button.dataset.trackKey;
      if (trackKey) void openLibraryDetail(trackKey);
      return;
    }

    if (action === 'library-sort') {
      if (!libraryState) return;
      const field = button.dataset.libraryField as LibraryField | undefined;
      if (!field) return;
      const current = libraryState.query.sorts.find((sort) => sort.field === field);
      const sorts = current
        ? current.direction === 'asc'
          ? libraryState.query.sorts.map((sort) => sort.field === field ? { ...sort, direction: 'desc' as const } : sort)
          : libraryState.query.sorts.filter((sort) => sort.field !== field)
        : [{ field, direction: 'asc' as const }];
      void queryLibrary({ ...libraryState.query, sorts, offset: 0 });
      return;
    }

    if (action === 'library-toggle-column') {
      const columnId = button.dataset.libraryColumn;
      if (columnId) {
        toggleLibraryColumn(columnId);
        render();
      }
      return;
    }

    if (action === 'library-apply-filter') {
      if (!libraryState) return;
      const fieldSelect = root.querySelector<HTMLSelectElement>('select[data-action="library-filter-field"]');
      const operatorSelect = root.querySelector<HTMLSelectElement>('select[data-action="library-filter-operator"]');
      const valueInput = root.querySelector<HTMLInputElement>('input[data-action="library-filter-value"]');
      if (!fieldSelect || !operatorSelect || !valueInput) return;
      const operator = operatorSelect.value as LibraryOperator;
      const filter: LibraryFilter = {
        field: fieldSelect.value as LibraryField,
        operator,
        value: ['is_empty', 'is_not_empty', 'is_true', 'is_false'].includes(operator)
          ? null
          : valueInput.value,
      };
      if (!filter.value && !['is_empty', 'is_not_empty', 'is_true', 'is_false'].includes(operator)) return;
      void queryLibrary({ ...libraryState.query, filters: [...libraryState.query.filters, filter], offset: 0 });
      return;
    }

    if (action === 'library-clear-filters') {
      if (libraryState) void queryLibrary({ ...libraryState.query, filters: [], offset: 0 });
      return;
    }

    if (action === 'library-lyrics-tab') {
      const tab = button.dataset.lyricsTab as LibraryLyricsTab | undefined;
      if (libraryState && tab) {
        libraryState = { ...libraryState, lyricsTab: tab, lyricsSearch: '' };
        render();
      }
      return;
    }

    if (action === 'library-copy-lyrics') {
      void copyLibraryLyrics();
      return;
    }

    if (action === 'library-download-lyrics') {
      downloadLibraryLyrics();
      return;
    }

    if (action === 'library-prev' || action === 'library-next') {
      if (libraryState) {
        const delta = action === 'library-prev' ? -libraryState.query.limit : libraryState.query.limit;
        const query = { ...libraryState.query, offset: Math.max(0, libraryState.query.offset + delta) };
        void queryLibrary(query);
      }
      return;
    }

    if (action === 'dismiss-onboarding') {
      onboardingVisible = false;
      onboardingStep = 0;
      localStorage.setItem('w4dj_onboarding_seen', '1');
      render();
      void discoverNeteaseAfterOnboarding();
      return;
    }

    if (action === 'onboarding-next') {
      if (onboardingStep === ONBOARDING_STEP_COUNT - 1) {
        onboardingVisible = false;
        onboardingStep = 0;
        localStorage.setItem('w4dj_onboarding_seen', '1');
      } else {
        onboardingStep = (onboardingStep + 1) as OnboardingStep;
      }
      render();
      if (!onboardingVisible) {
        void discoverNeteaseAfterOnboarding();
      }
      return;
    }

    if (action === 'onboarding-previous') {
      if (onboardingStep > 0) {
        onboardingStep = (onboardingStep - 1) as OnboardingStep;
        render();
      }
      return;
    }

    if (action === 'reopen-onboarding') {
      aboutInfo = null;
      helpVisible = false;
      onboardingVisible = true;
      onboardingStep = 0;
      render();
      return;
    }

    if (action === 'close-about') {
      aboutInfo = null;
      updateInfo = null;
      render();
      return;
    }

    if (action === 'close-help') {
      helpVisible = false;
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

    if (action === 'check-updates') {
      void services.checkForUpdates()
        .then((result) => {
          updateInfo = result;
          render();
        })
        .catch(reportError);
      return;
    }

    if (action === 'open-release-page') {
      const url = button.dataset.url;
      if (url) {
        void services.openExternalUrl(url);
      }
      return;
    }

    if (action === 'cancel-preview' || action === 'edit-preview') {
      if (!previewBusy) {
        previewModal = null;
        render();
      }
      return;
    }

    if (action === 'cancel-scan') {
      void cancelScanFlow();
      return;
    }

    if (action === 'close-scan') {
      if (scanProgress?.status !== 'running') {
        scanProgress = null;
        pendingGlobalAction = null;
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

    if (action === 'clear-analysis-cache') {
      void clearEnhancedCache();
      return;
    }

    if (action === 'clear-scan-cache') {
      void clearScanCache();
      return;
    }

    if (action === 'download-essentia-models') {
      void downloadEssentiaModels();
      return;
    }

    if (action === 'cancel-slot' && slotIndex !== null) {
      void runAction(() => services.cancelSync(slotIndex), slotIndex);
      return;
    }

    if (action === 'cancel-all') {
      void runAction(() => services.cancelAllSync(), 'all', 'cancel-all');
      return;
    }

    if (action === 'cancel-analysis') {
      cancelAnalysisFlow();
      return;
    }

    if (action === 'pick-source' && slotIndex !== null) {
      void runAction(async () => {
        const path = await services.pickSource(slotIndex);
        return path ? services.selectSourceDirectory(slotIndex, path) : undefined;
      }, slotIndex);
      return;
    }

    if (action === 'clear-source' && slotIndex !== null) {
      void runAction(() => services.selectSourceDirectory(slotIndex, ''), slotIndex);
      return;
    }

    if (action === 'open-source' && slotIndex !== null) {
      const source = state.slots[slotIndex].sourceDirectory.trim();
      if (source) {
        void runAction(() => services.openSource(source), slotIndex);
      }
      return;
    }

    if (action === 'pick-destination' && slotIndex !== null) {
      void runAction(async () => {
        const path = await services.pickDirectory('destination', slotIndex);
        return path ? services.selectDestinationDirectory(slotIndex, path) : undefined;
      }, slotIndex);
      return;
    }

    if (action === 'open-destination' && slotIndex !== null) {
      const slot = state.slots[slotIndex];
      const destination = slot.destinationDirectory.trim()
        || (slotIndex === 1 ? state.slots[0].destinationDirectory.trim() : '');
      if (destination) {
        void runAction(() => services.openDestination(destination), slotIndex);
      }
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

    if (conversionMode) {
      void runSelectionAction(
        'conversion-mode',
        state.conversionMode !== conversionMode,
        () => services.chooseConversionMode(conversionMode),
      );
      return;
    }

    if (enhancedMode === 'on' || enhancedMode === 'off') {
      const enabled = enhancedMode === 'on';
      void runSelectionAction(
        'enhanced-mode',
        state.enhancedMode !== enabled,
        () => services.chooseEnhancedMode(enabled),
      );
      return;
    }

    if (action === 'start-all') {
      void beginScan();
      return;
    }

    if (action === 'pause-all') {
      void runAction(() => services.pauseAllSync(), 'all', 'pause-all');
    }
  });

  root.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && libraryState?.visible) {
      event.preventDefault();
      libraryState = null;
      render();
      return;
    }
    if (event.key === 'Escape' && helpVisible) {
      event.preventDefault();
      helpVisible = false;
      render();
      return;
    }

    if (!onboardingVisible) {
      return;
    }

    if (event.key === 'Escape') {
      event.preventDefault();
      onboardingVisible = false;
      onboardingStep = 0;
      localStorage.setItem('w4dj_onboarding_seen', '1');
      render();
      void discoverNeteaseAfterOnboarding();
      return;
    }

    if (event.key === 'ArrowLeft' && onboardingStep > 0) {
      event.preventDefault();
      onboardingStep = (onboardingStep - 1) as OnboardingStep;
      render();
      return;
    }

    if (event.key === 'ArrowRight') {
      event.preventDefault();
      if (onboardingStep === ONBOARDING_STEP_COUNT - 1) {
        onboardingVisible = false;
        onboardingStep = 0;
        localStorage.setItem('w4dj_onboarding_seen', '1');
        void discoverNeteaseAfterOnboarding();
      } else {
        onboardingStep = (onboardingStep + 1) as OnboardingStep;
      }
      render();
    }
  });

  root.addEventListener('toggle', (event) => {
    const settings = event.target;
    if (
      settings instanceof HTMLDetailsElement
      && settings.dataset.role === 'advanced-output-settings'
    ) {
      outputSettingsExpanded = settings.open;
    }
  }, true);

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
      return;
    }

    if (select.dataset.action === 'choose-netease-filename-format') {
      const format = select.value as AppNeteaseFilenameFormat;
      if (format !== state.neteaseFilenameFormat) {
        void runAction(() => services.chooseNeteaseFilenameFormat(format), 'all');
      }
    }
  });

  root.addEventListener('input', (event) => {
    const target = event.target as HTMLElement | null;
    const input = target?.closest<HTMLInputElement>('input');
    if (!input || !libraryState) return;
    if (input.dataset.action === 'library-search') {
      const query = { ...libraryState.query, text: input.value, offset: 0 };
      if (librarySearchTimer) clearTimeout(librarySearchTimer);
      librarySearchTimer = setTimeout(() => {
        librarySearchTimer = null;
        void queryLibrary(query);
      }, 180);
    } else if (input.dataset.action === 'library-lyrics-search') {
      libraryState = { ...libraryState, lyricsSearch: input.value };
      render();
    }
  });

  const clearDropTargets = () => {
    root.querySelectorAll<HTMLElement>('[data-drop-kind].is-drag-over').forEach((target) => {
      target.classList.remove('is-drag-over');
    });
  };

  const dropTargetAt = (position: { x: number; y: number }, scaleFactor: number) => {
    const targets = Array.from(root.querySelectorAll<HTMLElement>('[data-drop-kind]')).map(
      (target) => {
        const rect = target.getBoundingClientRect();
        return {
          value: target,
          rect: {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
          },
        };
      },
    );

    return resolveDropTargetAt(
      targets,
      position,
      scaleFactor,
      nativeDropCoordinatesArePhysical() ? 'physical' : 'logical',
    );
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

  const browserDropTargetAt = (event: DragEvent): HTMLElement | null => {
    // WKWebView may keep dispatching drag events from the element first entered.
    // Hit-test the live pointer coordinates so moving in either direction can
    // switch between all four source/destination fields.
    const hasPointerPosition = Number.isFinite(event.clientX) && Number.isFinite(event.clientY);
    if (hasPointerPosition) {
      return dropTargetAt({ x: event.clientX, y: event.clientY }, 1);
    }
    return (event.target as HTMLElement | null)?.closest<HTMLElement>('[data-drop-kind]')
      ?? null;
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
    const target = browserDropTargetAt(event);
    clearDropTargets();
    if (!target) {
      return;
    }

    event.preventDefault();
    target.classList.add('is-drag-over');
  });

  root.addEventListener('drop', (event) => {
    const target = browserDropTargetAt(event);
    clearDropTargets();
    if (!target) {
      return;
    }

    event.preventDefault();
    handleDirectoryDrop(target, pathFromBrowserDrop(event));
  });

  try {
    const currentWindow = getCurrentWindow();
    const scaleFactorPromise = currentWindow
      .scaleFactor()
      .catch(() => window.devicePixelRatio || 1);
    const listener = currentWindow.onDragDropEvent(async ({ payload }: { payload: DragDropEvent }) => {
      if (payload.type === 'leave') {
        clearDropTargets();
        return;
      }

      const target = dropTargetAt(payload.position, await scaleFactorPromise);
      clearDropTargets();
      target?.classList.add('is-drag-over');

      if (payload.type !== 'drop' || !target || payload.paths.length === 0) {
        return;
      }

      handleDirectoryDrop(target, payload.paths[0]);
    });
    void listener.catch((error) => console.error('Failed to register path drag-and-drop:', error));
  } catch {
    // Tauri drag-and-drop is unavailable in the browser test environment.
  }

  render();
  analysisCacheLoadPromise = loadAnalysisCache();
  void runAction(() => services.loadDesktopState());
  void refreshHistory();
  if (!onboardingVisible) {
    void discoverNeteaseAfterOnboarding();
  }
  if (services.getEssentiaModelStatus) {
    void services.getEssentiaModelStatus()
      .then((status) => {
        modelStatus = status;
        render();
      })
      .catch((error) => console.warn('Failed to load Essentia model status:', error));
  }
}

function renderLosslessFormats(state: AppViewState, pendingSelection: PendingSelection = null): string {
  const formats: AppLosslessFormat[] = ['wav', 'aiff'];
  return `
    <div class="format-slot">
      <div class="format-row" data-selected-format="${state.losslessFormat || 'wav'}" data-visible="${state.mode === 'lossless'}" aria-label="${t('losslessFormat', state.lang)}" aria-hidden="${state.mode !== 'lossless'}">
        ${formats
          .map(
            (format) => `
              <button type="button" class="format-button ${state.losslessFormat === format ? 'selected' : ''}" data-format="${format}" aria-disabled="${pendingSelection === 'format' ? 'true' : 'false'}">
                ${format.toUpperCase()}
              </button>
            `,
          )
          .join('')}
      </div>
    </div>
  `;
}

function renderOutputSettings(
  state: AppViewState,
  expanded = false,
  modelStatus: EssentiaModelStatus = defaultEssentiaModelStatus,
): string {
  const modelsReady = modelStatus.embedding && modelStatus.genre && modelStatus.mood && modelStatus.instrument;
  return `
    <details class="output-settings" data-role="advanced-output-settings" aria-label="${t('advancedOptions', state.lang)}" ${expanded ? 'open' : ''}>
      <summary>${t('advancedOptions', state.lang)}</summary>
      <div class="output-settings-content">
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
        <label>
          <span>${t('neteaseFilenameFormat', state.lang)}</span>
          <select data-action="choose-netease-filename-format" aria-label="${t('neteaseFilenameFormat', state.lang)}">
            <option value="title_only" ${state.neteaseFilenameFormat === 'title_only' ? 'selected' : ''}>${t('neteaseTitleOnly', state.lang)}</option>
            <option value="artist_title" ${state.neteaseFilenameFormat === 'artist_title' ? 'selected' : ''}>${t('neteaseArtistTitle', state.lang)}</option>
            <option value="title_artist" ${state.neteaseFilenameFormat === 'title_artist' ? 'selected' : ''}>${t('neteaseTitleArtist', state.lang)}</option>
          </select>
        </label>
        <div class="essentia-model-settings">
          <span class="essentia-model-title">${t('essentiaModelsTitle', state.lang)}</span>
          <small>${modelsReady ? t('essentiaModelsReady', state.lang) : t('essentiaModelsMissing', state.lang)}</small>
          <div class="essentia-model-actions">
            <button type="button" class="secondary-action essentia-model-download" data-action="download-essentia-models" ${modelStatus.downloading ? 'disabled' : ''}>
              ${modelStatus.downloading ? t('essentiaModelsDownloading', state.lang) : t('essentiaModelsDownload', state.lang)}
            </button>
            <button type="button" class="secondary-action enhanced-cache-clear" data-action="clear-analysis-cache">
              ${t('clearEnhancedCache', state.lang)}
            </button>
          </div>
        </div>
        <button type="button" class="secondary-action scan-cache-clear" data-action="clear-scan-cache">
          ${t('clearScanCache', state.lang)}
        </button>
      </div>
    </details>
  `;
}

function renderAboutModal(info: AppInfo | null, update: AppUpdateCheck | null, lang: AppLanguage): string {
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
          <button type="button" class="about-link" data-action="check-updates">${t('checkUpdates', lang)}</button>
        </div>
        ${update ? `<p class="about-update">${update.update_available ? t('updateAvailable', lang).replace('{version}', escapeHtml(update.latest_version)) : t('alreadyLatest', lang)}${update.update_available ? ` <button type="button" class="about-link" data-action="open-release-page" data-url="${escapeHtml(update.release_url)}">${t('viewRelease', lang)}</button>` : ''}</p>` : ''}
        <button type="button" class="global-action" data-action="close-about">${t('close', lang)}</button>
      </section>
    </div>
  `;
}

function renderHelpModal(visible: boolean, lang: AppLanguage): string {
  if (!visible) {
    return '';
  }

  return `
    <div class="about-modal help-modal" data-role="help-modal" role="dialog" aria-modal="true" aria-labelledby="help-title" aria-describedby="help-intro">
      <section class="help-dialog">
        <header class="help-dialog-head">
          <div>
            <p class="panel-kicker">W4DJ RKB</p>
            <h2 id="help-title">${t('helpTitle', lang)}</h2>
          </div>
        </header>
        <p id="help-intro" class="help-dialog-intro">${t('helpIntro', lang)}</p>

        <section class="help-section" aria-labelledby="help-conversion-title">
          <h3 id="help-conversion-title">${t('helpConversionTitle', lang)}</h3>
          <div class="help-card-grid">
            <article class="help-card">
              <h4>${t('scanThenConvert', lang)}</h4>
              <p>${t('helpScanThenConvertBody', lang)}</p>
            </article>
            <article class="help-card">
              <h4>${t('directConvert', lang)}</h4>
              <p>${t('helpDirectConvertBody', lang)}</p>
            </article>
          </div>
        </section>

        <section class="help-section" aria-labelledby="help-output-title">
          <h3 id="help-output-title">${t('helpOutputTitle', lang)}</h3>
          <div class="help-card-grid">
            <article class="help-card">
              <h4>${t('compatMode', lang)}</h4>
              <p class="help-note">${t('compatNote', lang)}</p>
            </article>
            <article class="help-card">
              <h4>${t('losslessMode', lang)}</h4>
              <p class="help-note">${t('losslessNote', lang)}</p>
            </article>
            <article class="help-card">
              <h4>${t('helpCompatibilityTitle', lang)}</h4>
              <p>${t('helpCompatibilityBody', lang)}</p>
            </article>
            <article class="help-card">
              <h4>${t('helpEnhancedTitle', lang)}</h4>
              <p>${t('helpEnhancedBody', lang)}</p>
            </article>
          </div>
        </section>

        <div class="help-actions">
          <button type="button" class="about-link" data-action="reopen-onboarding">${t('usageGuide', lang)}</button>
          <button type="button" class="global-action" data-action="close-help">${t('close', lang)}</button>
        </div>
      </section>
    </div>
  `;
}

function renderOnboardingModal(visible: boolean, lang: AppLanguage, step: OnboardingStep = 0): string {
  if (!visible) {
    return '';
  }

  const steps = [
    { target: 'mode', title: t('onboardingStepOneTitle', lang), body: t('onboardingStepOneBody', lang) },
    { target: 'source', title: t('onboardingStepTwoTitle', lang), body: t('onboardingStepTwoBody', lang) },
    { target: 'destination', title: t('onboardingStepThreeTitle', lang), body: t('onboardingStepThreeBody', lang) },
    { target: 'start', title: t('onboardingStepFourTitle', lang), body: t('onboardingStepFourBody', lang) },
    { target: 'tutorial', title: t('onboardingStepFiveTitle', lang), body: t('onboardingStepFiveBody', lang) },
  ] as const;
  const currentStep = steps[step];
  const isLastStep = step === ONBOARDING_STEP_COUNT - 1;

  return `
    <div class="onboarding-modal" data-role="onboarding-modal" data-step="${step}" role="dialog" aria-modal="true" aria-labelledby="onboarding-title" aria-describedby="onboarding-body">
      <section class="onboarding-callout" data-role="onboarding-step">
        <div class="onboarding-callout-head">
          <div>
            <p class="panel-kicker">W4DJ RKB</p>
            <h2 id="onboarding-title">${currentStep.title}</h2>
          </div>
          <span class="onboarding-counter">${step + 1}/${ONBOARDING_STEP_COUNT}</span>
        </div>
        <p id="onboarding-body" class="onboarding-intro">${currentStep.body}</p>
        <div class="onboarding-progress" aria-hidden="true"><span style="width: ${((step + 1) / ONBOARDING_STEP_COUNT) * 100}%"></span></div>
        <footer class="onboarding-actions">
          <button type="button" class="onboarding-skip" data-action="dismiss-onboarding">${t('onboardingSkip', lang)}</button>
          <div>
            <button type="button" class="secondary-action" data-action="onboarding-previous" ${step === 0 ? 'disabled' : ''}>${t('onboardingPrevious', lang)}</button>
            <button type="button" class="global-action" data-action="onboarding-next">${isLastStep ? t('onboardingFinish', lang) : t('onboardingNext', lang)}</button>
          </div>
        </footer>
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
      logs: slot.logs,
    })) as [AppSyncSlotViewState, AppSyncSlotViewState],
    mode: state.mode,
    losslessFormat: state.lossless_format,
    conversionMode: state.conversion_mode,
    enhancedMode: state.enhanced_mode,
    conflictStrategy: state.conflict_strategy,
    filenameRule: state.filename_rule,
    neteaseFilenameFormat: state.netease_filename_format,
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

function scanPhaseLabel(phase: AppScanPhase, lang: AppLanguage): string {
  const keys: Record<AppScanPhase, keyof typeof translations.zh> = {
    preparing: 'scanPreparing',
    scanning_source: 'scanSource',
    scanning_destination: 'scanDestination',
    checking: 'scanChecking',
    analyzing: 'scanAnalyzing',
    completed: 'scanCompleted',
    cancelled: 'scanCancelled',
    error: 'scanError',
  };
  return t(keys[phase], lang);
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

function displayPath(path: string, lang: AppLanguage, emptyLabel = t('pickFolder', lang)): string {
  return escapeHtml(path || emptyLabel);
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

function icon(name: 'folder' | 'music' | 'export' | 'open' | 'trash' | 'check' | 'convert' | 'disc' | 'play' | 'pause' | 'list' | 'sun' | 'moon' | 'arrow' | 'help'): string {
  const icons = {
    folder: '<path d="M2.5 5.1h3.4l1.1 1.2h6.5v5.2H2.5z"/><path d="M2.5 4.5h3.2l1.3 1.2"/>',
    music: '<path d="M6.2 11.2V4.6l6-1.2v6.4"/><path d="M6.2 6.5l6-1.2"/><circle cx="4.5" cy="11.5" r="1.7"/><circle cx="10.5" cy="10.1" r="1.7"/>',
    export: '<path d="M3 12.2h10"/><path d="M8 4v6.1"/><path d="M5.6 6.4 8 4l2.4 2.4"/>',
    open: '<path d="M9.2 3H13v3.8"/><path d="m13 3-6 6"/><path d="M11 8.5v4H3V4.2h4"/>',
    trash: '<path d="M3.8 5.2h8.4"/><path d="M6.2 5.2V3.8h3.6v1.4"/><path d="m5 5.2.5 7.2h5l.5-7.2"/><path d="M6.8 7.1v3.7M9.2 7.1v3.7"/>',
    check: '<path d="M3.3 8.5 6.4 11.4 12.8 4.7"/>',
    convert: '<path d="M2.5 5.1h8.2"/><path d="m8.2 2.6 2.6 2.5-2.6 2.5"/><path d="M13.5 10.9H5.3"/><path d="m7.8 8.4-2.6 2.5 2.6 2.5"/>',
    disc: '<circle cx="8" cy="8" r="5.1"/><circle cx="8" cy="8" r="1"/>',
    play: '<path d="M5.2 4v8l6.6-4z"/>',
    pause: '<path d="M5.1 4.2v7.6"/><path d="M10.9 4.2v7.6"/>',
    list: '<path d="M5 4.7h8"/><path d="M5 8h8"/><path d="M5 11.3h8"/><path d="M2.7 4.7h.5"/><path d="M2.7 8h.5"/><path d="M2.7 11.3h.5"/>',
    sun: '<circle cx="8" cy="8" r="2.8"/><path d="M8 1.8v1.3M8 12.9v1.3M1.8 8h1.3M12.9 8h1.3M3.6 3.6l.9.9M11.5 11.5l.9.9M12.4 3.6l-.9.9M4.5 11.5l-.9.9"/>',
    moon: '<path d="M12.7 10.4A5.3 5.3 0 0 1 5.6 3.3a5.3 5.3 0 1 0 7.1 7.1z"/>',
    arrow: '<path d="M2.5 8h10.2"/><path d="m9.4 4.8 3.3 3.2-3.3 3.2"/>',
    help: '<circle cx="8" cy="8" r="5.5"/><path d="M6.4 6.3a1.8 1.8 0 1 1 3.1 1.3c-.8.6-1.4 1-1.4 2"/><path d="M8 11.9h.01"/>',
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
