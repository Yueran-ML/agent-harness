//! The settings of a scope as the protocol writes them (`PROTOCOL.md`, Scopes), and the figures
//! that a scope gives back: its graticule and the statistics of a frame.
//!
//! The settings are those of [`columbina_compose::scopes::ScopeSettings`], written flat: the
//! kind, the kind's own fields beside it, and the signal and the look as objects of their own.
//! Every field but `kind` may be left out, and is then what the compositor makes it; the settings
//! are given back with every field written, as they were taken.

use columbina_compose::scopes::{
    self as compose, Bins, Channel, Graticule, HistogramChannels, Mapping, Primaries, ScaleStyle,
    Targets, Tones, Triangle, WaveformChannels,
};
use columbina_model::{Gamma, Gamut};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The settings of a scope, as they are written. A field that is not one of them is refused by
/// the kind's own fields, which take what the signal and the look do not.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Settings {
    #[serde(flatten)]
    pub kind: Kind,
    /// Left out, the signal of the project's output space, followed as it changes.
    #[serde(default = "Signal::of_project")]
    pub signal: Signal,
    #[serde(default)]
    pub look: Look,
}

/// Which scope, with the fields of its own.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Kind {
    Waveform(Waveform),
    Parade(Parade),
    Vectorscope(Vectorscope),
    Histogram(Histogram),
    Chromaticity(Chromaticity),
}

/// The units of a scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum Scale {
    #[default]
    #[serde(rename = "10-bit")]
    TenBit,
    #[serde(rename = "percent")]
    Percent,
    #[serde(rename = "nits")]
    Nits,
}

impl From<Scale> for ScaleStyle {
    fn from(scale: Scale) -> ScaleStyle {
        match scale {
            Scale::TenBit => ScaleStyle::TenBit,
            Scale::Percent => ScaleStyle::Percent,
            Scale::Nits => ScaleStyle::Nits,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Waveform {
    pub width: u32,
    pub levels: u32,
    pub channels: WaveformShows,
    pub scale: Scale,
}

impl Default for Waveform {
    fn default() -> Self {
        let made = compose::Waveform::default();
        Waveform {
            width: made.width,
            levels: made.levels,
            channels: WaveformShows::Luma,
            scale: Scale::TenBit,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WaveformShows {
    #[default]
    Luma,
    Rgb,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Parade {
    pub width: u32,
    pub levels: u32,
    pub scale: Scale,
}

impl Default for Parade {
    fn default() -> Self {
        let made = compose::Parade::default();
        Parade {
            width: made.width,
            levels: made.levels,
            scale: Scale::TenBit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Vectorscope {
    pub size: u32,
    pub zoom: f32,
    pub tones: ToneRange,
    pub low: f32,
    pub high: f32,
    pub targets: TargetBoxes,
    pub skin_tone: bool,
}

impl Default for Vectorscope {
    fn default() -> Self {
        let made = compose::Vectorscope::default();
        Vectorscope {
            size: made.size,
            zoom: made.zoom,
            tones: ToneRange::All,
            low: made.low,
            high: made.high,
            targets: TargetBoxes::Percent75,
            skin_tone: made.skin_tone,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToneRange {
    #[default]
    All,
    Low,
    Mid,
    High,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum TargetBoxes {
    #[serde(rename = "off")]
    Off,
    #[default]
    #[serde(rename = "75")]
    Percent75,
    #[serde(rename = "100")]
    Percent100,
    #[serde(rename = "both")]
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Histogram {
    pub bins: u32,
    pub height: u32,
    pub channels: HistogramShows,
    pub scale: Scale,
}

impl Default for Histogram {
    fn default() -> Self {
        let made = compose::Histogram::default();
        Histogram {
            bins: made.bins,
            height: made.height,
            channels: HistogramShows::Rgb,
            scale: Scale::TenBit,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HistogramShows {
    #[default]
    Rgb,
    Yrgb,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Chromaticity {
    pub width: u32,
    pub height: u32,
    pub space: Points,
    pub gamuts: Vec<Gamut3>,
}

impl Default for Chromaticity {
    fn default() -> Self {
        let made = compose::Chromaticity::default();
        Chromaticity {
            width: made.width,
            height: made.height,
            space: Points::Named(Gamut::Rec709),
            gamuts: vec![Gamut3 {
                name: "Rec.709".into(),
                primaries: Points::Named(Gamut::Rec709),
            }],
        }
    }
}

/// A triangle of a gamut drawn on the chromaticity diagram.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Gamut3 {
    pub name: String,
    pub primaries: Points,
}

/// The primaries and white of a space: by the name of a gamut of the model (`rec709`,
/// `rec2020`, `dci-p3`, `p3-d65`, `rec601-pal`, `rec601-ntsc`), or as CIE 1931 xy.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Points {
    Named(Gamut),
    Xy(Xy),
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xy {
    pub red: [f64; 2],
    pub green: [f64; 2],
    pub blue: [f64; 2],
    pub white: [f64; 2],
}

impl Points {
    fn primaries(self) -> Result<Primaries, String> {
        match self {
            Points::Named(gamut) => Primaries::of(gamut)
                .ok_or_else(|| format!("the gamut {gamut:?} has no primaries that are known")),
            Points::Xy(xy) => Ok(Primaries {
                red: xy.red,
                green: xy.green,
                blue: xy.blue,
                white: xy.white,
            }),
        }
    }
}

/// What the signal of the frame is: its luma coefficients and its transfer; or, with `project`,
/// those of the output space of the project's colour management, whatever they are (the luma and
/// transfer written are then those of Rec.709, which `DaVinci YRGB` has).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Signal {
    pub luma: Luma,
    pub transfer: Gamma,
    /// Whether the scope measures by the signal of the project's output space, followed as it
    /// changes, rather than by these.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub project: bool,
}

impl Default for Signal {
    fn default() -> Self {
        Signal {
            luma: Luma::Named(LumaOf::Rec709),
            transfer: Gamma::Rec709,
            project: false,
        }
    }
}

impl Signal {
    /// The signal of a scope that says none: the project's.
    fn of_project() -> Signal {
        Signal {
            project: true,
            ..Signal::default()
        }
    }
}

/// Luma coefficients: of a standard, or red, green and blue.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Luma {
    Named(LumaOf),
    Coefficients([f32; 3]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LumaOf {
    Rec709,
    Rec601,
    Rec2020,
}

impl Luma {
    fn coefficients(self) -> [f32; 3] {
        match self {
            Luma::Named(LumaOf::Rec709) => compose::REC709_LUMA,
            Luma::Named(LumaOf::Rec601) => compose::REC601_LUMA,
            Luma::Named(LumaOf::Rec2020) => compose::REC2020_LUMA,
            Luma::Coefficients(coefficients) => coefficients,
        }
    }
}

impl Signal {
    pub fn compose(self) -> compose::Signal {
        compose::Signal {
            luma: self.luma.coefficients(),
            transfer: self.transfer,
        }
    }
}

/// How a scope is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Look {
    pub mapping: Map,
    pub brightness: f32,
    pub graticule: f32,
}

impl Default for Look {
    fn default() -> Self {
        let made = compose::Look::default();
        Look {
            mapping: Map::Log,
            brightness: made.brightness,
            graticule: made.graticule,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Map {
    #[default]
    Log,
    Gamma,
}

impl Settings {
    /// The settings that the compositor counts and draws by, once they are checked.
    ///
    /// # Errors
    /// A value is out of range, or a gamut has no primaries; said.
    pub fn compose(&self) -> Result<compose::ScopeSettings, String> {
        let kind = match &self.kind {
            Kind::Waveform(waveform) => compose::Kind::Waveform(compose::Waveform {
                width: waveform.width,
                levels: waveform.levels,
                channels: match waveform.channels {
                    WaveformShows::Luma => WaveformChannels::Luma,
                    WaveformShows::Rgb => WaveformChannels::Rgb,
                },
                scale: waveform.scale.into(),
            }),
            Kind::Parade(parade) => compose::Kind::Parade(compose::Parade {
                width: parade.width,
                levels: parade.levels,
                scale: parade.scale.into(),
            }),
            Kind::Vectorscope(scope) => compose::Kind::Vectorscope(compose::Vectorscope {
                size: scope.size,
                zoom: scope.zoom,
                tones: match scope.tones {
                    ToneRange::All => Tones::All,
                    ToneRange::Low => Tones::Low,
                    ToneRange::Mid => Tones::Mid,
                    ToneRange::High => Tones::High,
                },
                low: scope.low,
                high: scope.high,
                targets: match scope.targets {
                    TargetBoxes::Off => Targets::Off,
                    TargetBoxes::Percent75 => Targets::Percent75,
                    TargetBoxes::Percent100 => Targets::Percent100,
                    TargetBoxes::Both => Targets::Both,
                },
                skin_tone: scope.skin_tone,
            }),
            Kind::Histogram(histogram) => compose::Kind::Histogram(compose::Histogram {
                bins: histogram.bins,
                height: histogram.height,
                channels: match histogram.channels {
                    HistogramShows::Rgb => HistogramChannels::Rgb,
                    HistogramShows::Yrgb => HistogramChannels::Yrgb,
                },
                scale: histogram.scale.into(),
            }),
            Kind::Chromaticity(diagram) => compose::Kind::Chromaticity(compose::Chromaticity {
                width: diagram.width,
                height: diagram.height,
                space: diagram.space.primaries()?,
                gamuts: diagram
                    .gamuts
                    .iter()
                    .map(|gamut| {
                        Ok(Triangle {
                            name: gamut.name.clone(),
                            primaries: gamut.primaries.primaries()?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
            }),
        };
        let settings = compose::ScopeSettings {
            kind,
            signal: self.signal.compose(),
            look: compose::Look {
                mapping: match self.look.mapping {
                    Map::Log => Mapping::Log,
                    Map::Gamma => Mapping::Gamma,
                },
                brightness: self.look.brightness,
                graticule: self.look.graticule,
            },
        };
        if !settings.look.brightness.is_finite() || !settings.look.graticule.is_finite() {
            return Err("brightness and graticule are from 0 to 1".into());
        }
        settings.check()?;
        Ok(settings)
    }
}

/// A graticule as it is written: its size, and its lines and labels in the pixels of the
/// picture, from its top left corner.
pub fn graticule(graticule: &Graticule) -> Value {
    json!({
        "width": graticule.width,
        "height": graticule.height,
        "lines": graticule.lines.iter().map(|line| json!({
            "from": line.from,
            "to": line.to,
            "colour": line.colour,
        })).collect::<Vec<_>>(),
        "labels": graticule.labels.iter().map(|label| json!({
            "at": label.at,
            "text": label.text,
            "colour": label.colour,
        })).collect::<Vec<_>>(),
    })
}

/// The statistics of a frame as they are written, with the 1%, 50% and 99% of each channel.
pub fn statistics(statistics: &compose::Statistics) -> Value {
    let channel =
        |channel: &Channel| json!({ "min": channel.min, "max": channel.max, "mean": channel.mean });
    let histogram = &statistics.histogram;
    let percentiles = |index: usize| {
        json!({
            "1": histogram.percentile(index, 0.01),
            "50": histogram.percentile(index, 0.5),
            "99": histogram.percentile(index, 0.99),
        })
    };
    let chroma = &statistics.chroma;
    json!({
        "samples": statistics.samples,
        "red": channel(&statistics.red),
        "green": channel(&statistics.green),
        "blue": channel(&statistics.blue),
        "luma": channel(&statistics.luma),
        "chroma": {
            "centroid": chroma.centroid,
            "spread": chroma.spread,
            "radius": chroma.radius,
            "saturation": chroma.saturation,
            "hue": chroma.hue,
        },
        "percentiles": {
            "red": percentiles(Bins::RED),
            "green": percentiles(Bins::GREEN),
            "blue": percentiles(Bins::BLUE),
            "luma": percentiles(Bins::LUMA),
        },
        "histogram": {
            "bins": histogram.bins,
            "red": histogram.counts[Bins::RED],
            "green": histogram.counts[Bins::GREEN],
            "blue": histogram.counts[Bins::BLUE],
            "luma": histogram.counts[Bins::LUMA],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(value: Value) -> Result<Settings, String> {
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    #[test]
    fn a_kind_alone_is_the_compositors_defaults_and_is_written_back_in_full() {
        for (name, made) in [
            (
                "waveform",
                compose::Kind::Waveform(compose::Waveform::default()),
            ),
            ("parade", compose::Kind::Parade(compose::Parade::default())),
            (
                "vectorscope",
                compose::Kind::Vectorscope(compose::Vectorscope::default()),
            ),
            (
                "histogram",
                compose::Kind::Histogram(compose::Histogram::default()),
            ),
            (
                "chromaticity",
                compose::Kind::Chromaticity(compose::Chromaticity::default()),
            ),
        ] {
            let settings = read(json!({ "kind": name })).unwrap();
            assert_eq!(
                settings.compose().unwrap(),
                compose::ScopeSettings::new(made),
                "{name}"
            );
            let written = serde_json::to_value(&settings).unwrap();
            assert_eq!(written["kind"], name);
            assert_eq!(read(written.clone()).unwrap(), settings, "{written}");
        }
        let written = serde_json::to_value(read(json!({"kind":"waveform"})).unwrap()).unwrap();
        assert_eq!(
            written,
            json!({"kind":"waveform","width":512,"levels":256,"channels":"luma","scale":"10-bit",
                "signal":{"luma":"rec709","transfer":"rec709","project":true},
                "look":{"mapping":"log","brightness":0.5,"graticule":0.5}})
        );
    }

    #[test]
    fn each_field_is_read_into_the_compositors_settings() {
        let settings = read(json!({
            "kind": "vectorscope", "size": 128, "zoom": 2, "tones": "mid", "low": 0.2,
            "high": 0.8, "targets": "both", "skin_tone": false,
            "signal": {"luma": "rec2020", "transfer": "st2084"},
            "look": {"mapping": "gamma", "brightness": 0.75, "graticule": 0},
        }))
        .unwrap()
        .compose()
        .unwrap();
        assert_eq!(
            settings.kind,
            compose::Kind::Vectorscope(compose::Vectorscope {
                size: 128,
                zoom: 2.0,
                tones: Tones::Mid,
                low: 0.2,
                high: 0.8,
                targets: Targets::Both,
                skin_tone: false,
            })
        );
        assert_eq!(settings.signal.luma, compose::REC2020_LUMA);
        assert_eq!(settings.signal.transfer, Gamma::St2084);
        assert_eq!(settings.look.mapping, Mapping::Gamma);
        let diagram = read(json!({
            "kind": "chromaticity", "space": "rec2020",
            "gamuts": [{"name":"2020","primaries":"rec2020"},
                {"name":"mine","primaries":{"red":[0.7,0.3],"green":[0.2,0.7],"blue":[0.15,0.05],
                    "white":[0.3127,0.329]}}],
            "signal": {"luma": [0.3, 0.6, 0.1]},
        }))
        .unwrap()
        .compose()
        .unwrap();
        let compose::Kind::Chromaticity(diagram_kind) = diagram.kind else {
            panic!("a chromaticity diagram")
        };
        assert_eq!(diagram_kind.space, Primaries::REC2020);
        assert_eq!(diagram_kind.gamuts[1].primaries.red, [0.7, 0.3]);
        assert_eq!(diagram.signal.luma, [0.3, 0.6, 0.1]);
        let histogram = read(json!({"kind":"histogram","channels":"yrgb","scale":"nits"}))
            .unwrap()
            .compose()
            .unwrap();
        assert_eq!(
            histogram.kind,
            compose::Kind::Histogram(compose::Histogram {
                channels: HistogramChannels::Yrgb,
                scale: ScaleStyle::Nits,
                ..compose::Histogram::default()
            })
        );
    }

    #[test]
    fn what_is_not_a_setting_or_out_of_range_is_refused() {
        for wrong in [
            json!({}),
            json!({"kind":"oscilloscope"}),
            json!({"kind":"waveform","size":3}),
            json!({"kind":"waveform","levels":"many"}),
            json!({"kind":"waveform","look":{"shine":1}}),
            json!({"kind":"waveform","extra":1}),
            json!({"kind":"vectorscope","targets":"50"}),
        ] {
            assert!(read(wrong.clone()).is_err(), "{wrong}");
        }
        for wrong in [
            json!({"kind":"waveform","width":8}),
            json!({"kind":"vectorscope","size":101}),
            json!({"kind":"vectorscope","zoom":9}),
            json!({"kind":"waveform","look":{"brightness":2}}),
            json!({"kind":"chromaticity","space":"camera-native"}),
            json!({"kind":"waveform","signal":{"luma":[0.9,0.0,0.1]}}),
        ] {
            assert!(read(wrong.clone()).unwrap().compose().is_err(), "{wrong}");
        }
    }
}
