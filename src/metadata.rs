use std::io::{Cursor, Seek, SeekFrom, Write};

use anyhow::Result;
use id3::frame::Picture;
use id3::{TagLike, Version};

use ncmdump::NcmInfo;

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
    use super::{Metadata, Mp3Metadata, build_id3_tag_from_parts};
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
}
