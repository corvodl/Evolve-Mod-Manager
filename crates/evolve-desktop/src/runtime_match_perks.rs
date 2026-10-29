//! Read-only match loadouts for the password-gated Developer page.
//!
//! The game logs a character and its three selected perks together at round
//! start. Log entries alone can be stale, so compare them with the currently
//! spawned actor and perk entities before presenting them as a live roster.
use crate::{runtime_actors, runtime_memory};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const LOG_TAIL_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct Player {
    pub slot: String,
    pub class: String,
    pub character: String,
    pub display: String,
    pub perks: Vec<String>,
    pub actor_id: Option<u32>,
    pub local: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Listing {
    pub players: Vec<Player>,
    pub status: String,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn slot(value: &str) -> bool {
    (value.starts_with("PlayerSlot_merc_") || value.starts_with("PlayerSlot_monster_"))
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && value.len() <= 48
}

fn log_message(line: &str) -> &str {
    line.trim_start()
        .strip_prefix('<')
        .and_then(|line| line.split_once('>'))
        .map_or(line, |(_, rest)| rest)
        .trim()
}

#[derive(Default)]
struct Pending {
    slot: String,
    class: String,
    character: String,
    display: String,
    perks: Option<Vec<String>>,
}

impl Pending {
    fn finish(self) -> Option<Player> {
        let perks = self.perks?;
        (slot(&self.slot)
            && identifier(&self.class)
            && identifier(&self.character)
            && !self.display.is_empty()
            && self.display.len() <= 80)
            .then_some(Player {
                slot: self.slot,
                class: self.class,
                character: self.character,
                display: self.display,
                perks,
                actor_id: None,
                local: false,
            })
    }
}

/// Keep only the last complete contiguous PlayerSlot block. Other log lines
/// (including older rounds and untrusted game/mod output) cannot join blocks.
fn parse_latest_roster(log: &str) -> Option<Vec<Player>> {
    let mut current = Vec::new();
    let mut latest = None;
    let mut pending: Option<Pending> = None;
    let mut in_block = false;
    for line in log.lines() {
        let message = log_message(line);
        if let Some(name) = message.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
            && slot(name)
        {
            if !in_block {
                // A new roster, even an incomplete one, invalidates the old
                // round. Never present last match's perks as the current match.
                latest = None;
                current.clear();
            }
            if let Some(entry) = pending.take().and_then(Pending::finish) {
                current.push(entry);
            } else if in_block {
                current.clear();
            }
            // A repeated slot begins a new round, even if another log line
            // between rounds was lost from the bounded tail.
            if current.iter().any(|entry: &Player| entry.slot == name) {
                current.clear();
            }
            pending = Some(Pending {
                slot: name.to_owned(),
                ..Pending::default()
            });
            in_block = true;
            continue;
        }
        if let Some(entry) = pending.as_mut() {
            if let Some(value) = message.strip_prefix("class:") {
                entry.class = value.trim().to_owned();
            } else if let Some(value) = message.strip_prefix("character:") {
                let value = value.trim();
                if let Some((id, display)) = value.split_once(" - ") {
                    entry.character = id.trim().to_owned();
                    entry.display = display.trim().to_owned();
                }
            } else if let Some(value) = message.strip_prefix("perks:") {
                let values: Vec<_> = value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .collect();
                if values.len() <= 3 && values.iter().all(|value| identifier(value)) {
                    entry.perks = Some(values.into_iter().map(str::to_owned).collect());
                }
            } else if !(message.starts_with("team:")
                || message.starts_with("stage:")
                || message.starts_with("inWorld:")
                || message.starts_with("user:"))
            {
                if let Some(entry) = pending.take().and_then(Pending::finish) {
                    current.push(entry);
                }
                if !current.is_empty() {
                    latest = Some(std::mem::take(&mut current));
                }
                in_block = false;
            }
        }
    }
    if let Some(entry) = pending.and_then(Pending::finish) {
        current.push(entry);
    }
    if !current.is_empty() {
        latest = Some(current);
    }
    latest.filter(|entries| entries.len() <= 32 && entries.iter().any(|p| p.class == "MONSTER"))
}

fn read_latest_roster(snapshot: &runtime_memory::Snapshot) -> Result<Vec<Player>, String> {
    let executable = snapshot
        .executable
        .strip_prefix("Actual running executable: ")
        .ok_or("Running executable path unavailable; cannot identify its Game.log")?;
    let game_root = Path::new(executable)
        .parent()
        .and_then(Path::parent)
        .ok_or("Running game directory unavailable")?;
    let log_path = game_root.join("Game.log");
    let mut file = std::fs::File::open(&log_path)
        .map_err(|e| format!("Cannot read running game's Game.log: {e}"))?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let start = len.saturating_sub(LOG_TAIL_LIMIT);
    file.seek(SeekFrom::Start(start))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(LOG_TAIL_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if start != 0 {
        let first = bytes
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |i| i + 1);
        bytes.drain(..first);
    }
    let log = String::from_utf8_lossy(&bytes);
    parse_latest_roster(&log)
        .ok_or_else(|| "No complete round-start perk roster in Game.log tail".into())
}

fn actor_matches(name: &str, character: &str) -> bool {
    let Some(rest) = name.strip_prefix(character) else {
        return false;
    };
    rest.is_empty()
        || rest
            .strip_prefix("Bot")
            .is_some_and(|suffix| suffix.bytes().all(|b| b.is_ascii_digit()))
        || (!rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

fn join_actors(players: &mut [Player], actors: &[runtime_actors::Actor]) -> bool {
    let character_counts = counts(players.iter().map(|player| player.character.clone()));
    let mut local_found = false;
    for player in players {
        if character_counts.get(&player.character) != Some(&1)
            || players_duplicate_character(actors, &player.character)
        {
            continue;
        }
        if let Some(actor) = actors
            .iter()
            .find(|actor| actor_matches(&actor.name, &player.character))
        {
            player.actor_id = Some(actor.id);
            player.local = actor.local;
            local_found |= actor.local;
        }
    }
    local_found
}

fn players_duplicate_character(actors: &[runtime_actors::Actor], character: &str) -> bool {
    actors
        .iter()
        .filter(|actor| actor_matches(&actor.name, character))
        .take(2)
        .count()
        > 1
}

fn counts(names: impl IntoIterator<Item = String>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for name in names {
        *counts.entry(name).or_default() += 1;
    }
    counts
}

#[cfg(windows)]
mod windows {
    use super::*;
    use read_process_memory::{ProcessHandle, copy_address};

    fn bytes(h: &ProcessHandle, at: usize, size: usize) -> Option<Vec<u8>> {
        (0x10000..0x0000_7FFF_FFFF_FFFF)
            .contains(&at)
            .then(|| copy_address(at, size, h).ok())?
    }
    fn u32_at(h: &ProcessHandle, at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(bytes(h, at, 4)?.try_into().ok()?))
    }
    fn q(h: &ProcessHandle, at: usize) -> Option<usize> {
        let p = u64::from_le_bytes(bytes(h, at, 8)?.try_into().ok()?);
        (0x10000..0x0000_7FFF_FFFF_FFFF)
            .contains(&(p as usize))
            .then_some(p as usize)
    }
    fn perk_name(h: &ProcessHandle, entity: usize) -> Option<String> {
        // The definition gives a type name, rather than the instance name's
        // spawn serial. Do not inspect pooled entity+0xE8 name nodes.
        let definition = q(h, entity.checked_add(0x30)?)?;
        let name = q(h, q(h, definition.checked_add(0x18)?)?.checked_add(0x20)?)?;
        let bytes = bytes(h, name, 96)?;
        let len = bytes.iter().position(|&b| b == 0)?;
        let name = std::str::from_utf8(&bytes[..len]).ok()?;
        ((name.starts_with("Perk") || name.starts_with("MPerk")) && identifier(name))
            .then(|| name.to_owned())
    }

    fn live_perks(h: &ProcessHandle, table: usize) -> Result<Vec<String>, String> {
        let mut perks = Vec::new();
        let mut occupied = 0usize;
        for start in (0..65536).step_by(512) {
            let chunk =
                bytes(h, table + start * 8, 4096).ok_or("Entity table became unreadable")?;
            for (i, ptr) in chunk.as_chunks::<8>().0.iter().enumerate() {
                let entity = u64::from_le_bytes(*ptr) as usize;
                if !(0x10000..0x0000_7FFF_FFFF_FFFF).contains(&entity) {
                    continue;
                }
                let Some(id) = entity.checked_add(0x18).and_then(|at| u32_at(h, at)) else {
                    continue;
                };
                if id == 0 || id as usize & 0xFFFF != start + i {
                    continue;
                }
                occupied += 1;
                if occupied > 8192 {
                    return Err("Entity roster exceeds bounded perk scan".into());
                }
                if let Some(name) = perk_name(h, entity) {
                    perks.push(name);
                }
            }
        }
        Ok(perks)
    }

    pub(crate) fn inspect(
        snapshot: &runtime_memory::Snapshot,
        root: &str,
        actors: &[runtime_actors::Actor],
    ) -> Result<Listing, String> {
        let pid = snapshot.pid.ok_or("Detect Evolve.exe first")?;
        let base = snapshot
            .module_base
            .ok_or("Running module base unavailable")?;
        let mut players = read_latest_roster(snapshot)?;
        if !join_actors(&mut players, actors) {
            return Err("Round-start roster does not match the current local actor".into());
        }
        let h: ProcessHandle = pid
            .try_into()
            .map_err(|e| format!("Cannot read process: {e}"))?;
        let entity_system = q(
            &h,
            base.checked_add(0x28D58E8).ok_or("Module base overflow")?,
        )
        .ok_or("Entity system unavailable")?;
        let table = q(&h, entity_system + 0xA8).ok_or("Entity table unavailable")?;
        let expected = counts(players.iter().flat_map(|p| p.perks.iter().cloned()));
        let observed = counts(live_perks(&h, table)?);
        if expected != observed {
            return Err(
                "Round-start perks do not match live perk entities; refresh after the match spawns"
                    .into(),
            );
        }
        let now = runtime_memory::inspect(root);
        if now.pid != Some(pid) || now.module_base != Some(base) {
            return Err("Evolve process changed during match roster scan".into());
        }
        let matched = players.iter().filter(|p| p.actor_id.is_some()).count();
        Ok(Listing {
            status: format!(
                "{} round-start loadout(s) checked against the live perk inventory; {matched} actor(s) identified. Per-character attribution comes from Game.log, not an owner pointer.",
                players.len()
            ),
            players,
        })
    }
}

#[cfg(windows)]
pub(super) use windows::inspect;
#[cfg(not(windows))]
pub(super) fn inspect(
    _: &runtime_memory::Snapshot,
    _: &str,
    _: &[runtime_actors::Actor],
) -> Result<Listing, String> {
    Err("Match perk inspection requires Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "<21:03:09> [PlayerSlot_merc_00]\n<21:03:09>   team:       merc\n<21:03:09>   class:      ASSAULT\n<21:03:09>   character:  AssaultAbeRenegade - AbeRenegade\n<21:03:09>   stage:      0\n<21:03:09>   perks:      PerkMovementSpeedLow2, PerkJetpackRechargeMedium1, PerkJetpackThrustHigh2, \n<21:03:09>   inWorld:    <null>\n<21:03:09> [PlayerSlot_monster_00]\n<21:03:09>   class:      MONSTER\n<21:03:09>   character:  RockyBobcicle - RockyBobcicle\n<21:03:09>   perks:      MPerkSavageNatureLow3, MPerkSavageNatureMedium3, MPerkSavageNatureHigh3, \n<21:03:09>   user:       user_Corvo\n<21:03:09> Starting Versus Screen\n";

    #[test]
    fn latest_round_keeps_perks_with_their_character() {
        let log = format!("{LOG}<21:12:00> other event\n{LOG}");
        let players = parse_latest_roster(&log).unwrap();
        assert_eq!(players.len(), 2);
        assert_eq!(players[0].character, "AssaultAbeRenegade");
        assert_eq!(players[0].perks[1], "PerkJetpackRechargeMedium1");
        assert_eq!(players[1].perks[2], "MPerkSavageNatureHigh3");
    }

    #[test]
    fn whole_hunt_roster_keeps_hunter_perks_separate_from_monster_perks() {
        let log = "<21:03:09> [PlayerSlot_merc_00]\n<21:03:09> class: ASSAULT\n<21:03:09> character: AssaultAbeRenegade - AbeRenegade\n<21:03:09> perks: PerkMovementSpeedLow2, PerkJetpackRechargeMedium1, PerkJetpackThrustHigh2,\n<21:03:09> [PlayerSlot_merc_01]\n<21:03:09> class: TRACKING\n<21:03:09> character: TrackingA - Griffin\n<21:03:09> perks: PerkHealthRegenLow1, PerkClassCooldownMedium2, PerkHealthRegenHigh2,\n<21:03:09> [PlayerSlot_merc_02]\n<21:03:09> class: MEDIC\n<21:03:09> character: MedicG - Emet\n<21:03:09> perks: PerkQuickSwitchLow3, PerkImprovedRegenerationMedium1, PerkRocketManHigh3,\n<21:03:09> [PlayerSlot_merc_03]\n<21:03:09> class: SUPPORT\n<21:03:09> character: SupportE - Cabot\n<21:03:09> perks: PerkJumpHeightLow3, PerkJetpackThrustMedium1, PerkDamageReductionHigh3,\n<21:03:09> [PlayerSlot_monster_00]\n<21:03:09> class: MONSTER\n<21:03:09> character: RockyBobcicle - RockyBobcicle\n<21:03:09> perks: MPerkSavageNatureLow3, MPerkSavageNatureMedium3, MPerkSavageNatureHigh3,\n<21:03:09> Starting Versus Screen\n";
        let players = parse_latest_roster(log).unwrap();
        assert_eq!(players.len(), 5);
        assert_eq!(players.iter().map(|p| p.perks.len()).sum::<usize>(), 15);
        assert_eq!(players[2].display, "Emet");
        assert_eq!(players[3].perks[2], "PerkDamageReductionHigh3");
        assert_eq!(players[4].perks[0], "MPerkSavageNatureLow3");
    }

    #[test]
    fn partial_and_malformed_rounds_do_not_inherit_old_perks() {
        assert!(
            parse_latest_roster("<1> [PlayerSlot_monster_00]\n<1> perks: MPerkOld3,\n").is_none()
        );
        let log = format!(
            "{LOG}<21:13:00> [PlayerSlot_monster_00]\n<21:13:00> class: MONSTER\n<21:13:00> character: RockyBobcicle - RockyBobcicle\n<21:13:00> perks: ../../unsafe,\n"
        );
        // The previous complete round must not silently stand in for a newer,
        // incomplete one; live entity comparison is an additional guard.
        assert!(parse_latest_roster(&log).is_none());
    }

    #[test]
    fn actor_match_rejects_prefixes_and_accepts_bot_spawn_serial() {
        assert!(actor_matches("RockyBobcicleBot", "RockyBobcicle"));
        assert!(actor_matches("RockyBobcicleBot427", "RockyBobcicle"));
        assert!(!actor_matches("RockyBobcicleVariant", "RockyBobcicle"));
    }

    #[test]
    fn only_unique_actor_names_can_identify_a_slot() {
        let actor = |id, name: &str, local| runtime_actors::Actor {
            id,
            address: 0x10000 + id as usize,
            entity: 0x20000 + id as usize,
            name: name.to_owned(),
            local,
            fields: Vec::new(),
        };
        let mut players = parse_latest_roster(LOG).unwrap();
        assert!(join_actors(
            &mut players,
            &[
                actor(1, "RockyBobcicleBot", true),
                actor(2, "AssaultAbeRenegadeBot", false)
            ]
        ));
        assert_eq!(players[0].actor_id, Some(2));
        assert_eq!(players[1].actor_id, Some(1));
        assert!(players[1].local);

        let mut players = parse_latest_roster(LOG).unwrap();
        assert!(!join_actors(
            &mut players,
            &[
                actor(1, "RockyBobcicleBot", true),
                actor(2, "RockyBobcicleBot2", false)
            ]
        ));
        assert!(players[1].actor_id.is_none());
    }

    #[test]
    fn multiset_comparison_detects_duplicate_or_missing_perks() {
        assert_ne!(
            counts(["PerkA".into(), "PerkA".into()]),
            counts(["PerkA".into()])
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an opted-in, running Evolve match and its Game.log"]
    fn live_match_roster_matches_spawned_entities() {
        let root = std::env::var("EVOLVE_LIVE_MATCH_ROOT").expect("set EVOLVE_LIVE_MATCH_ROOT");
        let snapshot = runtime_memory::inspect(&root);
        let actors = runtime_actors::inspect(&snapshot, &root).unwrap();
        let roster = inspect(&snapshot, &root, &actors.actors).unwrap();
        assert!(roster.players.iter().any(|player| player.local));
        assert!(roster.players.iter().all(|player| player.perks.len() <= 3));
        eprintln!("{}", roster.status);
    }
}
