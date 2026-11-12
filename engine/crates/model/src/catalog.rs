//! The settings of the picture that take keys, listed once: what each is called, the values it
//! may take, the value it has at first, the unit it is in, and where it is shown.
//!
//! A setting is named by its path in what holds it, as the checks of the model name it in their
//! errors: `primaries.lift.r` in a corrector. An item of a list is named by its place in the
//! list, and the catalog has a star in its stead: `windows.*.transform.pan`. The curve editor
//! reads the catalog to draw a setting on its scale, and an agent to know what there is to key
//! and within which values; both are told the same, since both are told from here.
//!
//! The values that a setting may take are not written a second time: an entry holds the bounds
//! that the model checks the setting against ([`crate::grade::Bounds`]), so the catalog cannot
//! say other than what is checked. That every setting of a corrector that takes keys is listed,
//! and nothing else, is what the tests of this module see to, on a corrector that has every
//! palette and a window of every shape.
//!
//! So far the catalog lists what a corrector of the Color page holds. Its keys are counted in
//! the frames of the source of the clip ([`crate::keyframes`]).

use std::sync::OnceLock;

use serde::Serialize;

use crate::grade::blur;
use crate::grade::bounds as b;
use crate::grade::window::bounds as w;
use crate::grade::Bounds;

/// What the number of a setting counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// A number as `DaVinci` shows it, of no unit.
    Number,
    /// Hundredths of a whole.
    Percent,
    /// Degrees, clockwise.
    Degrees,
    /// Pixels of a frame 1080 high.
    Pixels1080,
    /// Heights of the picture.
    Heights,
}

/// The page that shows a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Page {
    /// The Color page.
    Color,
}

/// The palette of the Color page that shows a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Palette {
    /// The Primaries palette, wheels and bars.
    Primaries,
    /// The Log mode of the Primaries palette.
    Log,
    /// The custom curves.
    Curves,
    /// Soft Clip.
    SoftClip,
    /// The Key palette.
    Key,
    /// The Blur palette.
    Blur,
    /// The adjustment of Shot Match, which no palette shows.
    ShotMatch,
    /// The Qualifier palette.
    Qualifier,
    /// The Power Window palette.
    Window,
}

/// What the keys of a setting are counted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Clock {
    /// The frames of the source of the clip, as the keys of a grade are.
    SourceFrame,
}

/// A setting that takes keys.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Entry {
    /// Its path in what holds it, with `*` for the place of an item in a list.
    pub path: String,
    /// The value it has at first, which changes nothing.
    pub default: f64,
    /// The lowest value it may take.
    pub low: f64,
    /// The highest value it may take.
    pub high: f64,
    /// What its number counts.
    pub unit: Unit,
    /// The page that shows it.
    pub page: Page,
    /// The palette that shows it.
    pub palette: Palette,
    /// What its keys are counted in.
    pub clock: Clock,
}

/// The entries of a corrector, as they are gathered.
struct Gathered(Vec<Entry>);

impl Gathered {
    fn one(&mut self, path: &str, bounds: Bounds, unit: Unit, palette: Palette) {
        self.0.push(Entry {
            path: path.to_owned(),
            default: bounds.default,
            low: bounds.low,
            high: bounds.high,
            unit,
            page: Page::Color,
            palette,
            clock: Clock::SourceFrame,
        });
    }

    /// A setting of each of `parts`, such as the channels of a wheel.
    fn each(&mut self, path: &str, parts: &[&str], bounds: Bounds, palette: Palette) {
        for part in parts {
            self.one(&format!("{path}.{part}"), bounds, Unit::Number, palette);
        }
    }
}

const YRGB: [&str; 4] = ["y", "r", "g", "b"];
const RGB: [&str; 3] = ["r", "g", "b"];

fn gathered() -> Vec<Entry> {
    use Palette::{Blur, Curves, Key, Log, Primaries, ShotMatch, SoftClip};
    use Unit::{Number, Percent};
    let mut all = Gathered(Vec::new());

    all.each("primaries.lift", &YRGB, b::LIFT, Primaries);
    all.each("primaries.gamma", &YRGB, b::GAMMA, Primaries);
    all.each("primaries.gain", &YRGB, b::GAIN, Primaries);
    all.each("primaries.offset", &RGB, b::OFFSET, Primaries);
    for (name, bounds) in [
        ("contrast", b::CONTRAST),
        ("pivot", b::PIVOT),
        ("saturation", b::SATURATION),
        ("hue", b::HUE),
        ("lum_mix", b::LUM_MIX),
        ("temperature", b::TEMPERATURE),
        ("tint", b::TINT),
        ("color_boost", b::COLOR_BOOST),
        ("midtone_detail", b::MIDTONE_DETAIL),
        ("shadows", b::SHADOWS),
        ("highlights", b::HIGHLIGHTS),
    ] {
        all.one(&format!("primaries.{name}"), bounds, Number, Primaries);
    }

    for wheel in ["shadow", "midtone", "highlight"] {
        all.each(&format!("log.{wheel}"), &RGB, b::LOG_WHEEL, Log);
    }
    all.one("log.low_range", b::LOW_RANGE, Number, Log);
    all.one("log.high_range", b::HIGH_RANGE, Number, Log);

    for curve in YRGB {
        let path = format!("curves.{curve}.intensity");
        all.one(&path, b::CURVE_INTENSITY, Percent, Curves);
    }

    all.each("soft_clip.low", &RGB, b::CLIP_LOW, SoftClip);
    all.each("soft_clip.high", &RGB, b::CLIP_HIGH, SoftClip);
    all.each("soft_clip.low_soft", &RGB, b::CLIP_SOFT, SoftClip);
    all.each("soft_clip.high_soft", &RGB, b::CLIP_SOFT, SoftClip);

    for key in ["input", "output", "qualifier"] {
        all.one(&format!("key.{key}.gain"), b::KEY_GAIN, Number, Key);
        all.one(&format!("key.{key}.offset"), b::KEY_OFFSET, Number, Key);
    }
    all.one("key.input_blur.radius", b::KEY_BLUR, Number, Key);
    all.one("key.input_blur.ratio", b::KEY_BLUR_RATIO, Number, Key);

    all.each("blur.radius", &RGB, blur::RADIUS, Blur);
    all.each("blur.ratio", &RGB, blur::RATIO, Blur);
    all.each("blur.scaling", &RGB, blur::SCALING, Blur);
    all.one("blur.coring_softness", blur::CORING_SOFTNESS, Number, Blur);
    all.one("blur.level", blur::LEVEL, Number, Blur);
    all.one("blur.mix", blur::MIX, Percent, Blur);

    all.each("shot_match.gamma", &RGB, b::SHOT_GAMMA, ShotMatch);
    all.each("shot_match.gain", &RGB, b::SHOT_GAIN, ShotMatch);
    all.each("shot_match.offset", &RGB, b::SHOT_OFFSET, ShotMatch);
    all.one("shot_match.contrast", b::SHOT_CONTRAST, Number, ShotMatch);
    all.one("shot_match.pivot", b::SHOT_PIVOT, Number, ShotMatch);

    of_keys_and_windows(&mut all);
    all.0
}

/// The settings of the Qualifier and of the windows, which say where a corrector works.
fn of_keys_and_windows(all: &mut Gathered) {
    use Palette::{Qualifier, Window};
    use Unit::{Degrees, Heights, Number, Percent, Pixels1080};
    for (name, bounds) in [
        ("center", b::HUE_CENTER),
        ("width", b::HUE_WIDTH),
        ("soft", b::HUE_SOFT),
        ("symmetry", b::HUE_SYMMETRY),
    ] {
        all.one(&format!("qualifier.hue.{name}"), bounds, Number, Qualifier);
    }
    for range in ["sat", "lum", "red", "green", "blue"] {
        for (name, bounds) in [
            ("low", b::RANGE_LOW),
            ("high", b::RANGE_HIGH),
            ("low_soft", b::RANGE_SOFT),
            ("high_soft", b::RANGE_SOFT),
        ] {
            let path = format!("qualifier.{range}.{name}");
            all.one(&path, bounds, Number, Qualifier);
        }
    }
    for (name, bounds, unit) in [
        ("pre_filter", b::PRE_FILTER, Number),
        ("clean_black", b::CLEAN, Number),
        ("black_clip", b::BLACK_CLIP, Number),
        ("clean_white", b::CLEAN, Number),
        ("white_clip", b::WHITE_CLIP, Number),
        ("blur_radius", b::KEY_BLUR, Number),
        ("in_out_ratio", b::IN_OUT_RATIO, Number),
        ("morph_radius", b::MORPH_RADIUS, Pixels1080),
    ] {
        all.one(
            &format!("qualifier.finesse.{name}"),
            bounds,
            unit,
            Qualifier,
        );
    }

    all.one("windows.*.opacity", w::OPACITY, Percent, Window);
    for (name, bounds, unit) in [
        ("size", w::SIZE, Number),
        ("aspect", w::ASPECT, Number),
        ("pan", w::PAN, Number),
        ("tilt", w::TILT, Number),
        ("rotate", w::ROTATE, Degrees),
    ] {
        all.one(&format!("windows.*.transform.{name}"), bounds, unit, Window);
    }
    // A Circular and a Gradient have one softness, a Linear one for each of its sides.
    all.one("windows.*.shape.softness", w::SOFTNESS, Number, Window);
    all.one("windows.*.shape.softness.*", w::SOFTNESS, Number, Window);
    all.one("windows.*.shape.inside", w::FEATHER, Number, Window);
    all.one("windows.*.shape.outside", w::FEATHER, Number, Window);
    // The corners of a Linear and of a Polygon; the anchors of a Curve and their two handles.
    for point in ["", ".position", ".in", ".out"] {
        for axis in ["x", "y"] {
            let path = format!("windows.*.shape.points.*{point}.{axis}");
            all.one(&path, w::COORDINATE, Heights, Window);
        }
    }
}

/// The settings of a corrector that take keys.
#[must_use]
pub fn corrector() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(gathered)
}

/// The path of the catalog that a path of a setting is listed under: with `*` in the stead of
/// each place in a list.
#[must_use]
pub fn listed_as(path: &str) -> String {
    let parts: Vec<&str> = path
        .split('.')
        .map(|part| {
            if !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()) {
                "*"
            } else {
                part
            }
        })
        .collect();
    parts.join(".")
}

/// The entry of a setting of a corrector, by its path, or none if it takes no keys.
#[must_use]
pub fn of_corrector(path: &str) -> Option<&'static Entry> {
    let listed = listed_as(path);
    corrector().iter().find(|entry| entry.path == listed)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{json, Value};

    use super::*;
    use crate::grade::window::{WindowBezierPoint, WindowPoint};
    use crate::grade::{Corrector, Qualifier, ShotMatch, Window, WindowShape};

    /// A corrector with every palette and a window of every shape.
    fn full() -> Corrector {
        let anchor = |x: f64, y: f64| WindowBezierPoint {
            position: WindowPoint::new(x, y),
            into: WindowPoint::new(-0.05, 0.0),
            out: WindowPoint::new(0.05, 0.0),
        };
        let curve = match WindowShape::curve() {
            WindowShape::Curve {
                inside, outside, ..
            } => WindowShape::Curve {
                points: vec![anchor(-0.2, -0.2), anchor(0.2, -0.2), anchor(0.0, 0.2)],
                closed: true,
                inside,
                outside,
            },
            _ => unreachable!("a curve is a curve"),
        };
        Corrector {
            qualifier: Some(Qualifier::default()),
            shot_match: Some(ShotMatch::default()),
            windows: vec![
                Window::new("w1", WindowShape::linear()),
                Window::new("w2", WindowShape::circular()),
                Window::new("w3", WindowShape::polygon()),
                Window::new("w4", curve),
                Window::new("w5", WindowShape::gradient()),
            ],
            ..Corrector::default()
        }
    }

    /// The paths of the numbers of a value, each with its number.
    fn numbers(value: &Value, path: &str, found: &mut Vec<(String, f64)>) {
        match value {
            Value::Number(number) => found.push((path.to_owned(), number.as_f64().unwrap())),
            Value::Object(fields) => {
                for (name, field) in fields {
                    let path = if path.is_empty() {
                        name.clone()
                    } else {
                        format!("{path}.{name}")
                    };
                    numbers(field, &path, found);
                }
            }
            Value::Array(items) => {
                for (place, item) in items.iter().enumerate() {
                    numbers(item, &format!("{path}.{place}"), found);
                }
            }
            _ => {}
        }
    }

    fn put(value: &mut Value, path: &str, new: Value) {
        let mut at = value;
        for part in path.split('.') {
            at = match at {
                Value::Array(items) => &mut items[part.parse::<usize>().unwrap()],
                other => &mut other[part],
            };
        }
        *at = new;
    }

    /// The corrector with a value at a path, if it can be read so.
    fn with(whole: &Value, path: &str, new: Value) -> Option<Corrector> {
        let mut changed = whole.clone();
        put(&mut changed, path, new);
        serde_json::from_value(changed).ok()
    }

    #[test]
    fn every_setting_of_a_corrector_that_takes_keys_is_listed_and_nothing_else() {
        let whole = serde_json::to_value(full()).unwrap();
        let mut found = Vec::new();
        numbers(&whole, "", &mut found);
        let mut keyed = BTreeSet::new();
        for (path, value) in &found {
            let keys = json!({"keys": [{"frame": 0, "value": value, "interp": "linear"}]});
            let takes_keys = with(&whole, path, keys).is_some();
            assert_eq!(
                of_corrector(path).is_some(),
                takes_keys,
                "{path} takes keys: {takes_keys}"
            );
            if takes_keys {
                keyed.insert(listed_as(path));
            }
        }
        let listed: BTreeSet<String> = corrector().iter().map(|e| e.path.clone()).collect();
        assert_eq!(listed, keyed);
        // And each is listed once.
        assert_eq!(listed.len(), corrector().len());
    }

    #[test]
    fn a_setting_is_at_first_what_its_entry_says_and_within_its_bounds() {
        let whole = serde_json::to_value(full()).unwrap();
        let mut found = Vec::new();
        numbers(&whole, "", &mut found);
        for (path, value) in found {
            let Some(entry) = of_corrector(&path) else {
                continue;
            };
            assert!(
                entry.low <= entry.default && entry.default <= entry.high,
                "{path}"
            );
            assert!(entry.low <= value && value <= entry.high, "{path}: {value}");
            // The corners and the handles of a window are where its shape puts them.
            if !entry.path.contains("points") {
                assert_eq!(value, entry.default, "{path}");
            }
        }
    }

    #[test]
    fn a_setting_is_refused_beyond_the_bounds_of_its_entry_and_taken_at_them() {
        let whole = serde_json::to_value(full()).unwrap();
        let mut found = Vec::new();
        numbers(&whole, "", &mut found);
        for (path, _) in found {
            let Some(entry) = of_corrector(&path) else {
                continue;
            };
            let step = (entry.high - entry.low) * 1e-9;
            for (value, taken) in [
                (entry.low, true),
                (entry.high, true),
                (entry.low - step, false),
                (entry.high + step, false),
            ] {
                let corrector = with(&whole, &path, json!(value)).unwrap();
                assert_eq!(corrector.check().is_ok(), taken, "{path} at {value}");
            }
        }
    }

    #[test]
    fn a_path_is_listed_with_a_star_for_each_place_in_a_list() {
        assert_eq!(listed_as("primaries.lift.r"), "primaries.lift.r");
        assert_eq!(
            listed_as("windows.12.shape.points.3.in.x"),
            "windows.*.shape.points.*.in.x"
        );
        let pan = of_corrector("windows.0.transform.pan").unwrap();
        assert_eq!((pan.default, pan.low, pan.high), (50.0, -1000.0, 1000.0));
        assert_eq!(
            of_corrector("windows.0.transform.rotate").unwrap().unit,
            Unit::Degrees
        );
        assert!(of_corrector("curves.y.points.0.0").is_none());
        assert!(of_corrector("windows..pan").is_none());
        assert!(of_corrector("").is_none());
    }

    #[test]
    fn an_entry_is_written_for_another_program_in_plain_words() {
        let lift = serde_json::to_value(of_corrector("primaries.lift.r").unwrap()).unwrap();
        assert_eq!(
            lift,
            json!({
                "path": "primaries.lift.r",
                "default": 0.0,
                "low": -1.0,
                "high": 1.0,
                "unit": "number",
                "page": "color",
                "palette": "primaries",
                "clock": "source_frame",
            })
        );
        let morph = of_corrector("qualifier.finesse.morph_radius").unwrap();
        assert_eq!(
            serde_json::to_value(morph.unit).unwrap(),
            json!("pixels1080")
        );
    }
}
