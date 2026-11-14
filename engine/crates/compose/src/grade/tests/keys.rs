//! Qualifiers (pp. 3157–3182), the Key palette (pp. 3361–3363), keys along links and through
//! the Key Mixer, the Outside node, and what the Highlight modes show (pp. 3170–3171): on the
//! processor, and the card held to it.

use columbina_model::grade::{Morph, Qualifier, QualifierMode};

use super::*;

/// A qualifier as it is first, set by `set`.
fn qualifier(set: impl FnOnce(&mut Qualifier)) -> Qualifier {
    let mut qualifier = Qualifier::default();
    set(&mut qualifier);
    qualifier
}

/// A corrector `id` that adds 0.5 to blue where `qualifier` keys.
fn keyed(id: &str, qualifier: Qualifier) -> Node {
    corrector(id, |c| {
        c.primaries.offset.b = fixed(75.0);
        c.qualifier = Some(qualifier);
    })
}

/// What a grade shows of node `node`, as Highlight BW shows it: its key.
fn key_of(grade: &Grade, node: &str, rgb: [f32; 3]) -> f32 {
    evaluate_showing(
        grade,
        0.0,
        &options(),
        &NoLuts,
        &|_, rgb| rgb,
        rgb,
        &Look {
            node: node.into(),
            show: Show::Matte,
        },
    )[0]
}

/// The key of a grade of one keyed corrector n1 of `qualifier`, at `rgb`.
fn key_at(qualifier: &Qualifier, rgb: [f32; 3]) -> f32 {
    key_of(&serial(vec![keyed("n1", qualifier.clone())]), "n1", rgb)
}

/// A colour of hue `h` (in turns: red 0, green 1/3, blue 2/3), as saturated as it is bright.
fn hue_colour(h: f32) -> [f32; 3] {
    let turn = std::f32::consts::TAU * h;
    let third = std::f32::consts::TAU / 3.0;
    [
        0.5 + 0.5 * turn.cos(),
        0.5 + 0.5 * (turn - third).cos(),
        0.5 + 0.5 * (turn + third).cos(),
    ]
}

#[track_caller]
fn about(got: f32, wanted: f32, most: f32) {
    assert!((got - wanted).abs() <= most, "{got} is not {wanted}");
}

// ---------------------------------------------------------------------------------------------
// Neutral
// ---------------------------------------------------------------------------------------------

#[test]
fn a_qualifier_as_it_is_first_keys_every_pixel_and_changes_nothing_to_the_bit() {
    let plain = one(|c| c.primaries.saturation = fixed(80.0));
    let qualified = one(|c| {
        c.primaries.saturation = fixed(80.0);
        c.qualifier = Some(Qualifier::default());
    });
    for x in colours() {
        assert_eq!(eval(&qualified, x), eval(&plain, x), "{x:?}");
        assert_eq!(key_of(&qualified, "n1", x), 1.0, "{x:?}");
    }
    // The Key palette at its neutral settings, a qualifier or none, and the key of a node
    // before: as T9 made them.
    let mut linked = serial(vec![
        corrector("a", |c| c.key.output.gain = fixed(0.4)),
        corrector("b", |c| {
            c.primaries.gain.r = fixed(1.5);
            c.qualifier = Some(Qualifier::default());
        }),
    ]);
    let mut unqualified = linked.clone();
    if let NodeKind::Corrector(c) = &mut unqualified.nodes[1].kind {
        c.qualifier = None;
    }
    for grade in [&mut linked, &mut unqualified] {
        grade.links.push(link("a", "b", Port::Key, 0));
        grade.sort_links();
    }
    for x in colours() {
        assert_eq!(eval(&linked, x), eval(&unqualified, x), "{x:?}");
    }
}

#[test]
fn a_node_without_a_qualifier_has_the_code_it_had_and_one_with_it_another() {
    let body = |grade: &Grade| {
        grade_ops(grade, 0.0, &options(), &NoLuts)[0]
            .body
            .to_string()
    };
    let plain = body(&one(|_| {}));
    assert!(!plain.contains("grade_qualify"));
    let qualified = body(&one(|c| c.qualifier = Some(Qualifier::default())));
    assert!(qualified.contains("grade_qualify(input,"));
    // Its settings, its mode, its toggles, its invert and the Key palette are numbers; Clip,
    // which is done a pixel at a time, too.
    let set = one(|c| {
        c.qualifier = Some(qualifier(|q| {
            q.mode = QualifierMode::Rgb;
            q.invert = true;
            q.sat.on = false;
            q.hue.center = fixed(30.0);
            q.lum.low = Param::Keys(vec![
                Key {
                    frame: 0,
                    value: 10.0,
                    interp: Interp::Linear,
                },
                Key {
                    frame: 9,
                    value: 40.0,
                    interp: Interp::Linear,
                },
            ]);
            q.finesse.black_clip = fixed(20.0);
            q.finesse.white_clip = fixed(70.0);
            q.finesse.morph = Morph::Grow;
        }));
        c.key.qualifier.invert = true;
        c.key.qualifier.gain = fixed(0.5);
    });
    assert_eq!(body(&set), qualified);
    let ops = grade_ops(&set, 0.0, &options(), &NoLuts);
    assert!(ops[0].neighbourhoods.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Ranges and softness
// ---------------------------------------------------------------------------------------------

#[test]
fn a_luma_qualifier_keys_its_range_and_falls_off_softly_beyond_it() {
    let lum = qualifier(|q| {
        q.mode = QualifierMode::Lum;
        q.lum.low = fixed(30.0);
        q.lum.high = fixed(60.0);
        q.lum.low_soft = fixed(10.0);
        q.lum.high_soft = fixed(20.0);
        // LUM leaves hue and saturation out, whatever they are set to (p. 3175).
        q.hue.width = fixed(0.0);
        q.sat.low = fixed(100.0);
    });
    let mut last = 0.0;
    for step in 0..=100_u8 {
        let v = f32::from(step) / 100.0;
        let k = key_at(&lum, [v; 3]);
        match step {
            0..=20 | 80..=100 => assert_eq!(k, 0.0, "{v}"),
            30..=60 => assert_eq!(k, 1.0, "{v}"),
            21..=29 => assert!(k > last && k < 1.0, "{v}: {k}"),
            _ => assert!(k < last && k > 0.0, "{v}: {k}"),
        }
        last = k;
    }
    // Smoothly: half way through the softness, half the key.
    about(key_at(&lum, [0.25; 3]), 0.5, 1e-5);
    about(key_at(&lum, [0.7; 3]), 0.5, 1e-5);
    // Without softness, a hard edge at once.
    let hard = qualifier(|q| {
        q.mode = QualifierMode::Lum;
        q.lum.low = fixed(30.0);
    });
    assert_eq!(key_at(&hard, [0.299; 3]), 0.0);
    assert_eq!(key_at(&hard, [0.301; 3]), 1.0);
    // Beyond black and white, as black and white: a range to 100 keeps what is above white.
    assert_eq!(key_at(&hard, [1.6; 3]), 1.0);
    assert_eq!(key_at(&hard, [-0.2; 3]), 0.0);
}

#[test]
fn an_hsl_qualifier_keys_a_wedge_of_hues_that_goes_round() {
    // Red, ±0.05 of a turn, falling off over a tenth of a turn each side.
    let red = qualifier(|q| {
        q.hue.center = fixed(0.0);
        q.hue.width = fixed(10.0);
        q.hue.soft = fixed(10.0);
    });
    for h in [0.0, 0.03, 0.97, 0.999] {
        about(key_at(&red, hue_colour(h)), 1.0, 1e-5);
    }
    for h in [0.2, 0.5, 0.8] {
        assert_eq!(key_at(&red, hue_colour(h)), 0.0, "{h}");
    }
    // Half way into the softness on either side, half the key, alike.
    about(key_at(&red, hue_colour(0.1)), 0.5, 2e-3);
    about(key_at(&red, hue_colour(0.9)), 0.5, 2e-3);
    // Symmetry: lowered, the right (higher hues) steeper and the left softer.
    let steep_right = qualifier(|q| {
        q.hue.center = fixed(0.0);
        q.hue.width = fixed(10.0);
        q.hue.soft = fixed(10.0);
        q.hue.symmetry = fixed(0.0);
    });
    assert_eq!(key_at(&steep_right, hue_colour(0.06)), 0.0);
    about(key_at(&steep_right, hue_colour(0.85)), 0.5, 2e-3);
    // A wedge of green, its centre a third of a turn round.
    let green = qualifier(|q| {
        q.hue.center = fixed(100.0 / 3.0);
        q.hue.width = fixed(20.0);
    });
    assert_eq!(key_at(&green, hue_colour(1.0 / 3.0)), 1.0);
    assert_eq!(key_at(&green, hue_colour(0.0)), 0.0);
    // A quarter of a turn: between yellow and green.
    let quarter = qualifier(|q| {
        q.hue.center = fixed(25.0);
        q.hue.width = fixed(10.0);
    });
    assert_eq!(key_at(&quarter, hue_colour(0.25)), 1.0);
    assert_eq!(key_at(&quarter, hue_colour(0.0)), 0.0);
    // Hue turned off keys every hue; a width of 100 too.
    let off = qualifier(|q| {
        q.hue.on = false;
        q.hue.width = fixed(0.0);
    });
    let all = qualifier(|q| q.hue.width = fixed(100.0));
    for h in [0.0, 0.3, 0.71] {
        assert_eq!(key_at(&off, hue_colour(h)), 1.0);
        assert_eq!(key_at(&all, hue_colour(h)), 1.0);
    }
}

#[test]
fn saturation_luma_and_the_channels_of_rgb_limit_the_key_together() {
    // The saturated: greys are left out, as they are in p. 3173.
    let saturated = qualifier(|q| {
        q.sat.low = fixed(50.0);
        q.hue.on = false;
    });
    assert_eq!(key_at(&saturated, [0.4; 3]), 0.0);
    assert_eq!(key_at(&saturated, [0.8, 0.2, 0.2]), 1.0);
    // And dark: the key is the product of the two.
    let both = qualifier(|q| {
        q.hue.on = false;
        q.sat.low = fixed(50.0);
        q.lum.high = fixed(30.0);
        q.lum.high_soft = fixed(20.0);
    });
    assert_eq!(key_at(&saturated, [1.0, 0.45, 0.45]), 1.0);
    assert_eq!(key_at(&both, [1.0, 0.45, 0.45]), 0.0);
    let dark = [0.4, 0.1, 0.1];
    assert_eq!(key_at(&both, dark), 1.0);
    // RGB: red from 60 up, green and blue below 30, blue turned off.
    let rgb = qualifier(|q| {
        q.mode = QualifierMode::Rgb;
        q.red.low = fixed(60.0);
        q.green.high = fixed(30.0);
        q.blue.high = fixed(0.0);
        q.blue.on = false;
    });
    assert_eq!(key_at(&rgb, [0.7, 0.2, 0.9]), 1.0);
    assert_eq!(key_at(&rgb, [0.5, 0.2, 0.9]), 0.0);
    assert_eq!(key_at(&rgb, [0.7, 0.4, 0.0]), 0.0);
    // The settings of the other modes are left out in RGB.
    let rgb_hue = qualifier(|q| {
        q.mode = QualifierMode::Rgb;
        q.hue.width = fixed(0.0);
        q.lum.low = fixed(100.0);
    });
    assert_eq!(key_at(&rgb_hue, [0.2, 0.5, 0.7]), 1.0);
}

#[test]
fn the_key_of_a_qualifier_goes_through_clip_invert_and_the_key_palette() {
    let ramp = |q: &Qualifier, v: f32| key_at(q, [v; 3]);
    // A luma key that rises from 0 at 0.2 to 1 at 0.6.
    let soft = |set: &dyn Fn(&mut Qualifier)| {
        qualifier(|q| {
            q.mode = QualifierMode::Lum;
            q.lum.low = fixed(60.0);
            q.lum.low_soft = fixed(40.0);
            set(q);
        })
    };
    let plain = soft(&|_| {});
    let k = ramp(&plain, 0.35);
    assert!(k > 0.2 && k < 0.8, "{k}");
    // Black Clip and White Clip stretch the levels between them to 0 to 1.
    let clipped = soft(&|q| {
        q.finesse.black_clip = fixed(25.0);
        q.finesse.white_clip = fixed(75.0);
    });
    about(
        ramp(&clipped, 0.35),
        ((k - 0.25) / 0.5).clamp(0.0, 1.0),
        1e-6,
    );
    assert_eq!(ramp(&clipped, 0.21), 0.0);
    assert_eq!(ramp(&clipped, 0.59), 1.0);
    // The palette's Invert, after them.
    let inverted = soft(&|q| q.invert = true);
    about(ramp(&inverted, 0.35), 1.0 - k, 1e-6);
    // The Key palette's Qualifier controls: invert, offset (white held), gain.
    let palette = serial(vec![corrector("n1", |c| {
        c.qualifier = Some(plain.clone());
        c.key.qualifier.offset = fixed(0.5);
        c.key.qualifier.gain = fixed(0.8);
    })]);
    about(
        key_of(&palette, "n1", [0.35; 3]),
        (k + 0.5 * (1.0 - k)) * 0.8,
        1e-6,
    );
    // Key Output Invert inverts all of the node's key (p. 3182).
    let output = serial(vec![corrector("n1", |c| {
        c.qualifier = Some(plain.clone());
        c.key.output.invert = true;
    })]);
    about(key_of(&output, "n1", [0.35; 3]), 1.0 - k, 1e-6);
    // The node works by its key: blue raised by 0.5 times the key.
    let grade = serial(vec![keyed("n1", plain.clone())]);
    about(eval(&grade, [0.35; 3])[2], 0.35 + 0.5 * k, 1e-6);
    about(eval(&grade, [0.1; 3])[2], 0.1, 0.0);
}

#[test]
fn a_channel_of_rgb_alone_keys_its_range() {
    for channel in 0..3 {
        let alone = qualifier(|q| {
            q.mode = QualifierMode::Rgb;
            for (i, range) in [&mut q.red, &mut q.green, &mut q.blue]
                .into_iter()
                .enumerate()
            {
                range.low = fixed(50.0);
                range.on = i == channel;
            }
        });
        let mut high = [0.1; 3];
        high[channel] = 0.8;
        let mut low = [0.9; 3];
        low[channel] = 0.2;
        assert_eq!(key_at(&alone, high), 1.0, "{channel}");
        assert_eq!(key_at(&alone, low), 0.0, "{channel}");
    }
}

#[test]
fn black_clip_or_white_clip_alone_stretches_the_key_and_both_at_one_level_cut_it() {
    // An RGB key of red, rising from 0 at red 0 to 1 at 0.5: 0.5 at red 0.25.
    let red = |set: &dyn Fn(&mut Qualifier)| {
        qualifier(|q| {
            q.mode = QualifierMode::Rgb;
            q.red.low = fixed(50.0);
            q.red.low_soft = fixed(50.0);
            q.green.on = false;
            q.blue.on = false;
            set(q);
        })
    };
    let at_red = |q: &Qualifier, r: f32| key_at(q, [r, 0.0, 0.0]);
    assert_eq!(at_red(&red(&|_| {}), 0.25), 0.5);
    let black = red(&|q| q.finesse.black_clip = fixed(40.0));
    about(at_red(&black, 0.25), (0.5 - 0.4) / 0.6, 1e-6);
    let white = red(&|q| q.finesse.white_clip = fixed(60.0));
    about(at_red(&white, 0.25), 0.5 / 0.6, 1e-6);
    let cut = red(&|q| {
        q.finesse.black_clip = fixed(50.0);
        q.finesse.white_clip = fixed(50.0);
    });
    assert_eq!(at_red(&cut, 0.25), 0.0, "at the level: none");
    assert_eq!(at_red(&cut, 0.375), 1.0);
    assert_eq!(at_red(&cut, 0.125), 0.0);
}

#[test]
fn the_key_of_matte_finesse_alone_is_that_of_a_node_that_also_changes_the_colour() {
    let pixels = picture(|x, y| x < 48 && (x, y) != (20, 20));
    for set in [
        (|q: &mut Qualifier| q.finesse.pre_filter = fixed(40.0)) as fn(&mut Qualifier),
        |q| q.finesse.clean_white = fixed(40.0),
        |q| q.finesse.blur_radius = fixed(30.0),
        |q| q.finesse.morph_radius = fixed(3.0),
    ] {
        let mut q = qualifier(|q| {
            q.mode = QualifierMode::Lum;
            q.lum.low = fixed(50.0);
        });
        set(&mut q);
        let bare = serial(vec![corrector("n1", |c| c.qualifier = Some(q.clone()))]);
        let changing = serial(vec![keyed("n1", q.clone())]);
        assert_eq!(key_picture(&bare, &pixels), key_picture(&changing, &pixels));
    }
    // The blur of a key coming in, alike.
    let coming_in = |offset: f64| {
        let mut grade = serial(vec![
            corrector("n1", |c| {
                c.qualifier = Some(qualifier(|q| {
                    q.mode = QualifierMode::Lum;
                    q.lum.low = fixed(50.0);
                }));
            }),
            corrector("n2", |c| {
                c.primaries.offset.b = fixed(offset);
                c.key.input_blur.radius = fixed(30.0);
            }),
        ]);
        grade.links.push(link("n1", "n2", Port::Key, 0));
        grade.sort_links();
        evaluate_picture_showing(
            &grade,
            0.0,
            &options(),
            &NoLuts,
            &|_, rgb| rgb,
            (SIZE, SIZE),
            &pixels,
            1.0,
            Some(&Look {
                node: "n2".into(),
                show: Show::Matte,
            }),
        )
    };
    assert_eq!(coming_in(25.0), coming_in(75.0));
}

#[test]
fn keyframes_of_a_qualifier_are_taken_at_the_frame() {
    let grade = serial(vec![keyed(
        "n1",
        qualifier(|q| {
            q.mode = QualifierMode::Lum;
            q.lum.low = Param::Keys(vec![
                Key {
                    frame: 0,
                    value: 0.0,
                    interp: Interp::Linear,
                },
                Key {
                    frame: 10,
                    value: 80.0,
                    interp: Interp::Linear,
                },
            ]);
        }),
    )]);
    let at = |frame: f64| evaluate(&grade, frame, &options(), [0.5; 3])[2];
    about(at(0.0), 1.0, 1e-6);
    about(at(5.0), 1.0, 1e-6);
    about(at(10.0), 0.5, 1e-6);
}

// ---------------------------------------------------------------------------------------------
// Keys between nodes
// ---------------------------------------------------------------------------------------------

/// A luma key of n1 from 0.3 up, which n2 takes at its key input as `set` sets n2.
fn chained(set: impl FnOnce(&mut Corrector)) -> Grade {
    let mut grade = serial(vec![
        corrector("n1", |c| {
            c.qualifier = Some(qualifier(|q| {
                q.mode = QualifierMode::Lum;
                q.lum.low = fixed(30.0);
            }));
        }),
        corrector("n2", |c| {
            c.primaries.offset.b = fixed(75.0);
            set(c);
        }),
    ]);
    grade.links.push(link("n1", "n2", Port::Key, 0));
    grade.sort_links();
    grade
}

#[test]
fn a_key_goes_along_its_link_and_meets_or_adds_to_the_qualifier_of_the_node_it_comes_into() {
    // n2 keyed by n1 alone: blue raised where luma is at least 0.3.
    let alone = chained(|_| {});
    assert_eq!(eval(&alone, [0.2; 3])[2], 0.2);
    assert_eq!(eval(&alone, [0.5; 3])[2], 1.0);
    // n2's own qualifier keys below 0.6; in Matte mode (the default) the two meet, in Mask
    // mode they add (p. 3362).
    let below = qualifier(|q| {
        q.mode = QualifierMode::Lum;
        q.lum.high = fixed(60.0);
    });
    let matte = chained(|c| c.qualifier = Some(below.clone()));
    let mask = chained(|c| {
        c.qualifier = Some(below.clone());
        c.key.input.matte = false;
    });
    for (v, meet, add) in [(0.2, 0.0, 1.0), (0.5, 1.0, 1.0), (0.8, 0.0, 1.0)] {
        assert_eq!(key_of(&matte, "n2", [v; 3]), meet, "{v}");
        assert_eq!(key_of(&mask, "n2", [v; 3]), add, "{v}");
    }
    // The key coming in through its own settings first: inverted, it is the outside of n1.
    let inverted = chained(|c| c.key.input.invert = true);
    assert_eq!(key_of(&inverted, "n2", [0.2; 3]), 1.0);
    assert_eq!(key_of(&inverted, "n2", [0.5; 3]), 0.0);
}

#[test]
fn an_outside_node_works_where_the_qualifier_before_it_does_not() {
    // As add_outside makes it: n2 after n1, on its picture and its key, inverted (p. 3342).
    let grade = chained(|c| {
        c.key.input.invert = true;
        c.primaries.offset.b = fixed(25.0);
        c.primaries.offset.r = fixed(45.0);
    });
    let mut n1 = grade.clone();
    if let NodeKind::Corrector(c) = &mut n1.nodes[0].kind {
        c.primaries.offset.g = fixed(45.0);
    }
    // n1 greens what it keys, n2 reds the rest: each pixel is changed by one of them.
    for v in [0.1_f32, 0.2, 0.31, 0.6, 0.9] {
        let out = eval(&n1, [v; 3]);
        if v >= 0.3 {
            near(out, [v, v + 0.2, v], 1e-6);
        } else {
            near(out, [v + 0.2, v, v], 1e-6);
        }
    }
}

#[test]
fn keys_of_qualifiers_go_through_a_key_mixer() {
    // n1 keys the dark (below 0.4) and n2 the light (above 0.6); the mixer gives n3 their sum
    // (Mask, the default: everything but the middle) or, its second input in Matte mode and
    // inverted, the first less the second.
    let build = |matte: bool, invert: bool| {
        let range = |low: f64, high: f64| {
            qualifier(|q| {
                q.mode = QualifierMode::Lum;
                q.lum.low = fixed(low);
                q.lum.high = fixed(high);
            })
        };
        let mut grade = serial(vec![
            corrector("n1", |c| c.qualifier = Some(range(0.0, 40.0))),
            corrector("n2", |c| c.qualifier = Some(range(60.0, 100.0))),
            corrector("n3", |c| c.primaries.offset.b = fixed(75.0)),
        ]);
        let mut mixer = KeyMixer::default();
        mixer.inputs[1].matte = matte;
        mixer.inputs[1].invert = invert;
        grade.nodes.push(Node {
            id: "km".into(),
            label: String::new(),
            enabled: true,
            locked: false,
            pos: None,
            kind: NodeKind::KeyMixer(mixer),
        });
        grade.links.extend([
            link("n1", "km", Port::Key, 0),
            link("n2", "km", Port::Key, 1),
            link("km", "n3", Port::Key, 0),
        ]);
        grade.sort_links();
        grade
    };
    let added = build(false, false);
    for (v, k) in [(0.2, 1.0), (0.5, 0.0), (0.8, 1.0)] {
        assert_eq!(key_of(&added, "km", [v; 3]), k, "{v}");
        assert_eq!(eval(&added, [v; 3])[2], v + 0.5 * k, "{v}");
    }
    let less = build(true, true);
    for (v, k) in [(0.2, 1.0), (0.5, 0.0), (0.8, 0.0)] {
        assert_eq!(key_of(&less, "km", [v; 3]), k, "{v}");
    }
}

// ---------------------------------------------------------------------------------------------
// Matte Finesse, on pictures
// ---------------------------------------------------------------------------------------------

const SIZE: u32 = 96;

/// A picture of 96 × 96, black (0.05) but where `white` says it is grey (0.8).
fn picture(white: impl Fn(u32, u32) -> bool) -> Vec<[f32; 3]> {
    (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .map(|(x, y)| if white(x, y) { [0.8; 3] } else { [0.05; 3] })
        .collect()
}

/// A luma qualifier that keys the grey of [`picture`], refined by `set`.
fn finesse(set: impl FnOnce(&mut Qualifier)) -> Grade {
    serial(vec![keyed(
        "n1",
        qualifier(|q| {
            q.mode = QualifierMode::Lum;
            q.lum.low = fixed(50.0);
            set(q);
        }),
    )])
}

/// The key of n1 of `grade` over `pixels`, one of them to a pixel of the reference frame.
fn key_picture(grade: &Grade, pixels: &[[f32; 3]]) -> Vec<f32> {
    evaluate_picture_showing(
        grade,
        0.0,
        &options(),
        &NoLuts,
        &|_, rgb| rgb,
        (SIZE, SIZE),
        pixels,
        1.0,
        Some(&Look {
            node: "n1".into(),
            show: Show::Matte,
        }),
    )
    .into_iter()
    .map(|rgb| rgb[0])
    .collect()
}

fn at(key: &[f32], x: u32, y: u32) -> f32 {
    key[(y * SIZE + x) as usize]
}

/// The left half grey: an edge at x = 48.
fn half() -> Vec<[f32; 3]> {
    picture(|x, _| x < 48)
}

#[test]
fn clean_black_takes_specks_out_and_clean_white_fills_holes() {
    // A speck of 2 × 2 on the black, and a hole of 2 × 2 in a square of grey.
    let pixels = picture(|x, y| {
        let speck = (20..22).contains(&x) && (20..22).contains(&y);
        let square = (50..90).contains(&x) && (50..90).contains(&y);
        let hole = (69..71).contains(&x) && (69..71).contains(&y);
        speck || (square && !hole)
    });
    let raw = key_picture(&finesse(|_| {}), &pixels);
    assert_eq!(at(&raw, 20, 20), 1.0);
    assert_eq!(at(&raw, 69, 69), 0.0);
    let black = key_picture(&finesse(|q| q.finesse.clean_black = fixed(40.0)), &pixels);
    assert!(at(&black, 20, 20) < 0.2, "{}", at(&black, 20, 20));
    // What is solid is left as it is.
    about(at(&black, 60, 60), 1.0, 1e-3);
    assert_eq!(at(&black, 69, 69), 0.0);
    let white = key_picture(&finesse(|q| q.finesse.clean_white = fixed(40.0)), &pixels);
    assert!(at(&white, 69, 69) > 0.8, "{}", at(&white, 69, 69));
    about(at(&white, 30, 70), 0.0, 1e-3);
    assert_eq!(at(&white, 20, 20), 1.0);
}

#[test]
fn blur_radius_softens_the_edge_and_in_out_ratio_moves_it_in_or_out() {
    let pixels = half();
    let hard = key_picture(&finesse(|_| {}), &pixels);
    assert_eq!((at(&hard, 47, 40), at(&hard, 48, 40)), (1.0, 0.0));
    // A Gaussian of 3 pixels: half the key at the edge, falling over some 6 each side.
    let blurred = key_picture(&finesse(|q| q.finesse.blur_radius = fixed(30.0)), &pixels);
    about(
        f32::midpoint(at(&blurred, 47, 40), at(&blurred, 48, 40)),
        0.5,
        2e-3,
    );
    assert!(at(&blurred, 44, 40) > 0.75 && at(&blurred, 44, 40) < 0.95);
    assert!(at(&blurred, 51, 40) > 0.05 && at(&blurred, 51, 40) < 0.25);
    about(at(&blurred, 30, 40), 1.0, 1e-3);
    about(at(&blurred, 70, 40), 0.0, 1e-3);
    // Out: the inside is all keyed, and the blur is outside the edge.
    let out = key_picture(
        &finesse(|q| {
            q.finesse.blur_radius = fixed(30.0);
            q.finesse.in_out_ratio = fixed(100.0);
        }),
        &pixels,
    );
    about(at(&out, 47, 40), 1.0, 1e-3);
    assert!(at(&out, 51, 40) > at(&blurred, 51, 40));
    // In: the outside is none, and the blur is inside it.
    let inside = key_picture(
        &finesse(|q| {
            q.finesse.blur_radius = fixed(30.0);
            q.finesse.in_out_ratio = fixed(-100.0);
        }),
        &pixels,
    );
    about(at(&inside, 48, 40), 0.0, 1e-3);
    assert!(at(&inside, 44, 40) < at(&blurred, 44, 40));
    // Far from the edge, both are whole and none.
    for keyed in [&out, &inside] {
        about(at(keyed, 20, 40), 1.0, 1e-3);
        about(at(keyed, 80, 40), 0.0, 1e-3);
    }
}

#[test]
fn in_out_ratio_without_blur_fills_small_holes_or_takes_small_specks() {
    // A hole of one pixel in the grey half, a speck of one in the black.
    let pixels = picture(|x, y| (x < 48 && (x, y) != (20, 20)) || (x, y) == (70, 70));
    let filled = key_picture(&finesse(|q| q.finesse.in_out_ratio = fixed(100.0)), &pixels);
    assert!(at(&filled, 20, 20) > 0.9, "{}", at(&filled, 20, 20));
    let cleared = key_picture(
        &finesse(|q| q.finesse.in_out_ratio = fixed(-100.0)),
        &pixels,
    );
    assert!(at(&cleared, 70, 70) < 0.1, "{}", at(&cleared, 70, 70));
}

#[test]
fn morph_shrinks_grows_opens_and_closes_the_key() {
    let pixels = half();
    let morph = |operation: Morph| {
        key_picture(
            &finesse(|q| {
                q.finesse.morph = operation;
                q.finesse.morph_radius = fixed(4.0);
            }),
            &pixels,
        )
    };
    // Grown by 4 pixels: keyed out to x = 51, and none from 53.
    let grown = morph(Morph::Grow);
    assert_eq!(at(&grown, 51, 40), 1.0);
    assert_eq!(at(&grown, 53, 40), 0.0);
    assert_eq!(at(&grown, 10, 40), 1.0);
    // Shrunk: none from x = 44 (47 less 4 and a little), all to 42.
    let shrunk = morph(Morph::Shrink);
    assert_eq!(at(&shrunk, 42, 40), 1.0);
    assert_eq!(at(&shrunk, 45, 40), 0.0);
    assert_eq!(at(&shrunk, 80, 40), 0.0);
    // Opening takes a speck smaller than the radius away and keeps the edge; closing fills a
    // hole.
    let specked = picture(|x, y| x < 48 || ((70..72).contains(&x) && (70..72).contains(&y)));
    let opened = key_picture(
        &finesse(|q| {
            q.finesse.morph = Morph::Opening;
            q.finesse.morph_radius = fixed(4.0);
        }),
        &specked,
    );
    assert_eq!(at(&opened, 70, 70), 0.0);
    assert_eq!((at(&opened, 46, 40), at(&opened, 49, 40)), (1.0, 0.0));
    let holed = picture(|x, y| x < 48 && !((20..22).contains(&x) && (20..22).contains(&y)));
    let closed = key_picture(
        &finesse(|q| {
            q.finesse.morph = Morph::Closing;
            q.finesse.morph_radius = fixed(4.0);
        }),
        &holed,
    );
    assert_eq!(at(&closed, 20, 20), 1.0);
    assert_eq!((at(&closed, 46, 40), at(&closed, 49, 40)), (1.0, 0.0));
}

#[test]
fn pre_filter_blurs_the_picture_before_it_is_keyed() {
    // A checker of grey and black of a pixel each: keyed as it is, it is a checker; blurred
    // first, a grey of about 0.43 that a key from 0.4 keeps throughout.
    let pixels = picture(|x, y| (x + y) % 2 == 0);
    let grade = |pre: f64| {
        serial(vec![keyed(
            "n1",
            qualifier(|q| {
                q.mode = QualifierMode::Lum;
                q.lum.low = fixed(40.0);
                q.finesse.pre_filter = fixed(pre);
            }),
        )])
    };
    let raw = key_picture(&grade(0.0), &pixels);
    assert_eq!((at(&raw, 40, 40), at(&raw, 41, 40)), (1.0, 0.0));
    let filtered = key_picture(&grade(40.0), &pixels);
    for (x, y) in [(40, 40), (41, 40), (60, 13)] {
        assert_eq!(at(&filtered, x, y), 1.0, "({x}, {y})");
    }
}

#[test]
fn the_key_coming_in_is_blurred_as_its_settings_say() {
    let pixels = half();
    let grade = |radius: f64, ratio: f64| {
        let mut grade = serial(vec![
            corrector("n1", |c| {
                c.qualifier = Some(qualifier(|q| {
                    q.mode = QualifierMode::Lum;
                    q.lum.low = fixed(50.0);
                }));
            }),
            corrector("n2", |c| {
                c.primaries.offset.b = fixed(75.0);
                c.key.input_blur.radius = fixed(radius);
                c.key.input_blur.ratio = fixed(ratio);
            }),
        ]);
        grade.links.push(link("n1", "n2", Port::Key, 0));
        grade.sort_links();
        grade
    };
    let key = |grade: &Grade| {
        evaluate_picture_showing(
            grade,
            0.0,
            &options(),
            &NoLuts,
            &|_, rgb| rgb,
            (SIZE, SIZE),
            &pixels,
            1.0,
            Some(&Look {
                node: "n2".into(),
                show: Show::Matte,
            }),
        )
    };
    let sharp = key(&grade(0.0, 0.5));
    assert_eq!((sharp[40 * 96 + 47][0], sharp[40 * 96 + 48][0]), (1.0, 0.0));
    // Without a blur, the key coming in is read at its pixel: no neighbourhood.
    assert!(grade_ops(&grade(0.0, 0.5), 0.0, &options(), &NoLuts)
        .iter()
        .all(|op| op.neighbourhoods.is_empty()));
    // With one, the node still does its own change where the key is full.
    let picture = evaluate_picture(
        &grade(30.0, 0.5),
        0.0,
        &options(),
        &NoLuts,
        &|_, rgb| rgb,
        (SIZE, SIZE),
        &pixels,
        1.0,
    );
    about(picture[40 * 96 + 2][2], pixels[40 * 96 + 2][2] + 0.5, 1e-3);
    let soft = key(&grade(30.0, 0.5));
    let middle = f32::midpoint(soft[40 * 96 + 47][0], soft[40 * 96 + 48][0]);
    about(middle, 0.5, 2e-3);
    // Blurred, the edge is a slope: neither side of it is full or none.
    assert!(soft[40 * 96 + 47][0] < 0.9 && soft[40 * 96 + 48][0] > 0.1);
    // All of the blur down: the edge, which goes down, stays sharp.
    let down = key(&grade(30.0, 0.0));
    assert!(down[40 * 96 + 47][0] > 0.99 && down[40 * 96 + 48][0] < 0.01);
}

#[test]
fn matte_finesse_keyframed_from_nothing_changes_nothing_where_it_is_nothing() {
    // A soft luma key over a ramp of values that half floats do not keep.
    #[allow(clippy::cast_precision_loss, reason = "places of pixels")]
    let pixels: Vec<[f32; 3]> = (0..SIZE * SIZE)
        .map(|i| [(i % SIZE) as f32 / 97.0 + 0.0001; 3])
        .collect();
    let rising = || {
        Param::Keys(vec![
            Key {
                frame: 0,
                value: 0.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 10,
                value: 50.0,
                interp: Interp::Linear,
            },
        ])
    };
    let soft = |set: &dyn Fn(&mut Qualifier)| {
        finesse(|q| {
            q.lum.low = fixed(30.0);
            q.lum.low_soft = fixed(40.0);
            set(q);
        })
    };
    let plain = key_picture(&soft(&|_| {}), &pixels);
    for (what, set) in [
        (
            "pre filter",
            &(|q: &mut Qualifier| q.finesse.pre_filter = rising()) as &dyn Fn(&mut Qualifier),
        ),
        ("blur radius", &|q: &mut Qualifier| {
            q.finesse.blur_radius = rising();
        }),
        ("morph radius", &|q: &mut Qualifier| {
            q.finesse.morph_radius = rising();
        }),
    ] {
        assert_eq!(key_picture(&soft(set), &pixels), plain, "{what}");
    }
    // The blur of a key coming in alike.
    let coming_in = |radius: Param| {
        let mut grade = serial(vec![
            soft(&|_| {}).nodes[0].clone(),
            corrector("n2", |c| {
                c.primaries.offset.b = fixed(75.0);
                c.key.input_blur.radius = radius;
            }),
        ]);
        grade.links.push(link("n1", "n2", Port::Key, 0));
        grade.sort_links();
        evaluate_picture(
            &grade,
            0.0,
            &options(),
            &NoLuts,
            &|_, rgb| rgb,
            (SIZE, SIZE),
            &pixels,
            1.0,
        )
    };
    assert_eq!(coming_in(rising()), coming_in(fixed(0.0)));
}

#[test]
fn matte_finesse_reads_a_neighbourhood_only_where_its_settings_ever_do_something() {
    let count = |grade: &Grade| {
        grade_ops(grade, 0.0, &options(), &NoLuts)[0]
            .neighbourhoods
            .len()
    };
    assert_eq!(count(&finesse(|_| {})), 0);
    assert_eq!(count(&finesse(|q| q.finesse.pre_filter = fixed(5.0))), 1);
    assert_eq!(
        count(&finesse(|q| {
            q.finesse.clean_black = fixed(5.0);
            q.finesse.clean_white = fixed(5.0);
        })),
        1
    );
    assert_eq!(count(&finesse(|q| q.finesse.in_out_ratio = fixed(5.0))), 1);
    assert_eq!(
        count(&finesse(|q| {
            q.finesse.morph = Morph::Grow;
            q.finesse.morph_radius = fixed(2.0);
        })),
        1
    );
    assert_eq!(
        count(&finesse(|q| {
            q.finesse.morph = Morph::Opening;
            q.finesse.morph_radius = fixed(2.0);
        })),
        2
    );
    // Keyframed through 0, it reads one at every frame, and the code is the same.
    let keyed_blur = |at_zero: f64| {
        finesse(|q| {
            q.finesse.blur_radius = Param::Keys(vec![
                Key {
                    frame: 0,
                    value: at_zero,
                    interp: Interp::Linear,
                },
                Key {
                    frame: 10,
                    value: 60.0,
                    interp: Interp::Linear,
                },
            ]);
        })
    };
    let grade = keyed_blur(0.0);
    let first = grade_ops(&grade, 0.0, &options(), &NoLuts);
    let later = grade_ops(&grade, 5.0, &options(), &NoLuts);
    assert_eq!(first[0].body, later[0].body);
    assert_eq!(
        first[0].neighbourhoods[0].body,
        later[0].neighbourhoods[0].body
    );
    // The filter is of the frame.
    let sigma = |op: &ColorOp| match op.neighbourhoods[0].filter {
        crate::color::Filter::Gaussian { sigma } => sigma[0],
        crate::color::Filter::Box { .. } => unreachable!(),
    };
    about(sigma(&first[0]), 0.0, 0.0);
    about(sigma(&later[0]), 3.0, 1e-6);
    // And the blur is not done where the key is 0 there: the key is the clipped key, exactly.
    let pixels = half();
    assert_eq!(
        key_picture(&grade, &pixels),
        key_picture(&finesse(|_| {}), &pixels)
    );
    // The key coming in reads one only where it is linked.
    let blurred = one(|c| c.key.input_blur.radius = fixed(30.0));
    assert_eq!(count(&blurred), 0);
}

#[test]
fn a_neighbourhood_reads_only_what_comes_before_it() {
    // n1 keyed with all of Matte Finesse; n2 takes its key, blurred, and has a qualifier with
    // a Pre-Filter of its own.
    let mut grade = serial(vec![
        keyed(
            "n1",
            qualifier(|q| {
                q.mode = QualifierMode::Lum;
                q.lum.low = fixed(50.0);
                q.finesse.pre_filter = fixed(10.0);
                q.finesse.clean_black = fixed(20.0);
                q.finesse.blur_radius = fixed(20.0);
                q.finesse.morph = Morph::Closing;
                q.finesse.morph_radius = fixed(3.0);
            }),
        ),
        corrector("n2", |c| {
            c.key.input_blur.radius = fixed(30.0);
            c.qualifier = Some(qualifier(|q| q.finesse.pre_filter = fixed(5.0)));
        }),
    ]);
    grade.links.push(link("n1", "n2", Port::Key, 0));
    grade.sort_links();
    let ops = grade_ops(&grade, 0.0, &options(), &NoLuts);
    let hoods = &ops[0].neighbourhoods;
    assert_eq!(hoods.len(), 7);
    for (number, hood) in hoods.iter().enumerate() {
        for later in number..7 {
            assert!(
                !hood.body.contains(&format!("$N{later}")),
                "{number} reads {later}"
            );
        }
    }
    // Clean reads the picked colour; Spread the clean key; the morphs each the one before; the
    // blur of the key coming in, n1's key.
    assert!(hoods[1].body.contains("grade_picked(input, $N0,"));
    assert!(hoods[2].body.contains("grade_clean(q, $N1.x,"));
    assert!(hoods[3].body.contains("grade_spread(q, $N2.x,"));
    assert!(hoods[4].body.contains("grade_morph(q, $N3.x,"));
    assert!(hoods[5].body.ends_with("rgb = o0;\n"));
    assert!(hoods[6].body.ends_with("rgb = vec3<f32>(k0);\n"));
    assert!(ops[0].body.contains("grade_key_blurred(k0, $N6.x,"));
    assert!(ops[0].body.contains("grade_picked(input, $N5,"));
    // On a picture, the processor takes each in its order.
    let pixels = half();
    let shown = evaluate_picture(
        &grade,
        0.0,
        &options(),
        &NoLuts,
        &|_, rgb| rgb,
        (SIZE, SIZE),
        &pixels,
        1.0,
    );
    assert_eq!(shown.len(), pixels.len());
}

// ---------------------------------------------------------------------------------------------
// What the Highlight modes show
// ---------------------------------------------------------------------------------------------

#[test]
fn highlight_shows_the_picture_where_the_key_is_and_grey_where_it_is_not() {
    let grade = serial(vec![
        keyed(
            "n1",
            qualifier(|q| {
                q.mode = QualifierMode::Lum;
                q.lum.low = fixed(30.0);
                q.lum.low_soft = fixed(20.0);
            }),
        ),
        corrector("n2", |c| c.primaries.gain.r = fixed(1.2)),
    ]);
    let show = |node: &str, show: Show, rgb: [f32; 3]| {
        evaluate_showing(
            &grade,
            0.0,
            &options(),
            &NoLuts,
            &|_, rgb| rgb,
            rgb,
            &Look {
                node: node.into(),
                show,
            },
        )
    };
    // Keyed fully: the picture of the whole grade.
    let light = [0.6, 0.5, 0.4];
    assert_eq!(show("n1", Show::Highlight, light), eval(&grade, light));
    // Not at all: a flat grey.
    assert_eq!(show("n1", Show::Highlight, [0.05; 3]), [0.5; 3]);
    // In part: the two mixed by the key.
    let some = [0.2; 3];
    let k = show("n1", Show::Matte, some)[0];
    assert!(k > 0.0 && k < 1.0);
    let out = eval(&grade, some);
    near(
        show("n1", Show::Highlight, some),
        out.map(|v| 0.5 * (1.0 - k) + v * k),
        1e-6,
    );
    // A node without a key is shown alone: its picture, not the grade's.
    let alone = show("n2", Show::Highlight, light);
    assert_eq!(alone, eval(&grade, light));
    let first = serial(vec![corrector("n2", |c| c.primaries.gain.r = fixed(1.2))]);
    let solo = evaluate_showing(
        &serial(vec![
            corrector("n2", |c| c.primaries.gain.r = fixed(1.2)),
            corrector("n3", |c| c.primaries.saturation = fixed(0.0)),
        ]),
        0.0,
        &options(),
        &NoLuts,
        &|_, rgb| rgb,
        light,
        &Look {
            node: "n2".into(),
            show: Show::Highlight,
        },
    );
    assert_eq!(solo, eval(&first, light));
    // And its key, white throughout.
    assert_eq!(show("n2", Show::Matte, [0.05; 3]), [1.0; 3]);
}

#[test]
fn highlight_of_a_node_whose_key_comes_in_shows_grey_where_that_key_is_not() {
    let mut grade = serial(vec![
        corrector("n1", |c| {
            c.qualifier = Some(qualifier(|q| {
                q.mode = QualifierMode::Lum;
                q.lum.low = fixed(50.0);
            }));
        }),
        corrector("n2", |c| c.primaries.offset.b = fixed(75.0)),
    ]);
    grade.links.push(link("n1", "n2", Port::Key, 0));
    grade.sort_links();
    let show = |rgb: [f32; 3]| {
        evaluate_showing(
            &grade,
            0.0,
            &options(),
            &NoLuts,
            &|_, rgb| rgb,
            rgb,
            &Look {
                node: "n2".into(),
                show: Show::Highlight,
            },
        )
    };
    // n2 has no qualifier, but a key of its own coming in.
    assert_eq!(show([0.05; 3]), [0.5; 3]);
    assert_eq!(show([0.8; 3]), eval(&grade, [0.8; 3]));
}

#[test]
fn a_look_is_made_for_its_node_and_none_for_a_node_that_is_not_there() {
    let grade = serial(vec![
        keyed("n1", Qualifier::default()),
        Node::corrector("n2"),
    ]);
    let prepared = Prepared::new(&grade, &options(), &NoLuts);
    let look = |node: &str, show: Show| Look {
        node: node.into(),
        show,
    };
    let body = |look: &Look| prepared.ops_showing(3.0, look).unwrap()[0].body.to_string();
    let plain = prepared.ops_at(3.0).next().unwrap();
    let matte = body(&look("n1", Show::Matte));
    assert!(matte.ends_with("rgb = vec3<f32>(k0);\n"), "{matte}");
    assert!(body(&look("n1", Show::Highlight)).ends_with("rgb = mix(vec3<f32>(0.5), o1, k0);\n"));
    assert!(body(&look("n2", Show::Highlight)).ends_with("rgb = o1;\n"));
    assert_ne!(matte, *plain.body);
    // The same numbers as the grade's.
    let shown = prepared.ops_showing(3.0, &look("n1", Show::Matte)).unwrap();
    assert_eq!(shown[0].params, plain.params);
    assert!(prepared
        .ops_showing(3.0, &look("n9", Show::Matte))
        .is_none());
    let empty = Prepared::new(&Grade::default(), &options(), &NoLuts);
    assert!(empty.ops_showing(0.0, &look("n1", Show::Matte)).is_none());
}

// ---------------------------------------------------------------------------------------------
// The card
// ---------------------------------------------------------------------------------------------

/// A random qualifier of every mode, with no neighbourhood: its ranges, toggles, clip and invert.
fn random_qualifier(rng: &mut Rng) -> Qualifier {
    // Softness none, or wide enough that the roundings of the card and the processor, a few
    // in ten million, stay as small through it: a fall over a millionth would make them a key.
    let soft = |rng: &mut Rng, high: f64| {
        fixed(if rng.chance(0.2) {
            0.0
        } else {
            rng.range(5.0, high)
        })
    };
    let mut q = Qualifier::default();
    q.hue.center = fixed(rng.range(0.0, 100.0));
    q.hue.width = fixed(rng.range(0.0, 60.0));
    q.hue.soft = soft(rng, 30.0);
    q.hue.symmetry = fixed(match rng.below(4) {
        0 => 0.0,
        1 => 100.0,
        _ => rng.range(20.0, 80.0),
    });
    for range in [
        &mut q.sat,
        &mut q.lum,
        &mut q.red,
        &mut q.green,
        &mut q.blue,
    ] {
        range.low = fixed(rng.range(0.0, 50.0));
        range.high = fixed(rng.range(40.0, 100.0));
        range.low_soft = soft(rng, 20.0);
        range.high_soft = soft(rng, 20.0);
    }
    let mut r = |low: f64, high: f64| fixed(rng.range(low, high));
    q.finesse.black_clip = r(0.0, 30.0);
    q.finesse.white_clip = r(60.0, 100.0);
    q.mode = [QualifierMode::Hsl, QualifierMode::Rgb, QualifierMode::Lum][rng.below(3)];
    q.invert = rng.chance(0.3);
    q.hue.on = rng.chance(0.8);
    q.sat.on = rng.chance(0.8);
    q.lum.on = rng.chance(0.8);
    q.red.on = rng.chance(0.8);
    q.green.on = rng.chance(0.8);
    q.blue.on = rng.chance(0.8);
    q
}

#[test]
fn the_card_qualifies_and_mixes_keys_as_the_processor_does() {
    let Some(mut card) = Card::new() else { return };
    let mut rng = Rng(0xa11f_1e5e);
    let colours =
        |rng: &mut Rng| -> Vec<[f32; 3]> { (0..256).map(|_| rng.colour(-0.1, 1.3)).collect() };
    for round in 0..60 {
        // n1 keyed by its qualifier, n2 by its own and n1's (Matte or Mask), n3 outside n1, all
        // into a key mixer that keys n4; and each shown in both Highlight modes.
        let mut nodes = Vec::new();
        for id in ["n1", "n2", "n3", "n4"] {
            let mut node = random_corrector(&mut rng, id);
            if let NodeKind::Corrector(c) = &mut node.kind {
                if id != "n3" {
                    c.qualifier = Some(random_qualifier(&mut rng));
                }
                c.key.input.matte = rng.chance(0.5);
                c.key.input.invert = id == "n3";
                c.key.qualifier.invert = rng.chance(0.2);
                c.key.qualifier.gain = fixed(rng.range(0.3, 2.0));
                c.key.qualifier.offset = fixed(rng.range(-0.3, 0.3));
            }
            nodes.push(node);
        }
        let mut mixer = KeyMixer::default();
        mixer.inputs[1].matte = rng.chance(0.5);
        nodes.push(Node {
            id: "km".into(),
            label: String::new(),
            enabled: true,
            locked: false,
            pos: None,
            kind: NodeKind::KeyMixer(mixer),
        });
        let grade = graph(
            nodes,
            vec![
                link(SOURCE, "n1", Port::Rgb, 0),
                link("n1", "n2", Port::Rgb, 0),
                link("n1", "n2", Port::Key, 0),
                link("n2", "n3", Port::Rgb, 0),
                link("n1", "n3", Port::Key, 0),
                link("n3", "n4", Port::Rgb, 0),
                link("n2", "km", Port::Key, 0),
                link("n3", "km", Port::Key, 1),
                link("km", "n4", Port::Key, 0),
                link("n4", OUTPUT, Port::Rgb, 0),
            ],
        );
        let prepared = Prepared::new(&grade, &options(), &NoLuts);
        let node = ["n1", "n2", "n3", "km", "n4"][round % 5];
        for show in [None, Some(Show::Matte), Some(Show::Highlight)] {
            let look = show.map(|show| Look {
                node: node.into(),
                show,
            });
            let stage = Stage {
                ops: match &look {
                    None => prepared.ops_at(0.0).collect(),
                    Some(look) => prepared.ops_showing(0.0, look).unwrap(),
                },
            };
            let x = colours(&mut rng);
            let on_card = card.run(&stage, &x);
            for (x, on_card) in x.iter().zip(on_card) {
                let on_processor = match &look {
                    None => evaluate(&grade, 0.0, &options(), *x),
                    Some(look) => {
                        evaluate_showing(&grade, 0.0, &options(), &NoLuts, &|_, rgb| rgb, *x, look)
                    }
                };
                // The hue of the card's atan2 is some 1e-5 of a turn from the processor's, which
                // the steepest fall of a range of hue (some 75 to a turn) makes keys some 1e-3
                // apart: within 4e-3 then, of the size of a value beyond 1.
                let off = (0..3)
                    .map(|i| (on_card[i] - on_processor[i]).abs() / on_processor[i].abs().max(1.0))
                    .fold(0.0_f32, f32::max);
                assert!(
                    off <= 4e-3,
                    "{node} {show:?} {x:?}: the card made {on_card:?}, the processor \
                     {on_processor:?}"
                );
            }
        }
    }
    // The shape of the grade made one program, and its looks one each, whatever the settings.
    assert_eq!(card.pipelines.len(), 1 + 10);
}

#[test]
fn the_card_keys_ranges_and_hues_to_the_processor_and_a_new_qualifier_to_the_bit() {
    let Some(mut card) = Card::new() else { return };
    // A ramp of greys and a wheel of hues, through qualifiers of each mode.
    let mut x: Vec<[f32; 3]> = (0..=200_u8).map(|v| [f32::from(v) / 200.0; 3]).collect();
    x.extend((0..360_u16).map(|h| hue_colour(f32::from(h) / 360.0)));
    let qualifiers = [
        qualifier(|q| {
            q.mode = QualifierMode::Lum;
            q.lum.low = fixed(30.0);
            q.lum.high = fixed(60.0);
            q.lum.low_soft = fixed(10.0);
            q.lum.high_soft = fixed(20.0);
        }),
        qualifier(|q| {
            q.hue.center = fixed(0.0);
            q.hue.width = fixed(10.0);
            q.hue.soft = fixed(10.0);
            q.hue.symmetry = fixed(30.0);
            q.sat.low = fixed(20.0);
            q.sat.low_soft = fixed(5.0);
        }),
        qualifier(|q| {
            q.mode = QualifierMode::Rgb;
            q.red.low = fixed(60.0);
            q.green.high = fixed(40.0);
            q.green.high_soft = fixed(10.0);
            q.invert = true;
        }),
    ];
    for q in qualifiers {
        let grade = serial(vec![keyed("n1", q)]);
        let prepared = Prepared::new(&grade, &options(), &NoLuts);
        let look = Look {
            node: "n1".into(),
            show: Show::Matte,
        };
        let stage = Stage {
            ops: prepared.ops_showing(0.0, &look).unwrap(),
        };
        for (x, on_card) in x.iter().zip(card.run(&stage, &x)) {
            let wanted =
                evaluate_showing(&grade, 0.0, &options(), &NoLuts, &|_, rgb| rgb, *x, &look);
            near(on_card, wanted, 1e-4);
        }
    }
    // A qualifier as it is first changes nothing, to the bit, on the card too.
    let colours = colours();
    let plain = one(|c| c.primaries.saturation = fixed(70.0));
    let qualified = one(|c| {
        c.primaries.saturation = fixed(70.0);
        c.qualifier = Some(Qualifier::default());
    });
    let a = card.run(&stage_of(&plain, 0.0, &options(), &NoLuts), &colours);
    let b = card.run(&stage_of(&qualified, 0.0, &options(), &NoLuts), &colours);
    assert_eq!(a, b);
}
