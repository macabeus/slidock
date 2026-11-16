//! Serde adapters for zod's field modifiers. `#[serde(default, skip_serializing_if = "Option::is_none", with =
//! "optional")]` is `.optional()` (absent or a value, never `null`); `with = "nullable"` is `.nullable()` (a
//! required key that may be `null`); `with = "double_option"` on a [`slidock_ir::Nullable`] is
//! `.nullable().optional()`.

pub(crate) use slidock_ir::{double_option, nullable};

pub(crate) mod optional {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, T: Serialize>(
        value: &Option<T>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Option<T>, D::Error> {
        T::deserialize(deserializer).map(Some)
    }
}
