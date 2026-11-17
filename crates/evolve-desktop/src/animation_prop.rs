//! Authored animation props and projectile physics simulation.
//!
//! Evaluates bone-attached props (e.g. Goliath rock pickup/hold) and
//! ballistic projectile trajectories (e.g. Goliath rock throw) in real time
//! across animation playback and timeline scrubbing.
use crate::textured::Surface;
use evolve_core::{RenderMesh, append_render_mesh, transform_render_mesh};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

type Mat4 = [[f32; 4]; 4];

/// Global cache for loaded prop geometry to prevent repeated file I/O during playback.
static PROP_CACHE: OnceLock<Mutex<std::collections::HashMap<String, Arc<RenderMesh>>>> =
    OnceLock::new();

fn prop_cache() -> &'static Mutex<std::collections::HashMap<String, Arc<RenderMesh>>> {
    PROP_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Locate any prop or effect CGF from the extracted asset library.
pub fn find_prop_cgf(assets_root: &str, relative_path: &str) -> Option<PathBuf> {
    let root = Path::new(assets_root);
    let rel_norm = relative_path.replace('\\', "/").to_ascii_lowercase();

    // Check direct relative path
    let direct = root.join(&rel_norm);
    if direct.is_file() {
        return Some(direct);
    }

    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let candidate1 = path.join(&rel_norm);
                if candidate1.is_file() {
                    return Some(candidate1);
                }
                let candidate2 = path.join(format!("files/{rel_norm}"));
                if candidate2.is_file() {
                    return Some(candidate2);
                }
            }
        }
    }
    None
}

/// Locate a rock throw projectile CGF from the extracted asset library.
#[allow(dead_code)]
pub fn find_rock_projectile_path(assets_root: &str, stage: u8) -> Option<PathBuf> {
    let stage_name = match stage {
        2 => "projectile_stage2.cgf",
        3 => "projectile_stage3.cgf",
        _ => "projectile_stage1.cgf",
    };
    find_prop_cgf(
        assets_root,
        &format!("objects/effects/abilities/goliath/rock_throw/{stage_name}"),
    )
}

/// Load and cache any centered render mesh.
pub fn get_or_load_prop_mesh(assets_root: &str, relative_path: &str) -> Option<Arc<RenderMesh>> {
    let path = find_prop_cgf(assets_root, relative_path)?;
    let key = path.to_string_lossy().into_owned();

    let mut cache = prop_cache().lock().ok()?;
    if let Some(mesh) = cache.get(&key) {
        return Some(mesh.clone());
    }

    let mut mesh = evolve_core::model::read_model(&path).ok()?;
    if mesh.positions.is_empty() {
        return None;
    }

    // Compute bounding center to ensure natural rotation and pivot point
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for pos in &mesh.positions {
        for i in 0..3 {
            min[i] = min[i].min(pos[i]);
            max[i] = max[i].max(pos[i]);
        }
    }

    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];

    // Center geometry at origin for bone socket attachment and tumbling
    for pos in &mut mesh.positions {
        pos[0] -= center[0];
        pos[1] -= center[1];
        pos[2] -= center[2];
    }

    let arc = Arc::new(mesh);
    cache.insert(key, arc.clone());
    Some(arc)
}

/// Load and cache the centered rock render mesh.
pub fn get_or_load_rock_mesh(assets_root: &str, stage: u8) -> Option<Arc<RenderMesh>> {
    let stage_name = match stage {
        2 => "projectile_stage2.cgf",
        3 => "projectile_stage3.cgf",
        _ => "projectile_stage1.cgf",
    };
    get_or_load_prop_mesh(
        assets_root,
        &format!("objects/effects/abilities/goliath/rock_throw/{stage_name}"),
    )
}

/// Multiply two 4x4 matrices.
fn mat4_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..4).map(|k| a[r][k] * b[k][c]).sum::<f32>()))
}

/// Construct a translation matrix.
fn mat4_translation(t: [f32; 3]) -> Mat4 {
    [
        [1.0, 0.0, 0.0, t[0]],
        [0.0, 1.0, 0.0, t[1]],
        [0.0, 0.0, 1.0, t[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Construct a rotation matrix around X axis (pitch / tumble).
fn mat4_rot_x(angle: f32) -> Mat4 {
    let (s, c) = (angle.sin(), angle.cos());
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, c, -s, 0.0],
        [0.0, s, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropEffectKind {
    GoliathRock,
    Cocoon { monster_family: &'static str },
    WraithWarpBlast,
    BehemothLavaBomb,
    GlacialBehemothIceSpikes,
    HunterDeployable { hunter: &'static str },
}

/// Classify whether an animation clip triggers an authored prop, ability geometry, or projectile.
pub fn classify_animation_prop(clip_name: &str) -> Option<PropEffectKind> {
    let lower = clip_name.to_ascii_lowercase();

    // 1. Goliath Rock Abilities
    if lower.contains("rock_pick_up")
        || lower.contains("rock_hold")
        || lower.contains("rock_throw")
        || lower.contains("rock_release")
        || lower.contains("rock_turn")
        || lower.contains("rockthrow")
    {
        return Some(PropEffectKind::GoliathRock);
    }

    // 2. Stage-Up Evolve Cocoons per Monster Family
    if lower.contains("cocoon") || lower.contains("stageup") || lower.contains("stage_up") {
        let family = if lower.contains("goliathmeteor") {
            "goliathmeteor"
        } else if lower.contains("goliath") {
            "goliath"
        } else if lower.contains("rockybobcicle") {
            "rockybobcicle"
        } else if lower.contains("rocky") || lower.contains("behemoth") {
            "rocky"
        } else if lower.contains("kraken") || lower.contains("flyer") {
            "kraken"
        } else if lower.contains("wraith")
            || lower.contains("teleporter")
            || lower.contains("scorpid")
        {
            "wraith"
        } else if lower.contains("grounder")
            || lower.contains("parasite")
            || lower.contains("whipspider")
            || lower.contains("gorgon")
        {
            "gorgon"
        } else {
            "shared"
        };
        return Some(PropEffectKind::Cocoon {
            monster_family: family,
        });
    }

    // 3. Wraith Warp Blast & Supernova
    if lower.contains("warp_blast") || lower.contains("warpblast") || lower.contains("supernova") {
        return Some(PropEffectKind::WraithWarpBlast);
    }

    // 4. Glacial Behemoth (RockyBobcicle) Frost Spikes & Killing Frost
    if lower.contains("rockybobcicle")
        || lower.contains("rending_spikes")
        || lower.contains("killing_frost")
        || lower.contains("ice_shard")
        || lower.contains("ice_spikes")
    {
        return Some(PropEffectKind::GlacialBehemothIceSpikes);
    }

    // 5. Standard Behemoth (Rocky / Bob) Lava Bomb, Rock Wall & Molten Fissure
    if lower.contains("rocky")
        || lower.contains("behemoth")
        || lower.contains("lavaball")
        || lower.contains("lavabomb")
        || lower.contains("fissure")
        || lower.contains("rock_wall")
    {
        return Some(PropEffectKind::BehemothLavaBomb);
    }

    // 6. Hunter Deployable Equipment per Character
    if lower.contains("harpoon_mine")
        || lower.contains("mine_place")
        || lower.contains("grenade_throw")
        || lower.contains("mine_deploy")
        || lower.contains("deploy")
        || lower.contains("triangulator")
        || lower.contains("beacon")
        || lower.contains("sentry")
        || lower.contains("turret")
    {
        let hunter = if lower.contains("markov") {
            "markov"
        } else if lower.contains("maggie") {
            "maggie"
        } else if lower.contains("abe") {
            "abe"
        } else if lower.contains("griffin") {
            "griffin"
        } else if lower.contains("bucket") {
            "bucket"
        } else if lower.contains("hank") {
            "hank"
        } else if lower.contains("hyde") {
            "hyde"
        } else if lower.contains("caira") {
            "caira"
        } else {
            "generic"
        };
        return Some(PropEffectKind::HunterDeployable { hunter });
    }

    None
}

/// Determine whether an animation path or clip label corresponds to Goliath rock abilities.
#[allow(dead_code)]
pub fn is_rock_throw_animation(clip_name: &str) -> bool {
    classify_animation_prop(clip_name) == Some(PropEffectKind::GoliathRock)
}

pub struct PropSimulationResult {
    pub combined_mesh: RenderMesh,
    pub surfaces: Option<Arc<Vec<Surface>>>,
}

/// Evaluate animation props and projectiles for the current frame.
/// If an active prop is present, transforms and attaches/projects it,
/// merging the geometry and creating a matching glowing material surface.
pub fn simulate_frame_props(
    base_mesh: RenderMesh,
    base_surfaces: Option<&Arc<Vec<Surface>>>,
    clip_name: &str,
    motion_time: f32,
    assembly: Option<&evolve_core::VerifiedCafAssembly>,
    skin: Option<&evolve_core::VerifiedCafSkin>,
    layers: &[evolve_core::CafLayer<'_>],
    weighted_clips: Option<&[evolve_core::WeightedCaf<'_>]>,
    in_place: bool,
    assets_root: &str,
    monster_stage: u8,
) -> PropSimulationResult {
    let Some(prop_kind) = classify_animation_prop(clip_name) else {
        return PropSimulationResult {
            combined_mesh: base_mesh,
            surfaces: base_surfaces.cloned(),
        };
    };

    let lower = clip_name.to_ascii_lowercase();

    let (prop_mesh, prop_matrix, tint, emissive, emissive_strength) = match prop_kind {
        PropEffectKind::GoliathRock => {
            let Some(mesh) = get_or_load_rock_mesh(assets_root, monster_stage) else {
                return PropSimulationResult {
                    combined_mesh: base_mesh,
                    surfaces: base_surfaces.cloned(),
                };
            };

            let mat: Option<Mat4> = if lower.contains("rock_pick_up") {
                if motion_time < 0.35 {
                    None
                } else {
                    get_bone_transform(
                        "bone_goliath_R_hand",
                        motion_time,
                        assembly,
                        skin,
                        layers,
                        weighted_clips,
                        in_place,
                    )
                    .map(|hand| {
                        let offset = mat4_translation([0.2, 0.4, 0.0]);
                        mat4_mul(&hand, &offset)
                    })
                }
            } else if lower.contains("rock_hold")
                || lower.contains("rock_turn")
                || lower.contains("rockthrow")
            {
                get_bone_transform(
                    "bone_goliath_R_hand",
                    motion_time,
                    assembly,
                    skin,
                    layers,
                    weighted_clips,
                    in_place,
                )
                .map(|hand| {
                    let offset = mat4_translation([0.15, 0.45, -0.05]);
                    mat4_mul(&hand, &offset)
                })
            } else if lower.contains("rock_throw") {
                const THROW_RELEASE_TIME: f32 = 0.6726;
                if motion_time < THROW_RELEASE_TIME {
                    get_bone_transform(
                        "bone_goliath_R_hand",
                        motion_time,
                        assembly,
                        skin,
                        layers,
                        weighted_clips,
                        in_place,
                    )
                    .map(|hand| {
                        let offset = mat4_translation([0.15, 0.4, 0.0]);
                        mat4_mul(&hand, &offset)
                    })
                } else {
                    let release_hand = get_bone_transform(
                        "bone_goliath_R_hand",
                        THROW_RELEASE_TIME,
                        assembly,
                        skin,
                        layers,
                        weighted_clips,
                        in_place,
                    );

                    let (p0_x, p0_y, p0_z) = if let Some(h) = release_hand {
                        (h[0][3] + 0.2, h[1][3] + 0.5, h[2][3])
                    } else {
                        (0.0, 1.5, 2.5)
                    };

                    let delta_t = motion_time - THROW_RELEASE_TIME;
                    let v_forward = 28.0;
                    let v_up = 3.8;
                    let gravity = 9.81;

                    let x = p0_x;
                    let y = p0_y + v_forward * delta_t;
                    let z = (p0_z + v_up * delta_t - 0.5 * gravity * delta_t * delta_t).max(0.0);

                    let tumble = delta_t * 9.0;
                    let trans = mat4_translation([x, y, z]);
                    let rot = mat4_rot_x(tumble);
                    Some(mat4_mul(&trans, &rot))
                }
            } else if lower.contains("rock_release") {
                let delta_t = motion_time;
                let p0_x = 0.2;
                let p0_y = 1.8;
                let p0_z = 2.4;
                let v_forward = 30.0;
                let v_up = 2.5;
                let gravity = 9.81;

                let x = p0_x;
                let y = p0_y + v_forward * delta_t;
                let z = (p0_z + v_up * delta_t - 0.5 * gravity * delta_t * delta_t).max(0.0);

                let tumble = delta_t * 9.0;
                let trans = mat4_translation([x, y, z]);
                let rot = mat4_rot_x(tumble);
                Some(mat4_mul(&trans, &rot))
            } else {
                None
            };

            (mesh, mat, [0.45, 0.38, 0.32, 1.0], [1.0, 0.32, 0.05], 2.2)
        }
        PropEffectKind::Cocoon { monster_family } => {
            let (rel_path, tint, emissive, strength) = match monster_family {
                "goliathmeteor" => (
                    "objects/effects/abilities/shared/stageup_coccoon_goliath01.cgf",
                    [0.2, 0.4, 0.6, 1.0],
                    [0.10, 0.75, 1.0], // Celestial Plasma Blue
                    2.4,
                ),
                "goliath" => {
                    if monster_stage >= 3 {
                        (
                            "objects/effects/abilities/shared/stageup_coccoon_goliath01_in.cgf",
                            [0.4, 0.2, 0.15, 1.0],
                            [1.0, 0.22, 0.02], // Hardened Deep Magma Crimson
                            2.6,
                        )
                    } else {
                        (
                            "objects/effects/abilities/shared/stageup_coccoon_goliath01.cgf",
                            [0.5, 0.3, 0.2, 1.0],
                            [1.0, 0.40, 0.05], // Molten Ember Red/Orange
                            2.2,
                        )
                    }
                }
                "rockybobcicle" => (
                    "objects/effects/abilities/Rocky/stageup/stageup_coccoon_rocky_in.cgf",
                    [0.7, 0.9, 1.0, 1.0],
                    [0.15, 0.85, 1.0], // Glacial Permafrost Ice Blue
                    2.5,
                ),
                "rocky" => (
                    "objects/effects/abilities/Rocky/stageup/stageup_coccoon_rocky_in.cgf",
                    [0.35, 0.15, 0.08, 1.0],
                    [1.0, 0.25, 0.02], // Volcanic Basalt Magma (Regular Behemoth)
                    2.6,
                ),
                "kraken" => (
                    "objects/effects/abilities/Elder_Kraken/stageup/elderkraken_stageup_fin.cgf",
                    [0.2, 0.4, 0.5, 1.0],
                    [0.20, 0.65, 1.0], // Bio-electric Cyan Arc
                    2.2,
                ),
                "wraith" => (
                    "Objects/characters/monsters/cocoon/Teleporter/teleporter.cgf",
                    [0.3, 0.1, 0.4, 1.0],
                    [0.60, 0.10, 0.95], // Dimensional Void Violet
                    2.5,
                ),
                "gorgon" => (
                    "Objects/characters/monsters/cocoon/ParasiteHost/parasitehost.cgf",
                    [0.2, 0.5, 0.1, 1.0],
                    [0.35, 0.90, 0.10], // Toxic Acidic Chitin Green
                    2.2,
                ),
                _ => (
                    "Objects/characters/monsters/cocoon/cocoon01.cgf",
                    [0.4, 0.4, 0.3, 1.0],
                    [0.85, 0.60, 0.10], // Amber Organic
                    1.6,
                ),
            };

            let Some(mesh) = get_or_load_prop_mesh(assets_root, rel_path) else {
                return PropSimulationResult {
                    combined_mesh: base_mesh,
                    surfaces: base_surfaces.cloned(),
                };
            };
            (
                mesh,
                Some(mat4_translation([0.0, 0.0, 0.0])),
                tint,
                emissive,
                strength,
            )
        }
        PropEffectKind::WraithWarpBlast => {
            let Some(mesh) = get_or_load_prop_mesh(
                assets_root,
                "objects/effects/abilities/wraith/warp_blast/warpblast_bubble_cluster.cgf",
            ) else {
                return PropSimulationResult {
                    combined_mesh: base_mesh,
                    surfaces: base_surfaces.cloned(),
                };
            };
            let mat = get_bone_transform(
                "Bip01 Spine",
                motion_time,
                assembly,
                skin,
                layers,
                weighted_clips,
                in_place,
            )
            .or_else(|| {
                get_bone_transform(
                    "bone_R_hand",
                    motion_time,
                    assembly,
                    skin,
                    layers,
                    weighted_clips,
                    in_place,
                )
            })
            .map(|b| mat4_mul(&b, &mat4_translation([0.0, 1.2, 0.2])))
            .or_else(|| Some(mat4_translation([0.0, 1.5, 1.2])));

            (mesh, mat, [0.2, 0.1, 0.3, 1.0], [0.55, 0.15, 0.95], 2.4)
        }
        PropEffectKind::GlacialBehemothIceSpikes => {
            // Glacial Behemoth (RockyBobcicle) ONLY - crystalline frost ice
            let Some(mesh) = get_or_load_prop_mesh(
                assets_root,
                "objects/effects/abilities/RockyBobcicle/rending_spikes/rending_spikes_ice02.cgf",
            ) else {
                return PropSimulationResult {
                    combined_mesh: base_mesh,
                    surfaces: base_surfaces.cloned(),
                };
            };
            (
                mesh,
                Some(mat4_translation([0.0, 3.2, 0.0])),
                [0.6, 0.8, 1.0, 1.0],
                [0.15, 0.80, 1.0], // Frost Blue / Cyan (Glacial Behemoth)
                2.2,
            )
        }
        PropEffectKind::BehemothLavaBomb => {
            // Standard Behemoth (Rocky / Bob) - molten incandescent red-orange lava
            let Some(mesh) =
                get_or_load_prop_mesh(assets_root, "objects/effects/lava/lavaball.cgf")
            else {
                return PropSimulationResult {
                    combined_mesh: base_mesh,
                    surfaces: base_surfaces.cloned(),
                };
            };
            (
                mesh,
                Some(mat4_translation([0.0, 2.5, 0.5])),
                [0.45, 0.20, 0.10, 1.0],
                [1.0, 0.28, 0.02], // Molten Incandescent Red/Orange Magma (Regular Behemoth)
                2.8,
            )
        }
        PropEffectKind::HunterDeployable { hunter } => {
            let (rel_path, tint, emissive, strength) = match hunter {
                "markov" => (
                    "characters/hunters/shared/weapons/projectiles/grenade_brass.cgf",
                    [0.8, 0.7, 0.3, 1.0],
                    [0.15, 0.65, 1.0], // Arc Lightning Blue (Markov Arc Mine)
                    2.2,
                ),
                "maggie" => (
                    "objects/weapons/Objects/weapons/harpoon_mine/harpoon_mine_projectile_tp.cgf",
                    [0.3, 0.3, 0.3, 1.0],
                    [1.0, 0.35, 0.0], // Sensor Hazard Red/Orange (Maggie Harpoon Trap)
                    1.8,
                ),
                "abe" => (
                    "characters/hunters/shared/weapons/projectiles/grenade_blue.cgf",
                    [0.2, 0.3, 0.7, 1.0],
                    [0.50, 0.15, 1.0], // Stasis Field Purple/Blue (Abe Stasis Grenade)
                    2.2,
                ),
                "griffin" => (
                    "objects/weapons/Objects/weapons/Sound_Triangulator_fp/Sound_Triangulator_tp.cgf",
                    [0.2, 0.35, 0.2, 1.0],
                    [0.10, 1.0, 0.35], // Acoustic Pulse Neon Green (Griffin Sound Spike)
                    2.0,
                ),
                "bucket" => (
                    "objects/effects/items/sentry/sentrygun_3p.cgf",
                    [0.7, 0.65, 0.2, 1.0],
                    [1.0, 0.85, 0.10], // Autonomous Sentry Amber/Yellow (Bucket Sentry Turret)
                    1.9,
                ),
                "hank" => (
                    "objects/weapons/Objects/weapons/beacon_tp/beacon_tp.cgf",
                    [0.3, 0.3, 0.3, 1.0],
                    [1.0, 0.05, 0.05], // Laser Target Crimson Beam (Hank Orbital Beacon)
                    2.5,
                ),
                "hyde" => (
                    "characters/hunters/shared/weapons/projectiles/grenade_projectile.cgf",
                    [0.2, 0.4, 0.1, 1.0],
                    [0.25, 0.95, 0.05], // Caustic Chemical Acid Green (Hyde Toxic Grenade)
                    2.0,
                ),
                "caira" => (
                    "characters/hunters/shared/weapons/projectiles/grenade_blue.cgf",
                    [0.1, 0.5, 0.5, 1.0],
                    [0.10, 0.90, 0.85], // Med-field Healing Teal (Caira Healing Grenade)
                    2.0,
                ),
                _ => (
                    "characters/hunters/shared/weapons/projectiles/grenade_projectile.cgf",
                    [0.35, 0.35, 0.35, 1.0],
                    [1.0, 0.60, 0.05], // Tactical Safety Amber (Generic Throw)
                    1.6,
                ),
            };

            let Some(mesh) = get_or_load_prop_mesh(assets_root, rel_path) else {
                return PropSimulationResult {
                    combined_mesh: base_mesh,
                    surfaces: base_surfaces.cloned(),
                };
            };

            let mat = if motion_time < 0.5 {
                get_bone_transform(
                    "Bip01 R Hand",
                    motion_time,
                    assembly,
                    skin,
                    layers,
                    weighted_clips,
                    in_place,
                )
                .or_else(|| {
                    get_bone_transform(
                        "weapon_bone",
                        motion_time,
                        assembly,
                        skin,
                        layers,
                        weighted_clips,
                        in_place,
                    )
                })
                .map(|h| mat4_mul(&h, &mat4_translation([0.1, 0.2, 0.0])))
                .or_else(|| Some(mat4_translation([0.2, 0.8, 1.2])))
            } else {
                Some(mat4_translation([0.0, 1.2, 0.0]))
            };

            (mesh, mat, tint, emissive, strength)
        }
    };

    let Some(world) = prop_matrix else {
        return PropSimulationResult {
            combined_mesh: base_mesh,
            surfaces: base_surfaces.cloned(),
        };
    };

    // Transform and attach the prop mesh
    let transformed_prop = transform_render_mesh(&prop_mesh, &world);
    let mut combined = base_mesh;
    let base_indices_count = combined.indices.len() as u32;
    append_render_mesh(&mut combined, &transformed_prop);

    // Update GPU material surfaces so the prop renders with authentic glow
    let surfaces = base_surfaces.map(|base| {
        let mut list = (**base).clone();
        let prop_surface = Surface {
            first_index: base_indices_count,
            index_count: prop_mesh.indices.len() as u32,
            visible: true,
            transparent: false,
            foliage_cutout: false,
            distance_clouds: None,
            uv_transform: None,
            material_tint: Some(tint),
            material_specular: Some([0.25, 0.22, 0.20]),
            material_emissive: Some(emissive),
            emission_on_diffuse: false,
            emission_alpha_mask: false,
            material_shininess: Some(25.0),
            waterfall: None,
            detail_map: None,
            blend_layer: None,
            blend_mask: None,
            vertex_colors: false,
            blend_normal: None,
            detail_tile: None,
            layer_params: None,
            foliage_transmit: None,
            layer_specular: None,
            alpha_cutoff: 0.5,
            diffuse: None,
            normal: None,
            emissive: None,
            armor_emissive: None,
            emission_strength: emissive_strength,
            emissive_uv: None,
            specular: None,
            gloss: None,
            opacity: None,
        };
        list.push(prop_surface);
        Arc::new(list)
    });

    PropSimulationResult {
        combined_mesh: combined,
        surfaces,
    }
}

/// Helper to get a bone matrix from assembly or skin.
fn get_bone_transform(
    bone_name: &str,
    seconds: f32,
    assembly: Option<&evolve_core::VerifiedCafAssembly>,
    skin: Option<&evolve_core::VerifiedCafSkin>,
    layers: &[evolve_core::CafLayer<'_>],
    weighted_clips: Option<&[evolve_core::WeightedCaf<'_>]>,
    in_place: bool,
) -> Option<Mat4> {
    if let Some(weights) = weighted_clips {
        if let Some(assembly) = assembly {
            if let Some(mat) = assembly.posed_bone_world_weighted(bone_name, weights, in_place) {
                return Some(mat);
            }
        }
        if let Some(skin) = skin {
            if let Some(mat) = skin.posed_bone_world_weighted(bone_name, weights, in_place) {
                return Some(mat);
            }
        }
    }
    if let Some(assembly) = assembly {
        if let Some(mat) = assembly.posed_bone_world(bone_name, seconds, layers, in_place) {
            return Some(mat);
        }
    }
    if let Some(skin) = skin {
        if let Some(mat) = skin.posed_bone_world(bone_name, seconds, layers, in_place) {
            return Some(mat);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_animation_prop_global() {
        assert_eq!(
            classify_animation_prop("ability_rock_throw"),
            Some(PropEffectKind::GoliathRock)
        );
        assert_eq!(
            classify_animation_prop("characters/monsters/shared/cocoon_enter.caf"),
            Some(PropEffectKind::Cocoon {
                monster_family: "shared"
            })
        );
        assert_eq!(
            classify_animation_prop("characters/monsters/rocky/stageup_start.caf"),
            Some(PropEffectKind::Cocoon {
                monster_family: "rocky"
            })
        );
        assert_eq!(
            classify_animation_prop("characters/monsters/wraith/ability_warp_blast.caf"),
            Some(PropEffectKind::WraithWarpBlast)
        );
        assert_eq!(
            classify_animation_prop("characters/monsters/rockybobcicle/rending_spikes.caf"),
            Some(PropEffectKind::GlacialBehemothIceSpikes)
        );
        assert_eq!(
            classify_animation_prop("characters/monsters/rocky/lavabomb.caf"),
            Some(PropEffectKind::BehemothLavaBomb)
        );
        assert_eq!(
            classify_animation_prop("characters/hunters/merc_markov/mine_place.caf"),
            Some(PropEffectKind::HunterDeployable { hunter: "markov" })
        );
        assert_eq!(
            classify_animation_prop("characters/hunters/harpoon_mine_deploy.caf"),
            Some(PropEffectKind::HunterDeployable { hunter: "generic" })
        );
        assert_eq!(classify_animation_prop("locomotion_walk"), None);
    }

    #[test]
    fn test_is_rock_throw_animation() {
        // Authentic rock throw animations
        assert!(is_rock_throw_animation("ability_rock_pick_up"));
        assert!(is_rock_throw_animation("ability_rock_hold"));
        assert!(is_rock_throw_animation("ability_rock_throw"));
        assert!(is_rock_throw_animation("ability_rock_release"));
        assert!(is_rock_throw_animation("ability_rock_turn_left"));
        assert!(is_rock_throw_animation("ability_rock_turn_right"));
        assert!(is_rock_throw_animation("ability_rock_turn_center"));
        assert!(is_rock_throw_animation("BS_RockThrow_Turn"));
        assert!(is_rock_throw_animation("RockThrow"));

        // Non-rock animations should return false
        assert!(!is_rock_throw_animation("ability_burrow_start"));
        assert!(!is_rock_throw_animation("ability_burrow_idle"));
        assert!(!is_rock_throw_animation("ability_burrow_exit"));
        assert!(!is_rock_throw_animation("ability_fire_breath"));
        assert!(!is_rock_throw_animation("ability_charge_attack"));
        assert!(!is_rock_throw_animation("locomotion_walk_forward"));
        assert!(!is_rock_throw_animation("locomotion_jump_forward_start"));
        assert!(!is_rock_throw_animation("idle"));
    }

    #[test]
    fn test_mat4_math() {
        let ident = mat4_mul(&mat4_translation([0.0, 0.0, 0.0]), &mat4_rot_x(0.0));
        let expected = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        assert_eq!(ident, expected);

        let t = mat4_translation([10.0, 20.0, 30.0]);
        assert_eq!(t[0][3], 10.0);
        assert_eq!(t[1][3], 20.0);
        assert_eq!(t[2][3], 30.0);
    }

    #[test]
    fn test_simulate_frame_props_passthrough_for_non_rock() {
        let mesh = RenderMesh {
            info: evolve_core::ModelInfo {
                format: "Test".to_string(),
                version: 1,
                chunk_count: 1,
                vertex_count: 3,
                triangle_count: 1,
                original_uv0: true,
                original_normals: true,
            },
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: Some(vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]]),
            uv0: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
            vertex_alpha: None,
            vertex_color: None,
            indices: vec![0, 1, 2],
        };

        let result = simulate_frame_props(
            mesh.clone(),
            None,
            "locomotion_walk_forward",
            1.0,
            None,
            None,
            &[],
            None,
            false,
            "C:/dummy",
            1,
        );

        assert_eq!(result.combined_mesh.positions.len(), 3);
        assert_eq!(result.combined_mesh.indices.len(), 3);
        assert!(result.surfaces.is_none());
    }
}
