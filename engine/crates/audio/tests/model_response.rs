//! Static curves use the same model conversion and DSP as the graph; no device or GPU.
use columbina_audio::fx::{dynamics, eq};
use columbina_audio::graph::{dynamics_curve, eq_response};
use columbina_model::audio::fx::{Dynamics, Equalizer};

#[test]
fn model_eq_response_has_bypass_clip_strip_and_gain() {
    for (mut model, variant) in [
        (Equalizer::clip(), eq::Variant::Clip),
        (Equalizer::strip(), eq::Variant::Strip),
    ] {
        let frequencies = [20.0, 80.0, 300.0, 1000.0, 16000.0];
        assert_eq!(eq_response(&model, 48000, &frequencies), vec![0.0; 5]);
        model.on = true;
        model.gain_db = 3.0;
        let mut settings = eq::Settings::of(variant);
        settings.on = true;
        settings.gain_db = 3.0;
        assert_eq!(
            eq_response(&model, 48000, &frequencies),
            eq::response(&settings, 48000, &frequencies)
        );
        model.bands[0].on = true;
        model.bands[0].slope = 24;
        settings.bands[0].on = true;
        settings.bands[0].slope = 24;
        assert_eq!(
            eq_response(&model, 32000, &frequencies),
            eq::response(&settings, 32000, &frequencies)
        );
    }
}

#[test]
fn model_dynamics_curve_reuses_mix_makeup_gate_and_bypass() {
    let mut model = Dynamics::default();
    let inputs = [-60.0, -40.0, -20.0, 0.0];
    assert_eq!(dynamics_curve(&model, &inputs), inputs);
    model.on = true;
    model.compressor.on = true;
    model.compressor.mix = 100.0;
    model.makeup_db = 4.0;
    assert_eq!(
        dynamics_curve(&model, &inputs),
        vec![-56.0, -36.0, -16.0, 4.0]
    );
    model.compressor.mix = 0.0;
    let mut settings = dynamics::Settings {
        on: true,
        ..dynamics::Settings::default()
    };
    settings.compressor.on = true;
    settings.makeup_db = 4.0;
    assert_eq!(
        dynamics_curve(&model, &inputs),
        dynamics::curve(&settings, &inputs)
    );
    model.expander.on = true;
    model.expander.mode = columbina_model::audio::fx::GateMode::Gate;
    settings.expander.on = true;
    settings.expander.gate = true;
    assert_eq!(
        dynamics_curve(&model, &inputs),
        dynamics::curve(&settings, &inputs)
    );
}
