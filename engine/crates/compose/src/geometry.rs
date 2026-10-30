//! Where a layer is put on the canvas, worked out without the card: the parts of the compositor
//! that are only arithmetic, each tested alone.
//!
//! Sizes and places are in pixels, from the top left corner, x to the right and y down. A layer
//! is first fitted to the canvas ("Scale to Fit", as `DaVinci` has it: as large as it fits, its
//! shape kept, centred), into its *frame*; its transform then moves the frame by shares of the
//! canvas, makes it larger about its middle, and turns it clockwise about its middle.

use columbina_model::project::TextAlign;

/// Where a layer is put, once it is fitted to the canvas: the transform of a clip, without its
/// opacity (which the layer has already, times its fades and transitions).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// How far it is moved right, as a share of the width of the canvas.
    pub x: f64,
    /// How far it is moved down, as a share of the height of the canvas.
    pub y: f64,
    /// How much larger it is made, about its middle.
    pub scale: f64,
    /// How far it is turned clockwise, in degrees.
    pub rotation: f64,
}

impl Transform {
    /// As it is fitted: not moved, not made larger, not turned.
    pub const NONE: Transform = Transform {
        x: 0.0,
        y: 0.0,
        scale: 1.0,
        rotation: 0.0,
    };
}

impl Default for Transform {
    fn default() -> Self {
        Self::NONE
    }
}

impl From<columbina_model::project::Transform> for Transform {
    /// The transform of a clip, whose opacity is left out: [`columbina_model::resolve::Layer`]
    /// has it already, in its own.
    fn from(transform: columbina_model::project::Transform) -> Self {
        Transform {
            x: transform.x,
            y: transform.y,
            scale: transform.scale,
            rotation: transform.rotation,
        }
    }
}

/// The quarter turns clockwise of a rotation in degrees clockwise, as the media has it: 0 to 3,
/// taken modulo 360 (so that -90 is 3). A rotation that is not a multiple of 90 is taken as
/// none (the model has only 0, 90, 180 and 270).
#[must_use]
pub fn quarter_turns(rotation: i16) -> u32 {
    match rotation.rem_euclid(360) {
        90 => 1,
        180 => 2,
        270 => 3,
        _ => 0,
    }
}

/// The size of a picture of `stored` pixels as it is shown, turned `rotation` degrees clockwise:
/// its sides swapped by a quarter turn.
#[must_use]
pub fn shown_size(stored: (u32, u32), rotation: i16) -> (u32, u32) {
    if quarter_turns(rotation) % 2 == 1 {
        (stored.1, stored.0)
    } else {
        stored
    }
}

/// The size of the frame that a layer of `shown` pixels is fitted into on a canvas of `canvas`
/// pixels: as large as it fits, its shape kept. Nothing, where either has no pixels.
#[must_use]
pub fn fit(canvas: (f64, f64), shown: (f64, f64)) -> (f64, f64) {
    if canvas.0 <= 0.0 || canvas.1 <= 0.0 || shown.0 <= 0.0 || shown.1 <= 0.0 {
        return (0.0, 0.0);
    }
    if canvas.0 * shown.1 <= canvas.1 * shown.0 {
        // As wide as the canvas, or of its shape.
        (canvas.0, shown.1 * canvas.0 / shown.0)
    } else {
        (shown.0 * canvas.1 / shown.1, canvas.1)
    }
}

/// An affine map of the plane: (x, y) is put at (a x + b y + tx, c x + d y + ty).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Affine {
    /// Where the point is put.
    #[must_use]
    pub fn apply(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (
            self.a * x + self.b * y + self.tx,
            self.c * x + self.d * y + self.ty,
        )
    }
}

impl Affine {
    /// The map that takes the points back where they were; none where it flattens the plane
    /// (a layer made 0 times larger).
    #[must_use]
    pub fn inverse(&self) -> Option<Affine> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Affine {
            a,
            b,
            c,
            d,
            tx: -(a * self.tx + b * self.ty),
            ty: -(c * self.tx + d * self.ty),
        })
    }
}

/// How many times smaller than its own pixels a picture of `shown` pixels is drawn, fitted into
/// a frame of `frame` pixels of the canvas made `scale` times larger: the most of across and
/// down; 1 or less where it is drawn as large or larger.
#[must_use]
pub fn minification(shown: (u32, u32), frame: (f64, f64), scale: f64) -> f64 {
    let drawn = (frame.0 * scale, frame.1 * scale);
    if drawn.0 <= 0.0 || drawn.1 <= 0.0 {
        return 1.0;
    }
    (f64::from(shown.0) / drawn.0).max(f64::from(shown.1) / drawn.1)
}

/// The most points across and down that a pixel of a layer is made of.
pub const MOST_SAMPLES: u32 = 8;

/// How many points across and down a pixel of a picture drawn `minification` times smaller is
/// made of, weighed as a tent over the pixel and as far again about it: one where it is drawn at
/// least as large as it is; else so many that the points lie at most half a pixel of the picture
/// apart, at most [`MOST_SAMPLES`]. So stripes of a pixel each, drawn three times smaller, are
/// grey, not stripes again.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a count from 2 to 8"
)]
pub fn samples(minification: f64) -> u32 {
    if minification.is_nan() || minification <= 1.0 + 1e-6 {
        return 1;
    }
    (2.0 * minification)
        .ceil()
        .clamp(2.0, f64::from(MOST_SAMPLES)) as u32
}

/// The pixels of a text drawn for a pixel of its frame, for a layer made `scale` times larger:
/// one, down to half as large; below that, as few as the scale needs, rounded up to a quarter
/// of an octave (so that a scale that moves draws its text again only now and then), and never
/// fewer than 1/64.
#[must_use]
pub fn raster_scale(scale: f64) -> f64 {
    if scale.is_nan() || scale >= 0.5 {
        return 1.0;
    }
    let octaves = (4.0 * scale.max(1.0 / 64.0).log2()).ceil() / 4.0;
    2.0_f64.powf(octaves)
}

/// The grid that the part of a text drawn is taken outward to, in its pixels: so that a text
/// that moves a little is not drawn again.
pub const TEXT_GRID: u32 = 64;

/// The part of the pixels of a text that is seen on a canvas of `canvas` pixels, where `place`
/// puts the frame on it: the pixels start at `origin` of the frame, `per` of them to a pixel of
/// the frame, and are `full` of them. As x0, y0, x1, y1 of its pixels, taken outward to
/// [`TEXT_GRID`] and kept within them; none where none is seen.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixels kept within those of the text"
)]
pub fn text_window(
    place: &Affine,
    canvas: (f64, f64),
    origin: (f64, f64),
    per: f64,
    full: (u32, u32),
) -> Option<[u32; 4]> {
    let back = place.inverse()?;
    // The corners of the canvas, a pixel more about it, in pixels of the text.
    let corners = [
        (-1.0, -1.0),
        (canvas.0 + 1.0, -1.0),
        (-1.0, canvas.1 + 1.0),
        (canvas.0 + 1.0, canvas.1 + 1.0),
    ]
    .map(|corner| {
        let (x, y) = back.apply(corner);
        ((x - origin.0) * per, (y - origin.1) * per)
    });
    let low = corners
        .iter()
        .fold((f64::INFINITY, f64::INFINITY), |(x, y), c| {
            (x.min(c.0), y.min(c.1))
        });
    let high = corners
        .iter()
        .fold((f64::NEG_INFINITY, f64::NEG_INFINITY), |(x, y), c| {
            (x.max(c.0), y.max(c.1))
        });
    let grid = f64::from(TEXT_GRID);
    let clip = |value: f64, most: u32| value.clamp(0.0, f64::from(most)) as u32;
    let x0 = clip((low.0 / grid).floor() * grid, full.0);
    let y0 = clip((low.1 / grid).floor() * grid, full.1);
    let x1 = clip((high.0 / grid).ceil() * grid, full.0);
    let y1 = clip((high.1 / grid).ceil() * grid, full.1);
    (x0 < x1 && y0 < y1).then_some([x0, y0, x1, y1])
}

/// How the points of a frame of `frame` pixels, fitted into a canvas of `canvas` pixels, are
/// put on the canvas by `transform`: the middle of the frame goes to the middle of the canvas
/// moved by `x` and `y` shares of its width and height, and the frame is made `scale` times
/// larger and turned `rotation` degrees clockwise about its middle.
#[must_use]
pub fn placement(canvas: (f64, f64), frame: (f64, f64), transform: &Transform) -> Affine {
    let (sin, cos) = transform.rotation.to_radians().sin_cos();
    let scale = transform.scale;
    // With y down, a clockwise turn takes x towards y.
    let (a, b, c, d) = (scale * cos, -scale * sin, scale * sin, scale * cos);
    let middle = (
        canvas.0 / 2.0 + transform.x * canvas.0,
        canvas.1 / 2.0 + transform.y * canvas.1,
    );
    let (half_w, half_h) = (frame.0 / 2.0, frame.1 / 2.0);
    Affine {
        a,
        b,
        c,
        d,
        tx: middle.0 - (a * half_w + b * half_h),
        ty: middle.1 - (c * half_w + d * half_h),
    }
}

/// Where the top left corner of a block of text of `block` pixels is put on a canvas of
/// `canvas` pixels, its anchor at `anchor` (shares of the width and height of the canvas): the
/// anchor is on the side of the block that `align` says across, and in its middle down. In whole
/// pixels, so that the letters are drawn as they were made.
#[must_use]
pub fn title_origin(
    canvas: (u32, u32),
    block: (f64, f64),
    anchor: (f64, f64),
    align: TextAlign,
) -> (f64, f64) {
    let x = anchor.0 * f64::from(canvas.0);
    let y = anchor.1 * f64::from(canvas.1);
    let width = block.0;
    let left = match align {
        TextAlign::Left => x,
        TextAlign::Center => x - width / 2.0,
        TextAlign::Right => x - width,
    };
    (left.round(), (y - block.1 / 2.0).round())
}

/// The height of the letters of subtitles on a canvas `height` pixels high: a twentieth of it.
#[must_use]
pub fn subtitle_size(height: u32) -> f32 {
    #[allow(clippy::cast_precision_loss, reason = "heights of at most 16384")]
    let height = height as f32;
    height / 20.0
}

/// Where the top left corner of the block of subtitles of `block` pixels is put on a canvas of
/// `canvas` pixels: in the middle across, its bottom a twentieth of the height above the bottom
/// of the canvas, in whole pixels.
#[must_use]
pub fn subtitle_origin(canvas: (u32, u32), block: (f64, f64)) -> (f64, f64) {
    let (width, height) = (f64::from(canvas.0), f64::from(canvas.1));
    let x = ((width - block.0) / 2.0).round();
    let y = (height - height / 20.0 - block.1).round();
    (x, y)
}

/// The bytes from a row of a plane of a picture of `width` pixels in `bits` to the next, as
/// [`columbina_media::Picture`] lays them out: a row of chroma (Cb and Cr of half the pixels,
/// rounded up), which is at least a row of luma, rounded up to 64 bytes.
#[must_use]
pub fn picture_stride(width: u32, bits: u8) -> usize {
    let bytes = if bits > 8 { 2 } else { 1 };
    ((width as usize).div_ceil(2) * 2 * bytes).next_multiple_of(64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    #[test]
    fn rotations_are_read_as_quarter_turns() {
        assert_eq!(
            [0, 90, 180, 270, 360, 450, 45, 1000, -90, -180, -450, -1].map(quarter_turns),
            [0, 1, 2, 3, 0, 1, 0, 0, 3, 2, 3, 0]
        );
    }

    #[test]
    fn a_quarter_turn_swaps_the_sides_of_what_is_shown() {
        assert_eq!(shown_size((1920, 1080), 0), (1920, 1080));
        assert_eq!(shown_size((1920, 1080), 90), (1080, 1920));
        assert_eq!(shown_size((1920, 1080), 180), (1920, 1080));
        assert_eq!(shown_size((1920, 1080), -90), (1080, 1920));
        assert_eq!(shown_size((1920, 1080), 45), (1920, 1080));
    }

    #[test]
    fn a_layer_is_fitted_to_the_canvas_its_shape_kept() {
        // Of the shape of the canvas: as large as it.
        assert_eq!(fit((1920.0, 1080.0), (3840.0, 2160.0)), (1920.0, 1080.0));
        assert_eq!(fit((1920.0, 1080.0), (640.0, 360.0)), (1920.0, 1080.0));
        // Narrower: as high as the canvas.
        assert_eq!(fit((1920.0, 1080.0), (1080.0, 1920.0)), (607.5, 1080.0));
        // Wider: as wide.
        assert_eq!(fit((1000.0, 1000.0), (1920.0, 1080.0)), (1000.0, 562.5));
        // Nothing.
        assert_eq!(fit((0.0, 1080.0), (640.0, 360.0)), (0.0, 0.0));
        assert_eq!(fit((1920.0, 1080.0), (640.0, 0.0)), (0.0, 0.0));
        assert_eq!(fit((1920.0, -1.0), (640.0, 360.0)), (0.0, 0.0));
        assert_eq!(fit((1920.0, 1080.0), (-640.0, 360.0)), (0.0, 0.0));
    }

    #[test]
    fn a_frame_not_moved_lies_over_the_middle_of_the_canvas() {
        let place = placement((1920.0, 1080.0), (1440.0, 1080.0), &Transform::NONE);
        assert!(near(place.apply((0.0, 0.0)), (240.0, 0.0)));
        assert!(near(place.apply((1440.0, 1080.0)), (1680.0, 1080.0)));
        assert!(near(place.apply((720.0, 540.0)), (960.0, 540.0)));
    }

    #[test]
    fn a_frame_is_moved_by_shares_of_the_canvas_and_made_larger_about_its_middle() {
        let transform = Transform {
            x: 0.25,
            y: -0.5,
            scale: 0.5,
            rotation: 0.0,
        };
        let place = placement((200.0, 100.0), (200.0, 100.0), &transform);
        // Its middle moved by 50 across and -50 down; its corners half as far from it.
        assert!(near(place.apply((100.0, 50.0)), (150.0, 0.0)));
        assert!(near(place.apply((0.0, 0.0)), (100.0, -25.0)));
        assert!(near(place.apply((200.0, 100.0)), (200.0, 25.0)));
        assert!(near(place.apply((200.0, 0.0)), (200.0, -25.0)));
    }

    #[test]
    fn a_frame_is_turned_clockwise_about_its_middle() {
        let transform = Transform {
            rotation: 90.0,
            ..Transform::NONE
        };
        let place = placement((100.0, 100.0), (100.0, 50.0), &transform);
        // Its middle stays; its top left corner goes to the top right, its top right to the
        // bottom right, as a clockwise quarter turn takes them.
        assert!(near(place.apply((50.0, 25.0)), (50.0, 50.0)));
        assert!(near(place.apply((0.0, 0.0)), (75.0, 0.0)));
        assert!(near(place.apply((100.0, 0.0)), (75.0, 100.0)));
        assert!(near(place.apply((100.0, 50.0)), (25.0, 100.0)));
        let half = Transform {
            rotation: -45.0,
            scale: 2.0,
            ..Transform::NONE
        };
        let place = placement((100.0, 100.0), (100.0, 100.0), &half);
        let corner = place.apply((100.0, 50.0));
        // The middle of its right side, 100 from its middle, turned 45 degrees back: up.
        let side = 100.0 / 2.0_f64.sqrt();
        assert!(near(corner, (50.0 + side, 50.0 - side)), "{corner:?}");
    }

    #[test]
    fn a_transform_is_that_of_a_clip_without_its_opacity() {
        let clip = columbina_model::project::Transform {
            x: 0.1,
            y: 0.2,
            scale: 1.5,
            rotation: 30.0,
            opacity: 0.25,
        };
        assert_eq!(
            Transform::from(clip),
            Transform {
                x: 0.1,
                y: 0.2,
                scale: 1.5,
                rotation: 30.0
            }
        );
        assert_eq!(Transform::default(), Transform::NONE);
    }

    #[test]
    fn a_title_is_put_by_its_anchor_on_the_side_that_its_alignment_says() {
        let canvas = (1920, 1080);
        assert_eq!(
            title_origin(canvas, (400.0, 100.0), (0.5, 0.5), TextAlign::Center),
            (760.0, 490.0)
        );
        assert_eq!(
            title_origin(canvas, (400.0, 100.0), (0.1, 0.9), TextAlign::Left),
            (192.0, 922.0)
        );
        assert_eq!(
            title_origin(canvas, (400.0, 100.0), (0.9, 0.1), TextAlign::Right),
            (1328.0, 58.0)
        );
        // Halves are rounded to whole pixels.
        assert_eq!(
            title_origin((101, 51), (10.0, 11.0), (0.5, 0.5), TextAlign::Center),
            (46.0, 20.0)
        );
    }

    #[test]
    fn subtitles_are_a_twentieth_of_the_height_at_the_bottom_in_the_middle() {
        assert!((subtitle_size(1080) - 54.0).abs() < 1e-6);
        assert!((subtitle_size(2160) - 108.0).abs() < 1e-6);
        assert_eq!(subtitle_origin((1920, 1080), (600.0, 70.0)), (660.0, 956.0));
        assert_eq!(subtitle_origin((1001, 100), (100.0, 10.0)), (451.0, 85.0));
    }

    #[test]
    fn the_planes_of_a_picture_are_laid_out_as_the_media_lays_them() {
        assert_eq!(picture_stride(1920, 8), 1920);
        assert_eq!(picture_stride(1920, 10), 3840);
        assert_eq!(picture_stride(1921, 8), 1984);
        assert_eq!(picture_stride(3, 8), 64);
        assert_eq!(picture_stride(33, 10), 128);
        assert_eq!(picture_stride(32, 10), 64);
    }

    #[test]
    fn a_placement_is_undone_by_its_inverse() {
        let transform = Transform {
            x: 0.1,
            y: -0.2,
            scale: 0.3,
            rotation: 33.0,
        };
        let place = placement((1920.0, 1080.0), (1440.0, 1080.0), &transform);
        let back = place.inverse().unwrap();
        for point in [(0.0, 0.0), (1440.0, 1080.0), (17.0, 900.0)] {
            assert!(near(back.apply(place.apply(point)), point), "{point:?}");
        }
        let flat = Affine {
            a: 1.0,
            b: 2.0,
            c: 2.0,
            d: 4.0,
            tx: 0.0,
            ty: 0.0,
        };
        assert_eq!(flat.inverse(), None);
    }

    #[test]
    fn a_picture_drawn_smaller_is_made_of_more_points_a_pixel() {
        // 4K fitted to 1280 × 720: three times smaller; as large, and larger: not at all.
        let third = minification((3840, 2160), (1280.0, 720.0), 1.0);
        assert!((third - 3.0).abs() < 1e-9);
        assert!((minification((3840, 2160), (1920.0, 1080.0), 0.5) - 4.0).abs() < 1e-9);
        assert!((minification((1920, 1080), (1920.0, 1080.0), 1.0) - 1.0).abs() < 1e-9);
        assert!(minification((640, 360), (1920.0, 1080.0), 1.0) < 1.0);
        // The most of across and down.
        assert!((minification((1000, 100), (500.0, 100.0), 1.0) - 2.0).abs() < 1e-9);
        assert!((minification((10, 10), (0.0, 0.0), 1.0) - 1.0).abs() < 1e-9);
        assert_eq!(
            [0.5, 1.0, 1.01, 1.5, 2.0, 3.0, 3.2, 4.0, 10.0, f64::NAN].map(samples),
            [1, 1, 3, 3, 4, 6, 7, 8, 8, 1]
        );
    }

    #[test]
    fn a_text_is_drawn_at_its_scale_below_a_half_in_quarters_of_an_octave() {
        for scale in [0.5, 1.0, 3.0, f64::NAN] {
            assert!((raster_scale(scale) - 1.0).abs() < 1e-12, "{scale}");
        }
        assert!((raster_scale(0.25) - 0.25).abs() < 1e-12);
        // 0.3 is drawn at 2^(-1.5), the quarter of an octave above it.
        assert!((raster_scale(0.3) - 2.0_f64.powf(-1.5)).abs() < 1e-12);
        assert!(raster_scale(0.3) >= 0.3);
        assert!((raster_scale(0.0) - 1.0 / 64.0).abs() < 1e-12);
    }

    #[test]
    fn only_the_part_of_a_text_on_the_canvas_is_drawn() {
        let canvas = (1920.0, 1080.0);
        let place = placement(canvas, canvas, &Transform::NONE);
        // A text of 10000 × 6000 of its pixels, a pixel each of the frame, from (-4000, -2000):
        // the canvas, a pixel more, taken outward to the grid.
        assert_eq!(
            text_window(&place, canvas, (-4000.0, -2000.0), 1.0, (10000, 6000)),
            Some([3968, 1984, 5952, 3136])
        );
        // Two of its pixels a pixel of the frame.
        assert_eq!(
            text_window(&place, canvas, (-4000.0, -2000.0), 2.0, (20000, 12000)),
            Some([7936, 3968, 11904, 6208])
        );
        // Within the canvas: the whole of it.
        assert_eq!(
            text_window(&place, canvas, (100.0, 100.0), 1.0, (300, 50)),
            Some([0, 0, 300, 50])
        );
        // Beyond it: nothing.
        assert_eq!(
            text_window(&place, canvas, (5000.0, 100.0), 1.0, (300, 50)),
            None
        );
        // Turned, the part that the turned canvas covers.
        let turned = placement(
            canvas,
            canvas,
            &Transform {
                rotation: 90.0,
                ..Transform::NONE
            },
        );
        let window = text_window(&turned, canvas, (0.0, 0.0), 1.0, (100_000, 100_000)).unwrap();
        // 1080 across and 1920 down of the frame, about its middle.
        assert_eq!(window, [384, 0, 1536, 1536]);
    }

    #[test]
    fn a_map_that_keeps_the_least_area_that_is_not_flat_is_undone() {
        let thin = Affine {
            a: 1e-12,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
        };
        let back = thin.inverse().unwrap();
        assert!(near(back.apply(thin.apply((3.0, 4.0))), (3.0, 4.0)));
    }

    #[test]
    fn a_picture_is_drawn_as_many_times_smaller_as_the_more_of_its_sides_is() {
        assert!((minification((1920, 1080), (960.0, 540.0), 1.0) - 2.0).abs() < 1e-12);
        assert!((minification((1920, 1080), (960.0, 540.0), 0.5) - 4.0).abs() < 1e-12);
        assert!((minification((1920, 1080), (960.0, 540.0), 2.0) - 1.0).abs() < 1e-12);
        // Across it is 4 times smaller, and down 2.
        assert!((minification((1920, 1080), (480.0, 540.0), 1.0) - 4.0).abs() < 1e-12);
        assert!((minification((1920, 1080), (960.0, 270.0), 1.0) - 4.0).abs() < 1e-12);
        // A frame of no width or no height is drawn as nothing.
        assert!((minification((1920, 1080), (0.0, 540.0), 1.0) - 1.0).abs() < 1e-12);
        assert!((minification((1920, 1080), (960.0, 0.0), 1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_part_of_a_text_drawn_reaches_a_pixel_beyond_each_corner_of_the_canvas() {
        // Turned a third of a right angle, each corner of the canvas is the furthest in one
        // direction; at 128 pixels of the text to one of the canvas, a pixel more is more than
        // the grid.
        let transform = Transform {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            rotation: 30.0,
        };
        let canvas = (100.0, 100.0);
        let place = placement(canvas, canvas, &transform);
        let (origin, per) = ((-200.0, -200.0), 128.0);
        let window = text_window(&place, canvas, origin, per, (100_000, 100_000)).unwrap();
        let back = place.inverse().unwrap();
        for corner in [(-1.0, -1.0), (101.0, -1.0), (-1.0, 101.0), (101.0, 101.0)] {
            let (x, y) = back.apply(corner);
            let (x, y) = ((x - origin.0) * per, (y - origin.1) * per);
            let [x0, y0, x1, y1] = window.map(f64::from);
            assert!(
                x0 <= x && x <= x1 && y0 <= y && y <= y1,
                "{corner:?} at {x}, {y}"
            );
        }
        // A text far beyond the canvas is not seen.
        for origin in [(-10_000.0, 0.0), (0.0, -10_000.0)] {
            assert_eq!(text_window(&place, canvas, origin, 1.0, (100, 100)), None);
        }
    }
}
