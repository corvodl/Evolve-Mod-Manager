//! Discover optional, separately licensed GFX tools staged by the installer.
//! User-selected paths always win. Portable distributions do not include
//! these binaries; discovery is best-effort and never downloads or installs.
use crate::settings::Preferences;
use std::fs;
use std::path::{Path, PathBuf};

fn regular_tool(directory: &Path, relative: &str) -> Option<PathBuf> {
    let candidate = directory.join(relative);
    let info = fs::symlink_metadata(&candidate).ok()?;
    if !info.file_type().is_file() || info.len() == 0 {
        return None;
    }
    fs::canonicalize(candidate).ok()
}
/// A status check, not an execution or signature verification. Installer and
/// in-app downloads use pinned archives with SHA-256 validation.
pub fn valid_ffdec(path: &str) -> Option<PathBuf> {
    let candidate = Path::new(path.trim());
    let filename = candidate.file_name()?.to_str()?;
    if !filename.eq_ignore_ascii_case("ffdec-cli.exe")
        && !filename.eq_ignore_ascii_case("ffdec.jar")
    {
        return None;
    }
    if !candidate.is_absolute() {
        return None;
    }
    let info = fs::symlink_metadata(candidate).ok()?;
    if !info.file_type().is_file() || info.len() == 0 {
        return None;
    }
    fs::canonicalize(candidate).ok()
}
pub fn valid_ruffle(path: &str) -> Option<PathBuf> {
    let candidate = Path::new(path.trim());
    if !candidate.is_absolute() || !candidate.file_name()?.eq_ignore_ascii_case("ruffle.exe") {
        return None;
    }
    let info = fs::symlink_metadata(candidate).ok()?;
    if !info.file_type().is_file() || info.len() == 0 {
        return None;
    }
    fs::canonicalize(candidate).ok()
}

/// A self-hosted Web Ruffle runtime is a JS/WASM directory, NOT ruffle.exe.
/// The executable name is fixed, and a co-located non-empty WebAssembly file
/// is required so Integration Settings cannot report a broken install as ready.
/// This is structural discovery; pinned SHA checks occur when downloading.
pub fn valid_ruffle_web(path: &str) -> Option<PathBuf> {
    let candidate = Path::new(path.trim());
    if !candidate.is_absolute() || !candidate.file_name()?.eq_ignore_ascii_case("ruffle.js") {
        return None;
    }
    let info = fs::symlink_metadata(candidate).ok()?;
    if !info.file_type().is_file() || info.len() == 0 {
        return None;
    }
    let folder = candidate.parent()?;
    if !fs::read_dir(folder).ok()?.flatten().any(|item| {
        item.path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("wasm"))
            && fs::symlink_metadata(item.path())
                .is_ok_and(|meta| meta.file_type().is_file() && meta.len() > 0)
    }) {
        return None;
    }
    fs::canonicalize(candidate).ok()
}

/// The bundled runtime takes precedence only if no custom path was entered.
pub fn detected_ruffle_web(configured: &str, executable_dir: &Path) -> Option<PathBuf> {
    if !configured.trim().is_empty() {
        return valid_ruffle_web(configured);
    }
    let candidate = executable_dir.join("third_party/RuffleWeb/ruffle.js");
    valid_ruffle_web(candidate.to_str()?)
}

pub fn fill_empty_paths_at(preferences: &mut Preferences, executable_dir: &Path) {
    if preferences.oggvorbis_remux_executable.trim().is_empty()
        && let Some(path) = regular_tool(
            executable_dir,
            "third_party/oggvorbis2fsb5/oggvorbis2fsb5.exe",
        )
    {
        preferences.oggvorbis_remux_executable = path.to_string_lossy().into_owned();
    }
    if preferences.vgmstream_executable.trim().is_empty()
        && let Some(path) = crate::audio_tools::default_decoder(executable_dir)
    {
        preferences.vgmstream_executable = path.to_string_lossy().into_owned();
    }
    if preferences.fmod_designer_executable.trim().is_empty()
        && let Some(path) = crate::audio_tools::default_designer()
    {
        preferences.fmod_designer_executable = path.to_string_lossy().into_owned();
    }
    if preferences.ffdec_executable.trim().is_empty()
        && let Some(path) = regular_tool(executable_dir, "third_party/FFDec/ffdec-cli.exe")
    {
        preferences.ffdec_executable = path.to_string_lossy().into_owned();
    }
    if preferences.ruffle_executable.trim().is_empty()
        && let Some(path) = regular_tool(executable_dir, "third_party/Ruffle/ruffle.exe")
    {
        preferences.ruffle_executable = path.to_string_lossy().into_owned();
    }
    if preferences.ruffle_web_runtime.trim().is_empty()
        && let Some(path) = detected_ruffle_web("", executable_dir)
    {
        preferences.ruffle_web_runtime = path.to_string_lossy().into_owned();
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
    fn ruffle_web_requires_js_and_wasm_and_is_independent_of_desktop_player() {
        let root = tempfile::tempdir().unwrap();
        let web = root.path().join("third_party/RuffleWeb");
        let desktop = root.path().join("third_party/Ruffle");
        fs::create_dir_all(&web).unwrap();
        fs::create_dir_all(&desktop).unwrap();
        fs::write(desktop.join("ruffle.exe"), b"desktop").unwrap();
        fs::write(web.join("ruffle.js"), b"window.RufflePlayer = {}").unwrap();
        let js = web.join("ruffle.js").to_string_lossy().into_owned();
        assert!(valid_ruffle_web(&js).is_none());
        fs::write(web.join("ruffle.wasm"), b"wasm").unwrap();
        assert_eq!(
            valid_ruffle_web(&js),
            fs::canonicalize(web.join("ruffle.js")).ok()
        );
        assert!(detected_ruffle_web("", root.path()).is_some());
        assert!(detected_ruffle_web(&js, root.path()).is_some());
        assert!(detected_ruffle_web("C:/missing/ruffle.js", root.path()).is_none());
        let mut prefs = Preferences::default();
        fill_empty_paths_at(&mut prefs, root.path());
        assert!(prefs.ruffle_executable.ends_with("ruffle.exe"));
        assert!(prefs.ruffle_web_runtime.ends_with("ruffle.js"));
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
