//! Persistent mod profiles for an isolated signed-PAK staging workspace.
//! Catalog operations are read-only with respect to staged PAKs and the game.
use crate::archive::safe_member_name;
use crate::signed_pak::inspect_signed_pak;
use crate::staging::stage_status;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("Invalid or duplicated profile data: {0}")]
    Invalid(String),
    #[error("Restore the pending stage swap first")]
    Pending,
    #[error("Two enabled mods target the same PAK: {0}")]
    Collision(String),
    #[error("Signed source or stage PAK failed verification: {0}")]
    Verify(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid catalog JSON: {0}")]
    Json(#[from] serde_json::Error),
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileMod {
    pub id: String,
    pub target: String,
    pub source: String,
    pub sha256: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModProfile {
    pub id: String,
    pub name: String,
    pub mods: Vec<ProfileMod>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileCatalog {
    pub schema: u32,
    pub profiles: Vec<ModProfile>,
}
impl Default for ProfileCatalog {
    fn default() -> Self {
        Self {
            schema: 1,
            profiles: vec![],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfilePlan {
    pub profile_id: String,
    pub mods: Vec<ProfileMod>,
    pub total_bytes: u64,
    /// Signed-member inventory changes are diagnostic; they are not proof of
    /// runtime compatibility or game-side mounting.
    #[serde(default)]
    pub member_changes: Vec<ProfileMemberChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileMemberChange {
    pub target: String,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub case_renamed: usize,
}

fn member_changes(
    target: &str,
    old: &[crate::signed_pak::SignedEntry],
    new: &[crate::signed_pak::SignedEntry],
) -> ProfileMemberChange {
    let before: HashMap<_, _> = old.iter().map(|e| (e.path.to_lowercase(), e)).collect();
    let after: HashMap<_, _> = new.iter().map(|e| (e.path.to_lowercase(), e)).collect();
    ProfileMemberChange {
        target: target.to_owned(),
        added: after.keys().filter(|p| !before.contains_key(*p)).count(),
        removed: before.keys().filter(|p| !after.contains_key(*p)).count(),
        changed: before
            .iter()
            .filter(|(p, a)| {
                after
                    .get(*p)
                    .is_some_and(|b| a.crc32 != b.crc32 || a.size != b.size)
            })
            .count(),
        case_renamed: before
            .iter()
            .filter(|(p, a)| after.get(*p).is_some_and(|b| a.path != b.path))
            .count(),
    }
}
fn root(stage: &Path, require_clear: bool) -> Result<PathBuf, ProfileError> {
    let state = stage_status(stage).map_err(|e| ProfileError::Invalid(e.to_string()))?;
    if require_clear && state.is_some() {
        return Err(ProfileError::Pending);
    }
    Ok(fs::canonicalize(stage)?)
}
fn name(input: &str) -> Result<&str, ProfileError> {
    let value = input.trim();
    if value.len() > 80 || value.is_empty() || value.chars().any(char::is_control) {
        return Err(ProfileError::Invalid(
            "Expected 1–80 printable characters".into(),
        ));
    }
    Ok(value)
}
fn target(value: &str) -> Result<String, ProfileError> {
    let value = safe_member_name(value).map_err(|e| ProfileError::Invalid(e.to_string()))?;
    if !value.to_ascii_lowercase().ends_with(".pak") {
        return Err(ProfileError::Invalid("Expected .pak target".into()));
    }
    Ok(value)
}
fn hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn check(catalog: &ProfileCatalog) -> Result<(), ProfileError> {
    if catalog.schema != 1 || catalog.profiles.len() > 100 {
        return Err(ProfileError::Invalid(
            "Invalid schema or profile count".into(),
        ));
    }
    let mut ids = HashSet::new();
    let mut labels = HashSet::new();
    for profile in &catalog.profiles {
        if !hex(&profile.id, 32)
            || !ids.insert(profile.id.clone())
            || name(&profile.name)? != profile.name
            || !labels.insert(profile.name.to_lowercase())
            || profile.mods.len() > 500
        {
            return Err(ProfileError::Invalid("Invalid profile metadata".into()));
        }
        let mut mod_ids = HashSet::new();
        let mut enabled = HashSet::new();
        for item in &profile.mods {
            if !hex(&item.id, 32)
                || !mod_ids.insert(item.id.clone())
                || !hex(&item.sha256, 64)
                || target(&item.target)? != item.target
                || item.source.len() > 4096
                || !Path::new(&item.source).is_absolute()
            {
                return Err(ProfileError::Invalid("Invalid mod source or ID".into()));
            }
            if item.enabled && !enabled.insert(item.target.to_lowercase()) {
                return Err(ProfileError::Collision(item.target.clone()));
            }
        }
    }
    Ok(())
}
pub fn list_mod_profiles(stage: &Path) -> Result<ProfileCatalog, ProfileError> {
    let path = root(stage, false)?.join("profiles.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ProfileCatalog::default()),
        Err(e) => return Err(e.into()),
    };
    if !metadata.file_type().is_file() || metadata.len() > 1024 * 1024 {
        return Err(ProfileError::Invalid("Invalid catalog file".into()));
    }
    let catalog: ProfileCatalog = serde_json::from_slice(&fs::read(path)?)?;
    check(&catalog)?;
    Ok(catalog)
}
fn save(stage: &Path, catalog: &ProfileCatalog) -> Result<(), ProfileError> {
    check(catalog)?;
    let dir = root(stage, true)?;
    let path = dir.join("profiles.json");
    if let Ok(meta) = fs::symlink_metadata(&path)
        && !meta.file_type().is_file()
    {
        return Err(ProfileError::Invalid("Unsafe catalog target".into()));
    }
    let mut file = tempfile::NamedTempFile::new_in(&dir)?;
    serde_json::to_writer_pretty(&mut file, catalog)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| ProfileError::Io(e.error))?;
    Ok(())
}
fn selected<'a>(
    catalog: &'a mut ProfileCatalog,
    id: &str,
) -> Result<&'a mut ModProfile, ProfileError> {
    catalog
        .profiles
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| ProfileError::Invalid("Unknown profile ID".into()))
}
pub fn create_mod_profile(stage: &Path, title: &str) -> Result<ModProfile, ProfileError> {
    let mut catalog = list_mod_profiles(stage)?;
    let title = name(title)?;
    if catalog.profiles.len() == 100
        || catalog
            .profiles
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(title))
    {
        return Err(ProfileError::Invalid(
            "Duplicate or too many profiles".into(),
        ));
    }
    let profile = ModProfile {
        id: format!("{:032x}", rand::random::<u128>()),
        name: title.into(),
        mods: vec![],
    };
    catalog.profiles.push(profile.clone());
    save(stage, &catalog)?;
    Ok(profile)
}
pub fn delete_mod_profile(stage: &Path, id: &str) -> Result<(), ProfileError> {
    let mut catalog = list_mod_profiles(stage)?;
    if !catalog.profiles.iter().any(|p| p.id == id) {
        return Err(ProfileError::Invalid("Unknown profile ID".into()));
    }
    catalog.profiles.retain(|p| p.id != id);
    save(stage, &catalog)
}
fn digest(file: &Path) -> Result<String, ProfileError> {
    let meta = fs::symlink_metadata(file)?;
    if !meta.file_type().is_file() || meta.len() > 16 * 1024 * 1024 * 1024 {
        return Err(ProfileError::Invalid("Invalid or linked source".into()));
    }
    let mut data = File::open(file)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let read = data.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hash.update(&buf[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn verify_pair(
    stage: &Path,
    file: &Path,
    relative: &str,
    original_key: &Path,
    mod_key: &Path,
    basename: &str,
) -> Result<(String, u64, ProfileMemberChange), ProfileError> {
    let directory = root(stage, true)?;
    let virtual_path = target(relative)?;
    let staged = directory.join("paks").join(&virtual_path);
    if !fs::symlink_metadata(&staged)?.file_type().is_file()
        || !fs::canonicalize(&staged)?.starts_with(directory.join("paks"))
    {
        return Err(ProfileError::Invalid(
            "Unsafe or missing stage target".into(),
        ));
    }
    if file.file_name() != staged.file_name() || fs::canonicalize(file)?.starts_with(&directory) {
        return Err(ProfileError::Invalid(
            "Mod must be external and have target basename".into(),
        ));
    }
    // A prepared stage can contain either the original-key version or a
    // locally signed baseline. The installed game build label is advisory;
    // signatures and the member layout remain mandatory before any swap.
    let lowercase = basename.to_ascii_lowercase();
    let old = inspect_signed_pak(&staged, original_key, basename)
        .or_else(|_| inspect_signed_pak(&staged, original_key, &lowercase))
        .or_else(|_| inspect_signed_pak(&staged, mod_key, basename))
        .or_else(|_| inspect_signed_pak(&staged, mod_key, &lowercase))
        .map_err(|e| ProfileError::Verify(e.to_string()))?;
    let new = inspect_signed_pak(file, mod_key, basename)
        .or_else(|_| inspect_signed_pak(file, mod_key, &lowercase))
        .map_err(|e| ProfileError::Verify(e.to_string()))?;
    // Different game revisions can legitimately add or remove PAK members.
    // A mismatch is a compatibility warning, not a failed cryptographic
    // signature. The offline staging journal still preserves exact originals.
    let changes = member_changes(&virtual_path, &old.entries, &new.entries);
    Ok((digest(file)?, fs::metadata(file)?.len(), changes))
}
pub fn add_mod_to_profile(
    stage: &Path,
    profile_id: &str,
    source: &Path,
    relative: &str,
    original_key: &Path,
    mod_key: &Path,
    basename: &str,
) -> Result<ProfileMod, ProfileError> {
    let (sha256, _, _) = verify_pair(stage, source, relative, original_key, mod_key, basename)?;
    let mut catalog = list_mod_profiles(stage)?;
    let profile = selected(&mut catalog, profile_id)?;
    if profile.mods.len() >= 500
        || profile
            .mods
            .iter()
            .any(|m| m.enabled && m.target.eq_ignore_ascii_case(relative))
    {
        return Err(ProfileError::Collision(relative.into()));
    }
    let item = ProfileMod {
        id: format!("{:032x}", rand::random::<u128>()),
        target: target(relative)?,
        source: fs::canonicalize(source)?.to_string_lossy().into(),
        sha256,
        enabled: true,
    };
    profile.mods.push(item.clone());
    save(stage, &catalog)?;
    Ok(item)
}
pub fn enable_profile_mod(
    stage: &Path,
    profile_id: &str,
    mod_id: &str,
    enabled: bool,
) -> Result<(), ProfileError> {
    let mut catalog = list_mod_profiles(stage)?;
    let profile = selected(&mut catalog, profile_id)?;
    let record = profile
        .mods
        .iter()
        .find(|m| m.id == mod_id)
        .ok_or_else(|| ProfileError::Invalid("Unknown mod ID".into()))?;
    if enabled
        && profile
            .mods
            .iter()
            .any(|m| m.id != mod_id && m.enabled && m.target.eq_ignore_ascii_case(&record.target))
    {
        return Err(ProfileError::Collision(record.target.clone()));
    }
    profile
        .mods
        .iter_mut()
        .find(|m| m.id == mod_id)
        .expect("validated")
        .enabled = enabled;
    save(stage, &catalog)
}
pub fn plan_mod_profile(
    stage: &Path,
    profile_id: &str,
    original_key: &Path,
    mod_key: &Path,
) -> Result<ProfilePlan, ProfileError> {
    let catalog = list_mod_profiles(stage)?;
    let profile = catalog
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| ProfileError::Invalid("Unknown profile ID".into()))?;
    let mut mods = Vec::new();
    let mut size = 0u64;
    let mut member_reports = Vec::new();
    for item in profile.mods.iter().filter(|m| m.enabled) {
        let (current, bytes, changes) = verify_pair(
            stage,
            Path::new(&item.source),
            &item.target,
            original_key,
            mod_key,
            item.target.rsplit('/').next().unwrap_or_default(),
        )?;
        // Iterating on a mod rebuild changes its SHA. A formerly registered
        // checksum is informational after the replacement has just passed
        // signed-archive and member-layout verification. Update the active
        // plan's digest; recovery continues to use the freshly verified bytes.
        let mut refreshed = item.clone();
        refreshed.sha256 = current;
        size = size
            .checked_add(bytes)
            .ok_or_else(|| ProfileError::Invalid("Plan size overflow".into()))?;
        if size > 16 * 1024 * 1024 * 1024 {
            return Err(ProfileError::Invalid("Plan exceeds safety limit".into()));
        }
        mods.push(refreshed);
        member_reports.push(changes);
    }
    Ok(ProfilePlan {
        profile_id: profile_id.into(),
        mods,
        total_bytes: size,
        member_changes: member_reports,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::staging::initialize_stage;
    #[test]
    fn inventory_report_distinguishes_add_replace_remove_and_case_rename() {
        let entry = |path: &str, crc32: u32| crate::signed_pak::SignedEntry {
            path: path.into(),
            method: 14,
            flags: 0,
            crc32,
            compressed_size: 1,
            size: 2,
            encrypted_data_offset: 0,
        };
        let result = member_changes(
            "Game/UI_Data.pak",
            &[
                entry("Keep.xml", 1),
                entry("Replace.xml", 2),
                entry("Lost.xml", 3),
            ],
            &[
                entry("keep.xml", 1),
                entry("Replace.xml", 4),
                entry("New.gfx", 5),
            ],
        );
        assert_eq!(
            (
                result.added,
                result.removed,
                result.changed,
                result.case_renamed
            ),
            (1, 1, 1, 1)
        );
    }
    #[test]
    fn profiles_persist_and_reject_broken_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let stage = tmp.path().join("stage");
        initialize_stage(&stage).unwrap();
        let created = create_mod_profile(&stage, "Hunters").unwrap();
        assert!(create_mod_profile(&stage, "hunters").is_err());
        assert_eq!(list_mod_profiles(&stage).unwrap().profiles.len(), 1);
        let file = tmp.path().join("mods.pak");
        fs::write(&file, b"not signed").unwrap();
        assert!(
            add_mod_to_profile(
                &stage,
                &created.id,
                &file,
                "Game/mods.pak",
                &file,
                &file,
                "mods.pak"
            )
            .is_err()
        );
        delete_mod_profile(&stage, &created.id).unwrap();
        assert!(list_mod_profiles(&stage).unwrap().profiles.is_empty());
        fs::write(stage.join("profiles.json"), b"{invalid").unwrap();
        assert!(create_mod_profile(&stage, "new").is_err());
    }
}
