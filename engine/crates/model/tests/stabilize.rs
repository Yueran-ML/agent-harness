//! The stabilizer of a clip (T18a; `crates/model/src/stabilize.rs`): written with every field and
//! read back as it was, a project without one written to the byte as before, set whole by its
//! operation and taken back by the inverse, kept by the clips that an edit makes of a clip, its
//! frames moved with the source of a clip that is replaced, and what a layer of the clip is at a
//! frame: the transform of the mathematics of the stabilizer at the frame of its source, through
//! which the picture of its `media_in` is drawn.

#![allow(
    clippy::float_cmp,
    reason = "transforms worked out the same way twice, which are the same to the bit"
)]

use std::collections::BTreeMap;

use columbina_model::fusion::{Composition, FusedKind, FusedNode, MEDIA_IN, STABILIZER};
use columbina_model::keyframes::{aligned_stabilize, Align, KeyBase};
use columbina_model::resolve::{fused_for, video_at};
use columbina_model::stabilize::{
    motion_of, numbers_of, transform_of, Analysis, Matrix, Method, IDENTITY,
};
use columbina_model::{apply_done, expand, Clip, Edit, Op, Project, Rate, Speed, Stabilize, Time};
use columbina_track::stabilize::{path, stabilized};
use columbina_track::{Mat3, Motion};
use serde_json::{json, Value};

/// The size of the frames that the analyses of the tests measured.
const SIZE: [u32; 2] = [640, 360];

/// A camera that drifts right and shakes, over frames 0 to 99 of the source: a shift, a small
/// turn and a breath of zoom, about the middle of a frame of [`SIZE`].
fn shaking() -> BTreeMap<i64, [f64; 8]> {
    (0..100)
        .map(|frame| {
            let t = f64::from(i32::try_from(frame).unwrap());
            let motion = Motion {
                pan: 1.5 * t + 5.0 * (t * 1.7).sin(),
                tilt: 3.0 * (t * 2.3).cos() - 3.0,
                zoom: 1.0 + 0.008 * (t * 0.9).sin(),
                rotate: 0.006 * (t * 1.3).sin(),
                ..Motion::IDENTITY
            };
            (frame, numbers_of(&motion))
        })
        .collect()
}

/// A stabilizer of Similarity, its Smooth at a half, with the analysis of [`shaking`].
fn measured() -> Stabilize {
    Stabilize {
        method: Method::Similarity,
        smooth: 0.5,
        analysis: Some(Analysis::new(Method::Similarity, SIZE, shaking())),
        ..Stabilize::default()
    }
}

/// The sample project with `stabilize` on its clip of picture, c1.
fn steadied(stabilize: Stabilize) -> Project {
    let mut project = Project::sample();
    project.timelines[0].tracks[0].clips[0].stabilize = Some(stabilize);
    project
}

/// `transform`, of pixels of a frame of `size`, in shares of it, as the model makes it.
fn in_shares(transform: &Mat3, size: (f64, f64)) -> Matrix {
    let scale = [size.0, size.1, 1.0];
    let mut out = [[0.0; 3]; 3];
    for (row, cells) in out.iter_mut().enumerate() {
        for (column, cell) in cells.iter_mut().enumerate() {
            *cell = transform.0[row][column] * scale[column] / scale[row];
        }
    }
    out
}

/// What the mathematics of the stabilizer makes of the analysis of `stabilize` for its Method
/// and controls, frame by frame of the analysis, in shares of the picture.
fn by_the_mathematics(stabilize: &Stabilize) -> Vec<Matrix> {
    let analysis = stabilize.analysis.as_ref().unwrap();
    let size = (f64::from(analysis.size()[0]), f64::from(analysis.size()[1]));
    let middle = [size.0 / 2.0, size.1 / 2.0];
    let motions: Vec<Mat3> = analysis
        .path()
        .values()
        .map(|numbers| motion_of(numbers).matrix(middle))
        .collect();
    let steps: Vec<Mat3> = motions
        .windows(2)
        .map(|pair| pair[1].mul(&pair[0].inverse().unwrap()))
        .collect();
    let camera = path(&steps, middle, stabilize.method).unwrap();
    stabilized(&camera, &stabilize.settings(), size)
        .transforms
        .iter()
        .map(|transform| in_shares(transform, size))
        .collect()
}

#[test]
fn a_clip_is_written_with_its_stabilizer_and_read_back_as_it_was() {
    let project = steadied(measured());
    let text = serde_json::to_string(&project).unwrap();
    let written: Value = serde_json::from_str(&text).unwrap();
    let stabilize = &written["timelines"][0]["tracks"][0]["clips"][0]["stabilize"];
    assert_eq!(
        [
            &stabilize["method"],
            &stabilize["camera_lock"],
            &stabilize["zoom"],
            &stabilize["cropping_ratio"],
            &stabilize["smooth"],
            &stabilize["strength"],
        ],
        [
            &json!("similarity"),
            &json!(false),
            &json!(true),
            &json!(0.5),
            &json!(0.5),
            &json!(1.0),
        ]
    );
    let analysis = &stabilize["analysis"];
    assert_eq!(analysis["method"], "similarity");
    assert_eq!(analysis["size"], json!([640, 360]));
    assert_eq!(analysis["path"].as_object().unwrap().len(), 100);
    assert_eq!(
        analysis["path"]["7"],
        serde_json::to_value(shaking()[&7]).unwrap()
    );
    // The clip of sound has none, and is written without the field.
    let sound = &written["timelines"][0]["tracks"][1]["clips"][0];
    assert_eq!(sound.get("stabilize"), None);
    let read = Project::from_value(written).unwrap();
    assert_eq!(read, project);
    assert_eq!(serde_json::to_string(&read).unwrap(), text);
}

#[test]
fn a_project_without_stabilizers_is_written_to_the_byte_as_it_was() {
    let project = Project::sample();
    let text = serde_json::to_string_pretty(&project).unwrap();
    assert!(!text.contains("stabilize"), "{text}");
    let read = Project::from_value(serde_json::from_str(&text).unwrap()).unwrap();
    assert_eq!(read, project);
    assert_eq!(serde_json::to_string_pretty(&read).unwrap(), text);
    assert_eq!(read.timelines[0].tracks[0].clips[0].stabilize, None);
    // A stabilizer of sound, or out of its range, is no project.
    let mut written: Value = serde_json::from_str(&text).unwrap();
    written["timelines"][0]["tracks"][1]["clips"][0]["stabilize"] =
        serde_json::to_value(Stabilize::default()).unwrap();
    assert!(Project::from_value(written.clone()).is_err());
    written["timelines"][0]["tracks"][1]["clips"][0]
        .as_object_mut()
        .unwrap()
        .remove("stabilize");
    let mut wild = serde_json::to_value(Stabilize::default()).unwrap();
    wild["strength"] = json!(2.0);
    written["timelines"][0]["tracks"][0]["clips"][0]["stabilize"] = wild;
    assert!(Project::from_value(written).is_err());
}

#[test]
fn the_operation_sets_a_stabilizer_whole_as_a_client_writes_it_and_its_inverse_takes_it_back() {
    let mut project = Project::sample();
    let before = project.clone();
    let value = serde_json::to_value(measured()).unwrap();
    let op: Op = serde_json::from_value(
        json!({"op": "set_stabilize", "timeline": "t1", "clip": "c1", "value": value}),
    )
    .unwrap();
    let (done, inverse) = apply_done(&mut project, op.clone()).unwrap();
    assert_eq!(done, op);
    assert_eq!(
        serde_json::to_value(&inverse).unwrap(),
        json!({"op": "set_stabilize", "timeline": "t1", "clip": "c1", "value": null})
    );
    assert_eq!(
        project.timelines[0].tracks[0].clips[0].stabilize,
        Some(measured())
    );
    // The inverse of the inverse is the operation, and the project comes back to the bit.
    let after = project.clone();
    let (_, again) = apply_done(&mut project, inverse).unwrap();
    assert_eq!(project, before);
    assert_eq!(again, op);
    apply_done(&mut project, again).unwrap();
    assert_eq!(project, after);
    // `value` must be written: `null` takes the stabilizer away.
    let unwritten = json!({"op": "set_stabilize", "timeline": "t1", "clip": "c1"});
    assert!(serde_json::from_value::<Op>(unwritten).is_err());
}

#[test]
fn a_layer_is_given_the_transform_of_the_mathematics_at_the_frame_of_its_source() {
    let stabilize = measured();
    let wanted = by_the_mathematics(&stabilize);
    let project = steadied(stabilize);
    let timeline = &project.timelines[0];
    let clip = &timeline.tracks[0].clips[0];
    for frame in [0, 1, 37, 99] {
        let at = usize::try_from(frame).unwrap();
        let layer = video_at(timeline, &project, frame).remove(0);
        assert_eq!(layer.stabilize, Some(wanted[at]), "frame {frame}");
        assert_eq!(
            transform_of(clip, &project, timeline.rate, frame),
            Some(wanted[at])
        );
        // The picture of the `media_in` of the composition that a clip has at first is drawn
        // through it.
        let fused = layer.fusion.unwrap();
        assert_eq!(fused.output, Some(1), "frame {frame}");
        assert_eq!(fused.nodes[0].kind, FusedKind::MediaIn);
        assert_eq!(
            fused.nodes[1],
            FusedNode {
                id: STABILIZER.into(),
                kind: FusedKind::Stabilize {
                    input: Some(0),
                    matrix: wanted[at],
                },
            }
        );
        // And so the Fusion view of the `media_in` shows it.
        let shown = fused_for(timeline, &project, clip, frame, Some(MEDIA_IN)).unwrap();
        assert_eq!(shown, fused);
    }
}

#[test]
fn a_composition_reads_the_picture_stabilized_where_it_read_the_media_in() {
    let mut project = steadied(measured());
    let composition = Composition::default();
    project.timelines[0].tracks[0].clips[0].fusion = Some(composition.clone());
    let layer = video_at(&project.timelines[0], &project, 20).remove(0);
    let at = composition.at_size(0.0, (1920, 1080));
    assert_eq!(layer.fusion, Some(at.stabilized(layer.stabilize.unwrap())));
    // A stabilizer that moves nothing leaves the composition, and a clip without one has none.
    let mut still = measured();
    still.strength = 0.0;
    still.zoom = false;
    let mut project = steadied(still);
    let layer = video_at(&project.timelines[0], &project, 20).remove(0);
    assert_eq!(layer.stabilize, Some(IDENTITY));
    assert_eq!(layer.fusion, None);
    project.timelines[0].tracks[0].clips[0].stabilize = Some(Stabilize::default());
    let layer = video_at(&project.timelines[0], &project, 20).remove(0);
    assert_eq!((layer.stabilize, layer.fusion), (None, None));
}

#[test]
fn the_frames_of_the_source_keep_their_transforms_as_the_clip_is_cut_or_played_faster() {
    let stabilize = measured();
    let wanted = by_the_mathematics(&stabilize);
    let mut project = steadied(stabilize.clone());
    let blade: Edit =
        serde_json::from_value(json!({"edit": "blade", "timeline": "t1", "at": 40})).unwrap();
    expand(&mut project, blade).unwrap();
    let pictures = &project.timelines[0].tracks[0].clips;
    assert_eq!(pictures.len(), 2);
    for half in pictures {
        assert_eq!(half.stabilize.as_ref(), Some(&stabilize), "{}", half.id);
    }
    // Each half puts each picture of the source where the clip put it.
    for frame in [10, 39, 40, 75] {
        let layer = video_at(&project.timelines[0], &project, frame).remove(0);
        let at = usize::try_from(frame).unwrap();
        assert_eq!(layer.stabilize, Some(wanted[at]), "frame {frame}");
    }
    // At twice the speed, frame 10 shows picture 20 of the source, put as it was.
    let mut faster = steadied(stabilize);
    faster.timelines[0].tracks[0].clips[0].speed = Speed { num: 2, den: 1 };
    let layer = video_at(&faster.timelines[0], &faster, 10).remove(0);
    assert_eq!(layer.stabilize, Some(wanted[20]));
    // Beyond the frames of the analysis, the last.
    let mut longer = faster.clone();
    longer.timelines[0].tracks[0].clips[0].length = 120;
    let layer = video_at(&longer.timelines[0], &longer, 110).remove(0);
    assert_eq!(layer.stabilize, Some(wanted[99]));
}

#[test]
fn the_frames_of_an_analysis_go_to_those_of_a_source_of_another_rate() {
    let mut project = Project::sample();
    let mut fifty = project.media[0].clone();
    fifty.id = "m9".into();
    fifty.facts.video.as_mut().unwrap().rate = Rate { num: 50, den: 1 };
    project.media.push(fifty);
    let timeline = project.timelines[0].clone();
    let from = KeyBase::of_clip(&timeline.tracks[0].clips[0], &project, &timeline);
    let start = Time {
        ticks: 0,
        scale: 15360,
    };
    let other = Clip::of_media("c9", "m9", start, 0, 100);
    let to = KeyBase::of_clip(&other, &project, &timeline);
    let moved = aligned_stabilize(&measured(), from, to, Align::SourceTimecode);
    let analysis = moved.analysis.as_ref().unwrap();
    // A picture of 25 a second is at the same time as the picture of twice its number at 50.
    assert_eq!(
        analysis.path().keys().copied().collect::<Vec<_>>(),
        (0..100).map(|frame| 2 * frame).collect::<Vec<_>>()
    );
    assert_eq!(analysis.path()[&14], shaking()[&7]);
    // The controls are as they were, and a clip of the same rate keeps its frames.
    assert_eq!(
        Stabilize {
            analysis: None,
            ..moved.clone()
        },
        Stabilize {
            analysis: None,
            ..measured()
        }
    );
    assert_eq!(
        aligned_stabilize(&measured(), from, from, Align::SourceTimecode),
        measured()
    );
}
