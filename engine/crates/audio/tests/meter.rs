//! The level meters (plan T9): the PPM of IEC 60268-18 falls 20 dB in 1.7 s and reads a burst
//! of 5 ms; its peak is held 2 s; the VU comes to 99 % in 300 ms; the RMS of a sine is 3 dB under
//! its peak; the taps are the same however the sound is cut, lose nothing a reader reads in
//! time, and take no memory on the thread of the mix.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::float_cmp,
    reason = "samples and levels of tests"
)]

use columbina_audio::meter::{
    level_tap, position, Ballistics, Detector, LevelCell, MeterType, PeakIndicator, Scale,
};
use columbina_audio::testing::{noise, realtime, sine, CountingAllocator, Rng};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const RATE: u32 = 48_000;

/// What a meter of `meter` shows of channel 0 of `signal` (mono), cell by cell: the bar and the
/// peak indicator after each cell of 10 ms.
fn shown(meter: MeterType, signal: &[f32]) -> Vec<(f64, Option<f64>)> {
    let (mut tap, mut reader) = level_tap(RATE, 1);
    let mut ballistics = Ballistics::new(meter, 1, reader.cell_seconds());
    let mut shown = Vec::new();
    for block in signal.chunks(480) {
        tap.take(&[block]);
        let lost = reader.drain(|cell| {
            ballistics.push(cell);
            let reading = ballistics.shown(0);
            shown.push((reading.bar, reading.peak));
        });
        assert_eq!(lost, 0);
    }
    shown
}

/// `frames` of a 1 kHz sine at full scale, then `after` of silence.
fn tone_then_silence(frames: usize, after: usize) -> Vec<f32> {
    let mut signal = sine(RATE, 1_000.0, 1.0, frames);
    signal.resize(frames + after, 0.0);
    signal
}

fn assert_near(what: &str, value: f64, expected: f64, within: f64) {
    assert!(
        (value - expected).abs() <= within,
        "{what}: {value}, expected {expected} ± {within}"
    );
}

#[test]
fn the_iec_meter_falls_20_db_in_1_7_seconds() {
    // 1 s of a full-scale tone, then 3 s of silence: 1.7 s after the stop, −20 ± 0.5 dB.
    let shown = shown(
        MeterType::Iec60268_18,
        &tone_then_silence(48_000, 3 * 48_000),
    );
    assert_near("steady", shown[99].0, 0.0, 0.01);
    assert_near("after 1.7 s", shown[99 + 170].0, -20.0, 0.5);
    assert_near("after 0.85 s", shown[99 + 85].0, -10.0, 0.5);
}

#[test]
fn the_iec_meter_reads_a_burst_of_5_ms() {
    let mut signal = vec![0.0; 4_800];
    signal.extend(sine(RATE, 1_000.0, 1.0, 240));
    signal.resize(48_000, 0.0);
    let shown = shown(MeterType::Iec60268_18, &signal);
    let top = shown
        .iter()
        .map(|&(bar, _)| bar)
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(top >= -2.0, "{top}");
}

#[test]
fn the_peak_is_held_two_seconds_then_falls() {
    let shown = shown(
        MeterType::Iec60268_18,
        &tone_then_silence(48_000, 5 * 48_000),
    );
    // Held at 0 dB through 2 s after the stop, then falling 20 dB in 1.7 s.
    assert_eq!(shown[99 + 199].1, Some(0.0));
    assert_near(
        "1.7 s into the fall",
        shown[99 + 200 + 170].1.unwrap(),
        -20.0,
        0.5,
    );
    // The bar has fallen meanwhile.
    assert!(shown[99 + 199].0 < -20.0);
}

#[test]
fn each_peak_indicator_holds_as_long_as_its_name() {
    let signal = tone_then_silence(48_000, 7 * 48_000);
    let custom = |peak| MeterType::Custom {
        detector: Detector::SamplePeak,
        scale: Scale::Iec,
        decay: 1.7,
        peak,
    };
    let off = shown(custom(PeakIndicator::Off), &signal);
    assert!(off.iter().all(|&(_, peak)| peak.is_none()));
    let short = shown(custom(PeakIndicator::ShortHoldFall), &signal);
    assert_eq!(short[99 + 99].1, Some(0.0));
    assert!(short[99 + 110].1.unwrap() < -0.5);
    let medium_hold = shown(custom(PeakIndicator::MediumHold), &signal);
    assert_eq!(medium_hold[99 + 199].1, Some(0.0));
    assert_eq!(medium_hold[99 + 201].1, Some(f64::NEG_INFINITY));
    let long = shown(custom(PeakIndicator::LongHold), &signal);
    assert_eq!(long[99 + 499].1, Some(0.0));
    assert_eq!(long[99 + 501].1, Some(f64::NEG_INFINITY));
    let infinite = shown(custom(PeakIndicator::InfiniteHold), &signal);
    assert_eq!(infinite.last().unwrap().1, Some(0.0));
}

#[test]
fn a_custom_decay_sets_the_fall() {
    let meter = MeterType::Custom {
        detector: Detector::SamplePeak,
        scale: Scale::QuasiLinear,
        decay: 0.5,
        peak: PeakIndicator::MediumHoldFall,
    };
    let shown = shown(meter, &tone_then_silence(48_000, 48_000));
    assert_near("after 0.5 s", shown[99 + 50].0, -20.0, 0.5);
    let clamped = MeterType::Custom {
        detector: Detector::SamplePeak,
        scale: Scale::Iec,
        decay: 100.0,
        peak: PeakIndicator::Off,
    };
    assert_eq!(clamped.bar_fall(), 4.0);
    assert_eq!(MeterType::DigitalVu.peak_fall(), 40.0);
    assert_eq!(MeterType::Iec60268_18.bar_fall(), 20.0 / 1.7);
}

#[test]
fn the_vu_comes_to_99_percent_in_300_ms_and_falls_as_fast() {
    // A full-scale sine has an RMS of −3.01 dB.
    let shown = shown(MeterType::DigitalVu, &tone_then_silence(96_000, 4 * 48_000));
    let steady = shown[199].0;
    assert_near("steady", steady, -3.01, 0.01);
    // At 300 ms, 99 % of the RMS: −0.087 dB under it.
    let at = 10f64.powf(shown[29].0 / 20.0) / 10f64.powf(steady / 20.0);
    assert_near("at 300 ms", at, 0.99, 0.003);
    assert!(10f64.powf(shown[24].0 / 20.0) / 10f64.powf(steady / 20.0) < 0.98);
    // Never over (critically damped).
    assert!(shown[..200].iter().all(|&(bar, _)| bar <= steady + 1e-9));
    // 300 ms after the stop, 1 % left.
    let left = 10f64.powf(shown[199 + 30].0 / 20.0) / 10f64.powf(steady / 20.0);
    assert!(left < 0.012, "{left}");
    // Its peak segment falls 20 dB in 0.5 s after 2 s.
    assert_near("peak", shown[199 + 200 + 50].1.unwrap(), -20.0, 0.5);
}

#[test]
fn the_rms_of_a_sine_is_3_db_under_its_peak_over_300_ms() {
    let meter = MeterType::Custom {
        detector: Detector::Rms,
        scale: Scale::Iec,
        decay: 1.7,
        peak: PeakIndicator::Off,
    };
    let shown = shown(meter, &tone_then_silence(48_000, 48_000));
    assert_near("steady", shown[99].0, -3.01, 0.01);
    // A rectangle of 300 ms: 100 ms into it, a third of the power.
    assert_near("at 100 ms", shown[9].0, -3.01 - 10.0 * 3f64.log10(), 0.01);
    // 300 ms after the stop, nothing.
    assert_eq!(shown[99 + 30].0, f64::NEG_INFINITY);
    assert!(shown[99 + 29].0 > -30.0);
}

#[test]
fn the_tap_publishes_the_same_cells_however_the_sound_is_cut() {
    let channels = [
        noise(1, 0.9, 24_000),
        noise(2, 0.2, 24_000),
        noise(3, 0.5, 24_000),
    ];
    let cells = |blocks: &[usize]| {
        let (mut tap, mut reader) = level_tap(RATE, 3);
        let mut cells: Vec<LevelCell> = Vec::new();
        let mut at = 0;
        let mut sizes = blocks.iter().cycle();
        while at < 24_000 {
            let end = (at + sizes.next().unwrap()).min(24_000);
            tap.take(&[
                &channels[0][at..end],
                &channels[1][at..end],
                &channels[2][at..end],
            ]);
            assert_eq!(reader.drain(|cell| cells.push(*cell)), 0);
            at = end;
        }
        cells
    };
    let whole = cells(&[1]);
    assert_eq!(whole.len(), 50);
    let mut rng = Rng::new(5);
    for _ in 0..4 {
        let blocks: Vec<usize> = (0..40).map(|_| rng.range(1, 4_096)).collect();
        assert_eq!(cells(&blocks), whole);
    }
    // The peak and the mean square of the first cell, by hand.
    let first = &channels[0][..480];
    let peak = first.iter().fold(0.0_f32, |p, x| p.max(x.abs()));
    let square = first
        .iter()
        .map(|&x| f64::from(x) * f64::from(x))
        .sum::<f64>()
        / 480.0;
    assert_eq!(whole[0].peaks[0], peak);
    assert_eq!(whole[0].squares[0], square as f32);
    assert_eq!(whole[0].peaks[3], 0.0);
}

#[test]
fn a_reader_late_loses_only_what_the_tap_no_longer_keeps() {
    let (mut tap, mut reader) = level_tap(RATE, 2);
    let block = vec![0.5_f32; 480 * 20];
    tap.take(&[&block, &block]);
    let mut read = 0;
    assert_eq!(reader.drain(|_| read += 1), 4);
    assert_eq!(read, 16);
    let mut second = tap.reader(RATE);
    tap.take(&[&block[..480], &block[..480]]);
    let mut count = 0;
    assert_eq!(second.drain(|_| count += 1), 0);
    assert_eq!(count, 1);
    assert_eq!(second.cell_seconds(), 0.01);
}

#[test]
fn the_tap_and_the_ballistics_take_no_memory() {
    let (mut tap, mut reader) = level_tap(RATE, 2);
    let mut ballistics = Ballistics::new(MeterType::DigitalVu, 2, reader.cell_seconds());
    let signal = noise(9, 0.5, 4_800);
    let ((), counted) = realtime(|| {
        for _ in 0..20 {
            tap.take(&[&signal, &signal]);
            tap.take(&[&signal[..33]]);
            reader.drain(|cell| ballistics.push(cell));
            let _ = ballistics.shown(1);
        }
        tap.reset();
        ballistics.reset();
        ballistics.set_meter(MeterType::Iec60268_18);
    });
    assert!(counted.none(), "{counted:?}");
}

#[test]
fn the_scales_place_their_levels() {
    assert_eq!(position(-60.0, Scale::Iec), 0.0);
    assert_eq!(position(-90.0, Scale::Iec), 0.0);
    assert_eq!(position(f32::NEG_INFINITY, Scale::QuasiLinear), 0.0);
    assert_eq!(position(-40.0, Scale::Iec), 0.25);
    assert_eq!(position(-20.0, Scale::Iec), 0.5);
    assert_eq!(position(-10.0, Scale::Iec), 0.75);
    assert_eq!(position(0.0, Scale::Iec), 1.0);
    assert_eq!(position(3.0, Scale::Iec), 1.0);
    assert_eq!(position(-30.0, Scale::QuasiLinear), 0.5);
    assert_eq!(position(-15.0, Scale::QuasiLinear), 0.75);
    assert_eq!(MeterType::default(), MeterType::Iec60268_18);
    assert_eq!(MeterType::DigitalVu.detector(), Detector::Vu);
    assert_eq!(MeterType::Iec60268_18.scale(), Scale::Iec);
    assert_eq!(MeterType::Iec60268_18.peak(), PeakIndicator::MediumHoldFall);
}
