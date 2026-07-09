use tempfile::tempdir;
use w4dj::config::{LosslessFormat, Mode};
use w4dj::preferences::{AppPreferences, load_preferences, save_preferences};

#[test]
fn preferences_roundtrip_persists_last_directories() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("preferences.json");

    let preferences = AppPreferences {
        source_directory: "/music/in".into(),
        destination_directory: "/music/out".into(),
        mode: Mode::Compat,
        lossless_format: Some(LosslessFormat::Aiff),
    };

    save_preferences(&path, &preferences).unwrap();
    let loaded = load_preferences(&path).unwrap();

    assert_eq!(loaded.source_directory, "/music/in");
    assert_eq!(loaded.destination_directory, "/music/out");
    assert!(matches!(loaded.mode, Mode::Compat));
    assert!(matches!(loaded.lossless_format, Some(LosslessFormat::Aiff)));
}

#[test]
fn missing_preferences_file_uses_defaults() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("missing.json");

    let loaded = load_preferences(&path).unwrap();

    assert_eq!(loaded.source_directory, "");
    assert_eq!(loaded.destination_directory, "");
    assert!(matches!(loaded.mode, Mode::Compat));
    assert_eq!(loaded.lossless_format, None);
}
