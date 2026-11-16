//! What the slide-scope passes share: a container's fields by kind, and fallible element and text-body maps over
//! `slidock-core-api`'s visitors.

use slidock_core_api::visit::{TextSite, map_elements, map_text_bodies};
use slidock_core_api::{OwnedContainer, PassError};
use slidock_ir::paint::Fill;
use slidock_ir::source::{Container, Element};
use slidock_ir::text::TextBody;

pub(crate) fn elements_mut(container: &mut OwnedContainer) -> &mut Vec<Element> {
    match container {
        OwnedContainer::Master(c) => &mut c.elements,
        OwnedContainer::Layout(c) => &mut c.elements,
        OwnedContainer::Slide(c) => &mut c.elements,
    }
}

pub(crate) fn background_mut(container: &mut OwnedContainer) -> &mut Option<Fill> {
    match container {
        OwnedContainer::Master(c) => &mut c.background,
        OwnedContainer::Layout(c) => &mut c.background,
        OwnedContainer::Slide(c) => &mut c.background,
    }
}

/// `container` with its elements mapped bottom-up by `f` (`None` keeps an element); the first error stops the map.
pub(crate) fn with_mapped_elements(
    container: Container<'_>,
    f: impl FnMut(&Element) -> Result<Option<Element>, PassError>,
) -> Result<OwnedContainer, PassError> {
    let mut out = OwnedContainer::from(container);
    if let Some(elements) = try_map_elements(container.elements(), f)? {
        *elements_mut(&mut out) = elements;
    }
    Ok(out)
}

/// [`map_elements`] with a fallible `f`: the first error is returned and the elements after it are left alone.
pub(crate) fn try_map_elements(
    elements: &[Element],
    mut f: impl FnMut(&Element) -> Result<Option<Element>, PassError>,
) -> Result<Option<Vec<Element>>, PassError> {
    let mut failure: Option<PassError> = None;
    let mapped = map_elements(elements, &mut |element| {
        if failure.is_some() {
            return None;
        }
        f(element).unwrap_or_else(|error| {
            failure = Some(error);
            None
        })
    });
    match failure {
        Some(error) => Err(error),
        None => Ok(mapped),
    }
}

/// [`map_text_bodies`] with a fallible `f`, which returns every body it is given, changed or not.
pub(crate) fn try_map_text_bodies(
    element: &Element,
    mut f: impl FnMut(&TextBody, TextSite) -> Result<TextBody, PassError>,
) -> Result<Option<Element>, PassError> {
    let mut failure: Option<PassError> = None;
    let mapped = map_text_bodies(element, &mut |text, site| {
        if failure.is_some() {
            return None;
        }
        match f(text, site) {
            Ok(next) if &next == text => None,
            Ok(next) => Some(next),
            Err(error) => {
                failure = Some(error);
                None
            }
        }
    });
    match failure {
        Some(error) => Err(error),
        None => Ok(mapped),
    }
}

/// A value that cannot be stored on its unit's grid, as a pass failure.
pub(crate) fn unit_error(what: &str, error: impl std::fmt::Display) -> PassError {
    PassError::failed(format!("{what}: {error}"))
}
