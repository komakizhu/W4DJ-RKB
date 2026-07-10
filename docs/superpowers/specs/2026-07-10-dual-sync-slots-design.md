# W4DJ Dual Sync Slots Design

Date: 2026-07-10

## Summary

W4DJ will expose two independent sync slots in the same desktop window. Each slot owns its source directory, configured destination directory, task state, progress, current file, and logs. Slot 2 uses slot 1's destination at execution time when its own destination is blank.

## Goals

- Keep both sync slots visible at the same time.
- Let each source run independently against its assigned destination.
- Fall back from slot 2's blank destination to slot 1's destination.
- Preserve all existing NCM decoding, audio conversion, metadata, filename cleanup, incremental sync, and FFmpeg discovery behavior.
- Preserve existing single-directory preferences by migrating them into slot 1.
- Keep mode and lossless output format global for this release.

## Non-Goals

- No per-slot compatibility/lossless mode.
- No merging or deduplication between source directories.
- No automatic disabling when both slots select the same source or destination.
- No additional slot beyond the fixed two-slot layout.

## User Interface

The control area contains two simultaneously visible sync cards:

- Slot 1: source directory 1, destination directory 1, status, progress, log access, and start/pause action.
- Slot 2: source directory 2, destination directory 2, status, progress, log access, and start/pause action.

The language control remains in the top-right corner. Mode and lossless format remain global controls. On narrow windows the cards stack vertically; on wider windows they can sit side by side.

When slot 2 has no configured destination, its destination control displays a localized fallback hint. The stored value remains blank. This distinguishes an explicit slot 2 destination from an inherited one.

## State Model

Rust and TypeScript expose the same conceptual shape:

```text
DesktopState
  mode
  lossless_format
  slots[2]
    source_directory
    destination_directory
    status
    progress_total
    progress_completed
    current_file
    logs
```

Each slot has its own task controller. Selecting directories and starting or pausing a task requires a zero-based slot index. Invalid indexes are rejected instead of silently selecting another slot.

## Destination Resolution

The backend is the source of truth for effective destinations:

```text
slot 1 effective destination = slot 1 configured destination
slot 2 effective destination = slot 2 configured destination when non-blank
                             = slot 1 configured destination otherwise
```

Whitespace-only values count as blank. Slot 2 may run while slot 1 has no source, as long as slot 2 has a source and either destination 2 or destination 1 is configured. A slot with no source fails only that slot. A slot with no effective destination fails only that slot.

## Execution

Each card starts and pauses independently. Starting one slot does not reset, pause, or overwrite the other slot's state. Both slots may run concurrently. Existing file-processing functions are reused unchanged after a slot-specific scan and comparison phase.

If both slots resolve to the same destination, W4DJ allows the configuration and makes the shared destination visible in the slot 2 fallback hint. No cross-slot duplicate suppression is introduced in this release.

## Persistence And Migration

New preference files store two slot directory records plus the global mode and lossless format. Loading an existing v1.0.0 preference file maps `source_directory` and `destination_directory` into slot 1 and initializes slot 2 as blank. Saving after migration writes only the new shape.

## Error Handling

- Directory validation and runtime processing errors update only the affected slot.
- A failure in one slot does not stop the other slot.
- Slot logs include the slot's configured source and effective destination.
- Fallback use is logged explicitly for troubleshooting.

## Testing

- Preference tests cover new-format round trips and v1.0.0 migration.
- Controller tests cover independent selection, task state, progress, and destination fallback.
- Tauri/core flow tests cover running slot 2 with destination 1 as fallback and running slot 2 without a configured slot 1 source.
- Frontend tests cover rendering both cards, selecting each directory independently, localized fallback text, per-slot start/pause, and state refresh without cross-slot overwrites.
- Existing Rust and frontend suites remain green.
- The Tauri crate is checked to catch command-signature and serialization mismatches.

## Acceptance Criteria

- Both source and both destination controls are visible together.
- Slot 1 syncs source 1 only to destination 1.
- Slot 2 syncs source 2 only to destination 2 when destination 2 is configured.
- Slot 2 syncs source 2 to destination 1 when destination 2 is blank.
- Either slot can start, pause, complete, or fail without replacing the other slot's state.
- Existing v1.0.0 preferences reopen with their directories in slot 1.
- Existing i18n, filename cleanup, incremental sync, metadata, and FFmpeg fixes continue to pass their tests.
