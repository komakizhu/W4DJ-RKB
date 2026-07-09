status: DONE
commits: [15d46b5]
tests: `cargo test parses_mode_and_lossless_output_format -v` — passed (1 test passed); verified `main.rs` wiring after the reviewer note
concerns: None remaining for Task 1.
changed_files:
  - `src/config.rs`
  - `src/main.rs`
  - `src/sync.rs`
  - `.superpowers/sdd/task-1-report.md`
summary: Added the shared `Mode` split (`compat`/`lossless`) plus `LosslessFormat` parsing in config, wired the new config shape through `main.rs`, and aligned the sync logic with the renamed mode names.
