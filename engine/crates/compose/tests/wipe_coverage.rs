use columbina_compose::wipe::{reference, Style, Wipe};

#[test]
fn current_coverage_follows_every_actual_wipe_pixel_including_softness_and_endpoints() {
    let canvas = (37, 23);
    let a = vec![[1.0; 4]; 37 * 23];
    let b = vec![[0.0, 0.0, 0.0, 1.0]; 37 * 23];
    for style in [
        Style::Horizontal,
        Style::Vertical,
        Style::Diagonal,
        Style::Box,
        Style::Mix,
        Style::Difference,
        Style::VenetianBlind,
        Style::Checkerboard,
    ] {
        for invert in [false, true] {
            for position in [0.0, 0.37, 0.5, 1.0] {
                for softness in [0.0, 0.25, 1.0] {
                    let wipe = Wipe {
                        style,
                        position,
                        softness,
                        invert,
                        angle: -31.0,
                        ..Wipe::default()
                    };
                    let pixels = reference(&wipe, canvas.0, canvas.1, &a, &b);
                    for (i, pixel) in pixels.iter().enumerate() {
                        #[allow(clippy::cast_precision_loss)]
                        let at = [(i % 37) as f32 + 0.5, (i / 37) as f32 + 0.5];
                        assert_eq!(
                            wipe.has_current(canvas, at),
                            pixel[0] > 0.0,
                            "{wipe:?}, {at:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn coverage_includes_the_mixed_ramp_but_not_the_pure_reference_side() {
    let wipe = Wipe {
        softness: 0.2,
        ..Wipe::default()
    };
    assert!(wipe.has_current((100, 80), [49.5, 40.5]));
    assert!(wipe.has_current((100, 80), [57.5, 40.5]));
    assert!(!wipe.has_current((100, 80), [58.5, 40.5]));
    let inverted = Wipe {
        invert: true,
        ..wipe
    };
    assert!(!inverted.has_current((100, 80), [41.5, 40.5]));
    assert!(inverted.has_current((100, 80), [42.5, 40.5]));
    assert!(Wipe {
        style: Style::Difference,
        gain: 0.0,
        invert: true,
        ..wipe
    }
    .has_current((100, 80), [99.5, 0.5]));
}

#[test]
fn bilinear_coverage_counts_positive_weights_and_clamps_the_source_footprint() {
    let wipe = Wipe::default();
    assert!(wipe.sample_has_current((120, 80), [60.25, 40.25]));
    assert!(
        !wipe.sample_has_current((120, 80), [60.5, 40.5]),
        "A neighbour has zero sampling weight"
    );
    assert!(!wipe.sample_has_current((120, 80), [60.75, 40.25]));
    let inverted = Wipe {
        invert: true,
        ..wipe
    };
    assert!(inverted.sample_has_current((120, 80), [59.75, 40.25]));
    assert!(!inverted.sample_has_current((120, 80), [59.5, 40.5]));
    let boxed = Wipe {
        style: Style::Box,
        ..wipe
    };
    assert!(
        boxed.sample_has_current((120, 80), [90.25, 60.25]),
        "positive corner product is 1/16"
    );
    assert!(
        !boxed.sample_has_current((120, 80), [90.5, 60.25]),
        "current corner's x weight is zero"
    );
    let checker = Wipe {
        style: Style::Checkerboard,
        ..wipe
    };
    assert!(checker.sample_has_current((120, 80), [30.25, 5.25]));
    assert!(!checker.sample_has_current((120, 80), [30.5, 5.5]));
    assert!(Wipe {
        invert: true,
        ..checker
    }
    .sample_has_current((120, 80), [29.75, 5.25]));
    for style in [
        Style::Horizontal,
        Style::Vertical,
        Style::Diagonal,
        Style::Box,
        Style::Mix,
    ] {
        for (position, invert) in [(0.0, false), (1.0, true)] {
            let endpoint = Wipe {
                style,
                position: if style == Style::Mix {
                    1.0 - position
                } else {
                    position
                },
                invert,
                ..wipe
            };
            for point in [[0.25, 0.25], [119.75, 79.75]] {
                assert!(
                    !endpoint.sample_has_current((120, 80), point),
                    "clamped pure-B endpoint: {endpoint:?}, {point:?}"
                );
                assert!(Wipe {
                    invert: !invert,
                    ..endpoint
                }
                .sample_has_current((120, 80), point));
            }
        }
    }
    assert!(Wipe {
        style: Style::Difference,
        gain: 0.0,
        ..wipe
    }
    .sample_has_current((120, 80), [119.75, 79.75]));
}
