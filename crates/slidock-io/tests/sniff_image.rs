//! Magic-byte sniffing and header-only image sizes, including the reads past the head.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

mod support;

use slidock_io::{
    BytesReader, Dpi, ImageSize, Mime, RgbaImage, encode_png, image_size, read_image_size,
    sniff_mime,
};
use support::concat;

fn le16(n: u16) -> Vec<u8> {
    n.to_le_bytes().to_vec()
}
fn le32(n: u32) -> Vec<u8> {
    n.to_le_bytes().to_vec()
}
fn be16(n: u16) -> Vec<u8> {
    n.to_be_bytes().to_vec()
}
fn be32(n: u32) -> Vec<u8> {
    n.to_be_bytes().to_vec()
}

fn bytes(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn size(width: u32, height: u32) -> ImageSize {
    ImageSize {
        width,
        height,
        dpi: None,
        frames: None,
    }
}

fn png_with(extra: &[&[u8]]) -> Vec<u8> {
    let png = encode_png(&RgbaImage {
        width: 3,
        height: 2,
        data: vec![0; 24],
    })
    .unwrap();
    // Insert chunks after IHDR (8-byte signature + 25-byte IHDR chunk).
    let mut parts: Vec<&[u8]> = vec![&png[..33]];
    parts.extend(extra);
    parts.push(&png[33..]);
    concat(&parts)
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    bytes(&[&be32(data.len() as u32), kind, data, &[0, 0, 0, 0]])
}

fn jpeg() -> Vec<u8> {
    bytes(&[
        &[0xff, 0xd8],
        &[0xff, 0xe0],
        &be16(16),
        b"JFIF\0",
        &[1, 1, 1],
        &be16(300),
        &be16(300),
        &[0, 0],
        &[0xff, 0xc0],
        &be16(11),
        &[8],
        &be16(480),
        &be16(640),
        &[1, 1, 0x11, 0],
    ])
}

fn gif(frames: usize) -> Vec<u8> {
    let frame = bytes(&[
        &[0x2c],
        &le16(0),
        &le16(0),
        &le16(4),
        &le16(4),
        &[0],
        &[2, 1, 0, 0],
    ]);
    let mut out = bytes(&[b"GIF89a", &le16(4), &le16(4), &[0x80, 0, 0], &[0; 6]]);
    out.extend([0x21, 0xf9, 4, 0, 0, 0, 0, 0]);
    for _ in 0..frames {
        out.extend(&frame);
    }
    out.push(0x3b);
    out
}

fn webp_vp8x(animated: bool, frames: usize) -> Vec<u8> {
    let anmf = bytes(&[b"ANMF", &le32(2), &[0, 0]]);
    let mut body = bytes(&[
        b"VP8X",
        &le32(10),
        &[if animated { 0x02 } else { 0 }, 0, 0, 0],
        &[99, 0, 0],
        &[49, 0, 0],
    ]);
    for _ in 0..frames {
        body.extend(&anmf);
    }
    bytes(&[b"RIFF", &le32(body.len() as u32 + 4), b"WEBP", &body])
}

fn tiff() -> Vec<u8> {
    bytes(&[
        b"II",
        &le16(42),
        &le32(8),
        &le16(5),
        &[le16(256), le16(3), le32(1), le16(40), vec![0, 0]].concat(),
        &[le16(257), le16(4), le32(1), le32(30)].concat(),
        &[le16(282), le16(5), le32(1), le32(74)].concat(),
        &[le16(283), le16(5), le32(1), le32(82)].concat(),
        &[le16(296), le16(3), le32(1), le16(2), vec![0, 0]].concat(),
        &le32(0),
        &le32(144),
        &le32(1),
        &le32(72),
        &le32(1),
    ])
}

fn bmp() -> Vec<u8> {
    bytes(&[
        b"BM",
        &le32(0),
        &le32(0),
        &le32(54),
        &le32(40),
        &le32(16),
        &le32((-9i32).cast_unsigned()),
        &le16(1),
        &le16(32),
        &le32(0),
        &le32(0),
        &le32(3780),
        &le32(3780),
    ])
}

#[test]
fn recognises_every_type() {
    let cases: Vec<(&str, Vec<u8>, Mime)> = vec![
        (
            "png",
            encode_png(&RgbaImage {
                width: 1,
                height: 1,
                data: vec![0; 4],
            })
            .unwrap(),
            Mime::Png,
        ),
        ("jpeg", jpeg(), Mime::Jpeg),
        ("gif", gif(1), Mime::Gif),
        ("webp", webp_vp8x(false, 0), Mime::Webp),
        ("tiff", tiff(), Mime::Tiff),
        ("bmp", bmp(), Mime::Bmp),
        (
            "svg",
            b"<?xml version=\"1.0\"?>\n<!-- c --><svg xmlns=\"http://www.w3.org/2000/svg\"/>"
                .to_vec(),
            Mime::Svg,
        ),
        ("emf", bytes(&[&le32(1), &[0; 36], b" EMF"]), Mime::Emf),
        ("wmf", vec![0xd7, 0xcd, 0xc6, 0x9a], Mime::Wmf),
        ("pdf", b"%PDF-1.7\n".to_vec(), Mime::Pdf),
        ("zip", vec![0x50, 0x4b, 3, 4], Mime::Zip),
        ("mp4", bytes(&[&be32(24), b"ftypisom", &be32(0)]), Mime::Mp4),
        (
            "mov",
            bytes(&[&be32(20), b"ftypqt  ", &be32(0)]),
            Mime::QuickTime,
        ),
        ("m4v", bytes(&[&be32(20), b"ftypM4V ", &be32(0)]), Mime::M4v),
        (
            "avif",
            bytes(&[&be32(20), b"ftypavif", &be32(0)]),
            Mime::Avif,
        ),
        (
            "webm",
            bytes(&[&[0x1a, 0x45, 0xdf, 0xa3], b"B\x82\x84webm"]),
            Mime::Webm,
        ),
        (
            "mkv",
            bytes(&[&[0x1a, 0x45, 0xdf, 0xa3], b"B\x82\x88matroska"]),
            Mime::Matroska,
        ),
        ("mp3", b"ID3\x04".to_vec(), Mime::Mpeg),
        ("mp3 frame", vec![0xff, 0xfb, 0x90], Mime::Mpeg),
        ("wav", bytes(&[b"RIFF", &le32(4), b"WAVE"]), Mime::Wav),
        ("ttf", vec![0, 1, 0, 0], Mime::Ttf),
        ("otf", b"OTTO".to_vec(), Mime::Otf),
        ("ttc", b"ttcf".to_vec(), Mime::FontCollection),
        ("woff", b"wOFF".to_vec(), Mime::Woff),
        ("woff2", b"wOF2".to_vec(), Mime::Woff2),
        (
            "heic",
            bytes(&[&be32(20), b"ftypheic", &be32(0)]),
            Mime::Heic,
        ),
        ("m4a", bytes(&[&be32(20), b"ftypM4A ", &be32(0)]), Mime::M4a),
        (
            "moov first",
            bytes(&[&be32(20), b"moov", &be32(0), &be32(0)]),
            Mime::QuickTime,
        ),
    ];
    for (name, head, mime) in cases {
        assert_eq!(sniff_mime(&head), Some(mime), "{name}");
    }
    assert_eq!(Mime::Svg.as_str(), "image/svg+xml");
}

/// An `ftyp` box with its major brand, a zero minor version and its compatible brands.
fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
    let brands: Vec<u8> = compatible.iter().flat_map(|brand| brand.to_vec()).collect();
    let size = u32::try_from(16 + brands.len()).unwrap();
    bytes(&[&be32(size), b"ftyp", major, &be32(0), &brands])
}

#[test]
fn types_iso_media_by_the_major_brand_then_the_compatible_brands() {
    let cases: [(&str, Vec<u8>, Option<Mime>); 14] = [
        // A major brand that names a type decides.
        (
            "isom",
            ftyp(b"isom", &[b"isom", b"iso2", b"avc1", b"mp41"]),
            Some(Mime::Mp4),
        ),
        ("mp42", ftyp(b"mp42", &[b"isom", b"mp42"]), Some(Mime::Mp4)),
        (
            "dash",
            ftyp(b"dash", &[b"iso6", b"avc1", b"mp41"]),
            Some(Mime::Mp4),
        ),
        ("qt", ftyp(b"qt  ", &[b"qt  "]), Some(Mime::QuickTime)),
        (
            "apple heic",
            ftyp(b"heic", &[b"mif1", b"MiHE", b"miaf", b"MiHB", b"heic"]),
            Some(Mime::Heic),
        ),
        (
            "heif sequence",
            ftyp(b"hevc", &[b"msf1", b"hevc"]),
            Some(Mime::Heic),
        ),
        // A structural major brand: the compatible brands name the codec.
        (
            "avif under mif1",
            ftyp(b"mif1", &[b"avif", b"mif1", b"miaf"]),
            Some(Mime::Avif),
        ),
        (
            "heic under mif1",
            ftyp(b"mif1", &[b"mif1", b"iso8", b"heic"]),
            Some(Mime::Heic),
        ),
        (
            "avis under msf1",
            ftyp(b"msf1", &[b"msf1", b"avis"]),
            Some(Mime::Avif),
        ),
        (
            "an unknown major brand of an mp4",
            ftyp(b"XYZW", &[b"XYZW", b"isom"]),
            Some(Mime::Mp4),
        ),
        // A HEIF of a codec the list does not know, and brands that name nothing known.
        ("heif, no codec", ftyp(b"mif1", &[b"mif1", b"miaf"]), None),
        (
            "heif, another codec",
            ftyp(b"mif1", &[b"mif1", b"jpeg", b"iso8"]),
            None,
        ),
        ("unknown", ftyp(b"zzzz", &[b"zzzz"]), None),
        ("no compatible brands", ftyp(b"zzzz", &[]), None),
    ];
    for (name, head, mime) in cases {
        assert_eq!(sniff_mime(&head), mime, "{name}");
    }
}

#[test]
fn reads_ftyp_brands_within_the_box_and_the_head() {
    // A 64-bit box size puts the major brand after it.
    let large = bytes(&[
        &be32(1),
        b"ftyp",
        &[0, 0, 0, 0, 0, 0, 0, 32],
        b"mif1",
        &be32(0),
        b"avif",
    ]);
    assert_eq!(sniff_mime(&large), Some(Mime::Avif));
    // A size of 0 runs to the end of the head.
    let open = bytes(&[&be32(0), b"ftyp", b"mif1", &be32(0), b"heic"]);
    assert_eq!(sniff_mime(&open), Some(Mime::Heic));
    // A brand past the box's own size belongs to the next box.
    let short = bytes(&[&be32(20), b"ftyp", b"zzzz", &be32(0), b"zzzz", b"isom"]);
    assert_eq!(sniff_mime(&short), None);
    // A head cut inside the major brand, and a box shorter than its header.
    assert_eq!(
        sniff_mime(&bytes(&[&be32(1), b"ftyp", &[0; 8], b"mi"])),
        None
    );
    assert_eq!(
        sniff_mime(&bytes(&[&be32(4), b"ftyp", b"zzzz", &be32(0), b"isom"])),
        None
    );
    // Only the first 64 compatible brands are read.
    let mut many: Vec<&[u8; 4]> = vec![b"zzzz"; 64];
    many.push(b"isom");
    assert_eq!(sniff_mime(&ftyp(b"zzzz", &many)), None);
    many.remove(0);
    assert_eq!(sniff_mime(&ftyp(b"zzzz", &many)), Some(Mime::Mp4));
}

#[test]
fn returns_none_for_unknown_bytes() {
    assert_eq!(sniff_mime(b"hello world"), None);
    assert_eq!(sniff_mime(b""), None);
    // A BMP signature without a whole header, and EMF's record type without its signature.
    assert_eq!(sniff_mime(b"BM\0\0"), None);
    assert_eq!(sniff_mime(&bytes(&[&le32(1), &[0; 40]])), None);
}

#[test]
fn reads_png_size_phys_resolution_and_apng_frames() {
    assert_eq!(image_size(&png_with(&[]), Mime::Png), Some(size(3, 2)));
    let physics = png_chunk(b"pHYs", &[be32(11811), be32(11811), vec![1]].concat());
    let animated = png_chunk(b"acTL", &[be32(4), be32(0)].concat());
    let read = image_size(&png_with(&[&physics, &animated]), Mime::Png).unwrap();
    assert_eq!(read.frames, Some(4));
    assert_eq!(read.dpi.unwrap().x, 11811.0 * 0.0254);
    assert!((read.dpi.unwrap().x - 300.0).abs() < 0.5);
    // A pHYs in unknown units states no resolution.
    let relative = png_chunk(b"pHYs", &[be32(1), be32(1), vec![0]].concat());
    assert_eq!(
        image_size(&png_with(&[&relative]), Mime::Png).unwrap().dpi,
        None
    );
}

#[test]
fn reads_on_from_the_source_to_an_apng_animation_chunk_past_the_head() {
    // A large text chunk pushes the acTL past the 64 KiB head.
    let text = png_chunk(b"zTXt", &vec![0; 70_000]);
    let animated = png_chunk(b"acTL", &[be32(4), be32(0)].concat());
    let big = png_with(&[&text, &animated]);
    let head = &big[..64 * 1024];
    assert_eq!(image_size(head, Mime::Png), None);
    assert_eq!(
        read_image_size(head, Mime::Png, &BytesReader::new(&big)).unwrap(),
        Some(ImageSize {
            frames: Some(4),
            ..size(3, 2)
        })
    );
    // A file that ends before its image data leaves the size unknown.
    assert_eq!(
        read_image_size(head, Mime::Png, &BytesReader::new(&big[..70_010])).unwrap(),
        None
    );
}

#[test]
fn reads_at_most_256_png_chunks_past_the_head() {
    let text = png_chunk(b"zTXt", &vec![0; 70_000]);
    let filler = png_chunk(b"tEXt", &[b'a'; 40]);
    let animated = png_chunk(b"acTL", &[be32(4), be32(0)].concat());
    // The read past the text chunk, one per filler, and one for the acTL.
    let frames = |fillers: usize| {
        let mut chunks: Vec<&[u8]> = vec![&text];
        chunks.extend(std::iter::repeat_n(&filler[..], fillers));
        chunks.push(&animated);
        let big = png_with(&chunks);
        read_image_size(&big[..64 * 1024], Mime::Png, &BytesReader::new(&big))
            .unwrap()
            .map(|size| size.frames)
    };
    assert_eq!(frames(254), Some(Some(4)));
    assert_eq!(frames(255), None);
}

#[test]
fn reads_jpeg_size_and_jfif_resolution() {
    assert_eq!(
        image_size(&jpeg(), Mime::Jpeg),
        Some(ImageSize {
            dpi: Some(Dpi { x: 300.0, y: 300.0 }),
            ..size(640, 480)
        })
    );
}

#[test]
fn reads_on_from_the_source_to_a_jpeg_frame_header_past_the_head() {
    // Two full APP2 segments (an ICC profile) push the frame header past the 64 KiB head.
    let app2 = bytes(&[&[0xff, 0xe2], &be16(0xffff), &vec![0; 0xfffd]]);
    let jpeg = jpeg();
    let big = concat(&[&jpeg[..20], &app2, &app2, &jpeg[20..]]);
    let head = &big[..64 * 1024];
    assert_eq!(image_size(head, Mime::Jpeg), None);
    let expected = ImageSize {
        dpi: Some(Dpi { x: 300.0, y: 300.0 }),
        ..size(640, 480)
    };
    assert_eq!(
        read_image_size(head, Mime::Jpeg, &BytesReader::new(&big)).unwrap(),
        Some(expected)
    );
    // A file that ends first, or that stops being a JPEG past the head, leaves the size unknown.
    assert_eq!(
        read_image_size(head, Mime::Jpeg, &BytesReader::new(&big[..100_000])).unwrap(),
        None
    );
    let broken = concat(&[&jpeg[..20], &app2, &[0, 0, 0, 0], &jpeg[20..]]);
    assert_eq!(
        read_image_size(&broken[..64 * 1024], Mime::Jpeg, &BytesReader::new(&broken)).unwrap(),
        None
    );
}

#[test]
fn reads_at_most_256_jpeg_segments_past_the_head() {
    let app2 = bytes(&[&[0xff, 0xe2], &be16(0xffff), &vec![0; 0xfffd]]);
    let app1 = bytes(&[&[0xff, 0xe1], &be16(40), &[0; 38]]);
    let jpeg = jpeg();
    let found = |segments: usize| {
        let mut parts: Vec<&[u8]> = vec![&jpeg[..20], &app2, &app2];
        parts.extend(std::iter::repeat_n(&app1[..], segments));
        parts.push(&jpeg[20..]);
        let big = concat(&parts);
        read_image_size(&big[..64 * 1024], Mime::Jpeg, &BytesReader::new(&big))
            .unwrap()
            .is_some()
    };
    assert!(found(254));
    assert!(!found(255));
}

#[test]
fn counts_gif_frames() {
    assert_eq!(image_size(&gif(1), Mime::Gif), Some(size(4, 4)));
    assert_eq!(
        image_size(&gif(3), Mime::Gif),
        Some(ImageSize {
            frames: Some(3),
            ..size(4, 4)
        })
    );
}

#[test]
fn reads_webp_canvas_size_and_animation() {
    assert_eq!(
        image_size(&webp_vp8x(false, 0), Mime::Webp),
        Some(size(100, 50))
    );
    assert_eq!(
        image_size(&webp_vp8x(true, 2), Mime::Webp),
        Some(ImageSize {
            frames: Some(2),
            ..size(100, 50)
        })
    );
    let lossless = bytes(&[
        b"RIFF",
        &le32(0),
        b"WEBPVP8L",
        &le32(0),
        &[0x2f],
        &le32(99 | (49 << 14)),
    ]);
    assert_eq!(image_size(&lossless, Mime::Webp), Some(size(100, 50)));
    let lossy = bytes(&[
        b"RIFF",
        &le32(0),
        b"WEBPVP8 ",
        &[0; 10],
        &le16(0xc000 | 100),
        &le16(50),
    ]);
    assert_eq!(image_size(&lossy, Mime::Webp), Some(size(100, 50)));
}

#[test]
fn reads_tiff_and_bmp_sizes_and_resolutions() {
    assert_eq!(
        image_size(&tiff(), Mime::Tiff),
        Some(ImageSize {
            dpi: Some(Dpi { x: 144.0, y: 72.0 }),
            ..size(40, 30)
        })
    );
    let read = image_size(&bmp(), Mime::Bmp).unwrap();
    assert_eq!((read.width, read.height), (16, 9));
    assert!((read.dpi.unwrap().x - 96.0).abs() < 0.5);
    // An OS/2 header holds 16-bit dimensions and no resolution.
    let os2 = bytes(&[b"BM", &[0; 12], &le32(12), &le16(7), &le16(5)]);
    assert_eq!(image_size(&os2, Mime::Bmp), Some(size(7, 5)));
}

/// A page of `width`×`height`, followed by the page at `next`.
fn tiff_page(width: u32, height: u32, next: u32) -> Vec<u8> {
    bytes(&[
        &le16(2),
        &[le16(256), le16(4), le32(1), le32(width)].concat(),
        &[le16(257), le16(4), le32(1), le32(height)].concat(),
        &le32(next),
    ])
}

#[test]
fn reads_the_largest_page_of_a_tiff_the_one_its_decoder_takes() {
    let header = bytes(&[b"II", &le16(42), &le32(8)]);
    let thumbnail_first = concat(&[&header, &tiff_page(4, 4, 38), &tiff_page(4000, 3000, 0)]);
    assert_eq!(
        image_size(&thumbnail_first, Mime::Tiff),
        Some(size(4000, 3000))
    );
    // Of equal pages, the first; a chain that loops ends where it loops.
    let equal = concat(&[&header, &tiff_page(5, 6, 38), &tiff_page(6, 5, 8)]);
    assert_eq!(image_size(&equal, Mime::Tiff), Some(size(5, 6)));
    // The second page's tags past the head are read from the source.
    let far = concat(&[
        &header,
        &tiff_page(4, 4, 70_000),
        &vec![0; 70_000 - 38],
        &tiff_page(4000, 3000, 0),
    ]);
    let head = &far[..64 * 1024];
    assert_eq!(image_size(head, Mime::Tiff), None);
    assert_eq!(
        read_image_size(head, Mime::Tiff, &BytesReader::new(&far)).unwrap(),
        Some(size(4000, 3000))
    );
}

#[test]
fn gives_up_on_a_tiff_chain_longer_than_the_decoder_reads() {
    let mut file = bytes(&[b"II", &le16(42), &le32(8)]);
    for i in 0..65u32 {
        let next = if i < 64 { 8 + (i + 1) * 30 } else { 0 };
        file.extend(tiff_page(1, 1, next));
    }
    assert_eq!(image_size(&file, Mime::Tiff), None);
    file.truncate(8 + 64 * 30);
    let last = file.len() - 4;
    file[last..].copy_from_slice(&le32(0));
    assert_eq!(image_size(&file, Mime::Tiff), Some(size(1, 1)));
}

#[test]
fn returns_none_for_truncated_headers_and_other_types() {
    assert_eq!(image_size(&jpeg()[..24], Mime::Jpeg), None);
    assert_eq!(image_size(&tiff()[..12], Mime::Tiff), None);
    assert_eq!(image_size(b"<svg/>", Mime::Svg), None);
    // A zero dimension is no size.
    let empty = bytes(&[b"GIF89a", &le16(0), &le16(4), &[0, 0, 0]]);
    assert_eq!(image_size(&empty, Mime::Gif), None);
}
