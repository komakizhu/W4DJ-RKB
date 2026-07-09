status: DONE
commits: [4ed8976]
tests: `cargo test --test gui_launch -v` -> passed (2 tests). Cargo emitted an environment warning about a readonly global last-use cache, but tests completed successfully.
concerns: This is still a framework-free shell, not a rendered desktop window, but it now exposes launcher, directory/mode/format actions, start/pause hooks, logs, and a `--gui` entrypoint path.
changed_files:
  - `/private/tmp/w4dj-wip/src/config.rs`
  - `/private/tmp/w4dj-wip/src/gui.rs`
  - `/private/tmp/w4dj-wip/src/lib.rs`
  - `/private/tmp/w4dj-wip/src/main.rs`
  - `/private/tmp/w4dj-wip/tests/gui_launch.rs`
summary: Expanded the GUI shell scaffold into a real launchable shell model with source/destination selection, mode and format updates, start/pause actions, logs, and a CLI `--gui` launch path.
