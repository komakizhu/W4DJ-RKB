import { describe, expect, it } from 'vitest';

import {
  canExportReviewedPlaylist,
  renderDjPlaylistReview,
  type DjPlaylistReviewCopy,
  type DjPlaylistReviewReport,
  type DjPlaylistReviewRow,
} from './dj-playlist-review';

const copy: DjPlaylistReviewCopy = {
  title: 'Review playlist',
  hint: 'Remove songs you do not want to export.',
  playlistColumn: 'Playlist song',
  outputColumn: 'Local output',
  emptyOutput: 'No local output',
  chooseLocal: 'Choose local file',
  recent: 'Recent conversion',
  library: 'Existing library',
  manual: 'Manual selection',
  unmatched: 'Unmatched',
  matchScore: 'Match',
  selectAll: 'Select all',
  clearSelection: 'Clear selection',
  selected: '{count} selected',
  deleteSelected: 'Delete selected',
  deleteRow: 'Delete this song',
  restoreExcluded: 'Restore removed',
  excludedSummary: '{count} removed',
  emptyList: 'The export list is empty.',
  selectRow: 'Select song {position}',
  exportCopy: 'Copy audio and export',
  exportExisting: 'Export with existing audio',
  exportDisabled: 'Choose a local file for every song kept in the export list.',
  busy: 'Exporting…',
};

const row = (overrides: Partial<DjPlaylistReviewRow> = {}): DjPlaylistReviewRow => ({
  position: 1,
  title: 'Anchor Point',
  artistDisplay: 'Ahmed Spins',
  status: 'matched',
  trackKey: 'track-1',
  destinationPath: '/music/Anchor Point.mp3',
  score: 88,
  reason: 'Internal matcher detail',
  candidateSource: 'library',
  manual: false,
  excluded: false,
  candidates: [],
  ...overrides,
});

const report = (matches: DjPlaylistReviewRow[], total = matches.length): DjPlaylistReviewReport => ({
  total,
  matchedCount: matches.filter((item) => item.status === 'matched').length,
  unmatchedCount: matches.filter((item) => item.status === 'unmatched').length,
  missingCount: matches.filter((item) => item.status === 'missing').length,
  matches,
});

describe('DJ playlist review', () => {
  it('exports every active bound row without an explicit confirmation flag', () => {
    expect(canExportReviewedPlaylist(report([row()]))).toBe(true);
    expect(canExportReviewedPlaylist(report([row({ excluded: true })]))).toBe(false);
    expect(canExportReviewedPlaylist(report([
      row(),
      row({ position: 2, status: 'unmatched', trackKey: null, destinationPath: null }),
    ]))).toBe(false);
    expect(canExportReviewedPlaylist(report([row(), row({ position: 2, excluded: true })]))).toBe(true);
    expect(canExportReviewedPlaylist(null)).toBe(false);
  });

  it('renders compact source and match score text with row/bulk removal controls', () => {
    const markup = renderDjPlaylistReview(
      report([
        row(),
        row({
          position: 2,
          title: '<Unmatched>',
          artistDisplay: 'Unknown & Co',
          status: 'unmatched',
          trackKey: null,
          destinationPath: null,
          score: null,
          candidateSource: null,
        }),
        row({ position: 3, excluded: true, title: 'Removed song' }),
      ]),
      copy,
      false,
      new Set([1]),
    );

    expect(markup.match(/class="dj-playlist-review-heading"/g)).toHaveLength(2);
    expect(markup.match(/class="dj-playlist-review-cell/g)).toHaveLength(4);
    expect(markup).toContain('Playlist song');
    expect(markup).toContain('Local output');
    expect(markup).toContain('Existing library');
    expect(markup).toContain('Match 88%');
    expect(markup).not.toContain('Internal matcher detail');
    expect(markup).toContain('No local output');
    expect(markup).toContain('&lt;Unmatched&gt;');
    expect(markup).toContain('data-action="dj-playlist-pick-local" data-position="2"');
    expect(markup).toContain('data-action="dj-playlist-select-row" data-position="1"');
    expect(markup).toContain('data-action="dj-playlist-select-all"');
    expect(markup).toContain('data-action="dj-playlist-delete-selected"');
    expect(markup).toContain('data-action="dj-playlist-delete-row" data-position="1"');
    expect(markup).toContain('data-action="dj-playlist-restore-excluded"');
    expect(markup).not.toContain('dj-playlist-confirm-match');
    expect(markup).not.toContain('Confirm row');
    expect(markup).not.toContain('The export list is empty.');
  });

  it('hides excluded rows and shows an empty export state when all rows are removed', () => {
    const markup = renderDjPlaylistReview(
      report([row({ excluded: true })]),
      copy,
    );

    expect(markup).not.toContain('Anchor Point');
    expect(markup).toContain('The export list is empty.');
    expect(markup).toContain('1 removed');
    expect(markup).toContain('data-action="dj-playlist-restore-excluded"');
    expect(markup).toContain('data-action="dj-playlist-export-copy" disabled');
  });
});
