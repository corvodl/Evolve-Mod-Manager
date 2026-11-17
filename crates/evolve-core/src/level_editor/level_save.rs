//! Saving a Level Editor draft into an editable Modding project. A draft's
//! changes become edits to the project files that hold each object, every
//! file is rewritten in memory first, and nothing is written unless all of
//! them succeed. Each write goes through the project's checked, backed-up
//! member replacement, which refuses a file that changed since it was read.
use super::level_draft::{self, Change, Draft};
use crate::modding_projects::{self, ProjectRow};
use crate::scene_authoring::{LayerAuthoringEdit, edit_native_layer_bytes};
use crate::{CompiledEdits, CompiledPlacement, PlacedObject, apply_compiled_edits};
use std::path::{Path, PathBuf};

/// Same bound as every other project member edit.
const MAX_MEMBER_BYTES: u64 = 192 * 1024 * 1024;

/// All of a map draft's changes to one project file.
#[derive(Debug, Clone)]
pub enum LevelFileEdit {
    Xml(Vec<LayerAuthoringEdit>),
    Compiled(CompiledEdits),
}

/// A placement stored in terrain.dat, including copies made in a draft.
pub fn is_compiled_kind(object: &PlacedObject) -> bool {
    object.kind.eq_ignore_ascii_case("CompiledBrush")
        || object.kind.eq_ignore_ascii_case("CompiledVegetation")
}

/// The terrain.dat record behind a Legacy-ported map's brush or vegetation.
pub fn compiled_placement(object: &PlacedObject) -> Option<CompiledPlacement> {
    let index = |prefix: &str| object.id.strip_prefix(prefix)?.parse::<usize>().ok();
    if object.kind.eq_ignore_ascii_case("CompiledBrush") {
        index("__compiled_brush_").map(CompiledPlacement::Brush)
    } else if object.kind.eq_ignore_ascii_case("CompiledVegetation") {
        index("__compiled_vegetation_").map(CompiledPlacement::Vegetation)
    } else {
        None
    }
}

/// Every unsaved change as edits to the project files that hold them.
/// `source` gives a scene object (by index in the draft's current scene) and
/// the project file that stores it.
pub fn file_edits<W>(
    draft: &Draft<W>,
    source: impl Fn(usize) -> Result<(PlacedObject, PathBuf), String>,
) -> Result<Vec<(PathBuf, LevelFileEdit)>, String> {
    let current = &draft.current;
    let loaded = |id: &str| {
        draft
            .original
            .scene
            .objects
            .iter()
            .find(|object| object.id == id)
    };
    let mut files: Vec<(PathBuf, LevelFileEdit)> = Vec::new();
    fn file(files: &mut Vec<(PathBuf, LevelFileEdit)>, path: PathBuf, compiled: bool) -> usize {
        if let Some(at) = files.iter().position(|(p, _)| *p == path) {
            return at;
        }
        files.push((
            path,
            if compiled {
                LevelFileEdit::Compiled(CompiledEdits::default())
            } else {
                LevelFileEdit::Xml(Vec::new())
            },
        ));
        files.len() - 1
    }
    for change in level_draft::changes(&draft.original, current) {
        let index = match &change {
            Change::Copy { index, .. }
            | Change::Transform { index }
            | Change::Shape { index }
            | Change::Delete { index } => *index,
        };
        let (object, path) = source(index)?;
        let compiled = is_compiled_kind(&object);
        let at = file(&mut files, path, compiled);
        let placement = |object: &PlacedObject| {
            compiled_placement(object)
                .ok_or_else(|| format!("{} is not a terrain.dat placement", object.name))
        };
        match (&mut files[at].1, change) {
            (LevelFileEdit::Compiled(edits), Change::Copy { .. }) => {
                let root = level_draft::root_origin(current, &object.id);
                let origin = loaded(root).ok_or("A copy's original placement is missing")?;
                edits.copies.push((placement(origin)?, object.world));
            }
            (LevelFileEdit::Compiled(edits), Change::Transform { .. }) => {
                // A copy is written where it is now.
                if !current.origins.contains_key(&object.id) {
                    edits.transforms.push((placement(&object)?, object.world));
                }
            }
            (LevelFileEdit::Compiled(edits), Change::Delete { .. }) => {
                edits.deletes.push(placement(&object)?);
            }
            (LevelFileEdit::Compiled(_), Change::Shape { .. }) => {}
            (LevelFileEdit::Xml(edits), Change::Copy { origin, .. }) => {
                edits.push(LayerAuthoringEdit::Duplicate {
                    guid: origin,
                    new_guid: object.id.clone(),
                    name: object.name.clone(),
                    position: Some(object.local.position),
                });
            }
            (LevelFileEdit::Xml(edits), Change::Transform { .. }) => {
                edits.push(LayerAuthoringEdit::Transform {
                    guid: object.id.clone(),
                    transform: object.local,
                });
            }
            (LevelFileEdit::Xml(edits), Change::Shape { .. }) => {
                edits.push(LayerAuthoringEdit::Shape {
                    guid: object.id.clone(),
                    points: object.shape_points.clone(),
                });
            }
            (LevelFileEdit::Xml(edits), Change::Delete { .. }) => {
                edits.push(LayerAuthoringEdit::Delete {
                    guid: object.id.clone(),
                });
            }
        }
    }
    Ok(files)
}

/// One project member as a draft save would rewrite it.
#[derive(Debug)]
pub struct EditedMember {
    /// Path inside the project's `files` folder, with `/` separators.
    pub member: String,
    pub original: Vec<u8>,
    pub output: Vec<u8>,
}

/// Rewrite each file in memory. Every source must be a regular file inside
/// the project's `files` folder; nothing is written here.
pub fn edit_files(
    project: &ProjectRow,
    files: Vec<(PathBuf, LevelFileEdit)>,
) -> Result<Vec<EditedMember>, String> {
    let root = std::fs::canonicalize(project.workspace.join("files")).map_err(|e| e.to_string())?;
    let mut outputs = Vec::with_capacity(files.len());
    for (source, edit) in files {
        let metadata = std::fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
        let source = std::fs::canonicalize(&source).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file()
            || metadata.len() > MAX_MEMBER_BYTES
            || !source.starts_with(&root)
        {
            return Err("Source is not a regular managed project member".into());
        }
        let member = source
            .strip_prefix(&root)
            .map_err(|_| "Level source is outside this Modding project")?
            .to_string_lossy()
            .replace('\\', "/");
        let lower = member.to_ascii_lowercase();
        let original = std::fs::read(&source).map_err(|e| e.to_string())?;
        let output = match edit {
            LevelFileEdit::Xml(edits) => {
                if !(lower.ends_with(".lyr") || lower.ends_with(".xml")) {
                    return Err(format!("{member} is not an XML layer"));
                }
                let mut bytes = original.clone();
                for edit in &edits {
                    bytes = edit_native_layer_bytes(&bytes, edit)
                        .map_err(|e| format!("{member}: {e}"))?;
                }
                bytes
            }
            LevelFileEdit::Compiled(edits) => {
                if lower != "terrain/terrain.dat" && !lower.ends_with("/terrain/terrain.dat") {
                    return Err(format!("{member} is not the map's terrain.dat"));
                }
                apply_compiled_edits(&original, &edits).map_err(|e| format!("{member}: {e}"))?
            }
        };
        outputs.push(EditedMember {
            member,
            original,
            output,
        });
    }
    Ok(outputs)
}

/// Save a draft's file edits into the project, all or nothing up to the
/// first write. Returns how many members changed.
pub fn save_files(
    project: &ProjectRow,
    projects_root: &Path,
    files: Vec<(PathBuf, LevelFileEdit)>,
) -> Result<usize, String> {
    let mut saved = 0;
    for edited in edit_files(project, files)? {
        if edited.output != edited.original
            && modding_projects::save_level_member_checked(
                project,
                projects_root,
                &edited.member,
                &edited.original,
                &edited.output,
            )?
        {
            saved += 1;
        }
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LevelScene, Transform};
    use std::sync::Arc;

    fn object(id: &str, kind: &str) -> PlacedObject {
        PlacedObject {
            id: id.into(),
            parent: String::new(),
            layer: "layers/main.lyr".into(),
            name: id.into(),
            kind: kind.into(),
            model: String::new(),
            material: String::new(),
            hidden: false,
            local: Transform::default(),
            world: Transform::default(),
            status: String::new(),
            populations: Vec::new(),
            home_area: String::new(),
            shape_points: Vec::new(),
            wildlife: Default::default(),
        }
    }

    fn draft(objects: Vec<PlacedObject>) -> Draft<()> {
        Draft::new(level_draft::Snapshot {
            scene: Arc::new(LevelScene {
                schema: 1,
                objects,
                diagnostics: Vec::new(),
            }),
            world: None,
            origins: Arc::default(),
            deleted: Arc::default(),
        })
    }

    #[test]
    fn compiled_placements_need_their_terrain_dat_id() {
        let brush = object("__compiled_brush_7", "CompiledBrush");
        assert!(is_compiled_kind(&brush));
        assert!(matches!(
            compiled_placement(&brush),
            Some(CompiledPlacement::Brush(7))
        ));
        assert!(compiled_placement(&object("__compiled_brush_x", "CompiledBrush")).is_none());
        let authored = object("{guid}", "Brush");
        assert!(!is_compiled_kind(&authored));
        assert!(compiled_placement(&authored).is_none());
    }

    #[test]
    fn draft_changes_group_into_one_edit_list_per_file() {
        let mut draft = draft(vec![object("a", "Brush"), object("b", "Brush")]);
        let moved = Transform {
            position: [1.0, 2.0, 3.0],
            ..Transform::default()
        };
        draft.push(level_draft::transform(&draft.current, &[(0, moved)]).unwrap());
        draft.push(level_draft::delete(&draft.current, &[1]));
        let scene = draft.current.scene.clone();
        let files = file_edits(&draft, |index| {
            Ok((scene.objects[index].clone(), PathBuf::from("main.lyr")))
        })
        .unwrap();
        assert_eq!(files.len(), 1);
        let LevelFileEdit::Xml(edits) = &files[0].1 else {
            panic!("an authored layer takes XML edits");
        };
        assert!(matches!(&edits[0], LayerAuthoringEdit::Transform { guid, .. } if guid == "a"));
        assert!(matches!(&edits[1], LayerAuthoringEdit::Delete { guid } if guid == "b"));
    }

    #[test]
    fn an_unresolvable_object_stops_the_save() {
        let mut draft = draft(vec![object("a", "Brush")]);
        draft.push(level_draft::delete(&draft.current, &[0]));
        let error = file_edits(&draft, |_| Err("no owning file".to_owned())).unwrap_err();
        assert_eq!(error, "no owning file");
    }

    #[test]
    fn a_compiled_object_without_its_record_id_is_refused() {
        let mut draft = draft(vec![object("__compiled_brush_?", "CompiledBrush")]);
        draft.push(level_draft::delete(&draft.current, &[0]));
        let scene = draft.current.scene.clone();
        let error = file_edits(&draft, |index| {
            Ok((scene.objects[index].clone(), PathBuf::from("terrain.dat")))
        })
        .unwrap_err();
        assert!(error.contains("not a terrain.dat placement"));
    }

    #[test]
    fn edits_outside_the_project_files_are_refused() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace.path().join("files")).unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let project = ProjectRow {
            label: "Test".into(),
            target: "Game/Levels/test.pak".into(),
            source_build: String::new(),
            status: String::new(),
            details: String::new(),
            workspace: workspace.path().to_path_buf(),
            modified: Vec::new(),
            changed_known: true,
            enabled: true,
        };
        let error = edit_files(
            &project,
            vec![(outside.path().to_path_buf(), LevelFileEdit::Xml(Vec::new()))],
        )
        .unwrap_err();
        assert!(error.contains("not a regular managed project member"));
    }
}
