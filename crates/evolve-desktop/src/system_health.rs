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
fn system32_executable(name: &str) -> Result<std::path::PathBuf, String> {
    crate::windows_system32::executable(name)
}

#[cfg(windows)]
fn bounded_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .take(12_000)
        .collect()
}

#[cfg(windows)]
fn read_command(program: &Path, arguments: &[&str]) -> String {
    match Command::new(program).args(arguments).output() {
        Ok(result) if result.status.success() => bounded_text(&result.stdout),
        Ok(result) => format!("Query unavailable (exit {})", result.status),
        Err(error) => format!("Query unavailable: {error}"),
    }
}

/// Only the manager-owned rule is ever queried, always filtered by its
/// derived name; global or third-party firewall rules are never requested.
#[cfg(windows)]
fn query_manager_rule(name: &str) -> Result<(bool, String), String> {
    let program = system32_executable("netsh.exe")?;
    let filter = format!("name={name}");
    let output = Command::new(&program)
        .args(["advfirewall", "firewall", "show", "rule", &filter])
        .output()
        .map_err(|error| format!("Cannot query Windows Firewall: {error}"))?;
    let mut text = bounded_text(&output.stdout);
    if !output.status.success() {
        text.push(' ');
        text.push_str(&bounded_text(&output.stderr));
    }
    Ok((output.status.success(), text))
}

/// Rule name derived from the selected game root, matching what
/// `online_guard::apply` creates. Empty paths are not queried at all.
fn manager_rule_name(game_path: &str) -> Option<String> {
    if game_path.trim().is_empty() {
        return None;
    }
    let root = Path::new(game_path);
    // Match the canonical executable used by apply; if it is absent, show
    // the prospective rule for the configured root without changing policy.
    let executable = crate::online_guard::game_executable(root).unwrap_or_else(|_| {
        let resolved = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        crate::online_guard::game_executable_path(&resolved)
    });
    Some(crate::online_guard::named_rule(&executable))
}

fn firewall_summary(game_path: &str) -> String {
    let Some(name) = manager_rule_name(game_path) else {
        return "Manager offline firewall rule not checked: no game folder selected".into();
    };
    #[cfg(windows)]
    {
        let (success, output) = match query_manager_rule(&name) {
            Ok(result) => result,
            Err(error) => return format!("Query unavailable: {error}"),
        };
        let lower = output.to_ascii_lowercase();
        if success && lower.contains(&name.to_ascii_lowercase()) {
            format!("Manager-owned offline firewall rule {name} is present")
        } else if success || lower.contains("no rules") {
            format!("Manager-owned offline firewall rule {name} not found")
        } else {
            format!(
                "Firewall query did not report manager-owned rule {name}; review Windows Firewall for exact policy"
            )
        }
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        "Windows Firewall diagnostics require Windows".into()
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
        let running = system32_executable("tasklist.exe")
            .map(|program| read_command(&program, &["/FO", "CSV", "/NH"]))
            .unwrap_or_else(|error| format!("Query unavailable: {error}"));
        let game_process = if running.starts_with("Query unavailable") {
            running
        } else if running.to_ascii_lowercase().contains("\"evolve.exe\"") {
            "Evolve.exe detected in current process list".into()
        } else {
            "Evolve.exe not detected in current process list".into()
        };
        // Only query the manager-owned rule; never create/remove/reconfigure.
        let firewall = firewall_summary(game_path);
        Snapshot {
            firewall,
            game_process,
            environment,
            recovery,
            dumps: "WER crash dump settings are not modified or enumerated by the Rust preview."
                .into(),
            storage: "Disk free space and orphaned-swap cleanup are not yet implemented.".into(),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("true");
        Snapshot {
            firewall: firewall_summary(game_path),
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
    #[test]
    fn firewall_query_uses_the_manager_derived_rule_name() {
        let root = Path::new(r"C:\Games\ModdedEvolve\EvolveGame");
        let expected =
            crate::online_guard::named_rule(&crate::online_guard::game_executable_path(root));
        let actual = manager_rule_name(&root.to_string_lossy()).expect("configured game root");
        assert_eq!(actual, expected);
        assert!(actual.starts_with("EvolveModManager-Offline-"));
        assert_ne!(actual, "Evolve Mod Manager Offline");
    }
    #[test]
    fn missing_game_path_skips_the_firewall_query() {
        assert_eq!(manager_rule_name(""), None);
        assert_eq!(manager_rule_name("   "), None);
        assert!(firewall_summary("").contains("no game folder selected"));
        let missing = std::env::temp_dir().join("evolve-health-missing-game-root");
        assert!(manager_rule_name(&missing.to_string_lossy()).is_some());
    }
    #[cfg(windows)]
    #[test]
    fn system32_queries_use_absolute_executable_paths() {
        for name in ["tasklist.exe", "netsh.exe"] {
            let path = system32_executable(name).expect("Windows system tool");
            assert!(path.is_absolute());
            assert_eq!(path.file_name().and_then(|part| part.to_str()), Some(name));
            assert!(
                path.parent()
                    .and_then(Path::file_name)
                    .and_then(|part| part.to_str())
                    .is_some_and(|part| part.eq_ignore_ascii_case("System32"))
            );
        }
    }
}
