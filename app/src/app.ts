import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { message, open, save } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow, type DragDropEvent } from '@tauri-apps/api/window';
import {
  analyzeAudioFile,
  assessTrackAnalysisCompleteness,
  ESSENTIA_MODEL_IDS,
  isBasicTrackAnalysisComplete,
  isCompleteTrackAnalysis,
  normalizeEssentiaModel,
  TRACK_ANALYSIS_VERSION,
  type EssentiaModelFile,
  type EssentiaModelWire,
  type TrackAnalysis,
  type TrackMetadata,
} from './analysis';
import {
  AnalysisWorkerCancelledError,
  AnalysisWorkerClient,
  AnalysisWorkerTimeoutError,
  type AnalysisWorkerSession,
} from './analysis-worker-client';
import {
  renderLibraryDashboard,
  libraryColumnIds,
  libraryOperatorsForField,
  saveLibraryColumnOrder,
  saveLibraryColumnWidth,
  toggleLibraryColumn,
  type LibraryDashboardState,
  type LibraryField,
  type LibraryFilter,
  type LibraryLyricsTab,
  type LibraryOperator,
  type LibraryPage,
  type LibraryQuery,
  type LibraryRefreshProgress,
  type LibraryInvalidScanProgress,
  type LibrarySourceRecord,
  type LibraryStatus,
  type LibraryTrack,
  isLibraryRefreshActive,
} from './library-dashboard';
import {
  buildNeteaseImportText,
  splitNeteaseQrPages,
  type ImportedDjPlaylist,
  type ImportedDjPlaylistSummary,
  type NeteaseQrPage,
} from './dj-playlist';
import { renderPlaintextQrDataUrl } from './qr-code';

export const DJ_PLAYLIST_QR_CONCURRENCY = 3;

export async function renderDjPlaylistQrPages(
  pages: readonly NeteaseQrPage[],
  renderQr: (text: string) => Promise<string> = renderPlaintextQrDataUrl,
): Promise<string[]> {
  const results = new Array<string>(pages.length);
  let nextIndex = 0;
  const workerCount = Math.min(DJ_PLAYLIST_QR_CONCURRENCY, pages.length);
  const workers = Array.from({ length: workerCount }, async () => {
    while (nextIndex < pages.length) {
      const index = nextIndex;
      nextIndex += 1;
      results[index] = await renderQr(pages[index].text);
    }
  });
  await Promise.all(workers);
  return results;
}

export type NeteaseDiscoveryProgress = {
  status: 'running' | 'completed' | 'error';
  stage: 'locatingDatabase' | 'readingRecords' | 'checkingMusicFolder';
  processed: number;
  total: number | null;
  currentItem: string;
  message: string;
  suggestion: LibraryStatus['netease'] | null;
  error: string | null;
};

export type NeteaseMetadataDatabaseStatus = {
  manualPath: string | null;
  effectivePath: string | null;
  source: 'manual' | 'automatic' | 'unavailable';
  loaded: boolean;
  recordCount: number;
  warning: string | null;
  cacheStatus?: 'idle' | 'ready' | 'stale' | 'building' | 'cancelling' | 'cancelled' | 'error' | null;
  cachedRecordCount?: number;
  databaseChanged?: boolean;
};

export type NeteaseMetadataCacheProgress = {
  status: 'idle' | 'ready' | 'stale' | 'building' | 'cancelling' | 'cancelled' | 'error';
  stage: string;
  processed: number;
  total: number | null;
  currentItem: string;
  message: string;
  error: string | null;
  databasePath: string | null;
  cachedRecordCount: number;
};

export type NeteaseMetadataDatabaseUiState = {
  status: NeteaseMetadataDatabaseStatus | null;
  busy: boolean;
  message: string | null;
  error: string | null;
};

export type LibraryAnalysisCandidate = {
  path: string;
  name: string;
  sizeBytes: number;
  slotIndex?: SyncSlotIndex;
};

export type AppMode = 'compat' | 'lossless';
export type AppLosslessFormat = 'wav' | 'aiff';
export type AppConversionMode = 'scan_then_convert' | 'direct';
export type AppConflictStrategy = 'skip' | 'overwrite' | 'rename' | 'update_metadata';
export type AppFilenameRule = 'title_artist' | 'artist_title' | 'original';
export type AppNeteaseFilenameFormat = 'title_only' | 'artist_title' | 'title_artist';
export type AppStatus = 'idle' | 'running' | 'paused' | 'completed' | 'error' | 'cancelled';
export type AppScanStatus = 'idle' | 'running' | 'cancelling' | 'completed' | 'cancelled' | 'error';
export type AppScanPhase = 'preparing' | 'scanning_source' | 'scanning_destination' | 'matching_metadata' | 'checking' | 'analyzing' | 'completed' | 'cancelled' | 'error';
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

// Keep the enhanced-analysis state, model loading, and cache actions available
// while the unstable controls are temporarily hidden from the conversion rail.
// Flip this single flag back to true when the complete enhanced-analysis UI is
// ready to return; the Rust analysis backend remains available throughout.
const ENHANCED_ANALYSIS_FEATURES_VISIBLE = false;

// Cache cleanup is a safe maintenance action and remains available even while
// the enhanced-analysis controls are hidden from the conversion rail.
const ANALYSIS_CACHE_CLEAR_VISIBLE = true;

// Keep the song-library backend and state intact while its user-facing entry
// point remains hidden during the current stability/product rollout.
const SONG_LIBRARY_FEATURE_VISIBLE = false;

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
  activeConcurrencyLimit: number | null;
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
  concurrencyLimit: number;
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
  active_concurrency_limit: number | null;
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
  concurrency_limit: number;
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
  netease_track_id?: string | null;
  netease_album_id?: string | null;
  album?: string | null;
  disambiguation_reason?: string | null;
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
  input_count?: number;
  output_duplicate_count?: number;
  action_kind?: 'skip' | 'overwrite' | 'rename' | 'update_metadata' | string;
  action_count?: number;
  database_directory?: string | null;
  detail_items?: AppPreviewDetailItem[];
};

export type AppPreviewDetailItem = {
  name: string;
  source_path: string;
  destination_path?: string | null;
  existing_output?: boolean;
  classification: 'new' | 'duplicate' | 'skip' | 'overwrite' | 'update_metadata' | 'rename' | 'error' | string;
  reason?: string | null;
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
  slotIndex: SyncSlotIndex | null;
  status: 'idle' | 'running' | 'completed' | 'cancelled' | 'error';
  completed: number;
  total: number;
  resultCount: number;
  failedCount: number;
  message: string;
  currentItem?: string;
  stage?: string;
  stageProcessed?: number;
  stageTotal?: number;
  workerJobId?: string;
  startedAt?: string;
  resumeAvailable?: boolean;
};

export type AppAnalysisFailure = {
  path: string;
  message: string;
  status?: 'failed' | 'timeout';
  stage?: string;
  elapsedMs?: number;
};

export type DjPlaylistMatchCandidate = {
  trackKey: string;
  title: string;
  artistDisplay: string;
  durationSeconds: number | null;
  destinationFilename: string;
  score: number;
  reason: string;
};

export type DjPlaylistTrackMatch = {
  position: number;
  dedupeKey: string;
  title: string;
  artistDisplay: string;
  neteaseTrackId: string | null;
  kind: 'neteaseTrackId' | 'uniqueTitleArtistFallback' | 'ambiguous' | 'unmatched' | 'missing' | 'manual';
  status: 'matched' | 'unmatched' | 'ambiguous' | 'missing';
  trackKey: string | null;
  matchMethod: string | null;
  score: number | null;
  reason: string;
  candidates: DjPlaylistMatchCandidate[];
  manual: boolean;
};

export type DjPlaylistMatchReport = {
  playlistId: string;
  total: number;
  matchedCount: number;
  ambiguousCount: number;
  unmatchedCount: number;
  missingCount: number;
  matches: DjPlaylistTrackMatch[];
};

export type DjPlaylistM3u8ExportResult = {
  path: string;
  exportDirectory: string;
  matchedCount: number;
  total: number;
  copiedCount: number;
  copyAudio: boolean;
  portable: boolean;
  omitted: Array<{ position: number; reason: string }>;
};

export type DjPlaylistUiState = {
  visible: boolean;
  launcher?: boolean;
  exportPicker?: boolean;
  exportChoice?: boolean;
  recentPlaylists?: ImportedDjPlaylistSummary[];
  busy: boolean;
  error: string | null;
  notice: string | null;
  playlist: ImportedDjPlaylist | null;
  pages: NeteaseQrPage[];
  pageIndex: number;
  qrDataUrl: string | null;
  qrDataUrls?: Array<string | null>;
  qrRevision: number;
  matchBusy: boolean;
  matchReport: DjPlaylistMatchReport | null;
  exportBusy: boolean;
  dropActive: boolean;
};

type ResumableAnalysis = {
  batchId: string;
  previews: AppPreview[];
  analysis?: AppAnalysisSummary | null;
  attemptId?: string;
};

type PersistAnalysisCandidate = (
  candidate: AppPreviewCandidate,
  analysis: TrackAnalysis | null,
  failure: AppAnalysisFailure | null,
) => Promise<void>;

const RESUMABLE_ANALYSIS_STORAGE_KEY = 'w4dj.resumable-analysis.v1';

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
  enhancedMode = highLevelModelsAvailable,
): cached is TrackAnalysis {
  const basicMatch = cached?.analysisVersion === TRACK_ANALYSIS_VERSION
    && cached.sourceSizeBytes === fingerprint.sizeBytes
    && (cached.sourceModifiedAt ?? null) === fingerprint.modifiedAt
    && (cached.sourceFilenameFormat ?? 'title_artist') === neteaseFilenameFormat;
  if (!basicMatch) {
    return false;
  }
  if (!enhancedMode) {
    return isBasicTrackAnalysisComplete(cached);
  }
  if (!highLevelModelsAvailable) return false;
  return isCompleteTrackAnalysis(cached)
    && cached.highLevel?.modelVersion === highLevelModelVersion;
}

export type AppHistoryStatus = 'completed' | 'partial' | 'cancelled' | 'error';

export type AppAnalysisSummary = {
  status: 'notRequested' | 'pending' | 'running' | 'completed' | 'partial' | 'cancelled' | 'interrupted' | 'error';
  total: number;
  completed: number;
  failed: number;
  timedOut: number;
  pending: number;
  currentItem?: string | null;
  currentStage?: string | null;
  workerJobId?: string | null;
  requestedAt?: string | null;
  startedAt?: string | null;
  finishedAt?: string | null;
  terminationReason?: string | null;
};

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
  analysis?: AppAnalysisSummary | null;
};

export type AppPreviewModalState = {
  previews: AppPreview[];
  retryOf: string | null;
  detail?: { slotIndex: SyncSlotIndex; kind: PreviewDetailKind } | null;
};

export type PreviewDetailKind = 'input' | 'duplicates' | 'action' | 'errors';

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
  source_processed?: number;
  source_total?: number | null;
  destination_processed?: number;
  destination_total?: number | null;
  metadata_processed?: number;
  metadata_total?: number | null;
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
  chooseConcurrencyLimit: (value: string) => Promise<DesktopState>;
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
  loadIncompleteAnalysisRun?: () => Promise<ResumableAnalysis | null>;
  claimAnalysisRun?: (batchId: string, attemptId: string) => Promise<boolean>;
  retryHistoryFailures: (id: string) => Promise<AppPreview>;
  exportHistoryErrorReport: (id: string, path: string) => Promise<void>;
  exportRuntimeSession: (id: string, path: string) => Promise<void>;
  saveFile?: (options: SaveFileOptions) => Promise<string | null>;
  recordRuntimeSessionEvent?: (
    batchId: string,
    event: string,
    details: Record<string, unknown>,
  ) => Promise<void>;
  finalizeAnalysisSession?: (batchId: string) => Promise<void>;
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
  ensureEssentiaModels?: () => Promise<EssentiaModelStatus>;
  loadEssentiaModel?: (id: string) => Promise<EssentiaModelFile>;
  createAnalysisWorker?: () => AnalysisWorkerSession;
  loadLibraryStatus?: () => Promise<LibraryStatus>;
  locateNeteaseLibrary?: (force?: boolean) => Promise<LibraryStatus['netease']>;
  refreshLibraryCatalog?: () => Promise<LibraryRefreshProgress>;
  cancelLibraryRefresh?: () => Promise<LibraryRefreshProgress>;
  loadNeteaseMetadataDatabaseStatus?: () => Promise<NeteaseMetadataDatabaseStatus>;
  prepareNeteaseMetadataCache?: () => Promise<NeteaseMetadataCacheProgress>;
  cancelNeteaseMetadataCache?: () => Promise<NeteaseMetadataCacheProgress>;
  loadNeteaseMetadataCacheStatus?: () => Promise<NeteaseMetadataCacheProgress>;
  selectNeteaseMetadataDatabase?: (path: string) => Promise<NeteaseMetadataDatabaseStatus>;
  clearNeteaseMetadataDatabase?: () => Promise<NeteaseMetadataDatabaseStatus>;
  selectNeteaseDatabaseFallback?: (path: string) => Promise<LibraryStatus>;
  clearNeteaseDatabaseFallback?: () => Promise<LibraryStatus>;
  pickNeteaseDatabase?: () => Promise<string | null>;
  listenLibraryRefreshProgress?: (handler: (progress: LibraryRefreshProgress) => void) => Promise<UnlistenFn>;
  listenNeteaseDiscoveryProgress?: (handler: (progress: NeteaseDiscoveryProgress) => void) => Promise<UnlistenFn>;
  listenNeteaseMetadataCacheProgress?: (handler: (progress: NeteaseMetadataCacheProgress) => void) => Promise<UnlistenFn>;
  queryLibraryCatalog?: (query: LibraryQuery) => Promise<LibraryPage>;
  getLibraryTrackDetail?: (trackKey: string) => Promise<LibraryTrack | null>;
  getLibraryTrackSourceRecords?: (trackKey: string) => Promise<LibrarySourceRecord[]>;
  getLibraryTrackCover?: (trackKey: string) => Promise<string | null>;
  listLibraryAnalysisCandidates?: () => Promise<LibraryAnalysisCandidate[]>;
  clearLibraryCatalogCache?: () => Promise<void>;
  pickLibraryTrackFile?: () => Promise<string | null>;
  relocateLibraryTrack?: (trackKey: string, path: string) => Promise<void>;
  removeLibraryTrack?: (trackKey: string) => Promise<boolean>;
  clearInvalidLibraryTracks?: () => Promise<number>;
  findInvalidLibraryTracks?: () => Promise<LibraryInvalidScanProgress>;
  cancelInvalidLibraryScan?: () => Promise<LibraryInvalidScanProgress>;
  listenInvalidLibraryScanProgress?: (handler: (progress: LibraryInvalidScanProgress) => void) => Promise<UnlistenFn>;
  pickW4djPlaylist?: () => Promise<string | null>;
  importW4djPlaylist?: (path: string) => Promise<ImportedDjPlaylist>;
  listImportedDjPlaylists?: () => Promise<ImportedDjPlaylistSummary[]>;
  loadImportedDjPlaylist?: (playlistId: string) => Promise<ImportedDjPlaylist>;
  exportImportedDjPlaylistW4dj?: (playlistId: string, path: string) => Promise<void>;
  exportNeteasePlaylistText?: (path: string, text: string) => Promise<void>;
  matchImportedDjPlaylist?: (playlistId: string) => Promise<DjPlaylistMatchReport>;
  loadImportedDjPlaylistMatches?: (playlistId: string) => Promise<DjPlaylistMatchReport>;
  setImportedDjPlaylistMatch?: (playlistId: string, position: number, trackKey: string) => Promise<DjPlaylistMatchReport>;
  clearImportedDjPlaylistMatch?: (playlistId: string, position: number) => Promise<DjPlaylistMatchReport>;
  exportImportedDjPlaylistM3u8?: (playlistId: string, path: string, allowPartial: boolean, copyAudio?: boolean) => Promise<DjPlaylistM3u8ExportResult>;
};

export type EssentiaModelStatus = {
  version: string;
  embedding: boolean;
  genre: boolean;
  mood: boolean;
  instrument: boolean;
  installing: boolean;
  emotionContinuous?: boolean;
  emotionCluster?: boolean;
  discogsEffnet?: {
    embedding: boolean;
    moodTheme: boolean;
    approachability: boolean;
    instrumentation: boolean;
    timbre: boolean;
    danceability: boolean;
  };
};

const MODEL_FILE_EXTENSIONS = new Set(['zip', 'json', 'bin']);

function containsModelFile(paths: string[]): boolean {
  return paths.some((path) => {
    const name = path.replaceAll('\\', '/').split('/').pop() ?? '';
    const extension = name.includes('.') ? name.split('.').pop()?.toLowerCase() ?? '' : '';
    return MODEL_FILE_EXTENSIONS.has(extension);
  });
}

function w4djPlaylistPaths(paths: string[]): string[] {
  return paths.filter((path) => (path.replaceAll('\\', '/').split('/').pop() ?? '').toLowerCase().endsWith('.w4dj'));
}

const defaultEssentiaModelStatus: EssentiaModelStatus = {
  version: '',
  embedding: false,
  genre: false,
  mood: false,
  instrument: false,
  installing: false,
  emotionContinuous: false,
  emotionCluster: false,
  discogsEffnet: {
    embedding: false,
    moodTheme: false,
    approachability: false,
    instrumentation: false,
    timbre: false,
    danceability: false,
  },
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
    neteaseSituationLabel: '情况',
    neteaseStatusLoading: '正在读取网易云状态…',
    neteaseIndexNotReady: '网易云轻量索引未就绪，转换前会按需准备',
    neteaseIndexBuilding: '正在建立网易云轻量索引',
    neteaseIndexReady: '网易云轻量索引已就绪',
    neteaseIndexCancelling: '正在取消网易云轻量索引…',
    neteaseIndexCancelled: '网易云轻量索引已取消',
    neteaseIndexError: '网易云轻量索引失败',
    scanLocalNetease: '扫描本地网易云文件夹',
    selectNeteaseDatabase: '选择网易云数据库',
    clearNeteaseDatabase: '恢复自动定位',
    neteaseDatabaseSelected: '已选择数据库，开始转换或分析时使用',
    neteaseDatabaseUnavailable: '未找到可用网易云数据库，将使用自动定位',
    scanLocalNeteaseRunning: '正在扫描本地网易云文件夹…',
    scanLocalNeteaseFallback: '手动选择文件夹',
    scanLocalNeteaseNotFound: '未能自动找到网易云音乐文件夹，请手动选择',
    scanLocalNeteaseSelected: '已选择网易云音乐文件夹',
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
    concurrencyLimit: '并行处理数量',
    activeConcurrency: '当前任务并发',
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
    inputTracks: '输入曲目',
    outputDuplicates: '输出重复曲目',
    willOverwrite: '将覆盖',
    willUpdateMetadata: '将更新元数据',
    willRename: '将重命名',
    errorFiles: '错误文件',
    duplicateDisambiguated: '同名歌曲共：{count}首，已按专辑区分并写入文件名',
    estimatedOutput: '预计输出',
    confirmStart: '确认并开始转换',
    cancel: '取消',
    noProcessableFiles: '没有可处理的文件',
    history: '转换历史',
    noHistory: '还没有转换记录',
    retryFailures: '重试失败项目',
    exportSession: '导出错误报告',
    exportRuntimeSession: '导出运行会话记录',
    exportReportSuccess: '错误报告已导出',
    exportReportFailed: '错误报告导出失败',
    exportRuntimeSuccess: '运行会话记录已导出',
    exportRuntimeFailed: '运行会话记录导出失败',
    reportPath: '错误报告位置',
    completedCount: '完成',
    failedCount: '失败',
    sourcePath: '输入来源',
    destinationPath: '输出目录',
    conflictStrategy: '已存在歌曲策略',
    conflictSkip: '跳过',
    conflictOverwrite: '覆盖',
    conflictMetadata: '仅更新元数据',
    filenameRule: '输出文件名规则',
    titleArtist: '歌曲名 - 歌手（默认）',
    artistTitle: '歌手 - 歌曲名',
    originalName: '保留原文件名',
    availableSpace: '可用空间',
    databasePath: '数据库目录',
    previewDetails: '查看明细',
    previewDetailClose: '关闭明细',
    openFile: '打开文件',
    noDetailItems: '没有符合条件的曲目',
    insufficientSpace: '磁盘空间不足，无法开始转换',
    cancelTask: '取消任务',
    resumeTasks: '继续未完成任务',
    resumeAnalysis: '继续未完成分析',
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
    essentiaModelsReady: '内置模型已就绪，增强模式会识别流派、情绪和人声/器乐。',
    essentiaModelsMissing: '内置模型缺失或损坏；重启应用后会自动修复，当前仍可进行基础分析和 Drop LUFS。',
    essentiaModelsDropDisabled: '模型导入入口已移除，增强模式使用内置模型。',
    scanTitle: '扫描歌曲',
    scanPreparing: '正在准备扫描',
    scanSource: '正在扫描输入目录',
    scanDestination: '正在扫描输出目录',
    scanMatchingMetadata: '正在匹配网易云元数据',
    scanChecking: '正在检查转换条件',
    scanAnalyzing: '正在分析歌曲并写入元数据',
    scanCompleted: '扫描完成',
    scanSucceeded: '扫描成功',
    scanCancelled: '扫描已取消',
    scanError: '扫描失败',
    scanCurrentFile: '当前文件',
    scanCancel: '取消扫描',
    conversionCancel: '取消转换',
    conversionRunning: '正在转换',
    analysisCancel: '取消分析',
    scanClose: '关闭',
    importDjPlaylist: '导入.w4dj',
    openLatestDjPlaylist: '导出播放列表',
    djPlaylistDialogTitle: 'DJ 歌单',
    djPlaylistSource: '如何获得 .w4dj？使用这个老炮DJ Skill：',
    djPlaylistSourceLink: 'dj-crate-digger',
    djPlaylistImportButton: '导入.w4dj',
    djPlaylistExportButton: '导出播放列表',
    djPlaylistChooseRecent: '选择最近歌单',
    djPlaylistCopyAudioTitle: '是否复制歌单中的音频？',
    djPlaylistCopyAudio: '是，复制音频并导出',
    djPlaylistUseExistingAudio: '否，仅导出歌单',
    djPlaylistCopyAudioExplanation: '是，复制音频并导出：歌曲会复制到导出文件夹，歌单可独立使用，但会占用更多磁盘空间。',
    djPlaylistUseExistingAudioExplanation: '否，仅导出歌单：不会复制歌曲。请勿移动或删除原音频，否则歌单可能无法播放。',
    djPlaylistExportPreparing: '正在准备播放列表…',
    djPlaylistExportCopied: '已复制 {copied}/{matched} 首音频',
    djPlaylistExportReferenced: '未复制音频，仅导出歌单',
    djPlaylistExportPortableError: '复制导出未生成完整的跨账户歌单，请重新导出。',
    djPlaylistInstructions: '1. 如何把歌单导入到网易云：导入 .w4dj 之后，扫描二维码，打开网易云-我的-三竖点-一键导入外部歌单-文字导入，粘贴结果即可导入歌单\n2. 如何把播放列表导入到Rekordbox：在 W4DJ RKB 进行成功转换之后，可以一键导出 m3u8。然后打开Rekordbox-文件-导入-导入播放列表',
    djPlaylistDrop: '松开导入 DJ 歌单',
    djPlaylistImporting: '正在导入 DJ 歌单…',
    djPlaylistTracks: '首歌曲',
    djPlaylistSkipped: '条重复已跳过',
    djPlaylistPage: '第 {current}/{total} 页',
    djPlaylistBytes: '{tracks} 首 · {bytes} 字节',
    djPlaylistPrevious: '上一页',
    djPlaylistNext: '下一页',
    djPlaylistCopyPage: '复制当前页',
    djPlaylistCopyAll: '复制全部',
    djPlaylistExportTxt: '导出 TXT',
    djPlaylistMatchExport: '识别并生成 M3U8',
    djPlaylistRematch: '重新识别',
    djPlaylistExportM3u8: '生成 M3U8',
    djPlaylistPartialExport: '仅导出已匹配',
    djPlaylistMatched: '已匹配 {matched}/{total}',
    djPlaylistUnresolved: '未解决歌曲',
    djPlaylistClose: '关闭',
    djPlaylistImportError: 'DJ 歌单导入失败',
    djPlaylistExportSuccess: 'M3U8 已导出',
    djPlaylistPartialConfirm: '仍有 {count} 首未匹配。只导出已匹配歌曲吗？',
  },
  en: {
    eyebrow: 'W4DJ RKB',
    title: 'If I Were a DJ',
    railLead: 'Output mode',
    sourceKicker: 'Music folders or tracks (NetEase, SoundCloud, etc.)',
    destKicker: 'Task 1 and Task 2 run independently. Scroll when the window is short.',
    sourceLabel: 'Music Folder or Track',
    neteaseSituationLabel: 'Status',
    neteaseStatusLoading: 'Reading NetEase status…',
    neteaseIndexNotReady: 'NetEase lightweight index is not ready; it will be prepared when needed',
    neteaseIndexBuilding: 'Building NetEase lightweight index',
    neteaseIndexReady: 'NetEase lightweight index is ready',
    neteaseIndexCancelling: 'Cancelling NetEase lightweight index…',
    neteaseIndexCancelled: 'NetEase lightweight index cancelled',
    neteaseIndexError: 'NetEase lightweight index failed',
    scanLocalNetease: 'Scan local NetEase folder',
    selectNeteaseDatabase: 'Choose NetEase database',
    clearNeteaseDatabase: 'Use automatic location',
    neteaseDatabaseSelected: 'Database selected; it will be used when conversion or analysis starts',
    neteaseDatabaseUnavailable: 'No usable NetEase database; automatic location will be used',
    scanLocalNeteaseRunning: 'Scanning local NetEase folder…',
    scanLocalNeteaseFallback: 'Choose folder manually',
    scanLocalNeteaseNotFound: 'Could not auto-locate a NetEase music folder. Choose one manually.',
    scanLocalNeteaseSelected: 'NetEase music folder selected',
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
    concurrencyLimit: 'Parallel processing count',
    activeConcurrency: 'Current task concurrency',
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
    inputTracks: 'Input tracks',
    outputDuplicates: 'Duplicate outputs',
    willOverwrite: 'Will overwrite',
    willUpdateMetadata: 'Will update metadata',
    willRename: 'Will rename',
    errorFiles: 'Errors',
    duplicateDisambiguated: 'Duplicate songs: {count}; separated by album and written into filenames',
    estimatedOutput: 'Estimated output',
    confirmStart: 'Confirm and convert',
    cancel: 'Cancel',
    noProcessableFiles: 'No files to process',
    history: 'Conversion history',
    noHistory: 'No conversion history yet',
    retryFailures: 'Retry failed files',
    exportSession: 'Export error report',
    exportRuntimeSession: 'Export runtime session',
    exportReportSuccess: 'Error report exported',
    exportReportFailed: 'Error report export failed',
    exportRuntimeSuccess: 'Runtime session exported',
    exportRuntimeFailed: 'Runtime session export failed',
    reportPath: 'Error report path',
    completedCount: 'Completed',
    failedCount: 'Failed',
    sourcePath: 'Input source',
    destinationPath: 'Output',
    conflictStrategy: 'Existing song strategy',
    conflictSkip: 'Skip',
    conflictOverwrite: 'Overwrite',
    conflictMetadata: 'Update metadata only',
    filenameRule: 'Output filename rule',
    titleArtist: 'Title - Artist (default)',
    artistTitle: 'Artist - Title',
    originalName: 'Keep original filename',
    availableSpace: 'Available space',
    databasePath: 'Database directory',
    previewDetails: 'View details',
    previewDetailClose: 'Close details',
    openFile: 'Open file',
    noDetailItems: 'No matching tracks',
    insufficientSpace: 'Not enough disk space to start',
    cancelTask: 'Cancel task',
    resumeTasks: 'Resume unfinished tasks',
    resumeAnalysis: 'Resume unfinished analysis',
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
    essentiaModelsReady: 'Bundled models are ready; Enhanced mode can identify genre, mood, and voice/instrument.',
    essentiaModelsMissing: 'Bundled models are missing or damaged; restart the app to repair them automatically. Basic analysis and Drop LUFS still work.',
    essentiaModelsDropDisabled: 'Model import is no longer available; Enhanced mode uses the bundled models.',
    scanTitle: 'Scanning songs',
    scanPreparing: 'Preparing scan',
    scanSource: 'Scanning input folders',
    scanDestination: 'Scanning output folders',
    scanMatchingMetadata: 'Matching NetEase metadata',
    scanChecking: 'Checking conversion conditions',
    scanAnalyzing: 'Analyzing tracks and writing metadata',
    scanCompleted: 'Scan complete',
    scanSucceeded: 'Scan succeeded',
    scanCancelled: 'Scan cancelled',
    scanError: 'Scan failed',
    scanCurrentFile: 'Current file',
    scanCancel: 'Cancel scan',
    conversionCancel: 'Cancel conversion',
    conversionRunning: 'Converting',
    analysisCancel: 'Cancel analysis',
    scanClose: 'Close',
    importDjPlaylist: 'Import .w4dj',
    openLatestDjPlaylist: 'Export playlist',
    djPlaylistDialogTitle: 'DJ playlist',
    djPlaylistSource: 'How to get .w4dj? Use this veteran DJ Skill:',
    djPlaylistSourceLink: 'dj-crate-digger',
    djPlaylistImportButton: 'Import .w4dj',
    djPlaylistExportButton: 'Export playlist',
    djPlaylistChooseRecent: 'Choose a recent playlist',
    djPlaylistCopyAudioTitle: 'Copy audio files with the playlist?',
    djPlaylistCopyAudio: 'Yes, copy audio and export',
    djPlaylistUseExistingAudio: 'No, export playlist only',
    djPlaylistCopyAudioExplanation: 'Yes, copy audio and export: Songs will be copied to the export folder so the playlist can be used independently, but this uses more disk space.',
    djPlaylistUseExistingAudioExplanation: 'No, export playlist only: Songs will not be copied. Do not move or delete the original audio, or the playlist may stop playing.',
    djPlaylistExportPreparing: 'Preparing playlist…',
    djPlaylistExportCopied: 'Copied {copied}/{matched} audio files',
    djPlaylistExportReferenced: 'Playlist exported without copying audio',
    djPlaylistExportPortableError: 'The copied export is not a complete cross-account playlist. Please export it again.',
    djPlaylistInstructions: '1. Import a playlist into NetEase Cloud Music: import the .w4dj file, scan the QR code, open NetEase Cloud Music → My → ⋮ → Import external playlist → Text import, then paste the result.\n2. Import the playlist into Rekordbox: after a successful conversion in W4DJ RKB, export the m3u8, then open Rekordbox → File → Import → Import playlist.',
    djPlaylistDrop: 'Drop to import DJ playlist',
    djPlaylistImporting: 'Importing DJ playlist…',
    djPlaylistTracks: 'tracks',
    djPlaylistSkipped: 'duplicates skipped',
    djPlaylistPage: 'Page {current}/{total}',
    djPlaylistBytes: '{tracks} tracks · {bytes} bytes',
    djPlaylistPrevious: 'Previous',
    djPlaylistNext: 'Next',
    djPlaylistCopyPage: 'Copy page',
    djPlaylistCopyAll: 'Copy all',
    djPlaylistExportTxt: 'Export TXT',
    djPlaylistMatchExport: 'Recognize and generate M3U8',
    djPlaylistRematch: 'Recognize again',
    djPlaylistExportM3u8: 'Generate M3U8',
    djPlaylistPartialExport: 'Export matched only',
    djPlaylistMatched: 'Matched {matched}/{total}',
    djPlaylistUnresolved: 'Unresolved tracks',
    djPlaylistClose: 'Close',
    djPlaylistImportError: 'DJ playlist import failed',
    djPlaylistExportSuccess: 'M3U8 exported',
    djPlaylistPartialConfirm: '{count} tracks are unresolved. Export matched tracks only?',
  },
} as const;

function t(key: keyof typeof translations.zh, lang: AppLanguage): string {
  return translations[lang][key];
}

export type NeteaseSituationTone = 'neutral' | 'running' | 'success' | 'warning' | 'error';

export type NeteaseSituation = {
  message: string;
  tone: NeteaseSituationTone;
};

/**
 * Resolve the one-line Task 1 NetEase status without coupling the toolbar to
 * any particular backend command.  The backend may be unavailable during the
 * first render, so loading is intentionally a first-class state.
 */
export function resolveNeteaseSituation(
  database: NeteaseMetadataDatabaseUiState | undefined,
  lang: AppLanguage,
): NeteaseSituation {
  const status = database?.status;
  if (!database || !status) {
    return { message: t('neteaseStatusLoading', lang), tone: 'running' };
  }

  if (database.error) {
    return { message: database.error, tone: 'error' };
  }

  const notReadyWarning = status.warning
    && (status.warning.includes('未就绪') || status.warning.includes('not ready'));
  if (status.warning && !notReadyWarning) {
    // Other warnings (for example a stale manual database) remain visibly
    // distinct and take precedence over ordinary cache progress.
    return { message: status.warning, tone: 'warning' };
  }

  const cacheStatus = status.cacheStatus;
  if (database.busy || cacheStatus === 'building' || cacheStatus === 'cancelling') {
    const message = database.message
      || (cacheStatus === 'cancelling'
        ? t('neteaseIndexCancelling', lang)
        : t('neteaseIndexBuilding', lang));
    return {
      message,
      tone: 'running',
    };
  }

  if (cacheStatus === 'cancelled') {
    return { message: database.message || t('neteaseIndexCancelled', lang), tone: 'warning' };
  }

  if (cacheStatus === 'error') {
    return {
      message: database.error || database.message || t('neteaseIndexError', lang),
      tone: 'error',
    };
  }

  if (notReadyWarning) {
    return { message: status.warning!, tone: 'neutral' };
  }

  if (status.source === 'manual' && status.manualPath) {
    return { message: t('neteaseDatabaseSelected', lang), tone: 'success' };
  }

  if (status.source === 'automatic' && (cacheStatus === 'ready' || status.loaded)) {
    const count = status.cachedRecordCount ?? status.recordCount;
    const suffix = count > 0 ? ` · ${count} ${lang === 'zh' ? '条' : count === 1 ? 'record' : 'records'}` : '';
    return { message: `${t('neteaseIndexReady', lang)}${suffix}`, tone: 'success' };
  }

  if (status.source === 'unavailable') {
    return { message: t('neteaseDatabaseUnavailable', lang), tone: 'warning' };
  }

  if (database.message) {
    return { message: database.message, tone: 'neutral' };
  }

  return { message: t('neteaseIndexNotReady', lang), tone: 'neutral' };
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
    activeConcurrencyLimit: null,
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
  concurrencyLimit: 2,
  lang: initialLanguage,
  theme: initialTheme,
};

const defaultAnalysisState: AppAnalysisState = {
  slotIndex: null,
  status: 'idle',
  completed: 0,
  total: 0,
  resultCount: 0,
  failedCount: 0,
  message: '',
  currentItem: '',
  stage: '',
  stageProcessed: 0,
  stageTotal: 0,
  workerJobId: '',
  startedAt: '',
  resumeAvailable: false,
};

type SourcePickerOpenOptions = {
  directory: boolean;
  title: string;
  filters?: Array<{ name: string; extensions: string[] }>;
};

type SaveFileOptions = {
  defaultPath: string;
  title: string;
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
      const errorText = error instanceof Error ? error.message : String(error);
      if (!errorText.includes('unified source picker is only available on macOS')) {
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
  chooseConcurrencyLimit: (value) =>
    invoke<DesktopState>('choose_concurrency_limit', { value }),
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
    batchId = undefined,
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
  loadIncompleteAnalysisRun: () => invoke<ResumableAnalysis | null>('load_incomplete_analysis_run'),
  claimAnalysisRun: (batchId, attemptId) => invoke<boolean>('claim_analysis_run', { batchId, attemptId }),
  retryHistoryFailures: (id) => invoke<AppPreview>('retry_history_failures', { id }),
  exportHistoryErrorReport: (id, path) =>
    invoke<void>('export_history_error_report', { id, path }),
  exportRuntimeSession: (id, path) =>
    invoke<void>('export_runtime_session', { id, path }),
  saveFile: (options) => save(options),
  recordRuntimeSessionEvent: (batchId, event, details) =>
    invoke<void>('record_runtime_session_event', { batchId, event, details }),
  finalizeAnalysisSession: (batchId) =>
    invoke<void>('finalize_analysis_session', { batchId }),
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
  ensureEssentiaModels: () => invoke<EssentiaModelStatus>('ensure_essentia_models'),
  loadEssentiaModel: async (id) => normalizeEssentiaModel(
    await invoke<EssentiaModelWire>('load_essentia_model', { id }),
  ),
  createAnalysisWorker: () => new AnalysisWorkerClient(),
  loadLibraryStatus: () => invoke<LibraryStatus>('load_library_status'),
  locateNeteaseLibrary: (force = false) =>
    invoke<LibraryStatus['netease']>('locate_netease_library', { force }),
  refreshLibraryCatalog: () => invoke<LibraryRefreshProgress>('refresh_library_catalog'),
  cancelLibraryRefresh: () => invoke<LibraryRefreshProgress>('cancel_library_refresh'),
  loadNeteaseMetadataDatabaseStatus: () =>
    invoke<NeteaseMetadataDatabaseStatus>('load_netease_metadata_database_status'),
  prepareNeteaseMetadataCache: () =>
    invoke<NeteaseMetadataCacheProgress>('prepare_netease_metadata_cache'),
  cancelNeteaseMetadataCache: () =>
    invoke<NeteaseMetadataCacheProgress>('cancel_netease_metadata_cache'),
  loadNeteaseMetadataCacheStatus: () =>
    invoke<NeteaseMetadataCacheProgress>('load_netease_metadata_cache_status'),
  selectNeteaseMetadataDatabase: (path) =>
    invoke<NeteaseMetadataDatabaseStatus>('select_netease_metadata_database', { path }),
  clearNeteaseMetadataDatabase: () =>
    invoke<NeteaseMetadataDatabaseStatus>('clear_netease_metadata_database'),
  selectNeteaseDatabaseFallback: (path) => invoke<LibraryStatus>('select_netease_database_fallback', { path }),
  clearNeteaseDatabaseFallback: () => invoke<LibraryStatus>('clear_netease_database_fallback'),
  pickNeteaseDatabase: async () => {
    const selected = await open({
      directory: false,
      multiple: false,
      title: '选择网易云数据库',
      filters: [{ name: 'SQLite database', extensions: ['sqlite3', 'sqlite', 'db'] }],
    });
    return typeof selected === 'string' ? selected : null;
  },
  listenLibraryRefreshProgress: (handler) =>
    listen<LibraryRefreshProgress>('library-refresh-progress', (event) => handler(event.payload)),
  listenNeteaseDiscoveryProgress: (handler) =>
    listen<NeteaseDiscoveryProgress>('netease-discovery-progress', (event) => handler(event.payload)),
  listenNeteaseMetadataCacheProgress: (handler) =>
    listen<NeteaseMetadataCacheProgress>('netease-metadata-cache-progress', (event) => handler(event.payload)),
  findInvalidLibraryTracks: () => invoke<LibraryInvalidScanProgress>('find_invalid_library_tracks'),
  cancelInvalidLibraryScan: () => invoke<LibraryInvalidScanProgress>('cancel_invalid_library_scan'),
  listenInvalidLibraryScanProgress: (handler) =>
    listen<LibraryInvalidScanProgress>('library-invalid-scan-progress', (event) => handler(event.payload)),
  queryLibraryCatalog: (query) => invoke<LibraryPage>('query_library_catalog', { query }),
  getLibraryTrackDetail: (trackKey) => invoke<LibraryTrack | null>('get_library_track_detail', { trackKey }),
  getLibraryTrackSourceRecords: (trackKey) => invoke<LibrarySourceRecord[]>('get_library_track_source_records', { trackKey }),
  getLibraryTrackCover: (trackKey) => invoke<string | null>('get_library_track_cover', { trackKey }),
  listLibraryAnalysisCandidates: () => invoke<LibraryAnalysisCandidate[]>('list_library_analysis_candidates'),
  clearLibraryCatalogCache: () => invoke<void>('clear_library_catalog_cache'),
  pickLibraryTrackFile: async () => {
    const lang = (localStorage.getItem('w4dj_lang') as AppLanguage) || 'zh';
    const selected = await open({
      directory: false,
      multiple: false,
      title: lang === 'zh' ? '选择新的歌曲文件' : 'Choose a replacement audio file',
      filters: [{
        name: lang === 'zh' ? '支持的音频文件' : 'Supported audio files',
        extensions: ['mp3', 'flac', 'ncm', 'wav', 'aif', 'aiff'],
      }],
    });
    return typeof selected === 'string' ? selected : null;
  },
  relocateLibraryTrack: (trackKey, path) =>
    invoke<void>('relocate_library_track', { trackKey, path }),
  removeLibraryTrack: (trackKey) =>
    invoke<boolean>('remove_library_track', { trackKey }),
  clearInvalidLibraryTracks: () =>
    invoke<number>('clear_invalid_library_tracks'),
  pickW4djPlaylist: async () => {
    const selected = await open({
      directory: false,
      multiple: false,
      title: '导入.w4dj',
      filters: [{ name: 'W4DJ playlist', extensions: ['w4dj'] }],
    });
    return typeof selected === 'string' ? selected : null;
  },
  importW4djPlaylist: (path) => invoke<ImportedDjPlaylist>('import_w4dj_playlist', { path }),
  listImportedDjPlaylists: () => invoke<ImportedDjPlaylistSummary[]>('list_imported_dj_playlists'),
  loadImportedDjPlaylist: (playlistId) => invoke<ImportedDjPlaylist>('load_imported_dj_playlist', { playlistId }),
  exportImportedDjPlaylistW4dj: (playlistId, path) =>
    invoke<void>('export_imported_dj_playlist_w4dj', { playlistId, path }),
  exportNeteasePlaylistText: (path, text) => invoke<void>('export_netease_playlist_text', { path, text }),
  matchImportedDjPlaylist: (playlistId) => invoke<DjPlaylistMatchReport>('match_imported_dj_playlist', { playlistId }),
  loadImportedDjPlaylistMatches: (playlistId) => invoke<DjPlaylistMatchReport>('load_imported_dj_playlist_matches', { playlistId }),
  setImportedDjPlaylistMatch: (playlistId, position, trackKey) =>
    invoke<DjPlaylistMatchReport>('set_imported_dj_playlist_match', { playlistId, position, trackKey }),
  clearImportedDjPlaylistMatch: (playlistId, position) =>
    invoke<DjPlaylistMatchReport>('clear_imported_dj_playlist_match', { playlistId, position }),
  exportImportedDjPlaylistM3u8: (playlistId, path, allowPartial, copyAudio = false) =>
    invoke<DjPlaylistM3u8ExportResult>('export_imported_dj_playlist_m3u8', {
      playlistId,
      path,
      allowPartial,
      copyAudio,
    }),
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
  neteaseDiscoveryProgress: NeteaseDiscoveryProgress | null = null,
  libraryRefreshProgress: LibraryRefreshProgress | null = null,
  neteaseDiscoveryInFlight = false,
  neteaseMetadataDatabase: NeteaseMetadataDatabaseUiState = {
    status: null,
    busy: false,
    message: null,
    error: null,
  },
  djPlaylistState: DjPlaylistUiState | null = null,
  importedDjPlaylistSummaries: ImportedDjPlaylistSummary[] = [],
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
  const scanRunning = scanProgress?.status === 'running' || scanProgress?.status === 'cancelling';
  const scanVisible = Boolean(scanProgress && scanProgress.status !== 'idle');
  const scanCancelling = scanProgress?.status === 'cancelling';
  const analysisRunning = analysisState.status === 'running';
  const analysisResumeAvailable = analysisState.status === 'cancelled'
    && analysisState.resumeAvailable === true;
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
        <button type="button" class="help-button" data-action="open-library" aria-label="${state.lang === 'zh' ? '歌曲库' : 'Song library'}" title="${state.lang === 'zh' ? '歌曲库' : 'Song library'}" data-feature-hidden="${SONG_LIBRARY_FEATURE_VISIBLE ? 'false' : 'true'}"${SONG_LIBRARY_FEATURE_VISIBLE ? '' : ' hidden'}>
          ${icon('list')}
          <span>${state.lang === 'zh' ? '歌曲库' : 'Library'}</span>
        </button>
        <button type="button" class="help-button" data-action="import-dj-playlist" aria-label="${t('importDjPlaylist', state.lang)}" title="${t('importDjPlaylist', state.lang)}">
          ${icon('list')}
          <span>${t('importDjPlaylist', state.lang)}</span>
        </button>
        ${importedDjPlaylistSummaries.length > 0 ? `<button type="button" class="help-button" data-action="open-latest-dj-playlist" aria-label="${t('openLatestDjPlaylist', state.lang)}" title="${t('openLatestDjPlaylist', state.lang)}">
          ${icon('list')}
          <span>${t('openLatestDjPlaylist', state.lang)}</span>
        </button>` : ''}
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
            class="enhanced-mode-row mode-row${ENHANCED_ANALYSIS_FEATURES_VISIBLE ? '' : ' enhanced-mode-row-hidden'}"
            data-role="enhanced-mode-switch"
            data-selected-enhanced-mode="${state.enhancedMode ? 'on' : 'off'}"
            data-feature-hidden="${ENHANCED_ANALYSIS_FEATURES_VISIBLE ? 'false' : 'true'}"
            aria-hidden="${ENHANCED_ANALYSIS_FEATURES_VISIBLE ? 'false' : 'true'}"
            ${ENHANCED_ANALYSIS_FEATURES_VISIBLE ? '' : 'inert'}
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
            <button type="button" class="global-action"${onboardingTarget === 'start' ? ' data-onboarding-target="start"' : ''} data-action="${scanRunning ? 'cancel-scan' : analysisRunning ? 'cancel-analysis' : conversionRunning ? 'cancel-all' : 'start-all'}" ${scanCancelling || (!scanRunning && !analysisRunning && !conversionRunning && (configuredTasks === 0 || pendingAction !== null)) ? 'disabled' : ''} aria-busy="${pendingAction !== null}">
              ${scanRunning || analysisRunning || conversionRunning ? icon('pause') : icon('play')}
              ${scanCancelling
                ? (state.lang === 'zh' ? '正在取消扫描…' : 'Cancelling scan…')
                : scanRunning
                  ? t('scanCancel', state.lang)
                : analysisRunning
                  ? t('analysisCancel', state.lang)
                  : conversionRunning
                    ? t('conversionCancel', state.lang)
                    : hasCancelled && !analysisResumeAvailable ? t('resumeTasks', state.lang) : t('startAll', state.lang)}
            </button>
            ${analysisResumeAvailable && !scanRunning && !analysisRunning && !conversionRunning && pendingAction === null
              ? `<button type="button" class="secondary-action analysis-resume-action" data-action="resume-analysis">${t('resumeAnalysis', state.lang)}</button>`
              : ''}
            ${!conversionRunning && scanProgress && (scanProgress.status === 'error' || scanProgress.status === 'cancelled' || scanProgress.status === 'cancelling')
              ? `<small class="global-stage-message" data-role="scan-message">${escapeHtml(scanProgress.message || scanPhaseLabel(scanProgress.phase, state.lang))}</small>`
                : !conversionRunning && neteaseDiscoveryProgress?.status === 'running'
                    ? `<small class="global-stage-message" data-role="netease-discovery-progress">${escapeHtml(neteaseDiscoveryProgress.message)} ${neteaseDiscoveryProgress.processed}${neteaseDiscoveryProgress.total == null ? '' : `/${neteaseDiscoveryProgress.total}`}</small>`
                    : !conversionRunning && neteaseDiscoveryProgress?.status === 'error'
                      ? `<small class="global-stage-message library-error" data-role="netease-discovery-progress">${escapeHtml(neteaseDiscoveryProgress.error || neteaseDiscoveryProgress.message)}</small>`
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
            scanVisible,
            neteaseDiscoveryProgress,
            libraryRefreshProgress,
            neteaseDiscoveryInFlight,
            analysisState,
            neteaseMetadataDatabase,
          )}
          ${renderSyncSlot(
            state,
            1,
            null,
            scanProgress?.tasks?.find((task) => task.slot_index === 1),
            scanVisible,
            null,
            null,
            false,
            analysisState,
            undefined,
          )}
        </div>
        ${renderHistory(history, state.lang, historyExpanded, historyLoadError)}
      </div>
    </section>
    ${renderPreviewModal(previewModal, state.lang, previewBusy)}
    ${renderAboutModal(aboutInfo, updateInfo, state.lang)}
    ${renderHelpModal(helpVisible, state.lang)}
    ${renderOnboardingModal(onboardingVisible, state.lang, onboardingStep)}
    ${renderLibraryDashboard(libraryState, state.lang)}
    ${renderDjPlaylistModal(djPlaylistState, state.lang)}
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
  const detailPreview = modal.detail
    ? modal.previews.find((item) => item.slot_index === modal.detail?.slotIndex)
    : undefined;
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
        ${detailPreview && modal.detail ? renderPreviewDetailDialog(detailPreview, modal.detail.kind, lang) : ''}
        ${canConfirm ? '' : `<p class="preview-empty">${t('noProcessableFiles', lang)}</p>`}
        <footer class="preview-actions">
          <button type="button" class="secondary-action" data-action="cancel-preview" ${busy ? 'disabled' : ''}>${t('cancel', lang)}</button>
          <button type="button" class="global-action preview-confirm" data-action="confirm-start" ${canConfirm && !busy ? '' : 'disabled'}>${busy ? t('scanning', lang) : t('confirmStart', lang)}</button>
        </footer>
      </div>
    </div>
  `;
}

function previewActionKind(item: AppPreview): string {
  return item.preview.action_kind || item.conflict_strategy;
}

function previewActionLabel(item: AppPreview, lang: AppLanguage): string {
  switch (previewActionKind(item)) {
    case 'overwrite': return t('willOverwrite', lang);
    case 'update_metadata': return t('willUpdateMetadata', lang);
    case 'rename': return t('willRename', lang);
    default: return t('willSkip', lang);
  }
}

function previewActionCount(item: AppPreview): number {
  if (item.preview.action_count != null) return item.preview.action_count;
  switch (item.conflict_strategy) {
    case 'overwrite': return item.preview.existing_count;
    case 'rename': return item.preview.new_count;
    case 'update_metadata': return item.preview.candidates.filter((candidate) => candidate.operation === 'update_metadata').length;
    default: return item.preview.skipped_count;
  }
}

function previewDetailItems(item: AppPreview, kind: PreviewDetailKind): AppPreviewDetailItem[] {
  const source = item.preview.detail_items || [];
  const fallback = [
    ...item.preview.candidates.map((candidate) => ({
      name: candidate.name,
      source_path: candidate.source_path,
      destination_path: candidate.destination_path,
      existing_output: false,
      classification: candidate.operation === 'update_metadata' ? 'update_metadata' : 'new',
      reason: null,
    })),
    ...item.preview.skipped.map((issue) => ({
      name: issue.path.split(/[\\/]/).pop() || issue.path,
      source_path: issue.path,
      destination_path: null,
      existing_output: item.conflict_strategy === 'skip',
      classification: 'skip',
      reason: issue.message,
    })),
    ...item.preview.errors.map((issue) => ({
      name: issue.path.split(/[\\/]/).pop() || issue.path,
      source_path: issue.path,
      destination_path: null,
      existing_output: false,
      classification: 'error',
      reason: issue.message,
    })),
  ] satisfies AppPreviewDetailItem[];
  const items = source.length > 0 ? source : fallback;
  const action = previewActionKind(item);
  return items
    .filter((detail) => {
      if (kind === 'input') return true;
      if (kind === 'duplicates') return detail.existing_output === true;
      if (kind === 'errors') return detail.classification === 'error';
      if (action === 'overwrite') return detail.classification === 'overwrite';
      if (action === 'update_metadata') return detail.classification === 'update_metadata';
      if (action === 'rename') return detail.classification === 'rename';
      return detail.classification === 'skip';
    })
    .sort((left, right) => left.name.localeCompare(right.name, undefined, { numeric: true, sensitivity: 'base' }));
}

function renderPreviewDetailDialog(item: AppPreview, kind: PreviewDetailKind, lang: AppLanguage): string {
  const items = previewDetailItems(item, kind);
  return `
    <div class="preview-detail-dialog" data-role="preview-detail-dialog" role="dialog" aria-modal="true">
      <header class="preview-detail-head">
        <h3>${escapeHtml(kind === 'input' ? t('inputTracks', lang) : kind === 'duplicates' ? t('outputDuplicates', lang) : kind === 'errors' ? t('errorFiles', lang) : previewActionLabel(item, lang))}</h3>
        <button type="button" class="secondary-action" data-action="close-preview-detail">${t('previewDetailClose', lang)}</button>
      </header>
      ${items.length === 0
        ? `<p class="preview-empty">${t('noDetailItems', lang)}</p>`
        : `<ol class="preview-detail-list">${items.map((detail) => {
          const target = detail.destination_path && (detail.existing_output || detail.classification === 'overwrite' || detail.classification === 'update_metadata')
            ? detail.destination_path
            : detail.source_path;
          const openTarget = detail.destination_path && (detail.existing_output || detail.classification === 'overwrite' || detail.classification === 'update_metadata') ? 'destination' : 'source';
          return `<li><button type="button" class="preview-detail-open" data-action="open-preview-file" data-open-target="${openTarget}" data-path="${escapeHtml(target)}" title="${t('openFile', lang)}">${icon('open')}</button><span>${escapeHtml(detail.name)}</span>${detail.reason ? `<small>${escapeHtml(humanizeError(detail.reason, lang))}</small>` : ''}</li>`;
        }).join('')}</ol>`}
    </div>
  `;
}

function renderPreviewCard(item: AppPreview, lang: AppLanguage): string {
  const preview = item.preview;
  const disambiguatedCount = preview.candidates.filter((candidate) => Boolean(candidate.disambiguation_reason)).length;
  const inputCount = preview.input_count ?? preview.new_count + preview.skipped_count + preview.error_count;
  const duplicateCount = preview.output_duplicate_count ?? preview.existing_count;
  const actionCount = previewActionCount(item);
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
          <div class="preview-card-head-meta">
            <span class="preview-batch-label">${escapeHtml(previewActionLabel(item, lang))}</span>
            <div class="preview-estimate-column"><div class="preview-estimate"><span>${t('estimatedOutput', lang)}</span><strong>${formatBytes(preview.estimated_output_bytes, lang)}</strong></div>${preview.available_space_bytes == null ? '' : `<div class="preview-available-space"><span>${t('availableSpace', lang)}</span><strong>${formatBytes(preview.available_space_bytes, lang)}</strong></div>`}</div>
          </div>
      </header>
      <dl class="preview-stats">
        <button type="button" class="preview-stat" data-action="preview-detail" data-slot="${item.slot_index}" data-detail-kind="input"><dt>${t('inputTracks', lang)}</dt><dd>${inputCount}</dd></button>
        <button type="button" class="preview-stat" data-action="preview-detail" data-slot="${item.slot_index}" data-detail-kind="duplicates"><dt>${t('outputDuplicates', lang)}</dt><dd>${duplicateCount}</dd></button>
        <button type="button" class="preview-stat preview-stat-action" data-action="preview-detail" data-slot="${item.slot_index}" data-detail-kind="action"><dt>${previewActionLabel(item, lang)}</dt><dd>${actionCount}</dd></button>
        <button type="button" class="preview-stat" data-action="preview-detail" data-slot="${item.slot_index}" data-detail-kind="errors"><dt>${t('errorFiles', lang)}</dt><dd class="preview-error-count">${preview.error_count}</dd></button>
      </dl>
      <div class="preview-paths">
        <p><span>${t('sourcePath', lang)}</span>${escapeHtml(preview.source_directory)}</p>
        <p><span>${t('destinationPath', lang)}</span>${escapeHtml(preview.destination_directory)}</p>
        ${preview.database_directory ? `<p><span>${t('databasePath', lang)}</span>${escapeHtml(preview.database_directory)}</p>` : ''}
      </div>
      ${preview.disk_space_sufficient === false ? `<p class="disk-space-error">${t('insufficientSpace', lang)}</p>` : ''}
      ${disambiguatedCount > 0 ? `<p class="preview-warning">${t('duplicateDisambiguated', lang).replace('{count}', String(disambiguatedCount))}</p>` : ''}
      ${issues ? `<ul class="preview-errors">${issues}</ul>` : ''}
    </article>
  `;
}

function djPlaylistText(key: keyof typeof translations.zh, lang: AppLanguage, values: Record<string, string | number> = {}): string {
  return Object.entries(values).reduce(
    (text, [name, value]) => text.replaceAll(`{${name}}`, String(value)),
    t(key, lang),
  );
}

function renderDjPlaylistModal(state: DjPlaylistUiState | null, lang: AppLanguage): string {
  if (!state) return '';
  const overlay = state.dropActive
    ? `<div class="dj-playlist-drop-overlay" data-role="dj-playlist-drop-overlay" aria-live="polite"><div>${t('djPlaylistDrop', lang)}</div></div>`
    : '';
  if (!state.visible) return overlay;
  if (state.exportPicker) {
    const recent = state.recentPlaylists || [];
    return `${overlay}
      <div class="dj-playlist-modal" data-role="dj-playlist-export-picker" role="dialog" aria-modal="true" aria-label="${t('djPlaylistChooseRecent', lang)}">
        <div class="dj-playlist-dialog dj-playlist-export-picker-dialog">
          <header class="dj-playlist-head">
            <div><p class="panel-kicker">W4DJ RKB</p><h2>${t('djPlaylistChooseRecent', lang)}</h2></div>
            <button type="button" class="secondary-action" data-action="close-dj-playlist">${t('djPlaylistClose', lang)}</button>
          </header>
          <div class="dj-playlist-recent-list">${recent.map((summary) => `<button type="button" class="dj-playlist-recent-item" data-action="dj-playlist-select-recent" data-playlist-id="${escapeHtml(summary.playlistId)}"><strong>${escapeHtml(summary.name)}</strong><span>${summary.trackCount} ${t('djPlaylistTracks', lang)}</span></button>`).join('')}</div>
        </div>
      </div>`;
  }
  if (state.exportChoice) {
    const playlistName = state.playlist?.name || t('djPlaylistDialogTitle', lang);
    const report = state.matchReport;
    return `${overlay}
      <div class="dj-playlist-modal" data-role="dj-playlist-export-choice" role="dialog" aria-modal="true" aria-label="${t('djPlaylistCopyAudioTitle', lang)}">
        <div class="dj-playlist-dialog dj-playlist-export-choice-dialog">
          <header class="dj-playlist-head">
            <div><p class="panel-kicker">W4DJ RKB</p><h2>${t('djPlaylistCopyAudioTitle', lang)}</h2></div>
            <button type="button" class="secondary-action" data-action="close-dj-playlist">${t('djPlaylistClose', lang)}</button>
          </header>
          <p class="dj-playlist-export-name">${escapeHtml(playlistName)}${report ? ` · ${report.matchedCount}/${report.total}` : ''}</p>
          <ul class="dj-playlist-export-explanation">
            <li>${t('djPlaylistCopyAudioExplanation', lang)}</li>
            <li>${t('djPlaylistUseExistingAudioExplanation', lang)}</li>
          </ul>
          <div class="dj-playlist-export-choice-actions">
            <button type="button" class="global-action" data-action="dj-playlist-export-copy" ${state.exportBusy ? 'disabled' : ''}>${t('djPlaylistCopyAudio', lang)}</button>
            <button type="button" class="secondary-action" data-action="dj-playlist-export-existing" ${state.exportBusy ? 'disabled' : ''}>${t('djPlaylistUseExistingAudio', lang)}</button>
          </div>
        </div>
      </div>`;
  }
  if (state.launcher) {
    return `${overlay}
      <div class="dj-playlist-modal" data-role="dj-playlist-launcher" role="dialog" aria-modal="true" aria-label="${t('djPlaylistDialogTitle', lang)}">
        <div class="dj-playlist-dialog dj-playlist-launcher-dialog">
          <header class="dj-playlist-head">
            <div><p class="panel-kicker">W4DJ RKB</p><h2>${t('djPlaylistDialogTitle', lang)}</h2></div>
            <button type="button" class="secondary-action" data-action="close-dj-playlist">${t('djPlaylistClose', lang)}</button>
          </header>
          <p class="dj-playlist-launcher-source">${t('djPlaylistSource', lang)} <a href="https://github.com/komakizhu/dj-crate-digger-skill" data-action="open-dj-crate-digger-link" target="_blank" rel="noreferrer">${t('djPlaylistSourceLink', lang)}</a></p>
          <div class="dj-playlist-launcher-actions">
            <button type="button" class="global-action dj-playlist-launcher-button" data-action="dj-playlist-open-import">${t('djPlaylistImportButton', lang)}</button>
            <button type="button" class="global-action dj-playlist-launcher-button" data-action="dj-playlist-open-export">${t('djPlaylistExportButton', lang)}</button>
          </div>
          <p class="dj-playlist-launcher-instructions">${escapeHtml(t('djPlaylistInstructions', lang))}</p>
        </div>
      </div>`;
  }
  const playlist = state.playlist;
  const qrDataUrls = state.qrDataUrls || (state.qrDataUrl ? [state.qrDataUrl] : []);
  return `${overlay}
    <div class="dj-playlist-modal" data-role="dj-playlist-dialog" role="dialog" aria-modal="true" aria-label="${t('djPlaylistDialogTitle', lang)}">
      <div class="dj-playlist-dialog">
        <header class="dj-playlist-head">
          <div><p class="panel-kicker">W4DJ RKB</p><h2>${escapeHtml(playlist?.name || t('djPlaylistDialogTitle', lang))}</h2></div>
          <button type="button" class="secondary-action" data-action="close-dj-playlist">${t('djPlaylistClose', lang)}</button>
        </header>
        ${state.busy ? `<p class="dj-playlist-loading">${t('djPlaylistImporting', lang)}</p>` : ''}
        ${state.error ? `<p class="library-error" role="alert">${escapeHtml(state.error)}</p>` : ''}
        ${playlist ? `
          <section class="dj-playlist-qr-panel">
            <div class="dj-playlist-qr-grid">${state.pages.map((page, index) => {
              const qrDataUrl = qrDataUrls[index];
              return `<div class="dj-playlist-qr-image">${qrDataUrl ? `<img src="${qrDataUrl}" alt="${escapeHtml(djPlaylistText('djPlaylistPage', lang, { current: index + 1, total: state.pages.length }))}">` : '<span>QR…</span>'}</div>`;
            }).join('')}</div>
            ${state.notice ? `<p class="dj-playlist-notice">${escapeHtml(state.notice)}</p>` : ''}
          </section>
        ` : ''}
      </div>
    </div>`;
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
  const analysis = entry.analysis;
  const analysisSummary = analysis
    ? lang === 'zh'
      ? `增强分析：${analysis.completed}/${analysis.total}，失败 ${analysis.failed}，超时 ${analysis.timedOut}，待处理 ${analysis.pending}`
      : `Enhanced analysis: ${analysis.completed}/${analysis.total}, failed ${analysis.failed}, timeout ${analysis.timedOut}, pending ${analysis.pending}`
    : lang === 'zh' ? '增强分析：未请求' : 'Enhanced analysis: not requested';
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
      <p class="history-conversion-summary">${lang === 'zh' ? '转换' : 'Conversion'}：${entry.completed_count}/${entry.new_count} · ${entry.failed_count} ${t('failedCount', lang)} · ${entry.pending_files.length} ${t('pendingCount', lang)}</p>
      <p class="history-analysis-summary" data-analysis-status="${escapeHtml(analysis?.status || 'notRequested')}">${escapeHtml(analysisSummary)}</p>
      ${entry.report_path ? `<p class="history-report-path">${t('reportPath', lang)}：${escapeHtml(entry.report_path)}</p>` : ''}
      ${failures ? `<details class="history-failures"><summary>${entry.failed_count} ${t('failedCount', lang)}</summary><ul>${failures}</ul></details>` : ''}
      <footer class="history-entry-actions">
        ${entry.failed_count > 0 || pendingFiles.length > 0 ? `<button type="button" class="secondary-action" data-action="retry-history" data-history-id="${escapeHtml(entry.id)}">${pendingFiles.length > 0 ? t('resumeTasks', lang) : t('retryFailures', lang)}</button>` : ''}
        <button type="button" class="secondary-action" data-action="export-error-report" data-history-id="${escapeHtml(entry.id)}">${t('exportSession', lang)}</button>
        <button type="button" class="secondary-action" data-action="export-runtime-session" data-history-id="${escapeHtml(entry.id)}">${t('exportRuntimeSession', lang)}</button>
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
  neteaseDiscoveryProgress: NeteaseDiscoveryProgress | null = null,
  libraryRefreshProgress: LibraryRefreshProgress | null = null,
  neteaseDiscoveryInFlight = false,
  analysisState: AppAnalysisState = defaultAnalysisState,
  neteaseMetadataDatabase: NeteaseMetadataDatabaseUiState | undefined = undefined,
): string {
  const slot = state.slots[slotIndex];
  const fallbackDestination = state.slots[0].destinationDirectory;
  const usesFallback = slotIndex === 1 && slot.destinationDirectory.trim() === '';
  const displayedDestination = usesFallback ? fallbackDestination : slot.destinationDirectory;
  const slotNumber = slotIndex + 1;
  const taskScanActive = scanActive && Boolean(scanTask);
  const sourcePhase = scanTask?.phase === 'scanning_source';
  const metadataPhase = scanTask?.phase === 'matching_metadata';
  const preparingPhase = scanTask?.phase === 'preparing';
  const completedPhase = scanTask?.phase === 'completed';
  const cancelledOrErrorPhase = scanTask?.phase === 'cancelled' || scanTask?.phase === 'error';
  const phaseProcessed = scanTask
    ? sourcePhase
      ? scanTask.source_processed ?? scanTask.processed
      : preparingPhase
        ? scanTask.processed
      : metadataPhase
        ? scanTask.metadata_processed ?? scanTask.processed
        : completedPhase || cancelledOrErrorPhase
          ? scanTask.source_processed ?? scanTask.processed
        : scanTask.destination_processed ?? scanTask.processed
    : 0;
  const phaseTotal = scanTask
    ? sourcePhase
      ? scanTask.source_total !== undefined
        ? scanTask.source_total
        : scanTask.total > 0 ? scanTask.total : null
      : preparingPhase
        ? scanTask.total > 0 ? scanTask.total : null
      : metadataPhase
        ? scanTask.metadata_total !== undefined
          ? scanTask.metadata_total
          : scanTask.source_total !== undefined
            ? scanTask.source_total
            : scanTask.total > 0 ? scanTask.total : null
        : completedPhase || cancelledOrErrorPhase
          ? scanTask.source_total !== undefined
            ? scanTask.source_total
            : scanTask.total > 0 ? scanTask.total : null
        : scanTask.destination_total !== undefined
          ? scanTask.destination_total
          : scanTask.total > 0 ? scanTask.total : null
    : null;
  // A stale backend may report a denominator from a previous enumeration.
  // Never render an impossible ratio; keep the phase indeterminate until its
  // own total catches up with the observed count.
  const renderedPhaseTotal = phaseTotal != null && phaseProcessed <= phaseTotal
    ? phaseTotal
    : null;
  const scanPercent = renderedPhaseTotal && renderedPhaseTotal > 0
    ? Math.min(100, Math.round((phaseProcessed / renderedPhaseTotal) * 100))
    : 0;
  const neteaseRefreshActive = slotIndex === 0 && isLibraryRefreshActive(libraryRefreshProgress);
  const neteaseDiscoveryActive = slotIndex === 0 && neteaseDiscoveryProgress?.status === 'running';
  const neteaseDiscoveryFailed = slotIndex === 0 && neteaseDiscoveryProgress?.status === 'error';
  const neteaseRefreshVisible = slotIndex === 0 && libraryRefreshProgress?.status !== undefined && libraryRefreshProgress.status !== 'idle';
  const neteaseDiscoveryVisible = slotIndex === 0 && neteaseDiscoveryProgress?.status !== undefined;
  const conversionInProgress = slotIndex === 0
    && slot.status === 'running'
    && !scanActive
    && analysisState.status !== 'running';
  const neteaseProgress = neteaseRefreshActive
    ? libraryRefreshProgress
    : neteaseDiscoveryActive
      ? neteaseDiscoveryProgress
      : neteaseRefreshVisible && !conversionInProgress
        ? libraryRefreshProgress
        : neteaseDiscoveryVisible
          ? neteaseDiscoveryProgress
          : null;
  const neteaseProgressTotal = neteaseProgress?.total;
  const neteaseProgressPercent = neteaseProgress && neteaseProgressTotal && neteaseProgressTotal > 0
    ? Math.min(100, Math.round((neteaseProgress.processed / neteaseProgressTotal) * 100))
    : 0;
  const neteaseProgressText = neteaseProgress
    ? `${neteaseProgress.message}${neteaseProgress.currentItem ? ` · ${neteaseProgress.currentItem}` : ''} ${neteaseProgress.processed}${neteaseProgressTotal == null ? '' : `/${neteaseProgressTotal}`}`
    : null;
  const analysisProgressVisible = analysisState.slotIndex === slotIndex
    && analysisState.status !== 'idle'
    && Boolean(analysisState.message);
  const analysisProgressSummary = analysisProgressVisible
    ? `${analysisState.message} ${analysisState.completed}/${analysisState.total}`
    : null;
  const analysisProgressText = analysisProgressVisible
    ? `${analysisProgressSummary}${analysisState.currentItem ? ` · ${analysisState.currentItem}` : ''}`
    : null;
  const analysisProgressPercent = analysisState.total > 0
    ? Math.min(100, Math.round((analysisState.completed / analysisState.total) * 100))
    : 0;
  const analysisIsActive = analysisState.status === 'running';
  const conversionProgressActive = slot.status === 'running'
    && !scanActive
    && !analysisIsActive
    && !neteaseRefreshActive
    && !neteaseDiscoveryActive;
  const scanProgressText = taskScanActive && scanTask
    ? renderedPhaseTotal == null
      ? `${scanTask.phase === 'completed' ? t('scanSucceeded', state.lang) : scanPhaseLabel(scanTask.phase, state.lang)} · ${state.lang === 'zh' ? `已扫描 ${phaseProcessed} 项` : `${phaseProcessed} scanned`}`
      : `${scanTask.phase === 'completed' ? t('scanSucceeded', state.lang) : scanPhaseLabel(scanTask.phase, state.lang)} ${phaseProcessed}/${renderedPhaseTotal}`
    : null;
  const displayedProgressText = (analysisIsActive ? analysisProgressText : null)
    || scanProgressText
    || (conversionProgressActive ? slot.progressText : null)
    || analysisProgressText
    || neteaseProgressText
    || slot.progressText;
  const showingAnalysisProgress = analysisProgressText !== null
    && displayedProgressText === analysisProgressText;
  const showProgressText = displayedProgressText !== slot.progressText
    ? true
    : slot.status !== 'idle' && slot.progressText !== t('idle', state.lang);
  const isNumericProgress = /^\d+\/\d+$/.test(displayedProgressText);
  const displayedProgressPercent = analysisIsActive && analysisProgressText !== null
    ? analysisProgressPercent
    : scanProgressText !== null
      ? scanPercent
      : conversionProgressActive
        ? progressPercent(slot)
        : analysisProgressText !== null
          ? analysisProgressPercent
          : neteaseProgress
            ? neteaseProgressPercent
            : progressPercent(slot);
  const scanProgressIndeterminate = scanProgressText !== null && renderedPhaseTotal == null;
  const indeterminateProgress = !showingAnalysisProgress
    && !conversionProgressActive
    && scanProgressText === null
    && Boolean(neteaseProgress && neteaseProgressTotal == null);
  const displayedSlotStatus = taskScanActive && !completedPhase && !cancelledOrErrorPhase
    ? 'running'
    : slot.status;
  const neteaseScanActive = neteaseDiscoveryInFlight || neteaseDiscoveryActive || neteaseRefreshActive;
  const neteaseScanLabel = neteaseScanActive
    ? t('scanLocalNeteaseRunning', state.lang)
    : neteaseDiscoveryFailed
      ? t('scanLocalNeteaseFallback', state.lang)
      : t('scanLocalNetease', state.lang);
  const manualDatabasePath = neteaseMetadataDatabase?.status?.manualPath || null;
  const databaseFileName = manualDatabasePath
    ? manualDatabasePath.split(/[\\/]/).filter(Boolean).pop() || manualDatabasePath
    : null;
  const databaseBusy = neteaseMetadataDatabase?.busy === true;
  const neteaseSituation = resolveNeteaseSituation(neteaseMetadataDatabase, state.lang);
  const progressCopy = showingAnalysisProgress
    ? `<span class="status-copy progress-copy analysis-progress-copy" data-role="analysis-message">
        <span class="analysis-progress-summary" data-role="analysis-summary">${escapeHtml(analysisProgressSummary || '')}</span>
        <span class="analysis-progress-current" data-role="analysis-current">${escapeHtml(analysisState.currentItem || '')}</span>
      </span>`
    : `<span class="status-copy progress-copy ${isNumericProgress ? 'progress-copy--numeric' : ''}" data-role="slot-progress-message">${escapeHtml(displayedProgressText)}</span>`;
  return `
    <article class="sync-slot-card" data-role="sync-slot" data-slot="${slotIndex}" data-status="${displayedSlotStatus}">
      <header class="sync-slot-head">
        <div>
          <h2>${t('syncSlot', state.lang)} ${slotNumber}</h2>
        </div>
        <div class="slot-head-actions">
          <span class="slot-status" data-status="${displayedSlotStatus}">${statusLabel(displayedSlotStatus, state.lang)}</span>
        </div>
      </header>

      <div class="path-flow">
          <div class="path-field" data-role="source-picker"${onboardingTarget === 'source' ? ' data-onboarding-target="source"' : ''} data-drop-kind="source" data-slot="${slotIndex}">
          <div class="path-field-heading">
            <span>${t('sourceLabel', state.lang)}</span>
            ${slotIndex === 0
              ? `<div class="netease-source-toolbar" data-role="netease-source-toolbar">
                  <div class="netease-database-status${neteaseSituation.tone === 'error' ? ' is-error' : ''}" data-role="netease-database-status" title="${escapeHtml(neteaseSituation.message)}">
                    <div class="netease-situation netease-situation--${neteaseSituation.tone}" data-role="netease-situation" data-tone="${neteaseSituation.tone}" title="${escapeHtml(neteaseSituation.message)}">
                      <span class="netease-situation-label">${t('neteaseSituationLabel', state.lang)}</span>
                      <span class="netease-situation-value" data-role="netease-situation-value">${escapeHtml(neteaseSituation.message)}</span>
                    </div>
                  </div>
                  <div class="netease-source-actions" data-role="netease-source-actions">
                    <button type="button" class="netease-scan-button" data-action="scan-local-netease" data-slot="0" ${neteaseScanActive ? 'disabled aria-busy="true"' : ''}>
                      ${icon('refresh')}
                      <span>${neteaseScanLabel}</span>
                    </button>
                    <button type="button" class="netease-scan-button netease-database-button" data-action="select-netease-database" data-slot="0" ${databaseBusy || neteaseScanActive ? 'disabled aria-busy="true"' : ''} title="${escapeHtml(manualDatabasePath || t('selectNeteaseDatabase', state.lang))}">
                      ${icon('folder')}
                      <span>${escapeHtml(databaseFileName || t('selectNeteaseDatabase', state.lang))}</span>
                    </button>
                    ${manualDatabasePath
                      ? `<button type="button" class="netease-scan-button netease-database-clear" data-action="clear-netease-database" data-slot="0" ${databaseBusy || neteaseScanActive ? 'disabled aria-busy="true"' : ''}>
                          <span>${t('clearNeteaseDatabase', state.lang)}</span>
                        </button>`
                      : ''}
                  </div>
                </div>`
              : ''}
          </div>
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
        ${showProgressText ? progressCopy : ''}
        <div class="progress-track" aria-hidden="true">
          <div class="progress-fill${indeterminateProgress || scanProgressIndeterminate ? ' is-indeterminate' : ''}"${showingAnalysisProgress ? ' data-role="analysis-progress"' : ' data-role="slot-progress"'} style="width: ${displayedProgressPercent}%"></div>
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
  let neteaseMetadataCachePreparing = false;
  let scanPreparationCancelled = false;
  let analysisCancelRequested = false;
  let analysisWorker: AnalysisWorkerSession | null = null;
  const terminatedAnalysisWorkers = new WeakSet<object>();
  const terminateAnalysisWorker = (worker: AnalysisWorkerSession | null, reason?: unknown) => {
    if (!worker || terminatedAnalysisWorkers.has(worker)) {
      if (analysisWorker === worker) {
        analysisWorker = null;
      }
      return;
    }
    terminatedAnalysisWorkers.add(worker);
    worker.terminate(reason);
    if (analysisWorker === worker) {
      analysisWorker = null;
    }
  };
  const onboardingSeen = (() => {
    try {
      return localStorage.getItem('w4dj_onboarding_seen') === '1'
        || sessionStorage.getItem('w4dj_onboarding_seen') === '1';
    } catch {
      // If storage is unavailable, do not trap the user in the guide on every
      // WebView reload.
      return true;
    }
  })();
  const wasWebViewReload = (() => {
    try {
      const navigation = performance.getEntriesByType('navigation')[0] as PerformanceNavigationTiming | undefined;
      return navigation?.type === 'reload';
    } catch {
      return false;
    }
  })();
  let onboardingVisible = !onboardingSeen && !wasWebViewReload;
  let onboardingStep: OnboardingStep = 0;
  const markOnboardingSeen = () => {
    try {
      localStorage.setItem('w4dj_onboarding_seen', '1');
    } catch {
      // The session marker below still prevents a reload loop when persistent
      // WebView storage is unavailable.
    }
    try {
      sessionStorage.setItem('w4dj_onboarding_seen', '1');
    } catch {
      // Ignore restricted storage environments.
    }
  };
  let analysisState: AppAnalysisState = { ...defaultAnalysisState };
  let analysisCache: TrackAnalysis[] = [];
  let analysisCacheLoadPromise: Promise<void> = Promise.resolve();
  // Conversion controls are rendered before the native desktop state arrives.
  // Keep the first snapshot as a barrier so a fast start click cannot make a
  // decision from the default (enhanced-off) state.
  let desktopStateHydration: Promise<void> = Promise.resolve();
  let analysisCacheRevision = 0;
  const loadResumableAnalysis = (): ResumableAnalysis | null => {
    try {
      const raw = localStorage.getItem(RESUMABLE_ANALYSIS_STORAGE_KEY);
      if (!raw) return null;
      const parsed = JSON.parse(raw) as Partial<ResumableAnalysis>;
      if (typeof parsed.batchId !== 'string' || !Array.isArray(parsed.previews)) {
        return null;
      }
      return parsed as ResumableAnalysis;
    } catch {
      return null;
    }
  };
  let resumableAnalysis: ResumableAnalysis | null = loadResumableAnalysis();
  const setResumableAnalysis = (value: ResumableAnalysis | null) => {
    resumableAnalysis = value;
    try {
      if (value) {
        localStorage.setItem(RESUMABLE_ANALYSIS_STORAGE_KEY, JSON.stringify(value));
      } else {
        localStorage.removeItem(RESUMABLE_ANALYSIS_STORAGE_KEY);
      }
    } catch {
      // Analysis can continue in memory when WebView storage is unavailable.
    }
  };
  const clearResumableAnalysisForBatch = (batchId: string) => {
    resumableAnalysis = null;
    try {
      const raw = localStorage.getItem(RESUMABLE_ANALYSIS_STORAGE_KEY);
      const stored = raw ? JSON.parse(raw) as Partial<ResumableAnalysis> : null;
      if (!stored?.batchId || stored.batchId === batchId) {
        localStorage.removeItem(RESUMABLE_ANALYSIS_STORAGE_KEY);
      }
    } catch {
      // Ignore unavailable storage; the in-memory state is still cleared.
    }
  };
  let analysisRunActive = false;
  let resumeAnalysisRequested = false;
  let libraryState: LibraryDashboardState | null = null;
  let librarySearchTimer: ReturnType<typeof setTimeout> | null = null;
  let librarySearchComposing = false;
  let libraryRenderDeferred = false;
  let draggedLibraryColumn: string | null = null;
  let resizingLibraryColumn: {
    id: string;
    startX: number;
    startWidth: number;
    width: number;
    header: HTMLElement;
  } | null = null;
  let libraryQueryRevision = 0;
  let neteaseDiscoveryProgress: NeteaseDiscoveryProgress | null = null;
  let libraryRefreshProgress: LibraryRefreshProgress | null = null;
  let neteaseDiscoveryInFlight = false;
  let neteaseMetadataDatabase: NeteaseMetadataDatabaseUiState = {
    status: null,
    busy: false,
    message: null,
    error: null,
  };
  let djPlaylistState: DjPlaylistUiState | null = null;
  let djPlaylistExportInFlight = false;
  let importedDjPlaylistSummaries: ImportedDjPlaylistSummary[] = [];

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
    if (librarySearchComposing) {
      libraryRenderDeferred = true;
      return;
    }
    const activeElement = document.activeElement;
    const activeLibrarySearch = activeElement instanceof HTMLInputElement
      && root.contains(activeElement)
      && activeElement.dataset.action === 'library-search'
      ? {
        start: activeElement.selectionStart ?? activeElement.value.length,
        end: activeElement.selectionEnd ?? activeElement.value.length,
      }
      : null;
    const currentLibraryTable = root.querySelector<HTMLElement>('.library-table-wrap');
    const libraryScroll = currentLibraryTable
      ? { top: currentLibraryTable.scrollTop, left: currentLibraryTable.scrollLeft }
      : null;
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
        neteaseDiscoveryProgress,
        libraryRefreshProgress,
        neteaseDiscoveryInFlight,
        neteaseMetadataDatabase,
        djPlaylistState,
        importedDjPlaylistSummaries,
      ),
    );

    if (onboardingVisible) {
      root.querySelector<HTMLButtonElement>('[data-action="onboarding-next"]')?.focus();
    } else if (helpVisible) {
      root.querySelector<HTMLButtonElement>('[data-action="close-help"]')?.focus();
    }

    if (!onboardingVisible && !helpVisible && activeLibrarySearch) {
      const nextSearch = root.querySelector<HTMLInputElement>('input[data-action="library-search"]');
      if (nextSearch) {
        nextSearch.focus();
        const start = Math.min(activeLibrarySearch.start, nextSearch.value.length);
        const end = Math.min(activeLibrarySearch.end, nextSearch.value.length);
        nextSearch.setSelectionRange(start, end);
      }
    }
    if (libraryScroll) {
      const nextLibraryTable = root.querySelector<HTMLElement>('.library-table-wrap');
      if (nextLibraryTable) {
        nextLibraryTable.scrollTop = libraryScroll.top;
        nextLibraryTable.scrollLeft = libraryScroll.left;
      }
    }

    const historyDetails = root.querySelector<HTMLDetailsElement>('[data-role="history"]');
    historyDetails?.querySelector('summary')?.addEventListener('click', () => {
      historyExpanded = !historyDetails.open;
    });
  };

  const updateNeteaseSituationDom = (): boolean => {
    const situation = root.querySelector<HTMLElement>('[data-role="netease-situation"]');
    const value = situation?.querySelector<HTMLElement>('[data-role="netease-situation-value"]');
    if (!situation || !value) {
      return false;
    }
    const resolved = resolveNeteaseSituation(neteaseMetadataDatabase, state.lang);
    situation.dataset.tone = resolved.tone;
    situation.className = `netease-situation netease-situation--${resolved.tone}`;
    situation.title = resolved.message;
    value.textContent = resolved.message;
    const container = situation.closest<HTMLElement>('[data-role="netease-database-status"]');
    if (container) {
      container.title = resolved.message;
      container.classList.toggle('is-error', resolved.tone === 'error');
    }
    return true;
  };

  const updateAnalysisProgressDom = () => {
    const analysisSlot = analysisState.slotIndex === null
      ? null
      : root.querySelector<HTMLElement>(
          `[data-role="sync-slot"][data-slot="${analysisState.slotIndex}"]`,
        );
    const messageElement = analysisSlot?.querySelector<HTMLElement>('[data-role="analysis-message"]');
    if (!messageElement || analysisState.status !== 'running') {
      return;
    }
    const summary = `${analysisState.message || t('analysisRunning', state.lang)} ${analysisState.completed}/${analysisState.total}`;
    const summaryElement = messageElement.querySelector<HTMLElement>('[data-role="analysis-summary"]');
    const currentElement = messageElement.querySelector<HTMLElement>('[data-role="analysis-current"]');
    if (summaryElement) {
      summaryElement.textContent = summary;
      if (currentElement) {
        currentElement.textContent = analysisState.currentItem || '';
      }
    } else {
      const current = analysisState.currentItem ? ` · ${analysisState.currentItem}` : '';
      messageElement.textContent = `${summary}${current}`;
    }
    messageElement.dataset.stage = analysisState.stage || '';
    messageElement.dataset.completed = String(analysisState.completed);
    messageElement.dataset.total = String(analysisState.total);
    if (analysisState.stageProcessed == null) {
      delete messageElement.dataset.stageProcessed;
    } else {
      messageElement.dataset.stageProcessed = String(analysisState.stageProcessed);
    }
    if (analysisState.stageTotal == null) {
      delete messageElement.dataset.stageTotal;
    } else {
      messageElement.dataset.stageTotal = String(analysisState.stageTotal);
    }
    const progressElement = analysisSlot?.querySelector<HTMLElement>('[data-role="analysis-progress"]');
    if (progressElement) {
      const percent = analysisState.total > 0
        ? Math.min(100, Math.round((analysisState.completed / analysisState.total) * 100))
        : 0;
      progressElement.style.width = `${percent}%`;
    }
  };

  const updateScanProgressDom = (progress: AppScanProgress): boolean => {
    if (!progress.tasks || progress.tasks.length === 0) {
      return false;
    }
    let updated = false;
    for (const task of progress.tasks) {
      const slot = root.querySelector<HTMLElement>(
        `[data-role="sync-slot"][data-slot="${task.slot_index}"]`,
      );
      const messageElement = slot?.querySelector<HTMLElement>('[data-role="slot-progress-message"]');
      const fill = slot?.querySelector<HTMLElement>('[data-role="slot-progress"]');
      if (!slot || !messageElement || !fill) {
        continue;
      }
      const sourcePhase = task.phase === 'scanning_source';
      const metadataPhase = task.phase === 'matching_metadata';
      const preparingPhase = task.phase === 'preparing';
      const completedPhase = task.phase === 'completed' || task.phase === 'cancelled' || task.phase === 'error';
      const processed = sourcePhase
        ? task.source_processed ?? task.processed
        : preparingPhase
          ? task.processed
        : metadataPhase
          ? task.metadata_processed ?? task.processed
          : completedPhase
            ? task.source_processed ?? task.processed
          : task.destination_processed ?? task.processed;
      const total = sourcePhase
        ? task.source_total ?? (task.total > 0 ? task.total : null)
        : preparingPhase
          ? (task.total > 0 ? task.total : null)
        : metadataPhase
          ? task.metadata_total ?? task.source_total ?? (task.total > 0 ? task.total : null)
          : completedPhase
            ? task.source_total ?? (task.total > 0 ? task.total : null)
          : task.destination_total ?? (task.total > 0 ? task.total : null);
      const renderedTotal = total != null && processed <= total ? total : null;
      const progressText = renderedTotal == null
        ? `${task.phase === 'completed' ? t('scanSucceeded', state.lang) : scanPhaseLabel(task.phase, state.lang)} · ${state.lang === 'zh' ? `已扫描 ${processed} 项` : `${processed} scanned`}`
        : `${task.phase === 'completed' ? t('scanSucceeded', state.lang) : scanPhaseLabel(task.phase, state.lang)} ${processed}/${renderedTotal}`;
      messageElement.textContent = progressText;
      const percent = renderedTotal && renderedTotal > 0
        ? Math.min(100, Math.round((processed / renderedTotal) * 100))
        : 0;
      fill.style.width = `${percent}%`;
      fill.classList.toggle('is-indeterminate', renderedTotal == null);
      updated = true;
    }
    return updated;
  };

  const updateLibraryRefreshProgressDom = (progress: LibraryRefreshProgress) => {
    const slot = root.querySelector<HTMLElement>('[data-role="sync-slot"][data-slot="0"]');
    if (!slot || slot.querySelector('[data-role="analysis-message"]')) return;
    const messageElement = slot.querySelector<HTMLElement>('[data-role="slot-progress-message"]');
    const currentItem = progress.currentItem ? ` · ${progress.currentItem}` : '';
    const total = progress.total == null ? '' : `/${progress.total}`;
    if (messageElement) {
      messageElement.textContent = `${progress.message}${currentItem} ${progress.processed}${total}`;
    }
    const fill = slot.querySelector<HTMLElement>('[data-role="slot-progress"]');
    if (fill) {
      const percent = progress.total && progress.total > 0
        ? Math.min(100, Math.round((progress.processed / progress.total) * 100))
        : 0;
      fill.style.width = `${percent}%`;
      fill.classList.toggle('is-indeterminate', progress.total == null);
    }
  };

  const updateNeteaseDiscoveryProgressDom = (progress: NeteaseDiscoveryProgress): boolean => {
    const slot = root.querySelector<HTMLElement>('[data-role="sync-slot"][data-slot="0"]');
    const messageElement = slot?.querySelector<HTMLElement>('[data-role="slot-progress-message"]');
    const canUpdateSlot = Boolean(messageElement && !slot?.querySelector('[data-role="analysis-message"]'));
    if (canUpdateSlot && messageElement) {
      const currentItem = progress.currentItem ? ` · ${progress.currentItem}` : '';
      const total = progress.total == null ? '' : `/${progress.total}`;
      messageElement.textContent = `${progress.message}${currentItem} ${progress.processed}${total}`;
      messageElement.dataset.stage = progress.stage;
      messageElement.dataset.processed = String(progress.processed);
      if (progress.total == null) {
        delete messageElement.dataset.total;
      } else {
        messageElement.dataset.total = String(progress.total);
      }
      const fill = slot?.querySelector<HTMLElement>('[data-role="slot-progress"]');
      if (fill) {
        const percent = progress.total && progress.total > 0
          ? Math.min(100, Math.round((progress.processed / progress.total) * 100))
          : 0;
        fill.style.width = `${percent}%`;
        fill.classList.toggle('is-indeterminate', progress.total == null);
      }
    }

    const globalMessage = root.querySelector<HTMLElement>('[data-role="netease-discovery-progress"]');
    if (globalMessage) {
      const total = progress.total == null ? '' : `/${progress.total}`;
      globalMessage.textContent = progress.status === 'error'
        ? progress.error || progress.message
        : `${progress.message} ${progress.processed}${total}`;
    }
    return canUpdateSlot || Boolean(globalMessage);
  };

  const updateInvalidScanProgressDom = (progress: LibraryInvalidScanProgress) => {
    if (!libraryState?.visible) return;
    const messageElement = root.querySelector<HTMLElement>('[data-role="library-scan-progress"]');
    if (!messageElement) return;
    const count = progress.total > 0 ? ` ${progress.processed}/${progress.total}` : '';
    const current = progress.currentItem ? ` · ${progress.currentItem}` : '';
    messageElement.textContent = `${progress.message}${count}${current}`;
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
      if (state.conversionMode === 'scan_then_convert') {
        previewModal = { previews, retryOf: null };
        pendingGlobalAction = null;
        render();
        return;
      }

      previewBusy = true;
      // Starting conversion is the next explicit operation, so the completed
      // scan snapshot no longer competes with conversion progress.
      scanProgress = null;
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
    if (!scanProgress || !['running', 'cancelling'].includes(scanProgress.status)) {
      return;
    }
    try {
      scanProgress = await services.loadScanState();
      const active = scanProgress.status === 'running' || scanProgress.status === 'cancelling';
      if (active) {
        if (!updateScanProgressDom(scanProgress)) {
          render();
        }
        scanTimer = setTimeout(() => void pollScan(), 120);
      } else {
        render();
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
    if (scanProgress && ['running', 'cancelling'].includes(scanProgress.status) || pendingGlobalAction !== null) {
      return;
    }
    pendingGlobalAction = 'start-all';
    scanPreparationCancelled = false;
    scanProgress = {
      status: 'running',
      phase: 'preparing',
      processed: 0,
      total: 0,
      current_file: '',
      message: t('scanPreparing', state.lang),
      tasks: state.slots
        .map((slot, slotIndex) => ({
          slot_index: slotIndex as SyncSlotIndex,
          phase: 'preparing' as AppScanPhase,
          processed: 0,
          total: 0,
          source_processed: 0,
          source_total: null,
          destination_processed: 0,
          destination_total: null,
          metadata_processed: 0,
          metadata_total: null,
          current_file: '',
        }))
        .filter((_, slotIndex) => state.slots[slotIndex].sourceDirectory.trim().length > 0),
    };
    neteaseMetadataCachePreparing = Boolean(services.prepareNeteaseMetadataCache);
    render();
    try {
      await desktopStateHydration;
      if (scanPreparationCancelled) {
        neteaseMetadataCachePreparing = false;
      }
      if (services.prepareNeteaseMetadataCache) {
        if (scanPreparationCancelled) {
          scanProgress = {
            ...scanProgress,
            status: 'cancelled',
            phase: 'cancelled',
            message: t('scanCancelled', state.lang),
          };
          pendingGlobalAction = null;
          render();
          return;
        }
        try {
          let cacheProgress = await services.prepareNeteaseMetadataCache();
          const applyCacheProgressToScan = (progress: NeteaseMetadataCacheProgress) => {
            if (!scanProgress || !['running', 'cancelling'].includes(scanProgress.status)) return;
            const nextTask = scanProgress.tasks?.map((task) => task.slot_index === 0
              ? {
                ...task,
                phase: 'preparing' as AppScanPhase,
                processed: progress.processed,
                total: progress.total ?? 0,
                current_file: progress.currentItem,
              }
              : task);
            scanProgress = {
              ...scanProgress,
              phase: 'preparing',
              processed: progress.processed,
              total: progress.total ?? 0,
              current_file: progress.currentItem,
              message: progress.message || t('scanPreparing', state.lang),
              tasks: nextTask,
            };
            if (!updateScanProgressDom(scanProgress)) render();
          };
          applyCacheProgressToScan(cacheProgress);
          while (
            services.loadNeteaseMetadataCacheStatus
            && (cacheProgress.status === 'building' || cacheProgress.status === 'cancelling')
          ) {
            await new Promise((resolve) => setTimeout(resolve, 100));
            cacheProgress = await services.loadNeteaseMetadataCacheStatus();
            applyCacheProgressToScan(cacheProgress);
          }
          if (cacheProgress.status === 'error' || cacheProgress.status === 'cancelled') {
            console.warn('NetEase metadata cache was not prepared:', cacheProgress.message);
          }
        } catch (error) {
          // A machine without a NetEase database is still allowed to scan
          // ordinary folders; conversion will use embedded/file-name fallback.
          console.warn('NetEase metadata cache preparation skipped:', error);
        } finally {
          neteaseMetadataCachePreparing = false;
        }
      }
      if (scanPreparationCancelled) {
        scanProgress = {
          ...scanProgress,
          status: 'cancelled',
          phase: 'cancelled',
          message: t('scanCancelled', state.lang),
        };
        pendingGlobalAction = null;
        render();
        return;
      }
      render();
      scanProgress = await services.startScan();
      render();
      if (scanProgress.status === 'running' || scanProgress.status === 'cancelling') {
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
      scanProgress = {
        ...scanProgress,
        status: 'cancelling',
        message: state.lang === 'zh' ? '正在取消扫描' : 'Cancelling scan',
      };
      render();
      if (neteaseMetadataCachePreparing) {
        scanPreparationCancelled = true;
        if (services.cancelNeteaseMetadataCache) {
          await services.cancelNeteaseMetadataCache();
        }
        scanProgress = {
          ...scanProgress,
          status: 'cancelled',
          phase: 'cancelled',
          message: t('scanCancelled', state.lang),
        };
        pendingGlobalAction = null;
        render();
        return;
      }
      const cancelSnapshot = await services.cancelScan();
      scanProgress = cancelSnapshot.status === 'running'
        ? { ...cancelSnapshot, status: 'cancelling', message: state.lang === 'zh' ? '正在取消扫描' : 'Cancelling scan' }
        : cancelSnapshot;
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
    terminateAnalysisWorker(analysisWorker, new AnalysisWorkerCancelledError());
    analysisState = {
      ...analysisState,
      status: 'cancelled',
      message: t('analysisCancelled', state.lang),
      currentItem: '',
      stage: 'cancelled',
      stageProcessed: 0,
      stageTotal: 0,
      workerJobId: '',
      resumeAvailable: resumableAnalysis !== null,
    };
    pendingGlobalAction = null;
    render();
  };

  const resumeAnalysisFlow = () => {
    const pending = resumableAnalysis;
    if (analysisState.status !== 'cancelled' || !pending || analysisRunActive) {
      if (analysisState.status === 'cancelled' && pending && analysisRunActive) {
        // The cancel button renders the resumable action immediately, while
        // the current candidate still unwinds its async I/O. Remember the
        // click and start it from the cancellation finalizer instead of
        // creating a timer that could outlive the view.
        resumeAnalysisRequested = true;
      }
      return;
    }
    resumeAnalysisRequested = false;
    analysisCancelRequested = false;
    analysisState = {
      ...analysisState,
      status: 'running',
      message: t('scanAnalyzing', state.lang),
      currentItem: '',
      stage: 'preparing',
      resumeAvailable: false,
    };
    render();
    void runPostConversionAnalysis(pending.batchId, pending.previews);
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
      await desktopStateHydration;
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

  const exportHistoryErrorReport = async (id: string) => {
    try {
      const saveFile = services.saveFile ?? ((options: SaveFileOptions) => save(options));
      const path = await saveFile({
        defaultPath: `W4DJ-error-report-${id}.txt`,
        title: state.lang === 'zh' ? '保存错误报告' : 'Save error report',
      });
      if (typeof path === 'string') {
        await services.exportHistoryErrorReport(id, path);
        await refreshHistory();
        window.alert(`${t('exportReportSuccess', state.lang)}：${path}`);
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      window.alert(`${t('exportReportFailed', state.lang)}：${message}`);
    }
  };

  const exportRuntimeSession = async (id: string) => {
    try {
      const saveFile = services.saveFile ?? ((options: SaveFileOptions) => save(options));
      const path = await saveFile({
        defaultPath: `W4DJ-runtime-session-${id}.json`,
        title: state.lang === 'zh' ? '保存运行会话记录' : 'Save runtime session',
      });
      if (typeof path === 'string') {
        await services.exportRuntimeSession(id, path);
        window.alert(`${t('exportRuntimeSuccess', state.lang)}：${path}`);
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      window.alert(`${t('exportRuntimeFailed', state.lang)}：${message}`);
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
      libraryRefreshProgress = status.refresh;
      const page = await loadLibraryPage(query);
      // The first status/page load can finish after the user has already
      // started typing into the visible search field. Preserve that newer
      // query instead of replacing it with the initial empty query.
      const currentQuery = libraryState?.query || query;
      libraryState = {
        ...libraryState,
        visible: true,
        busy: isLibraryRefreshActive(status.refresh)
          || status.invalidScan?.status === 'running'
          || status.invalidScan?.status === 'cancelling',
        status,
        page,
        query: currentQuery,
        detail: null,
        error: status.databaseWarning,
        coverData: libraryState?.coverData || {},
      };
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

  const searchLibrary = () => {
    if (!libraryState || !services.queryLibraryCatalog) return;
    const input = root.querySelector<HTMLInputElement>('input[data-action="library-search"]');
    const query = {
      ...libraryState.query,
      text: input?.value ?? libraryState.query.text,
      offset: 0,
    };
    if (librarySearchTimer) {
      clearTimeout(librarySearchTimer);
      librarySearchTimer = null;
    }
    libraryQueryRevision += 1;
    libraryState = { ...libraryState, query, error: null, contextMenu: null };
    void queryLibrary(query);
  };

  const persistAnalysisCandidate = async (
    batchId: string,
    previews: AppPreview[],
    candidate: AppPreviewCandidate,
    analysis: TrackAnalysis | null,
    failure: AppAnalysisFailure | null,
  ): Promise<DesktopState> => {
    const sourcePreview = previews.find((preview) => preview.preview.candidates.some(
      (item) => item.source_path === candidate.source_path,
    ));
    if (!sourcePreview) {
      throw new Error(`找不到分析候选所属任务槽：${candidate.source_path}`);
    }
    const singleCandidatePreview: AppPreview = {
      ...sourcePreview,
      preview: {
        ...sourcePreview.preview,
        candidates: [candidate],
      },
    };
    return services.applyTrackAnalysisResults(
      batchId,
      [singleCandidatePreview],
      analysis ? [analysis] : [],
      failure ? [failure] : [],
    );
  };

  const reanalyzeLibrary = async () => {
    if (!libraryState || libraryState.busy || analysisRunActive || !services.listLibraryAnalysisCandidates) {
      return;
    }
    libraryState = { ...libraryState, busy: true, error: null, notice: null };
    render();
    try {
      const candidates = await services.listLibraryAnalysisCandidates();
      if (candidates.length === 0) {
        libraryState = {
          ...libraryState,
          busy: false,
          notice: state.lang === 'zh' ? '当前歌曲库没有可分析的本地输出文件' : 'No readable output files are available for analysis',
        };
        render();
        return;
      }
      const preview: AppPreview = {
        slot_index: candidates.find((candidate) => candidate.slotIndex === 1)?.slotIndex ?? 0,
        mode: state.mode,
        lossless_format: state.losslessFormat,
        conflict_strategy: 'update_metadata',
        filename_rule: state.filenameRule,
        retry_of: null,
        preview: {
          source_directory: '',
          destination_directory: '',
          new_count: 0,
          existing_count: candidates.length,
          skipped_count: 0,
          error_count: 0,
          estimated_output_bytes: null,
          candidates: candidates.map((candidate) => ({
            name: candidate.name,
            source_path: candidate.path,
            destination_path: candidate.path,
            source_size_bytes: candidate.sizeBytes,
            estimated_output_bytes: null,
            operation: 'update_metadata',
          })),
          skipped: [],
          errors: [],
          warnings: [],
          available_space_bytes: null,
          disk_space_sufficient: null,
        },
      };
      const batchId = `library-analysis-${createAnalysisBatchId()}`;
      const attemptId = `attempt-${createAnalysisBatchId()}`;
      setResumableAnalysis({ batchId, previews: [preview], attemptId });
      if (services.claimAnalysisRun) {
        await services.claimAnalysisRun(batchId, attemptId);
      }
      const analysis = await analyzePreviewCandidates(
        [preview],
        batchId,
        async (candidate, result, failure) => {
          analysisState = {
            ...analysisState,
            slotIndex: preview.slot_index,
            stage: 'writingBack',
            message: state.lang === 'zh' ? '正在写回分析结果' : 'Writing analysis results',
            currentItem: candidate.name,
            stageProcessed: 0,
            stageTotal: 1,
          };
          updateAnalysisProgressDom();
          const nextState = await persistAnalysisCandidate(
            batchId,
            [preview],
            candidate,
            result,
            failure,
          );
          applyDesktopState(nextState);
          analysisState = {
            ...analysisState,
            stage: 'completed',
            stageProcessed: 1,
            stageTotal: 1,
          };
          updateAnalysisProgressDom();
        },
        attemptId,
      );
      if (analysis.cancelled) {
        if (libraryState) {
          libraryState = {
            ...libraryState,
            busy: false,
            notice: state.lang === 'zh' ? '分析已取消，可稍后继续' : 'Analysis cancelled; it can be resumed later',
          };
        }
        render();
        return;
      }
      if (analysis.analyses.length === 0 && analysis.failures.length === 0) {
        clearResumableAnalysisForBatch(batchId);
        throw new Error(state.lang === 'zh' ? '没有生成任何分析结果' : 'No analysis results were produced');
      }
      clearResumableAnalysisForBatch(batchId);
      await reloadLibraryProjection();
      if (libraryState) {
        libraryState = {
          ...libraryState,
          busy: false,
          notice: state.lang === 'zh'
            ? `已完成 ${analysis.analyses.length}/${candidates.length} 首歌曲分析并回写`
            : `Analyzed and wrote back ${analysis.analyses.length}/${candidates.length} tracks`,
        };
      }
    } catch (error) {
      if (analysisCancelRequested || error instanceof AnalysisWorkerCancelledError) {
        if (libraryState) {
          libraryState = {
            ...libraryState,
            busy: false,
            notice: state.lang === 'zh' ? '分析已取消，可稍后继续' : 'Analysis cancelled; it can be resumed later',
          };
        }
      } else if (libraryState) {
        libraryState = {
          ...libraryState,
          busy: false,
          error: error instanceof Error ? error.message : String(error),
        };
      }
      render();
    }
  };

  const findInvalidLibraryRecords = async () => {
    if (!libraryState || libraryState.busy || !services.findInvalidLibraryTracks) return;
    try {
      const progress = await services.findInvalidLibraryTracks();
      const status = libraryState.status
        ? { ...libraryState.status, invalidScan: progress }
        : null;
      libraryState = { ...libraryState, status, busy: true, error: null };
      render();
    } catch (error) {
      libraryState = {
        ...libraryState,
        busy: false,
        error: error instanceof Error ? error.message : String(error),
      };
      render();
    }
  };

  const cancelInvalidLibraryScan = async () => {
    if (!libraryState || !services.cancelInvalidLibraryScan) return;
    try {
      const progress = await services.cancelInvalidLibraryScan();
      if (libraryState) {
        libraryState = {
          ...libraryState,
          status: libraryState.status ? { ...libraryState.status, invalidScan: progress } : null,
        };
        render();
      }
    } catch (error) {
      if (libraryState) {
        libraryState = { ...libraryState, error: error instanceof Error ? error.message : String(error) };
        render();
      }
    }
  };

  const reloadLibraryProjection = async () => {
    if (!libraryState || !services.loadLibraryStatus || !services.queryLibraryCatalog) return;
    const status = await services.loadLibraryStatus();
    const page = await services.queryLibraryCatalog(libraryState.query);
    libraryRefreshProgress = status.refresh;
    libraryState = {
      ...libraryState,
      status,
      page,
      busy: isLibraryRefreshActive(status.refresh),
      error: status.databaseWarning,
      detail: null,
    };
    void loadLibraryCovers(page);
  };

  const clearInvalidLibraryRecords = async () => {
    if (!libraryState || libraryState.busy || !libraryState.confirmClearInvalid || !services.clearInvalidLibraryTracks) return;
    libraryState = { ...libraryState, busy: true, error: null, notice: null };
    render();
    try {
      const removed = await services.clearInvalidLibraryTracks();
      if (!libraryState?.visible) return;
      await reloadLibraryProjection();
      if (libraryState) {
        libraryState = {
          ...libraryState,
          busy: false,
          confirmClearInvalid: false,
          notice: state.lang === 'zh' ? `已清除 ${removed} 首失效歌曲` : `${removed} invalid track(s) removed`,
        };
      }
    } catch (error) {
      if (libraryState) {
        libraryState = { ...libraryState, busy: false, error: error instanceof Error ? error.message : String(error) };
      }
    }
    render();
  };

  const relocateLibraryTrack = async () => {
    if (!libraryState || libraryState.busy || !services.pickLibraryTrackFile || !services.relocateLibraryTrack) return;
    const context = libraryState.contextMenu;
    if (!context) return;
    const trackKey = context.trackKey;
    const currentLibraryState = libraryState;
    libraryState = { ...currentLibraryState, contextMenu: null, busy: true, error: null, notice: null };
    render();
    try {
      const path = await services.pickLibraryTrackFile();
      if (!path) {
        if (libraryState) libraryState = { ...libraryState, busy: false };
        render();
        return;
      }
      await services.relocateLibraryTrack(trackKey, path);
      if (!libraryState?.visible) return;
      await reloadLibraryProjection();
      if (libraryState) {
        libraryState = {
          ...libraryState,
          busy: false,
          notice: state.lang === 'zh' ? '文件已重新定位，原有分析结果已保留' : 'File relocated; existing analysis was preserved',
        };
      }
    } catch (error) {
      if (libraryState) {
        libraryState = { ...libraryState, busy: false, error: error instanceof Error ? error.message : String(error) };
      }
    }
    render();
  };

  const removeLibraryTrack = async () => {
    if (!libraryState || libraryState.busy || !services.removeLibraryTrack) return;
    const context = libraryState.contextMenu;
    if (!context) return;
    const trackKey = context.trackKey;
    const currentLibraryState = libraryState;
    libraryState = { ...currentLibraryState, contextMenu: null, busy: true, error: null, notice: null };
    render();
    try {
      const removed = await services.removeLibraryTrack(trackKey);
      if (!libraryState?.visible) return;
      await reloadLibraryProjection();
      if (libraryState) {
        libraryState = {
          ...libraryState,
          busy: false,
          notice: state.lang === 'zh'
            ? (removed ? '记录已从 W4DJ SQLite 移除，本地音乐和网易云数据未被删除' : '记录不存在或已经移除')
            : (removed ? 'Record removed from W4DJ SQLite; local and NetEase data were kept' : 'Record was already absent'),
        };
      }
    } catch (error) {
      if (libraryState) {
        libraryState = { ...libraryState, busy: false, error: error instanceof Error ? error.message : String(error) };
      }
    }
    render();
  };

  const handleLibraryRefreshProgress = async (progress: LibraryRefreshProgress) => {
    const currentId = libraryState?.status?.refresh.refreshId || libraryRefreshProgress?.refreshId;
    if (currentId && progress.refreshId && currentId !== progress.refreshId) return;
    libraryRefreshProgress = progress;
    const terminal = ['completed', 'cancelled', 'error'].includes(progress.status);
    if (!libraryState?.visible) {
      if (!terminal) updateLibraryRefreshProgressDom(progress);
      else render();
      return;
    }
    const nextStatus = libraryState.status ? { ...libraryState.status, refresh: progress } : null;
    libraryState = { ...libraryState, status: nextStatus, busy: isLibraryRefreshActive(progress) };
    if (!terminal) {
      updateLibraryRefreshProgressDom(progress);
      return;
    }
    if (!services.loadLibraryStatus) {
      render();
      return;
    }
    try {
      const status = await services.loadLibraryStatus();
      const page = progress.status === 'completed' && services.queryLibraryCatalog
        ? await services.queryLibraryCatalog(libraryState.query)
        : libraryState.page;
      libraryState = { ...libraryState, busy: false, status, page, error: status.databaseWarning };
      void loadLibraryCovers(page);
      render();
    } catch (error) {
      libraryState = { ...libraryState, busy: false, error: error instanceof Error ? error.message : String(error) };
      render();
    }
  };

  const handleInvalidLibraryScanProgress = async (progress: LibraryInvalidScanProgress) => {
    if (libraryState?.status?.invalidScan?.scanId
      && progress.scanId
      && libraryState.status.invalidScan.scanId !== progress.scanId) {
      return;
    }
    if (libraryState?.status) {
      libraryState = {
        ...libraryState,
        busy: progress.status === 'running' || progress.status === 'cancelling',
        status: { ...libraryState.status, invalidScan: progress },
      };
      if (!['completed', 'cancelled', 'error'].includes(progress.status)) {
        updateInvalidScanProgressDom(progress);
      }
    }
    if (!['completed', 'cancelled', 'error'].includes(progress.status)
      || !libraryState?.visible
      || !services.loadLibraryStatus
      || !services.queryLibraryCatalog) {
      return;
    }
    try {
      const status = await services.loadLibraryStatus();
      const page = await services.queryLibraryCatalog(libraryState.query);
      libraryState = {
        ...libraryState,
        busy: false,
        status,
        page,
        error: status.databaseWarning,
      };
      render();
    } catch (error) {
      if (libraryState) {
        libraryState = {
          ...libraryState,
          busy: false,
          error: error instanceof Error ? error.message : String(error),
        };
        render();
      }
    }
  };

  const clearLibraryCache = async () => {
    if (!libraryState || !services.clearLibraryCatalogCache) return;
    if (!window.confirm(state.lang === 'zh'
      ? '确定清除 W4DJ 分析歌曲库吗？音乐文件、扫描缓存和增强分析缓存不会被删除。'
      : 'Clear the W4DJ analysis library? Audio files, scan cache and analysis cache will stay intact.')) {
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
    const requestRevision = ++libraryQueryRevision;
    try {
      const page = await services.queryLibraryCatalog(query);
      if (requestRevision === libraryQueryRevision && libraryState.visible) {
        libraryState = { ...libraryState, query, page, error: null, contextMenu: null };
        void loadLibraryCovers(page);
        render();
      }
    } catch (error) {
      if (requestRevision === libraryQueryRevision) {
        libraryState = { ...libraryState, error: error instanceof Error ? error.message : String(error) };
        render();
      }
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

  const copyDjPlaylistText = async (text: string) => {
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
      djPlaylistState = djPlaylistState ? { ...djPlaylistState, notice: '已复制到剪贴板' } : null;
      render();
    } catch (error) {
      reportError(error);
    }
  };

  const sanitizedDjPlaylistName = (name: string, extension: string) => {
    const safe = name.trim().replace(/[\\/:*?"<>|\r\n]+/g, '_').replace(/\s+/g, ' ').slice(0, 80) || 'w4dj-playlist';
    return `${safe}.${extension}`;
  };

  const refreshDjPlaylistQrs = async () => {
    const current = djPlaylistState;
    if (!current?.visible || current.pages.length === 0) return;
    const revision = current.qrRevision + 1;
    djPlaylistState = {
      ...current,
      qrRevision: revision,
      qrDataUrl: null,
      qrDataUrls: current.pages.map(() => null),
    };
    render();
    try {
      const qrDataUrls = await renderDjPlaylistQrPages(current.pages);
      if (djPlaylistState?.visible && djPlaylistState.qrRevision === revision) {
        djPlaylistState = { ...djPlaylistState, qrDataUrls };
        render();
      }
    } catch (error) {
      if (djPlaylistState?.visible && djPlaylistState.qrRevision === revision) {
        djPlaylistState = { ...djPlaylistState, error: error instanceof Error ? error.message : String(error) };
        render();
      }
    }
  };

  const showDjPlaylist = (playlist: ImportedDjPlaylist, report: DjPlaylistMatchReport | null = null) => {
    const pages = splitNeteaseQrPages(playlist.tracks);
    djPlaylistState = {
      visible: true,
      launcher: false,
      busy: false,
      error: null,
      notice: null,
      playlist,
      pages,
      pageIndex: 0,
      qrDataUrl: null,
      qrRevision: 0,
      matchBusy: false,
      matchReport: report,
      exportBusy: false,
      dropActive: false,
    };
    render();
    void refreshDjPlaylistQrs();
  };

  const openDjPlaylistLauncher = () => {
    djPlaylistState = {
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
    };
    render();
  };

  const loadImportedDjPlaylistList = async () => {
    if (!services.listImportedDjPlaylists) return;
    try {
      importedDjPlaylistSummaries = await services.listImportedDjPlaylists();
      render();
    } catch (error) {
      console.warn('Failed to load imported DJ playlists:', error);
    }
  };

  const importDjPlaylistPath = async (path: string) => {
    if (!services.importW4djPlaylist) return;
    djPlaylistState = {
      visible: true,
      launcher: false,
      busy: true,
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
    };
    render();
    try {
      const playlist = await services.importW4djPlaylist(path);
      showDjPlaylist(playlist);
      void loadImportedDjPlaylistList();
    } catch (error) {
      djPlaylistState = { ...djPlaylistState!, busy: false, error: error instanceof Error ? error.message : String(error) };
      render();
    }
  };

  const importDjPlaylist = async () => {
    if (!services.pickW4djPlaylist) return;
    try {
      const path = await services.pickW4djPlaylist();
      if (path) await importDjPlaylistPath(path);
    } catch (error) {
      reportError(error);
    }
  };

  const openDjPlaylistExport = async () => {
    if (importedDjPlaylistSummaries.length === 0) {
      djPlaylistState = {
        ...(djPlaylistState || {
          visible: true,
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
        }),
        visible: true,
        launcher: true,
        exportPicker: false,
        exportChoice: false,
        notice: state.lang === 'zh' ? '请先导入 .w4dj 歌单。' : 'Import a .w4dj playlist first.',
      };
      render();
      return;
    }
    djPlaylistState = {
      visible: true,
      launcher: false,
      exportPicker: true,
      exportChoice: false,
      recentPlaylists: importedDjPlaylistSummaries,
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
    };
    render();
  };

  const selectRecentDjPlaylistForExport = async (playlistId: string) => {
    if (!services.loadImportedDjPlaylist || !services.matchImportedDjPlaylist) return;
    djPlaylistState = {
      ...(djPlaylistState || {
        visible: true,
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
      }),
      visible: true,
      launcher: false,
      exportPicker: false,
      exportChoice: false,
      busy: true,
      error: null,
      playlist: null,
      pages: [],
      qrDataUrls: [],
    };
    render();
    try {
      const playlist = await services.loadImportedDjPlaylist(playlistId);
      const report = await services.matchImportedDjPlaylist(playlistId);
      if (report.matchedCount !== report.total || report.total === 0) {
        throw new Error(`歌单仍有 ${report.total - report.matchedCount} 首歌曲无法在两个输出目录中找到`);
      }
      djPlaylistState = {
        ...djPlaylistState!,
        busy: false,
        exportChoice: true,
        playlist,
        matchReport: report,
        pages: [],
        qrDataUrl: null,
        qrDataUrls: [],
      };
      render();
    } catch (error) {
      djPlaylistState = {
        ...djPlaylistState!,
        busy: false,
        error: error instanceof Error ? error.message : String(error),
      };
      render();
    }
  };

  const exportDjPlaylistTxt = async () => {
    const playlist = djPlaylistState?.playlist;
    if (!playlist || !services.exportNeteasePlaylistText) return;
    try {
      const saveFile = services.saveFile ?? ((options: SaveFileOptions) => save(options));
      const path = await saveFile({
        defaultPath: sanitizedDjPlaylistName(playlist.name, 'txt'),
        title: state.lang === 'zh' ? '导出网易云歌单 TXT' : 'Export NetEase playlist TXT',
      });
      if (typeof path === 'string') {
        await services.exportNeteasePlaylistText(path, buildNeteaseImportText(playlist.tracks));
        djPlaylistState = { ...djPlaylistState!, notice: `已导出：${path}` };
        render();
      }
    } catch (error) {
      djPlaylistState = { ...djPlaylistState!, error: error instanceof Error ? error.message : String(error) };
      render();
    }
  };

  const exportDjPlaylistW4dj = async () => {
    const playlist = djPlaylistState?.playlist;
    if (!playlist || !services.exportImportedDjPlaylistW4dj) return;
    try {
      const saveFile = services.saveFile ?? ((options: SaveFileOptions) => save(options));
      const path = await saveFile({
        defaultPath: sanitizedDjPlaylistName(playlist.name, 'w4dj'),
        title: state.lang === 'zh' ? '导出 W4DJ v2 歌单' : 'Export W4DJ v2 playlist',
      });
      if (typeof path !== 'string') return;
      await services.exportImportedDjPlaylistW4dj(playlist.playlistId, path);
      djPlaylistState = { ...djPlaylistState!, notice: `已导出：${path}` };
      render();
    } catch (error) {
      djPlaylistState = { ...djPlaylistState!, error: error instanceof Error ? error.message : String(error) };
      render();
    }
  };

  const exportDjPlaylistM3u8 = async (allowPartial: boolean, copyAudio = false) => {
    const playlist = djPlaylistState?.playlist;
    if (!playlist || !services.exportImportedDjPlaylistM3u8 || djPlaylistExportInFlight) return;
    if (allowPartial && djPlaylistState?.matchReport) {
      const omitted = djPlaylistState.matchReport.total - djPlaylistState.matchReport.matchedCount;
      if (omitted > 0 && !window.confirm(djPlaylistText('djPlaylistPartialConfirm', state.lang, { count: omitted }))) return;
    }
    djPlaylistExportInFlight = true;
    djPlaylistState = { ...djPlaylistState!, exportBusy: true, error: null };
    render();
    try {
      const saveFile = services.saveFile ?? ((options: SaveFileOptions) => save(options));
      const path = await saveFile({
        defaultPath: sanitizedDjPlaylistName(playlist.name, 'm3u8'),
        title: t('djPlaylistExportButton', state.lang),
      });
      if (typeof path !== 'string') return;
      const result = await services.exportImportedDjPlaylistM3u8(playlist.playlistId, path, allowPartial, copyAudio);
      if (copyAudio && (!result.copyAudio || !result.portable || result.copiedCount !== result.matchedCount)) {
        throw new Error(t('djPlaylistExportPortableError', state.lang));
      }
      const exportDetail = copyAudio
        ? djPlaylistText('djPlaylistExportCopied', state.lang, {
          copied: result.copiedCount,
          matched: result.matchedCount,
        })
        : t('djPlaylistExportReferenced', state.lang);
      djPlaylistState = {
        ...djPlaylistState!,
        exportBusy: false,
        exportChoice: false,
        notice: `${exportDetail}\n${t('djPlaylistExportSuccess', state.lang)}：${result.path}`,
      };
      render();
    } catch (error) {
      djPlaylistState = { ...djPlaylistState!, error: error instanceof Error ? error.message : String(error) };
      render();
    } finally {
      djPlaylistExportInFlight = false;
      if (djPlaylistState?.exportBusy) {
        djPlaylistState = { ...djPlaylistState, exportBusy: false };
        render();
      }
    }
  };

  const matchDjPlaylist = async () => {
    const playlist = djPlaylistState?.playlist;
    if (!playlist || !services.matchImportedDjPlaylist) return;
    djPlaylistState = { ...djPlaylistState!, matchBusy: true, error: null };
    render();
    try {
      const report = await services.matchImportedDjPlaylist(playlist.playlistId);
      djPlaylistState = { ...djPlaylistState!, matchBusy: false, matchReport: report };
      render();
      if (report.matchedCount === report.total && report.total > 0) void exportDjPlaylistM3u8(false);
    } catch (error) {
      djPlaylistState = { ...djPlaylistState!, matchBusy: false, error: error instanceof Error ? error.message : String(error) };
      render();
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

  const scanLocalNeteaseFolder = async () => {
    if (neteaseDiscoveryInFlight) return;

    const finishDiscovery = (progress: NeteaseDiscoveryProgress) => {
      neteaseDiscoveryProgress = progress;
      render();
      if (progress.status === 'completed') {
        setTimeout(() => {
          if (neteaseDiscoveryProgress === progress) {
            neteaseDiscoveryProgress = null;
            render();
          }
        }, 1800);
      }
    };

    if (neteaseDiscoveryProgress?.status === 'error') {
      neteaseDiscoveryInFlight = true;
      render();
      try {
        await desktopStateHydration;
        const path = await services.pickSource(0);
        if (!path) {
          return;
        }
        applyDesktopState(await services.selectSourceDirectory(0, path));
        finishDiscovery({
          status: 'completed',
          stage: 'checkingMusicFolder',
          processed: 0,
          total: null,
          currentItem: '',
          message: t('scanLocalNeteaseSelected', state.lang),
          suggestion: null,
          error: null,
        });
      } catch (error) {
        const detail = error instanceof Error ? error.message : String(error);
        neteaseDiscoveryProgress = {
          status: 'error',
          stage: 'checkingMusicFolder',
          processed: 0,
          total: null,
          currentItem: '',
          message: t('scanLocalNeteaseFallback', state.lang),
          suggestion: null,
          error: detail,
        };
        render();
      } finally {
        neteaseDiscoveryInFlight = false;
        render();
      }
      return;
    }

    neteaseDiscoveryInFlight = true;
    neteaseDiscoveryProgress = {
      status: 'running',
      stage: 'locatingDatabase',
      processed: 0,
      total: null,
      currentItem: '',
      message: t('scanLocalNeteaseRunning', state.lang),
      suggestion: null,
      error: null,
    };
    render();
    try {
      await desktopStateHydration;
      const discovery = services.locateNeteaseLibrary
        ? await services.locateNeteaseLibrary(true)
        : null;
      const path = discovery?.musicFolder?.trim();
      if (!path) {
        const message = t('scanLocalNeteaseNotFound', state.lang);
        finishDiscovery({
          status: 'error',
          stage: 'checkingMusicFolder',
          processed: discovery?.localFileCount || 0,
          total: discovery?.localFileCount || null,
          currentItem: '',
          message,
          suggestion: discovery,
          error: message,
        });
        return;
      }
      applyDesktopState(await services.selectSourceDirectory(0, path));
      if ((discovery?.localFileCount || 0) > 0) {
        finishDiscovery({
          status: 'completed',
          stage: 'checkingMusicFolder',
          processed: discovery?.localFileCount || 0,
          total: discovery?.localFileCount || null,
          currentItem: '',
          message: `${t('scanLocalNeteaseSelected', state.lang)} ${discovery?.localFileCount || 0}`,
          suggestion: discovery,
          error: null,
        });
      } else {
        neteaseDiscoveryProgress = {
          status: 'running',
          stage: 'checkingMusicFolder',
          processed: 0,
          total: null,
          currentItem: '',
          message: t('scanLocalNeteaseSelected', state.lang),
          suggestion: discovery,
          error: null,
        };
        render();
      }
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      const message = t('scanLocalNeteaseNotFound', state.lang);
      neteaseDiscoveryProgress = {
        status: 'error',
        stage: 'locatingDatabase',
        processed: 0,
        total: null,
        currentItem: '',
        message,
        suggestion: null,
        error: `${message}：${detail}`,
      };
      render();
    } finally {
      neteaseDiscoveryInFlight = false;
      render();
    }
  };

  const selectNeteaseMetadataDatabase = async () => {
    if (
      neteaseMetadataDatabase.busy
      || !services.pickNeteaseDatabase
      || !services.selectNeteaseMetadataDatabase
    ) {
      return;
    }
    const path = await services.pickNeteaseDatabase();
    if (!path) {
      return;
    }
    neteaseMetadataDatabase = {
      ...neteaseMetadataDatabase,
      busy: true,
      message: null,
      error: null,
    };
    render();
    try {
      const status = await services.selectNeteaseMetadataDatabase(path);
      neteaseMetadataDatabase = {
        status,
        busy: false,
        message: t('neteaseDatabaseSelected', state.lang),
        error: null,
      };
    } catch (error) {
      neteaseMetadataDatabase = {
        ...neteaseMetadataDatabase,
        busy: false,
        error: error instanceof Error ? error.message : String(error),
      };
    }
    render();
  };

  const clearNeteaseMetadataDatabase = async () => {
    if (
      neteaseMetadataDatabase.busy
      || !services.clearNeteaseMetadataDatabase
    ) {
      return;
    }
    neteaseMetadataDatabase = {
      ...neteaseMetadataDatabase,
      busy: true,
      message: null,
      error: null,
    };
    render();
    try {
      const status = await services.clearNeteaseMetadataDatabase();
      neteaseMetadataDatabase = {
        status,
        busy: false,
        message: null,
        error: null,
      };
    } catch (error) {
      neteaseMetadataDatabase = {
        ...neteaseMetadataDatabase,
        busy: false,
        error: error instanceof Error ? error.message : String(error),
      };
    }
    render();
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
    sessionId?: string,
    onCandidateResult?: PersistAnalysisCandidate,
    attemptId?: string,
  ): Promise<{ analyses: TrackAnalysis[]; failures: AppAnalysisFailure[]; cancelled: boolean }> => {
    analysisCancelRequested = false;
    const recordSessionEvent = (
      event: string,
      details: Record<string, unknown> = {},
    ): Promise<void> => {
      if (!sessionId || !services.recordRuntimeSessionEvent) {
        return Promise.resolve();
      }
      return services.recordRuntimeSessionEvent(sessionId, event, details)
        .catch((error) => console.warn(`Failed to record runtime session event ${event}:`, error));
    };
    const groups = new Map<SyncSlotIndex, AppPreviewCandidate[]>();
    const seenSourcePaths = new Set<string>();
    for (const preview of previews) {
      const group = groups.get(preview.slot_index) || [];
      for (const candidate of preview.preview.candidates) {
        if (seenSourcePaths.has(candidate.source_path)) {
          continue;
        }
        seenSourcePaths.add(candidate.source_path);
        group.push(candidate);
      }
      if (group.length > 0) {
        groups.set(preview.slot_index, group);
      }
    }
    const candidates = Array.from(groups.values()).flat();
    if (candidates.length === 0) {
      analysisRunActive = false;
      terminateAnalysisWorker(analysisWorker);
      analysisState = {
        ...defaultAnalysisState,
        status: 'completed',
        message: t('analysisNoResults', state.lang),
      };
      render();
      return { analyses: [], failures: [], cancelled: false };
    }
    // Prepare the first cancellation handle before any asynchronous cache or
    // model work. This keeps the Cancel button effective even while startup
    // awaits those services; the same Worker is consumed by the first song.
    let preallocatedWorker: AnalysisWorkerSession | null = services.createAnalysisWorker?.()
      ?? new AnalysisWorkerClient();
    let preallocatedWorkerJobId = `analysis-${createAnalysisBatchId()}`;
    analysisWorker = preallocatedWorker;
    await recordSessionEvent('analysis_started', {
      candidate_count: candidates.length,
      slot_count: groups.size,
      enhanced_mode: state.enhancedMode,
      attempt_id: attemptId,
    });
    analysisRunActive = true;
    const firstGroup = groups.entries().next().value as [SyncSlotIndex, AppPreviewCandidate[]];

    analysisState = {
      slotIndex: firstGroup[0],
      status: 'running',
      completed: 0,
      total: firstGroup[1].length,
      resultCount: 0,
      failedCount: 0,
      message: t('scanAnalyzing', state.lang),
      currentItem: '',
      stage: 'preparing',
      resumeAvailable: false,
    };
    render();

    const results: TrackAnalysis[] = [];
    const freshResults: TrackAnalysis[] = [];
    const failures: AppAnalysisFailure[] = [];
    let failedCount = 0;
    const analysisCacheRevisionAtStart = analysisCacheRevision;
    const persistFreshResults = async (entries: TrackAnalysis[] = freshResults) => {
      if (entries.length === 0 || analysisCacheRevisionAtStart !== analysisCacheRevision) {
        return;
      }
      try {
        await services.saveTrackAnalyses(entries);
        if (analysisCacheRevisionAtStart === analysisCacheRevision) {
          mergeAnalysisCache(entries);
        }
      } catch (error) {
        console.warn('Failed to save Essentia analysis cache:', error);
      }
    };
    const finishCancelledAnalysis = async () => {
      await persistFreshResults();
      terminateAnalysisWorker(analysisWorker, new AnalysisWorkerCancelledError());
      await recordSessionEvent('analysis_cancelled', {
        completed_count: results.length,
        failed_count: failedCount,
        candidate_count: candidates.length,
      });
      analysisState = {
        ...analysisState,
        status: 'cancelled',
        resultCount: results.length,
        failedCount,
        message: t('analysisCancelled', state.lang),
        currentItem: '',
        stage: 'cancelled',
        stageProcessed: 0,
        stageTotal: 0,
        workerJobId: '',
        resumeAvailable: resumableAnalysis !== null,
      };
      // Make the resumable action usable as soon as the cancelled state is
      // rendered; the outer finally also performs this assignment for normal
      // exits, but the button can be clicked before that microtask runs.
      analysisRunActive = false;
      render();
      if (resumeAnalysisRequested) {
        setTimeout(() => resumeAnalysisFlow(), 0);
      }
      return { analyses: results, failures, cancelled: true };
    };
    try {
      await analysisCacheLoadPromise;
      const cacheByPath = new Map(analysisCache.map((entry) => [entry.path, entry]));
      let highLevelModels: EssentiaModelFile[] | undefined;
      if (state.enhancedMode && services.loadEssentiaModel) {
        // Startup deliberately leaves the model directory untouched. Ensure
        // and inspect it only after the user has requested enhanced analysis;
        // test doubles and older integrations without the new command retain
        // the previous status-query fallback.
        if (services.ensureEssentiaModels) {
          recordSessionEvent('analysis_models_initializing', {
            model_ids: ESSENTIA_MODEL_IDS,
            model_version: modelStatus.version,
          });
          try {
            modelStatus = await services.ensureEssentiaModels();
            render();
          } catch (error) {
            recordSessionEvent('analysis_models_unavailable', {
              model_version: modelStatus.version,
              reason: 'model_initialization_failed',
              error: error instanceof Error ? error.message : String(error),
            });
          }
        } else if (!modelStatus.embedding && services.getEssentiaModelStatus) {
          try {
            modelStatus = await services.getEssentiaModelStatus();
          } catch (error) {
            recordSessionEvent('analysis_models_unavailable', {
              model_version: modelStatus.version,
              reason: 'model_status_unavailable',
              error: error instanceof Error ? error.message : String(error),
            });
          }
        }

        if (modelStatus.embedding) {
          recordSessionEvent('analysis_models_loading', {
            model_ids: ESSENTIA_MODEL_IDS,
            model_version: modelStatus.version,
          });
          const loadedModels: EssentiaModelFile[] = [];
          const missingModels: string[] = [];
          for (const id of ESSENTIA_MODEL_IDS) {
            try {
              const model = await services.loadEssentiaModel(id);
              loadedModels.push(model);
            } catch (error) {
              missingModels.push(id);
              recordSessionEvent('analysis_model_unavailable', {
                model_id: id,
                model_version: modelStatus.version,
                error: error instanceof Error ? error.message : String(error),
              });
            }
          }
          highLevelModels = loadedModels;
          recordSessionEvent('analysis_models_loaded', {
            model_ids: loadedModels.map((model) => model.id),
            missing_model_ids: missingModels,
            model_version: modelStatus.version,
          });
        }
      } else if (state.enhancedMode) {
        recordSessionEvent('analysis_models_unavailable', {
          model_version: modelStatus.version,
          reason: 'required_models_missing',
          embedding: modelStatus.embedding,
          genre: modelStatus.genre,
          mood: modelStatus.mood,
          instrument: modelStatus.instrument,
          emotion_continuous: modelStatus.emotionContinuous,
          emotion_cluster: modelStatus.emotionCluster,
        });
      }

      if (analysisCancelRequested) {
        return finishCancelledAnalysis();
      }

      for (const [slotIndex, group] of groups) {
        let groupFailedCount = 0;
        let groupResultCount = 0;
        analysisState = {
          ...analysisState,
          slotIndex,
          completed: 0,
          total: group.length,
          resultCount: 0,
          failedCount: 0,
          currentItem: '',
          stage: 'preparing',
          stageProcessed: 0,
          stageTotal: 0,
          workerJobId: '',
          message: t('scanAnalyzing', state.lang),
        };
        render();

        for (const candidate of group) {
          if (analysisCancelRequested) {
            return finishCancelledAnalysis();
          }
          const candidateWorker: AnalysisWorkerSession = preallocatedWorker
            ?? (services.createAnalysisWorker?.() ?? new AnalysisWorkerClient());
          const workerJobId = preallocatedWorkerJobId || `analysis-${createAnalysisBatchId()}`;
          const candidateStartedAt = Date.now();
          preallocatedWorker = null;
          preallocatedWorkerJobId = '';
          analysisWorker = candidateWorker;
          await recordSessionEvent('analysis_candidate_started', {
            slot_index: slotIndex,
            name: candidate.name,
            source_path: candidate.source_path,
            destination_path: candidate.destination_path,
            worker_job_id: workerJobId,
          });
          analysisState = {
            ...analysisState,
            currentItem: candidate.name,
            stage: 'preparing',
            stageProcessed: 0,
            stageTotal: 0,
            workerJobId,
            message: t('scanAnalyzing', state.lang),
          };
          updateAnalysisProgressDom();
          let fingerprint: AppAudioFileFingerprint | null = null;
          try {
            fingerprint = await services.getAudioFileFingerprint(candidate.source_path);
          } catch (error) {
            console.warn(`Failed to fingerprint ${candidate.source_path}; reanalyzing it:`, error);
          }

          const cached = cacheByPath.get(candidate.source_path);
          const highLevelModelsAvailable = Boolean(
            state.enhancedMode
            && highLevelModels?.some((model) => model.id === 'musicnn_embedding')
            && modelStatus.version,
          );
            const canReuse = fingerprint !== null
              && canReuseTrackAnalysis(
                cached,
                fingerprint,
                state.neteaseFilenameFormat,
                modelStatus.version || null,
                highLevelModelsAvailable,
                state.enhancedMode,
              );

          if (canReuse && cached) {
            if (onCandidateResult) {
              await onCandidateResult(candidate, cached, null);
            }
            results.push(cached);
            groupResultCount += 1;
            await recordSessionEvent('analysis_candidate_finished', {
              slot_index: slotIndex,
              name: candidate.name,
              source_path: candidate.source_path,
              destination_path: candidate.destination_path,
              status: 'completed',
              cached: true,
              worker_job_id: workerJobId,
              elapsed_ms: 0,
            });
            await recordSessionEvent('analysis_worker_terminated', {
              slot_index: slotIndex,
              name: candidate.name,
              source_path: candidate.source_path,
              destination_path: candidate.destination_path,
              worker_job_id: workerJobId,
              cached: true,
            });
            terminateAnalysisWorker(candidateWorker);
          } else {
            let lastProgressEventAt = 0;
            let lastProgressStage = '';
            try {
              // A Worker belongs to exactly one uncached song. Keeping this
              // lifetime narrow prevents Essentia/WASM state from surviving
              // into the next song and also makes cancellation local to the
              // current candidate.
              analysisState = {
                ...analysisState,
                workerJobId,
                startedAt: new Date().toISOString(),
              };
              await recordSessionEvent('analysis_worker_starting', {
                slot_index: slotIndex,
                name: candidate.name,
                source_path: candidate.source_path,
                destination_path: candidate.destination_path,
                worker_job_id: workerJobId,
              });
              await candidateWorker.start(workerJobId, highLevelModels ?? []);
              await recordSessionEvent('analysis_worker_started', {
                slot_index: slotIndex,
                name: candidate.name,
                source_path: candidate.source_path,
                destination_path: candidate.destination_path,
                worker_job_id: workerJobId,
                high_level_models: Boolean(highLevelModels),
              });
              const bytes = await services.readAudioFile(candidate.source_path);
              if (analysisCancelRequested) {
                return finishCancelledAnalysis();
              }
              let metadata: TrackMetadata | undefined;
              try {
                metadata = await services.readTrackMetadata(candidate.source_path);
              } catch {
                // Analysis can continue using the filename identity.
              }
              if (analysisCancelRequested) {
                return finishCancelledAnalysis();
              }
              const analysis = await analyzeAudioFile(
                candidate.source_path,
                Uint8Array.from(bytes),
                metadata,
                {
                  fingerprint: fingerprint || undefined,
                  neteaseFilenameFormat: state.neteaseFilenameFormat,
                  highLevelModels,
                  workerClient: candidateWorker,
                  workerJobId,
                  onProgress: (progress) => {
                    analysisState = {
                      ...analysisState,
                      stage: progress.stage,
                      message: progress.message,
                      currentItem: candidate.name,
                      stageProcessed: progress.processed,
                      stageTotal: progress.total,
                      workerJobId,
                    };
                    updateAnalysisProgressDom();
                    const now = Date.now();
                    const stageChanged = progress.stage !== lastProgressStage;
                    if (stageChanged || now - lastProgressEventAt >= 1000) {
                      lastProgressEventAt = now;
                      lastProgressStage = progress.stage;
                      void recordSessionEvent('analysis_candidate_progress', {
                        slot_index: slotIndex,
                        name: candidate.name,
                        source_path: candidate.source_path,
                        destination_path: candidate.destination_path,
                        worker_job_id: workerJobId,
                        stage: progress.stage,
                        model_id: progress.modelId,
                        processed: progress.processed,
                        total: progress.total,
                        message: progress.message,
                        stage_started_at: progress.stageStartedAt,
                        elapsed_ms: progress.elapsedMs,
                        backend: progress.backend,
                        patch_count: progress.patchCount,
                        tf_memory: progress.tfMemory,
                      });
                    }
                  },
                },
              );
              const completeness = state.enhancedMode
                ? assessTrackAnalysisCompleteness(analysis)
                : {
                  complete: isBasicTrackAnalysisComplete(analysis),
                  reasons: isBasicTrackAnalysisComplete(analysis) ? [] : ['基础分析未完成'],
                  discogsCompletedHeads: 0,
                  discogsTotalHeads: 0,
                };
              if (completeness.complete) {
                if (onCandidateResult) {
                  await onCandidateResult(candidate, analysis, null);
                }
                results.push(analysis);
                groupResultCount += 1;
              } else {
                const failure: AppAnalysisFailure = {
                  path: candidate.source_path,
                  message: completeness.reasons.join('；') || '分析未满足完整性要求',
                  status: 'failed',
                  stage: 'analysis-completion',
                  elapsedMs: Date.now() - candidateStartedAt,
                };
                failures.push(failure);
                failedCount += 1;
                groupFailedCount += 1;
                // Persist partial basic values and successful individual
                // heads together with the terminal failure. The Rust side
                // keeps the existing successful head projections intact.
                if (onCandidateResult) {
                  await onCandidateResult(candidate, analysis, failure);
                }
                await recordSessionEvent('analysis_completion_rejected', {
                  slot_index: slotIndex,
                  name: candidate.name,
                  source_path: candidate.source_path,
                  destination_path: candidate.destination_path,
                  worker_job_id: workerJobId,
                  reasons: completeness.reasons,
                  discogs_completed_heads: completeness.discogsCompletedHeads,
                  discogs_total_heads: completeness.discogsTotalHeads,
                });
              }
              freshResults.push(analysis);
              await persistFreshResults([analysis]);
              await recordSessionEvent('analysis_candidate_finished', {
                slot_index: slotIndex,
                name: candidate.name,
                source_path: candidate.source_path,
                destination_path: candidate.destination_path,
                status: completeness.complete ? 'completed' : 'failed',
                cached: false,
                worker_job_id: workerJobId,
                stage: completeness.complete ? 'completed' : 'analysis-completion',
                elapsed_ms: Date.now() - candidateStartedAt,
              });
            } catch (error) {
              if (analysisCancelRequested || error instanceof AnalysisWorkerCancelledError) {
                return finishCancelledAnalysis();
              }
              failedCount += 1;
              groupFailedCount += 1;
              const failure: AppAnalysisFailure = {
                path: candidate.source_path,
                message: error instanceof Error ? error.message : String(error),
              };
              if (error instanceof AnalysisWorkerTimeoutError) {
                failure.status = 'timeout';
                failure.stage = error.stage;
                failure.elapsedMs = error.elapsedMs;
              }
              failures.push(failure);
              if (onCandidateResult) {
                try {
                  await onCandidateResult(candidate, null, failure);
                } catch (persistenceError) {
                  console.warn(`Failed to persist Essentia analysis failure for ${candidate.source_path}:`, persistenceError);
                }
              }
              await recordSessionEvent('analysis_candidate_finished', {
                slot_index: slotIndex,
                name: candidate.name,
                source_path: candidate.source_path,
                status: failure.status ?? 'failed',
                cached: false,
                error: error instanceof Error ? error.message : String(error),
                worker_job_id: workerJobId,
                stage: failure.stage,
                elapsed_ms: failure.elapsedMs ?? Date.now() - candidateStartedAt,
              });
              console.warn(`Essentia analysis failed for ${candidate.source_path}`, error);
            } finally {
              if (candidateWorker) {
                await recordSessionEvent('analysis_worker_terminated', {
                  slot_index: slotIndex,
                  name: candidate.name,
                  source_path: candidate.source_path,
                  destination_path: candidate.destination_path,
                  worker_job_id: workerJobId,
                  elapsed_ms: Date.now() - candidateStartedAt,
                });
                terminateAnalysisWorker(candidateWorker);
              }
            }
          }
          if (analysisCancelRequested) {
            return finishCancelledAnalysis();
          }
          analysisState = {
            ...analysisState,
            completed: analysisState.completed + 1,
            resultCount: groupResultCount,
            failedCount: groupFailedCount,
            currentItem: candidate.name,
            stage: 'completed',
            stageProcessed: 0,
            stageTotal: 0,
            workerJobId: '',
            message: t('scanAnalyzing', state.lang),
          };
          updateAnalysisProgressDom();
          await yieldToUi();
        }
      }

      analysisState = {
        ...analysisState,
        slotIndex: null,
        status: results.length > 0 ? 'completed' : 'error',
        resultCount: results.length,
        failedCount,
        currentItem: '',
        stage: results.length > 0 ? 'completed' : 'error',
        stageProcessed: 0,
        stageTotal: 0,
        workerJobId: '',
        resumeAvailable: false,
        message: failedCount > 0
          ? t('analysisPartial', state.lang)
            .replace('{done}', String(results.length))
            .replace('{total}', String(candidates.length))
            .replace('{failed}', String(failedCount))
          : t('analysisComplete', state.lang).replace('{count}', String(results.length)),
      };
      await persistFreshResults();
      await recordSessionEvent('analysis_completed', {
        result_count: results.length,
        failure_count: failures.length,
        candidate_count: candidates.length,
      });
      render();
      return { analyses: results, failures, cancelled: false };
    } catch (error) {
      if (!analysisCancelRequested && !(error instanceof AnalysisWorkerCancelledError)) {
        await recordSessionEvent('analysis_error', {
          message: error instanceof Error ? error.message : String(error),
        });
      }
      throw error;
    } finally {
      analysisRunActive = false;
      terminateAnalysisWorker(analysisWorker);
    }
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
    const attemptId = `attempt-${createAnalysisBatchId()}`;
    setResumableAnalysis({ batchId, previews, attemptId });
    const recordSessionEvent = (
      event: string,
      details: Record<string, unknown> = {},
    ): Promise<void> => {
      if (!services.recordRuntimeSessionEvent) {
        return Promise.resolve();
      }
      return services.recordRuntimeSessionEvent(batchId, event, details)
        .catch((error) => console.warn(`Failed to record runtime session event ${event}:`, error));
    };
    const finalizeSession = async () => {
      if (!services.finalizeAnalysisSession) {
        return;
      }
      try {
        await services.finalizeAnalysisSession(batchId);
      } catch (error) {
        console.warn('Failed to finalize runtime analysis session:', error);
      }
    };
    await recordSessionEvent('analysis_requested', {
      candidate_count: previews.reduce(
        (total, preview) => total + preview.preview.candidates.length,
        0,
      ),
      enhanced_mode: state.enhancedMode,
      attempt_id: attemptId,
    });
    try {
      const conversionReady = await waitForConversionBatch(previews);
      if (!conversionReady) {
        await recordSessionEvent('analysis_cancelled', { reason: 'conversion_wait_cancelled' });
        await finalizeSession();
        analysisState = {
          ...analysisState,
          status: 'cancelled',
          message: t('analysisCancelled', state.lang),
          currentItem: '',
          stage: 'cancelled',
          resumeAvailable: resumableAnalysis !== null,
        };
        render();
        return;
      }
      await recordSessionEvent('analysis_conversion_ready');
      if (services.claimAnalysisRun) {
        await services.claimAnalysisRun(batchId, attemptId);
      }
      const analysis = await analyzePreviewCandidates(
        previews,
        batchId,
        async (candidate, result, failure) => {
          // Keep writeback visible as its own post-conversion phase.  The
          // worker has already completed here; exposing this transition makes
          // it clear that the output-tag transaction, not model inference, is
          // the current operation.
          const sourcePreview = previews.find((preview) => preview.preview.candidates.some(
            (item) => item.source_path === candidate.source_path,
          ));
          analysisState = {
            ...analysisState,
            slotIndex: sourcePreview?.slot_index ?? analysisState.slotIndex,
            stage: 'writingBack',
            message: state.lang === 'zh' ? '正在写回分析结果' : 'Writing analysis results',
            currentItem: candidate.name,
            stageProcessed: 0,
            stageTotal: 1,
          };
          updateAnalysisProgressDom();
          const nextState = await persistAnalysisCandidate(
            batchId,
            previews,
            candidate,
            result,
            failure,
          );
          applyDesktopState(nextState);
          await recordSessionEvent('analysis_candidate_persisted', {
            source_path: candidate.source_path,
            destination_path: candidate.destination_path,
            status: failure?.status ?? (failure ? 'failed' : 'completed'),
            stage: failure?.stage,
            elapsed_ms: failure?.elapsedMs,
          });
          analysisState = {
            ...analysisState,
            stage: 'completed',
            stageProcessed: 1,
            stageTotal: 1,
          };
          updateAnalysisProgressDom();
        },
        attemptId,
      );
      if (analysis.cancelled) {
        await finalizeSession();
        return;
      }
      if (analysis.analyses.length === 0 && analysis.failures.length === 0) {
        await recordSessionEvent('analysis_completed', {
          result_count: 0,
          failure_count: 0,
          persisted: false,
          reason: 'no_results',
        });
        await finalizeSession();
        clearResumableAnalysisForBatch(batchId);
        analysisState = {
          ...analysisState,
          slotIndex: null,
          status: 'completed',
          message: t('analysisNoResults', state.lang),
          currentItem: '',
          stage: 'completed',
          resumeAvailable: false,
        };
        render();
        return;
      }
      await recordSessionEvent('analysis_results_ready', {
        result_count: analysis.analyses.length,
        failure_count: analysis.failures.length,
      });
      await recordSessionEvent('analysis_persisted', {
        result_count: analysis.analyses.length,
        failure_count: analysis.failures.length,
        persistence: 'per_candidate',
      });
      await refreshHistory(false);
      await finalizeSession();
      clearResumableAnalysisForBatch(batchId);
      analysisState = { ...analysisState, resumeAvailable: false };
    } catch (error) {
      if (analysisCancelRequested || error instanceof AnalysisWorkerCancelledError) {
        await recordSessionEvent('analysis_cancelled', {
          reason: 'worker_cancelled',
          message: error instanceof Error ? error.message : String(error),
        });
        await finalizeSession();
        analysisState = {
          ...analysisState,
          status: 'cancelled',
          message: t('analysisCancelled', state.lang),
          currentItem: '',
          stage: 'cancelled',
          resumeAvailable: resumableAnalysis !== null,
        };
        render();
        return;
      }
      await recordSessionEvent('analysis_error', {
        message: error instanceof Error ? error.message : String(error),
      });
      await finalizeSession();
      analysisState = {
        ...analysisState,
        slotIndex: null,
        status: 'error',
        message: error instanceof Error ? error.message : String(error),
        resumeAvailable: resumableAnalysis !== null,
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

  // The native WebView menu exposes a Reload action that recreates the page and
  // can interrupt the first-use flow. Reloading the app is intentionally not a
  // product action; the explicit NetEase scan button owns that workflow.
  root.addEventListener('contextmenu', (event) => {
    const target = event.target as HTMLElement | null;
    const row = target?.closest<HTMLElement>('[data-action="library-track-detail"]');
    const trackKey = row?.dataset.trackKey;
    if (row && trackKey && libraryState?.visible && !libraryState.busy) {
      const pointer = event as MouseEvent;
      event.preventDefault();
      libraryState = {
        ...libraryState,
        contextMenu: { trackKey, x: pointer.clientX, y: pointer.clientY },
      };
      render();
      return;
    }
    event.preventDefault();
  });

  // Native WebView reload/close can happen without a DOM click. Persist a
  // best-effort unloading marker and ask for confirmation while a song is in
  // flight; recovery remains based on the durable state file, not this event.
  window.addEventListener('beforeunload', (event) => {
    if (!analysisRunActive || !resumableAnalysis || !services.recordRuntimeSessionEvent) {
      return;
    }
    event.preventDefault();
    event.returnValue = '';
    void services.recordRuntimeSessionEvent(
      resumableAnalysis.batchId,
      'analysis_renderer_unloading',
      { attempt_id: resumableAnalysis.attemptId, reason: 'window_unload' },
    );
  });

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

  root.addEventListener('pointerdown', (event) => {
    const resizer = (event.target as HTMLElement | null)?.closest<HTMLElement>('[data-library-column-resizer]');
    if (!resizer) return;
    const header = resizer.closest<HTMLElement>('[data-library-column-header]');
    const id = resizer.dataset.libraryColumnResizer;
    if (!header || !id) return;
    resizingLibraryColumn = {
      id,
      startX: event.clientX,
      startWidth: header.getBoundingClientRect().width,
      width: header.getBoundingClientRect().width,
      header,
    };
    header.draggable = false;
    resizer.setPointerCapture?.(event.pointerId);
    event.preventDefault();
    event.stopPropagation();
  });

  root.addEventListener('pointermove', (event) => {
    if (!resizingLibraryColumn) return;
    const header = Array.from(root.querySelectorAll<HTMLElement>('[data-library-column-header]'))
      .find((candidate) => candidate.dataset.libraryColumnHeader === resizingLibraryColumn?.id);
    if (header) {
      const width = Math.max(72, Math.min(420, resizingLibraryColumn.startWidth + event.clientX - resizingLibraryColumn.startX));
      resizingLibraryColumn.width = width;
      header.style.width = `${width}px`;
    }
  });

  const finishLibraryColumnResize = () => {
    if (!resizingLibraryColumn) return;
    saveLibraryColumnWidth(resizingLibraryColumn.id, resizingLibraryColumn.width);
    resizingLibraryColumn.header.draggable = true;
    resizingLibraryColumn = null;
  };

  root.addEventListener('pointerup', finishLibraryColumnResize);
  root.addEventListener('pointercancel', finishLibraryColumnResize);

  root.addEventListener('click', (event) => {
    const target = event.target as HTMLElement | null;
    const contextAction = target?.closest<HTMLElement>('[data-action="relocate-library-track"], [data-action="remove-library-track"]');
    const clearedContextMenu = Boolean(libraryState?.contextMenu && !contextAction);
    if (clearedContextMenu && libraryState) {
      libraryState = { ...libraryState, contextMenu: null };
    }
    const modal = target?.closest('.about-modal');
    const libraryModal = target?.closest('.library-modal');
    const djPlaylistModal = target?.closest('.dj-playlist-modal');
    const dialog = target?.closest('.about-dialog, .help-dialog, .library-dialog, .dj-playlist-dialog');
    if (libraryModal && !dialog) {
      libraryState = null;
      render();
      return;
    }
    if (djPlaylistModal && !dialog) {
      djPlaylistState = null;
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

    const sourceLink = target?.closest<HTMLAnchorElement>('[data-action="open-dj-crate-digger-link"]');
    if (sourceLink) {
      event.preventDefault();
      void services.openExternalUrl(sourceLink.href);
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
      if (clearedContextMenu) render();
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

    if (action === 'import-dj-playlist') {
      openDjPlaylistLauncher();
      return;
    }

    if (action === 'dj-playlist-open-import') {
      void importDjPlaylist();
      return;
    }

    if (action === 'dj-playlist-open-export') {
      void openDjPlaylistExport();
      return;
    }

    if (action === 'open-latest-dj-playlist') {
      void openDjPlaylistExport();
      return;
    }

    if (action === 'dj-playlist-select-recent') {
      const playlistId = button.dataset.playlistId;
      if (playlistId) void selectRecentDjPlaylistForExport(playlistId);
      return;
    }

    if (action === 'dj-playlist-export-copy') {
      void exportDjPlaylistM3u8(false, true);
      return;
    }

    if (action === 'dj-playlist-export-existing') {
      void exportDjPlaylistM3u8(false, false);
      return;
    }

    if (action === 'close-dj-playlist') {
      djPlaylistState = null;
      render();
      return;
    }

    if (action === 'dj-playlist-prev' || action === 'dj-playlist-next') {
      if (djPlaylistState?.pages.length) {
        const delta = action === 'dj-playlist-prev' ? -1 : 1;
        const pageIndex = Math.max(0, Math.min(djPlaylistState.pages.length - 1, djPlaylistState.pageIndex + delta));
        if (pageIndex !== djPlaylistState.pageIndex) {
          djPlaylistState = { ...djPlaylistState, pageIndex };
          render();
          void refreshDjPlaylistQrs();
        }
      }
      return;
    }

    if (action === 'dj-playlist-copy-page') {
      const page = djPlaylistState?.pages[djPlaylistState.pageIndex];
      if (page) void copyDjPlaylistText(page.text);
      return;
    }

    if (action === 'dj-playlist-copy-all') {
      if (djPlaylistState?.playlist) void copyDjPlaylistText(buildNeteaseImportText(djPlaylistState.playlist.tracks));
      return;
    }

    if (action === 'dj-playlist-export-txt') {
      void exportDjPlaylistTxt();
      return;
    }

    if (action === 'dj-playlist-export-w4dj') {
      void exportDjPlaylistW4dj();
      return;
    }

    if (action === 'dj-playlist-match') {
      void matchDjPlaylist();
      return;
    }

    if (action === 'dj-playlist-export-m3u8') {
      void exportDjPlaylistM3u8(false);
      return;
    }

    if (action === 'dj-playlist-export-partial') {
      void exportDjPlaylistM3u8(true);
      return;
    }

    if (action === 'dj-playlist-set-match') {
      const position = Number(button.dataset.position);
      const trackKey = button.dataset.trackKey;
      if (djPlaylistState?.playlist && Number.isInteger(position) && trackKey && services.setImportedDjPlaylistMatch) {
        void services.setImportedDjPlaylistMatch(djPlaylistState.playlist.playlistId, position, trackKey)
          .then((report) => { if (djPlaylistState) { djPlaylistState = { ...djPlaylistState, matchReport: report }; render(); } })
          .catch((error) => { if (djPlaylistState) { djPlaylistState = { ...djPlaylistState, error: error instanceof Error ? error.message : String(error) }; render(); } });
      }
      return;
    }

    if (action === 'dj-playlist-clear-match') {
      const position = Number(button.dataset.position);
      if (djPlaylistState?.playlist && Number.isInteger(position) && services.clearImportedDjPlaylistMatch) {
        void services.clearImportedDjPlaylistMatch(djPlaylistState.playlist.playlistId, position)
          .then((report) => { if (djPlaylistState) { djPlaylistState = { ...djPlaylistState, matchReport: report }; render(); } })
          .catch((error) => { if (djPlaylistState) { djPlaylistState = { ...djPlaylistState, error: error instanceof Error ? error.message : String(error) }; render(); } });
      }
      return;
    }

    if (action === 'close-library') {
      libraryState = null;
      render();
      return;
    }

    if (action === 'search-library') {
      searchLibrary();
      return;
    }

    if (action === 'reanalyze-library') {
      void reanalyzeLibrary();
      return;
    }

    if (action === 'find-invalid-library') {
      void findInvalidLibraryRecords();
      return;
    }

    if (action === 'cancel-invalid-scan') {
      void cancelInvalidLibraryScan();
      return;
    }

    if (action === 'clear-invalid-library') {
      void clearInvalidLibraryRecords();
      return;
    }

    if (action === 'relocate-library-track') {
      void relocateLibraryTrack();
      return;
    }

    if (action === 'remove-library-track') {
      void removeLibraryTrack();
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
      let sorts: LibraryQuery['sorts'];
      if (event.shiftKey) {
        sorts = current
          ? current.direction === 'asc'
            ? libraryState.query.sorts.map((sort) => sort.field === field ? { ...sort, direction: 'desc' as const } : sort)
            : libraryState.query.sorts.filter((sort) => sort.field !== field)
          : [...libraryState.query.sorts, { field, direction: 'asc' as const }];
      } else {
        sorts = current
          ? current.direction === 'asc'
            ? [{ field, direction: 'desc' as const }]
            : []
          : [{ field, direction: 'asc' as const }];
      }
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
      const secondValueInput = root.querySelector<HTMLInputElement>('input[data-action="library-filter-second-value"]');
      if (!fieldSelect || !operatorSelect || !valueInput) return;
      const field = fieldSelect.value as LibraryField;
      const operator = operatorSelect.value as LibraryOperator;
      if (!libraryOperatorsForField(field).includes(operator)) return;
      if (valueInput.type === 'number' && valueInput.value.trim() && !Number.isFinite(Number(valueInput.value))) return;
      const filter: LibraryFilter = {
        field,
        operator,
        value: ['is_empty', 'is_not_empty', 'is_true', 'is_false'].includes(operator)
          ? null
          : valueInput.value,
        secondValue: operator === 'between' ? secondValueInput?.value || null : null,
      };
      if (!filter.value && !['is_empty', 'is_not_empty', 'is_true', 'is_false'].includes(operator)) return;
      if (operator === 'between' && !filter.secondValue) return;
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
      markOnboardingSeen();
      render();
      return;
    }

    if (action === 'onboarding-next') {
      if (onboardingStep === ONBOARDING_STEP_COUNT - 1) {
        onboardingVisible = false;
        onboardingStep = 0;
        markOnboardingSeen();
      } else {
        onboardingStep = (onboardingStep + 1) as OnboardingStep;
      }
      render();
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

    if (action === 'cancel-preview') {
      if (!previewBusy) {
        previewModal = null;
        render();
      }
      return;
    }

    if (action === 'preview-detail') {
      if (!previewModal) return;
      const detailSlot = parseSlotIndex(button.dataset.slot);
      const kind = button.dataset.detailKind as PreviewDetailKind | undefined;
      if (detailSlot !== null && kind) {
        previewModal = { ...previewModal, detail: { slotIndex: detailSlot, kind } };
        render();
      }
      return;
    }

    if (action === 'close-preview-detail') {
      if (previewModal) {
        previewModal = { ...previewModal, detail: null };
        render();
      }
      return;
    }

    if (action === 'open-preview-file') {
      const path = button.dataset.path;
      if (path) {
        void (button.dataset.openTarget === 'destination'
          ? services.openDestination(path)
          : services.openSource(path));
      }
      return;
    }

    if (action === 'cancel-scan') {
      void cancelScanFlow();
      return;
    }

    if (action === 'close-scan') {
      if (scanProgress?.status !== 'running' && scanProgress?.status !== 'cancelling') {
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

    if (action === 'export-error-report') {
      const historyId = button.dataset.historyId;
      if (historyId) {
        void exportHistoryErrorReport(historyId);
      }
      return;
    }

    if (action === 'export-runtime-session') {
      const historyId = button.dataset.historyId;
      if (historyId) {
        void exportRuntimeSession(historyId);
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

    if (action === 'scan-local-netease') {
      void scanLocalNeteaseFolder();
      return;
    }

    if (action === 'select-netease-database') {
      void selectNeteaseMetadataDatabase();
      return;
    }

    if (action === 'clear-netease-database') {
      void clearNeteaseMetadataDatabase();
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

    if (action === 'resume-analysis') {
      resumeAnalysisFlow();
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
    const searchInput = (event.target as HTMLElement | null)?.closest<HTMLInputElement>('input[data-action="library-search"]');
    if (event.key === 'Enter' && searchInput && libraryState?.visible && !librarySearchComposing && !event.isComposing) {
      event.preventDefault();
      if (librarySearchTimer) {
        clearTimeout(librarySearchTimer);
        librarySearchTimer = null;
      }
      libraryQueryRevision += 1;
      const query = { ...libraryState.query, text: searchInput.value, offset: 0 };
      libraryState = { ...libraryState, query };
      void queryLibrary(query);
      return;
    }

    if (event.key === 'Escape' && libraryState?.visible) {
      event.preventDefault();
      if (libraryState.contextMenu) {
        libraryState = { ...libraryState, contextMenu: null };
        render();
        return;
      }
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
    if (event.key === 'Escape' && djPlaylistState?.visible) {
      event.preventDefault();
      djPlaylistState = null;
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
      markOnboardingSeen();
      render();
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
        markOnboardingSeen();
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
    const checkbox = (event.target as HTMLElement | null)?.closest<HTMLInputElement>('input[data-action="library-confirm-clear-invalid"]');
    if (checkbox && libraryState) {
      libraryState = { ...libraryState, confirmClearInvalid: checkbox.checked, notice: null };
      render();
      return;
    }
    const concurrencyInput = (event.target as HTMLElement | null)?.closest<HTMLInputElement>(
      'input[data-action="choose-concurrency-number"]',
    );
    if (concurrencyInput) {
      const parsed = Number(concurrencyInput.value);
      const normalized = Number.isFinite(parsed)
        ? Math.min(10, Math.max(1, Math.round(parsed)))
        : state.concurrencyLimit;
      concurrencyInput.value = String(normalized);
      void runAction(() => services.chooseConcurrencyLimit(String(normalized)), 'all');
      return;
    }
    const concurrencyRange = (event.target as HTMLElement | null)?.closest<HTMLInputElement>(
      'input[data-action="choose-concurrency-range"]',
    );
    if (concurrencyRange) {
      const parsed = Number(concurrencyRange.value);
      const normalized = Number.isFinite(parsed)
        ? Math.min(10, Math.max(1, Math.round(parsed)))
        : state.concurrencyLimit;
      concurrencyRange.value = String(normalized);
      void runAction(() => services.chooseConcurrencyLimit(String(normalized)), 'all');
      return;
    }
    const select = (event.target as HTMLElement | null)?.closest<HTMLSelectElement>('select');
    if (!select) {
      return;
    }

    if (select.dataset.action === 'library-filter-field') {
      const operator = root.querySelector<HTMLSelectElement>('select[data-action="library-filter-operator"]');
      const value = root.querySelector<HTMLInputElement>('input[data-action="library-filter-value"]');
      const field = select.value as LibraryField;
      const allowed = libraryOperatorsForField(field);
      if (operator) {
        operator.replaceChildren(...allowed.map((item) => {
          const option = document.createElement('option');
          option.value = item;
          option.textContent = item;
          return option;
        }));
      }
      if (value) {
        value.type = ['bpm', 'bitrate', 'file_size', 'duration', 'energy', 'danceability', 'loudness', 'updated_at'].includes(field) ? 'number' : 'text';
        value.value = '';
      }
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

  });

  root.addEventListener('compositionstart', (event) => {
    const input = (event.target as HTMLElement | null)?.closest<HTMLInputElement>('input[data-action="library-search"]');
    if (!input) return;
    librarySearchComposing = true;
    libraryQueryRevision += 1;
    if (librarySearchTimer) {
      clearTimeout(librarySearchTimer);
      librarySearchTimer = null;
    }
  });

  root.addEventListener('compositionend', (event) => {
    const input = (event.target as HTMLElement | null)?.closest<HTMLInputElement>('input[data-action="library-search"]');
    if (input) {
      librarySearchComposing = false;
    }
  });

  root.addEventListener('input', (event) => {
    const target = event.target as HTMLElement | null;
    const input = target?.closest<HTMLInputElement>('input');
    if (!input) return;
    if (input.dataset.action === 'choose-concurrency-range') {
      const normalized = Math.min(10, Math.max(1, Math.round(Number(input.value))));
      const numberInput = root.querySelector<HTMLInputElement>('input[data-action="choose-concurrency-number"]');
      if (numberInput) numberInput.value = String(normalized);
      return;
    }
    if (!libraryState) return;
    if (input.dataset.action === 'library-search') {
      // Invalidate an in-flight request as soon as the user changes the text.
      // Waiting until the next debounce fires lets an older, shorter query
      // overwrite the current IME/search value in the meantime.
      libraryQueryRevision += 1;
      const query = { ...libraryState.query, text: input.value, offset: 0 };
      // Keep the state in sync immediately. A result from an older debounced
      // request can otherwise re-render the input with stale text and move the
      // caret while the user is still typing.
      libraryState = { ...libraryState, query };
      if (librarySearchComposing || (event as InputEvent).isComposing) {
        librarySearchComposing = true;
        if (librarySearchTimer) {
          clearTimeout(librarySearchTimer);
          librarySearchTimer = null;
        }
        return;
      }
      if (libraryRenderDeferred) {
        libraryRenderDeferred = false;
        render();
      }
      if (librarySearchTimer) clearTimeout(librarySearchTimer);
      librarySearchTimer = setTimeout(() => {
        librarySearchTimer = null;
        void queryLibrary(query);
      }, 250);
    } else if (input.dataset.action === 'library-lyrics-search') {
      libraryState = { ...libraryState, lyricsSearch: input.value };
      render();
    }
  });

  const setDjDropActive = (active: boolean) => {
    if (!djPlaylistState && !active) return;
    if (djPlaylistState?.dropActive === active) return;
    if (active) {
      djPlaylistState = djPlaylistState
        ? { ...djPlaylistState, dropActive: true }
        : {
          visible: false,
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
          dropActive: true,
        };
    } else if (djPlaylistState) {
      djPlaylistState = { ...djPlaylistState, dropActive: false };
    }
    render();
  };

  let nativePlaylistDragActive = false;

  const clearDropTargets = () => {
    root.querySelectorAll<HTMLElement>('[data-drop-kind].is-drag-over').forEach((target) => {
      target.classList.remove('is-drag-over');
    });
    setDjDropActive(false);
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

  const pathsFromBrowserDrop = (event: DragEvent): string[] => {
    const files = Array.from(event.dataTransfer?.files ?? []) as Array<File & { path?: string }>;
    const paths = files
      .map((file) => file.path || file.name)
      .filter((path): path is string => Boolean(path));
    if (paths.length > 0) {
      return paths;
    }

    return (event.dataTransfer?.getData('text/uri-list') ?? '')
      .split('\n')
      .map((value) => value.trim())
      .filter((value) => value && !value.startsWith('#') && value.startsWith('file://'))
      .flatMap((uri) => {
        try {
          return [decodeURIComponent(new URL(uri).pathname)];
        } catch {
          return [];
        }
      });
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
    const paths = pathsFromBrowserDrop(event);
    const playlistPaths = w4djPlaylistPaths(paths);
    if (playlistPaths.length > 0) {
      clearDropTargets();
      if (playlistPaths.length === 1 && paths.length === 1) setDjDropActive(true);
      event.preventDefault();
      return;
    }
    if (containsModelFile(paths)) {
      clearDropTargets();
      event.preventDefault();
      return;
    }
    const target = browserDropTargetAt(event);
    clearDropTargets();
    if (!target) {
      return;
    }

    event.preventDefault();
    target.classList.add('is-drag-over');
  });

  root.addEventListener('drop', (event) => {
    const paths = pathsFromBrowserDrop(event);
    const playlistPaths = w4djPlaylistPaths(paths);
    if (playlistPaths.length > 0) {
      clearDropTargets();
      event.preventDefault();
      if (playlistPaths.length === 1 && paths.length === 1) {
        void importDjPlaylistPath(playlistPaths[0]);
      } else {
        window.alert(state.lang === 'zh' ? '请一次只拖入一个 .w4dj 文件。' : 'Drop exactly one .w4dj file.');
      }
      return;
    }
    if (containsModelFile(paths)) {
      clearDropTargets();
      event.preventDefault();
      window.alert(t('essentiaModelsDropDisabled', state.lang));
      return;
    }
    const target = browserDropTargetAt(event);
    clearDropTargets();
    if (!target) {
      return;
    }

    event.preventDefault();
    handleDirectoryDrop(target, paths[0] ?? null);
  });

  root.addEventListener('dragleave', (event) => {
    if (event.relatedTarget instanceof Node && root.contains(event.relatedTarget)) {
      return;
    }
    clearDropTargets();
  });

  try {
    const currentWindow = getCurrentWindow();
    const scaleFactorPromise = currentWindow
      .scaleFactor()
      .catch(() => window.devicePixelRatio || 1);
    const listener = currentWindow.onDragDropEvent(async ({ payload }: { payload: DragDropEvent }) => {
      if (payload.type === 'leave') {
        nativePlaylistDragActive = false;
        clearDropTargets();
        return;
      }

      const droppedPaths = payload.type === 'enter' || payload.type === 'drop' ? payload.paths : [];
      const playlistPaths = w4djPlaylistPaths(droppedPaths);
      if (playlistPaths.length > 0) {
        clearDropTargets();
        if (playlistPaths.length === 1 && droppedPaths.length === 1) {
          nativePlaylistDragActive = payload.type !== 'drop';
          setDjDropActive(true);
          if (payload.type === 'drop') {
            nativePlaylistDragActive = false;
            void importDjPlaylistPath(playlistPaths[0]);
          }
        } else if (payload.type === 'drop') {
          nativePlaylistDragActive = false;
          window.alert(state.lang === 'zh' ? '请一次只拖入一个 .w4dj 文件。' : 'Drop exactly one .w4dj file.');
        }
        return;
      }

      if (payload.type === 'over' && nativePlaylistDragActive) {
        clearDropTargets();
        setDjDropActive(true);
        return;
      }
      if (payload.type === 'drop') {
        nativePlaylistDragActive = false;
      }

      if ((payload.type === 'enter' || payload.type === 'drop') && containsModelFile(payload.paths)) {
        clearDropTargets();
        if (payload.type === 'drop') {
          window.alert(t('essentiaModelsDropDisabled', state.lang));
        }
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

  if (services.listenLibraryRefreshProgress) {
    void services.listenLibraryRefreshProgress((progress) => {
      void handleLibraryRefreshProgress(progress);
    }).catch((error) => console.warn('Failed to subscribe to library refresh progress:', error));
  }

  if (services.listenInvalidLibraryScanProgress) {
    void services.listenInvalidLibraryScanProgress((progress) => {
      void handleInvalidLibraryScanProgress(progress);
    }).catch((error) => console.warn('Failed to subscribe to invalid library scan progress:', error));
  }

  if (services.listenNeteaseDiscoveryProgress) {
    void services.listenNeteaseDiscoveryProgress((progress) => {
      neteaseDiscoveryProgress = progress;
      if (progress.status === 'running') {
        if (!updateNeteaseDiscoveryProgressDom(progress)) {
          render();
        }
        return;
      }
      if (progress.status === 'completed') {
        setTimeout(() => {
          if (neteaseDiscoveryProgress === progress) {
            neteaseDiscoveryProgress = null;
            render();
          }
        }, 1800);
      }
      render();
    }).catch((error) => console.warn('Failed to subscribe to NetEase discovery progress:', error));
  }

  if (services.listenNeteaseMetadataCacheProgress) {
    void services.listenNeteaseMetadataCacheProgress((progress) => {
      if (scanProgress && ['running', 'cancelling'].includes(scanProgress.status)) {
        const tasks = scanProgress.tasks?.map((task) => task.slot_index === 0
          ? {
            ...task,
            phase: 'preparing' as AppScanPhase,
            processed: progress.processed,
            total: progress.total ?? 0,
            current_file: progress.currentItem,
          }
          : task);
        scanProgress = {
          ...scanProgress,
          phase: 'preparing',
          processed: progress.processed,
          total: progress.total ?? 0,
          current_file: progress.currentItem,
          message: progress.message || t('scanPreparing', state.lang),
          tasks,
        };
        if (!updateScanProgressDom(scanProgress)) render();
      }
      const status = neteaseMetadataDatabase.status;
      const wasBusy = neteaseMetadataDatabase.busy;
      neteaseMetadataDatabase = {
        ...neteaseMetadataDatabase,
        status: status
          ? {
            ...status,
            cacheStatus: progress.status,
            cachedRecordCount: progress.cachedRecordCount,
          }
          : status,
        busy: progress.status === 'building' || progress.status === 'cancelling',
        message: progress.message || neteaseMetadataDatabase.message,
        error: progress.error,
      };
      // Keep the task card and focused controls stable during frequent cache
      // progress events.  Re-render only when button affordances or terminal
      // state change; the situation value itself is updated in place.
      const needsStructuralRender = wasBusy !== neteaseMetadataDatabase.busy
        || !['building', 'cancelling'].includes(progress.status);
      if (needsStructuralRender || !updateNeteaseSituationDom()) {
        render();
      }
    }).catch((error) => console.warn('Failed to subscribe to NetEase metadata cache progress:', error));
  }

  if (resumableAnalysis) {
    const resumableTotal = resumableAnalysis.previews.reduce(
      (total, preview) => total + preview.preview.candidates.length,
      0,
    );
    analysisState = {
      ...analysisState,
      status: 'cancelled',
      total: resumableTotal,
      stage: 'cancelled',
      message: state.lang === 'zh' ? '上次增强分析未完成，可继续' : 'The previous enhanced analysis was interrupted; resume it',
      resumeAvailable: true,
    };
  }

  // The WebView can lose localStorage on a reload or after an application
  // restart. Runtime sessions are the durable source of truth; restore only
  // the resumable offer here and never start analysis automatically.
  if (services.loadIncompleteAnalysisRun) {
    void services.loadIncompleteAnalysisRun()
      .then((run) => {
        if (!run || run.previews.length === 0) {
          return;
        }
        setResumableAnalysis({
          batchId: run.batchId,
          previews: run.previews,
          analysis: run.analysis,
        });
        const total = run.analysis?.total
          ?? run.previews.reduce(
            (count, preview) => count + preview.preview.candidates.length,
            0,
          );
        analysisState = {
          ...analysisState,
          status: 'cancelled',
          total,
          completed: run.analysis?.completed ?? analysisState.completed,
          failedCount: (run.analysis?.failed ?? 0) + (run.analysis?.timedOut ?? 0),
          currentItem: run.analysis?.currentItem ?? '',
          stage: run.analysis?.currentStage ?? 'cancelled',
          workerJobId: run.analysis?.workerJobId ?? '',
          message: state.lang === 'zh'
            ? '上次增强分析未完成，可继续'
            : 'The previous enhanced analysis was interrupted; resume it',
          resumeAvailable: true,
        };
        render();
      })
      .catch((error) => console.warn('Failed to restore incomplete analysis run:', error));
  }

  render();
  analysisCacheLoadPromise = loadAnalysisCache();
  void loadImportedDjPlaylistList();
  desktopStateHydration = services.loadDesktopState()
    .then((desktopState) => applyDesktopState(desktopState))
    .catch((error) => {
      console.warn('Failed to hydrate desktop state before conversion:', error);
    });
  if (services.loadNeteaseMetadataDatabaseStatus) {
    void desktopStateHydration
      .then(async () => {
        try {
          const status = await services.loadNeteaseMetadataDatabaseStatus?.();
          if (status) {
            neteaseMetadataDatabase = {
              status,
              busy: false,
              message: null,
              error: null,
            };
            render();
          }
        } catch (error) {
          neteaseMetadataDatabase = {
            ...neteaseMetadataDatabase,
            error: error instanceof Error ? error.message : String(error),
          };
          render();
        }
      });
  }
  void refreshHistory();
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
  const discogs = modelStatus.discogsEffnet;
  const discogsReady = !discogs || Object.values(discogs).every(Boolean);
  const modelsReady = modelStatus.embedding
    && modelStatus.genre
    && modelStatus.mood
    && modelStatus.instrument
    && discogsReady;
  return `
    <details class="output-settings" data-role="advanced-output-settings" aria-label="${t('advancedOptions', state.lang)}" ${expanded ? 'open' : ''}>
      <summary aria-label="${t('advancedOptions', state.lang)}">${t('advancedOptions', state.lang)}</summary>
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
        <div class="concurrency-setting" data-role="concurrency-setting">
          <label for="concurrency-limit-range"><span>${t('concurrencyLimit', state.lang)}</span></label>
          <div class="concurrency-controls">
            <input
              id="concurrency-limit-range"
              type="range"
              min="1"
              max="10"
              step="1"
              value="${state.concurrencyLimit}"
              data-action="choose-concurrency-range"
              aria-label="${t('concurrencyLimit', state.lang)}"
            />
            <input
              type="number"
              min="1"
              max="10"
              step="1"
              value="${state.concurrencyLimit}"
              data-action="choose-concurrency-number"
              aria-label="${t('concurrencyLimit', state.lang)}"
            />
          </div>
        </div>
        ${ENHANCED_ANALYSIS_FEATURES_VISIBLE ? `
          <div class="essentia-model-settings">
            <span class="essentia-model-title">${t('essentiaModelsTitle', state.lang)}</span>
            <small>${modelsReady ? t('essentiaModelsReady', state.lang) : t('essentiaModelsMissing', state.lang)}</small>
            <div class="essentia-model-actions">
              <button type="button" class="secondary-action enhanced-cache-clear" data-action="clear-analysis-cache">
                ${t('clearEnhancedCache', state.lang)}
              </button>
            </div>
          </div>
        ` : ''}
        ${ANALYSIS_CACHE_CLEAR_VISIBLE ? `
          <button type="button" class="secondary-action enhanced-cache-clear" data-action="clear-analysis-cache">
            ${t('clearEnhancedCache', state.lang)}
          </button>
        ` : ''}
        ${ENHANCED_ANALYSIS_FEATURES_VISIBLE ? `
          <button type="button" class="secondary-action scan-cache-clear" data-action="clear-scan-cache">
            ${t('clearScanCache', state.lang)}
          </button>
        ` : ''}
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
      activeConcurrencyLimit: slot.active_concurrency_limit ?? null,
    })) as [AppSyncSlotViewState, AppSyncSlotViewState],
    mode: state.mode,
    losslessFormat: state.lossless_format,
    conversionMode: state.conversion_mode,
    enhancedMode: state.enhanced_mode,
    conflictStrategy: state.conflict_strategy,
    filenameRule: state.filename_rule,
    neteaseFilenameFormat: state.netease_filename_format,
    concurrencyLimit: Math.min(10, Math.max(1, Math.round(state.concurrency_limit || 2))),
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
    matching_metadata: 'scanMatchingMetadata',
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

function icon(name: 'folder' | 'music' | 'export' | 'open' | 'trash' | 'check' | 'convert' | 'disc' | 'play' | 'pause' | 'list' | 'sun' | 'moon' | 'arrow' | 'help' | 'refresh'): string {
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
    refresh: '<path d="M13 5.2A5.4 5.4 0 1 0 13.2 9"/><path d="M13 2.8v2.8h-2.8"/>',
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
