//! Open images: settings, history, snapshots, sidecar persistence; the
//! background loader and thumbnailer.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use darkslide_core::params::DevelopSettings;
use darkslide_core::{Engine, xmp};
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ColorLabel {
    #[default]
    None,
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ColorLabel {
    pub const ALL: [ColorLabel; 6] = [Self::None, Self::Red, Self::Yellow, Self::Green, Self::Blue, Self::Purple];
    pub fn name(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Red => Some("Red"),
            Self::Yellow => Some("Yellow"),
            Self::Green => Some("Green"),
            Self::Blue => Some("Blue"),
            Self::Purple => Some("Purple"),
        }
    }
    pub fn from_name(s: &str) -> Self {
        Self::ALL.iter().copied().find(|l| l.name() == Some(s)).unwrap_or(Self::None)
    }
    pub fn color(self) -> egui::Color32 {
        match self {
            Self::None => egui::Color32::TRANSPARENT,
            Self::Red => egui::Color32::from_rgb(220, 60, 60),
            Self::Yellow => egui::Color32::from_rgb(230, 200, 50),
            Self::Green => egui::Color32::from_rgb(70, 180, 80),
            Self::Blue => egui::Color32::from_rgb(70, 120, 230),
            Self::Purple => egui::Color32::from_rgb(160, 80, 200),
        }
    }
}

pub struct Doc {
    pub id: u64,
    pub path: PathBuf,
    pub engine: Option<Arc<Engine>>,
    pub load_error: Option<String>,
    pub settings: DevelopSettings,
    /// Settings when the image was opened (for "before" / reset to open).
    pub opened: DevelopSettings,
    pub committed: DevelopSettings,
    pub undo: Vec<DevelopSettings>,
    pub redo: Vec<DevelopSettings>,
    pub snapshots: Vec<(String, DevelopSettings)>,
    pub rating: i32,
    pub label: ColorLabel,
    pub rejected: bool,
    pub dirty: bool,
    pub last_change: Option<Instant>,
    pub thumb: Option<egui::TextureHandle>,
    pub thumb_requested: bool,
    pub selected: bool,
    /// Settings came from a loaded engine at least once (they stay after unloading).
    pub loaded: bool,
    pub raw: bool,
    /// Rating or label set before the photo was loaded, kept over the sidecar's.
    pub meta_pending: bool,
    /// Rating/label history, each entry tagged with the settings undo depth
    /// it happened at, so one Ctrl+Z order covers both.
    pub meta_undo: Vec<(usize, Meta)>,
    pub meta_redo: Vec<(usize, Meta)>,
}

#[derive(Clone, Copy, PartialEq)]
pub struct Meta {
    pub rating: i32,
    pub rejected: bool,
    pub label: ColorLabel,
}

impl Doc {
    pub fn new(id: u64, path: PathBuf) -> Self {
        let s = DevelopSettings::default();
        let path_for_raw = path.clone();
        Self {
            id,
            path,
            engine: None,
            load_error: None,
            settings: s.clone(),
            opened: s.clone(),
            committed: s,
            undo: vec![],
            redo: vec![],
            snapshots: vec![],
            rating: 0,
            label: ColorLabel::None,
            rejected: false,
            dirty: false,
            last_change: None,
            thumb: None,
            thumb_requested: false,
            selected: false,
            loaded: false,
            raw: !darkslide_core::decode::is_raster(&path_for_raw),
            meta_pending: false,
            meta_undo: vec![],
            meta_redo: vec![],
        }
    }

    pub fn name(&self) -> String {
        self.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }

    pub fn is_raw(&self) -> bool {
        self.engine.as_ref().map(|e| e.is_raw()).unwrap_or(true)
    }

    /// Called once the engine is loaded: pick up sidecar settings.
    pub fn attach(&mut self, engine: Engine) {
        let is_raw = engine.is_raw();
        let mut s = DevelopSettings::defaults_for(is_raw);
        if let Some(def) = load_camera_default(&engine) {
            s = def;
        }
        let pending = self.meta();
        let side = xmp::read_sidecar(&self.path, is_raw);
        // a sidecar holding only a rating or label doesn't replace camera or embedded settings
        let meta_only = side.as_ref().is_some_and(|(_, p)| p.ds_json.is_none() && p.crs.is_empty() && p.masks.is_empty());
        if let Some((side, parsed)) = side.filter(|_| !meta_only) {
            s = side;
            self.rating = parsed.rating.unwrap_or(0);
            self.rejected = self.rating < 0;
            self.label = parsed.label.as_deref().map(ColorLabel::from_name).unwrap_or_default();
        } else if let Some(parsed) = engine.source.embedded_xmp.as_deref().map(xmp::parse).filter(|p| p.ds_json.is_some() || !p.crs.is_empty()) {
            // settings embedded in the file (e.g. a DNG saved with settings)
            s = xmp::settings_from_parsed(&parsed, is_raw);
            self.rating = parsed.rating.unwrap_or(0);
        } else if let Some(r) = engine.source.meta.rating {
            self.rating = r as i32;
        }
        if meta_only && let Some((_, parsed)) = xmp::read_sidecar(&self.path, is_raw) {
            self.rating = parsed.rating.unwrap_or(0);
            self.rejected = self.rating < 0;
            self.label = parsed.label.as_deref().map(ColorLabel::from_name).unwrap_or_default();
        }
        if std::mem::take(&mut self.meta_pending) {
            self.set_meta_raw(pending);
            self.dirty = true;
        }
        self.loaded = true;
        self.raw = is_raw;
        self.snapshots = load_snapshots(&self.path);
        self.settings = s.clone();
        self.opened = s.clone();
        self.committed = s;
        self.engine = Some(Arc::new(engine));
    }

    /// Record history when the settings changed and no drag is in progress.
    pub fn track_history(&mut self, pointer_down: bool) {
        if self.settings != self.committed && !pointer_down {
            self.undo.push(std::mem::replace(&mut self.committed, self.settings.clone()));
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.meta_redo.clear();
            self.dirty = true;
            self.last_change = Some(Instant::now());
        }
    }

    /// A finished AI detection fills its mask everywhere in history, so the
    /// fill isn't an undo step of its own and undo never lands on an
    /// undetected mask (which would detect again).
    pub fn fill_ai_raster(&mut self, mi: usize, kind: darkslide_core::params::AiMaskKind, raster: &str) {
        let fill = |s: &mut DevelopSettings| {
            if let Some(m) = s.masks.get_mut(mi) {
                for c in m.components.iter_mut() {
                    if let darkslide_core::params::MaskShape::Ai { kind: k, raster: r @ None, .. } = &mut c.shape
                        && *k == kind
                    {
                        *r = Some(raster.to_string());
                        break;
                    }
                }
            }
        };
        fill(&mut self.settings);
        fill(&mut self.committed);
        self.undo.iter_mut().chain(self.redo.iter_mut()).for_each(fill);
    }

    pub fn meta(&self) -> Meta {
        Meta { rating: self.rating, rejected: self.rejected, label: self.label }
    }

    fn set_meta_raw(&mut self, m: Meta) {
        (self.rating, self.rejected, self.label) = (m.rating, m.rejected, m.label);
    }

    /// Change rating/label as an undoable step.
    pub fn set_meta(&mut self, m: Meta) {
        if m == self.meta() {
            return;
        }
        self.meta_undo.push((self.undo.len(), self.meta()));
        self.meta_redo.clear();
        self.redo.clear();
        self.set_meta_raw(m);
        self.meta_pending |= !self.loaded;
        self.dirty = true;
        self.last_change = Some(Instant::now());
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || !self.meta_undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty() || !self.meta_redo.is_empty()
    }

    pub fn undo(&mut self) {
        if self.meta_undo.last().is_some_and(|(n, _)| *n == self.undo.len()) {
            let (n, m) = self.meta_undo.pop().unwrap();
            self.meta_redo.push((n, self.meta()));
            self.set_meta_raw(m);
            self.dirty = true;
            self.last_change = Some(Instant::now());
            return;
        }
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.settings, prev));
            self.committed = self.settings.clone();
            self.dirty = true;
            self.last_change = Some(Instant::now());
        }
    }

    pub fn redo(&mut self) {
        if self.meta_redo.last().is_some_and(|(n, _)| *n == self.undo.len()) {
            let (n, m) = self.meta_redo.pop().unwrap();
            self.meta_undo.push((n, self.meta()));
            self.set_meta_raw(m);
            self.dirty = true;
            self.last_change = Some(Instant::now());
            return;
        }
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.settings, next));
            self.committed = self.settings.clone();
            self.dirty = true;
            self.last_change = Some(Instant::now());
        }
    }

    pub fn save_sidecar(&mut self) {
        let label = self.label.name().map(|s| s.to_string());
        let rating = if self.rejected { -1 } else { self.rating };
        let r = if self.loaded {
            save_snapshots(&self.path, &self.snapshots);
            xmp::write_sidecar_full(&self.path, &self.settings, self.raw, Some(rating), label)
        } else {
            // never loaded: only the rating and label are known
            match xmp::read_sidecar(&self.path, self.raw) {
                None => xmp::write_meta_sidecar(&self.path, rating, label),
                Some((s, parsed)) if parsed.ds_json.is_some() => xmp::write_sidecar_full(&self.path, &s, self.raw, Some(rating), label),
                // a foreign sidecar we'd rewrite lossily: the rating waits for the photo to load
                Some(_) => Ok(()),
            }
        };
        if let Err(e) = r {
            log::warn!("writing sidecar for {}: {e}", self.path.display());
        }
        self.dirty = false;
    }
}

// Snapshots are kept next to our own data so they don't bloat sidecars.
fn snapshot_file(path: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    darkslide_core::paths::data_dir().join("snapshots").join(format!("{:016x}.json", h.finish()))
}

pub fn load_snapshots(path: &Path) -> Vec<(String, DevelopSettings)> {
    std::fs::read_to_string(snapshot_file(path)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_snapshots(path: &Path, snaps: &[(String, DevelopSettings)]) {
    let f = snapshot_file(path);
    if snaps.is_empty() {
        let _ = std::fs::remove_file(&f);
        return;
    }
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(t) = serde_json::to_string(snaps) {
        let _ = std::fs::write(f, t);
    }
}

/// Camera-specific defaults ("Save new Camera Raw defaults").
fn defaults_file(make: &str, model: &str) -> PathBuf {
    darkslide_core::paths::config_dir().join("defaults").join(format!("{}.json", xmp::sanitize(&format!("{make} {model}"))))
}

pub fn load_camera_default(engine: &Engine) -> Option<DevelopSettings> {
    let m = &engine.source.meta;
    let p = defaults_file(&m.make, &m.model);
    let p = if p.exists() { p } else { darkslide_core::paths::config_dir().join("defaults").join("_global.json") };
    let t = std::fs::read_to_string(p).ok()?;
    let mut s: DevelopSettings = serde_json::from_str(&t).ok()?;
    if !engine.is_raw() {
        // raw-only defaults don't carry over to rendered files
        s.temperature = 0.0;
        s.tint = 0.0;
        s.sharpen.amount = 0.0;
        s.nr.color = 0.0;
    }
    Some(s)
}

pub fn save_camera_default(engine: &Engine, s: &DevelopSettings, global: bool) -> anyhow::Result<()> {
    let m = &engine.source.meta;
    let p = if global { darkslide_core::paths::config_dir().join("defaults").join("_global.json") } else { defaults_file(&m.make, &m.model) };
    std::fs::create_dir_all(p.parent().unwrap())?;
    let mut s = s.clone();
    // defaults never carry image-specific edits
    s.crop = Default::default();
    s.masks.clear();
    s.retouch.clear();
    s.red_eye.clear();
    s.wb_mode = darkslide_core::params::WbMode::AsShot;
    std::fs::write(p, serde_json::to_string_pretty(&s)?)?;
    Ok(())
}

pub fn reset_camera_default(engine: &Engine) {
    let m = &engine.source.meta;
    let _ = std::fs::remove_file(defaults_file(&m.make, &m.model));
}

// ---------------------------------------------------------------------------
// Loader / thumbnailer threads

pub enum Loaded {
    Engine(u64, Result<Engine, String>),
    Thumb(u64, Option<image::RgbImage>),
}

pub struct Loader {
    tx: Sender<(u64, PathBuf, bool)>,
    pub rx: Receiver<Loaded>,
}

impl Loader {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, jrx) = channel::<(u64, PathBuf, bool)>();
        let (otx, rx) = channel::<Loaded>();
        // full loads are serialised on one thread; thumbnails on another
        let (ttx, trx) = channel::<(u64, PathBuf)>();
        let ctx2 = ctx.clone();
        let otx2 = otx.clone();
        std::thread::Builder::new()
            .name("loader".into())
            .spawn(move || {
                while let Ok((id, path, thumb)) = jrx.recv() {
                    if thumb {
                        let _ = ttx.send((id, path));
                        continue;
                    }
                    let r = Engine::open(&path).map_err(|e| e.to_string());
                    if let Ok(e) = &r {
                        // warm the draft cache so the first render is fast
                        let _ = e.base(darkslide_core::demosaic::Quality::Draft);
                    }
                    let _ = otx.send(Loaded::Engine(id, r));
                    ctx.request_repaint();
                }
            })
            .unwrap();
        std::thread::Builder::new()
            .name("thumbs".into())
            .spawn(move || {
                while let Ok((id, path)) = trx.recv() {
                    let t = darkslide_core::thumbcache::load(&path, 256).ok();
                    let _ = otx2.send(Loaded::Thumb(id, t));
                    ctx2.request_repaint();
                }
            })
            .unwrap();
        Self { tx, rx }
    }

    pub fn load(&self, id: u64, path: PathBuf) {
        let _ = self.tx.send((id, path, false));
    }

    pub fn thumb(&self, id: u64, path: PathBuf) {
        let _ = self.tx.send((id, path, true));
    }
}

/// Supported image files in a folder, sorted by name.
pub fn scan_folder(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && darkslide_core::decode::is_supported(p) && p.extension().map(|e| e != "xmp").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}
