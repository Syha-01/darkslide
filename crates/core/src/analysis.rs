//! Histogram, clipping, vectorscope and sampler readouts on rendered output.

use rayon::prelude::*;

/// 256-bin RGB + luminance histogram of 8-bit RGBA pixels.
#[derive(Clone, Debug)]
pub struct Histogram {
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
    pub l: Vec<u32>,
    pub clip_shadows: bool,
    pub clip_highlights: bool,
}

impl Histogram {
    pub fn from_rgba(px: &[u8]) -> Self {
        let (r, g, b, l) = px
            .par_chunks(4 * 1024)
            .fold(
                || (vec![0u32; 256], vec![0u32; 256], vec![0u32; 256], vec![0u32; 256]),
                |mut acc, chunk| {
                    for p in chunk.chunks_exact(4) {
                        acc.0[p[0] as usize] += 1;
                        acc.1[p[1] as usize] += 1;
                        acc.2[p[2] as usize] += 1;
                        let y = (p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19) >> 8;
                        acc.3[y as usize] += 1;
                    }
                    acc
                },
            )
            .reduce(
                || (vec![0u32; 256], vec![0u32; 256], vec![0u32; 256], vec![0u32; 256]),
                |mut a, b| {
                    for i in 0..256 {
                        a.0[i] += b.0[i];
                        a.1[i] += b.1[i];
                        a.2[i] += b.2[i];
                        a.3[i] += b.3[i];
                    }
                    a
                },
            );
        let n = (px.len() / 4).max(1) as f32;
        let thresh = (n * 0.0005) as u32;
        let clip_shadows = r[0].max(g[0]).max(b[0]) > thresh;
        let clip_highlights = r[255].max(g[255]).max(b[255]) > thresh;
        Self { r, g, b, l, clip_shadows, clip_highlights }
    }
}

/// Overlay clipping warnings in place: blue where all channels are black,
/// red where any channel is at 255 (ACR convention).
pub fn clipping_overlay(px: &mut [u8], shadows: bool, highlights: bool) {
    px.par_chunks_mut(4).for_each(|p| {
        if highlights && (p[0] == 255 || p[1] == 255 || p[2] == 255) {
            p[0] = 255;
            p[1] = 0;
            p[2] = 0;
        } else if shadows && p[0] == 0 && p[1] == 0 && p[2] == 0 {
            p[0] = 0;
            p[1] = 70;
            p[2] = 255;
        }
    });
}

/// Vectorscope: 2D density of (Cb, Cr) in a `size` x `size` grid.
pub fn vectorscope(px: &[u8], size: usize) -> Vec<u32> {
    let mut grid = vec![0u32; size * size];
    let step = (px.len() / 4 / 200_000).max(1);
    for p in px.chunks_exact(4).step_by(step) {
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let cb = (b - y) / 1.8556;
        let cr = (r - y) / 1.5748;
        let x = ((cb + 0.5) * size as f32) as isize;
        let yy = ((0.5 - cr) * size as f32) as isize;
        if x >= 0 && yy >= 0 && (x as usize) < size && (yy as usize) < size {
            grid[yy as usize * size + x as usize] += 1;
        }
    }
    grid
}

/// Skin tone line angle on the vectorscope (degrees, Cb/Cr plane).
pub const SKIN_TONE_ANGLE: f32 = 123.0;
