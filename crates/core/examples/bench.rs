use darkslide_core::*;
fn main() {
    env_logger::init();
    let p = std::env::args().nth(1).unwrap();
    let e = Engine::open(std::path::Path::new(&p)).unwrap();
    let t = std::time::Instant::now();
    let _ = e.base(demosaic::Quality::Draft);
    println!("draft base {:?}", t.elapsed());
    let mut cases: Vec<(&str, DevelopSettings)> = vec![("default", DevelopSettings::defaults_for(e.is_raw()))];
    let mut s = DevelopSettings::defaults_for(e.is_raw());
    s.clarity = 30.0;
    s.texture = 30.0;
    s.dehaze = 20.0;
    s.shadows = 40.0;
    s.highlights = -40.0;
    cases.push(("tone heavy", s.clone()));
    s.nr.luminance = 40.0;
    s.hsl[3].saturation = 20.0;
    s.grading.shadows.sat = 20.0;
    s.vignette.amount = -20.0;
    s.grain.amount = 20.0;
    cases.push(("all panels", s.clone()));
    for (name, s) in cases {
        for (w, h) in [(1500usize, 1000usize), (900, 600)] {
            let _ = e.render_fit(&s, w, h, demosaic::Quality::Draft, false);
            let t = std::time::Instant::now();
            let n = 5;
            for k in 0..n {
                let mut s2 = s.clone();
                s2.exposure = k as f32 * 0.01; // defeat the ctx cache like a slider drag would
                let _ = e.render_fit(&s2, w, h, demosaic::Quality::Draft, false);
            }
            println!("{name:12} {w}x{h}: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
        }
    }
}
