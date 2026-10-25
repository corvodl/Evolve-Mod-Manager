//! Read-only What's New: latest public GitHub release and a separately hosted,
//! sanitized Discord Channel Following feed. Neither source receives credentials.
//! Failures never block game startup, updating, or mod operations.
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const RELEASES_API: &str =
    "https://api.github.com/repos/corvodl/Evolve-Mod-Manager/releases?per_page=20";
const DISCORD_FEED: &str = "https://corvodl.github.io/Evolve-Mod-Manager/announcements.json";
const MAX_RESPONSE: u64 = 1_000_000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cache {
    pub release: Option<Release>,
    pub announcements: Vec<Announcement>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub title: String,
    pub version: String,
    pub notes: String,
    pub url: String,
    pub published_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Announcement {
    pub id: String,
    pub author: String,
    pub timestamp: String,
    pub title: String,
    pub content: String,
    pub description: String,
    pub url: String,
    pub image_urls: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct NewsState {
    pub cache: Cache,
    pub loading: bool,
    pub loaded_at: Option<Instant>,
    pub release_status: String,
    pub discord_status: String,
}

#[derive(Clone, Debug)]
pub struct NewsResult {
    pub cache: Cache,
    pub release_status: String,
    pub discord_status: String,
}

impl NewsState {
    pub fn apply(&mut self, result: NewsResult) {
        self.cache = result.cache;
        self.loading = false;
        self.loaded_at = Some(Instant::now());
        self.release_status = result.release_status;
        self.discord_status = result.discord_status;
    }

    pub fn needs_refresh(&self) -> bool {
        !self.loading
            && self
                .loaded_at
                .is_none_or(|instant| instant.elapsed() >= Duration::from_secs(600))
    }
}

fn cache_file() -> Option<PathBuf> {
    super::settings_path()
        .ok()
        .map(|path| path.with_file_name("whats-new-cache.json"))
}

/// Show the last successful feed immediately, even before networking finishes.
pub fn cached_on_start() -> Cache {
    read_cache()
}

fn read_cache() -> Cache {
    cache_file()
        .and_then(|path| fs::read(path).ok())
        .filter(|bytes| bytes.len() <= MAX_RESPONSE as usize)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_cache(data: &Cache) {
    let Some(path) = cache_file() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(mut temp) = tempfile::NamedTempFile::new_in(parent) else {
        return;
    };
    if serde_json::to_writer(&mut temp, data).is_ok() {
        let _ = temp.as_file().sync_all();
        let _ = temp.persist(path);
    }
}

fn get_json(client: &Client, url: &str) -> Result<serde_json::Value, String> {
    let response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RESPONSE)
    {
        return Err("News response is too large".into());
    }
    let mut buffer = Vec::new();
    response
        .take(MAX_RESPONSE + 1)
        .read_to_end(&mut buffer)
        .map_err(|error| error.to_string())?;
    if buffer.len() as u64 > MAX_RESPONSE {
        return Err("News response exceeds the size limit".into());
    }
    serde_json::from_slice(&buffer).map_err(|error| error.to_string())
}

fn shorten(value: &str, max_chars: usize) -> String {
    let mut text: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        text.push('…');
    }
    text
}

fn release_from_response(value: &serde_json::Value) -> Result<Release, String> {
    let releases = value
        .as_array()
        .ok_or("GitHub release listing is invalid")?;
    let release = releases
        .iter()
        .filter(|value| value["draft"] == false)
        .filter(|value| value["published_at"].as_str().is_some())
        .max_by_key(|value| value["published_at"].as_str().unwrap_or_default().to_owned())
        .ok_or("No published GitHub Releases found")?;
    let version = release["tag_name"]
        .as_str()
        .filter(|tag| tag.starts_with('v') && tag.len() < 80)
        .ok_or("Missing valid GitHub release tag")?;
    let url = release["html_url"]
        .as_str()
        .filter(|url| {
            url.starts_with("https://github.com/corvodl/Evolve-Mod-Manager/releases/tag/")
        })
        .ok_or("GitHub release URL was not the expected repository")?;
    let notes = release["body"]
        .as_str()
        .unwrap_or("")
        .lines()
        .filter(|line| !line.starts_with("SHA256 "))
        .collect::<Vec<_>>()
        .join("\n");
    let name = release["name"]
        .as_str()
        .filter(|x| !x.is_empty())
        .unwrap_or(version);
    Ok(Release {
        title: shorten(name, 180),
        version: version.to_owned(),
        notes: shorten(&notes, 2500),
        url: url.to_owned(),
        published_at: release["published_at"].as_str().unwrap_or("").to_owned(),
    })
}

pub fn allowed_link(url: &str) -> bool {
    if url.len() > 2048 || url.contains(|c: char| c.is_control() || c == '\\') {
        return false;
    }
    let trimmed = url.strip_prefix("https://");
    match trimmed {
        Some(path) => {
            path.starts_with("github.com/corvodl/Evolve-Mod-Manager/releases/")
                || path.starts_with("discord.com/channels/")
                || path.starts_with("cdn.discordapp.com/attachments/")
                || path.starts_with("media.discordapp.net/attachments/")
        }
        None => false,
    }
}

fn announcements_from_response(value: &serde_json::Value) -> Result<Vec<Announcement>, String> {
    if value["schema_version"].as_u64() != Some(1) {
        return Err("Unsupported Discord feed schema".into());
    }
    let posts = value["announcements"]
        .as_array()
        .ok_or("Discord feed does not contain an announcement list")?;
    let mut parsed = Vec::new();
    for value in posts.iter().take(20) {
        let Some(id) = value["id"]
            .as_str()
            .filter(|v| !v.is_empty() && v.len() < 40)
        else {
            continue;
        };
        let Some(url) = value["url"]
            .as_str()
            .filter(|v| allowed_link(v) && v.starts_with("https://discord.com/channels/"))
        else {
            continue;
        };
        let field =
            |name: &str, max: usize| shorten(value[name].as_str().unwrap_or("").trim(), max);
        let images = value["image_urls"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .filter(|v| allowed_link(v) && !v.contains('#'))
            .take(4)
            .map(str::to_owned)
            .collect();
        parsed.push(Announcement {
            id: id.to_owned(),
            url: url.to_owned(),
            author: field("author", 100),
            timestamp: field("timestamp", 60),
            title: field("title", 200),
            content: field("content", 6000),
            description: field("description", 6000),
            image_urls: images,
        });
    }
    Ok(parsed)
}

/// Refresh both sources independently, retaining last good cached data when
/// either GitHub, Discord/Pages or the network cannot be reached.
pub fn refresh() -> NewsResult {
    let mut cache = read_cache();
    let mut result = NewsResult {
        cache: Cache::default(),
        release_status: String::new(),
        discord_status: String::new(),
    };
    let client = Client::builder()
        .user_agent("Evolve-Mod-Manager-WhatsNew/1.0")
        .timeout(Duration::from_secs(12))
        .build();
    match client {
        Ok(client) => {
            match get_json(&client, RELEASES_API).and_then(|v| release_from_response(&v)) {
                Ok(release) => {
                    cache.release = Some(release);
                    result.release_status = "Live GitHub release notes".into();
                }
                Err(error) => {
                    result.release_status =
                        format!("GitHub unavailable; cached notes if available: {error}");
                }
            }
            match get_json(&client, DISCORD_FEED).and_then(|v| announcements_from_response(&v)) {
                Ok(announcements) => {
                    cache.announcements = announcements;
                    result.discord_status = "Synced forwarded Discord announcements".into();
                }
                Err(error) => {
                    result.discord_status =
                        format!("Discord feed unavailable; cached posts if available: {error}");
                }
            }
        }
        Err(error) => {
            result.release_status = format!("Offline: {error}");
            result.discord_status = "Offline: showing cached announcements when available".into();
        }
    }
    save_cache(&cache);
    result.cache = cache;
    result
}

/// Launch only known HTTPS announcement, image or official release links.
pub fn open_link(url: &str) -> Result<(), String> {
    if !allowed_link(url) {
        return Err("The announcement contained an unsupported external URL".into());
    }
    #[cfg(windows)]
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open external announcement: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_published_release_includes_nightlies() {
        let releases = serde_json::json!([
            {"draft":false,"prerelease":false,"published_at":"2026-10-24T14:00:00Z",
             "tag_name":"v1.0.0","html_url":"https://github.com/corvodl/Evolve-Mod-Manager/releases/tag/v1.0.0","name":"Stable","body":"Old"},
            {"draft":false,"prerelease":true,"published_at":"2026-10-25T14:00:00Z",
             "tag_name":"v0.1.0-nightly.60","html_url":"https://github.com/corvodl/Evolve-Mod-Manager/releases/tag/v0.1.0-nightly.60","name":"Nightly 60","body":"Progress\nSHA256 file abc"},
            {"draft":true,"published_at":"2026-10-26T14:00:00Z",
             "tag_name":"v2.0.0","html_url":"https://github.com/corvodl/Evolve-Mod-Manager/releases/tag/v2.0.0","name":"Draft"}
        ]);
        let release = release_from_response(&releases).unwrap();
        assert_eq!(release.version, "v0.1.0-nightly.60");
        assert_eq!(release.notes, "Progress");
    }

    #[test]
    fn feed_sanitizes_bad_links_and_never_claims_fake_posts() {
        let value = serde_json::json!({
            "schema_version":1,
            "announcements":[
                {"id":"1","url":"javascript:alert(1)","content":"bad"},
                {"id":"2","url":"https://discord.com/channels/1/2/3","content":"ok",
                 "image_urls":["https://evil.org/a.png","https://cdn.discordapp.com/attachments/1/2/3.png"]}
            ]
        });
        let posts = announcements_from_response(&value).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].content, "ok");
        assert_eq!(posts[0].image_urls.len(), 1);
        assert!(!allowed_link(
            "https://discord.com\\@evil.example/channels/1"
        ));
    }

    #[test]
    fn cache_defaults_to_empty_when_never_fetched() {
        assert!(Cache::default().release.is_none());
        assert!(Cache::default().announcements.is_empty());
    }
}
