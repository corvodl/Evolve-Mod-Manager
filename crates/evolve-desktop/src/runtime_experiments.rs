//! Developer-only offline runtime operation planner.
//!
//! Game-specific actor layouts, authenticated offline session ownership and
//! engine function signatures are NOT available yet. Accordingly this module
//! validates proposed changes and provides reproducible previews, but cannot
//! write memory, invoke RMI, inject a DLL or spawn an entity. The plan is
//! deliberately not an executable command or a serialization format.
use std::fmt::Write;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Phase2Action {
    Health,
    Cooldown,
    Teleport,
    Equip,
}

impl Phase2Action {
    fn label(self) -> &'static str {
        match self {
            Self::Health => "Health",
            Self::Cooldown => "Cooldown",
            Self::Teleport => "Teleport",
            Self::Equip => "Weapon reference",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Phase3Action {
    Spawn,
    SwapCharacter,
    Ability,
}

impl Phase3Action {
    fn label(self) -> &'static str {
        match self {
            Self::Spawn => "Spawn entity",
            Self::SwapCharacter => "Change character",
            Self::Ability => "Assign custom ability",
        }
    }
}

fn hex_address(input: &str) -> Result<u64, String> {
    let input = input.trim();
    let value = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
        .ok_or("Use a 0x-prefixed hexadecimal address")?;
    if value.is_empty() || value.len() > 16 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid hexadecimal address".into());
    }
    let address = u64::from_str_radix(value, 16).map_err(|_| "Address exceeds u64")?;
    if address < 0x1_0000 || address > 0x0000_7FFF_FFFF_FFFF {
        return Err("Address is outside the supported 64-bit user address range".into());
    }
    Ok(address)
}

fn bounded_float(input: &str, label: &str, min: f32, max: f32) -> Result<f32, String> {
    let value: f32 = input
        .trim()
        .parse()
        .map_err(|_| format!("Invalid {label} number"))?;
    if !value.is_finite() || value < min || value > max {
        return Err(format!(
            "{label} must be finite and between {min} and {max}"
        ));
    }
    Ok(value)
}

fn serialized_value(kind: Phase2Action, input: &str) -> Result<Vec<u8>, String> {
    match kind {
        Phase2Action::Health => Ok(bounded_float(input, "Health", 0.0, 100_000.0)?
            .to_le_bytes()
            .to_vec()),
        Phase2Action::Cooldown => Ok(bounded_float(input, "Cooldown", 0.0, 3_600.0)?
            .to_le_bytes()
            .to_vec()),
        Phase2Action::Teleport => {
            let parts = input.split(',').map(str::trim).collect::<Vec<_>>();
            if parts.len() != 3 {
                return Err("Position requires three comma-separated X, Y, Z floats".into());
            }
            let mut bytes = Vec::with_capacity(12);
            for part in parts {
                bytes.extend_from_slice(
                    &bounded_float(part, "Position component", -100_000.0, 100_000.0)?
                        .to_le_bytes(),
                );
            }
            Ok(bytes)
        }
        Phase2Action::Equip => {
            let address = hex_address(input)?;
            if address % 8 != 0 {
                return Err("Weapon reference must be 8-byte aligned".into());
            }
            Ok(address.to_le_bytes().to_vec())
        }
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut output = String::new();
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            output.push(' ');
        }
        let _ = write!(&mut output, "{byte:02X}");
    }
    output
}

/// A typed, bounded description of intended bytes. This intentionally DOES
/// NOT know whether the address is a health field or a pointer in this build.
pub(super) fn preview_phase2(
    kind: Phase2Action,
    address: &str,
    value: &str,
    process_detected: bool,
) -> Result<String, String> {
    let at = hex_address(address)?;
    if at % 4 != 0 {
        return Err("Target address must be 4-byte aligned".into());
    }
    let bytes = serialized_value(kind, value)?;
    Ok(format!(
        "{} preview (NOT APPLIED)\nTarget: 0x{at:016X}\nSuggested value bytes (little-endian): {}\nProcess detected: {}\nBlocked: live actor identity and field offset for this exact build are unknown; offline-only session is unverified; no baseline/restore journal exists. This preview does not write memory.",
        kind.label(),
        hex_bytes(&bytes),
        if process_detected {
            "yes (unverified)"
        } else {
            "no"
        }
    ))
}

fn identifier(input: &str) -> Result<&str, String> {
    let name = input.trim();
    if name.is_empty()
        || name.len() > 64
        || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(
            "Provide an engine class identifier (1-64 ASCII letters, digits or underscores)".into(),
        );
    }
    Ok(name)
}

pub(super) fn preview_phase3(kind: Phase3Action, input: &str) -> Result<String, String> {
    let (name, count) = match kind {
        Phase3Action::Spawn => {
            let mut fields = input.split(',');
            let name = identifier(fields.next().unwrap_or_default())?;
            let count = fields
                .next()
                .map(|s| s.trim().parse::<u8>())
                .transpose()
                .map_err(|_| "Spawn count must be an integer from 1 to 32")?
                .unwrap_or(1);
            if fields.next().is_some() || !(1..=32).contains(&count) {
                return Err("Spawn count must be an integer from 1 to 32".into());
            }
            (name, count)
        }
        _ => (identifier(input)?, 1),
    };
    let detail = match kind {
        Phase3Action::Spawn => format!("Candidate class: {name}; count: {count}"),
        Phase3Action::SwapCharacter => format!("Candidate actor class: {name}"),
        Phase3Action::Ability => format!("Candidate ability identifier: {name}"),
    };
    Ok(format!(
        "{} preview (NOT EXECUTED)\n{detail}\nBlocked: Evolve's version-specific class registry, entity initialization, ownership and offline authority are not verified. No engine function, RMI, or allocation is invoked.",
        kind.label()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_and_cooldown_are_bounded_and_little_endian() {
        assert_eq!(
            serialized_value(Phase2Action::Health, "100000").unwrap(),
            100_000_f32.to_le_bytes()
        );
        assert!(serialized_value(Phase2Action::Health, "NaN").is_err());
        assert!(serialized_value(Phase2Action::Health, "-5").is_err());
        assert!(serialized_value(Phase2Action::Cooldown, "3601").is_err());
        assert_eq!(
            serialized_value(Phase2Action::Cooldown, "1.5").unwrap(),
            1.5_f32.to_le_bytes()
        );
    }

    #[test]
    fn teleport_and_equipment_require_bounded_inputs() {
        assert_eq!(
            serialized_value(Phase2Action::Teleport, "1,2,3")
                .unwrap()
                .len(),
            12
        );
        assert!(serialized_value(Phase2Action::Teleport, "1,2").is_err());
        assert!(serialized_value(Phase2Action::Teleport, "0,inf,0").is_err());
        assert!(serialized_value(Phase2Action::Equip, "0x10003").is_err());
        assert!(serialized_value(Phase2Action::Equip, "0x10008").is_ok());
        assert!(preview_phase2(Phase2Action::Health, "0x10003", "1", true).is_err());
    }

    #[test]
    fn unverified_process_does_not_become_an_executable_plan() {
        let text = preview_phase2(Phase2Action::Health, "0x10000", "999", true).unwrap();
        assert!(text.contains("NOT APPLIED"));
        assert!(text.contains("Blocked"));
        assert!(text.contains("offline-only session is unverified"));
    }

    #[test]
    fn spawning_bounds_and_class_names_are_validated() {
        assert!(preview_phase3(Phase3Action::Spawn, "Goliath,2").is_ok());
        assert!(preview_phase3(Phase3Action::Spawn, "Tyrant,200").is_err());
        assert!(preview_phase3(Phase3Action::Spawn, "../Goliath").is_err());
        assert!(
            preview_phase3(Phase3Action::SwapCharacter, "Markov")
                .unwrap()
                .contains("NOT EXECUTED")
        );
        assert!(
            preview_phase3(Phase3Action::Ability, "LightningGun")
                .unwrap()
                .contains("No engine function")
        );
    }
}
