# W4DJ Tauri GUI Design

Date: 2026-07-09

## Summary

This design introduces a single-window desktop GUI for W4DJ using Tauri and a web-based UI. The GUI is macOS-first in feel, remains compatible with Windows, and keeps the interface intentionally minimal: one screen, fast start, and low cognitive load. The existing Rust sync engine remains the source of truth for all sync behavior.

## Goals

- Make W4DJ feel like a real desktop app rather than a shell.
- Keep the main flow fast: pick source, pick destination, choose mode, start.
- Preserve the current CLI workflow and shared Rust core.
- Remember the last used directories and mode automatically.
- Present status and logs without forcing a separate settings page.
- Support macOS as the primary visual target while avoiding a broken Windows experience.

## Non-Goals

- No multi-window workflow.
- No advanced audio tuning controls in the UI.
- No separate settings page in the first version.
- No rewrite of the sync engine logic inside the UI layer.
- No platform-specific UI branching that changes the product shape between macOS and Windows.

## Product Shape

The app uses one window with three visible regions:

1. **Primary action area**
   - Source directory selector
   - Destination directory selector
   - Mode selector: `兼容模式` / `无损模式`
   - Primary action button: `开始` / `暂停`

2. **Compact status strip**
   - Shows current state, progress, and current file
   - Stays visible by default
   - Expands into a log view when clicked

3. **Expandable detail area**
   - Shows logs and recent actions
   - Hidden by default to preserve the minimal feel
   - Slides or expands from the bottom without changing the window model

## Interaction Model

- On launch, the UI loads the last used source directory, destination directory, and mode.
- The user can immediately start a sync after verifying the directories.
- `开始` switches to `暂停` while a job is active.
- Pause is deferred until the current file completes.
- Logs stay collapsed unless the user opens the bottom detail area.
- Any error message appears in the status strip first, with details in the log area.

## Visual Direction

The visual system uses a restrained liquid-glass treatment:

- translucent surfaces with soft blur
- subtle borders instead of hard outlines
- light depth through shadow and highlight, not heavy contrast
- generous spacing and low visual noise
- rounded corners and calm neutral colors

### macOS Treatment

- Stronger glass effect and softer shadows
- More organic spacing and visual rhythm
- Buttons and panels should feel native to a modern macOS utility

### Windows Treatment

- Same layout and information hierarchy
- Glass effect toned down to avoid looking out of place
- Maintain readability and polish without relying on platform-specific flair

## Architecture

### Desktop Shell

Tauri provides the application window, native file picking, app lifecycle, and packaging.

### Web UI

The frontend owns:

- layout
- glass styling
- form interactions
- compact log expansion
- status rendering

The frontend does not implement sync rules or file-processing policy.

### Rust Core

The Rust backend remains the source of truth for:

- mode definitions
- sync policy
- task state
- pause semantics
- progress reporting
- logging events

### Persistence Layer

A small settings store persists:

- last source directory
- last destination directory
- last selected mode
- last selected lossless format

The store should be simple and local, suitable for cross-platform use.

## Data Flow

1. App launches and loads saved UI state.
2. User edits directories or mode in the UI.
3. UI sends the chosen values to the Rust backend.
4. Backend starts the sync task and emits progress updates.
5. UI updates the compact status strip and log detail area.
6. On exit, the latest selections are persisted.

## State Model

The UI should track a small set of explicit states:

- `Idle`
- `Running`
- `Pausing`
- `Paused`
- `Completed`
- `Error`

The state model must be simple enough for the status strip to summarize clearly.

## Error Handling

- Validation errors should appear before the sync starts.
- Runtime errors should not crash the shell UI.
- The status strip should always show the latest important state.
- Logs should preserve detailed error context for troubleshooting.

## Testing Strategy

- Verify that the GUI can load and save the last used directories.
- Verify that the status strip reflects task transitions correctly.
- Verify that start/pause behavior matches the shared Rust task controller.
- Verify that the single-window layout remains stable when the log area expands or collapses.
- Verify that the app still builds and runs on macOS and Windows targets.

## Implementation Principles

- Keep the sync engine shared between CLI and GUI.
- Avoid duplicate business logic in the frontend.
- Prefer small, composable UI components.
- Keep the first version focused on the fast-start workflow.
- Make the glass aesthetic expressive but not distracting.

## Outcome

This design delivers a compact, polished, single-window desktop GUI that feels native on macOS, remains acceptable on Windows, and preserves W4DJ’s low-friction syncing workflow.
