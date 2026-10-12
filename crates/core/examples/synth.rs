//! Make synthetic brackets / pano halves from one raw for merge tests.
use darkslide_core::*;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (src, out) = (std::path::Path::new(&a[1]), std::path::Path::new(&a[2]));
    let e = Engine::open(src).unwrap();
    let base = e.base(demosaic::Quality::Full);
    let n = e.source.as_shot_neutral;
    let cam = base.map(|p| [p[0] * n[0], p[1] * n[1], p[2] * n[2]]);
    for (k, ev) in [-2.0f32, 0.0, 2.0].iter().enumerate() {
        let f = ev.exp2();
        let shifted = image::Image3::from_fn(cam.w, cam.h, |x, y| {
            cam.get_clamped(x as isize + k as isize * 3 - 3, y as isize + k as isize - 1).map(|v| (v * f).min(1.0))
        });
        merge::write_linear_dng(&out.join(format!("br{k}.dng")), &shifted, src).unwrap();
    }
    let w = cam.w;
    let left = cam.crop(0, 0, w * 6 / 10, cam.h);
    let right = cam.crop(w * 4 / 10, 0, w - w * 4 / 10, cam.h);
    merge::write_linear_dng(&out.join("pano0.dng"), &left, src).unwrap();
    merge::write_linear_dng(&out.join("pano1.dng"), &right, src).unwrap();
    println!("ok");
}
