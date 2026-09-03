export type DjPlaylistReviewCandidate = {
  trackKey: string;
  title: string;
  artistDisplay: string;
  destinationFilename: string;
  score: number;
  reason: string;
};

export type DjPlaylistReviewRow = {
  position: number;
  title: string;
  artistDisplay: string;
  status: 'matched' | 'unmatched' | 'ambiguous' | 'missing';
  trackKey: string | null;
  destinationPath: string | null;
  score: number | null;
  reason: string;
  candidateSource: string | null;
  manual: boolean;
  /** Removed from this playlist's export list, without touching the library. */
  excluded: boolean;
  candidates: DjPlaylistReviewCandidate[];
};

export type DjPlaylistReviewReport = {
  total: number;
  matchedCount: number;
  unmatchedCount: number;
  missingCount: number;
  matches: DjPlaylistReviewRow[];
};

export type DjPlaylistReviewCopy = {
  title: string;
  hint: string;
  playlistColumn: string;
  outputColumn: string;
  emptyOutput: string;
  chooseLocal: string;
  recent: string;
  library: string;
  manual: string;
  unmatched: string;
  matchScore: string;
  selectAll: string;
  clearSelection: string;
  selected: string;
  deleteSelected: string;
  deleteRow: string;
  restoreExcluded: string;
  excludedSummary: string;
  emptyList: string;
  selectRow: string;
  exportCopy: string;
  exportExisting: string;
  exportDisabled: string;
  busy: string;
};

export function canExportReviewedPlaylist(report: DjPlaylistReviewReport | null | undefined): boolean {
  const activeRows = report?.matches.filter((row) => !row.excluded) || [];
  return Boolean(
    report
      && report.total > 0
      && report.matches.length === report.total
      && activeRows.length > 0
      && activeRows.every((row) => (
        row.status === 'matched'
        && Boolean(row.trackKey?.trim())
        && Boolean(row.destinationPath?.trim())
      )),
  );
}

export function renderDjPlaylistReview(
  report: DjPlaylistReviewReport | null,
  copy: DjPlaylistReviewCopy,
  busy = false,
  selectedPositions: ReadonlySet<number> = new Set(),
): string {
  if (!report) {
    return `<section class="dj-playlist-review" data-role="dj-playlist-review"><p>${escapeHtml(copy.hint)}</p></section>`;
  }
  const ready = canExportReviewedPlaylist(report);
  const rows = [...report.matches]
    .filter((row) => !row.excluded)
    .sort((left, right) => left.position - right.position);
  const excludedCount = report.matches.length - rows.length;
  const matchedCount = rows.filter((row) => row.status === 'matched').length;
  const selectedCount = rows.filter((row) => selectedPositions.has(row.position)).length;
  const allSelected = rows.length > 0 && selectedCount === rows.length;
  return `
    <section class="dj-playlist-review" data-role="dj-playlist-review">
      <header class="dj-playlist-review-head">
        <div>
          <h2>${escapeHtml(copy.title)}</h2>
          <p>${escapeHtml(copy.hint)}</p>
        </div>
        <strong class="dj-playlist-review-count">${matchedCount}/${rows.length}</strong>
      </header>
      <div class="dj-playlist-review-toolbar">
        <label class="dj-playlist-review-select-all">
          <input type="checkbox" data-action="dj-playlist-select-all" ${allSelected ? 'checked' : ''} ${rows.length > 0 && !busy ? '' : 'disabled'}>
          <span>${escapeHtml(allSelected ? copy.clearSelection : copy.selectAll)}</span>
        </label>
        <span class="dj-playlist-review-selection" aria-live="polite">${escapeHtml(copy.selected.replace('{count}', String(selectedCount)))}</span>
        <button type="button" class="secondary-action dj-playlist-review-bulk-delete" data-action="dj-playlist-delete-selected" ${selectedCount > 0 && !busy ? '' : 'disabled'}>${escapeHtml(copy.deleteSelected)}</button>
        ${excludedCount > 0
          ? `<span class="dj-playlist-review-excluded" aria-live="polite">${escapeHtml(copy.excludedSummary.replace('{count}', String(excludedCount)))}</span>
             <button type="button" class="secondary-action dj-playlist-review-restore" data-action="dj-playlist-restore-excluded" ${busy ? 'disabled' : ''}>${escapeHtml(copy.restoreExcluded)}</button>`
          : ''}
      </div>
      <div class="dj-playlist-review-grid" role="table" aria-label="${escapeHtml(copy.title)}">
        <div class="dj-playlist-review-heading" role="columnheader">${escapeHtml(copy.playlistColumn)}</div>
        <div class="dj-playlist-review-heading" role="columnheader">${escapeHtml(copy.outputColumn)}</div>
        ${rows.length > 0
          ? rows.map((row) => renderReviewRow(row, copy, selectedPositions, busy)).join('')
          : `<p class="dj-playlist-review-empty-list" role="cell">${escapeHtml(copy.emptyList)}</p>`}
      </div>
      <p class="dj-playlist-review-disabled" ${ready ? 'hidden' : ''}>${escapeHtml(copy.exportDisabled)}</p>
      <footer class="dj-playlist-export-choice-actions">
        <button type="button" class="global-action" data-action="dj-playlist-export-copy" ${ready && !busy ? '' : 'disabled'}>${busy ? escapeHtml(copy.busy) : escapeHtml(copy.exportCopy)}</button>
        <button type="button" class="secondary-action" data-action="dj-playlist-export-existing" ${ready && !busy ? '' : 'disabled'}>${escapeHtml(copy.exportExisting)}</button>
      </footer>
    </section>
  `;
}

function renderReviewRow(
  row: DjPlaylistReviewRow,
  copy: DjPlaylistReviewCopy,
  selectedPositions: ReadonlySet<number>,
  busy: boolean,
): string {
  const hasOutput = Boolean(row.destinationPath?.trim());
  const filename = row.destinationPath?.split(/[\\/]/).pop() || row.trackKey || copy.emptyOutput;
  const source = row.candidateSource === 'recent'
    ? copy.recent
    : row.candidateSource === 'manual' || row.manual
      ? copy.manual
      : copy.library;
  const matchDetails = hasOutput
    ? [source, row.score === null ? '' : `${copy.matchScore} ${row.score}%`].filter(Boolean).join(' · ')
    : copy.unmatched;
  const selectLabel = copy.selectRow.replace('{position}', String(row.position));
  return `
    <div class="dj-playlist-review-cell dj-playlist-review-playlist" role="cell">
      <label class="dj-playlist-review-row-select" title="${escapeHtml(selectLabel)}">
        <input type="checkbox" data-action="dj-playlist-select-row" data-position="${row.position}" aria-label="${escapeHtml(selectLabel)}" ${selectedPositions.has(row.position) ? 'checked' : ''} ${busy ? 'disabled' : ''}>
      </label>
      <span class="dj-playlist-review-position">${row.position}</span>
      <span><strong>${escapeHtml(row.title)}</strong><small>${escapeHtml(row.artistDisplay)}</small></span>
    </div>
    <div class="dj-playlist-review-cell dj-playlist-review-output" role="cell">
      ${hasOutput
        ? `<button type="button" class="dj-playlist-review-file" data-action="open-dj-playlist-local" data-path="${escapeHtml(row.destinationPath || '')}" title="${escapeHtml(row.destinationPath || '')}">${escapeHtml(filename)}</button>
           <small>${escapeHtml(matchDetails)}</small>`
        : `<span class="dj-playlist-review-empty">${escapeHtml(copy.emptyOutput)}</span>
           <small>${escapeHtml(matchDetails)}</small>`}
      <div class="dj-playlist-review-actions">
        <button type="button" class="secondary-action" data-action="dj-playlist-pick-local" data-position="${row.position}" ${busy ? 'disabled' : ''}>${escapeHtml(copy.chooseLocal)}</button>
        <button type="button" class="secondary-action dj-playlist-review-delete" data-action="dj-playlist-delete-row" data-position="${row.position}" ${busy ? 'disabled' : ''}>${escapeHtml(copy.deleteRow)}</button>
      </div>
    </div>
  `;
}

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}
