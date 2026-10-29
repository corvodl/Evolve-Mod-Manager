//! Explicit, experimental global AI-update pause through a user-verified
//! CryEngine ai_NoUpdate Int32 storage address. The current Evolve binary's
//! IConsole/ICVar layout has not been validated: NEVER guess a CVar address,
//! infer it from nearby strings, or advertise automatic discovery.
//!
//! This is a local process-memory edit, not a console command. Only use in an
//! isolated testing session. Changes are 0 -> 1 -> 0 with bit-exact compare,
//! readback, and PID/main-module identity checks.
use crate::runtime_memory::{self, Snapshot};

#[derive(Clone, Copy, Debug)]
pub(super) struct Probe {
    pub pid: u32,
    pub module_base: usize,
    pub address: usize,
    pub value: u32,
}

impl Probe {
    pub fn describe(self) -> String {
        format!("PID {} · ai_NoUpdate candidate 0x{:X} · Int32 {} (address identity must be verified externally)",
            self.pid, self.address, self.value)
    }
}

fn acceptable_value(value: u32) -> Result<(), String> {
    if value > 1 {
        Err("Expected an Int32 0 or 1; this is not a usable AI update flag address".into())
    } else {
        Ok(())
    }
}

fn valid_address(address: usize) -> Result<(), String> {
    if !(0x10000..=0x0000_7FFF_FFFF_FFFCusize).contains(&address) || !address.is_multiple_of(4) {
        Err("Enter a 4-byte-aligned user-mode CVar value address".into())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn session(snapshot: &Snapshot, game_root: &str) -> Result<(u32, usize), String> {
    let pid = snapshot.pid.ok_or("Detect Evolve.exe first")?;
    let base = snapshot.module_base.ok_or("Evolve module base unavailable")?;
    let now = runtime_memory::inspect(game_root);
    if now.pid != Some(pid) || now.module_base != Some(base) {
        return Err("Game PID/module changed; rediscover the verified ai_NoUpdate address".into());
    }
    Ok((pid, base))
}

#[cfg(windows)]
fn read_i32(pid: u32, address: usize) -> Result<u32, String> {
    let handle: read_process_memory::ProcessHandle = pid
        .try_into()
        .map_err(|e| format!("Cannot read Evolve: {e}"))?;
    let bytes = read_process_memory::copy_address(address, 4, &handle)
        .map_err(|e| format!("Cannot read ai_NoUpdate candidate: {e}"))?;
    let raw: [u8; 4] = bytes.as_slice().try_into()
        .map_err(|_| "Incomplete Int32 read")?;
    Ok(u32::from_le_bytes(raw))
}

/// Does not prove that this address belongs to ai_NoUpdate. A 0/1 value is
/// necessary but not sufficient; the user must verify the CVar's identity.
#[cfg(windows)]
pub(super) fn inspect(snapshot: &Snapshot, root: &str, input: &str) -> Result<Probe, String> {
    let (pid, module_base) = session(snapshot, root)?;
    let address = runtime_memory::resolve_address(snapshot, input)?;
    valid_address(address)?;
    let value = read_i32(pid, address)?;
    acceptable_value(value)?;
    let (pid_again, base_again) = session(snapshot, root)?;
    if pid != pid_again || module_base != base_again || read_i32(pid, address)? != value {
        return Err("CVar value or game session changed during inspection".into());
    }
    Ok(Probe { pid, module_base, address, value })
}

/// One-shot 0->1 to pause and 1->0 to restore. Ownership of a paused value is
/// tracked separately by the manager; we never resume a pre-existing 1.
#[cfg(windows)]
pub(super) fn change(
    snapshot: &Snapshot,
    root: &str,
    probe: Probe,
    pause: bool,
) -> Result<Probe, String> {
    let expected = if pause { 0 } else { 1 };
    let new = if pause { 1 } else { 0 };
    if probe.value != expected {
        return Err(format!("Expected original Int32 {expected}; inspect the address again")).into();
    }
    valid_address(probe.address)?;
    let (pid, base) = session(snapshot, root)?;
    if pid != probe.pid || base != probe.module_base {
        return Err("Game session changed; cannot apply old CVar address".into());
    }
    let current = read_i32(pid, probe.address)?;
    if current != expected {
        return Err("AI update flag changed; no write performed".into());
    }
    runtime_live_write(pid, probe.address, expected, new)?;
    if read_i32(pid, probe.address)? != new {
        return Err("Write readback differs; AI state is not confirmed".into());
    }
    Ok(Probe { value: new, ..probe })
}

#[cfg(windows)]
fn runtime_live_write(pid: u32, address: usize, expected: u32, desired: u32) -> Result<(), String> {
    crate::runtime_live::write_four(pid, address, expected, desired)
}

#[cfg(not(windows))]
pub(super) fn inspect(_: &Snapshot, _: &str, _: &str) -> Result<Probe, String> {
    Err("AI memory control requires Windows".into())
}

#[cfg(not(windows))]
pub(super) fn change(_: &Snapshot, _: &str, _: Probe, _: bool) -> Result<Probe, String> {
    Err("AI memory control requires Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_accepts_only_int32_boolean() {
        assert!(acceptable_value(0).is_ok());
        assert!(acceptable_value(1).is_ok());
        assert!(acceptable_value(2).is_err());
        assert!(acceptable_value(u32::MAX).is_err());
    }

    #[test]
    fn address_is_aligned_and_in_user_space() {
        assert!(valid_address(0x10000).is_ok());
        assert!(valid_address(0x10001).is_err());
        assert!(valid_address(0).is_err());
        assert!(valid_address(usize::MAX).is_err());
    }
}
