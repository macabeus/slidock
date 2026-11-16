//! The animation preset and transition tables the frontends share, mapping native effects onto the IR's effect
//! vocabulary ([`slidock_ir::effects`]):
//! - [`pptx`]: every `p:transition` child (`p14`, `p15` and `p159` included) with its options, and the build presets
//!   (`presetClass` + `presetID` + `presetSubtype`, the subtype's bit fields decoded to directions);
//! - [`odp`]: SMIL transitions (`smil:type`/`subtype`/`direction`) and the `ooo-*` build presets ([`ooo`]), whose
//!   catalog is LibreOffice's own (`sd/xml/effects.xml`, `transitions.xml`, as generated MPL-2.0 data files);
//! - [`keynote`]: Keynote's effect ids and its direction numbers.
//!
//! Every row maps to a vocabulary entry or to a declared fallback. A value a table does not know is never
//! coerced: the lookup returns `None` (the caller plays the canonical fallback and reports it), or, for an option,
//! the schema default together with an [`Unknown`] the caller reports.
//!
//! A row's `canonical_fallback` is the vocabulary's, for a format whose frontend has no fallback
//! rule of its own. It never replaces the IR's `fallback` where the frontend already prints one: PPTX and Keynote
//! keep their frontends' fallbacks (`cover` and `apple:wipe-iris` dissolve there, though their entries' canonical
//! fallbacks are push and wipe), and the motion parity rule holds the parity path to them.

pub mod keynote;
mod libreoffice;
pub mod odp;
pub mod ooo;
pub mod pptx;

use slidock_ir::effects::{
    BuildEffect, Delivery, Easing, StepFallback, StepKind, TransitionEffect, TransitionFallback,
};

pub use libreoffice::{EffectPreset, TransitionPreset};

/// The LibreOffice core commit the ODP catalogs are generated from.
pub const LIBREOFFICE_COMMIT: &str = libreoffice::COMMIT;

/// Every build preset of LibreOffice's `sd/xml/effects.xml`, in file order.
pub fn libreoffice_effects() -> &'static [EffectPreset] {
    libreoffice::effects::EFFECTS
}

/// Every transition preset of LibreOffice's `sd/xml/transitions.xml`, in file order.
pub fn libreoffice_transitions() -> &'static [TransitionPreset] {
    libreoffice::transitions::TRANSITIONS
}

/// Where a row's meaning comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Certainty {
    /// A specification or LibreOffice's own tables pin it.
    Confirmed,
    /// Read from the id or a hypothesis (Keynote's names and direction numbers); a probe deck confirms it.
    Inferred,
}

/// What a build preset plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildPlay {
    /// A vocabulary entry.
    Effect(BuildEffect),
    /// Tier 2: the keyframes the source's own behaviours compile to; the fallback when they cannot.
    Behaviors,
    /// No entry: the canonical fallback.
    Fallback,
}

/// A build preset's row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildRow {
    pub play: BuildPlay,
    /// The vocabulary's fallback for `play` (and what `Fallback` rows play), for a frontend with no fallback rule
    /// of its own; never the IR's `fallback` where the frontend prints one (see the crate doc).
    pub canonical_fallback: StepFallback,
    /// The delivery the preset implies (Keynote's `… character` ids, PowerPoint's Colored Lettering).
    pub delivery: Option<Delivery>,
    /// The effect's parameters hold defaults the source's own values replace: a spin's angle, a scale, a colour, a
    /// motion path (from the behaviours), or a Keynote direction (from its direction number).
    pub refine: bool,
    /// The entry plays a recognisable approximation of the native effect.
    pub approximate: bool,
    pub certainty: Certainty,
}

impl BuildRow {
    /// A confirmed, exact row playing `effect` in a step of `kind`.
    pub fn effect(effect: BuildEffect, kind: StepKind) -> BuildRow {
        BuildRow {
            canonical_fallback: effect.fallback(kind),
            play: BuildPlay::Effect(effect),
            delivery: None,
            refine: false,
            approximate: false,
            certainty: Certainty::Confirmed,
        }
    }

    /// A tier-2 row: the behaviours play.
    pub fn behaviors(kind: StepKind) -> BuildRow {
        BuildRow {
            play: BuildPlay::Behaviors,
            canonical_fallback: kind_fallback(kind),
            delivery: None,
            refine: false,
            approximate: false,
            certainty: Certainty::Confirmed,
        }
    }

    /// A row with no entry: the canonical fallback plays.
    pub fn fallback(kind: StepKind) -> BuildRow {
        BuildRow {
            play: BuildPlay::Fallback,
            ..BuildRow::behaviors(kind)
        }
    }

    pub(crate) fn approximate(mut self) -> BuildRow {
        self.approximate = true;
        self
    }

    pub(crate) fn refine(mut self) -> BuildRow {
        self.refine = true;
        self
    }

    pub(crate) fn inferred(mut self) -> BuildRow {
        self.certainty = Certainty::Inferred;
        self
    }

    pub(crate) fn confirmed(mut self) -> BuildRow {
        self.certainty = Certainty::Confirmed;
        self
    }

    pub(crate) fn delivered(mut self, delivery: Delivery) -> BuildRow {
        self.delivery = Some(delivery);
        self
    }

    /// The vocabulary entry, if the row has one.
    pub fn build_effect(&self) -> Option<&BuildEffect> {
        match &self.play {
            BuildPlay::Effect(effect) => Some(effect),
            BuildPlay::Behaviors | BuildPlay::Fallback => None,
        }
    }
}

/// The canonical fallback of a step of `kind` with no effect to play: entrances and exits dissolve, the rest play
/// none.
pub fn kind_fallback(kind: StepKind) -> StepFallback {
    match kind {
        StepKind::Entrance | StepKind::Exit => StepFallback::Dissolve,
        StepKind::Emphasis | StepKind::Motion | StepKind::Media => StepFallback::None,
    }
}

/// A transition's row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionRow {
    /// The vocabulary entry; `None` when the player implements no recipe for the effect.
    pub effect: Option<TransitionEffect>,
    /// The vocabulary's fallback for `effect` (or the declared one when there is none), for a frontend with no
    /// fallback rule of its own; never the IR's `fallback` where the frontend prints one (see the crate doc).
    pub canonical_fallback: TransitionFallback,
    /// The effect's parameters hold defaults the source's own values replace (a Keynote direction or colour).
    pub refine: bool,
    pub approximate: bool,
    pub certainty: Certainty,
    /// The curve the source application plays the effect on when its file stores none, where its own playback was
    /// measured; `None` leaves it to the player's default for the recipe.
    pub own_easing: Option<Easing>,
}

impl TransitionRow {
    pub fn effect(effect: TransitionEffect) -> TransitionRow {
        TransitionRow {
            canonical_fallback: effect.fallback(),
            effect: Some(effect),
            refine: false,
            approximate: false,
            certainty: Certainty::Confirmed,
            own_easing: None,
        }
    }

    pub fn fallback(fallback: TransitionFallback) -> TransitionRow {
        TransitionRow {
            effect: None,
            canonical_fallback: fallback,
            refine: false,
            approximate: false,
            certainty: Certainty::Confirmed,
            own_easing: None,
        }
    }

    pub(crate) fn approximate(mut self) -> TransitionRow {
        self.approximate = true;
        self
    }

    pub(crate) fn refine(mut self) -> TransitionRow {
        self.refine = true;
        self
    }

    pub(crate) fn confirmed(mut self) -> TransitionRow {
        self.certainty = Certainty::Confirmed;
        self
    }

    pub(crate) fn inferred(mut self) -> TransitionRow {
        self.certainty = Certainty::Inferred;
        self
    }

    pub(crate) fn plays_on(mut self, easing: Easing) -> TransitionRow {
        self.own_easing = Some(easing);
        self
    }
}

/// An option value a table does not know, read as `used` (the schema default) instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unknown {
    /// The attribute or field (`dir`, `presetSubtype`).
    pub option: &'static str,
    pub value: String,
    pub used: &'static str,
}

/// A row and the option values it read as defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lookup<R> {
    pub row: R,
    pub unknown: Vec<Unknown>,
}

impl<R> Lookup<R> {
    pub(crate) fn exact(row: R) -> Lookup<R> {
        Lookup {
            row,
            unknown: Vec::new(),
        }
    }
}
