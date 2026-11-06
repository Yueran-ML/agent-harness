//! The makings of the operations on sound (schema 4): patches of tracks, buses, VCAs, groups of
//! tracks and timelines; buses, VCAs and groups added and taken away; inserts of strips and the
//! effects of clips; and the sound of clips. Each names things of the project most of the time,
//! and now and then something that is not there, or settings that are refused.

use columbina_model::audio::fx::{GainFx, LimiterFx};
use columbina_model::{
    ChannelMap, ChannelSource, ChannelSources, ClipPitch, Effect, Equalizer, Insert, InsertsOf, Op,
    Project, Timeline, TrackKind,
};
use serde_json::{json, Value};

/// One of `ids`, or one time in sixteen `bogus`, which names nothing.
fn pick(ids: &[String], n: usize, bogus: &str) -> String {
    if n.is_multiple_of(16) || ids.is_empty() {
        bogus.into()
    } else {
        ids[n / 16 % ids.len()].clone()
    }
}

fn sound_tracks(timeline: Option<&Timeline>) -> Vec<String> {
    timeline
        .map(|t| {
            t.tracks
                .iter()
                .filter(|r| r.kind == TrackKind::Audio)
                .map(|r| r.id.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn ids_of<T>(list: Option<&Vec<T>>, id: impl Fn(&T) -> &String) -> Vec<String> {
    list.map(|l| l.iter().map(|x| id(x).clone()).collect())
        .unwrap_or_default()
}

/// An insert: a Gain, or a Limiter whose sidechain is on (which a clip refuses), named or not.
fn insert(n: usize) -> Insert {
    Insert {
        id: match n % 5 {
            0 => "fx1".into(),
            1 => "fx9".into(),
            _ => String::new(),
        },
        on: !n.is_multiple_of(3),
        effect: if n.is_multiple_of(4) {
            let mut limiter = LimiterFx::default();
            limiter.sidechain.on = n.is_multiple_of(8);
            Effect::Limiter(limiter)
        } else {
            #[allow(clippy::cast_precision_loss, reason = "a small number")]
            let gain_db = (n % 49) as f64 - 24.0;
            Effect::Gain(GainFx { gain_db })
        },
    }
}

/// A patch of a track of sound.
fn track_patch(n: usize, m: usize, buses: &[String], vcas: &[String], tracks: &[String]) -> Value {
    match n % 13 {
        0 => json!({"strip": {"fader_db": ([-6.0, 4.5, 11.0, -150.0][m % 4])}}),
        1 => json!({"mute": m.is_multiple_of(2), "solo": m.is_multiple_of(3)}),
        2 => {
            json!({"solo_safe": m.is_multiple_of(2), "adc": m.is_multiple_of(3), "automation_arm": true})
        }
        3 => json!({"format": (["mono", "stereo", "5.1", "adaptive"][m % 4])}),
        4 => json!({"format": "adaptive", "channels": ([1, 3, 24, 25][m % 4])}),
        5 => json!({"strip": {"pan": {"x": ([-0.5, 1.0, 2.0][m % 3]), "spread": 50}}}),
        6 => json!({"strip": {"outputs": [{"bus": pick(buses, m, "B99")}]}}),
        7 => json!({"strip": {"sends": [{"bus": pick(buses, m, "B99"), "level_db": -12}]}}),
        8 => json!({"strip": {"vcas": [pick(vcas, m, "VCA99")]}}),
        9 => {
            // A Ducker or the sidechain of the compressor, driven by a track or a bus.
            let source = if m.is_multiple_of(2) {
                json!({"track": pick(tracks, m, "A99")})
            } else {
                json!({"bus": pick(buses, m, "B99")})
            };
            if m % 3 == 1 {
                json!({"strip": {"dynamics": {"compressor": {"sidechain": {"on": true, "source": source}}}}})
            } else {
                json!({"strip": {"track_fx": {"sources": [source]}}})
            }
        }
        10 => json!({"strip": {"inserts": [insert(m), insert(m / 7)]}}),
        11 => json!({"strip": {"order": ([["eq", "fx", "dynamics"], ["fx", "fx", "eq"]][m % 2])}}),
        _ => [json!(1), json!({"lanes": {}}), json!({"strip": {"x": 1}})][m % 3].clone(),
    }
}

/// A bus, named by the engine or as one that may be held.
fn bus(n: usize, m: usize, buses: &[String]) -> Value {
    let outputs = if m.is_multiple_of(3) {
        json!([])
    } else {
        json!([{"bus": pick(buses, m, "B99")}])
    };
    json!({"id": (["", "", "B1", "B30", "A1", "$b"][n % 6]), "name": "Bus",
        "color": "#336699ff", "format": (["stereo", "5.1", "mono", "7.1"][m % 4]),
        "strip": {"outputs": outputs, "inserts": [insert(n)]}})
}

/// The operation on sound that `n` makes against `project`.
#[allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    reason = "an arm for each operation, of the four numbers of a making"
)]
pub fn sound_op_of(project: &Project, n: [usize; 4]) -> Op {
    let [a, b, c, d] = n;
    let timeline_ids: Vec<String> = project.timelines.iter().map(|t| t.id.clone()).collect();
    let timeline_id = pick(&timeline_ids, a, "t99");
    let timeline = project.timelines.iter().find(|t| t.id == timeline_id);
    let tracks = sound_tracks(timeline);
    let buses = ids_of(timeline.map(|t| &t.buses), |x| &x.id);
    let vcas = ids_of(timeline.map(|t| &t.vcas), |x| &x.id);
    let groups = ids_of(timeline.map(|t| &t.track_groups), |x| &x.id);
    let clips: Vec<String> = timeline
        .map(|t| {
            t.tracks
                .iter()
                .flat_map(|r| &r.clips)
                .map(|c| c.id.clone())
                .collect()
        })
        .unwrap_or_default();
    let of = match c % 3 {
        0 => InsertsOf::Track(pick(&tracks, b, "V1")),
        1 => InsertsOf::Bus(pick(&buses, b, "B99")),
        _ => InsertsOf::Clip(pick(&clips, b, "c99")),
    };
    let op = |value: Value| -> Op { serde_json::from_value(value).unwrap() };
    let t = timeline_id.clone();
    match d % 19 {
        0 | 1 => op(json!({"op": "set_track_audio", "timeline": t,
            "track": pick(&tracks, b, "V1"), "patch": track_patch(c, d / 16, &buses, &vcas, &tracks)})),
        2 => op(
            json!({"op": "add_bus", "timeline": t, "bus": bus(b, c, &buses),
            "index": ([None, Some(0), Some(99)][c % 3])}),
        ),
        3 => op(json!({"op": "remove_bus", "timeline": t, "id": pick(&buses, b, "B99")})),
        4 => {
            let patch = match c % 5 {
                0 => json!({"strip": {"outputs": [{"bus": pick(&buses, d, "B99")}]}}),
                1 => json!({"name": "Renamed", "format": "5.1", "mute": true}),
                2 => json!({"solo": true, "solo_safe": c.is_multiple_of(2), "color": "#ff0000ff"}),
                3 => json!({"strip": {"fader_db": -3, "track_fx": {}}}),
                _ => json!({"id": "B5"}),
            };
            op(
                json!({"op": "set_bus", "timeline": t, "id": pick(&buses, b, "B99"), "patch": patch}),
            )
        }
        5 => op(
            json!({"op": "add_vca", "timeline": t, "vca": {"id": (["", "VCA1", "VCA7"][b % 3]),
            "name": "VCA", "fader_db": ([0.0, -4.0, 12.0][c % 3]),
            "vcas": if c.is_multiple_of(2) { json!([]) } else { json!([pick(&vcas, d, "VCA99")]) }}}),
        ),
        6 => op(json!({"op": "remove_vca", "timeline": t, "id": pick(&vcas, b, "VCA99")})),
        7 => {
            let patch = match c % 3 {
                0 => json!({"vcas": [pick(&vcas, d, "VCA99")]}),
                1 => json!({"fader_db": -2.5, "mute": true, "name": "Strings"}),
                _ => json!({"lanes": {}}),
            };
            op(
                json!({"op": "set_vca", "timeline": t, "id": pick(&vcas, b, "VCA99"), "patch": patch}),
            )
        }
        8 => {
            let members: Vec<String> = (0..=c % 3)
                .map(|k| pick(&tracks, d + k * 17, "A99"))
                .collect();
            op(
                json!({"op": "add_track_group", "timeline": t, "group": {"id": (["", "TG1"][b % 2]),
                "name": "Group", "members": members, "active": !c.is_multiple_of(4),
                "linked": ([json!(["fader", "mute"]), json!(["solo", "solo"]), json!(["panning", "sends", "automation", "editing"])][d % 3])}}),
            )
        }
        9 => op(json!({"op": "remove_track_group", "timeline": t, "id": pick(&groups, b, "TG99")})),
        10 => {
            let patch = match c % 3 {
                0 => json!({"active": d.is_multiple_of(2), "linked": ["fader"]}),
                1 => json!({"members": [pick(&tracks, d, "A99")]}),
                _ => json!({"name": "Renamed", "color": "#00ff00ff"}),
            };
            op(
                json!({"op": "set_track_group", "timeline": t, "id": pick(&groups, b, "TG99"), "patch": patch}),
            )
        }
        11 => Op::AddInsert {
            timeline: t,
            of,
            insert: insert(d / 16),
            index: [None, Some(0), Some(1), Some(9)][a % 4],
        },
        12 => Op::SetInsert {
            timeline: t,
            of,
            slot: d / 16 % 4,
            insert: (!a.is_multiple_of(3)).then(|| insert(a)),
        },
        13 => Op::MoveInsert {
            timeline: t,
            of,
            from: d / 16 % 3,
            to: a % 3,
        },
        14 => {
            let patch = match c % 4 {
                0 => json!({"exclusive_solo": d.is_multiple_of(2), "follows_edit": false}),
                1 => {
                    json!({"monitor": if d.is_multiple_of(3) { Value::Null } else { json!(pick(&buses, d, "B99")) }})
                }
                2 => json!({"loudness": {"target": ([-14, -23, -40][d % 3])}, "meter": "vu"}),
                _ => json!({"record": {"mode": "trim", "touch": "snap"}}),
            };
            op(json!({"op": "set_timeline_audio", "timeline": t, "patch": patch}))
        }
        16 | 17 => lane_op(timeline, &t, [a, b, c, d / 19]),
        _ => set_sound_of_clip(project, &t, &pick(&clips, b, "c99"), [a, c, d]),
    }
}

/// The paths that lanes are made of: of settings that the strips of `tidy` have, of settings that
/// a strip may have (a send, an insert), of a setting of a switch, and of nothing.
const PATHS: [&str; 10] = [
    "fader_db",
    "mute",
    "pan.x",
    "eq.2.gain_db",
    "send.B2.level_db",
    "insert.fx1.gain_db",
    "insert.fx1.on",
    "dynamics.compressor.ratio",
    "eq.9.freq",
    "order",
];

/// A lane of one to four points at times of 48 kHz, of `kind`; now and then out of order, of a
/// value out of range, or of a switch that is neither on nor off.
fn lane_of(n: usize, kind: &str) -> Value {
    let count = 1 + n % 4;
    let points: Vec<Value> = (0..count)
        .map(|k| {
            let ticks = if n % 23 == 7 {
                (count - k) * 4800
            } else {
                (k + n % 3) * 4800
            };
            let value = match (kind, (n / 4 + k) % 9) {
                ("switch", 8) => json!(0.5),
                ("switch", v) => json!(v % 2),
                (_, 8) => json!(40.0),
                (_, v) => json!(-0.25 * f64::from(u8::try_from(v).unwrap())),
            };
            json!({"at": {"ticks": ticks, "scale": 48000}, "value": value})
        })
        .collect();
    json!({"points": points, "kind": kind})
}

/// An operation on a lane: one set or taken away, or points of one spliced, of a track of sound,
/// a bus or a VCA of the timeline, or of what is not there.
#[allow(
    clippy::many_single_char_names,
    reason = "the four numbers of a making"
)]
fn lane_op(timeline: Option<&Timeline>, t: &str, n: [usize; 4]) -> Op {
    let [a, b, c, d] = n;
    let tracks = sound_tracks(timeline);
    let buses = ids_of(timeline.map(|t| &t.buses), |x| &x.id);
    let vcas = ids_of(timeline.map(|t| &t.vcas), |x| &x.id);
    let (of, lanes) = match a % 3 {
        0 => {
            let id = pick(&tracks, b, "V1");
            let lanes = timeline
                .and_then(|t| t.tracks.iter().find(|r| r.id == id))
                .map(|r| r.audio.lanes.clone());
            (json!({"track": id}), lanes)
        }
        1 => {
            let id = pick(&buses, b, "B99");
            let lanes = timeline
                .and_then(|t| t.buses.iter().find(|x| x.id == id))
                .map(|x| x.lanes.clone());
            (json!({"bus": id}), lanes)
        }
        _ => {
            let id = pick(&vcas, b, "VCA99");
            let lanes = timeline
                .and_then(|t| t.vcas.iter().find(|x| x.id == id))
                .map(|x| x.lanes.clone());
            (json!({"vca": id}), lanes)
        }
    };
    let lanes = lanes.unwrap_or_default();
    let op = |value: Value| -> Op { serde_json::from_value(value).unwrap() };
    let held: Vec<&String> = lanes.keys().collect();
    if c % 2 == 0 || held.is_empty() {
        let path = PATHS[d % PATHS.len()];
        let kind = if matches!(path, "mute" | "insert.fx1.on") == (c % 7 != 3) {
            "switch"
        } else {
            "continuous"
        };
        let lane = if c % 5 == 4 {
            Value::Null
        } else {
            lane_of(d / 10, kind)
        };
        op(json!({"op": "set_lane", "timeline": t, "of": of, "path": path, "lane": lane}))
    } else {
        let path = held[d % held.len()].clone();
        let lane = &lanes[&path];
        let len = lane.points.len();
        let kind = serde_json::to_value(lane.kind).unwrap();
        let points = lane_of(d / 3, kind.as_str().unwrap())["points"].clone();
        let points = points.as_array().unwrap();
        let points = points[..points.len().min((d / 5) % 3)].to_vec();
        op(
            json!({"op": "splice_lane", "timeline": t, "of": of, "path": path,
            "at": (c / 2) % (len + 2), "remove": (d / 7) % 3, "points": points}),
        )
    }
}

/// A `set_clip` of the settings of sound of schema 4 of a clip: its equalizer, effects, pitch,
/// pitch correction and channels, now and then out of their ranges or too many channels.
fn set_sound_of_clip(project: &Project, timeline: &str, clip: &str, n: [usize; 3]) -> Op {
    let [a, c, d] = n;
    let channels_of_track = project
        .timelines
        .iter()
        .flat_map(|t| &t.tracks)
        .find(|r| r.clips.iter().any(|x| x.id == clip))
        .map_or(2, |r| usize::from(r.audio.channels));
    let mut eq = Equalizer::clip();
    eq.on = a.is_multiple_of(2);
    eq.bands[a % 4].gain_db = [3.0, -30.0][c % 2];
    let channel = |k: u32| ChannelMap {
        remix: None,
        source: ChannelSources(vec![ChannelSource {
            stream: 0,
            channel: k,
        }]),
        enabled: true,
        mute: k % 2 == 1,
        level_db: [0.0, -6.0, 30.0][d % 3],
    };
    let count = if d.is_multiple_of(5) {
        1
    } else {
        channels_of_track
    };
    let bit = |k: u32| (d >> k) & 1 == 1;
    Op::SetClip {
        timeline: timeline.into(),
        id: clip.into(),
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
        eq: bit(4).then_some(eq),
        effects: bit(5).then(|| vec![insert(a), insert(c)]),
        pitch_correction: bit(6).then(|| bit(7)),
        pitch: bit(8).then(|| ClipPitch {
            semitones: [-12, 3, 13][a % 3],
            cents: [0.0, -50.0, 120.0][c % 3],
        }),
        channels: bit(9).then(|| {
            bit(10).then(|| {
                (0..count)
                    .map(|k| channel(u32::try_from(k).unwrap()))
                    .collect()
            })
        }),
        source: None,
    }
}
