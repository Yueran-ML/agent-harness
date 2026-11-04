//! Settings of sound that change over time: the keyframes of a clip (its volume and pan, as the
//! inspector and the volume line of `DaVinci Resolve` key them, Reference Manual 20, pp. 970–972,
//! 3837–3840) and the lanes of the automation of a track, a bus or a VCA (pp. 3893–3900).
//!
//! The two are kept apart, as `DaVinci` keeps them (p. 3897): the keys of a clip are at times of
//! its source, follow the clip and may ease; the points of a lane are at times of the timeline,
//! are joined by straight lines ("not Bezier") and never ease.

use std::collections::BTreeMap;

use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ops::LARGEST;
use crate::time::Time;

/// How the value of an [`AudioParam`] comes into a key and leaves it, as the right-click menu of
/// a keyframe of `DaVinci` sets it (pp. 972, 3838): on straight lines, or easing into it or out
/// of it.
///
/// The segment between two keys is a straight line where neither eases on it. Where one does, it
/// is the cubic of Hermite through the two values whose slope is 0 at the end that eases and that
/// of the straight line at the other: a key that eases in is reached with a slope of 0, and one
/// that eases out is left with a slope of 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyInterp {
    /// On straight lines, into the key and out of it.
    #[default]
    Linear,
    /// The value slows down into the key: the segment before it reaches it with a slope of 0.
    EaseIn,
    /// The value starts slowly out of the key: the segment after it leaves it with a slope of 0.
    EaseOut,
}

/// A keyframe of an [`AudioParam`]: a value at a time of the source of its clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioKey {
    /// The time that it is at: from the start of the media of its clip, as the place where the
    /// clip begins in its media is counted, so that it stays on the sound that it was set on
    /// when the clip is trimmed, slipped or moved. For a clip of no media, from its start.
    pub at: Time,
    /// The value there.
    pub value: f64,
    /// How the value comes into it and leaves it. It may be left out, as linear.
    #[serde(default)]
    pub interp: KeyInterp,
}

/// A setting of a clip's sound that may change over its source: one value, or keyframes.
///
/// A fixed value is written as a number, as the gain and pan of a clip were before keyframes,
/// so that `"gain_db": -6` is read and written as it was. Keyframes are written as
/// `{"keys": [{"at": {"ticks": 0, "scale": 48000}, "value": -6, "interp": "linear"}, ...]}`:
/// at least one, at times in order with at most one at a time. Before the first key the value is
/// that of the first, and after the last that of the last.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioParam {
    /// One value, over all of the clip.
    Fixed(f64),
    /// Keyframes.
    Keys(Vec<AudioKey>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeys {
    keys: Vec<AudioKey>,
}

impl Serialize for AudioParam {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Fixed(value) => serializer.serialize_f64(*value),
            Self::Keys(keys) => {
                let mut object = serializer.serialize_struct("AudioParam", 1)?;
                object.serialize_field("keys", keys)?;
                object.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for AudioParam {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Number(number) => number
                .as_f64()
                .map(Self::Fixed)
                .ok_or_else(|| serde::de::Error::custom("a setting is not a finite number")),
            serde_json::Value::Object(_) => serde_json::from_value::<RawKeys>(value)
                .map(|raw| Self::Keys(raw.keys))
                .map_err(serde::de::Error::custom),
            _ => Err(serde::de::Error::custom(
                "a setting of sound is a number or an object of `keys`",
            )),
        }
    }
}

impl From<f64> for AudioParam {
    fn from(value: f64) -> Self {
        Self::Fixed(value)
    }
}

impl Default for AudioParam {
    fn default() -> Self {
        Self::Fixed(0.0)
    }
}

/// A time as seconds.
#[allow(clippy::cast_precision_loss, reason = "a place to interpolate at")]
pub(crate) fn seconds(time: Time) -> f64 {
    time.ticks as f64 / f64::from(time.scale)
}

impl AudioParam {
    /// Its value where it is one value; none where it has keys.
    #[must_use]
    pub fn fixed(&self) -> Option<f64> {
        match self {
            Self::Fixed(value) => Some(*value),
            Self::Keys(_) => None,
        }
    }

    /// The value at `at` seconds from the start of the media of its clip.
    #[must_use]
    pub fn value_at(&self, at: f64) -> f64 {
        let keys = match self {
            Self::Fixed(value) => return *value,
            Self::Keys(keys) => keys,
        };
        let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
            return 0.0;
        };
        if at <= seconds(first.at) {
            return first.value;
        }
        if at >= seconds(last.at) {
            return last.value;
        }
        // The last key at or before the time; there is one after it.
        let i = keys.partition_point(|key| seconds(key.at) <= at) - 1;
        let (from, to) = (&keys[i], &keys[i + 1]);
        let (t0, t1) = (seconds(from.at), seconds(to.at));
        let share = ((at - t0) / (t1 - t0)).clamp(0.0, 1.0);
        let rise = to.value - from.value;
        let (eases_out, eases_in) = (
            from.interp == KeyInterp::EaseOut,
            to.interp == KeyInterp::EaseIn,
        );
        if !eases_out && !eases_in {
            return from.value + rise * share;
        }
        let leaves = if eases_out { 0.0 } else { 1.0 };
        let arrives = if eases_in { 0.0 } else { 1.0 };
        let (s2, s3) = (share * share, share * share * share);
        (2.0 * s3 - 3.0 * s2 + 1.0) * from.value
            + (s3 - 2.0 * s2 + share) * leaves * rise
            + (-2.0 * s3 + 3.0 * s2) * to.value
            + (s3 - s2) * arrives * rise
    }

    /// Whether it is a value, or keyframes, that `check` takes: finite values, and keys at
    /// times of a scale above 0, in order, at most one at a time, and at least one.
    #[must_use]
    pub fn is_valid(&self, check: impl Fn(f64) -> bool) -> bool {
        match self {
            Self::Fixed(value) => value.is_finite() && check(*value),
            Self::Keys(keys) => {
                !keys.is_empty()
                    && keys.iter().all(|k| {
                        k.value.is_finite()
                            && check(k.value)
                            && k.at.scale > 0
                            && k.at.ticks.unsigned_abs() <= LARGEST.unsigned_abs()
                    })
                    && keys.windows(2).all(|pair| earlier(pair[0].at, pair[1].at))
            }
        }
    }

    /// Every value that it takes at a key, or its one value.
    pub fn values(&self) -> impl Iterator<Item = f64> + '_ {
        let (fixed, keys) = match self {
            Self::Fixed(value) => (Some(*value), &[][..]),
            Self::Keys(keys) => (None, &keys[..]),
        };
        fixed.into_iter().chain(keys.iter().map(|k| k.value))
    }
}

/// Whether time `a` is before time `b`, each of a scale above 0, counted exactly.
pub(crate) fn earlier(a: Time, b: Time) -> bool {
    i128::from(a.ticks) * i128::from(b.scale) < i128::from(b.ticks) * i128::from(a.scale)
}

/// What a lane of automation is of: a setting that changes smoothly, or one that is on or off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneKind {
    /// A setting that goes on a straight line from a point to the next (p. 3897).
    #[default]
    Continuous,
    /// A setting that is on or off, such as a mute: a point holds until the next (p. 3900).
    Switch,
}

/// On what a lane of automation goes in a straight line between two points: its values as they
/// are, or their logarithms (C-A7, the decision of T14's review: a frequency or a Q, which their
/// controls show on a scale of logarithms, so that 100 Hz to 10 kHz passes 1 kHz half way). A
/// level in decibels and a pan are straight in their own units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneScale {
    /// Straight in the values.
    #[default]
    Linear,
    /// Straight in their logarithms: the value at a share `s` of the way from `a` to `b` is
    /// `exp(ln a + (ln b − ln a)·s)`. Every value of such a lane is above 0.
    Log,
}

impl LaneScale {
    /// The scale of a lane of the setting at `path`: [`LaneScale::Log`] of a frequency (a path
    /// whose last part is `freq` or ends in `frequency_hz`) or of a Q (`q`), else linear.
    #[must_use]
    pub fn of_path(path: &str) -> Self {
        let last = path.rsplit('.').next().unwrap_or(path);
        if last == "freq" || last == "q" || last.ends_with("frequency_hz") {
            Self::Log
        } else {
            Self::Linear
        }
    }

    /// The value at `share` of the way from `from` to `to`, as [`Lane::value_at`] works it out.
    #[must_use]
    pub fn between(self, from: f64, to: f64, share: f64) -> f64 {
        match self {
            Self::Linear => from + (to - from) * share,
            Self::Log => {
                let (low, high) = (from.ln(), to.ln());
                (low + (high - low) * share).exp()
            }
        }
    }
}

/// A point of a lane of automation: a value at a time of the timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanePoint {
    /// The time of the timeline that it is at, from its start: written in the scale of the
    /// samples of the timeline when it was written, so that a point is on a sample (p. 3897),
    /// and stays at its time where the rate of the samples changes.
    pub at: Time,
    /// The value there, in the units of the setting (decibels for a fader, from -1 to 1 for a
    /// pan, 0 or 1 for a switch).
    pub value: f64,
}

/// The automation of a setting of a track, a bus or a VCA over its timeline (Ch. 178): points
/// in the order of their times. Where a lane is, it takes the place of the setting's value. Before
/// the first point the value is that of the first, and after the last that of the last (p. 3899);
/// between two, of a continuous lane, on the straight line between them, and of a switch, that
/// of the first. Two points may be at one time, as where a value jumps: the later one in the list
/// holds from that time on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    /// Its points.
    pub points: Vec<LanePoint>,
    /// What it is of. It may be left out, as continuous.
    #[serde(default)]
    pub kind: LaneKind,
    /// On what it goes straight between two points: set by the path of its setting
    /// ([`LaneScale::of_path`]) where it is made or read without one, and written as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<LaneScale>,
}

impl Lane {
    /// The value at time `at` of its timeline, worked out in ticks, exactly but for the share
    /// of the way between two points.
    #[must_use]
    pub fn value_at(&self, at: Time) -> f64 {
        let (Some(first), Some(last)) = (self.points.first(), self.points.last()) else {
            return 0.0;
        };
        // `at` as a count of ticks of the scale of the lane, times the scale of `at`.
        let scale = i128::from(first.at.scale);
        let place = |time: Time| i128::from(time.ticks) * i128::from(at.scale);
        let now = i128::from(at.ticks) * scale;
        if now < place(first.at) {
            return first.value;
        }
        if now >= place(last.at) {
            return last.value;
        }
        let i = self.points.partition_point(|p| place(p.at) <= now) - 1;
        let (from, to) = (&self.points[i], &self.points[i + 1]);
        match self.kind {
            LaneKind::Switch => from.value,
            LaneKind::Continuous => {
                #[allow(clippy::cast_precision_loss, reason = "a share of the way")]
                let share = ((now - place(from.at)) as f64
                    / (place(to.at) - place(from.at)) as f64)
                    .clamp(0.0, 1.0);
                self.scale
                    .unwrap_or_default()
                    .between(from.value, to.value, share)
            }
        }
    }

    /// Whether it has points, of finite values at times all of one scale above 0, from no
    /// earlier than the start of the timeline and no further than 2^53 - 1 ticks, in order.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let Some(first) = self.points.first() else {
            return false;
        };
        self.points.iter().all(|p| {
            p.value.is_finite()
                && p.at.scale > 0
                && p.at.scale == first.at.scale
                && (0..=LARGEST).contains(&p.at.ticks)
        }) && self
            .points
            .windows(2)
            .all(|pair| pair[0].at.ticks <= pair[1].at.ticks)
    }
}

/// The lanes of automation of a track, a bus or a VCA, by the path of the setting that each is
/// of (see PROTOCOL: `fader_db`, `pan.x`, `send.B3.level_db`, `insert.fx1.on` and so on), in
/// the order of their paths.
pub type Lanes = BTreeMap<String, Lane>;

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seconds: i64, value: f64, interp: KeyInterp) -> AudioKey {
        AudioKey {
            at: Time {
                ticks: seconds * 48000,
                scale: 48000,
            },
            value,
            interp,
        }
    }

    #[test]
    fn a_fixed_value_is_written_as_a_number_and_keys_as_an_object() {
        let fixed: AudioParam = serde_json::from_str("-6").unwrap();
        assert_eq!(fixed, AudioParam::Fixed(-6.0));
        assert_eq!(serde_json::to_string(&fixed).unwrap(), "-6.0");
        let keys = AudioParam::Keys(vec![key(1, -6.0, KeyInterp::EaseIn)]);
        let written = serde_json::to_value(&keys).unwrap();
        assert_eq!(
            written,
            serde_json::json!({ "keys": [
                { "at": { "ticks": 48000, "scale": 48000 }, "value": -6.0, "interp": "ease_in" },
            ] })
        );
        assert_eq!(serde_json::from_value::<AudioParam>(written).unwrap(), keys);
        let left_out: AudioParam = serde_json::from_value(serde_json::json!({ "keys": [
            { "at": { "ticks": 0, "scale": 48000 }, "value": 1.0 },
        ] }))
        .unwrap();
        assert_eq!(
            left_out,
            AudioParam::Keys(vec![key(0, 1.0, KeyInterp::Linear)])
        );
        for wrong in [
            serde_json::json!("x"),
            serde_json::json!({ "keys": [], "more": 1 }),
            serde_json::json!(null),
        ] {
            assert!(serde_json::from_value::<AudioParam>(wrong).is_err());
        }
    }

    #[test]
    fn keys_go_on_straight_lines_and_ease_with_a_slope_of_nothing_at_the_easing_end() {
        let linear = AudioParam::Keys(vec![
            key(1, -10.0, KeyInterp::Linear),
            key(3, 0.0, KeyInterp::Linear),
        ]);
        assert_eq!(linear.value_at(0.0), -10.0);
        assert_eq!(linear.value_at(1.5), -7.5);
        assert_eq!(linear.value_at(2.0), -5.0);
        assert_eq!(linear.value_at(9.0), 0.0);
        // Easing out of the first key: slow at first, the cubic h(s) = 2s^2 - s^3 of the rise.
        let out = AudioParam::Keys(vec![
            key(1, -10.0, KeyInterp::EaseOut),
            key(3, 0.0, KeyInterp::Linear),
        ]);
        let near = |a: f64, b: f64| (a - b).abs() < 1e-12;
        assert!(near(
            out.value_at(1.5),
            -10.0 + 10.0 * (2.0 * 0.0625 - 0.015_625)
        ));
        assert!(near(out.value_at(2.0), -10.0 + 10.0 * 0.375));
        // Easing into the second: the mirror image.
        let into = AudioParam::Keys(vec![
            key(1, -10.0, KeyInterp::Linear),
            key(3, 0.0, KeyInterp::EaseIn),
        ]);
        assert!(near(into.value_at(2.0), -10.0 + 10.0 * 0.625));
        // Easing at both ends: smoothstep, through the middle.
        let both = AudioParam::Keys(vec![
            key(1, -10.0, KeyInterp::EaseOut),
            key(3, 0.0, KeyInterp::EaseIn),
        ]);
        assert!(near(both.value_at(2.0), -5.0));
        assert!(near(both.value_at(1.5), -10.0 + 10.0 * 0.156_25));
    }

    #[test]
    fn keys_out_of_order_or_none_are_not_valid() {
        let any = |_: f64| true;
        assert!(AudioParam::Fixed(1.0).is_valid(any));
        assert!(!AudioParam::Fixed(f64::NAN).is_valid(any));
        assert!(!AudioParam::Keys(vec![]).is_valid(any));
        let both = |a, b| {
            AudioParam::Keys(vec![
                key(a, 0.0, KeyInterp::Linear),
                key(b, 0.0, KeyInterp::Linear),
            ])
        };
        assert!(both(1, 2).is_valid(any));
        assert!(!both(2, 2).is_valid(any));
        assert!(!both(2, 1).is_valid(any));
        assert!(!AudioParam::Fixed(30.0).is_valid(|v| v <= 24.0));
    }

    #[test]
    fn a_lane_goes_on_straight_lines_or_holds_and_jumps_where_two_points_meet() {
        let point = |ms: i64, value| LanePoint {
            at: Time {
                ticks: ms * 48,
                scale: 48000,
            },
            value,
        };
        let mut lane = Lane {
            points: vec![point(0, -10.0), point(1000, 0.0), point(1000, -20.0)],
            kind: LaneKind::Continuous,
            scale: None,
        };
        assert!(lane.is_valid());
        // Read at times of other scales, in ticks.
        let at = |ticks, scale| Time { ticks, scale };
        assert!((lane.value_at(at(1, 4)) - -7.5).abs() < 1e-12);
        assert_eq!(lane.value_at(at(-1, 1)), -10.0);
        assert!(lane.value_at(at(47_999, 48000)).abs() < 1e-3);
        assert_eq!(lane.value_at(at(44100, 44100)), -20.0);
        lane.kind = LaneKind::Switch;
        lane.points = vec![point(0, 0.0), point(1000, 1.0)];
        assert_eq!(lane.value_at(at(999, 1000)), 0.0);
        assert_eq!(lane.value_at(at(1, 1)), 1.0);
        lane.points = vec![point(1000, 1.0), point(0, 0.0)];
        assert!(!lane.is_valid());
        // Points of two scales, or past 2^53 - 1 ticks, are not.
        lane.points = vec![
            point(0, 0.0),
            LanePoint {
                at: at(1, 1),
                value: 1.0,
            },
        ];
        assert!(!lane.is_valid());
        lane.points = vec![LanePoint {
            at: at(1 << 53, 48000),
            value: 1.0,
        }];
        assert!(!lane.is_valid());
        lane.points = vec![];
        assert!(!lane.is_valid());
    }

    #[test]
    fn a_lane_of_a_frequency_goes_straight_in_its_logarithm() {
        let at = |ticks| Time {
            ticks,
            scale: 48000,
        };
        let lane = Lane {
            points: vec![
                LanePoint {
                    at: at(0),
                    value: 100.0,
                },
                LanePoint {
                    at: at(48000),
                    value: 10_000.0,
                },
            ],
            kind: LaneKind::Continuous,
            scale: Some(LaneScale::of_path("eq.2.freq")),
        };
        assert!((lane.value_at(at(24000)) - 1000.0).abs() < 1e-9);
        assert!((lane.value_at(at(12000)) - 10f64.powf(2.5)).abs() < 1e-9);
        assert_eq!(LaneScale::of_path("eq.2.q"), LaneScale::Log);
        assert_eq!(
            LaneScale::of_path("insert.fx1.frequency_hz"),
            LaneScale::Log
        );
        assert_eq!(LaneScale::of_path("eq.2.gain_db"), LaneScale::Linear);
        assert_eq!(LaneScale::of_path("pan.x"), LaneScale::Linear);
    }
}
