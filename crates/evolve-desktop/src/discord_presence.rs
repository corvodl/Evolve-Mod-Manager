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
    pub manager_page: Page,
    /// Exact selected virtual asset/member where known, never an absolute
    /// Windows path. Unknown/ambiguous selections remain absent.
    pub asset_name: Option<String>,
    pub mod_profile: Option<String>,
    // The current client bridge has no verified map/mode source. Never invent one.
    pub map: Option<String>,
    pub game_mode: Option<String>,
}
impl Config {
    pub fn from_state(state: &State) -> Self {
        let p = &state.preferences;
        let manager_page = state.page.unwrap_or(Page::Home);
        // Use the selected or loaded asset, not a stale editable input path.
        // A virtual GFX member is preferable to a temporary extraction name.
        // Never expose full local installation or account paths over Discord.
        let asset_name = match manager_page {
            Page::Models if state.gfx_active => state
                .gfx_session
                .as_ref()
                .map(|session| session.relative.clone()),
            Page::Models if state.text_document.is_some() => state
                .text_document
                .as_ref()
                .map(|document| document.member.clone()),
            Page::Models
                if state.model.is_some()
                    || state.texture_preview.is_some()
                    || state.character.is_some() =>
            {
                std::path::Path::new(&state.input)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            }
            Page::Levels => state
                .library_map
                .as_ref()
                .and_then(|(_, maps, index)| maps.get(*index))
                .map(|map| map.name.clone())
                .or_else(|| {
                    state
                        .level_source
                        .as_ref()
                        .and_then(|path| path.file_name())
                        .and_then(|name| name.to_str())
                        .map(str::to_owned)
                }),
            Page::AssetLibrary => state
                .library_selected
                .and_then(|index| state.library_catalog.as_ref()?.archives.get(index))
                .map(|archive| archive.name.clone()),
            Page::Files => state
                .modding_selected
                .and_then(|index| state.modding_projects.as_ref()?.get(index))
                .map(|project| {
                    if project.modified.len() == 1 {
                        project.modified[0].clone()
                    } else {
                        project.target.clone()
                    }
                }),
            _ => None,
        }
        .filter(|name| !name.trim().is_empty());
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
            manager_page,
            asset_name,
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
    /// Rounded 0–100, derived from the validated encoded evolution bar.
    /// None for stage 3, unverified actors and failed runtime reads.
    pub evolution_percentage: Option<u8>,
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
    non_player_entities: Vec<String>,
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

/// Discord artwork keys for the manager's *actual* Page SVG icons.
/// Matching PNGs must be uploaded to the manager Discord application.
fn manager_tab_art(page: Page) -> (&'static str, &'static str) {
    match page {
        Page::Home => ("tab_home", "Home"),
        Page::Docs => ("tab_instructions", "Instructions"),
        Page::AssetLibrary => ("tab_asset_library", "Asset Library"),
        Page::Files => ("tab_modding", "Modding"),
        Page::WhatsNew => ("tab_whats_new", "What's New"),
        Page::Credits => ("tab_credits", "Credits"),
        Page::Developer => ("tab_developer", "Developer"),
        Page::Models => ("tab_models", "Model Viewer"),
        Page::Levels => ("tab_levels", "Level Editor"),
        Page::Settings => ("tab_settings", "Settings"),
    }
}

// Validated against repeated live Stage 2 Cheat Engine sessions. These are
// module-relative roots, never absolute process addresses. Reject mismatched
// vtables and unknown enum values rather than guessing a game state.
const LOBBY_VTABLE_RVA: usize = 0x23852D0;
const LOBBY_STATE_OFFSET: usize = 0x8AC;
const LOBBY_POINTER_PATHS: [(usize, usize); 2] = [
    (0x3246360, 0x38),  // Primary; survived Evolve restart
    (0x2C67518, 0x1D8), // Fallback; independently restart-validated
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LobbyState {
    Menu,
    Initializing,
    FindGame,
    JoinSession,
    Lobby,
    PreRound,
    Game,
    Leaving,
}
impl LobbyState {
    fn from_raw(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Menu),
            1 => Some(Self::Initializing),
            2 => Some(Self::FindGame),
            3 => Some(Self::JoinSession),
            4 => Some(Self::Lobby),
            5 => Some(Self::PreRound),
            6 => Some(Self::Game),
            10 => Some(Self::Leaving),
            _ => None,
        }
    }
    fn presence(self) -> &'static str {
        match self {
            Self::Menu => "In Menus",
            Self::Initializing => "Connecting",
            Self::FindGame => "Finding a Match",
            Self::JoinSession => "Joining Session",
            Self::Lobby => "In Lobby",
            Self::PreRound => "Selecting Character",
            Self::Game => "In Match",
            Self::Leaving => "Leaving Match",
        }
    }
}

/// Resolve both known static chains and read the native UInt32 lobby state.
/// Pure so that fallback and fail-closed behavior can be tested without a game.
fn resolve_lobby_state<P, U>(
    module_base: usize,
    mut pointer_at: P,
    mut u32_at: U,
) -> Option<LobbyState>
where
    P: FnMut(usize) -> Option<usize>,
    U: FnMut(usize) -> Option<u32>,
{
    let expected_vtable = module_base.checked_add(LOBBY_VTABLE_RVA)?;
    for (root_rva, field_offset) in LOBBY_POINTER_PATHS {
        let Some(root) = module_base.checked_add(root_rva) else {
            continue;
        };
        let Some(first) = pointer_at(root).filter(|p| *p >= 0x10000) else {
            continue;
        };
        let Some(slot) = first.checked_add(field_offset) else {
            continue;
        };
        let Some(lobby) = pointer_at(slot).filter(|p| *p >= 0x10000) else {
            continue;
        };
        if pointer_at(lobby) != Some(expected_vtable) {
            continue;
        }
        let Some(state_address) = lobby.checked_add(LOBBY_STATE_OFFSET) else {
            continue;
        };
        if let Some(state) = u32_at(state_address).and_then(LobbyState::from_raw) {
            return Some(state);
        }
    }
    None
}

fn format_activity_with_lobby(
    cfg: &Config,
    game_pid: Option<u32>,
    actor: Option<&LocalActor>,
    start: i64,
    lobby_state: Option<LobbyState>,
) -> Option<serde_json::Value> {
    use serde_json::json;
    let game = game_pid.is_some() && cfg.show_game;
    let active_match = game && lobby_state == Some(LobbyState::Game);
    if !cfg.enabled || (!game && !cfg.show_manager) {
        return None;
    }
    let (details, state) = if game {
        if !active_match {
            // No actor, portrait or stale health values in menus or during
            // queueing/character select. Unknown reads must not say "In match".
            (
                "Playing Evolve".to_owned(),
                lobby_state
                    .map(LobbyState::presence)
                    .unwrap_or("Activity unavailable")
                    .to_owned(),
            )
        } else if cfg.minimal {
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
            if let Some(percent) = actor
                .filter(|a| {
                    cfg.show_stats
                        && a.verified_monster
                        && a.evolution
                            .as_deref()
                            .and_then(presence_stage)
                            .is_some_and(|stage| stage < 3)
                })
                .and_then(|a| a.evolution_percentage)
            {
                stats.push(format!("Evolution {percent}%"));
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
        } else if let Some(name) = cfg.asset_name.as_ref() {
            // Keep an exact selected member/path when available. The 128-char
            // Discord limit is applied only at the final serialization edge.
            format!("{} · {name}", cfg.manager_action)
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
        if active_match && cfg.show_character && !cfg.minimal {
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
        let (tab_image, tab_text) = manager_tab_art(cfg.manager_page);
        json!({
            "large_image": "evolve",
            "large_text": "Evolve Mod Manager",
            "small_image": tab_image,
            "small_text": tab_text
        })
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

// Keep legacy formatting tests explicitly in an active match; production
// always supplies a freshly validated lobby state.
#[cfg(test)]
fn format_activity(
    cfg: &Config,
    game_pid: Option<u32>,
    actor: Option<&LocalActor>,
    start: i64,
) -> Option<serde_json::Value> {
    format_activity_with_lobby(cfg, game_pid, actor, start, Some(LobbyState::Game))
}

#[cfg(windows)]
mod windows {
    use super::*;
    use crate::{runtime_actors, runtime_memory, runtime_research};
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
    fn inspect_lobby_state(snapshot: &runtime_memory::Snapshot) -> Option<LobbyState> {
        let pid = snapshot.pid?;
        let base = snapshot.module_base?;
        let handle: read_process_memory::ProcessHandle = pid.try_into().ok()?;
        let read_bytes = |at: usize, len: usize| {
            // Never dereference arbitrary invalid or kernel addresses.
            if !(0x10000..0x0000_7FFF_FFFF_FFFF).contains(&(at as u64)) {
                return None;
            }
            read_process_memory::copy_address(at, len, &handle).ok()
        };
        resolve_lobby_state(
            base,
            |at| {
                let bytes: [u8; 8] = read_bytes(at, 8)?.try_into().ok()?;
                usize::try_from(u64::from_le_bytes(bytes)).ok()
            },
            |at| {
                let bytes: [u8; 4] = read_bytes(at, 4)?.try_into().ok()?;
                Some(u32::from_le_bytes(bytes))
            },
        )
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
        if info.verified_monster {
            // Do not parse formatted Developer UI strings or treat a plain
            // actor-relative Float32 as evolution. The read-only helper
            // validates the actor, stage and session key anew.
            if let Ok(energy) = runtime_research::inspect_evolution(snapshot, root, actor) {
                info.evolution = Some(energy.stage.to_string());
                info.evolution_percentage = energy.percentage();
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
                local_actor = None;
                thread::sleep(Duration::from_secs(3));
                continue;
            }
            let lobby_state: Option<LobbyState>;
            let running_game = if cfg.show_game {
                let snapshot = runtime_memory::inspect(&cfg.game_root);
                if snapshot.pid != last_pid {
                    session_start = epoch();
                    local_actor = None;
                    last_actor_check = Instant::now() - Duration::from_secs(60);
                    last_pid = snapshot.pid;
                }
                lobby_state = snapshot.pid.and_then(|_| inspect_lobby_state(&snapshot));
                if lobby_state != Some(LobbyState::Game) {
                    local_actor = None;
                    last_actor_check = Instant::now() - Duration::from_secs(60);
                }
                if lobby_state == Some(LobbyState::Game)
                    && (cfg.show_character || cfg.show_stats)
                    && last_actor_check.elapsed() >= Duration::from_secs(8)
                {
                    last_actor_check = Instant::now();
                    local_actor = actor_info(&snapshot, &cfg.game_root);
                }
                snapshot.pid
            } else {
                last_pid = None;
                local_actor = None;
                lobby_state = None;
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
            let visible_actor = if lobby_state == Some(LobbyState::Game)
                && (cfg.show_character || cfg.show_stats)
            {
                local_actor.as_ref()
            } else {
                None
            };
            let next = format_activity_with_lobby(
                &cfg,
                running_game,
                visible_actor,
                if running_game.is_some() {
                    session_start
                } else {
                    manager_start
                },
                lobby_state,
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
            // Menus/character selection are stable for seconds; more frequent
            // polling shortens incorrect status displays after transitions.
            thread::sleep(Duration::from_secs(3));
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

    #[test]
    fn lobby_state_values_match_observed_engine_states() {
        for (raw, state, presence) in [
            (0, LobbyState::Menu, "In Menus"),
            (1, LobbyState::Initializing, "Connecting"),
            (2, LobbyState::FindGame, "Finding a Match"),
            (3, LobbyState::JoinSession, "Joining Session"),
            (4, LobbyState::Lobby, "In Lobby"),
            (5, LobbyState::PreRound, "Selecting Character"),
            (6, LobbyState::Game, "In Match"),
            (10, LobbyState::Leaving, "Leaving Match"),
        ] {
            assert_eq!(LobbyState::from_raw(raw), Some(state));
            assert_eq!(state.presence(), presence);
        }
        for raw in [7, 8, 9, 11, u32::MAX] {
            assert_eq!(LobbyState::from_raw(raw), None);
        }
    }

    #[test]
    fn lobby_pointer_chains_are_independent_and_fail_closed() {
        let base = 0x140000000usize;
        let (primary_rva, primary_off) = LOBBY_POINTER_PATHS[0];
        let (fallback_rva, fallback_off) = LOBBY_POINTER_PATHS[1];
        let primary_parent = 0x22000000usize;
        let fallback_parent = 0x33000000usize;
        let lobby = 0x44000000usize;
        let mut pointers = std::collections::HashMap::new();
        pointers.insert(base + primary_rva, primary_parent);
        pointers.insert(primary_parent + primary_off, lobby);
        pointers.insert(base + fallback_rva, fallback_parent);
        pointers.insert(fallback_parent + fallback_off, lobby);
        pointers.insert(lobby, base + LOBBY_VTABLE_RVA);
        let read = |at: usize| pointers.get(&at).copied();
        let state_at = |at: usize| (at == lobby + LOBBY_STATE_OFFSET).then_some(5u32);
        assert_eq!(
            resolve_lobby_state(base, read, state_at),
            Some(LobbyState::PreRound)
        );
        // A broken primary chain must still permit the validated fallback.
        pointers.remove(&(primary_parent + primary_off));
        assert_eq!(
            resolve_lobby_state(base, |at| pointers.get(&at).copied(), state_at),
            Some(LobbyState::PreRound)
        );
        // A bogus vtable, or unknown state, must never be treated as in-match.
        pointers.insert(lobby, base + 0x1234);
        assert_eq!(
            resolve_lobby_state(base, |at| pointers.get(&at).copied(), state_at),
            None
        );
        pointers.insert(lobby, base + LOBBY_VTABLE_RVA);
        assert_eq!(
            resolve_lobby_state(
                base,
                |at| pointers.get(&at).copied(),
                |at| { (at == lobby + LOBBY_STATE_OFFSET).then_some(87) }
            ),
            None
        );
    }

    #[test]
    fn non_game_lobby_state_never_leaks_character_or_stats() {
        let actor = LocalActor {
            name: Some("Goliath".into()),
            verified_monster: true,
            evolution: Some("2".into()),
            evolution_percentage: Some(75),
            health: Some("9000".into()),
            armor: Some("1000".into()),
        };
        for (state, label) in [
            (Some(LobbyState::Menu), "In Menus"),
            (Some(LobbyState::FindGame), "Finding a Match"),
            (Some(LobbyState::Lobby), "In Lobby"),
            (Some(LobbyState::PreRound), "Selecting Character"),
            (None, "Activity unavailable"),
        ] {
            let activity =
                format_activity_with_lobby(&sample(), Some(12), Some(&actor), 100, state).unwrap();
            assert_eq!(activity["details"], "Playing Evolve");
            assert_eq!(activity["state"], label);
            assert!(activity["assets"].get("large_image").is_none());
            assert!(!activity.to_string().contains("Health"));
            assert!(!activity.to_string().contains("Evolution"));
        }
        let activity = format_activity_with_lobby(
            &sample(),
            Some(12),
            Some(&actor),
            100,
            Some(LobbyState::Game),
        )
        .unwrap();
        assert_eq!(activity["details"], "Playing as Goliath · Stage 2");
        assert!(activity["state"].as_str().unwrap().contains("Health"));
    }

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
            manager_page: Page::Files,
            asset_name: None,
            mod_profile: Some("My Mod".into()),
            map: None,
            game_mode: None,
        }
    }
    #[test]
    fn manager_tab_art_matches_all_navigation_icons() {
        for (page, asset, tooltip) in [
            (Page::Home, "tab_home", "Home"),
            (Page::Docs, "tab_instructions", "Instructions"),
            (Page::AssetLibrary, "tab_asset_library", "Asset Library"),
            (Page::Files, "tab_modding", "Modding"),
            (Page::WhatsNew, "tab_whats_new", "What's New"),
            (Page::Credits, "tab_credits", "Credits"),
            (Page::Developer, "tab_developer", "Developer"),
            (Page::Models, "tab_models", "Model Viewer"),
            (Page::Levels, "tab_levels", "Level Editor"),
            (Page::Settings, "tab_settings", "Settings"),
        ] {
            assert_eq!(manager_tab_art(page), (asset, tooltip));
            let mut cfg = sample();
            cfg.manager_page = page;
            let activity = format_activity(&cfg, None, None, 100).unwrap();
            assert_eq!(activity["assets"]["large_image"], "evolve");
            assert_eq!(activity["assets"]["small_image"], asset);
            assert_eq!(activity["assets"]["small_text"], tooltip);
        }
    }

    #[test]
    fn presence_reports_only_active_selected_assets_and_keeps_game_art_separate() {
        let mut state = State {
            page: Some(Page::AssetLibrary),
            ..State::default()
        };
        // No selection -> don't describe an asset that isn't being viewed.
        assert_eq!(Config::from_state(&state).asset_name, None);
        state.page = Some(Page::Files);
        assert_eq!(Config::from_state(&state).asset_name, None);
        state.page = Some(Page::Models);
        state.input = r"C:\Users\Someone\my secret files\old.gfx".into();
        // Typed file name alone is not proof that the user is viewing it.
        assert_eq!(Config::from_state(&state).asset_name, None);

        let mut cfg = sample();
        cfg.manager_page = Page::Models;
        cfg.asset_name = Some("Objects/Monsters/Goliath/Goliath.cdf".into());
        cfg.manager_action = "Model Viewer".into();
        let activity = format_activity(&cfg, None, None, 100).unwrap();
        assert_eq!(
            activity["state"],
            "Model Viewer · Objects/Monsters/Goliath/Goliath.cdf"
        );
        assert_eq!(activity["assets"]["small_image"], "tab_models");

        // Even if an asset is visible in the manager, the in-game app keeps
        // the existing character portrait without a manager tab badge.
        let actor = LocalActor {
            name: Some("Goliath".into()),
            verified_monster: true,
            ..LocalActor::default()
        };
        let game = format_activity(&cfg, Some(900), Some(&actor), 100).unwrap();
        assert_eq!(game["assets"]["large_image"], "goliath");
        assert!(game["assets"].get("small_image").is_none());
        cfg.minimal = true;
        let privacy = format_activity(&cfg, None, None, 100).unwrap();
        assert_eq!(privacy["state"], "In manager");
        assert!(!privacy.to_string().contains("Goliath.cdf"));
    }

    #[test]
    fn gameplay_shows_verified_character_name_and_monster_stats() {
        let actor = LocalActor {
            name: Some("Goliath".into()),
            verified_monster: true,
            evolution: Some("Stage 2".into()),
            evolution_percentage: Some(50),
            health: Some("8500".into()),
            armor: Some("1000".into()),
        };
        let act = format_activity(&sample(), Some(100), Some(&actor), 100).unwrap();
        assert_eq!(act["details"], "Playing as Goliath · Stage 2");
        assert_eq!(act["state"], "Health 8,500 · Armor 1,000 · Evolution 50%");
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
            evolution_percentage: Some(50),
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
    fn monster_stage_three_keeps_stage_but_hides_percentage() {
        let actor = LocalActor {
            name: Some("Goliath".into()),
            verified_monster: true,
            evolution: Some("3".into()),
            evolution_percentage: Some(100), // stale input must not leak.
            health: Some("6000".into()),
            armor: Some("200".into()),
        };
        let activity = format_activity(&sample(), Some(100), Some(&actor), 100).unwrap();
        assert_eq!(activity["details"], "Playing as Goliath · Stage 3");
        assert_eq!(activity["state"], "Health 6,000 · Armor 200");
        assert!(!activity.to_string().contains("Evolution"));
    }

    #[test]
    fn monster_evolution_percentage_needs_stats_and_verified_progress() {
        let actor = LocalActor {
            name: Some("Wraith".into()),
            verified_monster: true,
            evolution: Some("2".into()),
            evolution_percentage: Some(0),
            ..LocalActor::default()
        };
        let activity = format_activity(&sample(), Some(1), Some(&actor), 100).unwrap();
        assert_eq!(activity["details"], "Playing as Wraith · Stage 2");
        assert_eq!(activity["state"], "Evolution 0%");
        let mut cfg = sample();
        cfg.show_stats = false;
        assert_eq!(
            format_activity(&cfg, Some(1), Some(&actor), 100).unwrap()["state"],
            "In match"
        );

        let unknown = LocalActor {
            evolution_percentage: None,
            ..actor
        };
        assert_eq!(
            format_activity(&sample(), Some(1), Some(&unknown), 100).unwrap()["state"],
            "In match"
        );
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
    fn registry_covers_all_39_playable_characters() {
        let portraits = character_portraits();
        assert_eq!(portraits.len(), 39);
        assert_eq!(
            portraits
                .iter()
                .filter(|p| !p.internal_names.is_empty())
                .count(),
            39
        );
        let mut seen = std::collections::HashSet::new();
        let mut unique_actor_names = std::collections::HashSet::new();
        for portrait in portraits {
            assert!(
                seen.insert(portrait.asset_key.as_str()),
                "Duplicate Discord asset key"
            );
            assert!(
                portrait.internal_names.len() <= 1,
                "Each playable character has exactly one canonical entity name"
            );
            for name in &portrait.internal_names {
                assert!(
                    unique_actor_names.insert(name.to_ascii_lowercase()),
                    "Playable entity name must not belong to multiple portraits"
                );
            }
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
            portrait_for_display_name("Kala").unwrap().team,
            Team::Hunter
        );
        assert_eq!(portrait_for_display_name("Kala").unwrap().asset_key, "tech");
        assert_eq!(
            character_from_entity_name("Tech"),
            Some(("Kala", Team::Hunter))
        );
        assert_eq!(
            character_from_entity_name("Scrap"),
            Some(("Jack", Team::Hunter))
        );
        assert_eq!(
            portrait_for_display_name("Jack").unwrap().asset_key,
            "scrap"
        );
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
            ("MedicParnellTankBot", "Parnell (Tank)"),
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
            ("MedicC_LowgravBot", "Lowgrav Caira"),
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
    fn coop_npc_and_minion_entities_do_not_resolve_as_playable_characters() {
        for excluded in &portrait_registry().non_player_entities {
            assert!(
                character_from_entity_name(excluded).is_none(),
                "NPC/minion/coop variant must not be treated as playable: {excluded}"
            );
        }
        // The confirmed player names are intentionally single-source.
        for identifier in ["TrackingGBot", "SupportGBot"] {
            assert_eq!(character_from_entity_name(identifier), None);
        }
    }

    #[test]
    fn jack_uses_scrap_internal_name_and_existing_transparent_portrait() {
        assert_eq!(
            character_from_entity_name("Scrap"),
            Some(("Jack", Team::Hunter))
        );
        let jack = portrait_for_display_name("Jack").unwrap();
        assert_eq!(jack.asset_key, "scrap");
        assert_eq!(jack.team, Team::Hunter);
        assert!(portrait_for_display_name("Necro Lazarus").is_none());
        assert_eq!(character_from_entity_name("MedicB_NecroBot"), None);
        let actor = LocalActor {
            name: Some("Jack".into()),
            health: Some("2000.000".into()),
            ..LocalActor::default()
        };
        let activity = format_activity(&sample(), Some(123), Some(&actor), 100).unwrap();
        assert_eq!(activity["details"], "Playing as Jack");
        assert_eq!(activity["state"], "Health 2,000");
        assert_eq!(activity["assets"]["large_image"], "scrap");
        assert_eq!(activity["assets"]["large_text"], "Jack");
        for npc in &portrait_registry().non_player_entities {
            assert!(
                character_from_entity_name(npc).is_none(),
                "Non-player name: {npc}"
            );
        }
    }

    #[test]
    fn game_activity_is_two_lines_with_compact_readable_numeric_values() {
        let actor = LocalActor {
            name: Some("Goliath".into()),
            verified_monster: true,
            evolution: Some("1".into()),
            evolution_percentage: Some(75),
            health: Some("10400.000".into()),
            armor: Some("2343.978".into()),
        };
        let activity = format_activity(&sample(), Some(123), Some(&actor), 100).unwrap();
        assert_eq!(activity["details"], "Playing as Goliath · Stage 1");
        assert_eq!(
            activity["state"],
            "Health 10,400 · Armor 2,344 · Evolution 75%"
        );
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
        assert_eq!(
            hidden["state"],
            "Health 10,400 · Armor 2,344 · Evolution 75%"
        );
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
