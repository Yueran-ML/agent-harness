//! Random operations of colour management: patches of the project's settings that set those that
//! apply and those that do not, spaces of the catalogue and ones that are not, numbers within
//! their ranges and out of them, and now and then what is not settings at all; and colours of
//! media set by hand, of media that is there and of media that is not.

use columbina_model::color_management::spaces;
use columbina_model::{Levels, Op, Project};
use serde_json::{json, Value};

/// The settings that a patch sets, each with values that it may take, and some that it may not.
fn values_of(setting: &str) -> Vec<Value> {
    let space = |n: usize| Value::from(spaces()[n % spaces().len()].clone());
    match setting {
        "science" => vec![
            json!("yrgb"),
            json!("yrgb_managed"),
            json!("yrgb_managed"),
            json!("acescct"),
            json!("acescc"),
            json!("legacy"),
        ],
        "automatic" | "separate_gamma" | "hdr_203_nits" | "inverse_drt" | "gamut_compress"
        | "s_curve_contrast" => vec![json!(true), json!(false)],
        "processing_mode" => vec![json!("sdr"), json!("hdr")],
        "preset" => vec![
            json!("custom"),
            json!("custom"),
            json!("sdr_rec709"),
            json!("davinci_wide_gamut"),
            json!("hdr_rec2020"),
        ],
        "output" | "input" => vec![
            space(0),
            space(19),
            space(55),
            json!("bypass"),
            json!("Log"),
        ],
        "timeline" | "idt" => vec![space(3), space(62), space(48), json!("bypass")],
        "working_luminance" => vec![
            json!("sdr_100"),
            json!("hdr_er_1000_2000"),
            json!("custom"),
            json!("hdr_10000"),
        ],
        "custom_luminance" | "graphics_white" => {
            vec![
                json!(500),
                json!(203.5),
                json!(0),
                json!(10_000),
                json!(10_001),
            ]
        }
        "input_drt" | "output_drt" => vec![
            json!("none"),
            json!("davinci"),
            json!("saturation_preserving"),
            json!("simple"),
        ],
        "input_sat_rolloff" | "output_sat_rolloff" => vec![
            json!({"start": 200}),
            json!({"limit": 50}),
            json!({"start": 300, "limit": 2000.5}),
        ],
        "limit_gamut" => vec![json!(null), json!("p3-d65"), json!("camera-native")],
        "resize_in" => vec![json!("log"), json!("linear_mapped")],
        "aces_version" => vec![json!("1.3"), json!("2.0"), json!("2.0")],
        "odt" => vec![
            json!("Rec.709"),
            json!("P3-DCI (D65 sim.)"),
            json!("Rec.2020 ST2084 (500 nits)"),
            json!("No Output Device Transform"),
        ],
        "node_luts_in" => vec![json!("timeline"), json!("ap0_linear")],
        "lut_interpolation" => vec![json!("trilinear"), json!("tetrahedral")],
        _ => vec![
            json!({"3d": "Film/print.cube"}),
            json!({"1d": null}),
            json!({"1d": "shaper.cube", "3d": null}),
            json!({"3d": "../up.cube"}),
        ],
    }
}

/// The settings, in the order of the patches.
const SETTINGS: [&str; 30] = [
    "science",
    "automatic",
    "processing_mode",
    "preset",
    "output",
    "separate_gamma",
    "input",
    "timeline",
    "working_luminance",
    "custom_luminance",
    "hdr_203_nits",
    "limit_gamut",
    "input_drt",
    "input_sat_rolloff",
    "output_drt",
    "output_sat_rolloff",
    "inverse_drt",
    "resize_in",
    "graphics_white",
    "aces_version",
    "idt",
    "odt",
    "gamut_compress",
    "node_luts_in",
    "lut_interpolation",
    "input_lut",
    "output_lut",
    "viewer_lut",
    "s_curve_contrast",
    "science",
];

/// A patch of colour settings made of `n`: from one to three settings, the science most often
/// among them so that the others come to apply; or now and then one that is not settings.
fn patch_of(n: [usize; 4]) -> Value {
    match n[0] % 32 {
        0 => return json!({"gamma": 2.4}),
        1 => return json!([]),
        2 => return json!({"science": 3}),
        _ => {}
    }
    let mut patch = serde_json::Map::new();
    for (i, &k) in n[1..=(1 + n[0] % 3)].iter().enumerate() {
        // The first setting is the science one time in three.
        let setting = if i == 0 && k % 3 == 0 {
            "science"
        } else {
            SETTINGS[k / 3 % SETTINGS.len()]
        };
        let values = values_of(setting);
        patch.insert(setting.into(), values[k / 97 % values.len()].clone());
    }
    patch.into()
}

/// An operation of colour management made of `n`.
pub fn color_op_of(project: &Project, n: [usize; 4]) -> Op {
    if !n[3].is_multiple_of(3) {
        return Op::SetColorSettings {
            settings: patch_of(n),
        };
    }
    let ids: Vec<&str> = project.media.iter().map(|m| m.id.as_str()).collect();
    let id = if ids.is_empty() || n[0].is_multiple_of(16) {
        "m99".to_owned()
    } else {
        ids[n[0] / 16 % ids.len()].to_owned()
    };
    let input = match n[1] % 6 {
        0 | 1 => None,
        2 => Some(None),
        3 => Some(Some("bypass".to_owned())),
        4 => Some(Some(spaces()[n[1] / 6 % spaces().len()].clone())),
        _ => Some(Some("Log".to_owned())),
    };
    let levels = match n[2] % 4 {
        0 => None,
        1 => Some(Levels::Auto),
        2 => Some(Levels::Video),
        _ => Some(Levels::Full),
    };
    Op::SetMediaColor { id, input, levels }
}
