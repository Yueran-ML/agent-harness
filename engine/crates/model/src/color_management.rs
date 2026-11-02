//! The colour management of a project, as the Color Management panel of `DaVinci Resolve`'s
//! Project Settings sets it (Reference Manual 20, pp. 149–153, 224–252), and the colour of
//! media set by hand in its Media Pool (pp. 220–224, 228–230, 245).
//!
//! These are settings only: what the picture is made of them is the compositor's.
//!
//! # Names
//!
//! A colour space is named as the catalogue of the engine names it (`columbina_ocio::catalogue`,
//! by the names of Resolve's menus), and is kept by that name, never by the name of the space of
//! OCIO that it is, so that a project need not change when the catalogue maps a name otherwise.
//! [`spaces`] are those names: the 48 spaces of media, each a gamut but Camera Native and a gamma
//! named by [`InputSpace::name`], and then the 20 spaces of cameras and of grading,
//! [`CAMERA_SPACES`]. The tests of the crate of OCIO check that the two lists are the same, in
//! the same order, and that each output transform of ACES ([`AcesVersion::odts`]) is a display
//! and a view of the built-in config of its version.
//!
//! Every space of the catalogue may be chosen wherever a space is (the author's decision Q3).
//!
//! # What applies
//!
//! Every setting is kept whatever the colour science, as Resolve keeps them, and is always
//! written; but a change sets only those that apply, to the settings that the project has or to
//! those that the change makes (see [`ColorSettings::applies`]):
//!
//! - always: `science`, `lut_interpolation`, `input_lut`, `output_lut`, `viewer_lut`,
//!   `s_curve_contrast`;
//! - in `DaVinci YRGB Color Managed` (`yrgb_managed`): `automatic`, `output` and
//!   `graphics_white`; `processing_mode` where `automatic` is on, and `preset` where it is off;
//!   and with the preset Custom, the rest of Resolve Color Management: `separate_gamma`,
//!   `input`, `timeline`, `working_luminance`, `hdr_203_nits`, `limit_gamut`, `input_drt`,
//!   `output_drt`, `inverse_drt`, `white_point_adaptation`, `color_space_aware_tools`,
//!   `resize_in`; `custom_luminance` where the working luminance is Custom, and
//!   `input_sat_rolloff` or `output_sat_rolloff` where that DRT is Saturation Preserving;
//! - in `ACEScct` or `ACEScc`: `aces_version`, `idt`, `odt`, `gamut_compress`, `node_luts_in`.
//!
//! A preset other than Custom sets the timeline space, the working luminance, the DRTs and the
//! gamut limit by [`Preset::settings`], and leaves the input space Rec.709 Gamma 2.4 (p. 230);
//! [`ColorSettings::pipeline`] gives the settings that are in force.
//!
//! # Defaults
//!
//! A project is not colour managed until it is set to be: `DaVinci YRGB` is the default (p. 225,
//! and the author's decision Q1). Where the manual gives a default it is that; where it does not,
//! ours is written beside the setting, to be checked against Resolve.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::color::{input_space, Basis, Gamma, Gamut, InputSpace, CAMERA_RAW_CODECS};
use crate::grade::LONGEST_PATH;
use crate::ops::OpError;
use crate::project::Media;

/// The name of the space that a picture is not converted from or to: Bypass, in the menus of
/// the input and the output colour space (p. 245), and of the media pool.
pub const BYPASS: &str = "bypass";

/// Rec.709 Gamma 2.4: the input space of every preset but Custom (p. 230), and the default
/// space of the timeline and of the output.
pub const REC709: &str = "Rec.709 Gamma 2.4";

/// The least and the most nits of a luminance that is set: up to the 10000 of PQ.
const NITS: (f64, f64) = (1.0, 10_000.0);

/// The gamuts of the spaces of media, in the order of the catalogue.
const GAMUTS: [Gamut; 6] = [
    Gamut::Rec709,
    Gamut::Rec601Pal,
    Gamut::Rec601Ntsc,
    Gamut::Rec2020,
    Gamut::DciP3,
    Gamut::P3D65,
];

/// The gammas of the spaces of media, in the order of the catalogue.
const GAMMAS: [Gamma; 8] = [
    Gamma::Rec709,
    Gamma::Gamma22,
    Gamma::Gamma26,
    Gamma::Gamma28,
    Gamma::Srgb,
    Gamma::Linear,
    Gamma::St2084,
    Gamma::Hlg,
];

/// The spaces of cameras and of grading of the catalogue, by their names, in its order.
pub const CAMERA_SPACES: [&str; 20] = [
    "Rec.709 (Scene)",
    "Apple Log",
    "Apple Log 2",
    "DJI D-Gamut/D-Log",
    "Sony S-Gamut3/S-Log3",
    "Sony S-Gamut3.Cine/S-Log3",
    "Panasonic V-Gamut/V-Log",
    "Canon Cinema Gamut/Canon Log 2",
    "Canon Cinema Gamut/Canon Log 3",
    "ARRI Wide Gamut 3/LogC3",
    "ARRI Wide Gamut 4/LogC4",
    "FujiFilm F-Gamut/F-Log2",
    "Blackmagic Design Wide Gamut Gen 5/Film Gen 5",
    "RED Wide Gamut RGB/Log3G10",
    "DaVinci Wide Gamut/DaVinci Intermediate",
    "DaVinci Wide Gamut/Linear",
    "ACEScct",
    "ACEScc",
    "ACEScg",
    "ACES (AP0) Linear",
];

/// The names of every space of the catalogue: those of media, each gamut with each gamma, then
/// [`CAMERA_SPACES`].
#[must_use]
pub fn spaces() -> &'static [String] {
    static SPACES: OnceLock<Vec<String>> = OnceLock::new();
    SPACES.get_or_init(|| {
        let media = GAMUTS
            .iter()
            .flat_map(|&gamut| GAMMAS.iter().map(move |&gamma| InputSpace { gamut, gamma }))
            .map(InputSpace::name);
        media
            .chain(CAMERA_SPACES.iter().map(|&name| name.to_owned()))
            .collect()
    })
}

/// Whether `name` is a space of the catalogue.
#[must_use]
pub fn is_space(name: &str) -> bool {
    spaces().iter().any(|space| space == name)
}

/// The space of media (a gamut and a gamma) that `name` is, if it is one.
#[must_use]
pub fn media_space(name: &str) -> Option<InputSpace> {
    GAMUTS
        .iter()
        .flat_map(|&gamut| GAMMAS.iter().map(move |&gamma| InputSpace { gamut, gamma }))
        .find(|space| space.name() == name)
}

// --- the settings -------------------------------------------------------------------------------

/// How the colours of a project are managed: the Color science menu (pp. 149, 224–226).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Science {
    /// `DaVinci YRGB`: not managed; every transform is made by hand, with LUTs or grades. The
    /// default (p. 225).
    #[default]
    Yrgb,
    /// `DaVinci YRGB Color Managed`: Resolve Color Management (RCM).
    YrgbManaged,
    /// `DaVinci ACEScct`.
    Acescct,
    /// `DaVinci ACEScc`.
    Acescc,
}

impl Science {
    /// Whether it is one of ACES.
    #[must_use]
    pub fn is_aces(self) -> bool {
        matches!(self, Self::Acescct | Self::Acescc)
    }
}

/// What Automatic color management manages for (p. 230): the Color processing mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingMode {
    /// Standard dynamic range. Ours: the manual gives no default.
    #[default]
    Sdr,
    /// High dynamic range.
    Hdr,
}

impl ProcessingMode {
    /// The preset that automatic colour management is taken to be. Ours: the manual says only
    /// that it chooses for the media (p. 230).
    #[must_use]
    pub fn preset(self) -> Preset {
        match self {
            Self::Sdr => Preset::SdrRec709,
            Self::Hdr => Preset::HdrRec2020,
        }
    }
}

/// The Resolve color management preset (pp. 230–232).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// SDR Rec.709, the default (p. 231).
    #[default]
    SdrRec709,
    /// SDR P3 Broadcast: P3-D65.
    SdrP3Broadcast,
    /// SDR P3 Cinema: P3-D60.
    SdrP3Cinema,
    /// SDR Rec.2020.
    SdrRec2020,
    /// `DaVinci` Wide Gamut, a log space of grading for SDR and HDR.
    DavinciWideGamut,
    /// HDR P3 Broadcast, up to 1000 nits, its gamut limited to P3-D65.
    HdrP3Broadcast,
    /// HDR Rec.2020, up to 1000 nits.
    HdrRec2020,
    /// Custom: every setting of RCM as it is set.
    Custom,
}

/// What a preset other than Custom sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresetSettings {
    /// The timeline colour space.
    pub timeline: &'static str,
    /// The output colour space that it is chosen with, which the output may be set apart from
    /// (p. 232).
    pub output: &'static str,
    /// The timeline working luminance.
    pub working_luminance: WorkingLuminance,
    /// The Input DRT.
    pub input_drt: ToneMapping,
    /// The Output DRT.
    pub output_drt: ToneMapping,
    /// The gamut that the output is limited to, if it is.
    pub limit_gamut: Option<Gamut>,
}

impl Preset {
    /// What the preset sets, or none for Custom.
    ///
    /// The manual names the gamut and the range of each (pp. 231–232) but not its settings, so
    /// these are ours, kept here in one place to be checked against Resolve:
    ///
    /// | preset | timeline | luminance | DRT in / out | limit |
    /// |---|---|---|---|---|
    /// | SDR Rec.709 | Rec.709 Gamma 2.4 | SDR 100 | None / None | – |
    /// | SDR P3 Broadcast | P3-D65 Gamma 2.4 | SDR 100 | None / None | – |
    /// | SDR P3 Cinema | DCI-P3 Gamma 2.6 (the catalogue has no P3-D60) | SDR 100 | None / None | – |
    /// | SDR Rec.2020 | Rec.2020 Gamma 2.4 | SDR 100 | None / None | – |
    /// | `DaVinci` Wide Gamut | `DaVinci` Wide Gamut/Intermediate | HDR ER 1000/2000 | `DaVinci` / `DaVinci` | – |
    /// | HDR P3 Broadcast | P3-D65 ST2084 | HDR 1000 | `DaVinci` / None | P3-D65 |
    /// | HDR Rec.2020 | Rec.2100 ST2084 | HDR 1000 | `DaVinci` / None | – |
    ///
    /// The output that each is chosen with is its timeline space, but Rec.709 Gamma 2.4 for
    /// `DaVinci` Wide Gamut.
    #[must_use]
    pub fn settings(self) -> Option<PresetSettings> {
        let sdr = |space: &'static str| PresetSettings {
            timeline: space,
            output: space,
            working_luminance: WorkingLuminance::Sdr100,
            input_drt: ToneMapping::None,
            output_drt: ToneMapping::None,
            limit_gamut: None,
        };
        let hdr = |space: &'static str, limit_gamut| PresetSettings {
            timeline: space,
            output: space,
            working_luminance: WorkingLuminance::Hdr1000,
            input_drt: ToneMapping::Davinci,
            output_drt: ToneMapping::None,
            limit_gamut,
        };
        Some(match self {
            Self::SdrRec709 => sdr(REC709),
            Self::SdrP3Broadcast => sdr("P3-D65 Gamma 2.4"),
            Self::SdrP3Cinema => sdr("DCI-P3 Gamma 2.6"),
            Self::SdrRec2020 => sdr("Rec.2020 Gamma 2.4"),
            Self::DavinciWideGamut => PresetSettings {
                timeline: "DaVinci Wide Gamut/DaVinci Intermediate",
                output: REC709,
                working_luminance: WorkingLuminance::HdrEr1000_2000,
                input_drt: ToneMapping::Davinci,
                output_drt: ToneMapping::Davinci,
                limit_gamut: None,
            },
            Self::HdrP3Broadcast => hdr("P3-D65 ST2084", Some(Gamut::P3D65)),
            Self::HdrRec2020 => hdr("Rec.2100 ST2084", None),
            Self::Custom => return None,
        })
    }
}

/// The Timeline working luminance (p. 238): the most nits that the Input DRT maps a picture to,
/// and the most that the timeline holds before it is clipped.
///
/// The manual names SDR 100, HDR 500 to 4000, the extended ranges of SDR and HDR (HDR ER
/// 1000/2000 is its example) and Custom; the levels of HDR between 500 and 4000 and of the
/// extended range of SDR are not listed, so only those it names are here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkingLuminance {
    /// SDR 100. Ours as the default, as SDR Rec.709 has it.
    #[default]
    #[serde(rename = "sdr_100")]
    Sdr100,
    /// HDR 500.
    #[serde(rename = "hdr_500")]
    Hdr500,
    /// HDR 1000.
    #[serde(rename = "hdr_1000")]
    Hdr1000,
    /// HDR 2000.
    #[serde(rename = "hdr_2000")]
    Hdr2000,
    /// HDR 4000.
    #[serde(rename = "hdr_4000")]
    Hdr4000,
    /// HDR ER 1000/2000: pictures are mapped to 1000 nits and clipped at 2000.
    #[serde(rename = "hdr_er_1000_2000")]
    HdrEr1000_2000,
    /// Custom: the nits of `custom_luminance`.
    #[serde(rename = "custom")]
    Custom,
}

impl WorkingLuminance {
    /// The nits that pictures are mapped to, and those that the timeline clips at; `custom` for
    /// Custom.
    #[must_use]
    pub fn nits(self, custom: f64) -> (f64, f64) {
        match self {
            Self::Sdr100 => (100.0, 100.0),
            Self::Hdr500 => (500.0, 500.0),
            Self::Hdr1000 => (1000.0, 1000.0),
            Self::Hdr2000 => (2000.0, 2000.0),
            Self::Hdr4000 => (4000.0, 4000.0),
            Self::HdrEr1000_2000 => (1000.0, 2000.0),
            Self::Custom => (custom, custom),
        }
    }
}

/// A tone mapping of a DRT, of input or of output (pp. 239–242).
///
/// Only None is a plain transform of colour; Resolve publishes the mathematics of none of the
/// others, so each of those is ours, made to do what the manual says of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToneMapping {
    /// None: a mapping of one to one, clipped out of gamut. Ours as the default.
    #[default]
    None,
    /// Simple: "a good mapping for color transforms from HDR to SDR" (about 5500 nits to 100,
    /// p. 2089).
    Simple,
    /// Luminance Mapping: as `DaVinci`, for media of one standard space.
    Luminance,
    /// `DaVinci`: a smooth roll-off of luminance in the shadows and highlights, and a controlled
    /// desaturation of the brightest and darkest values.
    Davinci,
    /// Saturation Preserving: the roll-off of `DaVinci` without its desaturation, but from the
    /// nits of its Sat. Rolloff Start to those of its Limit.
    SaturationPreserving,
}

/// The Sat. Rolloff of a DRT that is Saturation Preserving (p. 240), in nits: saturation rolls
/// off from `start` and is gone at `limit`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SatRolloff {
    /// Sat. Rolloff Start.
    pub start: f64,
    /// Sat. Rolloff Limit, above `start`.
    pub limit: f64,
}

impl Default for SatRolloff {
    /// Ours: the manual gives no defaults.
    fn default() -> Self {
        Self {
            start: 100.0,
            limit: 1000.0,
        }
    }
}

/// The colour space that resizing is done in, by Apply resize transformations in (p. 243).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResizeSpace {
    /// Timeline.
    Timeline,
    /// Log.
    Log,
    /// Linear: what resizing is "ordinarily" done in.
    #[default]
    Linear,
    /// Linear Mapped.
    LinearMapped,
    /// Gamma.
    Gamma,
    /// Gamma Mapped.
    GammaMapped,
}

/// The version of ACES (pp. 150, 247). Resolve offers 1.0.3, 1.1, 1.2, 1.3 and 2.0; OCIO has
/// built-in configs of 1.3 and of 2.0, so those are offered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AcesVersion {
    /// ACES 1.3.
    #[serde(rename = "1.3")]
    V1_3,
    /// ACES 2.0, "the latest version". Ours as the default.
    #[default]
    #[serde(rename = "2.0")]
    V2_0,
}

/// What the picture of a project in ACES is put out as, by an output device transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OdtOutput {
    /// A view of a display of the built-in config of the version.
    View {
        display: &'static str,
        view: &'static str,
    },
    /// A space of the catalogue, without tone mapping (`ACEScct`, `ACEScc`, `ACEScg`).
    Space(&'static str),
    /// No Output Device Transform: ACES2065-1, linear (p. 252).
    Aces,
}

/// An output device transform of ACES: its name, as the ODT menu shows it (pp. 248–250), and
/// what it puts the picture out as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Odt {
    pub name: &'static str,
    pub output: OdtOutput,
}

const fn view(name: &'static str, display: &'static str, view: &'static str) -> Odt {
    Odt {
        name,
        output: OdtOutput::View { display, view },
    }
}

/// The ODTs of ACES 2.0: every tone-mapped view of every display of OCIO's built-in studio
/// config of ACES 2, named in the manner of pp. 248–250, and the spaces of ACES.
const ODTS_2_0: [Odt; 28] = [
    view(
        "Rec.709",
        "Rec.1886 Rec.709 - Display",
        "ACES 2.0 - SDR 100 nits (Rec.709)",
    ),
    view(
        "Rec.709 (Gamma 2.2)",
        "Gamma 2.2 Rec.709 - Display",
        "ACES 2.0 - SDR 100 nits (Rec.709)",
    ),
    view(
        "sRGB",
        "sRGB - Display",
        "ACES 2.0 - SDR 100 nits (Rec.709)",
    ),
    view(
        "Display P3",
        "Display P3 - Display",
        "ACES 2.0 - SDR 100 nits (P3 D65)",
    ),
    view(
        "Display P3 HDR (1000 nits)",
        "Display P3 HDR - Display",
        "ACES 2.0 - HDR 1000 nits (P3 D65)",
    ),
    view(
        "P3-D65",
        "Gamma 2.6 P3-D65 - Display",
        "ACES 2.0 - SDR 100 nits (P3 D65)",
    ),
    view(
        "P3-D65 (Rec.709 Limited)",
        "Gamma 2.6 P3-D65 - Display",
        "ACES 2.0 - SDR 100 nits (Rec.709)",
    ),
    view(
        "P3-D65 ST2084 (108 nits)",
        "ST2084-P3-D65 - Display",
        "ACES 2.0 - HDR 108 nits (P3 D65)",
    ),
    view(
        "P3-D65 ST2084 (500 nits)",
        "ST2084-P3-D65 - Display",
        "ACES 2.0 - HDR 500 nits (P3 D65)",
    ),
    view(
        "P3-D65 ST2084 (1000 nits)",
        "ST2084-P3-D65 - Display",
        "ACES 2.0 - HDR 1000 nits (P3 D65)",
    ),
    view(
        "P3-D65 ST2084 (2000 nits)",
        "ST2084-P3-D65 - Display",
        "ACES 2.0 - HDR 2000 nits (P3 D65)",
    ),
    view(
        "P3-D65 ST2084 (4000 nits)",
        "ST2084-P3-D65 - Display",
        "ACES 2.0 - HDR 4000 nits (P3 D65)",
    ),
    view(
        "Rec.2020 ST2084 (500 nits)",
        "Rec.2100-PQ - Display",
        "ACES 2.0 - HDR 500 nits (Rec.2020)",
    ),
    view(
        "Rec.2020 ST2084 (1000 nits)",
        "Rec.2100-PQ - Display",
        "ACES 2.0 - HDR 1000 nits (Rec.2020)",
    ),
    view(
        "Rec.2020 ST2084 (2000 nits)",
        "Rec.2100-PQ - Display",
        "ACES 2.0 - HDR 2000 nits (Rec.2020)",
    ),
    view(
        "Rec.2020 ST2084 (4000 nits)",
        "Rec.2100-PQ - Display",
        "ACES 2.0 - HDR 4000 nits (Rec.2020)",
    ),
    view(
        "Rec.2020 ST2084 (500 nits, P3-D65 Limited)",
        "Rec.2100-PQ - Display",
        "ACES 2.0 - HDR 500 nits (P3 D65)",
    ),
    view(
        "Rec.2020 ST2084 (1000 nits, P3-D65 Limited)",
        "Rec.2100-PQ - Display",
        "ACES 2.0 - HDR 1000 nits (P3 D65)",
    ),
    view(
        "Rec.2020 ST2084 (2000 nits, P3-D65 Limited)",
        "Rec.2100-PQ - Display",
        "ACES 2.0 - HDR 2000 nits (P3 D65)",
    ),
    view(
        "Rec.2020 ST2084 (4000 nits, P3-D65 Limited)",
        "Rec.2100-PQ - Display",
        "ACES 2.0 - HDR 4000 nits (P3 D65)",
    ),
    view(
        "Rec.2020 HLG (1000 nits, P3-D65 Limited)",
        "Rec.2100-HLG - Display",
        "ACES 2.0 - HDR 1000 nits (P3 D65)",
    ),
    // The views of SDR on the displays of HDR, for a grade of SDR seen on one.
    view(
        "Display P3 HDR (100 nits, Rec.709 Limited)",
        "Display P3 HDR - Display",
        "ACES 2.0 - SDR 100 nits (Rec.709)",
    ),
    view(
        "Rec.2020 ST2084 (100 nits, Rec.709 Limited)",
        "Rec.2100-PQ - Display",
        "ACES 2.0 - SDR 100 nits (Rec.709)",
    ),
    view(
        "P3-D65 ST2084 (100 nits, Rec.709 Limited)",
        "ST2084-P3-D65 - Display",
        "ACES 2.0 - SDR 100 nits (Rec.709)",
    ),
    Odt {
        name: "ACEScct",
        output: OdtOutput::Space("ACEScct"),
    },
    Odt {
        name: "ACEScc",
        output: OdtOutput::Space("ACEScc"),
    },
    Odt {
        name: "ACEScg",
        output: OdtOutput::Space("ACEScg"),
    },
    Odt {
        name: "No Output Device Transform",
        output: OdtOutput::Aces,
    },
];

/// The ODTs of ACES 1.3: every tone-mapped view of the displays of OCIO's last built-in studio
/// config of ACES 1.3, by the names of pp. 248–250, and the spaces of ACES. Resolve's ADX and
/// DCDM are not in it.
const ODTS_1_3: [Odt; 27] = [
    view(
        "Rec.709",
        "Rec.1886 Rec.709 - Display",
        "ACES 1.0 - SDR Video",
    ),
    view(
        "Rec.709 (D60 sim.)",
        "Rec.1886 Rec.709 - Display",
        "ACES 1.0 - SDR Video (D60 sim on D65)",
    ),
    view("sRGB", "sRGB - Display", "ACES 1.0 - SDR Video"),
    view(
        "sRGB (D60 sim.)",
        "sRGB - Display",
        "ACES 1.0 - SDR Video (D60 sim on D65)",
    ),
    view("Display P3", "Display P3 - Display", "ACES 1.0 - SDR Video"),
    view(
        "Display P3 (D60 sim.)",
        "Display P3 - Display",
        "ACES 1.0 - SDR Video (D60 sim on D65)",
    ),
    view("P3-D60", "P3-D60 - Display", "ACES 1.0 - SDR Cinema"),
    view("P3-D65", "P3-D65 - Display", "ACES 1.0 - SDR Cinema"),
    view(
        "P3-D65 (D60 sim.)",
        "P3-D65 - Display",
        "ACES 1.1 - SDR Cinema (D60 sim on D65)",
    ),
    view(
        "P3-D65 (Rec.709 Limited)",
        "P3-D65 - Display",
        "ACES 1.1 - SDR Cinema (Rec.709 lim)",
    ),
    view(
        "P3-DCI (D60 sim.)",
        "P3-DCI - Display",
        "ACES 1.0 - SDR Cinema (D60 sim on DCI)",
    ),
    view(
        "P3-DCI (D65 sim.)",
        "P3-DCI - Display",
        "ACES 1.1 - SDR Cinema (D65 sim on DCI)",
    ),
    view(
        "Rec.2020",
        "Rec.1886 Rec.2020 - Display",
        "ACES 1.0 - SDR Video",
    ),
    view(
        "Rec.2020 (P3-D65 Limited)",
        "Rec.1886 Rec.2020 - Display",
        "ACES 1.1 - SDR Video (P3 lim)",
    ),
    view(
        "Rec.2020 (Rec.709 Limited)",
        "Rec.1886 Rec.2020 - Display",
        "ACES 1.1 - SDR Video (Rec.709 lim)",
    ),
    view(
        "Rec.2020 HLG (1000 nits)",
        "Rec.2100-HLG - Display",
        "ACES 1.1 - HDR Video (1000 nits & Rec.2020 lim)",
    ),
    view(
        "Rec.2020 ST2084 (1000 nits)",
        "Rec.2100-PQ - Display",
        "ACES 1.1 - HDR Video (1000 nits & Rec.2020 lim)",
    ),
    view(
        "Rec.2020 ST2084 (2000 nits)",
        "Rec.2100-PQ - Display",
        "ACES 1.1 - HDR Video (2000 nits & Rec.2020 lim)",
    ),
    view(
        "Rec.2020 ST2084 (4000 nits)",
        "Rec.2100-PQ - Display",
        "ACES 1.1 - HDR Video (4000 nits & Rec.2020 lim)",
    ),
    view(
        "P3-D65 ST2084 (108 nits)",
        "ST2084-P3-D65 - Display",
        "ACES 1.1 - HDR Cinema (108 nits & P3 lim)",
    ),
    view(
        "P3-D65 ST2084 (1000 nits)",
        "ST2084-P3-D65 - Display",
        "ACES 1.1 - HDR Video (1000 nits & P3 lim)",
    ),
    view(
        "P3-D65 ST2084 (2000 nits)",
        "ST2084-P3-D65 - Display",
        "ACES 1.1 - HDR Video (2000 nits & P3 lim)",
    ),
    view(
        "P3-D65 ST2084 (4000 nits)",
        "ST2084-P3-D65 - Display",
        "ACES 1.1 - HDR Video (4000 nits & P3 lim)",
    ),
    Odt {
        name: "ACEScct",
        output: OdtOutput::Space("ACEScct"),
    },
    Odt {
        name: "ACEScc",
        output: OdtOutput::Space("ACEScc"),
    },
    Odt {
        name: "ACEScg",
        output: OdtOutput::Space("ACEScg"),
    },
    Odt {
        name: "No Output Device Transform",
        output: OdtOutput::Aces,
    },
];

impl AcesVersion {
    /// The built-in config of OCIO that its transforms are of.
    #[must_use]
    pub fn config(self) -> &'static str {
        match self {
            Self::V1_3 => "ocio://studio-config-v2.2.0_aces-v1.3_ocio-v2.4",
            Self::V2_0 => "ocio://studio-config-v5.0.0_aces-v2.1_ocio-v2.6",
        }
    }

    /// Its output device transforms, in the order of the menu.
    #[must_use]
    pub fn odts(self) -> &'static [Odt] {
        match self {
            Self::V1_3 => &ODTS_1_3,
            Self::V2_0 => &ODTS_2_0,
        }
    }

    /// Its output device transform named `name`, if it has one.
    #[must_use]
    pub fn odt(self, name: &str) -> Option<&'static Odt> {
        self.odts().iter().find(|odt| odt.name == name)
    }
}

/// What the LUTs of a project are processed in, in ACES: Process Node LUTs in (p. 250).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeLutSpace {
    /// `ACEScc` AP1 Timeline Space: the timeline's space of ACES, the default.
    #[default]
    Timeline,
    /// ACES AP0 Linear.
    Ap0Linear,
}

/// How a 3D LUT is read between its points: 3D Lookup Table Interpolation (pp. 152–153).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LutInterpolation {
    /// Trilinear, the default.
    #[default]
    Trilinear,
    /// Tetrahedral.
    Tetrahedral,
}

/// The LUTs of a stage of the project's Lookup Tables (pp. 151–152): a 1D and a 3D LUT, the
/// 1D first, each by its name relative to the folders of LUTs, or none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LutPair {
    /// The 1D LUT.
    #[serde(rename = "1d")]
    pub one: Option<String>,
    /// The 3D LUT.
    #[serde(rename = "3d")]
    pub three: Option<String>,
}

/// The colour management of a project: the Color Space & Transforms and the Lookup Tables of
/// the Color Management panel of the Project Settings, and the Use S-curve for contrast of its
/// General Options (p. 156).
///
/// It is always written; a field that is left out is read as its default, as is the whole of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools, reason = "the checkboxes of the panel")]
pub struct ColorSettings {
    /// Color science (pp. 149, 224–226).
    pub science: Science,
    /// Automatic color management (p. 230). Ours: off.
    pub automatic: bool,
    /// Color processing mode, of automatic colour management (p. 230).
    pub processing_mode: ProcessingMode,
    /// Resolve color management preset (pp. 230–232).
    pub preset: Preset,
    /// Output color space (pp. 227, 232): a space, or [`BYPASS`]. Ours: as SDR Rec.709 has it.
    pub output: String,
    /// Use Separate Color Space and Gamma (pp. 150, 234–235): off, the default. How the menus
    /// show a space; the space is kept by its one name.
    pub separate_gamma: bool,
    /// Input color space (pp. 230, 235): a space, or [`BYPASS`]; Rec.709 Gamma 2.4.
    pub input: String,
    /// Timeline color space (pp. 226, 236): a space; Rec.709 Gamma 2.4.
    pub timeline: String,
    /// Timeline working luminance (p. 238).
    pub working_luminance: WorkingLuminance,
    /// The nits of a working luminance that is Custom (p. 238), from 1 to 10000. Ours: 100.
    pub custom_luminance: f64,
    /// Use 203 nits reference for Rec.2100 HDR (pp. 238–239): off. The checkbox is shown only
    /// when the output is HDR and the timeline SDR.
    pub hdr_203_nits: bool,
    /// Limit output gamut to (p. 239): a gamut, or none, the default.
    pub limit_gamut: Option<Gamut>,
    /// Input DRT (pp. 239–240). Ours: None.
    pub input_drt: ToneMapping,
    /// The Sat. Rolloff of the Input DRT, where it is Saturation Preserving (p. 240).
    pub input_sat_rolloff: SatRolloff,
    /// Output DRT (pp. 240–242). Ours: None. Resolve's RED IPP2 is not offered.
    pub output_drt: ToneMapping,
    /// The Sat. Rolloff of the Output DRT, where it is Saturation Preserving (p. 242).
    pub output_sat_rolloff: SatRolloff,
    /// Use inverse DRT for SDR to HDR conversion (p. 242). Ours: off.
    pub inverse_drt: bool,
    /// Use white point adaptation (p. 242). Ours: on.
    pub white_point_adaptation: bool,
    /// Use color space aware grading tools (p. 243): on, the default.
    pub color_space_aware_tools: bool,
    /// Apply resize transformations in (p. 243).
    pub resize_in: ResizeSpace,
    /// Graphics white level (p. 243), in nits, from 1 to 10000. Ours: 100.
    pub graphics_white: f64,
    /// ACES version (pp. 150, 247).
    pub aces_version: AcesVersion,
    /// ACES Input Device Transform (pp. 247–248): a space. Ours: Rec.709 Gamma 2.4.
    pub idt: String,
    /// ACES Output Device Transform (pp. 248–250, 252): one of the ODTs of `aces_version`. Ours:
    /// Rec.709 ("You must manually select an ODT").
    pub odt: String,
    /// Apply ACES reference gamut compress (p. 251). Ours: off.
    pub gamut_compress: bool,
    /// Process Node LUTs in (p. 250).
    pub node_luts_in: NodeLutSpace,
    /// 3D Lookup Table Interpolation (pp. 152–153).
    pub lut_interpolation: LutInterpolation,
    /// Input Lookup Table: before everything else (p. 152).
    pub input_lut: LutPair,
    /// Output Lookup Table: after the Color page, before the display LUT (p. 152).
    pub output_lut: LutPair,
    /// Color Viewer Lookup Table: of the viewer only (p. 152); none, as there is no Video
    /// Monitor LUT for it to follow.
    pub viewer_lut: LutPair,
    /// Use S-curve for contrast (p. 156): on, the default.
    pub s_curve_contrast: bool,
}

impl Default for ColorSettings {
    fn default() -> Self {
        Self {
            science: Science::Yrgb,
            automatic: false,
            processing_mode: ProcessingMode::Sdr,
            preset: Preset::SdrRec709,
            output: REC709.into(),
            separate_gamma: false,
            input: REC709.into(),
            timeline: REC709.into(),
            working_luminance: WorkingLuminance::Sdr100,
            custom_luminance: 100.0,
            hdr_203_nits: false,
            limit_gamut: None,
            input_drt: ToneMapping::None,
            input_sat_rolloff: SatRolloff::default(),
            output_drt: ToneMapping::None,
            output_sat_rolloff: SatRolloff::default(),
            inverse_drt: false,
            white_point_adaptation: true,
            color_space_aware_tools: true,
            resize_in: ResizeSpace::Linear,
            graphics_white: 100.0,
            aces_version: AcesVersion::V2_0,
            idt: REC709.into(),
            odt: "Rec.709".into(),
            gamut_compress: false,
            node_luts_in: NodeLutSpace::Timeline,
            lut_interpolation: LutInterpolation::Trilinear,
            input_lut: LutPair::default(),
            output_lut: LutPair::default(),
            viewer_lut: LutPair::default(),
            s_curve_contrast: true,
        }
    }
}

/// When a setting applies (see the module's documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Always,
    Rcm,
    Automatic,
    Manual,
    Custom,
    CustomLuminance,
    InputRolloff,
    OutputRolloff,
    Aces,
}

/// When the setting of the name that it is written by applies, if it is one.
fn scope(setting: &str) -> Option<Scope> {
    Some(match setting {
        "science" | "lut_interpolation" | "input_lut" | "output_lut" | "viewer_lut"
        | "s_curve_contrast" => Scope::Always,
        "automatic" | "output" | "graphics_white" => Scope::Rcm,
        "processing_mode" => Scope::Automatic,
        "preset" => Scope::Manual,
        "separate_gamma"
        | "input"
        | "timeline"
        | "working_luminance"
        | "hdr_203_nits"
        | "limit_gamut"
        | "input_drt"
        | "output_drt"
        | "inverse_drt"
        | "white_point_adaptation"
        | "color_space_aware_tools"
        | "resize_in" => Scope::Custom,
        "custom_luminance" => Scope::CustomLuminance,
        "input_sat_rolloff" => Scope::InputRolloff,
        "output_sat_rolloff" => Scope::OutputRolloff,
        "aces_version" | "idt" | "odt" | "gamut_compress" | "node_luts_in" => Scope::Aces,
        _ => return None,
    })
}

/// The settings of colour management that are in force.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pipeline<'a> {
    /// `DaVinci YRGB`: no transform of colour.
    Unmanaged,
    /// Resolve Color Management.
    Rcm(Rcm<'a>),
    /// `ACEScct` or `ACEScc`.
    Aces(Aces<'a>),
}

/// The settings of Resolve Color Management in force: those of the preset, or of Custom.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools, reason = "the checkboxes of the panel")]
pub struct Rcm<'a> {
    /// The input space of media that says none, or [`BYPASS`].
    pub input: &'a str,
    pub timeline: &'a str,
    /// The output space, or [`BYPASS`].
    pub output: &'a str,
    /// The nits that pictures are mapped to and those that the timeline clips at.
    pub luminance: (f64, f64),
    pub input_drt: ToneMapping,
    pub input_sat_rolloff: SatRolloff,
    pub output_drt: ToneMapping,
    pub output_sat_rolloff: SatRolloff,
    pub limit_gamut: Option<Gamut>,
    pub hdr_203_nits: bool,
    pub inverse_drt: bool,
    pub white_point_adaptation: bool,
    pub color_space_aware_tools: bool,
    pub resize_in: ResizeSpace,
    pub graphics_white: f64,
}

/// The settings of ACES in force.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aces<'a> {
    pub version: AcesVersion,
    /// The space that grades are done in: `ACEScct` or `ACEScc`, as the science is (p. 252).
    pub timeline: &'static str,
    /// The input space of media that says none.
    pub idt: &'a str,
    pub odt: &'static Odt,
    pub gamut_compress: bool,
    pub node_luts_in: NodeLutSpace,
}

impl ColorSettings {
    /// Whether the setting of the name that it is written by applies to these settings.
    #[must_use]
    pub fn applies(&self, setting: &str) -> bool {
        let rcm = self.science == Science::YrgbManaged;
        let custom = rcm && !self.automatic && self.preset == Preset::Custom;
        match scope(setting) {
            None => false,
            Some(Scope::Always) => true,
            Some(Scope::Rcm) => rcm,
            Some(Scope::Automatic) => rcm && self.automatic,
            Some(Scope::Manual) => rcm && !self.automatic,
            Some(Scope::Custom) => custom,
            Some(Scope::CustomLuminance) => {
                custom && self.working_luminance == WorkingLuminance::Custom
            }
            Some(Scope::InputRolloff) => {
                custom && self.input_drt == ToneMapping::SaturationPreserving
            }
            Some(Scope::OutputRolloff) => {
                custom && self.output_drt == ToneMapping::SaturationPreserving
            }
            Some(Scope::Aces) => self.science.is_aces(),
        }
    }

    /// The settings of colour management that are in force.
    ///
    /// # Panics
    /// Only if the ODT is not one of its version, which no project has: its settings are
    /// checked so when they are set and when they are read.
    #[must_use]
    pub fn pipeline(&self) -> Pipeline<'_> {
        match self.science {
            Science::Yrgb => Pipeline::Unmanaged,
            Science::Acescct | Science::Acescc => Pipeline::Aces(Aces {
                version: self.aces_version,
                timeline: if self.science == Science::Acescct {
                    "ACEScct"
                } else {
                    "ACEScc"
                },
                idt: &self.idt,
                odt: self
                    .aces_version
                    .odt(&self.odt)
                    .expect("the ODT of settings that were checked is of its version"),
                gamut_compress: self.gamut_compress,
                node_luts_in: self.node_luts_in,
            }),
            Science::YrgbManaged => {
                let preset = if self.automatic {
                    self.processing_mode.preset()
                } else {
                    self.preset
                };
                let custom = Rcm {
                    input: &self.input,
                    timeline: &self.timeline,
                    output: &self.output,
                    luminance: self.working_luminance.nits(self.custom_luminance),
                    input_drt: self.input_drt,
                    input_sat_rolloff: self.input_sat_rolloff,
                    output_drt: self.output_drt,
                    output_sat_rolloff: self.output_sat_rolloff,
                    limit_gamut: self.limit_gamut,
                    hdr_203_nits: self.hdr_203_nits,
                    inverse_drt: self.inverse_drt,
                    white_point_adaptation: self.white_point_adaptation,
                    color_space_aware_tools: self.color_space_aware_tools,
                    resize_in: self.resize_in,
                    graphics_white: self.graphics_white,
                };
                Pipeline::Rcm(match preset.settings() {
                    None => custom,
                    Some(set) => {
                        let defaults = Self::default();
                        Rcm {
                            input: REC709,
                            timeline: set.timeline,
                            luminance: set.working_luminance.nits(defaults.custom_luminance),
                            input_drt: set.input_drt,
                            input_sat_rolloff: defaults.input_sat_rolloff,
                            output_drt: set.output_drt,
                            output_sat_rolloff: defaults.output_sat_rolloff,
                            limit_gamut: set.limit_gamut,
                            hdr_203_nits: defaults.hdr_203_nits,
                            inverse_drt: defaults.inverse_drt,
                            white_point_adaptation: defaults.white_point_adaptation,
                            color_space_aware_tools: defaults.color_space_aware_tools,
                            resize_in: defaults.resize_in,
                            ..custom
                        }
                    }
                })
            }
        }
    }

    /// Check that every setting is one that it may be, whether it applies or not: each space is
    /// one of the catalogue (or Bypass, for the input and the output), the ODT one of the
    /// version, each number of nits within 1 to 10000 with each rolloff starting below its
    /// limit, the gamut limit not Camera Native, and each LUT a name relative to the folders of
    /// LUTs.
    pub(crate) fn check(&self) -> Result<(), OpError> {
        let unknown = |setting: &'static str, name: &str| OpError::UnknownSpace {
            setting,
            name: name.into(),
        };
        for (setting, name, bypass) in [
            ("input", &self.input, true),
            ("timeline", &self.timeline, false),
            ("output", &self.output, true),
            ("idt", &self.idt, false),
        ] {
            if !(is_space(name) || bypass && name == BYPASS) {
                return Err(unknown(setting, name));
            }
        }
        if self.aces_version.odt(&self.odt).is_none() {
            return Err(unknown("odt", &self.odt));
        }
        if self.limit_gamut == Some(Gamut::CameraNative) {
            return Err(OpError::OutOfRange {
                what: "limit_gamut",
            });
        }
        for (what, nits) in [
            ("custom_luminance", self.custom_luminance),
            ("graphics_white", self.graphics_white),
        ] {
            if !in_nits(nits) {
                return Err(OpError::OutOfRange { what });
            }
        }
        for (what, rolloff) in [
            ("input_sat_rolloff", self.input_sat_rolloff),
            ("output_sat_rolloff", self.output_sat_rolloff),
        ] {
            if !(in_nits(rolloff.start) && in_nits(rolloff.limit) && rolloff.start < rolloff.limit)
            {
                return Err(OpError::OutOfRange { what });
            }
        }
        for (what, pair) in [
            ("input_lut", &self.input_lut),
            ("output_lut", &self.output_lut),
            ("viewer_lut", &self.viewer_lut),
        ] {
            if [&pair.one, &pair.three]
                .into_iter()
                .flatten()
                .any(|name| !is_lut_name(name))
            {
                return Err(OpError::OutOfRange { what });
            }
        }
        Ok(())
    }
}

/// Whether a number of nits is within 1 to 10000.
fn in_nits(nits: f64) -> bool {
    (NITS.0..=NITS.1).contains(&nits)
}

/// Whether `name` names a LUT relative to the folders of LUTs: not empty, of at most
/// [`LONGEST_PATH`] characters and no NUL, not absolute (no leading `/` or `\`, no drive or
/// stream, so no `:`), and never going up (no part `..`).
fn is_lut_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= LONGEST_PATH
        && !name.contains(['\0', ':'])
        && !name.starts_with(['/', '\\'])
        && name.split(['/', '\\']).all(|part| part != "..")
}

// --- media ------------------------------------------------------------------------------------

/// How the codes of the picture of media are read: Data Levels, in the Clip Attributes (pp.
/// 221–222).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Levels {
    /// Auto: as the file says, the default when media is taken in.
    #[default]
    Auto,
    /// Video: limited, 64–940 for Y' in 10 bits.
    Video,
    /// Full.
    Full,
}

/// The colour of media set by hand in the media pool: its Input Color Space (pp. 228–230, 245),
/// its Data Levels (pp. 221–222), the LUTs of the source clip (pp. 3409–3410) and whether its
/// Inverse DRT for SDR to HDR Conversion is on (p. 233).
///
/// It is always written; a field that is left out is read as its default, as is the whole of it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ColorOverride {
    /// A space of the catalogue that the picture is in, over what its file says and the
    /// project's default; [`BYPASS`], for Bypass Color Management; or none, for the project's
    /// default (the Project Default of the menu), the default. Camera raw has none.
    pub input: Option<String>,
    /// Its data levels.
    pub levels: Levels,
    /// The LUTs applied to the source clip, from the 1D LUT ▸ and 3D LUT ▸ menus of the media pool
    /// and the thumbnails of the Color page (pp. 3409–3410), each by its name relative to the
    /// folders of LUTs, the 1D first; none, the default. They are the media's, not a grade's:
    /// "source clip LUTs cannot be copied from one timeline to another using `ColorTrace`".
    pub lut: LutPair,
    /// Inverse DRT for SDR to HDR Conversion of this media (p. 233): on, the default, where the
    /// project's "Use inverse DRT for SDR to HDR conversion" maps it up; off, for a clip whose
    /// "large flat expanses of bright colors" the expansion makes noisy.
    pub inverse_drt: bool,
}

impl Default for ColorOverride {
    fn default() -> Self {
        ColorOverride {
            input: None,
            levels: Levels::default(),
            lut: LutPair::default(),
            inverse_drt: true,
        }
    }
}

/// Check the colour that media is set to: an input space of the catalogue, or Bypass, and none
/// for camera raw, whose space its maker's colour science decides (p. 228); and nothing but the
/// default for media without a picture.
pub(crate) fn check_override(media: &Media) -> Result<(), OpError> {
    let set = &media.color_override;
    if let Some(name) = &set.input {
        if !(is_space(name) || name == BYPASS) {
            return Err(OpError::UnknownSpace {
                setting: "input",
                name: name.clone(),
            });
        }
    }
    if [&set.lut.one, &set.lut.three]
        .into_iter()
        .flatten()
        .any(|name| !is_lut_name(name))
    {
        return Err(OpError::OutOfRange { what: "media_lut" });
    }
    let Some(video) = &media.facts.video else {
        if *set != ColorOverride::default() {
            return Err(OpError::NoStream {
                media: media.id.clone(),
                kind: crate::project::TrackKind::Video,
            });
        }
        return Ok(());
    };
    if set.input.as_deref().is_some_and(|name| name != BYPASS)
        && CAMERA_RAW_CODECS.contains(&video.codec.as_str())
    {
        return Err(OpError::CameraRaw {
            media: media.id.clone(),
        });
    }
    Ok(())
}

/// The input space of media, as colour management decides it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaInput {
    /// A space of the catalogue.
    Space(String),
    /// Bypass: the picture is not converted.
    Bypass,
    /// Camera raw, in its maker's space.
    CameraRaw,
}

/// The input space of the picture of `media` in a project of `settings`, and what decided it, as
/// [`input_space`] decides it (p. 228–230): camera raw by its maker; else the space set by hand;
/// else the metadata of the file; else the default input space in force (of RCM, or the IDT of
/// ACES). None where the project is not managed, or the media has no picture.
#[must_use]
pub fn media_input(settings: &ColorSettings, media: &Media) -> Option<(MediaInput, Basis)> {
    let default = match settings.pipeline() {
        Pipeline::Unmanaged => return None,
        Pipeline::Rcm(rcm) => rcm.input,
        Pipeline::Aces(aces) => aces.idt,
    };
    let video = media.facts.video.as_ref()?;
    if CAMERA_RAW_CODECS.contains(&video.codec.as_str()) {
        return Some((MediaInput::CameraRaw, Basis::CameraRaw));
    }
    match media.color_override.input.as_deref() {
        Some(BYPASS) => return Some((MediaInput::Bypass, Basis::Manual)),
        Some(name) => return Some((MediaInput::Space(name.into()), Basis::Manual)),
        None => {}
    }
    // What the file does not say of a gamut or a gamma is the default's, where it is of media.
    let fill = media_space(default).unwrap_or(InputSpace::REC709);
    let found = match input_space(video, None, fill) {
        (space, Basis::Metadata) => MediaInput::Space(space.name()),
        _ if default == BYPASS => return Some((MediaInput::Bypass, Basis::Default)),
        _ => return Some((MediaInput::Space(default.into()), Basis::Default)),
    };
    Some((found, Basis::Metadata))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_has_its_68_names_once_each() {
        let names = spaces();
        assert_eq!(names.len(), 68);
        let mut sorted = names.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 68);
        assert_eq!(names[0], REC709);
        assert!(is_space("Rec.2100 ST2084") && is_space("sRGB") && is_space("Apple Log"));
        assert!(!is_space(BYPASS) && !is_space("Camera Native Linear") && !is_space(""));
        assert_eq!(media_space("P3-D65 ST2084").unwrap().gamma, Gamma::St2084);
        assert_eq!(media_space("Apple Log"), None);
    }

    #[test]
    fn a_project_is_unmanaged_and_written_with_every_setting_by_default() {
        let settings = ColorSettings::default();
        assert_eq!(settings.pipeline(), Pipeline::Unmanaged);
        settings.check().unwrap();
        let written = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "science": "yrgb",
                "automatic": false,
                "processing_mode": "sdr",
                "preset": "sdr_rec709",
                "output": "Rec.709 Gamma 2.4",
                "separate_gamma": false,
                "input": "Rec.709 Gamma 2.4",
                "timeline": "Rec.709 Gamma 2.4",
                "working_luminance": "sdr_100",
                "custom_luminance": 100.0,
                "hdr_203_nits": false,
                "limit_gamut": null,
                "input_drt": "none",
                "input_sat_rolloff": { "start": 100.0, "limit": 1000.0 },
                "output_drt": "none",
                "output_sat_rolloff": { "start": 100.0, "limit": 1000.0 },
                "inverse_drt": false,
                "white_point_adaptation": true,
                "color_space_aware_tools": true,
                "resize_in": "linear",
                "graphics_white": 100.0,
                "aces_version": "2.0",
                "idt": "Rec.709 Gamma 2.4",
                "odt": "Rec.709",
                "gamut_compress": false,
                "node_luts_in": "timeline",
                "lut_interpolation": "trilinear",
                "input_lut": { "1d": null, "3d": null },
                "output_lut": { "1d": null, "3d": null },
                "viewer_lut": { "1d": null, "3d": null },
                "s_curve_contrast": true,
            })
        );
        // Every setting that is written has a rule of when it applies.
        for name in written.as_object().unwrap().keys() {
            assert!(scope(name).is_some(), "{name}");
        }
        // Each field may be left out, and so may all of them.
        assert_eq!(
            serde_json::from_value::<ColorSettings>(serde_json::json!({})).unwrap(),
            settings
        );
        let some: ColorSettings =
            serde_json::from_value(serde_json::json!({"science": "acescct"})).unwrap();
        assert_eq!(some.science, Science::Acescct);
        assert_eq!(some.odt, "Rec.709");
        assert!(serde_json::from_value::<ColorSettings>(serde_json::json!({"gamma": 1})).is_err());
    }

    #[test]
    fn each_setting_applies_to_its_science_and_preset() {
        let mut settings = ColorSettings::default();
        let applying = |settings: &ColorSettings| -> Vec<String> {
            let written = serde_json::to_value(settings).unwrap();
            let mut found: Vec<String> = written
                .as_object()
                .unwrap()
                .keys()
                .filter(|name| settings.applies(name))
                .cloned()
                .collect();
            found.sort_unstable();
            found
        };
        let always = [
            "input_lut",
            "lut_interpolation",
            "output_lut",
            "s_curve_contrast",
            "science",
            "viewer_lut",
        ];
        assert_eq!(applying(&settings), always);
        settings.science = Science::YrgbManaged;
        let mut rcm = [
            &always[..],
            &["automatic", "graphics_white", "output", "preset"],
        ]
        .concat();
        rcm.sort_unstable();
        assert_eq!(applying(&settings), rcm);
        settings.automatic = true;
        assert!(settings.applies("processing_mode") && !settings.applies("preset"));
        settings.automatic = false;
        settings.preset = Preset::Custom;
        for name in [
            "input",
            "timeline",
            "limit_gamut",
            "output_drt",
            "resize_in",
        ] {
            assert!(settings.applies(name), "{name}");
        }
        for name in [
            "custom_luminance",
            "input_sat_rolloff",
            "output_sat_rolloff",
            "odt",
        ] {
            assert!(!settings.applies(name), "{name}");
        }
        settings.working_luminance = WorkingLuminance::Custom;
        settings.output_drt = ToneMapping::SaturationPreserving;
        assert!(settings.applies("custom_luminance") && settings.applies("output_sat_rolloff"));
        assert!(!settings.applies("input_sat_rolloff"));
        settings.science = Science::Acescc;
        let mut aces = [
            &always[..],
            &[
                "aces_version",
                "gamut_compress",
                "idt",
                "node_luts_in",
                "odt",
            ],
        ]
        .concat();
        aces.sort_unstable();
        assert_eq!(applying(&settings), aces);
        assert!(!settings.applies("not a setting"));
    }

    #[test]
    fn a_preset_sets_what_is_in_force_and_custom_the_settings_themselves() {
        let mut settings = ColorSettings {
            science: Science::YrgbManaged,
            preset: Preset::DavinciWideGamut,
            timeline: "Apple Log".into(),
            input: "sRGB".into(),
            ..ColorSettings::default()
        };
        let Pipeline::Rcm(rcm) = settings.pipeline() else {
            panic!()
        };
        assert_eq!(rcm.timeline, "DaVinci Wide Gamut/DaVinci Intermediate");
        assert_eq!(rcm.input, REC709);
        assert_eq!(rcm.output, REC709);
        assert_eq!(rcm.luminance, (1000.0, 2000.0));
        assert_eq!(
            (rcm.input_drt, rcm.output_drt),
            (ToneMapping::Davinci, ToneMapping::Davinci)
        );
        settings.preset = Preset::Custom;
        settings.working_luminance = WorkingLuminance::Custom;
        settings.custom_luminance = 600.0;
        let Pipeline::Rcm(rcm) = settings.pipeline() else {
            panic!()
        };
        assert_eq!((rcm.timeline, rcm.input), ("Apple Log", "sRGB"));
        assert_eq!(rcm.luminance, (600.0, 600.0));
        settings.automatic = true;
        settings.processing_mode = ProcessingMode::Hdr;
        let Pipeline::Rcm(rcm) = settings.pipeline() else {
            panic!()
        };
        assert_eq!(rcm.timeline, "Rec.2100 ST2084");
        settings.science = Science::Acescct;
        settings.aces_version = AcesVersion::V1_3;
        settings.odt = "P3-DCI (D65 sim.)".into();
        let Pipeline::Aces(aces) = settings.pipeline() else {
            panic!()
        };
        assert_eq!(aces.timeline, "ACEScct");
        assert_eq!(
            aces.odt.output,
            OdtOutput::View {
                display: "P3-DCI - Display",
                view: "ACES 1.1 - SDR Cinema (D65 sim on DCI)"
            }
        );
        // Every space that a preset sets is of the catalogue.
        for preset in [
            Preset::SdrRec709,
            Preset::SdrP3Broadcast,
            Preset::SdrP3Cinema,
            Preset::SdrRec2020,
            Preset::DavinciWideGamut,
            Preset::HdrP3Broadcast,
            Preset::HdrRec2020,
        ] {
            let set = preset.settings().unwrap();
            assert!(is_space(set.timeline) && is_space(set.output), "{preset:?}");
        }
        assert_eq!(Preset::Custom.settings(), None);
    }

    #[test]
    fn the_odts_have_names_of_their_own_and_rec709_in_each_version() {
        for version in [AcesVersion::V1_3, AcesVersion::V2_0] {
            let names: Vec<&str> = version.odts().iter().map(|odt| odt.name).collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), names.len(), "{version:?}");
            assert!(version.odt("Rec.709").is_some());
            assert!(version.odt("No Output Device Transform").is_some());
            for odt in version.odts() {
                if let OdtOutput::Space(space) = odt.output {
                    assert!(is_space(space), "{space}");
                }
            }
        }
        assert!(AcesVersion::V2_0.odt("P3-DCI (D65 sim.)").is_none());
        assert!(AcesVersion::V1_3
            .odt("Rec.2020 ST2084 (500 nits)")
            .is_none());
    }

    #[test]
    fn settings_out_of_their_lists_or_ranges_are_refused() {
        let wrong = |change: &dyn Fn(&mut ColorSettings)| {
            let mut settings = ColorSettings::default();
            change(&mut settings);
            settings.check().unwrap_err()
        };
        let unknown = |setting, name: &str| OpError::UnknownSpace {
            setting,
            name: name.into(),
        };
        let range = |what| OpError::OutOfRange { what };
        assert_eq!(
            wrong(&|s| s.input = "Rec.709".into()),
            unknown("input", "Rec.709")
        );
        assert_eq!(
            wrong(&|s| s.timeline = BYPASS.into()),
            unknown("timeline", BYPASS)
        );
        assert_eq!(wrong(&|s| s.output = "PQ".into()), unknown("output", "PQ"));
        assert_eq!(wrong(&|s| s.idt = BYPASS.into()), unknown("idt", BYPASS));
        assert_eq!(
            wrong(&|s| s.odt = "P3-DCI (D65 sim.)".into()),
            unknown("odt", "P3-DCI (D65 sim.)")
        );
        let settings = ColorSettings {
            aces_version: AcesVersion::V1_3,
            odt: "Rec.2020 ST2084 (500 nits)".into(),
            ..ColorSettings::default()
        };
        assert_eq!(
            settings.check(),
            Err(unknown("odt", "Rec.2020 ST2084 (500 nits)"))
        );
        assert_eq!(
            wrong(&|s| s.limit_gamut = Some(Gamut::CameraNative)),
            range("limit_gamut")
        );
        for nits in [0.5, 10_001.0, f64::INFINITY] {
            assert_eq!(
                wrong(&|s| s.custom_luminance = nits),
                range("custom_luminance")
            );
            assert_eq!(wrong(&|s| s.graphics_white = nits), range("graphics_white"));
        }
        assert_eq!(
            wrong(&|s| s.input_sat_rolloff = SatRolloff {
                start: 500.0,
                limit: 500.0
            }),
            range("input_sat_rolloff")
        );
        assert_eq!(
            wrong(&|s| s.output_sat_rolloff.limit = 20_000.0),
            range("output_sat_rolloff")
        );
        for name in [
            "",
            "/a.cube",
            "\\a.cube",
            "C:a.cube",
            "a/../../b.cube",
            "a\0",
        ] {
            assert_eq!(
                wrong(&|s| s.input_lut.three = Some(name.into())),
                range("input_lut"),
                "{name:?}"
            );
        }
        assert_eq!(
            wrong(&|s| s.viewer_lut.one = Some("x".repeat(LONGEST_PATH + 1))),
            range("viewer_lut")
        );
        let mut fine = ColorSettings {
            input: BYPASS.into(),
            output: BYPASS.into(),
            ..ColorSettings::default()
        };
        fine.output_lut.three = Some("Film/Kodak 2383.cube".into());
        fine.input_lut.one = Some("looks\\..x\\a.cube".into());
        fine.check().unwrap();
    }
}
