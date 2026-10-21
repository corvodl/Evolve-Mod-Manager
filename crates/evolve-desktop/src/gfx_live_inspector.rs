//! Read-only GFx hierarchy snapshots matching the private inspector schema.
//! This validates externally captured data but does not claim the public
//! manager has a verified Evolve game-side Scaleform traversal adapter.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

const MAX_SNAPSHOT: usize = 12_000;
const MAX_NODES: usize = 128;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiNode {
    pub id: String,
    pub parent: Option<String>,
    pub name: String,
    pub kind: String,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub schema: u8,
    pub movie: String,
    pub frame: u32,
    pub nodes: Vec<UiNode>,
}
pub fn validate_json(bytes: &[u8]) -> Result<UiSnapshot, String> {
    if bytes.len() > MAX_SNAPSHOT || bytes.is_empty() {
        return Err("UI hierarchy JSON must be 1–12000 bytes".into());
    }
    let value: UiSnapshot =
        serde_json::from_slice(bytes).map_err(|_| "Malformed UI hierarchy schema")?;
    if value.schema != 1
        || value.movie.is_empty()
        || value.movie.len() > 256
        || value.frame > 100_000
        || value.nodes.len() > MAX_NODES
    {
        return Err("Unsupported or oversized live UI hierarchy snapshot".into());
    }
    let mut ids = HashSet::new();
    let mut parents = HashMap::new();
    for item in &value.nodes {
        if item.id.is_empty()
            || item.id.len() > 120
            || !ids.insert(item.id.as_str())
            || item
                .parent
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 120)
            || item.name.len() > 256
            || item.text.len() > 512
            || ![
                "movieclip",
                "button",
                "textfield",
                "sprite",
                "shape",
                "unknown",
            ]
            .contains(&item.kind.as_str())
            || [item.x, item.y, item.width, item.height]
                .iter()
                .any(|n| !n.is_finite() || n.abs() > 100_000.)
        {
            return Err("Invalid identity/type/bounds in UI hierarchy".into());
        }
        parents.insert(item.id.as_str(), item.parent.as_deref());
    }
    for item in &value.nodes {
        let mut parent = item.parent.as_deref();
        let mut trail = HashSet::from([item.id.as_str()]);
        while let Some(id) = parent {
            if !trail.insert(id) {
                return Err("Live UI hierarchy contains a parent cycle".into());
            }
            parent = *parents.get(id).ok_or("Live UI node has missing parent")?;
        }
    }
    Ok(value)
}
pub fn from_file(path: &Path) -> Result<UiSnapshot, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_SNAPSHOT as u64 {
        return Err("Select a bounded regular UI snapshot JSON file".into());
    }
    validate_json(&fs::read(path).map_err(|e| e.to_string())?)
}
pub fn indent(snapshot: &UiSnapshot, node: &UiNode) -> usize {
    let parents: HashMap<_, _> = snapshot
        .nodes
        .iter()
        .map(|x| (x.id.as_str(), x.parent.as_deref()))
        .collect();
    let mut depth = 0;
    let mut parent = node.parent.as_deref();
    while let Some(id) = parent {
        depth += 1;
        parent = parents.get(id).copied().flatten();
        if depth > MAX_NODES {
            break;
        }
    }
    depth
}
#[cfg(test)]
mod tests {
    use super::*;
    fn payload() -> Vec<u8> {
        br#"{"schema":1,"movie":"Libs/UI/main_menu.gfx","frame":1,"nodes":[
        {"id":"root","parent":null,"name":"menu","kind":"movieclip","text":"",
        "x":0,"y":0,"width":1920,"height":1080,"visible":true},
        {"id":"title","parent":"root","name":"Featured title","kind":"textfield",
        "text":"SYNTHETIC","x":35,"y":40,"width":200,"height":25,"visible":true}]}"#
            .to_vec()
    }
    #[test]
    fn reads_synthetic_private_inspector_schema() {
        let data = validate_json(&payload()).unwrap();
        assert_eq!(data.nodes.len(), 2);
        assert_eq!(indent(&data, &data.nodes[1]), 1);
    }
    #[test]
    fn fails_closed_for_invalid_cycles_and_numbers() {
        let original = String::from_utf8(payload()).unwrap();
        assert!(validate_json(original.replace("null", "\"title\"").as_bytes()).is_err());
        assert!(validate_json(original.replace("\"y\":40", "\"y\":1e9999").as_bytes()).is_err());
        assert!(
            validate_json(
                original
                    .replace("\"id\":\"title\"", "\"id\":\"root\"")
                    .as_bytes()
            )
            .is_err()
        );
    }
}
