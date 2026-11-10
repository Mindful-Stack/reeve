//! `strip_metadata` (kb/ADR-0023 R3) over each metadata carrier of each accepted type. The fixtures are built here:
//! a small image encoded by `image`, with EXIF, XMP and PNG text injected through `img-parts`. The oracle is a
//! container walker written in this file, independent of the code under test, and pixels are compared by decoding.
use bytes::Bytes;
use image::{ImageFormat, Rgba, RgbaImage};
use img_parts::{ImageEXIF, jpeg::Jpeg, jpeg::JpegSegment, png::Png, png::PngChunk};
use reeve_planning::{ImageType, strip_metadata};
use std::io::Cursor;

const EXIF: &[u8] = b"MM\0*\0\0\0\x08\0\x01\x01\x10\0\x02\0\0\0\x0aTestPhone\0\0\0\0\0";
const XMP: &[u8] =
    b"<x:xmpmeta xmlns:x='adobe:ns:meta/'><dc:creator>Tester</dc:creator></x:xmpmeta>";

fn picture() -> RgbaImage {
    RgbaImage::from_fn(64, 48, |x, y| {
        Rgba([(x * 4) as u8, (y * 5) as u8, 128, 255])
    })
}

fn encode(f: ImageFormat) -> Bytes {
    let mut out = Cursor::new(Vec::new());
    let img = image::DynamicImage::ImageRgba8(picture());
    match f {
        ImageFormat::Jpeg => img.to_rgb8().write_to(&mut out, f).unwrap(),
        _ => img.write_to(&mut out, f).unwrap(),
    }
    Bytes::from(out.into_inner())
}

fn pixels(b: &[u8]) -> Vec<u8> {
    image::load_from_memory(b).unwrap().to_rgba8().into_raw()
}

// ---- the oracle: list the containers' parts by hand ----

/// JPEG markers with their payloads, up to the start of scan.
fn jpeg_segments(b: &[u8]) -> Vec<(u8, Vec<u8>)> {
    assert_eq!(&b[..2], &[0xFF, 0xD8]);
    let mut i = 2;
    let mut out = Vec::new();
    while i + 4 <= b.len() {
        assert_eq!(b[i], 0xFF, "marker expected at {i}");
        let m = b[i + 1];
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        out.push((m, b[i + 4..i + 2 + len].to_vec()));
        if m == 0xDA {
            break;
        }
        i += 2 + len;
    }
    out
}

/// PNG chunk kinds in order.
fn png_chunks(b: &[u8]) -> Vec<[u8; 4]> {
    assert_eq!(&b[..8], b"\x89PNG\r\n\x1a\n");
    let mut i = 8;
    let mut out = Vec::new();
    while i + 8 <= b.len() {
        let len = u32::from_be_bytes(b[i..i + 4].try_into().unwrap()) as usize;
        out.push(b[i + 4..i + 8].try_into().unwrap());
        i += 12 + len;
    }
    out
}

/// WebP chunk ids in order, and the VP8X flags byte when there is one.
fn webp_chunks(b: &[u8]) -> (Vec<[u8; 4]>, Option<u8>) {
    assert_eq!(&b[..4], b"RIFF");
    assert_eq!(&b[8..12], b"WEBP");
    let mut i = 12;
    let mut ids = Vec::new();
    let mut flags = None;
    while i + 8 <= b.len() {
        let id: [u8; 4] = b[i..i + 4].try_into().unwrap();
        let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
        if &id == b"VP8X" {
            flags = Some(b[i + 8]);
        }
        ids.push(id);
        i += 8 + len + (len & 1);
    }
    (ids, flags)
}

// ---- the fixtures ----

fn jpeg_with(exif: bool, xmp: bool, iptc: bool) -> Bytes {
    let mut j = Jpeg::from_bytes(encode(ImageFormat::Jpeg)).unwrap();
    if exif {
        j.set_exif(Some(Bytes::from_static(EXIF)));
    }
    if xmp {
        let mut c = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
        c.extend_from_slice(XMP);
        j.segments_mut()
            .insert(1, JpegSegment::new_with_contents(0xE1, Bytes::from(c)));
    }
    if iptc {
        j.segments_mut().insert(
            1,
            JpegSegment::new_with_contents(0xED, Bytes::from_static(b"Photoshop 3.0\0secret")),
        );
    }
    j.encoder().bytes()
}

fn png_with_metadata() -> Bytes {
    let mut p = Png::from_bytes(encode(ImageFormat::Png)).unwrap();
    let at = 1; // after IHDR
    p.chunks_mut()
        .insert(at, PngChunk::new(*b"eXIf", Bytes::from_static(EXIF)));
    p.chunks_mut().insert(
        at,
        PngChunk::new(*b"tEXt", Bytes::from_static(b"Comment\0secret")),
    );
    p.chunks_mut().insert(
        at,
        PngChunk::new(
            *b"zTXt",
            Bytes::from_static(b"Author\0\0x\x9c\x03\0\0\0\0\x01"),
        ),
    );
    let mut itxt = b"XML:com.adobe.xmp\0\0\0\0\0".to_vec();
    itxt.extend_from_slice(XMP);
    p.chunks_mut()
        .insert(at, PngChunk::new(*b"iTXt", Bytes::from(itxt)));
    p.encoder().bytes()
}

fn webp_with_metadata() -> Bytes {
    use img_parts::riff::{RiffChunk, RiffContent};
    let mut w = img_parts::webp::WebP::from_bytes(encode(ImageFormat::WebP)).unwrap();
    // set_exif turns the file into the extended format (VP8X) and sets its EXIF flag.
    w.set_exif(Some(Bytes::from_static(EXIF)));
    w.chunks_mut().push(RiffChunk::new(
        *b"XMP ",
        RiffContent::Data(Bytes::from_static(XMP)),
    ));
    // Set the XMP flag (0x04) by hand, as a writer that adds XMP would.
    let mut b = w.encoder().bytes().to_vec();
    let vp8x = b.windows(4).position(|x| x == b"VP8X").unwrap();
    b[vp8x + 8] |= 0x04;
    Bytes::from(b)
}

// ---- the tests ----

#[test]
fn the_fixtures_carry_their_metadata() {
    // Without this, every strip test below would pass on files that never had metadata.
    let j = jpeg_segments(&jpeg_with(true, true, true));
    assert!(
        j.iter()
            .any(|(m, c)| *m == 0xE1 && c.starts_with(b"Exif\0\0"))
    );
    assert!(
        j.iter()
            .any(|(m, c)| *m == 0xE1 && c.starts_with(b"http://ns.adobe.com/xap/1.0/"))
    );
    assert!(j.iter().any(|(m, _)| *m == 0xED));
    let p = png_chunks(&png_with_metadata());
    for k in [b"eXIf", b"tEXt", b"zTXt", b"iTXt"] {
        assert!(p.contains(k), "{k:?}");
    }
    let (w, flags) = webp_chunks(&webp_with_metadata());
    assert!(w.contains(b"EXIF") && w.contains(b"XMP "));
    assert_eq!(flags.unwrap() & 0x0C, 0x0C);
}

#[test]
fn jpeg_loses_exif_xmp_and_iptc_and_keeps_its_pixels() {
    let input = jpeg_with(true, true, true);
    let out = strip_metadata(ImageType::Jpeg, input.clone()).unwrap();
    for (m, c) in jpeg_segments(&out) {
        assert!(m != 0xED, "APP13 left");
        assert!(
            !(m == 0xE1 && (c.starts_with(b"Exif\0\0") || c.starts_with(b"http://ns.adobe.com/"))),
            "APP1 metadata left"
        );
    }
    assert!(
        jpeg_segments(&out).iter().any(|(m, _)| *m == 0xDA),
        "scan data kept"
    );
    assert_eq!(pixels(&input), pixels(&out));
}

#[test]
fn png_loses_every_metadata_chunk_and_keeps_its_pixels() {
    let input = png_with_metadata();
    let out = strip_metadata(ImageType::Png, input.clone()).unwrap();
    let kinds = png_chunks(&out);
    for k in [b"eXIf", b"tEXt", b"zTXt", b"iTXt"] {
        assert!(!kinds.contains(k), "{k:?} left");
    }
    assert!(kinds.contains(b"IDAT") && kinds.contains(b"IEND"));
    assert_eq!(pixels(&input), pixels(&out));
}

#[test]
fn webp_loses_exif_and_xmp_and_their_header_flags_and_keeps_its_pixels() {
    let input = webp_with_metadata();
    let out = strip_metadata(ImageType::Webp, input.clone()).unwrap();
    let (ids, flags) = webp_chunks(&out);
    assert!(!ids.contains(b"EXIF") && !ids.contains(b"XMP "), "{ids:?}");
    // A VP8X header that still names EXIF or XMP makes strict decoders reject the file.
    if let Some(f) = flags {
        assert_eq!(f & 0x0C, 0, "VP8X flags still name metadata: {f:#x}");
    }
    assert_eq!(pixels(&input), pixels(&out));
}

#[test]
fn a_clean_image_comes_back_unchanged() {
    let png = encode(ImageFormat::Png);
    assert_eq!(strip_metadata(ImageType::Png, png.clone()).unwrap(), png);
}
