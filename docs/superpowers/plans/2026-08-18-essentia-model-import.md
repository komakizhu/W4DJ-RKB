# Essentia Model Import Implementation Plan

> **2026-08-18 implementation outcome:** After this import plan was approved and implemented, the user approved bundling the small model set and TensorFlow.js. The final runtime therefore uses bundled local resources; the download-oriented names and the separate `genre_rosamerica` head shown in the original steps below are historical. MusiCNN's own 50-tag output now supplies the broad Genre projection. On 2026-08-22 the user simplified the product surface: the three model-maintenance buttons and full-window model drag/drop were removed from the frontend; Rust validation/restore/import commands remain only as compatibility maintenance paths.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Do not dispatch subagents unless the user explicitly asks for subagent execution. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an official Essentia model web link, a multi-file import button, and full-window model drag-and-drop while preserving the existing song/folder drop behavior.

**Architecture:** Keep network download and installed-model status in the existing Tauri command layer, but put untrusted ZIP/JSON/BIN parsing and transactional pair installation in a focused Rust module. The frontend classifies incoming paths before the existing slot hit-test: model candidates use a full-window backdrop, ordinary audio/folder paths use the current task-slot routing, and mixed drops are rejected.

**Tech Stack:** Rust 2024, Tauri 2, `serde_json`, existing `flate2`, TypeScript, DOM/CSS backdrop filters, Vitest/jsdom.

## Global Constraints

- Product version remains W4DJ 3.2.0 beta-3; code SemVer remains `3.2.0-beta.3`.
- Preserve all existing tracked and untracked worktree changes; do not reset, clean, checkout, or rewrite unrelated files.
- Do not add a model marketplace, automatic model updates, arbitrary TensorFlow/PyTorch/ONNX support, uploads, hash baselines, frozen contracts, or release gates.
- Model files may only be installed under the application-owned Essentia model directory.
- Do not start conversion or analysis and do not write conversion history while importing models.
- Do not commit, push, merge, create a release, or publish artifacts. Repository rules require the user to say `定稿` before any commit.
- Implementation must not begin until the user explicitly approves this plan.

## File Structure

- Create `src-tauri/src/essentia_model_import.rs`: bounded input reading, official ZIP extraction, TensorFlow.js manifest validation, known-model identification, staged installation, rollback, and pure unit tests.
- Modify `src-tauri/src/main.rs`: connect the importer to existing model specs/status, serialize model writes, add the import DTO/command, allow the fixed official URL, and register the command.
- Modify `app/src/app.ts`: add DTO/service methods, picker and official-link actions, model import state, drop classification, full-window drop routing, and overlay markup.
- Modify `app/src/styles.css`: style the full-window blur overlay and model action buttons, including reduced-motion behavior.
- Modify `app/src/app.test.ts`: cover buttons, picker normalization, import result state, path classification, overlay behavior, mixed rejection, and unchanged song/folder routing.
- Modify `docs/project-state.md`, `docs/handoff.md`, and `计划.md`: record implementation and exact validation outcomes only after code and tests pass.

---

### Task 1: Tauri model contracts, write serialization, and official URL

**Files:**
- Modify: `src-tauri/src/main.rs:557-980`
- Modify: `src-tauri/src/main.rs:2598-2620`
- Modify: `src-tauri/src/main.rs:3440-3520`
- Test: `src-tauri/src/main.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: the existing runtime model specs/status, downloader, and `open_external_url` command.
- Produces:

```rust
const ESSENTIA_MODELS_URL: &str = "https://essentia.upf.edu/models/";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EssentiaModelImportIssueDto { file_name: String, reason: String }

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EssentiaModelImportResult {
    installed_ids: Vec<String>,
    issues: Vec<EssentiaModelImportIssueDto>,
    missing_ids: Vec<String>,
    status: EssentiaModelStatus,
    message: String,
}

```

- [x] **Step 1: Write failing integration tests**

Add exact allowlist and casing tests:

```rust
#[test]
fn external_url_allowlist_includes_only_project_and_official_essentia_pages() {
    assert!(external_url_is_allowed("https://essentia.upf.edu/models/"));
    assert!(!external_url_is_allowed("https://essentia.upf.edu.evil.example/models/"));
    assert!(!external_url_is_allowed("http://essentia.upf.edu/models/"));
}

#[test]
fn essentia_import_result_uses_camel_case() {
    let value = serde_json::to_value(EssentiaModelImportResult {
        installed_ids: vec!["musicnn_embedding".into()],
        issues: vec![EssentiaModelImportIssueDto {
            file_name: "model.json".into(),
            reason: "缺少权重".into(),
        }],
        missing_ids: vec!["genre_rosamerica".into()],
        status: EssentiaModelStatus {
            version: ESSENTIA_MODEL_VERSION,
            embedding: true,
            genre: false,
            mood: false,
            instrument: false,
            downloading: false,
        },
        message: "部分导入完成".into(),
    }).unwrap();
    assert!(value.get("installedIds").is_some());
    assert!(value.get("missingIds").is_some());
    assert_eq!(value["issues"][0]["fileName"], "model.json");
}

```

- [x] **Step 2: Run the focused tests and verify failure**

```bash
CARGO_TARGET_DIR=/private/tmp/w4dj-essentia-import cargo test --manifest-path src-tauri/Cargo.toml external_url_allowlist -- --nocapture
CARGO_TARGET_DIR=/private/tmp/w4dj-essentia-import cargo test --manifest-path src-tauri/Cargo.toml essentia_import_result -- --nocapture
```

Expected: failure because the helper and DTO do not exist.

- [x] **Step 3: Add the DTO and serialize existing model downloads**

Add both DTOs exactly as declared above. Add `models_write_lock: Arc<Mutex<()>>` to `AppState`, initialize it in the Tauri builder, and acquire it in `download_essentia_models`. Do not acquire the desktop-controller or library-state locks. This establishes the serialization seam that Task 2's local importer will share.

- [x] **Step 4: Extend the external URL allowlist by exact match**

```rust
fn external_url_is_allowed(url: &str) -> bool {
    const PROJECT_URL: &str = "https://github.com/komakizhu/W4DJ-RKB";
    url == PROJECT_URL
        || url.starts_with("https://github.com/komakizhu/W4DJ-RKB/releases/")
        || url == ESSENTIA_MODELS_URL
}
```

`open_external_url` calls this helper before the existing platform command. Do not permit arbitrary Essentia subpaths, HTTP, or lookalike hosts.

- [x] **Step 5: Run Tauri tests and check**

```bash
CARGO_TARGET_DIR=/private/tmp/w4dj-essentia-import cargo test --manifest-path src-tauri/Cargo.toml -- --nocapture
CARGO_TARGET_DIR=/private/tmp/w4dj-essentia-import cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: all Tauri tests and check pass.

- [x] **Step 6: Review checkpoint without commit**

Run `git diff --check`, inspect `src-tauri/src/main.rs`, and confirm the exact URL allowlist and that the new lock serializes existing model downloads. Do not stage or commit.

---

### Task 2: Safe model importer and Tauri command

**Files:**
- Create: `src-tauri/src/essentia_model_import.rs`
- Modify: `src-tauri/src/main.rs:1-40,557-980,3440-3520`
- Test: `src-tauri/src/essentia_model_import.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: local paths supplied by Tauri, Task 1's DTO/write lock, and the existing runtime model specs/status.
- Produces:

```rust
pub const MAX_IMPORT_FILES: usize = 16;
pub const MAX_MODEL_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_MODEL_BATCH_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_ARCHIVE_OUTPUT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelImportIssue { pub file_name: String, pub reason: String }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelImportReport {
    pub installed_ids: Vec<String>,
    pub issues: Vec<ModelImportIssue>,
}

pub fn import_model_paths(paths: &[PathBuf], models_path: &Path)
    -> Result<ModelImportReport, String>;
pub fn known_import_model_ids() -> &'static [&'static str];

#[tauri::command]
fn import_essentia_models(
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<EssentiaModelImportResult, String>;
```

Known IDs are exactly `musicnn_embedding`, `genre_rosamerica`, `mood_aggressive`, `mood_happy`, `mood_relaxed`, `mood_party`, `mood_sad`, and `voice_instrumental`.

- [x] **Step 1: Write failing validation and security tests**

Declare `mod essentia_model_import;` in `main.rs`. In the new module, use isolated `std::env::temp_dir()` fixtures and assert these exact behaviors:

```rust
#[test]
fn imports_loose_known_tensorflow_model_and_weight() {
    let fixture = TestFixture::new("genre_rosamerica-msd-musicnn-1-tfjs");
    fixture.write_model_json(&["group1-shard1of1.bin"]);
    fixture.write_weight("group1-shard1of1.bin", b"weights");
    let report = import_model_paths(&fixture.input_paths(), fixture.models_path()).unwrap();
    assert_eq!(report.installed_ids, ["genre_rosamerica"]);
    assert!(report.issues.is_empty());
    assert_eq!(fs::read(fixture.models_path().join("genre_rosamerica.bin")).unwrap(), b"weights");
}

#[test]
fn missing_weight_does_not_replace_an_installed_model() {
    let fixture = TestFixture::with_installed_pair("mood_happy", b"old-json", b"old-bin");
    fixture.write_named_json("mood_happy-msd-musicnn-1-tfjs/model.json", &["missing.bin"]);
    let report = import_model_paths(&fixture.input_paths(), fixture.models_path()).unwrap();
    assert!(report.installed_ids.is_empty());
    assert!(report.issues.iter().any(|issue| issue.reason.contains("缺少权重")));
    assert_eq!(fs::read(fixture.models_path().join("mood_happy.bin")).unwrap(), b"old-bin");
}

#[test]
fn ambiguous_renamed_head_is_not_guessed() {
    let fixture = TestFixture::new("renamed-model");
    fixture.write_model_json(&["group1-shard1of1.bin"]);
    fixture.write_weight("group1-shard1of1.bin", b"weights");
    let report = import_model_paths(&fixture.input_paths(), fixture.models_path()).unwrap();
    assert!(report.installed_ids.is_empty());
    assert!(report.issues.iter().any(|issue| issue.reason.contains("无法识别")));
}
```

Add named ZIP fixtures and separate tests for: official MusiCNN layout; the official archive's incorrect extra-length offset; `../` and absolute paths; duplicate entries; nested ZIP; expanded-size overflow; symlink/directory inputs; more than 16 inputs; single-file and aggregate size limits. Every rejected case must assert that no destination pair was created or overwritten.

Add a main-module consistency test so the importer table cannot drift from runtime specs:

```rust
#[test]
fn importer_identity_table_matches_runtime_model_specs() {
    let runtime = essentia_model_specs().into_iter().map(|spec| spec.id).collect::<BTreeSet<_>>();
    let importable = known_import_model_ids().iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(runtime, importable);
}
```

- [x] **Step 2: Run the tests to verify the red state**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/w4dj-essentia-import cargo test --manifest-path src-tauri/Cargo.toml essentia_model_import -- --nocapture
```

Expected: compilation or behavioral failure because the importer is not implemented. Record the first meaningful failure.

- [x] **Step 3: Implement bounded input reading and known-model identification**

Use this exact internal identity table shape:

```rust
struct KnownModelIdentity {
    id: &'static str,
    path_markers: &'static [&'static str],
    archive: bool,
    expected_output_units: Option<u64>,
}

const KNOWN_MODELS: &[KnownModelIdentity] = &[
    KnownModelIdentity { id: "musicnn_embedding", path_markers: &["msd-musicnn-1-tfjs"], archive: true, expected_output_units: None },
    KnownModelIdentity { id: "genre_rosamerica", path_markers: &["genre_rosamerica-msd-musicnn-1-tfjs", "genre_rosamerica"], archive: false, expected_output_units: Some(8) },
    KnownModelIdentity { id: "mood_aggressive", path_markers: &["mood_aggressive-msd-musicnn-1-tfjs", "mood_aggressive"], archive: false, expected_output_units: Some(2) },
    KnownModelIdentity { id: "mood_happy", path_markers: &["mood_happy-msd-musicnn-1-tfjs", "mood_happy"], archive: false, expected_output_units: Some(2) },
    KnownModelIdentity { id: "mood_relaxed", path_markers: &["mood_relaxed-msd-musicnn-1-tfjs", "mood_relaxed"], archive: false, expected_output_units: Some(2) },
    KnownModelIdentity { id: "mood_party", path_markers: &["mood_party-msd-musicnn-1-tfjs", "mood_party"], archive: false, expected_output_units: Some(2) },
    KnownModelIdentity { id: "mood_sad", path_markers: &["mood_sad-msd-musicnn-1-tfjs", "mood_sad"], archive: false, expected_output_units: Some(2) },
    KnownModelIdentity { id: "voice_instrumental", path_markers: &["voice_instrumental-msd-musicnn-1-tfjs", "voice_instrumental"], archive: false, expected_output_units: Some(2) },
];
```

Use `fs::symlink_metadata`; accept only regular files with case-insensitive `zip`, `json`, or `bin` extensions. Enforce all four published limits before reading. Errors contain `file_name()` only.

Parse JSON as `serde_json::Value`; require `modelTopology` and a non-empty `weightsManifest`. Flatten manifest `paths` in order, reject absolute and non-`Normal` components, then concatenate referenced shards in manifest order. Match a loose model only when exactly one identity marker matches the JSON path or parent directory and the final output units equal the runtime class count (8 for Rosamerica, 2 for mood/voice heads). Validate the embedding archive's known MusiCNN input/output layer names. A renamed ambiguous classification head is an issue, never a guess.

- [x] **Step 4: Implement bounded official ZIP extraction**

Implement:

```rust
struct ArchiveEntry { name: String, data: Vec<u8> }
fn extract_model_archive(archive: &[u8]) -> Result<Vec<ArchiveEntry>, String>;
pub fn extract_official_musicnn_pair(archive: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String>;
```

Read ZIP local headers with checked arithmetic. Accept methods 0 and 8. For the official malformed header, search no more than 64 bytes after the declared data start and accept only a stream whose decoded size exactly matches the header. Reject encryption/data descriptors, invalid UTF-8, absolute or parent paths, duplicates, `.zip` entries, more than 32 entries, and aggregate output over 128 MiB. Require one valid `model.json`, every manifest shard, and an `msd-musicnn-1-tfjs` path marker.

- [x] **Step 5: Implement staged pair installation with rollback**

Implement:

```rust
pub fn install_pair_with_rollback(
    models_path: &Path,
    id: &str,
    model_json: &[u8],
    weights: &[u8],
) -> Result<(), String>;
```

Stage both files under unique `.w4dj-import-<pid>-<timestamp>` names, re-read and validate them, move old targets to unique backups, and rename both staged files into place. If either target rename fails, remove only this operation's new target and restore both backups. A scope guard removes only this operation's staging/backup files on ordinary errors.

- [x] **Step 6: Connect the importer command and reuse it for downloads**

In `main.rs`, acquire Task 1's `models_write_lock`, convert strings to `PathBuf`, call `import_model_paths`, recalculate `EssentiaModelStatus`, and compute missing IDs:

```rust
let mut missing_ids = essentia_model_specs()
    .into_iter()
    .filter(|spec| !essentia_model_is_installed(&models_path, *spec))
    .map(|spec| spec.id.to_string())
    .collect::<Vec<_>>();
missing_ids.sort();
missing_ids.dedup();
```

Sort/deduplicate installed IDs, map internal issues to the camelCase DTO, and use different messages for complete success, partial success, and zero valid models. Never include full paths. Register `import_essentia_models` in `tauri::generate_handler!`.

Change `download_essentia_embedding_model` to call `extract_official_musicnn_pair` and the same staged installer. Remove `extract_malformed_essentia_zip_entries` from `main.rs` after its coverage is present in this module. Downloaded and dragged official ZIP files must share one validation path.

- [x] **Step 7: Run importer/Tauri tests and formatting**

```bash
CARGO_TARGET_DIR=/private/tmp/w4dj-essentia-import cargo test --manifest-path src-tauri/Cargo.toml essentia_model_import -- --nocapture
CARGO_TARGET_DIR=/private/tmp/w4dj-essentia-import cargo test --manifest-path src-tauri/Cargo.toml -- --nocapture
cargo fmt --all -- --check
```

Expected: importer tests, all Tauri tests, and formatting pass.

- [x] **Step 8: Review checkpoint without commit**

```bash
git diff --check
git diff --stat -- src-tauri/src/essentia_model_import.rs src-tauri/src/main.rs
```

Expected: no whitespace errors. Do not stage or commit.

---

### Task 3: Frontend model page and import-button flow

**Files:**
- Modify: `app/src/app.ts:340-390`
- Modify: `app/src/app.ts:570-590`
- Modify: `app/src/app.ts:740-760`
- Modify: `app/src/app.ts:880-1030`
- Modify: `app/src/app.ts:2180-2220`
- Modify: `app/src/app.ts:3250-3280`
- Modify: `app/src/app.ts:3715-3740`
- Modify: `app/src/app.test.ts`

**Interfaces:**
- Consumes: Task 2 `import_essentia_models` and existing `open_external_url`.
- Produces:

```ts
export type EssentiaModelImportIssue = { fileName: string; reason: string };
export type EssentiaModelImportResult = {
  installedIds: string[];
  issues: EssentiaModelImportIssue[];
  missingIds: string[];
  status: EssentiaModelStatus;
  message: string;
};

// AppServices additions
pickEssentiaModelFiles?: () => Promise<string[]>;
importEssentiaModels?: (paths: string[]) => Promise<EssentiaModelImportResult>;
```

- [x] **Step 1: Write failing button, picker, and result tests**

Extend `makeMockServices` with deterministic picker/import defaults, then add tests that click `[data-action="open-essentia-models-page"]` and expect the exact URL `https://essentia.upf.edu/models/`; select two paths through `[data-action="import-essentia-models"]` and expect both to reach `importEssentiaModels`; cancel with `[]` and expect no command; reject import and assert both task slots remain `idle`.

Use this result fixture:

```ts
const importedModels: EssentiaModelImportResult = {
  installedIds: ['musicnn_embedding'],
  issues: [],
  missingIds: ['genre_rosamerica'],
  status: {
    version: 'essentia-musicnn-2022-v2',
    embedding: true, genre: false, mood: false, instrument: false, downloading: false,
  },
  message: '已导入 1 个模型，仍缺少 1 个模型。',
};
```

- [x] **Step 2: Run the frontend test file and verify failure**

```bash
pnpm --dir app test -- --run app/src/app.test.ts
```

Expected: failure because service methods and action elements do not exist.

- [x] **Step 3: Add translations, services, and picker normalization**

Add Chinese/English copy for official page, import button, picker title, importing, drop-ready, mixed drop, and import failure. Add:

```ts
export const ESSENTIA_MODELS_URL = 'https://essentia.upf.edu/models/';

pickEssentiaModelFiles: async () => {
  const lang = (localStorage.getItem('w4dj_lang') as AppLanguage) || 'zh';
  const selected = await open({
    directory: false,
    multiple: true,
    title: lang === 'zh' ? '导入 Essentia 预训练模型' : 'Import Essentia pretrained models',
    filters: [{ name: 'Essentia TensorFlow.js models', extensions: ['zip', 'json', 'bin'] }],
  });
  if (Array.isArray(selected)) return selected;
  return typeof selected === 'string' ? [selected] : [];
},
importEssentiaModels: (paths) =>
  invoke<EssentiaModelImportResult>('import_essentia_models', { paths }),
```

- [x] **Step 4: Implement one shared import action**

Both picker and Task 4 drag/drop must call:

```ts
const importEssentiaModelPaths = async (paths: string[]) => {
  if (!services.importEssentiaModels || paths.length === 0 || modelImporting) return;
  modelImporting = true;
  render();
  try {
    const result = await services.importEssentiaModels(paths);
    modelStatus = result.status;
    window.alert(result.message);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    window.alert(`${t('essentiaModelsImportFailed', state.lang)}\n\n${message}`);
  } finally {
    modelImporting = false;
    render();
  }
};
```

The official-page action calls `services.openExternalUrl(ESSENTIA_MODELS_URL)`. The picker action awaits `pickEssentiaModelFiles` and passes every selected path. Neither error path calls the generic conversion `reportError`.

- [x] **Step 5: Render all model actions**

Keep “下载分析模型” and add:

```html
<button type="button" class="secondary-action essentia-model-web" data-action="open-essentia-models-page">官网下载</button>
<button type="button" class="secondary-action essentia-model-import" data-action="import-essentia-models">导入模型</button>
```

Add `let modelImporting = false` beside `modelStatus`. Disable download/import while `modelStatus.downloading || modelImporting`; keep the web button enabled. Task 4 will map this boolean into the richer overlay state without changing the shared import result/error behavior.

- [x] **Step 6: Run focused frontend tests**

Run `pnpm --dir app test -- --run app/src/app.test.ts`.

Expected: all existing and new button/picker/import tests pass.

- [x] **Step 7: Review checkpoint without commit**

Run `git diff --check` and inspect `git diff -- app/src/app.ts app/src/app.test.ts`. Confirm import failures do not change either sync slot's `data-status`. Do not stage or commit.

---

### Task 4: Full-window model drag overlay and drop routing

**Files:**
- Modify: `app/src/app.ts:1038-1530`
- Modify: `app/src/app.ts:1530-1600`
- Modify: `app/src/app.ts:3490-3635`
- Modify: `app/src/styles.css:1250-1310`
- Modify: `app/src/styles.css` reduced-motion section
- Modify: `app/src/app.test.ts:1070-1225`

**Interfaces:**
- Consumes: Task 3 `importEssentiaModelPaths(paths)`.
- Produces:

```ts
export type ModelDropClassification = 'model' | 'mixed' | 'other';
export type ModelDropState = 'idle' | 'ready' | 'mixed' | 'importing';
export function classifyModelDropPaths(paths: string[]): ModelDropClassification;
```

- [x] **Step 1: Write failing classification and overlay tests**

Add exact pure expectations:

```ts
expect(classifyModelDropPaths(['/Downloads/msd-musicnn-1-tfjs.zip'])).toBe('model');
expect(classifyModelDropPaths(['/Downloads/model.json', '/Downloads/group1-shard1of1.bin'])).toBe('model');
expect(classifyModelDropPaths(['/Music/song.flac'])).toBe('other');
expect(classifyModelDropPaths(['/Music/NetEase Cloud Music'])).toBe('other');
expect(classifyModelDropPaths(['/Downloads/model.json', '/Music/song.mp3'])).toBe('mixed');
expect(classifyModelDropPaths([])).toBe('other');
```

Using the existing synthetic `File.path` drop-test pattern, assert: JSON/BIN drag activates `.app-shell[data-model-drop-state="ready"]`; dropping anywhere invokes the importer and not `selectSourceDirectory`; MP3/folder drops do not activate the model overlay and retain slot hit-testing; mixed input invokes neither flow and alerts; drag/native leave restores `idle`.

- [x] **Step 2: Run focused tests and verify failure**

Run `pnpm --dir app test -- --run app/src/app.test.ts`.

Expected: classification export, overlay, and routing assertions fail.

- [x] **Step 3: Implement deterministic path classification**

```ts
const MODEL_EXTENSIONS = new Set(['zip', 'json', 'bin']);

export function classifyModelDropPaths(paths: string[]): ModelDropClassification {
  if (paths.length === 0) return 'other';
  const modelCount = paths.filter((path) => {
    const name = path.replaceAll('\\', '/').split('/').pop() ?? '';
    const extension = name.includes('.') ? name.split('.').pop()?.toLowerCase() ?? '' : '';
    return MODEL_EXTENSIONS.has(extension);
  }).length;
  if (modelCount === paths.length) return 'model';
  if (modelCount > 0) return 'mixed';
  return 'other';
}
```

This controls routing only; Rust remains the validity authority.

- [x] **Step 4: Add persistent overlay state without per-over rendering**

Replace Task 3's `modelImporting` boolean with `modelDropState`, pass it as an optional final `renderApp` argument, and set `root.dataset.modelDropState`. Update `importEssentiaModelPaths` to reject state `importing`, set that state before invoking, and restore `idle` in `finally`. Render one inert overlay:

```html
<div class="model-drop-overlay" data-role="model-drop-overlay" aria-hidden="true">
  <div class="model-drop-card" role="status" aria-live="polite">
    <span class="model-drop-icon">${icon('download')}</span>
    <strong data-role="model-drop-title"></strong>
    <small data-role="model-drop-copy"></small>
  </div>
</div>
```

`syncModelDropUi()` updates the current shell dataset, localized text, and `aria-hidden` directly. Native `over` events must not call `render()` or rebuild the DOM; unrelated renders restore the current state.

- [x] **Step 5: Route browser and native drops before slot hit-testing**

Replace the first-file helper with `pathsFromBrowserDrop(event): string[]`, retaining URI-list fallback. Keep `activeNativeDropClassification` and the paths received by Tauri's `enter` event because its `over` payload contains only a pointer position. Reclassify the authoritative paths on `drop`. In browser listeners and `currentWindow.onDragDropEvent`:

1. classify all paths or visible file names on browser drag, native `enter`, and native `drop`; native `over` reuses the classification saved at `enter`;
2. `model`: clear slot targets, show `ready`, prevent default, import on drop anywhere;
3. `mixed`: clear targets, show `mixed`, prevent default, and alert without invoking either flow;
4. `other`: set overlay to `idle` and run the existing `dropTargetAt` / `handleDirectoryDrop` unchanged;
5. leave: clear overlay and slot classes.

Ignore repeated drops while `importing`. Never show full local paths.

- [x] **Step 6: Implement the full-window blur visual**

```css
.model-drop-overlay {
  position: fixed;
  inset: 0;
  z-index: 300;
  display: grid;
  place-items: center;
  padding: 24px;
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
  background: color-mix(in srgb, var(--rail) 38%, transparent);
  backdrop-filter: blur(22px) saturate(72%);
  -webkit-backdrop-filter: blur(22px) saturate(72%);
  transition: opacity 160ms ease, visibility 160ms ease;
}

.app-shell[data-model-drop-state="ready"] .model-drop-overlay,
.app-shell[data-model-drop-state="mixed"] .model-drop-overlay,
.app-shell[data-model-drop-state="importing"] .model-drop-overlay {
  opacity: 1;
  visibility: visible;
}
```

Use the existing rounded/glass language for the sharp center card, orange for `ready`, warning styling for `mixed`, and an indeterminate indicator for `importing`. Remove transition/animation in `prefers-reduced-motion`.

- [x] **Step 7: Run frontend tests and production build**

```bash
pnpm --dir app test -- --run app/src/app.test.ts
pnpm --dir app test -- --run
pnpm --dir app build
```

Expected: all frontend tests pass; build succeeds with only already-documented size warnings.

- [x] **Step 8: Review checkpoint without commit**

Inspect the three frontend diffs and run `git diff --check`. Confirm drag-over does not rebuild the DOM, song/folder hit-testing is unchanged, and model paths cannot reach `selectSourceDirectory`. Do not stage or commit.

---

### Task 5: Documentation and full acceptance

**Files:**
- Modify: `docs/project-state.md`
- Modify: `docs/handoff.md`
- Modify: `计划.md`
- Verify: all files changed by Tasks 1-4

**Interfaces:**
- Consumes: completed backend command, frontend actions, drop overlay, and recorded command output.
- Produces: evidence-backed project state and handoff; no Git commit or release artifact.

- [x] **Step 1: Run full automated validation**

```bash
CARGO_TARGET_DIR=/private/tmp/w4dj-essentia-import cargo test --manifest-path src-tauri/Cargo.toml
CARGO_TARGET_DIR=/private/tmp/w4dj-root-tests cargo test --all
pnpm --dir app test -- --run
pnpm --dir app build
cargo fmt --all -- --check
CARGO_TARGET_DIR=/private/tmp/w4dj-essentia-check cargo check --manifest-path src-tauri/Cargo.toml
CARGO_TARGET_DIR=/private/tmp/w4dj-essentia-clippy cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
git diff --check
```

Expected: every command passes. If the root-workspace strict all-targets Clippy variant is additionally run, preserve and report only the already-documented legacy `dead_code` baseline. New importer code must pass strict Tauri Clippy without allowances.

If a `pnpm` wrapper command fails only because this sandbox cannot create `app/_tmp_*`, run the equivalent installed tools without changing dependencies:

```bash
node app/node_modules/vitest/vitest.mjs --run
node app/node_modules/vite/bin/vite.js build --configLoader runner --outDir /private/tmp/w4dj-app-build --emptyOutDir
```

Record both the wrapper limitation and the equivalent command result.

- [x] **Step 2: Build the Apple Silicon application**

```bash
CARGO_TARGET_DIR=/private/tmp/w4dj-tauri-build cargo tauri build --target aarch64-apple-darwin --bundles app
/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' '/private/tmp/w4dj-tauri-build/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app/Contents/Info.plist'
```

Expected: build succeeds and prints `3.2.0-beta.3`.

- [ ] **Step 3: Perform application-level acceptance when GUI files are available**

当前环境无可用 GUI 驱动、真实官方模型文件或可解析的 `essentia.upf.edu` 网络；保留为人工验收项，不把合成夹具通过写成真实文件验收。

Use a MusiCNN ZIP downloaded from `https://essentia.upf.edu/models/feature-extractors/musicnn/` and one official classification directory containing `model.json` plus `group1-shard1of1.bin`:

1. Click “官网下载” and confirm the system browser opens the exact official directory.
2. Drag the ZIP over an empty area and confirm the whole window blurs with the model message.
3. Drop it and confirm `musicnn_embedding` is installed without changing either conversion task.
4. Use “导入模型” for the classification JSON/BIN pair and confirm its capability becomes available.
5. Drag MP3, FLAC, and a folder onto both source fields and confirm prior localized routing.
6. Drag `model.json` together with an MP3 and confirm neither model nor song is accepted.
7. Import an incomplete model and confirm previously installed models remain loadable.

If GUI access or official files are unavailable, record each unexecuted item, its reason, and these manual steps; continue all independent validation.

- [x] **Step 4: Update project documents with evidence only**

Record in all three documents: official-page/local-import implementation; full-window model blur and preserved song/folder routing; exact test/build counts; whether real official files were imported; and remaining network/Windows/Rekordbox/GUI limitations. Do not mark real import accepted when only synthetic fixtures ran.

- [x] **Step 5: Final Git inspection and handoff**

```bash
git status --short --branch
git diff --stat
git diff --check
```

Report all task outcomes, test/build evidence, environment-limited acceptance, current status, and diff stat. Wait for user confirmation. Do not stage, commit, push, merge, release, or publish.
