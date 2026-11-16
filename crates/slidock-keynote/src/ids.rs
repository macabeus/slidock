//! Keynote ids: the object ids of the index, which survive saves. Slides are `kn-<slide node id>`, layouts
//! `kn-<template slide id>`, elements `kn-<drawable id>`. Each is unique in its document, so one scope covers the
//! presentation.

use slidock_core_api::lowering::IdScope;
use slidock_ir::formats::IdPrefix;
use slidock_ir::ids::{IdError, IdPart, NodeId, node_id};

use crate::iwa::ObjectId;

/// The IR id of an object.
pub fn kn_id(object: ObjectId) -> Result<NodeId, IdError> {
    node_id(IdPrefix::Kn, &[IdPart::Str(&object.to_string())])
}

/// The presentation's one id scope: an object keeps its id where it is lowered first and gets `.<n>` where placed
/// again.
pub fn id_scope() -> IdScope {
    IdScope::new(IdPrefix::Kn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_objects_by_their_decimal_id() {
        assert_eq!(kn_id(ObjectId(5_213_335)).unwrap().as_str(), "kn-5213335");
        assert_eq!(
            kn_id(ObjectId(u64::MAX)).unwrap().as_str(),
            "kn-18446744073709551615"
        );
        let mut scope = id_scope();
        let part = "7".to_owned();
        assert_eq!(
            scope.take(&[IdPart::Str(&part)]).unwrap().id.as_str(),
            "kn-7"
        );
        let again = scope.take(&[IdPart::Str(&part)]).unwrap();
        assert_eq!((again.id.as_str(), again.duplicate), ("kn-7.2", true));
    }
}
