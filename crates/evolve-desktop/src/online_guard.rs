//! Restore online networking by removing only Evolve Mod Manager's own
//! per-game outbound firewall rule (including rules from the private manager).
//! Never touches game authentication, remote servers or third-party firewall rules.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const PREFIX: &str = "EvolveModManager-Offline-";

fn win_path_text(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    if let Some(tail) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{tail}")
    } else if let Some(tail) = value.strip_prefix(r"\\?\") {
        tail.to_owned()
    } else {
        value
    }
}

pub(crate) fn game_executable_path(game_root: &Path) -> PathBuf {
    game_root.join("bin64_SteamRetail").join("Evolve.exe")
}

pub(crate) fn named_rule(executable: &Path) -> String {
    // Private Python manager: sha256(str(Path(exe).resolve()).casefold())[:20].
    // On ordinary Windows game paths, lowercase is equivalent to casefold.
    let canonical = win_path_text(executable).to_lowercase();
    let hash = Sha256::digest(canonical.as_bytes());
    let mut label = String::from(PREFIX);
    for byte in &hash[..10] {
        use std::fmt::Write;
        let _ = write!(label, "{byte:02x}");
    }
    label
}

pub fn game_executable(game_root: &Path) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(game_root)
        .map_err(|e| format!("Cannot locate selected EvolveGame folder: {e}"))?;
    let game = game_executable_path(&root);
    let meta =
        std::fs::symlink_metadata(&game).map_err(|e| format!("Cannot locate Evolve.exe: {e}"))?;
    if !meta.file_type().is_file() {
        return Err("Evolve.exe is not a regular file".into());
    }
    std::fs::canonicalize(game).map_err(|e| e.to_string())
}

fn manager_rule_delete_args(name: &str, executable: &Path) -> Vec<String> {
    // Name alone is not sufficient: netsh deletes *all* matching rules,
    // including another program's rule with the same display name.
    // Restrict deletion to outbound rules belonging to this Evolve executable.
    vec![
        "delete".into(),
        "rule".into(),
        format!("name={name}"),
        "dir=out".into(),
        format!("program={}", win_path_text(executable)),
    ]
}

#[cfg(windows)]
pub fn apply(game_root: &Path, allow_online: bool) -> Result<String, String> {
    use std::collections::HashSet;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Output, Stdio};

    fn command(args: &[String]) -> Result<Output, String> {
        let root = std::env::var_os("WINDIR")
            .or_else(|| std::env::var_os("SystemRoot"))
            .unwrap_or_else(|| r"C:\Windows".into());
        Command::new(PathBuf::from(root).join("System32").join("netsh.exe"))
            .args(["advfirewall", "firewall"])
            .args(args)
            .stdin(Stdio::null())
            .creation_flags(0x08000000)
            .output()
            .map_err(|e| format!("Cannot query Windows Firewall: {e}"))
    }
    fn exists(name: &str) -> Result<bool, String> {
        let out = command(&["show".into(), "rule".into(), format!("name={name}")])?;
        let text = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
        if text.contains(&name.to_ascii_lowercase()) {
            if !out.status.success() {
                return Err("Could not verify the manager-owned firewall rule".into());
            }
            return Ok(true);
        }
        // Never interpret a failed query as proof that the rule is absent.
        // A permissions or firewall-service failure must not silently turn
        // online policy into an apparently successful operation.
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            return Err(format!(
                "Cannot verify manager firewall rule {name} (exit {}). {} {}",
                out.status,
                stderr.chars().take(240).collect::<String>(),
                stdout.chars().take(240).collect::<String>()
            ));
        }
        // A successful query that did not return this named rule is absent.
        Ok(false)
    }
    fn run(args: &[String], verb: &str) -> Result<(), String> {
        let out = command(args)?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        Err(format!(
            "Windows Firewall {verb} failed. Administrator permission may be required. {} {}",
            stderr.chars().take(320).collect::<String>(),
            stdout.chars().take(320).collect::<String>(),
        ))
    }

    let exe = game_executable(game_root)?;
    let mut seen = HashSet::new();
    let mut rules = Vec::new();
    for candidate in [&exe, &game_executable_path(game_root)] {
        let label = named_rule(candidate);
        if seen.insert(label.clone()) {
            rules.push((label, candidate.to_path_buf()));
        }
    }
    let mut removed = 0;
    for (name, executable) in &rules {
        if exists(name)? {
            run(
                &manager_rule_delete_args(name, executable),
                "rule removal",
            )?;
            if exists(name)? {
                return Err(format!(
                    "Firewall rule {name} remains active; online mode was NOT enabled."
                ));
            }
            removed += 1;
        }
    }
    if allow_online {
        return Ok(if removed == 0 {
            "No manager-owned Evolve outbound block remains. Online connectivity is now permitted by this manager; server acceptance is separate.".into()
        } else {
            format!(
                "Removed {removed} Evolve Mod Manager offline firewall rule(s). Online connectivity is permitted by this manager; server acceptance is separate."
            )
        });
    }
    let name = named_rule(&exe);
    run(
        &[
            "add".into(),
            "rule".into(),
            format!("name={name}"),
            "dir=out".into(),
            "action=block".into(),
            format!("program={}", win_path_text(&exe)),
            "enable=yes".into(),
            "profile=any".into(),
        ],
        "offline isolation",
    )?;
    if !exists(&name)? {
        return Err("Could not verify the Evolve outbound offline firewall rule".into());
    }
    Ok("Evolve outbound traffic blocked for modded offline testing".into())
}

#[cfg(not(windows))]
pub fn apply(_game_root: &Path, _allow_online: bool) -> Result<String, String> {
    Err("Windows Firewall integration requires Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_rule_id_matches_extended_prefix_and_path_casing() {
        let ordinary = Path::new(r"C:\Games\ModdedEvolve\EvolveGame\bin64_SteamRetail\Evolve.exe");
        let prefixed =
            Path::new(r"\\?\C:\Games\ModdedEvolve\EvolveGame\bin64_SteamRetail\Evolve.exe");
        let lowercase = Path::new(r"c:\games\moddedevolve\evolvegame\bin64_steamretail\evolve.exe");
        assert_eq!(named_rule(ordinary), named_rule(prefixed));
        assert_eq!(named_rule(ordinary), named_rule(lowercase));
        assert_eq!(named_rule(ordinary).len(), PREFIX.len() + 20);
    }
    #[test]
    fn different_game_paths_are_distinct_firewall_rules() {
        assert_ne!(
            named_rule(Path::new(r"C:\A\Evolve.exe")),
            named_rule(Path::new(r"C:\B\Evolve.exe"))
        );
    }
    #[test]
    fn derived_game_executable_shares_the_game_root_rule_name() {
        let root = Path::new(r"C:\Games\ModdedEvolve\EvolveGame");
        let expected = Path::new(r"C:\Games\ModdedEvolve\EvolveGame\bin64_SteamRetail\Evolve.exe");
        assert_eq!(
            named_rule(&game_executable_path(root)),
            named_rule(expected)
        );
    }
    #[test]
    fn firewall_deletion_is_scoped_to_selected_game_executable_and_outbound_rules() {
        let exe = Path::new(r"C:\Games\ModdedEvolve\EvolveGame\bin64_SteamRetail\Evolve.exe");
        let rule = named_rule(exe);
        let args = manager_rule_delete_args(&rule, exe);
        assert_eq!(
            args,
            [
                "delete",
                "rule",
                &format!("name={rule}"),
                "dir=out",
                &format!("program={}", win_path_text(exe)),
            ]
        );
        assert!(!args.iter().any(|arg| arg == "name=all"));
    }
}
