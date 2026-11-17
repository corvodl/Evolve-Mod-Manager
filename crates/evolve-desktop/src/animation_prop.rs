//! Authored animation props, deployables, and projectile simulation.
//!
//! Evaluates bone-attached props (e.g. Goliath rock pickup/hold), stage-up cocoons,
//! hunter deployables, and ballistic projectiles in real time across playback and timeline
//! scrubbing with zero GPU surface table thrashing and zero flickering.
use crate::textured::Surface;
use evolve_core::{RenderMesh, append_render_mesh, transform_render_mesh};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

type Mat4 = [[f32; 4]; 4];

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PropPivotMode {
    Centered,   // Socket attached (rocks, grenades, projectiles)
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

/// Global cache for assembled prop surfaces to prevent reallocating Arc<Vec<Surface>>
/// every frame, which guarantees GPU surface identity stability and zero flickering.
static PROP_SURFACE_CACHE: OnceLock<Mutex<std::collections::HashMap<String, Arc<Vec<Surface>>>>> =
    OnceLock::new();

fn prop_surface_cache() -> &'static Mutex<std::collections::HashMap<String, Arc<Vec<Surface>>>> {
    PROP_SURFACE_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
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
pub fn get_or_load_rock_mesh(
    assets_root: &str,
    stage: u8,
    is_meteor: bool,
) -> Option<Arc<RenderMesh>> {
    let stage_name = if is_meteor {
        match stage {
            2 => "projectile_goliathmeteor2.cgf",
            3 => "projectile_goliathmeteor3.cgf",
            _ => "projectile_goliathmeteor1.cgf",
        }
    } else {
        match stage {
            2 => "projectile_stage2.cgf",
            3 => "projectile_stage3.cgf",
            _ => "projectile_stage1.cgf",
        }
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

fn mat4_identity() -> Mat4 {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropEffectKind {
    GoliathRock,
    GoliathFireBreath,
    GoliathLeapSmash,
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

    // 2. Goliath Fire Breath Plume
    if lower.contains("fire_breath") || lower.contains("flame_breath") {
        return Some(PropEffectKind::GoliathFireBreath);
    }

    // 3. Goliath Leap Attack Ground Smash
    if lower.contains("leap_attack_smash")
        || (lower.contains("leap_attack") && lower.contains("smash"))
    {
        return Some(PropEffectKind::GoliathLeapSmash);
    }

    // 4. Stage-Up Evolve Cocoons per Monster Family
    if lower.contains("cocoon") || lower.contains("stageup") || lower.contains("stage_up") {
        let family = if lower.contains("goliathmeteor") {
            "goliathmeteor"
        } else if lower.contains("goliath") || lower.contains("scorpid") {
            "goliath"
        } else if lower.contains("rockybobcicle") {
            "rockybobcicle"
        } else if lower.contains("rocky") || lower.contains("behemoth") {
            "rocky"
        } else if lower.contains("kraken") || lower.contains("flyer") {
            "kraken"
        } else if lower.contains("wraith") || lower.contains("teleporter") {
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

    // 5. Wraith Warp Blast & Supernova
    if lower.contains("warp_blast") || lower.contains("warpblast") || lower.contains("supernova") {
        return Some(PropEffectKind::WraithWarpBlast);
    }

    // 6. Glacial Behemoth (RockyBobcicle) Frost Spikes & Killing Frost
    if lower.contains("rending_spikes")
        || lower.contains("killing_frost")
        || lower.contains("ice_shard")
        || lower.contains("ice_spikes")
    {
        return Some(PropEffectKind::GlacialBehemothIceSpikes);
    }

    // 7. Standard Behemoth (Rocky / Bob) Lava Bomb, Rock Wall & Molten Fissure
    if lower.contains("lavaball")
        || lower.contains("lavabomb")
        || lower.contains("lava_bomb")
        || lower.contains("fissure")
        || lower.contains("rock_wall")
        || lower.contains("rockwall")
    {
        return Some(PropEffectKind::BehemothLavaBomb);
    }

    // 8. Hunter Deployable Equipment per Character
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

fn default_base_surface(index_count: u32) -> Surface {
    Surface {
        first_index: 0,
        index_count,
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
    }
}

fn new_empty_mesh() -> RenderMesh {
    RenderMesh {
        info: evolve_core::ModelInfo {
            format: "ProceduralVFX".to_string(),
            version: 1,
            chunk_count: 1,
            vertex_count: 0,
            triangle_count: 0,
            original_uv0: true,
            original_normals: true,
        },
        positions: Vec::new(),
        indices: Vec::new(),
        uv0: Some(Vec::new()),
        normals: Some(Vec::new()),
        vertex_alpha: None,
        vertex_color: None,
    }
}

fn append_billboard_quad(
    mesh: &mut RenderMesh,
    center: [f32; 3],
    size: [f32; 2],
    uv_min: [f32; 2],
    uv_max: [f32; 2],
) {
    let half_w = size[0] * 0.5;
    let half_h = size[1] * 0.5;
    let start_idx = mesh.positions.len() as u32;

    mesh.positions
        .push([center[0] - half_w, center[1], center[2] - half_h]);
    mesh.positions
        .push([center[0] + half_w, center[1], center[2] - half_h]);
    mesh.positions
        .push([center[0] + half_w, center[1], center[2] + half_h]);
    mesh.positions
        .push([center[0] - half_w, center[1], center[2] + half_h]);

    if let Some(normals) = &mut mesh.normals {
        for _ in 0..4 {
            normals.push([0.0, -1.0, 0.0]);
        }
    }
    if let Some(uvs) = &mut mesh.uv0 {
        uvs.push([uv_min[0], uv_max[1]]);
        uvs.push([uv_max[0], uv_max[1]]);
        uvs.push([uv_max[0], uv_min[1]]);
        uvs.push([uv_min[0], uv_min[1]]);
    }

    mesh.indices.extend_from_slice(&[
        start_idx,
        start_idx + 1,
        start_idx + 2,
        start_idx,
        start_idx + 2,
        start_idx + 3,
    ]);
}

fn append_crossed_quads(mesh: &mut RenderMesh, center: [f32; 3], size: [f32; 2]) {
    let half_w = size[0] * 0.5;
    let half_h = size[1] * 0.5;
    let start_idx = mesh.positions.len() as u32;

    // Plane 1: X-Z plane
    mesh.positions
        .push([center[0] - half_w, center[1], center[2] - half_h]);
    mesh.positions
        .push([center[0] + half_w, center[1], center[2] - half_h]);
    mesh.positions
        .push([center[0] + half_w, center[1], center[2] + half_h]);
    mesh.positions
        .push([center[0] - half_w, center[1], center[2] + half_h]);

    // Plane 2: Y-Z plane
    mesh.positions
        .push([center[0], center[1] - half_w, center[2] - half_h]);
    mesh.positions
        .push([center[0], center[1] + half_w, center[2] - half_h]);
    mesh.positions
        .push([center[0], center[1] + half_w, center[2] + half_h]);
    mesh.positions
        .push([center[0], center[1] - half_w, center[2] + half_h]);

    if let Some(normals) = &mut mesh.normals {
        for _ in 0..8 {
            normals.push([0.0, 0.0, 1.0]);
        }
    }
    if let Some(uvs) = &mut mesh.uv0 {
        for _ in 0..2 {
            uvs.push([0.0, 1.0]);
            uvs.push([1.0, 1.0]);
            uvs.push([1.0, 0.0]);
            uvs.push([0.0, 0.0]);
        }
    }

    mesh.indices.extend_from_slice(&[
        start_idx,
        start_idx + 1,
        start_idx + 2,
        start_idx,
        start_idx + 2,
        start_idx + 3,
        start_idx + 4,
        start_idx + 5,
        start_idx + 6,
        start_idx + 4,
        start_idx + 6,
        start_idx + 7,
    ]);
}

fn get_goliath_hand_midpoint(
    motion_time: f32,
    assembly: Option<&evolve_core::VerifiedCafAssembly>,
    skin: Option<&evolve_core::VerifiedCafSkin>,
    layers: &[evolve_core::CafLayer<'_>],
    weighted_clips: Option<&[evolve_core::WeightedCaf<'_>]>,
    in_place: bool,
) -> Option<Mat4> {
    let r_hand = get_bone_transform(
        "bone_goliath_R_hand",
        motion_time,
        assembly,
        skin,
        layers,
        weighted_clips,
        in_place,
    )?;
    let l_hand = get_bone_transform(
        "bone_goliath_L_hand",
        motion_time,
        assembly,
        skin,
        layers,
        weighted_clips,
        in_place,
    );
    if let Some(l) = l_hand {
        let mut mid = r_hand;
        mid[0][3] = (r_hand[0][3] + l[0][3]) * 0.5;
        mid[1][3] = (r_hand[1][3] + l[1][3]) * 0.5 + 0.15;
        mid[2][3] = (r_hand[2][3] + l[2][3]) * 0.5 + 0.05;
        Some(mid)
    } else {
        Some(r_hand)
    }
}

fn append_meteor_body_flames(
    mesh: &mut RenderMesh,
    motion_time: f32,
    assembly: Option<&evolve_core::VerifiedCafAssembly>,
    skin: Option<&evolve_core::VerifiedCafSkin>,
    layers: &[evolve_core::CafLayer<'_>],
    weighted_clips: Option<&[evolve_core::WeightedCaf<'_>]>,
    in_place: bool,
) {
    if mesh.normals.is_none() {
        mesh.normals = Some(vec![[0.0, 1.0, 0.0]; mesh.positions.len()]);
    }
    if mesh.uv0.is_none() {
        mesh.uv0 = Some(vec![[0.0, 0.0]; mesh.positions.len()]);
    }

    const FLAME_BONES: &[(&str, f32)] = &[
        ("bone_goliath_spine2", 0.7),
        ("bone_goliath_spine3", 0.8),
        ("bone_goliath_spine4", 0.9),
        ("bone_goliath_R_clavicle", 0.6),
        ("bone_goliath_L_clavicle", 0.6),
        ("bone_goliath_R_forearm", 0.5),
        ("bone_goliath_L_forearm", 0.5),
        ("bone_goliath_tail2", 0.6),
        ("bone_goliath_tail4", 0.5),
        ("bone_goliath_head", 0.5),
    ];

    for &(bone, scale) in FLAME_BONES {
        if let Some(trans) = get_bone_transform(
            bone,
            motion_time,
            assembly,
            skin,
            layers,
            weighted_clips,
            in_place,
        ) {
            let center = [trans[0][3], trans[1][3], trans[2][3] + scale * 0.15];
            let size = [scale * 0.8, scale * 1.2];
            append_crossed_quads(mesh, center, size);
        }
    }
}

pub struct PropSimulationResult {
    pub combined_mesh: RenderMesh,
    pub surfaces: Option<Arc<Vec<Surface>>>,
}

/// Evaluate animation props and projectiles for the current frame.
/// If an active prop is present, transforms and attaches it, merging geometry
/// while returning a cached stable Surface table pointer to eliminate GPU flickering.
pub fn simulate_frame_props(
    base_mesh: RenderMesh,
    base_surfaces: Option<&Arc<Vec<Surface>>>,
    clip_name: &str,
    character_path: &str,
    motion_time: f32,
    clip_duration: f32,
    assembly: Option<&evolve_core::VerifiedCafAssembly>,
    skin: Option<&evolve_core::VerifiedCafSkin>,
    layers: &[evolve_core::CafLayer<'_>],
    weighted_clips: Option<&[evolve_core::WeightedCaf<'_>]>,
    in_place: bool,
    assets_root: &str,
    monster_stage: u8,
) -> PropSimulationResult {
    let lower_clip = clip_name.to_ascii_lowercase();
    let is_meteor =
        character_path.to_ascii_lowercase().contains("meteor") || lower_clip.contains("meteor");

    let Some(prop_kind) = classify_animation_prop(clip_name) else {
        if is_meteor {
            let mut combined = base_mesh;
            let base_indices_count = combined.indices.len() as u32;
            append_meteor_body_flames(
                &mut combined,
                motion_time,
                assembly,
                skin,
                layers,
                weighted_clips,
                in_place,
            );
            let flame_indices_count = combined.indices.len() as u32 - base_indices_count;
            if flame_indices_count > 0 {
                let base_identity = base_surfaces.map_or(0, |s| Arc::as_ptr(s) as usize);
                let cache_key = format!(
                    "vfx_meteor_ambient:{base_indices_count}:{flame_indices_count}:{base_identity}"
                );
                let surfaces = {
                    let mut cache = prop_surface_cache().lock().unwrap();
                    if let Some(existing) = cache.get(&cache_key) {
                        existing.clone()
                    } else {
                        let mut list = base_surfaces
                            .map(|s| (**s).clone())
                            .unwrap_or_else(|| vec![default_base_surface(base_indices_count)]);
                        let flame_diff = get_or_load_prop_texture(
                            assets_root,
                            "objects/effects/abilities/Goliath_Meteor/goliathmeteor_fireball01_diff.dds.0",
                        )
                        .or_else(|| {
                            Some(generate_procedural_texture(
                                [40, 180, 255, 255],
                                [10, 80, 180, 255],
                                2,
                            ))
                        });
                        list.push(Surface {
                            first_index: base_indices_count,
                            index_count: flame_indices_count,
                            visible: true,
                            transparent: false,
                            foliage_cutout: false,
                            distance_clouds: None,
                            uv_transform: None,
                            material_tint: Some([0.25, 0.75, 1.0, 0.85]),
                            material_specular: Some([0.2, 0.5, 0.8]),
                            material_emissive: Some([0.15, 0.85, 1.0]),
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
                            alpha_cutoff: 0.1,
                            diffuse: flame_diff,
                            normal: None,
                            emissive: None,
                            armor_emissive: None,
                            emission_strength: 3.0,
                            emissive_uv: None,
                            specular: None,
                            gloss: None,
                            opacity: None,
                        });
                        let arc = Arc::new(list);
                        cache.insert(cache_key, arc.clone());
                        arc
                    }
                };
                return PropSimulationResult {
                    combined_mesh: combined,
                    surfaces: Some(surfaces),
                };
            }
            return PropSimulationResult {
                combined_mesh: combined,
                surfaces: base_surfaces.cloned(),
            };
        }
        return PropSimulationResult {
            combined_mesh: base_mesh,
            surfaces: base_surfaces.cloned(),
        };
    };

    let (prop_mesh, prop_matrix, prop_cache_key, tint, emissive, strength, diff_tex, glow_tex) =
        match prop_kind {
            PropEffectKind::GoliathRock => {
                let Some(mesh) = get_or_load_rock_mesh(assets_root, monster_stage, is_meteor)
                else {
                    return PropSimulationResult {
                        combined_mesh: base_mesh,
                        surfaces: base_surfaces.cloned(),
                    };
                };

                let throw_release_time = 0.6726 * clip_duration.max(0.1);

                let mat: Option<Mat4> = if lower_clip.contains("rock_pick_up") {
                    if motion_time < 0.35 {
                        None
                    } else {
                        get_goliath_hand_midpoint(
                            motion_time,
                            assembly,
                            skin,
                            layers,
                            weighted_clips,
                            in_place,
                        )
                        .map(|hand| {
                            let offset = mat4_translation([0.0, 0.25, 0.0]);
                            mat4_mul(&hand, &offset)
                        })
                    }
                } else if lower_clip.contains("rock_hold")
                    || lower_clip.contains("rock_turn")
                    || lower_clip.contains("rockthrow")
                {
                    get_goliath_hand_midpoint(
                        motion_time,
                        assembly,
                        skin,
                        layers,
                        weighted_clips,
                        in_place,
                    )
                    .map(|hand| {
                        let offset = mat4_translation([0.0, 0.25, 0.0]);
                        mat4_mul(&hand, &offset)
                    })
                } else if lower_clip.contains("rock_throw") {
                    if motion_time < throw_release_time {
                        get_goliath_hand_midpoint(
                            motion_time,
                            assembly,
                            skin,
                            layers,
                            weighted_clips,
                            in_place,
                        )
                        .map(|hand| {
                            let offset = mat4_translation([0.0, 0.25, 0.0]);
                            mat4_mul(&hand, &offset)
                        })
                    } else {
                        let release_hand = get_goliath_hand_midpoint(
                            throw_release_time,
                            assembly,
                            skin,
                            layers,
                            weighted_clips,
                            in_place,
                        );

                        let (p0_x, p0_y, p0_z) = if let Some(h) = release_hand {
                            (h[0][3], h[1][3] + 0.3, h[2][3])
                        } else {
                            (0.0, 1.5, 2.5)
                        };

                        let delta_t = motion_time - throw_release_time;
                        let v_forward = 28.0;
                        let v_up = 3.8;
                        let gravity = 9.81;

                        let x = p0_x;
                        let y = p0_y + v_forward * delta_t;
                        let z =
                            (p0_z + v_up * delta_t - 0.5 * gravity * delta_t * delta_t).max(0.0);

                        let tumble = delta_t * 9.0;
                        let trans = mat4_translation([x, y, z]);
                        let rot = mat4_rot_x(tumble);
                        Some(mat4_mul(&trans, &rot))
                    }
                } else if lower_clip.contains("rock_release") {
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

                let (diff, glow, tint, emissive_color, strength, key) = if is_meteor {
                    (
                        get_or_load_prop_texture(
                            assets_root,
                            "objects/effects/abilities/Goliath_Meteor/goliathmeteor_rock_smolder01_diff.dds.0",
                        )
                        .or_else(|| {
                            Some(generate_procedural_texture(
                                [45, 55, 65, 255],
                                [15, 20, 25, 255],
                                1,
                            ))
                        }),
                        get_or_load_prop_texture(
                            assets_root,
                            "objects/effects/abilities/Goliath_Meteor/goliathmeteor_rock_smolder01_glow.dds.0",
                        ),
                        [1.0, 1.0, 1.0, 1.0],
                        None,
                        2.8,
                        "rock_meteor",
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
                        [1.0, 1.0, 1.0, 1.0],
                        None,
                        2.4,
                        "rock_goliath",
                    )
                };

                (mesh, mat, key, tint, emissive_color, strength, diff, glow)
            }
            PropEffectKind::GoliathFireBreath => {
                let head_bone = get_bone_transform(
                    "bone_goliath_jaw",
                    motion_time,
                    assembly,
                    skin,
                    layers,
                    weighted_clips,
                    in_place,
                )
                .or_else(|| {
                    get_bone_transform(
                        "bone_goliath_head",
                        motion_time,
                        assembly,
                        skin,
                        layers,
                        weighted_clips,
                        in_place,
                    )
                })
                .unwrap_or_else(|| mat4_translation([0.0, 1.5, 2.0]));

                let mut flame_mesh = new_empty_mesh();
                let steps = 14;
                let (plume_tint, plume_emissive) = if is_meteor {
                    ([0.25, 0.75, 1.0, 0.9], [0.15, 0.85, 1.0])
                } else {
                    ([1.0, 0.50, 0.1, 0.9], [1.0, 0.40, 0.05])
                };

                for i in 1..=steps {
                    let frac = i as f32 / steps as f32;
                    let dist = frac * 5.5;
                    let radius = 0.25 + frac * 1.8;
                    let jitter_x = ((motion_time * 24.0 + i as f32 * 1.7).sin()) * 0.15 * frac;
                    let jitter_z = ((motion_time * 18.0 + i as f32 * 2.3).cos()) * 0.15 * frac;

                    let center = [jitter_x, dist, jitter_z];
                    let size = [radius * 2.0, radius * 2.0];
                    append_crossed_quads(&mut flame_mesh, center, size);
                }

                let diff = get_or_load_prop_texture(
                    assets_root,
                    if is_meteor {
                        "objects/effects/abilities/Goliath_Meteor/goliathmeteor_fireball01_diff.dds.0"
                    } else {
                        "objects/effects/abilities/goliath/fireball/fireball01_diff.dds.0"
                    },
                )
                .or_else(|| {
                    Some(generate_procedural_texture(
                        if is_meteor { [40, 180, 255, 255] } else { [255, 120, 20, 255] },
                        if is_meteor { [10, 80, 180, 255] } else { [180, 40, 5, 255] },
                        2,
                    ))
                });

                (
                    Arc::new(flame_mesh),
                    Some(head_bone),
                    if is_meteor {
                        "vfx_meteor_fire_breath"
                    } else {
                        "vfx_fire_breath"
                    },
                    plume_tint,
                    Some(plume_emissive),
                    3.2,
                    diff,
                    None,
                )
            }
            PropEffectKind::GoliathLeapSmash => {
                let mut smash_mesh = new_empty_mesh();
                let impact_center = get_goliath_hand_midpoint(
                    motion_time,
                    assembly,
                    skin,
                    layers,
                    weighted_clips,
                    in_place,
                )
                .map(|h| [h[0][3], h[1][3], 0.05])
                .unwrap_or([0.0, 1.5, 0.05]);

                let (smash_tint, smash_emissive) = if is_meteor {
                    ([0.25, 0.75, 1.0, 0.85], [0.15, 0.85, 1.0])
                } else {
                    ([1.0, 0.45, 0.08, 0.85], [1.0, 0.35, 0.02])
                };

                let ring_segments = 16;
                let ring_radius = (motion_time * 8.0).min(4.5).max(0.5);
                for i in 0..ring_segments {
                    let angle = (i as f32 / ring_segments as f32) * std::f32::consts::TAU;
                    let x = impact_center[0] + angle.cos() * ring_radius;
                    let y = impact_center[1] + angle.sin() * ring_radius;
                    let z = impact_center[2];
                    append_billboard_quad(
                        &mut smash_mesh,
                        [x, y, z + 0.3],
                        [1.4, 1.4],
                        [0.0, 0.0],
                        [1.0, 1.0],
                    );
                }

                if motion_time >= 0.12 {
                    for i in 0..6 {
                        let offset_x = ((i as f32 * 1.5).sin()) * 0.8;
                        let offset_y = ((i as f32 * 1.5).cos()) * 0.8;
                        let height = 0.5 + (i as f32 * 0.3);
                        append_crossed_quads(
                            &mut smash_mesh,
                            [
                                impact_center[0] + offset_x,
                                impact_center[1] + offset_y,
                                impact_center[2] + height,
                            ],
                            [1.2, 1.8],
                        );
                    }
                }

                let diff = get_or_load_prop_texture(
                    assets_root,
                    if is_meteor {
                        "objects/effects/abilities/Goliath_Meteor/goliathmeteor_fireball01_diff.dds.0"
                    } else {
                        "objects/effects/abilities/goliath/rock_throw/rock_throw_glow01.dds.0"
                    },
                )
                .or_else(|| {
                    Some(generate_procedural_texture(
                        if is_meteor { [40, 180, 255, 255] } else { [255, 120, 20, 255] },
                        if is_meteor { [10, 80, 180, 255] } else { [180, 40, 5, 255] },
                        2,
                    ))
                });

                (
                    Arc::new(smash_mesh),
                    Some(mat4_identity()),
                    if is_meteor {
                        "vfx_meteor_leap_smash"
                    } else {
                        "vfx_leap_smash"
                    },
                    smash_tint,
                    Some(smash_emissive),
                    3.5,
                    diff,
                    None,
                )
            }
            PropEffectKind::Cocoon { monster_family } => {
                let (rel_path, tint, emissive, strength, scale_factor, key) = match monster_family {
                    "goliathmeteor" => (
                        "objects/effects/abilities/shared/stageup_coccoon_goliath01.cgf",
                        [0.3, 0.5, 0.8, 1.0],
                        [0.10, 0.75, 1.0],
                        2.5,
                        match monster_stage {
                            2 => 1.12,
                            3 => 1.25,
                            _ => 1.00,
                        },
                        "cocoon_goliathmeteor",
                    ),
                    "goliath" => {
                        let (path, emissive_col, mult) = if monster_stage >= 3 {
                            (
                                "objects/effects/abilities/shared/stageup_coccoon_goliath01_in.cgf",
                                [1.0, 0.22, 0.02],
                                1.25,
                            )
                        } else {
                            (
                                "objects/effects/abilities/shared/stageup_coccoon_goliath01.cgf",
                                [1.0, 0.40, 0.05],
                                if monster_stage == 2 { 1.12 } else { 1.00 },
                            )
                        };
                        (
                            path,
                            [0.6, 0.35, 0.25, 1.0],
                            emissive_col,
                            2.4,
                            mult,
                            "cocoon_goliath",
                        )
                    }
                    "rockybobcicle" => (
                        "objects/effects/abilities/Rocky/stageup/stageup_coccoon_rocky_in.cgf",
                        [0.8, 0.95, 1.0, 1.0],
                        [0.15, 0.85, 1.0],
                        2.6,
                        match monster_stage {
                            2 => 1.12,
                            3 => 1.25,
                            _ => 1.00,
                        },
                        "cocoon_rockybobcicle",
                    ),
                    "rocky" => (
                        "objects/effects/abilities/Rocky/stageup/stageup_coccoon_rocky_in.cgf",
                        [0.45, 0.25, 0.15, 1.0],
                        [1.0, 0.30, 0.05],
                        2.6,
                        match monster_stage {
                            2 => 1.12,
                            3 => 1.25,
                            _ => 1.00,
                        },
                        "cocoon_rocky",
                    ),
                    "kraken" => (
                        "objects/effects/abilities/Elder_Kraken/stageup/elderkraken_stageup_fin.cgf",
                        [0.3, 0.5, 0.6, 1.0],
                        [0.20, 0.65, 1.0],
                        2.4,
                        match monster_stage {
                            2 => 1.12,
                            3 => 1.25,
                            _ => 1.00,
                        },
                        "cocoon_kraken",
                    ),
                    "wraith" => (
                        "Objects/characters/monsters/cocoon/Teleporter/teleporter.cgf",
                        [0.4, 0.15, 0.5, 1.0],
                        [0.60, 0.10, 0.95],
                        2.5,
                        match monster_stage {
                            2 => 1.12,
                            3 => 1.25,
                            _ => 1.00,
                        },
                        "cocoon_wraith",
                    ),
                    "gorgon" => (
                        "Objects/characters/monsters/cocoon/ParasiteHost/parasitehost.cgf",
                        [0.3, 0.6, 0.15, 1.0],
                        [0.35, 0.90, 0.10],
                        2.3,
                        match monster_stage {
                            2 => 1.12,
                            3 => 1.25,
                            _ => 1.00,
                        },
                        "cocoon_gorgon",
                    ),
                    _ => (
                        "Objects/characters/monsters/cocoon/cocoon01.cgf",
                        [0.5, 0.5, 0.4, 1.0],
                        [0.85, 0.60, 0.10],
                        2.0,
                        match monster_stage {
                            2 => 1.12,
                            3 => 1.25,
                            _ => 1.00,
                        },
                        "cocoon_shared",
                    ),
                };

                let Some(mesh) = get_or_load_prop_mesh_with_pivot(
                    assets_root,
                    rel_path,
                    PropPivotMode::GroundBase,
                ) else {
                    return PropSimulationResult {
                        combined_mesh: base_mesh,
                        surfaces: base_surfaces.cloned(),
                    };
                };

                let matrix = Some(mat4_scale([scale_factor, scale_factor, scale_factor]));
                let diff = get_or_load_prop_texture(
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

                (
                    mesh,
                    matrix,
                    key,
                    tint,
                    Some(emissive),
                    strength,
                    diff,
                    None,
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

                (
                    mesh,
                    mat,
                    "warp_blast",
                    [0.3, 0.15, 0.45, 1.0],
                    Some([0.65, 0.15, 0.95]),
                    2.6,
                    Some(generate_procedural_texture(
                        [150, 50, 200, 255],
                        [70, 10, 120, 255],
                        2,
                    )),
                    None,
                )
            }
            PropEffectKind::GlacialBehemothIceSpikes => {
                let Some(mesh) = get_or_load_prop_mesh_with_pivot(
                    assets_root,
                    "objects/effects/abilities/RockyBobcicle/rending_spikes/rending_spikes_ice02.cgf",
                    PropPivotMode::GroundBase,
                ) else {
                    return PropSimulationResult {
                        combined_mesh: base_mesh,
                        surfaces: base_surfaces.cloned(),
                    };
                };
                (
                    mesh,
                    Some(mat4_translation([0.0, 3.2, 0.0])),
                    "ice_spikes",
                    [0.7, 0.9, 1.0, 1.0],
                    Some([0.15, 0.85, 1.0]),
                    2.4,
                    Some(generate_procedural_texture(
                        [180, 220, 255, 255],
                        [120, 170, 220, 255],
                        1,
                    )),
                    None,
                )
            }
            PropEffectKind::BehemothLavaBomb => {
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
                    "lava_bomb",
                    [0.55, 0.25, 0.15, 1.0],
                    Some([1.0, 0.30, 0.02]),
                    2.8,
                    Some(generate_procedural_texture(
                        [120, 40, 20, 255],
                        [40, 15, 5, 255],
                        1,
                    )),
                    None,
                )
            }
            PropEffectKind::HunterDeployable { hunter } => {
                let effective_hunter = if hunter == "generic" {
                    let char_lower = character_path.to_ascii_lowercase();
                    if char_lower.contains("markov") {
                        "markov"
                    } else if char_lower.contains("maggie") {
                        "maggie"
                    } else if char_lower.contains("bucket") {
                        "bucket"
                    } else if char_lower.contains("griffin") {
                        "griffin"
                    } else if char_lower.contains("hank") {
                        "hank"
                    } else if char_lower.contains("abe") {
                        "abe"
                    } else if char_lower.contains("hyde") {
                        "hyde"
                    } else if char_lower.contains("caira") {
                        "caira"
                    } else {
                        "generic"
                    }
                } else {
                    hunter
                };
                let (rel_path, tint, emissive, strength, pivot, key) = match effective_hunter {
                    "markov" => (
                        "characters/hunters/shared/weapons/projectiles/grenade_brass.cgf",
                        [0.8, 0.7, 0.3, 1.0],
                        [0.15, 0.65, 1.0],
                        2.2,
                        PropPivotMode::Centered,
                        "deploy_markov",
                    ),
                    "maggie" => (
                        "objects/weapons/Objects/weapons/harpoon_mine/harpoon_mine_projectile_tp.cgf",
                        [0.4, 0.4, 0.4, 1.0],
                        [1.0, 0.35, 0.0],
                        2.0,
                        PropPivotMode::GroundBase,
                        "deploy_maggie",
                    ),
                    "abe" => (
                        "characters/hunters/shared/weapons/projectiles/grenade_blue.cgf",
                        [0.2, 0.3, 0.7, 1.0],
                        [0.50, 0.15, 1.0],
                        2.2,
                        PropPivotMode::Centered,
                        "deploy_abe",
                    ),
                    "griffin" => (
                        "objects/weapons/Objects/weapons/Sound_Triangulator_fp/Sound_Triangulator_tp.cgf",
                        [0.3, 0.45, 0.3, 1.0],
                        [0.10, 1.0, 0.35],
                        2.2,
                        PropPivotMode::GroundBase,
                        "deploy_griffin",
                    ),
                    "bucket" => (
                        "objects/effects/items/sentry/sentrygun_3p.cgf",
                        [0.8, 0.75, 0.3, 1.0],
                        [1.0, 0.85, 0.10],
                        2.0,
                        PropPivotMode::GroundBase,
                        "deploy_bucket",
                    ),
                    "hank" => (
                        "objects/weapons/Objects/weapons/beacon_tp/beacon_tp.cgf",
                        [0.4, 0.4, 0.4, 1.0],
                        [1.0, 0.05, 0.05],
                        2.5,
                        PropPivotMode::GroundBase,
                        "deploy_hank",
                    ),
                    "hyde" => (
                        "characters/hunters/shared/weapons/projectiles/grenade_projectile.cgf",
                        [0.3, 0.5, 0.2, 1.0],
                        [0.25, 0.95, 0.05],
                        2.0,
                        PropPivotMode::Centered,
                        "deploy_hyde",
                    ),
                    "caira" => (
                        "characters/hunters/shared/weapons/projectiles/grenade_blue.cgf",
                        [0.2, 0.6, 0.6, 1.0],
                        [0.10, 0.90, 0.85],
                        2.0,
                        PropPivotMode::Centered,
                        "deploy_caira",
                    ),
                    _ => (
                        "characters/hunters/shared/weapons/projectiles/grenade_projectile.cgf",
                        [0.4, 0.4, 0.4, 1.0],
                        [1.0, 0.60, 0.05],
                        1.8,
                        PropPivotMode::Centered,
                        "deploy_generic",
                    ),
                };

                let Some(mesh) = get_or_load_prop_mesh_with_pivot(assets_root, rel_path, pivot)
                else {
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

                let diff = if effective_hunter == "maggie" {
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

                (mesh, mat, key, tint, Some(emissive), strength, diff, None)
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
    let prop_indices_count = prop_mesh.indices.len() as u32;
    append_render_mesh(&mut combined, &transformed_prop);

    // If meteor monster, also append ambient body flames
    let mut ambient_flame_indices = 0u32;
    if is_meteor {
        let pre_flame_count = combined.indices.len() as u32;
        append_meteor_body_flames(
            &mut combined,
            motion_time,
            assembly,
            skin,
            layers,
            weighted_clips,
            in_place,
        );
        ambient_flame_indices = combined.indices.len() as u32 - pre_flame_count;
    }

    // Retrieve or create a stable Arc<Vec<Surface>> pointer so textured.rs never detects
    // a spurious identity change from frame to frame, completely preventing GPU flickering.
    let base_identity = base_surfaces.map_or(0, |s| Arc::as_ptr(s) as usize);
    let cache_key = format!(
        "{prop_cache_key}:{base_indices_count}:{prop_indices_count}:{ambient_flame_indices}:{base_identity}"
    );

    let surfaces = {
        let mut cache = prop_surface_cache().lock().unwrap();
        if let Some(existing) = cache.get(&cache_key) {
            existing.clone()
        } else {
            let mut list = base_surfaces
                .map(|s| (**s).clone())
                .unwrap_or_else(|| vec![default_base_surface(base_indices_count)]);

            let prop_surface = Surface {
                first_index: base_indices_count,
                index_count: prop_indices_count,
                visible: true,
                transparent: false,
                foliage_cutout: false,
                distance_clouds: None,
                uv_transform: None,
                material_tint: Some(tint),
                material_specular: Some([0.25, 0.22, 0.20]),
                material_emissive: emissive,
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
                diffuse: diff_tex,
                normal: None,
                emissive: glow_tex,
                armor_emissive: None,
                emission_strength: strength,
                emissive_uv: None,
                specular: None,
                gloss: None,
                opacity: None,
            };
            list.push(prop_surface);

            if ambient_flame_indices > 0 {
                let flame_diff = get_or_load_prop_texture(
                    assets_root,
                    "objects/effects/abilities/Goliath_Meteor/goliathmeteor_fireball01_diff.dds.0",
                )
                .or_else(|| {
                    Some(generate_procedural_texture(
                        [40, 180, 255, 255],
                        [10, 80, 180, 255],
                        2,
                    ))
                });
                let flame_surface = Surface {
                    first_index: base_indices_count + prop_indices_count,
                    index_count: ambient_flame_indices,
                    visible: true,
                    transparent: false,
                    foliage_cutout: false,
                    distance_clouds: None,
                    uv_transform: None,
                    material_tint: Some([0.25, 0.75, 1.0, 0.85]),
                    material_specular: Some([0.2, 0.5, 0.8]),
                    material_emissive: Some([0.15, 0.85, 1.0]),
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
                    alpha_cutoff: 0.1,
                    diffuse: flame_diff,
                    normal: None,
                    emissive: None,
                    armor_emissive: None,
                    emission_strength: 3.0,
                    emissive_uv: None,
                    specular: None,
                    gloss: None,
                    opacity: None,
                };
                list.push(flame_surface);
            }

            let arc = Arc::new(list);
            cache.insert(cache_key, arc.clone());
            arc
        }
    };

    PropSimulationResult {
        combined_mesh: combined,
        surfaces: Some(surfaces),
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
        assert_eq!(classify_animation_prop("Rocky_AimPoses_idle"), None);
        assert_eq!(classify_animation_prop("rockybobcicle_store_pose"), None);
        assert_eq!(classify_animation_prop("ability_charge_attack"), None);
        assert_eq!(
            classify_animation_prop("ability_fire_breath"),
            Some(PropEffectKind::GoliathFireBreath)
        );
        assert_eq!(
            classify_animation_prop("locomotion_leap_attack_smash"),
            Some(PropEffectKind::GoliathLeapSmash)
        );
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
        assert!(!is_rock_throw_animation("ability_fire_breath"));
        assert!(!is_rock_throw_animation("ability_charge_attack"));
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
            "characters/monsters/goliath/goliath1.cdf",
            1.0,
            4.0,
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
