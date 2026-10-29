//! Read-only Stage 2 research inspection for the Developer > Actors section.
//! PR #135 documents observations; only build/identity-validated memory reads
//! are exposed here. This module never writes memory or invokes game methods.
use crate::{runtime_actors, runtime_memory};
use runtime_memory::Snapshot;

const ENTITY_SYSTEM_RVA: usize = 0x28D58E8;
const ENTITY_VTABLE_RVA: usize = 0x22DBC60;
const ACTOR_VTABLE_RVA: usize = 0x24B3790;
const INVENTORY_VTABLE_RVA: usize = 0x2263350;
const PROXY_VTABLE_RVA: usize = 0x22E2DE0;
const LIVING_VTABLE_RVA: usize = 0x2336010;
const MONSTER_ENERGY_KEY_RVA: usize = 0x28D5AE0;

/// Verified on the inspected Stage 2 runtime. The key changes with the process;
/// never persist these decoded values or treat them as directly writable floats.
fn monster_stage(word: u32) -> Option<u8> {
    match word & 0xFFFF {
        0x0101 => Some(1),
        0x0202 => Some(2),
        0x0303 => Some(3),
        _ => None,
    }
}

fn known_monster(name: &str) -> bool {
    // Match internal entity-name families, never the fallback "Actor entity ..."
    // label. This prevents hunter offsets being interpreted as monster energy.
    [
        "Goliath",
        "Kraken",
        "Grounder",
        "Teleporter",
        "Rocky",
        "Rockybobsicle",
        "Lurker",
        "Scorpid",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

fn evolution_current(raw: u32, key: u32) -> f32 {
    f32::from_bits((raw ^ key).rotate_right(9))
}
fn evolution_max(raw: u32, key: u32) -> f32 {
    f32::from_bits((raw ^ key).rotate_left(16))
}
fn traversal_current(raw: u32, key: u32) -> f32 {
    f32::from_bits(raw ^ key ^ 0x0093F6E4)
}
fn traversal_max(raw: u32, key: u32) -> f32 {
    f32::from_bits((raw ^ key).rotate_left(7))
}
fn bounded_resource(current: f32, maximum: f32) -> bool {
    current.is_finite()
        && maximum.is_finite()
        && (0.0..=maximum).contains(&current)
        && (0.0..=1000.0).contains(&maximum)
}

fn checked_evolution(
    stage_word: u32,
    current_raw: u32,
    max_raw: u32,
    key: u32,
) -> Result<(u8, f32, f32), &'static str> {
    let stage = monster_stage(stage_word).ok_or("Evolution stage unrecognized")?;
    let current = evolution_current(current_raw, key);
    let maximum = evolution_max(max_raw, key);
    let expected = match stage {
        1 => 24.0,
        2 => 28.0,
        3 => 0.0,
        _ => return Err("Unsupported evolution stage"),
    };
    if !bounded_resource(current, maximum) || maximum != expected {
        return Err("Energy decode/rules mismatch; no readout for this build");
    }
    Ok((stage, current, maximum))
}

fn checked_traversal(current_raw: u32, max_raw: u32, key: u32) -> Result<(f32, f32), &'static str> {
    let current = traversal_current(current_raw, key);
    let maximum = traversal_max(max_raw, key);
    if !bounded_resource(current, maximum) || maximum <= 0.0 {
        return Err("Traversal decode out of plausible bounds");
    }
    Ok((current, maximum))
}

/// Decoded, read-only snapshot of a validated local monster's evolution bar.
/// Stage 3 intentionally has a zero-length bar and no percentage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct EvolutionEnergy {
    pub stage: u8,
    pub current: f32,
    pub maximum: f32,
}
impl EvolutionEnergy {
    pub fn percentage(self) -> Option<u8> {
        if self.stage == 3 || !bounded_resource(self.current, self.maximum) || self.maximum <= 0.0 {
            return None;
        }
        Some(
            (self.current / self.maximum * 100.0)
                .round()
                .clamp(0.0, 100.0) as u8,
        )
    }
}

#[derive(Clone, Debug)]
pub(super) struct Inventory {
    pub actor_name: String,
    pub actor_id: u32,
    pub inventory_address: usize,
    pub item_ids: Vec<u32>,
    pub registered_ids: usize,
    pub loadout: Vec<(u32, u32)>,
    pub current: u32,
    pub previous: u32,
    pub selected_in_list: bool,
    pub actor_mirrors_match: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Report {
    pub pid: u32,
    pub image_sha256: String,
    pub base: usize,
    pub local_id: u32,
    pub inventory: Vec<Inventory>,
    pub inventory_errors: Vec<String>,
    pub character_gravity: String,
    pub world_gravity: String,
    pub jetpack: String,
    pub slots: String,
    pub projectiles: Vec<String>,
    pub evolution_energy: String,
    pub traversal_energy: String,
    pub feature_status: String,
}

fn item_bounds(start: usize, end: usize) -> Result<usize, String> {
    if start < 0x10000 || end < start || !start.is_multiple_of(4) || !end.is_multiple_of(4) {
        return Err("Inventory item-ID array bounds are invalid".into());
    }
    let span = end - start;
    if span > 256 || !span.is_multiple_of(4) {
        return Err("Inventory item-ID array exceeds 64 entries".into());
    }
    Ok(span / 4)
}

fn selected_registered(current: u32, items: &[u32]) -> bool {
    current == 0 || items.contains(&current)
}

fn slot_default_bytes(bytes: &[u8], offset: u8, count: u32) -> bool {
    bytes.len() == 7
        && bytes[0] == 0xC7
        && bytes[1] == 0x41
        && bytes[2] == offset
        && bytes[3..7] == count.to_le_bytes()
}

pub(super) fn projectile_observations() -> Vec<String> {
    vec![
        "Lightning Ball: observed physics vtable RVA 0x2338150 and XYZ velocity candidate +0x2E0; class verification required before treating it as speed.".into(),
        "Abe grenade/dart, Emet homing shot, Kraken Plasma Blast: observed vtable RVA 0x2336380 and velocity candidate +0x1E0; individual class verification required.".into(),
        "Goliath Rock Throw shares vtable RVA 0x2338150 but +0x2E0 was NOT its observed flight velocity. No universal projectile speed edit.".into(),
    ]
}

#[cfg(windows)]
mod windows {
    use super::*;
    use read_process_memory::{ProcessHandle, copy_address};
    use sha2::{Digest, Sha256};
    use std::io::Read;
    use std::path::Path;

    fn raw(h: &ProcessHandle, address: usize, count: usize) -> Result<Vec<u8>, String> {
        if !(0x10000..0x0000_7FFF_FFFF_FFFF).contains(&address)
            || address
                .checked_add(count)
                .is_none_or(|v| v >= 0x0000_7FFF_FFFF_FFFF)
            || count > 4096
        {
            return Err("Memory read outside permitted address/size".into());
        }
        copy_address(address, count, h)
            .map_err(|e| format!("Read-only memory inspection failed at 0x{address:X}: {e}"))
    }
    fn u32_at(h: &ProcessHandle, at: usize) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            raw(h, at, 4)?.try_into().map_err(|_| "Incomplete u32")?,
        ))
    }
    fn f32_at(h: &ProcessHandle, at: usize) -> Result<f32, String> {
        let value = f32::from_bits(u32_at(h, at)?);
        if !value.is_finite() {
            return Err("Non-finite observed float".into());
        }
        Ok(value)
    }
    fn ptr(h: &ProcessHandle, at: usize) -> Result<usize, String> {
        let n = u64::from_le_bytes(
            raw(h, at, 8)?
                .try_into()
                .map_err(|_| "Incomplete pointer")?,
        );
        let addr = usize::try_from(n).map_err(|_| "Pointer exceeds address width")?;
        if !(0x10000..0x0000_7FFF_FFFF_FFFF).contains(&addr) {
            return Err(format!("Invalid process pointer at 0x{at:X}"));
        }
        Ok(addr)
    }
    fn expected_vtable(
        h: &ProcessHandle,
        address: usize,
        base: usize,
        rva: usize,
    ) -> Result<(), String> {
        let expected = base.checked_add(rva).ok_or("Vtable RVA overflow")?;
        if ptr(h, address)? != expected {
            return Err(format!("Unsupported build/object vtable at 0x{address:X}"));
        }
        Ok(())
    }
    fn image_digest(snapshot: &Snapshot) -> String {
        let Some(path) = snapshot
            .executable
            .strip_prefix("Actual running executable: ")
        else {
            return "Unavailable: running executable path was not verified".into();
        };
        let Ok(mut f) = std::fs::File::open(Path::new(path)) else {
            return "Unavailable: could not open running executable on disk".into();
        };
        let mut hash = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => hash.update(&buf[..n]),
                Err(_) => return "Unavailable: could not finish executable SHA256".into(),
            }
        }
        format!("{:x}", hash.finalize())
    }

    /// A fresh, local actor-only read. A failed backlink, unexpected vtable,
    /// invalid stage or changed key never produces a misleading numeric readout.
    /// No offline authority can be inferred from these memory reads.
    fn monster_resources(
        h: &ProcessHandle,
        base: usize,
        local: &runtime_actors::Actor,
    ) -> Result<(String, String, Option<EvolutionEnergy>), String> {
        if !local.local || !known_monster(&local.name) {
            return Err("Local actor is not a recognized Stage 2 monster".into());
        }
        expected_vtable(h, local.address, base, ACTOR_VTABLE_RVA)?;
        if ptr(h, local.address + 0x38)? != local.entity
            || u32_at(h, local.entity + 0x18)? != local.id
        {
            return Err("Local monster actor/entity backlink changed".into());
        }
        let movement = ptr(h, local.address + 0x240)?;
        if ptr(h, movement + 0x38)? != local.entity {
            return Err("Monster movement/entity backlink is unverified".into());
        }

        let stage_word = u32_at(h, local.address + 0x1260)?;
        let key_at = base
            .checked_add(MONSTER_ENERGY_KEY_RVA)
            .ok_or("Key RVA overflow")?;
        let key = u32_at(h, key_at)?;
        let evolution_raw = u32_at(h, local.address + 0x16FC)?;
        let evolution_max_raw = u32_at(h, local.address + 0x1700)?;
        let traversal_raw = u32_at(h, local.address + 0x1754)?;
        let traversal_max_raw = u32_at(h, local.address + 0x1758)?;

        // Reject torn reads or an actor swap before presenting captured energy.
        if u32_at(h, key_at)? != key
            || u32_at(h, local.address + 0x1260)? != stage_word
            || ptr(h, local.address + 0x38)? != local.entity
            || ptr(h, movement + 0x38)? != local.entity
            || u32_at(h, local.entity + 0x18)? != local.id
        {
            return Err("Monster identity, stage or encoding key changed during read".into());
        }

        let decoded_evolution =
            checked_evolution(stage_word, evolution_raw, evolution_max_raw, key);
        let snapshot = decoded_evolution
            .as_ref()
            .ok()
            .map(|&(stage, current, maximum)| EvolutionEnergy {
                stage,
                current,
                maximum,
            });
        let evolution = match decoded_evolution {
            Ok((stage, current, maximum)) => {
                if stage == 3 {
                    "Evolution: stage 3 · bar inactive (0/0); READ-ONLY".to_owned()
                } else {
                    format!("Evolution: stage {stage} · {current:.2}/{maximum:.2}; READ-ONLY")
                }
            }
            Err(reason) => format!("Evolution unavailable: {reason}"),
        };
        let traversal = match checked_traversal(traversal_raw, traversal_max_raw, key) {
            Ok((current, maximum)) => {
                format!(
                    "Traversal: {current:.2}/{maximum:.2}; READ-ONLY (no recharge-rate offset verified)"
                )
            }
            Err(reason) => format!("Traversal unavailable: {reason}"),
        };
        Ok((evolution, traversal, snapshot))
    }

    /// Used by Discord without invoking the much heavier inventory/roster
    /// research. The caller passes a freshly discovered local actor; this
    /// revalidates its identity, stage and process-specific encoding key.
    pub(super) fn inspect_evolution(
        snapshot: &Snapshot,
        root: &str,
        local: &runtime_actors::Actor,
    ) -> Result<EvolutionEnergy, String> {
        let now = runtime_memory::inspect(root);
        if snapshot.pid.is_none()
            || now.pid != snapshot.pid
            || now.module_base.is_none()
            || now.module_base != snapshot.module_base
            || now.executable != snapshot.executable
        {
            return Err("Evolve executable or process changed since actor discovery".into());
        }
        let pid = now.pid.ok_or("Evolve process unavailable")?;
        let base = now.module_base.ok_or("Module base unavailable")?;
        let h: ProcessHandle = pid
            .try_into()
            .map_err(|e| format!("Cannot inspect local monster: {e}"))?;
        let (_, _, decoded) = monster_resources(&h, base, local)?;
        decoded.ok_or_else(|| "Evolution fields failed stage/rules validation".into())
    }

    fn inventory(
        h: &ProcessHandle,
        base: usize,
        table: usize,
        actor: &runtime_actors::Actor,
    ) -> Result<Inventory, String> {
        // Do not read a stale snapshot after another match or character switch.
        if ptr(h, actor.address + 0x38)? != actor.entity
            || u32_at(h, actor.entity + 0x18)? != actor.id
        {
            return Err("Actor/entity identity changed during inspection".into());
        }
        expected_vtable(h, actor.address, base, ACTOR_VTABLE_RVA)?;
        expected_vtable(h, actor.entity, base, ENTITY_VTABLE_RVA)?;

        let inv = ptr(h, actor.address + 0x170)?;
        expected_vtable(h, inv, base, INVENTORY_VTABLE_RVA)?;
        if u32_at(h, inv + 0x20)? != actor.id || ptr(h, inv + 0x38)? != actor.entity {
            return Err("Inventory owner ID or entity backlink mismatch".into());
        }
        let start = ptr(h, inv + 0x40)?;
        let end = ptr(h, inv + 0x48)?;
        let count = item_bounds(start, end)?;
        let mut items = Vec::with_capacity(count);
        let mut registered = 0;
        for index in 0..count {
            let id = u32_at(h, start + index * 4)?;
            items.push(id);
            if id > 0 {
                let slot = (id as usize & 0xFFFF) * 8;
                if table
                    .checked_add(slot)
                    .and_then(|address| ptr(h, address).ok())
                    .and_then(|entity| u32_at(h, entity + 0x18).ok())
                    == Some(id)
                {
                    registered += 1;
                }
            }
        }

        let mut loadout = Vec::with_capacity(16);
        for i in 0..16 {
            let address = inv + 0x6C + i * 8;
            loadout.push((u32_at(h, address)?, u32_at(h, address + 4)?));
        }
        let current = u32_at(h, inv + 0xF0)?;
        let previous = u32_at(h, inv + 0xF8)?;
        let mirror_a = u32_at(h, actor.address + 0x2F0)?;
        let mirror_b = u32_at(h, actor.address + 0x14E0)?;
        let mirrors_match = mirror_a == current && mirror_b == current;

        // Final freshness test before returning values to the GUI.
        if ptr(h, actor.address + 0x170)? != inv
            || u32_at(h, inv + 0x20)? != actor.id
            || ptr(h, inv + 0x40)? != start
            || ptr(h, inv + 0x48)? != end
        {
            return Err("Inventory changed while reading; refresh again".into());
        }
        let in_list = selected_registered(current, &items);
        Ok(Inventory {
            actor_name: actor.name.clone(),
            actor_id: actor.id,
            inventory_address: inv,
            item_ids: items,
            registered_ids: registered,
            loadout,
            current,
            previous,
            selected_in_list: in_list,
            actor_mirrors_match: mirrors_match,
        })
    }

    fn character_gravity(
        h: &ProcessHandle,
        base: usize,
        local: &runtime_actors::Actor,
    ) -> Result<f32, String> {
        // Same validated type-1 physics proxy route as runtime_teleport.rs.
        let header = ptr(h, local.entity + 0xD8)?;
        let mut node = ptr(h, header + 8)?;
        let mut candidate = header;
        for _ in 0..128 {
            if node == header || raw(h, node + 0x19, 1)?[0] != 0 {
                break;
            }
            if u32_at(h, node + 0x20)? < 1 {
                node = ptr(h, node + 0x10)?;
            } else {
                candidate = node;
                node = ptr(h, node)?;
            }
        }
        if candidate == header || u32_at(h, candidate + 0x20)? != 1 {
            return Err("Local character has no validated physics proxy".into());
        }
        let proxy = ptr(h, candidate + 0x28)?;
        expected_vtable(h, proxy, base, PROXY_VTABLE_RVA)?;
        let physics = ptr(h, proxy + 0x38)?;
        expected_vtable(h, physics, base, LIVING_VTABLE_RVA)?;
        let value = f32_at(h, physics + 0x1C0)?;
        if !(-100.0..=100.0).contains(&value) {
            return Err("Character gravity candidate is out of range".into());
        }
        Ok(value)
    }

    pub(crate) fn inspect(
        snapshot: &Snapshot,
        root: &str,
        actors: &runtime_actors::Listing,
    ) -> Result<Report, String> {
        let pid = snapshot
            .pid
            .ok_or("Detect Evolve.exe before runtime research")?;
        let base = snapshot.module_base.ok_or("Evolve.exe base unavailable")?;
        if actors.pid != pid {
            return Err("Actor listing belongs to another process".into());
        }
        let now = runtime_memory::inspect(root);
        if now.pid != Some(pid)
            || now.module_base != Some(base)
            || now.executable != snapshot.executable
        {
            return Err("Executable identity changed; redetect Evolve.exe".into());
        }
        let h: ProcessHandle = pid
            .try_into()
            .map_err(|e| format!("Cannot read Evolve: {e}"))?;
        let anchor = base
            .checked_add(ENTITY_SYSTEM_RVA)
            .ok_or("Entity RVA overflow")?;
        let game = ptr(&h, anchor - 0x10)?;
        let framework = ptr(&h, game + 0x60)?;
        let client = ptr(&h, framework + 0x528)?;
        let actor_addr = ptr(&h, client + 0x178)?;
        let local_id = u32_at(&h, client + 0x170)?;
        let factory = ptr(&h, anchor)?;
        let table = ptr(&h, factory + 0xA8)?;
        let entity = ptr(&h, table + (local_id as usize & 0xFFFF) * 8)?;
        if local_id == 0
            || u32_at(&h, entity + 0x18)? != local_id
            || ptr(&h, actor_addr + 0x38)? != entity
        {
            return Err("Local actor/session chain changed".into());
        }
        expected_vtable(&h, actor_addr, base, ACTOR_VTABLE_RVA)?;
        expected_vtable(&h, entity, base, ENTITY_VTABLE_RVA)?;
        let local = actors
            .actors
            .iter()
            .find(|a| a.local && a.id == local_id && a.address == actor_addr && a.entity == entity)
            .ok_or("Actor listing is stale; refresh the game first")?;

        let mut inventories = Vec::new();
        let mut errors = Vec::new();
        for actor in actors.actors.iter().take(64) {
            match inventory(&h, base, table, actor) {
                Ok(inv) => inventories.push(inv),
                Err(e) => errors.push(format!("{} (0x{:X}): {}", actor.name, actor.id, e)),
            }
        }

        let gravity = match character_gravity(&h, base, local) {
            Ok(v) => format!(
                "Local CLivingEntity +0x1C0: {v:.3} (candidate, read-only; not a gravity-off switch)"
            ),
            Err(e) => format!("Character gravity unavailable: {e}"),
        };

        let jetpack = if local.name == "TrackingCBot" {
            match raw(&h, actor_addr + 0xE68, 4) {
                Ok(bytes) => format!(
                    "Abe actor +0xE68: bytes {:02X} {:02X} {:02X} {:02X}; observed thrust-state candidate, NOT fuel",
                    bytes[0], bytes[1], bytes[2], bytes[3]
                ),
                Err(e) => format!("Abe jetpack state unreadable: {e}"),
            }
        } else {
            "Jetpack +0xE68 was observed only on Abe; unverified for this character".into()
        };

        let merc_bytes = raw(&h, base + 0x15CF820, 7).ok();
        let monster_bytes = raw(&h, base + 0x15CF827, 7).ok();
        let slots = match (merc_bytes, monster_bytes) {
            (Some(m), Some(n))
                if slot_default_bytes(&m, 8, 4)
                    && slot_default_bytes(&n, 0xC, 1) =>
            {
                "Observed Hunt initialization code: 4 merc slots and 1 monster slot. Team-3 admission getter at RVA 0x1124240 was separately observed to cap at 0/1. Code signature matched; no second-player support inferred.".into()
            }
            _ => "Hunt slot-default code signature did not match this executable; slot capacity is NOT verified (no patch/write attempted).".into(),
        };

        // Energy decoding is a separate, strictly read-only local monster path.
        // Failed validation is reported, not replaced with guessed values.
        let (evolution_energy, traversal_energy) = match monster_resources(&h, base, local) {
            Ok((evolution, traversal, _)) => (evolution, traversal),
            Err(reason) => (
                format!("Evolution unavailable: {reason}"),
                format!("Traversal unavailable: {reason}"),
            ),
        };

        // Do not equate these offsets with a complete projectile class decoder.
        let projectiles = projectile_observations();

        let end = runtime_memory::inspect(root);
        if end.pid != Some(pid)
            || end.module_base != Some(base)
            || u32_at(&h, client + 0x170)? != local_id
            || ptr(&h, client + 0x178)? != actor_addr
            || ptr(&h, actor_addr + 0x38)? != entity
        {
            return Err(
                "Evolve session/actor changed during inspection; no values retained".into(),
            );
        }

        Ok(Report {
            pid,
            image_sha256: image_digest(snapshot),
            base,
            local_id,
            inventory: inventories,
            inventory_errors: errors,
            character_gravity: gravity,
            world_gravity: "Physics-world p_gravity_z +0x128 was -13.0 in research, but the world object has no independently validated live pointer chain; not read or writable here.".into(),
            jetpack,
            slots,
            projectiles,
            evolution_energy,
            traversal_energy,
            feature_status: "Read-only observations. The equip RVA 0x446E88, give/pickup API, projectile velocity writes, gravity writes and second-monster slot creation remain UNVERIFIED and are not callable. Foreign item-ID swaps are explicitly unsupported.".into(),
        })
    }
}
#[cfg(windows)]
pub(super) use windows::{inspect, inspect_evolution};

#[cfg(not(windows))]
pub(super) fn inspect_evolution(
    _: &Snapshot,
    _: &str,
    _: &runtime_actors::Actor,
) -> Result<EvolutionEnergy, String> {
    Err("Live monster energy inspection requires Windows".into())
}

#[cfg(not(windows))]
pub(super) fn inspect(
    _: &Snapshot,
    _: &str,
    _: &runtime_actors::Listing,
) -> Result<Report, String> {
    Err("Runtime research requires Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn item_bounds_reject_unaligned_reversed_and_excessive() {
        assert!(item_bounds(0x10000, 0x1002).is_err());
        assert!(item_bounds(0x11000, 0x10000).is_err());
        assert!(item_bounds(0x10000, 0x10104).is_err());
        assert_eq!(item_bounds(0x10000, 0x1002C).unwrap(), 11);
    }
    #[test]
    fn selected_item_must_be_registered_in_the_inventory_list() {
        assert!(selected_registered(0, &[]));
        assert!(selected_registered(42, &[4, 42, 9]));
        assert!(!selected_registered(42, &[4, 9]));
    }
    #[test]
    fn slot_defaults_only_match_exact_tested_instructions() {
        assert!(slot_default_bytes(&[0xC7, 0x41, 8, 4, 0, 0, 0], 8, 4));
        assert!(slot_default_bytes(&[0xC7, 0x41, 12, 1, 0, 0, 0], 12, 1));
        assert!(!slot_default_bytes(&[0xC7, 0x41, 12, 2, 0, 0, 0], 12, 1));
        assert!(!slot_default_bytes(&[0x90, 0x41, 12, 1, 0, 0, 0], 12, 1));
    }
    #[test]
    fn stage_decode_ignores_upper_bits_but_rejects_bad_low_word() {
        assert_eq!(monster_stage(0xFA36_0101), Some(1));
        assert_eq!(monster_stage(0x55FA_0202), Some(2));
        assert_eq!(monster_stage(0x1234_0303), Some(3));
        assert_eq!(monster_stage(0x0102), None);
        assert_eq!(monster_stage(0), None);
    }

    #[test]
    fn evolution_decodes_two_keys_and_stage_bounds() {
        for key in [0xB805B8B7, 0xC0B73BB2] {
            for (stage, current, maximum) in [
                (0x101, 0.0f32, 24.0f32),
                (0x101, 24.0, 24.0),
                (0x202, 0.0, 28.0),
                (0x202, 28.0, 28.0),
                (0x303, 0.0, 0.0),
            ] {
                let raw_current = current.to_bits().rotate_left(9) ^ key;
                let raw_max = maximum.to_bits().rotate_right(16) ^ key;
                assert_eq!(
                    checked_evolution(stage, raw_current, raw_max, key).unwrap(),
                    (monster_stage(stage).unwrap(), current, maximum)
                );
            }
            let bad_current = f32::NAN.to_bits().rotate_left(9) ^ key;
            let max = 24.0f32.to_bits().rotate_right(16) ^ key;
            assert!(checked_evolution(0x101, bad_current, max, key).is_err());
            assert!(
                checked_evolution(
                    0x303,
                    1.0f32.to_bits().rotate_left(9) ^ key,
                    0.0f32.to_bits().rotate_right(16) ^ key
                )
                .is_err()
            );
        }
    }

    #[test]
    fn evolution_percentage_is_rounded_and_stage_three_hidden() {
        assert_eq!(
            EvolutionEnergy {
                stage: 1,
                current: 0.0,
                maximum: 24.0
            }
            .percentage(),
            Some(0)
        );
        assert_eq!(
            EvolutionEnergy {
                stage: 1,
                current: 12.0,
                maximum: 24.0
            }
            .percentage(),
            Some(50)
        );
        assert_eq!(
            EvolutionEnergy {
                stage: 2,
                current: 11.0,
                maximum: 28.0
            }
            .percentage(),
            Some(39)
        );
        assert_eq!(
            EvolutionEnergy {
                stage: 2,
                current: 28.0,
                maximum: 28.0
            }
            .percentage(),
            Some(100)
        );
        assert_eq!(
            EvolutionEnergy {
                stage: 3,
                current: 0.0,
                maximum: 0.0
            }
            .percentage(),
            None
        );
        assert_eq!(
            EvolutionEnergy {
                stage: 1,
                current: f32::INFINITY,
                maximum: 24.0
            }
            .percentage(),
            None
        );
        assert_eq!(
            EvolutionEnergy {
                stage: 2,
                current: 29.0,
                maximum: 28.0
            }
            .percentage(),
            None
        );
    }

    #[test]
    fn traversal_decode_is_distinct_from_evolution() {
        for key in [0xB805B8B7, 0xC0B73BB2] {
            let current = 30.0f32;
            let maximum = 100.0f32;
            let raw_current = current.to_bits() ^ key ^ 0x0093F6E4;
            let raw_max = maximum.to_bits().rotate_right(7) ^ key;
            assert_eq!(
                checked_traversal(raw_current, raw_max, key).unwrap(),
                (current, maximum)
            );
            let nan = f32::INFINITY.to_bits() ^ key ^ 0x0093F6E4;
            assert!(checked_traversal(nan, raw_max, key).is_err());
        }
        assert!(known_monster("TeleporterABot"));
        assert!(known_monster("KrakenElderBot"));
        assert!(!known_monster("TrackingABot"));
        assert!(!known_monster("Actor entity 0x42 (name unresolved)"));
    }

    #[test]
    fn projectile_offsets_remain_class_specific_hypotheses() {
        let findings = projectile_observations();
        assert_eq!(findings.len(), 3);
        assert!(findings[2].contains("NOT"));
    }
}
