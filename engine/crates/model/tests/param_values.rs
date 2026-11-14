//! The values of settings with keys, and the handles of eases, as the engine finds them, written
//! out for the copies that clients keep: a curve editor draws a curve where the engine reads it,
//! to the last bit, and an ease that it sets is the ease that the engine would set.
//!
//! The file `tests/data/param-values.json` is written anew when `COLUMBINA_WRITE_EXAMPLES=1` is
//! set; otherwise it is checked to be as the engine finds the values now. It holds `values`,
//! each a setting with the frames it was asked at and the value at each, and `eases`, each a
//! row of keys, the key that was eased, the way, and the keys after it.

use std::path::Path;

use columbina_model::grade::ease::{ease, ease_of, Ease, EASE_SHARE};
use columbina_model::grade::{Interp, Key, Param};
use serde_json::{json, Value};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/param-values.json");

fn key(frame: i64, value: f64, interp: Interp) -> Key {
    Key {
        frame,
        value,
        interp,
    }
}

fn curve(out: [f64; 2], into: [f64; 2]) -> Interp {
    Interp::Bezier { out, into }
}

/// Three keys, the second eased in a way.
fn eased(way: Ease) -> Param {
    let mut keys = vec![
        key(0, 0.0, Interp::Linear),
        key(30, 12.0, Interp::Linear),
        key(90, -6.0, Interp::Linear),
    ];
    ease(&mut keys, 1, way);
    Param::Keys(keys)
}

fn settings() -> Vec<(&'static str, Param)> {
    use Interp::{Hold, Linear};
    vec![
        ("one value", Param::Fixed(0.75)),
        ("one key", Param::Keys(vec![key(12, -3.5, Linear)])),
        (
            "straight, from before frame 0",
            Param::Keys(vec![
                key(-10, 1.0, Linear),
                key(0, -2.5, Linear),
                key(24, 100.0, Linear),
            ]),
        ),
        (
            "held, then straight",
            Param::Keys(vec![
                key(0, 0.25, Hold),
                key(7, 0.5, Linear),
                key(8, 0.125, Hold),
                key(20, 1.0, Linear),
            ]),
        ),
        ("eased in", eased(Ease::In)),
        ("eased out", eased(Ease::Out)),
        ("eased in and out", eased(Ease::InOut)),
        (
            "handles of both eases as the protocol writes them",
            Param::Keys(vec![
                key(0, 10.0, curve([0.33, 0.0], [0.33, 0.0])),
                key(48, 90.0, Linear),
            ]),
        ),
        (
            "a curve beyond its keys",
            Param::Keys(vec![
                key(100, 1.0, curve([0.9, 5.0], [0.05, -3.0])),
                key(160, 2.0, curve([0.2, -40.0], [0.7, 25.5])),
                key(161, -2.0, Linear),
            ]),
        ),
        (
            "handles at the ends of their shares",
            Param::Keys(vec![
                key(0, 0.0, curve([0.0, 1.0], [1.0, 0.0])),
                key(10, 1.0, curve([1.0, 0.5], [0.0, -0.5])),
                key(20, 0.0, curve([0.0, 0.0], [0.0, 0.0])),
                key(30, 3.0, curve([1.0, 0.0], [1.0, 0.0])),
                key(40, -3.0, Linear),
            ]),
        ),
        (
            "far frames and small values",
            Param::Keys(vec![
                key(
                    1_099_511_627_776,
                    1e-9,
                    curve([EASE_SHARE, 0.0], [EASE_SHARE, 0.0]),
                ),
                key(1_099_511_627_800, 3e-9, Linear),
                key(1_099_511_628_000, -1e-9, Linear),
            ]),
        ),
    ]
}

/// The frames that a setting is asked at: from a little before its keys to a little after
/// them, in 48 steps that fall between frames, and each of its keys.
fn frames(param: &Param) -> Vec<f64> {
    let Param::Keys(keys) = param else {
        return vec![-1.5, 0.0, 7.25];
    };
    #[allow(clippy::cast_precision_loss, reason = "frames within 2^53")]
    let (first, last) = (keys[0].frame as f64, keys[keys.len() - 1].frame as f64);
    let span = (last - first).max(1.0);
    let mut at: Vec<f64> = (0..=48)
        .map(|step| first - span / 8.0 + f64::from(step) * (span * 1.25 / 48.0))
        .collect();
    #[allow(clippy::cast_precision_loss, reason = "frames within 2^53")]
    at.extend(keys.iter().map(|key| key.frame as f64));
    at
}

fn name_of(way: Ease) -> &'static str {
    match way {
        Ease::Linear => "linear",
        Ease::In => "in",
        Ease::Out => "out",
        Ease::InOut => "in_out",
    }
}

/// Rows of keys to ease, each key of each in each way.
fn rows() -> Vec<(&'static str, Vec<Key>)> {
    use Interp::{Hold, Linear};
    vec![
        (
            "three straight keys",
            vec![
                key(0, 0.0, Linear),
                key(30, 12.0, Linear),
                key(90, -6.0, Linear),
            ],
        ),
        (
            "a held stretch, a pulled handle and a level one",
            vec![
                key(0, 1.0, Hold),
                key(10, 2.0, curve([0.5, 2.0], [0.25, -1.0])),
                key(20, 0.5, Linear),
                key(40, 0.5, Linear),
                key(41, 0.75, Linear),
            ],
        ),
        ("a key alone", vec![key(5, 1.0, Linear)]),
    ]
}

fn written() -> String {
    let mut values = Vec::new();
    for (name, param) in settings() {
        let frames = frames(&param);
        let found: Vec<f64> = frames.iter().map(|frame| param.value_at(*frame)).collect();
        values.push(json!({"name": name, "param": param, "frames": frames, "values": found}));
    }
    let mut eases = Vec::new();
    for (name, keys) in rows() {
        for index in 0..keys.len() {
            let before = ease_of(&keys, index).map(name_of);
            for way in Ease::ALL {
                let mut after = keys.clone();
                ease(&mut after, index, way);
                eases.push(json!({
                    "name": name,
                    "keys": keys,
                    "index": index,
                    "is": before,
                    "ease": name_of(way),
                    "eased": after,
                    "then": ease_of(&after, index).map(name_of),
                }));
            }
        }
    }
    let lines = |cases: &[Value]| {
        let lines: Vec<String> = cases.iter().map(Value::to_string).collect();
        lines.join(",\n")
    };
    format!(
        "{{\"values\":[\n{}\n],\n\"eases\":[\n{}\n]}}\n",
        lines(&values),
        lines(&eases)
    )
}

#[test]
fn the_values_of_keys_are_written_as_the_engine_finds_them() {
    let text = written();
    if std::env::var("COLUMBINA_WRITE_EXAMPLES").as_deref() == Ok("1") {
        std::fs::write(FILE, &text).unwrap();
        return;
    }
    let kept = std::fs::read_to_string(Path::new(FILE)).unwrap_or_default();
    let read = |text: &str| serde_json::from_str::<Value>(text).ok();
    assert!(
        read(&kept).is_some() && read(&kept) == read(&text),
        "{FILE} is not as the engine finds the values of keys now: run the tests of \
         columbina-model with COLUMBINA_WRITE_EXAMPLES=1 to write it anew"
    );
}

#[test]
fn what_is_written_is_read_back_to_the_bit() {
    let read: Value = serde_json::from_str(&written()).unwrap();
    let cases = read["values"].as_array().unwrap();
    assert_eq!(cases.len(), settings().len());
    for (case, (name, param)) in cases.iter().zip(settings()) {
        let again: Param = serde_json::from_value(case["param"].clone()).unwrap();
        assert_eq!(again, param, "{name}");
        let frames = case["frames"].as_array().unwrap();
        let values = case["values"].as_array().unwrap();
        assert!(frames.len() >= 3 && frames.len() == values.len(), "{name}");
        for (frame, value) in frames.iter().zip(values) {
            let (frame, value) = (frame.as_f64().unwrap(), value.as_f64().unwrap());
            assert_eq!(
                param.value_at(frame).to_bits(),
                value.to_bits(),
                "{name} at {frame}"
            );
        }
    }
    // Every key of every row in every way.
    let rows: usize = rows().iter().map(|(_, keys)| keys.len()).sum();
    assert_eq!(
        read["eases"].as_array().unwrap().len(),
        rows * Ease::ALL.len()
    );
}
