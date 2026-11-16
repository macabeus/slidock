//! Google Fonts as a font source: the offline catalog of open-licence families, `css2` request URLs for them, and
//! the reader of `css2` responses. No I/O: the hosts fetch (the web app in TypeScript, the CLI with `ureq`).

pub mod catalog;
pub mod css2;
pub mod unicode_range;

pub use catalog::{
    Axis, GoogleCatalog, GoogleFamily, GoogleMatch, Licence, Weights, google_catalog,
    google_catalog_data,
};
pub use css2::{
    CSS2_ENDPOINT, Css2Error, MAX_CSS2_BYTES, RemoteFace, css2_faces, css2_url, parse_css2,
};
pub use unicode_range::UnicodeRange;
