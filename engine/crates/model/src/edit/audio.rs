//! The edits of sound: a setting of a track of sound and of the tracks grouped with it, the
//! keyframes of the volume and pan of a clip, the attributes of clips, a fade to the playhead,
//! and the effects of one chain copied to others. Each is worked out into the operations of
//! [`crate::ops`], as one transaction and one step of undo.

use std::collections::{BTreeSet, HashSet};

use serde_json::{json, Map, Value};

use super::work::{missing, Work};
use super::{Attribute, Edge, EditError, KeyAction, Keyed};
use crate::audio::param::{earlier, seconds};
use crate::audio::{
    AudioFade, AudioKey, AudioParam, Equalizer, GroupControl, KeyInterp, Pan, SILENT_DB,
};
use crate::ops::{inserts_of, InsertsOf, Op, OpError};
use crate::project::{Clip, Id, Timeline, Track, TrackKind};
use crate::time::Time;

/// The lowest and the highest gain of a clip, in decibels.
const GAIN: (f64, f64) = (-96.0, 24.0);

fn range(what: &'static str) -> EditError {
    EditError::Op(OpError::OutOfRange { what })
}

/// An operation that sets nothing of clip `id` yet.
fn set_clip(timeline: Id, id: &str) -> Op {
    Op::SetClip {
        timeline,
        id: id.into(),
        enabled: None,
        speed: None,
        reverse: None,
        link: None,
        transform: None,
        gain_db: None,
        pan: None,
        fade_in: None,
        fade_out: None,
        audio_fade_in: None,
        audio_fade_out: None,
        eq: None,
        effects: None,
        pitch_correction: None,
        pitch: None,
        channels: None,
        source: None,
    }
}

/// A clip that an edit changes, on a track that is not locked, and the id of its track.
fn open_clip(work: &Work, id: &str) -> Result<(Id, Clip), EditError> {
    let (track, clip) = work.owned(id)?;
    work.open(&track)?;
    Ok((track, clip))
}

/// A clip of sound that an edit of its sound changes, on a track that is not locked: one on a
/// track of picture is refused as `wrong_track` (`what: "sound"`).
fn sound_clip(work: &Work, id: &str) -> Result<(Id, Clip), EditError> {
    let (track, clip) = open_clip(work, id)?;
    let kind = work.track(&track)?.kind;
    if kind == TrackKind::Audio {
        Ok((track, clip))
    } else {
        Err(EditError::Op(OpError::WrongTrack {
            what: "sound",
            kind,
        }))
    }
}

/// The stretch of its media that a clip plays, from where it begins in it to where it ends there,
/// in the scale of its media: from its first frame's place to its last's, whichever way it plays.
fn source_range(clip: &Clip, rate: crate::time::Rate) -> (Time, Time) {
    let start = origin(clip);
    let span = crate::span::source_span(clip, rate, start.scale);
    (
        start,
        Time {
            ticks: start.ticks.saturating_add(span),
            scale: start.scale,
        },
    )
}

/// Refuse a time of a key that is outside the media that a clip plays, as `out_of_range` (`what`).
fn within_source(
    clip: &Clip,
    rate: crate::time::Rate,
    at: Time,
    what: &'static str,
) -> Result<(), EditError> {
    let (first, last) = source_range(clip, rate);
    if earlier(at, first) || earlier(last, at) {
        Err(range(what))
    } else {
        Ok(())
    }
}

// Tracks of sound, and the groups that link them.

/// What of a track's sound a group links, among those that changed from `before` to `after`.
fn controls_changed(before: &Track, after: &Track) -> HashSet<GroupControl> {
    let (b, a) = (&before.audio, &after.audio);
    let mut changed = HashSet::new();
    if b.strip.fader_db.to_bits() != a.strip.fader_db.to_bits() {
        changed.insert(GroupControl::Fader);
    }
    if b.mute != a.mute {
        changed.insert(GroupControl::Mute);
    }
    if b.solo != a.solo {
        changed.insert(GroupControl::Solo);
    }
    if b.strip.pan != a.strip.pan {
        changed.insert(GroupControl::Panning);
    }
    if b.strip.sends != a.strip.sends {
        changed.insert(GroupControl::Sends);
    }
    if b.automation_arm != a.automation_arm {
        changed.insert(GroupControl::Automation);
    }
    changed
}

/// The other tracks of sound that change with `track` (Ch. 179, p. 3917): each in an active group
/// with it, with what those groups link of what changed; each once, in the order of the tracks.
fn followers(
    timeline: &Timeline,
    track: &str,
    changed: &HashSet<GroupControl>,
) -> Vec<(Id, HashSet<GroupControl>)> {
    let mut found: Vec<(Id, HashSet<GroupControl>)> = vec![];
    for other in timeline.tracks.iter().filter(|r| r.id != track) {
        let mut linked = HashSet::new();
        for group in &timeline.track_groups {
            let together = group.active
                && group.members.iter().any(|m| m == track)
                && group.members.contains(&other.id);
            if together {
                linked.extend(group.linked.iter().filter(|c| changed.contains(c)));
            }
        }
        if !linked.is_empty() {
            found.push((other.id.clone(), linked));
        }
    }
    found
}

/// A level moved by `delta` decibels, as a fader of a group follows another (§4.2): no higher than
/// `top`, and no lower than -∞ (-100), where one that is there already stays as it is.
fn moved_level(level: f64, delta: f64, top: f64) -> f64 {
    if level <= SILENT_DB && delta <= 0.0 {
        return level;
    }
    (level + delta).clamp(SILENT_DB, top)
}

/// How far a level moved from `before` to `after`, in decibels, a level at -∞ (-100 or below)
/// counted as -100: so a fader brought up from -120 dB to -6 dB moved by 94 dB.
fn level_change(before: f64, after: f64) -> f64 {
    after.max(SILENT_DB) - before.max(SILENT_DB)
}

/// A setting of a pan moved as another moved from `before` to `after`, within `lo` and `hi`.
fn moved_by(value: f64, before: f64, after: f64, lo: f64, hi: f64) -> f64 {
    (value + (after - before)).clamp(lo, hi)
}

/// The pan of a follower, moved as the pan of the track that it follows moved: by the same
/// amount for each number, to the same value for each switch.
fn moved_pan(pan: &Pan, before: &Pan, after: &Pan) -> Pan {
    Pan {
        on: if before.on == after.on {
            pan.on
        } else {
            after.on
        },
        x: moved_by(pan.x, before.x, after.x, -1.0, 1.0),
        y: moved_by(pan.y, before.y, after.y, -1.0, 1.0),
        rotate: moved_by(pan.rotate, before.rotate, after.rotate, -180.0, 180.0),
        spread: moved_by(pan.spread, before.spread, after.spread, 1.0, 99.0),
        divergence: moved_by(
            pan.divergence,
            before.divergence,
            after.divergence,
            0.0,
            100.0,
        ),
        lfe_db: moved_level(pan.lfe_db, level_change(before.lfe_db, after.lfe_db), 10.0),
        lfe_on: if before.lfe_on == after.lfe_on {
            pan.lfe_on
        } else {
            after.lfe_on
        },
        lfe_pre: if before.lfe_pre == after.lfe_pre {
            pan.lfe_pre
        } else {
            after.lfe_pre
        },
    }
}

/// The patch that a follower is given, as the track that it follows changed from `before` to
/// `after`, of what `linked` links: a fader, a level of a send to a bus and a pan by the same
/// change, and a mute, a solo, the switch of a send and an arm to the same value. Only what
/// changes of the follower is in it.
fn follow(member: &Track, before: &Track, after: &Track, linked: &HashSet<GroupControl>) -> Value {
    let (b, a, m) = (&before.audio, &after.audio, &member.audio);
    let mut patch = Map::new();
    let mut strip = Map::new();
    if linked.contains(&GroupControl::Fader) {
        let fader = moved_level(
            m.strip.fader_db,
            level_change(b.strip.fader_db, a.strip.fader_db),
            10.0,
        );
        if fader.to_bits() != m.strip.fader_db.to_bits() {
            strip.insert("fader_db".into(), fader.into());
        }
    }
    if linked.contains(&GroupControl::Panning) {
        let pan = moved_pan(&m.strip.pan, &b.strip.pan, &a.strip.pan);
        if pan != m.strip.pan {
            strip.insert("pan".into(), json!(pan));
        }
    }
    if linked.contains(&GroupControl::Sends) {
        let sends: Vec<_> = m
            .strip
            .sends
            .iter()
            .map(|send| {
                let mut send = send.clone();
                let was = b.strip.sends.iter().find(|s| s.bus == send.bus);
                let now = a.strip.sends.iter().find(|s| s.bus == send.bus);
                if let (Some(was), Some(now)) = (was, now) {
                    send.level_db = moved_level(
                        send.level_db,
                        level_change(was.level_db, now.level_db),
                        10.0,
                    );
                    if was.on != now.on {
                        send.on = now.on;
                    }
                }
                send
            })
            .collect();
        if sends != m.strip.sends {
            strip.insert("sends".into(), json!(sends));
        }
    }
    if !strip.is_empty() {
        patch.insert("strip".into(), strip.into());
    }
    for (control, was, now, own, name) in [
        (GroupControl::Mute, b.mute, a.mute, m.mute, "mute"),
        (GroupControl::Solo, b.solo, a.solo, m.solo, "solo"),
    ] {
        if linked.contains(&control) && was != now && own != now {
            patch.insert(name.into(), now.into());
        }
    }
    let (was, now, own) = (b.automation_arm, a.automation_arm, m.automation_arm);
    if linked.contains(&GroupControl::Automation) && was != now && own != now {
        patch.insert("automation_arm".into(), json!(now));
    }
    patch.into()
}

/// Set what track `track` has of sound by `patch`, and with `follow_groups` what the tracks of
/// the active groups that it is in link of what changed (§4.2); and while the timeline's solo is
/// exclusive (p. 3885), take the solo off every other track and bus once one is soloed. Tracks
/// that are locked are left alone.
pub(super) fn set_track_audio(
    work: &mut Work,
    track: &str,
    patch: &Value,
    follow_groups: bool,
) -> Result<(), EditError> {
    let before = work.open(track)?.clone();
    let timeline = work.timeline_id();
    super::automation::strip_op(
        work,
        Op::SetTrackAudio {
            timeline: timeline.clone(),
            track: track.into(),
            patch: patch.clone(),
        },
    )?;
    let after = work.track(track)?.clone();
    let mut soloed: HashSet<Id> = HashSet::from([track.to_string()]);
    if follow_groups {
        let changed = controls_changed(&before, &after);
        for (id, linked) in followers(work.timeline(), track, &changed) {
            let member = work.track(&id)?;
            if member.locked || member.kind != TrackKind::Audio {
                continue;
            }
            let patch = follow(member, &before, &after, &linked);
            if linked.contains(&GroupControl::Solo) {
                soloed.insert(id.clone());
            }
            if patch.as_object().is_some_and(|p| !p.is_empty()) {
                super::automation::strip_op(
                    work,
                    Op::SetTrackAudio {
                        timeline: timeline.clone(),
                        track: id,
                        patch,
                    },
                )?;
            }
        }
    }
    if after.audio.solo && !before.audio.solo {
        exclusive_solo(work, &soloed, None)?;
    }
    Ok(())
}

/// While the solo of the timeline is exclusive (p. 3885), take the solo off every track that is
/// not locked but those of `tracks`, every bus but `bus`, and every VCA, once one is soloed. The
/// Solo Safe of a track stays: it is not a solo.
fn exclusive_solo(
    work: &mut Work,
    tracks: &HashSet<Id>,
    bus: Option<&str>,
) -> Result<(), EditError> {
    if !work.timeline().audio.exclusive_solo {
        return Ok(());
    }
    let timeline = work.timeline_id();
    let t = work.timeline();
    let others: Vec<Id> = t
        .tracks
        .iter()
        .filter(|r| r.kind == TrackKind::Audio && r.audio.solo && !r.locked)
        .filter(|r| !tracks.contains(&r.id))
        .map(|r| r.id.clone())
        .collect();
    let buses: Vec<Id> = t
        .buses
        .iter()
        .filter(|b| b.solo && Some(b.id.as_str()) != bus)
        .map(|b| b.id.clone())
        .collect();
    let vcas: Vec<Id> = t
        .vcas
        .iter()
        .filter(|v| v.solo)
        .map(|v| v.id.clone())
        .collect();
    let off = json!({ "solo": false });
    for id in others {
        work.op(Op::SetTrackAudio {
            timeline: timeline.clone(),
            track: id,
            patch: off.clone(),
        })?;
    }
    for id in buses {
        work.op(Op::SetBus {
            timeline: timeline.clone(),
            id,
            patch: off.clone(),
        })?;
    }
    for id in vcas {
        work.op(Op::SetVca {
            timeline: timeline.clone(),
            id,
            patch: off.clone(),
        })?;
    }
    Ok(())
}

/// Set a bus by a patch, as the operation `set_bus` does; and while the solo of the timeline is
/// exclusive, a bus that is soloed takes the solo off every track, other bus and VCA, as a track
/// does.
pub(super) fn set_bus(work: &mut Work, id: &str, patch: &Value) -> Result<(), EditError> {
    let was = work
        .timeline()
        .buses
        .iter()
        .find(|b| b.id == id)
        .is_some_and(|b| b.solo);
    super::automation::strip_op(
        work,
        Op::SetBus {
            timeline: work.timeline_id(),
            id: id.into(),
            patch: patch.clone(),
        },
    )?;
    let now = work
        .timeline()
        .buses
        .iter()
        .find(|b| b.id == id)
        .is_some_and(|b| b.solo);
    if now && !was {
        exclusive_solo(work, &HashSet::new(), Some(id))?;
    }
    Ok(())
}

// The keyframes of a clip.

/// Whether two times are one, counted exactly.
fn same(a: Time, b: Time) -> bool {
    !earlier(a, b) && !earlier(b, a)
}

fn param_of(clip: &Clip, param: Keyed) -> &AudioParam {
    match param {
        Keyed::GainDb => &clip.gain_db,
        Keyed::Pan => &clip.pan,
    }
}

/// The operation that sets `param` of clip `id` to `value`.
fn set_param(timeline: Id, id: &str, param: Keyed, value: AudioParam) -> Op {
    let mut op = set_clip(timeline, id);
    if let Op::SetClip { gain_db, pan, .. } = &mut op {
        match param {
            Keyed::GainDb => *gain_db = Some(value),
            Keyed::Pan => *pan = Some(value),
        }
    }
    op
}

/// Put a key among keys in the order of their times.
fn put_key(keys: &mut Vec<AudioKey>, key: AudioKey) {
    let at = keys.partition_point(|k| earlier(k.at, key.at));
    keys.insert(at, key);
}

fn key_name(at: Time) -> String {
    format!("{}/{}", at.ticks, at.scale)
}

/// Add, take away, move or ease a keyframe of the volume or the pan of a clip (pp. 3838–3840,
/// 970–972). A key is named by its time, a time of the media of the clip; one that is added
/// where there is one already takes its place, and one that is added with no value takes the
/// value that the setting has there. The last key taken away leaves its value, fixed.
#[allow(clippy::too_many_arguments, reason = "the fields of the edit")]
pub(super) fn clip_gain_keys(
    work: &mut Work,
    clip: &str,
    param: Keyed,
    action: KeyAction,
    at: Time,
    to: Option<Time>,
    value: Option<f64>,
    interp: Option<KeyInterp>,
) -> Result<(), EditError> {
    if at.scale == 0 || to.is_some_and(|to| to.scale == 0) {
        return Err(range("at"));
    }
    let (_, c) = sound_clip(work, clip)?;
    let rate = work.rate();
    match action {
        KeyAction::Add => within_source(&c, rate, at, "at")?,
        KeyAction::Move => {
            if let Some(to) = to {
                within_source(&c, rate, to, "to")?;
            }
        }
        KeyAction::Remove | KeyAction::SetInterp => {}
    }
    let old = param_of(&c, param);
    let mut keys = match old {
        AudioParam::Fixed(_) => vec![],
        AudioParam::Keys(keys) => keys.clone(),
    };
    let found = keys.iter().position(|k| same(k.at, at));
    let key_missing = || missing("key", &key_name(at));
    match action {
        KeyAction::Add => {
            let value = value.unwrap_or_else(|| old.value_at(seconds(at)));
            match found {
                Some(i) => {
                    keys[i].value = value;
                    if let Some(interp) = interp {
                        keys[i].interp = interp;
                    }
                }
                None => put_key(
                    &mut keys,
                    AudioKey {
                        at,
                        value,
                        interp: interp.unwrap_or_default(),
                    },
                ),
            }
        }
        KeyAction::Remove => {
            let removed = keys.remove(found.ok_or_else(key_missing)?);
            if keys.is_empty() {
                let op = set_param(work.timeline_id(), clip, param, removed.value.into());
                work.op(op)?;
                return Ok(());
            }
        }
        KeyAction::Move => {
            let i = found.ok_or_else(key_missing)?;
            let to = to.ok_or_else(|| range("to"))?;
            if keys
                .iter()
                .enumerate()
                .any(|(j, k)| j != i && same(k.at, to))
            {
                return Err(range("to"));
            }
            let mut key = keys.remove(i);
            key.at = to;
            if let Some(value) = value {
                key.value = value;
            }
            put_key(&mut keys, key);
        }
        KeyAction::SetInterp => {
            let i = found.ok_or_else(key_missing)?;
            keys[i].interp = interp.ok_or_else(|| range("interp"))?;
        }
    }
    let op = set_param(work.timeline_id(), clip, param, AudioParam::Keys(keys));
    work.op(op)?;
    Ok(())
}

/// The sample of `rate` nearest to time `at`.
fn sample_of(at: Time, rate: u32) -> i64 {
    let (ticks, scale) = (i128::from(at.ticks), i128::from(at.scale));
    let twice = ticks * i128::from(rate) * 2;
    i64::try_from((twice + scale).div_euclid(2 * scale)).unwrap_or(i64::MAX)
}

/// A gain within its range; and whether it had to be brought into it.
fn clamped_gain(db: f64) -> (f64, bool) {
    let clamped = db.clamp(GAIN.0, GAIN.1);
    (clamped, clamped.to_bits() != db.to_bits())
}

/// Raise or lower the volume of a clip between two times of its media by `delta_db` (p. 3840):
/// keys at the samples of the timeline nearest to them, a pair at each end, the outer one at the
/// value that was there and the inner one at that value moved, one sample apart; and every key
/// between them moved by as much. A value that would leave the range of a gain is kept at its
/// end, and the edit says that it was clamped.
pub(super) fn clip_gain_range(
    work: &mut Work,
    clip: &str,
    from: Time,
    to: Time,
    delta_db: f64,
) -> Result<(), EditError> {
    if !delta_db.is_finite() {
        return Err(range("gain_db"));
    }
    if from.scale == 0 || to.scale == 0 {
        return Err(range("at"));
    }
    let (_, c) = sound_clip(work, clip)?;
    within_source(&c, work.rate(), from, "from")?;
    within_source(&c, work.rate(), to, "to")?;
    let rate = work.timeline().sample_rate;
    let (a, b) = (sample_of(from, rate), sample_of(to, rate));
    if a >= b || delta_db == 0.0 {
        return Err(EditError::EmptyRange);
    }
    let time = |sample: i64| Time {
        ticks: sample,
        scale: rate,
    };
    let (outer_a, inner_a, inner_b, outer_b) = (time(a - 1), time(a), time(b - 1), time(b));
    let old = &c.gain_db;
    let mut clamped = false;
    let mut moved = |db: f64| {
        let (db, was) = clamped_gain(db + delta_db);
        clamped |= was;
        db
    };
    let existing: &[AudioKey] = match old {
        AudioParam::Keys(keys) => keys,
        AudioParam::Fixed(_) => &[],
    };
    // A key that is replaced keeps its interpolation; an outer key that splits a segment takes
    // the ease that the segment has at its side: the ease into the key after `outer_a`, and the
    // ease out of the key before `outer_b`. Inner keys are linear.
    let interp_of = |time: Time, outer: Option<bool>| {
        if let Some(key) = existing.iter().find(|k| same(k.at, time)) {
            return key.interp;
        }
        match outer {
            Some(true) => existing
                .iter()
                .find(|k| earlier(time, k.at))
                .filter(|k| k.interp == KeyInterp::EaseIn)
                .map_or(KeyInterp::Linear, |k| k.interp),
            Some(false) => existing
                .iter()
                .rev()
                .find(|k| earlier(k.at, time))
                .filter(|k| k.interp == KeyInterp::EaseOut)
                .map_or(KeyInterp::Linear, |k| k.interp),
            None => KeyInterp::Linear,
        }
    };
    let at = |time: Time, value: f64, outer: Option<bool>| AudioKey {
        at: time,
        value,
        interp: interp_of(time, outer),
    };
    let mut keys = vec![
        at(outer_a, old.value_at(seconds(outer_a)), Some(true)),
        at(inner_a, moved(old.value_at(seconds(inner_a))), None),
        at(outer_b, old.value_at(seconds(outer_b)), Some(false)),
    ];
    if inner_b != inner_a {
        keys.push(at(inner_b, moved(old.value_at(seconds(inner_b))), None));
    }
    {
        for key in existing {
            let outside = earlier(key.at, outer_a) || earlier(outer_b, key.at);
            let inside = earlier(inner_a, key.at) && earlier(key.at, inner_b);
            if outside {
                keys.push(key.clone());
            } else if inside {
                keys.push(AudioKey {
                    value: moved(key.value),
                    ..key.clone()
                });
            }
        }
    }
    keys.sort_by(|x, y| {
        if earlier(x.at, y.at) {
            std::cmp::Ordering::Less
        } else if earlier(y.at, x.at) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    if clamped {
        work.clamp();
    }
    let op = set_param(
        work.timeline_id(),
        clip,
        Keyed::GainDb,
        AudioParam::Keys(keys),
    );
    work.op(op)?;
    Ok(())
}

/// A volume moved by `delta_db`, its keys and all; and whether a value had to be kept within the
/// range of a gain.
fn nudged(gain: &AudioParam, delta_db: f64) -> (AudioParam, bool) {
    let mut clamped = false;
    let mut moved = |db: f64| {
        let (db, was) = clamped_gain(db + delta_db);
        clamped |= was;
        db
    };
    let gain = match gain {
        AudioParam::Fixed(db) => AudioParam::Fixed(moved(*db)),
        AudioParam::Keys(keys) => AudioParam::Keys(
            keys.iter()
                .map(|k| AudioKey {
                    value: moved(k.value),
                    ..k.clone()
                })
                .collect(),
        ),
    };
    (gain, clamped)
}

/// Raise or lower the volume of clips of sound by `delta_db`, keys and all (±1 and ±3 dB from the
/// keyboard, p. 969; a relative drag of several, p. 3837), with the clips linked to them unless
/// `linked` is false; clips of picture among them are passed over.
pub(super) fn nudge_clip_gain(
    work: &mut Work,
    clips: &[Id],
    delta_db: f64,
    linked: bool,
) -> Result<(), EditError> {
    if !delta_db.is_finite() {
        return Err(range("gain_db"));
    }
    if delta_db == 0.0 {
        return Err(EditError::EmptyRange);
    }
    let mut any = false;
    for id in work.group(clips, linked)? {
        let (track, clip) = work.owned(&id)?;
        if work.track(&track)?.kind != TrackKind::Audio {
            continue;
        }
        any = true;
        let (gain, clamped) = nudged(&clip.gain_db, delta_db);
        if clamped {
            work.clamp();
        }
        if gain != clip.gain_db {
            let op = set_param(work.timeline_id(), &id, Keyed::GainDb, gain);
            work.op(op)?;
        }
    }
    if any {
        Ok(())
    } else {
        Err(range("clips"))
    }
}

/// `at` - `from` + `to`, exactly, in the smallest scale that holds it; none where that scale does
/// not fit in 32 bits or the ticks are further than [`crate::ops::LARGEST`] from 0.
fn shifted(at: Time, from: Time, to: Time) -> Option<Time> {
    fn gcd(mut a: i128, mut b: i128) -> i128 {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a.abs()
    }
    let lcm = |a: i128, b: i128| a / gcd(a, b) * b;
    let scale = lcm(
        lcm(i128::from(at.scale), i128::from(from.scale)),
        i128::from(to.scale),
    );
    let ticks_of = |t: Time| i128::from(t.ticks) * (scale / i128::from(t.scale));
    let ticks = ticks_of(at) - ticks_of(from) + ticks_of(to);
    let common = gcd(ticks, scale).max(1);
    let (ticks, scale) = (ticks / common, scale / common);
    let fits = ticks.unsigned_abs() <= u128::from(crate::ops::LARGEST.unsigned_abs());
    Some(Time {
        ticks: i64::try_from(ticks).ok().filter(|_| fits)?,
        scale: u32::try_from(scale).ok()?,
    })
}

/// The start of a clip in its media: where it begins in it, or 0 for a title or a colour.
fn origin(clip: &Clip) -> Time {
    clip.source_in().unwrap_or(Time { ticks: 0, scale: 1 })
}

/// `a` + `b` - `at`, exactly, as [`shifted`] gives `at` - `a` + `b`.
fn mirrored(at: Time, a: Time, b: Time) -> Option<Time> {
    let zero = Time { ticks: 0, scale: 1 };
    let sum = shifted(a, zero, b)?;
    let negated = Time {
        ticks: at.ticks.checked_neg()?,
        scale: at.scale,
    };
    shifted(negated, zero, sum)
}

/// The place in its media that a clip plays at its start on the timeline: where it begins in it,
/// or, for one that plays backward, where it ends there.
fn timeline_origin(clip: &Clip, rate: crate::time::Rate) -> Time {
    let (first, last) = source_range(clip, rate);
    if clip.reverse {
        last
    } else {
        first
    }
}

/// The volume of `from` for clip `to`: each key as far from where `to` starts on the timeline,
/// in its media, as it is from where `from` starts, whichever way each plays; so the envelope is
/// heard at the same places of each clip.
fn volume_for(from: &Clip, to: &Clip, rate: crate::time::Rate) -> Result<AudioParam, EditError> {
    let (a, b) = (timeline_origin(from, rate), timeline_origin(to, rate));
    let mirror = from.reverse != to.reverse;
    match &from.gain_db {
        AudioParam::Keys(keys) if mirror || !same(a, b) => {
            let mut moved = keys
                .iter()
                .map(|k| {
                    let at = if mirror {
                        mirrored(k.at, a, b)
                    } else {
                        shifted(k.at, a, b)
                    };
                    let at = at.ok_or(OpError::TooLarge { what: "at" })?;
                    Ok(AudioKey { at, ..k.clone() })
                })
                .collect::<Result<Vec<_>, EditError>>()?;
            if mirror {
                // The other way round, the keys are in the other order, and an ease into a key
                // is an ease out of it.
                moved.reverse();
                for key in &mut moved {
                    key.interp = match key.interp {
                        KeyInterp::EaseIn => KeyInterp::EaseOut,
                        KeyInterp::EaseOut => KeyInterp::EaseIn,
                        KeyInterp::Linear => KeyInterp::Linear,
                    };
                }
            }
            Ok(AudioParam::Keys(moved))
        }
        gain => Ok(gain.clone()),
    }
}

/// The ids of a list, each once, in their order.
fn once(ids: &[Id]) -> Vec<Id> {
    let mut seen = HashSet::new();
    ids.iter().filter(|id| seen.insert(*id)).cloned().collect()
}

/// Paste the attributes of a clip onto others (pp. 3803, 3842): its volume, keys and all, at the
/// same places from where each begins in its media; its effects (Plugins); its equalizer.
pub(super) fn paste_attributes(
    work: &mut Work,
    from: &str,
    to: &[Id],
    items: &[Attribute],
) -> Result<(), EditError> {
    if items.is_empty() {
        return Err(range("items"));
    }
    if to.is_empty() {
        return Err(range("clips"));
    }
    let (track, source) = work.owned(from)?;
    let kind = work.track(&track)?.kind;
    if kind != TrackKind::Audio {
        return Err(EditError::Op(OpError::WrongTrack {
            what: "sound",
            kind,
        }));
    }
    let rate = work.rate();
    let items: BTreeSet<Attribute> = items.iter().copied().collect();
    for id in once(to) {
        let (_, clip) = sound_clip(work, &id)?;
        let mut op = set_clip(work.timeline_id(), &id);
        if let Op::SetClip {
            gain_db,
            effects,
            eq,
            ..
        } = &mut op
        {
            if items.contains(&Attribute::Volume) {
                *gain_db = Some(volume_for(&source, &clip, rate)?);
            }
            if items.contains(&Attribute::Plugins) {
                *effects = Some(source.effects.clone());
            }
            if items.contains(&Attribute::Equalizer) {
                *eq = Some(source.eq.clone());
            }
        }
        work.op(op)?;
    }
    Ok(())
}

/// Put the attributes of clips back as a new clip has them (pp. 3804, 3842): its volume at 0 dB
/// with no keys, no effects, and an equalizer that is off. A clip that has them so already is
/// left as it is.
pub(super) fn remove_attributes(
    work: &mut Work,
    clips: &[Id],
    items: &[Attribute],
) -> Result<(), EditError> {
    if items.is_empty() {
        return Err(range("items"));
    }
    if clips.is_empty() {
        return Err(range("clips"));
    }
    let items: BTreeSet<Attribute> = items.iter().copied().collect();
    for id in once(clips) {
        let (_, clip) = sound_clip(work, &id)?;
        let mut op = set_clip(work.timeline_id(), &id);
        let mut any = false;
        if let Op::SetClip {
            gain_db,
            effects,
            eq,
            ..
        } = &mut op
        {
            let none = AudioParam::Fixed(0.0);
            if items.contains(&Attribute::Volume) && clip.gain_db != none {
                *gain_db = Some(none);
                any = true;
            }
            if items.contains(&Attribute::Plugins) && !clip.effects.is_empty() {
                *effects = Some(vec![]);
                any = true;
            }
            let off = Equalizer::clip();
            if items.contains(&Attribute::Equalizer) && clip.eq != off {
                *eq = Some(off);
                any = true;
            }
        }
        if any {
            work.op(op)?;
        }
    }
    Ok(())
}

/// Fade clips in from their start to the playhead, or out from the playhead to their end (Fade In
/// to Playhead, Fade Out to Playhead, p. 3812), in one step: over frames of a clip of picture,
/// and over that time of a clip of sound, keeping the curve of the fade that it has. The
/// playhead, frame `at`, is after the first frame of each clip and before its end; a fade that
/// would meet the other fade of its clip is refused (`out_of_range`, `fade`).
pub(super) fn fade_to_playhead(
    work: &mut Work,
    clips: &[Id],
    side: Edge,
    at: i64,
) -> Result<(), EditError> {
    if clips.is_empty() {
        return Err(range("clips"));
    }
    for clip in once(clips) {
        fade_one_to_playhead(work, &clip, side, at)?;
    }
    Ok(())
}

fn fade_one_to_playhead(work: &mut Work, clip: &str, side: Edge, at: i64) -> Result<(), EditError> {
    let (track, c) = open_clip(work, clip)?;
    if at <= c.start || at >= c.end() {
        return Err(EditError::NothingAt {
            track: Some(track),
            frame: at,
        });
    }
    let frames = match side {
        Edge::In => at - c.start,
        Edge::Out => c.end() - at,
    };
    let rate = work.rate();
    let kind = work.track(&track)?.kind;
    let mut op = set_clip(work.timeline_id(), clip);
    if let Op::SetClip {
        fade_in,
        fade_out,
        audio_fade_in,
        audio_fade_out,
        ..
    } = &mut op
    {
        if kind == TrackKind::Audio {
            let had = match side {
                Edge::In => c.audio_fade_in,
                Edge::Out => c.audio_fade_out,
            };
            let ticks = i64::try_from(i128::from(frames) * i128::from(rate.den))
                .map_err(|_| OpError::TooLarge { what: "fade" })?;
            let fade = AudioFade {
                length: Time {
                    ticks,
                    scale: rate.num,
                },
                curve: had.map_or(0.0, |f| f.curve),
                skew: had.map_or(0.0, |f| f.skew),
            };
            match side {
                Edge::In => *audio_fade_in = Some(Some(fade)),
                Edge::Out => *audio_fade_out = Some(Some(fade)),
            }
        } else {
            match side {
                Edge::In => *fade_in = Some(frames),
                Edge::Out => *fade_out = Some(frames),
            }
        }
    }
    work.op(op)?;
    Ok(())
}

/// Copy the inserts of one chain onto others, in place of all that each has (pp. 3855–3860): a
/// strip of a track of sound or a bus, or the effects of a clip. Only the inserts are copied, not
/// Dynamics nor the equalizer, each as a new insert, which a strip gives the next ids of its own.
/// A chain that is the one copied is passed over.
pub(super) fn copy_effects(
    work: &mut Work,
    from: &InsertsOf,
    to: &[InsertsOf],
) -> Result<(), EditError> {
    if to.is_empty() {
        return Err(range("to"));
    }
    let timeline = work.timeline_id();
    // A copy is a new insert: a strip gives it the next id of its own.
    let inserts: Vec<_> = inserts_of(work.project(), &timeline, from)?
        .iter()
        .map(|insert| crate::audio::Insert {
            id: String::new(),
            ..insert.clone()
        })
        .collect();
    let mut seen = HashSet::new();
    for target in to {
        if target == from || !seen.insert(target.clone()) {
            continue;
        }
        // The copies are new inserts, which may be given the ids of those that they replace:
        // no lane of those is left to be taken for theirs.
        if !matches!(target, InsertsOf::Clip(_)) {
            if let InsertsOf::Track(id) = target {
                work.open(id)?;
            }
            super::automation::take_all_insert_lanes_away(work, target)?;
        }
        let op = match target {
            InsertsOf::Track(id) => {
                work.open(id)?;
                Op::SetTrackAudio {
                    timeline: timeline.clone(),
                    track: id.clone(),
                    patch: json!({ "strip": { "inserts": inserts } }),
                }
            }
            InsertsOf::Bus(id) => Op::SetBus {
                timeline: timeline.clone(),
                id: id.clone(),
                patch: json!({ "strip": { "inserts": inserts } }),
            },
            InsertsOf::Clip(id) => {
                open_clip(work, id)?;
                let mut op = set_clip(timeline.clone(), id);
                if let Op::SetClip { effects, .. } = &mut op {
                    *effects = Some(inserts.clone());
                }
                op
            }
        };
        super::automation::strip_op(work, op)?;
    }
    Ok(())
}
