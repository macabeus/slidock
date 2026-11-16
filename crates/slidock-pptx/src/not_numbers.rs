//! Custom-geometry coordinates that are not numbers fail their container with IR0001. The IR has no value for NaN, so a
//! presentation with such coordinates goes to verify-source as JSON with `null` at each of them, and the verifier fails
//! those containers the same way.

use serde_json::Value;
use slidock_core_api::Unverified;
use slidock_ir::ids::NodeId;
use slidock_ir::source::Presentation;

use crate::context::ElementNaN;

/// The presentation as verify-source reads it: typed, or JSON with `null` at each coordinate in `nan` (by container).
pub(crate) fn unverified(
    presentation: Presentation,
    nan: &[(NodeId, Vec<ElementNaN>)],
) -> Result<Unverified, serde_json::Error> {
    if nan.iter().all(|(_, elements)| elements.is_empty()) {
        return Ok(Unverified::Typed(Box::new(presentation)));
    }
    let mut value = serde_json::to_value(&presentation)?;
    for (container, elements) in nan {
        let Some(container) = container_mut(&mut value, container) else {
            continue;
        };
        for nan in elements {
            if let Some(coordinate) = element_mut(container, &nan.node)
                .and_then(|element| element.get_mut(nan.field))
                .and_then(|geometry| geometry.get_mut("paths"))
                .and_then(|paths| paths.get_mut(nan.at.path))
                .and_then(|path| path.get_mut("segments"))
                .and_then(|segments| segments.get_mut(nan.at.segment))
                .and_then(|segment| segment.get_mut(nan.at.index))
            {
                *coordinate = Value::Null;
            }
        }
    }
    Ok(Unverified::Json(value))
}

fn container_mut<'a>(presentation: &'a mut Value, id: &NodeId) -> Option<&'a mut Value> {
    let id = id.as_str();
    for kind in ["masters", "layouts"] {
        if presentation.get(kind).and_then(|map| map.get(id)).is_some() {
            return presentation.get_mut(kind)?.get_mut(id);
        }
    }
    presentation
        .get_mut("slides")?
        .as_array_mut()?
        .iter_mut()
        .find(|slide| slide.get("id").and_then(Value::as_str) == Some(id))
}

/// The element `id` among a container's elements and their groups' children.
fn element_mut<'a>(container: &'a mut Value, id: &NodeId) -> Option<&'a mut Value> {
    fn find<'a>(elements: &'a mut [Value], id: &str) -> Option<&'a mut Value> {
        for element in elements {
            if element.get("id").and_then(Value::as_str) == Some(id) {
                return Some(element);
            }
            if let Some(found) = element
                .get_mut("children")
                .and_then(Value::as_array_mut)
                .and_then(|children| find(children, id))
            {
                return Some(found);
            }
        }
        None
    }
    find(container.get_mut("elements")?.as_array_mut()?, id.as_str())
}
