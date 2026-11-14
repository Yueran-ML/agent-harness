//! The operations of colour management, one by one: what each does, what it gives back, and
//! what it refuses; and the colour settings of a project as it is read.

use columbina_model::color_management::{
    media_input, AcesVersion, ColorSettings, LutPair, MediaInput, Pipeline, Preset, Science,
};
use columbina_model::{
    apply, apply_done, Basis, Change, ColorOverride, History, Levels, Op, OpError, Project,
    ReadError, TransactionError,
};
use serde_json::json;

fn set(settings: serde_json::Value) -> Op {
    Op::SetColorSettings { settings }
}

#[allow(clippy::option_option, reason = "left out and null are told apart")]
fn media_color(id: &str, input: Option<Option<&str>>, levels: Option<Levels>) -> Op {
    Op::SetMediaColor {
        id: id.into(),
        input: input.map(|input| input.map(Into::into)),
        levels,
        lut: None,
        inverse_drt: None,
    }
}

/// Apply an operation that must be applied, and check that its inverse takes it back exactly and
/// that what was done does it again.
fn applied(project: &mut Project, op: Op) -> (Op, Op) {
    let before = project.clone();
    let (done, inverse) = apply_done(project, op).unwrap();
    let after = project.clone();
    let mut undone = after.clone();
    let again = apply(&mut undone, inverse.clone()).unwrap();
    assert_eq!(undone, before, "{inverse:?}");
    assert_eq!(again, done);
    apply(&mut undone, done.clone()).unwrap();
    assert_eq!(undone, after, "{done:?}");
    (done, inverse)
}

/// Apply an operation that must be refused, and check that it changed nothing.
fn refused(project: &Project, op: Op) -> OpError {
    let mut copy = project.clone();
    let error = apply(&mut copy, op).unwrap_err();
    assert_eq!(&copy, project);
    error
}

fn managed() -> Project {
    let mut project = Project::sample();
    apply(
        &mut project,
        set(json!({"science": "yrgb_managed", "preset": "custom"})),
    )
    .unwrap();
    project
}

#[test]
fn colour_settings_are_patched_and_given_back_as_the_engine_writes_them() {
    let mut project = Project::sample();
    assert_eq!(project.color, ColorSettings::default());
    let (done, inverse) = applied(
        &mut project,
        set(json!({
            "science": "yrgb_managed",
            "preset": "custom",
            "timeline": "DaVinci Wide Gamut/DaVinci Intermediate",
            "working_luminance": "custom",
            "custom_luminance": 500,
            "input_drt": "saturation_preserving",
            "input_sat_rolloff": {"limit": 4000},
        })),
    );
    assert_eq!(project.color.science, Science::YrgbManaged);
    assert_eq!(project.color.custom_luminance, 500.0);
    assert_eq!(project.color.input_sat_rolloff.start, 100.0);
    assert_eq!(project.color.input_sat_rolloff.limit, 4000.0);
    assert_eq!(
        serde_json::to_value(&done).unwrap(),
        json!({
            "op": "set_color_settings",
            "settings": {
                "science": "yrgb_managed",
                "preset": "custom",
                "timeline": "DaVinci Wide Gamut/DaVinci Intermediate",
                "working_luminance": "custom",
                "custom_luminance": 500.0,
                "input_drt": "saturation_preserving",
                "input_sat_rolloff": {"limit": 4000.0},
            },
        })
    );
    assert_eq!(
        serde_json::to_value(&inverse).unwrap(),
        json!({
            "op": "set_color_settings",
            "settings": {
                "science": "yrgb",
                "preset": "sdr_rec709",
                "timeline": "Rec.709 Gamma 2.4",
                "working_luminance": "sdr_100",
                "custom_luminance": 100.0,
                "input_drt": "none",
                "input_sat_rolloff": {"limit": 1000.0},
            },
        })
    );
    let Pipeline::Rcm(rcm) = project.color.pipeline() else {
        panic!("{:?}", project.color)
    };
    assert_eq!(rcm.luminance, (500.0, 500.0));

    // An object within the patch is put into the object of its name; the 1D LUT stays.
    project.color.output_lut.one = Some("shaper.cube".into());
    let (_, inverse) = applied(
        &mut project,
        set(json!({"output_lut": {"3d": "Film/Kodak 2383.cube"}})),
    );
    assert_eq!(project.color.output_lut.one.as_deref(), Some("shaper.cube"));
    assert_eq!(
        project.color.output_lut.three.as_deref(),
        Some("Film/Kodak 2383.cube")
    );
    assert_eq!(inverse, set(json!({"output_lut": {"3d": null}})));
}

#[test]
fn every_science_has_its_settings() {
    let mut project = Project::sample();
    applied(
        &mut project,
        set(
            json!({"science": "acescct", "aces_version": "1.3", "odt": "P3-DCI (D65 sim.)",
                   "idt": "Sony S-Gamut3.Cine/S-Log3", "gamut_compress": true,
                   "node_luts_in": "ap0_linear"}),
        ),
    );
    let Pipeline::Aces(aces) = project.color.pipeline() else {
        panic!()
    };
    assert_eq!(aces.version, AcesVersion::V1_3);
    assert_eq!(aces.idt, "Sony S-Gamut3.Cine/S-Log3");
    assert!(aces.gamut_compress);
    // A version and an ODT of it are set together.
    applied(
        &mut project,
        set(json!({"aces_version": "2.0", "odt": "Rec.2020 ST2084 (1000 nits)"})),
    );
    // RCM with a preset, and automatic.
    applied(
        &mut project,
        set(json!({"science": "yrgb_managed", "preset": "hdr_p3_broadcast", "output": "bypass"})),
    );
    assert_eq!(project.color.preset, Preset::HdrP3Broadcast);
    applied(
        &mut project,
        set(json!({"automatic": true, "processing_mode": "hdr"})),
    );
    // The settings of every science stay as they were, and are in force again when it is.
    applied(&mut project, set(json!({"science": "acescc"})));
    let Pipeline::Aces(aces) = project.color.pipeline() else {
        panic!()
    };
    assert_eq!(aces.odt.name, "Rec.2020 ST2084 (1000 nits)");
    assert_eq!(aces.timeline, "ACEScc");
    // The settings of every science.
    applied(
        &mut project,
        set(
            json!({"lut_interpolation": "tetrahedral", "s_curve_contrast": false,
                   "viewer_lut": {"1d": "calibration/monitor.cube"}}),
        ),
    );
}

#[test]
fn a_space_not_of_the_catalogue_or_an_odt_not_of_the_version_is_refused() {
    let project = managed();
    for (patch, setting, name) in [
        (json!({"timeline": "Rec.709"}), "timeline", "Rec.709"),
        (json!({"timeline": "bypass"}), "timeline", "bypass"),
        (
            json!({"input": "Camera Native Linear"}),
            "input",
            "Camera Native Linear",
        ),
        (
            json!({"output": "rec.709 gamma 2.4"}),
            "output",
            "rec.709 gamma 2.4",
        ),
        (
            json!({"science": "acescct", "idt": "bypass"}),
            "idt",
            "bypass",
        ),
        (
            json!({"science": "acescct", "odt": "P3-DCI (D65 sim.)"}),
            "odt",
            "P3-DCI (D65 sim.)",
        ),
    ] {
        assert_eq!(
            refused(&project, set(patch.clone())),
            OpError::UnknownSpace {
                setting,
                name: name.into()
            },
            "{patch}"
        );
    }
    // A version whose ODTs do not have the one set is refused, unless the ODT is set with it.
    let mut aces = Project::sample();
    apply(
        &mut aces,
        set(json!({"science": "acescct", "odt": "Rec.2020 ST2084 (500 nits)"})),
    )
    .unwrap();
    assert!(matches!(
        refused(&aces, set(json!({"aces_version": "1.3"}))),
        OpError::UnknownSpace { setting: "odt", .. }
    ));
    applied(
        &mut aces,
        set(json!({"aces_version": "1.3", "odt": "Rec.2020 ST2084 (4000 nits)"})),
    );
}

#[test]
fn a_setting_that_applies_neither_before_nor_after_is_refused() {
    let unmanaged = Project::sample();
    let inapplicable = |setting: &str| OpError::Inapplicable {
        setting: setting.into(),
    };
    for (patch, setting) in [
        (json!({"odt": "sRGB"}), "odt"),
        (json!({"output": "sRGB"}), "output"),
        (
            json!({"science": "yrgb_managed", "timeline": "sRGB"}),
            "timeline",
        ),
        (
            json!({"science": "yrgb_managed", "processing_mode": "hdr"}),
            "processing_mode",
        ),
        (json!({"science": "acescct", "preset": "custom"}), "preset"),
    ] {
        assert_eq!(
            refused(&unmanaged, set(patch.clone())),
            inapplicable(setting),
            "{patch}"
        );
    }
    let custom = managed();
    assert_eq!(
        refused(&custom, set(json!({"custom_luminance": 500}))),
        inapplicable("custom_luminance")
    );
    assert_eq!(
        refused(&custom, set(json!({"output_sat_rolloff": {"start": 200}}))),
        inapplicable("output_sat_rolloff")
    );
    assert_eq!(
        refused(&custom, set(json!({"processing_mode": "hdr"}))),
        inapplicable("processing_mode")
    );
    // What applies before is taken back: leaving ACES with its ODT set is the inverse of
    // coming into it so.
    let mut project = Project::sample();
    let (_, inverse) = applied(
        &mut project,
        set(json!({"science": "acescct", "odt": "sRGB"})),
    );
    assert_eq!(inverse, set(json!({"science": "yrgb", "odt": "Rec.709"})));
}

#[test]
fn numbers_out_of_their_ranges_luts_not_in_the_folders_and_what_is_not_settings_are_refused() {
    let mut project = managed();
    apply(
        &mut project,
        set(json!({"working_luminance": "custom", "output_drt": "saturation_preserving"})),
    )
    .unwrap();
    let range = |what| OpError::OutOfRange { what };
    for (patch, what) in [
        (json!({"custom_luminance": 0}), "custom_luminance"),
        (json!({"custom_luminance": 10_000.5}), "custom_luminance"),
        (json!({"graphics_white": -1}), "graphics_white"),
        (
            json!({"output_sat_rolloff": {"start": 1000, "limit": 1000}}),
            "output_sat_rolloff",
        ),
        (json!({"limit_gamut": "camera-native"}), "limit_gamut"),
        (json!({"input_lut": {"1d": "../up.cube"}}), "input_lut"),
        (json!({"output_lut": {"3d": "C:/LUT/a.cube"}}), "output_lut"),
        (json!({"viewer_lut": {"3d": ""}}), "viewer_lut"),
    ] {
        assert_eq!(
            refused(&project, set(patch.clone())),
            range(what),
            "{patch}"
        );
    }
    for patch in [
        json!(null),
        json!([]),
        json!({"gamma": 2.4}),
        json!({"science": 3}),
        json!({"science": "aces"}),
        json!({"aces_version": "1.2"}),
        json!({"working_luminance": "hdr_10000"}),
        json!({"input_lut": null}),
        json!({"input_lut": {"2d": null}}),
        json!({"input_drt": "red_ipp2"}),
    ] {
        let error = refused(&project, set(patch.clone()));
        assert!(
            matches!(error, OpError::ColorSettings { .. }),
            "{patch}: {error}"
        );
    }
    applied(
        &mut project,
        set(json!({"limit_gamut": "p3-d65", "custom_luminance": 10_000})),
    );
}

#[test]
fn media_has_its_input_space_and_data_levels_set_by_hand() {
    let mut project = Project::sample();
    assert_eq!(project.media[0].color_override, ColorOverride::default());
    let (done, inverse) = applied(
        &mut project,
        media_color("m1", Some(Some("Apple Log")), Some(Levels::Full)),
    );
    assert_eq!(
        serde_json::to_value(&done).unwrap(),
        json!({"op": "set_media_color", "id": "m1", "input": "Apple Log", "levels": "full"})
    );
    assert_eq!(
        serde_json::to_value(&inverse).unwrap(),
        json!({"op": "set_media_color", "id": "m1", "input": null, "levels": "auto"})
    );
    assert_eq!(
        serde_json::to_value(&project.media[0]).unwrap()["color_override"],
        json!({"input": "Apple Log", "levels": "full", "lut": {"1d": null, "3d": null},
            "inverse_drt": true})
    );
    // Only what is given is set: null takes the space set by hand away.
    let (_, inverse) = applied(&mut project, media_color("m1", Some(None), None));
    assert_eq!(inverse, media_color("m1", Some(Some("Apple Log")), None));
    assert_eq!(project.media[0].color_override.levels, Levels::Full);
    applied(&mut project, media_color("m1", Some(Some("bypass")), None));
    let read: Op = serde_json::from_value(json!({"op": "set_media_color", "id": "m1"})).unwrap();
    assert_eq!(read, media_color("m1", None, None));

    assert_eq!(
        refused(&project, media_color("m1", Some(Some("Log")), None)),
        OpError::UnknownSpace {
            setting: "input",
            name: "Log".into()
        }
    );
    assert_eq!(
        refused(&project, media_color("m9", None, Some(Levels::Video))),
        OpError::Missing {
            what: "media",
            id: "m9".into()
        }
    );
}

#[test]
fn a_source_clip_has_its_luts_and_its_inverse_drt_set_by_hand() {
    let mut project = Project::sample();
    let set = |patch: serde_json::Value| -> Op {
        let mut op = json!({"op": "set_media_color", "id": "m1"});
        for (key, value) in patch.as_object().unwrap() {
            op[key] = value.clone();
        }
        serde_json::from_value(op).unwrap()
    };
    // None, and the inverse DRT on, at first and where a file leaves them out.
    assert_eq!(
        serde_json::to_value(&project.media[0]).unwrap()["color_override"],
        json!({"input": null, "levels": "auto", "lut": {"1d": null, "3d": null},
            "inverse_drt": true})
    );
    let read: ColorOverride = serde_json::from_value(json!({"levels": "video"})).unwrap();
    assert!(read.inverse_drt);
    assert_eq!(read.lut, LutPair::default());

    let (done, inverse) = applied(
        &mut project,
        set(json!({"lut": {"3d": "Film/Look.cube"}, "inverse_drt": false})),
    );
    assert_eq!(
        serde_json::to_value(&done).unwrap(),
        json!({"op": "set_media_color", "id": "m1",
            "lut": {"1d": null, "3d": "Film/Look.cube"}, "inverse_drt": false})
    );
    assert_eq!(
        serde_json::to_value(&inverse).unwrap(),
        json!({"op": "set_media_color", "id": "m1", "lut": {"1d": null, "3d": null},
            "inverse_drt": true})
    );
    assert_eq!(
        project.media[0].color_override,
        ColorOverride {
            lut: LutPair {
                one: None,
                three: Some("Film/Look.cube".into()),
            },
            inverse_drt: false,
            ..ColorOverride::default()
        }
    );
    // Only what is given is set: the levels leave the LUTs as they are.
    applied(&mut project, set(json!({"levels": "full"})));
    assert_eq!(
        project.media[0].color_override.lut.three.as_deref(),
        Some("Film/Look.cube")
    );
    // A name of a LUT is relative to the folders of LUTs, as the project's LUTs are.
    for bad in [
        "",
        "../up.cube",
        "C:/Looks/a.cube",
        "/a.cube",
        r"\\server\a.cube",
    ] {
        assert_eq!(
            refused(&project, set(json!({"lut": {"1d": bad}}))),
            OpError::OutOfRange { what: "media_lut" },
            "{bad}"
        );
    }
    // Media without a picture has none.
    let mut sound = project.media[0].clone();
    sound.id = "sound".into();
    sound.facts.video = None;
    sound.color_override = ColorOverride::default();
    apply(
        &mut project,
        Op::AddMedia {
            media: sound,
            index: None,
        },
    )
    .unwrap();
    let sound = |patch: serde_json::Value| -> Op {
        let mut op = set(patch);
        if let Op::SetMediaColor { id, .. } = &mut op {
            *id = "sound".into();
        }
        op
    };
    assert!(matches!(
        refused(&project, sound(json!({"inverse_drt": false}))),
        OpError::NoStream { .. }
    ));
    assert!(matches!(
        refused(&project, sound(json!({"lut": {"3d": "Film/Look.cube"}}))),
        OpError::NoStream { .. }
    ));
}

#[test]
fn camera_raw_and_media_without_a_picture_keep_what_they_are() {
    let mut project = Project::sample();
    let mut raw = project.media[0].clone();
    raw.id = "raw".into();
    raw.facts.video.as_mut().unwrap().codec = "braw".into();
    let mut sound = project.media[0].clone();
    sound.id = "sound".into();
    sound.facts.video = None;
    for media in [raw, sound] {
        apply(&mut project, Op::AddMedia { media, index: None }).unwrap();
    }
    assert_eq!(
        refused(&project, media_color("raw", Some(Some("sRGB")), None)),
        OpError::CameraRaw {
            media: "raw".into()
        }
    );
    applied(
        &mut project,
        media_color("raw", Some(Some("bypass")), Some(Levels::Video)),
    );
    // To the defaults, which it has, media without a picture may be set.
    applied(
        &mut project,
        media_color("sound", Some(None), Some(Levels::Auto)),
    );
    assert!(matches!(
        refused(&project, media_color("sound", None, Some(Levels::Full))),
        OpError::NoStream { .. }
    ));
    // Media taken in with a colour that it cannot have is refused too.
    let mut sound = project.media[0].clone();
    sound.id = "silent".into();
    sound.facts.video = None;
    sound.color_override.levels = Levels::Video;
    assert!(matches!(
        refused(
            &project,
            Op::AddMedia {
                media: sound,
                index: None
            }
        ),
        OpError::NoStream { .. }
    ));
}

#[test]
fn the_refusals_of_colour_are_written_as_the_protocol_names_them() {
    let project = Project::sample();
    let written = |error: OpError| serde_json::to_value(error).unwrap();
    assert_eq!(
        written(refused(
            &project,
            set(json!({"science": "yrgb_managed", "preset": "custom", "timeline": "x"}))
        )),
        json!({"reason": "unknown_space", "setting": "timeline", "name": "x",
               "message": "x is not a colour space or transform that the timeline may be"})
    );
    let error = written(refused(&project, set(json!({"odt": "sRGB"}))));
    assert_eq!(error["reason"], "inapplicable");
    assert_eq!(error["setting"], "odt");
    let error = written(refused(&project, set(json!({"gamma": 1}))));
    assert_eq!(error["reason"], "color_settings");
    assert!(error["why"].as_str().unwrap().contains("gamma"), "{error}");
    let error = written(refused(&project, set(json!({"graphics_white": 0}))));
    assert_eq!(error["reason"], "out_of_range");
    assert_eq!(error["what"], "graphics_white");
    let error = written(OpError::CameraRaw { media: "m1".into() });
    assert_eq!(error["reason"], "camera_raw");
    assert_eq!(error["media"], "m1");
}

#[test]
fn a_change_of_colour_to_what_it_is_makes_no_step() {
    let mut history = History::new(Project::sample());
    for op in [
        set(json!({"science": "yrgb", "lut_interpolation": "trilinear"})),
        media_color("m1", Some(None), Some(Levels::Auto)),
    ] {
        let error = history
            .change(vec![Change::Op(op)], "agent", |change, _| change)
            .unwrap_err();
        assert!(matches!(error, TransactionError::Empty), "{error:?}");
    }
    assert!(!history.can_undo());
    history
        .change(
            vec![Change::Op(set(json!({"lut_interpolation": "tetrahedral"})))],
            "agent",
            |change, _| change,
        )
        .unwrap();
    assert!(history.can_undo());
}

#[test]
fn a_project_is_read_with_its_colour_settings_checked_and_defaults_for_what_is_left_out() {
    let project = Project::sample();
    let mut value = serde_json::to_value(&project).unwrap();
    value.as_object_mut().unwrap().remove("color");
    value["media"][0]
        .as_object_mut()
        .unwrap()
        .remove("color_override");
    assert_eq!(Project::from_value(value.clone()).unwrap(), project);
    value["color"] = json!({"science": "acescct"});
    value["media"][0]["color_override"] = json!({"levels": "video"});
    let read = Project::from_value(value.clone()).unwrap();
    assert_eq!(read.color.science, Science::Acescct);
    assert_eq!(read.color.odt, "Rec.709");
    assert_eq!(read.media[0].color_override.levels, Levels::Video);
    assert_eq!(read.media[0].color_override.input, None);

    for (path, bad) in [
        ("color", json!({"timeline": "Rec.709"})),
        (
            "color",
            json!({"odt": "Rec.2020 ST2084 (500 nits)", "aces_version": "1.3"}),
        ),
        ("color", json!({"custom_luminance": 0})),
        ("color_override", json!({"input": "Log"})),
    ] {
        let mut wrong = value.clone();
        if path == "color" {
            wrong["color"] = bad;
        } else {
            wrong["media"][0]["color_override"] = bad;
        }
        let error = Project::from_value(wrong).unwrap_err();
        assert!(matches!(error, ReadError::Invalid(_)), "{error}");
    }
    let mut wrong = value;
    wrong["color"] = json!({"science": "legacy"});
    assert!(matches!(
        Project::from_value(wrong).unwrap_err(),
        ReadError::Json(_)
    ));
}

#[test]
fn media_is_in_the_space_of_its_hand_its_file_or_the_project_as_colour_management_says() {
    let mut project = Project::sample();
    let media = project.media[0].clone();
    // Not managed: no space at all.
    assert_eq!(media_input(&project.color, &media), None);
    apply(
        &mut project,
        set(json!({"science": "yrgb_managed", "preset": "custom", "input": "Apple Log"})),
    )
    .unwrap();
    // The file says nothing: the project's default.
    assert_eq!(
        media_input(&project.color, &media),
        Some((MediaInput::Space("Apple Log".into()), Basis::Default))
    );
    // The file says BT.709 and PQ.
    let mut hdr = media.clone();
    hdr.facts.video.as_mut().unwrap().color = Some(columbina_model::ColorFacts {
        primaries: 9,
        transfer: 16,
        matrix: 9,
        ..columbina_model::ColorFacts::default()
    });
    assert_eq!(
        media_input(&project.color, &hdr),
        Some((MediaInput::Space("Rec.2100 ST2084".into()), Basis::Metadata))
    );
    // A space set by hand is over the file's.
    hdr.color_override.input = Some("bypass".into());
    assert_eq!(
        media_input(&project.color, &hdr),
        Some((MediaInput::Bypass, Basis::Manual))
    );
    // Camera raw is its maker's, whatever is set.
    hdr.facts.video.as_mut().unwrap().codec = "r3d".into();
    assert_eq!(
        media_input(&project.color, &hdr),
        Some((MediaInput::CameraRaw, Basis::CameraRaw))
    );
    // A preset other than Custom takes Rec.709 Gamma 2.4, and ACES its IDT.
    apply(&mut project, set(json!({"preset": "sdr_rec709"}))).unwrap();
    assert_eq!(
        media_input(&project.color, &media),
        Some((
            MediaInput::Space("Rec.709 Gamma 2.4".into()),
            Basis::Default
        ))
    );
    apply(
        &mut project,
        set(json!({"science": "acescct", "idt": "DJI D-Gamut/D-Log"})),
    )
    .unwrap();
    assert_eq!(
        media_input(&project.color, &media),
        Some((
            MediaInput::Space("DJI D-Gamut/D-Log".into()),
            Basis::Default
        ))
    );
}
