//! End-to-end acceptance for the reviewed W4DJ playlist export workflow.
//!
//! This test is intentionally ignored during the normal suite because it
//! invokes the local FFmpeg binary and uses a real supplied `.w4dj` file.
//! Run it with `W4DJ_ACCEPTANCE_PLAYLIST` pointing at the frozen handoff file.

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tempfile::tempdir;
use w4dj::analysis::read_embedded_track_metadata;
use w4dj::concurrency::GlobalConcurrencyBudget;
use w4dj::config::{
    ConflictStrategy, FilenameNormalizationPolicy, FilenameRule, Mode, NeteaseFilenameFormat,
};
use w4dj::dj_playlist::{ImportedDjPlaylist, parse_w4dj_playlist, serialize_w4dj_playlist};
use w4dj::dj_playlist_match::{DjOutputCandidate, DjPlaylistMatchKind};
use w4dj::m3u8::{
    ResolvedDjPlaylistTrack, build_relative_m3u8_with_summary, write_relative_m3u8_atomic,
};
use w4dj::preview::build_sync_preview_with_settings_and_netease_observed_with_policy;
use w4dj::sync::{
    ActiveFfmpegRegistry, ConversionMetadataContext,
    sync_music_library_transactional_with_observer_and_budget_and_context_with_policy,
};
use w4dj::task::TaskController;
use w4dj::w4dj_library::{CommittedOutputFacts, W4djLibrary};

#[test]
#[ignore = "requires the supplied W4DJ playlist and local FFmpeg"]
fn john_summit_playlist_converts_matches_and_exports_complete_m3u8() {
    let playlist_path = PathBuf::from(
        env::var("W4DJ_ACCEPTANCE_PLAYLIST")
            .expect("set W4DJ_ACCEPTANCE_PLAYLIST to the supplied .w4dj file"),
    );
    assert!(
        playlist_path.is_file(),
        "acceptance playlist is missing: {}",
        playlist_path.display()
    );

    let workspace = tempdir().expect("acceptance workspace");
    let source_directory = workspace.path().join("silence-sources");
    let output_directory = workspace.path().join("converted-output");
    let source_labels_path = workspace.path().join("source-labels.json");
    fs::create_dir_all(&output_directory).expect("create output directory");
    fs::write(
        &source_labels_path,
        r#"[
          {"position":1,"title":"Ferrari (Extended Mix)","artist_display":"James Hype / Miggy Dela Rosa"},
          {"position":2,"title":"Eat Your Man (with Nelly Furtado) [Extended]","artist_display":"Dom Dolla / Nelly Furtado"},
          {"position":3,"title":"Sun Goes Down (Extended Mix)","artist_display":"Cloonee"},
          {"position":4,"title":"Gimme That Bounce (Original Mix)","artist_display":"Mau P"},
          {"position":5,"title":"Atmosphere (Extended Mix)","artist_display":"FISHER / Kita Alexander"},
          {"position":6,"title":"Taka (Extended Mix)","artist_display":"SIDEPIECE / San Pacho"},
          {"position":7,"title":"Voodoo (Extended Mix)","artist_display":"Gorgon City"},
          {"position":8,"title":"Where You Are (Extended Mix)","artist_display":"John Summit / HAYLA"}
        ]"#,
    )
    .expect("write source label map");
    generate_silence_sources(&playlist_path, &source_directory, Some(&source_labels_path));

    let playlist_bytes = fs::read(&playlist_path).expect("read acceptance playlist");
    let playlist = parse_w4dj_playlist(&playlist_bytes, Some(&playlist_path))
        .expect("parse supplied W4DJ playlist");
    assert_eq!(playlist.format_version, 2);
    assert_eq!(playlist.tracks.len(), 8);
    let serialized = serialize_w4dj_playlist(&playlist).expect("serialize playlist");
    let serialized_json: serde_json::Value =
        serde_json::from_slice(&serialized).expect("serialized playlist JSON");
    assert!(
        serialized_json["tracks"]
            .as_array()
            .expect("serialized tracks")
            .iter()
            .all(|track| track["netease_track_id"].is_null())
    );

    let preview = build_sync_preview_with_settings_and_netease_observed_with_policy(
        source_directory.to_str().expect("UTF-8 source directory"),
        output_directory.to_str().expect("UTF-8 output directory"),
        Mode::Compat,
        None,
        ConflictStrategy::Skip,
        FilenameRule::TitleArtist,
        NeteaseFilenameFormat::default(),
        FilenameNormalizationPolicy::PreserveSource,
        None,
    )
    .expect("build silence-source preview")
    .expect("preview was not cancelled");
    assert_eq!(preview.candidates.len(), playlist.tracks.len());

    let source_files = preview
        .candidates
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
    let candidates_by_name = preview
        .candidates
        .iter()
        .map(|candidate| (candidate.name.clone(), candidate.clone()))
        .collect::<HashMap<_, _>>();
    let mut library = W4djLibrary::open(&workspace.path().join("w4dj.sqlite3"))
        .expect("open W4DJ acceptance library");
    let batch_id = "acceptance-john-summit-batch".to_string();
    let mut failures = Vec::new();
    let task = TaskController::running(source_files.len());
    let snapshot =
        sync_music_library_transactional_with_observer_and_budget_and_context_with_policy(
            &source_files,
            output_directory.to_str().expect("UTF-8 output directory"),
            &Mode::Compat,
            None,
            NeteaseFilenameFormat::default(),
            FilenameNormalizationPolicy::PreserveSource,
            &task,
            |_name, _temporary| Ok(()),
            |name, _task, error| {
                if let Some(error) = error {
                    failures.push(format!("{name}: {error}"));
                    return Ok(());
                }
                let candidate = candidates_by_name
                    .get(name)
                    .ok_or_else(|| io::Error::other(format!("unknown converted source: {name}")))?;
                let destination = Path::new(&candidate.destination_path);
                let embedded = read_embedded_track_metadata(destination);
                let title = if embedded.title.trim().is_empty() {
                    candidate.name.as_str()
                } else {
                    embedded.title.as_str()
                };
                let artist = embedded.artist.as_str();
                library
                    .upsert_committed_output_in_root(
                        0,
                        &output_directory,
                        Some(Path::new(&candidate.source_path)),
                        destination,
                        title,
                        artist,
                        &CommittedOutputFacts {
                            conversion_batch_id: Some(batch_id.clone()),
                            conversion_mode: Some("compat".to_string()),
                            filename_rule: Some("title_artist".to_string()),
                            filename_normalization_policy: Some("preserve_source".to_string()),
                            ..Default::default()
                        },
                    )
                    .map_err(|error| io::Error::other(error.to_string()))?;
                Ok(())
            },
            Arc::new(GlobalConcurrencyBudget::new(2)),
            Arc::new(ActiveFfmpegRegistry::new()),
            &ConversionMetadataContext::default(),
        )
        .expect("convert silence sources");
    assert!(failures.is_empty(), "conversion failures: {failures:?}");
    assert_eq!(snapshot.completed, playlist.tracks.len());

    let converted_paths = preview
        .candidates
        .iter()
        .map(|candidate| PathBuf::from(&candidate.destination_path))
        .collect::<Vec<_>>();
    assert!(
        converted_paths
            .iter()
            .all(|path| path.is_file()
                && fs::metadata(path).is_ok_and(|metadata| metadata.len() > 0))
    );

    library
        .upsert_imported_dj_playlist(&playlist)
        .expect("persist imported playlist");
    let initial_report = library
        .compute_imported_dj_playlist_matches(&playlist.playlist_id)
        .expect("compute playlist matches");
    assert_eq!(initial_report.total, 8);
    assert_eq!(initial_report.matched_count, 8);
    assert_eq!(initial_report.unmatched_count, 0);
    assert!(initial_report.matches.iter().all(|row| {
        row.kind == DjPlaylistMatchKind::RecentBm25f
            && row.status == "matched"
            && row.score.is_some_and(|score| (65..=100).contains(&score))
    }));
    library
        .replace_imported_dj_playlist_matches(&playlist.playlist_id, &initial_report)
        .expect("persist computed matches");
    for position in playlist.tracks.iter().map(|track| track.position) {
        library
            .set_imported_dj_playlist_match_confirmed(&playlist.playlist_id, position, true)
            .expect("confirm matched row");
    }

    let report = library
        .get_imported_dj_playlist_match_report(&playlist.playlist_id)
        .expect("read reviewed report");
    assert_eq!(report.matched_count, 8);
    assert!(report.matches.iter().all(|row| {
        row.status == "matched"
            && row.confirmed
            && row.track_key.is_some()
            && row
                .destination_path
                .as_ref()
                .is_some_and(|path| path.is_file())
    }));

    let candidates = library
        .available_dj_output_candidates()
        .expect("read output candidates");
    let candidates_by_key = candidates
        .iter()
        .map(|candidate| (candidate.track_key.as_str(), candidate))
        .collect::<HashMap<_, _>>();
    let resolved = report
        .matches
        .iter()
        .map(|row| {
            let track_key = row.track_key.as_deref().expect("reviewed track key");
            let candidate = candidates_by_key
                .get(track_key)
                .expect("reviewed candidate");
            ResolvedDjPlaylistTrack {
                position: row.position,
                title: row.title.clone(),
                artist_display: row.artist_display.clone(),
                duration_seconds: candidate.duration_seconds,
                destination_path: row
                    .destination_path
                    .clone()
                    .expect("reviewed destination path"),
            }
        })
        .collect::<Vec<_>>();
    let m3u8_path = workspace.path().join("exports").join("john-summit.m3u8");
    fs::create_dir_all(m3u8_path.parent().expect("M3U8 parent")).expect("create M3U8 directory");
    let (contents, summary) = build_relative_m3u8_with_summary(&playlist, &resolved, &m3u8_path)
        .expect("build complete M3U8");
    assert_eq!(summary.matched_count, 8);
    assert_eq!(summary.total, 8);
    assert!(summary.omitted.is_empty());
    assert_eq!(contents.matches("#EXTINF:").count(), 8);
    let relative_entries = contents
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    assert_eq!(relative_entries.len(), 8);
    assert!(relative_entries.iter().all(|entry| {
        m3u8_path
            .parent()
            .expect("M3U8 parent")
            .join(entry)
            .is_file()
    }));
    write_relative_m3u8_atomic(&m3u8_path, &contents).expect("write complete M3U8");
    assert_eq!(fs::read_to_string(&m3u8_path).expect("read M3U8"), contents);

    let positions = report
        .matches
        .iter()
        .map(|row| row.position)
        .collect::<HashSet<_>>();
    assert_eq!(positions.len(), playlist.tracks.len());
}

#[test]
#[ignore = "requires frozen W4DJ fixtures and local FFmpeg"]
fn frozen_generated_playlist_matrix_matches_and_exports_without_omissions() {
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-artifacts/w4dj");

    let tech_house_path = fixture_root.join("acceptance-tech-house-8.w4dj");
    let tech_house = load_fixture_playlist(&tech_house_path);
    let tech_workspace = tempdir().expect("tech house workspace");
    let tech_source = tech_workspace.path().join("sources");
    let tech_output = tech_workspace.path().join("outputs");
    let tech_labels = tech_workspace.path().join("source-labels.json");
    write_source_labels(
        &tech_labels,
        &[
            (1, "Deeper MSTR C Extended Mix", "MicahelBM / JAYIE"),
            (2, "Trago de Ron Original Mix", "Marc Suarez"),
            (3, "Bounce Back (Original Mix)", "Trizzoh"),
            (4, "ONLYFANS Extended Mix", "S Zer0 & Valmonte"),
            (5, "The Way Original Mix", "MXJ / AJSE"),
            (
                6,
                "Like This Extended Mix",
                "Diseptix / Incognet / Alex Helder",
            ),
            (7, "Paralyzed Original Mix", "Cyrus"),
            (8, "Wrong Feels Right Extended Mix", "Format B"),
        ],
    );
    generate_silence_sources(&tech_house_path, &tech_source, Some(&tech_labels));
    let mut tech_library = W4djLibrary::open(&tech_workspace.path().join("w4dj.sqlite3"))
        .expect("open tech house library");
    let tech_candidates = convert_fixture_batch(
        &tech_source,
        &tech_output,
        &mut tech_library,
        "acceptance-tech-house-batch",
    );
    assert_eq!(tech_candidates.len(), 8);
    tech_library
        .upsert_imported_dj_playlist(&tech_house)
        .expect("persist tech house playlist");
    let tech_report = tech_library
        .compute_imported_dj_playlist_matches(&tech_house.playlist_id)
        .expect("match tech house playlist");
    assert_eq!(tech_report.matched_count, 8);
    assert!(tech_report.matches.iter().all(|row| {
        row.kind == DjPlaylistMatchKind::RecentBm25f
            && row.score.is_some_and(|score| (65..=100).contains(&score))
    }));
    tech_library
        .replace_imported_dj_playlist_matches(&tech_house.playlist_id, &tech_report)
        .expect("persist tech house matches");
    confirm_all_rows(&mut tech_library, &tech_house);
    assert_complete_m3u8(
        &tech_library,
        &tech_house,
        &tech_workspace.path().join("exports/tech-house.m3u8"),
        8,
    );

    let ukg_path = fixture_root.join("acceptance-uk-garage-10.w4dj");
    let ukg = load_fixture_playlist(&ukg_path);
    let ukg_workspace = tempdir().expect("UK Garage workspace");
    let ukg_source = ukg_workspace.path().join("sources");
    let ukg_output = ukg_workspace.path().join("outputs");
    let ukg_labels = ukg_workspace.path().join("source-labels.json");
    write_source_labels(
        &ukg_labels,
        &[
            (1, "Lose My Cool", "DJ Q"),
            (2, "Hyper", "Bodhi"),
            (3, "This Bassline Smells Like Oil", "Ghoulish"),
            (4, "Best of Me", "1111"),
            (5, "Target", "Gemi / Kori"),
            (6, "Dub Selecta 16", "PJ Bridger & Daffy"),
            (7, "Riddim", "Eloquin / Reimond"),
            (8, "On Tour", "Sempa"),
            (9, "The Power", "TARZI"),
            (10, "Up A Little", "Me and George"),
        ],
    );
    generate_silence_sources(&ukg_path, &ukg_source, Some(&ukg_labels));
    generate_tagged_silence_file(
        &ukg_source,
        "extra-target-practice.wav",
        "Target Practice",
        "Kori",
    );
    generate_tagged_silence_file(
        &ukg_source,
        "extra-the-power-within.wav",
        "The Power Within",
        "TARZAN",
    );
    let mut ukg_library = W4djLibrary::open(&ukg_workspace.path().join("w4dj.sqlite3"))
        .expect("open UK Garage library");
    let ukg_candidates = convert_fixture_batch(
        &ukg_source,
        &ukg_output,
        &mut ukg_library,
        "acceptance-ukg-batch",
    );
    assert_eq!(ukg_candidates.len(), 12);
    ukg_library
        .upsert_imported_dj_playlist(&ukg)
        .expect("persist UK Garage playlist");
    let ukg_report = ukg_library
        .compute_imported_dj_playlist_matches(&ukg.playlist_id)
        .expect("match UK Garage playlist");
    assert_eq!(ukg_report.matched_count, 10);
    assert!(ukg_report.matches.iter().all(|row| {
        row.kind == DjPlaylistMatchKind::RecentBm25f
            && row.score.is_some_and(|score| (65..=100).contains(&score))
    }));
    assert!(ukg_report.matches.iter().all(|row| {
        row.destination_path.as_ref().is_some_and(|path| {
            !path.to_string_lossy().contains("extra-target-practice")
                && !path.to_string_lossy().contains("extra-the-power-within")
        })
    }));
    ukg_library
        .replace_imported_dj_playlist_matches(&ukg.playlist_id, &ukg_report)
        .expect("persist UK Garage matches");
    confirm_all_rows(&mut ukg_library, &ukg);
    assert_complete_m3u8(
        &ukg_library,
        &ukg,
        &ukg_workspace.path().join("exports/uk-garage.m3u8"),
        10,
    );

    let melodic_path = fixture_root.join("acceptance-melodic-techno-6.w4dj");
    let melodic = load_fixture_playlist(&melodic_path);
    let melodic_workspace = tempdir().expect("Melodic Techno workspace");
    let melodic_output = melodic_workspace.path().join("outputs");
    let historical_source = melodic_workspace.path().join("historical-sources");
    generate_tagged_silence_file(
        &historical_source,
        "Ipnosi Original Mix.wav",
        "Ipnosi Original Mix",
        "RIVE",
    );
    generate_tagged_silence_file(
        &historical_source,
        "Rhea Original Mix.wav",
        "Rhea Original Mix",
        "ANRA",
    );
    let recent_source = melodic_workspace.path().join("recent-sources");
    generate_tagged_silence_file(
        &recent_source,
        "Solara Original Mix.wav",
        "Solara Original Mix",
        "Hakan",
    );
    generate_tagged_silence_file(
        &recent_source,
        "Vespertine Original Mix.wav",
        "Vespertine Original Mix",
        "Salbah",
    );
    generate_tagged_silence_file(
        &recent_source,
        "Calling Black Sharp Remix.wav",
        "Calling Black Sharp Remix",
        "Rene Diehl",
    );
    generate_tagged_silence_file(
        &recent_source,
        "Nlreb Mra Alrrih Playing With The Wind.wav",
        "Nlreb Mra Alrrih Playing With The Wind",
        "Sahale",
    );
    let mut melodic_library = W4djLibrary::open(&melodic_workspace.path().join("w4dj.sqlite3"))
        .expect("open Melodic Techno library");
    assert_eq!(
        convert_fixture_batch(
            &historical_source,
            &melodic_output,
            &mut melodic_library,
            "acceptance-melodic-historical-1",
        )
        .len(),
        2
    );
    assert_eq!(
        convert_fixture_batch(
            &recent_source,
            &melodic_output,
            &mut melodic_library,
            "acceptance-melodic-recent-2",
        )
        .len(),
        4
    );
    melodic_library
        .upsert_imported_dj_playlist(&melodic)
        .expect("persist Melodic Techno playlist");
    let melodic_report = melodic_library
        .compute_imported_dj_playlist_matches(&melodic.playlist_id)
        .expect("match Melodic Techno playlist");
    assert_eq!(melodic_report.matched_count, 6);
    assert_eq!(
        melodic_report
            .matches
            .iter()
            .filter(|row| row.kind == DjPlaylistMatchKind::RecentBm25f)
            .count(),
        4
    );
    assert_eq!(
        melodic_report
            .matches
            .iter()
            .filter(|row| row.kind == DjPlaylistMatchKind::LibraryBm25f)
            .count(),
        2
    );
    assert!(
        melodic_report
            .matches
            .iter()
            .filter(|row| row.kind == DjPlaylistMatchKind::LibraryBm25f)
            .all(|row| row.score.is_some_and(|score| score >= 50))
    );
    melodic_library
        .replace_imported_dj_playlist_matches(&melodic.playlist_id, &melodic_report)
        .expect("persist Melodic Techno matches");
    confirm_all_rows(&mut melodic_library, &melodic);
    assert_complete_m3u8(
        &melodic_library,
        &melodic,
        &melodic_workspace.path().join("exports/melodic-techno.m3u8"),
        6,
    );

    let manual_workspace = tempdir().expect("manual recovery workspace");
    let manual_output = manual_workspace.path().join("outputs");
    let manual_historical_source = manual_workspace.path().join("historical-sources");
    generate_tagged_silence_file(
        &manual_historical_source,
        "Untitled Fixture.wav",
        "Untitled Fixture",
        "Unknown Artist",
    );
    generate_tagged_silence_file(
        &manual_historical_source,
        "Rhea Original Mix.wav",
        "Rhea Original Mix",
        "ANRA",
    );
    let manual_recent_source = manual_workspace.path().join("recent-sources");
    generate_tagged_silence_file(
        &manual_recent_source,
        "Solara Original Mix.wav",
        "Solara Original Mix",
        "Hakan",
    );
    generate_tagged_silence_file(
        &manual_recent_source,
        "Vespertine Original Mix.wav",
        "Vespertine Original Mix",
        "Salbah",
    );
    generate_tagged_silence_file(
        &manual_recent_source,
        "Calling Black Sharp Remix.wav",
        "Calling Black Sharp Remix",
        "Rene Diehl",
    );
    generate_tagged_silence_file(
        &manual_recent_source,
        "Nlreb Mra Alrrih Playing With The Wind.wav",
        "Nlreb Mra Alrrih Playing With The Wind",
        "Sahale",
    );
    let manual_file = manual_workspace
        .path()
        .join("selected/Ipnosi Original Mix.wav");
    generate_tagged_silence_file(
        manual_file.parent().expect("manual file parent"),
        "Ipnosi Original Mix.wav",
        "Ipnosi Original Mix",
        "RIVE",
    );
    let mut manual_library = W4djLibrary::open(&manual_workspace.path().join("w4dj.sqlite3"))
        .expect("open manual recovery library");
    convert_fixture_batch(
        &manual_historical_source,
        &manual_output,
        &mut manual_library,
        "acceptance-manual-historical-1",
    );
    convert_fixture_batch(
        &manual_recent_source,
        &manual_output,
        &mut manual_library,
        "acceptance-manual-recent-2",
    );
    manual_library
        .upsert_imported_dj_playlist(&melodic)
        .expect("persist manual recovery playlist");
    let manual_initial = manual_library
        .compute_imported_dj_playlist_matches(&melodic.playlist_id)
        .expect("match manual recovery playlist");
    assert_eq!(manual_initial.matched_count, 5);
    assert!(manual_initial.matches.iter().any(|row| {
        row.position == 1 && row.status == "unmatched" && row.destination_path.is_none()
    }));
    manual_library
        .replace_imported_dj_playlist_matches(&melodic.playlist_id, &manual_initial)
        .expect("persist manual recovery matches");
    manual_library
        .set_imported_dj_playlist_match_by_path(&melodic.playlist_id, 1, &manual_file)
        .expect("bind manual recovery file");
    let after_manual = manual_library
        .get_imported_dj_playlist_match_report(&melodic.playlist_id)
        .expect("read manual recovery report");
    let manual_row = after_manual
        .matches
        .iter()
        .find(|row| row.position == 1)
        .expect("manual recovery row");
    assert_eq!(manual_row.match_method.as_deref(), Some("manual"));
    assert_eq!(manual_row.score, Some(100));
    assert!(manual_row.confirmed);
    confirm_all_rows(&mut manual_library, &melodic);
    assert_complete_m3u8(
        &manual_library,
        &melodic,
        &fs::canonicalize(manual_workspace.path())
            .expect("canonicalize manual recovery workspace")
            .join("exports/manual-recovery.m3u8"),
        6,
    );
}

fn load_fixture_playlist(path: &Path) -> ImportedDjPlaylist {
    parse_w4dj_playlist(
        &fs::read(path).expect("read frozen W4DJ fixture"),
        Some(path),
    )
    .expect("parse frozen W4DJ fixture")
}

fn write_source_labels(path: &Path, labels: &[(u64, &str, &str)]) {
    let value = labels
        .iter()
        .map(|(position, title, artist_display)| {
            serde_json::json!({
                "position": position,
                "title": title,
                "artist_display": artist_display,
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec(&value).expect("serialize source label map"),
    )
    .expect("write source label map");
}

fn generate_tagged_silence_file(
    directory: &Path,
    file_name: &str,
    title: &str,
    artist: &str,
) -> PathBuf {
    fs::create_dir_all(directory).expect("create tagged silence directory");
    let path = directory.join(file_name);
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=44100:cl=stereo",
            "-t",
            "2",
            "-metadata",
            &format!("title={title}"),
            "-metadata",
            &format!("artist={artist}"),
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&path)
        .status()
        .expect("run FFmpeg for tagged silence");
    assert!(status.success(), "FFmpeg failed for {}", path.display());
    assert!(path.is_file() && fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 0));
    path
}

fn convert_fixture_batch(
    source_directory: &Path,
    output_directory: &Path,
    library: &mut W4djLibrary,
    batch_id: &str,
) -> Vec<DjOutputCandidate> {
    fs::create_dir_all(output_directory).expect("create fixture output directory");
    let preview = build_sync_preview_with_settings_and_netease_observed_with_policy(
        source_directory.to_str().expect("UTF-8 source directory"),
        output_directory.to_str().expect("UTF-8 output directory"),
        Mode::Compat,
        None,
        ConflictStrategy::Skip,
        FilenameRule::TitleArtist,
        NeteaseFilenameFormat::default(),
        FilenameNormalizationPolicy::PreserveSource,
        None,
    )
    .expect("build fixture preview")
    .expect("fixture preview was not cancelled");
    let source_files_by_name = preview
        .candidates
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
    let source_files = source_files_by_name.iter().collect::<HashMap<_, _>>();
    let candidates_by_name = preview
        .candidates
        .iter()
        .map(|candidate| (candidate.name.clone(), candidate.clone()))
        .collect::<HashMap<_, _>>();
    let task = TaskController::running(source_files.len());
    let mut failures = Vec::new();
    let snapshot =
        sync_music_library_transactional_with_observer_and_budget_and_context_with_policy(
            &source_files,
            output_directory.to_str().expect("UTF-8 output directory"),
            &Mode::Compat,
            None,
            NeteaseFilenameFormat::default(),
            FilenameNormalizationPolicy::PreserveSource,
            &task,
            |_name, _temporary| Ok(()),
            |name, _task, error| {
                if let Some(error) = error {
                    failures.push(format!("{name}: {error}"));
                    return Ok(());
                }
                let candidate = candidates_by_name
                    .get(name)
                    .ok_or_else(|| io::Error::other(format!("unknown fixture source: {name}")))?;
                let destination = Path::new(&candidate.destination_path);
                let embedded = read_embedded_track_metadata(destination);
                let title = if embedded.title.trim().is_empty() {
                    candidate.name.as_str()
                } else {
                    embedded.title.as_str()
                };
                library
                    .upsert_committed_output_in_root(
                        0,
                        output_directory,
                        Some(Path::new(&candidate.source_path)),
                        destination,
                        title,
                        embedded.artist.as_str(),
                        &CommittedOutputFacts {
                            conversion_batch_id: Some(batch_id.to_string()),
                            conversion_mode: Some("compat".to_string()),
                            filename_rule: Some("title_artist".to_string()),
                            filename_normalization_policy: Some("preserve_source".to_string()),
                            ..Default::default()
                        },
                    )
                    .map_err(|error| io::Error::other(error.to_string()))?;
                Ok(())
            },
            Arc::new(GlobalConcurrencyBudget::new(2)),
            Arc::new(ActiveFfmpegRegistry::new()),
            &ConversionMetadataContext::default(),
        )
        .expect("convert fixture batch");
    assert!(
        failures.is_empty(),
        "fixture conversion failures: {failures:?}"
    );
    assert_eq!(snapshot.completed, preview.candidates.len());
    let candidates = library
        .dj_output_candidates_for_batch(batch_id)
        .expect("read fixture batch candidates");
    assert_eq!(candidates.len(), preview.candidates.len());
    assert!(candidates.iter().all(|candidate| {
        candidate.destination_path.is_file()
            && fs::metadata(&candidate.destination_path).is_ok_and(|metadata| metadata.len() > 0)
    }));
    candidates
}

fn confirm_all_rows(library: &mut W4djLibrary, playlist: &ImportedDjPlaylist) {
    for position in playlist.tracks.iter().map(|track| track.position) {
        library
            .set_imported_dj_playlist_match_confirmed(&playlist.playlist_id, position, true)
            .expect("confirm fixture row");
    }
}

fn assert_complete_m3u8(
    library: &W4djLibrary,
    playlist: &ImportedDjPlaylist,
    m3u8_path: &Path,
    expected_entries: usize,
) {
    let report = library
        .get_imported_dj_playlist_match_report(&playlist.playlist_id)
        .expect("read complete fixture report");
    assert_eq!(report.total, expected_entries);
    assert_eq!(report.matched_count, expected_entries);
    assert!(
        report
            .matches
            .iter()
            .all(|row| row.status == "matched" && row.confirmed)
    );
    let candidates = library
        .available_dj_output_candidates()
        .expect("read complete fixture candidates");
    let candidates_by_key = candidates
        .iter()
        .map(|candidate| (candidate.track_key.as_str(), candidate))
        .collect::<HashMap<_, _>>();
    let resolved = report
        .matches
        .iter()
        .map(|row| {
            let track_key = row
                .track_key
                .as_deref()
                .expect("complete fixture track key");
            let candidate = candidates_by_key
                .get(track_key)
                .expect("complete fixture candidate");
            ResolvedDjPlaylistTrack {
                position: row.position,
                title: row.title.clone(),
                artist_display: row.artist_display.clone(),
                duration_seconds: candidate.duration_seconds,
                destination_path: row
                    .destination_path
                    .clone()
                    .expect("complete fixture destination"),
            }
        })
        .collect::<Vec<_>>();
    fs::create_dir_all(m3u8_path.parent().expect("M3U8 parent"))
        .expect("create fixture M3U8 directory");
    let (contents, summary) = build_relative_m3u8_with_summary(playlist, &resolved, m3u8_path)
        .expect("build complete fixture M3U8");
    assert_eq!(summary.matched_count, expected_entries);
    assert_eq!(summary.total, expected_entries);
    assert!(summary.omitted.is_empty());
    assert_eq!(contents.matches("#EXTINF:").count(), expected_entries);
    write_relative_m3u8_atomic(m3u8_path, &contents).expect("write complete fixture M3U8");
    let entries = contents
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), expected_entries);
    assert!(
        entries.iter().all(|entry| {
            m3u8_path
                .parent()
                .expect("M3U8 parent")
                .join(entry)
                .is_file()
        }),
        "M3U8 entries do not resolve: {entries:?}\n{contents}"
    );
}

fn generate_silence_sources(
    playlist_path: &Path,
    source_directory: &Path,
    source_labels_path: Option<&Path>,
) {
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/generate-w4dj-silence-fixtures.sh");
    let mut command = Command::new(&script);
    command.arg(playlist_path).arg(source_directory);
    if let Some(source_labels_path) = source_labels_path {
        command.arg(source_labels_path);
    }
    let status = command.status().expect("run silence fixture generator");
    assert!(
        status.success(),
        "silence fixture generator failed: {}",
        script.display()
    );
}
