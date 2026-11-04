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

use super::param::{Lane, LaneKind, LanePoint, Lanes};
use super::routing::Vca;
use super::{automatable, Equalizer, Insert, Output, Send, Strip};
use crate::ops::{OpError, LARGEST};
use crate::time::Time;

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
            let mut view = automatable(strip);
            if let Some(object) = view.as_object_mut() {
                object.insert("mute".into(), mute.into());
            }
            view
        }
        Owner::Vca(vca) => serde_json::json!({ "fader_db": vca.fader_db, "mute": vca.mute }),
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

/// The place in the JSON of a strip of the setting at `path`, as `automatable` (`audio.rs`) lays it out.
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

/// Check the lanes of an owner: each of a setting that it has, of the kind of that setting, with
/// points (see [`Lane::is_valid`]) whose values the setting may be set to: from the least of
/// them to the greatest, or 0 and 1 for a switch.
///
/// # Errors
/// One is not: `out_of_range`, `lane`.
pub fn check_lanes(lanes: &Lanes, owner: Owner<'_>) -> Result<(), OpError> {
    if lanes.is_empty() {
        return Ok(());
    }
    let view = view(owner);
    let fine = |path: &str, lane: &Lane| {
        if !lane.is_valid() || kind_of(&view, path) != Some(lane.kind) {
            return false;
        }
        match lane.kind {
            LaneKind::Switch => lane.points.iter().all(|p| {
                p.value.to_bits() == 0f64.to_bits() || p.value.to_bits() == 1f64.to_bits()
            }),
            LaneKind::Continuous => {
                let values = lane.points.iter().map(|p| p.value);
                let least = values.clone().fold(f64::INFINITY, f64::min);
                let most = values.fold(f64::NEG_INFINITY, f64::max);
                fits(owner, path, least) && fits(owner, path, most)
            }
        }
    };
    if lanes.iter().all(|(path, lane)| fine(path, lane)) {
        Ok(())
    } else {
        Err(OpError::OutOfRange { what: "lane" })
    }
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

/// The value of a lane of `kind` with `points` (in ticks of one scale) at tick `at`, the later
/// of two points at one time holding from it; `fixed` where it has none. As [`Lane::value_at`].
#[must_use]
pub fn value_at(points: &[LanePoint], kind: LaneKind, fixed: f64, at: i64) -> f64 {
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
    between(&points[i], &points[i + 1], kind, at)
}

/// The value between two points at tick `at`, from the first.
fn between(from: &LanePoint, to: &LanePoint, kind: LaneKind, at: i64) -> f64 {
    match kind {
        LaneKind::Switch => from.value,
        LaneKind::Continuous => {
            let span = i128::from(to.at.ticks) - i128::from(from.at.ticks);
            if span == 0 {
                return to.value;
            }
            #[allow(clippy::cast_precision_loss, reason = "a share of the way")]
            let share =
                ((i128::from(at) - i128::from(from.at.ticks)) as f64 / span as f64).clamp(0.0, 1.0);
            from.value + (to.value - from.value) * share
        }
    }
}

/// The value of a lane just before tick `at`: what it comes to from the left, before any point
/// at `at` holds (p. 3898: what is before a range stays as it was).
#[must_use]
pub fn value_before(points: &[LanePoint], kind: LaneKind, fixed: f64, at: i64) -> f64 {
    let Some(first) = points.first() else {
        return fixed;
    };
    let i = points.partition_point(|p| p.at.ticks < at);
    if i == 0 {
        return first.value;
    }
    match (kind, points.get(i)) {
        (LaneKind::Continuous, Some(next)) => between(&points[i - 1], next, kind, at),
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
fn without_twins(points: Vec<LanePoint>) -> Vec<LanePoint> {
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

/// The points of a lane with those from tick `from` to tick `to`, both in, put in place of by
/// `new`: the value before `from` and the value from `to` on stay as they were, as a point at
/// `from` of the value that came to it and one at `to` of the value that was there (the bounding
/// keyframes of p. 3899; the ends of a range taken away keep their values, p. 3898). `new` must be
/// in order, from `from` to `to`.
#[must_use]
pub fn replaced(
    points: &[LanePoint],
    kind: LaneKind,
    fixed: f64,
    scale: u32,
    (from, to): (i64, i64),
    new: &[LanePoint],
) -> Vec<LanePoint> {
    let left = value_before(points, kind, fixed, from);
    let right = value_at(points, kind, fixed, to);
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

/// The points of a lane with the values from tick `from` to tick `to` moved by `delta`, through
/// `change` (which keeps a value within the range of its setting), the values outside the range
/// staying as they were (p. 3899): at each end a point of the value that was there and, `ramp`
/// ticks within, one of the value moved, so that it ramps to it (C-A6; a switch jumps: `ramp`
/// 0). The points between the two within move; those between the pairs go.
#[must_use]
pub fn shifted(
    points: &[LanePoint],
    kind: LaneKind,
    fixed: f64,
    scale: u32,
    (from, to): (i64, i64),
    ramp: i64,
    change: &dyn Fn(f64) -> f64,
) -> Vec<LanePoint> {
    let ramp = ramp.clamp(0, (to - from) / 2);
    let (inner_from, inner_to) = (from + ramp, to - ramp);
    let mut new = vec![point(
        inner_from,
        scale,
        change(value_at(points, kind, fixed, inner_from)),
    )];
    new.extend(
        points
            .iter()
            .filter(|p| p.at.ticks > inner_from && p.at.ticks < inner_to)
            .map(|p| point(p.at.ticks, scale, change(p.value))),
    );
    new.push(point(
        inner_to,
        scale,
        change(value_before(points, kind, fixed, inner_to)),
    ));
    replaced(points, kind, fixed, scale, (from, to), &new)
}

/// The points that Douglas and Peucker's thinning keeps of `points` (in order of time): the first,
/// the last, and between them those without which the lane would go further than `tolerance`
/// from one of them, in its value at its time (C-A4). Of a switch, where its value changes.
#[must_use]
pub fn thinned(points: &[LanePoint], kind: LaneKind, tolerance: f64) -> Vec<LanePoint> {
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
    match kind {
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
                // The point between that is furthest from the straight line from `a` to `b`,
                // which are at two times, since every point of a jump is kept.
                let (far, distance) = (a + 1..b)
                    .map(|i| {
                        let on = between(&points[a], &points[b], kind, points[i].at.ticks);
                        (i, (points[i].value - on).abs())
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
/// thinned (C-A4): 0.1 dB of a level in decibels, 0.5 % of the width of a pan (0.01), and 0.1 %
/// of the range `range` of any other setting.
#[must_use]
pub fn tolerance_of(path: &str, range: (f64, f64)) -> f64 {
    let last = path.rsplit('.').next().unwrap_or(path);
    if last.ends_with("_db") {
        0.1
    } else if path.contains("pan.") && matches!(last, "x" | "y") {
        0.01
    } else {
        let width = range.1 - range.0;
        if width.is_finite() && width > 0.0 {
            width * 0.001
        } else {
            0.001
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn a_range_taken_away_keeps_the_values_at_its_ends() {
        let lane = points(&[(0, -10.0), (1000, 0.0), (3000, -20.0)]);
        let kind = LaneKind::Continuous;
        let new = replaced(&lane, kind, 0.0, 1000, (500, 2000), &[]);
        assert_eq!(
            new,
            points(&[(0, -10.0), (500, -5.0), (2000, -10.0), (3000, -20.0)])
        );
        for at in [0, 250, 499, 2000, 2500, 3000] {
            let (old, now) = (
                value_at(&lane, kind, 0.0, at),
                value_at(&new, kind, 0.0, at),
            );
            assert!((old - now).abs() < 1e-12, "{at}: {old} {now}");
        }
        // A jump at the start of the range: what came to it stays.
        let jump = points(&[(0, 0.0), (500, -10.0), (500, 5.0), (1000, 5.0)]);
        let new = replaced(&jump, kind, 0.0, 1000, (500, 800), &[]);
        assert_eq!(
            new,
            points(&[(0, 0.0), (500, -10.0), (800, 5.0), (1000, 5.0)])
        );
    }

    #[test]
    fn a_range_moved_ramps_in_and_out_of_its_new_values() {
        let lane = points(&[(0, -10.0), (1000, -10.0)]);
        let kind = LaneKind::Continuous;
        let new = shifted(&lane, kind, 0.0, 1000, (200, 600), 10, &|v| v - 6.0);
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
        let new = shifted(&mute, LaneKind::Switch, 0.0, 1000, (200, 600), 0, &|v| {
            switched(v + 1.0)
        });
        assert_eq!(
            new,
            points(&[(0, 0.0), (200, 0.0), (200, 1.0), (600, 1.0), (600, 0.0)])
        );
        assert_eq!(value_at(&new, LaneKind::Switch, 0.0, 199), 0.0);
        assert_eq!(value_at(&new, LaneKind::Switch, 0.0, 200), 1.0);
        assert_eq!(value_at(&new, LaneKind::Switch, 0.0, 599), 1.0);
        assert_eq!(value_at(&new, LaneKind::Switch, 0.0, 600), 0.0);
    }

    #[test]
    fn thinning_keeps_what_bends_further_than_its_tolerance() {
        let ramp: Vec<LanePoint> = (0..=100)
            .map(|t| point(t, 1000, f64::from(i32::try_from(t).unwrap()) * -0.2))
            .collect();
        assert_eq!(
            thinned(&ramp, LaneKind::Continuous, 0.1),
            points(&[(0, 0.0), (100, -20.0)])
        );
        let bent = points(&[(0, 0.0), (10, 0.05), (20, 0.0), (30, 1.0), (40, 0.0)]);
        assert_eq!(
            thinned(&bent, LaneKind::Continuous, 0.1),
            points(&[(0, 0.0), (20, 0.0), (30, 1.0), (40, 0.0)])
        );
        let mute = points(&[(0, 0.0), (5, 0.0), (6, 1.0), (9, 1.0), (12, 0.0), (15, 0.0)]);
        assert_eq!(
            thinned(&mute, LaneKind::Switch, 0.0),
            points(&[(0, 0.0), (6, 1.0), (12, 0.0), (15, 0.0)])
        );
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
