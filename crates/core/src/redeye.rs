//! Red eye / pet eye correction on display-linear ProPhoto, in T-frame coords.

use rayon::prelude::*;

use crate::color::{Oklab, lab_to_lch, lch_to_lab};
use crate::image::Image3;
use crate::params::RedEyeSpot;
use crate::pipeline::tone::smoothstep;

pub fn apply(img: &mut Image3, spots: &[RedEyeSpot], tco: &[[f32; 2]], ok: &Oklab) {
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let t = tco[i];
        for s in spots {
            let dx = (t[0] - s.center[0]) / s.radius[0].max(1e-5);
            let dy = (t[1] - s.center[1]) / s.radius[1].max(1e-5);
            let d = (dx * dx + dy * dy).sqrt();
            // pupil occupies pupil_size% of the selection
            let pr = 0.35 + s.pupil_size / 100.0 * 0.6;
            let w = 1.0 - smoothstep(pr * 0.85, pr, d);
            if w <= 0.0 {
                continue;
            }
            let mut lch = lab_to_lch(ok.from_prophoto(*p));
            if s.pet {
                // pet eye: any bright glow becomes a dark pupil
                let glow = smoothstep(0.35, 0.7, lch[0]);
                let k = w * glow.max(0.3);
                lch[0] = lch[0] * (1.0 - k) + 0.12 * (1.0 - s.darken / 100.0 * 0.8) * k;
                lch[1] *= 1.0 - k;
                if s.catchlight {
                    let cd = ((dx + 0.25 * pr).powi(2) + (dy + 0.25 * pr).powi(2)).sqrt();
                    let c = 1.0 - smoothstep(pr * 0.08, pr * 0.14, cd);
                    lch[0] = lch[0] * (1.0 - c) + 0.95 * c;
                }
            } else {
                // only reddish pixels
                let red = 1.0 - smoothstep(25.0, 45.0, crate::color::hue_diff(lch[2], 20.0).abs());
                let k = w * red * smoothstep(0.03, 0.08, lch[1]);
                lch[1] *= 1.0 - k;
                lch[0] *= 1.0 - k * (0.3 + s.darken / 100.0 * 0.6);
            }
            *p = ok.to_prophoto(lch_to_lab(lch)).map(|v| v.max(0.0));
        }
    });
}
