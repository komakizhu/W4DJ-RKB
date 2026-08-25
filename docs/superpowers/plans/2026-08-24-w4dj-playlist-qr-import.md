# W4DJ Playlist QR Import Implementation Plan

> **Current wire-format policy (2026-08-25):** This document records the
> original implementation plan and its v1 examples. The active `.w4dj` wire
> format is now the new minimal v2 format; v1 files and legacy fields are
> rejected explicitly and are not migrated. See
> `docs/superpowers/plans/2026-08-25-minimal-w4dj-playlist-format.md` for the
> current contract.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** First import a versioned `.w4dj` DJ playlist, turn it into NetEase-ready `歌名 - 歌手` text and paginated plaintext QR codes; after the songs have been downloaded and converted, recognize the corresponding `available` W4DJ output tracks with one action and export the playlist as a UTF-8 `.m3u8` containing relative audio paths.

**Architecture (historical v1 plan):** Rust owned untrusted-file parsing, validation, local playlist/match storage, deterministic output-track matching, and atomic M3U8 generation. The active v2 implementation keeps the local storage boundary but does not migrate or read v1 wire files. TypeScript owns UTF-8 byte-aware QR pagination, local QR rendering, user orchestration, review UI, and save dialogs. Matching reads only `available` destination outputs from `w4dj.sqlite3`; it never lets the NetEase database enumerate the Dashboard or playlist. Match rows store stable W4DJ `track_key` references and resolve the current destination path only at export time, so an output relocation cannot leave a stale path frozen in a playlist.

**Tech Stack:** Rust, Serde, Rusqlite, Tauri 2, TypeScript, Vite, Vitest, `qrcode` (bundled locally; no CDN), extended M3U8 plaintext.

## Global Constraints

- Historical v1 rule: this original plan accepted `format_version: 1`; that wire format is now retired. The active importer accepts only the new minimal v2 contract and rejects v1/legacy fields without migration.
- NetEase text is exactly one `title - artist_display` line per track, ordered by `position`.
- A QR page contains at most 40 complete track lines and at most 1,500 UTF-8 bytes; a track line is never split.
- QR error correction is `M`; the QR payload contains plaintext only, with no LAN server, cloud relay, URL, playlist header, page number, or local path.
- Require a non-empty `dedupe_key`; deduplicate by it, retaining the lowest-position occurrence and reporting skipped duplicates.
- Keep existing audio/folder/model drag-and-drop behavior unchanged; `.w4dj` has whole-window precedence only when exactly one `.w4dj` path is dropped.
- Persist imported playlists in `<app-data>/w4dj.sqlite3`; do not read from or write to the NetEase database.
- Match only current `available` W4DJ outputs whose destination files still exist and are readable. `outOfScope`, `missing`, and `unreadable` rows are never automatic match candidates.
- Preserve imported playlist order in M3U8 output. Resolve each match through its stable `track_key`, then calculate the audio path relative to the user-selected M3U8 file's parent directory.
- Write M3U8 as UTF-8 without BOM, with `\n` line endings, `#EXTM3U`, one `#EXTINF` entry per song, and unescaped filesystem paths using `/` separators.
- Never silently guess an ambiguous match or silently omit a song. Complete export requires every imported song to have one valid match; partial export is a separate explicit action with a matched/total confirmation.
- Do not copy, move, rename, retag, or delete audio while matching or exporting M3U8.
- Do not modify output audio, `track-analysis.json`, conversion history, or existing analysis records.
- Do not add hashes, baselines, frozen contracts, release gates, or a network service.
- Do not modify version `3.2.0-beta.3`, commit, push, merge, release, or publish.
- After implementation, build the latest arm64 App and provide clickable App and DMG links.

---

## File Structure

- Create `src/dj_playlist.rs`: historical v1 wire DTOs were superseded by strict v2 DTOs; active code owns v2 validation, internal deduplication, and import-text line sanitization shared by Rust exports.
- Create `src/dj_playlist_match.rs`: normalized identity keys, candidate indexing, deterministic match decisions, ambiguity reporting, and manual-override validation.
- Create `src/m3u8.rs`: safe relative-path calculation and deterministic extended-M3U8 rendering.
- Modify `src/lib.rs`: expose the `dj_playlist` module.
- Modify `src/w4dj_library.rs`: schema v2 migration and transactional imported-playlist/match persistence/query methods.
- Create `tests/dj_playlist.rs`: parser, validation, migration, persistence, and re-import tests.
- Create `tests/dj_playlist_match.rs`: output candidate, normalization, exact/ambiguous/unmatched, override, relocation, and status tests.
- Create `tests/m3u8.rs`: order, UTF-8, relative path, partial-export, and atomic-write tests.
- Modify `src-tauri/src/main.rs`: Tauri commands for import, list/load, matching, review overrides, explicit TXT export, and explicit M3U8 export.
- Create `app/src/dj-playlist.ts`: frontend DTOs, UTF-8 byte counting, exact-text joining, and QR page splitting over the backend-normalized track order.
- Create `app/src/dj-playlist.test.ts`: pure formatting and pagination tests.
- Create `app/src/qr-code.ts`: isolated wrapper around the bundled QR dependency.
- Create `app/src/qr-code.test.ts`: QR option and failure tests.
- Modify `app/src/app.ts`: services, state, button, modal, clipboard/export actions, and browser/native whole-window drop routing.
- Modify `app/src/app.test.ts`: UI, service, drag/drop, copy, export, and regression tests.
- Modify `app/src/styles.css`: full-window blur overlay and playlist/QR modal styles.
- Modify `app/package.json`, `app/package-lock.json`, and `app/pnpm-lock.yaml`: add bundled `qrcode` runtime and types consistently with the repository's current lockfiles.
- Modify `计划.md`, `docs/project-state.md`, and `docs/handoff.md`: record implementation and acceptance status after verification.

---

## Plan A: `.w4dj` import, NetEase text, and paginated QR

### Task 1: Historical v1 parser (superseded; do not implement or migrate)

**Files:**

- Create: `src/dj_playlist.rs`
- Modify: `src/lib.rs`
- Test: `tests/dj_playlist.rs`

**Interfaces:**

- Produces: `parse_w4dj_playlist(bytes: &[u8], source_path: Option<&Path>) -> Result<ImportedDjPlaylist, DjPlaylistError>`.
- Produces: Serde camelCase DTOs `ImportedDjPlaylist`, `ImportedDjPlaylistTrack`, and `DjPlaylistImportWarning` for Tauri and persistence.
- Produces: `netease_import_line(title: &str, artist_display: &str) -> Result<String, DjPlaylistError>`.

- [ ] **Step 1: Add failing parser and validation tests**

  Cover the supplied v1 shape plus wrong `format`, unsupported version, malformed JSON, empty `export_id`, empty playlist name, empty title/artist/dedupe key, zero/duplicate position, a file larger than 10 MiB, more than 10,000 tracks, embedded CR/LF/control characters, and duplicate `dedupe_key` values. Assert unknown JSON fields remain forward-compatible rather than causing rejection.

- [ ] **Step 2: Run the focused Rust test and confirm failure**

  Run: `cargo test --test dj_playlist`

  Expected: compilation fails because `w4dj::dj_playlist` does not exist.

- [ ] **Step 3: Define v1 wire DTOs and normalized DTOs**

  Use Serde snake_case fields for the file and camelCase for returned DTOs. The normalized track must contain `position`, `recordId`, `title`, `artistDisplay`, `artists`, `albumOrEp`, `durationSeconds`, `bpm`, `musicalKey`, `platformRefs`, `dedupeKey`, `expectedFilenameHint`, and precomputed `neteaseImportLine`.

- [ ] **Step 4: Implement bounded parsing, normalization, and deduplication**

  Reject files above 10 MiB and playlists above 10,000 input tracks. Trim ordinary surrounding whitespace, replace embedded `\r`, `\n`, and `\t` in title/artist with one space, collapse repeated whitespace, sort by ascending `position`, and retain the first occurrence of a repeated non-empty `dedupe_key`. Return one structured warning per skipped duplicate.

- [ ] **Step 5: Run the focused test**

  Run: `cargo test --test dj_playlist`

  Expected: all parser and validation cases pass.

---

### Task 2: Persist imported DJ playlists in `w4dj.sqlite3`

**Files:**

- Modify: `src/w4dj_library.rs`
- Test: `tests/dj_playlist.rs`

**Interfaces:**

- Consumes: `ImportedDjPlaylist` from Task 1.
- Produces: `W4djLibrary::upsert_imported_dj_playlist(&mut self, playlist: &ImportedDjPlaylist) -> W4djResult<()>`.
- Produces: `W4djLibrary::list_imported_dj_playlists(&self) -> W4djResult<Vec<ImportedDjPlaylistSummary>>`.
- Produces: `W4djLibrary::get_imported_dj_playlist(&self, playlist_id: &str) -> W4djResult<Option<ImportedDjPlaylist>>`.

- [ ] **Step 1: Add failing migration and round-trip tests**

  Test opening an existing schema-v1 database, importing a playlist, preserving track order and arrays/refs, re-importing the same `export_id`, rolling back a deliberately invalid replacement, deleting no output tracks, and reopening the database with identical results.

- [ ] **Step 2: Add schema-v2 tables**

  Increment `W4DJ_SCHEMA_VERSION` to `2` and create:

  ```sql
  CREATE TABLE IF NOT EXISTS imported_dj_playlists (
      playlist_id TEXT PRIMARY KEY,
      format_version INTEGER NOT NULL,
      name TEXT NOT NULL,
      output_mode TEXT,
      scenario TEXT,
      target_region TEXT,
      platform_priority_json TEXT NOT NULL,
      source_path TEXT,
      created_at TEXT,
      imported_at_ms INTEGER NOT NULL,
      warnings_json TEXT NOT NULL
  );
  CREATE TABLE IF NOT EXISTS imported_dj_playlist_tracks (
      playlist_id TEXT NOT NULL,
      position INTEGER NOT NULL,
      record_id TEXT,
      title TEXT NOT NULL,
      artist_display TEXT NOT NULL,
      artists_json TEXT NOT NULL,
      album_or_ep TEXT,
      duration_seconds INTEGER,
      bpm TEXT,
      musical_key TEXT,
      platform_refs_json TEXT NOT NULL,
      dedupe_key TEXT NOT NULL,
      expected_filename_hint TEXT,
      netease_import_line TEXT NOT NULL,
      PRIMARY KEY (playlist_id, position),
      UNIQUE (playlist_id, dedupe_key),
      FOREIGN KEY (playlist_id) REFERENCES imported_dj_playlists(playlist_id) ON DELETE CASCADE
  );
  CREATE TABLE IF NOT EXISTS imported_dj_playlist_matches (
      playlist_id TEXT NOT NULL,
      position INTEGER NOT NULL,
      track_key TEXT,
      status TEXT NOT NULL CHECK (status IN ('matched', 'unmatched', 'ambiguous', 'missing')),
      match_method TEXT,
      score INTEGER,
      candidates_json TEXT NOT NULL,
      matched_at_ms INTEGER NOT NULL,
      PRIMARY KEY (playlist_id, position),
      FOREIGN KEY (playlist_id, position)
          REFERENCES imported_dj_playlist_tracks(playlist_id, position) ON DELETE CASCADE,
      FOREIGN KEY (track_key) REFERENCES tracks(track_key) ON DELETE SET NULL
  );
  ```

- [ ] **Step 3: Implement transactional upsert and queries**

  Use `export_id` as `playlist_id`. Re-importing the same ID updates playlist metadata and replaces its imported track rows and obsolete match rows in one SQLite transaction. Validate/serialize all JSON columns before starting the transaction; no partial playlist may remain after an error. The match table is created in the same schema-v2 migration so Plan B does not require a second migration merely because both phases land together.

- [ ] **Step 4: Run persistence tests and existing library tests**

  Run: `cargo test --test dj_playlist && cargo test --test w4dj_library`

  Expected: both suites pass and existing track/analysis rows remain unchanged.

---

### Task 3: Add Tauri import, query, and TXT export commands

**Files:**

- Modify: `src-tauri/src/main.rs`
- Test: Tauri unit tests in `src-tauri/src/main.rs`

**Interfaces:**

- Produces command: `import_w4dj_playlist(path: String, state) -> Result<ImportedDjPlaylist, String>`.
- Produces command: `list_imported_dj_playlists(state) -> Result<Vec<ImportedDjPlaylistSummary>, String>`.
- Produces command: `load_imported_dj_playlist(playlist_id: String, state) -> Result<ImportedDjPlaylist, String>`.
- Produces command: `export_netease_playlist_text(path: String, text: String) -> Result<(), String>`.

- [ ] **Step 1: Add failing command tests**

  Cover a valid absolute `.w4dj` file, wrong extension, non-file/symlink input, invalid JSON, unsupported version, persistence failure, unknown playlist ID, non-absolute export path, and UTF-8 TXT output with `\n` line endings.

- [ ] **Step 2: Implement read-only file import command**

  Canonicalize the selected path, require a regular non-symlink `.w4dj` file, read at most 10 MiB, call `parse_w4dj_playlist`, then persist through `W4djLibrary`. Parsing failure must not change the database.

- [ ] **Step 3: Implement list/load commands**

  Return summaries newest-imported first and return a clear error for an unknown ID. These commands read only `w4dj.sqlite3` and never consult the NetEase database.

- [ ] **Step 4: Implement explicit TXT export**

  Accept only an absolute user-selected path, write UTF-8 text without a BOM, and preserve exact `\n` separators. Do not auto-export or overwrite any previous file.

- [ ] **Step 5: Register commands and run Tauri tests**

  Add all four commands to `tauri::generate_handler!`.

  Run: `cargo test --manifest-path src-tauri/Cargo.toml`

  Expected: all Tauri tests pass.

---

### Task 4: Format and split NetEase QR pages deterministically

**Files:**

- Create: `app/src/dj-playlist.ts`
- Create: `app/src/dj-playlist.test.ts`

**Interfaces:**

- Produces: `buildNeteaseImportText(tracks: ImportedDjPlaylistTrack[]): string`.
- Produces: `splitNeteaseQrPages(tracks: ImportedDjPlaylistTrack[], limits?: { maxTracks: number; maxBytes: number }): NeteaseQrPage[]`.
- Produces type: `NeteaseQrPage { index: number; total: number; trackCount: number; byteLength: number; firstPosition: number; lastPosition: number; text: string }`.

- [ ] **Step 1: Write failing Vitest cases**

  Assert exact joining of the backend-provided `neteaseImportLine`, preservation of normalized input order, no header/page number/path in payload, 40-track maximum, 41-track split, 1,500-byte split, complete-line boundaries, UTF-8 Chinese byte counting, empty-list behavior, and a single line over 1,500 bytes returning a typed error.

- [ ] **Step 2: Run the focused frontend test and confirm failure**

  Run: `pnpm --dir app test -- --run app/src/dj-playlist.test.ts`

  Expected: module-not-found failure.

- [ ] **Step 3: Implement pure formatting and splitting**

  Use each normalized track's `neteaseImportLine` without recomputing title/artist formatting in TypeScript. Use `TextEncoder` for byte counts. A page byte count includes inter-line `\n` bytes but no trailing newline. Default limits are constants `NETEASE_QR_MAX_TRACKS = 40` and `NETEASE_QR_MAX_BYTES = 1500`.

- [ ] **Step 4: Run the focused frontend test**

  Run: `pnpm --dir app test -- --run app/src/dj-playlist.test.ts`

  Expected: all cases pass.

---

### Task 5: Bundle QR generation behind a testable adapter

**Files:**

- Modify: `app/package.json`
- Modify: `app/package-lock.json`
- Modify: `app/pnpm-lock.yaml`
- Create: `app/src/qr-code.ts`
- Create: `app/src/qr-code.test.ts`

**Interfaces:**

- Consumes: `NeteaseQrPage.text` from Task 4.
- Produces: `renderPlaintextQrDataUrl(text: string) -> Promise<string>`.

- [ ] **Step 1: Add `qrcode` and its TypeScript types locally**

  Add the runtime dependency and types with the repository package manager. Do not load scripts, fonts, analytics, or QR services from a CDN.

- [ ] **Step 2: Add failing adapter tests**

  Mock `qrcode.toDataURL` and assert options `{ errorCorrectionLevel: 'M', margin: 2, width: 512, type: 'image/png' }`, exact plaintext input, empty-text rejection, and surfaced rendering errors.

- [ ] **Step 3: Implement the adapter**

  Keep the third-party import in this file so `app.ts` and its tests do not depend on QR implementation details.

- [ ] **Step 4: Run focused tests and a production build**

  Run: `pnpm --dir app test -- --run app/src/qr-code.test.ts && pnpm --dir app build`

  Expected: tests and Vite production build pass without any network request at runtime.

---

### Task 6: Add button import, full-window drop, modal, copy, and TXT export

**Files:**

- Modify: `app/src/app.ts`
- Modify: `app/src/styles.css`
- Modify: `app/src/app.test.ts`

**Interfaces:**

- Consumes: Tasks 3–5 commands, DTOs, page splitter, and QR renderer.
- Produces: visible action `data-action="import-dj-playlist"` and dialog `data-role="dj-playlist-dialog"`.
- Produces: whole-window overlay `data-role="dj-playlist-drop-overlay"`.

- [ ] **Step 1: Add AppServices and UI-state tests first**

  Extend `AppServices` with `pickW4djPlaylist`, `importW4djPlaylist`, `listImportedDjPlaylists`, `loadImportedDjPlaylist`, and `exportNeteasePlaylistText`. Test picker cancellation, import success/error, modal close/reopen, async QR revision filtering, and persisted playlist reload.

- [ ] **Step 2: Add the explicit import button and picker**

  Put “导入 DJ 歌单 / Import DJ playlist” beside the existing Library top-bar action. Open a single-file dialog filtered to `.w4dj`; cancellation is silent and never invokes the backend.

- [ ] **Step 3: Add whole-window `.w4dj` drag precedence**

  Detect an extension case-insensitively before model and slot hit-testing in both browser and Tauri drag handlers. During enter/over, show a full-window blurred overlay reading “松开导入 DJ 歌单”. On drop, accept exactly one `.w4dj`; reject mixed/multiple drops with a localized error. Directory/audio source drops and the disabled model-drop message retain their current behavior.

- [ ] **Step 4: Render the playlist and QR dialog**

  Show playlist name, total imported/skipped count, a compact ordered track preview, current QR image, “第 X/Y 页” outside the QR payload, per-page track/byte counts, previous/next controls, and a readable plaintext preview. Disable navigation while QR rendering is pending and ignore completion from a stale page request.

- [ ] **Step 5: Add clipboard and TXT actions**

  “复制当前页” copies only the current QR payload; “复制全部” copies all formatted lines in one text block; “导出 TXT” opens a save dialog named from the sanitized playlist name and calls the backend only after a path is selected. Reuse the existing clipboard fallback used by library lyrics.

- [ ] **Step 6: Add CSS and accessibility behavior**

  The overlay must cover and blur the complete window, honor reduced-motion settings, and never appear for ordinary audio/folder drags. The modal needs a dialog label, keyboard-focusable controls, visible loading/error states, alt text for the QR page, and no full local source path.

- [ ] **Step 7: Run the complete app test file**

  Run: `pnpm --dir app test -- --run app/src/app.test.ts`

  Expected: all existing and new UI tests pass.

---

### Task 7: Verify Plan A with the real sample and pagination fixtures

**Files:**

- Test: focused Rust, Tauri, and frontend suites from Tasks 1–6

- [ ] **Step 1: Run Plan A focused verification**

  ```bash
  cargo test --test dj_playlist
  cargo test --test w4dj_library
  cargo test --manifest-path src-tauri/Cargo.toml
  pnpm --dir app test -- --run app/src/dj-playlist.test.ts app/src/qr-code.test.ts app/src/app.test.ts
  pnpm --dir app build
  git diff --check
  ```

- [ ] **Step 2: Accept the supplied real `.w4dj` sample**

  Import `/Users/mac2/Documents/Codex/2026-08-11/handoff-md-dj-crate-digger-skill/afro-house-club.w4dj`. Verify 10 tracks, one page, 386 UTF-8 bytes without a trailing newline, exact first line `Anchor Point - Ahmed Spins, Stevo Atambire`, exact last line `The Boy Is Mine - James Mac, Vall, Rosalie`, and no database changes outside the two imported-playlist tables.

- [ ] **Step 3: Accept pagination and scanner behavior**

  Use a generated in-memory/test fixture of at least 100 tracks to verify multiple pages, 40-track/1,500-byte limits, stable order, and complete-line boundaries. Scan the displayed PNG with at least one phone scanner when available; verify it presents plaintext directly and that copied text can be pasted into NetEase. If a phone or NetEase is unavailable, record the exact unexecuted manual steps rather than marking them passed.


---

## Plan B: Recognize converted outputs and generate a relative-path M3U8

### Task 8: Build deterministic output-track matching

**Files:**

- Create: `src/dj_playlist_match.rs`
- Modify: `src/lib.rs`
- Test: `tests/dj_playlist_match.rs`

**Interfaces:**

- Produces: `match_imported_playlist(playlist: &ImportedDjPlaylist, candidates: &[DjOutputCandidate]) -> DjPlaylistMatchReport`.
- Produces camelCase DTOs: `DjPlaylistMatchReport`, `DjPlaylistTrackMatch`, and `DjPlaylistMatchCandidate`.
- Uses statuses: `matched | unmatched | ambiguous | missing`.

- [ ] **Step 1: Add failing identity and match tests**

  Cover Unicode NFKC, case, repeated whitespace, punctuation, `feat.`/`ft.` spelling, artist-order differences, exact title/artist matches, duration tolerance, mix/version distinctions, multiple equally valid outputs, absent metadata, duplicate candidate reuse, and candidates with non-`available` status. Do not introduce a fuzzy-score baseline or snapshot.

- [ ] **Step 2: Define safe normalization and candidate indexing**

  Normalize comparison keys without changing stored display text: Unicode NFKC, lowercase, punctuation-to-space, whitespace collapse, normalized featured-artist separators, and an unordered normalized artist token set. Preserve meaningful title qualifiers such as `remix`, `mix`, `edit`, `live`, `radio`, and version names so two editions are not collapsed into one identity.

- [ ] **Step 3: Implement deterministic match tiers**

  Build an index from `available` W4DJ output metadata rather than doing an unrestricted O(playlist × library) scan. Automatic `matched` requires exactly one candidate with normalized title and artist agreement; when both durations exist, their absolute difference must be no more than 5 seconds. If several candidates satisfy the same strongest tier, return `ambiguous`. If none qualifies, return `unmatched`. A single `track_key` cannot automatically satisfy two different imported `dedupe_key` values.

- [ ] **Step 4: Keep suggestions separate from accepted matches**

  Lower-confidence filename hints or partial artist matches may appear in `candidates`, but must not populate `track_key` automatically. The report records an explainable `matchMethod`, integer `score`, and reason text for every row; no hidden nearest-neighbor guess is permitted.

- [ ] **Step 5: Run focused matching tests**

  Run: `cargo test --test dj_playlist_match`

  Expected: exact matches are stable, ambiguous cases stay unresolved, non-available outputs are ignored, and no output is assigned twice.

---

### Task 9: Persist and refresh playlist match snapshots

**Files:**

- Modify: `src/w4dj_library.rs`
- Test: `tests/dj_playlist_match.rs`

**Interfaces:**

- Produces: `W4djLibrary::available_dj_output_candidates()`.
- Produces: `W4djLibrary::replace_imported_dj_playlist_matches(playlist_id, matches)`.
- Produces: `W4djLibrary::get_imported_dj_playlist_match_report(playlist_id)`.
- Produces: `W4djLibrary::set_imported_dj_playlist_match(playlist_id, position, track_key)` and `clear_imported_dj_playlist_match(...)` for explicit review overrides.

- [ ] **Step 1: Add failing transaction and lifecycle tests**

  Cover one-action refresh, rollback on an invalid row, playlist re-import clearing obsolete matches, application restart, manual override validation, an output becoming `missing`, and an output destination path changing after a successful match.

- [ ] **Step 2: Query candidates from the independent W4DJ library only**

  Join the current W4DJ track and local-file metadata needed for title, artist, duration, destination path, and status. Exclude NetEase SQLite, `library-dashboard.sqlite3`, source-folder enumeration, conversion history, and `track-analysis.json` as authorities.

- [ ] **Step 3: Replace each automatic match snapshot transactionally**

  Fully compute and validate a report before opening the transaction, then replace all non-manual results for the playlist at once. Store `track_key`, never a frozen destination path. Preserve a still-valid explicit manual override; turn it into `missing` if its referenced track is no longer available/readable.

- [ ] **Step 4: Validate manual overrides**

  A manual choice must reference an existing `available` output, cannot reuse an output already assigned to a different imported song, and must be visibly marked `manual`. Clearing it returns the row to the next automatic refresh rather than deleting any song.

- [ ] **Step 5: Run persistence and library regression tests**

  Run: `cargo test --test dj_playlist_match && cargo test --test w4dj_library`

---

### Task 10: Generate deterministic extended M3U8 with relative paths

**Files:**

- Create: `src/m3u8.rs`
- Test: `tests/m3u8.rs`

**Interfaces:**

- Produces: `build_relative_m3u8(playlist, resolved_tracks, playlist_path, allow_partial) -> Result<String, M3u8Error>`.
- Produces: `write_relative_m3u8_atomic(path, contents) -> Result<(), M3u8Error>`.

- [ ] **Step 1: Add failing format and path tests**

  Cover imported order, Chinese/emoji/spaces/`#` in filenames, sibling directories requiring `..`, audio in the same directory, `/` separators, duration rounding, missing artist, embedded line-break sanitization, UTF-8 without BOM, LF-only output, incomplete matching, a disappeared file, and an empty result.

- [ ] **Step 2: Implement extended-M3U8 rendering**

  Emit exactly:

  ```text
  #EXTM3U
  #EXTINF:<whole-seconds>,<artist> - <title>
  <path-relative-to-the-m3u8-parent>
  ```

  Preserve `.w4dj` position order and use current destination paths resolved from `track_key` at export time. Do not URL-encode or add `file://`. Replace CR/LF in `#EXTINF` display fields with spaces. Reject a path that cannot be represented relative to the selected playlist parent.

- [ ] **Step 3: Enforce complete versus partial export explicitly**

  With `allow_partial=false`, reject unless every imported row resolves to one existing readable output. With `allow_partial=true`, include only valid matched rows but return a summary listing every omitted position and reason; never produce an empty playlist or omit rows silently.

- [ ] **Step 4: Write atomically to the selected destination**

  Create a temporary file in the same directory, flush it, then rename it to the user-selected `.m3u8` path. A failure must not leave a truncated final playlist. This protects the explicit filesystem export boundary and does not add a release gate or baseline.

- [ ] **Step 5: Run focused M3U8 tests**

  Run: `cargo test --test m3u8`

---

### Task 11: Expose matching, review, and M3U8 export commands for one UI workflow

**Files:**

- Modify: `src-tauri/src/main.rs`
- Test: Tauri unit tests in `src-tauri/src/main.rs`

**Interfaces:**

- Produces command: `match_imported_dj_playlist(playlist_id, state) -> Result<DjPlaylistMatchReport, String>`.
- Produces command: `load_imported_dj_playlist_matches(playlist_id, state) -> Result<DjPlaylistMatchReport, String>`.
- Produces command: `set_imported_dj_playlist_match(playlist_id, position, track_key, state) -> Result<DjPlaylistMatchReport, String>`.
- Produces command: `clear_imported_dj_playlist_match(playlist_id, position, state) -> Result<DjPlaylistMatchReport, String>`.
- Produces command: `export_imported_dj_playlist_m3u8(playlist_id, path, allow_partial, state) -> Result<DjPlaylistM3u8ExportResult, String>`.

- [ ] **Step 1: Add failing command tests**

  Cover unknown playlist, no candidates, complete match, ambiguity, override, non-absolute save path, wrong extension, save cancellation at the UI boundary, full-export rejection, explicit partial export, path relocation between match and export, disappeared audio, and write failure.

- [ ] **Step 2: Implement the one-click match command**

  Load the stored playlist and `available` output candidates, compute a full report in memory, then persist its snapshot transactionally. Matching is read-only with respect to audio and does not trigger conversion, analysis, metadata writes, or a NetEase refresh.

- [ ] **Step 3: Implement review commands and export-time revalidation**

  Validate every override in Rust. Before M3U8 generation, reload current track status/path, verify readability, and return the exact unmatched/ambiguous/missing rows rather than relying on stale UI state.

- [ ] **Step 4: Register commands and run Tauri tests**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml`

---

### Task 12: Add one-click recognition-and-M3U8 workflow with review fallback

**Files:**

- Modify: `app/src/dj-playlist.ts`
- Modify: `app/src/app.ts`
- Modify: `app/src/app.test.ts`
- Modify: `app/src/styles.css`

- [ ] **Step 1: Add service and UI tests first**

  Test “识别并生成 M3U8”, loading and error states, matched/ambiguous/unmatched counts, detailed reasons, manual candidate choice, clearing an override, stale async response filtering, direct N/N export, explicit partial confirmation, save-dialog cancellation, and successful saved-path feedback.

- [ ] **Step 2: Add the primary one-click workflow**

  Add “识别并生成 M3U8” as the primary action. One click scans the independent W4DJ library. If the result is N/N, immediately open the M3U8 save dialog and export after the user confirms a path. If any row is ambiguous, unmatched, or missing, do not open the save dialog or create a file; instead open the review state with `已匹配 X / 总计 Y` and every unresolved row visible. Provide “重新识别” as a secondary action. Do not auto-start conversion or analysis.

- [ ] **Step 3: Add ambiguity review without weakening safety**

  For ambiguous/unmatched rows, show the small backend-provided candidate list with title, artist, duration, and destination filename only. A user may explicitly select or clear a match; do not expose full local paths in the normal list and do not auto-select the first candidate.

- [ ] **Step 4: Add post-review complete and partial M3U8 actions**

  Enable “生成 M3U8” only when all rows are currently matched. When unresolved rows exist, offer a separate “仅导出已匹配 X/Y” action that shows the omitted count and requires confirmation. Both open a save dialog defaulting to `<sanitized-playlist-name>.m3u8`; cancellation performs no backend write.

- [ ] **Step 5: Run frontend regression tests and build**

  Run: `pnpm --dir app test -- --run app/src/dj-playlist.test.ts app/src/app.test.ts && pnpm --dir app build`

---

### Task 13: Complete end-to-end verification, documentation, and latest App build

**Files:**

- Modify: `计划.md`
- Modify: `docs/project-state.md`
- Modify: `docs/handoff.md`

- [ ] **Step 1: Run complete automated verification**

  ```bash
  pnpm --dir app test -- --run
  pnpm --dir app build
  cargo test --all
  cargo test --manifest-path src-tauri/Cargo.toml
  cargo fmt --all -- --check
  cargo check --manifest-path src-tauri/Cargo.toml
  cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
  git diff --check
  ```

  Record the existing `src/sync.rs` `dead_code` warning honestly if strict Clippy remains blocked by unrelated code; do not weaken Clippy or delete unrelated safety code.

- [ ] **Step 2: Run the real end-to-end playlist flow**

  Import the supplied Afro House sample, verify its QR text, then use a controlled fixture/output set representing downloaded and converted songs. Confirm one-click matching preserves the 10 imported positions, exposes deliberate ambiguous/unmatched cases, and reaches 10/10 only after every song has a valid unique output.

- [ ] **Step 3: Validate M3U8 outside the application**

  Save the M3U8 beside or above the controlled output tree, verify every non-comment line resolves from the M3U8 parent to the expected readable audio file, and import it into an available M3U8-compatible player. When Rekordbox is available, import it there and verify ordering and all resolved tracks; otherwise record the exact manual Rekordbox steps as unexecuted.

- [ ] **Step 4: Update project documents**

  Record the schema-v2 playlist/match tables, matching rules, ambiguity policy, relative-path base, full/partial export behavior, real acceptance evidence, and remaining phone/NetEase/Rekordbox environment limits. Do not describe conversion completion as playlist matching completion.

- [ ] **Step 5: Build and identify the latest arm64 application**

  Run: `cargo tauri build --target aarch64-apple-darwin`

  Verify Mach-O arm64 and version `3.2.0-beta.3`. Report clickable links to the `.app` and `.dmg` bundles.

- [ ] **Step 6: Report the shared worktree without committing**

  Run: `git status --short --branch` and `git diff --stat`. Report this feature separately from pre-existing dirty-worktree changes. Do not commit or push; wait for user confirmation.

---

## Plan A Acceptance Criteria: `.w4dj` → NetEase text → QR

- Selecting or dropping the supplied `.w4dj` anywhere in W4DJ imports exactly 10 ordered tracks and persists them across application restart.
- The generated payload is exactly newline-separated `歌名 - 歌手` plaintext, with no trailing newline, paths, JSON wrapper, playlist header, or page number. The supplied sample is exactly 386 UTF-8 bytes; its first and last lines match Task 7.
- Every QR page contains at most 40 complete tracks and at most 1,500 UTF-8 bytes. A 100-track fixture produces multiple stable, navigable pages without splitting a line.
- Scanning a QR presents plaintext directly; copy-current, copy-all, and explicit TXT export produce byte-for-byte expected text that can be pasted into NetEase.
- Invalid, oversized, unsupported, duplicate, mixed, or malformed imports do not partially modify `w4dj.sqlite3` or disturb conversion/library state.
- QR generation is entirely bundled and works without a runtime network connection. Existing directory, audio, and model drag/drop behavior remains unchanged.

## Plan B Acceptance Criteria: Converted outputs → recognition → relative M3U8

- “识别并生成 M3U8” considers only existing/readable `available` destination outputs from `w4dj.sqlite3`; NetEase SQLite, source folders, conversion history, and analysis JSON cannot add candidates.
- When matching reaches N/N, the primary action proceeds directly to the save dialog and produces the M3U8 after path confirmation. If any row is unresolved, the same action creates no file and opens the review list instead.
- Exact deterministic matches are made once, playlist order is preserved, one output cannot satisfy two different imported songs, and ambiguous or low-confidence candidates are never silently accepted.
- Every imported row visibly reports `matched`, `ambiguous`, `unmatched`, or `missing`, with an explainable method/reason. Manual selection accepts only a current valid output and survives restart through `track_key`.
- Complete M3U8 export is possible only at N/N valid matches. Partial export requires a separate explicit confirmation and returns the exact omitted positions/reasons; no song is silently dropped.
- The saved file is UTF-8 without BOM, LF-only extended M3U8. Audio entries use `/`-separated paths relative to the selected M3U8 file's parent and resolve to the current destination files in original `.w4dj` position order.
- Relocating a matched W4DJ output before export resolves its new path through `track_key`; a missing/unreadable output blocks complete export. Matching/export never changes audio or metadata.
- A write failure leaves no truncated final M3U8. A successfully exported fixture opens in an M3U8-compatible player, and Rekordbox verification is recorded separately when that environment is available.

## Combined End-to-End Acceptance

- One persisted playlist can complete the full local flow without re-import: `.w4dj` import → NetEase plaintext/QR → songs downloaded externally → W4DJ conversion registers outputs → one-click recognition and, at N/N, relative M3U8 export; unresolved songs enter a review fallback before export.
- The supplied 10-track sample retains the same playlist ID, dedupe keys, and position order across both phases. At 10/10 valid outputs, the M3U8 contains 10 `#EXTINF` records and 10 relative paths, all resolving to readable files.
- Restarting W4DJ between any two stages preserves the imported playlist and valid match decisions. Re-running recognition is idempotent and does not duplicate playlists, match rows, or audio.
- The full Rust/frontend/Tauri test suites, Vite build, format/check/Clippy policy, diff check, and latest arm64 App build complete or have any unrelated pre-existing limitation reported precisely.

## Self-Review

- Spec coverage: Tasks 1–7 implement and accept `.w4dj` import, NetEase text, and paginated QR; Tasks 8–12 implement safe converted-output recognition and relative M3U8; Task 13 performs combined verification, documentation, and packaging.
- Type consistency: Rust owns the normalized imported track and match report; TypeScript consumes camelCase DTOs and never independently redefines match truth or path resolution.
- Data authority: imported playlist rows describe requested songs; `w4dj.sqlite3` `available` destination rows describe actual outputs; the NetEase database remains conversion metadata only.
- Safety: ordinary size limits, types, transactions, foreign keys, uniqueness, export-time file validation, atomic write, and focused tests cover the concrete failure modes. No hash, frozen contract, baseline, or release gate is added.

## 2026-08-25 Execution Audit

- [x] Code implementation completed for Tasks 1–12, including the persisted recent-playlist reopen action and native `.w4dj` drag-over overlay continuity.
- [x] Supplied sample `/Users/mac2/Documents/Codex/2026-08-11/handoff-md-dj-crate-digger-skill/afro-house-club.w4dj` was parsed by the production Rust parser and persisted/reloaded in an isolated `w4dj.sqlite3`: 10 tracks, 0 warnings, 386 UTF-8 bytes without the trailing newline, and the expected first/last lines.
- [x] Controlled 10-output fixture reached deterministic 10/10 matching, generated 10 `#EXTINF` records and 10 relative paths, wrote atomically, and verified every path resolved to a readable fixture file.
- [x] Frontend Vitest completed with 26 files/386 tests, TypeScript and Vite build passed; playlist/match/M3U8 Rust suites passed 5/5, 7/7, 4/4; Tauri suite passed 57/57; Tauri strict Clippy passed after removing unrelated command warnings; `git diff --check` passed.
- [ ] Phone QR scanning, NetEase paste, an external M3U8 player, and Rekordbox import remain manual environment-dependent checks. Exact follow-up steps are: (1) open the latest arm64 App, import the supplied `.w4dj`, advance through every QR page, scan each page with a phone, and verify the copied/scanned text matches the page text; (2) paste the copied all-pages text into NetEase and verify the expected 10-song playlist is accepted; (3) export the controlled 10-track `.m3u8`, open it in an available compatible player such as IINA/VLC, and verify all 10 entries and order; (4) in Rekordbox, import that same `.m3u8`, verify all resolved tracks and order, then record the result. The current environment also did not produce a new DMG: the Tauri DMG script failed before invocation and `hdiutil` returned “设备未配置”; the arm64 `.app` was built successfully. These checks were not simulated or marked passed.
- [x] Final root `cargo test --all` rerun exited 0; the earlier three filename-fallback failures did not reproduce. Root `cargo fmt --all -- --check` still reports pre-existing broad formatting differences, which remain recorded rather than hidden or batch-reformatted.

## 2026-08-25 Final Verification Update

- [x] Added and passed the required 100-track pagination fixture; current frontend full run is 26 files / 388 tests.
- [x] Current worktree verification: root `cargo test --all`, Tauri 57 tests, focused playlist/match/M3U8 suites (7/7, 5/5, 4/4), TypeScript, Vite, `cargo fmt --all -- --check`, Tauri `cargo check`, strict all-targets Clippy, and `git diff --check` all passed.
- [x] Rebuilt `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app` at 2026-08-25 01:00:33; Mach-O is arm64, version is `3.2.0-beta.3`, and the bundle contains the FFmpeg sidecar.
- [ ] Phone QR scan, NetEase paste, external-player playback, and Rekordbox import remain unexecuted. Manual sequence: scan every QR page and compare decoded text; paste all-pages text into NetEase; open exported M3U8 in IINA/VLC and verify all 10 entries/order; import the same M3U8 in Rekordbox and verify order. No result is claimed for these environment-dependent steps. DMG creation remains unavailable (`bundle_dmg.sh` argument failure and `hdiutil` “设备未配置”).
