//! Optional, local-only Discord Rich Presence. No OAuth, tokens, network API,
//! injection, game hooks, or dependency on launching Evolve.
//! Separate registered application IDs for Evolve Mod Manager and Evolve.
//! No account linking; the manager's development override is in Integrations.
//! A separate game RPC publisher may still compete with this activity.
use crate::{Page, State};

const MODDED_EVOLVE_DISCORD_APPLICATION_ID: &str = "1563784115785048114";
const EVOLVE_GAME_DISCORD_APPLICATION_ID: &str = "1565093139616763987";

/// The active IPC connection must use the app matching the visible activity.
/// User-supplied developer override applies only to the manager application.
fn active_application_id(cfg: &Config, game_pid: Option<u32>) -> &str {
    if game_pid.is_some() && cfg.show_game {
        EVOLVE_GAME_DISCORD_APPLICATION_ID
    } else {
        &cfg.app_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Config {
    pub enabled: bool,
    pub app_id: String,
    pub show_game: bool,
    pub show_manager: bool,
    pub show_character: bool,
    pub show_stats: bool,
    pub show_profile: bool,
    pub minimal: bool,
    pub game_root: String,
    pub game_version: String,
    pub manager_action: String,
    pub model_name: Option<String>,
    pub mod_profile: Option<String>,
    // The current client bridge has no verified map/mode source. Never invent one.
    pub map: Option<String>,
    pub game_mode: Option<String>,
}
impl Config {
    pub fn from_state(state: &State) -> Self {
        let p = &state.preferences;
        let model_name = if state.page == Some(Page::Models) {
            state
                .render_source
                .as_ref()
                .and_then(|file| file.file_stem())
                .or_else(|| std::path::Path::new(&state.input).file_stem())
                .map(|name| name.to_string_lossy().into_owned())
        } else {
            None
        };
        let manager_action = match state.page {
            Some(Page::Home) | None => "Home",
            Some(Page::Docs) => "Reading instructions",
            Some(Page::AssetLibrary) => "Browsing Asset Library",
            Some(Page::Files) => "Modding Evolve",
            Some(Page::WhatsNew) => "Reading updates",
            Some(Page::Credits) => "Credits",
            Some(Page::Developer) => "Developer tools",
            Some(Page::Models) => "Model Viewer",
            Some(Page::Levels) => "Level Editor",
            Some(Page::Settings) => "Settings",
        }
        .to_owned();
        let enabled: Vec<_> = state
            .modding_projects
            .as_ref()
            .into_iter()
            .flatten()
            .filter(|p| p.enabled)
            .collect();
        let mod_profile = match enabled.as_slice() {
            [] => None,
            [one] => Some(one.label.clone()),
            many => Some(format!("{} enabled mod projects", many.len())),
        };
        let use_test = p.play_through_test_client && !p.test_game_installation.is_empty();
        let game_root = if use_test {
            &p.test_game_installation
        } else {
            &p.game_installation
        };
        let game_version = if use_test {
            "Stage 2 • Test Client"
        } else {
            "Stage 2"
        }
        .to_owned();
        Self {
            enabled: p.discord_presence_enabled,
            app_id: if p.discord_application_id.is_empty() {
                MODDED_EVOLVE_DISCORD_APPLICATION_ID.to_owned()
            } else {
                p.discord_application_id.clone()
            },
            show_game: p.discord_show_game,
            show_manager: p.discord_show_manager,
            show_character: p.discord_show_character,
            show_stats: p.discord_show_stats,
            show_profile: p.discord_show_profile,
            minimal: p.discord_minimal,
            game_root: game_root.clone(),
            game_version,
            manager_action,
            model_name,
            mod_profile,
            map: None,
            game_mode: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct LocalActor {
    pub name: Option<String>,
    // Only a verified local character class/definition may enable
    // monster-specific information. The current nearby-string heuristic and
    // apparently plausible offsets cannot distinguish hunters from monsters.
    pub verified_monster: bool,
    pub evolution: Option<String>,
    pub health: Option<String>,
    pub armor: Option<String>,
}

/// The portrait registry is the sole mapping from the original entity
/// identifier to display name, team, and Discord image key. Every portrait is
/// present even when its game identifier is not yet known; adding a newly
/// discovered name only requires editing the manifest's internal_names list.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Team {
    Monster,
    Hunter,
    Unknown,
}

#[derive(Debug, serde::Deserialize)]
struct CharacterPortrait {
    asset_key: String,
    display_name: String,
    team: Team,
    internal_names: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct CharacterPortraitRegistry {
    characters: Vec<CharacterPortrait>,
    unillustrated_characters: Vec<UnillustratedCharacter>,
    non_player_entities: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct UnillustratedCharacter {
    display_name: String,
    team: Team,
    internal_names: Vec<String>,
}

fn portrait_registry() -> &'static CharacterPortraitRegistry {
    static REGISTRY: std::sync::OnceLock<CharacterPortraitRegistry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../assets/discord/character_portraits.json"
        ))
        .expect("checked-in Discord portrait manifest must be valid JSON")
    })
}

fn character_portraits() -> &'static [CharacterPortrait] {
    &portrait_registry().characters
}

fn character_from_entity_name(raw: &str) -> Option<(&'static str, Team)> {
    let name = raw.trim();
    let registry = portrait_registry();
    // NPC copies, summons, decoys and co-op actors are not evidence of the
    // currently controlled player character. Never assign their portraits.
    if registry
        .non_player_entities
        .iter()
        .any(|excluded| excluded.eq_ignore_ascii_case(name))
    {
        return None;
    }
    character_portraits()
        .iter()
        .find(|portrait| {
            portrait
                .internal_names
                .iter()
                .any(|known| known.eq_ignore_ascii_case(name))
        })
        .map(|portrait| (portrait.display_name.as_str(), portrait.team))
        .or_else(|| {
            registry
                .unillustrated_characters
                .iter()
                .find(|character| {
                    character
                        .internal_names
                        .iter()
                        .any(|known| known.eq_ignore_ascii_case(name))
                })
                .map(|character| (character.display_name.as_str(), character.team))
        })
}

fn portrait_for_display_name(name: &str) -> Option<&'static CharacterPortrait> {
    character_portraits()
        .iter()
        .find(|portrait| portrait.display_name.eq_ignore_ascii_case(name))
}

fn label(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_control())
        .take(128)
        .collect()
}
/// The runtime reports stage as a decimal 1, 2 or 3. Accept the older
/// "Stage N" form in tests and previously cached state without duplicating
/// the word Stage in Discord.
fn presence_stage(value: &str) -> Option<u8> {
    let value = value.trim().strip_prefix("Stage ").unwrap_or(value.trim());
    let stage = value.parse::<u8>().ok()?;
    (1..=3).contains(&stage).then_some(stage)
}

/// External Float32 values arrive with three decimal places. Discord's
/// compact activity lines use rounded integers with thousands separators.
fn presence_number(value: &str) -> Option<String> {
    let numeric = value.trim().parse::<f64>().ok()?;
    if !numeric.is_finite() || !(0.0..=1_000_000_000.0).contains(&numeric) {
        return None;
    }
    let digits = (numeric.round() as u64).to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    Some(grouped)
}

fn format_activity(
    cfg: &Config,
    game_pid: Option<u32>,
    actor: Option<&LocalActor>,
    start: i64,
) -> Option<serde_json::Value> {
    use serde_json::json;
    let game = game_pid.is_some() && cfg.show_game;
    if !cfg.enabled || (!game && !cfg.show_manager) {
        return None;
    }
    let (details, state) = if game {
        if cfg.minimal {
            ("Playing Evolve".to_owned(), "In game".to_owned())
        } else {
            let visible = cfg.show_character.then_some(actor).flatten();
            let mut details = visible
                .and_then(|a| a.name.as_deref())
                .map(|name| format!("Playing as {name}"))
                .unwrap_or_else(|| "Playing Evolve".to_owned());
            if let Some(stage) = visible
                .filter(|a| a.verified_monster)
                .and_then(|a| a.evolution.as_deref())
                .and_then(presence_stage)
            {
                details.push_str(&format!(" · Stage {stage}"));
            }
            let mut stats = Vec::new();
            if cfg.show_stats {
                if let Some(health) = actor
                    .and_then(|a| a.health.as_deref())
                    .and_then(presence_number)
                {
                    stats.push(format!("Health {health}"));
                }
                if let Some(armor) = actor
                    .filter(|a| a.verified_monster)
                    .and_then(|a| a.armor.as_deref())
                    .and_then(presence_number)
                {
                    stats.push(format!("Armor {armor}"));
                }
            }
            let state = if stats.is_empty() {
                "In match".to_owned()
            } else {
                stats.join(" · ")
            };
            (details, state)
        }
    } else {
        let details = "Evolve Mod Manager".to_owned();
        let state = if cfg.minimal {
            "In manager".to_owned()
        } else if let Some(name) = cfg.model_name.as_ref() {
            format!("Viewing {name}")
        } else if cfg.show_profile {
            cfg.mod_profile
                .as_ref()
                .map(|profile| format!("{} • {profile}", cfg.manager_action))
                .unwrap_or_else(|| cfg.manager_action.clone())
        } else {
            cfg.manager_action.clone()
        };
        (details, state)
    };
    // Character art belongs to the separate Evolve Discord application.
    // Asset keys match the 39 PNGs in the import-ready portrait bundle.
    // Unrecognized characters have no guessed picture; the manager keeps its
    // own logo, and privacy/minimal modes never reveal character imagery.
    let assets = if game {
        if cfg.show_character && !cfg.minimal {
            actor
                .and_then(|a| a.name.as_deref())
                .and_then(portrait_for_display_name)
                .map(|portrait| {
                    json!({"large_image": portrait.asset_key, "large_text": portrait.display_name})
                })
                .unwrap_or_else(|| json!({}))
        } else {
            json!({})
        }
    } else {
        json!({"large_image":"evolve", "large_text":"Evolve Mod Manager"})
    };
    Some(json!({
        // Discord may ignore the name override and use the registered app
        // title. The details line always describes the correct application.
        "name": if game { "Evolve" } else { "Evolve Mod Manager" },
        "details": label(&details),
        "state": label(&state),
        "timestamps": {"start":start},
        "assets":assets,
        "buttons":[{"label":"Modded Evolve","url":"https://modded-evolve.com/"}],
        "instance":false
    }))
}

#[cfg(windows)]
mod windows {
    use super::*;
    use crate::{runtime_actors, runtime_memory};
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn epoch() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }
    fn frame(pipe: &mut File, opcode: u32, data: &serde_json::Value) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(data).map_err(std::io::Error::other)?;
        pipe.write_all(&opcode.to_le_bytes())?;
        pipe.write_all(&(bytes.len() as u32).to_le_bytes())?;
        pipe.write_all(&bytes)?;
        pipe.flush()
    }
    fn receive(pipe: &mut File) -> std::io::Result<(u32, serde_json::Value)> {
        let mut header = [0u8; 8];
        pipe.read_exact(&mut header)?;
        let opcode = u32::from_le_bytes(header[..4].try_into().unwrap());
        let size = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
        if size > 1_048_576 {
            return Err(std::io::Error::other("Discord RPC response too large"));
        }
        let mut bytes = vec![0; size];
        pipe.read_exact(&mut bytes)?;
        let json = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        Ok((opcode, json))
    }
    fn connect(app_id: &str) -> std::io::Result<File> {
        for i in 0..10 {
            let path = format!(r"\\.\pipe\discord-ipc-{i}");
            if let Ok(mut pipe) = OpenOptions::new().read(true).write(true).open(path) {
                frame(&mut pipe, 0, &serde_json::json!({"v":1,"client_id":app_id}))?;
                let (opcode, response) = receive(&mut pipe)?;
                if opcode == 1 && response["evt"] != "ERROR" {
                    return Ok(pipe);
                }
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "Discord desktop IPC unavailable",
        ))
    }
    fn activity(
        pipe: &mut File,
        value: Option<serde_json::Value>,
        nonce: u64,
    ) -> std::io::Result<()> {
        frame(
            pipe,
            1,
            &serde_json::json!({
                "cmd":"SET_ACTIVITY",
                "args":{"pid":std::process::id(),"activity":value},
                "nonce":nonce.to_string()
            }),
        )?;
        // Discord responds once per command; drain the acknowledgement to avoid
        // exhausting the named-pipe buffer during long sessions.
        let (opcode, response) = receive(pipe)?;
        if opcode != 1 || response["evt"] == "ERROR" {
            return Err(std::io::Error::other(
                "Discord rejected Rich Presence update",
            ));
        }
        Ok(())
    }
    fn actor_info(snapshot: &runtime_memory::Snapshot, root: &str) -> Option<LocalActor> {
        let listing = runtime_actors::inspect(snapshot, root).ok()?;
        let actor = listing.actors.iter().find(|actor| actor.local)?;
        let mut info = LocalActor::default();
        // Actor::name comes from the independently validated local IEntity,
        // not a nearby-string search or plausible monster field offsets.
        if let Some((name, team)) = character_from_entity_name(&actor.name) {
            info.name = Some(name.to_owned());
            info.verified_monster = team == Team::Monster;
        }
        for field in &actor.fields {
            use runtime_actors::Field;
            match field.field {
                Field::Health => info.health = Some(field.display.clone()),
                Field::Armor if info.verified_monster => info.armor = Some(field.display.clone()),
                Field::Stage if info.verified_monster => {
                    info.evolution = Some(field.display.clone())
                }
                _ => {}
            }
        }
        Some(info)
    }
    pub(super) fn worker(shared: Arc<Mutex<Config>>, running: Arc<AtomicBool>) {
        let mut pipe: Option<File> = None;
        let mut app_id = String::new();
        let mut last_activity: Option<serde_json::Value> = None;
        let mut last_pid = None;
        let mut session_start = epoch();
        let manager_start = session_start;
        let mut local_actor: Option<LocalActor> = None;
        let mut last_actor_check = Instant::now() - Duration::from_secs(60);
        let mut nonce = 1u64;
        let mut last_presence_update = Instant::now() - Duration::from_secs(60);
        while running.load(Ordering::Relaxed) {
            let cfg = shared
                .lock()
                .map(|s| s.clone())
                .unwrap_or_else(|e| e.into_inner().clone());
            if !cfg.enabled {
                if let Some(mut connected) = pipe.take() {
                    let _ = activity(&mut connected, None, nonce);
                    nonce += 1;
                }
                last_activity = None;
                last_pid = None;
                thread::sleep(Duration::from_secs(3));
                continue;
            }
            let running_game = if cfg.show_game {
                let snapshot = runtime_memory::inspect(&cfg.game_root);
                if snapshot.pid != last_pid {
                    session_start = epoch();
                    local_actor = None;
                    last_actor_check = Instant::now() - Duration::from_secs(60);
                    last_pid = snapshot.pid;
                }
                if snapshot.pid.is_some()
                    && (cfg.show_character || cfg.show_stats)
                    && last_actor_check.elapsed() >= Duration::from_secs(20)
                {
                    last_actor_check = Instant::now();
                    local_actor = actor_info(&snapshot, &cfg.game_root);
                }
                snapshot.pid
            } else {
                last_pid = None;
                local_actor = None;
                None
            };
            let selected_app_id = active_application_id(&cfg, running_game);
            let valid_app_id = selected_app_id.bytes().all(|b| b.is_ascii_digit())
                && (15..=25).contains(&selected_app_id.len());
            if !valid_app_id {
                if let Some(mut connected) = pipe.take() {
                    let _ = activity(&mut connected, None, nonce);
                    nonce += 1;
                }
                last_activity = None;
                app_id.clear();
                thread::sleep(Duration::from_secs(3));
                continue;
            }
            if app_id != selected_app_id {
                // Clear our activity under the previous application before
                // changing the handshake ID. Never clear a game's own RPC
                // activity or send a false PID to impersonate Evolve.exe.
                if let Some(mut connected) = pipe.take() {
                    let _ = activity(&mut connected, None, nonce);
                    nonce += 1;
                }
                app_id = selected_app_id.to_owned();
                last_activity = None;
            }
            if pipe.is_none() {
                pipe = connect(&app_id).ok();
                last_activity = None;
            }
            let visible_actor = if cfg.show_character || cfg.show_stats {
                local_actor.as_ref()
            } else {
                None
            };
            let next = format_activity(
                &cfg,
                running_game,
                visible_actor,
                if running_game.is_some() {
                    session_start
                } else {
                    manager_start
                },
            );
            // Periodically refresh only while Evolve is running: a native
            // presence publisher may update the same application independently.
            // This is best-effort and cannot guarantee Discord display priority.
            let refresh_due =
                running_game.is_some() && last_presence_update.elapsed() >= Duration::from_secs(30);
            if (next != last_activity || (next.is_some() && refresh_due))
                && let Some(ref mut connected) = pipe
            {
                match activity(connected, next.clone(), nonce) {
                    Ok(()) => {
                        last_activity = next;
                        last_presence_update = Instant::now();
                    }
                    Err(_) => {
                        pipe = None;
                        last_activity = None;
                    }
                }
                nonce += 1;
            }
            thread::sleep(Duration::from_secs(8));
        }
        if let Some(mut connected) = pipe {
            let _ = activity(&mut connected, None, nonce);
        }
    }
}

pub(super) struct Service {
    #[cfg(windows)]
    shared: std::sync::Arc<std::sync::Mutex<Config>>,
    #[cfg(windows)]
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Service {
    pub fn start(initial: Config) -> Self {
        #[cfg(windows)]
        {
            let shared = std::sync::Arc::new(std::sync::Mutex::new(initial));
            let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            let thread_shared = shared.clone();
            let thread_running = running.clone();
            // All pipe IO, process enumeration and read-only actor detection
            // remain on an independent thread, never the GUI or launch worker.
            let _ = std::thread::Builder::new()
                .name("discord-presence".into())
                .spawn(move || windows::worker(thread_shared, thread_running));
            Self { shared, running }
        }
        #[cfg(not(windows))]
        {
            let _ = initial;
            Self {}
        }
    }
    pub fn publish(&self, next: Config) {
        #[cfg(windows)]
        {
            if let Ok(mut guard) = self.shared.lock() {
                *guard = next;
            }
        }
        #[cfg(not(windows))]
        let _ = next;
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        #[cfg(windows)]
        self.running
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Config {
        Config {
            enabled: true,
            app_id: "123456789012345678".into(),
            show_game: true,
            show_manager: true,
            show_character: true,
            show_stats: true,
            show_profile: true,
            minimal: false,
            game_root: String::new(),
            game_version: "Stage 2".into(),
            manager_action: "Modding Evolve".into(),
            model_name: None,
            mod_profile: Some("My Mod".into()),
            map: None,
            game_mode: None,
        }
    }
    #[test]
    fn gameplay_shows_verified_character_name_and_monster_stats() {
        let actor = LocalActor {
            name: Some("Goliath".into()),
            verified_monster: true,
            evolution: Some("Stage 2".into()),
            health: Some("8500".into()),
            armor: Some("1000".into()),
        };
        let act = format_activity(&sample(), Some(100), Some(&actor), 100).unwrap();
        assert_eq!(act["details"], "Playing as Goliath · Stage 2");
        assert_eq!(act["state"], "Health 8,500 · Armor 1,000");
        assert!(!act["state"].as_str().unwrap().contains("Stage 2"));
        assert!(act["assets"].get("small_image").is_none());
    }
    #[test]
    fn hunter_never_shows_armor_or_evolution_even_with_plausible_raw_offsets() {
        let hunter = LocalActor {
            verified_monster: false,
            health: Some("1600".into()),
            // These deliberately mimic false-positive monster offsets.
            armor: Some("1000".into()),
            evolution: Some("Stage 2".into()),
            ..LocalActor::default()
        };
        let activity = format_activity(&sample(), Some(123), Some(&hunter), 100).unwrap();
        let state = activity["state"].as_str().unwrap();
        assert_eq!(activity["details"], "Playing Evolve");
        assert_eq!(state, "Health 1,600");
        assert!(!state.contains("Armor"));
        assert!(!state.contains("Evolution"));
    }

    #[test]
    fn unknown_actor_does_not_infer_monster_from_stage_or_armor() {
        let unknown = LocalActor {
            armor: Some("999".into()),
            evolution: Some("Stage 1".into()),
            ..LocalActor::default()
        };
        let state = format_activity(&sample(), Some(123), Some(&unknown), 100).unwrap();
        assert!(!state.to_string().contains("Armor"));
        assert!(!state.to_string().contains("Evolution"));
    }

    #[test]
    fn registry_keeps_all_portraits_even_without_internal_names() {
        let portraits = character_portraits();
        assert_eq!(portraits.len(), 39);
        assert_eq!(
            portraits
                .iter()
                .filter(|p| !p.internal_names.is_empty())
                .count(),
            37
        );
        let mut seen = std::collections::HashSet::new();
        for portrait in portraits {
            assert!(
                seen.insert(portrait.asset_key.as_str()),
                "Duplicate Discord asset key"
            );
            assert!(
                portrait
                    .asset_key
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            );
            assert!(!portrait.display_name.is_empty());
        }
        assert_eq!(
            portrait_for_display_name("Renegade Abe").unwrap().asset_key,
            "aberenegade"
        );
        assert_eq!(
            portrait_for_display_name("Glacial Behemoth")
                .unwrap()
                .asset_key,
            "rockybobcicle"
        );
        assert_eq!(
            portrait_for_display_name("Tech").unwrap().team,
            Team::Unknown
        );
        assert_eq!(portrait_registry().unillustrated_characters.len(), 2);
        assert_eq!(character_from_entity_name("UnknownABot"), None);
    }

    #[test]
    fn game_portrait_uses_exact_character_art_key() {
        let actor = LocalActor {
            name: Some("Elder Kraken".into()),
            verified_monster: true,
            ..LocalActor::default()
        };
        let activity = format_activity(&sample(), Some(123), Some(&actor), 100).unwrap();
        assert_eq!(activity["assets"]["large_image"], "krakenelder");
        assert_eq!(activity["assets"]["large_text"], "Elder Kraken");
        let mut hidden = sample();
        hidden.show_character = false;
        let activity = format_activity(&hidden, Some(123), Some(&actor), 100).unwrap();
        assert!(activity["assets"].get("large_image").is_none());
        hidden.show_character = true;
        hidden.minimal = true;
        let activity = format_activity(&hidden, Some(123), Some(&actor), 100).unwrap();
        assert!(activity["assets"].get("large_image").is_none());
    }

    #[test]
    fn maps_new_assault_internal_names_to_existing_portrait_assets() {
        for (internal, display, key) in [
            ("AssaultFBot", "Torvald", "torvald"),
            ("AssaultGBot", "Lennox", "lennox"),
            ("AssaultE_BlitzBot", "Blitz Markov", "markovblitz"),
        ] {
            assert_eq!(
                character_from_entity_name(internal),
                Some((display, Team::Hunter))
            );
            let portrait = portrait_for_display_name(display).unwrap();
            assert_eq!(portrait.asset_key, key);
            assert_eq!(portrait.team, Team::Hunter);
            let actor = LocalActor {
                name: Some(display.into()),
                ..LocalActor::default()
            };
            let activity = format_activity(&sample(), Some(123), Some(&actor), 100).unwrap();
            assert_eq!(activity["assets"]["large_image"], key);
            assert_eq!(activity["assets"]["large_text"], display);
        }
        assert_eq!(
            character_from_entity_name("assaulte_blitzbot"),
            Some(("Blitz Markov", Team::Hunter))
        );
    }
    #[test]
    fn maps_observed_local_entity_names_to_characters() {
        assert_eq!(
            character_from_entity_name("GoliathABot"),
            Some(("Goliath", Team::Monster))
        );
        assert_eq!(
            character_from_entity_name("TeleporterABot"),
            Some(("Wraith", Team::Monster))
        );
        assert_eq!(
            character_from_entity_name("AssaultEBot"),
            Some(("Markov", Team::Hunter))
        );
        assert_eq!(
            character_from_entity_name("AssaultBBot"),
            Some(("Parnell", Team::Hunter))
        );
        assert_eq!(
            character_from_entity_name("AssaultDBot"),
            Some(("Hyde", Team::Hunter))
        );
    }
    #[test]
    fn maps_all_verified_monsters_and_glacial_behemoth_spelling() {
        for (internal, expected) in [
            ("GoliathABot", "Goliath"),
            ("GoliathMeteorBot", "Meteor Goliath"),
            ("GrounderABot", "Kraken"),
            ("KrakenElderBot", "Elder Kraken"),
            ("TeleporterABot", "Wraith"),
            ("RockyABot", "Behemoth"),
            ("RockyBobcicleBot", "Glacial Behemoth"),
            ("LurkerABot", "Gorgon"),
        ] {
            assert_eq!(
                character_from_entity_name(internal),
                Some((expected, Team::Monster))
            );
        }
        assert_eq!(
            character_from_entity_name("ROCKYBOBCICLEBOT"),
            Some(("Glacial Behemoth", Team::Monster))
        );
        assert_eq!(character_from_entity_name("RockybobsicleABot"), None);
        assert_eq!(character_from_entity_name("KrakenABot"), None);
        assert_eq!(character_from_entity_name("UnknownABot"), None);
        assert_eq!(character_from_entity_name("AssaultABot"), None);
    }

    #[test]
    fn all_supplied_playable_hunter_names_have_exact_display_and_portraits() {
        for (internal, display) in [
            ("AssaultEBot", "Markov"),
            ("AssaultE_BlitzBot", "Blitz Markov"),
            ("AssaultDBot", "Hyde"),
            ("AssaultD_HellfireBot", "Hellfire Hyde"),
            ("AssaultBBot", "Parnell"),
            ("MedicParnellTankBot", "Tank Parnell"),
            ("AssaultFBot", "Torvald"),
            ("AssaultGBot", "Lennox"),
            ("AssaultAbeRenegadeBot", "Renegade Abe"),
            ("TrackingABot", "Griffin"),
            ("TrackingA_ElectroBot", "Electro Griffin"),
            ("TrackingCBot", "Abe"),
            ("TrackingEBot", "Maggie"),
            ("TrackingE_SavageBot", "Savage Maggie"),
            ("TrackingFBot", "Crow"),
            ("MedicBBot", "Lazarus"),
            ("MedicCBot", "Caira"),
            ("MedicC_LowgravBot", "Caira (Lowgrav)"),
            ("MedicEBot", "Val"),
            ("MedicE_RogueBot", "Rogue Val"),
            ("MedicFBot", "Slim"),
            ("MedicGBot", "Emet"),
            ("SupportABot", "Bucket"),
            ("SupportA_CaptainBot", "Captain Bucket"),
            ("SupportCBot", "Hank"),
            ("SupportC_SgtBot", "Sgt. Hank"),
            ("SupportEBot", "Cabot"),
            ("SupportE_BattleBot", "Battle Cabot"),
            ("SupportFBot", "Sunny"),
        ] {
            assert_eq!(
                character_from_entity_name(internal),
                Some((display, Team::Hunter))
            );
            assert!(portrait_for_display_name(display).is_some());
        }
    }

    #[test]
    fn local_coop_variants_use_the_corresponding_playable_portrait() {
        for (internal, expected) in [
            ("GoliathCoopBot", "Goliath"),
            ("GoliathBGHCoopBot", "Goliath"),
            ("GoliathMeteorCoopBot", "Meteor Goliath"),
            ("GoliathMeteorBGHCoopBot", "Meteor Goliath"),
            ("GrounderCoopBot", "Kraken"),
            ("KrakenElderCoopBot", "Elder Kraken"),
            ("TeleporterCoopBot", "Wraith"),
            ("RockyCoopBot", "Behemoth"),
            ("LurkerCoopBot", "Gorgon"),
            ("LurkerDDBot", "Gorgon"),
        ] {
            assert_eq!(
                character_from_entity_name(internal),
                Some((expected, Team::Monster))
            );
        }
    }

    #[test]
    fn missing_portraits_still_resolve_names_and_npcs_never_become_players() {
        for internal in ["TrackingGBot", "SupportGBot"] {
            assert_eq!(character_from_entity_name(internal), Some(("Jack", Team::Hunter)));
        }
        assert_eq!(
            character_from_entity_name("MedicB_NecroBot"),
            Some(("Necro Lazarus", Team::Hunter))
        );
        assert!(portrait_for_display_name("Jack").is_none());
        assert!(portrait_for_display_name("Necro Lazarus").is_none());
        for npc in &portrait_registry().non_player_entities {
            assert!(character_from_entity_name(npc).is_none(), "Non-player name: {npc}");
        }
    }

    #[test]
    fn game_activity_is_two_lines_with_compact_readable_numeric_values() {
        let actor = LocalActor {
            name: Some("Goliath".into()),
            verified_monster: true,
            evolution: Some("1".into()),
            health: Some("10400.000".into()),
            armor: Some("2343.978".into()),
        };
        let activity = format_activity(&sample(), Some(123), Some(&actor), 100).unwrap();
        assert_eq!(activity["details"], "Playing as Goliath · Stage 1");
        assert_eq!(activity["state"], "Health 10,400 · Armor 2,344");
        assert!(!activity.to_string().contains("Stage 2"));
        assert!(!activity.to_string().contains("My Mod"));
        let mut private = sample();
        private.show_stats = false;
        assert_eq!(
            format_activity(&private, Some(123), Some(&actor), 100).unwrap()["state"],
            "In match"
        );
        private.show_stats = true;
        private.show_character = false;
        let hidden = format_activity(&private, Some(123), Some(&actor), 100).unwrap();
        assert_eq!(hidden["details"], "Playing Evolve");
        assert_eq!(hidden["state"], "Health 10,400 · Armor 2,344");
        assert!(hidden["assets"].get("large_image").is_none());
    }
    #[test]
    fn unrecognized_name_does_not_display_untrusted_character() {
        let actor = LocalActor {
            name: None,
            ..Default::default()
        };
        let activity = format_activity(&sample(), Some(123), Some(&actor), 100).unwrap();
        assert_eq!(activity["details"], "Playing Evolve");
    }
    #[test]
    fn character_setting_off_hides_known_name() {
        let mut cfg = sample();
        cfg.show_character = false;
        let actor = LocalActor {
            name: Some("Markov".into()),
            ..Default::default()
        };
        let activity = format_activity(&cfg, Some(123), Some(&actor), 100).unwrap();
        assert_eq!(activity["details"], "Playing Evolve");
    }
    #[test]
    fn privacy_mode_hides_character_stats_and_mods() {
        let mut cfg = sample();
        cfg.minimal = true;
        let actor = LocalActor {
            name: Some("Wraith".into()),
            health: Some("1".into()),
            ..Default::default()
        };
        let act = format_activity(&cfg, Some(123), Some(&actor), 100).unwrap();
        assert_eq!(act["details"], "Playing Evolve");
        assert!(!act.to_string().contains("Wraith"));
        assert!(!act.to_string().contains("My Mod"));
        assert!(!act.to_string().contains("HP"));
    }
    #[test]
    fn disabled_and_no_manager_fallback_clear_activity() {
        let mut cfg = sample();
        cfg.enabled = false;
        assert!(format_activity(&cfg, Some(1), None, 1).is_none());
        cfg.enabled = true;
        cfg.show_manager = false;
        assert!(format_activity(&cfg, None, None, 1).is_none());
    }
    #[test]
    fn switches_application_id_between_manager_and_game() {
        let cfg = sample();
        assert_eq!(active_application_id(&cfg, None), "123456789012345678");
        assert_eq!(
            active_application_id(&cfg, Some(52288)),
            EVOLVE_GAME_DISCORD_APPLICATION_ID
        );
        let mut no_game = cfg.clone();
        no_game.show_game = false;
        assert_eq!(
            active_application_id(&no_game, Some(52288)),
            "123456789012345678"
        );
    }
    #[test]
    fn uses_supplied_application_ids_without_account_linking() {
        let state = State::default();
        let cfg = Config::from_state(&state);
        assert_eq!(cfg.app_id, MODDED_EVOLVE_DISCORD_APPLICATION_ID);
        assert_eq!(active_application_id(&cfg, None), "1563784115785048114");
        assert_eq!(
            active_application_id(&cfg, Some(11112)),
            "1565093139616763987"
        );
    }
    #[test]
    fn manager_and_game_have_distinct_presence_content() {
        let cfg = sample();
        let manager = format_activity(&cfg, None, None, 100).unwrap();
        assert_eq!(manager["name"], "Evolve Mod Manager");
        assert_eq!(manager["details"], "Evolve Mod Manager");
        assert!(
            manager["state"]
                .as_str()
                .unwrap()
                .contains("Modding Evolve")
        );
        let game = format_activity(&cfg, Some(11112), None, 100).unwrap();
        assert_eq!(game["name"], "Evolve");
        assert_eq!(game["details"], "Playing Evolve");
        assert!(game["assets"].get("large_image").is_none());
    }
    #[test]
    fn labels_are_capped_to_discord_limits() {
        assert_eq!(label(&"a".repeat(140)).len(), 128);
    }
}
