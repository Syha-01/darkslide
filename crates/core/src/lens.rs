//! Lens corrections: manual distortion, lens profiles (lensfun-style
//! polynomial models) and automatic lateral CA estimation.

use rayon::prelude::*;

use crate::image::{Image3, Plane};
use crate::params::LensCorrection;
use crate::pipeline::geometry::LensModel;

/// Distortion/vignetting/TCA coefficients for one lens at one focal length.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LensProfile {
    pub maker: String,
    pub model: String,
    /// lensfun poly3/ptlens: r_d = r_u * (a r^3 + b r^2 + c r + 1 - a - b - c)
    pub ptlens: [f64; 3],
    /// lensfun pa vignetting: k1, k2, k3
    pub vignette: [f64; 3],
    /// TCA linear terms (vr, vb) as radial scales
    pub tca: [f64; 2],
    /// Even radial polynomial (k1, k2, k3) on r² normalised to the half
    /// diagonal (Adobe LCP profiles, converted on lookup).
    pub poly: Option<[f64; 3]>,
}

/// Build the geometric lens model for rendering.
pub fn model_for(lc: &LensCorrection, profile: Option<&LensProfile>, auto_ca: Option<[f64; 2]>) -> LensModel {
    let mut m = LensModel::default();
    // manual: +amount corrects barrel (samples closer to centre)
    let k = -(lc.manual_distortion as f64) / 100.0 * 0.12;
    m.k[0] += k;
    if lc.profile_enable {
        if let Some(p) = profile {
            let s = lc.distortion_scale as f64 / 100.0;
            match p.poly {
                Some(k) => {
                    for i in 0..3 {
                        m.k[i] += k[i] * s;
                    }
                }
                None => m.ptlens = Some(p.ptlens.map(|v| v * s)),
            }
            if lc.remove_ca {
                m.ca_scale = [p.tca[0] - 1.0, p.tca[1] - 1.0];
            }
        }
    }
    if lc.remove_ca && m.ca_scale == [0.0, 0.0] {
        if let Some(ca) = auto_ca {
            m.ca_scale = ca;
        }
    }
    m
}

/// Estimate lateral CA as radial scale factors of R and B relative to G. Each
/// strong radial edge gives a sub-pixel shift (normalised correlation of the
/// channel's radial profile against G, so edge colour doesn't bias it); a
/// robust fit of shift = scale * radius over all edges gives the scale.
pub fn estimate_lateral_ca(img: &Image3) -> [f64; 2] {
    let f = (img.w.max(img.h) / 2000).max(1);
    let s = img.downscale_int(f);
    let (w, h) = (s.w, s.h);
    let enc = |c: usize| s.channel(c).map(|v| v.max(0.0).sqrt());
    let g = enc(1);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let rmax = (cx * cx + cy * cy).sqrt();
    const HALF: i32 = 6;
    const MAXD: f32 = 3.0;
    let margin = (HALF as f32 + MAXD + 2.0) as usize;
    // strongest radial edges away from the centre, where CA is measurable
    let mut pts: Vec<(f32, usize, usize)> = (margin..h.saturating_sub(margin))
        .into_par_iter()
        .step_by(2)
        .flat_map_iter(|y| {
            let g = &g;
            (margin..w.saturating_sub(margin)).step_by(2).filter_map(move |x| {
                let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                let r = (dx * dx + dy * dy).sqrt();
                if r < 0.3 * rmax {
                    return None;
                }
                let gx = g.get(x + 1, y) - g.get(x - 1, y);
                let gy = g.get(x, y + 1) - g.get(x, y - 1);
                let rad = (gx * dx + gy * dy) / r;
                // mostly radial edges: tangential ones carry no radial shift
                (rad.abs() > 0.03 && rad * rad > 0.6 * (gx * gx + gy * gy)).then_some((rad.abs(), x, y))
            })
        })
        .collect();
    if pts.len() < 100 {
        return [0.0, 0.0];
    }
    pts.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    pts.truncate(30_000);
    let profile = |p: &Plane, x: f32, y: f32, ux: f32, uy: f32, d: f32| -> [f32; (2 * HALF + 1) as usize] {
        let mut o = [0.0; (2 * HALF + 1) as usize];
        for (k, t) in (-HALF..=HALF).enumerate() {
            let t = t as f32 + d;
            o[k] = p.sample_bilinear(x + ux * t, y + uy * t);
        }
        o
    };
    let ncc = |a: &[f32], b: &[f32]| -> f32 {
        let n = a.len() as f32;
        let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
        let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(b) {
            let (x, y) = (x - ma, y - mb);
            sab += x * y;
            saa += x * x;
            sbb += y * y;
        }
        if saa < 1e-8 || sbb < 1e-8 { -1.0 } else { sab / (saa * sbb).sqrt() }
    };
    let mut out = [0.0; 2];
    for (k, ch) in [0usize, 2].into_iter().enumerate() {
        let c = enc(ch);
        // (radius, unit dir, shift) per edge
        let obs: Vec<(f32, f32, f32, f32)> = pts
            .par_iter()
            .filter_map(|&(_, x, y)| {
                let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
                let (dx, dy) = (x - cx, y - cy);
                let r = (dx * dx + dy * dy).sqrt();
                let (ux, uy) = (dx / r, dy / r);
                let gp = profile(&g, x, y, ux, uy, 0.0);
                let score = |d: f32| ncc(&gp, &profile(&c, x, y, ux, uy, d));
                let mut best = (-2.0, 0.0);
                let mut d = -MAXD;
                while d <= MAXD + 1e-3 {
                    let v = score(d);
                    if v > best.0 {
                        best = (v, d);
                    }
                    d += 0.25;
                }
                if best.0 < 0.9 || best.1.abs() >= MAXD {
                    return None;
                }
                // parabolic refinement around the best step
                let (a, b, cc) = (score(best.1 - 0.25), best.0, score(best.1 + 0.25));
                let den = a - 2.0 * b + cc;
                // soft (defocused) edges correlate at any shift; they only add noise
                if den > -0.004 {
                    return None;
                }
                let off = (0.5 * (a - cc) / den).clamp(-0.5, 0.5) * 0.25;
                Some((r, ux, uy, best.1 + off))
            })
            .collect();
        if obs.len() < 50 {
            continue;
        }
        // shift = scale * r + (tx, ty) . u: the translation soaks up channel
        // offsets from demosaicing (X-Trans drafts) that aren't lens CA
        let fit = |o: &[(f32, f32, f32, f32)]| -> Option<[f64; 3]> {
            let mut a = [[0.0f64; 3]; 3];
            let mut v = [0.0f64; 3];
            for &(r, ux, uy, s) in o {
                let row = [r as f64 / rmax as f64, ux as f64, uy as f64];
                for i in 0..3 {
                    for j in 0..3 {
                        a[i][j] += row[i] * row[j];
                    }
                    v[i] += row[i] * s as f64;
                }
            }
            solve3(a, v)
        };
        let resid =
            |p: &[f64; 3], &(r, ux, uy, s): &(f32, f32, f32, f32)| (s as f64 - p[0] * r as f64 / rmax as f64 - p[1] * ux as f64 - p[2] * uy as f64).abs();
        let mut keep = obs;
        let mut sc = 0.0f64;
        for _ in 0..4 {
            let Some(p) = fit(&keep) else { break };
            sc = p[0] / rmax as f64;
            let mut res: Vec<f64> = keep.iter().map(|o| resid(&p, o)).collect();
            res.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let mad = res[res.len() / 2].max(0.02);
            let next: Vec<_> = keep.iter().copied().filter(|o| resid(&p, o) <= 2.5 * mad).collect();
            if next.len() < 50 {
                break;
            }
            keep = next;
        }
        out[k] = sc.clamp(-0.01, 0.01);
    }
    out
}

/// Cramer's rule; None when singular.
fn solve3(a: [[f64; 3]; 3], v: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(&a);
    if d.abs() < 1e-12 {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut m = a;
        for i in 0..3 {
            m[i][k] = v[i];
        }
        *o = det(&m) / d;
    }
    Some(out)
}

/// Vignette gain from a profile at normalised radius r.
pub fn profile_vignette_gain(p: &LensProfile, r: f64, scale: f64) -> f64 {
    let [k1, k2, k3] = p.vignette;
    let r2 = r * r;
    let v = 1.0 + k1 * r2 + k2 * r2 * r2 + k3 * r2 * r2 * r2;
    1.0 + (1.0 / v.max(0.1) - 1.0) * scale
}

/// Apply profile vignetting correction to a region given T-frame coords.
pub fn apply_profile_vignette(img: &mut Image3, p: &LensProfile, scale: f64, tco: &[[f32; 2]], aspect: f32) {
    let (sx, sy) = if aspect >= 1.0 { (1.0, 1.0 / aspect) } else { (aspect, 1.0) };
    let norm = ((sx * sx + sy * sy) as f64).sqrt();
    img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        let t = tco[i];
        let x = ((t[0] - 0.5) * 2.0 * sx) as f64;
        let y = ((t[1] - 0.5) * 2.0 * sy) as f64;
        let r = (x * x + y * y).sqrt() / norm;
        let g = profile_vignette_gain(p, r, scale) as f32;
        *px = px.map(|c| c * g);
    });
}

// ---------------------------------------------------------------------------
// lensfun database (XML). Looked up in the usual system locations and in
// ~/.local/share/darkslide/lensfun.

pub fn lensfun_dirs() -> Vec<std::path::PathBuf> {
    vec![
        crate::paths::data_dir().join("lensfun"),
        "/usr/share/lensfun/version_1".into(),
        "/usr/share/lensfun".into(),
        "/usr/local/share/lensfun/version_1".into(),
        "/var/lib/lensfun-updates/version_1".into(),
    ]
}

#[derive(Clone, Debug, Default)]
pub struct LensfunLens {
    pub maker: String,
    pub model: String,
    /// focal -> (a, b, c)
    pub distortion: Vec<(f64, [f64; 3])>,
    /// focal -> (vr, vb)
    pub tca: Vec<(f64, [f64; 2])>,
    /// (focal, aperture) -> k1..k3
    pub vignetting: Vec<(f64, f64, [f64; 3])>,
}

const LENSFUN_DB_URL: &str = "https://lensfun.github.io/db/version_1.tar.bz2";

/// Fetch the current lensfun database into ~/.local/share/darkslide/lensfun
/// (uses curl and tar). Returns the number of lenses it holds.
pub fn download_lensfun_db() -> anyhow::Result<usize> {
    use anyhow::{Context, bail};
    let dest = crate::paths::data_dir().join("lensfun");
    let tmp = crate::paths::data_dir().join("lensfun.download");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("db"))?;
    let tarball = tmp.join("db.tar.bz2");
    let st =
        std::process::Command::new("curl").args(["-fsSL", "--max-time", "120", "-o"]).arg(&tarball).arg(LENSFUN_DB_URL).status().context("running curl")?;
    if !st.success() {
        bail!("download failed (curl exit {})", st.code().unwrap_or(-1));
    }
    let st = std::process::Command::new("tar").arg("xjf").arg(&tarball).arg("-C").arg(tmp.join("db")).status().context("running tar")?;
    if !st.success() {
        bail!("couldn't unpack the database");
    }
    let n: usize = std::fs::read_dir(tmp.join("db"))?
        .flatten()
        .filter(|e| e.path().extension().and_then(|e| e.to_str()) == Some("xml"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .map(|t| parse_lensfun_xml(&t).len())
        .sum();
    if n == 0 {
        bail!("the download held no lenses");
    }
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(tmp.join("db"), &dest)?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(n)
}

pub fn load_lensfun_db() -> Vec<LensfunLens> {
    let mut lenses = vec![];
    for d in lensfun_dirs() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|e| e.to_str()) == Some("xml") {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    lenses.extend(parse_lensfun_xml(&text));
                }
            }
        }
        if !lenses.is_empty() {
            break;
        }
    }
    lenses
}

pub fn parse_lensfun_xml(text: &str) -> Vec<LensfunLens> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(text);
    let mut out = vec![];
    let mut cur: Option<LensfunLens> = None;
    let mut field = String::new();
    let mut buf = vec![];
    let attr = |e: &quick_xml::events::BytesStart, k: &str| -> Option<f64> {
        e.attributes().flatten().find(|a| a.key.as_ref() == k.as_bytes()).and_then(|a| String::from_utf8_lossy(&a.value).parse().ok())
    };
    let attr_s = |e: &quick_xml::events::BytesStart, k: &str| -> Option<String> {
        e.attributes().flatten().find(|a| a.key.as_ref() == k.as_bytes()).map(|a| String::from_utf8_lossy(&a.value).to_string())
    };
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "lens" {
                    cur = Some(LensfunLens::default());
                }
                field = name;
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if let Some(l) = cur.as_mut() {
                    let focal = attr(&e, "focal").unwrap_or(0.0);
                    match name.as_str() {
                        "distortion" => {
                            let model = attr_s(&e, "model").unwrap_or_default();
                            let a = attr(&e, "a").unwrap_or(0.0);
                            let b = attr(&e, "b").unwrap_or(0.0);
                            let c = attr(&e, "c").unwrap_or(0.0);
                            let k1 = attr(&e, "k1").unwrap_or(0.0);
                            match model.as_str() {
                                "ptlens" => l.distortion.push((focal, [a, b, c])),
                                "poly3" => l.distortion.push((focal, [0.0, 0.0, k1])),
                                _ => {}
                            }
                        }
                        "tca" => {
                            let vr = attr(&e, "vr").unwrap_or(1.0);
                            let vb = attr(&e, "vb").unwrap_or(1.0);
                            l.tca.push((focal, [vr, vb]));
                        }
                        "vignetting" => {
                            let ap = attr(&e, "aperture").unwrap_or(0.0);
                            l.vignetting.push((focal, ap, [attr(&e, "k1").unwrap_or(0.0), attr(&e, "k2").unwrap_or(0.0), attr(&e, "k3").unwrap_or(0.0)]));
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(l) = cur.as_mut() {
                    let s = t.decode().map(|c| c.to_string()).unwrap_or_default();
                    match field.as_str() {
                        "maker" if l.maker.is_empty() => l.maker = s,
                        "model" if l.model.is_empty() => l.model = s,
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"lens" {
                    if let Some(l) = cur.take() {
                        out.push(l);
                    }
                }
                field.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

impl LensfunLens {
    /// Interpolate a profile for a focal length / aperture.
    pub fn at(&self, focal: f64, aperture: f64) -> LensProfile {
        fn interp<const N: usize>(v: &[(f64, [f64; N])], f: f64, def: [f64; N]) -> [f64; N] {
            if v.is_empty() {
                return def;
            }
            let mut v = v.to_vec();
            v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            if f <= v[0].0 {
                return v[0].1;
            }
            if f >= v[v.len() - 1].0 {
                return v[v.len() - 1].1;
            }
            for i in 0..v.len() - 1 {
                if f >= v[i].0 && f <= v[i + 1].0 {
                    let t = (f - v[i].0) / (v[i + 1].0 - v[i].0).max(1e-9);
                    let mut o = [0.0; N];
                    for k in 0..N {
                        o[k] = v[i].1[k] + (v[i + 1].1[k] - v[i].1[k]) * t;
                    }
                    return o;
                }
            }
            def
        }
        // vignetting: nearest aperture, then interpolate focal
        let mut vig = vec![];
        if !self.vignetting.is_empty() {
            let best_ap = self.vignetting.iter().map(|v| v.1).min_by(|a, b| (a - aperture).abs().partial_cmp(&(b - aperture).abs()).unwrap()).unwrap();
            vig = self.vignetting.iter().filter(|v| v.1 == best_ap).map(|v| (v.0, v.2)).collect();
        }
        LensProfile {
            maker: self.maker.clone(),
            model: self.model.clone(),
            ptlens: interp(&self.distortion, focal, [0.0; 3]),
            vignette: interp(&vig, focal, [0.0; 3]),
            tca: interp(&self.tca, focal, [1.0, 1.0]),
            poly: None,
        }
    }
}

/// Find a lens in the database by EXIF lens name (fuzzy, token overlap).
pub fn find_lens<'a>(db: &'a [LensfunLens], name: &str) -> Option<&'a LensfunLens> {
    let toks = |s: &str| -> Vec<String> {
        s.to_lowercase().split(|c: char| !c.is_alphanumeric() && c != '.').filter(|t| !t.is_empty()).map(|t| t.to_string()).collect()
    };
    let q = toks(name);
    if q.is_empty() {
        return None;
    }
    let mut best: Option<(f32, &LensfunLens)> = None;
    for l in db {
        let lt = toks(&format!("{} {}", l.maker, l.model));
        let hits = q.iter().filter(|t| lt.contains(t)).count() as f32;
        let score = hits / q.len().max(lt.len()) as f32;
        if score > 0.5 && best.map(|b| score > b.0).unwrap_or(true) {
            best = Some((score, l));
        }
    }
    best.map(|b| b.1)
}

// ---------------------------------------------------------------------------
// Adobe LCP lens profiles

#[derive(Clone, Debug, Default)]
pub struct LcpEntry {
    pub make: String,
    pub lens: String,
    pub focal: f64,
    pub aperture: f64,
    /// focal length as a fraction of the long image side
    pub fx: f64,
    pub dist: [f64; 3],
    pub vig: [f64; 3],
    pub ca: [f64; 2],
}

fn props(n: &crate::xmp_masks::Node) -> &crate::xmp_masks::Node {
    n.child("rdf:Description").unwrap_or(n)
}

pub fn parse_lcp(text: &str) -> Vec<LcpEntry> {
    let dom = crate::xmp_masks::parse_dom(text);
    let Some(list) = dom.find("photoshop:CameraProfiles") else { return vec![] };
    let mut out = vec![];
    for item in list.items() {
        let it = props(item);
        let Some(pm) = it.child("stCamera:PerspectiveModel").map(props) else { continue };
        let f = |n: &crate::xmp_masks::Node, k: &str| n.f(k).map(|v| v as f64);
        let mut e = LcpEntry {
            make: it.attr("stCamera:Make").unwrap_or_default().to_string(),
            lens: it.attr("stCamera:LensPrettyName").or_else(|| it.attr("stCamera:Lens")).unwrap_or_default().to_string(),
            focal: f(it, "stCamera:FocalLength").unwrap_or(0.0),
            aperture: f(it, "stCamera:ApertureValue").map(|av| 2f64.powf(av / 2.0)).unwrap_or(8.0),
            fx: f(pm, "stCamera:FocalLengthX").unwrap_or(1.0),
            dist: [
                f(pm, "stCamera:RadialDistortParam1").unwrap_or(0.0),
                f(pm, "stCamera:RadialDistortParam2").unwrap_or(0.0),
                f(pm, "stCamera:RadialDistortParam3").unwrap_or(0.0),
            ],
            vig: [0.0; 3],
            ca: [1.0, 1.0],
        };
        if let Some(v) = pm.child("stCamera:VignetteModel").map(props) {
            e.vig = [
                f(v, "stCamera:VignetteModelParam1").unwrap_or(0.0),
                f(v, "stCamera:VignetteModelParam2").unwrap_or(0.0),
                f(v, "stCamera:VignetteModelParam3").unwrap_or(0.0),
            ];
        }
        if let Some(c) = pm.child("stCamera:ChromaticRedGreenModel").map(props) {
            e.ca[0] = f(c, "stCamera:ScaleFactor").unwrap_or(1.0);
        }
        if let Some(c) = pm.child("stCamera:ChromaticBlueGreenModel").map(props) {
            e.ca[1] = f(c, "stCamera:ScaleFactor").unwrap_or(1.0);
        }
        out.push(e);
    }
    out
}

pub fn lcp_dirs() -> Vec<std::path::PathBuf> {
    let mut v = vec![crate::paths::config_dir().join("lcp")];
    if let Some(h) = std::env::var_os("HOME") {
        v.push(std::path::PathBuf::from(h).join(".adobe/CameraRaw/LensProfiles/1.0"));
    }
    v
}

/// All LCP entries on disk (cached by the caller).
pub fn load_lcps() -> Vec<LcpEntry> {
    let mut out = vec![];
    fn walk(d: &std::path::Path, out: &mut Vec<LcpEntry>, depth: usize) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 4 {
                walk(&p, out, depth + 1);
            } else if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("lcp")).unwrap_or(false) {
                if let Ok(t) = std::fs::read_to_string(&p) {
                    out.extend(parse_lcp(&t));
                }
            }
        }
    }
    for d in lcp_dirs() {
        walk(&d, &mut out, 0);
    }
    out
}

/// Pick the LCP entries for a lens name and convert the nearest focal length
/// to our model for an image of `w` x `h` pixels.
pub fn lcp_profile(entries: &[LcpEntry], lens: &str, focal: f64, w: f64, h: f64) -> Option<LensProfile> {
    let q = lens.to_lowercase();
    let cands: Vec<&LcpEntry> =
        entries.iter().filter(|e| !e.lens.is_empty() && (q.contains(&e.lens.to_lowercase()) || e.lens.to_lowercase().contains(&q))).collect();
    let e = cands.iter().min_by(|a, b| (a.focal - focal).abs().partial_cmp(&(b.focal - focal).abs()).unwrap())?;
    // LCP radius is in units of the focal length (fx * long side, pixels)
    let fpx = e.fx * w.max(h);
    let half_diag = (w * w + h * h).sqrt() / 2.0;
    let c = (half_diag / fpx).powi(2);
    Some(LensProfile {
        maker: e.make.clone(),
        model: e.lens.clone(),
        ptlens: [0.0; 3],
        // Adobe's vignette model is the falloff itself (1 + a r^2 ...)
        vignette: [e.vig[0] * c, e.vig[1] * c * c, e.vig[2] * c * c * c],
        tca: e.ca,
        poly: Some([e.dist[0] * c, e.dist[1] * c * c, e.dist[2] * c * c * c]),
    })
}

#[cfg(test)]
mod lcp_tests {
    use super::*;

    #[test]
    fn finds_injected_lateral_ca() {
        // random hard-edged coloured discs, so edge colour can't bias the fit;
        // R and B are sampled at a radially scaled position
        let (w, h) = (1200usize, 800usize);
        let mut seed = 7u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as f32 / (1u64 << 31) as f32
        };
        let discs: Vec<(f32, f32, f32, [f32; 3])> = (0..300).map(|_| (rnd() * w as f32, rnd() * h as f32, 8.0 + rnd() * 40.0, [rnd(), rnd(), rnd()])).collect();
        let scene = |x: f32, y: f32| -> [f32; 3] {
            let mut c = [0.2, 0.2, 0.2];
            for &(dx, dy, r, col) in &discs {
                if (x - dx).powi(2) + (y - dy).powi(2) < r * r {
                    c = col;
                }
            }
            c
        };
        let (sr, sb) = (0.0015f32, -0.001f32);
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let img = Image3::from_fn(w, h, |x, y| {
            let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
            let at = |s: f32| scene(cx + (x - cx) * (1.0 + s), cy + (y - cy) * (1.0 + s));
            [at(sr)[0], scene(x, y)[1], at(sb)[2]]
        });
        let est = estimate_lateral_ca(&img);
        // the correction undoes the injected scale
        assert!((est[0] + sr as f64).abs() < 0.0003 && (est[1] + sb as f64).abs() < 0.0003, "{est:?}");
    }

    #[test]
    fn parses_lcp() {
        let t = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
        <rdf:Description rdf:about="" xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/" xmlns:stCamera="http://ns.adobe.com/photoshop/1.0/camera-profile">
        <photoshop:CameraProfiles><rdf:Seq><rdf:li rdf:parseType="Resource">
          <stCamera:Make>Canon</stCamera:Make><stCamera:Lens>EF24-70mm f/2.8L USM</stCamera:Lens>
          <stCamera:FocalLength>24</stCamera:FocalLength><stCamera:ApertureValue>3</stCamera:ApertureValue>
          <stCamera:PerspectiveModel rdf:parseType="Resource">
            <stCamera:FocalLengthX>0.68</stCamera:FocalLengthX>
            <stCamera:RadialDistortParam1>-0.12</stCamera:RadialDistortParam1>
            <stCamera:RadialDistortParam2>0.05</stCamera:RadialDistortParam2>
            <stCamera:VignetteModel rdf:parseType="Resource"><stCamera:VignetteModelParam1>-0.6</stCamera:VignetteModelParam1></stCamera:VignetteModel>
          </stCamera:PerspectiveModel>
        </rdf:li></rdf:Seq></photoshop:CameraProfiles></rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let e = parse_lcp(t);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].lens, "EF24-70mm f/2.8L USM");
        assert!((e[0].dist[0] + 0.12).abs() < 1e-6 && (e[0].vig[0] + 0.6).abs() < 1e-6);
        let p = lcp_profile(&e, "Canon EF24-70mm f/2.8L USM", 24.0, 6000.0, 4000.0).unwrap();
        assert!(p.poly.unwrap()[0] < 0.0);
    }
}
