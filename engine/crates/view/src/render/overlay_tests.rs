use super::tests::{draw_using, gpu, halves_using, near_mapped};
use super::Painter;
use crate::display::{mapping, Content, HdrTransfer, Output};
use crate::window_overlay::render::OverlayPainter;
use crate::window_overlay::{Affine, Bezier, Shape, ShapeKind, Style, WindowOverlay};
use crate::Viewport;
use columbina_compose::wipe::{Style as WipeStyle, Wipe};

fn overlay() -> WindowOverlay {
    WindowOverlay {
        shapes: vec![Shape {
            curve: None,
            id: "w1".into(),
            radius: 1.0,
            kind: ShapeKind::Circular,
            feather: 0.0,
            points: vec![],
            feathers: vec![],
            to_canvas: Affine {
                xx: 20.0,
                xy: 0.0,
                yx: 0.0,
                yy: 10.0,
                tx: 60.5,
                ty: 40.5,
            },
        }]
        .into(),
        selected: Some("w1".into()),
        style: Style::default(),
        coverage: None,
        points: vec![],
        band: None,
    }
}

fn source(gpu: &super::tests::Gpu) -> wgpu::Texture {
    solid_source(gpu, [26, 51, 77, 255])
}

fn solid_source(gpu: &super::tests::Gpu, colour: [u8; 4]) -> wgpu::Texture {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("overlay source must remain untouched"),
        size: wgpu::Extent3d {
            width: 120,
            height: 80,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        texture.as_image_copy(),
        &colour.repeat(120 * 80),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(120 * 4),
            rows_per_image: None,
        },
        texture.size(),
    );
    texture
}

#[test]
fn window_overlay_loads_the_display_surface_and_never_changes_the_composed_texture() {
    let Some(gpu) = gpu() else { return };
    let frame = source(&gpu);
    let size = (120, 120);
    let map = Viewport::Fit.map(size, (120, 80), 0).unwrap();
    let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
    painter.use_texture(&frame).unwrap();
    let before = draw_using(&gpu, size, |v| painter.draw_mapped(v, size, 0, Some(map)));
    let osc = overlay();
    let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    let drawn = draw_using(&gpu, size, |target| {
        painter.draw_mapped_with(target, size, 0, Some(map), |encoder| {
            overlay_painter.encode(&gpu.device, encoder, target, &osc, map, Output::Sdr);
        })
    });
    assert_eq!(
        drawn[60 * 120 + 80],
        [38, 115, 255],
        "the blue Aspect handle is a surface control"
    );
    assert_eq!(
        drawn[60 * 120 + 60],
        before[60 * 120 + 60],
        "interior is still the image"
    );
    assert_eq!(drawn[10 * 120 + 60], [0; 3], "black bars stay black");
    assert_eq!(
        before,
        draw_using(&gpu, size, |v| painter.draw_mapped(v, size, 0, Some(map))),
        "sampling the composed texture again contains no controls"
    );
}

#[test]
fn window_overlay_uses_actual_output_sdr_white_independently_of_hdr_content() {
    let Some(gpu) = gpu() else { return };
    let frame = source(&gpu);
    let size = (120, 80);
    let map = Viewport::Fit.map(size, size, 0).unwrap();
    for output in [
        Output::Sdr,
        Output::Hdr {
            peak_nits: 400.0,
            sdr_white_nits: 240.0,
        },
        Output::Hdr {
            peak_nits: 1000.0,
            sdr_white_nits: 80.0,
        },
    ] {
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba16Float);
        painter.set_mapping(mapping(
            Content::Hdr {
                transfer: HdrTransfer::Pq,
                peak_nits: 1000.0,
            },
            output,
        ));
        painter.use_texture(&frame).unwrap();
        let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Rgba16Float);
        let mut osc = overlay();
        osc.style.selected = [1.0, 0.5, 0.0];
        let pixels = halves_using(&gpu, size, |target| {
            painter.draw_mapped_with(target, size, 0, Some(map), |encoder| {
                overlay_painter.encode(&gpu.device, encoder, target, &osc, map, output);
            })
        });
        let wanted =
            mapping(Content::Sdr, output).map_or(osc.style.selected, |m| m.map(osc.style.selected));
        assert!(
            near_mapped(pixels[33 * 120 + 60], wanted),
            "{output:?}: {:?}, {wanted:?}",
            pixels[33 * 120 + 60]
        );
    }
}

#[test]
fn window_overlay_gpu_clips_each_outline_fragment_like_cpu_hit_for_all_wipes() {
    let Some(gpu) = gpu() else { return };
    let frame = source(&gpu);
    let size = (120, 80);
    let map = Viewport::Fit.map(size, size, 0).unwrap();
    let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
    painter.use_texture(&frame).unwrap();
    let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    let mut osc = overlay();
    for style in [
        WipeStyle::Horizontal,
        WipeStyle::Vertical,
        WipeStyle::Diagonal,
        WipeStyle::Box,
        WipeStyle::Mix,
        WipeStyle::Difference,
        WipeStyle::VenetianBlind,
        WipeStyle::Checkerboard,
    ] {
        for invert in [false, true] {
            for position in [0.0, 0.5, 1.0] {
                osc.coverage = Some(Wipe {
                    style,
                    invert,
                    position,
                    softness: 0.25,
                    ..Wipe::default()
                });
                let drawn = draw_using(&gpu, size, |target| {
                    painter.draw_mapped_with(target, size, 0, Some(map), |encoder| {
                        overlay_painter.encode(
                            &gpu.device,
                            encoder,
                            target,
                            &osc,
                            map,
                            Output::Sdr,
                        );
                    })
                });
                for (i, pixel) in drawn.iter().enumerate() {
                    #[allow(clippy::cast_precision_loss)]
                    let point = [(i % 120) as f32 + 0.5, (i / 120) as f32 + 0.5];
                    let alpha = osc.outline(map, point).map_or(0.0, |(_, alpha)| alpha);
                    let wanted = cpu_pixel(&osc, map, point, [26.0, 51.0, 77.0]);
                    assert!(
                        (0..3).all(|ch| (f32::from(pixel[ch]) - wanted[ch]).abs() < 2.1),
                        "{style:?}, invert={invert}, position={position}, point={point:?}, actual={pixel:?}, wanted={wanted:?}"
                    );
                    if alpha > 0.0 {
                        assert_eq!(osc.hit(map, point), Some("w1"));
                    }
                }
            }
        }
    }
}

#[test]
fn window_overlay_gpu_matches_rotated_zoomed_geometry_and_extreme_legal_viewports() {
    let Some(gpu) = gpu() else { return };
    let frame = source(&gpu);
    let size = (100, 100);
    let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
    painter.use_texture(&frame).unwrap();
    let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    let mut osc = overlay();
    osc.shapes[0].to_canvas = Affine {
        xx: 0.0,
        xy: -10.0,
        yx: 20.0,
        yy: 0.0,
        tx: 60.0,
        ty: 40.0,
    };
    for scale in [2.0_f64.powi(-126), 0.25, 2.0, 1e20, 2.0_f64.powi(112)] {
        let map = Viewport::Scale {
            scale,
            center: [0.5, 0.5],
        }
        .map(size, (120, 80), 0)
        .unwrap();
        let base = draw_using(&gpu, size, |target| {
            painter.draw_mapped(target, size, 0, Some(map))
        });
        let drawn = draw_using(&gpu, size, |target| {
            painter.draw_mapped_with(target, size, 0, Some(map), |encoder| {
                overlay_painter.encode(&gpu.device, encoder, target, &osc, map, Output::Sdr);
            })
        });
        for (i, pixel) in drawn.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let point = [(i % 100) as f32 + 0.5, (i / 100) as f32 + 0.5];
            let wanted = cpu_pixel(&osc, map, point, base[i].map(f32::from));
            assert!(
                (0..3).all(|ch| (f32::from(pixel[ch]) - wanted[ch]).abs() < 2.1),
                "scale={scale}, point={point:?}: {pixel:?}, {wanted:?}"
            );
        }
    }
}

#[test]
fn window_overlay_bilinear_footprint_follows_actual_composed_image_samples() {
    let Some(shared) = columbina_compose::gpu_for_tests() else {
        return;
    };
    let gpu = super::tests::Gpu {
        device: shared.device().clone(),
        queue: shared.queue().clone(),
    };
    let (current, reference) = (
        solid_source(&gpu, [255, 0, 0, 255]),
        solid_source(&gpu, [0, 0, 0, 255]),
    );
    let composed = columbina_compose::Target::preview(&shared, 120, 80).unwrap();
    let wiper = columbina_compose::wipe::Wiper::new(shared);
    let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba16Float);
    painter.use_texture(composed.texture()).unwrap();
    let controls = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Rgba16Float);
    let mut osc = overlay();
    osc.style.selected = [0.0, 1.0, 0.0];
    // Expected current shares are hand-computed filter weights, checked against the actual
    // GPU Wiper -> Painter output before checking controls. They are not coverage predicates.
    for (style, invert, position, pixel, doubled, share) in [
        (WipeStyle::Horizontal, false, 0.5, [120, 80], true, 0.25),
        (WipeStyle::Horizontal, false, 0.5, [121, 80], true, 0.0),
        (WipeStyle::Horizontal, true, 0.5, [119, 80], true, 0.25),
        (WipeStyle::Horizontal, true, 0.5, [118, 80], true, 0.0),
        (WipeStyle::Box, false, 0.5, [180, 80], true, 0.25),
        (WipeStyle::Box, false, 0.5, [180, 120], true, 0.0625),
        (WipeStyle::Box, true, 0.5, [179, 80], true, 0.25),
        (WipeStyle::Checkerboard, false, 0.5, [60, 10], true, 0.25),
        (WipeStyle::Checkerboard, false, 0.5, [60, 20], true, 0.625),
        (WipeStyle::Checkerboard, true, 0.5, [60, 20], true, 0.375),
        (WipeStyle::Horizontal, false, 0.0, [0, 0], true, 0.0),
        (WipeStyle::Horizontal, true, 1.0, [239, 159], true, 0.0),
        (WipeStyle::Horizontal, true, 0.0, [0, 0], true, 1.0),
        (WipeStyle::Horizontal, false, 1.0, [239, 159], true, 1.0),
        (WipeStyle::Mix, false, 1.0, [120, 80], true, 0.0),
        (WipeStyle::Mix, false, 0.0, [120, 80], true, 1.0),
        (WipeStyle::Mix, true, 1.0, [120, 80], true, 1.0),
        (WipeStyle::Mix, true, 0.0, [120, 80], true, 0.0),
        (WipeStyle::Difference, true, 0.5, [120, 80], true, 0.0),
        (WipeStyle::Horizontal, false, 0.5, [60, 40], false, 0.0),
        (WipeStyle::Horizontal, true, 0.5, [59, 40], false, 0.0),
        (WipeStyle::Box, false, 0.5, [90, 60], false, 0.0),
        (WipeStyle::Checkerboard, false, 0.5, [30, 5], false, 0.0),
    ] {
        let wipe = Wipe {
            style,
            invert,
            position,
            gain: 0.0,
            ..Wipe::default()
        };
        let size = if doubled { (240, 160) } else { (120, 80) };
        let map = Viewport::Scale {
            scale: if doubled { 2.0 } else { 1.0 },
            center: [0.5, 0.5],
        }
        .map(size, (120, 80), 0)
        .unwrap();
        wiper
            .wipe(&current, &reference, &wipe, composed.texture())
            .unwrap();
        let sample = usize::try_from(pixel[1] * size.0 + pixel[0]).unwrap();
        let base = halves_using(&gpu, size, |target| {
            painter.draw_mapped(target, size, 0, Some(map))
        });
        assert!(
            (base[sample][0] - share).abs() < 1e-3,
            "actual filtered image: {wipe:?}, {pixel:?}, {:?}, expected {share}",
            base[sample]
        );
        #[allow(clippy::cast_precision_loss)]
        let device = pixel.map(|value| value as f32 + 0.5);
        let canvas = map.to_picture((f64::from(device[0]), f64::from(device[1])));
        // Put a full-opacity right stroke precisely on this device sample.
        osc.shapes[0].to_canvas = Affine {
            xx: 10.0,
            xy: 0.0,
            yx: 0.0,
            yy: 10.0,
            tx: canvas.0 - 10.0,
            ty: canvas.1,
        };
        osc.coverage = Some(wipe);
        let drawn = halves_using(&gpu, size, |target| {
            painter.draw_mapped_with(target, size, 0, Some(map), |encoder| {
                controls.encode(&gpu.device, encoder, target, &osc, map, Output::Sdr);
            })
        });
        let allowed = share > 0.0 || style == WipeStyle::Difference;
        assert!(
            (drawn[sample][1] - if allowed { 0.45 } else { 0.0 }).abs() < 1e-3,
            "stroke must follow actual current contribution: {wipe:?}, {pixel:?}, base={:?}, drawn={:?}",
            base[sample],
            drawn[sample]
        );
        assert_eq!(
            osc.hit(map, device),
            allowed.then_some("w1"),
            "{wipe:?}, {pixel:?}"
        );
    }
}

// Use the CPU's shared primitive coverage, not a second implementation of ellipse/line maths.
fn cpu_pixel(
    osc: &WindowOverlay,
    map: crate::ViewportMap,
    point: [f32; 2],
    base: [f32; 3],
) -> [f32; 3] {
    if osc.outline(map, point).is_none() {
        return base;
    }
    osc.ordered()
        .flat_map(|shape| osc.primitives(shape, map))
        .fold(base, |pixel, primitive| {
            let alpha = primitive.alpha(point, osc.style.width);
            std::array::from_fn(|ch| {
                pixel[ch] * (1.0 - alpha) + primitive.colour[ch] * 255.0 * alpha
            })
        })
}

#[test]
fn window_overlay_gradient_controls_share_rotated_device_geometry_and_wipe_coverage() {
    let Some(gpu) = gpu() else { return };
    let size = (120, 80);
    let frame = source(&gpu);
    let map = Viewport::Scale {
        scale: 1.5,
        center: [0.5, 0.5],
    }
    .map(size, size, 0)
    .unwrap();
    let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
    painter.use_texture(&frame).unwrap();
    let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    let mut osc = overlay();
    osc.shapes[0].kind = ShapeKind::Gradient;
    osc.shapes[0].to_canvas = Affine {
        xx: 20.0,
        xy: -10.0,
        yx: 10.0,
        yy: 20.0,
        tx: 60.5,
        ty: 40.5,
    };
    for feather in [0.0, 0.25] {
        osc.shapes[0].feather = feather;
        for style in [WipeStyle::Horizontal, WipeStyle::Mix, WipeStyle::Difference] {
            osc.coverage = Some(Wipe {
                style,
                softness: 0.25,
                ..Wipe::default()
            });
            let base = draw_using(&gpu, size, |v| painter.draw_mapped(v, size, 0, Some(map)));
            let drawn = draw_using(&gpu, size, |target| {
                painter.draw_mapped_with(target, size, 0, Some(map), |encoder| {
                    overlay_painter.encode(&gpu.device, encoder, target, &osc, map, Output::Sdr);
                })
            });
            for (i, pixel) in drawn.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let point = [(i % 120) as f32 + 0.5, (i / 120) as f32 + 0.5];
                let wanted = cpu_pixel(&osc, map, point, base[i].map(f32::from));
                assert!(
                    (0..3).all(|ch| (f32::from(pixel[ch]) - wanted[ch]).abs() < 2.1),
                    "{point:?} {pixel:?} {wanted:?}"
                );
                // Its feather lines are display only (author decision 2026-11-02).
                if osc.outline(map, point).is_some() && controls(&osc, map, point) {
                    assert_eq!(osc.hit(map, point), Some("w1"));
                }
            }
        }
    }
}

#[test]
fn window_overlay_draws_controls_only_on_the_selected_window_and_greys_the_others() {
    let Some(gpu) = gpu() else { return };
    let size = (120, 80);
    let frame = source(&gpu);
    let map = Viewport::Fit.map(size, size, 0).unwrap();
    let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
    painter.use_texture(&frame).unwrap();
    let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    let mut osc = overlay();
    let mut other = osc.shapes[0].clone();
    osc.shapes[0].to_canvas.tx = 30.5;
    other.id = "w2".into();
    other.to_canvas.tx = 90.5;
    other.feather = 0.25;
    osc.shapes = vec![osc.shapes[0].clone(), other].into();
    let base = draw_using(&gpu, size, |v| painter.draw_mapped(v, size, 0, Some(map)));
    let drawn = draw_using(&gpu, size, |target| {
        painter.draw_mapped_with(target, size, 0, Some(map), |encoder| {
            overlay_painter.encode(&gpu.device, encoder, target, &osc, map, Output::Sdr);
        })
    });
    let at = |x: usize, y: usize| drawn[y * 120 + x];
    // The selected window's corner Size handle is blue; the other window's corner is untouched.
    assert!(
        at(10, 30)
            .iter()
            .zip([38, 115, 255])
            .all(|(&c, want)| c.abs_diff(want) <= 1),
        "{:?}",
        at(10, 30)
    );
    assert_eq!(at(70, 30), base[30 * 120 + 70]);
    // The unselected window's outline is grey where the selected one would put an Aspect handle,
    // and its outer soft edge is not drawn.
    assert!(
        at(70, 40).iter().all(|&c| c.abs_diff(128) <= 1),
        "{:?}",
        at(70, 40)
    );
    assert_eq!(at(115, 40), base[40 * 120 + 115]);
    for (i, pixel) in drawn.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let point = [(i % 120) as f32 + 0.5, (i / 120) as f32 + 0.5];
        let wanted = cpu_pixel(&osc, map, point, base[i].map(f32::from));
        assert!(
            (0..3).all(|ch| (f32::from(pixel[ch]) - wanted[ch]).abs() < 2.1),
            "{point:?} {pixel:?} {wanted:?}"
        );
    }
}

#[test]
fn window_overlay_linear_and_polygon_controls_match_cpu_geometry_through_wipes() {
    let Some(gpu) = gpu() else { return };
    let size = (120, 80);
    let frame = source(&gpu);
    let map = Viewport::Scale {
        scale: 1.25,
        center: [0.5, 0.5],
    }
    .map(size, size, 0)
    .unwrap();
    let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
    painter.use_texture(&frame).unwrap();
    let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    let rotated = Affine {
        xx: 70.0,
        xy: -25.0,
        yx: 20.0,
        yy: 60.0,
        tx: 50.5,
        ty: 40.5,
    };
    let linear = Shape {
        curve: None,
        id: "lin".into(),
        radius: 0.25,
        kind: ShapeKind::Linear,
        feather: 0.0,
        points: vec![[-0.25, -0.2], [0.3, -0.25], [0.2, 0.25], [-0.25, 0.2]],
        feathers: vec![[0.04, 0.04], [0.0, 0.0], [0.06, 0.06], [0.02, 0.02]],
        to_canvas: rotated,
    };
    let polygon = Shape {
        curve: None,
        id: "poly".into(),
        kind: ShapeKind::Polygon,
        // Self-intersecting on purpose: drawn and hit in the stored order.
        points: vec![
            [-0.3, -0.1],
            [0.3, 0.2],
            [0.25, -0.2],
            [-0.2, 0.25],
            [0.0, -0.3],
        ],
        feathers: vec![[0.03, 0.05]; 5],
        to_canvas: Affine {
            tx: 70.5,
            ..rotated
        },
        ..linear.clone()
    };
    let mut osc = overlay();
    osc.shapes = vec![linear, polygon].into();
    for selected in ["lin", "poly"] {
        osc.selected = Some(selected.into());
        for style in [WipeStyle::Horizontal, WipeStyle::Mix] {
            osc.coverage = Some(Wipe {
                style,
                softness: 0.25,
                ..Wipe::default()
            });
            let base = draw_using(&gpu, size, |v| painter.draw_mapped(v, size, 0, Some(map)));
            let drawn = draw_using(&gpu, size, |target| {
                painter.draw_mapped_with(target, size, 0, Some(map), |encoder| {
                    overlay_painter.encode(&gpu.device, encoder, target, &osc, map, Output::Sdr);
                })
            });
            let mut stroked = 0;
            for (i, pixel) in drawn.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let point = [(i % 120) as f32 + 0.5, (i / 120) as f32 + 0.5];
                let wanted = cpu_pixel(&osc, map, point, base[i].map(f32::from));
                assert!(
                    (0..3).all(|ch| (f32::from(pixel[ch]) - wanted[ch]).abs() < 2.1),
                    "{selected} {style:?} {point:?} {pixel:?} {wanted:?}"
                );
                if osc.outline(map, point).is_some() {
                    stroked += 1;
                    if controls(&osc, map, point) {
                        assert!(osc.hit(map, point).is_some(), "{point:?}");
                    }
                }
            }
            assert!(stroked > 200, "{stroked}");
        }
    }
}

/// Whether a stroke or handle that can be hit covers the point; guides are display only.
fn controls(osc: &WindowOverlay, map: crate::ViewportMap, point: [f32; 2]) -> bool {
    osc.ordered()
        .flat_map(|shape| osc.primitives(shape, map))
        .any(|p| !p.guide && p.alpha(point, osc.style.width) > 0.0)
}

#[test]
fn window_overlay_exact_softness_guides_match_cpu_and_are_never_hit() {
    let Some(gpu) = gpu() else { return };
    let size = (120, 80);
    let frame = source(&gpu);
    let map = Viewport::Scale {
        scale: 1.25,
        center: [0.5, 0.5],
    }
    .map(size, size, 0)
    .unwrap();
    let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
    painter.use_texture(&frame).unwrap();
    let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    // Rotated with a different scale per local axis, as a non-square Window maps.
    let (sin, cos) = 0.4_f64.sin_cos();
    let placed = |tx: f64, ty: f64| Affine {
        xx: 70.0 * cos,
        xy: -45.0 * sin,
        yx: 70.0 * sin,
        yy: 45.0 * cos,
        tx,
        ty,
    };
    // A reflex L: an inner arc and an outer mitre at its notch.
    let reflex = Shape {
        curve: None,
        id: "l".into(),
        radius: 0.25,
        kind: ShapeKind::Polygon,
        feather: 0.0,
        points: vec![
            [-0.3, -0.3],
            [0.3, -0.3],
            [0.3, 0.0],
            [0.0, 0.0],
            [0.0, 0.3],
            [-0.3, 0.3],
        ],
        feathers: vec![[0.06, 0.1]; 6],
        to_canvas: placed(35.5, 40.5),
    };
    // Narrow, with more inside softness than it holds: only the outer guide is drawn.
    let narrow = Shape {
        curve: None,
        id: "n".into(),
        points: vec![[-0.4, -0.05], [0.4, -0.05], [0.4, 0.05], [-0.4, 0.05]],
        feathers: vec![[0.2, 0.05]; 4],
        to_canvas: placed(85.5, 30.5),
        ..reflex.clone()
    };
    let linear = Shape {
        curve: None,
        id: "q".into(),
        kind: ShapeKind::Linear,
        points: vec![[-0.2, -0.2], [0.2, -0.15], [0.2, 0.2], [-0.2, 0.2]],
        feathers: vec![[0.05; 2], [0.0; 2], [0.1; 2], [0.02; 2]],
        to_canvas: placed(85.5, 60.5),
        ..reflex.clone()
    };
    let mut osc = overlay();
    osc.shapes = vec![reflex, narrow, linear].into();
    for selected in ["l", "n", "q"] {
        osc.selected = Some(selected.into());
        let base = draw_using(&gpu, size, |v| painter.draw_mapped(v, size, 0, Some(map)));
        let drawn = draw_using(&gpu, size, |target| {
            painter.draw_mapped_with(target, size, 0, Some(map), |encoder| {
                overlay_painter.encode(&gpu.device, encoder, target, &osc, map, Output::Sdr);
            })
        });
        let mut guides = 0;
        for (i, pixel) in drawn.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let point = [(i % 120) as f32 + 0.5, (i / 120) as f32 + 0.5];
            let wanted = cpu_pixel(&osc, map, point, base[i].map(f32::from));
            assert!(
                (0..3).all(|ch| (f32::from(pixel[ch]) - wanted[ch]).abs() < 2.1),
                "{selected} {point:?} {pixel:?} {wanted:?}"
            );
            if osc.outline(map, point).is_some() && !controls(&osc, map, point) {
                guides += 1;
                // Off every body, a guide pixel selects and moves nothing.
                if osc.shapes.iter().all(|s| !s.contains(map, point)) {
                    assert_eq!(osc.hit(map, point), None, "{selected} {point:?}");
                }
            }
        }
        assert!(guides > 100, "{selected}: {guides}");
    }
}

/// Encode and GPU time per frame of a selected 128-point Polygon with softness, the documented
/// maximum, at 1080p and 4K. GPU is the submit-to-idle wall time after encoding.
#[test]
#[ignore = "GPU timing, run explicitly"]
fn window_overlay_128_point_selected_polygon_timing() {
    let Some(gpu) = gpu() else { return };
    let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    for size in [(1920_u32, 1080_u32), (3840, 2160)] {
        let map = Viewport::Fit.map(size, size, 0).unwrap();
        let height = f64::from(size.1);
        let shape = Shape {
            curve: None,
            id: "p".into(),
            radius: 0.25,
            kind: ShapeKind::Polygon,
            feather: 0.0,
            points: (0..128)
                .map(|k| {
                    let a = f64::from(k) / 128.0 * std::f64::consts::TAU;
                    [0.4 * a.cos(), 0.4 * a.sin()]
                })
                .collect(),
            feathers: vec![[0.05, 0.05]; 128],
            to_canvas: Affine {
                xx: height,
                xy: 0.0,
                yx: 0.0,
                yy: height,
                tx: f64::from(size.0) / 2.0,
                ty: height / 2.0,
            },
        };
        let osc = WindowOverlay {
            shapes: vec![shape].into(),
            selected: Some("p".into()),
            style: Style::default(),
            coverage: None,
            points: vec![],
            band: None,
        };
        let built = std::time::Instant::now();
        let primitives = osc.primitives(&osc.shapes[0], map).len();
        let built = built.elapsed().as_secs_f64() * 1000.0;
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let (mut encode, mut total) = (0.0, 0.0);
        for frame in 0..25 {
            let start = std::time::Instant::now();
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            overlay_painter.encode(&gpu.device, &mut encoder, &view, &osc, map, Output::Sdr);
            let commands = encoder.finish();
            let spent = start.elapsed();
            gpu.queue.submit([commands]);
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            if frame >= 5 {
                encode += spent.as_secs_f64() * 1000.0 / 20.0;
                total += start.elapsed().as_secs_f64() * 1000.0 / 20.0;
            }
        }
        println!(
            "TIMING {}x{} primitives={primitives} (built in {built:.2} ms): encode {encode:.2} ms, gpu {:.2} ms, encode+gpu {total:.2} ms per frame",
            size.0,
            size.1,
            total - encode
        );
    }
}

/// A Curve of `count` anchors on a circle of `radius`, with tangent handles (closed or a draft).
fn round_curve(id: &str, count: u32, radius: f64, closed: bool, to_canvas: Affine) -> Shape {
    let step = std::f64::consts::TAU / f64::from(count);
    let k = radius * 4.0 / 3.0 * (step / 4.0).tan();
    let anchors: Vec<_> = (0..count)
        .map(|i| {
            let (sin, cos) = (f64::from(i) * step).sin_cos();
            (
                [radius * cos, radius * sin],
                [k * sin, -k * cos],
                [-k * sin, k * cos],
            )
        })
        .collect();
    Shape {
        id: id.into(),
        curve: Some(Bezier {
            handles: anchors.iter().map(|a| [a.1, a.2]).collect(),
            closed,
            tolerance: columbina_compose::grade::curve::tolerance(1.0),
        }),
        radius: 0.25,
        kind: ShapeKind::Curve,
        feather: 0.0,
        points: anchors.iter().map(|a| a.0).collect(),
        feathers: vec![[0.05, 0.03]],
        to_canvas,
    }
}

#[test]
fn window_overlay_curve_controls_and_guides_match_cpu_geometry_through_wipes() {
    let Some(gpu) = gpu() else { return };
    let size = (120, 80);
    let frame = source(&gpu);
    let map = Viewport::Scale {
        scale: 1.25,
        center: [0.5, 0.5],
    }
    .map(size, size, 0)
    .unwrap();
    let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
    painter.use_texture(&frame).unwrap();
    let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    // Rotated with a different scale per local axis.
    let rotated = Affine {
        xx: 70.0,
        xy: -25.0,
        yx: 20.0,
        yy: 60.0,
        tx: 45.5,
        ty: 40.5,
    };
    let closed = round_curve("closed", 5, 0.3, true, rotated);
    let draft = round_curve(
        "draft",
        3,
        0.2,
        false,
        Affine {
            tx: 85.5,
            ..rotated
        },
    );
    let mut osc = overlay();
    osc.shapes = vec![closed, draft].into();
    for selected in ["closed", "draft"] {
        osc.selected = Some(selected.into());
        for style in [WipeStyle::Horizontal, WipeStyle::Mix] {
            osc.coverage = Some(Wipe {
                style,
                softness: 0.25,
                ..Wipe::default()
            });
            let base = draw_using(&gpu, size, |v| painter.draw_mapped(v, size, 0, Some(map)));
            let drawn = draw_using(&gpu, size, |target| {
                painter.draw_mapped_with(target, size, 0, Some(map), |encoder| {
                    overlay_painter.encode(&gpu.device, encoder, target, &osc, map, Output::Sdr);
                })
            });
            let (mut stroked, mut guided) = (0, 0);
            for (i, pixel) in drawn.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let point = [(i % 120) as f32 + 0.5, (i / 120) as f32 + 0.5];
                let wanted = cpu_pixel(&osc, map, point, base[i].map(f32::from));
                assert!(
                    (0..3).all(|ch| (f32::from(pixel[ch]) - wanted[ch]).abs() < 2.1),
                    "{selected} {style:?} {point:?} {pixel:?} {wanted:?}"
                );
                if osc.outline(map, point).is_some() {
                    stroked += 1;
                    if controls(&osc, map, point) {
                        assert!(osc.hit(map, point).is_some(), "{point:?}");
                    } else {
                        guided += 1;
                    }
                }
            }
            assert!(stroked > 200, "{stroked}");
            if selected == "closed" {
                assert!(guided > 50, "guides drawn: {guided}");
            }
        }
    }
}

#[test]
fn window_overlay_selected_points_box_and_band_match_cpu_and_only_box_handles_hit() {
    let Some(gpu) = gpu() else { return };
    let size = (120, 80);
    let frame = source(&gpu);
    let map = Viewport::Fit.map(size, size, 0).unwrap();
    let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
    painter.use_texture(&frame).unwrap();
    let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    let mut osc = overlay();
    // Five anchors on a circle of radius .3 about device (60.5, 40.5), 80 device px per unit.
    osc.shapes = vec![round_curve(
        "c",
        5,
        0.3,
        true,
        Affine {
            xx: 80.0,
            xy: 0.0,
            yx: 0.0,
            yy: 80.0,
            tx: 60.5,
            ty: 40.5,
        },
    )]
    .into();
    osc.selected = Some("c".into());
    osc.points = vec![0, 1];
    osc.band = Some([5.0, 5.0, 30.0, 20.0]);
    let base = draw_using(&gpu, size, |v| painter.draw_mapped(v, size, 0, Some(map)));
    let drawn = draw_using(&gpu, size, |target| {
        painter.draw_mapped_with(target, size, 0, Some(map), |encoder| {
            overlay_painter.encode(&gpu.device, encoder, target, &osc, map, Output::Sdr);
        })
    });
    let at = |x: usize, y: usize| drawn[y * 120 + x];
    let near =
        |pixel: [u8; 3], want: [u8; 3]| pixel.iter().zip(want).all(|(&c, w)| c.abs_diff(w) <= 1);
    // Anchor 0 (device 84.5, 40.5) is amber; anchor 2 is white; a box corner handle is blue.
    assert!(near(at(84, 40), [255, 179, 26]), "{:?}", at(84, 40));
    let (sin, cos) = (std::f64::consts::TAU * 2.0 / 5.0).sin_cos();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let two = ((60.5 + 24.0 * cos) as usize, (40.5 + 24.0 * sin) as usize);
    assert!(
        near(at(two.0, two.1), [255, 255, 255]),
        "{:?}",
        at(two.0, two.1)
    );
    // Anchors 0 and 1 span device x 67.9..84.5, y 40.5..63.3: the box corner 0 is 8 px off.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let corner = (
        (60.5 + 24.0 * (std::f64::consts::TAU / 5.0).cos() - 8.0) as usize,
        32,
    );
    assert!(
        near(at(corner.0, corner.1), [38, 115, 255]),
        "{:?}",
        at(corner.0, corner.1)
    );
    let mut banded = 0;
    for (i, pixel) in drawn.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let point = [(i % 120) as f32 + 0.5, (i / 120) as f32 + 0.5];
        let wanted = cpu_pixel(&osc, map, point, base[i].map(f32::from));
        assert!(
            (0..3).all(|ch| (f32::from(pixel[ch]) - wanted[ch]).abs() < 2.1),
            "{point:?} {pixel:?} {wanted:?}"
        );
        // The band is display only (the Curve and its margin start right of x = 32).
        if point[0] < 31.0 && osc.outline(map, point).is_some() {
            banded += 1;
            assert_eq!(osc.hit(map, point), None, "{point:?}");
        }
    }
    assert!(banded > 60, "{banded}");
}

/// Encode and GPU time per frame of a selected closed 128-anchor Curve with Bezier handles and
/// softness guides, at 1080p and 4K, measured like the 128-point Polygon above.
#[test]
#[ignore = "GPU timing, run explicitly"]
fn window_overlay_128_point_selected_curve_timing() {
    let Some(gpu) = gpu() else { return };
    let overlay_painter = OverlayPainter::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm);
    for size in [(1920_u32, 1080_u32), (3840, 2160)] {
        let map = Viewport::Fit.map(size, size, 0).unwrap();
        let height = f64::from(size.1);
        let mut shape = round_curve(
            "c",
            128,
            0.4,
            true,
            Affine {
                xx: height,
                xy: 0.0,
                yx: 0.0,
                yy: height,
                tx: f64::from(size.0) / 2.0,
                ty: height / 2.0,
            },
        );
        shape.feathers = vec![[0.05, 0.05]];
        let osc = WindowOverlay {
            shapes: vec![shape].into(),
            selected: Some("c".into()),
            style: Style::default(),
            coverage: None,
            points: vec![],
            band: None,
        };
        let built = std::time::Instant::now();
        let primitives = osc.primitives(&osc.shapes[0], map).len();
        let built = built.elapsed().as_secs_f64() * 1000.0;
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let (mut encode, mut total) = (0.0, 0.0);
        for frame in 0..25 {
            let start = std::time::Instant::now();
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            overlay_painter.encode(&gpu.device, &mut encoder, &view, &osc, map, Output::Sdr);
            let commands = encoder.finish();
            let spent = start.elapsed();
            gpu.queue.submit([commands]);
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            if frame >= 5 {
                encode += spent.as_secs_f64() * 1000.0 / 20.0;
                total += start.elapsed().as_secs_f64() * 1000.0 / 20.0;
            }
        }
        println!(
            "TIMING curve {}x{} primitives={primitives} (built in {built:.2} ms): encode {encode:.2} ms, gpu {:.2} ms, encode+gpu {total:.2} ms per frame",
            size.0,
            size.1,
            total - encode
        );
    }
}
