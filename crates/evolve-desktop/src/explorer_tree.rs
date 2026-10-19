//! Bounded, deterministic file hierarchy for the read-only Asset Viewer.
//! A path is only a display label here: selecting it always returns the
//! original inventory index, which is validated by the preview handler.
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorerRow {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub directory: bool,
    pub index: Option<usize>,
    pub bytes: u64,
}

fn add_entries<'a>(
    entries: impl IntoIterator<Item = (usize, &'a str, u64)>,
    query: &str,
) -> BTreeMap<String, Vec<ExplorerRow>> {
    let mut nodes: BTreeMap<String, Vec<ExplorerRow>> = BTreeMap::new();
    let mut folders = HashSet::new();
    let filter = query.trim().to_lowercase();
    for (index, original, bytes) in entries {
        let path = original.replace('\\', "/");
        // Archive inventory paths are validated by the core. Also exclude
        // malformed paths here so the hierarchy cannot mask unsafe names.
        if path.starts_with('/')
            || path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
            || (!filter.is_empty() && !path.to_lowercase().contains(&filter))
        {
            continue;
        }
        let parts: Vec<&str> = path.split('/').collect();
        let mut parent = String::new();
        for (depth, name) in parts.iter().enumerate().take(parts.len().saturating_sub(1)) {
            let full = if parent.is_empty() {
                (*name).to_owned()
            } else {
                format!("{parent}/{name}")
            };
            if folders.insert(full.clone()) {
                nodes.entry(parent.clone()).or_default().push(ExplorerRow {
                    path: full.clone(),
                    name: (*name).to_owned(),
                    depth,
                    directory: true,
                    index: None,
                    bytes: 0,
                });
            }
            parent = full;
        }
        nodes.entry(parent).or_default().push(ExplorerRow {
            path,
            name: (*parts.last().unwrap_or(&"")).to_owned(),
            depth: parts.len().saturating_sub(1),
            directory: false,
            index: Some(index),
            bytes,
        });
    }
    for siblings in nodes.values_mut() {
        siblings.sort_by(|a, b| {
            b.directory.cmp(&a.directory).then_with(|| {
                a.name.to_lowercase().cmp(&b.name.to_lowercase())
            }).then_with(|| a.path.cmp(&b.path))
        });
    }
    nodes
}

fn walk(
    parent: &str,
    nodes: &BTreeMap<String, Vec<ExplorerRow>>,
    collapsed: &HashSet<String>,
    filtering: bool,
    limit: usize,
    result: &mut Vec<ExplorerRow>,
) {
    if result.len() >= limit {
        return;
    }
    if let Some(children) = nodes.get(parent) {
        for item in children {
            if result.len() >= limit {
                break;
            }
            result.push(item.clone());
            if item.directory && (filtering || !collapsed.contains(&item.path)) {
                walk(&item.path, nodes, collapsed, filtering, limit, result);
            }
        }
    }
}

/// Hierarchical rows, folders-first at each depth. Search displays matching
/// file paths and their ancestors, temporarily overriding collapsed folders.
pub fn visible_rows<'a>(
    entries: impl IntoIterator<Item = (usize, &'a str, u64)>,
    query: &str,
    collapsed: &HashSet<String>,
    limit: usize,
) -> Vec<ExplorerRow> {
    let nodes = add_entries(entries, query);
    let mut rows = Vec::new();
    walk("", &nodes, collapsed, !query.trim().is_empty(), limit, &mut rows);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<(usize, &'static str, u64)> {
        vec![
            (0, "characters/hunters/emet/emet_3p.cdf", 300),
            (1, "characters/hunters/emet/textures/diffuse.dds", 400),
            (2, "readme.txt", 10),
        ]
    }

    #[test]
    fn folders_are_nested_and_leaves_keep_inventory_indices() {
        let tree = visible_rows(fixture(), "", &HashSet::new(), 50);
        assert_eq!(tree[0].path, "characters");
        assert!(tree[0].directory);
        assert_eq!(tree[3].name, "emet");
        assert!(tree.iter().any(|r| r.index == Some(1) && r.depth == 4));
        assert_eq!(tree.last().unwrap().index, Some(2));
    }

    #[test]
    fn collapse_and_search_do_not_change_file_identity() {
        let mut collapsed = HashSet::new();
        collapsed.insert("characters".to_owned());
        let tree = visible_rows(fixture(), "", &collapsed, 50);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[1].index, Some(2));
        let found = visible_rows(fixture(), "diffuse.dds", &collapsed, 50);
        assert_eq!(found.last().unwrap().index, Some(1));
        assert_eq!(found.len(), 5);
    }

    #[test]
    fn caps_rendered_rows_without_discarding_hierarchy() {
        let tree = visible_rows(fixture(), "", &HashSet::new(), 2);
        assert_eq!(tree.len(), 2);
    }
}
