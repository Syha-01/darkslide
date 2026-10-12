use darkslide_core::*;
fn save(img: &image::Image3, p: &str) {
    let px: Vec<u8> = img.data.iter().flat_map(|q| q.map(|v| (color::srgb_encode((v * 1.6).clamp(0.0, 1.0)) * 255.0) as u8)).collect();
    ::image::RgbImage::from_raw(img.w as u32, img.h as u32, px).unwrap().save(p).unwrap();
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let e = Engine::open(std::path::Path::new(&a[1])).unwrap();
    let base = e.base(demosaic::Quality::Full);
    // window over grass/dirt boundary
    let win = base.crop(4600, 1500, 600, 700);
    for (name, x0, x1) in [("mid", 250usize, 330usize), ("edge", 520, 600)] {
        let hole = image::Plane::from_fn(win.w, win.h, |x, _| if x >= x0 && x < x1 { 1.0 } else { 0.0 });
        let t = std::time::Instant::now();
        let out = inpaint::inpaint(&win, &hole);
        println!("{name}: {:?}", t.elapsed());
        save(&out, &format!("{}/inp_{name}.png", a[2]));
    }
    save(&win, &format!("{}/inp_orig.png", a[2]));
}
