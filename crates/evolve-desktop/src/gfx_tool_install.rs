//! Explicit, on-demand installation of pinned GFX tools from official releases.
//! All downloads are SHA-256 verified and extracted into a scratch directory
//! under the application's third_party folder. No untrusted ZIP paths,
//! symlinks, overwrites, or game assets are accepted.
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_ARCHIVE: u64 = 55 * 1024 * 1024;
const MAX_EXTRACTED: u64 = 220 * 1024 * 1024;
const MAX_MEMBERS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Ffdec,
    Ruffle,
}
impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ffdec => "FFDec",
            Self::Ruffle => "Ruffle",
        }
    }
    fn url(self) -> &'static str {
        match self {
            Self::Ffdec => {
                "https://github.com/jindrapetrik/jpexs-decompiler/releases/download/version26.3.0/ffdec_26.3.0.zip"
            }
            Self::Ruffle => {
                "https://github.com/ruffle-rs/ruffle/releases/download/v0.7.1/ruffle-0.7.1-windows-x86_64.zip"
            }
        }
    }
    fn sha256(self) -> &'static str {
        match self {
            Self::Ffdec => "35f4930eb7c380afe66f2117f90b006deac0631473ad7500bb39c78f68645ecd",
            Self::Ruffle => "a5f6df9881771226e330e08a1fa7666f4caff50ddf9f2f993cdd95293edca627",
        }
    }
    fn executable(self) -> &'static str {
        match self {
            Self::Ffdec => "ffdec-cli.exe",
            Self::Ruffle => "ruffle.exe",
        }
    }
}

/// Only a fixed, explicitly user-requested upstream URL is downloaded. The
/// hash is pinned to the official GitHub release ZIP; no "latest" redirects.
fn verified_archive(bytes: &[u8], expected: &str) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_ARCHIVE {
        return Err("GFX utility download exceeds its 55 MiB safety limit".into());
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err("GFX utility SHA-256 mismatch; no executable was installed".into());
    }
    Ok(())
}

fn checked_member(name: &str) -> Result<PathBuf, String> {
    if name.is_empty()
        || name.len() > 1024
        || name.starts_with('/')
        || name.contains(['\\', ':', '\0'])
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("GFX utility archive contains an unsafe member path".into());
    }
    Ok(PathBuf::from(name))
}

fn extract_bounded(zip_data: &[u8], output: &Path) -> Result<(), String> {
    let reader = std::io::Cursor::new(zip_data);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| e.to_string())?;
    if archive.len() == 0 || archive.len() > MAX_MEMBERS {
        return Err("Unexpected GFX utility archive entry count".into());
    }
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut item = archive.by_index(i).map_err(|e| e.to_string())?;
        let is_dir = item.is_dir();
        // ZIP directories legitimately have a trailing slash; no path segment
        // may escape the staging root.
        let name = if is_dir {
            item.name().trim_end_matches('/')
        } else {
            item.name()
        };
        let relative = checked_member(name)?;
        if item
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("Symlink in GFX utility archive is not allowed".into());
        }
        let dest = output.join(relative);
        if is_dir {
            fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            continue;
        }
        total = total
            .checked_add(item.size())
            .ok_or("GFX utility unpack size overflow")?;
        if item.size() > MAX_EXTRACTED || total > MAX_EXTRACTED {
            return Err("GFX utility unpacked data exceeds 220 MiB".into());
        }
        let parent = dest.parent().ok_or("Invalid unpack destination")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)
            .map_err(|e| e.to_string())?;
        let copied = io::copy(&mut item, &mut file).map_err(|e| e.to_string())?;
        if copied != item.size() {
            return Err("Truncated GFX utility archive member".into());
        }
    }
    Ok(())
}

fn validated_file(root: &Path, name: &str) -> Result<PathBuf, String> {
    let path = root.join(name);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| format!("Required GFX tool file missing: {name}"))?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err(format!("Invalid GFX utility file: {name}"));
    }
    Ok(path)
}

fn fetch_limited(
    client: &reqwest::blocking::Client,
    url: &str,
    max: u64,
) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(url)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    if response.content_length().is_some_and(|length| length > max) {
        return Err("GFX utility download was larger than expected".into());
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err("GFX utility download exceeded its size budget".into());
    }
    Ok(bytes)
}

/// Run only from an explicit Install click in Settings. The existing app
/// already requires elevation and lives in Program Files when installed;
/// place tool executables alongside it, not in a writable per-user folder.
pub fn install(tool: Tool) -> Result<PathBuf, String> {
    if !cfg!(target_os = "windows") {
        return Err("In-app GFX tool installation is only supported on Windows".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let folder = exe
        .parent()
        .ok_or("Unable to locate the manager installation")?;
    install_at(tool, folder)
}

fn install_at(tool: Tool, folder: &Path) -> Result<PathBuf, String> {
    let third_party = folder.join("third_party");
    fs::create_dir_all(&third_party).map_err(|e| e.to_string())?;
    if !fs::symlink_metadata(&third_party)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_dir()
    {
        return Err("Third-party destination is not a regular directory".into());
    }
    let destination = third_party.join(tool.name());
    // Never change a prior installation or delete user content. The installer
    // will also install to this location if the user selected the optional task.
    if destination.exists() || fs::symlink_metadata(&destination).is_ok() {
        if !fs::symlink_metadata(&destination)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_dir()
        {
            return Err("Existing GFX tool directory is not a regular directory".into());
        }
        let existing = validated_file(&destination, tool.executable());
        return existing.or_else(|_| Err(format!(
            "{} directory already exists but is incomplete; reinstall via the manager installer",
            tool.name()
        )));
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| e.to_string())?;
    let archive_bytes = fetch_limited(&client, tool.url(), MAX_ARCHIVE)?;
    verified_archive(&archive_bytes, tool.sha256())?;
    let scratch = tempfile::Builder::new()
        .prefix(".gfx-tool-")
        .tempdir_in(&third_party)
        .map_err(|e| e.to_string())?;
    let payload = scratch.path().join("payload");
    fs::create_dir(&payload).map_err(|e| e.to_string())?;
    extract_bounded(&archive_bytes, &payload)?;
    validated_file(&payload, tool.executable())?;
    if tool == Tool::Ffdec {
        validated_file(&payload, "ffdec.jar")?;
        validated_file(&payload, "ffdec.exe")?;
        validated_file(&payload, "license.txt")?;
    } else {
        // The upstream desktop release ZIP does not always include its full
        // dependency notices. Fetch the pinned release-tagged license text.
        let license = fetch_limited(
            &client,
            "https://raw.githubusercontent.com/ruffle-rs/ruffle/v0.7.1/LICENSE.md",
            512 * 1024,
        )?;
        if !license.starts_with(b"# Ruffle") || license.len() < 512 {
            return Err("Ruffle license notice was not available; installation aborted".into());
        }
        let notice = payload.join("LICENSE.md");
        if fs::symlink_metadata(&notice).is_err() {
            let mut license_file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&notice)
                .map_err(|e| e.to_string())?;
            license_file
                .write_all(&license)
                .map_err(|e| e.to_string())?;
        } else {
            validated_file(&payload, "LICENSE.md")?;
        }
    }
    // Within the same filesystem: make validated content visible only after
    // every required download, ZIP check, and copyright notice succeeded.
    fs::rename(&payload, &destination).map_err(|e| e.to_string())?;
    validated_file(&destination, tool.executable())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_archive_traversal_and_drive_prefixes() {
        for invalid in [
            "../evil.exe",
            "/tmp/a",
            "C:/evil",
            "folder//a",
            "folder/./a",
            "folder/../a",
            "x\\y",
        ] {
            assert!(checked_member(invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            checked_member("ffdec-cli.exe").unwrap(),
            PathBuf::from("ffdec-cli.exe")
        );
        assert!(checked_member("lib/extension.jar").is_ok());
    }
    #[test]
    fn refuses_wrong_hash() {
        assert!(verified_archive(b"not an official zip", Tool::Ffdec.sha256()).is_err());
    }
    #[test]
    fn does_not_replace_or_modify_existing_directories() {
        let temp = tempfile::tempdir().unwrap();
        let installed = temp.path().join("third_party/FFDec");
        fs::create_dir_all(&installed).unwrap();
        let marker = installed.join("keep.txt");
        fs::write(&marker, b"keep").unwrap();
        assert!(install_at(Tool::Ffdec, temp.path()).is_err());
        assert_eq!(fs::read(&marker).unwrap(), b"keep");
    }
}
