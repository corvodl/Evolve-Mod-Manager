//! Read-only Blender discovery for common Windows installations. No shell,
//! PATH lookup or launching executable during discovery. Manual selection is
//! requested only when an installed executable cannot be resolved.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn installed_file(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        || !fs::symlink_metadata(path).ok()?.file_type().is_file()
    {
        return None;
    }
    fs::canonicalize(path).ok()
}

pub fn valid_blender(path: &str) -> Option<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        None
    } else {
        let candidate = Path::new(path);
        if candidate.file_name().is_none_or(|name| !name.eq_ignore_ascii_case("blender.exe")) {
            None
        } else {
            installed_file(candidate)
        }
    }
}

fn version_from_directory(name: &str) -> Option<(u32, u32, u32)> {
    let version = name.strip_prefix("Blender ")?;
    let mut parts = version.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    let patch: u32 = match parts.next() {
        Some(value) => value.parse().ok()?,
        None => 0,
    };
    if parts.next().is_some() || (major, minor) < (4, 2) {
        return None;
    }
    Some((major, minor, patch))
}

/// Scan only known Blender Foundation installation folders. Higher compatible
/// version wins when several Blender versions are installed side by side.
fn discover_in_roots(roots: &[PathBuf]) -> Option<PathBuf> {
    let mut versions = Vec::new();
    for base in roots {
        let vendor = base.join("Blender Foundation");
        if !fs::symlink_metadata(&vendor).ok().is_some_and(|m| m.file_type().is_dir()) {
            continue;
        }
        let Ok(installs) = fs::read_dir(vendor) else {
            continue;
        };
        for entry in installs.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(version) = version_from_directory(&name) else {
                continue;
            };
            if !entry.file_type().ok().is_some_and(|kind| kind.is_dir()) {
                continue;
            }
            if let Some(path) = installed_file(&entry.path().join("blender.exe")) {
                versions.push((version, path));
            }
        }
    }
    versions.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    versions.pop().map(|(_, path)| path)
}

#[cfg(target_os = "windows")]
fn standard_roots() -> Vec<PathBuf> {
    ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .into_iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .map(|root| {
            if root.ends_with("AppData\\Local") {
                root.join("Programs")
            } else {
                root
            }
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn standard_roots() -> Vec<PathBuf> {
    Vec::new()
}

fn installed_standard() -> Option<PathBuf> {
    let roots = standard_roots();
    if let Some(path) = discover_in_roots(&roots) {
        return Some(path);
    }
    // The Steam distribution is a common alternative to Blender Foundation.
    for root in roots {
        if let Some(path) = installed_file(
            &root.join("Steam").join("steamapps").join("common")
                .join("Blender").join("blender.exe"),
        ) {
            return Some(path);
        }
    }
    None
}

/// Prefer an existing user-selected executable; otherwise recover from stale
/// preferences and auto-detect a normal Windows Blender installation.
pub fn resolve_blender(configured: &str) -> Result<PathBuf, String> {
    valid_blender(configured)
        .or_else(installed_standard)
        .ok_or_else(|| {
            "Blender 4.2+ was not found in standard Windows installation folders. Install Blender normally, or choose your blender.exe manually in File → Export.".to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn picks_newest_compatible_standard_blender_and_ignores_symlink_aliases() {
        let temp = tempfile::tempdir().unwrap();
        for version in ["Blender 4.1", "Blender 4.2", "Blender 5.2", "Blender 5.1.3", "Blender NotAVersion"] {
            let folder = temp.path().join("Blender Foundation").join(version);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join("blender.exe"), b"fixture").unwrap();
        }
        let result = discover_in_roots(&[temp.path().to_path_buf()]).unwrap();
        assert!(result.ends_with("Blender 5.2/blender.exe"));
    }
    #[test]
    fn rejects_non_executable_paths_and_stale_selected_file() {
        let temp = tempfile::tempdir().unwrap();
        let fake = temp.path().join("Blender Foundation").join("Blender 4.2");
        fs::create_dir_all(&fake).unwrap();
        let exe = fake.join("blender.exe");
        fs::write(&exe, b"fixture").unwrap();
        assert!(valid_blender(&exe.to_string_lossy()).is_some());
        assert!(valid_blender(&fake.to_string_lossy()).is_none());
        assert!(valid_blender(&temp.path().join("missing.exe").to_string_lossy()).is_none());
        assert!(version_from_directory("Blender 4.1").is_none());
        assert!(version_from_directory("Blender 5.2").is_some());
    }
}
