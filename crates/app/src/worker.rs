//! Background render thread. Jobs are coalesced: only the newest job of each
//! kind is rendered.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use darkslide_core::analysis::{self, Histogram};
use darkslide_core::demosaic::Quality;
use darkslide_core::image::Plane;
use darkslide_core::params::DevelopSettings;
use darkslide_core::pipeline::geometry::Region;
use darkslide_core::pipeline::{Engine, RenderRequest, to_srgb8};
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Whole frame fitted to the view.
    Fit,
    /// Visible region when zoomed in.
    Detail,
    /// "Before" rendering for before/after views.
    Before,
}

#[derive(Clone)]
pub struct Job {
    pub doc: u64,
    pub kind: Kind,
    pub engine: Arc<Engine>,
    pub settings: DevelopSettings,
    pub region: Region,
    pub ignore_crop: bool,
    pub clip_shadows: bool,
    pub clip_highlights: bool,
    pub interactive: bool,
    /// Mask group to visualise as an overlay.
    pub mask_overlay: Option<usize>,
    pub want_scope: bool,
    pub alt_preview: Option<crate::widgets::AltPreview>,
    /// Point colour swatch to visualise (index into settings.point_colors).
    pub point_range: Option<usize>,
}

pub struct Output {
    pub doc: u64,
    pub kind: Kind,
    pub rgba: Vec<u8>,
    pub w: usize,
    pub h: usize,
    pub region: Region,
    pub ignore_crop: bool,
    pub hist: Option<Histogram>,
    pub scope: Option<Vec<u32>>,
    pub mask: Option<Plane>,
    pub ms: u128,
}

pub struct Worker {
    tx: Sender<Job>,
    pub rx: Receiver<Output>,
}

pub const SCOPE_SIZE: usize = 96;

impl Worker {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, jrx) = channel::<Job>();
        let (otx, rx) = channel::<Output>();
        std::thread::Builder::new().name("render".into()).spawn(move || run(jrx, otx, ctx)).expect("spawn render thread");
        Self { tx, rx }
    }

    pub fn submit(&self, job: Job) {
        let _ = self.tx.send(job);
    }
}

fn run(rx: Receiver<Job>, tx: Sender<Output>, ctx: egui::Context) {
    let mut pending: Vec<Job> = vec![];
    loop {
        if pending.is_empty() {
            match rx.recv() {
                Ok(j) => pending.push(j),
                Err(_) => return,
            }
        }
        while let Ok(j) = rx.try_recv() {
            pending.push(j);
        }
        // newest per kind; Fit first so the whole image updates quickly
        let mut latest: Vec<Job> = vec![];
        for j in pending.drain(..) {
            latest.retain(|l| l.kind != j.kind);
            latest.push(j);
        }
        latest.sort_by_key(|j| match j.kind {
            Kind::Fit => 0,
            Kind::Detail => 1,
            Kind::Before => 2,
        });
        let job = latest.remove(0);
        pending = latest;
        let out = render(&job);
        if tx.send(out).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

fn render(job: &Job) -> Output {
    let t = std::time::Instant::now();
    let mut region = job.region;
    // fast low-res pass while dragging a slider
    if job.interactive && job.kind == Kind::Fit {
        let f = (900.0 / region.w.max(region.h) as f64).min(1.0);
        region.w = ((region.w as f64 * f) as usize).max(1);
        region.h = ((region.h as f64 * f) as usize).max(1);
    }
    let req = RenderRequest { region, quality: Quality::Draft, ignore_crop: job.ignore_crop };
    let r = job.engine.render(&job.settings, &req);
    let shown = if job.settings.hdr.enabled { darkslide_core::pipeline::hdr_to_sdr(&r.image, &job.settings.hdr) } else { r.image.clone() };
    let mut rgba = to_srgb8(&shown, Some(&r.alpha), [40, 40, 40]);
    if job.settings.hdr.enabled && job.settings.hdr.visualize {
        // tint pixels that use HDR headroom (brighter = further above SDR white)
        rgba.chunks_mut(4).zip(r.image.data.iter()).for_each(|(px, p)| {
            let y = darkslide_core::color::luma_prophoto(*p);
            if y > 1.0 {
                let t = ((y.log2()) / job.settings.hdr.max_headroom).clamp(0.0, 1.0);
                px[0] = (px[0] as f32 * (1.0 - t) + 255.0 * t) as u8;
                px[1] = (px[1] as f32 * (1.0 - t) + 120.0 * t) as u8;
                px[2] = (px[2] as f32 * (1.0 - t)) as u8;
            }
        });
    }
    let hist = if job.kind == Kind::Fit { Some(Histogram::from_rgba(&rgba)) } else { None };
    let scope = if job.want_scope && job.kind == Kind::Fit { Some(analysis::vectorscope(&rgba, SCOPE_SIZE)) } else { None };
    if job.clip_shadows || job.clip_highlights {
        analysis::clipping_overlay(&mut rgba, job.clip_shadows, job.clip_highlights);
    }
    match job.alt_preview {
        // ACR-style threshold views: black (white) background, clipped channels shown
        Some(crate::widgets::AltPreview::Highlights) => rgba.chunks_mut(4).for_each(|p| {
            for c in 0..3 {
                p[c] = if p[c] >= 254 { 255 } else { 0 };
            }
        }),
        Some(crate::widgets::AltPreview::Shadows) => rgba.chunks_mut(4).for_each(|p| {
            for c in 0..3 {
                p[c] = if p[c] <= 1 { 0 } else { 255 };
            }
        }),
        Some(crate::widgets::AltPreview::SharpenMask) => {
            let m = job.settings.sharpen.masking / 100.0 * 0.06;
            let (w, h) = (region.w, region.h);
            let y: Vec<f32> = rgba.chunks(4).map(|p| (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0).collect();
            let out: Vec<u8> = (0..w * h)
                .map(|i| {
                    let (x, yy) = (i % w, i / w);
                    let g = |dx: isize, dy: isize| {
                        y[((yy as isize + dy).clamp(0, h as isize - 1) as usize) * w + (x as isize + dx).clamp(0, w as isize - 1) as usize]
                    };
                    let gm = ((g(1, 0) - g(-1, 0)).powi(2) + (g(0, 1) - g(0, -1)).powi(2)).sqrt();
                    let v = darkslide_core::pipeline::tone::smoothstep(m * 0.5, m * 1.5 + 1e-4, gm);
                    (v * 255.0) as u8
                })
                .collect();
            rgba.chunks_mut(4).zip(out).for_each(|(p, v)| {
                p[0] = v;
                p[1] = v;
                p[2] = v;
            });
        }
        None => {}
    }
    if let Some(pi) = job.point_range {
        if let Some(pc) = job.settings.point_colors.get(pi) {
            // grey everything outside the swatch's range
            let ok = darkslide_core::color::Oklab::new();
            let to = darkslide_core::color::to_prophoto(darkslide_core::color::ColorSpace::SRgb);
            let range = pc.range / 50.0;
            let hr = (8.0 + pc.hue_range / 100.0 * 40.0) * range;
            rgba.chunks_mut(4).for_each(|p| {
                let lin = [p[0], p[1], p[2]].map(|v| darkslide_core::color::srgb_decode(v as f32 / 255.0));
                let lch = darkslide_core::color::lab_to_lch(ok.from_prophoto(darkslide_core::color::mul_mv(&to, lin)));
                let inside = darkslide_core::color::hue_diff(lch[2], pc.sample[2]).abs() < hr && lch[1] > 0.02;
                if !inside {
                    let g = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) as u8;
                    p[0] = g;
                    p[1] = g;
                    p[2] = g;
                }
            });
        }
    }
    let mask = job.mask_overlay.and_then(|mi| {
        let s = &job.settings;
        let g = s.masks.get(mi)?;
        let (fw, fh) = job.engine.full_size();
        let mut geo = job.engine.geometry(s, fw, fh, job.ignore_crop);
        if job.ignore_crop {
            geo = geo.without_crop();
        }
        let lab = vec![[0.5f32, 0.0, 0.0]; r.tco.len()];
        // range masks need colours: approximate from the output
        let ok = darkslide_core::color::Oklab::new();
        let lab: Vec<[f32; 3]> = if g.components.iter().any(|c| {
            matches!(c.shape, darkslide_core::params::MaskShape::ColorRange { .. } | darkslide_core::params::MaskShape::LuminanceRange { .. })
                || matches!(&c.shape, darkslide_core::params::MaskShape::Brush { strokes } if strokes.iter().any(|s| s.auto_mask))
        }) {
            r.image.data.iter().map(|p| ok.from_prophoto(*p)).collect()
        } else {
            lab
        };
        let depth = job.engine.depth();
        let env = darkslide_core::pipeline::local::MaskEnv { geo: &geo, region: &region, tco: &r.tco, lab: &lab, depth: depth.as_deref() };
        Some(darkslide_core::pipeline::local::eval_group(g, &env))
    });
    Output {
        doc: job.doc,
        kind: job.kind,
        w: region.w,
        h: region.h,
        rgba,
        region,
        ignore_crop: job.ignore_crop,
        hist,
        scope,
        mask,
        ms: t.elapsed().as_millis(),
    }
}
