//! Separation between authoritative song identity and filesystem-safe names.
//!
//! Metadata fields are user/data values and must only be trimmed when they are
//! reliable. Filesystem cleanup is applied at the final output-name boundary.

use crate::config::{FilenameNormalizationPolicy, FilenameRule, NeteaseFilenameFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityBasis {
    SourceTags,
    FilenameInference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongIdentity {
    pub title: String,
    pub artist: String,
    pub basis: IdentityBasis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeOutputName {
    pub stem: String,
    pub collision_key: String,
    pub transformations: Vec<String>,
}

pub fn resolve_song_identity(
    fallback_stem: &str,
    source_title: Option<&str>,
    source_artist: Option<&str>,
    netease_format: NeteaseFilenameFormat,
) -> SongIdentity {
    let title = source_title
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let artist = source_artist
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let (Some(title), Some(artist)) = (title, artist) {
        return SongIdentity {
            title: title.to_string(),
            artist: artist.to_string(),
            basis: IdentityBasis::SourceTags,
        };
    }

    let fallback = fallback_stem.trim();
    let (left, right) = fallback
        .split_once(" - ")
        .map(|(left, right)| (left.trim(), right.trim()))
        .unwrap_or((fallback, ""));
    let (fallback_title, fallback_artist) = match netease_format {
        NeteaseFilenameFormat::TitleOnly => (fallback, ""),
        NeteaseFilenameFormat::ArtistTitle => (right, left),
        NeteaseFilenameFormat::TitleArtist => (left, right),
    };
    SongIdentity {
        title: title.unwrap_or(fallback_title).to_string(),
        artist: artist.unwrap_or(fallback_artist).to_string(),
        basis: IdentityBasis::FilenameInference,
    }
}

pub fn build_safe_output_name(
    identity: &SongIdentity,
    rule: FilenameRule,
    extension: &str,
    rename_index: Option<usize>,
) -> SafeOutputName {
    build_output_name(
        identity,
        rule,
        extension,
        rename_index,
        FilenameNormalizationPolicy::SoundCloud,
    )
}

pub fn build_output_name(
    identity: &SongIdentity,
    rule: FilenameRule,
    extension: &str,
    rename_index: Option<usize>,
    policy: FilenameNormalizationPolicy,
) -> SafeOutputName {
    let raw = match rule {
        FilenameRule::ArtistTitle => format_pair(&identity.artist, &identity.title),
        FilenameRule::TitleArtist | FilenameRule::Original => {
            format_pair(&identity.title, &identity.artist)
        }
    };
    let mut transformations = Vec::new();
    let mut stem = match policy {
        FilenameNormalizationPolicy::PreserveSource => raw.clone(),
        FilenameNormalizationPolicy::SoundCloud => sanitize_filename_component(&raw),
    };
    if stem != raw {
        transformations.push("filesystemCharactersReplaced".to_string());
    }
    if let Some(index) = rename_index.filter(|index| *index > 1) {
        stem = format!("{stem} ({index})");
        transformations.push("collisionRename".to_string());
    }
    let ext = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    let collision_key = format!("{}.{}", fold_collision_key(&stem), ext);
    SafeOutputName {
        stem,
        collision_key,
        transformations,
    }
}

fn format_pair(left: &str, right: &str) -> String {
    match (left.trim().is_empty(), right.trim().is_empty()) {
        (true, true) => "未命名".to_string(),
        (false, true) => left.trim().to_string(),
        (true, false) => right.trim().to_string(),
        (false, false) => format!("{} - {}", left.trim(), right.trim()),
    }
}

pub fn sanitize_filename_component(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let cleaned = trimmed
        .chars()
        .filter_map(|ch| match ch {
            // Quotes are valid on macOS and, when cleanup is requested for a
            // non-NetEase source, removing them avoids inventing a misleading
            // `-P3-` title. Other Windows-forbidden separators retain the
            // established replacement behavior.
            '"' => None,
            '/' | '\\' | ':' | '*' | '?' | '<' | '>' | '|' => Some('-'),
            control if control.is_control() => Some(' '),
            other => Some(other),
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches([' ', '.'])
        .to_string();
    let cleaned = if cleaned.is_empty() {
        "未命名"
    } else {
        &cleaned
    };
    let reserved = matches!(
        cleaned
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase()
            .as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    let mut cleaned = if reserved {
        format!("_{cleaned}")
    } else {
        cleaned.to_string()
    };
    if cleaned.starts_with('.') {
        cleaned.insert(0, '_');
    }
    truncate_component(&cleaned)
}

fn fold_collision_key(value: &str) -> String {
    // Removing combining marks makes NFC/NFD spellings collide conservatively
    // on filesystems that normalize names differently (notably macOS).
    value
        .chars()
        .filter(|ch| !matches!(*ch, '\u{0300}'..='\u{036f}'))
        .map(collapse_latin_accent)
        .flat_map(char::to_lowercase)
        .collect::<String>()
}

fn collapse_latin_accent(ch: char) -> char {
    match ch {
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'Ç' | 'ç' => 'c',
        'È' | 'É' | 'Ê' | 'Ë' | 'è' | 'é' | 'ê' | 'ë' => 'e',
        'Ì' | 'Í' | 'Î' | 'Ï' | 'ì' | 'í' | 'î' | 'ï' => 'i',
        'Ñ' | 'ñ' => 'n',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'ù' | 'ú' | 'û' | 'ü' => 'u',
        'Ý' | 'Ÿ' | 'ý' | 'ÿ' => 'y',
        other => other,
    }
}

fn truncate_component(value: &str) -> String {
    const MAX_UTF8_BYTES: usize = 220;
    const MAX_UTF16_UNITS: usize = 220;
    let mut result = String::new();
    for ch in value.chars() {
        let next_bytes = result.len() + ch.len_utf8();
        let next_utf16 = result.encode_utf16().count() + ch.len_utf16();
        if next_bytes > MAX_UTF8_BYTES || next_utf16 > MAX_UTF16_UNITS {
            break;
        }
        result.push(ch);
    }
    result.trim_end_matches([' ', '.']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reliable_source_tags_are_not_filesystem_sanitized() {
        let identity = resolve_song_identity(
            "fallback",
            Some(r#"Mass Destruction ("P3" + "P3F" ver.)"#),
            Some("川村ゆみ, Lotus Juice"),
            NeteaseFilenameFormat::TitleArtist,
        );
        assert_eq!(identity.basis, IdentityBasis::SourceTags);
        assert_eq!(identity.title, r#"Mass Destruction ("P3" + "P3F" ver.)"#);
        let output = build_output_name(
            &identity,
            FilenameRule::TitleArtist,
            "mp3",
            None,
            FilenameNormalizationPolicy::PreserveSource,
        );
        assert_eq!(
            output.stem,
            r#"Mass Destruction ("P3" + "P3F" ver.) - 川村ゆみ, Lotus Juice"#
        );
        let cleaned = build_safe_output_name(&identity, FilenameRule::TitleArtist, "mp3", None);
        assert_eq!(
            cleaned.stem,
            "Mass Destruction (P3 + P3F ver.) - 川村ゆみ, Lotus Juice"
        );
    }

    #[test]
    fn filename_fallback_is_kept_separate_from_safe_name() {
        let identity = resolve_song_identity(
            "Artist - A:B",
            None,
            None,
            NeteaseFilenameFormat::ArtistTitle,
        );
        assert_eq!(identity.title, "A:B");
        assert_eq!(identity.artist, "Artist");
        assert_eq!(
            build_safe_output_name(&identity, FilenameRule::TitleArtist, "flac", None).stem,
            "A-B - Artist"
        );
    }

    #[test]
    fn collision_keys_are_case_insensitive_and_extension_specific() {
        let identity = SongIdentity {
            title: "Song".into(),
            artist: "Artist".into(),
            basis: IdentityBasis::SourceTags,
        };
        let mp3 = build_safe_output_name(&identity, FilenameRule::TitleArtist, "mp3", None);
        let mp3_lower = build_safe_output_name(
            &SongIdentity {
                title: "song".into(),
                ..identity
            },
            FilenameRule::TitleArtist,
            "MP3",
            None,
        );
        assert_eq!(mp3.collision_key, mp3_lower.collision_key);
    }

    #[test]
    fn safe_names_handle_hidden_reserved_long_and_decomposed_values() {
        assert_eq!(sanitize_filename_component(".hidden"), "_.hidden");
        assert_eq!(sanitize_filename_component("CON"), "_CON");
        let long = sanitize_filename_component(&"界".repeat(200));
        assert!(long.len() <= 220);
        assert!(long.encode_utf16().count() <= 220);

        let nfc = SongIdentity {
            title: "Café".into(),
            artist: String::new(),
            basis: IdentityBasis::SourceTags,
        };
        let nfd = SongIdentity {
            title: "Cafe\u{301}".into(),
            ..nfc.clone()
        };
        assert_eq!(
            build_safe_output_name(&nfc, FilenameRule::Original, "mp3", None).collision_key,
            build_safe_output_name(&nfd, FilenameRule::Original, "mp3", None).collision_key
        );
    }
}
