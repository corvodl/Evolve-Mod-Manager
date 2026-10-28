//! Offline-only one-shot player teleport for the password-gated Developer page.
//! Uses externally verified Stage 2 IEntity -> type-1 proxy -> CLivingEntity.
//! No injection, debugger, hook, remote thread, freeze or repeated writes.
//! Layout/RVAs are pinned to the examined build; unknown builds fail closed.
use crate::runtime_memory;
use runtime_memory::Snapshot;

const ENTITY_SYSTEM_RVA: usize = 0x28D58E8;
const ENTITY_VTABLE_RVA: usize = 0x22DBC60;
const PROXY_VTABLE_RVA: usize = 0x22E2DE0;
const LIVING_VTABLE_RVA: usize = 0x2336010;
const MAX_COORD: f32 = 100_000.0;
const MAX_STEP: f32 = 10_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}
impl Position {
    fn finite(self) -> bool {
        [self.x, self.y, self.z]
            .into_iter()
            .all(|v| v.is_finite() && v.abs() <= MAX_COORD)
    }
    fn near(self, other: Self) -> bool {
        (self.x - other.x).abs() < 0.5
            && (self.y - other.y).abs() < 0.5
            && (self.z - other.z).abs() < 0.5
    }
    pub fn label(self) -> String {
        format!("{:.3}, {:.3}, {:.3}", self.x, self.y, self.z)
    }
    pub fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z + other.z,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Reading {
    pub pid: u32,
    pub actor_id: u32,
    pub position: Position,
}
#[derive(Clone, Copy, Debug)]
pub(super) enum Destination {
    Absolute(Position),
    Relative(Position),
    /// Approximate forward destination from the actor's own (w,x,y,z) rotation.
    ActorAim(f32),
}
pub(super) fn parse_position(x: &str, y: &str, z: &str) -> Result<Position, String> {
    let parse = |s: &str| {
        s.trim()
            .parse::<f32>()
            .map_err(|_| "Enter valid Float32 coordinates".to_owned())
    };
    let pos = Position {
        x: parse(x)?,
        y: parse(y)?,
        z: parse(z)?,
    };
    if !pos.finite() {
        return Err("Coordinates must be finite and within +/-100000".into());
    }
    Ok(pos)
}

#[cfg(windows)]
mod windows {
    use super::*;
    use read_process_memory::{ProcessHandle, copy_address};
    use std::ffi::c_void;

    const PTR_MIN: usize = 0x10000;
    const PTR_MAX: usize = 0x0000_7FFF_FFFF_FFFF;

    fn read(h: &ProcessHandle, at: usize, size: usize) -> Result<Vec<u8>, String> {
        if !(PTR_MIN..PTR_MAX).contains(&at)
            || at.checked_add(size).is_none_or(|end| end >= PTR_MAX)
        {
            return Err("Invalid runtime address".into());
        }
        copy_address(at, size, h).map_err(|e| format!("Cannot read game memory at 0x{at:X}: {e}"))
    }
    fn u32_at(h: &ProcessHandle, at: usize) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            read(h, at, 4)?
                .try_into()
                .map_err(|_| "Incomplete UInt32")?,
        ))
    }
    fn q(h: &ProcessHandle, at: usize) -> Result<usize, String> {
        let p = u64::from_le_bytes(
            read(h, at, 8)?
                .try_into()
                .map_err(|_| "Incomplete pointer")?,
        ) as usize;
        if !(PTR_MIN..PTR_MAX).contains(&p) {
            return Err(format!("Invalid pointer at 0x{at:X}"));
        }
        Ok(p)
    }
    fn position(h: &ProcessHandle, at: usize, offsets: [usize; 3]) -> Result<Position, String> {
        let f = |off| u32_at(h, at + off).map(f32::from_bits);
        Ok(Position {
            x: f(offsets[0])?,
            y: f(offsets[1])?,
            z: f(offsets[2])?,
        })
    }
    fn valid_layout(
        h: &ProcessHandle,
        object: usize,
        base: usize,
        vtable_rva: usize,
    ) -> Result<(), String> {
        if q(h, object)? != base.checked_add(vtable_rva).ok_or("Module RVA overflow")? {
            return Err("Unrecognized game build / object vtable; teleport disabled".into());
        }
        Ok(())
    }
    // Check the running executable and ASLR base directly instead of launching
    // tasklist + PowerShell for every teleport (formerly multiple times per action).
    // The main module is first in the process module list. Fail closed if the
    // process is gone, inaccessible, or no longer the expected Evolve.exe.
    #[allow(unsafe_code)]
    fn running_evolve_base(pid: u32) -> Result<usize, String> {
        use std::io;
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
            fn CloseHandle(handle: *mut c_void) -> i32;
            fn K32EnumProcessModules(
                handle: *mut c_void,
                modules: *mut *mut c_void,
                size: u32,
                needed: *mut u32,
            ) -> i32;
            fn K32GetModuleBaseNameW(
                handle: *mut c_void,
                module: *mut c_void,
                name: *mut u16,
                size: u32,
            ) -> u32;
        }
        const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
        const PROCESS_VM_READ: u32 = 0x0010;
        let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
        if handle.is_null() {
            return Err(format!("Cannot verify Evolve process: {}", io::Error::last_os_error()));
        }
        let result = (|| {
            let mut main_module: *mut c_void = std::ptr::null_mut();
            let mut needed = 0u32;
            let ok = unsafe {
                K32EnumProcessModules(
                    handle,
                    &mut main_module,
                    std::mem::size_of::<*mut c_void>() as u32,
                    &mut needed,
                )
            };
            if ok == 0 || main_module.is_null() || needed < std::mem::size_of::<*mut c_void>() as u32 {
                return Err("Cannot verify running Evolve module".into());
            }
            let mut name = [0u16; 260];
            let count = unsafe {
                K32GetModuleBaseNameW(handle, main_module, name.as_mut_ptr(), name.len() as u32)
            } as usize;
            if count == 0 || !String::from_utf16_lossy(&name[..count]).eq_ignore_ascii_case("Evolve.exe") {
                return Err("Game PID no longer points to Evolve.exe; detect game again".into());
            }
            Ok(main_module as usize)
        })();
        unsafe { CloseHandle(handle) };
        result
    }

    fn session(snapshot: &Snapshot, _root: &str) -> Result<(u32, ProcessHandle, usize), String> {
        let pid = snapshot.pid.ok_or("Detect Evolve.exe first")?;
        let base = snapshot.module_base.ok_or("Evolve module base unavailable")?;
        if running_evolve_base(pid)? != base {
            return Err("Game executable or module base changed; detect Evolve again".into());
        }
        let h: ProcessHandle = pid
            .try_into()
            .map_err(|e| format!("Cannot open Evolve: {e}"))?;
        Ok((pid, h, base))
    }
    struct Resolved {
        pid: u32,
        actor_id: u32,
        entity: usize,
        movement: usize,
        physics: usize,
        pos: Position,
    }
    fn resolve(snapshot: &Snapshot, root: &str) -> Result<Resolved, String> {
        let (pid, h, base) = session(snapshot, root)?;
        let anchor = base
            .checked_add(ENTITY_SYSTEM_RVA)
            .ok_or("Entity anchor overflow")?;
        let game = q(&h, anchor - 0x10)?;
        let framework = q(&h, game + 0x60)?;
        let client = q(&h, framework + 0x528)?;
        let actor = q(&h, client + 0x178)?;
        let actor_id = u32_at(&h, client + 0x170)?;
        if actor_id == 0 {
            return Err("Spawn into an offline match first".into());
        }
        let entity_system = q(&h, anchor)?;
        let table = q(&h, entity_system + 0xA8)?;
        let entity = q(&h, table + (actor_id as usize & 0xFFFF) * 8)?;
        if u32_at(&h, entity + 0x18)? != actor_id || q(&h, actor + 0x38)? != entity {
            return Err("Local actor/entity identity mismatch".into());
        }
        valid_layout(&h, entity, base, ENTITY_VTABLE_RVA)?;
        let movement = q(&h, actor + 0x240)?;
        if q(&h, movement + 0x38)? != entity {
            return Err("Local movement/entity identity mismatch".into());
        }
        // Exactly the red-black tree walk in IEntity::GetProxy(type=1).
        let header = q(&h, entity + 0xD8)?;
        let mut node = q(&h, header + 8)?;
        let mut candidate = header;
        for _ in 0..128 {
            if node == header || read(&h, node + 0x19, 1)?[0] != 0 {
                break;
            }
            let key = u32_at(&h, node + 0x20)?;
            if key < 1 {
                node = q(&h, node + 0x10)?;
            } else {
                candidate = node;
                node = q(&h, node)?;
            }
        }
        if candidate == header || u32_at(&h, candidate + 0x20)? != 1 {
            return Err("Type-1 physics proxy not found".into());
        }
        let proxy = q(&h, candidate + 0x28)?;
        valid_layout(&h, proxy, base, PROXY_VTABLE_RVA)?;
        let physics = q(&h, proxy + 0x38)?;
        valid_layout(&h, physics, base, LIVING_VTABLE_RVA)?;
        let pos = position(&h, physics, [0xB8, 0xBC, 0xC0])?;
        let local = position(&h, entity, [0x40, 0x44, 0x48])?;
        let world = position(&h, entity, [0x74, 0x84, 0x94])?;
        let movement_pos = position(&h, movement, [0x214, 0x218, 0x21C])?;
        if !pos.finite() || !pos.near(local) || !pos.near(world) || !pos.near(movement_pos) {
            return Err(
                "Entity, physics, and movement coordinates disagree; stand still and retry".into(),
            );
        }
        Ok(Resolved {
            pid,
            actor_id,
            entity,
            movement,
            physics,
            pos,
        })
    }

    fn approximate_actor_aim(
        handle: &ProcessHandle,
        entity: usize,
        origin: Position,
        distance: f32,
    ) -> Result<Position, String> {
        if !distance.is_finite() || !(5.0..=100.0).contains(&distance) {
            return Err("Aim distance must be 5 to 100 units".into());
        }
        // CryEngine Quat is w,x,y,z, in the entity's local transform at +0x4C.
        // +Y is the entity's local forward axis; this is NOT camera pitch or a raycast.
        let [w, x, y, z] = [
            u32_at(handle, entity + 0x4C)?,
            u32_at(handle, entity + 0x50)?,
            u32_at(handle, entity + 0x54)?,
            u32_at(handle, entity + 0x58)?,
        ]
        .map(f32::from_bits);
        let len2 = w * w + x * x + y * y + z * z;
        if !len2.is_finite() || !(0.8..=1.2).contains(&len2) {
            return Err("Actor orientation is not a valid unit quaternion".into());
        }
        let normal = len2.sqrt();
        let (w, x, y, z) = (w / normal, x / normal, y / normal, z / normal);
        let forward = Position {
            x: 2.0 * (x * y - w * z),
            y: 1.0 - 2.0 * (x * x + z * z),
            z: 2.0 * (y * z + w * x),
        };
        let target = origin.add(Position {
            x: forward.x * distance,
            y: forward.y * distance,
            // Slight lift to reduce ground interpenetration; no collision test performed.
            z: forward.z * distance + 2.0,
        });
        if !target.finite() {
            return Err("Calculated aim target is outside world limits".into());
        }
        Ok(target)
    }

    pub(crate) fn preview_aim(
        snapshot: &Snapshot,
        root: &str,
        offline: bool,
        distance: f32,
    ) -> Result<Position, String> {
        if !offline {
            return Err("Confirm an isolated offline match".into());
        }
        let r = resolve(snapshot, root)?;
        let (_, handle, _) = session(snapshot, root)?;
        approximate_actor_aim(&handle, r.entity, r.pos, distance)
    }

    pub(crate) fn inspect(
        snapshot: &Snapshot,
        root: &str,
        offline: bool,
    ) -> Result<Reading, String> {
        if !offline {
            return Err("Confirm an isolated offline match".into());
        }
        let r = resolve(snapshot, root)?;
        Ok(Reading {
            pid: r.pid,
            actor_id: r.actor_id,
            position: r.pos,
        })
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(unsafe_code)] // Scoped FFI follows runtime_live's reviewed external process-write pattern.
    fn write_xyz(
        pid: u32,
        addr: usize,
        expected: Position,
        target: Position,
    ) -> Result<(), String> {
        use std::io;
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
            fn CloseHandle(handle: *mut c_void) -> i32;
            fn ReadProcessMemory(
                handle: *mut c_void,
                at: *const c_void,
                buf: *mut c_void,
                size: usize,
                got: *mut usize,
            ) -> i32;
            fn WriteProcessMemory(
                handle: *mut c_void,
                at: *mut c_void,
                buf: *const c_void,
                size: usize,
                wrote: *mut usize,
            ) -> i32;
        }
        const ACCESS: u32 = 0x0008 | 0x0010 | 0x0020;
        let handle = unsafe { OpenProcess(ACCESS, 0, pid) };
        if handle.is_null() {
            return Err(format!(
                "Cannot open game for teleport: {}",
                io::Error::last_os_error()
            ));
        }
        let result = (|| {
            let mut baseline = [0u8; 12];
            let mut len = 0usize;
            if unsafe {
                ReadProcessMemory(
                    handle,
                    addr as *const c_void,
                    baseline.as_mut_ptr().cast(),
                    12,
                    &mut len,
                )
            } == 0
                || len != 12
            {
                return Err("Could not re-read physics XYZ".into());
            }
            let original = [expected.x, expected.y, expected.z].map(f32::to_bits);
            let observed = [
                u32::from_le_bytes(baseline[0..4].try_into().unwrap()),
                u32::from_le_bytes(baseline[4..8].try_into().unwrap()),
                u32::from_le_bytes(baseline[8..12].try_into().unwrap()),
            ];
            if observed != original {
                return Err("Player moved before teleport; retry when stationary".into());
            }
            let mut bytes = [0u8; 12];
            for (i, v) in [target.x, target.y, target.z].iter().enumerate() {
                bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            len = 0;
            if unsafe {
                WriteProcessMemory(
                    handle,
                    addr as *mut c_void,
                    bytes.as_ptr().cast(),
                    12,
                    &mut len,
                )
            } == 0
                || len != 12
            {
                return Err("Physics XYZ write was refused or incomplete".into());
            }
            Ok(())
        })();
        unsafe {
            CloseHandle(handle);
        }
        result
    }

    pub(crate) fn teleport(
        snapshot: &Snapshot,
        root: &str,
        offline: bool,
        dest: Destination,
        expected_identity: Option<(u32, u32)>,
    ) -> Result<Reading, String> {
        if !offline {
            return Err("Confirm an isolated offline match".into());
        }
        let r = resolve(snapshot, root)?;
        if expected_identity.is_some_and(|id| id != (r.pid, r.actor_id)) {
            return Err("Saved location belongs to a different actor or game session".into());
        }
        let target = match dest {
            Destination::Absolute(p) => p,
            Destination::Relative(delta) => r.pos.add(delta),
            Destination::ActorAim(distance) => {
                let (_, h, _) = session(snapshot, root)?;
                approximate_actor_aim(&h, r.entity, r.pos, distance)?
            }
        };
        if !target.finite() {
            return Err("Target coordinates outside valid range".into());
        }
        let diff = Position {
            x: target.x - r.pos.x,
            y: target.y - r.pos.y,
            z: target.z - r.pos.z,
        };
        if [diff.x, diff.y, diff.z]
            .into_iter()
            .any(|n| n.abs() > MAX_STEP)
        {
            return Err("One-shot teleport limited to 10000 units per axis".into());
        }
        // Validate identity again immediately before one single XYZ write.
        let now = resolve(snapshot, root)?;
        if now.pid != r.pid
            || now.actor_id != r.actor_id
            || now.entity != r.entity
            || now.movement != r.movement
            || now.physics != r.physics
            || !now.pos.near(r.pos)
        {
            return Err("Actor or position changed while preparing teleport; retry".into());
        }
        write_xyz(r.pid, r.physics + 0xB8, now.pos, target)?;
        Ok(Reading {
            pid: r.pid,
            actor_id: r.actor_id,
            position: target,
        })
    }
}
#[cfg(windows)]
pub(super) use windows::{inspect, preview_aim, teleport};
#[cfg(not(windows))]
pub(super) fn inspect(_: &Snapshot, _: &str, _: bool) -> Result<Reading, String> {
    Err("Teleport requires Windows".into())
}
#[cfg(not(windows))]
pub(super) fn preview_aim(_: &Snapshot, _: &str, _: bool, _: f32) -> Result<Position, String> {
    Err("Aim preview requires Windows".into())
}
#[cfg(not(windows))]
pub(super) fn teleport(
    _: &Snapshot,
    _: &str,
    _: bool,
    _: Destination,
    _: Option<(u32, u32)>,
) -> Result<Reading, String> {
    Err("Teleport requires Windows".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_xyz_rejects_nonfinite_and_oob() {
        assert!(parse_position("1", "2", "3").is_ok());
        assert!(parse_position("NaN", "0", "0").is_err());
        assert!(parse_position("999999", "0", "0").is_err());
    }
    #[test]
    fn direction_requires_unit_quaternion() {
        // An identity quaternion, in CryEngine wxyz order, means local +Y.
        let f = |w: f32, x: f32, y: f32, z: f32| Position {
            x: 2.0 * (x * y - w * z),
            y: 1.0 - 2.0 * (x * x + z * z),
            z: 2.0 * (y * z + w * x),
        };
        assert!(f(1.0, 0.0, 0.0, 0.0).near(Position {
            x: 0.0,
            y: 1.0,
            z: 0.0
        }));
        assert!(f(0.70710677, 0.0, 0.0, 0.70710677).near(Position {
            x: -1.0,
            y: 0.0,
            z: 0.0
        }));
    }
    #[test]
    fn relative_position_is_three_axis() {
        assert_eq!(
            Position {
                x: 1.0,
                y: 2.0,
                z: 3.0
            }
            .add(Position {
                x: 0.0,
                y: 0.0,
                z: 20.0
            }),
            Position {
                x: 1.0,
                y: 2.0,
                z: 23.0
            }
        );
    }
}
