use std::fs;
use tempfile::tempdir;
use w4dj::config::{LosslessFormat, Mode};
use w4dj::preferences::{AppPreferences, SyncSlotPreferences, load_preferences, save_preferences};

#[test]
fn preferences_roundtrip_persists_both_sync_slots() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("preferences.json");

    let preferences = AppPreferences {
        slots: [
            SyncSlotPreferences::new("/music/in-1", "/music/out-1"),
            SyncSlotPreferences::new("/music/in-2", ""),
        ],
        mode: Mode::Compat,
        lossless_format: Some(LosslessFormat::Aiff),
    };

    save_preferences(&path, &preferences).unwrap();
    let loaded = load_preferences(&path).unwrap();

    assert_eq!(loaded.slots[0].source_directory, "/music/in-1");
    assert_eq!(loaded.slots[0].destination_directory, "/music/out-1");
    assert_eq!(loaded.slots[1].source_directory, "/music/in-2");
    assert_eq!(loaded.slots[1].destination_directory, "");
    assert!(matches!(loaded.mode, Mode::Compat));
    assert!(matches!(loaded.lossless_format, Some(LosslessFormat::Aiff)));
}

#[test]
fn legacy_preferences_migrate_into_slot_one() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("preferences.json");
    fs::write(
        &path,
        r#"{
            "source_directory": "/legacy/in",
            "destination_directory": "/legacy/out",
            "mode": "compat",
            "lossless_format": null
        }"#,
    )
    .unwrap();

    let loaded = load_preferences(&path).unwrap();

    assert_eq!(loaded.slots[0].source_directory, "/legacy/in");
    assert_eq!(loaded.slots[0].destination_directory, "/legacy/out");
    assert_eq!(loaded.slots[1], SyncSlotPreferences::default());
}

#[test]
fn missing_preferences_file_uses_defaults() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("missing.json");

    let loaded = load_preferences(&path).unwrap();

    assert_eq!(
        loaded.slots,
        [
            SyncSlotPreferences::default(),
            SyncSlotPreferences::default(),
        ]
    );
    assert!(matches!(loaded.mode, Mode::Compat));
    assert_eq!(loaded.lossless_format, None);
}
