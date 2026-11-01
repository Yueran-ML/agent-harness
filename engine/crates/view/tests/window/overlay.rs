use super::viewport::until_presentation;
use super::*;
use columbina_view::window_overlay::{Affine, Circular, Style, WindowOverlay};
use columbina_view::Viewport;

fn overlay(x: f64) -> Arc<WindowOverlay> {
    Arc::new(WindowOverlay {
        circles: vec![Circular {
            id: "w1".into(),
            radius: 1.0,
            to_canvas: Affine {
                xx: 10.0,
                xy: 0.0,
                yx: 0.0,
                yy: 10.0,
                tx: x,
                ty: 40.5,
            },
        }]
        .into(),
        selected: Some("w1".into()),
        style: Style::default(),
        coverage: None,
    })
}

fn texture(gpu: &Gpu, colour: [u8; 4]) -> Arc<wgpu::Texture> {
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("native overlay test frame"),
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
    gpu.queue().write_texture(
        texture.as_image_copy(),
        &colour.repeat(120 * 80),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(120 * 4),
            rows_per_image: None,
        },
        texture.size(),
    );
    Arc::new(texture)
}

#[test]
#[ignore = "needs a Windows desktop: run with --ignored"]
fn a_single_view_releases_its_last_surface_submission_before_device_replacement() {
    columbina_view::prepare_process();
    let parent = Parent::open(800, 300, 180, 180);
    let first_gpu = gpu();
    let view = sdr_view(
        RawParent::Windows(parent.hwnd),
        Rect {
            x: 20,
            y: 20,
            width: 120,
            height: 120,
        },
        Arc::clone(&first_gpu),
        Arc::new(|event| eprintln!("single view recovery: {event:?}")),
    );
    view.present_texture_tagged(
        texture(&first_gpu, [255, 0, 0, 255]),
        Arc::clone(&first_gpu),
        101,
    )
    .unwrap();
    until_presentation(&view, |p| p.tag == Some(101));
    view.set_viewport(Viewport::Scale {
        scale: 1.0,
        center: [1.0 / 3.0, 0.5],
    })
    .unwrap();
    until_presentation(&view, |p| p.map.is_some_and(|m| m.dest[0] > 0.0));
    parent.until(
        "unadorned frame before replacement",
        &[((120, 80), [255, 0, 0])],
    );
    view.use_gpu(gpu());
    until_presentation(&view, |p| p.map.is_none());
    parent.until(
        "unadorned frame cleared on replacement",
        &[((120, 80), [0; 3])],
    );
}

#[test]
#[ignore = "needs a Windows desktop: run with --ignored; PrintWindow is not real mouse input"]
fn overlay_and_texture_stay_in_the_same_frame_through_pending_redraw_and_device_change() {
    columbina_view::prepare_process();
    let parent = Parent::open(800, 300, 180, 180);
    let first_gpu = gpu();
    let view = sdr_view(
        RawParent::Windows(parent.hwnd),
        Rect {
            x: 20,
            y: 20,
            width: 120,
            height: 120,
        },
        Arc::clone(&first_gpu),
        Arc::new(|event| eprintln!("native overlay: {event:?}")),
    );
    let first = overlay(40.5);
    let first_weak = Arc::downgrade(&first);
    let red = texture(&first_gpu, [255, 0, 0, 255]);
    view.present_texture_overlay(Arc::clone(&red), Arc::clone(&first_gpu), 101, Some(first))
        .unwrap();
    let shown = until_presentation(&view, |p| p.tag == Some(101));
    parent.until(
        "first frame with its own overlay",
        &[((70, 80), [255; 3]), ((110, 80), [255, 0, 0])],
    );
    assert!(
        first_weak.upgrade().is_some(),
        "view retains the submitted Arc"
    );

    view.stall_for(Duration::from_millis(650));
    let second = overlay(80.5);
    let second_weak = Arc::downgrade(&second);
    let green = texture(&first_gpu, [0, 255, 0, 255]);
    view.present_texture_overlay(green, Arc::clone(&first_gpu), 102, Some(second))
        .unwrap();
    assert_eq!(
        view.presentation().unwrap().tag,
        Some(101),
        "pending is not presented"
    );
    let pixels = parent.shown(&[((70, 80), [255; 3]), ((110, 80), [255, 0, 0])]);
    assert!(
        near(pixels[0], [255; 3]) && near(pixels[1], [255, 0, 0]),
        "pending must leave the successful surface alone: {pixels:?}"
    );
    let second = until_presentation(&view, |p| p.serial > shown.serial && p.tag == Some(102));
    parent.until(
        "second frame replaces texture and overlay together",
        &[((70, 80), [0, 255, 0]), ((110, 80), [255; 3])],
    );
    assert!(first_weak.upgrade().is_none());

    view.show(false).unwrap();
    view.show(true).unwrap();
    let exposed = until_presentation(&view, |p| p.serial > second.serial);
    assert_eq!(exposed.tag, Some(102));
    parent.until(
        "expose redraw keeps the same overlay",
        &[((110, 80), [255; 3])],
    );
    view.set_viewport(Viewport::Scale {
        scale: 1.0,
        center: [1.0 / 3.0, 0.5],
    })
    .unwrap();
    let panned = until_presentation(&view, |p| p.serial > exposed.serial);
    assert_eq!(panned.tag, Some(102));
    parent.until(
        "viewport redraw transforms the same overlay",
        &[((120, 80), [0, 255, 0]), ((130, 80), [255; 3])],
    );
    assert!(second_weak.upgrade().is_some());

    let replacement = gpu();
    assert!(view
        .present_texture_overlay(
            Arc::clone(&red),
            Arc::clone(&replacement),
            999,
            Some(overlay(60.5))
        )
        .is_err());
    assert_eq!(
        view.presentation().unwrap().tag,
        Some(102),
        "rejected submission does not publish geometry"
    );
    view.use_gpu(Arc::clone(&replacement));
    until_presentation(&view, |p| p.map.is_none());
    parent.until(
        "device recovery discards old texture and overlay",
        &[((130, 80), [0; 3])],
    );
    assert!(second_weak.upgrade().is_none());
    view.present_texture_tagged(texture(&replacement, [0, 0, 255, 255]), replacement, 103)
        .unwrap();
    until_presentation(&view, |p| p.tag == Some(103));
    parent.until(
        "legacy submission has no resurrected overlay",
        &[((130, 80), [0, 0, 255])],
    );
}
