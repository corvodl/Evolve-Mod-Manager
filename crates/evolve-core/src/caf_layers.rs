//! Verified, bounded blending of ordinary CryEngine CAF controller poses.
//! This mixes decoded controller data by original CRC. It does not apply skin
//! deformation or claim to decode a DBA container.
use crate::native_caf::{CafBonePose, NativeCafError, NativeCafMotion};
use std::collections::{HashMap, HashSet};

const MAX_LAYERS: usize = 8;
const MAX_BONES: usize = 1024;

/// A secondary non-additive CAF sampled on the same original skeleton.
pub struct CafLayer<'a> {
    pub motion: &'a NativeCafMotion,
    pub seconds: f32,
    /// 0 leaves the base untouched; 1 replaces the matching controller.
    pub weight: f32,
}

fn bad(message: &str) -> NativeCafError {
    NativeCafError::Format(message.into())
}

/// Normalized shortest-arc quaternion interpolation. Never blend malformed
/// quaternions or invent a missing orientation.
fn quaternion_blend(a: [f32; 4], b: [f32; 4], weight: f32) -> Result<[f32; 4], NativeCafError> {
    let len_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let len_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if !len_a.is_finite()
        || !len_b.is_finite()
        || !(0.95..=1.05).contains(&len_a)
        || !(0.95..=1.05).contains(&len_b)
    {
        return Err(bad("Invalid original controller quaternion"));
    }
    let mut rhs = b.map(|x| x / len_b);
    let lhs = a.map(|x| x / len_a);
    if (0..4).map(|i| lhs[i] * rhs[i]).sum::<f32>() < 0. {
        rhs = rhs.map(|x| -x);
    }
    let mut output = std::array::from_fn(|i| lhs[i] * (1.0 - weight) + rhs[i] * weight);
    let len = output.iter().map(|x| x * x).sum::<f32>().sqrt();
    if !len.is_finite() || len < 1e-6 {
        return Err(bad("Degenerate layered controller quaternion"));
    }
    for component in &mut output {
        *component /= len;
    }
    Ok(output)
}

fn validate(pose: &[CafBonePose], allowed: &HashSet<u32>) -> Result<(), NativeCafError> {
    let mut seen = HashSet::new();
    if pose.len() > MAX_BONES || allowed.is_empty() || allowed.len() > MAX_BONES {
        return Err(bad("Too many bones, or no verified original skeleton"));
    }
    for controller in pose {
        if !allowed.contains(&controller.bone_crc32) || !seen.insert(controller.bone_crc32) {
            return Err(bad(
                "CAF has unknown or repeated original skeleton bone CRC",
            ));
        }
        if controller.translation.is_none() && controller.rotation.is_none() {
            return Err(bad("CAF controller contains no sampled transform"));
        }
        if controller
            .translation
            .is_some_and(|v| v.iter().any(|x| !x.is_finite()))
        {
            return Err(bad("Non-finite source translation"));
        }
    }
    Ok(())
}

/// Blend a base decoded CAF with at most eight non-additive overlays.
///
/// Every controller must belong to the explicitly supplied compiled-skeleton
/// CRC set. Overlays must be sparse subsets of the BASE pose; applying an
/// overlay to a bone with no base/bind-pose transform is not supported.
/// Bones absent from a secondary clip retain the base pose. Missing channels
/// retain their original source transform. This is a sampled-data operation,
/// not an animation applied to a GPU-skinned mesh.
pub fn sample_verified_caf_layers(
    base: &NativeCafMotion,
    seconds: f32,
    layers: &[CafLayer<'_>],
    original_bone_crcs: &HashSet<u32>,
) -> Result<Vec<CafBonePose>, NativeCafError> {
    if layers.len() > MAX_LAYERS || base.additive {
        return Err(bad("Unsupported number of layers or additive base clip"));
    }
    let pose = base.sample_seconds(seconds)?;
    validate(&pose, original_bone_crcs)?;
    blend_pose(pose, layers, original_bone_crcs)
}

/// Fill sparse CAF channels from verified compiled local bind transforms
/// before blending. This allows a secondary clip to animate a bone omitted
/// by the base clip without inventing an identity transform.
pub fn sample_caf_layers_with_bind(
    base: &NativeCafMotion,
    seconds: f32,
    layers: &[CafLayer<'_>],
    bind: &[CafBonePose],
) -> Result<Vec<CafBonePose>, NativeCafError> {
    if base.additive || layers.len() > MAX_LAYERS {
        return Err(bad("Unsupported additive base or layer count"));
    }
    let allowed = bind.iter().map(|p| p.bone_crc32).collect::<HashSet<_>>();
    validate(bind, &allowed)?;
    if bind
        .iter()
        .any(|p| p.translation.is_none() || p.rotation.is_none())
    {
        return Err(bad("Incomplete original local bind transforms"));
    }
    let sampled = base.sample_seconds(seconds)?;
    validate(&sampled, &allowed)?;
    let lookup = sampled
        .into_iter()
        .map(|p| (p.bone_crc32, p))
        .collect::<HashMap<_, _>>();
    let pose = bind
        .iter()
        .cloned()
        .map(|mut pose| {
            if let Some(value) = lookup.get(&pose.bone_crc32) {
                pose.translation = value.translation.or(pose.translation);
                pose.rotation = value.rotation.or(pose.rotation);
            }
            pose
        })
        .collect();
    blend_pose(pose, layers, &allowed)
}

fn blend_pose(
    mut pose: Vec<CafBonePose>,
    layers: &[CafLayer<'_>],
    original_bone_crcs: &HashSet<u32>,
) -> Result<Vec<CafBonePose>, NativeCafError> {
    let lookup: HashMap<u32, usize> = pose
        .iter()
        .enumerate()
        .map(|(i, p)| (p.bone_crc32, i))
        .collect();
    for layer in layers {
        if layer.motion.additive
            || !layer.weight.is_finite()
            || !(0.0..=1.0).contains(&layer.weight)
        {
            return Err(bad(
                "Additive clips or invalid layer weight require a verified bind-pose mixer",
            ));
        }
        if layer.weight == 0.0 {
            continue;
        }
        let overlay = layer.motion.sample_seconds(layer.seconds)?;
        validate(&overlay, original_bone_crcs)?;
        for controller in overlay {
            let index = *lookup.get(&controller.bone_crc32).ok_or_else(|| {
                bad("Layer references a bone absent from the base controller pose")
            })?;
            let target = &mut pose[index];
            match (target.translation, controller.translation) {
                (Some(a), Some(b)) => {
                    target.translation = Some(std::array::from_fn(|i| {
                        a[i] * (1.0 - layer.weight) + b[i] * layer.weight
                    }));
                }
                (None, Some(_)) => {
                    return Err(bad("Layer translation has no verified base transform"));
                }
                _ => {}
            }
            match (target.rotation, controller.rotation) {
                (Some(a), Some(b)) => target.rotation = Some(quaternion_blend(a, b, layer.weight)?),
                (None, Some(_)) => {
                    return Err(bad("Layer rotation has no verified base transform"));
                }
                _ => {}
            }
        }
    }
    Ok(pose)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_caf::CafController;

    fn clip(id: u32, value: f32) -> NativeCafMotion {
        NativeCafMotion {
            schema: 1,
            start_frame: 0.0,
            end_frame: 30.0,
            fps: 30.0,
            additive: false,
            cyclic: true,
            controllers: vec![CafController {
                bone_crc32: id,
                rotation_times: vec![0.0, 30.0],
                rotations: vec![[0.0, 0.0, 0.0, 1.0]; 2],
                position_times: vec![0.0, 30.0],
                positions: vec![[value, 0.0, 0.0]; 2],
            }],
        }
    }

    #[test]
    fn blends_matching_original_controller_and_weight_zero_is_noop() {
        let base = clip(123, 2.0);
        let overlay = clip(123, 10.0);
        let allowed = HashSet::from([123]);
        let blend = sample_verified_caf_layers(
            &base,
            0.25,
            &[CafLayer {
                motion: &overlay,
                seconds: 0.2,
                weight: 0.25,
            }],
            &allowed,
        )
        .unwrap();
        assert_eq!(blend[0].translation, Some([4.0, 0.0, 0.0]));
        assert_eq!(blend[0].rotation, Some([0.0, 0.0, 0.0, 1.0]));
        assert_eq!(
            sample_verified_caf_layers(
                &base,
                0.0,
                &[CafLayer {
                    motion: &overlay,
                    seconds: 0.0,
                    weight: 0.0
                }],
                &allowed
            )
            .unwrap()[0]
                .translation,
            Some([2.0, 0.0, 0.0])
        );
    }

    #[test]
    fn refuses_unknown_bone_and_additive_layer() {
        let base = clip(123, 0.0);
        let different = clip(999, 10.0);
        let allowed = HashSet::from([123]);
        assert!(
            sample_verified_caf_layers(
                &base,
                0.0,
                &[CafLayer {
                    motion: &different,
                    seconds: 0.0,
                    weight: 0.5
                }],
                &allowed
            )
            .is_err()
        );
        let mut additive = clip(123, 2.0);
        additive.additive = true;
        assert!(
            sample_verified_caf_layers(
                &base,
                0.0,
                &[CafLayer {
                    motion: &additive,
                    seconds: 0.0,
                    weight: 0.5
                }],
                &allowed
            )
            .is_err()
        );
    }

    #[test]
    fn refuses_non_finite_weight_and_no_skeleton() {
        let base = clip(123, 1.0);
        let allowed = HashSet::from([123]);
        assert!(
            sample_verified_caf_layers(
                &base,
                0.0,
                &[CafLayer {
                    motion: &base,
                    seconds: 0.0,
                    weight: f32::NAN
                }],
                &allowed
            )
            .is_err()
        );
        assert!(sample_verified_caf_layers(&base, 0.0, &[], &HashSet::new()).is_err());
    }
}
