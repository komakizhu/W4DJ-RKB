# W4DJ RKB Sync Preview, History, and Retry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a combined preflight confirmation flow, persistent conversion history, failed-file retry and error reports, while preventing repeated mode/format selection animations.

**Architecture:** Keep file classification and output estimates in the Rust library so the frontend never duplicates sync policy. Extend the Tauri desktop state with serializable preflight/history/failure data, and route normal starts and retries through one confirmed candidate-set workflow. Keep the existing two-slot controller and conversion engine intact, adding focused modules for history persistence and preview records.

**Tech Stack:** Rust 2024, serde/serde_json, existing W4DJ sync engine, Tauri 2 commands, TypeScript, Vite, Vitest, jsdom.

## Global Constraints

- The preflight is read-only and must run before conversion starts.
- “同时开始” opens one summary confirmation window containing both configured slots.
- A run with no processable files cannot be confirmed; a run with valid files and preflight errors may continue.
- History is stored in the app data directory as `history.json` and keeps the newest 50 task records.
- Retry processes only files recorded as failed and uses the same preflight/confirmation flow.
- Existing conversion policy, pause behavior, drag-and-drop, language switching, and packaging behavior must remain compatible.
- Repeated clicks on the selected mode/format do not invoke the backend or restart CSS animations.

---

## File Map

- Create `src/history.rs`: serializable task history records, bounded persistence, failed-file report formatting.
- Create `src/preview.rs`: owned preflight summary/candidate types and filesystem classification helpers.
- Modify `src/lib.rs`: expose the new library modules.
- Modify `src/desktop.rs`: add failure/preflight counters to slot state and controller methods for confirmed candidates and result recording.
- Modify `src-tauri/src/main.rs`: add app-data paths, history state, preflight/confirm/start/retry/report commands, and worker integration.
- Modify `app/src/app.ts`: add service contracts, summary modal, history view, retry/report actions, and selection request guards.
- Modify `app/src/styles.css`: modal/history styles and locked selector state; preserve the existing visual language.
- Modify `tests/desktop_flow.rs` and add `tests/preview.rs`, `tests/history.rs`: Rust seam tests.
- Modify `app/src/app.test.ts`: frontend behavior tests for modal/history and animation guards.

## Task 1: Add core preflight classification and owned candidates

**Files:**

- Create: `src/preview.rs`
- Modify: `src/lib.rs`
- Modify: `src/sync.rs` only where visibility is needed for shared output-path/policy helpers
- Test: `tests/preview.rs`

**Interfaces:**

- Consumes: `Mode`, `LosslessFormat`, `get_music_dict`, `get_destination_music_dict`, `compare_music_dicts`, `resolve_output_policy`.
- Produces: `SyncPreview`, `PreviewCandidate`, `PreviewIssue`, `build_sync_preview(source, destination, mode, lossless_format)`.

- [ ] **Step 1: Write failing tests for classification.**

```rust
#[test]
fn preview_separates_new_existing_and_estimated_bytes() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    write_file(source.path().join("new.mp3"), 120);
    write_file(destination.path().join("existing.mp3"), 80);

    let preview = build_sync_preview(
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.new_count, 1);
    assert_eq!(preview.existing_count, 1);
    assert_eq!(preview.skipped_count, 0);
    assert_eq!(preview.error_count, 0);
    assert_eq!(preview.candidates[0].source_size_bytes, 120);
    assert_eq!(preview.estimated_output_bytes, 120);
}

#[test]
fn preview_reports_missing_source_and_invalid_destination() {
    let preview = build_sync_preview(
        "/path/that/does/not/exist",
        "/path/that/cannot/be/used",
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.new_count, 0);
    assert!(preview.error_count >= 1);
    assert!(!preview.errors[0].message.is_empty());
}
```

- [ ] **Step 2: Run the focused test and verify it fails.**

Run: `cargo test --test preview -- --nocapture`

Expected: FAIL because `preview` types and `build_sync_preview` do not exist.

- [ ] **Step 3: Implement the owned preview model and classifier.**

Add these public serde types in `src/preview.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewCandidate {
    pub name: String,
    pub source_path: String,
    pub destination_path: String,
    pub source_size_bytes: u64,
    pub estimated_output_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncPreview {
    pub source_directory: String,
    pub destination_directory: String,
    pub new_count: usize,
    pub existing_count: usize,
    pub skipped_count: usize,
    pub error_count: usize,
    pub estimated_output_bytes: Option<u64>,
    pub candidates: Vec<PreviewCandidate>,
    pub skipped: Vec<PreviewIssue>,
    pub errors: Vec<PreviewIssue>,
}

pub fn build_sync_preview(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
) -> io::Result<SyncPreview>;
```

The implementation must reject a missing source directory, create no output files, classify every source entry selected by the existing sync dictionaries, use `compare_music_dicts` for new candidates, and calculate the estimate by summing source byte sizes. Use `None` for the total only when a candidate size cannot be read. Make the existing output-path helper `pub(crate)` so preview and processing produce the same target path.

- [ ] **Step 4: Run the focused test and verify it passes.**

Run: `cargo test --test preview -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Run the existing sync policy tests.**

Run: `cargo test --test sync_policy`

Expected: PASS with no changes to output policy behavior.

- [ ] **Step 6: Commit the vertical slice.**

```bash
git add src/preview.rs src/lib.rs src/sync.rs tests/preview.rs
git commit -m "feat: add conversion preflight classification"
```

## Task 2: Add history persistence and failure records

**Files:**

- Create: `src/history.rs`
- Modify: `src/lib.rs`
- Modify: `src/desktop.rs`
- Test: `tests/history.rs`
- Test: `tests/desktop_flow.rs`

**Interfaces:**

- Consumes: `SyncPreview` and worker result events.
- Produces: `HistoryEntry`, `FailedFile`, `HistoryStatus`, `load_history`, `append_history`, `write_error_report`.

- [ ] **Step 1: Write failing persistence and retry-data tests.**

```rust
#[test]
fn history_keeps_newest_fifty_entries() {
    let path = tempdir().unwrap().path().join("history.json");
    let mut entries = (0..51).map(test_entry).collect::<Vec<_>>();

    append_history(&path, entries.remove(0)).unwrap();
    for entry in entries {
        append_history(&path, entry).unwrap();
    }

    let loaded = load_history(&path).unwrap();
    assert_eq!(loaded.len(), 50);
    assert_eq!(loaded[0].batch_id, "batch-1");
    assert_eq!(loaded[49].batch_id, "batch-50");
}

#[test]
fn error_report_contains_failed_path_and_reason() {
    let entry = test_entry_with_failure("/music/in/song.flac", "FFmpeg failed");
    let report = format_error_report(&entry);

    assert!(report.contains("/music/in/song.flac"));
    assert!(report.contains("FFmpeg failed"));
}
```

- [ ] **Step 2: Run the focused test and verify it fails.**

Run: `cargo test --test history -- --nocapture`

Expected: FAIL because the history module and functions do not exist.

- [ ] **Step 3: Implement bounded JSON history.**

Use these serializable types:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HistoryStatus { Completed, Partial, Cancelled, Error }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailedFile {
    pub name: String,
    pub source_path: String,
    pub destination_path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: String,
    pub batch_id: String,
    pub slot_index: usize,
    pub started_at: String,
    pub finished_at: String,
    pub duration_seconds: u64,
    pub source_directory: String,
    pub destination_directory: String,
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
    pub new_count: usize,
    pub existing_count: usize,
    pub skipped_count: usize,
    pub error_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
    pub failed_files: Vec<FailedFile>,
    pub status: HistoryStatus,
    pub retry_of: Option<String>,
}
```

`load_history` returns an empty vector for a missing file, returns `InvalidData` for malformed JSON so the caller can log and recover, and `append_history` writes atomically through a temporary file before renaming. Sort history newest first and truncate to 50 entries. `format_error_report` returns UTF-8 text with task metadata followed by one failure per line.

Extend `SyncSlotState` with preview counts and `failed_files`, and add controller methods that record a failed file without counting it as a successful new track. Preserve the existing serialized fields and default values so old state tests remain valid.

- [ ] **Step 4: Run history and desktop flow tests.**

Run: `cargo test --test history --test desktop_flow`

Expected: PASS, including the existing pause and slot-isolation assertions.

- [ ] **Step 5: Commit the vertical slice.**

```bash
git add src/history.rs src/lib.rs src/desktop.rs tests/history.rs tests/desktop_flow.rs
git commit -m "feat: persist conversion history and failed files"
```

## Task 3: Wire Tauri preflight, confirmed starts, history, retry, and reports

**Files:**

- Modify: `src-tauri/src/main.rs`
- Modify: `src/desktop.rs`
- Modify: `tests/desktop_flow.rs`
- Add tests inside `src-tauri/src/main.rs` for command-independent worker helpers

**Interfaces:**

- Consumes: `build_sync_preview`, `SyncPreview`, `HistoryEntry`, `FailedFile`.
- Produces Tauri commands: `preview_all_sync`, `start_confirmed_sync`, `load_history`, `retry_history_failures`, `export_history_error_report`.

- [ ] **Step 1: Add controller tests for confirmed candidate totals and failure capture.**

```rust
#[test]
fn confirmed_start_uses_preview_candidate_count() {
    let mut controller = test_controller();
    controller.start_confirmed_sync(0, 3).unwrap();

    assert_eq!(controller.state().slots[0].progress_total, 3);
    assert!(matches!(controller.state().slots[0].status, DesktopStatus::Running));
}

#[test]
fn failed_result_is_available_for_retry_without_increasing_completed_count() {
    let mut controller = test_controller();
    controller.start_confirmed_sync(0, 1).unwrap();
    controller.record_failed_file(0, FailedFile {
        name: "song".into(),
        source_path: "/in/song.flac".into(),
        destination_path: "/out/song.mp3".into(),
        message: "conversion failed".into(),
    }).unwrap();

    assert_eq!(controller.state().slots[0].progress_completed, 0);
    assert_eq!(controller.state().slots[0].failed_files.len(), 1);
}
```

- [ ] **Step 2: Run the focused Rust tests and verify they fail.**

Run: `cargo test --test desktop_flow confirmed_start_uses_preview_candidate_count -- --nocapture`

Expected: FAIL because confirmed-start and failed-file APIs do not exist.

- [ ] **Step 3: Add app paths and history loading to Tauri state.**

Extend `AppState` with `history_path: Arc<Mutex<PathBuf>>` and initialize it beside `preferences.json`:

```rust
let app_dir = app.path().app_config_dir().expect("failed to resolve app config directory");
let preferences_path = app_dir.join("preferences.json");
let history_path = app_dir.join("history.json");
```

Load history at setup, keep malformed history as an empty in-memory list, and register the new commands in `generate_handler!`.

- [ ] **Step 4: Replace direct global start with preflight and confirmed candidate execution.**

Implement `preview_all_sync` as a read-only command returning an owned `Vec<SlotPreview>` for startable slots. Implement `start_confirmed_sync(previews: Vec<SlotPreview>)` so it validates source/destination/mode/format against the current controller, starts only the candidates returned by the preflight, and creates one `batch_id` for all slots. The command must reject an empty candidate set and must not start a worker before confirmation.

Use a worker input struct with owned values:

```rust
struct ConfirmedSyncJob {
    batch_id: String,
    slot_index: usize,
    source: String,
    destination: String,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    candidates: Vec<PreviewCandidate>,
    preview: SyncPreview,
    retry_of: Option<String>,
}
```

The worker resolves candidates back to the existing sync map by name, runs `sync_music_library_with_observer`, records `FailedFile` entries, updates controller state, and appends one `HistoryEntry` at every terminal state. It must not reclassify successful candidates or include temporary files in failure output.

- [ ] **Step 5: Implement history commands and retry/report flow.**

`load_history` returns newest-first history. `retry_history_failures(id)` loads the entry, verifies each failed source path, constructs a preview containing only those files, and returns the same `SlotPreview` shape used by the confirmation modal. `start_confirmed_sync` accepts `retry_of` and writes the new linked history record. `export_history_error_report(id, path)` writes `format_error_report` to the user-selected path.

- [ ] **Step 6: Run the full Rust suite.**

Run: `cargo test`

Expected: PASS, including existing sync, task, preference, GUI-shell, controller, and metadata tests.

- [ ] **Step 7: Commit the Tauri integration slice.**

```bash
git add src-tauri/src/main.rs src/desktop.rs tests/desktop_flow.rs
git commit -m "feat: add confirmed sync history and failed retry commands"
```

## Task 4: Add frontend service contracts and summary confirmation modal

**Files:**

- Modify: `app/src/app.ts`
- Modify: `app/src/app.test.ts`
- Modify: `app/src/styles.css`

**Interfaces:**

- Consumes Tauri commands from Task 3.
- Produces `AppPreview`, `AppHistoryEntry`, modal state, and the user-visible confirm/cancel behavior.

- [ ] **Step 1: Add failing frontend tests for summary confirmation.**

```ts
it('shows one combined preview modal before starting both slots', async () => {
  const services = makeMockServices({
    previewAllSync: vi.fn().mockResolvedValue(makePreviewResponse()),
    startConfirmedSync: vi.fn().mockResolvedValue(makeDesktopState({
      slots: [makeDesktopSlot({ status: 'running' }), makeDesktopSlot({ status: 'running' })],
    })),
  });
  const root = document.createElement('div');
  bindApp(root, makeViewState(), services);

  (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();

  await vi.waitFor(() => {
    expect(root.querySelector('[data-role="preview-modal"]')).not.toBeNull();
    expect(root.querySelector('[data-role="preview-modal"]')?.textContent).toContain('新增');
    expect(root.querySelector('[data-role="preview-modal"]')?.textContent).toContain('预计输出');
  });
  expect(services.startConfirmedSync).not.toHaveBeenCalled();
});

it('starts only after confirming the combined preview', async () => {
  const services = makeMockServices({ previewAllSync: vi.fn().mockResolvedValue(makePreviewResponse()) });
  const root = document.createElement('div');
  bindApp(root, makeViewState(), services);

  (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
  await vi.waitFor(() => expect(root.querySelector('[data-role="preview-modal"]')).not.toBeNull());
  (root.querySelector('[data-action="confirm-start"]') as HTMLButtonElement).click();

  await vi.waitFor(() => expect(services.startConfirmedSync).toHaveBeenCalledTimes(1));
});
```

- [ ] **Step 2: Run the focused frontend tests and verify they fail.**

Run: `npm test -- --run app/src/app.test.ts -t "preview modal"`

Expected: FAIL because preview service types, modal rendering, and confirm action do not exist.

- [ ] **Step 3: Add TypeScript API types and Tauri service methods.**

Add `AppPreview`, `AppPreviewCandidate`, `AppPreviewIssue`, and `AppHistoryEntry` matching the Rust serde shape. Extend `AppServices` with:

```ts
previewAllSync: () => Promise<AppPreviewResponse>;
startConfirmedSync: (previews: AppPreview[], retryOf?: string | null) => Promise<DesktopState>;
loadHistory: () => Promise<AppHistoryEntry[]>;
retryHistoryFailures: (id: string) => Promise<AppPreview>;
exportHistoryErrorReport: (id: string, path: string) => Promise<void>;
```

Map them to `invoke('preview_all_sync')`, `invoke('start_confirmed_sync', { previews, retryOf })`, `invoke('load_history')`, `invoke('retry_history_failures', { id })`, and `invoke('export_history_error_report', { id, path })`.

- [ ] **Step 4: Render the single combined modal.**

Store `previewModal: { previews: AppPreview[]; retryOf: string | null } | null` and a pending preview action in `bindApp`. On `start-all`, call `previewAllSync`, set the modal, and do not call `startConfirmedSync`. Render `[data-role="preview-modal"]` with one card per slot, all five counters, estimated size, paths, and the three actions:

```html
<button data-action="cancel-preview">取消</button>
<button data-action="edit-preview">返回修改</button>
<button data-action="confirm-start">确认并开始转换</button>
```

Disable confirmation when every preview has zero candidates or a blocking destination error. Confirmation closes only after `startConfirmedSync` resolves successfully.

- [ ] **Step 5: Add modal CSS and run focused tests.**

Use the existing card, rail, and accent variables; add only `.preview-modal`, `.preview-dialog`, `.preview-stat`, `.history-*` primitives needed by later tasks. Run:

```bash
npm test -- --run app/src/app.test.ts -t "preview"
```

Expected: PASS.

- [ ] **Step 6: Commit the frontend preview slice.**

```bash
git add app/src/app.ts app/src/app.test.ts app/src/styles.css
git commit -m "feat: add combined preflight confirmation dialog"
```

## Task 5: Add history UI, retry, and error-report export

**Files:**

- Modify: `app/src/app.ts`
- Modify: `app/src/app.test.ts`
- Modify: `app/src/styles.css`

- [ ] **Step 1: Write failing history interaction tests.**

```ts
it('renders history entries and retries only the failed entry', async () => {
  const services = makeMockServices({
    loadHistory: vi.fn().mockResolvedValue([makeHistoryEntry({ failed_count: 2 })]),
    retryHistoryFailures: vi.fn().mockResolvedValue(makeRetryPreview()),
  });
  const root = document.createElement('div');
  bindApp(root, makeViewState(), services);

  await vi.waitFor(() => expect(root.querySelector('[data-role="history"]')).not.toBeNull());
  (root.querySelector('[data-action="retry-history"]') as HTMLButtonElement).click();

  await vi.waitFor(() => {
    expect(services.retryHistoryFailures).toHaveBeenCalledWith('history-1');
    expect(root.querySelector('[data-role="preview-modal"]')).not.toBeNull();
  });
});
```

- [ ] **Step 2: Run the focused test and verify it fails.**

Run: `npm test -- --run app/src/app.test.ts -t "history entries"`

Expected: FAIL because history state, rendering, and retry actions do not exist.

- [ ] **Step 3: Load and render history.**

Load history alongside desktop state during `bindApp`, render a collapsible `[data-role="history"]` section below the workbench, show newest entries first, and include time, status, completed/failed counts, output directory, and expandable failure details. A history item with failures shows:

```html
<button data-action="retry-history" data-history-id="history-1">重试失败项目</button>
<button data-action="export-history" data-history-id="history-1">导出错误报告</button>
```

Use the selected language for all labels and preserve the existing light/dark theme.

- [ ] **Step 4: Connect retry to the same modal.**

On retry, call `retryHistoryFailures(id)`, open the existing preview modal with `retryOf: id`, and confirm through `startConfirmedSync(previews, id)`. Do not add a second confirmation implementation.

- [ ] **Step 5: Connect error-report export to the directory/file picker.**

Use the existing dialog plugin to select a save path, then call `exportHistoryErrorReport`. Show success/failure in the app log without mutating the history list.

- [ ] **Step 6: Run frontend history tests and the complete frontend suite.**

Run:

```bash
npm test -- --run app/src/app.test.ts -t "history"
npm test -- --run
```

Expected: PASS.

- [ ] **Step 7: Commit the history UI slice.**

```bash
git add app/src/app.ts app/src/app.test.ts app/src/styles.css
git commit -m "feat: add conversion history and failed retry UI"
```

## Task 6: Fix selector animation re-triggering and rapid format clicks

**Files:**

- Modify: `app/src/app.ts`
- Modify: `app/src/styles.css`
- Modify: `app/src/app.test.ts`

- [ ] **Step 1: Add failing animation guard tests.**

```ts
it('does not invoke the backend or animation for the already selected mode', async () => {
  const services = makeMockServices();
  const root = document.createElement('div');
  bindApp(root, makeViewState({ mode: 'compat' }), services);

  (root.querySelector('[data-mode="compat"]') as HTMLButtonElement).click();
  await Promise.resolve();

  expect(services.chooseMode).not.toHaveBeenCalled();
  expect(root.querySelector('.app-shell')?.dataset.selectionMotion).not.toBe('mode');
});

it('serializes rapid WAV and AIFF selection clicks', async () => {
  const first = createDeferred<DesktopState>();
  const services = makeMockServices({
    chooseLosslessFormat: vi.fn().mockReturnValue(first.promise),
  });
  const root = document.createElement('div');
  bindApp(root, makeViewState({ mode: 'lossless', losslessFormat: 'wav' }), services);

  (root.querySelector('[data-format="aiff"]') as HTMLButtonElement).click();
  const button = root.querySelector('[data-format="wav"]') as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  button.click();
  expect(services.chooseLosslessFormat).toHaveBeenCalledTimes(1);
  first.resolve(makeDesktopState({ mode: 'lossless', lossless_format: 'aiff' }));
});
```

- [ ] **Step 2: Run the focused tests and verify they fail.**

Run: `npm test -- --run app/src/app.test.ts -t "selected mode|rapid WAV"`

Expected: FAIL because current handlers invoke the backend for repeated selections and do not lock the selector.

- [ ] **Step 3: Add value-change guards and pending locks.**

Before calling `chooseMode` or `chooseLosslessFormat`, compare the requested value to the current state. Return without `runAction` when equal. Track `pendingSelection: 'mode' | 'format' | null`; render the related buttons with `disabled` while pending, clear it in both success and error paths, and only set `selectionMotion` when the value changes.

- [ ] **Step 4: Make CSS animation classes one-shot.**

Keep the selected-position transforms as the stable state. Apply the keyframe only when `data-selection-motion` is set for a changed selection, and clear the attribute after the render tick/animation completion so an unrelated state update cannot restart the slide. Add a disabled cursor/opacity rule for `.mode-button:disabled` and `.format-button:disabled`.

- [ ] **Step 5: Run the complete frontend suite and commit.**

Run: `npm test -- --run`

Expected: PASS.

```bash
git add app/src/app.ts app/src/styles.css app/src/app.test.ts
git commit -m "fix: stabilize mode and lossless format selectors"
```

## Task 7: Full verification and release notes

**Files:**

- Modify: `README.md` only if the new workflow needs user-facing instructions
- No changes to existing untracked `dist/` or `src-tauri/.DS_Store`

- [ ] **Step 1: Run Rust formatting and tests.**

Run:

```bash
cargo fmt --all -- --check
cargo test
```

Expected: both commands pass.

- [ ] **Step 2: Run frontend type/build/test verification.**

Run:

```bash
cd app
npm test -- --run
npm run build
```

Expected: all Vitest tests pass and Vite produces the application bundle.

- [ ] **Step 3: Inspect the final diff for scope and regressions.**

Run:

```bash
git diff HEAD~6 --stat
git diff --check
git status --short
```

Confirm only the requested feature files and the design/plan documents changed; leave pre-existing untracked files untouched.

- [ ] **Step 4: Commit any final documentation update.**

```bash
git add README.md
git commit -m "docs: explain preflight history and retry workflow"
```

Skip this commit when no README change is needed.
