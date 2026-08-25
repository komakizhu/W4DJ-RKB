import { formatDanceabilityRating } from './danceability-rating';
import { formatEnergyRating } from './energy-rating';

export type LibraryLocalStatus = 'available' | 'out_of_scope' | 'missing' | 'unreadable' | 'database_only';

export type LibraryTrack = {
  trackKey: string;
  neteaseTrackId: string | null;
  title: string;
  artists: string;
  album: string;
  neteaseGenre: string;
  aliasesJson?: string;
  copyrightText?: string;
  publishDate?: string;
  essentiaGenre: string;
  coverPath: string | null;
  coverAvailable: boolean;
  localStatus: LibraryLocalStatus;
  effectiveDurationSeconds: number | null;
  durationSource: 'essentia' | 'measured' | 'netease' | null;
  effectiveFormat: string | null;
  effectiveBitrateBps: number | null;
  effectiveSizeBytes: number | null;
  bpm: number | null;
  musicalKey: string | null;
  scale: string | null;
  integratedLoudnessLufs: number | null;
  energy: number | null;
  danceability: number | null;
  moodJson: string;
  instrumentJson: string;
  styleJson?: string;
  discogsMoodThemeJson: string;
  discogsApproachabilityJson: string;
  discogsInstrumentationJson: string;
  discogsTimbreJson: string;
  discogsDanceabilityJson: string;
  dropLoudnessLufs: number | null;
  lyricPlainText: string;
  lyricTranslatedText: string;
  lyricRomanizedText: string;
  lyricLrcText: string;
  lyricLanguage: string;
  lyricSyncType: string;
  lyricSource: string;
  updatedAtMs: number;
};

export type LibrarySourceRecord = {
  trackKey: string;
  sourceTable: string;
  sourcePrimaryKey: string;
  sourceVersion: string | null;
  rawJson: string;
  importedAtMs: number;
};

export type LibraryPage = {
  items: LibraryTrack[];
  total: number;
  limit: number;
  offset: number;
};

export type LibraryQuery = {
  text: string;
  filters: LibraryFilter[];
  filterLogic: 'and' | 'or';
  sorts: LibrarySort[];
  limit: number;
  offset: number;
};

export type LibraryField =
  | 'title' | 'artists' | 'album' | 'file_name' | 'local_status'
  | 'format' | 'bitrate' | 'file_size' | 'duration' | 'netease_genre'
  | 'essentia_genre' | 'bpm' | 'musical_key' | 'loudness' | 'energy'
  | 'danceability' | 'discogs_mood_theme' | 'discogs_approachability'
  | 'discogs_instrumentation' | 'discogs_timbre' | 'discogs_danceability'
  | 'cover_available' | 'lyrics' | 'updated_at';

export type LibraryOperator =
  | 'is' | 'is_not' | 'contains' | 'not_contains' | 'starts_with'
  | 'ends_with' | 'greater_than' | 'greater_or_equal' | 'less_than'
  | 'less_or_equal' | 'between' | 'is_empty' | 'is_not_empty'
  | 'is_true' | 'is_false';

export type LibraryFilter = {
  field: LibraryField;
  operator: LibraryOperator;
  value?: string | null;
  secondValue?: string | null;
};

export type LibrarySort = {
  field: LibraryField;
  direction: 'asc' | 'desc';
};

export type LibraryLyricsTab = 'plain' | 'translated' | 'romanized' | 'lrc';

export type NeteaseDiscovery = {
  databasePath: string | null;
  musicFolder: string | null;
  recordCount: number;
  localFileCount: number;
};

export type LibraryRefreshStatus = 'idle' | 'running' | 'cancelling' | 'completed' | 'cancelled' | 'error';

export type LibraryRefreshStage =
  | 'locatingDatabase' | 'readingRecords' | 'checkingLocalFiles'
  | 'probingLocalFiles' | 'importingAnalysis' | 'committing';

export type LibraryRefreshSummary = {
  trackCount: number;
  localFileCount: number;
  readableFileCount: number;
  reusedFileCount: number;
  databasePath: string;
  musicFolder: string | null;
};

export type LibraryRefreshProgress = {
  refreshId: string;
  status: LibraryRefreshStatus;
  stage: LibraryRefreshStage;
  processed: number;
  total: number | null;
  currentItem: string;
  message: string;
  summary: LibraryRefreshSummary | null;
  error: string | null;
};

export type LibraryStatus = {
  catalogPath: string;
  trackCount: number;
  analyzedTrackCount?: number;
  netease: NeteaseDiscovery;
  manualDatabasePath: string | null;
  refresh: LibraryRefreshProgress;
  databaseWarning: string | null;
  totalTrackCount?: number;
  availableTrackCount?: number;
  invalidTrackCount?: number;
  notAnalyzedCount?: number;
  analysisFailedCount?: number;
  analysisCompletedCount?: number;
  invalidScan?: LibraryInvalidScanProgress;
};

export type LibraryInvalidScanProgress = {
  scanId: string;
  status: 'idle' | 'running' | 'cancelling' | 'completed' | 'cancelled' | 'error';
  processed: number;
  total: number;
  currentItem: string;
  message: string;
  error: string | null;
};

export type LibraryDashboardState = {
  visible: boolean;
  busy: boolean;
  status: LibraryStatus | null;
  page: LibraryPage | null;
  query: LibraryQuery;
  detail: LibraryTrack | null;
  error: string | null;
  coverData?: Record<string, string>;
  lyricsTab?: LibraryLyricsTab;
  lyricsSearch?: string;
  sourceRecords?: LibrarySourceRecord[];
  contextMenu?: { trackKey: string; x: number; y: number } | null;
  confirmClearInvalid?: boolean;
  notice?: string | null;
};

export function isLibraryRefreshActive(progress: LibraryRefreshProgress | null | undefined): boolean {
  return progress?.status === 'running' || progress?.status === 'cancelling';
}

type LibraryColumn = {
  id: string;
  field?: LibraryField;
  label: string;
  alwaysVisible?: boolean;
};

const LIBRARY_COLUMNS: LibraryColumn[] = [
  { id: 'cover', label: '封面', alwaysVisible: true },
  { id: 'title', field: 'title', label: '歌曲名' },
  { id: 'artists', field: 'artists', label: '歌手' },
  { id: 'album', field: 'album', label: '专辑' },
  { id: 'localStatus', field: 'local_status', label: '本地状态' },
  { id: 'format', field: 'format', label: '格式' },
  { id: 'bitrate', field: 'bitrate', label: '平均码率' },
  { id: 'size', field: 'file_size', label: '文件大小' },
  { id: 'duration', field: 'duration', label: '时长' },
  { id: 'essentiaGenre', field: 'essentia_genre', label: 'Essentia Genre' },
  { id: 'bpmKey', field: 'bpm', label: 'BPM / Key' },
  { id: 'loudness', field: 'loudness', label: '响度' },
  { id: 'energy', field: 'energy', label: '能量' },
  { id: 'danceability', field: 'danceability', label: '可舞性（10级）' },
  { id: 'discogsMoodTheme', field: 'discogs_mood_theme', label: 'Discogs Mood/Theme' },
  { id: 'discogsApproachability', field: 'discogs_approachability', label: '可接近度' },
  { id: 'discogsInstrumentation', field: 'discogs_instrumentation', label: 'Discogs 乐器' },
  { id: 'discogsTimbre', field: 'discogs_timbre', label: 'Timbre' },
  { id: 'discogsDanceability', field: 'discogs_danceability', label: 'Discogs 可舞性' },
  { id: 'instrument', label: '器乐/人声' },
  { id: 'acousticElectronic', label: '原生乐器/电子' },
  { id: 'lyrics', field: 'lyrics', label: '歌词' },
  { id: 'updated', field: 'updated_at', label: '最后更新' },
  { id: 'aliases', label: '别名' },
];

/** @deprecated The raw 0..3 scale is retained for query compatibility only. */
export const DANCEABILITY_MAX = 3;

const LIBRARY_COLUMN_STORAGE_KEY = 'w4dj.libraryDashboard.columns.v1';

export type LibraryColumnSettings = {
  order: string[];
  hidden: string[];
  widths: Record<string, number>;
};

const TEXT_LIBRARY_OPERATORS: LibraryOperator[] = [
  'is', 'is_not', 'contains', 'not_contains', 'starts_with', 'ends_with',
  'is_empty', 'is_not_empty',
];
const NUMBER_LIBRARY_OPERATORS: LibraryOperator[] = [
  'is', 'is_not', 'greater_than', 'greater_or_equal', 'less_than', 'less_or_equal', 'between',
  'is_empty', 'is_not_empty',
];

export function libraryOperatorsForField(field: LibraryField): LibraryOperator[] {
  if (field === 'cover_available') return ['is_true', 'is_false'];
  if (['bpm', 'bitrate', 'file_size', 'duration', 'energy', 'danceability', 'loudness', 'updated_at'].includes(field)) {
    return NUMBER_LIBRARY_OPERATORS;
  }
  return TEXT_LIBRARY_OPERATORS;
}

export function loadLibraryColumnSettings(): LibraryColumnSettings {
  const fallback = LIBRARY_COLUMNS.map((column) => column.id);
  const defaultSettings: LibraryColumnSettings = {
    order: fallback,
    hidden: ['discogsMoodTheme', 'discogsApproachability', 'discogsInstrumentation', 'discogsTimbre', 'discogsDanceability'],
    widths: {},
  };
  try {
    const parsed = JSON.parse(localStorage.getItem(LIBRARY_COLUMN_STORAGE_KEY) || 'null');
    if (Array.isArray(parsed)) {
      return { ...defaultSettings, order: parsed.filter((value): value is string => typeof value === 'string') };
    }
    if (!parsed || typeof parsed !== 'object') return defaultSettings;
    const widths = Object.fromEntries(
      Object.entries(parsed.widths || {}).filter(([, value]) => typeof value === 'number' && Number.isFinite(value))
        .map(([key, value]) => [key, Math.max(72, Math.min(420, value as number))]),
    );
    return {
      order: Array.isArray(parsed.order) ? parsed.order.filter((value: unknown): value is string => typeof value === 'string') : fallback,
      hidden: Array.isArray(parsed.hidden) ? parsed.hidden.filter((value: unknown): value is string => typeof value === 'string') : [],
      widths,
    };
  } catch {
    return defaultSettings;
  }
}

function visibleLibraryColumns(): LibraryColumn[] {
  const fallback = LIBRARY_COLUMNS.map((column) => column.id);
  const stored = loadLibraryColumnSettings();
  const order = stored.order.length ? stored.order : fallback;
  const ids = [...new Set([...order, ...fallback])];
  const columns = ids
    .map((id) => LIBRARY_COLUMNS.find((column) => column.id === id))
    .filter((column): column is LibraryColumn => Boolean(column));
  // The display-field controls are intentionally not shown in the compact
  // dashboard. Keep the stored setting for backwards compatibility, but keep
  // every column available so an old hidden-column preference cannot make a
  // column impossible to restore.
  return columns;
}

export function saveLibraryColumnOrder(order: string[]): void {
  try {
    const existing = loadLibraryColumnSettings();
    localStorage.setItem(LIBRARY_COLUMN_STORAGE_KEY, JSON.stringify({ ...existing, order }));
  } catch {
    // Rendering the dashboard must continue in private browsing / test DOMs.
  }
}

export function saveLibraryColumnWidth(columnId: string, width: number): void {
  if (!LIBRARY_COLUMNS.some((column) => column.id === columnId)) return;
  try {
    const existing = loadLibraryColumnSettings();
    localStorage.setItem(LIBRARY_COLUMN_STORAGE_KEY, JSON.stringify({
      ...existing,
      widths: { ...existing.widths, [columnId]: Math.max(72, Math.min(420, Math.round(width))) },
    }));
  } catch {
    // Ignore storage failures in private browsing.
  }
}

export function toggleLibraryColumn(columnId: string): void {
  if (!LIBRARY_COLUMNS.some((column) => column.id === columnId && !column.alwaysVisible)) return;
  try {
    const parsed = loadLibraryColumnSettings();
    const hidden = new Set<string>(parsed.hidden);
    if (hidden.has(columnId)) hidden.delete(columnId); else hidden.add(columnId);
    localStorage.setItem(LIBRARY_COLUMN_STORAGE_KEY, JSON.stringify({ ...parsed, hidden: [...hidden] }));
  } catch {
    // Ignore storage failures; the next render uses defaults.
  }
}

export function libraryColumnIds(): string[] {
  // Drag-and-drop must start from the order currently rendered in the table.
  // Using the static declaration order here made the second reorder operation
  // silently jump back to the original layout.
  return visibleLibraryColumns().map((column) => column.id);
}

function escapeHtml(value: unknown): string {
  return String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

function label(value: string, lang: 'zh' | 'en'): string {
  if (lang === 'en') {
    return {
      available: 'Available',
      out_of_scope: 'Out of scope',
      missing: 'Missing',
      unreadable: 'Unreadable',
      database_only: 'Database only',
    }[value] || value;
  }
  return {
    available: '本地可用',
    out_of_scope: '已移出当前输出范围',
    missing: '文件缺失',
    unreadable: '无法读取',
    database_only: '仅数据库记录',
  }[value] || value;
}

function formatBytes(value: number | null, lang: 'zh' | 'en'): string {
  if (value == null || !Number.isFinite(value)) return '—';
  const units = lang === 'zh' ? ['B', 'KB', 'MB', 'GB'] : ['B', 'KB', 'MB', 'GB'];
  let amount = value;
  let index = 0;
  while (amount >= 1024 && index < units.length - 1) {
    amount /= 1024;
    index += 1;
  }
  return `${amount.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

function formatDuration(value: number | null): string {
  if (value == null || !Number.isFinite(value)) return '—';
  const seconds = Math.max(0, Math.round(value));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}

function formatNumber(value: number | null, suffix = ''): string {
  return value == null || !Number.isFinite(value) ? '—' : `${value.toFixed(2)}${suffix}`;
}

/**
 * Presents an analysis score on its declared scale as a ten-level rating.
 * A full star is 0.2 of the scale, an outline star is 0.1, and the
 * percentage is rounded for readability while the stored score remains
 * unchanged. This helper is retained for normalized scores. Danceability and
 * Energy use their calibrated renderers so their raw Essentia values stay
 * inspectable without changing query or storage semantics.
 */
export function formatRatingStars(value: number | null, maximum = 1): string {
  if (value == null || !Number.isFinite(value)) return '—';
  const safeMaximum = Number.isFinite(maximum) && maximum > 0 ? maximum : 1;
  const percentage = Math.round(Math.min(safeMaximum, Math.max(0, value)) / safeMaximum * 100);
  const level = Math.round(percentage / 10);
  const solidStars = Math.floor(level / 2);
  const outlineStar = level % 2 === 1 ? '☆' : '';
  const stars = `${'★'.repeat(solidStars)}${outlineStar}`;
  return `${stars || '—'} ${percentage}%`;
}

function renderRatingStars(value: number | null, maximum = 1): string {
  const text = formatRatingStars(value, maximum);
  const [stars, percentage = ''] = text.split(' ');
  return `<span class="library-rating" title="${escapeHtml(text)}"><span class="library-rating-stars">${escapeHtml(stars)}</span>${percentage ? `<span class="library-rating-percent">${escapeHtml(percentage)}</span>` : ''}</span>`;
}

function renderDanceabilityRating(value: number | null): string {
  const text = formatDanceabilityRating(value);
  const [stars, rating = ''] = text.split(' ');
  const rawValue = value == null || !Number.isFinite(value) ? '—' : String(value);
  return `<span class="library-rating" title="${escapeHtml(`Essentia raw: ${rawValue} · ${text}`)}"><span class="library-rating-stars">${escapeHtml(stars)}</span>${rating ? `<span class="library-rating-percent">${escapeHtml(rating)}</span>` : ''}</span>`;
}

function renderEnergyRating(value: number | null): string {
  const text = formatEnergyRating(value);
  const [stars, rating = ''] = text.split(' ');
  const rawValue = value == null || !Number.isFinite(value) ? '—' : String(value);
  const title = `Essentia RMS² raw: ${rawValue} · ${text}`;
  return `<span class="library-rating" title="${escapeHtml(title)}"><span class="library-rating-stars">${escapeHtml(stars)}</span>${rating ? `<span class="library-rating-percent">${escapeHtml(rating)}</span>` : ''}</span>`;
}

type StoredAnalysisLabel = { label: string; confidence?: number };

function storedAnalysisLabels(raw: string | null | undefined): StoredAnalysisLabel[] {
  if (!raw || raw === '[]') return [];
  try {
    const value: unknown = JSON.parse(raw);
    if (!Array.isArray(value)) return [];
    return value.filter((item): item is StoredAnalysisLabel => Boolean(item)
      && typeof item === 'object'
      && typeof (item as { label?: unknown }).label === 'string');
  } catch {
    return [];
  }
}

function analysisLabelsForTable(
  raw: string | null | undefined,
  lang: 'zh' | 'en',
  labels: Readonly<Record<string, [string, string]>>,
  include?: (label: string) => boolean,
): string {
  const values = storedAnalysisLabels(raw)
    .map((item) => item.label.trim())
    .filter((value) => value && (!include || include(value.toLowerCase())))
    .map((value) => labels[value.toLowerCase()]?.[lang === 'zh' ? 0 : 1] || value);
  return values.length ? values.join(' · ') : '—';
}

const INSTRUMENT_LABELS: Readonly<Record<string, [string, string]>> = {
  instrumental: ['器乐', 'Instrumental'],
  voice: ['人声', 'Voice'],
};

const ACOUSTIC_ELECTRONIC_LABELS: Readonly<Record<string, [string, string]>> = {
  acoustic: ['原生乐器', 'Acoustic'],
  electronic: ['电子', 'Electronic'],
};

type DiscogsStoredResult = {
  status?: string;
  labels?: Array<{ label?: string; confidence?: number }>;
  selectedClass?: string;
  selectedConfidence?: number;
  scores?: Record<string, number>;
  reason?: string | null;
};

function discogsStoredResult(raw: string | null | undefined): DiscogsStoredResult | null {
  if (!raw || raw === '{}' || raw === '[]') return null;
  try {
    const value = JSON.parse(raw) as DiscogsStoredResult;
    return value && typeof value === 'object' ? value : null;
  } catch {
    return null;
  }
}

function discogsCell(raw: string | null | undefined, lang: 'zh' | 'en', mode: 'labels' | 'class' | 'dance'): string {
  const result = discogsStoredResult(raw);
  if (!result) return lang === 'zh' ? '未生成' : 'Not generated';
  if (result.status && result.status !== 'completed') {
    return result.status === 'model_missing'
      ? (lang === 'zh' ? '模型缺失' : 'Model missing')
      : (lang === 'zh' ? `失败：${result.reason || result.status}` : `Failed: ${result.reason || result.status}`);
  }
  if (mode === 'labels') {
    return result.labels?.map((entry) => `${entry.label || ''}${entry.confidence == null ? '' : ` ${(entry.confidence * 100).toFixed(0)}%`}`)
      .filter(Boolean).join(' · ') || (lang === 'zh' ? '未生成' : 'Not generated');
  }
  const selected = result.selectedClass || result.labels?.[0]?.label;
  const confidence = result.selectedConfidence ?? result.labels?.[0]?.confidence;
  if (!selected) return lang === 'zh' ? '未生成' : 'Not generated';
  if (mode === 'dance') return `${selected} ${confidence == null ? '' : `${(confidence * 100).toFixed(0)}%`}`.trim();
  return `${selected}${confidence == null ? '' : ` ${(confidence * 100).toFixed(0)}%`}`;
}

function trackRows(page: LibraryPage, lang: 'zh' | 'en', coverData: Record<string, string> = {}, columns = visibleLibraryColumns()): string {
  if (page.items.length === 0) {
    return `<tr><td class="library-empty-cell" colspan="${columns.length}">${lang === 'zh' ? '没有匹配的歌曲' : 'No matching tracks'}</td></tr>`;
  }
  const cell = (column: LibraryColumn, track: LibraryTrack): string => {
    switch (column.id) {
      case 'cover': return `<td class="library-cover-cell">${coverData[track.trackKey]
        ? `<img loading="lazy" src="${escapeHtml(coverData[track.trackKey])}" alt="" />`
        : '<span class="library-cover-placeholder">♪</span>'}</td>`;
      case 'title': return `<td><strong>${escapeHtml(track.title || '—')}</strong></td>`;
      case 'artists': return `<td>${escapeHtml(track.artists || '—')}</td>`;
      case 'album': return `<td>${escapeHtml(track.album || '—')}</td>`;
      case 'localStatus': return `<td><span class="library-status" data-status="${escapeHtml(track.localStatus)}">${label(track.localStatus, lang)}</span></td>`;
      case 'format': return `<td>${escapeHtml(track.effectiveFormat || '—')}</td>`;
      case 'bitrate': return `<td>${formatNumber(track.effectiveBitrateBps == null ? null : track.effectiveBitrateBps / 1000, ' kbps')}</td>`;
      case 'size': return `<td>${formatBytes(track.effectiveSizeBytes, lang)}</td>`;
      case 'duration': return `<td>${formatDuration(track.effectiveDurationSeconds)}</td>`;
      case 'essentiaGenre': return `<td>${escapeHtml(track.essentiaGenre || '—')}</td>`;
      case 'bpmKey': return `<td>${track.bpm == null ? '—' : `${track.bpm.toFixed(1)} · ${escapeHtml(track.musicalKey || '')}`}</td>`;
      case 'loudness': return `<td>${formatNumber(track.integratedLoudnessLufs, ' LUFS')}</td>`;
      case 'energy': return `<td>${renderEnergyRating(track.energy)}</td>`;
      case 'danceability': return `<td>${renderDanceabilityRating(track.danceability)}</td>`;
      case 'discogsMoodTheme': return `<td>${escapeHtml(discogsCell(track.discogsMoodThemeJson, lang, 'labels'))}</td>`;
      case 'discogsApproachability': return `<td>${escapeHtml(discogsCell(track.discogsApproachabilityJson, lang, 'class'))}</td>`;
      case 'discogsInstrumentation': return `<td>${escapeHtml(discogsCell(track.discogsInstrumentationJson, lang, 'labels'))}</td>`;
      case 'discogsTimbre': return `<td>${escapeHtml(discogsCell(track.discogsTimbreJson, lang, 'class'))}</td>`;
      case 'discogsDanceability': return `<td>${escapeHtml(discogsCell(track.discogsDanceabilityJson, lang, 'dance'))}</td>`;
      case 'instrument': return `<td>${escapeHtml(analysisLabelsForTable(track.instrumentJson, lang, INSTRUMENT_LABELS))}</td>`;
      case 'acousticElectronic': return `<td>${escapeHtml(analysisLabelsForTable(track.styleJson, lang, ACOUSTIC_ELECTRONIC_LABELS, (label) => label === 'acoustic' || label === 'electronic'))}</td>`;
      case 'lyrics': return `<td>${track.lyricPlainText || track.lyricTranslatedText || track.lyricRomanizedText || track.lyricLrcText ? (lang === 'zh' ? '有' : 'Yes') : '—'}</td>`;
      case 'updated': return `<td>${track.updatedAtMs > 0 ? escapeHtml(new Date(track.updatedAtMs).toLocaleDateString(lang === 'zh' ? 'zh-CN' : 'en-US')) : '—'}</td>`;
      case 'aliases': return `<td>${escapeHtml(track.aliasesJson || '—')}</td>`;
      default: return '<td>—</td>';
    }
  };
  return page.items.map((track) => `
    <tr data-action="library-track-detail" data-track-key="${escapeHtml(track.trackKey)}">
      ${columns.map((column) => cell(column, track)).join('')}
    </tr>
  `).join('');
}

function lyricsForTab(track: LibraryTrack, tab: LibraryLyricsTab): string {
  return tab === 'translated' ? track.lyricTranslatedText
    : tab === 'romanized' ? track.lyricRomanizedText
      : tab === 'lrc' ? track.lyricLrcText : track.lyricPlainText;
}

function renderDetail(
  track: LibraryTrack | null,
  lang: 'zh' | 'en',
  tab: LibraryLyricsTab,
  search: string,
  sourceRecords: LibrarySourceRecord[] = [],
): string {
  if (!track) return '';
  const mood = track.moodJson && track.moodJson !== '[]' ? track.moodJson : '—';
  const instruments = track.instrumentJson && track.instrumentJson !== '[]' ? track.instrumentJson : '—';
  const lyrics = lyricsForTab(track, tab);
  const normalizedSearch = search.trim().toLocaleLowerCase();
  const visibleLyrics = normalizedSearch
    ? lyrics.split('\n').filter((line) => line.toLocaleLowerCase().includes(normalizedSearch)).join('\n')
    : lyrics;
  const tabs: Array<[LibraryLyricsTab, string]> = [
    ['plain', lang === 'zh' ? '原文' : 'Original'],
    ['translated', lang === 'zh' ? '翻译' : 'Translation'],
    ['romanized', lang === 'zh' ? '罗马音' : 'Romanized'],
    ['lrc', 'LRC'],
  ];
  return `
    <aside class="library-detail-drawer" data-role="library-detail">
      <header><div><p class="panel-kicker">W4DJ RKB</p><h3>${escapeHtml(track.title || '—')}</h3></div>
        <button class="secondary-action" type="button" data-action="close-library-detail">${lang === 'zh' ? '关闭' : 'Close'}</button>
      </header>
      <section class="library-detail-section"><h4>${lang === 'zh' ? '标准化信息' : 'Normalized information'}</h4><dl class="library-detail-grid">
        <div><dt>${lang === 'zh' ? '歌手' : 'Artists'}</dt><dd>${escapeHtml(track.artists || '—')}</dd></div>
        <div><dt>${lang === 'zh' ? '专辑' : 'Album'}</dt><dd>${escapeHtml(track.album || '—')}</dd></div>
        <div><dt>${lang === 'zh' ? '网易云 Genre' : 'NetEase Genre'}</dt><dd>${escapeHtml(track.neteaseGenre || '—')}</dd></div>
        <div><dt>${lang === 'zh' ? '别名' : 'Aliases'}</dt><dd>${escapeHtml(track.aliasesJson || '—')}</dd></div>
        <div><dt>${lang === 'zh' ? '版权' : 'Copyright'}</dt><dd>${escapeHtml(track.copyrightText || '—')}</dd></div>
        <div><dt>${lang === 'zh' ? '发布日期' : 'Publish date'}</dt><dd>${escapeHtml(track.publishDate || '—')}</dd></div>
      </dl></section>
      <section class="library-detail-section"><h4>${lang === 'zh' ? '本地文件实测' : 'Measured local file'}</h4><dl class="library-detail-grid">
        <div><dt>${lang === 'zh' ? '格式' : 'Format'}</dt><dd>${escapeHtml(track.effectiveFormat || '—')}</dd></div>
        <div><dt>${lang === 'zh' ? '大小' : 'Size'}</dt><dd>${formatBytes(track.effectiveSizeBytes, lang)}</dd></div>
        <div><dt>${lang === 'zh' ? '时长' : 'Duration'}</dt><dd>${formatDuration(track.effectiveDurationSeconds)}</dd></div>
        <div><dt>${lang === 'zh' ? '平均码率' : 'Bitrate'}</dt><dd>${formatNumber(track.effectiveBitrateBps == null ? null : track.effectiveBitrateBps / 1000, ' kbps')}</dd></div>
      </dl></section>
      <section class="library-detail-section"><h4>${lang === 'zh' ? 'Essentia 分析' : 'Essentia analysis'}</h4><dl class="library-detail-grid">
        <div><dt>${lang === 'zh' ? 'Essentia Genre' : 'Essentia Genre'}</dt><dd>${escapeHtml(track.essentiaGenre || '—')}</dd></div>
        <div><dt>BPM / Key</dt><dd>${track.bpm == null ? '—' : `${track.bpm.toFixed(2)} / ${escapeHtml(track.musicalKey || '—')} ${escapeHtml(track.scale || '')}`}</dd></div>
        <div><dt>LUFS / Energy</dt><dd>${formatNumber(track.integratedLoudnessLufs, ' LUFS')} / ${renderEnergyRating(track.energy)}</dd></div>
        <div><dt>${lang === 'zh' ? '可舞性（10级）' : 'Danceability (10-level)'}</dt><dd>${renderDanceabilityRating(track.danceability)}</dd></div>
        <div><dt>${lang === 'zh' ? '情绪' : 'Mood'}</dt><dd>${escapeHtml(mood)}</dd></div>
        <div><dt>${lang === 'zh' ? '器乐/人声' : 'Instrument / Vocal'}</dt><dd>${escapeHtml(instruments)}</dd></div>
        <div><dt>${lang === 'zh' ? '原生乐器/电子' : 'Acoustic / Electronic'}</dt><dd>${escapeHtml(analysisLabelsForTable(track.styleJson, lang, ACOUSTIC_ELECTRONIC_LABELS, (label) => label === 'acoustic' || label === 'electronic'))}</dd></div>
        <div><dt>${lang === 'zh' ? 'Drop LUFS' : 'Drop LUFS'}</dt><dd>${formatNumber(track.dropLoudnessLufs, ' LUFS')}</dd></div>
      </dl></section>
      <section class="library-detail-section"><h4>Discogs-EffNet</h4><dl class="library-detail-grid">
        <div><dt>${lang === 'zh' ? 'Mood / Theme' : 'Mood / Theme'}</dt><dd>${escapeHtml(discogsCell(track.discogsMoodThemeJson, lang, 'labels'))}</dd></div>
        <div><dt>${lang === 'zh' ? '可接近度' : 'Approachability'}</dt><dd>${escapeHtml(discogsCell(track.discogsApproachabilityJson, lang, 'class'))}</dd></div>
        <div><dt>${lang === 'zh' ? '乐器' : 'Instrumentation'}</dt><dd>${escapeHtml(discogsCell(track.discogsInstrumentationJson, lang, 'labels'))}</dd></div>
        <div><dt>${lang === 'zh' ? '音色' : 'Timbre'}</dt><dd>${escapeHtml(discogsCell(track.discogsTimbreJson, lang, 'class'))}</dd></div>
        <div><dt>${lang === 'zh' ? 'Discogs 可舞性' : 'Discogs Danceability'}</dt><dd>${escapeHtml(discogsCell(track.discogsDanceabilityJson, lang, 'dance'))}</dd></div>
      </dl></section>
      <section class="library-detail-section library-raw-records"><h4>${lang === 'zh' ? '网易云原始信息' : 'Raw NetEase information'}</h4>
        ${sourceRecords.length ? `<p>${sourceRecords.length} ${lang === 'zh' ? '条来源记录' : 'source records'}</p>` : `<p>${lang === 'zh' ? '没有原始来源记录' : 'No source records'}</p>`}
        ${sourceRecords.length ? `<details><summary>${lang === 'zh' ? '查看完整 JSON（可能包含本机路径）' : 'View full JSON (may contain local paths)'}</summary>${sourceRecords.map((record) => `<article><strong>${escapeHtml(record.sourceTable)} · ${escapeHtml(record.sourcePrimaryKey)}</strong><pre>${escapeHtml(record.rawJson)}</pre></article>`).join('')}</details>` : ''}
      </section>
      <section class="library-lyrics"><div class="library-lyrics-head"><h4>${lang === 'zh' ? '歌词' : 'Lyrics'}</h4>
        <div class="library-lyrics-actions"><button class="secondary-action" type="button" data-action="library-copy-lyrics">${lang === 'zh' ? '复制' : 'Copy'}</button><button class="secondary-action" type="button" data-action="library-download-lyrics" ${track.lyricLrcText ? '' : 'disabled'}>${lang === 'zh' ? '下载 LRC' : 'Download LRC'}</button></div>
      </div>
      <div class="library-lyrics-tabs">${tabs.map(([value, text]) => `<button class="${tab === value ? 'selected' : ''}" type="button" data-action="library-lyrics-tab" data-lyrics-tab="${value}">${text}</button>`).join('')}</div>
      <input class="library-lyrics-search" data-action="library-lyrics-search" value="${escapeHtml(search)}" placeholder="${lang === 'zh' ? '搜索当前歌词版本' : 'Search this lyrics view'}" />
        <pre>${escapeHtml(visibleLyrics || (lang === 'zh' ? '没有歌词记录' : 'No lyrics recorded'))}</pre>
      </section>
    </aside>
  `;
}

export function renderLibraryDashboard(
  state: LibraryDashboardState | null,
  lang: 'zh' | 'en',
): string {
  if (!state?.visible) return '';
  const page = state.page;
  const status = state.status;
  const pageStart = page ? page.offset + 1 : 0;
  const pageEnd = page ? Math.min(page.total, page.offset + page.items.length) : 0;
  const totalPages = page ? Math.max(1, Math.ceil(page.total / Math.max(1, page.limit))) : 1;
  const currentPage = page ? Math.floor(page.offset / Math.max(1, page.limit)) + 1 : 1;
  const analyzedTrackCount = status?.analyzedTrackCount ?? status?.trackCount ?? 0;
  const totalTrackCount = status?.totalTrackCount ?? status?.trackCount ?? 0;
  const availableTrackCount = status?.availableTrackCount ?? 0;
  const invalidTrackCount = status?.invalidTrackCount ?? 0;
  const notAnalyzedCount = status?.notAnalyzedCount ?? 0;
  const analysisFailedCount = status?.analysisFailedCount ?? analyzedTrackCount;
  const analysisCompletedCount = status?.analysisCompletedCount ?? analyzedTrackCount;
  const invalidScan = status?.invalidScan;
  const columns = visibleLibraryColumns();
  const columnSettings = loadLibraryColumnSettings();
  const columnGroup = `<colgroup>${columns.map((column) => {
    const width = columnSettings.widths[column.id];
    return `<col data-library-column="${column.id}"${width ? ` style="width:${width}px"` : ''} />`;
  }).join('')}</colgroup>`;
  const columnHeaders = columns.map((column) => {
    const width = columnSettings.widths[column.id];
    const widthStyle = width ? ` style="width:${width}px"` : '';
    if (!column.field) {
      return `<th draggable="true" data-library-column-header="${column.id}"${widthStyle}>${column.label}<span class="library-column-resizer" data-library-column-resizer="${column.id}" aria-hidden="true"></span></th>`;
    }
    const sortIndex = state.query.sorts.findIndex((sort) => sort.field === column.field);
    const sort = sortIndex >= 0 ? state.query.sorts[sortIndex] : null;
    const indicator = sort ? ` <span class="library-sort-indicator" aria-label="${sortIndex + 1}">${sort.direction === 'asc' ? '▲' : '▼'}${state.query.sorts.length > 1 ? `<sup>${sortIndex + 1}</sup>` : ''}</span>` : '';
    return `<th draggable="true" data-library-column-header="${column.id}"${widthStyle} aria-sort="${sort ? (sort.direction === 'asc' ? 'ascending' : 'descending') : 'none'}"><button class="library-sort-button" type="button" data-action="library-sort" data-library-field="${column.field}">${column.label}${indicator}</button><span class="library-column-resizer" data-library-column-resizer="${column.id}" aria-hidden="true"></span></th>`;
  }).join('');
  const filterField = state.query.filters[0]?.field || 'title';
  const selectedOperator = state.query.filters[0]?.operator;
  const selectedFilter = state.query.filters[0];
  const operatorOptions = libraryOperatorsForField(filterField).map((operator) => `<option value="${operator}" ${selectedOperator === operator ? 'selected' : ''}>${operator}</option>`).join('');
  return `
    <div class="library-modal" data-role="library-modal" role="dialog" aria-modal="true" aria-label="${lang === 'zh' ? '歌曲库' : 'Song library'}">
      <section class="library-dialog">
        <header class="library-head"><div><p class="panel-kicker">W4DJ RKB</p><h2>${lang === 'zh' ? '歌曲库' : 'Song library'}</h2>
          <p>${analyzedTrackCount > 0 ? `${lang === 'zh' ? 'W4DJ 分析库' : 'W4DJ analysis library'} · ${analyzedTrackCount} ${lang === 'zh' ? '首' : 'tracks'}` : (lang === 'zh' ? '尚未有完成分析的歌曲' : 'No completed analyses yet')}</p>
        </div><button class="secondary-action" type="button" data-action="close-library">${lang === 'zh' ? '关闭' : 'Close'}</button></header>
        <div class="library-toolbar">
          <span class="library-stats" data-role="library-stats">${lang === 'zh'
            ? `总歌曲 ${totalTrackCount} · 可用 ${availableTrackCount} · 失效 ${invalidTrackCount} · 未分析 ${notAnalyzedCount} · 分析失败 ${analysisFailedCount} · 已完成 ${analysisCompletedCount}`
            : `Total ${totalTrackCount} · Available ${availableTrackCount} · Invalid ${invalidTrackCount} · Not analyzed ${notAnalyzedCount} · Failed ${analysisFailedCount} · Completed ${analysisCompletedCount}`}</span>
          <button class="secondary-action" type="button" data-action="${invalidScan && (invalidScan.status === 'running' || invalidScan.status === 'cancelling') ? 'cancel-invalid-scan' : 'find-invalid-library'}" ${state.busy && !(invalidScan && invalidScan.status === 'running') ? 'disabled' : ''}>${invalidScan?.status === 'cancelling' ? (lang === 'zh' ? '正在取消…' : 'Cancelling…') : invalidScan?.status === 'running' ? (lang === 'zh' ? '取消失效扫描' : 'Cancel scan') : (lang === 'zh' ? '批量寻找失效歌曲' : 'Find invalid songs')}</button>
          <input id="library-search" type="search" data-action="library-search" value="${escapeHtml(state.query.text)}" aria-label="${lang === 'zh' ? '搜索歌曲' : 'Search songs'}" placeholder="${lang === 'zh' ? '搜索歌曲名、歌手、专辑、别名或 Genre' : 'Search title, artist, album, alias or genre'}" />
          <button class="secondary-action" type="button" data-action="reanalyze-library" ${state.busy ? 'disabled' : ''}>${lang === 'zh' ? '重新分析歌曲库' : 'Reanalyze library'}</button>
          <button class="global-action" type="button" data-action="search-library">${lang === 'zh' ? '搜索' : 'Search'}</button>
          <label class="library-invalid-confirm"><input type="checkbox" data-action="library-confirm-clear-invalid" ${state.confirmClearInvalid ? 'checked' : ''} /><span>${lang === 'zh' ? '确认清除失效文件' : 'Confirm clearing missing files'}</span></label>
          <button class="secondary-action" type="button" data-action="clear-invalid-library" ${state.confirmClearInvalid && !state.busy ? '' : 'disabled'}>${lang === 'zh' ? '清除所有失效文件' : 'Clear invalid files'}</button>
        </div>
        ${invalidScan && invalidScan.status !== 'idle' ? `<p class="library-scan-progress" data-role="library-scan-progress">${escapeHtml(invalidScan.message)} ${invalidScan.total ? `${invalidScan.processed}/${invalidScan.total}` : ''}${invalidScan.currentItem ? ` · ${escapeHtml(invalidScan.currentItem)}` : ''}</p>` : ''}
        <div class="library-filter-bar">
          <span class="library-filter-label">${lang === 'zh' ? '匹配字段' : 'Match fields'}</span>
          <select data-action="library-filter-field"><option value="title" ${filterField === 'title' ? 'selected' : ''}>${lang === 'zh' ? '歌曲名' : 'Title'}</option><option value="artists" ${filterField === 'artists' ? 'selected' : ''}>${lang === 'zh' ? '歌手' : 'Artists'}</option><option value="album" ${filterField === 'album' ? 'selected' : ''}>${lang === 'zh' ? '专辑' : 'Album'}</option><option value="bpm" ${filterField === 'bpm' ? 'selected' : ''}>BPM</option><option value="energy" ${filterField === 'energy' ? 'selected' : ''}>${lang === 'zh' ? '能量原始值' : 'Energy raw value'}</option><option value="danceability" ${filterField === 'danceability' ? 'selected' : ''}>${lang === 'zh' ? '可舞性原始值' : 'Danceability raw value'}</option><option value="discogs_mood_theme" ${filterField === 'discogs_mood_theme' ? 'selected' : ''}>Discogs Mood/Theme</option><option value="discogs_approachability" ${filterField === 'discogs_approachability' ? 'selected' : ''}>Approachability</option><option value="discogs_instrumentation" ${filterField === 'discogs_instrumentation' ? 'selected' : ''}>Discogs Instrumentation</option><option value="discogs_timbre" ${filterField === 'discogs_timbre' ? 'selected' : ''}>Timbre</option><option value="discogs_danceability" ${filterField === 'discogs_danceability' ? 'selected' : ''}>Discogs Danceability</option><option value="netease_genre" ${filterField === 'netease_genre' ? 'selected' : ''}>NetEase Genre</option><option value="essentia_genre" ${filterField === 'essentia_genre' ? 'selected' : ''}>Essentia Genre</option><option value="cover_available" ${filterField === 'cover_available' ? 'selected' : ''}>${lang === 'zh' ? '有封面' : 'Cover available'}</option></select>
          <select data-action="library-filter-operator">${operatorOptions}</select>
          <input data-action="library-filter-value" value="${escapeHtml(selectedFilter?.value || '')}" type="${['bpm', 'bitrate', 'file_size', 'duration', 'energy', 'danceability', 'loudness', 'updated_at'].includes(filterField) ? 'number' : 'text'}" placeholder="${lang === 'zh' ? '筛选值' : 'Filter value'}" />
          ${selectedOperator === 'between' ? `<input data-action="library-filter-second-value" value="${escapeHtml(selectedFilter?.secondValue || '')}" type="number" placeholder="${lang === 'zh' ? '上限' : 'Upper bound'}" />` : ''}
          <button class="secondary-action" type="button" data-action="library-apply-filter">${lang === 'zh' ? '应用筛选' : 'Apply filter'}</button>
          <button class="secondary-action" type="button" data-action="library-clear-filters">${lang === 'zh' ? '清除筛选' : 'Clear filters'}</button>
        </div>
        ${state.error ? `<p class="library-error">${escapeHtml(state.error)}</p>` : ''}
        ${state.notice ? `<p class="library-notice">${escapeHtml(state.notice)}</p>` : ''}
        ${!page && analyzedTrackCount === 0 ? `<div class="library-empty-state"><h3>${lang === 'zh' ? '还没有完成分析的歌曲' : 'No completed analyses yet'}</h3><p>${lang === 'zh' ? '请在增强模式中完成歌曲分析，结果会自动写入 W4DJ 歌曲库。' : 'Complete an Enhanced mode analysis; its results will be added to the W4DJ library automatically.'}</p></div>` : ''}
        ${page ? `<div class="library-table-wrap"><table class="library-table">${columnGroup}<thead><tr>${columnHeaders}
        </tr></thead><tbody>${trackRows(page, lang, state.coverData || {}, columns)}</tbody></table></div>
        <footer class="library-pagination"><span>${page.total ? `${pageStart}–${pageEnd} / ${page.total}` : (lang === 'zh' ? '0 首' : '0 tracks')}</span><button class="secondary-action" data-action="library-prev" ${currentPage <= 1 ? 'disabled' : ''}>${lang === 'zh' ? '上一页' : 'Previous'}</button><span>${currentPage} / ${totalPages}</span><button class="secondary-action" data-action="library-next" ${currentPage >= totalPages ? 'disabled' : ''}>${lang === 'zh' ? '下一页' : 'Next'}</button></footer>` : ''}
        ${renderDetail(state.detail, lang, state.lyricsTab || 'plain', state.lyricsSearch || '', state.sourceRecords || [])}
        ${state.contextMenu ? `<div class="library-context-menu" data-role="library-context-menu" style="left:${Math.max(8, state.contextMenu.x)}px;top:${Math.max(8, state.contextMenu.y)}px"><button type="button" data-action="relocate-library-track" data-track-key="${escapeHtml(state.contextMenu.trackKey)}">${lang === 'zh' ? '重新定位文件' : 'Relocate file'}</button><button type="button" data-action="remove-library-track" data-track-key="${escapeHtml(state.contextMenu.trackKey)}">${lang === 'zh' ? '移除记录' : 'Remove record'}</button></div>` : ''}
      </section>
    </div>
  `;
}
