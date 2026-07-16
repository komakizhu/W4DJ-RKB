use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Compat,
    Lossless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LosslessFormat {
    Wav,
    Aiff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStrategy {
    #[default]
    Skip,
    Overwrite,
    Rename,
    UpdateMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FilenameRule {
    #[default]
    TitleArtist,
    ArtistTitle,
    Original,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum CandidateOperation {
    #[default]
    Convert,
    UpdateMetadata,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub source: String,
    pub destination: String,
    pub mode: Mode,
    #[serde(default)]
    pub lossless_format: Option<LosslessFormat>,
    #[serde(default)]
    pub conflict_strategy: ConflictStrategy,
    #[serde(default)]
    pub filename_rule: FilenameRule,
}

#[derive(clap::Parser)]
#[command(
    name = "w4dj",
    version = "2.2.1",
    author = "slipstream",
    about = "网易云音乐曲库同步器"
)]
pub struct Cmd {
    #[arg(long, short, default_value = "config.toml")]
    pub config: Option<String>,
    #[arg(long, default_value_t = false)]
    pub gui: bool,
}

#[cfg(test)]
mod tests {
    use super::{Config, LosslessFormat, Mode};

    #[test]
    fn parses_mode_and_lossless_output_format() {
        let toml = r#"
source = "/music/in"
destination = "/music/out"
mode = "compat"
lossless_format = "aiff"
"#;

        let config: Config = toml::from_str(toml).unwrap();
        assert!(matches!(config.mode, Mode::Compat));
        assert!(matches!(config.lossless_format, Some(LosslessFormat::Aiff)));
    }
}
