//! Bounded, read-only asset dependency graph for the native explorer.
//! Only explicit CryEngine CDF and MTL references produce edges. No guessed
//! basename matches or fabricated bone/animation relationships are emitted.
use crate::{
    AssetKind, AssetResolution, Inventory, MaterialDocument, parse_cdf,
    parse_material, resolve_virtual,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const MAX_INPUT: u64 = 5 * 1024 * 1024;
const MAX_NODES: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum DependencyError {
    #[error("Choose an indexed .cdf or .mtl file inside the selected asset root")]
    Unsupported,
    #[error("Unsafe, linked, missing or too-large asset")]
    Unsafe,
    #[error("Unable to inspect selected asset: {0}")]
    Parse(String),
    #[error("File access failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyStatus {
    Source,
    Resolved,
    Missing,
    Ambiguous,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DependencyNode {
    pub id: usize,
    pub label: String,
    pub kind: String,
    pub status: DependencyStatus,
    pub candidates: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub from: usize,
    pub to: usize,
    pub label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub schema: u32,
    pub source: String,
    pub nodes: Vec<DependencyNode>,
    pub edges: Vec<DependencyEdge>,
    pub missing: usize,
    pub ambiguous: usize,
}
impl DependencyGraph {
    fn new(source: &str) -> Self {
        Self {
            schema: 1,
            source: source.into(),
            nodes: vec![DependencyNode {
                id: 0,
                label: source.into(),
                kind: "Source".into(),
                status: DependencyStatus::Source,
                candidates: vec![],
            }],
            edges: Vec::new(),
            missing: 0,
            ambiguous: 0,
        }
    }

    fn reference(
        &mut self,
        label: &str,
        kind: &str,
        relation: &str,
        resolution: AssetResolution,
    ) -> Result<(), DependencyError> {
        if self.nodes.len() >= MAX_NODES {
            return Err(DependencyError::Unsafe);
        }
        let (status, candidates) = match resolution {
            AssetResolution::Found(path) => (DependencyStatus::Resolved, vec![path]),
            AssetResolution::Missing => {
                self.missing += 1;
                (DependencyStatus::Missing, vec![])
            }
            AssetResolution::Ambiguous(paths) => {
                self.ambiguous += 1;
                (DependencyStatus::Ambiguous, paths)
            }
        };
        let id = self.nodes.len();
        self.nodes.push(DependencyNode {
            id,
            label: label.into(),
            kind: kind.into(),
            status,
            candidates,
        });
        self.edges.push(DependencyEdge {
            from: 0,
            to: id,
            label: relation.into(),
        });
        Ok(())
    }
}

fn add_material(
    graph: &mut DependencyGraph,
    inventory: &Inventory,
    material: &MaterialDocument,
) -> Result<(), DependencyError> {
    for slot in &material.slots {
        for reference in &slot.maps {
            // Use the same authored mip preference as the material renderer.
            let source = &reference.path;
            let lower = source.to_ascii_lowercase();
            let candidates = if lower.ends_with(".tif") || lower.ends_with(".dds") {
                let stem = &source[..source.len() - 4];
                vec![format!("{stem}.dds.0"), format!("{stem}.dds")]
            } else {
                vec![source.clone()]
            };
            let mut resolved = AssetResolution::Missing;
            for path in candidates {
                resolved = resolve_virtual(inventory, &path);
                if !matches!(resolved, AssetResolution::Missing) {
                    break;
                }
            }
            graph.reference(
                source,
                "Texture",
                &format!("{} / {}", slot.name, reference.role),
                resolved,
            )?;
        }
    }
    Ok(())
}

/// Files are restricted to the inventory's canonical root; no symlink or path
/// traversal is followed. The inspector never modifies either asset library.
pub fn inspect_dependencies(
    inventory: &Inventory,
    relative: &str,
) -> Result<DependencyGraph, DependencyError> {
    let record = inventory
        .entries
        .iter()
        .find(|entry| entry.relative_path == relative)
        .ok_or(DependencyError::Unsupported)?;
    if !matches!(
        record.kind,
        AssetKind::CharacterDefinition | AssetKind::Material
    ) {
        return Err(DependencyError::Unsupported);
    }
    let root = PathBuf::from(&inventory.root);
    let canonical_root = fs::canonicalize(&root)?;
    let candidate = root.join(&record.relative_path);
    let metadata = fs::symlink_metadata(&candidate)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_INPUT {
        return Err(DependencyError::Unsafe);
    }
    let canonical_file = fs::canonicalize(&candidate)?;
    if !canonical_file.starts_with(&canonical_root) {
        return Err(DependencyError::Unsafe);
    }
    let bytes = fs::read(candidate)?;
    let mut graph = DependencyGraph::new(relative);
    match record.kind {
        AssetKind::CharacterDefinition => {
            let cdf = parse_cdf(&bytes).map_err(|e| DependencyError::Parse(e.to_string()))?;
            graph.reference(
                &cdf.skeleton,
                "Skeleton",
                "CDF model",
                resolve_virtual(inventory, &cdf.skeleton),
            )?;
            for attachment in cdf.attachments {
                graph.reference(
                    &attachment.binding,
                    "Character attachment",
                    &attachment.name,
                    resolve_virtual(inventory, &attachment.binding),
                )?;
            }
        }
        AssetKind::Material => {
            let document =
                parse_material(&bytes).map_err(|e| DependencyError::Parse(e.to_string()))?;
            add_material(&mut graph, inventory, &document)?;
        }
        _ => return Err(DependencyError::Unsupported),
    }
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan_directory;

    #[test]
    fn maps_only_explicit_cdf_references_and_reports_missing() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("models")).unwrap();
        fs::write(dir.path().join("models/hunter.chr"), b"skeleton").unwrap();
        fs::write(
            dir.path().join("character.cdf"),
            br#"<CharacterDefinition><Model File="models/hunter.chr"/><AttachmentList><Attachment AName="Body" Type="CA_SKIN" Binding="models/body.skin"/></AttachmentList></CharacterDefinition>"#,
        ).unwrap();
        let inventory = scan_directory(dir.path()).unwrap();
        let report = inspect_dependencies(&inventory, "character.cdf").unwrap();
        assert_eq!(report.nodes.len(), 3);
        assert_eq!(report.missing, 1);
        assert_eq!(report.ambiguous, 0);
        assert_eq!(report.nodes[1].status, DependencyStatus::Resolved);
        assert_eq!(report.nodes[2].status, DependencyStatus::Missing);
    }

    #[test]
    fn unindexed_or_linked_files_cannot_be_inspected() {
        let dir = tempfile::tempdir().unwrap();
        let inventory = scan_directory(dir.path()).unwrap();
        assert!(matches!(
            inspect_dependencies(&inventory, "../outside.cdf"),
            Err(DependencyError::Unsupported)
        ));
    }
}
