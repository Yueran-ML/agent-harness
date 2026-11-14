//! The keys of grades on the frames of the sources of clips, as `DaVinci Resolve` puts them:
//! edits that change which pictures a clip shows leave each key on its picture, a title counts
//! its own frames, and a grade copied to another clip brings its keys as Apply Grades Using says.

use std::collections::BTreeMap;

use columbina_model::edit::Edge;
use columbina_model::grade::{Interp, Key, Kind, Link, Node, NodeKind, Param};
use columbina_model::keyframes::{Align, KeyClock};
use columbina_model::resolve::{video_at, Content};
use columbina_model::span::media_frame;
use columbina_model::{
    apply, expand, Clip, Edit, Grade, Group, Material, Project, Rate, Source, Speed, Target, Time,
    TitleStyle, TrackKind,
};
use proptest::prelude::*;

const SCALE: u32 = 15360;
const PAL: Rate = Rate { num: 25, den: 1 };
const FIFTY: Rate = Rate { num: 50, den: 1 };

/// The place where frame `frame` of `rate` begins.
fn at(frame: i64, rate: Rate) -> Time {
    Time {
        ticks: rate.ticks_of(frame, SCALE),
        scale: SCALE,
    }
}

/// A grade of one corrector whose gain of R, G and B is keyed at these frames, linearly.
fn gain(keys: &[(i64, f64)]) -> Grade {
    let param = Param::Keys(
        keys.iter()
            .map(|&(frame, value)| Key {
                frame,
                value,
                interp: Interp::Linear,
            })
            .collect(),
    );
    let mut node = Node::new("n1", Kind::Corrector);
    let NodeKind::Corrector(settings) = &mut node.kind else {
        unreachable!("a corrector")
    };
    let g = &mut settings.primaries.gain;
    (g.r, g.g, g.b) = (param.clone(), param.clone(), param);
    Grade {
        nodes: vec![node],
        links: vec![Link::rgb("source", "n1"), Link::rgb("n1", "output")],
    }
}

/// The keys of the gain of R of the first node of `grade`.
fn keys(grade: &Grade) -> Vec<(i64, f64)> {
    let NodeKind::Corrector(settings) = &grade.nodes[0].kind else {
        panic!("a corrector")
    };
    match &settings.primaries.gain.r {
        Param::Keys(keys) => keys.iter().map(|k| (k.frame, k.value)).collect(),
        Param::Fixed(value) => vec![(i64::MIN, *value)],
    }
}

/// The value of the gain of R of the first node of `grade` at `frame`.
fn gain_at(grade: &Grade, frame: i64) -> f64 {
    let NodeKind::Corrector(settings) = &grade.nodes[0].kind else {
        panic!("a corrector")
    };
    #[allow(clippy::cast_precision_loss)]
    settings.primaries.gain.r.value_at(frame as f64)
}

/// The keys at frames 30 and 50 of the source: the gain goes from 1 to 0 between them.
const FADE: [(i64, f64); 2] = [(30, 1.0), (50, 0.0)];

/// A project of the sample's media m1 (25 frames a second, 10 s), and m2 (50 frames a second,
/// 10 s, no sound), whose timeline t1, at 25 frames a second, has on V1 the clip c1 of m1 from
/// its frame 20, at frame 100 for 50 frames, graded by [`FADE`]; and no sound.
fn project() -> Project {
    let mut project = Project::sample();
    let mut m2 = project.media[0].clone();
    m2.id = "m2".into();
    m2.facts.audio = None;
    m2.facts.video.as_mut().unwrap().rate = FIFTY;
    project.media.push(m2);
    let timeline = &mut project.timelines[0];
    timeline
        .tracks
        .retain(|track| track.kind == TrackKind::Video);
    let mut clip = Clip::of_media("c1", "m1", at(20, PAL), 100, 50);
    clip.grade = gain(&FADE);
    timeline.tracks[0].clips = vec![clip];
    project.next_id = 100;
    project
}

fn clip_target(clip: &str) -> Target {
    Target::Clip {
        timeline: "t1".into(),
        clip: clip.into(),
    }
}

fn clip<'a>(project: &'a Project, id: &str) -> &'a Clip {
    project.timelines[0].tracks[0]
        .clips
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no clip {id}"))
}

/// Apply `edit`, and check that its inverse takes the project back.
fn edited(project: &mut Project, edit: Edit) {
    let before = project.clone();
    let text = format!("{edit:?}");
    let expansion = expand(project, edit).unwrap_or_else(|e| panic!("{text}: {e}"));
    let mut undone = project.clone();
    for inverse in expansion.inverse.iter().rev() {
        apply(&mut undone, inverse.clone()).unwrap();
    }
    undone.next_id = before.next_id;
    assert_eq!(undone, before, "{text} undone");
}

/// What V1 of t1 shows from frame 0 to 300: for each frame that shows a clip of media, the
/// clip, the frame of its media whose picture is shown, and the gain of its grade there. The
/// frame of the keys of each clip there is checked to be that of the picture.
fn seen(project: &Project) -> BTreeMap<i64, (String, i64, f64)> {
    let timeline = &project.timelines[0];
    let mut seen = BTreeMap::new();
    for frame in 0..300 {
        for layer in video_at(timeline, project, frame) {
            let Content::Media { media, tick } = &layer.content else {
                continue;
            };
            let media = project.media.iter().find(|m| m.id == *media).unwrap();
            let rate = media.facts.video.as_ref().unwrap().rate;
            let picture = media_frame(*tick, rate, media.facts.duration);
            let shown = clip(project, &layer.clip);
            let clock = KeyClock::of(shown, project, timeline.rate);
            assert!(clock.counts_source());
            assert_eq!(clock.at(frame), picture, "the key frame at {frame}");
            seen.insert(
                frame,
                (layer.clip.clone(), picture, gain_at(&shown.grade, picture)),
            );
        }
    }
    seen
}

/// The gain that each picture of m1 is shown at, which is that of [`FADE`] at its frame.
fn pictures_keep_their_gain(seen: &BTreeMap<i64, (String, i64, f64)>) {
    let fade = gain(&FADE);
    for (frame, (_, picture, value)) in seen {
        assert!(
            (value - gain_at(&fade, *picture)).abs() < 1e-12,
            "frame {frame}, picture {picture}: {value}"
        );
    }
}

#[test]
fn a_key_is_at_the_frame_of_the_media_whose_picture_is_shown() {
    let project = project();
    let seen = seen(&project);
    assert_eq!(seen.len(), 50);
    // Frame 100 shows picture 20, where the gain holds the first key; 110 shows 30, the key.
    assert_eq!(seen[&100], ("c1".into(), 20, 1.0));
    assert_eq!(seen[&110], ("c1".into(), 30, 1.0));
    assert_eq!(seen[&120], ("c1".into(), 40, 0.5));
    assert_eq!(seen[&130], ("c1".into(), 50, 0.0));
    pictures_keep_their_gain(&seen);
}

#[test]
fn trimming_the_head_leaves_the_keys_on_their_pictures() {
    let mut project = project();
    let trim = |delta| Edit::Trim {
        timeline: "t1".into(),
        clip: "c1".into(),
        edge: Edge::In,
        delta,
        ripple: false,
        linked: true,
    };
    edited(&mut project, trim(10));
    let shown = seen(&project);
    // The clip starts at frame 110, on picture 30, and the key is still there.
    assert_eq!(
        shown.first_key_value().unwrap(),
        (&110, &("c1".into(), 30, 1.0))
    );
    assert_eq!(shown[&120], ("c1".into(), 40, 0.5));
    pictures_keep_their_gain(&shown);
    assert_eq!(keys(&clip(&project, "c1").grade), FADE);
    // Trimmed out past where it began: the pictures before the first key hold it.
    edited(&mut project, trim(-15));
    let shown = seen(&project);
    assert_eq!(shown[&95], ("c1".into(), 15, 1.0));
    assert_eq!(shown[&130], ("c1".into(), 50, 0.0));
    pictures_keep_their_gain(&shown);
}

#[test]
fn slipping_leaves_the_keys_on_their_pictures() {
    let mut project = project();
    edited(
        &mut project,
        Edit::Slip {
            timeline: "t1".into(),
            clip: "c1".into(),
            delta: 5,
            linked: true,
        },
    );
    let shown = seen(&project);
    // The clip stays at frame 100, and shows picture 25 there: the key at 30 is at frame 105.
    assert_eq!(shown[&100], ("c1".into(), 25, 1.0));
    assert_eq!(shown[&105], ("c1".into(), 30, 1.0));
    assert_eq!(shown[&125], ("c1".into(), 50, 0.0));
    pictures_keep_their_gain(&shown);
}

#[test]
fn the_blade_splits_the_keys_and_the_two_halves_play_as_the_clip_did() {
    let mut project = project();
    let before: Vec<(i64, f64)> = seen(&project)
        .into_iter()
        .map(|(frame, (_, picture, value))| (frame + picture * 1000, value))
        .collect();
    edited(
        &mut project,
        Edit::Blade {
            timeline: "t1".into(),
            at: 115,
            tracks: None,
            linked: true,
            grouped: false,
        },
    );
    let clips = &project.timelines[0].tracks[0].clips;
    assert_eq!(clips.len(), 2);
    // Both halves keep the grade, each the keys of its own pictures among them.
    assert_eq!(clips[0].grade, clips[1].grade);
    let shown = seen(&project);
    let after: Vec<(i64, f64)> = shown
        .iter()
        .map(|(frame, (_, picture, value))| (frame + picture * 1000, *value))
        .collect();
    assert_eq!(after, before);
    assert_eq!(shown[&114].0, "c1");
    assert_ne!(shown[&115].0, "c1");
    assert_eq!(shown[&115].1, 35);
    assert_eq!(shown[&115].2, 0.75);
}

#[test]
fn at_twice_the_speed_a_key_stays_on_its_picture_and_comes_sooner() {
    let mut project = project();
    edited(
        &mut project,
        Edit::SetSpeed {
            timeline: "t1".into(),
            clip: "c1".into(),
            speed: Speed { num: 2, den: 1 },
            reverse: None,
            ripple: false,
            linked: true,
        },
    );
    assert_eq!(clip(&project, "c1").length, 25);
    let shown = seen(&project);
    assert_eq!(shown.len(), 25);
    // Pictures 20, 22, …: the key at 30 is five frames in, the one at 50 fifteen.
    assert_eq!(shown[&105], ("c1".into(), 30, 1.0));
    assert_eq!(shown[&110], ("c1".into(), 40, 0.5));
    assert_eq!(shown[&115], ("c1".into(), 50, 0.0));
    pictures_keep_their_gain(&shown);
    assert_eq!(keys(&clip(&project, "c1").grade), FADE);
}

#[test]
fn at_half_the_speed_a_picture_shown_twice_has_one_gain() {
    let mut project = project();
    edited(
        &mut project,
        Edit::SetSpeed {
            timeline: "t1".into(),
            clip: "c1".into(),
            speed: Speed { num: 1, den: 2 },
            reverse: None,
            ripple: false,
            linked: true,
        },
    );
    let shown = seen(&project);
    assert_eq!(shown[&120], ("c1".into(), 30, 1.0));
    assert_eq!(shown[&121], ("c1".into(), 30, 1.0));
    assert_eq!(shown[&140], ("c1".into(), 40, 0.5));
    assert_eq!(shown[&141], ("c1".into(), 40, 0.5));
    pictures_keep_their_gain(&shown);
}

#[test]
fn played_backward_the_keys_come_last_first_on_their_pictures() {
    let mut project = project();
    edited(
        &mut project,
        Edit::SetSpeed {
            timeline: "t1".into(),
            clip: "c1".into(),
            speed: Speed::ONE,
            reverse: Some(true),
            ripple: false,
            linked: true,
        },
    );
    let shown = seen(&project);
    assert_eq!(shown[&100], ("c1".into(), 69, 0.0));
    assert_eq!(shown[&119], ("c1".into(), 50, 0.0));
    assert_eq!(shown[&129], ("c1".into(), 40, 0.5));
    assert_eq!(shown[&139], ("c1".into(), 30, 1.0));
    assert_eq!(shown[&149], ("c1".into(), 20, 1.0));
    pictures_keep_their_gain(&shown);
}

#[test]
fn a_title_a_colour_and_a_still_count_the_frames_of_the_clip() {
    let mut project = project();
    let style = TitleStyle {
        font: String::new(),
        size: 40,
        color: columbina_model::Rgba([255; 4]),
        x: 0.5,
        y: 0.5,
        align: columbina_model::TextAlign::Center,
    };
    let source = Source::Title {
        text: "Hi".into(),
        style,
    };
    let mut title = Clip::new("c2", source, 200, 20);
    title.grade = gain(&[(0, 1.0), (10, 0.0)]);
    project.timelines[0].tracks[0].clips.push(title);
    let rate = project.timelines[0].rate;
    let clock = KeyClock::of(clip(&project, "c2"), &project, rate);
    assert!(!clock.counts_source());
    assert_eq!(clock.at(200), 0);
    assert_eq!(clock.at(210), 10);
    assert_eq!(clock.at(195), -5);
    // Its keys count from its head; a trim of its head moves them back by as much, so that
    // they stay at their frames of the timeline, as the keys of media stay on their pictures.
    edited(
        &mut project,
        Edit::Trim {
            timeline: "t1".into(),
            clip: "c2".into(),
            edge: Edge::In,
            delta: 5,
            ripple: false,
            linked: true,
        },
    );
    let clock = KeyClock::of(clip(&project, "c2"), &project, rate);
    assert_eq!(clock.at(205), 0);
    assert_eq!(keys(&clip(&project, "c2").grade), [(-5, 1.0), (5, 0.0)]);
    // A colour, and a still, as a title.
    let colour = Clip::new(
        "c3",
        Source::Color {
            color: columbina_model::Rgba([0, 0, 0, 255]),
        },
        300,
        10,
    );
    assert_eq!(KeyClock::of(&colour, &project, rate).at(303), 3);
    project.media[1].facts.video.as_mut().unwrap().still = true;
    let still = Clip::of_media("c4", "m2", at(40, FIFTY), 300, 10);
    let clock = KeyClock::of(&still, &project, rate);
    assert!(!clock.counts_source());
    assert_eq!(clock.at(304), 4);
}

/// Put a clip of `media` from frame `from` (of the media's rate) on V1 at `start`.
fn put(project: &mut Project, id: &str, media: &str, from: i64, start: i64, length: i64) {
    let rate = project
        .media
        .iter()
        .find(|m| m.id == media)
        .unwrap()
        .facts
        .video
        .as_ref()
        .unwrap()
        .rate;
    let new = Clip::of_media(id, media, at(from, rate), start, length);
    project.timelines[0].tracks[0].clips.push(new);
    project.timelines[0].tracks[0]
        .clips
        .sort_by_key(|c| c.start);
}

fn copy(project: &mut Project, from: &str, to: Target, keyframes: Option<Align>) -> Grade {
    edited(
        project,
        Edit::CopyGrade {
            from: clip_target(from),
            to: vec![to.clone()],
            preserve_nodes: None,
            keyframes,
        },
    );
    match to {
        Target::Clip { clip: id, .. } => clip(project, &id).grade.clone(),
        Target::Timeline { .. } => project.timelines[0].grade.clone(),
        Target::PreClip { group } => project
            .groups
            .iter()
            .find(|k| k.id == group)
            .unwrap()
            .pre
            .clone(),
        Target::PostClip { .. } => unreachable!("not copied to"),
    }
}

#[test]
fn a_grade_copied_to_the_same_source_keeps_its_keys_on_the_same_pictures() {
    let mut project = project();
    // m1 from frame 25: its pictures 25 to 54 are among those of c1, 20 to 69.
    put(&mut project, "c3", "m1", 25, 200, 30);
    let copied = copy(&mut project, "c1", clip_target("c3"), None);
    assert_eq!(keys(&copied), FADE);
    let shown = seen(&project);
    assert_eq!(shown[&205], ("c3".into(), 30, 1.0));
    assert_eq!(shown[&225], ("c3".into(), 50, 0.0));
    // Written as `DaVinci` does at first: the field is left out.
    let edit = Edit::CopyGrade {
        from: clip_target("c1"),
        to: vec![clip_target("c3")],
        preserve_nodes: None,
        keyframes: None,
    };
    assert!(serde_json::to_value(&edit)
        .unwrap()
        .get("keyframes")
        .is_none());
}

#[test]
fn where_no_source_timecode_overlaps_the_keys_align_with_the_start_frames() {
    let mut project = project();
    // m1 from frame 100, after every picture of c1: aligned with the start, frame 20 of c1 at
    // frame 100 of c4.
    put(&mut project, "c4", "m1", 100, 200, 30);
    let copied = copy(&mut project, "c1", clip_target("c4"), None);
    assert_eq!(keys(&copied), [(110, 1.0), (130, 0.0)]);
    let shown = seen(&project);
    assert_eq!(shown[&210], ("c4".into(), 110, 1.0));
    assert_eq!(shown[&220], ("c4".into(), 120, 0.5));
}

#[test]
fn keys_aligned_by_source_timecode_go_to_the_frames_of_the_same_time_at_another_rate() {
    let mut project = project();
    // m2, at 50 a second, from 1 s (its frame 50) for 30 frames of the timeline, 1.2 s: it shows
    // 1 s to 2.2 s, which c1, 0.8 s to 2.8 s, shows too.
    put(&mut project, "c5", "m2", 50, 200, 30);
    let copied = copy(&mut project, "c1", clip_target("c5"), None);
    // Frame 30 of 25 a second is 1.2 s, frame 60 of 50 a second; 50 is 2 s, 100.
    assert_eq!(keys(&copied), [(60, 1.0), (100, 0.0)]);
    let shown = seen(&project);
    // 1.2 s of m2 is shown 0.2 s, 5 frames, into the clip.
    assert_eq!(shown[&205], ("c5".into(), 60, 1.0));
    // Aligned with the start frames instead: frame 20 of c1 at frame 50 of c5, and 10 frames
    // of 25 a second after it 20 of 50.
    let copied = copy(
        &mut project,
        "c1",
        clip_target("c5"),
        Some(Align::StartFrame),
    );
    assert_eq!(keys(&copied), [(70, 1.0), (110, 0.0)]);
}

#[test]
fn keys_from_frames_of_one_rate_that_come_to_one_frame_of_another_are_the_last_of_them() {
    let mut project = project();
    put(&mut project, "c5", "m2", 50, 200, 30);
    project.timelines[0].tracks[0].clips[1].grade = gain(&[(60, 1.0), (61, 0.5), (100, 0.0)]);
    // From 50 a second to 25: frames 60 and 61 are both in frame 30.
    let copied = copy(&mut project, "c5", clip_target("c1"), None);
    assert_eq!(keys(&copied), [(30, 0.5), (50, 0.0)]);
}

#[test]
fn keys_copied_to_or_from_a_clip_of_no_source_align_with_the_start_frames() {
    let mut project = project();
    let colour = Clip::new(
        "c6",
        Source::Color {
            color: columbina_model::Rgba([0, 0, 0, 255]),
        },
        200,
        30,
    );
    project.timelines[0].tracks[0].clips.push(colour);
    let copied = copy(&mut project, "c1", clip_target("c6"), None);
    assert_eq!(keys(&copied), [(10, 1.0), (30, 0.0)]);
    project.timelines[0].tracks[0].clips[1].grade = gain(&[(0, 1.0), (5, 0.0)]);
    let copied = copy(&mut project, "c6", clip_target("c1"), None);
    assert_eq!(keys(&copied), [(20, 1.0), (25, 0.0)]);
    // To the timeline's grade, whose frames are the timeline's: from its first.
    let timeline = Target::Timeline {
        timeline: "t1".into(),
    };
    let copied = copy(&mut project, "c1", timeline, None);
    assert_eq!(keys(&copied), [(0, 1.0), (5, 0.0)]);
}

#[test]
fn keys_copied_to_a_group_which_is_of_no_one_clip_stay_at_their_frames() {
    let mut project = project();
    project.groups.push(Group::new("k1", "Scene"));
    let pre = Target::PreClip { group: "k1".into() };
    let copied = copy(&mut project, "c1", pre, None);
    assert_eq!(keys(&copied), FADE);
}

#[test]
fn a_reversed_clip_starts_at_its_in_point() {
    let mut project = project();
    put(&mut project, "c4", "m1", 100, 200, 30);
    project.timelines[0].tracks[0].clips[0].reverse = true;
    // c1 shows 69 down to 20; its start, as keys are aligned, is still picture 20.
    let copied = copy(&mut project, "c1", clip_target("c4"), None);
    assert_eq!(keys(&copied), [(110, 1.0), (130, 0.0)]);
}

#[test]
fn append_node_graph_aligns_the_keys_of_the_nodes_it_adds() {
    let mut project = project();
    put(&mut project, "c4", "m1", 100, 200, 30);
    edited(
        &mut project,
        Edit::AppendNodeGraph {
            from: clip_target("c1"),
            to: vec![clip_target("c4")],
            keyframes: Some(Align::StartFrame),
        },
    );
    let grade = &clip(&project, "c4").grade;
    assert_eq!(grade.nodes.len(), 1);
    assert_eq!(keys(grade), [(110, 1.0), (130, 0.0)]);
    let written = serde_json::to_value(Edit::AppendNodeGraph {
        from: clip_target("c1"),
        to: vec![clip_target("c4")],
        keyframes: Some(Align::StartFrame),
    })
    .unwrap();
    assert_eq!(written["keyframes"], "start_frame");
}

#[test]
fn a_replacing_source_takes_the_grade_with_its_keys_on_its_frames() {
    let mut project = project();
    // The transform has keys at the frames of the grade's.
    let opacity = Param::Keys(
        FADE.iter()
            .map(|&(frame, value)| Key {
                frame,
                value,
                interp: Interp::Linear,
            })
            .collect(),
    );
    project.timelines[0].tracks[0].clips[0].transform.opacity = opacity;
    edited(
        &mut project,
        Edit::Replace {
            timeline: "t1".into(),
            source: Material::Media {
                media: "m2".into(),
                from: None,
                to: None,
                length: None,
            },
            at: 110,
            video_track: None,
            audio_track: None,
        },
    );
    let replaced = &project.timelines[0].tracks[0].clips[0];
    let Source::Media { media, .. } = &replaced.source else {
        panic!("media")
    };
    assert_eq!(media, "m2");
    // m2 from its start shows 0 s to 2 s, as c1 showed 0.8 s to 2.8 s: by source timecode.
    assert_eq!(keys(&replaced.grade), [(60, 1.0), (100, 0.0)]);
    // And the keys of its transform went with them.
    let Param::Keys(opacity) = &replaced.transform.opacity else {
        panic!("keys")
    };
    let frames: Vec<(i64, f64)> = opacity.iter().map(|k| (k.frame, k.value)).collect();
    assert_eq!(frames, [(60, 1.0), (100, 0.0)]);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// Whatever edits trim, slip, cut and change the speed of a clip, the keys of its grade stay
    /// at their frames, every frame shows the gain of the picture it shows, and the frame of the
    /// keys is the frame of that picture.
    #[test]
    fn edits_of_a_clip_leave_each_key_on_its_picture(
        edits in prop::collection::vec((0usize..6, -12i64..12, 0usize..4), 1..8),
    ) {
        let mut project = project();
        for (kind, delta, which) in edits {
            let clips: Vec<String> = project.timelines[0].tracks[0]
                .clips
                .iter()
                .map(|c| c.id.clone())
                .collect();
            let clip = clips[which % clips.len()].clone();
            let found = clip_of(&project, &clip);
            let edit = match kind {
                0 | 1 => Edit::Trim {
                    timeline: "t1".into(),
                    clip,
                    edge: if kind == 0 { Edge::In } else { Edge::Out },
                    delta,
                    ripple: false,
                    linked: true,
                },
                2 => Edit::Slip { timeline: "t1".into(), clip, delta, linked: true },
                3 => Edit::Blade {
                    timeline: "t1".into(),
                    at: found.start + delta.rem_euclid(found.length.max(1)),
                    tracks: None,
                    linked: true,
                    grouped: false,
                },
                4 => Edit::SetSpeed {
                    timeline: "t1".into(),
                    clip,
                    speed: Speed {
                        num: u32::try_from(delta.rem_euclid(3) + 1).unwrap(),
                        den: u32::try_from(delta.rem_euclid(2) + 1).unwrap(),
                    },
                    reverse: Some(delta < 0),
                    ripple: false,
                    linked: true,
                },
                _ => Edit::Slide { timeline: "t1".into(), clip, delta, linked: true },
            };
            let mut next = project.clone();
            if expand(&mut next, edit).is_ok() {
                project = next;
            }
        }
        for clip in &project.timelines[0].tracks[0].clips {
            prop_assert_eq!(keys(&clip.grade), FADE.to_vec());
        }
        pictures_keep_their_gain(&seen(&project));
    }
}

fn clip_of(project: &Project, id: &str) -> Clip {
    clip(project, id).clone()
}
