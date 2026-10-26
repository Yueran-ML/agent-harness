//! Names that live for one transaction.
//!
//! A client that adds a thing does not know the id that the engine will give it, and so could
//! not name it in a later operation of the same transaction. Instead it may give the thing an id
//! of the form `$name` (a `$` and 1 to 32 of `A`-`Z`, `a`-`z`, `0`-`9` and `_`): the thing is
//! added as if its id were empty, and given one by the engine, and every later operation of the
//! transaction that names `$name` where it names a thing is made to name that id before it is
//! applied. The ids that the names were given are answered in `names`.
//!
//! An id that starts with `$` is always such a name, so no thing is ever given one. A name is
//! given to one thing only, and may be named only after the operation that gives it.
//!
//! The id of a track names it only among the tracks of its timeline, so a name of a track is
//! named only with its timeline: an operation that names it must name the timeline as the
//! operation that gave it did, by the same name or the same id.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::fmt;

use columbina_model::{Id, Op, Track};

/// The most characters of a name after its `$`.
const LONGEST: usize = 32;

/// Why the names of a transaction cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameError {
    /// The place of the operation in the transaction.
    pub index: usize,
    /// The name, as it was written.
    pub name: String,
    /// What is wrong with it: `invalid_name`, `unknown_name`, `duplicate_name` or
    /// `name_elsewhere`.
    pub reason: &'static str,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { index, name, .. } = self;
        match self.reason {
            "invalid_name" => write!(
                f,
                "operation {index} names {name}, which is not a name of a transaction: a $ and \
                 1 to {LONGEST} letters, digits or _"
            ),
            "unknown_name" => write!(
                f,
                "operation {index} names {name}, which no operation before it in the \
                 transaction gives to a thing"
            ),
            "name_elsewhere" => write!(
                f,
                "operation {index} names the track {name} in another timeline than the one that \
                 it was given in"
            ),
            _ => write!(
                f,
                "operation {index} gives the name {name}, which an operation before it gives \
                 already"
            ),
        }
    }
}

/// Where a thing that is given a name is, in the operation that adds it.
#[derive(Clone, Copy, Debug)]
enum Place {
    Media,
    Timeline,
    /// A track: the one that is added, or the one at this place in a timeline that is added.
    Track(Option<usize>),
    /// A clip: the one that is put on a track, or the one that starts at this frame on the
    /// track that is added, or on the track at this place in a timeline that is added. Clips
    /// on a track cover no frame in common, so no two start at one frame.
    Clip(Option<usize>, i64),
}

/// What an id of an operation is to it.
#[derive(Clone, Debug)]
enum Role {
    /// It names a thing that is there, other than a track.
    Names,
    /// It names a track of the timeline that the operation names, as it is written there.
    NamesTrack(Id),
    /// It is the id of a thing that the operation adds, other than a track.
    Gives(Place),
    /// It is the id of a track that the operation adds, to the timeline written so; none for
    /// a timeline that is added without an id, which no later operation can name.
    GivesTrack(Place, Option<Id>),
}

/// The names of a transaction, and the ids that the engine gave them as it was applied.
#[derive(Debug, Default)]
pub struct Names {
    /// For each operation, the names that it gives and where.
    given: Vec<Vec<(String, Place)>>,
    /// The ids of the names of the operations that have been applied.
    ids: BTreeMap<String, Id>,
    /// How many operations have been applied.
    applied: usize,
}

/// Check the names of a transaction, and make each thing that is given one have an empty id,
/// for the engine to give it one.
///
/// # Errors
/// A name is not of the form of a name, is named before it is given, or is given twice.
pub fn prepare(ops: &mut [Op]) -> Result<Names, NameError> {
    let mut known = HashSet::new();
    // The timeline of each track that is given a name, as it is written.
    let mut tracks: HashMap<String, Option<Id>> = HashMap::new();
    let mut given = Vec::with_capacity(ops.len());
    for (index, op) in ops.iter_mut().enumerate() {
        let mut here = Vec::new();
        visit(op, &mut |id, role| {
            if !id.starts_with('$') {
                return Ok(());
            }
            let error = |reason| NameError {
                index,
                name: id.clone(),
                reason,
            };
            if !is_name(id) {
                return Err(error("invalid_name"));
            }
            match role {
                Role::Names | Role::NamesTrack(_) if !known.contains(id.as_str()) => {
                    Err(error("unknown_name"))
                }
                Role::NamesTrack(of) => match tracks.get(id.as_str()) {
                    Some(given_in) if given_in.as_deref() != Some(of.as_str()) => {
                        Err(error("name_elsewhere"))
                    }
                    _ => Ok(()),
                },
                Role::Names => Ok(()),
                Role::Gives(_) | Role::GivesTrack(..) if known.contains(id.as_str()) => {
                    Err(error("duplicate_name"))
                }
                Role::Gives(place) => {
                    known.insert(id.clone());
                    here.push((std::mem::take(id), place));
                    Ok(())
                }
                Role::GivesTrack(place, of) => {
                    known.insert(id.clone());
                    tracks.insert(id.clone(), of);
                    here.push((std::mem::take(id), place));
                    Ok(())
                }
            }
        })?;
        given.push(here);
    }
    Ok(Names {
        given,
        ..Names::default()
    })
}

impl Names {
    /// Make an operation name the ids that the names it names were given, from the operations
    /// before it as they were done. It is the `map` of
    /// [`History::apply_mapped`](columbina_model::History::apply_mapped).
    pub fn map(&mut self, mut op: Op, done: &[Op]) -> Op {
        self.learn(done);
        let ids = &self.ids;
        let _ = visit(&mut op, &mut |id, role| {
            if let (Role::Names | Role::NamesTrack(_), Some(given)) = (role, ids.get(id.as_str())) {
                id.clone_from(given);
            }
            Ok::<(), Infallible>(())
        });
        op
    }

    /// The id that each name was given, once the whole transaction was applied and `done` is
    /// what it did.
    pub fn finish(mut self, done: &[Op]) -> BTreeMap<String, Id> {
        self.learn(done);
        self.ids
    }

    /// Learn the ids of the names of the operations that were applied since the last time.
    fn learn(&mut self, done: &[Op]) {
        for (op, given) in done.iter().zip(&self.given).skip(self.applied) {
            for (name, place) in given {
                if let Some(id) = id_at(op, *place) {
                    self.ids.insert(name.clone(), id.to_owned());
                }
            }
        }
        self.applied = self.applied.max(done.len());
    }
}

/// Whether an id is a name of a transaction.
fn is_name(id: &str) -> bool {
    id.strip_prefix('$').is_some_and(|rest| {
        (1..=LONGEST).contains(&rest.len())
            && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    })
}

/// The id of the thing at `place` in an operation as it was done.
fn id_at(op: &Op, place: Place) -> Option<&str> {
    fn starting(track: &Track, start: i64) -> Option<&str> {
        track
            .clips
            .iter()
            .find(|clip| clip.start == start)
            .map(|clip| clip.id.as_str())
    }
    match (op, place) {
        (Op::AddMedia { media, .. }, Place::Media) => Some(&media.id),
        (Op::AddTimeline { timeline, .. }, Place::Timeline) => Some(&timeline.id),
        (Op::AddTimeline { timeline, .. }, Place::Track(Some(at))) => {
            timeline.tracks.get(at).map(|track| track.id.as_str())
        }
        (Op::AddTimeline { timeline, .. }, Place::Clip(Some(at), start)) => {
            starting(timeline.tracks.get(at)?, start)
        }
        (Op::AddTrack { track, .. }, Place::Track(None)) => Some(&track.id),
        (Op::AddTrack { track, .. }, Place::Clip(None, start)) => starting(track, start),
        (Op::InsertClip { clip, .. }, Place::Clip(None, _)) => Some(&clip.id),
        _ => None,
    }
}

/// Call `f` on each id of an operation, with what it is to the operation: first those that name
/// things that are there, then those of the things that it adds.
fn visit<E>(op: &mut Op, f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>) -> Result<(), E> {
    match op {
        Op::AddMedia { media, .. } => f(&mut media.id, Role::Gives(Place::Media)),
        Op::RemoveMedia { id } | Op::RemoveTimeline { id } => f(id, Role::Names),
        Op::AddTimeline { timeline, .. } => {
            for clip in timeline.tracks.iter_mut().flat_map(|r| &mut r.clips) {
                f(&mut clip.media, Role::Names)?;
            }
            let of = Some(timeline.id.clone()).filter(|id| !id.is_empty());
            f(&mut timeline.id, Role::Gives(Place::Timeline))?;
            for (at, track) in timeline.tracks.iter_mut().enumerate() {
                let place = Place::Track(Some(at));
                f(&mut track.id, Role::GivesTrack(place, of.clone()))?;
                for clip in &mut track.clips {
                    let start = clip.start;
                    f(&mut clip.id, Role::Gives(Place::Clip(Some(at), start)))?;
                }
            }
            Ok(())
        }
        Op::AddTrack {
            timeline, track, ..
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            for clip in &mut track.clips {
                f(&mut clip.media, Role::Names)?;
            }
            f(
                &mut track.id,
                Role::GivesTrack(Place::Track(None), Some(of)),
            )?;
            for clip in &mut track.clips {
                let start = clip.start;
                f(&mut clip.id, Role::Gives(Place::Clip(None, start)))?;
            }
            Ok(())
        }
        Op::RemoveTrack { timeline, id } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            f(id, Role::NamesTrack(of))
        }
        Op::RemoveClip { timeline, id } | Op::TrimClip { timeline, id, .. } => {
            f(timeline, Role::Names)?;
            f(id, Role::Names)
        }
        Op::InsertClip {
            timeline,
            track,
            clip,
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            f(track, Role::NamesTrack(of))?;
            f(&mut clip.media, Role::Names)?;
            let start = clip.start;
            f(&mut clip.id, Role::Gives(Place::Clip(None, start)))
        }
        Op::MoveClip {
            timeline,
            id,
            track,
            ..
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            f(id, Role::Names)?;
            f(track, Role::NamesTrack(of))
        }
    }
}

#[cfg(test)]
mod tests {
    use columbina_model::{History, Project, TransactionError};
    use serde_json::json;

    use super::*;

    fn ops(value: serde_json::Value) -> Vec<Op> {
        serde_json::from_value(value).unwrap()
    }

    fn media(id: &str) -> serde_json::Value {
        let mut media = serde_json::to_value(&Project::sample().media[0]).unwrap();
        media["id"] = id.into();
        media
    }

    fn clip(id: &str, media: &str, start: i64) -> serde_json::Value {
        json!({
            "id": id, "media": media, "source_in": {"ticks": 0, "scale": 15360},
            "start": start, "length": 10,
        })
    }

    /// Apply a transaction with names to the sample, and give back what was done and the ids
    /// that the names were given.
    fn apply(
        history: &mut History,
        mut ops: Vec<Op>,
    ) -> Result<(Vec<Op>, BTreeMap<String, Id>), TransactionError> {
        let mut names = prepare(&mut ops).unwrap();
        let applied = history.apply_mapped(ops, "agent", |op, done| names.map(op, done))?;
        let ids = names.finish(&applied.done);
        Ok((applied.done, ids))
    }

    #[test]
    fn things_named_in_a_transaction_are_named_by_the_operations_after_them() {
        let mut history = History::new(Project::sample());
        let transaction = ops(json!([
            {"op": "add_media", "media": media("$m")},
            {"op": "add_timeline", "timeline": {
                "id": "$t", "name": "Second", "rate": {"num": 25, "den": 1},
                "tracks": [
                    {"id": "$v", "kind": "video", "clips": [
                        clip("$late", "$m", 50), clip("$early", "m1", 0),
                    ]},
                    {"id": "", "kind": "audio", "clips": []},
                ],
            }},
            {"op": "add_track", "timeline": "$t", "track": {
                "id": "$a", "kind": "audio", "clips": [clip("$sound", "$m", 5)],
            }},
            {"op": "insert_clip", "timeline": "$t", "track": "$v", "clip": clip("$c", "$m", 100)},
            {"op": "move_clip", "timeline": "$t", "id": "$late", "track": "$v", "start": 200},
            {"op": "trim_clip", "timeline": "$t", "id": "$c", "source_in": {"ticks": 0, "scale": 15360},
             "start": 100, "length": 5},
            {"op": "remove_clip", "timeline": "$t", "id": "$early"},
        ]));
        let (done, ids) = apply(&mut history, transaction).unwrap();
        // next_id is 3: m3, then t4 and its clips in the order of the list, c5 and c6; the
        // track of picture is V1 and the new track of sound A2; then c7 and c8.
        let expected: BTreeMap<String, Id> = [
            ("$m", "m3"),
            ("$t", "t4"),
            ("$v", "V1"),
            ("$late", "c5"),
            ("$early", "c6"),
            ("$a", "A2"),
            ("$sound", "c7"),
            ("$c", "c8"),
        ]
        .into_iter()
        .map(|(name, id)| (name.to_owned(), id.to_owned()))
        .collect();
        assert_eq!(ids, expected);
        assert_eq!(
            done[4],
            Op::MoveClip {
                timeline: "t4".into(),
                id: "c5".into(),
                track: "V1".into(),
                start: 200,
            }
        );
        assert_eq!(
            done[6],
            Op::RemoveClip {
                timeline: "t4".into(),
                id: "c6".into(),
            }
        );
        let text = serde_json::to_string(history.project()).unwrap();
        assert!(!text.contains('$'), "{text}");
    }

    #[test]
    fn a_name_is_named_only_after_it_is_given() {
        let mut transaction = ops(json!([
            {"op": "remove_media", "id": "$m"},
            {"op": "add_media", "media": media("$m")},
        ]));
        assert_eq!(
            prepare(&mut transaction).unwrap_err(),
            NameError {
                index: 0,
                name: "$m".into(),
                reason: "unknown_name",
            }
        );
        let mut transaction = ops(json!([
            {"op": "add_media", "media": media("$m")},
            {"op": "insert_clip", "timeline": "t1", "track": "V1", "clip": clip("", "$n", 100)},
        ]));
        let error = prepare(&mut transaction).unwrap_err();
        assert_eq!((error.index, error.reason), (1, "unknown_name"));
        assert_eq!(
            error.to_string(),
            "operation 1 names $n, which no operation before it in the transaction gives to a \
             thing"
        );
    }

    fn timeline(id: &str, tracks: &serde_json::Value) -> serde_json::Value {
        json!({ "id": id, "name": "N", "rate": { "num": 25, "den": 1 }, "tracks": tracks })
    }

    #[test]
    fn a_name_of_a_track_is_named_only_with_its_timeline() {
        // $v is a track of the new timeline $t, and t1 has a V1 of its own.
        let with_v = timeline("$t", &json!([{ "id": "$v", "kind": "video", "clips": [] }]));
        let mut transaction = ops(json!([
            { "op": "add_timeline", "timeline": with_v },
            { "op": "insert_clip", "timeline": "t1", "track": "$v", "clip": clip("", "m1", 200) },
        ]));
        let error = prepare(&mut transaction).unwrap_err();
        assert_eq!(
            (error.index, error.name.as_str(), error.reason),
            (1, "$v", "name_elsewhere")
        );
        assert_eq!(
            error.to_string(),
            "operation 1 names the track $v in another timeline than the one that it was given in"
        );
        // A track of a timeline that is added without an id cannot be named at all.
        let anonymous = timeline("", &json!([{ "id": "$x", "kind": "video", "clips": [] }]));
        let mut transaction = ops(json!([
            { "op": "add_timeline", "timeline": anonymous },
            { "op": "move_clip", "timeline": "t3", "id": "c1", "track": "$x", "start": 0 },
        ]));
        assert_eq!(
            prepare(&mut transaction).unwrap_err().reason,
            "name_elsewhere"
        );

        // With its timeline, by the name or the id that the timeline was named by, it is.
        let mut history = History::new(Project::sample());
        let transaction = ops(json!([
            { "op": "add_timeline", "timeline": with_v },
            { "op": "insert_clip", "timeline": "$t", "track": "$v", "clip": clip("$c", "m1", 0) },
            { "op": "add_track", "timeline": "t1", "track": { "id": "$w", "kind": "video", "clips": [] } },
            { "op": "move_clip", "timeline": "t1", "id": "c1", "track": "$w", "start": 0 },
            { "op": "remove_track", "timeline": "$t", "id": "$v" },
        ]));
        let (done, ids) = apply(&mut history, transaction).unwrap();
        assert_eq!(ids["$t"], "t3");
        assert_eq!(ids["$v"], "V1");
        assert_eq!(ids["$c"], "c4");
        assert_eq!(ids["$w"], "V2");
        assert_eq!(
            done[3],
            Op::MoveClip {
                timeline: "t1".into(),
                id: "c1".into(),
                track: "V2".into(),
                start: 0,
            }
        );
        let tracks = &history.project().timelines[0].tracks;
        assert_eq!(tracks[2].clips[0].id, "c1");
        assert_eq!(history.project().timelines[1].tracks, []);
    }

    #[test]
    fn clips_that_start_at_one_frame_on_two_tracks_keep_their_names() {
        let mut history = History::new(Project::sample());
        let tracks = json!([
            { "id": "$pic", "kind": "video", "clips": [clip("$p2", "m1", 30), clip("$p1", "m1", 0)] },
            { "id": "$snd", "kind": "audio", "clips": [clip("$s1", "m1", 0), clip("$s2", "m1", 30)] },
        ]);
        let transaction = ops(json!([
            { "op": "add_timeline", "timeline": timeline("$t", &tracks) },
            { "op": "trim_clip", "timeline": "$t", "id": "$s1", "source_in": {"ticks": 0, "scale": 15360},
              "start": 0, "length": 5 },
        ]));
        let (done, ids) = apply(&mut history, transaction).unwrap();
        // Given in the order of the lists: t3, V1, c4 ($p2), c5 ($p1), A1, c6 ($s1), c7 ($s2).
        let expected: Vec<(&str, &str)> = vec![
            ("$p1", "c5"),
            ("$p2", "c4"),
            ("$pic", "V1"),
            ("$s1", "c6"),
            ("$s2", "c7"),
            ("$snd", "A1"),
            ("$t", "t3"),
        ];
        let got: Vec<(&str, &str)> = ids.iter().map(|(n, i)| (n.as_str(), i.as_str())).collect();
        assert_eq!(got, expected);
        let Op::TrimClip { id, .. } = &done[1] else {
            panic!("{done:?}")
        };
        assert_eq!(id, "c6");
        let sound = &history.project().timelines[1].tracks[1].clips;
        assert_eq!((sound[0].id.as_str(), sound[0].length), ("c6", 5));
    }

    #[test]
    fn a_name_is_given_once() {
        let mut transaction = ops(json!([
            {"op": "add_media", "media": media("$x")},
            {"op": "insert_clip", "timeline": "t1", "track": "V1", "clip": clip("$x", "m1", 100)},
        ]));
        let error = prepare(&mut transaction).unwrap_err();
        assert_eq!((error.index, error.reason), (1, "duplicate_name"));
    }

    #[test]
    fn an_id_that_starts_with_a_dollar_is_a_name_or_refused() {
        let long = format!("${}", "a".repeat(LONGEST + 1));
        for id in ["$", "$a-b", "$é", long.as_str()] {
            let mut transaction = ops(json!([{"op": "add_media", "media": media(id)}]));
            let error = prepare(&mut transaction).unwrap_err();
            assert_eq!((error.index, error.reason), (0, "invalid_name"), "{id}");
            let mut transaction = ops(json!([{"op": "remove_timeline", "id": id}]));
            let error = prepare(&mut transaction).unwrap_err();
            assert_eq!(error.reason, "invalid_name", "{id}");
        }
        let longest = format!("${}", "Az_9".repeat(LONGEST / 4));
        assert!(is_name(&longest));
        assert!(!is_name("m1"));
    }

    #[test]
    fn a_transaction_without_names_is_applied_as_it_is() {
        let mut history = History::new(Project::sample());
        let transaction = ops(json!([
            {"op": "insert_clip", "timeline": "t1", "track": "V1", "clip": clip("", "m1", 100)},
        ]));
        let (done, ids) = apply(&mut history, transaction).unwrap();
        assert!(ids.is_empty());
        let Op::InsertClip { clip, .. } = &done[0] else {
            panic!("{done:?}")
        };
        assert_eq!(clip.id, "c3");
    }
}
