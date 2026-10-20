//! CryEngine CDF inspection. Reference resolution is exact/case-insensitive and
//! intentionally refuses to guess between asset-library duplicates.
use crate::archive::safe_member_name;
use crate::inventory::Inventory;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

const MAX_CDF_BYTES: usize = 2 * 1024 * 1024;
const MAX_DEPTH: usize = 40;
const MAX_ATTACHMENTS: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum CdfError {
    #[error("CDF is too large or uses unsafe XML entities")]
    UnsafeXml,
    #[error("Malformed CryEngine character definition: {0}")]
    Xml(String),
    #[error("CDF has no valid Model File skeleton")]
    MissingSkeleton,
    #[error("Invalid game asset path: {0}")]
    UnsafePath(String),
    #[error("I/O failure: {0}")]
    Io(#[from] io::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Skin,
    UniqueSkin,
    Bone,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CdfAttachment {
    pub name: String,
    pub binding: String,
    pub kind: AttachmentKind,
    /// The authored socket for a CA_BONE attachment, never inferred by name.
    #[serde(default)]
    pub bone_name: Option<String>,
    /// An explicit CDF per-attachment material override.
    #[serde(default)]
    pub material: Option<String>,
    /// CryEngine's authored relative attachment position (default origin).
    #[serde(default)]
    pub position: Option<[f32; 3]>,
    /// Source quaternion in CryEngine WXYZ order, if explicitly present.
    #[serde(default)]
    pub rotation: Option<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CdfDefinition {
    pub schema: u32,
    pub skeleton: String,
    pub attachments: Vec<CdfAttachment>,
    pub skin_sets: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetResolution {
    Found(String),
    Missing,
    Ambiguous(Vec<String>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CdfDependencyReport {
    pub schema: u32,
    pub skeleton: AssetResolution,
    pub attachments: Vec<(String, AssetResolution)>,
}

fn attr(node: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>, CdfError> {
    for value in node.attributes() {
        let value = value.map_err(|e| CdfError::Xml(e.to_string()))?;
        if value.key.as_ref().eq_ignore_ascii_case(name) {
            return value
                .unescape_value()
                .map(|s| Some(s.into_owned()))
                .map_err(|e| CdfError::Xml(e.to_string()));
        }
    }
    Ok(None)
}

fn normalize(raw: &str) -> Result<String, CdfError> {
    safe_member_name(raw).map_err(|_| CdfError::UnsafePath(raw.into()))
}

fn numeric_components<const N: usize>(input: &str) -> Result<[f32; N], CdfError> {
    let values = input
        .split([',', ';', ' ', '\\t'])
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<f32>().map_err(|_| CdfError::Xml("invalid attachment transform".into())))
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != N || values.iter().any(|v| !v.is_finite() || v.abs() > 1e6) {
        return Err(CdfError::Xml("invalid attachment transform dimensions".into()));
    }
    Ok(std::array::from_fn(|i| values[i]))
}

fn handle_element(
    element: &BytesStart<'_>,
    parents: &[Vec<u8>],
    skeleton: &mut Option<String>,
    attachments: &mut Vec<CdfAttachment>,
    skin_sets: &mut BTreeMap<String, Vec<String>>,
    current_skin_set: &mut Option<String>,
) -> Result<(), CdfError> {
    let name = element.name();
    let tag = name.as_ref();
    if parents.len() == 1 && tag.eq_ignore_ascii_case(b"Model") {
        if let Some(value) = attr(element, b"File")? {
            *skeleton = Some(normalize(&value)?);
        }
    } else if parents.len() == 2
        && parents
            .last()
            .is_some_and(|p| p.eq_ignore_ascii_case(b"AttachmentList"))
        && tag.eq_ignore_ascii_case(b"Attachment")
    {
        let ty = attr(element, b"Type")?.unwrap_or_default();
        let kind = match ty.to_ascii_lowercase().as_str() {
            "ca_skin" => AttachmentKind::Skin,
            "ca_skin_unique" => AttachmentKind::UniqueSkin,
            "ca_bone" => AttachmentKind::Bone,
            _ => return Ok(()),
        };
        let Some(binding) = attr(element, b"Binding")? else {
            return Ok(());
        };
        let binding = normalize(&binding)?;
        let ext = binding.rsplit('.').next().unwrap_or_default();
        let supported = match kind {
            AttachmentKind::Bone => ["cgf", "cga", "chr", "skin"].iter().any(|e| ext.eq_ignore_ascii_case(e)),
            _ => ext.eq_ignore_ascii_case("skin") || ext.eq_ignore_ascii_case("chr"),
        };
        if !supported {
            return Ok(());
        }
        if attachments.len() >= MAX_ATTACHMENTS {
            return Err(CdfError::UnsafeXml);
        }
        let fallback = binding
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .rsplit_once('.')
            .map_or(binding.as_str(), |(base, _)| base);
        let name = attr(element, b"AName")?
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback.to_string());
        let bone_name = attr(element, b"BoneName")?.filter(|value| !value.trim().is_empty());
        let material = attr(element, b"Material")?
            .or(attr(element, b"MaterialName")?)
            .or(attr(element, b"MaterialFile")?)
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                let path = if value.to_ascii_lowercase().ends_with(".mtl") {
                    value
                } else {
                    format!("{value}.mtl")
                };
                normalize(&path)
            })
            .transpose()?;
        let position = attr(element, b"Position")?
            .map(|value| numeric_components::<3>(&value))
            .transpose()?;
        let rotation = attr(element, b"Rotation")?
            .map(|value| numeric_components::<4>(&value))
            .transpose()?;
        attachments.push(CdfAttachment {
            name,
            binding,
            kind,
            bone_name,
            material,
            position,
            rotation,
        });
    } else if parents.len() == 2
        && parents
            .last()
            .is_some_and(|p| p.eq_ignore_ascii_case(b"SkinSets"))
        && tag.eq_ignore_ascii_case(b"SkinSet")
    {
        let name = attr(element, b"name")?.unwrap_or_default();
        let key = name
            .to_ascii_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        if !key.is_empty() {
            skin_sets.entry(key.clone()).or_default();
            *current_skin_set = Some(key);
        }
    } else if parents.len() == 3
        && parents
            .last()
            .is_some_and(|p| p.eq_ignore_ascii_case(b"SkinSet"))
        && tag.eq_ignore_ascii_case(b"skin")
        && let Some(group) = current_skin_set.as_ref()
        && let Some(value) = attr(element, b"name")?
    {
        let normalized = value
            .to_ascii_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        if !normalized.is_empty() {
            let set = skin_sets.entry(group.to_string()).or_default();
            if set.len() < MAX_ATTACHMENTS {
                set.push(normalized);
            }
        }
    }
    Ok(())
}

/// Parse a CDF's skeleton, actual skin/CHR attachments, and explicit skin sets.
/// Does not infer or manufacture missing meshes, bones, or equipment.
pub fn parse_cdf(bytes: &[u8]) -> Result<CdfDefinition, CdfError> {
    let decoded =
        crate::cryxml::xml_bytes(bytes, MAX_CDF_BYTES).map_err(|e| CdfError::Xml(e.to_string()))?;
    let bytes = decoded.as_ref();
    if bytes.is_empty()
        || bytes.len() > MAX_CDF_BYTES
        || bytes
            .windows(9)
            .any(|w| w.eq_ignore_ascii_case(b"<!DOCTYPE"))
        || bytes
            .windows(8)
            .any(|w| w.eq_ignore_ascii_case(b"<!ENTITY"))
    {
        return Err(CdfError::UnsafeXml);
    }
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut parents: Vec<Vec<u8>> = Vec::new();
    let mut skeleton: Option<String> = None;
    let mut attachments = Vec::new();
    let mut skin_sets = BTreeMap::new();
    let mut current_skin_set = None;
    let mut found_root = false;
    loop {
        match reader
            .read_event()
            .map_err(|e| CdfError::Xml(e.to_string()))?
        {
            Event::Start(e) => {
                if parents.len() >= MAX_DEPTH {
                    return Err(CdfError::UnsafeXml);
                }
                if parents.is_empty() {
                    if !e
                        .name()
                        .as_ref()
                        .eq_ignore_ascii_case(b"CharacterDefinition")
                    {
                        return Err(CdfError::Xml("Unexpected root element".into()));
                    }
                    found_root = true;
                } else {
                    handle_element(
                        &e,
                        &parents,
                        &mut skeleton,
                        &mut attachments,
                        &mut skin_sets,
                        &mut current_skin_set,
                    )?;
                }
                parents.push(e.name().as_ref().to_vec());
            }
            Event::Empty(e) => {
                if parents.is_empty() || !found_root {
                    return Err(CdfError::Xml("Unexpected empty root".into()));
                }
                handle_element(
                    &e,
                    &parents,
                    &mut skeleton,
                    &mut attachments,
                    &mut skin_sets,
                    &mut current_skin_set,
                )?;
            }
            Event::End(e) => {
                let Some(previous) = parents.pop() else {
                    return Err(CdfError::Xml("Unexpected closing tag".into()));
                };
                if !previous.eq_ignore_ascii_case(e.name().as_ref()) {
                    return Err(CdfError::Xml("Mismatched closing tag".into()));
                }
                if previous.eq_ignore_ascii_case(b"SkinSet") {
                    current_skin_set = None;
                }
            }
            Event::DocType(_) | Event::GeneralRef(_) => return Err(CdfError::UnsafeXml),
            Event::Eof => break,
            _ => (),
        }
    }
    if !parents.is_empty() || !found_root {
        return Err(CdfError::Xml("Unclosed or absent root".into()));
    }
    Ok(CdfDefinition {
        schema: 1,
        skeleton: skeleton.ok_or(CdfError::MissingSkeleton)?,
        attachments,
        skin_sets,
    })
}

pub fn read_cdf(path: &Path) -> Result<CdfDefinition, CdfError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > MAX_CDF_BYTES as u64 {
        return Err(CdfError::UnsafeXml);
    }
    parse_cdf(&fs::read(path)?)
}

/// Only exact virtual paths count. Project roots may prepend "files/" or
/// other scoped folders; suffix matches are accepted ONLY if unambiguous.
pub fn resolve_virtual(inventory: &Inventory, virtual_path: &str) -> AssetResolution {
    let Ok(valid) = normalize(virtual_path) else {
        return AssetResolution::Missing;
    };
    let key = valid.to_ascii_lowercase();
    let mut matches = inventory
        .entries
        .iter()
        .filter(|e| {
            let value = e.relative_path.to_ascii_lowercase();
            value == key || value.ends_with(&format!("/{key}"))
        })
        .map(|e| e.relative_path.clone())
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    match matches.len() {
        0 => AssetResolution::Missing,
        1 => AssetResolution::Found(matches.remove(0)),
        _ => AssetResolution::Ambiguous(matches),
    }
}

pub fn resolve_cdf(inventory: &Inventory, cdf: &CdfDefinition) -> CdfDependencyReport {
    CdfDependencyReport {
        schema: 1,
        skeleton: resolve_virtual(inventory, &cdf.skeleton),
        attachments: cdf
            .attachments
            .iter()
            .map(|a| (a.binding.clone(), resolve_virtual(inventory, &a.binding)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{AssetEntry, AssetKind};

    #[test]
    fn parses_legacy_kraken_and_skin_sets_without_manufacturing_geometry() {
        let text = br#"<CharacterDefinition>
            <Model File="characters/monsters/krakenelder/krakenelder1_skeleton.chr"/>
            <AttachmentList>
                <Attachment Type="CA_SKIN_UNIQUE" AName="body" Binding="characters/monsters/krakenelder/krakenelder1.skin"/>
                <Attachment Type="CA_SKIN" AName="helmet" Binding="characters/monsters/krakenelder/helmet.chr"/>
                <Attachment Type="CA_BONE" AName="eye" Binding="characters/monsters/krakenelder/eye.cgf"/>
            </AttachmentList>
            <SkinSets><SkinSet name=" Default "><skin name="body"/><skin name="Helmet"/></SkinSet></SkinSets>
        </CharacterDefinition>"#;
        let cdf = parse_cdf(text).unwrap();
        assert_eq!(cdf.attachments.len(), 2);
        assert_eq!(cdf.attachments[0].kind, AttachmentKind::UniqueSkin);
        assert_eq!(cdf.attachments[1].kind, AttachmentKind::Skin);
        assert_eq!(cdf.skin_sets["default"], vec!["body", "helmet"]);
        assert!(cdf.skeleton.ends_with("krakenelder1_skeleton.chr"));
    }

    #[test]
    fn rejects_unsafe_entities_and_traversal() {
        assert!(
            parse_cdf(
                br#"<!DOCTYPE foo [<!ENTITY x SYSTEM "file:///secret">]><CharacterDefinition/>"#
            )
            .is_err()
        );
        assert!(
            parse_cdf(
                br#"<CharacterDefinition><Model File="../outside.chr"/></CharacterDefinition>"#
            )
            .is_err()
        );
        assert!(
            parse_cdf(br#"<CharacterDefinition><AttachmentList/></CharacterDefinition>"#).is_err()
        );
    }

    #[test]
    fn exact_and_ambiguous_resolution_never_guess() {
        let mut inventory = Inventory {
            schema: 1,
            root: "synthetic".into(),
            entries: vec![],
            total_bytes: 0,
            skipped_symlinks: 0,
        };
        let skeleton = "characters/hunters/merc_tech/tech_skeleton.chr";
        inventory.entries.push(AssetEntry {
            relative_path: format!("ExtractedA/files/{skeleton}"),
            byte_size: 4,
            kind: AssetKind::Skeleton,
        });
        let cdf = parse_cdf(br#"<CharacterDefinition><Model File="characters/hunters/merc_tech/tech_skeleton.chr"/></CharacterDefinition>"#).unwrap();
        assert_eq!(
            resolve_cdf(&inventory, &cdf).skeleton,
            AssetResolution::Found(format!("ExtractedA/files/{skeleton}"))
        );
        inventory.entries.push(AssetEntry {
            relative_path: format!("ExtractedB/files/{skeleton}"),
            byte_size: 4,
            kind: AssetKind::Skeleton,
        });
        assert!(matches!(
            resolve_cdf(&inventory, &cdf).skeleton,
            AssetResolution::Ambiguous(_)
        ));
    }
}

#[cfg(test)]
mod attachment_tests {
    use super::*;

    #[test]
    fn authored_equipment_socket_and_material_are_preserved() {
        let doc = parse_cdf(br#"<CharacterDefinition><Model File="characters/rig.chr"/><AttachmentList>
            <Attachment Type="CA_BONE" AName="Scope" BoneName="weapon_socket" Binding="objects/scope.cgf"
            Position="1, 2, 3" Rotation="1,0,0,0" Material="materials/scope"/>
            </AttachmentList></CharacterDefinition>"#).unwrap();
        assert_eq!(doc.attachments.len(), 1);
        let item = &doc.attachments[0];
        assert_eq!(item.kind, AttachmentKind::Bone);
        assert_eq!(item.bone_name.as_deref(), Some("weapon_socket"));
        assert_eq!(item.material.as_deref(), Some("materials/scope.mtl"));
        assert_eq!(item.position, Some([1., 2., 3.]));
    }
}
