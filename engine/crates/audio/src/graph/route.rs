//! How a strip puts its sound among the channels of the buses it feeds (plan T4b, "声像"): the
//! laws of pan, and the matrices that turn one layout of channels into another.
//!
//! - A mono source into stereo pans at equal power: cos θ and sin θ, θ = (x + 1)·π/4, so that the
//!   centre is −3.01 dB on each side (C-F5, C-F6).
//! - A stereo source into stereo is balanced, as the engine has done since phase 1: √2·cos θ and
//!   √2·sin θ on its own sides, unity at the centre, +3 dB on the near side at an edge (C-F5).
//! - Into a bus of more than two channels a mono source (and each channel of an Adaptive track,
//!   C-F9) is panned between the front speakers by `x` and towards the back by `y`, each pair of
//!   speakers at equal power; `divergence` spreads the centre to the left and right; the Boom
//!   sends it to the LFE.
//! - Other sources go channel by channel to the channels of the same name, and what has none is
//!   folded down by ITU-R BS.775: a centre into left and right at −3 dB, a surround into its side
//!   at −3 dB, the LFE dropped (C-F7); a source of fewer channels leaves the others silent.
//!
//! Every function here is pure and allocates nothing: the thread of the mix works a matrix out
//! again while a pan ramps or follows a lane.

use std::f32::consts::FRAC_1_SQRT_2;
use std::f64::consts::FRAC_PI_4;

use crate::constants::{
    BALANCE_EDGE_GAIN, DOWNMIX_CENTRE_GAIN, DOWNMIX_LFE_GAIN, DOWNMIX_SURROUND_GAIN, MAX_CHANNELS,
};
use crate::node::{Layout, Planar, Role};

/// The gains from each channel of a source to each channel of a bus: `gain(output, input)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    outputs: usize,
    inputs: usize,
    gains: [[f32; MAX_CHANNELS]; MAX_CHANNELS],
}

impl Matrix {
    /// Silence from `inputs` channels to `outputs`.
    #[must_use]
    pub const fn zero(outputs: usize, inputs: usize) -> Self {
        Matrix {
            outputs,
            inputs,
            gains: [[0.0; MAX_CHANNELS]; MAX_CHANNELS],
        }
    }

    /// The channels out.
    #[must_use]
    pub fn outputs(&self) -> usize {
        self.outputs
    }

    /// The channels in.
    #[must_use]
    pub fn inputs(&self) -> usize {
        self.inputs
    }

    /// The gain from `input` to `output`.
    #[must_use]
    pub fn gain(&self, output: usize, input: usize) -> f32 {
        self.gains[output][input]
    }

    /// Adds `gain` from `input` to `output`.
    pub fn add(&mut self, output: usize, input: usize, gain: f32) {
        self.gains[output][input] += gain;
    }

    /// Scales what goes to `output` by `factor`.
    pub fn scale_output(&mut self, output: usize, factor: f32) {
        for gain in &mut self.gains[output][..self.inputs] {
            *gain *= factor;
        }
    }

    /// Adds `frames` samples of `source`, through the matrix, to `into`: each channel out gets
    /// each channel in times its gain, the channels in taken in order, and a gain of 0 adds
    /// nothing (not even `0 · x`), so that a channel that goes straight through is added to the
    /// bit.
    pub fn mix(&self, source: &Planar, into: &mut Planar, frames: usize) {
        for output in 0..self.outputs {
            let into = &mut into.channel_mut(output)[..frames];
            for input in 0..self.inputs {
                let gain = self.gains[output][input];
                if gain == 0.0 {
                    continue;
                }
                let source = &source.channel(input)[..frames];
                for (into, source) in into.iter_mut().zip(source) {
                    *into += gain * source;
                }
            }
        }
    }

    /// Adds sample `at` of `source`, through the matrix, to sample `at` of `into`: as
    /// [`Matrix::mix`], for a matrix that changes from one sample to the next.
    pub fn mix_sample(&self, source: &Planar, into: &mut Planar, at: usize) {
        for output in 0..self.outputs {
            for input in 0..self.inputs {
                let gain = self.gains[output][input];
                if gain != 0.0 {
                    into.channel_mut(output)[at] += gain * source.channel(input)[at];
                }
            }
        }
    }
}

/// A pan as the graph reads it from a strip or a send at a sample: the model's `Pan`, its
/// divergence as a share and its Boom as a gain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanAt {
    /// Whether it pans: off, a mono source goes to the centre and the others channel by channel.
    pub on: bool,
    /// Left (−1) to right (1).
    pub x: f32,
    /// Back (−1) to front (1).
    pub y: f32,
    /// How much of the centre spreads to the left and right, 0 to 1.
    pub divergence: f32,
    /// The gain into the LFE, 0 when the Boom is off.
    pub boom: f32,
}

impl PanAt {
    /// The centre, on, with no divergence and no Boom.
    pub const CENTRE: PanAt = PanAt {
        on: true,
        x: 0.0,
        y: 0.0,
        divergence: 0.0,
        boom: 0.0,
    };
}

/// The angle of a pan of `x` between the left (0) and the right (π/2).
fn angle(x: f32) -> f64 {
    (f64::from(x.clamp(-1.0, 1.0)) + 1.0) * FRAC_PI_4
}

/// The gains of a stereo source panned to `x` into stereo, a balance: √2·cos θ on the left and
/// √2·sin θ on the right; unity at the centre, and at an edge +3 dB on the near side and nothing
/// on the other, exactly (`mix.rs::pan` of phase 1, C-F5).
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::float_cmp,
    reason = "gains are f32; the edges are given exactly"
)]
pub fn balance(x: f32) -> [f32; 2] {
    if x <= -1.0 {
        [BALANCE_EDGE_GAIN, 0.0]
    } else if x >= 1.0 {
        [0.0, BALANCE_EDGE_GAIN]
    } else {
        let theta = angle(x);
        [
            (std::f64::consts::SQRT_2 * theta.cos()) as f32,
            (std::f64::consts::SQRT_2 * theta.sin()) as f32,
        ]
    }
}

/// The gains of a mono source panned to `x` into stereo, at equal power: cos θ and sin θ, −3.01 dB
/// on each side at the centre (C-F5, C-F6); at an edge unity on the near side and nothing on the
/// other, exactly.
#[must_use]
#[allow(clippy::cast_possible_truncation, reason = "gains are f32")]
pub fn equal_power(x: f32) -> [f32; 2] {
    if x <= -1.0 {
        [1.0, 0.0]
    } else if x >= 1.0 {
        [0.0, 1.0]
    } else {
        let theta = angle(x);
        [theta.cos() as f32, theta.sin() as f32]
    }
}

/// Where `role` is among the channels of `layout`.
fn place(layout: Layout, role: Role) -> Option<usize> {
    layout.roles().iter().position(|&r| r == role)
}

/// The matrix that takes the channels of `from` to those of `to` with no pan (the pan of a strip
/// switched off, a change of format): channel by channel where the names meet, and folded down
/// by ITU-R BS.775 where they do not (see the module); an Adaptive channel goes to the channel of
/// its number, if there is one.
#[must_use]
pub fn convert(from: Layout, to: Layout) -> Matrix {
    let mut matrix = Matrix::zero(to.channels(), from.channels());
    for (input, &role) in from.roles().iter().enumerate() {
        fold(role, input, to, 1.0, &mut matrix);
    }
    matrix
}

/// Adds channel `input` of `role` at `gain` to the channels of `to` that it folds into.
fn fold(role: Role, input: usize, to: Layout, gain: f32, matrix: &mut Matrix) {
    if gain == 0.0 {
        return;
    }
    if let Layout::Adaptive(_) = to {
        // An Adaptive layout has no names: channel by channel.
        if input < to.channels() {
            matrix.add(input, input, gain);
        }
        return;
    }
    if let Some(output) = place(to, role) {
        matrix.add(output, input, gain);
        return;
    }
    let half = FRAC_1_SQRT_2;
    match role {
        Role::Mono => {
            if let Some(centre) = place(to, Role::Centre) {
                matrix.add(centre, input, gain);
            } else {
                fold(Role::Left, input, to, gain * half, matrix);
                fold(Role::Right, input, to, gain * half, matrix);
            }
        }
        Role::Left | Role::Right => {
            if let Some(mono) = place(to, Role::Mono) {
                matrix.add(mono, input, gain * half);
            }
        }
        Role::Centre => {
            if let Some(mono) = place(to, Role::Mono) {
                matrix.add(mono, input, gain);
            } else {
                fold(Role::Left, input, to, gain * DOWNMIX_CENTRE_GAIN, matrix);
                fold(Role::Right, input, to, gain * DOWNMIX_CENTRE_GAIN, matrix);
            }
        }
        Role::Lfe => {
            if DOWNMIX_LFE_GAIN != 0.0 {
                fold(Role::Centre, input, to, gain * DOWNMIX_LFE_GAIN, matrix);
            }
        }
        Role::Surround => {
            fold(Role::LeftSurround, input, to, gain * half, matrix);
            fold(Role::RightSurround, input, to, gain * half, matrix);
        }
        Role::LeftSurround | Role::RightSurround => {
            if let Some(surround) = place(to, Role::Surround) {
                matrix.add(surround, input, gain * half);
            } else {
                let side = if role == Role::LeftSurround {
                    Role::Left
                } else {
                    Role::Right
                };
                fold(side, input, to, gain * DOWNMIX_SURROUND_GAIN, matrix);
            }
        }
        Role::LeftRear => fold(Role::LeftSurround, input, to, gain, matrix),
        Role::RightRear => fold(Role::RightSurround, input, to, gain, matrix),
        Role::Discrete(channel) => {
            let channel = usize::from(channel);
            if channel < to.channels() {
                matrix.add(channel, input, gain);
            }
        }
    }
}

/// The matrix that pans `from` into `to` as `pan` says (see the module).
#[must_use]
pub fn panned(from: Layout, to: Layout, pan: &PanAt) -> Matrix {
    if !pan.on || to == Layout::Mono {
        return convert(from, to);
    }
    let mono_like = matches!(from, Layout::Mono | Layout::Adaptive(_));
    let mut matrix = if to == Layout::Stereo {
        if mono_like {
            let [left, right] = equal_power(pan.x);
            let mut matrix = Matrix::zero(2, from.channels());
            for input in 0..from.channels() {
                matrix.add(0, input, left);
                matrix.add(1, input, right);
            }
            matrix
        } else {
            let [left, right] = balance(pan.x);
            let mut matrix = convert(from, to);
            matrix.scale_output(0, left);
            matrix.scale_output(1, right);
            matrix
        }
    } else if mono_like {
        let gains = positioned(to, pan);
        let mut matrix = Matrix::zero(to.channels(), from.channels());
        for input in 0..from.channels() {
            for (output, &gain) in gains[..to.channels()].iter().enumerate() {
                if gain != 0.0 {
                    matrix.add(output, input, gain);
                }
            }
        }
        matrix
    } else {
        let [left, right] = balance(pan.x);
        let mut matrix = convert(from, to);
        for (output, &role) in to.roles().iter().enumerate() {
            match role {
                Role::Left | Role::LeftSurround | Role::LeftRear => {
                    matrix.scale_output(output, left);
                }
                Role::Right | Role::RightSurround | Role::RightRear => {
                    matrix.scale_output(output, right);
                }
                _ => {}
            }
        }
        matrix
    };
    boom(from, to, pan, &mut matrix);
    matrix
}

/// Adds the Boom of `pan`: each channel of `from` but its LFE, at an equal share of its gain, to
/// the LFE of `to`.
#[allow(clippy::cast_precision_loss, reason = "at most 24 channels")]
fn boom(from: Layout, to: Layout, pan: &PanAt, matrix: &mut Matrix) {
    let Some(lfe) = to.lfe() else {
        return;
    };
    if pan.boom == 0.0 {
        return;
    }
    let sources = from.channels() - usize::from(from.lfe().is_some());
    if sources == 0 {
        return;
    }
    let share = pan.boom / sources as f32;
    for input in 0..from.channels() {
        if Some(input) != from.lfe() {
            matrix.add(lfe, input, share);
        }
    }
}

/// The gains of a mono source into each channel of `to`, a layout of three channels or more, at
/// `pan`: between the front speakers by `x` (left to centre, centre to right, each pair at equal
/// power); towards the back as `y` falls below 0, at equal power (through the side surrounds to
/// the rear ones where there are both); and the centre spread to the left and right by the
/// divergence, keeping the power.
#[allow(clippy::cast_possible_truncation, reason = "gains are f32")]
fn positioned(to: Layout, pan: &PanAt) -> [f32; MAX_CHANNELS] {
    use std::f64::consts::FRAC_PI_2;
    let mut gains = [0.0; MAX_CHANNELS];
    let x = f64::from(pan.x.clamp(-1.0, 1.0));
    // The front: L, C, R.
    let (mut left, mut centre, mut right) = if x <= 0.0 {
        let share = (x + 1.0) * FRAC_PI_2;
        (share.cos(), share.sin(), 0.0)
    } else {
        let share = x * FRAC_PI_2;
        (0.0, share.cos(), share.sin())
    };
    let divergence = f64::from(pan.divergence.clamp(0.0, 1.0));
    if divergence > 0.0 {
        let spread = centre * centre * divergence / 2.0;
        left = (left * left + spread).sqrt();
        right = (right * right + spread).sqrt();
        centre *= (1.0 - divergence).sqrt();
    }
    // Front to back, where there is a back.
    let behind_at_all =
        place(to, Role::Surround).is_some() || place(to, Role::LeftSurround).is_some();
    let back = if behind_at_all {
        f64::from((-pan.y).clamp(0.0, 1.0))
    } else {
        0.0
    };
    let rear = place(to, Role::LeftRear).is_some();
    let (front, side, behind) = if rear {
        if back <= 0.5 {
            let turn = back * std::f64::consts::PI;
            (turn.cos(), turn.sin(), 0.0)
        } else {
            let turn = (back - 0.5) * std::f64::consts::PI;
            (0.0, turn.cos(), turn.sin())
        }
    } else {
        let turn = back * FRAC_PI_2;
        (turn.cos(), turn.sin(), 0.0)
    };
    let mut put = |role: Role, gain: f64| {
        if let Some(at) = place(to, role) {
            gains[at] += gain as f32;
        }
    };
    put(Role::Left, left * front);
    put(Role::Centre, centre * front);
    put(Role::Right, right * front);
    // The back, by x between its two sides; the one surround of LCRS takes it all.
    let [back_left, back_right] = equal_power(pan.x);
    if place(to, Role::Surround).is_some() {
        put(Role::Surround, side);
    } else {
        put(Role::LeftSurround, f64::from(back_left) * side);
        put(Role::RightSurround, f64::from(back_right) * side);
    }
    put(Role::LeftRear, f64::from(back_left) * behind);
    put(Role::RightRear, f64::from(back_right) * behind);
    gains
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "the gains of the centre and the edges are exact"
    )]
    fn the_laws_of_pan_are_exact_at_the_centre_and_the_edges() {
        assert_eq!(balance(0.0), [1.0, 1.0]);
        assert_eq!(balance(-1.0), [std::f32::consts::SQRT_2, 0.0]);
        assert_eq!(balance(1.0), [0.0, std::f32::consts::SQRT_2]);
        assert_eq!(equal_power(0.0), [FRAC_1_SQRT_2, FRAC_1_SQRT_2]);
        assert_eq!(equal_power(-1.0), [1.0, 0.0]);
        assert_eq!(equal_power(1.0), [0.0, 1.0]);
    }
}
