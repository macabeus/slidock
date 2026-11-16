//! The attributes and style properties the frontend reads, by canonical qualified name (`draw:fill`). Names are
//! written with [`qn!`], which looks them up in [`VOCABULARY`] at compile time: a name outside it does not build.

/// An attribute or style property name from [`VOCABULARY`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct QName(&'static str);

impl QName {
    pub(crate) const fn as_str(self) -> &'static str {
        self.0
    }

    /// `name`, which must be in [`VOCABULARY`]; [`qn!`] evaluates this at compile time.
    #[allow(
        clippy::panic,
        reason = "evaluated in a const block, where it is a compile error"
    )]
    pub(crate) const fn known(name: &'static str) -> QName {
        let mut rest = VOCABULARY;
        while let Some((first, tail)) = rest.split_first() {
            if bytes_eq(first.as_bytes(), name.as_bytes()) {
                return QName(name);
            }
            rest = tail;
        }
        panic!("the name is not in the ODF vocabulary (names.rs)")
    }
}

const fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let (mut a, mut b) = (a, b);
    while let (Some((x, a_rest)), Some((y, b_rest))) = (a.split_first(), b.split_first()) {
        if *x != *y {
            return false;
        }
        (a, b) = (a_rest, b_rest);
    }
    true
}

/// A name of [`VOCABULARY`] as a [`QName`], checked at compile time.
macro_rules! qn {
    ($name:literal) => {
        const { $crate::names::QName::known($name) }
    };
}
pub(crate) use qn;

/// Every name the frontend reads, sorted.
pub(crate) const VOCABULARY: &[&str] = &[
    "anim:command",
    "anim:formula",
    "anim:iterate-interval",
    "anim:iterate-type",
    "anim:sub-item",
    "draw:angle",
    "draw:auto-grow-height",
    "draw:auto-grow-width",
    "draw:blue",
    "draw:border",
    "draw:color",
    "draw:color-inversion",
    "draw:color-mode",
    "draw:contrast",
    "draw:corner-radius",
    "draw:cx",
    "draw:cy",
    "draw:display",
    "draw:distance",
    "draw:dots1",
    "draw:dots1-length",
    "draw:dots2",
    "draw:dots2-length",
    "draw:end",
    "draw:end-angle",
    "draw:end-color",
    "draw:end-intensity",
    "draw:enhanced-path",
    "draw:fill",
    "draw:fill-color",
    "draw:fill-gradient-name",
    "draw:fill-hatch-name",
    "draw:fill-hatch-solid",
    "draw:fill-image-name",
    "draw:fit-to-size",
    "draw:formula",
    "draw:gamma",
    "draw:gradient-step-count",
    "draw:green",
    "draw:id",
    "draw:image-opacity",
    "draw:kind",
    "draw:luminance",
    "draw:marker-end",
    "draw:marker-end-width",
    "draw:marker-start",
    "draw:marker-start-width",
    "draw:master-page-name",
    "draw:mime-type",
    "draw:mirror-horizontal",
    "draw:mirror-vertical",
    "draw:modifiers",
    "draw:name",
    "draw:opacity",
    "draw:opacity-name",
    "draw:path-stretchpoint-x",
    "draw:path-stretchpoint-y",
    "draw:points",
    "draw:red",
    "draw:shadow",
    "draw:shadow-color",
    "draw:shadow-offset-x",
    "draw:shadow-offset-y",
    "draw:shadow-opacity",
    "draw:start",
    "draw:start-angle",
    "draw:start-color",
    "draw:start-intensity",
    "draw:stroke",
    "draw:stroke-dash",
    "draw:stroke-linejoin",
    "draw:style",
    "draw:style-name",
    "draw:text-style-name",
    "draw:textarea-horizontal-align",
    "draw:textarea-vertical-align",
    "draw:transform",
    "draw:type",
    "draw:value",
    "fo:background-color",
    "fo:border",
    "fo:border-bottom",
    "fo:border-left",
    "fo:border-right",
    "fo:border-top",
    "fo:clip",
    "fo:color",
    "fo:column-count",
    "fo:font-family",
    "fo:font-size",
    "fo:font-style",
    "fo:font-variant",
    "fo:font-weight",
    "fo:letter-spacing",
    "fo:line-height",
    "fo:margin",
    "fo:margin-bottom",
    "fo:margin-left",
    "fo:margin-right",
    "fo:margin-top",
    "fo:padding",
    "fo:padding-bottom",
    "fo:padding-left",
    "fo:padding-right",
    "fo:padding-top",
    "fo:page-height",
    "fo:page-width",
    "fo:text-align",
    "fo:text-indent",
    "fo:text-shadow",
    "fo:text-transform",
    "fo:wrap-option",
    "loext:color-type",
    "loext:color-value",
    "loext:font-style",
    "loext:font-weight",
    "loext:shadow-blur",
    "manifest:full-path",
    "manifest:media-type",
    "office:version",
    "presentation:action",
    "presentation:after-effect",
    "presentation:background-objects-visible",
    "presentation:background-visible",
    "presentation:class",
    "presentation:display-date-time",
    "presentation:display-footer",
    "presentation:display-page-number",
    "presentation:duration",
    "presentation:master-element",
    "presentation:name",
    "presentation:node-type",
    "presentation:placeholder",
    "presentation:preset-class",
    "presentation:preset-id",
    "presentation:preset-sub-type",
    "presentation:source",
    "presentation:style-name",
    "presentation:transition-speed",
    "presentation:transition-style",
    "presentation:transition-type",
    "presentation:use-date-time-name",
    "presentation:use-footer-name",
    "presentation:use-header-name",
    "presentation:visibility",
    "script:event-name",
    "smil:accelerate",
    "smil:attributeName",
    "smil:autoReverse",
    "smil:begin",
    "smil:by",
    "smil:calcMode",
    "smil:decelerate",
    "smil:direction",
    "smil:dur",
    "smil:end",
    "smil:fadeColor",
    "smil:fill",
    "smil:from",
    "smil:keySplines",
    "smil:keyTimes",
    "smil:mode",
    "smil:repeatCount",
    "smil:repeatDur",
    "smil:subtype",
    "smil:targetElement",
    "smil:to",
    "smil:type",
    "smil:values",
    "style:column-width",
    "style:diagonal-bl-tr",
    "style:diagonal-tl-br",
    "style:display-name",
    "style:family",
    "style:font-name",
    "style:font-relief",
    "style:line-height-at-least",
    "style:line-spacing",
    "style:list-style-name",
    "style:min-row-height",
    "style:mirror",
    "style:name",
    "style:num-format",
    "style:num-prefix",
    "style:num-suffix",
    "style:page-layout-name",
    "style:parent-style-name",
    "style:position",
    "style:rel-column-width",
    "style:repeat",
    "style:row-height",
    "style:shrink-to-fit",
    "style:tab-stop-distance",
    "style:text-line-through-style",
    "style:text-line-through-type",
    "style:text-outline",
    "style:text-position",
    "style:text-underline-style",
    "style:text-underline-type",
    "style:use-window-font-color",
    "style:vertical-align",
    "style:writing-mode",
    "svg:d",
    "svg:font-family",
    "svg:height",
    "svg:offset",
    "svg:path",
    "svg:stop-opacity",
    "svg:string",
    "svg:stroke-color",
    "svg:stroke-linecap",
    "svg:stroke-opacity",
    "svg:stroke-width",
    "svg:type",
    "svg:viewBox",
    "svg:width",
    "svg:x",
    "svg:x1",
    "svg:x2",
    "svg:y",
    "svg:y1",
    "svg:y2",
    "table:default-cell-style-name",
    "table:name",
    "table:number-columns-repeated",
    "table:number-columns-spanned",
    "table:number-rows-repeated",
    "table:number-rows-spanned",
    "table:style-name",
    "table:template-name",
    "table:use-banding-columns-styles",
    "table:use-banding-rows-styles",
    "table:use-first-column-styles",
    "table:use-first-row-styles",
    "table:use-last-column-styles",
    "table:use-last-row-styles",
    "text:bullet-char",
    "text:bullet-relative-size",
    "text:c",
    "text:level",
    "text:list-level-position-and-space-mode",
    "text:min-label-width",
    "text:space-before",
    "text:start-value",
    "text:style-name",
    "xlink:href",
    "xml:id",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_each_name_once_in_order() {
        assert!(VOCABULARY.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(qn!("draw:fill").as_str(), "draw:fill");
    }

    /// The timing and transition names native motion reads beyond the static vocabulary, which only detects motion.
    const MOTION: [&str; 35] = [
        "anim:command",
        "anim:formula",
        "anim:iterate-interval",
        "anim:iterate-type",
        "anim:sub-item",
        "presentation:after-effect",
        "presentation:duration",
        "presentation:master-element",
        "presentation:node-type",
        "presentation:preset-id",
        "presentation:preset-sub-type",
        "presentation:transition-speed",
        "smil:accelerate",
        "smil:attributeName",
        "smil:autoReverse",
        "smil:begin",
        "smil:by",
        "smil:calcMode",
        "smil:decelerate",
        "smil:direction",
        "smil:dur",
        "smil:end",
        "smil:fadeColor",
        "smil:fill",
        "smil:from",
        "smil:keySplines",
        "smil:keyTimes",
        "smil:mode",
        "smil:repeatCount",
        "smil:repeatDur",
        "smil:subtype",
        "smil:to",
        "smil:values",
        "svg:path",
        "svg:type",
    ];

    /// Each name is one of the static vocabulary (`tests/vectors/odf-names.txt`), one of the embedded-font declarations
    /// the frontend leaves to the font stage, or a motion name.
    #[test]
    fn reads_only_names_of_the_static_vocabulary() {
        let known: Vec<&str> = include_str!("../tests/vectors/odf-names.txt")
            .lines()
            .filter(|line| !line.starts_with('#'))
            .chain(["loext:font-style", "loext:font-weight", "svg:string"])
            .chain(MOTION)
            .collect();
        let unknown: Vec<&&str> = VOCABULARY
            .iter()
            .filter(|name| !known.contains(name))
            .collect();
        assert!(unknown.is_empty(), "{unknown:?}");
    }
}
