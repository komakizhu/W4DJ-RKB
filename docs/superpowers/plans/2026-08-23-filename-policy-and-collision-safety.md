# W4DJ Filename Policy and Collision Safety Implementation Plan

## Execution status (2026-08-25)

The shared worktree contains the identity/safe-name boundary, source-path
collision retention, explicit preview collision handling, cross-format deletion
protection, platform-safe truncation/reserved-name handling, and scan-cache
schema 2 invalidation. The 2026-08-25 rule-priority amendment is implemented:
the selected `FilenameRule` controls ordering, while only embedded NUL and
ASCII `/` are transformed at the final macOS filename boundary. Rust focused
and full automation is green, frontend Vitest and Vite are green, and the
arm64 App was rebuilt at version `3.2.0-beta.3`.

## Amendment (2026-08-25): configured filename rule has priority

The earlier `PreserveSource` wording is superseded for the conversion path.
The selected `FilenameRule` remains authoritative for the output filename:

- `TitleArtist` produces `title - artist`.
- `ArtistTitle` produces `artist - title`.
- `Original` uses the source file basename as the explicit original-name rule.

For `TitleArtist` and `ArtistTitle`, values come from the resolved source
metadata identity; the source basename is used only when the required metadata
field is missing. Metadata identity is never rewritten. Filename-only handling
is limited to characters that cannot be represented in one macOS path
component: an embedded NUL is rendered as `, ` to preserve a multi-artist
separator, and ASCII `/` is rendered as full-width `／` so the visual title is
kept without creating a subdirectory. Quotes, Unicode, punctuation, and other
valid characters remain unchanged. This is not a new broad cross-platform
sanitization pass.

The filename stem used for the destination path is separate from the source
metadata used for tags, cover art, W4DJ matching, and analysis. The five real
NUL-artist failures below are therefore required acceptance fixtures. The
implementation must not claim this amendment complete until all five produce
the rule-selected names and retain their source metadata and cover art.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Separate authoritative song metadata from filesystem-safe names, prevent cleaned-name collisions from dropping or overwriting tracks, and make filename behavior safe and diagnosable across macOS and Windows.

**Architecture:** Introduce a focused filename policy module that returns an authoritative SongIdentity and a separate SafeOutputName. Scanning retains one record per source path and resolves destination collisions only during preview; metadata writing consumes the authoritative identity and never consumes a cleaned filename. Cross-format cleanup is disabled unless the coordinator can prove that a previous output belongs to the same source.

**Tech Stack:** Rust 2024, id3, metaflac, ncmdump, walkdir, Serde, existing W4DJ scan cache and preview pipeline, optional unicode-normalization and unicode-segmentation crates.

## Global Constraints

- Keep the product version at 3.2.0-beta.3.
- Do not commit, push, merge, create a release, or publish artifacts until the user says “定稿”.
- Preserve the current dirty worktree and unrelated changes.
- Do not automatically rename, overwrite, repair, or delete existing user audio.
- Reliable source title, artist, album, and artwork remain authoritative metadata.
- Filename cleaning is permitted only at the final path-generation boundary.
- No scan candidate may disappear because another source has the same cleaned name.
- No audio file may be deleted solely because it shares a cleaned stem with another output.
- Do not add file hashes, baselines, frozen contracts, or release gates.
- All new serialized fields must use Serde defaults so existing cache and history files remain readable.

---

## Target File Structure

- Create src/filename_policy.rs: song identity resolution, filename-only presentation cleanup, path sanitization, Unicode-aware truncation, and conservative collision keys.
- Modify src/lib.rs: expose the filename policy module to sync and preview.
- Modify src/sync.rs: consume SongIdentity and SafeOutputName, preserve metadata, return one scan result per source path, and stop unowned cross-format deletion.
- Modify src/preview.rs: group destination collisions and apply explicit conflict-strategy behavior.
- Modify src/scan_cache.rs: invalidate legacy derived-name cache entries and persist the new safe-output fields.
- Modify src/history.rs: include filename transformation and collision diagnostics in manual reports.
- Modify Cargo.toml and Cargo.lock only if unicode-normalization and unicode-segmentation are required.
- Modify tests/sync_policy.rs and tests/preview.rs; keep focused unit tests beside src/filename_policy.rs and src/scan_cache.rs.
- Update 计划.md, docs/project-state.md, and docs/handoff.md after implementation and acceptance.

---

### Task 1: Add Characterization and Failure Tests

**Files:**

- Modify: src/sync.rs test module
- Modify: tests/sync_policy.rs
- Modify: tests/preview.rs

**Interfaces:**

- Consumes: current derive_song_name_with_settings, ensure_output_metadata_with_settings, scan, and preview behavior.
- Produces: failing regression tests that define the corrected behavior before implementation.

- [ ] **Step 1: Add the reliable-title separation test**

Create a tagged source and destination in a temporary directory. Use this exact title and artist:

    let title = r#"Mass Destruction ("P3" + "P3F" ver.)"#;
    let artist = "川村ゆみ, Lotus Juice";

Assert that the safe output stem may be:

    Mass Destruction ("P3" + "P3F" ver.) - 川村ゆみ, Lotus Juice

but the destination ID3 title after metadata finalization is exactly:

    Mass Destruction ("P3" + "P3F" ver.)

- [ ] **Step 2: Add semantic-preservation cases**

Add table-driven assertions that reliable source titles remain unchanged:

    Song (Claudio)
    Song (Credits)
    Song (Liverpool Mix)
    Dancing With Myself
    Live and Let Die
    AC/DC
    Vol.2
    《Title》

The filename may replace characters forbidden by the filesystem, but metadata must match the source string exactly.

- [ ] **Step 3: Add cleaned-name collision tests**

Scan two distinct source paths whose titles are:

    A:B
    A?B

Assert that both source paths survive scanning and both reach preview. Add equivalent cases for:

    Song / song
    Café / Café

where the second Café uses a decomposed combining accent.

- [ ] **Step 4: Add cross-format deletion protection**

Create a prior A-B.flac owned by a different source and convert an A:B source to A-B.mp3. Assert that the FLAC remains untouched.

- [ ] **Step 5: Run the focused tests and record the expected failures**

Run:

    cargo test --test sync_policy -- --nocapture
    cargo test --test preview -- --nocapture
    cargo test sync::tests -- --nocapture

Expected before implementation: failures for exact title preservation, candidate retention, collision handling, and cross-format deletion protection. Existing unrelated tests must continue running.

---

### Task 2: Separate Authoritative Identity from Filesystem Names

**Files:**

- Create: src/filename_policy.rs
- Modify: src/lib.rs
- Modify: src/sync.rs
- Test: src/filename_policy.rs

**Interfaces:**

- Produces:

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum IdentityBasis {
        SourceTags,
        FilenameInference,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SongIdentity {
        pub title: String,
        pub artist: String,
        pub basis: IdentityBasis,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SafeOutputName {
        pub stem: String,
        pub collision_key: String,
        pub transformations: Vec<String>,
    }

    pub fn resolve_song_identity(
        fallback_stem: &str,
        source_title: Option<&str>,
        source_artist: Option<&str>,
        netease_format: NeteaseFilenameFormat,
    ) -> SongIdentity;

    pub fn build_safe_output_name(
        identity: &SongIdentity,
        rule: FilenameRule,
        extension: &str,
        rename_index: Option<usize>,
    ) -> SafeOutputName;

- [ ] **Step 1: Implement source-tag authority**

When both source title and source artist are non-empty, return them after trim-only processing and set basis to SourceTags. Do not call normalize_display_text, strip_promotional_suffixes, normalize_collaboration_markers, or sanitize_filename_component.

- [ ] **Step 2: Implement filename fallback identity**

Only when a source field is missing, parse the fallback stem according to NeteaseFilenameFormat. Preserve the parsed text as identity data; do not apply filesystem substitutions while resolving identity.

- [ ] **Step 3: Move path-only transformations into build_safe_output_name**

Apply FilenameRule ordering, optional filename presentation cleanup, invalid-character replacement, reserved-name handling, length budgeting, and collision-key generation only after SongIdentity is complete.

- [ ] **Step 4: Replace the duplicated identity paths in sync.rs**

Update song_name_from_ncm, song_name_from_flac, song_name_from_audio_tag, derive_song_name_with_settings, and ensure_output_metadata_with_settings to consume the new types.

- [ ] **Step 5: Make metadata finalization preserve reliable fields**

Replace the current behavior that overwrites any unequal title or artist. Normal conversion must:

    if output title is blank, copy source title;
    if output artist is blank, copy source artist;
    if output contains the exact source value, leave it unchanged;
    if output differs and the operation is not explicit metadata repair, preserve the already embedded source-derived value and report the mismatch.

The explicit UpdateMetadata operation may overwrite title and artist with reliable source tags.

- [ ] **Step 6: Run focused tests**

Run:

    cargo test filename_policy -- --nocapture
    cargo test sync::tests -- --nocapture

Expected: the Mass Destruction metadata test and semantic-preservation tests pass while existing naming-rule tests remain green or are updated only where the corrected contract intentionally changes them.

---

### Task 3: Preserve Every Source Candidate During Scanning

**Files:**

- Modify: src/sync.rs
- Modify: src/preview.rs
- Test: tests/sync_policy.rs
- Test: tests/preview.rs

**Interfaces:**

- Consumes: SongIdentity and SafeOutputName from Task 2.
- Produces:

    #[derive(Debug, Clone)]
    pub struct ScannedTrack {
        pub source_path: PathBuf,
        pub size_bytes: u64,
        pub source_extension: String,
        pub identity: SongIdentity,
        pub output_name: SafeOutputName,
    }

- [ ] **Step 1: Stop keying source scans by cleaned song name**

Replace the source HashMap keyed by derived_name with a collection keyed by normalized source path or a Vec<ScannedTrack>. A later item must never replace an earlier item because the output stems match.

- [ ] **Step 2: Remove quality-based silent selection from collisions**

Do not call should_prefer_file when two different source paths share an output stem. If legacy duplicate-format preference remains necessary, require an explicit equivalence condition based on the same source record; otherwise keep both candidates.

- [ ] **Step 3: Group collisions in preview**

Compute a collision group from the complete destination filename, including extension. The collision key must be Unicode-normalized and case-folded conservatively so macOS and Windows collisions are detected before conversion.

- [ ] **Step 4: Define conflict-strategy behavior**

Apply these exact rules:

- Rename: produce Name.ext, Name (2).ext, Name (3).ext while respecting the length budget.
- Skip: retain the candidate in the preview result with a nameCollision issue and mark it skipped explicitly.
- Overwrite: may overwrite a file that existed before the batch, but two sources in the same batch may not overwrite one another; report a nameCollision issue.
- UpdateMetadata: reject ambiguous source ownership with a nameCollision issue.

- [ ] **Step 5: Run focused tests**

Run:

    cargo test --test sync_policy -- --nocapture
    cargo test --test preview -- --nocapture

Expected: A:B and A?B both appear in preview; Rename produces deterministic unique names; other strategies report an explicit collision rather than dropping a track.

---

### Task 4: Remove Unowned Cross-Format Deletion

**Files:**

- Modify: src/sync.rs
- Modify: src-tauri/src/main.rs only if the coordinator supplies proven ownership
- Test: tests/sync_policy.rs

**Interfaces:**

- Consumes: source_path and destination_path for the completed track.
- Produces either:

    no automatic cross-format deletion;

or an ownership-proven API:

    pub fn remove_owned_superseded_outputs(
        protected_source_path: &Path,
        current_destination: &Path,
        owned_previous_destinations: &[PathBuf],
    ) -> io::Result<Vec<PathBuf>>;

- [ ] **Step 1: Remove the unconditional remove_conflicting_outputs call**

Do not derive deletion targets from dest_folder plus cleaned name_stem.

- [ ] **Step 2: If ownership data is already available, pass explicit paths**

Only paths recorded by w4dj.sqlite3 as previous outputs for the same normalized source path may be considered. Revalidate that each path is inside the applied output root, is not the protected source, and is not the current destination.

- [ ] **Step 3: Preserve files when ownership cannot be proven**

Return a non-fatal warning describing the retained superseded format. Do not delete it.

- [ ] **Step 4: Run deletion safety tests**

Run:

    cargo test --test sync_policy -- --nocapture

Expected: no distinct track is removed because of a stem collision; same-source cleanup occurs only when explicit ownership paths are supplied.

---

### Task 5: Make Filename Sanitization Unicode- and Platform-Safe

**Files:**

- Modify: src/filename_policy.rs
- Modify: Cargo.toml
- Modify: Cargo.lock
- Test: src/filename_policy.rs

**Interfaces:**

- Consumes: raw ordered stem, output extension, and optional rename suffix.
- Produces: a non-empty safe stem plus a conservative collision key.

- [ ] **Step 1: Add Unicode support if not already available**

Add direct dependencies:

    unicode-normalization = "0.1"
    unicode-segmentation = "1"

- [ ] **Step 2: Sanitize illegal path characters**

Replace Windows-forbidden characters and control characters only in the filesystem stem. Preserve quotes and punctuation in SongIdentity.

- [ ] **Step 3: Enforce complete-component limits**

Budget the stem, rename suffix, dot, and extension together. Keep the complete filename at or below 240 UTF-8 bytes and 240 UTF-16 code units. Truncate by grapheme cluster, then repeat trailing-space and trailing-dot cleanup.

- [ ] **Step 4: Handle reserved and hidden names**

Prefix Windows device names such as CON, PRN, AUX, NUL, COM1 through COM9, and LPT1 through LPT9. Prefix stems beginning with a dot so valid outputs do not become hidden on macOS or Linux. Guarantee a non-empty 未命名 fallback.

- [ ] **Step 5: Tighten semantic cleanup**

Run promotional cleanup only for FilenameInference. Replace substring checks with exact normalized phrase checks. Do not automatically remove Live, Instrumental, Radio Edit, Club Edit, Extended Mix, Remastered, or other version-defining text. Do not rewrite standalone with, x, or × inside reliable titles.

- [ ] **Step 6: Add length and normalization tests**

Cover:

    200 Chinese characters;
    repeated multi-code-point emoji;
    a name whose truncation point previously ended on a space or dot;
    leading-dot names;
    Windows device names with extensions;
    NFC and NFD equivalents;
    a rename suffix appended near the length limit.

- [ ] **Step 7: Run focused tests**

Run:

    cargo test filename_policy -- --nocapture

Expected: every generated component satisfies both limits, remains non-empty, avoids trailing invalid characters, and produces deterministic collision keys.

#### Current-scope override for Task 5

For the 2026-08-25 regression, do not expand the implementation into the
historical Windows-reserved-name, length-budget, Unicode-normalization, or
semantic-cleanup work described above. The only filename transformations in
this acceptance are NUL → `, ` and ASCII `/` → full-width `／`, applied after
the selected `FilenameRule` has produced the ordered stem. Existing task 2
SoundCloud behavior and previously completed safety code remain unchanged.

---

### Task 6: Invalidate Stale Scan Names and Improve Diagnostics

**Files:**

- Modify: src/scan_cache.rs
- Modify: src/sync.rs
- Modify: src/history.rs
- Test: src/scan_cache.rs
- Test: tests/history.rs

**Interfaces:**

- Consumes: SafeOutputName.transformations and collision information.
- Produces: scan cache schema 2 and report fields that separate metadata identity from filename policy.

- [ ] **Step 1: Bump the internal scan cache schema**

Set SCAN_CACHE_SCHEMA_VERSION to 2. Loading schema 1 must return an empty cache and trigger a rescan rather than surfacing a blocking error or reusing derived_name.

- [ ] **Step 2: Persist the corrected cache fields**

Cache the source-path identity inputs and safe output name separately. Continue using Serde defaults for new optional fields.

- [ ] **Step 3: Record filename diagnostics**

For each candidate record:

    identity basis: sourceTags or filenameInference;
    source title and artist;
    safe output stem;
    applied transformations;
    collision group and resolution;
    metadata read-back result.

- [ ] **Step 4: Keep manual reports unambiguous**

The report must distinguish “文件名按安全策略变化” from “音频标签与源标签不一致”. A changed filename alone must not cause output_tags_match=false.

- [ ] **Step 5: Run cache and report tests**

Run:

    cargo test scan_cache -- --nocapture
    cargo test --test history -- --nocapture

Expected: schema 1 safely rebuilds, schema 2 round-trips, and reports show independent filename and metadata results.

---

### Task 7: Repair Existing Outputs Only on Explicit User Action

**Files:**

- Modify: src/sync.rs
- Modify: src-tauri/src/main.rs if command diagnostics need additional fields
- Test: tests/sync_policy.rs

**Interfaces:**

- Consumes: existing UpdateMetadata operation and reliable source tags.
- Produces: transactional repair of metadata without renaming the destination.

- [ ] **Step 1: Route UpdateMetadata through authoritative SongIdentity**

For a source that still exists, copy exact source title, artist, album, artwork, and lyrics to the temporary destination, write atomically, then read back.

- [ ] **Step 2: Do not infer missing originals from a cleaned output name**

If the source no longer exists and no reliable library record contains the original title, return an explicit repair error and preserve the output unchanged.

- [ ] **Step 3: Add the Mass Destruction repair test**

Start with an output title of:

    Mass Destruction ("P3" + "P3F" ver.)

Run explicit metadata repair from a reliable source title of:

    Mass Destruction ("P3" + "P3F" ver.)

Assert that the destination filename is unchanged, the title is restored exactly, and artist, album, and artwork remain present.

- [ ] **Step 4: Run repair tests**

Run:

    cargo test --test sync_policy -- --nocapture

Expected: explicit repair succeeds transactionally; unavailable source data produces no mutation.

---

## Acceptance Plan

### Automated Acceptance

- [ ] Run all focused Rust tests:

    cargo test filename_policy -- --nocapture
    cargo test scan_cache -- --nocapture
    cargo test sync::tests -- --nocapture
    cargo test --test sync_policy -- --nocapture
    cargo test --test preview -- --nocapture
    cargo test --test history -- --nocapture

- [ ] Run the complete Rust suite:

    cargo test --all

Expected: all tests pass. No test may be ignored merely because it exposes an existing filename collision.

- [ ] Run frontend regression checks:

    pnpm --dir app test -- --run
    pnpm --dir app build

Expected: all Vitest files pass and the Vite production build completes.

- [ ] Run formatting and static checks:

    cargo fmt --all -- --check
    cargo check --manifest-path src-tauri/Cargo.toml
    cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
    git diff --check

Expected: formatting, check, Clippy, and whitespace validation pass. If strict all-targets exposes a documented pre-existing warning, record the exact warning and prove it is outside this change instead of weakening the command.

### Fixture Acceptance Matrix

- [ ] Verify the following deterministic matrix with temporary files:

| Case | Source title | Expected filename behavior | Expected metadata behavior |
|---|---|---|---|
| Quotes | Mass Destruction ("P3" + "P3F" ver.) | Quotes remain unchanged | Title remains exact |
| Invalid collision | A:B and A?B | Explicit collision or Rename suffix | Both tracks retained |
| Case collision | Song and song | Explicit collision on conservative key | Both tracks retained |
| Unicode collision | Café and decomposed Café | Explicit collision on conservative key | Both tracks retained |
| Version suffix | Song (Live) | Version remains in name | Title remains exact |
| False promotion | Song (Claudio) | Claudio remains | Title remains exact |
| Collaboration word | Dancing With Myself | With is not rewritten | Title remains exact |
| Slash | AC/DC | ASCII `/` becomes full-width `／` | Title remains AC/DC |
| Long CJK | 200 Chinese characters | Safely truncated within both limits | Full title retained |
| Emoji | repeated family emoji | Grapheme-safe truncation | Full title retained |

Every row fails acceptance if a source disappears before preview, a destination is overwritten by another source, or metadata inherits the cleaned filename.

### Real NetEase Acceptance

- [ ] Use the original NetEase source for:

    Mass Destruction ("P3" + "P3F" ver.)

- [ ] Run a normal compat conversion with title_artist naming into an empty temporary output directory.

- [ ] Confirm the preview shows exactly one candidate for that source and displays any filename transformation separately from metadata.

- [ ] Inspect the output with ExifTool:

    exiftool -Title -Artist -Album -Picture -FileName "/path/to/output.mp3"

Expected:

    Title: Mass Destruction ("P3" + "P3F" ver.)
    Artist: 川村ゆみ, Lotus Juice
    Album: 『P3D』＆『P5D』フルサウンドトラック
    Artwork remains present
    FileName keeps the quotes because they are valid on macOS

- [ ] Run explicit UpdateMetadata against a copy of the previously broken output.

Expected: filename remains unchanged, Title is restored exactly, and read-back validation passes.

- [ ] Export the error report manually.

Expected: the report states that the filename was safely transformed, metadata matches reliable source tags, output_tags_match=true, and no collision or deletion warning is present.

### Collision and Deletion Acceptance

- [ ] Place A:B and A?B sources in different input subdirectories and run all four conflict strategies.

Expected:

- Rename creates two distinct outputs.
- Skip reports the second collision explicitly.
- Overwrite does not let two current-batch sources overwrite each other.
- UpdateMetadata rejects ambiguous ownership.

- [ ] Place an unrelated A-B.flac in the output directory before converting A:B to MP3.

Expected: the FLAC remains byte-for-byte present. No cleanup is allowed without a same-source ownership record.

- [ ] Repeat the collision test with Song/song and NFC/NFD Café names on the current macOS filesystem.

Expected: collisions are resolved during preview, not by filesystem errors during conversion.

### Cache Upgrade Acceptance

- [ ] Start with a schema 1 scan cache containing the old cleaned Mass Destruction derived_name.

- [ ] Launch the corrected scan.

Expected: schema 1 is discarded as cache data, the source is rescanned, the task does not fail, and schema 2 is saved atomically.

- [ ] Run the same scan again without changing the source.

Expected: schema 2 is reused and returns the same identity, safe output name, and collision key.

### Application Build Acceptance

- [ ] Build the current desktop application using the repository’s established Tauri packaging workflow after all automated checks pass.

- [ ] Launch the built application on macOS and complete one normal conversion plus one explicit metadata repair.

- [ ] Verify that the application remains responsive, history records the correct source and destination, and Dashboard registration uses the actual destination path.

- [ ] Provide the user a clickable absolute path to the newest W4DJ RKB.app build. Do not replace or delete older user builds automatically.

### Environment-Limited Acceptance

- [ ] Record Windows filename validation as not executed if no Windows environment is available.

- [ ] Provide these Windows manual steps:

    convert the fixture matrix on NTFS;
    confirm no reserved-name or trailing-dot failure;
    verify case-insensitive collision preview;
    inspect MP3 tags with ExifTool;
    confirm no source or unrelated output is deleted.

- [ ] Do not block macOS completion on Windows availability, but do not claim Windows acceptance passed without executing it.

---

### Rule-Priority Acceptance: five real NUL-artist fixtures

The following five records are mandatory real-data acceptance cases from
`summary-slot-1.json`. They must be tested with the currently selected
`TitleArtist` rule; if another rule is selected, only the order changes.

| Source file basename | Source Artist tag | Expected `TitleArtist` filename stem |
|---|---|---|
| `Kalawanji - Kromestar,Cessman.mp3` | `Kromestar\0Cessman` | `Kalawanji - Kromestar, Cessman` |
| `Waiting For Tremor (COOL BROS Edit).mp3` | `COOL BROS\0Avicii\0Dimitri Vegas & Like Mike\0Martin Garrix\0Daft Punk` | `Waiting For Tremor (COOL BROS Edit) - COOL BROS, Avicii, Dimitri Vegas & Like Mike, Martin Garrix, Daft Punk` |
| `バギー・ブギー／Buggy Boogie.mp3` | `ミッキー吉野\0小林亜星` | `バギー・ブギー／Buggy Boogie - ミッキー吉野, 小林亜星` |
| `爱是甜的.mp3` | `叶树茵\0方文琳\0于冠华\0米志宏\0郭子` | `爱是甜的 - 叶树茵, 方文琳, 于冠华, 米志宏, 郭子` |
| `通撒美.mp3` | `老黑\0司岗里阿妹` | `通撒美 - 老黑, 司岗里阿妹` |

- [x] Each source file is read from its original source path; no filename is
  reconstructed from the NetEase database title.
- [x] Each output follows the selected filename rule and contains no NUL or
  ASCII `/`; only those two unrepresentable characters may change.
- [x] No record reports `file name contained an unexpected NUL byte`.
- [x] The source Artist tag, Title tag, album, and cover remain unchanged or
  are copied according to the existing metadata policy; filename conversion
  alone must not change them.
- [ ] ExifTool confirms output Artist/Title and cover presence for all five;
  the W4DJ database and analysis cache point to the actual output path.
- [ ] The 10 previously observed titles containing ASCII `/` are included as
  a secondary sweep; their output uses full-width `／` and never creates an
  unintended directory.
- [x] The two unrelated duplicate-output failures remain separately reported
  and are not counted as NUL/name-rule failures.

## Completion Record

2026-08-25 implementation and acceptance record: `src/sync.rs` now applies
the selected rule before the conservative NUL/ASCII-slash path conversion;
`src/preview.rs` uses the same final path boundary. Five real MP3 fixtures
were copied to a temporary directory, converted with the PreserveSource
policy, and checked for the expected stems, source Title/Artist and cover
frames. No user output or database was changed. The arm64 App was rebuilt at
`src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`.

After implementation and acceptance:

- [x] Update 计划.md with the completed task and remaining environment limits.
- [x] Update docs/project-state.md with the new identity/path boundary and actual test counts.
- [x] Update docs/handoff.md with cache schema 2, collision behavior, repair behavior, and manual acceptance steps.
- [x] Run:

    git status --short
    git diff --stat

- [ ] Report changed files, automated results, real NetEase results, environment-limited checks, current git status, and git diff stat.
- [ ] Wait for user confirmation. Do not commit or push unless the user says “定稿”.
