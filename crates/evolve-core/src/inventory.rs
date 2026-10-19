use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Bound traversal to protect the UI from malformed or enormous directories.
pub const MAX_ENTRIES: usize = 250_000;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Pak,
    CharacterDefinition,
    Skeleton,
    SkinnedMesh,
    Mesh,
    Material,
    Dds,
    AnimationCaf,
    AnimationDba,
    AnimationGraph,
    Level,
    UserInterface,
    Other,
}

impl AssetKind {
    pub fn display(self) -> &'static str {
        match self {
            Self::Pak => "PAK archive",
            Self::CharacterDefinition => "CDF assembly",
            Self::Skeleton => "Skeleton",
            Self::SkinnedMesh => "Skinned mesh",
            Self::Mesh => "Model mesh",
            Self::Material => "Material",
            Self::Dds => "DDS texture",
            Self::AnimationCaf => "CAF animation",
            Self::AnimationDba => "DBA animation",
            Self::AnimationGraph => "Animation graph",
            Self::Level => "Level",
            Self::UserInterface => "UI asset",
            Self::Other => "Other",
        }
    }
}

pub fn classify_path(path: &Path) -> AssetKind {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".dds") || name.ends_with(".dds.0") {
        return AssetKind::Dds;
    }
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        "pak" => AssetKind::Pak,
        "cdf" => AssetKind::CharacterDefinition,
        "chr" => AssetKind::Skeleton,
        "skinm" | "skin" => AssetKind::SkinnedMesh,
        "cgf" | "chrm" => AssetKind::Mesh,
        "mtl" => AssetKind::Material,
        "caf" => AssetKind::AnimationCaf,
        "dba" => AssetKind::AnimationDba,
        "bspace" | "comb" | "chrparams" => AssetKind::AnimationGraph,
        "cry" | "lvl" => AssetKind::Level,
        "gfx" | "swf" | "usm" => AssetKind::UserInterface,
        _ => AssetKind::Other,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssetEntry {
    /// Relative to the chosen input root; never an absolute OS path.
    pub relative_path: String,
    pub byte_size: u64,
    pub kind: AssetKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inventory {
    pub schema: u32,
    pub root: String,
    pub entries: Vec<AssetEntry>,
    pub total_bytes: u64,
    pub skipped_symlinks: usize,
}

impl Inventory {
    pub fn count_kind(&self, kind: AssetKind) -> usize {
        self.entries.iter().filter(|entry| entry.kind == kind).count()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    #[error("Choose an existing regular directory")]
    InvalidRoot,
    #[error("Traversal exceeds the safe file limit of {MAX_ENTRIES}")]
    TooManyFiles,
    #[error("Nested directories exceed the supported depth")]
    TooDeep,
    #[error("Invalid or unreadable local file: {0}")]
    Io(#[from] io::Error),
}

/// Scan an explicitly chosen directory. Never follow junctions or symbolic links.
/// No game files are read, altered, converted, or copied by this operation.
pub fn scan_directory(root: &Path) -> Result<Inventory, InventoryError> {
    if !root.is_dir() || root.symlink_metadata()?.file_type().is_symlink() {
        return Err(InventoryError::InvalidRoot);
    }
    let canonical = fs::canonicalize(root)?;
    let mut pending: Vec<(PathBuf, usize)> = vec![(canonical.clone(), 0)];
    let mut entries = Vec::new();
    let mut skipped_symlinks = 0;
    let mut total_bytes = 0u64;
    while let Some((folder, depth)) = pending.pop() {
        if depth > MAX_DEPTH {
            return Err(InventoryError::TooDeep);
        }
        let mut children = fs::read_dir(&folder)?
            .collect::<Result<Vec<_>, io::Error>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let path = child.path();
            let kind = child.file_type()?;
            if kind.is_symlink() {
                skipped_symlinks += 1;
                continue;
            }
            if kind.is_dir() {
                if child.file_name() != ".git" {
                    pending.push((path, depth + 1));
                }
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            if entries.len() == MAX_ENTRIES {
                return Err(InventoryError::TooManyFiles);
            }
            let relative = path.strip_prefix(&canonical).map_err(|_| InventoryError::InvalidRoot)?;
            let byte_size = child.metadata()?.len();
            total_bytes = total_bytes.saturating_add(byte_size);
            entries.push(AssetEntry {
                relative_path: relative.to_string_lossy().replace('\\', "/"),
                byte_size,
                kind: classify_path(relative),
            });
        }
    }
    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(Inventory {
        schema: 1,
        root: canonical.display().to_string(),
        entries,
        total_bytes,
        skipped_symlinks,
    })
}

/// Writes an inventory (not game assets). Refuses overwrites and directory targets.
pub fn write_inventory_json(inventory: &Inventory, destination: &Path) -> Result<(), InventoryError> {
    let data = serde_json::to_vec_pretty(inventory).map_err(io::Error::other)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    if let Err(err) = output.write_all(&data).and_then(|_| output.sync_all()) {
        let _ = fs::remove_file(destination);
        return Err(InventoryError::Io(err));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_cryengine_assets_and_split_textures() {
        assert_eq!(classify_path(Path::new("textures/Kala_DIFF.DDS.0")), AssetKind::Dds);
        assert_eq!(classify_path(Path::new("goliath_3p.cdf")), AssetKind::CharacterDefinition);
        assert_eq!(classify_path(Path::new("Tech_3p.DBA")), AssetKind::AnimationDba);
        assert_eq!(classify_path(Path::new("UI_Data.PAK")), AssetKind::Pak);
    }

    #[test]
    fn scans_read_only_and_is_deterministic() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("character")).unwrap();
        fs::write(root.path().join("character/z.cdf"), b"cdf").unwrap();
        fs::write(root.path().join("character/a.caf"), b"caf").unwrap();
        let first = scan_directory(root.path()).unwrap();
        let second = scan_directory(root.path()).unwrap();
        assert_eq!(first.entries, second.entries);
        assert_eq!(first.total_bytes, 6);
        assert_eq!(first.entries[0].relative_path, "character/a.caf");
        assert_eq!(first.entries[1].relative_path, "character/z.cdf");
        assert_eq!(first.count_kind(AssetKind::AnimationCaf), 1);
    }

    #[test]
    fn inventory_export_never_overwrites() {
        let source = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let inventory = scan_directory(source.path()).unwrap();
        let output = dest.path().join("inventory.json");
        write_inventory_json(&inventory, &output).unwrap();
        assert!(write_inventory_json(&inventory, &output).is_err());
        let loaded: Inventory = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(loaded.schema, 1);
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks_without_following_them() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("private.pak"), b"data").unwrap();
        symlink(outside.path(), root.path().join("outside")).unwrap();
        let inventory = scan_directory(root.path()).unwrap();
        assert!(inventory.entries.is_empty());
        assert_eq!(inventory.skipped_symlinks, 1);
    }
}
