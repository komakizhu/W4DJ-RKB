use std::io::{Cursor, Seek, SeekFrom, Write};

use anyhow::Result;
use id3::frame::Picture;
use id3::{TagLike, Version};

use ncmdump::NcmInfo;

pub(crate) fn get_image_mime_type(bytes: &[u8]) -> &'static str {
    if bytes.len() < 12 {
        return "image/*";
    }

    match &bytes[..12] {
        [0x89, 0x50, 0x4e, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, ..] => "image/png",
        [0xFF, 0xD8, 0xFF, 0xE0 | 0xE1 | 0xE2 | 0xE3 | 0xE8, ..] => "image/jpeg",
        [0x52, 0x49, 0x46, 0x46, _, _, _, _, 0x57, 0x45, 0x42, 0x50] => "image/webp",
        [0x47, 0x49, 0x46, 0x38, ..] => "image/gif",
        [0x42, 0x4d, ..] => "image/bmp",
        _ => "image/*",
    }
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

    if !image.is_empty() {
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
    let image = tag
        .pictures()
        .find(|picture| picture.picture_type == metaflac::block::PictureType::CoverFront)
        .or_else(|| tag.pictures().next())
        .map(|picture| picture.data.as_slice())
        .unwrap_or_default();

    build_id3_tag_from_parts(title, album, &artists, image)
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
        for picture in base_tag.pictures() {
            tag.add_frame(Picture {
                mime_type: picture.mime_type.clone(),
                picture_type: picture.picture_type,
                description: picture.description.clone(),
                data: picture.data.clone(),
            });
        }
        Self(tag)
    }
}

impl Metadata for Mp3Metadata {
    fn inject_metadata(&mut self, data: Vec<u8>) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(data);
        _ = cursor.seek(SeekFrom::Start(0));
        self.0.write_to_file(&mut cursor, Version::Id3v24)?;
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
        if !image.is_empty() {
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
