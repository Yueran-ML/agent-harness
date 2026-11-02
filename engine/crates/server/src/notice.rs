//! What the timeline monitor writes over its picture, for the view alone: where a LUT that the
//! grades of the timeline name cannot be read, "an overlay at the bottom right of the screen
//! indicating the name of the LUT if a single LUT is missing, or an indicator that multiple LUTs
//! are missing" (Reference Manual 20, p. 3413).
//!
//! It is written into a copy of the frame that the view is handed, after the Viewer LUT: the
//! scopes, a still grabbed, `view.sample` and an export see the picture without it. The page of
//! the desktop cannot draw over the native view, so the engine does.
//!
//! The words are `DaVinci Resolve`'s, in English, as the picture of its viewer has them. They are
//! drawn as subtitles are, white letters outlined in black, on a black plate as wide as they are,
//! [`SIZE`] of the height of the frame, [`MARGIN`] of it from its lower right corner. The white is
//! that of SDR, and of HDR a white that is not glaring ([`white`]). Nothing is mixed with the
//! picture under the plate: the plate is written as it is, so that no frame is read back.

use std::sync::Arc;

use columbina_compose::{overlay, Gpu, Target};

use crate::grading::MissingLut;
use crate::managed::Output;

/// How high the letters are, a share of the height of the frame.
pub const SIZE: f32 = 0.028;
/// How far the plate lies from the right and the bottom of the frame, a share of its height.
pub const MARGIN: f32 = 0.025;
/// The least height of the letters, in pixels of the frame.
const LEAST: f32 = 10.0;

/// The words of the overlay for the LUTs that cannot be read; none where all can.
#[must_use]
pub fn words(missing: &[MissingLut]) -> Option<String> {
    match missing {
        [] => None,
        [one] => Some(format!("Missing LUT: {}", one.name)),
        _ => Some("Multiple LUTs missing".to_owned()),
    }
}

/// The level of the white of the letters in the signal of the output: 0.9 of SDR; of PQ, 203
/// nits (BT.2408's reference white, 0.58 of the signal); of HLG, 0.75 (the same white).
#[must_use]
pub fn white(output: Option<&Output>) -> f32 {
    match output.and_then(|output| output.hdr) {
        Some(hdr) if hdr.pq => 0.58,
        Some(_) => 0.75,
        None => 0.9,
    }
}

/// Where the plate of a label `width` × `height` lies on a frame of `frame`: its left and top
/// corner, or none where it does not fit.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "pixels of a frame"
)]
pub fn place(frame: (u32, u32), width: u32, height: u32) -> Option<(u32, u32)> {
    let margin = (frame.1 as f32 * MARGIN).round() as u32;
    let x = frame.0.checked_sub(width)?.checked_sub(margin)?;
    let y = frame.1.checked_sub(height)?.checked_sub(margin)?;
    Some((x, y))
}

/// The pixels of the plate: the label's letters, premultiplied R'G'B'A of 8 bits, over black,
/// their white `white`, as half floats R, G, B, A (opaque), row after row.
#[must_use]
pub fn plate(pixels: &[[u8; 4]], white: f32) -> Vec<u16> {
    let mut out = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        for channel in &pixel[..3] {
            out.push(crate::gallery::f32_to_half(
                f32::from(*channel) / 255.0 * white,
            ));
        }
        out.push(crate::gallery::f32_to_half(1.0));
    }
    out
}

/// `shown` with `words` written over it at its lower right, into `into` (a target of its size,
/// on its card): the frame copied, then the plate written.
///
/// # Errors
/// The words do not fit on the frame, or the targets are not of a size on the card.
#[allow(
    clippy::cast_precision_loss,
    reason = "the height of a frame, for the size of letters"
)]
pub fn write(
    gpu: &Arc<Gpu>,
    shown: &Target,
    into: &Target,
    words: &str,
    white: f32,
) -> Result<(), String> {
    let frame = shown.size();
    let size = (frame.1 as f32 * SIZE).max(LEAST);
    let (width, height, pixels) = overlay::label(words, size);
    let at = place(frame, width, height)
        .filter(|_| width > 0 && height > 0)
        .ok_or_else(|| {
            format!(
                "{words:?} does not fit on a frame of {}×{}",
                frame.0, frame.1
            )
        })?;
    overlay::copy_with(
        gpu,
        shown,
        into,
        at,
        (width, height),
        &plate(&pixels, white),
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing(name: &str) -> MissingLut {
        MissingLut {
            name: name.to_owned(),
            message: "cannot be read".to_owned(),
        }
    }

    #[test]
    fn the_words_name_one_lut_or_say_that_several_are_missing() {
        assert_eq!(words(&[]), None);
        assert_eq!(
            words(&[missing("looks/day.cube")]).as_deref(),
            Some("Missing LUT: looks/day.cube")
        );
        assert_eq!(
            words(&[missing("a.cube"), missing("b.cube")]).as_deref(),
            Some("Multiple LUTs missing")
        );
    }

    #[test]
    fn the_plate_lies_at_the_lower_right_within_the_frame_and_its_white_is_not_glaring() {
        // 2.5% of 1080 is 27 pixels from the right and the bottom.
        assert_eq!(
            place((1920, 1080), 300, 40),
            Some((1920 - 300 - 27, 1080 - 40 - 27))
        );
        assert_eq!(place((100, 50), 300, 40), None);
        assert!((white(None) - 0.9).abs() < 1e-6);
        let half = |value: f32| crate::gallery::f32_to_half(value);
        // A white pixel of the letters, an outline, and what is clear: black, opaque.
        assert_eq!(
            plate(&[[255, 255, 255, 255], [0, 0, 0, 255], [0, 0, 0, 0]], 0.5),
            vec![
                half(0.5),
                half(0.5),
                half(0.5),
                half(1.0),
                0,
                0,
                0,
                half(1.0),
                0,
                0,
                0,
                half(1.0)
            ]
        );
    }

    #[test]
    fn the_words_are_written_over_a_copy_of_the_frame_at_its_lower_right() {
        let Some(gpu) = columbina_compose::gpu_for_tests() else {
            return;
        };
        let (width, height) = (640_u32, 360_u32);
        let shown = Target::preview(&gpu, width, height).unwrap();
        // A grey frame, as the compositor draws it.
        let scene = columbina_compose::Scene {
            width,
            height,
            layers: vec![columbina_compose::SceneLayer {
                source: columbina_compose::Source::Color([0.25, 0.25, 0.25, 1.0]),
                transform: columbina_compose::Transform::NONE,
                opacity: 1.0,
                color: columbina_compose::LayerColor::default(),
            }],
            ..columbina_compose::Scene::default()
        };
        columbina_compose::Compositor::new(Arc::clone(&gpu))
            .render(&scene, &shown)
            .unwrap();
        let into = Target::preview(&gpu, width, height).unwrap();
        write(&gpu, &shown, &into, "Missing LUT: day.cube", 0.9).unwrap();
        let read = into.read_floats().unwrap();
        let at = |x: u32, y: u32| {
            let i = ((y * width + x) * 4) as usize;
            [read[i], read[i + 1], read[i + 2]]
        };
        // The upper left is the frame as it was; so is the frame shown.
        assert!(at(10, 10).iter().all(|v| (v - 0.25).abs() < 1e-3));
        assert!(shown
            .read_floats()
            .unwrap()
            .chunks(4)
            .all(|p| (p[0] - 0.25).abs() < 1e-3));
        // The lower right: letters of the white given, on black.
        let corner: Vec<[f32; 3]> = (height * 3 / 4..height)
            .flat_map(|y| (width / 2..width).map(move |x| (x, y)))
            .map(|(x, y)| at(x, y))
            .collect();
        assert!(corner
            .iter()
            .any(|p| p.iter().all(|v| (v - 0.9).abs() < 1e-2)));
        assert!(corner.iter().any(|p| p.iter().all(|v| v.abs() < 1e-3)));
        // Words too long for the frame are refused, and nothing is written.
        assert!(write(&gpu, &shown, &into, &"x".repeat(400), 0.9).is_err());
    }
}
