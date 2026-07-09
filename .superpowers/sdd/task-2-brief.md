### Task 2: Refactor sync engine around explicit policy objects

**Files:**
- Modify: `/private/tmp/w4dj-review/src/sync.rs`
- Modify: `/private/tmp/w4dj-review/src/metadata.rs`
- Test: `/private/tmp/w4dj-review/tests/sync_policy.rs`

**Interfaces:**
- Consumes: `Mode`, `LosslessFormat`, source file metadata, destination state.
- Produces: a processed file plan with target path, output format, and transcode policy.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn compat_mode_always_targets_mp3() {
    let policy = resolve_output_policy(Mode::Compat, None, "flac");
    assert_eq!(policy.output_extension, "mp3");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test compat_mode_always_targets_mp3 -v`
Expected: FAIL because `resolve_output_policy` is not defined yet.

- [ ] **Step 3: Write minimal implementation**

```rust
pub struct OutputPolicy {
    pub output_extension: &'static str,
    pub target_profile: TargetProfile,
}

pub enum TargetProfile {
    CompatMp3,
    LosslessWav,
    LosslessFlac,
    LosslessAiff,
}

pub fn resolve_output_policy(mode: Mode, lossless_format: Option<LosslessFormat>, source_extension: &str) -> OutputPolicy {
    match mode {
        Mode::Compat => OutputPolicy { output_extension: "mp3", target_profile: TargetProfile::CompatMp3 },
        Mode::Lossless => {
            let profile = match lossless_format.unwrap_or(LosslessFormat::Flac) {
                LosslessFormat::Wav => TargetProfile::LosslessWav,
                LosslessFormat::Flac => TargetProfile::LosslessFlac,
                LosslessFormat::Aiff => TargetProfile::LosslessAiff,
            };
            OutputPolicy { output_extension: match profile { TargetProfile::LosslessWav => "wav", TargetProfile::LosslessFlac => "flac", TargetProfile::LosslessAiff => "aiff", TargetProfile::CompatMp3 => "mp3" }, target_profile: profile }
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test compat_mode_always_targets_mp3 -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sync.rs src/metadata.rs tests/sync_policy.rs
git commit -m "feat: add explicit transcode policy"
```

