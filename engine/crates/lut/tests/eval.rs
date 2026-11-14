//! The evaluation on the processor, the reference: against functions whose interpolation is
//! known exactly, against an independent statement of tetrahedral interpolation, and against
//! FFmpeg's `lut3d` where FFmpeg is on the path.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]

use columbina_lut::{parse_cube, Domain, Interpolation, Lut, Lut1d, Lut3d};
use proptest::prelude::*;

const BOTH: [Interpolation; 2] = [Interpolation::Tetrahedral, Interpolation::Trilinear];

fn close(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= tolerance)
}

/// A spread of inputs over 0..1, the same each run.
fn inputs(count: usize) -> Vec<[f32; 3]> {
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let unit = ((state >> 40) as f64 / f64::from(1 << 24)) as f32;
        unit
    };
    (0..count).map(|_| [next(), next(), next()]).collect()
}

#[test]
fn identities_of_every_size_are_the_identity() {
    for size in [2, 3, 5, 17, 33, 65] {
        let cube = Lut3d::from_fn(size, Domain::default(), |x| x).unwrap();
        for rgb in inputs(2000) {
            for interpolation in BOTH {
                let out = cube.eval(rgb, interpolation);
                assert!(
                    close(out, rgb, 1e-6),
                    "{size} {interpolation:?} {rgb:?} {out:?}"
                );
            }
        }
        // The points themselves, the corners included.
        for rgb in [[0.0; 3], [1.0; 3], [1.0, 0.0, 1.0], [0.5, 0.25, 0.75]] {
            assert!(close(cube.tetrahedral(rgb), rgb, 1e-7));
        }
    }
    for size in [2, 4, 1024, 65536] {
        let curve = Lut1d::from_fn(size, Domain::default(), |x| x).unwrap();
        for rgb in inputs(500) {
            assert!(close(curve.eval(rgb), rgb, 1e-6), "{size} {rgb:?}");
        }
    }
}

#[test]
fn an_affine_map_is_kept_exactly_by_both_interpolations() {
    // A matrix and an offset, in a domain of its own: both interpolations are exact on any
    // function that is affine, between the points as at them.
    let matrix = [[0.8, 0.15, 0.05], [-0.1, 1.2, -0.1], [0.3, -0.2, 0.9]];
    let offset = [0.02, -0.01, 0.05];
    let affine = |x: [f64; 3]| {
        [0, 1, 2].map(|row| offset[row] + (0..3).map(|k| matrix[row][k] * x[k]).sum::<f64>())
    };
    let domain = Domain::uniform(-0.5, 1.5);
    let cube = Lut3d::from_fn(9, domain, affine).unwrap();
    for rgb in inputs(3000) {
        let rgb = rgb.map(|v| v * 2.0 - 0.5);
        let want = affine(rgb.map(f64::from)).map(|v| v as f32);
        for interpolation in BOTH {
            assert!(close(cube.eval(rgb, interpolation), want, 2e-6), "{rgb:?}");
        }
    }
}

#[test]
fn tetrahedral_is_exact_along_the_grey_diagonal_of_any_cube() {
    // On r = g = b only the lowest and highest corners of a cell are weighed, so a cube of a
    // function that mixes its channels gives, along grey, the linear interpolation of the
    // function between the grey points; trilinear weighs the off-diagonal corners too.
    let f = |[r, g, b]: [f64; 3]| {
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        [
            luma.powf(1.0 / 2.2),
            (r * g * b).cbrt(),
            (r - b).abs() + g * g,
        ]
    };
    let size = 17;
    let cube = Lut3d::from_fn(size, Domain::default(), f).unwrap();
    let last = (size - 1) as f64;
    let mut trilinear_differs = false;
    for step in 0..=1000 {
        let v = f64::from(step) / 1000.0;
        let below = (v * last).floor().min(last - 1.0);
        let t = v * last - below;
        let a = f([below / last; 3]);
        let b = f([(below + 1.0) / last; 3]);
        let want = [0, 1, 2].map(|c| (a[c] + (b[c] - a[c]) * t) as f32);
        let grey = [v as f32; 3];
        assert!(close(cube.tetrahedral(grey), want, 2e-6), "{v}");
        trilinear_differs |= !close(cube.trilinear(grey), want, 1e-4);
    }
    assert!(trilinear_differs, "the test tells the two apart");
}

/// Tetrahedral interpolation as it is usually stated: sort the fractions, and walk from the
/// lowest corner to the highest, one axis at a time, in that order.
fn tetrahedral_by_sorting(cube: &Lut3d, rgb: [f32; 3]) -> [f32; 3] {
    let last = cube.size() - 1;
    let domain = cube.domain();
    let x: Vec<f64> = (0..3)
        .map(|c| {
            let (min, max) = (f64::from(domain.min[c]), f64::from(domain.max[c]));
            ((f64::from(rgb[c]) - min) / (max - min)).clamp(0.0, 1.0) * last as f64
        })
        .collect();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let base: Vec<usize> = x
        .iter()
        .map(|x| (x.floor() as usize).min(last - 1))
        .collect();
    let f: Vec<f64> = (0..3).map(|c| x[c] - base[c] as f64).collect();
    let mut order = [0, 1, 2];
    order.sort_by(|&a, &b| f[b].total_cmp(&f[a]));
    let mut at = [base[0], base[1], base[2]];
    let point = |at: [usize; 3]| cube.at(at[0], at[1], at[2]).map(f64::from);
    let mut out = point(at).map(|v| v * (1.0 - f[order[0]]));
    for (k, &axis) in order.iter().enumerate() {
        at[axis] += 1;
        let weight = f[axis] - order.get(k + 1).map_or(0.0, |&next| f[next]);
        let p = point(at);
        for c in 0..3 {
            out[c] += weight * p[c];
        }
    }
    out.map(|v| v as f32)
}

fn random_cube(size: usize, values: &[f32]) -> Lut3d {
    let table = (0..size * size * size)
        .map(|i| [0, 1, 2].map(|c| values[(i * 3 + c) % values.len()]))
        .collect();
    Lut3d::new(size, Domain::uniform(-0.25, 1.25), table).unwrap()
}

proptest! {
    #[test]
    fn tetrahedral_is_the_walk_along_the_sorted_fractions(
        size in 2_usize..7,
        values in prop::collection::vec(-2.0_f32..3.0, 30..60),
        rgb in prop::array::uniform3(-0.5_f32..1.5),
    ) {
        let cube = random_cube(size, &values);
        let want = tetrahedral_by_sorting(&cube, rgb);
        prop_assert!(close(cube.tetrahedral(rgb), want, 1e-5), "{:?} {:?}", cube.tetrahedral(rgb), want);
    }

    #[test]
    fn both_interpolations_stay_within_the_corners_of_their_cell(
        size in 2_usize..7,
        values in prop::collection::vec(-2.0_f32..3.0, 30..60),
        rgb in prop::array::uniform3(-0.5_f32..1.5),
    ) {
        // Each is a weighing of the corners with weights from 0 to 1 that sum to 1, so each lies
        // between the least and the greatest corner, and the two differ by no more than that.
        let cube = random_cube(size, &values);
        let last = size - 1;
        let domain = cube.domain();
        let base: Vec<usize> = (0..3).map(|c| {
            let t = ((f64::from(rgb[c]) - f64::from(domain.min[c]))
                / f64::from(domain.max[c] - domain.min[c])).clamp(0.0, 1.0);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let i = (t * last as f64).floor() as usize;
            i.min(last - 1)
        }).collect();
        let tet = cube.tetrahedral(rgb);
        let tri = cube.trilinear(rgb);
        for c in 0..3 {
            let corners: Vec<f32> = (0..8).map(|k| {
                cube.at(base[0] + (k & 1), base[1] + ((k >> 1) & 1), base[2] + (k >> 2))[c]
            }).collect();
            let low = corners.iter().copied().fold(f32::INFINITY, f32::min) - 1e-5;
            let high = corners.iter().copied().fold(f32::NEG_INFINITY, f32::max) + 1e-5;
            prop_assert!((low..=high).contains(&tet[c]) && (low..=high).contains(&tri[c]));
            prop_assert!((tet[c] - tri[c]).abs() <= high - low);
        }
    }
}

#[test]
fn on_a_smooth_cube_trilinear_and_tetrahedral_differ_by_little() {
    // A power and a mix of the channels, smooth as a look is away from black: at 33 points the
    // two readings differ by less than a code of 10 bits (1/1023).
    let look = |[r, g, b]: [f64; 3]| {
        let mixed = [
            0.9 * r + 0.1 * g,
            0.05 * r + 0.9 * g + 0.05 * b,
            0.1 * g + 0.9 * b,
        ];
        mixed.map(|v| v.max(0.0).powf(1.25))
    };
    let cube = Lut3d::from_fn(33, Domain::default(), look).unwrap();
    let mut most = 0.0_f32;
    for rgb in inputs(20_000) {
        let (tet, tri) = (cube.tetrahedral(rgb), cube.trilinear(rgb));
        for c in 0..3 {
            most = most.max((tet[c] - tri[c]).abs());
        }
    }
    assert!(most < 1.0 / 1023.0, "{most}");
    assert!(most > 0.0);
}

#[test]
fn input_beyond_the_domain_is_what_the_edge_gives() {
    let gamma = |x: [f64; 3]| x.map(|v| v.powf(1.0 / 2.4));
    let cube = Lut3d::from_fn(17, Domain::default(), gamma).unwrap();
    for interpolation in BOTH {
        assert_eq!(
            cube.eval([-3.0, 7.0, 0.5], interpolation),
            cube.eval([0.0, 1.0, 0.5], interpolation)
        );
        assert_eq!(
            cube.eval([f32::NAN, 0.5, 0.5], interpolation),
            cube.eval([0.0, 0.5, 0.5], interpolation)
        );
    }
    let curve = Lut1d::from_fn(64, Domain::uniform(-1.0, 2.0), |x| x.map(|v| v * v)).unwrap();
    assert_eq!(curve.eval([-5.0, 5.0, f32::NAN]), [1.0, 4.0, 1.0]);
}

#[test]
fn a_shaper_feeds_the_cube() {
    // A log-to-linear shaper over a wide domain, then a cube of 0..1: the LUT is the cube of the
    // shaper's output.
    let shaper = Lut1d::from_fn(4096, Domain::uniform(-0.1, 1.6), |x| {
        x.map(|v| ((v + 0.1) / 1.7).max(0.0).sqrt())
    })
    .unwrap();
    let cube = Lut3d::from_fn(17, Domain::default(), |[r, g, b]| [g, b, r]).unwrap();
    let lut = Lut::new(
        Some("shaped".into()),
        Some(shaper.clone()),
        Some(cube.clone()),
    )
    .unwrap();
    for rgb in inputs(500) {
        let want = cube.tetrahedral(shaper.eval(rgb));
        assert_eq!(lut.eval(rgb, Interpolation::Tetrahedral), want);
    }
    let read = parse_cube(lut.to_cube().unwrap().as_bytes()).unwrap();
    assert_eq!(read, lut);
}

#[test]
fn the_lut3d_of_ffmpeg_reads_a_cube_as_this_does() {
    // FFmpeg's `lut3d`, on pictures of 32-bit floats, is an implementation of its own of both
    // interpolations. Where it is not on the path, there is nothing to compare with.
    let Ok(version) = std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
    else {
        eprintln!("no ffmpeg on the path: not compared");
        return;
    };
    assert!(version.status.success());
    let dir = tempfile::tempdir().unwrap();
    let values: Vec<f32> = inputs(400)
        .into_iter()
        .flatten()
        .map(|v| v * 1.4 - 0.2)
        .collect();
    let size = 9;
    let table = (0..size * size * size)
        .map(|i| [0, 1, 2].map(|c| values[(i * 3 + c) % values.len()]))
        .collect();
    let cube = Lut3d::new(size, Domain::default(), table).unwrap();
    let lut = Lut::new(None, None, Some(cube.clone())).unwrap();
    std::fs::write(dir.path().join("random.cube"), lut.to_cube().unwrap()).unwrap();
    let (width, height) = (64, 32);
    let pixels = inputs(width * height);
    // Planar G, B, R, as gbrpf32le lays them out.
    let mut raw = Vec::new();
    for plane in [1, 2, 0] {
        for p in &pixels {
            raw.extend_from_slice(&p[plane].to_le_bytes());
        }
    }
    std::fs::write(dir.path().join("in.raw"), &raw).unwrap();
    for (interpolation, name) in [
        (Interpolation::Tetrahedral, "tetrahedral"),
        (Interpolation::Trilinear, "trilinear"),
    ] {
        let status = std::process::Command::new("ffmpeg")
            .current_dir(dir.path())
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo"])
            .args(["-pix_fmt", "gbrpf32le", "-s", &format!("{width}x{height}")])
            .args(["-i", "in.raw", "-vf"])
            .arg(format!("lut3d=file=random.cube:interp={name}"))
            .args(["-f", "rawvideo", "-pix_fmt", "gbrpf32le", "out.raw"])
            .status()
            .unwrap();
        assert!(status.success(), "ffmpeg {name}");
        let out = std::fs::read(dir.path().join("out.raw")).unwrap();
        assert_eq!(out.len(), raw.len());
        let value = |plane: usize, i: usize| {
            let at = (plane * width * height + i) * 4;
            f32::from_le_bytes(out[at..at + 4].try_into().unwrap())
        };
        for (i, rgb) in pixels.iter().enumerate() {
            let theirs = [value(2, i), value(0, i), value(1, i)];
            let ours = cube.eval(*rgb, interpolation);
            assert!(
                close(ours, theirs, 1e-5),
                "{name} {rgb:?}: {ours:?} {theirs:?}"
            );
        }
    }
}
