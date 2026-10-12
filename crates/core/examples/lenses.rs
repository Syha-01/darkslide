fn main() {
    let db = darkslide_core::lens::load_lensfun_db();
    println!("{} lenses", db.len());
    for p in std::env::args().skip(1) {
        let s = darkslide_core::decode::load(std::path::Path::new(&p)).unwrap();
        let name = s.meta.lens.clone().unwrap_or_default();
        let l = darkslide_core::lens::find_lens(&db, &name);
        println!(
            "{p}: '{name}' -> {:?}",
            l.map(|l| (l.maker.clone(), l.model.clone(), l.at(s.meta.focal_length.unwrap_or(50.0) as f64, s.meta.fnumber.unwrap_or(8.0) as f64)))
        );
    }
}
