//! Authored animation props, ability VFX, and projectile simulation.
//!
//! Evaluates bone-attached props (e.g. Goliath rock pickup/hold, Meteor Goliath
//! ambient body flames, Flame Breath torrents), stage-up cocoons, and ballistic
//! projectiles in real time across playback and timeline scrubbing.
use crate::textured::Surface;
use evolve_core::{RenderMesh, append_render_mesh, transform_render_mesh};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

type Mat4 = [[f32; 4]; 4];

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PropPivotMode {
    Centered,   // Socket attached (rocks, grenades, ability flames)
    GroundBase, // Floor resting (stageup cocoons, deployable turrets, spikes)
}

/// Global cache for loaded prop geometry to prevent repeated file I/O during playback.
static PROP_CACHE: OnceLock<Mutex<std::collections::HashMap<String, Arc<RenderMesh>>>> =
    OnceLock::new();

fn prop_cache() -> &'static Mutex<std::collections::HashMap<String, Arc<RenderMesh>>> {
    PROP_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Global cache for decoded prop textures.
static PROP_TEX_CACHE: OnceLock<Mutex<std::collections::HashMap<String, Arc<image::RgbaImage>>>> =
    OnceLock::new();

fn prop_tex_cache() -> &'static Mutex<std::collections::HashMap<String, Arc<image::RgbaImage>>> {
    PROP_TEX_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Locate any prop, effect, or weapon CGF or DDS from the extracted asset library.
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

/// Load and cache any render mesh with the specified pivot mode.
pub fn get_or_load_prop_mesh_with_pivot(
    assets_root: &str,
    relative_path: &str,
    pivot_mode: PropPivotMode,
) -> Option<Arc<RenderMesh>> {
    let path = find_prop_cgf(assets_root, relative_path)?;
    let key = format!("{}:{pivot_mode:?}", path.to_string_lossy());

    let mut cache = prop_cache().lock().ok()?;
    if let Some(mesh) = cache.get(&key) {
        return Some(mesh.clone());
    }

    let mut mesh = evolve_core::model::read_model(&path).ok()?;
    if mesh.positions.is_empty() {
        return None;
    }

    // Compute bounding box
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

    match pivot_mode {
        PropPivotMode::Centered => {
            for pos in &mut mesh.positions {
                pos[0] -= center[0];
                pos[1] -= center[1];
                pos[2] -= center[2];
            }
        }
        PropPivotMode::GroundBase => {
            // Ground-base props rest with their lowest point exactly on the floor plane z=0.0
            for pos in &mut mesh.positions {
                pos[0] -= center[0];
                pos[1] -= center[1];
                pos[2] -= min[2];
            }
        }
    }

    let arc = Arc::new(mesh);
    cache.insert(key, arc.clone());
    Some(arc)
}

/// Load and cache any centered render mesh.
pub fn get_or_load_prop_mesh(assets_root: &str, relative_path: &str) -> Option<Arc<RenderMesh>> {
    get_or_load_prop_mesh_with_pivot(assets_root, relative_path, PropPivotMode::Centered)
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

/// Load and decode a DDS texture into an Arc<RgbaImage>.
pub fn get_or_load_prop_texture(
    assets_root: &str,
    relative_path: &str,
) -> Option<Arc<image::RgbaImage>> {
    let mut key = relative_path.replace('\\', "/").to_ascii_lowercase();
    if !key.ends_with(".dds.0") && key.ends_with(".dds") {
        key = format!("{key}.0");
    }

    let mut cache = prop_tex_cache().lock().ok()?;
    if let Some(img) = cache.get(&key) {
        return Some(img.clone());
    }

    if let Some(path) = find_prop_cgf(assets_root, &key) {
        if let Ok((_, rgba)) = evolve_core::decode_dds_rgba(&path) {
            let arc = Arc::new(rgba);
            cache.insert(key, arc.clone());
            return Some(arc);
        }
    }
    None
}

/// Generate a high-contrast procedural texture fallback if the exact DDS is absent.
pub fn generate_procedural_texture(
    color_a: [u8; 4],
    color_b: [u8; 4],
    pattern: u8,
) -> Arc<image::RgbaImage> {
    let width = 64;
    let height = 64;
    let mut img = image::RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let factor = match pattern {
                0 => (((x / 8) + (y / 8)) % 2 == 0) as u8 as f32,
                1 => ((x * 7 + y * 13) % 19) as f32 / 19.0,
                _ => (((x as f32 - 32.0).hypot(y as f32 - 32.0) as u32 / 6) % 2 == 0) as u8 as f32,
            };
            let r = (color_a[0] as f32 * (1.0 - factor) + color_b[0] as f32 * factor) as u8;
            let g = (color_a[1] as f32 * (1.0 - factor) + color_b[1] as f32 * factor) as u8;
            let b = (color_a[2] as f32 * (1.0 - factor) + color_b[2] as f32 * factor) as u8;
            let a = (color_a[3] as f32 * (1.0 - factor) + color_b[3] as f32 * factor) as u8;
            img.put_pixel(x, y, image::Rgba([r, g, b, a]));
        }
    }
    Arc::new(img)
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

/// Construct a non-uniform scale matrix.
fn mat4_scale(s: [f32; 3]) -> Mat4 {
    [
        [s[0], 0.0, 0.0, 0.0],
        [0.0, s[1], 0.0, 0.0],
        [0.0, 0.0, s[2], 0.0],
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

/// Construct a rotation matrix around Z axis (yaw).
fn mat4_rot_z(angle: f32) -> Mat4 {
    let (s, c) = (angle.sin(), angle.cos());
    [
        [c, -s, 0.0, 0.0],
        [s, c, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropEffectKind {
    GoliathRock,
    Cocoon { monster_family: &'static str },
    FlameBreath { is_meteor: bool },
    LeapAttackSmash { is_meteor: bool },
    ChargeAttack { is_meteor: bool },
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

    // 2. Flame Breath Ability VFX
    if lower.contains("fire_breath") || lower.contains("flame_breath") {
        return Some(PropEffectKind::FlameBreath {
            is_meteor: lower.contains("meteor"),
        });
    }

    // 3. Leap Attack Smash Impact VFX
    if lower.contains("leap_attack_smash")
        || lower.contains("leap_attack_smash_alt")
        || lower.contains("leap_attack_smash_wall")
    {
        return Some(PropEffectKind::LeapAttackSmash {
            is_meteor: lower.contains("meteor"),
        });
    }

    // 4. Charge Attack Kinetic Slipstream VFX
    if lower.contains("charge_attack") || lower.contains("charge_forward") {
        return Some(PropEffectKind::ChargeAttack {
            is_meteor: lower.contains("meteor"),
        });
    }

    // 5. Stage-Up Evolve Cocoons per Monster Family
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

    // 6. Wraith Warp Blast & Supernova
    if lower.contains("warp_blast") || lower.contains("warpblast") || lower.contains("supernova") {
        return Some(PropEffectKind::WraithWarpBlast);
    }

    // 7. Glacial Behemoth (RockyBobcicle) Frost Spikes & Killing Frost
    if lower.contains("rockybobcicle")
        || lower.contains("rending_spikes")
        || lower.contains("killing_frost")
        || lower.contains("ice_shard")
        || lower.contains("ice_spikes")
    {
        return Some(PropEffectKind::GlacialBehemothIceSpikes);
    }

    // 8. Standard Behemoth (Rocky / Bob) Lava Bomb, Rock Wall & Molten Fissure
    if lower.contains("rocky")
        || lower.contains("behemoth")
        || lower.contains("lavaball")
        || lower.contains("lavabomb")
        || lower.contains("fissure")
        || lower.contains("rock_wall")
    {
        return Some(PropEffectKind::BehemothLavaBomb);
    }

    // 9. Hunter Deployable Equipment per Character
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

struct ActivePropEntry {
    mesh: Arc<RenderMesh>,
    matrix: Mat4,
    diffuse: Option<Arc<image::RgbaImage>>,
    emissive: Option<Arc<image::RgbaImage>>,
    tint: [f32; 4],
    emissive_color: [f32; 3],
    emissive_strength: f32,
}

/// Evaluate animation props and projectiles for the current frame.
/// If active props or ability VFX are present, transforms and attaches them,
/// merging the geometry and ensuring 100% contiguous GPU surface index tables.
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
    let lower = clip_name.to_ascii_lowercase();
    let is_meteor = lower.contains("meteor") || lower.contains("goliathmeteor");
    let prop_kind = classify_animation_prop(clip_name);

    if prop_kind.is_none() && !is_meteor {
        return PropSimulationResult {
            combined_mesh: base_mesh,
            surfaces: base_surfaces.cloned(),
        };
    }

    let mut active_props: Vec<ActivePropEntry> = Vec::new();

    // 1. Primary Prop / Ability Effect
    if let Some(kind) = prop_kind {
        match kind {
            PropEffectKind::GoliathRock => {
                if let Some(mesh) = get_or_load_rock_mesh(assets_root, monster_stage) {
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
                            let z = (p0_z + v_up * delta_t - 0.5 * gravity * delta_t * delta_t)
                                .max(0.0);

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
                        let z =
                            (p0_z + v_up * delta_t - 0.5 * gravity * delta_t * delta_t).max(0.0);

                        let tumble = delta_t * 9.0;
                        let trans = mat4_translation([x, y, z]);
                        let rot = mat4_rot_x(tumble);
                        Some(mat4_mul(&trans, &rot))
                    } else {
                        None
                    };

                    if let Some(matrix) = mat {
                        let (diff, glow, tint, emissive_color, strength) = if is_meteor {
                            (
                                get_or_load_prop_texture(
                                    assets_root,
                                    "objects/effects/abilities/Goliath_Meteor/goliathmeteor_rock_smolder01_diff.dds.0",
                                )
                                .or_else(|| {
                                    Some(generate_procedural_texture(
                                        [50, 60, 80, 255],
                                        [10, 20, 30, 255],
                                        1,
                                    ))
                                }),
                                get_or_load_prop_texture(
                                    assets_root,
                                    "objects/effects/abilities/Goliath_Meteor/goliathmeteor_rock_smolder01_glow.dds.0",
                                ),
                                [0.35, 0.45, 0.65, 1.0],
                                [0.10, 0.75, 1.0], // Celestial Plasma Blue
                                2.6,
                            )
                        } else {
                            (
                                get_or_load_prop_texture(
                                    assets_root,
                                    "objects/effects/abilities/goliath/rock_throw/rock_throw_smolder01_diff.dds.0",
                                )
                                .or_else(|| {
                                    Some(generate_procedural_texture(
                                        [80, 70, 60, 255],
                                        [40, 35, 30, 255],
                                        1,
                                    ))
                                }),
                                get_or_load_prop_texture(
                                    assets_root,
                                    "objects/effects/abilities/goliath/rock_throw/rock_throw_glow01.dds.0",
                                ),
                                [0.55, 0.48, 0.42, 1.0],
                                [1.0, 0.35, 0.05], // Molten Amber
                                2.4,
                            )
                        };

                        active_props.push(ActivePropEntry {
                            mesh,
                            matrix,
                            diffuse: diff,
                            emissive: glow,
                            tint,
                            emissive_color,
                            emissive_strength: strength,
                        });
                    }
                }
            }
            PropEffectKind::FlameBreath {
                is_meteor: is_meteor_breath,
            } => {
                // Torrent of expanding fire projecting forward from the mouth
                if let Some(mesh) = get_or_load_prop_mesh(
                    assets_root,
                    "objects/effects/shared/fxm_cylinder_curved01_1m.cgf",
                ) {
                    let head = get_bone_transform(
                        "Bip01 Head",
                        motion_time,
                        assembly,
                        skin,
                        layers,
                        weighted_clips,
                        in_place,
                    )
                    .or_else(|| {
                        get_bone_transform(
                            "tongue_a",
                            motion_time,
                            assembly,
                            skin,
                            layers,
                            weighted_clips,
                            in_place,
                        )
                    });

                    if let Some(h) = head {
                        let fwd_trans = mat4_translation([0.0, 2.5, -0.2]);
                        let cone_scale = mat4_scale([1.8, 5.2, 1.8]);
                        let roll = mat4_rot_x(motion_time * 8.0);
                        let local_t = mat4_mul(&fwd_trans, &mat4_mul(&roll, &cone_scale));
                        let matrix = mat4_mul(&h, &local_t);

                        let (tint, emissive_color, strength) = if is_meteor_breath || is_meteor {
                            ([0.2, 0.5, 0.9, 1.0], [0.10, 0.75, 1.0], 3.2)
                        } else {
                            ([0.9, 0.4, 0.1, 1.0], [1.0, 0.45, 0.05], 3.0)
                        };

                        let flame_tex = get_or_load_prop_texture(
                            assets_root,
                            "textures/sprites/fire_flame7.dds.0",
                        )
                        .or_else(|| {
                            Some(generate_procedural_texture(
                                [255, 200, 50, 255],
                                [255, 80, 10, 255],
                                2,
                            ))
                        });

                        active_props.push(ActivePropEntry {
                            mesh,
                            matrix,
                            diffuse: flame_tex.clone(),
                            emissive: flame_tex,
                            tint,
                            emissive_color,
                            emissive_strength: strength,
                        });
                    }
                }
            }
            PropEffectKind::LeapAttackSmash { is_meteor: _ } => {
                // Expanding kinetic impact shockwave ring on the ground plane
                if let Some(mesh) = get_or_load_prop_mesh_with_pivot(
                    assets_root,
                    "objects/effects/shared/fxm_disc_ring01_1m.cgf",
                    PropPivotMode::GroundBase,
                ) {
                    let impact_time = 0.20;
                    if motion_time >= impact_time {
                        let dt = motion_time - impact_time;
                        let ring_scale = 2.5 + dt * 9.0;
                        let matrix = mat4_scale([ring_scale, ring_scale, 1.0]);

                        active_props.push(ActivePropEntry {
                            mesh,
                            matrix,
                            diffuse: Some(generate_procedural_texture(
                                [255, 180, 50, 255],
                                [200, 50, 10, 255],
                                2,
                            )),
                            emissive: None,
                            tint: [1.0, 0.6, 0.1, 1.0],
                            emissive_color: [1.0, 0.4, 0.05],
                            emissive_strength: 2.8,
                        });
                    }
                }
            }
            PropEffectKind::ChargeAttack { is_meteor: _ } => {
                // Kinetic slipstream ring attached around the monster torso
                if let Some(mesh) = get_or_load_prop_mesh(
                    assets_root,
                    "objects/effects/shared/fxm_disc_ring01_1m_curved.cgf",
                ) {
                    if let Some(spine) = get_bone_transform(
                        "bone_goliath_chest",
                        motion_time,
                        assembly,
                        skin,
                        layers,
                        weighted_clips,
                        in_place,
                    ) {
                        let ring_scale = mat4_scale([2.4, 2.4, 2.4]);
                        let matrix = mat4_mul(&spine, &ring_scale);

                        active_props.push(ActivePropEntry {
                            mesh,
                            matrix,
                            diffuse: Some(generate_procedural_texture(
                                [240, 200, 150, 255],
                                [150, 120, 80, 255],
                                0,
                            )),
                            emissive: None,
                            tint: [0.8, 0.7, 0.5, 1.0],
                            emissive_color: [1.0, 0.8, 0.3],
                            emissive_strength: 2.2,
                        });
                    }
                }
            }
            PropEffectKind::Cocoon { monster_family } => {
                let (rel_path, tint, emissive, strength, scale_factor) = match monster_family {
                    "goliathmeteor" => (
                        "objects/effects/abilities/shared/stageup_coccoon_goliath01.cgf",
                        [0.3, 0.5, 0.8, 1.0],
                        [0.10, 0.75, 1.0], // Celestial Blue
                        2.5,
                        match monster_stage {
                            2 => 1.35,
                            3 => 1.55,
                            _ => 1.20,
                        },
                    ),
                    "goliath" => {
                        let (path, emissive_col, mult) = if monster_stage >= 3 {
                            (
                                "objects/effects/abilities/shared/stageup_coccoon_goliath01_in.cgf",
                                [1.0, 0.22, 0.02],
                                1.50,
                            )
                        } else {
                            (
                                "objects/effects/abilities/shared/stageup_coccoon_goliath01.cgf",
                                [1.0, 0.40, 0.05],
                                if monster_stage == 2 { 1.30 } else { 1.15 },
                            )
                        };
                        (path, [0.6, 0.35, 0.25, 1.0], emissive_col, 2.4, mult)
                    }
                    "rockybobcicle" => (
                        "objects/effects/abilities/Rocky/stageup/stageup_coccoon_rocky_in.cgf",
                        [0.8, 0.95, 1.0, 1.0],
                        [0.15, 0.85, 1.0], // Glacial Permafrost Blue
                        2.6,
                        match monster_stage {
                            2 => 1.75,
                            3 => 1.95,
                            _ => 1.55,
                        },
                    ),
                    "rocky" => (
                        "objects/effects/abilities/Rocky/stageup/stageup_coccoon_rocky_in.cgf",
                        [0.45, 0.25, 0.15, 1.0],
                        [1.0, 0.25, 0.02], // Volcanic Basalt Magma (Regular Behemoth)
                        2.6,
                        match monster_stage {
                            2 => 1.75,
                            3 => 1.95,
                            _ => 1.55,
                        },
                    ),
                    "kraken" => (
                        "objects/effects/abilities/Elder_Kraken/stageup/elderkraken_stageup_fin.cgf",
                        [0.3, 0.5, 0.6, 1.0],
                        [0.20, 0.65, 1.0], // Bio-electric Cyan Arc
                        2.4,
                        match monster_stage {
                            2 => 1.35,
                            3 => 1.50,
                            _ => 1.25,
                        },
                    ),
                    "wraith" => (
                        "Objects/characters/monsters/cocoon/Teleporter/teleporter.cgf",
                        [0.4, 0.15, 0.5, 1.0],
                        [0.60, 0.10, 0.95], // Void Violet
                        2.5,
                        match monster_stage {
                            2 => 1.30,
                            3 => 1.40,
                            _ => 1.20,
                        },
                    ),
                    "gorgon" => (
                        "Objects/characters/monsters/cocoon/ParasiteHost/parasitehost.cgf",
                        [0.3, 0.6, 0.15, 1.0],
                        [0.35, 0.90, 0.10], // Toxic Acidic Chitin Green
                        2.3,
                        match monster_stage {
                            2 => 1.35,
                            3 => 1.45,
                            _ => 1.20,
                        },
                    ),
                    _ => (
                        "Objects/characters/monsters/cocoon/cocoon01.cgf",
                        [0.5, 0.5, 0.4, 1.0],
                        [0.85, 0.60, 0.10], // Amber Organic
                        2.0,
                        match monster_stage {
                            2 => 1.35,
                            3 => 1.50,
                            _ => 1.20,
                        },
                    ),
                };

                // GroundBase ensures the cocoon stands upright with base resting at z=0.0
                if let Some(mesh) = get_or_load_prop_mesh_with_pivot(
                    assets_root,
                    rel_path,
                    PropPivotMode::GroundBase,
                ) {
                    let matrix = mat4_scale([scale_factor, scale_factor, scale_factor]);
                    let diff_tex = get_or_load_prop_texture(
                        assets_root,
                        "Objects/characters/monsters/cocoon/cocoon01.dds.0",
                    )
                    .or_else(|| {
                        Some(generate_procedural_texture(
                            [90, 75, 55, 255],
                            [50, 40, 25, 255],
                            1,
                        ))
                    });

                    active_props.push(ActivePropEntry {
                        mesh,
                        matrix,
                        diffuse: diff_tex,
                        emissive: None,
                        tint,
                        emissive_color: emissive,
                        emissive_strength: strength,
                    });
                }
            }
            PropEffectKind::WraithWarpBlast => {
                if let Some(mesh) = get_or_load_prop_mesh(
                    assets_root,
                    "objects/effects/abilities/wraith/warp_blast/warpblast_bubble_cluster.cgf",
                ) {
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

                    if let Some(matrix) = mat {
                        active_props.push(ActivePropEntry {
                            mesh,
                            matrix,
                            diffuse: Some(generate_procedural_texture(
                                [150, 50, 200, 255],
                                [70, 10, 120, 255],
                                2,
                            )),
                            emissive: None,
                            tint: [0.3, 0.15, 0.45, 1.0],
                            emissive_color: [0.65, 0.15, 0.95],
                            emissive_strength: 2.6,
                        });
                    }
                }
            }
            PropEffectKind::GlacialBehemothIceSpikes => {
                if let Some(mesh) = get_or_load_prop_mesh_with_pivot(
                    assets_root,
                    "objects/effects/abilities/RockyBobcicle/rending_spikes/rending_spikes_ice02.cgf",
                    PropPivotMode::GroundBase,
                ) {
                    let matrix = mat4_translation([0.0, 3.2, 0.0]);
                    active_props.push(ActivePropEntry {
                        mesh,
                        matrix,
                        diffuse: Some(generate_procedural_texture(
                            [180, 220, 255, 255],
                            [120, 170, 220, 255],
                            1,
                        )),
                        emissive: None,
                        tint: [0.7, 0.9, 1.0, 1.0],
                        emissive_color: [0.15, 0.85, 1.0],
                        emissive_strength: 2.4,
                    });
                }
            }
            PropEffectKind::BehemothLavaBomb => {
                if let Some(mesh) =
                    get_or_load_prop_mesh(assets_root, "objects/effects/lava/lavaball.cgf")
                {
                    let matrix = mat4_translation([0.0, 2.5, 0.5]);
                    active_props.push(ActivePropEntry {
                        mesh,
                        matrix,
                        diffuse: Some(generate_procedural_texture(
                            [120, 40, 20, 255],
                            [40, 15, 5, 255],
                            1,
                        )),
                        emissive: None,
                        tint: [0.55, 0.25, 0.15, 1.0],
                        emissive_color: [1.0, 0.30, 0.02],
                        emissive_strength: 2.8,
                    });
                }
            }
            PropEffectKind::HunterDeployable { hunter } => {
                let (rel_path, tint, emissive, strength, pivot) = match hunter {
                    "markov" => (
                        "characters/hunters/shared/weapons/projectiles/grenade_brass.cgf",
                        [0.8, 0.7, 0.3, 1.0],
                        [0.15, 0.65, 1.0],
                        2.2,
                        PropPivotMode::Centered,
                    ),
                    "maggie" => (
                        "objects/weapons/Objects/weapons/harpoon_mine/harpoon_mine_projectile_tp.cgf",
                        [0.4, 0.4, 0.4, 1.0],
                        [1.0, 0.35, 0.0],
                        2.0,
                        PropPivotMode::GroundBase,
                    ),
                    "abe" => (
                        "characters/hunters/shared/weapons/projectiles/grenade_blue.cgf",
                        [0.2, 0.3, 0.7, 1.0],
                        [0.50, 0.15, 1.0],
                        2.2,
                        PropPivotMode::Centered,
                    ),
                    "griffin" => (
                        "objects/weapons/Objects/weapons/Sound_Triangulator_fp/Sound_Triangulator_tp.cgf",
                        [0.3, 0.45, 0.3, 1.0],
                        [0.10, 1.0, 0.35],
                        2.2,
                        PropPivotMode::GroundBase,
                    ),
                    "bucket" => (
                        "objects/effects/items/sentry/sentrygun_3p.cgf",
                        [0.8, 0.75, 0.3, 1.0],
                        [1.0, 0.85, 0.10],
                        2.0,
                        PropPivotMode::GroundBase,
                    ),
                    "hank" => (
                        "objects/weapons/Objects/weapons/beacon_tp/beacon_tp.cgf",
                        [0.4, 0.4, 0.4, 1.0],
                        [1.0, 0.05, 0.05],
                        2.5,
                        PropPivotMode::GroundBase,
                    ),
                    "hyde" => (
                        "characters/hunters/shared/weapons/projectiles/grenade_projectile.cgf",
                        [0.3, 0.5, 0.2, 1.0],
                        [0.25, 0.95, 0.05],
                        2.0,
                        PropPivotMode::Centered,
                    ),
                    "caira" => (
                        "characters/hunters/shared/weapons/projectiles/grenade_blue.cgf",
                        [0.2, 0.6, 0.6, 1.0],
                        [0.10, 0.90, 0.85],
                        2.0,
                        PropPivotMode::Centered,
                    ),
                    _ => (
                        "characters/hunters/shared/weapons/projectiles/grenade_projectile.cgf",
                        [0.4, 0.4, 0.4, 1.0],
                        [1.0, 0.60, 0.05],
                        1.8,
                        PropPivotMode::Centered,
                    ),
                };

                if let Some(mesh) = get_or_load_prop_mesh_with_pivot(assets_root, rel_path, pivot) {
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

                    if let Some(matrix) = mat {
                        let diff_tex = if hunter == "maggie" {
                            get_or_load_prop_texture(
                                assets_root,
                                "objects/weapons/harpoon_mine/harpoon_mine.dds.0",
                            )
                        } else {
                            None
                        }
                        .or_else(|| {
                            Some(generate_procedural_texture(
                                [80, 80, 80, 255],
                                [40, 40, 40, 255],
                                0,
                            ))
                        });

                        active_props.push(ActivePropEntry {
                            mesh,
                            matrix,
                            diffuse: diff_tex,
                            emissive: None,
                            tint,
                            emissive_color: emissive,
                            emissive_strength: strength,
                        });
                    }
                }
            }
        }
    }

    // 2. Meteor Goliath Ambient Body Flames (libs/EffectSets/GoliathMeteorBurningSelf.xml)
    // Persistent celestial blue flames burning on body/spine, hands, and mouth
    if is_meteor {
        if let Some(flame_mesh) = get_or_load_prop_mesh(
            assets_root,
            "objects/effects/shared/fxm_cylinder_curved01_1m.cgf",
        )
        .or_else(|| {
            get_or_load_prop_mesh(
                assets_root,
                "objects/effects/abilities/shared/sphere_twist.cgf",
            )
        }) {
            let flame_diffuse =
                get_or_load_prop_texture(assets_root, "textures/sprites/fire_flame7.dds.0")
                    .or_else(|| {
                        Some(generate_procedural_texture(
                            [50, 180, 255, 255],
                            [10, 80, 200, 255],
                            2,
                        ))
                    });

            // Bone 1: Back/Shoulder Body Flames (MeteorBurningSelfBody)
            if let Some(back) = get_bone_transform(
                "bone_goliath_R_shoulder_blade",
                motion_time,
                assembly,
                skin,
                layers,
                weighted_clips,
                in_place,
            )
            .or_else(|| {
                get_bone_transform(
                    "bone_goliath_spine02",
                    motion_time,
                    assembly,
                    skin,
                    layers,
                    weighted_clips,
                    in_place,
                )
            }) {
                let s = mat4_scale([0.45, 0.45, 0.65]);
                let roll = mat4_rot_z(motion_time * 4.0);
                let offset = mat4_translation([0.0, 0.15, 0.25]);
                let matrix = mat4_mul(&back, &mat4_mul(&offset, &mat4_mul(&roll, &s)));

                active_props.push(ActivePropEntry {
                    mesh: flame_mesh.clone(),
                    matrix,
                    diffuse: flame_diffuse.clone(),
                    emissive: flame_diffuse.clone(),
                    tint: [0.2, 0.6, 1.0, 1.0],
                    emissive_color: [0.10, 0.75, 1.0],
                    emissive_strength: 3.0,
                });
            }

            // Bone 2: Right Hand Flame (MeteorBurningSelfHands)
            if let Some(r_hand) = get_bone_transform(
                "bone_goliath_R_hand",
                motion_time,
                assembly,
                skin,
                layers,
                weighted_clips,
                in_place,
            ) {
                let s = mat4_scale([0.35, 0.35, 0.45]);
                let roll = mat4_rot_x(motion_time * 5.0);
                let offset = mat4_translation([0.1, 0.1, 0.0]);
                let matrix = mat4_mul(&r_hand, &mat4_mul(&offset, &mat4_mul(&roll, &s)));

                active_props.push(ActivePropEntry {
                    mesh: flame_mesh.clone(),
                    matrix,
                    diffuse: flame_diffuse.clone(),
                    emissive: flame_diffuse.clone(),
                    tint: [0.2, 0.6, 1.0, 1.0],
                    emissive_color: [0.10, 0.75, 1.0],
                    emissive_strength: 3.0,
                });
            }

            // Bone 3: Left Hand Flame (MeteorBurningSelfHands)
            if let Some(l_hand) = get_bone_transform(
                "bone_goliath_L_hand",
                motion_time,
                assembly,
                skin,
                layers,
                weighted_clips,
                in_place,
            ) {
                let s = mat4_scale([0.35, 0.35, 0.45]);
                let roll = mat4_rot_x(-motion_time * 5.0);
                let offset = mat4_translation([-0.1, 0.1, 0.0]);
                let matrix = mat4_mul(&l_hand, &mat4_mul(&offset, &mat4_mul(&roll, &s)));

                active_props.push(ActivePropEntry {
                    mesh: flame_mesh.clone(),
                    matrix,
                    diffuse: flame_diffuse.clone(),
                    emissive: flame_diffuse.clone(),
                    tint: [0.2, 0.6, 1.0, 1.0],
                    emissive_color: [0.10, 0.75, 1.0],
                    emissive_strength: 3.0,
                });
            }

            // Bone 4: Mouth Flame (MeteorBurningSelfMouth)
            if let Some(mouth) = get_bone_transform(
                "tongue_a",
                motion_time,
                assembly,
                skin,
                layers,
                weighted_clips,
                in_place,
            )
            .or_else(|| {
                get_bone_transform(
                    "Bip01 Head",
                    motion_time,
                    assembly,
                    skin,
                    layers,
                    weighted_clips,
                    in_place,
                )
            }) {
                let s = mat4_scale([0.30, 0.30, 0.40]);
                let offset = mat4_translation([0.0, 0.25, -0.05]);
                let matrix = mat4_mul(&mouth, &mat4_mul(&offset, &s));

                active_props.push(ActivePropEntry {
                    mesh: flame_mesh,
                    matrix,
                    diffuse: flame_diffuse.clone(),
                    emissive: flame_diffuse,
                    tint: [0.2, 0.6, 1.0, 1.0],
                    emissive_color: [0.10, 0.75, 1.0],
                    emissive_strength: 3.2,
                });
            }
        }
    }

    if active_props.is_empty() {
        return PropSimulationResult {
            combined_mesh: base_mesh,
            surfaces: base_surfaces.cloned(),
        };
    }

    // Merge all active props into combined_mesh
    let mut combined = base_mesh;
    let base_indices_count = combined.indices.len() as u32;

    // Build the surfaces list starting from pristine base surfaces,
    // or create a default base surface if base_surfaces is None.
    let mut surfaces_list = base_surfaces.map(|s| (**s).clone()).unwrap_or_else(|| {
        vec![Surface {
            first_index: 0,
            index_count: base_indices_count,
            visible: true,
            transparent: false,
            foliage_cutout: false,
            distance_clouds: None,
            uv_transform: None,
            material_tint: Some([1.0, 1.0, 1.0, 1.0]),
            material_specular: Some([0.3, 0.3, 0.3]),
            material_emissive: None,
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
            emission_strength: 0.0,
            emissive_uv: None,
            specular: None,
            gloss: None,
            opacity: None,
        }]
    });

    for prop in active_props {
        let transformed = transform_render_mesh(&prop.mesh, &prop.matrix);
        let prop_first_index = combined.indices.len() as u32;
        let prop_index_count = prop.mesh.indices.len() as u32;

        append_render_mesh(&mut combined, &transformed);

        let prop_surface = Surface {
            first_index: prop_first_index,
            index_count: prop_index_count,
            visible: true,
            transparent: false,
            foliage_cutout: false,
            distance_clouds: None,
            uv_transform: None,
            material_tint: Some(prop.tint),
            material_specular: Some([0.3, 0.3, 0.3]),
            material_emissive: Some(prop.emissive_color),
            emission_on_diffuse: false,
            emission_alpha_mask: false,
            material_shininess: Some(30.0),
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
            diffuse: prop.diffuse,
            normal: None,
            emissive: prop.emissive,
            armor_emissive: None,
            emission_strength: prop.emissive_strength,
            emissive_uv: None,
            specular: None,
            gloss: None,
            opacity: None,
        };
        surfaces_list.push(prop_surface);
    }

    PropSimulationResult {
        combined_mesh: combined,
        surfaces: Some(Arc::new(surfaces_list)),
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
            classify_animation_prop("ability_fire_breath"),
            Some(PropEffectKind::FlameBreath { is_meteor: false })
        );
        assert_eq!(
            classify_animation_prop("characters/monsters/goliathmeteor/fire_breath.caf"),
            Some(PropEffectKind::FlameBreath { is_meteor: true })
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
        assert!(is_rock_throw_animation("ability_rock_pick_up"));
        assert!(is_rock_throw_animation("ability_rock_hold"));
        assert!(is_rock_throw_animation("ability_rock_throw"));
        assert!(is_rock_throw_animation("ability_rock_release"));
        assert!(is_rock_throw_animation("ability_rock_turn_left"));
        assert!(is_rock_throw_animation("ability_rock_turn_right"));
        assert!(is_rock_throw_animation("ability_rock_turn_center"));
        assert!(is_rock_throw_animation("BS_RockThrow_Turn"));
        assert!(is_rock_throw_animation("RockThrow"));

        assert!(!is_rock_throw_animation("ability_burrow_start"));
        assert!(!is_rock_throw_animation("ability_burrow_idle"));
        assert!(!is_rock_throw_animation("ability_burrow_exit"));
        assert!(!is_rock_throw_animation("locomotion_walk_forward"));
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

        let s = mat4_scale([2.0, 3.0, 4.0]);
        assert_eq!(s[0][0], 2.0);
        assert_eq!(s[1][1], 3.0);
        assert_eq!(s[2][2], 4.0);
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

    #[test]
    fn test_procedural_texture_generation() {
        let tex = generate_procedural_texture([255, 0, 0, 255], [0, 0, 255, 255], 0);
        assert_eq!(tex.width(), 64);
        assert_eq!(tex.height(), 64);
        let pixel0 = tex.get_pixel(0, 0);
        assert_eq!(pixel0[3], 255);
    }
}
