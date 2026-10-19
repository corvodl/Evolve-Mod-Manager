//! Persistent per-user preferences shared by portable and installed builds.
//! Settings mirror the established manager keys where possible and never
//! contain keys, game assets, or session credentials.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub ui_density: String,
    pub file_explorer_side: String,
    pub viewer_split: f32,
    pub viewer_show_inspector: bool,
    pub viewer_wireframe: bool,
    pub animation_speed: f32,
    pub animation_loop: bool,
    pub saved_yaw: f32,
    pub saved_pitch: f32,
    pub saved_zoom: f32,
    pub appearance_mode: String,
    pub appearance_colors: BTreeMap<String, String>,
    pub theme_presets: BTreeMap<String, BTreeMap<String, String>>,
    pub font_family: String,
    pub reduce_motion: bool,
    pub keep_original_snapshots: bool,
    pub show_activity_on_tasks: bool,
    pub stage: String,
    pub swap: String,
    pub projects: String,
    pub game_storage_root: String,
    pub current_workspace: String,
    pub output_pak: String,
    pub batch_root: String,
    pub archive_label: String,
    pub asset_library_root: String,
    pub blender_executable: String,
    pub experimental_updates: bool,
    pub automatic_update_checks: bool,
    pub create_desktop_shortcut: bool,
    /// Retain older application settings that the Rust interface does not
    /// interpret yet. Existing user choices survive a Save Settings action.
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub previous_options: BTreeMap<String, serde_json::Value>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            ui_density: "Comfortable".into(),
            file_explorer_side: "Right".into(),
            viewer_split: 0.67,
            viewer_show_inspector: false,
            viewer_wireframe: false,
            animation_speed: 1.0,
            animation_loop: false,
            saved_yaw: 0.0,
            saved_pitch: 0.0,
            saved_zoom: 1.0,
            appearance_mode: "Dark".into(),
            appearance_colors: BTreeMap::new(),
            theme_presets: BTreeMap::new(),
            font_family: "Default".into(),
            reduce_motion: false,
            keep_original_snapshots: true,
            show_activity_on_tasks: true,
            stage: String::new(),
            swap: String::new(),
            projects: String::new(),
            game_storage_root: String::new(),
            current_workspace: String::new(),
            output_pak: String::new(),
            batch_root: String::new(),
            archive_label: String::new(),
            asset_library_root: String::new(),
            blender_executable: String::new(),
            experimental_updates: false,
            automatic_update_checks: true,
            create_desktop_shortcut: false,
            previous_options: BTreeMap::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("Settings JSON is invalid: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("Cannot read or save preferences: {0}")]
    Io(#[from] io::Error),
    #[error("An unsafe preference value was rejected")]
    InvalidValue,
}

pub fn settings_path() -> Result<PathBuf, SettingsError> {
    // Deliberately don't save next to the executable. The portable build and
    // in-place updater share per-user settings without copying private data.
    #[cfg(windows)]
    let home = std::env::var_os("LOCALAPPDATA").ok_or(SettingsError::InvalidValue)?;
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME").ok_or(SettingsError::InvalidValue)?;
    let base = PathBuf::from(home);
    #[cfg(windows)]
    let root = base.join("EvolveModManager");
    #[cfg(not(windows))]
    let root = base.join(".config").join("EvolveModManager");
    Ok(root.join("settings.json"))
}

impl Preferences {
    pub fn validate(&mut self) -> Result<(), SettingsError> {
        if !["Compact", "Comfortable"].contains(&self.ui_density.as_str()) {
            self.ui_density = "Comfortable".into();
        }
        if !["Right", "Left"].contains(&self.file_explorer_side.as_str()) {
            self.file_explorer_side = "Right".into();
        }
        if !self.viewer_split.is_finite() || !(0.25..=0.8).contains(&self.viewer_split) {
            self.viewer_split = 0.67;
        }
        if !self.animation_speed.is_finite() || !(0.1..=3.0).contains(&self.animation_speed) {
            self.animation_speed = 1.0;
        }
        if !self.saved_yaw.is_finite()
            || !self.saved_pitch.is_finite()
            || !self.saved_zoom.is_finite()
        {
            self.saved_yaw = 0.0;
            self.saved_pitch = 0.0;
            self.saved_zoom = 1.0;
        }
        self.saved_zoom = self.saved_zoom.clamp(0.2, 4.0);
        if !["Dark", "Light"].contains(&self.appearance_mode.as_str()) {
            self.appearance_mode = "Dark".into();
        }
        if self.font_family.len() > 120 || self.archive_label.len() > 200 {
            return Err(SettingsError::InvalidValue);
        }
        for path in [
            &self.stage,
            &self.swap,
            &self.projects,
            &self.game_storage_root,
            &self.current_workspace,
            &self.output_pak,
            &self.batch_root,
            &self.asset_library_root,
            &self.blender_executable,
        ] {
            if path.len() > 4096 || path.contains('\0') {
                return Err(SettingsError::InvalidValue);
            }
        }
        if self.previous_options.len() > 256 {
            return Err(SettingsError::InvalidValue);
        }
        if self.appearance_colors.len() > 64 || self.theme_presets.len() > 64 {
            return Err(SettingsError::InvalidValue);
        }
        for (role, color) in &self.appearance_colors {
            if !valid_theme_entry(role, color) {
                return Err(SettingsError::InvalidValue);
            }
        }
        for (name, colors) in &self.theme_presets {
            if name.len() > 120 || colors.len() > 64 {
                return Err(SettingsError::InvalidValue);
            }
            if colors
                .iter()
                .any(|(role, color)| !valid_theme_entry(role, color))
            {
                return Err(SettingsError::InvalidValue);
            }
        }
        Ok(())
    }
}

fn valid_theme_entry(role: &str, value: &str) -> bool {
    role.len() <= 80
        && role.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}

pub fn load_from(path: &Path) -> Result<Preferences, SettingsError> {
    // The previous Windows manager stores preferences relative to its install
    // directory. Read that file when the new Rust profile has not been saved.
    // Never move, rewrite or delete the original preferences or recovery data.
    let legacy = path
        .parent()
        .ok_or(SettingsError::InvalidValue)?
        .join("Data")
        .join("Settings")
        .join("manager_settings.json");
    let source = if path.exists() {
        path
    } else if legacy.exists() {
        legacy.as_path()
    } else {
        return Ok(Preferences::default());
    };
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.file_type().is_file() || metadata.len() > 512 * 1024 {
        return Err(SettingsError::InvalidValue);
    }
    let mut prefs: Preferences = serde_json::from_slice(&fs::read(source)?)?;
    prefs.validate()?;
    Ok(prefs)
}

pub fn save_to(path: &Path, prefs: &Preferences) -> Result<(), SettingsError> {
    let mut checked = prefs.clone();
    checked.validate()?;
    let folder = path.parent().ok_or(SettingsError::InvalidValue)?;
    fs::create_dir_all(folder)?;
    if fs::symlink_metadata(folder)?.file_type().is_symlink()
        || fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_file())
    {
        return Err(SettingsError::InvalidValue);
    }
    let bytes = serde_json::to_vec_pretty(&checked)?;
    // Atomic staged settings replacement, even if the application or machine
    // closes in the middle of a write.
    let mut temporary = tempfile::Builder::new()
        .prefix(".preferences-")
        .tempfile_in(folder)?;
    std::io::Write::write_all(temporary.as_file_mut(), &bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path)
        .map_err(|e| SettingsError::Io(e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrates_familiar_settings_keys_without_forgetting_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{
            "ui_density":"Compact","appearance_mode":"Light",
            "file_explorer_side":"Left","keep_original_snapshots":false,
            "show_activity_on_tasks":false,"current_workspace":"C:\\Projects\\Evolve",
            "another_old_option":true
        }"#,
        )
        .unwrap();
        let prefs = load_from(&path).unwrap();
        assert_eq!(prefs.ui_density, "Compact");
        assert_eq!(prefs.file_explorer_side, "Left");
        assert!(!prefs.keep_original_snapshots);
        assert!(!prefs.experimental_updates);
        assert_eq!(
            prefs.previous_options.get("another_old_option"),
            Some(&serde_json::json!(true))
        );
        save_to(&path, &prefs).unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("another_old_option")
        );
        assert_eq!(load_from(&path).unwrap().appearance_mode, "Light");
    }
    #[test]
    fn uses_previous_installed_settings_without_changing_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let old_path = dir
            .path()
            .join("Data")
            .join("Settings")
            .join("manager_settings.json");
        fs::create_dir_all(old_path.parent().unwrap()).unwrap();
        let original =
            br#"{"appearance_mode":"Light","ui_density":"Compact","projects":"C:\\Games\\Mods"}"#;
        fs::write(&old_path, original).unwrap();
        let migrated = load_from(&path).unwrap();
        assert_eq!(migrated.appearance_mode, "Light");
        assert_eq!(migrated.ui_density, "Compact");
        assert!(!path.exists());
        assert_eq!(fs::read(&old_path).unwrap(), original);
    }
    #[test]
    fn malformed_settings_are_not_silently_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, b"{no").unwrap();
        assert!(load_from(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{no");
    }
    #[test]
    fn rejects_unsafe_colors_and_preserves_existing_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let prefs = Preferences::default();
        save_to(&path, &prefs).unwrap();
        let original = fs::read(&path).unwrap();
        let mut invalid = prefs;
        invalid
            .appearance_colors
            .insert("primary".into(), "invalid".into());
        assert!(save_to(&path, &invalid).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}
