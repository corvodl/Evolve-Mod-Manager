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
    pub feature_status: String,
}

fn item_bounds(start: usize, end: usize) -> Result<usize, String> {
    if start < 0x10000 || end < start || start % 4 != 0 || end % 4 != 0 {
        return Err("Inventory item-ID array bounds are invalid".into());
    }
    let span = end - start;
    if span > 256 || span % 4 != 0 {
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
        Ok(Inventory {
            actor_name: actor.name.clone(),
            actor_id: actor.id,
            inventory_address: inv,
            item_ids: items.clone(),
            registered_ids: registered,
            loadout,
            current,
            previous,
            selected_in_list: selected_registered(current, &items),
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

    pub(super) fn inspect(
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
            feature_status: "Read-only observations. The equip RVA 0x446E88, give/pickup API, projectile velocity writes, gravity writes and second-monster slot creation remain UNVERIFIED and are not callable. Foreign item-ID swaps are explicitly unsupported.".into(),
        })
    }
}
#[cfg(windows)]
pub(super) use windows::inspect;

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
        assert_eq!(item_bounds(0x10000, 0x1002).is_err(), true);
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
    fn projectile_offsets_remain_class_specific_hypotheses() {
        let findings = projectile_observations();
        assert_eq!(findings.len(), 3);
        assert!(findings[2].contains("NOT"));
    }
}
