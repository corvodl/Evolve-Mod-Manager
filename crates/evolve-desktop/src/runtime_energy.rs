//! Pure, read-only decoding of Stage 2 monster resources.
//! The offsets and transforms come from the read-only evidence in
//! docs/MONSTER_EVOLUTION_ENERGY_HANDOFF.md and
//! docs/MONSTER_TRAVERSAL_ENERGY_HANDOFF.md.
//!
//! No process access, writes, freezes, or cached process-specific keys here.
//! Live reads and actor identity validation stay in runtime_research.rs.
pub(super) const KEY_RVA: usize = 0x28D5AE0;
pub(super) const ACTOR_VTABLE_RVA: usize = 0x24B3790;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Energy {
    pub current: f32,
    pub maximum: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct EvolutionEnergy {
    pub stage: u8,
    pub current: f32,
    pub maximum: f32,
}

fn plausible(current: f32, maximum: f32) -> bool {
    current.is_finite()
        && maximum.is_finite()
        && (0.0..=maximum).contains(&current)
        && (0.0..=1000.0).contains(&maximum)
}

impl EvolutionEnergy {
    /// Rounded percentage of this stage's bar; Stage 3 has no bar.
    pub fn percentage(self) -> Option<u8> {
        if self.stage == 3 || !plausible(self.current, self.maximum) || self.maximum <= 0.0 {
            return None;
        }
        Some(
            (self.current / self.maximum * 100.0)
                .round()
                .clamp(0.0, 100.0) as u8,
        )
    }
}

/// Stage is duplicated in the low two bytes; valid captured actors have
/// process-specific data in the high two bytes.
pub(super) fn stage(word: u32) -> Option<u8> {
    match word & 0xFFFF {
        0x0101 => Some(1),
        0x0202 => Some(2),
        0x0303 => Some(3),
        _ => None,
    }
}

/// Internal names resolved from the local IEntity; never accept a hunter or
/// the fallback name used when the name resolver fails. Read-side validation
/// additionally requires the local actor and build-specific vtable/backlinks.
pub(super) fn known_monster(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "goliath",
        "teleporter",
        "kraken",
        "grounder",
        "rocky",
        "lurker",
        "scorpid",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

/// The two evolution fields have *different* rotations; neither is a plain
/// Float32 in memory. Expected maxima are specific to the documented rules.
pub(super) fn evolution(
    stage_word: u32,
    current_raw: u32,
    max_raw: u32,
    key: u32,
) -> Option<EvolutionEnergy> {
    let stage = stage(stage_word)?;
    let current = f32::from_bits((current_raw ^ key).rotate_right(9));
    let maximum = f32::from_bits((max_raw ^ key).rotate_left(16));
    let expected = match stage {
        1 => 24.0,
        2 => 28.0,
        3 => 0.0,
        _ => return None,
    };
    if !plausible(current, maximum) || maximum != expected {
        return None;
    }
    Some(EvolutionEnergy {
        stage,
        current,
        maximum,
    })
}

/// Traversal energy is distinct from evolution energy. The sampled current
/// pool uses a XOR constant and the maximum uses a seven-bit rotation.
pub(super) fn traversal(current_raw: u32, max_raw: u32, key: u32) -> Option<Energy> {
    let current = f32::from_bits(current_raw ^ key ^ 0x0093_F6E4);
    let maximum = f32::from_bits((max_raw ^ key).rotate_left(7));
    // PR #145's strict check: 100 is the observed traversal maximum.
    // Different rules/builds must be revalidated, not silently accepted.
    (plausible(current, maximum) && maximum == 100.0).then_some(Energy { current, maximum })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded_evolution(current: f32, maximum: f32, key: u32) -> (u32, u32) {
        (
            current.to_bits().rotate_left(9) ^ key,
            maximum.to_bits().rotate_right(16) ^ key,
        )
    }

    fn encoded_traversal(current: f32, maximum: f32, key: u32) -> (u32, u32) {
        (
            current.to_bits() ^ key ^ 0x0093_F6E4,
            maximum.to_bits().rotate_right(7) ^ key,
        )
    }

    #[test]
    fn high_stage_bits_do_not_hide_monster_stage() {
        assert_eq!(stage(0xFA36_0101), Some(1));
        assert_eq!(stage(0x55FA_0202), Some(2));
        assert_eq!(stage(0x55FA_0303), Some(3));
        assert_eq!(stage(0xFA36_0404), None);
        assert_eq!(stage(0), None);
    }

    #[test]
    fn evolution_roundtrips_observed_stage_limits_and_two_process_keys() {
        for key in [0xB805_B8B7, 0xC0B7_3BB2] {
            for (word, current, maximum) in [
                (0xFA36_0101, 0.0f32, 24.0f32),
                (0x0101, 14.0, 24.0),
                (0x0101, 24.0, 24.0),
                (0xFA36_0202, 0.0, 28.0),
                (0x0202, 11.0, 28.0),
                (0x0202, 28.0, 28.0),
                (0x55FA_0303, 0.0, 0.0),
            ] {
                let (a, b) = encoded_evolution(current, maximum, key);
                assert_eq!(
                    evolution(word, a, b, key),
                    Some(EvolutionEnergy {
                        stage: stage(word).unwrap(),
                        current,
                        maximum
                    })
                );
            }
        }
    }

    #[test]
    fn reject_bad_evolution_values_and_stage_rules() {
        let key = 0xB805_B8B7;
        for (current, maximum) in [
            (25.0, 24.0),
            (-1.0, 24.0),
            (f32::NAN, 24.0),
            (f32::INFINITY, 24.0),
            (10.0, 100.0),
        ] {
            let (a, b) = encoded_evolution(current, maximum, key);
            assert!(evolution(0x0101, a, b, key).is_none());
        }
        let (a, b) = encoded_evolution(1.0, 0.0, key);
        assert!(evolution(0x0303, a, b, key).is_none());
    }

    #[test]
    fn traversal_has_separate_decode_and_requires_observed_maximum() {
        for key in [0xB805_B8B7, 0xC0B7_3BB2] {
            for current in [0.0, 30.0, 70.0, 100.0] {
                let (a, b) = encoded_traversal(current, 100.0, key);
                assert_eq!(
                    traversal(a, b, key),
                    Some(Energy {
                        current,
                        maximum: 100.0
                    })
                );
            }
            for max in [24.0, 99.0, 200.0] {
                let (a, b) = encoded_traversal(25.0, max, key);
                assert!(traversal(a, b, key).is_none());
            }
        }
    }

    #[test]
    fn percentage_rounding_does_not_show_stage_three() {
        for (stage, current, maximum, expected) in [
            (1, 0.0f32, 24.0f32, Some(0)),
            (1, 12.0, 24.0, Some(50)),
            (2, 11.0, 28.0, Some(39)),
            (2, 28.0, 28.0, Some(100)),
            (3, 0.0, 0.0, None),
            (1, f32::INFINITY, 24.0, None),
            (2, 29.0, 28.0, None),
        ] {
            assert_eq!(
                EvolutionEnergy {
                    stage,
                    current,
                    maximum
                }
                .percentage(),
                expected
            );
        }
    }

    #[test]
    fn hunter_fallback_and_unrecognized_names_cannot_enable_monster_offsets() {
        for name in [
            "GoliathABot",
            "GoliathMeteorBot",
            "TeleporterABot",
            "KrakenElderBot",
            "GrounderABot",
            "RockyABot",
            "RockyBobcicleBot",
            "RockybobsicleABot",
            "LurkerABot",
        ] {
            assert!(known_monster(name), "{name}");
        }
        for name in [
            "AssaultEBot",
            "TrackingCBot",
            "Actor entity 0x14 (name unresolved)",
            "",
        ] {
            assert!(!known_monster(name), "{name}");
        }
    }
}
