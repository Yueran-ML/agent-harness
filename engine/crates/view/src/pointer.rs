//! The pointer on a view: where it is on the picture, as the window's pixels are laid over it.
//!
//! A view takes no input unless it is asked to ([`crate::View::take_pointer`]): its window is
//! disabled and lets the clicks of the user through to the page beneath it. While it takes the
//! pointer, its window is enabled and is the target of the mouse, and each press, move and release
//! of a button over it (or, once a button is pressed over it, anywhere, as the window captures the
//! mouse until the button is released) is reported ([`crate::Event::Pointer`]), with the place on
//! the picture that it is over and the keys held, for the tools of the page that are used on the
//! picture: the pickers of colour, the line of a wipe, the windows.

/// What the pointer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    /// A button was pressed.
    Down,
    /// It moved.
    Move,
    /// A button was released.
    Up,
    /// The window lost the mouse while a button was held (another window took it): the drag
    /// ends where it was, as if the button were released.
    Cancel,
}

impl PointerKind {
    /// Its name, as it is written.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::Move => "move",
            Self::Up => "up",
            Self::Cancel => "cancel",
        }
    }
}

/// A button of the mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    /// Its name, as it is written.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
        }
    }
}

/// What the pointer did on a view, where, and with what held.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the keys held, and whether it is on the picture, as the system tells them"
)]
pub struct Pointer {
    pub kind: PointerKind,
    /// The button pressed or released; none for a move.
    pub button: Option<Button>,
    /// The buttons held once it was done, a bit each: 1 left, 2 right, 4 middle.
    pub buttons: u8,
    /// Where it is on the picture shown, in pixels of the frame drawn from its top left corner
    /// (of the picture turned as it is shown), the centre of the first pixel at 0.5; none where
    /// no picture is drawn.
    pub picture: Option<(f64, f64)>,
    /// Captured under the same state lock as `picture`, for native tools bound to a frame.
    /// Kept inside the engine; the current pointer protocol does not serialize it.
    pub presentation: Option<crate::Presentation>,
    /// Whether it is over the picture, not over the black bars about it or beyond the window.
    pub inside: bool,
    /// Where it is in the window, in device pixels from its top left corner.
    pub window: (i32, i32),
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// Where a point of a window of `window` pixels is on a frame of `frame` pixels (as it is shown,
/// turned) drawn into it as [`crate::letterbox`] lays it, and whether it is on it: none where
/// nothing is drawn. Points of the window are read at the centres of its pixels.
#[must_use]
pub fn on_picture(
    window: (u32, u32),
    frame: (u32, u32),
    point: (i32, i32),
) -> Option<((f64, f64), bool)> {
    Some(
        crate::Viewport::Fit
            .map(window, frame, 0)?
            .on_picture(point),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_of_the_window_is_found_on_the_picture_laid_in_it() {
        // 1920 × 1080 in 960 × 600: 960 × 540, 30 pixels of black above and below.
        let on = |x, y| on_picture((960, 600), (1920, 1080), (x, y));
        assert_eq!(on(0, 30), Some(((1.0, 1.0), true)));
        assert_eq!(on(959, 569), Some(((1919.0, 1079.0), true)));
        assert_eq!(on(479, 299), Some(((959.0, 539.0), true)));
        // On the bars, and beyond the window, as a captured drag goes.
        assert_eq!(on(10, 10), Some(((21.0, -39.0), false)));
        assert_eq!(on(-5, 300), Some(((-9.0, 541.0), false)));
        assert_eq!(on(960, 300), Some(((1921.0, 541.0), false)));
        // At its own size, a pixel is a pixel.
        assert_eq!(
            on_picture((64, 36), (64, 36), (3, 4)),
            Some(((3.5, 4.5), true))
        );
        // A tall picture, with bars on its sides.
        assert_eq!(
            on_picture((100, 100), (50, 100), (25, 0)),
            Some(((0.5, 0.5), true))
        );
        assert_eq!(
            on_picture((100, 100), (50, 100), (24, 0)),
            Some(((-0.5, 0.5), false))
        );
        // Nothing drawn.
        assert_eq!(on_picture((100, 100), (0, 0), (1, 1)), None);
        assert_eq!(on_picture((0, 0), (10, 10), (1, 1)), None);
    }
}
