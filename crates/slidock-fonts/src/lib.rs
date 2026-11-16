//! Everything about fonts: sfnt identification (via `read-fonts`/`skrifa`), EOT/MTX and WOFF/WOFF2
//! containers, the font catalog and builtin metrics, CSS Fonts 4 face matching, the licence policy, subsetting
//! (`skera`), WOFF2 encoding (`ttf2woff2`) and the `@font-face` rules of a deck.
//!
//! The output side: [`ship_face`] takes one bound face and the code points the deck draws with it (from the
//! passes' collect-glyphs) to a subset WOFF2 with a true `@font-face` rule and its `LICENSES.txt` entry;
//! [`claim_rules`] keeps a family's rules from claiming one weight twice; [`font_rules`] orders them for
//! `<style id="sd-fonts">`; [`decide`] says which faces the licence policy lets ship.

pub mod advance;
pub mod bind;
pub mod catalog;
pub mod container;
pub mod face_css;
pub mod glyphs;
pub mod google;
pub mod limits;
pub mod names;
pub mod policy;
pub mod rename;
pub mod resolve;
pub mod sfnt;
pub mod ship;
pub mod subset;
pub mod woff2_out;

pub use advance::FaceFile;
pub use bind::{FaceDescriptor, WeightRange, css_match};
pub use catalog::builtin::builtin_catalog;
pub use catalog::{CatalogFace, CatalogFamily, FamilyKey, FamilyMatch, FontCatalog};
pub use container::{ContainerError, ContainerLimits, Format, Hint, Unwrapped};
pub use face_css::{FontDisplay, FontFaceRule, FontFormat, claim_rules, font_rules};
pub use glyphs::CodepointSet;
pub use limits::FontLimits;
pub use policy::{
    Decision, FontOrigin, FsType, LicenceEntry, LicencePolicy, Target, Toggle, decide,
};
pub use sfnt::{FaceRepair, SfntError, SfntFace, read_sfnt};
pub use ship::{FontNotice, ShipError, ShipInput, ShipOptions, ShippedFont, ship_face};
