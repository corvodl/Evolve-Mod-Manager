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
    let (info, pixels) = decode_preview_image(&staged, layout)
        .map_err(|e| format!("Cannot reconstruct {virtual_path} from imported DDS mips: {e}"))?;
    Ok((info, Arc::new(pixels)))
}

/// Display every face of a verified six-face cube DDS as an ordinary 3x2
/// image, so even envprobe textures are useful in the 2D asset viewport.
/// This is a preview, not a reinterpretation of the game's original bytes.
/// Face-major complete DDS and mip-major split DDS sets are both supported.
fn decode_preview_image(path: &Path, layout: DdsLayout) -> Result<(DdsInfo, RgbaImage), String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 128 || &bytes[..4] != b"DDS " {
        return Err("DDS header missing; import the matching low-mip archive".into());
    }
    let value = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    if value(112) != 0xFE00 {
        return decode_dds_rgba_with_layout(path, layout).map_err(|e| e.to_string());
    }
    // Exactly six declared cube faces; reject partial, volume and DX10 cube
    // arrays rather than assigning a made-up face to a source bitmap.
    let width = value(16);
    let height = value(12);
    let mips = if value(8) & 0x20000 != 0 { value(28) } else { 1 };
    if width == 0 || height == 0 || width > 8192 || height > 8192
        || mips == 0 || mips > 16
        || u64::from(width) * u64::from(height) * 6 > 40_000_000
    {
        return Err("Unsupported cubemap dimensions/mip count".into());
    }
    let fourcc = &bytes[84..88];
    let block = if value(80) & 4 != 0 {
        match fourcc {
            b"DXT1" | b"CTX1" | b"ATI1" | b"BC4U" => Some(8u64),
            b"DXT3" | b"DXT5" | b"ATI2" | b"BC5U" => Some(16u64),
            _ => return Err("Unknown DDS cubemap compression; no guessed channels".into()),
        }
    } else {
        if value(88) != 32 || value(80) & 0x40 == 0 {
            return Err("Unknown DDS cubemap pixel layout".into());
        }
        None
    };
    let surface_size = |w: u32, h: u32| -> Result<usize, String> {
        let len = if let Some(bytes_per_block) = block {
            u64::from(w.div_ceil(4)) * u64::from(h.div_ceil(4)) * bytes_per_block
        } else {
            u64::from(w) * u64::from(h) * 4
        };
        usize::try_from(len).map_err(|e| e.to_string())
    };
    let top_size = surface_size(width, height)?;
    let full_face_size = (0..mips).try_fold(0usize, |total, level| {
        let w = (width >> level).max(1);
        let h = (height >> level).max(1);
        total.checked_add(surface_size(w, h).ok()?)
    }).ok_or("DDS cubemap size overflow")?;
    let source_bytes = bytes.len() - 128;
    let is_staged = path.file_name().is_some_and(|n| n.to_string_lossy() == "preview.dds.0");
    let mut top_mip = None;
    if is_staged {
        let parent = path.parent().ok_or("Missing staging folder")?;
        let numbered = (1..20usize)
            .filter_map(|i| {
                let candidate = parent.join(format!("preview.dds.{i}"));
                candidate.is_file().then_some((i, candidate))
            })
            .collect::<Vec<_>>();
        if !numbered.is_empty() {
            let mip_sizes = (0..mips)
                .map(|level| surface_size((width >> level).max(1), (height >> level).max(1)))
                .collect::<Result<Vec<_>, _>>()?;
            let tail_count = (1..mips as usize)
                .find(|n| mip_sizes[mip_sizes.len() - n..].iter().sum::<usize>() * 6 == source_bytes)
                .ok_or("Cubemap low-mip DDS.0 has an invalid tail size")?;
            let required = mips as usize - tail_count;
            if numbered.len() != required
                || numbered.iter().enumerate().any(|(i, (n, _))| *n != i + 1)
            {
                return Err("Cubemap high/low mip set is incomplete or not contiguous".into());
            }
            for (n, file) in &numbered {
                let size = fs::metadata(file).map_err(|e| e.to_string())?.len();
                if size != mip_sizes[mips as usize - tail_count - n] as u64 * 6 {
                    return Err(format!("Cubemap fragment .{n} has an incorrect mip length"));
                }
            }
            let high = fs::read(&numbered.last().ok_or("Missing cubemap top mip")?.1)
                .map_err(|e| e.to_string())?;
            if high.len() != top_size * 6 {
                return Err("Cubemap high mip does not contain exactly six faces".into());
            }
            top_mip = Some(high);
        }
    }
    if top_mip.is_none() && source_bytes != full_face_size * 6 {
        return Err("Incomplete six-face cubemap mip set".into());
    }
    let face_stride = if top_mip.is_some() { top_size } else { full_face_size };
    let source = top_mip.as_deref().unwrap_or(&bytes[128..]);
    let mut atlas = RgbaImage::new(width * 3, height * 2);
    let folder = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut info = None;
    for face in 0..6usize {
        let start = face * face_stride;
        let end = start.checked_add(top_size).ok_or("Cubemap face size overflow")?;
        let pixels = source.get(start..end)
            .ok_or("Cubemap face extends past original texture")?;
        let mut header = bytes[..128].to_vec();
        let flags = value(8) & !0x20000;
        header[8..12].copy_from_slice(&flags.to_le_bytes());
        header[28..32].copy_from_slice(&1u32.to_le_bytes());
        header[112..116].copy_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(pixels);
        let file = folder.path().join(format!("cube-face-{face}.dds"));
        fs::write(&file, &header).map_err(|e| e.to_string())?;
        let (details, image) = decode_dds_rgba_with_layout(&file, layout)
            .map_err(|e| format!("Cubemap face {face}: {e}"))?;
        image::imageops::replace(
            &mut atlas, &image,
            i64::from((face % 3) as u32 * width),
            i64::from((face / 3) as u32 * height),
        );
        info = Some(details);
    }
    let mut info = info.ok_or("Empty cubemap")?;
    info.width = atlas.width();
    info.height = atlas.height();
    info.top_mip_bytes = top_size * 6;
    Ok((info, atlas))
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
    let (info, pixels) = decode_preview_image(path, layout)?;
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

    fn rgba_header(width: u32, height: u32, mips: u32, cube: bool) -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&(0x1007u32 | if mips > 1 {0x20000} else {0}).to_le_bytes());
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[28..32].copy_from_slice(&mips.to_le_bytes());
        bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
        bytes[80..84].copy_from_slice(&0x41u32.to_le_bytes());
        bytes[88..92].copy_from_slice(&32u32.to_le_bytes());
        for (at, mask) in [(92, 0x00ff0000u32), (96, 0x0000ff00), (100, 0x000000ff), (104, 0xff000000)] {
            bytes[at..at + 4].copy_from_slice(&mask.to_le_bytes());
        }
        bytes[108..112].copy_from_slice(&0x1000u32.to_le_bytes());
        if cube { bytes[112..116].copy_from_slice(&0xfe00u32.to_le_bytes()); }
        bytes
    }

    #[test]
    fn six_face_cubemap_is_displayed_as_color_preserving_image_atlas() {
        let temp = tempfile::tempdir().unwrap();
        let mut texture = rgba_header(4, 4, 1, true);
        for face in 0u8..6 {
            for _ in 0..16 { texture.extend_from_slice(&[0, 0, 40 + face * 25, 255]); }
        }
        let file = temp.path().join("environment.dds.0");
        fs::write(&file, texture).unwrap();
        let (info, atlas) = decode_preview_image(&file, DdsLayout::Linear).unwrap();
        assert_eq!((info.width, info.height), (12, 8));
        assert_eq!(atlas.get_pixel(0, 0).0, [40, 0, 0, 255]);
        assert_eq!(atlas.get_pixel(8, 4).0, [165, 0, 0, 255]);
    }

    #[test]
    fn high_mip_in_another_imported_pak_uses_exact_build_and_low_header() {
        let library = tempfile::tempdir().unwrap();
        let ids = ["11111111111111111111", "22222222222222222222"];
        let names = ["dds_low.pak", "dds_high.pak"];
        let virtual_path = "objects/characters/monsters/scorpid_2/scorpid_body.dds";
        let mut hdr = rgba_header(8, 8, 2, false);
        for _ in 0..16 { hdr.extend_from_slice(&[0, 255, 0, 255]); }
        let high = [255u8, 0, 0, 255].repeat(64);
        let mut low_path = None;
        for i in 0..2 {
            let base = library.path().join(format!("{}__{}", names[i], ids[i]));
            let member = format!("{virtual_path}.{}", i);
            let dest = base.join("files").join(&member);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            fs::write(&dest, if i == 0 {&hdr} else {&high}).unwrap();
            fs::write(
                base.join("asset-library.json"),
                serde_json::to_vec(&serde_json::json!({
                    "version":1, "name":names[i], "source":"/original",
                    "entries":[{"path":member,"bytes":if i == 0 {hdr.len()} else {high.len()}}]
                })).unwrap(),
            ).unwrap();
            if i == 0 {low_path = Some(dest);}
        }
        fs::write(library.path().join("build-collections.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":1,"builds":{"legacy":{"label":"Legacy 2012",
                "members":{ids[0]:{},ids[1]:{}}}}
            })).unwrap(),
        ).unwrap();
        let source = low_path.unwrap();
        let inventory = evolve_core::scan_directory(source.parent().unwrap().parent().unwrap().parent().unwrap().parent().unwrap()).unwrap();
        let cache = Arc::new(Mutex::new(PreviewCache::default()));
        let (info, image) = decoded_dds_indexed(
            &cache, &source, &format!("{virtual_path}.0"),
            &inventory, Some(library.path()), Some("Legacy 2012")
        ).unwrap();
        assert_eq!((info.width, info.height), (8, 8));
        assert_eq!(image.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(info.streaming_parts, 2);
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
