export type LibraryLocalStatus = 'available' | 'missing' | 'unreadable' | 'database_only';

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
  | 'danceability' | 'cover_available' | 'lyrics' | 'updated_at';

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

export type LibraryStatus = {
  catalogPath: string;
  trackCount: number;
  netease: NeteaseDiscovery;
};

export type LibraryRefreshSummary = {
  trackCount: number;
  localFileCount: number;
  readableFileCount: number;
  reusedFileCount: number;
  databasePath: string;
  musicFolder: string | null;
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
};

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
  { id: 'neteaseGenre', field: 'netease_genre', label: '网易云 Genre' },
  { id: 'essentiaGenre', field: 'essentia_genre', label: 'Essentia Genre' },
  { id: 'bpmKey', field: 'bpm', label: 'BPM / Key' },
  { id: 'loudness', field: 'loudness', label: '响度' },
  { id: 'energy', field: 'energy', label: '能量' },
  { id: 'lyrics', field: 'lyrics', label: '歌词' },
  { id: 'updated', field: 'updated_at', label: '最后更新' },
  { id: 'aliases', label: '别名' },
  { id: 'copyright', label: '版权' },
  { id: 'publishDate', label: '发布日期' },
];

const LIBRARY_COLUMN_STORAGE_KEY = 'w4dj.libraryDashboard.columns.v1';

function visibleLibraryColumns(): LibraryColumn[] {
  const fallback = LIBRARY_COLUMNS.map((column) => column.id);
  let stored: { order: string[]; hidden: string[] } = { order: fallback, hidden: [] };
  try {
    const parsed = JSON.parse(localStorage.getItem(LIBRARY_COLUMN_STORAGE_KEY) || 'null');
    if (Array.isArray(parsed)) {
      stored = { order: parsed.filter((value): value is string => typeof value === 'string'), hidden: [] };
    } else if (parsed && typeof parsed === 'object') {
      stored = {
        order: Array.isArray(parsed.order) ? parsed.order.filter((value: unknown): value is string => typeof value === 'string') : fallback,
        hidden: Array.isArray(parsed.hidden) ? parsed.hidden.filter((value: unknown): value is string => typeof value === 'string') : [],
      };
    }
  } catch {
    // Use defaults when a previous preference is malformed.
  }
  const order = stored.order;
  const ids = [...new Set([...order, ...fallback])];
  const columns = ids
    .map((id) => LIBRARY_COLUMNS.find((column) => column.id === id))
    .filter((column): column is LibraryColumn => Boolean(column));
  const hidden = new Set(stored.hidden);
  return columns.filter((column) => column.alwaysVisible || !hidden.has(column.id));
}

export function saveLibraryColumnOrder(order: string[]): void {
  try {
    const existing = JSON.parse(localStorage.getItem(LIBRARY_COLUMN_STORAGE_KEY) || '{}');
    localStorage.setItem(LIBRARY_COLUMN_STORAGE_KEY, JSON.stringify({
      order,
      hidden: Array.isArray(existing?.hidden) ? existing.hidden : [],
    }));
  } catch {
    // Rendering the dashboard must continue in private browsing / test DOMs.
  }
}

export function toggleLibraryColumn(columnId: string): void {
  if (!LIBRARY_COLUMNS.some((column) => column.id === columnId && !column.alwaysVisible)) return;
  try {
    const parsed = JSON.parse(localStorage.getItem(LIBRARY_COLUMN_STORAGE_KEY) || '{}');
    const hidden = new Set<string>(Array.isArray(parsed?.hidden) ? parsed.hidden : []);
    if (hidden.has(columnId)) hidden.delete(columnId); else hidden.add(columnId);
    saveLibraryColumnOrder(Array.isArray(parsed?.order) ? parsed.order : libraryColumnIds());
    const updated = JSON.parse(localStorage.getItem(LIBRARY_COLUMN_STORAGE_KEY) || '{}');
    localStorage.setItem(LIBRARY_COLUMN_STORAGE_KEY, JSON.stringify({ ...updated, hidden: [...hidden] }));
  } catch {
    // Ignore storage failures; the next render uses defaults.
  }
}

export function libraryColumnIds(): string[] {
  return LIBRARY_COLUMNS.map((column) => column.id);
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
      missing: 'Missing',
      unreadable: 'Unreadable',
      database_only: 'Database only',
    }[value] || value;
  }
  return {
    available: '本地可用',
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
      case 'neteaseGenre': return `<td>${escapeHtml(track.neteaseGenre || '—')}</td>`;
      case 'essentiaGenre': return `<td>${escapeHtml(track.essentiaGenre || '—')}</td>`;
      case 'bpmKey': return `<td>${track.bpm == null ? '—' : `${track.bpm.toFixed(1)} · ${escapeHtml(track.musicalKey || '')}`}</td>`;
      case 'loudness': return `<td>${formatNumber(track.integratedLoudnessLufs, ' LUFS')}</td>`;
      case 'energy': return `<td>${formatNumber(track.energy)}</td>`;
      case 'lyrics': return `<td>${track.lyricPlainText || track.lyricTranslatedText || track.lyricRomanizedText || track.lyricLrcText ? (lang === 'zh' ? '有' : 'Yes') : '—'}</td>`;
      case 'updated': return `<td>${track.updatedAtMs > 0 ? escapeHtml(new Date(track.updatedAtMs).toLocaleDateString(lang === 'zh' ? 'zh-CN' : 'en-US')) : '—'}</td>`;
      case 'aliases': return `<td>${escapeHtml(track.aliasesJson || '—')}</td>`;
      case 'copyright': return `<td>${escapeHtml(track.copyrightText || '—')}</td>`;
      case 'publishDate': return `<td>${escapeHtml(track.publishDate || '—')}</td>`;
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
        <div><dt>LUFS / Energy</dt><dd>${formatNumber(track.integratedLoudnessLufs, ' LUFS')} / ${formatNumber(track.energy)}</dd></div>
        <div><dt>${lang === 'zh' ? '情绪' : 'Mood'}</dt><dd>${escapeHtml(mood)}</dd></div>
        <div><dt>${lang === 'zh' ? '器乐/人声' : 'Instrument / Vocal'}</dt><dd>${escapeHtml(instruments)}</dd></div>
        <div><dt>${lang === 'zh' ? 'Drop LUFS' : 'Drop LUFS'}</dt><dd>${formatNumber(track.dropLoudnessLufs, ' LUFS')}</dd></div>
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
  const columns = visibleLibraryColumns();
  const columnHeaders = columns.map((column) => column.field
    ? `<th draggable="true" data-library-column-header="${column.id}"><button class="library-sort-button" type="button" data-action="library-sort" data-library-field="${column.field}">${column.label}</button></th>`
    : `<th draggable="true" data-library-column-header="${column.id}">${column.label}</th>`).join('');
  return `
    <div class="library-modal" data-role="library-modal" role="dialog" aria-modal="true" aria-label="${lang === 'zh' ? '歌曲库' : 'Song library'}">
      <section class="library-dialog">
        <header class="library-head"><div><p class="panel-kicker">W4DJ RKB</p><h2>${lang === 'zh' ? '歌曲库' : 'Song library'}</h2>
          <p>${status?.netease.databasePath ? `${lang === 'zh' ? '已连接网易云数据库' : 'NetEase database connected'} · ${status.trackCount} ${lang === 'zh' ? '首' : 'tracks'}` : (lang === 'zh' ? '尚未建立本地歌曲索引' : 'Local song index is empty')}</p>
        </div><button class="secondary-action" type="button" data-action="close-library">${lang === 'zh' ? '关闭' : 'Close'}</button></header>
        <div class="library-toolbar">
          <input data-action="library-search" value="${escapeHtml(state.query.text)}" placeholder="${lang === 'zh' ? '搜索歌曲名、歌手、专辑或 Genre' : 'Search title, artist, album or genre'}" />
          <button class="global-action" type="button" data-action="refresh-library" ${state.busy ? 'disabled' : ''}>${state.busy ? (lang === 'zh' ? '正在更新…' : 'Updating…') : (lang === 'zh' ? '更新歌曲库' : 'Refresh library')}</button>
          <button class="secondary-action" type="button" data-action="clear-library-cache" ${state.busy ? 'disabled' : ''}>${lang === 'zh' ? '清除歌曲库缓存' : 'Clear library cache'}</button>
        </div>
        <div class="library-filter-bar">
          <select data-action="library-filter-field"><option value="title">${lang === 'zh' ? '歌曲名' : 'Title'}</option><option value="artists">${lang === 'zh' ? '歌手' : 'Artists'}</option><option value="album">${lang === 'zh' ? '专辑' : 'Album'}</option><option value="bpm">BPM</option><option value="netease_genre">NetEase Genre</option><option value="essentia_genre">Essentia Genre</option><option value="cover_available">${lang === 'zh' ? '有封面' : 'Cover available'}</option></select>
          <select data-action="library-filter-operator"><option value="contains">${lang === 'zh' ? '包含' : 'Contains'}</option><option value="is">${lang === 'zh' ? '等于' : 'Is'}</option><option value="is_empty">${lang === 'zh' ? '为空' : 'Is empty'}</option><option value="is_not_empty">${lang === 'zh' ? '不为空' : 'Is not empty'}</option><option value="greater_or_equal">≥</option></select>
          <input data-action="library-filter-value" placeholder="${lang === 'zh' ? '筛选值' : 'Filter value'}" />
          <button class="secondary-action" type="button" data-action="library-apply-filter">${lang === 'zh' ? '应用筛选' : 'Apply filter'}</button>
          <button class="secondary-action" type="button" data-action="library-clear-filters">${lang === 'zh' ? '清除筛选' : 'Clear filters'}</button>
        </div>
        <div class="library-column-bar"><span>${lang === 'zh' ? '显示字段' : 'Columns'}</span>${LIBRARY_COLUMNS.filter((column) => !column.alwaysVisible).map((column) => `<button type="button" class="${columns.some((visible) => visible.id === column.id) ? 'selected' : ''}" data-action="library-toggle-column" data-library-column="${column.id}">${column.label}</button>`).join('')}</div>
        ${state.error ? `<p class="library-error">${escapeHtml(state.error)}</p>` : ''}
        ${!page && !status?.netease.databasePath ? `<div class="library-empty-state"><h3>${lang === 'zh' ? '找不到网易云本地数据库' : 'NetEase database not found'}</h3><p>${lang === 'zh' ? '可以继续使用任务来源；点击“更新歌曲库”后重试。' : 'You can keep using the task sources and retry with Refresh library.'}</p></div>` : ''}
        ${page ? `<div class="library-table-wrap"><table class="library-table"><thead><tr>${columnHeaders}
        </tr></thead><tbody>${trackRows(page, lang, state.coverData || {}, columns)}</tbody></table></div>
        <footer class="library-pagination"><span>${page.total ? `${pageStart}–${pageEnd} / ${page.total}` : (lang === 'zh' ? '0 首' : '0 tracks')}</span><button class="secondary-action" data-action="library-prev" ${currentPage <= 1 ? 'disabled' : ''}>${lang === 'zh' ? '上一页' : 'Previous'}</button><span>${currentPage} / ${totalPages}</span><button class="secondary-action" data-action="library-next" ${currentPage >= totalPages ? 'disabled' : ''}>${lang === 'zh' ? '下一页' : 'Next'}</button></footer>` : ''}
        ${renderDetail(state.detail, lang, state.lyricsTab || 'plain', state.lyricsSearch || '', state.sourceRecords || [])}
      </section>
    </div>
  `;
}
