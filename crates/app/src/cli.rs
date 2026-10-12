//! Headless batch mode: `darkslide --cli IN... [options]`.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use darkslide_core::export::{self, ExportSettings, Format, Resize};
use darkslide_core::xmp::{self, CrsValue};
use darkslide_core::{DevelopSettings, Engine};

const HELP: &str = "usage: darkslide --cli [options] FILE...
  -o, --out DIR          output folder (default: next to source)
  -f, --format FMT       jpg | tif | png | exr | psd | avif | jxl | dng (default jpg)
  -q, --quality N        JPEG quality 1-100 (default 92)
  -b, --bits N           8 | 16 | 32
  -s, --size N           resize long edge to N px
      --space NAME       srgb | adobergb | p3 | prophoto | rec2020
  -x, --xmp FILE         apply settings / preset from an XMP file
      --set KEY=VALUE    set a crs: value, e.g. --set Exposure2012=+0.5
      --auto             auto tone
      --upright MODE     auto | level | vertical | full
      --constrain        crop away areas outside the image after transforms
      --json FILE        apply full settings JSON
      --no-sidecar       ignore existing .xmp sidecars
      --name TEMPLATE    {name} {seq} {date} {camera}
      --merge-hdr        merge the FILEs (brackets) into an HDR DNG
      --merge-pano       merge the FILEs into a panorama DNG
      --projection P     spherical | cylindrical | perspective (pano)
      --deghost L        off | low | medium | high (HDR)";

pub fn run(args: Vec<String>) -> Result<()> {
    let mut files = vec![];
    let mut ex = ExportSettings::default();
    let mut xmp_file: Option<PathBuf> = None;
    let mut json_file: Option<PathBuf> = None;
    let mut sets: HashMap<String, CrsValue> = HashMap::new();
    let mut auto = false;
    let mut upright: Option<darkslide_core::params::Upright> = None;
    let mut constrain = false;
    let mut use_sidecar = true;
    let mut merge: Option<&str> = None;
    let mut projection = darkslide_core::merge::Projection::Spherical;
    let mut deghost = darkslide_core::merge::Deghost::Off;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or_else(|| anyhow!("{a} needs a value"));
        match a.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(());
            }
            "-o" | "--out" => ex.dest = Some(val()?.into()),
            "-f" | "--format" => {
                ex.format = match val()?.to_lowercase().as_str() {
                    "jpg" | "jpeg" => Format::Jpeg,
                    "tif" | "tiff" => Format::Tiff,
                    "png" => Format::Png,
                    "exr" => Format::Exr,
                    "psd" => Format::Psd,
                    "avif" => Format::Avif,
                    "jxl" => Format::Jxl,
                    "dng" => Format::Dng,
                    f => bail!("unknown format {f}"),
                }
            }
            "-q" | "--quality" => ex.jpeg_quality = val()?.parse()?,
            "-b" | "--bits" => ex.bit_depth = val()?.parse()?,
            "-s" | "--size" => ex.resize = Resize::LongEdge(val()?.parse()?),
            "--space" => {
                use darkslide_core::color::ColorSpace::*;
                ex.color_space = match val()?.to_lowercase().as_str() {
                    "srgb" => SRgb,
                    "adobergb" => AdobeRgb,
                    "p3" => DisplayP3,
                    "prophoto" => ProPhoto,
                    "rec2020" => Rec2020,
                    s => bail!("unknown space {s}"),
                }
            }
            "-x" | "--xmp" => xmp_file = Some(val()?.into()),
            "--json" => json_file = Some(val()?.into()),
            "--set" => {
                let kv = val()?;
                let (k, v) = kv.split_once('=').ok_or_else(|| anyhow!("--set expects KEY=VALUE"))?;
                sets.insert(k.trim_start_matches("crs:").to_string(), CrsValue::Text(v.to_string()));
            }
            "--auto" => auto = true,
            "--constrain" => constrain = true,
            "--upright" => {
                use darkslide_core::params::Upright::*;
                upright = Some(match val()?.as_str() {
                    "level" => Level,
                    "vertical" => Vertical,
                    "full" => Full,
                    _ => Auto,
                })
            }
            "--no-sidecar" => use_sidecar = false,
            "--merge-hdr" => merge = Some("hdr"),
            "--merge-pano" => merge = Some("pano"),
            "--projection" => {
                use darkslide_core::merge::Projection::*;
                projection = match val()?.as_str() {
                    "cylindrical" => Cylindrical,
                    "perspective" => Perspective,
                    _ => Spherical,
                }
            }
            "--deghost" => {
                use darkslide_core::merge::Deghost::*;
                deghost = match val()?.as_str() {
                    "low" => Low,
                    "medium" => Medium,
                    "high" => High,
                    _ => Off,
                }
            }
            "--name" => ex.naming = val()?,
            _ if a.starts_with('-') => bail!("unknown option {a}\n{HELP}"),
            _ => files.push(PathBuf::from(a)),
        }
    }
    if files.is_empty() {
        bail!("{HELP}");
    }
    if let Some(kind) = merge {
        use darkslide_core::merge;
        let t = std::time::Instant::now();
        let (img, first) = if kind == "hdr" {
            merge::merge_hdr(&files, &merge::HdrOptions { align: true, deghost }, &|_| {})?
        } else {
            merge::merge_panorama(&files, &merge::PanoOptions { projection, auto_crop: true, fill_edges: false, boundary_warp: 0.0 }, &|_| {})?
        };
        let mut out = merge::output_path(&first, if kind == "hdr" { "HDR" } else { "Pano" });
        if let Some(d) = &ex.dest {
            out = d.join(out.file_name().unwrap());
        }
        merge::write_linear_dng(&out, &img, &first)?;
        println!("{} ({}x{}, {:.1}s)", out.display(), img.w, img.h, t.elapsed().as_secs_f32());
        return Ok(());
    }
    for (i, f) in files.iter().enumerate() {
        let t = std::time::Instant::now();
        let engine = Engine::open(f)?;
        let is_raw = engine.is_raw();
        let mut s = if use_sidecar { xmp::read_sidecar(f, is_raw).map(|(s, _)| s) } else { None }.unwrap_or_else(|| DevelopSettings::defaults_for(is_raw));
        if let Some(p) = &json_file {
            s = serde_json::from_str(&std::fs::read_to_string(p)?)?;
        }
        if let Some(p) = &xmp_file {
            let preset = xmp::load_preset(p).ok_or_else(|| anyhow!("cannot read {}", p.display()))?;
            s = preset.apply(&s, 100.0);
        }
        if auto {
            let v = engine.auto_tone(&s);
            s.exposure = v[0];
            s.contrast = v[1];
            s.highlights = v[2];
            s.shadows = v[3];
            s.whites = v[4];
            s.blacks = v[5];
        }
        if let Some(mode) = upright {
            s.transform.upright_matrix = None;
            match darkslide_core::upright::analyze(&engine, &s, mode) {
                Some(m) => {
                    s.transform.upright = mode;
                    s.transform.upright_matrix = Some(m);
                }
                None => eprintln!("{}: upright found no usable lines", f.display()),
            }
        }
        if constrain {
            s.transform.constrain_crop = true;
        }
        if !sets.is_empty() {
            if sets.contains_key("Temperature") || sets.contains_key("Tint") {
                s.wb_mode = darkslide_core::params::WbMode::Custom;
            }
            xmp::apply_crs(&sets, &mut s);
        }
        if s.transform.constrain_crop {
            engine.constrain_crop(&mut s);
        }
        let out = export::save(&engine, &s, &ex, ex.start_seq + i as u32, &|_| {})?;
        println!("{} -> {} ({:.2}s)", f.display(), out.display(), t.elapsed().as_secs_f32());
    }
    Ok(())
}
