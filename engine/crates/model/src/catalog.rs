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
//! The catalog lists what the nodes of the Color page hold: a corrector, and a Key Mixer,
//! whose inputs and output each have a gain and an offset; and what a clip holds of the Edit
//! page: the settings of its transform and of its crop, and, of a clip that is a title, the
//! size of its letters, the place of its anchor and the lengths of its stroke, its shadow and
//! its box, where it has them ([`clip`]). The keys of all of them are
//! counted in the
//! frames of the source of the clip ([`crate::keyframes`]). The operations that set the keys of
//! a setting by its path take the paths that are listed: [`crate::ops::Op::SetKeys`] those of a
//! node, [`crate::ops::Op::SetClipKeys`] those of a clip.
//!
//! The place and the turn of a clip may be any finite numbers in the model, as they were before
//! they took keys; for them the catalog gives the values that the editors show, 16 pictures
//! either way and ten turns, and for the scale the least that the Inspector sets, a hundredth.
//! The anchor of a title may be anywhere too, and is given as far either way, so that a title
//! may be keyed from beyond an edge of the picture to beyond the other. A title has no size
//! that changes nothing: its entry gives the size that the clients give a new title, 72. Nor
//! is there a stroke, a shadow or a box that changes nothing, since a title without one has
//! none: their entries give the lengths that the clients give a new one, a stroke 4 pixels
//! wide, a shadow moved 4 right and 4 down and blurred by 4, a box 16 beyond the lines with
//! corners of 8.
//!
//! And it lists what the nodes of the composition of a clip hold, of the Fusion page
//! ([`fusion`]): the settings of a Merge, of a Transform, of a Background, of the masks (a
//! Rectangle, an Ellipse and a Polygon), of a Blur, of a Mosaic, of a Delta Keyer and of a
//! Text+, in Fusion's own units ([`crate::fusion`]), each with the bounds that the model checks
//! it against.
//! [`crate::ops::Op::SetFusionKeys`] takes their paths, after the id of the node.

use std::sync::OnceLock;

use serde::Serialize;

use crate::grade::blur;
use crate::grade::bounds as b;
use crate::grade::window::bounds as w;
use crate::grade::{Bounds, Kind};

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
    /// Degrees, counter-clockwise, as the Fusion page counts an angle.
    DegreesCounterclockwise,
    /// Pixels of a frame 1080 high.
    Pixels1080,
    /// Heights of the picture.
    Heights,
    /// Widths of the picture.
    Widths,
    /// Pixels of the timeline; and, of a setting of a node of a composition, pixels of the
    /// pictures of the composition.
    Pixels,
}

/// The page that shows a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Page {
    /// The Color page.
    Color,
    /// The Edit page, and the Cut page with it.
    Edit,
    /// The Fusion page.
    Fusion,
}

/// The palette of the Color page, or the group of the Inspector of the Edit page, that shows a
/// setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Palette {
    /// The Primaries palette, wheels and bars.
    Primaries,
    /// The Log mode of the Primaries palette.
    Log,
    /// The custom curves.
    Curves,
    /// The HSL curves.
    HslCurves,
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
    /// The Transform of the Inspector.
    Transform,
    /// The Composite of the Inspector.
    Composite,
    /// The Cropping of the Inspector.
    Cropping,
    /// The Title of the Inspector, which a clip that is a title has.
    Title,
    /// The Inspector of the Fusion page, which shows the settings of a node of a composition.
    Fusion,
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
    use Palette::{Blur, Curves, HslCurves, Key, Log, Primaries, ShotMatch, SoftClip};
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
        // A point of a curve is where it is, `in` and `out`, and has no value that it is at first.
        all.each(
            &format!("curves.{curve}.points.*"),
            &["0", "1"],
            b::CURVE_POINT,
            Curves,
        );
    }
    for curve in [
        "hue_vs_hue",
        "hue_vs_sat",
        "hue_vs_lum",
        "lum_vs_sat",
        "sat_vs_sat",
        "sat_vs_lum",
    ] {
        all.one(
            &format!("hsl.{curve}.*.0"),
            b::CURVE_POINT,
            Number,
            HslCurves,
        );
        all.one(&format!("hsl.{curve}.*.1"), b::HSL_OUT, Number, HslCurves);
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

/// The settings of a Key Mixer that take keys.
#[must_use]
pub fn key_mixer() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let mut all = Gathered(Vec::new());
        for of in ["inputs.*", "output"] {
            let gain = format!("{of}.gain");
            all.one(&gain, b::KEY_GAIN, Unit::Number, Palette::Key);
            let offset = format!("{of}.offset");
            all.one(&offset, b::KEY_OFFSET, Unit::Number, Palette::Key);
        }
        all.0
    })
}

/// The settings of a clip that take keys: those of its transform and of its crop, named as the
/// clip holds them, `transform.x`, `crop.left`; and those of the style of a title, which a clip
/// that is a title has, named under `title`: `title.size`.
#[must_use]
pub fn clip() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        use crate::project::{TitleBackground, TitleShadow, TitleStroke};
        let one = |path: &str, bounds: [f64; 3], unit, palette| Entry {
            path: path.to_owned(),
            default: bounds[0],
            low: bounds[1],
            high: bounds[2],
            unit,
            page: Page::Edit,
            palette,
            clock: Clock::SourceFrame,
        };
        let most = crate::project::Transform::MOST_SCALE;
        let largest = crate::project::TitleStyle::MOST_SIZE;
        let (place, share) = ([0.0, -16.0, 16.0], [0.0, 0.0, 1.0]);
        let anchor = [0.5, -16.0, 16.0];
        let shifted = TitleShadow::MOST_OFFSET;
        // A length of a stroke, a shadow or a box of a title, at first what the clients give a
        // new one.
        let title_part = |path: &str, bounds| one(path, bounds, Unit::Pixels, Palette::Title);
        vec![
            one("transform.x", place, Unit::Widths, Palette::Transform),
            one("transform.y", place, Unit::Heights, Palette::Transform),
            one(
                "transform.scale",
                [1.0, 0.01, most],
                Unit::Number,
                Palette::Transform,
            ),
            one(
                "transform.rotation",
                [0.0, -3600.0, 3600.0],
                Unit::Degrees,
                Palette::Transform,
            ),
            one(
                "transform.opacity",
                [1.0, 0.0, 1.0],
                Unit::Number,
                Palette::Composite,
            ),
            one("crop.left", share, Unit::Widths, Palette::Cropping),
            one("crop.right", share, Unit::Widths, Palette::Cropping),
            one("crop.top", share, Unit::Heights, Palette::Cropping),
            one("crop.bottom", share, Unit::Heights, Palette::Cropping),
            one("crop.softness", share, Unit::Heights, Palette::Cropping),
            one(
                "title.size",
                [72.0, 1.0, largest],
                Unit::Pixels,
                Palette::Title,
            ),
            one("title.x", anchor, Unit::Widths, Palette::Title),
            one("title.y", anchor, Unit::Heights, Palette::Title),
            title_part("title.stroke.width", [4.0, 0.0, TitleStroke::MOST_WIDTH]),
            title_part("title.shadow.x", [4.0, -shifted, shifted]),
            title_part("title.shadow.y", [4.0, -shifted, shifted]),
            title_part("title.shadow.blur", [4.0, 0.0, TitleShadow::MOST_BLUR]),
            title_part(
                "title.background.padding",
                [16.0, 0.0, TitleBackground::MOST_PADDING],
            ),
            title_part(
                "title.background.radius",
                [8.0, 0.0, TitleBackground::MOST_RADIUS],
            ),
        ]
    })
}

/// The entries of settings of a node of a composition, each given by its path, its bounds
/// and its unit.
fn fused(settings: &[(&str, Bounds, Unit)]) -> Vec<Entry> {
    settings
        .iter()
        .map(|&(path, bounds, unit)| Entry {
            path: path.to_owned(),
            default: bounds.default,
            low: bounds.low,
            high: bounds.high,
            unit,
            page: Page::Fusion,
            palette: Palette::Fusion,
            clock: Clock::SourceFrame,
        })
        .collect()
}

/// The settings of a mask node of a composition that take keys: the controls that every mask
/// has and where its middle is, and after them `shape`, those of its own shape. A length is
/// in widths of the picture, but for the Height of a Rectangle.
fn masked(shape: &[(&str, Bounds, Unit)]) -> Vec<Entry> {
    use crate::fusion::bounds as f;
    let mut all = fused(&[
        ("level", f::MASK_LEVEL, Unit::Number),
        ("soft_edge", f::MASK_SOFT_EDGE, Unit::Widths),
        ("border_width", f::MASK_BORDER_WIDTH, Unit::Widths),
        ("center.x", f::MASK_CENTER, Unit::Widths),
        ("center.y", f::MASK_CENTER, Unit::Heights),
    ]);
    all.extend(fused(shape));
    all
}

/// The settings of a mask node of a composition of a kind that take keys, in the order of
/// its `params`. The two numbers of a point of a Polygon are listed once, with a star for the
/// place of the point: `points.*.x`.
fn mask(kind: crate::fusion::Kind) -> &'static [Entry] {
    use crate::fusion::bounds as f;
    use crate::fusion::Kind;
    use Unit::{DegreesCounterclockwise, Heights, Number, Widths};
    static RECTANGLE: OnceLock<Vec<Entry>> = OnceLock::new();
    static ELLIPSE: OnceLock<Vec<Entry>> = OnceLock::new();
    static POLYGON: OnceLock<Vec<Entry>> = OnceLock::new();
    let angle = ("angle", f::MASK_ANGLE, DegreesCounterclockwise);
    match kind {
        Kind::Rectangle => RECTANGLE.get_or_init(|| {
            masked(&[
                ("width", f::MASK_EXTENT, Widths),
                ("height", f::MASK_EXTENT, Heights),
                ("corner_radius", f::MASK_CORNER_RADIUS, Number),
                angle,
            ])
        }),
        // The height of an ellipse is in widths, as its width is: alike, they are a circle.
        Kind::Ellipse => ELLIPSE.get_or_init(|| {
            masked(&[
                ("width", f::MASK_EXTENT, Widths),
                ("height", f::MASK_EXTENT, Widths),
                angle,
            ])
        }),
        Kind::Polygon => POLYGON.get_or_init(|| {
            masked(&[
                ("size", f::MASK_SIZE, Number),
                angle,
                ("points.*.x", f::MASK_POINT, Widths),
                ("points.*.y", f::MASK_POINT, Widths),
            ])
        }),
        Kind::MediaIn
        | Kind::MediaOut
        | Kind::Merge
        | Kind::Transform
        | Kind::Background
        | Kind::Blur
        | Kind::Mosaic
        | Kind::DeltaKeyer
        | Kind::Tracker
        | Kind::PlanarTracker
        | Kind::TextPlus => &[],
    }
}

/// The settings of a Blur or of a Mosaic of a composition that take keys, in the order of
/// its `params`. A Blur Size is in pixels of a picture 1080 high, as the filters of the Color
/// page are, and the offsets of a Mosaic are in pixels of the picture.
fn filter(kind: crate::fusion::Kind) -> &'static [Entry] {
    use crate::fusion::bounds as f;
    use crate::fusion::Kind;
    use Unit::{Number, Pixels, Pixels1080};
    static BLUR: OnceLock<Vec<Entry>> = OnceLock::new();
    static MOSAIC: OnceLock<Vec<Entry>> = OnceLock::new();
    match kind {
        Kind::Blur => BLUR.get_or_init(|| {
            fused(&[
                ("size.x", f::BLUR_SIZE, Pixels1080),
                ("size.y", f::BLUR_SIZE, Pixels1080),
                ("blend", f::BLUR_BLEND, Number),
            ])
        }),
        Kind::Mosaic => MOSAIC.get_or_init(|| {
            fused(&[
                ("frequency", f::MOSAIC_FREQUENCY, Number),
                ("aspect", f::MOSAIC_ASPECT, Number),
                ("offset.x", f::MOSAIC_OFFSET, Pixels),
                ("offset.y", f::MOSAIC_OFFSET, Pixels),
            ])
        }),
        Kind::MediaIn
        | Kind::MediaOut
        | Kind::Merge
        | Kind::Transform
        | Kind::Background
        | Kind::Rectangle
        | Kind::Ellipse
        | Kind::Polygon
        | Kind::DeltaKeyer
        | Kind::Tracker
        | Kind::PlanarTracker
        | Kind::TextPlus => &[],
    }
}

/// The settings of a Delta Keyer of a composition that take keys, in the order of its
/// `params`. Its Pre-Blur, its Erode/Dilate and its Matte Blur are in pixels of a picture
/// 1080 high, as a Blur Size is; the channels of its colours are numbers from 0, as those of
/// a picture are.
fn keyer() -> &'static [Entry] {
    use crate::fusion::bounds as f;
    use Unit::{Number, Pixels1080};
    static KEYER: OnceLock<Vec<Entry>> = OnceLock::new();
    KEYER.get_or_init(|| {
        fused(&[
            ("screen.r", f::KEYER_SCREEN, Number),
            ("screen.g", f::KEYER_SCREEN_GREEN, Number),
            ("screen.b", f::KEYER_SCREEN, Number),
            ("gain", f::KEYER_GAIN, Number),
            ("balance", f::KEYER_BALANCE, Number),
            ("pre_blur", f::KEYER_BLUR, Pixels1080),
            ("low", f::KEYER_LOW, Number),
            ("high", f::KEYER_HIGH, Number),
            ("erode_dilate", f::KEYER_ERODE_DILATE, Pixels1080),
            ("matte_blur", f::KEYER_BLUR, Pixels1080),
            ("clean_foreground", f::KEYER_CLEAN, Number),
            ("clean_background", f::KEYER_CLEAN, Number),
            ("replace_color.r", f::KEYER_REPLACE_COLOR, Number),
            ("replace_color.g", f::KEYER_REPLACE_COLOR, Number),
            ("replace_color.b", f::KEYER_REPLACE_COLOR, Number),
            ("spill_suppression", f::KEYER_SPILL_SUPPRESSION, Number),
        ])
    })
}

/// The settings of a Text+ of a composition that may take keys, in the order of its `params`
/// ([`crate::fusion::TextPlus::SETTINGS`]): those of its outline, of its border and of its
/// follower too, which are its settings only where it has those. The size of its letters, the
/// offsets and how far its border reaches are in heights of the picture; the thickness of its
/// outline (in ems of its letters), its tracking (in ems), its line spacing (in lines of its
/// font), its write-on, its scales and its colours are numbers.
fn text_plus() -> &'static [Entry] {
    use crate::fusion::TextPlus;
    use Unit::{DegreesCounterclockwise, Heights, Number, Widths};
    static TEXT_PLUS: OnceLock<Vec<Entry>> = OnceLock::new();
    TEXT_PLUS.get_or_init(|| {
        let listed: Vec<(&str, Bounds, Unit)> = TextPlus::SETTINGS
            .iter()
            .map(|&(path, _, bounds)| {
                let unit = match path {
                    "center.x" => Widths,
                    "size" | "center.y" | "transform.offset.x" | "transform.offset.y"
                    | "border.extend" | "follower.offset.x" | "follower.offset.y" => Heights,
                    "angle" | "transform.angle" | "follower.angle" => DegreesCounterclockwise,
                    _ => Number,
                };
                (path, bounds, unit)
            })
            .collect();
        fused(&listed)
    })
}

/// The settings of a node of a composition of a kind that take keys
/// ([`crate::fusion`]), each named by its path in the `params` of the node: `center.x`. A
/// `media_in` and a `media_out` have none, and neither has a `tracker` nor a
/// `planar_tracker`, whose numbers take no keys: their paths are what moves them. A point of
/// a Polygon is listed with a star for its place in the list of points: `points.*.x`. A
/// `text_plus` has those of its outline, of its border and of its follower listed whether it
/// has them or not.
#[must_use]
pub fn fusion(kind: crate::fusion::Kind) -> &'static [Entry] {
    use crate::fusion::bounds as f;
    use crate::fusion::Kind;
    use Unit::{DegreesCounterclockwise, Heights, Number, Widths};
    static MERGE: OnceLock<Vec<Entry>> = OnceLock::new();
    static TRANSFORM: OnceLock<Vec<Entry>> = OnceLock::new();
    static BACKGROUND: OnceLock<Vec<Entry>> = OnceLock::new();
    // Where the middle of a picture is put, how large it is made and how far it is turned.
    let placed = [
        ("center.x", f::CENTER, Widths),
        ("center.y", f::CENTER, Heights),
        ("size", f::SIZE, Number),
        ("angle", f::ANGLE, DegreesCounterclockwise),
    ];
    match kind {
        Kind::MediaIn | Kind::MediaOut | Kind::Tracker | Kind::PlanarTracker => &[],
        Kind::Merge => MERGE.get_or_init(|| {
            let mut all = fused(&[
                ("subtractive_additive", f::SUBTRACTIVE_ADDITIVE, Number),
                ("alpha_gain", f::ALPHA_GAIN, Number),
                ("burn_in", f::BURN_IN, Number),
                ("blend", f::BLEND, Number),
            ]);
            all.extend(fused(&placed));
            all
        }),
        Kind::Transform => TRANSFORM.get_or_init(|| fused(&placed)),
        Kind::Background => BACKGROUND.get_or_init(|| {
            fused(&[
                ("color.r", f::COLOR, Number),
                ("color.g", f::COLOR, Number),
                ("color.b", f::COLOR, Number),
                ("color.a", f::ALPHA, Number),
            ])
        }),
        Kind::Rectangle | Kind::Ellipse | Kind::Polygon => mask(kind),
        Kind::Blur | Kind::Mosaic => filter(kind),
        Kind::DeltaKeyer => keyer(),
        Kind::TextPlus => text_plus(),
    }
}

/// The entry of a setting of a node of a composition of a kind, by its path in the `params`
/// of the node, or none if it takes no keys. A point of a Polygon is found by its place:
/// `points.2.x`.
#[must_use]
pub fn of_fusion(kind: crate::fusion::Kind, path: &str) -> Option<&'static Entry> {
    fusion(kind).iter().find(|entry| lists(&entry.path, path))
}

/// The entry of a setting of a clip, by its path, or none if it takes no keys.
#[must_use]
pub fn of_clip(path: &str) -> Option<&'static Entry> {
    clip().iter().find(|entry| entry.path == path)
}

/// The settings of a node of a kind that take keys: none for a mixer that has only inputs.
#[must_use]
pub fn of_kind(kind: Kind) -> &'static [Entry] {
    match kind {
        Kind::Corrector => corrector(),
        Kind::KeyMixer => key_mixer(),
        Kind::ParallelMixer | Kind::LayerMixer => &[],
    }
}

/// The entry of a setting of a node of a kind, by its path, or none if it takes no keys.
#[must_use]
pub fn of_node(kind: Kind, path: &str) -> Option<&'static Entry> {
    of_kind(kind).iter().find(|entry| lists(&entry.path, path))
}

/// Whether a path of the catalog lists a path of a setting: part by part, a `*` of the catalog
/// standing for a place in a list, a number of any length, and every other part for itself. A
/// number that the catalog writes out is itself too: the `0` and the `1` of a point of a curve
/// are its `in` and its `out`, and not places in a list.
#[must_use]
pub fn lists(listed: &str, path: &str) -> bool {
    let (mut listed, mut parts) = (listed.split('.'), path.split('.'));
    loop {
        match (listed.next(), parts.next()) {
            (None, None) => return true,
            (Some("*"), Some(part))
                if !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()) => {}
            (Some(named), Some(part)) if named == part => {}
            _ => return false,
        }
    }
}

/// The entry of a setting of a corrector, by its path, or none if it takes no keys.
#[must_use]
pub fn of_corrector(path: &str) -> Option<&'static Entry> {
    of_node(Kind::Corrector, path)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{json, Value};

    use super::*;
    use crate::grade::window::{WindowBezierPoint, WindowPoint};
    use crate::grade::{
        fixed_points, Corrector, HslCurves, KeyMixer, NodeKind, Qualifier, ShotMatch, Window,
        WindowShape,
    };

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
            hsl: HslCurves {
                hue_vs_hue: fixed_points(&[[0.1, 0.5]]),
                hue_vs_sat: fixed_points(&[[0.2, -0.5]]),
                hue_vs_lum: fixed_points(&[[0.3, 0.25]]),
                lum_vs_sat: fixed_points(&[[0.4, -0.25]]),
                sat_vs_sat: fixed_points(&[[0.5, 0.75]]),
                sat_vs_lum: fixed_points(&[[0.6, -0.75]]),
            },
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
                keyed.insert(of_corrector(path).unwrap().path.clone());
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
            // The corners and the handles of a window are where its shape puts them, and the
            // points of a curve where somebody put them.
            if !entry.path.contains("points") && !entry.path.starts_with("hsl.") {
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
            // The `in` of a point of a custom curve is also held between its neighbours.
            let between = entry.path.starts_with("curves.") && entry.path.ends_with("points.*.0");
            for (value, taken) in [
                (entry.low, true),
                (entry.high, true),
                (entry.low - step, false),
                (entry.high + step, false),
            ] {
                let corrector = with(&whole, &path, json!(value)).unwrap();
                if taken && between {
                    continue;
                }
                assert_eq!(corrector.check().is_ok(), taken, "{path} at {value}");
            }
        }
    }

    #[test]
    fn every_setting_of_a_key_mixer_that_takes_keys_is_listed_and_a_mixer_of_pictures_has_none() {
        let whole = NodeKind::KeyMixer(KeyMixer::default()).params();
        let mut found = Vec::new();
        numbers(&whole, "", &mut found);
        let mut keyed = BTreeSet::new();
        for (path, value) in &found {
            let keys = json!({"keys": [{"frame": 0, "value": value, "interp": "linear"}]});
            let mut changed = whole.clone();
            put(&mut changed, path, keys);
            let takes_keys = NodeKind::from_params(Kind::KeyMixer, changed).is_ok();
            assert_eq!(
                of_node(Kind::KeyMixer, path).is_some(),
                takes_keys,
                "{path}"
            );
            if takes_keys {
                keyed.insert(of_node(Kind::KeyMixer, path).unwrap().path.clone());
            }
        }
        let listed: BTreeSet<String> = key_mixer().iter().map(|e| e.path.clone()).collect();
        assert_eq!(listed, keyed);
        assert_eq!(listed.len(), 4);
        // A setting is listed for the kind of node that has it.
        assert!(of_node(Kind::KeyMixer, "inputs.7.gain").is_some());
        assert!(of_node(Kind::Corrector, "inputs.7.gain").is_none());
        assert!(of_node(Kind::KeyMixer, "primaries.lift.r").is_none());
        for kind in [Kind::ParallelMixer, Kind::LayerMixer] {
            assert_eq!(of_kind(kind), &[] as &[Entry]);
            assert!(of_node(kind, "inputs").is_none());
        }
    }

    #[test]
    fn a_path_is_listed_with_a_star_for_each_place_in_a_list() {
        assert_eq!(
            of_corrector("primaries.lift.r").unwrap().path,
            "primaries.lift.r"
        );
        assert_eq!(
            of_corrector("windows.12.shape.points.3.in.x").unwrap().path,
            "windows.*.shape.points.*.in.x"
        );
        assert!(lists("windows.*.opacity", "windows.007.opacity"));
        assert!(!lists("windows.*.opacity", "windows.first.opacity"));
        assert!(!lists("windows.*.opacity", "windows..opacity"));
        assert!(!lists("windows.*.opacity", "windows.0.opacity.more"));
        assert!(!lists("windows.*.opacity", "windows.0"));
        let pan = of_corrector("windows.0.transform.pan").unwrap();
        assert_eq!((pan.default, pan.low, pan.high), (50.0, -1000.0, 1000.0));
        assert_eq!(
            of_corrector("windows.0.transform.rotate").unwrap().unit,
            Unit::Degrees
        );
        // A point of a curve has two numbers, and what is no number takes no keys.
        assert!(of_corrector("curves.y.points.0.0").is_some());
        assert!(of_corrector("curves.y.points.0.2").is_none());
        assert!(of_corrector("curves.ganged").is_none());
        let out = of_corrector("hsl.sat_vs_lum.3.1").unwrap();
        assert_eq!(
            (out.low, out.high, out.palette),
            (-1.0, 1.0, Palette::HslCurves)
        );
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

    /// The kinds of node of a composition.
    const FUSION_KINDS: [crate::fusion::Kind; 14] = [
        crate::fusion::Kind::Tracker,
        crate::fusion::Kind::PlanarTracker,
        crate::fusion::Kind::TextPlus,
        crate::fusion::Kind::MediaIn,
        crate::fusion::Kind::MediaOut,
        crate::fusion::Kind::Merge,
        crate::fusion::Kind::Transform,
        crate::fusion::Kind::Background,
        crate::fusion::Kind::Rectangle,
        crate::fusion::Kind::Ellipse,
        crate::fusion::Kind::Polygon,
        crate::fusion::Kind::Blur,
        crate::fusion::Kind::Mosaic,
        crate::fusion::Kind::DeltaKeyer,
    ];

    /// A node of a kind as a new one is set; but a Polygon, which has no points at first,
    /// with two, each at its centre, so that it has every setting that one may have; and a
    /// Tracker, which has no patterns at first, with one that has a path of two frames, so
    /// that it has every number that one may have; and so a Planar Tracker, with a path of two
    /// frames; and a Text+, which has no outline, no border and no follower at first, with
    /// each of them as a new one is.
    fn first_of(kind: crate::fusion::Kind) -> crate::fusion::NodeKind {
        use crate::fusion::{
            NodeKind, Pattern, PlanarTracker, Point, Polygon, TextBorder, TextFollower,
            TextOutline, TextPlus, Tracker, STILL,
        };
        match kind {
            crate::fusion::Kind::TextPlus => NodeKind::TextPlus(Box::new(TextPlus {
                outline: Some(TextOutline::default()),
                border: Some(TextBorder::default()),
                follower: Some(TextFollower::default()),
                ..TextPlus::default()
            })),
            crate::fusion::Kind::PlanarTracker => NodeKind::PlanarTracker(PlanarTracker {
                path: [(0, STILL), (12, [1.0, 0.0, 0.1, 0.0, 1.0, -0.05, 0.0, 0.0])]
                    .into_iter()
                    .collect(),
                ..PlanarTracker::default()
            }),
            crate::fusion::Kind::Polygon => NodeKind::Polygon(Polygon {
                points: vec![Point::new(0.0, 0.0), Point::new(0.0, 0.0)],
                ..Polygon::default()
            }),
            crate::fusion::Kind::Tracker => NodeKind::Tracker(Tracker {
                trackers: vec![Pattern {
                    path: [(0, [0.5, 0.5]), (12, [0.25, 0.75])].into_iter().collect(),
                    ..Pattern::default()
                }],
                ..Tracker::default()
            }),
            _ => NodeKind::new(kind),
        }
    }

    /// A path with a star in the stead of each place in a list, as the catalog writes it.
    fn starred(path: &str) -> String {
        let parts: Vec<&str> = path
            .split('.')
            .map(|part| {
                if part.bytes().all(|byte| byte.is_ascii_digit()) {
                    "*"
                } else {
                    part
                }
            })
            .collect();
        parts.join(".")
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "a node of each kind, its settings made one by one"
    )]
    fn every_setting_of_a_node_of_a_composition_that_takes_keys_is_listed_and_nothing_else() {
        use crate::fusion::NodeKind;
        for kind in FUSION_KINDS {
            let whole = first_of(kind).params();
            let mut found = Vec::new();
            numbers(&whole, "", &mut found);
            let mut keyed = BTreeSet::new();
            for (path, value) in &found {
                let keys = json!({"keys": [{"frame": 0, "value": value, "interp": "linear"}]});
                let mut changed = whole.clone();
                put(&mut changed, path, keys);
                let takes_keys = NodeKind::from_params(kind, changed).is_ok();
                assert_eq!(
                    of_fusion(kind, path).is_some(),
                    takes_keys,
                    "{kind:?} {path}"
                );
                // What the catalog lists is what the operations on keys find, by that path.
                assert_eq!(
                    first_of(kind).setting_mut(path).is_some(),
                    takes_keys,
                    "{kind:?} {path}"
                );
                if takes_keys {
                    keyed.insert(starred(path));
                }
            }
            let listed: Vec<String> = fusion(kind).iter().map(|e| e.path.clone()).collect();
            assert_eq!(listed.iter().cloned().collect::<BTreeSet<_>>(), keyed);
            // Each is listed once, in the order of the settings of the node: those of the
            // points of a Polygon once for all of them.
            let mut settings: Vec<String> = Vec::new();
            for path in first_of(kind).settings() {
                let path = starred(&path);
                if !settings.contains(&path) {
                    settings.push(path);
                }
            }
            assert_eq!(listed, settings, "{kind:?}");
        }
        assert_eq!(fusion(crate::fusion::Kind::MediaIn), &[] as &[Entry]);
        assert_eq!(fusion(crate::fusion::Kind::MediaOut), &[] as &[Entry]);
        // No number of a Tracker takes keys: not its pivot, not its reference, and not a
        // number of a pattern or a place of its path.
        let tracker = crate::fusion::Kind::Tracker;
        assert_eq!(fusion(tracker), &[] as &[Entry]);
        for path in [
            "reference",
            "pivot_tracker",
            "pivot_point.x",
            "trackers.0.pattern.width",
            "trackers.0.offset.x",
            "trackers.0.path.12.0",
        ] {
            assert!(of_fusion(tracker, path).is_none(), "{path}");
        }
        // Nor does a number of a Planar Tracker: of its region, its corners or its path.
        let planar = crate::fusion::Kind::PlanarTracker;
        assert_eq!(fusion(planar), &[] as &[Entry]);
        for path in ["reference", "region.0.x", "corners.tl.y", "path.12.2"] {
            assert!(of_fusion(planar, path).is_none(), "{path}");
        }
        assert_eq!(fusion(crate::fusion::Kind::Merge).len(), 8);
        assert_eq!(fusion(crate::fusion::Kind::Transform).len(), 4);
        assert_eq!(fusion(crate::fusion::Kind::Background).len(), 4);
        assert_eq!(fusion(crate::fusion::Kind::Rectangle).len(), 9);
        assert_eq!(fusion(crate::fusion::Kind::Ellipse).len(), 8);
        assert_eq!(fusion(crate::fusion::Kind::Polygon).len(), 9);
        assert_eq!(fusion(crate::fusion::Kind::Blur).len(), 3);
        assert_eq!(fusion(crate::fusion::Kind::Mosaic).len(), 4);
        assert_eq!(fusion(crate::fusion::Kind::DeltaKeyer).len(), 16);
        // A Text+ lists those of its outline, its border and its follower, which it may have;
        // its text, its font, its menus and the delay of its follower take no keys.
        let text = crate::fusion::Kind::TextPlus;
        assert_eq!(fusion(text).len(), 32);
        for path in [
            "text",
            "font",
            "justify",
            "center",
            "transform.select",
            "outline",
            "follower.delay",
            "follower.order",
            "fill.color",
        ] {
            assert!(of_fusion(text, path).is_none(), "{path}");
        }
        assert_eq!(of_fusion(text, "size").unwrap().unit, Unit::Heights);
        assert_eq!(of_fusion(text, "center.x").unwrap().unit, Unit::Widths);
        assert_eq!(
            of_fusion(text, "follower.angle").unwrap().unit,
            Unit::DegreesCounterclockwise
        );
        let border = of_fusion(text, "border.color.a").unwrap();
        assert_eq!((border.default, border.low, border.high), (0.6, 0.0, 1.0));
        let keyer = crate::fusion::Kind::DeltaKeyer;
        for path in [
            "screen",
            "replace",
            "replace_color",
            "garbage_invert",
            "spill_method",
            "view",
            "size.x",
        ] {
            assert!(of_fusion(keyer, path).is_none(), "{path}");
        }
        assert!(of_fusion(crate::fusion::Kind::Blur, "gain").is_none());
        assert!(of_fusion(crate::fusion::Kind::Blur, "lock_xy").is_none());
        assert!(of_fusion(crate::fusion::Kind::Blur, "filter").is_none());
        assert!(of_fusion(crate::fusion::Kind::Blur, "size").is_none());
        assert!(of_fusion(crate::fusion::Kind::Mosaic, "whole_cells_only").is_none());
        assert!(of_fusion(crate::fusion::Kind::Mosaic, "blend").is_none());
        // A setting is listed for the kind of node that has it, and what is no number is not.
        assert!(of_fusion(crate::fusion::Kind::Transform, "blend").is_none());
        assert!(of_fusion(crate::fusion::Kind::Merge, "apply").is_none());
        assert!(of_fusion(crate::fusion::Kind::Merge, "center").is_none());
        assert!(of_fusion(crate::fusion::Kind::Transform, "flip_vertical").is_none());
        assert!(of_fusion(crate::fusion::Kind::Ellipse, "corner_radius").is_none());
        assert!(of_fusion(crate::fusion::Kind::Rectangle, "solid").is_none());
        assert!(of_fusion(crate::fusion::Kind::Rectangle, "paint_mode").is_none());
        // A point of a Polygon is found by its place, and is not a setting whole.
        let polygon = crate::fusion::Kind::Polygon;
        assert_eq!(
            of_fusion(polygon, "points.12.y").unwrap().path,
            "points.*.y"
        );
        assert!(of_fusion(polygon, "points.12").is_none());
        assert!(of_fusion(polygon, "points").is_none());
        assert!(of_fusion(polygon, "fill").is_none());
    }

    #[test]
    fn a_setting_of_a_composition_is_at_first_and_within_what_its_entry_says() {
        use crate::fusion::NodeKind;
        for kind in FUSION_KINDS {
            let whole = first_of(kind).params();
            for entry in fusion(kind) {
                assert_eq!(
                    (entry.page, entry.palette, entry.clock),
                    (Page::Fusion, Palette::Fusion, Clock::SourceFrame),
                    "{}",
                    entry.path
                );
                // A setting of a point, as that of the first point.
                let path = entry.path.replace('*', "0");
                let first = whole
                    .pointer(&format!("/{}", path.replace('.', "/")))
                    .unwrap();
                assert_eq!(first, &json!(entry.default), "{kind:?} {}", entry.path);
                let step = (entry.high - entry.low) * 1e-9;
                for (end, (value, taken)) in [
                    (entry.low, true),
                    (entry.high, true),
                    (entry.low - step, false),
                    (entry.high + step, false),
                ]
                .into_iter()
                .enumerate()
                {
                    // The screen colour of a keyer is also to have one channel above its
                    // other two: its green at its least, the others being none, leaves none.
                    let grey = kind == crate::fusion::Kind::DeltaKeyer
                        && entry.path == "screen.g"
                        && end == 0;
                    if grey {
                        continue;
                    }
                    let mut changed = whole.clone();
                    put(&mut changed, &path, json!(value));
                    let node = NodeKind::from_params(kind, changed).unwrap();
                    assert_eq!(
                        node.check().is_ok(),
                        taken,
                        "{kind:?} {} at {value}",
                        entry.path
                    );
                }
            }
        }
        // The units are Fusion's: a place in shares of the picture, an angle counter-clockwise.
        let merge = crate::fusion::Kind::Merge;
        let x = of_fusion(merge, "center.x").unwrap();
        assert_eq!(
            (x.default, x.low, x.high, x.unit),
            (0.5, -15.5, 16.5, Unit::Widths)
        );
        assert_eq!(of_fusion(merge, "center.y").unwrap().unit, Unit::Heights);
        // A Blur Size is counted as the filters of the Color page are, and the offsets of a
        // Mosaic in pixels of the picture.
        let size = of_fusion(crate::fusion::Kind::Blur, "size.y").unwrap();
        assert_eq!(
            (size.default, size.low, size.high, size.unit),
            (5.0, 0.0, 500.0, Unit::Pixels1080)
        );
        let offset = of_fusion(crate::fusion::Kind::Mosaic, "offset.x").unwrap();
        assert_eq!(
            (offset.default, offset.low, offset.high, offset.unit),
            (0.0, -16_384.0, 16_384.0, Unit::Pixels)
        );
        let frequency = of_fusion(crate::fusion::Kind::Mosaic, "frequency").unwrap();
        assert_eq!(
            (frequency.default, frequency.low, frequency.high),
            (32.0, 1.0, 16_384.0)
        );
        // The lengths of a Delta Keyer are counted as a Blur Size is, and the channels of its
        // colours as those of a picture are, which may be above 1.
        let keyer = crate::fusion::Kind::DeltaKeyer;
        for (path, wanted) in [
            ("pre_blur", (0.0, 0.0, 100.0, Unit::Pixels1080)),
            ("matte_blur", (0.0, 0.0, 100.0, Unit::Pixels1080)),
            ("erode_dilate", (0.0, -100.0, 100.0, Unit::Pixels1080)),
            ("screen.g", (1.0, 0.0, 65_504.0, Unit::Number)),
            ("screen.b", (0.0, 0.0, 65_504.0, Unit::Number)),
            ("replace_color.r", (0.5, 0.0, 65_504.0, Unit::Number)),
            ("gain", (1.0, 0.0, 10.0, Unit::Number)),
            ("high", (1.0, 0.0, 1.0, Unit::Number)),
        ] {
            let entry = of_fusion(keyer, path).unwrap();
            assert_eq!(
                (entry.default, entry.low, entry.high, entry.unit),
                wanted,
                "{path}"
            );
        }
        let angle = of_fusion(merge, "angle").unwrap();
        assert_eq!(
            serde_json::to_value(angle).unwrap(),
            json!({
                "path": "angle",
                "default": 0.0,
                "low": -3600.0,
                "high": 3600.0,
                "unit": "degrees_counterclockwise",
                "page": "fusion",
                "palette": "fusion",
                "clock": "source_frame",
            })
        );
    }
}
