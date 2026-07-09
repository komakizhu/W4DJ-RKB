status: DONE
commits: [b1dabed]
tests: `cargo test --test sync_policy -v` → passed (3 tests)
concerns: None remaining for Task 2.
changed_files:
  - /private/tmp/w4dj-wip/src/main.rs
  - /private/tmp/w4dj-wip/src/sync.rs
  - /private/tmp/w4dj-wip/tests/sync_policy.rs
summary: Added explicit `OutputPolicy`/`TargetProfile` resolution, wired the CLI path to pass `lossless_format`, and added integration tests covering compat MP3 and the three lossless targets.
