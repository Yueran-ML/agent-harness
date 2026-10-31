//! The picture shown in SDR or in HDR, as the monitor that the window is on is set, and as the
//! view is allowed: "Display HDR On Viewers If Available" (RM20 p244).
//!
//! # The monitor
//!
//! What the monitor says of itself is read through DXGI by wgpu
//! ([`wgpu::Surface::display_hdr_info`]): the monitor that the window is on
//! (`MonitorFromWindow`), its output found among the outputs of every adapter, and
//! `IDXGIOutput6::GetDesc1` read for its colour space (HDR is on where it is
//! `DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020`, or the scRGB `G10_NONE_P709`) and its peak
//! (`MaxLuminance`); the white of SDR comes from `DisplayConfigGetDeviceInfo` with
//! `DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL` ("SDR content brightness" of the settings of
//! Windows). The drawer reads it before the first frame, and again at most every [`CHECK`] while
//! HDR is allowed, so that the window moved to another monitor, HDR turned on or off, or the
//! brightness of SDR changed is followed within a second.
//!
//! # The surface
//!
//! Where HDR is allowed and on: `Rgba16Float` in extended linear sRGB (scRGB,
//! `DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709`, which wgpu sets with `SetColorSpace1`): BT.709
//! primaries, linear, 1.0 = [`SCRGB_WHITE_NITS`] (80 nits), values beyond 1 brighter than SDR
//! white and values below 0 colours beyond BT.709. Otherwise the 8-bit surface of SDR, as
//! before (`choose_surface`).
//!
//! # The mapping
//!
//! The picture (a composed frame, or a picture of a source) is first drawn as before; where the
//! content and the surface differ, it is then mapped ([`Mapping`], `display.wgsl`):
//!
//! - **SDR content in HDR**: `nits = sRGB⁻¹(clamp(v, 0, 1)) × white` (the piecewise sRGB curve,
//!   as Windows shows an SDR window in HDR), `out = nits / 80`: 1.0 lands on the white of SDR
//!   that Windows has, so the picture looks as it does with HDR off.
//! - **HDR content** (PQ, or HLG at a nominal peak, both BT.2020): decoded to nits (PQ is
//!   absolute; HLG by the OOTF of BT.2100, its system gamma `1.2 + 0.42 log10(Lw / 1000)`), taken
//!   into BT.709 by the matrix of BT.2087, its largest channel rolled off to the peak of the
//!   monitor ([`roll_off`]), the others scaled with it so that hue is kept; then `nits / 80` in
//!   HDR, or `sRGB(clamp(nits / 100, 0, 1))` in SDR ([`SDR_REFERENCE_NITS`]).
//! - **SDR content in SDR**: nothing; the picture is drawn as it always was, bit for bit.
//!
//! These are simple stand-ins until the colour management of the project (T7) says what the
//! content is: until then, the content is SDR ([`Content::Sdr`]).

/// The nits of 1.0 in scRGB.
pub const SCRGB_WHITE_NITS: f32 = 80.0;

/// The white of SDR taken where Windows does not say it: the default of its "SDR content
/// brightness".
pub const FALLBACK_SDR_WHITE_NITS: f32 = 80.0;

/// The peak taken for a monitor in HDR that does not say its own: the least of a monitor of VESA
/// `DisplayHDR` 400.
pub const FALLBACK_PEAK_NITS: f32 = 400.0;

/// The white of a monitor in SDR, for HDR content shown in SDR: the reference of SDR (BT.2035),
/// and the SDR luminance of 100 nits of Resolve (p238).
pub const SDR_REFERENCE_NITS: f32 = 100.0;

/// Where the roll-off begins, as a share of the peak that it rolls off to.
pub const KNEE: f32 = 0.75;

/// How often the drawer reads the monitor again while HDR is allowed.
pub const CHECK: std::time::Duration = std::time::Duration::from_secs(1);

/// What the monitor that the window is on says of itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    /// Whether Windows has HDR on for it.
    pub hdr: bool,
    /// Its peak, in nits (DXGI `MaxLuminance`), if it says one.
    pub peak_nits: Option<f32>,
    /// The nits that Windows shows the white of SDR at, if it says.
    pub sdr_white_nits: Option<f32>,
}

impl Monitor {
    /// What wgpu read of the monitor; none where nothing at all is known (no `IDXGIOutput6`, a
    /// window on no monitor).
    #[must_use]
    pub fn of(info: &wgpu::DisplayHdrInfo) -> Option<Monitor> {
        let luminance = info.luminance;
        let hdr = info.coarse.and_then(|coarse| coarse.high_dynamic_range);
        if hdr.is_none() && luminance.is_none() {
            return None;
        }
        let positive = |nits: Option<f32>| nits.filter(|nits| nits.is_finite() && *nits > 0.0);
        Some(Monitor {
            hdr: hdr == Some(true),
            peak_nits: positive(luminance.and_then(|luminance| luminance.max_nits)),
            sdr_white_nits: positive(luminance.and_then(|luminance| luminance.sdr_white_nits)),
        })
    }

    /// The white of SDR, or [`FALLBACK_SDR_WHITE_NITS`].
    #[must_use]
    pub fn sdr_white(&self) -> f32 {
        self.sdr_white_nits.unwrap_or(FALLBACK_SDR_WHITE_NITS)
    }

    /// The peak, or [`FALLBACK_PEAK_NITS`]; never below the white of SDR.
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.peak_nits
            .unwrap_or(FALLBACK_PEAK_NITS)
            .max(self.sdr_white())
    }
}

/// How the signal of HDR content is coded. Its primaries are BT.2020.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HdrTransfer {
    /// SMPTE ST 2084, absolute: 1.0 is 10000 nits.
    Pq,
    /// ARIB STD-B67 (BT.2100 HLG), relative to the peak of the content.
    Hlg,
}

/// What the picture given to the view holds.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Content {
    /// R'G'B' of SDR (BT.709), 1.0 its white: what the view has always been given.
    #[default]
    Sdr,
    /// R'G'B' of HDR, `peak_nits` the brightest that it holds (for HLG, its nominal peak,
    /// `Lw` of BT.2100).
    Hdr {
        transfer: HdrTransfer,
        peak_nits: f32,
    },
}

/// What the surface shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Output {
    /// SDR: R'G'B' from 0 to 1 on an 8-bit surface.
    Sdr,
    /// HDR: scRGB on a surface of half floats, to a monitor of this peak and white of SDR.
    Hdr { peak_nits: f32, sdr_white_nits: f32 },
}

/// What a view shows, as its drawer last found: for the server to say, and the tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    /// What the monitor said of itself, if anything.
    pub monitor: Option<Monitor>,
    /// What the surface shows.
    pub output: Output,
}

/// The format and colour space of a surface, and what it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SurfaceChoice {
    pub format: wgpu::TextureFormat,
    pub color_space: wgpu::SurfaceColorSpace,
    pub output: Output,
}

/// The surface for a window on `monitor`, HDR allowed or not: `Rgba16Float` in scRGB where HDR
/// is allowed, the monitor has it on and the surface takes it; else the SDR surface of before
/// (8 bits that are not sRGB, else the first that is not sRGB, in the colour space that wgpu
/// picks for it). None where the window takes no format that the view draws.
pub(crate) fn choose_surface(
    capabilities: &wgpu::SurfaceCapabilities,
    monitor: Option<&Monitor>,
    hdr_allowed: bool,
) -> Option<SurfaceChoice> {
    let hdr = monitor.filter(|monitor| monitor.hdr && hdr_allowed);
    if let Some(monitor) = hdr {
        if capabilities
            .color_spaces(wgpu::TextureFormat::Rgba16Float)
            .contains(wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR)
        {
            return Some(SurfaceChoice {
                format: wgpu::TextureFormat::Rgba16Float,
                color_space: wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
                output: Output::Hdr {
                    peak_nits: monitor.peak(),
                    sdr_white_nits: monitor.sdr_white(),
                },
            });
        }
    }
    sdr_format(&capabilities.formats).map(|format| SurfaceChoice {
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        output: Output::Sdr,
    })
}

/// The format of an SDR surface: 8 bits that are not sRGB, else the first that is not sRGB, else
/// none.
fn sdr_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ]
    .into_iter()
    .find(|format| formats.contains(format))
    .or_else(|| {
        formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb() && *format != wgpu::TextureFormat::Rgba16Float)
    })
}

/// How a picture of some content is shown on a surface that shows `Output`, where they differ.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mapping {
    pub(crate) content: Content,
    pub(crate) output: Output,
}

/// The mapping of `content` onto `output`; none where there is nothing to map (SDR onto SDR),
/// and the picture is drawn as it always was.
#[must_use]
pub fn mapping(content: Content, output: Output) -> Option<Mapping> {
    match (content, output) {
        (Content::Sdr, Output::Sdr) => None,
        _ => Some(Mapping { content, output }),
    }
}

/// The size of the uniform of `display.wgsl`.
pub(crate) const PARAMS_SIZE: u64 = 32;

impl Mapping {
    /// The nits of the peak that the picture is rolled off to, and of 1.0 of the surface.
    fn target(&self) -> (f32, f32) {
        match self.output {
            Output::Hdr { peak_nits, .. } => (peak_nits, SCRGB_WHITE_NITS),
            Output::Sdr => (SDR_REFERENCE_NITS, SDR_REFERENCE_NITS),
        }
    }

    /// The uniform of `display.wgsl` (`Mapping` there): its kinds, the scale of SDR content, the
    /// gamma of HLG; the knee, the peak of the monitor, the peak of the content and the nits of
    /// 1.0 of the surface.
    pub(crate) fn params(&self) -> [f32; 8] {
        let (transfer, content_peak) = match self.content {
            Content::Sdr => (0.0, 0.0),
            Content::Hdr {
                transfer: HdrTransfer::Pq,
                peak_nits,
            } => (1.0, peak_nits),
            Content::Hdr {
                transfer: HdrTransfer::Hlg,
                peak_nits,
            } => (2.0, peak_nits),
        };
        let (output, sdr_scale) = match self.output {
            Output::Sdr => (0.0, 1.0),
            Output::Hdr { sdr_white_nits, .. } => (1.0, sdr_white_nits / SCRGB_WHITE_NITS),
        };
        let (peak, unit) = self.target();
        #[allow(clippy::cast_possible_truncation, reason = "the gamma in f32")]
        let gamma = hlg_gamma(f64::from(content_peak)) as f32;
        [
            transfer,
            output,
            sdr_scale,
            gamma,
            KNEE * peak,
            peak,
            content_peak,
            unit,
        ]
    }

    /// What the mapping makes of a pixel of R'G'B' as it is drawn: the reference of
    /// `display.wgsl`, on the processor, in f64.
    #[must_use]
    pub fn map(&self, rgb: [f32; 3]) -> [f32; 3] {
        let rgb = rgb.map(f64::from);
        let (peak, unit) = self.target();
        let (peak, unit) = (f64::from(peak), f64::from(unit));
        let mapped = match self.content {
            Content::Sdr => {
                let white = match self.output {
                    Output::Hdr { sdr_white_nits, .. } => f64::from(sdr_white_nits),
                    Output::Sdr => unit,
                };
                rgb.map(|value| srgb_eotf(value.clamp(0.0, 1.0)) * white / unit)
            }
            Content::Hdr {
                transfer,
                peak_nits,
            } => {
                let content_peak = f64::from(peak_nits);
                let nits = match transfer {
                    HdrTransfer::Pq => rgb.map(pq_eotf),
                    HdrTransfer::Hlg => hlg_display(rgb, content_peak),
                };
                let nits = bt2020_to_bt709(nits);
                let largest = nits[0].max(nits[1]).max(nits[2]);
                let scale = if largest > 0.0 {
                    roll_off(largest, f64::from(KNEE) * peak, peak, content_peak) / largest
                } else {
                    1.0
                };
                nits.map(|value| value * scale / unit)
            }
        };
        #[allow(clippy::cast_possible_truncation, reason = "pixels in f32")]
        match self.output {
            Output::Hdr { .. } => mapped.map(|value| value as f32),
            Output::Sdr => mapped.map(|value| srgb_oetf(value.clamp(0.0, 1.0)) as f32),
        }
    }
}

/// The sRGB curve from code to light (IEC 61966-2-1), as Windows shows SDR in HDR.
#[must_use]
pub fn srgb_eotf(value: f64) -> f64 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// The sRGB curve from light to code.
#[must_use]
pub fn srgb_oetf(light: f64) -> f64 {
    if light <= 0.003_130_8 {
        light * 12.92
    } else {
        1.055 * light.powf(1.0 / 2.4) - 0.055
    }
}

/// SMPTE ST 2084: nits of a code of PQ, from 0 to 1.
#[must_use]
pub fn pq_eotf(code: f64) -> f64 {
    let (m1, m2) = (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0);
    let (c1, c2, c3) = (
        3424.0 / 4096.0,
        2413.0 / 4096.0 * 32.0,
        2392.0 / 4096.0 * 32.0,
    );
    let power = code.clamp(0.0, 1.0).powf(1.0 / m2);
    10000.0 * ((power - c1).max(0.0) / (c2 - c3 * power)).powf(1.0 / m1)
}

/// The inverse of `pq_eotf`: the code of PQ of so many nits.
#[must_use]
pub fn pq_oetf(nits: f64) -> f64 {
    let (m1, m2) = (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0);
    let (c1, c2, c3) = (
        3424.0 / 4096.0,
        2413.0 / 4096.0 * 32.0,
        2392.0 / 4096.0 * 32.0,
    );
    let power = (nits / 10000.0).clamp(0.0, 1.0).powf(m1);
    ((c1 + c2 * power) / (1.0 + c3 * power)).powf(m2)
}

/// The system gamma of HLG for a display of `peak` nits (BT.2100, note 5f).
#[must_use]
pub fn hlg_gamma(peak: f64) -> f64 {
    if peak > 0.0 {
        1.2 + 0.42 * (peak / 1000.0).log10()
    } else {
        1.2
    }
}

/// The light of the scene, from 0 to 1, of a code of HLG (the inverse OETF of BT.2100).
#[must_use]
pub fn hlg_inverse_oetf(code: f64) -> f64 {
    let (a, b, c) = (0.178_832_77, 0.284_668_92, 0.559_910_73);
    let code = code.clamp(0.0, 1.0);
    if code <= 0.5 {
        code * code / 3.0
    } else {
        (((code - c) / a).exp() + b) / 12.0
    }
}

/// The nits of R'G'B' of HLG shown at a peak of `peak` nits: the OOTF of BT.2100, its black at 0.
#[must_use]
pub fn hlg_display(codes: [f64; 3], peak: f64) -> [f64; 3] {
    let scene = codes.map(hlg_inverse_oetf);
    let luma = 0.2627 * scene[0] + 0.6780 * scene[1] + 0.0593 * scene[2];
    let gain = if luma > 0.0 {
        peak * luma.powf(hlg_gamma(peak) - 1.0)
    } else {
        0.0
    };
    scene.map(|value| value * gain)
}

/// Linear BT.2020 into linear BT.709 (BT.2087): colours beyond BT.709 come out below 0.
#[must_use]
pub fn bt2020_to_bt709(rgb: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = rgb;
    [
        1.660_491 * r - 0.587_641 * g - 0.072_850 * b,
        -0.124_550 * r + 1.132_900 * g - 0.008_349 * b,
        -0.018_151 * r - 0.100_579 * g + 1.118_730 * b,
    ]
}

/// The roll-off of `nits` into a monitor of `peak` nits, for content whose brightest is
/// `content_peak`: unchanged up to `knee`; above it, where the content goes beyond the peak,
/// `knee + a·f(x)` with `a = peak − knee`, `x = (nits − knee) / a`, `w = (content_peak − knee) /
/// a` and `f(x) = x (1 + x / w²) / (1 + x)` (Reinhard's, with its white at `w`): of slope 1 at the
/// knee, rising, and at the peak exactly where the content is at its brightest. Content that
/// does not go beyond the peak is only clipped at it.
#[must_use]
pub fn roll_off(nits: f64, knee: f64, peak: f64, content_peak: f64) -> f64 {
    if nits <= knee {
        return nits;
    }
    if content_peak <= peak {
        return nits.min(peak);
    }
    let a = peak - knee;
    let w = (content_peak - knee) / a;
    let x = (nits - knee) / a;
    if x >= w {
        return peak;
    }
    knee + a * x * (1.0 + x / (w * w)) / (1.0 + x)
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "values that are exact")]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, within: f64) -> bool {
        (a - b).abs() <= within
    }

    #[test]
    fn the_curves_of_srgb_pq_and_hlg_are_those_of_their_standards() {
        // sRGB: the ends, the joint of its two pieces, and grey.
        assert!(close(srgb_eotf(0.0), 0.0, 1e-12));
        assert!(close(srgb_eotf(1.0), 1.0, 1e-12));
        assert!(close(srgb_eotf(0.5), 0.214_041, 1e-6));
        for code in [0.0, 0.01, 0.040_45, 0.2, 0.5, 0.9, 1.0] {
            // Within the step at the joint of the two pieces, which IEC 61966-2-1 leaves.
            assert!(close(srgb_oetf(srgb_eotf(code)), code, 1e-7), "{code}");
        }
        // PQ: 0, 100 nits at 0.508, 1000 nits at 0.752, 10000 at 1 (ST 2084, BT.2100 tables).
        assert!(close(pq_eotf(0.0), 0.0, 1e-9));
        assert!(close(pq_eotf(1.0), 10000.0, 1e-6));
        assert!(close(pq_oetf(100.0), 0.508_078, 1e-5));
        assert!(close(pq_oetf(1000.0), 0.751_827, 1e-5));
        assert!(close(pq_oetf(203.0), 0.580_690, 1e-5));
        for nits in [0.01, 1.0, 80.0, 203.0, 400.0, 1000.0, 4000.0] {
            assert!(close(pq_eotf(pq_oetf(nits)), nits, nits * 1e-9), "{nits}");
        }
        // HLG: the joint at 0.5 is 1/12 of the scene, 1 is all of it; gamma 1.2 at 1000 nits.
        assert!(close(hlg_inverse_oetf(0.5), 1.0 / 12.0, 1e-12));
        assert!(close(hlg_inverse_oetf(1.0), 1.0, 1e-6));
        assert!(close(hlg_gamma(1000.0), 1.2, 1e-12));
        assert!(close(hlg_gamma(400.0), 1.2 + 0.42 * 0.4_f64.log10(), 1e-12));
        // White at 1 is the peak; HLG's reference white (75%) on 1000 nits is 203 (BT.2408).
        let white = hlg_display([1.0; 3], 1000.0);
        assert!(
            white.iter().all(|nits| close(*nits, 1000.0, 1e-3)),
            "{white:?}"
        );
        let reference = hlg_display([0.75; 3], 1000.0);
        assert!(close(reference[0], 203.0, 1.0), "{reference:?}");
    }

    #[test]
    fn bt_2020_white_is_bt_709_white_and_its_primaries_fall_outside() {
        let white = bt2020_to_bt709([1.0; 3]);
        assert!(
            white.iter().all(|value| close(*value, 1.0, 1e-5)),
            "{white:?}"
        );
        let green = bt2020_to_bt709([0.0, 1.0, 0.0]);
        assert!(
            green[0] < 0.0 && green[2] < 0.0 && green[1] > 1.0,
            "{green:?}"
        );
    }

    #[test]
    fn the_roll_off_keeps_what_is_below_the_knee_and_ends_at_the_peak() {
        let (knee, peak, content) = (300.0, 400.0, 1000.0);
        assert_eq!(roll_off(120.0, knee, peak, content), 120.0);
        assert_eq!(roll_off(knee, knee, peak, content), knee);
        assert!(close(roll_off(content, knee, peak, content), peak, 1e-9));
        assert_eq!(roll_off(4000.0, knee, peak, content), peak);
        // Rising, below the peak, and of slope 1 at the knee.
        let mut last = knee;
        for step in 1..=700 {
            let nits = knee + f64::from(step);
            let rolled = roll_off(nits, knee, peak, content);
            assert!(rolled > last && rolled <= peak, "{nits}: {rolled}");
            last = rolled;
        }
        let slope = (roll_off(knee + 1e-3, knee, peak, content) - knee) / 1e-3;
        assert!(close(slope, 1.0, 1e-3), "{slope}");
        // Content within the peak is only clipped at it.
        assert_eq!(roll_off(350.0, knee, peak, 380.0), 350.0);
        assert_eq!(roll_off(390.0, knee, peak, 380.0), 390.0);
        assert_eq!(roll_off(450.0, knee, peak, 380.0), peak);
    }

    #[test]
    fn sdr_content_in_hdr_puts_its_white_on_the_white_of_sdr_of_windows() {
        let output = Output::Hdr {
            peak_nits: 400.0,
            sdr_white_nits: 240.0,
        };
        let mapping = mapping(Content::Sdr, output).unwrap();
        // White: 240 nits, 3 in scRGB; black 0; grey 0.5 is 0.214 of white; beyond 0..1 clamped.
        assert_eq!(mapping.map([1.0; 3]), [3.0; 3]);
        assert_eq!(mapping.map([0.0; 3]), [0.0; 3]);
        let grey = mapping.map([0.5; 3]);
        assert!(close(f64::from(grey[0]), 0.214_041 * 3.0, 1e-5), "{grey:?}");
        assert_eq!(mapping.map([2.0, -1.0, 1.0]), [3.0, 0.0, 3.0]);
        // At the default white of Windows, 80 nits, white is 1.0 of scRGB.
        let default = super::mapping(
            Content::Sdr,
            Output::Hdr {
                peak_nits: 400.0,
                sdr_white_nits: 80.0,
            },
        )
        .unwrap();
        assert_eq!(default.map([1.0; 3]), [1.0; 3]);
        assert_eq!(default.params()[2], 1.0);
    }

    #[test]
    fn sdr_content_in_sdr_is_not_mapped() {
        assert_eq!(mapping(Content::Sdr, Output::Sdr), None);
    }

    #[test]
    fn hdr_content_is_shown_in_nits_and_rolled_off_to_the_peak_of_the_monitor() {
        let output = Output::Hdr {
            peak_nits: 400.0,
            sdr_white_nits: 200.0,
        };
        let pq = mapping(
            Content::Hdr {
                transfer: HdrTransfer::Pq,
                peak_nits: 1000.0,
            },
            output,
        )
        .unwrap();
        // 100 nits of grey, below the knee (300): 100 nits, 1.25 of scRGB.
        #[allow(clippy::cast_possible_truncation)]
        let grey = pq.map([pq_oetf(100.0) as f32; 3]);
        assert!(
            grey.iter().all(|v| close(f64::from(*v), 1.25, 1e-4)),
            "{grey:?}"
        );
        // 1000 nits, the peak of the content: the peak of the monitor, 5 of scRGB.
        #[allow(clippy::cast_possible_truncation)]
        let top = pq.map([pq_oetf(1000.0) as f32; 3]);
        assert!(
            top.iter().all(|v| close(f64::from(*v), 5.0, 1e-3)),
            "{top:?}"
        );
        // A saturated red keeps its hue: the other channels scaled as the largest is.
        #[allow(clippy::cast_possible_truncation)]
        let red = pq.map([pq_oetf(800.0) as f32, pq_oetf(80.0) as f32, 0.0]);
        let linear = bt2020_to_bt709([800.0, 80.0, 0.0]);
        let ratio = f64::from(red[1]) / f64::from(red[0]);
        assert!(close(ratio, linear[1] / linear[0], 1e-4), "{red:?}");
        assert!(f64::from(red[0]) * 80.0 <= 400.0 + 1e-3);
        // The same content into SDR: 100 nits is white of SDR, everything above it white.
        let sdr = mapping(
            Content::Hdr {
                transfer: HdrTransfer::Pq,
                peak_nits: 1000.0,
            },
            Output::Sdr,
        )
        .unwrap();
        #[allow(clippy::cast_possible_truncation)]
        let white = sdr.map([pq_oetf(1000.0) as f32; 3]);
        assert!(
            white.iter().all(|v| close(f64::from(*v), 1.0, 1e-5)),
            "{white:?}"
        );
        #[allow(clippy::cast_possible_truncation)]
        let dark = sdr.map([pq_oetf(10.0) as f32; 3]);
        assert!(
            dark.iter()
                .all(|v| close(f64::from(*v), srgb_oetf(0.1), 1e-4)),
            "{dark:?}"
        );
        // HLG at 1000 nits: white at the peak of the monitor; its reference white at 203 nits.
        let hlg = mapping(
            Content::Hdr {
                transfer: HdrTransfer::Hlg,
                peak_nits: 1000.0,
            },
            output,
        )
        .unwrap();
        let top = hlg.map([1.0; 3]);
        assert!(
            top.iter().all(|v| close(f64::from(*v), 5.0, 1e-3)),
            "{top:?}"
        );
        let reference = hlg.map([0.75; 3]);
        assert!(
            close(f64::from(reference[0]) * 80.0, 203.0, 1.5),
            "{reference:?}"
        );
    }

    #[test]
    fn the_uniform_says_the_mapping() {
        let mapping = mapping(
            Content::Hdr {
                transfer: HdrTransfer::Hlg,
                peak_nits: 1000.0,
            },
            Output::Hdr {
                peak_nits: 400.0,
                sdr_white_nits: 240.0,
            },
        )
        .unwrap();
        assert_eq!(
            mapping.params(),
            [2.0, 1.0, 3.0, 1.2, 300.0, 400.0, 1000.0, 80.0]
        );
        let sdr = super::mapping(
            Content::Hdr {
                transfer: HdrTransfer::Pq,
                peak_nits: 4000.0,
            },
            Output::Sdr,
        )
        .unwrap();
        let params = sdr.params();
        assert_eq!(params[..2], [1.0, 0.0]);
        assert_eq!(params[4..], [75.0, 100.0, 4000.0, 100.0]);
    }

    fn capabilities(hdr_format: bool) -> wgpu::SurfaceCapabilities {
        // As DX12 reports them (wgpu-hal's `surface_capabilities`).
        let mut formats = vec![
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgb10a2Unorm,
        ];
        if hdr_format {
            formats.push(wgpu::TextureFormat::Rgba16Float);
        }
        let format_capabilities = formats
            .iter()
            .map(|format| wgpu::SurfaceFormatCapabilities {
                format: *format,
                color_spaces: match format {
                    wgpu::TextureFormat::Rgba16Float => {
                        wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR
                    }
                    wgpu::TextureFormat::Rgb10a2Unorm => {
                        wgpu::SurfaceColorSpaces::SRGB | wgpu::SurfaceColorSpaces::BT2100_PQ
                    }
                    _ => wgpu::SurfaceColorSpaces::SRGB,
                },
            })
            .collect();
        wgpu::SurfaceCapabilities {
            formats,
            format_capabilities,
            ..wgpu::SurfaceCapabilities::default()
        }
    }

    fn monitor(hdr: bool) -> Monitor {
        Monitor {
            hdr,
            peak_nits: Some(400.0),
            sdr_white_nits: Some(240.0),
        }
    }

    #[test]
    fn the_surface_is_of_half_floats_in_scrgb_only_where_hdr_is_on_and_allowed() {
        let sdr = SurfaceChoice {
            format: wgpu::TextureFormat::Bgra8Unorm,
            color_space: wgpu::SurfaceColorSpace::Auto,
            output: Output::Sdr,
        };
        let hdr = SurfaceChoice {
            format: wgpu::TextureFormat::Rgba16Float,
            color_space: wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
            output: Output::Hdr {
                peak_nits: 400.0,
                sdr_white_nits: 240.0,
            },
        };
        let caps = capabilities(true);
        assert_eq!(choose_surface(&caps, Some(&monitor(true)), true), Some(hdr));
        assert_eq!(
            choose_surface(&caps, Some(&monitor(true)), false),
            Some(sdr)
        );
        assert_eq!(
            choose_surface(&caps, Some(&monitor(false)), true),
            Some(sdr)
        );
        assert_eq!(
            choose_surface(&caps, Some(&monitor(false)), false),
            Some(sdr)
        );
        assert_eq!(choose_surface(&caps, None, true), Some(sdr));
        // A surface that takes no half floats in scRGB stays in SDR.
        assert_eq!(
            choose_surface(&capabilities(false), Some(&monitor(true)), true),
            Some(sdr)
        );
        // A monitor that says no peak nor white: the fallbacks.
        let silent = Monitor {
            hdr: true,
            peak_nits: None,
            sdr_white_nits: None,
        };
        assert_eq!(
            choose_surface(&caps, Some(&silent), true).map(|choice| choice.output),
            Some(Output::Hdr {
                peak_nits: FALLBACK_PEAK_NITS,
                sdr_white_nits: FALLBACK_SDR_WHITE_NITS,
            })
        );
        // The SDR surface never falls to half floats, nor to sRGB.
        let only = wgpu::SurfaceCapabilities {
            formats: vec![
                wgpu::TextureFormat::Bgra8UnormSrgb,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureFormat::Rgb10a2Unorm,
            ],
            ..wgpu::SurfaceCapabilities::default()
        };
        assert_eq!(
            choose_surface(&only, None, true).map(|choice| choice.format),
            Some(wgpu::TextureFormat::Rgb10a2Unorm)
        );
        assert_eq!(
            choose_surface(&wgpu::SurfaceCapabilities::default(), None, true),
            None
        );
    }

    #[test]
    fn a_monitor_is_read_from_what_dxgi_says() {
        let info = wgpu::DisplayHdrInfo {
            luminance: Some(wgpu::DisplayLuminance {
                max_nits: Some(399.5),
                max_full_frame_nits: Some(270.0),
                min_nits: Some(0.1),
                sdr_white_nits: Some(240.0),
            }),
            coarse: Some(wgpu::DisplayCoarseRange {
                high_dynamic_range: Some(true),
                gamut: None,
            }),
            ..wgpu::DisplayHdrInfo::default()
        };
        assert_eq!(
            Monitor::of(&info),
            Some(Monitor {
                hdr: true,
                peak_nits: Some(399.5),
                sdr_white_nits: Some(240.0),
            })
        );
        assert_eq!(Monitor::of(&wgpu::DisplayHdrInfo::default()), None);
        // Zeros are not known: the fallbacks; a peak below the white is the white.
        let zero = wgpu::DisplayHdrInfo {
            luminance: Some(wgpu::DisplayLuminance {
                max_nits: Some(0.0),
                sdr_white_nits: None,
                ..wgpu::DisplayLuminance::default()
            }),
            ..wgpu::DisplayHdrInfo::default()
        };
        let read = Monitor::of(&zero).unwrap();
        assert!(!read.hdr);
        assert_eq!(read.peak(), FALLBACK_PEAK_NITS);
        assert_eq!(read.sdr_white(), FALLBACK_SDR_WHITE_NITS);
        let dim = Monitor {
            hdr: true,
            peak_nits: Some(200.0),
            sdr_white_nits: Some(300.0),
        };
        assert_eq!(dim.peak(), 300.0);
    }
}
