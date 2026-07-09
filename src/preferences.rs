use crate::config::{LosslessFormat, Mode};
use crate::gui::GuiShell;
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

impl AppPreferences {
    pub fn from_shell_state(shell: &GuiShell) -> Self {
        Self {
            source_directory: shell.source_directory.clone(),
            destination_directory: shell.destination_directory.clone(),
            mode: shell.mode,
            lossless_format: shell.lossless_format,
        }
    }
}

pub fn load_preferences(path: impl AsRef<Path>) -> io::Result<AppPreferences> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(AppPreferences::default()),
        Err(err) => Err(err),
    }
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
