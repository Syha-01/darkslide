//! On-disk filmstrip thumbnails in `~/.cache/darkslide/thumbs`, keyed by the
//! file's path, size and mtime so a changed file misses. The app overwrites an
//! entry with the edited render, so thumbnails show edits on the next visit.

use std::path::{Path, PathBuf};

const MAX_FILES: usize = 4000;

fn dir() -> PathBuf {
    crate::paths::cache_dir().join("thumbs")
}

// FNV-1a; std's hasher isn't stable across Rust releases
fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn entry(path: &Path) -> Option<PathBuf> {
    let md = std::fs::metadata(path).ok()?;
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mtime = md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let mut h = fnv(abs.as_os_str().as_encoded_bytes(), 0xcbf2_9ce4_8422_2325);
    h = fnv(&md.len().to_le_bytes(), h);
    h = fnv(&mtime.to_le_bytes(), h);
    Some(dir().join(format!("{h:016x}.jpg")))
}

pub fn get(path: &Path) -> Option<image::RgbImage> {
    Some(image::open(entry(path)?).ok()?.to_rgb8())
}

pub fn put(path: &Path, img: &image::RgbImage) {
    let Some(p) = entry(path) else { return };
    if std::fs::create_dir_all(dir()).is_err() {
        return;
    }
    // write then rename so a reader never sees half a file
    let tmp = p.with_extension("tmp");
    let mut buf = vec![];
    if image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85).encode(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgb8).is_err()
    {
        return;
    }
    if std::fs::write(&tmp, buf).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
    prune();
}

/// Keep the cache bounded by dropping the least recently written entries.
fn prune() {
    let Ok(rd) = std::fs::read_dir(dir()) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if files.len() <= MAX_FILES {
        return;
    }
    files.sort();
    for (_, p) in &files[..files.len() - MAX_FILES * 9 / 10] {
        let _ = std::fs::remove_file(p);
    }
}

/// Cached thumbnail, or the embedded preview (which is then cached).
pub fn load(path: &Path, max: u32) -> anyhow::Result<image::RgbImage> {
    if let Some(t) = get(path) {
        return Ok(t);
    }
    let t = crate::decode::load_thumbnail(path, max)?;
    put(path, &t);
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_changes_with_file() {
        let d = std::env::temp_dir().join(format!("ds_tc_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("a.jpg");
        std::fs::write(&f, b"one").unwrap();
        let k1 = entry(&f).unwrap();
        assert_eq!(k1, entry(&f).unwrap());
        std::fs::write(&f, b"three").unwrap();
        assert_ne!(k1, entry(&f).unwrap());
        std::fs::remove_dir_all(&d).ok();
    }
}
