//! Runtime actor inspection for the password-gated Developer page.
//! Evolve Stage 2 offsets observed in offline Goliath sessions. The engine actor
//! registry/name layout is not known, so non-local actors are ONLY included if
//! a backlink to the independently verified local actor can be established.
//! All reads are external. Writes are compare-before-write and process-validated;
//! the explicit Developer write arm controls access (not TCP connection state).
use crate::{runtime_energy, runtime_live, runtime_memory};
use runtime_memory::Snapshot;

const ENTITY_SYSTEM_RVA: usize = 0x28D58E8;
const MAX_ENTITY_INDEX: usize = 65536;
const MAX_DISCOVERED_ACTORS: usize = 2048;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(super) enum Field {
    Health,
    MaxHealth,
    Armor,
    MaxArmor,
    Stage,
}
impl Field {
    pub fn title(self) -> &'static str {
        match self {
            Self::Health => "Current health",
            Self::MaxHealth => "Maximum health",
            Self::Armor => "Current armor",
            Self::MaxArmor => "Maximum armor",
            Self::Stage => "Evolution stage",
        }
    }
    pub fn offset(self) -> usize {
        match self {
            Self::Health => 0x198,
            Self::MaxHealth => 0x1A0,
            Self::Armor => 0x1038,
            Self::MaxArmor => 0x103C,
            Self::Stage => 0x1260,
        }
    }
    pub fn kind(self) -> &'static str {
        if self == Self::Stage {
            "UInt32"
        } else {
            "Float32"
        }
    }
}
const FIELDS: [Field; 5] = [
    Field::Health,
    Field::MaxHealth,
    Field::Armor,
    Field::MaxArmor,
    Field::Stage,
];

#[derive(Clone, Debug)]
pub(super) struct Value {
    pub field: Field,
    pub address: usize,
    pub bits: u32,
    pub display: String,
}
#[derive(Clone, Debug)]
pub(super) struct Actor {
    pub id: u32,
    pub address: usize,
    pub entity: usize,
    pub name: String,
    pub local: bool,
    pub fields: Vec<Value>,
    /// Never writable/frozen through the generic Float32 actor editor.
    pub energy: runtime_energy::Reading,
}
#[derive(Clone, Debug)]
pub(super) struct Listing {
    pub pid: u32,
    pub actors: Vec<Actor>,
    pub status: String,
}
#[derive(Clone, Debug)]
pub(super) struct WriteResult {
    pub actor: String,
    pub field: Field,
    pub before: String,
    pub after: String,
}

// Validate individual fields without making actor discovery depend on them.
fn stage_number(bits: u32) -> Option<u32> {
    runtime_energy::stage(bits)
}
fn plausible_pair(current_bits: u32, max_bits: u32) -> bool {
    let current = f32::from_bits(current_bits);
    let maximum = f32::from_bits(max_bits);
    current.is_finite()
        && maximum.is_finite()
        && maximum > 0.0
        && maximum <= 1_000_000.0
        && (0.0..=100_000.0).contains(&current) // Offline edits may exceed nominal maximum
}

#[cfg(windows)]
mod windows {
    use super::*;
    use read_process_memory::{ProcessHandle, copy_address};

    fn read_bytes(handle: &ProcessHandle, address: usize, count: usize) -> Option<Vec<u8>> {
        if !(0x10000..0x0000_7FFF_FFFF_FFFF).contains(&address) {
            return None;
        }
        copy_address(address, count, handle).ok()
    }
    fn u32_at(h: &ProcessHandle, at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(read_bytes(h, at, 4)?.try_into().ok()?))
    }
    fn q(h: &ProcessHandle, at: usize) -> Option<usize> {
        let p = u64::from_le_bytes(read_bytes(h, at, 8)?.try_into().ok()?);
        let p = usize::try_from(p).ok()?;
        (0x10000..0x0000_7FFF_FFFF_FFFF).contains(&p).then_some(p)
    }
    fn pointers(h: &ProcessHandle, at: usize, size: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        // Page/chunk failure is not fatal: never dereference an unverified pointer.
        for start in (0..size).step_by(0x100) {
            if let Some(bytes) = read_bytes(h, at.saturating_add(start), 0x100) {
                for i in (0..=0xF8).step_by(8) {
                    let value = u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
                    if (0x10000..0x0000_7FFF_FFFF_FFFF).contains(&value) {
                        out.push((start + i, value as usize));
                    }
                }
            }
        }
        out
    }

    /// Derive an entity->actor route from the KNOWN local entity/actor pair.
    /// Do not assume that every entity is an actor.
    #[derive(Clone, Copy)]
    enum Backlink {
        Direct(usize),
        Indirect(usize, usize),
    }
    fn discover_backlink(h: &ProcessHandle, entity: usize, actor: usize) -> Option<Backlink> {
        let children = pointers(h, entity, 0x400);
        if let Some(&(off, _)) = children.iter().find(|(_, p)| *p == actor) {
            return Some(Backlink::Direct(off));
        }
        for &(first, child) in children.iter().take(80) {
            if let Some(&(second, _)) = pointers(h, child, 0x400).iter().find(|(_, p)| *p == actor)
            {
                return Some(Backlink::Indirect(first, second));
            }
        }
        None
    }
    fn actor_for_entity(h: &ProcessHandle, entity: usize, route: Backlink) -> Option<usize> {
        let actor = match route {
            Backlink::Direct(off) => q(h, entity.checked_add(off)?)?,
            Backlink::Indirect(a, b) => q(h, q(h, entity.checked_add(a)?)?.checked_add(b)?)?,
        };
        (q(h, actor.checked_add(0x38)?)? == entity).then_some(actor)
    }
    // Actor identity is established independently of any monster-specific
    // health/stage offsets. Wraith and other playable actors may differ from
    // the original Goliath observations.
    fn actor_identity(h: &ProcessHandle, addr: usize, entity: usize) -> bool {
        q(h, addr.saturating_add(0x38)) == Some(entity)
    }
    /// Reconstructed read-only IEntity::GetName lookup (vtable +0x78) from
    /// live Stage 2 runtime traces. Resolve through the current process tables:
    /// name_id = [entity+0x124], index = name_id - 0xE11;
    /// encoded = [[resolver+0x08] + index*4];
    /// entry = [[resolver+0x20] + (encoded ^ 0x47FA)*8];
    /// returned C string = *entry.
    ///
    /// These are build-specific offsets, not universal CryEngine ABI fields.
    /// Any unreadable/stale pointer fails closed to an unresolved name.
    fn name_from_entity(h: &ProcessHandle, entity: usize, base: usize) -> Option<String> {
        const NAME_RESOLVER_RVA: usize = 0x28D59A8;
        const INDEX_ADJUST: u32 = 0xE11;
        const INDEX_XOR: u32 = 0x47FA;
        const MAX_NAME_INDEX: u32 = 0x10000;
        let name_id = u32_at(h, entity.checked_add(0x124)?)?;
        let index = name_id.checked_sub(INDEX_ADJUST)?;
        if index >= MAX_NAME_INDEX {
            return None;
        }

        let resolver = q(h, base.checked_add(NAME_RESOLVER_RVA)?)?;
        let index_table = q(h, resolver.checked_add(0x08)?)?;
        let entry_table = q(h, resolver.checked_add(0x20)?)?;

        let encoded = u32_at(
            h,
            index_table.checked_add((index as usize).checked_mul(4)?)?,
        )?;
        let decoded = encoded ^ INDEX_XOR;
        if decoded >= MAX_NAME_INDEX {
            return None;
        }

        let entry_addr = entry_table.checked_add((decoded as usize).checked_mul(8)?)?;
        let entry_object = q(h, entry_addr)?;
        let c_string = q(h, entry_object)?;

        // No substring searches: the resolved pointer must be a bounded,
        // NUL-terminated ASCII entity name (e.g. "AssaultEBot").
        let bytes = read_bytes(h, c_string, 32)?;
        let len = bytes.iter().position(|&b| b == 0)?;
        if !(3..32).contains(&len) || !bytes[..len].iter().all(u8::is_ascii_graphic) {
            return None;
        }
        String::from_utf8(bytes[..len].to_vec()).ok()
    }
    fn read_actor(
        h: &ProcessHandle,
        id: u32,
        at: usize,
        entity: usize,
        local: bool,
        base: usize,
    ) -> Actor {
        // Never label arbitrary offsets as health or armor. Require both
        // members of a pair to be readable and mutually plausible first.
        let health = u32_at(h, at.saturating_add(Field::Health.offset()))
            .zip(u32_at(h, at.saturating_add(Field::MaxHealth.offset())))
            .filter(|&(current, maximum)| plausible_pair(current, maximum));
        let armor = u32_at(h, at.saturating_add(Field::Armor.offset()))
            .zip(u32_at(h, at.saturating_add(Field::MaxArmor.offset())))
            .filter(|&(current, maximum)| plausible_pair(current, maximum));
        let stage = u32_at(h, at.saturating_add(Field::Stage.offset())).and_then(stage_number);
        let fields = FIELDS
            .into_iter()
            .filter_map(|field| {
                let address = at.checked_add(field.offset())?;
                let bits = match field {
                    Field::Health => health?.0,
                    Field::MaxHealth => health?.1,
                    Field::Armor => armor?.0,
                    Field::MaxArmor => armor?.1,
                    Field::Stage => {
                        stage?;
                        u32_at(h, address)?
                    }
                };
                let display = if field == Field::Stage {
                    format!("{}", stage?)
                } else {
                    format!("{:.3}", f32::from_bits(bits))
                };
                Some(Value {
                    field,
                    address,
                    bits,
                    display,
                })
            })
            .collect();
        let name = name_from_entity(h, entity, base)
            .unwrap_or_else(|| format!("Actor entity 0x{id:08X} (name unresolved)"));
        let energy = if local {
            let stage_word =
                u32_at(h, at.saturating_add(Field::Stage.offset())).unwrap_or_default();
            runtime_energy::read_local(h, base, at, &name, stage_word)
        } else {
            runtime_energy::Reading::default()
        };
        Actor {
            id,
            address: at,
            entity,
            local,
            name,
            fields,
            energy,
        }
    }
    fn process_and_handle(
        snapshot: &Snapshot,
        root: &str,
    ) -> Result<(u32, ProcessHandle, usize), String> {
        let pid = snapshot.pid.ok_or("Detect Evolve.exe first")?;
        let now = runtime_memory::inspect(root);
        if now.pid != Some(pid)
            || snapshot.module_base.is_none()
            || now.module_base != snapshot.module_base
        {
            return Err("Process or executable base changed; redetect Evolve".into());
        }
        let handle = pid
            .try_into()
            .map_err(|e| format!("Cannot read process: {e}"))?;
        Ok((pid, handle, now.module_base.unwrap()))
    }

    pub(crate) fn inspect(snapshot: &Snapshot, root: &str) -> Result<Listing, String> {
        let (pid, h, base) = process_and_handle(snapshot, root)?;
        let anchor = base
            .checked_add(ENTITY_SYSTEM_RVA)
            .ok_or("Anchor overflow")?;
        let game = q(&h, anchor - 0x10).ok_or("Game unavailable")?;
        let fw = q(&h, game + 0x60).ok_or("Framework unavailable")?;
        let client = q(&h, fw + 0x528).ok_or("Client actor interface unavailable")?;
        let local_actor = q(&h, client + 0x178).ok_or("Spawn in a match first")?;
        let local_id = u32_at(&h, client + 0x170).ok_or("No local entity ID")?;
        if local_id == 0 {
            return Err("No local actor spawned".into());
        }
        let sys = q(&h, anchor).ok_or("Entity system unavailable")?;
        let table = q(&h, sys + 0xA8).ok_or("Entity table unavailable")?;
        let local_entity =
            q(&h, table + ((local_id as usize & 0xFFFF) * 8)).ok_or("Local entity unavailable")?;
        if u32_at(&h, local_entity + 0x18) != Some(local_id) {
            return Err("Local entity ID mismatch (client vs entity table)".into());
        }
        if !actor_identity(&h, local_actor, local_entity) {
            return Err(
                "Local actor/entity pointer mismatch; verify build-specific actor layout".into(),
            );
        }
        // Show the structurally verified local actor even if no Goliath-based
        // field offsets match. Unsupported fields remain hidden and unwritable.
        let mut actors = vec![read_actor(
            &h,
            local_id,
            local_actor,
            local_entity,
            true,
            base,
        )];
        let route = discover_backlink(&h, local_entity, local_actor);
        if let Some(route) = route {
            // Entity IDs are salted 32-bit values with a 16-bit slot index.
            // Read this bounded table in chunks. No execution inside the game.
            for start in (0..MAX_ENTITY_INDEX).step_by(512) {
                if actors.len() >= MAX_DISCOVERED_ACTORS {
                    break;
                }
                let Some(chunk) = read_bytes(&h, table + start * 8, 512 * 8) else {
                    continue;
                };
                for i in 0..512 {
                    if actors.len() >= MAX_DISCOVERED_ACTORS {
                        break;
                    }
                    let slot = start + i;
                    if slot == (local_id & 0xFFFF) as usize {
                        continue;
                    }
                    let p =
                        u64::from_le_bytes(chunk[i * 8..i * 8 + 8].try_into().unwrap()) as usize;
                    if p < 0x10000 {
                        continue;
                    }
                    let Some(id) = u32_at(&h, p + 0x18) else {
                        continue;
                    };
                    if id == 0 || (id as usize & 0xFFFF) != slot {
                        continue;
                    }
                    let Some(actor) = actor_for_entity(&h, p, route) else {
                        continue;
                    };
                    if !actor_identity(&h, actor, p) {
                        continue;
                    }
                    let candidate = read_actor(&h, id, actor, p, false, base);
                    // Actor identity is structural; absence of known stats does not
                    // make a valid actor disappear from the viewer.
                    actors.push(candidate);
                }
            }
        }
        if runtime_memory::inspect(root).pid != Some(pid) {
            return Err("Evolve changed while enumerating actors".into());
        }
        actors.sort_by_key(|a| (!a.local, a.id));
        let local_fields = actors.first().map(|a| a.fields.len()).unwrap_or(0);
        let coverage = format!(
            "{} actor(s) found; local fields validated: {local_fields}/{}.",
            actors.len(),
            FIELDS.len()
        );
        let status = if local_fields == 0 {
            format!("{coverage} Field offsets unverified for this actor/build; writes unavailable.")
        } else if route.is_none() {
            format!(
                "{coverage} Entity-to-actor backlink unavailable for this build; nonlocal actor discovery requires validated mapping."
            )
        } else {
            coverage
        };
        Ok(Listing {
            pid,
            actors,
            status,
        })
    }

    /// One bounded freeze iteration. Read and validate the live actor before any
    /// write; refuse to freeze a stale or replaced actor.
    pub(crate) fn freeze_once(
        snapshot: &Snapshot,
        root: &str,
        id: u32,
        actor_addr: usize,
        field: Field,
        desired: u32,
    ) -> Result<bool, String> {
        let pid = snapshot.pid.ok_or("Game process not detected")?;
        let listing = inspect(snapshot, root)?;
        let actor = listing
            .actors
            .iter()
            .find(|a| a.id == id && a.address == actor_addr)
            .ok_or("Frozen actor despawned or its identity changed")?;
        let value = actor
            .fields
            .iter()
            .find(|v| v.field == field)
            .ok_or("Frozen field is no longer validated")?;
        if value.bits == desired {
            return Ok(false);
        }
        let (_, h, _) = process_and_handle(snapshot, root)?;
        if q(&h, actor_addr + 0x38) != Some(actor.entity)
            || u32_at(&h, actor.entity + 0x18) != Some(id)
            || u32_at(&h, value.address) != Some(value.bits)
        {
            return Err("Frozen actor or field changed before write".into());
        }
        runtime_live::write_four(pid, value.address, value.bits, desired)?;
        Ok(true)
    }

    pub(crate) fn write(
        snapshot: &Snapshot,
        root: &str,
        id: u32,
        actor_addr: usize,
        field: Field,
        expected: u32,
        input: &str,
    ) -> Result<WriteResult, String> {
        let pid = snapshot.pid.ok_or("Detect Evolve first")?;
        let listing = inspect(snapshot, root)?;
        let actor = listing
            .actors
            .iter()
            .find(|a| a.id == id && a.address == actor_addr)
            .ok_or("Actor moved, despawned or changed; refresh before writing")?;
        let value = actor
            .fields
            .iter()
            .find(|v| v.field == field)
            .ok_or("Requested field unavailable for this actor")?;
        if value.bits != expected {
            return Err("Value changed since last refresh; reread before writing".into());
        }
        let new_bits = if field == Field::Stage {
            // Stage has been observed as 0x0101, 0x0202, 0x0303;
            // do not silently change only the low byte.
            let stage: u32 = input
                .trim()
                .parse()
                .map_err(|_| "Stage must be 1, 2 or 3")?;
            if !(1..=3).contains(&stage) || ![0x101, 0x202, 0x303].contains(&expected) {
                return Err("Unsupported evolution stage encoding".into());
            }
            stage | (stage << 8)
        } else {
            let v: f32 = input.trim().parse().map_err(|_| "Enter a Float32 value")?;
            if !v.is_finite() || !(0.0..=100_000.0).contains(&v) {
                return Err("Float32 value must be between 0 and 100000".into());
            }
            v.to_bits()
        };
        if new_bits == expected {
            return Err("Value is unchanged".into());
        }
        let (_, h, _) = process_and_handle(snapshot, root)?;
        // Revalidate the actor identity and compare the same bytes immediately
        // before the existing single-shot CAS memory writer.
        if q(&h, actor_addr + 0x38) != Some(actor.entity)
            || u32_at(&h, actor.entity + 0x18) != Some(id)
            || u32_at(&h, value.address) != Some(expected)
        {
            return Err("Actor/field changed before write; refresh".into());
        }
        runtime_live::write_four(pid, value.address, expected, new_bits)?;
        let observed = u32_at(&h, value.address).ok_or("Write performed, but readback failed")?;
        if observed != new_bits {
            return Err("Write did not persist at readback".into());
        }
        Ok(WriteResult {
            actor: actor.name.clone(),
            field,
            before: value.display.clone(),
            after: if field == Field::Stage {
                format!("{}", new_bits & 0xFF)
            } else {
                format!("{:.3}", f32::from_bits(new_bits))
            },
        })
    }
}
#[cfg(windows)]
pub(super) use windows::{freeze_once, inspect, write};
#[cfg(not(windows))]
pub(super) fn inspect(_: &Snapshot, _: &str) -> Result<Listing, String> {
    Err("Runtime actor inspection requires Windows".into())
}
#[cfg(not(windows))]
pub(super) fn freeze_once(
    _: &Snapshot,
    _: &str,
    _: u32,
    _: usize,
    _: Field,
    _: u32,
) -> Result<bool, String> {
    Err("Runtime freezing requires Windows".into())
}
#[cfg(not(windows))]
pub(super) fn write(
    _: &Snapshot,
    _: &str,
    _: u32,
    _: usize,
    _: Field,
    _: u32,
    _: &str,
) -> Result<WriteResult, String> {
    Err("Runtime writes require Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stage_and_float_offsets_are_distinct() {
        assert_eq!(Field::Health.offset(), 0x198);
        assert_eq!(Field::MaxHealth.offset(), 0x1A0);
        assert_eq!(Field::Armor.offset(), 0x1038);
        assert_eq!(Field::MaxArmor.offset(), 0x103C);
        assert_eq!(Field::Stage.offset(), 0x1260);
        assert_eq!(Field::Stage.kind(), "UInt32");
        assert_eq!(Field::Health.kind(), "Float32");
    }
    #[test]
    fn independent_field_validation_rejects_invalid_offsets() {
        assert_eq!(stage_number(0x0101), Some(1));
        assert_eq!(stage_number(0x0202), Some(2));
        assert_eq!(stage_number(0x0303), Some(3));
        assert_eq!(stage_number(0xFA36_0101), Some(1));
        assert_eq!(stage_number(0x55FA_0202), Some(2));
        assert_eq!(stage_number(0), None);
        assert_eq!(stage_number(0x0404), None);
        assert!(plausible_pair(200.0f32.to_bits(), 500.0f32.to_bits()));
        assert!(plausible_pair(600.0f32.to_bits(), 500.0f32.to_bits()));
        assert!(!plausible_pair(f32::NAN.to_bits(), 500.0f32.to_bits()));
        assert!(!plausible_pair(0.0f32.to_bits(), 0.0f32.to_bits()));
    }
}
