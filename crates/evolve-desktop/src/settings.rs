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
    pub viewer_explorer_visible: bool,
    pub viewer_split: f32,
    pub light_strength: f32,
    pub viewer_show_inspector: bool,
    pub viewer_wireframe: bool,
    pub animation_speed: f32,
    pub animation_loop: bool,
    pub saved_yaw: f32,
    pub saved_pitch: f32,
    pub saved_zoom: f32,
    pub appearance_mode: String,
    /// Optional native Evolve Legacy styling; existing Modern UI remains default.
    pub ui_theme: String,
    /// Keep Evolve artwork and translucent Home panels on by default.
    /// Turning it off restores opaque Home surfaces, independent of theme.
    pub show_home_background: bool,
    pub appearance_colors: BTreeMap<String, String>,
    pub theme_presets: BTreeMap<String, BTreeMap<String, String>>,
    pub font_family: String,
    pub reduce_motion: bool,
    pub keep_original_snapshots: bool,
    pub show_activity_on_tasks: bool,
    /// On by default for both portable and installed builds. Can be disabled in Advanced.
    pub crash_logging_enabled: bool,
    /// Installed EvolveGame root (bin64_SteamRetail is accepted in setup).
    pub game_installation: String,
    /// Normal Modded Evolve launcher EXE selected by the user.
    pub client_executable: String,
    /// The separately installed test client and test game are not production.
    pub test_client_executable: String,
    pub test_game_installation: String,
    /// Opt in to the separate test pair for both Play actions.
    pub play_through_test_client: bool,
    /// Test-only stage and rollback journal, never shared with production.
    pub test_stage: String,
    pub test_swap: String,
    /// Client Launch (full relay/profile/overlay) is default; Direct is opt-in testing.
    pub game_launch_mode: String,
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
    pub ffdec_executable: String,
    /// Optional open-source FSB decoder used for WAV preview/export.
    pub vgmstream_executable: String,
    /// MIT-licensed Vorbis-to-FSB5 remuxer for native bank rebuilding.
    pub oggvorbis_remux_executable: String,
    /// User-installed legacy FMOD Designer editor (never bundled).
    pub fmod_designer_executable: String,
    /// Optional separately-installed Ruffle desktop player for ActionScript previews.
    pub ruffle_executable: String,
    /// Optional self-hosted Ruffle Web ruffle.js for persistent live GFX controls.
    /// Separate from the standalone desktop ruffle.exe executable.
    pub ruffle_web_runtime: String,
    /// Optional user-selected FFmpeg executable for source-verified USM menu video previews.
    pub gfx_ffmpeg_executable: String,
    pub experimental_updates: bool,
    /// Separate opt-in for prerelease Nightly builds.
    pub nightly_updates: bool,
    pub automatic_update_checks: bool,
    /// Non-sensitive user-selected Developer overlay hotkey presets (0=disabled).
    // Vector preserves existing 9-entry settings while adding the Fly toggle as slot 10.
    pub developer_hotkeys: Vec<u8>,
    pub discord_presence_enabled: bool,
    pub discord_application_id: String,
    pub discord_show_game: bool,
    pub discord_show_manager: bool,
    pub discord_show_character: bool,
    pub discord_show_stats: bool,
    pub discord_show_profile: bool,
    pub discord_minimal: bool,
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
            file_explorer_side: "Left".into(),
            viewer_explorer_visible: true,
            viewer_split: 0.67,
            light_strength: 1.6,
            viewer_show_inspector: false,
            viewer_wireframe: false,
            animation_speed: 1.0,
            animation_loop: false,
            saved_yaw: 0.0,
            saved_pitch: 0.0,
            saved_zoom: 1.0,
            appearance_mode: "Dark".into(),
            ui_theme: "Modern".into(),
            show_home_background: true,
            appearance_colors: BTreeMap::new(),
            theme_presets: BTreeMap::new(),
            font_family: "Default".into(),
            reduce_motion: false,
            keep_original_snapshots: false,
            show_activity_on_tasks: true,
            crash_logging_enabled: true,
            game_installation: String::new(),
            client_executable: String::new(),
            test_client_executable: String::new(),
            test_game_installation: String::new(),
            play_through_test_client: false,
            test_stage: String::new(),
            test_swap: String::new(),
            game_launch_mode: "Client Launch".into(),
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
            ffdec_executable: String::new(),
            vgmstream_executable: String::new(),
            oggvorbis_remux_executable: String::new(),
            fmod_designer_executable: String::new(),
            ruffle_executable: String::new(),
            ruffle_web_runtime: String::new(),
            gfx_ffmpeg_executable: String::new(),
            experimental_updates: false,
            nightly_updates: env!("CARGO_PKG_VERSION").contains("-nightly."),
            automatic_update_checks: true,
            developer_hotkeys: vec![0; 10],
            discord_presence_enabled: false,
            discord_application_id: String::new(),
            discord_show_game: true,
            discord_show_manager: true,
            discord_show_character: true,
            discord_show_stats: false,
            discord_show_profile: false,
            discord_minimal: false,
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

/// Keep the private manager's on-disk layout. The Rust rewrite should
/// never require users to re-enter the location on the Asset Library page.
pub fn library_directory(settings_file: &Path, preference: &str) -> Result<PathBuf, SettingsError> {
    if !preference.trim().is_empty() {
        return Ok(PathBuf::from(preference));
    }
    let profile = settings_file.parent().ok_or(SettingsError::InvalidValue)?;
    Ok(profile.join("Data").join("AssetLibrary"))
}

/// Shared GameSets root for new installations. Existing stage/swap/projects
/// preferences remain untouched and continue to point at their old setup.
pub fn game_storage_directory(
    settings_file: &Path,
    preference: &str,
) -> Result<PathBuf, SettingsError> {
    if !preference.trim().is_empty() {
        return Ok(PathBuf::from(preference));
    }
    let profile = settings_file.parent().ok_or(SettingsError::InvalidValue)?;
    Ok(profile.join("Data").join("GameSets"))
}

/// Resolve the destination for automatic setup, independent of legacy stages
/// and without relying on the process working directory. A missing GameSets
/// value (or an old relative UI label) means the Rust manager's per-user
/// GameSets folder. Explicit custom destinations must be absolute and safe.
pub fn setup_storage_directory(
    settings_file: &Path,
    preference: &str,
) -> Result<PathBuf, SettingsError> {
    let raw = preference.trim();
    let alias = raw.replace('\\', "/");
    if raw.is_empty()
        || alias.eq_ignore_ascii_case("GameSets")
        || alias.eq_ignore_ascii_case("Data/GameSets")
    {
        return game_storage_directory(settings_file, "");
    }
    let selected = PathBuf::from(raw);
    if !selected.is_absolute()
        || selected
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(SettingsError::InvalidValue);
    }
    Ok(selected)
}

/// Only change the two accent roles; keep any custom surface, typography,
/// or other private-manager palette choices intact.
pub fn apply_accent_preset(prefs: &mut Preferences, preset: &str) -> bool {
    let colors = match preset {
        "Classic Red" => Some(("#c9384a", "#48303b")),
        "Evolve Red" => Some(("#e0354f", "#5d2936")),
        "Deep Red" => Some(("#a91f38", "#472631")),
        "Default" => None,
        _ => return false,
    };
    if let Some((accent, selected)) = colors {
        prefs
            .appearance_colors
            .insert("accent".into(), accent.into());
        prefs
            .appearance_colors
            .insert("selection".into(), selected.into());
    } else {
        prefs.appearance_colors.remove("accent");
        prefs.appearance_colors.remove("selection");
    }
    true
}

impl Preferences {
    pub fn validate(&mut self) -> Result<(), SettingsError> {
        // Old installations saved nine shortcuts; retain them and add an
        // unassigned Fly Mode binding without invalidating their settings.
        self.developer_hotkeys.resize(10, 0);
        self.developer_hotkeys.truncate(10);
        let mut seen = std::collections::HashSet::new();
        for key in &mut self.developer_hotkeys {
            if *key > 28 || (*key != 0 && !seen.insert(*key)) {
                *key = 0;
            }
        }
        if !["Compact", "Comfortable"].contains(&self.ui_density.as_str()) {
            self.ui_density = "Comfortable".into();
        }
        if !["Direct Launch", "Client Launch"].contains(&self.game_launch_mode.as_str()) {
            self.game_launch_mode = "Client Launch".into();
        }
        if !["Right", "Left"].contains(&self.file_explorer_side.as_str()) {
            self.file_explorer_side = "Left".into();
        }
        if !self.viewer_split.is_finite() || !(0.25..=0.8).contains(&self.viewer_split) {
            self.viewer_split = 0.67;
        }
        if !self.light_strength.is_finite() || !(0.2..=4.0).contains(&self.light_strength) {
            self.light_strength = 1.6;
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
        if !["Modern", "Evolve Legacy"].contains(&self.ui_theme.as_str()) {
            self.ui_theme = "Modern".into();
        }
        if !["Dark", "Light"].contains(&self.appearance_mode.as_str()) {
            self.appearance_mode = "Dark".into();
        }
        if !self.discord_application_id.is_empty()
            && (self.discord_application_id.len() < 15
                || self.discord_application_id.len() > 25
                || !self
                    .discord_application_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit()))
        {
            return Err(SettingsError::InvalidValue);
        }
        if self.font_family.len() > 120 || self.archive_label.len() > 200 {
            return Err(SettingsError::InvalidValue);
        }
        for path in [
            &self.stage,
            &self.swap,
            &self.test_game_installation,
            &self.test_client_executable,
            &self.test_stage,
            &self.test_swap,
            &self.projects,
            &self.game_storage_root,
            &self.current_workspace,
            &self.output_pak,
            &self.batch_root,
            &self.asset_library_root,
            &self.blender_executable,
            &self.ffdec_executable,
            &self.vgmstream_executable,
            &self.oggvorbis_remux_executable,
            &self.fmod_designer_executable,
            &self.ruffle_executable,
            &self.ruffle_web_runtime,
            &self.gfx_ffmpeg_executable,
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
    fn nine_existing_hotkeys_migrate_without_losing_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"developer_hotkeys":[22,23,24,25,26,27,28,1,2]}"#).unwrap();
        let mut prefs = load_from(&path).unwrap();
        assert_eq!(
            prefs.developer_hotkeys,
            vec![22, 23, 24, 25, 26, 27, 28, 1, 2, 0]
        );
        prefs.developer_hotkeys[9] = 3;
        save_to(&path, &prefs).unwrap();
        assert_eq!(
            load_from(&path).unwrap().developer_hotkeys,
            vec![22, 23, 24, 25, 26, 27, 28, 1, 2, 3]
        );
    }
    #[test]
    fn legacy_theme_is_optional_and_restores_previous_modern_customizations() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        fs::write(
            &file,
            r##"{"appearance_mode":"Light","appearance_colors":{"accent":"#123456"}}"##,
        )
        .unwrap();
        let mut settings = load_from(&file).unwrap();
        assert_eq!(settings.ui_theme, "Modern");
        assert_eq!(settings.appearance_mode, "Light");
        settings.ui_theme = "Evolve Legacy".into();
        save_to(&file, &settings).unwrap();
        let mut restored = load_from(&file).unwrap();
        assert_eq!(restored.ui_theme, "Evolve Legacy");
        assert_eq!(restored.appearance_colors["accent"], "#123456");
        restored.ui_theme = "Modern".into();
        assert_eq!(restored.appearance_mode, "Light");
        restored.ui_theme = "invalid theme".into();
        restored.validate().unwrap();
        assert_eq!(restored.ui_theme, "Modern");
    }

    #[test]
    fn test_environment_paths_and_choice_roundtrip_without_changing_production() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = Preferences::default();
        assert!(!original.play_through_test_client);
        let mut prefs = original.clone();
        prefs.test_game_installation = "C:/EvolveTest/Game".into();
        prefs.test_client_executable = "C:/EvolveTest/TestClient.exe".into();
        prefs.test_stage = "C:/EvolveTest/Stage".into();
        prefs.test_swap = "C:/EvolveTest/Swap".into();
        prefs.play_through_test_client = true;
        save_to(&path, &prefs).unwrap();
        let restored = load_from(&path).unwrap();
        assert!(restored.play_through_test_client);
        assert_eq!(restored.test_game_installation, "C:/EvolveTest/Game");
        assert_eq!(
            restored.test_client_executable,
            "C:/EvolveTest/TestClient.exe"
        );
        assert_eq!(restored.test_stage, "C:/EvolveTest/Stage");
        assert_eq!(restored.test_swap, "C:/EvolveTest/Swap");
        assert_eq!(restored.game_installation, original.game_installation);
        assert_eq!(restored.client_executable, original.client_executable);
    }

    #[test]
    fn accent_presets_change_only_accent_roles_and_are_valid() {
        let mut prefs = Preferences::default();
        prefs
            .appearance_colors
            .insert("surface".into(), "#15171c".into());
        assert!(apply_accent_preset(&mut prefs, "Deep Red"));
        assert_eq!(
            prefs.appearance_colors.get("accent").map(String::as_str),
            Some("#a91f38")
        );
        assert_eq!(
            prefs.appearance_colors.get("surface").map(String::as_str),
            Some("#15171c")
        );
        assert!(prefs.validate().is_ok());
        assert!(!apply_accent_preset(&mut prefs, "Not a preset"));
        assert!(apply_accent_preset(&mut prefs, "Default"));
        assert!(!prefs.appearance_colors.contains_key("accent"));
        assert_eq!(
            prefs.appearance_colors.get("surface").map(String::as_str),
            Some("#15171c")
        );
    }

    #[test]
    fn full_client_defaults_without_overwriting_explicit_direct_choice() {
        assert_eq!(Preferences::default().game_launch_mode, "Client Launch");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        fs::write(&file, r#"{"ui_density":"Compact"}"#).unwrap();
        assert_eq!(load_from(&file).unwrap().game_launch_mode, "Client Launch");
        let mut p = Preferences {
            game_launch_mode: "Direct Launch".into(),
            ..Preferences::default()
        };
        save_to(&file, &p).unwrap();
        assert_eq!(load_from(&file).unwrap().game_launch_mode, "Direct Launch");
        p.game_launch_mode = "invalid".into();
        p.validate().unwrap();
        assert_eq!(p.game_launch_mode, "Client Launch");
    }

    #[test]
    fn viewer_explorer_preference_roundtrips_and_legacy_defaults_visible() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let prefs = Preferences {
            file_explorer_side: "Left".into(),
            viewer_explorer_visible: false,
            ..Preferences::default()
        };
        save_to(&file, &prefs).unwrap();
        let saved = load_from(&file).unwrap();
        assert_eq!(saved.file_explorer_side, "Left");
        assert!(!saved.viewer_explorer_visible);

        fs::write(
            &file,
            r#"{"file_explorer_side":"Right","viewer_split":0.65}"#,
        )
        .unwrap();
        assert!(load_from(&file).unwrap().viewer_explorer_visible);
    }

    #[test]
    fn detects_existing_manager_library_and_gamesets_without_user_input() {
        let home = tempfile::tempdir().unwrap();
        let settings = home.path().join("settings.json");
        let library = home.path().join("Data").join("AssetLibrary");
        let games = home.path().join("Data").join("GameSets");
        assert_eq!(library_directory(&settings, "").unwrap(), library);
        assert_eq!(game_storage_directory(&settings, "").unwrap(), games);
        assert_eq!(
            library_directory(&settings, "D:/MyLibrary").unwrap(),
            PathBuf::from("D:/MyLibrary")
        );
        assert_eq!(
            game_storage_directory(&settings, "D:/NewGameSets").unwrap(),
            PathBuf::from("D:/NewGameSets")
        );
        assert!(
            !library.exists(),
            "Discovery must not silently create or modify source data"
        );
    }

    #[test]
    fn fresh_setup_uses_public_manager_gamesets_without_private_paths() {
        let home = tempfile::tempdir().unwrap();
        let settings = home.path().join("settings.json");
        let expected = home.path().join("Data").join("GameSets");
        for input in ["", "   ", "GameSets", "Data/GameSets", "data\\gamesets"] {
            assert_eq!(setup_storage_directory(&settings, input).unwrap(), expected);
        }
        assert!(
            !expected.exists(),
            "Resolving a default must not touch disk"
        );
        let chosen = home.path().join("OtherLocation");
        assert_eq!(
            setup_storage_directory(&settings, chosen.to_str().unwrap()).unwrap(),
            chosen
        );
        assert!(setup_storage_directory(&settings, "../GameSets").is_err());
        assert!(setup_storage_directory(&settings, "folder/GameSets").is_err());
        assert!(setup_storage_directory(&settings, "GameSets/../elsewhere").is_err());
    }

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
            prefs.nightly_updates,
            env!("CARGO_PKG_VERSION").contains("-nightly.")
        );
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
    #[test]
    fn home_background_setting_defaults_on_and_survives_roundtrip() {
        let old: Preferences = serde_json::from_str(r#"{"ui_theme":"Modern"}"#).unwrap();
        assert!(old.show_home_background);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let mut prefs = old;
        prefs.show_home_background = false;
        save_to(&file, &prefs).unwrap();
        let loaded = load_from(&file).unwrap();
        assert!(!loaded.show_home_background);
        assert_eq!(loaded.ui_theme, "Modern");
    }

    #[test]
    fn crash_diagnostics_default_enabled_even_with_old_preferences() {
        let json = r#"{"show_activity_on_tasks":false}"#;
        let prefs: Preferences = serde_json::from_str(json).unwrap();
        assert!(prefs.crash_logging_enabled);
        let mut off = prefs;
        off.crash_logging_enabled = false;
        let saved = serde_json::to_string(&off).unwrap();
        let restored: Preferences = serde_json::from_str(&saved).unwrap();
        assert!(!restored.crash_logging_enabled);
    }
}
