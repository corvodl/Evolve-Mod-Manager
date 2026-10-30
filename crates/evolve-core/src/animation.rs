//! CryEngine CAF/DBA references and verified converter-backed GLB timelines.
//! Raw CAF/DBA controller decoding is not guessed from chunk headers.
use crate::animated_glb_preview::RiggedGlbPreview;
use crate::archive::safe_member_name;
use crate::model::RenderMesh;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Turn a measured UI tick interval into playback time. A missing prior tick
/// (play/resume) and long focus stalls must not jump through an entire clip.
pub fn playback_step(previous: Option<Duration>, speed: f32) -> f32 {
    if !speed.is_finite() || speed <= 0.0 {
        return 0.0;
    }
    previous
        .map(|elapsed| elapsed.as_secs_f32().min(0.1) * speed)
        .unwrap_or(0.0)
}

const MAX_CONFIG: usize = 2 * 1024 * 1024;
const MAX_GLTF: usize = 256 * 1024 * 1024;
const MAX_FRAMES: usize = 250000;
const MAX_NODES: usize = 4096;
const MAX_CAF: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum AnimationError {
    #[error("Unsupported or corrupt motion data: {0}")]
    Invalid(String),
    #[error("Missing compatible converter or animation track")]
    Unavailable,
    #[error("Converter failed or timed out: {0}")]
    Converter(String),
    #[error("File access: {0}")]
    Io(#[from] std::io::Error),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnimationReference {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub database: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CafInspection {
    pub version: u32,
    pub chunks: usize,
    pub additive: bool,
}
pub fn inspect_caf_bytes(bytes: &[u8]) -> Result<CafInspection, AnimationError> {
    if bytes.len() < 32 || bytes.len() > MAX_CAF || bytes.get(..8) != Some(b"CrChF\x07\0\0") {
        return Err(AnimationError::Invalid(
            "not a recognized CrChF v7 CAF".into(),
        ));
    }
    let count = u32::from_le_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| AnimationError::Unavailable)?,
    ) as usize;
    let table = u32::from_le_bytes(
        bytes[12..16]
            .try_into()
            .map_err(|_| AnimationError::Unavailable)?,
    ) as usize;
    let end = table
        .checked_add(count.checked_mul(16).ok_or(AnimationError::Unavailable)?)
        .ok_or(AnimationError::Unavailable)?;
    if count == 0 || count > 10000 || table < 16 || end > bytes.len() {
        return Err(AnimationError::Invalid("unsafe CAF chunk directory".into()));
    }
    let mut additive = false;
    for i in 0..count {
        let at = table + i * 16;
        let ty = u32::from_le_bytes(
            bytes[at..at + 4]
                .try_into()
                .map_err(|_| AnimationError::Unavailable)?,
        );
        let length = u32::from_le_bytes(
            bytes[at + 8..at + 12]
                .try_into()
                .map_err(|_| AnimationError::Unavailable)?,
        ) as usize;
        let offset = u32::from_le_bytes(
            bytes[at + 12..at + 16]
                .try_into()
                .map_err(|_| AnimationError::Unavailable)?,
        ) as usize;
        if length == 0 || offset < end || offset.checked_add(length).is_none_or(|n| n > bytes.len())
        {
            return Err(AnimationError::Invalid(
                "CAF chunk boundaries invalid".into(),
            ));
        }
        if ty == 0x09253002 {
            if length < 40 {
                return Err(AnimationError::Invalid(
                    "CAF motion params truncated".into(),
                ));
            }
            additive = u32::from_le_bytes(
                bytes[offset..offset + 4]
                    .try_into()
                    .map_err(|_| AnimationError::Unavailable)?,
            ) & 1
                != 0;
        }
    }
    Ok(CafInspection {
        version: 7,
        chunks: count,
        additive,
    })
}
pub fn inspect_caf_file(path: &Path) -> Result<CafInspection, AnimationError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > MAX_CAF as u64 {
        return Err(AnimationError::Unavailable);
    }
    inspect_caf_bytes(&fs::read(path)?)
}
fn handle_animation(
    element: &BytesStart<'_>,
    parents: &[Vec<u8>],
    base: &mut String,
    out: &mut Vec<AnimationReference>,
) -> Result<(), AnimationError> {
    if !parents
        .last()
        .is_some_and(|p| p.eq_ignore_ascii_case(b"AnimationList"))
        || !element.name().as_ref().eq_ignore_ascii_case(b"Animation")
    {
        return Ok(());
    }
    let mut name = String::new();
    let mut reference = String::new();
    for attr in element.attributes() {
        let attr = attr.map_err(|e| AnimationError::Invalid(e.to_string()))?;
        let value = attr
            .unescape_value()
            .map_err(|e| AnimationError::Invalid(e.to_string()))?
            .into_owned();
        if attr.key.as_ref().eq_ignore_ascii_case(b"name") {
            name = value;
        } else if attr.key.as_ref().eq_ignore_ascii_case(b"path") {
            reference = value.replace('\\', "/");
        }
    }
    if name.eq_ignore_ascii_case("#filepath") {
        *base = reference.trim_matches('/').to_owned();
        return Ok(());
    }
    let database = name.eq_ignore_ascii_case("$TracksDatabase");
    let directive = name.starts_with('#') || name.starts_with(char::from(36u8));
    if name.is_empty()
        || reference.is_empty()
        || name.contains('*')
        || reference.contains('*')
        || (directive && !database)
    {
        return Ok(());
    }
    if let Some(offset) = reference.to_ascii_lowercase().find(".caf (") {
        reference.truncate(offset + 4);
    }
    let lower = reference.to_ascii_lowercase();
    let virtual_path = if lower.starts_with("characters/") || lower.starts_with("animations/") {
        reference
    } else {
        format!("{base}/{reference}")
            .trim_start_matches('/')
            .to_owned()
    };
    if let Ok(path) = safe_member_name(&virtual_path) {
        let kind = path
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(kind.as_str(), "caf" | "dba" | "bspace" | "comb") && out.len() < MAX_FRAMES {
            out.push(AnimationReference {
                name,
                path,
                kind,
                database,
            });
        }
    }
    Ok(())
}
pub fn animation_references(bytes: &[u8]) -> Result<Vec<AnimationReference>, AnimationError> {
    let decoded = crate::cryxml::xml_bytes(bytes, MAX_CONFIG)
        .map_err(|e| AnimationError::Invalid(e.to_string()))?;
    let bytes = decoded.as_ref();
    if bytes.is_empty()
        || bytes.len() > MAX_CONFIG
        || bytes
            .windows(9)
            .any(|w| w.eq_ignore_ascii_case(b"<!DOCTYPE"))
        || bytes
            .windows(8)
            .any(|w| w.eq_ignore_ascii_case(b"<!ENTITY"))
    {
        return Err(AnimationError::Invalid("unsafe chrparams".into()));
    }
    let mut reader = Reader::from_reader(bytes);
    let mut base = String::new();
    let mut parents: Vec<Vec<u8>> = Vec::new();
    let mut items = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|e| AnimationError::Invalid(e.to_string()))?
        {
            Event::Start(e) => {
                handle_animation(&e, &parents, &mut base, &mut items)?;
                if parents.len() >= 64 {
                    return Err(AnimationError::Invalid("XML nesting too deep".into()));
                }
                parents.push(e.name().as_ref().to_vec());
            }
            Event::Empty(e) => {
                handle_animation(&e, &parents, &mut base, &mut items)?;
            }
            Event::End(_) => {
                if parents.pop().is_none() {
                    return Err(AnimationError::Invalid("unexpected closing XML tag".into()));
                }
            }
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err(AnimationError::Invalid("unsafe XML entities".into()));
            }
            Event::Eof => break,
            _ => (),
        }
    }
    if !parents.is_empty() {
        return Err(AnimationError::Invalid("truncated XML".into()));
    }
    Ok(items)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MotionClip {
    pub name: String,
    pub duration: f32,
    pub channels: usize,
}
#[derive(Clone, Debug)]
pub struct PoseSample {
    pub node: usize,
    pub path: String,
    /// Source glTF data: translations/scale float3, rotation xyzw float4.
    pub value: Vec<f32>,
}
#[derive(Clone, Debug)]
struct Track {
    target: usize,
    path: String,
    interp: String,
    times: Vec<f32>,
    values: Vec<Vec<f32>>,
}
#[derive(Clone, Debug)]
struct Clip {
    name: String,
    duration: f32,
    tracks: Vec<Track>,
}
#[derive(Clone, Debug)]
pub struct AnimatedGlb {
    clips: Vec<Clip>,
    pub node_names: Vec<String>,
    /// Only a rig and mesh contained together in this converted GLB can
    /// animate without proving compatibility with another source skeleton.
    rigged_preview: Option<Arc<RiggedGlbPreview>>,
    pub rigged_preview_error: Option<String>,
}
fn json_u(v: &serde_json::Value, key: &str) -> Result<usize, AnimationError> {
    v[key]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| AnimationError::Invalid(format!("missing {key} integer")))
}
fn floats(
    doc: &serde_json::Value,
    binary: &[u8],
    index: usize,
    width: usize,
) -> Result<Vec<Vec<f32>>, AnimationError> {
    let accessor = doc["accessors"]
        .get(index)
        .ok_or_else(|| AnimationError::Invalid("invalid accessor".into()))?;
    if !accessor["sparse"].is_null()
        || accessor["componentType"] != 5126
        || accessor["type"]
            != match width {
                1 => "SCALAR",
                3 => "VEC3",
                4 => "VEC4",
                _ => "invalid",
            }
    {
        return Err(AnimationError::Invalid(
            "only dense float32 clip tracks supported".into(),
        ));
    }
    let count = json_u(accessor, "count")?;
    if count > MAX_FRAMES {
        return Err(AnimationError::Invalid(
            "animation key count excessive".into(),
        ));
    }
    let view = doc["bufferViews"]
        .get(json_u(accessor, "bufferView")?)
        .ok_or_else(|| AnimationError::Invalid("invalid animation bufferView".into()))?;
    if view["buffer"].as_u64().unwrap_or(0) != 0 {
        return Err(AnimationError::Invalid(
            "external buffers unsupported".into(),
        ));
    }
    let offset = json_u(view, "byteOffset")
        .unwrap_or(0)
        .checked_add(json_u(accessor, "byteOffset").unwrap_or(0))
        .ok_or_else(|| AnimationError::Invalid("accessor overflow".into()))?;
    let step = json_u(view, "byteStride").unwrap_or(width * 4);
    if step < width * 4 {
        return Err(AnimationError::Invalid("invalid animation stride".into()));
    }
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let start = offset
            .checked_add(i.checked_mul(step).ok_or(AnimationError::Unavailable)?)
            .ok_or(AnimationError::Unavailable)?;
        let row = binary
            .get(start..start + width * 4)
            .ok_or(AnimationError::Unavailable)?;
        let mut values = Vec::with_capacity(width);
        for j in 0..width {
            let v = f32::from_le_bytes(
                row[j * 4..j * 4 + 4]
                    .try_into()
                    .map_err(|_| AnimationError::Unavailable)?,
            );
            if !v.is_finite() {
                return Err(AnimationError::Invalid("non-finite keyframe".into()));
            }
            values.push(v);
        }
        result.push(values);
    }
    Ok(result)
}
/// Converter GLB rotation channels use spherical interpolation, matching the
/// private manager's shortest-arc quaternion sampling rather than nlerp.
fn gltf_slerp(first: &[f32], second: &[f32], fraction: f32) -> Result<Vec<f32>, AnimationError> {
    if first.len() != 4 || second.len() != 4 || !fraction.is_finite() {
        return Err(AnimationError::Invalid(
            "invalid GLB quaternion channel".into(),
        ));
    }
    let normalized = |values: &[f32]| -> Result<[f32; 4], AnimationError> {
        let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
        if !norm.is_finite() || norm < 1e-6 {
            return Err(AnimationError::Invalid(
                "invalid GLB joint quaternion".into(),
            ));
        }
        Ok(std::array::from_fn(|i| values[i] / norm))
    };
    let first = normalized(first)?;
    let mut second = normalized(second)?;
    let mut dot = first.iter().zip(second).map(|(a, b)| a * b).sum::<f32>();
    if dot < 0. {
        second = second.map(|v| -v);
        dot = -dot;
    }
    let alpha = fraction.clamp(0., 1.);
    let (a, b) = if dot > 0.9995 {
        (1. - alpha, alpha)
    } else {
        let theta = dot.clamp(-1., 1.).acos();
        let denominator = theta.sin();
        (
            ((1. - alpha) * theta).sin() / denominator,
            (alpha * theta).sin() / denominator,
        )
    };
    let mixed: [f32; 4] = std::array::from_fn(|i| first[i] * a + second[i] * b);
    Ok(normalized(&mixed)?.to_vec())
}
impl AnimatedGlb {
    pub fn read(path: &Path) -> Result<Self, AnimationError> {
        let meta = fs::symlink_metadata(path)?;
        if !meta.file_type().is_file() || meta.len() > MAX_GLTF as u64 || meta.len() < 32 {
            return Err(AnimationError::Invalid("invalid GLB input file".into()));
        }
        let mut file = File::open(path)?;
        let mut header = [0u8; 20];
        file.read_exact(&mut header)?;
        if &header[..4] != b"glTF"
            || &header[16..20] != b"JSON"
            || u32::from_le_bytes(
                header[8..12]
                    .try_into()
                    .map_err(|_| AnimationError::Unavailable)?,
            ) as u64
                != meta.len()
        {
            return Err(AnimationError::Invalid("invalid GLB header".into()));
        }
        let json_size = u32::from_le_bytes(
            header[12..16]
                .try_into()
                .map_err(|_| AnimationError::Unavailable)?,
        ) as usize;
        if json_size > 12 * 1024 * 1024 || json_size + 20 > meta.len() as usize {
            return Err(AnimationError::Invalid("GLB JSON too large".into()));
        }
        let mut json = vec![0u8; json_size];
        file.read_exact(&mut json)?;
        let doc: serde_json::Value =
            serde_json::from_slice(&json).map_err(|e| AnimationError::Invalid(e.to_string()))?;
        let mut marker = [0u8; 8];
        file.read_exact(&mut marker)?;
        let bytes = u32::from_le_bytes(
            marker[..4]
                .try_into()
                .map_err(|_| AnimationError::Unavailable)?,
        ) as usize;
        if &marker[4..] != b"BIN\0" || bytes > MAX_GLTF {
            return Err(AnimationError::Invalid("GLB BIN chunk unsupported".into()));
        }
        let mut binary = vec![0u8; bytes];
        file.read_exact(&mut binary)?;
        let nodes = doc["nodes"].as_array().ok_or(AnimationError::Unavailable)?;
        if nodes.len() > MAX_NODES {
            return Err(AnimationError::Unavailable);
        }
        let names = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                n["name"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("Node_{i}"))
            })
            .collect();
        let mut clips = Vec::new();
        if let Some(animations) = doc["animations"].as_array() {
            for animation in animations {
                let samples = animation["samplers"]
                    .as_array()
                    .ok_or(AnimationError::Unavailable)?;
                let channels = animation["channels"]
                    .as_array()
                    .ok_or(AnimationError::Unavailable)?;
                let mut tracks = Vec::new();
                let mut duration = 0f32;
                for channel in channels {
                    let sampler = samples
                        .get(json_u(channel, "sampler")?)
                        .ok_or(AnimationError::Unavailable)?;
                    let path = channel["target"]["path"].as_str().unwrap_or("");
                    let width = match path {
                        "translation" | "scale" => 3,
                        "rotation" => 4,
                        _ => continue,
                    };
                    let target = json_u(&channel["target"], "node")?;
                    if target >= nodes.len() {
                        return Err(AnimationError::Invalid(
                            "channel references unknown bone".into(),
                        ));
                    }
                    let times = floats(&doc, &binary, json_u(sampler, "input")?, 1)?;
                    let output = floats(&doc, &binary, json_u(sampler, "output")?, width)?;
                    if times.len() != output.len()
                        || times.is_empty()
                        || times.windows(2).any(|w| w[0] >= w[1])
                    {
                        return Err(AnimationError::Invalid("track times invalid".into()));
                    }
                    let interpolation = sampler["interpolation"].as_str().unwrap_or("LINEAR");
                    if !matches!(interpolation, "LINEAR" | "STEP") {
                        return Err(AnimationError::Invalid("unsupported interpolation".into()));
                    }
                    duration = duration.max(times.last().map(|v| v[0]).unwrap_or(0.0));
                    tracks.push(Track {
                        target,
                        path: path.into(),
                        interp: interpolation.into(),
                        times: times.into_iter().map(|v| v[0]).collect(),
                        values: output,
                    });
                }
                clips.push(Clip {
                    name: animation["name"].as_str().unwrap_or("unnamed").to_owned(),
                    duration,
                    tracks,
                });
            }
        }
        let (rigged_preview, rigged_preview_error) = match RiggedGlbPreview::from_doc(&doc, &binary)
        {
            Ok(preview) => (Some(Arc::new(preview)), None),
            Err(reason) => (None, Some(reason)),
        };
        Ok(Self {
            clips,
            node_names: names,
            rigged_preview,
            rigged_preview_error,
        })
    }
    /// True only if this exact GLB contains a verified mesh, joint table,
    /// inverse binds and weights that can be deformed by its own clips.
    pub fn has_skinned_preview(&self) -> bool {
        self.rigged_preview.is_some()
    }

    pub fn mesh_at(&self, clip_index: usize, seconds: f32) -> Result<RenderMesh, AnimationError> {
        let preview = self.rigged_preview.as_ref().ok_or_else(|| {
            AnimationError::Invalid(
                self.rigged_preview_error
                    .clone()
                    .unwrap_or_else(|| "No matching skinned GLB mesh".into()),
            )
        })?;
        let samples = self.sample(clip_index, seconds)?;
        preview.mesh_at(&samples).map_err(AnimationError::Invalid)
    }

    pub fn list_clips(&self) -> Vec<MotionClip> {
        self.clips
            .iter()
            .map(|v| MotionClip {
                name: v.name.clone(),
                duration: v.duration,
                channels: v.tracks.len(),
            })
            .collect()
    }
    pub fn sample(
        &self,
        clip_index: usize,
        seconds: f32,
    ) -> Result<Vec<PoseSample>, AnimationError> {
        if !seconds.is_finite() || seconds < 0. {
            return Err(AnimationError::Invalid("invalid time".into()));
        }
        let clip = self
            .clips
            .get(clip_index)
            .ok_or(AnimationError::Unavailable)?;
        let seconds = seconds.min(clip.duration);
        let mut result = Vec::new();
        for track in &clip.tracks {
            let upper = track.times.partition_point(|v| *v <= seconds);
            let start = upper.saturating_sub(1).min(track.times.len() - 1);
            let end = (start + 1).min(track.times.len() - 1);
            let alpha = if end > start && track.interp != "STEP" {
                ((seconds - track.times[start]) / (track.times[end] - track.times[start]))
                    .clamp(0., 1.)
            } else {
                0.
            };
            let mut vector = track.values[start]
                .iter()
                .zip(&track.values[end])
                .map(|(a, b)| a * (1. - alpha) + b * alpha)
                .collect::<Vec<_>>();
            if track.path == "rotation" {
                vector = gltf_slerp(&track.values[start], &track.values[end], alpha)?;
            }
            result.push(PoseSample {
                node: track.target,
                path: track.path.clone(),
                value: vector,
            });
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn converter_glb_slerp_uses_shortest_arc_and_unit_norm() {
        let identity = [0., 0., 0., 1.];
        let turn = [0., 0., 1., 0.];
        let halfway = gltf_slerp(&identity, &turn, 0.5).unwrap();
        assert!((halfway[2] - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.0001);
        assert!((halfway[3] - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.0001);
        let opposite = gltf_slerp(&identity, &[0., 0., 0., -1.], 0.6).unwrap();
        assert!((opposite[3] - 1.).abs() < 0.0001);
        assert!(gltf_slerp(&identity, &[0., 0., 0., 0.], 0.5).is_err());
    }

    #[test]
    fn playback_uses_measured_intervals_without_focus_jump() {
        assert_eq!(playback_step(None, 1.0), 0.0);
        assert!((playback_step(Some(Duration::from_millis(37)), 1.0) - 0.037).abs() < 0.0001);
        assert_eq!(playback_step(Some(Duration::from_secs(5)), 1.0), 0.1);
        assert_eq!(
            playback_step(Some(Duration::from_millis(40)), f32::NAN),
            0.0
        );
    }

    #[test]
    fn rejects_corrupt_caf_metadata() {
        assert!(inspect_caf_bytes(b"CrChF\x07\0\0").is_err());
    }
    #[test]
    fn reads_exact_chrparams_catalog() {
        let xml=br##"<Params><AnimationList><Animation name="#filepath" path="characters/hunters/merc_tech/"/>
          <Animation name="idle" path="animations/tech_idle.caf"/>
          <Animation name="$TracksDatabase" path="animations/tech_3p.dba"/>
          <Animation name="walk*" path="animations/*.caf"/>
        </AnimationList></Params>"##;
        let catalog = animation_references(xml).unwrap();
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog[0].name, "idle");
        assert!(catalog[1].database);
    }
}
