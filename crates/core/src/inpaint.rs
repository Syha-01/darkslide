//! Exemplar-based inpainting: multi-scale PatchMatch with patch voting
//! (Wexler/Barnes style). Used by content-aware Remove and crop expansion.

use rayon::prelude::*;

use crate::image::{Image3, Plane, Rgb};

const PR: isize = 3; // patch radius (7x7)

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn downscale_mask(m: &Plane) -> Plane {
    let (w, h) = (m.w.div_ceil(2), m.h.div_ceil(2));
    Plane::from_fn(w, h, |x, y| {
        let mut v: f32 = 0.0;
        for dy in 0..2 {
            for dx in 0..2 {
                v = v.max(m.get_clamped((x * 2 + dx) as isize, (y * 2 + dy) as isize));
            }
        }
        v
    })
}

fn downscale_img(img: &Image3, mask: &Plane) -> Image3 {
    let (w, h) = (img.w.div_ceil(2), img.h.div_ceil(2));
    Image3::from_fn(w, h, |x, y| {
        let mut acc = [0.0f32; 3];
        let mut n = 0.0;
        for dy in 0..2 {
            for dx in 0..2 {
                let (sx, sy) = ((x * 2 + dx).min(img.w - 1), (y * 2 + dy).min(img.h - 1));
                if mask.get(sx, sy) < 0.5 {
                    let p = img.get(sx, sy);
                    acc[0] += p[0];
                    acc[1] += p[1];
                    acc[2] += p[2];
                    n += 1.0;
                }
            }
        }
        if n > 0.0 { acc.map(|v| v / n) } else { img.get((x * 2).min(img.w - 1), (y * 2).min(img.h - 1)) }
    })
}

/// Fill pixels where `hole` > 0.5 using content from the rest of `img`.
pub fn inpaint(img: &Image3, hole: &Plane) -> Image3 {
    if !hole.data.iter().any(|&v| v > 0.5) {
        return img.clone();
    }
    // build pyramid until the hole is small
    let mut imgs = vec![img.clone()];
    let mut masks = vec![hole.clone()];
    loop {
        let m = masks.last().unwrap();
        let hole_px = m.data.iter().filter(|&&v| v > 0.5).count();
        let i = imgs.last().unwrap();
        if hole_px < 600 || i.w < 48 || i.h < 48 {
            break;
        }
        let nm = downscale_mask(m);
        let ni = downscale_img(i, m);
        imgs.push(ni);
        masks.push(nm);
    }
    let levels = imgs.len();
    // coarsest init: diffuse known colours into the hole
    let mut est = diffuse_fill(&imgs[levels - 1], &masks[levels - 1]);
    let mut nnf: Option<Vec<(i32, i32)>> = None;
    for lvl in (0..levels).rev() {
        let m = &masks[lvl];
        let orig = &imgs[lvl];
        if lvl != levels - 1 {
            // upsample estimate into the hole
            let up = est.resize(orig.w, orig.h);
            est = Image3::from_fn(orig.w, orig.h, |x, y| if m.get(x, y) > 0.5 { up.get(x, y) } else { orig.get(x, y) });
            nnf = nnf.map(|f| upscale_nnf(&f, &masks[lvl + 1], m));
        }
        let iters = if lvl == 0 { 2 } else { 4 };
        let mut field = nnf.take().unwrap_or_else(|| init_nnf(m, lvl as u64 + 1));
        for it in 0..iters {
            patchmatch(&est, m, &mut field, 4, (lvl * 31 + it) as u64 + 7);
            est = vote(&est, orig, m, &field);
        }
        nnf = Some(field);
    }
    est
}

fn is_valid_src(m: &Plane, x: isize, y: isize) -> bool {
    if x < PR || y < PR || x >= m.w as isize - PR || y >= m.h as isize - PR {
        return false;
    }
    m.get(x as usize, y as usize) < 0.5
}

fn random_valid(m: &Plane, rng: &mut Rng) -> (i32, i32) {
    for _ in 0..1000 {
        let x = rng.range(m.w) as isize;
        let y = rng.range(m.h) as isize;
        if is_valid_src(m, x, y) {
            return (x as i32, y as i32);
        }
    }
    (PR as i32, PR as i32)
}

fn init_nnf(m: &Plane, seed: u64) -> Vec<(i32, i32)> {
    let mut rng = Rng(0x9E3779B97F4A7C15 ^ seed);
    (0..m.w * m.h).map(|i| if m.data[i] > 0.5 { random_valid(m, &mut rng) } else { ((i % m.w) as i32, (i / m.w) as i32) }).collect()
}

fn upscale_nnf(f: &[(i32, i32)], small: &Plane, m: &Plane) -> Vec<(i32, i32)> {
    let mut rng = Rng(12345);
    (0..m.w * m.h)
        .map(|i| {
            let (x, y) = (i % m.w, i / m.w);
            if m.data[i] <= 0.5 {
                return (x as i32, y as i32);
            }
            let sx = (x / 2).min(small.w - 1);
            let sy = (y / 2).min(small.h - 1);
            let (tx, ty) = f[sy * small.w + sx];
            let c = ((tx * 2) + (x % 2) as i32, (ty * 2) + (y % 2) as i32);
            if is_valid_src(m, c.0 as isize, c.1 as isize) { c } else { random_valid(m, &mut rng) }
        })
        .collect()
}

#[inline]
fn patch_dist(img: &Image3, ax: isize, ay: isize, bx: isize, by: isize, cutoff: f32) -> f32 {
    let mut d = 0.0;
    for dy in -PR..=PR {
        for dx in -PR..=PR {
            let p = img.get_clamped(ax + dx, ay + dy);
            let q = img.get_clamped(bx + dx, by + dy);
            for c in 0..3 {
                let e = p[c].max(0.0).sqrt() - q[c].max(0.0).sqrt();
                d += e * e;
            }
        }
        if d > cutoff {
            return d;
        }
    }
    d
}

fn patchmatch(img: &Image3, m: &Plane, f: &mut [(i32, i32)], iters: usize, seed: u64) {
    let (w, h) = (m.w, m.h);
    let hole: Vec<usize> = (0..w * h).filter(|&i| m.data[i] > 0.5).collect();
    let mut cost: Vec<f32> = hole
        .iter()
        .map(|&i| {
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            let (tx, ty) = f[i];
            patch_dist(img, x, y, tx as isize, ty as isize, f32::MAX)
        })
        .collect();
    let pos: std::collections::HashMap<usize, usize> = hole.iter().enumerate().map(|(k, &i)| (i, k)).collect();
    let mut rng = Rng(seed.wrapping_mul(0x2545F4914F6CDD1D) | 1);
    for it in 0..iters {
        let order: Box<dyn Iterator<Item = usize>> = if it % 2 == 0 { Box::new(0..hole.len()) } else { Box::new((0..hole.len()).rev()) };
        let dir: isize = if it % 2 == 0 { 1 } else { -1 };
        for k in order {
            let i = hole[k];
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            let mut best = f[i];
            let mut bc = cost[k];
            // propagation from left/up (or right/down)
            for (nx, ny) in [(x - dir, y), (x, y - dir)] {
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let ni = ny as usize * w + nx as usize;
                let (tx, ty) = f[ni];
                let cand = (tx as isize + (x - nx), ty as isize + (y - ny));
                if is_valid_src(m, cand.0, cand.1) {
                    let c = patch_dist(img, x, y, cand.0, cand.1, bc);
                    if c < bc {
                        bc = c;
                        best = (cand.0 as i32, cand.1 as i32);
                    }
                }
            }
            // random search
            let mut radius = w.max(h) as isize;
            while radius >= 1 {
                let cx = best.0 as isize + (rng.range((2 * radius + 1) as usize) as isize - radius);
                let cy = best.1 as isize + (rng.range((2 * radius + 1) as usize) as isize - radius);
                if is_valid_src(m, cx, cy) {
                    let c = patch_dist(img, x, y, cx, cy, bc);
                    if c < bc {
                        bc = c;
                        best = (cx as i32, cy as i32);
                    }
                }
                radius /= 2;
            }
            f[i] = best;
            cost[k] = bc;
        }
    }
    let _ = pos;
}

fn vote(est: &Image3, orig: &Image3, m: &Plane, f: &[(i32, i32)]) -> Image3 {
    let (w, h) = (m.w, m.h);
    Image3::from_fn(w, h, |x, y| {
        let i = y * w + x;
        if m.data[i] <= 0.5 {
            return orig.data[i];
        }
        let mut acc: Rgb = [0.0; 3];
        let mut wsum = 0.0;
        // every patch covering (x,y) votes with its corresponding pixel
        for dy in -PR..=PR {
            for dx in -PR..=PR {
                let (px, py) = (x as isize + dx, y as isize + dy);
                if px < 0 || py < 0 || px >= w as isize || py >= h as isize {
                    continue;
                }
                let j = py as usize * w + px as usize;
                if m.data[j] <= 0.5 {
                    continue;
                }
                let (tx, ty) = f[j];
                let s = est.get_clamped(tx as isize - dx, ty as isize - dy);
                acc[0] += s[0];
                acc[1] += s[1];
                acc[2] += s[2];
                wsum += 1.0;
            }
        }
        if wsum > 0.0 { acc.map(|v| v / wsum) } else { est.data[i] }
    })
}

/// Iterative neighbour averaging into the hole; a smooth initial guess.
pub fn diffuse_fill(img: &Image3, m: &Plane) -> Image3 {
    let (w, h) = (img.w, img.h);
    let mut cur = img.clone();
    let mut known: Vec<bool> = m.data.iter().map(|&v| v <= 0.5).collect();
    for _ in 0..(w.max(h)) {
        if known.iter().all(|&k| k) {
            break;
        }
        let snapshot = cur.clone();
        let kn = known.clone();
        let updates: Vec<(usize, Rgb)> = (0..w * h)
            .into_par_iter()
            .filter(|&i| !kn[i])
            .filter_map(|i| {
                let (x, y) = (i % w, i / w);
                let mut acc = [0.0f32; 3];
                let mut n = 0.0;
                for (dx, dy) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if kn[j] {
                        let p = snapshot.data[j];
                        acc[0] += p[0];
                        acc[1] += p[1];
                        acc[2] += p[2];
                        n += 1.0;
                    }
                }
                if n > 0.0 { Some((i, acc.map(|v| v / n))) } else { None }
            })
            .collect();
        if updates.is_empty() {
            break;
        }
        for (i, p) in updates {
            cur.data[i] = p;
            known[i] = true;
        }
    }
    cur
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_flat_region() {
        let img = Image3::from_fn(64, 64, |x, _| if x < 32 { [0.2, 0.3, 0.4] } else { [0.2, 0.3, 0.4] });
        let mut broken = img.clone();
        let hole = Plane::from_fn(64, 64, |x, y| if (20..40).contains(&x) && (20..40).contains(&y) { 1.0 } else { 0.0 });
        for y in 20..40 {
            for x in 20..40 {
                broken.data[y * 64 + x] = [1.0, 0.0, 0.0];
            }
        }
        let out = inpaint(&broken, &hole);
        let p = out.get(30, 30);
        assert!((p[0] - 0.2).abs() < 0.02 && (p[2] - 0.4).abs() < 0.02, "{p:?}");
    }
}
