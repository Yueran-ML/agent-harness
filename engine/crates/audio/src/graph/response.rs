//! Model-static curves for control clients. These functions run off the mix thread and reuse
//! the graph's model conversion; they neither evaluate automation nor expose transient meters.
use columbina_model::audio::fx::{Dynamics, Equalizer};

use super::build::{dynamics_settings, eq_settings};
use crate::fx::{dynamics, eq};

/// The EQ response in dB of an already validated clip (4 bands) or strip (6 bands).
#[must_use]
pub fn eq_response(model: &Equalizer, sample_rate: u32, frequencies: &[f32]) -> Vec<f32> {
    let variant = if model.bands.len() == 4 {
        eq::Variant::Clip
    } else {
        eq::Variant::Strip
    };
    eq::response(&eq_settings(model, variant), sample_rate, frequencies)
}

/// The steady-state output in dBFS, including dry mix and make-up, from model settings.
#[must_use]
pub fn dynamics_curve(model: &Dynamics, inputs_db: &[f32]) -> Vec<f32> {
    dynamics::curve(&dynamics_settings(model, false), inputs_db)
}
