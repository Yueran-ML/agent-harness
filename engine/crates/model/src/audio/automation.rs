//! The settings that lanes of automation are of (Ch. 178, pp. 3893–3915), found by the paths of
//! their lanes: what kind of lane a setting takes, what its value is where no lane is, what range
//! its lane keeps to, and how the points of a lane are changed: a range taken away, raised or
//! lowered, replaced by points drawn or pasted, and thinned.
//!
//! A path names a field of the strip of a track or a bus as `automatable` (`audio.rs`) lays it out
//! (`fader_db`, `eq.2.gain_db`, `send.B2.level_db`, `insert.fx1.ceiling_db`), or the mute of the
//! track or the bus beside it (`mute`); a VCA has `fader_db` and `mute`. A lane keeps to the range
//! of its setting: each value of it is one that the setting may be set to while the rest of the
//! strip is as it is, and a switch is 0 or 1.
//!
//! The points of a lane are changed exactly, in the ticks of its scale; a time of another scale
//! is put on the nearest tick of it (C-A10).

use serde_json::Value;

use super::param::{Lane, LaneKind, LanePoint, LaneScale, Lanes};
use super::routing::Vca;
use super::{settings_of_strip, Equalizer, Insert, Output, Send, Strip};
use crate::ops::{OpError, LARGEST};
use crate::time::Time;

/// How long a range that is raised or lowered ramps in and out at each end (C-A6), as a part of a
/// second: 1/100, 10 ms.
pub const LANE_RAMP_PER_SECOND: u32 = 100;

/// How far a lane of a fader or of the level of a send may go from what was drawn or recorded
/// where it is thinned (C-A4): 0.1 dB.
pub const LANE_THIN_LEVEL_DB: f64 = 0.1;

/// How far a lane of a pan may go from what was drawn, as a part of the range of its setting
/// (C-A4): 0.5 %.
pub const LANE_THIN_PAN_SHARE: f64 = 0.005;

/// How far a lane of any other setting may go from what was drawn, as a part of the range of its
/// setting as its control shows it (in logarithms for a frequency or a Q; C-A4): 0.1 %.
pub const LANE_THIN_SHARE: f64 = 0.001;

/// What lanes of automation belong to, as it is: the strip of a track or a bus with its mute, or
/// a VCA.
#[derive(Clone, Copy, Debug)]
pub enum Owner<'a> {
    /// The strip of a track of sound (`bus: false`) or of a bus, and its mute.
    Strip {
        /// The strip.
        strip: &'a Strip,
        /// Its mute.
        mute: bool,
        /// Whether it is the strip of a bus.
        bus: bool,
    },
    /// A VCA.
    Vca(&'a Vca),
}

/// The settings of an owner that lanes may be of, as an object whose paths are the paths of the
/// lanes (see `automatable` (`audio.rs`)), with their values as they are set.
#[must_use]
pub fn view(owner: Owner<'_>) -> Value {
    match owner {
        Owner::Strip { strip, mute, .. } => {
            let mut view = super::automatable(strip);
            if let Some(object) = view.as_object_mut() {
                object.insert("mute".into(), mute.into());
            }
            view
        }
        Owner::Vca(vca) => serde_json::json!({ "fader_db": vca.fader_db, "mute": vca.mute }),
    }
}

/// Take out of a view of the settings of a strip what the graph does not move as a lane would,
/// and whole numbers (T14's review): Effects In (`inserts_on`); of its pan all but `x`, `y` and
/// `divergence`; of a send whether it is before the fader (`pre`) and of its pan all but `x` and
/// `y`; the Lookahead of a Ducker (its latency, a structure of the graph); every setting of a
/// Pitch (which has no node yet); and any setting written as a whole number.
pub fn drop_unwired(view: &mut Value) {
    let Some(object) = view.as_object_mut() else {
        return;
    };
    object.remove("inserts_on");
    let keep = |pan: &mut Value, kept: &[&str]| {
        if let Some(pan) = pan.as_object_mut() {
            pan.retain(|key, _| kept.contains(&key.as_str()));
        }
    };
    if let Some(pan) = object.get_mut("pan") {
        keep(pan, &["x", "y", "divergence"]);
    }
    if let Some(Value::Object(sends)) = object.get_mut("send") {
        for send in sends.values_mut().filter_map(Value::as_object_mut) {
            send.remove("pre");
            if let Some(pan) = send.get_mut("pan") {
                keep(pan, &["x", "y"]);
            }
        }
    }
    if let Some(Value::Object(ducker)) = object.get_mut("track_fx") {
        ducker.remove("lookahead_ms");
    }
    if let Some(Value::Object(inserts)) = object.get_mut("insert") {
        inserts.retain(|_, effect| effect.get("kind").and_then(Value::as_str) != Some("pitch"));
    }
    drop_whole_numbers(view);
}

/// Take every number that is written as a whole number out of a view, at any depth.
fn drop_whole_numbers(view: &mut Value) {
    if let Value::Object(object) = view {
        object.retain(|_, value| !matches!(value, Value::Number(n) if !n.is_f64()));
        object.values_mut().for_each(drop_whole_numbers);
    }
}

/// The setting at `path` of `view` (see [`view`]): a number or a switch, or none where there is
/// no such setting that a lane may be of. A key that holds a `.` (a bus or an insert named so)
/// is found as a whole.
#[must_use]
pub fn setting<'v>(view: &'v Value, path: &str) -> Option<&'v Value> {
    match view {
        Value::Number(_) | Value::Bool(_) if path.is_empty() => Some(view),
        Value::Object(object) if !path.is_empty() => object.iter().find_map(|(key, inner)| {
            if path == key {
                setting(inner, "")
            } else {
                setting(inner, path.strip_prefix(key.as_str())?.strip_prefix('.')?)
            }
        }),
        _ => None,
    }
}

/// The kind of a lane of the setting at `path` of `view`: continuous for a number, a switch for
/// one that is on or off; none where it has no lane.
#[must_use]
pub fn kind_of(view: &Value, path: &str) -> Option<LaneKind> {
    match setting(view, path)? {
        Value::Number(_) => Some(LaneKind::Continuous),
        Value::Bool(_) => Some(LaneKind::Switch),
        _ => None,
    }
}

/// The paths of every setting of `owner` that a lane may be of, in the order of `view`: what a
/// menu of lanes lists (p. 3896).
#[must_use]
pub fn paths(owner: Owner<'_>) -> Vec<String> {
    fn walk(view: &Value, prefix: &str, into: &mut Vec<String>) {
        match view {
            Value::Number(_) | Value::Bool(_) => into.push(prefix.into()),
            Value::Object(object) => {
                for (key, inner) in object {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    walk(inner, &path, into);
                }
            }
            _ => {}
        }
    }
    let mut into = Vec::new();
    walk(&view(owner), "", &mut into);
    into
}

/// Why a lane of `path` of `owner` is refused where no setting of a lane is there: `lane_integer`
/// of a setting that is a whole number, `lane_not_wired` of one that the graph does not move as a
/// lane would (see [`drop_unwired`]), and `lane` of anything else.
#[must_use]
pub fn no_lane(owner: Owner<'_>, path: &str) -> &'static str {
    let Owner::Strip { strip, .. } = owner else {
        return "lane";
    };
    match setting(&settings_of_strip(strip), path) {
        Some(Value::Number(n)) if !n.is_f64() => "lane_integer",
        Some(_) => "lane_not_wired",
        None => "lane",
    }
}

/// The value of the setting at `path` of `view` as it is set, where no lane is: a number, or 1
/// for a switch that is on and 0 for one that is off.
#[must_use]
pub fn fixed_value(view: &Value, path: &str) -> Option<f64> {
    match setting(view, path)? {
        Value::Number(number) => number.as_f64(),
        Value::Bool(on) => Some(if *on { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// A value of a switch: on (1) at 0.5 and above, else off (0) (C-A5).
#[must_use]
pub fn switched(value: f64) -> f64 {
    if value >= 0.5 {
        1.0
    } else {
        0.0
    }
}

/// Give each lane that has no [`LaneScale`] the one of its path, as a lane that is read or made
/// without one is given it.
pub fn fill_scales(lanes: &mut Lanes) {
    for (path, lane) in lanes.iter_mut() {
        lane.scale.get_or_insert(LaneScale::of_path(path));
    }
}

/// What an owner becomes with the setting at a path set to a value.
#[derive(Clone, Debug, PartialEq)]
pub enum Set {
    /// A strip, and its mute.
    Strip(Box<Strip>, bool),
    /// A VCA.
    Vca(Vca),
}

/// The place of the item of `list` (an array of objects) whose `field` starts `path` before a
/// `.`, and the rest of the path after it.
fn keyed<'p>(list: &Value, field: &str, path: &'p str) -> Option<(usize, &'p str)> {
    list.as_array()?.iter().enumerate().find_map(|(at, item)| {
        let key = item.get(field)?.as_str()?;
        Some((at, path.strip_prefix(key)?.strip_prefix('.')?))
    })
}

/// The place in the JSON of a strip of the setting at `path`, as `automatable` (`audio.rs`) lays
/// it out.
fn place_in<'s>(strip: &'s mut Value, path: &str) -> Option<&'s mut Value> {
    let (head, rest) = path.split_once('.').unwrap_or((path, ""));
    let (base, rest): (&mut Value, &str) = match head {
        "send" => {
            let (at, rest) = keyed(strip.get("sends")?, "bus", rest)?;
            (strip.get_mut("sends")?.get_mut(at)?, rest)
        }
        "output" => {
            let (at, rest) = keyed(strip.get("outputs")?, "bus", rest)?;
            (strip.get_mut("outputs")?.get_mut(at)?, rest)
        }
        "insert" => {
            let (at, rest) = keyed(strip.get("inserts")?, "id", rest)?;
            let insert = strip.get_mut("inserts")?.get_mut(at)?;
            if rest == "on" {
                (insert, rest)
            } else {
                (insert.get_mut("effect")?, rest)
            }
        }
        "eq" => match rest.split_once('.') {
            Some((band, inner)) if band.parse::<usize>().is_ok() => {
                let at: usize = band.parse().ok()?;
                (strip.get_mut("eq")?.get_mut("bands")?.get_mut(at)?, inner)
            }
            _ => (strip.get_mut("eq")?, rest),
        },
        "order" | "inserts" | "sends" | "outputs" | "vcas" => return None,
        _ => (strip, path),
    };
    let mut at = base;
    for key in rest.split('.') {
        at = at.as_object_mut()?.get_mut(key)?;
    }
    Some(at)
}

/// `owner` with the setting at `path` set to `value` (a switch on at 0.5 and above), or none
/// where it has no such setting, or the value is not a number or not one of the kind of the
/// setting.
#[must_use]
pub fn with_value(owner: Owner<'_>, path: &str, value: f64) -> Option<Set> {
    if !value.is_finite() {
        return None;
    }
    match owner {
        Owner::Vca(vca) => {
            let mut vca = vca.clone();
            match path {
                "fader_db" => vca.fader_db = value,
                "mute" => vca.mute = value >= 0.5,
                _ => return None,
            }
            Some(Set::Vca(vca))
        }
        Owner::Strip { strip, mute, .. } => {
            if path == "mute" {
                return Some(Set::Strip(Box::new(strip.clone()), value >= 0.5));
            }
            let mut json = serde_json::to_value(strip).ok()?;
            let place = place_in(&mut json, path)?;
            *place = match place {
                Value::Bool(_) => Value::Bool(value >= 0.5),
                Value::Number(_) => serde_json::Number::from_f64(value)?.into(),
                _ => return None,
            };
            let strip = serde_json::from_value(json).ok()?;
            Some(Set::Strip(Box::new(strip), mute))
        }
    }
}

/// Whether the setting at `path` of `owner` may be set to `value`, the rest as it is.
#[must_use]
pub fn fits(owner: Owner<'_>, path: &str, value: f64) -> bool {
    let bus = matches!(owner, Owner::Strip { bus: true, .. });
    match with_value(owner, path, value) {
        Some(Set::Strip(strip, _)) => strip.check(bus).is_ok(),
        Some(Set::Vca(vca)) => vca.fader_db.is_finite() && vca.fader_db <= 10.0,
        None => false,
    }
}

/// A double as a number whose order is that of the doubles, for halving the way between two.
fn ordered(value: f64) -> i64 {
    let bits = value.to_bits().cast_signed();
    if bits < 0 {
        i64::MIN - bits
    } else {
        bits
    }
}

/// The double of a number of [`ordered`].
fn unordered(key: i64) -> f64 {
    let bits = if key < 0 { i64::MIN - key } else { key };
    f64::from_bits(bits.cast_unsigned())
}

/// `value`, or the nearest to it that the setting at `path` of `owner` may be set to, between it
/// and `inside`, a value that the setting may be set to; and whether it was moved.
#[must_use]
pub fn kept_within(owner: Owner<'_>, path: &str, value: f64, inside: f64) -> (f64, bool) {
    if fits(owner, path, value) {
        return (value, false);
    }
    // Halve the way, in the order of the doubles, between one that fits and one that does not:
    // at most 64 halvings find the last double that fits.
    let (mut good, mut bad) = (ordered(inside), ordered(value));
    while good.abs_diff(bad) > 1 {
        let middle =
            i64::try_from(i128::midpoint(i128::from(good), i128::from(bad))).unwrap_or(good);
        if fits(owner, path, unordered(middle)) {
            good = middle;
        } else {
            bad = middle;
        }
    }
    (unordered(good), true)
}

/// The least and the greatest value that the setting at `path` of `owner` may be set to, the
/// rest as it is, found once by halving from `inside`, a value that it may be set to: a setting
/// takes every value between them. An end that it has not is `∓f64::MAX`.
#[must_use]
pub fn range_of(owner: Owner<'_>, path: &str, inside: f64) -> (f64, f64) {
    (
        kept_within(owner, path, -f64::MAX, inside).0,
        kept_within(owner, path, f64::MAX, inside).0,
    )
}

/// Check one lane of an owner whose settings are `view` ([`view`]): of a setting that it has, of
/// the kind and the scale of that setting, with points (see [`Lane::is_valid`]) whose values the
/// setting may be set to (from the least of them to the greatest, every one above 0 on a scale of
/// logarithms), or 0 and 1 for a switch.
///
/// # Errors
/// It is not: `out_of_range`, `lane` (or `lane_integer`, `lane_not_wired`, see [`no_lane`]).
pub fn check_lane(path: &str, lane: &Lane, owner: Owner<'_>, view: &Value) -> Result<(), OpError> {
    let wrong = Err(OpError::OutOfRange { what: "lane" });
    let Some(kind) = kind_of(view, path) else {
        return Err(OpError::OutOfRange {
            what: no_lane(owner, path),
        });
    };
    if !lane.is_valid() || kind != lane.kind || lane.scale != Some(LaneScale::of_path(path)) {
        return wrong;
    }
    let fine = match lane.kind {
        LaneKind::Switch => lane
            .points
            .iter()
            .all(|p| p.value.to_bits() == 0f64.to_bits() || p.value.to_bits() == 1f64.to_bits()),
        LaneKind::Continuous => {
            let values = lane.points.iter().map(|p| p.value);
            let least = values.clone().fold(f64::INFINITY, f64::min);
            let most = values.fold(f64::NEG_INFINITY, f64::max);
            let above = lane.scale != Some(LaneScale::Log) || least > 0.0;
            above && fits(owner, path, least) && fits(owner, path, most)
        }
    };
    if fine {
        Ok(())
    } else {
        wrong
    }
}

/// Check the lanes of an owner, each as [`check_lane`] does.
///
/// # Errors
/// One is not as it should be.
pub fn check_lanes(lanes: &Lanes, owner: Owner<'_>) -> Result<(), OpError> {
    if lanes.is_empty() {
        return Ok(());
    }
    let view = view(owner);
    lanes
        .iter()
        .try_for_each(|(path, lane)| check_lane(path, lane, owner, &view))
}

/// Whether `lanes` has a lane of an insert that is in both `before` and `after` with an effect of
/// another kind: a lane that the insert there now would take for its own (T14's review).
#[must_use]
pub fn lanes_of_changed_inserts(before: &Strip, after: &Strip, lanes: &Lanes) -> Vec<String> {
    let changed: Vec<&str> = after
        .inserts
        .iter()
        .filter(|insert| {
            before
                .inserts
                .iter()
                .any(|old| old.id == insert.id && old.effect.name() != insert.effect.name())
        })
        .map(|insert| insert.id.as_str())
        .collect();
    lanes
        .keys()
        .filter(|path| {
            changed.iter().any(|id| {
                path.strip_prefix("insert.")
                    .and_then(|rest| rest.strip_prefix(id))
                    .is_some_and(|rest| rest.starts_with('.'))
            })
        })
        .cloned()
        .collect()
}

/// A strip with the settings of a new one, but laid out as `strip` is: its sends, outputs and
/// inserts (each a new effect of its kind) where they are, so that every path of `strip` is
/// found in it (Clear Track Automation › Reset, p. 3912).
#[must_use]
pub fn defaults_like(strip: &Strip) -> Strip {
    let new = |insert: &Insert| -> Insert {
        let kind = serde_json::json!({ "kind": insert.effect.name() });
        Insert {
            id: insert.id.clone(),
            on: true,
            effect: serde_json::from_value(kind).unwrap_or_else(|_| insert.effect.clone()),
        }
    };
    Strip {
        track_fx: strip.track_fx.as_ref().map(|ducker| super::fx::Ducker {
            sources: ducker.sources.clone(),
            ..super::fx::Ducker::default()
        }),
        order: strip.order.clone(),
        inserts: strip.inserts.iter().map(new).collect(),
        eq: Equalizer::strip(),
        sends: strip
            .sends
            .iter()
            .map(|send| Send {
                bus: send.bus.clone(),
                on: true,
                pre: false,
                level_db: 0.0,
                pan: super::Pan::default(),
            })
            .collect(),
        outputs: strip
            .outputs
            .iter()
            .map(|output| Output {
                bus: output.bus.clone(),
                on: true,
            })
            .collect(),
        vcas: strip.vcas.clone(),
        ..Strip::default()
    }
}

// The points of a lane, in ticks of its scale.

/// What the points of a lane are joined by: its kind (a switch holds) and the scale that a
/// continuous lane goes straight on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// Its kind.
    pub kind: LaneKind,
    /// Its scale.
    pub scale: LaneScale,
}

impl Shape {
    /// The shape of a lane.
    #[must_use]
    pub fn of(lane: &Lane) -> Self {
        Self {
            kind: lane.kind,
            scale: lane.scale.unwrap_or_default(),
        }
    }

    /// A value as the scale measures it: itself, or its logarithm.
    #[must_use]
    pub fn measured(self, value: f64) -> f64 {
        match self.scale {
            LaneScale::Linear => value,
            LaneScale::Log => value.ln(),
        }
    }
}

/// A time as ticks of `scale`, on the nearest tick (halves away from 0), or none where it does
/// not fit within 0…2^53−1 ticks.
#[must_use]
pub fn ticks_in(time: Time, scale: u32) -> Option<i64> {
    if time.scale == 0 || scale == 0 {
        return None;
    }
    let (num, den) = (
        i128::from(time.ticks) * i128::from(scale),
        i128::from(time.scale),
    );
    let half = den / 2;
    let ticks = if num >= 0 {
        (num + half) / den
    } else {
        (num - half) / den
    };
    i64::try_from(ticks)
        .ok()
        .filter(|t| (0..=LARGEST).contains(t))
}

/// The value of a lane of `shape` with `points` (in ticks of one scale) at tick `at`, the later
/// of two points at one time holding from it; `fixed` where it has none. As [`Lane::value_at`].
#[must_use]
pub fn value_at(points: &[LanePoint], shape: Shape, fixed: f64, at: i64) -> f64 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return fixed;
    };
    if at < first.at.ticks {
        return first.value;
    }
    if at >= last.at.ticks {
        return last.value;
    }
    let i = points.partition_point(|p| p.at.ticks <= at) - 1;
    between(&points[i], &points[i + 1], shape, at)
}

/// The value between two points at tick `at`, from the first.
fn between(from: &LanePoint, to: &LanePoint, shape: Shape, at: i64) -> f64 {
    match shape.kind {
        LaneKind::Switch => from.value,
        LaneKind::Continuous => {
            let span = i128::from(to.at.ticks) - i128::from(from.at.ticks);
            if span == 0 {
                return to.value;
            }
            #[allow(clippy::cast_precision_loss, reason = "a share of the way")]
            let way =
                ((i128::from(at) - i128::from(from.at.ticks)) as f64 / span as f64).clamp(0.0, 1.0);
            shape.scale.between(from.value, to.value, way)
        }
    }
}

/// The value of a lane just before tick `at`: what it comes to from the left, before any point
/// at `at` holds (p. 3898: what is before a range stays as it was).
#[must_use]
pub fn value_before(points: &[LanePoint], shape: Shape, fixed: f64, at: i64) -> f64 {
    let Some(first) = points.first() else {
        return fixed;
    };
    let i = points.partition_point(|p| p.at.ticks < at);
    if i == 0 {
        return first.value;
    }
    match (shape.kind, points.get(i)) {
        (LaneKind::Continuous, Some(next)) => between(&points[i - 1], next, shape, at),
        _ => points[i - 1].value,
    }
}

/// A point at tick `ticks` of `scale`.
#[must_use]
pub fn point(ticks: i64, scale: u32, value: f64) -> LanePoint {
    LanePoint {
        at: Time { ticks, scale },
        value,
    }
}

/// Take away a point that is at the time and of the value of the one before it.
#[must_use]
pub fn without_twins(points: Vec<LanePoint>) -> Vec<LanePoint> {
    let mut kept: Vec<LanePoint> = Vec::with_capacity(points.len());
    for p in points {
        let twin = kept
            .last()
            .is_some_and(|q| q.at.ticks == p.at.ticks && q.value.to_bits() == p.value.to_bits());
        if !twin {
            kept.push(p);
        }
    }
    kept
}

/// `value` within the least and the greatest of the values of `points` (as it is, where there
/// are none): a value that a lane comes to between two of its points is between theirs but for
/// the rounding of `a + (b − a)·s` (0.2 to −0.9 comes to −0.9000000000000001), which this takes
/// away, so that a point made of it gives the lane no value beyond those it had.
#[must_use]
pub fn bounded(points: &[LanePoint], value: f64) -> f64 {
    let (least, most) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.value), hi.max(p.value))
        });
    if least <= most {
        value.clamp(least, most)
    } else {
        value
    }
}

/// The points of a lane with those from tick `from` to tick `to`, both in, put in place of by
/// `new`: the value before `from` and the value from `to` on stay as they were, as a point at
/// `from` of the value that came to it and one at `to` of the value that was there (the bounding
/// keyframes of p. 3899; the ends of a range taken away keep their values, p. 3898), each within
/// the values that the lane had ([`bounded`]). `new` must be in order, from `from` to `to`.
#[must_use]
pub fn replaced(
    points: &[LanePoint],
    shape: Shape,
    fixed: f64,
    scale: u32,
    (from, to): (i64, i64),
    new: &[LanePoint],
) -> Vec<LanePoint> {
    let left = bounded(points, value_before(points, shape, fixed, from));
    let right = bounded(points, value_at(points, shape, fixed, to));
    let before = points.iter().filter(|p| p.at.ticks < from);
    let after = points.iter().filter(|p| p.at.ticks > to);
    let all: Vec<LanePoint> = before
        .cloned()
        .chain([point(from, scale, left)])
        .chain(new.iter().cloned())
        .chain([point(to, scale, right)])
        .chain(after.cloned())
        .collect();
    without_twins(all)
}

/// The points of a lane with those from tick `from` on put in place of by `new`: the value before
/// `from` stays as it was, as a point at `from` of the value that came to it, and nothing of the
/// lane is left after `new` (Hold at the stop of a pass of recording, p. 3905: the last value
/// held to the end). `new` must be in order, from `from` on.
#[must_use]
pub fn held(
    points: &[LanePoint],
    shape: Shape,
    fixed: f64,
    scale: u32,
    from: i64,
    new: &[LanePoint],
) -> Vec<LanePoint> {
    let left = value_before(points, shape, fixed, from);
    let all: Vec<LanePoint> = points
        .iter()
        .filter(|p| p.at.ticks < from)
        .cloned()
        .chain([point(from, scale, left)])
        .chain(new.iter().cloned())
        .collect();
    without_twins(all)
}

/// A lane of decibels moved by an offset that changes (a Trim recorded, pp. 3900–3901, 3905): at
/// the times of the points of the offset `offset` (in order, from its first, `from`) and of the
/// lane `points` after `from` (before `until`, where it is given), the value of the lane there
/// plus the offset there, both straight between those times, so their sum is too; a point of
/// each value where either jumps. The offset is 0 before `from`, and holds its last value after
/// its last point. A value of the lane at −∞ ([`super::SILENT_DB`] or below) stays as it was.
/// The values are not kept within a range.
#[must_use]
pub fn offset_by(
    points: &[LanePoint],
    shape: Shape,
    fixed: f64,
    scale: u32,
    offset: &[LanePoint],
    until: Option<i64>,
) -> Vec<LanePoint> {
    let Some(first) = offset.first() else {
        return Vec::new();
    };
    let from = first.at.ticks;
    let line = Shape {
        kind: LaneKind::Continuous,
        scale: LaneScale::Linear,
    };
    let mut times: Vec<i64> = offset
        .iter()
        .map(|p| p.at.ticks)
        .chain(
            points
                .iter()
                .map(|p| p.at.ticks)
                .filter(|&t| t > from && until.is_none_or(|until| t < until)),
        )
        .collect();
    times.sort_unstable();
    times.dedup();
    let moved = |value: f64, by: f64| {
        if value <= super::SILENT_DB {
            value
        } else {
            value + by
        }
    };
    let mut out = Vec::with_capacity(times.len() * 2);
    for t in times {
        let right = moved(
            value_at(points, shape, fixed, t),
            value_at(offset, line, 0.0, t),
        );
        if t > from {
            let left = moved(
                value_before(points, shape, fixed, t),
                value_before(offset, line, 0.0, t),
            );
            if left.to_bits() != right.to_bits() {
                out.push(point(t, scale, left));
            }
        }
        out.push(point(t, scale, right));
    }
    out
}

/// The points of a lane with the values from tick `from` to tick `to` moved by `delta`, through
/// `change` (which keeps a value within the range of its setting), the values outside the range
/// staying as they were (p. 3899): at each end a point of the value that was there and, `ramp`
/// ticks within, one of the value moved, so that it ramps to it (C-A6; a switch jumps: `ramp`
/// 0). The points between the two within move; those between the pairs go. Where the two within
/// meet (a range of twice `ramp`), the one there goes from the value that came to it to the one
/// that leaves it, in that order.
#[must_use]
pub fn shifted(
    points: &[LanePoint],
    shape: Shape,
    fixed: f64,
    scale: u32,
    (from, to): (i64, i64),
    ramp: i64,
    change: &dyn Fn(f64) -> f64,
) -> Vec<LanePoint> {
    let ramp = ramp.clamp(0, (to - from) / 2);
    let (inner_from, inner_to) = (from + ramp, to - ramp);
    let (start, end) = (
        point(
            inner_from,
            scale,
            change(value_at(points, shape, fixed, inner_from)),
        ),
        point(
            inner_to,
            scale,
            change(value_before(points, shape, fixed, inner_to)),
        ),
    );
    let new = if inner_from == inner_to {
        vec![end, start]
    } else {
        let mut new = vec![start];
        new.extend(
            points
                .iter()
                .filter(|p| p.at.ticks > inner_from && p.at.ticks < inner_to)
                .map(|p| point(p.at.ticks, scale, change(p.value))),
        );
        new.push(end);
        new
    };
    replaced(points, shape, fixed, scale, (from, to), &without_twins(new))
}

/// The points that Douglas and Peucker's thinning keeps of `points` (in order of time): the first,
/// the last, and between them those without which the lane would go further than `tolerance`
/// from one of them, in its value at its time as its scale measures it (C-A4). Of a switch,
/// where its value changes.
#[must_use]
pub fn thinned(points: &[LanePoint], shape: Shape, tolerance: f64) -> Vec<LanePoint> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let last = points.len() - 1;
    // The ends, and both points of a jump (two at one time), are kept.
    let mut keep: Vec<bool> = (0..points.len())
        .map(|i| {
            i == 0
                || i == last
                || points[i - 1].at.ticks == points[i].at.ticks
                || points[i + 1].at.ticks == points[i].at.ticks
        })
        .collect();
    match shape.kind {
        LaneKind::Switch => {
            for i in 1..points.len() {
                if points[i].value.to_bits() != points[i - 1].value.to_bits() {
                    keep[i] = true;
                }
            }
        }
        LaneKind::Continuous => {
            let kept: Vec<usize> = (0..points.len()).filter(|&i| keep[i]).collect();
            let mut stack: Vec<(usize, usize)> = kept.windows(2).map(|w| (w[0], w[1])).collect();
            while let Some((a, b)) = stack.pop() {
                if b <= a + 1 {
                    continue;
                }
                // The point between that is furthest from the line from `a` to `b`, which are
                // at two times, since every point of a jump is kept.
                let (far, distance) = (a + 1..b)
                    .map(|i| {
                        let on = between(&points[a], &points[b], shape, points[i].at.ticks);
                        let off = shape.measured(points[i].value) - shape.measured(on);
                        (i, off.abs())
                    })
                    .fold(
                        (a, -1.0),
                        |best, next| if next.1 > best.1 { next } else { best },
                    );
                if distance > tolerance {
                    keep[far] = true;
                    stack.push((a, far));
                    stack.push((far, b));
                }
            }
        }
    }
    points
        .iter()
        .zip(keep)
        .filter(|(_, kept)| *kept)
        .map(|(p, _)| p.clone())
        .collect()
}

/// How far a lane of the setting at `path` may go from what was drawn, where its points are
/// thinned (C-A4), as its scale measures it: [`LANE_THIN_LEVEL_DB`] of a fader or the level of a
/// send, [`LANE_THIN_PAN_SHARE`] of the range `range` of a pan, and [`LANE_THIN_SHARE`] of the
/// range of any other setting (of the logarithms of its ends for a frequency or a Q). A range
/// without an end has 0.001.
#[must_use]
pub fn tolerance_of(path: &str, range: (f64, f64)) -> f64 {
    let last = path.rsplit('.').next().unwrap_or(path);
    if path == "fader_db" || (path.starts_with("send.") && last == "level_db") {
        return LANE_THIN_LEVEL_DB;
    }
    let shape = Shape {
        kind: LaneKind::Continuous,
        scale: LaneScale::of_path(path),
    };
    let width = shape.measured(range.1) - shape.measured(range.0);
    let part = if path.starts_with("pan.") || path.contains(".pan.") {
        LANE_THIN_PAN_SHARE
    } else {
        LANE_THIN_SHARE
    };
    if width.is_finite() && width > 0.0 && width < f64::MAX / 2.0 {
        width * part
    } else {
        0.001
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: Shape = Shape {
        kind: LaneKind::Continuous,
        scale: LaneScale::Linear,
    };
    const SWITCH: Shape = Shape {
        kind: LaneKind::Switch,
        scale: LaneScale::Linear,
    };

    fn points(list: &[(i64, f64)]) -> Vec<LanePoint> {
        list.iter().map(|&(t, v)| point(t, 1000, v)).collect()
    }

    #[test]
    fn a_path_finds_its_setting_and_its_kind() {
        let mut strip = Strip::to_first_bus();
        strip.sends.push(Send {
            bus: "B2".into(),
            on: true,
            pre: false,
            level_db: -6.0,
            pan: super::super::Pan::default(),
        });
        let owner = Owner::Strip {
            strip: &strip,
            mute: true,
            bus: false,
        };
        let view = view(owner);
        assert_eq!(fixed_value(&view, "send.B2.level_db"), Some(-6.0));
        assert_eq!(fixed_value(&view, "mute"), Some(1.0));
        assert_eq!(kind_of(&view, "output.B1.on"), Some(LaneKind::Switch));
        assert_eq!(kind_of(&view, "eq.5.freq"), Some(LaneKind::Continuous));
        assert_eq!(kind_of(&view, "eq.6.freq"), None);
        assert_eq!(kind_of(&view, "send.B3.level_db"), None);
        // What the graph does not move, and whole numbers, have no lane, and say why.
        for (path, why) in [
            ("inserts_on", "lane_not_wired"),
            ("pan.spread", "lane_not_wired"),
            ("send.B2.pre", "lane_not_wired"),
            ("send.B2.pan.divergence", "lane_not_wired"),
            ("eq.1.slope", "lane_integer"),
            ("eq.1.kind", "lane"),
            ("nothing", "lane"),
        ] {
            assert_eq!(kind_of(&view, path), None, "{path}");
            assert_eq!(no_lane(owner, path), why, "{path}");
        }
        let Some(Set::Strip(set, mute)) = with_value(owner, "send.B2.level_db", 3.0) else {
            panic!("the send is there");
        };
        assert_eq!(set.sends[0].level_db, 3.0);
        assert!(mute);
        let Some(Set::Strip(set, _)) = with_value(owner, "eq.2.gain_db", 4.0) else {
            panic!("the band is there");
        };
        assert_eq!(set.eq.bands[2].gain_db, 4.0);
        assert!(fits(owner, "fader_db", 10.0));
        assert!(!fits(owner, "fader_db", 10.5));
        assert!(!fits(owner, "pan.x", 1.5));
        assert_eq!(kept_within(owner, "fader_db", 30.0, 0.0), (10.0, true));
        assert_eq!(kept_within(owner, "pan.x", -3.0, 0.0), (-1.0, true));
        assert_eq!(kept_within(owner, "pan.x", 0.5, 0.0), (0.5, false));
        assert_eq!(range_of(owner, "pan.x", 0.0), (-1.0, 1.0));
        assert_eq!(range_of(owner, "fader_db", 0.0), (-f64::MAX, 10.0));
    }

    #[test]
    fn a_range_taken_away_keeps_the_values_at_its_ends() {
        let lane = points(&[(0, -10.0), (1000, 0.0), (3000, -20.0)]);
        let new = replaced(&lane, LINE, 0.0, 1000, (500, 2000), &[]);
        assert_eq!(
            new,
            points(&[(0, -10.0), (500, -5.0), (2000, -10.0), (3000, -20.0)])
        );
        for at in [0, 250, 499, 2000, 2500, 3000] {
            let (old, now) = (
                value_at(&lane, LINE, 0.0, at),
                value_at(&new, LINE, 0.0, at),
            );
            assert!((old - now).abs() < 1e-12, "{at}: {old} {now}");
        }
        // A jump at the start of the range: what came to it stays.
        let jump = points(&[(0, 0.0), (500, -10.0), (500, 5.0), (1000, 5.0)]);
        let new = replaced(&jump, LINE, 0.0, 1000, (500, 800), &[]);
        assert_eq!(
            new,
            points(&[(0, 0.0), (500, -10.0), (800, 5.0), (1000, 5.0)])
        );
    }

    #[test]
    fn a_range_moved_ramps_in_and_out_of_its_new_values() {
        let lane = points(&[(0, -10.0), (1000, -10.0)]);
        let new = shifted(&lane, LINE, 0.0, 1000, (200, 600), 10, &|v| v - 6.0);
        assert_eq!(
            new,
            points(&[
                (0, -10.0),
                (200, -10.0),
                (210, -16.0),
                (590, -16.0),
                (600, -10.0),
                (1000, -10.0)
            ])
        );
        // A switch jumps at both ends.
        let mute = points(&[(0, 0.0)]);
        let new = shifted(&mute, SWITCH, 0.0, 1000, (200, 600), 0, &|v| {
            switched(v + 1.0)
        });
        assert_eq!(
            new,
            points(&[(0, 0.0), (200, 0.0), (200, 1.0), (600, 1.0), (600, 0.0)])
        );
        assert_eq!(value_at(&new, SWITCH, 0.0, 199), 0.0);
        assert_eq!(value_at(&new, SWITCH, 0.0, 200), 1.0);
        assert_eq!(value_at(&new, SWITCH, 0.0, 599), 1.0);
        assert_eq!(value_at(&new, SWITCH, 0.0, 600), 0.0);
    }

    #[test]
    fn a_range_moved_whose_ramps_meet_keeps_a_jump_where_they_meet_in_order() {
        // A range of 20 ticks with ramps of 10: the two points within meet at 210, where the lane
        // jumps from 0 to 5. Moved by 6, it comes to 6 there and leaves from 11, as it came to 0
        // and left from 5: the value at 210 is the one after the jump, moved.
        let lane = points(&[(0, 0.0), (210, 0.0), (210, 5.0), (1000, 5.0)]);
        let new = shifted(&lane, LINE, 0.0, 1000, (200, 220), 10, &|v| v + 6.0);
        let times: Vec<i64> = new.iter().map(|p| p.at.ticks).collect();
        assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
        assert_eq!(
            new,
            points(&[
                (0, 0.0),
                (200, 0.0),
                (210, 6.0),
                (210, 11.0),
                (220, 5.0),
                (1000, 5.0)
            ])
        );
        assert_eq!(value_at(&new, LINE, 0.0, 210), 11.0);
        assert_eq!(value_at(&new, LINE, 0.0, 215), 8.0);
    }

    #[test]
    fn thinning_keeps_what_bends_further_than_its_tolerance() {
        let ramp: Vec<LanePoint> = (0..=100)
            .map(|t| point(t, 1000, f64::from(i32::try_from(t).unwrap()) * -0.2))
            .collect();
        assert_eq!(thinned(&ramp, LINE, 0.1), points(&[(0, 0.0), (100, -20.0)]));
        let bent = points(&[(0, 0.0), (10, 0.05), (20, 0.0), (30, 1.0), (40, 0.0)]);
        assert_eq!(
            thinned(&bent, LINE, 0.1),
            points(&[(0, 0.0), (20, 0.0), (30, 1.0), (40, 0.0)])
        );
        let mute = points(&[(0, 0.0), (5, 0.0), (6, 1.0), (9, 1.0), (12, 0.0), (15, 0.0)]);
        assert_eq!(
            thinned(&mute, SWITCH, 0.0),
            points(&[(0, 0.0), (6, 1.0), (12, 0.0), (15, 0.0)])
        );
        // A sweep of a frequency, even in its logarithm, is two points; in Hz it would bend.
        let log = Shape {
            kind: LaneKind::Continuous,
            scale: LaneScale::Log,
        };
        let sweep: Vec<LanePoint> = (0..=100)
            .map(|t| {
                point(
                    t,
                    1000,
                    100.0 * 100f64.powf(f64::from(i32::try_from(t).unwrap()) / 100.0),
                )
            })
            .collect();
        assert_eq!(thinned(&sweep, log, 1e-6).len(), 2);
        assert!(thinned(&sweep, LINE, 1.0).len() > 2);
    }

    #[test]
    fn the_tolerances_of_thinning_are_those_of_c_a4() {
        assert_eq!(tolerance_of("fader_db", (-f64::MAX, 10.0)), 0.1);
        assert_eq!(tolerance_of("send.B2.level_db", (-f64::MAX, 10.0)), 0.1);
        assert!((tolerance_of("pan.x", (-1.0, 1.0)) - 0.01).abs() < 1e-15);
        assert!((tolerance_of("pan.divergence", (0.0, 100.0)) - 0.5).abs() < 1e-12);
        assert!((tolerance_of("send.B2.pan.y", (-1.0, 1.0)) - 0.01).abs() < 1e-15);
        assert!((tolerance_of("eq.1.gain_db", (-20.0, 20.0)) - 0.04).abs() < 1e-12);
        let decade = 10f64.ln() * 3.0 * 0.001;
        assert!((tolerance_of("eq.1.freq", (20.0, 20_000.0)) - decade).abs() < 1e-12);
    }

    #[test]
    fn a_time_is_put_on_the_nearest_tick_of_a_scale() {
        let at = |ticks, scale| Time { ticks, scale };
        assert_eq!(ticks_in(at(1, 3), 48000), Some(16000));
        assert_eq!(ticks_in(at(1, 7), 48000), Some(6857));
        assert_eq!(ticks_in(at(3, 7), 48000), Some(20571));
        assert_eq!(ticks_in(at(-1, 1), 48000), None);
        assert_eq!(ticks_in(at(1, 0), 48000), None);
    }
}
