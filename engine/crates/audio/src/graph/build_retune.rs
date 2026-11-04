//! A change of the mix that the graph playing it takes as it plays (plan T5): the settings that a
//! [`Control`] of the graph sets ([`Graph::control`]), sent to the thread of the mix and ramped
//! there; anything else needs another graph, built in the background and swapped in.
//!
//! [`retune`] tells the two apart. A change of a strip is taken as it plays when every setting
//! that differs is a number or a switch of the strip's lanes (the model's `automatable`: the
//! fader, the pan, the levels of the sends, the parameters of Dynamics, the equalizer and the
//! inserts) that the graph has a control of, and nothing else of the strip differs. A mute is a
//! control of its own; a solo, a solo-safe and the solo of a VCA change what every strip hears by
//! the solos ([`audible`]), sent as the control `solo` of each strip whose outcome changed. A
//! switch that builds or takes out a node (Dynamics or the equalizer turned on where the graph has
//! none, an insert added), the routing, the lanes, the formats, the VCAs that a strip belongs to,
//! a Listen: all need another graph. Of the clips, a fixed gain or pan changed to another fixed
//! one is a control of the clip (`clip.<id>.gain_db`, `clip.<id>.pan`); the rest of the clips is
//! not looked at here: the caller compares what they sound ([`without_clip_levels`], T5,
//! `play/sound.rs` of the server).
//!
//! [`Graph::control`]: super::Graph::control

use std::collections::BTreeMap;

use columbina_model::audio::automatable;
use columbina_model::{AudioParam, Strip, Timeline, TrackKind};
use serde_json::Value;

use super::{audible, Control, Controls, Owner};

/// A switch as [`super::Graph::set`] takes it.
fn switch(on: bool) -> f32 {
    if on {
        1.0
    } else {
        0.0
    }
}

/// The settings of `view` by the paths of their lanes, into `into`.
fn leaves(view: &Value, prefix: &str, into: &mut BTreeMap<String, Value>) {
    match view {
        Value::Object(object) => {
            for (key, inner) in object {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaves(inner, &path, into);
            }
        }
        other => {
            into.insert(prefix.to_owned(), other.clone());
        }
    }
}

/// A key of a JSON pointer (RFC 6901).
fn escaped(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Where the setting of the lane at `path` is in the JSON of `strip`, as a pointer.
fn pointer(strip: &Strip, path: &str) -> Option<String> {
    let field = |rest: &str, id: &str| {
        rest.strip_prefix(id)
            .and_then(|r| r.strip_prefix('.'))
            .map(|field| field.replace('.', "/"))
    };
    if let Some(rest) = path.strip_prefix("send.") {
        return strip.sends.iter().enumerate().find_map(|(at, send)| {
            field(rest, &send.bus).map(|field| format!("/sends/{at}/{field}"))
        });
    }
    if let Some(rest) = path.strip_prefix("output.") {
        return strip.outputs.iter().enumerate().find_map(|(at, output)| {
            field(rest, &output.bus).map(|field| format!("/outputs/{at}/{field}"))
        });
    }
    if let Some(rest) = path.strip_prefix("insert.") {
        return strip.inserts.iter().enumerate().find_map(|(at, insert)| {
            field(rest, &insert.id).map(|field| {
                if field == "on" {
                    format!("/inserts/{at}/on")
                } else {
                    format!("/inserts/{at}/effect/{field}")
                }
            })
        });
    }
    if let Some(rest) = path.strip_prefix("eq.") {
        return Some(match rest.split_once('.') {
            Some((band, field)) => {
                format!("/eq/bands/{}/{}", escaped(band), field.replace('.', "/"))
            }
            None => format!("/eq/{}", escaped(rest)),
        });
    }
    Some(format!("/{}", path.replace('.', "/")))
}

/// The settings of strip `owner` that differ from `was` to `now`, as controls, into `sets`; none
/// where anything else of the strip differs, or a setting that differs has no control.
fn strip(
    owner: &Owner,
    was: &Strip,
    now: &Strip,
    controls: &Controls,
    sets: &mut Vec<(Control, f32)>,
) -> Option<()> {
    if was == now {
        return Some(());
    }
    let (mut before, mut after) = (BTreeMap::new(), BTreeMap::new());
    leaves(&automatable(was), "", &mut before);
    leaves(&automatable(now), "", &mut after);
    if !before.keys().eq(after.keys()) {
        return None;
    }
    let mut was_json = serde_json::to_value(was).ok()?;
    let now_json = serde_json::to_value(now).ok()?;
    for (path, old) in &before {
        let new = &after[path];
        if old == new {
            continue;
        }
        let value = match (old, new) {
            (Value::Number(_), Value::Number(new)) => {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "a setting, as the graph takes it"
                )]
                let value = new.as_f64()? as f32;
                value
            }
            (Value::Bool(_), Value::Bool(new)) => switch(*new),
            _ => return None,
        };
        sets.push((controls.control(owner, path)?, value));
        let at = pointer(was, path)?;
        *was_json.pointer_mut(&at)? = now_json.pointer(&at)?.clone();
    }
    (was_json == now_json).then_some(())
}

/// `now` with the fixed gain and pan of each of its clips that has a fixed gain or pan in `was`
/// too as it is there: what the clips sound but for what [`retune`] sets as controls, for the
/// caller to compare with `was`.
#[must_use]
pub fn without_clip_levels(was: &Timeline, now: &Timeline) -> Timeline {
    let mut timeline = now.clone();
    for clip in timeline
        .tracks
        .iter_mut()
        .flat_map(|track| &mut track.clips)
    {
        let Some(before) = was
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|old| old.id == clip.id)
        else {
            continue;
        };
        if let (AudioParam::Fixed(_), AudioParam::Fixed(old)) = (&clip.gain_db, &before.gain_db) {
            clip.gain_db = AudioParam::Fixed(*old);
        }
        if let (AudioParam::Fixed(_), AudioParam::Fixed(old)) = (&clip.pan, &before.pan) {
            clip.pan = AudioParam::Fixed(*old);
        }
    }
    timeline
}

/// The tracks of sound of `timeline`.
fn sound(timeline: &Timeline) -> Vec<&columbina_model::Track> {
    timeline
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Audio)
        .collect()
}

/// What changes from `was` to `now` (the same timeline) as controls of the graph of `was` whose
/// `controls` these are, each with the value to set it to; none where the change needs another
/// graph. See the module.
#[must_use]
pub fn retune(was: &Timeline, now: &Timeline, controls: &Controls) -> Option<Vec<(Control, f32)>> {
    let mut sets = Vec::new();
    let mut solos = false;
    let (before, after) = (sound(was), sound(now));
    if before.len() != after.len() {
        return None;
    }
    for (a, b) in before.iter().zip(&after) {
        if a.id != b.id || a.enabled != b.enabled {
            return None;
        }
        if !a.enabled {
            // Not in the graph: nothing of it is heard, nor does it drive anything.
            continue;
        }
        let owner = Owner::Track(a.id.clone());
        let (x, y) = (&a.audio, &b.audio);
        let rest = |audio: &columbina_model::TrackAudio| {
            (
                audio.format,
                audio.channels,
                audio.order,
                audio.adc,
                audio.lanes.clone(),
            )
        };
        if rest(x) != rest(y) {
            return None;
        }
        if x.mute != y.mute {
            sets.push((controls.control(&owner, "mute")?, switch(y.mute)));
        }
        solos |= x.solo != y.solo || x.solo_safe != y.solo_safe;
        strip(&owner, &x.strip, &y.strip, controls, &mut sets)?;
        for after in &b.clips {
            let Some(before) = a.clips.iter().find(|clip| clip.id == after.id) else {
                continue;
            };
            for (field, old, new) in [
                ("gain_db", &before.gain_db, &after.gain_db),
                ("pan", &before.pan, &after.pan),
            ] {
                if let (AudioParam::Fixed(old), AudioParam::Fixed(new)) = (old, new) {
                    #[allow(clippy::float_cmp, reason = "a setting changed or not")]
                    if old != new {
                        let path = format!("clip.{}.{field}", after.id);
                        #[allow(clippy::cast_possible_truncation, reason = "a setting")]
                        sets.push((controls.control(&owner, &path)?, *new as f32));
                    }
                }
            }
        }
    }
    if was.buses.len() != now.buses.len() {
        return None;
    }
    for (a, b) in was.buses.iter().zip(&now.buses) {
        if (&a.id, a.format, &a.lanes) != (&b.id, b.format, &b.lanes) {
            return None;
        }
        let owner = Owner::Bus(a.id.clone());
        if a.mute != b.mute {
            sets.push((controls.control(&owner, "mute")?, switch(b.mute)));
        }
        solos |= a.solo != b.solo || a.solo_safe != b.solo_safe;
        strip(&owner, &a.strip, &b.strip, controls, &mut sets)?;
    }
    if was.vcas.len() != now.vcas.len() {
        return None;
    }
    for (a, b) in was.vcas.iter().zip(&now.vcas) {
        if (&a.id, &a.vcas, &a.lanes) != (&b.id, &b.vcas, &b.lanes) {
            return None;
        }
        let owner = Owner::Vca(a.id.clone());
        #[allow(clippy::float_cmp, reason = "a setting changed or not")]
        if a.fader_db != b.fader_db {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "a setting, as the graph takes it"
            )]
            let value = b.fader_db as f32;
            sets.push((controls.control(&owner, "fader_db")?, value));
        }
        if a.mute != b.mute {
            sets.push((controls.control(&owner, "mute")?, switch(b.mute)));
        }
        solos |= a.solo != b.solo;
    }
    if was.audio.monitor != now.audio.monitor {
        return None;
    }
    if solos {
        for ((owner, heard), (_, hears)) in audible(was).into_iter().zip(audible(now)) {
            if heard != hears {
                sets.push((controls.control(&owner, "solo")?, switch(hears)));
            }
        }
    }
    Some(sets)
}
