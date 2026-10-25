//! Read the installed Modded Evolve Client launcher's version metadata.
//! This is deliberately independent from the Evolve Mod Manager Cargo version.
//! Version inspection never runs the launcher or modifies the installation.
use std::path::Path;

/// Accept ordinary numeric client versions, including four-part Windows file
/// versions and optional prerelease suffixes. Do not claim a build version
/// from an empty or arbitrary ProductVersion string.
fn normalize_version(input: &str) -> Option<String> {
    let token = input
        .trim()
        .split_whitespace()
        .next()?
        .trim_start_matches(['v', 'V']);
    if token.is_empty() || token.len() > 64 {
        return None;
    }
    if !token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
    {
        return None;
    }
    let numeric = token.split(['-', '+']).next()?;
    let parts: Vec<_> = numeric.split('.').collect();
    if !(2..=4).contains(&parts.len())
        || !parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some(token.to_owned())
}

/// Return the client version, not the manager or CryEngine version.
/// The launcher path is the one verified/chosen under Settings → Storage.
#[cfg(windows)]
pub fn installed_version(path: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let file = Path::new(path);
    if !file.is_file() || !is_launcher(file) {
        return None;
    }
    // -LiteralPath and an environment variable avoid treating user-provided
    // filenames as PowerShell commands. CREATE_NO_WINDOW avoids CMD flashes.
    // Only the PE version resource is read; the client is never executed.
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$info = (Get-Item -LiteralPath $env:EVOLVE_CLIENT_VERSION_PATH -ErrorAction Stop).VersionInfo
foreach ($version in @($info.ProductVersion, $info.FileVersion)) {
    if ($version -and $version.Trim() -match '^v?\d+\.\d+') {
        [Console]::WriteLine($version.Trim())
        exit 0
    }
}
exit 1
"#;
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("EVOLVE_CLIENT_VERSION_PATH", file)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 1024 {
        return None;
    }
    normalize_version(std::str::from_utf8(&output.stdout).ok()?)
}

#[cfg(not(windows))]
pub fn installed_version(_path: &str) -> Option<String> {
    None
}

fn is_launcher(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        name.to_string_lossy()
            .eq_ignore_ascii_case("ModdedEvolveLauncher.exe")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_confuse_product_names_or_manager_versions_with_client_version() {
        assert_eq!(normalize_version("v0.1.87"), Some("0.1.87".into()));
        assert_eq!(normalize_version("0.1.80.0"), Some("0.1.80.0".into()));
        assert_eq!(
            normalize_version("0.1.87-beta.2"),
            Some("0.1.87-beta.2".into())
        );
        assert_eq!(normalize_version("Modded Evolve Client"), None);
        assert_eq!(normalize_version(""), None);
        assert_eq!(normalize_version("1"), None);
    }

    #[test]
    fn only_configured_client_launcher_is_supported() {
        assert!(is_launcher(Path::new("ModdedEvolveLauncher.exe")));
        assert!(!is_launcher(Path::new(
            r"C:\Games\Modded Evolve\Evolve.exe"
        )));
        assert!(!is_launcher(Path::new("evolve-mod-manager.exe")));
    }
}
