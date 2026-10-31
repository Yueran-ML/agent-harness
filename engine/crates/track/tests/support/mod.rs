//! Frames made in the tests: a textured scene (value noise of three scales and soft-edged
//! shapes) seen through a known motion, with noise, a change of brightness and occluders.

#![allow(dead_code, reason = "each test file uses its own part")]
#![allow(clippy::many_single_char_names, reason = "the letters of the formulas")]

use columbina_track::{run, Direction, Job, Luma, Mat3, Point, Region, Report, TrackData, Tracker};
use columbina_track::{Components, Step};
use std::sync::atomic::AtomicBool;

/// `SplitMix64`.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// From 0 to 1.
    pub fn unit(&mut self) -> f64 {
        #[allow(clippy::cast_precision_loss, reason = "53 bits")]
        let v = (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64;
        v
    }

    /// Normal, mean 0 and spread 1 (Box–Muller).
    pub fn normal(&mut self) -> f64 {
        let (a, b) = (self.unit().max(1e-300), self.unit());
        (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
    }
}

struct Octave {
    spacing: f64,
    amp: f64,
    size: usize,
    values: Vec<f64>,
}

enum Shape {
    Disk { c: Point, r: f64, v: f64 },
    Rect { lo: Point, hi: Point, v: f64 },
}

/// A scene with texture everywhere (it repeats), and shapes about the frame it is made for.
pub struct Scene {
    octaves: Vec<Octave>,
    shapes: Vec<Shape>,
    /// The shapes that reach into each cell of a grid of [`CELL`] pixels from `origin`.
    cells: Vec<Vec<usize>>,
    origin: Point,
    columns: usize,
    rows: usize,
}

const CELL: f64 = 32.0;

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

impl Scene {
    /// A scene for frames of about `width × height`.
    pub fn new(seed: u64, width: usize, height: usize) -> Self {
        let mut rng = Rng::new(seed);
        let octaves = [(4.0, 0.25), (9.0, 0.35), (23.0, 0.4)]
            .into_iter()
            .map(|(spacing, amp)| {
                let size = 256;
                Octave {
                    spacing,
                    amp,
                    size,
                    values: (0..size * size).map(|_| rng.unit()).collect(),
                }
            })
            .collect();
        #[allow(clippy::cast_precision_loss, reason = "small")]
        let (w, h) = (width as f64, height as f64);
        let count = width * height / 1500;
        let shapes = (0..count)
            .map(|_| {
                let c = [
                    rng.unit() * 1.6 * w - 0.3 * w,
                    rng.unit() * 1.6 * h - 0.3 * h,
                ];
                let v = rng.unit();
                if rng.unit() < 0.5 {
                    Shape::Disk {
                        c,
                        r: 3.0 + rng.unit() * 12.0,
                        v,
                    }
                } else {
                    let (a, b) = (3.0 + rng.unit() * 14.0, 3.0 + rng.unit() * 14.0);
                    Shape::Rect {
                        lo: [c[0] - a, c[1] - b],
                        hi: [c[0] + a, c[1] + b],
                        v,
                    }
                }
            })
            .collect::<Vec<_>>();
        let origin = [-0.3 * w - 40.0, -0.3 * h - 40.0];
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "positive"
        )]
        let (columns, rows) = (
            (1.6 * w / CELL).ceil() as usize + 4,
            (1.6 * h / CELL).ceil() as usize + 4,
        );
        let mut cells = vec![Vec::new(); columns * rows];
        for (i, shape) in shapes.iter().enumerate() {
            let (lo, hi) = match shape {
                Shape::Disk { c, r, .. } => (
                    [c[0] - r - 2.0, c[1] - r - 2.0],
                    [c[0] + r + 2.0, c[1] + r + 2.0],
                ),
                Shape::Rect { lo, hi, .. } => {
                    ([lo[0] - 2.0, lo[1] - 2.0], [hi[0] + 2.0, hi[1] + 2.0])
                }
            };
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "within the grid"
            )]
            let cell =
                |v: f64, o: f64, n: usize| (((v - o) / CELL).floor().max(0.0) as usize).min(n - 1);
            for y in cell(lo[1], origin[1], rows)..=cell(hi[1], origin[1], rows) {
                for x in cell(lo[0], origin[0], columns)..=cell(hi[0], origin[0], columns) {
                    cells[y * columns + x].push(i);
                }
            }
        }
        Self {
            octaves,
            shapes,
            cells,
            origin,
            columns,
            rows,
        }
    }

    /// Its brightness at `p`, from 0 to 1.
    pub fn value(&self, p: Point) -> f64 {
        let mut out = 0.0;
        for o in &self.octaves {
            let (x, y) = (p[0] / o.spacing + 1000.0, p[1] / o.spacing + 1000.0);
            let (fx, fy) = (x.floor(), y.floor());
            let (tx, ty) = (smooth(x - fx), smooth(y - fy));
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "positive"
            )]
            let (ix, iy) = (fx as usize, fy as usize);
            let at = |i: usize, j: usize| o.values[(j % o.size) * o.size + i % o.size];
            let top = at(ix, iy) + (at(ix + 1, iy) - at(ix, iy)) * tx;
            let bottom = at(ix, iy + 1) + (at(ix + 1, iy + 1) - at(ix, iy + 1)) * tx;
            out += o.amp * (top + (bottom - top) * ty);
        }
        let (gx, gy) = (
            ((p[0] - self.origin[0]) / CELL).floor(),
            ((p[1] - self.origin[1]) / CELL).floor(),
        );
        #[allow(clippy::cast_precision_loss, reason = "small")]
        let inside = gx >= 0.0 && gy >= 0.0 && gx < self.columns as f64 && gy < self.rows as f64;
        if !inside {
            return out;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "within the grid"
        )]
        let here = &self.cells[gy as usize * self.columns + gx as usize];
        for s in here.iter().map(|&i| &self.shapes[i]) {
            let (cover, v) = match s {
                Shape::Disk { c, r, v } => {
                    if (p[0] - c[0]).abs() > r + 1.0 || (p[1] - c[1]).abs() > r + 1.0 {
                        continue;
                    }
                    let d = (p[0] - c[0]).hypot(p[1] - c[1]);
                    (smooth(((r - d) * 0.5 + 0.5).clamp(0.0, 1.0)), *v)
                }
                Shape::Rect { lo, hi, v } => {
                    let inside_x = ((p[0] - lo[0]).min(hi[0] - p[0]) * 0.5 + 0.5).clamp(0.0, 1.0);
                    let inside_y = ((p[1] - lo[1]).min(hi[1] - p[1]) * 0.5 + 0.5).clamp(0.0, 1.0);
                    if inside_x <= 0.0 || inside_y <= 0.0 {
                        continue;
                    }
                    (smooth(inside_x.min(inside_y)), *v)
                }
            };
            out += (v - out) * cover * 0.7;
        }
        out
    }
}

/// How a frame looks besides its motion.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    /// The spread of Gaussian noise added.
    pub noise: f64,
    /// Brightness times this...
    pub gain: f64,
    /// ...plus this.
    pub offset: f64,
    /// The seed of the noise.
    pub seed: u64,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            noise: 0.0,
            gain: 1.0,
            offset: 0.0,
            seed: 1,
        }
    }
}

/// The frame of `width × height` where the scene has moved by `motion` (a point of the scene at
/// `s` is seen at `motion(s)`), looking as `look` says; `cover` paints over it.
pub fn render(
    scene: &Scene,
    width: usize,
    height: usize,
    motion: &Mat3,
    look: Look,
    cover: Option<&dyn Fn(Point) -> Option<f64>>,
) -> Luma {
    render_with(&|p| scene.value(p), width, height, motion, look, cover)
}

/// As [`render`], for any scene.
pub fn render_with(
    scene: &dyn Fn(Point) -> f64,
    width: usize,
    height: usize,
    motion: &Mat3,
    look: Look,
    cover: Option<&dyn Fn(Point) -> Option<f64>>,
) -> Luma {
    let back = motion.inverse().expect("a motion that can be undone");
    let mut rng = Rng::new(look.seed);
    let mut data = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            #[allow(clippy::cast_precision_loss, reason = "small")]
            let p = [x as f64, y as f64];
            let mut v = scene(back.apply(p));
            if let Some(over) = cover.and_then(|c| c(p)) {
                v = over;
            }
            v = v * look.gain + look.offset;
            if look.noise > 0.0 {
                v += look.noise * rng.normal();
            }
            #[allow(clippy::cast_possible_truncation, reason = "a sample")]
            data.push(v as f32);
        }
    }
    Luma::new(width, height, data).expect("a plane of the right size")
}

/// The similarity that scales by `zoom` and turns by `turn` radians about `c`, then shifts by
/// `shift`.
pub fn similarity(c: Point, zoom: f64, turn: f64, shift: Point) -> Mat3 {
    let (s, k) = turn.sin_cos();
    let local = Mat3([
        [zoom * k, -zoom * s, 0.0],
        [zoom * s, zoom * k, 0.0],
        [0.0, 0.0, 1.0],
    ]);
    Mat3::translation(c[0] + shift[0], c[1] + shift[1])
        .mul(&local)
        .mul(&Mat3::translation(-c[0], -c[1]))
}

/// The corners of a rectangle.
pub fn corners(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

/// Track `frames` (frame `i` of the clip is `frames[i]`) from `start` as `direction` says, into
/// `data`.
pub fn track(
    frames: &[Luma],
    data: &mut TrackData,
    tracker: Tracker,
    components: Components,
    region: Region,
    start: i64,
    direction: Direction,
) -> Report {
    let job = Job {
        tracker,
        components,
        region,
        start,
        first: 0,
        last: i64::try_from(frames.len()).unwrap() - 1,
        direction,
    };
    let stop = AtomicBool::new(false);
    run(
        &job,
        data,
        &mut |f| usize::try_from(f).ok().and_then(|i| frames.get(i)).cloned(),
        &stop,
        &mut |_: i64, _: &Step| {},
    )
    .expect("a track")
}

/// The largest distance between where the track puts `points` at `frame` and where `truth` does.
pub fn error_at(data: &TrackData, frame: i64, truth: &Mat3, points: &[Point]) -> f64 {
    data.track_matrix(frame).distance_on(truth, points)
}
