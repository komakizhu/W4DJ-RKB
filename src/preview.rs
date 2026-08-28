use crate::analysis::read_track_metadata;
use crate::concurrency::GlobalConcurrencyBudget;
use crate::config::{
    CandidateOperation, ConflictStrategy, FilenameNormalizationPolicy, FilenameRule,
    LosslessFormat, Mode, NeteaseFilenameFormat,
};
use crate::filename_policy::sanitize_filename_component;
use crate::history::HistoryEntry;
use crate::netease::NeteaseMetadataResolver;
use crate::scan_cache::ScanCache;
use crate::sync::{
    ScanObserver, derive_song_name_with_policy_and_resolver_cancellable,
    effective_source_extension, find_ffmpeg, get_destination_music_dict_with_rule,
    get_destination_music_dict_with_rule_and_observer,
    get_music_dict_with_scan_issues_with_settings_and_cache_observer_with_budget_and_policy,
    get_music_dict_with_scan_issues_with_settings_and_cache_observer_with_policy,
    get_music_dict_with_scan_issues_with_settings_and_observer_with_budget_and_policy,
    get_music_dict_with_scan_issues_with_settings_and_observer_with_policy,
    get_music_dict_with_scan_issues_with_settings_and_policy, is_ignored_music_file,
    is_supported_source_file, resolve_output_policy, source_entry_name,
    target_output_path_with_policy,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};

pub use crate::config::CandidateOperation as PreviewOperation;

/// Recover a single-file source after the original downloaded file was
/// replaced in the same directory, for example `Track.ncm` -> `Track.mp3`.
/// Prefer an exact stem match. If the downloaded replacement also changed its
/// filename, accept it only when it is the *only* supported audio file left in
/// the parent directory. An ambiguous directory is left unresolved so the
/// caller cannot convert the wrong track silently.
pub fn resolve_missing_single_source_path(source: &Path) -> Option<std::path::PathBuf> {
    if source.exists() || source.extension().is_none() {
        return None;
    }

    let parent = source.parent()?;
    let source_stem = source.file_stem()?.to_string_lossy();
    let files = fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_supported_source_file(path))
        .collect::<Vec<_>>();

    let mut exact_matches = files
        .iter()
        .filter(|path| {
            path.file_stem()
                .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(&source_stem))
        })
        .cloned()
        .collect::<Vec<_>>();
    if exact_matches.len() == 1 {
        return exact_matches.pop();
    }

    if files.len() == 1 {
        files.into_iter().next()
    } else {
        None
    }
}

pub fn is_recovered_single_source(original: &str, resolved: &str) -> bool {
    resolve_missing_single_source_path(Path::new(original))
        .is_some_and(|candidate| candidate == Path::new(resolved))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewCandidate {
    pub name: String,
    pub source_path: String,
    pub destination_path: String,
    pub source_size_bytes: u64,
    pub estimated_output_bytes: Option<u64>,
    /// Existing output to remove after a successful overwrite when the
    /// selected filename rule produces a new destination path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_destination_path: Option<String>,
    #[serde(default)]
    pub operation: CandidateOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub netease_track_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub netease_album_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub netease_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub netease_artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disambiguation_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputTrackIdentity {
    pub track_id: Option<String>,
    pub album_id: Option<String>,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub source_path: PathBuf,
}

impl OutputTrackIdentity {
    /// Stable identity used to decide whether two same-name candidates are
    /// actually the same track.  NetEase track IDs take precedence; for
    /// files without a database match, the normalized metadata and source
    /// path keep the fallback deterministic without affecting ordinary names.
    pub fn stable_key(&self) -> String {
        if let Some(track_id) = self
            .track_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return format!("track:{track_id}");
        }
        let normalize = |value: &str| value.trim().to_ascii_lowercase();
        format!(
            "fallback:{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            normalize(&self.title),
            normalize(&self.artists),
            normalize(&self.album),
            normalize(self.album_id.as_deref().unwrap_or_default()),
            self.source_path.display()
        )
    }

    fn same_metadata(&self, metadata: &crate::analysis::TrackMetadata) -> bool {
        let title_matches = !self.title.trim().is_empty()
            && self
                .title
                .trim()
                .eq_ignore_ascii_case(metadata.title.trim());
        let artists_matches = !self.artists.trim().is_empty()
            && self
                .artists
                .trim()
                .eq_ignore_ascii_case(metadata.artist.trim());
        let album_matches = !self.album.trim().is_empty()
            && self
                .album
                .trim()
                .eq_ignore_ascii_case(metadata.album.trim());
        title_matches && artists_matches && (album_matches || self.album.trim().is_empty())
    }
}

/// Resolve only true output-name collisions. A candidate without a collision
/// is left byte-for-byte unchanged, including its filename and destination.
pub fn disambiguate_duplicate_output_names(
    preview: &mut SyncPreview,
    identities: &HashMap<String, OutputTrackIdentity>,
) {
    let mut groups = HashMap::<String, Vec<usize>>::new();
    for (index, candidate) in preview.candidates.iter().enumerate() {
        groups
            .entry(candidate.destination_path.clone())
            .or_default()
            .push(index);
    }

    let mut removals = HashSet::new();
    for indices in groups.into_values().filter(|indices| indices.len() > 1) {
        let base_path = PathBuf::from(&preview.candidates[indices[0]].destination_path);
        let existing_metadata = base_path.is_file().then(|| read_track_metadata(&base_path));
        let existing_index = existing_metadata.as_ref().and_then(|metadata| {
            indices.iter().copied().find(|index| {
                identities
                    .get(&preview.candidates[*index].source_path)
                    .is_some_and(|identity| identity.same_metadata(metadata))
            })
        });

        let distinct_albums = indices
            .iter()
            .filter_map(|index| identities.get(&preview.candidates[*index].source_path))
            .map(|identity| identity.album.trim())
            .filter(|album| !album.is_empty())
            .collect::<HashSet<_>>()
            .len()
            > 1;
        let distinct_track_ids = indices
            .iter()
            .filter_map(|index| {
                identities
                    .get(&preview.candidates[*index].source_path)
                    .and_then(|identity| identity.track_id.as_deref())
            })
            .collect::<HashSet<_>>()
            .len()
            > 1;

        let mut ordered = indices.clone();
        ordered.sort_by(|left, right| {
            preview.candidates[*left]
                .source_path
                .cmp(&preview.candidates[*right].source_path)
        });
        let mut occupied = HashSet::<PathBuf>::new();
        for index in ordered {
            if Some(index) == existing_index {
                removals.insert(index);
                continue;
            }
            let candidate = &mut preview.candidates[index];
            let identity = identities.get(&candidate.source_path);
            let suffix = if distinct_albums {
                identity
                    .map(|identity| sanitize_filename_component(&identity.album))
                    .filter(|album| !album.is_empty())
            } else if distinct_track_ids {
                identity.and_then(|identity| identity.track_id.clone())
            } else {
                None
            };
            let base_name = candidate.name.clone();
            let mut name = suffix
                .filter(|suffix| !suffix.is_empty())
                .map(|suffix| format!("{base_name} [{suffix}]"))
                .unwrap_or_else(|| base_name.clone());
            let extension = Path::new(&candidate.destination_path)
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default();
            let mut destination = target_output_path_with_policy(
                &preview.destination_directory,
                &name,
                extension,
                FilenameNormalizationPolicy::PreserveSource,
            );
            let mut ordinal = 2usize;
            while destination.exists() || occupied.contains(&destination) {
                name = format!("{base_name} ({ordinal})");
                destination = target_output_path_with_policy(
                    &preview.destination_directory,
                    &name,
                    extension,
                    FilenameNormalizationPolicy::PreserveSource,
                );
                ordinal += 1;
            }
            candidate.name = name;
            candidate.destination_path = destination.display().to_string();
            candidate.disambiguation_reason = Some(if distinct_albums {
                "同名歌曲，已按专辑区分".to_string()
            } else if distinct_track_ids {
                "同名歌曲，已按网易云歌曲 ID 区分".to_string()
            } else {
                "同名歌曲，已按稳定序号区分".to_string()
            });
            occupied.insert(destination);
        }
    }

    if !removals.is_empty() {
        let mut retained = Vec::with_capacity(preview.candidates.len() - removals.len());
        for (index, candidate) in std::mem::take(&mut preview.candidates)
            .into_iter()
            .enumerate()
        {
            if removals.contains(&index) {
                preview.existing_count += 1;
                preview.skipped_count += 1;
                preview.new_count = preview.new_count.saturating_sub(1);
                if let Some(removed_bytes) = candidate.estimated_output_bytes {
                    preview.estimated_output_bytes = preview
                        .estimated_output_bytes
                        .and_then(|total| total.checked_sub(removed_bytes));
                }
            } else {
                retained.push(candidate);
            }
        }
        preview.candidates = retained;
    }
}

/// Attach the immutable NetEase identity to candidates after scanning.  This
/// is intentionally metadata-only: the final path selected by the collision
/// pass is never changed here, and the identity is not reconstructed from a
/// disambiguation suffix.
pub fn attach_netease_identities(preview: &mut SyncPreview, resolver: &NeteaseMetadataResolver) {
    for candidate in &mut preview.candidates {
        if let Some(identity) =
            resolver.track_identity_for_preview(Path::new(&candidate.source_path))
        {
            candidate.netease_track_id = identity.track_id;
            candidate.netease_album_id = identity.album_id;
            if !identity.title.trim().is_empty() {
                candidate.netease_title = Some(identity.title);
            }
            if !identity.artists.trim().is_empty() {
                candidate.netease_artist = Some(identity.artists);
            }
            if !identity.album.trim().is_empty() {
                candidate.album = Some(identity.album);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncPreview {
    pub source_directory: String,
    pub destination_directory: String,
    pub new_count: usize,
    pub existing_count: usize,
    pub skipped_count: usize,
    pub error_count: usize,
    pub estimated_output_bytes: Option<u64>,
    pub candidates: Vec<PreviewCandidate>,
    pub skipped: Vec<PreviewIssue>,
    pub errors: Vec<PreviewIssue>,
    pub warnings: Vec<PreviewIssue>,
    #[serde(default)]
    pub available_space_bytes: Option<u64>,
    #[serde(default)]
    pub disk_space_sufficient: Option<bool>,
    /// Total supported input tracks represented by this preview. This is
    /// intentionally independent from the destination duplicate count.
    #[serde(default)]
    pub input_count: usize,
    /// Number of input tracks whose resolved output already existed before
    /// this preview was built.
    #[serde(default)]
    pub output_duplicate_count: usize,
    /// Primary action represented by `action_count` (skip/overwrite/
    /// update_metadata/rename).
    #[serde(default)]
    pub action_kind: String,
    #[serde(default)]
    pub action_count: usize,
    /// Database path used for this immutable preview snapshot.
    #[serde(default)]
    pub database_directory: Option<String>,
    /// One row per input/issue, rendered lazily by the confirmation UI.
    #[serde(default)]
    pub detail_items: Vec<PreviewDetailItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewDetailItem {
    pub name: String,
    pub source_path: String,
    #[serde(default)]
    pub destination_path: Option<String>,
    #[serde(default)]
    pub existing_output: bool,
    /// new | duplicate | skip | overwrite | update_metadata | rename | error
    pub classification: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlotPreview {
    pub slot_index: usize,
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
    #[serde(default)]
    pub conflict_strategy: ConflictStrategy,
    #[serde(default)]
    pub filename_rule: FilenameRule,
    #[serde(default)]
    pub netease_filename_format: NeteaseFilenameFormat,
    pub preview: SyncPreview,
    pub retry_of: Option<String>,
}

pub fn build_sync_preview(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
) -> io::Result<SyncPreview> {
    build_sync_preview_with_settings(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        ConflictStrategy::default(),
        FilenameRule::default(),
    )
}

pub fn build_sync_preview_with_settings(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
) -> io::Result<SyncPreview> {
    build_sync_preview_with_settings_and_netease(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        NeteaseFilenameFormat::default(),
    )
}

pub fn build_sync_preview_with_settings_and_netease(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
) -> io::Result<SyncPreview> {
    build_sync_preview_with_settings_and_netease_observed_with_policy(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
        None,
    )?
    .ok_or_else(|| io::Error::other("扫描被取消"))
}

pub fn build_sync_preview_with_settings_observed(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    observer: Option<&mut ScanObserver<'_>>,
) -> io::Result<Option<SyncPreview>> {
    build_sync_preview_with_settings_and_netease_observed(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        NeteaseFilenameFormat::default(),
        observer,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_sync_preview_with_settings_and_netease_observed(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    observer: Option<&mut ScanObserver<'_>>,
) -> io::Result<Option<SyncPreview>> {
    build_sync_preview_with_settings_and_netease_observed_with_policy(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
        observer,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_sync_preview_with_settings_and_netease_observed_with_policy(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    observer: Option<&mut ScanObserver<'_>>,
) -> io::Result<Option<SyncPreview>> {
    build_sync_preview_with_settings_and_netease_observed_internal(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        filename_policy,
        observer,
        None,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_sync_preview_with_settings_and_netease_observed_with_cache(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    observer: Option<&mut ScanObserver<'_>>,
    cache: &mut ScanCache,
) -> io::Result<Option<SyncPreview>> {
    build_sync_preview_with_settings_and_netease_observed_with_cache_and_policy(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
        observer,
        cache,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_sync_preview_with_settings_and_netease_observed_with_cache_and_policy(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    observer: Option<&mut ScanObserver<'_>>,
    cache: &mut ScanCache,
) -> io::Result<Option<SyncPreview>> {
    build_sync_preview_with_settings_and_netease_observed_internal(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        filename_policy,
        observer,
        Some((cache, Path::new(source_directory))),
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_sync_preview_with_settings_and_netease_observed_with_cache_and_budget(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    observer: Option<&mut ScanObserver<'_>>,
    cache: &mut ScanCache,
    budget: Arc<GlobalConcurrencyBudget>,
    cancel: Arc<AtomicBool>,
) -> io::Result<Option<SyncPreview>> {
    build_sync_preview_with_settings_and_netease_observed_with_cache_and_budget_and_policy(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
        observer,
        cache,
        budget,
        cancel,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_sync_preview_with_settings_and_netease_observed_with_cache_and_budget_and_policy(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    observer: Option<&mut ScanObserver<'_>>,
    cache: &mut ScanCache,
    budget: Arc<GlobalConcurrencyBudget>,
    cancel: Arc<AtomicBool>,
) -> io::Result<Option<SyncPreview>> {
    build_sync_preview_with_settings_and_netease_observed_internal(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        filename_policy,
        observer,
        Some((cache, Path::new(source_directory))),
        Some((budget, cancel)),
        None,
    )
}

/// Task 1 preview variant. It keeps the existing scanning APIs intact while
/// allowing the coordinator to apply a read-only NetEase identity before it
/// calculates expected paths and conflict groups.
#[allow(clippy::too_many_arguments)]
pub fn build_sync_preview_with_settings_and_netease_observed_with_policy_and_resolver(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    observer: Option<&mut ScanObserver<'_>>,
    resolver: &NeteaseMetadataResolver,
) -> io::Result<Option<SyncPreview>> {
    build_sync_preview_with_settings_and_netease_observed_internal(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        filename_policy,
        observer,
        None,
        None,
        Some(resolver),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_sync_preview_with_settings_and_netease_observed_with_cache_and_budget_and_policy_and_resolver(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    observer: Option<&mut ScanObserver<'_>>,
    cache: &mut ScanCache,
    budget: Arc<GlobalConcurrencyBudget>,
    cancel: Arc<AtomicBool>,
    resolver: &NeteaseMetadataResolver,
) -> io::Result<Option<SyncPreview>> {
    build_sync_preview_with_settings_and_netease_observed_internal(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        filename_policy,
        observer,
        Some((cache, Path::new(source_directory))),
        Some((budget, cancel)),
        Some(resolver),
    )
}

#[allow(clippy::too_many_arguments)]
fn build_sync_preview_with_settings_and_netease_observed_internal(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    mut observer: Option<&mut ScanObserver<'_>>,
    scan_cache: Option<(&mut ScanCache, &Path)>,
    budget: Option<(Arc<GlobalConcurrencyBudget>, Arc<AtomicBool>)>,
    metadata_resolver: Option<&NeteaseMetadataResolver>,
) -> io::Result<Option<SyncPreview>> {
    let mut preview = SyncPreview {
        source_directory: source_directory.to_string(),
        destination_directory: destination_directory.to_string(),
        new_count: 0,
        existing_count: 0,
        skipped_count: 0,
        error_count: 0,
        estimated_output_bytes: Some(0),
        candidates: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
        available_space_bytes: None,
        disk_space_sufficient: None,
        input_count: 0,
        output_duplicate_count: 0,
        action_kind: String::new(),
        action_count: 0,
        database_directory: metadata_resolver
            .and_then(|resolver| resolver.database_path())
            .map(|path| path.display().to_string()),
        detail_items: Vec::new(),
    };

    let source_path = Path::new(source_directory);
    let resolved_source_path = resolve_missing_single_source_path(source_path);
    let effective_source_directory = resolved_source_path
        .as_deref()
        .unwrap_or(source_path)
        .to_string_lossy()
        .into_owned();
    preview.source_directory = effective_source_directory.clone();
    let source_path = Path::new(&effective_source_directory);
    if !source_path.exists() {
        preview.warnings.push(PreviewIssue {
            path: source_directory.to_string(),
            message: "输入来源不存在或不可读取".to_string(),
        });
        preview.estimated_output_bytes = None;
        return Ok(Some(preview));
    }

    if source_path.is_file() && !is_supported_source_file(source_path) {
        preview.errors.push(PreviewIssue {
            path: source_directory.to_string(),
            message: "不支持的单曲格式；请选择 MP3、FLAC、NCM、WAV 或 AIFF 文件".to_string(),
        });
        preview.error_count = 1;
        preview.estimated_output_bytes = None;
        return Ok(Some(preview));
    }

    if !source_path.is_dir() && !source_path.is_file() {
        preview.warnings.push(PreviewIssue {
            path: source_directory.to_string(),
            message: "输入来源不是文件夹或音频文件".to_string(),
        });
        preview.estimated_output_bytes = None;
        return Ok(Some(preview));
    }

    if !destination_directory.trim().is_empty() {
        let destination_path = Path::new(destination_directory);
        if destination_path.exists() && !destination_path.is_dir() {
            preview.warnings.push(PreviewIssue {
                path: destination_directory.to_string(),
                message: "输出路径不是文件夹".to_string(),
            });
        } else if !destination_path.exists()
            && destination_path
                .parent()
                .is_some_and(|parent| !parent.exists())
        {
            preview.warnings.push(PreviewIssue {
                path: destination_directory.to_string(),
                message: "输出目录及其父目录不存在".to_string(),
            });
        }
    }

    let (mut source_files, scan_issues, cancelled) = match scan_cache {
        Some((cache, _source_root)) => {
            let mut no_op_observer = |_: crate::sync::ScanPhase, _: &Path| true;
            let scan_observer = observer.as_deref_mut().unwrap_or(&mut no_op_observer);
            if let Some((budget, cancel)) = budget.as_ref() {
                get_music_dict_with_scan_issues_with_settings_and_cache_observer_with_budget_and_policy(
                    &effective_source_directory,
                    filename_rule,
                    netease_filename_format,
                    filename_policy,
                    Path::new(destination_directory),
                    cache,
                    Arc::clone(budget),
                    Arc::clone(cancel),
                    scan_observer,
                )
            } else {
                get_music_dict_with_scan_issues_with_settings_and_cache_observer_with_policy(
                    &effective_source_directory,
                    filename_rule,
                    netease_filename_format,
                    filename_policy,
                    Path::new(destination_directory),
                    cache,
                    scan_observer,
                )
            }
        }
        None => {
            if let Some(observer) = observer.as_deref_mut() {
                get_music_dict_with_scan_issues_with_settings_and_observer_with_policy(
                    &effective_source_directory,
                    filename_rule,
                    netease_filename_format,
                    filename_policy,
                    observer,
                )
            } else {
                let (files, issues) = get_music_dict_with_scan_issues_with_settings_and_policy(
                    &effective_source_directory,
                    filename_rule,
                    netease_filename_format,
                    filename_policy,
                );
                (files, issues, false)
            }
        }
    };
    if cancelled {
        return Ok(None);
    }

    // The scanner/cache intentionally remains resolver-agnostic so ordinary
    // Task 2 folders keep their historical behavior. Task 1 applies the
    // matched database identity here, before any expected path, collision or
    // conflict decision is made.
    if matches!(filename_policy, FilenameNormalizationPolicy::PreserveSource)
        && !matches!(filename_rule, FilenameRule::Original)
        && let Some(resolver) = metadata_resolver
    {
        let scan_cancel = budget.as_ref().map(|(_, cancel)| cancel.as_ref());
        let mut resolved = HashMap::with_capacity(source_files.len());
        for (_, (size, path)) in source_files.into_iter() {
            if scan_cancel.is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::SeqCst)) {
                return Ok(None);
            }
            if let Some(observer) = observer.as_deref_mut()
                && !observer(crate::sync::ScanPhase::Metadata, &path)
            {
                return Ok(None);
            }
            let name = derive_song_name_with_policy_and_resolver_cancellable(
                &path,
                filename_rule,
                netease_filename_format,
                filename_policy,
                Some(resolver),
                scan_cancel,
            );
            if scan_cancel.is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::SeqCst)) {
                return Ok(None);
            }
            let mut key = name.clone();
            if resolved.contains_key(&key) {
                key = format!("{name}\u{1f}{}", path.display());
            }
            resolved.insert(key, (size, path));
        }
        source_files = resolved;
    }
    for issue in scan_issues {
        preview.errors.push(PreviewIssue {
            path: issue.path.display().to_string(),
            message: issue.message,
        });
        preview.error_count += 1;
    }
    let (destination_files, cancelled) = if !destination_directory.trim().is_empty()
        && Path::new(destination_directory).is_dir()
    {
        if let Some(observer) = observer.as_mut() {
            if let Some((budget, cancel)) = budget.as_ref() {
                let (files, _issues, cancelled) =
                        get_music_dict_with_scan_issues_with_settings_and_observer_with_budget_and_policy(
                            destination_directory,
                            &["mp3", "wav", "aiff"],
                            filename_rule,
                            NeteaseFilenameFormat::default(),
                            filename_policy,
                            Arc::clone(budget),
                            Arc::clone(cancel),
                            crate::sync::ScanPhase::Destination,
                            observer,
                        );
                (files, cancelled)
            } else {
                get_destination_music_dict_with_rule_and_observer(
                    destination_directory,
                    filename_rule,
                    observer,
                )
            }
        } else {
            (
                get_destination_music_dict_with_rule(destination_directory, filename_rule),
                false,
            )
        }
    } else {
        (Default::default(), false)
    };
    if cancelled {
        return Ok(None);
    };
    let mut occupied_paths = destination_files
        .values()
        .map(|(_, path)| path.clone())
        .collect::<HashSet<_>>();
    let mut planned_paths = HashSet::new();
    let mut source_entries = source_files.iter().collect::<Vec<_>>();
    source_entries.sort_by(|(left, _), (right, _)| left.cmp(right));

    // Count the paths before applying conflict handling.  A collision group
    // is the only case where the duplicate-name disambiguation rule applies;
    // ordinary candidates continue through the existing conflict strategy
    // unchanged.
    let mut expected_path_counts = HashMap::<PathBuf, usize>::new();
    for (raw_name, (_, path)) in &source_entries {
        let source_extension = effective_source_extension(path);
        let output_extension =
            resolve_output_policy(mode, lossless_format, &source_extension).output_extension;
        let source_name = source_entry_name(raw_name);
        let expected_path = target_output_path_with_policy(
            destination_directory,
            source_name,
            output_extension,
            filename_policy,
        );
        *expected_path_counts.entry(expected_path).or_default() += 1;
    }

    for (raw_name, (_, path)) in source_entries {
        let name = source_entry_name(raw_name);
        let source_size = match fs::metadata(path) {
            Ok(metadata) if metadata.len() > 0 => metadata.len(),
            Ok(_) => {
                preview.errors.push(PreviewIssue {
                    path: path.display().to_string(),
                    message: "源文件为空，无法转换".to_string(),
                });
                preview.error_count += 1;
                continue;
            }
            Err(error) => {
                preview.errors.push(PreviewIssue {
                    path: path.display().to_string(),
                    message: format!("无法读取源文件：{error}"),
                });
                preview.error_count += 1;
                continue;
            }
        };

        let source_extension = effective_source_extension(path);
        let output_extension =
            resolve_output_policy(mode, lossless_format, &source_extension).output_extension;
        let expected_path = target_output_path_with_policy(
            destination_directory,
            name,
            output_extension,
            filename_policy,
        );
        let collision_group = expected_path_counts
            .get(&expected_path)
            .copied()
            .unwrap_or_default()
            > 1;
        let in_batch_collision = planned_paths.contains(&expected_path);
        if in_batch_collision && !collision_group {
            match conflict_strategy {
                ConflictStrategy::Rename => {}
                ConflictStrategy::Skip => {
                    preview.skipped_count += 1;
                    preview.skipped.push(PreviewIssue {
                        path: path.display().to_string(),
                        message: "输出文件名与本批次其他歌曲冲突，已跳过".to_string(),
                    });
                    continue;
                }
                ConflictStrategy::Overwrite | ConflictStrategy::UpdateMetadata => {
                    preview.error_count += 1;
                    preview.errors.push(PreviewIssue {
                        path: path.display().to_string(),
                        message: "本批次歌曲生成相同输出文件名，已拒绝覆盖或仅更新元数据"
                            .to_string(),
                    });
                    continue;
                }
            }
        }
        let existing_path = if collision_group {
            None
        } else {
            destination_files
                .get(name)
                .filter(|(_, path)| {
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case(output_extension))
                })
                .map(|(_, path)| path.clone())
                .or_else(|| expected_path.exists().then_some(expected_path.clone()))
        };
        let has_existing = existing_path.is_some();
        let mut candidate_name = name.to_string();
        let mut operation = CandidateOperation::Convert;

        if has_existing {
            preview.existing_count += 1;
            match conflict_strategy {
                ConflictStrategy::Skip => {
                    preview.skipped_count += 1;
                    continue;
                }
                ConflictStrategy::Overwrite => {}
                ConflictStrategy::Rename => {}
                ConflictStrategy::UpdateMetadata => {
                    if supports_metadata_update(
                        path,
                        existing_path.as_ref().expect("checked above"),
                    ) {
                        operation = CandidateOperation::UpdateMetadata;
                    } else {
                        preview.warnings.push(PreviewIssue {
                            path: path.display().to_string(),
                            message: "此格式暂不支持仅更新元数据；将保留现有输出文件".to_string(),
                        });
                        preview.skipped_count += 1;
                        continue;
                    }
                }
            }
        }

        if matches!(conflict_strategy, ConflictStrategy::Rename)
            && !collision_group
            && matches!(operation, CandidateOperation::Convert)
        {
            let desired_path = target_output_path_with_policy(
                destination_directory,
                &candidate_name,
                output_extension,
                filename_policy,
            );
            if has_existing
                || in_batch_collision
                || desired_path.exists()
                || occupied_paths.contains(&desired_path)
            {
                candidate_name = next_available_name(
                    destination_directory,
                    name,
                    output_extension,
                    filename_policy,
                    &mut occupied_paths,
                );
            } else {
                occupied_paths.insert(desired_path);
            }
        }

        let estimated_bytes = if matches!(operation, CandidateOperation::UpdateMetadata) {
            0
        } else {
            source_size
        };
        let estimated_output_bytes = Some(estimated_bytes);
        let destination_path = if matches!(operation, CandidateOperation::UpdateMetadata) {
            existing_path.clone().unwrap_or_else(|| {
                target_output_path_with_policy(
                    destination_directory,
                    &candidate_name,
                    output_extension,
                    filename_policy,
                )
            })
        } else {
            target_output_path_with_policy(
                destination_directory,
                &candidate_name,
                output_extension,
                filename_policy,
            )
        };
        let previous_destination_path = if matches!(conflict_strategy, ConflictStrategy::Overwrite)
            && existing_path
                .as_ref()
                .is_some_and(|existing| existing != &destination_path)
        {
            existing_path
                .as_ref()
                .map(|existing| existing.display().to_string())
        } else {
            None
        };
        if paths_refer_to_same_file(path, &destination_path) {
            preview.errors.push(PreviewIssue {
                path: path.display().to_string(),
                message: "输出文件与源文件相同；请选择其他输出目录，避免覆盖原曲".to_string(),
            });
            preview.error_count += 1;
            continue;
        }
        preview.candidates.push(PreviewCandidate {
            name: candidate_name.clone(),
            source_path: path.display().to_string(),
            destination_path: destination_path.display().to_string(),
            source_size_bytes: source_size,
            estimated_output_bytes,
            previous_destination_path,
            operation,
            netease_track_id: None,
            netease_album_id: None,
            album: None,
            netease_title: None,
            netease_artist: None,
            disambiguation_reason: None,
        });
        planned_paths.insert(destination_path);
        preview.new_count += 1;
        preview.estimated_output_bytes = preview
            .estimated_output_bytes
            .and_then(|total| total.checked_add(estimated_bytes));
    }

    if matches!(filename_policy, FilenameNormalizationPolicy::PreserveSource)
        && let Some(resolver) = metadata_resolver
    {
        attach_netease_identities(&mut preview, resolver);
    }

    let identities = preview
        .candidates
        .iter()
        .filter_map(|candidate| {
            let destination = Path::new(&candidate.destination_path);
            let collision_group = expected_path_counts
                .get(&target_output_path_with_policy(
                    destination_directory,
                    &candidate.name,
                    destination
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default(),
                    filename_policy,
                ))
                .copied()
                .unwrap_or_default()
                > 1;
            collision_group.then(|| {
                let metadata = read_track_metadata(Path::new(&candidate.source_path));
                (
                    candidate.source_path.clone(),
                    OutputTrackIdentity {
                        title: candidate.netease_title.clone().unwrap_or(metadata.title),
                        artists: candidate.netease_artist.clone().unwrap_or(metadata.artist),
                        album: candidate.album.clone().unwrap_or(metadata.album),
                        track_id: candidate.netease_track_id.clone(),
                        album_id: candidate.netease_album_id.clone(),
                        source_path: PathBuf::from(&candidate.source_path),
                    },
                )
            })
        })
        .collect::<HashMap<_, _>>();
    disambiguate_duplicate_output_names(&mut preview, &identities);

    let requires_ffmpeg = preview.candidates.iter().any(|candidate| {
        if matches!(candidate.operation, CandidateOperation::UpdateMetadata) {
            return false;
        }
        let extension = Path::new(&candidate.source_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_lowercase();
        !matches!(mode, Mode::Compat if extension == "mp3")
            && !matches!(mode, Mode::Lossless if extension == "mp3")
    });
    if requires_ffmpeg && find_ffmpeg().is_none() {
        preview.warnings.push(PreviewIssue {
            path: destination_directory.to_string(),
            message: "当前转换需要 FFmpeg，但未找到 FFmpeg".to_string(),
        });
    }

    finalize_preview_summary(&mut preview, conflict_strategy);

    preview.available_space_bytes = available_disk_space(Path::new(destination_directory));
    if let (Some(required), Some(available)) = (
        preview.estimated_output_bytes,
        preview.available_space_bytes,
    ) {
        let sufficient = available >= required;
        preview.disk_space_sufficient = Some(sufficient);
        if !sufficient {
            preview.warnings.push(PreviewIssue {
                path: destination_directory.to_string(),
                message: format!(
                    "磁盘空间不足：预计需要 {} 字节，当前可用 {} 字节",
                    required, available
                ),
            });
        }
    }

    Ok(Some(preview))
}

fn finalize_preview_summary(preview: &mut SyncPreview, strategy: ConflictStrategy) {
    preview.input_count = preview
        .candidates
        .len()
        .saturating_add(preview.skipped.len())
        .saturating_add(preview.errors.len());
    preview.output_duplicate_count = preview.existing_count;
    preview.action_kind = match strategy {
        ConflictStrategy::Skip => "skip",
        ConflictStrategy::Overwrite => "overwrite",
        ConflictStrategy::Rename => "rename",
        ConflictStrategy::UpdateMetadata => "update_metadata",
    }
    .to_string();
    preview.action_count = match strategy {
        ConflictStrategy::Skip => preview.skipped_count,
        ConflictStrategy::Overwrite => preview
            .candidates
            .iter()
            .filter(|candidate| Path::new(&candidate.destination_path).exists())
            .count(),
        ConflictStrategy::Rename => preview.candidates.len(),
        ConflictStrategy::UpdateMetadata => preview
            .candidates
            .iter()
            .filter(|candidate| matches!(candidate.operation, CandidateOperation::UpdateMetadata))
            .count(),
    };

    let mut items = Vec::with_capacity(preview.input_count);
    for candidate in &preview.candidates {
        let destination = Path::new(&candidate.destination_path);
        let existing_output = destination.exists()
            || candidate
                .previous_destination_path
                .as_deref()
                .is_some_and(|path| Path::new(path).exists());
        let classification = if matches!(candidate.operation, CandidateOperation::UpdateMetadata) {
            "update_metadata"
        } else if matches!(strategy, ConflictStrategy::Overwrite) && existing_output {
            "overwrite"
        } else if matches!(strategy, ConflictStrategy::Rename) {
            "rename"
        } else if existing_output {
            "duplicate"
        } else {
            "new"
        };
        items.push(PreviewDetailItem {
            name: candidate.name.clone(),
            source_path: candidate.source_path.clone(),
            destination_path: Some(candidate.destination_path.clone()),
            existing_output,
            classification: classification.to_string(),
            reason: candidate.disambiguation_reason.clone(),
        });
    }
    for issue in &preview.skipped {
        let existing_output = !issue.message.contains("本批次") && !issue.message.contains("batch");
        items.push(PreviewDetailItem {
            name: Path::new(&issue.path)
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| issue.path.clone()),
            source_path: issue.path.clone(),
            destination_path: None,
            existing_output,
            classification: "skip".to_string(),
            reason: Some(issue.message.clone()),
        });
    }
    for issue in &preview.errors {
        items.push(PreviewDetailItem {
            name: Path::new(&issue.path)
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| issue.path.clone()),
            source_path: issue.path.clone(),
            destination_path: None,
            existing_output: false,
            classification: "error".to_string(),
            reason: Some(issue.message.clone()),
        });
    }
    items.sort_by_cached_key(|item| item.name.to_lowercase());
    preview.detail_items = items;
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

pub fn build_retry_preview(entry: &HistoryEntry) -> SyncPreview {
    let mut preview = SyncPreview {
        source_directory: entry.source_directory.clone(),
        destination_directory: entry.destination_directory.clone(),
        new_count: 0,
        existing_count: 0,
        skipped_count: 0,
        error_count: 0,
        estimated_output_bytes: Some(0),
        candidates: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
        available_space_bytes: None,
        disk_space_sufficient: None,
        input_count: 0,
        output_duplicate_count: 0,
        action_kind: String::new(),
        action_count: 0,
        database_directory: None,
        detail_items: Vec::new(),
    };

    for pending_file in &entry.pending_files {
        let source_path = Path::new(&pending_file.source_path);
        if is_ignored_music_file(source_path) {
            continue;
        }
        match fs::metadata(source_path) {
            Ok(metadata) if metadata.len() > 0 => {
                let estimated = pending_file.estimated_output_bytes.or(Some(metadata.len()));
                preview.estimated_output_bytes = match (preview.estimated_output_bytes, estimated) {
                    (Some(total), Some(value)) => total.checked_add(value),
                    _ => None,
                };
                preview.candidates.push(PreviewCandidate {
                    name: pending_file.name.clone(),
                    source_path: pending_file.source_path.clone(),
                    destination_path: pending_file.destination_path.clone(),
                    source_size_bytes: metadata.len(),
                    estimated_output_bytes: estimated,
                    previous_destination_path: pending_file.previous_destination_path.clone(),
                    operation: pending_file.operation,
                    netease_track_id: None,
                    netease_album_id: None,
                    album: None,
                    netease_title: None,
                    netease_artist: None,
                    disambiguation_reason: None,
                });
                preview.new_count += 1;
            }
            Ok(_) => {
                preview.errors.push(PreviewIssue {
                    path: pending_file.source_path.clone(),
                    message: "源文件为空，无法继续".to_string(),
                });
                preview.error_count += 1;
            }
            Err(error) => {
                preview.errors.push(PreviewIssue {
                    path: pending_file.source_path.clone(),
                    message: format!("继续任务时找不到源文件：{error}"),
                });
                preview.error_count += 1;
            }
        }
    }

    for failed_file in &entry.failed_files {
        let source_path = Path::new(&failed_file.source_path);
        if is_ignored_music_file(source_path) {
            continue;
        }
        match fs::metadata(source_path) {
            Ok(metadata) if metadata.len() > 0 => {
                let candidate = PreviewCandidate {
                    name: failed_file.name.clone(),
                    source_path: failed_file.source_path.clone(),
                    destination_path: failed_file.destination_path.clone(),
                    source_size_bytes: metadata.len(),
                    estimated_output_bytes: Some(metadata.len()),
                    previous_destination_path: None,
                    operation: CandidateOperation::Convert,
                    netease_track_id: None,
                    netease_album_id: None,
                    album: None,
                    netease_title: None,
                    netease_artist: None,
                    disambiguation_reason: None,
                };
                preview.estimated_output_bytes = preview
                    .estimated_output_bytes
                    .and_then(|total| total.checked_add(metadata.len()));
                preview.candidates.push(candidate);
                preview.new_count += 1;
            }
            Ok(_) => {
                preview.errors.push(PreviewIssue {
                    path: failed_file.source_path.clone(),
                    message: "源文件为空，无法重试".to_string(),
                });
                preview.error_count += 1;
            }
            Err(error) => {
                preview.errors.push(PreviewIssue {
                    path: failed_file.source_path.clone(),
                    message: format!("重试时找不到源文件：{error}"),
                });
                preview.error_count += 1;
            }
        }
    }

    if preview.candidates.is_empty() && preview.errors.is_empty() {
        preview.estimated_output_bytes = None;
    }
    preview.available_space_bytes = available_disk_space(Path::new(&preview.destination_directory));
    if let (Some(required), Some(available)) = (
        preview.estimated_output_bytes,
        preview.available_space_bytes,
    ) {
        preview.disk_space_sufficient = Some(available >= required);
    }
    preview
}

fn next_available_name(
    destination_directory: &str,
    base_name: &str,
    extension: &str,
    filename_policy: FilenameNormalizationPolicy,
    occupied_paths: &mut HashSet<std::path::PathBuf>,
) -> String {
    let mut index = 2usize;
    loop {
        let candidate = format!("{} ({})", base_name, index);
        let path = target_output_path_with_policy(
            destination_directory,
            &candidate,
            extension,
            filename_policy,
        );
        if !path.exists() && !occupied_paths.contains(&path) {
            occupied_paths.insert(path);
            return candidate;
        }
        index += 1;
    }
}

fn supports_metadata_update(source: &Path, destination: &Path) -> bool {
    let extension = |path: &Path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_lowercase())
    };
    matches!(
        extension(source).as_deref(),
        Some("mp3") | Some("wav") | Some("aiff") | Some("flac") | Some("ncm")
    ) && matches!(
        extension(destination).as_deref(),
        Some("mp3") | Some("wav") | Some("aiff")
    )
}

#[cfg(unix)]
fn available_disk_space(path: &Path) -> Option<u64> {
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let probe = path.ancestors().find(|candidate| candidate.exists())?;
    let c_path = CString::new(probe.as_os_str().as_bytes()).ok()?;
    let mut stat = MaybeUninit::<libc::statvfs>::zeroed();
    let result = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    let stat = unsafe { stat.assume_init() };
    (stat.f_bavail as u64).checked_mul(stat.f_frsize)
}

#[cfg(target_os = "windows")]
fn available_disk_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory_name: *const u16,
            free_bytes_available: *mut u64,
            total_bytes: *mut u64,
            total_free_bytes: *mut u64,
        ) -> i32;
    }

    let probe = path.ancestors().find(|candidate| candidate.exists())?;
    let wide = probe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut available = 0u64;
    let result = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (result != 0).then_some(available)
}

#[cfg(not(any(unix, target_os = "windows")))]
fn available_disk_space(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::build_sync_preview;
    use crate::config::Mode;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn previews_a_single_supported_audio_file() {
        let source_dir = tempdir().unwrap();
        let destination_dir = tempdir().unwrap();
        let source_file = source_dir.path().join("single-track.mp3");
        fs::write(&source_file, b"not-empty-audio-placeholder").unwrap();

        let preview = build_sync_preview(
            source_file.to_str().unwrap(),
            destination_dir.path().to_str().unwrap(),
            Mode::Compat,
            None,
        )
        .unwrap();

        assert_eq!(preview.new_count, 1);
        assert_eq!(preview.error_count, 0);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(
            preview.candidates[0].source_path,
            source_file.display().to_string()
        );
    }
}
