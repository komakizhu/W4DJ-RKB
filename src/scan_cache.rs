use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCAN_CACHE_SCHEMA_VERSION: u32 = 3;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanCache {
    pub schema_version: u32,
    pub entries: BTreeMap<String, ScanCacheEntry>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanCacheEntry {
    pub source_path: String,
    pub source_root: String,
    pub output_directory: String,
    pub filename_rule: String,
    pub netease_filename_format: String,
    #[serde(default)]
    pub filename_policy: String,
    pub size_bytes: u64,
    pub modified_at_ms: Option<u64>,
    pub derived_name: String,
    #[serde(default)]
    pub source_extension: String,
    #[serde(default)]
    pub scan_issue: Option<String>,
    /// Authoritative identity is kept separate from the legacy derived name.
    /// These fields are optional so older cache entries remain readable while
    /// a schema upgrade forces a safe rescan.
    #[serde(default)]
    pub identity_basis: Option<String>,
    #[serde(default)]
    pub source_title: Option<String>,
    #[serde(default)]
    pub source_artist: Option<String>,
    #[serde(default)]
    pub safe_output_stem: Option<String>,
    #[serde(default)]
    pub collision_key: Option<String>,
    #[serde(default)]
    pub transformations: Vec<String>,
    #[serde(default)]
    pub collision_resolution: Option<String>,
}

impl ScanCache {
    pub fn empty() -> Self {
        Self {
            schema_version: SCAN_CACHE_SCHEMA_VERSION,
            entries: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, entry: ScanCacheEntry) {
        self.entries.insert(entry.source_path.clone(), entry);
        self.schema_version = SCAN_CACHE_SCHEMA_VERSION;
    }

    pub fn remove_missing_sources(&mut self, source_root: &Path) {
        let root = normalize_path(source_root);
        self.entries.retain(|_, entry| {
            let entry_root = normalize_path(Path::new(&entry.source_root));
            entry_root != root || Path::new(&entry.source_path).exists()
        });
    }

    /// Remove entries below a root using the paths already collected by the
    /// current scan. This keeps the fast path free of a second existence/stat
    /// pass over every cached source.
    pub fn remove_missing_sources_from_snapshot(
        &mut self,
        normalized_source_root: &Path,
        observed_paths: &HashSet<String>,
    ) {
        let root = normalized_source_root.to_string_lossy();
        self.entries.retain(|key, entry| {
            let same_root = entry.source_root == root
                || normalize_path(Path::new(&entry.source_root)) == normalized_source_root;
            !same_root || observed_paths.contains(key)
        });
    }
}

pub fn load_scan_cache(path: &Path) -> io::Result<ScanCache> {
    if !path.exists() {
        return Ok(ScanCache::empty());
    }

    let contents = fs::read_to_string(path)?;
    let cache: ScanCache = serde_json::from_str(&contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if cache.schema_version != SCAN_CACHE_SCHEMA_VERSION {
        // A cache is an optimization, never authoritative input.  Discarding
        // an older schema is safer than reusing a cleaned name as identity;
        // the next scan will atomically write the current schema.
        return Ok(ScanCache::empty());
    }
    Ok(cache)
}

pub fn save_scan_cache_atomic(path: &Path, cache: &ScanCache) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temporary_path = temporary_cache_path(path);
    let contents = serde_json::to_vec_pretty(cache)
        .map_err(|error| io::Error::other(format!("serialize scan cache: {error}")))?;
    let mut file = File::create(&temporary_path)?;
    file.write_all(&contents)?;
    file.sync_all()?;
    drop(file);

    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    Ok(())
}

pub fn clear_scan_cache(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn can_reuse_entry(
    entry: &ScanCacheEntry,
    source_path: &Path,
    source_root: &Path,
    output_directory: &Path,
    filename_rule: &str,
    netease_filename_format: &str,
    filename_policy: &str,
    size_bytes: u64,
    modified_at_ms: Option<u64>,
) -> bool {
    can_reuse_entry_normalized(
        entry,
        &normalize_path(source_path),
        &normalize_path(source_root),
        &normalize_path(output_directory),
        filename_rule,
        netease_filename_format,
        filename_policy,
        size_bytes,
        modified_at_ms,
    )
}

/// Fast path for a scan that has already normalized its roots once.  File
/// paths still carry their own fingerprint, but the expensive root
/// canonicalization is no longer repeated for every candidate.
#[allow(clippy::too_many_arguments)]
pub fn can_reuse_entry_normalized(
    entry: &ScanCacheEntry,
    normalized_source_path: &Path,
    normalized_source_root: &Path,
    normalized_output_directory: &Path,
    filename_rule: &str,
    netease_filename_format: &str,
    filename_policy: &str,
    size_bytes: u64,
    modified_at_ms: Option<u64>,
) -> bool {
    let source = normalized_source_path.to_string_lossy();
    let root = normalized_source_root.to_string_lossy();
    let output = normalized_output_directory.to_string_lossy();
    (entry.source_path == source
        || normalize_path(Path::new(&entry.source_path)) == normalized_source_path)
        && (entry.source_root == root
            || normalize_path(Path::new(&entry.source_root)) == normalized_source_root)
        && (entry.output_directory == output
            || normalize_path(Path::new(&entry.output_directory)) == normalized_output_directory)
        && entry.filename_rule == filename_rule
        && entry.netease_filename_format == netease_filename_format
        && entry.filename_policy == filename_policy
        && entry.size_bytes == size_bytes
        && entry.modified_at_ms == modified_at_ms
}

/// Fast-scan cache predicate for the value the cache actually owns: the
/// filename derivation.  The output directory is deliberately not part of
/// this identity because changing an output root must not force the same
/// source filename to be parsed again.  The older `can_reuse_entry*` APIs keep
/// their stricter output-directory semantics for compatibility callers.
#[allow(clippy::too_many_arguments)]
pub fn can_reuse_derived_name_entry_normalized(
    entry: &ScanCacheEntry,
    normalized_source_path: &Path,
    normalized_source_root: &Path,
    filename_rule: &str,
    netease_filename_format: &str,
    filename_policy: &str,
    size_bytes: u64,
    modified_at_ms: Option<u64>,
) -> bool {
    let source = normalized_source_path.to_string_lossy();
    let root = normalized_source_root.to_string_lossy();
    (entry.source_path == source
        || normalize_path(Path::new(&entry.source_path)) == normalized_source_path)
        && (entry.source_root == root
            || normalize_path(Path::new(&entry.source_root)) == normalized_source_root)
        && entry.filename_rule == filename_rule
        && entry.netease_filename_format == netease_filename_format
        && entry.filename_policy == filename_policy
        && entry.size_bytes == size_bytes
        && entry.modified_at_ms == modified_at_ms
}

pub fn modified_at_ms(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

pub fn normalize_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|current| current.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

fn temporary_cache_path(path: &Path) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("scan-cache.json");
    path.with_file_name(format!(".{filename}.{suffix}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn entry(root: &Path, output: &Path, source: &Path) -> ScanCacheEntry {
        ScanCacheEntry {
            source_path: source.display().to_string(),
            source_root: root.display().to_string(),
            output_directory: output.display().to_string(),
            filename_rule: "title_artist".to_string(),
            netease_filename_format: "title_artist".to_string(),
            filename_policy: "soundcloud".to_string(),
            size_bytes: 3,
            modified_at_ms: modified_at_ms(source),
            derived_name: "Song - Artist".to_string(),
            source_extension: "mp3".to_string(),
            scan_issue: None,
            ..Default::default()
        }
    }

    #[test]
    fn cache_round_trips_atomically() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("scan-cache.json");
        let source = directory.path().join("song.mp3");
        let output = directory.path().join("out");
        fs::write(&source, b"abc").unwrap();

        let mut cache = ScanCache::empty();
        cache.insert(entry(directory.path(), &output, &source));
        save_scan_cache_atomic(&path, &cache).unwrap();

        assert_eq!(load_scan_cache(&path).unwrap(), cache);
        assert!(!directory.path().join(".scan-cache.json.tmp").exists());
    }

    #[test]
    fn cache_reuse_requires_file_and_context_fingerprint() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("song.mp3");
        let output = directory.path().join("out");
        fs::write(&source, b"abc").unwrap();
        let cached = entry(directory.path(), &output, &source);

        assert!(can_reuse_entry(
            &cached,
            &source,
            directory.path(),
            &output,
            "title_artist",
            "title_artist",
            "soundcloud",
            3,
            modified_at_ms(&source),
        ));
        assert!(!can_reuse_entry(
            &cached,
            &source,
            directory.path(),
            &directory.path().join("other"),
            "title_artist",
            "title_artist",
            "soundcloud",
            3,
            modified_at_ms(&source),
        ));
        assert!(!can_reuse_entry(
            &cached,
            &source,
            directory.path(),
            &output,
            "artist_title",
            "title_artist",
            "soundcloud",
            3,
            modified_at_ms(&source),
        ));
    }

    #[test]
    fn derived_name_reuse_is_independent_of_output_root() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("song.mp3");
        fs::write(&source, b"abc").unwrap();
        let cached = entry(directory.path(), &directory.path().join("out-a"), &source);
        let normalized_source = normalize_path(&source);
        let normalized_root = normalize_path(directory.path());

        assert!(can_reuse_derived_name_entry_normalized(
            &cached,
            &normalized_source,
            &normalized_root,
            "title_artist",
            "title_artist",
            "soundcloud",
            3,
            modified_at_ms(&source),
        ));
    }

    #[test]
    fn damaged_cache_is_rejected_but_old_schema_is_safely_discarded() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("scan-cache.json");
        fs::write(&path, "{not-json").unwrap();
        assert_eq!(
            load_scan_cache(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );

        fs::write(
            &path,
            serde_json::to_vec(&ScanCache {
                schema_version: 99,
                entries: BTreeMap::new(),
            })
            .unwrap(),
        )
        .unwrap();
        let cache = load_scan_cache(&path).unwrap();
        assert_eq!(cache.schema_version, SCAN_CACHE_SCHEMA_VERSION);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn clear_cache_is_idempotent() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("scan-cache.json");
        fs::write(&path, "{}").unwrap();
        clear_scan_cache(&path).unwrap();
        clear_scan_cache(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn removing_missing_sources_is_scoped_to_one_task_root() {
        let directory = tempdir().unwrap();
        let first_root = directory.path().join("first");
        let second_root = directory.path().join("second");
        fs::create_dir_all(&first_root).unwrap();
        fs::create_dir_all(&second_root).unwrap();
        let first_source = first_root.join("first.mp3");
        let second_source = second_root.join("second.mp3");
        fs::write(&first_source, b"first").unwrap();
        fs::write(&second_source, b"second").unwrap();

        let mut cache = ScanCache::empty();
        cache.insert(entry(
            &first_root,
            &directory.path().join("out-1"),
            &first_source,
        ));
        cache.insert(entry(
            &second_root,
            &directory.path().join("out-2"),
            &second_source,
        ));
        fs::remove_file(&first_source).unwrap();

        cache.remove_missing_sources(&first_root);

        assert!(
            !cache
                .entries
                .values()
                .any(|item| item.source_path == first_source.display().to_string())
        );
        assert!(
            cache
                .entries
                .values()
                .any(|item| item.source_path == second_source.display().to_string())
        );
    }
}
