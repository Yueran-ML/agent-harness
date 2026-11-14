//! Good features to track (Shi and Tomasi, 1994): the corners whose matrix of gradients has a
//! large smallest eigenvalue.

use crate::image::Luma;
use crate::region::Raster;

/// The settings of [`detect`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Detect {
    /// The most features to return.
    pub max: usize,
    /// A feature's eigenvalue must be at least this share of the largest one in the region.
    pub quality: f32,
    /// And at least this, per sample (the window of a flat, noisy region stays below it).
    pub min_eigen: f32,
    /// The least distance between two features, and to those already there.
    pub min_distance: f32,
    /// The least distance of a feature from the edge of the frame.
    pub border: usize,
}

/// The half side of the box over which the gradients are summed (a 5×5 box).
const BLOCK: usize = 2;

/// The best features within `region` of `img`, the strongest first, at least
/// `settings.min_distance` from each other and from `existing`. Ties go to the earlier pixel, so
/// the same plane always gives the same features.
#[allow(
    clippy::too_many_lines,
    reason = "the gradients, the box sums and the choice, in turn"
)]
pub(crate) fn detect(
    img: &Luma,
    region: &Raster,
    existing: &[[f32; 2]],
    settings: &Detect,
) -> Vec<[f32; 2]> {
    let (w, h) = (img.width(), img.height());
    let edge = settings.border.max(BLOCK + 2);
    if settings.max == 0 || region.is_empty() || w <= 2 * edge || h <= 2 * edge {
        return Vec::new();
    }
    // The box of pixels that can hold a feature.
    let x0 = region.x0.max(edge);
    let y0 = region.y0.max(edge);
    let x1 = (region.x0 + region.width).min(w - edge);
    let y1 = (region.y0 + region.height).min(h - edge);
    if x1 <= x0 || y1 <= y0 {
        return Vec::new();
    }
    // Gradients (Sobel) over the box and a border of BLOCK.
    let (gx0, gy0) = (x0 - BLOCK, y0 - BLOCK);
    let (gw, gh) = (x1 - x0 + 2 * BLOCK, y1 - y0 + 2 * BLOCK);
    let mut xx = vec![0.0_f32; gw * gh];
    let mut xy = vec![0.0_f32; gw * gh];
    let mut yy = vec![0.0_f32; gw * gh];
    let data = img.data();
    for j in 0..gh {
        let y = gy0 + j;
        let (up, mid, down) = (&data[(y - 1) * w..], &data[y * w..], &data[(y + 1) * w..]);
        for i in 0..gw {
            let x = gx0 + i;
            let ix = (up[x + 1] - up[x - 1] + 2.0 * (mid[x + 1] - mid[x - 1]) + down[x + 1]
                - down[x - 1])
                / 8.0;
            let iy =
                (down[x - 1] - up[x - 1] + 2.0 * (down[x] - up[x]) + down[x + 1] - up[x + 1]) / 8.0;
            let k = j * gw + i;
            xx[k] = ix * ix;
            xy[k] = ix * iy;
            yy[k] = iy * iy;
        }
    }
    // Box sums across, then down, giving the smallest eigenvalue per pixel of the box.
    let (bw, bh) = (x1 - x0, y1 - y0);
    let side = 2 * BLOCK + 1;
    #[allow(clippy::cast_precision_loss, reason = "a small box")]
    let area = (side * side) as f32;
    let across = |src: &[f32]| -> Vec<f32> {
        let mut out = vec![0.0_f32; bw * gh];
        for j in 0..gh {
            let row = &src[j * gw..][..gw];
            let mut s: f32 = row[..side].iter().sum();
            out[j * bw] = s;
            for i in 1..bw {
                s += row[i + side - 1] - row[i - 1];
                out[j * bw + i] = s;
            }
        }
        out
    };
    let (axx, axy, ayy) = (across(&xx), across(&xy), across(&yy));
    let mut eig = vec![0.0_f32; bw * bh];
    let mut sums = vec![[0.0_f32; 3]; bw];
    for (i, s) in sums.iter_mut().enumerate() {
        for j in 0..side {
            let k = j * bw + i;
            *s = [s[0] + axx[k], s[1] + axy[k], s[2] + ayy[k]];
        }
    }
    let mut best = 0.0_f32;
    for j in 0..bh {
        if j > 0 {
            for (i, s) in sums.iter_mut().enumerate() {
                let (add, sub) = ((j + side - 1) * bw + i, (j - 1) * bw + i);
                *s = [
                    s[0] + axx[add] - axx[sub],
                    s[1] + axy[add] - axy[sub],
                    s[2] + ayy[add] - ayy[sub],
                ];
            }
        }
        for (i, s) in sums.iter().enumerate() {
            let (a, b, c) = (s[0] / area, s[1] / area, s[2] / area);
            let e = f32::midpoint(a, c) - (0.25 * (a - c) * (a - c) + b * b).sqrt();
            let inside = region.contains(x0 + i, y0 + j);
            let e = if inside { e } else { 0.0 };
            eig[j * bw + i] = e;
            best = best.max(e);
        }
    }
    let floor = (best * settings.quality).max(settings.min_eigen);
    if best < floor {
        return Vec::new();
    }
    // The local maxima above the floor, strongest first.
    let mut candidates: Vec<(f32, usize)> = Vec::new();
    for j in 0..bh {
        for i in 0..bw {
            let e = eig[j * bw + i];
            if e < floor {
                continue;
            }
            let mut peak = true;
            'around: for dj in -1_isize..=1 {
                for di in -1_isize..=1 {
                    let (Some(ni), Some(nj)) = (i.checked_add_signed(di), j.checked_add_signed(dj))
                    else {
                        continue;
                    };
                    if (di, dj) != (0, 0) && ni < bw && nj < bh && eig[nj * bw + ni] > e {
                        peak = false;
                        break 'around;
                    }
                }
            }
            if peak {
                candidates.push((e, j * bw + i));
            }
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    // Keep them apart, with a grid of cells as wide as the distance.
    let d = settings.min_distance.max(1.0);
    let d2 = d * d;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a frame of at most 2^24 samples a side"
    )]
    let (cw, ch) = (
        (w as f32 / d).ceil() as usize + 1,
        (h as f32 / d).ceil() as usize + 1,
    );
    let mut grid: Vec<Vec<[f32; 2]>> = vec![Vec::new(); cw * ch];
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "within the frame"
    )]
    let cell = |p: [f32; 2]| {
        ((p[0].max(0.0) / d) as usize).min(cw - 1) + ((p[1].max(0.0) / d) as usize).min(ch - 1) * cw
    };
    for &p in existing {
        grid[cell(p)].push(p);
    }
    let mut out = Vec::new();
    for (_, k) in candidates {
        if out.len() >= settings.max {
            break;
        }
        #[allow(clippy::cast_precision_loss, reason = "a place in a frame")]
        let p = [(x0 + k % bw) as f32, (y0 + k / bw) as f32];
        let c = cell(p);
        let (cx, cy) = (c % cw, c / cw);
        let near = (cy.saturating_sub(1)..=(cy + 1).min(ch - 1)).any(|gy| {
            (cx.saturating_sub(1)..=(cx + 1).min(cw - 1)).any(|gx| {
                grid[gy * cw + gx]
                    .iter()
                    .any(|q| (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) < d2)
            })
        });
        if !near {
            grid[c].push(p);
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::region::Region;

    #[test]
    fn the_corners_of_squares_are_found_inside_the_region_only() {
        let (w, h) = (96, 64);
        let mut data = vec![0.0_f32; w * h];
        for (sx, sy) in [(20, 20), (60, 20), (20, 44), (60, 44)] {
            for y in sy..sy + 10 {
                for x in sx..sx + 10 {
                    data[y * w + x] = 1.0;
                }
            }
        }
        let img = Luma::new(w, h, data).unwrap();
        let settings = Detect {
            max: 100,
            quality: 0.1,
            min_eigen: 1e-4,
            min_distance: 4.0,
            border: 3,
        };
        let left = Region::rect(0.0, 0.0, 45.0, 63.0).raster(w, h);
        let found = detect(&img, &left, &[], &settings);
        assert_eq!(found.len(), 8, "{found:?}");
        assert!(found.iter().all(|p| p[0] < 45.0));
        for p in &found {
            let near_corner = [20.0, 29.0].iter().any(|&cx| (p[0] - cx).abs() <= 2.0)
                && [20.0, 29.0, 44.0, 53.0]
                    .iter()
                    .any(|&cy| (p[1] - cy).abs() <= 2.0);
            assert!(near_corner, "{p:?}");
        }
        // Those already there keep the new ones away.
        let fewer = detect(&img, &left, &found[..4], &settings);
        assert_eq!(fewer.len(), 4);
        assert_eq!(detect(&img, &left, &[], &settings), found);
    }
}
