//! The colour management of a project on the card: each layer's input transform, the output
//! transform, the space that grades work in, and what an export says of its colour, as the
//! project's settings say (`columbina_model::color_management`, RM20 pp. 219–252).
//!
//! # What each science does
//!
//! - **`DaVinci YRGB`** (the default, p. 225): nothing. No transform, decoded R'G'B' held to 0..1
//!   as phase 1 held it, luma of Rec.709 for the grades, and an export tagged BT.709: a project
//!   that is not managed is drawn to the bit as it was.
//! - **`DaVinci YRGB Color Managed`** (RCM, pp. 226–245): each picture from its input space (set
//!   by hand, else as its file says, else the project's, p. 228) into the timeline's space,
//!   without clipping (p. 227), through the Input DRT where one maps it (from the most of its
//!   space to the timeline's working luminance, p. 238); titles and colours from Rec.709 Gamma
//!   2.4, their white at the Graphics White Level (p. 243). Grades are done in the timeline
//!   space, whose primaries give their luma. Then from the timeline space into the output
//!   space: through the Output DRT (from what the timeline holds to the most of the output), the
//!   203-nit reference of SDR in HDR (pp. 238–239), the limit of the output gamut (p. 239) and,
//!   where it is off, the undoing of the adaptation of white (p. 242), then clipped (p. 227). A
//!   preset other than Custom converts between SDR and HDR by the rule of pp. 232–233 where its
//!   output is of the other range. Bypass, of the input or of the output, is no transform.
//! - **`DaVinci ACEScct` and `ACEScc`** (pp. 246–252): each picture by its IDT into the timeline
//!   (`ACEScct` or `ACEScc`), through the reference gamut compression of ACES 1.3 where it is asked
//!   for (p. 251); titles as Rec.709; grades in AP1; and out through the ODT, a view of a display
//!   of the built-in config of the version, as OCIO has it (or a space, or ACES2065-1 for No
//!   Output Device Transform), clipped.
//!
//! # The project's LUTs
//!
//! The Lookup Tables of the Color Management panel (pp. 151–152) apply in every science, and
//! in `DaVinci YRGB` too, each level its 1D LUT before its 3D LUT (p. 152), read between their
//! points as the project's 3D Lookup Table Interpolation says (pp. 152–153), at the places of
//! the overall order of p. 3330:
//!
//! - the **Input LUT** on each picture of media after its input transform (RCM's into the
//!   timeline, or the IDT of ACES) and before its grades, "before every other image processing
//!   operation" of the Color page (p. 152); not on titles and colours, which are no source;
//! - the **LUTs of a source clip** (`color_override.lut`, pp. 3409–3410), 1D first, on the
//!   pictures of its media alone, after the Input LUT and before its grades;
//! - the **Output LUT** on the frame after the timeline grade and the subtitles, before the
//!   output transform (RCM's, or the ODT of ACES): "after the operations applied in the Color
//!   page, but before" the display LUT (p. 152);
//! - the **Color Viewer LUT** after all of that, only on what the view of the timeline monitor
//!   shows (p. 152; p. 672: the viewer of the Color page). An export never has it, and nor do
//!   the scopes, which measure "the actual video data levels" where no Scopes LUT is chosen
//!   (p. 152), nor `view.sample`, nor a still grabbed.
//!
//! A LUT that cannot be read is left out of its level and told as one that a node names is
//! ([`Colour::missing`]).
//!
//! The transforms between spaces are OCIO's ([`columbina_compose::ocio`]); the DRTs, gains,
//! limits and matrices ours ([`columbina_compose::tone`]), on linear Rec.2020 where 1.0 is 100
//! nits.
//!
//! # Made once
//!
//! [`Colour::make`] makes every operation of a project: OCIO's processors on the processor and
//! their shaders on the card, once each for what they do ([`CardOps`]), and its LUTs read and put
//! on the card. What it was made of is its [`Key`]: the settings, and the input space and levels
//! of each media; the colour of a project is made again only when that changes, or, where a LUT
//! of it could not be read, at the next change of the project (`crate::grading`).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use columbina_compose::grade::{CardLuts, Luts as _};
use columbina_compose::ocio::{self, catalogue, CardOps, Direction, Processor};
use columbina_compose::scopes::{Signal, REC2020_LUMA, REC601_LUMA, REC709_LUMA};
use columbina_compose::tone::{self, Drt, Kind, Matrix};
use columbina_compose::{ColorOp, ColorRange, Gpu, Interpolation, Scene, Source, Stage, YuvMatrix};
use columbina_model::color_management::{
    self as cm, Aces, ColorSettings, Levels, LutInterpolation, LutPair, MediaInput, OdtOutput,
    Pipeline, Preset, Rcm, ToneMapping,
};
use columbina_model::{Gamma, Gamut, InputSpace, Media, Project};
use columbina_view::{Content, HdrTransfer};

use crate::grading::{LutPlaces, MissingLut};

/// The space that the DRTs, gains and limits work in: linear light of Rec.2020, 1.0 at 100 nits.
const LINEAR: &str = "Linear Rec.2020";

/// The space that titles and colours are in: Rec.709 Gamma 2.4, as Resolve draws its graphics.
const GRAPHICS: &str = cm::REC709;

/// The nits of the white of SDR.
const SDR_WHITE: f64 = 100.0;

/// The white of SDR in HDR by BT.2408, where "Use 203 nits reference" is on (pp. 238–239).
const REFERENCE_WHITE: f64 = 203.0;

/// The most nits of what a space of HDR or of a scene holds, where its file says no more: PQ's
/// 10000; HLG's nominal 1000 (BT.2100).
const PQ_PEAK: f64 = 10_000.0;
const HLG_PEAK: f64 = 1_000.0;

/// The most that the output of HDR is mastered for where the timeline is of SDR and is mapped up
/// to it (by the inverse DRT or the rule of p. 232: "101 to 1000 nits").
const SDR_TO_HDR_PEAK: f64 = 1_000.0;

/// The least luminance of the display that an export of HDR says it was mastered on: ours.
const MASTERING_MIN_NITS: f64 = 0.0001;

/// What the colour of a project is made of: its settings, and the input space, levels and most
/// nits of each media with a picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    settings: ColorSettings,
    media: BTreeMap<String, MediaKey>,
}

#[derive(Clone, Debug, PartialEq)]
struct MediaKey {
    input: Option<MediaInput>,
    levels: Levels,
    peak: Option<u32>,
    /// The LUTs of the source clip (pp. 3409–3410).
    lut: LutPair,
    /// Whether its Inverse DRT for SDR to HDR Conversion is on (p. 233).
    inverse_drt: bool,
}

impl Key {
    /// What colour management makes of `project`.
    #[must_use]
    pub fn of(project: &Project) -> Key {
        let settings = project.color.clone();
        let media = project
            .media
            .iter()
            .filter(|media| media.facts.video.is_some())
            .map(|media| {
                (
                    media.id.clone(),
                    MediaKey {
                        input: cm::media_input(&settings, media).map(|(input, _)| input),
                        levels: media.color_override.levels,
                        peak: mastered_peak(media),
                        lut: media.color_override.lut.clone(),
                        inverse_drt: media.color_override.inverse_drt,
                    },
                )
            })
            .collect();
        Key { settings, media }
    }

    /// The settings it was made of.
    #[must_use]
    pub fn settings(&self) -> &ColorSettings {
        &self.settings
    }

    /// Whether the project is managed at all.
    #[must_use]
    pub fn managed(&self) -> bool {
        self.settings.pipeline() != Pipeline::Unmanaged
    }
}

/// The most luminance, in nits, of the display that the file of `media` says it was mastered on.
fn mastered_peak(media: &Media) -> Option<u32> {
    let color = media.facts.video.as_ref()?.color.as_ref()?;
    color
        .mastering
        .and_then(|mastering| mastering.luminance)
        .map(|luminance| luminance.max / 10_000)
        .filter(|&nits| nits >= 100)
}

/// What the timeline's frames are put out as: their tags, the matrix of an export's Y'CbCr, the
/// signal of its scopes, and of HDR, what it was mastered for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    /// H.273's numbers of its primaries, transfer and matrix.
    pub primaries: u8,
    pub transfer: u8,
    pub matrix_code: u8,
    /// The matrix that an export's Y'CbCr is made by.
    pub matrix: YuvMatrix,
    /// What the scopes of the timeline monitor measure by, where a scope says no signal of its
    /// own: the luma coefficients and transfer of the output space.
    pub signal: Signal,
    /// Of PQ or HLG.
    pub hdr: Option<Hdr>,
}

/// What an output of HDR is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hdr {
    /// PQ, else HLG.
    pub pq: bool,
    /// The most nits that the picture holds: what it is mastered for.
    pub peak: f64,
    /// The chromaticities of the primaries (red, green, blue) and white of the display it is
    /// mastered on: of the gamut it is limited to, or of its own.
    pub primaries: [[f64; 2]; 3],
    pub white: [f64; 2],
    /// The least luminance of the display it is mastered on.
    pub min_nits: f64,
}

impl Default for Output {
    /// BT.709, as phase 1 tagged every export.
    fn default() -> Self {
        Output {
            primaries: 1,
            transfer: 1,
            matrix_code: 1,
            matrix: YuvMatrix::Bt709,
            signal: Signal::default(),
            hdr: None,
        }
    }
}

/// The colour of a media's pictures: its input transform, the range its codes are read in
/// where it is set by hand (Data Levels, pp. 221–222), and the LUTs of the source clip (pp.
/// 3409–3410).
#[derive(Clone, Debug, Default)]
struct MediaColour {
    input: Vec<ColorOp>,
    range: Option<ColorRange>,
    luts: Vec<ColorOp>,
}

/// The colour management of a project, made on one card.
pub struct Colour {
    key: Key,
    managed: bool,
    media: HashMap<String, MediaColour>,
    graphics: Vec<ColorOp>,
    output: Vec<ColorOp>,
    /// The luma coefficients of the timeline space, for the grades.
    luma: [f32; 3],
    out: Output,
    /// The project's LUTs.
    luts: ProjectLuts,
}

/// The LUTs of the Lookup Tables of a project (pp. 151–152), each level's 1D first, and those of
/// each source clip (pp. 3409–3410), by its media, on the card; and those that could not be read.
#[derive(Default)]
struct ProjectLuts {
    input: Vec<ColorOp>,
    output: Vec<ColorOp>,
    viewer: Vec<ColorOp>,
    media: HashMap<String, Vec<ColorOp>>,
    missing: Vec<MissingLut>,
}

impl ProjectLuts {
    /// The LUTs that the settings and the media of `key` name, read from `places` and put on
    /// `card`, read between their points as the project says. A name that cannot be read is
    /// left out, and told.
    fn read(card: &Arc<Gpu>, places: &LutPlaces, key: &Key) -> Self {
        let settings = &key.settings;
        let levels = [
            &settings.input_lut,
            &settings.output_lut,
            &settings.viewer_lut,
        ];
        let none = |pair: &LutPair| pair.one.is_none() && pair.three.is_none();
        if levels.iter().all(|pair| none(pair)) && key.media.values().all(|media| none(&media.lut))
        {
            return ProjectLuts::default();
        }
        let mut luts = CardLuts::new(
            Arc::clone(card),
            Arc::clone(&places.cache),
            places.roots.clone(),
        );
        luts.set_interpolation(interpolation(settings.lut_interpolation));
        let mut missing = BTreeMap::new();
        let mut level = |pair: &LutPair| -> Vec<ColorOp> {
            [&pair.one, &pair.three]
                .into_iter()
                .flatten()
                .filter_map(|name| {
                    let op = luts.lut(name);
                    if op.is_none() {
                        let why = luts
                            .error(name)
                            .unwrap_or_else(|| "it cannot be read".to_owned());
                        missing.insert(name.clone(), why);
                    }
                    op
                })
                .collect()
        };
        let [input, output, viewer] = levels.map(&mut level);
        let media = key
            .media
            .iter()
            .filter(|(_, media)| !none(&media.lut))
            .map(|(id, media)| (id.clone(), level(&media.lut)))
            .collect();
        ProjectLuts {
            input,
            output,
            viewer,
            media,
            missing: missing
                .into_iter()
                .map(|(name, message)| MissingLut { name, message })
                .collect(),
        }
    }
}

/// How the compositor reads a cube between its points, as the project's 3D Lookup Table
/// Interpolation says (pp. 152–153).
#[must_use]
pub fn interpolation(setting: LutInterpolation) -> Interpolation {
    match setting {
        LutInterpolation::Trilinear => Interpolation::Trilinear,
        LutInterpolation::Tetrahedral => Interpolation::Tetrahedral,
    }
}

impl std::fmt::Debug for Colour {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Colour")
            .field("managed", &self.managed)
            .field("luma", &self.luma)
            .field("output", &self.out)
            .finish_non_exhaustive()
    }
}

impl Colour {
    /// No colour management: `DaVinci YRGB` as phase 1 drew it, with no project yet.
    #[must_use]
    pub fn unmanaged() -> Colour {
        Colour {
            key: Key {
                settings: ColorSettings::default(),
                media: BTreeMap::new(),
            },
            managed: false,
            media: HashMap::new(),
            graphics: Vec::new(),
            output: Vec::new(),
            luma: REC709_LUMA,
            out: Output::default(),
            luts: ProjectLuts::default(),
        }
    }

    /// What it was made of.
    #[must_use]
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// The luma coefficients of the timeline space, which the grades take.
    #[must_use]
    pub fn luma(&self) -> [f32; 3] {
        self.luma
    }

    /// What the frames are put out as.
    #[must_use]
    pub fn output(&self) -> Output {
        self.out
    }

    /// The Color Viewer LUT (p. 152): what the view of the timeline monitor does over the frame,
    /// and nothing else does.
    #[must_use]
    pub fn viewer(&self) -> Stage {
        Stage {
            ops: self.luts.viewer.clone(),
        }
    }

    /// The LUTs of the project's Lookup Tables that cannot be read, by name.
    #[must_use]
    pub fn missing(&self) -> &[MissingLut] {
        &self.luts.missing
    }

    /// The colour management of the project of `key`, on the card of `ops`, its LUTs read from
    /// `places`.
    ///
    /// # Errors
    /// A transform cannot be made (OCIO's message): a space that the catalogue names and the
    /// config lacks, which the tests hold none to be.
    pub fn make(ops: &CardOps, places: &LutPlaces, key: Key) -> Result<Colour, String> {
        let luts = ProjectLuts::read(ops.gpu(), places, &key);
        let mut levels: HashMap<String, MediaColour> = key
            .media
            .iter()
            .map(|(id, media)| {
                let range = match media.levels {
                    Levels::Auto => None,
                    Levels::Video => Some(ColorRange::Limited),
                    Levels::Full => Some(ColorRange::Full),
                };
                (
                    id.clone(),
                    MediaColour {
                        input: Vec::new(),
                        range,
                        luts: luts.media.get(id).cloned().unwrap_or_default(),
                    },
                )
            })
            .collect();
        let settings = key.settings.clone();
        let made = match settings.pipeline() {
            Pipeline::Unmanaged => {
                let mut colour = Colour::unmanaged();
                colour.media = levels;
                colour.key = key;
                colour.luts = luts;
                return Ok(colour);
            }
            Pipeline::Rcm(rcm) => {
                let custom = !settings.automatic && settings.preset == Preset::Custom;
                Maker {
                    ops,
                    config: ocio::config().map_err(|e| e.to_string())?,
                }
                .rcm(&rcm, custom, &key, &mut levels)
            }
            Pipeline::Aces(aces) => Maker {
                ops,
                config: ocio::config().map_err(|e| e.to_string())?,
            }
            .aces(&aces, &key, &mut levels),
        };
        let (graphics, output, luma, out) = made.map_err(|error| error.to_string())?;
        Ok(Colour {
            key,
            managed: true,
            media: levels,
            graphics,
            output,
            luma,
            out,
            luts,
        })
    }

    /// Give each layer of `scene` its input transform and how its codes are read, each picture
    /// then the Input LUT, and the scene the Output LUT and then its output transform. `media`
    /// names the media of each layer of a picture (none of a layer that is no clip of media).
    pub fn apply<S: AsRef<str>>(&self, scene: &mut Scene, media: &[Option<S>]) {
        for (index, layer) in scene.layers.iter_mut().enumerate() {
            layer.color.unclamped = self.managed;
            match &layer.source {
                Source::Picture(..) | Source::Between { .. } => {
                    let own = media
                        .get(index)
                        .and_then(Option::as_ref)
                        .and_then(|id| self.media.get(id.as_ref()));
                    if let Some(own) = own {
                        if let Some(range) = own.range {
                            layer.color.coding.range = range;
                        }
                        layer.color.input.ops.clone_from(&own.input);
                    } else {
                        layer.color.input.ops.clear();
                    }
                    layer
                        .color
                        .input
                        .ops
                        .extend(self.luts.input.iter().cloned());
                    // The source clip's LUTs, after the project's Input LUT.
                    if let Some(own) = own {
                        layer.color.input.ops.extend(own.luts.iter().cloned());
                    }
                }
                _ => layer.color.input.ops.clone_from(&self.graphics),
            }
        }
        scene.output.ops.clone_from(&self.luts.output);
        scene.output.ops.extend(self.output.iter().cloned());
    }
}

/// The operations of graphics, of the output, the luma of the timeline, and the output.
type Made = (Vec<ColorOp>, Vec<ColorOp>, [f32; 3], Output);

/// What makes the operations of a project.
struct Maker<'a> {
    ops: &'a CardOps,
    config: ocio::Config,
}

impl Maker<'_> {
    /// The operation from `from` to `to` (spaces of OCIO), none where it does nothing.
    fn between(&self, from: &str, to: &str) -> Result<Option<ColorOp>, ocio::Error> {
        self.ops.op(&processor(&self.config, from, to)?)
    }

    fn push(ops: &mut Vec<ColorOp>, op: Option<ColorOp>) {
        ops.extend(op);
    }

    fn rcm(
        &self,
        rcm: &Rcm<'_>,
        custom: bool,
        key: &Key,
        media: &mut HashMap<String, MediaColour>,
    ) -> Result<Made, ocio::Error> {
        let timeline = space_of(rcm.timeline);
        let (map, clip) = rcm.luminance;
        let luma = luma_of(&self.config, &timeline.ocio)?;
        let rec2020 = REC2020_LUMA;
        // Each media, from its space into the timeline's.
        let mut made: HashMap<(String, bool), Vec<ColorOp>> = HashMap::new();
        for (id, own) in &key.media {
            let Some(MediaInput::Space(name)) = &own.input else {
                continue;
            };
            let space = space_of(name);
            let peak = source_peak(&space, own.peak);
            // Mapped up only where the project's inverse DRT and the media's are both on (p. 233:
            // "you can disable this behavior on a clip by clip basis").
            let inverse = rcm.inverse_drt && own.inverse_drt;
            let mapped = (rcm.input_drt != ToneMapping::None)
                .then(|| drt(rcm.input_drt, peak, map, rec2020, rcm.input_sat_rolloff))
                .flatten()
                .filter(|drt| peak > map || inverse && !drt.is_identity());
            let adapt = (!rcm.white_point_adaptation)
                .then(|| unadapt(white_of(name), D65))
                .flatten();
            let made_key = (name.clone(), inverse);
            let ops = if let Some(ops) = made.get(&made_key) {
                ops.clone()
            } else {
                let mut ops = Vec::new();
                if mapped.is_none() && adapt.is_none() {
                    Self::push(&mut ops, self.between(&space.ocio, &timeline.ocio)?);
                } else {
                    Self::push(&mut ops, self.between(&space.ocio, LINEAR)?);
                    ops.extend(adapt.map(|m| tone::matrix_op(&m)));
                    ops.extend(mapped.and_then(|drt| drt.op()));
                    Self::push(&mut ops, self.between(LINEAR, &timeline.ocio)?);
                }
                made.insert(made_key, ops.clone());
                ops
            };
            media.entry(id.clone()).or_default().input = ops;
        }
        // Titles and colours: Rec.709 Gamma 2.4, white at the Graphics White Level.
        let mut graphics = Vec::new();
        let gain = rcm.graphics_white / SDR_WHITE;
        let graphics_space = space_of(GRAPHICS);
        if (gain - 1.0).abs() < 1e-9 {
            Self::push(
                &mut graphics,
                self.between(&graphics_space.ocio, &timeline.ocio)?,
            );
        } else {
            Self::push(&mut graphics, self.between(&graphics_space.ocio, LINEAR)?);
            graphics.push(tone::matrix_op(&diagonal(gain)));
            Self::push(&mut graphics, self.between(LINEAR, &timeline.ocio)?);
        }
        // Out.
        if rcm.output == cm::BYPASS {
            let out = Output {
                signal: Signal {
                    luma,
                    transfer: timeline.input.map_or(Gamma::Rec709, |space| space.gamma),
                },
                ..output_of(&timeline, None)
            };
            return Ok((graphics, Vec::new(), luma, out));
        }
        let output = space_of(rcm.output);
        let timeline_hdr = map > SDR_WHITE;
        let output_gamma = output.input.map(|space| space.gamma);
        let output_hdr = output_gamma.is_some_and(Gamma::is_hdr);
        let OutputPeak { peak, rule, expand } = output_peak(rcm, custom);
        let kind = if rule && rcm.output_drt == ToneMapping::None {
            ToneMapping::Luminance
        } else {
            rcm.output_drt
        };
        let from = if timeline_hdr { clip } else { SDR_WHITE };
        let mapped = (kind != ToneMapping::None
            && output_gamma.is_some_and(|gamma| gamma != Gamma::Linear))
        .then(|| drt(kind, from, peak, rec2020, rcm.output_sat_rolloff))
        .flatten()
        .filter(|drt| from > peak || expand && !drt.is_identity());
        let reference = (!timeline_hdr && output_hdr && rcm.hdr_203_nits && mapped.is_none())
            .then(|| diagonal(REFERENCE_WHITE / SDR_WHITE));
        let limit = rcm.limit_gamut.and_then(limit_of);
        let adapt = (!rcm.white_point_adaptation)
            .then(|| unadapt(D65, white_of(rcm.output)))
            .flatten();
        let mut ops = Vec::new();
        if mapped.is_none() && reference.is_none() && limit.is_none() && adapt.is_none() {
            Self::push(&mut ops, self.between(&timeline.ocio, &output.ocio)?);
        } else {
            Self::push(&mut ops, self.between(&timeline.ocio, LINEAR)?);
            ops.extend(reference.map(|m| tone::matrix_op(&m)));
            ops.extend(mapped.and_then(|drt| drt.op()));
            ops.extend(limit.map(|(into, back)| tone::limit_op(&into, &back)));
            ops.extend(adapt.map(|m| tone::matrix_op(&m)));
            Self::push(&mut ops, self.between(LINEAR, &output.ocio)?);
        }
        ops.push(clipped(output_gamma));
        Ok((graphics, ops, luma, rcm_output(rcm, custom, luma)))
    }

    fn aces(
        &self,
        aces: &Aces<'_>,
        key: &Key,
        media: &mut HashMap<String, MediaColour>,
    ) -> Result<Made, ocio::Error> {
        let timeline = space_of(aces.timeline);
        let luma = luma_of(&self.config, &timeline.ocio)?;
        let mut made: HashMap<String, Vec<ColorOp>> = HashMap::new();
        for (id, own) in &key.media {
            let Some(MediaInput::Space(name)) = &own.input else {
                continue;
            };
            let ops = if let Some(ops) = made.get(name) {
                ops.clone()
            } else {
                let space = space_of(name);
                let mut ops = Vec::new();
                if aces.gamut_compress {
                    Self::push(&mut ops, self.between(&space.ocio, ACES)?);
                    Self::push(
                        &mut ops,
                        self.ops
                            .op(&self.config.builtin(GAMUT_COMPRESS, Direction::Forward)?)?,
                    );
                    Self::push(&mut ops, self.between(ACES, &timeline.ocio)?);
                } else {
                    Self::push(&mut ops, self.between(&space.ocio, &timeline.ocio)?);
                }
                made.insert(name.clone(), ops.clone());
                ops
            };
            media.entry(id.clone()).or_default().input = ops;
        }
        let mut graphics = Vec::new();
        Self::push(
            &mut graphics,
            self.between(&space_of(GRAPHICS).ocio, &timeline.ocio)?,
        );
        let mut ops = Vec::new();
        match aces.odt.output {
            OdtOutput::View { display, view } => {
                let config = ocio::builtin_config(aces.version.config())?;
                Self::push(
                    &mut ops,
                    self.ops.op(&config.display_view(
                        &timeline.ocio,
                        display,
                        view,
                        Direction::Forward,
                    )?)?,
                );
                ops.push(tone::clamp_op(0.0, 1.0));
            }
            OdtOutput::Space(name) => {
                Self::push(
                    &mut ops,
                    self.between(&timeline.ocio, &space_of(name).ocio)?,
                );
            }
            OdtOutput::Aces => {
                Self::push(&mut ops, self.between(&timeline.ocio, ACES)?);
            }
        }
        Ok((graphics, ops, luma, aces_output(aces)))
    }
}

/// The most nits of the output of RCM, and whether the rule of SDR and HDR of pp. 232–233
/// applies and the output maps up.
struct OutputPeak {
    peak: f64,
    rule: bool,
    expand: bool,
}

/// The most nits that the output of RCM holds, as its output space and timeline make it: SDR's
/// 100; of PQ, the working luminance of a timeline of HDR, or 1000 where a timeline of SDR is
/// mapped up (the inverse DRT, or the rule of p. 232 between a preset and an output of HDR), else
/// 100, or 203 by the reference of BT.2408 (pp. 238–239); of HLG its nominal 1000.
fn output_peak(rcm: &Rcm<'_>, custom: bool) -> OutputPeak {
    let (map, _) = rcm.luminance;
    let timeline_hdr = map > SDR_WHITE;
    let output_gamma = (rcm.output != cm::BYPASS)
        .then(|| space_of(rcm.output).input.map(|space| space.gamma))
        .flatten();
    let output_hdr = output_gamma.is_some_and(Gamma::is_hdr);
    // The rule of pp. 232–233 between a preset and an output of the other range.
    let rule = !custom && timeline_hdr != output_hdr;
    let expand = rcm.inverse_drt || rule;
    let peak = match output_gamma {
        Some(Gamma::St2084) if timeline_hdr => map,
        Some(Gamma::St2084 | Gamma::Hlg) if expand => SDR_TO_HDR_PEAK,
        Some(Gamma::Hlg) => HLG_PEAK,
        Some(Gamma::St2084) if rcm.hdr_203_nits => REFERENCE_WHITE,
        _ => SDR_WHITE,
    };
    OutputPeak { peak, rule, expand }
}

/// What the frames of RCM are put out as; `luma` that of the timeline, for a bypassed output.
fn rcm_output(rcm: &Rcm<'_>, custom: bool, luma: [f32; 3]) -> Output {
    if rcm.output == cm::BYPASS {
        let timeline = space_of(rcm.timeline);
        return Output {
            signal: Signal {
                luma,
                transfer: timeline.input.map_or(Gamma::Rec709, |space| space.gamma),
            },
            ..output_of(&timeline, None)
        };
    }
    let mut out = output_of(&space_of(rcm.output), rcm.limit_gamut);
    if let Some(hdr) = &mut out.hdr {
        hdr.peak = output_peak(rcm, custom).peak;
    }
    out
}

/// What the frames of ACES are put out as by its ODT.
fn aces_output(aces: &Aces<'_>) -> Output {
    match aces.odt.output {
        OdtOutput::View { display, view } => display_output(display, view),
        OdtOutput::Space(name) => output_of(&space_of(name), None),
        OdtOutput::Aces => Output {
            primaries: 2,
            transfer: 8,
            matrix_code: 2,
            ..Output::default()
        },
    }
}

/// What the frames of a project of `settings` are put out as, made without a card: what an export
/// is tagged with before its first frame.
///
/// # Errors
/// OCIO's, where the luma of a bypassed output's timeline cannot be found.
pub fn output_of_settings(settings: &ColorSettings) -> Result<Output, String> {
    Ok(match settings.pipeline() {
        Pipeline::Unmanaged => Output::default(),
        Pipeline::Rcm(rcm) => {
            let custom = !settings.automatic && settings.preset == Preset::Custom;
            let config = ocio::config().map_err(|e| e.to_string())?;
            let luma = luma_of(&config, &space_of(rcm.timeline).ocio).map_err(|e| e.to_string())?;
            rcm_output(&rcm, custom, luma)
        }
        Pipeline::Aces(aces) => aces_output(&aces),
    })
}

/// The luma coefficients that the grades of a project of `settings` take, made without a card:
/// those of its timeline space, or of Rec.709 where it is not managed (as [`Colour::luma`]).
///
/// # Errors
/// OCIO's, where the luma of the timeline space cannot be found.
pub fn grade_luma(settings: &ColorSettings) -> Result<[f32; 3], String> {
    let timeline = match settings.pipeline() {
        Pipeline::Unmanaged => return Ok(REC709_LUMA),
        Pipeline::Rcm(rcm) => rcm.timeline,
        Pipeline::Aces(aces) => aces.timeline,
    };
    let config = ocio::config().map_err(|e| e.to_string())?;
    luma_of(&config, &space_of(timeline).ocio).map_err(|e| e.to_string())
}

/// The light of a frame of PQ read back for an export, of `bits` 10 (P010) whose Y'CbCr is of
/// BT.2020 of limited range: the most nits of any channel of any pixel, and the mean over the
/// pixels of the most of their channels (CTA-861.3's `MaxCLL` and `MaxFALL` of one frame).
/// Each pair of rows on a thread of its own, at most eight.
#[must_use]
pub fn light_of(picture: &columbina_media::Picture) -> (f64, f64) {
    let (width, height) = (picture.width as usize, picture.height as usize);
    if picture.bits <= 8 || width == 0 || height == 0 {
        return (0.0, 0.0);
    }
    // The nits of each code of 10 bits of PQ.
    let nits: Vec<f64> = (0..1024)
        .map(|code| pq_nits(f64::from(code) / 1023.0))
        .collect();
    let word =
        |plane: &[u8], at: usize| f64::from(u16::from_le_bytes([plane[at], plane[at + 1]]) >> 6);
    let (kr, kb) = (0.2627, 0.0593);
    let kg = 1.0 - kr - kb;
    let pairs = height.div_ceil(2);
    let threads = 8.min(pairs);
    let per = pairs.div_ceil(threads);
    let parts: Vec<(f64, f64)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|part| {
                let nits = &nits;
                scope.spawn(move || {
                    let (mut most, mut sum) = (0.0f64, 0.0f64);
                    for pair in part * per..((part + 1) * per).min(pairs) {
                        for x in 0..width {
                            let at = pair * picture.chroma_stride + (x / 2) * 4;
                            let cb = (word(&picture.chroma, at) - 512.0) / 896.0;
                            let cr = (word(&picture.chroma, at + 2) - 512.0) / 896.0;
                            for y in [2 * pair, 2 * pair + 1] {
                                if y >= height {
                                    continue;
                                }
                                let luma = (word(&picture.luma, y * picture.luma_stride + x * 2)
                                    - 64.0)
                                    / 876.0;
                                let r = luma + 2.0 * (1.0 - kr) * cr;
                                let b = luma + 2.0 * (1.0 - kb) * cb;
                                let g = luma
                                    - 2.0 * kb * (1.0 - kb) / kg * cb
                                    - 2.0 * kr * (1.0 - kr) / kg * cr;
                                let code = r.max(g).max(b).clamp(0.0, 1.0);
                                #[allow(
                                    clippy::cast_possible_truncation,
                                    clippy::cast_sign_loss,
                                    reason = "a code from 0 to 1023"
                                )]
                                let light = nits[(code * 1023.0).round() as usize];
                                most = most.max(light);
                                sum += light;
                            }
                        }
                    }
                    (most, sum)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or((0.0, 0.0)))
            .collect()
    });
    let most = parts.iter().fold(0.0f64, |most, part| most.max(part.0));
    #[allow(clippy::cast_precision_loss, reason = "pixels of a frame")]
    let mean = parts.iter().map(|part| part.1).sum::<f64>() / (width * height) as f64;
    (most, mean)
}

/// The nits of a code of PQ (SMPTE ST 2084), from 0 to 1.
fn pq_nits(code: f64) -> f64 {
    let (m1, m2) = (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0);
    let (c1, c2, c3) = (
        3424.0 / 4096.0,
        2413.0 / 4096.0 * 32.0,
        2392.0 / 4096.0 * 32.0,
    );
    let p = code.max(0.0).powf(1.0 / m2);
    10_000.0 * ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1)
}

/// ACES2065-1, where the reference gamut compression is done.
const ACES: &str = "ACES2065-1";

/// OCIO's built-in transform of the reference gamut compression of ACES 1.3 (p. 251).
const GAMUT_COMPRESS: &str = "ACES-LMT - ACES 1.3 Reference Gamut Compression";

/// A processor of the config, made once for the process for each pair of spaces: making one
/// reads the config's transforms, which takes a millisecond or more for some.
fn processor(config: &ocio::Config, from: &str, to: &str) -> Result<Processor, ocio::Error> {
    static MADE: OnceLock<Mutex<HashMap<(String, String), Processor>>> = OnceLock::new();
    let made = MADE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (from.to_owned(), to.to_owned());
    if let Some(processor) = made
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return Ok(processor.clone());
    }
    let processor = config.processor(from, to)?;
    made.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, processor.clone());
    Ok(processor)
}

/// A space of the catalogue, by its name; Rec.709 Gamma 2.4 for one that is not (which the
/// model refuses to set).
fn space_of(name: &str) -> catalogue::Space {
    catalogue::by_name(name)
        .or_else(|| catalogue::by_name(cm::REC709))
        .cloned()
        .expect("the catalogue has Rec.709 Gamma 2.4")
}

/// The most nits of what a space holds, where its file says no more (`mastered`): SDR's 100; PQ
/// its 10000; HLG 1000; linear light and the spaces of cameras and of grading, which hold a
/// scene, 10000 (as `DaVinci` Wide Gamut keeps highlights "up to 10,000 nits", p. 232).
fn source_peak(space: &catalogue::Space, mastered: Option<u32>) -> f64 {
    match space.input.map(|space| space.gamma) {
        Some(Gamma::St2084) => mastered.map_or(PQ_PEAK, f64::from),
        Some(Gamma::Hlg) => HLG_PEAK,
        Some(Gamma::Linear) | None => PQ_PEAK,
        Some(_) => SDR_WHITE,
    }
}

/// A DRT of `kind` from `from` nits to `to`, on linear Rec.2020; none of None.
#[allow(
    clippy::cast_possible_truncation,
    reason = "nits, as floats of the card"
)]
fn drt(
    kind: ToneMapping,
    from: f64,
    to: f64,
    luma: [f32; 3],
    rolloff: cm::SatRolloff,
) -> Option<Drt> {
    let kind = match kind {
        ToneMapping::None => return None,
        ToneMapping::Simple => Kind::Simple,
        ToneMapping::Luminance => Kind::Luminance,
        ToneMapping::Davinci => Kind::Davinci,
        ToneMapping::SaturationPreserving => Kind::SaturationPreserving,
    };
    Some(Drt {
        kind,
        from: from as f32,
        to: to as f32,
        luma,
        rolloff: [rolloff.start as f32, rolloff.limit as f32],
    })
}

/// The clipping at the end of the output (p. 227): each channel within 0 to 1 of an encoding,
/// and from 0 of linear light.
fn clipped(gamma: Option<Gamma>) -> ColorOp {
    if matches!(gamma, Some(Gamma::Linear)) {
        tone::clamp_op(0.0, 65504.0)
    } else {
        tone::clamp_op(0.0, 1.0)
    }
}

/// A matrix that multiplies each channel by `gain`.
fn diagonal(gain: f64) -> Matrix {
    [[gain, 0.0, 0.0], [0.0, gain, 0.0], [0.0, 0.0, gain]]
}

const D65: [f64; 2] = [0.3127, 0.3290];
const DCI_WHITE: [f64; 2] = [0.314, 0.351];
const ACES_WHITE: [f64; 2] = [0.32168, 0.33767];

/// The white of a space of the catalogue: of DCI-P3, DCI's; of ACES, its own (near D60); else
/// D65.
fn white_of(name: &str) -> [f64; 2] {
    let space = space_of(name);
    match space.input.map(|space| space.gamut) {
        Some(Gamut::DciP3) => DCI_WHITE,
        _ if name.starts_with("ACES") => ACES_WHITE,
        _ => D65,
    }
}

/// The chromaticities of a gamut: red, green, blue and white.
fn chromaticities(gamut: Gamut) -> Option<[[f64; 2]; 4]> {
    Some(match gamut {
        Gamut::Rec709 => [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06], D65],
        Gamut::Rec601Pal => [[0.64, 0.33], [0.29, 0.60], [0.15, 0.06], D65],
        Gamut::Rec601Ntsc => [[0.630, 0.340], [0.310, 0.595], [0.155, 0.070], D65],
        Gamut::Rec2020 => [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046], D65],
        Gamut::DciP3 => [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060], DCI_WHITE],
        Gamut::P3D65 => [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060], D65],
        Gamut::CameraNative => return None,
    })
}

fn xyz(xy: [f64; 2]) -> [f64; 3] {
    [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]]
}

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn invert(matrix: &Matrix) -> Matrix {
    let m = matrix;
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    // The cofactor of row `row` and column `column`, over the determinant.
    let cofactor = |row: usize, column: usize| {
        let (r1, r2) = ((row + 1) % 3, (row + 2) % 3);
        let (c1, c2) = ((column + 1) % 3, (column + 2) % 3);
        (m[r1][c1] * m[r2][c2] - m[r1][c2] * m[r2][c1]) / det
    };
    std::array::from_fn(|row| std::array::from_fn(|column| cofactor(column, row)))
}

/// The matrix from linear RGB of the chromaticities `rgbw` to CIE XYZ.
fn to_xyz(rgbw: [[f64; 2]; 4]) -> Matrix {
    let primaries = [xyz(rgbw[0]), xyz(rgbw[1]), xyz(rgbw[2])];
    let p: Matrix = std::array::from_fn(|i| std::array::from_fn(|j| primaries[j][i]));
    let white = xyz(rgbw[3]);
    let s = tone::apply_matrix(&invert(&p), white);
    std::array::from_fn(|i| std::array::from_fn(|j| p[i][j] * s[j]))
}

/// Bradford's adaptation from the white `from` to `to`, in XYZ.
fn bradford(from: [f64; 2], to: [f64; 2]) -> Matrix {
    const B: Matrix = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let (a, b) = (
        tone::apply_matrix(&B, xyz(from)),
        tone::apply_matrix(&B, xyz(to)),
    );
    let scale = [
        [b[0] / a[0], 0.0, 0.0],
        [0.0, b[1] / a[1], 0.0],
        [0.0, 0.0, b[2] / a[2]],
    ];
    multiply(&invert(&B), &multiply(&scale, &B))
}

/// The linear Rec.2020 of [`LINEAR`] to XYZ.
fn rec2020() -> Matrix {
    to_xyz(chromaticities(Gamut::Rec2020).expect("a gamut"))
}

/// Where white points differ: the matrix of linear Rec.2020 that undoes OCIO's adaptation of the
/// white `from` to `to` (Use White Point Adaptation off, p. 242), so that a white keeps its own
/// chromaticity; none where they are the same.
fn unadapt(from: [f64; 2], to: [f64; 2]) -> Option<Matrix> {
    if (from[0] - to[0]).abs() < 1e-6 && (from[1] - to[1]).abs() < 1e-6 {
        return None;
    }
    let into = rec2020();
    Some(multiply(
        &invert(&into),
        &multiply(&bradford(to, from), &into),
    ))
}

/// The matrices into the linear light of a smaller gamut, and back, from linear Rec.2020: Limit
/// Output Gamut To (p. 239), white adapted by Bradford where it is another.
fn limit_of(gamut: Gamut) -> Option<(Matrix, Matrix)> {
    let rgbw = chromaticities(gamut)?;
    let into = multiply(
        &invert(&to_xyz(rgbw)),
        &multiply(&bradford(D65, rgbw[3]), &rec2020()),
    );
    Some((into, invert(&into)))
}

/// The luma coefficients of a space of OCIO: those of its primaries, found from the processor
/// into linear Rec.2020 (its matrix, as the processor's slopes at a grey, each row summing to 1),
/// so that a space of log or of a camera has those of its gamut; of a space whose white is not
/// D65 (ACES), of its primaries as OCIO adapts them to D65. Kept for the process.
fn luma_of(config: &ocio::Config, space: &str) -> Result<[f32; 3], ocio::Error> {
    static MADE: OnceLock<Mutex<HashMap<String, [f32; 3]>>> = OnceLock::new();
    let made = MADE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(luma) = made
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(space)
    {
        return Ok(*luma);
    }
    let to = processor(config, space, LINEAR)?;
    let from = processor(config, LINEAR, space)?;
    let grey = from.apply_one([0.18; 3])?;
    let step = 1e-3;
    let mut columns = [[0.0f64; 3]; 3];
    for (j, column) in columns.iter_mut().enumerate() {
        let mut up = grey;
        let mut down = grey;
        up[j] += step;
        down[j] -= step;
        let (up, down) = (to.apply_one(up)?, to.apply_one(down)?);
        *column = std::array::from_fn(|i| f64::from(up[i] - down[i]));
    }
    // The matrix, rows of the timeline's linear into Rec.2020's, each scaled to sum to 1.
    let rows: Matrix = std::array::from_fn(|i| {
        let sum: f64 = (0..3).map(|j| columns[j][i]).sum();
        std::array::from_fn(|j| columns[j][i] / sum)
    });
    let weights = REC2020_LUMA.map(f64::from);
    #[allow(clippy::cast_possible_truncation, reason = "coefficients of the card")]
    let luma: [f32; 3] =
        std::array::from_fn(|j| (0..3).map(|i| weights[i] * rows[i][j]).sum::<f64>() as f32);
    made.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(space.to_owned(), luma);
    Ok(luma)
}

/// What a space of the catalogue is put out as, `limit` the gamut its picture is limited to.
fn output_of(space: &catalogue::Space, limit: Option<Gamut>) -> Output {
    let Some(InputSpace { gamut, gamma }) = space.input else {
        // A space of a camera or of grading: no tag of H.273 says it.
        return Output {
            primaries: 2,
            transfer: 2,
            matrix_code: 2,
            ..Output::default()
        };
    };
    let (primaries, matrix, matrix_code, luma) = match gamut {
        Gamut::Rec709 => (1, YuvMatrix::Bt709, 1, REC709_LUMA),
        Gamut::Rec601Pal => (5, YuvMatrix::Bt601, 5, REC601_LUMA),
        Gamut::Rec601Ntsc => (6, YuvMatrix::Bt601, 6, REC601_LUMA),
        Gamut::Rec2020 => (9, YuvMatrix::Bt2020, 9, REC2020_LUMA),
        Gamut::DciP3 => (11, YuvMatrix::Bt2020, 9, p3_luma()),
        Gamut::P3D65 => (12, YuvMatrix::Bt2020, 9, p3_luma()),
        Gamut::CameraNative => (2, YuvMatrix::Bt709, 2, REC709_LUMA),
    };
    let transfer = match gamma {
        Gamma::Rec709 => 1,
        Gamma::Gamma22 => 4,
        Gamma::Gamma28 => 5,
        Gamma::Srgb => 13,
        Gamma::Linear => 8,
        Gamma::St2084 => 16,
        Gamma::Hlg => 18,
        Gamma::Gamma26 => 2,
    };
    let hdr = gamma.is_hdr().then(|| {
        let rgbw = chromaticities(limit.unwrap_or(gamut))
            .or_else(|| chromaticities(gamut))
            .unwrap_or([[0.0; 2]; 4]);
        Hdr {
            pq: gamma == Gamma::St2084,
            peak: if gamma == Gamma::Hlg {
                HLG_PEAK
            } else {
                SDR_WHITE
            },
            primaries: [rgbw[0], rgbw[1], rgbw[2]],
            white: rgbw[3],
            min_nits: MASTERING_MIN_NITS,
        }
    });
    Output {
        primaries,
        transfer,
        matrix_code,
        matrix,
        signal: Signal {
            luma,
            transfer: gamma,
        },
        hdr,
    }
}

/// The luma coefficients of P3 (D65): the Y of its primaries.
#[allow(clippy::cast_possible_truncation, reason = "coefficients of the card")]
fn p3_luma() -> [f32; 3] {
    let m = to_xyz(chromaticities(Gamut::P3D65).expect("a gamut"));
    m[1].map(|y| y as f32)
}

/// What a view shows the frames of `output` as: HDR of PQ or HLG, by the most nits they hold, or
/// SDR.
#[must_use]
#[allow(clippy::cast_possible_truncation, reason = "nits")]
pub fn content(output: &Output) -> Content {
    match output.hdr {
        Some(hdr) => Content::Hdr {
            transfer: if hdr.pq {
                HdrTransfer::Pq
            } else {
                HdrTransfer::Hlg
            },
            peak_nits: hdr.peak as f32,
        },
        None => Content::Sdr,
    }
}

/// What a view shows the pictures of media of `facts` as, as they are (the source monitor): HDR
/// where their file says PQ or HLG, else SDR.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "nits")]
pub fn content_of_media(facts: &columbina_model::Facts) -> Content {
    let Some(video) = &facts.video else {
        return Content::Sdr;
    };
    let (space, _) = columbina_model::input_space(video, None, InputSpace::REC709);
    let peak = video
        .color
        .as_ref()
        .and_then(|color| color.mastering)
        .and_then(|mastering| mastering.luminance)
        .map(|luminance| luminance.max / 10_000)
        .filter(|&nits| nits >= 100);
    match space.gamma {
        Gamma::St2084 => Content::Hdr {
            transfer: HdrTransfer::Pq,
            peak_nits: peak.map_or(10_000.0, |nits| nits as f32),
        },
        Gamma::Hlg => Content::Hdr {
            transfer: HdrTransfer::Hlg,
            peak_nits: 1_000.0,
        },
        _ => Content::Sdr,
    }
}

/// What an ODT of ACES to a display puts out, by the names of OCIO's built-in configs.
fn display_output(display: &str, view: &str) -> Output {
    let space = |name: &str| catalogue::by_name(name).cloned();
    let nits = view
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|n| n.parse::<f64>().ok())
        .find(|&n| (100.0..=10_000.0).contains(&n));
    let limited = (view.contains("P3 D65") || view.contains("P3-D65")).then_some(Gamut::P3D65);
    let named = if display.contains("PQ") && display.contains("2100") {
        space("Rec.2100 ST2084")
    } else if display.contains("HLG") {
        space("Rec.2100 HLG")
    } else if display.contains("ST2084") && display.contains("P3") {
        space("P3-D65 ST2084")
    } else if display.starts_with("sRGB") {
        space("sRGB")
    } else if display.contains("Display P3") {
        space("P3-D65 sRGB")
    } else if display.contains("2.2") {
        space("Rec.709 Gamma 2.2")
    } else if display.contains("P3-D65") || display.contains("P3-D60") {
        space("P3-D65 Gamma 2.6")
    } else if display.contains("P3-DCI") {
        space("DCI-P3 Gamma 2.6")
    } else {
        space(cm::REC709)
    };
    let Some(named) = named else {
        return Output::default();
    };
    let mut out = output_of(&named, limited);
    if let Some(hdr) = &mut out.hdr {
        if let Some(nits) = nits {
            hdr.peak = nits;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use columbina_compose::TextureKind;

    /// What each layer's input and the output are made of, as the bodies of their operations.
    fn bodies(ops: &[ColorOp]) -> Vec<String> {
        ops.iter().map(|op| op.body.to_string()).collect()
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "a scene of two layers, and each stage of it"
    )]
    fn the_luts_of_the_project_come_after_the_input_transform_and_before_the_output_transform() {
        let Some(gpu) = columbina_compose::gpu_for_tests() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("half.cube"),
            "LUT_1D_SIZE 2\n0 0 0\n0.5 0.5 0.5\n",
        )
        .unwrap();
        let mut cube = String::from("LUT_3D_SIZE 2\n");
        for _ in 0..8 {
            cube.push_str("0.5 0.5 0.5\n");
        }
        std::fs::write(dir.path().join("grey.cube"), cube).unwrap();
        let places = LutPlaces::with_library(dir.path(), vec![]);
        let ops = CardOps::new(Arc::clone(&gpu));
        let both = LutPair {
            one: Some("half.cube".into()),
            three: Some("grey.cube".into()),
        };
        let managed = ColorSettings {
            science: cm::Science::YrgbManaged,
            preset: Preset::Custom,
            timeline: "DaVinci Wide Gamut/DaVinci Intermediate".into(),
            ..ColorSettings::default()
        };
        let aces = ColorSettings {
            science: cm::Science::Acescct,
            ..ColorSettings::default()
        };
        for settings in [ColorSettings::default(), managed, aces] {
            let mut project = columbina_model::Project::sample();
            project.color = settings.clone();
            let without = Colour::make(&ops, &places, Key::of(&project)).unwrap();
            project.color.input_lut = both.clone();
            project.color.output_lut = both.clone();
            project.color.viewer_lut = LutPair {
                one: None,
                three: Some("grey.cube".into()),
            };
            let with = Colour::make(&ops, &places, Key::of(&project)).unwrap();
            assert_eq!(with.missing(), []);
            let scene = |colour: &Colour| {
                let mut scene = Scene {
                    layers: vec![
                        columbina_compose::SceneLayer {
                            source: Source::Picture(Arc::default(), 0),
                            transform: columbina_compose::Transform::NONE,
                            opacity: 1.0,
                            color: columbina_compose::LayerColor::default(),
                            crop: columbina_compose::Crop::NONE,
                            composite: columbina_compose::Composite::Normal,
                        },
                        columbina_compose::SceneLayer {
                            source: Source::Color([1.0; 4]),
                            transform: columbina_compose::Transform::NONE,
                            opacity: 1.0,
                            color: columbina_compose::LayerColor::default(),
                            crop: columbina_compose::Crop::NONE,
                            composite: columbina_compose::Composite::Normal,
                        },
                    ],
                    ..Scene::default()
                };
                colour.apply(&mut scene, &[Some("m1"), None]);
                scene
            };
            let (plain, looked) = (scene(&without), scene(&with));
            let lut_ops = &looked.layers[0].color.input.ops;
            let transform = bodies(&plain.layers[0].color.input.ops);
            // The picture: its input transform (none in YRGB), then the 1D, then the 3D.
            assert_eq!(bodies(lut_ops)[..transform.len()], transform[..]);
            let luts = &lut_ops[transform.len()..];
            assert_eq!(luts.len(), 2, "{:?}", settings.science);
            assert_eq!(luts[0].textures[0].kind, TextureKind::D2);
            assert_eq!(luts[1].textures[0].kind, TextureKind::D3);
            // A colour has no Input LUT.
            assert_eq!(
                bodies(&looked.layers[1].color.input.ops),
                bodies(&plain.layers[1].color.input.ops)
            );
            // The frame: the 1D, the 3D, then the output transform.
            let output = &looked.output.ops;
            assert_eq!(output.len(), plain.output.ops.len() + 2);
            assert_eq!(output[0].textures[0].kind, TextureKind::D2);
            assert_eq!(output[1].textures[0].kind, TextureKind::D3);
            assert_eq!(bodies(&output[2..]), bodies(&plain.output.ops));
            // The viewer: its 3D alone, and nothing without.
            let viewer = with.viewer();
            assert_eq!(viewer.ops.len(), 1);
            assert_eq!(viewer.ops[0].textures[0].kind, TextureKind::D3);
            assert!(without.viewer().is_identity());
        }
        // A LUT that cannot be read is left out and told.
        let mut project = columbina_model::Project::sample();
        project.color.output_lut = LutPair {
            one: Some("none.cube".into()),
            three: Some("grey.cube".into()),
        };
        let colour = Colour::make(&ops, &places, Key::of(&project)).unwrap();
        assert_eq!(colour.missing().len(), 1);
        assert_eq!(colour.missing()[0].name, "none.cube");
        let mut scene = Scene::default();
        colour.apply(&mut scene, &[] as &[Option<&str>]);
        assert_eq!(scene.output.ops.len(), 1);
        assert_eq!(scene.output.ops[0].textures[0].kind, TextureKind::D3);
    }

    /// A picture of media and one of a colour, for [`Colour::apply`].
    fn layers(media: usize) -> Scene {
        let picture = || columbina_compose::SceneLayer {
            source: Source::Picture(Arc::default(), 0),
            transform: columbina_compose::Transform::NONE,
            opacity: 1.0,
            color: columbina_compose::LayerColor::default(),
            crop: columbina_compose::Crop::NONE,
            composite: columbina_compose::Composite::Normal,
        };
        let mut layers: Vec<_> = (0..media).map(|_| picture()).collect();
        layers.push(columbina_compose::SceneLayer {
            source: Source::Color([1.0; 4]),
            ..picture()
        });
        Scene {
            layers,
            ..Scene::default()
        }
    }

    /// The sample project with a second media, `m2`, like `m1`.
    fn two_media() -> columbina_model::Project {
        let mut project = columbina_model::Project::sample();
        let mut other = project.media[0].clone();
        other.id = "m2".into();
        project.media.push(other);
        project
    }

    #[test]
    fn the_luts_of_a_source_clip_come_after_the_input_lut_on_its_own_pictures_alone() {
        let Some(gpu) = columbina_compose::gpu_for_tests() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("half.cube"),
            "LUT_1D_SIZE 2\n0 0 0\n0.5 0.5 0.5\n",
        )
        .unwrap();
        let mut cube = String::from("LUT_3D_SIZE 2\n");
        for _ in 0..8 {
            cube.push_str("0.5 0.5 0.5\n");
        }
        std::fs::write(dir.path().join("grey.cube"), cube).unwrap();
        let places = LutPlaces::with_library(dir.path(), vec![]);
        let ops = CardOps::new(Arc::clone(&gpu));
        let managed = ColorSettings {
            science: cm::Science::YrgbManaged,
            preset: Preset::Custom,
            timeline: "DaVinci Wide Gamut/DaVinci Intermediate".into(),
            ..ColorSettings::default()
        };
        for settings in [ColorSettings::default(), managed] {
            let mut project = two_media();
            project.color = settings.clone();
            let plain = Colour::make(&ops, &places, Key::of(&project)).unwrap();
            project.color.input_lut.three = Some("grey.cube".into());
            project.media[0].color_override.lut = LutPair {
                one: Some("half.cube".into()),
                three: Some("grey.cube".into()),
            };
            let colour = Colour::make(&ops, &places, Key::of(&project)).unwrap();
            assert_eq!(colour.missing(), []);
            let (mut before, mut after) = (layers(2), layers(2));
            let media = [Some("m1"), Some("m2"), None];
            plain.apply(&mut before, &media);
            colour.apply(&mut after, &media);
            let kinds = |layer: usize| -> Vec<TextureKind> {
                let transform = before.layers[layer].color.input.ops.len();
                let ops = &after.layers[layer].color.input.ops;
                assert_eq!(
                    bodies(&ops[..transform]),
                    bodies(&before.layers[layer].color.input.ops)
                );
                ops[transform..]
                    .iter()
                    .map(|op| op.textures[0].kind)
                    .collect()
            };
            // m1: its input transform, the project's Input LUT, then its own 1D and 3D.
            assert_eq!(
                kinds(0),
                [TextureKind::D3, TextureKind::D2, TextureKind::D3],
                "{:?}",
                settings.science
            );
            // m2: the Input LUT alone; the colour: nothing.
            assert_eq!(kinds(1), [TextureKind::D3]);
            assert_eq!(kinds(2), []);
            // Nothing of the frame.
            assert_eq!(bodies(&after.output.ops), bodies(&before.output.ops));
        }
        // One that cannot be read is left out and told.
        let mut project = two_media();
        project.media[1].color_override.lut.three = Some("gone.cube".into());
        let colour = Colour::make(&ops, &places, Key::of(&project)).unwrap();
        assert_eq!(colour.missing().len(), 1);
        assert_eq!(colour.missing()[0].name, "gone.cube");
        let mut scene = layers(2);
        colour.apply(&mut scene, &[Some("m1"), Some("m2"), None]);
        assert!(scene.layers[1].color.input.ops.is_empty());
    }

    #[test]
    fn media_whose_inverse_drt_is_off_is_not_mapped_up_where_the_others_are() {
        let Some(gpu) = columbina_compose::gpu_for_tests() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let places = LutPlaces::with_library(dir.path(), vec![]);
        let ops = CardOps::new(Arc::clone(&gpu));
        let mut project = two_media();
        project.color = ColorSettings {
            science: cm::Science::YrgbManaged,
            preset: Preset::Custom,
            timeline: "Rec.2100 ST2084".into(),
            working_luminance: cm::WorkingLuminance::Hdr1000,
            input_drt: ToneMapping::Luminance,
            inverse_drt: true,
            ..ColorSettings::default()
        };
        project.media[1].color_override.inverse_drt = false;
        let colour = Colour::make(&ops, &places, Key::of(&project)).unwrap();
        let mut scene = layers(2);
        colour.apply(&mut scene, &[Some("m1"), Some("m2"), None]);
        let mapped = bodies(&scene.layers[0].color.input.ops);
        let plain = bodies(&scene.layers[1].color.input.ops);
        // m1, of SDR, is mapped up to the working luminance through the inverse DRT: into linear,
        // the DRT, then into the timeline; m2 goes straight into the timeline.
        assert_eq!(plain.len(), 1, "{plain:?}");
        assert!(mapped.len() > plain.len(), "{mapped:?}");
        // As the project's inverse DRT off leaves them all.
        project.color.inverse_drt = false;
        let off = Colour::make(&ops, &places, Key::of(&project)).unwrap();
        let mut scene = layers(2);
        off.apply(&mut scene, &[Some("m1"), Some("m2"), None]);
        assert_eq!(bodies(&scene.layers[0].color.input.ops), plain);
        assert_eq!(bodies(&scene.layers[1].color.input.ops), plain);
    }

    #[test]
    fn the_light_of_a_frame_of_pq_is_its_brightest_channel_and_its_mean() {
        // 4 × 2 of P010: a grey of PQ 0.508 (100 nits) but one pixel of Y' alone at 0.7518
        // (1000 nits) on its left half; codes in the top 10 bits of words.
        let (width, height) = (4u32, 2u32);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "codes"
        )]
        let code = |value: f64| ((64.0 + 876.0 * value).round() as u16) << 6;
        let mut luma = vec![0u8; 8 * 2];
        for (at, value) in [0.508_078, 0.751_827, 0.508_078, 0.508_078]
            .iter()
            .enumerate()
        {
            for row in 0..2 {
                luma[row * 8 + at * 2..][..2].copy_from_slice(&code(*value).to_le_bytes());
            }
        }
        let mut chroma = Vec::new();
        for _ in 0..2 {
            chroma.extend_from_slice(&(512u16 << 6).to_le_bytes());
        }
        let chroma = chroma.repeat(2);
        let picture = columbina_media::Picture {
            frame: 0,
            width,
            height,
            bits: 10,
            luma,
            chroma,
            luma_stride: 8,
            chroma_stride: 8,
        };
        let (most, mean) = light_of(&picture);
        assert!((most - 1000.0).abs() < 15.0, "{most}");
        assert!(
            (mean - (6.0 * 100.0 + 2.0 * most) / 8.0).abs() < 2.0,
            "{mean}"
        );
    }

    #[test]
    #[ignore = "a timing"]
    fn how_long_the_light_of_a_frame_of_4k_takes() {
        let (width, height) = (3840u32, 2160u32);
        let picture = columbina_media::Picture {
            frame: 0,
            width,
            height,
            bits: 10,
            luma: vec![0x80; width as usize * 2 * height as usize],
            chroma: vec![0x80; width as usize * 2 * height as usize / 2],
            luma_stride: width as usize * 2,
            chroma_stride: width as usize * 2,
        };
        let since = std::time::Instant::now();
        for _ in 0..10 {
            std::hint::black_box(light_of(&picture));
        }
        eprintln!("{:.1} ms a frame", since.elapsed().as_secs_f64() * 100.0);
    }

    #[test]
    fn the_codes_of_pq_are_the_nits_of_st_2084() {
        // ST 2084: 0.508078 is 100 nits, 0.751827 is 1000 (RM20 p. 256 has 10-bit 520 and 767).
        assert!((pq_nits(0.508_078) - 100.0).abs() < 0.01);
        assert!((pq_nits(0.751_827) - 1000.0).abs() < 0.1);
        assert!((pq_nits(1.0) - 10_000.0).abs() < 1e-6);
        assert_eq!(pq_nits(0.0), 0.0);
    }

    #[test]
    fn the_luma_of_a_space_is_that_of_its_primaries() {
        let config = ocio::config().unwrap();
        for (space, wanted) in [
            ("Gamma 2.4 Encoded Rec.709", REC709_LUMA),
            ("Rec.2100-PQ - Display", REC2020_LUMA),
            ("ACEScct", [0.272_228_7, 0.674_081_8, 0.053_689_5]),
            (
                "DaVinci Intermediate WideGamut",
                [0.274_119, 0.873_632, -0.147_750],
            ),
        ] {
            let luma = luma_of(&config, space).unwrap();
            // ACES's white is not D65: its primaries are taken as OCIO adapts them to it, which
            // moves its coefficients by some 5e-3 from those of AP1 at its own white.
            let within = if space.starts_with("ACES") {
                6e-3
            } else {
                4e-3
            };
            for c in 0..3 {
                assert!((luma[c] - wanted[c]).abs() < within, "{space}: {luma:?}");
            }
        }
    }

    #[test]
    fn white_is_unadapted_only_between_whites_that_differ() {
        assert!(unadapt(D65, D65).is_none());
        let m = unadapt(DCI_WHITE, D65).unwrap();
        // White of Rec.2020 (D65) goes to the chromaticity of DCI's white.
        let white = tone::apply_matrix(&m, [1.0, 1.0, 1.0]);
        let xyz = tone::apply_matrix(&rec2020(), white);
        let sum: f64 = xyz.iter().sum();
        assert!((xyz[0] / sum - DCI_WHITE[0]).abs() < 1e-4);
        assert!((xyz[1] / sum - DCI_WHITE[1]).abs() < 1e-4);
    }

    #[test]
    fn a_limit_of_gamut_keeps_what_is_inside_it_and_clips_what_is_not() {
        let (into, back) = limit_of(Gamut::Rec709).unwrap();
        // A grey and a colour of Rec.709, in Rec.2020: as they were.
        let inside = tone::apply_matrix(&invert(&into), [0.5, 0.2, 0.1]);
        let kept = tone::apply_limit(&into, &back, inside);
        assert!(kept.iter().zip(inside).all(|(a, b)| (a - b).abs() < 1e-9));
        // Pure green of Rec.2020 lies outside Rec.709: it is changed.
        let outside = tone::apply_limit(&into, &back, [0.0, 1.0, 0.0]);
        assert!((outside[1] - 1.0).abs() > 1e-3 || outside[0].abs() > 1e-3);
    }

    #[test]
    fn the_tags_of_an_output_are_those_of_its_space() {
        let pq = output_of(&space_of("Rec.2100 ST2084"), None);
        assert_eq!((pq.primaries, pq.transfer, pq.matrix_code), (9, 16, 9));
        assert_eq!(pq.matrix, YuvMatrix::Bt2020);
        assert!(pq.hdr.unwrap().pq);
        let hlg = output_of(&space_of("Rec.2100 HLG"), None);
        assert_eq!((hlg.primaries, hlg.transfer), (9, 18));
        let sdr = output_of(&space_of(cm::REC709), None);
        assert_eq!(sdr, Output::default());
        let limited = output_of(&space_of("Rec.2100 ST2084"), Some(Gamut::P3D65)).hdr;
        assert_eq!(limited.unwrap().primaries[0], [0.680, 0.320]);
        let aces = display_output("Rec.2100-PQ - Display", "ACES 2.0 - HDR 1000 nits (P3 D65)");
        assert_eq!((aces.primaries, aces.transfer), (9, 16));
        assert_eq!(aces.hdr.unwrap().peak, 1000.0);
        assert_eq!(
            display_output("sRGB - Display", "ACES 2.0 - SDR 100 nits (Rec.709)").transfer,
            13
        );
    }
}
