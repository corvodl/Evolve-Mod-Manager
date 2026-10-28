//! Explicitly armed, single-shot Float32 edits for OFFLINE developer tests.
//! Never auto-writes, freezes, injects, sets debugger breakpoints, or changes
//! memory protection. A read-before-write probe is tied to a process and a
//! specific four-byte baseline; applying always requires fresh authorization.
use crate::runtime_armor;
use crate::runtime_memory::{self, Snapshot};
use std::sync::atomic::{AtomicBool, Ordering};

const MIN_USER_ADDRESS: usize = 0x10000;
const MAX_USER_ADDRESS: usize = 0x0000_7FFF_FFFF_FFFC;
const MAX_WRITE_VALUE: f32 = 100_000.0;

// Legacy manual Float32 actions are retained for compatibility but hidden by the new actor UI.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Source {
    VerifiedArmor,
    ManualFloat,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::VerifiedArmor => "Guided armor candidate",
            Self::ManualFloat => "Manual Float32 (unverified)",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Probe {
    pub pid: u32,
    pub address: usize,
    pub source: Source,
    pub previous: f32,
    expected_bits: u32,
}

impl Probe {
    pub fn describe(&self) -> String {
        format!(
            "{}: PID {}, address 0x{:X}, current Float32 {:.4}. Read-only probe complete. Enter a new value and explicitly click Write once.",
            self.source.label(),
            self.pid,
            self.address,
            self.previous
        )
    }
}

#[derive(Clone, Debug)]
pub(super) struct WriteResult {
    pub address: usize,
    pub previous: f32,
    pub applied: f32,
}

fn value(input: &str) -> Result<f32, String> {
    let parsed = input
        .trim()
        .parse::<f32>()
        .map_err(|_| "Enter a Float32 number (0 to 100000)")?;
    if !parsed.is_finite() || !(0.0..=MAX_WRITE_VALUE).contains(&parsed) {
        return Err("Float32 write must be finite and between 0 and 100000".into());
    }
    Ok(parsed)
}

fn validate_address(at: usize) -> Result<(), String> {
    if !(MIN_USER_ADDRESS..=MAX_USER_ADDRESS).contains(&at) || !at.is_multiple_of(4) {
        return Err("Address must be a 4-byte-aligned 64-bit user-mode address".into());
    }
    Ok(())
}

fn validate_baseline(expected: u32, current: u32) -> Result<(), String> {
    if current != expected {
        return Err(
            "Value changed since it was read. Read current Float32 again before writing.".into(),
        );
    }
    Ok(())
}

#[cfg(windows)]
fn verify_session(snapshot: &Snapshot, game_root: &str, confirmed: bool) -> Result<u32, String> {
    let pid = snapshot
        .pid
        .ok_or("Detect one running Evolve.exe process")?;
    let current = runtime_memory::inspect(game_root);
    if current.pid != Some(pid) {
        return Err("Evolve restarted; old addresses cannot be edited".into());
    }
    if let (Some(initial), Some(now)) = (snapshot.module_base, current.module_base)
        && initial != now
    {
        return Err("Evolve image changed; redetect the process before editing".into());
    }
    // Fails closed when the Windows TCP query fails. This is only a best-effort
    // online detection, not proof of the game session's actual offline status.
    runtime_armor::offline_guard(pid, confirmed)?;
    Ok(pid)
}

#[cfg(windows)]
fn current_bits(pid: u32, at: usize) -> Result<u32, String> {
    let handle: read_process_memory::ProcessHandle = pid
        .try_into()
        .map_err(|e| format!("Could not open Evolve for read access: {e}"))?;
    let bytes = read_process_memory::copy_address(at, 4, &handle)
        .map_err(|e| format!("Cannot read Float32 address: {e}"))?;
    let raw: [u8; 4] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| "Incomplete Float32 read")?;
    Ok(u32::from_le_bytes(raw))
}

#[cfg(windows)]
pub(super) fn probe(
    snapshot: &Snapshot,
    game_root: &str,
    source: Source,
    address: usize,
    confirmed: bool,
) -> Result<Probe, String> {
    let pid = verify_session(snapshot, game_root, confirmed)?;
    validate_address(address)?;
    let raw = current_bits(pid, address)?;
    let previous = f32::from_bits(raw);
    if !previous.is_finite() || previous.abs() > 150_000.0 {
        return Err(
            "Float32 at this address is nonfinite or implausible; refusing to arm it".into(),
        );
    }
    verify_session(snapshot, game_root, confirmed)?;
    Ok(Probe {
        pid,
        address,
        source,
        previous,
        expected_bits: raw,
    })
}

/// Compare and write four bytes using a single native process handle.
/// Avoid launching a new PowerShell CLR for every write or freeze tick.
#[cfg(windows)]
#[allow(unsafe_code)] // Scoped, reviewed FFI needed for verified offline process-memory writes.
pub(super) fn write_four(pid: u32, at: usize, old_bits: u32, new_bits: u32) -> Result<(), String> {
    use std::ffi::c_void;
    use std::io;
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn ReadProcessMemory(
            handle: *mut c_void,
            address: *const c_void,
            buffer: *mut c_void,
            size: usize,
            read: *mut usize,
        ) -> i32;
        fn WriteProcessMemory(
            handle: *mut c_void,
            address: *mut c_void,
            buffer: *const c_void,
            size: usize,
            written: *mut usize,
        ) -> i32;
    }
    validate_address(at)?;
    const PROCESS_VM_OPERATION: u32 = 0x0008;
    const PROCESS_VM_READ: u32 = 0x0010;
    const PROCESS_VM_WRITE: u32 = 0x0020;
    let access = PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE;
    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        return Err(format!(
            "OpenProcess PID {pid} failed: {}",
            io::Error::last_os_error()
        ));
    }
    let result = (|| {
        let mut observed = [0u8; 4];
        let mut count = 0usize;
        let ok = unsafe {
            ReadProcessMemory(
                handle,
                at as *const c_void,
                observed.as_mut_ptr().cast(),
                4,
                &mut count,
            )
        };
        if ok == 0 || count != 4 {
            return Err(format!(
                "ReadProcessMemory at 0x{at:X} failed: {}",
                io::Error::last_os_error()
            ));
        }
        if u32::from_le_bytes(observed) != old_bits {
            return Err("Value changed before write; refresh the actor field".into());
        }
        let desired = new_bits.to_le_bytes();
        count = 0;
        let ok = unsafe {
            WriteProcessMemory(
                handle,
                at as *mut c_void,
                desired.as_ptr().cast(),
                4,
                &mut count,
            )
        };
        if ok == 0 || count != 4 {
            return Err(format!(
                "WriteProcessMemory at 0x{at:X} failed: {}",
                io::Error::last_os_error()
            ));
        }
        count = 0;
        let ok = unsafe {
            ReadProcessMemory(
                handle,
                at as *const c_void,
                observed.as_mut_ptr().cast(),
                4,
                &mut count,
            )
        };
        if ok == 0 || count != 4 || observed != desired {
            return Err("Write was attempted, but immediate readback did not match".into());
        }
        Ok(())
    })();
    unsafe { CloseHandle(handle) };
    result
}

#[cfg(windows)]
pub(super) fn apply(
    snapshot: &Snapshot,
    game_root: &str,
    probe: Probe,
    desired: &str,
    confirmed: bool,
    cancel: &AtomicBool,
) -> Result<WriteResult, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Offline live edit was canceled".into());
    }
    let next = value(desired)?;
    validate_address(probe.address)?;
    let pid = verify_session(snapshot, game_root, confirmed)?;
    if pid != probe.pid {
        return Err("Evolve PID changed since Float32 was read; refusing write".into());
    }
    validate_baseline(probe.expected_bits, current_bits(pid, probe.address)?)?;
    verify_session(snapshot, game_root, confirmed)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Offline live edit canceled before write".into());
    }
    write_four(pid, probe.address, probe.expected_bits, next.to_bits())?;
    let observed = current_bits(pid, probe.address)?;
    if observed != next.to_bits() {
        return Err("Write attempted, but readback differs; address may update in-game. No persistent write claimed.".into());
    }
    verify_session(snapshot, game_root, confirmed)?;
    Ok(WriteResult {
        address: probe.address,
        previous: probe.previous,
        applied: next,
    })
}

#[cfg(not(windows))]
pub(super) fn probe(_: &Snapshot, _: &str, _: Source, _: usize, _: bool) -> Result<Probe, String> {
    Err("Offline Float32 edits require Windows".into())
}

#[cfg(not(windows))]
pub(super) fn apply(
    _: &Snapshot,
    _: &str,
    _: Probe,
    _: &str,
    _: bool,
    _: &AtomicBool,
) -> Result<WriteResult, String> {
    Err("Offline Float32 edits require Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_nan_inf_negatives_and_oversized_values() {
        for bad in ["", "NaN", "inf", "-1", "100001", "hello"] {
            assert!(value(bad).is_err(), "{bad} should be rejected");
        }
        assert_eq!(value("100000").unwrap(), 100_000.0);
        assert_eq!(value("0").unwrap(), 0.0);
    }

    #[test]
    fn reject_misaligned_or_kernel_addresses() {
        assert!(validate_address(0x20000000).is_ok());
        assert!(validate_address(0x20000001).is_err());
        assert!(validate_address(0x7fff_ffff_fffc).is_ok());
        assert!(validate_address(usize::MAX).is_err());
        assert!(validate_address(0).is_err());
    }

    #[test]
    fn baseline_compare_is_bit_exact() {
        assert!(validate_baseline(1000.0f32.to_bits(), 1000.0f32.to_bits()).is_ok());
        assert!(validate_baseline(1000.0f32.to_bits(), 990.0f32.to_bits()).is_err());
    }
}
