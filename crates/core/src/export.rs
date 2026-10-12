//! Save Image: tiled full-resolution render, resize, output sharpening,
//! colour space conversion, encoding and metadata.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::ImageEncoder;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::color::ColorSpace;
use crate::demosaic::Quality;
use crate::exif_write::MetadataMode;
use crate::image::{Image3, Plane, gaussian_blur};
use crate::params::DevelopSettings;
use crate::pipeline::geometry::Region;
use crate::pipeline::{Engine, RenderRequest, to_output};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    Jpeg,
    Tiff,
    Png,
    /// OpenEXR, linear float (HDR capable).
    Exr,
    /// Photoshop document (flat RGB).
    Psd,
    /// AVIF (sRGB, 10-bit).
    Avif,
    /// DNG of the original raw data; settings go to a sidecar.
    Dng,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Tiff => "tif",
            Self::Png => "png",
            Self::Exr => "exr",
            Self::Psd => "psd",
            Self::Avif => "avif",
            Self::Dng => "dng",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Resize {
    None,
    LongEdge(u32),
    ShortEdge(u32),
    WidthHeight(u32, u32),
    Megapixels(f32),
    Percent(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SharpenFor {
    None,
    Screen,
    Matte,
    Glossy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SharpenAmount {
    Low,
    Standard,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TiffCompression {
    None,
    Lzw,
    Deflate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    pub format: Format,
    /// JPEG quality 1..100 (ACR shows 0..12; mapped by the UI).
    pub jpeg_quality: u8,
    /// Limit JPEG file size in KB (0 = off).
    pub jpeg_limit_kb: u32,
    /// 8, 16 or 32 (TIFF only)
    pub bit_depth: u8,
    pub tiff_compression: TiffCompression,
    pub color_space: ColorSpace,
    pub resize: Resize,
    pub dont_enlarge: bool,
    pub ppi: u32,
    pub sharpen_for: SharpenFor,
    pub sharpen_amount: SharpenAmount,
    pub metadata: MetadataMode,
    /// Destination folder; None = same as source.
    pub dest: Option<PathBuf>,
    /// File naming template: {name}, {seq}, {date}, {camera}, {ext}
    pub naming: String,
    pub start_seq: u32,
    pub write_sidecar: bool,
    /// HDR edits saved as JPEG get a gain map (Ultra HDR / Adobe gain map).
    pub hdr_gainmap: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: Format::Jpeg,
            jpeg_quality: 92,
            jpeg_limit_kb: 0,
            bit_depth: 8,
            tiff_compression: TiffCompression::Lzw,
            color_space: ColorSpace::SRgb,
            resize: Resize::None,
            dont_enlarge: true,
            ppi: 300,
            sharpen_for: SharpenFor::None,
            sharpen_amount: SharpenAmount::Standard,
            metadata: MetadataMode::All,
            dest: None,
            naming: "{name}".into(),
            start_seq: 1,
            write_sidecar: false,
            hdr_gainmap: true,
        }
    }
}

/// Output size for the crop size and resize mode.
pub fn output_size(crop_w: f64, crop_h: f64, r: Resize, dont_enlarge: bool) -> (usize, usize) {
    let f = match r {
        Resize::None => 1.0,
        Resize::LongEdge(px) => px as f64 / crop_w.max(crop_h),
        Resize::ShortEdge(px) => px as f64 / crop_w.min(crop_h),
        Resize::WidthHeight(w, h) => (w as f64 / crop_w).min(h as f64 / crop_h),
        Resize::Megapixels(mp) => ((mp as f64 * 1e6) / (crop_w * crop_h)).sqrt(),
        Resize::Percent(p) => p as f64 / 100.0,
    };
    let f = if dont_enlarge { f.min(1.0) } else { f };
    (((crop_w * f).round() as usize).max(1), ((crop_h * f).round() as usize).max(1))
}

/// Render the full image at the given size in tiles. Returns display-linear
/// ProPhoto and alpha.
pub fn render_full(engine: &Engine, s: &DevelopSettings, out_w: usize, out_h: usize, progress: &(dyn Fn(f32) + Sync)) -> (Image3, Plane) {
    const TILE: usize = 1536;
    // neighbourhood filters need context; lens blur needs a lot
    let (fw, fh) = engine.full_size();
    let blur_pad = if s.lens_blur.enabled { (fw.max(fh) as f32 * 0.012 * 3.0 * out_w as f32 / fw.max(1) as f32) as usize } else { 0 };
    let pad: usize = 48 + blur_pad;
    let ctx = engine.tone_ctx(s);
    let mut out = Image3::new(out_w, out_h);
    let mut alpha = Plane::new(out_w, out_h);
    let tiles_x = out_w.div_ceil(TILE);
    let tiles_y = out_h.div_ceil(TILE);
    let total = (tiles_x * tiles_y) as f32;
    let mut done = 0.0;
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let x0 = tx * TILE;
            let y0 = ty * TILE;
            let x1 = ((tx + 1) * TILE).min(out_w);
            let y1 = ((ty + 1) * TILE).min(out_h);
            // padded tile so neighbourhood filters see context
            let px0 = x0 as isize - pad as isize;
            let py0 = y0 as isize - pad as isize;
            let pw = (x1 - x0) + 2 * pad;
            let ph = (y1 - y0) + 2 * pad;
            let region = Region {
                x0: px0 as f64 / out_w as f64,
                y0: py0 as f64 / out_h as f64,
                x1: (px0 + pw as isize) as f64 / out_w as f64,
                y1: (py0 + ph as isize) as f64 / out_h as f64,
                w: pw,
                h: ph,
            };
            let r = engine.render_with_ctx(s, &RenderRequest { region, quality: Quality::Full, ignore_crop: false }, ctx.clone());
            for y in y0..y1 {
                let sy = y - y0 + pad;
                for x in x0..x1 {
                    let sx = x - x0 + pad;
                    out.data[y * out_w + x] = r.image.data[sy * pw + sx];
                    alpha.data[y * out_w + x] = r.alpha.data[sy * pw + sx];
                }
            }
            done += 1.0;
            progress(done / total);
        }
    }
    (out, alpha)
}

/// Output sharpening (ACR: screen / matte / glossy x low / standard / high).
pub fn output_sharpen(img: &mut Image3, mode: SharpenFor, amt: SharpenAmount) {
    let (radius, base) = match mode {
        SharpenFor::None => return,
        SharpenFor::Screen => (0.6, 0.45),
        SharpenFor::Matte => (1.0, 0.8),
        SharpenFor::Glossy => (0.8, 0.6),
    };
    let k = base
        * match amt {
            SharpenAmount::Low => 0.6,
            SharpenAmount::Standard => 1.0,
            SharpenAmount::High => 1.5,
        };
    let l = Plane { w: img.w, h: img.h, data: img.data.par_iter().map(|p| crate::pipeline::tone::enc(crate::color::luma_prophoto(*p).max(0.0))).collect() };
    let b = gaussian_blur(&l, radius);
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let d = l.data[i] - b.data[i];
        let e2 = (l.data[i] + k * d.clamp(-0.1, 0.1)).max(0.0);
        let y0 = crate::pipeline::tone::dec(l.data[i]).max(1e-6);
        let r = crate::pipeline::tone::dec(e2) / y0;
        *p = p.map(|c| c * r);
    });
}

pub fn expand_name(tpl: &str, src: &Path, seq: u32, meta: &crate::decode::Metadata) -> String {
    let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "image".into());
    let date = meta.date.as_deref().map(|d| d.split(' ').next().unwrap_or("").replace(':', "")).unwrap_or_default();
    tpl.replace("{name}", &stem).replace("{seq}", &format!("{seq:04}")).replace("{date}", &date).replace("{camera}", &meta.model.replace(' ', "_"))
}

/// Encode and write. Returns the written path.
pub fn save(engine: &Engine, s: &DevelopSettings, ex: &ExportSettings, seq: u32, progress: &(dyn Fn(f32) + Sync)) -> Result<PathBuf> {
    let src_path = engine.source.path.clone();
    if ex.format == Format::Dng {
        return save_dng(engine, s, ex, seq);
    }
    let (fw, fh) = engine.full_size();
    let geo = engine.geometry(s, fw, fh, false);
    let (cw, ch) = geo.crop_size();
    let sr = if s.enhance.super_resolution { 2.0 } else { 1.0 };
    let (ow, oh) = output_size(cw * sr, ch * sr, ex.resize, ex.dont_enlarge && !s.enhance.super_resolution);
    let (mut img, alpha) = render_full(engine, s, ow, oh, &|p| progress(p * 0.9));
    if s.enhance.super_resolution {
        crate::enhance::super_res_refine(&mut img);
    }
    output_sharpen(&mut img, ex.sharpen_for, ex.sharpen_amount);
    if s.crop.expand && alpha.data.iter().any(|&a| a < 0.5) {
        // expanded canvas: content-aware fill instead of a blank border
        let hole = alpha.map(|a| if a < 0.5 { 1.0 } else { 0.0 });
        let f = (img.w.max(img.h) / 1200).max(1);
        let small = img.downscale_int(f);
        let small_hole = hole.resize(small.w, small.h).map(|v| if v > 0.01 { 1.0 } else { 0.0 });
        let filled = crate::inpaint::inpaint(&small, &small_hole).resize(img.w, img.h);
        img.data.par_iter_mut().zip(alpha.data.par_iter()).zip(filled.data.par_iter()).for_each(|((p, &a), f)| {
            if a < 1.0 {
                *p = [0, 1, 2].map(|c| p[c] * a + f[c] * (1.0 - a));
            }
        });
    } else {
        // composite outside-of-image areas over white, as ACR does
        img.data.par_iter_mut().zip(alpha.data.par_iter()).for_each(|(p, &a)| {
            if a < 1.0 {
                *p = p.map(|v| v * a + (1.0 - a));
            }
        });
    }
    // HDR output keeps linear values above 1; everything else is tone-mapped to SDR
    let hdr_out = matches!(ex.format, Format::Exr) || (ex.format == Format::Tiff && ex.bit_depth == 32);
    let gainmap = s.hdr.enabled && ex.format == Format::Jpeg && ex.hdr_gainmap;
    let hdr_img = if gainmap { Some(img.clone()) } else { None };
    if s.hdr.enabled && !hdr_out {
        img = crate::pipeline::hdr_to_sdr(&img, &s.hdr);
    }
    let enc = to_output(&img, ex.color_space, hdr_out);

    let dir = ex.dest.clone().unwrap_or_else(|| src_path.parent().map(|p| p.to_path_buf()).unwrap_or_default());
    std::fs::create_dir_all(&dir).ok();
    let name = expand_name(&ex.naming, &src_path, seq, &engine.source.meta);
    let mut path = dir.join(format!("{name}.{}", ex.format.ext()));
    // never overwrite the source or an existing export silently
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{name}-{n}.{}", ex.format.ext()));
        n += 1;
    }
    let icc = crate::icc::profile(ex.color_space);
    let meta = crate::exif_write::exif_block(&engine.source.meta, ex.metadata, concat!("Darkslide ", env!("CARGO_PKG_VERSION")));
    match ex.format {
        Format::Jpeg if gainmap => {
            // gain maps are defined on sRGB primaries here
            let m = crate::color::prophoto_to(ColorSpace::SRgb);
            let lin = |i: &Image3| i.map(|p| crate::color::mul_mv(&m, p).map(|v| v.max(0.0)));
            let sdr_lin = lin(&img);
            let hdr_lin = lin(hdr_img.as_ref().unwrap());
            let sdr8 = quantize8(&to_output(&img, ColorSpace::SRgb, false));
            let srgb_icc = crate::icc::profile(ColorSpace::SRgb);
            crate::gainmap::write(&path, &sdr8, &sdr_lin, &hdr_lin, ex.jpeg_quality.clamp(1, 100), &srgb_icc, meta.as_deref())?;
        }
        Format::Jpeg => {
            let rgb8 = quantize8(&enc);
            let mut q = ex.jpeg_quality.clamp(1, 100);
            let bytes = loop {
                let mut buf = vec![];
                let mut e = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q);
                e.set_pixel_density(image::codecs::jpeg::PixelDensity::dpi(ex.ppi as u16));
                let _ = e.set_icc_profile(icc.clone());
                e.encode(&rgb8, ow as u32, oh as u32, image::ExtendedColorType::Rgb8)?;
                if ex.jpeg_limit_kb == 0 || buf.len() <= ex.jpeg_limit_kb as usize * 1024 || q <= 10 {
                    break buf;
                }
                q = q.saturating_sub(5);
            };
            let bytes = match &meta {
                Some(m) => crate::exif_write::insert_app1(&bytes, m),
                None => bytes,
            };
            std::fs::File::create(&path)?.write_all(&bytes)?;
        }
        Format::Png => {
            let f = std::io::BufWriter::new(std::fs::File::create(&path)?);
            let mut e = image::codecs::png::PngEncoder::new(f);
            let _ = e.set_icc_profile(icc);
            if ex.bit_depth >= 16 {
                let v = quantize16(&enc);
                let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_be_bytes()).collect();
                e.write_image(&bytes, ow as u32, oh as u32, image::ExtendedColorType::Rgb16)?;
            } else {
                e.write_image(&quantize8(&enc), ow as u32, oh as u32, image::ExtendedColorType::Rgb8)?;
            }
        }
        Format::Tiff => write_tiff(&path, &enc, ex, &icc).context("writing TIFF")?,
        Format::Psd => write_psd(&path, &enc, ex, &icc)?,
        Format::Avif => write_avif(&path, &img, ex, meta.as_deref())?,
        Format::Dng => unreachable!(),
        Format::Exr => {
            let buf = image::Rgb32FImage::from_raw(ow as u32, oh as u32, enc.data.iter().flat_map(|p| *p).collect()).unwrap();
            image::DynamicImage::ImageRgb32F(buf).save_with_format(&path, image::ImageFormat::OpenExr)?;
        }
    }
    if ex.write_sidecar {
        crate::xmp::write_sidecar(&path, s).ok();
    }
    progress(1.0);
    Ok(path)
}

fn quantize8(img: &Image3) -> Vec<u8> {
    // light ordered dither to avoid banding in smooth gradients
    const B: [f32; 16] = [0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0];
    let w = img.w;
    img.data
        .par_iter()
        .enumerate()
        .flat_map_iter(|(i, p)| {
            let d = (B[(i % w % 4) + (i / w % 4) * 4] / 16.0 - 0.47) / 255.0;
            p.map(|v| ((v + d).clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
        })
        .collect()
}

fn quantize16(img: &Image3) -> Vec<u16> {
    img.data.par_iter().flat_map_iter(|p| p.map(|v| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16)).collect()
}

fn write_tiff(path: &Path, img: &Image3, ex: &ExportSettings, icc: &[u8]) -> Result<()> {
    use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, colortype};
    let f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let comp = match ex.tiff_compression {
        TiffCompression::None => Compression::Uncompressed,
        TiffCompression::Lzw => Compression::Lzw,
        TiffCompression::Deflate => Compression::Deflate(DeflateLevel::Balanced),
    };
    let mut enc = TiffEncoder::new(f)?.with_compression(comp);
    let (w, h) = (img.w as u32, img.h as u32);
    macro_rules! write_with {
        ($ct:ty, $data:expr) => {{
            let data = $data;
            let mut im = enc.new_image::<$ct>(w, h)?;
            im.encoder().write_tag(tiff::tags::Tag::Unknown(34675), icc)?;
            im.resolution(tiff::tags::ResolutionUnit::Inch, tiff::encoder::Rational { n: ex.ppi, d: 1 });
            im.write_data(&data)?;
        }};
    }
    match ex.bit_depth {
        16 => write_with!(colortype::RGB16, quantize16(img)),
        32 => write_with!(colortype::RGB32Float, img.data.iter().flat_map(|p| *p).collect::<Vec<f32>>()),
        _ => write_with!(colortype::RGB8, quantize8(img)),
    }
    Ok(())
}

/// Save as DNG: the original raw data losslessly, settings in a sidecar.
fn save_dng(engine: &Engine, s: &DevelopSettings, ex: &ExportSettings, seq: u32) -> Result<PathBuf> {
    let src = &engine.source.path;
    if !engine.is_raw() {
        anyhow::bail!("DNG output needs a raw source");
    }
    let dir = ex.dest.clone().unwrap_or_else(|| src.parent().map(|p| p.to_path_buf()).unwrap_or_default());
    std::fs::create_dir_all(&dir).ok();
    let name = expand_name(&ex.naming, src, seq, &engine.source.meta);
    let mut path = dir.join(format!("{name}.dng"));
    let mut n = 1;
    while path.exists() || path == *src {
        path = dir.join(format!("{name}-{n}.dng"));
        n += 1;
    }
    let params = rawler::dng::convert::ConvertParams::default()
        .with_compression(rawler::dng::DngCompression::Lossless)
        .with_preview(true)
        .with_thumbnail(true)
        .with_software(concat!("Darkslide ", env!("CARGO_PKG_VERSION")).to_string());
    let mut f = std::io::BufWriter::new(std::fs::File::create(&path)?);
    rawler::dng::convert::convert_raw_file(src, &mut f, &params).map_err(|e| anyhow::anyhow!("{e}"))?;
    drop(f);
    crate::xmp::write_sidecar(&path, s)?;
    Ok(path)
}

/// Flat RGB Photoshop file (8 or 16 bit) with an embedded ICC profile.
fn write_psd(path: &Path, img: &Image3, ex: &ExportSettings, icc: &[u8]) -> Result<()> {
    let bits: u16 = if ex.bit_depth >= 16 { 16 } else { 8 };
    let mut b: Vec<u8> = vec![];
    b.extend(b"8BPS");
    b.extend(1u16.to_be_bytes());
    b.extend([0u8; 6]);
    b.extend(3u16.to_be_bytes()); // channels
    b.extend((img.h as u32).to_be_bytes());
    b.extend((img.w as u32).to_be_bytes());
    b.extend(bits.to_be_bytes());
    b.extend(3u16.to_be_bytes()); // RGB
    b.extend(0u32.to_be_bytes()); // colour mode data
    // image resources: ICC profile (1039) and resolution (1005)
    let mut res: Vec<u8> = vec![];
    let mut resource = |id: u16, data: &[u8]| {
        res.extend(b"8BIM");
        res.extend(id.to_be_bytes());
        res.extend([0u8, 0u8]); // empty pascal name, padded
        res.extend((data.len() as u32).to_be_bytes());
        res.extend(data);
        if data.len() % 2 == 1 {
            res.push(0);
        }
    };
    resource(1039, icc);
    let ppi = (ex.ppi as u32) << 16;
    let mut resinfo = vec![];
    resinfo.extend(ppi.to_be_bytes());
    resinfo.extend(1u16.to_be_bytes());
    resinfo.extend(1u16.to_be_bytes());
    resinfo.extend(ppi.to_be_bytes());
    resinfo.extend(1u16.to_be_bytes());
    resinfo.extend(1u16.to_be_bytes());
    resource(1005, &resinfo);
    b.extend((res.len() as u32).to_be_bytes());
    b.extend(res);
    b.extend(0u32.to_be_bytes()); // layer and mask info
    b.extend(0u16.to_be_bytes()); // raw image data, planar
    for c in 0..3 {
        for p in &img.data {
            let v = p[c].clamp(0.0, 1.0);
            if bits == 16 {
                b.extend(((v * 65535.0 + 0.5) as u16).to_be_bytes());
            } else {
                b.push((v * 255.0 + 0.5) as u8);
            }
        }
    }
    std::fs::write(path, b)?;
    Ok(())
}

/// AVIF output. The encoder signals sRGB, so convert to sRGB regardless of the
/// chosen space; 10-bit keeps gradients clean.
#[cfg(feature = "avif")]
fn write_avif(path: &Path, img: &Image3, ex: &ExportSettings, exif: Option<&[u8]>) -> Result<()> {
    let srgb = to_output(img, ColorSpace::SRgb, false);
    let px: Vec<ravif::RGB8> = srgb
        .data
        .iter()
        .map(|p| {
            let q = p.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            ravif::RGB8::new(q[0], q[1], q[2])
        })
        .collect();
    let mut enc = ravif::Encoder::new().with_quality(ex.jpeg_quality as f32).with_speed(6).with_bit_depth(ravif::BitDepth::Ten);
    if let Some(e) = exif {
        // ravif wants raw TIFF EXIF without the JPEG "Exif\0\0" prefix
        enc = enc.with_exif(e.strip_prefix(b"Exif\0\0".as_slice()).unwrap_or(e).to_vec());
    }
    let out = enc.encode_rgb(ravif::Img::new(px.as_slice(), srgb.w, srgb.h)).map_err(|e| anyhow::anyhow!("AVIF: {e}"))?;
    std::fs::write(path, out.avif_file)?;
    Ok(())
}

#[cfg(not(feature = "avif"))]
fn write_avif(_path: &Path, _img: &Image3, _ex: &ExportSettings, _exif: Option<&[u8]>) -> Result<()> {
    anyhow::bail!("built without AVIF support")
}
