# Unified Source Picker and Task Card Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (\`- [ ]\`) syntax for tracking.

**Goal:** Replace the separate source-folder/source-track buttons with one source picker that accepts either a file or folder, and remove the task-card log controls while retaining status text and the progress bar.

**Architecture:** The frontend exposes one \`pickSource\` service and routes its returned path through the existing \`selectSourceDirectory\` command, so file and folder inputs continue to use the same backend validation and conversion pipeline. On macOS, a small Tauri command opens \`NSOpenPanel\` with both files and directories enabled; the frontend keeps a safe file-picker fallback for other desktop targets. The task card renders only progress text and the progress track; backend log data remains available for errors/history but is not rendered in the card.

**Tech Stack:** TypeScript, Vitest, Vite, Tauri 2, Rust, macOS AppKit \`NSOpenPanel\`.

## Global Constraints

- The visible source UI has exactly one source-selection button labeled “选择来源” / “Choose source”.
- Dragging either a single supported audio file or a folder continues to save the dropped path directly.
- The task card keeps status/progress text and the horizontal progress bar.
- The task card does not render the log icon, detail toggle, current-track drawer, or log lines.
- Output-directory selection, preflight, conflict strategy, filename rules, conversion, GitHub, main, and release behavior stay unchanged.
- macOS is the exact native-picker target; non-macOS builds keep a usable single-button file-picker fallback.

---

### Task 1: Update frontend source selection and task-card rendering

**Files:**
- Modify: \`app/src/app.ts\`
- Modify: \`app/src/app.test.ts\`
- Modify: \`app/src/styles.css\`

**Interfaces:**
- Replace \`AppServices.pickSourceFile(slotIndex)\` with \`AppServices.pickSource(slotIndex): Promise<string | null>\`.
- Keep \`AppServices.pickDirectory(kind, slotIndex)\` for output-folder selection.
- Keep \`AppServices.selectSourceDirectory(slotIndex, path)\` as the single state-update path for both files and folders.

- [ ] **Step 1: Add failing frontend assertions for the new UI contract**

Update the render tests to assert that the progress fill and progress text remain, while these selectors are absent:

\`\`\`ts
expect(slotTwo.querySelector('.progress-fill')).not.toBeNull();
expect(slotTwo.querySelector('.progress-copy')?.textContent).toBe('45/100');
expect(slotTwo.querySelector('.status-toggle')).toBeNull();
expect(slotTwo.querySelector('[data-role="log-drawer"]')).toBeNull();
expect(slotTwo.querySelector('.detail-toggle-copy')).toBeNull();
\`\`\`

Replace the old log-expansion tests with a test that renders a slot containing \`currentFile\` and \`logs\`, then asserts neither value appears in the task card:

\`\`\`ts
const root = renderApp(makeViewStateWithSlot(0, {
  currentFile: '悟空传 - MC赵小六.wav',
  logs: ['Desktop shell ready'],
}));
const slot = root.querySelector('[data-role="sync-slot"][data-slot="0"]') as HTMLElement;

expect(slot.querySelector('[data-role="log-drawer"]')).toBeNull();
expect(slot.querySelector('.status-toggle')).toBeNull();
expect(slot.textContent).not.toContain('悟空传 - MC赵小六.wav');
expect(slot.textContent).not.toContain('Desktop shell ready');
\`\`\`

Replace the single-track button test with a unified source-picker test:

\`\`\`ts
const services = makeMockServices({
  pickSource: vi.fn().mockResolvedValue('/music/single-track.flac'),
  selectSourceDirectory: vi.fn().mockResolvedValue(
    makeDesktopStateWithSlot(1, { source_directory: '/music/single-track.flac' }),
  ),
});
const root = document.createElement('div');
bindApp(root, makeViewState(), services);

(root.querySelector('[data-action="pick-source"][data-slot="1"]') as HTMLButtonElement).click();

await vi.waitFor(() => {
  expect(services.pickSource).toHaveBeenCalledWith(1);
  expect(services.selectSourceDirectory).toHaveBeenCalledWith(1, '/music/single-track.flac');
  expect(root.textContent).toContain('/music/single-track.flac');
});
\`\`\`

Add a render assertion that the source card contains exactly one \`[data-action="pick-source"]\` and no \`[data-action="pick-source-file"]\`.

- [ ] **Step 2: Run the focused frontend tests and verify they fail**

Run:

\`\`\`bash
export PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH
./node_modules/.bin/vitest run src/app.test.ts
\`\`\`

Expected: FAIL because the current renderer still emits the separate track button and log drawer, and the mock service still exposes \`pickSourceFile\` rather than \`pickSource\`.

- [ ] **Step 3: Implement the minimal frontend change**

In \`app/src/app.ts\`:

1. Change the service type and default service from \`pickSourceFile\` to \`pickSource\`.
2. Make \`pickSource\` invoke Tauri command \`pick_source_path\`; if that command rejects on a non-macOS target, fall back to the existing file dialog with the supported-audio filter.
3. Change the source action handler to call \`services.pickSource(slotIndex)\) and remove the \`pick-source-file\` branch.
4. Change the source button text to \`t('pickSource')\` and remove the second source button.
5. Remove log/detail rendering and the \`toggle-log\` action. Keep backend log/current-file fields only where existing state/error code needs them.
6. Render the footer as status/progress text plus the existing \`.progress-track\` and \`.progress-fill\`.
7. Remove unused detail/log translation keys and task-log id generation.

The resulting footer must have this shape:

\`\`\`ts
<footer class="slot-status-strip">
  <span class="status-copy progress-copy">progressText</span>
  <div class="progress-track" aria-hidden="true">
    <div class="progress-fill">...</div>
  </div>
</footer>
\`\`\`

In \`app/src/styles.css\`, remove styles used only by \`.status-toggle\`, \`.detail-toggle-copy\`, and \`.log-drawer\`; keep the progress styles and ensure the footer has no reserved space for removed controls.

- [ ] **Step 4: Run the focused frontend tests and verify they pass**

Run:

\`\`\`bash
export PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH
./node_modules/.bin/vitest run src/app.test.ts
\`\`\`

Expected: the frontend test file passes with no references to the removed selectors or \`pickSourceFile\`.

- [ ] **Step 5: Commit the frontend change**

\`\`\`bash
git add app/src/app.ts app/src/app.test.ts app/src/styles.css
git commit -m "refactor: unify source picker and remove task logs"
\`\`\`

---

### Task 2: Add the macOS unified native source picker

**Files:**
- Modify: \`src-tauri/Cargo.toml\`
- Modify: \`src-tauri/src/main.rs\`
- Modify: \`src-tauri/Cargo.lock\`

**Interfaces:**
- Add Tauri command \`pick_source_path(title: String) -> Result<Option<String>, String>\`.
- The command returns \`Some(path)\) for one selected file or folder, \`None\) when cancelled, and \`Err(message)\` on native-dialog failure.
- The frontend invokes this command through \`AppServices.pickSource\`.

- [ ] **Step 1: Add macOS AppKit dependencies and the helper seam**

Add target-specific \`objc2-app-kit\` and \`objc2-foundation\` dependencies with only the features required by \`NSOpenPanel\`, \`NSSavePanel\`, \`NSPanel\`, \`NSResponder\`, \`NSWindow\`, \`NSArray\`, \`NSURL\`, and \`NSString\`. Keep them under the macOS target dependency table.

Extract this helper contract:

\`\`\`rust
#[cfg(target_os = "macos")]
fn selected_source_path_from_open_panel(title: &str) -> Result<Option<String>, String>
\`\`\`

The helper configures one \`NSOpenPanel\` with \`setCanChooseFiles(true)\`, \`setCanChooseDirectories(true)\`, and \`setAllowsMultipleSelection(false)\`, runs it modally, and converts the selected \`NSURL\` to a \`String\`. Cancel returns \`Ok(None)\`.

- [ ] **Step 2: Implement and register the command**

Add a platform branch with this contract:

\`\`\`rust
#[tauri::command]
fn pick_source_path(title: String) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        return selected_source_path_from_open_panel(&title);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = title;
        Err(String::from("unified source picker is only available on macOS"))
    }
}
\`\`\`

Register \`pick_source_path\` in \`tauri::generate_handler![]\`. The non-macOS error branch must remain so other builds compile and the frontend fallback remains available.

- [ ] **Step 3: Run Rust checks**

Run:

\`\`\`bash
cargo fmt --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
\`\`\`

Expected: formatting and all desktop tests pass.

- [ ] **Step 4: Commit the native picker change**

\`\`\`bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/main.rs
git commit -m "feat: add unified native source picker"
\`\`\`

---

### Task 3: Full verification and Debug app UI check

**Files:**
- Read/verify: \`app/src/app.ts\`, \`app/src/app.test.ts\`, \`app/src/styles.css\`, \`src-tauri/src/main.rs\`
- Build output: \`app/dist/\`, \`src-tauri/target/\` (ignored)

- [ ] **Step 1: Run complete frontend verification**

\`\`\`bash
export PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH
./node_modules/.bin/vitest run
./node_modules/.bin/vite build --base ./
\`\`\`

Expected: all frontend tests pass and Vite exits with code 0.

- [ ] **Step 2: Run complete Rust verification**

\`\`\`bash
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
\`\`\`

Expected: all commands exit with code 0 and no warnings are promoted to errors.

- [ ] **Step 3: Build and open the macOS Debug app**

Run from \`src-tauri/\`:

\`\`\`bash
export PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH
CI=true pnpm dlx @tauri-apps/cli@2.11.4 build --debug --bundles app
open -n "target/debug/bundle/macos/W4DJ RKB.app"
\`\`\`

Expected: the app opens with one source button per task, no log icon, no detail toggle, no log panel, and a visible progress bar below each task.

- [ ] **Step 4: Perform final repository check**

\`\`\`bash
git status --short --branch
git log -3 --oneline
\`\`\`

Expected: only intentional commits are present; no GitHub push, main merge, or release is performed.

