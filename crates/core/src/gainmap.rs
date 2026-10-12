//! HDR JPEG with a gain map (Adobe gain map / Ultra HDR layout): SDR base
//! image + 8-bit log2 gain map, linked by XMP and an MPF index.

use anyhow::Result;
use image::ImageEncoder;
use rayon::prelude::*;

use crate::image::Image3;

const OFFSET: f32 = 1.0 / 64.0;

fn app_segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut s = vec![0xFF, marker];
    s.extend(((payload.len() + 2) as u16).to_be_bytes());
    s.extend(payload);
    s
}

fn insert_after_soi(jpeg: &[u8], segments: &[Vec<u8>]) -> Vec<u8> {
    let mut out = jpeg[..2].to_vec();
    for s in segments {
        out.extend(s);
    }
    out.extend(&jpeg[2..]);
    out
}

fn encode_jpeg(px: &[u8], w: u32, h: u32, color: image::ExtendedColorType, q: u8, icc: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut buf = vec![];
    let mut e = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q);
    if let Some(p) = icc {
        let _ = e.set_icc_profile(p.to_vec());
    }
    e.write_image(px, w, h, color)?;
    Ok(buf)
}

fn mpf_segment(primary_len: u32, gain_len: u32, gain_offset: u32) -> Vec<u8> {
    // TIFF-style MP index IFD, big endian
    let mut t: Vec<u8> = b"MM\0\x2a\0\0\0\x08".to_vec();
    let entries = 3u16;
    t.extend(entries.to_be_bytes());
    let entry_data_off = 8 + 2 + 12 * entries as u32 + 4;
    // MPFVersion
    t.extend(0xB000u16.to_be_bytes());
    t.extend(7u16.to_be_bytes());
    t.extend(4u32.to_be_bytes());
    t.extend(b"0100");
    // NumberOfImages
    t.extend(0xB001u16.to_be_bytes());
    t.extend(4u16.to_be_bytes());
    t.extend(1u32.to_be_bytes());
    t.extend(2u32.to_be_bytes());
    // MPEntry
    t.extend(0xB002u16.to_be_bytes());
    t.extend(7u16.to_be_bytes());
    t.extend(32u32.to_be_bytes());
    t.extend(entry_data_off.to_be_bytes());
    t.extend(0u32.to_be_bytes()); // next IFD
    // entry 1: primary
    t.extend(0x0003_0000u32.to_be_bytes());
    t.extend(primary_len.to_be_bytes());
    t.extend(0u32.to_be_bytes());
    t.extend([0u8; 4]);
    // entry 2: gain map
    t.extend(0u32.to_be_bytes());
    t.extend(gain_len.to_be_bytes());
    t.extend(gain_offset.to_be_bytes());
    t.extend([0u8; 4]);
    let mut p = b"MPF\0".to_vec();
    p.extend(t);
    app_segment(0xE2, &p)
}

/// Write an HDR JPEG. `sdr` and `hdr` are display-linear sRGB-primaries
/// renditions (hdr may exceed 1.0); `sdr8` is the encoded SDR base.
pub fn write(path: &std::path::Path, sdr8: &[u8], sdr: &Image3, hdr: &Image3, quality: u8, icc: &[u8], exif: Option<&[u8]>) -> Result<()> {
    let (w, h) = (sdr.w, sdr.h);
    // gain in log2 per pixel from luminance
    let lum = |p: &[f32; 3]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let gains: Vec<f32> =
        sdr.data.par_iter().zip(hdr.data.par_iter()).map(|(s, hh)| ((lum(hh).max(0.0) + OFFSET) / (lum(s).max(0.0) + OFFSET)).log2()).collect();
    let gmin = gains.iter().cloned().fold(f32::MAX, f32::min).max(0.0).min(0.0);
    let gmax = gains.iter().cloned().fold(0.0f32, f32::max).max(0.01);
    // quarter-resolution map is plenty and keeps files small
    let (gw, gh) = ((w / 4).max(1), (h / 4).max(1));
    let gpx: Vec<u8> = (0..gw * gh)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % gw) * 4, (i / gw) * 4);
            let mut acc = 0.0;
            for dy in 0..4 {
                for dx in 0..4 {
                    acc += gains[(y + dy).min(h - 1) * w + (x + dx).min(w - 1)];
                }
            }
            let g = acc / 16.0;
            (((g - gmin) / (gmax - gmin)).clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        })
        .collect();
    let gain_xmp = format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:hdrgm="http://ns.adobe.com/hdr-gain-map/1.0/" hdrgm:Version="1.0" hdrgm:GainMapMin="{gmin}" hdrgm:GainMapMax="{gmax}" hdrgm:Gamma="1" hdrgm:OffsetSDR="{OFFSET}" hdrgm:OffsetHDR="{OFFSET}" hdrgm:HDRCapacityMin="0" hdrgm:HDRCapacityMax="{gmax}" hdrgm:BaseRenditionIsHDR="False"/></rdf:RDF></x:xmpmeta>"#
    );
    let mut gxmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    gxmp.extend(gain_xmp.as_bytes());
    let gain_jpeg = insert_after_soi(&encode_jpeg(&gpx, gw as u32, gh as u32, image::ExtendedColorType::L8, 90, None)?, &[app_segment(0xE1, &gxmp)]);

    let primary_xmp = format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:Container="http://ns.google.com/photos/1.0/container/" xmlns:Item="http://ns.google.com/photos/1.0/container/item/" xmlns:hdrgm="http://ns.adobe.com/hdr-gain-map/1.0/" hdrgm:Version="1.0"><Container:Directory><rdf:Seq><rdf:li rdf:parseType="Resource"><Container:Item Item:Semantic="Primary" Item:Mime="image/jpeg"/></rdf:li><rdf:li rdf:parseType="Resource"><Container:Item Item:Semantic="GainMap" Item:Mime="image/jpeg" Item:Length="{}"/></rdf:li></rdf:Seq></Container:Directory></rdf:Description></rdf:RDF></x:xmpmeta>"#,
        gain_jpeg.len()
    );
    let mut pxmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    pxmp.extend(primary_xmp.as_bytes());
    let base = encode_jpeg(sdr8, w as u32, h as u32, image::ExtendedColorType::Rgb8, quality, Some(icc))?;
    let mut segs = vec![];
    if let Some(e) = exif {
        segs.push(app_segment(0xE1, e));
    }
    segs.push(app_segment(0xE1, &pxmp));
    // MPF needs the final primary length; its own size is fixed, so build twice
    let placeholder = mpf_segment(0, 0, 0);
    let mut with_mpf = segs.clone();
    with_mpf.push(placeholder.clone());
    let primary_len = insert_after_soi(&base, &with_mpf).len() as u32;
    // offsets in MPF are relative to the TIFF header inside the MPF segment
    let mpf_pos = 2 + segs.iter().map(|s| s.len()).sum::<usize>() as u32;
    let tiff_start = mpf_pos + 4 + 4; // marker+len, "MPF\0"
    let gain_offset = primary_len - tiff_start;
    segs.push(mpf_segment(primary_len, gain_jpeg.len() as u32, gain_offset));
    let mut out = insert_after_soi(&base, &segs);
    out.extend(gain_jpeg);
    std::fs::write(path, out)?;
    Ok(())
}
