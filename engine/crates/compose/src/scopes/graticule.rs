//! The graticules of the scopes: the scales of levels, the targets and the skin tone line of the
//! vectorscope, the spectral locus and the triangles of gamuts of the chromaticity diagram. They
//! are lines and labels in the pixels of a scope's picture, which an interface draws over it (as
//! vectors, at any size), and which the engine also draws into the picture, as brightly as the
//! look's `graticule` says, so that a picture of a scope reads alone (an agent shown it, say).

use columbina_model::Gamma;

use super::reference::{CIE_X, CIE_Y};
use super::{
    Chromaticity, Histogram, Kind, Parade, ScaleStyle, ScopeSettings, Targets, Vectorscope,
    Waveform,
};

/// The angle of the skin tone line (the In-phase axis) of the vectorscope, counterclockwise from
/// the axis of Cb, as vectorscopes draw it.
pub const SKIN_TONE_DEGREES: f64 = 123.0;

/// A line of a graticule, from one point to another, in pixels of the scope's picture (from its
/// top left corner).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub from: [f32; 2],
    pub to: [f32; 2],
    pub colour: [u8; 3],
}

/// A label of a graticule: its text, written from `at` (its left, at the middle of its height),
/// in pixels of the scope's picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub at: [f32; 2],
    pub text: String,
    pub colour: [u8; 3],
}

/// The graticule of a scope, in the pixels of its picture, `width` by `height`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Graticule {
    pub width: u32,
    pub height: u32,
    pub lines: Vec<Line>,
    pub labels: Vec<Label>,
}

/// A mark of a scale: the signal it is at, from 0 to 1, and what it is called.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub signal: f64,
    pub label: String,
}

/// The colour of the scales.
const SCALE: [u8; 3] = [168, 150, 104];
/// The colour of the faint lines of the chromaticity diagram.
const FAINT: [u8; 3] = [72, 72, 72];
/// The colour of the spectral locus.
const LOCUS: [u8; 3] = [190, 190, 190];
/// The colour of the skin tone line.
const SKIN: [u8; 3] = [222, 160, 120];
/// The colours of the triangles of gamuts, the first for the space of the signal.
const TRIANGLES: [[u8; 3]; 4] = [
    [240, 240, 240],
    [255, 186, 72],
    [80, 200, 255],
    [255, 110, 200],
];

/// The constants of SMPTE ST 2084.
const PQ_M1: f64 = 2610.0 / 16384.0;
const PQ_M2: f64 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f64 = 3424.0 / 4096.0;
const PQ_C2: f64 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f64 = 2392.0 / 4096.0 * 32.0;

/// The constants of ARIB STD-B67 (BT.2100 HLG).
const HLG_A: f64 = 0.178_832_77;
const HLG_B: f64 = 0.284_668_92;
const HLG_C: f64 = 0.559_910_73;
/// The display that HLG is measured on: 1000 nits, its system gamma 1.2 (BT.2100).
const HLG_PEAK: f64 = 1000.0;
const HLG_GAMMA: f64 = 1.2;

/// The nits of a signal of PQ.
#[must_use]
pub fn pq_nits(signal: f64) -> f64 {
    let power = signal.clamp(0.0, 1.0).powf(1.0 / PQ_M2);
    10_000.0 * ((power - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * power)).powf(1.0 / PQ_M1)
}

/// The signal of PQ of `nits`.
#[must_use]
pub fn pq_signal(nits: f64) -> f64 {
    let power = (nits / 10_000.0).clamp(0.0, 1.0).powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * power) / (1.0 + PQ_C3 * power)).powf(PQ_M2)
}

/// The linear light of the scene, from 0 to 1, of a signal of HLG.
#[must_use]
pub fn hlg_scene(signal: f64) -> f64 {
    let signal = signal.clamp(0.0, 1.0);
    if signal <= 0.5 {
        signal * signal / 3.0
    } else {
        (((signal - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

/// The nits of a grey of a signal of HLG, on a display of 1000 nits.
#[must_use]
pub fn hlg_nits(signal: f64) -> f64 {
    HLG_PEAK * hlg_scene(signal).powf(HLG_GAMMA)
}

/// The signal of HLG of a grey of `nits`, on a display of 1000 nits.
#[must_use]
pub fn hlg_signal(nits: f64) -> f64 {
    let scene = (nits / HLG_PEAK).clamp(0.0, 1.0).powf(1.0 / HLG_GAMMA);
    if scene <= 1.0 / 12.0 {
        (3.0 * scene).sqrt()
    } else {
        HLG_A * (12.0 * scene - HLG_B).ln() + HLG_C
    }
}

/// A signal from 0 to 1 in the units of a scale: a code of 10 bits, a percent, or nits (of HLG
/// where the transfer is HLG, of PQ otherwise, as `DaVinci Resolve`'s HDR scale is whatever
/// the output is, p3009).
#[must_use]
pub fn scale_value(style: ScaleStyle, transfer: Gamma, signal: f64) -> f64 {
    match style {
        ScaleStyle::TenBit => signal * 1023.0,
        ScaleStyle::Percent => signal * 100.0,
        ScaleStyle::Nits if transfer == Gamma::Hlg => hlg_nits(signal),
        ScaleStyle::Nits => pq_nits(signal),
    }
}

/// The marks of a scale: every 128 codes of 10 bits (and 1023), every ten percent, or the nits
/// of 0, 1, 10, 100, 203 (the white of graphics, BT.2408), 1000 and, on PQ, 4000 and 10 000.
#[must_use]
pub fn marks(style: ScaleStyle, transfer: Gamma) -> Vec<Mark> {
    match style {
        ScaleStyle::TenBit => (0..=8u32)
            .map(|step| {
                let code = (step * 128).min(1023);
                Mark {
                    signal: f64::from(code) / 1023.0,
                    label: code.to_string(),
                }
            })
            .collect(),
        ScaleStyle::Percent => (0..=10u32)
            .map(|step| Mark {
                signal: f64::from(step) / 10.0,
                label: (step * 10).to_string(),
            })
            .collect(),
        ScaleStyle::Nits => {
            let hlg = transfer == Gamma::Hlg;
            let nits: &[f64] = if hlg {
                &[0.0, 1.0, 10.0, 100.0, 203.0, 1000.0]
            } else {
                &[0.0, 1.0, 10.0, 100.0, 203.0, 1000.0, 4000.0, 10_000.0]
            };
            nits.iter()
                .map(|&value| Mark {
                    signal: if hlg {
                        hlg_signal(value)
                    } else {
                        pq_signal(value)
                    },
                    label: format!("{value}"),
                })
                .collect()
        }
    }
}

/// Where the colour bars of `amplitude` (0.75 or 1) fall on the vectorscope: the name of each
/// hue (red, magenta, blue, cyan, green, yellow), and its Cb and Cr, by the luma coefficients.
#[must_use]
pub fn targets(luma: [f32; 3], amplitude: f64) -> [(&'static str, [f64; 2]); 6] {
    let [kr, kg, kb] = luma.map(f64::from);
    let at = |r: f64, g: f64, b: f64| {
        let y = kr * r + kg * g + kb * b;
        [
            amplitude * (b - y) / (2.0 * (1.0 - kb)),
            amplitude * (r - y) / (2.0 * (1.0 - kr)),
        ]
    };
    [
        ("R", at(1.0, 0.0, 0.0)),
        ("Mg", at(1.0, 0.0, 1.0)),
        ("B", at(0.0, 0.0, 1.0)),
        ("Cy", at(0.0, 1.0, 1.0)),
        ("G", at(0.0, 1.0, 0.0)),
        ("Yl", at(1.0, 1.0, 0.0)),
    ]
}

/// The spectral locus of CIE 1931 xy (the standard observer of 2°), from 380 to 700 nm by 5 nm,
/// as the tables of the CIE have it to four places.
const LOCUS_XY: [[f64; 2]; 65] = [
    [0.1741, 0.0050],
    [0.1740, 0.0050],
    [0.1738, 0.0049],
    [0.1736, 0.0049],
    [0.1733, 0.0048],
    [0.1730, 0.0048],
    [0.1726, 0.0048],
    [0.1721, 0.0048],
    [0.1714, 0.0051],
    [0.1703, 0.0058],
    [0.1689, 0.0069],
    [0.1669, 0.0086],
    [0.1644, 0.0109],
    [0.1611, 0.0138],
    [0.1566, 0.0177],
    [0.1510, 0.0227],
    [0.1440, 0.0297],
    [0.1355, 0.0399],
    [0.1241, 0.0578],
    [0.1096, 0.0868],
    [0.0913, 0.1327],
    [0.0687, 0.2007],
    [0.0454, 0.2950],
    [0.0235, 0.4127],
    [0.0082, 0.5384],
    [0.0039, 0.6548],
    [0.0139, 0.7502],
    [0.0389, 0.8120],
    [0.0743, 0.8338],
    [0.1142, 0.8262],
    [0.1547, 0.8059],
    [0.1929, 0.7816],
    [0.2296, 0.7543],
    [0.2658, 0.7243],
    [0.3016, 0.6923],
    [0.3373, 0.6589],
    [0.3731, 0.6245],
    [0.4087, 0.5896],
    [0.4441, 0.5547],
    [0.4788, 0.5202],
    [0.5125, 0.4866],
    [0.5448, 0.4544],
    [0.5752, 0.4242],
    [0.6029, 0.3965],
    [0.6270, 0.3725],
    [0.6482, 0.3514],
    [0.6658, 0.3340],
    [0.6801, 0.3197],
    [0.6915, 0.3083],
    [0.7006, 0.2993],
    [0.7079, 0.2920],
    [0.7140, 0.2859],
    [0.7190, 0.2809],
    [0.7230, 0.2770],
    [0.7260, 0.2740],
    [0.7283, 0.2717],
    [0.7300, 0.2700],
    [0.7311, 0.2689],
    [0.7320, 0.2680],
    [0.7327, 0.2673],
    [0.7334, 0.2666],
    [0.7340, 0.2660],
    [0.7344, 0.2656],
    [0.7346, 0.2654],
    [0.7347, 0.2653],
];

/// The spectral locus of CIE 1931 xy, from 380 to 700 nm by 5 nm.
#[must_use]
pub fn spectral_locus() -> Vec<[f64; 2]> {
    LOCUS_XY.to_vec()
}

/// The graticule of a scope, as its settings say: what [`super::Scope::graticule`] gives once
/// the scope is made on the card, worked out without one.
#[must_use]
pub fn graticule(settings: &ScopeSettings) -> Graticule {
    let (width, height) = settings.kind.picture_size();
    let mut graticule = Graticule {
        width,
        height,
        ..Graticule::default()
    };
    let transfer = settings.signal.transfer;
    match &settings.kind {
        Kind::Waveform(Waveform { scale, .. }) => {
            levels(&mut graticule, *scale, transfer);
        }
        Kind::Parade(Parade {
            width: panel,
            scale,
            ..
        }) => {
            levels(&mut graticule, *scale, transfer);
            #[allow(clippy::cast_precision_loss, reason = "a size of at most 4096")]
            for panel in [*panel, 2 * *panel].map(|x| x as f32) {
                graticule.lines.push(Line {
                    from: [panel, 0.0],
                    to: [panel, height as f32],
                    colour: SCALE,
                });
            }
        }
        Kind::Histogram(histogram) => histogram_scale(&mut graticule, histogram, transfer),
        Kind::Vectorscope(scope) => vectorscope(&mut graticule, scope, settings.signal.luma),
        Kind::Chromaticity(diagram) => chromaticity(&mut graticule, diagram),
    }
    graticule
}

/// The lines of a scale across a waveform or a parade, the levels up its height.
#[allow(clippy::cast_precision_loss, reason = "sizes of at most 4096")]
fn levels(graticule: &mut Graticule, style: ScaleStyle, transfer: Gamma) {
    let (width, height) = (graticule.width as f32, graticule.height as f32);
    for mark in marks(style, transfer) {
        #[allow(clippy::cast_possible_truncation, reason = "a place in a picture")]
        let y = (height - 1.0) * (1.0 - mark.signal as f32) + 0.5;
        graticule.lines.push(Line {
            from: [0.0, y],
            to: [width, y],
            colour: SCALE,
        });
        graticule.labels.push(Label {
            at: [2.0, y],
            text: mark.label,
            colour: SCALE,
        });
    }
}

/// The lines of a scale up a histogram, the levels across its width, and between its strips.
#[allow(clippy::cast_precision_loss, reason = "sizes of at most 4096")]
fn histogram_scale(graticule: &mut Graticule, histogram: &Histogram, transfer: Gamma) {
    let (width, height) = (graticule.width as f32, graticule.height as f32);
    for mark in marks(histogram.scale, transfer) {
        #[allow(clippy::cast_possible_truncation, reason = "a place in a picture")]
        let x = (width - 1.0) * mark.signal as f32 + 0.5;
        graticule.lines.push(Line {
            from: [x, 0.0],
            to: [x, height],
            colour: SCALE,
        });
        graticule.labels.push(Label {
            at: [x + 2.0, height - 6.0],
            text: mark.label,
            colour: SCALE,
        });
    }
    let strips = histogram.channels.count();
    let strip = graticule.height / strips;
    for between in 1..strips {
        let y = (between * strip) as f32;
        graticule.lines.push(Line {
            from: [0.0, y],
            to: [width, y],
            colour: SCALE,
        });
    }
}

/// The circle, the cross at the centre, the targets of the bars and the skin tone line.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "places in a picture of at most 1024"
)]
fn vectorscope(graticule: &mut Graticule, scope: &Vectorscope, luma: [f32; 3]) {
    let size = f64::from(scope.size);
    let centre = size / 2.0;
    let radius = centre - 0.5;
    let zoom = f64::from(scope.zoom);
    let point = |x: f64, y: f64| [x as f32, y as f32];
    let mut line = |from: [f32; 2], to: [f32; 2], colour: [u8; 3]| {
        graticule.lines.push(Line { from, to, colour });
    };
    // The circle, of 96 sides.
    let around = |step: u32| {
        let angle = f64::from(step) * std::f64::consts::TAU / 96.0;
        point(centre + radius * angle.cos(), centre - radius * angle.sin())
    };
    for step in 0..96 {
        line(around(step), around(step + 1), SCALE);
    }
    let arm = size * 0.04;
    line(
        point(centre - arm, centre),
        point(centre + arm, centre),
        SCALE,
    );
    line(
        point(centre, centre - arm),
        point(centre, centre + arm),
        SCALE,
    );
    if scope.skin_tone {
        let angle = SKIN_TONE_DEGREES.to_radians();
        line(
            point(centre, centre),
            point(centre + radius * angle.cos(), centre - radius * angle.sin()),
            SKIN,
        );
    }
    let amplitudes: &[f64] = match scope.targets {
        Targets::Off => &[],
        Targets::Percent75 => &[0.75],
        Targets::Percent100 => &[1.0],
        Targets::Both => &[0.75, 1.0],
    };
    let half = (size * 0.02).max(2.0);
    let mut labels = Vec::new();
    for &amplitude in amplitudes {
        for (name, [cb, cr]) in targets(luma, amplitude) {
            let (x, y) = ((cb * zoom + 0.5) * size, (0.5 - cr * zoom) * size);
            if x < 0.0 || y < 0.0 || x > size || y > size {
                continue;
            }
            let corners = [
                point(x - half, y - half),
                point(x + half, y - half),
                point(x + half, y + half),
                point(x - half, y + half),
            ];
            for side in 0..4 {
                line(corners[side], corners[(side + 1) % 4], SCALE);
            }
            // The name, out beyond the outermost target of its hue.
            let (dx, dy) = (x - centre, y - centre);
            let length = dx.hypot(dy).max(1.0);
            let out = 3.0 * half;
            labels.retain(|(each, _): &(&str, [f32; 2])| *each != name);
            labels.push((
                name,
                point(x + dx / length * out - half, y + dy / length * out),
            ));
        }
    }
    graticule
        .labels
        .extend(labels.into_iter().map(|(name, at)| Label {
            at,
            text: name.to_owned(),
            colour: SCALE,
        }));
}

/// The lines every 0.1, the spectral locus, the triangles of the gamuts and their whites.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "places in a picture of at most 1024"
)]
fn chromaticity(graticule: &mut Graticule, diagram: &Chromaticity) {
    let (width, height) = (f64::from(diagram.width), f64::from(diagram.height));
    let at = |[x, y]: [f64; 2]| {
        [
            (x / CIE_X * width) as f32,
            ((1.0 - y / CIE_Y) * height) as f32,
        ]
    };
    for tenth in 1..8u32 {
        let x = f64::from(tenth) / 10.0;
        graticule.lines.push(Line {
            from: at([x, 0.0]),
            to: at([x, CIE_Y]),
            colour: FAINT,
        });
        let [left, bottom] = at([x, 0.0]);
        graticule.labels.push(Label {
            at: [left + 2.0, bottom - 6.0],
            text: format!("0.{tenth}"),
            colour: FAINT,
        });
    }
    for tenth in 1..9u32 {
        let y = f64::from(tenth) / 10.0;
        graticule.lines.push(Line {
            from: at([0.0, y]),
            to: at([CIE_X, y]),
            colour: FAINT,
        });
        let [_, top] = at([0.0, y]);
        graticule.labels.push(Label {
            at: [2.0, top - 6.0],
            text: format!("0.{tenth}"),
            colour: FAINT,
        });
    }
    let locus = spectral_locus();
    for (from, to) in locus.iter().zip(locus.iter().cycle().skip(1)) {
        graticule.lines.push(Line {
            from: at(*from),
            to: at(*to),
            colour: LOCUS,
        });
    }
    for (triangle, colour) in diagram.gamuts.iter().zip(TRIANGLES.iter().cycle()) {
        let primaries = &triangle.primaries;
        let corners = [primaries.red, primaries.green, primaries.blue];
        for side in 0..3 {
            graticule.lines.push(Line {
                from: at(corners[side]),
                to: at(corners[(side + 1) % 3]),
                colour: *colour,
            });
        }
        let [x, y] = at(primaries.white);
        let arm = (width * 0.012) as f32;
        graticule.lines.push(Line {
            from: [x - arm, y],
            to: [x + arm, y],
            colour: *colour,
        });
        graticule.lines.push(Line {
            from: [x, y - arm],
            to: [x, y + arm],
            colour: *colour,
        });
        let [rx, ry] = at(primaries.red);
        graticule.labels.push(Label {
            at: [rx + 4.0, ry],
            text: triangle.name.clone(),
            colour: *colour,
        });
    }
}

/// How far a line is drawn on each side of its middle, in pixels.
const HALF_WIDTH: f32 = 0.6;

/// The lines of a graticule drawn into a picture of RGBA of 8 bits, not premultiplied, on
/// nothing (transparent): each pixel as covered as the nearest line covers it, in its colour.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "places in a picture of at most 4096, and coverage from 0 to 1"
)]
pub(crate) fn rasterize(graticule: &Graticule) -> Vec<u8> {
    let (width, height) = (graticule.width as usize, graticule.height as usize);
    let mut rgba = vec![0u8; width * height * 4];
    let reach = HALF_WIDTH + 1.0;
    for line in &graticule.lines {
        let [ax, ay] = line.from;
        let [bx, by] = line.to;
        let left = (ax.min(bx) - reach).floor().max(0.0) as usize;
        let top = (ay.min(by) - reach).floor().max(0.0) as usize;
        let right = ((ax.max(bx) + reach).ceil().max(0.0) as usize).min(width);
        let bottom = ((ay.max(by) + reach).ceil().max(0.0) as usize).min(height);
        let (dx, dy) = (bx - ax, by - ay);
        let length = dx * dx + dy * dy;
        for y in top..bottom {
            for x in left..right {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let along = if length > 0.0 {
                    (((px - ax) * dx + (py - ay) * dy) / length).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let distance = (px - (ax + along * dx)).hypot(py - (ay + along * dy));
                let cover = (HALF_WIDTH + 0.5 - distance).clamp(0.0, 1.0);
                let alpha = (cover * 255.0).round() as u8;
                let at = (y * width + x) * 4;
                if alpha > rgba[at + 3] {
                    rgba[at..at + 3].copy_from_slice(&line.colour);
                    rgba[at + 3] = alpha;
                }
            }
        }
    }
    rgba
}
