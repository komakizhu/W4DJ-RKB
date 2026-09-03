export type ImportedDjPlaylistTrack = {
  position: number;
  title: string;
  artistDisplay: string;
  dedupeKey: string;
  neteaseImportLine: string;
};

export type DjPlaylistImportWarning = {
  code: string;
  message: string;
  position: number;
  dedupeKey: string;
};

export type ImportedDjPlaylist = {
  playlistId: string;
  formatVersion: number;
  name: string;
  sourcePath: string | null;
  importedAtMs: number | null;
  tracks: ImportedDjPlaylistTrack[];
  warnings: DjPlaylistImportWarning[];
};

export type ImportedDjPlaylistSummary = {
  playlistId: string;
  name: string;
  trackCount: number;
  warningCount: number;
  importedAtMs: number;
  sourcePath: string | null;
};

export type NeteaseQrPage = {
  index: number;
  total: number;
  trackCount: number;
  byteLength: number;
  firstPosition: number;
  lastPosition: number;
  text: string;
};

export const NETEASE_QR_MAX_TRACKS = 40;
export const NETEASE_QR_MAX_BYTES = 1500;

export class NeteaseQrPaginationError extends Error {
  readonly code = 'lineTooLong';
  readonly position: number;
  readonly byteLength: number;

  constructor(position: number, byteLength: number) {
    super(`歌曲 ${position} 的导入文本超过二维码单页 ${NETEASE_QR_MAX_BYTES} 字节限制`);
    this.name = 'NeteaseQrPaginationError';
    this.position = position;
    this.byteLength = byteLength;
  }
}

const encoder = new TextEncoder();

export function buildNeteaseImportText(tracks: ImportedDjPlaylistTrack[]): string {
  return tracks.map((track) => track.neteaseImportLine).join('\n');
}

export function splitNeteaseQrPages(
  tracks: ImportedDjPlaylistTrack[],
  limits: { maxTracks?: number; maxBytes?: number } = {},
): NeteaseQrPage[] {
  const maxTracks = limits.maxTracks ?? NETEASE_QR_MAX_TRACKS;
  const maxBytes = limits.maxBytes ?? NETEASE_QR_MAX_BYTES;
  if (!Number.isInteger(maxTracks) || maxTracks < 1 || !Number.isInteger(maxBytes) || maxBytes < 1) {
    throw new RangeError('二维码分页限制必须是正整数');
  }
  if (tracks.length === 0) return [];

  const pages: Array<{ tracks: ImportedDjPlaylistTrack[]; byteLength: number }> = [];
  let current: ImportedDjPlaylistTrack[] = [];
  let currentBytes = 0;
  for (const track of tracks) {
    const lineBytes = encoder.encode(track.neteaseImportLine).byteLength;
    if (lineBytes > maxBytes) {
      throw new NeteaseQrPaginationError(track.position, lineBytes);
    }
    const separatorBytes = current.length > 0 ? 1 : 0;
    const nextBytes = currentBytes + separatorBytes + lineBytes;
    if (current.length > 0 && (current.length >= maxTracks || nextBytes > maxBytes)) {
      pages.push({ tracks: current, byteLength: currentBytes });
      current = [];
      currentBytes = 0;
    }
    current.push(track);
    currentBytes += (current.length > 1 ? 1 : 0) + lineBytes;
  }
  if (current.length > 0) pages.push({ tracks: current, byteLength: currentBytes });

  return pages.map((page, index) => ({
    index,
    total: pages.length,
    trackCount: page.tracks.length,
    byteLength: page.byteLength,
    firstPosition: page.tracks[0].position,
    lastPosition: page.tracks[page.tracks.length - 1].position,
    text: buildNeteaseImportText(page.tracks),
  }));
}
