//! Exemplar-based inpainting (Criminisi et al.): the hole is filled from its
//! boundary inward by copying whole patches from the known image, highest
//! priority first. Copying patches (rather than averaging per pixel) keeps
//! texture coherent. Used by content-aware Remove and crop expansion.

use rayon::prelude::*;

use crate::image::{Image3, Plane, Rgb};

const PR: isize = 4; // patch radius (9x9)

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

#[inline]
fn sq(p: Rgb) -> Rgb {
    p.map(|v| v.max(0.0).sqrt())
}

/// Fill pixels where `hole` > 0.5 using content from the rest of `img`.
pub fn inpaint(img: &Image3, hole: &Plane) -> Image3 {
    let (w, h) = (img.w, img.h);
    let mut known: Vec<bool> = hole.data.iter().map(|&v| v <= 0.5).collect();
    if known.iter().all(|&k| k) || !known.iter().any(|&k| k) {
        return img.clone();
    }
    let mut out = img.clone();
    // perceptual copy used for matching
    let mut lum: Vec<Rgb> = img.data.par_iter().map(|p| sq(*p)).collect();
    let mut conf: Vec<f32> = known.iter().map(|&k| if k { 1.0 } else { 0.0 }).collect();
    // source offset each filled pixel was copied from (for coherence candidates)
    let mut offs: Vec<Option<(i32, i32)>> = vec![None; w * h];
    // a source patch must lie fully inside originally known pixels
    let orig_known = known.clone();
    let valid_src: Vec<bool> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            if x < PR || y < PR || x >= w as isize - PR || y >= h as isize - PR {
                return false;
            }
            for dy in -PR..=PR {
                for dx in -PR..=PR {
                    if !orig_known[((y + dy) as usize) * w + (x + dx) as usize] {
                        return false;
                    }
                }
            }
            true
        })
        .collect();
    let sources: Vec<usize> = (0..w * h).filter(|&i| valid_src[i]).collect();
    if sources.is_empty() {
        return diffuse_fill(img, hole);
    }
    // search near the hole first: its bounding box grown by 3x
    let (mut bx0, mut by0, mut bx1, mut by1) = (w, h, 0, 0);
    for i in 0..w * h {
        if !known[i] {
            let (x, y) = (i % w, i / w);
            bx0 = bx0.min(x);
            by0 = by0.min(y);
            bx1 = bx1.max(x);
            by1 = by1.max(y);
        }
    }
    let (hw, hh) = (bx1 - bx0 + 1, by1 - by0 + 1);
    let near: Vec<usize> = sources
        .iter()
        .copied()
        .filter(|&i| {
            let (x, y) = (i % w, i / w);
            x + 3 * hw >= bx0 && x <= bx1 + 3 * hw && y + 3 * hh >= by0 && y <= by1 + 3 * hh
        })
        .collect();
    let pool: &[usize] = if near.len() > 200 { &near } else { &sources };

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut remaining = known.iter().filter(|&&k| !k).count();
    let mut guard = 0;
    while remaining > 0 && guard < 200_000 {
        guard += 1;
        // fill front: unknown pixels with a known 4-neighbour
        let front: Vec<usize> = (0..w * h)
            .into_par_iter()
            .filter(|&i| {
                if known[i] {
                    return false;
                }
                let (x, y) = (i % w, i / w);
                (x > 0 && known[i - 1]) || (x + 1 < w && known[i + 1]) || (y > 0 && known[i - w]) || (y + 1 < h && known[i + w])
            })
            .collect();
        if front.is_empty() {
            break;
        }
        let grad = |x: isize, y: isize| -> f32 {
            let at = |xx: isize, yy: isize| -> Option<f32> {
                if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                    return None;
                }
                let j = yy as usize * w + xx as usize;
                known[j].then(|| lum[j][0] + lum[j][1] + lum[j][2])
            };
            let mut g: f32 = 0.0;
            for (dx, dy) in [(1, 0), (0, 1)] {
                if let (Some(a), Some(b)) = (at(x + dx, y + dy), at(x - dx, y - dy)) {
                    g = g.max((a - b).abs());
                }
            }
            g
        };
        // priority = confidence x structure, so edges propagate first
        let mut prio: Vec<(f32, usize)> = front
            .par_iter()
            .map(|&i| {
                let (x, y) = ((i % w) as isize, (i / w) as isize);
                let mut c = 0.0;
                let mut g: f32 = 0.0;
                for dy in -PR..=PR {
                    for dx in -PR..=PR {
                        let (xx, yy) = (x + dx, y + dy);
                        if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                            continue;
                        }
                        let j = yy as usize * w + xx as usize;
                        c += conf[j];
                        if known[j] {
                            g = g.max(grad(xx, yy));
                        }
                    }
                }
                let n = ((2 * PR + 1) * (2 * PR + 1)) as f32;
                ((c / n) * (0.2 + g), i)
            })
            .collect();
        prio.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        // several well-separated targets per pass for speed
        let mut picked: Vec<usize> = vec![];
        for &(_, i) in &prio {
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            if picked.iter().all(|&p| {
                let (px, py) = ((p % w) as isize, (p / w) as isize);
                (px - x).abs() > 2 * PR + 1 || (py - y).abs() > 2 * PR + 1
            }) {
                picked.push(i);
            }
            if picked.len() >= 16 {
                break;
            }
        }
        let seeds: Vec<u64> = picked.iter().map(|_| rng.next()).collect();
        let best: Vec<(usize, usize)> = picked
            .par_iter()
            .zip(seeds.par_iter())
            .map(|(&t, &seed)| {
                let (tx, ty) = ((t % w) as isize, (t / w) as isize);
                // SSD over the target patch's known pixels
                let dist = |s: usize, cutoff: f32| -> f32 {
                    let (sx, sy) = ((s % w) as isize, (s / w) as isize);
                    let mut d = 0.0;
                    let mut n = 0.0;
                    for dy in -PR..=PR {
                        let yy = ty + dy;
                        if yy < 0 || yy >= h as isize {
                            continue;
                        }
                        for dx in -PR..=PR {
                            let xx = tx + dx;
                            if xx < 0 || xx >= w as isize {
                                continue;
                            }
                            let j = yy as usize * w + xx as usize;
                            if !known[j] {
                                continue;
                            }
                            let a = lum[j];
                            let b = lum[((sy + dy) as usize) * w + (sx + dx) as usize];
                            d += (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
                            n += 1.0;
                        }
                        if d > cutoff * 81.0 {
                            return f32::MAX;
                        }
                    }
                    if n > 0.0 { d / n } else { f32::MAX }
                };
                let mut r = Rng(seed | 1);
                let mut best = (f32::MAX, pool[0]);
                let consider = |s: usize, best: &mut (f32, usize)| {
                    let d = dist(s, best.0);
                    if d < best.0 {
                        *best = (d, s);
                    }
                };
                // coherence: continue the copy offsets of filled neighbours
                for dy in -PR..=PR {
                    for dx in -PR..=PR {
                        let (xx, yy) = (tx + dx, ty + dy);
                        if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                            continue;
                        }
                        if let Some((ox, oy)) = offs[yy as usize * w + xx as usize] {
                            let (sx, sy) = (tx + ox as isize, ty + oy as isize);
                            if sx >= 0 && sy >= 0 && sx < w as isize && sy < h as isize {
                                let s = sy as usize * w + sx as usize;
                                if valid_src[s] {
                                    consider(s, &mut best);
                                }
                            }
                        }
                    }
                }
                // random sampling of the pool, then local refinement
                for _ in 0..400.min(pool.len()) {
                    consider(pool[r.range(pool.len())], &mut best);
                }
                let mut radius = (hw.max(hh) as isize).max(8);
                while radius >= 1 {
                    for _ in 0..4 {
                        let (bx, by) = ((best.1 % w) as isize, (best.1 / w) as isize);
                        let sx = bx + r.range((2 * radius + 1) as usize) as isize - radius;
                        let sy = by + r.range((2 * radius + 1) as usize) as isize - radius;
                        if sx >= 0 && sy >= 0 && sx < w as isize && sy < h as isize {
                            let s = sy as usize * w + sx as usize;
                            if valid_src[s] {
                                consider(s, &mut best);
                            }
                        }
                    }
                    radius /= 2;
                }
                (t, best.1)
            })
            .collect();
        // copy the unknown part of each chosen source patch
        for (t, s) in best {
            let (tx, ty) = ((t % w) as isize, (t / w) as isize);
            let (sx, sy) = ((s % w) as isize, (s / w) as isize);
            let mut c = 0.0f32;
            let mut n = 0.0f32;
            for dy in -PR..=PR {
                for dx in -PR..=PR {
                    let (xx, yy) = (tx + dx, ty + dy);
                    if xx >= 0 && yy >= 0 && xx < w as isize && yy < h as isize {
                        c += conf[yy as usize * w + xx as usize];
                        n += 1.0;
                    }
                }
            }
            let c = c / n.max(1.0);
            for dy in -PR..=PR {
                for dx in -PR..=PR {
                    let (xx, yy) = (tx + dx, ty + dy);
                    if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                        continue;
                    }
                    let j = yy as usize * w + xx as usize;
                    if known[j] {
                        continue;
                    }
                    let si = ((sy + dy) as usize) * w + (sx + dx) as usize;
                    out.data[j] = img.data[si];
                    lum[j] = lum[si];
                    conf[j] = c;
                    offs[j] = Some(((sx - tx) as i32, (sy - ty) as i32));
                    known[j] = true;
                    remaining -= 1;
                }
            }
        }
    }
    out
}

/// Iterative neighbour averaging into the hole; a smooth fallback fill.
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
        let img = Image3::from_fn(64, 64, |_, _| [0.2, 0.3, 0.4]);
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

    #[test]
    fn continues_stripes() {
        // vertical stripes must continue through the hole, not blur
        let img = Image3::from_fn(96, 96, |x, _| if (x / 6) % 2 == 0 { [0.8; 3] } else { [0.1; 3] });
        let hole = Plane::from_fn(96, 96, |x, y| if (40..56).contains(&x) && (30..70).contains(&y) { 1.0 } else { 0.0 });
        let mut broken = img.clone();
        for y in 30..70 {
            for x in 40..56 {
                broken.data[y * 96 + x] = [0.5; 3];
            }
        }
        let out = inpaint(&broken, &hole);
        let err: f32 =
            (30..70).flat_map(|y| (40..56).map(move |x| (x, y))).map(|(x, y)| (out.get(x, y)[0] - img.get(x, y)[0]).abs()).sum::<f32>() / (40.0 * 16.0);
        // a flat fill scores about 0.35 here
        assert!(err < 0.2, "mean error {err}");
    }
}
