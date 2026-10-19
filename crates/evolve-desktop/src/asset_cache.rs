//! Per-process, bounded decoded DDS cache for all native preview windows.
//! Entries are keyed by canonical location + size + file modification time.
//! Work happens on existing background workers; no decoded game data is saved.
use evolve_core::{DdsInfo, decode_dds_rgba};
use image::RgbaImage;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

const LIMIT_BYTES: u64 = 384 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceKey {
    canonical: PathBuf,
    size: u64,
    modified: SystemTime,
}
#[derive(Debug)]
struct CachedImage {
    key: SourceKey,
    info: DdsInfo,
    image: Arc<RgbaImage>,
    bytes: u64,
}
#[derive(Debug)]
pub struct PreviewCache {
    entries: VecDeque<CachedImage>,
    bytes: u64,
}
impl Default for PreviewCache {
    fn default() -> Self {
        Self { entries: VecDeque::new(), bytes: 0 }
    }
}
pub type SharedCache = Arc<Mutex<PreviewCache>>;

fn key(path: &Path) -> Result<SourceKey, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > 16 * 1024 * 1024 * 1024 {
        return Err("Source DDS is not a bounded regular file".into());
    }
    Ok(SourceKey {
        canonical: fs::canonicalize(path).map_err(|e| e.to_string())?,
        size: meta.len(),
        modified: meta.modified().map_err(|e| e.to_string())?,
    })
}
impl PreviewCache {
    fn lookup(&mut self, wanted: &SourceKey) -> Option<(DdsInfo, Arc<RgbaImage>)> {
        let index = self.entries.iter().position(|entry| entry.key == *wanted)?;
        let entry = self.entries.remove(index)?;
        let result = (entry.info.clone(), Arc::clone(&entry.image));
        self.entries.push_front(entry);
        Some(result)
    }
    fn insert(&mut self, key: SourceKey, info: DdsInfo, image: Arc<RgbaImage>) {
        let bytes = image.as_raw().len() as u64;
        if bytes > LIMIT_BYTES {
            return;
        }
        if let Some(index) = self.entries.iter().position(|entry| entry.key.canonical == key.canonical)
            && let Some(old) = self.entries.remove(index)
        {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
        while !self.entries.is_empty() &&
            (self.bytes.saturating_add(bytes) > LIMIT_BYTES || self.entries.len() >= MAX_ENTRIES)
        {
            if let Some(old) = self.entries.pop_back() {
                self.bytes = self.bytes.saturating_sub(old.bytes);
            }
        }
        self.bytes += bytes;
        self.entries.push_front(CachedImage { key, info, image, bytes });
    }
}
/// Reuse verified decoded pixels without blocking the UI. A changed source
/// invalidates a cached entry; only current decoded pixels remain in memory.
pub fn decoded_dds(
    cache: &SharedCache,
    path: &Path,
) -> Result<(DdsInfo, Arc<RgbaImage>), String> {
    let before = key(path)?;
    if let Ok(mut cache) = cache.lock() {
        if let Some(found) = cache.lookup(&before) {
            return Ok(found);
        }
    }
    let (info, pixels) = decode_dds_rgba(path).map_err(|e| e.to_string())?;
    if key(path)? != before {
        return Err("Source DDS changed while being decoded; try again.".into());
    }
    let pixels = Arc::new(pixels);
    if let Ok(mut cache) = cache.lock() {
        cache.insert(before, info.clone(), Arc::clone(&pixels));
    }
    Ok((info, pixels))
}
#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::DdsFormat;
    #[test]
    fn cache_reuses_arc_and_expires_when_source_changes() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("fake.dds");
        fs::write(&source, b"original").unwrap();
        let original_key = key(&source).unwrap();
        let info = DdsInfo {
            schema: 1, width: 2, height: 2, mipmaps: 1,
            format: DdsFormat::Rgba32, streaming_parts: 1, top_mip_bytes: 16,
        };
        let image = Arc::new(RgbaImage::new(2, 2));
        let mut cache = PreviewCache::default();
        cache.insert(original_key.clone(), info, image.clone());
        let (_, reused) = cache.lookup(&original_key).unwrap();
        assert!(Arc::ptr_eq(&image, &reused));
        fs::write(&source, b"modified-changed-size").unwrap();
        assert!(cache.lookup(&key(&source).unwrap()).is_none());
    }
}
