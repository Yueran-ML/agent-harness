//! Lines and dots that a page has the view draw over its picture: the motion path of a clip in
//! the timeline viewer of the Edit page (`view.overlay` of `PROTOCOL.md`).
//!
//! The page says what is where, in pixels of the picture from its top left corner (a pixel's
//! centre at `.5`, as the pointer is told); the view says how each kind of mark looks, so that
//! every page draws the same line and the same dots. The marks are the view's, not a frame's:
//! they are drawn over whichever composed picture of their size the view shows, and drawn again
//! as they change, without a new picture. Nothing hits them: the page, which made them, knows
//! where they are.
//!
//! Each mark is drawn twice, a dark one under a light one a little smaller, so that it is seen
//! over a bright picture and over a dark one. The colours and the sizes are choices of this
//! project, made without the manual (which shows a path and its points, not their pixels).

use crate::window_overlay::Primitive;
use crate::ViewportMap;

/// How wide a line of the marks is, in device pixels.
pub const LINE_WIDTH: f32 = 1.5;
/// How much wider the dark line under it is, and how much larger the dark disk under a dot, in
/// device pixels each side.
pub const HALO: f32 = 1.0;
/// The colour of a line, and of the dots that have no other.
const LIGHT: [f32; 3] = [1.0, 1.0, 1.0];
/// The colour under every mark.
const DARK: [f32; 3] = [0.0, 0.0, 0.0];

/// What a dot marks, which says how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DotKind {
    /// A place on a line at one frame: small, so that their spacing shows the speed.
    Tick,
    /// A key: larger, to be pressed.
    Key,
    /// The key that is being dragged.
    Active,
    /// The place at the playhead.
    Current,
}

impl DotKind {
    /// The radius of its disk in device pixels, and its colour.
    #[must_use]
    pub fn look(self) -> (f32, [f32; 3]) {
        match self {
            Self::Tick => (2.0, LIGHT),
            Self::Key => (4.5, LIGHT),
            Self::Active => (5.5, [1.0, 0.62, 0.12]),
            Self::Current => (3.5, [1.0, 0.3, 0.24]),
        }
    }
}

/// Dots of one kind, in pixels of the picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Dots {
    pub kind: DotKind,
    pub at: Vec<[f64; 2]>,
}

/// What a page has the view draw over its picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Marks {
    /// The size of the picture that the marks were made for. They are drawn over a picture of
    /// this size and over no other, where they would lie elsewhere than they were meant to.
    pub picture: (u32, u32),
    /// Lines, each through its points in order, in pixels of the picture.
    pub lines: Vec<Vec<[f64; 2]>>,
    /// Dots, drawn in this order over the lines: the last lie on top.
    pub dots: Vec<Dots>,
}

impl Marks {
    /// Whether the marks are drawn over the picture that `map` places: one of their size.
    #[must_use]
    pub fn fits(&self, map: ViewportMap) -> bool {
        self.picture == map.picture
    }

    /// What is drawn for the marks where `map` places the picture: the strokes of the lines,
    /// then the disks of the dots, in device pixels. With `under`, the dark marks that lie
    /// beneath them, each larger by [`HALO`]; a line's are as long, and drawn wider by the
    /// painter.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "quantize once for the CPU and the GPU, as the Window controls do"
    )]
    pub(crate) fn primitives(&self, map: ViewportMap, under: bool) -> Vec<Primitive> {
        let device = |point: [f64; 2]| {
            let (x, y) = map.to_device((point[0], point[1]));
            [x as f32, y as f32]
        };
        let colour = |own: [f32; 3]| if under { DARK } else { own };
        let mut out = Vec::new();
        for line in &self.lines {
            out.extend(line.windows(2).filter_map(|ends| {
                Primitive::stroke(device(ends[0]), device(ends[1]), colour(LIGHT))
            }));
        }
        for dots in &self.dots {
            let (radius, own) = dots.kind.look();
            let radius = if under { radius + HALO } else { radius };
            out.extend(
                dots.at
                    .iter()
                    .map(|at| device(*at))
                    .filter(|centre| centre.iter().all(|v| v.is_finite()))
                    .map(|centre| Primitive::dot(centre, radius, colour(own))),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Viewport;

    /// A picture of 200 by 100 shown twice as large in a view of 400 by 300: 50 black pixels
    /// above and below it.
    fn map() -> ViewportMap {
        Viewport::Fit.map((400, 300), (200, 100), 0).unwrap()
    }

    fn marks() -> Marks {
        Marks {
            picture: (200, 100),
            lines: vec![vec![
                [10.0, 20.0],
                [110.0, 20.0],
                [110.0, 20.0],
                [110.0, 70.0],
            ]],
            dots: vec![
                Dots {
                    kind: DotKind::Tick,
                    at: vec![[60.0, 20.0]],
                },
                Dots {
                    kind: DotKind::Key,
                    at: vec![[10.0, 20.0], [110.0, 70.0]],
                },
                Dots {
                    kind: DotKind::Current,
                    at: vec![[110.0, 20.0]],
                },
            ],
        }
    }

    #[test]
    fn marks_are_drawn_over_a_picture_of_their_size_only() {
        assert_eq!(map().dest, [0.0, 50.0, 400.0, 200.0]);
        assert!(marks().fits(map()));
        let other = Viewport::Fit.map((400, 300), (100, 200), 0).unwrap();
        assert!(!marks().fits(other));
    }

    #[test]
    fn a_line_is_a_stroke_between_each_two_of_its_points_that_differ() {
        let drawn = marks().primitives(map(), false);
        let strokes: Vec<_> = drawn.iter().filter(|p| p.kind > 2.5).collect();
        // The piece between the two points that are one place has no direction: it is left out.
        assert_eq!(strokes.len(), 2);
        // A point of the picture is where the map puts it: twice as far, 50 pixels down.
        assert_eq!(strokes[0].geometry.centre, [20.0, 90.0]);
        assert_eq!(strokes[0].geometry.inverse, [200.0, 0.0, 0.0, 0.0]);
        assert_eq!(strokes[1].geometry.centre, [220.0, 90.0]);
        assert_eq!(strokes[1].geometry.inverse, [0.0, 100.0, 0.0, 0.0]);
        for stroke in &strokes {
            assert_eq!(stroke.colour, LIGHT);
        }
        // On the stroke a pixel is covered whole, and one well beside it not at all.
        assert!((strokes[0].alpha([120.5, 90.5], LINE_WIDTH) - 0.75).abs() < 1e-6);
        assert!((strokes[0].alpha([120.5, 90.0], LINE_WIDTH) - 1.0).abs() < 1e-6);
        assert!(strokes[0].alpha([120.5, 93.5], LINE_WIDTH).abs() < 1e-6);
    }

    #[test]
    fn dots_are_disks_of_their_kind_in_the_order_given_after_the_lines() {
        let drawn = marks().primitives(map(), false);
        let dots: Vec<_> = drawn.iter().filter(|p| p.kind < 2.5).collect();
        assert_eq!(drawn.len(), 2 + 4);
        assert!(drawn[..2].iter().all(|p| p.kind > 2.5));
        let places: Vec<_> = dots.iter().map(|p| p.geometry.centre).collect();
        assert_eq!(
            places,
            [[120.0, 90.0], [20.0, 90.0], [220.0, 190.0], [220.0, 90.0]]
        );
        let sizes: Vec<_> = dots.iter().map(|p| p.geometry.scale).collect();
        assert_eq!(sizes, [2.0, 4.5, 4.5, 3.5]);
        assert_eq!(dots[0].colour, LIGHT);
        assert_eq!(dots[3].colour, DotKind::Current.look().1);
        // A disk covers the pixels within its radius, and none a pixel beyond it.
        assert!((dots[1].alpha([20.5, 90.5], LINE_WIDTH) - 1.0).abs() < 1e-6);
        assert!((dots[1].alpha([24.0, 90.0], LINE_WIDTH) - 1.0).abs() < 1e-6);
        assert!(dots[1].alpha([25.5, 90.0], LINE_WIDTH).abs() < 1e-6);
    }

    #[test]
    fn the_marks_beneath_are_dark_and_larger() {
        let over = marks().primitives(map(), false);
        let under = marks().primitives(map(), true);
        assert_eq!(over.len(), under.len());
        for (light, dark) in over.iter().zip(&under) {
            assert_eq!(dark.colour, DARK);
            assert_eq!(light.geometry.centre, dark.geometry.centre);
            if light.kind < 2.5 {
                assert!((dark.geometry.scale - light.geometry.scale - HALO).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn a_place_that_is_no_number_is_not_drawn() {
        let odd = Marks {
            picture: (200, 100),
            lines: vec![vec![[f64::NAN, 0.0], [10.0, 10.0]]],
            dots: vec![Dots {
                kind: DotKind::Key,
                at: vec![[f64::INFINITY, 0.0], [5.0, 5.0]],
            }],
        };
        let drawn = odd.primitives(map(), false);
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].geometry.centre, [10.0, 60.0]);
    }
}
