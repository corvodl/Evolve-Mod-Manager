//! Read-only decoding of Stage 2 monster energy from validated local actors.
//! See docs/MONSTER_EVOLUTION_ENERGY_HANDOFF.md and
//! docs/MONSTER_TRAVERSAL_ENERGY_HANDOFF.md.
//! Offsets/rotations are build-specific. Never persist an address or key.
//! There are intentionally no write, freeze, or recharge setter functions.
const EVOLUTION_CURRENT_OFFSET: usize = 0x16FC;
const EVOLUTION_MAX_OFFSET: usize = 0x1700;
const TRAVERSAL_CURRENT_OFFSET: usize = 0x1754;
const TRAVERSAL_MAX_OFFSET: usize = 0x1758;
pub(super) const KEY_RVA: usize = 0x28D5AE0;
pub(super) const ACTOR_VTABLE_RVA: usize = 0x24B3790;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Energy {
    pub current: f32,
    pub maximum: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Reading {
    pub evolution: Option<Energy>,
    pub traversal: Option<Energy>,
}

pub(super) fn stage(bits: u32) -> Option<u32> {
    match bits & 0xffff {
        0x0101 => Some(1),
        0x0202 => Some(2),
        0x0303 => Some(3),
        _ => None,
    }
}

/// The original Evolve actor identifiers are not the user-facing names.
/// Reject hunter actors: random floats in their unused monster fields must
/// never be displayed as a validated energy pool.
pub(super) fn known_monster(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["goliath", "teleporter", "kraken", "grounder", "rocky",
     "rockybobsicle", "lurker", "wraith", "gorgon", "behemoth"]
        .iter()
        .any(|prefix| n.starts_with(prefix))
}

fn plausible(current: f32, maximum: f32) -> bool {
    current.is_finite()
        && maximum.is_finite()
        && (0.0..=100_000.0).contains(&maximum)
        && (0.0..=maximum).contains(&current)
}

/// Matches the two native getter wrappers. A low-16-bit stage is necessary:
/// upper stage bits vary between valid monster actors/processes.
pub(super) fn evolution(current: u32, max: u32, key: u32, stage_word: u32) -> Option<Energy> {
    let stage = stage(stage_word)?;
    let current = f32::from_bits((current ^ key).rotate_right(9));
    let maximum = f32::from_bits((max ^ key).rotate_left(16));
    let expected = match stage {
        1 => 24.0,
        2 => 28.0,
        3 => 0.0,
        _ => return None,
    };
    // These exact expected maxima are for the build documented in PR #144.
    // A changed ruleset must be revalidated rather than silently trusted.
    if !plausible(current, maximum) || maximum != expected {
        return None;
    }
    Some(Energy { current, maximum })
}

pub(super) fn traversal(current: u32, max: u32, key: u32) -> Option<Energy> {
    let current = f32::from_bits(current ^ key ^ 0x0093_F6E4);
    let maximum = f32::from_bits((max ^ key).rotate_left(7));
    // 100 is an observed maximum for the inspected monster build. A
    // different maximum may mean a different build or a non-monster actor.
    (plausible(current, maximum) && maximum == 100.0)
        .then_some(Energy { current, maximum })
}

#[cfg(windows)]
pub(super) fn read_local(
    handle: &read_process_memory::ProcessHandle,
    base: usize,
    actor: usize,
    actor_name: &str,
    stage_word: u32,
) -> Reading {
    use read_process_memory::copy_address;
    let mut result = Reading::default();
    if !known_monster(actor_name) || stage(stage_word).is_none() {
        return result;
    }
    let raw = |at: usize| -> Option<u32> {
        let bytes = copy_address(at, 4, handle).ok()?;
        Some(u32::from_le_bytes(bytes.as_slice().try_into().ok()?))
    };
    let pointer = |at: usize| -> Option<usize> {
        let bytes = copy_address(at, 8, handle).ok()?;
        usize::try_from(u64::from_le_bytes(bytes.as_slice().try_into().ok()?)).ok()
    };
    // Do not decode offsets from a different actor class or game build.
    let Some(expected_vtable) = base.checked_add(ACTOR_VTABLE_RVA) else {
        return result;
    };
    if pointer(actor) != Some(expected_vtable) {
        return result;
    }
    let Some(key) = base.checked_add(KEY_RVA).and_then(raw) else {
        return result;
    };
    let field = |offset: usize| actor.checked_add(offset).and_then(raw);
    if let (Some(a), Some(b)) = (
        field(EVOLUTION_CURRENT_OFFSET),
        field(EVOLUTION_MAX_OFFSET),
    ) {
        result.evolution = evolution(a, b, key, stage_word);
    }
    if let (Some(a), Some(b)) = (
        field(TRAVERSAL_CURRENT_OFFSET),
        field(TRAVERSAL_MAX_OFFSET),
    ) {
        result.traversal = traversal(a, b, key);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc_evolution(current: f32, maximum: f32, key: u32) -> (u32, u32) {
        (
            current.to_bits().rotate_left(9) ^ key,
            maximum.to_bits().rotate_right(16) ^ key,
        )
    }
    fn enc_traversal(current: f32, maximum: f32, key: u32) -> (u32, u32) {
        (
            current.to_bits() ^ key ^ 0x0093_F6E4,
            maximum.to_bits().rotate_right(7) ^ key,
        )
    }

    #[test]
    fn stage_accepts_verified_low_bytes_even_with_upper_bits() {
        assert_eq!(stage(0xFA36_0101), Some(1));
        assert_eq!(stage(0x55FA_0202), Some(2));
        assert_eq!(stage(0x55FA_0303), Some(3));
        assert_eq!(stage(0xFA36_0404), None);
    }

    #[test]
    fn evolution_roundtrips_two_process_keys_and_final_stage() {
        for key in [0xB805_B8B7, 0xC0B7_3BB2] {
            for (stage_word, current, maximum) in [
                (0x0101, 0.0, 24.0),
                (0xFA36_0101, 14.0, 24.0),
                (0x0101, 24.0, 24.0),
                (0xFA36_0202, 0.0, 28.0),
                (0x0202, 28.0, 28.0),
                (0x55FA_0303, 0.0, 0.0),
            ] {
                let (raw, raw_max) = enc_evolution(current, maximum, key);
                assert_eq!(
                    evolution(raw, raw_max, key, stage_word),
                    Some(Energy { current, maximum })
                );
            }
        }
    }

    #[test]
    fn rejects_impossible_or_nonfinite_energy_and_other_rules() {
        let key = 0xB805_B8B7;
        for (value, max) in [(25.0, 24.0), (f32::NAN, 24.0),
                              (f32::INFINITY, 24.0), (-2.0, 24.0),
                              (10.0, 100.0)] {
            let (raw, raw_max) = enc_evolution(value, max, key);
            assert_eq!(evolution(raw, raw_max, key, 0x0101), None);
        }
    }

    #[test]
    fn traversal_uses_its_distinct_encoding_and_rejects_other_maxima() {
        for key in [0xB805_B8B7, 0xC0B7_3BB2] {
            for current in [0.0, 30.0, 70.0, 100.0] {
                let (raw, max) = enc_traversal(current, 100.0, key);
                assert_eq!(
                    traversal(raw, max, key),
                    Some(Energy { current, maximum: 100.0 })
                );
            }
            let (raw, max) = enc_traversal(25.0, 24.0, key);
            assert!(traversal(raw, max, key).is_none());
        }
    }

    #[test]
    fn monster_allowlist_does_not_accept_hunters() {
        assert!(known_monster("TeleporterABot"));
        assert!(known_monster("KrakenElderBot"));
        assert!(known_monster("RockyABot"));
        assert!(!known_monster("AssaultEBot"));
        assert!(!known_monster("TrackingCBot"));
        assert!(!known_monster("Actor entity 0x14 (name unresolved)"));
    }
}
