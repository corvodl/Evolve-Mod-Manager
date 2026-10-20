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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialSlot {
    pub name: String,
    pub shader: String,
    pub maps: Vec<TextureReference>,
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
                    if let Some((_, parent)) = material_indices.last() {
                        nonleaf[*parent] = true;
                    }
                    slots.push(MaterialSlot {
                        name,
                        shader,
                        maps: Vec::new(),
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
                }
                parents.push(tag.to_vec());
            }
            Event::Empty(node) => {
                let tag = node.name();
                let tag = tag.as_ref();
                if parents.is_empty() {
                    return Err(MaterialError::Xml("Empty XML root".into()));
                }
                if tag.eq_ignore_ascii_case(b"Texture")
                    && parents
                        .last()
                        .is_some_and(|p| p.eq_ignore_ascii_case(b"Textures"))
                    && let Some((_, index)) = material_indices.last()
                {
                    append_texture(&node, &mut slots, *index)?;
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
    let slots = slots.into_iter().enumerate()
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
mod leaf_slot_tests {
    use super::*;
    #[test]
    fn original_multi_material_leaf_order_matches_subset_indices() {
        let xml = br#"<Material Name="root"><SubMaterials>
            <Material Name="body"><Textures><Texture Map="Diffuse" File="characters/body.dds"/></Textures></Material>
            <Material Name="equipment"><Textures><Texture Map="Emittance" File="characters/glow.dds"/></Textures></Material>
            </SubMaterials></Material>"#;
        let parsed = parse_material(xml).unwrap();
        assert_eq!(parsed.slots.len(),2);
        assert_eq!(parsed.slots[0].name,"body");
        assert_eq!(parsed.slots[1].name,"equipment");
        assert_eq!(parsed.slots[1].maps[0].role,"emittance");
    }
}
