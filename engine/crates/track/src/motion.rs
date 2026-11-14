//! The motion of a window as the eight numbers that the tracker graph draws.

use serde::{Deserialize, Serialize};

use crate::linalg::{Mat3, Point};

/// The motion of a window from where it was placed, as the components of the tracker graph
/// (Reference Manual of Resolve 20, pp. 3235–3236: Pan, Tilt, Zoom, Rotate, Perspective 3D).
///
/// It stands for the homography, about the window's anchor `c`,
///
/// ```text
/// H = T(c) · T(pan, tilt) · R(rotate) · S(zoom) · K(aspect, shear) · P(yaw, pitch) · T(−c)
/// ```
///
/// with `R` a rotation, `S` a uniform scale, `K = [[aspect, shear], [0, 1/aspect]]` and
/// `P = [[1, 0, 0], [0, 1, 0], [yaw, pitch, 1]]`. This is the decomposition of Hartley and
/// Zisserman (§2.4.6: similarity · affine · projective), so every homography that keeps the
/// orientation of the plane has exactly one such set of numbers ([`Motion::from_matrix`]), and
/// the set gives back the matrix ([`Motion::matrix`]).
///
/// - `pan`, `tilt`: where the anchor goes, in pixels (`H(c) = c + [pan, tilt]`), right and down.
/// - `zoom`: the scale about the anchor, 1 for none.
/// - `rotate`: in radians, from x towards y, which on a frame whose y points down is clockwise.
/// - `aspect`, `shear`, `yaw`, `pitch`: the rest of a homography, all under the Perspective 3D
///   box; `yaw` and `pitch` (per pixel from the anchor) tilt the plane about the vertical and the
///   horizontal axis, the "pitch and yaw in 3D space" of p. 3235. Without Perspective 3D they are
///   1, 0, 0, 0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    /// The shift of the anchor across, in pixels.
    pub pan: f64,
    /// The shift of the anchor down, in pixels.
    pub tilt: f64,
    /// The scale about the anchor.
    pub zoom: f64,
    /// The rotation about the anchor, in radians.
    pub rotate: f64,
    /// The stretch across against down (`K₁₁`).
    pub aspect: f64,
    /// The shear (`K₁₂`).
    pub shear: f64,
    /// The perspective across (`P₃₁`), per pixel.
    pub yaw: f64,
    /// The perspective down (`P₃₂`), per pixel.
    pub pitch: f64,
}

impl Default for Motion {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// One of the eight numbers of a [`Motion`], each a curve of the tracker graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    /// [`Motion::pan`].
    Pan,
    /// [`Motion::tilt`].
    Tilt,
    /// [`Motion::zoom`].
    Zoom,
    /// [`Motion::rotate`].
    Rotate,
    /// [`Motion::aspect`].
    Aspect,
    /// [`Motion::shear`].
    Shear,
    /// [`Motion::yaw`].
    Yaw,
    /// [`Motion::pitch`].
    Pitch,
}

impl Component {
    /// All eight, in the order of the fields of [`Motion`].
    pub const ALL: [Self; 8] = [
        Self::Pan,
        Self::Tilt,
        Self::Zoom,
        Self::Rotate,
        Self::Aspect,
        Self::Shear,
        Self::Yaw,
        Self::Pitch,
    ];

    /// Its place in [`Component::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    /// Its value when there is no motion.
    #[must_use]
    pub fn identity(self) -> f64 {
        match self {
            Self::Zoom | Self::Aspect => 1.0,
            _ => 0.0,
        }
    }
}

impl Motion {
    /// No motion.
    pub const IDENTITY: Self = Self {
        pan: 0.0,
        tilt: 0.0,
        zoom: 1.0,
        rotate: 0.0,
        aspect: 1.0,
        shear: 0.0,
        yaw: 0.0,
        pitch: 0.0,
    };

    /// The value of `c`.
    #[must_use]
    pub fn get(&self, c: Component) -> f64 {
        match c {
            Component::Pan => self.pan,
            Component::Tilt => self.tilt,
            Component::Zoom => self.zoom,
            Component::Rotate => self.rotate,
            Component::Aspect => self.aspect,
            Component::Shear => self.shear,
            Component::Yaw => self.yaw,
            Component::Pitch => self.pitch,
        }
    }

    /// Set `c` to `v`.
    pub fn set(&mut self, c: Component, v: f64) {
        *match c {
            Component::Pan => &mut self.pan,
            Component::Tilt => &mut self.tilt,
            Component::Zoom => &mut self.zoom,
            Component::Rotate => &mut self.rotate,
            Component::Aspect => &mut self.aspect,
            Component::Shear => &mut self.shear,
            Component::Yaw => &mut self.yaw,
            Component::Pitch => &mut self.pitch,
        } = v;
    }

    /// The homography it stands for, about `anchor`.
    #[must_use]
    pub fn matrix(&self, anchor: Point) -> Mat3 {
        let (s, c) = self.rotate.sin_cos();
        let z = self.zoom;
        let (k1, k2, k3) = (self.aspect, self.shear, 1.0 / self.aspect);
        // A = z·R·K, then H_local = [[A + t·vᵀ, t], [vᵀ, 1]].
        let a = [
            [z * c * k1, z * (c * k2 - s * k3)],
            [z * s * k1, z * (s * k2 + c * k3)],
        ];
        let (t, v) = ([self.pan, self.tilt], [self.yaw, self.pitch]);
        let local = Mat3([
            [a[0][0] + t[0] * v[0], a[0][1] + t[0] * v[1], t[0]],
            [a[1][0] + t[1] * v[0], a[1][1] + t[1] * v[1], t[1]],
            [v[0], v[1], 1.0],
        ]);
        Mat3::translation(anchor[0], anchor[1])
            .mul(&local)
            .mul(&Mat3::translation(-anchor[0], -anchor[1]))
    }

    /// The motion that `h` makes about `anchor`, or `None` when `h` turns the plane over, sends
    /// the anchor to infinity or is not finite.
    #[must_use]
    pub fn from_matrix(h: &Mat3, anchor: Point) -> Option<Self> {
        let local = Mat3::translation(-anchor[0], -anchor[1])
            .mul(h)
            .mul(&Mat3::translation(anchor[0], anchor[1]));
        let w = local.0[2][2];
        if !w.is_finite() || w.abs() < 1e-12 {
            return None;
        }
        let m = local.scaled(1.0 / w).0;
        let (t, v) = ([m[0][2], m[1][2]], [m[2][0], m[2][1]]);
        // M = A − t·vᵀ = z·R·K.
        let mm = [
            [m[0][0] - t[0] * v[0], m[0][1] - t[0] * v[1]],
            [m[1][0] - t[1] * v[0], m[1][1] - t[1] * v[1]],
        ];
        let det = mm[0][0] * mm[1][1] - mm[0][1] * mm[1][0];
        if !(det.is_finite() && det > 0.0) {
            return None;
        }
        let zoom = det.sqrt();
        let rotate = mm[1][0].atan2(mm[0][0]);
        let (s, c) = rotate.sin_cos();
        let aspect = (c * mm[0][0] + s * mm[1][0]) / zoom;
        let shear = (c * mm[0][1] + s * mm[1][1]) / zoom;
        let out = Self {
            pan: t[0],
            tilt: t[1],
            zoom,
            rotate,
            aspect,
            shear,
            yaw: v[0],
            pitch: v[1],
        };
        Component::ALL
            .iter()
            .all(|&c| out.get(c).is_finite())
            .then_some(out)
    }

    /// The motion with every component that `enabled` leaves out put back to no motion.
    #[must_use]
    pub fn masked(&self, enabled: Components) -> Self {
        let mut out = *self;
        for c in Component::ALL {
            if !enabled.has(c) {
                out.set(c, c.identity());
            }
        }
        out
    }

    /// The motion `share` of the way from `self` to `other`, component by component.
    #[must_use]
    pub fn lerp(&self, other: &Self, share: f64) -> Self {
        let mut out = *self;
        for c in Component::ALL {
            let (a, b) = (self.get(c), other.get(c));
            out.set(c, a + (b - a) * share);
        }
        out
    }
}

/// The boxes of the tracker that say which motion a track applies (p. 3235), to be set before
/// the track: Pan, Tilt, Zoom, Rotate and Perspective 3D. The default is the first four on and
/// Perspective 3D off (an assumption: the manual does not say).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the five boxes of the palette"
)]
pub struct Components {
    /// Pan (across).
    pub pan: bool,
    /// Tilt (down).
    pub tilt: bool,
    /// Zoom.
    pub zoom: bool,
    /// Rotate.
    pub rotate: bool,
    /// Perspective 3D: aspect, shear, yaw and pitch.
    pub perspective: bool,
}

impl Default for Components {
    fn default() -> Self {
        Self {
            pan: true,
            tilt: true,
            zoom: true,
            rotate: true,
            perspective: false,
        }
    }
}

impl Components {
    /// All five on.
    pub const ALL: Self = Self {
        pan: true,
        tilt: true,
        zoom: true,
        rotate: true,
        perspective: true,
    };

    /// Whether `c` is applied.
    #[must_use]
    pub fn has(&self, c: Component) -> bool {
        match c {
            Component::Pan => self.pan,
            Component::Tilt => self.tilt,
            Component::Zoom => self.zoom,
            Component::Rotate => self.rotate,
            Component::Aspect | Component::Shear | Component::Yaw | Component::Pitch => {
                self.perspective
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn no_motion_is_the_identity_matrix() {
        assert_eq!(Motion::IDENTITY.matrix([100.0, 50.0]), Mat3::IDENTITY);
        let m = Motion {
            zoom: 2.0,
            ..Motion::IDENTITY
        };
        // A zoom about the anchor keeps the anchor.
        let p = m.matrix([10.0, 20.0]).apply([10.0, 20.0]);
        assert!((p[0] - 10.0).abs() < 1e-12 && (p[1] - 20.0).abs() < 1e-12);
    }

    proptest! {
        #[test]
        fn a_motion_comes_back_from_its_matrix(
            pan in -300.0..300.0_f64, tilt in -300.0..300.0_f64,
            zoom in 0.3..3.0_f64, rotate in -3.1..3.1_f64,
            aspect in 0.5..2.0_f64, shear in -0.5..0.5_f64,
            yaw in -1e-3..1e-3_f64, pitch in -1e-3..1e-3_f64,
            ax in -500.0..2000.0_f64, ay in -500.0..1500.0_f64,
        ) {
            let m = Motion { pan, tilt, zoom, rotate, aspect, shear, yaw, pitch };
            let back = Motion::from_matrix(&m.matrix([ax, ay]), [ax, ay]).unwrap();
            for c in Component::ALL {
                let (a, b) = (m.get(c), back.get(c));
                prop_assert!((a - b).abs() <= 1e-8 * (1.0 + a.abs()), "{c:?}: {a} {b}");
            }
        }
    }

    #[test]
    fn a_turned_over_plane_has_no_motion() {
        let flip = Mat3([[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert!(Motion::from_matrix(&flip, [0.0, 0.0]).is_none());
    }

    #[test]
    fn a_mask_puts_back_what_is_off() {
        let m = Motion {
            pan: 3.0,
            tilt: 4.0,
            zoom: 1.5,
            rotate: 0.2,
            aspect: 1.1,
            shear: 0.1,
            yaw: 1e-4,
            pitch: 2e-4,
        };
        let only = m.masked(Components {
            pan: true,
            tilt: false,
            zoom: false,
            rotate: true,
            perspective: false,
        });
        assert_eq!(
            only,
            Motion {
                pan: 3.0,
                rotate: 0.2,
                ..Motion::IDENTITY
            }
        );
        assert_eq!(m.masked(Components::ALL), m);
    }
}
