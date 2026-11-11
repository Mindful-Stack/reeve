//! Attachments (kb/ADR-0022, kb/ADR-0023): the rules an upload passes before it is stored, and the attachment
//! itself. Pure: bytes and the handler's clock in, the stored form out. Nothing here reads the clock or the database.
use bytes::Bytes;
use chrono::{DateTime, Utc};
use img_parts::{ImageEXIF, png::Png, webp::WebP};
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

/// The PNG chunks kept: the critical ones, and the ancillary ones that only say how to show the pixels (transparency,
/// colour, gamma, density, animation). Every other chunk (text, EXIF, time, C2PA, anything unknown) is dropped.
const PNG_KEEP: [[u8; 4]; 16] = [
    *b"IHDR", *b"PLTE", *b"IDAT", *b"IEND", *b"tRNS", *b"gAMA", *b"cHRM", *b"sRGB", *b"iCCP",
    *b"sBIT", *b"bKGD", *b"pHYs", *b"cICP", *b"acTL", *b"fcTL", *b"fdAT",
];
/// The WebP chunks kept: the bitstream, alpha, animation, the colour profile and the extended header.
const WEBP_KEEP: [[u8; 4]; 7] = [
    *b"VP8 ", *b"VP8L", *b"VP8X", *b"ALPH", *b"ANIM", *b"ANMF", *b"ICCP",
];
/// The VP8X flag that says an XMP chunk follows.
const WEBP_XMP_FLAG: u8 = 0b0000_0100;
const ORIENTATION: u16 = 0x0112;

/// Keeps only what draws the image, without decoding or re-encoding pixels (kb/ADR-0023 R3): every metadata carrier,
/// known or not (EXIF, XMP, IPTC, comments, MPF, thumbnails, C2PA manifests, PNG text and time, data past the end), is
/// dropped by allowlist. Only the EXIF orientation survives, as a one-tag EXIF block, so a rotated photo still displays
/// upright. A file with no image in it is unreadable.
pub fn strip_metadata(t: ImageType, b: Bytes) -> Result<Bytes, UploadError> {
    let bad = |_| UploadError::Unreadable;
    Ok(match t {
        ImageType::Jpeg => strip_jpeg(&b)?,
        ImageType::Png => {
            let mut p = Png::from_bytes(b).map_err(bad)?;
            let o = p.exif().and_then(|e| orientation(&e));
            p.chunks_mut().retain(|c| PNG_KEEP.contains(&c.kind()));
            let has = |k: &[u8; 4]| p.chunks().iter().any(|c| &c.kind() == k);
            if !(has(b"IHDR") && has(b"IDAT") && has(b"IEND")) {
                return Err(UploadError::Unreadable);
            }
            // Right after IHDR, before the image data, as an eXIf chunk must be.
            p.set_exif(o.map(orientation_exif));
            p.encoder().bytes()
        }
        ImageType::Webp => {
            let mut w = WebP::from_bytes(b).map_err(bad)?;
            let o = w.exif().and_then(|e| orientation(&e));
            w.chunks_mut().retain(|c| WEBP_KEEP.contains(&c.id()));
            // An animation frame carries chunks of its own: only its bitstream and alpha are kept.
            for c in w.chunks_mut() {
                if c.id() == *b"ANMF"
                    && let img_parts::riff::RiffContent::Data(d) = c.content_mut()
                {
                    *d = webp_frame(d).ok_or(UploadError::Unreadable)?;
                }
            }
            if !w
                .chunks()
                .iter()
                .any(|c| matches!(&c.id(), b"VP8 " | b"VP8L" | b"ANMF"))
            {
                return Err(UploadError::Unreadable);
            }
            // set_exif rewrites the VP8X ICC and EXIF flags from the chunks left (or drops VP8X when nothing needs
            // it); the XMP flag it leaves alone, so that one is cleared by hand.
            w.set_exif(o.map(orientation_exif));
            for c in w.chunks_mut() {
                if c.id() == img_parts::webp::CHUNK_VP8X
                    && let img_parts::riff::RiffContent::Data(d) = c.content_mut()
                    && let Some(first) = d.first().copied()
                {
                    let mut v = d.to_vec();
                    v[0] = first & !WEBP_XMP_FLAG;
                    *d = Bytes::from(v);
                }
            }
            w.encoder().bytes()
        }
    })
}

/// A JPEG rebuilt from the markers that draw it, walked through every scan to the end-of-image marker (a progressive
/// file has tables and, legally, metadata between its scans). Kept: frame and scan headers with their entropy-coded
/// data, tables (DHT, DQT, DAC, DRI, DNL), APP0 `JFIF`, APP2 `ICC_PROFILE` and APP14 `Adobe`. Dropped: every other
/// APPn (EXIF, XMP, MPF, JFXX thumbnails, IPTC, C2PA), comments, and anything after the end marker.
fn strip_jpeg(b: &[u8]) -> Result<Bytes, UploadError> {
    let bad = UploadError::Unreadable;
    if !b.starts_with(&[0xFF, 0xD8]) {
        return Err(bad);
    }
    let exif = jpeg_app1_exif(b);
    let mut out: Vec<u8> = vec![0xFF, 0xD8];
    let mut orientation_written = false;
    let (mut frame, mut scan) = (false, false);
    let mut i = 2;
    loop {
        // Fill bytes before a marker are allowed.
        while b.get(i) == Some(&0xFF) && b.get(i + 1) == Some(&0xFF) {
            i += 1;
        }
        if b.get(i) != Some(&0xFF) {
            return Err(bad);
        }
        let m = *b.get(i + 1).ok_or(bad)?;
        if m == 0xD9 {
            out.extend_from_slice(&[0xFF, 0xD9]);
            break;
        }
        // Standalone markers with no length.
        if (0xD0..=0xD7).contains(&m) || m == 0x01 {
            out.extend_from_slice(&b[i..i + 2]);
            i += 2;
            continue;
        }
        let len =
            u16::from_be_bytes([*b.get(i + 2).ok_or(bad)?, *b.get(i + 3).ok_or(bad)?]) as usize;
        if len < 2 {
            return Err(bad);
        }
        let end = i + 2 + len;
        let seg = b.get(i..end).ok_or(bad)?;
        let payload = &seg[4..];
        // What of the segment is kept: all of it, a rebuilt form of it, or nothing.
        let kept: Option<std::borrow::Cow<[u8]>> = match m {
            0xC0..=0xCF => {
                if m != 0xC4 && m != 0xC8 && m != 0xCC {
                    frame = true;
                }
                Some(seg.into())
            }
            0xDA..=0xDD => Some(seg.into()),
            // JFIF: version, units and density only; its thumbnail (an unredacted preview) and anything after it go.
            0xE0 if payload.len() >= 12 && payload.starts_with(b"JFIF\0") => {
                let mut v = vec![0xFF, 0xE0, 0, 16];
                v.extend_from_slice(&payload[..12]);
                v.extend_from_slice(&[0, 0]);
                Some(v.into())
            }
            0xE2 => payload.starts_with(b"ICC_PROFILE\0").then_some(seg.into()),
            // Adobe: its fixed twelve bytes (the colour transform), nothing after them.
            0xEE if payload.len() >= 12 && payload.starts_with(b"Adobe") => {
                let mut v = vec![0xFF, 0xEE, 0, 14];
                v.extend_from_slice(&payload[..12]);
                Some(v.into())
            }
            _ => None,
        };
        let keep = kept.is_some();
        // The orientation block goes right after SOI and a JFIF APP0, before anything else.
        if !orientation_written && !(m == 0xE0 && keep) {
            if let Some(o) = exif {
                let tiff = orientation_exif(o);
                out.extend_from_slice(&[0xFF, 0xE1]);
                out.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
                out.extend_from_slice(b"Exif\0\0");
                out.extend_from_slice(&tiff);
            }
            orientation_written = true;
        }
        if let Some(k) = kept {
            out.extend_from_slice(&k);
        }
        i = end;
        if m == 0xDA {
            scan = true;
            // Entropy-coded data runs to the next marker that is not a stuffed byte (FF 00), a fill byte or a restart.
            let start = i;
            while i + 1 < b.len() {
                if b[i] == 0xFF
                    && b[i + 1] != 0x00
                    && b[i + 1] != 0xFF
                    && !(0xD0..=0xD7).contains(&b[i + 1])
                {
                    break;
                }
                i += 1;
            }
            if i + 1 >= b.len() {
                // A file cut short in its last scan, which browsers still draw: its data is kept and the end marker
                // added.
                out.extend_from_slice(&b[start..]);
                if !frame {
                    return Err(bad);
                }
                out.extend_from_slice(&[0xFF, 0xD9]);
                return Ok(Bytes::from(out));
            }
            out.extend_from_slice(&b[start..i]);
        }
    }
    if !(frame && scan) {
        return Err(bad);
    }
    Ok(Bytes::from(out))
}

/// An ANMF payload with only the chunks that draw the frame: its 16-byte header, then ALPH, `VP8 ` and VP8L (padded
/// to even lengths, as RIFF requires); None when its chunk list does not parse.
fn webp_frame(d: &[u8]) -> Option<Bytes> {
    let mut out = d.get(..16)?.to_vec();
    let mut i = 16;
    while i < d.len() {
        let id: [u8; 4] = d.get(i..i + 4)?.try_into().ok()?;
        let len = u32::from_le_bytes(d.get(i + 4..i + 8)?.try_into().ok()?) as usize;
        let end = (i + 8).checked_add(len)?;
        d.get(i + 8..end)?;
        if matches!(&id, b"ALPH" | b"VP8 " | b"VP8L") {
            out.extend_from_slice(&d[i..end]);
            if len % 2 == 1 {
                out.push(0);
            }
        }
        i = end + len % 2;
    }
    Some(Bytes::from(out))
}

/// The non-upright orientation of a JPEG's EXIF (APP1 `Exif\0\0`) before its first scan, if it has one.
fn jpeg_app1_exif(b: &[u8]) -> Option<u16> {
    let mut i = 2;
    while i + 4 <= b.len() && b[i] == 0xFF {
        let m = b[i + 1];
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let seg = b.get(i + 4..i + 2 + len)?;
        if m == 0xE1 && seg.starts_with(b"Exif\0\0") {
            return orientation(&seg[6..]);
        }
        if m == 0xDA {
            return None;
        }
        i += 2 + len;
    }
    None
}

/// The orientation tag of a TIFF-structured EXIF block's first IFD, when it names a rotation or flip (2 to 8).
fn orientation(tiff: &[u8]) -> Option<u16> {
    let be = match tiff.get(..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |i: usize| -> Option<u16> {
        let b: [u8; 2] = tiff.get(i..i + 2)?.try_into().ok()?;
        Some(if be {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        })
    };
    let u32_at = |i: usize| -> Option<u32> {
        let b: [u8; 4] = tiff.get(i..i + 4)?.try_into().ok()?;
        Some(if be {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    };
    let ifd = u32_at(4)? as usize;
    let n = u16_at(ifd)? as usize;
    (0..n.min(256))
        .map(|k| ifd + 2 + 12 * k)
        .find(|&e| u16_at(e) == Some(ORIENTATION))
        .and_then(|e| u16_at(e + 8))
        .filter(|o| (2..=8).contains(o))
}

/// A one-tag EXIF block (big-endian TIFF) holding only an orientation.
fn orientation_exif(o: u16) -> Bytes {
    let mut v = b"MM\0*\0\0\0\x08\0\x01".to_vec();
    v.extend_from_slice(&ORIENTATION.to_be_bytes());
    v.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    v.extend_from_slice(&1u32.to_be_bytes()); // one value
    v.extend_from_slice(&o.to_be_bytes());
    v.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // padding, then no next IFD
    Bytes::from(v)
}

/// A filename safe to store and to put in `Content-Disposition`: the last path segment, without control characters,
/// at most 200 characters (keeping a short extension), and `image` when nothing is left.
pub fn clean_filename(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let name: String = last
        .chars()
        .filter(|c| !c.is_control() && !invisible(*c))
        .collect();
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

/// Format characters and line and paragraph separators (Unicode Cf, Zl, Zp): bidirectional overrides that make a name
/// read differently from what it is, zero-width characters, and line breaks.
fn invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
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
    fn filenames_lose_invisible_and_direction_characters() {
        // "invoice\u{202E}gnp.exe" reads as "invoiceexe.png".
        assert_eq!(clean_filename("invoice\u{202E}gnp.exe"), "invoicegnp.exe");
        assert_eq!(
            clean_filename("a\u{200B}b\u{FEFF}c\u{2066}d.png"),
            "abcd.png"
        );
        assert_eq!(clean_filename("one\u{2028}two\u{2029}.png"), "onetwo.png");
        assert_eq!(clean_filename("\u{200B}"), "image");
        assert_eq!(clean_filename("skärmbild ✓.png"), "skärmbild ✓.png");
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

#[cfg(test)]
mod orientation_tests {
    use super::*;

    #[test]
    fn orientation_is_read_in_either_byte_order_and_written_back() {
        assert_eq!(orientation(&orientation_exif(6)), Some(6));
        let le = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x03\0\0\0\0\0\0\0";
        assert_eq!(orientation(le), Some(3));
        // 1 is upright: nothing to keep.
        assert_eq!(orientation(&orientation_exif(1)), None);
        assert_eq!(orientation(b"junk"), None);
        assert_eq!(orientation(b"MM\0*\xff\xff\xff\xff"), None);
    }
}
