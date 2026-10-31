//! The processor of the crate against references outside it:
//! - OCIO's Python bindings (`tests/reference/make.py`, `PyOpenColorIO` 2.6.0): every space of the
//!   catalogue to ACES2065-1, from it, and to the ACES 2.0 SDR view of a Rec.709 display;
//! - the F-Log2 Data Sheet's own formula, for the space we wrote;
//! - the studio config, for our way of writing a gamut and a gamma, on the spaces it has too.

#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use std::collections::BTreeSet;

use columbina_model::{Gamma, Gamut, InputSpace};
use columbina_ocio::catalogue::{self, flog2};
use columbina_ocio::{Config, Direction};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reference");

fn close(a: [f32; 3], b: [f64; 3], tolerance: f64) -> bool {
    (0..3).all(|c| {
        let (a, b) = (f64::from(a[c]), b[c]);
        (a.is_nan() && b.is_nan()) || (a - b).abs() <= tolerance * b.abs().max(1.0)
    })
}

fn triple(value: &serde_json::Value) -> [f64; 3] {
    let v = value.as_array().unwrap();
    [0, 1, 2].map(|c| v[c].as_f64().unwrap_or(f64::NAN))
}

/// The names of OCIO's spaces that the catalogue uses, each once, in its order.
fn names() -> Vec<String> {
    let mut seen = BTreeSet::new();
    catalogue::spaces()
        .iter()
        .filter(|s| seen.insert(s.ocio.clone()))
        .map(|s| s.ocio.clone())
        .collect()
}

#[test]
fn the_list_of_spaces_for_the_reference_is_the_catalogue() {
    let path = format!("{DIR}/spaces.txt");
    let written = names().join("\n") + "\n";
    if std::env::var_os("COLUMBINA_WRITE_FRAGMENT").is_some() {
        std::fs::write(&path, &written).unwrap();
    }
    let kept = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        kept == written,
        "tests/reference/spaces.txt is not the catalogue: run with COLUMBINA_WRITE_FRAGMENT=1, \
         then make.py"
    );
}

#[test]
fn every_space_is_what_pyopencolorio_makes_of_it() {
    let text = std::fs::read_to_string(format!("{DIR}/reference.json")).unwrap();
    let reference: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        reference["ocio"].as_str().unwrap(),
        columbina_ocio::version()
    );
    let config = Config::columbina().unwrap();
    assert_eq!(reference["config"].as_str().unwrap(), config.name());
    let aces_inputs: Vec<[f32; 3]> = reference["aces_inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| triple(v).map(|x| x as f32))
        .collect();
    let mut failed = Vec::new();
    let mut compared = 0;
    for name in names() {
        let entry = &reference["spaces"][&name];
        assert!(
            !entry.is_null(),
            "{name} is not in reference.json: run make.py"
        );
        let inputs: Vec<[f32; 3]> = entry["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| triple(v).map(|x| x as f32))
            .collect();
        let display = config
            .display_view(
                &name,
                "Rec.1886 Rec.709 - Display",
                "ACES 2.0 - SDR 100 nits (Rec.709)",
                Direction::Forward,
            )
            .unwrap();
        for (what, processor, inputs) in [
            (
                "to_aces",
                config.processor(&name, "ACES2065-1").unwrap(),
                &inputs,
            ),
            (
                "from_aces",
                config.processor("ACES2065-1", &name).unwrap(),
                &aces_inputs,
            ),
            ("to_display", display, &inputs),
        ] {
            let mut ours = inputs.clone();
            processor.apply(&mut ours).unwrap();
            for (i, expected) in entry[what].as_array().unwrap().iter().enumerate() {
                compared += 1;
                if !close(ours[i], triple(expected), 1e-5) {
                    failed.push(format!(
                        "{name} {what} {:?}: ours {:?}, PyOpenColorIO {:?}",
                        inputs[i],
                        ours[i],
                        triple(expected)
                    ));
                }
            }
        }
    }
    eprintln!("{compared} colours compared");
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn flog2_is_the_curve_of_its_data_sheet() {
    let config = Config::columbina().unwrap();
    // F-Gamut has the primaries and white of BT.2020: grey in it is grey in Linear Rec.2020.
    let processor = config
        .processor(catalogue::FLOG2, "Linear Rec.2020")
        .unwrap();
    for x in [0.0, 0.0005, 0.000_889, 0.002, 0.01, 0.18, 0.9, 4.0, 16.0] {
        let code = flog2::encode(x);
        #[allow(clippy::cast_possible_truncation)]
        let linear = processor.apply_one([code as f32; 3]).unwrap();
        for value in linear {
            assert!(
                (f64::from(value) - x).abs() <= 2e-5 * x.max(0.01),
                "F-Log2 {code} is {value}, the data sheet's {x}"
            );
        }
    }
    // 18% grey at 39.1%, as the data sheet has it.
    assert!((flog2::encode(0.18) - 0.391).abs() < 5e-4);
}

#[test]
fn our_spaces_are_written_as_the_studio_config_writes_its_own() {
    // The spaces of the media that the studio config has, written our way too.
    let shared: Vec<InputSpace> = [
        (Gamut::Rec709, Gamma::Rec709),
        (Gamut::Rec709, Gamma::Gamma22),
        (Gamut::Rec709, Gamma::Srgb),
        (Gamut::Rec709, Gamma::Linear),
        (Gamut::Rec2020, Gamma::Linear),
        (Gamut::Rec2020, Gamma::St2084),
        (Gamut::Rec2020, Gamma::Hlg),
        (Gamut::P3D65, Gamma::Linear),
        (Gamut::P3D65, Gamma::Srgb),
        (Gamut::P3D65, Gamma::St2084),
    ]
    .into_iter()
    .map(|(gamut, gamma)| InputSpace { gamut, gamma })
    .collect();
    let mut text = catalogue::fragment_of(shared.iter().copied());
    for input in &shared {
        let ours = catalogue::our_name(*input);
        text = text.replace(
            &format!("name: {ours}\n"),
            &format!("name: {ours} (ours)\n"),
        );
    }
    text = text.replace(
        &format!("name: {}\n", catalogue::FLOG2),
        &format!("name: {} (ours)\n", catalogue::FLOG2),
    );
    text = text.replace(
        &format!("colorspace: {}}}", catalogue::FLOG2),
        &format!("colorspace: {} (ours)}}", catalogue::FLOG2),
    );
    let config = Config::columbina()
        .unwrap()
        .extend(&Config::from_text(&text, "").unwrap())
        .unwrap();
    let encoded: Vec<[f32; 3]> = (0..=20)
        .map(|i| {
            let v = i as f32 / 20.0;
            [v, (v * 0.7 + 0.1).min(1.0), v * v]
        })
        .collect();
    for input in &shared {
        let studio = catalogue::for_input(*input).unwrap().ocio.clone();
        let ours = format!("{} (ours)", catalogue::our_name(*input));
        let mut a = encoded.clone();
        let mut b = encoded.clone();
        config
            .processor(&studio, "ACES2065-1")
            .unwrap()
            .apply(&mut a)
            .unwrap();
        config
            .processor(&ours, "ACES2065-1")
            .unwrap()
            .apply(&mut b)
            .unwrap();
        for (x, y) in a.iter().zip(&b) {
            assert!(
                close(*y, x.map(f64::from), 2e-5),
                "{studio}: studio {x:?}, ours {y:?}"
            );
        }
    }
}
