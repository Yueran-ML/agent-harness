//! What the inserts of T10 share (Gain, Limiter, Soft Clipper, Stereo Width, De-Esser,
//! De-Hummer): the description of a parameter, the numbering of an effect's parameters, and the
//! small steps that each of them takes at every sample.
//!
//! # Parameters
//!
//! Each effect numbers its parameters by an enum of its own (`Param`, made by a macro here): the
//! id of a parameter is its place in the enum, and [`ParamInfo`] gives its path in the
//! model's effect (`audio/fx.rs` of `columbina-model`, the fields of `GainFx`, `LimiterFx` and
//! the others, in the units there), its kind, its range and its default. `on` is the switch of
//! the insert (`Insert::on` beside the effect in the model): off, the effect lets its input
//! through, delayed by its latency, and still runs underneath, so that it sounds at once as it
//! would have when it is turned on again.
//!
//! # A switch of the monitor
//!
//! A Listen ([`Kind::Monitor`]) changes what is heard, not what is rendered: it is never
//! automated ([`ParamInfo::is_automatable`]: an effect leaves its lane alone), and the graphs of
//! an export and of an analysis never set it (T4b, T6). The node itself does not know which
//! graph it is in.

/// What is left of a difference of state when a filter of an effect counts as settled: −120 dB,
/// 30 dB under the floor of `Node::settle` for what a resonance makes of it (as the EQ).
pub const SETTLE_RESIDUE: f64 = 1e-6;

/// What a parameter of an effect is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A number within its range.
    Number,
    /// On at 0.5 and above, off below.
    Switch,
    /// One of a few choices, coded 0, 1, 2 …: the nearest whole number within the range. Never
    /// automated: the model keeps a choice (De-Esser's Range, Reaction) as a name, with no lane.
    Choice,
    /// A switch of the monitor (Listen): on at 0.5 and above, never automated.
    Monitor,
}

/// A parameter of an effect: where it is in the model, what it is, its range and its default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamInfo {
    /// Its path in the model's effect, as its fields go (`sidechain.listen`); `on` for the switch
    /// of the insert.
    pub path: &'static str,
    /// What it is.
    pub kind: Kind,
    /// The lowest and the highest value it takes: 0 and 1 for a switch, 0 and the last code for a
    /// choice.
    pub range: (f32, f32),
    /// The value of a new effect.
    pub default: f32,
}

impl ParamInfo {
    /// A number from `low` to `high`, `default` for a new effect.
    #[must_use]
    pub const fn number(path: &'static str, low: f32, high: f32, default: f32) -> Self {
        ParamInfo {
            path,
            kind: Kind::Number,
            range: (low, high),
            default,
        }
    }

    /// A switch, on or off for a new effect.
    #[must_use]
    pub const fn switch(path: &'static str, on: bool) -> Self {
        ParamInfo {
            path,
            kind: Kind::Switch,
            range: (0.0, 1.0),
            default: if on { 1.0 } else { 0.0 },
        }
    }

    /// One of `count` choices, `default` for a new effect.
    #[must_use]
    pub const fn choice(path: &'static str, count: u8, default: u8) -> Self {
        ParamInfo {
            path,
            kind: Kind::Choice,
            range: (0.0, (count - 1) as f32),
            default: default as f32,
        }
    }

    /// A switch of the monitor, off for a new effect.
    #[must_use]
    pub const fn monitor(path: &'static str) -> Self {
        ParamInfo {
            path,
            kind: Kind::Monitor,
            range: (0.0, 1.0),
            default: 0.0,
        }
    }

    /// A lane of automation may drive it: all but a switch of the monitor and a choice.
    #[must_use]
    pub const fn is_automatable(&self) -> bool {
        !matches!(self.kind, Kind::Monitor | Kind::Choice)
    }

    /// It is on or off.
    #[must_use]
    pub const fn is_switch(&self) -> bool {
        matches!(self.kind, Kind::Switch | Kind::Monitor)
    }

    /// `value` as the parameter takes it: a number kept within the range, a switch as 0 or 1, a
    /// choice as the nearest code within the range. None when `value` is not a number (NaN,
    /// ±∞): what is set stays.
    #[must_use]
    pub fn clamp(&self, value: f32) -> Option<f32> {
        if !value.is_finite() {
            return None;
        }
        Some(match self.kind {
            Kind::Number => value.clamp(self.range.0, self.range.1),
            Kind::Switch | Kind::Monitor => {
                if value >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Kind::Choice => value.round().clamp(self.range.0, self.range.1),
        })
    }
}

/// Makes the enum of the parameters of an effect: each variant with its [`ParamInfo`], its id
/// its place in the enum, and `ALL`, `id`, `from_id`, `info`, `path` and `is_automatable`.
macro_rules! params {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$variant_meta:meta])* $variant:ident => $info:expr, )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        $vis enum $name {
            $( $(#[$variant_meta])* $variant, )*
        }

        impl $name {
            /// Every parameter, in the order of their ids.
            pub const ALL: &'static [$name] = &[$( $name::$variant, )*];

            /// Its id for `Node::set` and the lanes of automation: its place in `ALL`.
            #[must_use]
            #[allow(clippy::cast_possible_truncation, reason = "a few parameters")]
            pub const fn id(self) -> $crate::node::ParamId {
                $crate::node::ParamId(self as u32)
            }

            /// The parameter of `id`.
            #[must_use]
            pub fn from_id(id: $crate::node::ParamId) -> Option<$name> {
                Self::ALL.get(usize::try_from(id.0).ok()?).copied()
            }

            /// What it is: its path in the model, its kind, range and default.
            #[must_use]
            pub const fn info(self) -> $crate::fx::common::ParamInfo {
                match self {
                    $( $name::$variant => $info, )*
                }
            }

            /// Its path in the model's effect.
            #[must_use]
            pub const fn path(self) -> &'static str {
                self.info().path
            }

            /// A lane of automation may drive it.
            #[must_use]
            pub const fn is_automatable(self) -> bool {
                self.info().is_automatable()
            }
        }
    };
}

pub(crate) use params;

/// The value of `lane` at the sample `n`, if there is a lane and its value is a number: a value
/// that is not (NaN, ±∞) leaves the parameter as it is set.
#[inline]
#[must_use]
pub fn lane(lane: Option<&[f32]>, n: usize) -> Option<f32> {
    lane.map(|lane| lane[n]).filter(|value| value.is_finite())
}

/// `dry` and `wet` mixed by `weight`, 0 all dry and 1 all wet, each end to the bit.
#[inline]
#[must_use]
pub fn mix(dry: f64, wet: f64, weight: f64) -> f64 {
    if weight >= 1.0 {
        wet
    } else if weight <= 0.0 {
        dry
    } else {
        dry + (wet - dry) * weight
    }
}

/// A sample of input as the state of a filter takes it: 0 when it is not a number, so that one
/// sample of ±∞ or NaN does not stay in a feedback for ever.
#[inline]
#[must_use]
pub fn finite(x: f32) -> f64 {
    if x.is_finite() {
        f64::from(x)
    } else {
        0.0
    }
}

/// The gain of amplitude of `db` decibels, in `f64`.
#[inline]
#[must_use]
pub fn gain_of(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

/// The samples in which a state that falls by `factor` a sample falls to [`SETTLE_RESIDUE`]:
/// ⌈ln 1e-6 / ln factor⌉; 0 for a factor of 0, `u32::MAX` for one that does not fall.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to the range of u32"
)]
pub fn decay_samples(factor: f64) -> u32 {
    if factor <= 0.0 {
        0
    } else if factor >= 1.0 || !factor.is_finite() {
        u32::MAX
    } else {
        (SETTLE_RESIDUE.ln() / factor.ln())
            .ceil()
            .min(f64::from(u32::MAX)) as u32
    }
}
