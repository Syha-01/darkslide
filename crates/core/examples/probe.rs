use rawler::rawsource::RawSource;
fn main() {
    for p in std::env::args().skip(1) {
        let t = std::time::Instant::now();
        let src = RawSource::new(std::path::Path::new(&p)).unwrap();
        let dec = rawler::get_decoder(&src).unwrap();
        let params = rawler::decoders::RawDecodeParams::default();
        let img = dec.raw_image(&src, &params, false).unwrap();
        let md = dec.raw_metadata(&src, &params).unwrap();
        println!(
            "{p}: {}x{} cpp={} bps={} wb={:?} white={:?} black={:?} act={:?} crop={:?} orient={:?} phot={:?} mats={:?} t={:?}",
            img.width,
            img.height,
            img.cpp,
            img.bps,
            img.wb_coeffs,
            img.whitelevel,
            img.blacklevel.levels,
            img.active_area,
            img.crop_area,
            img.orientation,
            match &img.photometric {
                rawler::rawimage::RawPhotometricInterpretation::Cfa(c) => format!("{}", c.cfa),
                o => format!("{:?}", o),
            },
            img.color_matrix.keys().collect::<Vec<_>>(),
            t.elapsed()
        );
        println!(
            "   lens={:?} iso={:?} f={:?} exp={:?} fl={:?} dngtags={:?}",
            md.exif.lens_model,
            md.exif.iso_speed_ratings,
            md.exif.fnumber,
            md.exif.exposure_time,
            md.exif.focal_length,
            img.dng_tags.keys().collect::<Vec<_>>()
        );
    }
}
