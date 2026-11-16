//! UTF-8 text as `new TextDecoder()` decodes it.

use std::borrow::Cow;

/// `bytes` as UTF-8 text: a leading byte order mark dropped, each invalid sequence replaced with U+FFFD.
pub(crate) fn decode_utf8(bytes: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))
}
