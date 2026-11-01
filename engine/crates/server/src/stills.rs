//! Stills of the Gallery: grabbing the picture of the timeline monitor with the grade that made
//! it, the albums they are kept in, and applying the grade of a still to clips (Reference Manual
//! of `DaVinci Resolve` 20, ch. 141 and 142, pp. 3256–3297, 3393). Where they are kept is
//! [`crate::gallery`].
//!
//! # What a still holds
//!
//! - **Its picture**: the frame of the timeline monitor as the viewer has it, after the output
//!   transform (image A of a wipe, never the comparison), as a PNG of 16 bits or, where a value
//!   is beyond 0..1 (an output of scene-linear light, or above white), as an `OpenEXR` of half
//!   floats, uncompressed. `DaVinci` saves a DPX (p. 3259).
//! - **Its grades** (`grades`): "both the Clip grade, and the Timeline grade, if one is applied"
//!   (p. 3286). Of a clip in a group, the grade of the Node Editor's mode only (p. 3306): the
//!   clip's own (`level` `clip`), or its group's pre-clip or post-clip grade (`pre_clip`,
//!   `post_clip`). Each with its keyframes, which "reference the source timecode, or start frame
//!   of the original clips" (p. 3393): `keyframes` tells what frames they count, and `source`
//!   the clip, its media and its timeline, from which they are put on the frames of another.
//! - **Its grades at the frame grabbed** (`frozen`), for "No Keyframes" (p. 3287): each setting
//!   that has keys as "whatever parameter values are contained by the next keyframe to the left
//!   of the position of the playhead" (p. 3393: halfway between keys of 50 and 0, 50).
//! - Its number, Track.Shot.Version (pp. 3258, 3261; the version is always 1), a label (p. 3262),
//!   when it was made, and where it came from: the timeline and its frame and record timecode,
//!   the clip, its media, its source frame and source timecode (the time from the start of the
//!   media, which has no timecode of its own), as Still Properties tells (p. 3258).
//!
//! # Applying it
//!
//! `still.apply` is worked out, while the project is locked, into a `set_grade` of each grade it
//! goes to, in one transaction, so one undo takes it back: what `copy_grade` (Apply Grade) and
//! `append_node_graph` (Append Node Graph) are worked out into. A grade of a timeline takes the
//! still's timeline grade, any other its clip grade (p. 3286: which grade "depends on which mode
//! the Node Editor is in").

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use columbina_compose::Target as Canvas;
use columbina_model::grade::{OUTPUT, SOURCE};
use columbina_model::keyframes::{Align, KeyBase, KeyClock};
use columbina_model::project::{Source, TrackKind};
use columbina_model::{
    Change, Clip, Edit, Given, Grade, Kind as NodeKind, Link, Media, Project, Rate, Target,
    Timeline,
};
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{json, Value};

use crate::gallery::{self, ImageFormat, Kind, Opened, Pixels, Places};
use crate::play::Transport;
use crate::rpc::{self, Fault};
use crate::session::{self, Connection, Engine, Reply};

/// The most nodes that Copy Grade: Preserve number of nodes keeps (p. 3285).
const MOST_PRESERVED_NODES: u32 = 10;
/// The longest label of a still, and name of an album, in characters.
const LONGEST_NAME: usize = 100;

/// The frame the timeline monitor shows, before it is compared with anything, and what it is
/// of: what a still is grabbed of.
pub struct Frame {
    pub target: Arc<Canvas>,
    /// The frame of the timeline it is.
    pub at: i64,
    pub project: Arc<Project>,
    pub timeline: usize,
}

/// Whether `method` is one of this module's.
fn is_method(method: &str) -> bool {
    method.starts_with("still.") || method.starts_with("gallery.") || method == "grade.append_node"
}

/// `viewer.wipe`, `viewer.split`: what the timeline monitor compares with, a still of the
/// Gallery named by its id as its file ([`resolve_references`]).
pub async fn compare(
    engine: &Engine,
    transport: &Transport,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Value, Fault> {
    let params = resolve_references(&engine.project, params)?;
    transport.compare(method, params.as_deref()).await
}

/// The params of a method that takes an object whose fields may all be left out: none is `{}`.
fn params_or_empty<'a, T: Deserialize<'a>>(
    method: &str,
    takes: &str,
    params: Option<&'a RawValue>,
) -> Result<T, Fault> {
    match params {
        Some(params) if params.get() != "null" => rpc::params(method, takes, Some(params)),
        _ => serde_json::from_str("{}")
            .map_err(|error| Fault::params(format!("the params of {method}: {error}"))),
    }
}

fn range(what: &str, message: impl Into<String>) -> Fault {
    Fault::params(message).with_data(json!({"reason": "out_of_range", "what": what}))
}

/// A refusal of a change, as a transaction's (`-32001`).
fn refused(reason: &str, message: &str, fields: Value) -> Fault {
    let mut data = json!({"reason": reason, "message": message});
    if let (Some(data), Value::Object(fields)) = (data.as_object_mut(), fields) {
        data.extend(fields);
    }
    Fault::new(rpc::REFUSED, message.to_string()).with_data(data)
}

/// Run `work` on a thread of its own, where it may read and write files.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Fault> + Send + 'static,
) -> Result<T, Fault> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| Fault::new(rpc::CANNOT_READ, error.to_string()))?
}

/// Answer the request `id` with `result`.
fn answer(connection: &Connection, id: &RawValue, result: &Value) -> Reply {
    match connection.queue(rpc::result_line(id, result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    }
}

/// `still.*`, `gallery.*` and `grade.append_node`; any other method is not there.
#[allow(clippy::too_many_lines, reason = "an arm a method")]
pub async fn call(
    engine: &Engine,
    transport: &Transport,
    connection: &Connection,
    kind: session::Kind,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    if !is_method(method) {
        return Err(Fault::new(
            rpc::METHOD_NOT_FOUND,
            format!("there is no method {method}"),
        )
        .into());
    }
    let places = Places::of(&engine.project);
    let (result, changed) = match method {
        "still.apply" => {
            let request: ApplyRequest = rpc::params(
                method,
                r#"{"still": "s1", "to": [{"level": "clip", "timeline": "t1", "clip": "c3"}], "mode": "apply", "keyframes": "none"}"#,
                params,
            )?;
            let read = places.clone();
            let still_id = request.still.clone();
            let still = blocking(move || {
                gallery::with(&read, gallery::kind_of(&still_id, true)?, |opened| {
                    opened.still(&still_id)
                })
            })
            .await?;
            let revision = request.revision;
            return session::apply_worked_out(engine, connection, kind, id, revision, |project| {
                work_apply(project, &still, &request)
            })
            .await;
        }
        "grade.append_node" => {
            let request: AppendNodeRequest = rpc::params(
                method,
                r#"{"from": {"level": "clip", "timeline": "t1", "clip": "c3"}, "node": "n4", "to": [...], "keyframes"?: "none" | "source_timecode" | "start_frame", "at"?: 120}"#,
                params,
            )
            .and_then(AppendNodeRequest::checked)?;
            let revision = request.revision;
            return session::apply_worked_out(engine, connection, kind, id, revision, |project| {
                work_append_node(project, &request)
            })
            .await;
        }
        "still.grab" => {
            let request: GrabRequest = params_or_empty(
                method,
                r#"{"view": "timeline", "album"?: "a1", "clip"?: "c3", "level"?: "clip"}"#,
                params,
            )?;
            if request.view != "timeline" {
                return Err(Fault::params(
                    "a still is grabbed of the timeline monitor (\"view\": \"timeline\")",
                )
                .with_data(json!({"reason": "view"}))
                .into());
            }
            let frame = transport.original(method).await?;
            let still = blocking(move || grab(&places, &request, &frame)).await?;
            (still, true)
        }
        "gallery.list" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct List {
                album: Option<String>,
            }
            let request: List = params_or_empty(method, r#"{"album"?: "a1"}"#, params)?;
            let list = blocking(move || gallery::list(&places, request.album.as_deref())).await?;
            (list, false)
        }
        "gallery.thumbnail" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Thumbnail {
                still: String,
                #[serde(default = "ninety")]
                height: u32,
            }
            fn ninety() -> u32 {
                90
            }
            let request: Thumbnail =
                rpc::params(method, r#"{"still": "s1", "height"?: 90}"#, params)?;
            if !(1..=gallery::MOST_THUMBNAIL).contains(&request.height) {
                return Err(range("height", "height is from 1 to 270").into());
            }
            let thumbnail =
                blocking(move || gallery::thumbnail(&places, &request.still, request.height))
                    .await?;
            (thumbnail, false)
        }
        "still.get" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Get {
                still: String,
            }
            let request: Get = rpc::params(method, r#"{"still": "s1"}"#, params)?;
            let still = blocking(move || {
                gallery::with(&places, gallery::kind_of(&request.still, true)?, |opened| {
                    let still = opened.still(&request.still)?;
                    let album = opened
                        .album_of(&request.still)
                        .map(|album| album.id.clone());
                    let mut still = still;
                    still["album"] = json!(album);
                    Ok(still)
                })
            })
            .await?;
            (still, false)
        }
        _ => {
            let method_name = method.to_string();
            let params: Value = match params {
                Some(params) => serde_json::from_str(params.get())
                    .map_err(|error| Fault::params(error.to_string()))?,
                None => json!({}),
            };
            let result = blocking(move || gallery_change(&places, &method_name, params)).await?;
            (result, true)
        }
    };
    if changed {
        session::tell_everyone(engine, "gallery.changed", &json!({})).await;
    }
    Ok(answer(connection, id, &result))
}

/// The methods that change the albums and stills, but for grabbing one.
#[allow(clippy::too_many_lines, reason = "an arm a method")]
fn gallery_change(places: &Places, method: &str, params: Value) -> Result<Value, Fault> {
    let read = |takes: &'static str| {
        move |error: serde_json::Error| {
            Fault::params(format!("the params of {method}, {takes}: {error}"))
        }
    };
    match method {
        "gallery.album.create" => {
            /// The kinds of album: of the gallery of each, and the project's album of the
            /// memories.
            #[derive(Clone, Copy, Default, Deserialize)]
            #[serde(rename_all = "snake_case")]
            enum AlbumKind {
                #[default]
                Stills,
                Powergrade,
                Memories,
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Create {
                #[serde(default)]
                kind: AlbumKind,
                name: Option<String>,
            }
            let takes = r#"{"kind"?: "stills" | "powergrade" | "memories", "name"?: "Looks"}"#;
            let request: Create = serde_json::from_value(params).map_err(read(takes))?;
            if let Some(name) = &request.name {
                check_name(name, false)?;
            }
            let kind = match request.kind {
                AlbumKind::Stills => Kind::Stills,
                AlbumKind::Powergrade => Kind::Powergrade,
                AlbumKind::Memories => {
                    return gallery::with(places, Kind::Stills, |opened| {
                        let id = opened.memories_album(request.name.clone());
                        opened.save()?;
                        let name = opened.album_mut(&id)?.name.clone();
                        Ok(json!({"id": id, "name": name, "kind": "memories"}))
                    });
                }
            };
            gallery::with(places, kind, |opened| {
                let id = opened.add_album(request.name.clone());
                opened.save()?;
                let name = opened.album_mut(&id)?.name.clone();
                Ok(json!({"id": id, "name": name, "kind": kind}))
            })
        }
        "gallery.album.rename" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Rename {
                album: String,
                name: String,
            }
            let takes = r#"{"album": "a1", "name": "Looks"}"#;
            let request: Rename = serde_json::from_value(params).map_err(read(takes))?;
            check_name(&request.name, false)?;
            gallery::with(places, gallery::kind_of(&request.album, false)?, |opened| {
                opened
                    .album_mut(&request.album)?
                    .name
                    .clone_from(&request.name);
                opened.save()?;
                Ok(json!({"id": request.album, "name": request.name}))
            })
        }
        "gallery.album.delete" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Delete {
                album: String,
            }
            let request: Delete =
                serde_json::from_value(params).map_err(read(r#"{"album": "a2"}"#))?;
            gallery::with(places, gallery::kind_of(&request.album, false)?, |opened| {
                let deleted = opened.remove_album(&request.album)?;
                opened.save()?;
                Ok(json!({"deleted": deleted}))
            })
        }
        "still.delete" => {
            let request: Stills =
                serde_json::from_value(params).map_err(read(r#"{"stills": ["s1", ...]}"#))?;
            let ids = gallery::still_ids(&request.stills)?;
            // Every one is checked before any is deleted, of both galleries where both are named.
            let delete = |opened: &mut [&mut Opened]| {
                for (id, kind) in &ids {
                    let of = opened.iter().position(|one| one.kind() == *kind);
                    of.and_then(|of| opened[of].album_of(id))
                        .ok_or_else(|| gallery::missing("still", id))?;
                }
                // Each gallery writes its index and moves its files; where the second cannot,
                // what the first deleted is put back, so that nothing is deleted.
                let of = |kind: Kind| -> Vec<String> {
                    ids.iter()
                        .filter(|(_, each)| *each == kind)
                        .map(|(id, _)| id.clone())
                        .collect()
                };
                for at in 0..opened.len() {
                    if let Err(fault) = opened[at].recycle_stills(&of(opened[at].kind())) {
                        for done in opened.iter_mut().take(at) {
                            let _ = done.restore_stills(&of(done.kind()));
                        }
                        return Err(fault);
                    }
                }
                Ok(())
            };
            let kinds: HashSet<Kind> = ids.iter().map(|(_, kind)| *kind).collect();
            if kinds.len() == 2 {
                gallery::with_both(places, |project, power| delete(&mut [project, power]))?;
            } else if let Some(kind) = kinds.into_iter().next() {
                gallery::with(places, kind, |opened| delete(&mut [opened]))?;
            }
            Ok(json!({"deleted": ids.into_iter().map(|(id, _)| id).collect::<Vec<_>>()}))
        }
        "still.restore" => {
            let request: Stills =
                serde_json::from_value(params).map_err(read(r#"{"stills": ["s1", ...]}"#))?;
            restore(places, &request.stills)
        }
        "still.copy" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Copy {
                stills: Vec<String>,
                album: String,
            }
            let takes = r#"{"stills": ["s1", ...], "album": "a3"}"#;
            let request: Copy = serde_json::from_value(params).map_err(read(takes))?;
            copy_stills(places, &request.stills, &request.album)
        }
        "still.label" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Label {
                still: String,
                label: String,
            }
            let takes = r#"{"still": "s1", "label": "Day exterior"}"#;
            let request: Label = serde_json::from_value(params).map_err(read(takes))?;
            check_name(&request.label, true)?;
            gallery::with(places, gallery::kind_of(&request.still, true)?, |opened| {
                let mut still = opened.still(&request.still)?;
                still["label"] = json!(request.label);
                opened.write_still(&request.still, &still)?;
                Ok(json!({"id": request.still, "label": request.label}))
            })
        }
        "still.move" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Move {
                stills: Vec<String>,
                album: String,
            }
            let takes = r#"{"stills": ["s1", ...], "album": "pa1"}"#;
            let request: Move = serde_json::from_value(params).map_err(read(takes))?;
            move_stills(places, &request.stills, &request.album)
        }
        "still.export" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Export {
                stills: Vec<String>,
                dir: String,
                #[serde(default)]
                prefix: String,
                #[serde(default)]
                replace: bool,
            }
            let takes = r#"{"stills": ["s1", ...], "dir": "C:\\looks", "prefix"?: "Show", "replace"?: false}"#;
            let request: Export = serde_json::from_value(params).map_err(read(takes))?;
            export(
                places,
                &request.stills,
                &request.dir,
                &request.prefix,
                request.replace,
            )
        }
        "still.import" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Import {
                paths: Vec<String>,
                album: Option<String>,
            }
            let takes = r#"{"paths": ["C:\\ref\\a.png", ...], "album"?: "a1"}"#;
            let request: Import = serde_json::from_value(params).map_err(read(takes))?;
            import(places, &request.paths, request.album.as_deref())
        }
        _ => Err(Fault::new(
            rpc::METHOD_NOT_FOUND,
            format!("there is no method {method}"),
        )),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stills {
    stills: Vec<String>,
}

/// Check the label of a still (which may be empty) or the name of an album (which may not).
fn check_name(name: &str, empty: bool) -> Result<(), Fault> {
    let count = name.chars().count();
    if count > LONGEST_NAME || (!empty && name.trim().is_empty()) || name.contains('\0') {
        let what = if empty { "label" } else { "name" };
        return Err(range(
            what,
            format!(
                "a {what} is of {} to {LONGEST_NAME} characters",
                u8::from(!empty)
            ),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Grabbing
// ---------------------------------------------------------------------------------------------

/// The level of the grade of a clip that a still keeps: the Node Editor's mode (p. 3306).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Level {
    #[default]
    Clip,
    PreClip,
    PostClip,
}

/// How the picture of a still is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Choice {
    /// PNG where every value is within 0..1, else `OpenEXR`.
    #[default]
    Auto,
    Png,
    Exr,
}

fn timeline_view() -> String {
    "timeline".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrabRequest {
    #[serde(default = "timeline_view")]
    view: String,
    album: Option<String>,
    clip: Option<String>,
    #[serde(default)]
    level: Level,
    #[serde(default)]
    format: Choice,
}

/// Grab a still of `frame` into the album asked for, or the first of the project's; logged as
/// `still_grabbed` with how long its picture took to be read back from the card and to be written
/// (a frame of 3840 × 2160 is 66 MB of half floats).
fn grab(places: &Places, request: &GrabRequest, frame: &Frame) -> Result<Value, Fault> {
    let started = std::time::Instant::now();
    let (width, height) = frame.target.size();
    let halves = frame.target.read_halves().map_err(|error| {
        Fault::new(rpc::TRANSPORT_FAILED, error.to_string()).with_data(json!({"reason": "draw"}))
    })?;
    let picture = gallery::Picture {
        width,
        height,
        pixels: Pixels::Halves(halves),
    };
    let format = match request.format {
        Choice::Png => ImageFormat::Png,
        Choice::Exr => ImageFormat::Exr,
        Choice::Auto => {
            if picture.within_unit() {
                ImageFormat::Png
            } else {
                ImageFormat::Exr
            }
        }
    };
    let still = describe(
        &frame.project,
        frame.timeline,
        frame.at,
        request.clip.as_deref(),
        request.level,
    )?;
    let kind = match &request.album {
        Some(album) => gallery::kind_of(album, false)?,
        None => Kind::Stills,
    };
    let read = started.elapsed();
    let grabbed = gallery::with(places, kind, |opened| {
        let album = match &request.album {
            Some(album) => album.clone(),
            None => first_album(opened)?,
        };
        let still = opened.add_still(&album, still, &picture, format)?;
        Ok(gallery::summary(&still, &album))
    })?;
    crate::log::log(
        "still_grabbed",
        &json!({
            "width": width,
            "height": height,
            "format": grabbed["image"]["format"],
            "read_ms": read.as_secs_f64() * 1000.0,
            "written_ms": started.elapsed().saturating_sub(read).as_secs_f64() * 1000.0,
        }),
    );
    Ok(grabbed)
}

/// The clip a still is of: the one named, or the one on top at `frame`; with its place in the
/// order of the thumbnail timeline (from 0) and the number of its track of picture (from 1).
fn current_clip<'a>(
    timeline: &'a Timeline,
    frame: i64,
    named: Option<&str>,
) -> Result<Option<(usize, usize, &'a Clip)>, Fault> {
    let ordered = crate::compare::clips_in_order(timeline);
    let place = match named {
        Some(named) => Some(
            ordered
                .iter()
                .position(|(_, clip)| clip.id == named)
                .ok_or_else(|| {
                    Fault::params(format!(
                        "there is no clip {named} on a track of picture that is on"
                    ))
                    .with_data(json!({"reason": "missing", "what": "clip", "id": named}))
                })?,
        ),
        None => ordered
            .iter()
            .enumerate()
            .filter(|(_, (_, clip))| clip.start <= frame && frame < clip.end())
            .max_by_key(|(_, (number, _))| *number)
            .map(|(place, _)| place),
    };
    Ok(place.map(|place| {
        let (number, clip) = ordered[place];
        let track = timeline.tracks[..=number]
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .count();
        (place, track, clip)
    }))
}

/// What a still of `frame` of the timeline `index` of `project` holds, but for its picture: its
/// grades, as they are and at the frame, the frames their keys count, and where it is from.
fn describe(
    project: &Project,
    index: usize,
    frame: i64,
    named: Option<&str>,
    level: Level,
) -> Result<Value, Fault> {
    let timeline = &project.timelines[index];
    let current = current_clip(timeline, frame, named)?;
    let mut grades = json!({"clip": null, "timeline": null});
    let mut frozen = json!({"clip": null, "timeline": null});
    let mut keyframes = json!({"clip": null, "timeline": null});
    let mut source = json!({
        "project": "",
        "timeline": {"id": timeline.id, "name": timeline.name, "rate": timeline.rate},
        "frame": frame,
        "record_timecode": timecode(frame, timeline.rate),
        "clip": null,
        "media": null,
        "group": null,
        "track": null,
        "shot": null,
        "source_frame": null,
        "source_timecode": null,
    });
    let mut name = String::new();
    if let Some((place, track, clip)) = current {
        let clock = KeyClock::of(clip, project, timeline.rate);
        let at = clock.at(frame);
        let grade = match level {
            Level::Clip => clip.grade.clone(),
            Level::PreClip | Level::PostClip => {
                let group = clip
                    .group
                    .as_ref()
                    .and_then(|id| project.groups.iter().find(|group| group.id == *id))
                    .ok_or_else(|| {
                        Fault::params(format!(
                            "the clip {} is in no group, so it has no group grade",
                            clip.id
                        ))
                        .with_data(json!({"reason": "ungrouped", "clip": clip.id}))
                    })?;
                if level == Level::PreClip {
                    group.pre.clone()
                } else {
                    group.post.clone()
                }
            }
        };
        let keyed = json!(grade);
        frozen["clip"] = freeze(&keyed, at);
        keyframes["clip"] = if level == Level::Clip {
            let ends = [clock.at(clip.start), clock.at(clip.end().saturating_sub(1))];
            key_info(
                &keyed,
                if clock.counts_source() {
                    "source"
                } else {
                    "clip"
                },
                clock.rate(),
                if clock.counts_source() {
                    ends[0].min(ends[1])
                } else {
                    0
                },
                clock.counts_source().then(|| ends[0].max(ends[1])),
            )
        } else {
            key_info(&keyed, "group", clock.rate(), 0, None)
        };
        grades["clip"] = keyed;
        let mut kept = clip.clone();
        kept.grade = Grade::default();
        source["clip"] = json!(kept);
        source["group"] = json!(clip.group);
        if let Source::Media { media, .. } = &clip.source {
            source["media"] = json!(project.media.iter().find(|m| m.id == *media));
        }
        source["track"] = json!(track);
        source["shot"] = json!(place + 1);
        source["source_frame"] = json!(at);
        if clock.counts_source() {
            source["source_timecode"] = json!(timecode(at, clock.rate()));
        }
        name = format!("{track}.{}.1", place + 1);
    }
    if !timeline.grade.is_empty() {
        let keyed = json!(timeline.grade);
        frozen["timeline"] = freeze(&keyed, frame);
        keyframes["timeline"] = key_info(&keyed, "timeline", timeline.rate, 0, None);
        grades["timeline"] = keyed;
    }
    Ok(json!({
        "name": name,
        "label": "",
        "created": gallery::now(),
        "level": level,
        "grades": grades,
        "frozen": frozen,
        "keyframes": keyframes,
        "source": source,
        "color": project.color,
    }))
}

/// The keys of `grade` (JSON) as a still tells them: what frames they count (`counts`: of the
/// clip's `source`, of the `clip`, of the `timeline`, or of whatever clip a `group` grade is on),
/// at what `rate`, from what `start` (the frame of the clip's start: its in point in its source,
/// or 0) to what `last`, and the frames they are at, as they are and from the start.
fn key_info(grade: &Value, counts: &str, rate: Rate, start: i64, last: Option<i64>) -> Value {
    let mut frames = Vec::new();
    key_frames(grade, &mut frames);
    frames.sort_unstable();
    frames.dedup();
    let from_start: Vec<i64> = frames.iter().map(|frame| frame - start).collect();
    json!({
        "counts": counts,
        "rate": rate,
        "start": start,
        "last": last,
        "frames": frames,
        "from_start": from_start,
    })
}

/// The keys of a parameter, where `value` is one: `{"keys": [...]}`.
fn keys_of(value: &Value) -> Option<&Vec<Value>> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    object.get("keys")?.as_array()
}

fn key_frames(value: &Value, frames: &mut Vec<i64>) {
    if let Some(keys) = keys_of(value) {
        frames.extend(keys.iter().filter_map(|key| key["frame"].as_i64()));
        return;
    }
    match value {
        Value::Object(map) => map.values().for_each(|value| key_frames(value, frames)),
        Value::Array(items) => items.iter().for_each(|value| key_frames(value, frames)),
        _ => {}
    }
}

/// `grade` (JSON) with each setting that has keys made the value of the key at `frame` or the
/// nearest before it (the first where it is before them all), as a still saves the grade
/// without its keys (p. 3393).
fn freeze(value: &Value, frame: i64) -> Value {
    if let Some(keys) = keys_of(value) {
        let key = keys
            .iter()
            .rev()
            .find(|key| key["frame"].as_i64().is_some_and(|at| at <= frame))
            .or_else(|| keys.first());
        return key.map_or(Value::Null, |key| key["value"].clone());
    }
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(name, value)| (name.clone(), freeze(value, frame)))
                .collect(),
        ),
        Value::Array(items) => {
            Value::Array(items.iter().map(|value| freeze(value, frame)).collect())
        }
        other => other.clone(),
    }
}

/// `frame` at `rate` as a timecode, `HH:MM:SS:FF`, not drop frame, of a whole number of frames a
/// second (the rate rounded up); a frame before 0 is 0.
fn timecode(frame: i64, rate: Rate) -> String {
    let base = i64::from(rate.num)
        .checked_div(i64::from(rate.den.max(1)))
        .map_or(1, |whole| {
            if i64::from(rate.num) % i64::from(rate.den.max(1)) == 0 {
                whole
            } else {
                whole + 1
            }
        })
        .max(1);
    let frame = frame.max(0);
    let (seconds, frames) = (frame / base, frame % base);
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        frames
    )
}

// ---------------------------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------------------------

/// Apply Grade or Append Node Graph (p. 3290).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    #[default]
    Apply,
    Append,
}

/// Apply Grades Using (pp. 3264, 3287).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Keyframes {
    /// No Keyframes: the grade as it was at the frame grabbed.
    #[default]
    None,
    SourceTimecode,
    StartFrame,
}

impl Keyframes {
    /// How the keys are put on the frames of another grade; none for No Keyframes, whose grade
    /// has no keys left.
    fn align(self) -> Option<Align> {
        match self {
            Keyframes::None => None,
            Keyframes::SourceTimecode => Some(Align::SourceTimecode),
            Keyframes::StartFrame => Some(Align::StartFrame),
        }
    }
}

/// Apply Grades Using of `grade.append_node`: aligning source timecode where it is left out, as
/// `copy_grade` does.
fn source_timecode() -> Keyframes {
    Keyframes::SourceTimecode
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyRequest {
    still: String,
    to: Vec<Target>,
    #[serde(default)]
    mode: Mode,
    #[serde(default)]
    keyframes: Keyframes,
    #[serde(default)]
    preserve_nodes: u32,
    revision: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendNodeRequest {
    from: Target,
    node: String,
    to: Vec<Target>,
    #[serde(default = "source_timecode")]
    keyframes: Keyframes,
    /// Of No Keyframes: the frame of the timeline whose values are kept (the playhead).
    at: Option<i64>,
    revision: Option<u64>,
}

impl AppendNodeRequest {
    /// The request, unless it gives `at` with keys put on frames: `at` is of No Keyframes alone,
    /// and would be left unread otherwise (`-32602` `{"reason": "at"}`).
    fn checked(self) -> Result<AppendNodeRequest, Fault> {
        if self.at.is_some() && self.keyframes.align().is_some() {
            return Err(Fault::params(
                "at is the frame of No Keyframes (\"keyframes\": \"none\"); with keys put on frames it is not taken",
            )
            .with_data(json!({"reason": "at"})));
        }
        Ok(self)
    }
}

/// The grade `target` names, or the refusal `missing`.
fn grade_of<'a>(project: &'a Project, target: &Target) -> Result<&'a Grade, Fault> {
    let missing = |what: &str, id: &str| {
        refused(
            "missing",
            &format!("there is no {what} {id}"),
            json!({"what": what, "id": id}),
        )
    };
    let timeline = |id: &str| {
        project
            .timelines
            .iter()
            .find(|timeline| timeline.id == id)
            .ok_or_else(|| missing("timeline", id))
    };
    let group = |id: &str| {
        project
            .groups
            .iter()
            .find(|group| group.id == id)
            .ok_or_else(|| missing("group", id))
    };
    match target {
        Target::Clip { timeline: t, clip } => timeline(t)?
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|found| found.id == *clip)
            .map(|found| &found.grade)
            .ok_or_else(|| missing("clip", clip)),
        Target::PreClip { group: g } => Ok(&group(g)?.pre),
        Target::PostClip { group: g } => Ok(&group(g)?.post),
        Target::Timeline { timeline: t } => Ok(&timeline(t)?.grade),
    }
}

/// The targets of `to`, each once, in their order; refused where there are none.
fn targets_once(to: &[Target]) -> Result<Vec<&Target>, Fault> {
    if to.is_empty() {
        return Err(refused(
            "out_of_range",
            "there is no grade to apply to",
            json!({"what": "to"}),
        ));
    }
    let mut seen = HashSet::new();
    Ok(to.iter().filter(|target| seen.insert(*target)).collect())
}

/// The edits of `still.apply`, worked out of `project` as it is: Copy Grade or Append Node Graph
/// of the still's grade, given whole (p. 3290), one of its clip grade and one of its timeline
/// grade where they go, worked out as those of a grade of the project are.
fn work_apply(
    project: &Project,
    still: &Value,
    request: &ApplyRequest,
) -> Result<Vec<Change>, Fault> {
    if request.preserve_nodes > MOST_PRESERVED_NODES {
        return Err(refused(
            "out_of_range",
            "preserve_nodes is from 0 to 10",
            json!({"what": "preserve_nodes"}),
        ));
    }
    if request.mode == Mode::Append && request.preserve_nodes > 0 {
        return Err(
            Fault::params("preserve_nodes is of applying a grade, not of appending one")
                .with_data(json!({"reason": "preserve_nodes"})),
        );
    }
    let targets = targets_once(&request.to)?;
    let mut changes = Vec::new();
    for slot in ["clip", "timeline"] {
        let to: Vec<Target> = targets
            .iter()
            .filter(|target| matches!(target, Target::Timeline { .. }) == (slot == "timeline"))
            .map(|target| (*target).clone())
            .collect();
        if to.is_empty() {
            continue;
        }
        for target in &to {
            grade_of(project, target)?;
        }
        let kept = if request.keyframes == Keyframes::None {
            &still["frozen"][slot]
        } else {
            &still["grades"][slot]
        };
        if !kept.is_object() {
            return Err(refused(
                "no_grade",
                &format!("the still {} has no {slot} grade", request.still),
                json!({"still": request.still, "grade": slot}),
            ));
        }
        let grade: Grade = serde_json::from_value(kept.clone()).map_err(|error| {
            Fault::new(
                rpc::CANNOT_READ,
                format!(
                    "the grade of the still {} cannot be read: {error}",
                    request.still
                ),
            )
            .with_data(json!({"reason": "gallery", "still": request.still}))
        })?;
        let given = Box::new(Given {
            grade,
            keys: still_base(still, slot),
            align: request.keyframes.align(),
        });
        changes.push(Change::Edit(match request.mode {
            Mode::Apply => Edit::CopyGivenGrade {
                given,
                to,
                preserve_nodes: request.preserve_nodes,
            },
            Mode::Append => Edit::AppendGivenGraph { given, to },
        }));
    }
    Ok(changes)
}

/// The frames that the keys of the still's `slot` grade count, as a copy of it needs them: of
/// its timeline, of its clip (where it is the clip's own grade), or none (a group's grade, or
/// one that came from no clip).
fn still_base(still: &Value, slot: &str) -> Option<KeyBase> {
    let source = &still["source"];
    let rate: Rate = serde_json::from_value(source["timeline"]["rate"].clone()).ok()?;
    let mut project = Project::new();
    let timeline = Timeline::new("t", "", rate);
    if slot == "timeline" {
        project.timelines.push(timeline);
        return KeyBase::of(
            &project,
            &Target::Timeline {
                timeline: "t".into(),
            },
        );
    }
    if still["level"] != "clip" {
        return None;
    }
    let clip: Clip = serde_json::from_value(source["clip"].clone()).ok()?;
    if let Ok(media) = serde_json::from_value::<Media>(source["media"].clone()) {
        project.media.push(media);
    }
    Some(KeyBase::of_clip(&clip, &project, &timeline))
}

/// The edit of `grade.append_node`: the node `node` of the grade `from`, alone, appended to the
/// end of each grade of `to`, as Append Node to Selected Clips does (p. 3297), and as Append Node
/// Graph appends a grade (`AppendGivenGraph`). With No Keyframes, each keyed setting of the node is
/// the value of its key at the frame `at` of the timeline or the nearest before it (p. 3393).
fn work_append_node(project: &Project, request: &AppendNodeRequest) -> Result<Vec<Change>, Fault> {
    let to: Vec<Target> = targets_once(&request.to)?.into_iter().cloned().collect();
    let from = grade_of(project, &request.from)?;
    let node = from.node(&request.node).ok_or_else(|| {
        refused(
            "missing",
            &format!("there is no node {}", request.node),
            json!({"what": "node", "id": request.node}),
        )
    })?;
    if node.kind.kind() != NodeKind::Corrector {
        return Err(refused(
            "wrong_node",
            &format!("the node {} is not a corrector", request.node),
            json!({"node": request.node, "kind": node.kind.kind().name()}),
        ));
    }
    let mut alone = from.clone();
    alone.nodes = vec![node.clone()];
    alone.links = vec![
        Link::rgb(SOURCE, node.id.clone()),
        Link::rgb(node.id.clone(), OUTPUT),
    ];
    alone.sort_links();
    let given = if let Some(align) = request.keyframes.align() {
        Given {
            grade: alone,
            keys: KeyBase::of(project, &request.from),
            align: Some(align),
        }
    } else {
        let frame = request
            .at
            .map_or(i64::MIN, |at| key_frame(project, &request.from, at));
        let frozen = freeze(&json!(alone), frame);
        Given {
            grade: serde_json::from_value(frozen).map_err(|error| {
                Fault::new(
                    rpc::CANNOT_READ,
                    format!("the node cannot be frozen: {error}"),
                )
            })?,
            keys: None,
            align: None,
        }
    };
    Ok(vec![Change::Edit(Edit::AppendGivenGraph {
        given: Box::new(given),
        to,
    })])
}

/// The frame that the keys of the grade `target` count at the frame `at` of its timeline: of the
/// clip's source for a clip of media, of the timeline for its grade; `at` itself where it is of a
/// group, whose keys count the frames of whatever clip it is on.
fn key_frame(project: &Project, target: &Target, at: i64) -> i64 {
    if let Target::Clip { timeline, clip } = target {
        let found = project
            .timelines
            .iter()
            .find(|each| each.id == *timeline)
            .and_then(|timeline| {
                timeline
                    .tracks
                    .iter()
                    .flat_map(|track| &track.clips)
                    .find(|each| each.id == *clip)
                    .map(|clip| KeyClock::of(clip, project, timeline.rate).at(at))
            });
        return found.unwrap_or(at);
    }
    at
}

// ---------------------------------------------------------------------------------------------
// Moving, exporting, importing
// ---------------------------------------------------------------------------------------------

/// Move stills to `album`: within a gallery, from one album to another (p. 3265, "Drag and drop
/// a still from the Gallery to an album"); from the project's to the `PowerGrades` or back, a
/// still of the other gallery under a new id. Gives the id of each where it is now.
fn move_stills(places: &Places, stills: &[String], album: &str) -> Result<Value, Fault> {
    let to = gallery::kind_of(album, false)?;
    let ids = gallery::still_ids(stills)?;
    gallery::with_both(places, |project, power| {
        let gallery_of = |kind: Kind| usize::from(kind != Kind::Stills);
        let mut both = [project, power];
        both[gallery_of(to)].album_mut(album)?;
        for (id, kind) in &ids {
            both[gallery_of(*kind)]
                .album_of(id)
                .ok_or_else(|| gallery::missing("still", id))?;
        }
        let mut moved = serde_json::Map::new();
        for (id, kind) in &ids {
            if *kind == to {
                both[gallery_of(to)].move_within(id, album)?;
                moved.insert(id.clone(), json!(id));
            } else {
                let [project, power] = &mut both;
                let (from, into): (&mut Opened, &mut Opened) = if to == Kind::Stills {
                    (power, project)
                } else {
                    (project, power)
                };
                let new = into.copy_in(from, id, album)?;
                into.save()?;
                from.remove_still(id)?;
                moved.insert(id.clone(), json!(new));
            }
        }
        for opened in &mut both {
            opened.save()?;
        }
        Ok(json!({"moved": moved}))
    })
}

/// The first album that the Gallery shows, where a grab or an import goes where none is named.
fn first_album(opened: &Opened) -> Result<String, Fault> {
    opened
        .first_album()
        .map(|album| album.id.clone())
        .ok_or_else(|| gallery::missing("album", "of stills"))
}

/// Restore stills deleted (Ctrl+Z in the Gallery, the interaction reference §9.2): each back in
/// its album where it was, its id kept. Every one of a gallery is checked before any of it is
/// restored. Gives the album of each.
fn restore(places: &Places, stills: &[String]) -> Result<Value, Fault> {
    let ids = gallery::still_ids(stills)?;
    let mut restored = serde_json::Map::new();
    for kind in [Kind::Stills, Kind::Powergrade] {
        let of: Vec<String> = ids
            .iter()
            .filter(|(_, each)| *each == kind)
            .map(|(id, _)| id.clone())
            .collect();
        if of.is_empty() {
            continue;
        }
        gallery::with(places, kind, |opened| {
            for (id, album) in opened.restore_stills(&of)? {
                restored.insert(id, json!(album));
            }
            opened.save()
        })?;
    }
    Ok(json!({"restored": restored}))
}

/// Copy stills into `album` under new ids, as a still dragged onto a memory is copied into it
/// (p. 3272): within a gallery or into the other. Gives the id of each copy.
fn copy_stills(places: &Places, stills: &[String], album: &str) -> Result<Value, Fault> {
    let to = gallery::kind_of(album, false)?;
    let ids = gallery::still_ids(stills)?;
    gallery::with_both(places, |project, power| {
        let gallery_of = |kind: Kind| usize::from(kind != Kind::Stills);
        let mut both = [project, power];
        both[gallery_of(to)].album_mut(album)?;
        for (id, kind) in &ids {
            both[gallery_of(*kind)]
                .album_of(id)
                .ok_or_else(|| gallery::missing("still", id))?;
        }
        let mut copied = serde_json::Map::new();
        for (id, kind) in &ids {
            let new = if *kind == to {
                both[gallery_of(to)].copy_within(id, album)?
            } else {
                let [project, power] = &mut both;
                let (from, into): (&mut Opened, &mut Opened) = if to == Kind::Stills {
                    (power, project)
                } else {
                    (project, power)
                };
                into.copy_in(from, id, album)?
            };
            copied.insert(id.clone(), json!(new));
        }
        both[gallery_of(to)].save()?;
        Ok(json!({"copied": copied}))
    })
}

/// A path that a client names: absolute, or refused.
fn absolute(path: &str, what: &str) -> Result<PathBuf, Fault> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(
            Fault::params(format!("{what} {} is not absolute", path.display()))
                .with_data(json!({"reason": "relative", "path": path})),
        )
    }
}

/// Export stills into `dir`: each its picture and its JSON (in place of `DaVinci`'s DRX, which is
/// not made), named `<prefix>_<number>` (p. 3271: "the name you typed …, followed by an
/// underscore, the still ID number"), or the number alone with no prefix.
fn export(
    places: &Places,
    stills: &[String],
    dir: &str,
    prefix: &str,
    replace: bool,
) -> Result<Value, Fault> {
    let dir = absolute(dir, "the folder")?;
    if !dir.is_dir() {
        return Err(Fault::params(format!("{} is not a folder", dir.display()))
            .with_data(json!({"reason": "not_a_folder", "path": dir})));
    }
    if prefix.contains(['/', '\\', ':', '\0']) || prefix.contains("..") {
        return Err(range("prefix", "a prefix is a name, not a path"));
    }
    let ids = gallery::still_ids(stills)?;
    let mut planned = Vec::new();
    for (id, kind) in &ids {
        let (picture, still) = gallery::with(places, *kind, |opened| {
            let still = opened.still(id)?;
            Ok((opened.picture_file(id, &still), still))
        })?;
        let number = match still["name"].as_str() {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => id.clone(),
        };
        let stem = if prefix.is_empty() {
            number
        } else {
            format!("{prefix}_{number}")
        };
        let extension = picture
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("png")
            .to_string();
        planned.push((
            picture,
            dir.join(format!("{stem}.{extension}")),
            still,
            dir.join(format!("{stem}.json")),
        ));
    }
    if !replace {
        for (_, image, _, json_path) in &planned {
            for path in [image, json_path] {
                if path.exists() {
                    return Err(refused(
                        "exists",
                        &format!("{} is there", path.display()),
                        json!({"path": path}),
                    ));
                }
            }
        }
    }
    let mut written = Vec::new();
    for (picture, image, still, json_path) in planned {
        std::fs::copy(&picture, &image).map_err(|error| write_fault(&image, &error))?;
        std::fs::write(
            &json_path,
            serde_json::to_vec_pretty(&still).expect("a still is written as JSON"),
        )
        .map_err(|error| write_fault(&json_path, &error))?;
        written.push(json!({"image": image, "json": json_path}));
    }
    Ok(json!({"written": written}))
}

fn write_fault(path: &Path, error: &std::io::Error) -> Fault {
    Fault::new(
        rpc::CANNOT_READ,
        format!("cannot write {}: {error}", path.display()),
    )
    .with_data(json!({"reason": "write", "path": path, "message": error.to_string()}))
}

/// Import pictures as stills into `album` (the project's first where none is named): a PNG, a
/// TIFF or an `OpenEXR` each (p. 3270), with the grades of the JSON of the same name beside it
/// where it is a still's, as an export leaves it.
fn import(places: &Places, paths: &[String], album: Option<&str>) -> Result<Value, Fault> {
    if paths.is_empty() {
        return Err(Fault::params("still.import takes at least one path")
            .with_data(json!({"reason": "paths"})));
    }
    let mut read = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let file = absolute(path, "the file").map_err(|fault| {
            fault.with_data(json!({"reason": "relative", "path": path, "index": index}))
        })?;
        let picture = gallery::Picture::read(&file).map_err(|message| {
            Fault::new(
                rpc::CANNOT_READ,
                format!("{} cannot be read as a still: {message}", file.display()),
            )
            .with_data(json!({"path": path, "reason": "not_a_still", "message": message}))
        })?;
        let beside = file.with_extension("json");
        let mut still = std::fs::read(&beside)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter(|still| still["format"] == gallery::FORMAT && still["grades"].is_object())
            .unwrap_or_else(|| {
                json!({
                    "name": "",
                    "label": file.file_stem().map(|stem| stem.to_string_lossy()),
                    "level": "clip",
                    "grades": {"clip": null, "timeline": null},
                    "frozen": {"clip": null, "timeline": null},
                    "keyframes": {"clip": null, "timeline": null},
                    "source": null,
                })
            });
        still["created"] = json!(gallery::now());
        still["imported"] = json!(path);
        let format = match picture.pixels {
            Pixels::Words(_) => ImageFormat::Png,
            Pixels::Halves(_) if picture.within_unit() => ImageFormat::Png,
            Pixels::Halves(_) => ImageFormat::Exr,
        };
        read.push((still, picture, format));
    }
    let kind = match album {
        Some(album) => gallery::kind_of(album, false)?,
        None => Kind::Stills,
    };
    gallery::with(places, kind, |opened| {
        let album = match album {
            Some(album) => album.to_string(),
            None => first_album(opened)?,
        };
        opened.album_mut(&album)?;
        let mut added = Vec::new();
        for (still, picture, format) in read {
            let still = opened.add_still(&album, still, &picture, format)?;
            added.push(gallery::summary(&still, &album));
        }
        Ok(json!({"added": added}))
    })
}

// ---------------------------------------------------------------------------------------------
// Wipes against stills
// ---------------------------------------------------------------------------------------------

/// The params of `viewer.wipe` or `viewer.split` with the file of each still of the Gallery that
/// a reference names by its id (`{"kind": "still", "still": "s3"}`) put in as its `path`; as they
/// are where none does.
///
/// # Errors
/// A still named is not in the Gallery, or its id is of no still.
fn resolve_references(
    project: &str,
    params: Option<&RawValue>,
) -> Result<Option<Box<RawValue>>, Fault> {
    let Some(raw) = params else {
        return Ok(None);
    };
    let Ok(mut value) = serde_json::from_str::<Value>(raw.get()) else {
        return Ok(Some(raw.to_owned()));
    };
    let mut named = Vec::new();
    let mut references: Vec<&mut Value> = Vec::new();
    if let Some(object) = value.as_object_mut() {
        for (name, field) in object.iter_mut() {
            match name.as_str() {
                "reference" => references.push(field),
                "items" => {
                    if let Some(items) = field.as_array_mut() {
                        references.extend(items.iter_mut());
                    }
                }
                _ => {}
            }
        }
    }
    for reference in references {
        if reference["kind"] == "still" {
            if let Some(id) = reference.get("still").and_then(Value::as_str) {
                named.push((id.to_string(), reference));
            }
        }
    }
    if named.is_empty() {
        return Ok(Some(raw.to_owned()));
    }
    let places = Places::of(project);
    for (id, reference) in named {
        if reference.get("path").is_some() {
            return Err(
                Fault::params("a still is named by its path or by its id, not both")
                    .with_data(json!({"reason": "still", "still": id})),
            );
        }
        let kind = gallery::kind_of(&id, true).map_err(|fault| still_fault(&fault, &id))?;
        let path = gallery::with(&places, kind, |opened| {
            let still = opened.still(&id)?;
            Ok(opened.picture_file(&id, &still))
        })
        .map_err(|fault| still_fault(&fault, &id))?;
        reference["path"] = json!(path);
    }
    Ok(Some(
        serde_json::value::to_raw_value(&value).expect("params are written as JSON"),
    ))
}

/// A still of a wipe that is not in the Gallery, as `viewer.wipe` refuses a still it cannot read.
fn still_fault(fault: &Fault, id: &str) -> Fault {
    Fault::params(format!(
        "there is no still {id} in the Gallery: {}",
        fault.message
    ))
    .with_data(json!({"reason": "still", "still": id}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_with_keys_is_frozen_at_the_key_at_or_before_the_frame() {
        let value = json!({"a": {"keys": [
            {"frame": 10, "value": 50.0, "interp": "linear"},
            {"frame": 20, "value": 0.0, "interp": "linear"},
        ]}, "b": [1.0, {"keys": [{"frame": 5, "value": 2.0, "interp": "hold"}]}], "c": 3.0});
        assert_eq!(
            freeze(&value, 15),
            json!({"a": 50.0, "b": [1.0, 2.0], "c": 3.0})
        );
        assert_eq!(freeze(&value, 0)["a"], 50.0);
        assert_eq!(freeze(&value, 20)["a"], 0.0);
        let mut frames = Vec::new();
        key_frames(&value, &mut frames);
        assert_eq!(frames, [10, 20, 5]);
    }

    #[test]
    fn a_frame_to_keep_the_node_at_is_taken_with_no_keyframes_only() {
        let request = |params: Value| -> AppendNodeRequest {
            let mut whole =
                json!({"from": {"level": "timeline", "timeline": "t1"}, "node": "n1", "to": []});
            whole
                .as_object_mut()
                .unwrap()
                .extend(params.as_object().unwrap().clone());
            serde_json::from_value(whole).unwrap()
        };
        assert!(request(json!({"keyframes": "none", "at": 12}))
            .checked()
            .is_ok());
        assert!(request(json!({"keyframes": "start_frame"}))
            .checked()
            .is_ok());
        for params in [
            json!({"keyframes": "source_timecode", "at": 12}),
            json!({"keyframes": "start_frame", "at": 0}),
            json!({"at": 12}),
        ] {
            let Err(fault) = request(params.clone()).checked() else {
                panic!("{params} is taken");
            };
            assert_eq!(fault.code, rpc::INVALID_PARAMS, "{params}");
            assert_eq!(fault.data.unwrap()["reason"], "at", "{params}");
        }
    }

    #[test]
    fn a_timecode_counts_whole_frames_of_a_second() {
        let rate = |num, den| Rate { num, den };
        assert_eq!(timecode(0, rate(25, 1)), "00:00:00:00");
        assert_eq!(timecode(25 * 3661 + 7, rate(25, 1)), "01:01:01:07");
        assert_eq!(timecode(30, rate(30000, 1001)), "00:00:01:00");
        assert_eq!(timecode(-4, rate(24, 1)), "00:00:00:00");
    }
}
