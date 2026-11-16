//! Input handling below the frontends: the synchronous [`RangeReader`] (positioned reads natively,
//! `FileReaderSync` in a worker), magic-byte sniffing, a hardened zip reader inflating with `miniz_oxide`
//! (overlap, ratio and budget refusals), the `roxmltree` wrapper with depth/attribute/node limits, header-only
//! image sizing, the SVG allow-list check, the video probe, the movie reader and the WebM reader, the TIFF/BMP to PNG transcode, and the HEIF reader that
//! plans a host's decode of a HEIC photo.

mod bmp;
#[cfg(all(feature = "std-fs", any(unix, windows)))]
mod fs;
mod heif;
mod hevc;
mod image_size;
mod inflate;
mod movie;
mod png;
mod range;
mod sniff;
mod svg;
mod text;
mod tiff;
mod transcode;
mod video_probe;
mod webm;
pub mod xml;
pub mod zip;

pub use bmp::{BmpError, decode_bmp};
#[cfg(all(feature = "std-fs", any(unix, windows)))]
pub use fs::FileReader;
pub use heif::{
    Crop, Extent, HeifError, HeifImage, HeifLimits, HeifTile, HevcConfig, IccColour, MirrorAxis,
    Nclx, Transform, icc_colour, read_heif,
};
pub use hevc::{VideoSignal, sps_video_signal};
pub use image_size::{
    Dpi, ImageSize, check_svg_source, image_size, jpeg_orientation, read_image_size,
};
pub use inflate::{InflateError, inflate_raw_exact, unzlib_into};
pub use movie::{
    AudioEntry, Edit, MAX_MOOV_BYTES, Movie, MovieColour, MovieTrack, SampleEntry, VisualEntry,
    read_movie,
};
pub use png::{PngError, RgbaImage, encode_png};
pub use range::{
    BytesReader, CountingReader, HASH_CHUNK, RangeError, RangeReader, SubReader, check_range,
    read_head, sha256_range,
};
pub use sniff::{Mime, sniff_mime};
pub use svg::{MAX_SVG_BYTES, SvgFacts, SvgRefusal, check_svg, picture_svg};
pub use tiff::{MAX_TIFF_PAGES, MAX_TIFF_TAGS, TiffError, decode_tiff};
pub use transcode::{TranscodeError, transcode_to_png};
pub use video_probe::{VideoFacts, probe_video};
pub use webm::{MAX_BLOCKS, Webm, WebmTrack, read_webm};
pub use xml::{XmlBudget, XmlError, XmlErrorReason, XmlLimits, decode_xml, parse_xml};
pub use zip::{ZipArchive, ZipEntry, ZipError, ZipErrorReason, ZipLimits};

/// The `roxmltree` types [`parse_xml`] returns.
pub use roxmltree;
