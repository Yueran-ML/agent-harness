//! Pyramidal Lucas–Kanade (Bouguet, "Pyramidal Implementation of the Lucas Kanade Feature
//! Tracker", 2000), with the window of each point compared after taking away its mean and
//! matching its spread, so that a change of brightness or contrast does not move the points.

use crate::image::Pyramid;

/// The settings of [`track`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lk {
    /// The half side of the window: it is `2·radius + 1` samples a side.
    pub radius: usize,
    /// The most steps at each level.
    pub iterations: usize,
    /// A step shorter than this (in samples of the level) ends the level.
    pub epsilon: f32,
    /// The smallest eigenvalue of the window's matrix of gradients, per sample, below which the
    /// window is too flat to track.
    pub min_eigen: f32,
}

/// What [`track`] found for a point.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tracked {
    /// Where the point is in the next frame.
    pub pos: [f32; 2],
    /// The normalised cross-correlation of its windows in the two frames, from −1 to 1.
    pub ncc: f32,
}

/// Scratch space for one point at a time.
pub(crate) struct Scratch {
    padded: Vec<f32>,
    t: Vec<f32>,
    gx: Vec<f32>,
    gy: Vec<f32>,
    j: Vec<f32>,
}

impl Scratch {
    pub fn new(radius: usize) -> Self {
        let side = 2 * radius + 1;
        Self {
            padded: vec![0.0; (side + 2) * (side + 2)],
            t: vec![0.0; side * side],
            gx: vec![0.0; side * side],
            gy: vec![0.0; side * side],
            j: vec![0.0; side * side],
        }
    }
}

fn mean(v: &[f32]) -> f32 {
    #[allow(clippy::cast_precision_loss, reason = "a small window")]
    let n = v.len() as f32;
    v.iter().sum::<f32>() / n
}

/// Where the point at `p` of `prev` is in `next`, starting from the guess `guess`, or `None`
/// when it is lost: its window too flat, or outside the frame at the finest level.
#[allow(clippy::too_many_lines, reason = "level after level, step after step")]
pub(crate) fn track(
    prev: &Pyramid,
    next: &Pyramid,
    p: [f32; 2],
    guess: [f32; 2],
    lk: &Lk,
    scratch: &mut Scratch,
) -> Option<Tracked> {
    let r = lk.radius;
    let side = 2 * r + 1;
    let levels = prev.len().min(next.len());
    #[allow(clippy::cast_precision_loss, reason = "a small window")]
    let count = (side * side) as f32;
    // The displacement at the current level, carried down level by level.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "a few levels"
    )]
    let top = 0.5_f32.powi((levels - 1) as i32);
    let mut g = [(guess[0] - p[0]) * top, (guess[1] - p[1]) * top];
    for level in (0..levels).rev() {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "a few levels"
        )]
        let scale = 0.5_f32.powi(level as i32);
        let (pl, img, img_next) = (
            [p[0] * scale, p[1] * scale],
            prev.level(level),
            next.level(level),
        );
        // The window of the template with a border of one, for its Scharr gradients.
        img.patch(pl[0], pl[1], r + 1, &mut scratch.padded);
        let ps = side + 2;
        let (mut gxx, mut gxy, mut gyy) = (0.0_f32, 0.0_f32, 0.0_f32);
        for y in 0..side {
            for x in 0..side {
                let at = |dx: usize, dy: usize| scratch.padded[(y + dy) * ps + x + dx];
                let ix = (3.0 * (at(2, 0) - at(0, 0))
                    + 10.0 * (at(2, 1) - at(0, 1))
                    + 3.0 * (at(2, 2) - at(0, 2)))
                    / 32.0;
                let iy = (3.0 * (at(0, 2) - at(0, 0))
                    + 10.0 * (at(1, 2) - at(1, 0))
                    + 3.0 * (at(2, 2) - at(2, 0)))
                    / 32.0;
                let k = y * side + x;
                scratch.t[k] = at(1, 1);
                scratch.gx[k] = ix;
                scratch.gy[k] = iy;
                gxx += ix * ix;
                gxy += ix * iy;
                gyy += iy * iy;
            }
        }
        let det = gxx * gyy - gxy * gxy;
        let min_eig = (gxx + gyy - ((gxx - gyy).powi(2) + 4.0 * gxy * gxy).sqrt()) * 0.5 / count;
        if min_eig < lk.min_eigen || det.abs() < f32::EPSILON {
            if level == 0 {
                return None;
            }
            g = [2.0 * g[0], 2.0 * g[1]];
            continue;
        }
        let mt = mean(&scratch.t);
        let st = scratch
            .t
            .iter()
            .map(|v| (v - mt).powi(2))
            .sum::<f32>()
            .sqrt();
        for v in &mut scratch.t {
            *v -= mt;
        }
        #[allow(clippy::cast_precision_loss, reason = "sides of at most 2^24")]
        let (w, h) = (img_next.width() as f32, img_next.height() as f32);
        let mut nu = [0.0_f32; 2];
        let mut last = [0.0_f32; 2];
        for step in 0..lk.iterations {
            let q = [pl[0] + g[0] + nu[0], pl[1] + g[1] + nu[1]];
            if !(q[0] >= 0.0 && q[1] >= 0.0 && q[0] <= w - 1.0 && q[1] <= h - 1.0) {
                if level == 0 {
                    return None;
                }
                break;
            }
            img_next.patch(q[0], q[1], r, &mut scratch.j);
            let mj = mean(&scratch.j);
            let sj = scratch
                .j
                .iter()
                .map(|v| (v - mj).powi(2))
                .sum::<f32>()
                .sqrt();
            let gain = if sj > 1e-6 { st / sj } else { 1.0 };
            let (mut bx, mut by) = (0.0_f32, 0.0_f32);
            for k in 0..scratch.t.len() {
                let e = scratch.t[k] - gain * (scratch.j[k] - mj);
                bx += e * scratch.gx[k];
                by += e * scratch.gy[k];
            }
            let eta = [(gyy * bx - gxy * by) / det, (gxx * by - gxy * bx) / det];
            if step > 0 && (eta[0] + last[0]).abs() < 0.01 && (eta[1] + last[1]).abs() < 0.01 {
                // Going back and forth about the answer: take the middle.
                nu = [nu[0] - eta[0] * 0.5, nu[1] - eta[1] * 0.5];
                break;
            }
            nu = [nu[0] + eta[0], nu[1] + eta[1]];
            last = eta;
            if eta[0] * eta[0] + eta[1] * eta[1] < lk.epsilon * lk.epsilon {
                break;
            }
        }
        g = [g[0] + nu[0], g[1] + nu[1]];
        if level > 0 {
            g = [2.0 * g[0], 2.0 * g[1]];
        }
    }
    let pos = [p[0] + g[0], p[1] + g[1]];
    let base = next.base();
    #[allow(clippy::cast_precision_loss, reason = "sides of at most 2^24")]
    let (w, h) = (base.width() as f32, base.height() as f32);
    if !(pos[0] >= 0.0 && pos[1] >= 0.0 && pos[0] <= w - 1.0 && pos[1] <= h - 1.0) {
        return None;
    }
    // The correlation at the answer, with the template of the finest level (still in `t`, less
    // its mean, unless that level was too flat, which returned above).
    base.patch(pos[0], pos[1], r, &mut scratch.j);
    let mj = mean(&scratch.j);
    let (mut tj, mut jj, mut tt) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (t, j) in scratch.t.iter().zip(&scratch.j) {
        let j = j - mj;
        tj += t * j;
        jj += j * j;
        tt += t * t;
    }
    let ncc = if tt > 0.0 && jj > 0.0 {
        tj / (tt * jj).sqrt()
    } else {
        0.0
    };
    Some(Tracked { pos, ncc })
}

/// `f` over `items` on up to `threads` threads, the results in the order of the items. The work
/// of each item depends on that item alone, so the results are the same whatever the threads.
pub(crate) fn par_map<T: Sync, R: Send>(
    items: &[T],
    threads: usize,
    f: impl Fn(&T, &mut Scratch) -> R + Sync,
    radius: usize,
) -> Vec<R> {
    let threads = threads.max(1).min(items.len().div_ceil(8).max(1));
    if threads == 1 {
        let mut scratch = Scratch::new(radius);
        return items.iter().map(|item| f(item, &mut scratch)).collect();
    }
    let chunk = items.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| {
                let f = &f;
                scope.spawn(move || {
                    let mut scratch = Scratch::new(radius);
                    part.iter()
                        .map(|item| f(item, &mut scratch))
                        .collect::<Vec<R>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("a thread of the tracker panicked"))
            .collect()
    })
}

/// The threads to use when the settings say 0: those of the machine, at most 8.
pub(crate) fn threads(asked: usize) -> usize {
    if asked > 0 {
        asked
    } else {
        std::thread::available_parallelism().map_or(1, |n| n.get().min(8))
    }
}
