//! Read-only NetEase import and normalization for the W4DJ library index.

use crate::library_catalog::{
    CatalogLocalFile, CatalogSnapshot, CatalogSource, CatalogSourceRecord, CatalogTrack,
    LibraryCatalog, LocalStatus,
};
use crate::media_probe::probe_local_audio;
use crate::netease::{NeteaseRecord, choose_record, database_candidates, load_records_from_db};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseDiscovery {
    pub database_path: Option<PathBuf>,
    pub music_folder: Option<PathBuf>,
    pub record_count: usize,
    pub local_file_count: usize,
}

pub fn discover_netease_library() -> NeteaseDiscovery {
    let mut best: Option<(PathBuf, usize, Vec<NeteaseRecord>)> = None;
    for path in database_candidates() {
        let Ok(records) = load_records_from_db(&path) else {
            continue;
        };
        if records.len() > best.as_ref().map_or(0, |(_, count, _)| *count) {
            best = Some((path, records.len(), records));
        }
    }

    let Some((database_path, record_count, best_records)) = best else {
        return NeteaseDiscovery {
            database_path: None,
            music_folder: None,
            record_count: 0,
            local_file_count: 0,
        };
    };

    let music_folder = candidate_music_folder(&database_path, &best_records);
    let local_file_count = music_folder
        .as_deref()
        .map(count_audio_files)
        .unwrap_or_default();
    NeteaseDiscovery {
        database_path: Some(database_path),
        music_folder,
        record_count,
        local_file_count,
    }
}

pub fn build_catalog_snapshot(
    database_path: &Path,
    music_folder: Option<&Path>,
) -> Result<CatalogSnapshot, String> {
    build_catalog_snapshot_incremental(database_path, music_folder, None)
}

pub fn build_catalog_snapshot_incremental(
    database_path: &Path,
    music_folder: Option<&Path>,
    catalog: Option<&LibraryCatalog>,
) -> Result<CatalogSnapshot, String> {
    let records = load_records_from_db(database_path)
        .map_err(|error| format!("无法只读打开网易云数据库：{error}"))?;
    let local_paths = music_folder
        .filter(|path| path.is_dir())
        .map(collect_audio_files)
        .unwrap_or_default();

    let mut matched_paths: HashMap<String, PathBuf> = HashMap::new();
    for path in &local_paths {
        if let Some(record) = choose_record(path, &records) {
            matched_paths
                .entry(record_key(record))
                .or_insert_with(|| path.clone());
        }
    }

    let local_files = local_paths
        .iter()
        .filter_map(|path| local_file_for_path_cached(path, &records, catalog))
        .collect::<Vec<_>>();

    let mut tracks = Vec::new();
    let mut source_records = Vec::new();
    let mut seen = HashSet::new();
    for record in &records {
        let key = record_key(record);
        if !seen.insert(key.clone()) {
            continue;
        }
        let local_path = matched_paths.get(&key);
        tracks.push(track_from_record(record, local_path));
        source_records.push(source_record_from_record(record, &key));
    }

    for path in &local_paths {
        if records
            .iter()
            .any(|record| choose_record(path, std::slice::from_ref(record)).is_some())
        {
            continue;
        }
        let key = format!("source:local:{}", path.to_string_lossy());
        if seen.insert(key.clone()) {
            tracks.push(local_only_track(&key, path));
        }
    }

    attach_local_measurements(&mut tracks, &local_files);

    let metadata = fs::metadata(database_path)
        .map_err(|error| format!("无法读取网易云数据库文件信息：{error}"))?;
    let modified_at_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64);
    Ok(CatalogSnapshot {
        tracks,
        local_files,
        source_records,
        sources: vec![CatalogSource {
            database_path: database_path.to_path_buf(),
            database_size_bytes: metadata.len().min(i64::MAX as u64) as i64,
            database_modified_at_ms: modified_at_ms,
            last_imported_at_ms: now_ms(),
            import_status: "success".to_string(),
            last_error: None,
        }],
    })
}

fn candidate_music_folder(database_path: &Path, records: &[NeteaseRecord]) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("W4DJ_NETEASE_MUSIC_DIR").map(PathBuf::from)
        && path.is_dir()
    {
        return Some(path);
    }

    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let known = [
        home.join("Music/网易云音乐"),
        home.join("Music/NetEase CloudMusic"),
        home.join("Music/Netease Cloud Music"),
    ];
    if let Some(path) = known.into_iter().find(|path| path.is_dir()) {
        return Some(path);
    }

    // A database copied from another macOS account often retains absolute
    // paths such as /Users/mac/Music/网易云音乐. Reuse that directory when it
    // is present instead of assuming the current OS account owns the library.
    records
        .iter()
        .filter_map(|record| {
            let path = Path::new(&record.path);
            let components = path.components().collect::<Vec<_>>();
            components
                .iter()
                .position(|component| component.as_os_str() == "网易云音乐")
                .map(|index| components[..=index].iter().collect::<PathBuf>())
        })
        .find(|path| path.is_dir())
        .or_else(|| {
            database_path
                .parent()
                .map(Path::to_path_buf)
                .filter(|path| path.is_dir())
        })
}

fn count_audio_files(root: &Path) -> usize {
    collect_audio_files(root).len()
}

fn collect_audio_files(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| is_audio_file(path))
        .collect()
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mp3" | "flac" | "wav" | "aif" | "aiff" | "ncm"
            )
        })
        .unwrap_or(false)
}

fn record_key(record: &NeteaseRecord) -> String {
    if !record.track_id.trim().is_empty() {
        format!("netease:{}", record.track_id.trim())
    } else if !record.path.trim().is_empty() {
        format!("source:netease:path:{}", record.path.trim())
    } else {
        format!("source:netease:file:{}", record.file_name.trim())
    }
}

fn track_from_record(record: &NeteaseRecord, local_path: Option<&PathBuf>) -> CatalogTrack {
    let mut track = CatalogTrack {
        track_key: record_key(record),
        netease_track_id: nonempty(record.track_id.clone()),
        title: record.title.clone(),
        artists: record.artist.clone(),
        artist_list_json: artist_list_json(&record.artist),
        album: record.album.clone(),
        netease_genre: record.genre.clone(),
        aliases_json: record.aliases_json.clone(),
        copyright_text: record.copyright_text.clone(),
        publish_date: record.publish_date.clone(),
        lyric_plain_text: record.lyric_plain_text.clone(),
        lyric_translated_text: record.lyric_translated_text.clone(),
        lyric_romanized_text: record.lyric_romanized_text.clone(),
        lyric_lrc_text: record.lyric_lrc_text.clone(),
        lyric_language: record.lyric_language.clone(),
        lyric_sync_type: record.lyric_sync_type.clone(),
        lyric_source: record.lyric_source.clone(),
        cover_path: nonempty(record.cover_path.clone()),
        cover_available: record.cover_data.is_some()
            || record.cover_path.trim() != ""
            || !record.cover_references.is_empty(),
        local_status: local_path
            .map(|_| LocalStatus::Available)
            .unwrap_or(LocalStatus::DatabaseOnly),
        db_duration_seconds: record.duration_ms.map(|value| value as f64 / 1000.0),
        db_size_bytes: record
            .size_bytes
            .and_then(|value| i64::try_from(value).ok()),
        db_format: Path::new(&record.file_name)
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase()),
        ..CatalogTrack::default()
    };
    track.effective_duration_seconds = track.db_duration_seconds;
    track.duration_source = track
        .db_duration_seconds
        .map(|_| crate::library_catalog::DurationSource::Netease);
    track.effective_format = track.db_format.clone();
    track.effective_size_bytes = track.db_size_bytes;
    track
}

fn local_only_track(key: &str, path: &Path) -> CatalogTrack {
    let mut track = CatalogTrack {
        track_key: key.to_string(),
        title: path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        local_status: LocalStatus::Available,
        measured_format: path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase()),
        effective_format: path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase()),
        ..CatalogTrack::default()
    };
    track.updated_at_ms = now_ms();
    track
}

fn local_file_for_path_cached(
    path: &Path,
    records: &[NeteaseRecord],
    catalog: Option<&LibraryCatalog>,
) -> Option<CatalogLocalFile> {
    let metadata = fs::metadata(path).ok()?;
    let record = records
        .iter()
        .find(|record| choose_record(path, std::slice::from_ref(record)).is_some());
    let track_key = record
        .map(record_key)
        .unwrap_or_else(|| format!("source:local:{}", path.to_string_lossy()));
    let modified_at_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64);
    let base = CatalogLocalFile {
        id: None,
        track_key,
        path: path.to_path_buf(),
        size_bytes: metadata.len().min(i64::MAX as u64) as i64,
        modified_at_ms,
        measured_format: path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase()),
        measured_bitrate_bps: None,
        measured_duration_seconds: None,
        sample_rate_hz: None,
        channels: None,
        readable: true,
        probe_error: None,
    };
    if let Some(previous) = catalog
        .and_then(|catalog| catalog.local_file_by_path(path).ok().flatten())
        .filter(|previous| {
            previous.size_bytes == base.size_bytes && previous.modified_at_ms == base.modified_at_ms
        })
    {
        return Some(previous);
    }

    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("ncm"))
    {
        return Some(base);
    }

    match probe_local_audio(path) {
        Ok(facts) => Some(CatalogLocalFile {
            measured_format: Some(facts.format),
            measured_bitrate_bps: facts.average_bitrate_bps,
            measured_duration_seconds: facts.duration_seconds,
            sample_rate_hz: facts.sample_rate_hz,
            channels: facts.channels,
            ..base
        }),
        Err(error) => Some(CatalogLocalFile {
            measured_format: None,
            readable: false,
            probe_error: Some(error.to_string()),
            ..base
        }),
    }
}

fn attach_local_measurements(tracks: &mut [CatalogTrack], local_files: &[CatalogLocalFile]) {
    let mut by_track: HashMap<String, Vec<&CatalogLocalFile>> = HashMap::new();
    for local_file in local_files {
        by_track
            .entry(local_file.track_key.clone())
            .or_default()
            .push(local_file);
    }
    for track in tracks {
        let Some(files) = by_track.get(&track.track_key) else {
            continue;
        };
        let preferred = files
            .iter()
            .copied()
            .find(|file| file.readable)
            .or_else(|| files.first().copied());
        let Some(file) = preferred else {
            continue;
        };
        track.local_status = if file.readable {
            LocalStatus::Available
        } else {
            LocalStatus::Unreadable
        };
        track.measured_format = file.measured_format.clone();
        track.measured_bitrate_bps = file.measured_bitrate_bps;
        track.measured_duration_seconds = file.measured_duration_seconds;
        track.measured_size_bytes = Some(file.size_bytes);
        track.effective_format = file.measured_format.clone().or(track.db_format.clone());
        track.effective_bitrate_bps = file.measured_bitrate_bps.or(track.db_bitrate_bps);
        track.effective_size_bytes = Some(file.size_bytes);
        let (duration, source) = crate::library_catalog::effective_duration(
            track.essentia_duration_seconds,
            file.measured_duration_seconds,
            track.db_duration_seconds,
        );
        track.effective_duration_seconds = duration;
        track.duration_source = source;
    }
}

fn source_record_from_record(record: &NeteaseRecord, key: &str) -> CatalogSourceRecord {
    CatalogSourceRecord {
        track_key: key.to_string(),
        source_table: "netease".to_string(),
        source_primary_key: nonempty(record.track_id.clone())
            .or_else(|| nonempty(record.path.clone()))
            .unwrap_or_else(|| record.file_name.clone()),
        source_version: Some("local-sqlite-v1".to_string()),
        raw_json: if record.raw_json.trim().is_empty() {
            serde_json::json!({
                "title": record.title,
                "artist": record.artist,
                "album": record.album,
                "trackId": record.track_id,
                "albumId": record.album_id,
            })
            .to_string()
        } else {
            record.raw_json.clone()
        },
        imported_at_ms: now_ms(),
    }
}

fn artist_list_json(artists: &str) -> String {
    serde_json::to_string(
        &artists
            .split(",")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string())
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}
