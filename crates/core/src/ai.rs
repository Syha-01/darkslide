//! "AI" masks without neural networks: classical detectors for subject, sky,
//! background and landscape classes. Results are alpha planes in the T frame
//! (uncropped, transformed). An ONNX backend can replace `detect` later.

use rayon::prelude::*;

use crate::color::{Oklab, lab_to_lch};
use crate::demosaic::Quality;
use crate::image::{Image3, Plane, box_blur, gaussian_blur, guided_filter};
use crate::params::{AiMaskKind, DevelopSettings};
use crate::pipeline::Engine;
use crate::pipeline::tone::smoothstep;

const SIZE: usize = 640;

/// Render a neutral view of the T frame for analysis.
fn analysis_view(engine: &Engine, s: &DevelopSettings) -> Image3 {
    let mut s2 = s.clone();
    s2.masks.clear();
    s2.vignette.amount = 0.0;
    s2.grain.amount = 0.0;
    s2.lens_blur.enabled = false;
    engine.render_fit(&s2, SIZE, SIZE, Quality::Draft, true).image
}

/// `hint` is an optional T-frame box [x0, y0, x1, y1] around the object.
pub fn detect(engine: &Engine, s: &DevelopSettings, kind: AiMaskKind, hint: Option<[f32; 4]>) -> Plane {
    detect_with_source(engine, s, kind, hint).0
}

/// Like `detect`, also saying whether an installed model did it (false: the
/// built-in colour and texture estimate).
pub fn detect_with_source(engine: &Engine, s: &DevelopSettings, kind: AiMaskKind, hint: Option<[f32; 4]>) -> (Plane, bool) {
    let img = analysis_view(engine, s);
    if let Some(p) = learned(&img, kind, hint) {
        return (p, true);
    }
    (estimate(&img, kind, hint), false)
}

fn estimate(img: &Image3, kind: AiMaskKind, hint: Option<[f32; 4]>) -> Plane {
    let ok = Oklab::new();
    let lab: Vec<[f32; 3]> = img.data.par_iter().map(|p| ok.from_prophoto(*p)).collect();
    let (w, h) = (img.w, img.h);
    let l = Plane { w, h, data: lab.iter().map(|v| v[0]).collect() };
    let tex = texture_map(&l);
    let raw = match kind {
        AiMaskKind::Sky => sky(&lab, &tex, w, h),
        AiMaskKind::Subject | AiMaskKind::Person | AiMaskKind::Object => subject(&lab, &l, w, h, kind == AiMaskKind::Person, hint),
        AiMaskKind::Background => subject(&lab, &l, w, h, false, None).map(|v| 1.0 - v),
        AiMaskKind::Water => class_map(&lab, &tex, w, h, |lch, t, y| {
            let hue = 1.0 - smoothstep(30.0, 60.0, crate::color::hue_diff(lch[2], 220.0).abs());
            hue * smoothstep(0.01, 0.04, lch[1]) * (1.0 - smoothstep(0.02, 0.08, t)) * smoothstep(0.3, 0.6, y)
        }),
        AiMaskKind::Vegetation => class_map(&lab, &tex, w, h, |lch, t, _| {
            let hue = 1.0 - smoothstep(35.0, 65.0, crate::color::hue_diff(lch[2], 125.0).abs());
            hue * smoothstep(0.02, 0.06, lch[1]) * smoothstep(0.005, 0.03, t)
        }),
        AiMaskKind::Snow => class_map(&lab, &tex, w, h, |lch, t, y| {
            smoothstep(0.75, 0.88, lch[0]) * (1.0 - smoothstep(0.02, 0.05, lch[1])) * (1.0 - smoothstep(0.03, 0.1, t)) * smoothstep(0.2, 0.5, y)
        }),
        AiMaskKind::NaturalGround => class_map(&lab, &tex, w, h, |lch, t, y| {
            let hue = 1.0 - smoothstep(30.0, 60.0, crate::color::hue_diff(lch[2], 70.0).abs());
            hue * smoothstep(0.015, 0.05, lch[1]) * smoothstep(0.45, 0.75, y) * smoothstep(0.003, 0.02, t)
        }),
        AiMaskKind::ArtificialGround => {
            class_map(&lab, &tex, w, h, |lch, t, y| (1.0 - smoothstep(0.02, 0.05, lch[1])) * smoothstep(0.55, 0.8, y) * (1.0 - smoothstep(0.03, 0.08, t)))
        }
        AiMaskKind::Mountains => {
            let sk = sky(&lab, &tex, w, h);
            // band just below the sky line, low saturation, distant (low contrast)
            let below = Plane::from_fn(w, h, |x, y| {
                let mut seen_sky = false;
                for yy in (0..y).rev().take(h / 3) {
                    if sk.get(x, yy) > 0.5 {
                        seen_sky = true;
                        break;
                    }
                }
                if seen_sky && sk.get(x, y) < 0.5 { 1.0 } else { 0.0 }
            });
            below.zip_map(&tex, |b, t| b * (1.0 - smoothstep(0.03, 0.09, t)))
        }
        AiMaskKind::Architecture => {
            // straight-edge density
            let gx = Plane::from_fn(w, h, |x, y| (l.get_clamped(x as isize + 1, y as isize) - l.get_clamped(x as isize - 1, y as isize)).abs());
            let gy = Plane::from_fn(w, h, |x, y| (l.get_clamped(x as isize, y as isize + 1) - l.get_clamped(x as isize, y as isize - 1)).abs());
            let axis = gx.zip_map(&gy, |a, b| {
                let m = a.max(b);
                let n = a.min(b);
                if m > 0.03 { (1.0 - n / m).powi(2) } else { 0.0 }
            });
            box_blur(&axis, 6).map(|v| smoothstep(0.05, 0.2, v))
        }
    };
    refine(&raw, &l)
}

/// Model-based masks when an ONNX model is installed (feature `ml`).
fn learned(img: &Image3, kind: AiMaskKind, hint: Option<[f32; 4]>) -> Option<Plane> {
    let l = img.luminance().map(|v| v.max(0.0).sqrt());
    let m = match kind {
        AiMaskKind::Sky => crate::ml::segment("sky.onnx", img)?,
        AiMaskKind::Background => crate::ml::segment("subject.onnx", img)?.map(|v| 1.0 - v),
        AiMaskKind::Subject | AiMaskKind::Person => crate::ml::segment("subject.onnx", img)?,
        AiMaskKind::Object => {
            let [x0, y0, x1, y1] = hint?;
            // segment the boxed region on its own for a tight object mask
            let (w, h) = (img.w as f32, img.h as f32);
            let (bx0, by0) = ((x0.min(x1) * w) as usize, (y0.min(y1) * h) as usize);
            let (bx1, by1) = (((x0.max(x1) * w) as usize).min(img.w), ((y0.max(y1) * h) as usize).min(img.h));
            if bx1 <= bx0 + 8 || by1 <= by0 + 8 {
                return None;
            }
            let crop = img.crop(bx0, by0, bx1 - bx0, by1 - by0);
            let seg = crate::ml::segment("subject.onnx", &crop)?;
            Plane::from_fn(img.w, img.h, |x, y| if x >= bx0 && x < bx1 && y >= by0 && y < by1 { seg.get(x - bx0, y - by0) } else { 0.0 })
        }
        _ => return None,
    };
    Some(refine(&m, &l))
}

fn texture_map(l: &Plane) -> Plane {
    let b = gaussian_blur(l, 2.0);
    let d = l.zip_map(&b, |a, b| (a - b).abs());
    gaussian_blur(&d, 3.0)
}

fn class_map(lab: &[[f32; 3]], tex: &Plane, w: usize, h: usize, f: impl Fn([f32; 3], f32, f32) -> f32 + Sync) -> Plane {
    Plane::from_fn(w, h, |x, y| {
        let i = y * w + x;
        f(lab_to_lch(lab[i]), tex.data[i], y as f32 / h as f32)
    })
}

/// Bright, blue-or-neutral, smooth regions connected to the top border.
fn sky(lab: &[[f32; 3]], tex: &Plane, w: usize, h: usize) -> Plane {
    let score = Plane::from_fn(w, h, |x, y| {
        let i = y * w + x;
        let lch = lab_to_lch(lab[i]);
        let bright = smoothstep(0.45, 0.7, lch[0]);
        let blue = 1.0 - smoothstep(40.0, 80.0, crate::color::hue_diff(lch[2], 250.0).abs());
        let neutral = 1.0 - smoothstep(0.02, 0.05, lch[1]);
        let colour = blue.max(neutral * 0.9).max(smoothstep(0.85, 0.95, lch[0]));
        let smooth = 1.0 - smoothstep(0.015, 0.06, tex.data[i]);
        let top = 1.0 - smoothstep(0.4, 0.9, y as f32 / h as f32);
        bright * colour * (0.4 + 0.6 * smooth) * (0.5 + 0.5 * top)
    });
    // flood fill from the top edge through good sky pixels
    let mut keep = vec![false; w * h];
    let mut stack: Vec<usize> = (0..w).filter(|&x| score.data[x] > 0.35).collect();
    for &i in &stack {
        keep[i] = true;
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        let ns = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)];
        for (nx, ny) in ns {
            if nx >= w || ny >= h {
                continue;
            }
            let j = ny * w + nx;
            if !keep[j] && score.data[j] > 0.3 {
                keep[j] = true;
                stack.push(j);
            }
        }
    }
    // sky seen through branches: keep high-score pixels near kept regions too
    let near = box_blur(&Plane { w, h, data: keep.iter().map(|&k| if k { 1.0 } else { 0.0 }).collect() }, 12);
    Plane::from_fn(w, h, |x, y| {
        let i = y * w + x;
        if keep[i] {
            1.0
        } else if near.data[i] > 0.25 && score.data[i] > 0.45 {
            score.data[i]
        } else {
            0.0
        }
    })
}

/// Saliency (colour contrast) + focus + centre prior, segmented and cleaned.
fn subject(lab: &[[f32; 3]], l: &Plane, w: usize, h: usize, person: bool, hint: Option<[f32; 4]>) -> Plane {
    let a = Plane { w, h, data: lab.iter().map(|v| v[1]).collect() };
    let b = Plane { w, h, data: lab.iter().map(|v| v[2]).collect() };
    // frequency-tuned saliency against a heavily blurred background estimate
    let big = (w.max(h) as f32 / 6.0).max(4.0);
    let (lb, ab, bb) = (gaussian_blur(l, big), gaussian_blur(&a, big), gaussian_blur(&b, big));
    let (ls, as_, bs) = (gaussian_blur(l, 1.5), gaussian_blur(&a, 1.5), gaussian_blur(&b, 1.5));
    let sal = Plane::from_fn(w, h, |x, y| {
        let i = y * w + x;
        ((ls.data[i] - lb.data[i]).powi(2) + 2.0 * (as_.data[i] - ab.data[i]).powi(2) + 2.0 * (bs.data[i] - bb.data[i]).powi(2)).sqrt()
    });
    // focus: local Laplacian energy (in-focus subjects vs bokeh backgrounds)
    let lap = Plane::from_fn(w, h, |x, y| {
        let c = l.get(x, y) * 4.0
            - l.get_clamped(x as isize - 1, y as isize)
            - l.get_clamped(x as isize + 1, y as isize)
            - l.get_clamped(x as isize, y as isize - 1)
            - l.get_clamped(x as isize, y as isize + 1);
        c.abs()
    });
    let focus = gaussian_blur(&lap, 6.0);
    let fmax = percentile(&focus.data, 0.98).max(1e-4);
    let smax = percentile(&sal.data, 0.98).max(1e-4);
    let mut score = Plane::from_fn(w, h, |x, y| {
        let i = y * w + x;
        let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
        let centre = match hint {
            // inside the user's box, falling off just outside it
            Some([x0, y0, x1, y1]) => {
                let (bx, by) = ((x1 - x0).abs().max(0.01), (y1 - y0).abs().max(0.01));
                let dx = ((x0.min(x1) - u).max(u - x0.max(x1)).max(0.0)) / bx;
                let dy = ((y0.min(y1) - v).max(v - y0.max(y1)).max(0.0)) / by;
                let cxn = (u - (x0 + x1) / 2.0) / bx * 2.0;
                let cyn = (v - (y0 + y1) / 2.0) / by * 2.0;
                (-(dx * dx + dy * dy) * 400.0).exp() * (0.6 + 0.4 * (-(cxn * cxn + cyn * cyn) * 0.5).exp())
            }
            None => {
                let cx = (u - 0.5) * 2.0;
                let cy = (v - 0.45) * 2.0;
                (-(cx * cx + cy * cy) * 1.2).exp()
            }
        };
        let mut s = 0.45 * (sal.data[i] / smax).min(1.0) + 0.35 * (focus.data[i] / fmax).min(1.0) + 0.2 * centre;
        if person {
            let lch = lab_to_lch(lab[i]);
            let skin = (1.0 - smoothstep(15.0, 30.0, crate::color::hue_diff(lch[2], 55.0).abs())) * smoothstep(0.02, 0.05, lch[1]);
            s = s * 0.7 + skin * 0.3;
        }
        if hint.is_some() { s * centre } else { s * (0.6 + 0.4 * centre) }
    });
    score = gaussian_blur(&score, 3.0);
    let t = otsu(&score.data);
    let bin: Vec<bool> = score.data.iter().map(|&v| v > t).collect();
    let comp = largest_components(&bin, w, h, 3);
    Plane { w, h, data: comp.iter().map(|&k| if k { 1.0 } else { 0.0 }).collect() }
}

fn percentile(v: &[f32], q: f32) -> f32 {
    let mut s: Vec<f32> = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s.get(((s.len() as f32 - 1.0) * q) as usize).copied().unwrap_or(0.0)
}

fn otsu(v: &[f32]) -> f32 {
    let max = v.iter().cloned().fold(0.0, f32::max).max(1e-6);
    let mut hist = [0u32; 256];
    for &x in v {
        hist[((x / max) * 255.0).clamp(0.0, 255.0) as usize] += 1;
    }
    let total = v.len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let (mut sb, mut wb, mut best, mut thr) = (0.0, 0.0, 0.0, 0usize);
    for (i, &c) in hist.iter().enumerate() {
        wb += c as f64;
        if wb == 0.0 {
            continue;
        }
        let wf = total - wb;
        if wf == 0.0 {
            break;
        }
        sb += i as f64 * c as f64;
        let mb = sb / wb;
        let mf = (sum - sb) / wf;
        let between = wb * wf * (mb - mf).powi(2);
        if between > best {
            best = between;
            thr = i;
        }
    }
    thr as f32 / 255.0 * max
}

/// Keep the `n` largest connected components (4-connectivity), filling holes.
fn largest_components(bin: &[bool], w: usize, h: usize, n: usize) -> Vec<bool> {
    let mut label = vec![0u32; w * h];
    let mut sizes = vec![0usize];
    let mut next = 1u32;
    for i in 0..w * h {
        if !bin[i] || label[i] != 0 {
            continue;
        }
        let mut stack = vec![i];
        label[i] = next;
        let mut size = 0;
        while let Some(j) = stack.pop() {
            size += 1;
            let (x, y) = (j % w, j / w);
            for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                if nx < w && ny < h {
                    let k = ny * w + nx;
                    if bin[k] && label[k] == 0 {
                        label[k] = next;
                        stack.push(k);
                    }
                }
            }
        }
        sizes.push(size);
        next += 1;
    }
    let mut order: Vec<usize> = (1..sizes.len()).collect();
    order.sort_by(|a, b| sizes[*b].cmp(&sizes[*a]));
    let biggest = sizes.get(order.first().copied().unwrap_or(0)).copied().unwrap_or(0);
    let keep: Vec<u32> = order.iter().take(n).filter(|&&k| sizes[k] * 6 >= biggest).map(|&k| k as u32).collect();
    let fg: Vec<bool> = label.iter().map(|l| keep.contains(l)).collect();
    // fill holes: background not reachable from the border becomes foreground
    let mut outside = vec![false; w * h];
    let mut stack: Vec<usize> =
        (0..w).chain((0..w).map(|x| (h - 1) * w + x)).chain((0..h).map(|y| y * w)).chain((0..h).map(|y| y * w + w - 1)).filter(|&i| !fg[i]).collect();
    for &i in &stack {
        outside[i] = true;
    }
    while let Some(j) = stack.pop() {
        let (x, y) = (j % w, j / w);
        for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
            if nx < w && ny < h {
                let k = ny * w + nx;
                if !fg[k] && !outside[k] {
                    outside[k] = true;
                    stack.push(k);
                }
            }
        }
    }
    outside.iter().map(|o| !o).collect()
}

/// Edge-aware refinement guided by luminance.
fn refine(m: &Plane, l: &Plane) -> Plane {
    let r = (m.w.max(m.h) / 120).max(2);
    guided_filter(l, m, r, 1e-3).map(|v| v.clamp(0.0, 1.0))
}

/// Approximate relative depth (0 near .. 1 far) for depth-range masks and
/// lens blur: defocus + vertical position + haze cues.
pub fn estimate_depth(engine: &Engine, s: &DevelopSettings) -> Plane {
    let img = analysis_view(engine, s);
    if let Some(d) = crate::ml::depth(&img) {
        let l = img.luminance().map(|v| v.max(0.0).sqrt());
        return refine(&d, &l);
    }
    let l = img.luminance().map(|v| v.max(0.0).sqrt());
    let (w, h) = (l.w, l.h);
    let lap = Plane::from_fn(w, h, |x, y| {
        let c = l.get(x, y) * 4.0
            - l.get_clamped(x as isize - 1, y as isize)
            - l.get_clamped(x as isize + 1, y as isize)
            - l.get_clamped(x as isize, y as isize - 1)
            - l.get_clamped(x as isize, y as isize + 1);
        c.abs()
    });
    let focus = gaussian_blur(&lap, 8.0);
    let fmax = percentile(&focus.data, 0.98).max(1e-4);
    let raw = Plane::from_fn(w, h, |x, y| {
        let f = (focus.get(x, y) / fmax).min(1.0);
        let vert = 1.0 - y as f32 / h as f32;
        (0.6 * (1.0 - f) + 0.4 * vert).clamp(0.0, 1.0)
    });
    refine(&gaussian_blur(&raw, 4.0), &l)
}
