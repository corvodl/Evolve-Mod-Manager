//! Verified CPU skin deformation for the existing wgpu character viewport.
//! Renders exact compiled source weights and non-additive CAF keys, never
//! guesses missing bones. CPU deformation is a correctness stepping stone;
//! a dedicated GPU joint palette remains a separate performance milestone.
use crate::caf_layers::{CafLayer, sample_caf_layers_with_bind};
use crate::model::{RenderMesh, read_model};
use crate::native_caf::CafBonePose;
use crate::native_caf::NativeCafMotion;
use crate::rig::{
    CompiledSkeleton, RigError, VerifiedWeights, parse_compiled_skeleton, remap_verified_weights,
    rigged_glb_bytes_with_caf,
};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

type Mat4 = [[f32; 4]; 4];

fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..4).map(|k| a[r][k] * b[k][c]).sum()))
}
fn column_major(m: &Mat4) -> [f32; 16] {
    std::array::from_fn(|i| m[i % 4][i / 4])
}
fn rigid_components(m: &Mat4) -> Result<([f32; 3], [f32; 4]), RigError> {
    crate::rig::rigid_trs(&json!(column_major(m)))
}
fn transform_point(matrix: &Mat4, point: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| {
        (0..3).map(|c| matrix[axis][c] * point[c]).sum::<f32>() + matrix[axis][3]
    })
}
fn transform_normal(matrix: &Mat4, normal: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| (0..3).map(|c| matrix[axis][c] * normal[c]).sum::<f32>())
}
fn trs(translation: [f32; 3], q: [f32; 4]) -> Result<Mat4, RigError> {
    if translation.iter().any(|v| !v.is_finite() || v.abs() > 1e7) {
        return Err(RigError::Invalid(
            "Non-finite/unbounded CAF translation".into(),
        ));
    }
    let norm = q.iter().map(|v| v * v).sum::<f32>();
    if !norm.is_finite() || (norm - 1.0).abs() > 0.1 {
        return Err(RigError::Invalid("Invalid native CAF quaternion".into()));
    }
    let inv = norm.sqrt().recip();
    let [x, y, z, w] = q.map(|v| v * inv);
    Ok([
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
            translation[0],
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
            translation[1],
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
            translation[2],
        ],
        [0.0, 0.0, 0.0, 1.0],
    ])
}
fn deform(
    mesh: &RenderMesh,
    weights: &VerifiedWeights,
    palette: &[Mat4],
) -> Result<RenderMesh, RigError> {
    if mesh.positions.len() != weights.joint_ids.len()
        || mesh.positions.len() != weights.weight_bytes.len()
        || mesh
            .normals
            .as_ref()
            .is_some_and(|normals| normals.len() != mesh.positions.len())
        || !(weights.influences == 4 || weights.influences == 8)
    {
        return Err(RigError::Invalid(
            "Source mesh/skin influence mismatch".into(),
        ));
    }
    let mut output = mesh.clone();
    for vertex in 0..mesh.positions.len() {
        let ids = &weights.joint_ids[vertex];
        let amounts = &weights.weight_bytes[vertex];
        if amounts.iter().map(|v| u32::from(*v)).sum::<u32>() != 255 {
            return Err(RigError::Invalid("Source weights do not total 255".into()));
        }
        let mut position = [0.0f32; 3];
        let mut normal = [0.0f32; 3];
        for lane in 0..weights.influences {
            let weight = f32::from(amounts[lane]) / 255.0;
            if weight <= 0.0 {
                continue;
            }
            let matrix = palette.get(usize::from(ids[lane])).ok_or_else(|| {
                RigError::Invalid("CAF weight references absent skeleton bone".into())
            })?;
            let shifted = transform_point(matrix, mesh.positions[vertex]);
            for axis in 0..3 {
                position[axis] += shifted[axis] * weight;
            }
            if let Some(normals) = &mesh.normals {
                let rotated = transform_normal(matrix, normals[vertex]);
                for axis in 0..3 {
                    normal[axis] += rotated[axis] * weight;
                }
            }
        }
        if position.iter().any(|v| !v.is_finite() || v.abs() > 1e8) {
            return Err(RigError::Invalid("Animated vertex is not finite".into()));
        }
        output.positions[vertex] = position;
        if let Some(ref mut normals) = output.normals {
            let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt();
            if !length.is_finite() || length < 1e-6 {
                return Err(RigError::Invalid("Animated normal is degenerate".into()));
            }
            normals[vertex] = normal.map(|v| v / length);
        }
    }
    Ok(output)
}

/// Keep all original vertex streams and bone tables in memory so frame
/// scrubbing does not re-read a game PAK or rebuild an entire GLB.
#[derive(Debug, Clone)]
pub struct VerifiedCafSkin {
    mesh: RenderMesh,
    skeleton: CompiledSkeleton,
    weights: VerifiedWeights,
    bind_local: Vec<Mat4>,
    bind_components: Vec<([f32; 3], [f32; 4])>,
    crc_to_bone: HashMap<u32, usize>,
    motion: NativeCafMotion,
}

impl VerifiedCafSkin {
    pub fn from_originals(
        mesh_path: &Path,
        skeleton_path: &Path,
        motion: NativeCafMotion,
    ) -> Result<Self, RigError> {
        for file in [mesh_path, skeleton_path] {
            let stat = fs::symlink_metadata(file)?;
            if !stat.file_type().is_file() || stat.len() == 0 || stat.len() > 256 * 1024 * 1024 {
                return Err(RigError::Limits);
            }
        }
        let mesh = read_model(mesh_path)?;
        let mesh_raw = fs::read(mesh_path)?;
        let skeleton_raw = fs::read(skeleton_path)?;
        Self::from_bytes(mesh, &mesh_raw, &skeleton_raw, motion)
    }

    pub fn from_bytes(
        mesh: RenderMesh,
        mesh_raw: &[u8],
        skeleton_raw: &[u8],
        motion: NativeCafMotion,
    ) -> Result<Self, RigError> {
        // Reuse the same exact CRC, bind-matrix and controller validation as
        // verified FBX export. Do not animate a mesh that Blender could not
        // bind without guessing its compiled skeleton.
        rigged_glb_bytes_with_caf(&mesh, mesh_raw, skeleton_raw, &motion)?;
        let skeleton = parse_compiled_skeleton(skeleton_raw)?;
        let weights = remap_verified_weights(mesh_raw, mesh.positions.len(), skeleton.names.len())?;
        let mut bind_local = Vec::with_capacity(skeleton.names.len());
        let mut bind_components = Vec::with_capacity(skeleton.names.len());
        let mut crc_to_bone = HashMap::new();
        for i in 0..skeleton.names.len() {
            let local = if let Some(parent) = skeleton.parents[i] {
                multiply(&skeleton.inverse[parent], &skeleton.world[i])
            } else {
                skeleton.world[i]
            };
            // CPU skinning currently supports rigid source rest transforms.
            // Unsupported mirrored/scaled bones fail explicitly.
            bind_components.push(rigid_components(&local)?);
            bind_local.push(local);
            let crc = skeleton.controller_ids[i];
            if crc_to_bone.insert(crc, i).is_some() {
                return Err(RigError::Invalid(
                    "Original compiled bone CRC collision".into(),
                ));
            }
        }
        Ok(Self {
            mesh,
            skeleton,
            weights,
            bind_local,
            bind_components,
            crc_to_bone,
            motion,
        })
    }

    fn posed_world(&self, seconds: f32, layers: &[CafLayer<'_>]) -> Result<Vec<Mat4>, RigError> {
        let bind = self
            .skeleton
            .controller_ids
            .iter()
            .zip(&self.bind_components)
            .map(|(&controller_id, (position, rotation))| CafBonePose {
                bone_crc32: controller_id,
                translation: Some(*position),
                rotation: Some(*rotation),
            })
            .collect::<Vec<_>>();
        let poses = sample_caf_layers_with_bind(&self.motion, seconds, layers, &bind)
            .map_err(|e| RigError::Invalid(format!("Native CAF sample: {e}")))?;
        let mut locals = self.bind_local.clone();
        for pose in poses {
            let index = *self
                .crc_to_bone
                .get(&pose.bone_crc32)
                .ok_or_else(|| RigError::Invalid("CAF references an unmatched bone".into()))?;
            let (bind_position, bind_rotation) = self.bind_components[index];
            let translation = pose.translation.unwrap_or(bind_position);
            let rotation = pose.rotation.unwrap_or(bind_rotation);
            locals[index] = trs(translation, rotation)?;
        }
        let mut world = Vec::with_capacity(locals.len());
        for (i, local) in locals.iter().enumerate() {
            let matrix = self.skeleton.parents[i]
                .map(|parent| multiply(&world[parent], local))
                .unwrap_or(*local);
            world.push(matrix);
        }
        Ok(world)
    }

    pub fn mesh_at(&self, seconds: f32) -> Result<RenderMesh, RigError> {
        self.mesh_at_layers(seconds, &[])
    }
    pub fn mesh_at_layers(
        &self,
        seconds: f32,
        layers: &[CafLayer<'_>],
    ) -> Result<RenderMesh, RigError> {
        let world = self.posed_world(seconds, layers)?;
        let skin = world
            .iter()
            .zip(&self.skeleton.inverse)
            .map(|(pose, inverse)| multiply(pose, inverse))
            .collect::<Vec<_>>();
        deform(&self.mesh, &self.weights, &skin)
    }
    pub fn original_vertices(&self) -> usize {
        self.mesh.positions.len()
    }
    pub fn original_bones(&self) -> usize {
        self.skeleton.names.len()
    }
    pub fn source_motion(&self) -> &NativeCafMotion {
        &self.motion
    }
}

/// One verified CAF drives every original skinned part in the same CDF.
/// Bone-mounted rigid objects use the authored bind socket from the original
/// combined mesh; at each frame only the source bone delta is applied.
/// Unverified skins are refused, never left frozen or assigned fake weights.
#[derive(Debug, Clone)]
pub struct VerifiedCafAssembly {
    rest: RenderMesh,
    skins: Vec<(usize, VerifiedCafSkin)>,
    sockets: Vec<(usize, usize, usize)>, // first vertex, count, exact original bone index
}

fn move_equipment(
    output: &mut RenderMesh,
    rest: &RenderMesh,
    first: usize,
    count: usize,
    delta: &Mat4,
) -> Result<(), RigError> {
    let end = first.checked_add(count).ok_or(RigError::Limits)?;
    if end > output.positions.len() || end > rest.positions.len() {
        return Err(RigError::Limits);
    }
    for vertex in first..end {
        let position = transform_point(delta, rest.positions[vertex]);
        if position.iter().any(|v| !v.is_finite() || v.abs() > 1e8) {
            return Err(RigError::Invalid(
                "Non-finite mounted equipment pose".into(),
            ));
        }
        output.positions[vertex] = position;
        if let (Some(source), Some(target)) = (&rest.normals, &mut output.normals) {
            let normal = transform_normal(delta, source[vertex]);
            let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt();
            if !length.is_finite() || length <= 1e-6 {
                return Err(RigError::Invalid(
                    "Invalid animated equipment normal".into(),
                ));
            }
            target[vertex] = normal.map(|v| v / length);
        }
    }
    Ok(())
}

impl VerifiedCafAssembly {
    pub fn from_parts(
        rest: RenderMesh,
        report: &crate::character_assembly::AssemblyReport,
        motion: NativeCafMotion,
    ) -> Result<Self, RigError> {
        let skeleton_source = report.skeleton_source.as_ref().ok_or_else(|| {
            RigError::Invalid("CDF has no verified original compiled skeleton".into())
        })?;
        let original_skeleton = fs::read(skeleton_source)?;
        let skeleton = parse_compiled_skeleton(&original_skeleton)?;
        if rest.positions.len() != report.vertices || report.geometry_parts.is_empty() {
            return Err(RigError::Invalid(
                "CDF geometry layout is incomplete".into(),
            ));
        }
        let mut skins = Vec::new();
        let mut sockets = Vec::new();
        let mut cursor = 0usize;
        for part in &report.geometry_parts {
            if part.vertex_first != cursor || part.vertex_count == 0 {
                return Err(RigError::Invalid(
                    "CDF vertex spans do not match source order".into(),
                ));
            }
            cursor = cursor
                .checked_add(part.vertex_count)
                .ok_or(RigError::Limits)?;
            if cursor > rest.positions.len() {
                return Err(RigError::Limits);
            }
            if let Some(socket) = &part.bone_socket {
                let matches = skeleton
                    .names
                    .iter()
                    .enumerate()
                    .filter(|(_, name)| name.eq_ignore_ascii_case(socket))
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                if matches.len() != 1 {
                    return Err(RigError::Invalid(format!(
                        "CDF equipment {} has no unique authored bone socket: {}",
                        part.name, socket
                    )));
                }
                sockets.push((part.vertex_first, part.vertex_count, matches[0]));
            } else {
                if !part.verified_skin_weights {
                    return Err(RigError::Invalid(format!(
                        "CDF part {} has no verified original skin weights",
                        part.name
                    )));
                }
                let skin =
                    VerifiedCafSkin::from_originals(&part.source, skeleton_source, motion.clone())?;
                if skin.original_vertices() != part.vertex_count
                    || skin.mesh.positions != rest.positions[part.vertex_first..cursor]
                {
                    return Err(RigError::Invalid(format!(
                        "CDF part {} differs from the source rest mesh",
                        part.name
                    )));
                }
                skins.push((part.vertex_first, skin));
            }
        }
        if cursor != rest.positions.len() || skins.is_empty() {
            return Err(RigError::Invalid(
                "CDF has unmatched geometry or no bindable original skin".into(),
            ));
        }
        Ok(Self {
            rest,
            skins,
            sockets,
        })
    }

    pub fn mesh_at(&self, seconds: f32) -> Result<RenderMesh, RigError> {
        self.mesh_at_layers(seconds, &[])
    }
    pub fn mesh_at_layers(
        &self,
        seconds: f32,
        layers: &[CafLayer<'_>],
    ) -> Result<RenderMesh, RigError> {
        let mut output = self.rest.clone();
        let primary = &self.skins[0].1;
        let world = primary.posed_world(seconds, layers)?;
        let palette = world
            .iter()
            .zip(&primary.skeleton.inverse)
            .map(|(pose, inverse)| multiply(pose, inverse))
            .collect::<Vec<_>>();
        for (offset, skin) in &self.skins {
            let animated = deform(&skin.mesh, &skin.weights, &palette)?;
            let end = offset
                .checked_add(animated.positions.len())
                .ok_or(RigError::Limits)?;
            if end > output.positions.len() {
                return Err(RigError::Limits);
            }
            output.positions[*offset..end].copy_from_slice(&animated.positions);
            if let (Some(target), Some(source)) = (&mut output.normals, animated.normals) {
                target[*offset..end].copy_from_slice(&source);
            }
        }
        for &(first, count, bone) in &self.sockets {
            let delta = multiply(&world[bone], &primary.skeleton.inverse[bone]);
            move_equipment(&mut output, &self.rest, first, count, &delta)?;
        }
        Ok(output)
    }

    pub fn part_count(&self) -> usize {
        self.skins.len() + self.sockets.len()
    }
    pub fn original_bones(&self) -> usize {
        self.skins[0].1.original_bones()
    }
    pub fn source_motion(&self) -> &NativeCafMotion {
        self.skins[0].1.source_motion()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelInfo;

    fn clip(name: &str, x: f32) -> NativeCafMotion {
        NativeCafMotion {
            schema: 1,
            start_frame: 0.0,
            end_frame: 30.0,
            fps: 30.0,
            additive: false,
            cyclic: true,
            controllers: vec![crate::native_caf::CafController {
                bone_crc32: crc32fast::hash(name.as_bytes()),
                position_times: vec![0.0, 30.0],
                positions: vec![[x, 0.0, 0.0]; 2],
                rotation_times: vec![],
                rotations: vec![],
            }],
        }
    }

    fn two_bone_skin() -> VerifiedCafSkin {
        let identity = trs([0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
        let child = trs([2.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]).unwrap();
        let inverse_child = trs([-2.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]).unwrap();
        let mut weights = weights();
        for ids in &mut weights.joint_ids {
            ids[0] = 1;
        }
        VerifiedCafSkin {
            mesh: mesh(),
            skeleton: CompiledSkeleton {
                controller_ids: vec![crc32fast::hash(b"root"), crc32fast::hash(b"hand")],
                names: vec!["root".into(), "hand".into()],
                parents: vec![None, Some(0)],
                world: vec![identity, child],
                inverse: vec![identity, inverse_child],
            },
            weights,
            bind_local: vec![identity, child],
            bind_components: vec![
                ([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
                ([2.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]),
            ],
            crc_to_bone: HashMap::from([
                (crc32fast::hash(b"root"), 0),
                (crc32fast::hash(b"hand"), 1),
            ]),
            motion: clip("root", 4.0),
        }
    }

    #[test]
    fn sparse_layer_uses_compiled_bind_channels_and_moves_every_skin_and_socket() {
        let skin = two_bone_skin();
        let (rest, _) = crate::character_assembly::combine_source_meshes(vec![
            ("a".into(), mesh()),
            ("b".into(), mesh()),
            ("prop".into(), mesh()),
        ])
        .unwrap();
        let assembly = VerifiedCafAssembly {
            rest: rest.clone(),
            skins: vec![(0, skin.clone()), (3, skin)],
            sockets: vec![(6, 3, 1)],
        };
        let overlay = clip("hand", 6.0);
        let layers = [CafLayer {
            motion: &overlay,
            seconds: 0.2,
            weight: 0.5,
        }];
        let posed = assembly.mesh_at_layers(0.2, &layers).unwrap();
        for offset in [0, 3, 6] {
            assert_eq!(posed.positions[offset], [7.0, 0.0, 0.0]);
        }
        assert_eq!(posed.uv0, rest.uv0);
        assert_eq!(posed.indices, rest.indices);
        assert_eq!(posed.normals, rest.normals);
        assert_eq!(assembly.mesh_at(0.2).unwrap().positions[0], [5.0, 0.0, 0.0]);
        let zero = [CafLayer {
            motion: &overlay,
            seconds: 0.2,
            weight: 0.0,
        }];
        assert_eq!(
            assembly.mesh_at_layers(0.2, &zero).unwrap().positions,
            assembly.mesh_at(0.2).unwrap().positions
        );
        let wrong = clip("unrelated", 10.0);
        assert!(
            assembly
                .mesh_at_layers(
                    0.2,
                    &[CafLayer {
                        motion: &wrong,
                        seconds: 0.2,
                        weight: 1.0
                    }]
                )
                .is_err()
        );
        assert_eq!(assembly.rest.positions, rest.positions);
    }
    fn mesh() -> RenderMesh {
        RenderMesh {
            info: ModelInfo {
                format: "synthetic".into(),
                version: 1,
                chunk_count: 1,
                vertex_count: 3,
                triangle_count: 1,
                original_uv0: true,
                original_normals: true,
            },
            positions: vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            indices: vec![0, 1, 2],
            uv0: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
            normals: Some(vec![[0.0, 0.0, 1.0]; 3]),
        }
    }
    fn weights() -> VerifiedWeights {
        VerifiedWeights {
            joint_ids: vec![[0; 8]; 3],
            weight_bytes: vec![[255, 0, 0, 0, 0, 0, 0, 0]; 3],
            influences: 4,
            subsets: 1,
        }
    }
    #[test]
    fn exact_one_bone_skinning_moves_vertices_and_preserves_uv_index_stream() {
        let binding = trs([2.0, 3.0, 4.0], [0.0, 0.0, 0.0, 1.0]).unwrap();
        let original = mesh();
        let deformed = deform(&original, &weights(), &[binding]).unwrap();
        assert_eq!(deformed.positions[0], [3.0, 3.0, 4.0]);
        assert_eq!(deformed.positions[1], [2.0, 4.0, 4.0]);
        assert_eq!(deformed.indices, original.indices);
        assert_eq!(deformed.uv0, original.uv0);
        assert_eq!(original.positions[0], [1.0, 0.0, 0.0]);
    }
    #[test]
    fn invalid_bone_reference_and_weights_fail_instead_of_drawing_wrong_pose() {
        let mut bad = weights();
        bad.joint_ids[0][0] = 7;
        assert!(
            deform(
                &mesh(),
                &bad,
                &[trs([0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap()]
            )
            .is_err()
        );
        let mut bad = weights();
        bad.weight_bytes[0][0] = 200;
        assert!(
            deform(
                &mesh(),
                &bad,
                &[trs([0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap()]
            )
            .is_err()
        );
    }
    #[test]
    fn mounted_equipment_follows_bone_delta_without_changing_uv_or_mesh_indices() {
        let original = mesh();
        let mut next = original.clone();
        let delta = trs([3.0, -2.0, 1.0], [0.0, 0.0, 0.0, 1.0]).unwrap();
        move_equipment(&mut next, &original, 1, 2, &delta).unwrap();
        assert_eq!(next.positions[0], original.positions[0]);
        assert_eq!(next.positions[1], [3.0, -1.0, 1.0]);
        assert_eq!(next.positions[2], [3.0, -2.0, 2.0]);
        assert_eq!(next.indices, original.indices);
        assert_eq!(next.uv0, original.uv0);
        assert_eq!(next.normals, original.normals);
        assert!(move_equipment(&mut next, &original, 2, 2, &delta).is_err());
    }

    #[test]
    fn quaternion_rotation_matches_source_geometry_axes() {
        let q = [
            0.0,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        ];
        let rotation = trs([0.0; 3], q).unwrap();
        let moved = transform_point(&rotation, [1.0, 0.0, 0.0]);
        assert!(moved[0].abs() < 1e-5 && (moved[1] - 1.0).abs() < 1e-5);
        assert!(trs([0.0; 3], [0.0; 4]).is_err());
    }
}
