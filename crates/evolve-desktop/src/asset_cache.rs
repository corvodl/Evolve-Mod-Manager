//! Per-process, bounded decoded DDS cache for all native preview windows.
//! Entries are keyed by canonical location + size + file modification time.
//! Work happens on existing background workers; no decoded game data is saved.
use evolve_core::{
    AssetResolution, DdsInfo, DdsLayout, Inventory, LibraryReferenceStatus,
    decode_dds_rgba_with_layout, resolve_imported_reference, resolve_virtual,
};
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
    layout: DdsLayout,
}
#[derive(Debug)]
struct CachedImage {
    key: SourceKey,
    info: DdsInfo,
    image: Arc<RgbaImage>,
    bytes: u64,
}
#[derive(Debug, Default)]
pub struct PreviewCache {
    entries: VecDeque<CachedImage>,
    bytes: u64,
}
pub type SharedCache = Arc<Mutex<PreviewCache>>;

fn key(path: &Path, layout: DdsLayout) -> Result<SourceKey, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > 16 * 1024 * 1024 * 1024 {
        return Err("Source DDS is not a bounded regular file".into());
    }
    Ok(SourceKey {
        canonical: fs::canonicalize(path).map_err(|e| e.to_string())?,
        size: meta.len(),
        modified: meta.modified().map_err(|e| e.to_string())?,
        layout,
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
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.key.canonical == key.canonical && entry.key.layout == key.layout
        }) && let Some(old) = self.entries.remove(index)
        {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
        while !self.entries.is_empty()
            && (self.bytes.saturating_add(bytes) > LIMIT_BYTES || self.entries.len() >= MAX_ENTRIES)
        {
            if let Some(old) = self.entries.pop_back() {
                self.bytes = self.bytes.saturating_sub(old.bytes);
            }
        }
        self.bytes += bytes;
        self.entries.push_front(CachedImage {
            key,
            info,
            image,
            bytes,
        });
    }
}
/// Reuse verified decoded pixels without blocking the UI. A changed source
/// invalidates a cached entry; only current decoded pixels remain in memory.
/// The uploaded historical Xbox 360 high-mip archive uses the FOUR bytes
/// FYRC (reverse of CRYF) in the DDS reserved header tail. Its compressed
/// surfaces use the original console layout flag in reserved1[0]. A PC
/// Stage 2 DDS must remain linear, even when its material looks unusual.
fn detect_source_layout(path: &Path) -> Result<DdsLayout, String> {
    use std::io::Read;
    let mut header = [0u8; 128];
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    if file.read_exact(&mut header).is_ok()
        && &header[..4] == b"DDS "
        && &header[124..128] == b"FYRC"
        && u32::from_le_bytes(header[80..84].try_into().unwrap()) & 4 != 0
        && matches!(
            u32::from_le_bytes(header[36..40].try_into().unwrap()),
            0x1800 | 0x1040 | 0x1840 | 0x1000 | 0x1440 | 0x1010
        )
        && matches!(
            u32::from_le_bytes(header[108..112].try_into().unwrap()),
            0x1000 | 0x401008
        )
    {
        return Ok(DdsLayout::Xbox360Tiled);
    }
    Ok(DdsLayout::Linear)
}

/// Decode an authored DDS from a local extraction plus verified cross-PAK
/// mip fragments of the SAME registered build. A numbered high fragment
/// contains no format or dimension metadata: its matching .dds.0 header
/// must exist, and every required intermediate mip must be present.
pub fn decoded_dds_indexed(
    cache: &SharedCache,
    source: &Path,
    virtual_path: &str,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<(DdsInfo, Arc<RgbaImage>), String> {
    let lower = virtual_path.replace('\\', "/").to_ascii_lowercase();
    let base = if let Some((name, suffix)) = lower.rsplit_once(".dds.") {
        if suffix.parse::<usize>().is_err() {
            return Err("Invalid numbered DDS fragment".into());
        }
        format!("{name}.dds")
    } else if lower.ends_with(".dds") {
        lower.clone()
    } else {
        return Err("Not a DDS texture path".into());
    };
    let source_index = lower
        .rsplit_once(".dds.")
        .and_then(|(_, n)| n.parse::<usize>().ok());
    if source_index.is_some_and(|n| n > 19) {
        return Err("DDS part index exceeds 20-mip safety limit".into());
    }
    if source_index.is_none() {
        return decoded_dds(cache, source);
    }
    let resolve = |part: usize| -> Result<Option<PathBuf>, String> {
        if source_index == Some(part) {
            return Ok(Some(source.to_path_buf()));
        }
        let virtual_part = format!("{base}.{part}");
        let local = resolve_virtual(inventory, &virtual_part);
        match local {
            AssetResolution::Found(relative) => {
                let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
                let target = fs::canonicalize(root.join(&relative)).map_err(|e| e.to_string())?;
                if !target.starts_with(&root) {
                    return Err("DDS fragment escaped the indexed root".into());
                }
                return Ok(Some(target));
            }
            AssetResolution::Ambiguous(_) => {
                return Err(format!("Ambiguous local DDS fragment {virtual_part}"));
            }
            AssetResolution::Missing => {}
        }
        let Some(imported) = imported else {
            return Ok(None);
        };
        match resolve_imported_reference(imported, &virtual_part, build)
            .map_err(|e| e.to_string())?
        {
            LibraryReferenceStatus::Found(reference) => Ok(Some(reference.absolute_path.into())),
            LibraryReferenceStatus::Missing => Ok(None),
            LibraryReferenceStatus::Ambiguous(_) => {
                Err(format!("Ambiguous imported DDS fragment {virtual_part}; select a build"))
            }
        }
    };
    let header = resolve(0)?.ok_or_else(|| format!(
        "Cannot preview {virtual_path}: missing original {base}.0 header. Import the matching dds_low.pak alongside dds_high.pak in the same Asset Library build."
    ))?;
    // Stage verified raw bytes in a private temporary folder; never write to
    // the extracted files, guessed file paths or installed game. All source
    // fragments have been checked against their indexed/manifest provenance.
    let staging = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut count = 0usize;
    let mut staged_bytes = 0u64;
    for i in 0..20 {
        let Some(original) = (if i == 0 { Some(header.clone()) } else { resolve(i)? }) else {
            continue;
        };
        let meta = fs::symlink_metadata(&original).map_err(|e| e.to_string())?;
        if !meta.file_type().is_file() {
            return Err(format!("DDS mip part {i} is not a regular file"));
        }
        staged_bytes = staged_bytes
            .checked_add(meta.len())
            .ok_or("DDS fragment aggregate size overflow")?;
        if staged_bytes > 256 * 1024 * 1024 {
            return Err("DDS fragment aggregate exceeds 256 MiB".into());
        }
        let dest = staging.path().join(format!("preview.dds.{i}"));
        fs::copy(&original, &dest).map_err(|e| e.to_string())?;
        count += 1;
    }
    if source_index.is_some_and(|index| count <= 1 && index != 0) {
        return Err(format!("No contiguous DDS mip chain for {virtual_path}"));
    }
    let staged = staging.path().join("preview.dds.0");
    let layout = detect_source_layout(&staged)?;
    let (info, pixels) = decode_dds_rgba_with_layout(&staged, layout)
        .map_err(|e| format!("Cannot reconstruct {virtual_path} from imported DDS mips: {e}"))?;
    Ok((info, Arc::new(pixels)))
}

pub fn decoded_dds(cache: &SharedCache, path: &Path) -> Result<(DdsInfo, Arc<RgbaImage>), String> {
    let layout = detect_source_layout(path)?;
    decoded_dds_with_layout(cache, path, layout)
}

pub fn decoded_dds_with_layout(
    cache: &SharedCache,
    path: &Path,
    layout: DdsLayout,
) -> Result<(DdsInfo, Arc<RgbaImage>), String> {
    let before = key(path, layout)?;
    if let Ok(mut cache) = cache.lock()
        && let Some(found) = cache.lookup(&before)
    {
        return Ok(found);
    }
    let (info, pixels) = decode_dds_rgba_with_layout(path, layout).map_err(|e| e.to_string())?;
    if key(path, layout)? != before {
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
    fn xbox_header_detection_does_not_treat_pc_dds_as_console() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("texture.dds.0");
        let mut hdr = [0u8; 128];
        hdr[..4].copy_from_slice(b"DDS ");
        hdr[80..84].copy_from_slice(&4u32.to_le_bytes());
        hdr[36..40].copy_from_slice(&0x1800u32.to_le_bytes());
        hdr[108..112].copy_from_slice(&0x1000u32.to_le_bytes());
        hdr[124..128].copy_from_slice(b"FYRC");
        fs::write(&file, hdr).unwrap();
        assert_eq!(detect_source_layout(&file).unwrap(), DdsLayout::Xbox360Tiled);
        let mut hdr = hdr;
        hdr[124..128].copy_from_slice(b"CRYF");
        fs::write(&file, hdr).unwrap();
        assert_eq!(detect_source_layout(&file).unwrap(), DdsLayout::Linear);
    }

    #[test]
    fn cache_reuses_arc_and_expires_when_source_changes() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("fake.dds");
        fs::write(&source, b"original").unwrap();
        let original_key = key(&source, DdsLayout::Linear).unwrap();
        let info = DdsInfo {
            schema: 1,
            width: 2,
            height: 2,
            mipmaps: 1,
            format: DdsFormat::Rgba32,
            streaming_parts: 1,
            top_mip_bytes: 16,
        };
        let image = Arc::new(RgbaImage::new(2, 2));
        let mut cache = PreviewCache::default();
        cache.insert(original_key.clone(), info, image.clone());
        let (_, reused) = cache.lookup(&original_key).unwrap();
        assert!(Arc::ptr_eq(&image, &reused));
        fs::write(&source, b"modified-changed-size").unwrap();
        assert!(
            cache
                .lookup(&key(&source, DdsLayout::Linear).unwrap())
                .is_none()
        );
    }
}
