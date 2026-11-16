//! Which faces a deck may ship, and the `LICENSES.txt` entry of each face it ships.
//!
//! Faces embedded in the presentation, served by Google Fonts' open catalog or bundled with slidock ship freely.
//! Faces from the viewer's computer or dropped by the user ship in the preview, and in the download unless the
//! policy or the per-family toggle leaves them out; the download lists them for a notice. `OS/2.fsType` overrides
//! the source: a restricted face (bit 1, without the preview/print or editable bits) ships in the preview only, and
//! out of the download is replaced by its substitute unless the user's toggle includes a face from their computer or
//! files; a bitmap-only face (bit 9) ships in neither.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::catalog::FamilyKey;
use crate::sfnt::FaceDetails;

/// Where a face's file came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontOrigin {
    /// Embedded in the presentation.
    Embedded,
    /// Installed on the computer that compiles the deck (Local Font Access, or the CLI's system fonts).
    Local,
    /// Google Fonts' open-source catalog.
    Google,
    /// A file the user dropped.
    User,
    /// One of slidock's substitute families.
    Bundled,
}

impl FontOrigin {
    /// The words `LICENSES.txt` names the source with.
    pub const fn describe(self) -> &'static str {
        match self {
            FontOrigin::Embedded => "embedded in the presentation",
            FontOrigin::Local => "installed on the computer that made the deck",
            FontOrigin::Google => "Google Fonts",
            FontOrigin::User => "added by the user",
            FontOrigin::Bundled => "a slidock substitute",
        }
    }
}

/// `OS/2.fsType`: the embedding permissions a font's vendor set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FsType(pub u16);

impl FsType {
    const RESTRICTED: u16 = 0x0002;
    const PREVIEW_AND_PRINT: u16 = 0x0004;
    const EDITABLE: u16 = 0x0008;
    const NO_SUBSETTING: u16 = 0x0100;
    const BITMAP_ONLY: u16 = 0x0200;

    /// Restricted license embedding: bit 1 set and neither of the more permissive bits 2 and 3 (old fonts may set
    /// several, and the most permissive wins).
    pub const fn restricted(self) -> bool {
        self.0 & Self::RESTRICTED != 0 && self.0 & (Self::PREVIEW_AND_PRINT | Self::EDITABLE) == 0
    }

    /// The face may not be subset: it ships whole.
    pub const fn no_subsetting(self) -> bool {
        self.0 & Self::NO_SUBSETTING != 0
    }

    /// Only the face's bitmaps may be embedded: its outlines never ship.
    pub const fn bitmap_only(self) -> bool {
        self.0 & Self::BITMAP_ONLY != 0
    }
}

/// The download's choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LicencePolicy {
    pub include_viewer_fonts_in_download: bool,
    pub include_user_fonts_in_download: bool,
    pub respect_restricted_fstype: bool,
}

impl Default for LicencePolicy {
    fn default() -> Self {
        LicencePolicy {
            include_viewer_fonts_in_download: true,
            include_user_fonts_in_download: true,
            respect_restricted_fstype: true,
        }
    }
}

/// What a face is shipped in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// The deck shown in the web app, on the computer that holds the fonts.
    Preview,
    /// The zip or single file the user takes away.
    Download,
}

/// Why a face does not ship.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Exclusion {
    /// `fsType` restricted license embedding.
    Restricted,
    /// `fsType` bitmap embedding only.
    BitmapOnly,
    /// The policy or the user's per-family toggle leaves the viewer's or user's face out.
    LeftOut,
}

impl Exclusion {
    /// The diagnostic code the compile reports for the face (font-licence-excluded).
    pub const fn code(self) -> &'static str {
        "MID5413"
    }
}

/// The user's per-family download toggle, which only viewer-installed and dropped faces offer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Toggle {
    /// Untouched: the policy and `fsType` decide.
    #[default]
    Default,
    /// Left out of the download.
    LeaveOut,
    /// In the download, a restricted face too: the user's call, with the notice.
    Include,
}

/// Whether a face ships in a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decision {
    Ship,
    /// Ships, and the download lists it in the notice: the face came from the viewer's computer or the user, who
    /// must make sure they may share it.
    ShipWithNotice,
    /// Does not ship; its substitute draws instead.
    Exclude(Exclusion),
}

impl Decision {
    pub const fn ships(self) -> bool {
        !matches!(self, Decision::Exclude(_))
    }
}

/// Whether a face from `origin` with `fs_type` ships in `target`. `toggle` is the user's per-family download toggle,
/// which only viewer-installed and dropped faces offer: it leaves one out, or includes it whatever its source's
/// policy or its restricted `fsType` says. A bitmap-only face never ships.
pub fn decide(
    origin: FontOrigin,
    fs_type: FsType,
    policy: &LicencePolicy,
    target: Target,
    toggle: Toggle,
) -> Decision {
    if fs_type.bitmap_only() {
        return Decision::Exclude(Exclusion::BitmapOnly);
    }
    if target == Target::Preview {
        return Decision::Ship;
    }
    let restricted = policy.respect_restricted_fstype && fs_type.restricted();
    let included = match origin {
        FontOrigin::Embedded | FontOrigin::Google | FontOrigin::Bundled => {
            return if restricted {
                Decision::Exclude(Exclusion::Restricted)
            } else {
                Decision::Ship
            };
        }
        FontOrigin::Local => policy.include_viewer_fonts_in_download,
        FontOrigin::User => policy.include_user_fonts_in_download,
    };
    match toggle {
        Toggle::LeaveOut => Decision::Exclude(Exclusion::LeftOut),
        Toggle::Include => Decision::ShipWithNotice,
        Toggle::Default if restricted => Decision::Exclude(Exclusion::Restricted),
        Toggle::Default if included => Decision::ShipWithNotice,
        Toggle::Default => Decision::Exclude(Exclusion::LeftOut),
    }
}

/// The families of the faces a download lists in its notice, once each, sorted by key: the families whose faces
/// ship with [`Decision::ShipWithNotice`].
pub fn notice_families<'a>(shipped: impl IntoIterator<Item = (&'a str, Decision)>) -> Vec<&'a str> {
    let mut seen = BTreeSet::new();
    let mut families: Vec<(FamilyKey, &str)> = Vec::new();
    for (family, decision) in shipped {
        let key = FamilyKey::of(family);
        if decision == Decision::ShipWithNotice && seen.insert(key.clone()) {
            families.push((key, family));
        }
    }
    families.sort_by(|a, b| a.0.cmp(&b.0));
    families.into_iter().map(|(_, family)| family).collect()
}

/// The SPDX id of the licence a name-table licence description (id 13) or URL (id 14) names, for the licences
/// fonts are commonly released under.
pub fn licence_id(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    let has = |needle: &str| lower.contains(needle);
    if has("open font license") || has("openfontlicense.org") || has("scripts.sil.org/ofl") {
        Some("OFL-1.1")
    } else if has("apache license") || has("apache.org/licenses/license-2.0") {
        Some("Apache-2.0")
    } else if has("ubuntu font licence") || has("ubuntu font license") {
        Some("UFL-1.0")
    } else if has("bitstream vera") {
        Some("Bitstream-Vera")
    } else {
        None
    }
}

/// The SPDX id of a face's licence: the one its source states (Google's catalog), else the one its name table's
/// licence description or URL names.
pub fn stated_licence(details: &FaceDetails, source_licence: Option<&str>) -> Option<String> {
    source_licence
        .or_else(|| details.licence.as_deref().and_then(licence_id))
        .or_else(|| details.licence_url.as_deref().and_then(licence_id))
        .map(str::to_owned)
}

/// One shipped face's entry in `LICENSES.txt`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LicenceEntry {
    pub family: String,
    /// `font-weight` as the rule prints it (`400`, `200 700`).
    pub weight: String,
    pub style: &'static str,
    pub origin: FontOrigin,
    /// The file's embedding permissions.
    pub fs_type: FsType,
    /// Name id 0.
    pub copyright: Option<String>,
    /// Name id 13.
    pub licence: Option<String>,
    /// Name id 14.
    pub licence_url: Option<String>,
    /// The SPDX id the source states (Google's catalog), else the one the name table's licence fields name.
    pub licence_id: Option<String>,
}

impl LicenceEntry {
    /// The entry of a face, from its name table and the licence its source states.
    pub fn new(
        family: &str,
        descriptor: crate::bind::FaceDescriptor,
        origin: FontOrigin,
        details: &FaceDetails,
        source_licence: Option<&str>,
    ) -> LicenceEntry {
        LicenceEntry {
            family: family.to_owned(),
            weight: crate::face_css::weight_value(descriptor.weight),
            style: crate::face_css::style_name(descriptor.style),
            origin,
            fs_type: FsType(details.fs_type),
            copyright: details.copyright.clone(),
            licence: details.licence.clone(),
            licence_url: details.licence_url.clone(),
            licence_id: stated_licence(details, source_licence),
        }
    }

    /// Whether the face ships in the download, with the user's `toggle` for its family and the default policy.
    pub fn download(&self, toggle: Toggle) -> Decision {
        decide(
            self.origin,
            self.fs_type,
            &LicencePolicy::default(),
            Target::Download,
            toggle,
        )
    }

    /// The copyright and licence lines: what the deck JSON's `licence` holds for the face.
    pub fn text(&self) -> String {
        let mut text = String::new();
        let mut line = |value: &str| {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(value);
        };
        if let Some(copyright) = &self.copyright {
            line(copyright);
        }
        if let Some(id) = &self.licence_id {
            line(&format!("Licence: {id}"));
        }
        if let Some(licence) = &self.licence {
            line(licence);
        }
        if let Some(url) = &self.licence_url {
            line(url);
        }
        if text.is_empty() {
            text.push_str("No copyright or licence string.");
        }
        text
    }
}

/// The full texts of the licences fonts are commonly released under, by SPDX id (`licenses/`, copies of the
/// repository's `LICENSES/`).
pub const LICENCE_TEXTS: [(&str, &str); 4] = [
    ("Apache-2.0", include_str!("../licenses/Apache-2.0.txt")),
    (
        "Bitstream-Vera",
        include_str!("../licenses/Bitstream-Vera.txt"),
    ),
    ("OFL-1.1", include_str!("../licenses/OFL-1.1.txt")),
    ("UFL-1.0", include_str!("../licenses/UFL-1.0.txt")),
];

/// The fonts part of `LICENSES.txt`: each entry's family, weight, style and source, then its copyright and
/// licence, in the order given; then the full text of each licence the entries name ([`LICENCE_TEXTS`]), once, in
/// SPDX id order.
pub fn licences_text(entries: &[LicenceEntry]) -> String {
    if entries.is_empty() {
        return "No font files ship with this presentation.\n".to_owned();
    }
    let mut out = String::new();
    for entry in entries {
        let _ = write!(
            out,
            "{} {} {} ({})\n{}\n\n",
            entry.family,
            entry.weight,
            entry.style,
            entry.origin.describe(),
            entry.text()
        );
    }
    let named: BTreeSet<&str> = entries
        .iter()
        .filter_map(|entry| entry.licence_id.as_deref())
        .collect();
    for (id, text) in LICENCE_TEXTS {
        if named.contains(id) {
            let _ = write!(out, "\nThe {id} licence\n\n{}\n\n", text.trim_end());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_common_licences() {
        assert_eq!(
            licence_id(
                "This Font Software is licensed under the SIL Open Font License, Version 1.1."
            ),
            Some("OFL-1.1")
        );
        assert_eq!(licence_id("https://openfontlicense.org"), Some("OFL-1.1"));
        assert_eq!(
            licence_id("Licensed under the Apache License, Version 2.0"),
            Some("Apache-2.0")
        );
        assert_eq!(licence_id("Ubuntu Font Licence 1.0"), Some("UFL-1.0"));
        assert_eq!(licence_id("All rights reserved."), None);
    }
}
