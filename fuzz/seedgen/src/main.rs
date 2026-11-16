//! Writes the fuzz targets' generated seeds into `fuzz/seeds/<target>/`, from our own probe decks and from the
//! Keynote schema: an XML part of each kind in the probes, small zips covering the reader's branches, a payload of
//! every type the trimmed schema decodes with each of its fields set (`proto`), and IWA files of those objects
//! framed as Keynote frames them (`iwa`). Deterministic: a second run writes the same bytes.
//!
//! `private <dir>` writes `<dir>/<target>/` from the decks in `$SLIDOCK_PRIVATE/corpus` instead: the small decks
//! whole (`zip`), their XML parts (`xml`), Keynote index files (`iwa`) and the payloads in them (`proto`). That
//! corpus is private: it stays in the gitignored `fuzz/corpus-private/`, and regression tests never use its bytes.

use std::fs;
use std::path::{Path, PathBuf};

use slidock_io::{BytesReader, FileReader, RangeReader, ZipArchive, ZipLimits};
use slidock_keynote::iwa::schema::{FieldKind, message_index, messages};
use slidock_keynote::iwa::{is_kept, registry, type_name, unframe_iwa};

type Result<T> = std::result::Result<T, String>;

/// A generated seed is at most this large.
const MAX_SEED: usize = 16 * 1024;
/// The XML parts kept from each probe deck, by kind.
const XML_PARTS: usize = 12;

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => committed(),
        [mode, out] if mode == "private" => private(Path::new(out)),
        _ => Err("usage: slidock-fuzz-seedgen [private <dir>]".into()),
    }
}

fn committed() -> Result<()> {
    let fuzz = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let repo = fuzz.join("..");
    let seeds = fuzz.join("seeds");
    let kept: Vec<u32> = registry()
        .map(|(id, _)| id)
        .filter(|&id| is_kept(id))
        .collect();

    let mut proto = Vec::new();
    for (index, &id) in kept.iter().enumerate() {
        let name = type_name(id).ok_or("a kept type has no name")?;
        let schema = message_index(name).ok_or_else(|| format!("{name} is not in the schema"))?;
        let payload = payload(schema, 0);
        let mut seed = (index as u16).to_le_bytes().to_vec();
        seed.extend(&payload);
        proto.push((format!("{index:03}-{name}"), seed, id, payload));
    }
    write(
        &seeds.join("proto"),
        proto
            .iter()
            .map(|(name, seed, ..)| (name.clone(), seed.clone())),
    )?;

    let objects: Vec<(u32, Vec<u8>)> = proto
        .iter()
        .map(|(_, _, id, payload)| (*id, payload.clone()))
        .collect();
    write(&seeds.join("iwa"), iwa_seeds(&objects))?;

    let mut parts = Vec::new();
    for deck in [
        "fonts-probe.pptx",
        "motion-probe.pptx",
        "motion-probe.odp",
        "fonts-probe.odp",
    ] {
        let path = repo.join("probes/out").join(deck);
        let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        parts.extend(xml_parts(deck, &bytes)?);
    }
    parts.extend(xml_samples());
    write(&seeds.join("xml"), parts)?;

    write(
        &seeds.join("zip"),
        zip_seeds(&iwa_seeds(&objects[..objects.len().min(8)])),
    )?;

    let mut media = probe_media(&repo)?;
    media.extend(raster_samples());
    media.push(("sample.webm".into(), webm_sample()));
    write(&seeds.join("media"), media)?;
    write(&seeds.join("compile"), compile_seeds(&repo)?)?;
    Ok(())
}

/// The probe decks seeds are read from, in order.
fn probe_decks(repo: &Path) -> Result<Vec<PathBuf>> {
    let dir = repo.join("probes/out");
    let mut decks: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|error| format!("{}: {error}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "pptx" || extension == "odp")
        })
        .collect();
    decks.sort();
    Ok(decks)
}

/// Every media part of the probe decks within [`MAX_SEED`], each once: `<deck>-<file>`.
fn probe_media(repo: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    for path in probe_decks(repo)? {
        let reader =
            FileReader::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let archive = ZipArchive::open(&reader, ZipLimits::default())
            .map_err(|error| format!("{}: {}", path.display(), error.message))?;
        let deck = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("deck");
        for entry in archive.entries() {
            let media = ["ppt/media/", "Pictures/", "Media/"]
                .iter()
                .any(|dir| entry.name.starts_with(dir));
            if !media || entry.size > MAX_SEED as u64 {
                continue;
            }
            let bytes = archive
                .read(entry)
                .map_err(|error| format!("{}: {}", entry.name, error.message))?
                .into_owned();
            if out.iter().any(|(_, seen)| *seen == bytes) {
                continue;
            }
            let file = entry.name.rsplit('/').next().unwrap_or(&entry.name);
            out.push((format!("{deck}-{file}"), bytes));
        }
    }
    Ok(out)
}

/// A classic TIFF of one page: `samples` of `depth` bits a pixel, its strips of `rows` rows compressed with
/// `compression` (1 none, 8 Deflate, 32773 PackBits of literal runs), in either byte order.
struct TiffSample<'a> {
    width: u32,
    height: u32,
    samples: u16,
    depth: u16,
    photometric: u16,
    compression: u16,
    predictor: u16,
    rows: u32,
    little: bool,
    colour_map: &'a [u16],
}

fn tiff(sample: &TiffSample<'_>) -> Vec<u8> {
    let bytes_per_row =
        sample.width as usize * usize::from(sample.samples) * usize::from(sample.depth / 8);
    let pixel = |x: usize, y: usize| ((x * 37 + y * 11) % 256) as u8;
    let mut strips = Vec::new();
    for first in (0..sample.height).step_by(sample.rows as usize) {
        let rows = sample.rows.min(sample.height - first) as usize;
        let mut raw = Vec::with_capacity(rows * bytes_per_row);
        for y in 0..rows {
            raw.extend((0..bytes_per_row).map(|x| pixel(x, first as usize + y)));
        }
        strips.push(match sample.compression {
            8 => miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
            32773 => raw
                .chunks(128)
                .flat_map(|run| {
                    let mut literal = vec![(run.len() - 1) as u8];
                    literal.extend(run);
                    literal
                })
                .collect(),
            _ => raw,
        });
    }
    let u16s = |value: u16| {
        if sample.little {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        }
    };
    let u32s = |value: u32| {
        if sample.little {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        }
    };
    // Header, then the strips, the arrays the tags point to, and the directory last.
    let mut out = if sample.little {
        b"II".to_vec()
    } else {
        b"MM".to_vec()
    };
    out.extend(u16s(42));
    out.extend([0; 4]);
    let mut offsets = Vec::new();
    for strip in &strips {
        offsets.push(out.len() as u32);
        out.extend(strip);
    }
    let array = |out: &mut Vec<u8>, values: &[u32], short: bool| -> u32 {
        let at = out.len() as u32;
        for &value in values {
            if short {
                out.extend(u16s(value as u16));
            } else {
                out.extend(u32s(value));
            }
        }
        at
    };
    let counts: Vec<u32> = strips.iter().map(|strip| strip.len() as u32).collect();
    let depths = vec![u32::from(sample.depth); usize::from(sample.samples)];
    let mut tags: Vec<(u16, u16, u32, u32)> =
        vec![(256, 4, 1, sample.width), (257, 4, 1, sample.height)];
    let depth_at = if sample.samples > 2 {
        array(&mut out, &depths, true)
    } else {
        u32::from(sample.depth) << if sample.little { 0 } else { 16 }
    };
    tags.push((258, 3, u32::from(sample.samples), depth_at));
    tags.push((259, 3, 1, u32::from(sample.compression)));
    tags.push((262, 3, 1, u32::from(sample.photometric)));
    let strip_offsets = if offsets.len() > 1 {
        array(&mut out, &offsets, false)
    } else {
        offsets.first().copied().unwrap_or(0)
    };
    tags.push((273, 4, offsets.len() as u32, strip_offsets));
    tags.push((277, 3, 1, u32::from(sample.samples)));
    tags.push((278, 4, 1, sample.rows));
    let strip_counts = if counts.len() > 1 {
        array(&mut out, &counts, false)
    } else {
        counts.first().copied().unwrap_or(0)
    };
    tags.push((279, 4, counts.len() as u32, strip_counts));
    if sample.predictor != 1 {
        tags.push((317, 3, 1, u32::from(sample.predictor)));
    }
    if !sample.colour_map.is_empty() {
        let map: Vec<u32> = sample
            .colour_map
            .iter()
            .map(|&value| u32::from(value))
            .collect();
        let at = array(&mut out, &map, true);
        tags.push((320, 3, map.len() as u32, at));
    }
    if sample.samples == 2 || sample.samples == 4 {
        tags.push((338, 3, 1, 2 << if sample.little { 0 } else { 16 }));
    }
    let directory = out.len() as u32;
    out.extend(u16s(tags.len() as u16));
    for (tag, kind, count, value) in tags {
        out.extend(u16s(tag));
        out.extend(u16s(kind));
        out.extend(u32s(count));
        // A SHORT that fits sits in the first two bytes of the value.
        if kind == 3 && count == 1 {
            out.extend(u16s(value as u16));
            out.extend([0, 0]);
        } else {
            out.extend(u32s(value));
        }
    }
    out.extend([0; 4]);
    let directory = u32s(directory);
    if let Some(slot) = out.get_mut(4..8) {
        slot.copy_from_slice(&directory);
    }
    out
}

/// A bottom-up BMP of `depth` bits a pixel (8 with a grey palette), with a BITMAPINFOHEADER.
fn bmp(width: u32, height: i32, depth: u16) -> Vec<u8> {
    let palette: Vec<u8> = if depth == 8 {
        (0..=255u8).flat_map(|grey| [grey, grey, grey, 0]).collect()
    } else {
        Vec::new()
    };
    let row = (width as usize * usize::from(depth)).div_ceil(32) * 4;
    let pixels: Vec<u8> = (0..row * height.unsigned_abs() as usize)
        .map(|at| (at * 29 % 256) as u8)
        .collect();
    let offset = 14 + 40 + palette.len();
    let mut out = b"BM".to_vec();
    out.extend(((offset + pixels.len()) as u32).to_le_bytes());
    out.extend([0; 4]);
    out.extend((offset as u32).to_le_bytes());
    out.extend(40u32.to_le_bytes());
    out.extend(width.to_le_bytes());
    out.extend(height.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(depth.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend((pixels.len() as u32).to_le_bytes());
    out.extend([0; 16]);
    out.extend(palette);
    out.extend(pixels);
    out
}

/// TIFFs and BMPs of the forms the transcoder reads: the probes hold none.
fn raster_samples() -> Vec<(String, Vec<u8>)> {
    let base = TiffSample {
        width: 9,
        height: 7,
        samples: 3,
        depth: 8,
        photometric: 2,
        compression: 1,
        predictor: 1,
        rows: 7,
        little: true,
        colour_map: &[],
    };
    let colour_map: Vec<u16> = (0..3 * 256).map(|at| (at * 257 % 65536) as u16).collect();
    vec![
        ("sample-rgb.tif".into(), tiff(&base)),
        (
            "sample-rgba-strips-be.tif".into(),
            tiff(&TiffSample {
                samples: 4,
                rows: 2,
                little: false,
                ..base
            }),
        ),
        (
            "sample-grey16-packbits.tif".into(),
            tiff(&TiffSample {
                samples: 1,
                depth: 16,
                photometric: 1,
                compression: 32773,
                rows: 3,
                ..base
            }),
        ),
        (
            "sample-palette-deflate.tif".into(),
            tiff(&TiffSample {
                samples: 1,
                photometric: 3,
                compression: 8,
                colour_map: &colour_map,
                ..base
            }),
        ),
        (
            "sample-predictor.tif".into(),
            tiff(&TiffSample {
                compression: 8,
                predictor: 2,
                rows: 4,
                ..base
            }),
        ),
        ("sample-24.bmp".into(), bmp(5, 3, 24)),
        ("sample-32-top-down.bmp".into(), bmp(4, -3, 32)),
        ("sample-8.bmp".into(), bmp(6, 2, 8)),
    ]
}

/// An EBML element: its ID, its size as an 8-byte vint, then its body.
fn ebml(id: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.push(0x01);
    out.extend(&(body.len() as u64).to_be_bytes()[1..]);
    out.extend(body);
    out
}

/// A WebM header with one VP9 track of 16 × 16, a duration and one SimpleBlock.
fn webm_sample() -> Vec<u8> {
    let header = ebml(
        &[0x1a, 0x45, 0xdf, 0xa3],
        &[ebml(&[0x42, 0x82], b"webm"), ebml(&[0x42, 0x87], &[4])].concat(),
    );
    let info = ebml(
        &[0x15, 0x49, 0xa9, 0x66],
        &[
            ebml(&[0x2a, 0xd7, 0xb1], &[0x0f, 0x42, 0x40]),
            ebml(&[0x44, 0x89], &1000f64.to_be_bytes()),
        ]
        .concat(),
    );
    let video = ebml(
        &[0xe0],
        &[ebml(&[0xb0], &[16]), ebml(&[0xba], &[16])].concat(),
    );
    let track = ebml(
        &[0xae],
        &[
            ebml(&[0xd7], &[1]),
            ebml(&[0x83], &[1]),
            ebml(&[0x86], b"V_VP9"),
            video,
        ]
        .concat(),
    );
    let tracks = ebml(&[0x16, 0x54, 0xae, 0x6b], &track);
    let cluster = ebml(
        &[0x1f, 0x43, 0xb6, 0x75],
        &[
            ebml(&[0xe7], &[0]),
            ebml(&[0xa3], &[0x81, 0, 0, 0x80, 0, 0]),
        ]
        .concat(),
    );
    let segment = ebml(&[0x18, 0x53, 0x80, 0x67], &[info, tracks, cluster].concat());
    [header, segment].concat()
}

/// The small probe decks whole, and one rewritten the way MEDIUM amplification inputs are: its presentation lists
/// its one slide 40 times.
fn compile_seeds(repo: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    for path in probe_decks(repo)? {
        let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if bytes.len() > MAX_SEED {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("deck");
        out.push((name.to_owned(), bytes));
    }
    let deck = repo.join("probes/out/line-ends-probe.pptx");
    let reader = FileReader::open(&deck).map_err(|error| format!("{}: {error}", deck.display()))?;
    let archive = ZipArchive::open(&reader, ZipLimits::default())
        .map_err(|error| format!("{}: {}", deck.display(), error.message))?;
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in archive.entries() {
        let mut bytes = archive
            .read(entry)
            .map_err(|error| format!("{}: {}", entry.name, error.message))?
            .into_owned();
        if entry.name == "ppt/presentation.xml" {
            let text = String::from_utf8(bytes).map_err(|error| error.to_string())?;
            let start = text.find("<p:sldId ").ok_or("the probe lists no slide")?;
            let end = start + text[start..].find("/>").ok_or("an unclosed sldId")? + 2;
            let again: String = (0..40)
                .map(|n| text[start..end].replacen("id=\"", &format!("id=\"{}", 300 + n), 1))
                .collect();
            bytes = format!("{}{again}{}", &text[..end], &text[end..]).into_bytes();
        }
        parts.push((entry.name.clone(), bytes));
    }
    let entries: Vec<Entry<'_>> = parts
        .iter()
        .map(|(name, data)| Entry {
            name,
            data,
            deflate: true,
        })
        .collect();
    out.push((
        "line-ends-probe-relisted.pptx".into(),
        zip(&entries, b"", false),
    ));
    Ok(out)
}

/// Replaces `dir`'s files with `seeds`.
fn write(dir: &Path, seeds: impl IntoIterator<Item = (String, Vec<u8>)>) -> Result<()> {
    if dir.exists() {
        for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
            let path: PathBuf = entry.map_err(|error| error.to_string())?.path();
            if path.is_file() {
                fs::remove_file(&path).map_err(|error| error.to_string())?;
            }
        }
    }
    fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let mut count = 0;
    for (name, bytes) in seeds {
        if bytes.len() > MAX_SEED {
            return Err(format!("seed {name} is {} bytes", bytes.len()));
        }
        fs::write(dir.join(&name), bytes).map_err(|error| format!("{name}: {error}"))?;
        count += 1;
    }
    println!("{}: {count} seeds", dir.display());
    Ok(())
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn tag(out: &mut Vec<u8>, number: u32, wire_type: u32) {
    varint(out, u64::from(number << 3 | wire_type));
}

/// The nesting the generated payloads reach; deeper message fields are left out.
const PAYLOAD_DEPTH: usize = 3;
/// A nested message is left out once its parent holds this much.
const PAYLOAD_BYTES: usize = 2048;

/// A payload of schema message `index` with every field set once (repeated numbers twice, once packed), messages
/// to [`PAYLOAD_DEPTH`].
fn payload(index: usize, depth: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(message) = messages().get(index) else {
        return out;
    };
    for field in message.fields {
        let number = field.number;
        match field.kind {
            FieldKind::Bool
            | FieldKind::Uint32
            | FieldKind::Int32
            | FieldKind::Enum
            | FieldKind::Uint64 => {
                tag(&mut out, number, 0);
                varint(&mut out, 1);
                if field.repeated {
                    tag(&mut out, number, 2);
                    out.extend([2, 2, 0x7f]);
                }
            }
            FieldKind::Fixed32 | FieldKind::Float => {
                tag(&mut out, number, 5);
                out.extend(1.5f32.to_le_bytes());
            }
            FieldKind::Double => {
                tag(&mut out, number, 1);
                out.extend(2.5f64.to_le_bytes());
            }
            FieldKind::String => {
                tag(&mut out, number, 2);
                out.extend([5]);
                out.extend(b"Fuzz\xc3\xa9"[..5].iter());
            }
            FieldKind::Bytes => {
                tag(&mut out, number, 2);
                out.extend([3, 1, 2, 3]);
            }
            FieldKind::Message(child) => {
                if depth >= PAYLOAD_DEPTH || out.len() > PAYLOAD_BYTES {
                    continue;
                }
                let body = payload(child, depth + 1);
                tag(&mut out, number, 2);
                varint(&mut out, body.len() as u64);
                out.extend(body);
            }
        }
    }
    out
}

/// An object stream record: its ArchiveInfo, then its payloads.
fn record(out: &mut Vec<u8>, id: u64, type_id: Option<u32>, payloads: &[&[u8]], merge: bool) {
    let mut info = Vec::new();
    tag(&mut info, 1, 0);
    varint(&mut info, id);
    for payload in payloads {
        let mut message_info = Vec::new();
        if let Some(type_id) = type_id {
            tag(&mut message_info, 1, 0);
            varint(&mut message_info, u64::from(type_id));
        }
        tag(&mut message_info, 2, 2);
        message_info.extend([3, 1, 0, 5]);
        tag(&mut message_info, 3, 0);
        varint(&mut message_info, payload.len() as u64);
        tag(&mut info, 2, 2);
        varint(&mut info, message_info.len() as u64);
        info.extend(message_info);
    }
    if merge {
        tag(&mut info, 3, 0);
        varint(&mut info, 1);
    }
    varint(out, info.len() as u64);
    out.extend(info);
    for payload in payloads {
        out.extend(*payload);
    }
}

/// `stream` in chunks of `chunk` bytes, each a raw Snappy block, compressed or one literal.
fn frame(stream: &[u8], chunk: usize, compress: bool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for part in stream.chunks(chunk.max(1)) {
        let block = if compress {
            snap::raw::Encoder::new()
                .compress_vec(part)
                .map_err(|error| error.to_string())?
        } else {
            literal(part)
        };
        let length = block.len();
        out.extend([0, length as u8, (length >> 8) as u8, (length >> 16) as u8]);
        out.extend(block);
    }
    Ok(out)
}

/// A raw Snappy block holding `bytes` as one literal.
fn literal(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    varint(&mut out, bytes.len() as u64);
    if let Some(n) = bytes.len().checked_sub(1) {
        if n < 60 {
            out.push((n as u8) << 2);
        } else {
            let width = (n.ilog2() / 8 + 1) as u8;
            out.push((59 + width) << 2);
            out.extend(&(n as u32).to_le_bytes()[..usize::from(width)]);
        }
    }
    out.extend(bytes);
    out
}

fn iwa_seeds(objects: &[(u32, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    let mut seeds = Vec::new();
    let mut id = 1;
    for (group, chunk) in objects.chunks(6).enumerate() {
        let mut stream = Vec::new();
        for (type_id, payload) in chunk {
            record(&mut stream, id, Some(*type_id), &[payload], false);
            id += 1;
        }
        let (size, compress) = [(64 * 1024, true), (700, true), (64 * 1024, false)][group % 3];
        if let Ok(framed) = frame(&stream, size, compress)
            && framed.len() <= MAX_SEED
        {
            seeds.push((format!("objects-{group:02}.iwa"), framed));
        }
    }
    // Records the stream reads apart: a merge, extra payloads, an untyped one, an unknown and a skipped type.
    let mut stream = Vec::new();
    if let Some((type_id, payload)) = objects.first() {
        record(&mut stream, 1, Some(*type_id), &[payload], false);
        record(&mut stream, 1, Some(*type_id), &[payload], true);
        record(
            &mut stream,
            2,
            Some(*type_id),
            &[payload, &[0x08, 1]],
            false,
        );
    }
    record(&mut stream, 3, None, &[&[0x08, 1]], false);
    record(&mut stream, 4, Some(999_999), &[&[0x08, 1]], false);
    if let Some(skipped) = registry().map(|(id, _)| id).find(|&id| !is_kept(id)) {
        record(&mut stream, 5, Some(skipped), &[&[0x08, 1]], false);
    }
    if let Ok(framed) = frame(&stream, 64 * 1024, true) {
        seeds.push(("records.iwa".into(), framed));
    }
    seeds.push(("empty.iwa".into(), vec![0, 1, 0, 0, 0]));
    seeds
}

/// An XML part of each kind (its name without digits) from a probe deck, the smallest first.
fn xml_parts(deck: &str, bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let archive = ZipArchive::open(BytesReader::new(bytes), ZipLimits::default())
        .map_err(|error| error.message)?;
    let mut entries: Vec<_> = archive
        .entries()
        .iter()
        .filter(|entry| entry.name.ends_with(".xml") || entry.name.ends_with(".rels"))
        .filter(|entry| entry.size as usize <= MAX_SEED)
        .collect();
    entries.sort_by_key(|entry| (entry.size, entry.name.clone()));
    let mut kinds = Vec::new();
    let mut out = Vec::new();
    for entry in entries {
        let kind: String = entry.name.chars().filter(|c| !c.is_ascii_digit()).collect();
        if kinds.contains(&kind) || out.len() >= XML_PARTS {
            continue;
        }
        let part = archive.read(entry).map_err(|error| error.message)?;
        kinds.push(kind);
        let name = format!("{deck}-{}", entry.name.replace(['/', '[', ']'], "_"));
        out.push((name, part.into_owned()));
    }
    Ok(out)
}

/// Markup the probes do not hold: comments, CDATA, processing instructions, references, namespaces declared and
/// redeclared, UTF-16LE.
fn xml_samples() -> Vec<(String, Vec<u8>)> {
    let markup = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- a comment -->",
        "<p:sld xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" ",
        "xmlns:a=\" http://schemas.openxmlformats.org/drawingml/2006/main \" xmlns=\"urn:default\">",
        "<p:sp a:id=\"1\" r:x='&#x41;&amp;&lt;&#66;' xmlns:r=\"urn:r\">text<?pi data?>more<![CDATA[<raw>]]>",
        "<a:t xml:space=\"preserve\"> spaced &#x1F600; </a:t><x xmlns:p=\"urn:other\"><p:y/></x></p:sp></p:sld>",
    );
    let mut utf16 = vec![0xff, 0xfe];
    for unit in "<r a=\"\u{e9}\">\u{1F600}</r>".encode_utf16() {
        utf16.extend(unit.to_le_bytes());
    }
    vec![
        ("sample-namespaces.xml".into(), markup.as_bytes().to_vec()),
        ("sample-utf16le.xml".into(), utf16),
        (
            "sample-bom.xml".into(),
            b"\xef\xbb\xbf<a b=\"1\"/>".to_vec(),
        ),
    ]
}

/// One zip entry to write.
struct Entry<'a> {
    name: &'a str,
    data: &'a [u8],
    deflate: bool,
}

/// A zip of `entries`, with `comment`; `zip64` writes each entry's sizes and offset in a ZIP64 extra field and the
/// directory end in a ZIP64 record.
fn zip(entries: &[Entry<'_>], comment: &[u8], zip64: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for entry in entries {
        let body = if entry.deflate {
            miniz_oxide::deflate::compress_to_vec(entry.data, 6)
        } else {
            entry.data.to_vec()
        };
        let method: u16 = if entry.deflate { 8 } else { 0 };
        let offset = out.len() as u32;
        let (compressed, size) = (body.len() as u32, entry.data.len() as u32);
        let mut extra = Vec::new();
        if zip64 {
            extra.extend(1u16.to_le_bytes());
            extra.extend(24u16.to_le_bytes());
            extra.extend(u64::from(size).to_le_bytes());
            extra.extend(u64::from(compressed).to_le_bytes());
            extra.extend(u64::from(offset).to_le_bytes());
        }
        let name = entry.name.as_bytes();
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([20, 0, 0, 0]);
        out.extend(method.to_le_bytes());
        out.extend([0; 8]);
        out.extend(compressed.to_le_bytes());
        out.extend(size.to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(name);
        out.extend(&body);
        let wide = |value: u32| if zip64 { u32::MAX } else { value };
        directory.extend(0x0201_4b50u32.to_le_bytes());
        directory.extend([20, 0, 20, 0, 0, 0]);
        directory.extend(method.to_le_bytes());
        directory.extend([0; 8]);
        directory.extend(wide(compressed).to_le_bytes());
        directory.extend(wide(size).to_le_bytes());
        directory.extend((name.len() as u16).to_le_bytes());
        directory.extend((extra.len() as u16).to_le_bytes());
        directory.extend([0; 8]);
        directory.extend(0u32.to_le_bytes());
        directory.extend(wide(offset).to_le_bytes());
        directory.extend(name);
        directory.extend(extra);
    }
    let (count, size, offset) = (
        entries.len() as u16,
        directory.len() as u32,
        out.len() as u32,
    );
    out.extend(directory);
    if zip64 {
        let end = out.len() as u64;
        out.extend(0x0606_4b50u32.to_le_bytes());
        out.extend(44u64.to_le_bytes());
        out.extend([45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend(u64::from(count).to_le_bytes());
        out.extend(u64::from(count).to_le_bytes());
        out.extend(u64::from(size).to_le_bytes());
        out.extend(u64::from(offset).to_le_bytes());
        out.extend(0x0706_4b50u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(end.to_le_bytes());
        out.extend(1u32.to_le_bytes());
    }
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    let (count, size, offset) = if zip64 {
        (u16::MAX, u32::MAX, u32::MAX)
    } else {
        (count, size, offset)
    };
    out.extend(count.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend(offset.to_le_bytes());
    out.extend((comment.len() as u16).to_le_bytes());
    out.extend(comment);
    out
}

fn zip_seeds(iwa: &[(String, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    let content_types = b"<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>";
    let text = "slidock ".repeat(200);
    let mut seeds = vec![
        ("empty.zip".into(), zip(&[], b"", false)),
        (
            "parts.zip".into(),
            zip(
                &[
                    Entry {
                        name: "[Content_Types].xml",
                        data: content_types,
                        deflate: true,
                    },
                    Entry {
                        name: "ppt/media/image1.bin",
                        data: &[7; 300],
                        deflate: false,
                    },
                    Entry {
                        name: "ppt/slides/slide1.xml",
                        data: text.as_bytes(),
                        deflate: true,
                    },
                ],
                b"a comment PK\x05\x06",
                false,
            ),
        ),
        (
            "zip64.zip".into(),
            zip(
                &[
                    Entry {
                        name: "mimetype",
                        data: b"application/vnd.oasis.opendocument.presentation",
                        deflate: false,
                    },
                    Entry {
                        name: "content.xml",
                        data: text.as_bytes(),
                        deflate: true,
                    },
                ],
                b"",
                true,
            ),
        ),
    ];
    let index: Vec<(String, &[u8])> = iwa
        .iter()
        .take(3)
        .map(|(name, bytes)| (format!("Index/{name}"), bytes.as_slice()))
        .collect();
    let mut entries = vec![Entry {
        name: "Index/Document.iwa",
        data: &[0, 1, 0, 0, 0],
        deflate: false,
    }];
    entries.extend(index.iter().map(|(name, data)| Entry {
        name,
        data,
        deflate: false,
    }));
    entries.push(Entry {
        name: "Metadata/Properties.plist",
        data: text.as_bytes(),
        deflate: true,
    });
    seeds.push(("keynote.zip".into(), zip(&entries, b"", false)));
    seeds
}

/// The private corpus.
fn corpus() -> PathBuf {
    std::env::var_os("SLIDOCK_PRIVATE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".slidock")))
        .unwrap_or_default()
        .join("corpus")
}

/// Private seeds are at most this large, by target.
const PRIVATE_ZIP: u64 = 512 * 1024;
const PRIVATE_PART: u64 = 64 * 1024;
const PRIVATE_PAYLOAD: usize = 16 * 1024;
/// Index files and XML parts kept per deck, payloads per type and deck.
const PRIVATE_FILES: usize = 80;
const PRIVATE_PAYLOADS: usize = 6;

fn private(out: &Path) -> Result<()> {
    let dir = corpus();
    let mut decks: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|error| format!("{}: {error}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("pptx" | "odp" | "key")
            )
        })
        .collect();
    decks.sort();
    let kept: Vec<u32> = registry()
        .map(|(id, _)| id)
        .filter(|&id| is_kept(id))
        .collect();
    let (mut zips, mut parts, mut index, mut payloads) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (number, path) in decks.iter().enumerate() {
        let reader =
            FileReader::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let tag = format!("deck{number:02}");
        if reader.len() <= PRIVATE_ZIP {
            let bytes = reader
                .read_at(0, reader.len() as usize)
                .map_err(|error| error.to_string())?;
            zips.push((format!("{tag}.zip"), bytes.into_owned()));
        }
        let Ok(archive) = ZipArchive::open(&reader, ZipLimits::default()) else {
            continue;
        };
        let mut entries: Vec<_> = archive
            .entries()
            .iter()
            .filter(|entry| entry.size <= PRIVATE_PART)
            .collect();
        entries.sort_by_key(|entry| (entry.size, entry.name.clone()));
        let (mut kinds, mut files) = (Vec::new(), 0);
        for entry in entries {
            let xml = entry.name.ends_with(".xml") || entry.name.ends_with(".rels");
            let iwa = entry.name.contains("Index/") && entry.name.ends_with(".iwa");
            let kind: String = entry.name.chars().filter(|c| !c.is_ascii_digit()).collect();
            if !(xml || iwa)
                || kinds.contains(&kind) && files >= PRIVATE_FILES / 2
                || files >= PRIVATE_FILES
            {
                continue;
            }
            let Ok(bytes) = archive.read(entry) else {
                continue;
            };
            kinds.push(kind);
            files += 1;
            let name = format!("{tag}-{}", entry.name.replace(['/', '[', ']', ' '], "_"));
            if xml {
                parts.push((name, bytes.into_owned()));
            } else {
                payloads.extend(index_payloads(&tag, &entry.name, &bytes, &kept));
                index.push((name, bytes.into_owned()));
            }
        }
    }
    for (target, seeds) in [
        ("zip", zips),
        ("xml", parts),
        ("iwa", index),
        ("proto", payloads),
    ] {
        let dir = out.join(target);
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        for (name, bytes) in &seeds {
            fs::write(dir.join(format!("{name}.seed")), bytes)
                .map_err(|error| format!("{name}: {error}"))?;
        }
        println!("{}: {} seeds", dir.display(), seeds.len());
    }
    Ok(())
}

/// The payloads of an index file's objects, as `proto` seeds: the kept type's index, then the payload.
fn index_payloads(tag: &str, file: &str, bytes: &[u8], kept: &[u32]) -> Vec<(String, Vec<u8>)> {
    let Ok(stream) = unframe_iwa(bytes, file, 64 << 20) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut counts = std::collections::BTreeMap::<u32, usize>::new();
    let mut at = 0;
    while let Some((length, start)) = read_varint(&stream, at) {
        let Some(info) = stream.get(start..start + length as usize) else {
            break;
        };
        at = start + length as usize;
        for (type_id, length) in message_infos(info) {
            let Some(payload) = stream.get(at..at + length as usize) else {
                return out;
            };
            at += length as usize;
            let Some(index) = kept.iter().position(|&id| id == type_id) else {
                continue;
            };
            let count = counts.entry(type_id).or_default();
            if *count < PRIVATE_PAYLOADS && payload.len() <= PRIVATE_PAYLOAD {
                *count += 1;
                let mut seed = (index as u16).to_le_bytes().to_vec();
                seed.extend(payload);
                let name = format!("{tag}-{}-{type_id}-{count}", file.replace(['/', ' '], "_"));
                out.push((name, seed));
            }
        }
    }
    out
}

fn read_varint(bytes: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(at)?;
        at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some((value, at));
        }
    }
    None
}

/// Each MessageInfo's type and length in an ArchiveInfo (fields 2; 1 and 3).
fn message_infos(info: &[u8]) -> Vec<(u32, u64)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some((tag, next)) = read_varint(info, at) {
        at = next;
        match (tag >> 3, tag & 7) {
            (2, 2) => {
                let Some((length, start)) = read_varint(info, at) else {
                    break;
                };
                let Some(body) = info.get(start..start + length as usize) else {
                    break;
                };
                at = start + length as usize;
                let (mut type_id, mut size, mut inner) = (0, 0, 0);
                while let Some((tag, next)) = read_varint(body, inner) {
                    let Some((value, next)) = read_varint(body, next) else {
                        break;
                    };
                    inner = next;
                    match tag {
                        0x08 => type_id = value as u32,
                        0x18 => size = value,
                        _ if tag & 7 == 0 => {}
                        _ => break,
                    }
                }
                out.push((type_id, size));
            }
            (_, 0) => match read_varint(info, at) {
                Some((_, next)) => at = next,
                None => break,
            },
            _ => break,
        }
    }
    out
}
