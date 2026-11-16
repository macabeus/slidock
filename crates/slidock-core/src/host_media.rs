//! The media a host decodes for the compiler, as it adds fonts: the HEIC photos the presentation shows
//! ([`HOST_DECODED`]), which no browser but Safari draws and the compiler does not decode. [`media_needs`] lists each
//! with the plan of its decode ([`slidock_io::read_heif`]), or why there is none; the host answers a need with a PNG
//! or JPEG of the plan's output size, which [`check_decoded`] checks before plan-media takes it. The output stays a
//! function of its inputs: the source and the files the host added.

use slidock_core_api::visit::container_media;
use slidock_core_api::{
    HOST_DECODED, HostDecoded, MediaHandle, MediaPolicy, MediaSource, MediaStore,
};
use slidock_io::{
    BytesReader, HeifError, HeifImage, HeifLimits, Mime, RangeReader, SubReader, image_size,
    jpeg_orientation, read_heif, sniff_mime,
};
use slidock_ir::ids::Sha256;
use slidock_ir::source::Presentation;
use thiserror::Error;

/// A medium the presentation shows that a host may decode.
#[derive(Clone, Debug)]
pub struct MediaNeed {
    /// The source item's, which `add_media` names.
    pub sha256: Sha256,
    /// The source's type (`image/heic`).
    pub mime: String,
    /// Where the source's bytes are: the decode plan's ranges are ranges of these.
    pub source: MediaHandle,
    /// The decode plan; its error says why the compiler plans none (the item stays omitted).
    pub plan: Result<HeifImage, HeifError>,
}

impl MediaNeed {
    /// The size a decoded file must have: the plan's output, once every transform applies.
    pub fn size(&self) -> Option<(u32, u32)> {
        self.plan.as_ref().ok().map(HeifImage::size)
    }
}

/// Why a host's file is not taken for a need.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum MediaRefusal {
    #[error("{0} is no medium the presentation needs decoded")]
    NotNeeded(String),
    #[error("media {0} has no decode plan: {1}")]
    NoPlan(String, String),
    #[error("the decoded file is {0}, not PNG or JPEG")]
    Type(String),
    #[error("the decoded file's size cannot be read")]
    Unsized,
    #[error(
        "the decoded JPEG is turned by its EXIF orientation {0}: it must be stored as it shows"
    )]
    Oriented(u16),
    #[error(
        "the decoded file is {width} × {height}, not the {expected_width} × {expected_height} its plan outputs"
    )]
    Size {
        width: u32,
        height: u32,
        expected_width: u32,
        expected_height: u32,
    },
    #[error("the decoded file has {bytes} bytes, more than the {limit} a medium may have")]
    TooLarge { bytes: u64, limit: u64 },
}

/// The bytes of a media source, as a range source.
fn with_source<T>(
    handle: &MediaHandle,
    input: &dyn RangeReader,
    read: impl FnOnce(&dyn RangeReader) -> T,
) -> Result<T, HeifError> {
    Ok(match &handle.source {
        MediaSource::InputRange { offset, len } => read(&SubReader::new(input, *offset, *len)?),
        MediaSource::Bytes(bytes) => read(&BytesReader::new(&bytes[..])),
        MediaSource::Reader(reader) => read(reader.as_ref()),
    })
}

/// The media of `presentation` a host may decode, each once, in presentation order: the items of a type in
/// [`HOST_DECODED`] that a container which did not fail shows, within the per-item budget (a larger one is omitted
/// as too large whatever a host does). `input` is the presentation's input, which `store`'s ranges are ranges of.
pub fn media_needs(
    presentation: &Presentation,
    store: &dyn MediaStore,
    input: &dyn RangeReader,
    policy: &MediaPolicy,
) -> Vec<MediaNeed> {
    let limits = HeifLimits::with_pixels(policy.host_decoded_pixels);
    let mut seen = Vec::new();
    let mut needs = Vec::new();
    for (_, shown) in container_media(presentation) {
        for sha256 in shown {
            if seen.contains(&sha256) {
                continue;
            }
            seen.push(sha256);
            let Some(entry) = presentation.media.get(&sha256) else {
                continue;
            };
            if !HOST_DECODED.contains(&entry.mime.as_str()) || entry.bytes > policy.item_bytes {
                continue;
            }
            let Some(source) = store.open(&sha256) else {
                continue;
            };
            let plan = with_source(&source, input, |reader| read_heif(reader, &limits))
                .and_then(|plan| plan);
            needs.push(MediaNeed {
                sha256,
                mime: entry.mime.as_str().to_owned(),
                source,
                plan,
            });
        }
    }
    needs
}

/// Checks a host's file for `need`: a PNG or JPEG (by its bytes) of the plan's output size, stored as it shows (a
/// JPEG's EXIF orientation absent or 1: a browser would turn another one again), within the per-item budget.
pub fn check_decoded(
    need: &MediaNeed,
    bytes: &[u8],
    policy: &MediaPolicy,
) -> Result<HostDecoded, MediaRefusal> {
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > policy.item_bytes {
        return Err(MediaRefusal::TooLarge {
            bytes: size,
            limit: policy.item_bytes,
        });
    }
    let (expected_width, expected_height) = match &need.plan {
        Ok(plan) => plan.size(),
        Err(error) => {
            return Err(MediaRefusal::NoPlan(
                need.sha256.to_hex(),
                error.to_string(),
            ));
        }
    };
    let mime = match sniff_mime(bytes.get(..64).unwrap_or(bytes)) {
        Some(mime @ (Mime::Png | Mime::Jpeg)) => mime,
        Some(other) => return Err(MediaRefusal::Type(other.as_str().to_owned())),
        None => return Err(MediaRefusal::Type("of no known type".to_owned())),
    };
    if mime == Mime::Jpeg
        && let Some(orientation) = jpeg_orientation(bytes).filter(|&value| value != 1)
    {
        return Err(MediaRefusal::Oriented(orientation));
    }
    let found = image_size(bytes, mime).ok_or(MediaRefusal::Unsized)?;
    if (found.width, found.height) != (expected_width, expected_height) {
        return Err(MediaRefusal::Size {
            width: found.width,
            height: found.height,
            expected_width,
            expected_height,
        });
    }
    Ok(HostDecoded {
        sha256: need.sha256,
        mime: mime.as_str(),
        width: found.width,
        height: found.height,
        bytes: size,
    })
}
