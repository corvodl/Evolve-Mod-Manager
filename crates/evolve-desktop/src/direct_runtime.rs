//! Filesystem-only preflight for experimental Direct Launch.
//!
//! Actual service readiness and Stage2Trace compatibility must be evaluated
//! by the running Windows service supervisor. Never reject a supported setup
//! merely because the manager has not already run an end-to-end gameplay test.
use std::path::Path;

/// Check only the essential local launch inputs. This deliberately does not
/// start a service or modify any PAK. Detailed diagnostics are reported by the
/// direct launch worker while the user is exercising a real game session.
pub fn require_ready(game: &Path, client: &Path) -> Result<(), String> {
    let binary = game.join("bin64_SteamRetail").join("Evolve.exe");
    if !binary.is_file() {
        return Err(format!(
            "Direct Launch: game executable not found at {}. No PAK files changed.",
            binary.display()
        ));
    }
    if client
        .file_name()
        .is_none_or(|name| !name.eq_ignore_ascii_case("ModdedEvolveLauncher.exe"))
        || !client.is_file()
    {
        return Err(
            "Direct Launch needs the installed ModdedEvolveLauncher.exe to start its profile and Steam compatibility services. No PAK files changed.".into()
        );
    }
    let root = client.parent().ok_or("Client executable has no parent directory")?;
    for name in [
        "ModdedEvolveLauncher.dll",
        "Compat/ModdedEvolveSteamCompat.dll",
    ] {
        let file = root.join(name);
        if !file.is_file() {
            return Err(format!(
                "Direct Launch: missing client runtime component {}. No PAK files changed.",
                file.display()
            ));
        }
    }
    // The actual selected Stage2Trace version, the game-local Steam provider,
    // the ownable catalog and the live service state are checked by the Python
    // runtime worker. They are not a global switch that disables Direct Launch.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_game_reports_an_actionable_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = require_ready(tmp.path(), tmp.path()).unwrap_err();
        assert!(err.contains("Evolve.exe"));
        assert!(err.contains("No PAK files changed"));
    }

    #[test]
    fn valid_files_allow_live_direct_launch_testing() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("game").join("bin64_SteamRetail");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("Evolve.exe"), b"test").unwrap();
        let client_dir = tmp.path().join("client");
        std::fs::create_dir_all(client_dir.join("Compat")).unwrap();
        for name in [
            "ModdedEvolveLauncher.exe",
            "ModdedEvolveLauncher.dll",
            "Compat/ModdedEvolveSteamCompat.dll",
        ] {
            std::fs::write(client_dir.join(name), b"test").unwrap();
        }
        assert!(require_ready(
            &tmp.path().join("game"),
            &client_dir.join("ModdedEvolveLauncher.exe")
        ).is_ok());
    }
}
