//! Source-file checks the Level Editor runs before a project write: a
//! deletion must not orphan a child, entity link or wildlife home area in
//! any layer of the owning map, and a terrain.dat paint must use a surface
//! type the map defines.
use crate::LevelScene;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

/// A compiled terrain.dat's surface IDs are the SurfaceTypes listed in the
/// map's compiled leveldata.xml, beside the terrain folder. Refuse an ID the
/// map does not define, naming the ones it does.
pub fn check_compiled_surface_type(terrain: &Path, id: u8) -> Result<(), String> {
    let level = terrain
        .parent()
        .and_then(Path::parent)
        .ok_or("terrain.dat has no map folder")?;
    let xml = std::fs::read(level.join("leveldata.xml"))
        .map_err(|e| format!("Cannot read the map's leveldata.xml: {e}"))?;
    let text = String::from_utf8_lossy(&xml);
    let mut known = Vec::new();
    for (at, _) in text.match_indices("<SurfaceType ") {
        let head = &text[at..at + text[at..].find('>').unwrap_or(0)];
        let attribute = |name: &str| {
            let marker = format!(" {name}=\"");
            let start = head.find(&marker)? + marker.len();
            Some(&head[start..start + head[start..].find('"')?])
        };
        if let Some(surface) = attribute("SurfaceTypeID").and_then(|v| v.parse::<u8>().ok()) {
            if surface == id {
                return Ok(());
            }
            let name = attribute("Name").unwrap_or_default();
            known.push(format!(
                "{surface} {}",
                name.rsplit('/').next().unwrap_or(name)
            ));
        }
    }
    Err(format!(
        "SurfaceTypeID {id} is not defined in this map's leveldata.xml. Available: {}",
        known.join(", ")
    ))
}

/// The per-layer writer cannot see references in other project members.
/// Require the combined loaded scene to be free of references before sending
/// a deletion to the source-file transaction.
pub fn validate_deletion(scene: &LevelScene, guid: &str) -> Result<(), String> {
    let mut found = scene
        .objects
        .iter()
        .filter(|object| object.id.eq_ignore_ascii_case(guid));
    let object = found
        .next()
        .ok_or("Selected placement is no longer in the loaded level")?;
    if found.next().is_some() {
        return Err("Cannot delete an object with a repeated GUID across layers".into());
    }
    if scene.objects.iter().any(|other| {
        other.id != object.id
            && (other.parent.eq_ignore_ascii_case(guid)
                || other
                    .wildlife
                    .links
                    .iter()
                    .any(|target| target.eq_ignore_ascii_case(guid))
                || (object.kind.eq_ignore_ascii_case("ZooKeeperShape")
                    && !object.name.is_empty()
                    && other.home_area.eq_ignore_ascii_case(&object.name)))
    }) {
        return Err(
            "Cannot delete a placement referenced by a child, an entity link or a wildlife home area"
                .into(),
        );
    }
    Ok(())
}

/// CryEngine also stores ID-only child links inside <Entities>. The scene
/// parser deliberately excludes these from placements, so search them before
/// deleting a target whose reference could reside in a different layer.
fn contains_entity_link(bytes: &[u8], guid: &str) -> Result<bool, String> {
    use quick_xml::{Reader, events::Event};
    let xml = crate::cryxml::xml_bytes(bytes, 48 * 1024 * 1024).map_err(|e| e.to_string())?;
    let mut reader = Reader::from_reader(xml.as_ref());
    let mut parents = Vec::<Vec<u8>>::new();
    let mut events = 0usize;
    loop {
        events += 1;
        if events > 500_000 {
            return Err("Map layer exceeds XML reference-check limit".into());
        }
        match reader.read_event().map_err(|e| e.to_string())? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let element = match &event {
                    Event::Start(element) | Event::Empty(element) => element,
                    _ => unreachable!(),
                };
                if element.name().as_ref().eq_ignore_ascii_case(b"Entity")
                    && parents
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(b"Entities"))
                {
                    let mut reference = None;
                    let mut id_only = true;
                    for attr in element.attributes() {
                        let attr = attr.map_err(|e| e.to_string())?;
                        if attr.key.as_ref().eq_ignore_ascii_case(b"Id")
                            || attr.key.as_ref().eq_ignore_ascii_case(b"EntityId")
                        {
                            reference = Some(
                                attr.unescape_value()
                                    .map_err(|e| e.to_string())?
                                    .into_owned(),
                            );
                        } else {
                            id_only = false;
                        }
                    }
                    if id_only && reference.is_some_and(|id| id.eq_ignore_ascii_case(guid)) {
                        return Ok(true);
                    }
                }
                if matches!(event, Event::Start(_)) {
                    parents.push(element.name().as_ref().to_vec());
                }
            }
            Event::End(_) => {
                parents.pop();
            }
            Event::Eof => return Ok(false),
            _ => {}
        }
    }
}

/// Re-read every original layer in the owning map on the save worker, rather
/// than trusting a potentially stale UI preview. The transaction checks this
/// snapshot again immediately before replacing the selected project member.
#[derive(PartialEq, Eq)]
pub struct ProjectLayerSnapshot(Vec<(PathBuf, [u8; 32])>);

pub fn validate_project_deletion(
    files_root: &Path,
    source: &Path,
    guid: &str,
) -> Result<ProjectLayerSnapshot, String> {
    const MAX_LAYERS: usize = 1024;
    const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
    let files_root = fs::canonicalize(files_root).map_err(|e| e.to_string())?;
    let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
    let relative = source
        .strip_prefix(&files_root)
        .map_err(|_| "Source escaped the project files root")?;
    let mut map = files_root.clone();
    let mut layers_found = false;
    for part in relative.components() {
        let Component::Normal(name) = part else {
            return Err("Unsafe project layer path".into());
        };
        if name.to_string_lossy().eq_ignore_ascii_case("layers") {
            layers_found = true;
            break;
        }
        map.push(name);
    }
    if !layers_found || map == files_root {
        return Err("Cannot establish the full owning map for this deletion".into());
    }
    let mut pending = vec![map];
    let mut names = Vec::new();
    let mut visited = 0usize;
    while let Some(folder) = pending.pop() {
        let meta = fs::symlink_metadata(&folder).map_err(|e| e.to_string())?;
        if !meta.file_type().is_dir()
            || !fs::canonicalize(&folder)
                .map_err(|e| e.to_string())?
                .starts_with(&files_root)
        {
            return Err("Linked or invalid map directory cannot be checked for references".into());
        }
        for entry in fs::read_dir(&folder).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            visited += 1;
            if visited > MAX_LAYERS * 16 {
                return Err("Map directory exceeds reference-check limit".into());
            }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return Err("Linked map member cannot be checked for references".into());
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file()
                && entry.path().extension().is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("lyr") || ext.eq_ignore_ascii_case("xml")
                })
            {
                names.push(entry.path());
                if names.len() > MAX_LAYERS {
                    return Err("Map has too many XML layers for a safe deletion".into());
                }
            }
        }
    }
    names.sort();
    if !names.iter().any(|name| name == &source) {
        return Err("Owning source layer was not included in the reference check".into());
    }
    let mut total = 0u64;
    let mut data = Vec::with_capacity(names.len());
    let mut snapshot = Vec::with_capacity(names.len());
    for path in names {
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        total = total
            .checked_add(meta.len())
            .ok_or("Map layer size overflow")?;
        if !meta.file_type().is_file() || meta.len() > 48 * 1024 * 1024 || total > MAX_TOTAL_BYTES {
            return Err("Map layers exceed the bounded reference-check size".into());
        }
        if !fs::canonicalize(&path)
            .map_err(|e| e.to_string())?
            .starts_with(&files_root)
        {
            return Err("Project map member escaped the managed files root".into());
        }
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        if bytes.len() as u64 != meta.len() {
            return Err("A project layer changed during reference checking".into());
        }
        snapshot.push((path.to_owned(), Sha256::digest(&bytes).into()));
        data.push((path.to_string_lossy().into_owned(), bytes));
    }
    let refs = data
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
        .collect::<Vec<_>>();
    let recovered = crate::parse_level_scene_read_only(&refs).map_err(|e| e.to_string())?;
    if recovered
        .duplicate_guids
        .iter()
        .any(|id| id.eq_ignore_ascii_case(guid))
    {
        return Err("Cannot delete a placement with duplicate GUID declarations".into());
    }
    validate_deletion(&recovered.scene, guid)?;
    for (_, bytes) in &data {
        if contains_entity_link(bytes, guid)? {
            return Err("Cannot delete a placement referenced by a nested Entity link".into());
        }
    }
    let snapshot = ProjectLayerSnapshot(snapshot);
    snapshot.verify()?;
    Ok(snapshot)
}

impl ProjectLayerSnapshot {
    pub fn verify(&self) -> Result<(), String> {
        for (path, expected) in &self.0 {
            let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            if !meta.file_type().is_file()
                || meta.len() > 48 * 1024 * 1024
                || <[u8; 32]>::from(Sha256::digest(fs::read(path).map_err(|e| e.to_string())?))
                    != *expected
            {
                return Err(
                    "A project map layer changed during deletion; reload before retrying".into(),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_authoring::LayerAuthoringEdit;

    #[test]
    fn deletion_checks_parent_and_home_area_across_layers() {
        let area = br#"<ObjectLayer><LayerObjects><Object Id="area" Name="den" EntityClass="ZooKeeperShape"><Area/><Points><Point Pos="0,0,0"/><Point Pos="1,0,0"/><Point Pos="1,1,0"/></Points></Object><Object Id="parent" Name="parent" Type="Brush" Prefab="objects/rock.cgf"/></LayerObjects></ObjectLayer>"#;
        let children = br#"<ObjectLayer><LayerObjects><Object Id="spawn" Name="spawn" EntityClass="CreatureSpawn" Parent="parent"><Properties HomeAreaName="den"/></Object><Object Id="free" Name="free" Type="Brush" Prefab="objects/rock.cgf"/></LayerObjects></ObjectLayer>"#;
        let scene =
            crate::parse_level_scene(&[("area.lyr", area), ("children.lyr", children)]).unwrap();
        assert!(validate_deletion(&scene, "area").is_err());
        assert!(validate_deletion(&scene, "parent").is_err());
        assert!(validate_deletion(&scene, "free").is_ok());
        assert!(validate_deletion(&scene, "missing").is_err());
    }

    #[test]
    fn worker_rechecks_all_project_layers_before_deleting() {
        let temp = tempfile::tempdir().unwrap();
        let files = temp.path().join("files");
        let layers = files.join("levels/Tutorial/layers");
        fs::create_dir_all(&layers).unwrap();
        let source = layers.join("shapes.lyr");
        let other = layers.join("spawns.lyr");
        fs::write(&source, br#"<ObjectLayer><LayerObjects><Object Id="area" Name="den" EntityClass="ZooKeeperShape"><Points><Point Pos="0,0,0"/><Point Pos="1,0,0"/><Point Pos="1,1,0"/></Points></Object></LayerObjects></ObjectLayer>"#).unwrap();
        fs::write(&other, br#"<ObjectLayer><LayerObjects><Object Id="spawn" EntityClass="CreatureSpawn"><Properties HomeAreaName="den"/></Object></LayerObjects></ObjectLayer>"#).unwrap();
        assert!(validate_project_deletion(&files, &source, "area").is_err());
        fs::write(&other, br#"<ObjectLayer><LayerObjects><Object Id="host" Type="Brush" Prefab="objects/rock.cgf"><Entities><Entity Id="area"/></Entities></Object></LayerObjects></ObjectLayer>"#).unwrap();
        assert!(validate_project_deletion(&files, &source, "area").is_err());
        fs::write(&other, br#"<ObjectLayer><LayerObjects><Object Id="spawn" EntityClass="CreatureSpawn"><Properties HomeAreaName=""/></Object></LayerObjects></ObjectLayer>"#).unwrap();
        let snapshot = validate_project_deletion(&files, &source, "area").unwrap();
        assert!(snapshot.verify().is_ok());
        let added = layers.join("late.lyr");
        fs::write(&added, br#"<ObjectLayer><LayerObjects><Object Id="new" Type="Brush" Prefab="objects/rock.cgf"/></LayerObjects></ObjectLayer>"#).unwrap();
        assert!(snapshot != validate_project_deletion(&files, &source, "area").unwrap());
        fs::remove_file(&added).unwrap();
        fs::write(&other, br#"<ObjectLayer><LayerObjects><Object Id="spawn" EntityClass="CreatureSpawn"><Properties HomeAreaName="den"/></Object></LayerObjects></ObjectLayer>"#).unwrap();
        assert!(snapshot.verify().is_err());
        assert!(validate_project_deletion(&files, &source, "area").is_err());
    }

    #[test]
    fn managed_level_delete_saves_backup_and_reopens_without_touching_other_layer() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("Projects");
        let workspace = projects.join("Tutorial");
        let member = "levels/Tutorial/layers/objects.lyr";
        let source = workspace.join("files").join(member);
        let other = workspace.join("files/levels/Tutorial/layers/spawns.lyr");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        let original = br#"<ObjectLayer><LayerObjects><Object Id="rock" Name="rock" Type="Brush" Prefab="objects/rock.cgf"/><Object Id="area" Name="den" EntityClass="ZooKeeperShape"><Points><Point Pos="0,0,0"/><Point Pos="1,0,0"/><Point Pos="1,1,0"/></Points></Object></LayerObjects></ObjectLayer>"#;
        let other_bytes = br#"<ObjectLayer><LayerObjects><Object Id="spawn" Name="spawn" EntityClass="CreatureSpawn"><Properties HomeAreaName="den"/></Object></LayerObjects></ObjectLayer>"#;
        fs::write(&source, original).unwrap();
        fs::write(&other, other_bytes).unwrap();
        fs::write(workspace.join(".evolve-pak-workspace.json"), serde_json::to_vec(&serde_json::json!({
            "version": 1, "entries": [{"path": member, "format": "raw", "sha256": format!("{:x}", Sha256::digest(original))}]
        })).unwrap()).unwrap();
        let project = crate::modding_projects::ProjectRow {
            label: "Tutorial".into(),
            target: "levels_tutorial.pak".into(),
            source_build: String::new(),
            status: "Draft".into(),
            details: String::new(),
            workspace: workspace.clone(),
            modified: vec![],
            changed_known: true,
            enabled: false,
        };
        let snapshot =
            validate_project_deletion(&workspace.join("files"), &source, "rock").unwrap();
        let edited = crate::scene_authoring::edit_native_layer_bytes(
            original,
            &LayerAuthoringEdit::Delete {
                guid: "rock".into(),
            },
        )
        .unwrap();
        assert!(snapshot.verify().is_ok());
        assert!(
            crate::modding_projects::save_level_member_checked(
                &project, &projects, member, original, &edited
            )
            .unwrap()
        );
        assert!(
            crate::modding_projects::save_level_member_checked(
                &project, &projects, member, original, &edited
            )
            .is_err()
        );
        assert_eq!(fs::read(&source).unwrap(), edited);
        assert_eq!(fs::read(&other).unwrap(), other_bytes);
        let before = crate::parse_level_scene_read_only(&[
            (member, original.as_slice()),
            ("spawns.lyr", other_bytes.as_slice()),
        ])
        .unwrap();
        let saved_bytes = fs::read(&source).unwrap();
        let after = crate::parse_level_scene_read_only(&[
            (member, saved_bytes.as_slice()),
            ("spawns.lyr", other_bytes.as_slice()),
        ])
        .unwrap();
        assert_eq!(before.scene.objects.len(), after.scene.objects.len() + 1);
        assert!(
            after
                .scene
                .objects
                .iter()
                .any(|obj| obj.id == "spawn" && obj.home_area == "den")
        );
        assert_eq!(
            fs::read_dir(workspace.join("EditorBackups"))
                .unwrap()
                .count(),
            1
        );
        assert!(
            crate::modding_projects::restore_original_raw_member(&project, &projects, member)
                .unwrap()
        );
        assert_eq!(fs::read(&source).unwrap(), original);
    }

    #[test]
    #[ignore = "requires local read-only Tutorial imports (EVOLVE_BENCH_LIBRARY)"]
    fn imported_tutorial_placement_round_trips_across_all_layers() {
        use std::{fs, path::Path};
        let library = std::env::var("EVOLVE_BENCH_LIBRARY").unwrap();
        let imports = crate::imported_library(Path::new(&library)).unwrap();
        let archive = imports
            .archives
            .iter()
            .find(|pak| {
                pak.name.eq_ignore_ascii_case("levels_tutorial.pak")
                    && pak.source.to_ascii_lowercase().contains("moddedevolve")
            })
            .unwrap();
        let inventory = crate::imported_archive_files(archive).unwrap();
        let mut layer_paths = inventory
            .entries
            .iter()
            .filter(|entry| entry.relative_path.to_ascii_lowercase().ends_with(".lyr"))
            .map(|entry| entry.relative_path.clone())
            .collect::<Vec<_>>();
        layer_paths.sort();
        assert_eq!(
            layer_paths.len(),
            39,
            "expected the complete Tutorial layer set"
        );
        let layers = layer_paths
            .iter()
            .map(|name| fs::read(Path::new(&inventory.root).join(name)).unwrap())
            .collect::<Vec<_>>();
        let before = layer_paths
            .iter()
            .zip(&layers)
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>();
        let combined = crate::parse_level_scene_read_only(&before).unwrap();
        let object = combined
            .scene
            .objects
            .iter()
            .find(|obj| {
                obj.model.to_ascii_lowercase().ends_with(".cgf")
                    && !combined.duplicate_guids.contains(&obj.id)
                    && obj.id.starts_with('{')
                    && obj.id.ends_with('}')
            })
            .expect("unique authored Tutorial Brush");
        let index = layer_paths
            .iter()
            .position(|name| *name == object.layer)
            .unwrap();
        let unreferenced = combined
            .scene
            .objects
            .iter()
            .find(|candidate| {
                candidate.model.to_ascii_lowercase().ends_with(".cgf")
                    && !combined.duplicate_guids.contains(&candidate.id)
                    && validate_deletion(&combined.scene, &candidate.id).is_ok()
            })
            .expect("unreferenced Tutorial placement");
        let unreferenced_source = Path::new(&inventory.root).join(&unreferenced.layer);
        validate_project_deletion(
            Path::new(&inventory.root),
            &unreferenced_source,
            &unreferenced.id,
        )
        .unwrap()
        .verify()
        .unwrap();
        let mut position = object.local;
        position.position[0] += 1.0;
        let updated = crate::scene_authoring::edit_native_layer_bytes(
            &layers[index],
            &LayerAuthoringEdit::Transform {
                guid: object.id.clone(),
                transform: position,
            },
        )
        .unwrap();
        assert_eq!(
            updated.starts_with(b"CryXmlB\0"),
            layers[index].starts_with(b"CryXmlB\0")
        );
        let mut changed = layers.clone();
        changed[index] = updated.clone();
        let after = layer_paths
            .iter()
            .zip(&changed)
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>();
        let reparsed = crate::parse_level_scene_read_only(&after).unwrap();
        assert_eq!(reparsed.scene.objects.len(), combined.scene.objects.len());
        assert_eq!(reparsed.duplicate_guids, combined.duplicate_guids);
        for (previous, current) in combined.scene.objects.iter().zip(&reparsed.scene.objects) {
            assert_eq!(previous.id, current.id);
            if previous.id != object.id {
                assert_eq!(previous.local, current.local);
            }
        }
        assert_eq!(
            reparsed
                .scene
                .objects
                .iter()
                .find(|obj| obj.id == object.id)
                .unwrap()
                .local
                .position,
            position.position
        );
        // Stage all 39 original Tutorial layers in an isolated managed project.
        // Exercise the same backup/CAS transaction as editor commits and verify
        // the native result across the complete map, never writing to imports.
        let temp = tempfile::tempdir().unwrap();
        let projects_root = temp.path().join("Projects");
        let workspace = projects_root.join("Tutorial");
        for (name, bytes) in layer_paths.iter().zip(&layers) {
            let destination = workspace.join("files").join(name);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(destination, bytes).unwrap();
        }
        let member = object.layer.as_str();
        fs::write(
            workspace.join(".evolve-pak-workspace.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "entries": [{
                    "path": member,
                    "format": "raw",
                    "sha256": format!("{:x}", Sha256::digest(&layers[index]))
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let project = crate::modding_projects::ProjectRow {
            label: "Tutorial".into(),
            target: "levels_tutorial.pak".into(),
            source_build: String::new(),
            status: "Draft".into(),
            details: String::new(),
            workspace: workspace.clone(),
            modified: vec![],
            changed_known: true,
            enabled: false,
        };
        assert!(
            crate::modding_projects::save_level_member_checked(
                &project,
                &projects_root,
                member,
                &layers[index],
                &updated
            )
            .unwrap()
        );
        let reread = layer_paths
            .iter()
            .map(|name| fs::read(workspace.join("files").join(name)).unwrap())
            .collect::<Vec<_>>();
        let references = layer_paths
            .iter()
            .zip(&reread)
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>();
        let reopened = crate::parse_level_scene_read_only(&references).unwrap();
        assert_eq!(reopened.scene.objects.len(), combined.scene.objects.len());
        assert_eq!(reopened.duplicate_guids, combined.duplicate_guids);
        assert_eq!(
            reopened
                .scene
                .objects
                .iter()
                .find(|candidate| candidate.id == object.id)
                .unwrap()
                .local
                .position,
            position.position
        );
        assert_eq!(
            fs::read_dir(workspace.join("EditorBackups"))
                .unwrap()
                .count(),
            1
        );
        assert!(
            crate::modding_projects::restore_original_raw_member(&project, &projects_root, member)
                .unwrap()
        );
        assert_eq!(
            fs::read(workspace.join("files").join(member)).unwrap(),
            layers[index]
        );
        for (name, bytes) in layer_paths.iter().zip(&layers) {
            assert_eq!(
                fs::read(Path::new(&inventory.root).join(name)).unwrap(),
                *bytes
            );
        }
    }
}
