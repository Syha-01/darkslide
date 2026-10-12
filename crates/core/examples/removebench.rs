use darkslide_core::*;
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let e = Engine::open(std::path::Path::new(&p)).unwrap();
    for q in [demosaic::Quality::Draft, demosaic::Quality::Full] {
        let mut img = (*e.base(q)).clone();
        // a stroke down the right edge, like painting out a bar
        let spot = params::RetouchSpot { mode: params::RetouchMode::Remove, points: (0..20).map(|i| [0.985, 0.02 + i as f32 * 0.05]).collect(), radius: 0.012, feather: 30.0, opacity: 100.0, source_offset: [0.0, 0.0], mask: None };
        let t = std::time::Instant::now();
        retouch::apply(&mut img, &[spot]);
        println!("{q:?} {}x{}: {:?}", img.w, img.h, t.elapsed());
    }
}
