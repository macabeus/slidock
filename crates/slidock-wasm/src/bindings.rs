//! The wasm-bindgen surface the worker calls: [`Compiler`] over a JavaScript [`RangeSource`] (the input `File` read
//! with `FileReaderSync`). Results cross as plain objects parsed from their JSON (the types of [`crate::results`]),
//! bytes as `Uint8Array`s, and requests' settings as JSON text. A failed stage throws the JSON of a [`WorkerError`];
//! anything else thrown is a crash.

use std::borrow::Cow;

use js_sys::{Function, Uint8Array};
use serde::Serialize;
use slidock::deck::DeckDiagnostic;
use slidock::{
    BundleError, BundleOptions, Diagnostic, RangeError, RangeReader, Sha256, check_range,
};
use slidock_fonts::face_css::weight_value;
use slidock_io::{ZipArchive, ZipLimits};
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsCast, JsValue};

use crate::host::{Host, HostError, MediaBytes, Opened, OpenedMedia, Printed, Swaps};
use crate::protocol::{
    AddedFontSource, BundleSettings, ErrorKind, FontIssue, FontNeed, GoogleFace, HeicPlan,
    LocalFontMeta, LocalPick, PlannedImage, PlannedVideo, SubstituteFace, WorkerError,
};
use crate::results::{
    DocumentsJson, FontJson, MediaJson, MediaNeedJson, OpenedJson, PrintedJson, RangeJson,
    ZipEntryJson,
};

#[wasm_bindgen(typescript_custom_section)]
const RANGE_SOURCE: &str = r#"
/** The input, read by byte range. */
export interface RangeSource {
  /** In bytes. */
  readonly size: number;
  /** Exactly `length` bytes from `offset`. */
  read(offset: number, length: number): Uint8Array;
}
"#;

#[wasm_bindgen(typescript_custom_section)]
const RESULTS: &str = crate::results::DECLARATIONS;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "RangeSource")]
    pub type RangeSource;

    #[wasm_bindgen(method, getter)]
    fn size(this: &RangeSource) -> f64;

    #[wasm_bindgen(method, catch)]
    fn read(this: &RangeSource, offset: f64, length: u32) -> Result<Uint8Array, JsValue>;
}

/// The largest integer a JavaScript number holds exactly.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// What a JavaScript exception says.
fn describe(value: &JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            value
                .dyn_ref::<js_sys::Error>()
                .map(|error| String::from(error.message()))
        })
        .unwrap_or_else(|| format!("{value:?}"))
}

/// A [`RangeSource`] as a [`RangeReader`]: each read is one call into JavaScript, copied into wasm memory.
struct JsReader {
    source: RangeSource,
    len: u64,
}

impl JsReader {
    fn new(source: RangeSource) -> Result<JsReader, HostError> {
        let size = source.size();
        if !(size.is_finite()
            && size >= 0.0
            && size.fract() == 0.0
            && size <= MAX_SAFE_INTEGER as f64)
        {
            return Err(host_error(
                ErrorKind::Request,
                format!("{size} is not the size of a file"),
            ));
        }
        Ok(JsReader {
            source,
            len: size as u64,
        })
    }
}

impl RangeReader for JsReader {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        check_range(self.len, offset, len as u64)?;
        let io = |message: String| RangeError::Io {
            offset,
            len: len as u64,
            message,
        };
        let length = u32::try_from(len).map_err(|_| io("a read longer than 4 GiB".to_owned()))?;
        let bytes = self
            .source
            .read(offset as f64, length)
            .map_err(|error| io(describe(&error)))?;
        if bytes.length() != length {
            return Err(io(format!(
                "the source gave {} bytes for {length}",
                bytes.length()
            )));
        }
        Ok(Cow::Owned(bytes.to_vec()))
    }
}

fn host_error(kind: ErrorKind, message: String) -> HostError {
    HostError {
        kind,
        message,
        diagnostics: Vec::new(),
    }
}

/// The exception a failed stage throws: its [`WorkerError`] as JSON.
fn thrown(error: &HostError) -> JsValue {
    match serde_json::to_string(&WorkerError::from(error)) {
        Ok(json) => JsValue::from_str(&json),
        Err(error) => JsValue::from_str(&error.to_string()),
    }
}

/// `value` as a plain JavaScript object: its JSON, parsed.
fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    let internal = |message: String| thrown(&host_error(ErrorKind::Internal, message));
    let json = serde_json::to_string(value).map_err(|error| internal(error.to_string()))?;
    js_sys::JSON::parse(&json).map_err(|error| internal(describe(&error)))
}

fn diagnostics(diagnostics: &[Diagnostic]) -> Vec<DeckDiagnostic> {
    diagnostics.iter().map(DeckDiagnostic::from).collect()
}

fn needs(needs: &[slidock::FontNeed]) -> Vec<FontNeed> {
    needs.iter().map(FontNeed::from).collect()
}

impl From<&OpenedMedia> for MediaNeedJson {
    fn from(need: &OpenedMedia) -> Self {
        MediaNeedJson {
            id: need.sha256.to_hex(),
            mime: need.mime.clone(),
            range: match need.bytes {
                MediaBytes::Range { offset, len } => Some(RangeJson {
                    offset,
                    length: len,
                }),
                MediaBytes::Written => None,
            },
            plan: need.plan.as_ref().ok().map(HeicPlan::from),
            refusal: need.plan.as_ref().err().cloned(),
        }
    }
}

impl From<&Opened> for OpenedJson {
    fn from(opened: &Opened) -> Self {
        OpenedJson {
            diagnostics: diagnostics(&opened.diagnostics),
            needs: needs(&opened.needs),
            media: opened.media.iter().map(MediaNeedJson::from).collect(),
        }
    }
}

impl<'a> From<&'a Printed> for PrintedJson<'a> {
    fn from(printed: &'a Printed) -> Self {
        PrintedJson {
            deck: &printed.deck,
            slides: &printed.slides,
            media: printed
                .media
                .iter()
                .map(|media| MediaJson {
                    path: media.path.clone(),
                    mime: media.mime.clone(),
                    range: match media.bytes {
                        MediaBytes::Range { offset, len } => Some(RangeJson {
                            offset,
                            length: len,
                        }),
                        MediaBytes::Written => None,
                    },
                })
                .collect(),
            diagnostics: diagnostics(&printed.diagnostics),
            fonts: printed
                .font_rules()
                .map(|rule| FontJson {
                    family: rule.family.clone(),
                    weight: weight_value(rule.descriptor.weight),
                    style: rule.descriptor.style,
                    unicode_range: rule
                        .unicode_range
                        .as_ref()
                        .and_then(|set| set.unicode_range()),
                    format: rule.format.css().to_owned(),
                })
                .collect(),
            faces: printed.faces(),
            font_issues: printed.font_issues.iter().map(FontIssue::from).collect(),
            substitutes: printed
                .substitutes
                .iter()
                .map(SubstituteFace::from)
                .collect(),
            page: slidock::page_defaults(&printed.deck),
        }
    }
}

/// `value` read from a JSON argument; a value that does not parse is a request error.
fn from_json<T: serde::de::DeserializeOwned>(what: &str, json: &str) -> Result<T, JsValue> {
    serde_json::from_str(json)
        .map_err(|error| thrown(&host_error(ErrorKind::Request, format!("{what}: {error}"))))
}

/// Lists the entries of the zip `source` as the compiler's zip reader reads them. For checking a packed folder.
#[wasm_bindgen(js_name = zipEntries, unchecked_return_type = "ZipEntry[]")]
pub fn zip_entries(source: RangeSource) -> Result<JsValue, JsValue> {
    let fail = |message: String| thrown(&host_error(ErrorKind::Refused, message));
    let reader = JsReader::new(source).map_err(|error| thrown(&error))?;
    let zip =
        ZipArchive::open(reader, ZipLimits::default()).map_err(|error| fail(error.message))?;
    let mut entries = Vec::with_capacity(zip.entries().len());
    for entry in zip.entries() {
        let bytes = zip.read(entry).map_err(|error| fail(error.message))?;
        entries.push(ZipEntryJson {
            name: entry.name.clone(),
            size: entry.size,
            stored: entry.method == slidock_io::zip::STORED,
            sha256: Sha256::of(&bytes).to_hex(),
        });
    }
    to_js(&entries)
}

/// The compiler as the worker drives it: one open presentation, one stage per call.
#[wasm_bindgen]
pub struct Compiler {
    host: Host,
}

impl Default for Compiler {
    fn default() -> Self {
        Compiler::new()
    }
}

#[wasm_bindgen]
impl Compiler {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Compiler {
        Compiler { host: Host::new() }
    }

    /// Opens `source` as the presentation `name`, replacing any open one. With `fonts`, its embedded fonts are added
    /// and `finish` ships the faces the deck draws; without, nothing ships (the parity path).
    #[wasm_bindgen(unchecked_return_type = "Opened")]
    pub fn open(
        &mut self,
        name: &str,
        source: RangeSource,
        fonts: bool,
    ) -> Result<JsValue, JsValue> {
        let reader = JsReader::new(source).map_err(|error| thrown(&error))?;
        let opened = self
            .host
            .open(name, Box::new(reader), fonts)
            .map_err(|error| thrown(&error))?;
        to_js(&OpenedJson::from(&opened))
    }

    /// The faces no font provides yet.
    #[wasm_bindgen(js_name = fontNeeds, unchecked_return_type = "FontNeed[]")]
    pub fn font_needs(&self) -> Result<JsValue, JsValue> {
        let found = self.host.font_needs().map_err(|error| thrown(&error))?;
        to_js(&needs(&found))
    }

    /// Adds a font file from `source`; returns what is still missing. `unicodeRange` marks one per-script chunk of
    /// a face (a `googleFaces` face's range).
    #[wasm_bindgen(js_name = addFont, unchecked_return_type = "FontNeed[]")]
    pub fn add_font(
        &mut self,
        bytes: Vec<u8>,
        #[wasm_bindgen(unchecked_param_type = "AddedFontSource")] source: &str,
        #[wasm_bindgen(js_name = unicodeRange)] unicode_range: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let source: AddedFontSource = serde_json::from_value(serde_json::Value::from(source))
            .map_err(|_| {
                thrown(&host_error(
                    ErrorKind::Request,
                    format!("{source} is not a font source"),
                ))
            })?;
        let found = self
            .host
            .add_font(bytes, source.into(), unicode_range.as_deref())
            .map_err(|error| thrown(&error))?;
        to_js(&needs(&found))
    }

    /// The media the app may decode (HEIC photos), each with its decode plan or why it has none.
    #[wasm_bindgen(js_name = mediaNeeds, unchecked_return_type = "OpenedMediaNeed[]")]
    pub fn media_needs(&self) -> Result<JsValue, JsValue> {
        let found = self.host.media_needs().map_err(|error| thrown(&error))?;
        to_js(&found.iter().map(MediaNeedJson::from).collect::<Vec<_>>())
    }

    /// The source file of the media need `need` (one with no `range`): what its plan's ranges index.
    #[wasm_bindgen(js_name = mediaNeedBytes)]
    pub fn media_need_bytes(&self, need: &str) -> Result<Vec<u8>, JsValue> {
        self.host.need_bytes(need).map_err(|error| thrown(&error))
    }

    /// Adds the app's decode of the media need `need`: a PNG or JPEG of its plan's output size, for the next
    /// `finish`. Another file is refused (a `request` error) and the medium stays omitted.
    #[wasm_bindgen(js_name = addMedia)]
    pub fn add_media(&mut self, need: &str, bytes: Vec<u8>) -> Result<(), JsValue> {
        self.host
            .add_media(need, bytes)
            .map_err(|error| thrown(&error))
    }

    /// The installed faces worth reading for the faces still open, of `fonts` (a JSON `LocalFontMeta[]`).
    #[wasm_bindgen(js_name = selectLocal, unchecked_return_type = "LocalPick[]")]
    pub fn select_local(&self, fonts: &str) -> Result<JsValue, JsValue> {
        let metas: Vec<LocalFontMeta> = from_json("local fonts", fonts)?;
        let metas: Vec<slidock_fonts::resolve::LocalFontMeta> =
            metas.iter().map(Into::into).collect();
        let picks = self
            .host
            .select_local(&metas)
            .map_err(|error| thrown(&error))?;
        to_js(&picks.iter().map(LocalPick::from).collect::<Vec<_>>())
    }

    /// The faces of a Google Fonts `css2` response worth fetching.
    #[wasm_bindgen(js_name = googleFaces, unchecked_return_type = "GoogleFace[]")]
    pub fn google_faces(&self, css: &str) -> Result<JsValue, JsValue> {
        let faces = self
            .host
            .google_faces(css)
            .map_err(|error| thrown(&error))?;
        to_js(&faces.iter().map(GoogleFace::from).collect::<Vec<_>>())
    }

    /// The file of the printed deck's `index`th shipped font rule (`Printed.fonts`).
    #[wasm_bindgen(js_name = fontBytes)]
    pub fn font_bytes(&self, index: u32) -> Result<Vec<u8>, JsValue> {
        self.host
            .font_bytes(index as usize)
            .map_err(|error| thrown(&error))
    }

    /// Runs the passes from bind-fonts with the fonts added so far.
    pub fn finish(&mut self) -> Result<(), JsValue> {
        self.host.finish().map_err(|error| thrown(&error))
    }

    /// Prints what `finish` resolved; a medium with no range is read with `mediaBytes`.
    #[wasm_bindgen(unchecked_return_type = "Printed")]
    pub fn print(&mut self) -> Result<JsValue, JsValue> {
        let printed = self.host.print().map_err(|error| thrown(&error))?;
        to_js(&PrintedJson::from(&printed))
    }

    /// The bytes of the printed deck's file `path` under `media/`.
    #[wasm_bindgen(js_name = mediaBytes)]
    pub fn media_bytes(&self, path: &str) -> Result<Vec<u8>, JsValue> {
        self.host.media_bytes(path).map_err(|error| thrown(&error))
    }

    /// The pictures of the printed deck the app may re-encode smaller, forgetting those it handed over before.
    #[wasm_bindgen(js_name = imagePlan, unchecked_return_type = "PlannedImage[]")]
    pub fn image_plan(&mut self) -> Result<JsValue, JsValue> {
        let images = self.host.image_plan().map_err(|error| thrown(&error))?;
        to_js(&images.iter().map(PlannedImage::from).collect::<Vec<_>>())
    }

    /// Takes the app's re-encode of the planned picture `path` for the bundles with `compressImages`. A file the
    /// compiler refuses is a `request` error, and the picture stays as it is.
    #[wasm_bindgen(js_name = addImage)]
    pub fn add_image(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), JsValue> {
        self.host
            .add_image(path, bytes)
            .map_err(|error| thrown(&error))
    }

    /// The videos of the printed deck, each with the size the app may re-encode it at or why it stays as it is,
    /// forgetting those it handed over before.
    #[wasm_bindgen(js_name = videoPlan, unchecked_return_type = "PlannedVideo[]")]
    pub fn video_plan(&mut self) -> Result<JsValue, JsValue> {
        let videos = self.host.video_plan().map_err(|error| thrown(&error))?;
        to_js(&videos.iter().map(PlannedVideo::from).collect::<Vec<_>>())
    }

    /// Takes the app's re-encode of the planned video `path`, read from `source` as it is checked and bundled, for
    /// the bundles with `compressVideos`. A file the compiler refuses is a `request` error, and the video stays as it
    /// is.
    #[wasm_bindgen(js_name = addVideo)]
    pub fn add_video(&mut self, path: &str, source: RangeSource) -> Result<(), JsValue> {
        let reader = JsReader::new(source).map_err(|error| thrown(&error))?;
        self.host
            .add_video(path, Box::new(reader))
            .map_err(|error| thrown(&error))
    }

    /// The videos of the printed deck that get a copy other browsers play, each with the size to make it at,
    /// forgetting the copies it took before.
    #[wasm_bindgen(js_name = alternatePlan, unchecked_return_type = "PlannedVideo[]")]
    pub fn alternate_plan(&mut self) -> Result<JsValue, JsValue> {
        let videos = self.host.alternate_plan().map_err(|error| thrown(&error))?;
        to_js(&videos.iter().map(PlannedVideo::from).collect::<Vec<_>>())
    }

    /// Takes the app's copy of the video `path`, read from `source` as it is checked and bundled, for the bundles
    /// with `alternateVideos`. A file the compiler refuses is a `request` error, and the video has no copy.
    #[wasm_bindgen(js_name = addAlternate)]
    pub fn add_alternate(&mut self, path: &str, source: RangeSource) -> Result<(), JsValue> {
        let reader = JsReader::new(source).map_err(|error| thrown(&error))?;
        self.host
            .add_alternate(path, Box::new(reader))
            .map_err(|error| thrown(&error))
    }

    /// The document set of the last printed deck (`--emit documents`). For parity checks.
    #[wasm_bindgen(unchecked_return_type = "DocumentSet")]
    pub fn documents(&self) -> Result<JsValue, JsValue> {
        let documents = self.host.documents().map_err(|error| thrown(&error))?;
        to_js(&DocumentsJson {
            text: &documents.text,
            media: documents.media.keys().collect(),
        })
    }

    /// Bundles the last printed deck as a zip with `settings` (a JSON `BundleSettings`), calling `sink` with each
    /// chunk of at most 1 MiB, in order: a fresh `Uint8Array` the sink may keep or transfer. An exception from
    /// `sink` stops the bundle with an error of kind `bundle`.
    #[wasm_bindgen(unchecked_return_type = "BundleReport")]
    pub fn bundle(
        &mut self,
        settings: &str,
        #[wasm_bindgen(unchecked_param_type = "(chunk: Uint8Array) => void")] sink: &Function,
    ) -> Result<JsValue, JsValue> {
        let settings: BundleSettings = from_json("bundle settings", settings)?;
        let mut out = |chunk: &[u8]| {
            let array = Uint8Array::new_with_length(chunk.len() as u32);
            array.copy_from(chunk);
            sink.call1(&JsValue::NULL, &array)
                .map(|_| ())
                .map_err(|error| BundleError::Sink(describe(&error)))
        };
        let report = self
            .host
            .bundle(
                &BundleOptions::from(&settings),
                Swaps {
                    images: settings.compress_images == Some(true),
                    videos: settings.compress_videos == Some(true),
                    alternates: settings.alternate_videos == Some(true),
                },
                &mut out,
            )
            .map_err(|error| thrown(&error))?;
        to_js(&report)
    }
}
