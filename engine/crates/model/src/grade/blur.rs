//! The Blur palette of a corrector (Reference Manual 20, pp. 3454–3456): its three modes, Blur,
//! Sharpen and Mist, and the settings they share.
//!
//! The palette is one set of settings shown three ways. Radius, H/V Ratio and Scaling are three
//! columns each, red, green and blue, ganged at first ("By default, these ganged sliders move
//! together as one", p. 3454); Coring Softness, Level and Mix are one value each. Which of them
//! a mode uses is the manual's: Blur, Radius and H/V Ratio (Scaling and the settings beneath are
//! grey on p. 3454); Sharpen, those and Scaling, Coring Softness and Level (Mix is grey on
//! p. 3455); Mist, those and Scaling and Mix (Coring Softness and Level are grey on p. 3456). The
//! manual does not say whether a mode keeps what another set; here they are one set (`mode` is
//! the tab, and what the others set is kept as it is, unused), so that going from one tab to
//! another and back changes nothing.
//!
//! The defaults are those the pages show: Radius and H/V Ratio 0.50, Scaling 0.25, Coring
//! Softness and Level 0.00, Mix 100.00. "The default value of 0.50 results in no effect being
//! applied to the image" (p. 3454); a node at its defaults in any mode is the identity, to the bit
//! (the compositor reads no pixels about each one for it).

use serde::{Deserialize, Serialize};

use super::{Bounds, Param, Rgb};
use crate::ops::OpError;

/// Radius (pp. 3454–3456): 0.5 changes nothing, higher blurs, to 1, and lower sharpens, to the
/// "maximum sharpness" of 0.
pub const RADIUS: Bounds = Bounds::new(0.5, 0.0, 1.0);
/// H/V Ratio: 0.5 alike both ways, higher more across, lower more down.
pub const RATIO: Bounds = Bounds::new(0.5, 0.0, 1.0);
/// Scaling: what the sharpening of a Radius below 0.5 is multiplied by, 0.25 as the pages show
/// it at first (pp. 3455–3456).
pub const SCALING: Bounds = Bounds::new(0.25, 0.0, 1.0);
/// Coring Softness (p. 3455), 0.00 at first.
pub const CORING_SOFTNESS: Bounds = Bounds::new(0.0, 0.0, 1.0);
/// Level (p. 3455): 0 sharpens the whole picture.
pub const LEVEL: Bounds = Bounds::new(0.0, 0.0, 1.0);
/// Mix (p. 3456): 100 adds no superimposed blur.
pub const MIX: Bounds = Bounds::new(100.0, 0.0, 100.0);

/// The tab of the Blur palette (pp. 3454–3456): which settings take part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlurMode {
    /// Radius and H/V Ratio alone: a Gaussian blur above 0.5, a sharpening below it.
    #[default]
    Blur,
    /// Sharpening tailored by Scaling, Coring Softness and Level.
    Sharpen,
    /// Sharpening by Radius and Scaling, mixed by Mix with a superimposed blur.
    Mist,
}

/// Whether each three columns are ganged (the white button left of each name, p. 3454): ganged,
/// a change of one column is made to all three. It changes no picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlurLinks {
    pub radius: bool,
    pub ratio: bool,
    pub scaling: bool,
}

impl Default for BlurLinks {
    fn default() -> Self {
        Self {
            radius: true,
            ratio: true,
            scaling: true,
        }
    }
}

/// The settings of the Blur palette of a corrector (see the module).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blur {
    /// The tab shown.
    pub mode: BlurMode,
    /// Radius of R, G and B.
    pub radius: Rgb,
    /// H/V Ratio of R, G and B.
    pub ratio: Rgb,
    /// Scaling of R, G and B.
    pub scaling: Rgb,
    /// Coring Softness.
    pub coring_softness: Param,
    /// Level.
    pub level: Param,
    /// Mix.
    pub mix: Param,
    /// Whether the columns are ganged.
    pub linked: BlurLinks,
}

impl Default for Blur {
    fn default() -> Self {
        Self {
            mode: BlurMode::Blur,
            radius: Rgb::of(RADIUS),
            ratio: Rgb::of(RATIO),
            scaling: Rgb::of(SCALING),
            coring_softness: CORING_SOFTNESS.param(),
            level: LEVEL.param(),
            mix: MIX.param(),
            linked: BlurLinks::default(),
        }
    }
}

/// Whether a setting is ever other than `neutral`.
fn ever(param: &Param, neutral: f64) -> bool {
    #[allow(
        clippy::float_cmp,
        reason = "a setting compared with its neutral value"
    )]
    let at = matches!(param, Param::Fixed(value) if *value == neutral);
    !at
}

impl Blur {
    /// Whether the Radius of a channel (0 red, 1 green, 2 blue) is ever other than 0.5: then
    /// that channel is blurred or sharpened at some frame, whatever the mode.
    #[must_use]
    pub fn radius_ever(&self, channel: usize) -> bool {
        let radius = [&self.radius.r, &self.radius.g, &self.radius.b];
        radius
            .get(channel)
            .is_some_and(|param| ever(param, RADIUS.default))
    }

    /// Whether Mist superimposes its blur at some frame: in the Mist mode, with a Mix ever below
    /// 100.
    #[must_use]
    pub fn mist_ever(&self) -> bool {
        self.mode == BlurMode::Mist && ever(&self.mix, MIX.default)
    }

    /// Whether the palette changes the picture at some frame, so that it reads the pixels about
    /// each one; a palette that never does is the identity, to the bit.
    #[must_use]
    pub fn reads_neighbourhood(&self) -> bool {
        (0..3).any(|channel| self.radius_ever(channel)) || self.mist_ever()
    }

    pub(super) fn check(&self) -> Result<(), OpError> {
        self.radius
            .check(["blur.radius.r", "blur.radius.g", "blur.radius.b"], RADIUS)?;
        self.ratio
            .check(["blur.ratio.r", "blur.ratio.g", "blur.ratio.b"], RATIO)?;
        self.scaling.check(
            ["blur.scaling.r", "blur.scaling.g", "blur.scaling.b"],
            SCALING,
        )?;
        CORING_SOFTNESS.check("blur.coring_softness", &self.coring_softness)?;
        LEVEL.check("blur.level", &self.level)?;
        MIX.check("blur.mix", &self.mix)
    }
}
