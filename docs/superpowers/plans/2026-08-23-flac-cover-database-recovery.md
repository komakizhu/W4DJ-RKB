# FLAC Cover Database Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **2026-08-24 验收入口更新：** 89 首真实 FLAC、数据库快照、封面恢复和 ExifTool/WAL/SHM 验收改用 `2026-08-24-headless-acceptance.md` 的 `flacCoverRecovery` 后台场景；不打开 W4DJ App GUI。

**Goal:** Ensure ordinary NetEase FLAC files use the same explicitly selected read-only NetEase database as discovery, recover locally available artwork during conversion or metadata-only refresh, and produce enough diagnostics to distinguish database selection, record matching, local artwork lookup, and output writing failures.

**Architecture:** Resolve the effective NetEase database once at batch start, build an immutable metadata resolver, and explicitly pass it into conversion workers instead of letting `src/netease.rs` guess a database from process-global state. The resolver remains local-only: it can read embedded artwork, SQLite blobs, explicit local paths, and NetEase cache files, but it does not download a remote `picUrl`. Per-track diagnostics record the exact stage and artwork source without embedding artwork bytes or modifying the NetEase database.

**Tech Stack:** Rust, Tauri, rusqlite read-only connections, ID3/metaflac metadata handling, Serde, existing history/runtime-session reports, Cargo tests.

## Global Constraints

- Do not change SemVer or the public product version.
- Do not commit, push, merge, release, or publish artifacts without the user's explicit `定稿` instruction.
- Open the NetEase database read-only and never alter its main file, WAL, SHM, schema, or records.
- Preserve existing valid output artwork; recovery fills missing artwork only.
- Do not re-encode audio during `仅更新元数据` backfill.
- Keep Dashboard authority in `w4dj.sqlite3`; the NetEase database remains conversion-time metadata input only.
- Do not add hashes, baselines, frozen contracts, or release gates.
- Do not download remote cover URLs in this plan. A remote-only cover must be reported as `remoteOnly`, not silently treated as a local lookup success.
- Existing preferences, history entries, runtime sessions, and reports must remain readable through optional/defaulted fields.

## Execution status (2026-08-23)

Tasks 1–6 are implemented in the shared worktree. The resolver, conversion context, conservative matching, local-only cover precedence, diagnostic propagation, and manual report section are covered by focused Rust/Tauri/history tests. Task 7 automation is complete on the available environment; the 89-song FLAC acceptance, database/WAL/SHM before/after check, ExifTool readback, and GUI acceptance remain explicitly pending because the corresponding source batch is not present in the current `/Users/Shared` tree.

---

## Confirmed Root Cause and Baseline

The supplied batch `batch-mt5sb601-50ut54-slot1` contains 89 unique FLAC inputs. Twenty-one had valid artwork at the merged source-metadata boundary and all twenty-one outputs retained it. Sixty-eight had no recovered source artwork; none represents a case where valid source artwork was lost by the output writer. Of those sixty-eight, sixty-four also lacked a recovered title/artist identity, which places the dominant failure before output writing, at database selection or conservative record matching.

The current conversion recovery in `src/netease.rs` discovers only fixed default paths plus `W4DJ_NETEASE_DB`. The manual database path held by Tauri `AppState` and persisted in preferences is used by discovery/Dashboard commands but is not explicitly supplied to conversion metadata recovery. Existing reports also omit the effective database path, record count, match method, cover reference type, and cover lookup result, so the remaining failure boundary cannot be proven from the exported report.

## File Map

- Modify `src/netease.rs`: immutable resolver, database selection, indexed matching, local artwork resolution, and typed diagnostics.
- Modify `src/sync.rs`: accept the resolver explicitly, fill only missing cover metadata, and propagate per-track diagnostics through normal conversion and metadata-only refresh.
- Modify `src/history.rs`: serialize and format database/artwork recovery diagnostics with backward-compatible optional fields.
- Modify `src-tauri/src/main.rs`: resolve the preferred database at batch start, pass the resolver to workers, and include resolver summary in runtime session exports.
- Modify `tests/history.rs`: report compatibility and readable diagnostic coverage.
- Modify or extend unit tests in `src/netease.rs` and `src/sync.rs`: matching, precedence, local cover recovery, and output preservation.
- Modify `docs/project-state.md`, `docs/handoff.md`, and `计划.md`: record the implemented behavior and remaining remote-only limitation after verification.

---

### Task 1: Define the resolver and diagnostic contract

**Files:**

- Modify: `src/netease.rs`
- Modify: `src/sync.rs`
- Modify: `src/history.rs`
- Test: unit tests in `src/netease.rs`
- Test: `tests/history.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NeteaseRecordMatchMethod {
    ExactPath,
    PathSuffix,
    FileNameAndSize,
    FileNameAndIdentity,
    NoMatch,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NeteaseCoverSource {
    Embedded,
    DatabaseBlob,
    ExplicitLocalPath,
    LocalCache,
    RemoteOnly,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseRecoveryDiagnostic {
    pub database_path: Option<String>,
    pub database_loaded: bool,
    pub database_record_count: usize,
    pub matched: bool,
    pub match_method: Option<NeteaseRecordMatchMethod>,
    pub track_id: Option<String>,
    pub album_id: Option<String>,
    pub cover_source: Option<NeteaseCoverSource>,
    pub cover_bytes: Option<usize>,
    pub message: Option<String>,
}

pub struct MetadataRecovery {
    pub metadata: Option<RecoveredMetadata>,
    pub diagnostic: NeteaseRecoveryDiagnostic,
}
```

- [ ] Add serialization tests proving new fields use camelCase and old report JSON without them still deserializes.
- [ ] Add a report-format test asserting the diagnostic distinguishes `database not loaded`, `no match`, `ambiguous`, `remote only`, `local cache found`, and `output write failed`.
- [ ] Run `cargo test --test history` and confirm the new tests fail before implementation.
- [ ] Implement the types with `#[serde(default)]` or optional fields at every persisted compatibility boundary.
- [ ] Extend `MetadataDiagnostic` with an optional `netease_recovery: Option<NeteaseRecoveryDiagnostic>` field; do not change the meaning of existing `source_artwork` and `output_artwork` fields.
- [ ] Run `cargo test --test history` and the `netease` unit tests; confirm they pass.

**Task acceptance:** An exported diagnostic can state where artwork recovery stopped without exposing artwork bytes, and an existing history/report fixture remains readable.

---

### Task 2: Resolve and preload the effective database once per batch

**Files:**

- Modify: `src/netease.rs`
- Modify: `src-tauri/src/main.rs`
- Test: unit tests in `src/netease.rs`
- Test: Tauri unit tests in `src-tauri/src/main.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone)]
pub struct NeteaseMetadataResolver {
    database_path: Option<PathBuf>,
    records: Arc<Vec<NeteaseRecord>>,
    // Private indexes keyed by normalized exact path, filename/stem,
    // track ID, and normalized title/artist identity.
}

impl NeteaseMetadataResolver {
    pub fn load(preferred_database: Option<&Path>) -> io::Result<Self>;
    pub fn database_path(&self) -> Option<&Path>;
    pub fn record_count(&self) -> usize;
    pub fn recover(&self, source_path: &Path) -> MetadataRecovery;
}
```

- [ ] Write tests for precedence: valid manual preference wins; invalid manual preference falls back to automatic discovery and returns a warning; no valid database creates an empty resolver rather than failing audio conversion.
- [ ] Write a read-only test that records the database file size and modification time before/after resolver loading and asserts they are unchanged.
- [ ] Write a batch-snapshot test proving a resolver does not reopen SQLite for every song and remains immutable while workers use it.
- [ ] Run the focused tests and confirm failure against the current process-global `load_cached_records()` behavior.
- [ ] Implement `NeteaseMetadataResolver::load` with `SQLITE_OPEN_READ_ONLY`; reuse the existing supported-table validation and merge logic.
- [ ] Build indexes once after loading. Preserve the current conservative ambiguity rejection and do not lower the acceptance threshold merely to increase match count.
- [ ] At Tauri batch start, read `AppState.manual_database_path`, validate it, apply the same fallback policy as library discovery, and create one `Arc<NeteaseMetadataResolver>`.
- [ ] Record the effective path, whether fallback occurred, and record count in the runtime session before the first file starts.
- [ ] Keep the legacy auto-resolving wrapper only for CLI/backward compatibility; desktop conversion must use the explicit resolver.
- [ ] Run the focused Rust and Tauri tests and confirm they pass.

**Task acceptance:** A manually selected valid database is demonstrably the database used by conversion, is opened read-only once per batch, and its path/record count appear in the runtime session.

---

### Task 3: Make FLAC matching explainable without unsafe guessing

**Files:**

- Modify: `src/netease.rs`
- Test: unit tests in `src/netease.rs`

- [ ] Add fixtures covering exact absolute path, normalized path suffix, same stem across `.ncm` and `.flac`, filename plus exact file size, title/artist filename identity, duplicate ambiguous rows, and a stale same-name row with a different identity.
- [ ] Run focused tests and confirm the diagnostic match method is unavailable before implementation.
- [ ] Refactor the existing score calculation to return both score and evidence, then map accepted evidence to `NeteaseRecordMatchMethod`.
- [ ] Require exact/suffix path, filename plus size, or filename plus title/artist identity. Continue rejecting filename-only and tied incompatible records.
- [ ] Return `Ambiguous` separately from `NoMatch`; include candidate count in the human-readable message but do not expose unrelated local paths.
- [ ] Preserve support for database rows retaining an `.ncm` filename while the actual source is `.flac` when the stem and stronger evidence agree.
- [ ] Run all `netease` unit tests and confirm legacy matching behavior remains covered.

**Task acceptance:** Every unmatched FLAC is categorized as no match or ambiguous, and the resolver never attaches another same-named song's artwork.

---

### Task 4: Recover local artwork with deterministic precedence

**Files:**

- Modify: `src/netease.rs`
- Modify: `src/sync.rs`
- Test: unit tests in `src/netease.rs`
- Test: unit tests in `src/sync.rs`

**Required precedence:**

1. Existing valid embedded FLAC artwork.
2. Valid image BLOB in the matched SQLite row.
3. Existing local file referenced by a database column or JSON field.
4. Existing NetEase local cache image resolved by track ID, album ID, or explicit cached filename.
5. `RemoteOnly` when references contain only HTTP(S) URLs and no local cache image exists.
6. `Missing` when no valid local cover source exists.

- [ ] Add tests for each precedence level and for invalid/oversized image data.
- [ ] Add a test proving an existing valid output cover is not replaced by a recovered cover.
- [ ] Add a test proving a FLAC without embedded artwork receives a database BLOB cover in an MP3 metadata-only update without audio transcoding.
- [ ] Add a test proving an HTTP(S) `picUrl` is never treated as a filesystem path and does not trigger network access.
- [ ] Run focused tests and confirm expected failures.
- [ ] Return both cover bytes and `NeteaseCoverSource` from local cover resolution.
- [ ] Keep the existing supported-image and maximum-size validation at every input boundary.
- [ ] Merge recovered artwork only when the destination lacks a valid cover; preserve title, artist, album, genre, lyrics, and analysis fields already present.
- [ ] Ensure metadata-only refresh writes and re-reads the tag, and reports output verification failure if the cover is not present afterward.
- [ ] Run focused tests and confirm they pass.

**Task acceptance:** Locally available database/cache artwork is added to missing-cover FLAC outputs without re-encoding, while existing artwork is preserved and remote-only references are explicitly diagnosed.

---

### Task 5: Pass the resolver through every conversion path

**Files:**

- Modify: `src/sync.rs`
- Modify: `src-tauri/src/main.rs`
- Test: unit tests in `src/sync.rs`
- Test: Tauri unit tests in `src-tauri/src/main.rs`

**Interface direction:**

```rust
pub struct ConversionMetadataContext {
    pub netease: Arc<NeteaseMetadataResolver>,
}
```

- [ ] Add tests covering new conversion, existing-output `仅更新元数据`, retry, resumed batch, slot 1, and slot 2.
- [ ] Run focused tests and confirm at least the manual-database case fails before implementation.
- [ ] Add `ConversionMetadataContext` to the transactional coordinator and worker calls; clone only the `Arc`, never reload the database in a worker.
- [ ] Replace desktop-path calls to process-global `recover_local_metadata(source_path)` with `context.netease.recover(source_path)`.
- [ ] Attach each recovery diagnostic to the corresponding `MetadataDiagnostic` and runtime-session candidate.
- [ ] Preserve CLI behavior by constructing an automatic resolver at the CLI boundary rather than inside each file operation.
- [ ] Confirm cancellation, pause/resume, errors, and worker panic handling do not retain SQLite connections or block task termination.
- [ ] Run `cargo test --all` and Tauri tests.

**Task acceptance:** Every desktop conversion path uses the same batch resolver and database choice, including metadata-only refresh and retries.

---

### Task 6: Complete manual diagnostic exports

**Files:**

- Modify: `src/history.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `tests/history.rs`
- Test: Tauri unit tests in `src-tauri/src/main.rs`

- [ ] Add report fixtures for embedded cover, database BLOB, explicit local path, cache hit, remote-only URL, no matching row, ambiguous row, invalid image, and verified output-write failure.
- [ ] Run report tests and confirm failure before formatting changes.
- [ ] Add a `[网易云数据库与封面恢复]` batch section containing effective path, preference/fallback source, database loaded status, record count, matched count, ambiguous count, no-match count, local cover success count, remote-only count, and missing/invalid count.
- [ ] Add per-track fields for match method, redacted track/album IDs if privacy policy requires it, cover source, byte count, and terminal reason.
- [ ] Keep report generation manual. Do not automatically create or overwrite an exported report.
- [ ] Ensure JSON runtime-session export and text error report derive from the same typed diagnostics.
- [ ] Run history and Tauri tests.

**Task acceptance:** A future report alone can determine whether a missing FLAC cover came from database selection, matching, local cache resolution, image validation, or output writing.

---

### Task 7: Documentation, full verification, and real-data acceptance

**Files:**

- Modify after implementation: `计划.md`
- Modify after implementation: `docs/project-state.md`
- Modify after implementation: `docs/handoff.md`

- [ ] Update project documents with the explicit resolver architecture, read-only guarantee, local-only artwork policy, metadata-only backfill behavior, and report schema.
- [ ] Run:

```bash
cargo test --test history
cargo test --all
cargo test --manifest-path src-tauri/Cargo.toml
cargo fmt --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
pnpm --dir app test -- --run
pnpm --dir app build
git diff --check
```

- [ ] Build the latest macOS application using the repository's established Tauri build command and provide a clickable absolute link to the resulting `.app` or installer.
- [ ] Re-run `仅更新元数据` against the existing output directory; do not delete or re-encode existing audio.
- [ ] Export a new runtime-session JSON and text error report manually.
- [ ] Use ExifTool to verify representative outputs for all locally resolvable cover sources and confirm existing analysis tags remain unchanged.
- [ ] Compare database file size and modification timestamp before/after the run to prove read-only behavior.
- [ ] Record environment-limited items rather than skipping independent verification.
- [ ] Show `git status --short` and `git diff --stat`; do not commit or push.

---

## Final Acceptance Criteria

### Database selection and safety

- [ ] A valid manually selected NetEase database is the effective database used by conversion and metadata-only refresh.
- [ ] An invalid manual path produces a visible warning and falls back to automatic discovery without deleting the preference.
- [ ] The runtime session records effective database path, selection source, load status, supported schema result, and loaded record count.
- [ ] The NetEase database, WAL, and SHM are unchanged after conversion and report export.
- [ ] The database is loaded once per batch rather than once per file.

### Matching correctness

- [ ] Every FLAC is classified as matched, no match, or ambiguous; no item remains unexplained.
- [ ] Matching accepts exact/suffix path, filename plus size, or filename plus verified title/artist identity.
- [ ] Filename-only collisions and tied incompatible rows remain rejected.
- [ ] A database row retaining an `.ncm` name can match the corresponding `.flac` only with additional reliable evidence.

### Cover recovery and output integrity

- [ ] For every FLAC whose embedded tag, matched database BLOB, explicit local path, or local NetEase cache contains a valid image, the destination contains a valid cover after metadata-only refresh.
- [ ] The count `source has valid/recovered cover && output has no valid cover` is exactly zero.
- [ ] Existing valid output covers are unchanged.
- [ ] Audio stream properties and file duration remain unchanged during metadata-only backfill; no audio transcoding occurs.
- [ ] Existing BPM, Key, LUFS, Energy, Danceability, Genre, lyrics, aliases, and analysis-result fields remain present after cover backfill.
- [ ] Remote-only `picUrl` records are reported as `remoteOnly`; they are not silently reported as database misses and do not trigger network traffic.
- [ ] Invalid or oversized image data is rejected and reported without corrupting the output file.

### Reporting

- [ ] Manual text and JSON exports include database selection summary and complete per-track cover recovery status.
- [ ] Missing covers are attributable to exactly one terminal category: database unavailable, no match, ambiguous, remote-only, local file missing, invalid image, or output verification failure.
- [ ] Old history/runtime-session files without the new fields remain readable.
- [ ] Reports are generated only after the user manually selects an export path.

### Real batch regression

- [ ] The supplied 89-FLAC batch is rechecked using unique source/destination pairs; duplicate report projections are not double-counted.
- [ ] The existing baseline of 21 recoverable/preserved FLAC covers does not regress.
- [ ] The previous 68 missing-cover FLACs are all assigned an evidence-backed terminal category.
- [ ] Any of those 68 with locally available database/cache artwork gain a verified output cover.
- [ ] Items whose only source is a remote URL remain explicitly listed for a separately approved online-download feature rather than being called fixed.

## Explicitly Out of Scope

- Downloading cover art from NetEase or third-party HTTP(S) URLs.
- Modifying the NetEase database or using it as the Dashboard source of truth.
- Replacing existing valid artwork based on database preference.
- Audio re-encoding solely to update cover metadata.
- Version changes, Git commits, pushes, merges, releases, or publication.

## 2026-08-24 验收入口迁移

后续可自动化的 FLAC/数据库验收改由隐藏场景和只读报告执行；真实素材、ExifTool 写后回读及外部人工步骤仍需独立证据，不打开 W4DJ GUI。
