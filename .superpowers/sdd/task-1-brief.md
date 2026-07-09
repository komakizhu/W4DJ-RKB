### Task 1: Define shared config and mode model

**Files:**
- Modify: `/private/tmp/w4dj-review/src/config.rs`
- Modify: `/private/tmp/w4dj-review/src/main.rs`
- Test: `/private/tmp/w4dj-review/src/config.rs` behavior through `cargo test` or `cargo check`

**Interfaces:**
- Consumes: TOML config from disk and parsed CLI args.
- Produces: `Config`, `Mode`, and a lossless output format type that later tasks can reuse.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn parses_mode_and_lossless_output_format() {
    let toml = r#"
source = "/music/in"
destination = "/music/out"
mode = "compat"
lossless_format = "flac"
"#;

    let config: Config = toml::from_str(toml).unwrap();
    assert!(matches!(config.mode, Mode::Compat));
    assert!(matches!(config.lossless_format, Some(LosslessFormat::Flac)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test parses_mode_and_lossless_output_format -v`
Expected: FAIL because the new mode and format types are not defined yet.

- [ ] **Step 3: Write minimal implementation**

```rust
#[derive(Debug, Deserialize)]
pub enum Mode {
    #[serde(rename = "compat")]
    Compat,
    #[serde(rename = "lossless")]
    Lossless,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum LosslessFormat {
    #[serde(rename = "wav")]
    Wav,
    #[serde(rename = "flac")]
    Flac,
    #[serde(rename = "aiff")]
    Aiff,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub source: String,
    pub destination: String,
    pub mode: Mode,
    #[serde(default)]
    pub lossless_format: Option<LosslessFormat>,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test parses_mode_and_lossless_output_format -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/main.rs
git commit -m "feat: define shared sync config"
```

