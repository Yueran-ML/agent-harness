//! The picture-to-device map shared by drawing and input. All sizes are physical pixels.

use crate::{letterbox, quarter_turns, Placement, Rect};

/// A viewer setting, independent of the picture and of any project transform.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Viewport {
    #[default]
    Fit,
    /// `center` is the normalized point of the oriented picture at the surface centre;
    /// it may lie outside the picture. `scale` is device pixels per picture pixel.
    Scale { scale: f64, center: [f64; 2] },
}

impl Viewport {
    /// Check that the setting can produce finite shader coordinates for every supported size.
    ///
    /// # Errors
    /// Nonfinite coordinates, a nonpositive scale, or geometry not representable in f32.
    #[allow(clippy::cast_possible_truncation)]
    pub fn checked(self) -> Result<Self, &'static str> {
        if let Self::Scale { scale, center } = self {
            if !scale.is_finite() || scale <= 0.0 || !(scale as f32).is_normal() {
                return Err("viewport scale must be positive and normal in f32");
            }
            if !center.iter().all(|value| value.is_finite()) {
                return Err("viewport center must be finite");
            }
            // Each destination edge is affine in picture and surface size. Their endpoints
            // cover 1..MOST, including a setting made before a picture or window exists.
            for picture in [1, Rect::MOST] {
                for surface in [1, Rect::MOST] {
                    for c in center {
                        axis(surface, picture, scale, c)
                            .ok_or("viewport geometry exceeds f32 coordinates")?;
                    }
                }
            }
        }
        Ok(self)
    }

    /// Lay out an unrotated stored picture, then its metadata quarter-turn, on a surface.
    /// None for empty dimensions, unsupported sizes, or an invalid viewport.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn map(
        self,
        surface: (u32, u32),
        stored: (u32, u32),
        rotation: i16,
    ) -> Option<ViewportMap> {
        self.checked().ok()?;
        if [surface.0, surface.1, stored.0, stored.1]
            .iter()
            .any(|side| *side == 0 || *side > Rect::MOST)
        {
            return None;
        }
        let picture = if quarter_turns(rotation) % 2 == 1 {
            (stored.1, stored.0)
        } else {
            stored
        };
        let (dest, fit) = match self {
            Self::Fit => {
                let fit = letterbox(surface, stored, rotation)?;
                (
                    [
                        fit.x as f32,
                        fit.y as f32,
                        fit.width as f32,
                        fit.height as f32,
                    ],
                    Some(fit),
                )
            }
            Self::Scale { scale, center } => {
                let (x, width) = axis(surface.0, picture.0, scale, center[0])?;
                let (y, height) = axis(surface.1, picture.1, scale, center[1])?;
                ([x, y, width, height], None)
            }
        };
        Some(ViewportMap {
            surface,
            picture,
            dest,
            fit,
        })
    }
}

#[allow(clippy::cast_possible_truncation)]
fn axis(surface: u32, picture: u32, scale: f64, center: f64) -> Option<(f32, f32)> {
    let length = f64::from(picture) * scale;
    let start = (f64::from(surface) * 0.5 - center * length) as f32;
    let length = length as f32;
    // WGSL 15.7.2 permits flushing subnormals to zero; 15.7.4.1 bounds division
    // accuracy only for divisors in [2^-126, 2^126]. These are arithmetic limits.
    let largest_divisor = f32::from_bits(253 << 23); // 2^126
    (start.is_finite()
        && length.is_normal()
        && length > 0.0
        && length <= largest_divisor
        && (start + length).is_finite())
    .then_some((start, length))
}

/// The exact destination sent to the shader, quantized once. Input uses these same values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportMap {
    pub surface: (u32, u32),
    /// Dimensions after metadata rotation. Timeline RGB is already oriented.
    pub picture: (u32, u32),
    /// Left, top, width, height in device pixels; left and top may be negative.
    pub dest: [f32; 4],
    pub(crate) fit: Option<Placement>,
}

impl ViewportMap {
    #[must_use]
    pub fn to_device(self, picture: (f64, f64)) -> (f64, f64) {
        let [x, y, width, height] = self.dest.map(f64::from);
        (
            x + picture.0 * width / f64::from(self.picture.0),
            y + picture.1 * height / f64::from(self.picture.1),
        )
    }

    #[must_use]
    pub fn to_picture(self, device: (f64, f64)) -> (f64, f64) {
        let [x, y, width, height] = self.dest.map(f64::from);
        (
            (device.0 - x) * f64::from(self.picture.0) / width,
            (device.1 - y) * f64::from(self.picture.1) / height,
        )
    }

    /// Read a native pointer at its pixel centre, also checking the visible surface bounds.
    #[must_use]
    pub fn on_picture(self, point: (i32, i32)) -> ((f64, f64), bool) {
        let device = (f64::from(point.0) + 0.5, f64::from(point.1) + 0.5);
        let picture = self.to_picture(device);
        let inside = (0.0..f64::from(self.surface.0)).contains(&device.0)
            && (0.0..f64::from(self.surface.1)).contains(&device.1)
            && (0.0..f64::from(self.picture.0)).contains(&picture.0)
            && (0.0..f64::from(self.picture.1)).contains(&picture.1);
        (picture, inside)
    }
}

/// One successful presentation. Redrawing a frame changes `serial`, but keeps its producer tag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Presentation {
    pub serial: u64,
    /// An opaque producer identity, never interpreted by the view.
    pub tag: Option<u64>,
    /// None when black was presented without a picture.
    pub map: Option<ViewportMap>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_the_integer_layout_and_oriented_picture_coordinates() {
        for surface in [(960, 600), (33, 17), (99, 177)] {
            for picture in [(1920, 1080), (17, 33), (1, 1)] {
                for turn in [0, 90, 180, 270, -90, 17] {
                    let map = Viewport::Fit.map(surface, picture, turn).unwrap();
                    assert_eq!(map.fit, letterbox(surface, picture, turn));
                    assert_eq!(
                        map.picture,
                        if quarter_turns(turn) % 2 == 1 {
                            (picture.1, picture.0)
                        } else {
                            picture
                        }
                    );
                    let (at, inside) = map.on_picture((0, 0));
                    assert_eq!(
                        Some((at, inside)),
                        crate::pointer::on_picture(surface, map.picture, (0, 0))
                    );
                }
            }
        }
        assert!(Viewport::Fit.map((0, 10), (10, 10), 0).is_none());
    }

    #[test]
    fn zoom_and_pan_share_one_inverse_and_clip_to_the_surface() {
        let map = Viewport::Scale {
            scale: 2.0,
            center: [0.5, 0.5],
        }
        .map((100, 60), (100, 80), 90)
        .unwrap();
        assert_eq!(map.picture, (80, 100));
        assert_eq!(map.dest, [-30.0, -70.0, 160.0, 200.0]);
        assert_eq!(map.on_picture((0, 0)), ((15.25, 35.25), true));
        assert_eq!(map.on_picture((-1, 0)), ((14.75, 35.25), false));
        for scale in [0.333, 1.0, 2.75] {
            let map = Viewport::Scale {
                scale,
                center: [-0.25, 1.25],
            }
            .map((107, 63), (101, 79), 0)
            .unwrap();
            for point in [(0.0, 0.0), (50.5, 39.5), (-100.0, 400.0)] {
                let got = map.to_picture(map.to_device(point));
                assert!((got.0 - point.0).abs() < 1e-10 && (got.1 - point.1).abs() < 1e-10);
            }
        }
    }

    #[test]
    fn settings_cannot_overflow_or_become_a_zero_shader_denominator_on_later_pictures() {
        for scale in [
            0.0,
            -1.0,
            f64::NAN,
            f64::INFINITY,
            1e-100,
            1e-40,
            1e34,
            1e35,
        ] {
            assert!(
                Viewport::Scale {
                    scale,
                    center: [0.5, 0.5]
                }
                .checked()
                .is_err(),
                "{scale}"
            );
        }
        for center in [f64::NAN, f64::INFINITY, 1e40, -1e40] {
            assert!(Viewport::Scale {
                scale: 1.0,
                center: [center, 0.5]
            }
            .checked()
            .is_err());
        }
        assert!(Viewport::Scale {
            scale: 0.001,
            center: [-100.0, 100.0]
        }
        .checked()
        .is_ok());
    }
}
