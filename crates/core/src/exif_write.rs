//! Tiny EXIF (TIFF IFD) writer for exported JPEGs.

use crate::decode::Metadata;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum MetadataMode {
    #[default]
    All,
    AllExceptLocation,
    CopyrightOnly,
    None,
}

enum Val {
    Ascii(String),
    Short(u16),
    Rational(u32, u32),
    SRational(i32, i32),
    Ifd(Vec<(u16, Val)>),
    Rationals(Vec<(u32, u32)>),
}

fn rat(v: f32) -> (u32, u32) {
    if v <= 0.0 {
        return (0, 1);
    }
    if v < 1.0 {
        let d = (1.0 / v).round() as u32;
        if ((1.0 / d as f32) - v).abs() < v * 0.01 {
            return (1, d);
        }
    }
    ((v * 1000.0).round() as u32, 1000)
}

fn dms(v: f64) -> Vec<(u32, u32)> {
    let v = v.abs();
    let d = v.floor();
    let m = ((v - d) * 60.0).floor();
    let s = ((v - d) * 60.0 - m) * 60.0;
    vec![(d as u32, 1), (m as u32, 1), ((s * 100.0).round() as u32, 100)]
}

/// Build an APP1 EXIF payload ("Exif\0\0" + TIFF) from metadata.
pub fn exif_block(meta: &Metadata, mode: MetadataMode, software: &str) -> Option<Vec<u8>> {
    if mode == MetadataMode::None {
        return None;
    }
    let mut ifd0: Vec<(u16, Val)> = vec![];
    let mut exif: Vec<(u16, Val)> = vec![];
    if let Some(c) = &meta.copyright {
        ifd0.push((0x8298, Val::Ascii(c.clone())));
    }
    if let Some(a) = &meta.artist {
        ifd0.push((0x013B, Val::Ascii(a.clone())));
    }
    if mode != MetadataMode::CopyrightOnly {
        if !meta.make.is_empty() {
            ifd0.push((0x010F, Val::Ascii(meta.make.clone())));
        }
        if !meta.model.is_empty() {
            ifd0.push((0x0110, Val::Ascii(meta.model.clone())));
        }
        ifd0.push((0x0112, Val::Short(1)));
        ifd0.push((0x0131, Val::Ascii(software.into())));
        if let Some(d) = &meta.date {
            ifd0.push((0x0132, Val::Ascii(d.clone())));
            exif.push((0x9003, Val::Ascii(d.clone())));
        }
        if let Some(t) = meta.exposure_time {
            let (n, d) = rat(t);
            exif.push((0x829A, Val::Rational(n, d)));
        }
        if let Some(f) = meta.fnumber {
            let (n, d) = rat(f);
            exif.push((0x829D, Val::Rational(n, d)));
        }
        if let Some(iso) = meta.iso {
            exif.push((0x8827, Val::Short(iso.min(65535) as u16)));
        }
        if let Some(fl) = meta.focal_length {
            let (n, d) = rat(fl);
            exif.push((0x920A, Val::Rational(n, d)));
        }
        if let Some(l) = &meta.lens {
            exif.push((0xA434, Val::Ascii(l.clone())));
        }
        if let Some(s) = &meta.serial {
            exif.push((0xA431, Val::Ascii(s.clone())));
        }
        let _ = Val::SRational(0, 1);
        if mode == MetadataMode::All {
            if let Some([lat, lon]) = meta.gps {
                let gps = vec![
                    (0x0001, Val::Ascii(if lat >= 0.0 { "N" } else { "S" }.into())),
                    (0x0002, Val::Rationals(dms(lat))),
                    (0x0003, Val::Ascii(if lon >= 0.0 { "E" } else { "W" }.into())),
                    (0x0004, Val::Rationals(dms(lon))),
                ];
                ifd0.push((0x8825, Val::Ifd(gps)));
            }
        }
    }
    if !exif.is_empty() {
        ifd0.push((0x8769, Val::Ifd(exif)));
    }
    if ifd0.is_empty() {
        return None;
    }
    let mut tiff = b"II*\0\x08\0\0\0".to_vec();
    write_ifd(&mut tiff, ifd0);
    let mut out = b"Exif\0\0".to_vec();
    out.extend(tiff);
    Some(out)
}

fn write_ifd(buf: &mut Vec<u8>, mut entries: Vec<(u16, Val)>) {
    entries.sort_by_key(|e| e.0);
    let start = buf.len();
    let n = entries.len();
    let data_start = start + 2 + n * 12 + 4;
    let mut data: Vec<u8> = vec![];
    let mut table: Vec<u8> = vec![];
    table.extend((n as u16).to_le_bytes());
    let mut sub_ifds: Vec<(usize, Vec<(u16, Val)>)> = vec![];
    for (tag, v) in entries {
        table.extend(tag.to_le_bytes());
        let pad = |data: &mut Vec<u8>| {
            if data.len() % 2 == 1 {
                data.push(0);
            }
        };
        match v {
            Val::Ascii(s) => {
                let mut b = s.into_bytes();
                b.push(0);
                table.extend(2u16.to_le_bytes());
                table.extend((b.len() as u32).to_le_bytes());
                if b.len() <= 4 {
                    let mut v4 = b.clone();
                    v4.resize(4, 0);
                    table.extend(v4);
                } else {
                    pad(&mut data);
                    table.extend(((data_start + data.len()) as u32).to_le_bytes());
                    data.extend(b);
                }
            }
            Val::Short(s) => {
                table.extend(3u16.to_le_bytes());
                table.extend(1u32.to_le_bytes());
                table.extend(s.to_le_bytes());
                table.extend([0, 0]);
            }
            Val::Rational(n, d) => {
                table.extend(5u16.to_le_bytes());
                table.extend(1u32.to_le_bytes());
                pad(&mut data);
                table.extend(((data_start + data.len()) as u32).to_le_bytes());
                data.extend(n.to_le_bytes());
                data.extend(d.to_le_bytes());
            }
            Val::Rationals(v) => {
                table.extend(5u16.to_le_bytes());
                table.extend((v.len() as u32).to_le_bytes());
                pad(&mut data);
                table.extend(((data_start + data.len()) as u32).to_le_bytes());
                for (n, d) in v {
                    data.extend(n.to_le_bytes());
                    data.extend(d.to_le_bytes());
                }
            }
            Val::SRational(n, d) => {
                table.extend(10u16.to_le_bytes());
                table.extend(1u32.to_le_bytes());
                pad(&mut data);
                table.extend(((data_start + data.len()) as u32).to_le_bytes());
                data.extend(n.to_le_bytes());
                data.extend(d.to_le_bytes());
            }
            Val::Ifd(sub) => {
                table.extend(4u16.to_le_bytes());
                table.extend(1u32.to_le_bytes());
                // patched after layout
                sub_ifds.push((table.len(), sub));
                table.extend(0u32.to_le_bytes());
            }
        }
    }
    table.extend(0u32.to_le_bytes()); // next IFD
    buf.extend(&table);
    buf.extend(&data);
    for (pos, sub) in sub_ifds {
        if buf.len() % 2 == 1 {
            buf.push(0);
        }
        // offsets are relative to the TIFF header, which starts `buf`
        let off = buf.len() as u32;
        let at = start + pos;
        buf[at..at + 4].copy_from_slice(&off.to_le_bytes());
        write_ifd(buf, sub);
    }
}

/// Insert an APP1 segment right after SOI of a JPEG stream.
pub fn insert_app1(jpeg: &[u8], payload: &[u8]) -> Vec<u8> {
    if jpeg.len() < 2 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 || payload.len() > 65533 {
        return jpeg.to_vec();
    }
    let mut out = Vec::with_capacity(jpeg.len() + payload.len() + 4);
    out.extend(&jpeg[..2]);
    out.extend([0xFF, 0xE1]);
    out.extend(((payload.len() + 2) as u16).to_be_bytes());
    out.extend(payload);
    out.extend(&jpeg[2..]);
    out
}
