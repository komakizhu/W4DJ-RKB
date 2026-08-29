use crate::concurrency::{DEFAULT_CONCURRENCY_LIMIT, MAX_CONCURRENCY_LIMIT, MIN_CONCURRENCY_LIMIT};
use crate::config::{
    ConflictStrategy, ConversionMode, FilenameRule, LosslessFormat, Mode, NeteaseFilenameFormat,
};
use crate::gui::GuiShell;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

pub const SYNC_SLOT_COUNT: usize = 2;

pub fn default_concurrency_limit() -> u8 {
    DEFAULT_CONCURRENCY_LIMIT as u8
}

pub fn normalize_concurrency_limit(value: f64, fallback: u8) -> u8 {
    let fallback = fallback.clamp(MIN_CONCURRENCY_LIMIT as u8, MAX_CONCURRENCY_LIMIT as u8);
    if !value.is_finite() {
        return fallback;
    }
    value
        .round()
        .clamp(MIN_CONCURRENCY_LIMIT as f64, MAX_CONCURRENCY_LIMIT as f64) as u8
}

fn default_netease_database_bound() -> bool {
    true
}

fn deserialize_concurrency_limit<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let parsed = match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    Ok(normalize_concurrency_limit(
        parsed.unwrap_or(f64::NAN),
        default_concurrency_limit(),
    ))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSlotPreferences {
    pub source_directory: String,
    pub destination_directory: String,
}

impl SyncSlotPreferences {
    pub fn new(
        source_directory: impl Into<String>,
        destination_directory: impl Into<String>,
    ) -> Self {
        Self {
            source_directory: source_directory.into(),
            destination_directory: destination_directory.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPreferences {
    pub slots: [SyncSlotPreferences; SYNC_SLOT_COUNT],
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
    #[serde(default)]
    pub conversion_mode: ConversionMode,
    #[serde(default)]
    pub enhanced_mode: bool,
    #[serde(default)]
    pub conflict_strategy: ConflictStrategy,
    #[serde(default)]
    pub filename_rule: FilenameRule,
    #[serde(default)]
    pub netease_filename_format: NeteaseFilenameFormat,
    #[serde(default)]
    pub netease_database_path: Option<String>,
    #[serde(default = "default_netease_database_bound")]
    pub netease_database_bound: bool,
    #[serde(
        default = "default_concurrency_limit",
        deserialize_with = "deserialize_concurrency_limit"
    )]
    pub concurrency_limit: u8,
}

#[derive(Debug, Deserialize)]
struct LegacyAppPreferences {
    source_directory: String,
    destination_directory: String,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            slots: [
                SyncSlotPreferences::default(),
                SyncSlotPreferences::default(),
            ],
            mode: Mode::Compat,
            lossless_format: None,
            conversion_mode: ConversionMode::default(),
            enhanced_mode: false,
            conflict_strategy: ConflictStrategy::default(),
            filename_rule: FilenameRule::default(),
            netease_filename_format: NeteaseFilenameFormat::default(),
            netease_database_path: None,
            netease_database_bound: true,
            concurrency_limit: default_concurrency_limit(),
        }
    }
}

impl AppPreferences {
    pub fn from_shell_state(shell: &GuiShell) -> Self {
        Self {
            slots: [
                SyncSlotPreferences::new(
                    shell.source_directory.clone(),
                    shell.destination_directory.clone(),
                ),
                SyncSlotPreferences::default(),
            ],
            mode: shell.mode,
            lossless_format: shell.lossless_format,
            conversion_mode: ConversionMode::default(),
            enhanced_mode: false,
            conflict_strategy: ConflictStrategy::default(),
            filename_rule: FilenameRule::default(),
            netease_filename_format: NeteaseFilenameFormat::default(),
            netease_database_path: None,
            netease_database_bound: true,
            concurrency_limit: default_concurrency_limit(),
        }
    }
}

pub fn load_preferences(path: impl AsRef<Path>) -> io::Result<AppPreferences> {
    match fs::read_to_string(path) {
        Ok(contents) => parse_preferences(&contents),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(AppPreferences::default()),
        Err(err) => Err(err),
    }
}

fn parse_preferences(contents: &str) -> io::Result<AppPreferences> {
    let value: serde_json::Value = serde_json::from_str(contents)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    if value.get("slots").is_some() {
        return serde_json::from_value(value)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err));
    }

    let legacy: LegacyAppPreferences = serde_json::from_value(value)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    Ok(AppPreferences {
        slots: [
            SyncSlotPreferences::new(legacy.source_directory, legacy.destination_directory),
            SyncSlotPreferences::default(),
        ],
        mode: legacy.mode,
        lossless_format: legacy.lossless_format,
        conversion_mode: ConversionMode::default(),
        enhanced_mode: false,
        conflict_strategy: ConflictStrategy::default(),
        filename_rule: FilenameRule::default(),
        netease_filename_format: NeteaseFilenameFormat::default(),
        netease_database_path: None,
        netease_database_bound: true,
        concurrency_limit: default_concurrency_limit(),
    })
}

pub fn save_preferences(path: impl AsRef<Path>, preferences: &AppPreferences) -> io::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = serde_json::to_string_pretty(preferences)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write(path, contents)
}
