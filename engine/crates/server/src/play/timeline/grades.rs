//! The grades of a timeline as its monitor draws them: each held to the maths of the grade on the
//! processor (`columbina_compose::grade::evaluate`), at the frame of its keys: of a clip of
//! media, the frame of the picture shown; of a colour or a title, the frame of the clip.

use std::path::Path;

use columbina_compose::grade::{evaluate, GradeOptions};
use columbina_model::grade::{Corrector, Interp, Key, Kind, Link, Node, NodeKind, Param};
use columbina_model::{Clip, Cue, Group, Rgba, Source as ClipSource, Time, Track};

use super::*;

/// A monitor of no view, whose LUTs are read from `dir` alone.
fn monitor(dir: &Path) -> Worker {
    let (_, inbox) = mpsc::channel();
    Worker {
        inbox,
        shared: Arc::default(),
        notify: Arc::new(|_| {}),
        view: Arc::default(),
        frames: Registry::default(),
        revision: 0,
        window: crate::window::Update::default(),
        frozen: 0,
        graphics: Graphics::new(),
        sources: Sources::new(Hardware::Off, Arc::new(|_, _| {})),
        grading: Grading::new(LutPlaces::with_library(dir, vec![])),
        output: None,
        scopes: Scopes::new(true, Graphics::new()),
        compare: Comparison::new(),
        status: Status {
            timeline: Some("t1".into()),
            ..Status::default()
        },
        project: None,
        timeline: None,
        generation: 0,
        readers: BTreeMap::new(),
        reader_budget: 0,
        previous: BTreeMap::new(),
        compositor: None,
        targets: Vec::new(),
        current: None,
        aside: false,
        clock: None,
        sound: SoundSlot::new(crate::audio::Kind::Null),
        mixed_from: None,
        heard: Cell::new(0),
        fine: None,
        epoch: Instant::now(),
        settle: None,
        told: None,
    }
}

const ORANGE: [u8; 4] = [200, 110, 40, 255];
const TEAL: [u8; 4] = [40, 150, 170, 255];

/// A clip of a colour.
fn colour(id: &str, rgba: [u8; 4], start: i64, length: i64) -> Clip {
    Clip::new(id, ClipSource::Color { color: Rgba(rgba) }, start, length)
}

/// A project whose timeline, t1, of `width` × `height`, has these clips on its one track of
/// picture, and no sound.
fn timeline_of(clips: Vec<Clip>, width: u32, height: u32) -> Project {
    let mut project = Project::sample();
    let timeline = &mut project.timelines[0];
    (timeline.width, timeline.height) = (width, height);
    timeline.tracks = vec![Track::new("V1", TrackKind::Video).with_clips(clips)];
    project
}

/// A grade of one corrector, set by `set`.
fn corrector(set: impl FnOnce(&mut Corrector)) -> Grade {
    let mut node = Node::new("n1", Kind::Corrector);
    let NodeKind::Corrector(settings) = &mut node.kind else {
        unreachable!("a corrector")
    };
    set(settings);
    Grade {
        nodes: vec![node],
        links: vec![Link::rgb("source", "n1"), Link::rgb("n1", "output")],
    }
}

fn grey() -> Grade {
    corrector(|settings| settings.primaries.saturation = Param::Fixed(0.0))
}

/// Gain of R, G and B, keyed.
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
    // The master ring, Y with R, G and B (p3062): with Lum Mix at 100 a gain of R, G and B alone
    // keeps the luminance that Y sets.
    corrector(|settings| {
        let gain = &mut settings.primaries.gain;
        (gain.y, gain.r, gain.g, gain.b) = (param.clone(), param.clone(), param.clone(), param);
    })
}

/// What the monitor shows of `project` at `frame`, as R, G, B and A of 8 bits a pixel.
fn shown(monitor: &mut Worker, project: &Project, frame: i64) -> Vec<u8> {
    monitor.replace(0, Arc::new(project.clone()));
    monitor.status.frame = Some(frame);
    monitor.draw(true, false).unwrap();
    // Drawn again once the pipelines of its grades are made, as the monitor does.
    monitor.finish_compiling();
    monitor.current.as_ref().unwrap().read_rgba().unwrap()
}

#[test]
fn presentation_of_a_missing_source_frame_is_not_editable() {
    let Some(media) = crate::play::sources::tests::sample_of(crate::test_media::Sample::Collide)
    else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let rate = media.facts.video.as_ref().unwrap().rate;
    let clip = Clip::of_media(
        "c1",
        &media.id,
        Time {
            ticks: 0,
            scale: media.facts.duration.scale,
        },
        0,
        30,
    );
    let mut project = timeline_of(vec![clip], 64, 36);
    project.timelines[0].rate = rate;
    project.media = vec![media];
    monitor.replace(25, Arc::new(project));
    // Collide has no frame numbered 4: decoded_at(4) legitimately gives frame 3.
    // A later request with its own actual frame remains editable.
    for (frame, decoded, editable) in [(4, 3, false), (6, 6, true)] {
        monitor.status.frame = Some(frame);
        let snapshot = monitor.draw_inner(true, false).unwrap().unwrap();
        let read = &monitor
            .previous
            .get(&("V1".into(), "c1".into()))
            .unwrap()
            .read;
        assert!(read.exact, "the decoder was asked for the requested frame");
        assert!(
            monitor.status.exact,
            "the existing playback exact flag is unchanged"
        );
        assert_eq!(monitor.status.shown, Some(frame));
        assert_eq!(read.picture.frame, decoded);
        let layer = &snapshot.layers[0];
        let source = layer.origin.media.as_ref().unwrap();
        assert_eq!(source.requested_frame, frame);
        assert_eq!(source.decoded_frame, decoded);
        assert_eq!(layer.origin.key_frame, decoded);
        assert!(snapshot.ready);
        assert_eq!(layer.editable, editable);
        assert_eq!(snapshot.editable(), editable);
    }
}

#[test]
fn presentation_fallback_keeps_the_original_clip_and_source_clock_across_two_cuts() {
    let Some(media) = crate::play::sources::tests::sample() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let rate = media.facts.video.as_ref().unwrap().rate;
    let scale = media.facts.duration.scale;
    let clip = |id: &str, start, source| {
        Clip::of_media(
            id,
            &media.id,
            Time {
                ticks: rate.ticks_of(source, scale),
                scale,
            },
            start,
            10,
        )
    };
    let mut project = timeline_of(
        vec![clip("c1", 0, 10), clip("c2", 10, 60), clip("c3", 20, 80)],
        64,
        36,
    );
    project.media = vec![media];
    let project = Arc::new(project);
    monitor.replace(17, Arc::clone(&project));
    monitor.status.frame = Some(4);
    let first = monitor.draw_inner(true, false).unwrap().unwrap();
    assert_eq!(first.revision, 17);
    assert!(Arc::ptr_eq(&first.project, &project));
    let actual = first.layers[0].origin.media.as_ref().unwrap();
    assert_eq!(actual.decoded_frame, 14);
    assert_eq!(first.layers[0].origin.key_frame, 14);
    // With no available readers both cuts must take the existing fallback path.
    monitor.readers.clear();
    for (frame, destination) in [(10, "c2"), (20, "c3")] {
        monitor.status.frame = Some(frame);
        let snapshot = monitor.draw_inner(false, false).unwrap().unwrap();
        let layer = &snapshot.layers[0];
        assert_eq!(layer.clip, destination);
        assert_eq!(layer.origin.clip, "c1");
        assert_eq!(layer.origin.timeline_frame, 4);
        assert_eq!(layer.origin.key_frame, 14);
        assert_eq!(layer.origin.media.as_ref().unwrap(), actual);
        assert!(!layer.editable);
        assert!(!snapshot.editable());
    }
}

#[test]
fn presentation_uses_the_highlight_filtered_scene_and_split_is_not_editable() {
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let mut chosen = colour("c2", TEAL, 3, 30);
    chosen.grade = grey();
    let mut project = timeline_of(vec![colour("c1", ORANGE, 0, 40)], 64, 36);
    project.timelines[0]
        .tracks
        .push(Track::new("V2", TrackKind::Video).with_clips(vec![chosen]));
    monitor.replace(23, Arc::new(project));
    monitor.status.frame = Some(7);
    monitor.status.highlight = Some(Highlight {
        mode: crate::highlight::Mode::Bw,
        grade: columbina_model::grade::Target::Clip {
            timeline: "t1".into(),
            clip: "c2".into(),
        },
        node: "n1".into(),
    });
    let snapshot = monitor.draw_inner(true, false).unwrap().unwrap();
    assert_eq!(snapshot.layers.len(), 1);
    assert_eq!(snapshot.layers[0].clip, "c2");
    assert_eq!(snapshot.layers[0].origin.key_frame, 4);
    assert_eq!(snapshot.layers[0].opacity, 1.0);
    let raw = serde_json::value::RawValue::from_string(r#"{"items":[{"kind":"current"}]}"#.into())
        .unwrap();
    let change = Change::read("viewer.split", Some(&raw)).unwrap();
    monitor.compare_with(change).unwrap();
    let split = monitor.draw_inner(true, false).unwrap().unwrap();
    assert_eq!(split.comparison["mode"], "split");
    assert!(!split.editable());
    assert_eq!(
        snapshot.comparison["mode"], "off",
        "an earlier snapshot is immutable"
    );
}

#[test]
fn presentation_still_uses_the_clip_clock_while_its_decoded_frame_stays_zero() {
    let Some(mut media) = crate::play::sources::tests::sample() else {
        return;
    };
    media.facts.video.as_mut().unwrap().still = true;
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let clip = Clip::of_media(
        "c1",
        &media.id,
        Time {
            ticks: 0,
            scale: media.facts.duration.scale,
        },
        5,
        30,
    );
    let mut project = timeline_of(vec![clip], 64, 36);
    project.media = vec![media];
    monitor.replace(24, Arc::new(project));
    monitor.status.frame = Some(12);
    let snapshot = monitor.draw_inner(true, false).unwrap().unwrap();
    assert_eq!(snapshot.layers[0].origin.key_frame, 7);
    assert_eq!(
        snapshot.layers[0]
            .origin
            .media
            .as_ref()
            .unwrap()
            .decoded_frame,
        0
    );
}

/// The pixel at `x`, `y` of a picture `width` wide.
fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = 4 * (y * width + x) as usize;
    rgba[at..at + 4].try_into().unwrap()
}

/// What grades done one after another, each at its frame, make of a colour, on the processor,
/// in 8 bits.
fn expected(rgba: [u8; 4], grades: &[(&Grade, f64)]) -> [u8; 3] {
    let mut rgb = [0, 1, 2].map(|i| f32::from(rgba[i]) / 255.0);
    for (grade, frame) in grades {
        rgb = evaluate(grade, *frame, &GradeOptions::default(), rgb);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    rgb.map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn near(got: [u8; 4], wanted: [u8; 3], what: &str) {
    for i in 0..3 {
        assert!(
            got[i].abs_diff(wanted[i]) <= 1,
            "{what}: {got:?}, and {wanted:?} was wanted"
        );
    }
}

fn is_grey(rgba: [u8; 4]) -> bool {
    rgba[0].abs_diff(rgba[1]) <= 1 && rgba[1].abs_diff(rgba[2]) <= 1
}

#[test]
fn a_grade_shows_as_it_is_set_and_goes_as_it_is_taken_away() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let plain = timeline_of(vec![colour("c1", ORANGE, 0, 50)], 64, 36);
    let before = shown(&mut monitor, &plain, 5);
    near(pixel(&before, 64, 32, 18), [200, 110, 40], "ungraded");
    let read_ahead = monitor.generation;
    let made = monitor.grading.made();

    let mut graded = plain.clone();
    graded.timelines[0].tracks[0].clips[0].grade = grey();
    assert!(
        monitor.replace(0, Arc::new(graded.clone())),
        "a grade is seen"
    );
    assert_eq!(
        monitor.generation, read_ahead,
        "a grade reads no picture again"
    );
    let grey_picture = shown(&mut monitor, &graded, 5);
    let at = pixel(&grey_picture, 64, 32, 18);
    assert!(is_grey(at), "{at:?}");
    near(at, expected(ORANGE, &[(&grey(), 5.0)]), "grey");
    assert_eq!(monitor.grading.made(), made + 1);

    // Another change of the project, which leaves the grade as it was: nothing is made again.
    let mut renamed = graded.clone();
    renamed.media[0].name = "Another".into();
    shown(&mut monitor, &renamed, 6);
    assert_eq!(monitor.grading.made(), made + 1);

    // Taken away (as an undo takes it): the colour as it was, to the bit.
    assert_eq!(shown(&mut monitor, &plain, 5), before);
    assert_eq!(monitor.status.missing_luts, []);
}

#[test]
fn keyed_gain_of_a_colour_changes_over_the_frames_of_the_clip_whatever_its_speed() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let fading = gain(&[(0, 1.0), (40, 0.0)]);
    let mut clip = colour("c1", ORANGE, 10, 60);
    clip.grade = fading.clone();
    let mut project = timeline_of(vec![clip], 64, 36);
    let mut seen = Vec::new();
    for frame in [10, 30, 50, 69] {
        let at = pixel(&shown(&mut monitor, &project, frame), 64, 32, 18);
        #[allow(clippy::cast_precision_loss)]
        near(
            at,
            expected(ORANGE, &[(&fading, (frame - 10) as f64)]),
            &format!("frame {frame}"),
        );
        seen.push(at);
    }
    assert!(
        seen[0][0] > seen[1][0] && seen[1][0] > seen[2][0],
        "{seen:?}"
    );
    assert_eq!(seen[2], [0, 0, 0, 255]);
    // A colour has no frames of its own: its keys count the frames of the clip from its first,
    // and a clip played at another speed, or backwards, keeps them where they were.
    let clip = &mut project.timelines[0].tracks[0].clips[0];
    clip.speed = columbina_model::Speed { num: 2, den: 1 };
    clip.reverse = true;
    near(
        pixel(&shown(&mut monitor, &project, 30), 64, 32, 18),
        expected(ORANGE, &[(&fading, 20.0)]),
        "at another speed",
    );
}

/// The number that a frame of the numbered media carries, read from the picture of a timeline
/// of its size that shows it whole and ungraded: three blocks of 64 pixels in its top left
/// corner, each a hexadecimal digit `d`, least significant first, of luma `16 + 14·d` of limited
/// range, which is `14·d·255/219` once made R'G'B' of full range.
fn number_shown(rgba: &[u8], width: u32) -> i64 {
    (0..3_u32).rev().fold(0, |number, block| {
        let [r, g, b, _] = pixel(rgba, width, 64 * block + 32, 32);
        assert!(
            r.abs_diff(g) <= 3 && g.abs_diff(b) <= 3,
            "a grey block: {r} {g} {b}"
        );
        let digit = (f64::from(r) * 219.0 / 255.0 / 14.0).round();
        assert!((0.0..16.0).contains(&digit), "a digit: {r}");
        #[allow(clippy::cast_possible_truncation)]
        let digit = digit as i64;
        number * 16 + digit
    })
}

/// Places of the picture of the numbered media, away from its number, that are bright.
fn bright_places(rgba: &[u8], width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut places = Vec::new();
    for y in (height / 12..height).step_by((height / 12) as usize) {
        for x in (width / 16..width).step_by((width / 16) as usize) {
            let [r, g, b, _] = pixel(rgba, width, x, y);
            if u32::from(r) + u32::from(g) + u32::from(b) > 150 {
                places.push((x, y));
            }
        }
    }
    assert!(places.len() >= 3, "{places:?}");
    places
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "each edit, and the frames it is seen at"
)]
fn a_clip_of_media_is_graded_at_the_frame_of_the_picture_it_shows() {
    use columbina_model::edit::Edge;
    use columbina_model::keyframes::KeyClock;
    use columbina_model::{expand, Edit, Speed};
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let Some(media) = crate::play::sources::tests::sample() else {
        return;
    };
    let (width, height) = (1920, 1080);
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    // The numbered media, 25 frames a second, from its frame 10 for 40 frames, on a timeline of
    // 25 frames a second; its gain keyed at its frames 20 and 60.
    let fading = gain(&[(20, 1.0), (60, 0.2)]);
    let mut plain = timeline_of(vec![], width, height);
    plain.media = vec![media.clone()];
    let rate = media.facts.video.as_ref().unwrap().rate;
    let from = Time {
        ticks: rate.ticks_of(10, media.facts.duration.scale),
        scale: media.facts.duration.scale,
    };
    let mut clip = Clip::of_media("c1", "m1", from, 0, 40);
    clip.grade = fading.clone();
    plain.timelines[0].tracks[0].clips = vec![clip];
    plain.next_id = 100;
    let edited = |edit: Edit| {
        let mut project = plain.clone();
        expand(&mut project, edit).unwrap();
        project
    };
    let edits = [
        ("as it is", plain.clone(), vec![0, 9, 25, 39]),
        (
            "its head trimmed",
            edited(Edit::Trim {
                timeline: "t1".into(),
                clip: "c1".into(),
                edge: Edge::In,
                delta: 8,
                ripple: false,
                linked: true,
            }),
            vec![8, 12, 30],
        ),
        (
            "slipped",
            edited(Edit::Slip {
                timeline: "t1".into(),
                clip: "c1".into(),
                delta: 6,
                linked: true,
            }),
            vec![0, 14, 33],
        ),
        (
            "cut",
            edited(Edit::Blade {
                timeline: "t1".into(),
                at: 15,
                tracks: None,
                linked: true,
            }),
            vec![14, 15, 30],
        ),
        (
            "twice as fast",
            edited(Edit::SetSpeed {
                timeline: "t1".into(),
                clip: "c1".into(),
                speed: Speed { num: 2, den: 1 },
                reverse: None,
                ripple: false,
                linked: true,
            }),
            vec![0, 7, 19],
        ),
        (
            "backward",
            edited(Edit::SetSpeed {
                timeline: "t1".into(),
                clip: "c1".into(),
                speed: Speed::ONE,
                reverse: Some(true),
                ripple: false,
                linked: true,
            }),
            vec![0, 20, 39],
        ),
    ];
    for (what, graded, frames) in edits {
        let mut ungraded = graded.clone();
        for clip in &mut ungraded.timelines[0].tracks[0].clips {
            clip.grade = Grade::default();
        }
        for frame in frames {
            let picture = shown(&mut monitor, &ungraded, frame);
            let number = number_shown(&picture, width);
            // The picture is the one that the model says the clip shows there, and its keys
            // count that frame.
            let timeline = &graded.timelines[0];
            let clip = timeline.tracks[0]
                .clips
                .iter()
                .find(|c| c.start <= frame && frame < c.end())
                .unwrap();
            let clock = KeyClock::of(clip, &graded, timeline.rate);
            assert_eq!(clock.at(frame), number, "{what}, frame {frame}");
            let places = bright_places(&picture, width, height);
            let got = shown(&mut monitor, &graded, frame);
            for (x, y) in places {
                let before = pixel(&picture, width, x, y);
                #[allow(clippy::cast_precision_loss)]
                let wanted = expected(before, &[(&fading, number as f64)]);
                let after = pixel(&got, width, x, y);
                for i in 0..3 {
                    assert!(
                        after[i].abs_diff(wanted[i]) <= 2,
                        "{what}, frame {frame} (picture {number}) at {x}, {y}: {after:?} from \
                         {before:?}, and {wanted:?} was wanted"
                    );
                }
            }
        }
    }
}

#[test]
fn a_group_grades_each_of_its_clips_between_its_pre_and_post_clip_grades() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let pre = gain(&[(0, 0.5)]);
    let own = grey();
    // A red post-clip grade, which a grey clip grade after it would take away.
    let post = corrector(|settings| settings.primaries.gain.r = Param::Fixed(2.0));
    let mut first = colour("c1", ORANGE, 0, 20);
    first.grade = own.clone();
    let mut second = colour("c3", TEAL, 20, 20);
    let third = colour("c4", TEAL, 40, 20);
    first.group = Some("k1".into());
    second.group = Some("k1".into());
    let mut project = timeline_of(vec![first, second, third], 64, 36);
    project.groups = vec![Group {
        pre: pre.clone(),
        post: post.clone(),
        ..Group::new("k1", "Interview")
    }];
    let at = |monitor: &mut Worker, frame| pixel(&shown(monitor, &project, frame), 64, 32, 18);
    let graded = at(&mut monitor, 5);
    near(
        graded,
        expected(ORANGE, &[(&pre, 5.0), (&own, 5.0), (&post, 5.0)]),
        "the first clip of the group",
    );
    assert!(graded[0] > graded[1] + 20, "red after grey: {graded:?}");
    near(
        at(&mut monitor, 25),
        expected(TEAL, &[(&pre, 5.0), (&post, 5.0)]),
        "the second clip of the group",
    );
    near(at(&mut monitor, 45), [40, 150, 170], "a clip in no group");
}

#[test]
fn the_timeline_grade_is_over_every_layer_and_under_the_subtitles() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let mut small = colour("c2", TEAL, 0, 50);
    small.transform.scale = 0.5;
    let mut project = timeline_of(vec![colour("c1", ORANGE, 0, 50)], 640, 360);
    let timeline = &mut project.timelines[0];
    timeline
        .tracks
        .push(Track::new("V2", TrackKind::Video).with_clips(vec![small]));
    let mut subtitles = Track::new("S1", TrackKind::Subtitle);
    subtitles.cues = vec![Cue {
        id: "q1".into(),
        start: 0,
        length: 50,
        text: "WWWWWWWWWWWW".into(),
    }];
    timeline.tracks.push(subtitles);
    let ungraded = shown(&mut monitor, &project, 5);
    near(
        pixel(&ungraded, 640, 320, 120),
        [40, 150, 170],
        "the top layer",
    );
    project.timelines[0].grade = gain(&[(0, 0.0)]);
    let graded = shown(&mut monitor, &project, 5);
    let rows = |picture: &[u8], rows: std::ops::Range<u32>| -> Vec<[u8; 4]> {
        rows.flat_map(|y| (0..640).map(move |x| (x, y)))
            .map(|(x, y)| pixel(picture, 640, x, y))
            .collect()
    };
    assert!(
        rows(&graded, 0..240).iter().all(|p| p[..3] == [0, 0, 0]),
        "every layer is black under a gain of 0"
    );
    let white = |picture: &[u8]| {
        rows(picture, 240..360)
            .iter()
            .filter(|p| p[0] > 230 && p[1] > 230 && p[2] > 230)
            .count()
    };
    assert!(white(&graded) > 20, "the subtitles are drawn over it");
    assert!(white(&ungraded) > 20);
}

#[test]
fn a_grade_that_is_not_complete_leaves_the_picture_as_it_is() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let plain = timeline_of(vec![colour("c1", ORANGE, 0, 50)], 64, 36);
    let before = shown(&mut monitor, &plain, 5);
    let mut loose = grey();
    // The node takes the picture, and its output goes nowhere: the source goes to the output.
    loose.links = vec![Link::rgb("source", "n1"), Link::rgb("source", "output")];
    assert!(!loose.is_complete());
    let mut project = plain.clone();
    project.timelines[0].tracks[0].clips[0].grade = loose.clone();
    project.timelines[0].grade = loose;
    assert_eq!(shown(&mut monitor, &project, 5), before);
}

/// A cube of 2 points a side that turns each colour into its complement.
const INVERT: &str = "LUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n";

#[test]
fn a_lut_that_cannot_be_read_is_told_and_the_node_has_none() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("invert.cube"), INVERT).unwrap();
    let mut monitor = monitor(dir.path());
    let with_lut = |name: &str| {
        corrector(|settings| {
            settings.primaries.saturation = Param::Fixed(0.0);
            settings.lut = Some(name.into());
        })
    };
    let mut project = timeline_of(vec![colour("c1", ORANGE, 0, 50)], 64, 36);
    project.timelines[0].tracks[0].clips[0].grade = with_lut("looks/gone.cube");
    let at = pixel(&shown(&mut monitor, &project, 5), 64, 32, 18);
    near(at, expected(ORANGE, &[(&grey(), 5.0)]), "grey, and no LUT");
    let missing = &monitor.status.missing_luts;
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert_eq!(missing[0].name, "looks/gone.cube");
    assert!(
        missing[0].message.contains("looks/gone.cube"),
        "{missing:?}"
    );
    let told = json!(monitor.status);
    assert_eq!(told["missing_luts"][0]["name"], "looks/gone.cube", "{told}");

    // The LUT that is there turns the grey into its complement.
    let mut found = project.clone();
    found.timelines[0].tracks[0].clips[0].grade = with_lut("invert.cube");
    let inverted = pixel(&shown(&mut monitor, &found, 5), 64, 32, 18);
    let [y, ..] = expected(ORANGE, &[(&grey(), 5.0)]);
    near(inverted, [255 - y; 3], "inverted");
    assert_eq!(monitor.status.missing_luts, []);
    assert!(json!(monitor.status).get("missing_luts").is_none());

    // A LUT put where one was missing is found at the next change of the project, though the
    // grade is as it was.
    shown(&mut monitor, &project, 5);
    assert_eq!(monitor.status.missing_luts.len(), 1);
    std::fs::create_dir(dir.path().join("looks")).unwrap();
    std::fs::write(dir.path().join("looks").join("gone.cube"), INVERT).unwrap();
    let mut renamed = project.clone();
    renamed.media[0].name = "Another".into();
    let now = pixel(&shown(&mut monitor, &renamed, 5), 64, 32, 18);
    near(now, [255 - y; 3], "found");
    assert_eq!(monitor.status.missing_luts, []);
}

/// A grade of six nodes, as the compositor's measure has it: three serial correctors, two of
/// them in parallel into a layer mixer, and one after; its contrast keyed.
fn six_nodes(seed: f64) -> Grade {
    use columbina_model::grade::{Composite, LayerMixer};
    let node = |id: &str, set: &dyn Fn(&mut Corrector)| {
        let mut node = Node::corrector(id);
        if let NodeKind::Corrector(settings) = &mut node.kind {
            set(settings);
        }
        node
    };
    let contrast = Param::Keys(vec![
        Key {
            frame: 0,
            value: 1.0,
            interp: Interp::Linear,
        },
        Key {
            frame: 90,
            value: 1.3 + seed * 0.1,
            interp: Interp::Linear,
        },
    ]);
    let mut grade = Grade {
        nodes: vec![
            node("n1", &|c| {
                c.primaries.contrast = contrast.clone();
                c.primaries.lift.y = Param::Fixed(0.02);
                c.curves.y.points = vec![[0.0, 0.0], [0.3, 0.25], [0.7, 0.75], [1.0, 1.0]];
            }),
            node("n2", &|c| {
                c.primaries.saturation = Param::Fixed(60.0);
                c.hsl.hue_vs_sat = vec![[0.0, 0.2], [0.3, 0.0], [0.6, -0.2]];
                c.soft_clip.high_soft.r = Param::Fixed(20.0);
            }),
            node("n3", &|c| c.log.shadow.b = Param::Fixed(0.1)),
            node("n4", &|c| {
                c.primaries.temperature = Param::Fixed(-600.0);
                c.key.output.gain = Param::Fixed(0.7);
            }),
            Node {
                id: "lm".into(),
                label: String::new(),
                enabled: true,
                locked: false,
                pos: None,
                kind: NodeKind::LayerMixer(LayerMixer {
                    inputs: 2,
                    composite: Composite::Overlay,
                }),
            },
            node("n6", &|c| c.hsl.sat_vs_lum = vec![[0.0, 0.0], [1.0, 0.1]]),
        ],
        links: vec![
            Link::rgb("source", "n1"),
            Link::rgb("n1", "n2"),
            Link::rgb("n2", "n3"),
            Link::rgb("n2", "n4"),
            Link::rgb("n3", "lm"),
            Link {
                input: 1,
                ..Link::rgb("n4", "lm")
            },
            Link::rgb("lm", "n6"),
            Link::rgb("n6", "output"),
        ],
    };
    grade.sort_links();
    assert!(grade.is_complete());
    grade
}

#[test]
#[ignore = "a measure of time: run it on its own, in release, under the lock of the card"]
#[allow(clippy::too_many_lines, reason = "a scene, and how long it takes")]
fn how_long_graded_frames_of_4k_take() {
    use columbina_compose::{LayerColor, Transform};
    let Some(gpu) = columbina_compose::gpu_for_tests() else {
        return;
    };
    let Some(made) = crate::test_media::media(crate::test_media::Sample::Uhd) else {
        return;
    };
    let mut decoder = columbina_media::VideoDecoder::open(&made.path, Hardware::Auto).unwrap();
    let picture = Arc::new(
        columbina_media::Converter::new()
            .native(&decoder.decoded_at(0).unwrap())
            .unwrap(),
    );
    let (width, height) = (picture.width, picture.height);
    let dir = tempfile::tempdir().unwrap();
    let mut grading = Grading::new(LutPlaces::with_library(dir.path(), vec![]));
    let mut project = timeline_of(vec![], width, height);
    let timeline = &mut project.timelines[0];
    timeline.tracks = (0..3_u8)
        .map(|i| {
            let mut clip = colour(&format!("c{i}"), ORANGE, 0, 90);
            clip.grade = six_nodes(f64::from(i));
            Track::new(format!("V{i}"), TrackKind::Video).with_clips(vec![clip])
        })
        .collect();
    timeline.grade = corrector(|c| {
        c.primaries.contrast = Param::Fixed(1.1);
        c.primaries.saturation = Param::Fixed(55.0);
    });
    let project = Arc::new(project);
    let started = Instant::now();
    grading.update(&gpu, &project, 0);
    eprintln!(
        "on {}: the four grades made ready in {:?}",
        gpu.name(),
        started.elapsed()
    );
    let clips = ["c0", "c1", "c2"];
    let layer = |transform: Transform, opacity: f32| SceneLayer {
        color: LayerColor {
            coding: columbina_compose::YuvCoding::of_native(&picture),
            ..LayerColor::default()
        },
        source: LayerSource::Picture(Arc::clone(&picture), 0),
        transform,
        opacity,
    };
    let scene = || Scene {
        width,
        height,
        layers: vec![
            layer(Transform::NONE, 1.0),
            layer(
                Transform {
                    x: 0.2,
                    y: 0.1,
                    scale: 0.5,
                    rotation: 10.0,
                },
                0.8,
            ),
            layer(Transform::NONE, 0.5),
        ],
        ..Scene::default()
    };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let target = Target::preview(&gpu, width, height).unwrap();
    // The card is waited for by reading the picture back, once after all the frames: what is
    // measured is how many frames are drawn in a while, back to back, as a monitor that plays
    // draws them.
    let wait = || drop(target.read_rgba().unwrap());
    for graded in [false, true] {
        let mut first = scene();
        if graded {
            grading.grade(&mut first, &clips, 0);
        }
        compositor.render(&first, &target).unwrap();
        wait();
        let frames = 60_i64;
        let mut grading_time = Duration::ZERO;
        let started = Instant::now();
        for frame in 0..frames {
            let mut frame_scene = scene();
            if graded {
                let at = Instant::now();
                grading.grade(&mut frame_scene, &clips, frame);
                grading_time += at.elapsed();
            }
            compositor.render(&frame_scene, &target).unwrap();
        }
        wait();
        let each = u32::try_from(frames).unwrap();
        eprintln!(
            "{width}×{height}, three layers of P010{}: {:?} a frame, of which {:?} to take the \
             grades at the frame",
            if graded {
                ", each graded by six nodes (contrast keyed), and the timeline by one"
            } else {
                ""
            },
            started.elapsed() / each,
            grading_time / each,
        );
    }
    assert_eq!(grading.made(), 4, "made once, whatever the frame");
    assert_eq!(gpu.uncaught_errors(), (0, None));
}
