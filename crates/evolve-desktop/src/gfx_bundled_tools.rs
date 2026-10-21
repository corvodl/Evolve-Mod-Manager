//! Discover optional, separately licensed GFX tools staged by the installer.
//! User-selected paths always win. Portable distributions do not include
//! these binaries; discovery is best-effort and never downloads or installs.
use crate::settings::Preferences;
use std::fs;
use std::path::Path;

fn regular_tool(directory: &Path, relative: &str) -> Option<PathBuf> {
    let candidate = directory.join(relative);
    let info = fs::symlink_metadata(&candidate).ok()?;
    if !info.file_type().is_file() || info.len() == 0 {
        return None;
    }
    fs::canonicalize(candidate).ok()
}
pub fn fill_empty_paths_at(preferences: &mut Preferences, executable_dir: &Path) {
    if preferences.ffdec_executable.trim().is_empty() {
        if let Some(path) = regular_tool(executable_dir, "third_party/FFDec/ffdec-cli.exe") {
            preferences.ffdec_executable = path.to_string_lossy().into_owned();
        }
    }
    if preferences.ruffle_executable.trim().is_empty() {
        if let Some(path) = regular_tool(executable_dir, "third_party/Ruffle/ruffle.exe") {
            preferences.ruffle_executable = path.to_string_lossy().into_owned();
        }
    }
}
pub fn detect_bundled(preferences: &mut Preferences) {
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        fill_empty_paths_at(preferences, directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_discovers_only_existing_regular_bundled_files() {
        let temp = tempfile::tempdir().unwrap();
        let ffdec = temp.path().join("third_party/FFDec");
        let ruffle = temp.path().join("third_party/Ruffle");
        fs::create_dir_all(&ffdec).unwrap();
        fs::create_dir_all(&ruffle).unwrap();
        fs::write(ffdec.join("ffdec-cli.exe"), b"exe").unwrap();
        fs::write(ruffle.join("ruffle.exe"), b"exe").unwrap();
        let mut prefs = Preferences::default();
        fill_empty_paths_at(&mut prefs, temp.path());
        assert!(prefs.ffdec_executable.ends_with("ffdec-cli.exe"));
        assert!(prefs.ruffle_executable.ends_with("ruffle.exe"));
    }
    #[test]
    fn preserves_custom_preferences_and_does_not_invent_missing_binaries() {
        let temp = tempfile::tempdir().unwrap();
        let mut prefs = Preferences {
            ffdec_executable: "C:/Tools/ffdec-cli.exe".into(),
            ..Preferences::default()
        };
        fill_empty_paths_at(&mut prefs, temp.path());
        assert_eq!(prefs.ffdec_executable, "C:/Tools/ffdec-cli.exe");
        assert!(prefs.ruffle_executable.is_empty());
    }
}
