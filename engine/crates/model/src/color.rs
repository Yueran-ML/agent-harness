//! What a file says of the colours of its picture, and the colour space that its picture is
//! taken to be in.
//!
//! The numbers of the primaries, the transfer and the matrix are those of ITU-T H.273, which
//! the NCLC of `QuickTime` (SMPTE RDD 36), the VUI of H.264 and HEVC, and libav
//! (`AVColorPrimaries`, `AVColorTransferCharacteristic`, `AVColorSpace`) all use: 1 is BT.709 in
//! each, 2 is unspecified, 9 is BT.2020 (primaries and matrix), 16 is PQ and 18 HLG.
//!
//! [`input_space`] decides the colour space of the picture of media as `DaVinci Resolve`'s colour
//! management does (Reference Manual 20, p228–230): camera raw by its maker, else a space set
//! by hand, else the metadata of the file, else the project's default.

use serde::{Deserialize, Serialize};

use crate::project::VideoFacts;

/// What a file says of the colours of its picture, as its stream and its first frame say.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorFacts {
    /// The colour primaries, numbered as H.273 numbers them: 1 BT.709, 5 BT.470BG (625 lines),
    /// 6 SMPTE 170M (525 lines), 9 BT.2020, 11 DCI-P3, 12 P3-D65; 2 unspecified.
    pub primaries: u8,
    /// The transfer characteristics: 1 BT.709, 4 gamma 2.2, 5 gamma 2.8, 6 SMPTE 170M, 8 linear,
    /// 13 sRGB, 14 and 15 BT.2020, 16 SMPTE ST 2084 (PQ), 17 SMPTE ST 428-1, 18 ARIB STD-B67
    /// (HLG); 2 unspecified.
    pub transfer: u8,
    /// The matrix from R'G'B' to Y'CbCr: 0 none (the picture is stored as R'G'B'), 1 BT.709,
    /// 5 BT.470BG, 6 SMPTE 170M (both BT.601), 9 BT.2020 non-constant luminance, 10 BT.2020
    /// constant luminance; 2 unspecified.
    pub matrix: u8,
    /// Whether its codes span the limited (video) range or the full one.
    pub range: ColorRange,
    /// Where its chroma samples lie, as libav numbers it (`AVChromaLocation`, the
    /// `chroma_sample_loc_type` of H.273 plus 1): 0 unspecified, 1 left, 2 center, 3 top left,
    /// 4 top, 5 bottom left, 6 bottom.
    pub chroma_location: u8,
    /// The display that the picture was mastered on (SMPTE ST 2086), if the file says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mastering: Option<MasteringDisplay>,
    /// The light levels of its content (CTA-861.3), if the file says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light_level: Option<LightLevel>,
}

impl Default for ColorFacts {
    /// A file that says nothing of its colours.
    fn default() -> Self {
        Self {
            primaries: UNSPECIFIED,
            transfer: UNSPECIFIED,
            matrix: UNSPECIFIED,
            range: ColorRange::Unspecified,
            chroma_location: 0,
            mastering: None,
            light_level: None,
        }
    }
}

/// The number of H.273 for a primaries, transfer or matrix that is not said.
pub const UNSPECIFIED: u8 = 2;

/// The range of the codes of a picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorRange {
    /// The file does not say.
    #[default]
    Unspecified,
    /// Video levels: in 8 bits luma from 16 to 235 and chroma from 16 to 240.
    Limited,
    /// Full levels: every code, from 0 to 255 in 8 bits.
    Full,
}

/// The display that a picture was mastered on, as SMPTE ST 2086 and the SEI of HEVC write it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MasteringDisplay {
    /// Its primaries and white point, where the file gives them.
    pub primaries: Option<DisplayPrimaries>,
    /// Its least and most luminance, where the file gives them.
    pub luminance: Option<Luminance>,
}

/// The chromaticities `[x, y]` of the primaries and the white point of a display, each in units
/// of 0.00002 (50000 is 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayPrimaries {
    pub red: [u32; 2],
    pub green: [u32; 2],
    pub blue: [u32; 2],
    pub white: [u32; 2],
}

/// The least and the most luminance of a display, in units of 0.0001 cd/m² (10000 is 1 nit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Luminance {
    pub min: u32,
    pub max: u32,
}

/// The light levels of the content of a picture, in cd/m² (nits).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightLevel {
    /// The brightest pixel of any frame (`MaxCLL`).
    pub max_cll: u32,
    /// The brightest mean of the pixels of a frame (`MaxFALL`).
    pub max_fall: u32,
}

/// The gamut of a colour space: its primaries and white point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Gamut {
    /// BT.709, as sRGB has it too.
    Rec709,
    /// BT.601 of 625 lines (BT.470BG, EBU).
    Rec601Pal,
    /// BT.601 of 525 lines (SMPTE 170M and 240M).
    Rec601Ntsc,
    /// BT.2020, as BT.2100 has it too.
    Rec2020,
    /// DCI-P3, of a white of about 6300 K.
    DciP3,
    /// P3 with the white of D65.
    P3D65,
    /// The primaries that a maker's colour science debayers camera raw to.
    CameraNative,
}

/// The gamma of a colour space: how its code values are made from light.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Gamma {
    /// BT.709 as displayed: BT.1886, a power of 2.4. What a file that says BT.709 (or BT.601,
    /// or BT.2020 of standard range) is shown with.
    Rec709,
    /// A power of 2.2.
    Gamma22,
    /// A power of 2.6, as SMPTE ST 428-1 has it.
    Gamma26,
    /// A power of 2.8.
    Gamma28,
    /// The piecewise curve of sRGB (IEC 61966-2-1).
    Srgb,
    /// Linear light.
    Linear,
    /// SMPTE ST 2084, the perceptual quantizer of HDR.
    St2084,
    /// ARIB STD-B67, hybrid log-gamma.
    Hlg,
}

/// The colour space that the picture of media is in: a gamut and a gamma.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSpace {
    pub gamut: Gamut,
    pub gamma: Gamma,
}

impl InputSpace {
    /// Rec.709 Gamma 2.4: the default input colour space of every preset of colour management
    /// but Custom (p230), and the space of a file that says nothing.
    pub const REC709: Self = Self {
        gamut: Gamut::Rec709,
        gamma: Gamma::Rec709,
    };
    /// sRGB.
    pub const SRGB: Self = Self {
        gamut: Gamut::Rec709,
        gamma: Gamma::Srgb,
    };
    /// Rec.2020 Gamma 2.4, of standard range.
    pub const REC2020: Self = Self {
        gamut: Gamut::Rec2020,
        gamma: Gamma::Rec709,
    };
    /// Rec.2100 ST2084: BT.2020 and PQ, as HDR10 has it.
    pub const REC2100_PQ: Self = Self {
        gamut: Gamut::Rec2020,
        gamma: Gamma::St2084,
    };
    /// Rec.2100 HLG: BT.2020 and hybrid log-gamma.
    pub const REC2100_HLG: Self = Self {
        gamut: Gamut::Rec2020,
        gamma: Gamma::Hlg,
    };
    /// Camera raw, debayered by the maker's colour science to its primaries and linear light.
    pub const CAMERA_RAW: Self = Self {
        gamut: Gamut::CameraNative,
        gamma: Gamma::Linear,
    };

    /// Its name, as `DaVinci Resolve` names the spaces of its menus where it has one: `Rec.709
    /// Gamma 2.4`, `sRGB`, `Rec.2100 ST2084`, `Rec.2100 HLG`; else the name of its gamut and of
    /// its gamma, as `P3-D65 ST2084`.
    #[must_use]
    pub fn name(self) -> String {
        match (self.gamut, self.gamma) {
            (Gamut::Rec709, Gamma::Srgb) => "sRGB".to_owned(),
            (Gamut::Rec2020, Gamma::St2084) => "Rec.2100 ST2084".to_owned(),
            (Gamut::Rec2020, Gamma::Hlg) => "Rec.2100 HLG".to_owned(),
            (gamut, gamma) => format!("{} {}", gamut.name(), gamma.name()),
        }
    }
}

impl Gamut {
    /// The gamut that the colour primaries of H.273 numbered `code` are, if it is one of these.
    #[must_use]
    pub fn of_primaries(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Rec709),
            5 => Some(Self::Rec601Pal),
            6 | 7 => Some(Self::Rec601Ntsc),
            9 => Some(Self::Rec2020),
            11 => Some(Self::DciP3),
            12 => Some(Self::P3D65),
            _ => None,
        }
    }

    /// Its name, as `DaVinci Resolve` names it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Rec709 => "Rec.709",
            Self::Rec601Pal => "Rec.601 (PAL)",
            Self::Rec601Ntsc => "Rec.601 (NTSC)",
            Self::Rec2020 => "Rec.2020",
            Self::DciP3 => "DCI-P3",
            Self::P3D65 => "P3-D65",
            Self::CameraNative => "Camera Native",
        }
    }
}

impl Gamma {
    /// The gamma that the transfer characteristics of H.273 numbered `code` are, if it is one of
    /// these. The OETFs of BT.709, BT.601 (SMPTE 170M), BT.2020 and the extended ones of BT.709
    /// (IEC 61966-2-4, BT.1361) are taken as the display's power of 2.4, as a picture coded with
    /// them is shown.
    #[must_use]
    pub fn of_transfer(code: u8) -> Option<Self> {
        match code {
            1 | 6 | 11 | 12 | 14 | 15 => Some(Self::Rec709),
            4 => Some(Self::Gamma22),
            5 => Some(Self::Gamma28),
            8 => Some(Self::Linear),
            13 => Some(Self::Srgb),
            16 => Some(Self::St2084),
            17 => Some(Self::Gamma26),
            18 => Some(Self::Hlg),
            _ => None,
        }
    }

    /// Its name, as `DaVinci Resolve` names it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Rec709 => "Gamma 2.4",
            Self::Gamma22 => "Gamma 2.2",
            Self::Gamma26 => "Gamma 2.6",
            Self::Gamma28 => "Gamma 2.8",
            Self::Srgb => "sRGB",
            Self::Linear => "Linear",
            Self::St2084 => "ST2084",
            Self::Hlg => "HLG",
        }
    }

    /// Whether it is a gamma of high dynamic range.
    #[must_use]
    pub fn is_hdr(self) -> bool {
        matches!(self, Self::St2084 | Self::Hlg)
    }
}

/// What decided the input colour space of media, in the order of p228: the first that applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Basis {
    /// The media is camera raw: its maker's colour science, which no hand overrides.
    CameraRaw,
    /// The space was set by hand, over the metadata of the file and the default.
    Manual,
    /// The colour metadata of the file.
    Metadata,
    /// The project's default, for a file that says nothing of its colours.
    Default,
}

/// The codecs (as libav names them) of camera raw, which a maker's colour science debayers.
pub const CAMERA_RAW_CODECS: [&str; 4] = ["prores_raw", "braw", "r3d", "arriraw"];

/// The input colour space of the picture of media, and what decided it, as colour management
/// decides it (p228–230):
///
/// 1. camera raw ([`CAMERA_RAW_CODECS`]) is in its maker's space, [`InputSpace::CAMERA_RAW`],
///    whatever else is said;
/// 2. a space set by hand (`manual`) overrides the metadata and the default;
/// 3. else the colour metadata of the file, where it says its primaries or its transfer: the
///    gamut of its primaries and the gamma of its transfer. Where it says one and not the
///    other, the other is the default's, but that a picture of PQ or HLG, or of the matrix of
///    BT.2020, whose primaries are not said is taken to be of BT.2020;
/// 4. else the project's `default`, which is [`InputSpace::REC709`] unless the project's preset
///    is Custom.
///
/// A picture whose facts were read before colours were (`color` is none) says nothing.
#[must_use]
pub fn input_space(
    video: &VideoFacts,
    manual: Option<InputSpace>,
    default: InputSpace,
) -> (InputSpace, Basis) {
    if CAMERA_RAW_CODECS.contains(&video.codec.as_str()) {
        return (InputSpace::CAMERA_RAW, Basis::CameraRaw);
    }
    if let Some(manual) = manual {
        return (manual, Basis::Manual);
    }
    let Some(color) = &video.color else {
        return (default, Basis::Default);
    };
    match from_metadata(color, default) {
        Some(space) => (space, Basis::Metadata),
        None => (default, Basis::Default),
    }
}

/// The space that colour metadata says, with what it does not say taken from `default`; none
/// where it says neither its primaries nor its transfer.
fn from_metadata(color: &ColorFacts, default: InputSpace) -> Option<InputSpace> {
    let gamma = Gamma::of_transfer(color.transfer);
    let gamut = Gamut::of_primaries(color.primaries).or_else(|| {
        (gamma.is_some_and(Gamma::is_hdr) || matches!(color.matrix, 9 | 10))
            .then_some(Gamut::Rec2020)
    });
    if Gamut::of_primaries(color.primaries).is_none() && gamma.is_none() {
        return None;
    }
    Some(InputSpace {
        gamut: gamut.unwrap_or(default.gamut),
        gamma: gamma.unwrap_or(default.gamma),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Rate;

    fn video(codec: &str, color: Option<ColorFacts>) -> VideoFacts {
        VideoFacts {
            still: false,
            width: 1920,
            height: 1080,
            rate: Rate { num: 25, den: 1 },
            codec: codec.into(),
            bits: 10,
            rotation: 0,
            color,
        }
    }

    fn nclc(primaries: u8, transfer: u8, matrix: u8) -> ColorFacts {
        ColorFacts {
            primaries,
            transfer,
            matrix,
            ..ColorFacts::default()
        }
    }

    fn detect(primaries: u8, transfer: u8, matrix: u8) -> (InputSpace, Basis) {
        input_space(
            &video("h264", Some(nclc(primaries, transfer, matrix))),
            None,
            InputSpace::REC709,
        )
    }

    #[test]
    fn the_nclc_triples_of_common_files_name_their_spaces() {
        let metadata = |space| (space, Basis::Metadata);
        assert_eq!(detect(1, 1, 1), metadata(InputSpace::REC709));
        assert_eq!(detect(1, 13, 1), metadata(InputSpace::SRGB));
        assert_eq!(detect(9, 16, 9), metadata(InputSpace::REC2100_PQ));
        assert_eq!(detect(9, 18, 9), metadata(InputSpace::REC2100_HLG));
        assert_eq!(detect(9, 14, 9), metadata(InputSpace::REC2020));
        assert_eq!(detect(9, 15, 9), metadata(InputSpace::REC2020));
        let space = |gamut, gamma| metadata(InputSpace { gamut, gamma });
        assert_eq!(detect(6, 6, 6), space(Gamut::Rec601Ntsc, Gamma::Rec709));
        assert_eq!(detect(5, 6, 5), space(Gamut::Rec601Pal, Gamma::Rec709));
        assert_eq!(detect(7, 1, 7), space(Gamut::Rec601Ntsc, Gamma::Rec709));
        assert_eq!(detect(12, 16, 0), space(Gamut::P3D65, Gamma::St2084));
        assert_eq!(detect(11, 17, 0), space(Gamut::DciP3, Gamma::Gamma26));
        assert_eq!(detect(1, 8, 1), space(Gamut::Rec709, Gamma::Linear));
        assert_eq!(detect(1, 4, 1), space(Gamut::Rec709, Gamma::Gamma22));
        assert_eq!(detect(1, 5, 1), space(Gamut::Rec709, Gamma::Gamma28));
    }

    #[test]
    fn every_code_of_the_nclc_table_of_the_manual_maps_as_h273_numbers_it() {
        // p229–230: the primaries 0–12 and the transfers 0–18 of the table.
        let primaries: Vec<Option<Gamut>> = (0..=12).map(Gamut::of_primaries).collect();
        assert_eq!(
            primaries,
            [
                None,                    // 0 reserved
                Some(Gamut::Rec709),     // 1 ITU-R BT.709
                None,                    // 2 unspecified
                None,                    // 3 reserved
                None,                    // 4 ITU-R BT.470M
                Some(Gamut::Rec601Pal),  // 5 ITU-R BT.470BG
                Some(Gamut::Rec601Ntsc), // 6 SMPTE 170M
                Some(Gamut::Rec601Ntsc), // 7 SMPTE 240M
                None,                    // 8 film
                Some(Gamut::Rec2020),    // 9 ITU-R BT.2020
                None,                    // 10 SMPTE ST 428-1
                Some(Gamut::DciP3),      // 11 DCI P3
                Some(Gamut::P3D65),      // 12 P3 D65
            ]
        );
        let transfers: Vec<Option<Gamma>> = (0..=18).map(Gamma::of_transfer).collect();
        assert_eq!(
            transfers,
            [
                None,                 // 0 reserved
                Some(Gamma::Rec709),  // 1 ITU-R BT.709
                None,                 // 2 unspecified
                None,                 // 3 reserved
                Some(Gamma::Gamma22), // 4 gamma 2.2
                Some(Gamma::Gamma28), // 5 gamma 2.8
                Some(Gamma::Rec709),  // 6 SMPTE 170M
                None,                 // 7 SMPTE 240M
                Some(Gamma::Linear),  // 8 linear
                None,                 // 9 log
                None,                 // 10 log sqrt
                Some(Gamma::Rec709),  // 11 IEC 61966-2-4
                Some(Gamma::Rec709),  // 12 ITU-R BT.1361 extended colour gamut
                Some(Gamma::Srgb),    // 13 IEC 61966-2-1
                Some(Gamma::Rec709),  // 14 ITU-R BT.2020 10 bit
                Some(Gamma::Rec709),  // 15 ITU-R BT.2020 12 bit
                Some(Gamma::St2084),  // 16 SMPTE ST 2084 (PQ)
                Some(Gamma::Gamma26), // 17 SMPTE ST 428-1
                Some(Gamma::Hlg),     // 18 ARIB STD-B67 (HLG)
            ]
        );
    }

    #[test]
    fn a_file_that_says_nothing_is_in_the_default_space() {
        assert_eq!(detect(2, 2, 2), (InputSpace::REC709, Basis::Default));
        assert_eq!(detect(0, 3, 1), (InputSpace::REC709, Basis::Default));
        assert_eq!(
            input_space(&video("h264", None), None, InputSpace::REC709),
            (InputSpace::REC709, Basis::Default)
        );
        // A project of the preset Custom, whose default is another space.
        assert_eq!(
            input_space(
                &video("h264", Some(nclc(2, 2, 2))),
                None,
                InputSpace::REC2020
            ),
            (InputSpace::REC2020, Basis::Default)
        );
    }

    #[test]
    fn what_the_metadata_leaves_unsaid_is_the_defaults() {
        // DaVinci Resolve writes Rec.709 Gamma 2.4 as 1-2-1: no transfer of H.273 is a power
        // of 2.4.
        assert_eq!(detect(1, 2, 1), (InputSpace::REC709, Basis::Metadata));
        let custom = InputSpace {
            gamut: Gamut::P3D65,
            gamma: Gamma::Gamma26,
        };
        let detected = |primaries, transfer, matrix| {
            input_space(
                &video("hevc", Some(nclc(primaries, transfer, matrix))),
                None,
                custom,
            )
        };
        assert_eq!(
            detected(1, 2, 1).0,
            InputSpace {
                gamut: Gamut::Rec709,
                gamma: Gamma::Gamma26
            }
        );
        assert_eq!(
            detected(2, 1, 1).0,
            InputSpace {
                gamut: Gamut::P3D65,
                gamma: Gamma::Rec709
            }
        );
        // PQ, HLG or the matrix of BT.2020, without primaries, are of BT.2020.
        assert_eq!(detected(2, 16, 2).0, InputSpace::REC2100_PQ);
        assert_eq!(detected(2, 18, 2).0, InputSpace::REC2100_HLG);
        assert_eq!(
            detected(2, 1, 9).0,
            InputSpace {
                gamut: Gamut::Rec2020,
                gamma: Gamma::Rec709
            }
        );
        // The matrix alone says no space.
        assert_eq!(detected(2, 2, 9), (custom, Basis::Default));
    }

    #[test]
    fn a_space_set_by_hand_overrides_the_metadata_but_not_camera_raw() {
        let by_hand = Some(InputSpace::REC2100_HLG);
        assert_eq!(
            input_space(
                &video("hevc", Some(nclc(9, 16, 9))),
                by_hand,
                InputSpace::REC709
            ),
            (InputSpace::REC2100_HLG, Basis::Manual)
        );
        assert_eq!(
            input_space(&video("h264", None), by_hand, InputSpace::REC709),
            (InputSpace::REC2100_HLG, Basis::Manual)
        );
        assert_eq!(
            input_space(
                &video("prores_raw", Some(nclc(1, 1, 1))),
                by_hand,
                InputSpace::REC709
            ),
            (InputSpace::CAMERA_RAW, Basis::CameraRaw)
        );
    }

    #[test]
    fn spaces_are_named_as_the_menus_of_resolve_name_them() {
        assert_eq!(InputSpace::REC709.name(), "Rec.709 Gamma 2.4");
        assert_eq!(InputSpace::SRGB.name(), "sRGB");
        assert_eq!(InputSpace::REC2100_PQ.name(), "Rec.2100 ST2084");
        assert_eq!(InputSpace::REC2100_HLG.name(), "Rec.2100 HLG");
        assert_eq!(InputSpace::REC2020.name(), "Rec.2020 Gamma 2.4");
        assert_eq!(
            InputSpace {
                gamut: Gamut::P3D65,
                gamma: Gamma::St2084
            }
            .name(),
            "P3-D65 ST2084"
        );
    }

    #[test]
    fn colour_facts_are_written_and_read_back_and_left_out_where_unknown() {
        let color = ColorFacts {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            range: ColorRange::Limited,
            chroma_location: 3,
            mastering: Some(MasteringDisplay {
                primaries: Some(DisplayPrimaries {
                    red: [34000, 16000],
                    green: [13250, 34500],
                    blue: [7500, 3000],
                    white: [15635, 16450],
                }),
                luminance: Some(Luminance {
                    min: 50,
                    max: 10_000_000,
                }),
            }),
            light_level: Some(LightLevel {
                max_cll: 1000,
                max_fall: 400,
            }),
        };
        let facts = video("hevc", Some(color));
        let json = serde_json::to_value(&facts).unwrap();
        assert_eq!(json["color"]["range"], "limited");
        assert_eq!(json["color"]["mastering"]["luminance"]["max"], 10_000_000);
        assert_eq!(serde_json::from_value::<VideoFacts>(json).unwrap(), facts);
        let unknown = serde_json::to_value(video("h264", None)).unwrap();
        assert!(unknown.get("color").is_none(), "{unknown}");
        assert_eq!(
            serde_json::from_value::<VideoFacts>(unknown).unwrap(),
            video("h264", None)
        );
    }
}
