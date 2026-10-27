//! Launch the real Modded Evolve client in the interactive user's medium-
//! integrity Windows session instead of inheriting this manager's admin token.
//! The Windows Win32 token boundary lives in a separately checked Python 3.11
//! helper; this Rust crate preserves the workspace-wide ban on unsafe code.
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;
#[cfg(windows)]
use std::process::{Command, Stdio};

const BROKER_SCRIPT: &str = include_str!("../launcher_support/standard_user_launch.py");

#[cfg(any(windows, test))]
fn parse_launch_report(stdout: &str) -> Result<u32, String> {
    let text = stdout.trim();
    let prefix = "STANDARD_USER_LAUNCH pid=";
    let line = text
        .lines()
        .find(|line| line.starts_with(prefix))
        .ok_or("Standard-user launcher did not return a verified process ID")?;
    if !line.contains(" elevated=false integrity=medium ") {
        return Err(
            "Client token verification did not confirm medium-integrity standard user".into(),
        );
    }
    let token = line[prefix.len()..]
        .split_whitespace()
        .next()
        .ok_or("Missing client PID")?;
    let pid: u32 = token.parse().map_err(|_| "Invalid client PID")?;
    if pid == 0 {
        return Err("Client PID was zero".into());
    }
    Ok(pid)
}

#[cfg(windows)]
fn call_broker(args: &[&std::ffi::OsStr]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let mut temp = tempfile::Builder::new()
        .prefix("evolve-standard-user-")
        .suffix(".py")
        .tempfile()
        .map_err(|e| format!("Could not create standard-user launcher: {e}"))?;
    temp.write_all(BROKER_SCRIPT.as_bytes())
        .and_then(|_| temp.as_file_mut().sync_all())
        .map_err(|e| format!("Could not write standard-user launcher: {e}"))?;
    let out = Command::new("py")
        .args(["-3.11", "-u"])
        .arg(temp.path())
        .args(args)
        .stdin(Stdio::null())
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| format!("Python 3.11 x64 is required for standard-user launch: {e}"))?;
    if out.stdout.len() > 32768 || out.stderr.len() > 32768 {
        return Err("Privilege helper returned excessive output".into());
    }
    if !out.status.success() {
        return Err(format!(
            "Standard-user launch failed: {}. Check that Explorer is running normally and launch the manager from the same Windows desktop session.",
            String::from_utf8_lossy(&out.stderr)
                .trim()
                .chars()
                .take(900)
                .collect::<String>()
        ));
    }
    String::from_utf8(out.stdout).map_err(|_| "Invalid privilege helper output".into())
}

#[cfg(windows)]
pub fn launch(executable: &Path) -> Result<u32, String> {
    let file = std::fs::canonicalize(executable)
        .map_err(|e| format!("Configured client executable is missing: {e}"))?;
    if !file
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("ModdedEvolveLauncher.exe"))
    {
        return Err("Only a configured ModdedEvolveLauncher.exe may be launched".into());
    }
    let report = call_broker(&[std::ffi::OsStr::new("--launch"), file.as_os_str()])?;
    parse_launch_report(&report)
}

#[cfg(not(windows))]
pub fn launch(_executable: &Path) -> Result<u32, String> {
    Err("Standard-user Evolve launch requires Windows".into())
}

#[cfg(windows)]
pub fn inspect_privileges() -> Result<String, String> {
    call_broker(&[std::ffi::OsStr::new("--diagnose-processes")])
}

#[cfg(not(windows))]
pub fn inspect_privileges() -> Result<String, String> {
    Err("Process token diagnostics require Windows".into())
}

/// Publish the exact source-reviewed helper into the existing private swap
/// directory. Reject foreign scripts; never run substituted Python code.
pub fn install_into_swap(swap: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(swap).map_err(|e| e.to_string())?;
    if !meta.file_type().is_dir() {
        return Err("Swap helper folder must be a normal directory".into());
    }
    let target = swap.join("standard_user_launch.py");
    match std::fs::symlink_metadata(&target) {
        Ok(meta) => {
            if !meta.file_type().is_file() || meta.len() > 128 * 1024 {
                return Err("Existing standard-user launch helper is not an ordinary file".into());
            }
            let existing = std::fs::read(&target).map_err(|e| e.to_string())?;
            if existing == BROKER_SCRIPT.as_bytes() {
                return Ok(());
            }
            // Preserve unrecognized and older helpers byte-for-byte, but
            // execute only the vetted script embedded in this release.
            let digest = format!("{:x}", Sha256::digest(&existing));
            let preserved = swap.join(format!(
                "standard_user_launch.py.previous-{}",
                &digest[..16]
            ));
            match std::fs::symlink_metadata(&preserved) {
                Ok(meta) => {
                    if !meta.file_type().is_file()
                        || std::fs::read(&preserved).map_err(|e| e.to_string())? != existing
                    {
                        return Err(
                            "Conflicting preserved standard-user helper; no files replaced".into(),
                        );
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(format!("Cannot inspect previous helper: {error}")),
            }
            let mut scratch = tempfile::Builder::new()
                .prefix(".evolve-standard-user-")
                .tempfile_in(swap)
                .map_err(|e| e.to_string())?;
            scratch
                .write_all(BROKER_SCRIPT.as_bytes())
                .map_err(|e| e.to_string())?;
            scratch
                .as_file_mut()
                .sync_all()
                .map_err(|e| e.to_string())?;
            if preserved.exists() {
                std::fs::remove_file(&target).map_err(|e| e.to_string())?;
            } else {
                std::fs::rename(&target, &preserved)
                    .map_err(|e| format!("Could not preserve existing helper: {e}"))?;
            }
            scratch
                .persist_noclobber(&target)
                .map_err(|e| format!("Could not publish vetted helper: {}", e.error))?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut scratch = tempfile::Builder::new()
                .prefix(".evolve-standard-user-")
                .tempfile_in(swap)
                .map_err(|e| e.to_string())?;
            scratch
                .write_all(BROKER_SCRIPT.as_bytes())
                .map_err(|e| e.to_string())?;
            scratch
                .as_file_mut()
                .sync_all()
                .map_err(|e| e.to_string())?;
            scratch
                .persist_noclobber(&target)
                .map_err(|e| format!("Cannot publish trusted standard-user helper: {}", e.error))?;
            Ok(())
        }
        Err(error) => Err(format!("Cannot inspect standard-user helper: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_requires_verified_medium_token_and_real_pid() {
        assert_eq!(
            parse_launch_report(
                "STANDARD_USER_LAUNCH pid=345 elevated=false integrity=medium session=1"
            )
            .unwrap(),
            345
        );
        assert!(
            parse_launch_report(
                "STANDARD_USER_LAUNCH pid=345 elevated=true integrity=high session=1"
            )
            .is_err()
        );
        assert!(
            parse_launch_report(
                "STANDARD_USER_LAUNCH pid=0 elevated=false integrity=medium session=1"
            )
            .is_err()
        );
        assert!(parse_launch_report("Launching 345").is_err());
    }

    #[test]
    fn private_helper_install_preserves_previous_bytes_and_uses_vetted_script() {
        let root = tempfile::tempdir().unwrap();
        install_into_swap(root.path()).unwrap();
        install_into_swap(root.path()).unwrap();
        let path = root.path().join("standard_user_launch.py");
        std::fs::write(&path, b"unknown script").unwrap();
        install_into_swap(root.path()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), BROKER_SCRIPT.as_bytes());
        let old_sha = format!("{:x}", Sha256::digest(b"unknown script"));
        let preserved = root.path().join(format!(
            "standard_user_launch.py.previous-{}",
            &old_sha[..16]
        ));
        assert_eq!(std::fs::read(&preserved).unwrap(), b"unknown script");
        install_into_swap(root.path()).unwrap();
        std::fs::write(&path, b"unknown script").unwrap();
        std::fs::write(&preserved, b"tampered backup").unwrap();
        assert!(install_into_swap(root.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"unknown script");
    }
}
