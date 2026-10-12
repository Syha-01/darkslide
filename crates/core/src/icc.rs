//! Minimal ICC v2 matrix/TRC profile writer for the output colour spaces.

use crate::color::{ColorSpace, Mat3};

fn s15f16(v: f32) -> [u8; 4] {
    ((v as f64 * 65536.0).round() as i32).to_be_bytes()
}

fn xyz_tag(x: f32, y: f32, z: f32) -> Vec<u8> {
    let mut t = b"XYZ \0\0\0\0".to_vec();
    t.extend(s15f16(x));
    t.extend(s15f16(y));
    t.extend(s15f16(z));
    t
}

fn curv_tag(space: ColorSpace) -> Vec<u8> {
    let mut t = b"curv\0\0\0\0".to_vec();
    match space {
        ColorSpace::AdobeRgb => {
            t.extend(1u32.to_be_bytes());
            // u8Fixed8 gamma 2.19921875 = 563/256
            t.extend(563u16.to_be_bytes());
        }
        ColorSpace::ProPhoto => {
            t.extend(1u32.to_be_bytes());
            t.extend(((1.8f32 * 256.0) as u16).to_be_bytes());
        }
        _ => {
            let n = 1024u32;
            t.extend(n.to_be_bytes());
            for i in 0..n {
                let e = i as f32 / (n - 1) as f32;
                let v = space.decode(e).clamp(0.0, 1.0);
                t.extend(((v * 65535.0).round() as u16).to_be_bytes());
            }
        }
    }
    t
}

fn desc_tag(text: &str) -> Vec<u8> {
    let mut t = b"desc\0\0\0\0".to_vec();
    let ascii: Vec<u8> = text.bytes().chain(std::iter::once(0)).collect();
    t.extend((ascii.len() as u32).to_be_bytes());
    t.extend(&ascii);
    t.extend(0u32.to_be_bytes()); // unicode lang
    t.extend(0u32.to_be_bytes()); // unicode count
    t.extend(0u16.to_be_bytes()); // scriptcode code
    t.push(0); // scriptcode count
    t.extend([0u8; 67]);
    t
}

fn text_tag(text: &str) -> Vec<u8> {
    let mut t = b"text\0\0\0\0".to_vec();
    t.extend(text.bytes());
    t.push(0);
    t
}

/// Build an ICC v2 display profile for the space.
pub fn profile(space: ColorSpace) -> Vec<u8> {
    let m: Mat3 = space.to_xyz(); // already D50-adapted
    let trc = curv_tag(space);
    let tags: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"desc", desc_tag(space.name())),
        (*b"cprt", text_tag("No copyright, use freely")),
        (*b"wtpt", xyz_tag(0.9642, 1.0, 0.8249)),
        (*b"rXYZ", xyz_tag(m[0][0], m[1][0], m[2][0])),
        (*b"gXYZ", xyz_tag(m[0][1], m[1][1], m[2][1])),
        (*b"bXYZ", xyz_tag(m[0][2], m[1][2], m[2][2])),
        (*b"rTRC", trc.clone()),
        (*b"gTRC", trc.clone()),
        (*b"bTRC", trc),
    ];
    let header_len = 128;
    let table_len = 4 + tags.len() * 12;
    let mut offset = header_len + table_len;
    let mut table = vec![];
    let mut data = vec![];
    for (sig, body) in &tags {
        while offset % 4 != 0 {
            data.push(0);
            offset += 1;
        }
        table.extend(sig);
        table.extend((offset as u32).to_be_bytes());
        table.extend((body.len() as u32).to_be_bytes());
        data.extend(body);
        offset += body.len();
    }
    let total = header_len + table_len + data.len();
    let mut h = vec![0u8; 128];
    h[0..4].copy_from_slice(&(total as u32).to_be_bytes());
    h[4..8].copy_from_slice(b"none");
    h[8..12].copy_from_slice(&[0x02, 0x10, 0, 0]);
    h[12..16].copy_from_slice(b"mntr");
    h[16..20].copy_from_slice(b"RGB ");
    h[20..24].copy_from_slice(b"XYZ ");
    h[24..36].copy_from_slice(&[0x07, 0xea, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
    h[36..40].copy_from_slice(b"acsp");
    h[40..44].copy_from_slice(b"APPL");
    h[64..68].copy_from_slice(&0u32.to_be_bytes()); // perceptual intent
    h[68..72].copy_from_slice(&s15f16(0.9642));
    h[72..76].copy_from_slice(&s15f16(1.0));
    h[76..80].copy_from_slice(&s15f16(0.8249));
    let mut out = h;
    out.extend((tags.len() as u32).to_be_bytes());
    out.extend(table);
    out.extend(data);
    out
}
