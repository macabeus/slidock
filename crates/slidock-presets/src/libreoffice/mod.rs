//! LibreOffice's preset tables, as data: the ODP catalogs (`sd/xml/effects.xml`, `sd/xml/transitions.xml`),
//! PowerPoint's preset ids and sub-type numbers as LibreOffice's import names them
//! (`oox/source/ppt/commontimenodecontext.cxx`), and the transition preset each ODF 1.0 transition style migrates to
//! (`sd/source/core/EffectMigration.cxx`). `effects.rs`, `transitions.rs`, `pptx.rs` and `migration.rs` are generated
//! by `cargo xtask libreoffice-presets` from the pinned commit and keep the sources' MPL-2.0 licence; this module and
//! the mappings that read them are slidock's own.

pub mod effects;
pub mod migration;
pub mod pptx;
pub mod transitions;

/// The LibreOffice core commit the catalogs are generated from: tag `libreoffice-25.2.3.2`.
pub const COMMIT: &str = "bbb074479178df812d175f709636b368952c2ce3";

/// A build preset: its class, id and sub-type, as an ODP file names them (`presentation:preset-class`,
/// `presentation:preset-id`, `presentation:preset-sub-type`), and how it iterates over text (`anim:iterate-type`:
/// `by-letter`, `by-word`) when it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EffectPreset {
    pub class: &'static str,
    pub id: &'static str,
    pub subtype: Option<&'static str>,
    pub iterate: Option<&'static str>,
}

/// A transition preset and the SMIL filter it writes (`smil:type`, `smil:subtype`, `smil:direction="reverse"`,
/// `smil:fadeColor`, LibreOffice's `loext:subtype`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TransitionPreset {
    pub id: &'static str,
    pub smil_type: &'static str,
    pub subtype: Option<&'static str>,
    pub reverse: bool,
    pub fade_color: Option<&'static str>,
    pub lo_subtype: Option<&'static str>,
}

/// A PowerPoint preset: its class (`entrance`, `exit`, `emphasis`, `motion-path`), its `presetID`, and the `ooo-*`
/// preset LibreOffice reads it as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PptxPreset {
    pub class: &'static str,
    pub id: u32,
    pub name: &'static str,
}
