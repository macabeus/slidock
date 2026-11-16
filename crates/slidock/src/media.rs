//! Materialising the media files of a document set (without the file system): each file is the source bytes copied as
//! they are, or transcoded: to PNG through the deterministic encoder, or to the file a host decoded the item to (a
//! HEIC photo, [`MediaStore::decoded`]), written as it is. An SVG is checked against the allow-list again as
//! it is copied, so a plan that names an unchecked SVG writes nothing, and written as a picture draws it: stretched to
//! its box (`slidock_io::picture_svg`). The host owns the output; this reads only the stores and the input it is given.

use std::io::Write;

use slidock_core_api::{MediaSource, MediaStore};
use slidock_html::{MediaAction, MediaPlanRef};
use slidock_io::{
    HASH_CHUNK, MAX_SVG_BYTES, Mime, RangeError, RangeReader, SvgRefusal, TranscodeError,
    check_range, picture_svg, sniff_mime,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("The bytes of media {0} are not available")]
    Unavailable(String),
    #[error("media {sha256}: {source}")]
    Read {
        sha256: String,
        #[source]
        source: RangeError,
    },
    #[error("media {sha256}: {source}")]
    Transcode {
        sha256: String,
        #[source]
        source: TranscodeError,
    },
    #[error("media {sha256}: {source}")]
    Svg {
        sha256: String,
        #[source]
        source: SvgRefusal,
    },
    #[error("media {sha256} cannot be transcoded to {mime}")]
    Unsupported { sha256: String, mime: String },
    #[error("media {sha256}: {source}")]
    Write {
        sha256: String,
        #[source]
        source: std::io::Error,
    },
}

const SVG: &str = "image/svg+xml";

/// The bytes of a media source, in chunks of at most [`HASH_CHUNK`].
fn read_chunks(
    source: &MediaSource,
    input: &dyn RangeReader,
    mut each: impl FnMut(&[u8]) -> Result<(), std::io::Error>,
) -> Result<Result<(), std::io::Error>, RangeError> {
    let (reader, offset, len): (&dyn RangeReader, u64, u64) = match source {
        MediaSource::Bytes(bytes) => return Ok(each(bytes)),
        MediaSource::InputRange { offset, len } => {
            check_range(input.len(), *offset, *len)?;
            (input, *offset, *len)
        }
        MediaSource::Reader(reader) => (reader.as_ref(), 0, reader.len()),
    };
    let mut at = 0;
    while at < len {
        // At most HASH_CHUNK, so it fits in usize.
        let chunk = (len - at).min(HASH_CHUNK as u64);
        let bytes = reader.read_at(offset + at, chunk as usize)?;
        if let Err(error) = each(&bytes) {
            return Ok(Err(error));
        }
        at += chunk;
    }
    Ok(Ok(()))
}

/// Writes the media file `plan` describes to `out`: the bytes of its source item from `store` (whose input ranges
/// are ranges of `input`), copied (an SVG as a picture draws it) or transcoded: the file a host decoded it to, as it
/// is (it must be of the plan's type), or PNG. Rasters the compiler transcodes over `max_pixels` are refused.
pub fn write_media_file(
    plan: &MediaPlanRef,
    store: &dyn MediaStore,
    input: &dyn RangeReader,
    max_pixels: u64,
    out: &mut dyn Write,
) -> Result<(), MediaError> {
    let sha256 = plan.sha256.to_hex();
    let handle = store
        .open(&plan.sha256)
        .ok_or_else(|| MediaError::Unavailable(sha256.clone()))?;
    let read = |error| MediaError::Read {
        sha256: sha256.clone(),
        source: error,
    };
    let write = |error| MediaError::Write {
        sha256: sha256.clone(),
        source: error,
    };
    match plan.action {
        MediaAction::Copy if plan.mime.as_str() == SVG => {
            let refused = |source| MediaError::Svg {
                sha256: sha256.clone(),
                source,
            };
            if handle.bytes > MAX_SVG_BYTES {
                return Err(refused(SvgRefusal::TooLarge));
            }
            let mut bytes = Vec::new();
            read_chunks(&handle.source, input, |chunk| {
                bytes.extend_from_slice(chunk);
                Ok(())
            })
            .map_err(read)?
            .map_err(write)?;
            let picture = picture_svg(&bytes).map_err(refused)?;
            out.write_all(&picture).map_err(write)
        }
        MediaAction::Copy => read_chunks(&handle.source, input, |chunk| out.write_all(chunk))
            .map_err(read)?
            .map_err(write),
        MediaAction::Transcode if store.decoded(&plan.sha256).is_some() => {
            let decoded = store
                .decoded(&plan.sha256)
                .ok_or_else(|| MediaError::Unavailable(sha256.clone()))?;
            let mut head = Vec::new();
            read_chunks(&decoded.source, input, |chunk| {
                if head.is_empty() {
                    head.extend_from_slice(chunk.get(..64).unwrap_or(chunk));
                }
                Ok(())
            })
            .map_err(read)?
            .map_err(write)?;
            if sniff_mime(&head).map(Mime::as_str) != Some(plan.mime.as_str()) {
                return Err(MediaError::Unsupported {
                    sha256,
                    mime: plan.mime.as_str().to_owned(),
                });
            }
            read_chunks(&decoded.source, input, |chunk| out.write_all(chunk))
                .map_err(read)?
                .map_err(write)
        }
        MediaAction::Transcode => {
            if plan.mime.as_str() != "image/png" {
                return Err(MediaError::Unsupported {
                    sha256,
                    mime: plan.mime.as_str().to_owned(),
                });
            }
            let mut bytes = Vec::new();
            read_chunks(&handle.source, input, |chunk| {
                bytes.extend_from_slice(chunk);
                Ok(())
            })
            .map_err(read)?
            .map_err(write)?;
            let png = slidock_io::transcode_to_png(&bytes, max_pixels).map_err(|error| {
                MediaError::Transcode {
                    sha256: sha256.clone(),
                    source: error,
                }
            })?;
            out.write_all(&png).map_err(write)
        }
    }
}
