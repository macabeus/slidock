//! The native motion data: what the common vocabulary adds to the IR. That is a step's native effect, timing and
//! background target, a transition's native effect and easing, and the elements' match keys. The parity path
//! compiles without it, so its outputs stay those of the IR before the vocabulary existed. It is stripped from a
//! verified presentation only: the parity path refuses what the deck path refuses.

use crate::source::{Element, Presentation};

/// Removes the native motion data from a presentation.
pub fn strip_native_motion(presentation: &mut Presentation) {
    let mut elements: Vec<&mut Element> = Vec::new();
    for slide in &mut presentation.slides {
        for step in &mut slide.timeline {
            step.effect.native = None;
            step.timing = None;
            step.target.background = false;
        }
        if let Some(transition) = &mut slide.transition {
            transition.effect.native = None;
            transition.easing = None;
        }
        elements.extend(slide.elements.iter_mut());
    }
    for layout in presentation.layouts.values_mut() {
        elements.extend(layout.elements.iter_mut());
    }
    for master in presentation.masters.values_mut() {
        elements.extend(master.elements.iter_mut());
    }
    while let Some(element) = elements.pop() {
        *element.match_key_mut() = None;
        match element {
            Element::Group(group) => elements.extend(group.children.iter_mut()),
            Element::Foreign(foreign) => {
                if let Some(fallback) = foreign
                    .fallback
                    .as_mut()
                    .and_then(|fallback| fallback.elements.as_mut())
                {
                    elements.extend(fallback.iter_mut());
                }
            }
            _ => {}
        }
    }
}
