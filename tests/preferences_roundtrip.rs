use std::fs;
use tempfile::tempdir;
use w4dj::config::{ConflictStrategy, ConversionMode, FilenameRule, LosslessFormat, Mode};
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
        conversion_mode: ConversionMode::Direct,
        enhanced_mode: true,
        conflict_strategy: ConflictStrategy::Overwrite,
        filename_rule: FilenameRule::ArtistTitle,
        netease_filename_format: Default::default(),
        netease_database_path: Some(String::from("/music/sqlite_storage.sqlite3")),
        netease_database_bound: false,
        concurrency_limit: 4,
    };

    save_preferences(&path, &preferences).unwrap();
    let loaded = load_preferences(&path).unwrap();

    assert_eq!(loaded.slots[0].source_directory, "/music/in-1");
    assert_eq!(loaded.slots[0].destination_directory, "/music/out-1");
    assert_eq!(loaded.slots[1].source_directory, "/music/in-2");
    assert_eq!(loaded.slots[1].destination_directory, "");
    assert!(matches!(loaded.mode, Mode::Compat));
    assert!(matches!(loaded.lossless_format, Some(LosslessFormat::Aiff)));
    assert!(matches!(loaded.conversion_mode, ConversionMode::Direct));
    assert!(loaded.enhanced_mode);
    assert_eq!(loaded.conflict_strategy, ConflictStrategy::Overwrite);
    assert_eq!(loaded.filename_rule, FilenameRule::ArtistTitle);
    assert_eq!(
        loaded.netease_database_path.as_deref(),
        Some("/music/sqlite_storage.sqlite3")
    );
    assert!(!loaded.netease_database_bound);
    assert_eq!(loaded.concurrency_limit, 4);
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
    assert!(!loaded.enhanced_mode);
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
    assert!(!loaded.enhanced_mode);
}

#[test]
fn preferences_without_enhanced_mode_default_to_disabled() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("preferences.json");
    fs::write(
        &path,
        r#"{
            "slots": [
                {"source_directory": "/music/in", "destination_directory": "/music/out"},
                {"source_directory": "", "destination_directory": ""}
            ],
            "mode": "compat",
            "lossless_format": null,
            "conversion_mode": "scan_then_convert",
            "conflict_strategy": "skip",
            "filename_rule": "title_artist"
        }"#,
    )
    .unwrap();

    let loaded = load_preferences(&path).unwrap();

    assert!(!loaded.enhanced_mode);
}

#[test]
fn concurrency_limit_migrates_and_normalizes_legacy_values() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("preferences.json");
    let base = r#"{
        "slots": [
            {"source_directory": "/music/in", "destination_directory": "/music/out"},
            {"source_directory": "", "destination_directory": ""}
        ],
        "mode": "compat",
        "lossless_format": null
    }"#;

    fs::write(
        &path,
        base.replace(
            "null\n    }",
            "null,\n        \"concurrency_limit\": 8\n    }",
        ),
    )
    .unwrap();
    assert_eq!(load_preferences(&path).unwrap().concurrency_limit, 8);

    for (raw, expected) in [("0", 1), ("10.6", 10), ("-4", 1), ("\"invalid\"", 2)] {
        let contents = base.replace(
            "null\n    }",
            &format!("null,\n        \"concurrency_limit\": {raw}\n    }}"),
        );
        fs::write(&path, contents).unwrap();
        assert_eq!(load_preferences(&path).unwrap().concurrency_limit, expected);
    }
}
