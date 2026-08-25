use std::collections::BTreeMap;
use std::io::{Cursor, Error, ErrorKind, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::Result;
use id3::frame::Picture;
use id3::{TagLike, Version};

use ncmdump::NcmInfo;

use crate::analysis::TrackAnalysis;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataWriteProfile {
    NcmCore,
    Enriched,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceMetadata {
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub genre: String,
    pub aliases: String,
    pub copyright: String,
    pub publish_date: String,
    pub cover: Option<Vec<u8>>,
    pub lyric_plain: String,
    pub lyric_translated: String,
    pub lyric_romanized: String,
    pub lyric_lrc: String,
}

pub type NeteaseMetadataBundle = SourceMetadata;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutputMetadataPlan {
    pub fields: BTreeMap<String, String>,
    pub cover: Option<Vec<u8>>,
    pub lyrics: Vec<(String, String)>,
    pub written_fields: Vec<String>,
    pub unsupported_fields: Vec<String>,
}

pub fn build_output_metadata(
    profile: MetadataWriteProfile,
    source: &SourceMetadata,
    netease: Option<&NeteaseMetadataBundle>,
    analysis: Option<&TrackAnalysis>,
) -> OutputMetadataPlan {
    let mut plan = OutputMetadataPlan::default();
    let netease = netease.unwrap_or(source);
    let mut set = |name: &str, value: &str| {
        if !value.trim().is_empty() {
            plan.fields
                .insert(name.to_string(), value.trim().to_string());
        }
    };
    set(
        "title",
        if !netease.title.trim().is_empty() {
            &netease.title
        } else {
            &source.title
        },
    );
    let artists = if !netease.artists.is_empty() {
        netease.artists.join(", ")
    } else {
        source.artists.join(", ")
    };
    set("artist", &artists);
    set(
        "album",
        if !netease.album.trim().is_empty() {
            &netease.album
        } else {
            &source.album
        },
    );
    plan.cover = netease.cover.clone().or_else(|| source.cover.clone());
    for (description, text) in [
        ("plain", netease.lyric_plain.as_str()),
        ("translated", netease.lyric_translated.as_str()),
        ("romanized", netease.lyric_romanized.as_str()),
    ] {
        if !text.trim().is_empty() {
            plan.lyrics
                .push((description.to_string(), text.to_string()));
        }
    }
    if !netease.lyric_lrc.trim().is_empty() {
        plan.lyrics
            .push(("lrc".to_string(), netease.lyric_lrc.clone()));
    }
    if profile == MetadataWriteProfile::Enriched {
        set("genre", &netease.genre);
        set("aliases", &netease.aliases);
        set("copyright", &netease.copyright);
        set("date", &netease.publish_date);
        if let Some(analysis) = analysis {
            if let Some(value) = analysis.bpm.filter(|value| value.is_finite()) {
                set("bpm", &format!("{value:.2}"));
            }
            if let Some(value) = analysis
                .key
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                set("key", value);
            }
            if let Some(value) = analysis
                .integrated_loudness_lufs
                .filter(|value| value.is_finite())
            {
                set("loudness", &format!("{value:.2}"));
            }
            if let Some(value) = analysis.energy.filter(|value| value.is_finite()) {
                set("energy", &format!("{value:.4}"));
            }
            if let Some(value) = analysis.danceability.filter(|value| value.is_finite()) {
                set("danceability", &format!("{value:.4}"));
            }
            if let Some(value) = analysis
                .drop_loudness_lufs
                .filter(|value| value.is_finite())
            {
                set("drop_loudness", &format!("{value:.2}"));
            }
            if let Some(high_level) = analysis.high_level.as_ref() {
                if let Some(label) = high_level.genre.first() {
                    set("essentia_genre", &label.label);
                }
                if !high_level.mood.is_empty() {
                    set(
                        "mood",
                        &serde_json::to_string(&high_level.mood).unwrap_or_default(),
                    );
                }
                if !high_level.instrument.is_empty() {
                    set(
                        "instrument",
                        &serde_json::to_string(&high_level.instrument).unwrap_or_default(),
                    );
                }
            }
        }
    }
    plan
}

pub fn supported_metadata_fields(extension: &str) -> Vec<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "mp3" | "aiff" | "aif" => vec![
            "title",
            "artist",
            "album",
            "genre",
            "date",
            "copyright",
            "bpm",
            "key",
            "loudness",
            "energy",
            "danceability",
            "drop_loudness",
            "aliases",
            "essentia_genre",
            "mood",
            "instrument",
        ],
        "flac" => vec![
            "title",
            "artist",
            "album",
            "genre",
            "date",
            "copyright",
            "bpm",
            "key",
            "loudness",
            "energy",
            "danceability",
            "drop_loudness",
            "aliases",
            "essentia_genre",
            "mood",
            "instrument",
        ],
        "wav" => vec!["title", "artist", "album", "genre", "date", "copyright"],
        _ => Vec::new(),
    }
}

pub fn split_supported_fields(
    plan: &OutputMetadataPlan,
    extension: &str,
) -> (Vec<String>, Vec<String>) {
    let supported = supported_metadata_fields(extension);
    let written = plan
        .fields
        .keys()
        .filter(|field| supported.contains(&field.as_str()))
        .cloned()
        .collect();
    let unsupported = plan
        .fields
        .keys()
        .filter(|field| !supported.contains(&field.as_str()))
        .cloned()
        .collect();
    (written, unsupported)
}

pub fn validate_output_metadata_plan(path: &Path, plan: &OutputMetadataPlan) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("输出文件不存在：{}", path.display()));
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let title = plan
        .fields
        .get("title")
        .map(String::as_str)
        .unwrap_or_default();
    let artist = plan
        .fields
        .get("artist")
        .map(String::as_str)
        .unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "mp3" | "aiff" | "aif" => {
            let tag = id3::Tag::read_from_path(path).map_err(|error| error.to_string())?;
            if !title.is_empty() && tag.title() != Some(title) {
                return Err("输出标题写入校验失败".to_string());
            }
            if !artist.is_empty() && tag.artist() != Some(artist) {
                return Err("输出歌手写入校验失败".to_string());
            }
        }
        "flac" => {
            let tag = metaflac::Tag::read_from_path(path).map_err(|error| error.to_string())?;
            let comments = tag
                .vorbis_comments()
                .ok_or_else(|| "FLAC 缺少 Vorbis Comment".to_string())?;
            if !title.is_empty()
                && comments
                    .title()
                    .and_then(|values| values.first())
                    .map(String::as_str)
                    != Some(title)
            {
                return Err("FLAC 标题写入校验失败".to_string());
            }
            if !artist.is_empty()
                && comments
                    .artist()
                    .and_then(|values| values.first())
                    .map(String::as_str)
                    != Some(artist)
            {
                return Err("FLAC 歌手写入校验失败".to_string());
            }
        }
        "wav" => {
            if !path
                .metadata()
                .map_err(|error| error.to_string())?
                .len()
                .gt(&44)
            {
                return Err("WAV 输出过小，无法完成写入校验".to_string());
            }
        }
        _ => return Err(format!("不支持的元数据格式：{extension}")),
    }
    Ok(())
}

#[allow(deprecated)]
pub fn write_output_metadata_plan(path: &Path, plan: &OutputMetadataPlan) -> std::io::Result<()> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let value = |name: &str| {
        plan.fields
            .get(name)
            .map(String::as_str)
            .unwrap_or_default()
    };
    match extension.as_str() {
        "mp3" | "aiff" | "aif" | "wav" => {
            let mut tag = id3::Tag::read_from_path(path).unwrap_or_else(|_| id3::Tag::new());
            if !value("title").is_empty() {
                tag.set_title(value("title"));
            }
            if !value("artist").is_empty() {
                tag.set_artist(value("artist"));
            }
            if !value("album").is_empty() {
                tag.set_album(value("album"));
            }
            if !value("genre").is_empty() {
                tag.set_genre(value("genre"));
            }
            if !value("date").is_empty() {
                tag.set_text("TDRC", value("date"));
            }
            if let Some(cover) = plan
                .cover
                .as_deref()
                .filter(|cover| get_image_mime_type(cover) != "image/*")
            {
                tag.remove("APIC");
                tag.add_frame(Picture {
                    mime_type: get_image_mime_type(cover).to_string(),
                    picture_type: id3::frame::PictureType::CoverFront,
                    description: String::new(),
                    data: cover.to_vec(),
                });
            }
            match extension.as_str() {
                "wav" => tag
                    .write_to_wav_path(path, Version::Id3v24)
                    .map_err(Error::other),
                "aiff" | "aif" => tag
                    .write_to_aiff_path(path, Version::Id3v24)
                    .map_err(Error::other),
                _ => tag
                    .write_to_path(path, Version::Id3v24)
                    .map_err(Error::other),
            }
        }
        "flac" => {
            let mut tag =
                metaflac::Tag::read_from_path(path).unwrap_or_else(|_| metaflac::Tag::new());
            if !value("title").is_empty() {
                tag.set_vorbis("TITLE", vec![value("title")]);
            }
            if !value("artist").is_empty() {
                tag.set_vorbis("ARTIST", vec![value("artist")]);
            }
            if !value("album").is_empty() {
                tag.set_vorbis("ALBUM", vec![value("album")]);
            }
            if !value("genre").is_empty() {
                tag.set_vorbis("GENRE", vec![value("genre")]);
            }
            if !value("date").is_empty() {
                tag.set_vorbis("DATE", vec![value("date")]);
            }
            if !value("copyright").is_empty() {
                tag.set_vorbis("COPYRIGHT", vec![value("copyright")]);
            }
            if let Some(cover) = plan
                .cover
                .as_deref()
                .filter(|cover| get_image_mime_type(cover) != "image/*")
            {
                tag.remove_blocks(metaflac::BlockType::Picture);
                tag.add_picture(
                    get_image_mime_type(cover),
                    metaflac::block::PictureType::CoverFront,
                    cover.to_vec(),
                );
            }
            tag.write_to_path(path).map_err(Error::other)
        }
        _ => Err(Error::new(
            ErrorKind::Unsupported,
            format!("不支持的元数据格式：{extension}"),
        )),
    }
}

pub(crate) fn get_image_mime_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "image/jpeg";
    }
    if bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return "image/png";
    }
    if bytes.len() >= 12
        && bytes.starts_with(b"RIFF")
        && bytes.get(8..12) == Some(b"WEBP".as_slice())
    {
        return "image/webp";
    }
    if bytes.starts_with(b"GIF8") {
        return "image/gif";
    }
    if bytes.starts_with(b"BM") {
        return "image/bmp";
    }

    "image/*"
}

pub(crate) trait Metadata {
    /// Get the data with metadata.
    fn inject_metadata(&mut self, data: Vec<u8>) -> Result<Vec<u8>>;
}

pub(crate) fn build_id3_tag(info: &NcmInfo, image: &[u8]) -> id3::Tag {
    let artists = info
        .artist
        .iter()
        .map(|item| item.0.to_owned())
        .collect::<Vec<String>>();

    build_id3_tag_from_parts(&info.name, &info.album, &artists, image)
}

pub(crate) fn build_id3_tag_from_parts(
    title: &str,
    album: &str,
    artists: &[String],
    image: &[u8],
) -> id3::Tag {
    let mut tag = id3::Tag::new();
    let artist = artists.join(", ");

    tag.set_title(title);
    tag.set_album(album);
    tag.set_artist(artist);

    if get_image_mime_type(image) != "image/*" {
        tag.add_frame(Picture {
            mime_type: get_image_mime_type(image).to_owned(),
            picture_type: id3::frame::PictureType::CoverFront,
            description: String::new(),
            data: image.to_vec(),
        });
    }

    tag
}

pub(crate) fn build_id3_tag_from_flac(tag: &metaflac::Tag) -> id3::Tag {
    let comments = tag.vorbis_comments();
    let title = comments
        .and_then(|block| block.title())
        .and_then(|values| values.first())
        .map(String::as_str)
        .unwrap_or_default();
    let album = comments
        .and_then(|block| block.album())
        .and_then(|values| values.first())
        .map(String::as_str)
        .unwrap_or_default();
    let artists = comments
        .and_then(|block| block.artist())
        .cloned()
        .unwrap_or_default();
    let genres = comments
        .and_then(|block| block.genre())
        .cloned()
        .unwrap_or_default();
    let image = tag
        .pictures()
        .filter(|picture| get_image_mime_type(&picture.data) != "image/*")
        .find(|picture| picture.picture_type == metaflac::block::PictureType::CoverFront)
        .or_else(|| {
            tag.pictures()
                .find(|picture| get_image_mime_type(&picture.data) != "image/*")
        })
        .map(|picture| picture.data.as_slice())
        .unwrap_or_default();

    let mut result = build_id3_tag_from_parts(title, album, &artists, image);
    if let Some(genre) = genres
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .find(|genre| !genre.is_empty())
    {
        result.set_genre(genre);
    }
    result
}

pub(crate) struct Mp3Metadata(id3::Tag);

impl Mp3Metadata {
    pub(crate) fn new(info: &NcmInfo, image: &[u8], data: &[u8]) -> Self {
        let cursor = Cursor::new(data.to_vec());
        let mut tag = id3::Tag::read_from2(cursor).unwrap_or_else(|_| id3::Tag::new());
        tag.remove_extended_text(Some("163 key(Don't modify)"), None);
        let base_tag = build_id3_tag(info, image);
        tag.set_title(base_tag.title().unwrap_or_default());
        tag.set_album(base_tag.album().unwrap_or_default());
        tag.set_artist(base_tag.artist().unwrap_or_default());
        if base_tag.pictures().next().is_some() {
            tag.remove("APIC");
            for picture in base_tag.pictures() {
                tag.add_frame(Picture {
                    mime_type: picture.mime_type.clone(),
                    picture_type: picture.picture_type,
                    description: picture.description.clone(),
                    data: picture.data.clone(),
                });
            }
        }
        Self(tag)
    }
}

impl Metadata for Mp3Metadata {
    fn inject_metadata(&mut self, data: Vec<u8>) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(data);
        _ = cursor.seek(SeekFrom::Start(0));
        self.0.write_to_file(&mut cursor, Version::Id3v23)?;
        Ok(cursor.into_inner())
    }
}

pub(crate) struct FlacMetadata(metaflac::Tag);

impl FlacMetadata {
    pub(crate) fn new(info: &NcmInfo, image: &[u8], data: &[u8]) -> Self {
        let mut tag = metaflac::Tag::read_from(&mut Cursor::new(&data))
            .unwrap_or_else(|_| metaflac::Tag::new());
        let mc = tag.vorbis_comments_mut();
        let artist = info
            .artist
            .iter()
            .cloned()
            .map(|item| item.0)
            .collect::<Vec<String>>();
        mc.set_title(vec![info.name.to_string()]);
        mc.set_album(vec![info.album.to_string()]);
        mc.set_artist(artist);
        if get_image_mime_type(image) != "image/*" {
            tag.remove_blocks(metaflac::BlockType::Picture);
            tag.add_picture(
                get_image_mime_type(image),
                metaflac::block::PictureType::CoverFront,
                image.to_vec(),
            );
        }
        Self(tag)
    }
}

impl Metadata for FlacMetadata {
    fn inject_metadata(&mut self, data: Vec<u8>) -> Result<Vec<u8>> {
        let data = metaflac::Tag::skip_metadata(&mut Cursor::new(&data));
        let mut buffer = Vec::new();
        self.0.remove_blocks(metaflac::BlockType::Padding);
        self.0.write_to(&mut buffer)?;
        buffer.write_all(&data)?;
        Ok(buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Metadata, MetadataWriteProfile, Mp3Metadata, SourceMetadata, build_id3_tag_from_parts,
        build_output_metadata, split_supported_fields,
    };
    use id3::{Tag, TagLike, Version};
    use ncmdump::NcmInfo;
    use std::io::Cursor;

    fn sample_info() -> NcmInfo {
        NcmInfo {
            name: "Track title".to_string(),
            id: 1,
            album: "Album".to_string(),
            artist: vec![("Artist".to_string(), 2)],
            bitrate: 320_000,
            duration: 180_000,
            format: "mp3".to_string(),
            mv_id: None,
            alias: None,
        }
    }

    #[test]
    fn ignores_invalid_cover_bytes_when_building_id3_metadata() {
        let tag = build_id3_tag_from_parts(
            "Track title",
            "Album",
            &["Artist".to_string()],
            b"not-an-image",
        );

        assert_eq!(tag.pictures().count(), 0);
    }

    #[test]
    fn replaces_stale_mp3_cover_with_the_extracted_cover() {
        let mut original = Tag::new();
        original.add_frame(id3::frame::Picture {
            mime_type: "image/jpeg".to_string(),
            picture_type: id3::frame::PictureType::CoverFront,
            description: String::new(),
            data: vec![0xff, 0xd8, 0xff, 0xe0, 0x01],
        });
        let mut original_bytes = Vec::new();
        original
            .write_to(&mut original_bytes, Version::Id3v23)
            .unwrap();
        original_bytes.extend_from_slice(b"audio");

        let extracted_cover = vec![0xff, 0xd8, 0xff, 0xe0, 0x02];
        let mut metadata = Mp3Metadata::new(&sample_info(), &extracted_cover, &original_bytes);
        let output = metadata.inject_metadata(b"audio".to_vec()).unwrap();
        let tag = Tag::read_from2(Cursor::new(output)).unwrap();
        let pictures = tag.pictures().collect::<Vec<_>>();

        assert_eq!(pictures.len(), 1);
        assert_eq!(pictures[0].data, extracted_cover);
        assert_eq!(tag.title(), Some("Track title"));
        assert_eq!(tag.artist(), Some("Artist"));
    }

    #[test]
    fn core_profile_keeps_identity_separate_from_analysis_fields() {
        let source = SourceMetadata {
            title: "Title".to_string(),
            artists: vec!["One".to_string(), "Two".to_string()],
            album: "Album".to_string(),
            genre: "NetEase genre".to_string(),
            aliases: "[\"Alias\"]".to_string(),
            ..SourceMetadata::default()
        };
        let plan = build_output_metadata(MetadataWriteProfile::NcmCore, &source, None, None);
        assert_eq!(plan.fields.get("artist"), Some(&"One, Two".to_string()));
        assert!(!plan.fields.contains_key("genre"));
        let (written, unsupported) = split_supported_fields(&plan, "mp3");
        assert!(written.contains(&"title".to_string()));
        assert!(unsupported.is_empty());
    }

    #[test]
    fn enriched_profile_includes_netease_extended_fields() {
        let source = SourceMetadata {
            title: "Title".to_string(),
            artists: vec!["Artist".to_string()],
            album: "Album".to_string(),
            genre: "J-Pop".to_string(),
            copyright: "Copyright".to_string(),
            publish_date: "2024-01-01".to_string(),
            ..SourceMetadata::default()
        };
        let plan = build_output_metadata(MetadataWriteProfile::Enriched, &source, None, None);
        assert_eq!(plan.fields.get("genre"), Some(&"J-Pop".to_string()));
        assert_eq!(plan.fields.get("copyright"), Some(&"Copyright".to_string()));
        assert_eq!(plan.fields.get("date"), Some(&"2024-01-01".to_string()));
    }
}
