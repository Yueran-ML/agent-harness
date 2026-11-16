//! The filters of the neighbourhoods that operations of colour read (`color.rs`): how the
//! compositor takes them on the card, in two passes of `neighbourhood.wgsl`, and the same on the
//! processor, the reference that the card is held to.
//!
//! A filter is separable: a pass across, from the picture into a texture as high as it, and a
//! pass down, from that into the texture that the operations read. Each pixel of a pass is the
//! weighed mean of the pixels of its row (or column) about its centre: by a Gaussian of the
//! filter's standard deviation, out to three of them, or equally within the filter's radius;
//! the pixels beyond the edges are those at the edges. A filter wide enough loses nothing by
//! being taken at fewer points: a pass then makes a pixel of [`reduction`] pixels of its source,
//! each the mean about its centre as wide as ever, so that the texture read is smaller by as many
//! across or down, and the operations read it between its texels. A Gaussian is taken so from a
//! standard deviation of 4 pixels of the picture, at a pixel for each half of it at most
//! (2, 4, 8, … to 64), and a box from a radius of 8, at a pixel for each quarter of it: a
//! Gaussian of 20 pixels of a picture of 4K is so taken at a pixel of 8, in a texture of 480 ×
//! 270, and costs a pass across that reads some 15 pixels for each of the picture, and little
//! more.
//!
//! The textures are of half floats, and the card writes into them as it may (to the nearest, or
//! towards 0); the processor rounds to the nearest.
//!
//! The Blur node of the composition of a clip is blurred by these filters too, by the same two
//! passes and held to the same reference (`fusion.rs`): its picture is the one filtered, of
//! all four channels, premultiplied, and what is read of the texture between its texels
//! ([`sample`]) is mixed with the picture as it came in.

use crate::color::{Filter, REFERENCE_HEIGHT};

/// The least standard deviation of a Gaussian, in pixels of its picture, that is taken at fewer
/// points than the pixels.
const GAUSSIAN_SPARSE: f32 = 4.0;

/// The least radius of a box, in pixels of its picture, that is taken at fewer points.
const BOX_SPARSE: f32 = 8.0;

/// The most pixels of a picture that make a pixel of a filtered texture, along one axis.
const MOST_REDUCTION: u32 = 64;

/// The least standard deviation of a Gaussian, in pixels of its picture: below it, a pixel is
/// taken as it is.
const LEAST_SIGMA: f32 = 1e-3;

/// How many pixels of a picture make a pixel of its filtered texture along an axis, for a
/// Gaussian (`gaussian`) of a standard deviation of `reach` pixels of the picture along it, or a
/// box of that radius: 1 below 4 pixels of a Gaussian and 8 of a box, and then the power of 2
/// within a half of the standard deviation, or a quarter of the radius.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "a power of 2, up to 64")]
pub fn reduction(gaussian: bool, reach: f32) -> u32 {
    let (least, per) = if gaussian {
        (GAUSSIAN_SPARSE, 2.0)
    } else {
        (BOX_SPARSE, 4.0)
    };
    if reach.is_nan() || reach < least {
        return 1;
    }
    let mut by = 1;
    while by < MOST_REDUCTION && (by * 2) as f32 <= reach / per {
        by *= 2;
    }
    by
}

/// A pass of a filter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pass {
    /// Along rows (across), or columns (down).
    pub across: bool,
    pub gaussian: bool,
    /// The standard deviation of the Gaussian or the radius of the box, in pixels of the source
    /// along the pass.
    pub reach: f32,
    /// How many pixels each side of the centre are read.
    pub taps: f32,
    /// The size of the source, and of the target.
    pub source: (u32, u32),
    pub target: (u32, u32),
}

impl Pass {
    /// The pixels of the source to a pixel of the target, along the pass.
    fn ratio(&self) -> f32 {
        #[allow(clippy::cast_precision_loss, reason = "sizes of textures")]
        if self.across {
            self.source.0 as f32 / self.target.0 as f32
        } else {
            self.source.1 as f32 / self.target.1 as f32
        }
    }

    /// The uniform of `neighbourhood.wgsl`.
    #[allow(clippy::cast_precision_loss, reason = "sizes of textures")]
    pub fn uniform(&self) -> [f32; 8] {
        [
            if self.across { 1.0 } else { 0.0 },
            if self.gaussian { 0.0 } else { 1.0 },
            self.reach,
            self.taps,
            self.source.0 as f32,
            self.source.1 as f32,
            self.ratio(),
            0.0,
        ]
    }
}

/// The two passes of `filter` over a picture of `size`, `units` of its pixels to a pixel of a
/// frame [`REFERENCE_HEIGHT`] high: across, into a texture as high as the picture, and down,
/// into the texture that is read, of the size that the second gives.
pub(crate) fn passes(filter: Filter, units: f32, size: (u32, u32)) -> [Pass; 2] {
    let (gaussian, [across, down]) = match filter {
        Filter::Gaussian { sigma } => (true, sigma),
        Filter::Box { radius } => (false, radius),
    };
    let reach = |value: f32| {
        let value = if value.is_finite() {
            value.max(0.0) * units
        } else {
            0.0
        };
        if gaussian {
            value.max(LEAST_SIGMA)
        } else {
            value
        }
    };
    let taps = |reach: f32| {
        if gaussian {
            (3.0 * reach).ceil()
        } else {
            reach.ceil()
        }
    };
    let (across, down) = (reach(across), reach(down));
    let target = (
        size.0.div_ceil(reduction(gaussian, across)),
        size.1.div_ceil(reduction(gaussian, down)),
    );
    let first = Pass {
        across: true,
        gaussian,
        reach: across,
        taps: taps(across),
        source: size,
        target: (target.0, size.1),
    };
    let second = Pass {
        across: false,
        gaussian,
        reach: down,
        taps: taps(down),
        source: first.target,
        target,
    };
    [first, second]
}

/// How many pixels of a picture of a layer make a pixel of a frame [`REFERENCE_HEIGHT`] high,
/// where `pixels` of it make `fitted` pixels of the canvas, and the canvas is `canvas` high.
#[must_use]
pub fn units(pixels: f64, fitted: f64, canvas: f64) -> f32 {
    if fitted <= 0.0 || canvas <= 0.0 {
        return 0.0;
    }
    #[allow(clippy::cast_possible_truncation, reason = "a ratio of sizes")]
    let units = (pixels / fitted * canvas / f64::from(REFERENCE_HEIGHT)) as f32;
    units
}

/// A picture of floats on the processor: R, G, B and A of each pixel, row after row.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 4]>,
}

impl Image {
    fn at(&self, x: u32, y: u32) -> [f32; 4] {
        self.pixels[y as usize * self.width as usize + x as usize]
    }
}

/// `image` through `filter`, `units` of its pixels to a pixel of a frame [`REFERENCE_HEIGHT`]
/// high: the texture that the card makes of it, and that operations read with [`sample`].
#[must_use]
pub fn filtered(image: &Image, filter: Filter, units: f32) -> Image {
    let [first, second] = passes(filter, units, (image.width, image.height));
    run(&second, &run(&first, image))
}

/// A pass over `source`, as `neighbourhood.wgsl` does it, rounded to half floats.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "places in textures, as the shader takes them"
)]
fn run(pass: &Pass, source: &Image) -> Image {
    let ratio = pass.ratio();
    let (width, height) = pass.target;
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            let (centre, row, last) = if pass.across {
                ((x as f32 + 0.5) * ratio, y, source.width as f32 - 1.0)
            } else {
                ((y as f32 + 0.5) * ratio, x, source.height as f32 - 1.0)
            };
            let first = (centre - 0.5).floor() - pass.taps;
            let count = (2.0 * pass.taps + 2.0) as u32;
            let mut sum = [0.0_f32; 4];
            let mut weights = 0.0_f32;
            for k in 0..count {
                let index = first + k as f32;
                let d = index + 0.5 - centre;
                let weight = if pass.gaussian {
                    (-0.5 * d * d / (pass.reach * pass.reach)).exp()
                } else if d.abs() <= pass.reach {
                    1.0
                } else {
                    0.0
                };
                let at = index.clamp(0.0, last) as u32;
                let texel = if pass.across {
                    source.at(at, row)
                } else {
                    source.at(row, at)
                };
                for (total, value) in sum.iter_mut().zip(texel) {
                    *total += value * weight;
                }
                weights += weight;
            }
            pixels.push(sum.map(|total| half(total / weights)));
        }
    }
    Image {
        width,
        height,
        pixels,
    }
}

/// A texture of a filter at `at` (from 0 to 1 across and down), between its four texels, as an
/// operation reads it (`color.rs`): premultiplied, as it is kept.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "places in textures, as the shader takes them"
)]
pub fn sample(image: &Image, at: [f32; 2]) -> [f32; 4] {
    let size = [image.width as f32, image.height as f32];
    let place = [at[0] * size[0] - 0.5, at[1] * size[1] - 0.5];
    let base = place.map(f32::floor);
    let blend = [place[0] - base[0], place[1] - base[1]];
    let low = [0, 1].map(|i| base[i].clamp(0.0, size[i] - 1.0) as u32);
    let high = [0, 1].map(|i| (base[i] + 1.0).clamp(0.0, size[i] - 1.0) as u32);
    let mix = |a: [f32; 4], b: [f32; 4], t: f32| -> [f32; 4] {
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
    };
    let top = mix(
        image.at(low[0], low[1]),
        image.at(high[0], low[1]),
        blend[0],
    );
    let bottom = mix(
        image.at(low[0], high[1]),
        image.at(high[0], high[1]),
        blend[0],
    );
    mix(top, bottom, blend[1])
}

/// The colour that an operation reads of a texture of a filter at `at`: [`sample`],
/// unpremultiplied (black where nothing is seen).
#[must_use]
pub fn colour_at(image: &Image, at: [f32; 2]) -> [f32; 3] {
    let [r, g, b, a] = sample(image, at);
    if a <= 0.0 {
        return [0.0; 3];
    }
    [r / a, g / a, b / a]
}

/// `value` as a half float keeps it: to the nearest, half way to the even one.
#[must_use]
pub fn half(value: f32) -> f32 {
    if !value.is_finite() {
        return value;
    }
    let bits = value.to_bits();
    let sign = bits & 0x8000_0000;
    let magnitude = bits & 0x7FFF_FFFF;
    let size = f32::from_bits(magnitude);
    if size >= 65520.0 {
        return f32::from_bits(sign | 0x7F80_0000);
    }
    if size < 6.103_515_6e-5 {
        // Below the least normal half float, its steps are of 2^-24.
        let steps = (size * 16_777_216.0).round_ties_even() / 16_777_216.0;
        return f32::from_bits(sign | steps.to_bits());
    }
    // Ten bits of the fraction kept of 23.
    let odd = (magnitude >> 13) & 1;
    f32::from_bits(sign | ((magnitude + 0x0FFF + odd) & !0x1FFF))
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss, reason = "places of pixels")]
mod tests {
    use super::*;

    #[test]
    fn half_floats_keep_ten_bits_rounded_to_the_nearest_even() {
        assert_eq!(half(1.0), 1.0);
        assert_eq!(half(0.1), 0.099_975_586);
        assert_eq!(half(-0.1), -0.099_975_586);
        // Half way between 1 and the next half float, 1 + 2^-10: to the even, 1.
        assert_eq!(half(1.0 + 2.0_f32.powi(-11)), 1.0);
        assert_eq!(
            half(1.0 + 3.0 * 2.0_f32.powi(-11)),
            1.0 + 2.0 * 2.0_f32.powi(-10)
        );
        assert_eq!(half(65504.0), 65504.0);
        assert_eq!(half(70000.0), f32::INFINITY);
        assert_eq!(half(2.0_f32.powi(-24)), 2.0_f32.powi(-24));
        assert_eq!(half(2.0_f32.powi(-26)), 0.0);
    }

    #[test]
    fn a_filter_is_taken_at_fewer_points_once_it_is_wide() {
        assert_eq!(reduction(true, 3.9), 1);
        assert_eq!(reduction(true, 4.0), 2);
        assert_eq!(reduction(true, 7.9), 2);
        assert_eq!(reduction(true, 8.0), 4);
        assert_eq!(reduction(true, 20.0), 8);
        assert_eq!(reduction(true, 1e6), MOST_REDUCTION);
        assert_eq!(reduction(false, 7.0), 1);
        assert_eq!(reduction(false, 8.0), 2);
        assert_eq!(reduction(false, 40.0), 8);
        assert_eq!(reduction(true, f32::NAN), 1);
        // 20 pixels of a Gaussian over 4K: 480 × 270, read across 2 · 60 + 2 pixels apart.
        let [first, second] = passes(Filter::Gaussian { sigma: [10.0; 2] }, 2.0, (3840, 2160));
        assert_eq!(first.target, (480, 2160));
        assert_eq!(second.source, (480, 2160));
        assert_eq!(second.target, (480, 270));
        assert_eq!(first.taps, 60.0);
        assert_eq!(
            first.uniform(),
            [1.0, 0.0, 20.0, 60.0, 3840.0, 2160.0, 8.0, 0.0]
        );
        assert_eq!(second.uniform()[..2], [0.0, 0.0]);
        // A Gaussian of nothing is the picture as it is.
        let [first, _] = passes(Filter::Gaussian { sigma: [0.0; 2] }, 1.0, (8, 8));
        assert_eq!((first.reach, first.taps), (LEAST_SIGMA, 1.0));
    }

    fn picture(width: u32, height: u32, f: impl Fn(u32, u32) -> f32) -> Image {
        Image {
            width,
            height,
            pixels: (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let v = f(x, y);
                    [v, v * 0.5, 1.0 - v, 1.0].map(half)
                })
                .collect(),
        }
    }

    #[test]
    fn a_filter_of_nothing_keeps_the_picture_and_a_flat_one_stays_flat() {
        let image = picture(13, 7, |x, y| ((x * 7 + y * 3) % 11) as f32 / 10.0);
        let kept = filtered(&image, Filter::Gaussian { sigma: [0.0; 2] }, 1.0);
        assert_eq!(kept, image);
        let kept = filtered(&image, Filter::Box { radius: [0.4; 2] }, 1.0);
        assert_eq!(kept, image);
        for (x, y) in [(0, 0), (5, 3), (12, 6)] {
            let at = [(x as f32 + 0.5) / 13.0, (y as f32 + 0.5) / 7.0];
            assert_eq!(sample(&kept, at), image.at(x, y));
        }
        let flat = picture(40, 30, |_, _| 0.375);
        for filter in [
            Filter::Gaussian { sigma: [9.0, 3.0] },
            Filter::Box {
                radius: [12.0, 2.0],
            },
        ] {
            let out = filtered(&flat, filter, 1.0);
            assert!(out.width < 40, "{filter:?}: {}", out.width);
            for pixel in &out.pixels {
                assert!((pixel[0] - 0.375).abs() < 1e-3, "{pixel:?}");
                assert!((pixel[3] - 1.0).abs() < 1e-3, "{pixel:?}");
            }
        }
    }

    #[test]
    fn a_gaussian_spreads_a_point_as_far_as_its_deviation_and_a_box_as_its_radius() {
        // A line of light down the middle of a dark picture, 1 pixel wide.
        let line = picture(41, 3, |x, _| if x == 20 { 1.0 } else { 0.0 });
        let row = |image: &Image| -> Vec<f32> { (0..41).map(|x| image.at(x, 1)[0]).collect() };
        let blurred = row(&filtered(
            &line,
            Filter::Gaussian { sigma: [2.0, 0.0] },
            1.0,
        ));
        let total: f32 = blurred.iter().sum();
        assert!((total - 1.0).abs() < 2e-3, "{total}");
        let peak = 1.0 / (2.0 * std::f32::consts::PI).sqrt() / 2.0;
        assert!((blurred[20] - peak).abs() < 1e-3, "{}", blurred[20]);
        assert!((blurred[22] / blurred[20] - (-0.5_f32).exp()).abs() < 2e-3);
        assert!(blurred[27] < 1e-3);
        let boxed = row(&filtered(&line, Filter::Box { radius: [2.0, 0.0] }, 1.0));
        for (x, value) in boxed.iter().enumerate() {
            let want = if (18..=22).contains(&x) { 0.2 } else { 0.0 };
            assert!((value - want).abs() < 1e-3, "{x}: {value}");
        }
        // Units: 2 pixels of the picture to a pixel of the reference frame.
        let wide = row(&filtered(
            &line,
            Filter::Gaussian { sigma: [1.0, 0.0] },
            2.0,
        ));
        assert_eq!(wide, blurred);
    }

    #[test]
    fn a_wide_filter_taken_at_fewer_points_is_close_to_the_whole() {
        // A Gaussian of 12 pixels, taken at a pixel of 4, and read between its texels, against
        // the same taken at every pixel.
        let image = picture(96, 64, |x, y| {
            let (x, y) = (x as f32, y as f32);
            0.5 + 0.3 * (x / 7.0).sin() * (y / 5.0).cos() + if x > 50.0 { 0.2 } else { 0.0 }
        });
        let sparse = filtered(&image, Filter::Gaussian { sigma: [12.0; 2] }, 1.0);
        assert_eq!((sparse.width, sparse.height), (24, 16));
        let whole = run(
            &Pass {
                across: false,
                gaussian: true,
                reach: 12.0,
                taps: 36.0,
                source: (96, 64),
                target: (96, 64),
            },
            &run(
                &Pass {
                    across: true,
                    gaussian: true,
                    reach: 12.0,
                    taps: 36.0,
                    source: (96, 64),
                    target: (96, 64),
                },
                &image,
            ),
        );
        // Within, the two are close; at the edges, the half of a texel of the smaller beyond
        // its last centre is held flat, a little off where the picture slopes.
        let (mut within, mut edges) = (0.0_f32, 0.0_f32);
        for y in 0..64 {
            for x in 0..96 {
                let at = [(x as f32 + 0.5) / 96.0, (y as f32 + 0.5) / 64.0];
                let off = (sample(&sparse, at)[0] - whole.at(x, y)[0]).abs();
                if (4..92).contains(&x) && (4..60).contains(&y) {
                    within = within.max(off);
                } else {
                    edges = edges.max(off);
                }
            }
        }
        assert!(within < 4e-3, "{within}");
        assert!(edges < 0.02, "{edges}");
    }

    #[test]
    fn a_pass_alone_is_the_weighed_mean_of_its_row() {
        // A box of radius 1 across: each pixel the mean of three, the edges held.
        let row = picture(5, 1, |x, _| x as f32 / 4.0);
        let across = run(
            &Pass {
                across: true,
                gaussian: false,
                reach: 1.0,
                taps: 1.0,
                source: (5, 1),
                target: (5, 1),
            },
            &row,
        );
        let red: Vec<f32> = (0..5).map(|x| across.at(x, 0)[0]).collect();
        let want = [1.0 / 12.0, 0.25, 0.5, 0.75, 11.0 / 12.0].map(half);
        for (got, want) in red.iter().zip(want) {
            assert!((got - want).abs() < 1e-3, "{red:?}");
        }
    }

    #[test]
    fn a_texture_is_read_unpremultiplied_and_held_at_its_edges() {
        let mut image = picture(4, 2, |x, _| x as f32 / 4.0);
        for pixel in &mut image.pixels {
            // Half seen: premultiplied by 0.5.
            *pixel = [pixel[0] * 0.5, pixel[1] * 0.5, pixel[2] * 0.5, 0.5];
        }
        let at = [(2.0 + 0.5) / 4.0, 0.25];
        let [r, g, b] = colour_at(&image, at);
        assert!((r - 0.5).abs() < 1e-3 && (g - 0.25).abs() < 1e-3 && (b - 0.5).abs() < 1e-3);
        // Beyond the right and bottom edges, the texels at them.
        assert_eq!(sample(&image, [1.25, 1.5]), image.at(3, 1));
        assert_eq!(sample(&image, [-0.5, -0.5]), image.at(0, 0));
    }

    #[test]
    fn units_are_pixels_of_the_picture_to_a_pixel_of_a_timeline_1080_high() {
        // A picture of 4K fitted into a canvas of 1080: 2.
        assert_eq!(units(2160.0, 1080.0, 1080.0), 2.0);
        // The same in a preview of half the size: still 2.
        assert_eq!(units(2160.0, 540.0, 540.0), 2.0);
        // A picture of 1080 in a timeline of 4K.
        assert_eq!(units(1080.0, 2160.0, 2160.0), 1.0);
        // A wide picture letterboxed: its height fills a part of the canvas.
        assert_eq!(units(800.0, 540.0, 1080.0), 800.0 / 540.0);
        assert_eq!(units(1.0, 0.0, 1080.0), 0.0);
    }
}
