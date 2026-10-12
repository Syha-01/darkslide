fn main() {
    let p = std::env::args().nth(1).unwrap();
    let e = darkslide_core::Engine::open(std::path::Path::new(&p)).unwrap();
    let s = darkslide_core::DevelopSettings::defaults_for(e.is_raw());
    let t = std::time::Instant::now();
    match darkslide_core::distract::remove_wires(&e, &s) {
        Some(sp) => {
            let m = darkslide_core::pipeline::local::decode_raster(sp.mask.as_ref().unwrap()).unwrap();
            println!("wires: {} px of {} ({:?})", m.data.iter().filter(|v| **v > 0.5).count(), m.data.len(), t.elapsed());
        }
        None => println!("no wires ({:?})", t.elapsed()),
    }
}
