//! Read-only, opt-in Stage 2 Goliath armor discovery.
//! No debug breakpoints, pointer scans, remote threads, injection, or writes.
//! Address matches remain candidates until they follow damage AND feeding.
//!
//! Windows memory-region metadata comes from VirtualQueryEx through a short,
//! hidden, query-only PowerShell helper. Reads use read-process-memory.
use crate::runtime_memory::{self, Snapshot};
use std::sync::atomic::{AtomicBool, Ordering};

const FULL_ARMOR: f32 = 1000.0; // Observed internal Stage 1 Goliath scale; NOT HUD armor HP.
const INITIAL_TOLERANCE: f32 = 25.0;
const MAX_CANDIDATES: usize = 300_000;
const MAX_SCAN_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const READ_CHUNK: usize = 64 * 1024;
const PAGE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Damage,
    Stable,
    Feed,
}

#[derive(Clone, Debug)]
struct Candidate {
    address: usize,
    value: f32,
}

#[derive(Clone, Debug)]
pub(super) struct Scan {
    pub pid: u32,
    candidates: Vec<Candidate>,
    damage_steps: u8,
    feed_steps: u8,
    stable_steps: u8,
    pub pages_read: usize,
}

impl Scan {
    pub fn ready(&self) -> bool {
        self.candidates.len() == 1 && self.damage_steps > 0 && self.feed_steps > 0
    }

    pub fn describe(&self) -> String {
        let mut description = format!(
            "Stage 2 Goliath armor (Float32): {} candidate(s), {} pages sampled. Damage passes: {}; feed passes: {}; stable passes: {}. READ ONLY.",
            self.candidates.len(),
            self.pages_read,
            self.damage_steps,
            self.feed_steps,
            self.stable_steps
        );
        for candidate in self.candidates.iter().take(12) {
            description.push_str(&format!(
                "\n0x{:X}: {:.3}",
                candidate.address, candidate.value
            ));
        }
        if self.candidates.len() > 12 {
            description
                .push_str("\nShowing 12 candidates. Alternate damage, stable and feeding filters.");
        }
        if self.ready() {
            description.push_str("\nBehavior verified across damage and feeding. Address is session-local; actor class is not independently verified. Live read enabled.");
        } else if self.candidates.is_empty() {
            description.push_str("\nNo candidates remain; refill armor and begin a new scan.");
        } else {
            description.push_str("\nGuided: take armor damage -> Damage; stay still -> Stable; feed to regain armor -> Feed. Do not use a step unless the HUD visibly changed as expected.");
        }
        description
    }

    pub(super) fn verified_address(&self) -> Option<usize> {
        self.ready().then(|| self.candidates[0].address)
    }
}

fn armor_initial(value: f32) -> bool {
    value.is_finite() && (value - FULL_ARMOR).abs() <= INITIAL_TOLERANCE
}

fn plausible(value: f32) -> bool {
    value.is_finite() && (0.0..=150_000.0).contains(&value)
}

fn matches_step(previous: f32, current: f32, step: Step) -> bool {
    if !plausible(current) || !plausible(previous) {
        return false;
    }
    match step {
        Step::Damage => current < previous - 0.001,
        Step::Stable => (current - previous).abs() <= 0.001,
        Step::Feed => current > previous + 0.001,
    }
}

fn parse_regions(output: &str) -> Result<Vec<(usize, usize)>, String> {
    let mut regions = Vec::new();
    for line in output.lines() {
        let Some((base, len)) = line.trim().split_once('|') else {
            continue;
        };
        let base = usize::from_str_radix(base, 16).map_err(|_| "Invalid region base")?;
        let len = usize::from_str_radix(len, 16).map_err(|_| "Invalid region length")?;
        if base >= 0x10000 && len >= PAGE && base.checked_add(len).is_some() {
            regions.push((base, len));
        }
        if regions.len() > 50_000 {
            return Err("Too many memory regions; scan aborted".into());
        }
    }
    Ok(regions)
}

#[cfg(windows)]
fn ps(command: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("Windows memory query unavailable: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Windows memory query failed: {}",
            String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(windows)]
fn verify_process(snapshot: &Snapshot, game_root: &str) -> Result<u32, String> {
    let pid = snapshot
        .pid
        .ok_or("Detect a unique running Evolve.exe first")?;
    if runtime_memory::inspect(game_root).pid != Some(pid) {
        return Err("Evolve restarted or PID changed. Candidate addresses discarded.".into());
    }
    Ok(pid)
}

/// Enumerates *committed writable* regions only; no debugger and no injected
/// code. PowerShell compiles a query-only Win32 wrapper in its own process.
#[cfg(windows)]
fn regions(pid: u32) -> Result<Vec<(usize, usize)>, String> {
    let script = format!(
        r#"
$ErrorActionPreference='Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class EvolveReadOnlyRegions {{
 [StructLayout(LayoutKind.Sequential)]
 struct MBI {{
  public IntPtr BaseAddress;
  public IntPtr AllocationBase;
  public uint AllocationProtect;
  public ushort PartitionId;
  public UIntPtr RegionSize;
  public uint State;
  public uint Protect;
  public uint Type;
 }}
 [DllImport("kernel32.dll", SetLastError=true)]
 static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
 [DllImport("kernel32.dll", SetLastError=true)]
 static extern bool CloseHandle(IntPtr process);
 [DllImport("kernel32.dll", SetLastError=true)]
 static extern UIntPtr VirtualQueryEx(IntPtr process, IntPtr address, out MBI info, UIntPtr size);
 public static string[] List(uint pid) {{
  var result=new List<string>();
  IntPtr process=OpenProcess(0x0400,false,pid);
  if(process==IntPtr.Zero) throw new Exception("Cannot query game process memory");
  try {{
   ulong at=0x10000;
   ulong max=0x00007FFFFFFF0000;
   while(at<max) {{
    MBI info;
    if(VirtualQueryEx(process,new IntPtr(unchecked((long)at)),out info,new UIntPtr((uint)Marshal.SizeOf(typeof(MBI))))==UIntPtr.Zero) break;
    ulong baseAddr=unchecked((ulong)info.BaseAddress.ToInt64());
    ulong size=info.RegionSize.ToUInt64();
    if(size==0 || baseAddr>ulong.MaxValue-size) break;
    ulong next=baseAddr+size;
    uint protection=info.Protect & 0xFF;
    bool writable=protection==0x04 || protection==0x08 || protection==0x40 || protection==0x80;
    if(info.State==0x1000 && writable && (info.Protect & 0x100)==0 && size>=4096)
      result.Add(baseAddr.ToString("X")+"|"+size.ToString("X"));
    if(next<=at) break;
    at=next;
   }}
  }} finally {{ CloseHandle(process); }}
  return result.ToArray();
 }}
}}
'@
[EvolveReadOnlyRegions]::List({pid})
"#
    );
    let region_list = parse_regions(&ps(&script)?)?;
    if region_list.is_empty() {
        return Err("No committed writable regions returned. Check Evolve process access.".into());
    }
    Ok(region_list)
}

#[cfg(windows)]
pub(super) fn discover(
    snapshot: &Snapshot,
    game_root: &str,
    _offline_confirmed: bool,
    cancel: &AtomicBool,
) -> Result<Scan, String> {
    let pid = verify_process(snapshot, game_root)?;
    
    let regions = regions(pid)?;
    let handle: read_process_memory::ProcessHandle = pid
        .try_into()
        .map_err(|e| format!("Cannot open game read-only: {e}"))?;
    let mut candidates = Vec::new();
    let mut bytes_scanned = 0_u64;
    let mut pages_read = 0_usize;
    for (base, length) in regions {
        let mut offset = 0_usize;
        while offset < length {
            if cancel.load(Ordering::Relaxed) {
                return Err("Armor scan canceled".into());
            }
            let next = READ_CHUNK.min(length - offset);
            bytes_scanned = bytes_scanned.saturating_add(next as u64);
            if bytes_scanned > MAX_SCAN_BYTES {
                return Err(
                    "Memory scan reached its 8 GiB safety limit; no partial result accepted".into(),
                );
            }
            if let Some(address) = base.checked_add(offset)
                && let Ok(bytes) = read_process_memory::copy_address(address, next, &handle)
            {
                pages_read += bytes.len().div_ceil(PAGE);
                for (i, part) in bytes.as_chunks::<4>().0.iter().enumerate() {
                    let value = f32::from_le_bytes(*part);
                    if armor_initial(value) {
                        candidates.push(Candidate {
                            address: address + i * 4,
                            value,
                        });
                        if candidates.len() > MAX_CANDIDATES {
                            return Err("Over 300000 near-full armor candidates; refine the initial tolerance before scanning again".into());
                        }
                    }
                }
            }
            offset += next;
        }
    }
    if runtime_memory::inspect(game_root).pid != Some(pid) {
        return Err("Evolve restarted during scan; partial candidates discarded".into());
    }
    Ok(Scan {
        pid,
        candidates,
        damage_steps: 0,
        feed_steps: 0,
        stable_steps: 0,
        pages_read,
    })
}

#[cfg(windows)]
pub(super) fn refine(
    snapshot: &Snapshot,
    game_root: &str,
    previous: Scan,
    step: Step,
    _offline_confirmed: bool,
    cancel: &AtomicBool,
) -> Result<Scan, String> {
    let pid = verify_process(snapshot, game_root)?;
    if pid != previous.pid {
        return Err("Different Evolve instance; begin another full armor scan".into());
    }
    
    if step == Step::Feed && previous.damage_steps == 0 {
        return Err("Take armor damage and filter Damage before feeding".into());
    }
    let handle: read_process_memory::ProcessHandle = pid
        .try_into()
        .map_err(|e| format!("Cannot open game read-only: {e}"))?;
    let mut surviving = Vec::new();
    let mut i = 0;
    while i < previous.candidates.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Armor refinement canceled".into());
        }
        let page_base = previous.candidates[i].address & !(PAGE - 1);
        let bytes = read_process_memory::copy_address(page_base, PAGE, &handle).ok();
        while i < previous.candidates.len()
            && (previous.candidates[i].address & !(PAGE - 1)) == page_base
        {
            let prior = &previous.candidates[i];
            if let Some(raw) = bytes.as_ref().and_then(|bytes| {
                let offset = prior.address - page_base;
                bytes.get(offset..offset + 4)
            }) {
                let value = f32::from_le_bytes(raw.try_into().unwrap());
                if matches_step(prior.value, value, step) {
                    surviving.push(Candidate {
                        address: prior.address,
                        value,
                    });
                }
            }
            i += 1;
        }
    }
    if runtime_memory::inspect(game_root).pid != Some(pid) {
        return Err("Evolve restarted during refinement; candidates discarded".into());
    }
    Ok(Scan {
        pid,
        candidates: surviving,
        damage_steps: previous.damage_steps + u8::from(step == Step::Damage),
        stable_steps: previous.stable_steps + u8::from(step == Step::Stable),
        feed_steps: previous.feed_steps + u8::from(step == Step::Feed),
        pages_read: previous.pages_read,
    })
}

#[cfg(windows)]
pub(super) fn monitor(
    snapshot: &Snapshot,
    game_root: &str,
    scan: &Scan,
    _offline_confirmed: bool,
) -> Result<Option<f32>, String> {
    let Some(address) = scan.verified_address() else {
        return Ok(None);
    };
    let pid = verify_process(snapshot, game_root)?;
    if pid != scan.pid {
        return Err("Game restarted; armor address is stale".into());
    }
    
    let handle: read_process_memory::ProcessHandle = pid
        .try_into()
        .map_err(|e| format!("Cannot open game read-only: {e}"))?;
    let raw = read_process_memory::copy_address(address, 4, &handle)
        .map_err(|e| format!("Armor address unreadable: {e}"))?;
    let value = f32::from_le_bytes(raw.as_slice().try_into().map_err(|_| "Short armor read")?);
    if !plausible(value) {
        return Err("Armor address is no longer a plausible Float32; rescan needed".into());
    }
    Ok(Some(value))
}

#[cfg(not(windows))]
pub(super) fn discover(_: &Snapshot, _: &str, _: bool, _: &AtomicBool) -> Result<Scan, String> {
    Err("Stage 2 armor detection requires Windows".into())
}
#[cfg(not(windows))]
pub(super) fn refine(
    _: &Snapshot,
    _: &str,
    _: Scan,
    _: Step,
    _: bool,
    _: &AtomicBool,
) -> Result<Scan, String> {
    Err("Stage 2 armor detection requires Windows".into())
}
#[cfg(not(windows))]
pub(super) fn monitor(_: &Snapshot, _: &str, _: &Scan, _: bool) -> Result<Option<f32>, String> {
    Err("Stage 2 armor detection requires Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_float_checks_allow_observed_scale_but_reject_nan_and_wrong_types() {
        assert!(armor_initial(997.6));
        assert!(armor_initial(1000.0));
        assert!(!armor_initial(f32::NAN));
        assert!(!armor_initial(5000.0));
        assert!(!plausible(f32::INFINITY));
        assert!(plausible(0.0));
    }

    #[test]
    fn guided_damage_feed_and_stable_behavior() {
        assert!(matches_step(997.6, 500.0, Step::Damage));
        assert!(matches_step(500.0, 500.0, Step::Stable));
        assert!(matches_step(500.0, 780.0, Step::Feed));
        assert!(!matches_step(500.0, 400.0, Step::Feed));
        assert!(!matches_step(500.0, f32::NAN, Step::Feed));
    }

    #[test]
    fn parses_committed_regions_and_skips_malformed_records() {
        let regions = parse_regions("20000000|1000\n0|1000\n20001000|5000\n").unwrap();
        assert_eq!(regions, vec![(0x20000000, 0x1000), (0x20001000, 0x5000)]);
        assert!(parse_regions("GG|1000").is_err());
    }

    #[test]
    fn live_armor_requires_a_damage_and_feed_match() {
        let candidate = Candidate {
            address: 0x20000000,
            value: 997.0,
        };
        let scan = Scan {
            pid: 123,
            candidates: vec![candidate],
            damage_steps: 1,
            feed_steps: 0,
            stable_steps: 1,
            pages_read: 1,
        };
        assert!(!scan.ready());
        assert!(scan.verified_address().is_none());
        let ready = Scan {
            feed_steps: 1,
            ..scan
        };
        assert!(ready.ready());
        assert_eq!(ready.verified_address(), Some(0x20000000));
    }
}
