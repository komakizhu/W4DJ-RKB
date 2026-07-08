use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Compat,
    Lossless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LosslessFormat {
    Wav,
    Flac,
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

#[derive(clap::Parser)]
#[command(
    name = "w4dj",
    version = "0.1.0",
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
lossless_format = "flac"
"#;

        let config: Config = toml::from_str(toml).unwrap();
        assert!(matches!(config.mode, Mode::Compat));
        assert!(matches!(config.lossless_format, Some(LosslessFormat::Flac)));
    }
}
