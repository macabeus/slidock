//! Which installed faces are worth reading: Local Font Access returns metadata for every installed face, and reading
//! one (`blob()`) costs a file read, so the faces are chosen from their names first. The CLI's system font scan
//! chooses from the same metadata. The bytes then go through `add_font`, where the file's own tables decide.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::needs::{FamilyNeeds, LocalQuery};
use crate::catalog::FamilyKey;
use crate::names::postscript_face;

/// At most this many faces are read per selection, and this many installed files per session.
pub const MAX_LOCAL_PICKS: usize = 48;
/// The most bytes of installed files a session reads.
pub const MAX_LOCAL_BYTES: u64 = 96 * 1024 * 1024;
/// The most bytes one installed file may have, unless it holds a PostScript name the presentation asks for (a large
/// CJK collection).
pub const MAX_LOCAL_FILE_BYTES: u64 = 48 * 1024 * 1024;

/// Why an installed file is not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum LocalRefusal {
    #[error("{MAX_LOCAL_PICKS} installed font files were read already")]
    TooManyFiles,
    #[error(
        "the file has {bytes} bytes, more than the {MAX_LOCAL_FILE_BYTES} an installed font may have"
    )]
    FileTooLarge { bytes: u64 },
    #[error(
        "the file has {bytes} bytes and only {left} of the {MAX_LOCAL_BYTES} for installed fonts are left"
    )]
    OverBudget { bytes: u64, left: u64 },
}

/// What a session has read of installed fonts: at most [`MAX_LOCAL_PICKS`] files and [`MAX_LOCAL_BYTES`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalBudget {
    files: usize,
    bytes: u64,
}

impl LocalBudget {
    /// Whether a file of `bytes` may be read; `exact_postscript`: it holds a PostScript name the presentation asks
    /// for, which lifts the per-file cap.
    pub fn check(&self, bytes: u64, exact_postscript: bool) -> Result<(), LocalRefusal> {
        if self.files >= MAX_LOCAL_PICKS {
            return Err(LocalRefusal::TooManyFiles);
        }
        if bytes > MAX_LOCAL_FILE_BYTES && !exact_postscript {
            return Err(LocalRefusal::FileTooLarge { bytes });
        }
        let left = MAX_LOCAL_BYTES.saturating_sub(self.bytes);
        if bytes > left {
            return Err(LocalRefusal::OverBudget { bytes, left });
        }
        Ok(())
    }

    /// Checks a file and counts it when it may be read.
    pub fn spend(&mut self, bytes: u64, exact_postscript: bool) -> Result<(), LocalRefusal> {
        self.check(bytes, exact_postscript)?;
        self.files += 1;
        self.bytes += bytes;
        Ok(())
    }
}

/// One installed face as Local Font Access describes it (`FontData`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalFontMeta {
    /// Name id 6, never localized.
    pub postscript_name: String,
    /// Name id 4.
    pub full_name: String,
    /// Name id 1, the legacy family ("Roboto Medium" for Roboto-Medium.ttf).
    pub family: String,
    /// Name id 2, localized to the viewer's language, so never parsed.
    pub style: String,
}

/// A face to read, by its index in the metadata list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPick {
    pub index: usize,
    pub postscript_name: String,
    /// The family of the needs it may fill.
    pub family: String,
    /// It is one of the PostScript names the presentation asked for.
    pub exact_postscript: bool,
}

/// The installed faces that may fill an open slot, at most [`MAX_LOCAL_PICKS`]: exact PostScript names first, then
/// faces of a needed family whose PostScript name spells an open slot's weight and slant. A family whose faces'
/// names spell no style at all is read whole (up to the cap).
pub fn select_local(metas: &[LocalFontMeta], needs: &[FamilyNeeds]) -> Vec<LocalPick> {
    let mut picks: Vec<LocalPick> = Vec::new();
    let push = |picks: &mut Vec<LocalPick>, index: usize, family: &str, exact: bool| {
        if picks.len() < MAX_LOCAL_PICKS
            && !picks.iter().any(|pick| pick.index == index)
            && let Some(meta) = metas.get(index)
        {
            picks.push(LocalPick {
                index,
                postscript_name: meta.postscript_name.clone(),
                family: family.to_owned(),
                exact_postscript: exact,
            });
        }
    };
    let queried: Vec<(&FamilyNeeds, &LocalQuery)> = needs
        .iter()
        .filter_map(|need| need.local.as_ref().map(|query| (need, query)))
        .collect();
    for (need, query) in &queried {
        for (index, meta) in metas.iter().enumerate() {
            if query.postscript_names.contains(&meta.postscript_name) {
                push(&mut picks, index, &need.family, true);
            }
        }
    }
    for (need, query) in &queried {
        let of_family: Vec<(usize, Option<u32>, bool)> = metas
            .iter()
            .enumerate()
            .filter_map(|(index, meta)| {
                let face = postscript_face(&meta.postscript_name);
                let named = query.families.contains(&FamilyKey::of(&meta.family))
                    || query.families.contains(&FamilyKey::of(&face.family));
                named.then_some((index, face.weight, face.italic))
            })
            .collect();
        let open: Vec<(u32, bool)> = need
            .open_slots()
            .map(|slot| {
                (
                    slot.weight,
                    slot.style == slidock_ir::fonts::FaceStyle::Italic,
                )
            })
            .collect();
        let hinted: Vec<usize> = of_family
            .iter()
            .filter(|(_, weight, italic)| open.contains(&(weight.unwrap_or(400), *italic)))
            .map(|(index, _, _)| *index)
            .collect();
        let unhinted = of_family
            .iter()
            .all(|(_, weight, italic)| weight.is_none() && !italic);
        let chosen: Vec<usize> = if hinted.is_empty() && unhinted {
            of_family.iter().map(|(index, _, _)| *index).collect()
        } else {
            hinted
        };
        for index in chosen {
            push(&mut picks, index, &need.family, false);
        }
    }
    picks
}
