use darkslide_core::decode::*;
use darkslide_core::demosaic::*;
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let s = load(std::path::Path::new(&p)).unwrap();
    let SourceData::Cfa(c) = &s.data else { return };
    let n = s.as_shot_neutral;
    let mul = [1.0 / n[0], 1.0, 1.0 / n[2]];
    for q in [Quality::Draft, Quality::Full] {
        let (mut img, clip) = demosaic(c, mul, q);
        let stat = |img: &darkslide_core::image::Image3| {
            let (w, h) = (img.w, img.h);
            let mut mag = 0;
            let mut tot = 0;
            let mut ex = None;
            for y in 0..h / 6 {
                for x in w * 3 / 10..w * 7 / 10 {
                    let p = img.get(x, y);
                    if p[1] > 0.8 || p[0] > 0.8 {
                        tot += 1;
                        if p[0] > p[1] * 1.08 && p[2] > p[1] * 1.0 {
                            mag += 1;
                            if ex.is_none() {
                                ex = Some((x, y, p));
                            }
                        }
                    }
                }
            }
            (mag, tot, ex)
        };
        println!("{q:?} before: {:?}", stat(&img));
        if let Some((_, _, Some((x, y, _)))) = Some(stat(&img)) {
            println!("  clip at ex {}", clip.get(x, y));
        }
        reconstruct_highlights(&mut img, &clip, mul, cfa_clip_levels(c));
        println!("{q:?} after: {:?}", stat(&img));
    }
}
