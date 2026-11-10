//! Attachments (kb/ADR-0022, kb/ADR-0023): the rules an upload passes before it is stored, and the attachment
//! itself. Pure: bytes and the handler's clock in, the stored form out. Nothing here reads the clock or the database.
use bytes::Bytes;
use chrono::{DateTime, Utc};
use img_parts::{ImageEXIF, jpeg::Jpeg, png::Png, webp::WebP};
use reeve_protocol::{ItemType, Key};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The largest upload accepted, in bytes (kb/ADR-0023 R2).
pub const MAX_BYTES: usize = 10_485_760;
/// The size of a stored chunk, in bytes (kb/ADR-0022 R1).
pub const CHUNK: usize = 1_048_576;
const MAX_NAME: usize = 200;
const MAX_EXT: usize = 5;

/// The image types an attachment may be, identified by their leading bytes (kb/ADR-0023 R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Png,
    Jpeg,
    Webp,
}

impl ImageType {
    /// The type the bytes identify, whatever the request declared; None for every other type.
    pub fn sniff(b: &[u8]) -> Option<Self> {
        if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            Some(Self::Png)
        } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
            Some(Self::Webp)
        } else {
            None
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }

    /// The inverse of `mime`, for a persisted row.
    pub fn from_mime(s: &str) -> Option<Self> {
        match s {
            "image/png" => Some(Self::Png),
            "image/jpeg" => Some(Self::Jpeg),
            "image/webp" => Some(Self::Webp),
            _ => None,
        }
    }
}

/// Who owns an attachment: one initiative, epic or story, or nobody yet (an upload not yet saved with any text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentOwner {
    Unclaimed,
    Item(ItemType, Key),
}

/// Why an upload is refused. The messages are what people read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UploadError {
    #[error("only PNG, JPEG and WebP images can be attached")]
    Unsupported,
    #[error("the image could not be read")]
    Unreadable,
    #[error("the file is larger than 10 MiB")]
    TooLarge,
}

/// The JPEG APP1 payload prefixes of EXIF and XMP.
const JPEG_EXIF: &[u8] = b"Exif\0\0";
const JPEG_XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/";
const JPEG_XMP_EXT: &[u8] = b"http://ns.adobe.com/xmp/extension/";
const APP1: u8 = 0xE1;
const APP13: u8 = 0xED;
/// The PNG chunks that carry metadata: EXIF and the three text chunks (XMP travels in `iTXt`).
const PNG_META: [[u8; 4]; 4] = [*b"eXIf", *b"tEXt", *b"zTXt", *b"iTXt"];

/// Removes EXIF and XMP metadata (and IPTC, and PNG text chunks) without decoding or re-encoding pixels
/// (kb/ADR-0023 R3). Image data, colour profiles and every other segment are kept byte for byte.
pub fn strip_metadata(t: ImageType, b: Bytes) -> Result<Bytes, UploadError> {
    let bad = |_| UploadError::Unreadable;
    Ok(match t {
        ImageType::Jpeg => {
            let mut j = Jpeg::from_bytes(b).map_err(bad)?;
            j.segments_mut().retain(|s| {
                let c = s.contents();
                !(s.marker() == APP13
                    || s.marker() == APP1
                        && (c.starts_with(JPEG_EXIF)
                            || c.starts_with(JPEG_XMP)
                            || c.starts_with(JPEG_XMP_EXT)))
            });
            j.encoder().bytes()
        }
        ImageType::Png => {
            let mut p = Png::from_bytes(b).map_err(bad)?;
            p.chunks_mut().retain(|c| !PNG_META.contains(&c.kind()));
            p.encoder().bytes()
        }
        ImageType::Webp => {
            let mut w = WebP::from_bytes(b).map_err(bad)?;
            w.remove_chunks_by_id(img_parts::webp::CHUNK_XMP);
            // `set_exif` also rewrites the VP8X flags from the chunks left, which clears the XMP flag too;
            // removing a chunk alone would leave a header that names metadata that is gone.
            w.set_exif(None);
            w.encoder().bytes()
        }
    })
}

/// A filename safe to store and to put in `Content-Disposition`: the last path segment, without control characters,
/// at most 200 characters (keeping a short extension), and `image` when nothing is left.
pub fn clean_filename(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let name: String = last.chars().filter(|c| !c.is_control()).collect();
    let name = name.trim();
    if name.is_empty() || name.chars().all(|c| c == '.') {
        return "image".into();
    }
    if name.chars().count() <= MAX_NAME {
        return name.to_string();
    }
    let ext = name
        .rfind('.')
        .map(|i| &name[i..])
        .filter(|e| e.chars().count() <= MAX_EXT + 1)
        .unwrap_or("");
    let keep = MAX_NAME - ext.chars().count();
    let stem: String = name.chars().take(keep).collect();
    format!("{stem}{ext}")
}

/// The stored chunks of a file, in order (kb/ADR-0022 R1).
pub fn chunks(b: &[u8]) -> impl Iterator<Item = &[u8]> {
    b.chunks(CHUNK)
}

/// An uploaded image: its metadata. The bytes are stored and read as chunks.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    id: Uuid,
    owner: AttachmentOwner,
    uploaded_by: String,
    uploaded_at: DateTime<Utc>,
    filename: String,
    image_type: ImageType,
    byte_size: i64,
    sha256: [u8; 32],
}

impl Attachment {
    /// A new, unclaimed attachment of the bytes to be stored (already stripped).
    pub fn new(
        id: Uuid,
        uploaded_by: &str,
        uploaded_at: DateTime<Utc>,
        filename: &str,
        image_type: ImageType,
        stored: &[u8],
    ) -> Self {
        Self {
            id,
            owner: AttachmentOwner::Unclaimed,
            uploaded_by: uploaded_by.to_string(),
            uploaded_at,
            filename: clean_filename(filename),
            image_type,
            byte_size: stored.len() as i64,
            sha256: Sha256::digest(stored).into(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        id: Uuid,
        owner: AttachmentOwner,
        uploaded_by: String,
        uploaded_at: DateTime<Utc>,
        filename: String,
        image_type: ImageType,
        byte_size: i64,
        sha256: [u8; 32],
    ) -> Self {
        Self {
            id,
            owner,
            uploaded_by,
            uploaded_at,
            filename,
            image_type,
            byte_size,
            sha256,
        }
    }

    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn owner(&self) -> &AttachmentOwner {
        &self.owner
    }
    pub fn uploaded_by(&self) -> &str {
        &self.uploaded_by
    }
    pub fn uploaded_at(&self) -> DateTime<Utc> {
        self.uploaded_at
    }
    pub fn filename(&self) -> &str {
        &self.filename
    }
    pub fn image_type(&self) -> ImageType {
        self.image_type
    }
    pub fn byte_size(&self) -> i64 {
        self.byte_size
    }
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
    pub fn is_claimed(&self) -> bool {
        self.owner != AttachmentOwner::Unclaimed
    }
    /// Who may see it (kb/ADR-0023 R6): anyone once claimed, only its uploader before.
    pub fn visible_to(&self, viewer: &str) -> bool {
        self.is_claimed() || self.uploaded_by == viewer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_accepts_the_three_types_by_their_bytes() {
        assert_eq!(
            ImageType::sniff(b"\x89PNG\r\n\x1a\n rest"),
            Some(ImageType::Png)
        );
        assert_eq!(
            ImageType::sniff(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(ImageType::Jpeg)
        );
        assert_eq!(
            ImageType::sniff(b"RIFF\x10\0\0\0WEBPVP8 "),
            Some(ImageType::Webp)
        );
    }

    #[test]
    fn sniff_refuses_everything_else() {
        for b in [
            &b"<svg xmlns='http://www.w3.org/2000/svg'/>"[..],
            b"<!doctype html><script>x</script>",
            b"GIF89a\x01\0\x01\0",
            b"",
            b"\x89PN",
            b"RIFF\x10\0\0\0WAVEfmt ",
        ] {
            assert_eq!(ImageType::sniff(b), None, "{b:?}");
        }
    }

    #[test]
    fn mime_round_trips() {
        for t in [ImageType::Png, ImageType::Jpeg, ImageType::Webp] {
            assert_eq!(ImageType::from_mime(t.mime()), Some(t));
        }
        assert_eq!(ImageType::from_mime("image/svg+xml"), None);
    }

    #[test]
    fn filenames_lose_paths_and_control_characters() {
        assert_eq!(clean_filename("../../etc/pa\u{0}ss.png"), "pass.png");
        assert_eq!(clean_filename("C:\\Users\\me\\shot.png"), "shot.png");
        assert_eq!(clean_filename("line\nbreak.png"), "linebreak.png");
        assert_eq!(clean_filename(""), "image");
        assert_eq!(clean_filename("  "), "image");
        assert_eq!(clean_filename("dir/"), "image");
        assert_eq!(clean_filename(".."), "image");
    }

    #[test]
    fn long_filenames_are_cut_and_keep_their_extension() {
        let long = format!("{}.png", "a".repeat(300));
        let c = clean_filename(&long);
        assert_eq!(c.chars().count(), 200);
        assert!(c.ends_with(".png"));
        let no_ext = "b".repeat(300);
        assert_eq!(clean_filename(&no_ext).chars().count(), 200);
    }

    #[test]
    fn chunking_splits_at_the_chunk_size() {
        assert_eq!(chunks(&[]).count(), 0);
        assert_eq!(chunks(&vec![0u8; CHUNK]).count(), 1);
        let sizes: Vec<usize> = chunks(&vec![0u8; CHUNK + 1]).map(<[u8]>::len).collect();
        assert_eq!(sizes, vec![CHUNK, 1]);
    }

    #[test]
    fn visibility_follows_ownership() {
        let now = Utc::now();
        let a = Attachment::new(Uuid::nil(), "a@x", now, "s.png", ImageType::Png, b"x");
        assert!(a.visible_to("a@x"));
        assert!(!a.visible_to("b@x"));
        let claimed = Attachment::from_persisted(
            a.id(),
            AttachmentOwner::Item(ItemType::Story, "GRA-1".parse().unwrap()),
            "a@x".into(),
            now,
            "s.png".into(),
            ImageType::Png,
            1,
            a.sha256(),
        );
        assert!(claimed.visible_to("b@x"));
    }

    #[test]
    fn unreadable_images_are_refused() {
        assert_eq!(
            strip_metadata(
                ImageType::Png,
                Bytes::from_static(b"\x89PNG\r\n\x1a\ngarbage")
            ),
            Err(UploadError::Unreadable)
        );
    }
}
