//! plan-media: what the output does with every media item, decided from its sniffed type, size and codec and the
//! policy's budgets; the host does the I/O. Images a browser shows are copied, and so is an SVG its frontend sized
//! (only an SVG that passed `slidock_io::check_svg` has a size); TIFF and BMP are transcoded to PNG; other vector and
//! unknown types are omitted (a visible loss, MID5601); video plays when a browser can decode its codec and
//! keeps only its poster otherwise (MID5605); audio is omitted (its frontend already reported it). Items over the
//! per-item budget are omitted (MID5603); past the presentation budget the largest items of the latest slides go first
//! (MID5604). The rasters transcoded share a pixel budget too: past it, the later ones in the media table are omitted
//! (MID5610). A HEIC photo a host decoded ([`HostDecoded`]: its browser drew it) is transcoded to the host's PNG or
//! JPEG file (MID5608: its bytes depend on that browser); without one it is omitted (MID5601), unless the picture
//! showing it carries a fallback the output shows (Keynote's JPEG preview of the photo): the picture then draws the
//! fallback, a loss reported on it with both sizes (MID5609). Every picture's fallback is resolved here, used or not,
//! so no later stage sees one. Each other loss is reported on every container that shows the item.

use indexmap::IndexMap;
use slidock_core_api::visit::{container_media, map_elements};
use slidock_core_api::{
    HOST_DECODED, HostDecoded, MediaPolicy, PassCtx, PassError, PassInfo, PresentationPass,
};
use slidock_ir::diagnostics::{DataValue, DiagnosticCode, DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::media::{MediaEntry, MediaFile, MediaPlan, Mime, PlanAction, PlanOutput};
use slidock_ir::source::{Element, Presentation};
use slidock_ir::verify::Invariant;

pub struct PlanMedia;

static INFO: PassInfo = PassInfo::new(
    "plan-media",
    1,
    &[Invariant::SourceValid],
    &[Invariant::MediaPlanned],
);

/// What a browser shows as is, by type, with the extension the file set uses.
fn copied(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "video/webm" => "webm",
        _ => return None,
    })
}

const TRANSCODED: [&str; 2] = ["image/tiff", "image/bmp"];

fn video(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "video/mp4" | "video/x-m4v" => "mp4",
        "video/quicktime" => "mov",
        _ => return None,
    })
}

/// Codecs every browser plays.
const PLAYABLE: [&str; 4] = ["avc1", "avc3", "vp09", "av01"];
/// Codecs only Safari plays (HEVC, ProRes).
const SAFARI_ONLY: [&str; 8] = [
    "hvc1", "hev1", "ap4h", "ap4x", "apch", "apcn", "apcs", "apco",
];
const SVG: &str = "image/svg+xml";
const VECTOR: [&str; 4] = [SVG, "image/emf", "image/wmf", "application/pdf"];

/// A diagnostic to report on every container that shows an item: its code, feature and data.
type Report = (
    DiagnosticCode,
    Option<FeatureId>,
    Vec<(&'static str, DataValue)>,
);

/// A plan, and what to report on the containers showing the item.
struct Decision {
    plan: MediaPlan,
    report: Option<Report>,
}

fn output(entry: &MediaEntry, mime: &str, ext: &str) -> Result<PlanOutput, PassError> {
    Ok(PlanOutput {
        mime: Mime::new(mime).map_err(|e| PassError::failed(e.to_string()))?,
        file: MediaFile::of(&entry.sha256, ext).map_err(|e| PassError::failed(e.to_string()))?,
    })
}

fn plan(action: PlanAction, output: Option<PlanOutput>) -> MediaPlan {
    MediaPlan {
        action,
        output,
        reason: None,
    }
}

fn omit(reason: &str) -> MediaPlan {
    MediaPlan {
        action: PlanAction::Omit,
        output: None,
        reason: Some(reason.to_owned()),
    }
}

fn decide(
    entry: &MediaEntry,
    policy: &MediaPolicy,
    host: Option<&HostDecoded>,
) -> Result<Decision, PassError> {
    let mime = entry.mime.as_str();
    let typed = || ("type", DataValue::from(mime));
    if entry.bytes > policy.item_bytes {
        return Ok(Decision {
            plan: omit("too-large"),
            report: Some((
                codes::MID5603,
                None,
                vec![
                    typed(),
                    ("bytes", entry.bytes.into()),
                    ("limit", policy.item_bytes.into()),
                ],
            )),
        });
    }
    if let Some(ext) = copied(mime) {
        let animated = entry.frames.is_some_and(|frames| frames > 1);
        return Ok(Decision {
            plan: plan(PlanAction::Copy, Some(output(entry, mime, ext)?)),
            report: animated.then(|| (codes::MID5606, None, Vec::new())),
        });
    }
    // An SVG is sized only when it passed the allow-list (`slidock_io::check_svg`), so only then is it shown.
    if mime == SVG && entry.width.is_some() && entry.height.is_some() {
        return Ok(Decision {
            plan: plan(PlanAction::Copy, Some(output(entry, mime, "svg")?)),
            report: None,
        });
    }
    if TRANSCODED.contains(&mime) {
        let pixels = entry
            .width
            .unwrap_or(0)
            .saturating_mul(entry.height.unwrap_or(0));
        if pixels > policy.transcode_pixels {
            return Ok(Decision {
                plan: omit("too-large"),
                report: Some((
                    codes::MID5603,
                    None,
                    vec![
                        typed(),
                        ("pixels", pixels.into()),
                        ("limit", policy.transcode_pixels.into()),
                    ],
                )),
            });
        }
        return Ok(Decision {
            plan: plan(
                PlanAction::Transcode,
                Some(output(entry, "image/png", "png")?),
            ),
            report: Some((
                codes::MID5602,
                None,
                vec![typed(), ("output", "PNG".into())],
            )),
        });
    }
    if let Some(ext) = video(mime) {
        let codec = entry.codec.as_deref().unwrap_or("unknown");
        let copy = || -> Result<MediaPlan, PassError> {
            Ok(plan(PlanAction::Copy, Some(output(entry, mime, ext)?)))
        };
        if PLAYABLE.contains(&codec) {
            return Ok(Decision {
                plan: copy()?,
                report: None,
            });
        }
        let codec_data = vec![("codec", DataValue::from(codec))];
        if SAFARI_ONLY.contains(&codec) {
            return Ok(Decision {
                plan: copy()?,
                report: Some((codes::MID5607, None, codec_data)),
            });
        }
        return Ok(Decision {
            plan: omit("unsupported-codec"),
            report: Some((codes::MID5605, None, codec_data)),
        });
    }
    if let Some(decoded) = host.filter(|_| HOST_DECODED.contains(&mime)) {
        let (ext, format) = match decoded.mime {
            "image/png" => ("png", "PNG"),
            _ => ("jpg", "JPEG"),
        };
        return Ok(Decision {
            plan: plan(
                PlanAction::Transcode,
                Some(output(entry, decoded.mime, ext)?),
            ),
            report: Some((
                codes::MID5608,
                None,
                vec![typed(), ("output", format.into())],
            )),
        });
    }
    if mime.starts_with("audio/") {
        return Ok(Decision {
            plan: omit("audio"),
            report: None,
        });
    }
    let feature = VECTOR.contains(&mime).then_some(FeatureId::ImageVector);
    Ok(Decision {
        plan: omit("unsupported-type"),
        report: Some((codes::MID5601, feature, vec![typed()])),
    })
}

/// An item's size as a diagnostic names it (`1600×1200`), when its header gave one.
fn size_of(entry: &MediaEntry) -> Option<String> {
    Some(format!("{}×{}", entry.width?, entry.height?))
}

/// A type as a diagnostic names it: its subtype in capitals (`HEIC`).
fn type_label(mime: &str) -> String {
    mime.rsplit('/').next().unwrap_or(mime).to_uppercase()
}

/// A picture drawn from its fallback: where, and what it reports (MID5609).
struct Fallback {
    container: NodeId,
    element: NodeId,
    data: Vec<(&'static str, DataValue)>,
}

/// Resolves every picture's fallback in `presentation`: a picture whose media the output cannot show (MID5601: no
/// host decoded it) draws its fallback when the output shows that; the fallback is then dropped either way. Returns
/// the pictures drawn from their fallback.
fn resolve_fallbacks(
    presentation: &mut Presentation,
    decisions: &IndexMap<Sha256, Decision>,
) -> Vec<Fallback> {
    let unshown = |sha: &Sha256| {
        decisions
            .get(sha)
            .is_some_and(|d| d.plan.reason.as_deref() == Some("unsupported-type"))
    };
    let shown = |sha: &Sha256| {
        decisions
            .get(sha)
            .is_some_and(|d| d.plan.action != PlanAction::Omit)
    };
    let media = &presentation.media;
    let mut drawn = Vec::new();
    // A failed container prints nothing, so nothing is reported on it.
    let mut resolve = |container: NodeId, failed: bool, elements: &[Element]| {
        map_elements(elements, &mut |element| {
            let Element::Picture(picture) = element else {
                return None;
            };
            let fallback = picture.fallback?;
            let mut out = picture.clone();
            out.fallback = None;
            if unshown(&picture.media) && shown(&fallback) {
                out.media = fallback;
                if failed {
                    return Some(Element::Picture(out));
                }
                let original = media.get(&picture.media);
                let label =
                    original.map_or_else(String::new, |entry| type_label(entry.mime.as_str()));
                let original = match original.and_then(size_of) {
                    Some(size) => format!("{size} {label}"),
                    None => label,
                };
                let preview = media
                    .get(&fallback)
                    .and_then(size_of)
                    .unwrap_or_else(|| "raster".to_owned());
                drawn.push(Fallback {
                    container: container.clone(),
                    element: picture.id.clone(),
                    data: vec![
                        ("sha256", picture.media.to_hex().into()),
                        ("preview", preview.into()),
                        ("original", original.into()),
                    ],
                });
            }
            Some(Element::Picture(out))
        })
    };
    for master in presentation.masters.values_mut() {
        if let Some(elements) =
            resolve(master.id.clone(), master.failed.is_some(), &master.elements)
        {
            master.elements = elements;
        }
    }
    for layout in presentation.layouts.values_mut() {
        if let Some(elements) =
            resolve(layout.id.clone(), layout.failed.is_some(), &layout.elements)
        {
            layout.elements = elements;
        }
    }
    for slide in &mut presentation.slides {
        if let Some(elements) = resolve(slide.id.clone(), slide.failed.is_some(), &slide.elements) {
            slide.elements = elements;
        }
    }
    drawn
}

/// Which containers show each media item, in presentation order, and the last show position of each.
struct Usage {
    users: IndexMap<Sha256, Vec<NodeId>>,
    last_shown: IndexMap<Sha256, i64>,
}

fn usage(presentation: &Presentation) -> Usage {
    let mut usage = Usage {
        users: IndexMap::new(),
        last_shown: IndexMap::new(),
    };
    for (container, shown) in container_media(presentation) {
        let index = presentation
            .slides
            .iter()
            .position(|slide| slide.id == container)
            .map_or(-1, |i| i as i64);
        for sha in shown {
            usage.users.entry(sha).or_default().push(container.clone());
            let last = usage.last_shown.entry(sha).or_insert(-1);
            *last = (*last).max(index);
        }
    }
    usage
}

impl PresentationPass for PlanMedia {
    fn info(&self) -> &PassInfo {
        &INFO
    }

    fn run(
        &self,
        presentation: &Presentation,
        ctx: &mut PassCtx<'_>,
    ) -> Result<Presentation, PassError> {
        let policy = ctx.policy;
        let mut decisions: IndexMap<Sha256, Decision> = IndexMap::new();
        let host_media = ctx.host_media;
        let host = |sha: &Sha256| host_media.iter().find(|decoded| &decoded.sha256 == sha);
        for entry in presentation.media.values() {
            decisions.insert(entry.sha256, decide(entry, policy, host(&entry.sha256))?);
        }
        // The budget and the reports count what each container shows once the fallbacks are drawn.
        let mut out = presentation.clone();
        let fallbacks = resolve_fallbacks(&mut out, &decisions);
        let presentation = &out;
        let usage = usage(presentation);
        // A decoded item counts its file's size.
        let bytes = |sha: &Sha256| {
            host(sha)
                .map(|decoded| decoded.bytes)
                .unwrap_or_else(|| presentation.media.get(sha).map_or(0, |entry| entry.bytes))
        };

        // The rasters the compiler transcodes and a container shows count their pixels against the presentation's
        // transcode budget, in the media table's order; one that would pass it is omitted (MID5610).
        let mut transcoded: u64 = 0;
        for (sha, decision) in &mut decisions {
            let Some(entry) = presentation.media.get(sha) else {
                continue;
            };
            if decision.plan.action != PlanAction::Transcode
                || !TRANSCODED.contains(&entry.mime.as_str())
                || !usage.users.contains_key(sha)
            {
                continue;
            }
            let pixels = entry
                .width
                .unwrap_or(0)
                .saturating_mul(entry.height.unwrap_or(0));
            let with = transcoded.saturating_add(pixels);
            if with > policy.presentation_transcode_pixels {
                *decision = Decision {
                    plan: omit("over-budget"),
                    report: Some((
                        codes::MID5610,
                        None,
                        vec![
                            ("pixels", pixels.into()),
                            ("budget", policy.presentation_transcode_pixels.into()),
                        ],
                    )),
                };
            } else {
                transcoded = with;
            }
        }

        // The presentation budget counts the items a container shows. A partial compile counts fewer, and runs only
        // when the whole presentation's media fit (IR0912), so it omits what the whole compile does: nothing.
        let mut total: u64 = decisions
            .iter()
            .filter(|(sha, d)| d.plan.action != PlanAction::Omit && usage.users.contains_key(*sha))
            .map(|(sha, _)| bytes(sha))
            .fold(0, u64::saturating_add);
        let mut candidates: Vec<Sha256> = decisions
            .iter()
            .filter(|(_, d)| d.plan.action != PlanAction::Omit)
            .map(|(sha, _)| *sha)
            .collect();
        let last = |sha: &Sha256| usage.last_shown.get(sha).copied().unwrap_or(-1);
        candidates.sort_by(|a, b| {
            last(b)
                .cmp(&last(a))
                .then_with(|| bytes(b).cmp(&bytes(a)))
                .then_with(|| a.cmp(b))
        });
        for sha in candidates {
            if total <= policy.presentation_bytes {
                break;
            }
            let over = Decision {
                plan: omit("over-budget"),
                report: Some((
                    codes::MID5604,
                    None,
                    vec![
                        ("bytes", bytes(&sha).into()),
                        ("budget", policy.presentation_bytes.into()),
                    ],
                )),
            };
            decisions.insert(sha, over);
            total = total.saturating_sub(bytes(&sha));
        }

        for (sha, decision) in &decisions {
            let Some((code, feature, data)) = &decision.report else {
                continue;
            };
            for container in usage.users.get(sha).into_iter().flatten() {
                let mut site = DiagnosticSite::new()
                    .container(container.clone())
                    .data("sha256", sha.to_hex());
                if let Some(feature) = feature {
                    site = site.feature(*feature);
                }
                for (key, value) in data {
                    site = site.data(*key, value.clone());
                }
                ctx.diag.report(*code, site);
            }
        }
        for Fallback {
            container,
            element,
            data,
        } in fallbacks
        {
            let mut site = DiagnosticSite::new().container(container).node(element);
            for (key, value) in data {
                site = site.data(key, value);
            }
            ctx.diag.report(codes::MID5609, site);
        }
        let mut out = out;
        for (sha, entry) in &mut out.media {
            if let Some(decision) = decisions.get(sha) {
                entry.plan = Some(decision.plan.clone());
            }
        }
        Ok(out)
    }
}
