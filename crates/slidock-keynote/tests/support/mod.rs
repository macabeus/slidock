//! Builders for synthetic Keynote packages: IWA files written with the crate's own `prost` types and zipped the way
//! Keynote zips (STORED), so the reader runs end to end on synthetic presentations; plus zips, a seeded PRNG and
//! the structure-blind mutations of the fuzz tests.

#![allow(dead_code, reason = "each test file uses some of the builders")]

pub mod fixtures;
pub mod harness;
pub mod json;
pub mod keynote;
pub mod tables;

use indexmap::IndexMap;
use prost::Message;
use slidock_keynote::iwa::type_id;
use slidock_keynote::proto::{kn, tsp, tswp};

pub type Ref = tsp::Reference;

/// A reference to an object id.
pub fn r(id: u64) -> Ref {
    Ref {
        identifier: Some(id),
        ..Ref::default()
    }
}

pub fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

pub fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A zip of `files` in order, every entry STORED (`level` 0) or DEFLATE at `level`.
pub fn zip<N: AsRef<str>, D: AsRef<[u8]>>(files: &[(N, D)], level: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, content) in files {
        let (name, content) = (name.as_ref(), content.as_ref());
        let (method, data): (u16, Vec<u8>) = if level == 0 {
            (0, content.to_vec())
        } else {
            (8, miniz_oxide::deflate::compress_to_vec(content, level))
        };
        let offset = out.len() as u32;
        let crc = crc32(content);
        let common = |header: &mut Vec<u8>| {
            header.extend(20u16.to_le_bytes()); // version needed
            header.extend(0u16.to_le_bytes()); // flags
            header.extend(method.to_le_bytes());
            header.extend(0u16.to_le_bytes()); // time
            header.extend(0x21u16.to_le_bytes()); // date: 1980-01-01
            header.extend(crc.to_le_bytes());
            header.extend((data.len() as u32).to_le_bytes());
            header.extend((content.len() as u32).to_le_bytes());
            header.extend((name.len() as u16).to_le_bytes());
            header.extend(0u16.to_le_bytes()); // extra field
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        common(&mut out);
        out.extend(name.as_bytes());
        out.extend(&data);

        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes()); // made by
        common(&mut central);
        central.extend(0u16.to_le_bytes()); // comment
        central.extend(0u16.to_le_bytes()); // disk
        central.extend(0u16.to_le_bytes()); // internal attributes
        central.extend(0u32.to_le_bytes()); // external attributes
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let directory_offset = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(directory_offset.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

/// A STORED zip's files, in order.
pub fn unzip_stored(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let mut at = 0;
    while bytes.get(at..at + 4) == Some(&0x0403_4b50u32.to_le_bytes()[..]) {
        let u16_at = |i: usize| usize::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
        let size = u32::from_le_bytes(bytes[at + 18..at + 22].try_into().unwrap()) as usize;
        let (name_length, extra) = (u16_at(at + 26), u16_at(at + 28));
        let name = String::from_utf8(bytes[at + 30..at + 30 + name_length].to_vec()).unwrap();
        let start = at + 30 + name_length + extra;
        files.push((name, bytes[start..start + size].to_vec()));
        at = start + size;
    }
    files
}

/// One raw Snappy block holding `bytes` as a single literal: valid Snappy, stored uncompressed.
pub fn snappy_literal(bytes: &[u8]) -> Vec<u8> {
    let mut out = varint(bytes.len() as u64);
    if !bytes.is_empty() {
        let n = bytes.len() - 1;
        if n < 60 {
            out.push((n as u8) << 2);
        } else {
            let length_bytes = if n < 0x100 {
                1
            } else if n < 0x1_0000 {
                2
            } else if n < 0x100_0000 {
                3
            } else {
                4
            };
            out.push((59 + length_bytes) << 2);
            out.extend(&(n as u32).to_le_bytes()[..usize::from(length_bytes)]);
        }
    }
    out.extend(bytes);
    out
}

/// iWork framing: `[0x00][uint24 LE length][snappy block]` per chunk.
pub fn frame_iwa_chunked(data: &[u8], chunk_size: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(chunk_size).collect()
    };
    for chunk in chunks {
        let block = snappy_literal(chunk);
        let len = block.len();
        out.extend([0, len as u8, (len >> 8) as u8, (len >> 16) as u8]);
        out.extend(block);
    }
    out
}

pub fn frame_iwa(data: &[u8]) -> Vec<u8> {
    frame_iwa_chunked(data, 64 * 1024)
}

/// A record's type: a registered message name, a raw type id, or none in its MessageInfo.
#[derive(Clone, Copy, Debug)]
pub enum Type {
    Name(&'static str),
    Id(u32),
    Untyped,
}

impl Type {
    fn id(self) -> Option<u32> {
        match self {
            Type::Name(name) => {
                Some(type_id(name).unwrap_or_else(|| panic!("no registered type {name}")))
            }
            Type::Id(id) => Some(id),
            Type::Untyped => None,
        }
    }
}

/// One object of an IWA file.
#[derive(Clone, Debug)]
pub struct IwaRecord {
    pub id: u64,
    pub ty: Type,
    pub payload: Vec<u8>,
    pub should_merge: bool,
    /// Further payloads after the first (diff records).
    pub extra: Vec<Vec<u8>>,
}

impl IwaRecord {
    pub fn new(id: u64, name: &'static str, message: &impl Message) -> Self {
        Self::raw(id, Type::Name(name), message.encode_to_vec())
    }

    pub fn raw(id: u64, ty: Type, payload: Vec<u8>) -> Self {
        IwaRecord {
            id,
            ty,
            payload,
            should_merge: false,
            extra: Vec::new(),
        }
    }

    pub fn merging(mut self) -> Self {
        self.should_merge = true;
        self
    }

    pub fn with_extra(mut self, payload: Vec<u8>) -> Self {
        self.extra.push(payload);
        self
    }
}

/// An object stream, unframed.
pub fn iwa_stream(records: &[IwaRecord]) -> Vec<u8> {
    let mut out = Vec::new();
    for record in records {
        let type_id = record.ty.id();
        let payloads: Vec<&Vec<u8>> = std::iter::once(&record.payload)
            .chain(&record.extra)
            .collect();
        let info = tsp::ArchiveInfo {
            identifier: Some(record.id),
            message_infos: payloads
                .iter()
                .map(|bytes| tsp::MessageInfo {
                    r#type: type_id,
                    version: vec![1, 0, 5],
                    length: Some(bytes.len() as u32),
                    ..Default::default()
                })
                .collect(),
            should_merge: record.should_merge.then_some(true),
        }
        .encode_to_vec();
        out.extend(varint(info.len() as u64));
        out.extend(info);
        for payload in payloads {
            out.extend(payload);
        }
    }
    out
}

/// An IWA file's bytes.
pub fn iwa_file(records: &[IwaRecord]) -> Vec<u8> {
    frame_iwa(&iwa_stream(records))
}

pub const DOCUMENT: &str = "Index/Document.iwa";

enum Body {
    Bytes(Vec<u8>),
    Node(Box<kn::SlideNodeArchive>),
}

struct Record {
    id: u64,
    ty: Type,
    body: Body,
}

/// A synthetic presentation: one IWA object graph spread over the files Keynote uses. `add` returns a reference to
/// the new object; `files` writes the package's files and `bytes` the `.key` zip.
pub struct PresentationBuilder {
    next_id: u64,
    records: IndexMap<String, Vec<Record>>,
    datas: Vec<tsp::DataInfo>,
    data_files: Vec<(String, Vec<u8>)>,
    slide_nodes: Vec<Ref>,
    templates: Vec<Ref>,
}

impl Default for PresentationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PresentationBuilder {
    pub fn new() -> Self {
        PresentationBuilder {
            next_id: 1000,
            records: IndexMap::new(),
            datas: Vec::new(),
            data_files: Vec::new(),
            slide_nodes: Vec::new(),
            templates: Vec::new(),
        }
    }

    fn push(&mut self, ty: Type, body: Body, file: &str) -> Ref {
        let id = self.next_id;
        self.next_id += 1;
        self.records
            .entry(file.to_owned())
            .or_default()
            .push(Record { id, ty, body });
        r(id)
    }

    pub fn add(&mut self, name: &'static str, message: &impl Message) -> Ref {
        self.add_in(name, message, DOCUMENT)
    }

    pub fn add_in(&mut self, name: &'static str, message: &impl Message, file: &str) -> Ref {
        self.push(Type::Name(name), Body::Bytes(message.encode_to_vec()), file)
    }

    /// An object with raw payload bytes.
    pub fn add_raw(&mut self, ty: Type, payload: Vec<u8>, file: &str) -> Ref {
        self.push(ty, Body::Bytes(payload), file)
    }

    /// A file under Data/, listed in the package's media table.
    pub fn data(&mut self, file_name: &str, bytes: Vec<u8>) -> tsp::DataReference {
        let identifier = self.next_id;
        self.next_id += 1;
        self.datas.push(tsp::DataInfo {
            identifier: Some(identifier),
            file_name: Some(file_name.to_owned()),
            preferred_file_name: Some(file_name.to_owned()),
            ..Default::default()
        });
        self.data_files.push((format!("Data/{file_name}"), bytes));
        tsp::DataReference {
            identifier: Some(identifier),
        }
    }

    /// A text storage with one paragraph style for the whole text.
    pub fn storage(&mut self, text: &str, paragraph_style: Option<Ref>) -> Ref {
        let storage = tswp::StorageArchive {
            text: vec![text.to_owned()],
            table_para_style: paragraph_style.map(|style| tswp::ObjectAttributeTable {
                entries: vec![tswp::object_attribute_table::ObjectAttribute {
                    character_index: Some(0),
                    object: Some(style),
                }],
            }),
            ..Default::default()
        };
        self.add("TSWP.StorageArchive", &storage)
    }

    pub fn paragraph_style(
        &mut self,
        char_properties: tswp::CharacterStylePropertiesArchive,
        parent: Option<Ref>,
    ) -> Ref {
        let style = tswp::ParagraphStyleArchive {
            super_: Some(style(parent)),
            char_properties: Some(char_properties),
            para_properties: Some(Default::default()),
            ..Default::default()
        };
        self.add_in(
            "TSWP.ParagraphStyleArchive",
            &style,
            "Index/DocumentStylesheet.iwa",
        )
    }

    /// A template slide (a layout) listed by the theme; returns the template's SlideArchive.
    pub fn template(&mut self, name: &str, drawables: Vec<Ref>) -> Ref {
        let slide = kn::SlideArchive {
            name: Some(name.to_owned()),
            drawables_z_order: drawables,
            ..Default::default()
        };
        let slide = self.add_in("KN.SlideArchive", &slide, "Index/TemplateSlide.iwa");
        let node = kn::SlideNodeArchive {
            slide: Some(slide),
            ..Default::default()
        };
        let node = self.add("KN.SlideNodeArchive", &node);
        self.templates.push(node);
        slide
    }

    /// A slide at the top of the slide tree (or under `parent`), returning its node.
    pub fn slide(
        &mut self,
        slide: kn::SlideArchive,
        node: kn::SlideNodeArchive,
        parent: Option<&Ref>,
    ) -> Ref {
        let slide = self.add_in("KN.SlideArchive", &slide, "Index/Slide.iwa");
        let node = kn::SlideNodeArchive {
            slide: Some(slide),
            ..node
        };
        let node = self.push(
            Type::Name("KN.SlideNodeArchive"),
            Body::Node(Box::new(node)),
            DOCUMENT,
        );
        match parent {
            Some(parent) => {
                let parent_node = self
                    .records
                    .values_mut()
                    .flatten()
                    .find(|record| Some(record.id) == parent.identifier)
                    .and_then(|record| match &mut record.body {
                        Body::Node(node) => Some(node),
                        Body::Bytes(_) => None,
                    })
                    .unwrap_or_else(|| panic!("no slide node {parent:?}"));
                parent_node.children.push(node);
            }
            None => self.slide_nodes.push(node),
        }
        node
    }

    /// Another node at the top of the slide tree, showing the slide `node` shows; returns the new node.
    pub fn show_again(&mut self, node: &Ref) -> Ref {
        let slide = self
            .records
            .values()
            .flatten()
            .find(|record| Some(record.id) == node.identifier)
            .and_then(|record| match &record.body {
                Body::Node(node) => node.slide,
                Body::Bytes(_) => None,
            })
            .unwrap_or_else(|| panic!("no slide node {node:?}"));
        let again = kn::SlideNodeArchive {
            slide: Some(slide),
            ..Default::default()
        };
        let again = self.push(
            Type::Name("KN.SlideNodeArchive"),
            Body::Node(Box::new(again)),
            DOCUMENT,
        );
        self.slide_nodes.push(again);
        again
    }

    /// The package's files at the default size.
    pub fn files(&mut self) -> Vec<(String, Vec<u8>)> {
        self.files_sized(1920.0, 1080.0)
    }

    pub fn files_sized(&mut self, width: f32, height: f32) -> Vec<(String, Vec<u8>)> {
        let theme = kn::ThemeArchive {
            templates: self.templates.clone(),
            ..Default::default()
        };
        let theme = self.add("KN.ThemeArchive", &theme);
        let show = kn::ShowArchive {
            theme: Some(theme),
            slide_tree: Some(kn::SlideTreeArchive {
                slides: self.slide_nodes.clone(),
                ..Default::default()
            }),
            size: Some(tsp::Size {
                width: Some(width),
                height: Some(height),
            }),
            ..Default::default()
        };
        let show = self.add("KN.ShowArchive", &show);
        let document = kn::DocumentArchive {
            show: Some(show),
            ..Default::default()
        };
        self.add("KN.DocumentArchive", &document);
        let metadata = tsp::PackageMetadata {
            datas: self.datas.clone(),
            ..Default::default()
        };
        self.add_in("TSP.PackageMetadata", &metadata, "Index/Metadata.iwa");
        let mut files = self.data_files.clone();
        for (file, records) in &self.records {
            let records: Vec<IwaRecord> = records
                .iter()
                .map(|record| {
                    let payload = match &record.body {
                        Body::Bytes(bytes) => bytes.clone(),
                        Body::Node(node) => node.encode_to_vec(),
                    };
                    IwaRecord::raw(record.id, record.ty, payload)
                })
                .collect();
            files.push((file.clone(), iwa_file(&records)));
        }
        files
    }

    pub fn bytes(&mut self) -> Vec<u8> {
        zip(&self.files(), 0)
    }
}

/// A style's `super` naming `parent`.
pub fn style(parent: Option<Ref>) -> slidock_keynote::proto::tss::StyleArchive {
    slidock_keynote::proto::tss::StyleArchive {
        parent,
        ..Default::default()
    }
}

/// A seeded PRNG (mulberry32), so fuzz runs are reproducible from their seed.
pub struct Seeded(u32);

impl Seeded {
    pub fn new(seed: u32) -> Self {
        Seeded(seed)
    }

    pub fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() * n as f64) as usize
    }
}

/// One structure-blind mutation: a truncation, a byte flip, a run of random bytes, or a duplicated slice.
pub fn mutate(bytes: &[u8], random: &mut Seeded) -> Vec<u8> {
    let at = random.below(bytes.len());
    let mut out = bytes.to_vec();
    match random.below(4) {
        0 => out.truncate(at),
        1 => {
            let bit = random.below(8);
            if let Some(byte) = out.get_mut(at) {
                *byte ^= 1 << bit;
            }
        }
        2 => {
            for i in at..out.len().min(at + 8) {
                out[i] = random.below(256) as u8;
            }
        }
        _ => {
            let length = random.below(64);
            let copied = length.min(out.len() - at);
            let mut copy = vec![0u8; out.len() + length];
            copy[..at].copy_from_slice(&out[..at]);
            copy[at..at + copied].copy_from_slice(&out[at..at + copied]);
            copy[at + copied..at + copied + out.len() - at].copy_from_slice(&out[at..]);
            return copy;
        }
    }
    out
}

/// How many mutated inputs each fuzz loop runs (`FUZZ_ITERATIONS`, default 200).
pub fn fuzz_iterations() -> usize {
    std::env::var("FUZZ_ITERATIONS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(200)
}
