//! A synthetic presentation written from JSON objects, as Keynote spreads one object graph over its files: `add`
//! returns a reference to the new object (`{"identifier": "<id>"}`, ids from 1000 in creation order); `files` writes
//! the package's files and `bytes` the `.key` zip, every entry STORED.

use indexmap::IndexMap;
use serde_json::{Value, json};

use super::json::encode;
use super::{IwaRecord, Type, iwa_file, zip};

pub const DOCUMENT: &str = "Index/Document.iwa";
pub const SLIDE: &str = "Index/Slide.iwa";
pub const STYLESHEET: &str = "Index/DocumentStylesheet.iwa";
pub const TEMPLATE_SLIDE: &str = "Index/TemplateSlide.iwa";

enum Body {
    Json(&'static str, Value),
    Raw(Type, Vec<u8>),
}

struct Record {
    id: u64,
    body: Body,
}

#[derive(Default)]
pub struct Presentation {
    next_id: u64,
    records: IndexMap<String, Vec<Record>>,
    datas: Vec<Value>,
    data_files: IndexMap<String, Vec<u8>>,
    slide_nodes: Vec<Value>,
    templates: Vec<Value>,
}

/// A reference to an object id.
pub fn r(id: u64) -> Value {
    json!({ "identifier": id.to_string() })
}

/// The object id a reference names.
pub fn id_of(reference: &Value) -> u64 {
    reference["identifier"].as_str().unwrap().parse().unwrap()
}

/// `{...a, ...b}`: `b`'s keys over `a`'s.
pub fn merge(a: Value, b: Value) -> Value {
    let mut out = a.as_object().cloned().unwrap_or_default();
    for (key, value) in b.as_object().cloned().unwrap_or_default() {
        out.insert(key, value);
    }
    Value::Object(out)
}

impl Presentation {
    pub fn new() -> Self {
        Presentation {
            next_id: 1000,
            ..Presentation::default()
        }
    }

    fn push(&mut self, body: Body, file: &str) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.records
            .entry(file.to_owned())
            .or_default()
            .push(Record { id, body });
        r(id)
    }

    pub fn add(&mut self, type_name: &'static str, message: Value) -> Value {
        self.add_in(type_name, message, DOCUMENT)
    }

    pub fn add_in(&mut self, type_name: &'static str, message: Value, file: &str) -> Value {
        self.push(Body::Json(type_name, message), file)
    }

    /// An object with raw payload bytes, of a registered type or a raw type id.
    pub fn add_record(&mut self, ty: Type, payload: Vec<u8>) -> Value {
        self.push(Body::Raw(ty, payload), DOCUMENT)
    }

    /// A file under Data/, listed in the package's media table.
    pub fn data(&mut self, file_name: &str, bytes: Vec<u8>) -> Value {
        let identifier = self.next_id.to_string();
        self.next_id += 1;
        self.datas.push(json!({
            "identifier": identifier, "file_name": file_name, "preferred_file_name": file_name,
        }));
        self.data_files.insert(format!("Data/{file_name}"), bytes);
        json!({ "identifier": identifier })
    }

    /// A text storage with one paragraph style for the whole text.
    pub fn storage(&mut self, text: &str, paragraph_style: Option<Value>, extra: Value) -> Value {
        let mut storage = json!({ "text": [text] });
        if let Some(style) = paragraph_style {
            storage["table_para_style"] =
                json!({ "entries": [{ "character_index": 0, "object": style }] });
        }
        self.add("TSWP.StorageArchive", merge(storage, extra))
    }

    pub fn paragraph_style(
        &mut self,
        char_properties: Value,
        parent: Option<Value>,
        para_properties: Value,
    ) -> Value {
        let parent = parent.map_or_else(|| json!({}), |parent| json!({ "parent": parent }));
        self.add_in(
            "TSWP.ParagraphStyleArchive",
            json!({ "super": parent, "char_properties": char_properties, "para_properties": para_properties }),
            STYLESHEET,
        )
    }

    pub fn text_box(&mut self, text: Value, geometry: Value, style: Option<Value>) -> Value {
        let mut shape = json!({ "super": { "geometry": geometry } });
        if let Some(style) = style {
            shape["style"] = style;
        }
        self.add_in(
            "TSWP.ShapeInfoArchive",
            json!({ "super": shape, "owned_storage": text, "is_text_box": true }),
            SLIDE,
        )
    }

    /// A template slide (a layout) listed by the theme; returns the template's SlideArchive.
    pub fn template(&mut self, name: &str, drawables: Vec<Value>, extra: Value) -> Value {
        let slide = self.add_in(
            "KN.SlideArchive",
            merge(
                json!({ "name": name, "drawables_z_order": drawables }),
                extra,
            ),
            TEMPLATE_SLIDE,
        );
        let node = self.add("KN.SlideNodeArchive", json!({ "slide": slide }));
        self.templates.push(node);
        slide
    }

    /// A slide at the top of the slide tree (or under `parent`), returning its node.
    pub fn slide(&mut self, slide: Value, node: Value, parent: Option<&Value>) -> Value {
        let slide = self.add_in("KN.SlideArchive", slide, SLIDE);
        let node = self.add(
            "KN.SlideNodeArchive",
            merge(json!({ "slide": slide, "children": [] }), node),
        );
        match parent {
            Some(parent) => {
                let parent = id_of(parent);
                let message = self
                    .records
                    .values_mut()
                    .flatten()
                    .find(|record| record.id == parent)
                    .and_then(|record| match &mut record.body {
                        Body::Json(_, message) => Some(message),
                        Body::Raw(..) => None,
                    })
                    .unwrap_or_else(|| panic!("no object {parent}"));
                message["children"]
                    .as_array_mut()
                    .unwrap()
                    .push(node.clone());
            }
            None => self.slide_nodes.push(node.clone()),
        }
        node
    }

    pub fn files(&mut self, width: f64, height: f64) -> Vec<(String, Vec<u8>)> {
        let theme = self.add(
            "KN.ThemeArchive",
            json!({ "templates": self.templates.clone() }),
        );
        let show = self.add(
            "KN.ShowArchive",
            json!({ "theme": theme, "slideTree": { "slides": self.slide_nodes.clone() },
                    "size": { "width": width, "height": height } }),
        );
        self.add("KN.DocumentArchive", json!({ "show": show }));
        self.add_in(
            "TSP.PackageMetadata",
            json!({ "datas": self.datas.clone() }),
            "Index/Metadata.iwa",
        );
        let mut files: Vec<(String, Vec<u8>)> = self.data_files.clone().into_iter().collect();
        for (file, records) in &self.records {
            let records: Vec<IwaRecord> = records
                .iter()
                .map(|record| match &record.body {
                    Body::Json(name, message) => {
                        IwaRecord::raw(record.id, Type::Name(name), encode(name, message))
                    }
                    Body::Raw(ty, payload) => IwaRecord::raw(record.id, *ty, payload.clone()),
                })
                .collect();
            files.push((file.clone(), iwa_file(&records)));
        }
        files
    }

    /// The `.key` zip at the default size.
    pub fn bytes(&mut self) -> Vec<u8> {
        self.bytes_sized(1920.0, 1080.0)
    }

    pub fn bytes_sized(&mut self, width: f64, height: f64) -> Vec<u8> {
        zip(&self.files(width, height), 0)
    }
}

/// A drawable's geometry: position, size, flags (3: both dimensions are the box's own) and an angle.
pub fn geometry(x: f64, y: f64, width: f64, height: f64) -> Value {
    geometry_with(x, y, width, height, 3, None)
}

pub fn geometry_with(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    flags: u32,
    angle: Option<f64>,
) -> Value {
    let mut out = json!({ "position": { "x": x, "y": y }, "size": { "width": width, "height": height }, "flags": flags });
    if let Some(angle) = angle {
        out["angle"] = json!(angle);
    }
    out
}

pub fn rgb(r: f64, g: f64, b: f64) -> Value {
    json!({ "model": 1, "r": r, "g": g, "b": b, "a": 1 })
}

fn iso_box(kind: &str, children: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = children.concat();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend(u32::try_from(8 + body.len()).unwrap().to_be_bytes());
    out.extend(kind.as_bytes());
    out.extend(body);
    out
}

/// A minimal ISO media file: one track with this handler and sample entry, `moov` before or after `mdat`, and a
/// movie header stating a length in milliseconds (or none).
pub fn movie(handler: &str, codec: &str, moov_first: bool, duration_ms: Option<u32>) -> Vec<u8> {
    let zeros = |n: usize| vec![0u8; n];
    let stsd = iso_box(
        "stsd",
        &[
            zeros(4),
            vec![0, 0, 0, 1],
            vec![0, 0, 0, 16],
            codec.as_bytes().to_vec(),
            zeros(4),
        ],
    );
    let trak = iso_box(
        "trak",
        &[
            iso_box("tkhd", &[zeros(8)]),
            iso_box(
                "mdia",
                &[
                    iso_box("hdlr", &[zeros(8), handler.as_bytes().to_vec(), zeros(12)]),
                    iso_box("minf", &[iso_box("stbl", &[stsd])]),
                ],
            ),
        ],
    );
    let mvhd = match duration_ms {
        None => iso_box("mvhd", &[zeros(8)]),
        Some(ms) => iso_box(
            "mvhd",
            &[
                zeros(12),
                1000u32.to_be_bytes().to_vec(),
                ms.to_be_bytes().to_vec(),
            ],
        ),
    };
    let moov = iso_box("moov", &[mvhd, trak]);
    let ftyp = iso_box("ftyp", &[b"isom".to_vec(), zeros(4)]);
    let mdat = iso_box("mdat", &[zeros(1000)]);
    if moov_first {
        [ftyp, moov, mdat].concat()
    } else {
        [ftyp, mdat, moov].concat()
    }
}
