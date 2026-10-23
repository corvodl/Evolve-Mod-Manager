//! Opt-in external runtime inspection for the password-gated Developer page.
//! All operations are READ-ONLY. This module never injects, writes memory,
//! changes a game file, hooks a network function, or guesses an Evolve offset.
//! No offsets are shipped: live object layouts must be validated per build.
use std::path::Path;

#[derive(Debug, Clone)]
pub(super) struct Snapshot {
    pub pid: Option<u32>,
    pub status: String,
    pub executable: String,
}

fn parse_tasklist(output: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    for line in output.lines() {
        // /FO CSV quotes the image name and PID; never parse the localized
        // memory-usage field (which may contain thousands separators).
        let columns: Vec<&str> = line.trim().split('"').collect();
        if columns.len() > 4 && columns[1].eq_ignore_ascii_case("Evolve.exe") {
            if let Ok(pid) = columns[3].parse::<u32>() {
                if pid != 0 && !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
        }
    }
    pids
}

pub(super) fn inspect(game_root: &str) -> Snapshot {
    let executable = Path::new(game_root).join("bin64_SteamRetail").join("Evolve.exe");
    let executable = if game_root.is_empty() {
        "Configured game root missing; running binary cannot be verified".to_owned()
    } else if executable.is_file() {
        "Configured Stage 2 Evolve.exe exists (running image path not verified)".to_owned()
    } else {
        "Configured Stage 2 executable not found (running image path not verified)".to_owned()
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Do not flash a console window for diagnostics.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let output = std::process::Command::new("tasklist")
            .args(["/FO", "CSV", "/NH", "/FI", "IMAGENAME eq Evolve.exe"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let pids = parse_tasklist(&String::from_utf8_lossy(&output.stdout));
                match pids.as_slice() {
                    [] => Snapshot { pid: None, status: "No Evolve.exe process detected".into(), executable },
                    [pid] => Snapshot {
                        pid: Some(*pid),
                        status: format!("Evolve.exe PID {pid} detected. Read-only inspection; offline status and exact executable identity NOT verified."),
                        executable,
                    },
                    _ => Snapshot {
                        pid: None,
                        status: format!("{} Evolve processes detected. Close extras before inspecting; no PID was guessed.", pids.len()),
                        executable,
                    },
                }
            }
            Ok(output) => Snapshot {
                pid: None,
                status: format!("Process discovery unavailable (exit {})", output.status),
                executable,
            },
            Err(error) => Snapshot { pid: None, status: format!("Process discovery unavailable: {error}"), executable },
        }
    }
    #[cfg(not(windows))]
    {
        let _ = game_root;
        Snapshot { pid: None, status: "External Evolve memory inspection requires Windows".into(), executable }
    }
}

/// Requires explicit hexadecimal input; never treats untrusted decimal text
/// as a pointer. Reject excessively long or invalid pointer chains.
fn parse_hex(value: &str) -> Result<usize, String> {
    let value = value.trim();
    let digits = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X"))
        .ok_or_else(|| "Use a hexadecimal address prefixed with 0x".to_owned())?;
    if digits.is_empty() || digits.len() > 16 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid hexadecimal address".into());
    }
    usize::from_str_radix(digits, 16).map_err(|_| "Address does not fit this platform".into())
}

fn parse_offsets(text: &str) -> Result<Vec<usize>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parts: Vec<_> = text.split(',').collect();
    if parts.len() > 8 {
        return Err("Maximum 8 pointer offsets".into());
    }
    parts.into_iter().map(|s| {
        let n = parse_hex(s)?;
        if n > 0x1_0000 {
            return Err("Pointer offset is too large (maximum 0x10000)".into());
        }
        Ok(n)
    }).collect()
}

/// Each step dereferences the current 64-bit pointer and adds an offset.
/// The final result is read as bytes, not silently interpreted as a new pointer.
fn resolve_chain<F>(base: usize, offsets: &[usize], mut pointer_at: F) -> Result<usize, String>
where F: FnMut(usize) -> Result<u64, String> {
    let mut address = base;
    for &offset in offsets {
        let pointer = pointer_at(address)?;
        let pointer: usize = pointer.try_into().map_err(|_| "Pointer exceeds address width".to_owned())?;
        if pointer < 0x1_0000 {
            return Err("Null or implausible pointer in chain".into());
        }
        address = pointer.checked_add(offset).ok_or_else(|| "Pointer offset overflow".to_owned())?;
    }
    Ok(address)
}

#[cfg(windows)]
pub(super) fn read(snapshot: &Snapshot, game_root: &str, address: &str, offsets: &str) -> Result<String, String> {
    let pid = snapshot.pid.ok_or_else(|| "Detect one Evolve process first".to_owned())?;
    // A PID may be reused or Evolve may have restarted since the last scan.
    let current = inspect(game_root);
    if current.pid != Some(pid) {
        return Err("Evolve process changed; detect again before reading".into());
    }
    let base = parse_hex(address)?;
    if base < 0x1_0000 {
        return Err("Address is below the allowed user-mode range".into());
    }
    let offsets = parse_offsets(offsets)?;
    let handle: read_process_memory::ProcessHandle = pid.try_into()
        .map_err(|error| format!("Cannot open Evolve for read-only memory access: {error}"))?;
    let target = resolve_chain(base, &offsets, |at| {
        let bytes = read_process_memory::copy_address(at, 8, &handle)
            .map_err(|error| format!("Pointer read failed: {error}"))?;
        let raw: [u8; 8] = bytes.try_into().map_err(|_| "Incomplete pointer read".to_owned())?;
        Ok(u64::from_le_bytes(raw))
    })?;
    let bytes = read_process_memory::copy_address(target, 32, &handle)
        .map_err(|error| format!("Memory read failed: {error}"))?;
    // Recheck for a match transition/restart; do not cache resolved pointers.
    if inspect(game_root).pid != Some(pid) {
        return Err("Evolve process changed during inspection; result discarded".into());
    }
    let dump = bytes.chunks(16)
        .map(|line| line.iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>().join("\n");
    Ok(format!("PID {pid}  |  0x{target:016X}  |  {} bytes (read only)\n{dump}", bytes.len()))
}

#[cfg(not(windows))]
pub(super) fn read(_snapshot: &Snapshot, _game_root: &str, _address: &str, _offsets: &str) -> Result<String, String> {
    Err("External memory inspection requires Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasklist_parses_quoted_pids_and_rejects_other_processes() {
        let output = "\"Evolve.exe\",\"5432\",\"Console\",\"1\",\"42,000 K\"\n\"other.exe\",\"100\",\"Console\",\"1\",\"3 K\"";
        assert_eq!(parse_tasklist(output), vec![5432]);
        assert!(parse_tasklist("INFO: No tasks are running").is_empty());
    }

    #[test]
    fn pointer_inputs_are_bounded() {
        assert_eq!(parse_hex("0x1234").unwrap(), 0x1234);
        assert!(parse_hex("1234").is_err());
        assert!(parse_hex("0xZZ").is_err());
        assert_eq!(parse_offsets("0x10,0x20").unwrap(), vec![16, 32]);
        assert!(parse_offsets("0x10,").is_err());
        assert!(parse_offsets(&vec!["0x10"; 9].join(",")).is_err());
        assert!(parse_offsets("0x10001").is_err());
    }

    #[test]
    fn pointer_chain_validates_nulls_and_overflow() {
        assert_eq!(resolve_chain(0x10000, &[0x20, 0x30], |at| {
            Ok(if at == 0x10000 { 0x20000 } else { 0x30000 })
        }).unwrap(), 0x30030);
        assert!(resolve_chain(0x10000, &[0], |_| Ok(0)).is_err());
        assert!(resolve_chain(0x10000, &[0x100], |_| Ok(u64::MAX)).is_err());
    }
}
