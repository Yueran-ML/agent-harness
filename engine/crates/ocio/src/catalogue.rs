//! The colour spaces that the engine offers, by the names of `DaVinci Resolve`, and the colour
//! space of OCIO that each is in [`Config::columbina`](crate::Config::columbina).
//!
//! # Where they come from
//!
//! - **Of the media** ([`InputSpace`] of the model: a gamut and a gamma, as a file's metadata
//!   says them, p. 228–230): every gamut but Camera Native with every gamma, 48 spaces. Where the
//!   built-in ACES Studio config has the space, it is that one (`Gamma 2.4 Encoded Rec.709`,
//!   `Rec.2100-PQ - Display`, …); the others are ours ([`fragment`]), made of OCIO's own
//!   transforms the same way:
//!   - a gamma of SDR (2.2, 2.4, 2.6, 2.8, sRGB) is *scene-referred* and encoded, as the studio
//!     config's `… Encoded …` spaces are: the inverse of a pure power (or of the sRGB curve) and
//!     the matrix of the gamut, what Resolve does with an SDR clip in a managed project;
//!   - linear is scene-referred linear light;
//!   - ST2084 (PQ) and HLG are *display-referred*, as the studio config's Rec.2100 spaces are:
//!     PQ decodes to nits / 100 in the gamut; HLG is the BT.2100 display of 1000 nits, whose
//!     OOTF works on the luminance of BT.2020 (so for another gamut it is done in BT.2020, and
//!     the gamut changed in scene light).
//!
//!   Gamuts are made of their primaries and white (ITU-R BT.709-6, BT.2020-2; BT.601-7 with the
//!   primaries of EBU Tech 3213 for 625 lines and of SMPTE ST 170 for 525; DCI-P3 and P3-D65 of
//!   SMPTE RP 431-2 and EG 432-1). A white other than D65 (DCI-P3's) is adapted to D65 with
//!   Bradford, as the studio config adapts the white of ACES.
//! - **Of cameras and of grading**: the camera spaces of the studio config (Apple Log, DJI D-Log,
//!   Sony, Panasonic, Canon, ARRI, Blackmagic, RED), `DaVinci` Wide Gamut / Intermediate, the ACES
//!   spaces; and Fujifilm F-Log2 / F-Gamut, which the studio config lacks, made from the F-Log2
//!   Data Sheet Ver.1.0 (Fujifilm, 2022) with OCIO's `LogCameraTransform`.
//!
//! The names of spaces of the media are those of [`InputSpace::name`]; the others are this
//! crate's, in the style of Resolve's menus.

use std::fmt::Write as _;
use std::sync::OnceLock;

use columbina_model::{Gamma, Gamut, InputSpace};

use crate::Referred;

/// What a space is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// A gamut and a gamma of video, of standard range.
    Video,
    /// PQ or HLG.
    Hdr,
    /// Linear light.
    Linear,
    /// A camera's log (or a camera's own encoding), of the maker named.
    Camera(&'static str),
    /// A space to grade in: `DaVinci` Wide Gamut, ACES.
    Working,
}

/// A colour space of the catalogue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Space {
    /// Its name, as Resolve names it.
    pub name: String,
    /// The colour space of OCIO that it is, in [`crate::Config::columbina`].
    pub ocio: String,
    pub family: Family,
    pub referred: Referred,
    /// The space of the model it is, for a space that media may say they are in.
    pub input: Option<InputSpace>,
    /// Whether it is ours ([`fragment`]) rather than the studio config's.
    pub ours: bool,
}

const GAMUTS: [Gamut; 6] = [
    Gamut::Rec709,
    Gamut::Rec601Pal,
    Gamut::Rec601Ntsc,
    Gamut::Rec2020,
    Gamut::DciP3,
    Gamut::P3D65,
];

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

/// The chromaticities (x, y) of a gamut's red, green, blue and white.
fn chromaticities(gamut: Gamut) -> Option<[[f64; 2]; 4]> {
    const D65: [f64; 2] = [0.3127, 0.3290];
    Some(match gamut {
        // ITU-R BT.709-6.
        Gamut::Rec709 => [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06], D65],
        // ITU-R BT.601-7, 625 lines (EBU Tech 3213).
        Gamut::Rec601Pal => [[0.64, 0.33], [0.29, 0.60], [0.15, 0.06], D65],
        // ITU-R BT.601-7, 525 lines (SMPTE ST 170).
        Gamut::Rec601Ntsc => [[0.630, 0.340], [0.310, 0.595], [0.155, 0.070], D65],
        // ITU-R BT.2020-2.
        Gamut::Rec2020 => [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046], D65],
        // SMPTE RP 431-2.
        Gamut::DciP3 => [
            [0.680, 0.320],
            [0.265, 0.690],
            [0.150, 0.060],
            [0.314, 0.351],
        ],
        // SMPTE EG 432-1.
        Gamut::P3D65 => [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060], D65],
        Gamut::CameraNative => return None,
    })
}

/// How a gamut is written in the names of our spaces.
fn gamut_label(gamut: Gamut) -> &'static str {
    match gamut {
        Gamut::Rec709 => "Rec.709",
        Gamut::Rec601Pal => "Rec.601 (PAL)",
        Gamut::Rec601Ntsc => "Rec.601 (NTSC)",
        Gamut::Rec2020 => "Rec.2020",
        Gamut::DciP3 => "DCI-P3",
        Gamut::P3D65 => "P3-D65",
        Gamut::CameraNative => "Camera Native",
    }
}

/// The studio config's colour space of a gamut and gamma, where it has one.
fn studio_space(gamut: Gamut, gamma: Gamma) -> Option<&'static str> {
    Some(match (gamut, gamma) {
        (Gamut::Rec709, Gamma::Rec709) => "Gamma 2.4 Encoded Rec.709",
        (Gamut::Rec709, Gamma::Gamma22) => "Gamma 2.2 Encoded Rec.709",
        (Gamut::Rec709, Gamma::Srgb) => "sRGB Encoded Rec.709 (sRGB)",
        (Gamut::Rec709, Gamma::Linear) => "Linear Rec.709 (sRGB)",
        (Gamut::Rec2020, Gamma::Linear) => "Linear Rec.2020",
        (Gamut::Rec2020, Gamma::St2084) => "Rec.2100-PQ - Display",
        (Gamut::Rec2020, Gamma::Hlg) => "Rec.2100-HLG - Display",
        (Gamut::P3D65, Gamma::Linear) => "Linear P3-D65",
        (Gamut::P3D65, Gamma::Srgb) => "sRGB Encoded P3-D65",
        (Gamut::P3D65, Gamma::St2084) => "ST2084-P3-D65 - Display",
        _ => return None,
    })
}

/// The name of our colour space of a gamut and gamma.
fn our_space(gamut: Gamut, gamma: Gamma) -> String {
    let g = gamut_label(gamut);
    match gamma {
        Gamma::Rec709 => format!("Gamma 2.4 Encoded {g}"),
        Gamma::Gamma22 => format!("Gamma 2.2 Encoded {g}"),
        Gamma::Gamma26 => format!("Gamma 2.6 Encoded {g}"),
        Gamma::Gamma28 => format!("Gamma 2.8 Encoded {g}"),
        Gamma::Srgb => format!("sRGB Encoded {g}"),
        Gamma::Linear => format!("Linear {g}"),
        Gamma::St2084 => format!("ST2084-{g} - Display"),
        Gamma::Hlg => format!("HLG-{g} - Display"),
    }
}

fn referred_of(gamma: Gamma) -> Referred {
    match gamma {
        Gamma::St2084 | Gamma::Hlg => Referred::Display,
        _ => Referred::Scene,
    }
}

fn family_of(gamma: Gamma) -> Family {
    match gamma {
        Gamma::St2084 | Gamma::Hlg => Family::Hdr,
        Gamma::Linear => Family::Linear,
        _ => Family::Video,
    }
}

/// The spaces of cameras and of grading: (Resolve's name, OCIO's, family).
const OTHERS: [(&str, &str, Family); 20] = [
    ("Rec.709 (Scene)", "Camera Rec.709", Family::Video),
    ("Apple Log", "Apple Log", Family::Camera("Apple")),
    ("Apple Log 2", "Apple Log 2", Family::Camera("Apple")),
    ("DJI D-Gamut/D-Log", "D-Log D-Gamut", Family::Camera("DJI")),
    (
        "Sony S-Gamut3/S-Log3",
        "S-Log3 S-Gamut3",
        Family::Camera("Sony"),
    ),
    (
        "Sony S-Gamut3.Cine/S-Log3",
        "S-Log3 S-Gamut3.Cine",
        Family::Camera("Sony"),
    ),
    (
        "Panasonic V-Gamut/V-Log",
        "V-Log V-Gamut",
        Family::Camera("Panasonic"),
    ),
    (
        "Canon Cinema Gamut/Canon Log 2",
        "CanonLog2 CinemaGamut D55",
        Family::Camera("Canon"),
    ),
    (
        "Canon Cinema Gamut/Canon Log 3",
        "CanonLog3 CinemaGamut D55",
        Family::Camera("Canon"),
    ),
    (
        "ARRI Wide Gamut 3/LogC3",
        "ARRI LogC3 (EI800)",
        Family::Camera("ARRI"),
    ),
    (
        "ARRI Wide Gamut 4/LogC4",
        "ARRI LogC4",
        Family::Camera("ARRI"),
    ),
    ("FujiFilm F-Gamut/F-Log2", FLOG2, Family::Camera("Fujifilm")),
    (
        "Blackmagic Design Wide Gamut Gen 5/Film Gen 5",
        "BMDFilm WideGamut Gen5",
        Family::Camera("Blackmagic Design"),
    ),
    (
        "RED Wide Gamut RGB/Log3G10",
        "Log3G10 REDWideGamutRGB",
        Family::Camera("RED"),
    ),
    (
        "DaVinci Wide Gamut/DaVinci Intermediate",
        "DaVinci Intermediate WideGamut",
        Family::Working,
    ),
    (
        "DaVinci Wide Gamut/Linear",
        "Linear DaVinci WideGamut",
        Family::Working,
    ),
    ("ACEScct", "ACEScct", Family::Working),
    ("ACEScc", "ACEScc", Family::Working),
    ("ACEScg", "ACEScg", Family::Working),
    ("ACES (AP0) Linear", "ACES2065-1", Family::Working),
];

/// Our name of Fujifilm's F-Log2 with F-Gamut, in the style of the studio config.
pub const FLOG2: &str = "F-Log2 F-Gamut";

/// Every space of the catalogue: those of the media in the order of the model's gamuts and
/// gammas, then the others.
#[must_use]
pub fn spaces() -> &'static [Space] {
    static SPACES: OnceLock<Vec<Space>> = OnceLock::new();
    SPACES.get_or_init(|| {
        let mut spaces = Vec::new();
        for gamut in GAMUTS {
            for gamma in GAMMAS {
                let input = InputSpace { gamut, gamma };
                let (ocio, ours) = match studio_space(gamut, gamma) {
                    Some(name) => (name.to_owned(), false),
                    None => (our_space(gamut, gamma), true),
                };
                spaces.push(Space {
                    name: input.name(),
                    ocio,
                    family: family_of(gamma),
                    referred: referred_of(gamma),
                    input: Some(input),
                    ours,
                });
            }
        }
        for (name, ocio, family) in OTHERS {
            spaces.push(Space {
                name: name.to_owned(),
                ocio: ocio.to_owned(),
                family,
                referred: Referred::Scene,
                input: None,
                ours: ocio == FLOG2,
            });
        }
        spaces
    })
}

/// The space of the catalogue of Resolve's name `name`.
#[must_use]
pub fn by_name(name: &str) -> Option<&'static Space> {
    spaces().iter().find(|space| space.name == name)
}

/// The space of the catalogue that media in `input` are in; none for camera raw, which is
/// debayered by its maker's colour science and has no space of OCIO's.
#[must_use]
pub fn for_input(input: InputSpace) -> Option<&'static Space> {
    spaces().iter().find(|space| space.input == Some(input))
}

// --- the fragment ---------------------------------------------------------------------------------

type Matrix = [[f64; 3]; 3];

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn apply(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn invert(m: &Matrix) -> Matrix {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let c =
        |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    [
        [
            c(1, 1, 2, 2) / det,
            -c(0, 1, 2, 2) / det,
            c(0, 1, 1, 2) / det,
        ],
        [
            -c(1, 0, 2, 2) / det,
            c(0, 0, 2, 2) / det,
            -c(0, 0, 1, 2) / det,
        ],
        [
            c(1, 0, 2, 1) / det,
            -c(0, 0, 2, 1) / det,
            c(0, 0, 1, 1) / det,
        ],
    ]
}

fn xyz([x, y]: [f64; 2]) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// The cone response of Bradford's transform of chromatic adaptation.
const BRADFORD: Matrix = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

/// RGB of `gamut` to CIE XYZ, its white adapted to D65 by Bradford where it is not D65.
#[allow(clippy::many_single_char_names)]
fn to_xyz_d65(gamut: Gamut) -> Option<Matrix> {
    let [r, g, b, w] = chromaticities(gamut)?;
    let (r, g, b) = (xyz(r), xyz(g), xyz(b));
    let primaries = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let white = xyz(w);
    let s = apply(&invert(&primaries), white);
    let npm = [0, 1, 2].map(|i| [0, 1, 2].map(|j| primaries[i][j] * s[j]));
    let d65 = xyz([0.3127, 0.3290]);
    if (w[0] - 0.3127).abs() < 1e-9 && (w[1] - 0.3290).abs() < 1e-9 {
        return Some(npm);
    }
    let (from, to) = (apply(&BRADFORD, white), apply(&BRADFORD, d65));
    let scale = [
        [to[0] / from[0], 0.0, 0.0],
        [0.0, to[1] / from[1], 0.0],
        [0.0, 0.0, to[2] / from[2]],
    ];
    let cat = multiply(&invert(&BRADFORD), &multiply(&scale, &BRADFORD));
    Some(multiply(&cat, &npm))
}

fn matrix_yaml(m: &Matrix) -> String {
    let mut out = String::from("!<MatrixTransform> {matrix: [");
    for row in m {
        for value in row {
            let _ = write!(out, "{value:?}, ");
        }
        out.push_str("0, ");
    }
    out.push_str("0, 0, 0, 1]}");
    out
}

const AP0_TO_XYZ: &str = "!<BuiltinTransform> {style: UTILITY - ACES-AP0_to_CIE-XYZ-D65_BFD}";

/// The definition of our colour space of `gamut` and `gamma`, in YAML.
fn space_yaml(gamut: Gamut, gamma: Gamma) -> Option<String> {
    let to_xyz = to_xyz_d65(gamut)?;
    let from_xyz = invert(&to_xyz);
    let name = our_space(gamut, gamma);
    let label = gamut_label(gamut);
    let curve = match gamma {
        Gamma::Rec709 => {
            Some("!<ExponentTransform> {value: 2.4, style: pass_thru, direction: inverse}")
        }
        Gamma::Gamma22 => {
            Some("!<ExponentTransform> {value: 2.2, style: pass_thru, direction: inverse}")
        }
        Gamma::Gamma26 => {
            Some("!<ExponentTransform> {value: 2.6, style: pass_thru, direction: inverse}")
        }
        Gamma::Gamma28 => {
            Some("!<ExponentTransform> {value: 2.8, style: pass_thru, direction: inverse}")
        }
        Gamma::Srgb => {
            Some("!<ExponentWithLinearTransform> {gamma: 2.4, offset: 0.055, direction: inverse}")
        }
        Gamma::Linear | Gamma::St2084 | Gamma::Hlg => None,
    };
    let mut out = String::new();
    match gamma {
        Gamma::St2084 => {
            let _ = write!(
                out,
                "  - !<ColorSpace>\n    name: {name}\n    family: Columbina/Display\n    \
                 encoding: hdr-video\n    description: \"SMPTE ST 2084 (PQ) of {label}, absolute, \
                 100 nits being 1.\"\n    from_display_reference: !<GroupTransform>\n      children:\n        \
                 - {}\n        - !<BuiltinTransform> {{style: CURVE - LINEAR_to_ST-2084}}\n",
                matrix_yaml(&from_xyz)
            );
        }
        Gamma::Hlg => {
            let from_2020 = multiply(&from_xyz, &to_xyz_d65(Gamut::Rec2020)?);
            let _ = write!(
                out,
                "  - !<ColorSpace>\n    name: {name}\n    family: Columbina/Display\n    \
                 encoding: hdr-video\n    description: \"ITU-R BT.2100 HLG of {label} for a display \
                 of 1000 nits; the OOTF is done in BT.2020 and the gamut changed in scene light.\"\n    \
                 from_display_reference: !<GroupTransform>\n      children:\n        \
                 - !<BuiltinTransform> {{style: DISPLAY - CIE-XYZ-D65_to_REC.2100-HLG-1000nit}}\n        \
                 - !<BuiltinTransform> {{style: CURVE - HLG-OETF-INVERSE}}\n        \
                 - {}\n        - !<BuiltinTransform> {{style: CURVE - HLG-OETF}}\n",
                matrix_yaml(&from_2020)
            );
        }
        _ => {
            let encoding = if curve.is_some() {
                "sdr-video"
            } else {
                "scene-linear"
            };
            let _ = write!(
                out,
                "  - !<ColorSpace>\n    name: {name}\n    family: Columbina/Utility\n    \
                 encoding: {encoding}\n    description: \"{label}, {}.\"\n    \
                 from_scene_reference: !<GroupTransform>\n      children:\n        \
                 - {AP0_TO_XYZ}\n        - {}\n",
                match gamma {
                    Gamma::Linear => "linear".to_owned(),
                    Gamma::Srgb => "encoded with the sRGB curve (IEC 61966-2-1)".to_owned(),
                    other => format!("encoded with {} (a pure power)", other.name()),
                },
                matrix_yaml(&from_xyz)
            );
            if let Some(curve) = curve {
                let _ = writeln!(out, "        - {curve}");
            }
        }
    }
    Some(out)
}

/// F-Log2 / F-Gamut: the curve of the F-Log2 Data Sheet Ver.1.0 (Fujifilm), whose constants are
/// `a` to `f` and `cut1` below; F-Gamut has the primaries and white of BT.2020.
pub mod flog2 {
    pub const A: f64 = 5.555_556;
    pub const B: f64 = 0.064_829;
    pub const C: f64 = 0.245_281;
    pub const D: f64 = 0.384_316;
    pub const E: f64 = 8.799_461;
    pub const F: f64 = 0.092_864;
    pub const CUT1: f64 = 0.000_889;

    /// The code value (0 to 1) of linear reflection `x`, as the data sheet writes it.
    #[must_use]
    pub fn encode(x: f64) -> f64 {
        if x >= CUT1 {
            C * (A * x + B).log10() + D
        } else {
            E * x + F
        }
    }
}

fn flog2_yaml() -> String {
    let to_xyz = to_xyz_d65(Gamut::Rec2020).unwrap_or_default();
    format!(
        "  - !<ColorSpace>\n    name: {FLOG2}\n    family: Columbina/Input/Fujifilm\n    \
         encoding: log\n    description: \"Fujifilm F-Log2 with F-Gamut, after the F-Log2 Data \
         Sheet Ver.1.0 (Fujifilm); F-Gamut has the primaries and white of ITU-R BT.2020.\"\n    \
         to_scene_reference: !<GroupTransform>\n      children:\n        \
         - !<LogCameraTransform> {{base: 10, log_side_slope: {c:?}, log_side_offset: {d:?}, \
         lin_side_slope: {a:?}, lin_side_offset: {b:?}, lin_side_break: {cut:?}, linear_slope: {e:?}, \
         direction: inverse}}\n        - {}\n        - !<BuiltinTransform> {{style: UTILITY - \
         ACES-AP0_to_CIE-XYZ-D65_BFD, direction: inverse}}\n",
        matrix_yaml(&to_xyz),
        a = flog2::A,
        b = flog2::B,
        c = flog2::C,
        d = flog2::D,
        e = flog2::E,
        cut = flog2::CUT1,
    )
}

/// Our config fragment: the colour spaces of the catalogue that the studio config lacks, as a
/// config of OCIO's (YAML) whose reference spaces are the studio config's (ACES2065-1 for the
/// scene, CIE XYZ D65 for displays), made only of OCIO's built-in and formula transforms.
/// [`crate::Config::columbina`] adds them to the studio config. The same text is kept in the
/// crate as `config/columbina.ocio`, for a reader and for OCIO's tools.
#[must_use]
pub fn fragment() -> String {
    fragment_of(spaces().iter().filter(|s| s.ours).filter_map(|s| s.input))
}

/// A fragment with our definition of each of `inputs` (and F-Log2): [`fragment`] has those that
/// the studio config lacks; the tests make the others too, and hold them to the studio config's.
#[must_use]
pub fn fragment_of(inputs: impl IntoIterator<Item = InputSpace>) -> String {
    let mut scene = String::new();
    let mut display = String::new();
    for input in inputs {
        let Some(yaml) = space_yaml(input.gamut, input.gamma) else {
            continue;
        };
        if referred_of(input.gamma) == Referred::Display {
            display.push_str(&yaml);
            display.push('\n');
        } else {
            scene.push_str(&yaml);
            scene.push('\n');
        }
    }
    scene.push_str(&flog2_yaml());
    format!(
        "ocio_profile_version: 2.4\n\
         name: columbina\n\
         description: |\n  \
         The colour spaces of the engine (columbina-ocio) that the built-in ACES Studio config \
         lacks, added to it. Written by catalogue::fragment; do not edit by hand.\n\
         roles:\n  {{}}\n\
         file_rules:\n  - !<Rule> {{name: Default, colorspace: {FLOG2}}}\n\
         colorspaces:\n{scene}\n\
         display_colorspaces:\n{display}"
    )
}

/// Our name of a space of the media, whether or not the studio config has one (for the tests).
#[must_use]
pub fn our_name(input: InputSpace) -> String {
    our_space(input.gamut, input.gamma)
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    #[test]
    fn every_space_of_the_media_but_camera_raw_is_there_once() {
        for gamut in GAMUTS {
            for gamma in GAMMAS {
                let input = InputSpace { gamut, gamma };
                let found: Vec<_> = spaces().iter().filter(|s| s.input == Some(input)).collect();
                assert_eq!(found.len(), 1, "{}", input.name());
                assert_eq!(found[0].name, input.name());
            }
        }
        assert!(for_input(InputSpace::CAMERA_RAW).is_none());
        let mut names: Vec<_> = spaces().iter().map(|s| &s.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), spaces().len());
    }

    #[test]
    fn the_matrix_of_bt709_is_the_one_of_the_standard() {
        let m = to_xyz_d65(Gamut::Rec709).unwrap();
        // ITU-R BT.709 / IEC 61966-2-1, to four places.
        let expected = [
            [0.4124, 0.3576, 0.1805],
            [0.2126, 0.7152, 0.0722],
            [0.0193, 0.1192, 0.9505],
        ];
        for i in 0..3 {
            for j in 0..3 {
                assert!((m[i][j] - expected[i][j]).abs() < 1e-4, "{i}{j}");
            }
        }
        let identity = multiply(&m, &invert(&m));
        for i in 0..3 {
            for j in 0..3 {
                let e = if i == j { 1.0 } else { 0.0 };
                assert!((identity[i][j] - e).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn dci_white_is_adapted_to_d65() {
        let m = to_xyz_d65(Gamut::DciP3).unwrap();
        let white = apply(&m, [1.0, 1.0, 1.0]);
        let d65 = xyz([0.3127, 0.3290]);
        for c in 0..3 {
            assert!((white[c] - d65[c]).abs() < 1e-9);
        }
    }

    #[test]
    fn the_flog2_curve_meets_itself_at_its_break() {
        let below = flog2::E * flog2::CUT1 + flog2::F;
        let above = flog2::C * (flog2::A * flog2::CUT1 + flog2::B).log10() + flog2::D;
        assert!((below - above).abs() < 2e-6);
    }
}
