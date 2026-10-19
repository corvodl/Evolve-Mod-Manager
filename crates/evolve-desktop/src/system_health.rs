//! Opt-in, read-only Windows diagnostics for the Home disclosure panel.
//! Does not change the firewall, registry, WER, game files or backup state.
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub firewall: String,
    pub game_process: String,
    pub storage: String,
    pub dumps: String,
    pub environment: String,
    pub recovery: String,
}

fn path_state(path: &Path, label: &str) -> String {
    if path.as_os_str().is_empty() {
        return format!("{label}: not configured");
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_dir() => format!("{label}: directory exists"),
        Ok(meta) if meta.file_type().is_file() => format!("{label}: file exists"),
        Ok(_) => format!("{label}: unsupported/linked path"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            format!("{label}: not found")
        }
        Err(error) => format!("{label}: cannot inspect ({error})"),
    }
}

#[cfg(windows)]
fn read_command(name: &str, arguments: &[&str]) -> String {
    match Command::new(name).args(arguments).output() {
        Ok(result) if result.status.success() => String::from_utf8_lossy(&result.stdout)
            .chars()
            .take(12_000)
            .collect(),
        Ok(result) => format!("Query unavailable (exit {})", result.status),
        Err(error) => format!("Query unavailable: {error}"),
    }
}

pub fn inspect(game_path: &str, stage_path: &str, projects_path: &str) -> Snapshot {
    let root = Path::new(game_path);
    let stage = Path::new(stage_path);
    let projects = Path::new(projects_path);
    let game_file = root.join("bin64_SteamRetail").join("Evolve.exe");
    let environment = format!(
        "{}; {}",
        path_state(root, "Game root"),
        path_state(&game_file, "Stage 2 executable"),
    );
    let recovery = format!(
        "{}; {}",
        path_state(stage, "Signing stage"),
        path_state(projects, "Projects"),
    );
    #[cfg(windows)]
    {
        let running = read_command("tasklist", &["/FO", "CSV", "/NH"]);
        let game_process = if running.starts_with("Query unavailable") {
            running
        } else if running.to_ascii_lowercase().contains("\"evolve.exe\"") {
            "Evolve.exe detected in current process list".into()
        } else {
            "Evolve.exe not detected in current process list".into()
        };
        // Only query the manager-owned rule; never create/remove/reconfigure.
        let firewall_status = read_command(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "show",
                "rule",
                "name=Evolve Mod Manager Offline",
            ],
        );
        let firewall = if firewall_status.starts_with("Query unavailable") {
            firewall_status
        } else if firewall_status
            .to_ascii_lowercase()
            .contains("no rules match")
            || firewall_status.to_ascii_lowercase().contains("no rules")
        {
            "Manager offline firewall rule not found".into()
        } else {
            "Firewall query returned rule information; review Windows Firewall for exact policy"
                .into()
        };
        Snapshot {
            firewall,
            game_process,
            environment,
            recovery,
            dumps: "WER crash dump settings are not modified or enumerated by the Rust preview."
                .into(),
            storage: "Disk free space and orphaned-swap cleanup are not yet implemented.".into(),
        };
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("true");
        Snapshot {
            firewall: "Windows Firewall diagnostics require Windows".into(),
            game_process: "Windows process diagnostics require Windows".into(),
            environment,
            recovery,
            dumps: "Windows WER diagnostics require Windows".into(),
            storage: "Disk space and backup cleanup are not yet implemented".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_paths_remain_read_only_and_report_not_configured() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("not-installed");
        let snapshot = inspect(&game.to_string_lossy(), "", "");
        assert!(snapshot.environment.contains("not found"));
        assert!(snapshot.recovery.contains("not configured"));
        assert!(!game.exists());
    }
}
