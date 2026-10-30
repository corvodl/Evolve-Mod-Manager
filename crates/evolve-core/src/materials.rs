//! Read-only CryEngine .mtl references. Resolve only authored texture paths.
//! This is not yet a complete shader/material renderer; unknown UV transforms,
//! opacity, surface types and legacy alpha-emission semantics remain distinct.
use crate::archive::safe_member_name;
use crate::cdf::{AssetResolution, resolve_virtual};
use crate::inventory::Inventory;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

const MAX_MTL_BYTES: usize = 5 * 1024 * 1024;
const MAX_SLOTS: usize = 1024;
const MAX_MAPS: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum MaterialError {
    #[error("Unsafe or excessive material XML")]
    Unsafe,
    #[error("Invalid CryEngine material XML: {0}")]
    Xml(String),
    #[error("Invalid referenced game asset: {0}")]
    Path(String),
    #[error("Material file I/O: {0}")]
    Io(#[from] io::Error),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextureReference {
    pub role: String,
    pub path: String,
}
/// The independently authored UV transform for an Illum Decal. This
/// applies to emission only; diffuse, normal, opacity and UV0 remain intact.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecalTexMod {
    pub tile_u: f32,
    pub tile_v: f32,
    pub u_type: i32,
    pub u_rate: f32,
    pub u_amplitude: f32,
    pub v_type: i32,
    pub v_rate: f32,
    pub v_amplitude: f32,
}

impl DecalTexMod {
    /// CryEngine ETexModMoveType: 0=no change, 1=fixed, 2=constant,
    /// 4=pan (pendulum). Unknown numeric types are not guessed.
    fn offset(kind: i32, rate: f32, amplitude: f32, seconds: f32) -> f32 {
        match kind {
            1 => amplitude,
            2 => seconds.max(0.0) * rate * amplitude,
            4 => amplitude * (seconds.max(0.0) * rate * std::f32::consts::TAU).sin(),
            _ => 0.0,
        }
    }

    /// (U tile, V tile, U offset, V offset). Called each render tick without
    /// re-decoding the source DDS or changing the actual mesh UVs.
    pub fn sample(self, seconds: f32) -> [f32; 4] {
        let t = if seconds.is_finite() {
            seconds.rem_euclid(3600.0)
        } else {
            0.0
        };
        [
            self.tile_u,
            self.tile_v,
            Self::offset(self.u_type, self.u_rate, self.u_amplitude, t),
            Self::offset(self.v_type, self.v_rate, self.v_amplitude, t),
        ]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialSlot {
    pub name: String,
    pub shader: String,
    pub maps: Vec<TextureReference>,
    /// Source MTL scalar Opacity attribute. Separate from diffuse DDS alpha
    /// (CryEngine may use that alpha for gloss, masking or emission).
    #[serde(default)]
    pub opacity: Option<f32>,
    /// CryEngine authored AlphaTest cutoff normalized to 0..1. Unlike Opacity,
    /// this clips foliage rather than alpha-blending the whole polygon.
    #[serde(default)]
    pub alpha_test: Option<f32>,
    /// Original material GlowAmount (e.g. 3.0 is not "100%" by assumption).
    #[serde(default)]
    pub glow_amount: Option<f32>,
    /// Exact CryEngine StringGenMask ALPHAGLOW flag, not diffuse alpha alone.
    #[serde(default)]
    pub alpha_glow: bool,
    /// The authored RGB Diffuse scalar. This is distinct from the Diffuse DDS.
    #[serde(default)]
    pub diffuse_tint: Option<[f32; 3]>,
    /// Source TexMod of an explicitly authored Decal, if present.
    #[serde(default)]
    pub decal_tex_mod: Option<DecalTexMod>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialDocument {
    pub schema: u32,
    pub slots: Vec<MaterialSlot>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedTexture {
    pub slot: String,
    pub role: String,
    pub authored_path: String,
    pub resolution: AssetResolution,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialReport {
    pub schema: u32,
    pub textures: Vec<ResolvedTexture>,
    pub slots: usize,
}

fn attr(node: &BytesStart<'_>, wanted: &[u8]) -> Result<Option<String>, MaterialError> {
    for attribute in node.attributes() {
        let value = attribute.map_err(|e| MaterialError::Xml(e.to_string()))?;
        if value.key.as_ref().eq_ignore_ascii_case(wanted) {
            return value
                .unescape_value()
                .map(|v| Some(v.into_owned()))
                .map_err(|e| MaterialError::Xml(e.to_string()));
        }
    }
    Ok(None)
}

/// Parse the authored scalar as a normalized material factor. It is never
/// inferred from diffuse DDS alpha or a shader name.
fn material_opacity(node: &BytesStart<'_>) -> Result<Option<f32>, MaterialError> {
    let Some(text) = attr(node, b"Opacity")? else {
        return Ok(None);
    };
    let value = text
        .trim()
        .parse::<f32>()
        .map_err(|_| MaterialError::Xml("Invalid material Opacity scalar".into()))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(MaterialError::Xml(
            "Material Opacity must be a finite value from 0 to 1".into(),
        ));
    }
    Ok(Some(value))
}

fn material_alpha_test(node: &BytesStart<'_>) -> Result<Option<f32>, MaterialError> {
    let Some(raw) = attr(node, b"AlphaTest")? else { return Ok(None) };
    let number = raw
        .trim()
        .parse::<f32>()
        .map_err(|_| MaterialError::Xml("Invalid authored AlphaTest cutoff".into()))?;
    if !number.is_finite() || !(0.0..=100.0).contains(&number) {
        return Err(MaterialError::Xml(
            "AlphaTest cutoff must be finite and in 0..1 or 0..100".into(),
        ));
    }
    // Compiled MTL values are normally normalized (0.5); the Sandbox UI
    // exposes percentages (50). Preserve both encodings, never infer a cutoff
    // from DDS alpha contents or a material filename.
    Ok(Some(if number <= 1.0 { number } else { number / 100.0 }))
}

fn authored_glow_amount(node: &BytesStart<'_>) -> Result<Option<f32>, MaterialError> {
    let raw = if let Some(amount) = attr(node, b"GlowAmount")? {
        Some(amount)
    } else if attr(node, b"Name")?.is_some_and(|name| name.eq_ignore_ascii_case("GlowAmount")) {
        attr(node, b"Value")?
    } else {
        None
    };
    let Some(raw) = raw else { return Ok(None) };
    let amount = raw
        .trim()
        .parse::<f32>()
        .map_err(|_| MaterialError::Xml("Invalid authored GlowAmount".into()))?;
    if !amount.is_finite() {
        return Err(MaterialError::Xml("GlowAmount must be finite".into()));
    }
    Ok(Some(amount))
}

fn source_alpha_glow(node: &BytesStart<'_>) -> Result<bool, MaterialError> {
    Ok(attr(node, b"StringGenMask")?
        .unwrap_or_default()
        .split('%')
        .any(|token| token.eq_ignore_ascii_case("ALPHAGLOW")))
}

/// Material Diffuse is an authored scalar, not a texture-name heuristic.
/// Malformed color values are left absent instead of replacing them with white.
fn source_diffuse_tint(node: &BytesStart<'_>) -> Result<Option<[f32; 3]>, MaterialError> {
    let Some(raw) = attr(node, b"Diffuse")? else {
        return Ok(None);
    };
    let values = raw
        .split(',')
        .map(|v| v.trim().parse::<f32>())
        .collect::<Result<Vec<_>, _>>();
    let Ok(values) = values else {
        return Ok(None);
    };
    if values.len() != 3 || values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Ok(None);
    }
    Ok(Some([values[0], values[1], values[2]]))
}

fn parse_decal_tex_mod(node: &BytesStart<'_>) -> Result<DecalTexMod, MaterialError> {
    fn scalar(node: &BytesStart<'_>, field: &[u8], default: f32) -> Result<f32, MaterialError> {
        let value = attr(node, field)?
            .map(|v| v.parse::<f32>())
            .transpose()
            .map_err(|_| MaterialError::Xml("Invalid Decal TexMod scalar".into()))?
            .unwrap_or(default);
        if !value.is_finite() || value.abs() > 64.0 {
            return Err(MaterialError::Xml(
                "Decal TexMod scalar out of bounds".into(),
            ));
        }
        Ok(value)
    }
    fn oscillator(node: &BytesStart<'_>, field: &[u8]) -> Result<i32, MaterialError> {
        attr(node, field)?
            .map(|v| {
                v.parse::<i32>()
                    .map_err(|_| MaterialError::Xml("Invalid Decal oscillator type".into()))
            })
            .transpose()
            .map(|v| v.unwrap_or(0))
    }
    let tile_u = scalar(node, b"TileU", 1.0)?;
    let tile_v = scalar(node, b"TileV", 1.0)?;
    if tile_u <= 0.0 || tile_v <= 0.0 {
        return Err(MaterialError::Xml("Decal UV tiles must be positive".into()));
    }
    Ok(DecalTexMod {
        tile_u,
        tile_v,
        u_type: oscillator(node, b"TexMod_UOscillatorType")?,
        u_rate: scalar(node, b"TexMod_UOscillatorRate", 0.0)?,
        u_amplitude: scalar(node, b"TexMod_UOscillatorAmplitude", 0.0)?,
        v_type: oscillator(node, b"TexMod_VOscillatorType")?,
        v_rate: scalar(node, b"TexMod_VOscillatorRate", 0.0)?,
        v_amplitude: scalar(node, b"TexMod_VOscillatorAmplitude", 0.0)?,
    })
}

fn normalize_texture(raw: &str) -> Result<String, MaterialError> {
    let slashes = raw.trim().replace('\\', "/");
    // Authored game materials sometimes repeat an interior separator. Keep
    // rooted paths rooted so the archive path validator still rejects them.
    let mut normalized = String::with_capacity(slashes.len());
    for c in slashes.chars() {
        if c != '/' || !normalized.ends_with('/') {
            normalized.push(c);
        }
    }
    safe_member_name(&normalized).map_err(|_| MaterialError::Path(raw.into()))
}

fn append_texture(
    node: &BytesStart<'_>,
    slots: &mut [MaterialSlot],
    index: usize,
) -> Result<(), MaterialError> {
    let Some(path) = attr(node, b"File")? else {
        return Ok(());
    };
    if path.trim().is_empty() {
        return Ok(());
    }
    let path = normalize_texture(&path)?;
    let role = attr(node, b"Map")?.unwrap_or_default().to_lowercase();
    if !matches!(
        role.as_str(),
        "diffuse"
            | "normals"
            | "normal"
            | "bumpmap"
            | "emittance"
            | "emissive"
            | "emission"
            | "glow"
            | "emissivecolor"
            | "emissionmap"
            | "glowmap"
            | "selfillum"
            | "selfillumination"
            | "decal"
            | "specular"
            | "specularcolor"
            | "gloss"
            | "glossiness"
            | "smoothness"
            | "opacity"
            | "detail"
            | "height"
            | "environment"
            | "custom"
            | "[1] custom"
    ) {
        // Keep unknown semantics out of a guessed GPU slot.
        return Ok(());
    }
    if slots.iter().map(|s| s.maps.len()).sum::<usize>() >= MAX_MAPS {
        return Err(MaterialError::Unsafe);
    }
    slots[index].maps.push(TextureReference { role, path });
    Ok(())
}

/// CryEngine Illum BLENDLAYER authored damage pair. The Opacity-named
/// damage_blend_diff file is a grayscale blend weight, *not* coverage.
/// "Custom" stores the alternate damaged diffuse. Other opacity and custom
/// channels are not repurposed as a damage blend.
pub fn authored_damage_layer(slot: &MaterialSlot) -> Option<(&str, &str)> {
    if !slot.shader.eq_ignore_ascii_case("Illum") {
        return None;
    }
    let mask = slot.maps.iter().find(|map| {
        map.role == "opacity"
            && map
                .path
                .to_ascii_lowercase()
                .contains("_damage_blend_diff.")
    })?;
    let alternate = slot.maps.iter().find(|map| {
        map.role == "custom" && map.path.to_ascii_lowercase().contains("_damage_diff.")
    })?;
    Some((alternate.path.as_str(), mask.path.as_str()))
}

/// Deterministic approximation of CryEngine's authored damage BLENDLAYER.
/// The existing diffuse and source UV0 stay unchanged, alpha is opaque for
/// Illum body materials, and a distinct authored mask drives the blend.
/// This is a viewport/FBX preview, not a recreation of every game shader knob.
pub fn blend_damage_rgba(
    base: &image::RgbaImage,
    damaged: &image::RgbaImage,
    mask: &image::RgbaImage,
    percent: u8,
) -> Result<image::RgbaImage, String> {
    if percent > 100
        || base.width() == 0
        || base.height() == 0
        || damaged.width() == 0
        || damaged.height() == 0
        || mask.width() == 0
        || mask.height() == 0
        || u64::from(base.width()) * u64::from(base.height()) > 32 * 1024 * 1024
    {
        return Err("Invalid or oversized damage-blend source image".into());
    }
    let mut blended = base.clone();
    for (x, y, pixel) in blended.enumerate_pixels_mut() {
        let dx = u64::from(x) * u64::from(damaged.width()) / u64::from(base.width());
        let dy = u64::from(y) * u64::from(damaged.height()) / u64::from(base.height());
        let mx = u64::from(x) * u64::from(mask.width()) / u64::from(base.width());
        let my = u64::from(y) * u64::from(mask.height()) / u64::from(base.height());
        let alt = damaged.get_pixel(dx as u32, dy as u32);
        let weight = u32::from(mask.get_pixel(mx as u32, my as u32)[0]) * u32::from(percent) / 100;
        for channel in 0..3 {
            pixel[channel] = ((u32::from(pixel[channel]) * (255 - weight)
                + u32::from(alt[channel]) * weight
                + 127)
                / 255) as u8;
        }
        pixel[3] = 255;
    }
    Ok(blended)
}

/// Parse explicit root/submaterial texture references, without XXE, DTD or
/// path traversal. Does not infer values from same-named model files.
pub fn parse_material(bytes: &[u8]) -> Result<MaterialDocument, MaterialError> {
    let decoded = crate::cryxml::xml_bytes(bytes, MAX_MTL_BYTES)
        .map_err(|e| MaterialError::Xml(e.to_string()))?;
    let bytes = decoded.as_ref();
    if bytes.is_empty()
        || bytes.len() > MAX_MTL_BYTES
        || bytes
            .windows(9)
            .any(|v| v.eq_ignore_ascii_case(b"<!DOCTYPE"))
        || bytes
            .windows(8)
            .any(|v| v.eq_ignore_ascii_case(b"<!ENTITY"))
    {
        return Err(MaterialError::Unsafe);
    }
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut parents: Vec<Vec<u8>> = Vec::new();
    let mut slots: Vec<MaterialSlot> = Vec::new();
    let mut nonleaf: Vec<bool> = Vec::new();
    let mut material_indices: Vec<(usize, usize)> = Vec::new();
    let mut root_seen = false;
    loop {
        match reader
            .read_event()
            .map_err(|e| MaterialError::Xml(e.to_string()))?
        {
            Event::Start(node) => {
                let tag = node.name();
                let tag = tag.as_ref();
                if parents.len() >= 64 {
                    return Err(MaterialError::Unsafe);
                }
                if parents.is_empty() {
                    if !tag.eq_ignore_ascii_case(b"Material")
                        && !tag.eq_ignore_ascii_case(b"Materials")
                    {
                        return Err(MaterialError::Xml("Expected Material root".into()));
                    }
                    root_seen = true;
                }
                if tag.eq_ignore_ascii_case(b"Material") {
                    if slots.len() >= MAX_SLOTS {
                        return Err(MaterialError::Unsafe);
                    }
                    let name =
                        attr(&node, b"Name")?.unwrap_or_else(|| format!("Material{}", slots.len()));
                    let shader = attr(&node, b"Shader")?.unwrap_or_default();
                    let opacity = material_opacity(&node)?;
                    let alpha_test = material_alpha_test(&node)?;
                    let glow_amount = authored_glow_amount(&node)?;
                    if let Some((_, parent)) = material_indices.last() {
                        nonleaf[*parent] = true;
                    }
                    slots.push(MaterialSlot {
                        name,
                        shader,
                        maps: Vec::new(),
                        opacity,
                        alpha_test,
                        glow_amount,
                        alpha_glow: source_alpha_glow(&node)?,
                        diffuse_tint: source_diffuse_tint(&node)?,
                        decal_tex_mod: None,
                    });
                    nonleaf.push(false);
                    material_indices.push((parents.len() + 1, slots.len() - 1));
                } else if tag.eq_ignore_ascii_case(b"Texture")
                    && parents
                        .last()
                        .is_some_and(|p| p.eq_ignore_ascii_case(b"Textures"))
                    && let Some((_, index)) = material_indices.last()
                {
                    append_texture(&node, &mut slots, *index)?;
                } else if tag.eq_ignore_ascii_case(b"TexMod")
                    && parents
                        .last()
                        .is_some_and(|parent| parent.eq_ignore_ascii_case(b"Texture"))
                    && let Some((_, index)) = material_indices.last()
                    && slots[*index]
                        .maps
                        .last()
                        .is_some_and(|map| map.role == "decal")
                {
                    slots[*index].decal_tex_mod = Some(parse_decal_tex_mod(&node)?);
                } else if (tag.eq_ignore_ascii_case(b"PublicParams")
                    || tag.eq_ignore_ascii_case(b"ShaderParams")
                    || (tag.eq_ignore_ascii_case(b"Param")
                        && parents.last().is_some_and(|p| {
                            p.eq_ignore_ascii_case(b"ShaderParams")
                                || p.eq_ignore_ascii_case(b"PublicParams")
                        })))
                    && let Some((_, index)) = material_indices.last()
                    && let Some(amount) = authored_glow_amount(&node)?
                {
                    slots[*index].glow_amount = Some(amount);
                }
                parents.push(tag.to_vec());
            }
            Event::Empty(node) => {
                let tag = node.name();
                let tag = tag.as_ref();
                if tag.eq_ignore_ascii_case(b"Material")
                    && (parents.is_empty()
                        || parents
                            .last()
                            .is_some_and(|p| p.eq_ignore_ascii_case(b"SubMaterials")))
                {
                    // CryEngine character MTLs contain self-closing NoDraw
                    // and otherwise empty leaves. They still occupy an
                    // original material-subset ID. Dropping them shifts all
                    // subsequent equipment and emissive material slots.
                    if slots.len() >= MAX_SLOTS || (parents.is_empty() && root_seen) {
                        return Err(MaterialError::Unsafe);
                    }
                    if parents.is_empty() {
                        root_seen = true;
                    } else if let Some((_, parent)) = material_indices.last() {
                        nonleaf[*parent] = true;
                    }
                    slots.push(MaterialSlot {
                        name: attr(&node, b"Name")?
                            .unwrap_or_else(|| format!("Material{}", slots.len())),
                        shader: attr(&node, b"Shader")?.unwrap_or_default(),
                        maps: Vec::new(),
                        opacity: material_opacity(&node)?,
                        alpha_test: material_alpha_test(&node)?,
                        glow_amount: authored_glow_amount(&node)?,
                        alpha_glow: source_alpha_glow(&node)?,
                        diffuse_tint: source_diffuse_tint(&node)?,
                        decal_tex_mod: None,
                    });
                    nonleaf.push(false);
                } else if tag.eq_ignore_ascii_case(b"Texture")
                    && parents
                        .last()
                        .is_some_and(|p| p.eq_ignore_ascii_case(b"Textures"))
                    && let Some((_, index)) = material_indices.last()
                {
                    append_texture(&node, &mut slots, *index)?;
                } else if tag.eq_ignore_ascii_case(b"TexMod")
                    && parents
                        .last()
                        .is_some_and(|parent| parent.eq_ignore_ascii_case(b"Texture"))
                    && let Some((_, index)) = material_indices.last()
                    && slots[*index]
                        .maps
                        .last()
                        .is_some_and(|map| map.role == "decal")
                {
                    slots[*index].decal_tex_mod = Some(parse_decal_tex_mod(&node)?);
                } else if (tag.eq_ignore_ascii_case(b"PublicParams")
                    || tag.eq_ignore_ascii_case(b"ShaderParams")
                    || (tag.eq_ignore_ascii_case(b"Param")
                        && parents.last().is_some_and(|p| {
                            p.eq_ignore_ascii_case(b"ShaderParams")
                                || p.eq_ignore_ascii_case(b"PublicParams")
                        })))
                    && let Some((_, index)) = material_indices.last()
                    && let Some(amount) = authored_glow_amount(&node)?
                {
                    slots[*index].glow_amount = Some(amount);
                } else if parents.is_empty() {
                    return Err(MaterialError::Xml("Empty XML root".into()));
                }
            }
            Event::End(node) => {
                let closed = parents.pop().ok_or(MaterialError::Unsafe)?;
                if !closed.eq_ignore_ascii_case(node.name().as_ref()) {
                    return Err(MaterialError::Unsafe);
                }
                if closed.eq_ignore_ascii_case(b"Material") {
                    material_indices.pop();
                }
            }
            Event::DocType(_) | Event::GeneralRef(_) => return Err(MaterialError::Unsafe),
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_seen || !parents.is_empty() || slots.is_empty() {
        return Err(MaterialError::Xml("No complete CryEngine materials".into()));
    }
    // Source subset IDs address the flattened material leaves, not their
    // parent multi-material container. Keeping the container in slot zero
    // previously shifted every character equipment/emission map by one.
    let slots = slots
        .into_iter()
        .enumerate()
        .filter_map(|(index, slot)| (!nonleaf[index]).then_some(slot))
        .collect::<Vec<_>>();
    if slots.is_empty() {
        return Err(MaterialError::Xml("Material has no leaf slots".into()));
    }
    Ok(MaterialDocument { schema: 1, slots })
}

pub fn read_material(path: &Path) -> Result<MaterialDocument, MaterialError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > MAX_MTL_BYTES as u64 {
        return Err(MaterialError::Unsafe);
    }
    parse_material(&fs::read(path)?)
}

/// Exact cooked texture search order for original CryEngine authored paths.
/// The imported Asset Library must use the same .dds.0 preference as local
/// assets; never request uncooked .tif or silently choose a different basename.
pub fn material_texture_candidates(source: &str) -> Vec<String> {
    let lower = source.to_ascii_lowercase();
    if lower.ends_with(".dds.0") {
        return vec![source.to_owned()];
    }
    if lower.ends_with(".tif") || lower.ends_with(".dds") {
        let base = &source[..source.len() - 4];
        return vec![format!("{base}.dds.0"), format!("{base}.dds")];
    }
    vec![source.to_owned()]
}

/// Port of the private manager's owner-qualified MTL lookup. Resolve each
/// cooked DDS0 before its lower-resolution DDS companion, searching the
/// material folder, its actual character, then the correct category's Shared
/// texture namespace. An explicitly global textures/ reference comes last.
/// Never match a DDS based on another actor's filename.
pub fn scoped_texture_candidates(
    authored: &str,
    material_virtual: &str,
) -> Result<Vec<String>, String> {
    let raw = authored.trim().replace('\\', "/");
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.contains(':')
        || raw.split('/').any(|part| matches!(part, "." | ".."))
    {
        return Err(format!("Unsafe original MTL texture reference: {authored}"));
    }
    let reference = raw
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if reference.is_empty() {
        return Err("Empty original material texture reference".into());
    }
    let owner = material_virtual.replace('\\', "/");
    let parents = owner.split('/').collect::<Vec<_>>();
    let folder = &parents[..parents.len().saturating_sub(1)];
    let is_character = folder.len() >= 3
        && folder[0].eq_ignore_ascii_case("characters")
        && ["hunters", "monsters", "creatures"]
            .iter()
            .any(|value| folder[1].eq_ignore_ascii_case(value));
    let is_legacy_character = folder.len() >= 4
        && folder[0].eq_ignore_ascii_case("objects")
        && folder[1].eq_ignore_ascii_case("characters")
        && ["hunters", "monsters", "creatures"]
            .iter()
            .any(|value| folder[2].eq_ignore_ascii_case(value));
    let category_length = if is_character {
        2
    } else if is_legacy_character {
        3
    } else {
        0
    };
    let minimum = if category_length > 0 {
        category_length + 1
    } else {
        1
    };
    let lower = reference.to_ascii_lowercase();
    let rooted = ["characters/", "objects/", "materials/", "levels/", "libs/"]
        .iter()
        .any(|prefix| lower.starts_with(prefix));
    let mut paths = Vec::new();
    for cooked in material_texture_candidates(&reference) {
        if rooted {
            paths.push(cooked);
            continue;
        }
        if folder.len() >= minimum {
            for count in (minimum..=folder.len()).rev() {
                let parent = folder[..count].join("/");
                paths.push(format!("{parent}/{cooked}"));
                if !cooked.contains('/') {
                    paths.push(format!("{parent}/textures/{cooked}"));
                }
            }
        }
        if category_length > 0 {
            let category = folder[..category_length].join("/");
            let lower_cooked = cooked.to_ascii_lowercase();
            if lower_cooked.starts_with("shared/") {
                paths.push(format!("{category}/{cooked}"));
            } else if lower_cooked.starts_with("textures/") {
                paths.push(format!("{category}/shared/{cooked}"));
            } else if !cooked.contains('/') {
                paths.push(format!("{category}/shared/textures/{cooked}"));
            }
            if lower_cooked.starts_with("textures/") {
                paths.push(cooked);
            }
        } else {
            paths.push(cooked);
        }
    }
    paths.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    Ok(paths)
}

/// Prefer highest-resolution cooked .dds.0 over .dds for author-provided .tif
/// references. A true ambiguity is returned, never silently resolved.
pub fn resolve_materials(inventory: &Inventory, document: &MaterialDocument) -> MaterialReport {
    let mut textures = Vec::new();
    for slot in &document.slots {
        for reference in &slot.maps {
            let source = &reference.path;
            let candidates = material_texture_candidates(source);
            let mut found = AssetResolution::Missing;
            for candidate in candidates {
                found = resolve_virtual(inventory, &candidate);
                if !matches!(found, AssetResolution::Missing) {
                    break;
                }
            }
            textures.push(ResolvedTexture {
                slot: slot.name.clone(),
                role: reference.role.clone(),
                authored_path: source.clone(),
                resolution: found,
            });
        }
    }
    MaterialReport {
        schema: 1,
        textures,
        slots: document.slots.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{AssetEntry, AssetKind};

    #[test]
    fn authored_alpha_test_cutoff_is_per_leaf_and_does_not_change_opacity() {
        let doc = parse_material(br#"<Material><SubMaterials>
            <Material Name="Leaves" Shader="Vegetation" Opacity="1" AlphaTest="0.72"/>
            <Material Name="Fern" Shader="Vegetation" AlphaTest="35"/>
            <Material Name="Default" Shader="Vegetation"/>
            <Material Name="Body" Shader="Illum" AlphaTest="0.15"/>
        </SubMaterials></Material>"#).unwrap();
        assert_eq!(doc.slots[0].alpha_test, Some(0.72));
        assert_eq!(doc.slots[0].opacity, Some(1.0));
        assert_eq!(doc.slots[1].alpha_test, Some(0.35));
        assert_eq!(doc.slots[2].alpha_test, None);
        assert_eq!(doc.slots[3].alpha_test, Some(0.15));
        for raw in ["NaN", "-1", "105", "inf", "text"] {
            assert!(parse_material(format!("<Material AlphaTest=\\\"{raw}\\\"/>").as_bytes()).is_err());
        }
    }

    #[test]
    fn glow_amount_is_preserved_per_leaf_without_invented_percent_calibration() {
        let parsed = parse_material(br#"<Material><SubMaterials>
            <Material Name="goliath" Shader="Illum"><PublicParams GlowAmount="3"/></Material>
            <Material Name="zero" Shader="Illum" GlowAmount="0"/>
            <Material Name="named" Shader="Illum"><ShaderParams><Param Name="GlowAmount" Value="0.75"/></ShaderParams></Material>
            <Material Name="absent" Shader="Illum"/>
        </SubMaterials></Material>"#).unwrap();
        assert_eq!(parsed.slots.len(), 4);
        assert_eq!(parsed.slots[0].glow_amount, Some(3.0));
        assert_eq!(parsed.slots[1].glow_amount, Some(0.0));
        assert_eq!(parsed.slots[2].glow_amount, Some(0.75));
        assert_eq!(parsed.slots[3].glow_amount, None);
        assert!(parse_material(br#"<Material GlowAmount="NaN"/>"#).is_err());
    }
    #[test]
    fn repeated_interior_separators_preserve_virtual_texture_identity() {
        assert_eq!(
            normalize_texture("textures/particle//misc/void03.tif").unwrap(),
            "textures/particle/misc/void03.tif"
        );
        assert!(normalize_texture("//server/share/file.tif").is_err());
        assert!(normalize_texture("textures//../secret.tif").is_err());
        assert!(normalize_texture("C:\\secret.tif").is_err());
    }
    #[test]
    fn original_kala_arm_diffuse_is_banshee_dxt_not_red_armor_reducer_glow() {
        // Source from merc_tech.7z and bansheegun.zip:
        // tech_3p.mtl slot Arm and techbansheegun_3p.mtl slot monster arm
        // refer to the SAME authored Banshee 3P diffuse. The latter also
        // contains distinct optional blue Banshee/red Armor Reducer glow
        // leaves, but selecting the blue gun must not repaint the red tool.
        let authored = "characters/hunters/merc_tech/weapons/bansheegun/techbansheegun_3p_diff.tif";
        let body = format!(
            r#"<Material><SubMaterials><Material Name="Arm" Shader="Illum"><Textures><Texture Map="Diffuse" File="{authored}"/></Textures></Material></SubMaterials></Material>"#
        );
        let equipment = format!(
            r#"<Material><SubMaterials>
            <Material Name="monster arm" Shader="Illum"><Textures><Texture Map="Diffuse" File="{authored}"/></Textures></Material>
            <Material Name="bansheegun glow" Shader="Illum" GlowAmount="3" Diffuse="0.072271854,0.1470273,0.42326769">
                <Textures><Texture Map="Decal" File="textures/particle/light/chroma_fan320tiled_01b.tif"/></Textures></Material>
            <Material Name="armorreducer glow" Shader="Illum" GlowAmount="3" Diffuse="0.17788844,0.0074990322,0.0043914425">
                <Textures><Texture Map="Decal" File="textures/particle/light/chroma_fan320tiled_01b.tif"/></Textures></Material>
            </SubMaterials></Material>"#
        );
        let arm = parse_material(body.as_bytes()).unwrap();
        let gun = parse_material(equipment.as_bytes()).unwrap();
        assert_eq!(arm.slots[0].maps[0].path, authored);
        assert_eq!(gun.slots[0].maps[0].path, authored);
        assert_eq!(
            material_texture_candidates(authored),
            vec![
                "characters/hunters/merc_tech/weapons/bansheegun/techbansheegun_3p_diff.dds.0",
                "characters/hunters/merc_tech/weapons/bansheegun/techbansheegun_3p_diff.dds"
            ]
        );
        let blue = gun.slots[1].diffuse_tint.unwrap();
        let red = gun.slots[2].diffuse_tint.unwrap();
        assert!(blue[2] > blue[0] && blue[2] > blue[1]);
        assert!(red[0] > red[2] && red[0] > red[1]);
    }

    #[test]
    fn real_kala_glow_material_semantics_are_distinct_from_body_alpha_glow() {
        // Synthetic MTL fixture based on the user's 2012+ Stage 2 material
        // properties. Original game files are never included in the repository.
        let source = br#"<Material><SubMaterials>
          <Material Name="Body" Shader="Illum"
            StringGenMask="%ALPHAGLOW%BUMP_MAP%GLOSS_MAP%"
            Diffuse="1,1,1" Emissive="0,0,0" GlowAmount="3">
              <Textures><Texture Map="Diffuse" File="characters/hunters/merc_tech/body.tif"/></Textures>
          </Material>
          <Material Name="bansheegun glow" Shader="Illum"
            StringGenMask="%SUBSURFACE_SCATTERING"
            Diffuse="0.072271854,0.1470273,0.42326769"
            Emissive="0,0,0" GlowAmount="3">
              <Textures><Texture Map="Diffuse" File="characters/hunters/merc_tech/body.tif"/>
                <Texture Map="Decal" File="textures/particle/light/chroma_fan320tiled_01b.tif">
                  <TexMod TileU="3" TileV="4"/>
                </Texture>
              </Textures>
          </Material>
        </SubMaterials></Material>"#;
        let parsed = parse_material(source).unwrap();
        assert_eq!(parsed.slots.len(), 2);
        assert!(parsed.slots[0].alpha_glow);
        assert!(!parsed.slots[1].alpha_glow);
        assert_eq!(parsed.slots[1].glow_amount, Some(3.0));
        assert_eq!(
            parsed.slots[1].diffuse_tint,
            Some([0.072_271_85, 0.1470273, 0.423_267_7])
        );
        assert_eq!(parsed.slots[1].maps[1].role, "decal");
    }

    #[test]
    fn exact_kala_banshee_decal_tiling_and_constant_motion() {
        let document = parse_material(br#"<Material Name="bansheegun glow" Shader="Illum"
            GlowAmount="3"><Textures><Texture Map="Decal" File="textures/particle/light/chroma_fan320tiled_01b.tif">
            <TexMod TileU="3" TileV="4" TexMod_UOscillatorType="2"
                TexMod_UOscillatorRate="1" TexMod_UOscillatorAmplitude="0.2"
                TexMod_VOscillatorAmplitude="0.1"/>
        </Texture></Textures></Material>"#).unwrap();
        let modulator = document.slots[0].decal_tex_mod.unwrap();
        assert_eq!(modulator.sample(0.0), [3.0, 4.0, 0.0, 0.0]);
        assert_eq!(modulator.sample(2.0), [3.0, 4.0, 0.4, 0.0]);
        assert_eq!(modulator.v_amplitude, 0.1);
        assert!(
            parse_material(
                br#"<Material><Textures><Texture Map="Decal" File="a.tif">
            <TexMod TileU="NaN"/></Texture></Textures></Material>"#
            )
            .is_err()
        );
    }

    #[test]
    fn kala_equipment_emissive_aliases_keep_authored_texture_paths() {
        // Equipment meshes may use alternate CryEngine material-map names.
        // These are explicitly authored slots, NOT inferred from a filename.
        let material = parse_material(
            br#"<Material Name="BansheeGun" Shader="Illum"><Textures>
            <Texture Map="Diffuse" File="characters/hunters/tech/banshee_diff.tif"/>
            <Texture Map="EmissiveColor" File="characters/hunters/tech/banshee_glow.tif"/>
            <Texture Map="GlowMap" File="characters/hunters/tech/banshee_secondary.tif"/>
        </Textures></Material>"#,
        )
        .unwrap();
        assert_eq!(material.slots[0].maps.len(), 3);
        assert_eq!(material.slots[0].maps[1].role, "emissivecolor");
        assert_eq!(
            material.slots[0].maps[1].path,
            "characters/hunters/tech/banshee_glow.tif"
        );
        assert_eq!(material.slots[0].maps[2].role, "glowmap");
    }

    #[test]
    fn imported_texture_candidates_prefer_highest_authored_mip_and_keep_spec_roles() {
        assert_eq!(
            material_texture_candidates("Characters/Kala/armor_ddn.tif"),
            vec![
                "Characters/Kala/armor_ddn.dds.0",
                "Characters/Kala/armor_ddn.dds"
            ]
        );
        assert_eq!(
            material_texture_candidates("Characters/Kala/spec.dds.0"),
            vec!["Characters/Kala/spec.dds.0"]
        );
        let authored = parse_material(
            br#"<Material Name="armor"><Textures>
                <Texture Map="SpecularColor" File="Characters/Kala/spec.tif"/>
                <Texture Map="Glossiness" File="Characters/Kala/gloss.tif"/>
                <Texture Map="Emittance" File="Characters/Kala/glow.tif"/>
            </Textures></Material>"#,
        )
        .unwrap();
        assert_eq!(authored.slots[0].maps.len(), 3);
        assert_eq!(authored.slots[0].maps[0].role, "specularcolor");
        assert_eq!(authored.slots[0].maps[1].role, "glossiness");
    }

    #[test]
    fn historical_mtl_scalar_opacity_is_not_a_diffuse_dds_alpha_channel() {
        let doc = parse_material(
            br#"<Material Name="Goliath"><SubMaterials>
            <Material Name="body" Shader="Illum" Opacity="1"><Textures>
                <Texture Map="Diffuse" File="characters/goliath.dds"/>
            </Textures></Material>
            <Material Name="saliva" Shader="Illum" Opacity="0.11"/>
        </SubMaterials></Material>"#,
        )
        .unwrap();
        assert_eq!(doc.slots.len(), 2);
        assert_eq!(doc.slots[0].opacity, Some(1.0));
        assert_eq!(doc.slots[1].opacity, Some(0.11));
        assert!(parse_material(br#"<Material Opacity="NaN"/>"#).is_err());
        assert!(parse_material(br#"<Material Opacity="-1"/>"#).is_err());
        assert!(parse_material(br#"<Material Opacity="99"/>"#).is_err());
    }

    #[test]
    fn mtl_parser_preserves_authored_maps_and_rejects_unsafe() {
        let data = br#"<Material Name="Kala" Shader="Illum"><Textures>
            <Texture Map="Diffuse" File="Characters/Kala/body.tif"/>
            <Texture Map="Normals" File="Characters/Kala/body_ddn.tif"/>
            <Texture Map="Emittance" File="Characters/Kala/glow.dds.0"/>
        </Textures></Material>"#;
        let m = parse_material(data).unwrap();
        assert_eq!(m.slots.len(), 1);
        assert_eq!(m.slots[0].maps.len(), 3);
        assert_eq!(m.slots[0].shader, "Illum");
        assert!(parse_material(br#"<!DOCTYPE x><Material/>"#).is_err());
        assert!(parse_material(br#"<Material><Textures><Texture Map="Diffuse" File="../bad.dds"/></Textures></Material>"#).is_err());
        let mut inv = Inventory {
            schema: 1,
            root: "local".into(),
            entries: vec![],
            total_bytes: 0,
            skipped_symlinks: 0,
        };
        inv.entries.push(AssetEntry {
            relative_path: "Shared/Characters/Kala/body.dds.0".into(),
            byte_size: 24,
            kind: AssetKind::Dds,
        });
        let report = resolve_materials(&inv, &m);
        assert_eq!(report.textures.len(), 3);
        assert!(matches!(
            report.textures[0].resolution,
            AssetResolution::Found(_)
        ));
        assert!(matches!(
            report.textures[1].resolution,
            AssetResolution::Missing
        ));
        inv.entries.push(AssetEntry {
            relative_path: "Other/Characters/Kala/body.dds.0".into(),
            byte_size: 24,
            kind: AssetKind::Dds,
        });
        let report = resolve_materials(&inv, &m);
        assert!(matches!(
            report.textures[0].resolution,
            AssetResolution::Ambiguous(_)
        ));
    }
}

#[cfg(test)]
mod damage_layer_tests {
    use super::*;

    #[test]
    fn authored_illum_damage_pair_is_preserved_without_becoming_opacity() {
        let doc = parse_material(
            br#"<Material Name="Goliath3"><SubMaterials>
                <Material Name="mesh" Shader="Illum"><Textures>
                    <Texture Map="Diffuse" File="characters/monsters/goliath/textures/goliath3_diff.tif"/>
                    <Texture Map="Opacity" File="characters/monsters/goliath/textures/goliath1_damage_blend_diff.tif"/>
                    <Texture Map="Custom" File="characters/monsters/goliath/textures/goliath3_damage_diff.tif"/>
                    <Texture Map="[1] Custom" File="characters/monsters/goliath/textures/goliath3_damage_ddn.tif"/>
                </Textures></Material>
                <Material Name="eyes" Shader="Illum"/>
            </SubMaterials></Material>"#,
        ).unwrap();
        let (damage, mask) = authored_damage_layer(&doc.slots[0]).unwrap();
        assert!(damage.ends_with("goliath3_damage_diff.tif"));
        assert!(mask.ends_with("goliath1_damage_blend_diff.tif"));
        assert_eq!(doc.slots[0].maps.len(), 4);
        assert!(authored_damage_layer(&doc.slots[1]).is_none());
    }

    #[test]
    fn damage_blending_changes_rgb_without_cutting_out_character() {
        let base = image::RgbaImage::from_pixel(2, 1, image::Rgba([200, 100, 50, 3]));
        let damaged = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 100, 200, 4]));
        let mask = image::RgbaImage::from_raw(2, 1, vec![0, 0, 0, 0, 255, 255, 255, 255]).unwrap();
        let mixed = blend_damage_rgba(&base, &damaged, &mask, 50).unwrap();
        assert_eq!(mixed.get_pixel(0, 0).0, [200, 100, 50, 255]);
        assert_eq!(mixed.get_pixel(1, 0).0, [100, 100, 125, 255]);
        assert_eq!(
            blend_damage_rgba(&base, &damaged, &mask, 100)
                .unwrap()
                .get_pixel(1, 0)
                .0,
            [0, 100, 200, 255]
        );
        assert!(blend_damage_rgba(&base, &damaged, &mask, 101).is_err());
    }
}

#[cfg(test)]
mod leaf_slot_tests {
    use super::*;
    #[test]
    fn empty_nodraw_leaves_retain_original_subset_positions() {
        let parsed = parse_material(
            br#"<Material Name="root"><SubMaterials>
                <Material Name="hidden" Shader="NoDraw"/>
                <Material Name="body"><Textures><Texture Map="Diffuse" File="characters/body.dds"/></Textures></Material>
                <Material Name="unused"/>
                <Material Name="equipment"><Textures><Texture Map="Emittance" File="characters/equipment_glow.dds"/></Textures></Material>
            </SubMaterials></Material>"#,
        ).unwrap();
        assert_eq!(parsed.slots.len(), 4);
        assert_eq!(parsed.slots[0].name, "hidden");
        assert_eq!(parsed.slots[0].shader, "NoDraw");
        assert!(parsed.slots[0].maps.is_empty());
        assert_eq!(parsed.slots[1].name, "body");
        assert_eq!(parsed.slots[2].name, "unused");
        assert_eq!(parsed.slots[3].name, "equipment");
        assert_eq!(parsed.slots[3].maps[0].role, "emittance");
        assert_eq!(
            parse_material(br#"<Material Name="empty"/>"#)
                .unwrap()
                .slots
                .len(),
            1
        );
    }

    #[test]
    fn original_multi_material_leaf_order_matches_subset_indices() {
        let xml = br#"<Material Name="root"><SubMaterials>
            <Material Name="body"><Textures><Texture Map="Diffuse" File="characters/body.dds"/></Textures></Material>
            <Material Name="equipment"><Textures><Texture Map="Emittance" File="characters/glow.dds"/></Textures></Material>
            </SubMaterials></Material>"#;
        let parsed = parse_material(xml).unwrap();
        assert_eq!(parsed.slots.len(), 2);
        assert_eq!(parsed.slots[0].name, "body");
        assert_eq!(parsed.slots[1].name, "equipment");
        assert_eq!(parsed.slots[1].maps[0].role, "emittance");
    }
}
