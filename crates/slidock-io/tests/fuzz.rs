//! Structure-blind mutations of valid inputs: every reader returns a `Result` or `None`, and never panics or hangs.
//! `FUZZ_ITERATIONS` sets the runs per test.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

mod support;

use slidock_io::xml::{attributes, qualified_name, text_content};
use slidock_io::zip::{ZipArchive, ZipLimits};
use slidock_io::{
    BytesReader, Mime, RangeReader, XmlBudget, XmlLimits, decode_bmp, decode_tiff, decode_xml,
    image_size, parse_xml, probe_video, read_image_size, sniff_mime, transcode_to_png,
};
use support::{
    BmpSpec, Seeded, bmp_file, deflated, fuzz_iterations, movie, mutate, red_bmp, stored,
    tiff_encodings, zip64,
};

const LIMITS: ZipLimits = ZipLimits {
    directory_bytes: 1024 * 1024,
    entries: 1000,
    entry_bytes: 1024 * 1024,
    memory_bytes: 4 * 1024 * 1024,
};

/// Runs `parse` on mutations of each base, one to three rounds each.
fn fuzz(seed: u32, bases: &[Vec<u8>], mut parse: impl FnMut(&[u8])) {
    let mut random = Seeded::new(seed);
    let iterations = fuzz_iterations();
    assert!(iterations > 0);
    for i in 0..iterations {
        let mut bytes = bases[i % bases.len()].clone();
        let rounds = 1 + random.below(3);
        for _ in 0..rounds {
            bytes = mutate(&bytes, &mut random);
        }
        parse(&bytes);
    }
}

#[test]
fn mutates_reproducibly_from_the_seed() {
    let mut first = Seeded::new(0x5eed);
    let mut second = Seeded::new(0x5eed);
    let draws: Vec<f64> = (0..5).map(|_| first.next()).collect();
    assert_eq!(draws, (0..5).map(|_| second.next()).collect::<Vec<_>>());
    assert!(draws.iter().all(|draw| (0.0..1.0).contains(draw)));
}

#[test]
fn zip_directories_and_entries() {
    let presentation = "<p/>".repeat(50);
    let document = "document".repeat(20);
    let bases = [
        deflated(&[
            ("ppt/presentation.xml", presentation.as_bytes()),
            ("media/a.png", &[0; 300]),
        ]),
        stored(&[
            ("Index/Document.iwa", document.as_bytes()),
            ("Data/movie.mov", &[0; 500]),
        ]),
        zip64(&[("a.txt", b"alpha"), ("b.txt", b"beta")]),
    ];
    fuzz(0x5eed, &bases, |bytes| {
        let Ok(archive) = ZipArchive::open(BytesReader::new(bytes), LIMITS) else {
            return;
        };
        for entry in archive.entries() {
            let _ = archive.read(entry);
            if let Ok(reader) = archive.open_entry(entry) {
                let _ = reader.read_at(0, reader.len() as usize);
            }
        }
        let all: Vec<_> = archive.entries().iter().collect();
        let _ = archive.read_many(&all);
    });
}

#[test]
fn xml_parts() {
    let base = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><p:sld xmlns:p=\"urn:p\" xmlns:a=\"urn:a\"><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sld>",
        "<p:sp a:x='1' y=\"&amp;\"><a:t xml:space=\"preserve\">Hello &amp; &#233;</a:t><!-- c --><?pi <a>?><a:b xmlns:a = 'urn:b' c=\"'>\"/><![CDATA[<x>]]></p:sp>".repeat(20)
    );
    let utf16: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain(base.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let limits = XmlLimits::default();
    let prefixes = [("urn:a", "a")];
    fuzz(0x5eed, &[base.into_bytes(), utf16], |bytes| {
        let Ok(text) = decode_xml(bytes) else {
            return;
        };
        let document = match parse_xml(&text, &limits, &mut XmlBudget::new(&limits)) {
            Ok(document) => document,
            // The tag scan stops only at markup roxmltree refuses.
            Err(error) => {
                assert_ne!(error.message, "The part could not be scanned");
                return;
            }
        };
        for node in document.descendants().filter(|node| node.is_element()) {
            let _ = qualified_name(node);
            let _ = attributes(node, Some(&prefixes));
            let _ = text_content(node);
        }
    });
}

#[test]
fn sniffing_image_headers_and_the_video_probe() {
    let bases = [
        vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 4, 0, 0, 0, 4,
        ],
        b"GIF89a\x04\x00\x04\x00\x80\x00\x00\x00\x00\x00\x00\x00\x00,\x00\x00\x00\x00\x04\x00\x04\x00\x00\x02\x01\x00\x00;".to_vec(),
        vec![0, 0, 0, 16, 102, 116, 121, 112, 105, 115, 111, 109, 0, 0, 0, 0, 0, 0, 0, 8, 109, 111, 111, 118],
        movie(b"vide", b"avc1", true, Some(1000)),
        tiff_encodings()[1].1.clone(),
    ];
    let types = [
        Mime::Png,
        Mime::Jpeg,
        Mime::Gif,
        Mime::Webp,
        Mime::Tiff,
        Mime::Bmp,
    ];
    fuzz(0x5eed, &bases, |bytes| {
        let mime = sniff_mime(bytes);
        let reader = BytesReader::new(bytes);
        let head = &bytes[..bytes.len().min(32)];
        for mime in types {
            let _ = image_size(bytes, mime);
            let _ = read_image_size(head, mime, &reader).unwrap();
        }
        if mime.is_some_and(|mime| mime.as_str().starts_with("video/")) {
            probe_video(&reader).unwrap();
        }
    });
}

#[test]
fn raster_decoders() {
    let paletted = bmp_file(&BmpSpec {
        palette: vec![[1, 2, 3], [4, 5, 6]],
        ..BmpSpec::new(3, 2, 4, vec![vec![0x01, 0x10], vec![0x11, 0x00]])
    });
    let masked = bmp_file(&BmpSpec {
        compression: 3,
        masks: vec![0xf800, 0x07e0, 0x001f],
        ..BmpSpec::new(2, 2, 16, vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]])
    });
    let mut bases: Vec<Vec<u8>> = tiff_encodings().into_iter().map(|(_, file)| file).collect();
    bases.extend([red_bmp(), paletted, masked]);
    fuzz(0x5eed, &bases, |bytes| {
        let _ = decode_tiff(bytes, 100_000);
        let _ = decode_bmp(bytes, 100_000);
        let _ = transcode_to_png(bytes, 100_000);
    });
}
