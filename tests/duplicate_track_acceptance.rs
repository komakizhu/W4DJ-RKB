//! Manual real-data acceptance for the duplicate-track disambiguation path.
//!
//! Run explicitly with the two source files mounted:
//! `W4DJ_DUPLICATE_INPUT=/path/to/originals W4DJ_DUPLICATE_DATABASE=/path/to/sqlite \
//!  cargo test --test duplicate_track_acceptance -- --ignored --nocapture`

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use w4dj::analysis::read_track_metadata;
use w4dj::concurrency::GlobalConcurrencyBudget;
use w4dj::config::{
    ConflictStrategy, FilenameNormalizationPolicy, FilenameRule, Mode, NeteaseFilenameFormat,
};
use w4dj::netease::NeteaseMetadataResolver;
use w4dj::preview::{
    attach_netease_identities, build_sync_preview_with_settings_and_netease_observed_with_policy,
};
use w4dj::sync::{
    ActiveFfmpegRegistry, ConversionMetadataContext,
    sync_music_library_transactional_with_observer_and_budget_and_context_with_policy,
};
use w4dj::task::TaskController;
use w4dj::w4dj_library::W4djLibrary;

fn assert_exiftool_identity(path: &Path, expected: &w4dj::analysis::TrackMetadata) {
    let output = Command::new("exiftool")
        .args(["-j", "-Title", "-Artist", "-Album"])
        .arg(path)
        .output()
        .expect("exiftool must be installed for the real acceptance");
    assert!(
        output.status.success(),
        "exiftool failed for {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("exiftool JSON output");
    let row = rows.first().expect("exiftool row");
    assert_eq!(
        row.get("Title").and_then(|value| value.as_str()),
        Some(expected.title.as_str())
    );
    assert_eq!(
        row.get("Artist").and_then(|value| value.as_str()),
        Some(expected.artist.as_str())
    );
    assert_eq!(
        row.get("Album").and_then(|value| value.as_str()),
        Some(expected.album.as_str())
    );
    eprintln!(
        "duplicate acceptance: exiftool {} => title={:?}, artist={:?}, album={:?}",
        path.display(),
        row.get("Title"),
        row.get("Artist"),
        row.get("Album")
    );
}

#[test]
#[ignore = "requires the mounted NetEase source directory and database"]
fn stone_kold_duplicate_tracks_convert_to_distinct_outputs_with_metadata() {
    let input = PathBuf::from(env::var("W4DJ_DUPLICATE_INPUT").expect("W4DJ_DUPLICATE_INPUT"));
    let database =
        PathBuf::from(env::var("W4DJ_DUPLICATE_DATABASE").expect("W4DJ_DUPLICATE_DATABASE"));
    let output = tempfile::tempdir().expect("temporary output directory");
    let library_path = output.path().join("w4dj.sqlite3");

    assert!(
        input.is_dir(),
        "duplicate source directory is missing: {}",
        input.display()
    );
    assert!(
        database.is_file(),
        "NetEase database is missing: {}",
        database.display()
    );
    let resolver = NeteaseMetadataResolver::load_exact(&database).expect("load NetEase database");
    eprintln!("duplicate acceptance: resolver loaded");
    let metadata_context = ConversionMetadataContext {
        netease: Arc::new(resolver),
    };

    let mut preview = build_sync_preview_with_settings_and_netease_observed_with_policy(
        input.to_str().expect("UTF-8 input path"),
        output.path().to_str().expect("UTF-8 output path"),
        Mode::Compat,
        None,
        ConflictStrategy::Skip,
        FilenameRule::TitleArtist,
        NeteaseFilenameFormat::default(),
        FilenameNormalizationPolicy::PreserveSource,
        None,
    )
    .expect("build duplicate preview")
    .expect("preview was not cancelled");
    eprintln!(
        "duplicate acceptance: preview built with {} candidates",
        preview.candidates.len()
    );
    attach_netease_identities(&mut preview, metadata_context.netease.as_ref());

    let mut candidates = preview
        .candidates
        .iter()
        .filter(|candidate| candidate.source_path.contains("STONE KOLD"))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.source_path.cmp(&right.source_path));
    assert_eq!(candidates.len(), 2, "preview candidates: {candidates:#?}");
    assert_ne!(
        candidates[0].destination_path,
        candidates[1].destination_path
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.netease_track_id.is_some())
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.netease_album_id.is_some())
    );
    assert!(candidates.iter().all(|candidate| candidate.album.is_some()));
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.disambiguation_reason.is_some())
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.destination_path.contains('['))
    );
    eprintln!("duplicate acceptance: identities attached, starting conversion");

    let source_files = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.name.clone(),
                (
                    candidate.source_size_bytes.to_string(),
                    PathBuf::from(&candidate.source_path),
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    let source_files = source_files.iter().collect::<HashMap<_, _>>();
    let task = TaskController::running(candidates.len());
    let mut failures = Vec::new();
    let snapshot =
        sync_music_library_transactional_with_observer_and_budget_and_context_with_policy(
            &source_files,
            output.path().to_str().expect("UTF-8 output path"),
            &Mode::Compat,
            None,
            NeteaseFilenameFormat::default(),
            FilenameNormalizationPolicy::PreserveSource,
            &task,
            |_name, _temporary| Ok(()),
            |_name, _task, error| {
                if let Some(error) = error {
                    failures.push(error.to_string());
                }
                Ok(())
            },
            Arc::new(GlobalConcurrencyBudget::new(2)),
            Arc::new(ActiveFfmpegRegistry::new()),
            &metadata_context,
        )
        .expect("convert duplicate tracks");
    eprintln!("duplicate acceptance: conversion completed");
    assert!(failures.is_empty(), "conversion failures: {failures:?}");
    assert_eq!(snapshot.completed, 2);

    let mut library = W4djLibrary::open(&library_path).expect("open W4DJ library");
    for candidate in &candidates {
        let destination = Path::new(&candidate.destination_path);
        assert!(
            destination.is_file(),
            "missing output: {}",
            destination.display()
        );
        let source_metadata = read_track_metadata(Path::new(&candidate.source_path));
        let output_metadata = read_track_metadata(destination);
        assert_eq!(output_metadata.title, source_metadata.title);
        assert_eq!(output_metadata.artist, source_metadata.artist);
        assert_eq!(output_metadata.album, source_metadata.album);
        assert_exiftool_identity(destination, &source_metadata);
        library
            .upsert_output_file(
                0,
                output.path(),
                Some(Path::new(&candidate.source_path)),
                destination,
            )
            .expect("register output");
        let key = format!(
            "output:{}",
            fs::canonicalize(destination).unwrap().display()
        );
        let track = library
            .track_detail(&key)
            .expect("query output")
            .expect("track row");
        assert_eq!(track.album, source_metadata.album);
        assert_eq!(track.netease_track_id, None);
    }

    if let Ok(existing_path) = env::var("W4DJ_DUPLICATE_EXISTING_OUTPUT") {
        let existing_dir = tempfile::tempdir().expect("existing-output directory");
        let existing_destination = existing_dir
            .path()
            .join("STONE KOLD - Skybreak, Subten.mp3");
        fs::copy(&existing_path, &existing_destination).expect("copy existing output fixture");
        let mut rerun = build_sync_preview_with_settings_and_netease_observed_with_policy(
            input.to_str().expect("UTF-8 input path"),
            existing_dir
                .path()
                .to_str()
                .expect("UTF-8 existing output path"),
            Mode::Compat,
            None,
            ConflictStrategy::Skip,
            FilenameRule::TitleArtist,
            NeteaseFilenameFormat::default(),
            FilenameNormalizationPolicy::PreserveSource,
            None,
        )
        .expect("build rerun preview")
        .expect("rerun preview was not cancelled");
        attach_netease_identities(&mut rerun, metadata_context.netease.as_ref());
        let rerun_candidates = rerun
            .candidates
            .iter()
            .filter(|candidate| candidate.source_path.contains("STONE KOLD"))
            .collect::<Vec<_>>();
        assert_eq!(rerun_candidates.len(), 1);
        assert!(rerun_candidates[0].destination_path.contains('['));
        assert!(
            existing_destination.is_file(),
            "existing output was not preserved"
        );
    }
}
