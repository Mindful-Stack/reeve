//! `strip_metadata` (kb/ADR-0023 R3) over each metadata carrier of each accepted type. The fixtures are built here:
//! a small image encoded by `image`, with EXIF, XMP and PNG text injected through `img-parts`. The oracle is a
//! container walker written in this file, independent of the code under test, and pixels are compared by decoding.
use bytes::Bytes;
use image::{ImageFormat, Rgba, RgbaImage};
use img_parts::{ImageEXIF, jpeg::Jpeg, jpeg::JpegSegment, png::Png, png::PngChunk};
use reeve_planning::{ImageType, strip_metadata};
use std::io::Cursor;

const EXIF: &[u8] = b"MM\0*\0\0\0\x08\0\x01\x01\x10\0\x02\0\0\0\x0aTestPhone\0\0\0\0\0";
/// Every planted value; none may be left anywhere in a stripped file, whatever parser looks.
const PLANTED: [&[u8]; 5] = [
    b"TestPhone",
    b"Tester",
    b"secret",
    b"ns.adobe.com",
    b"MPF\0",
];
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
    // Extended XMP, a comment and an MPF index, each carrying a planted value.
    let mut ext = b"http://ns.adobe.com/xmp/extension/\0".to_vec();
    ext.extend_from_slice(b"Tester extended");
    j.segments_mut()
        .insert(1, JpegSegment::new_with_contents(0xE1, Bytes::from(ext)));
    j.segments_mut().insert(
        1,
        JpegSegment::new_with_contents(0xFE, Bytes::from_static(b"secret comment")),
    );
    j.segments_mut().insert(
        1,
        JpegSegment::new_with_contents(0xE2, Bytes::from_static(b"MPF\0secret index")),
    );
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
    // A trailer after the end of the image, as motion photos and secondary images have.
    let mut b = j.encoder().bytes().to_vec();
    b.extend_from_slice(b"secret trailer TestPhone");
    Bytes::from(b)
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
    use img_parts::ImageICC;
    use img_parts::riff::{RiffChunk, RiffContent};
    let mut w = img_parts::webp::WebP::from_bytes(encode(ImageFormat::WebP)).unwrap();
    // An ICC profile keeps the extended header (VP8X) after the strip, so its flags are really checked.
    w.set_icc_profile(Some(Bytes::from_static(b"icc profile bytes")));
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

fn assert_nothing_planted(out: &[u8]) {
    for p in PLANTED {
        assert!(
            !out.windows(p.len()).any(|w| w == p),
            "{:?} left",
            String::from_utf8_lossy(p)
        );
    }
}

/// A big-endian EXIF block with the camera model and an orientation.
fn exif_with_orientation(o: u16) -> Bytes {
    let mut v = b"MM\0*\0\0\0\x08\0\x02".to_vec();
    v.extend_from_slice(&[0x01, 0x10, 0, 2, 0, 0, 0, 4]);
    v.extend_from_slice(b"TPh\0"); // a model short enough to sit in the entry
    v.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1]);
    v.extend_from_slice(&o.to_be_bytes());
    v.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    Bytes::from(v)
}

/// The compressed image data, which a strip must leave byte for byte: PNG's IDAT contents, the JPEG scan up to its
/// end marker, the WebP bitstream chunk. Decoded pixels alone could not tell a lossless re-encode from a strip.
fn payload(t: ImageType, b: &[u8]) -> Vec<u8> {
    match t {
        ImageType::Png => Png::from_bytes(Bytes::copy_from_slice(b))
            .unwrap()
            .chunks_by_type(*b"IDAT")
            .flat_map(|c| c.contents().to_vec())
            .collect(),
        ImageType::Jpeg => {
            let sos = b.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
            let eoi = b[sos..].windows(2).position(|w| w == [0xFF, 0xD9]).unwrap();
            b[sos..sos + eoi + 2].to_vec()
        }
        ImageType::Webp => img_parts::webp::WebP::from_bytes(Bytes::copy_from_slice(b))
            .unwrap()
            .chunks()
            .iter()
            .filter(|c| matches!(&c.id(), b"VP8 " | b"VP8L"))
            .flat_map(|c| c.content().data().unwrap().to_vec())
            .collect(),
    }
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
    assert!(j.iter().any(|(m, _)| *m == 0xFE));
    assert!(j.iter().any(|(m, c)| *m == 0xE2 && c.starts_with(b"MPF\0")));
    assert!(
        jpeg_with(false, false, false).ends_with(b"TestPhone"),
        "the trailer is planted"
    );
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
    const T: ImageType = ImageType::Jpeg;
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
    assert!(
        out.ends_with(&[0xFF, 0xD9]),
        "nothing after the end of the image"
    );
    assert_nothing_planted(&out);
    assert_eq!(payload(T, &input), payload(T, &out), "image data changed");
    // The planted extended XMP is not well-formed for decoders, so compare with the clean encoding instead.
    assert_eq!(pixels(&encode(ImageFormat::Jpeg)), pixels(&out));
}

#[test]
fn png_loses_every_metadata_chunk_and_keeps_its_pixels() {
    const T: ImageType = ImageType::Png;
    let input = png_with_metadata();
    let out = strip_metadata(ImageType::Png, input.clone()).unwrap();
    let kinds = png_chunks(&out);
    for k in [b"eXIf", b"tEXt", b"zTXt", b"iTXt"] {
        assert!(!kinds.contains(k), "{k:?} left");
    }
    assert!(kinds.contains(b"IDAT") && kinds.contains(b"IEND"));
    assert_nothing_planted(&out);
    assert_eq!(payload(T, &input), payload(T, &out), "image data changed");
    assert_eq!(pixels(&input), pixels(&out));
}

#[test]
fn webp_loses_exif_and_xmp_and_their_header_flags_and_keeps_its_pixels() {
    const T: ImageType = ImageType::Webp;
    let input = webp_with_metadata();
    let out = strip_metadata(ImageType::Webp, input.clone()).unwrap();
    let (ids, flags) = webp_chunks(&out);
    assert!(!ids.contains(b"EXIF") && !ids.contains(b"XMP "), "{ids:?}");
    // A VP8X header that still names EXIF or XMP makes strict decoders reject the file.
    let f = flags.expect("the ICC profile keeps VP8X");
    assert_eq!(f & 0x0C, 0, "VP8X flags still name metadata: {f:#x}");
    assert_eq!(f & 0x20, 0x20, "the ICC flag is kept");
    assert!(ids.contains(b"ICCP"));
    assert_nothing_planted(&out);
    assert_eq!(payload(T, &input), payload(T, &out), "image data changed");
    assert_eq!(pixels(&input), pixels(&out));
}

#[test]
fn a_clean_image_comes_back_unchanged() {
    let png = encode(ImageFormat::Png);
    assert_eq!(strip_metadata(ImageType::Png, png.clone()).unwrap(), png);
}

#[test]
fn the_orientation_survives_and_nothing_else_of_the_exif_does() {
    for t in [ImageType::Jpeg, ImageType::Webp] {
        let input = match t {
            ImageType::Jpeg => {
                let mut j = Jpeg::from_bytes(encode(ImageFormat::Jpeg)).unwrap();
                j.set_exif(Some(exif_with_orientation(6)));
                j.encoder().bytes()
            }
            _ => {
                let mut w = img_parts::webp::WebP::from_bytes(encode(ImageFormat::WebP)).unwrap();
                w.set_exif(Some(exif_with_orientation(6)));
                w.encoder().bytes()
            }
        };
        let out = strip_metadata(t, input).unwrap();
        let exif = match t {
            ImageType::Jpeg => Jpeg::from_bytes(out.clone()).unwrap().exif(),
            _ => img_parts::webp::WebP::from_bytes(out.clone())
                .unwrap()
                .exif(),
        }
        .expect("an orientation block");
        // Exactly one IFD entry, the orientation, with the value 6.
        assert_eq!(&exif[8..10], &[0, 1], "{t:?}");
        assert_eq!(&exif[10..12], &[0x01, 0x12], "{t:?}");
        assert_eq!(&exif[18..20], &[0, 6], "{t:?}");
        assert!(
            !out.windows(3).any(|w| w == b"TPh"),
            "{t:?}: the model is left"
        );
    }
}

#[test]
fn an_upright_orientation_leaves_no_exif_at_all() {
    let mut j = Jpeg::from_bytes(encode(ImageFormat::Jpeg)).unwrap();
    j.set_exif(Some(exif_with_orientation(1)));
    let out = strip_metadata(ImageType::Jpeg, j.encoder().bytes()).unwrap();
    assert!(Jpeg::from_bytes(out).unwrap().exif().is_none());
}

/// A JPEG segment: marker and payload, with its length.
fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0xFF, marker];
    v.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    v.extend_from_slice(payload);
    v
}

#[test]
fn jpeg_metadata_after_the_scan_and_unknown_carriers_go_and_a_stray_end_marker_does_not_cut_the_image()
 {
    let clean = encode(ImageFormat::Jpeg);
    let eoi = clean.len() - 2;
    assert_eq!(&clean[eoi..], &[0xFF, 0xD9]);
    let mut b = clean[..2].to_vec();
    // Before the frame: a JFXX thumbnail and a C2PA/JUMBF manifest (APP11).
    b.extend(segment(0xE0, b"JFXX\0secret thumbnail"));
    b.extend(segment(0xEB, b"JP\0\0jumbf TestPhone"));
    b.extend_from_slice(&clean[2..eoi]);
    // After the scan, before the end: what a progressive file may carry between scans, one segment holding FF D9.
    b.extend(segment(0xE1, b"Exif\0\0MM\0*TestPhone"));
    b.extend(segment(0xFE, b"secret comment"));
    b.extend(segment(
        0xEF,
        &[b's', b'e', b'c', b'r', b'e', b't', 0xFF, 0xD9, 1, 2],
    ));
    b.extend_from_slice(&[0xFF, 0xD9]);
    b.extend_from_slice(b"secret trailer");
    let out = strip_metadata(ImageType::Jpeg, Bytes::from(b)).unwrap();
    assert_nothing_planted(&out);
    for (m, _) in jpeg_segments(&out) {
        assert!(
            matches!(m, 0xC0..=0xCF | 0xDA..=0xDD | 0xE0 | 0xE2 | 0xEE),
            "marker {m:#x} kept"
        );
    }
    assert!(out.ends_with(&[0xFF, 0xD9]));
    assert_eq!(
        payload(ImageType::Jpeg, &out),
        payload(ImageType::Jpeg, &clean),
        "image data changed"
    );
    assert_eq!(pixels(&out), pixels(&clean));
}

#[test]
fn png_and_webp_keep_only_what_draws_the_image() {
    let mut p = Png::from_bytes(encode(ImageFormat::Png)).unwrap();
    for (k, v) in [
        (*b"tIME", &b"\x07\xea\x0b\x0a\x09\0\0"[..]),
        (*b"caBX", b"c2pa TestPhone"),
        (*b"prVt", b"secret"),
    ] {
        p.chunks_mut()
            .insert(1, PngChunk::new(k, Bytes::copy_from_slice(v)));
    }
    let out = strip_metadata(ImageType::Png, p.encoder().bytes()).unwrap();
    let kinds = png_chunks(&out);
    for k in [b"tIME", b"caBX", b"prVt"] {
        assert!(!kinds.contains(k), "{k:?} left");
    }
    assert_nothing_planted(&out);

    use img_parts::riff::{RiffChunk, RiffContent};
    let mut w = img_parts::webp::WebP::from_bytes(encode(ImageFormat::WebP)).unwrap();
    w.chunks_mut().push(RiffChunk::new(
        *b"abcd",
        RiffContent::Data(Bytes::from_static(b"secret")),
    ));
    let out = strip_metadata(ImageType::Webp, w.encoder().bytes()).unwrap();
    assert!(!webp_chunks(&out).0.contains(b"abcd"));
    assert_nothing_planted(&out);
    assert_eq!(pixels(&out), pixels(&encode(ImageFormat::WebP)));
}

#[test]
fn a_container_with_no_image_in_it_is_unreadable() {
    for (t, b) in [
        (ImageType::Jpeg, &[0xFF, 0xD8, 0xFF, 0xD9][..]),
        (
            ImageType::Jpeg,
            &[0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x04, 0, 0][..],
        ),
        (ImageType::Png, b"\x89PNG\r\n\x1a\n"),
    ] {
        assert!(
            strip_metadata(t, Bytes::copy_from_slice(b)).is_err(),
            "{t:?} {b:?}"
        );
    }
}

#[test]
fn clean_jpeg_and_webp_come_back_byte_for_byte() {
    for (t, f) in [
        (ImageType::Jpeg, ImageFormat::Jpeg),
        (ImageType::Webp, ImageFormat::WebP),
    ] {
        let b = encode(f);
        assert_eq!(strip_metadata(t, b.clone()).unwrap(), b, "{t:?}");
    }
}

#[test]
fn a_multi_scan_jpeg_with_restarts_and_fill_bytes_is_walked_to_its_end() {
    // The walker never decodes: a synthetic progressive-shaped stream is enough.
    let mut b = vec![0xFF, 0xD8];
    b.extend(segment(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
    b.extend(segment(0xC2, &[8, 0, 1, 0, 1, 1, 1, 0x11, 0]));
    b.extend(segment(
        0xC4,
        &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ));
    b.extend(segment(0xDA, &[1, 1, 0, 0, 63, 0]));
    b.extend_from_slice(&[
        0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56, 0xFF, 0xFF, 0xD1, 0x78,
    ]);
    b.extend(segment(0xFE, b"secret between scans"));
    b.extend(segment(
        0xC4,
        &[0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ));
    b.extend(segment(0xDA, &[1, 1, 0, 1, 63, 0]));
    b.extend_from_slice(&[0x9A, 0xBC]);
    b.extend_from_slice(&[0xFF, 0xD9]);
    let out = strip_metadata(ImageType::Jpeg, Bytes::from(b.clone())).unwrap();
    assert_nothing_planted(&out);
    // Everything but the comment, byte for byte, the stuffed bytes and restarts of the first scan included.
    let mut expected = b.clone();
    let com = segment(0xFE, b"secret between scans");
    let at = expected
        .windows(com.len())
        .position(|w| w == com.as_slice())
        .unwrap();
    expected.drain(at..at + com.len());
    // The walker drops a fill byte before a marker inside a scan only as part of the entropy run: compare as is.
    assert_eq!(out.as_ref(), expected.as_slice());
}

#[test]
fn a_phone_layout_exif_first_keeps_one_orientation_block_right_after_soi() {
    let clean = encode(ImageFormat::Jpeg);
    let mut j = Jpeg::from_bytes(clean.clone()).unwrap();
    j.segments_mut().retain(|s| s.marker() != 0xE0); // no JFIF, as many cameras write
    let le = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    j.set_exif(Some(Bytes::from_static(le)));
    let out = strip_metadata(ImageType::Jpeg, j.encoder().bytes()).unwrap();
    let segs = jpeg_segments(&out);
    assert_eq!(segs[0].0, 0xE1, "the orientation block comes first");
    assert_eq!(segs.iter().filter(|(m, _)| *m == 0xE1).count(), 1);
    let exif = Jpeg::from_bytes(out.clone()).unwrap().exif().unwrap();
    assert_eq!(&exif[18..20], &[0, 6]);
    assert_eq!(pixels(&out), pixels(&clean));
}

#[test]
fn containers_without_a_bitstream_are_unreadable() {
    let mut p = Png::from_bytes(encode(ImageFormat::Png)).unwrap();
    p.chunks_mut().retain(|c| c.kind() != *b"IDAT");
    assert!(strip_metadata(ImageType::Png, p.encoder().bytes()).is_err());
    let mut w = img_parts::webp::WebP::from_bytes(encode(ImageFormat::WebP)).unwrap();
    w.chunks_mut()
        .retain(|c| !matches!(&c.id(), b"VP8 " | b"VP8L"));
    assert!(strip_metadata(ImageType::Webp, w.encoder().bytes()).is_err());
}

#[test]
fn every_prefix_of_every_fixture_is_refused_or_stripped_and_never_panics() {
    // Untrusted input through hand-written indexing: a truncated file must answer, not crash the server.
    for (t, b) in [
        (ImageType::Jpeg, jpeg_with(true, true, true)),
        (ImageType::Png, png_with_metadata()),
        (ImageType::Webp, webp_with_metadata()),
    ] {
        for n in 0..b.len() {
            let _ = strip_metadata(t, b.slice(..n));
        }
    }
}
