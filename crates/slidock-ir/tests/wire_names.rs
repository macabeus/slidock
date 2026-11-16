//! Each enum's `as_str` spells every value as the IR's JSON does.
#![allow(clippy::unwrap_used, reason = "test code")]

use serde::Serialize;
use slidock_ir::color::MappedSlot;
use slidock_ir::elements::{ForeignType, PlaceholderRole};
use slidock_ir::fonts::GenericFamily;
use slidock_ir::text::{FieldKind, LineBasis, LinkAction};

fn check<T: Serialize + Copy + std::fmt::Debug>(values: &[T], as_str: fn(T) -> &'static str) {
    for &value in values {
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::Value::String(as_str(value).to_owned()),
            "{value:?}"
        );
    }
}

#[test]
fn spells_every_value_as_the_json_does() {
    use PlaceholderRole as R;
    check(
        &[
            R::Title,
            R::CtrTitle,
            R::SubTitle,
            R::Body,
            R::Obj,
            R::Pic,
            R::Chart,
            R::Tbl,
            R::Media,
            R::Dt,
            R::Ftr,
            R::SldNum,
            R::Hdr,
            R::Notes,
            R::Other,
        ],
        R::as_str,
    );
    use ForeignType as F;
    check(
        &[
            F::Table,
            F::Chart,
            F::Smartart,
            F::Ole,
            F::Math,
            F::Ink,
            F::ThreeD,
            F::Zoom,
            F::Unknown,
        ],
        F::as_str,
    );
    use MappedSlot as M;
    check(
        &[
            M::Dk1,
            M::Lt1,
            M::Dk2,
            M::Lt2,
            M::Accent1,
            M::Accent2,
            M::Accent3,
            M::Accent4,
            M::Accent5,
            M::Accent6,
            M::Hlink,
            M::FolHlink,
            M::Bg1,
            M::Tx1,
            M::Bg2,
            M::Tx2,
        ],
        M::as_str,
    );
    use GenericFamily as G;
    check(
        &[G::SansSerif, G::Serif, G::Monospace, G::Cursive, G::Fantasy],
        G::as_str,
    );
    use LinkAction as A;
    check(&[A::Next, A::Prev, A::First, A::Last], A::as_str);
    use FieldKind as K;
    check(
        &[K::SlideNumber, K::SlideCount, K::Date, K::Time],
        K::as_str,
    );
    use LineBasis as B;
    check(
        &[B::Size1_2, B::Hhea, B::Typo, B::Win, B::Coretext],
        B::as_str,
    );
}
