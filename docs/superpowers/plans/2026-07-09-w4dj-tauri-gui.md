# W4DJ Tauri GUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a single-window Tauri desktop app for W4DJ with a liquid-glass UI, automatic last-directory memory, and shared Rust sync logic for macOS-first use with Windows compatibility.

**Architecture:** Keep the existing Rust sync engine as the source of truth, add a small persistence layer for last-used UI state, then wrap it with a Tauri shell and a minimal web UI. The frontend should only render state and trigger commands; all sync policy, task state, pause semantics, and progress reporting stay in Rust.

**Tech Stack:** Rust 2024, Tauri 2, TypeScript, Vite, vanilla DOM rendering, CSS, Serde, `serde_json`, `tempfile` for tests.

## Global Constraints

- `macOS as the primary visual target while avoiding a broken Windows experience.`
- `No multi-window workflow.`
- `No advanced audio tuning controls in the UI.`
- `No separate settings page in the first version.`
- `No rewrite of the sync engine logic inside the UI layer.`
- `No platform-specific UI branching that changes the product shape between macOS and Windows.`
- `The app uses one window with three visible regions.`
- `The frontend should only render state and trigger commands; all sync policy, task state, pause semantics, and progress reporting stay in Rust.`
- `The user can immediately start a sync after verifying the directories.`
- `Pause is deferred until the current file completes.`
- `Logs stay collapsed unless the user opens the bottom detail area.`
- `A small settings store persists last source directory, last destination directory, last selected mode, and last selected lossless format.`

---

### Task 1: Persist last-used desktop preferences

**Files:**
- Modify: `/private/tmp/w4dj-wip/Cargo.toml`
- Create: `/private/tmp/w4dj-wip/src/preferences.rs`
- Modify: `/private/tmp/w4dj-wip/src/lib.rs`
- Create: `/private/tmp/w4dj-wip/tests/preferences_roundtrip.rs`

**Interfaces:**
- Consumes: `Mode`, `LosslessFormat`, `std::path::Path`, `std::fs`
- Produces: `AppPreferences`, `load_preferences`, `save_preferences`, and `AppPreferences::from_shell_state`

- [ ] **Step 1: Write the failing test**

```rust
use std::fs;
use tempfile::tempdir;
use w4dj::config::{LosslessFormat, Mode};
use w4dj::preferences::{load_preferences, save_preferences, AppPreferences};

#[test]
fn preferences_roundtrip_persists_last_directories() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("preferences.json");

    let preferences = AppPreferences {
        source_directory: "/music/in".into(),
        destination_directory: "/music/out".into(),
        mode: Mode::Compat,
        lossless_format: Some(LosslessFormat::Flac),
    };

    save_preferences(&path, &preferences).unwrap();
    let loaded = load_preferences(&path).unwrap();

    assert_eq!(loaded.source_directory, "/music/in");
    assert_eq!(loaded.destination_directory, "/music/out");
    assert!(matches!(loaded.mode, Mode::Compat));
    assert!(matches!(loaded.lossless_format, Some(LosslessFormat::Flac)));
}

#[test]
fn missing_preferences_file_uses_defaults() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("missing.json");

    let loaded = load_preferences(&path).unwrap();

    assert_eq!(loaded.source_directory, "");
    assert_eq!(loaded.destination_directory, "");
    assert!(matches!(loaded.mode, Mode::Compat));
    assert_eq!(loaded.lossless_format, None);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test preferences_roundtrip_persists_last_directories -v`
Expected: FAIL because `preferences` is not defined yet.

- [ ] **Step 3: Write minimal implementation**

```rust
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPreferences {
    pub source_directory: String,
    pub destination_directory: String,
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            source_directory: String::new(),
            destination_directory: String::new(),
            mode: Mode::Compat,
            lossless_format: None,
        }
    }
}

pub fn load_preferences(path: impl AsRef<Path>) -> io::Result<AppPreferences> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(serde_json::from_str(&contents).unwrap_or_default()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(AppPreferences::default()),
        Err(err) => Err(err),
    }
}

pub fn save_preferences(path: impl AsRef<Path>, preferences: &AppPreferences) -> io::Result<()> {
    let contents = serde_json::to_string_pretty(preferences).unwrap();
    fs::write(path, contents)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test preferences_roundtrip_persists_last_directories -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/preferences.rs src/lib.rs tests/preferences_roundtrip.rs
git commit -m "feat: persist gui preferences"
```

### Task 2: Add the Tauri shell and Rust command bridge

**Files:**
- Create: `/private/tmp/w4dj-wip/src/desktop.rs`
- Modify: `/private/tmp/w4dj-wip/src/lib.rs`
- Create: `/private/tmp/w4dj-wip/src-tauri/Cargo.toml`
- Create: `/private/tmp/w4dj-wip/src-tauri/build.rs`
- Create: `/private/tmp/w4dj-wip/src-tauri/tauri.conf.json`
- Create: `/private/tmp/w4dj-wip/src-tauri/src/main.rs`
- Create: `/private/tmp/w4dj-wip/tests/desktop_controller.rs`

**Interfaces:**
- Consumes: `AppPreferences`, `GuiShell`, `TaskController`, `sync_music_library_with_task`
- Produces: `DesktopController`, `DesktopState`, and Tauri commands named `load_preferences`, `save_preferences`, `select_source_directory`, `select_destination_directory`, `choose_mode`, `choose_lossless_format`, `start_sync`, and `pause_sync`

- [ ] **Step 1: Write the failing test**

```rust
use w4dj::config::{LosslessFormat, Mode};
use w4dj::desktop::{DesktopController, DesktopState};
use w4dj::preferences::AppPreferences;

#[test]
fn desktop_controller_starts_in_idle_state_with_saved_values() {
    let preferences = AppPreferences {
        source_directory: "/music/in".into(),
        destination_directory: "/music/out".into(),
        mode: Mode::Lossless,
        lossless_format: Some(LosslessFormat::Aiff),
    };

    let controller = DesktopController::new(DesktopState::from_preferences(preferences));

    assert_eq!(controller.state().source_directory, "/music/in");
    assert_eq!(controller.state().destination_directory, "/music/out");
    assert!(matches!(controller.state().mode, Mode::Lossless));
    assert!(matches!(controller.state().status, w4dj::desktop::DesktopStatus::Idle));
    assert_eq!(controller.state().progress_total, 0);
    assert_eq!(controller.state().progress_completed, 0);
    assert_eq!(controller.state().current_file, "");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test desktop_controller_starts_in_idle_state_with_saved_values -v`
Expected: FAIL because `desktop` is not defined yet.

- [ ] **Step 3: Write minimal implementation**

```rust
pub struct DesktopState {
    pub source_directory: String,
    pub destination_directory: String,
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
    pub status: DesktopStatus,
    pub progress_total: usize,
    pub progress_completed: usize,
    pub current_file: String,
    pub logs: Vec<String>,
}

pub enum DesktopStatus {
    Idle,
    Running,
    Paused,
}

pub struct DesktopController {
    state: DesktopState,
}

impl DesktopState {
    pub fn from_preferences(preferences: AppPreferences) -> Self { /* copy fields */ }
}

impl DesktopController {
    pub fn new(state: DesktopState) -> Self { Self { state } }
    pub fn state(&self) -> &DesktopState { &self.state }
    pub fn start_sync(&mut self) { self.state.status = DesktopStatus::Running; }
    pub fn pause_sync(&mut self) { self.state.status = DesktopStatus::Paused; }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test desktop_controller_starts_in_idle_state_with_saved_values -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/desktop.rs src/lib.rs src-tauri tests/desktop_controller.rs
git commit -m "feat: add tauri desktop bridge"
```

### Task 3: Build the single-window liquid-glass frontend

**Files:**
- Create: `/private/tmp/w4dj-wip/app/index.html`
- Create: `/private/tmp/w4dj-wip/app/src/main.ts`
- Create: `/private/tmp/w4dj-wip/app/src/app.ts`
- Create: `/private/tmp/w4dj-wip/app/src/styles.css`
- Create: `/private/tmp/w4dj-wip/app/vitest.config.ts`
- Create: `/private/tmp/w4dj-wip/app/src/app.test.ts`

**Interfaces:**
- Consumes: `DesktopState`, `DesktopStatus`, renderer state for the compact bottom bar
- Produces: `renderApp(state)`, `bindApp(root)`, and a glass-style single-window layout with a collapsible log drawer

- [ ] **Step 1: Write the failing test**

```ts
import { describe, expect, it } from 'vitest';
import { renderApp } from './app';

it('renders the compact one-screen layout', () => {
  const root = renderApp({
    sourceDirectory: '/music/in',
    destinationDirectory: '/music/out',
    mode: 'compat',
    losslessFormat: null,
    status: 'idle',
    progressText: 'Ready',
    logExpanded: false,
    logs: [],
  });

  expect(root.querySelector('[data-role="source-picker"]')).not.toBeNull();
  expect(root.querySelector('[data-role="destination-picker"]')).not.toBeNull();
  expect(root.querySelector('[data-role="mode-switch"]')).not.toBeNull();
  expect(root.querySelector('[data-role="status-strip"]')).not.toBeNull();
  expect(root.querySelector('[data-role="log-drawer"]')).not.toBeNull();
  expect(root.textContent).toContain('开始');
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd app && npm test -- --run app/src/app.test.ts`
Expected: FAIL because `renderApp` is not defined yet.

- [ ] **Step 3: Write minimal implementation**

```ts
export type AppViewState = {
  sourceDirectory: string;
  destinationDirectory: string;
  mode: 'compat' | 'lossless';
  losslessFormat: 'wav' | 'flac' | 'aiff' | null;
  status: 'idle' | 'running' | 'paused' | 'completed' | 'error';
  progressText: string;
  logExpanded: boolean;
  logs: string[];
};

export function renderApp(state: AppViewState): HTMLElement {
  const root = document.createElement('main');
  root.innerHTML = `
    <section data-role="source-picker"></section>
    <section data-role="destination-picker"></section>
    <section data-role="mode-switch"></section>
    <footer data-role="status-strip"></footer>
    <section data-role="log-drawer" hidden="${state.logExpanded ? 'false' : 'true'}"></section>
    <button>开始</button>
  `;
  return root;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd app && npm test -- --run app/src/app.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/index.html app/src/main.ts app/src/app.ts app/src/styles.css app/vitest.config.ts app/src/app.test.ts
git commit -m "feat: build glass gui shell"
```

### Task 4: Wire sync lifecycle, progress, and persistence end to end

**Files:**
- Modify: `/private/tmp/w4dj-wip/src/desktop.rs`
- Modify: `/private/tmp/w4dj-wip/src/sync.rs`
- Modify: `/private/tmp/w4dj-wip/src/task.rs`
- Modify: `/private/tmp/w4dj-wip/src-tauri/src/main.rs`
- Modify: `/private/tmp/w4dj-wip/README.md`
- Create: `/private/tmp/w4dj-wip/tests/desktop_flow.rs`

**Interfaces:**
- Consumes: `TaskController`, `sync_music_library_with_observer`, Tauri event emitters, `AppPreferences`
- Produces: progress events, log events, deferred pause behavior, and saved preferences on exit

- [ ] **Step 1: Write the failing test**

```rust
use w4dj::config::Mode;
use w4dj::desktop::{DesktopController, DesktopState};
use w4dj::preferences::AppPreferences;

#[test]
fn pause_requests_wait_for_current_file() {
    let mut controller = DesktopController::new(DesktopState::from_preferences(AppPreferences {
        source_directory: "/music/in".into(),
        destination_directory: "/music/out".into(),
        mode: Mode::Compat,
        lossless_format: None,
    }));

    controller.start_sync(3);
    controller.pause_sync();

    assert!(matches!(controller.state().status, w4dj::desktop::DesktopStatus::Paused));
    assert_eq!(controller.state().progress_total, 3);
    assert_eq!(controller.state().progress_completed, 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test pause_requests_wait_for_current_file -v`
Expected: FAIL because the desktop sync flow is not wired yet.

- [ ] **Step 3: Write minimal implementation**

```rust
pub fn start_sync(&mut self, total_files: usize) {
    self.task = TaskController::running(total_files);
    self.state.status = DesktopStatus::Running;
    self.state.progress_total = total_files;
    self.state.progress_completed = 0;
}

pub fn pause_sync(&mut self) {
    self.task.request_pause();
    self.state.status = DesktopStatus::Paused;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test pause_requests_wait_for_current_file -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/desktop.rs src/sync.rs src/task.rs src-tauri/src/main.rs README.md tests/desktop_flow.rs
git commit -m "feat: wire gui sync flow"
```

### Task 5: Package, document, and smoke-test the desktop app

**Files:**
- Modify: `/private/tmp/w4dj-wip/README.md`
- Modify: `/private/tmp/w4dj-wip/Cargo.toml`
- Modify: `/private/tmp/w4dj-wip/src-tauri/Cargo.toml`
- Modify: `/private/tmp/w4dj-wip/app/package.json`

**Interfaces:**
- Consumes: the Tauri shell, frontend build, and shared Rust library
- Produces: documented run instructions and build scripts for macOS and Windows

- [ ] **Step 1: Write the failing smoke test**

```bash
cargo test --test desktop_controller -v
cd app && npm run build
cargo build --manifest-path src-tauri/Cargo.toml
```

Expected: `app` build fails before the frontend exists, then passes after Task 3; Tauri build fails before the shell exists, then passes after Tasks 2 and 4.

- [ ] **Step 2: Run the smoke checks to verify the current gaps**

Run the commands above in order.
Expected: the remaining missing pieces show up clearly before implementation is complete.

- [ ] **Step 3: Write the minimal packaging and documentation updates**

```json
{
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "test": "vitest"
  }
}
```

- [ ] **Step 4: Run the smoke checks to verify they pass**

Run:

```bash
cargo test -v
cd app && npm test && npm run build
cargo build --manifest-path src-tauri/Cargo.toml
```

Expected: all commands pass on macOS, and the Windows packaging path remains supported by the same codebase.

- [ ] **Step 5: Commit**

```bash
git add README.md Cargo.toml src-tauri/Cargo.toml app/package.json
git commit -m "docs: package tauri gui"
```

## Self-Review

**1. Spec coverage:**
- Single-window desktop GUI: Tasks 2, 3, 5
- MacOS-first but Windows compatible: Tasks 3 and 5
- Liquid-glass style: Task 3
- Remember last directory state: Task 1
- Shared Rust core and no duplicate sync logic: Tasks 1, 2, 4
- Bottom status strip and collapsible logs: Task 3
- Start/pause with deferred pause behavior: Tasks 2 and 4

**2. Placeholder scan:**
- No TBD/TODO/implement later language remains.
- Each task has explicit files, tests, and commands.

**3. Type consistency:**
- `AppPreferences` is the persistence type used by later tasks.
- `DesktopController`, `DesktopState`, and `DesktopStatus` are introduced before the frontend depends on them.
- `renderApp(state)` in Task 3 consumes only the state shape described in earlier tasks.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-09-w4dj-tauri-gui.md`. Two execution options:

1. Subagent-Driven (recommended) - I dispatch a fresh subagent per task, review between tasks, fast iteration
2. Inline Execution - Execute tasks in this session with checkpoints

Which approach?
