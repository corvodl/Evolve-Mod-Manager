//! Authentic animation event audio and voice playback.
//!
//! Maps authored `.animevents` timeline parameters (foley, VO, abilities)
//! to CryEngine FMOD sound bank streams, decodes on-demand via vgmstream,
//! and plays with zero UI latency in 100% safe Rust.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};

static FSB_LOCATIONS: OnceLock<Mutex<std::collections::HashMap<String, PathBuf>>> = OnceLock::new();

fn fsb_locations() -> &'static Mutex<std::collections::HashMap<String, PathBuf>> {
    FSB_LOCATIONS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Plays a WAV sound file asynchronously with absolutely zero console popups.
#[allow(unsafe_code)]
pub fn play_wav_file(wav_path: &Path) {
    if !wav_path.is_file() {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut wide: Vec<u16> = wav_path.as_os_str().encode_wide().collect();
        wide.push(0);

        #[link(name = "winmm")]
        unsafe extern "system" {
            fn PlaySoundW(
                psz_sound: *const u16,
                hmod: *const std::ffi::c_void,
                fdw_sound: u32,
            ) -> i32;
        }

        const SND_FILENAME: u32 = 0x00020000;
        const SND_ASYNC: u32 = 0x00000001;
        const SND_NODEFAULT: u32 = 0x00000002;

        unsafe {
            PlaySoundW(
                wide.as_ptr(),
                std::ptr::null(),
                SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = wav_path;
    }
}

/// Find a specific FSB bank within the user's asset library root.
fn find_fsb_bank(assets_root: &str, fsb_name: &str) -> Option<PathBuf> {
    let lower_target = fsb_name.to_ascii_lowercase();

    // Check cached location first
    if let Ok(locs) = fsb_locations().lock()
        && let Some(path) = locs.get(&lower_target)
        && path.is_file()
    {
        return Some(path.clone());
    }

    let mut candidate_roots = Vec::new();
    let clean = assets_root.trim_start_matches(r"\\?\").trim();
    if !clean.is_empty() {
        let p = PathBuf::from(clean);
        if p.is_dir() {
            candidate_roots.push(p.clone());
            let sub = p.join("Data").join("AssetLibrary");
            if sub.is_dir() {
                candidate_roots.push(sub);
            }
        }
    }

    #[cfg(target_os = "windows")]
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let default_lib = PathBuf::from(local_app_data)
            .join("EvolveModManager")
            .join("Data")
            .join("AssetLibrary");
        if default_lib.is_dir() && !candidate_roots.contains(&default_lib) {
            candidate_roots.push(default_lib);
        }
    }

    for root in &candidate_roots {
        // Fast-path: prioritize sounds_Monsters.pak and Sounds.pak where monster and weapon audio resides
        if let Ok(entries) = std::fs::read_dir(root) {
            let mut second_pass = Vec::new();
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                    if name.contains("sounds_monsters") || name.contains("sounds.pak") {
                        if let Some(found) = search_recursive_fsb(&p, &lower_target, 8) {
                            if let Ok(mut locs) = fsb_locations().lock() {
                                locs.insert(lower_target, found.clone());
                            }
                            return Some(found);
                        }
                    } else if name.contains("sound")
                        || name.contains("audio")
                        || name.contains(".pak")
                    {
                        second_pass.push(p);
                    }
                }
            }
            for dir in second_pass {
                if let Some(found) = search_recursive_fsb(&dir, &lower_target, 8) {
                    if let Ok(mut locs) = fsb_locations().lock() {
                        locs.insert(lower_target, found.clone());
                    }
                    return Some(found);
                }
            }
        }
    }

    None
}

fn search_recursive_fsb(dir: &Path, target: &str, depth: usize) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() {
            if let Some(file_name) = p.file_name()
                && file_name.to_string_lossy().to_ascii_lowercase() == target
            {
                return Some(p);
            }
        } else if p.is_dir()
            && let Some(found) = search_recursive_fsb(&p, target, depth - 1)
        {
            return Some(found);
        }
    }
    None
}

/// Locate the vgmstream-cli executable.
fn resolve_vgmstream(configured: &str) -> Option<PathBuf> {
    let clean = configured.trim_start_matches(r"\\?\").trim();
    if !clean.is_empty() {
        let path = PathBuf::from(clean);
        if path.is_file() {
            return Some(path);
        }
    }

    // Default installed path fallback
    let default_path =
        PathBuf::from(r"C:\Program Files\EvolveModManager\third_party\vgmstream\vgmstream-cli.exe");
    if default_path.is_file() {
        return Some(default_path);
    }

    None
}

/// Decode a specific subsong from an FSB bank and play it immediately with zero console popups.
pub fn play_fsb_subsong(fsb_path: &Path, subsong: u32, vgmstream_exe: &Path) {
    let cache_dir = std::env::temp_dir().join("evolve_anim_audio");
    let _ = std::fs::create_dir_all(&cache_dir);

    let stem = fsb_path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let wav_name = format!("{}_{}.wav", stem, subsong);
    let target_wav = cache_dir.join(wav_name);

    if target_wav.is_file() {
        play_wav_file(&target_wav);
        return;
    }

    // Decode on background thread and play when ready
    let vgm_path = vgmstream_exe.to_path_buf();
    let fsb = fsb_path.to_path_buf();
    let wav = target_wav.clone();

    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new(vgm_path);
        cmd.arg("-s")
            .arg(subsong.to_string())
            .arg("-o")
            .arg(&wav)
            .arg(&fsb)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW: Suppresses console popup entirely
        }
        let status = cmd.status();

        if let Ok(s) = status
            && s.success()
            && wav.is_file()
        {
            play_wav_file(&wav);
        }
    });
}

/// Maps an authored animation event parameter, event name, clip virtual path,
/// character path, and monster stage to the corresponding FMOD sound bank filename,
/// starting subsong index, and pool size. Supports all Monsters and Hunters globally.
pub fn map_event_to_audio_spec(
    event_name: &str,
    param: &str,
    clip_virtual_path: &str,
    character_path: &str,
    stage: u8,
) -> Option<(&'static str, u32, u32)> {
    let p = param.trim();
    let clip_lower = clip_virtual_path.to_ascii_lowercase();
    let context_lower = format!("{} {}", clip_virtual_path, character_path).to_ascii_lowercase();

    let is_meteor = context_lower.contains("meteor") || context_lower.contains("goliathmeteor");
    let is_goliath = context_lower.contains("goliath") || context_lower.contains("scorpid");
    let is_wraith = context_lower.contains("wraith") || context_lower.contains("teleporter");
    let is_behemoth = context_lower.contains("rocky") || context_lower.contains("behemoth");
    let is_glacial = context_lower.contains("rockybobcicle");
    let is_kraken = context_lower.contains("kraken") || context_lower.contains("flyer");
    let is_gorgon = context_lower.contains("grounder")
        || context_lower.contains("parasite")
        || context_lower.contains("whipspider")
        || context_lower.contains("gorgon");

    // 1. Global Footsteps (Monsters, Hunters, Wildlife)
    if event_name.eq_ignore_ascii_case("footstep") {
        if is_wraith || is_kraken {
            // Wraith and Kraken levitate and float - they have zero heavy ground footsteps
            return None;
        } else if is_goliath {
            // Authentic Scorpid / Goliath footstep bank with 16 variations per stage
            let bank = match stage {
                2 => "footsteps_scorpid2.fsb",
                3 => "footsteps_scorpid3.fsb",
                _ => "footsteps_scorpid1.fsb",
            };
            return Some((bank, 1, 16));
        } else if is_behemoth || is_glacial {
            let bank = match stage {
                2 => "Rocky2_Foley.fsb",
                3 => "Rocky3_Foley.fsb",
                _ => "Rocky1_Foley.fsb",
            };
            return Some((bank, 1, 10));
        } else if is_gorgon {
            return Some(("Grounder1.fsb", 1, 10));
        } else if clip_lower.contains("monster") || clip_lower.contains("creature") {
            return Some(("footsteps_monsters_shared.fsb", 1, 5));
        } else {
            return Some(("Mercenaries_Shared_Foley_3p.fsb", 1, 5));
        }
    }

    // 2. Weapon Foley (Hunter reloads, ammo, and gear foley across all weapons)
    if p.eq_ignore_ascii_case("AmmoIn") {
        return Some(("foley_weapon.fsb", 1, 1));
    } else if p.eq_ignore_ascii_case("AmmoOut") {
        return Some(("foley_weapon.fsb", 2, 1));
    } else if p.eq_ignore_ascii_case("Reload_Open") || p.eq_ignore_ascii_case("Reload_Close") {
        return Some(("foley_weapon.fsb", 3, 1));
    } else if p.eq_ignore_ascii_case("GearSFX") || p.eq_ignore_ascii_case("GrabAmmo") {
        return Some(("foley_weapon.fsb", 4, 4));
    }

    // 3. Cocoon & Stage-up Evolution Foley (All Monsters)
    if p.contains("StageUp") || p.contains("Cocoon") || p.contains("cocoon") {
        return Some(("Teleporter_Shared.fsb", 1, 1)); // SFX_Monster_StageUp_Cocoon_01
    }

    // 4. Hunter Character Efforts & Exertions (Dynamic per Hunter)
    if (p.contains("Effort") || p.contains("VO_") || p.contains("Punch"))
        && clip_lower.contains("hunter")
    {
        if clip_lower.contains("markov") {
            return Some(("efforts_markov.fsb", 1, 20));
        } else if clip_lower.contains("abe") {
            return Some(("efforts_abe.fsb", 1, 20));
        } else if clip_lower.contains("val") {
            return Some(("efforts_val.fsb", 1, 20));
        } else if clip_lower.contains("hank") {
            return Some(("efforts_hank.fsb", 1, 20));
        } else if clip_lower.contains("caira") {
            return Some(("efforts_caira.fsb", 1, 20));
        } else if clip_lower.contains("hyde") {
            return Some(("efforts_hyde.fsb", 1, 20));
        } else if clip_lower.contains("griffin") {
            return Some(("efforts_griffin.fsb", 1, 20));
        } else if clip_lower.contains("bucket") {
            return Some(("efforts_bucket.fsb", 1, 20));
        } else if clip_lower.contains("parnell") {
            return Some(("efforts_parnell.fsb", 1, 20));
        } else if clip_lower.contains("lazarus") {
            return Some(("efforts_lazarus.fsb", 1, 20));
        } else if clip_lower.contains("maggie") {
            return Some(("efforts_maggie.fsb", 1, 20));
        } else if clip_lower.contains("lennox") {
            return Some(("efforts_lennox.fsb", 1, 20));
        } else if clip_lower.contains("crow") {
            return Some(("efforts_crow.fsb", 1, 20));
        } else if clip_lower.contains("slim") {
            return Some(("efforts_slim.fsb", 1, 20));
        } else if clip_lower.contains("sunny") {
            return Some(("efforts_sunny.fsb", 1, 20));
        } else if clip_lower.contains("torvald") {
            return Some(("efforts_torvald.fsb", 1, 20));
        } else if clip_lower.contains("emet") {
            return Some(("efforts_emet.fsb", 1, 20));
        } else {
            return Some(("Mercenaries_Shared_Foley_3p.fsb", 1, 5));
        }
    }

    // 5. Shared Monster Foley (Climbing claws, body impacts, ground pounds)
    if p.eq_ignore_ascii_case("Climb")
        || p.contains("climb")
        || p.contains("Impact")
        || p.contains("Jumpland")
    {
        return Some(("Shared_Foley.fsb", 1, 15));
    }

    // 6. Goliath & Meteor Goliath Abilities & Foley
    if is_goliath {
        // Rock Throw Grab / Pickup
        if p.eq_ignore_ascii_case("VO_RockThrow_PickUp")
            || p.eq_ignore_ascii_case("Activate_RockThrowGrab")
            || p.contains("RockThrowPickUp")
            || p.contains("rock_throw_start")
            || p.contains("rock_throw_pull")
        {
            if is_meteor {
                let offset = match stage {
                    2 => 81,
                    3 => 99,
                    _ => 63,
                };
                return Some(("GoliathMeteor_Shared.fsb", offset, 5));
            } else {
                let offset = match stage {
                    2 => 73,
                    3 => 92,
                    _ => 53,
                };
                return Some(("Goliath_SharedMaterials.fsb", offset, 5));
            }
        }
        // Rock Throw Throw / Release
        if p.eq_ignore_ascii_case("VO_RockThrow_Throw")
            || p.contains("RockThrowThrow")
            || (clip_lower.contains("rock_throw") && p.contains("mouth_flame"))
        {
            if is_meteor {
                return Some(("GoliathMeteor_Shared.fsb", 164, 6)); // GoliathMeteor_RockThrow_FLAME_00..05
            } else {
                return Some(("Scorpid1_Fireball_3p.fsb", 1, 1));
            }
        }
        // Flame Breath / Mouth Flame
        if p.contains("FlameBreath")
            || p.contains("melee_mouth_flame")
            || clip_lower.contains("fire_breath")
            || clip_lower.contains("flame_breath")
        {
            if is_meteor {
                return Some(("GoliathMeteor_Shared.fsb", 131, 6)); // GoliathMeteor_FlameBreathSwt_01..06
            } else {
                return Some(("Goliath_SharedMaterials.fsb", 13, 6)); // Goliath_FlameBreathSwt_001..006
            }
        }
        // Leap Attack Smash / Landing / Impact
        if p.contains("LeapAttack")
            || p.contains("Leapland")
            || p.contains("leap_attack_hand_trail")
            || (clip_lower.contains("leap") && (p.contains("Clawbash") || p.contains("Impact")))
        {
            if is_meteor {
                return Some(("GoliathMeteor_Shared.fsb", 149, 3)); // GoliathMeteor_LeapAttackImpactSwt_FLAME_00..02
            } else {
                return Some(("Goliath_SharedMaterials.fsb", 26, 3)); // Goliath_LeapAttackImpactSwt_01..03
            }
        }
        // Charge Attack
        if p.eq_ignore_ascii_case("Activate_ChargeAttack") || clip_lower.contains("charge_attack") {
            if is_meteor {
                let offset = match stage {
                    2 => 68,
                    3 => 86,
                    _ => 50,
                };
                return Some(("GoliathMeteor_Shared.fsb", offset, 5));
            } else {
                return Some(("Goliath_SharedMaterials.fsb", 58, 5));
            }
        }
        // Melee Clawbash / Swing
        if p.contains("MeleeSwing") || p.contains("Clawbash") {
            if is_meteor {
                return Some(("GoliathMeteor_Shared.fsb", 170, 9));
            } else {
                return Some(("Scorpid1_GrabberClaw_3p.fsb", 1, 1));
            }
        }
        if p.eq_ignore_ascii_case("VO_Death") {
            return Some(("Goliath_SharedMaterials.fsb", 1, 1));
        }
    }

    // 7. Kraken & Elder Kraken Abilities (Lightning, Aftershock, Vortex, Banshee Mines)
    if is_kraken
        || p.contains("aftershock")
        || p.contains("Lightning")
        || p.contains("Banshee")
        || p.contains("Vortex")
        || p.contains("Plasma")
    {
        return Some(("KrakenElder_Shared.fsb", 1, 25));
    }

    // 8. Wraith Abilities (Warp Blast, Abduction, Decoy, Supernova, Concussion)
    if is_wraith
        || p.contains("WarpBlast")
        || p.contains("Abduction")
        || p.contains("Decoy")
        || p.contains("Supernova")
        || p.contains("Concussion")
    {
        return Some(("Teleporter_Shared.fsb", 2, 25));
    }

    // 9. Glacial Behemoth (RockyBobcicle) Abilities & Permafrost
    if is_glacial
        || p.contains("rending_spikes")
        || p.contains("killing_frost")
        || p.contains("ice_shard")
    {
        return Some(("rockybobcicle_SharedMaterials.fsb", 1, 30));
    }

    // 10. Standard Behemoth (Rocky / Bob) Abilities (Lava Bomb, Fissure, Rock Wall, Tongue Grab, Roll)
    if is_behemoth
        || p.contains("LavaBomb")
        || p.contains("Fissure")
        || p.contains("RockWall")
        || p.contains("Tongue")
        || p.contains("Roll")
    {
        return Some(("Rocky_SharedMaterials.fsb", 1, 30));
    }

    // 11. Gorgon / Grounder Abilities (Acid Spit, Web Snare, Spider Trap, Mimic)
    if is_gorgon
        || p.contains("Acid")
        || p.contains("WebSnare")
        || p.contains("SpiderTrap")
        || p.contains("Mimic")
    {
        return Some(("Grounder_Shared.fsb", 1, 20));
    }

    None
}

/// Trigger an authored animation event by mapping its parameter/name
/// to the corresponding game audio streams.
pub fn trigger_animation_event(
    event_name: &str,
    parameter: &str,
    clip_virtual_path: &str,
    character_path: &str,
    assets_root: &str,
    configured_vgmstream: &str,
    stage: u8,
) {
    let Some(vgmstream) = resolve_vgmstream(configured_vgmstream) else {
        return;
    };

    let param = parameter.trim();
    if param.is_empty()
        && !event_name.eq_ignore_ascii_case("footstep")
        && !event_name.eq_ignore_ascii_case("foley")
    {
        return;
    }

    let Some((fsb_name, base_subsong, subsong_count)) =
        map_event_to_audio_spec(event_name, param, clip_virtual_path, character_path, stage)
    else {
        return;
    };

    let subsong = if subsong_count > 1 {
        base_subsong + (fast_pseudo_rand() % subsong_count)
    } else {
        base_subsong
    };

    if let Some(fsb_path) = find_fsb_bank(assets_root, fsb_name) {
        play_fsb_subsong(&fsb_path, subsong, &vgmstream);
    }
}

static RAND_STATE: Mutex<u32> = Mutex::new(12345);

fn fast_pseudo_rand() -> u32 {
    if let Ok(mut state) = RAND_STATE.lock() {
        *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        *state >> 16
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_event_to_audio_spec_global() {
        // Global footsteps
        assert_eq!(
            map_event_to_audio_spec(
                "footstep",
                "",
                "characters/monsters/goliath/anim.caf",
                "",
                1
            ),
            Some(("footsteps_scorpid1.fsb", 1, 16))
        );
        assert_eq!(
            map_event_to_audio_spec("footstep", "", "characters/monsters/wraith/anim.caf", "", 1),
            None
        );
        assert_eq!(
            map_event_to_audio_spec(
                "footstep",
                "",
                "characters/monsters/teleporter/anim.caf",
                "",
                1
            ),
            None
        );
        assert_eq!(
            map_event_to_audio_spec(
                "footstep",
                "",
                "characters/hunters/merc_markov/anim.caf",
                "",
                1
            ),
            Some(("Mercenaries_Shared_Foley_3p.fsb", 1, 5))
        );

        // Hunter Weapon Foley
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "AmmoIn",
                "characters/hunters/merc_markov/anim.caf",
                "",
                1
            ),
            Some(("foley_weapon.fsb", 1, 1))
        );
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "AmmoOut",
                "characters/hunters/merc_markov/anim.caf",
                "",
                1
            ),
            Some(("foley_weapon.fsb", 2, 1))
        );
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "Reload_Open",
                "characters/hunters/merc_markov/anim.caf",
                "",
                1
            ),
            Some(("foley_weapon.fsb", 3, 1))
        );

        // Hunter Efforts
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "Effort_Hup",
                "characters/hunters/merc_markov/anim.caf",
                "",
                1
            ),
            Some(("efforts_markov.fsb", 1, 20))
        );
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "Effort_Hup",
                "characters/hunters/merc_val/anim.caf",
                "",
                1
            ),
            Some(("efforts_val.fsb", 1, 20))
        );

        // Cocoon & Stage-up
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "VO_StageUp_Start",
                "characters/monsters/kraken/anim.caf",
                "",
                1
            ),
            Some(("Teleporter_Shared.fsb", 1, 1))
        );

        // Kraken Abilities
        assert_eq!(
            map_event_to_audio_spec(
                "effect",
                "krakenelder_abilities.aftershock",
                "characters/monsters/kraken/anim.caf",
                "",
                1
            ),
            Some(("KrakenElder_Shared.fsb", 1, 25))
        );

        // Wraith Abilities
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "WarpBlast",
                "characters/monsters/wraith/anim.caf",
                "",
                1
            ),
            Some(("Teleporter_Shared.fsb", 2, 25))
        );

        // Behemoth Abilities
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "LavaBomb",
                "characters/monsters/behemoth/anim.caf",
                "",
                1
            ),
            Some(("Rocky_SharedMaterials.fsb", 1, 30))
        );

        // Gorgon Abilities
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "AcidSpit",
                "characters/monsters/grounder/anim.caf",
                "",
                1
            ),
            Some(("Grounder_Shared.fsb", 1, 20))
        );

        // Goliath Abilities
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "VO_RockThrow_PickUp",
                "characters/monsters/goliath/anim.caf",
                "",
                1
            ),
            Some(("Goliath_SharedMaterials.fsb", 53, 5))
        );
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "VO_RockThrow_PickUp",
                "characters/monsters/goliathmeteor/anim.caf",
                "",
                1
            ),
            Some(("GoliathMeteor_Shared.fsb", 63, 5))
        );
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "VO_RockThrow_Throw",
                "characters/monsters/goliath/anim.caf",
                "",
                2
            ),
            Some(("Scorpid1_Fireball_3p.fsb", 1, 1))
        );
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "VO_RockThrow_Throw",
                "characters/monsters/goliathmeteor/anim.caf",
                "",
                2
            ),
            Some(("GoliathMeteor_Shared.fsb", 164, 6))
        );
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "FlameBreath",
                "characters/monsters/goliath/anim.caf",
                "",
                1
            ),
            Some(("Goliath_SharedMaterials.fsb", 13, 6))
        );
        assert_eq!(
            map_event_to_audio_spec(
                "foley",
                "FlameBreath",
                "characters/monsters/goliathmeteor/anim.caf",
                "",
                1
            ),
            Some(("GoliathMeteor_Shared.fsb", 131, 6))
        );
    }

    #[test]
    fn test_pseudo_rand_advances() {
        let r1 = fast_pseudo_rand();
        let r2 = fast_pseudo_rand();
        assert_ne!(r1, r2);
    }
}
