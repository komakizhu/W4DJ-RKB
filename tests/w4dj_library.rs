use std::collections::BTreeMap;
use std::fs;
use tempfile::tempdir;
use w4dj::analysis::{
    AnalysisLabel, ContinuousEmotionResult, DiscogsEffnetAnalysis, DiscogsEffnetHeadResult,
    EmotionCandidates, EmotionHeadStatus, HighLevelAnalysis, TrackAnalysis,
};
use w4dj::w4dj_library::W4djLibrary;

#[test]
fn high_level_json_round_trips_old_and_new_emotion_fields() {
    let old = serde_json::json!({
        "status": "completed",
        "modelVersion": "legacy",
        "genre": [],
        "mood": [{"label": "happy", "confidence": 0.9}],
        "instrument": [],
        "filtered": []
    });
    let parsed_old: HighLevelAnalysis = serde_json::from_value(old).unwrap();
    assert!(parsed_old.style.is_empty());
    assert!(parsed_old.emotion_candidates.is_none());
    assert!(parsed_old.mood_cluster.is_empty());
    assert_eq!(parsed_old.mood[0].label, "happy");

    let current = HighLevelAnalysis {
        status: "completed".into(),
        model_version: Some("emotion-v1".into()),
        reason: None,
        genre: Vec::new(),
        style: vec![AnalysisLabel {
            label: "House".into(),
            confidence: 0.8,
        }],
        mood: vec![AnalysisLabel {
            label: "happy".into(),
            confidence: 0.9,
        }],
        instrument: Vec::new(),
        emotion_candidates: Some(EmotionCandidates {
            emomusic: Some(ContinuousEmotionResult {
                model: "emomusic".into(),
                status: EmotionHeadStatus::Completed,
                valence: Some(7.0),
                arousal: Some(6.0),
                reason: None,
            }),
            muse: Some(ContinuousEmotionResult {
                model: "muse".into(),
                status: EmotionHeadStatus::ModelMissing,
                valence: None,
                arousal: None,
                reason: Some("missing".into()),
            }),
        }),
        mood_cluster: vec![AnalysisLabel {
            label: "passionate".into(),
            confidence: 0.7,
        }],
        mood_cluster_status: Some(EmotionHeadStatus::Completed),
        mood_cluster_reason: None,
        filtered: Vec::new(),
        discogs_effnet: None,
    };
    let value = serde_json::to_value(&current).unwrap();
    assert_eq!(value["emotionCandidates"]["emomusic"]["valence"], 7.0);
    assert_eq!(value["moodCluster"][0]["label"], "passionate");
    let round_trip: HighLevelAnalysis = serde_json::from_value(value).unwrap();
    assert_eq!(round_trip, current);
}

#[test]
fn continuous_emotion_json_rejects_invalid_completed_or_non_completed_values() {
    let invalid_completed = serde_json::json!({
        "model": "emomusic",
        "status": "completed",
        "valence": 10.0,
        "arousal": 5.0
    });
    assert!(serde_json::from_value::<ContinuousEmotionResult>(invalid_completed).is_err());

    let invalid_missing = serde_json::json!({
        "model": "muse",
        "status": "model_missing",
        "valence": 5.0,
        "arousal": null
    });
    assert!(serde_json::from_value::<ContinuousEmotionResult>(invalid_missing).is_err());
}

#[test]
fn output_root_switch_marks_old_slot_records_out_of_scope() {
    let directory = tempdir().unwrap();
    let root_a = directory.path().join("A");
    let root_b = directory.path().join("B");
    fs::create_dir_all(&root_a).unwrap();
    fs::create_dir_all(&root_b).unwrap();
    let output_a = root_a.join("a.mp3");
    let output_b = root_b.join("b.mp3");
    fs::write(&output_a, b"a").unwrap();
    fs::write(&output_b, b"b").unwrap();

    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    library
        .upsert_output_file(0, &root_a, None, &output_a)
        .unwrap();
    assert_eq!(library.stats().unwrap().available, 1);
    library
        .upsert_output_file(0, &root_b, None, &output_b)
        .unwrap();
    let stats = library.stats().unwrap();
    assert_eq!(stats.total, 2);
    assert_eq!(stats.available, 1);
    assert_eq!(stats.invalid, 1);
}

#[test]
fn invalid_cleanup_only_removes_database_rows() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("out");
    fs::create_dir_all(&root).unwrap();
    let output = root.join("song.mp3");
    fs::write(&output, b"audio").unwrap();
    let database_path = directory.path().join("w4dj.sqlite3");
    let mut library = W4djLibrary::open(&database_path).unwrap();
    library.upsert_output_file(0, &root, None, &output).unwrap();
    fs::remove_file(&output).unwrap();

    let stats = library.scan_invalid(|| false, |_, _, _| {}).unwrap();
    assert_eq!(stats.invalid, 1);
    assert_eq!(library.remove_invalid().unwrap(), 1);
    assert_eq!(library.stats().unwrap().total, 0);
    assert!(!output.exists());
}

#[test]
fn output_without_analysis_can_be_removed_from_the_independent_library() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("out");
    fs::create_dir_all(&root).unwrap();
    let output = root.join("not-yet-analyzed.mp3");
    fs::write(&output, b"audio").unwrap();
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    let key = library.upsert_output_file(0, &root, None, &output).unwrap();

    assert!(library.remove_analyzed_track(&key).unwrap());
    assert_eq!(library.stats().unwrap().total, 0);
    assert!(output.is_file());
}

#[test]
fn emotion_manifest_samples_available_outputs_and_preserves_legacy_mood() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("out");
    fs::create_dir_all(root.join("Album")).unwrap();
    let first = root.join("Album/first.mp3");
    let second = root.join("Album/second.mp3");
    let missing = root.join("Album/missing.mp3");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();
    fs::write(&missing, b"missing").unwrap();

    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    library.upsert_output_file(0, &root, None, &first).unwrap();
    library.upsert_output_file(0, &root, None, &second).unwrap();
    library
        .upsert_output_file(0, &root, None, &missing)
        .unwrap();
    let analysis = TrackAnalysis {
        path: first.display().to_string(),
        title: "First".into(),
        artist: "Artist".into(),
        album: "Album".into(),
        genre: String::new(),
        duration_seconds: Some(20.0),
        bpm: Some(120.0),
        key: None,
        scale: None,
        key_strength: None,
        integrated_loudness_lufs: None,
        loudness_range_lu: None,
        energy: Some(0.5),
        danceability: Some(0.5),
        beat_positions: Vec::new(),
        analyzed_at: String::new(),
        analyzer: "test".into(),
        analysis_version: "test".into(),
        source_size_bytes: None,
        source_modified_at: None,
        source_filename_format: None,
        drop_loudness_lufs: None,
        drop_analysis: None,
        high_level: Some(HighLevelAnalysis {
            status: "completed".into(),
            model_version: None,
            reason: None,
            genre: Vec::new(),
            style: Vec::new(),
            mood: vec![AnalysisLabel {
                label: "happy".into(),
                confidence: 0.91,
            }],
            instrument: Vec::new(),
            emotion_candidates: Some(EmotionCandidates {
                emomusic: Some(ContinuousEmotionResult {
                    model: "emomusic".into(),
                    status: EmotionHeadStatus::Completed,
                    valence: Some(7.0),
                    arousal: Some(6.0),
                    reason: None,
                }),
                muse: Some(ContinuousEmotionResult {
                    model: "muse".into(),
                    status: EmotionHeadStatus::Completed,
                    valence: Some(5.0),
                    arousal: Some(4.0),
                    reason: None,
                }),
            }),
            mood_cluster: vec![AnalysisLabel {
                label: "passionate".into(),
                confidence: 0.8,
            }],
            mood_cluster_status: Some(EmotionHeadStatus::Completed),
            mood_cluster_reason: None,
            filtered: Vec::new(),
            discogs_effnet: None,
        }),
    };
    library
        .apply_analysis_for_destination(&first, &analysis)
        .unwrap();

    let first_manifest = library.emotion_evaluation_manifest(0, 42).unwrap();
    let second_manifest = library.emotion_evaluation_manifest(0, 42).unwrap();
    assert_eq!(first_manifest.seed, second_manifest.seed);
    assert_eq!(first_manifest.tracks, second_manifest.tracks);
    let json = serde_json::to_value(&first_manifest).unwrap();
    assert!(json.get("schemaVersion").is_some());
    assert!(json["tracks"][0].get("clipSelection").is_some());
    assert!(json["tracks"][0].get("legacyMood").is_some());
    assert_eq!(first_manifest.sample_size, 3);
    assert!(
        first_manifest
            .tracks
            .iter()
            .all(|track| track.relative_path.starts_with("Album/"))
    );
    let first_entry = first_manifest
        .tracks
        .iter()
        .find(|track| track.track_id.contains("first.mp3"))
        .unwrap();
    assert_eq!(first_entry.legacy_mood["status"], "completed");
    assert_eq!(first_entry.legacy_mood["labels"][0]["label"], "happy");
    assert_eq!(first_entry.emomusic["status"], "completed");
    assert_eq!(first_entry.emomusic["valence"], 7.0);
    assert_eq!(first_entry.muse["arousal"], 4.0);
    assert_eq!(first_entry.mirex["status"], "completed");
    assert_eq!(first_entry.mirex["labels"][0]["label"], "passionate");
    let second_entry = first_manifest
        .tracks
        .iter()
        .find(|track| track.track_id.contains("second.mp3"))
        .unwrap();
    assert_eq!(second_entry.emomusic["status"], "model_missing");
    assert_eq!(second_entry.muse["status"], "model_missing");
    assert_eq!(second_entry.mirex["status"], "model_missing");
    assert!(matches!(
        first_entry.clip_selection.as_str(),
        "peakEnergy" | "startFallback" | "fullTrack" | "drop"
    ));
    assert!(
        first_manifest
            .tracks
            .iter()
            .all(|track| track.clip_duration_seconds <= 10.0)
    );
    assert_eq!(
        library
            .emotion_evaluation_manifest(2, 42)
            .unwrap()
            .sample_size,
        2
    );
}

#[test]
fn discogs_heads_use_independent_projection_columns_and_preserve_completed_siblings() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("out");
    fs::create_dir_all(&root).unwrap();
    let output = root.join("discogs.mp3");
    fs::write(&output, b"audio").unwrap();
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    let track_key = library.upsert_output_file(0, &root, None, &output).unwrap();

    let mut heads = BTreeMap::new();
    for (id, label) in [
        ("moodTheme", "dark"),
        ("approachability", "approachable"),
        ("instrumentation", "synthesizer"),
        ("timbre", "bright"),
        ("danceability", "danceable"),
    ] {
        heads.insert(
            id.to_string(),
            DiscogsEffnetHeadResult {
                model: id.to_string(),
                status: "completed".into(),
                version: "discogs-test".into(),
                labels: vec![AnalysisLabel {
                    label: label.into(),
                    confidence: 0.9,
                }],
                scores: BTreeMap::from([(label.to_string(), 0.9)]),
                frame_count: 3,
                threshold: Some(0.35),
                selected_class: Some(label.into()),
                selected_confidence: Some(0.9),
                reason: None,
            },
        );
    }
    let base = TrackAnalysis {
        path: output.display().to_string(),
        title: "Discogs".into(),
        artist: "Artist".into(),
        album: String::new(),
        genre: String::new(),
        duration_seconds: Some(12.0),
        bpm: Some(124.0),
        key: Some("C".into()),
        scale: Some("major".into()),
        key_strength: None,
        integrated_loudness_lufs: None,
        loudness_range_lu: None,
        energy: Some(0.4),
        danceability: Some(0.25),
        beat_positions: Vec::new(),
        analyzed_at: String::new(),
        analyzer: "test".into(),
        analysis_version: "discogs-test".into(),
        source_size_bytes: None,
        source_modified_at: None,
        source_filename_format: None,
        drop_loudness_lufs: None,
        drop_analysis: None,
        high_level: Some(HighLevelAnalysis {
            status: "completed".into(),
            model_version: Some("discogs-test".into()),
            reason: None,
            genre: Vec::new(),
            style: vec![AnalysisLabel {
                label: "House".into(),
                confidence: 0.8,
            }],
            mood: Vec::new(),
            instrument: Vec::new(),
            emotion_candidates: None,
            mood_cluster: Vec::new(),
            mood_cluster_status: None,
            mood_cluster_reason: None,
            filtered: Vec::new(),
            discogs_effnet: Some(DiscogsEffnetAnalysis {
                embedding_model: "discogs-effnet-bs64-1".into(),
                embedding_dimensions: 1280,
                input_shape: vec![64, 128, 96],
                heads,
            }),
        }),
    };
    library
        .apply_analysis_for_destination(&output, &base)
        .unwrap();
    let detail = library.track_detail(&track_key).unwrap().unwrap();
    assert!(detail.discogs_mood_theme_json.contains("dark"));
    assert!(detail.discogs_approachability_json.contains("approachable"));
    assert!(detail.discogs_instrumentation_json.contains("synthesizer"));
    assert!(detail.discogs_timbre_json.contains("bright"));
    assert!(detail.discogs_danceability_json.contains("danceable"));
    assert_eq!(detail.style_json, r#"[{"label":"House","confidence":0.8}]"#);
    assert_eq!(detail.danceability, Some(0.25));

    let mut failed = base.clone();
    if let Some(high_level) = failed.high_level.as_mut()
        && let Some(discogs) = high_level.discogs_effnet.as_mut()
    {
        discogs.heads.get_mut("moodTheme").unwrap().status = "failed".into();
        discogs.heads.get_mut("danceability").unwrap().status = "model_missing".into();
    }
    library
        .apply_analysis_for_destination(&output, &failed)
        .unwrap();
    let after = library.track_detail(&track_key).unwrap().unwrap();
    assert!(after.discogs_mood_theme_json.contains("dark"));
    assert!(after.discogs_danceability_json.contains("danceable"));
}
