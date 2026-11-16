//! ODP ids. ODF names objects only optionally (`xml:id`, `draw:id`), so when a page or drawable has neither, its
//! 1-based document-order index inside its part stands in: slides `od-p<n>`, drawables `od-<owner>.<n>`. Master pages
//! are named: `od-m.<style:name>`. Indexes are stable for a given file.

use slidock_core_api::lowering::{IdScope, LowerError, TakenId};
use slidock_ir::formats::IdPrefix;
use slidock_ir::ids::{IdError, IdPart, NodeId, node_id};

use crate::names::qn;
use crate::odf::Element;

/// The ids of one namespace: the document's containers, or one container's elements.
pub(crate) fn id_scope() -> IdScope {
    IdScope::new(IdPrefix::Od)
}

/// The native id an element states, if any: `xml:id`, else the older `draw:id`.
pub(crate) fn native_id(element: &Element) -> Option<&str> {
    element
        .attr(qn!("xml:id"))
        .filter(|s| !s.is_empty())
        .or_else(|| element.attr(qn!("draw:id")).filter(|s| !s.is_empty()))
}

/// A slide's key: its native id, else `p<n>`.
pub(crate) fn slide_key(page: &Element, index: usize) -> String {
    native_id(page).map_or_else(|| format!("p{index}"), str::to_owned)
}

/// A master page's id parts.
pub(crate) fn master_parts(name: &str) -> [IdPart<'_>; 2] {
    [IdPart::Str("m"), IdPart::Str(name)]
}

/// A master page's id.
pub(crate) fn master_id(name: &str) -> Result<NodeId, IdError> {
    node_id(IdPrefix::Od, &master_parts(name))
}

/// An id error as a lowering error; ids are escaped and fitted, so only a bug makes one.
pub(crate) fn id_failure(error: IdError) -> LowerError {
    LowerError::Other(error.to_string())
}

/// Takes an id in a scope.
pub(crate) fn take(scope: &mut IdScope, parts: &[IdPart<'_>]) -> Result<TakenId, LowerError> {
    scope.take(parts).map_err(id_failure)
}
