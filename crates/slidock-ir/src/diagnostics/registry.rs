//! The diagnostic registry: every code slidock's emitters report, with its kebab-case name, severity, feature and
//! message template. The registry alone decides severities.
//!
//! A code's letters name its emitter: IR the core (input checks, verification, the driver), KN the Keynote frontend,
//! PX the PPTX frontend, OD the ODP frontend, MID the passes between frontend and backend, HTML the HTML backend. GS,
//! PD, RX and EXP are reserved for emitters to come. A number, once given a meaning, is never reused for another.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{CodePrefix, Severity};
use crate::features::FeatureId;

macro_rules! diagnostics {
    ($($code:ident => $variant:ident, $name:literal, $severity:ident, $feature:expr, $template:literal;)*) => {
        /// A registered diagnostic code. Serialises as its code (`"IR0001"`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum DiagnosticCode {
            $($variant,)*
        }

        /// Each code under its number (`codes::IR0909`).
        pub mod codes {
            use super::DiagnosticCode;
            $(pub const $code: DiagnosticCode = DiagnosticCode::$variant;)*
        }

        impl DiagnosticCode {
            /// Every registered code, in registry order.
            pub const ALL: &'static [DiagnosticCode] = &[$(DiagnosticCode::$variant,)*];

            /// The code (`"IR0001"`).
            pub const fn code(self) -> &'static str {
                match self {
                    $(DiagnosticCode::$variant => stringify!($code),)*
                }
            }

            /// The kebab-case name (`"schema-invalid"`).
            pub const fn name(self) -> &'static str {
                match self {
                    $(DiagnosticCode::$variant => $name,)*
                }
            }

            pub const fn severity(self) -> Severity {
                match self {
                    $(DiagnosticCode::$variant => Severity::$severity,)*
                }
            }

            /// The feature the code reports a loss of, if any.
            pub const fn feature(self) -> Option<FeatureId> {
                match self {
                    $(DiagnosticCode::$variant => $feature,)*
                }
            }

            /// The message, with `{key}` placeholders filled from the diagnostic's data.
            pub const fn template(self) -> &'static str {
                match self {
                    $(DiagnosticCode::$variant => $template,)*
                }
            }
        }
    };
}

diagnostics! {
    IR0001 => SchemaInvalid, "schema-invalid", Error, None, "Schema violation at {path}: {reason}";
    IR0002 => DuplicateId, "duplicate-id", Error, None, "Id {id} is used more than once";
    IR0003 => DanglingReference, "dangling-reference", Error, None, "The {kind} reference {target} does not resolve";
    IR0004 => Cycle, "cycle", Error, None, "Inheritance cycle through {id}";
    IR0005 => DepthExceeded, "depth-exceeded", Error, None, "{kind} depth exceeds {limit}";
    IR0006 => TooManySlides, "too-many-slides", Error, None, "The presentation has {count} slides; the limit is {limit}";
    IR0007 => NumberNotQuantised, "number-not-quantised", Error, None, "The number at {path} is off its unit grid: {reason}";
    IR0008 => BoundExceeded, "bound-exceeded", Error, None, "The {what} at {path} exceeds {limit}";
    IR0009 => PhaseMismatch, "phase-mismatch", Error, None, "Expected a {expected} presentation, got {found}";
    IR0011 => OriginInvalid, "origin-invalid", Error, None, "The origin at {path} does not point inside the source file";
    IR0012 => ResolvedFieldInSource, "resolved-field-in-source", Warning, None, "The resolved-only {field} was dropped: passes compute it";
    IR0013 => MediaUnavailable, "media-unavailable", Lossy, None, "The bytes of media {sha256} are {reason}";
    IR0014 => TableGridInvalid, "table-grid-invalid", Error, None, "The table grid at {path} is invalid: {reason}";
    IR0015 => TableSizeMismatch, "table-size-mismatch", Error, None, "The table frame at {path} is {frame} pt, but its {tracks} add up to {sum} pt";
    IR0101 => ThemeColorUnresolved, "theme-color-unresolved", Error, None, "The theme colour at {path} has no resolved value";
    IR0102 => ParagraphPropsIncomplete, "paragraph-props-incomplete", Error, None, "The paragraph at {path} lacks {missing}";
    IR0103 => RunPropsIncomplete, "run-props-incomplete", Error, None, "The inline at {path} lacks {missing}";
    IR0104 => FontUnbound, "font-unbound", Error, None, "Font {family} has no binding";
    IR0105 => GeometryNotLowered, "geometry-not-lowered", Error, None, "The geometry at {path} is not in its resolved form: {reason}";
    IR0106 => MediaUnplanned, "media-unplanned", Error, None, "Media {sha256} has no plan";
    IR0107 => AutofitIncomplete, "autofit-incomplete", Error, None, "The shrink at {path} lacks {missing}";
    IR0108 => FrameMissing, "frame-missing", Error, None, "The element at {path} has no frame";
    IR0109 => A11yMissing, "a11y-missing", Error, None, "The slide lacks {missing}";
    IR0110 => LineMetricsMissing, "line-metrics-missing", Error, None, "The paragraph at {path} lacks {missing}";
    IR0111 => BulletUnlabelled, "bullet-unlabelled", Error, None, "The {bullet} bullet at {path} has no label";
    IR0113 => PaintNotLowered, "paint-not-lowered", Error, None, "The {what} at {path} is not lowered";
    IR0114 => ThemeFontUnresolved, "theme-font-unresolved", Error, None, "The theme font reference at {path} is not resolved";
    IR0115 => BodyPropsIncomplete, "body-props-incomplete", Error, None, "The text body at {path} lacks {missing}";
    IR0901 => UnaccountedLoss, "unaccounted-loss", Error, None, "{feature} was lost by the {check} with no diagnostic that names it";
    IR0902 => InputUnrecognised, "input-unrecognised", Error, None, "{name} is not an input any frontend reads";
    IR0903 => InputTooLarge, "input-too-large", Error, None, "The input is {bytes} bytes; the limit is {limit}";
    IR0904 => SlidesOverLimit, "slides-over-limit", Error, None, "The presentation has {count} slides; the limit is {limit}";
    IR0905 => StageTimeout, "stage-timeout", Error, None, "{stage} stopped: {reason}";
    IR0906 => FrontendCrashed, "frontend-crashed", Error, None, "The {format} frontend failed in {stage}: {error}";
    IR0907 => PresentationFailed, "presentation-failed", Error, None, "The presentation failed in {pass}: {reason}";
    IR0908 => PassThrew, "pass-threw", Error, None, "Pass {pass} threw: {error}";
    IR0909 => InvariantViolated, "invariant-violated", Error, None, "{invariant} does not hold after {pass}: {detail}";
    IR0910 => ForeignDeclined, "foreign-declined", Decline, None, "A {type} is not represented";
    IR0911 => ForeignFallback, "foreign-fallback", Lossy, None, "A {type} is shown as its fallback";
    IR0912 => OnlyOverBudget, "only-over-budget", Error, None, "The presentation holds {mediaBytes} bytes of media, over the {budget} budget; compile it whole";
    IR0913 => OnlyUnknownSlide, "only-unknown-slide", Error, None, "{id} is not a slide of this presentation";
    IR0914 => OutputOverBudget, "output-over-budget", Error, None, "The container is left out: with what {pass} made of it, the presentation's {what} pass its budget of {limit}";
    KN1001 => LegacyFormat, "legacy-format", Error, None, "{name} was saved by Keynote 6 or older; open and save it in a current Keynote first";
    KN1002 => PasswordProtected, "password-protected", Error, None, "{name} is password-protected";
    KN1003 => NotKeynote, "not-keynote", Error, None, "{name} is not a Keynote presentation: {reason}";
    KN1004 => IndexTooLarge, "index-too-large", Error, None, "{name} is refused: {reason}";
    KN1005 => MergeUnapplied, "merge-unapplied", Warning, None, "A change record for object {id} ({file}) was not applied; the object reads as last saved whole";
    KN1006 => ExtraPayloadIgnored, "extra-payload-ignored", Info, None, "Change records after the first payload of {objects} objects were not read";
    KN1007 => IndexFileUnreadable, "index-file-unreadable", Error, None, "The index file {file} cannot be read, and the objects it holds are missing: {reason}";
    KN1008 => UnresolvedReference, "unresolved-reference", Warning, None, "Object {id} is referenced but not in the document; it is left out";
    KN1009 => ContainerUnreadable, "container-unreadable", Error, None, "The container cannot be read: {reason}";
    KN1010 => RepeatedObject, "repeated-object", Warning, None, "Object {id} is placed more than once; each container shows it once";
    KN1011 => ContainerLoweringFailed, "container-lowering-failed", Error, None, "The container failed to lower: {reason}";
    KN1012 => SlideRelisted, "slide-relisted", Warning, None, "Slide {id} is shown by more than one node of the slide tree; it shows once, in its first place";
    KN1104 => CalloutTail, "callout-tail", Lossy, None, "A callout's tail is not drawn; its body is";
    KN1105 => MovieMissing, "movie-missing", Lossy, None, "The movie (data {data}) is not in the package; its poster stands in";
    KN1107 => AudioOnlyMovie, "audio-only-movie", Lossy, Some(FeatureId::ElementAudio), "An audio-only movie is left out; it shows nothing on the slide";
    KN1108 => ObjectLinkDropped, "object-link-dropped", Lossy, Some(FeatureId::ElementLink), "An object link to {url} is not kept";
    KN1109 => CaptionDropped, "caption-dropped", Lossy, None, "A drawable's {what} is not shown";
    KN1110 => ImageDataMissing, "image-data-missing", Decline, Some(FeatureId::ElementPicture), "The image (data {data}) is not in the package";
    KN1111 => CellFormatApproximated, "cell-format-approximated", Lossy, None, "A table cell's {format} format is shown as a plain value";
    KN1112 => TableApproximated, "table-approximated", Lossy, None, "A table is approximated: {what}";
    KN1113 => VectorPreview, "vector-preview", Lossy, None, "The image is drawn from its raster preview: its SVG original (data {data}) is not shown: {reason}";
    KN1114 => InstantAlphaUnread, "instant-alpha-unread", Lossy, Some(FeatureId::ImageClip), "The image is drawn whole: its Instant Alpha outline is not read: {reason}";
    KN1201 => AttachmentDropped, "attachment-dropped", Lossy, None, "An inline {attachment} is left out of the text";
    KN1202 => AttributeTableUnsorted, "attribute-table-unsorted", Warning, None, "A text attribute table is out of order; it was read sorted";
    KN1203 => ImageBullet, "image-bullet", Lossy, Some(FeatureId::TextBulletImage), "An image bullet is shown as •";
    KN1204 => TabStopsDropped, "tab-stops-dropped", Lossy, None, "Explicit tab stops are not kept; tabs advance on the default grid";
    KN1206 => VerticalTextHorizontal, "vertical-text-horizontal", Lossy, Some(FeatureId::TextVertical), "Vertical text is laid out horizontally";
    KN1207 => TextStyleApproximated, "text-style-approximated", Lossy, None, "Text set with {style} is approximated";
    KN1208 => NumberingTypeApproximated, "numbering-type-approximated", Lossy, None, "List numbering type {type} is shown as 1. 2. 3.";
    KN1209 => LineSpacingApproximated, "line-spacing-approximated", Lossy, None, "Line spacing mode {mode} is shown as single spacing";
    KN1301 => KeynoteGradientApproximated, "keynote-gradient-approximated", Lossy, None, "A gradient is approximated: {reason}";
    KN1302 => ImageFillTint, "image-fill-tint", Lossy, Some(FeatureId::ImageRecolor), "An image fill's tint is not applied";
    KN1304 => ShadowNotDrawn, "shadow-not-drawn", Lossy, Some(FeatureId::EffectShadow), "A shadow is not drawn: {reason}";
    KN1305 => ImageFillTechnique, "image-fill-technique", Lossy, None, "An image fill set to {technique} is stretched over the box";
    KN1306 => ImageFillMissing, "image-fill-missing", Lossy, None, "The image of a fill (data {data}) is not in the package; the fill is left out";
    KN1401 => BuildUnresolved, "build-unresolved", Warning, None, "Build chunk {chunk} (build {build}) names no drawable on the slide; it is left out";
    KN1402 => BuildEffectUnmapped, "build-effect-unmapped", Info, None, "The build effect {effect} is not supported and plays as a dissolve";
    KN1403 => TransitionEffectUnmapped, "transition-effect-unmapped", Info, None, "The transition {effect} is not supported and plays as a dissolve";
    KN1404 => EffectApproximated, "effect-approximated", Info, None, "The {effect} effect plays as an approximation: {reason}";
    KN1405 => DirectionUnknown, "direction-unknown", Info, None, "The direction {direction} of the {effect} effect is unknown; it plays its default direction";
    KN1406 => DeliveryOrderForward, "delivery-order-forward", Info, None, "The delivery order {order} of build {build} plays forward";
    KN1407 => MagicMoveUnmatched, "magic-move-unmatched", Info, None, "The Magic Move into slide {slide} matches no object; every object fades";
    KN1408 => TimingCurveApproximated, "timing-curve-approximated", Info, None, "The timing curve of the {effect} effect plays as {easing}";
    KN1409 => TransitionLeftOut, "transition-left-out", Info, None, "The {effect} transition is left out: {reason}";
    KN1901 => ContainerLimit, "container-limit", Error, None, "The container is over a limit: {reason}";
    PX2001 => NotPptx, "not-pptx", Error, None, "{name} is not a PowerPoint presentation: {reason}";
    PX2002 => PartMissing, "part-missing", Error, None, "The part {part} cannot be read: {reason}";
    PX2004 => DuplicateNativeId, "duplicate-native-id", Warning, None, "The id {id} is used twice; the later object gets a suffix";
    PX2005 => AlternateContentFallback, "alternate-content-fallback", Info, None, "Content that needs {requires} is read from its fallback";
    PX2006 => StyleRefOnPlaceholder, "style-ref-on-placeholder", Info, None, "The placeholder carries a shape style, which applies where it sets nothing itself";
    PX2007 => ContainerFailed, "container-failed", Error, None, "{part} failed to lower: {reason}";
    PX2008 => ThemeIncomplete, "theme-incomplete", Warning, None, "The theme has no {slots}; Office's defaults stand in";
    PX2009 => ObjectUnsupported, "object-unsupported", Decline, Some(FeatureId::ElementOther), "A {element} is not read";
    PX2010 => NotesUnreadable, "notes-unreadable", Warning, None, "The notes part {part} cannot be read: {reason}";
    PX2011 => TableStyleUndefined, "table-style-undefined", Lossy, None, "The table style {style} is not defined in the presentation; the table shows without it";
    PX2012 => TableRightToLeft, "table-right-to-left", Lossy, None, "The right-to-left table is shown with its columns left to right";
    PX2013 => PartRelisted, "part-relisted", Warning, None, "{list} names {part} again; the part is read once, in its first place";
    PX2101 => EffectDropped, "effect-dropped", Lossy, None, "The {effect} effect is not modelled and is dropped";
    PX2103 => PatternFill, "pattern-fill", Lossy, Some(FeatureId::FillPattern), "A pattern fill is shown as its foreground colour";
    PX2104 => TileFill, "tile-fill", Lossy, Some(FeatureId::FillTile), "A tiled picture fill is stretched over the shape";
    PX2105 => GradientStroke, "gradient-stroke", Lossy, Some(FeatureId::StrokeGradient), "A gradient line is shown in its first colour";
    PX2106 => PptxGradientApproximated, "pptx-gradient-approximated", Lossy, None, "A gradient is approximated: {reason}";
    PX2107 => ColorModDropped, "color-mod-dropped", Lossy, None, "The colour modifier {modifier} is not modelled and is dropped";
    PX2108 => TableBackgroundPerCell, "table-background-per-cell", Lossy, None, "The table's {fill} background is drawn in each cell without a fill, not across the table";
    PX2111 => PptxShadowNotDrawn, "pptx-shadow-not-drawn", Lossy, Some(FeatureId::EffectShadow), "A shadow is not drawn: {reason}";
    PX2201 => TextColumns, "text-columns", Lossy, Some(FeatureId::TextColumns), "The text is set in {columns} columns, shown as one";
    PX2202 => VerticalText, "vertical-text", Lossy, Some(FeatureId::TextVertical), "The {vert} vertical text is shown horizontally";
    PX2203 => AnchorCentered, "anchor-centered", Lossy, None, "The text block is centred horizontally in its box, which is not modelled";
    PX2204 => BulletImage, "bullet-image", Lossy, Some(FeatureId::TextBulletImage), "A picture bullet is shown as a round bullet";
    PX2205 => TabStops, "tab-stops", Info, None, "Custom tab stops are not modelled; the default tab grid applies";
    PX2206 => FieldUnknown, "field-unknown", Info, None, "The {type} field shows its stored text";
    PX2207 => EastAsianFont, "east-asian-font", Info, None, "The {script} font {typeface} differs from the Latin font, which renders all the text";
    PX2208 => NumberingSchemeApproximated, "numbering-scheme-approximated", Lossy, None, "The numbering scheme {scheme} is shown as arabic numbers";
    PX2209 => BulletPropertiesWithoutType, "bullet-properties-without-type", Info, None, "Bullet properties without a bullet type are dropped: {properties}";
    PX2210 => UnderlineStyleApproximated, "underline-style-approximated", Info, None, "The {style} underline is shown as a single line";
    PX2211 => TextFillApproximated, "text-fill-approximated", Lossy, None, "A {fill} text fill is shown as one colour";
    PX2301 => BlipExternal, "blip-external", Decline, Some(FeatureId::ElementPicture), "The picture is linked from {target}, which is never fetched";
    PX2302 => ImageRecolor, "image-recolor", Lossy, Some(FeatureId::ImageRecolor), "The picture is shown without its colour effects ({effects})";
    PX2303 => FillRectInsets, "fill-rect-insets", Lossy, None, "The picture is stretched over its whole box, without its fill insets";
    PX2304 => AudioOnly, "audio-only", Lossy, Some(FeatureId::ElementAudio), "The sound is not included; its picture stays";
    PX2305 => CropDegenerate, "crop-degenerate", Decline, Some(FeatureId::ElementPicture), "The picture is cropped to nothing ({crop})";
    PX2306 => MediaMissing, "media-missing", Lossy, None, "The movie {target} is not included ({reason}); its poster stands in";
    PX2307 => ImageMissing, "image-missing", Decline, Some(FeatureId::ElementPicture), "The picture {part} cannot be read: {reason}";
    PX2308 => PictureShapeUnknown, "picture-shape-unknown", Lossy, Some(FeatureId::ImageClip), "The picture is shown as a rectangle: its shape {geometry} is not read";
    PX2309 => VideoTrimEndUnknown, "video-trim-end-unknown", Lossy, Some(FeatureId::ElementVideo), "The video plays to its end: the {end} ms its trim cuts from the end are not kept, its length unknown";
    PX2310 => SvgBlipRefused, "svg-blip-refused", Lossy, None, "The picture is drawn from its raster preview: its SVG {part} is not shown: {reason}";
    PX2401 => TimingDropped, "timing-dropped", Lossy, Some(FeatureId::Animation), "The slide animations are not included; the slide shows fully built";
    PX2402 => TransitionUnmapped, "transition-unmapped", Info, None, "The {effect} transition is not supported and plays as a dissolve";
    PX2403 => InteractiveSequenceDropped, "interactive-sequence-dropped", Lossy, Some(FeatureId::Animation), "The {effect} effect starts when a shape is clicked (an interactive sequence); it is left out";
    PX2410 => PresetUnmapped, "preset-unmapped", Info, None, "The {effect} build effect has no exact equivalent; it plays {play}";
    PX2411 => TimingApproximated, "timing-approximated", Lossy, Some(FeatureId::Animation), "The slide's {what} timing is approximated: {reason}";
    PX2412 => MotionValueUnknown, "motion-value-unknown", Info, None, "The {option} value {value} of the {effect} effect is unknown; it reads as {used}";
    PX2413 => PptxEffectApproximated, "pptx-effect-approximated", Info, None, "The {effect} effect plays as an approximation: {reason}";
    PX2501 => EmbeddedFontNotUsed, "embedded-font-not-used", Info, Some(FeatureId::FontEmbedded), "The embedded font {family} is not unwrapped; the font set renders it";
    PX2901 => ContainerOverLimit, "container-over-limit", Error, None, "{part} exceeds a limit: {reason}";
    OD9001 => NotOdp, "not-odp", Error, None, "{name} is not an OpenDocument presentation: {reason}";
    OD9002 => MimetypeMismatch, "mimetype-mismatch", Error, None, "{name} is not an OpenDocument presentation package: {reason}";
    OD9003 => PartUnreadable, "part-unreadable", Error, None, "The part {part} cannot be read: {reason}";
    OD9004 => RepeatedObjectId, "repeated-object-id", Warning, None, "The id {id} is used twice; the later object gets a suffix";
    OD9005 => LoweringFailed, "lowering-failed", Error, None, "{part} failed to lower: {reason}";
    OD9006 => StyleChainCut, "style-chain-cut", Warning, None, "The parent chain of the style {style} in {part} is cut ({reason})";
    OD9007 => MasterPageMissing, "master-page-missing", Warning, None, "The page shows the master page {name}, which the document does not define";
    OD9101 => PluginMediaUnavailable, "plugin-media-unavailable", Lossy, None, "The movie {target} is not in the package; its replacement image stands in";
    OD9102 => HatchFill, "hatch-fill", Lossy, Some(FeatureId::FillPattern), "A hatch fill is shown as one colour";
    OD9103 => GradientApproximated, "gradient-approximated", Lossy, None, "A gradient is approximated: {reason}";
    OD9104 => OdpShadowNotDrawn, "odp-shadow-not-drawn", Lossy, Some(FeatureId::EffectShadow), "A shadow is not drawn: {reason}";
    OD9105 => SoundDropped, "sound-dropped", Lossy, Some(FeatureId::ElementAudio), "The sound is not included; its replacement image stays";
    OD9106 => ShearDropped, "shear-dropped", Lossy, Some(FeatureId::FrameShear), "The skew of the transform is not modelled and is dropped";
    OD9107 => FillImageStretched, "fill-image-stretched", Lossy, None, "A picture fill set to {repeat} is stretched over the shape";
    OD9108 => MarkerApproximated, "marker-approximated", Info, None, "The line end {marker} is shown as a triangle";
    OD9109 => ShapeAdjustmentsDropped, "shape-adjustments-dropped", Info, None, "The {preset} shape keeps its default proportions: its adjustments use another coordinate system";
    OD9110 => ConnectorStraightened, "connector-straightened", Lossy, Some(FeatureId::GeometryConnector), "The routed connector is drawn as a straight line between its ends";
    OD9111 => TransformUnreadable, "transform-unreadable", Warning, None, "The transform {transform} does not parse; the shape stays at its unrotated position";
    OD9112 => TableTemplateMissing, "table-template-missing", Lossy, None, "The table template {name} is not defined; the cells show their own styles";
    OD9113 => CellBorderApproximated, "cell-border-approximated", Lossy, None, "The {style} cell border is drawn as a solid line";
    OD9201 => LineSpacingLeading, "line-spacing-leading", Lossy, Some(FeatureId::TextLineSpacing), "Line spacing by leading ({leading}) is not modelled; the lines are single-spaced";
    OD9202 => ParagraphTabStops, "paragraph-tab-stops", Info, None, "Custom tab stops are not modelled; the default tab grid applies";
    OD9203 => NumberingApproximated, "numbering-approximated", Lossy, None, "The numbering {format} is shown in the nearest scheme";
    OD9204 => TextEffectDropped, "text-effect-dropped", Lossy, None, "The text effects ({effects}) are not modelled and are dropped";
    OD9205 => CaseTransformDropped, "case-transform-dropped", Lossy, Some(FeatureId::TextCaps), "The {transform} case transform is not modelled; the text shows as written";
    OD9206 => UnderlineShownSolid, "underline-shown-solid", Info, None, "The {style} underline is shown as a solid line";
    OD9207 => VerticalTextShownHorizontally, "vertical-text-shown-horizontally", Lossy, Some(FeatureId::TextVertical), "The {mode} vertical text is shown horizontally";
    OD9208 => ColumnsShownAsOne, "columns-shown-as-one", Lossy, Some(FeatureId::TextColumns), "The text is set in {columns} columns, shown as one";
    OD9209 => PictureBullet, "picture-bullet", Lossy, Some(FeatureId::TextBulletImage), "A picture bullet is shown as a round bullet";
    OD9210 => FieldAsText, "field-as-text", Info, None, "The {field} field shows its stored text";
    OD9301 => ClipWithoutSize, "clip-without-size", Lossy, Some(FeatureId::ImageCrop), "The picture is shown uncropped: its intrinsic size is unknown";
    OD9302 => CropToNothing, "crop-to-nothing", Decline, Some(FeatureId::ElementPicture), "The picture is cropped to nothing ({crop})";
    OD9303 => ImageExternal, "image-external", Decline, Some(FeatureId::ElementPicture), "The picture is linked from {target}, which is never fetched";
    OD9304 => PictureUnreadable, "picture-unreadable", Decline, Some(FeatureId::ElementPicture), "The picture {part} cannot be read: {reason}";
    OD9305 => ColourAdjustmentsDropped, "colour-adjustments-dropped", Lossy, Some(FeatureId::ImageRecolor), "The picture is shown without its colour adjustments ({adjustments})";
    OD9306 => SvgImageRefused, "svg-image-refused", Lossy, None, "The picture is drawn from its raster preview: its SVG {part} is not shown: {reason}";
    OD9401 => AnimationDropped, "animation-dropped", Lossy, Some(FeatureId::Animation), "The slide animations are not included; the slide shows fully built";
    OD9402 => TransitionDropped, "transition-dropped", Lossy, Some(FeatureId::SlideTransition), "The slide transition is not included";
    OD9404 => OdpInteractiveSequenceDropped, "odp-interactive-sequence-dropped", Lossy, Some(FeatureId::Animation), "The {effect} effect starts when a shape is clicked (an interactive sequence); it is left out";
    OD9410 => OdpPresetUnmapped, "odp-preset-unmapped", Info, None, "The {effect} build effect has no exact equivalent; it plays {play}";
    OD9411 => OdpTimingApproximated, "odp-timing-approximated", Lossy, Some(FeatureId::Animation), "The slide's {what} timing is approximated: {reason}";
    OD9412 => OdpMotionValueUnknown, "odp-motion-value-unknown", Info, None, "The {option} value {value} of the {effect} effect is unknown; it reads as {used}";
    OD9413 => OdpEffectApproximated, "odp-effect-approximated", Info, None, "The {effect} effect plays as an approximation: {reason}";
    OD9901 => OverLimit, "over-limit", Error, None, "{part} exceeds a limit: {reason}";
    MID5101 => PlaceholderUnbound, "placeholder-unbound", Warning, None, "The {role} placeholder matches no placeholder of {parent}";
    MID5301 => ThemeMissing, "theme-missing", Warning, None, "No {what} to resolve theme references against; the defaults stand in";
    MID5401 => FontSubstituted, "font-substituted", Lossy, None, "{family} is not available; {binding} stands in";
    MID5402 => FontGeneric, "font-generic", Lossy, None, "{family} is not available and has no substitute; the generic {binding} stands in";
    MID5403 => ShrinkNotFitted, "shrink-not-fitted", Lossy, Some(FeatureId::TextAutofitShrink), "The text is set to shrink to fit, but the file stores no scale and no font file for {family} is available to measure it; it keeps its size";
    MID5404 => FontRefused, "font-refused", Lossy, None, "{family} is found but cannot be used: {reason}; {binding} stands in";
    MID5405 => VariableFontDefaultInstance, "variable-font-default-instance", Info, None, "{family} is a variable font bound at its default instance; weight {weight} is approximated";
    MID5406 => FontPlatformEmoji, "font-platform-emoji", Info, None, "{family} is an emoji font: the viewer's emoji font draws it, and no file ships";
    MID5601 => MediaUnsupportedType, "media-unsupported-type", Decline, None, "Media {sha256} is {type}, which the output cannot show";
    MID5602 => MediaTranscoded, "media-transcoded", Info, None, "Media {sha256} is {type} and is transcoded to {output}";
    MID5603 => MediaTooLarge, "media-too-large", Lossy, None, "Media {sha256} is over its {limit} limit and is left out";
    MID5604 => MediaOverBudget, "media-over-budget", Lossy, None, "Media {sha256} ({bytes} bytes) is left out to keep the presentation within its {budget}-byte budget";
    MID5605 => VideoCodecUnsupported, "video-codec-unsupported", Lossy, None, "The video {sha256} is {codec}, which browsers do not play; its poster stands in";
    MID5606 => AnimatedImageStatic, "animated-image-static", Info, None, "Media {sha256} is animated; a static render shows its first frame";
    MID5607 => VideoSafariOnly, "video-safari-only", Info, None, "The video {sha256} is {codec}, which only Safari plays";
    MID5608 => MediaDecodedByHost, "media-decoded-by-host", Info, None, "Media {sha256} is {type}, which the host's browser decoded to {output}: the deck's copy of it depends on that browser";
    MID5609 => MediaPreviewDrawn, "media-preview-drawn", Lossy, None, "Media {sha256} is drawn from its {preview} preview: the {original} original is not decoded here";
    MID5610 => MediaTranscodesOverBudget, "media-transcodes-over-budget", Lossy, None, "Media {sha256} ({pixels} pixels) is left out: transcoding it would pass the presentation's budget of {budget} pixels";
    MID5701 => TimelineTargetMissing, "timeline-target-missing", Warning, Some(FeatureId::SlideTimeline), "Build step {step} changed: its target {target} {reason}";
    MID5801 => TitleMissing, "title-missing", Info, None, "The slide has no text to title it";
    HTML6001 => LinkSchemeDropped, "link-scheme-dropped", Lossy, Some(FeatureId::TextLink), "The link to {url} is dropped: documents link only to http, https and mailto URLs";
    HTML6002 => FeatureUnsupported, "feature-unsupported", Decline, None, "The HTML backend does not print {feature}";
    HTML6003 => FeatureNotShown, "feature-not-shown", Lossy, None, "The HTML backend does not print {feature}, which a static render does not show";
    HTML6004 => FeatureNotUsed, "feature-not-used", Info, None, "The HTML backend does not use {feature}";
    HTML6005 => VideoPosterMissing, "video-poster-missing", Decline, Some(FeatureId::ElementVideo), "The video has no poster to show in its place; a marker stands in";
    HTML6006 => BorderDashApproximated, "border-dash-approximated", Lossy, Some(FeatureId::StrokeDash), "A table cell edge dashed {dash} prints as a CSS {style} border";
    HTML6010 => EffectFallback, "effect-fallback", Info, None, "The {effect} effect prints only its {fallback} fallback";
    HTML6011 => PaintNotAnimatable, "paint-not-animatable", Info, None, "The {effect} effect plays its {fallback} fallback: the element's {property} cannot be animated ({reason})";
    HTML6900 => LintError, "lint-error", Error, None, "{file} breaks the format ({rule}): {problem}";
}

impl DiagnosticCode {
    /// The emitter family, from the code's letters.
    pub fn prefix(self) -> CodePrefix {
        let code = self.code();
        let letters = code.trim_end_matches(|c: char| c.is_ascii_digit());
        CodePrefix::ALL
            .iter()
            .copied()
            .find(|prefix| prefix.as_str() == letters)
            .unwrap_or(CodePrefix::Ir)
    }

    /// The code's number (`909` for IR0909).
    pub fn number(self) -> u16 {
        let code = self.code();
        let digits = code.trim_start_matches(|c: char| c.is_ascii_uppercase());
        digits.parse().unwrap_or(0)
    }

    /// The registered code a string names, if any (`lookupCode`).
    pub fn lookup(code: &str) -> Option<DiagnosticCode> {
        Self::ALL.iter().copied().find(|entry| entry.code() == code)
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// A string that is not a registered code.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unregistered diagnostic code {0}")]
pub struct UnregisteredCode(pub String);

impl FromStr for DiagnosticCode {
    type Err = UnregisteredCode;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        Self::lookup(code).ok_or_else(|| UnregisteredCode(code.to_owned()))
    }
}

impl Serialize for DiagnosticCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.code())
    }
}

impl<'de> Deserialize<'de> for DiagnosticCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = String::deserialize(deserializer)?;
        code.parse().map_err(serde::de::Error::custom)
    }
}
