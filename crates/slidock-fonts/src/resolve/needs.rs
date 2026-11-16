//! What a host can still fetch: per family, the slots no source has exactly, the Google Fonts request for them (only
//! for catalog families) and the keys Local Font Access matches installed faces by.

use serde::Serialize;
use slidock_ir::fonts::FaceStyle;

use super::source::{SourceKind, SourceOrder};
use super::{Fill, ResolvedFamily, ResolvedSlot};
use crate::catalog::FamilyKey;
use crate::google::{GoogleCatalog, Licence, css2_faces, css2_url};
use crate::names::split_weight_name;

/// A Google Fonts request for the open slots of one family.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GooglePlan {
    /// As the catalog spells it.
    pub family: String,
    #[serde(serialize_with = "licence_spdx")]
    pub licence: Licence,
    /// The faces requested, `(weight, style)`, in `css2` order.
    pub faces: Vec<(u32, FaceStyle)>,
    pub url: String,
}

fn licence_spdx<S: serde::Serializer>(licence: &Licence, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(licence.spdx())
}

/// What Local Font Access (or a system font scan) matches installed faces against.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalQuery {
    /// Exact PostScript names (Keynote names faces by them).
    pub postscript_names: Vec<String>,
    /// The family keys an installed face's family (or the family its PostScript name spells) may have: the family's,
    /// and the family a trailing weight name splits off.
    pub families: Vec<FamilyKey>,
}

/// A family with slots a source could still fill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyNeeds {
    pub key: FamilyKey,
    pub family: String,
    /// Every slot, filled or not.
    pub slots: Vec<ResolvedSlot>,
    /// Set when Google Fonts is on and has the family, for the slots no earlier source has.
    pub google: Option<GooglePlan>,
    /// Set when installed fonts are on, for the slots no earlier source has.
    pub local: Option<LocalQuery>,
}

impl FamilyNeeds {
    /// The slots no source has exactly.
    pub fn open_slots(&self) -> impl Iterator<Item = &ResolvedSlot> {
        self.slots.iter().filter(|slot| !slot.fill.is_exact())
    }
}

/// Whether a slot is open for `source`: no source before it in the order has the face exactly.
fn open_for(slot: &ResolvedSlot, source: SourceKind, order: &SourceOrder) -> bool {
    match &slot.fill {
        Fill::Exact { source: filled, .. } => !order.before(*filled, source) && *filled != source,
        Fill::Demoted { .. } | Fill::Empty => true,
    }
}

/// The families with a slot no source has exactly, each with the Google request and the local query for the slots
/// a source that is on could still fill.
pub fn font_needs(
    resolved: &[ResolvedFamily],
    order: &SourceOrder,
    catalog: &GoogleCatalog,
) -> Vec<FamilyNeeds> {
    resolved
        .iter()
        .filter(|family| family.slots.iter().any(|slot| !slot.fill.is_exact()))
        .map(|family| {
            let slots_for = |source: SourceKind| -> Vec<(u32, FaceStyle)> {
                family
                    .slots
                    .iter()
                    .filter(|slot| open_for(slot, source, order))
                    .map(|slot| (slot.requested.weight, slot.style))
                    .collect()
            };
            let google = order
                .contains(SourceKind::Google)
                .then(|| slots_for(SourceKind::Google))
                .filter(|slots| !slots.is_empty())
                .and_then(|slots| google_plan(catalog, &family.family, &slots));
            let local = order
                .contains(SourceKind::ViewerInstalled)
                .then(|| slots_for(SourceKind::ViewerInstalled))
                .filter(|slots| !slots.is_empty())
                .map(|_| local_query(family));
            FamilyNeeds {
                key: family.key.clone(),
                family: family.family.clone(),
                slots: family.slots.clone(),
                google,
                local,
            }
        })
        .collect()
}

fn google_plan(
    catalog: &GoogleCatalog,
    family: &str,
    slots: &[(u32, FaceStyle)],
) -> Option<GooglePlan> {
    let found = catalog.find(family)?;
    let url = css2_url(catalog, family, slots)?;
    Some(GooglePlan {
        family: found.family.family.clone(),
        licence: found.family.licence,
        faces: css2_faces(catalog, family, slots),
        url,
    })
}

fn local_query(family: &ResolvedFamily) -> LocalQuery {
    let mut families = vec![family.key.clone()];
    if let Some(split) = split_weight_name(&family.family) {
        let base = FamilyKey::of(split.family);
        if !families.contains(&base) {
            families.push(base);
        }
    }
    LocalQuery {
        postscript_names: family.postscript_names.clone(),
        families,
    }
}
