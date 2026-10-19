//! Opt-in public GitHub Releases updater.
//!
//! The release description contains SHA256 lines for the only two distributable
//! assets: the portable ZIP and the installer. Downloads are checked before any
//! installer is launched; game archives and user preferences are never modified.
use reqwest::blocking::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const API: &str = "https://api.github.com/repos/corvodl/Evolve-Mod-Manager";
const OWNER: &str = "corvodl";
const REPOSITORY: &str = "Evolve-Mod-Manager";
const MAX_METADATA: u64 = 1_000_000;
const MAX_DOWNLOAD: u64 = 500 * 1024 * 1024;
const USER_AGENT: &str = "EvolveModManager-Updater/1.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    Stable,
    Beta,
}

#[derive(Clone, Debug)]
pub struct VerifiedAsset {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub url: String,
}

#[derive(Clone, Debug)]
pub struct ReleaseUpdate {
    pub version: String,
    pub channel: UpdateChannel,
    pub installer: VerifiedAsset,
    pub portable: VerifiedAsset,
    pub release_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("The update server could not be reached: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Release metadata failed verification: {0}")]
    Invalid(String),
    #[error("The downloaded file failed SHA-256 integrity verification")]
    HashMismatch,
    #[error("The selected update exceeds the allowed size")]
    TooLarge,
    #[error("File access error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    body: Option<String>,
    draft: bool,
    prerelease: bool,
    html_url: String,
    assets: Vec<ApiAsset>,
}
#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

fn client() -> Result<Client, UpdateError> {
    Ok(Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?)
}

fn github_release_prefix(version: &str) -> String {
    format!("https://github.com/{OWNER}/{REPOSITORY}/releases/download/v{version}/")
}

fn validate_digest(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|ch| ch.is_ascii_hexdigit())
}

fn asset_checksum(body: &str, name: &str) -> Option<String> {
    let candidates = body
        .lines()
        .filter_map(|line| {
            let tokens: Vec<&str> = line.split_ascii_whitespace().collect();
            if tokens.len() == 3
                && tokens[0] == "SHA256"
                && tokens[1] == name
                && validate_digest(tokens[2])
            {
                Some(tokens[2].to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        None
    } else {
        candidates.first().cloned()
    }
}

fn parse_asset(
    release: &ApiRelease,
    version: &str,
    file: &str,
) -> Result<VerifiedAsset, UpdateError> {
    let matches = release
        .assets
        .iter()
        .filter(|item| item.name == file)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(UpdateError::Invalid(format!(
            "missing or duplicate {file} asset"
        )));
    }
    let asset = matches[0];
    if asset.size < 256
        || asset.size > MAX_DOWNLOAD
        || asset.browser_download_url != format!("{}{file}", github_release_prefix(version))
    {
        return Err(UpdateError::Invalid(
            "download address or size is not the expected public release".into(),
        ));
    }
    let checksum = asset_checksum(release.body.as_deref().unwrap_or(""), file)
        .ok_or_else(|| UpdateError::Invalid(format!("no unique published SHA256 for {file}")))?;
    Ok(VerifiedAsset {
        name: asset.name.clone(),
        size: asset.size,
        sha256: checksum,
        url: asset.browser_download_url.clone(),
    })
}

fn parse_release(release: ApiRelease) -> Result<ReleaseUpdate, UpdateError> {
    let version = release
        .tag_name
        .strip_prefix('v')
        .ok_or_else(|| UpdateError::Invalid("release tag must start with v".into()))?;
    let semver = Version::parse(version)
        .map_err(|_| UpdateError::Invalid("release tag is not semver".into()))?;
    if release.draft || release.prerelease != !semver.pre.is_empty() {
        return Err(UpdateError::Invalid(
            "draft or inconsistent release metadata".into(),
        ));
    }
    let is_beta = !semver.pre.is_empty();
    let channel = if is_beta {
        UpdateChannel::Beta
    } else {
        UpdateChannel::Stable
    };
    let prefix = format!("EvolveModManager-");
    let portable = parse_asset(
        &release,
        version,
        &format!("{prefix}Portable-{version}.zip"),
    )?;
    let installer = parse_asset(&release, version, &format!("{prefix}Setup-{version}.exe"))?;
    if release.assets.len() != 2 {
        return Err(UpdateError::Invalid(
            "release must have exactly a portable ZIP and installer".into(),
        ));
    }
    if release.html_url
        != format!("https://github.com/{OWNER}/{REPOSITORY}/releases/tag/v{version}")
    {
        return Err(UpdateError::Invalid("unexpected public release URL".into()));
    }
    Ok(ReleaseUpdate {
        version: version.into(),
        channel,
        portable,
        installer,
        release_url: release.html_url,
    })
}

fn read_metadata(mut response: impl Read) -> Result<Vec<u8>, UpdateError> {
    let mut bounded = response.by_ref().take(MAX_METADATA + 1);
    let mut result = Vec::new();
    bounded.read_to_end(&mut result)?;
    if result.len() as u64 > MAX_METADATA {
        return Err(UpdateError::TooLarge);
    }
    Ok(result)
}

pub fn discover_update(
    channel: UpdateChannel,
    installed: &str,
) -> Result<Option<ReleaseUpdate>, UpdateError> {
    let installed = Version::parse(installed)
        .map_err(|_| UpdateError::Invalid("current version is not semver".into()))?;
    let url = match channel {
        UpdateChannel::Stable => format!("{API}/releases/latest"),
        UpdateChannel::Beta => format!("{API}/releases?per_page=25"),
    };
    let response = client()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()?;
    if response.status().as_u16() == 404 {
        return Ok(None);
    }
    let response = response.error_for_status()?;
    let bytes = read_metadata(response)?;
    let candidates = match channel {
        UpdateChannel::Stable => vec![
            serde_json::from_slice::<ApiRelease>(&bytes)
                .map_err(|e| UpdateError::Invalid(e.to_string()))?,
        ],
        UpdateChannel::Beta => serde_json::from_slice::<Vec<ApiRelease>>(&bytes)
            .map_err(|e| UpdateError::Invalid(e.to_string()))?,
    };
    let mut available = Vec::new();
    for candidate in candidates {
        if candidate.draft || (channel == UpdateChannel::Beta && !candidate.prerelease) {
            continue;
        }
        let Ok(parsed) = parse_release(candidate) else {
            continue;
        };
        if parsed.channel != channel {
            continue;
        }
        let Ok(version) = Version::parse(&parsed.version) else {
            continue;
        };
        if version > installed {
            available.push((version, parsed));
        }
    }
    available.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(available.pop().map(|(_, value)| value))
}

fn check_file_hash(path: &Path, expected: &VerifiedAsset) -> Result<(), UpdateError> {
    let info = fs::symlink_metadata(path)?;
    if !info.file_type().is_file() || info.len() != expected.size {
        return Err(UpdateError::HashMismatch);
    }
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher_writer(&mut hasher))?;
    if format!("{:x}", hasher.finalize()) != expected.sha256 {
        return Err(UpdateError::HashMismatch);
    }
    Ok(())
}

struct DigestWriter<'a>(&'a mut Sha256);
impl Write for DigestWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn hasher_writer(digest: &mut Sha256) -> DigestWriter<'_> {
    DigestWriter(digest)
}

pub fn download_installer(update: &ReleaseUpdate, root: &Path) -> Result<PathBuf, UpdateError> {
    let target_root = root.join("Updates");
    fs::create_dir_all(&target_root)?;
    if fs::symlink_metadata(&target_root)?.file_type().is_symlink() {
        return Err(UpdateError::Invalid("updates folder is a symlink".into()));
    }
    let target = target_root.join(&update.installer.name);
    if target.exists() {
        check_file_hash(&target, &update.installer)?;
        return Ok(target);
    }
    let mut response = client()?
        .get(&update.installer.url)
        .send()?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|v| v > update.installer.size)
    {
        return Err(UpdateError::TooLarge);
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".update-")
        .tempfile_in(&target_root)?;
    let mut digest = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or(UpdateError::TooLarge)?;
        if size > update.installer.size || size > MAX_DOWNLOAD {
            return Err(UpdateError::TooLarge);
        }
        temporary.write_all(&buffer[..count])?;
        digest.update(&buffer[..count]);
    }
    if size != update.installer.size
        || format!("{:x}", digest.finalize()) != update.installer.sha256
    {
        return Err(UpdateError::HashMismatch);
    }
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist_noclobber(&target)
        .map_err(|e| UpdateError::Io(e.error))?;
    check_file_hash(&target, &update.installer)?;
    Ok(target)
}

/// The user must explicitly select "Launch installer" in Settings. This
/// function does not install anything silently, modify application data,
/// or write to the game's directory.
pub fn launch_verified_installer(
    path: &Path,
    update: &ReleaseUpdate,
    root: &Path,
) -> Result<(), UpdateError> {
    let updates = fs::canonicalize(root.join("Updates"))?;
    let canonical = fs::canonicalize(path)?;
    if canonical.parent() != Some(updates.as_path())
        || canonical.file_name().and_then(|s| s.to_str()) != Some(update.installer.name.as_str())
        || !canonical
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
    {
        return Err(UpdateError::Invalid(
            "installer must be staged inside the managed Updates directory".into(),
        ));
    }
    check_file_hash(&canonical, &update.installer)?;
    Command::new(canonical).spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> ApiRelease {
        let version = "1.0.2";
        let prefix = github_release_prefix(version);
        let zip = format!("EvolveModManager-Portable-{version}.zip");
        let exe = format!("EvolveModManager-Setup-{version}.exe");
        let checksum = "a".repeat(64);
        ApiRelease {
            tag_name: format!("v{version}"),
            body: Some(format!(
                "Version 1.0.2\nSHA256 {zip} {checksum}\nSHA256 {exe} {checksum}"
            )),
            draft: false,
            prerelease: false,
            html_url: format!("https://github.com/{OWNER}/{REPOSITORY}/releases/tag/v{version}"),
            assets: vec![
                ApiAsset {
                    name: zip.clone(),
                    size: 2000,
                    browser_download_url: format!("{prefix}{zip}"),
                },
                ApiAsset {
                    name: exe.clone(),
                    size: 2000,
                    browser_download_url: format!("{prefix}{exe}"),
                },
            ],
        }
    }
    #[test]
    fn accepts_only_expected_two_asset_release() {
        let parsed = parse_release(release()).unwrap();
        assert_eq!(parsed.channel, UpdateChannel::Stable);
        assert_eq!(parsed.version, "1.0.2");
    }
    #[test]
    fn refuses_release_with_missing_checksum_or_extra_asset() {
        let mut data = release();
        data.body = Some("Version only".into());
        assert!(parse_release(data).is_err());
        let mut extra = release();
        extra.assets.push(ApiAsset {
            name: "screenshot.png".into(),
            size: 3000,
            browser_download_url: "https://example.com".into(),
        });
        assert!(parse_release(extra).is_err());
    }
    #[test]
    fn rejects_repacked_installer_without_published_digest() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("setup.exe");
        fs::write(&path, vec![0u8; 2000]).unwrap();
        let update = parse_release(release()).unwrap();
        assert!(matches!(
            check_file_hash(&path, &update.installer),
            Err(UpdateError::HashMismatch)
        ));
    }
}
