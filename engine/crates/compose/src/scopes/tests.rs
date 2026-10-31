//! The scopes, held to the processor's counts, to frames whose scopes are known (colour bars, a
//! grey ramp, one colour), and to their own pictures; and how long they take.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Instant;

use columbina_model::Gamma;

use super::graticule::{self, hlg_signal, marks, pq_nits, pq_signal, spectral_locus, targets};
use super::reference::{self, Frame};
use super::*;
use crate::{gpu_for_tests, Gpu};

/// A generator of numbers that are the same each run (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// From 0 to 1.
    #[allow(clippy::cast_precision_loss, reason = "24 bits of a number")]
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    #[allow(clippy::cast_possible_truncation, reason = "a small count")]
    fn below(&mut self, count: u32) -> u32 {
        (self.next() % u64::from(count)) as u32
    }
}

/// The bits of a half float nearest a float (ties to even).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "the fields of floats"
)]
fn half_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x7f_ffff;
    if exponent == 0xff {
        return sign | 0x7c00 | if mantissa == 0 { 0 } else { 0x200 };
    }
    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1f {
        return sign | 0x7c00;
    }
    let (kept, shift) = if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        (mantissa | 0x80_0000, (14 - half_exponent) as u32)
    } else {
        ((half_exponent as u32) << 23 | mantissa, 13)
    };
    let rounded = kept >> shift;
    let rest = kept & ((1 << shift) - 1);
    let middle = 1 << (shift - 1);
    let rounded = if rest > middle || (rest == middle && rounded & 1 == 1) {
        rounded + 1
    } else {
        rounded
    };
    sign | rounded as u16
}

fn half_value(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    sign * match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

/// The value a half float keeps of a float.
fn half(value: f32) -> f32 {
    half_value(half_bits(value))
}

#[test]
fn half_floats_are_rounded_as_the_card_rounds_them() {
    for value in [0.0, 1.0, 0.5, 65504.0, -2.0, 6.0e-8, 0.75, 1.0 / 3.0] {
        assert_eq!(half_value(half_bits(value)), half(value));
    }
    assert_eq!(half(1.0 + 1.0 / 2048.0), 1.0, "ties go to even");
    assert_eq!(half(1.0 + 3.0 / 2048.0), 1.0 + 2.0 / 1024.0);
    assert_eq!(half(2f32.powi(-24)), 2f32.powi(-24));
    assert_eq!(half(70000.0), f32::INFINITY);
}

/// A frame of pixels that are half floats already, as the card holds them.
struct Picture {
    width: u32,
    height: u32,
    pixels: Vec<[f32; 4]>,
}

impl Picture {
    fn new(width: u32, height: u32, mut colour: impl FnMut(u32, u32) -> [f32; 3]) -> Picture {
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = colour(x, y).map(half);
                pixels.push([r, g, b, 1.0]);
            }
        }
        Picture {
            width,
            height,
            pixels,
        }
    }

    fn frame(&self) -> Frame<'_> {
        Frame {
            width: self.width,
            height: self.height,
            pixels: &self.pixels,
        }
    }

    /// The frame on the card, of half floats as the compositor's.
    fn texture(&self, gpu: &Gpu) -> wgpu::Texture {
        let size = wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };
        let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("scope test frame"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::WORKING_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bytes: Vec<u8> = self
            .pixels
            .iter()
            .flatten()
            .flat_map(|&value| half_bits(value).to_le_bytes())
            .collect();
        gpu.queue().write_texture(
            texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 8),
                rows_per_image: Some(self.height),
            },
            size,
        );
        texture
    }
}

/// A frame of blocks of colours, of noise, and of a few values far beyond 0…1.
fn busy(rng: &mut Rng, width: u32, height: u32) -> Picture {
    let blocks: Vec<[f32; 3]> = (0..64)
        .map(|_| [rng.unit(), rng.unit(), rng.unit()].map(|value| value * 1.2 - 0.1))
        .collect();
    Picture::new(width, height, |x, y| {
        match rng.below(16) {
            // Noise.
            0..=4 => [rng.unit(), rng.unit(), rng.unit()].map(|value| value * 1.4 - 0.2),
            // Far beyond.
            5 => [
                rng.unit() * 40.0 - 20.0,
                rng.unit() * 3.0,
                -rng.unit() * 5.0,
            ],
            // Blocks, so that runs of one bin are counted.
            _ => blocks[((x / 8 + y / 8 * 7) % 64) as usize],
        }
    })
}

/// Every kind of scope, with settings that reach each path of the counting.
fn every_kind() -> Vec<ScopeSettings> {
    let hdr = Signal {
        luma: REC2020_LUMA,
        transfer: Gamma::St2084,
    };
    let mut all = vec![
        ScopeSettings::new(Kind::Waveform(Waveform::default())),
        ScopeSettings::new(Kind::Waveform(Waveform {
            width: 300,
            levels: 1024,
            channels: WaveformChannels::Rgb,
            scale: ScaleStyle::Percent,
        })),
        ScopeSettings::new(Kind::Parade(Parade::default())),
        ScopeSettings::new(Kind::Vectorscope(Vectorscope::default())),
        ScopeSettings::new(Kind::Vectorscope(Vectorscope {
            size: 300,
            zoom: 2.5,
            tones: Tones::Mid,
            targets: Targets::Both,
            ..Vectorscope::default()
        })),
        ScopeSettings::new(Kind::Vectorscope(Vectorscope {
            tones: Tones::Low,
            ..Vectorscope::default()
        })),
        ScopeSettings::new(Kind::Vectorscope(Vectorscope {
            tones: Tones::High,
            zoom: 8.0,
            ..Vectorscope::default()
        })),
        ScopeSettings::new(Kind::Histogram(Histogram::default())),
        ScopeSettings::new(Kind::Histogram(Histogram {
            bins: 1024,
            height: 100,
            channels: HistogramChannels::Yrgb,
            scale: ScaleStyle::Nits,
        })),
        ScopeSettings::new(Kind::Chromaticity(Chromaticity::default())),
    ];
    for (luma, transfer, space) in [
        (REC2020_LUMA, Gamma::St2084, Primaries::REC2020),
        (REC2020_LUMA, Gamma::Hlg, Primaries::REC2020),
        (REC709_LUMA, Gamma::Srgb, Primaries::P3_D65),
        (REC601_LUMA, Gamma::Linear, Primaries::REC601_NTSC),
    ] {
        all.push(ScopeSettings {
            signal: Signal { luma, transfer },
            ..ScopeSettings::new(Kind::Chromaticity(Chromaticity {
                width: 400,
                height: 450,
                space,
                gamuts: vec![
                    Triangle {
                        name: "Rec.2020".into(),
                        primaries: Primaries::REC2020,
                    },
                    Triangle {
                        name: "P3-D65".into(),
                        primaries: Primaries::P3_D65,
                    },
                ],
            }))
        });
    }
    all.push(ScopeSettings {
        signal: hdr,
        ..ScopeSettings::new(Kind::Waveform(Waveform {
            scale: ScaleStyle::Nits,
            ..Waveform::default()
        }))
    });
    all
}

/// Count `picture` on the card and read the counts back.
fn card_counts(
    scopes: &Scopes,
    texture: &wgpu::Texture,
    settings: &ScopeSettings,
    sampling: Sampling,
) -> Vec<u32> {
    let scope = scopes.scope(settings.clone()).unwrap();
    scopes.draw(texture, &scope, sampling).unwrap();
    scopes.read_counts(&scope).unwrap()
}

fn no_errors(gpu: &Gpu) {
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn the_counts_of_the_card_are_the_processors_exactly() {
    let Some(gpu) = gpu_for_tests() else { return };
    let scopes = Scopes::new(Arc::clone(&gpu));
    let mut rng = Rng(0x5eed_5c09e);
    // Odd sizes; one narrower than the scopes are wide; one as large as a frame of HD.
    for (width, height) in [(333, 187), (64, 41), (1001, 9), (1280, 720)] {
        let picture = busy(&mut rng, width, height);
        let texture = picture.texture(&gpu);
        for settings in every_kind() {
            for sampling in [Sampling::Full, Sampling::Half, Sampling::Quarter] {
                let card = card_counts(&scopes, &texture, &settings, sampling);
                let processor = reference::counts(
                    &picture.frame(),
                    &settings.kind,
                    &settings.signal,
                    sampling.step(),
                );
                assert_eq!(card.len(), processor.len());
                let differ = card.iter().zip(&processor).filter(|(a, b)| a != b).count();
                assert_eq!(
                    differ, 0,
                    "{width}×{height}, {sampling:?}, {settings:?}: {differ} counts differ"
                );
            }
        }
    }
    no_errors(&gpu);
}

#[test]
fn every_pixel_counted_is_in_a_waveform_or_a_histogram() {
    let mut rng = Rng(7);
    let picture = busy(&mut rng, 97, 61);
    for sampling in [Sampling::Full, Sampling::Half, Sampling::Quarter] {
        let step = sampling.step();
        let sampled = u64::from(97u32.div_ceil(step) * 61u32.div_ceil(step));
        let waveform = reference::counts(
            &picture.frame(),
            &Kind::Waveform(Waveform::default()),
            &Signal::default(),
            step,
        );
        let total: u64 = waveform.iter().map(|&count| u64::from(count)).sum();
        assert_eq!(total, sampled);
        let histogram = reference::counts(
            &picture.frame(),
            &Kind::Histogram(Histogram::default()),
            &Signal::default(),
            step,
        );
        let red: u64 = histogram[..256].iter().map(|&count| u64::from(count)).sum();
        assert_eq!(red, sampled);
    }
}

/// Colour bars of `amplitude`: white, yellow, cyan, green, magenta, red, blue, black.
fn bars(amplitude: f32) -> Picture {
    let colours = [
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [0.0, 1.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0],
    ];
    Picture::new(800, 60, |x, _| {
        colours[(x / 100) as usize].map(|channel: f32| channel * amplitude)
    })
}

/// The bins of a vectorscope that are counted, as (column, row).
fn lit(counts: &[u32], size: u32) -> Vec<(u32, u32)> {
    (0..)
        .zip(counts)
        .filter(|(_, &count)| count > 0)
        .map(|(bin, _): (u32, _)| (bin % size, bin / size))
        .collect()
}

#[test]
fn colour_bars_land_on_the_targets_of_the_vectorscope() {
    let Some(gpu) = gpu_for_tests() else { return };
    let scopes = Scopes::new(Arc::clone(&gpu));
    for (amplitude, luma) in [
        (0.75, REC709_LUMA),
        (1.0, REC709_LUMA),
        (0.75, REC2020_LUMA),
        (0.75, REC601_LUMA),
    ] {
        let picture = bars(amplitude);
        let texture = picture.texture(&gpu);
        let size = 256;
        let settings = ScopeSettings {
            signal: Signal {
                luma,
                ..Signal::default()
            },
            ..ScopeSettings::new(Kind::Vectorscope(Vectorscope::default()))
        };
        let counts = card_counts(&scopes, &texture, &settings, Sampling::Full);
        let bins = lit(&counts, size);
        // The six hues, and the centre (white and black).
        assert_eq!(bins.len(), 7, "{amplitude} {luma:?}: {bins:?}");
        let centre = size / 2;
        assert!(bins.contains(&(centre, centre - 1)) || bins.contains(&(centre, centre)));
        for (name, [cb, cr]) in targets(luma, f64::from(amplitude)) {
            let x = (cb + 0.5) * f64::from(size);
            let y = (0.5 - cr) * f64::from(size);
            let near = bins.iter().find(|&&(column, row)| {
                (f64::from(column) + 0.5 - x).abs() <= 1.0
                    && (f64::from(row) + 0.5 - y).abs() <= 1.0
            });
            let Some(&(column, row)) = near else {
                panic!("{name} of {amplitude} {luma:?} at {x}, {y}: {bins:?}");
            };
            // Each bar is 100 × 60 pixels, all in one bin.
            assert_eq!(counts[(row * size + column) as usize], 6000, "{name}");
        }
    }
    no_errors(&gpu);
}

#[test]
fn a_grey_ramp_is_a_straight_line_on_the_waveform() {
    let Some(gpu) = gpu_for_tests() else { return };
    let scopes = Scopes::new(Arc::clone(&gpu));
    #[allow(clippy::cast_precision_loss, reason = "a small count")]
    let picture = Picture::new(512, 90, |x, _| [x as f32 / 511.0; 3]);
    let texture = picture.texture(&gpu);
    let levels = 1024;
    for channels in [WaveformChannels::Luma, WaveformChannels::Rgb] {
        let settings = ScopeSettings::new(Kind::Waveform(Waveform {
            width: 512,
            levels,
            channels,
            scale: ScaleStyle::TenBit,
        }));
        let counts = card_counts(&scopes, &texture, &settings, Sampling::Full);
        let planes = counts.len() / (512 * levels as usize);
        for plane in 0..planes {
            for column in 0..512usize {
                let lit: Vec<usize> = (0..levels as usize)
                    .filter(|level| counts[(plane * levels as usize + level) * 512 + column] > 0)
                    .collect();
                // One level, of every pixel of the column, on the line from 0 to 1023.
                assert_eq!(lit.len(), 1, "{channels:?} column {column}: {lit:?}");
                assert_eq!(
                    counts[(plane * levels as usize + lit[0]) * 512 + column],
                    90
                );
                #[allow(clippy::cast_precision_loss, reason = "small counts")]
                let line = column as f64 * 1023.0 / 511.0;
                #[allow(clippy::cast_precision_loss, reason = "small counts")]
                let off = (lit[0] as f64 - line).abs();
                assert!(off <= 1.0, "column {column} at {}, not {line}", lit[0]);
            }
        }
    }
    no_errors(&gpu);
}

#[test]
fn one_colour_is_a_spike_on_the_histogram_and_in_the_statistics() {
    let Some(gpu) = gpu_for_tests() else { return };
    let scopes = Scopes::new(Arc::clone(&gpu));
    let colour = [0.25, 0.5, 0.75];
    let picture = Picture::new(200, 100, |_, _| colour);
    let texture = picture.texture(&gpu);
    let settings = ScopeSettings::new(Kind::Histogram(Histogram::default()));
    let counts = card_counts(&scopes, &texture, &settings, Sampling::Full);
    let [kr, kg, kb] = REC709_LUMA.map(f64::from);
    let luma = kr * 0.25 + kg * 0.5 + kb * 0.75;
    for (channel, value) in [0.25, 0.5, 0.75, luma].into_iter().enumerate() {
        let spike: Vec<(usize, u32)> = counts[channel * 256..(channel + 1) * 256]
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, count)| *count > 0)
            .collect();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bin = (value * 255.0).round() as usize;
        assert_eq!(spike, vec![(bin, 20_000)], "channel {channel}");
        assert_eq!(counts[4 * 256 + channel], 20_000, "the highest count");
    }
    let statistics = scopes
        .statistics(&texture, &Signal::default(), 256)
        .unwrap();
    assert_eq!(statistics.samples, 20_000);
    for (channel, value) in [
        (statistics.red, 0.25),
        (statistics.green, 0.5),
        (statistics.blue, 0.75),
    ] {
        assert_eq!((channel.min, channel.max), (value, value));
        assert!((channel.mean - f64::from(value)).abs() < 1e-6);
    }
    assert!((statistics.luma.mean - luma).abs() < 1e-5);
    let chroma = statistics.chroma;
    let cb = (0.75 - luma) / (2.0 * (1.0 - kb));
    let cr = (0.25 - luma) / (2.0 * (1.0 - kr));
    assert!((chroma.centroid[0] - cb).abs() < 1e-5 && (chroma.centroid[1] - cr).abs() < 1e-5);
    assert!(chroma.radius < 1e-3, "{chroma:?}");
    assert!((chroma.saturation - cb.hypot(cr)).abs() < 1e-5);
    let histogram = &statistics.histogram;
    for share in [0.01, 0.5, 0.99] {
        assert!((histogram.percentile(Bins::GREEN, share) - 0.5).abs() < 0.5 / 255.0);
    }
    no_errors(&gpu);
}

#[test]
fn the_statistics_of_the_card_are_the_processors() {
    let Some(gpu) = gpu_for_tests() else { return };
    let scopes = Scopes::new(Arc::clone(&gpu));
    let mut rng = Rng(99);
    for (width, height) in [(777, 333), (16, 1), (1920, 1080)] {
        let picture = busy(&mut rng, width, height);
        let texture = picture.texture(&gpu);
        for (signal, bins) in [
            (Signal::default(), 256),
            (
                Signal {
                    luma: REC2020_LUMA,
                    transfer: Gamma::St2084,
                },
                1024,
            ),
        ] {
            let card = scopes.statistics(&texture, &signal, bins).unwrap();
            let processor = reference::statistics(&picture.frame(), &signal, bins);
            assert_eq!(card.samples, processor.samples);
            assert_eq!(card.histogram, processor.histogram, "{width}×{height}");
            let pairs = [
                (card.red, processor.red),
                (card.green, processor.green),
                (card.blue, processor.blue),
            ];
            for (on_card, on_processor) in pairs {
                assert_eq!(
                    (on_card.min, on_card.max),
                    (on_processor.min, on_processor.max)
                );
                assert!((on_card.mean - on_processor.mean).abs() < 1e-5);
            }
            // Luma is a sum of floats on the card: within a rounding of its extremes.
            let luma = (card.luma, processor.luma);
            assert!((luma.0.min - luma.1.min).abs() <= 1e-5 * luma.1.min.abs().max(1.0));
            assert!((luma.0.max - luma.1.max).abs() <= 1e-5 * luma.1.max.abs().max(1.0));
            assert!((luma.0.mean - luma.1.mean).abs() < 1e-5);
            let (a, b) = (card.chroma, processor.chroma);
            for axis in 0..2 {
                assert!(
                    (a.centroid[axis] - b.centroid[axis]).abs() < 1e-5,
                    "{a:?} {b:?}"
                );
                assert!(
                    (a.spread[axis] - b.spread[axis]).abs() < 1e-4,
                    "{a:?} {b:?}"
                );
            }
            assert!((a.radius - b.radius).abs() < 1e-4);
            assert!((a.saturation - b.saturation).abs() < 1e-4);
            if b.centroid[0].hypot(b.centroid[1]) > 0.01 {
                assert!((a.hue - b.hue).abs() < 1e-2 || (a.hue - b.hue).abs() > 359.99);
            }
            // The same frame gives the same figures.
            assert_eq!(scopes.statistics(&texture, &signal, bins).unwrap(), card);
        }
    }
    no_errors(&gpu);
}

#[test]
fn pictures_are_the_same_each_time_and_show_what_was_counted() {
    let Some(gpu) = gpu_for_tests() else { return };
    let scopes = Scopes::new(Arc::clone(&gpu));
    let mut rng = Rng(3);
    let picture = busy(&mut rng, 640, 360);
    let texture = picture.texture(&gpu);
    for settings in every_kind() {
        for mapping in [Mapping::Log, Mapping::Gamma] {
            let settings = ScopeSettings {
                look: Look {
                    mapping,
                    ..settings.look
                },
                ..settings.clone()
            };
            let scope = scopes.scope(settings.clone()).unwrap();
            scopes.draw(&texture, &scope, Sampling::Full).unwrap();
            let first = scopes.read_image(&scope).unwrap();
            scopes.draw(&texture, &scope, Sampling::Full).unwrap();
            let again = scopes.read_image(&scope).unwrap();
            let other = scopes.scope(settings.clone()).unwrap();
            scopes.draw(&texture, &other, Sampling::Full).unwrap();
            let elsewhere = scopes.read_image(&other).unwrap();
            assert_eq!(first, again, "{settings:?}");
            assert_eq!(first, elsewhere, "{settings:?}");
            assert_eq!((first.width, first.height), scope.size());
            assert!(first.rgba.chunks(4).all(|pixel| pixel[3] == 255));
        }
    }
    no_errors(&gpu);
}

#[test]
fn a_picture_without_its_graticule_is_lit_where_there_are_counts() {
    let Some(gpu) = gpu_for_tests() else { return };
    let scopes = Scopes::new(Arc::clone(&gpu));
    // Wider than the waveform, so that every column of it has pixels.
    let picture = Picture::new(600, 40, |_, _| [0.6, 0.3, 0.2]);
    let texture = picture.texture(&gpu);
    let bare = Look {
        graticule: 0.0,
        ..Look::default()
    };
    // A vectorscope: one bin, white, the rest black.
    let scope = scopes
        .scope(ScopeSettings {
            look: bare,
            ..ScopeSettings::new(Kind::Vectorscope(Vectorscope::default()))
        })
        .unwrap();
    scopes.draw(&texture, &scope, Sampling::Full).unwrap();
    let image = scopes.read_image(&scope).unwrap();
    let counts = scopes.read_counts(&scope).unwrap();
    let lit_pixels: Vec<usize> = (0..counts.len())
        .filter(|&at| image.rgba[at * 4..at * 4 + 3] != [0, 0, 0])
        .collect();
    let counted: Vec<usize> = (0..counts.len()).filter(|&at| counts[at] > 0).collect();
    assert_eq!(lit_pixels, counted);
    assert_eq!(counted.len(), 1);
    let at = counted[0] * 4;
    assert!(image.rgba[at..at + 3].iter().all(|&channel| channel > 200));
    // A waveform of luma: one row lit across.
    let scope = scopes
        .scope(ScopeSettings {
            look: bare,
            ..ScopeSettings::new(Kind::Waveform(Waveform::default()))
        })
        .unwrap();
    scopes.draw(&texture, &scope, Sampling::Full).unwrap();
    let image = scopes.read_image(&scope).unwrap();
    let rows: Vec<u32> = (0..image.height)
        .filter(|row| {
            let at = (row * image.width * 4) as usize;
            image.rgba[at..at + 4 * image.width as usize]
                .chunks(4)
                .all(|pixel| pixel[..3] != [0, 0, 0])
        })
        .collect();
    let codes = reference::wide(picture.pixels[0]);
    let level = reference::level(reference::Coefficients::of(REC709_LUMA).luma(codes), 256);
    assert_eq!(rows, vec![255 - level]);
    let lit_count = image
        .rgba
        .chunks(4)
        .filter(|pixel| pixel[..3] != [0, 0, 0])
        .count();
    assert_eq!(lit_count, image.width as usize);
    // The graticule drawn in: its lines show where there are no counts.
    let scope = scopes
        .scope(ScopeSettings::new(Kind::Waveform(Waveform::default())))
        .unwrap();
    scopes.draw(&texture, &scope, Sampling::Full).unwrap();
    let image = scopes.read_image(&scope).unwrap();
    let top = &image.rgba[0..4 * image.width as usize];
    assert!(
        top.chunks(4).all(|pixel| pixel[..3] != [0, 0, 0]),
        "1023 is marked"
    );
    no_errors(&gpu);
}

#[test]
fn a_frame_the_compositor_made_is_measured() {
    let Some(gpu) = gpu_for_tests() else { return };
    let scopes = Scopes::new(Arc::clone(&gpu));
    let mut compositor = crate::Compositor::new(Arc::clone(&gpu));
    let target = crate::Target::preview(&gpu, 320, 180).unwrap();
    let scene = crate::Scene {
        width: 320,
        height: 180,
        layers: vec![crate::SceneLayer {
            source: crate::Source::Color([0.5, 0.25, 0.75, 1.0]),
            transform: crate::Transform::NONE,
            opacity: 1.0,
            color: crate::LayerColor::default(),
        }],
        ..crate::Scene::default()
    };
    compositor.render(&scene, &target).unwrap();
    let statistics = scopes
        .statistics(target.texture(), &Signal::default(), 256)
        .unwrap();
    assert_eq!(statistics.samples, 320 * 180);
    for (channel, value) in [
        (statistics.red, 0.5),
        (statistics.green, 0.25),
        (statistics.blue, 0.75),
    ] {
        assert!((channel.mean - value).abs() < 1e-3, "{channel:?}");
        assert!(channel.max - channel.min < 1e-3, "{channel:?}");
    }
    let scope = scopes
        .scope(ScopeSettings::new(Kind::Histogram(Histogram::default())))
        .unwrap();
    scopes
        .draw(target.texture(), &scope, Sampling::Half)
        .unwrap();
    let counts = scopes.read_counts(&scope).unwrap();
    // 0.5 is the edge between bins 127 and 128: what the compositor rounds falls on either side.
    assert_eq!(
        counts[127] + counts[128],
        160 * 90,
        "the red of 0.5 near bin 128"
    );
    no_errors(&gpu);
}

#[test]
fn settings_out_of_range_are_refused() {
    let bad = [
        Kind::Waveform(Waveform {
            levels: 2048,
            ..Waveform::default()
        }),
        Kind::Parade(Parade {
            width: 2000,
            ..Parade::default()
        }),
        Kind::Vectorscope(Vectorscope {
            size: 255,
            ..Vectorscope::default()
        }),
        Kind::Vectorscope(Vectorscope {
            zoom: 0.5,
            ..Vectorscope::default()
        }),
        Kind::Vectorscope(Vectorscope {
            low: 0.8,
            high: 0.2,
            ..Vectorscope::default()
        }),
        Kind::Histogram(Histogram {
            bins: 4096,
            ..Histogram::default()
        }),
        Kind::Chromaticity(Chromaticity {
            space: Primaries {
                red: [0.3, 0.3],
                green: [0.3, 0.3],
                blue: [0.3, 0.3],
                white: [0.3, 0.3],
            },
            ..Chromaticity::default()
        }),
    ];
    for kind in bad {
        assert!(
            ScopeSettings::new(kind.clone()).check().is_err(),
            "{kind:?}"
        );
    }
    let mut settings = ScopeSettings::new(Kind::Waveform(Waveform::default()));
    assert_eq!(settings.check(), Ok(()));
    settings.signal.luma = [0.7, 0.2, 0.1];
    assert!(settings.check().is_err());
    settings.signal.luma = REC709_LUMA;
    settings.look.brightness = 1.5;
    assert!(settings.check().is_err());
    for kind in every_kind().into_iter().map(|each| each.kind) {
        assert_eq!(ScopeSettings::new(kind).check(), Ok(()));
    }
}

#[test]
fn a_frame_a_scope_cannot_read_is_refused() {
    let Some(gpu) = gpu_for_tests() else { return };
    let scopes = Scopes::new(Arc::clone(&gpu));
    let scope = scopes
        .scope(ScopeSettings::new(Kind::Histogram(Histogram::default())))
        .unwrap();
    let integers = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    assert!(matches!(
        scopes.draw(&integers, &scope, Sampling::Full),
        Err(ScopeError::Frame(_))
    ));
    no_errors(&gpu);
}

#[test]
fn the_scales_mark_codes_percents_and_nits() {
    let ten = marks(ScaleStyle::TenBit, Gamma::Rec709);
    assert_eq!(ten.first().unwrap().label, "0");
    assert_eq!(ten.last().unwrap().label, "1023");
    assert!((ten.last().unwrap().signal - 1.0).abs() < 1e-12);
    assert!((ten[4].signal - 512.0 / 1023.0).abs() < 1e-12);
    let percent = marks(ScaleStyle::Percent, Gamma::Rec709);
    assert_eq!(percent.len(), 11);
    assert!((percent[5].signal - 0.5).abs() < 1e-12);
    // PQ: 100 nits at 0.508, 1000 at 0.752, 10 000 at 1.
    let pq = marks(ScaleStyle::Nits, Gamma::St2084);
    let at = |marks: &[Mark], label: &str| marks.iter().find(|m| m.label == label).unwrap().signal;
    assert!((at(&pq, "100") - 0.508).abs() < 1e-3);
    assert!((at(&pq, "1000") - 0.7518).abs() < 1e-3);
    assert!((at(&pq, "10000") - 1.0).abs() < 1e-9);
    for nits in [0.5, 100.0, 203.0, 4000.0] {
        assert!((pq_nits(pq_signal(nits)) - nits).abs() < 1e-6 * nits.max(1.0));
    }
    // HLG: 203 nits (the white of graphics) at 0.75, 1000 at 1.
    let hlg = marks(ScaleStyle::Nits, Gamma::Hlg);
    assert!((at(&hlg, "203") - 0.75).abs() < 1e-3);
    assert!((at(&hlg, "1000") - 1.0).abs() < 1e-6);
    assert!((graticule::hlg_nits(hlg_signal(50.0)) - 50.0).abs() < 1e-6);
    assert!((graticule::scale_value(ScaleStyle::TenBit, Gamma::Rec709, 0.5) - 511.5).abs() < 1e-9);
    assert!((graticule::scale_value(ScaleStyle::Nits, Gamma::St2084, 0.508) - 100.0).abs() < 0.5);
}

#[test]
fn the_targets_and_the_skin_tone_line_are_where_vectorscopes_have_them() {
    // Red of 100 %: Cr 0.5 by any coefficients; blue: Cb 0.5.
    let full = targets(REC709_LUMA, 1.0);
    assert!((full[0].1[1] - 0.5).abs() < 1e-9 && (full[2].1[0] - 0.5).abs() < 1e-9);
    // The angles of the bars of BT.709: red near 103°, yellow 167°… in the plane of Cb and Cr.
    let angle = |[cb, cr]: [f64; 2]| cr.atan2(cb).to_degrees().rem_euclid(360.0);
    assert!((angle(full[0].1) - 102.9).abs() < 0.2);
    assert!((angle(full[5].1) - 174.8).abs() < 0.2);
    // The skin tone line lies between red and yellow.
    assert!(
        graticule::SKIN_TONE_DEGREES > angle(full[0].1)
            && graticule::SKIN_TONE_DEGREES < angle(full[5].1)
    );
    // Drawn: a box around each target of 75 %, and the line from the centre.
    let settings = ScopeSettings::new(Kind::Vectorscope(Vectorscope::default()));
    let drawn = graticule::graticule(&settings);
    assert_eq!(drawn.labels.len(), 6);
    let skin: Vec<&Line> = drawn
        .lines
        .iter()
        .filter(|line| line.from.map(f32::to_bits) == [128.0f32.to_bits(); 2])
        .collect();
    assert_eq!(skin.len(), 1);
    let [x, y] = skin[0].to;
    let degrees = f64::from(128.0 - y)
        .atan2(f64::from(x - 128.0))
        .to_degrees();
    assert!((degrees - 123.0).abs() < 0.01);
    // 96 sides of the circle, 2 of the cross, the skin tone line, 4 sides of 6 boxes.
    assert_eq!(drawn.lines.len(), 96 + 2 + 1 + 24);
}

#[test]
fn the_chromaticity_diagram_has_the_locus_and_its_gamuts() {
    let locus = spectral_locus();
    let at = |nanometres: usize| locus[(nanometres - 380) / 5];
    for (nanometres, [x, y]) in [
        (450, [0.1566, 0.0177]),
        (520, [0.0743, 0.8338]),
        (560, [0.3731, 0.6245]),
        (700, [0.7347, 0.2653]),
    ] {
        let [fx, fy] = at(nanometres);
        assert!(
            (fx - x).abs() < 0.015 && (fy - y).abs() < 0.015,
            "{nanometres}: {fx}, {fy}"
        );
    }
    // The matrix of Rec.709 to XYZ, as BT.709 has it.
    let matrix = Primaries::REC709.to_xyz();
    let expected = [
        [0.4124, 0.3576, 0.1805],
        [0.2126, 0.7152, 0.0722],
        [0.0193, 0.1192, 0.9505],
    ];
    for (row, values) in matrix.iter().zip(expected) {
        for (value, wanted) in row.iter().zip(values) {
            assert!((value - wanted).abs() < 2e-4, "{matrix:?}");
        }
    }
    // White falls on the white point, red on the red corner.
    // Sizes that put neither point on the edge of a bin.
    let integers = reference::Chromaticity::of(731, 601, &Primaries::REC709, Gamma::Rec709);
    let white = integers.bin([reference::ONE; 3]);
    let red = integers.bin([reference::ONE, 0, 0]);
    let place = |bin: u32| (bin % 731, 600 - bin / 731);
    // x 0.3127 of 0.8 is 285.7 of 731; y 0.3290 of 0.9 is 219.7 of 601.
    assert_eq!(place(white), (285, 219));
    // x 0.64 → 584.8, y 0.33 → 220.4.
    assert_eq!(place(red), (584, 220));
    assert_eq!(
        integers.bin([0; 3]),
        reference::NONE,
        "black has no chromaticity"
    );
    let settings = ScopeSettings::new(Kind::Chromaticity(Chromaticity::default()));
    let drawn = graticule::graticule(&settings);
    assert!(drawn.labels.iter().any(|label| label.text == "Rec.709"));
}

#[test]
fn the_graticule_is_drawn_into_its_pixels() {
    let settings = ScopeSettings::new(Kind::Waveform(Waveform::default()));
    let drawn = graticule::graticule(&settings);
    let rgba = graticule::rasterize(&drawn);
    assert_eq!(rgba.len(), 512 * 256 * 4);
    // The line of 0 along the bottom row, of 1023 along the top; none between 128 and 256.
    let row_alpha = |row: usize| rgba[row * 512 * 4 + 100 * 4 + 3];
    assert_eq!(row_alpha(255), 255);
    assert_eq!(row_alpha(0), 255);
    assert_eq!(row_alpha(200), 0);
    assert_eq!(graticule::rasterize(&drawn), rgba, "the same each time");
}

#[test]
fn integer_bins_of_the_vectorscope_are_those_of_its_floats() {
    let coefficients = reference::Coefficients::of(REC709_LUMA);
    assert_eq!(coefficients.kr + coefficients.kg + coefficients.kb, 1 << 15);
    let vector = reference::Vector::of(256, 1.0, Tones::All, 0.3, 0.7);
    let mut rng = Rng(11);
    let [kr, kg, kb] = REC709_LUMA.map(f64::from);
    for _ in 0..10_000 {
        let rgb = [rng.unit(), rng.unit(), rng.unit()];
        let bin = vector.bin(
            coefficients,
            rgb.map(|value| reference::code(value, -1.0, 2.0)),
        );
        let [r, g, b] = rgb.map(f64::from);
        let y = kr * r + kg * g + kb * b;
        let (cb, cr) = ((b - y) / (2.0 * (1.0 - kb)), (r - y) / (2.0 * (1.0 - kr)));
        let column = ((cb + 0.5) * 256.0).floor();
        let row = 255.0 - ((cr + 0.5) * 256.0).floor();
        let (bin_column, bin_row) = (f64::from(bin % 256), f64::from(bin / 256));
        assert!(
            (bin_column - column).abs() <= 1.0 && (bin_row - row).abs() <= 1.0,
            "{rgb:?}"
        );
    }
}

#[test]
fn sampling_is_chosen_by_the_size_of_the_frame() {
    assert_eq!(Sampling::auto(1920, 1080), Sampling::Full);
    assert_eq!(Sampling::auto(2560, 1440), Sampling::Full);
    assert_eq!(Sampling::auto(3840, 2160), Sampling::Full);
    assert_eq!(Sampling::auto(4096, 2160), Sampling::Full);
    assert_eq!(Sampling::auto(6144, 3456), Sampling::Half);
    assert_eq!(Sampling::auto(8192, 4320), Sampling::Quarter);
}

/// A frame like a picture: gradients, soft shapes and noise.
fn natural(width: u32, height: u32) -> Picture {
    let mut rng = Rng(42);
    #[allow(clippy::cast_precision_loss, reason = "places in a frame")]
    Picture::new(width, height, |x, y| {
        let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
        let noise = (rng.unit() - 0.5) * 0.04;
        let circle = if (u - 0.6).hypot(v - 0.4) < 0.2 {
            0.3
        } else {
            0.0
        };
        [
            0.2 + 0.6 * u + circle + noise,
            0.15 + 0.5 * v + noise,
            0.4 - 0.3 * u * v + circle * 0.5 + noise,
        ]
    })
}

/// Milliseconds a draw takes, `times` submitted one after another, after a few to warm up.
fn milliseconds(gpu: &Gpu, times: u32, mut draw: impl FnMut()) -> f64 {
    for _ in 0..3 {
        draw();
    }
    gpu.device()
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let start = Instant::now();
    for _ in 0..times {
        draw();
    }
    gpu.device()
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    start.elapsed().as_secs_f64() * 1000.0 / f64::from(times)
}

/// How long each scope takes of a frame of 4K. Run alone, with `--release --ignored`.
#[test]
#[ignore = "a measurement of time, for a card and a build of release"]
fn how_long_scopes_of_4k_take() {
    let Some(gpu) = gpu_for_tests() else { return };
    let scopes = Scopes::new(Arc::clone(&gpu));
    let picture = natural(3840, 2160);
    let texture = picture.texture(&gpu);
    let flat = Picture::new(3840, 2160, |_, _| [0.5, 0.4, 0.3]);
    let flat_texture = flat.texture(&gpu);
    let auto = Sampling::auto(3840, 2160);
    let mut slowest_auto: f64 = 0.0;
    println!("{gpu:?}");
    for settings in every_kind().into_iter().take(10) {
        let scope = scopes.scope(settings.clone()).unwrap();
        let mut line = format!(
            "{:<14}",
            format!("{:?}", settings.kind)
                .split('(')
                .next()
                .unwrap_or("")
        );
        for (name, frame) in [("picture", &texture), ("flat", &flat_texture)] {
            for sampling in [Sampling::Full, Sampling::Half, Sampling::Quarter] {
                let time = milliseconds(&gpu, 60, || {
                    scopes.draw(frame, &scope, sampling).unwrap();
                });
                if sampling == auto {
                    slowest_auto = slowest_auto.max(time);
                }
                write!(line, " {name} {sampling:?} {time:.3} ms;").unwrap();
            }
        }
        println!("{line} {:?}", settings.kind);
    }
    let time = milliseconds(&gpu, 10, || {
        scopes
            .statistics(&texture, &Signal::default(), 256)
            .unwrap();
    });
    println!("statistics with a read back: {time:.3} ms");
    println!("slowest scope at {auto:?}: {slowest_auto:.3} ms");
    no_errors(&gpu);
    assert!(slowest_auto < 2.0, "{slowest_auto} ms");
}

/// The pictures of the scopes of a frame like a picture, written as raw RGBA to the directory
/// that `COLUMBINA_SCOPES_OUT` names, to be looked at.
#[test]
#[ignore = "writes pictures to look at"]
fn pictures_to_look_at() {
    let Some(gpu) = gpu_for_tests() else { return };
    let Ok(directory) = std::env::var("COLUMBINA_SCOPES_OUT") else {
        return;
    };
    let scopes = Scopes::new(Arc::clone(&gpu));
    let mut frames = vec![("natural", natural(1280, 720)), ("bars", bars(0.75))];
    #[allow(clippy::cast_precision_loss, reason = "a small count")]
    frames.push(("ramp", Picture::new(512, 64, |x, _| [x as f32 / 511.0; 3])));
    for (name, picture) in frames {
        let texture = picture.texture(&gpu);
        for (index, settings) in every_kind().into_iter().enumerate() {
            let scope = scopes.scope(settings).unwrap();
            scopes.draw(&texture, &scope, Sampling::Full).unwrap();
            let image = scopes.read_image(&scope).unwrap();
            let path = format!(
                "{directory}/{name}-{index}-{}x{}.rgba",
                image.width, image.height
            );
            std::fs::write(path, &image.rgba).unwrap();
        }
    }
}
