//! bind-fonts: every family the presentation's text and list labels use is requested in `presentation.fonts` (merged
//! with what the frontend recorded) and bound with metrics: the catalog's family, directly or through a weight in its
//! name (`Roboto Medium` → Roboto at 500) or a PostScript name the frontend recorded, else a metric-compatible
//! substitute (MID5401), else the family of the generic its name suggests (MID5402). Substitutions are reported per
//! container that uses the family; one for a family the host holds a file of whose every face was refused says why
//! (MID5404: browsers would refuse the file, or its licence keeps it out).
//!
//! An emoji family (`slidock_fonts::catalog::emoji`) binds first, whatever the catalog holds, to the platform emoji
//! face and the `emoji` generic: the viewer's emoji font draws it and no file ships, which is reported once per family
//! (MID5406, info: the platform the deck came from draws the same pictures).
//!
//! A family's face is the one CSS font matching picks for the weight and style (`slidock_fonts::css_match`), so the
//! face the compiler measures is the face the browser draws. Its metrics are those of its file, and for a
//! variable face those of its instance at the weight drawn when the catalog read them; otherwise the default
//! instance stands in (MID5405). Without font files the catalog is the built-in metrics table.

use indexmap::IndexMap;
use slidock_canon::utf16;
use slidock_core_api::visit::walk_elements;
use slidock_core_api::{PassCtx, PassError, PassInfo, PresentationPass};
use slidock_fonts::catalog::emoji::{is_emoji_family, platform_emoji};
use slidock_fonts::catalog::substitutes::{generic_family, substitute};
use slidock_fonts::names::{generic_of, split_weight_name};
use slidock_fonts::{CatalogFace, CatalogFamily, FamilyKey, FontCatalog};
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::fonts::{
    BindingKind, BoundFace, FaceRequest, FaceStyle, FontBinding, FontEntry, FontFile, FontRequest,
    GenericFamily, font_key,
};
use slidock_ir::ids::NodeId;
use slidock_ir::source::{Container, Presentation};
use slidock_ir::text::{Bullet, FontRef, Inline};
use slidock_ir::verify::{Invariant, element_texts, family_font};

pub struct BindFonts;

static INFO: PassInfo = PassInfo::new(
    "bind-fonts",
    1,
    &[Invariant::ListsLabelled],
    &[Invariant::FontsBound],
);

/// A family a container's text draws with, and the face it draws.
struct Use {
    family: String,
    face: FaceRequest,
}

fn face(weight: u32, italic: bool) -> FaceRequest {
    FaceRequest {
        weight,
        style: if italic {
            FaceStyle::Italic
        } else {
            FaceStyle::Normal
        },
    }
}

/// The families a container's text renders with: runs, fields, styled breaks, paragraph marks and list labels.
fn uses_of(container: Container<'_>) -> Result<Vec<Use>, PassError> {
    let mut uses = Vec::new();
    let mut add = |font: Option<&FontRef>, face: FaceRequest, node: &NodeId| {
        if let Some(font) = font {
            let family = family_font(font, Some(node))?.family.clone();
            uses.push(Use { family, face });
        }
        Ok::<(), PassError>(())
    };
    for element in walk_elements(container.elements()) {
        let node = element.id();
        for (text, _) in element_texts(element) {
            for paragraph in &text.paragraphs {
                for inline in &paragraph.inlines {
                    let props = match inline {
                        Inline::Tab {} => None,
                        other => other.props(),
                    };
                    if let Some(props) = props {
                        let drawn =
                            face(props.weight.unwrap_or(400), props.italic.unwrap_or(false));
                        add(props.font.as_ref(), drawn, node)?;
                    }
                }
                if let Some(end) = &paragraph.end_props {
                    let drawn = face(end.weight.unwrap_or(400), end.italic.unwrap_or(false));
                    add(end.font.as_ref(), drawn, node)?;
                }
                if let Some(Bullet::Label { font, .. }) = &paragraph.props.bullet {
                    add(Some(&FontRef::Family(font.clone())), face(400, false), node)?;
                }
            }
        }
    }
    Ok(uses)
}

fn style_name(style: FaceStyle) -> &'static str {
    match style {
        FaceStyle::Normal => "normal",
        FaceStyle::Italic => "italic",
    }
}

/// Faces once each, by weight and then style name.
fn merge_faces(faces: impl IntoIterator<Item = FaceRequest>) -> Vec<FaceRequest> {
    let mut out: Vec<FaceRequest> = Vec::new();
    for face in faces {
        if !out.contains(&face) {
            out.push(face);
        }
    }
    out.sort_by(|a, b| {
        a.weight
            .cmp(&b.weight)
            .then_with(|| style_name(a.style).cmp(style_name(b.style)))
    });
    out
}

/// The face CSS draws for a weight and style.
fn matched(
    family: &CatalogFamily,
    weight: u32,
    style: FaceStyle,
) -> Result<&CatalogFace, PassError> {
    family.css_match(weight, style).ok_or_else(|| {
        PassError::failed(format!("The font catalog's {} has no faces", family.family))
    })
}

/// A binding with the catalog face it measures and the family it binds to.
#[derive(Clone, Debug, PartialEq)]
pub struct Bound<'a> {
    pub binding: FontBinding,
    pub face: &'a CatalogFace,
    pub family: &'a CatalogFamily,
}

fn bind_to(
    kind: BindingKind,
    family: &CatalogFamily,
    weight: Option<u32>,
) -> Result<Bound<'_>, PassError> {
    let at = weight.unwrap_or(400);
    let face = matched(family, at, FaceStyle::Normal)?;
    let binding = FontBinding {
        kind,
        family: family.family.clone(),
        weight,
        generic: family.generic,
        metrics: face.metrics_at(at).unwrap_or(&face.metrics).clone(),
        faces: None,
        file: face.sha256.map(|sha256| FontFile {
            sha256,
            postscript_name: face.postscript_name.clone(),
        }),
    };
    Ok(Bound {
        binding,
        face,
        family,
    })
}

/// The faces the presentation draws whose metrics differ from the binding's own face, by the weight drawn.
fn drawn_faces(bound: &Bound<'_>, requested: &[FaceRequest]) -> Result<Vec<BoundFace>, PassError> {
    let mut faces: Vec<BoundFace> = Vec::new();
    for request in requested {
        let drawn = bound.binding.drawn_weight(request.weight);
        let face = matched(bound.family, drawn, request.style)?;
        let metrics = face.metrics_at(drawn).unwrap_or(&face.metrics);
        if *metrics == bound.binding.metrics {
            continue;
        }
        let entry = BoundFace {
            weight: drawn,
            style: request.style,
            metrics: metrics.clone(),
        };
        match faces
            .iter_mut()
            .find(|f| f.weight == drawn && f.style == request.style)
        {
            Some(slot) => *slot = entry,
            None => faces.push(entry),
        }
    }
    faces.sort_by(|a, b| {
        a.weight
            .cmp(&b.weight)
            .then_with(|| style_name(a.style).cmp(style_name(b.style)))
    });
    Ok(faces)
}

/// The binding for a requested family (and the PostScript names it was requested by), with the face it
/// measures. Fails only when the catalog lacks the family of the request's generic.
pub fn bind_family<'a, S: AsRef<str>>(
    catalog: &'a FontCatalog,
    requested: &str,
    postscript_names: &[S],
) -> Result<Bound<'a>, PassError> {
    if is_emoji_family(&FamilyKey::of(requested)) {
        return bind_to(BindingKind::Generic, platform_emoji(), None);
    }
    if let Some(installed) = catalog.find_family(requested) {
        return bind_to(BindingKind::Installed, installed.family, installed.weight);
    }
    if let Some(named) = catalog.find_postscript(postscript_names) {
        return bind_to(BindingKind::Installed, named, None);
    }
    let split = split_weight_name(requested);
    let direct = substitute(&FamilyKey::of(requested));
    let via_weight = match (direct, split) {
        (None, Some(split)) => substitute(&FamilyKey::of(split.family)),
        _ => None,
    };
    let target = direct.or(via_weight).unwrap_or("");
    if let Some(family) = catalog.get(&FamilyKey::of(target)) {
        let weight = via_weight.and(split).map(|split| split.weight);
        return bind_to(BindingKind::Substitute, family, weight);
    }
    let generic = generic_of(requested);
    let name = generic_family(generic);
    let fallback = catalog.get(&FamilyKey::of(name)).ok_or_else(|| {
        PassError::failed(format!(
            "The font catalog has no {name} for the {} generic",
            generic_name(generic)
        ))
    })?;
    let mut bound = bind_to(BindingKind::Generic, fallback, None)?;
    bound.binding.generic = generic;
    Ok(bound)
}

fn generic_name(generic: slidock_ir::fonts::GenericFamily) -> &'static str {
    generic.as_str()
}

impl PresentationPass for BindFonts {
    fn info(&self) -> &PassInfo {
        &INFO
    }

    fn run(
        &self,
        presentation: &Presentation,
        ctx: &mut PassCtx<'_>,
    ) -> Result<Presentation, PassError> {
        // Per font key: the family first seen, the faces drawn, and the containers drawing it.
        let mut requests: IndexMap<String, (String, Vec<FaceRequest>)> = IndexMap::new();
        let mut used_by: IndexMap<String, Vec<NodeId>> = IndexMap::new();
        for container in presentation.containers().filter(|c| c.failed().is_none()) {
            for Use { family, face } in uses_of(container)? {
                let key = font_key(&family);
                requests
                    .entry(key.clone())
                    .or_insert_with(|| (family, Vec::new()))
                    .1
                    .push(face);
                let users = used_by.entry(key).or_default();
                if !users.contains(container.id()) {
                    users.push(container.id().clone());
                }
            }
        }

        let mut keys: Vec<&String> = presentation.fonts.keys().collect();
        keys.extend(
            requests
                .keys()
                .filter(|key| !presentation.fonts.contains_key(*key)),
        );
        keys.sort_by(|a, b| utf16::cmp(a, b));

        let mut fonts: IndexMap<String, FontEntry> = IndexMap::new();
        for key in keys {
            let recorded = presentation.fonts.get(key).map(|entry| &entry.request);
            let used = requests.get(key);
            let request = FontRequest {
                family: recorded
                    .map(|r| r.family.clone())
                    .or_else(|| used.map(|(family, _)| family.clone()))
                    .unwrap_or_else(|| key.clone()),
                faces: merge_faces(
                    recorded
                        .into_iter()
                        .flat_map(|r| r.faces.iter().cloned())
                        .chain(
                            used.into_iter()
                                .flat_map(|(_, faces)| faces.iter().cloned()),
                        ),
                ),
                postscript_names: recorded
                    .map(|r| r.postscript_names.clone())
                    .unwrap_or_default(),
                embedded: recorded.is_some_and(|r| r.embedded),
            };
            let bound = bind_family(ctx.fonts, &request.family, &request.postscript_names)?;
            let faces = drawn_faces(&bound, &request.faces)?;
            let mut binding = bound.binding.clone();
            if !faces.is_empty() {
                binding.faces = Some(faces);
            }

            let refusal = ctx
                .fonts
                .refusal(&request.family, &request.postscript_names);
            let code = match (binding.kind, refusal) {
                (BindingKind::Installed, _) => None,
                (BindingKind::Generic, _) if binding.generic == GenericFamily::Emoji => {
                    ctx.diag.report(
                        codes::MID5406,
                        DiagnosticSite::new().data("family", request.family.as_str()),
                    );
                    None
                }
                (_, Some(_)) => Some(codes::MID5404),
                (BindingKind::Substitute, None) => Some(codes::MID5401),
                (BindingKind::Generic, None) => Some(codes::MID5402),
            };
            if let Some(code) = code {
                let site = || {
                    let site = DiagnosticSite::new()
                        .data("family", request.family.as_str())
                        .data("binding", binding.family.as_str());
                    match refusal {
                        Some(reason) => site.data("reason", reason),
                        None => site,
                    }
                };
                match used_by.get(key) {
                    Some(users) => {
                        for container in users {
                            ctx.diag.report(code, site().container(container.clone()));
                        }
                    }
                    None => ctx.diag.report(code, site()),
                }
            }
            // A variable face whose instance at a weight the presentation draws was not read binds at its default
            // instance there; that weight is approximated.
            let mut approximated = None;
            for f in &request.faces {
                let drawn = bound.binding.drawn_weight(f.weight);
                let face = matched(bound.family, drawn, f.style)?;
                if face.variable && drawn != face.weight && face.metrics_at(drawn).is_none() {
                    approximated = Some(drawn);
                    break;
                }
            }
            if let Some(weight) = approximated {
                ctx.diag.report(
                    codes::MID5405,
                    DiagnosticSite::new()
                        .data("family", binding.family.as_str())
                        .data("weight", weight),
                );
            }
            fonts.insert(
                key.clone(),
                FontEntry {
                    request,
                    binding: Some(binding),
                },
            );
        }
        let mut out = presentation.clone();
        out.fonts = fonts;
        Ok(out)
    }
}
