//! The equalizer: of a strip, 6 bands with a gain after them (pp. 3866–3869), and of a clip, 4
//! bands (pp. 3827–3829), the same code with other bands and other types; only Earth (the
//! reference, §9.1). Every channel of the layout, the LFE too, goes through the same filters.
//!
//! # The filters
//!
//! Each band is a structure of up to two biquads of the RBJ Audio EQ Cookbook
//! ([`super::biquad`]), designed in `f64` and run in direct form I in `f64`:
//!
//! - Bell (peaking) of Q, Low and High Shelf of slope S = 1 ([`EQ_SHELF_SLOPE`]), Notch of Q;
//! - High and Low Pass of 6, 12, 18 or 24 dB an octave, a Butterworth filter of order 1 to 4 made
//!   of a section of one pole and sections of two ([`super::biquad::butterworth_q`]: 12 is Q
//!   1/√2, 18 is one pole and Q 1.0, 24 is Q 0.5412 and 1.3066).
//!
//! The frequency is kept from 20 Hz to 20 kHz, within the zone L, ML, MH or H of a middle band
//! of a strip ([`EQ_ZONES`]), and designed at most at 0.45 of the rate
//! ([`EQ_MAX_FREQUENCY_FRACTION`]). The gains are ±20 dB, the Q 0.3 to 10.3.
//!
//! # Changes while the sound plays
//!
//! - The frequency and the Q ramp over 10 ms in the logarithmic domain, the gains of the bands and
//!   the gain after them in the linear one ([`Smoothed`]); while a band's ramp runs its biquads
//!   are designed again for each sample. Each is a stable filter: the coefficients are never
//!   interpolated (the plan's worry), the parameters are. When nothing of a band moves, nothing
//!   is designed.
//! - The automated frequency, gain and Q of a band are ramped as the values set are: each value
//!   of the lane starts a ramp of 10 ms from where the parameter is, so that a lane that moves
//!   at every sample is heard through a lag of one pole of 10 ms. This is where the EQ departs
//!   from [`crate::Automation`], whose values a node takes as they are: a filter whose
//!   coefficients jump at every sample is not stable (a lane of random frequencies at every
//!   sample drives a strip to infinity), and the lag of 10 ms is not heard. The gain after the
//!   bands takes its lane as it is.
//! - What is not a number on a line (a band on or off, the EQ on or off, a type, a slope) changes
//!   the structure of the band: the new structure is put in the band's other slot, starting from
//!   the past of the sound, and the two are crossfaded over 10 ms ([`EQ_CROSSFADE`]). The past
//!   is true for the first input of the new structure (the band's input) and taken from the
//!   band's output for every section after it: the outputs of the inner sections of a structure
//!   that was not running are not known, and the band's output is the nearest there is. A
//!   change back to the structure that is going is a turn of the same crossfade; a third
//!   structure waits until the crossfade has ended.
//!
//! Why not crossfade every change (the plan's first idea): a node started in the middle (the
//! preview from the playhead, the zero test of T6) must come to sound as the node that played
//! from the start ([`Node::settle`]). Ramps and automation do: their values at each sample are
//! the same in both, or (lanes followed through a ramp) come together. A crossfade at each
//! change would start when the last one ended, which is not the same sample in the two, and
//! under automation that moves all the time the two would never meet.
//!
//! # Settling
//!
//! [`Node::settle`] is worked out from the parameters set ([`settle_for`]), every band counted
//! as on whatever its switch and the EQ's: a switch turned on by its lane costs nothing more.
//! Lanes that take a band where it rings longer than its setting (a lower frequency, a higher
//! Q, a larger gain) are the graph's to count: it calls [`settle_for`] with the worst values of
//! the lanes in its preroll (T4b).
//!
//! # To the bit
//!
//! An EQ off, a band off, and a bell or a shelf of 0 dB pass the sound to the bit, but for a
//! sample smaller than 1e-30 in size, which the filters put to 0 ([`crate::rt::flush`]).
//!
//! # Parameters
//!
//! [`ON`] and [`GAIN`] (dB) of the whole, and for each band `param(band, field)` ([`Field`]). The
//! on and off of the bands and of the whole, the frequencies, the gains and the Q can be
//! automated; the type, the slope and the zone are set only.

use super::biquad::{butterworth_q, Coefficients};
use crate::constants::{
    samples, EQ_CROSSFADE, EQ_MAX_FREQUENCY_FRACTION, EQ_SHELF_SLOPE, EQ_ZONES, SMOOTHING,
};
use crate::node::{Block, Node, ParamId, Spec, Unsupported};
use crate::rt::{flush, Domain, Smoothed};
use crate::{db_to_gain, Layout};

/// The ramps of 10 ms in which two parameters following the same lane from different values come
/// within 1e-6 of each other: each sample takes 1/N of what is left, (1 − 1/N)^(k·N) < e^(−k),
/// and the widest difference, in the domain of its ramp, is the gain's, 10 − 0.1 = 9.9 of
/// amplitude (the frequency's is ln 1000 = 6.9, the Q's ln 34.3 = 3.5): k = ⌈ln(9.9 / 1e-6)⌉ =
/// 17.
pub const LANE_SETTLE: u64 = 17;

/// The most bands: a strip's 6.
pub const MAX_BANDS: usize = 6;

/// The frequencies of a band, in Hz (the reference, §9.2–9.4).
pub const FREQUENCY_RANGE: (f32, f32) = (20.0, 20_000.0);

/// The gain of a band and the gain after the bands, in dB (pp. 3868–3869; C-E4).
pub const GAIN_RANGE_DB: (f32, f32) = (-20.0, 20.0);

/// The Q of a bell and a notch (p. 3869; C-E4).
pub const Q_RANGE: (f32, f32) = (0.3, 10.3);

/// The slopes of a pass, in dB an octave (p. 3868; C-E1).
pub const SLOPES: [u8; 4] = [6, 12, 18, 24];

/// The slope of a pass of a clip: fixed (C-E5).
pub const CLIP_SLOPE: u8 = 12;

/// What is left of a difference of state when a filter counts as settled: −120 dB, 30 dB under
/// the floor of [`Node::settle`] for what a resonance makes of it.
const SETTLE_RESIDUE: f64 = 1e-6;

/// What a band does (pp. 3827–3829, 3868–3869), in the order of the model's `EqKind`; its code
/// for [`Node::set`] is its place in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Raises or lowers what is below its frequency.
    LowShelf,
    /// Raises or lowers what is about its frequency (Peaking).
    Bell,
    /// Raises or lowers what is above its frequency.
    HighShelf,
    /// Lets through what is above its frequency.
    HighPass,
    /// Lets through what is below its frequency.
    LowPass,
    /// Takes away what is about its frequency.
    Notch,
}

impl Kind {
    /// Every kind, in the order of their codes.
    pub const ALL: [Kind; 6] = [
        Kind::LowShelf,
        Kind::Bell,
        Kind::HighShelf,
        Kind::HighPass,
        Kind::LowPass,
        Kind::Notch,
    ];

    /// Its code for [`Node::set`].
    #[must_use]
    pub fn code(self) -> f32 {
        f32::from(self as u8)
    }

    /// The kind of `code`.
    #[must_use]
    #[allow(clippy::float_cmp, reason = "the codes are whole numbers, exactly")]
    pub fn from_code(code: f32) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.code() == code)
    }

    /// It raises or lowers by a gain.
    #[must_use]
    pub fn has_gain(self) -> bool {
        matches!(self, Kind::LowShelf | Kind::Bell | Kind::HighShelf)
    }

    /// It has a Q.
    #[must_use]
    pub fn has_q(self) -> bool {
        matches!(self, Kind::Bell | Kind::Notch)
    }

    /// It has a slope.
    #[must_use]
    pub fn has_slope(self) -> bool {
        matches!(self, Kind::HighPass | Kind::LowPass)
    }
}

/// The zone of the frequency of a middle band of a strip, its L, ML, MH and H buttons (p. 3869;
/// the bounds in [`EQ_ZONES`]); its code is its place in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Zone {
    /// 20–200 Hz.
    Low,
    /// 100 Hz–1 kHz.
    MediumLow,
    /// 500 Hz–5 kHz.
    MediumHigh,
    /// 2–20 kHz.
    High,
}

impl Zone {
    /// Every zone, in the order of their codes.
    pub const ALL: [Zone; 4] = [Zone::Low, Zone::MediumLow, Zone::MediumHigh, Zone::High];

    /// Its code for [`Node::set`].
    #[must_use]
    pub fn code(self) -> f32 {
        f32::from(self as u8)
    }

    /// The zone of `code`.
    #[must_use]
    #[allow(clippy::float_cmp, reason = "the codes are whole numbers, exactly")]
    pub fn from_code(code: f32) -> Option<Zone> {
        Zone::ALL.into_iter().find(|zone| zone.code() == code)
    }

    /// Its lowest and highest frequency, in Hz.
    #[must_use]
    pub fn bounds(self) -> (f32, f32) {
        EQ_ZONES[self as usize]
    }
}

/// The EQ of a clip or of a strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Variant {
    /// 4 bands (pp. 3827–3829).
    Clip,
    /// 6 bands, the middle four in zones, the passes of four slopes (pp. 3866–3869).
    Strip,
}

impl Variant {
    /// Its bands.
    #[must_use]
    pub fn bands(self) -> usize {
        match self {
            Variant::Clip => 4,
            Variant::Strip => 6,
        }
    }

    /// The kinds that `band` may be (as the model's `Equalizer::check`).
    #[must_use]
    pub fn kinds(self, band: usize) -> &'static [Kind] {
        use Kind::{Bell, HighPass, HighShelf, LowPass, LowShelf, Notch};
        match (self, band) {
            (Variant::Strip, 0) => &[LowShelf, Bell, HighShelf, HighPass],
            (Variant::Strip, 5) => &[LowPass, LowShelf, Bell, HighShelf],
            (Variant::Clip, 0 | 3) => &[HighShelf, HighPass, Bell, Notch, LowPass, LowShelf],
            _ => &[LowShelf, Bell, Notch, HighShelf],
        }
    }

    /// `band` is in a zone: a middle band of a strip.
    #[must_use]
    pub fn has_zone(self, band: usize) -> bool {
        self == Variant::Strip && (1..=4).contains(&band)
    }

    /// The slope of its passes can be chosen: a strip's.
    #[must_use]
    pub fn has_slope(self) -> bool {
        self == Variant::Strip
    }
}

/// A band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    /// It is on.
    pub on: bool,
    /// What it does.
    pub kind: Kind,
    /// Its frequency, in Hz.
    pub freq: f32,
    /// Its gain, in dB: of a bell and a shelf.
    pub gain_db: f32,
    /// Its Q: of a bell and a notch.
    pub q: f32,
    /// Its slope, in dB an octave: of a pass.
    pub slope: u8,
    /// Its zone: of a middle band of a strip.
    pub zone: Option<Zone>,
}

impl Band {
    /// A band of no gain, Q 1 and 12 dB an octave.
    #[must_use]
    pub const fn new(on: bool, kind: Kind, freq: f32, zone: Option<Zone>) -> Band {
        Band {
            on,
            kind,
            freq,
            gain_db: 0.0,
            q: 1.0,
            slope: 12,
            zone,
        }
    }

    /// The band as `band` of `variant` takes it: a kind it may not be becomes `fallback`'s, the
    /// numbers are kept in their ranges (a number that is not finite becomes `fallback`'s), the
    /// slope goes to the nearest of [`SLOPES`] (12 on a clip), and the zone is there only on a
    /// middle band of a strip.
    #[must_use]
    pub fn sanitized(self, variant: Variant, band: usize, fallback: &Band) -> Band {
        let kind = if variant.kinds(band).contains(&self.kind) {
            self.kind
        } else {
            fallback.kind
        };
        let zone = if variant.has_zone(band) {
            self.zone.or(fallback.zone).or(Some(Zone::ALL[band - 1]))
        } else {
            None
        };
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        Band {
            on: self.on,
            kind,
            freq: clamp_freq(finite(self.freq, fallback.freq), zone),
            gain_db: clamp_gain(finite(self.gain_db, fallback.gain_db)),
            q: clamp_q(finite(self.q, fallback.q)),
            slope: if variant.has_slope() {
                nearest_slope(f32::from(self.slope))
            } else {
                CLIP_SLOPE
            },
            zone,
        }
    }
}

/// The settings of an EQ: what the model keeps (`Equalizer` of `columbina-model`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Of a clip or of a strip.
    pub variant: Variant,
    /// It is on.
    pub on: bool,
    /// The gain after the bands, in dB.
    pub gain_db: f32,
    /// The bands: the first [`Variant::bands`] of them.
    pub bands: [Band; MAX_BANDS],
}

impl Settings {
    /// The EQ of a new strip: off; High Pass at 80 Hz and Low Pass at 16 kHz, off; bells of no
    /// gain at 100 Hz, 500 Hz, 2 kHz and 8 kHz in L, ML, MH and H (C-E1, C-E2).
    #[must_use]
    pub fn strip() -> Settings {
        Settings {
            variant: Variant::Strip,
            on: false,
            gain_db: 0.0,
            bands: [
                Band::new(false, Kind::HighPass, 80.0, None),
                Band::new(true, Kind::Bell, 100.0, Some(Zone::Low)),
                Band::new(true, Kind::Bell, 500.0, Some(Zone::MediumLow)),
                Band::new(true, Kind::Bell, 2_000.0, Some(Zone::MediumHigh)),
                Band::new(true, Kind::Bell, 8_000.0, Some(Zone::High)),
                Band::new(false, Kind::LowPass, 16_000.0, None),
            ],
        }
    }

    /// The EQ of a new clip: off; Hi-Pass at 80 Hz and Lo-Pass at 16 kHz, off; bells of no gain
    /// at 300 Hz and 3 kHz (C-E5). The last two bands are not used.
    #[must_use]
    pub fn clip() -> Settings {
        let unused = Band::new(false, Kind::Bell, 1_000.0, None);
        Settings {
            variant: Variant::Clip,
            on: false,
            gain_db: 0.0,
            bands: [
                Band::new(false, Kind::HighPass, 80.0, None),
                Band::new(true, Kind::Bell, 300.0, None),
                Band::new(true, Kind::Bell, 3_000.0, None),
                Band::new(false, Kind::LowPass, 16_000.0, None),
                unused,
                unused,
            ],
        }
    }

    /// The new EQ of `variant`.
    #[must_use]
    pub fn of(variant: Variant) -> Settings {
        match variant {
            Variant::Clip => Settings::clip(),
            Variant::Strip => Settings::strip(),
        }
    }

    /// The bands used.
    #[must_use]
    pub fn bands(&self) -> &[Band] {
        &self.bands[..self.variant.bands()]
    }

    /// The settings as the EQ takes them ([`Band::sanitized`], the gain within ±20 dB), what
    /// cannot be taken from the new EQ of the variant.
    #[must_use]
    pub fn sanitized(&self) -> Settings {
        let new = Settings::of(self.variant);
        let mut settings = *self;
        settings.gain_db = clamp_gain(if self.gain_db.is_finite() {
            self.gain_db
        } else {
            0.0
        });
        for (band, (settings, new)) in settings.bands.iter_mut().zip(&new.bands).enumerate() {
            *settings = if band < self.variant.bands() {
                settings.sanitized(self.variant, band, new)
            } else {
                *new
            };
        }
        settings
    }

    /// Every parameter and its value, as [`Node::set`] takes them: an EQ of the same variant set
    /// so has these settings. Allocates: off the thread of the mix.
    #[must_use]
    pub fn params(&self) -> Vec<(ParamId, f32)> {
        let mut params = vec![(ON, f32::from(u8::from(self.on))), (GAIN, self.gain_db)]; // rt-ok: off the thread of the mix
        for (index, band) in self.bands().iter().enumerate() {
            params.extend([
                (param(index, Field::On), f32::from(u8::from(band.on))),
                (param(index, Field::Kind), band.kind.code()),
                (
                    param(index, Field::Zone),
                    band.zone.map_or(-1.0, Zone::code),
                ),
                (param(index, Field::Freq), band.freq),
                (param(index, Field::Gain), band.gain_db),
                (param(index, Field::Q), band.q),
                (param(index, Field::Slope), f32::from(band.slope)),
            ]);
        }
        params
    }
}

/// The EQ on (1) or off (0): off, the bands pass the sound to the bit (but for samples below
/// 1e-30) and the gain is 0 dB.
pub const ON: ParamId = ParamId(0);

/// The gain after the bands, in dB.
pub const GAIN: ParamId = ParamId(1);

/// A setting of a band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// On (1) or off (0).
    On,
    /// The [`Kind::code`].
    Kind,
    /// The frequency, in Hz.
    Freq,
    /// The gain, in dB.
    Gain,
    /// The Q.
    Q,
    /// The slope, in dB an octave.
    Slope,
    /// The [`Zone::code`].
    Zone,
}

impl Field {
    const ALL: [Field; 7] = [
        Field::On,
        Field::Kind,
        Field::Freq,
        Field::Gain,
        Field::Q,
        Field::Slope,
        Field::Zone,
    ];
}

/// The first number of the parameters of the bands, and how many each band has.
const BAND_PARAMS: (u32, u32) = (16, 8);

/// The parameter of `field` of `band` (from 0): 16 + 8·band + the field's place.
#[must_use]
#[allow(clippy::cast_possible_truncation, reason = "a band of 6 at most")]
pub fn param(band: usize, field: Field) -> ParamId {
    ParamId(BAND_PARAMS.0 + BAND_PARAMS.1 * band as u32 + field as u32)
}

/// The band and the field of `param`.
fn field_of(param: ParamId) -> Option<(usize, Field)> {
    let offset = param.0.checked_sub(BAND_PARAMS.0)?;
    let band = usize::try_from(offset / BAND_PARAMS.1).ok()?;
    let field = Field::ALL.get(usize::try_from(offset % BAND_PARAMS.1).ok()?)?;
    (band < MAX_BANDS).then_some((band, *field))
}

fn clamp_freq(freq: f32, zone: Option<Zone>) -> f32 {
    let freq = freq.clamp(FREQUENCY_RANGE.0, FREQUENCY_RANGE.1);
    zone.map_or(freq, |zone| {
        let (low, high) = zone.bounds();
        freq.clamp(low, high)
    })
}

fn clamp_gain(db: f32) -> f32 {
    db.clamp(GAIN_RANGE_DB.0, GAIN_RANGE_DB.1)
}

fn clamp_q(q: f32) -> f32 {
    q.clamp(Q_RANGE.0, Q_RANGE.1)
}

/// The slope of [`SLOPES`] nearest `slope`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 1 to 4"
)]
fn nearest_slope(slope: f32) -> u8 {
    let order = (slope / 6.0).round().clamp(1.0, 4.0) as u8;
    order * 6
}

/// What a band is made of: no filter, or one or two biquads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Structure {
    Off,
    Bell,
    LowShelf,
    HighShelf,
    Notch,
    /// Of order 1 to 4: 6 to 24 dB an octave.
    HighPass(u8),
    LowPass(u8),
}

impl Structure {
    fn of(on: bool, kind: Kind, slope: u8) -> Structure {
        if !on {
            return Structure::Off;
        }
        let order = slope / 6;
        match kind {
            Kind::Bell => Structure::Bell,
            Kind::LowShelf => Structure::LowShelf,
            Kind::HighShelf => Structure::HighShelf,
            Kind::Notch => Structure::Notch,
            Kind::HighPass => Structure::HighPass(order),
            Kind::LowPass => Structure::LowPass(order),
        }
    }

    /// Its biquads at `rate` for `freq` Hz, `gain` of amplitude and `q`, and how many there are.
    fn design(self, rate: f64, freq: f64, gain: f64, q: f64) -> ([Coefficients; 2], usize) {
        let identity = Coefficients::IDENTITY;
        let freq = freq.min(EQ_MAX_FREQUENCY_FRACTION * rate);
        let shelf_slope = f64::from(EQ_SHELF_SLOPE);
        let one = |section| ([section, identity], 1);
        match self {
            Structure::Off => ([identity; 2], 0),
            Structure::Bell => one(Coefficients::peaking(rate, freq, q, gain)),
            Structure::LowShelf => one(Coefficients::low_shelf(rate, freq, gain, shelf_slope)),
            Structure::HighShelf => one(Coefficients::high_shelf(rate, freq, gain, shelf_slope)),
            Structure::Notch => one(Coefficients::notch(rate, freq, q)),
            Structure::HighPass(order) => pass(
                order,
                |q| Coefficients::high_pass(rate, freq, q),
                Coefficients::high_pass_first(rate, freq),
            ),
            Structure::LowPass(order) => pass(
                order,
                |q| Coefficients::low_pass(rate, freq, q),
                Coefficients::low_pass_first(rate, freq),
            ),
        }
    }
}

/// A Butterworth pass of `order`: the section of one pole `first` for an odd order, and the
/// sections of two poles that `second` makes of their Q.
fn pass(
    order: u8,
    second: impl Fn(f64) -> Coefficients,
    first: Coefficients,
) -> ([Coefficients; 2], usize) {
    let identity = Coefficients::IDENTITY;
    match (order, butterworth_q(order)) {
        (1, _) => ([first, identity], 1),
        (3, [q]) => ([first, second(*q)], 2),
        (_, [q]) => ([second(*q), identity], 1),
        (_, [low, high]) => ([second(*low), second(*high)], 2),
        _ => ([identity; 2], 0),
    }
}

/// A ramp from 0 to 1 or back, of a fixed length: the crossfade of two structures, the EQ on
/// and off. Each value is worked out from the samples since the change, as in [`Smoothed`].
#[derive(Clone, Copy, Debug)]
struct Ramp {
    from: f64,
    to: f64,
    step: u32,
    len: u32,
}

impl Ramp {
    fn at(value: f64) -> Ramp {
        Ramp {
            from: value,
            to: value,
            step: 0,
            len: 0,
        }
    }

    fn prepare(&mut self, len: u32) {
        self.len = len;
        self.finish();
    }

    fn finish(&mut self) {
        self.from = self.to;
        self.step = self.len;
    }

    fn is_ramping(&self) -> bool {
        self.step < self.len
    }

    fn current(&self) -> f64 {
        if self.is_ramping() {
            self.from + (self.to - self.from) * f64::from(self.step) / f64::from(self.len)
        } else {
            self.to
        }
    }

    /// Ramps from where it is to `to`; the same target changes nothing.
    #[allow(clippy::float_cmp, reason = "the targets are 0 and 1, exactly")]
    fn set(&mut self, to: f64) {
        if to == self.to {
            return;
        }
        self.from = self.current();
        self.to = to;
        self.step = 0;
        if self.len == 0 {
            self.finish();
        }
    }

    fn next_value(&mut self) -> f64 {
        if self.step + 1 < self.len {
            self.step += 1;
            self.current()
        } else {
            self.step = self.len;
            self.to
        }
    }

    /// The slot it goes to: 0 or 1.
    fn target(&self) -> usize {
        usize::from(self.to > 0.5)
    }
}

/// A band as it runs: its settings, its two slots of structures and the crossfade between them,
/// and the biquads designed for its parameters now.
#[derive(Clone, Copy, Debug)]
struct Live {
    on: bool,
    kind: Kind,
    slope: u8,
    zone: Option<Zone>,
    /// The frequency, the gain (of amplitude) and the Q as set.
    set: [f32; 3],
    /// The frequency, the gain and the Q heard: ramping to the values set, or to the values of
    /// their lanes where they are automated.
    freq: Smoothed,
    gain: Smoothed,
    q: Smoothed,
    /// The structures of the two slots; the slot that is not sounding holds none.
    slots: [Option<Structure>; 2],
    weight: Ramp,
    coefficients: [[Coefficients; 2]; 2],
    sections: [usize; 2],
    /// The frequency, gain and Q (their bits) the slots are designed for; none when they must be
    /// designed again.
    designed: Option<[u32; 3]>,
}

impl Live {
    fn new(band: &Band, eq_on: bool) -> Live {
        Live {
            on: band.on,
            kind: band.kind,
            slope: band.slope,
            zone: band.zone,
            set: [band.freq, db_to_gain(band.gain_db), band.q],
            freq: Smoothed::new(band.freq, Domain::Log),
            gain: Smoothed::new(db_to_gain(band.gain_db), Domain::Linear),
            q: Smoothed::new(band.q, Domain::Log),
            slots: [
                Some(Structure::of(eq_on && band.on, band.kind, band.slope)),
                None,
            ],
            weight: Ramp::at(0.0),
            coefficients: [[Coefficients::IDENTITY; 2]; 2],
            sections: [0; 2],
            designed: None,
        }
    }

    /// Designs the slots for `freq`, `gain` and `q` at `rate`.
    fn design(&mut self, rate: f64, freq: f32, gain: f32, q: f32) {
        for slot in 0..2 {
            if let Some(structure) = self.slots[slot] {
                let (coefficients, sections) =
                    structure.design(rate, f64::from(freq), f64::from(gain), f64::from(q));
                self.coefficients[slot] = coefficients;
                self.sections[slot] = sections;
            }
        }
        self.designed = Some([freq.to_bits(), gain.to_bits(), q.to_bits()]);
    }

    /// `x` through the structure of `slot`, from the band's last two inputs.
    #[inline]
    fn run(&self, slot: usize, past: &mut Past, x: f64) -> f64 {
        let mut value = x;
        let mut input = past.input;
        for (section, output) in self.coefficients[slot][..self.sections[slot]]
            .iter()
            .zip(&mut past.slots[slot])
        {
            let before = *output;
            value = section.tick(value, input, output);
            input = before;
        }
        value
    }

    /// `x` through the band: through the slot that sounds, or the two mixed by `weight`.
    #[inline]
    fn tick(&self, past: &mut Past, x: f64, mix: Mix) -> f64 {
        let y = match mix {
            Mix::Rest(slot) => self.run(slot, past, x),
            Mix::Cross(weight) => {
                let a = self.run(0, past, x);
                let b = self.run(1, past, x);
                flush(a + (b - a) * weight)
            }
        };
        past.input = [x, past.input[0]];
        past.output = [y, past.output[0]];
        y
    }
}

/// How a band sounds at a sample: one slot, or the two mixed, the second by the weight.
#[derive(Clone, Copy, Debug)]
enum Mix {
    Rest(usize),
    Cross(f64),
}

/// The past of a band on a channel: its last two samples in and out, and the last two outputs
/// of each section of each slot.
#[derive(Clone, Copy, Debug, Default)]
struct Past {
    input: [f64; 2],
    output: [f64; 2],
    slots: [[[f64; 2]; 2]; 2],
}

/// The equalizer of a clip or of a strip, a [`Node`] of no latency.
#[derive(Clone, Debug)]
pub struct Eq {
    variant: Variant,
    rate: u32,
    on: bool,
    /// The EQ on, from 0 to 1: the part of the gain after the bands that is heard.
    wet: Ramp,
    /// The gain after the bands, of amplitude.
    gain: Smoothed,
    bands: [Live; MAX_BANDS],
    /// For each channel, the past of each band.
    past: Vec<[Past; MAX_BANDS]>,
    crossfade: u32,
}

impl Eq {
    /// An EQ of `settings` ([`Settings::sanitized`]), at once with no ramp.
    #[must_use]
    pub fn new(settings: &Settings) -> Eq {
        let settings = settings.sanitized();
        let bands = std::array::from_fn(|band| Live::new(&settings.bands[band], settings.on));
        Eq {
            variant: settings.variant,
            rate: 0,
            on: settings.on,
            wet: Ramp::at(if settings.on { 1.0 } else { 0.0 }),
            gain: Smoothed::new(db_to_gain(settings.gain_db), Domain::Linear),
            bands,
            past: Vec::new(), // rt-ok: empty, before the graph is built
            crossfade: 0,
        }
    }

    /// The EQ of a new strip.
    #[must_use]
    pub fn strip() -> Eq {
        Eq::new(&Settings::strip())
    }

    /// The EQ of a new clip.
    #[must_use]
    pub fn clip() -> Eq {
        Eq::new(&Settings::clip())
    }

    /// Of a clip or of a strip.
    #[must_use]
    pub fn variant(&self) -> Variant {
        self.variant
    }

    fn rate(&self) -> f64 {
        f64::from(self.rate)
    }

    /// The frequency (Hz), the gain (dB) and the Q that band `band` has at the last sample
    /// processed: where its ramps are.
    #[must_use]
    pub fn heard(&self, band: usize) -> (f32, f32, f32) {
        let band = &self.bands[band];
        (
            band.freq.current(),
            crate::gain_to_db(band.gain.current()),
            band.q.current(),
        )
    }
}

/// The samples after which an EQ of `settings` at `rate`, started late, sounds within −90 dBFS
/// of one that played from earlier ([`Node::settle`]): two crossfades; [`LANE_SETTLE`] ramps, in
/// which values following the same lane come together; and for each biquad of each band, every
/// band counted as on whatever its switch and the EQ's, the samples in which a difference of
/// state falls to 1e-6 by its slowest pole ([`Coefficients::decay`]), added up along the bands.
/// The EQ reports it for its parameters as set; the graph calls it with the worst values of the
/// lanes in its preroll (the lowest frequency, the highest Q, the largest gain in size).
#[must_use]
pub fn settle_for(settings: &Settings, rate: u32) -> u32 {
    let settings = settings.sanitized();
    settle_of(
        settings.variant,
        rate,
        settings.bands.iter().map(|band| {
            (
                band.kind,
                band.slope,
                [band.freq, db_to_gain(band.gain_db), band.q],
            )
        }),
    )
}

/// [`settle_for`] the kind, slope, and frequency, gain of amplitude and Q of each band.
fn settle_of(
    variant: Variant,
    rate: u32,
    bands: impl Iterator<Item = (Kind, u8, [f32; 3])>,
) -> u32 {
    if rate == 0 {
        return 0;
    }
    let crossfade = u64::try_from(samples(EQ_CROSSFADE, rate)).unwrap_or(u64::MAX);
    let ramp = u64::try_from(samples(SMOOTHING, rate)).unwrap_or(u64::MAX);
    let mut total = 2 * crossfade + LANE_SETTLE * ramp;
    for (kind, slope, [freq, gain, q]) in bands.take(variant.bands()) {
        let (coefficients, sections) = Structure::of(true, kind, slope).design(
            f64::from(rate),
            f64::from(freq),
            f64::from(gain),
            f64::from(q),
        );
        for section in &coefficients[..sections] {
            total = total.saturating_add(u64::from(section.decay(SETTLE_RESIDUE)));
        }
    }
    u32::try_from(total).unwrap_or(u32::MAX)
}

/// The values of a parameter through a block: automated, or none.
type Lane<'a> = Option<&'a [f32]>;

/// The automated parameters of a band: on, frequency, gain, Q.
type BandLanes<'a> = [Lane<'a>; 4];

/// The value of `lane` at sample `n`, if there is a lane and its value is a number: a value that
/// is not (NaN, ±∞) leaves the parameter as it is set.
#[inline]
fn value(lane: Lane, n: usize) -> Option<f32> {
    lane.map(|lane| lane[n]).filter(|value| value.is_finite())
}

impl Eq {
    /// The structure that band `index` should have, the EQ on or not, its on given.
    fn structure(&self, index: usize, eq_on: bool, band_on: bool) -> Structure {
        let band = &self.bands[index];
        Structure::of(eq_on && band_on, band.kind, band.slope)
    }

    /// Moves band `index` on by a sample, `n` of the block: its structure, its crossfade, its
    /// ramps and its design. How it sounds at this sample.
    fn advance(&mut self, index: usize, n: usize, eq_on: bool, lanes: &BandLanes) -> Mix {
        let on = value(lanes[0], n).map_or(self.bands[index].on, |on| on >= 0.5);
        let target = self.structure(index, eq_on, on);
        let rate = self.rate();
        let band = &mut self.bands[index];
        let current = band.weight.target();
        if band.slots[current] != Some(target) {
            let other = 1 - current;
            if band.slots[other] == Some(target) {
                band.weight.set(if other == 1 { 1.0 } else { 0.0 });
            } else if !band.weight.is_ramping() {
                band.slots[other] = Some(target);
                band.weight.set(if other == 1 { 1.0 } else { 0.0 });
                band.designed = None;
                for past in &mut self.past {
                    let past = &mut past[index];
                    past.slots[other] = [past.output; 2];
                }
            }
        }
        let weight = band.weight.next_value();
        let mix = if band.weight.is_ramping() {
            Mix::Cross(weight)
        } else {
            let rest = band.weight.target();
            band.slots[1 - rest] = None;
            Mix::Rest(rest)
        };
        // Each new value, set or automated, starts a ramp of 10 ms from where the parameter is:
        // a lane that moves at every sample is heard through a lag of one pole of 10 ms, so that
        // no automation, however rough, moves the coefficients faster than a ramp does.
        let [freq, gain, q] = band.set;
        band.freq
            .set(value(lanes[1], n).map_or(freq, |freq| clamp_freq(freq, band.zone)));
        band.gain
            .set(value(lanes[2], n).map_or(gain, |db| db_to_gain(clamp_gain(db))));
        band.q.set(value(lanes[3], n).map_or(q, clamp_q));
        let moving = band.freq.is_ramping() || band.gain.is_ramping() || band.q.is_ramping();
        if moving || band.designed.is_none() {
            let freq = band.freq.next_value();
            let gain = band.gain.next_value();
            let q = band.q.next_value();
            if band.designed != Some([freq.to_bits(), gain.to_bits(), q.to_bits()]) {
                band.design(rate, freq, gain, q);
            }
        }
        mix
    }
}

impl Node for Eq {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        let unsupported = spec.layout != spec.output
            || spec.layout == Layout::Empty
            || spec.sidechain > 0
            || spec.rate == 0;
        if unsupported {
            return Err(Unsupported::from(spec));
        }
        self.rate = spec.rate;
        self.crossfade = u32::try_from(samples(EQ_CROSSFADE, spec.rate)).unwrap_or(u32::MAX);
        self.wet.prepare(self.crossfade);
        self.gain.prepare(spec.rate);
        for band in &mut self.bands {
            band.weight.prepare(self.crossfade);
            band.freq.prepare(spec.rate);
            band.gain.prepare(spec.rate);
            band.q.prepare(spec.rate);
        }
        self.past = vec![[Past::default(); MAX_BANDS]; spec.inputs()]; // rt-ok: when the graph is built
        self.reset();
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, reason = "the samples are f32")]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let count = self.variant.bands();
        let automation = &ctx.automation;
        let eq_lane = automation.get(ON);
        let gain_lane = automation.get(GAIN);
        let mut lanes: [BandLanes; MAX_BANDS] = [[None; 4]; MAX_BANDS];
        if !automation.is_empty() {
            for (index, lanes) in lanes.iter_mut().enumerate().take(count) {
                *lanes = [Field::On, Field::Freq, Field::Gain, Field::Q]
                    .map(|field| automation.get(param(index, field)));
            }
        }
        let mut mixes = [Mix::Rest(0); MAX_BANDS];
        for n in 0..ctx.frames {
            let eq_on = value(eq_lane, n).map_or(self.on, |on| on >= 0.5);
            self.wet.set(if eq_on { 1.0 } else { 0.0 });
            let wet = self.wet.next_value();
            let gain = self.gain.next_value();
            let gain = value(gain_lane, n).map_or(gain, |db| db_to_gain(clamp_gain(db)));
            let after = 1.0 + wet * (f64::from(gain) - 1.0);
            for (index, mix) in mixes.iter_mut().enumerate().take(count) {
                *mix = self.advance(index, n, eq_on, &lanes[index]);
            }
            for ((output, input), past) in output.iter_mut().zip(input).zip(&mut self.past) {
                let mut x = f64::from(input[n]);
                for ((band, past), mix) in self.bands[..count].iter().zip(past).zip(&mixes) {
                    x = band.tick(past, x, *mix);
                }
                output[n] = (x * after) as f32;
            }
        }
    }

    fn latency(&self) -> u32 {
        0
    }

    /// [`settle_for`] the parameters set.
    fn settle(&self) -> u32 {
        settle_of(
            self.variant,
            self.rate,
            self.bands
                .iter()
                .map(|band| (band.kind, band.slope, band.set)),
        )
    }

    fn reset(&mut self) {
        self.wet.set(if self.on { 1.0 } else { 0.0 });
        self.wet.finish();
        self.gain.finish();
        for index in 0..MAX_BANDS {
            let target = self.structure(index, self.on, self.bands[index].on);
            let band = &mut self.bands[index];
            band.freq.jump(band.set[0]);
            band.gain.jump(band.set[1]);
            band.q.jump(band.set[2]);
            band.weight.finish();
            let rest = band.weight.target();
            band.slots[rest] = Some(target);
            band.slots[1 - rest] = None;
            band.designed = None;
        }
        for past in &mut self.past {
            *past = [Past::default(); MAX_BANDS];
        }
    }

    fn set(&mut self, param: ParamId, value: f32) {
        if !value.is_finite() {
            return;
        }
        if param == ON {
            self.on = value >= 0.5;
            return;
        }
        if param == GAIN {
            self.gain.set(db_to_gain(clamp_gain(value)));
            return;
        }
        let Some((index, field)) = field_of(param) else {
            return;
        };
        if index >= self.variant.bands() {
            return;
        }
        let variant = self.variant;
        let band = &mut self.bands[index];
        match field {
            Field::On => band.on = value >= 0.5,
            Field::Kind => {
                if let Some(kind) =
                    Kind::from_code(value).filter(|kind| variant.kinds(index).contains(kind))
                {
                    band.kind = kind;
                }
            }
            Field::Freq => band.set[0] = clamp_freq(value, band.zone),
            Field::Gain => band.set[1] = db_to_gain(clamp_gain(value)),
            Field::Q => band.set[2] = clamp_q(value),
            Field::Slope => {
                if variant.has_slope() {
                    band.slope = nearest_slope(value);
                }
            }
            Field::Zone => {
                if let Some(zone) = Zone::from_code(value).filter(|_| variant.has_zone(index)) {
                    band.zone = Some(zone);
                    band.set[0] = clamp_freq(band.set[0], band.zone);
                }
            }
        }
    }
}

/// The response of an EQ of `settings` at `rate`, in dB, at each of `freqs` (Hz): the curve of the
/// window. The bands that are on and the gain after them; 0 dB everywhere when the EQ is off;
/// at least [`crate::constants::SILENCE_DB`] (a notch is −∞ at its frequency). Allocates: off the
/// thread of the mix.
#[must_use]
#[allow(clippy::cast_possible_truncation, reason = "decibels")]
pub fn response(settings: &Settings, rate: u32, freqs: &[f32]) -> Vec<f32> {
    let settings = settings.sanitized();
    let rate = f64::from(rate);
    let designs: [([Coefficients; 2], usize); MAX_BANDS] = std::array::from_fn(|index| {
        let band = &settings.bands[index];
        let used = index < settings.variant.bands();
        Structure::of(used && band.on, band.kind, band.slope).design(
            rate,
            f64::from(band.freq),
            f64::from(db_to_gain(band.gain_db)),
            f64::from(band.q),
        )
    });
    let floor = f64::from(crate::constants::SILENCE_DB);
    freqs
        .iter()
        .map(|&freq| {
            if !settings.on {
                return 0.0;
            }
            let mut db = f64::from(settings.gain_db);
            for (coefficients, sections) in &designs {
                for section in &coefficients[..*sections] {
                    db += 20.0 * section.magnitude(rate, f64::from(freq)).log10();
                }
            }
            db.max(floor) as f32
        })
        .collect() // rt-ok: the curve of the window, off the thread of the mix
}
