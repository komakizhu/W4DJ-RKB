# W4DJ Dual Sync Slots Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two independently controllable source/destination sync slots, with slot 2 falling back to slot 1's destination when its configured destination is blank.

**Architecture:** Represent directories as two fixed slot records in persisted preferences and represent runtime state as two independent `SyncSlotState` values with separate `TaskController` instances. Tauri commands take a zero-based slot index, while global mode and lossless format remain shared. The frontend mirrors the Rust state, renders both cards together, and polls while either slot is running.

**Tech Stack:** Rust 2024, Serde/serde_json, Tauri 2, TypeScript, vanilla DOM rendering, Vitest/jsdom, vanilla CSS.

## Global Constraints

- Both slots are visible simultaneously.
- Slot 1 source only feeds slot 1's effective destination.
- Slot 2 source only feeds slot 2's effective destination.
- Slot 2's blank or whitespace-only destination resolves to slot 1's configured destination at execution time.
- Slot 2's stored destination remains blank when fallback is active.
- Each slot owns status, progress, current file, logs, and pause state.
- Mode and lossless format remain global.
- Existing v1.0.0 preference files migrate their single source/destination into slot 1.
- Existing NCM, filename cleanup, metadata, incremental sync, and FFmpeg behavior must not regress.

---

### Task 1: Restore The Verified Post-v1.0.0 Baseline

**Files:**
- Modify: `app/src/app.ts`
- Modify: `app/src/app.test.ts`
- Modify: `app/src/styles.css`
- Modify: `src/sync.rs`
- Modify: `src/metadata.rs`
- Modify: `tests/sync_policy.rs`
- Modify: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: the recorded patches from the previous `/private/tmp/w4dj-wip` worktree.
- Produces: the previously verified i18n, naming cleanup, target-existence incremental policy, metadata artist normalization, and platform-aware FFmpeg lookup.

- [ ] **Step 1: Replay the recorded patches in their original order**

Apply only patches whose target starts with `/private/tmp/w4dj-wip/`, replacing that prefix with the current repository root. Do not replay exploratory changes made only in `/Users/mac/Documents/w4dj-0.6`.

- [ ] **Step 2: Verify the recovered baseline**

Run: `cargo test`

Expected: all recovered Rust tests pass.

Run: `npm test -- --run` from `app/`

Expected: all recovered Vitest tests pass and exit instead of watching.

- [ ] **Step 3: Inspect recovered scope**

Run: `git diff --check`

Expected: no whitespace errors.

Run: `git status --short`

Expected: only the seven recovered files above plus this plan are modified/untracked.

### Task 2: Persist Two Slots And Migrate v1.0.0 Preferences

**Files:**
- Modify: `src/preferences.rs`
- Modify: `tests/preferences_roundtrip.rs`
- Modify: `src/gui.rs`
- Modify: `tests/gui_launch.rs`

**Interfaces:**
- Produces: `pub const SYNC_SLOT_COUNT: usize = 2`
- Produces: `SyncSlotPreferences { source_directory: String, destination_directory: String }`
- Produces: `AppPreferences { slots: [SyncSlotPreferences; 2], mode: Mode, lossless_format: Option<LosslessFormat> }`
- Produces: `load_preferences` migration from the v1.0.0 flat JSON shape.

- [ ] **Step 1: Write failing round-trip and migration tests**

Add tests with literal expected data:

```rust
#[test]
fn preferences_roundtrip_persists_both_sync_slots() {
    let preferences = AppPreferences {
        slots: [
            SyncSlotPreferences::new("/music/in-1", "/music/out-1"),
            SyncSlotPreferences::new("/music/in-2", ""),
        ],
        mode: Mode::Compat,
        lossless_format: Some(LosslessFormat::Aiff),
    };
    // Save and reload, then assert all four directory values exactly.
}

#[test]
fn legacy_preferences_migrate_into_slot_one() {
    fs::write(
        &path,
        r#"{"source_directory":"/legacy/in","destination_directory":"/legacy/out","mode":"compat","lossless_format":null}"#,
    ).unwrap();
    let loaded = load_preferences(&path).unwrap();
    assert_eq!(loaded.slots[0].source_directory, "/legacy/in");
    assert_eq!(loaded.slots[0].destination_directory, "/legacy/out");
    assert_eq!(loaded.slots[1], SyncSlotPreferences::default());
}
```

- [ ] **Step 2: Run the preference tests and confirm RED**

Run: `cargo test --test preferences_roundtrip`

Expected: compilation fails because `SyncSlotPreferences` and `slots` do not exist.

- [ ] **Step 3: Implement the new persisted shape and legacy loader**

Use this public shape:

```rust
pub const SYNC_SLOT_COUNT: usize = 2;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSlotPreferences {
    pub source_directory: String,
    pub destination_directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPreferences {
    pub slots: [SyncSlotPreferences; SYNC_SLOT_COUNT],
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
}
```

`load_preferences` first parses `serde_json::Value`. If `slots` exists, deserialize `AppPreferences`; otherwise deserialize a private `LegacyAppPreferences` and map its flat directories to `slots[0]` while defaulting `slots[1]`.

- [ ] **Step 4: Update the legacy `GuiShell` adapter**

`AppPreferences::from_shell_state` maps the shell's single source/destination into slot 1 and leaves slot 2 blank. Update existing GUI tests to assert this behavior without changing the CLI shell itself.

- [ ] **Step 5: Run preference and GUI tests and confirm GREEN**

Run: `cargo test --test preferences_roundtrip --test gui_launch`

Expected: both test binaries pass.

### Task 3: Give Each Desktop Slot Independent Runtime State

**Files:**
- Modify: `src/desktop.rs`
- Modify: `tests/desktop_controller.rs`
- Modify: `tests/desktop_flow.rs`

**Interfaces:**
- Consumes: `AppPreferences::slots` and `SYNC_SLOT_COUNT`.
- Produces: `SyncSlotState` and `DesktopState::slots`.
- Produces: slot-indexed controller methods returning `Result<_, String>`.
- Produces: `effective_destination(slot_index: usize) -> Result<Option<String>, String>`.

- [ ] **Step 1: Write a failing slot isolation test**

```rust
#[test]
fn starting_slot_two_does_not_change_slot_one() {
    let mut controller = test_controller();
    controller.start_sync(1, 3).unwrap();
    controller.record_file_started(1, "track.wav").unwrap();
    controller.complete_current_file(1).unwrap();

    assert!(matches!(controller.state().slots[0].status, DesktopStatus::Idle));
    assert!(matches!(controller.state().slots[1].status, DesktopStatus::Running));
    assert_eq!(controller.state().slots[1].progress_completed, 1);
}
```

Add a second test asserting that slot 2 resolves a blank destination to slot 1's destination and that whitespace-only slot 2 destinations also fall back.

- [ ] **Step 2: Run controller tests and confirm RED**

Run: `cargo test --test desktop_controller --test desktop_flow`

Expected: compilation fails because runtime state and methods are still single-slot.

- [ ] **Step 3: Implement the slot runtime model**

Use this state boundary:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSlotState {
    pub source_directory: String,
    pub destination_directory: String,
    pub status: DesktopStatus,
    pub progress_total: usize,
    pub progress_completed: usize,
    pub current_file: String,
    pub logs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopState {
    pub slots: [SyncSlotState; SYNC_SLOT_COUNT],
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
}
```

Store `[TaskController; SYNC_SLOT_COUNT]` in `DesktopController`. Every directory/task mutation validates the index, mutates only the selected slot, and leaves global mode/format behavior unchanged.

- [ ] **Step 4: Implement destination fallback as a pure controller query**

```rust
pub fn effective_destination(&self, slot_index: usize) -> Result<Option<String>, String> {
    let slot = self.slot(slot_index)?;
    let configured = slot.destination_directory.trim();
    if !configured.is_empty() {
        return Ok(Some(configured.to_string()));
    }
    if slot_index == 1 {
        let fallback = self.state.slots[0].destination_directory.trim();
        return Ok((!fallback.is_empty()).then(|| fallback.to_string()));
    }
    Ok(None)
}
```

- [ ] **Step 5: Run controller tests and confirm GREEN**

Run: `cargo test --test desktop_controller --test desktop_flow`

Expected: all tests pass.

### Task 4: Route Tauri Commands And Sync Workers By Slot

**Files:**
- Modify: `src-tauri/src/main.rs`
- Modify: `tests/desktop_flow.rs`

**Interfaces:**
- Consumes: slot-indexed `DesktopController` methods.
- Produces Tauri commands: `select_source_directory(slot_index, path)`, `select_destination_directory(slot_index, path)`, `start_sync(slot_index)`, and `pause_sync(slot_index)`.
- Produces: `run_sync_task(controller, slot_index)` that updates only the selected slot.

- [ ] **Step 1: Add a core regression test for slot 2 fallback**

Construct preferences where slot 1 has only `/music/out-1`, slot 2 has `/music/in-2` and a blank destination, then assert `effective_destination(1)` is `/music/out-1` even though slot 1's source is blank.

- [ ] **Step 2: Run the regression test and confirm RED if the bridge is incomplete**

Run: `cargo test --test desktop_flow`

Expected before bridge completion: public API/signature failures identify remaining single-slot calls.

- [ ] **Step 3: Update Tauri commands**

All slot-specific commands return `Result<DesktopState, String>`. They validate `slot_index`, persist directory changes, and preserve the other slot. `choose_mode` and `choose_lossless_format` remain global and return `DesktopState`.

- [ ] **Step 4: Split the background worker by slot**

`start_sync(slot_index)` checks only that slot's running state, resets only that slot, and spawns `run_sync_task(controller, slot_index)`. The worker captures the selected source, `effective_destination(slot_index)`, global mode/format, and selected task controller. Every `record_file_started`, `record_file_completed`, `finish_sync`, and `fail_sync` call includes the same slot index.

- [ ] **Step 5: Check the Tauri crate**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

Expected: command generation, Serde state shape, and worker signatures compile.

### Task 5: Mirror Two Slots In TypeScript And Test User Actions

**Files:**
- Modify: `app/src/app.ts`
- Modify: `app/src/app.test.ts`

**Interfaces:**
- Consumes: Rust `DesktopState { slots, mode, lossless_format }`.
- Produces: `AppSyncSlotViewState`, tuple state for two slots, and slot-indexed `AppServices` methods.
- Produces: `renderSyncSlot(state, slotIndex)`.

- [ ] **Step 1: Write failing frontend tests**

Add tests that assert:

```ts
expect(root.querySelectorAll('[data-role="sync-slot"]')).toHaveLength(2);
expect(root.querySelector('[data-slot="0"]')?.textContent).toContain('/music/in-1');
expect(root.querySelector('[data-slot="1"]')?.textContent).toContain('/music/in-2');
```

Click slot 2's source picker and assert `selectSourceDirectory(1, '/new/source-2')`. Click slot 2's start button and assert `startSync(1)`. Render a blank slot 2 destination and assert the localized fallback hint references output directory 1.

- [ ] **Step 2: Run Vitest and confirm RED**

Run: `npm test -- --run` from `app/`.

Expected: tests fail because the frontend still exposes one source/destination and unindexed services.

- [ ] **Step 3: Implement the mirrored state and services**

Use zero-based tuple types:

```ts
export type SyncSlotIndex = 0 | 1;

export type DesktopSyncSlotState = {
  source_directory: string;
  destination_directory: string;
  status: AppStatus;
  progress_total: number;
  progress_completed: number;
  current_file: string;
  logs: string[];
};

export type DesktopState = {
  slots: [DesktopSyncSlotState, DesktopSyncSlotState];
  mode: AppMode;
  lossless_format: AppLosslessFormat | null;
};
```

`AppServices` passes `{ slotIndex, path }` to directory commands and `{ slotIndex }` to task commands. Poll while `state.slots.some(slot => slot.status === 'running')`. Preserve each slot's local `logExpanded` value when applying a backend refresh.

- [ ] **Step 4: Render both cards and bind indexed actions**

Each interactive element includes `data-slot="0"` or `data-slot="1"`. Event handling parses and validates the index before calling a service. Slot 2's blank destination shows the localized fallback copy while its actual `destinationDirectory` stays empty.

- [ ] **Step 5: Run Vitest and confirm GREEN**

Run: `npm test -- --run` from `app/`.

Expected: all old i18n tests and new dual-slot tests pass.

### Task 6: Fit Both Cards Into The Existing Desktop Layout

**Files:**
- Modify: `app/src/styles.css`
- Modify: `app/src/app.ts`

**Interfaces:**
- Consumes: two `[data-role="sync-slot"]` cards.
- Produces: a responsive two-column/stacked card layout without moving the language button from the top-right corner.

- [ ] **Step 1: Add explicit layout classes in the renderer**

Wrap both cards in `.sync-slots`, keep global mode/format controls in `.global-controls`, and keep each slot's progress/log controls inside `.sync-slot-card`.

- [ ] **Step 2: Add responsive CSS**

Use a two-column grid when space permits and a single column below the existing compact-window breakpoint. Path buttons retain `min-width: 0`, overflow ellipsis, and a visible fallback state. The compatibility/lossless explanatory note stays inside its bordered container.

- [ ] **Step 3: Build the frontend**

Run: `npm run build` from `app/`.

Expected: TypeScript and Vite finish with exit code 0.

### Task 7: Full Regression And Cross-Platform Static Verification

**Files:**
- Review: all files changed by Tasks 1-6.

**Interfaces:**
- Verifies: Rust core, frontend, Tauri bridge, formatting, and change scope.

- [ ] **Step 1: Format and run Rust tests**

Run: `cargo fmt --all -- --check`

Expected: exit code 0.

Run: `cargo test`

Expected: all Rust tests pass.

- [ ] **Step 2: Run frontend tests and build**

Run: `npm test -- --run` from `app/`.

Expected: all tests pass.

Run: `npm run build` from `app/`.

Expected: production build succeeds.

- [ ] **Step 3: Check Tauri and release-sensitive files**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

Expected: exit code 0 on the current host.

Review `.github/workflows/release.yml`, `src/sync.rs` FFmpeg selection tests, and `src-tauri/tauri.conf.json` sidecar entries together. No platform-specific path assumption may be introduced by the dual-slot work.

- [ ] **Step 4: Inspect final diff**

Run: `git diff --check`

Expected: no whitespace errors.

Run: `git status --short`

Expected: only approved recovery, dual-slot implementation, tests, and documentation are changed.
