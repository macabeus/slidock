//! Per-face font resolution: each face slot a presentation draws (family, weight, style) is filled
//! by the first source, in the user's order, that has that face exactly. A family can mix sources (embedded regular
//! and bold, a Google italic); later sources only fill slots that are still empty. Embedded faces that fail their
//! identity check ([`identity`]) fill their own slot only when no source has the face. The bundled substitutes
//! under every source are not resolved here: what no source fills binds to them.

pub mod identity;
pub mod local;
pub mod needs;
pub mod source;
pub mod traits;

use std::collections::BTreeSet;

use serde::Serialize;
use slidock_ir::fonts::{FaceRequest, FaceStyle};

use crate::bind::WeightRange;
use crate::catalog::FamilyKey;
use crate::names::split_weight_name;

pub use identity::{Declared, FileIdentity, Identity, Mismatch, check_identity};
pub use local::{
    LocalBudget, LocalFontMeta, LocalPick, LocalRefusal, MAX_LOCAL_BYTES, MAX_LOCAL_FILE_BYTES,
    MAX_LOCAL_PICKS, select_local,
};
pub use needs::{FamilyNeeds, GooglePlan, LocalQuery, font_needs};
pub use source::{SourceKind, SourceOrder};
pub use traits::{FaceTraits, face_traits, local_meta_of_name_table, missing_code_points};

/// A family the presentation draws, with the faces it draws as its text asks for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontUse {
    pub family: String,
    pub faces: Vec<FaceRequest>,
    pub postscript_names: Vec<String>,
}

/// A face one source holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub source: SourceKind,
    /// Name id 16, else 1.
    pub family: String,
    /// Name id 1.
    pub legacy_family: Option<String>,
    pub postscript_name: String,
    /// One weight, or the range of a variable face.
    pub weight: WeightRange,
    pub style: FaceStyle,
    /// For an embedded face: each face the presentation declares the file holds, and its identity check.
    pub declared: Vec<(Declared, Identity)>,
}

impl Candidate {
    /// An embedded face that failed every declared slot's identity check.
    pub fn is_demoted(&self) -> bool {
        self.source == SourceKind::Embedded
            && !self.declared.is_empty()
            && self
                .declared
                .iter()
                .all(|(_, identity)| !identity.is_accepted())
    }

    fn file(&self) -> FileIdentity<'_> {
        FileIdentity {
            family: &self.family,
            legacy_family: self.legacy_family.as_deref(),
            weight: self.weight,
            italic: self.style == FaceStyle::Italic,
        }
    }
}

/// How a slot is filled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Fill {
    /// A candidate that has the face exactly, from the first source in the order that has one (of one source's, the
    /// last added); an embedded face that failed its identity check comes after every source.
    Exact {
        candidate: usize,
        source: SourceKind,
    },
    /// No source has the face; an embedded file declared for the slot stands in. `mismatch` is `None` when the file
    /// passed its identity check but has another weight (a Black file in a bold slot).
    Demoted {
        candidate: usize,
        mismatch: Option<Mismatch>,
    },
    Empty,
}

impl Fill {
    pub fn is_exact(&self) -> bool {
        matches!(self, Fill::Exact { .. })
    }
}

/// One face slot of a family.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSlot {
    /// As the text asks for it.
    pub requested: FaceRequest,
    /// The weight drawn: the weight a family name spells (`Roboto Medium` → 500), raised to bold for a bold run.
    pub weight: u32,
    pub style: FaceStyle,
    pub fill: Fill,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedFamily {
    pub key: FamilyKey,
    pub family: String,
    /// The weight a trailing weight name spells, when the family itself is nowhere but the split family is.
    pub name_weight: Option<u32>,
    pub postscript_names: Vec<String>,
    /// By weight, then style (normal first).
    pub slots: Vec<ResolvedSlot>,
}

/// The weight a face of a weight-named family draws: the name's weight, raised to bold for a bold slot.
pub fn drawn_weight(named: Option<u32>, weight: u32) -> u32 {
    match named {
        None => weight,
        Some(named) if weight >= 700 => named.max(700),
        Some(named) => named,
    }
}

/// How a candidate names a requested family: its weight for this slot, or `None` when it is another family.
fn candidate_weight(
    candidate: &Candidate,
    request: &FamilyKey,
    family: &str,
    weight: u32,
) -> Option<u32> {
    if FamilyKey::of(&candidate.family) == *request {
        return Some(weight);
    }
    let split = split_weight_name(family);
    let named = split.map(|split| split.weight);
    if candidate
        .legacy_family
        .as_deref()
        .is_some_and(|legacy| FamilyKey::of(legacy) == *request)
    {
        return Some(drawn_weight(named, weight));
    }
    let split = split?;
    let base = FamilyKey::of(split.family);
    let by_base = FamilyKey::of(&candidate.family) == base
        || candidate
            .legacy_family
            .as_deref()
            .is_some_and(|legacy| FamilyKey::of(legacy) == base);
    by_base.then(|| drawn_weight(named, weight))
}

/// Whether a candidate is the requested family at all (at any weight).
pub fn names_family(candidate: &Candidate, request: &FamilyKey, family: &str) -> bool {
    candidate_weight(candidate, request, family, 400).is_some()
}

fn style_rank(style: FaceStyle) -> u8 {
    match style {
        FaceStyle::Normal => 0,
        FaceStyle::Italic => 1,
    }
}

/// Resolves every slot of every family against the candidates of the sources that are on.
pub fn resolve(
    uses: &[FontUse],
    candidates: &[Candidate],
    order: &SourceOrder,
) -> Vec<ResolvedFamily> {
    uses.iter()
        .map(|font| resolve_family(font, candidates, order))
        .collect()
}

fn resolve_family(font: &FontUse, candidates: &[Candidate], order: &SourceOrder) -> ResolvedFamily {
    let key = FamilyKey::of(&font.family);
    let on: Vec<(usize, &Candidate)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| order.contains(candidate.source))
        .collect();
    let direct = on.iter().any(|(_, c)| {
        FamilyKey::of(&c.family) == key
            || c.legacy_family
                .as_deref()
                .is_some_and(|l| FamilyKey::of(l) == key)
    });
    let name_weight = if direct {
        None
    } else {
        split_weight_name(&font.family).map(|split| split.weight)
    };
    let mut requested: Vec<FaceRequest> = Vec::new();
    for face in &font.faces {
        if !requested.contains(face) {
            requested.push(face.clone());
        }
    }
    requested.sort_by_key(|face| (face.weight, style_rank(face.style)));
    let slots = requested
        .into_iter()
        .map(|face| {
            let exact = on
                .iter()
                .filter(|(_, c)| c.style == face.style)
                .filter(|(_, c)| {
                    candidate_weight(c, &key, &font.family, face.weight)
                        .is_some_and(|w| (c.weight.min..=c.weight.max).contains(&w))
                })
                .min_by_key(|(index, c)| {
                    (
                        c.is_demoted(),
                        order.rank(c.source),
                        std::cmp::Reverse(*index),
                    )
                });
            let fill = match exact {
                Some((index, c)) => Fill::Exact {
                    candidate: *index,
                    source: c.source,
                },
                None => demoted_fill(&on, &key, &face),
            };
            ResolvedSlot {
                weight: drawn_weight(name_weight, face.weight),
                style: face.style,
                requested: face,
                fill,
            }
        })
        .collect();
    ResolvedFamily {
        key,
        family: font.family.clone(),
        name_weight,
        postscript_names: font.postscript_names.clone(),
        slots,
    }
}

/// The embedded file the presentation declares for this slot, when no source has the face.
fn demoted_fill(on: &[(usize, &Candidate)], key: &FamilyKey, face: &FaceRequest) -> Fill {
    on.iter()
        .filter(|(_, c)| c.source == SourceKind::Embedded)
        .find_map(|(index, c)| {
            let (_, identity) = c.declared.iter().find(|(declared, _)| {
                FamilyKey::of(&declared.family) == *key
                    && declared.weight == face.weight
                    && declared.style == face.style
            })?;
            Some(Fill::Demoted {
                candidate: *index,
                mismatch: match identity {
                    Identity::Accepted => None,
                    Identity::Demoted(mismatch) => Some(mismatch.clone()),
                },
            })
        })
        .unwrap_or(Fill::Empty)
}

/// A face the binding catalog takes, under the family it is listed by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    pub candidate: usize,
    /// The file's family, or the declared family of a demoted face whose family no source has.
    pub family: String,
}

/// The candidates of the sources that are on, in the order a catalog must take them so that, of faces with the same
/// family, weight and style, the face of the earliest source wins (a catalog keeps the last of equal faces), and of one
/// source's, the last added. Demoted
/// faces come first, under their own family. Last come the demoted faces that stand in for a family no source has,
/// under that family's name, so the family binds to them before any substitute.
pub fn catalog_entries(
    resolved: &[ResolvedFamily],
    candidates: &[Candidate],
    order: &SourceOrder,
) -> Vec<CatalogEntry> {
    let mut indexed: Vec<(usize, &Candidate)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| order.contains(c.source))
        .collect();
    indexed.sort_by_key(|(index, c)| {
        let rank = order.rank(c.source).unwrap_or(usize::MAX);
        (!c.is_demoted(), std::cmp::Reverse(rank), *index)
    });
    let mut entries: Vec<CatalogEntry> = indexed
        .iter()
        .map(|(index, c)| CatalogEntry {
            candidate: *index,
            family: c.family.clone(),
        })
        .collect();
    let mut aliased = BTreeSet::new();
    for family in resolved {
        let nameless = !indexed
            .iter()
            .any(|(_, c)| names_family(c, &family.key, &family.family));
        if !nameless {
            continue;
        }
        for slot in &family.slots {
            if let Fill::Demoted { candidate, .. } = slot.fill
                && aliased.insert((family.key.clone(), candidate))
            {
                entries.push(CatalogEntry {
                    candidate,
                    family: family.family.clone(),
                });
            }
        }
    }
    entries
}

/// Checks a candidate's declared faces: the embedded file's identity against each face the presentation declares.
pub fn declare(candidate: &mut Candidate, declared: impl IntoIterator<Item = Declared>) {
    candidate.declared = declared
        .into_iter()
        .map(|declared| {
            let identity = check_identity(&declared, candidate.file());
            (declared, identity)
        })
        .collect();
}

/// Demotes the declared faces a subset file cannot draw: `missing[i]` is how many code points the presentation draws
/// with declared face `i` the file has no glyph for. A face that failed its identity check keeps its mismatch.
pub fn require_coverage(candidate: &mut Candidate, missing: &[u32]) {
    for ((_, identity), &missing) in candidate.declared.iter_mut().zip(missing) {
        if missing > 0 && identity.is_accepted() {
            *identity = Identity::Demoted(Mismatch::Subset { missing });
        }
    }
}

#[cfg(test)]
mod tests;
