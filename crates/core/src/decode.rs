//! Loading raw and raster files into a normalised source representation.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use rawler::decoders::RawDecodeParams;
use rawler::imgop::xyz::Illuminant;
use rawler::rawimage::{RawImageData, RawPhotometricInterpretation};
use rawler::rawsource::RawSource;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::color::{CameraColor, Mat3};
use crate::image::{Image3, Rgb};

/// Repeating colour filter pattern. Colours: 0=R 1=G 2=B.
#[derive(Clone, Debug, PartialEq)]
pub struct CfaPattern {
    pub width: usize,
    pub height: usize,
    pub colors: Vec<u8>,
}

impl CfaPattern {
    #[inline]
    pub fn color_at(&self, x: usize, y: usize) -> usize {
        self.colors[(y % self.height) * self.width + (x % self.width)] as usize
    }

    pub fn is_bayer(&self) -> bool {
        self.width == 2 && self.height == 2
    }

    pub fn is_xtrans(&self) -> bool {
        self.width == 6 && self.height == 6
    }

    pub fn shifted(&self, dx: usize, dy: usize) -> Self {
        let mut colors = vec![0u8; self.colors.len()];
        for y in 0..self.height {
            for x in 0..self.width {
                colors[y * self.width + x] = self.color_at(x + dx, y + dy) as u8;
            }
        }
        Self { width: self.width, height: self.height, colors }
    }
}

/// Mosaiced sensor data, black-subtracted and scaled so 1.0 = clipping.
#[derive(Clone, Debug)]
pub struct CfaImage {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
    pub pattern: CfaPattern,
}

#[derive(Clone, Debug)]
pub enum SourceData {
    Cfa(CfaImage),
    /// Linear camera RGB (LinearRaw DNG) or linear sRGB (raster files).
    Rgb(Image3),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub make: String,
    pub model: String,
    pub lens: Option<String>,
    pub iso: Option<u32>,
    pub exposure_time: Option<f32>,
    pub fnumber: Option<f32>,
    pub focal_length: Option<f32>,
    pub date: Option<String>,
    pub orientation: u16,
    pub rating: Option<u32>,
    pub artist: Option<String>,
    pub copyright: Option<String>,
    pub gps: Option<[f64; 2]>,
    pub serial: Option<String>,
}

impl Metadata {
    pub fn exposure_summary(&self) -> String {
        let mut parts = vec![];
        if let Some(iso) = self.iso {
            parts.push(format!("ISO {iso}"));
        }
        if let Some(fl) = self.focal_length {
            parts.push(format!("{fl:.0} mm"));
        }
        if let Some(f) = self.fnumber {
            parts.push(format!("f/{f:.1}"));
        }
        if let Some(t) = self.exposure_time {
            parts.push(if t < 1.0 && t > 0.0 { format!("1/{:.0} s", 1.0 / t) } else { format!("{t:.1} s") });
        }
        parts.join("  ")
    }
}

#[derive(Clone, Debug)]
pub struct Source {
    pub path: PathBuf,
    pub is_raw: bool,
    pub meta: Metadata,
    pub color: CameraColor,
    /// Camera neutral at capture (G = 1).
    pub as_shot_neutral: Rgb,
    /// Exposure compensation baked into the default rendering (EV).
    pub baseline_exposure: f32,
    pub data: SourceData,
    /// XMP packet embedded in the file (DNG/JPEG/TIFF), if any.
    pub embedded_xmp: Option<String>,
}

/// Find an embedded XMP packet anywhere in the file.
pub fn find_xmp(buf: &[u8]) -> Option<String> {
    let start = memchr::memmem::find(buf, b"<x:xmpmeta")?;
    let end = memchr::memmem::find(&buf[start..], b"</x:xmpmeta>")? + start + b"</x:xmpmeta>".len();
    String::from_utf8(buf[start..end].to_vec()).ok()
}

impl Source {
    /// Dimensions after orientation.
    pub fn oriented_size(&self) -> (usize, usize) {
        let (w, h) = self.sensor_size();
        if orientation_swaps(self.meta.orientation) { (h, w) } else { (w, h) }
    }

    pub fn sensor_size(&self) -> (usize, usize) {
        match &self.data {
            SourceData::Cfa(c) => (c.w, c.h),
            SourceData::Rgb(i) => (i.w, i.h),
        }
    }

    pub fn as_shot_xy(&self) -> [f32; 2] {
        self.color.xy_for_neutral(self.as_shot_neutral)
    }
}

pub const RASTER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "tif", "tiff", "webp"];

pub fn is_supported(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    RASTER_EXTENSIONS.contains(&ext.as_str()) || rawler::decoders::supported_extensions().iter().any(|e| e.eq_ignore_ascii_case(&ext))
}

pub fn is_raster(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    // TIFFs may be raw-ish DNG relatives, but rawler rejects plain TIFF, so try raster for those
    RASTER_EXTENSIONS.contains(&ext.as_str())
}

pub fn load(path: &Path) -> Result<Source> {
    if is_raster(path) { load_raster(path) } else { load_raw(path) }
}

fn illuminant_temp(i: Illuminant) -> f32 {
    match i {
        Illuminant::A | Illuminant::Tungsten | Illuminant::IsoStudioTungsten => 2856.0,
        Illuminant::D50 => 5003.0,
        Illuminant::D55 | Illuminant::Daylight | Illuminant::FineWeather | Illuminant::Flash => 5503.0,
        Illuminant::D65 | Illuminant::CloudyWeather => 6504.0,
        Illuminant::D75 | Illuminant::Shade => 7504.0,
        Illuminant::Fluorescent | Illuminant::CoolWhiteFluorescent => 4150.0,
        Illuminant::DaylightFluorescent => 6430.0,
        Illuminant::DaylightWhiteFluorescent => 5000.0,
        Illuminant::WhiteFluorescent => 3450.0,
        Illuminant::B => 4874.0,
        Illuminant::C => 6774.0,
        Illuminant::Unknown => 5003.0,
    }
}

fn flat_to_mat3(m: &[f32]) -> Option<Mat3> {
    if m.len() < 9 {
        return None;
    }
    let mat = [[m[0], m[1], m[2]], [m[3], m[4], m[5]], [m[6], m[7], m[8]]];
    if mat.iter().flatten().all(|v| *v == 0.0) {
        return None;
    }
    Some(mat)
}

pub fn load_raw(path: &Path) -> Result<Source> {
    let src = RawSource::new(path).with_context(|| format!("opening {}", path.display()))?;
    let dec = rawler::get_decoder(&src).map_err(|e| anyhow!("{e}"))?;
    let params = RawDecodeParams::default();
    let raw = dec.raw_image(&src, &params, false).map_err(|e| anyhow!("{e}"))?;
    let md = dec.raw_metadata(&src, &params).ok();

    let mut meta =
        Metadata { make: raw.clean_make.clone(), model: raw.clean_model.clone(), orientation: raw.orientation.to_u16().max(1), ..Default::default() };
    if let Some(md) = &md {
        let ex = &md.exif;
        meta.lens = md.lens.as_ref().map(|l| format!("{} {}", l.lens_make, l.lens_model).trim().to_string()).or(ex.lens_model.clone());
        meta.iso = ex.iso_speed_ratings.map(|v| v as u32).or(ex.iso_speed).or(ex.recommended_exposure_index);
        meta.exposure_time = ex.exposure_time.map(|r| r.n as f32 / r.d.max(1) as f32);
        meta.fnumber = ex.fnumber.map(|r| r.n as f32 / r.d.max(1) as f32);
        meta.focal_length = ex.focal_length.map(|r| r.n as f32 / r.d.max(1) as f32);
        meta.date = ex.date_time_original.clone().or(ex.create_date.clone());
        meta.rating = md.rating;
        meta.artist = ex.artist.clone();
        meta.copyright = ex.copyright.clone();
        meta.serial = ex.serial_number.clone();
        if let Some(gps) = &ex.gps {
            if let (Some(lat), Some(lon)) = (gps.gps_latitude, gps.gps_longitude) {
                let dms = |v: [rawler::formats::tiff::Rational; 3]| {
                    v[0].n as f64 / v[0].d.max(1) as f64 + v[1].n as f64 / v[1].d.max(1) as f64 / 60.0 + v[2].n as f64 / v[2].d.max(1) as f64 / 3600.0
                };
                let mut la = dms(lat);
                let mut lo = dms(lon);
                if gps.gps_latitude_ref.as_deref() == Some("S") {
                    la = -la;
                }
                if gps.gps_longitude_ref.as_deref() == Some("W") {
                    lo = -lo;
                }
                meta.gps = Some([la, lo]);
            }
        }
        if let Some(o) = ex.orientation {
            if (1..=8).contains(&o) && meta.orientation <= 1 {
                meta.orientation = o;
            }
        }
    }

    // Colour matrices
    let mut matrices = vec![];
    for (ill, m) in &raw.color_matrix {
        if let Some(mat) = flat_to_mat3(m) {
            matrices.push((illuminant_temp(*ill), mat));
        }
    }
    if matrices.is_empty() {
        let m = raw.xyz_to_cam;
        let mat = [m[0], m[1], m[2]];
        if mat.iter().flatten().any(|v| *v != 0.0) {
            matrices.push((6504.0, mat));
        }
    }
    if matrices.is_empty() {
        return Err(anyhow!("no colour matrix for {} {}", raw.clean_make, raw.clean_model));
    }
    // DNG/rawler matrices are XYZ(D50-relative?) -> camera; adobe ColorMatrix maps XYZ to reference camera space
    let color = CameraColor { matrices, is_rgb_space: false };

    let wb = raw.wb_coeffs;
    let as_shot_neutral = if wb[0].is_finite() && wb[0] > 0.0 && wb[1] > 0.0 && wb[2] > 0.0 {
        [wb[1] / wb[0], 1.0, wb[1] / wb[2]]
    } else {
        color.neutral_for_xy(crate::color::D65_XY)
    };

    let (crop_x, crop_y, cw, ch) = match raw.crop_area.or(raw.active_area) {
        Some(r) => (r.p.x, r.p.y, r.d.w, r.d.h),
        None => (0, 0, raw.width, raw.height),
    };

    let white = raw.whitelevel.as_vec();
    let black = &raw.blacklevel;
    let black_vals = black.as_vec();
    let (bw, bh, bcpp) = (black.width.max(1), black.height.max(1), black.cpp.max(1));
    let black_at = |x: usize, y: usize, c: usize| -> f32 {
        if black_vals.len() == 1 { black_vals[0] } else { black_vals[((y % bh) * bw + (x % bw)) * bcpp + c.min(bcpp - 1)] }
    };

    let data_f = raw.data.as_f32();
    let is_float = matches!(raw.data, RawImageData::Float(_));
    let width = raw.width;

    let data = match &raw.photometric {
        RawPhotometricInterpretation::Cfa(cfg) => {
            let cfa = &cfg.cfa;
            let pat = CfaPattern {
                width: cfa.width,
                height: cfa.height,
                colors: (0..cfa.height)
                    .flat_map(|y| (0..cfa.width).map(move |x| (y, x)))
                    .map(|(y, x)| match cfa.color_at(y, x) {
                        0 => 0,
                        2 => 2,
                        _ => 1,
                    })
                    .collect(),
            };
            let white_at = |c: usize| -> f32 { if white.len() >= 3 { white[c.min(white.len() - 1)] } else { white[0] } };
            let mut out = vec![0.0f32; cw * ch];
            out.par_chunks_mut(cw).enumerate().for_each(|(y, row)| {
                let sy = y + crop_y;
                for (x, v) in row.iter_mut().enumerate() {
                    let sx = x + crop_x;
                    let c = pat.color_at(sx, sy);
                    let raw_v = data_f[sy * width + sx];
                    if is_float {
                        *v = raw_v;
                    } else {
                        let b = black_at(sx, sy, 0);
                        *v = (raw_v - b) / (white_at(c) - b).max(1.0);
                    }
                }
            });
            SourceData::Cfa(CfaImage { w: cw, h: ch, data: out, pattern: pat.shifted(crop_x, crop_y) })
        }
        RawPhotometricInterpretation::LinearRaw | RawPhotometricInterpretation::BlackIsZero => {
            let cpp = raw.cpp;
            let mut img = Image3::new(cw, ch);
            img.data.par_chunks_mut(cw).enumerate().for_each(|(y, row)| {
                let sy = y + crop_y;
                for (x, p) in row.iter_mut().enumerate() {
                    let sx = x + crop_x;
                    let base = (sy * width + sx) * cpp;
                    for c in 0..3 {
                        let cc = c.min(cpp - 1);
                        let raw_v = data_f[base + cc];
                        if is_float {
                            p[c] = raw_v;
                        } else {
                            let b = black_at(sx, sy, cc);
                            let wl = white[cc.min(white.len() - 1)];
                            p[c] = (raw_v - b) / (wl - b).max(1.0);
                        }
                    }
                }
            });
            SourceData::Rgb(img)
        }
    };

    // DNG BaselineExposure if present (tag 0xC62A)
    let baseline_exposure = raw
        .dng_tags
        .get(&0xC62A)
        .and_then(|v| match v {
            rawler::formats::tiff::Value::SRational(r) => r.first().map(|r| r.n as f32 / r.d.max(1) as f32),
            rawler::formats::tiff::Value::Rational(r) => r.first().map(|r| r.n as f32 / r.d.max(1) as f32),
            _ => None,
        })
        .unwrap_or(0.0);

    let embedded_xmp = find_xmp(src.buf());
    Ok(Source { path: path.to_path_buf(), is_raw: true, meta, color, as_shot_neutral, baseline_exposure, data, embedded_xmp })
}

pub fn load_raster(path: &Path) -> Result<Source> {
    use image::ImageDecoder;
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation().map(|o| o.to_exif()).unwrap_or(1) as u16;
    let img = image::DynamicImage::from_decoder(decoder)?;
    let rgb = img.to_rgb32f();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let raw = rgb.into_raw();
    // Build a 256-entry LUT for 8-bit sources? Values are already float; decode sRGB transfer.
    let data: Vec<Rgb> =
        raw.par_chunks(3).map(|c| [crate::color::srgb_decode(c[0]), crate::color::srgb_decode(c[1]), crate::color::srgb_decode(c[2])]).collect();
    let meta = Metadata { orientation: orientation.max(1), ..read_exif_raster(path).unwrap_or_default() };
    let embedded_xmp = std::fs::read(path).ok().and_then(|b| find_xmp(&b));
    Ok(Source {
        embedded_xmp,
        path: path.to_path_buf(),
        is_raw: false,
        meta,
        color: CameraColor::srgb(),
        as_shot_neutral: [1.0, 1.0, 1.0],
        baseline_exposure: 0.0,
        data: SourceData::Rgb(Image3 { w, h, data }),
    })
}

/// Best-effort EXIF for raster files via rawler's TIFF/JFIF parsing is not
/// available, so only basic info is read here.
fn read_exif_raster(_path: &Path) -> Option<Metadata> {
    None
}

/// Embedded preview / thumbnail for quick filmstrip display.
pub fn load_thumbnail(path: &Path, max: u32) -> Result<image::RgbImage> {
    if is_raster(path) {
        let img = image::open(path)?;
        return Ok(img.thumbnail(max, max).to_rgb8());
    }
    let src = RawSource::new(path)?;
    let dec = rawler::get_decoder(&src).map_err(|e| anyhow!("{e}"))?;
    let params = RawDecodeParams::default();
    let img = dec
        .thumbnail_image(&src, &params)
        .ok()
        .flatten()
        .or_else(|| dec.preview_image(&src, &params).ok().flatten())
        .or_else(|| dec.full_image(&src, &params).ok().flatten())
        .ok_or_else(|| anyhow!("no embedded preview"))?;
    let md = dec.raw_metadata(&src, &params).ok();
    let o = md.and_then(|m| m.exif.orientation).unwrap_or(1);
    let img = img.thumbnail(max, max);
    let img = match o {
        3 => img.rotate180(),
        6 => img.rotate90(),
        8 => img.rotate270(),
        _ => img,
    };
    Ok(img.to_rgb8())
}

pub fn orientation_swaps(o: u16) -> bool {
    matches!(o, 5..=8)
}

/// Apply an EXIF orientation to an image.
pub fn apply_orientation(img: &Image3, o: u16) -> Image3 {
    let (w, h) = (img.w, img.h);
    match o {
        2 => Image3::from_fn(w, h, |x, y| img.get(w - 1 - x, y)),
        3 => Image3::from_fn(w, h, |x, y| img.get(w - 1 - x, h - 1 - y)),
        4 => Image3::from_fn(w, h, |x, y| img.get(x, h - 1 - y)),
        5 => Image3::from_fn(h, w, |x, y| img.get(y, x)),
        6 => Image3::from_fn(h, w, |x, y| img.get(y, h - 1 - x)),
        7 => Image3::from_fn(h, w, |x, y| img.get(w - 1 - y, h - 1 - x)),
        8 => Image3::from_fn(h, w, |x, y| img.get(w - 1 - y, x)),
        _ => img.clone(),
    }
}

/// Map a point in oriented coords back to sensor coords (both in pixels).
pub fn oriented_to_sensor(o: u16, x: f32, y: f32, sw: f32, sh: f32) -> (f32, f32) {
    match o {
        2 => (sw - x, y),
        3 => (sw - x, sh - y),
        4 => (x, sh - y),
        5 => (y, x),
        6 => (y, sh - x),
        7 => (sw - y, sh - x),
        8 => (sw - y, x),
        _ => (x, y),
    }
}
