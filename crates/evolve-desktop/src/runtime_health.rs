//! Read-only health *candidate* discovery, not Evolve actor recognition.
//!
//! Until validated per-build actor/class layouts are available, scanning one
//! caller-specified 4 KiB page and filtering its four-byte numeric values is
//! all that can be claimed. No memory writes, DLL loading or RMI calls.
use crate::runtime_memory::{self, Snapshot};

const PAGE_BYTES: usize = 4096;
const MAX_CANDIDATES: usize = PAGE_BYTES / 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Float,
    Integer,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Float => "float32",
            Self::Integer => "unsigned 4-byte integer",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Filter {
    Decreased,
    Increased,
    Unchanged,
    Changed,
}

#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub address: usize,
    raw: u32,
}

#[derive(Clone, Debug)]
pub(super) struct Scan {
    pub pid: u32,
    pub page_base: usize,
    kind: Kind,
    candidates: Vec<Candidate>,
}

fn numeric(kind: Kind, raw: u32) -> Option<f64> {
    let result = match kind {
        Kind::Float => f64::from(f32::from_bits(raw)),
        Kind::Integer => f64::from(raw),
    };
    if result.is_finite() && (0.001..=100_000.0).contains(&result) {
        Some(result)
    } else {
        None
    }
}

fn parsed_expected(value: &str, kind: Kind) -> Result<Option<f64>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let number = match kind {
        Kind::Float => f64::from(
            value
                .parse::<f32>()
                .map_err(|_| "Expected health must be a number")?,
        ),
        Kind::Integer => f64::from(
            value
                .parse::<u32>()
                .map_err(|_| "Expected health must be a whole number for 4 Bytes")?,
        ),
    };
    if !number.is_finite() || !(0.001..=100_000.0).contains(&number) {
        return Err("Expected health must be between 0.001 and 100000".into());
    }
    Ok(Some(number))
}

/// Matches *candidates* within one known readable page. A coincidental match
/// has no actor identity, even when it decreases at the same time as damage.
fn initial_candidates(
    bytes: &[u8],
    base: usize,
    kind: Kind,
    expected: Option<f64>,
) -> Vec<Candidate> {
    let mut found = Vec::new();
    for (index, chunk) in bytes.chunks_exact(4).enumerate().take(MAX_CANDIDATES) {
        let raw = u32::from_le_bytes(chunk.try_into().unwrap());
        let Some(value) = numeric(kind, raw) else {
            continue;
        };
        if expected.is_some_and(|wanted| {
            // Float HUD rounding is not necessarily equal to stored health.
            // Avoid broad percentage-matching against large numbers.
            let tolerance = if kind == Kind::Float { 0.01 } else { 0.0 };
            (value - wanted).abs() > tolerance
        }) {
            continue;
        }
        let Some(address) = base.checked_add(index * 4) else {
            break;
        };
        found.push(Candidate { address, raw });
    }
    found
}

fn filter_candidates(previous: &Scan, bytes: &[u8], filter: Filter) -> Scan {
    let mut candidates = Vec::new();
    for candidate in &previous.candidates {
        let Some(offset) = candidate.address.checked_sub(previous.page_base) else {
            continue;
        };
        let Some(slice) = bytes.get(offset..offset.saturating_add(4)) else {
            continue;
        };
        let raw = u32::from_le_bytes(slice.try_into().unwrap());
        let (Some(old), Some(new)) = (
            numeric(previous.kind, candidate.raw),
            numeric(previous.kind, raw),
        ) else {
            continue;
        };
        let keep = match filter {
            Filter::Decreased => new < old,
            Filter::Increased => new > old,
            Filter::Unchanged => raw == candidate.raw,
            Filter::Changed => raw != candidate.raw,
        };
        if keep {
            candidates.push(Candidate {
                address: candidate.address,
                raw,
            });
        }
    }
    Scan {
        pid: previous.pid,
        page_base: previous.page_base,
        kind: previous.kind,
        candidates,
    }
}

impl Scan {
    pub(super) fn describe(&self) -> String {
        let mut lines = vec![format!(
            "{} candidate(s), {} in 4 KiB at 0x{:X}. Read-only; actor identity UNVERIFIED.",
            self.candidates.len(),
            self.kind.name(),
            self.page_base
        )];
        let mut sorted = self.candidates.clone();
        sorted.sort_by_key(|candidate| candidate.address);
        for c in sorted.iter().take(20) {
            if let Some(value) = numeric(self.kind, c.raw) {
                lines.push(format!("0x{:X}: {value:.3}", c.address));
            }
        }
        if sorted.len() > 20 {
            lines.push(
                "Only the first 20 are shown. Re-scan after taking damage to narrow results."
                    .into(),
            );
        }
        if sorted.is_empty() {
            lines.push(
                "No matches remain. Start a fresh scan with another page or value type.".into(),
            );
        }
        lines.join("\n")
    }
}

#[cfg(windows)]
fn read_page(snapshot: &Snapshot, game_root: &str, page_base: usize) -> Result<Vec<u8>, String> {
    if runtime_memory::inspect(game_root).pid != snapshot.pid {
        return Err("Process changed; detect Evolve again".into());
    }
    let pid = snapshot.pid.ok_or("No unique Evolve process")?;
    let handle: read_process_memory::ProcessHandle = pid
        .try_into()
        .map_err(|e| format!("Cannot open Evolve read-only: {e}"))?;
    let bytes = read_process_memory::copy_address(page_base, PAGE_BYTES, &handle)
        .map_err(|e| format!("Cannot read the selected 4 KiB memory page: {e}"))?;
    if runtime_memory::inspect(game_root).pid != Some(pid) {
        return Err("Evolve process changed during scan; candidates discarded".into());
    }
    Ok(bytes)
}

#[cfg(windows)]
pub(super) fn start(
    snapshot: &Snapshot,
    game_root: &str,
    address: &str,
    expected: &str,
    kind: Kind,
) -> Result<Scan, String> {
    let pid = snapshot.pid.ok_or("Detect Evolve first")?;
    let address = runtime_memory::resolve_address(snapshot, address)?;
    if address < 0x10000 {
        return Err("Start with a readable 0x-prefixed in-game address".into());
    }
    let page_base = address & !(PAGE_BYTES - 1);
    let expected = parsed_expected(expected, kind)?;
    let bytes = read_page(snapshot, game_root, page_base)?;
    Ok(Scan {
        pid,
        page_base,
        kind,
        candidates: initial_candidates(&bytes, page_base, kind, expected),
    })
}

#[cfg(not(windows))]
pub(super) fn start(
    _snapshot: &Snapshot,
    _game_root: &str,
    _address: &str,
    _expected: &str,
    _kind: Kind,
) -> Result<Scan, String> {
    Err("Windows process memory is required".into())
}

#[cfg(windows)]
pub(super) fn refine(
    snapshot: &Snapshot,
    game_root: &str,
    previous: Scan,
    filter: Filter,
) -> Result<Scan, String> {
    if snapshot.pid != Some(previous.pid) {
        return Err("Process changed; start a new scan".into());
    }
    let bytes = read_page(snapshot, game_root, previous.page_base)?;
    Ok(filter_candidates(&previous, &bytes, filter))
}

#[cfg(not(windows))]
pub(super) fn refine(
    _snapshot: &Snapshot,
    _game_root: &str,
    _previous: Scan,
    _filter: Filter,
) -> Result<Scan, String> {
    Err("Windows process memory is required".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_floats(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn finds_expected_float_without_inventing_an_actor_class() {
        let raw = bytes_floats(&[20.0, 1500.0, 0.0, f32::NAN, 4000.0]);
        let found = initial_candidates(&raw, 0x2000, Kind::Float, Some(1500.0));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].address, 0x2004);
    }

    #[test]
    fn damage_refinement_retains_only_decreased_values() {
        let first = bytes_floats(&[100.0, 500.0, 1000.0, 0.0]);
        let second = bytes_floats(&[100.0, 400.0, 1200.0, 0.0]);
        let original = Scan {
            pid: 123,
            page_base: 0x10000,
            kind: Kind::Float,
            candidates: initial_candidates(&first, 0x10000, Kind::Float, None),
        };
        let filtered = filter_candidates(&original, &second, Filter::Decreased);
        assert_eq!(filtered.candidates.len(), 1);
        assert_eq!(filtered.candidates[0].address, 0x10004);
        let stable = filter_candidates(&original, &second, Filter::Unchanged);
        assert_eq!(stable.candidates.len(), 1);
        assert_eq!(stable.candidates[0].address, 0x10000);
    }

    #[test]
    fn numeric_guards_and_integer_mode() {
        assert!(parsed_expected("NaN", Kind::Float).is_err());
        assert!(parsed_expected("-1", Kind::Float).is_err());
        assert!(parsed_expected("1.5", Kind::Integer).is_err());
        let raw = [1000_u32.to_le_bytes(), 600_u32.to_le_bytes()].concat();
        assert_eq!(
            initial_candidates(&raw, 0x10000, Kind::Integer, Some(600.0)).len(),
            1
        );
        assert!(initial_candidates(&[0, 1], 0x10000, Kind::Integer, None).is_empty());
    }

    #[test]
    fn byte_filter_does_not_cross_page() {
        let scan = Scan {
            pid: 1,
            page_base: 0x10000,
            kind: Kind::Float,
            candidates: vec![Candidate {
                address: 0x10FFC,
                raw: 100.0_f32.to_bits(),
            }],
        };
        let short = vec![0; 4];
        assert!(
            filter_candidates(&scan, &short, Filter::Decreased)
                .candidates
                .is_empty()
        );
    }
}
