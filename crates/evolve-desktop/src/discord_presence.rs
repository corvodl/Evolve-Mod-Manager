//! Optional, local-only Discord Rich Presence. No OAuth, tokens, network API,
//! injection, game hooks, or dependency on launching Evolve.
//! Registered Discord developer application ID is supplied by the release build
//! (EVOLVE_DISCORD_APP_ID) or a local developer override in Integrations.
use crate::{Page, State};

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
            state.render_source.as_ref().and_then(|file| file.file_stem())
                .or_else(|| std::path::Path::new(&state.input).file_stem())
                .map(|name| name.to_string_lossy().into_owned())
        } else { None };
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
        }.to_owned();
        let enabled: Vec<_> = state.modding_projects.as_ref().into_iter()
            .flatten().filter(|p| p.enabled).collect();
        let mod_profile = match enabled.as_slice() {
            [] => None,
            [one] => Some(one.label.clone()),
            many => Some(format!("{} enabled mod projects", many.len())),
        };
        let use_test = p.play_through_test_client && !p.test_game_installation.is_empty();
        let game_root = if use_test { &p.test_game_installation } else { &p.game_installation };
        let game_version = if use_test { "Stage 2 • Test Client" } else { "Stage 2" }.to_owned();
        Self {
            enabled: p.discord_presence_enabled,
            app_id: if p.discord_application_id.is_empty() {
                option_env!("EVOLVE_DISCORD_APP_ID").unwrap_or("").to_owned()
            } else { p.discord_application_id.clone() },
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
    pub evolution: Option<String>,
    pub health: Option<String>,
    pub armor: Option<String>,
}

fn label(input: &str) -> String {
    input.chars().filter(|c| !c.is_control()).take(128).collect()
}
fn character_asset(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "goliath" => Some("goliath"),
        "meteor goliath" => Some("meteor_goliath"),
        "kraken" => Some("kraken"),
        "elder kraken" => Some("elder_kraken"),
        "wraith" => Some("wraith"),
        "behemoth" => Some("behemoth"),
        "gorgon" => Some("gorgon"),
        "maggie" => Some("maggie"),
        "markov" => Some("markov"),
        "val" => Some("val"),
        "hank" => Some("hank"),
        "lazarus" => Some("lazarus"),
        "hyde" => Some("hyde"),
        "parnell" => Some("parnell"),
        "cabot" => Some("cabot"),
        "caira" => Some("caira"),
        "slim" => Some("slim"),
        "sunny" => Some("sunny"),
        "emet" => Some("emet"),
        "kala" => Some("kala"),
        "jack" => Some("jack"),
        "crow" => Some("crow"),
        "griffin" => Some("griffin"),
        "abe" => Some("abe"),
        "bucket" => Some("bucket"),
        _ => None,
    }
}
fn format_activity(cfg: &Config, game_pid: Option<u32>, actor: Option<&LocalActor>, start: i64) -> Option<serde_json::Value> {
    use serde_json::json;
    let game = game_pid.is_some() && cfg.show_game;
    if !cfg.enabled || (!game && !cfg.show_manager) { return None; }
    let (details, state, small_image) = if game {
        if cfg.minimal {
            ("Playing Modded Evolve".to_owned(), "In game".to_owned(), None)
        } else {
            let name = actor.and_then(|a| a.name.as_ref()).filter(|_| cfg.show_character);
            let details = name.map_or_else(|| "Playing Evolve".to_owned(), |n| format!("Playing as {n}"));
            let mut values = vec![cfg.game_version.clone()];
            if cfg.show_character {
                if let Some(stage) = actor.and_then(|a| a.evolution.as_ref()) {
                    values.push(format!("Evolution {stage}"));
                }
            }
            if let Some(mode) = cfg.game_mode.as_ref() { values.push(mode.clone()); }
            if let Some(map) = cfg.map.as_ref() { values.push(map.clone()); }
            if cfg.show_stats {
                if let Some(h) = actor.and_then(|a| a.health.as_ref()) { values.push(format!("HP {h}")); }
                if let Some(a) = actor.and_then(|a| a.armor.as_ref()) { values.push(format!("Armor {a}")); }
            }
            if cfg.show_profile {
                if let Some(profile) = cfg.mod_profile.as_ref() { values.push(profile.clone()); }
            }
            (details, values.join(" • "), name.and_then(|n| character_asset(n)))
        }
    } else {
        let details = if cfg.minimal { "Using Evolve Mod Manager".to_owned() }
            else { cfg.manager_action.clone() };
        let state = if cfg.minimal { "Modding Evolve".to_owned() }
            else if let Some(name) = cfg.model_name.as_ref() {
                format!("Viewing {name}")
            } else if cfg.show_profile {
                cfg.mod_profile.clone().unwrap_or_else(|| "Modded Evolve".into())
            } else { "Modded Evolve".to_owned() };
        (details, state, None)
    };
    let mut assets = json!({"large_image":"evolve", "large_text":"Evolve Mod Manager"});
    if let Some(asset) = small_image {
        assets["small_image"] = json!(asset);
        assets["small_text"] = json!(actor.and_then(|a| a.name.as_deref()).unwrap_or(""));
    }
    Some(json!({
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
    use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn epoch() -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH)
            .unwrap_or_default().as_secs() as i64
    }
    fn frame(pipe: &mut File, opcode: u32, data: &serde_json::Value) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(data).map_err(std::io::Error::other)?;
        pipe.write_all(&opcode.to_le_bytes())?;
        pipe.write_all(&(bytes.len() as u32).to_le_bytes())?;
        pipe.write_all(&bytes)?;
        pipe.flush()
    }
    fn receive(pipe: &mut File) -> std::io::Result<(u32, serde_json::Value)> {
        let mut header = [0u8;8];
        pipe.read_exact(&mut header)?;
        let opcode = u32::from_le_bytes(header[..4].try_into().unwrap());
        let size = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
        if size > 1_048_576 { return Err(std::io::Error::other("Discord RPC response too large")); }
        let mut bytes = vec![0;size];
        pipe.read_exact(&mut bytes)?;
        let json = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        Ok((opcode,json))
    }
    fn connect(app_id: &str) -> std::io::Result<File> {
        for i in 0..10 {
            let path = format!(r"\\.\pipe\discord-ipc-{i}");
            if let Ok(mut pipe) = OpenOptions::new().read(true).write(true).open(path) {
                frame(&mut pipe, 0, &serde_json::json!({"v":1,"client_id":app_id}))?;
                let (opcode, response) = receive(&mut pipe)?;
                if opcode == 1 && response["evt"] != "ERROR" { return Ok(pipe); }
            }
        }
        Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "Discord desktop IPC unavailable"))
    }
    fn activity(pipe:&mut File, value:Option<serde_json::Value>, nonce:u64) -> std::io::Result<()> {
        frame(pipe, 1, &serde_json::json!({
            "cmd":"SET_ACTIVITY",
            "args":{"pid":std::process::id(),"activity":value},
            "nonce":nonce.to_string()
        }))?;
        // Discord responds once per command; drain the acknowledgement to avoid
        // exhausting the named-pipe buffer during long sessions.
        let (opcode, response) = receive(pipe)?;
        if opcode != 1 || response["evt"] == "ERROR" {
            return Err(std::io::Error::other("Discord rejected Rich Presence update"));
        }
        Ok(())
    }
    fn actor_info(snapshot:&runtime_memory::Snapshot, root:&str) -> Option<LocalActor> {
        let listing = runtime_actors::inspect(snapshot, root).ok()?;
        let actor = listing.actors.iter().find(|actor| actor.local)?;
        let mut info = LocalActor::default();
        if !actor.name.starts_with("Actor entity") {
            info.name = Some(actor.name.clone());
        }
        for field in &actor.fields {
            use runtime_actors::Field;
            match field.field {
                Field::Health => info.health = Some(field.display.clone()),
                Field::Armor => info.armor = Some(field.display.clone()),
                Field::Stage => info.evolution = Some(format!("Stage {}",field.display)),
                _ => {}
            }
        }
        Some(info)
    }
    pub(super) fn worker(shared:Arc<Mutex<Config>>, running:Arc<AtomicBool>) {
        let mut pipe: Option<File> = None;
        let mut app_id = String::new();
        let mut last_activity: Option<serde_json::Value> = None;
        let mut last_pid = None;
        let mut session_start = epoch();
        let manager_start = session_start;
        let mut local_actor:Option<LocalActor> = None;
        let mut last_actor_check = Instant::now() - Duration::from_secs(60);
        let mut nonce = 1u64;
        while running.load(Ordering::Relaxed) {
            let cfg = shared.lock().map(|s| s.clone()).unwrap_or_else(|e| e.into_inner().clone());
            if !cfg.enabled || !cfg.app_id.bytes().all(|b| b.is_ascii_digit())
                || cfg.app_id.len() < 15 || cfg.app_id.len() > 25 {
                if let Some(mut connected) = pipe.take() {
                    let _ = activity(&mut connected,None,nonce);
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
                if snapshot.pid.is_some() && cfg.show_character
                    && last_actor_check.elapsed() >= Duration::from_secs(20) {
                    last_actor_check = Instant::now();
                    local_actor = actor_info(&snapshot,&cfg.game_root);
                }
                snapshot.pid
            } else {
                last_pid = None;
                local_actor = None;
                None
            };
            if pipe.is_none() || app_id != cfg.app_id {
                pipe = connect(&cfg.app_id).ok();
                app_id = cfg.app_id.clone();
                last_activity = None;
            }
            let visible_actor = if cfg.show_character { local_actor.as_ref() } else { None };
            let next = format_activity(&cfg,running_game,visible_actor,if running_game.is_some() {session_start} else {manager_start});
            if next != last_activity {
                if let Some(ref mut connected) = pipe {
                    match activity(connected,next.clone(),nonce) {
                        Ok(()) => last_activity = next,
                        Err(_) => {pipe = None; last_activity = None;}
                    }
                    nonce += 1;
                }
            }
            thread::sleep(Duration::from_secs(8));
        }
        if let Some(mut connected) = pipe {
            let _ = activity(&mut connected,None,nonce);
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
    pub fn start(initial:Config) -> Self {
        #[cfg(windows)] {
            let shared = std::sync::Arc::new(std::sync::Mutex::new(initial));
            let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            let thread_shared = shared.clone();
            let thread_running = running.clone();
            // All pipe IO, process enumeration and read-only actor detection
            // remain on an independent thread, never the GUI or launch worker.
            let _ = std::thread::Builder::new().name("discord-presence".into())
                .spawn(move || windows::worker(thread_shared,thread_running));
            return Self {shared,running};
        }
        #[cfg(not(windows))] {
            let _ = initial;
            Self {}
        }
    }
    pub fn publish(&self, next:Config) {
        #[cfg(windows)] {
            if let Ok(mut guard) = self.shared.lock() {*guard = next;}
        }
        #[cfg(not(windows))] let _ = next;
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        #[cfg(windows)]
        self.running.store(false,std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Config {
        Config {
            enabled:true,app_id:"123456789012345678".into(),
            show_game:true,show_manager:true,show_character:true,show_stats:true,
            show_profile:true,minimal:false,game_root:String::new(),
            game_version:"Stage 2".into(),manager_action:"Modding Evolve".into(),
            model_name:None,mod_profile:Some("My Mod".into()),map:None,game_mode:None,
        }
    }
    #[test]
    fn activity_uses_validated_actor_fields_and_never_guesses_map() {
        let actor = LocalActor{name:Some("Goliath".into()),evolution:Some("Stage 2".into()),
            health:Some("8500".into()),armor:Some("1000".into())};
        let act = format_activity(&sample(),Some(100),Some(&actor),100).unwrap();
        assert_eq!(act["details"],"Playing as Goliath");
        assert!(act["state"].as_str().unwrap().contains("HP 8500"));
        assert!(act["state"].as_str().unwrap().contains("Evolution Stage 2"));
        assert!(!act["state"].as_str().unwrap().contains("Map:"));
        assert_eq!(act["assets"]["small_image"],"goliath");
    }
    #[test]
    fn privacy_mode_hides_character_stats_and_mods() {
        let mut cfg = sample();
        cfg.minimal = true;
        let actor=LocalActor{name:Some("Wraith".into()),health:Some("1".into()),..Default::default()};
        let act=format_activity(&cfg,Some(123),Some(&actor),100).unwrap();
        assert!(!act.to_string().contains("Wraith"));
        assert!(!act.to_string().contains("My Mod"));
        assert!(!act.to_string().contains("HP"));
    }
    #[test]
    fn disabled_and_no_manager_fallback_clear_activity() {
        let mut cfg=sample();
        cfg.enabled=false;
        assert!(format_activity(&cfg,Some(1),None,1).is_none());
        cfg.enabled=true;
        cfg.show_manager=false;
        assert!(format_activity(&cfg,None,None,1).is_none());
    }
    #[test]
    fn labels_are_capped_to_discord_limits() {
        assert_eq!(label(&"a".repeat(140)).len(),128);
    }
}
