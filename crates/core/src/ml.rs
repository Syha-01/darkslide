//! Optional ONNX Runtime models (feature `ml`). libonnxruntime is loaded at
//! runtime (ORT_DYLIB_PATH or the usual library paths); models live in
//! `~/.local/share/darkslide/models/`:
//!
//! - `subject.onnx`  salient object segmentation (U²-Net 320 / ISNet or RMBG 1024)
//! - `sky.onnx`      sky segmentation (U²-Net style output)
//! - `depth.onnx`    monocular depth (Depth Anything style, 518x518, inverse depth)
//!
//! Every entry point returns None when the runtime or model is unavailable so
//! callers fall back to the classical detectors.

use std::path::PathBuf;

use crate::image::{Image3, Plane};

pub fn models_dir() -> PathBuf {
    crate::paths::data_dir().join("models")
}

fn model(name: &str) -> Option<PathBuf> {
    let p = models_dir().join(name);
    p.exists().then_some(p)
}

#[cfg(feature = "ml")]
mod imp {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    use ort::session::Session;
    use ort::value::Tensor;

    fn runtime_ok() -> bool {
        static OK: OnceLock<bool> = OnceLock::new();
        *OK.get_or_init(|| {
            let path = std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("libonnxruntime.so"));
            match ort::init_from(&path) {
                Ok(b) => {
                    b.with_name("darkslide").commit();
                    true
                }
                Err(e) => {
                    log::info!("ONNX Runtime not available ({e}); using classical detectors");
                    false
                }
            }
        })
    }

    static SESSIONS: OnceLock<Mutex<Vec<(String, Session)>>> = OnceLock::new();

    /// Run a single-input image model; returns the first output and its shape.
    fn run(name: &str, input: Vec<f32>, size: (usize, usize)) -> Option<(Vec<i64>, Vec<f32>)> {
        if !runtime_ok() {
            return None;
        }
        let path = model(name)?;
        let store = SESSIONS.get_or_init(|| Mutex::new(vec![]));
        let mut guard = store.lock().ok()?;
        if !guard.iter().any(|(n, _)| n == name) {
            let s = Session::builder().ok()?.commit_from_file(&path).map_err(|e| log::warn!("loading {}: {e}", path.display())).ok()?;
            guard.push((name.to_string(), s));
        }
        let session = &mut guard.iter_mut().find(|(n, _)| n == name)?.1;
        let tensor = Tensor::from_array(([1usize, 3, size.1, size.0], input)).ok()?;
        let outputs = session.run(ort::inputs![tensor]).map_err(|e| log::warn!("running {name}: {e}")).ok()?;
        let (shape, data) = outputs[0].try_extract_tensor::<f32>().ok()?;
        Some((shape.to_vec(), data.to_vec()))
    }

    /// Expected square input size of a model (from its input shape), default `def`.
    fn input_size(name: &str, def: usize) -> usize {
        if !runtime_ok() {
            return def;
        }
        let Some(path) = model(name) else { return def };
        let Ok(mut b) = Session::builder() else { return def };
        let Ok(s) = b.commit_from_file(path) else { return def };
        s.inputs()
            .first()
            .and_then(|i| i.dtype().tensor_shape().map(|sh| sh.to_vec()))
            .and_then(|d| d.last().copied())
            .filter(|v| *v > 0)
            .map(|v| v as usize)
            .unwrap_or(def)
    }

    /// NCHW float input from a display-referred sRGB image.
    fn prepare(img: &Image3, size: usize, mean: [f32; 3], std: [f32; 3]) -> Vec<f32> {
        let r = img.resize(size, size);
        let to = crate::color::prophoto_to(crate::color::ColorSpace::SRgb);
        let mut out = vec![0f32; 3 * size * size];
        for (i, p) in r.data.iter().enumerate() {
            let s = crate::color::mul_mv(&to, *p).map(|v| crate::color::srgb_encode(v.clamp(0.0, 1.0)));
            for c in 0..3 {
                out[c * size * size + i] = (s[c] - mean[c]) / std[c];
            }
        }
        out
    }

    fn to_plane(shape: &[i64], data: &[f32], w: usize, h: usize, invert: bool) -> Option<Plane> {
        let (ow, oh) = (*shape.last()? as usize, shape[shape.len() - 2] as usize);
        if ow * oh > data.len() {
            return None;
        }
        let d = &data[..ow * oh];
        let (mn, mx) = d.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        let span = (mx - mn).max(1e-6);
        let p = Plane {
            w: ow,
            h: oh,
            data: d
                .iter()
                .map(|v| {
                    let t = (v - mn) / span;
                    if invert { 1.0 - t } else { t }
                })
                .collect(),
        };
        Some(p.resize(w, h))
    }

    const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

    pub fn segment(name: &str, img: &Image3) -> Option<Plane> {
        model(name)?;
        let size = input_size(name, 320);
        // 1024px background-removal models expect (x - 0.5) / 1.0
        let (mean, std) = if size >= 1024 { ([0.5; 3], [1.0; 3]) } else { (IMAGENET_MEAN, IMAGENET_STD) };
        let (shape, data) = run(name, prepare(img, size, mean, std), (size, size))?;
        to_plane(&shape, &data, img.w, img.h, false)
    }

    pub fn depth(img: &Image3) -> Option<Plane> {
        model("depth.onnx")?;
        let size = input_size("depth.onnx", 518);
        let (shape, data) = run("depth.onnx", prepare(img, size, IMAGENET_MEAN, IMAGENET_STD), (size, size))?;
        // models output inverse depth (near = large); we want 0 = near
        to_plane(&shape, &data, img.w, img.h, true)
    }
}

#[cfg(feature = "ml")]
pub use imp::{depth, segment};

#[cfg(not(feature = "ml"))]
pub fn segment(_name: &str, _img: &Image3) -> Option<Plane> {
    let _ = model;
    None
}

#[cfg(not(feature = "ml"))]
pub fn depth(_img: &Image3) -> Option<Plane> {
    None
}
