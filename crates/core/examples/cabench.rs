use darkslide_core::demosaic::Quality;
use darkslide_core::image::Image3;
use darkslide_core::pipeline::Engine;

// inject a known radial scale into R and B and see how much the estimator recovers
fn scaled(img: &Image3, sr: f64, sb: f64) -> Image3 {
    let (cx, cy) = (img.w as f64 / 2.0, img.h as f64 / 2.0);
    let r = img.channel(0);
    let b = img.channel(2);
    Image3::from_fn(img.w, img.h, |x, y| {
        let dx = x as f64 + 0.5 - cx;
        let dy = y as f64 + 0.5 - cy;
        let p = img.get(x, y);
        let rv = r.sample_bilinear((cx + dx * (1.0 + sr) - 0.5) as f32, (cy + dy * (1.0 + sr) - 0.5) as f32);
        let bv = b.sample_bilinear((cx + dx * (1.0 + sb) - 0.5) as f32, (cy + dy * (1.0 + sb) - 0.5) as f32);
        [rv, p[1], bv]
    })
}

fn main() {
    for p in std::env::args().skip(1) {
        let e = Engine::open(std::path::Path::new(&p)).unwrap();
        let base = e.base(Quality::Draft);
        let t = std::time::Instant::now();
        let e0 = darkslide_core::lens::estimate_lateral_ca(&base);
        let dt = t.elapsed();
        let mut errs = vec![];
        for (sr, sb) in [(0.0015, -0.001), (-0.002, 0.0012), (0.0008, 0.0008)] {
            let est = darkslide_core::lens::estimate_lateral_ca(&scaled(&base, sr, sb));
            errs.push(format!("inj ({sr:+.4},{sb:+.4}) got ({:+.4},{:+.4})", est[0] - e0[0], est[1] - e0[1]));
        }
        println!("{p}: base ({:+.5},{:+.5}) {:?}\n  {}", e0[0], e0[1], dt, errs.join("\n  "));
    }
}
