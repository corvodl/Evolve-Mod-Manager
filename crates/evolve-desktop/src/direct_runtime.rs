//! Stage 2 services provenance preflight for manager-owned game creation.
//! Real readiness is established by the isolated Python runtime supervisor,
//! NOT by open TCP ports alone. Fail before touching original PAKs when the
//! required private client compatibility components are unavailable.
use std::path::Path;

pub fn require_ready(game: &Path, client: &Path) -> Result<(), String> {
    let game = game.join("bin64_SteamRetail").join("Evolve.exe");
    if !game.is_file() {
        return Err("Installed Evolve.exe is missing; no PAK files modified".into());
    }
    if !client.file_name().is_some_and(|name| name.eq_ignore_ascii_case("ModdedEvolveLauncher.exe"))
        || !client.is_file()
    {
        return Err("Direct Launch needs an installed ModdedEvolveLauncher.exe for profile and SteamCompat service hosting. Choose Client Launch if unavailable; no PAK files modified.".into());
    }
    let root = client.parent().ok_or("The client executable has no containing folder")?;
    for name in [
        "ModdedEvolveLauncher.dll",
        "Compat/ModdedEvolveSteamCompat.dll",
        "Compat/Stage2/Stage2Trace.dll",
    ] {
        if !root.join(name).is_file() {
            return Err(format!("Direct Launch needs the client runtime file {name}; no PAK files modified"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_services_prevent_game_start() {
        let tmp = tempfile::tempdir().unwrap();
        let err = require_ready(tmp.path(), &tmp.path().join("ModdedEvolveLauncher.exe")).unwrap_err();
        assert!(err.contains("Evolve.exe"));
    }
    #[test]
    fn full_service_layout_is_accepted_for_real_runtime_verification() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("game/bin64_SteamRetail");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("Evolve.exe"), b"test").unwrap();
        let client = tmp.path().join("client");
        std::fs::create_dir_all(client.join("Compat/Stage2")).unwrap();
        for name in [
            "ModdedEvolveLauncher.exe",
            "ModdedEvolveLauncher.dll",
            "Compat/ModdedEvolveSteamCompat.dll",
            "Compat/Stage2/Stage2Trace.dll",
        ] {
            std::fs::write(client.join(name), b"test").unwrap();
        }
        require_ready(&tmp.path().join("game"), &client.join("ModdedEvolveLauncher.exe")).unwrap();
    }
}
