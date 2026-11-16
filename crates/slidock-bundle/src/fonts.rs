//! Which faces the download ships, and the deck JSON's `fonts` that says so.
//!
//! A shipped face's file goes into `<style id="sd-fonts">` when the licence policy lets it ship in a download
//! ([`slidock_fonts::LicenceEntry::download`]) with the host's per-family toggle: a face from this
//! computer or the user ships unless its family is left out, and a restricted face ships only when its family is
//! included. A deck face drawn by a file that ships is listed with that file's source and licence; every other face
//! ships no file and is listed as `system` (or `emoji`, for an emoji family) with no licence.

use std::collections::BTreeSet;

use slidock_fonts::face_css::rule_order;
use slidock_fonts::{FamilyKey, FontFaceRule, FontOrigin, Toggle, font_rules};
use slidock_html::deck::{DeckFont, FontSource};

use crate::{ServedFace, ShippedFace};

/// The faces of a download.
pub struct FontPlan<'a> {
    /// The deck JSON's `fonts`, in the deck's order (slides index into it).
    pub faces: Vec<DeckFont>,
    /// The faces that ship, in the order of their first rule.
    pub shipped: Vec<&'a ShippedFace>,
    /// Their `@font-face` rules, in rule order.
    pub rules: Vec<String>,
    /// The families left out, by the host or the licence policy, of those with a face to ship, sorted.
    pub left_out: Vec<String>,
}

/// The deck JSON's source of a face from `origin`.
pub fn deck_source(origin: FontOrigin) -> FontSource {
    match origin {
        FontOrigin::Embedded => FontSource::Embedded,
        FontOrigin::Local => FontSource::Local,
        FontOrigin::Google => FontSource::Google,
        FontOrigin::User => FontSource::User,
        FontOrigin::Bundled => FontSource::Bundled,
    }
}

fn serves(file: &ShippedFace, face: &DeckFont) -> bool {
    file.serves.iter().any(|served: &ServedFace| {
        served.family == face.family && served.weight == face.weight && served.style == face.style
    })
}

/// The host's toggle for `file`'s family, named by the file or a deck face it draws: leaving out wins over including.
fn toggle(
    file: &ShippedFace,
    leave_out: &BTreeSet<FamilyKey>,
    include: &BTreeSet<FamilyKey>,
) -> Toggle {
    let named = |keys: &BTreeSet<FamilyKey>| {
        keys.contains(&FamilyKey::of(&file.licence.family))
            || file
                .serves
                .iter()
                .any(|served| keys.contains(&FamilyKey::of(&served.family)))
    };
    if named(leave_out) {
        Toggle::LeaveOut
    } else if named(include) {
        Toggle::Include
    } else {
        Toggle::Default
    }
}

pub fn plan<'a>(
    faces: &[DeckFont],
    shipped: &'a [ShippedFace],
    leave_out: &BTreeSet<String>,
    include: &BTreeSet<String>,
) -> FontPlan<'a> {
    let keys = |families: &BTreeSet<String>| -> BTreeSet<FamilyKey> {
        families
            .iter()
            .map(|family| FamilyKey::of(family))
            .collect()
    };
    let (leave_out, include) = (keys(leave_out), keys(include));
    let mut kept: Vec<&ShippedFace> = shipped
        .iter()
        .filter(|file| {
            file.licence
                .download(toggle(file, &leave_out, &include))
                .ships()
        })
        .collect();
    let first_rule =
        |file: &&ShippedFace| file.rules.iter().min_by(|a, b| rule_order(a, b)).cloned();
    kept.sort_by(|a, b| match (first_rule(a), first_rule(b)) {
        (Some(a), Some(b)) => rule_order(&a, &b),
        (a, b) => a.is_some().cmp(&b.is_some()),
    });
    let mut left_out = BTreeSet::new();
    let listed = faces
        .iter()
        .map(|face| match kept.iter().find(|file| serves(file, face)) {
            Some(file) => DeckFont {
                source: deck_source(file.licence.origin),
                licence: Some(file.licence.text()),
                ..face.clone()
            },
            None => {
                if shipped.iter().any(|file| serves(file, face)) {
                    left_out.insert(face.family.clone());
                }
                DeckFont {
                    source: match face.source {
                        FontSource::Emoji => FontSource::Emoji,
                        _ => FontSource::System,
                    },
                    licence: None,
                    ..face.clone()
                }
            }
        })
        .collect();
    let rules: Vec<FontFaceRule> = kept
        .iter()
        .flat_map(|file| file.rules.iter().cloned())
        .collect();
    FontPlan {
        faces: listed,
        rules: font_rules(&rules),
        shipped: kept,
        left_out: left_out.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use slidock_fonts::bind::{FaceDescriptor, WeightRange};
    use slidock_fonts::{FontDisplay, FontFormat, FsType, LicenceEntry};
    use slidock_ir::fonts::FaceStyle;

    use super::*;

    fn face(family: &str, weight: u32, source: FontSource) -> DeckFont {
        DeckFont {
            family: family.into(),
            weight,
            style: FaceStyle::Normal,
            source,
            licence: None,
        }
    }

    fn shipped(
        family: &str,
        weight: u32,
        origin: FontOrigin,
        serves: &[(&str, u32)],
    ) -> ShippedFace {
        let descriptor = FaceDescriptor {
            weight: WeightRange::point(weight),
            style: FaceStyle::Normal,
        };
        ShippedFace {
            rules: vec![FontFaceRule {
                family: family.into(),
                descriptor,
                unicode_range: None,
                format: FontFormat::Woff2,
                display: FontDisplay::Block,
                bytes: format!("wOF2{family}{weight}").into_bytes(),
            }],
            licence: LicenceEntry {
                family: family.into(),
                weight: weight.to_string(),
                style: "normal",
                origin,
                fs_type: FsType(0),
                copyright: Some(format!("{family} authors")),
                licence: None,
                licence_url: None,
                licence_id: Some("OFL-1.1".into()),
            },
            serves: serves
                .iter()
                .map(|(family, weight)| ServedFace {
                    family: (*family).into(),
                    weight: *weight,
                    style: FaceStyle::Normal,
                })
                .collect(),
        }
    }

    #[test]
    fn lists_each_face_as_the_file_that_draws_it_ships_and_the_rest_as_system() {
        let faces = [
            face("Arial", 400, FontSource::System),
            face("Inter", 400, FontSource::Google),
            face("Inter", 700, FontSource::Google),
            face("Roboto Medium", 400, FontSource::Embedded),
            face("Brand", 400, FontSource::Local),
            face("Mine", 400, FontSource::User),
        ];
        let files = [
            shipped("Roboto", 500, FontOrigin::Google, &[("Roboto Medium", 500)]),
            shipped("Inter", 400, FontOrigin::Google, &[("Inter", 400)]),
            shipped("Brand", 400, FontOrigin::Local, &[("Brand", 400)]),
            shipped("Mine", 400, FontOrigin::User, &[("Mine", 400)]),
            shipped("Rob", 400, FontOrigin::Embedded, &[("Roboto Medium", 400)]),
        ];
        let plan = plan(
            &faces,
            &files,
            &BTreeSet::from(["mine".to_owned()]),
            &BTreeSet::new(),
        );
        let listed: Vec<(&str, u32, FontSource)> = plan
            .faces
            .iter()
            .map(|f| (f.family.as_str(), f.weight, f.source))
            .collect();
        assert_eq!(
            listed,
            [
                ("Arial", 400, FontSource::System),
                ("Inter", 400, FontSource::Google),
                ("Inter", 700, FontSource::System),
                ("Roboto Medium", 400, FontSource::Embedded),
                ("Brand", 400, FontSource::Local),
                ("Mine", 400, FontSource::System),
            ]
        );
        assert_eq!(
            plan.faces[1].licence.as_deref(),
            Some("Inter authors\nLicence: OFL-1.1")
        );
        assert!(plan.faces[2].licence.is_none() && plan.faces[5].licence.is_none());
        // Every kept file ships, the Roboto file too (it draws no face the deck lists), in rule order.
        let shipped: Vec<&str> = plan
            .shipped
            .iter()
            .map(|f| f.licence.family.as_str())
            .collect();
        assert_eq!(shipped, ["Brand", "Inter", "Rob", "Roboto"]);
        assert_eq!(plan.rules.len(), 4);
        assert!(plan.rules[0].starts_with("@font-face{font-family:\"Brand\";"));
        // The toggle matches by key, and leaves out only a face from this computer or the user.
        assert_eq!(plan.left_out, ["Mine"]);
        let all = BTreeSet::from(["Inter".to_owned(), "Brand".to_owned()]);
        let plan = super::plan(&faces, &files, &all, &BTreeSet::new());
        assert_eq!(plan.faces[1].source, FontSource::Google);
        assert_eq!(plan.faces[4].source, FontSource::System);
        assert_eq!(plan.left_out, ["Brand"]);
    }

    #[test]
    fn leaves_nothing_out_of_a_deck_without_files() {
        let faces = [face("Arial", 400, FontSource::System)];
        let plan = plan(
            &faces,
            &[],
            &BTreeSet::from(["Arial".to_owned()]),
            &BTreeSet::new(),
        );
        assert!(plan.left_out.is_empty() && plan.rules.is_empty());
        assert_eq!(plan.faces, faces);
    }
}
