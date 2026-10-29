//! Offline-only one-shot player teleport for the password-gated Developer page.
//! Uses externally verified Stage 2 IEntity -> type-1 proxy -> CLivingEntity.
//! No injection, debugger, hook or remote thread. Coordinate hold uses an\n//! explicitly cancellable, bounded-frequency external write loop.
//! Layout/RVAs are pinned to the examined build; unknown builds fail closed.
use crate::runtime_memory;
use runtime_memory::Snapshot;
#[cfg(windows)]
use std::sync::atomic::Ordering;
use std::sync::{Arc, atomic::AtomicBool};

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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HoldMode {
    /// Keep altitude and explicitly move X/Y using WASD. CryEngine's
    /// normal locomotion can stop when a character is off the ground.
    Z,
    /// Keep all three world-space coordinates fixed.
    Xyz,
    /// Approximate no-fall flight by updating the local player's physics XYZ.
    /// This does not modify CryEngine gravity or velocity fields.
    Fly,
}

impl HoldMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Z => "Z (WASD hover)",
            Self::Xyz => "XYZ (position lock)",
            Self::Fly => "Fly (WASD/Space/Ctrl)",
        }
    }
}

/// Pure target selection used by the hold worker and its tests.
fn held_position(anchor: Position, current: Position, mode: HoldMode) -> Position {
    match mode {
        HoldMode::Z => Position {
            z: anchor.z,
            ..current
        },
        HoldMode::Xyz | HoldMode::Fly => anchor,
    }
}

/// User-configurable world units per second for position-based flight.
pub(super) fn parse_fly_speed(raw: &str) -> Result<f32, String> {
    let speed = raw
        .trim()
        .parse::<f32>()
        .map_err(|_| "Fly speed must be a number from 1 to 40 units/sec".to_owned())?;
    if !speed.is_finite() || !(1.0..=40.0).contains(&speed) {
        return Err("Fly speed must be between 1 and 40 units/sec".into());
    }
    Ok(speed)
}

#[derive(Clone, Copy, Debug, Default)]
struct FlyInput {
    forward: f32,
    strafe: f32,
    vertical: f32,
}

impl FlyInput {
    fn active(self) -> bool {
        self.forward != 0.0 || self.strafe != 0.0 || self.vertical != 0.0
    }
}

/// Convert CryEngine actor orientation (w,x,y,z) to horizontal forward.
fn fly_forward(quaternion: [f32; 4]) -> Result<Position, String> {
    let [w, x, y, z] = quaternion;
    let len2 = w * w + x * x + y * y + z * z;
    if !len2.is_finite() || !(0.8..=1.2).contains(&len2) {
        return Err("Fly Mode stopped: local actor orientation is invalid".into());
    }
    let len = len2.sqrt();
    let (w, x, y, z) = (w / len, x / len, y / len, z / len);
    let fx = 2.0 * (x * y - w * z);
    let fy = 1.0 - 2.0 * (x * x + z * z);
    let horizontal_len = (fx * fx + fy * fy).sqrt();
    if horizontal_len < 0.01 || !horizontal_len.is_finite() {
        return Err("Fly Mode stopped: local actor has no usable horizontal direction".into());
    }
    Ok(Position {
        x: fx / horizontal_len,
        y: fy / horizontal_len,
        z: 0.0,
    })
}

/// Pure movement math, independent of Windows keyboard APIs and memory writes.
/// W follows the actor's horizontal facing, D is local right, and Space/Ctrl
/// control vertical movement. WASD diagonals are normalized.
fn fly_step(anchor: Position, forward: Position, input: FlyInput, speed: f32, dt: f32) -> Position {
    let right = Position {
        x: forward.y,
        y: -forward.x,
        z: 0.0,
    };
    let mut horizontal_x = forward.x * input.forward + right.x * input.strafe;
    let mut horizontal_y = forward.y * input.forward + right.y * input.strafe;
    let length = (horizontal_x * horizontal_x + horizontal_y * horizontal_y).sqrt();
    if length > 1.0 {
        horizontal_x /= length;
        horizontal_y /= length;
    }
    let amount = speed * dt.clamp(0.0, 0.05);
    Position {
        x: anchor.x + horizontal_x * amount,
        y: anchor.y + horizontal_y * amount,
        z: anchor.z + input.vertical * amount,
    }
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
    use std::time::{Duration, Instant};

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
            return Err(format!(
                "Cannot verify Evolve process: {}",
                io::Error::last_os_error()
            ));
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
            if ok == 0
                || main_module.is_null()
                || needed < std::mem::size_of::<*mut c_void>() as u32
            {
                return Err("Cannot verify running Evolve module".into());
            }
            let mut name = [0u16; 260];
            let count = unsafe {
                K32GetModuleBaseNameW(handle, main_module, name.as_mut_ptr(), name.len() as u32)
            } as usize;
            if count == 0
                || !String::from_utf16_lossy(&name[..count]).eq_ignore_ascii_case("Evolve.exe")
            {
                return Err("Game PID no longer points to Evolve.exe; detect game again".into());
            }
            Ok(main_module as usize)
        })();
        unsafe { CloseHandle(handle) };
        result
    }

    fn session(snapshot: &Snapshot, _root: &str) -> Result<(u32, ProcessHandle, usize), String> {
        let pid = snapshot.pid.ok_or("Detect Evolve.exe first")?;
        let base = snapshot
            .module_base
            .ok_or("Evolve module base unavailable")?;
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
        client: usize,
        actor: usize,
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
            client,
            actor,
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

    /// Opens the game once for the duration of a coordinate hold; no repeated
    /// PowerShell/tasklist queries, Win32 process creation, injection or hook.
    /// The handle is scoped to the worker thread and is closed on all exits.
    #[allow(unsafe_code)]
    struct HoldWriter(*mut c_void);
    #[allow(unsafe_code)]
    impl HoldWriter {
        fn open(pid: u32) -> Result<Self, String> {
            use std::io;
            unsafe extern "system" {
                fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
            }
            const ACCESS: u32 = 0x0008 | 0x0010 | 0x0020;
            let handle = unsafe { OpenProcess(ACCESS, 0, pid) };
            if handle.is_null() {
                return Err(format!(
                    "Cannot open game for coordinate hold: {}",
                    io::Error::last_os_error()
                ));
            }
            Ok(Self(handle))
        }

        fn write(&self, address: usize, bytes: &[u8]) -> Result<(), String> {
            use std::io;
            unsafe extern "system" {
                fn WriteProcessMemory(
                    handle: *mut c_void,
                    address: *mut c_void,
                    bytes: *const c_void,
                    size: usize,
                    written: *mut usize,
                ) -> i32;
            }
            let mut written = 0usize;
            let ok = unsafe {
                WriteProcessMemory(
                    self.0,
                    address as *mut c_void,
                    bytes.as_ptr().cast(),
                    bytes.len(),
                    &mut written,
                )
            };
            if ok == 0 || written != bytes.len() {
                return Err(format!(
                    "Coordinate hold write failed: {}",
                    io::Error::last_os_error()
                ));
            }
            Ok(())
        }
    }
    #[allow(unsafe_code)]
    impl Drop for HoldWriter {
        fn drop(&mut self) {
            unsafe extern "system" {
                fn CloseHandle(handle: *mut c_void) -> i32;
            }
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    #[allow(unsafe_code)]
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> *mut c_void;
        fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
        fn GetAsyncKeyState(key: i32) -> i16;
    }

    #[allow(unsafe_code)]
    fn fly_input(game_pid: u32) -> FlyInput {
        let mut focused_pid = 0u32;
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.is_null() {
            return FlyInput::default();
        }
        unsafe { GetWindowThreadProcessId(foreground, &mut focused_pid) };
        if focused_pid != game_pid {
            // Do not consume manager keystrokes or fly when alt-tabbed.
            return FlyInput::default();
        }
        let pressed = |vk: i32| (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0;
        FlyInput {
            forward: i32::from(pressed(0x57)) as f32 - i32::from(pressed(0x53)) as f32,
            strafe: i32::from(pressed(0x44)) as f32 - i32::from(pressed(0x41)) as f32,
            vertical: i32::from(pressed(0x20)) as f32 - i32::from(pressed(0x11)) as f32,
        }
    }

    /// Repeatedly correct physics position at ~60 Hz. This is positional
    /// hovering, not a gravity/velocity override; some jitter can remain and
    /// releasing the hold permits normal gravity to resume.
    pub(crate) fn hold(
        snapshot: &Snapshot,
        root: &str,
        offline: bool,
        mode: HoldMode,
        fly_speed: f32,
        stop: Arc<AtomicBool>,
    ) -> Result<Reading, String> {
        if !offline {
            return Err("Confirm an isolated offline match".into());
        }
        if stop.load(Ordering::Acquire) {
            return Err("Coordinate hold canceled before starting".into());
        }
        if matches!(mode, HoldMode::Z | HoldMode::Fly)
            && (!fly_speed.is_finite() || !(1.0..=40.0).contains(&fly_speed))
        {
            return Err("Invalid hover/fly speed; enter 1 to 40 units/sec".into());
        }
        let r = resolve(snapshot, root)?;
        let (_, handle, base) = session(snapshot, root)?;
        let writer = HoldWriter::open(r.pid)?;
        let result = Reading {
            pid: r.pid,
            actor_id: r.actor_id,
            position: r.pos,
        };
        let mut fly_anchor = r.pos;
        let mut previous_tick = Instant::now();
        let mut override_ticks = 0u32;
        while !stop.load(Ordering::Acquire) {
            // Check that this is still the same controlled actor and validated
            // physics object. Never write to a stale/repurposed player pointer.
            if u32_at(&handle, r.client + 0x170)? != r.actor_id
                || q(&handle, r.client + 0x178)? != r.actor
                || q(&handle, r.actor + 0x38)? != r.entity
                || u32_at(&handle, r.entity + 0x18)? != r.actor_id
                || q(&handle, r.entity)? != base + ENTITY_VTABLE_RVA
                || q(&handle, r.physics)? != base + LIVING_VTABLE_RVA
            {
                return Err(
                    "Local actor or physics identity changed; coordinate hold stopped".into(),
                );
            }
            let current = position(&handle, r.physics, [0xB8, 0xBC, 0xC0])?;
            if !current.finite() {
                return Err("Physics XYZ became invalid; coordinate hold stopped".into());
            }
            let target = if matches!(mode, HoldMode::Z | HoldMode::Fly) {
                let now = Instant::now();
                let dt = now.duration_since(previous_tick).as_secs_f32();
                previous_tick = now;
                let mut input = fly_input(r.pid);
                // Freeze Z and Fly both move X/Y by writing position directly:
                // neither relies on native ground locomotion in mid-air.
                if mode == HoldMode::Z {
                    input.vertical = 0.0;
                }
                if input.active() {
                    let facing = [
                        u32_at(&handle, r.entity + 0x4C)?,
                        u32_at(&handle, r.entity + 0x50)?,
                        u32_at(&handle, r.entity + 0x54)?,
                        u32_at(&handle, r.entity + 0x58)?,
                    ]
                    .map(f32::from_bits);
                    let forward = fly_forward(facing)?;
                    fly_anchor = fly_step(fly_anchor, forward, input, fly_speed, dt);
                    if !fly_anchor.finite() {
                        return Err("Hover/Fly Mode reached out-of-range world coordinates".into());
                    }
                }
                // Engine movement may overwrite the external physics position
                // at the next tick. If it keeps snapping >3 units behind the
                // commanded point while movement is held, fail visibly instead
                // of pretending flight works. A collision may cause this too.
                let dx = fly_anchor.x - current.x;
                let dy = fly_anchor.y - current.y;
                let dz = fly_anchor.z - current.z;
                let lag = (dx * dx + dy * dy + dz * dz).sqrt();
                if input.active() && lag > 3.0 {
                    override_ticks += 1;
                } else {
                    override_ticks = 0;
                }
                if override_ticks >= 12 {
                    return Err(
                        "Hover/Fly movement is being overridden by the game or collision; verify the live physics position/movement state before using flight".into()
                    );
                }
                fly_anchor
            } else {
                held_position(r.pos, current, mode)
            };
            if (target.x - current.x).abs() > 0.01
                || (target.y - current.y).abs() > 0.01
                || (target.z - current.z).abs() > 0.01
            {
                // XYZ lock and controlled hovering/flight write the verified
                // physics position together. Gravity/velocity remain untouched.
                match mode {
                    HoldMode::Z | HoldMode::Xyz | HoldMode::Fly => {
                        let mut bytes = [0u8; 12];
                        for (index, value) in [target.x, target.y, target.z].iter().enumerate() {
                            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
                        }
                        writer.write(r.physics + 0xB8, &bytes)?;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        let position = position(&handle, r.physics, [0xB8, 0xBC, 0xC0])?;
        if !position.finite() {
            return Err("Player position became invalid after coordinate hold".into());
        }
        Ok(Reading { position, ..result })
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
pub(super) use windows::{hold, inspect, preview_aim, teleport};
#[cfg(not(windows))]
pub(super) fn hold(
    _: &Snapshot,
    _: &str,
    _: bool,
    _: HoldMode,
    _: f32,
    _: Arc<AtomicBool>,
) -> Result<Reading, String> {
    Err("Coordinate hold requires Windows".into())
}
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
    fn hover_holds_only_height_and_xyz_locks_all_axes() {
        let anchor = Position {
            x: 12.0,
            y: 34.0,
            z: 56.0,
        };
        let current = Position {
            x: 91.0,
            y: 82.0,
            z: -3.0,
        };
        assert_eq!(
            held_position(anchor, current, HoldMode::Z),
            Position {
                x: 91.0,
                y: 82.0,
                z: 56.0
            }
        );
        assert_eq!(held_position(anchor, current, HoldMode::Xyz), anchor);
    }
    #[test]
    fn speed_rejects_invalid_values() {
        assert_eq!(parse_fly_speed("8").unwrap(), 8.0);
        assert!(parse_fly_speed("0").is_err());
        assert!(parse_fly_speed("100").is_err());
        assert!(parse_fly_speed("NaN").is_err());
        assert!(parse_fly_speed("inf").is_err());
        assert!(parse_fly_speed("hello").is_err());
    }

    #[test]
    fn flight_forward_and_strafe_follow_actor_yaw() {
        let north = fly_forward([1.0, 0.0, 0.0, 0.0]).unwrap();
        assert!(north.near(Position {
            x: 0.0,
            y: 1.0,
            z: 0.0
        }));
        let origin = Position {
            x: 10.0,
            y: 20.0,
            z: 30.0,
        };
        let forward = fly_step(
            origin,
            north,
            FlyInput {
                forward: 1.0,
                ..FlyInput::default()
            },
            10.0,
            0.1,
        );
        assert!((forward.y - 20.5).abs() < 0.001);
        let strafe = fly_step(
            origin,
            north,
            FlyInput {
                strafe: 1.0,
                ..FlyInput::default()
            },
            10.0,
            0.05,
        );
        assert!((strafe.x - 10.5).abs() < 0.001);
        assert!(fly_forward([0.0; 4]).is_err());
    }

    #[test]
    fn freeze_z_uses_explicit_xy_motion_and_never_changes_height() {
        let anchor = Position {
            x: 1.0,
            y: 2.0,
            z: 50.0,
        };
        let north = Position {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };
        // This is the same movement math that drives both Z hover and Fly,
        // with vertical deliberately ignored for Z hover.
        let input = FlyInput {
            forward: 1.0,
            vertical: 1.0,
            ..FlyInput::default()
        };
        let target = fly_step(
            anchor,
            north,
            FlyInput {
                vertical: 0.0,
                ..input
            },
            8.0,
            0.05,
        );
        assert!((target.y - 2.4).abs() < 0.001);
        assert_eq!(target.z, 50.0);
    }

    #[test]
    fn flight_vertical_and_diagonal_speed_are_bounded() {
        let origin = Position {
            x: 0.0,
            y: 0.0,
            z: 100.0,
        };
        let forward = Position {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };
        let climb = fly_step(
            origin,
            forward,
            FlyInput {
                vertical: 1.0,
                ..FlyInput::default()
            },
            8.0,
            0.05,
        );
        assert!((climb.z - 100.4).abs() < 0.001);
        let diagonal = fly_step(
            origin,
            forward,
            FlyInput {
                forward: 1.0,
                strafe: 1.0,
                ..FlyInput::default()
            },
            8.0,
            0.05,
        );
        assert!((diagonal.x * diagonal.x + diagonal.y * diagonal.y).sqrt() <= 0.401);
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
